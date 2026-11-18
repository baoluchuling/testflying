use super::{
    diagnostic::{codex_diagnostic, Log},
    skill::InstalledSkill,
    worker::{drain_stderr, finish_process, wait_for_exit},
};
use serde_json::{json, Value};
use std::{io, path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
    sync::watch,
    time::{timeout, timeout_at, Instant},
};
use uuid::Uuid;

const GOAL: &str = "Fulfill the user's screenshot request and obtain confirmed delivery from the Connector upload_screenshot tool.";
const MAX_MESSAGE: u64 = 1024 * 1024;

pub(super) const INSTRUCTIONS: &str = "You own the screenshot task. Call get_capture_task first to read the task identity, center-generated jobId, app and ordered target array. The user's message is the original visual request. Fulfill each target separately without a Cartesian product. Pass its zero-based targetIndex to capture_screenshot and upload_screenshot; omit targetIndex only for legacy tasks with target=null. Finish uploading each captured image before capturing another. After delivered=false continue with an unfinished target, and finish only after delivered=true. Find the project on this computer and use the local development tools to prepare the requested screen. Choose an appropriate iOS Simulator or Android Emulator, reuse a suitable one or create and boot one as needed. Decide whether to build, install or launch the app. Treat project source as read-only and use the provided build output paths. If no simulator is ready, prepare one using the installed development tools and runtimes. Report a simulator environment blocker only when the required tools or runtime are missing and prevent preparation. Call validate_simulator with the chosen device, then capture_screenshot when the requested screen is visible. Upload the returned screenshotId using upload_screenshot. If a tool returns a retryable error, fix the issue or wait if appropriate and call it again within this Goal. Only a successful upload_screenshot response with delivered=true completes the task. If delivery cannot be retried, report the blocker. Use the provided upload tool for delivery and preserve the requested app screen until the capture is complete.";

pub(super) const SKILL_INSTRUCTIONS: &str = "Use the attached screenshot skill for preparation, navigation and visual checks. Before each final upload, apply its visual checks to the original image captured for this target. Connector tools and task identity, target order, one-image-at-a-time upload and confirmed delivery remain authoritative. Skill product examples are historical examples; use the app and targets from get_capture_task.";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GoalChange {
    Continue,
    Delivered,
    Blocked,
}

pub(super) struct ToolReply {
    pub(super) value: Value,
    pub(super) success: bool,
    pub(super) change: GoalChange,
}

impl ToolReply {
    pub(super) fn error(code: &str, message: &str, retryable: bool, change: GoalChange) -> Self {
        Self {
            value: json!({"code":code,"message":message,"retryable":retryable}),
            success: false,
            change,
        }
    }

    pub(super) fn ok(value: Value, change: GoalChange) -> Self {
        Self {
            value,
            success: true,
            change,
        }
    }
}

fn tool_specs() -> Value {
    let device = json!({
        "type":"object",
        "properties":{
            "platform":{"type":"string","enum":["ios","android"]},
            "deviceId":{"type":"string","description":"iOS Simulator UUID or Android emulator serial"}
        },
        "required":["platform","deviceId"],
        "additionalProperties":false
    });
    let mut capture = device.clone();
    capture["properties"]["targetIndex"] = json!({"type":"integer","minimum":0,"description":"Required for target-array tasks; position in get_capture_task target array"});
    json!([
        {"name":"get_capture_task","description":"Read structured task identity, original prompt, targets and completed target indices.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"validate_simulator","description":"Check that the requested local simulator is booted and available.","inputSchema":device},
        {"name":"capture_screenshot","description":"Capture the visible screen of this local simulator and return a screenshotId.","inputSchema":capture},
        {"name":"upload_screenshot","description":"Upload the captured PNG to the configured storage and obtain center confirmation. Retry the same screenshotId when instructed.","inputSchema":{
            "type":"object","properties":{"screenshotId":{"type":"string"},"targetIndex":{"type":"integer","minimum":0}},
            "required":["screenshotId"],"additionalProperties":false
        }}
    ])
}

struct Protocol {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    deadline: Instant,
}

impl Protocol {
    async fn send(&mut self, message: Value) -> io::Result<()> {
        let mut line = serde_json::to_vec(&message)?;
        line.push(b'\n');
        timeout_at(self.deadline, self.input.write_all(&line)).await?
    }

    async fn recv(&mut self) -> io::Result<Value> {
        let mut line = Vec::new();
        let count = timeout_at(
            self.deadline,
            (&mut self.output)
                .take(MAX_MESSAGE)
                .read_until(b'\n', &mut line),
        )
        .await??;
        if count == 0 || !line.ends_with(b"\n") {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Invalid App Server frame",
            ));
        }
        Ok(serde_json::from_slice(&line)?)
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) -> io::Result<Value> {
        self.send(json!({"id":id,"method":method,"params":params}))
            .await?;
        loop {
            let message = self.recv().await?;
            if message.get("method").is_some() {
                if let Some(id) = message.get("id") {
                    self.unsupported(id).await?;
                }
            } else if message["id"] == id {
                return message
                    .get("result")
                    .cloned()
                    .ok_or_else(|| io::Error::other("App Server rejected the request"));
            }
        }
    }

    async fn unsupported(&mut self, id: &Value) -> io::Result<()> {
        self.send(
            json!({"id":id,"error":{"code":-32601,"message":"Unsupported App Server request"}}),
        )
        .await
    }
}

