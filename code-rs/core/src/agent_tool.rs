use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write as IoWrite;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Duration as TokioDuration;
use tracing::warn;
use uuid::Uuid;

use crate::config_types::AgentConfig;
use crate::protocol::AgentInfo;
use crate::protocol::AgentSourceKind;

mod exec;
mod manager;
mod naming;
mod tool_schema;

pub(crate) use exec::execute_agent;
pub use exec::external_agent_command_exists;
pub use exec::smoke_test_agent_blocking;
pub use exec::split_command_and_args;
pub use manager::AGENT_MANAGER;
pub(crate) use manager::AgentManager;
pub use manager::AgentCreateRequest;
pub use manager::AgentStatus;
pub use manager::AgentStatusUpdatePayload;
pub(crate) use naming::normalize_agent_name;
pub use tool_schema::AgentToolRequest;
pub use tool_schema::CancelAgentParams;
pub use tool_schema::CheckAgentStatusParams;
pub use tool_schema::GetAgentResultParams;
pub use tool_schema::ListAgentsParams;
pub use tool_schema::RunAgentParams;
pub use tool_schema::WaitForAgentParams;
pub use tool_schema::create_agent_tool;
#[cfg(test)]
pub(crate) use exec::ExecuteModelRequest;
#[cfg(test)]
pub(crate) use exec::current_code_binary_path;
#[cfg(test)]
pub(crate) use exec::execute_model_with_permissions;
#[cfg(test)]
pub(crate) use exec::maybe_set_gemini_config_dir;
#[cfg(test)]
pub(crate) use exec::prefer_json_result;
#[cfg(test)]
pub(crate) use exec::resolve_program_path;
#[cfg(test)]
pub(crate) use exec::should_use_current_exe_for_agent;
#[cfg(test)]
mod tests {
    use super::normalize_agent_name;
    use super::maybe_set_gemini_config_dir;
    use super::execute_model_with_permissions;
    use super::resolve_program_path;
    use super::should_use_current_exe_for_agent;
    use super::prefer_json_result;
    use super::current_code_binary_path;
    use super::ExecuteModelRequest;
    use crate::config_types::AgentConfig;
    use code_protocol::config_types::ReasoningEffort;
    use std::collections::HashMap;
    use std::ffi::OsString;
    use tempfile::tempdir;
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::OnceLock;
    use tokio::sync::Mutex;

    #[test]
    fn drops_empty_names() {
        assert_eq!(normalize_agent_name(None), None);
        assert_eq!(normalize_agent_name(Some("   ".into())), None);
    }

    #[test]
    fn title_cases_and_restores_separators() {
        assert_eq!(
            normalize_agent_name(Some("plan_tui_refactor".into())),
            Some("Plan TUI Refactor".into())
        );
        assert_eq!(
            normalize_agent_name(Some("run-ui-tests".into())),
            Some("Run UI Tests".into())
        );
    }

    #[test]
    fn handles_camel_case_and_acronyms() {
        assert_eq!(
            normalize_agent_name(Some("shipCloudAPI".into())),
            Some("Ship Cloud API".into())
        );
    }

    #[test]
    fn prefer_json_result_uses_json_when_available() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("out.json");
        let payload = "{\"findings\":[],\"overall_explanation\":\"ok\"}";
        std::fs::write(&path, payload).unwrap();

