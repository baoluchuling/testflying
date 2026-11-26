use super::{
    diagnostic::{codex_diagnostic, Log},
    skill::InstalledSkill,
    worker::{drain_stderr, finish_process, wait_for_exit},
};
use serde_json::{json, Value};
use std::sync::Mutex;
use std::{io, path::Path, process::Stdio, time::Duration};
use store_protocol::screenshot_failure::{sanitize, FailureDetails};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout, Command},
    sync::watch,
    time::{timeout, timeout_at, Instant},
};
use uuid::Uuid;

const GOAL: &str = "Fulfill the screenshot request, review the current images, and obtain confirmed final delivery using the Connector tools.";
const MAX_MESSAGE: u64 = 1024 * 1024;

pub(super) const INSTRUCTIONS: &str = concat!(
    "You own the screenshot task. Call get_capture_task first to read the task identity, center-generated jobId, app, ordered target array, requiresFinalization and current screenshot IDs. The user message is the original visual request. Fulfill targets separately without a Cartesian product. Use the zero-based targetIndex with capture_screenshot, upload_screenshot and discard_screenshot; omit it only for legacy target=null. ",
    "Find the project on this computer, prepare a suitable iOS Simulator or Android Emulator, reuse one or create and boot it as needed, and decide whether to build, install or launch the app. You may modify project source, dependencies and build configuration as needed to build and run the requested app. Preserve unrelated user changes and use the provided build output paths. ",
    "A failed build, install, launch or shell command is feedback for recovery, not a terminal task failure. Inspect the relevant diagnostics, identify the cause, repair the source, dependencies, configuration or environment as appropriate, then retry and continue this same Goal and its original targets. Choose a different valid approach when needed. Do not report failure or mark the Goal blocked merely because a command failed. Keep working within the task execution budget; only an actual blocker that remains after attempted recovery, or budget exhaustion, justifies failure. Preserve the requested app's real behavior and visual requirements while repairing it. ",
    "A missing existing simulator is not a blocker; prepare or create one, and attempt to resolve missing development tools or runtimes before declaring preparation impossible. Validate the selected simulator, navigate to the requested screen and capture it. Upload or discard the current capture before capturing another. Use discard_screenshot to reject an unsuitable local image or an already confirmed uploaded image, then fix the screen and recapture that target. ",
    "If upload has started or its outcome is unknown, first confirm upload_screenshot using the same screenshotId; do not abandon or replay an unknown upload. get_capture_task lists the screenshot IDs available for discard. When requiresFinalization=true, uploading every target is only staging: review all images, discard and retake unsuitable ones, then call finish_capture_task with an empty object. Only its delivered=true response completes this review-enabled task. If a discard or final submission is uncertain, retry that same operation before any other change. Legacy tasks with requiresFinalization=false finish when upload_screenshot returns delivered=true. ",
    "Retryable tool errors stay in this same Goal: fix or wait and retry. Only after recovery cannot succeed, report an unrecoverable blocker using report_capture_failure with the specific blocker, attempted repairs and remaining missing conditions, without secrets or full logs. Confirm pending upload, discard or final submission first. Do not mark the Goal complete before confirmed delivery. Use the provided tools for delivery and preserve the requested app screen until capture completes."
);

pub(super) const SKILL_INSTRUCTIONS: &str = "Use the attached screenshot skill for preparation, navigation and visual checks on the original captured images. Discard and retake unsuitable images before final submission. Connector tools, task identity, target order, one current capture at a time and confirmed final delivery remain authoritative. Skill product examples are historical examples; use the app and targets from get_capture_task.";

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
        {"name":"get_capture_task","description":"Read task identity, original prompt, targets, uploaded screenshot IDs, pending capture and final submission requirement.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"validate_simulator","description":"Check that the requested local simulator is booted and available.","inputSchema":device},
        {"name":"capture_screenshot","description":"Capture the visible screen of this local simulator and return a screenshotId.","inputSchema":capture},
        {"name":"upload_screenshot","description":"Upload a captured PNG and confirm that candidate. Review-enabled tasks still need finish_capture_task after all images pass review. Retry the same screenshotId when instructed.","inputSchema":{
            "type":"object","properties":{"screenshotId":{"type":"string"},"targetIndex":{"type":"integer","minimum":0}},
            "required":["screenshotId"],"additionalProperties":false
        }},
        {"name":"discard_screenshot","description":"Discard an unsuitable current local capture or confirmed uploaded candidate before final submission, then recapture that target. An uncertain upload must be confirmed first.","inputSchema":{
            "type":"object","properties":{"screenshotId":{"type":"string"},"targetIndex":{"type":"integer","minimum":0}},
            "required":["screenshotId"],"additionalProperties":false
        }},
        {"name":"finish_capture_task","description":"Finalize the task only after every current target image is uploaded and visually acceptable. Retry this tool if final submission is uncertain.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"report_capture_failure","description":"End this task only for a factual blocker that remains after attempted recovery. A failed build or command alone is not a blocker: diagnose, repair and retry in the same Goal first. State the attempted repairs and remaining missing conditions, without secrets, full prompts or logs. Pending upload, discard or final submission must be confirmed first.","inputSchema":{"type":"object","properties":{"reason":{"type":"string","minLength":1,"maxLength":2000}},"required":["reason"],"additionalProperties":false}}
    ])
}