pub(super) struct Session<'a> {
    pub(super) root: &'a Path,
    pub(super) instructions: String,
    pub(super) prompt: &'a str,
    pub(super) job: Uuid,
    pub(super) skill: Option<&'a InstalledSkill>,
}

impl Session<'_> {
    pub(super) async fn run(
        &self,
        command: &mut Command,
        log: &Log,
        stop: &mut watch::Receiver<bool>,
        call: impl AsyncFnMut(String, Value, watch::Receiver<bool>) -> ToolReply,
    ) -> Result<(), &'static str> {
        let mut child = command
            .args(["--enable", "goals", "app-server"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                log.event(
                    Some(self.job),
                    format_args!("codex_spawn_failed kind={:?}", error.kind()),
                );
                "codex_failed"
            })?;
        let pid = child.id().expect("new child has a PID") as libc::pid_t;
        let stderr = tokio::spawn(drain_stderr(child.stderr.take().expect("stderr is piped")));
        let mut rpc = Protocol {
            input: child.stdin.take().expect("stdin is piped"),
            output: BufReader::new(child.stdout.take().expect("stdout is piped")),
            deadline: Instant::now() + Duration::from_secs(45),
        };
        let result = self.drive(&mut rpc, log, stop, call).await;
        drop(rpc);
        let exited = timeout(Duration::from_secs(2), wait_for_exit(pid))
            .await
            .is_ok_and(|r| r.is_ok());
        let status = finish_process(&mut child, pid, !exited).await;
        let mut stderr = stderr;
        let diagnostic = match timeout(Duration::from_secs(1), &mut stderr).await {
            Ok(Ok(bytes)) => codex_diagnostic(&bytes).0,
            _ => {
                stderr.abort();
                "unavailable"
            }
        };
        if let Err(stage) = result {
            log.event(
                Some(self.job),
                format_args!(
                    "{stage} exit={:?} diagnostic={diagnostic}",
                    status.ok().and_then(|s| s.code())
                ),
            );
        }
        result
    }

    async fn start(&self, rpc: &mut Protocol, log: &Log) -> Result<String, &'static str> {
        rpc.request(
            0,
            "initialize",
            json!({
                "clientInfo":{"name":"testflying_screenshot","version":env!("CARGO_PKG_VERSION")},
                "capabilities":{"experimentalApi":true}
            }),
        )
        .await
        .map_err(|_| "codex_initialize_failed")?;
        rpc.send(json!({"method":"initialized"}))
            .await
            .map_err(|_| "codex_initialize_failed")?;
        if let Some(skill) = self.skill {
            let listed = rpc
                .request(
                    5,
                    "skills/list",
                    json!({"cwds":[self.root],"forceReload":true}),
                )
                .await
                .map_err(|_| "codex_skill_unavailable")?;
            if !skill_available(&listed, self.root, skill) {
                return Err("codex_skill_unavailable");
            }
            log.event(
                Some(self.job),
                format_args!(
                    "skill_discovered name={} version={} sha256={}",
                    skill.name, skill.version, skill.sha256
                ),
            );
        }
        let started = rpc
            .request(
                1,
                "thread/start",
                json!({
                    "cwd":self.root,
                    "approvalPolicy":"never",
                    "sandbox":"danger-full-access",
                    "developerInstructions":self.instructions,
                    "dynamicTools":tool_specs()
                }),
            )
            .await
            .map_err(|_| "codex_thread_start_failed")?;
        let thread = started["thread"]["id"]
            .as_str()
            .ok_or("codex_thread_start_failed")?
            .to_owned();
        let goal = rpc
            .request(
                2,
                "thread/goal/set",
                json!({
                    "threadId":thread,"objective":GOAL,"status":"active"
                }),
            )
            .await
            .map_err(|_| "codex_goal_setup_failed")?;
        if goal["goal"]["status"] != "active" {
            return Err("codex_goal_setup_failed");
        }
        let mut input = vec![json!({"type":"text","text":self.prompt})];
        if let Some(skill) = self.skill {
            input.push(json!({"type":"skill","name":skill.name,"path":skill.path}));
            log.event(
                Some(self.job),
                format_args!(
                    "skill_explicit_input name={} version={} sha256={}",
                    skill.name, skill.version, skill.sha256
                ),
            );
        }
        // The first turn can issue a tool call immediately after its response.
        rpc.request(
            3,
            "turn/start",
            json!({
                "threadId":thread,"input":input,
                "approvalPolicy":"never",
                "sandboxPolicy":{"type":"dangerFullAccess"}
            }),
        )
        .await
        .map_err(|_| "codex_turn_start_failed")?;
        Ok(thread)
    }

    async fn drive(
        &self,
        rpc: &mut Protocol,
        log: &Log,
        stop: &mut watch::Receiver<bool>,
        mut call: impl AsyncFnMut(String, Value, watch::Receiver<bool>) -> ToolReply,
    ) -> Result<(), &'static str> {
        let thread = tokio::select! {
            r = self.start(rpc, log) => r?,
            _ = stop.wait_for(|stopped| *stopped) => return Err("codex_interrupted"),
        };
        log.event(Some(self.job), "codex_goal_started");
        let limit = Instant::now() + Duration::from_secs(24 * 60);
        rpc.deadline = limit;
        loop {
            let message = tokio::select! {
                result = rpc.recv() => result.map_err(|_| "codex_goal_stream_closed")?,
                _ = stop.wait_for(|stopped| *stopped) => return Err("codex_interrupted"),
            };
            let params = &message["params"];
            let method = message["method"].as_str();
            if let Some(id) = message.get("id").filter(|_| method.is_some()) {
                if method != Some("item/tool/call") || params["threadId"] != thread {
                    rpc.unsupported(id)
                        .await
                        .map_err(|_| "codex_tool_reply_failed")?;
                    continue;
                }
                let name = params["tool"].as_str().unwrap_or_default().to_owned();
                let reply = call(name, params["arguments"].clone(), stop.clone()).await;
                // Give the result a bounded delivery window even if the tool consumed the time budget.
                rpc.deadline = Instant::now() + Duration::from_secs(5);
                rpc.send(json!({"id":id,"result":{
                    "success":reply.success,"contentItems":[{"type":"inputText","text":reply.value.to_string()}]
                }})).await.map_err(|_| "codex_tool_reply_failed")?;
                let status = match reply.change {
                    GoalChange::Delivered => "complete",
                    GoalChange::Blocked => "blocked",
                    GoalChange::Continue => {
                        rpc.deadline = limit;
                        continue;
                    }
                };
                let _ = rpc
                    .request(
                        4,
                        "thread/goal/set",
                        json!({"threadId":thread,"status":status}),
                    )
                    .await;
                return Ok(());
            }
            if params["threadId"] == thread {
                match method {
                    Some("thread/goal/updated") if params["goal"]["status"] != "active" => {
                        return Err("codex_goal_unconfirmed")
                    }
                    Some("thread/goal/cleared") => return Err("codex_goal_unconfirmed"),
                    Some("turn/completed") if params["turn"]["status"] == "completed" => {
                        // An active native Goal continues in a new turn on the same thread.
                        rpc.deadline = limit.min(Instant::now() + Duration::from_secs(30));
                    }
                    Some("turn/completed") => return Err("codex_turn_failed"),
                    Some("turn/started") => rpc.deadline = limit,
                    _ => {}
                }
            }
        }
    }
}