        let res = prefer_json_result(Some(&path), Err("fallback".to_string()));
        assert_eq!(res.unwrap(), payload);
    }

    #[test]
    fn prefer_json_result_falls_back_when_missing() {
        let missing = PathBuf::from("/nonexistent/path.json");
        let res = prefer_json_result(Some(&missing), Ok("orig".to_string()));
        assert_eq!(res.unwrap(), "orig");
    }

    fn agent_with_command(command: &str) -> AgentConfig {
        AgentConfig {
            name: "code-gpt-5.2-codex".to_string(),
            command: command.to_string(),
            args: Vec::new(),
            read_only: false,
            enabled: true,
            description: None,
            env: None,
            args_read_only: None,
            args_write: None,
            instructions: None,
        }
    }

    #[test]
    fn code_family_falls_back_when_command_missing() {
        let cfg = agent_with_command("definitely-not-present-429");
        let use_current = should_use_current_exe_for_agent("code", true, Some(&cfg));
        assert!(use_current);
    }

    #[test]
    fn code_family_prefers_current_exe_even_if_coder_in_path() {
        let cfg = agent_with_command("coder");
        let use_current = should_use_current_exe_for_agent("code", false, Some(&cfg));
        assert!(use_current);
    }

    #[test]
    fn code_family_respects_custom_command_override() {
        let cfg = agent_with_command("/usr/local/bin/my-coder");
        let use_current = should_use_current_exe_for_agent("code", false, Some(&cfg));
        assert!(!use_current);
    }

    #[test]
    fn program_path_uses_current_exe_when_requested() {
        let expected = current_code_binary_path().expect("current binary path");
        let resolved = resolve_program_path(true, "coder").expect("resolved program");
        assert_eq!(resolved, expected);

        let custom = resolve_program_path(false, "custom-coder").expect("resolved custom");
        assert_eq!(custom, std::path::PathBuf::from("custom-coder"));
    }

    #[tokio::test]
    async fn read_only_agents_use_code_binary_path() {
        let _lock = env_lock().lock().await;
        let _reset_path = EnvReset::capture("PATH");
        let _reset_binary = EnvReset::capture("CODE_BINARY_PATH");

        let dir = tempdir().expect("tempdir");
        let current = script_path(dir.path(), "current");
        let shim = script_path(dir.path(), "coder");

        write_script(&current, "current");
        write_script(&shim, "path");

        unsafe {
            std::env::set_var("CODE_BINARY_PATH", &current);
            std::env::set_var("PATH", prepend_path(dir.path()));
        }

        let output = execute_model_with_permissions(ExecuteModelRequest {
            agent_id: "agent-test",
            model: "code-gpt-5.2-codex",
            prompt: "ok",
            read_only: true,
            working_dir: None,
            config: None,
            reasoning_effort: ReasoningEffort::Low,
            review_output_json_path: None,
            source_kind: None,
            log_tag: None,
        })
        .await
        .expect("execute read-only agent");

        assert_eq!(output.trim(), "current");
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvReset {
        key: &'static str,
        value: Option<OsString>,
    }

    impl EnvReset {
        fn capture(key: &'static str) -> Self {
            let value = std::env::var_os(key);
            Self { key, value }
        }
    }

    impl Drop for EnvReset {
        fn drop(&mut self) {
            unsafe {
                match &self.value {
                    Some(value) => std::env::set_var(self.key, value),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    fn prepend_path(dir: &Path) -> OsString {
        let original = std::env::var_os("PATH");
        let mut parts: Vec<OsString> = Vec::new();
        parts.push(dir.as_os_str().to_os_string());
        if let Some(orig) = original {
            parts.extend(std::env::split_paths(&orig).map(std::path::PathBuf::into_os_string));
        }
        std::env::join_paths(parts).expect("join PATH")
    }

    #[cfg(target_os = "windows")]
    fn script_path(dir: &Path, name: &str) -> PathBuf {
        dir.join(format!("{name}.cmd"))
    }

    #[cfg(not(target_os = "windows"))]
    fn script_path(dir: &Path, name: &str) -> PathBuf {
        dir.join(name)
    }

    #[cfg(target_os = "windows")]
    fn write_script(path: &Path, marker: &str) {
        let script = format!("@echo off\r\necho {marker}\r\nexit /b 0\r\n");
        std::fs::write(path, script).expect("write cmd");
    }

    #[cfg(not(target_os = "windows"))]
    fn write_script(path: &Path, marker: &str) {
        let script = format!("#!/bin/sh\necho {marker}\nexit 0\n");
        std::fs::write(path, script).expect("write script");
        let mut perms = std::fs::metadata(path)
            .expect("script metadata")
            .permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod script");
    }

    #[test]
    fn gemini_config_dir_is_injected_when_missing_api_key() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let gem_dir = tmp.path().join(".gemini");
        std::fs::create_dir_all(&gem_dir).expect("create .gemini");

        let mut env: HashMap<String, String> = HashMap::new();
        maybe_set_gemini_config_dir(&mut env, Some(tmp.path().to_string_lossy().to_string()));

        assert_eq!(
            env.get("GEMINI_CONFIG_DIR"),
            Some(&gem_dir.to_string_lossy().to_string())
        );
    }

    #[test]
    fn gemini_config_dir_not_overwritten_when_api_key_present() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut env: HashMap<String, String> = HashMap::new();
        env.insert("GEMINI_API_KEY".to_string(), "abc".to_string());

        maybe_set_gemini_config_dir(&mut env, Some(tmp.path().to_string_lossy().to_string()));

        assert!(!env.contains_key("GEMINI_CONFIG_DIR"));
    }
}