struct Protocol {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    deadline: Instant,
}

#[derive(Debug)]
struct RpcFailure {
    code: Option<i64>,
    message: String,
}

impl std::fmt::Display for RpcFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("App Server rejected the request")
    }
}
impl std::error::Error for RpcFailure {}

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
                return message.get("result").cloned().ok_or_else(|| {
                    io::Error::other(RpcFailure {
                        code: message["error"]["code"].as_i64(),
                        message: sanitize(
                            message["error"]["message"]
                                .as_str()
                                .unwrap_or("App Server rejected the request"),
                            &[],
                            500,
                        ),
                    })
                });
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
    pub(super) failure_details: Option<&'a Mutex<FailureDetails>>,
    pub(super) execution_timeout: Option<Duration>,
    pub(super) root: &'a Path,
    pub(super) instructions: String,
    pub(super) prompt: &'a str,
    pub(super) job: Uuid,
    pub(super) skill: Option<&'a InstalledSkill>,
}

impl Session<'_> {
    fn record(&self, update: impl FnOnce(&mut FailureDetails)) {
        if let Some(details) = self.failure_details {
            update(&mut details.lock().unwrap());
        }
    }

    fn rpc_failure(&self, method: &str, error: &io::Error) {
        self.record(|d| {
            d.rpc_method = Some(method.into());
            if let Some(error) = error.get_ref().and_then(|e| e.downcast_ref::<RpcFailure>()) {
                d.rpc_error_code = error.code;
                d.native_message = Some(error.message.clone());
            }
        });
    }
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
            let exit_code = status.ok().and_then(|s| s.code());
            self.record(|d| {
                d.exit_code = exit_code;
                d.diagnostic = Some(diagnostic.into());
            });
            log.event(
                Some(self.job),
                format_args!("{stage} exit={:?} diagnostic={diagnostic}", exit_code),
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
        .map_err(|e| {
            self.rpc_failure("initialize", &e);
            "codex_initialize_failed"
        })?;
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
                .map_err(|e| {
                    self.rpc_failure("skills/list", &e);
                    "codex_skill_unavailable"
                })?;
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
            .map_err(|e| {
                self.rpc_failure("thread/start", &e);
                "codex_thread_start_failed"
            })?;
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
            .map_err(|e| {
                self.rpc_failure("thread/goal/set", &e);
                "codex_goal_setup_failed"
            })?;
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
        .map_err(|e| {
            self.rpc_failure("turn/start", &e);
            "codex_turn_start_failed"
        })?;
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
        let budget = self.execution_timeout.unwrap_or(Duration::from_secs(
            store_protocol::screenshot::LEGACY_SCREENSHOT_TIMEOUT_SECONDS,
        ));
        let limit = Instant::now() + budget;
        log.event(
            Some(self.job),
            format_args!("codex_execution_budget seconds={}", budget.as_secs()),
        );
        rpc.deadline = limit;
        loop {
            let message = tokio::select! {
                result = rpc.recv() => result.map_err(|_| if Instant::now()>=limit {"codex_execution_timeout"}else{"codex_goal_stream_closed"})?,
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
                let reply = call(name.clone(), params["arguments"].clone(), stop.clone()).await;
                self.record(|d| {
                    d.last_tool = Some(sanitize(&name, &[], 80));
                    d.last_tool_error = if reply.success {
                        None
                    } else {
                        reply.value["code"].as_str().map(|s| sanitize(s, &[], 80))
                    };
                });
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
                        self.record(|d| {
                            d.event = Some("thread/goal/updated".into());
                            d.goal_status = params["goal"]["status"]
                                .as_str()
                                .map(|s| sanitize(s, &[], 80));
                        });
                        return Err("codex_goal_unconfirmed");
                    }
                    Some("thread/goal/cleared") => {
                        self.record(|d| {
                            d.event = Some("thread/goal/cleared".into());
                            d.goal_status = Some("cleared".into());
                        });
                        return Err("codex_goal_unconfirmed");
                    }
                    Some("turn/completed") if params["turn"]["status"] == "completed" => {
                        // An active native Goal continues in a new turn on the same thread.
                        rpc.deadline = limit;
                        self.record(|d| {
                            d.native_message = None;
                            d.event = None;
                        });
                    }
                    Some("turn/completed") => {
                        self.record(|d| {
                            d.event = Some("turn/completed".into());
                            if let Some(message) = params["turn"]["error"]["message"].as_str() {
                                d.native_message = Some(sanitize(message, &[], 500));
                            }
                        });
                        return Err("codex_turn_failed");
                    }
                    Some("error") => self.record(|d| {
                        d.event = Some("error".into());
                        d.native_message = params["error"]["message"]
                            .as_str()
                            .map(|s| sanitize(s, &[], 500));
                    }),
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
    async fn native_goal_and_turn_errors_preserve_context_without_guessing_reasons() {
        for (tail,code,status,message) in [
            ("reply '{\"method\":\"thread/goal/updated\",\"params\":{\"threadId\":\"fixture-thread\",\"goal\":{\"status\":\"blocked\"}}}'","codex_goal_unconfirmed",Some("blocked"),None),
            ("reply '{\"method\":\"thread/goal/cleared\",\"params\":{\"threadId\":\"fixture-thread\"}}'","codex_goal_unconfirmed",Some("cleared"),None),
            ("reply '{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"fixture-thread\",\"turn\":{\"status\":\"failed\",\"error\":{\"message\":\"Runtime unavailable\"}}}}'","codex_turn_failed",None,Some("Runtime unavailable")),
        ] {
            let root=tempfile::tempdir().unwrap();
            let fixture=budget_fixture(root.path(),tail);
            let details=Mutex::new(FailureDetails::default());
            let session=Session {failure_details:Some(&details),execution_timeout:Some(Duration::from_secs(10)),root:root.path(),instructions:INSTRUCTIONS.into(),prompt:"Capture",job:Uuid::new_v4(),skill:None};
            let log=Log::for_config(&root.path().join("connector.json"));
            let mut command=Command::new("/bin/sh");
            command.arg(fixture);
            let (_stop,mut stop)=watch::channel(false);
            assert_eq!(session.run(&mut command,&log,&mut stop,async |_,_,_|ToolReply::ok(json!({}),GoalChange::Continue)).await,Err(code));
            let d=details.lock().unwrap();
            assert_eq!(d.goal_status.as_deref(),status);
            assert_eq!(d.native_message.as_deref(),message);
        }
    }

    #[tokio::test]
    async fn rejected_rpc_preserves_the_method_code_and_sanitized_message() {
        let root = tempfile::tempdir().unwrap();
        let fixture = root.path().join("rejected.sh");
        std::fs::write(&fixture,"read line\nprintf '%s\\n' '{\"id\":0,\"error\":{\"code\":-32000,\"message\":\"Unavailable endpoint https://example/?secret=value\"}}'\n").unwrap();
        let details = Mutex::new(FailureDetails::default());
        let session = Session {
            failure_details: Some(&details),
            execution_timeout: None,
            root: root.path(),
            instructions: INSTRUCTIONS.into(),
            prompt: "Capture",
            job: Uuid::new_v4(),
            skill: None,
        };
        let log = Log::for_config(&root.path().join("connector.json"));
        let mut command = Command::new("/bin/sh");
        command.arg(fixture);
        let (_stop, mut stop) = watch::channel(false);
        assert_eq!(
            session
                .run(
                    &mut command,
                    &log,
                    &mut stop,
                    async |_, _, _| ToolReply::ok(json!({}), GoalChange::Continue)
                )
                .await,
            Err("codex_initialize_failed")
        );
        let d = details.lock().unwrap();
        assert_eq!(d.rpc_method.as_deref(), Some("initialize"));
        assert_eq!(d.rpc_error_code, Some(-32000));
        assert_eq!(
            d.native_message.as_deref(),
            Some("Unavailable endpoint [redacted]")
        );
    }

    fn budget_fixture(root: &Path, tail: &str) -> std::path::PathBuf {
        let path = root.join("budget-codex.sh");
        let script = format!(
            r#"#!/bin/sh
set -eu
read_request() {{ IFS= read -r line; }}
reply() {{ printf '%s\n' "$1"; }}
read_request
reply '{{"id":0,"result":{{}}}}'
read_request
read_request
reply '{{"id":1,"result":{{"thread":{{"id":"fixture-thread"}}}}}}'
read_request
reply '{{"id":2,"result":{{"goal":{{"status":"active"}}}}}}'
read_request
reply '{{"id":3,"result":{{"turn":{{"id":"first-turn"}}}}}}'
{tail}
"#
        );
        std::fs::write(&path, script).unwrap();
        path
    }

    #[tokio::test]
    async fn failed_build_can_be_repaired_in_the_same_goal_before_delivery() {
        let root = tempfile::tempdir().unwrap();
        let fixture = budget_fixture(
            root.path(),
            r#"
printf '%s' broken > "$control/project-source"
if [ "$(cat "$control/project-source")" = repaired ]; then exit 9; fi
reply '{"method":"item/completed","params":{"threadId":"fixture-thread","item":{"type":"commandExecution","status":"completed","exitCode":1,"aggregatedOutput":"fixture build failed"}}}'
reply '{"id":"after-build-failure","method":"item/tool/call","params":{"threadId":"fixture-thread","tool":"get_capture_task","arguments":{}}}'
read_request
reply '{"method":"turn/completed","params":{"threadId":"fixture-thread","turn":{"status":"completed"}}}'
reply '{"method":"turn/started","params":{"threadId":"fixture-thread","turn":{"id":"repair-turn"}}}'
printf '%s' repaired > "$control/project-source"
[ "$(cat "$control/project-source")" = repaired ]
reply '{"method":"item/completed","params":{"threadId":"fixture-thread","item":{"type":"commandExecution","status":"completed","exitCode":0}}}'
reply '{"id":"finish","method":"item/tool/call","params":{"threadId":"fixture-thread","tool":"finish_capture_task","arguments":{}}}'
read_request
read_request
reply '{"id":4,"result":{"goal":{"status":"complete"}}}'
"#,
        );
        let script = std::fs::read_to_string(&fixture)
            .unwrap()
            .replace("set -eu\n", "set -eu\ncontrol=$1\n")
            .replace(
                "read_request() { IFS= read -r line; }",
                "read_request() { IFS= read -r line; printf '%s\\n' \"$line\" >> \"$control/input.jsonl\"; }",
            );
        std::fs::write(&fixture, script).unwrap();
        let session = Session {
            failure_details: None,
            execution_timeout: Some(Duration::from_secs(10)),
            root: root.path(),
            instructions: INSTRUCTIONS.into(),
            prompt: "Build this app and capture its home screen.",
            job: Uuid::new_v4(),
            skill: None,
        };
        let log = Log::for_config(&root.path().join("connector.json"));
        let mut command = Command::new("/bin/sh");
        command.arg(fixture).arg(root.path());
        let (_stop, mut stop) = watch::channel(false);
        let mut calls = Vec::new();
        let outcome = session
            .run(&mut command, &log, &mut stop, async |name, _, _| {
                calls.push(name.clone());
                let source = std::fs::read_to_string(root.path().join("project-source")).unwrap();
                match name.as_str() {
                    "get_capture_task" => {
                        assert_eq!(source, "broken");
                        ToolReply::ok(json!({"delivered":false}), GoalChange::Continue)
                    }
                    "finish_capture_task" => {
                        assert_eq!(source, "repaired");
                        ToolReply::ok(json!({"delivered":true}), GoalChange::Delivered)
                    }
                    _ => panic!("Unexpected fixture tool"),
                }
            })
            .await;
        assert_eq!(outcome, Ok(()));
        assert_eq!(calls, ["get_capture_task", "finish_capture_task"]);
        let messages: Vec<Value> = std::fs::read_to_string(root.path().join("input.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(
            messages
                .iter()
                .filter(|m| m["method"] == "thread/start")
                .count(),
            1
        );
        let goal_states: Vec<_> = messages
            .iter()
            .filter(|m| m["method"] == "thread/goal/set")
            .map(|m| m["params"]["status"].as_str().unwrap())
            .collect();
        assert_eq!(goal_states, ["active", "complete"]);
    }

    #[tokio::test]
    async fn configured_budget_reports_a_distinct_timeout_instead_of_the_old_fixed_limit() {
        let root = tempfile::tempdir().unwrap();
        let fixture = budget_fixture(root.path(), "cat >/dev/null");
        let session = Session {
            failure_details: None,
            execution_timeout: Some(Duration::from_millis(100)),
            root: root.path(),
            instructions: INSTRUCTIONS.into(),
            prompt: "Capture",
            job: Uuid::new_v4(),
            skill: None,
        };
        let log = Log::for_config(&root.path().join("connector.json"));
        let mut command = Command::new("/bin/sh");
        command.arg(fixture);
        let (_stop, mut stop) = watch::channel(false);
        assert_eq!(
            session
                .run(
                    &mut command,
                    &log,
                    &mut stop,
                    async |_, _, _| ToolReply::ok(json!({}), GoalChange::Continue)
                )
                .await,
            Err("codex_execution_timeout")
        );
        let text = std::fs::read_to_string(root.path().join("connector.log")).unwrap();
        assert!(text.contains("codex_execution_timeout"));
    }

    #[tokio::test]
    async fn completed_turn_waits_for_native_goal_continuation_beyond_thirty_seconds() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let root = tempfile::tempdir().unwrap();
        let fixture = budget_fixture(
            root.path(),
            r#"
reply '{"id":"clock","method":"item/tool/call","params":{"threadId":"fixture-thread","tool":"clock","arguments":{}}}'
read_request
reply '{"method":"turn/completed","params":{"threadId":"fixture-thread","turn":{"status":"completed"}}}'
touch "$1/turn-ended"
while [ ! -f "$1/continue" ]; do sleep 0.01; done
reply '{"method":"turn/started","params":{"threadId":"fixture-thread"}}'
reply '{"id":"done","method":"item/tool/call","params":{"threadId":"fixture-thread","tool":"done","arguments":{}}}'
read_request
read_request
reply '{"id":4,"result":{"goal":{"status":"complete"}}}'
"#,
        );
        let session = Session {
            failure_details: None,
            execution_timeout: Some(Duration::from_secs(120)),
            root: root.path(),
            instructions: INSTRUCTIONS.into(),
            prompt: "Capture",
            job: Uuid::new_v4(),
            skill: None,
        };
        let log = Log::for_config(&root.path().join("connector.json"));
        let awake = Arc::new(AtomicBool::new(true));
        let active = awake.clone();
        let keep_awake = tokio::spawn(async move {
            while active.load(Ordering::Relaxed) {
                tokio::task::yield_now().await;
            }
        });
        let path = root.path().to_owned();
        let controller = tokio::spawn(async move {
            tokio::task::spawn_blocking({
                let path = path.clone();
                move || {
                    let until = std::time::Instant::now() + Duration::from_secs(5);
                    while !path.join("turn-ended").exists() {
                        assert!(std::time::Instant::now() < until);
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            })
            .await
            .unwrap();
            tokio::time::pause();
            tokio::time::advance(Duration::from_secs(31)).await;
            for _ in 0..5 {
                tokio::task::yield_now().await;
            }
            std::fs::write(path.join("continue"), b"ready").unwrap();
            tokio::time::resume();
        });
        let mut command = Command::new("/bin/sh");
        command.arg(fixture).arg(root.path());
        let (_stop, mut stop) = watch::channel(false);
        let outcome = session
            .run(&mut command, &log, &mut stop, async |name, _, _| {
                if name == "done" {
                    ToolReply::ok(json!({"delivered":true}), GoalChange::Delivered)
                } else {
                    ToolReply::ok(json!({}), GoalChange::Continue)
                }
            })
            .await;
        controller.await.unwrap();
        awake.store(false, Ordering::Relaxed);
        keep_awake.await.unwrap();
        assert_eq!(outcome, Ok(()));
    }

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
                failure_details: None,
                execution_timeout: None,
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
            failure_details: None,
            execution_timeout: None,
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
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "discard_screenshot"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "finish_capture_task"));
    }
}
