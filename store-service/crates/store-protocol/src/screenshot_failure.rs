//! Bounded, sanitized context for screenshot failures. Never accepts prompts or raw logs.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureDetails {
    pub reason: Option<String>,
    pub native_message: Option<String>,
    pub event: Option<String>,
    pub goal_status: Option<String>,
    pub rpc_method: Option<String>,
    pub rpc_error_code: Option<i64>,
    pub last_tool: Option<String>,
    pub last_tool_error: Option<String>,
    pub exit_code: Option<i32>,
    pub diagnostic: Option<String>,
    #[serde(default)]
    pub requires_finalization: bool,
    #[serde(default)]
    pub finalization_started: bool,
}

pub fn sanitize(value: &str, secrets: &[&str], limit: usize) -> String {
    let mut value: String = value
        .chars()
        .filter(|c| !c.is_control() || c.is_whitespace())
        .take(8192)
        .collect();
    for secret in secrets.iter().filter(|s| !s.is_empty()) {
        value = value.replace(secret, "[redacted]");
    }
    let mut words = Vec::new();
    for word in value.split_whitespace() {
        let lower = word.to_ascii_lowercase();
        if [
            "-----begin",
            "authorization",
            "bearer",
            "token",
            "secret",
            "accesskey",
            "password",
            "cookie",
            "private_key",
            "privatekey",
            "api_key",
            "apikey",
            "credential",
        ]
        .iter()
        .any(|key| lower.contains(key))
        {
            words.push("[redacted]");
            break; // Do not leak the value following a sensitive label.
        }
        let opaque = word
            .split(|c: char| !(c.is_ascii_alphanumeric() || "_-+/=".contains(c)))
            .any(|part| part.len() >= 32);
        if lower.contains("://")
            || word.starts_with('/')
            || word.contains("/Users/")
            || word.contains("/home/")
            || word.contains(":\\")
            || opaque
        {
            words.push("[redacted]");
        } else {
            words.push(word);
        }
    }
    words.join(" ").chars().take(limit).collect()
}

impl FailureDetails {
    pub fn sanitized(&self, secrets: &[&str]) -> Self {
        let text = |s: &Option<String>, limit| {
            s.as_ref()
                .map(|s| sanitize(s, secrets, limit))
                .filter(|s| !s.is_empty())
        };
        Self {
            reason: text(&self.reason, 800),
            native_message: text(&self.native_message, 500),
            event: text(&self.event, 80),
            goal_status: text(&self.goal_status, 80),
            rpc_method: text(&self.rpc_method, 80),
            rpc_error_code: self.rpc_error_code,
            last_tool: text(&self.last_tool, 80),
            last_tool_error: text(&self.last_tool_error, 80),
            exit_code: self.exit_code,
            diagnostic: text(&self.diagnostic, 80),
            requires_finalization: self.requires_finalization,
            finalization_started: self.finalization_started,
        }
    }

    pub fn within_limits(&self) -> bool {
        serde_json::to_vec(self).is_ok_and(|v| v.len() <= 8192)
            && [
                &self.reason,
                &self.native_message,
                &self.event,
                &self.goal_status,
                &self.rpc_method,
                &self.last_tool,
                &self.last_tool_error,
                &self.diagnostic,
            ]
            .into_iter()
            .flatten()
            .all(|s| s.chars().count() <= 2000)
    }
}