fn skill_available(listed: &Value, root: &Path, skill: &InstalledSkill) -> bool {
    if !skill.path.is_absolute() {
        return false;
    }
    let Some(skill_root) = skill.path.parent() else {
        return false;
    };
    listed["data"].as_array().is_some_and(|data| {
        data.iter().any(|entry| {
            entry["cwd"].as_str() == root.to_str()
                && entry["errors"].as_array().is_some_and(|errors| {
                    errors.iter().all(|error| {
                        error["path"].as_str().is_some_and(|path| {
                            let path = Path::new(path);
                            path.is_absolute() && !path.starts_with(skill_root)
                        })
                    })
                })
                && entry["skills"].as_array().is_some_and(|skills| {
                    skills.iter().any(|found| {
                        found["name"] == skill.name
                            && found["path"].as_str() == skill.path.to_str()
                            && found["enabled"] == true
                    })
                })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn verifies_native_skill_discovery_before_turn_and_preserves_original_text() {
        for mode in [
            "enabled",
            "unrelated_error",
            "disabled",
            "missing",
            "wrong_path",
            "target_error",
            "rpc_error",
        ] {
            let root = tempfile::tempdir().unwrap();
            let cwd = root.path().canonicalize().unwrap();
            let skill = InstalledSkill {
                name: "capture".into(),
                path: cwd.join(".agents/skills/capture/SKILL.md"),
                version: "a".repeat(64),
                sha256: "a".repeat(64),
            };
            let listed = json!({"data":[{"cwd":cwd,"skills": if mode == "missing" { json!([]) } else { json!([{
                "name":"capture","path":if mode == "wrong_path" { cwd.join("other/SKILL.md") } else { skill.path.clone() },"enabled":mode != "disabled"
            }]) },"errors":if mode == "target_error" { json!([{"path":skill.path,"message":"invalid skill"}]) } else if mode == "unrelated_error" { json!([{"path":cwd.join(".agents/skills/unrelated/SKILL.md"),"message":"unrelated broken skill"}]) } else { json!([]) }}]});
            let response = if mode == "rpc_error" {
                json!({"id":5,"error":{"code":-32601,"message":"unsupported skills/list"}})
            } else {
                json!({"id":5,"result":listed})
            };
            let fixture = root.path().join("fake-codex.sh");
            let transcript = root.path().join("input.jsonl");
            let mut script = format!(
                r#"#!/bin/sh
set -eu
file=$1
shift
[ "$1 $2 $3" = "--enable goals app-server" ]
read_request() {{ IFS= read -r line; printf '%s\n' "$line" >> "$file"; }}
reply() {{ printf '%s\n' "$1"; }}
read_request
reply '{{"id":0,"result":{{}}}}'
read_request
read_request
cat <<'SKILLS_RESPONSE'
{response}
SKILLS_RESPONSE
"#
            );
            if matches!(mode, "enabled" | "unrelated_error") {
                script.push_str(r#"read_request
reply '{"id":1,"result":{"thread":{"id":"fixture-thread"}}}'
read_request
reply '{"id":2,"result":{"goal":{"status":"active"}}}'
read_request
reply '{"id":3,"result":{"turn":{"id":"first-turn"}}}'
reply '{"id":"stop","method":"item/tool/call","params":{"threadId":"fixture-thread","tool":"fixture_block","arguments":{}}}'
read_request
read_request
reply '{"id":4,"result":{"goal":{"status":"blocked"}}}'
"#);
            }
            std::fs::write(&fixture, script).unwrap();
            let prompt = "原始请求\n  keep spacing and \"quotes\"";
            let session = Session {
                root: &cwd,
                instructions: INSTRUCTIONS.into(),
                prompt,
                job: Uuid::new_v4(),
                skill: Some(&skill),
            };
            let log = Log::for_config(&root.path().join("connector.json"));
            let mut command = Command::new("/bin/sh");
            command.arg(&fixture).arg(&transcript);
            let (_stop, mut stop) = watch::channel(false);
            let result = session
                .run(&mut command, &log, &mut stop, async |_, _, _| {
                    ToolReply::error("fixture_block", "fixture", false, GoalChange::Blocked)
                })
                .await;
            let input = std::fs::read_to_string(&transcript).unwrap();
            let messages: Vec<Value> = input
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            assert_eq!(messages[2]["method"], "skills/list", "{mode}");
            assert_eq!(
                messages[2]["params"],
                json!({"cwds":[cwd],"forceReload":true})
            );
            if matches!(mode, "enabled" | "unrelated_error") {
                assert_eq!(result, Ok(()));
                let turn = messages
                    .iter()
                    .find(|message| message["method"] == "turn/start")
                    .unwrap();
                assert_eq!(
                    turn["params"]["input"],
                    json!([
                        {"type":"text","text":prompt}, {"type":"skill","name":"capture","path":skill.path}
                    ])
                );
            } else {
                assert_eq!(result, Err("codex_skill_unavailable"), "{mode}");
                assert!(
                    !messages
                        .iter()
                        .any(|message| message["method"] == "thread/start"
                            || message["method"] == "turn/start"),
                    "{mode}"
                );
            }
        }
    }

    #[tokio::test]
    async fn keeps_original_prompt_separate_and_returns_tool_errors_to_the_same_goal() {
        let root = tempfile::tempdir().unwrap();
        let fixture = root.path().join("fake-codex.sh");
        let transcript = root.path().join("input.jsonl");
        std::fs::write(&fixture, r#"#!/bin/sh
set -eu
file=$1
shift
[ "$1 $2 $3" = "--enable goals app-server" ]
read_request() {
    IFS= read -r line
    printf '%s\n' "$line" >> "$file"
}
reply() { printf '%s\n' "$1"; }
read_request
reply '{"id":0,"result":{}}'
read_request
read_request
reply '{"id":1,"result":{"thread":{"id":"fixture-thread"}}}'
read_request
reply '{"id":2,"result":{"goal":{"status":"active"}}}'
read_request
reply '{"id":3,"result":{"turn":{"id":"first-turn"}}}'
reply '{"id":"retry","method":"item/tool/call","params":{"threadId":"fixture-thread","tool":"validate_simulator","arguments":{"deviceId":"booted"}}}'
read_request
reply '{"method":"turn/completed","params":{"threadId":"fixture-thread","turn":{"status":"completed"}}}'
reply '{"method":"turn/started","params":{"threadId":"fixture-thread","turn":{"id":"second-turn"}}}'
reply '{"id":"upload","method":"item/tool/call","params":{"threadId":"fixture-thread","tool":"upload_screenshot","arguments":{"screenshotId":"fixture"}}}'
read_request
reply '{"id":"last-upload","method":"item/tool/call","params":{"threadId":"fixture-thread","tool":"upload_screenshot","arguments":{"screenshotId":"fixture-2","targetIndex":1}}}'
read_request
read_request
reply '{"id":4,"result":{"goal":{"status":"complete"}}}'
"#).unwrap();
        let prompt = "Amberia\n请截取 iPhone 首页的 \"商店图\"".repeat(170);
        let session = Session {
            root: root.path(),
            instructions: INSTRUCTIONS.into(),
            prompt: &prompt,
            job: Uuid::new_v4(),
            skill: None,
        };
        let log = Log::for_config(&root.path().join("connector.json"));
        let mut command = Command::new("/bin/sh");
        command.arg(&fixture).arg(&transcript);
        let (_stop, mut stop) = watch::channel(false);
        session
            .run(
                &mut command,
                &log,
                &mut stop,
                async |name, args, _| match name.as_str() {
                    "validate_simulator" => {
                        assert_eq!(args["deviceId"], "booted");
                        ToolReply::error(
                            "invalid_simulator_id",
                            "Provide a simulator UUID.",
                            true,
                            GoalChange::Continue,
                        )
                    }
                    "upload_screenshot" => {
                        if args["targetIndex"] == 1 {
                            ToolReply::ok(json!({"delivered":true}), GoalChange::Delivered)
                        } else {
                            ToolReply::ok(json!({"delivered":false}), GoalChange::Continue)
                        }
                    }
                    _ => panic!("Unexpected fixture tool"),
                },
            )
            .await
            .unwrap();
        let input = std::fs::read_to_string(transcript).unwrap();
        let messages: Vec<Value> = input
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(messages[2]["params"]["developerInstructions"], INSTRUCTIONS);
        assert_eq!(messages[2]["params"]["approvalPolicy"], "never");
        assert_eq!(messages[2]["params"]["sandbox"], "danger-full-access");
        assert!(messages[2]["params"].get("permissions").is_none());
        assert_eq!(messages[4]["params"]["approvalPolicy"], "never");
        assert_eq!(
            messages[4]["params"]["sandboxPolicy"],
            json!({"type":"dangerFullAccess"})
        );
        assert_eq!(messages[3]["params"]["objective"], GOAL);
        assert_eq!(messages[4]["params"]["input"][0]["text"], prompt);
        assert_eq!(messages[5]["id"], "retry");
        assert_eq!(messages[5]["result"]["success"], false);
        assert!(messages[5]["result"]["contentItems"][0]["text"]
            .as_str()
            .unwrap()
            .contains("invalid_simulator_id"));
        assert_eq!(messages[6]["result"]["success"], true);
        assert_eq!(messages[7]["id"], "last-upload");
        assert_eq!(messages[8]["params"]["status"], "complete");
        assert_eq!(messages[4]["params"]["input"].as_array().unwrap().len(), 1);
        let tools = messages[2]["params"]["dynamicTools"].as_array().unwrap();
        assert!(tools.iter().any(|tool| tool["name"] == "get_capture_task"));
    }
}