/// Counts come from persisted task data, never from node-provided diagnostics.
pub fn summary(
    code: &str,
    details: Option<&FailureDetails>,
    targets: &Value,
    result: &Value,
) -> String {
    let base = match code {
        "codex_goal_unconfirmed" => "Codex Goal 已退出活跃状态，但截图任务尚未确认完成",
        "codex_reported_failure" => "Codex 报告截图任务无法继续",
        "codex_execution_timeout" | "screenshot_timeout" => "截图任务超过执行时限",
        "codex_turn_failed" => "Codex 执行回合失败",
        "codex_goal_stream_closed" => "Codex 连接在确认任务完成前关闭",
        "codex_interrupted" => "截图节点执行被中断",
        "profile_unavailable" => "本机 Codex 程序或配置目录不可用",
        "selection_invalid" => "模拟器选择或可用性校验未通过",
        "capture_failed" => "截图采集未完成",
        "delivery_unknown" => "图片交付结果尚未确认",
        "codex_skill_unavailable" => "Codex 未能加载任务指定的截图 Skill",
        "codex_initialize_failed" => "Codex App Server 初始化失败",
        "codex_thread_start_failed" => "Codex 任务会话创建失败",
        "codex_turn_start_failed" => "Codex 执行回合启动失败",
        "codex_goal_setup_failed" => "Codex Goal 设置失败",
        "codex_tool_reply_failed" => "向 Codex 返回工具结果失败",
        "codex_failed" => "Codex 执行未能完成",
        "invalid_execution_budget" => "中心返回的任务时限不合法",
        "screenshot_oss_not_configured" | "screenshot_upload_unconfigured" => {
            "截图存储上传接口尚未配置"
        }
        "screenshot_upload_rejected" => "截图上传被拒绝",
        "screenshot_object_mismatch" => "OSS 图片对象与预期文件信息不一致",
        "screenshot_oss_config_changed" => "任务上传期间 OSS 配置发生变更",
        "invalid_upload_ticket" => "截图上传凭证格式或内容不合法",
        "screenshot_skill_download_failed" => "截图 Skill 下载失败",
        "screenshot_skill_invalid" => "截图 Skill 内容校验未通过",
        "screenshot_skill_cache_unsafe" => "截图 Skill 缓存安全校验未通过",
        "screenshot_skill_install_failed" => "截图 Skill 本机准备失败",
        _ => "截图任务执行失败",
    };
    let mut parts = vec![base.to_owned()];
    let count = targets.as_array().map_or(1, Vec::len);
    let uploaded = result["images"].as_array().map_or(0, Vec::len);
    parts.push(format!("已确认候选图 {uploaded}/{count}"));
    let d = details.cloned().unwrap_or_default().sanitized(&[]);
    if let Some(reason) = &d.reason {
        parts.push(format!("阻塞原因：{reason}"));
    }
    if let Some(message) = &d.native_message {
        parts.push(format!("Codex 返回：{message}"));
    }
    if d.reason.is_none() && d.native_message.is_none() {
        parts.push("未提供具体原因，现有记录无法确认根因".into());
    }
    for (label, value) in [
        ("事件", d.event),
        ("Goal 状态", d.goal_status),
        ("RPC", d.rpc_method),
        ("最后工具", d.last_tool),
        ("工具错误", d.last_tool_error),
        ("诊断分类", d.diagnostic),
    ] {
        if let Some(value) = value {
            parts.push(format!("{label}：{value}"));
        }
    }
    if let Some(code) = d.rpc_error_code {
        parts.push(format!("RPC 错误码：{code}"));
    }
    if let Some(code) = d.exit_code {
        parts.push(format!("进程退出码：{code}"));
    }
    if d.requires_finalization {
        parts.push(
            if d.finalization_started {
                "整单最终提交已开始，尚未确认"
            } else {
                "整单尚未最终提交"
            }
            .into(),
        );
    }
    parts.join("；").chars().take(2000).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn details_are_bounded_and_secrets_are_removed_before_persistence() {
        for value in [
            "Missing runtime https://host/p?sig=oops",
            "bad token = real-secret",
            "/Users/private/project",
            "-----BEGIN PRIVATE KEY----- oops",
            "reason abcdefghijklmnopqrstuvwxyz0123456789",
            "blocked known-value",
        ] {
            let output = sanitize(value, &["known-value"], 800);
            for secret in [
                "?sig=oops",
                "real-secret",
                "/Users/private",
                "PRIVATE KEY",
                "abcdefghijklmnopqrstuvwxyz0123456789",
                "known-value",
            ] {
                assert!(!output.contains(secret));
            }
        }
        assert_eq!(
            sanitize("模拟器未启动\n正在检查", &[], 8),
            "模拟器未启动 正"
        );
        let d = FailureDetails {
            reason: Some("字".repeat(2001)),
            ..Default::default()
        };
        assert!(!d.within_limits());
        assert!(d.sanitized(&[]).within_limits());
        assert!(serde_json::from_value::<FailureDetails>(json!({"prompt":"secret"})).is_err());
    }

    #[test]
    fn summaries_use_actual_counts_without_inventing_a_root_cause() {
        let text = summary(
            "codex_goal_unconfirmed",
            None,
            &json!([{}, {}]),
            &json!({"images":[{}]}),
        );
        assert!(text.contains("1/2") && text.contains("无法确认根因"));
        let d = FailureDetails {
            reason: Some("缺少可用的 Android 系统镜像".into()),
            goal_status: Some("blocked".into()),
            ..Default::default()
        };
        let text = summary(
            "codex_reported_failure",
            Some(&d),
            &json!([{}]),
            &Value::Null,
        );
        assert!(text.contains("系统镜像") && text.contains("blocked"));
        assert!(!text.contains("无法确认根因"));
    }
}
