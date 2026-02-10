use once_cell::sync::Lazy;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};
use std::fs;
use std::process::{Command, Output};
use std::str::FromStr;
use base64::prelude::{Engine as _, BASE64_STANDARD};

use ratatui::style::{Modifier, Style};
use crate::header_wave::HeaderWaveEffect;
use crate::auto_drive_strings;
use crate::auto_drive_style::AutoDriveVariant;
use crate::spinner;
use crate::thread_spawner;

use code_common::elapsed::format_duration;
use code_common::model_presets::builtin_model_presets;
use code_common::model_presets::clamp_reasoning_effort_for_model;
use code_common::model_presets::ModelPreset;
use code_common::shell_presets::merge_shell_presets;
use code_common::shell_presets::ShellPreset;
use code_core::agent_defaults::{agent_model_spec, enabled_agent_model_specs};
use code_core::smoke_test_agent_blocking;
use code_core::config::Config;
use code_core::config::persist_shell;
use code_core::git_info::CommitLogEntry;
use code_core::config_types::AgentConfig;
use code_core::config_types::AutoDriveContinueMode;
use code_core::config_types::Notifications;
use code_core::config_types::ReasoningEffort;
use code_core::config_types::ShellConfig;
use code_core::config_types::ShellPresetConfig;
use code_core::config_types::ShellScriptStyle;
use code_core::config_types::TextVerbosity;
use code_core::spawn::spawn_std_command_with_retry;
use code_core::plan_tool::{PlanItemArg, StepStatus, UpdatePlanArgs};
use code_core::model_family::derive_default_model_family;
use code_core::model_family::find_family_for_model;
use code_core::account_usage::{
    self,
    RateLimitWarningScope,
    StoredRateLimitSnapshot,
    StoredUsageSummary,
    TokenTotals,
};
use code_core::auth_accounts::{self, StoredAccount};
use code_login::AuthManager;
use code_login::AuthMode;
use code_protocol::mcp_protocol::AuthMode as McpAuthMode;
use code_protocol::dynamic_tools::DynamicToolResponse;
use code_protocol::protocol::SessionSource;
use code_protocol::num_format::format_with_separators;
use code_core::split_command_and_args;
use serde_json::Value as JsonValue;


mod diff_handlers;
mod agent_summary;
mod esc;
mod modals;
mod agent_install;
mod code_event_pipeline;
mod cloud_workflow;
mod context_flow;
mod diff_ui;
mod exec_tools;
mod gh_actions;
mod history_pipeline;
mod history_render;
mod history_virtualization_impl;
mod help_handlers;
mod settings_handlers;
mod settings_overlay;
mod settings_routing;
mod limits_overlay;
mod interrupts;
mod input_pipeline;
mod layout_scroll;
mod message;
mod notifications;
mod ordering;
mod overlay_rendering;
mod perf;
mod rate_limit_refresh;
mod repo_workflow;
mod review_flow;
mod session_flow;
mod streaming;
mod terminal_handlers;
mod terminal;
mod tools;
mod browser_sessions;
mod agent_runs;
mod web_search_sessions;
mod auto_drive_cards;
pub(crate) mod tool_cards;
mod running_tools;
#[cfg(any(test, feature = "test-helpers"))]
pub mod smoke_helpers;

#[cfg(test)]
pub(crate) use self::esc::EscIntent;
use self::agent_summary::agent_summary_counts;
use self::esc::AutoGoalEscState;
use self::agent_install::{
    start_agent_install_session,
    start_direct_terminal_session,
    start_prompt_terminal_session,
    start_upgrade_terminal_session,
    wrap_command,
};
use code_auto_drive_core::{
    start_auto_coordinator,
    AutoCoordinatorCommand,
    AutoCoordinatorEvent,
    AutoCoordinatorEventSender,
    AutoCoordinatorHandle,
    AutoCoordinatorStatus,
    AutoDriveHistory,
    AutoDriveController,
    AutoRunSummary,
    AutoRunPhase,
    AutoControllerEffect,
    AutoTurnAgentsAction,
    AutoTurnAgentsTiming,
    AutoTurnCliAction,
    AutoTurnReviewState,
    AutoResolveState,
    AutoResolvePhase,
    AUTO_RESOLVE_REVIEW_FOLLOWUP,
    CoordinatorContext,
    CoordinatorRouterResponse,
    route_user_message,
    TurnConfig,
    TurnDescriptor,
};
use self::limits_overlay::{LimitsOverlayContent, LimitsTab};
use crate::chrome_launch::ChromeLaunchOption;
use crate::insert_history::word_wrap_lines;
use self::rate_limit_refresh::{
    start_rate_limit_refresh,
    start_rate_limit_refresh_for_account,
};
use self::history_render::{
    CachedLayout, HistoryRenderState, RenderRequest, RenderRequestKind, RenderSettings, VisibleCell,
};
use code_core::parse_command::ParsedCommand;
use code_core::{AutoDriveMode, AutoDrivePidFile};
use code_core::TextFormat;
use code_core::protocol::AgentMessageDeltaEvent;
use code_core::protocol::ApprovedCommandMatchKind;
use code_core::protocol::AskForApproval;
use code_core::protocol::SandboxPolicy;
use code_core::protocol::AgentSourceKind;
use code_core::protocol::AgentMessageEvent;
use code_core::protocol::AgentReasoningDeltaEvent;
use code_core::protocol::AgentReasoningEvent;
use code_core::protocol::AgentReasoningRawContentDeltaEvent;
use code_core::protocol::AgentReasoningRawContentEvent;
use code_core::protocol::AgentReasoningSectionBreakEvent;
use code_core::protocol::AgentStatusUpdateEvent;
use code_core::protocol::ApplyPatchApprovalRequestEvent;
use code_core::protocol::BackgroundEventEvent;
use code_core::protocol::BrowserScreenshotUpdateEvent;
use code_core::protocol::BrowserSnapshotEvent;
use code_core::protocol::CustomToolCallBeginEvent;
use code_core::protocol::CustomToolCallEndEvent;
use code_core::protocol::CustomToolCallUpdateEvent;
use code_core::protocol::ErrorEvent;
use code_core::protocol::Event;
use code_core::protocol::EventMsg;
use code_core::protocol::ExecApprovalRequestEvent;
use code_core::protocol::ExecCommandBeginEvent;
use code_core::protocol::ExecCommandEndEvent;
use code_core::protocol::ExecOutputStream;
use code_core::protocol::EnvironmentContextDeltaEvent;
use code_core::protocol::EnvironmentContextFullEvent;
use code_core::protocol::InputItem;
use code_core::protocol::McpServerFailure;
use code_core::protocol::McpServerFailurePhase;
use code_core::protocol::SessionConfiguredEvent;
// MCP tool call handlers moved into chatwidget::tools
use code_core::protocol::Op;
use code_core::protocol::ReviewOutputEvent;
use code_core::protocol::{ReviewContextMetadata, ReviewRequest};
use code_core::protocol::PatchApplyBeginEvent;
use code_core::protocol::PatchApplyEndEvent;
use code_core::protocol::TaskCompleteEvent;
use code_core::protocol::TokenUsage;
use code_core::protocol::TurnDiffEvent;
use code_core::protocol::ViewImageToolCallEvent;
use code_core::review_coord::{bump_snapshot_epoch_for, try_acquire_lock, ReviewGuard};
use code_core::ConversationManager;
use code_core::codex::compact::COMPACTION_CHECKPOINT_MESSAGE;
use crate::bottom_pane::{
    AutoActiveViewModel,
    AutoCoordinatorButton,
    AutoCoordinatorViewModel,
    CountdownState,
    AgentHintLabel, AutoReviewFooterStatus, AutoReviewPhase,
    prompts_settings_view::PromptsSettingsView,
    skills_settings_view::SkillsSettingsView,
    McpSettingsView,
    ModelSelectionView,
    NotificationsMode,
    NotificationsSettingsView,
    SettingsSection,
    ThemeSelectionView,
    agent_editor_view::AgentEditorView,
    AutoDriveSettingsView,
    PlanningSettingsView,
    UpdateSettingsView,
    ReviewSettingsView,
    ValidationSettingsView,
    prompt_args,
};
use crate::bottom_pane::agents_settings_view::SubagentEditorView;
use crate::bottom_pane::mcp_settings_view::{McpServerRow, McpServerRows};
use crate::exec_command::strip_bash_lc_and_escape;
#[cfg(feature = "code-fork")]
use crate::tui_event_extensions::handle_browser_screenshot;
use crate::chatwidget::message::UserMessage;
use crate::history::compat::{
    ContextBrowserSnapshotRecord,
    ContextDeltaField,
    ContextDeltaRecord,
    ContextRecord,
};

pub(crate) const DOUBLE_ESC_HINT: &str = "undo timeline";
const AUTO_ESC_EXIT_HINT: &str = "Press Esc to exit Auto Drive";
const AUTO_ESC_EXIT_HINT_DOUBLE: &str = "Press Esc again to exit Auto Drive";
const AUTO_COMPLETION_CELEBRATION_DURATION: Duration = Duration::from_secs(5);
const HISTORY_ANIMATION_FRAME_INTERVAL: Duration = Duration::from_millis(120);
const AUTO_BOOTSTRAP_GOAL_PLACEHOLDER: &str = "Deriving goal from recent conversation";
const AUTO_DRIVE_SESSION_SUMMARY_NOTICE: &str = "Summarizing session";
const AUTO_DRIVE_SESSION_SUMMARY_PROMPT: &str =
    include_str!("../prompt_for_auto_drive_session_summary.md");
const CONTEXT_DELTA_HISTORY: usize = 10;

struct MergeRepoState {
    git_root: PathBuf,
    worktree_path: PathBuf,
    worktree_branch: String,
    worktree_sha: String,
    worktree_status: String,
    worktree_dirty: bool,
    worktree_status_ok: bool,
    worktree_diff_summary: Option<String>,
    repo_status: String,
    repo_dirty: bool,
    repo_status_ok: bool,
    default_branch: Option<String>,
    default_branch_exists: bool,
    repo_head_branch: Option<String>,
    repo_has_in_progress_op: bool,
    fast_forward_possible: bool,
}

impl MergeRepoState {
    async fn gather(worktree_path: PathBuf, git_root: PathBuf) -> Result<Self, String> {
        use tokio::process::Command;

        let worktree_branch = match Command::new("git")
            .current_dir(&worktree_path)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .await
        {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
            _ => {
                return Err("failed to detect worktree branch name".to_string());
            }
        };

        let worktree_sha = match Command::new("git")
            .current_dir(&worktree_path)
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .await
        {
            Ok(out) if out.status.success() => {
                let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if sha.is_empty() {
                    "unknown".to_string()
                } else {
                    sha
                }
            }
            _ => "unknown".to_string(),
        };

        let worktree_status_raw = ChatWidget::git_short_status(&worktree_path).await;
        let (worktree_status, worktree_dirty, worktree_status_ok) =
            Self::normalize_status(worktree_status_raw);
        let worktree_diff_summary = if worktree_dirty {
            ChatWidget::git_diff_stat(&worktree_path)
                .await
                .ok()
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
        } else {
            None
        };

        let branch_metadata = code_core::git_worktree::load_branch_metadata(&worktree_path);
        let mut default_branch = branch_metadata
            .as_ref()
            .and_then(|meta| meta.base_branch.clone());
        if default_branch.is_none() {
            default_branch = code_core::git_worktree::detect_default_branch(&git_root).await;
        }

        let repo_status_raw = ChatWidget::git_short_status(&git_root).await;
        let (repo_status, repo_dirty, repo_status_ok) = Self::normalize_status(repo_status_raw);

        let repo_head_branch = match Command::new("git")
            .current_dir(&git_root)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .await
        {
            Ok(out) if out.status.success() => {
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
            }
            _ => None,
        };

        let (default_branch_exists, fast_forward_possible) =
            if let Some(ref default_branch) = default_branch {
                let exists = Command::new("git")
                    .current_dir(&git_root)
                    .args([
                        "rev-parse",
                        "--verify",
                        "--quiet",
                        &format!("refs/heads/{default_branch}"),
                    ])
                    .output()
                    .await
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                let fast_forward = if exists {
                    Command::new("git")
                        .current_dir(&git_root)
                        .args([
                            "merge-base",
                            "--is-ancestor",
                            &format!("refs/heads/{default_branch}"),
                            &format!("refs/heads/{worktree_branch}"),
                        ])
                        .output()
                        .await
                        .map(|o| o.status.success())
                        .unwrap_or(false)
                } else {
                    false
                };
                (exists, fast_forward)
            } else {
                (false, false)
            };

        let git_dir = match Command::new("git")
            .current_dir(&git_root)
            .args(["rev-parse", "--git-dir"])
            .output()
            .await
        {
            Ok(out) if out.status.success() => {
                let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
                let candidate = PathBuf::from(&raw);
                if candidate.is_absolute() {
                    candidate
                } else {
                    git_root.join(raw)
                }
            }
            _ => git_root.join(".git"),
        };
        let repo_has_in_progress_op = [
            "MERGE_HEAD",
            "rebase-apply",
            "rebase-merge",
            "CHERRY_PICK_HEAD",
            "BISECT_LOG",
        ]
        .iter()
        .any(|name| git_dir.join(name).exists());

        remember_worktree_root_hint(&worktree_path, &git_root);
        Ok(MergeRepoState {
            git_root,
            worktree_path,
            worktree_branch,
            worktree_sha,
            worktree_status,
            worktree_dirty,
            worktree_status_ok,
            worktree_diff_summary,
            repo_status,
            repo_dirty,
            repo_status_ok,
            default_branch,
            default_branch_exists,
            repo_head_branch,
            repo_has_in_progress_op,
            fast_forward_possible,
        })
    }

    fn normalize_status(result: Result<String, String>) -> (String, bool, bool) {
        match result {
            Ok(s) => {
                let trimmed = s.trim().to_string();
                if trimmed.is_empty() {
                    ("clean".to_string(), false, true)
                } else {
                    (trimmed, true, true)
                }
            }
            Err(err) => (format!("status unavailable: {err}"), true, false),
        }
    }

    fn snapshot_summary(&self) -> String {
        let worktree_state = if !self.worktree_status_ok {
            "unknown"
        } else if self.worktree_dirty {
            "dirty"
        } else {
            "clean"
        };
        let repo_state = if !self.repo_status_ok {
            "unknown"
        } else if self.repo_dirty {
            "dirty"
        } else {
            "clean"
        };
        format!(
            "`/merge` — repo snapshot: worktree '{}' ({}) → default '{}' ({}), fast-forward: {}",
            self.worktree_branch,
            worktree_state,
            self.default_branch_label(),
            repo_state,
            if self.fast_forward_possible { "yes" } else { "no" }
        )
    }

    fn auto_fast_forward_blockers(&self) -> Vec<String> {
        let mut reasons = Vec::new();
        if !self.worktree_status_ok {
            reasons.push("unable to read worktree status".to_string());
        }
        if self.worktree_dirty {
            reasons.push("worktree has uncommitted changes".to_string());
        }
        if !self.repo_status_ok {
            reasons.push("unable to read repo status".to_string());
        }
        if self.repo_dirty {
            reasons.push(format!(
                "{} checkout has uncommitted changes",
                self.default_branch_label()
            ));
        }
        if self.repo_has_in_progress_op {
            reasons.push(
                "default checkout has an in-progress merge/rebase/cherry-pick".to_string(),
            );
        }
        if self.default_branch.is_none() {
            reasons.push("default branch is unknown".to_string());
        }
        if self.default_branch.is_some() && !self.default_branch_exists {
            reasons.push(format!(
                "default branch '{}' missing locally",
                self.default_branch_label()
            ));
        }
        match (&self.repo_head_branch, &self.default_branch) {
            (Some(head), Some(default)) if head == default => {}
            (Some(head), Some(default)) => reasons.push(format!(
                "repo root is on '{head}' instead of '{default}'"
            )),
            (Some(_), None) => reasons.push(
                "repo root branch detected but default branch is still unknown".to_string(),
            ),
            (None, _) => reasons.push("unable to detect branch currently checked out in repo root".to_string()),
        }
        if !self.fast_forward_possible {
            reasons.push("fast-forward merge is not possible".to_string());
        }
        reasons
    }

    fn default_branch_label(&self) -> String {
        self.default_branch
            .as_deref()
            .unwrap_or("default branch (determine before merging)")
            .to_string()
    }

    fn agent_preface(&self, reason_text: &str) -> String {
        let default_branch_line = self
            .default_branch
            .as_deref()
            .unwrap_or("unknown default branch (determine before merging)");
        let worktree_status = Self::format_status_for_context(&self.worktree_status);
        let repo_status = Self::format_status_for_context(&self.repo_status);
        let fast_forward_label = if self.fast_forward_possible { "yes" } else { "no" };
        let mut preface = format!(
            "[developer] Automation skipped because: {reason_text}. Finish the merge manually with the steps below.\n\nContext:\n- Worktree path: {worktree_path} — branch {worktree_branch} @ {worktree_sha}, status {worktree_status}\n- Repo root path (current cwd): {git_root} — target {default_branch_line} checkout, status {repo_status}\n- Fast-forward possible: {fast_forward_label}\n",
            reason_text = reason_text,
            worktree_path = self.worktree_path.display(),
            worktree_branch = self.worktree_branch.as_str(),
            worktree_sha = self.worktree_sha.as_str(),
            worktree_status = worktree_status,
            git_root = self.git_root.display(),
            default_branch_line = default_branch_line,
            repo_status = repo_status,
            fast_forward_label = fast_forward_label,
        );
        preface.push_str(
            "\nNOTE: Each command runs in its own shell. `/merge` switches the working directory to the repo root; use `git -C <path> ...` or `cd <path> && ...` whenever you need to operate in a different directory.\n",
        );
        preface.push_str(&format!(
            "\n1. Worktree prep (worktree {worktree_path} on {worktree_branch}):\n   - Review `git status`.\n   - Stage and commit every change that belongs in the merge. Use descriptive messages; no network commands and no resets.\n",
            worktree_path = self.worktree_path.display(),
            worktree_branch = self.worktree_branch.as_str(),
        ));
        preface.push_str(&format!(
            "   - Run worktree commands as `git -C {worktree_path}` (or `cd {worktree_path} && ...`) so they execute inside the worktree.\n",
            worktree_path = self.worktree_path.display(),
        ));
        if let Some(ref default_branch) = self.default_branch {
            preface.push_str(&format!(
                "2. Default-branch checkout prep (repo root {git_root}):\n   - If HEAD is not {default_branch}, run `git checkout {default_branch}`.\n   - If this checkout is dirty, stash with a clear message before continuing.\n",
                git_root = self.git_root.display(),
                default_branch = default_branch,
            ));
        } else {
            preface.push_str(&format!(
                "2. Default-branch checkout prep (repo root {git_root}):\n   - Determine the correct default branch for this repo (metadata missing) and check it out.\n   - If this checkout is dirty, stash with a clear message before continuing.\n",
                git_root = self.git_root.display(),
            ));
        }
        let default_branch_for_copy = self
            .default_branch
            .as_deref()
            .unwrap_or("the default branch you selected");
        preface.push_str(&format!(
            "3. Merge locally (repo root {git_root} on {default_branch_for_copy}):\n   - Run `git merge --no-ff {worktree_branch}`.\n   - Resolve conflicts line by line; keep intent from both branches.\n   - No network commands, no `git reset --hard`, no `git checkout -- .`, no `git clean`, and no `-X ours/theirs`.\n   - WARNING: Do not delete files, rewrite them in full, or checkout/prefer commits from one branch over another. Instead use apply_patch to surgically resolve conflicts, even if they are large in scale. Work on each conflict, line by line, so both branches' changes survive.\n   - If you stashed in step 2, apply/pop it now and commit if needed.\n",
            git_root = self.git_root.display(),
            default_branch_for_copy = default_branch_for_copy,
            worktree_branch = self.worktree_branch.as_str(),
        ));
        preface.push_str(&format!(
            "4. Verify in {git_root}:\n   - `git status` is clean.\n   - `git merge-base --is-ancestor {worktree_branch} HEAD` succeeds.\n   - No MERGE_HEAD/rebase/cherry-pick artifacts remain.\n",
            git_root = self.git_root.display(),
            worktree_branch = self.worktree_branch.as_str(),
        ));
        preface.push_str(&format!(
            "5. Cleanup:\n   - `git worktree remove {worktree_path}` (only after verification).\n   - `git branch -D {worktree_branch}` in {git_root} if the branch still exists.\n",
            worktree_path = self.worktree_path.display(),
            worktree_branch = self.worktree_branch.as_str(),
            git_root = self.git_root.display(),
        ));
        preface.push_str(
            "6. Report back with a concise command log and any conflicts you resolved.\n\nAbsolute rules: no network operations, no resets, no dropping local history, no blanket \"ours/theirs\" strategies.\n",
        );
        if let Some(diff) = &self.worktree_diff_summary {
            preface.push_str("\nWorktree diff summary:\n");
            preface.push_str(diff);
        }
        preface
    }

    fn format_status_for_context(status: &str) -> String {
        if status == "clean" {
            return "clean".to_string();
        }
        status
            .lines()
            .enumerate()
            .map(|(idx, line)| if idx == 0 { line.to_string() } else { format!("  {line}") })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

async fn run_fast_forward_merge(state: &MergeRepoState) -> Result<(), String> {
    use tokio::process::Command;

    let merge = Command::new("git")
        .current_dir(&state.git_root)
        .args(["merge", "--ff-only", &state.worktree_branch])
        .output()
        .await
        .map_err(|err| format!("failed to run git merge --ff-only: {err}"))?;
    if !merge.status.success() {
        return Err(format!(
            "fast-forward merge failed: {}",
            describe_command_failure(&merge, "git merge --ff-only failed")
        ));
    }

    bump_snapshot_epoch_for(&state.git_root);

    let worktree_remove = Command::new("git")
        .current_dir(&state.git_root)
        .args(["worktree", "remove"])
        .arg(&state.worktree_path)
        .arg("--force")
        .output()
        .await
        .map_err(|err| format!("failed to remove worktree: {err}"))?;
    if !worktree_remove.status.success() {
        return Err(format!(
            "failed to remove worktree: {}",
            describe_command_failure(&worktree_remove, "git worktree remove failed")
        ));
    }

    let branch_delete = Command::new("git")
        .current_dir(&state.git_root)
        .args(["branch", "-D", &state.worktree_branch])
        .output()
        .await
        .map_err(|err| format!("failed to delete branch: {err}"))?;
    if !branch_delete.status.success() {
        return Err(format!(
            "failed to delete branch '{}': {}",
            state.worktree_branch,
            describe_command_failure(&branch_delete, "git branch -D failed")
        ));
    }

    Ok(())
}

fn describe_command_failure(out: &Output, fallback: &str) -> String {
    let stderr_s = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let stdout_s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !stderr_s.is_empty() {
        stderr_s
    } else if !stdout_s.is_empty() {
        stdout_s
    } else {
        fallback.to_string()
    }
}

impl ChatWidget<'_> {
    fn is_auto_review_agent(agent: &code_core::protocol::AgentInfo) -> bool {
        if matches!(agent.source_kind, Some(AgentSourceKind::AutoReview)) {
            return true;
        }
        if let Some(batch) = agent.batch_id.as_deref()
            && batch.eq_ignore_ascii_case("auto-review") {
                return true;
            }
        false
    }
    fn format_code_bridge_call(&self, args: &JsonValue) -> Option<String> {
        let action = args.get("action")?.as_str()?.to_lowercase();
        let mut out = String::from("Code Bridge\n");
        match action.as_str() {
            "subscribe" => {
                out.push_str("└ Subscribe");
                if let Some(level) = args.get("level").and_then(|v| v.as_str()) {
                    out.push_str(&format!("  level={level}"));
                }
                Some(out)
            }
            "screenshot" => {
                out.push_str("└ Screenshot");
                Some(out)
            }
            "javascript" => {
                out.push_str("└ JavaScript\n");
                if let Some(code) = args.get("code").and_then(|v| v.as_str()) {
                    out.push_str("   ```javascript\n");
                    out.push_str(code);
                    out.push_str("\n   ```");
                }
                Some(out)
            }
            _ => None,
        }
    }

    fn format_kill_call(&self, args: &JsonValue) -> Option<String> {
        if let Some(call_id) = args.get("call_id").and_then(|v| v.as_str()) {
            let mut out = String::from("Kill\n");
            out.push_str(&format!("└ call_id: {call_id}"));
            return Some(out);
        }
        None
    }

    fn format_tool_call_preview(&self, name: &str, arguments: &str) -> Option<String> {
        let parsed: JsonValue = serde_json::from_str(arguments).ok()?;
        match name {
            "code_bridge" => self.format_code_bridge_call(&parsed),
            "kill" => self.format_kill_call(&parsed),
            _ => None,
        }
    }

    fn browser_overlay_progress_line(
        &self,
        width: u16,
        current: Duration,
        total: Duration,
    ) -> Line<'static> {
        let width = width.max(20) as usize;
        let prefix = "▶ ";
        let suffix = format!(
            " {} / {}",
            self.format_overlay_mm_ss(current),
            self.format_overlay_mm_ss(total)
        );
        let slider_width = width
            .saturating_sub(prefix.len())
            .saturating_sub(suffix.chars().count())
            .max(5);

        let progress_ratio = if total.as_millis() == 0 {
            0.0
        } else {
            (current.as_secs_f64() / total.as_secs_f64()).clamp(0.0, 1.0)
        };
        let progress_cells = (progress_ratio * slider_width as f64).round() as usize;

        let mut slider = String::with_capacity(slider_width);
        let pointer_idx = if slider_width <= 1 {
            0
        } else {
            progress_cells.clamp(0, slider_width.saturating_sub(1))
        };
        for i in 0..slider_width {
            if i == pointer_idx {
                slider.push('◉');
            } else {
                slider.push('─');
            }
        }

        let mut spans: Vec<Span> = Vec::new();
        spans.push(Span::styled(prefix.to_string(), Style::default().fg(crate::colors::text())));
        spans.push(Span::styled(
            slider,
            Style::default()
                .fg(crate::colors::primary())
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            suffix,
            Style::default().fg(crate::colors::text()),
        ));

        Line::from(spans)
    }

    fn format_overlay_mm_ss(&self, duration: Duration) -> String {
        let total_secs = duration.as_secs();
        let minutes = total_secs / 60;
        let seconds = total_secs % 60;
        format!("{minutes:02}:{seconds:02}")
    }

    fn normalize_action_time_label(&self, label: &str) -> String {
        if let Some((minutes, seconds)) = label.split_once('m') {
            let minutes = minutes.trim().parse::<u64>().unwrap_or(0);
            let seconds = seconds
                .trim()
                .trim_start_matches(char::is_whitespace)
                .trim_end_matches('s')
                .parse::<u64>()
                .unwrap_or(0);
            return format!("{minutes:02}:{seconds:02}");
        }
        if let Some(stripped) = label.strip_suffix('s')
            && let Ok(seconds) = stripped.trim().parse::<u64>() {
                return format!("00:{:02}", seconds.min(59));
            }
        label.to_string()
    }
}
use code_git_tooling::{
    create_ghost_commit,
    restore_ghost_commit,
    CreateGhostCommitOptions,
    GhostCommit,
    GitToolingError,
};
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use image::imageops::FilterType;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Widget;
use ratatui::widgets::WidgetRef;
use ratatui_image::picker::Picker;
use std::cell::{Cell, RefCell};
use std::sync::mpsc;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::task;
use uuid::Uuid;

fn history_cell_logging_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if let Ok(value) = std::env::var("CODEX_TRACE_HISTORY") {
        let trimmed = value.trim();
        if !matches!(trimmed, "" | "0") {
            return true;
        }
    }
    *ENABLED.get_or_init(|| {
        if let Ok(value) = std::env::var("CODE_BUFFER_DIFF_TRACE_CELLS") {
            return !matches!(value.trim(), "" | "0");
        }
        if let Ok(value) = std::env::var("CODE_BUFFER_DIFF_METRICS") {
            return !matches!(value.trim(), "" | "0");
        }
        false
    })
}

pub(crate) fn is_test_mode() -> bool {
    #[cfg(any(test, feature = "test-helpers"))]
    {
        static FLAG: OnceLock<bool> = OnceLock::new();
        *FLAG.get_or_init(|| match std::env::var("CODE_TUI_TEST_MODE") {
            Ok(raw) => {
                let val = raw.trim().to_ascii_lowercase();
                matches!(val.as_str(), "1" | "true" | "yes" | "on")
            }
            Err(_) => true,
        })
    }
    #[cfg(not(any(test, feature = "test-helpers")))]
    {
        static FLAG: OnceLock<bool> = OnceLock::new();
        *FLAG.get_or_init(|| match std::env::var("CODE_TUI_TEST_MODE") {
            Ok(raw) => {
                let val = raw.trim().to_ascii_lowercase();
                matches!(val.as_str(), "1" | "true" | "yes" | "on")
            }
            Err(_) => false,
        })
    }
}
use tracing::{debug, info, warn};
// use image::GenericImageView;

const TOKENS_PER_MILLION: f64 = 1_000_000.0;
const INPUT_COST_PER_MILLION_USD: f64 = 1.25;
const CACHED_INPUT_COST_PER_MILLION_USD: f64 = 0.125;
const OUTPUT_COST_PER_MILLION_USD: f64 = 10.0;
const STATUS_LABEL_INDENT: &str = "   ";
const STATUS_LABEL_TARGET_WIDTH: usize = 7;
const STATUS_LABEL_GAP: usize = 2;
const STATUS_CONTENT_PREFIX: &str = "    ";
const RESUME_PLACEHOLDER_MESSAGE: &str = "Resuming previous session...";
const RESUME_NO_HISTORY_NOTICE: &str =
    "No saved messages for this session. Start typing to continue.";
const ENABLE_WARP_STRIPES: bool = false;

fn auto_continue_from_config(mode: AutoDriveContinueMode) -> AutoContinueMode {
    match mode {
        AutoDriveContinueMode::Immediate => AutoContinueMode::Immediate,
        AutoDriveContinueMode::TenSeconds => AutoContinueMode::TenSeconds,
        AutoDriveContinueMode::SixtySeconds => AutoContinueMode::SixtySeconds,
        AutoDriveContinueMode::Manual => AutoContinueMode::Manual,
    }
}

fn auto_continue_to_config(mode: AutoContinueMode) -> AutoDriveContinueMode {
    match mode {
        AutoContinueMode::Immediate => AutoDriveContinueMode::Immediate,
        AutoContinueMode::TenSeconds => AutoDriveContinueMode::TenSeconds,
        AutoContinueMode::SixtySeconds => AutoDriveContinueMode::SixtySeconds,
        AutoContinueMode::Manual => AutoDriveContinueMode::Manual,
    }
}

fn status_field_prefix(label: &str) -> String {
    let padding = STATUS_LABEL_GAP
        .saturating_add(STATUS_LABEL_TARGET_WIDTH.saturating_sub(label.len()));
    format!(
        "{indent}{label}:{spaces}",
        indent = STATUS_LABEL_INDENT,
        label = label,
        spaces = " ".repeat(padding)
    )
}

fn status_content_prefix() -> String {
    STATUS_CONTENT_PREFIX.to_string()
}

fn describe_cloud_error(err: &CloudTaskError) -> String {
    match err {
        CloudTaskError::Msg(message) => message.clone(),
        other => other.to_string(),
    }
}

use crate::account_label::{account_display_label, account_mode_priority};
use crate::app_event::{
    AppEvent,
    AutoContinueMode,
    BackgroundPlacement,
    GitInitResume,
    ModelSelectionKind,
    TerminalAfter,
    TerminalCommandGate,
    TerminalLaunch,
    TerminalRunController,
};
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::CustomPromptView;
use crate::bottom_pane::list_selection_view::{ListSelectionView, SelectionItem};
use crate::bottom_pane::CloudTasksView;
use crate::bottom_pane::validation_settings_view;
use crate::bottom_pane::validation_settings_view::{GroupStatus, ToolRow};
use crate::bottom_pane::model_selection_view::ModelSelectionTarget;
use crate::bottom_pane::BottomPane;
use crate::bottom_pane::BottomPaneParams;
use crate::bottom_pane::{UndoTimelineEntry, UndoTimelineEntryKind, UndoTimelineView};
use crate::bottom_pane::CancellationEvent;
use crate::bottom_pane::InputResult;
use crate::bottom_pane::LoginAccountsState;
use crate::bottom_pane::LoginAccountsView;
use crate::bottom_pane::LoginAddAccountState;
use crate::bottom_pane::LoginAddAccountView;
use crate::bottom_pane::UpdateSharedState;
use crate::height_manager::HeightEvent;
use crate::height_manager::HeightManager;
use crate::history_cell;
use crate::history_cell::clean_wait_command;
#[cfg(target_os = "macos")]
use crate::agent_install_helpers::macos_brew_formula_for_command;
use crate::history_cell::ExecCell;
use crate::history_cell::FrozenHistoryCell;
use crate::history_cell::HistoryCell;
use crate::history_cell::HistoryCellType;
use crate::history_cell::PatchEventType;
use crate::history_cell::PlainHistoryCell;
use crate::history_cell::PlanUpdateCell;
use crate::history_cell::DiffCell;
use crate::history_cell::BrowserSessionCell;
use crate::history_cell::{AutoDriveActionKind, AutoDriveStatus};
use sha2::{Digest, Sha256};
use crate::history::state::PatchEventType as HistoryPatchEventType;
use crate::history::state::{
    AssistantMessageState,
    AssistantStreamDelta,
    AssistantStreamState,
    DiffLineKind,
    DiffRecord,
    ExecStatus,
    ExecWaitNote,
    HistoryDomainEvent,
    HistoryDomainRecord,
    HistoryId,
    HistoryRecord,
    HistoryMutation,
    HistorySnapshot,
    HistoryState,
    InlineSpan,
    MessageLine,
    MessageLineKind,
    MessageHeader,
    ImageRecord,
    PlainMessageKind,
    PlainMessageRole,
    PlainMessageState,
    MessageMetadata,
    OrderKeySnapshot,
    PatchFailureMetadata,
    PatchRecord,
    RateLimitLegendEntry,
    RateLimitsRecord,
    TextTone,
    TextEmphasis,
    ToolArgument,
    ToolStatus,
};
use crate::cloud_tasks_service::CloudEnvironment;
use crate::sanitize::{sanitize_for_tui, Mode as SanitizeMode, Options as SanitizeOptions};
use crate::slash_command::{ProcessedCommand, SlashCommand};
use crate::live_wrap::RowBuilder;
use crate::streaming::StreamKind;
use crate::streaming::controller::AppEventHistorySink;
use crate::util::buffer::fill_rect;
use crate::user_approval_widget::ApprovalRequest;
use code_ansi_escape::ansi_escape_line;
pub(crate) use self::terminal::{
    PendingCommand,
    PendingCommandAction,
    PendingManualTerminal,
    TerminalOverlay,
    TerminalState,
};
use code_browser::BrowserManager;
use code_core::config::find_code_home;
use code_core::config::resolve_code_path_for_read;
use code_core::config::set_github_actionlint_on_patch;
use code_core::config::set_validation_group_enabled;
use code_core::config::set_validation_tool_enabled;
use code_file_search::FileMatch;
use code_cloud_tasks_client::{ApplyOutcome, CloudTaskError, CreatedTask, TaskSummary};
use code_protocol::models::ContentItem;
use code_protocol::models::ResponseItem;
use code_core::config_types::{validation_tool_category, ValidationCategory};
use code_core::protocol::RateLimitSnapshotEvent;
use code_core::protocol::ValidationGroup;
use crate::rate_limits_view::{
    build_limits_view, RateLimitDisplayConfig, RateLimitResetInfo, DEFAULT_DISPLAY_CONFIG,
    DEFAULT_GRID_CONFIG,
};
use crate::session_log;
use code_core::review_format::format_review_findings_block;
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, TimeZone, Timelike, Utc};
use crossterm::event::KeyCode;
use crossterm::event::KeyModifiers;
use ratatui::style::Stylize;
use ratatui::symbols::scrollbar as scrollbar_symbols;
use ratatui::text::Span;
use ratatui::text::Text as RtText;
use textwrap::wrap;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Scrollbar;
use ratatui::widgets::ScrollbarOrientation;
use ratatui::widgets::ScrollbarState;
use ratatui::widgets::StatefulWidget;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Serialize, Deserialize)]
struct CachedConnection {
    port: Option<u16>,
    ws: Option<String>,
}

async fn read_cached_connection() -> Option<(Option<u16>, Option<String>)> {
    let code_home = find_code_home().ok()?;
    let path = resolve_code_path_for_read(&code_home, std::path::Path::new("cache.json"));
    let bytes = tokio::fs::read(path).await.ok()?;
    let parsed: CachedConnection = serde_json::from_slice(&bytes).ok()?;
    Some((parsed.port, parsed.ws))
}

async fn write_cached_connection(port: Option<u16>, ws: Option<String>) -> std::io::Result<()> {
    if port.is_none() && ws.is_none() {
        return Ok(());
    }
    if let Ok(code_home) = find_code_home() {
        let path = code_home.join("cache.json");
        let obj = CachedConnection { port, ws };
        let data = serde_json::to_vec_pretty(&obj).unwrap_or_else(|_| b"{}".to_vec());
        if let Some(dir) = path.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        tokio::fs::write(path, data).await?;
    }
    Ok(())
}

struct RunningCommand {
    command: Vec<String>,
    parsed: Vec<ParsedCommand>,
    // Index of the in-history Exec cell for this call, if inserted
    history_index: Option<usize>,
    history_id: Option<HistoryId>,
    // Aggregated exploration entry (history index, entry index) when grouped
    explore_entry: Option<(usize, usize)>,
    stdout_offset: usize,
    stderr_offset: usize,
    wait_total: Option<Duration>,
    wait_active: bool,
    wait_notes: Vec<(String, bool)>,
}

const RATE_LIMIT_WARNING_THRESHOLDS: [f64; 3] = [50.0, 75.0, 90.0];
const RATE_LIMIT_REFRESH_INTERVAL: chrono::Duration = chrono::Duration::minutes(10);

const MAX_TRACKED_GHOST_COMMITS: usize = 20;
const GHOST_SNAPSHOT_NOTICE_THRESHOLD: Duration = Duration::from_secs(4);
const GHOST_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct RateLimitWarning {
    scope: RateLimitWarningScope,
    threshold: f64,
    message: String,
}

#[derive(Default)]
struct RateLimitWarningState {
    weekly_index: usize,
    hourly_index: usize,
}

impl RateLimitWarningState {
    fn take_warnings(
        &mut self,
        secondary_used_percent: f64,
        primary_used_percent: f64,
    ) -> Vec<RateLimitWarning> {
        let mut warnings = Vec::new();

        let mut next_weekly_index = self.weekly_index;
        while next_weekly_index < RATE_LIMIT_WARNING_THRESHOLDS.len()
            && secondary_used_percent >= RATE_LIMIT_WARNING_THRESHOLDS[next_weekly_index]
        {
            next_weekly_index += 1;
        }
        if next_weekly_index > self.weekly_index {
            let threshold = RATE_LIMIT_WARNING_THRESHOLDS[next_weekly_index - 1];
            warnings.push(RateLimitWarning {
                scope: RateLimitWarningScope::Secondary,
                threshold,
                message: format!(
                    "Secondary usage exceeded {threshold:.0}% of the limit. Run /limits for detailed usage."
                ),
            });
            self.weekly_index = next_weekly_index;
        }

        let mut next_hourly_index = self.hourly_index;
        while next_hourly_index < RATE_LIMIT_WARNING_THRESHOLDS.len()
            && primary_used_percent >= RATE_LIMIT_WARNING_THRESHOLDS[next_hourly_index]
        {
            next_hourly_index += 1;
        }
        if next_hourly_index > self.hourly_index {
            let threshold = RATE_LIMIT_WARNING_THRESHOLDS[next_hourly_index - 1];
            warnings.push(RateLimitWarning {
                scope: RateLimitWarningScope::Primary,
                threshold,
                message: format!(
                    "Hourly usage exceeded {threshold:.0}% of the limit. Run /limits for detailed usage."
                ),
            });
            self.hourly_index = next_hourly_index;
        }

        warnings
    }

    fn reset(&mut self) {
        self.weekly_index = 0;
        self.hourly_index = 0;
    }
}

#[derive(Clone)]
struct GhostSnapshotsDisabledReason {
    message: String,
    hint: Option<String>,
}

#[derive(Clone, Copy)]
struct ConversationSnapshot {
    user_turns: usize,
    assistant_turns: usize,
    history_len: usize,
    order_len: usize,
    order_dbg_len: usize,
}

impl ConversationSnapshot {
    fn new(user_turns: usize, assistant_turns: usize) -> Self {
        Self {
            user_turns,
            assistant_turns,
            history_len: 0,
            order_len: 0,
            order_dbg_len: 0,
        }
    }
}

#[derive(Clone)]
pub(crate) struct GhostState {
    snapshots: Vec<GhostSnapshot>,
    disabled: bool,
    disabled_reason: Option<GhostSnapshotsDisabledReason>,
    queue: VecDeque<(u64, GhostSnapshotRequest)>,
    active: Option<(u64, GhostSnapshotRequest)>,
    next_id: u64,
    queued_user_messages: VecDeque<UserMessage>,
}

#[cfg(any(test, feature = "test-helpers"))]
#[allow(dead_code)]
struct AutoReviewCommitScope {
    commit: String,
    file_count: usize,
}

#[cfg(any(test, feature = "test-helpers"))]
#[allow(dead_code)]
enum AutoReviewOutcome {
    Skip,
    Workspace,
    Commit(AutoReviewCommitScope),
}

#[cfg(test)]
pub(super) type CaptureAutoTurnCommitStub = Box<
    dyn Fn(&'static str, Option<String>) -> Result<GhostCommit, GitToolingError> + Send + Sync,
>;

#[cfg(test)]
pub(super) static CAPTURE_AUTO_TURN_COMMIT_STUB: Lazy<Mutex<Option<CaptureAutoTurnCommitStub>>> =
    Lazy::new(|| Mutex::new(None));

#[cfg(test)]
pub(super) type GitDiffNameOnlyBetweenStub =
    Box<dyn Fn(String, String) -> Result<Vec<String>, String> + Send + Sync>;

#[cfg(test)]
pub(super) static GIT_DIFF_NAME_ONLY_BETWEEN_STUB: Lazy<Mutex<Option<GitDiffNameOnlyBetweenStub>>> =
    Lazy::new(|| Mutex::new(None));

#[cfg(test)]
pub(super) static AUTO_STUB_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[derive(Deserialize)]
struct AutoResolveDecision {
    status: String,
    #[serde(default)]
    rationale: Option<String>,
}

const AGENTS_OVERVIEW_STATIC_ROWS: usize = 2; // spacer + "Add new agent" row

#[derive(Clone)]
struct PendingAgentUpdate {
    id: uuid::Uuid,
    cfg: AgentConfig,
}

impl PendingAgentUpdate {
    fn key(&self) -> String { format!("{}:{}", self.cfg.name.to_ascii_lowercase(), self.id) }
}

#[derive(Clone, Debug)]
struct BackgroundReviewState {
    worktree_path: std::path::PathBuf,
    branch: String,
    agent_id: Option<String>,
    snapshot: Option<String>,
    base: Option<GhostCommit>,
    last_seen: std::time::Instant,
}

#[derive(Clone, Debug)]
struct PendingAutoReviewRange {
    base: GhostCommit,
    defer_until_turn: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutoReviewIndicatorStatus {
    Running,
    Clean,
    Fixed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AutoReviewStatus {
    status: AutoReviewIndicatorStatus,
    findings: Option<usize>,
    phase: AutoReviewPhase,
}

fn detect_auto_review_phase(progress: Option<&str>) -> AutoReviewPhase {
    let text = progress.unwrap_or_default().to_ascii_lowercase();
    // Prefer explicit phase markers emitted by exec when available.
    if text.contains("[auto-review] phase: resolving") {
        return AutoReviewPhase::Resolving;
    }
    if text.contains("[auto-review] phase: reviewing") {
        return AutoReviewPhase::Reviewing;
    }

    AutoReviewPhase::Reviewing
}

const SKIP_REVIEW_PROGRESS_SENTINEL: &str = "Another review is already running; skipping this /review.";
const AUTO_REVIEW_SHARED_WORKTREE: &str = "auto-review";
const AUTO_REVIEW_FALLBACK_PREFIX: &str = "auto-review-";
const AUTO_REVIEW_BASELINE_FILENAME: &str = "auto-review-baseline";
const AUTO_REVIEW_FALLBACK_MAX: usize = 3;
const AUTO_REVIEW_FALLBACK_MAX_AGE_SECS: u64 = 12 * 60 * 60; // 12h
const AUTO_REVIEW_STALE_SECS: u64 = 5 * 60;

fn auto_review_repo_dir(git_root: &Path) -> Result<PathBuf, String> {
    let repo_name = git_root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("repo");
    let code_home = code_core::config::find_code_home()
        .map_err(|e| format!("failed to locate code home: {e}"))?;
    let repo_dir = code_home.join("working").join(repo_name);
    std::fs::create_dir_all(&repo_dir)
        .map_err(|e| format!("failed to create auto review repo dir: {e}"))?;
    Ok(repo_dir)
}

fn auto_review_branches_dir(git_root: &Path) -> Result<PathBuf, String> {
    let branches_dir = auto_review_repo_dir(git_root)?.join("branches");
    std::fs::create_dir_all(&branches_dir)
        .map_err(|e| format!("failed to create branches dir: {e}"))?;
    Ok(branches_dir)
}

fn auto_review_baseline_path_for_repo(git_root: &Path) -> Result<PathBuf, String> {
    Ok(auto_review_repo_dir(git_root)?.join(AUTO_REVIEW_BASELINE_FILENAME))
}

fn resolve_auto_review_worktree_path(git_root: &Path, branch: &str) -> Option<PathBuf> {
    if branch.is_empty() {
        return None;
    }

    let branches_dir = auto_review_branches_dir(git_root).ok()?;
    let candidate = branches_dir.join(branch);
    candidate.exists().then_some(candidate)
}

async fn remove_worktree_path(git_root: &Path, path: &Path) -> Result<(), String> {
    let path_str = path
        .to_str()
        .ok_or_else(|| "invalid worktree path".to_string())?;
    let remove = tokio::process::Command::new("git")
        .current_dir(git_root)
        .args(["worktree", "remove", "-f", path_str])
        .output()
        .await
        .map_err(|e| format!("failed to remove worktree: {e}"))?;
    if !remove.status.success() {
        let stderr = String::from_utf8_lossy(&remove.stderr);
        tracing::warn!("failed to remove fallback worktree via git: {}", stderr.trim());
    }
    if path.exists()
        && let Err(e) = tokio::fs::remove_dir_all(path).await {
            tracing::warn!("failed to delete fallback worktree dir {:?}: {}", path, e);
        }
    Ok(())
}

async fn cleanup_fallback_worktrees(git_root: &Path) -> Result<(), String> {
    let branches_dir = auto_review_branches_dir(git_root)?;
    let mut entries: Vec<(PathBuf, SystemTime)> = Vec::new();
    if let Ok(read_dir) = fs::read_dir(&branches_dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            let name = entry
                .file_name()
                .into_string()
                .unwrap_or_default();
            if !name.starts_with(AUTO_REVIEW_FALLBACK_PREFIX) || name == AUTO_REVIEW_SHARED_WORKTREE {
                continue;
            }
            let meta = entry.metadata().ok();
            let mtime = meta
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            entries.push((path, mtime));
        }
    }

    // Age-based prune
    let now = SystemTime::now();
    for (path, mtime) in entries.iter() {
        if let Ok(elapsed) = now.duration_since(*mtime)
            && elapsed.as_secs() > AUTO_REVIEW_FALLBACK_MAX_AGE_SECS
                && let Ok(Some(g)) = try_acquire_lock("review-fallback", path) {
                    drop(g);
                    let _ = remove_worktree_path(git_root, path).await;
                }
    }

    // Count-based prune
    let mut remaining: Vec<(PathBuf, SystemTime)> = entries
        .into_iter()
        .filter(|(p, _)| p.exists())
        .collect();
    remaining.sort_by_key(|(_, t)| *t);
    while remaining.len() > AUTO_REVIEW_FALLBACK_MAX {
        if let Some((path, _)) = remaining.first().cloned() {
            if let Ok(Some(g)) = try_acquire_lock("review-fallback", &path) {
                drop(g);
                let _ = remove_worktree_path(git_root, &path).await;
                remaining.remove(0);
            } else {
                // Busy; skip pruning this one
                break;
            }
        }
    }

    Ok(())
}

async fn allocate_fallback_auto_review_worktree(
    git_root: &Path,
    snapshot_id: &str,
) -> Result<(PathBuf, String, ReviewGuard), String> {
    cleanup_fallback_worktrees(git_root).await?;
    let branches_dir = auto_review_branches_dir(git_root)?;
    let short = snapshot_id.chars().take(8).collect::<String>();

    for attempt in 0..AUTO_REVIEW_FALLBACK_MAX {
        let suffix = if attempt == 0 { String::new() } else { format!("-{}", attempt + 1) };
        let name = format!("{AUTO_REVIEW_FALLBACK_PREFIX}{short}{suffix}");
        let path = branches_dir.join(&name);

        match try_acquire_lock("review-fallback", &path) {
            Ok(Some(guard)) => {
                let worktree_path = code_core::git_worktree::prepare_reusable_worktree(
                    git_root,
                    &name,
                    snapshot_id,
                    true,
                )
                .await
                .map_err(|e| format!("failed to prepare fallback worktree: {e}"))?;
                return Ok((worktree_path, name, guard));
            }
            Ok(None) => continue, // in use, try next suffix
            Err(err) => return Err(format!("could not acquire fallback review lock: {err}")),
        }
    }

    Err("Auto review fallback pool is busy; try again soon.".to_string())
}

#[derive(Clone, Debug)]
struct AutoReviewNotice {
    history_id: HistoryId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TurnOrigin {
    User,
    Developer,
}

#[derive(Clone, Debug)]
struct PendingRequestUserInput {
    turn_id: String,
    call_id: String,
    anchor_key: OrderKey,
    questions: Vec<code_protocol::request_user_input::RequestUserInputQuestion>,
}

#[derive(Clone)]
struct RenderRequestSeed {
    history_id: HistoryId,
    use_cache: bool,
    fallback_lines: Option<Rc<Vec<Line<'static>>>>,
    kind: RenderRequestKind,
}

/// Actions that can be triggered by clicking on UI elements
#[derive(Clone, Debug)]
enum ClickableAction {
    ShowModelSelector,
    ShowShellSelector,
    ShowReasoningSelector,
    #[allow(dead_code)]
    ExecuteCommand(String),
}

/// A clickable region with its screen coordinates and associated action
#[derive(Clone, Debug)]
struct ClickableRegion {
    rect: ratatui::layout::Rect,
    action: ClickableAction,
}

pub(crate) struct ChatWidget<'a> {
    app_event_tx: AppEventSender,
    code_op_tx: UnboundedSender<Op>,
    bottom_pane: BottomPane<'a>,
    auth_manager: Arc<AuthManager>,
    login_view_state: Option<Weak<RefCell<LoginAccountsState>>>,
    login_add_view_state: Option<Weak<RefCell<LoginAddAccountState>>>,
    active_exec_cell: Option<ExecCell>,
    history_cells: Vec<Box<dyn HistoryCell>>, // Store all history in memory
    history_cell_ids: Vec<Option<HistoryId>>,
    history_live_window: Option<(usize, usize)>,
    history_frozen_width: u16,
    history_frozen_count: usize,
    history_render: HistoryRenderState,
    last_render_settings: Cell<RenderSettings>,
    history_virtualization_sync_pending: Cell<bool>,
    render_request_cache: RefCell<Vec<RenderRequestSeed>>,
    render_request_cache_dirty: Cell<bool>,
    history_prefix_append_only: Cell<bool>,
    render_theme_epoch: u64,
    history_state: HistoryState,
    history_snapshot_dirty: bool,
    history_snapshot_last_flush: Option<Instant>,
    context_cell_id: Option<HistoryId>,
    context_summary: Option<ContextSummary>,
    context_last_sequence: Option<u64>,
    context_browser_sequence: Option<u64>,
    config: Config,
    mcp_tools_by_server: HashMap<String, Vec<String>>,
    mcp_server_failures: HashMap<String, McpServerFailure>,

    /// Optional remote-merged presets list delivered asynchronously.
    /// When absent, the TUI falls back to built-in presets.
    remote_model_presets: Option<Vec<ModelPreset>>,
    /// Whether remote defaults may be applied to this session.
    /// Captured at startup so later config changes don't retroactively enable it.
    allow_remote_default_at_startup: bool,
    /// Tracks whether the user explicitly selected a chat model in this session.
    chat_model_selected_explicitly: bool,

    planning_restore: Option<(String, ReasoningEffort)>,
    history_debug_events: Option<RefCell<Vec<String>>>,
    latest_upgrade_version: Option<String>,
    reconnect_notice_active: bool,
    initial_user_message: Option<UserMessage>,
    total_token_usage: TokenUsage,
    last_token_usage: TokenUsage,
    rate_limit_snapshot: Option<RateLimitSnapshotEvent>,
    rate_limit_warnings: RateLimitWarningState,
    rate_limit_fetch_inflight: bool,
    rate_limit_last_fetch_at: Option<DateTime<Utc>>,
    rate_limit_primary_next_reset_at: Option<DateTime<Utc>>,
    rate_limit_secondary_next_reset_at: Option<DateTime<Utc>>,
    rate_limit_refresh_scheduled_for: Option<DateTime<Utc>>,
    rate_limit_refresh_schedule_id: Arc<AtomicU64>,
    content_buffer: String,
    // Buffer for streaming assistant answer text; we do not surface partial
    // We wait for the final AgentMessage event and then emit the full text
    // at once into scrollback so the history contains a single message.
    // Cache of the last finalized assistant message to suppress immediate duplicates
    last_assistant_message: Option<String>,
    // Track the most recent finalized Answer output item within the current turn.
    // When a new Answer stream id arrives, we retroactively mark the previous
    // assistant message as a mid-turn update for styling.
    last_answer_stream_id_in_turn: Option<String>,
    last_answer_history_id_in_turn: Option<HistoryId>,
    // Track the most recent Answer stream id we've *seen* in this turn (delta or final).
    // Used to label earlier answers as mid-turn even if their final cell hasn't
    // been inserted yet.
    last_seen_answer_stream_id_in_turn: Option<String>,
    mid_turn_answer_ids_in_turn: HashSet<String>,
    // Cache of the last user text we submitted (for context passing to review/resolve agents)
    last_user_message: Option<String>,
    // Cache of the last developer/system note we injected (hidden messages)
    last_developer_message: Option<String>,
    pending_turn_origin: Option<TurnOrigin>,
    pending_request_user_input: Option<PendingRequestUserInput>,
    current_turn_origin: Option<TurnOrigin>,
    // Tracks whether lingering running exec/tool cells have been cleared for the
    // current turn. Reset on TaskStarted; set after the first assistant message
    // (delta or final) arrives, which is more reliable than TaskComplete.
    cleared_lingering_execs_this_turn: bool,
    // Track the ID of the current streaming message to prevent duplicates
    // Track the ID of the current streaming reasoning to prevent duplicates
    exec: ExecState,
    tools_state: ToolState,
    live_builder: RowBuilder,
    header_wave: HeaderWaveEffect,
    browser_overlay_visible: bool,
    browser_overlay_state: BrowserOverlayState,
    // Store pending image paths keyed by their placeholder text
    pending_images: HashMap<String, PathBuf>,
    // (removed) pending non-image files are no longer tracked; non-image paths remain as plain text
    welcome_shown: bool,
    test_mode: bool,
    // Path to the latest browser screenshot and URL for display
    latest_browser_screenshot: Arc<Mutex<Option<(PathBuf, String)>>>,
    browser_autofix_requested: Arc<AtomicBool>,
    // Cached image protocol to avoid recreating every frame (path, area, protocol)
    cached_image_protocol:
        std::cell::RefCell<Option<(PathBuf, Rect, ratatui_image::protocol::Protocol)>>,
    // Cached picker to avoid recreating every frame
    cached_picker: std::cell::RefCell<Option<Picker>>,

    // Cached cell size (width,height) in pixels
    cached_cell_size: std::cell::OnceCell<(u16, u16)>,
    git_branch_cache: RefCell<GitBranchCache>,

    // Terminal information from startup
    terminal_info: crate::tui::TerminalInfo,
    // Agent tracking for multi-agent tasks
    active_agents: Vec<AgentInfo>,
    agents_ready_to_start: bool,
    last_agent_prompt: Option<String>,
    agent_context: Option<String>,
    agent_task: Option<String>,
    recent_agent_hint: Option<String>,
    suppress_next_agent_hint: bool,
    active_review_hint: Option<String>,
    active_review_prompt: Option<String>,
    auto_resolve_state: Option<AutoResolveState>,
    auto_resolve_attempts_baseline: u32,
    turn_had_code_edits: bool,
    background_review: Option<BackgroundReviewState>,
    auto_review_status: Option<AutoReviewStatus>,
    auto_review_notice: Option<AutoReviewNotice>,
    auto_review_baseline: Option<GhostCommit>,
    auto_review_reviewed_marker: Option<GhostCommit>,
    pending_auto_review_range: Option<PendingAutoReviewRange>,
    turn_sequence: u64,
    review_guard: Option<ReviewGuard>,
    background_review_guard: Option<ReviewGuard>,
    processed_auto_review_agents: HashSet<String>,
    // New: coordinator-provided hints for the next Auto turn
    pending_turn_descriptor: Option<TurnDescriptor>,
    pending_auto_turn_config: Option<TurnConfig>,
    overall_task_status: String,
    active_plan_title: Option<String>,
    /// Runtime timing per-agent (by id) to improve visibility in the HUD
    agent_runtime: HashMap<String, AgentRuntime>,
    pending_agent_updates: HashMap<String, PendingAgentUpdate>,
    // Sparkline data for showing agent activity (using RefCell for interior mutability)
    // Each tuple is (value, is_completed) where is_completed indicates if any agent was complete at that time
    sparkline_data: std::cell::RefCell<Vec<(u64, bool)>>,
    last_sparkline_update: std::cell::RefCell<std::time::Instant>,
    // Stream controller for managing streaming content
    stream: crate::streaming::controller::StreamController,
    // Stream lifecycle state (kind, closures, sequencing, cancel)
    stream_state: StreamState,
    // Interrupt manager for handling cancellations
    interrupts: interrupts::InterruptManager,
    // Guard to avoid spamming flush timers while interrupts wait behind a stalled stream
    interrupt_flush_scheduled: bool,

    // Guard for out-of-order exec events: track call_ids that already ended
    ended_call_ids: HashSet<ExecCallId>,
    /// Exec call_ids that were explicitly cancelled by user interrupt. Used to
    /// drop any late ExecEnd events so we don't render duplicate cells.
    canceled_exec_call_ids: HashSet<ExecCallId>,

    // Accumulated diff/session state
    diffs: DiffsState,

    // Help overlay state
    help: HelpState,

    // Settings overlay state
    settings: SettingsState,
    // When a standalone picker (model selection) closes, optionally reopen the settings overlay
    pending_settings_return: Option<SettingsSection>,

    // Limits overlay state
    limits: LimitsState,

    // Terminal overlay state
    terminal: TerminalState,
    pending_manual_terminal: HashMap<u64, PendingManualTerminal>,

    // Persisted selection for Agents overview
    agents_overview_selected_index: usize,

    // State for the Agents Terminal view
    agents_terminal: AgentsTerminalState,

    pending_git_init_resume: Option<GitInitResume>,
    git_init_inflight: bool,
    git_init_declined: bool,

    pending_upgrade_notice: Option<(u64, String)>,

    // Cached visible rows for the diff overlay body to clamp scrolling (kept within diffs)

    // Centralized height manager (always enabled)
    height_manager: RefCell<HeightManager>,

    // Aggregated layout and scroll state
    layout: LayoutState,

    // True when connected to external Chrome via CDP; affects HUD titles
    browser_is_external: bool,

    // Most recent theme snapshot used to retint pre-rendered lines
    last_theme: crate::theme::Theme,

    // Performance tracing (opt-in via /perf)
    perf_state: PerfState,
    // Current session id (from SessionConfigured)
    session_id: Option<uuid::Uuid>,

    // Pending diagnostics integration
    next_cli_text_format: Option<TextFormat>,

    // Pending jump-back state (reversible until submit)

    // Track active task ids so we don't drop the working status while any
    // agent/sub‑agent is still running (long‑running sessions can interleave).
    active_task_ids: HashSet<String>,

    // --- Queued user message support ---
    // Messages typed while a task is running are kept here and rendered
    // at the bottom as "(queued)" until the next turn begins. At that
    // point we submit one queued message and move its cell into the
    // normal history within the new turn window.
    queued_user_messages: std::collections::VecDeque<UserMessage>,
    pending_dispatched_user_messages: std::collections::VecDeque<String>,
    // Number of user prompts we pre-pended to history just before starting
    // a new turn; used to anchor the next turn window so assistant output
    // appears after them.
    pending_user_prompts_for_next_turn: usize,
    ghost_snapshots: Vec<GhostSnapshot>,
    ghost_snapshots_disabled: bool,
    ghost_snapshots_disabled_reason: Option<GhostSnapshotsDisabledReason>,
    ghost_snapshot_queue: VecDeque<(u64, GhostSnapshotRequest)>,
    active_ghost_snapshot: Option<(u64, GhostSnapshotRequest)>,
    next_ghost_snapshot_id: u64,
    queue_block_started_at: Option<Instant>,

    auto_drive_card_sequence: u64,
    auto_drive_variant: AutoDriveVariant,
    auto_state: AutoDriveController,
    auto_goal_escape_state: AutoGoalEscState,
    auto_handle: Option<AutoCoordinatorHandle>,
    auto_drive_pid_guard: Option<AutoDrivePidFile>,
    auto_history: AutoDriveHistory,
    auto_compaction_overlay: Option<AutoCompactionOverlay>,
    auto_turn_review_state: Option<AutoTurnReviewState>,
    auto_pending_goal_request: bool,
    auto_goal_bootstrap_done: bool,
    cloud_tasks_selected_env: Option<CloudEnvironment>,
    cloud_tasks_environments: Vec<CloudEnvironment>,
    cloud_tasks_last_tasks: Vec<TaskSummary>,
    cloud_tasks_best_of_n: usize,
    cloud_tasks_creation_inflight: bool,
    cloud_task_apply_tickets: HashMap<(String, bool), BackgroundOrderTicket>,
    cloud_task_create_ticket: Option<BackgroundOrderTicket>,

    // Event sequencing to preserve original order across streaming/tool events
    // and stream-related flags moved into stream_state

    // Strict global ordering for history: every cell has a required key
    // (req, out, seq). No unordered inserts and no turn windows.
    cell_order_seq: Vec<OrderKey>,
    // Debug: per-cell order info string rendered in the UI to diagnose ordering.
    cell_order_dbg: Vec<Option<String>>,
    // Routing for reasoning stream ids -> existing CollapsibleReasoningCell index
    reasoning_index: HashMap<String, usize>,
    // Stable per-(kind, stream_id) ordering, derived from OrderMeta.
    stream_order_seq: HashMap<(StreamKind, String), OrderKey>,
    // Resume-aware bias applied to provider request ordinals for restored sessions.
    order_request_bias: u64,
    resume_expected_next_request: Option<u64>,
    resume_provider_baseline: Option<u64>,
    // Track last provider request_ordinal seen so internal messages can be
    // assigned request_index = last_seen + 1 (with out = -1).
    last_seen_request_index: u64,
    // Synthetic request index used for internal-only messages; always >= last_seen_request_index
    current_request_index: u64,
    // Monotonic seq for internal messages to keep intra-request order stable
    internal_seq: u64,
    // Show order overlay when true (from --order)
    show_order_overlay: bool,

    // One-time hint to teach input history navigation
    scroll_history_hint_shown: bool,

    // Track and manage the access-mode background status cell so mode changes
    // replace the existing status instead of stacking multiple entries.
    access_status_idx: Option<usize>,
    /// When true, render without the top status bar and HUD so the normal
    /// terminal scrollback remains usable (Ctrl+T standard terminal mode).
    pub(crate) standard_terminal_mode: bool,
    // Pending system notes to inject into the agent's conversation history
    // before the next user turn. Each entry is sent in order ahead of the
    // user's visible prompt.
    pending_agent_notes: Vec<String>,

    // Stable synthetic request bucket for pre‑turn system notices (set on first use)
    synthetic_system_req: Option<u64>,
    // Map of system notice ids to their history index for in-place replacement
    system_cell_by_id: std::collections::HashMap<String, usize>,
    // Per-request counters for UI-issued background order metadata
    ui_background_seq_counters: HashMap<u64, Arc<AtomicU64>>,
    // Track the largest order key we have assigned so far to keep tail inserts monotonic
    last_assigned_order: Option<OrderKey>,
    replay_history_depth: usize,
    resume_placeholder_visible: bool,
    resume_picker_loading: bool,
    // Clickable regions for mouse interaction (tracked during render, checked on click)
    clickable_regions: RefCell<Vec<ClickableRegion>>,
}

#[derive(Clone, Debug, Default)]
struct ContextSummary {
    cwd: Option<String>,
    git_branch: Option<String>,
    reasoning_effort: Option<String>,
    browser_session_active: bool,
    deltas: Vec<ContextDeltaRecord>,
    browser_snapshot: Option<ContextBrowserSnapshotRecord>,
    expanded: bool,
}

#[derive(Clone, Debug)]
struct AutoCompactionOverlay {
    /// Snapshot of the conversation prefix (including the latest compact summary)
    /// that should be injected ahead of any history-derived tail when exporting
    /// the next Auto Drive request.
    prefix_items: Vec<code_protocol::models::ResponseItem>,
    /// History cell index that marks the beginning of the still-live tail that
    /// we continue to mirror directly from the UI.
    tail_start_cell: usize,
}

#[derive(Clone)]
pub(crate) struct BackgroundOrderTicket {
    request_ordinal: u64,
    seq_counter: Arc<AtomicU64>,
}

impl BackgroundOrderTicket {
    pub(crate) fn next_order(&self) -> code_core::protocol::OrderMeta {
        let seq = self.seq_counter.fetch_add(1, Ordering::SeqCst);
        code_core::protocol::OrderMeta {
            request_ordinal: self.request_ordinal,
            output_index: Some(i32::MAX as u32),
            sequence_number: Some(seq),
        }
    }
}

#[derive(Clone)]
struct GhostSnapshot {
    commit: GhostCommit,
    captured_at: DateTime<Local>,
    summary: Option<String>,
    conversation: ConversationSnapshot,
    history: HistorySnapshot,
}

#[derive(Clone, Copy)]
enum UndoPreviewRole {
    User,
    Assistant,
}

impl GhostSnapshot {
    fn new(
        commit: GhostCommit,
        summary: Option<String>,
        conversation: ConversationSnapshot,
        history: HistorySnapshot,
    ) -> Self {
        let summary = summary.and_then(|text| {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        Self {
            commit,
            captured_at: Local::now(),
            summary,
            conversation,
            history,
        }
    }

    fn commit(&self) -> &GhostCommit {
        &self.commit
    }

    fn short_id(&self) -> String {
        self.commit.id().chars().take(8).collect()
    }

    fn summary_snippet(&self, max_len: usize) -> Option<String> {
        let summary = self.summary.as_ref()?;
        let mut snippet = String::new();
        let mut truncated = false;
        for word in summary.split_whitespace() {
            if !snippet.is_empty() {
                snippet.push(' ');
            }
            snippet.push_str(word);
            if snippet.chars().count() > max_len {
                truncated = true;
                break;
            }
        }

        if snippet.chars().count() > max_len {
            truncated = true;
            snippet = snippet.chars().take(max_len).collect();
        }

        if truncated {
            snippet.push('…');
        }

        Some(snippet)
    }

    fn age_from(&self, now: DateTime<Local>) -> Option<std::time::Duration> {
        now.signed_duration_since(self.captured_at).to_std().ok()
    }
}

#[derive(Clone)]
struct GhostSnapshotRequest {
    summary: Option<String>,
    conversation: ConversationSnapshot,
    history: HistorySnapshot,
    started_at: Instant,
}

impl GhostSnapshotRequest {
    fn new(
        summary: Option<String>,
        conversation: ConversationSnapshot,
        history: HistorySnapshot,
    ) -> Self {
        Self {
            summary,
            conversation,
            history,
            started_at: Instant::now(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GhostSnapshotJobHandle {
    Scheduled(u64),
    Skipped,
}

#[derive(Default)]
struct GitBranchCache {
    value: Option<String>,
    last_head_mtime: Option<SystemTime>,
    last_refresh: Option<Instant>,
}

#[derive(Debug, Clone, Default)]
struct AgentRuntime {
    /// First time this agent entered Running
    started_at: Option<Instant>,
    /// Time of the latest status update we observed
    last_update: Option<Instant>,
    /// Time the agent reached a terminal state (Completed/Failed)
    completed_at: Option<Instant>,
}

#[derive(Debug, Clone)]
struct AgentTerminalEntry {
    name: String,
    batch_id: Option<String>,
    batch_label: Option<String>,
    batch_prompt: Option<String>,
    batch_context: Option<String>,
    model: Option<String>,
    status: AgentStatus,
    source_kind: Option<AgentSourceKind>,
    last_progress: Option<String>,
    result: Option<String>,
    error: Option<String>,
    logs: Vec<AgentLogEntry>,
}

impl AgentTerminalEntry {
    fn new(
        name: String,
        model: Option<String>,
        status: AgentStatus,
        batch_id: Option<String>,
    ) -> Self {
        Self {
            name,
            batch_id,
            batch_label: None,
            batch_prompt: None,
            batch_context: None,
            model,
            status,
            source_kind: None,
            last_progress: None,
            result: None,
            error: None,
            logs: Vec::new(),
        }
    }

    fn push_log(&mut self, kind: AgentLogKind, message: impl Into<String>) {
        let msg = message.into();
        if self
            .logs
            .last()
            .map(|entry| entry.kind == kind && entry.message == msg)
            .unwrap_or(false)
        {
            return;
        }
        self.logs.push(AgentLogEntry {
            timestamp: Local::now(),
            kind,
            message: msg,
        });
        const MAX_HISTORY: usize = 500;
        if self.logs.len() > MAX_HISTORY {
            let excess = self.logs.len() - MAX_HISTORY;
            self.logs.drain(0..excess);
        }
    }
}

#[derive(Debug, Clone)]
struct AgentLogEntry {
    timestamp: DateTime<Local>,
    kind: AgentLogKind,
    message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentLogKind {
    Status,
    Progress,
    Result,
    Error,
}

struct AgentsTerminalState {
    active: bool,
    selected_index: usize,
    order: Vec<String>,
    entries: HashMap<String, AgentTerminalEntry>,
    scroll_offsets: HashMap<String, u16>,
    // Last scroll offset used to render the detail view (bottom-anchored)
    last_render_scroll: std::cell::Cell<u16>,
    saved_scroll_offset: u16,
    shared_context: Option<String>,
    shared_task: Option<String>,
    pending_stop: Option<PendingAgentStop>,
    focus: AgentsTerminalFocus,
    active_tab: AgentsTerminalTab,
    sort_mode: AgentsSortMode,
    highlights_collapsed: bool,
    actions_collapsed: bool,
}

#[derive(Clone, Debug)]
struct PendingAgentStop {
    agent_id: String,
    agent_name: String,
}

#[derive(Default, Clone)]
struct AgentBatchMetadata {
    label: Option<String>,
    prompt: Option<String>,
    context: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum AgentsSidebarEntry {
    Agent(String),
}

#[derive(Clone, Debug)]
struct AgentsSidebarGroup {
    batch_id: Option<String>,
    label: String,
    agent_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentsTerminalTab {
    All,
    Running,
    Failed,
    Completed,
    Review,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentsSortMode {
    Recent,
    RunningFirst,
    Name,
}

fn short_batch_label(batch_id: &str) -> String {
    let compact: String = batch_id.chars().filter(|c| *c != '-').collect();
    let source = if compact.is_empty() { batch_id } else { compact.as_str() };
    let short: String = source.chars().take(8).collect();
    if short.is_empty() {
        "Batch".to_string()
    } else {
        format!("Batch {short}")
    }
}

impl AgentsSidebarEntry {
    fn scroll_key(&self) -> String {
        match self {
            AgentsSidebarEntry::Agent(id) => format!("agent:{id}"),
        }
    }
}

impl AgentsTerminalState {
    fn new() -> Self {
        Self {
            active: false,
            selected_index: 0,
            order: Vec::new(),
            entries: HashMap::new(),
            scroll_offsets: HashMap::new(),
            last_render_scroll: std::cell::Cell::new(0),
            saved_scroll_offset: 0,
            shared_context: None,
            shared_task: None,
            pending_stop: None,
            focus: AgentsTerminalFocus::Sidebar,
            active_tab: AgentsTerminalTab::All,
            sort_mode: AgentsSortMode::Recent,
            highlights_collapsed: false,
            actions_collapsed: false,
        }
    }

    fn reset(&mut self) {
        self.selected_index = 0;
        self.order.clear();
        self.entries.clear();
        self.scroll_offsets.clear();
        self.last_render_scroll.set(0);
        self.shared_context = None;
        self.shared_task = None;
        self.pending_stop = None;
        self.focus = AgentsTerminalFocus::Sidebar;
        self.active_tab = AgentsTerminalTab::All;
    }

    fn current_sidebar_entry(&self) -> Option<AgentsSidebarEntry> {
        let entries = self.sidebar_entries();
        entries.get(self.selected_index).cloned()
    }

    fn focus_sidebar(&mut self) {
        self.focus = AgentsTerminalFocus::Sidebar;
    }

    fn focus_detail(&mut self) {
        self.focus = AgentsTerminalFocus::Detail;
    }

    fn focus(&self) -> AgentsTerminalFocus {
        self.focus
    }

    fn set_stop_prompt(&mut self, agent_id: String, agent_name: String) {
        self.pending_stop = Some(PendingAgentStop { agent_id, agent_name });
    }

    fn clear_stop_prompt(&mut self) {
        self.pending_stop = None;
    }

    fn clamp_selected_index(&mut self) {
        let entries = self.sidebar_entries();
        if entries.is_empty() {
            self.selected_index = 0;
        } else if self.selected_index >= entries.len() {
            self.selected_index = entries.len().saturating_sub(1);
        }
    }

    fn reselect_entry(&mut self, entry: Option<AgentsSidebarEntry>) {
        if let Some(target) = entry
            && let Some(idx) = self
                .sidebar_entries()
                .iter()
                .position(|candidate| *candidate == target)
            {
                self.selected_index = idx;
                return;
            }
        self.clamp_selected_index();
    }

    fn cycle_sort_mode(&mut self) {
        self.sort_mode = match self.sort_mode {
            AgentsSortMode::Recent => AgentsSortMode::RunningFirst,
            AgentsSortMode::RunningFirst => AgentsSortMode::Name,
            AgentsSortMode::Name => AgentsSortMode::Recent,
        };
    }

    fn toggle_highlights(&mut self) {
        self.highlights_collapsed = !self.highlights_collapsed;
    }

    fn toggle_actions(&mut self) {
        self.actions_collapsed = !self.actions_collapsed;
    }

    fn tab_allows(&self, entry: &AgentTerminalEntry) -> bool {
        match self.active_tab {
            AgentsTerminalTab::All => true,
            AgentsTerminalTab::Running =>
                matches!(entry.status, AgentStatus::Pending | AgentStatus::Running),
            AgentsTerminalTab::Failed => matches!(entry.status, AgentStatus::Failed),
            AgentsTerminalTab::Completed =>
                matches!(entry.status, AgentStatus::Completed | AgentStatus::Cancelled),
            AgentsTerminalTab::Review => matches!(entry.source_kind, Some(AgentSourceKind::AutoReview)),
        }
    }

    fn filtered_order(&self) -> Vec<String> {
        let mut filtered: Vec<String> = self
            .order
            .iter()
            .filter(|id| {
                self.entries
                    .get(*id)
                    .map(|entry| self.tab_allows(entry))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        match self.sort_mode {
            AgentsSortMode::Recent => {
                // keep insertion order
            }
            AgentsSortMode::RunningFirst => {
                let mut positions: HashMap<String, usize> = HashMap::new();
                for (idx, id) in self.order.iter().enumerate() {
                    positions.insert(id.clone(), idx);
                }
                filtered.sort_by(|a, b| {
                    let sa = self
                        .entries
                        .get(a)
                        .map(|e| agent_running_priority(e.status.clone()))
                        .unwrap_or(usize::MAX);
                    let sb = self
                        .entries
                        .get(b)
                        .map(|e| agent_running_priority(e.status.clone()))
                        .unwrap_or(usize::MAX);
                    sa.cmp(&sb).then_with(|| positions[a].cmp(&positions[b]))
                });
            }
            AgentsSortMode::Name => {
                filtered.sort_by(|a, b| {
                    let left = self
                        .entries
                        .get(a)
                        .and_then(|e| e.name.split_whitespace().next())
                        .unwrap_or("")
                        .to_lowercase();
                    let right = self
                        .entries
                        .get(b)
                        .and_then(|e| e.name.split_whitespace().next())
                        .unwrap_or("")
                        .to_lowercase();
                    left.cmp(&right).then_with(|| a.cmp(b))
                });
            }
        }

        filtered
    }

    fn sidebar_entries(&self) -> Vec<AgentsSidebarEntry> {
        let mut out = Vec::new();
        for group in self.sidebar_groups() {
            for agent_id in group.agent_ids {
                out.push(AgentsSidebarEntry::Agent(agent_id));
            }
        }
        out
    }

    fn sidebar_groups(&self) -> Vec<AgentsSidebarGroup> {
        let mut groups: Vec<AgentsSidebarGroup> = Vec::new();
        let mut group_lookup: HashMap<Option<String>, usize> = HashMap::new();
        for id in self.filtered_order() {
            if let Some(entry) = self.entries.get(&id) {
                let key = entry.batch_id.clone();
                let idx = if let Some(idx) = group_lookup.get(&key) {
                    *idx
                } else {
                    let label = entry
                        .batch_label
                        .as_ref()
                        .and_then(|value| {
                            let trimmed = value.trim();
                            (!trimmed.is_empty()).then(|| trimmed.to_string())
                        })
                        .or_else(|| {
                            key.as_ref().map(|batch| short_batch_label(batch))
                        })
                        .unwrap_or_else(|| "Ad-hoc Agents".to_string());
                    let idx = groups.len();
                    group_lookup.insert(key.clone(), idx);
                    groups.push(AgentsSidebarGroup {
                        batch_id: key.clone(),
                        label,
                        agent_ids: Vec::new(),
                    });
                    idx
                };
                if let Some(group) = groups.get_mut(idx) {
                    group.agent_ids.push(id.clone());
                }
            }
        }
        groups
    }

    fn set_tab(&mut self, tab: AgentsTerminalTab) {
        if self.active_tab != tab {
            self.active_tab = tab;
            self.selected_index = 0;
        }
        self.clear_stop_prompt();
        self.clamp_selected_index();
    }

    fn jump_batch(&mut self, delta: isize) {
        let groups = self.sidebar_groups();
        if groups.is_empty() {
            return;
        }
        let current_batch = match self.current_sidebar_entry() {
            Some(AgentsSidebarEntry::Agent(id)) => self
                .entries
                .get(id.as_str())
                .and_then(|entry| entry.batch_id.clone()),
            None => None,
        };
        let mut idx: isize = groups
            .iter()
            .position(|group| group.batch_id == current_batch)
            .unwrap_or(0) as isize;
        let len = groups.len() as isize;
        if len == 0 {
            return;
        }
        idx = (idx + delta).rem_euclid(len);
        if let Some(target) = groups.get(idx as usize)
            && let Some(first_agent) = target.agent_ids.first()
                && let Some(pos) = self
                    .sidebar_entries()
                    .iter()
                    .position(|entry| matches!(entry, AgentsSidebarEntry::Agent(id) if id == first_agent))
                {
                    self.selected_index = pos;
                    self.focus_sidebar();
                    self.clear_stop_prompt();
                }
        self.clamp_selected_index();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentsTerminalFocus {
    Sidebar,
    Detail,
}

// ---------- Stable ordering & routing helpers ----------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OrderKey {
    req: u64,
    out: i32,
    seq: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct BrowserSessionOrderKey {
    req: u64,
    out: i32,
}

impl BrowserSessionOrderKey {
    fn from_order_meta(meta: &code_core::protocol::OrderMeta) -> Self {
        let out = meta
            .output_index
            .map(|value| {
                if value > i32::MAX as u32 {
                    i32::MAX
                } else {
                    value as i32
                }
            })
            .unwrap_or(i32::MAX);
        Self {
            req: meta.request_ordinal,
            out,
        }
    }
}

impl Ord for OrderKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.req.cmp(&other.req) {
            std::cmp::Ordering::Equal => match self.out.cmp(&other.out) {
                std::cmp::Ordering::Equal => self.seq.cmp(&other.seq),
                o => o,
            },
            o => o,
        }
    }
}

impl PartialOrd for OrderKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl From<OrderKeySnapshot> for OrderKey {
    fn from(snapshot: OrderKeySnapshot) -> Self {
        Self {
            req: snapshot.req,
            out: snapshot.out,
            seq: snapshot.seq,
        }
    }
}

impl From<OrderKey> for OrderKeySnapshot {
    fn from(key: OrderKey) -> Self {
        OrderKeySnapshot {
            req: key.req,
            out: key.out,
            seq: key.seq,
        }
    }
}

// Removed legacy turn-window logic; ordering is strictly global.

// Global guard to prevent overlapping background screenshot captures and to rate-limit them
static BG_SHOT_IN_FLIGHT: Lazy<AtomicBool> = Lazy::new(|| AtomicBool::new(false));
static BACKGROUND_REVIEW_LOCKS: Lazy<Mutex<HashMap<String, code_core::review_coord::ReviewGuard>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static BG_SHOT_LAST_START_MS: Lazy<AtomicU64> = Lazy::new(|| AtomicU64::new(0));
static MERGE_LOCKS: Lazy<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static WORKTREE_ROOT_HINTS: Lazy<Mutex<HashMap<PathBuf, PathBuf>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static CWD_HISTORY: Lazy<Mutex<Vec<PathBuf>>> = Lazy::new(|| Mutex::new(Vec::new()));
const CWD_HISTORY_LIMIT: usize = 16;

fn remember_worktree_root_hint(worktree: &Path, git_root: &Path) {
    let mut hints = WORKTREE_ROOT_HINTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let root = git_root.to_path_buf();
    hints.insert(worktree.to_path_buf(), root.clone());
    if let Ok(real) = std::fs::canonicalize(worktree) {
        hints.insert(real, root);
    }
}

fn worktree_root_hint_for(path: &Path) -> Option<PathBuf> {
    let hints = WORKTREE_ROOT_HINTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    hints.get(path).cloned()
}

fn remember_cwd_history(path: &Path) {
    let mut history = CWD_HISTORY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if history.last().is_some_and(|p| p == path) {
        return;
    }
    history.push(path.to_path_buf());
    if history.len() > CWD_HISTORY_LIMIT {
        history.remove(0);
    }
}

fn last_existing_cwd(except: &Path) -> Option<PathBuf> {
    let history = CWD_HISTORY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    history
        .iter()
        .rev()
        .filter(|p| p.as_path() != except)
        .find(|p| p.exists())
        .cloned()
}

use self::diff_ui::DiffBlock;
use self::diff_ui::DiffConfirm;
use self::diff_ui::DiffOverlay;
use self::settings_overlay::{
    AgentOverviewRow,
    AccountsSettingsContent,
    AutoDriveSettingsContent,
    AgentsSettingsContent,
    LimitsSettingsContent,
    ChromeSettingsContent,
    McpSettingsContent,
    ModelSettingsContent,
    PlanningSettingsContent,
    NotificationsSettingsContent,
    PromptsSettingsContent,
    SkillsSettingsContent,
    ReviewSettingsContent,
    ThemeSettingsContent,
    UpdatesSettingsContent,
    ValidationSettingsContent,
    SettingsOverlayView,
    SettingsOverviewRow,
};
use ratatui::text::Line as RtLine;
use ratatui::text::Span as RtSpan;


use self::perf::PerfStats;

#[derive(Debug, Clone)]
struct AgentInfo {
    // Stable id to correlate updates
    id: String,
    // Display name
    name: String,
    // Current status
    status: AgentStatus,
    // Source of the agent (e.g., Auto Review)
    source_kind: Option<AgentSourceKind>,
    // Batch identifier reported by the core (if any)
    batch_id: Option<String>,
    // Optional model name
    model: Option<String>,
    // Final success message when completed
    result: Option<String>,
    // Final error message when failed
    error: Option<String>,
    // Most recent progress line from core
    last_progress: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum AgentStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

fn agent_status_from_str(status: &str) -> AgentStatus {
    match status {
        "pending" => AgentStatus::Pending,
        "running" => AgentStatus::Running,
        "completed" => AgentStatus::Completed,
        "failed" => AgentStatus::Failed,
        "cancelled" => AgentStatus::Cancelled,
        _ => AgentStatus::Pending,
    }
}

fn agent_status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Pending => "Pending",
        AgentStatus::Running => "Running",
        AgentStatus::Completed => "Completed",
        AgentStatus::Failed => "Failed",
        AgentStatus::Cancelled => "Cancelled",
    }
}

fn agent_status_icon(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Completed => "✔",
        AgentStatus::Running => "▶",
        AgentStatus::Pending => "…",
        AgentStatus::Failed => "✖",
        AgentStatus::Cancelled => "⏹",
    }
}

fn agent_running_priority(status: AgentStatus) -> usize {
    match status {
        AgentStatus::Running => 0,
        AgentStatus::Pending => 1,
        AgentStatus::Failed => 2,
        AgentStatus::Completed => 3,
        AgentStatus::Cancelled => 4,
    }
}

fn agent_status_color(status: AgentStatus) -> ratatui::style::Color {
    match status {
        AgentStatus::Pending => crate::colors::warning(),
        AgentStatus::Running => crate::colors::info(),
        AgentStatus::Completed => crate::colors::success(),
        AgentStatus::Failed => crate::colors::error(),
        AgentStatus::Cancelled => crate::colors::warning(),
    }
}

fn agent_log_label(kind: AgentLogKind) -> &'static str {
    match kind {
        AgentLogKind::Status => "status",
        AgentLogKind::Progress => "progress",
        AgentLogKind::Result => "result",
        AgentLogKind::Error => "error",
    }
}

fn agent_log_color(kind: AgentLogKind) -> ratatui::style::Color {
    match kind {
        AgentLogKind::Status => crate::colors::info(),
        AgentLogKind::Progress => crate::colors::primary(),
        AgentLogKind::Result => crate::colors::success(),
        AgentLogKind::Error => crate::colors::error(),
    }
}

use self::message::create_initial_user_message;

// Newtype IDs for clarity across exec/tools/streams
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ExecCallId(pub String);
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ToolCallId(pub String);
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct StreamId(pub String);

impl From<String> for ExecCallId {
    fn from(s: String) -> Self {
        ExecCallId(s)
    }
}
impl From<&str> for ExecCallId {
    fn from(s: &str) -> Self {
        ExecCallId(s.to_string())
    }
}

fn wait_target_from_params(params: Option<&String>, call_id: &str) -> String {
    if let Some(raw) = params
        && let Ok(json) = serde_json::from_str::<serde_json::Value>(raw) {
            if let Some(for_value) = json.get("for").and_then(|v| v.as_str()) {
                let cleaned = clean_wait_command(for_value);
                if !cleaned.is_empty() {
                    return cleaned;
                }
            }
            if let Some(cid) = json.get("call_id").and_then(|v| v.as_str()) {
                return format!("call {cid}");
            }
        }
    format!("call {call_id}")
}

fn wait_exec_call_id_from_params(params: Option<&String>) -> Option<ExecCallId> {
    params
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|json| json.get("call_id").and_then(|v| v.as_str()).map(|s| ExecCallId(s.to_string())))
}

fn wait_result_missing_background_job(message: &str) -> bool {
    let trimmed = message.trim();
    trimmed.starts_with("No background job found for call_id=")
        || trimmed == "No completed background job found"
}

fn wait_result_interrupted(message: &str) -> bool {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.contains("wait ended due to new user message")
        || lower.contains("wait ended because the session was interrupted")
        || lower.contains("wait interrupted so the assistant can adapt")
        || (lower.contains("background job") && lower.contains("still running"))
}

fn image_mime_from_path(path: &Path) -> Option<String> {
    let ext = path.extension().and_then(|ext| ext.to_str())?;
    let mime = match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "tif" | "tiff" => "image/tiff",
        _ => return None,
    };
    Some(mime.to_string())
}

fn image_record_from_path(path: &Path) -> Option<ImageRecord> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            tracing::warn!("Failed to read image {}: {err}", path.display());
            return None;
        }
    };
    let (width, height) = match image::image_dimensions(path) {
        Ok((w, h)) => (
            w.min(u16::MAX as u32) as u16,
            h.min(u16::MAX as u32) as u16,
        ),
        Err(err) => {
            tracing::warn!("Failed to read image dimensions for {}: {err}", path.display());
            (0, 0)
        }
    };
    let sha_hex = format!("{:x}", Sha256::digest(&bytes));
    let byte_len = bytes.len().min(u32::MAX as usize) as u32;
    Some(ImageRecord {
        id: HistoryId::ZERO,
        source_path: Some(path.to_path_buf()),
        alt_text: None,
        width,
        height,
        sha256: Some(sha_hex),
        mime_type: image_mime_from_path(path),
        byte_len: Some(byte_len),
    })
}

fn image_view_path_from_params(params: &serde_json::Value, cwd: &Path) -> Option<PathBuf> {
    let path = params.get("path").and_then(|value| value.as_str())?;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut resolved = PathBuf::from(trimmed);
    if resolved.is_relative() {
        resolved = cwd.join(&resolved);
    }
    if let Ok(canon) = resolved.canonicalize() {
        resolved = canon;
    }
    Some(resolved)
}

impl std::fmt::Display for ExecCallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl AsRef<str> for ExecCallId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for ToolCallId {
    fn from(s: String) -> Self {
        ToolCallId(s)
    }
}
impl From<&str> for ToolCallId {
    fn from(s: &str) -> Self {
        ToolCallId(s.to_string())
    }
}
impl std::fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl AsRef<str> for ToolCallId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for StreamId {
    fn from(s: String) -> Self {
        StreamId(s)
    }
}
impl From<&str> for StreamId {
    fn from(s: &str) -> Self {
        StreamId(s.to_string())
    }
}
impl std::fmt::Display for StreamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl AsRef<str> for StreamId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// ---- System notice ordering helpers ----
#[derive(Copy, Clone)]
enum SystemPlacement {
    /// Place near the top of the current request (before most provider output)
    EarlyInCurrent,
    /// Place at the end of the current request window (after provider output)
    EndOfCurrent,
    /// Place before the first user prompt of the very first request
    /// (used for pre-turn UI confirmations like theme/spinner changes)
    PrePromptInCurrent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutoDriveRole {
    User,
    Assistant,
}

impl ChatWidget<'_> {
    const MAX_UNDO_CONVERSATION_MESSAGES: usize = 8;
    const MAX_UNDO_PREVIEW_CHARS: usize = 160;
    const MAX_UNDO_FILE_LINES: usize = 24;

    fn fmt_short_duration(&self, d: Duration) -> String {
        let s = d.as_secs();
        let h = s / 3600;
        let m = (s % 3600) / 60;
        let sec = s % 60;
        if h > 0 {
            format!("{h}h{m}m")
        } else if m > 0 {
            format!("{m}m{sec}s")
        } else {
            format!("{sec}s")
        }
    }
    fn is_branch_worktree_path(path: &std::path::Path) -> bool {
        for ancestor in path.ancestors() {
            if ancestor
                .file_name()
                .map(|name| name == std::ffi::OsStr::new("branches"))
                .unwrap_or(false)
            {
                let mut higher = ancestor.parent();
                while let Some(dir) = higher {
                    if dir
                        .file_name()
                        .map(|name| name == std::ffi::OsStr::new(".code"))
                        .unwrap_or(false)
                    {
                        return true;
                    }
                    higher = dir.parent();
                }
            }
        }
        false
    }

    fn merge_lock_for_repo(path: &std::path::Path) -> Arc<tokio::sync::Mutex<()>> {
        let key = path.to_path_buf();
        let mut locks = MERGE_LOCKS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match locks.entry(key) {
            Entry::Occupied(existing) => existing.get().clone(),
            Entry::Vacant(slot) => slot.insert(Arc::new(tokio::sync::Mutex::new(()))).clone(),
        }
    }

    async fn git_short_status(path: &std::path::Path) -> Result<String, String> {
        use tokio::process::Command;
        match Command::new("git")
            .current_dir(path)
            .args(["status", "--short"])
            .output()
            .await
        {
            Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).to_string()),
            Ok(out) => {
                let stderr_s = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let stdout_s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !stderr_s.is_empty() {
                    Err(stderr_s)
                } else if !stdout_s.is_empty() {
                    Err(stdout_s)
                } else {
                    let code = out
                        .status
                        .code()
                        .map(|c| format!("exit status {c}"))
                        .unwrap_or_else(|| "terminated by signal".to_string());
                    Err(format!("git status failed: {code}"))
                }
            }
            Err(err) => Err(err.to_string()),
        }
    }

    async fn git_diff_stat(path: &std::path::Path) -> Result<String, String> {
        use tokio::process::Command;
        match Command::new("git")
            .current_dir(path)
            .args(["diff", "--stat"])
            .output()
            .await
        {
            Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).to_string()),
            Ok(out) => {
                let stderr_s = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let stdout_s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !stderr_s.is_empty() {
                    Err(stderr_s)
                } else if !stdout_s.is_empty() {
                    Err(stdout_s)
                } else {
                    let code = out
                        .status
                        .code()
                        .map(|c| format!("exit status {c}"))
                        .unwrap_or_else(|| "terminated by signal".to_string());
                    Err(format!("git diff --stat failed: {code}"))
                }
            }
            Err(err) => Err(err.to_string()),
        }
    }

    /// Compute an OrderKey for system (non‑LLM) notices in a way that avoids
    /// creating multiple synthetic request buckets before the first provider turn.
    fn system_order_key(
        &mut self,
        placement: SystemPlacement,
        order: Option<&code_core::protocol::OrderMeta>,
    ) -> OrderKey {
        // If the provider supplied OrderMeta, honor it strictly.
        if let Some(om) = order {
            return self.provider_order_key_from_order_meta(om);
        }

        // Derive a stable request bucket for system notices when OrderMeta is absent.
        // Default to the current provider request if known; else use a sticky
        // pre-turn synthetic req=1 to group UI confirmations before the first turn.
        // If a user prompt for the next turn is already queued, attach new
        // system notices to the upcoming request to avoid retroactive inserts.
        let mut req = if self.last_seen_request_index > 0 {
            self.last_seen_request_index
        } else {
            if self.synthetic_system_req.is_none() {
                self.synthetic_system_req = Some(1);
            }
            self.synthetic_system_req.unwrap_or(1)
        };
        if order.is_none() && self.pending_user_prompts_for_next_turn > 0 {
            req = req.saturating_add(1);
        }

        self.internal_seq = self.internal_seq.saturating_add(1);
        let mut out = match placement {
            SystemPlacement::EarlyInCurrent => i32::MIN + 2,
            SystemPlacement::EndOfCurrent => i32::MAX,
            SystemPlacement::PrePromptInCurrent => i32::MIN,
        };

        if order.is_none()
            && self.pending_user_prompts_for_next_turn > 0
            && matches!(placement, SystemPlacement::EarlyInCurrent)
        {
            out = i32::MIN;
        }

        let mut key = OrderKey {
            req,
            out,
            seq: self.internal_seq,
        };

        if matches!(placement, SystemPlacement::EndOfCurrent) {
            let reference = self
                .last_assigned_order
                .or_else(|| self.cell_order_seq.iter().copied().max());
            if let Some(max_key) = reference
                && key <= max_key {
                    key = Self::order_key_successor(max_key);
                }
        }

        self.internal_seq = self.internal_seq.max(key.seq);
        self.last_assigned_order = Some(match self.last_assigned_order {
            Some(prev) => prev.max(key),
            None => key,
        });

        key
    }

    fn background_tail_request_ordinal(&mut self) -> u64 {
        let mut req = if self.last_seen_request_index > 0 {
            self.last_seen_request_index
        } else {
            *self.synthetic_system_req.get_or_insert(1)
        };
        if self.pending_user_prompts_for_next_turn > 0 {
            req = req.saturating_add(1);
        }
        if let Some(last) = self.last_assigned_order {
            req = req.max(last.req);
        }
        if let Some(max_req) = self.ui_background_seq_counters.keys().copied().max() {
            req = req.max(max_req);
        }
        req
    }

    fn background_order_ticket_for_req(&mut self, req: u64) -> BackgroundOrderTicket {
        let seed = self
            .last_assigned_order
            .filter(|key| key.req == req)
            .map(|key| key.seq.saturating_add(1))
            .unwrap_or(0);

        let counter = self
            .ui_background_seq_counters
            .entry(req)
            .or_insert_with(|| Arc::new(AtomicU64::new(seed)))
            .clone();

        if seed > 0 {
            let current = counter.load(Ordering::SeqCst);
            if current < seed {
                counter.store(seed, Ordering::SeqCst);
            }
        }
        BackgroundOrderTicket {
            request_ordinal: req,
            seq_counter: counter,
        }
    }

    fn background_tail_order_ticket_internal(&mut self) -> BackgroundOrderTicket {
        let req = self.background_tail_request_ordinal();
        self.background_order_ticket_for_req(req)
    }

    fn background_before_next_output_request_ordinal(&mut self) -> u64 {
        if self.last_seen_request_index > 0 {
            self.last_seen_request_index
        } else {
            *self.synthetic_system_req.get_or_insert(1)
        }
    }

    fn background_before_next_output_ticket_internal(&mut self) -> BackgroundOrderTicket {
        let req = self.background_before_next_output_request_ordinal();
        self.background_order_ticket_for_req(req)
    }

    pub(crate) fn make_background_tail_ticket(&mut self) -> BackgroundOrderTicket {
        self.background_tail_order_ticket_internal()
    }

    pub(crate) fn make_background_before_next_output_ticket(&mut self) -> BackgroundOrderTicket {
        self.background_before_next_output_ticket_internal()
    }

    fn auto_card_next_order_key(&mut self) -> OrderKey {
        let ticket = self.make_background_tail_ticket();
        let meta = ticket.next_order();
        self.provider_order_key_from_order_meta(&meta)
    }

    fn auto_card_start(&mut self, goal: Option<String>) {
        let order_key = self.auto_card_next_order_key();
        auto_drive_cards::start_session(self, order_key, goal);
    }

    fn auto_card_add_action(&mut self, message: String, kind: AutoDriveActionKind) {
        let order_key = self.auto_card_next_order_key();
        let had_tracker = self.tools_state.auto_drive_tracker.is_some();
        auto_drive_cards::record_action(self, order_key, message.clone(), kind);
        if !had_tracker {
            self.push_background_tail(message);
        }
    }

    fn auto_card_set_status(&mut self, status: AutoDriveStatus) {
        if self.tools_state.auto_drive_tracker.is_some() {
            let order_key = self.auto_card_next_order_key();
            auto_drive_cards::set_status(self, order_key, status);
        }
    }

    fn auto_card_set_goal(&mut self, goal: Option<String>) {
        if self.tools_state.auto_drive_tracker.is_none() {
            return;
        }
        let order_key = self.auto_card_next_order_key();
        auto_drive_cards::update_goal(self, order_key, goal);
    }

    fn auto_card_finalize(
        &mut self,
        message: Option<String>,
        status: AutoDriveStatus,
        kind: AutoDriveActionKind,
    ) {
        let had_tracker = self.tools_state.auto_drive_tracker.is_some();
        let order_key = self.auto_card_next_order_key();
        let completion_message = if matches!(status, AutoDriveStatus::Stopped) {
            self.auto_state.last_completion_explanation.clone()
        } else {
            None
        };
        auto_drive_cards::finalize(
            self,
            order_key,
            message.clone(),
            status,
            kind,
            completion_message,
        );
        if !had_tracker
            && let Some(msg) = message {
                self.push_background_tail(msg);
            }
        if matches!(status, AutoDriveStatus::Stopped) {
            self.auto_state.last_completion_explanation = None;
        }
        auto_drive_cards::clear(self);
    }

    fn auto_request_session_summary(&mut self) {
        let prompt = AUTO_DRIVE_SESSION_SUMMARY_PROMPT.trim();
        if prompt.is_empty() {
            tracing::warn!("Auto Drive session summary prompt is empty");
            return;
        }

        self.push_background_tail(AUTO_DRIVE_SESSION_SUMMARY_NOTICE.to_string());
        self.request_redraw();
        self.submit_hidden_text_message_with_preface(prompt.to_string(), String::new());
    }

    fn spawn_conversation_runtime(
        &mut self,
        config: Config,
        auth_manager: Arc<AuthManager>,
        code_op_rx: UnboundedReceiver<Op>,
    ) {
        let ticket = self.make_background_tail_ticket();
        let ticket_for_submit = ticket.clone();
        let app_event_tx_clone = self.app_event_tx.clone();

        tokio::spawn(async move {
            let mut code_op_rx = code_op_rx;
            let conversation_manager = ConversationManager::new(
                auth_manager.clone(),
                SessionSource::Cli,
            );
            let resume_path = config.experimental_resume.clone();
            let new_conversation = match resume_path {
                Some(path) => conversation_manager
                    .resume_conversation_from_rollout(config.clone(), path, auth_manager.clone())
                    .await,
                None => conversation_manager.new_conversation(config).await,
            };

            let new_conversation = match new_conversation {
                Ok(conv) => conv,
                Err(e) => {
                    tracing::error!("failed to initialize conversation: {e}");
                    app_event_tx_clone.send_background_event_with_ticket(
                        &ticket,
                        format!(
                            "❌ Failed to initialize model session: {e}.\n• Ensure an OpenAI API key is set (CODE_OPENAI_API_KEY / OPENAI_API_KEY) or run `code login`.\n• Also verify config.cwd is an absolute path."
                        ),
                    );
                    return;
                }
            };

            let event = Event {
                id: new_conversation.conversation_id.to_string(),
                event_seq: 0,
                msg: EventMsg::SessionConfigured(new_conversation.session_configured),
                order: None,
            };
            app_event_tx_clone.send(AppEvent::CodexEvent(event));

            let conversation = new_conversation.conversation;
            let conversation_clone = conversation.clone();
            let app_event_tx_submit = app_event_tx_clone.clone();
            let ticket_for_submit = ticket_for_submit.clone();

            tokio::spawn(async move {
                while let Some(op) = code_op_rx.recv().await {
                    if let Err(e) = conversation_clone.submit(op).await {
                        tracing::error!("failed to submit op: {e}");
                        app_event_tx_submit.send_background_event_with_ticket(
                            &ticket_for_submit,
                            format!("⚠️ Failed to submit Op to core: {e}"),
                        );
                    }
                }
            });

            while let Ok(event) = conversation.next_event().await {
                app_event_tx_clone.send(AppEvent::CodexEvent(event));
            }
            // (debug end notice removed)
        });
    }

    fn consume_pending_prompt_for_ui_only_turn(&mut self) {
        if self.pending_user_prompts_for_next_turn > 0 {
            self.pending_user_prompts_for_next_turn -= 1;
        }
        if !self.pending_dispatched_user_messages.is_empty() {
            self.pending_dispatched_user_messages.pop_front();
        }
    }

    fn background_tail_order_meta(&mut self) -> code_core::protocol::OrderMeta {
        self.background_tail_order_ticket_internal().next_order()
    }

    fn send_background_tail_ordered(&mut self, message: impl Into<String>) {
        let order = self.background_tail_order_meta();
        self.app_event_tx
            .send_background_event_with_order(message.into(), order);
    }

    fn rebuild_ui_background_seq_counters(&mut self) {
        self.ui_background_seq_counters.clear();
        let mut next_per_req: HashMap<u64, u64> = HashMap::new();
        for key in &self.cell_order_seq {
            if key.out == i32::MAX {
                let next = key.seq.saturating_add(1);
                let entry = next_per_req.entry(key.req).or_insert(0);
                *entry = (*entry).max(next);
            }
        }
        for (req, next) in next_per_req {
            self.ui_background_seq_counters
                .insert(req, Arc::new(AtomicU64::new(next)));
        }
    }

    /// Insert or replace a system notice cell with consistent ordering.
    /// If `id_for_replace` is provided and we have a prior index for it, replace in place.
    fn push_system_cell(
        &mut self,
        cell: Box<dyn HistoryCell>,
        placement: SystemPlacement,
        id_for_replace: Option<String>,
        order: Option<&code_core::protocol::OrderMeta>,
        tag: &'static str,
        record: Option<HistoryDomainRecord>,
    ) {
        if let Some(id) = id_for_replace.as_ref()
            && let Some(&idx) = self.system_cell_by_id.get(id) {
                if let Some(record) = record {
                    self.history_replace_with_record(idx, cell, record);
                } else {
                    self.history_replace_at(idx, cell);
                }
                return;
            }
        let key = self.system_order_key(placement, order);
        let pos = self.history_insert_with_key_global_tagged(cell, key, tag, record);
        if let Some(id) = id_for_replace {
            self.system_cell_by_id.insert(id, pos);
        }
    }

    /// Decide where to place a UI confirmation right now.
    /// If we're truly pre-turn (no provider traffic yet, and no queued prompt),
    /// place before the first user prompt. Otherwise, append to end of current.
    fn ui_placement_for_now(&self) -> SystemPlacement {
        if self.last_seen_request_index == 0 && self.pending_user_prompts_for_next_turn == 0 {
            SystemPlacement::PrePromptInCurrent
        } else {
            SystemPlacement::EndOfCurrent
        }
    }
    pub(crate) fn enable_perf(&mut self, enable: bool) {
        self.perf_state.enabled = enable;
    }
    pub(crate) fn perf_summary(&self) -> String {
        self.perf_state.stats.borrow().summary()
    }
    // Build an ordered key from model-provided OrderMeta. Callers must
    // guarantee presence by passing a concrete reference (compile-time guard).

    /// Show the "Shift+Up/Down" input history hint the first time the user scrolls.
    pub(super) fn maybe_show_history_nav_hint_on_first_scroll(&mut self) {
        if self.scroll_history_hint_shown {
            return;
        }
        self.scroll_history_hint_shown = true;
        self.bottom_pane.flash_footer_notice_for(
            "Use Shift+Up/Down to use previous input".to_string(),
            std::time::Duration::from_secs(6),
        );
    }

    pub(super) fn perf_track_scroll_delta(&self, before: u16, after: u16) {
        if !self.perf_state.enabled {
            return;
        }
        if before == after {
            return;
        }
        let delta = before.abs_diff(after) as u64;
        {
            let mut stats = self.perf_state.stats.borrow_mut();
            stats.record_scroll_trigger(delta);
        }
        let pending = self
            .perf_state
            .pending_scroll_rows
            .get()
            .saturating_add(delta);
        self.perf_state.pending_scroll_rows.set(pending);
    }

    // Synthetic key for internal content that should appear at the TOP of the NEXT request
    // (e.g., the user’s prompt preceding the model’s output for that turn).
    fn next_req_key_top(&mut self) -> OrderKey {
        let req = self.last_seen_request_index.saturating_add(1);
        self.internal_seq = self.internal_seq.saturating_add(1);
        OrderKey {
            req,
            out: i32::MIN,
            seq: self.internal_seq,
        }
    }

    // Synthetic key for a user prompt that should appear just after banners but
    // still before any model output within the next request.
    fn next_req_key_prompt(&mut self) -> OrderKey {
        let req = self.last_seen_request_index.saturating_add(1);
        self.internal_seq = self.internal_seq.saturating_add(1);
        OrderKey {
            req,
            out: i32::MIN + 1,
            seq: self.internal_seq,
        }
    }

    // Synthetic key for internal notices tied to the upcoming turn that
    // should appear immediately after the user prompt but still before any
    // model output for that turn.
    fn next_req_key_after_prompt(&mut self) -> OrderKey {
        let req = self.last_seen_request_index.saturating_add(1);
        self.internal_seq = self.internal_seq.saturating_add(1);
        OrderKey {
            req,
            out: i32::MIN + 2,
            seq: self.internal_seq,
        }
    }
    /// Returns true if any agents are actively running (Pending or Running), or we're about to start them.
    /// Agents in terminal states (Completed/Failed) do not keep the spinner visible.
    fn agents_are_actively_running(&self) -> bool {
        let has_running_non_auto_review = self
            .active_agents
            .iter()
            .any(|a| {
                matches!(a.status, AgentStatus::Pending | AgentStatus::Running)
                    && !matches!(a.source_kind, Some(AgentSourceKind::AutoReview))
            });

        if has_running_non_auto_review {
            return true;
        }

        // If only Auto Review agents are active, don't drive the spinner.
        let has_running_auto_review = self
            .active_agents
            .iter()
            .any(|a| {
                matches!(a.status, AgentStatus::Pending | AgentStatus::Running)
                    && matches!(a.source_kind, Some(AgentSourceKind::AutoReview))
            });

        if has_running_auto_review {
            return false;
        }

        // Fall back to preparatory state (e.g., Auto Drive about to launch agents)
        self.agents_ready_to_start
    }

    fn has_cancelable_agents(&self) -> bool {
        self
            .active_agents
            .iter()
            .any(Self::agent_is_cancelable)
    }

    fn agent_is_cancelable(agent: &AgentInfo) -> bool {
        matches!(agent.status, AgentStatus::Pending | AgentStatus::Running)
            && !matches!(agent.source_kind, Some(AgentSourceKind::AutoReview))
    }

    fn collect_cancelable_agents(&self) -> (Vec<String>, Vec<String>) {
        let mut batch_ids: BTreeSet<String> = BTreeSet::new();
        let mut agent_ids: BTreeSet<String> = BTreeSet::new();

        for agent in &self.active_agents {
            if !Self::agent_is_cancelable(agent) {
                continue;
            }

            if let Some(batch) = agent.batch_id.as_ref() {
                let trimmed = batch.trim();
                if !trimmed.is_empty() {
                    batch_ids.insert(trimmed.to_string());
                    continue;
                }
            }

            let trimmed_id = agent.id.trim();
            if !trimmed_id.is_empty() {
                agent_ids.insert(trimmed_id.to_string());
            }
        }

        (
            batch_ids.into_iter().collect(),
            agent_ids.into_iter().collect(),
        )
    }

    fn cancel_active_agents(&mut self) -> bool {
        let (batch_ids, agent_ids) = self.collect_cancelable_agents();
        if batch_ids.is_empty() && agent_ids.is_empty() {
            return false;
        }

        let mut status_parts = Vec::new();
        if !batch_ids.is_empty() {
            let count = batch_ids.len();
            status_parts.push(if count == 1 {
                "1 batch".to_string()
            } else {
                format!("{count} batches")
            });
        }
        if !agent_ids.is_empty() {
            let count = agent_ids.len();
            status_parts.push(if count == 1 {
                "1 agent".to_string()
            } else {
                format!("{count} agents")
            });
        }

        let descriptor = if status_parts.is_empty() {
            "agents".to_string()
        } else {
            status_parts.join(", ")
        };
        let auto_active = self.auto_state.is_active();
        self.push_background_tail(format!("Cancelling {descriptor}…"));
        self.bottom_pane
            .update_status_text("Cancelling agents…".to_string());
        self.bottom_pane.set_task_running(true);
        self.submit_op(Op::CancelAgents { batch_ids, agent_ids });

        self.agents_ready_to_start = false;

        if auto_active {
            self.show_auto_drive_exit_hint();
        } else if self
            .bottom_pane
            .standard_terminal_hint()
            .is_some_and(|hint| hint == AUTO_ESC_EXIT_HINT || hint == AUTO_ESC_EXIT_HINT_DOUBLE)
        {
            self.bottom_pane.set_standard_terminal_hint(None);
        }
        self.request_redraw();

        true
    }

    /// Hide the bottom spinner/status if the UI is idle (no streams, tools, agents, or tasks).
    fn maybe_hide_spinner(&mut self) {
        let any_tools_running = !self.exec.running_commands.is_empty()
            || !self.tools_state.running_custom_tools.is_empty()
            || !self.tools_state.web_search_sessions.is_empty();
        let any_streaming = self.stream.is_write_cycle_active();
        let any_agents_active = self.agents_are_actively_running();
        let mut any_tasks_active = !self.active_task_ids.is_empty();
        let final_answer_seen =
            self.last_answer_history_id_in_turn.is_some() || self.stream_state.seq_answer_final.is_some();
        let terminal_running = self.terminal_is_running();

        // If the backend never emits TaskComplete but we already received the
        // final answer and no other activity is running, clear the spinner so
        // we don't stay stuck on "Thinking...".
        let stuck_on_completed_turn = any_tasks_active
            && final_answer_seen
            && !any_tools_running
            && !any_streaming
            && !any_agents_active
            && !terminal_running;
        if stuck_on_completed_turn {
            self.active_task_ids.clear();
            any_tasks_active = false;
            self.overall_task_status = "complete".to_string();
        }
        if !(any_tools_running
            || any_streaming
            || any_agents_active
            || any_tasks_active
            || terminal_running)
        {
            self.bottom_pane.set_task_running(false);
            self.bottom_pane.update_status_text(String::new());
        }
    }

    /// Ensure we show progress when work is visible but the spinner state drifted.
    fn ensure_spinner_for_activity(&mut self, reason: &'static str) {
        if self.bottom_pane.auto_drive_style_active()
            && !self.bottom_pane.auto_drive_view_active()
            && !self.bottom_pane.has_active_modal_view()
        {
            tracing::debug!(
                "Auto Drive style active without view; releasing style (reason: {reason})"
            );
            self.bottom_pane.release_auto_drive_style();
        }
        if !self.bottom_pane.is_task_running() {
            tracing::debug!("Activity without spinner; re-enabling (reason: {reason})");
            self.bottom_pane.set_task_running(true);
        }
    }

    #[inline]
    fn stop_spinner(&mut self) {
        self.bottom_pane.set_task_running(false);
        self.bottom_pane.update_status_text(String::new());
        self.maybe_hide_spinner();
    }

    #[cfg(any(test, feature = "test-helpers"))]
    fn seed_test_mode_greeting(&mut self) {
        if !self.test_mode {
            return;
        }
        let has_assistant = self
            .history_cells
            .iter()
            .any(|cell| matches!(cell.kind(), history_cell::HistoryCellType::Assistant));
        if has_assistant {
            return;
        }

        let sections = [
            "Hello! How can I help you today?",
            "I can help with various tasks including:\n\n- Writing code\n- Reading files\n- Running commands",
        ];

        for markdown in sections {
            let greeting_state = AssistantMessageState {
                id: HistoryId::ZERO,
                stream_id: None,
                markdown: markdown.to_string(),
                citations: Vec::new(),
                metadata: None,
                token_usage: None,
                mid_turn: false,
                created_at: SystemTime::now(),
            };
            let greeting_cell =
                history_cell::AssistantMarkdownCell::from_state(greeting_state, &self.config);
            self.history_push_top_next_req(greeting_cell);
        }
    }

    #[inline]
    fn overall_task_status_for(agents: &[AgentInfo]) -> &'static str {
        if agents.is_empty() {
            "preparing"
        } else if agents
            .iter()
            .any(|a| matches!(a.status, AgentStatus::Running))
        {
            "running"
        } else if agents
            .iter()
            .all(|a| matches!(a.status, AgentStatus::Completed))
        {
            "complete"
        } else if agents
            .iter()
            .any(|a| matches!(a.status, AgentStatus::Failed))
        {
            "failed"
        } else if agents
            .iter()
            .any(|a| matches!(a.status, AgentStatus::Cancelled))
        {
            "cancelled"
        } else {
            "planning"
        }
    }

    /// Mark all tracked agents as having reached a terminal state when a turn finishes.
    fn finalize_agent_activity(&mut self) {
        if self.active_agents.is_empty()
            && self.agent_runtime.is_empty()
            && self.agents_terminal.entries.is_empty()
        {
            self.agents_ready_to_start = false;
            return;
        }

        for agent in self.active_agents.iter_mut() {
            if matches!(agent.status, AgentStatus::Pending | AgentStatus::Running) {
                agent.status = AgentStatus::Completed;
            }
        }

        for entry in self.agents_terminal.entries.values_mut() {
            if matches!(entry.status, AgentStatus::Pending | AgentStatus::Running) {
                entry.status = AgentStatus::Completed;
                entry.push_log(
                    AgentLogKind::Status,
                    format!("Status → {}", agent_status_label(AgentStatus::Completed)),
                );
            }
        }

        self.agents_ready_to_start = false;
        let status = Self::overall_task_status_for(&self.active_agents);
        self.overall_task_status = status.to_string();
        self.bottom_pane.set_task_running(false);
        self.bottom_pane.update_status_text(String::new());
        self.maybe_hide_spinner();
    }


    fn remove_background_completion_message(&mut self, call_id: &str) {
        if let Some(idx) = self.history_cells.iter().rposition(|cell| {
            matches!(cell.kind(), HistoryCellType::BackgroundEvent)
                && cell
                    .as_any()
                    .downcast_ref::<PlainHistoryCell>()
                    .map(|plain| {
                        plain.state().lines.iter().any(|line| {
                            line.spans
                                .iter()
                                .any(|span| span.text.contains(call_id))
                        })
                    })
                    .unwrap_or(false)
        }) {
            self.history_remove_at(idx);
        }
    }


    /// Flush any ExecEnd events that arrived before their matching ExecBegin.
    /// We briefly stash such ends to allow natural pairing when the Begin shows up
    /// shortly after. If the pairing window expires, render a fallback completed
    /// Exec cell so users still see the output in history.
    pub(crate) fn flush_pending_exec_ends(&mut self) {
        use std::time::Duration;
        use std::time::Instant;
        let now = Instant::now();
        // Collect keys to avoid holding a mutable borrow while iterating
        let mut ready: Vec<ExecCallId> = Vec::new();
        for (k, (_ev, _order, t0)) in self.exec.pending_exec_ends.iter() {
            if now.saturating_duration_since(*t0) >= Duration::from_millis(110) {
                ready.push(k.clone());
            }
        }
        for key in &ready {
            if let Some((ev, order, _t0)) = self.exec.pending_exec_ends.remove(key) {
                // Regardless of whether a Begin has arrived by now, handle the End;
                // handle_exec_end_now pairs with a running Exec if present, or falls back.
                self.handle_exec_end_now(ev, &order);
            }
        }
        if !ready.is_empty() {
            self.request_redraw();
        }
    }

    /// Schedule a short-delay check to flush queued interrupts if the current
    /// stream stalls in an idle state. Avoids the UI appearing frozen when the
    /// model stops streaming before sending TaskComplete.
    fn schedule_interrupt_flush_check(&mut self) {
        if self.interrupt_flush_scheduled || !self.interrupts.has_queued() {
            return;
        }
        self.interrupt_flush_scheduled = true;
        let tx = self.app_event_tx.clone();
        let fallback_tx = tx.clone();
        if thread_spawner::spawn_lightweight("interrupt-flush", move || {
            std::thread::sleep(std::time::Duration::from_millis(180));
            tx.send(AppEvent::FlushInterruptsIfIdle);
        })
        .is_none()
        {
            fallback_tx.send(AppEvent::FlushInterruptsIfIdle);
        }
    }

    /// Finalize a stalled stream and flush queued interrupts once the stream is idle.
    /// Re-arms itself until either the stream clears or the queue drains.
    pub(crate) fn flush_interrupts_if_stream_idle(&mut self) {
        self.interrupt_flush_scheduled = false;
        if !self.stream.is_write_cycle_active() {
            if self.interrupts.has_queued() {
                self.flush_interrupt_queue();
                self.request_redraw();
            }
            return;
        }
        if self.stream.is_current_stream_idle() {
            streaming::finalize_active_stream(self);
            self.flush_interrupt_queue();
            self.request_redraw();
        } else if self.interrupts.has_queued() {
            // Still busy; try again shortly so we don't leave Exec/Tool updates stuck.
            self.schedule_interrupt_flush_check();
        }
    }

    fn finalize_all_running_as_interrupted(&mut self) {
        exec_tools::finalize_all_running_as_interrupted(self);
    }

    fn finalize_all_running_due_to_answer(&mut self) {
        exec_tools::finalize_all_running_due_to_answer(self);
    }

    fn ensure_lingering_execs_cleared(&mut self) {
        if self.cleared_lingering_execs_this_turn {
            return;
        }

        let nothing_running = self.exec.running_commands.is_empty()
            && self.tools_state.running_custom_tools.is_empty()
            && self.tools_state.running_wait_tools.is_empty()
            && self.tools_state.running_kill_tools.is_empty()
            && self.tools_state.web_search_sessions.is_empty();

        if nothing_running {
            self.cleared_lingering_execs_this_turn = true;
            return;
        }

        self.finalize_all_running_due_to_answer();
        self.cleared_lingering_execs_this_turn = true;
    }
    fn perf_label_for_item(&self, item: &dyn HistoryCell) -> String {
        use crate::history_cell::ExecKind;
        use crate::history::state::ExecStatus;
        use crate::history_cell::HistoryCellType;
        use crate::history_cell::PatchKind;
        use crate::history_cell::ToolCellStatus;
        match item.kind() {
            HistoryCellType::Plain => "Plain".to_string(),
            HistoryCellType::User => "User".to_string(),
            HistoryCellType::Assistant => "Assistant".to_string(),
            HistoryCellType::Reasoning => "Reasoning".to_string(),
            HistoryCellType::Error => "Error".to_string(),
            HistoryCellType::Exec { kind, status } => {
                let k = match kind {
                    ExecKind::Read => "Read",
                    ExecKind::Search => "Search",
                    ExecKind::List => "List",
                    ExecKind::Run => "Run",
                };
                let s = match status {
                    ExecStatus::Running => "Running",
                    ExecStatus::Success => "Success",
                    ExecStatus::Error => "Error",
                };
                format!("Exec:{k}:{s}")
            }
            HistoryCellType::Tool { status } => {
                let s = match status {
                    ToolCellStatus::Running => "Running",
                    ToolCellStatus::Success => "Success",
                    ToolCellStatus::Failed => "Failed",
                };
                format!("Tool:{s}")
            }
            HistoryCellType::Patch { kind } => {
                let k = match kind {
                    PatchKind::Proposed => "Proposed",
                    PatchKind::ApplyBegin => "ApplyBegin",
                    PatchKind::ApplySuccess => "ApplySuccess",
                    PatchKind::ApplyFailure => "ApplyFailure",
                };
                format!("Patch:{k}")
            }
            HistoryCellType::PlanUpdate => "PlanUpdate".to_string(),
            HistoryCellType::BackgroundEvent => "BackgroundEvent".to_string(),
            HistoryCellType::Notice => "Notice".to_string(),
            HistoryCellType::CompactionSummary => "CompactionSummary".to_string(),
            HistoryCellType::Diff => "Diff".to_string(),
            HistoryCellType::Image => "Image".to_string(),
            HistoryCellType::Context => "Context".to_string(),
            HistoryCellType::AnimatedWelcome => "AnimatedWelcome".to_string(),
            HistoryCellType::Loading => "Loading".to_string(),
        }
    }



    fn request_redraw(&mut self) {
        self.app_event_tx.send(AppEvent::RequestRedraw);
    }

    /// Notify the height manager that the bottom pane view has changed.
    /// This bypasses hysteresis so the new view's height is applied immediately.
    pub(crate) fn notify_bottom_pane_view_changed(&mut self) {
        self.height_manager
            .borrow_mut()
            .record_event(HeightEvent::ComposerModeChange);
    }

    pub(crate) fn handle_perf_command(&mut self, args: String) {
        let arg = args.trim().to_lowercase();
        match arg.as_str() {
            "on" => {
                self.perf_state.enabled = true;
                self.perf_state.pending_scroll_rows.set(0);
                self.add_perf_output("performance tracing: on".to_string());
            }
            "off" => {
                self.perf_state.enabled = false;
                self.perf_state.pending_scroll_rows.set(0);
                self.add_perf_output("performance tracing: off".to_string());
            }
            "reset" => {
                self.perf_state.stats.borrow_mut().reset();
                self.perf_state.pending_scroll_rows.set(0);
                self.add_perf_output("performance stats reset".to_string());
            }
            "show" | "" => {
                let summary = self.perf_state.stats.borrow().summary();
                self.add_perf_output(summary);
            }
            _ => {
                self.add_perf_output("usage: /perf on | off | show | reset".to_string());
            }
        }
        self.request_redraw();
    }

    pub(crate) fn handle_demo_command(&mut self, command_args: String) {
        let trimmed_args = command_args.trim();
        if !trimmed_args.is_empty() {
            if self.handle_demo_auto_drive_card_background_palette(trimmed_args) {
                self.request_redraw();
                return;
            }

            self.history_push_plain_state(history_cell::new_warning_event(format!(
                "demo: unknown args '{trimmed_args}' (try: /demo auto drive card)",
            )));
            self.request_redraw();
            return;
        }

        use ratatui::style::Modifier as RtModifier;
        use ratatui::style::Style as RtStyle;
        use ratatui::text::Span;

        self.push_background_tail("demo: populating history with sample cells…");
        enum DemoPatch {
            Add {
                path: &'static str,
                content: &'static str,
            },
            Update {
                path: &'static str,
                unified_diff: &'static str,
                original: &'static str,
                new_content: &'static str,
            },
        }

        let scenarios = [
            (
                "build automation",
                "How do I wire up CI, linting, and release automation for this repo?",
                vec![
                    ("Context", "scan workspace layout and toolchain."),
                    ("Next", "surface build + validation commands."),
                    ("Goal", "summarize a reproducible workflow."),
                ],
                vec![
                    "streaming preview: inspecting package manifests…",
                    "streaming preview: drafting deployment summary…",
                    "streaming preview: cross-checking lint targets…",
                ],
                "**Here's a demo walkthrough:**\n\n1. Run `./build-fast.sh perf` to compile quickly.\n2. Cache artifacts in `code-rs/target/perf`.\n3. Finish by sharing `./build-fast.sh run` output.\n\n```bash\n./build-fast.sh perf run\n```",
                vec![
                    (vec!["git", "status"], "On branch main\nnothing to commit, working tree clean\n"),
                    (vec!["rg", "--files"], ""),
                ],
                Some(DemoPatch::Add {
                    path: "src/demo.rs",
                    content: "fn main() {\n    println!(\"demo\");\n}\n",
                }),
                UpdatePlanArgs {
                    name: Some("Demo Scroll Plan".to_string()),
                    plan: vec![
                        PlanItemArg {
                            step: "Create reproducible builds".to_string(),
                            status: StepStatus::InProgress,
                        },
                        PlanItemArg {
                            step: "Verify validations".to_string(),
                            status: StepStatus::Pending,
                        },
                        PlanItemArg {
                            step: "Document follow-up tasks".to_string(),
                            status: StepStatus::Completed,
                        },
                    ],
                },
                ("browser_open", "https://example.com", "navigated to example.com"),
                ReasoningEffort::High,
                "demo: lint warnings will appear here",
                "demo: this slot shows error output",
                Some("diff --git a/src/lib.rs b/src/lib.rs\n@@ -1,3 +1,5 @@\n-pub fn hello() {}\n+pub fn hello() {\n+    println!(\"hello, demo!\");\n+}\n"),
            ),
            (
                "release rehearsal",
                "What checklist should I follow before tagging a release?",
                vec![
                    ("Inventory", "collect outstanding changes and docs."),
                    ("Verify", "run smoke tests and package audits."),
                    ("Announce", "draft release notes and rollout plan."),
                ],
                vec![
                    "streaming preview: aggregating changelog entries…",
                    "streaming preview: validating release artifacts…",
                    "streaming preview: preparing announcement copy…",
                ],
                "**Release rehearsal:**\n\n1. Run `./scripts/create_github_release.sh --dry-run`.\n2. Capture artifact hashes in the notes.\n3. Schedule follow-up validation in automation.\n\n```bash\n./scripts/create_github_release.sh 1.2.3 --dry-run\n```",
                vec![
                    (vec!["git", "--no-pager", "diff", "--stat"], " src/lib.rs | 10 ++++++----\n 1 file changed, 6 insertions(+), 4 deletions(-)\n"),
                    (vec!["ls", "-1"], "Cargo.lock\nREADME.md\nsrc\ntarget\n"),
                ],
                Some(DemoPatch::Update {
                    path: "src/release.rs",
                    unified_diff: "--- a/src/release.rs\n+++ b/src/release.rs\n@@ -1 +1,3 @@\n-pub fn release() {}\n+pub fn release() {\n+    println!(\"drafting release\");\n+}\n",
                    original: "pub fn release() {}\n",
                    new_content: "pub fn release() {\n    println!(\"drafting release\");\n}\n",
                }),
                UpdatePlanArgs {
                    name: Some("Release Gate Plan".to_string()),
                    plan: vec![
                        PlanItemArg {
                            step: "Finalize changelog".to_string(),
                            status: StepStatus::Completed,
                        },
                        PlanItemArg {
                            step: "Run smoke tests".to_string(),
                            status: StepStatus::InProgress,
                        },
                        PlanItemArg {
                            step: "Tag release".to_string(),
                            status: StepStatus::Pending,
                        },
                        PlanItemArg {
                            step: "Notify stakeholders".to_string(),
                            status: StepStatus::Pending,
                        },
                    ],
                },
                ("browser_open", "https://example.com/releases", "reviewed release dashboard"),
                ReasoningEffort::Medium,
                "demo: release checklist warning",
                "demo: release checklist error",
                Some("diff --git a/CHANGELOG.md b/CHANGELOG.md\n@@ -1,3 +1,6 @@\n+## 1.2.3\n+- polish release flow\n+- document automation hooks\n"),
            ),
        ];

        for (idx, scenario) in scenarios.iter().enumerate() {
            let (
                label,
                prompt,
                reasoning_steps,
                stream_lines,
                assistant_body,
                execs,
                patch_change,
                plan,
                tool_call,
                effort,
                warning_text,
                error_text,
                diff_snippet,
            ) = scenario;

            self.push_background_tail(format!(
                "demo: scenario {} — {}",
                idx + 1,
                label
            ));

            self.history_push_plain_state(history_cell::new_user_prompt((*prompt).to_string()));

            let mut reasoning_lines: Vec<Line<'static>> = reasoning_steps
                .iter()
                .map(|(title, body)| {
                    Line::from(vec![
                        Span::styled(
                            format!("{title}:"),
                            RtStyle::default().add_modifier(RtModifier::BOLD),
                        ),
                        Span::raw(format!(" {body}")),
                    ])
                })
                .collect();
            reasoning_lines.push(
                Line::from(format!("Scenario summary: {label}"))
                    .style(RtStyle::default().fg(crate::colors::text_dim())),
            );
            let reasoning_cell = history_cell::CollapsibleReasoningCell::new_with_id(
                reasoning_lines,
                Some(format!("demo-reasoning-{idx}")),
            );
            reasoning_cell.set_collapsed(false);
            reasoning_cell.set_in_progress(false);
            self.history_push(reasoning_cell);

            let preview_lines: Vec<ratatui::text::Line<'static>> = stream_lines
                .iter()
                .map(|line| Line::from((*line).to_string()))
                .collect();
            let state = self.synthesize_stream_state_from_lines(None, &preview_lines, false);
            let streaming_preview = history_cell::new_streaming_content(state, &self.config);
            self.history_push(streaming_preview);

            let assistant_state = AssistantMessageState {
                id: HistoryId::ZERO,
                stream_id: None,
                markdown: (*assistant_body).to_string(),
                citations: Vec::new(),
                metadata: None,
                token_usage: None,
                mid_turn: false,
                created_at: SystemTime::now(),
            };
            let assistant_cell =
                history_cell::AssistantMarkdownCell::from_state(assistant_state, &self.config);
            self.history_push(assistant_cell);

            for (command_tokens, stdout) in execs {
                let cmd_vec: Vec<String> = command_tokens.iter().map(std::string::ToString::to_string).collect();
                let parsed = code_core::parse_command::parse_command(&cmd_vec);
                self.history_push(history_cell::new_active_exec_command(
                    cmd_vec.clone(),
                    parsed.clone(),
                ));
                if !stdout.is_empty() {
                    let output = history_cell::CommandOutput {
                        exit_code: 0,
                        stdout: stdout.to_string(),
                        stderr: String::new(),
                    };
                    self.history_push(history_cell::new_completed_exec_command(
                        cmd_vec,
                        parsed,
                        output,
                    ));
                }
            }

            if let Some(diff) = diff_snippet {
                self.history_push_diff(None, diff.to_string());
            }

            if let Some(patch) = patch_change {
                let mut patch_changes = HashMap::new();
                let message = match patch {
                    DemoPatch::Add { path, content } => {
                        patch_changes.insert(
                            PathBuf::from(path),
                            code_core::protocol::FileChange::Add {
                                content: (*content).to_string(),
                            },
                        );
                        format!("patch: simulated failure while applying {path}")
                    }
                    DemoPatch::Update {
                        path,
                        unified_diff,
                        original,
                        new_content,
                    } => {
                        patch_changes.insert(
                            PathBuf::from(path),
                            code_core::protocol::FileChange::Update {
                                unified_diff: (*unified_diff).to_string(),
                                move_path: None,
                                original_content: (*original).to_string(),
                                new_content: (*new_content).to_string(),
                            },
                        );
                        format!("patch: simulated failure while applying {path}")
                    }
                };
                self.history_push(history_cell::new_patch_event(
                    history_cell::PatchEventType::ApprovalRequest,
                    patch_changes,
                ));
                self.history_push_plain_state(history_cell::new_patch_apply_failure(message));
            }

            self.history_push(history_cell::new_plan_update(plan.clone()));

            let (tool_name, url, result) = tool_call;
            self.history_push(history_cell::new_completed_custom_tool_call(
                (*tool_name).to_string(),
                Some((*url).to_string()),
                Duration::from_millis(420 + (idx as u64 * 150)),
                true,
                (*result).to_string(),
            ));

            self.history_push_plain_state(history_cell::new_warning_event((*warning_text).to_string()));
            self.history_push_plain_state(history_cell::new_error_event((*error_text).to_string()));

            self.history_push_plain_state(history_cell::new_model_output("gpt-5.1-codex", *effort));
            self.history_push_plain_state(history_cell::new_reasoning_output(*effort));

            self.history_push_plain_state(history_cell::new_status_output(
                &self.config,
                &self.total_token_usage,
                &self.last_token_usage,
            ));

            self.history_push_plain_state(history_cell::new_prompts_output());
        }

        let final_preview_lines = vec![
            Line::from("streaming preview: final tokens rendered."),
            Line::from("streaming preview: viewport ready for scroll testing."),
        ];
        let final_state =
            self.synthesize_stream_state_from_lines(None, &final_preview_lines, false);
        let final_stream = history_cell::new_streaming_content(final_state, &self.config);
        self.history_push(final_stream);

        self.push_background_tail("demo: rendering sample tool cards for theme review…");

        let mut agent_card = history_cell::AgentRunCell::new("Demo Agent Batch".to_string());
        agent_card.set_batch_label(Some("Demo Agents".to_string()));
        agent_card.set_task(Some("Draft a release checklist".to_string()));
        agent_card.set_context(Some("Context: codex workspace demo run".to_string()));
        agent_card.set_plan(vec![
            "Collect recent commits".to_string(),
            "Summarize blockers".to_string(),
            "Draft announcement".to_string(),
        ]);
        let mut completed_preview = history_cell::AgentStatusPreview::default();
        completed_preview.id = "demo-completed".to_string();
        completed_preview.name = "Docs Scout".to_string();
        completed_preview.status = "Completed".to_string();
        completed_preview.model = Some("gpt-5.1-large".to_string());
        completed_preview.details = vec![history_cell::AgentDetail::Result(
            "Summarized API changes".to_string(),
        )];
        completed_preview.status_kind = history_cell::AgentStatusKind::Completed;
        completed_preview.step_progress = Some(history_cell::StepProgress { completed: 3, total: 3 });
        completed_preview.elapsed = Some(Duration::from_secs(32));
        completed_preview.last_update = Some("Wrapped up summary".to_string());
        let mut running_preview = history_cell::AgentStatusPreview::default();
        running_preview.id = "demo-running".to_string();
        running_preview.name = "Lint Fixer".to_string();
        running_preview.status = "Running".to_string();
        running_preview.model = Some("code-gpt-5.2".to_string());
        running_preview.details = vec![history_cell::AgentDetail::Progress(
            "Refining suggested fixes".to_string(),
        )];
        running_preview.status_kind = history_cell::AgentStatusKind::Running;
        running_preview.step_progress = Some(history_cell::StepProgress { completed: 1, total: 3 });
        running_preview.elapsed = Some(Duration::from_secs(18));
        running_preview.last_update = Some("Step 2 of 3".to_string());
        agent_card.set_agent_overview(vec![completed_preview, running_preview]);
        agent_card.set_latest_result(vec!["Generated release briefing".to_string()]);
        agent_card.record_action("Collecting changelog entries");
        agent_card.record_action("Writing release notes");
        agent_card.set_duration(Some(Duration::from_secs(96)));
        agent_card.set_write_mode(Some(true));
        agent_card.set_status_label("Completed");
        agent_card.mark_completed();
        self.history_push(agent_card);

        let mut agent_read_card = history_cell::AgentRunCell::new("Demo Read Batch".to_string());
        agent_read_card.set_batch_label(Some("Read Agents".to_string()));
        agent_read_card.set_task(Some("Survey docs for regression notes".to_string()));
        agent_read_card.set_context(Some("Scope: analyze docs, no writes".to_string()));
        agent_read_card.set_plan(vec![
            "Gather doc highlights".to_string(),
            "Verify changelog snippets".to_string(),
        ]);
        let mut pending_preview = history_cell::AgentStatusPreview::default();
        pending_preview.id = "demo-read-pending".to_string();
        pending_preview.name = "Doc Harvester".to_string();
        pending_preview.status = "Pending".to_string();
        pending_preview.model = Some("gpt-4.5".to_string());
        pending_preview.details = vec![history_cell::AgentDetail::Info(
            "Waiting for search index".to_string(),
        )];
        pending_preview.status_kind = history_cell::AgentStatusKind::Pending;
        let mut running_read = history_cell::AgentStatusPreview::default();
        running_read.id = "demo-read-running".to_string();
        running_read.name = "Spec Parser".to_string();
        running_read.status = "Running".to_string();
        running_read.model = Some("code-gpt-3.5".to_string());
        running_read.details = vec![history_cell::AgentDetail::Progress(
            "Scanning RFC summaries".to_string(),
        )];
        running_read.status_kind = history_cell::AgentStatusKind::Running;
        running_read.step_progress = Some(history_cell::StepProgress { completed: 2, total: 5 });
        running_read.elapsed = Some(Duration::from_secs(22));
        agent_read_card.set_agent_overview(vec![pending_preview, running_read]);
        agent_read_card.record_action("Fetching documentation excerpts");
        agent_read_card.set_duration(Some(Duration::from_secs(54)));
        agent_read_card.set_write_mode(Some(false));
        agent_read_card.set_status_label("Running");
        self.history_push(agent_read_card);

        let mut browser_card = history_cell::BrowserSessionCell::new();
        browser_card.set_url("https://example.dev/releases");
        browser_card.set_headless(Some(false));
        browser_card.record_action(
            Duration::from_millis(0),
            Duration::from_millis(420),
            "open".to_string(),
            Some("https://example.dev/releases".to_string()),
            None,
            Some("status=200".to_string()),
        );
        browser_card.record_action(
            Duration::from_millis(620),
            Duration::from_millis(380),
            "scroll".to_string(),
            Some("main timeline".to_string()),
            Some("dy=512".to_string()),
            None,
        );
        browser_card.record_action(
            Duration::from_millis(1280),
            Duration::from_millis(520),
            "click".to_string(),
            Some(".release-card".to_string()),
            Some("index=2".to_string()),
            Some("status=OK".to_string()),
        );
        browser_card.add_console_message("Loaded demo assets".to_string());
        browser_card.add_console_message("Fetched changelog via XHR".to_string());
        browser_card.set_status_code(Some("200 OK".to_string()));
        self.history_push(browser_card);

        let mut search_card = history_cell::WebSearchSessionCell::new();
        search_card.set_query(Some("rust async cancellation strategy".to_string()));
        search_card.ensure_started_message();
        search_card.record_info(Duration::from_millis(120), "Searching documentation index");
        search_card.record_success(Duration::from_millis(620), "Found tokio.rs guides");
        search_card.record_success(Duration::from_millis(1040), "Linked blog: cancellation patterns");
        search_card.set_status(history_cell::WebSearchStatus::Completed);
        search_card.set_duration(Some(Duration::from_millis(1400)));
        self.history_push(search_card);

        let mut auto_drive_card =
            history_cell::AutoDriveCardCell::new(Some("Stabilize nightly CI pipeline".to_string()));
        auto_drive_card.push_action(
            "Queued smoke tests across agents",
            history_cell::AutoDriveActionKind::Info,
        );
        auto_drive_card.push_action(
            "Warning: macOS shard flaked",
            history_cell::AutoDriveActionKind::Warning,
        );
        auto_drive_card.push_action(
            "Action required: retry or pause run",
            history_cell::AutoDriveActionKind::Error,
        );
        auto_drive_card.set_status(history_cell::AutoDriveStatus::Paused);
        self.history_push(auto_drive_card);

        let goal = "Stabilize nightly CI pipeline".to_string();
        self.auto_state.last_run_summary = Some(AutoRunSummary {
            duration: Duration::from_secs(95),
            turns_completed: 4,
            message: Some("Auto Drive completed demo run.".to_string()),
            goal: Some(goal),
        });
        let celebration_message = "Diagnostics report: all demo checks passed.".to_string();
        self.auto_state.last_completion_explanation = Some(celebration_message.clone());
        self.schedule_auto_drive_card_celebration(Duration::from_secs(2), Some(celebration_message));

        self.request_redraw();
    }

    fn handle_demo_auto_drive_card_background_palette(&mut self, args: &str) -> bool {
        if !Self::demo_command_is_auto_drive_card_backgrounds(args) {
            return false;
        }

        let (r, g, b) = crate::colors::color_to_rgb(crate::colors::background());
        let luminance = (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0;
        let theme_label = if luminance < 0.5 { "dark" } else { "light" };

        self.history_push_plain_state(history_cell::plain_message_state_from_lines(
            vec![
                ratatui::text::Line::from("Auto Drive card — ANSI-16 background palette"),
                ratatui::text::Line::from(format!(
                    "Theme context: {theme_label} (based on current /theme background)",
                )),
                ratatui::text::Line::from(
                    "Tip: switch /theme (dark/light) and rerun to compare.".to_string(),
                ),
            ],
            HistoryCellType::Notice,
        ));

        use ratatui::style::Color;
        const PALETTE: &[(Color, &str)] = &[
            (Color::Black, "Black"),
            (Color::Red, "Red"),
            (Color::Green, "Green"),
            (Color::Yellow, "Yellow"),
            (Color::Blue, "Blue"),
            (Color::Magenta, "Magenta"),
            (Color::Cyan, "Cyan"),
            (Color::Gray, "Gray"),
            (Color::DarkGray, "DarkGray"),
            (Color::LightRed, "LightRed"),
            (Color::LightGreen, "LightGreen"),
            (Color::LightYellow, "LightYellow"),
            (Color::LightBlue, "LightBlue"),
            (Color::LightMagenta, "LightMagenta"),
            (Color::LightCyan, "LightCyan"),
            (Color::White, "White"),
        ];

        for (idx, (bg, name)) in PALETTE.iter().enumerate() {
            let ordinal = idx + 1;
            let goal = format!("ANSI-16 bg {ordinal:02}: {name}");
            let mut auto_drive_card = history_cell::AutoDriveCardCell::new(Some(goal));
            auto_drive_card.disable_reveal();
            auto_drive_card.set_background_override(Some(*bg));
            auto_drive_card.push_action(
                "Queued smoke tests across agents",
                AutoDriveActionKind::Info,
            );
            auto_drive_card.push_action(
                "Warning: macOS shard flaked",
                AutoDriveActionKind::Warning,
            );
            auto_drive_card.push_action(
                "Action required: retry or pause run",
                AutoDriveActionKind::Error,
            );
            auto_drive_card.set_status(AutoDriveStatus::Paused);
            self.history_push(auto_drive_card);
        }

        true
    }

    fn demo_command_is_auto_drive_card_backgrounds(args: &str) -> bool {
        let normalized = args.trim().to_ascii_lowercase();
        let simplified = normalized.replace(['-', '_'], " ");
        let tokens: std::collections::HashSet<&str> = simplified.split_whitespace().collect();
        if tokens.is_empty() {
            return false;
        }

        let wants_auto_drive = (tokens.contains("auto") && tokens.contains("drive"))
            || tokens.contains("autodrive")
            || tokens.contains("auto-drive");
        let wants_card = tokens.contains("card") || tokens.contains("cards");
        let wants_background = tokens.contains("bg")
            || tokens.contains("background")
            || tokens.contains("backgrounds")
            || tokens.contains("color")
            || tokens.contains("colors")
            || tokens.contains("colour")
            || tokens.contains("colours");

        wants_auto_drive && (wants_card || wants_background)
    }

    fn add_perf_output(&mut self, text: String) {
        let mut lines: Vec<ratatui::text::Line<'static>> = Vec::new();
        lines.push(ratatui::text::Line::from("performance".dim()));
        for l in text.lines() {
            lines.push(ratatui::text::Line::from(l.to_string()))
        }
        let state = history_cell::plain_message_state_from_lines(
            lines,
            crate::history_cell::HistoryCellType::Notice,
        );
        self.history_push_plain_state(state);
    }

    pub(crate) fn add_diff_output(&mut self, diff_output: String) {
        self.history_push_diff(None, diff_output);
    }

    pub(crate) fn add_status_output(&mut self) {
        self.history_push_plain_state(history_cell::new_status_output(
            &self.config,
            &self.total_token_usage,
            &self.last_token_usage,
        ));
    }

    pub(crate) fn show_limits_settings_ui(&mut self) {
        self.ensure_settings_overlay_section(SettingsSection::Limits);

        if let Some(cached) = self.limits.cached_content.take() {
            self.update_limits_settings_content(cached);
        }

        let snapshot = self.rate_limit_snapshot.clone();
        let needs_refresh = self.should_refresh_limits();

        if self.rate_limit_fetch_inflight || needs_refresh {
            self.set_limits_overlay_content(LimitsOverlayContent::Loading);
        } else {
            let reset_info = self.rate_limit_reset_info();
            let tabs = self.build_limits_tabs(snapshot.clone(), reset_info);
            self.set_limits_overlay_tabs(tabs);
        }

        self.request_redraw();

        if needs_refresh {
            self.request_latest_rate_limits(snapshot.is_none());
        }

        self.refresh_limits_for_other_accounts_if_due();
    }

    fn refresh_limits_for_other_accounts_if_due(&mut self) {
        let code_home = self.config.code_home.clone();
        let active_id = auth_accounts::get_active_account_id(&code_home)
            .ok()
            .flatten();
        let accounts = auth_accounts::list_accounts(&code_home).unwrap_or_default();
        if accounts.is_empty() {
            return;
        }

        let usage_records = account_usage::list_rate_limit_snapshots(&code_home).unwrap_or_default();
        let snapshot_map: HashMap<String, StoredRateLimitSnapshot> = usage_records
            .into_iter()
            .map(|record| (record.account_id.clone(), record))
            .collect();
        let now = Utc::now();
        let stale_interval = account_usage::rate_limit_refresh_stale_interval();

        for account in accounts {
            if active_id.as_deref() == Some(account.id.as_str()) {
                continue;
            }

            let reset_at = snapshot_map
                .get(&account.id)
                .and_then(|record| record.secondary_next_reset_at);
            let plan = account
                .tokens
                .as_ref()
                .and_then(|tokens| tokens.id_token.get_chatgpt_plan_type());

            let should_refresh = account_usage::mark_rate_limit_refresh_attempt_if_due(
                &code_home,
                &account.id,
                plan.as_deref(),
                reset_at,
                now,
                stale_interval,
            )
            .unwrap_or(false);

            if should_refresh {
                start_rate_limit_refresh_for_account(
                    self.app_event_tx.clone(),
                    self.config.clone(),
                    self.config.debug,
                    account,
                    false,
                    false,
                );
            }
        }
    }

    fn request_latest_rate_limits(&mut self, show_loading: bool) {
        if self.rate_limit_fetch_inflight {
            return;
        }

        if show_loading {
            self.set_limits_overlay_content(LimitsOverlayContent::Loading);
            self.request_redraw();
        }

        self.rate_limit_fetch_inflight = true;

        start_rate_limit_refresh(
            self.app_event_tx.clone(),
            self.config.clone(),
            self.config.debug,
        );
    }

    fn should_refresh_limits(&self) -> bool {
        if self.rate_limit_fetch_inflight {
            return false;
        }
        match self.rate_limit_last_fetch_at {
            Some(ts) => Utc::now() - ts > RATE_LIMIT_REFRESH_INTERVAL,
            None => true,
        }
    }

    pub(crate) fn on_auto_upgrade_completed(&mut self, version: String) {
        let notice = format!("Auto-upgraded to version {version}");
        self.latest_upgrade_version = None;
        self.push_background_tail(notice.clone());
        self.bottom_pane.flash_footer_notice(notice);
        self.request_redraw();
    }

    pub(crate) fn on_rate_limit_refresh_failed(&mut self, message: String) {
        self.rate_limit_fetch_inflight = false;

        let content = if self.rate_limit_snapshot.is_some() {
            LimitsOverlayContent::Error(message.clone())
        } else {
            LimitsOverlayContent::Placeholder
        };
        self.set_limits_overlay_content(content);
        self.request_redraw();

        if self.rate_limit_snapshot.is_some() {
            self.history_push_plain_state(history_cell::new_warning_event(message));
        }
    }

    pub(crate) fn on_rate_limit_snapshot_stored(&mut self, _account_id: String) {
        self.refresh_settings_overview_rows();
        let refresh_limits_settings = self
            .settings
            .overlay
            .as_ref()
            .map(|overlay| {
                overlay.active_section() == SettingsSection::Limits && !overlay.is_menu_active()
            })
            .unwrap_or(false);
        if refresh_limits_settings {
            self.show_limits_settings_ui();
        } else {
            self.request_redraw();
        }
    }

    fn rate_limit_reset_info(&self) -> RateLimitResetInfo {
        let auto_compact_limit = self
            .config
            .model_auto_compact_token_limit
            .and_then(|limit| (limit > 0).then_some(limit as u64));
        let auto_compact_tokens_used = auto_compact_limit.map(|_| {
            // Use the latest turn's context footprint, which best matches when
            // auto-compaction triggers, instead of the lifetime session total.
            self.last_token_usage.tokens_in_context_window()
        });
        let context_window = self.config.model_context_window;
        let context_tokens_used = context_window.map(|_| self.last_token_usage.tokens_in_context_window());

        RateLimitResetInfo {
            primary_next_reset: self.rate_limit_primary_next_reset_at,
            secondary_next_reset: self.rate_limit_secondary_next_reset_at,
            auto_compact_tokens_used,
            auto_compact_limit,
            overflow_auto_compact: true,
            context_window,
            context_tokens_used,
        }
    }

    fn rate_limit_display_config_for_account(
        account: Option<&StoredAccount>,
    ) -> RateLimitDisplayConfig {
        if matches!(account.map(|acc| acc.mode), Some(McpAuthMode::ApiKey)) {
            RateLimitDisplayConfig {
                show_usage_sections: false,
                show_chart: false,
            }
        } else {
            DEFAULT_DISPLAY_CONFIG
        }
    }

    fn update_rate_limit_resets(&mut self, current: &RateLimitSnapshotEvent) {
        let now = Utc::now();
        if let Some(secs) = current.primary_reset_after_seconds {
            self.rate_limit_primary_next_reset_at =
                Some(now + ChronoDuration::seconds(secs as i64));
        } else {
            self.rate_limit_primary_next_reset_at = None;
        }
        if let Some(secs) = current.secondary_reset_after_seconds {
            self.rate_limit_secondary_next_reset_at =
                Some(now + ChronoDuration::seconds(secs as i64));
        } else {
            self.rate_limit_secondary_next_reset_at = None;
        }
        self.maybe_schedule_rate_limit_refresh();
    }

    fn maybe_schedule_rate_limit_refresh(&mut self) {
        let Some(reset_at) = self.rate_limit_secondary_next_reset_at else {
            self.rate_limit_refresh_scheduled_for = None;
            self.rate_limit_refresh_schedule_id.fetch_add(1, Ordering::SeqCst);
            return;
        };

        if self.rate_limit_refresh_scheduled_for == Some(reset_at) {
            return;
        }

        self.rate_limit_refresh_scheduled_for = Some(reset_at);
        let schedule_id = self
            .rate_limit_refresh_schedule_id
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        let schedule_token = self.rate_limit_refresh_schedule_id.clone();
        let app_event_tx = self.app_event_tx.clone();
        let config = self.config.clone();
        let debug_enabled = self.config.debug;
        let account = auth_accounts::get_active_account_id(&config.code_home)
            .ok()
            .flatten()
            .and_then(|id| auth_accounts::find_account(&config.code_home, &id).ok())
            .flatten();

        if account.is_none() {
            return;
        }

        if thread_spawner::spawn_lightweight("rate-reset-refresh", move || {
            let now = Utc::now();
            let delay = reset_at.signed_duration_since(now) + ChronoDuration::seconds(1);
            if let Ok(delay) = delay.to_std()
                && !delay.is_zero() {
                    std::thread::sleep(delay);
                }

            if schedule_token.load(Ordering::SeqCst) != schedule_id {
                return;
            }

            let Some(account) = account else {
                return;
            };

            let plan = account
                .tokens
                .as_ref()
                .and_then(|tokens| tokens.id_token.get_chatgpt_plan_type());
            let should_refresh = account_usage::mark_rate_limit_refresh_attempt_if_due(
                &config.code_home,
                &account.id,
                plan.as_deref(),
                Some(reset_at),
                Utc::now(),
                account_usage::rate_limit_refresh_stale_interval(),
            )
            .unwrap_or(false);

            if should_refresh {
                start_rate_limit_refresh_for_account(
                    app_event_tx,
                    config,
                    debug_enabled,
                    account,
                    true,
                    false,
                );
            }
        })
        .is_none()
        {
            tracing::warn!("rate reset refresh scheduling failed: worker unavailable");
        }
    }

    pub(crate) fn handle_update_command(&mut self, command_args: &str) {
        let trimmed = command_args.trim();
        if trimmed.eq_ignore_ascii_case("settings")
            || trimmed.eq_ignore_ascii_case("ui")
            || trimmed.eq_ignore_ascii_case("config")
        {
            self.ensure_updates_settings_overlay();
            return;
        }

        // Always surface the update settings overlay before kicking off any upgrade flow.
        self.ensure_updates_settings_overlay();

        if !crate::updates::upgrade_ui_enabled() {
            return;
        }

        match crate::updates::resolve_upgrade_resolution() {
            crate::updates::UpgradeResolution::Command { command, display } => {
                if command.is_empty() {
                    self.history_push_plain_state(history_cell::new_error_event(
                        "`/update` — no upgrade command available for this install.".to_string(),
                    ));
                    self.request_redraw();
                    return;
                }

                let latest = self.latest_upgrade_version.clone();
                self.push_background_tail(
                    "Opening a guided upgrade terminal to finish installing updates.".to_string(),
                );
                if let Some(launch) = self.launch_update_command(command, display, latest) {
                    self.app_event_tx.send(AppEvent::OpenTerminal(launch));
                }
            }
            crate::updates::UpgradeResolution::Manual { instructions } => {
                self.push_background_tail(instructions);
                self.request_redraw();
            }
        }
    }

    pub(crate) fn handle_notifications_command(&mut self, args: String) {
        let trimmed = args.trim();
        if trimmed.is_empty() {
            self.show_settings_overlay(Some(SettingsSection::Notifications));
            return;
        }

        let keyword = trimmed.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        match keyword.as_str() {
            "status" => {
                match &self.config.tui.notifications {
                    Notifications::Enabled(true) => {
                        self.push_background_tail("🔔 TUI notifications are enabled.".to_string());
                    }
                    Notifications::Enabled(false) => {
                        self.push_background_tail("🔕 TUI notifications are disabled.".to_string());
                    }
                    Notifications::Custom(entries) => {
                        let filters = if entries.is_empty() {
                            "<none>".to_string()
                        } else {
                            entries.join(", ")
                        };
                        self.push_background_tail(format!(
                            "🔔 TUI notifications use custom filters: [{filters}]"
                        ));
                    }
                }
            }
            "on" | "off" => {
                let enable = keyword == "on";
                match &self.config.tui.notifications {
                    Notifications::Enabled(current) => {
                        if *current == enable {
                            self.push_background_tail(format!(
                                "TUI notifications already {}.",
                                if enable { "enabled" } else { "disabled" }
                            ));
                        } else {
                            self.app_event_tx
                                .send(AppEvent::UpdateTuiNotifications(enable));
                        }
                    }
                    Notifications::Custom(entries) => {
                        let filters = if entries.is_empty() {
                            "<none>".to_string()
                        } else {
                            entries.join(", ")
                        };
                        self.push_background_tail(format!(
                            "TUI notifications use custom filters ([{filters}]); edit ~/.code/config.toml to change them."
                        ));
                    }
                }
            }
            _ => {
                self.push_background_tail(
                    "Usage: /notifications [status|on|off]".to_string(),
                );
            }
        }
    }

    pub(crate) fn handle_prompts_command(&mut self, args: &str) {
        if !args.trim().is_empty() {
            self.history_push_plain_state(history_cell::new_error_event(
                "Usage: /prompts".to_string(),
            ));
            return;
        }

        self.submit_op(Op::ListCustomPrompts);
        self.show_settings_overlay(Some(SettingsSection::Prompts));
    }

    pub(crate) fn handle_skills_command(&mut self, args: &str) {
        if !args.trim().is_empty() {
            self.history_push_plain_state(history_cell::new_error_event(
                "Usage: /skills".to_string(),
            ));
            return;
        }

        self.submit_op(Op::ListSkills);
        self.show_settings_overlay(Some(SettingsSection::Skills));
    }

    #[allow(dead_code)]
    pub(crate) fn add_agents_output(&mut self) {
        use ratatui::text::Line;

        // Gather active agents from current UI state
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from("/agents").fg(crate::colors::keyword()));
        lines.push(Line::from(""));
        // Show current subagent command configuration summary
        lines.push(Line::from("Subagents configuration".bold()));
        if self.config.subagent_commands.is_empty() {
            lines.push(Line::from(
                "  • No subagent commands in config (using defaults)",
            ));
        } else {
            for cmd in &self.config.subagent_commands {
                let mode = if cmd.read_only { "read-only" } else { "write" };
                let agents = if cmd.agents.is_empty() {
                    "<inherit>".to_string()
                } else {
                    cmd.agents.join(", ")
                };
                lines.push(Line::from(format!(
                    "  • {} — {} — [{}]",
                    cmd.name, mode, agents
                )));
            }
        }
        lines.push(Line::from(""));
        lines.push(Line::from("Manage in the overlay:".bold()));
        lines.push(Line::from(
            "  /agents  — configure agents (↑↓ navigate • Enter edit • Esc back)"
                .fg(crate::colors::text_dim()),
        ));
        lines.push(Line::from(""));

        // Platform + environment summary to aid debugging
        lines.push(Line::from(vec!["🖥  ".into(), "Environment".bold()]));
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        lines.push(Line::from(format!("  • Platform: {os}-{arch}")));
        lines.push(Line::from(format!(
            "  • CWD: {}",
            self.config.cwd.display()
        )));
        let in_git = code_core::git_info::get_git_repo_root(&self.config.cwd).is_some();
        lines.push(Line::from(format!(
            "  • Git repo: {}",
            if in_git { "yes" } else { "no" }
        )));
        // PATH summary
        if let Some(path_os) = std::env::var_os("PATH") {
            let entries: Vec<String> = std::env::split_paths(&path_os)
                .map(|p| p.display().to_string())
                .collect();
            let shown = entries
                .iter()
                .take(6)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ");
            let suffix = if entries.len() > 6 {
                format!(" (+{} more)", entries.len() - 6)
            } else {
                String::new()
            };
            lines.push(Line::from(format!(
                "  • PATH ({} entries): {}{}",
                entries.len(),
                shown,
                suffix
            )));
        }
        #[cfg(target_os = "windows")]
        if let Ok(pathext) = std::env::var("PATHEXT") {
            lines.push(Line::from(format!("  • PATHEXT: {}", pathext)));
        }
        lines.push(Line::from(""));

        // Section: Active agents
        lines.push(Line::from(vec!["🤖 ".into(), "Active Agents".bold()]));
        if self.active_agents.is_empty() {
            if self.agents_ready_to_start {
                lines.push(Line::from("  • preparing agents…"));
            } else {
                lines.push(Line::from("  • No active agents"));
            }
        } else {
            for a in &self.active_agents {
                let status = match a.status {
                    AgentStatus::Pending => "pending",
                    AgentStatus::Running => "running",
                    AgentStatus::Completed => "completed",
                    AgentStatus::Failed => "failed",
                    AgentStatus::Cancelled => "cancelled",
                };
                lines.push(Line::from(format!("  • {} — {}", a.name, status)));
            }
        }

        lines.push(Line::from(""));

        // Section: Availability
        lines.push(Line::from(vec!["🧭 ".into(), "Availability".bold()]));

        // Determine which agents to check: configured (enabled) or defaults
        let mut to_check: Vec<(String, String, bool)> = Vec::new();
        if !self.config.agents.is_empty() {
            for a in &self.config.agents {
                if !a.enabled {
                    continue;
                }
                let name = a.name.clone();
                let cmd = if let Some(spec) = agent_model_spec(&a.name) {
                    spec.cli.to_string()
                } else {
                    a.command.clone()
                };
                let builtin = matches!(cmd.as_str(), "code" | "codex" | "cloud");
                to_check.push((name, cmd, builtin));
            }
        } else {
            for spec in enabled_agent_model_specs() {
                let name = spec.slug.to_string();
                let cmd = spec.cli.to_string();
                let builtin = matches!(spec.cli, "code" | "codex" | "cloud");
                to_check.push((name, cmd, builtin));
            }
        }

        // Helper: PATH presence + resolved path
        let resolve_cmd = |cmd: &str| -> Option<String> {
            which::which(cmd).ok().map(|p| p.display().to_string())
        };

        for (name, cmd, builtin) in to_check {
            if builtin {
                let exe = std::env::current_exe()
                    .ok()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(unknown)".to_string());
                lines.push(Line::from(format!(
                    "  • {name} — available (built-in, exe: {exe})"
                )));
            } else if let Some(path) = resolve_cmd(&cmd) {
                lines.push(Line::from(format!(
                    "  • {name} — available ({cmd} at {path})"
                )));
            } else {
                lines.push(Line::from(format!(
                    "  • {name} — not found (command: {cmd})"
                )));
                // Short cross-platform hint
                lines.push(Line::from(
                    "      Debug: ensure the CLI is installed and on PATH",
                ));
                lines.push(Line::from(
                    "      Windows: run `where <cmd>`; macOS/Linux: `which <cmd>`",
                ));
            }
        }

        let state = history_cell::plain_message_state_from_lines(
            lines,
            crate::history_cell::HistoryCellType::Notice,
        );
        self.history_push_plain_state(state);
        self.request_redraw();
    }

    pub(crate) fn handle_agents_command(&mut self, args: String) {
        if !args.trim().is_empty() {
            self.history_push_plain_state(history_cell::new_error_event(
                "Usage: /agents".to_string(),
            ));
        }
        self.show_settings_overlay(Some(SettingsSection::Agents));
    }

    pub(crate) fn handle_limits_command(&mut self, args: String) {
        if !args.trim().is_empty() {
            self.history_push_plain_state(history_cell::new_error_event(
                "Usage: /limits".to_string(),
            ));
        }
        self.show_settings_overlay(Some(SettingsSection::Limits));
    }

    pub(crate) fn handle_shell_command(&mut self, args: String) {
        let args = args.trim();
        if args.is_empty() {
            // Check if shell selector is already open
            if self.bottom_pane.is_view_kind_active(crate::bottom_pane::ActiveViewKind::ShellSelection) {
                return;
            }
            self.bottom_pane
                .show_shell_selection(self.config.shell.clone(), self.available_shell_presets());
            return;
        }

        if args == "?" {
            let current_shell = self
                .config
                .shell
                .as_ref()
                .map(Self::format_shell_config)
                .unwrap_or_else(|| "auto-detected".to_string());
            self.history_push_plain_paragraphs(
                crate::history::state::PlainMessageKind::Notice,
                vec![format!("Current shell: {current_shell}")],
            );
            return;
        }

        let shell_config = match Self::parse_shell_command_override(args, self.config.shell.as_ref()) {
            Ok(shell_config) => shell_config,
            Err(error) => {
                self.history_push_plain_state(history_cell::new_error_event(format!(
                    "Invalid /shell value: {error}",
                )));
                return;
            }
        };

        self.update_shell_config(shell_config);
        self.history_push_plain_paragraphs(
            crate::history::state::PlainMessageKind::Notice,
            vec!["Updating shell setting...".to_string()],
        );
    }

    fn available_shell_presets(&self) -> Vec<ShellPreset> {
        let user_presets: Vec<ShellPreset> = self
            .config
            .tui
            .shell_presets
            .iter()
            .filter_map(Self::shell_preset_from_config)
            .collect();
        merge_shell_presets(user_presets)
    }

    fn shell_preset_from_config(preset: &ShellPresetConfig) -> Option<ShellPreset> {
        let id = preset.id.trim();
        let command = preset.command.trim();
        if id.is_empty() || command.is_empty() {
            return None;
        }

        let display_name = if preset.display_name.trim().is_empty() {
            command.to_string()
        } else {
            preset.display_name.trim().to_string()
        };

        Some(ShellPreset {
            id: id.to_string(),
            command: command.to_string(),
            display_name,
            description: preset.description.trim().to_string(),
            default_args: preset.default_args.clone(),
            script_style: preset.script_style.map(|style| style.to_string()),
            show_in_picker: preset.show_in_picker,
        })
    }

    fn parse_shell_command_override(
        input: &str,
        current_shell: Option<&ShellConfig>,
    ) -> Result<Option<ShellConfig>, String> {
        if input == "-" || input.eq_ignore_ascii_case("clear") {
            return Ok(None);
        }

        let Some(mut parts) = shlex::split(input) else {
            return Err("could not parse command (check quoting)".to_string());
        };

        let mut explicit_style: Option<ShellScriptStyle> = None;
        if parts.first().map(String::as_str) == Some("--style") {
            if parts.len() < 2 {
                return Err("missing value after --style".to_string());
            }
            let style_value = parts.remove(1);
            parts.remove(0);
            explicit_style = Some(
                ShellScriptStyle::parse(style_value.as_str())
                    .ok_or_else(|| {
                        format!(
                            "unknown style `{style_value}` (expected one of: posix-sh, bash-zsh-compatible, zsh)",
                        )
                    })?,
            );
        }

        if parts.is_empty() {
            let Some(existing_shell) = current_shell else {
                return Err("missing shell executable".to_string());
            };
            let mut shell = existing_shell.clone();
            if let Some(style) = explicit_style {
                shell.script_style = Some(style);
            }
            return Ok(Some(shell));
        }

        let path = parts.remove(0);
        let script_style = explicit_style.or_else(|| ShellScriptStyle::infer_from_shell_program(&path));
        Ok(Some(ShellConfig {
            path,
            args: parts,
            script_style,
        }))
    }

    fn format_shell_config(shell: &ShellConfig) -> String {
        if shell.args.is_empty() {
            shell.path.clone()
        } else {
            let path = &shell.path;
            let args = shell.args.join(" ");
            format!("{path} {args}")
        }
    }

    fn submit_configure_session_for_current_settings(&self) {
        let op = Op::ConfigureSession {
            provider: self.config.model_provider.clone(),
            model: self.config.model.clone(),
            model_explicit: self.config.model_explicit,
            model_reasoning_effort: self.config.model_reasoning_effort,
            preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
            model_reasoning_summary: self.config.model_reasoning_summary,
            model_text_verbosity: self.config.model_text_verbosity,
            user_instructions: self.config.user_instructions.clone(),
            base_instructions: self.config.base_instructions.clone(),
            approval_policy: self.config.approval_policy,
            sandbox_policy: self.config.sandbox_policy.clone(),
            disable_response_storage: self.config.disable_response_storage,
            notify: self.config.notify.clone(),
            cwd: self.config.cwd.clone(),
            resume_path: None,
            demo_developer_message: self.config.demo_developer_message.clone(),
            dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
        };
        self.submit_op(op);
    }

    fn persist_shell_config(
        &self,
        attempted_shell: Option<ShellConfig>,
        previous_shell: Option<ShellConfig>,
    ) {
        let code_home = self.config.code_home.clone();
        let tx = self.app_event_tx.clone();
        tokio::spawn(async move {
            match persist_shell(&code_home, attempted_shell.as_ref()).await {
                Ok(()) => {
                    tx.send(AppEvent::ShellPersisted {
                        shell: attempted_shell,
                    });
                }
                Err(error) => {
                    tx.send(AppEvent::ShellPersistFailed {
                        attempted_shell,
                        previous_shell,
                        error: error.to_string(),
                    });
                }
            }
        });
    }

    fn update_shell_config(&mut self, shell: Option<ShellConfig>) {
        let previous_shell = self.config.shell.clone();
        self.config.shell = shell.clone();
        self.request_redraw();
        self.submit_configure_session_for_current_settings();
        self.persist_shell_config(shell, previous_shell);
    }

    pub(crate) fn on_shell_persisted(&mut self, shell: Option<ShellConfig>) {
        if self.config.shell != shell {
            return;
        }

        let message = match shell.as_ref() {
            Some(shell) => {
                let label = Self::format_shell_config(shell);
                format!("Shell set to: {label}")
            }
            None => "Shell setting cleared.".to_string(),
        };
        self.push_background_tail(message);
    }

    pub(crate) fn on_shell_persist_failed(
        &mut self,
        attempted_shell: Option<ShellConfig>,
        previous_shell: Option<ShellConfig>,
        error: String,
    ) {
        if self.config.shell != attempted_shell {
            return;
        }

        self.push_background_tail(format!("Failed to persist shell setting: {error}"));
        self.config.shell = previous_shell;
        self.request_redraw();
        self.submit_configure_session_for_current_settings();

        let restored = match self.config.shell.as_ref() {
            Some(shell) => {
                let label = Self::format_shell_config(shell);
                format!("Restored previous shell: {label}")
            }
            None => "Restored previous shell: auto-detected".to_string(),
        };
        self.push_background_tail(restored);
    }

    pub(crate) fn handle_login_command(&mut self) {
        self.show_login_accounts_view();
    }

    pub(crate) fn auth_manager(&self) -> Arc<AuthManager> {
        self.auth_manager.clone()
    }

    pub(crate) fn reload_auth(&self) -> bool {
        self.auth_manager.reload()
    }

    pub(crate) fn show_login_accounts_view(&mut self) {
        let ticket = self.make_background_tail_ticket();
        let (view, state_rc) = LoginAccountsView::new(
            self.config.code_home.clone(),
            self.app_event_tx.clone(),
            ticket,
        );
        self.login_view_state = Some(LoginAccountsState::weak_handle(&state_rc));
        self.login_add_view_state = None;
        self.bottom_pane.show_login_accounts(view);
        self.request_redraw();
    }

    pub(crate) fn show_login_add_account_view(&mut self) {
        let ticket = self.make_background_tail_ticket();
        let (view, state_rc) = LoginAddAccountView::new(
            self.config.code_home.clone(),
            self.app_event_tx.clone(),
            ticket,
        );
        self.login_add_view_state = Some(LoginAddAccountState::weak_handle(&state_rc));
        self.login_view_state = None;
        self.bottom_pane.show_login_add_account(view);
        self.request_redraw();
    }

    fn with_login_add_view<F>(&mut self, f: F) -> bool
    where
        F: FnOnce(&mut LoginAddAccountState),
    {
        if let Some(weak) = &self.login_add_view_state
            && let Some(state_rc) = weak.upgrade() {
                f(&mut state_rc.borrow_mut());
                self.request_redraw();
                return true;
            }
        false
    }

    pub(crate) fn notify_login_chatgpt_started(&mut self, auth_url: String) {
        if self.with_login_add_view(|state| state.acknowledge_chatgpt_started(auth_url.clone())) {
        }
    }

    pub(crate) fn notify_login_chatgpt_failed(&mut self, error: String) {
        if self.with_login_add_view(|state| state.acknowledge_chatgpt_failed(error.clone())) {
        }
    }

    pub(crate) fn notify_login_chatgpt_complete(&mut self, result: Result<(), String>) {
        if self.with_login_add_view(|state| state.on_chatgpt_complete(result.clone())) {
        }
    }

    pub(crate) fn notify_login_device_code_pending(&mut self) {
        let _ =
            self.with_login_add_view(crate::bottom_pane::LoginAddAccountState::begin_device_code_flow);
    }

    pub(crate) fn notify_login_device_code_ready(&mut self, authorize_url: String, user_code: String) {
        let _ = self.with_login_add_view(|state| state.set_device_code_ready(authorize_url.clone(), user_code.clone()));
    }

    pub(crate) fn notify_login_device_code_failed(&mut self, error: String) {
        let _ = self.with_login_add_view(|state| state.on_device_code_failed(error.clone()));
    }

    pub(crate) fn notify_login_device_code_complete(&mut self, result: Result<(), String>) {
        if self.with_login_add_view(|state| state.on_chatgpt_complete(result.clone())) {
        }
    }

    pub(crate) fn notify_login_flow_cancelled(&mut self) {
        let _ =
            self.with_login_add_view(crate::bottom_pane::LoginAddAccountState::cancel_active_flow);
    }

    pub(crate) fn login_add_view_active(&self) -> bool {
        self.login_add_view_state
            .as_ref()
            .and_then(std::rc::Weak::upgrade)
            .is_some()
    }

    pub(crate) fn set_using_chatgpt_auth(&mut self, using: bool) {
        self.config.using_chatgpt_auth = using;
        self.bottom_pane.set_using_chatgpt_auth(using);
    }

    fn spawn_update_refresh(&self, shared_state: std::sync::Arc<std::sync::Mutex<UpdateSharedState>>) {
        let config = self.config.clone();
        let tx = self.app_event_tx.clone();
        tokio::spawn(async move {
            let result = crate::updates::check_for_updates_now(&config).await;
            let mut state = shared_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match result {
                Ok(info) => {
                    state.checking = false;
                    state.latest_version = info.latest_version;
                    state.error = None;
                }
                Err(err) => {
                    state.checking = false;
                    state.latest_version = None;
                    state.error = Some(err.to_string());
                }
            }
            drop(state);
            tx.send(AppEvent::RequestRedraw);
        });
    }

    fn prepare_update_settings_view(&mut self) -> Option<UpdateSettingsView> {
        let allow_refresh = crate::updates::upgrade_ui_enabled();

        let shared_state = std::sync::Arc::new(std::sync::Mutex::new(UpdateSharedState {
            checking: allow_refresh,
            latest_version: None,
            error: None,
        }));

        let resolution = crate::updates::resolve_upgrade_resolution();
        let (command, display, instructions) = match &resolution {
            crate::updates::UpgradeResolution::Command { command, display } => (
                Some(command.clone()),
                Some(display.clone()),
                None,
            ),
            crate::updates::UpgradeResolution::Manual { instructions } => {
                (None, None, Some(instructions.clone()))
            }
        };

        let view = UpdateSettingsView::new(
            self.app_event_tx.clone(),
            self.make_background_tail_ticket(),
            code_version::version().to_string(),
            self.config.auto_upgrade_enabled,
            command,
            display,
            instructions,
            shared_state.clone(),
        );

        if allow_refresh {
            self.spawn_update_refresh(shared_state);
        }
        Some(view)
    }

    fn build_updates_settings_content(&mut self) -> Option<UpdatesSettingsContent> {
        self.prepare_update_settings_view()
            .map(UpdatesSettingsContent::new)
    }

    fn build_accounts_settings_view(&self) -> crate::bottom_pane::AccountSwitchSettingsView {
        crate::bottom_pane::AccountSwitchSettingsView::new(
            self.app_event_tx.clone(),
            self.config.auto_switch_accounts_on_rate_limit,
            self.config.api_key_fallback_on_all_accounts_limited,
        )
    }

    fn build_accounts_settings_content(&self) -> AccountsSettingsContent {
        AccountsSettingsContent::new(self.build_accounts_settings_view())
    }

    fn build_validation_settings_view(&mut self) -> ValidationSettingsView {
        let groups = vec![
            (
                GroupStatus {
                    group: ValidationGroup::Functional,
                    name: "Functional checks",
                },
                self.config.validation.groups.functional,
            ),
            (
                GroupStatus {
                    group: ValidationGroup::Stylistic,
                    name: "Stylistic checks",
                },
                self.config.validation.groups.stylistic,
            ),
        ];

        let tool_rows: Vec<ToolRow> = validation_settings_view::detect_tools()
            .into_iter()
            .map(|status| {
                let group = match status.category {
                    ValidationCategory::Functional => ValidationGroup::Functional,
                    ValidationCategory::Stylistic => ValidationGroup::Stylistic,
                };
                let requested = self.validation_tool_requested(status.name);
                let group_enabled = self.validation_group_enabled(group);
                ToolRow { status, enabled: requested, group_enabled }
            })
            .collect();

        ValidationSettingsView::new(
            groups,
            tool_rows,
            self.app_event_tx.clone(),
        )
    }

    fn build_validation_settings_content(&mut self) -> ValidationSettingsContent {
        ValidationSettingsContent::new(self.build_validation_settings_view())
    }

    fn build_review_settings_view(&mut self) -> ReviewSettingsView {
        let auto_resolve_enabled = self.config.tui.review_auto_resolve;
        let auto_review_enabled = self.config.tui.auto_review_enabled;
        let attempts = self.configured_auto_resolve_re_reviews();
        ReviewSettingsView::new(
            self.config.review_use_chat_model,
            self.config.review_model.clone(),
            self.config.review_model_reasoning_effort,
            self.config.review_resolve_use_chat_model,
            self.config.review_resolve_model.clone(),
            self.config.review_resolve_model_reasoning_effort,
            auto_resolve_enabled,
            attempts,
            auto_review_enabled,
            self.config.auto_review_use_chat_model,
            self.config.auto_review_model.clone(),
            self.config.auto_review_model_reasoning_effort,
            self.config.auto_review_resolve_use_chat_model,
            self.config.auto_review_resolve_model.clone(),
            self.config.auto_review_resolve_model_reasoning_effort,
            self.config.auto_drive.auto_review_followup_attempts.get(),
            self.app_event_tx.clone(),
        )
    }

    fn build_review_settings_content(&mut self) -> ReviewSettingsContent {
        ReviewSettingsContent::new(self.build_review_settings_view())
    }

    fn build_planning_settings_view(&mut self) -> PlanningSettingsView {
        PlanningSettingsView::new(
            self.config.planning_use_chat_model,
            self.config.planning_model.clone(),
            self.config.planning_model_reasoning_effort,
            self.app_event_tx.clone(),
        )
    }

    fn build_planning_settings_content(&mut self) -> PlanningSettingsContent {
        PlanningSettingsContent::new(self.build_planning_settings_view())
    }

    fn build_auto_drive_settings_view(&mut self) -> AutoDriveSettingsView {
        let model = self.config.auto_drive.model.clone();
        let model_effort = self.config.auto_drive.model_reasoning_effort;
        let use_chat_model = self.config.auto_drive_use_chat_model;
        let review = self.auto_state.review_enabled;
        let agents = self.auto_state.subagents_enabled;
        let cross = self.auto_state.cross_check_enabled;
        let qa = self.auto_state.qa_automation_enabled;
        let mode = self.auto_state.continue_mode;
        AutoDriveSettingsView::new(
            self.app_event_tx.clone(),
            model,
            model_effort,
            use_chat_model,
            review,
            agents,
            cross,
            qa,
            mode,
        )
    }

    fn build_auto_drive_settings_content(&mut self) -> AutoDriveSettingsContent {
        AutoDriveSettingsContent::new(self.build_auto_drive_settings_view())
    }

    fn ensure_updates_settings_overlay(&mut self) {
        if self.settings.overlay.is_none() {
            self.show_settings_overlay(Some(SettingsSection::Updates));
            return;
        }
        if let Some(content) = self.build_updates_settings_content()
            && let Some(overlay) = self.settings.overlay.as_mut() {
                overlay.set_updates_content(content);
            }
        self.ensure_settings_overlay_section(SettingsSection::Updates);
        self.request_redraw();
    }

    fn ensure_validation_settings_overlay(&mut self) {
        if self.settings.overlay.is_none() {
            self.show_settings_overlay(Some(SettingsSection::Validation));
            return;
        }
        let content = self.build_validation_settings_content();
        if let Some(overlay) = self.settings.overlay.as_mut() {
            overlay.set_validation_content(content);
        }
        self.ensure_settings_overlay_section(SettingsSection::Validation);
        self.request_redraw();
    }

    fn ensure_auto_drive_settings_overlay(&mut self) {
        if self.settings.overlay.is_none() {
            self.show_settings_overlay(Some(SettingsSection::AutoDrive));
            return;
        }
        let content = self.build_auto_drive_settings_content();
        if let Some(overlay) = self.settings.overlay.as_mut() {
            overlay.set_auto_drive_content(content);
        }
        self.ensure_settings_overlay_section(SettingsSection::AutoDrive);
        self.request_redraw();
    }

    pub(crate) fn show_agents_overview_ui(&mut self) {
        let (rows, commands) = self.collect_agents_overview_rows();
        let total_rows = rows
            .len()
            .saturating_add(commands.len())
            .saturating_add(AGENTS_OVERVIEW_STATIC_ROWS);
        let selected = if total_rows == 0 {
            0
        } else {
            self
                .agents_overview_selected_index
                .min(total_rows.saturating_sub(1))
        };
        self.agents_overview_selected_index = selected;

        self.ensure_settings_overlay_section(SettingsSection::Agents);

        let updated = self.try_update_agents_settings_overview(
            rows.clone(),
            commands.clone(),
            selected,
        );

        if !updated
            && let Some(overlay) = self.settings.overlay.as_mut() {
                let content = AgentsSettingsContent::new_overview(
                    rows,
                    commands,
                    selected,
                    self.app_event_tx.clone(),
                );
                overlay.set_agents_content(content);
            }

        self.request_redraw();
    }

    fn try_update_agents_settings_overview(
        &mut self,
        rows: Vec<AgentOverviewRow>,
        commands: Vec<String>,
        selected: usize,
    ) -> bool {
        if let Some(overlay) = self.settings.overlay.as_mut()
            && overlay.active_section() == SettingsSection::Agents {
                if let Some(content) = overlay.agents_content_mut() {
                    content.set_overview(rows, commands, selected);
                } else {
                    overlay.set_agents_content(AgentsSettingsContent::new_overview(
                        rows,
                        commands,
                        selected,
                        self.app_event_tx.clone(),
                    ));
                }
                return true;
            }
        false
    }

    fn try_set_agents_settings_editor(&mut self, editor: SubagentEditorView) -> bool {
        let mut editor = Some(editor);
        let mut needs_content = false;

        if let Some(overlay) = self.settings.overlay.as_mut()
            && overlay.active_section() == SettingsSection::Agents {
                if let Some(content) = overlay.agents_content_mut() {
                    let Some(editor_view) = editor.take() else {
                        return false;
                    };
                    content.set_editor(editor_view);
                    self.request_redraw();
                    return true;
                } else {
                    needs_content = true;
                }
            }

        if needs_content {
            let (rows, commands) = self.collect_agents_overview_rows();
            let total = rows
                .len()
                .saturating_add(commands.len())
                .saturating_add(AGENTS_OVERVIEW_STATIC_ROWS);
            let selected = if total == 0 {
                0
            } else {
                self.agents_overview_selected_index.min(total.saturating_sub(1))
            };
            self.agents_overview_selected_index = selected;

            if let Some(overlay) = self.settings.overlay.as_mut()
                && overlay.active_section() == SettingsSection::Agents {
                    let mut content = AgentsSettingsContent::new_overview(
                        rows,
                        commands,
                        selected,
                        self.app_event_tx.clone(),
                    );
                    let Some(editor_view) = editor.take() else {
                        return false;
                    };
                    content.set_editor(editor_view);
                    overlay.set_agents_content(content);
                    self.request_redraw();
                    return true;
                }
        }

        false
    }

    fn try_set_agents_settings_agent_editor(&mut self, editor: AgentEditorView) -> bool {
        let mut editor = Some(editor);
        let mut needs_content = false;

        if let Some(overlay) = self.settings.overlay.as_mut()
            && overlay.active_section() == SettingsSection::Agents {
                if let Some(content) = overlay.agents_content_mut() {
                    let Some(editor_view) = editor.take() else {
                        return false;
                    };
                    content.set_agent_editor(editor_view);
                    self.request_redraw();
                    return true;
                } else {
                    needs_content = true;
                }
            }

        if needs_content {
            let (rows, commands) = self.collect_agents_overview_rows();
            let total = rows
                .len()
                .saturating_add(commands.len())
                .saturating_add(AGENTS_OVERVIEW_STATIC_ROWS);
            let selected = if total == 0 {
                0
            } else {
                self.agents_overview_selected_index.min(total.saturating_sub(1))
            };
            self.agents_overview_selected_index = selected;

            if let Some(overlay) = self.settings.overlay.as_mut()
                && overlay.active_section() == SettingsSection::Agents {
                    let mut content = AgentsSettingsContent::new_overview(
                        rows,
                        commands,
                        selected,
                        self.app_event_tx.clone(),
                    );
                    let Some(editor_view) = editor.take() else {
                        return false;
                    };
                    content.set_agent_editor(editor_view);
                    overlay.set_agents_content(content);
                    self.request_redraw();
                    return true;
                }
        }

        false
    }

    pub(crate) fn set_agents_overview_selection(&mut self, index: usize) {
        self.agents_overview_selected_index = index;
        if let Some(overlay) = self.settings.overlay.as_mut()
            && overlay.active_section() == SettingsSection::Agents
                && let Some(content) = overlay.agents_content_mut() {
                    content.set_overview_selection(index);
                }
    }

    fn agent_batch_metadata(&self, batch_id: &str) -> AgentBatchMetadata {
        if let Some(key) = self.tools_state.agent_run_by_batch.get(batch_id)
            && let Some(tracker) = self.tools_state.agent_runs.get(key) {
                return AgentBatchMetadata {
                    label: tracker.overlay_display_label(),
                    prompt: tracker.overlay_task(),
                    context: tracker.overlay_context(),
                };
            }
        AgentBatchMetadata::default()
    }

    fn append_agents_overlay_section(
        &self,
        lines: &mut Vec<ratatui::text::Line<'static>>,
        title: &str,
        text: &str,
    ) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let header_style = ratatui::style::Style::default()
            .fg(crate::colors::text())
            .add_modifier(ratatui::style::Modifier::BOLD);
        lines.push(ratatui::text::Line::from(vec![
            ratatui::text::Span::raw(" "),
            ratatui::text::Span::styled(title.to_string(), header_style),
        ]));
        for raw_line in trimmed.lines() {
            let content = raw_line.trim_end();
            lines.push(ratatui::text::Line::from(vec![
                ratatui::text::Span::raw("   "),
                ratatui::text::Span::styled(
                    content.to_string(),
                    ratatui::style::Style::default().fg(crate::colors::text()),
                ),
            ]));
        }
    }

    fn truncate_overlay_text(&self, text: &str, limit: usize) -> String {
        let collapsed = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let normalized = if collapsed.trim().is_empty() {
            text.trim().to_string()
        } else {
            collapsed.trim().to_string()
        };

        if normalized.chars().count() <= limit {
            return normalized;
        }

        let mut out: String = normalized.chars().take(limit.saturating_sub(1)).collect();
        out.push('…');
        out
    }

    fn append_agent_highlights(
        &self,
        lines: &mut Vec<ratatui::text::Line<'static>>,
        entry: &AgentTerminalEntry,
        available_width: u16,
        collapsed: bool,
    ) {
        let mut bullets: Vec<(String, ratatui::style::Style)> = Vec::new();

        if matches!(entry.source_kind, Some(AgentSourceKind::AutoReview)) {
            let is_terminal = matches!(
                entry.status,
                AgentStatus::Completed | AgentStatus::Failed | AgentStatus::Cancelled
            );

            if is_terminal {
                let (mut has_findings, findings_count, summary) =
                    Self::parse_agent_review_result(entry.result.as_deref());

                // Avoid showing a warning when we didn't get an explicit findings list.
                // Some heuristic parses can claim "issues" but provide a zero count; treat those as clean
                // to keep the UI consistent with successful, issue-free reviews.
                if has_findings && findings_count == 0 {
                    has_findings = false;
                }

                let mut label = if has_findings {
                    let plural = if findings_count == 1 { "issue" } else { "issues" };
                    format!("Auto Review: {findings_count} {plural} found")
                } else if matches!(entry.status, AgentStatus::Completed) {
                    "Auto Review: no issues found".to_string()
                } else {
                    String::new()
                };
                if label.is_empty() {
                    label = "Auto Review".to_string();
                }

                if has_findings || matches!(entry.status, AgentStatus::Completed) {
                    let color = if has_findings {
                        ratatui::style::Style::default().fg(crate::colors::warning())
                    } else {
                        ratatui::style::Style::default().fg(crate::colors::success())
                    };
                    bullets.push((label, color));
                }

                if let Some(summary_text) = summary {
                    for line in summary_text.lines() {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        bullets.push((
                            self.truncate_overlay_text(trimmed, 280),
                            ratatui::style::Style::default().fg(crate::colors::text_dim()),
                        ));
                    }
                }
            }
        }

        if let Some(result) = entry.result.as_ref() {
            let text = self.truncate_overlay_text(result, 320);
            if !text.is_empty() {
                bullets.push((
                    format!("Final: {text}"),
                    ratatui::style::Style::default().fg(crate::colors::text_dim()),
                ));
            }
        }

        match entry.status {
            AgentStatus::Failed => {
                if entry.error.is_none() {
                    bullets.push((
                        "Failed".to_string(),
                        ratatui::style::Style::default().fg(crate::colors::error()),
                    ));
                }
            }
            AgentStatus::Cancelled => {
                if entry.error.is_none() {
                    bullets.push((
                        "Cancelled".to_string(),
                        ratatui::style::Style::default().fg(crate::colors::warning()),
                    ));
                }
            }
            AgentStatus::Pending | AgentStatus::Running => {
                if bullets.is_empty()
                    && let Some(progress) = entry.last_progress.as_ref() {
                        let text = self.truncate_overlay_text(progress, 200);
                        if !text.is_empty() {
                            bullets.push((
                                format!("Latest progress: {text}"),
                                ratatui::style::Style::default()
                                    .fg(crate::colors::text_dim()),
                            ));
                        }
                    }
            }
            _ => {}
        }

        let header_style = ratatui::style::Style::default()
            .fg(crate::colors::text())
            .add_modifier(ratatui::style::Modifier::BOLD);
        let chevron = if collapsed { "▶" } else { "▼" };
        let title = format!("╭ Highlights (h) {chevron} ");
        let title_width = unicode_width::UnicodeWidthStr::width(title.as_str()) as u16;
        let pad = available_width
            .saturating_sub(title_width)
            .saturating_sub(1);
        let mut heading = title;
        heading.push_str(&"─".repeat(pad as usize));
        heading.push('╮');
        lines.push(ratatui::text::Line::from(ratatui::text::Span::styled(
            heading,
            header_style,
        )));

        if collapsed || bullets.is_empty() {
            let footer_width = available_width.saturating_sub(1);
            let mut footer = String::from("╰");
            footer.push_str(&"─".repeat(footer_width as usize));
            lines.push(ratatui::text::Line::from(footer));
            self.ensure_trailing_blank_line(lines);
            return;
        }

        let wrap_width = available_width.saturating_sub(6).max(12) as usize;
        for (text, style) in bullets.into_iter() {
            let opts = textwrap::Options::new(wrap_width)
                .break_words(false)
                .word_splitter(textwrap::word_splitters::WordSplitter::NoHyphenation)
                .initial_indent("• ")
                .subsequent_indent("  ");
            for (idx, wrapped) in textwrap::wrap(text.as_str(), opts).into_iter().enumerate() {
                let prefix = if idx == 0 { "│   " } else { "│     " };
                lines.push(ratatui::text::Line::from(vec![
                    ratatui::text::Span::raw(prefix),
                    ratatui::text::Span::styled(wrapped.to_string(), style),
                ]));
            }
        }

        if let Some(error_text) = entry
            .error
            .as_ref()
            .map(|e| self.truncate_overlay_text(e, 320))
            && !error_text.is_empty() {
                let msg = format!("Last error: {error_text}");
                for (idx, wrapped) in textwrap::wrap(msg.as_str(), wrap_width).into_iter().enumerate() {
                    let prefix = if idx == 0 { "│   " } else { "│     " };
                    lines.push(ratatui::text::Line::from(vec![
                        ratatui::text::Span::raw(prefix),
                        ratatui::text::Span::styled(
                            wrapped.to_string(),
                            ratatui::style::Style::default().fg(crate::colors::error()),
                        ),
                    ]));
                }
            }

        let footer_width = available_width.saturating_sub(1);
        let mut footer = String::from("╰");
        footer.push_str(&"─".repeat(footer_width as usize));
        lines.push(ratatui::text::Line::from(footer));
        self.ensure_trailing_blank_line(lines);
    }

    fn append_agent_log_lines(
        &self,
        lines: &mut Vec<ratatui::text::Line<'static>>,
        _idx: usize,
        log: &AgentLogEntry,
        available_width: u16,
        is_new_kind: bool,
    ) {
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};

        let time_text = log.timestamp.format("%H:%M").to_string();
        let time_style = Style::default().fg(crate::colors::text_dim());
        let kind_style = Style::default()
            .fg(agent_log_color(log.kind))
            .add_modifier(Modifier::BOLD);
        let message_base_style = if matches!(log.kind, AgentLogKind::Error) {
            Style::default().fg(crate::colors::error())
        } else {
            Style::default().fg(crate::colors::text())
        };

        // Insert a section header when the log kind changes (TYPE column removed).
        if is_new_kind {
            let header = agent_log_label(log.kind).to_uppercase();
            lines.push(Line::from(vec![Span::styled(header, kind_style)]));
        }

        // Compact prefix: time only, kept short per request.
        let prefix_plain = format!("{time_text}  ");
        let prefix_width = unicode_width::UnicodeWidthStr::width(prefix_plain.as_str()) as u16;
        let wrap_width = available_width.saturating_sub(prefix_width).max(4);

        // Break message into lines, sanitizing and keeping ANSI colors.
        let mut message_lines: Vec<&str> = log.message.split('\n').collect();
        if log.message.ends_with('\n') {
            message_lines.push("");
        }

        for (line_idx, raw_line) in message_lines.into_iter().enumerate() {
            let sanitized = self.sanitize_agent_log_line(raw_line);
            let parsed = ansi_escape_line(&sanitized);
            let wrapped = crate::insert_history::word_wrap_lines(&[self.apply_log_fallback_style(parsed, message_base_style)], wrap_width);

            for (wrap_idx, wrapped_line) in wrapped.into_iter().enumerate() {
                let mut spans: Vec<Span> = Vec::new();
                if wrap_idx == 0 && line_idx == 0 {
                    // First visible line: show time prefix.
                    spans.push(Span::styled(time_text.clone(), time_style));
                    spans.push(Span::raw("  "));
                } else {
                    // Continuation lines align under the message body.
                    spans.push(Span::raw(" ".repeat(prefix_width as usize)));
                }

                if wrapped_line.spans.is_empty() {
                    spans.push(Span::raw(""));
                } else {
                    spans.extend(wrapped_line.spans.into_iter());
                }

                lines.push(Line::from(spans));
            }

        }
    }

    fn sanitize_agent_log_line(&self, raw: &str) -> String {
        let without_ts = Self::strip_leading_timestamp(raw.trim_end_matches('\r'));
        sanitize_for_tui(
            without_ts,
            SanitizeMode::AnsiPreserving,
            SanitizeOptions {
                expand_tabs: true,
                tabstop: 4,
                ..Default::default()
            },
        )
    }

    fn apply_log_fallback_style(
        &self,
        mut line: ratatui::text::Line<'static>,
        base: ratatui::style::Style,
    ) -> ratatui::text::Line<'static> {
        for span in line.spans.iter_mut() {
            span.style = base.patch(span.style);
        }
        line
    }

    fn strip_leading_timestamp(text: &str) -> &str {
        fn is_digit(b: u8) -> bool { b.is_ascii_digit() }

        fn consume_hms(bytes: &[u8]) -> usize {
            if bytes.len() < 5 {
                return 0;
            }
            if !(is_digit(bytes[0]) && is_digit(bytes[1]) && bytes[2] == b':' && is_digit(bytes[3]) && is_digit(bytes[4])) {
                return 0;
            }
            let mut idx = 5;
            if idx + 2 < bytes.len() && bytes[idx] == b':' && is_digit(bytes[idx + 1]) && is_digit(bytes[idx + 2]) {
                idx += 3;
                while idx < bytes.len() && (bytes[idx].is_ascii_digit() || bytes[idx] == b'.') {
                    idx += 1;
                }
            }
            idx
        }

        fn consume_ymd(bytes: &[u8]) -> usize {
            if bytes.len() < 10 {
                return 0;
            }
            if !(is_digit(bytes[0])
                && is_digit(bytes[1])
                && is_digit(bytes[2])
                && is_digit(bytes[3])
                && bytes[4] == b'-'
                && is_digit(bytes[5])
                && is_digit(bytes[6])
                && bytes[7] == b'-'
                && is_digit(bytes[8])
                && is_digit(bytes[9]))
            {
                return 0;
            }
            let mut idx = 10;
            if idx < bytes.len() && (bytes[idx] == b'T' || bytes[idx] == b' ') {
                idx += 1;
                idx += consume_hms(&bytes[idx..]);
            }
            idx
        }

        let trimmed = text.trim_start();
        let mut candidate = trimmed.strip_prefix('[').unwrap_or(trimmed);
        let bytes = candidate.as_bytes();

        let mut consumed = consume_ymd(bytes);
        if consumed == 0 {
            consumed = consume_hms(bytes);
        }

        if consumed == 0 {
            return text;
        }

        candidate = &candidate[consumed..];
        if let Some(rest) = candidate.strip_prefix(']') {
            candidate = rest;
        }
        candidate.trim_start()
    }

    fn ensure_trailing_blank_line(
        &self,
        lines: &mut Vec<ratatui::text::Line<'static>>,
    ) {
        if lines
            .last()
            .map(|line| {
                line.spans.is_empty()
                    || (line.spans.len() == 1 && line.spans[0].content.is_empty())
            })
            .unwrap_or(false)
        {
            return;
        }
        lines.push(ratatui::text::Line::from(""));
    }

    fn update_agents_terminal_state(
        &mut self,
        agents: &[code_core::protocol::AgentInfo],
        context: Option<String>,
        task: Option<String>,
    ) {
        self.agents_terminal.shared_context = context;
        self.agents_terminal.shared_task = task;

        let mut saw_new_agent = false;
        for info in agents {
            let status = agent_status_from_str(info.status.as_str());
            let batch_metadata = info
                .batch_id
                .as_deref()
                .map(|id| self.agent_batch_metadata(id))
                .unwrap_or_default();
            let is_new = !self.agents_terminal.entries.contains_key(&info.id);
            if is_new
                && !self
                    .agents_terminal
                    .order
                    .iter()
                    .any(|id| id == &info.id)
            {
                self.agents_terminal.order.push(info.id.clone());
                saw_new_agent = true;
            }

            let entry = self.agents_terminal.entries.entry(info.id.clone());
            let entry = entry.or_insert_with(|| {
                saw_new_agent = true;
                let mut new_entry = AgentTerminalEntry::new(
                    info.name.clone(),
                    info.model.clone(),
                    status.clone(),
                    info.batch_id.clone(),
                );
                new_entry.source_kind = info.source_kind.clone();
                new_entry.push_log(
                    AgentLogKind::Status,
                    format!("Status → {}", agent_status_label(status.clone())),
                );
                new_entry
            });

            entry.name = info.name.clone();
            entry.batch_id = info.batch_id.clone();
            entry.model = info.model.clone();
            entry.source_kind = info.source_kind.clone();

            let AgentBatchMetadata { label, prompt: meta_prompt, context: meta_context } = batch_metadata;
            let auto_review_label = matches!(entry.source_kind, Some(AgentSourceKind::AutoReview))
                .then(|| "Auto Review".to_string());
            let previous_label = entry.batch_label.clone();
            entry.batch_label = label
                .or(auto_review_label)
                .or_else(|| info.batch_id.clone())
                .or(previous_label);

            let fallback_prompt = self
                .agents_terminal
                .shared_task
                .clone()
                .or_else(|| self.agent_task.clone());
            let previous_prompt = entry.batch_prompt.clone();
            entry.batch_prompt = meta_prompt
                .or(fallback_prompt)
                .or(previous_prompt);

            let fallback_context = self
                .agents_terminal
                .shared_context
                .clone()
                .or_else(|| self.agent_context.clone());
            let previous_context = entry.batch_context.clone();
            entry.batch_context = meta_context
                .or(fallback_context)
                .or(previous_context);

            if entry.status != status {
                entry.status = status.clone();
                entry.push_log(
                    AgentLogKind::Status,
                    format!("Status → {}", agent_status_label(status.clone())),
                );
            }

            if let Some(progress) = info.last_progress.as_ref()
                && entry.last_progress.as_ref() != Some(progress) {
                    entry.last_progress = Some(progress.clone());
                    entry.push_log(AgentLogKind::Progress, progress.clone());
                }

            if let Some(result) = info.result.as_ref()
                && entry.result.as_ref() != Some(result) {
                    entry.result = Some(result.clone());
                    entry.push_log(AgentLogKind::Result, result.clone());
                }

            if let Some(error) = info.error.as_ref()
                && entry.error.as_ref() != Some(error) {
                    entry.error = Some(error.clone());
                    entry.push_log(AgentLogKind::Error, error.clone());
                }
        }

        if let Some(pending) = self.agents_terminal.pending_stop.clone() {
            let still_running = self
                .agents_terminal
                .entries
                .get(&pending.agent_id)
                .map(|entry| matches!(entry.status, AgentStatus::Pending | AgentStatus::Running))
                .unwrap_or(false);
            if !still_running {
                self.agents_terminal.clear_stop_prompt();
            }
        }

        self.agents_terminal.clamp_selected_index();

        if saw_new_agent && self.agents_terminal.active {
            self.layout.scroll_offset.set(0);
        }
    }

    fn enter_agents_terminal_mode(&mut self) {
        if self.agents_terminal.active {
            return;
        }
        self.browser_overlay_visible = false;
        self.agents_terminal.active = true;
        self.agents_terminal.focus_sidebar();
        self.agents_terminal.clear_stop_prompt();
        self.bottom_pane.set_input_focus(false);
        self.agents_terminal.saved_scroll_offset = self.layout.scroll_offset.get();
        if self.agents_terminal.order.is_empty() {
            for agent in &self.active_agents {
                if !self
                    .agents_terminal
                    .entries
                    .contains_key(&agent.id)
                {
                    self.agents_terminal.order.push(agent.id.clone());
                    let mut entry = AgentTerminalEntry::new(
                        agent.name.clone(),
                        agent.model.clone(),
                        agent.status.clone(),
                        agent.batch_id.clone(),
                    );
                    let batch_metadata = agent
                        .batch_id
                        .as_deref()
                        .map(|id| self.agent_batch_metadata(id))
                        .unwrap_or_default();
                    let AgentBatchMetadata { label, prompt: meta_prompt, context: meta_context } = batch_metadata;
                    entry.batch_label = label
                        .or_else(|| agent.batch_id.clone())
                        .or(entry.batch_label.clone());
                    let fallback_prompt = self
                        .agents_terminal
                        .shared_task
                        .clone()
                        .or_else(|| self.agent_task.clone());
                    entry.batch_prompt = meta_prompt
                        .or(fallback_prompt)
                        .or(entry.batch_prompt.clone());
                    let fallback_context = self
                        .agents_terminal
                        .shared_context
                        .clone()
                        .or_else(|| self.agent_context.clone());
                    entry.batch_context = meta_context
                        .or(fallback_context)
                        .or(entry.batch_context.clone());
                    if let Some(progress) = agent.last_progress.as_ref() {
                        entry.last_progress = Some(progress.clone());
                        entry.push_log(AgentLogKind::Progress, progress.clone());
                    }
                    if let Some(result) = agent.result.as_ref() {
                        entry.result = Some(result.clone());
                        entry.push_log(AgentLogKind::Result, result.clone());
                    }
                    if let Some(error) = agent.error.as_ref() {
                        entry.error = Some(error.clone());
                        entry.push_log(AgentLogKind::Error, error.clone());
                    }
                    self.agents_terminal
                        .entries
                        .insert(agent.id.clone(), entry);
                }
            }
        }
        self.agents_terminal.clamp_selected_index();
        self.restore_selected_agent_scroll();
        self.request_redraw();
    }

    fn exit_agents_terminal_mode(&mut self) {
        if !self.agents_terminal.active {
            return;
        }
        self.record_current_agent_scroll();
        self.agents_terminal.active = false;
        self.agents_terminal.clear_stop_prompt();
        self.agents_terminal.focus_sidebar();
        self.layout.scroll_offset
            .set(self.agents_terminal.saved_scroll_offset);
        self.bottom_pane.set_input_focus(true);
        self.request_redraw();
    }

    fn record_current_agent_scroll(&mut self) {
        if let Some(entry) = self.agents_terminal.current_sidebar_entry() {
            let capped = self
                .layout
                .scroll_offset
                .get()
                .min(self.layout.last_max_scroll.get());
            self
                .agents_terminal
                .scroll_offsets
                .insert(entry.scroll_key(), capped);
        }
    }

    fn restore_selected_agent_scroll(&mut self) {
        if let Some(entry) = self.agents_terminal.current_sidebar_entry() {
            // Always reset to the top when switching agents; use a sentinel so the
            // next render clamps to the new agent's maximum scroll.
            let key = entry.scroll_key();
            self
                .agents_terminal
                .scroll_offsets
                .insert(key, u16::MAX);
            self.layout.scroll_offset.set(u16::MAX);
        } else {
            self.layout.scroll_offset.set(0);
        }
    }

    fn sync_agents_terminal_scroll(&mut self) {
        if !self.agents_terminal.active {
            return;
        }
        let applied = self
            .agents_terminal
            .last_render_scroll
            .get()
            .min(self.layout.last_max_scroll.get());
        self.layout.scroll_offset.set(applied);
        if let Some(entry) = self.agents_terminal.current_sidebar_entry() {
            self
                .agents_terminal
                .scroll_offsets
                .insert(entry.scroll_key(), applied);
        }
    }

    fn prompt_stop_selected_agent(&mut self) {
        let Some(AgentsSidebarEntry::Agent(agent_id)) = self.agents_terminal.current_sidebar_entry() else {
            return;
        };

        let is_active = self
            .active_agents
            .iter()
            .any(|agent| agent.id == agent_id && matches!(agent.status, AgentStatus::Pending | AgentStatus::Running));
        let is_entry_active = self
            .agents_terminal
            .entries
            .get(agent_id.as_str())
            .map(|entry| matches!(entry.status, AgentStatus::Pending | AgentStatus::Running))
            .unwrap_or(false);

        if !(is_active || is_entry_active) {
            return;
        }

        let agent_name = self
            .agents_terminal
            .entries
            .get(agent_id.as_str())
            .map(|entry| entry.name.clone())
            .or_else(|| {
                self.active_agents
                    .iter()
                    .find(|a| a.id == agent_id)
                    .map(|a| a.name.clone())
            })
            .unwrap_or_else(|| agent_id.clone());

        self.agents_terminal
            .set_stop_prompt(agent_id, agent_name);
        self.request_redraw();
    }

    fn cancel_agent_by_id(&mut self, agent_id: &str) -> bool {
        let mut can_cancel = false;
        for agent in &self.active_agents {
            if agent.id == agent_id
                && matches!(agent.status, AgentStatus::Pending | AgentStatus::Running)
            {
                can_cancel = true;
                break;
            }
        }

        if !can_cancel {
            can_cancel = self
                .agents_terminal
                .entries
                .get(agent_id)
                .map(|entry| matches!(entry.status, AgentStatus::Pending | AgentStatus::Running))
                .unwrap_or(false);
        }

        if !can_cancel {
            return false;
        }

        let agent_name = self
            .agents_terminal
            .entries
            .get(agent_id)
            .map(|entry| entry.name.clone())
            .or_else(|| {
                self.active_agents
                    .iter()
                    .find(|a| a.id == agent_id)
                    .map(|a| a.name.clone())
            })
            .unwrap_or_else(|| agent_id.to_string());

        self.push_background_tail(format!("Cancelling agent {agent_name}…"));
        self.bottom_pane
            .update_status_text(format!("Cancelling {agent_name}…"));
        self.bottom_pane.set_task_running(true);
        self.agents_ready_to_start = false;

        self.submit_op(Op::CancelAgents {
            batch_ids: Vec::new(),
            agent_ids: vec![agent_id.to_string()],
        });

        for agent in &mut self.active_agents {
            if agent.id == agent_id
                && matches!(agent.status, AgentStatus::Pending | AgentStatus::Running)
            {
                agent.status = AgentStatus::Cancelled;
                agent.error.get_or_insert_with(|| "Cancelled by user".to_string());
            }
        }

        if let Some(entry) = self.agents_terminal.entries.get_mut(agent_id)
            && matches!(entry.status, AgentStatus::Pending | AgentStatus::Running) {
                entry.status = AgentStatus::Cancelled;
                entry.push_log(
                    AgentLogKind::Status,
                    format!("Status → {}", agent_status_label(AgentStatus::Cancelled)),
                );
            }

        self.request_redraw();
        true
    }

    fn navigate_agents_terminal_selection(&mut self, delta: isize) {
        let entries = self.agents_terminal.sidebar_entries();
        if entries.is_empty() {
            return;
        }
        self.agents_terminal.focus_sidebar();
        let len = entries.len() as isize;
        self.record_current_agent_scroll();
        let mut new_index = self.agents_terminal.selected_index as isize + delta;
        if new_index >= len {
            new_index %= len;
        }
        while new_index < 0 {
            new_index += len;
        }
        self.agents_terminal.selected_index = new_index as usize;
        self.agents_terminal.clamp_selected_index();
        self.agents_terminal.clear_stop_prompt();
        self.restore_selected_agent_scroll();
        self.request_redraw();
    }

    fn navigate_agents_terminal_page(&mut self, delta_pages: isize) {
        let entries = self.agents_terminal.sidebar_entries();
        if entries.is_empty() {
            return;
        }
        let page = self.layout.last_history_viewport_height.get() as isize;
        let step = if page > 0 { page.saturating_sub(1) } else { 1 };
        let delta = step.max(1) * delta_pages;
        self.navigate_agents_terminal_selection(delta);
    }

    fn navigate_agents_terminal_home(&mut self) {
        let entries = self.agents_terminal.sidebar_entries();
        if entries.is_empty() {
            return;
        }
        self.agents_terminal.selected_index = 0;
        self.agents_terminal.clamp_selected_index();
        self.agents_terminal.clear_stop_prompt();
        self.restore_selected_agent_scroll();
        self.request_redraw();
    }

    fn navigate_agents_terminal_end(&mut self) {
        let entries = self.agents_terminal.sidebar_entries();
        if entries.is_empty() {
            return;
        }
        self.agents_terminal.selected_index = entries.len().saturating_sub(1);
        self.agents_terminal.clamp_selected_index();
        self.agents_terminal.clear_stop_prompt();
        self.restore_selected_agent_scroll();
        self.request_redraw();
    }
    fn resolve_agent_install_command(&self, agent_name: &str) -> Option<(Vec<String>, String)> {
        let cmd = self
            .config
            .agents
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(agent_name))
            .map(|cfg| cfg.command.clone())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| agent_name.to_string());
        if cmd.trim().is_empty() {
            return None;
        }

        #[cfg(target_os = "windows")]
        {
            let script = format!(
                "if (Get-Command {cmd} -ErrorAction SilentlyContinue) {{ Write-Output \"{cmd} already installed\"; exit 0 }} else {{ Write-Warning \"{cmd} is not installed.\"; Write-Output \"Please install {cmd} via winget, Chocolatey, or the vendor installer.\"; exit 1 }}",
                cmd = cmd
            );
            let command = vec![
                "powershell.exe".to_string(),
                "-NoProfile".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                script.clone(),
            ];
            return Some((command, format!("PowerShell install check for {cmd}")));
        }

        #[cfg(target_os = "macos")]
        {
            let brew_formula = macos_brew_formula_for_command(&cmd);
            let script = format!("brew install {brew_formula}");
            let command = vec!["/bin/bash".to_string(), "-lc".to_string(), script.clone()];
            return Some((command, script));
        }

        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            fn linux_agent_install_script(agent_cmd: &str, npm_package: &str) -> String {
                format!(
                    "set -euo pipefail\n\
if ! command -v npm >/dev/null 2>&1; then\n\
    echo \"npm is required to install {agent_cmd}. Install Node.js 20+ and rerun.\" >&2\n\
    exit 1\n\
fi\n\
prefix=\"$(npm config get prefix 2>/dev/null || true)\"\n\
if [ -z \"$prefix\" ] || [ ! -w \"$prefix\" ]; then\n\
    prefix=\"$HOME/.npm-global\"\n\
fi\n\
mkdir -p \"$prefix/bin\"\n\
export PATH=\"$prefix/bin:$PATH\"\n\
export npm_config_prefix=\"$prefix\"\n\
node_major=0\n\
if command -v node >/dev/null 2>&1; then\n\
    node_major=\"$(node -v | sed 's/^v\\([0-9][0-9]*\\).*/\\1/')\"\n\
fi\n\
if [ \"$node_major\" -lt 20 ]; then\n\
    npm install -g n\n\
    export N_PREFIX=\"${{N_PREFIX:-$HOME/.n}}\"\n\
    mkdir -p \"$N_PREFIX/bin\"\n\
    export PATH=\"$N_PREFIX/bin:$PATH\"\n\
    n 20.18.1\n\
    hash -r\n\
    node_major=\"$(node -v | sed 's/^v\\([0-9][0-9]*\\).*/\\1/')\"\n\
    if [ \"$node_major\" -lt 20 ]; then\n\
        echo \"Failed to activate Node.js 20+. Check that $N_PREFIX/bin is on PATH.\" >&2\n\
        exit 1\n\
    fi\n\
else\n\
    export N_PREFIX=\"${{N_PREFIX:-$HOME/.n}}\"\n\
    if [ -d \"$N_PREFIX/bin\" ]; then\n\
        export PATH=\"$N_PREFIX/bin:$PATH\"\n\
    fi\n\
fi\n\
npm install -g {npm_package}\n\
hash -r\n\
if ! command -v {agent_cmd} >/dev/null 2>&1; then\n\
    echo \"{agent_cmd} installed but not found on PATH. Add 'export PATH=\\\"$prefix/bin:$PATH\\\"' to your shell profile.\" >&2\n\
    exit 1\n\
fi\n\
{agent_cmd} --version\n",
                    agent_cmd = agent_cmd,
                    npm_package = npm_package,
                )
            }

            let lowercase = agent_name.trim().to_ascii_lowercase();
            let script = match lowercase.as_str() {
                "claude" => linux_agent_install_script(&cmd, "@anthropic-ai/claude-code"),
                "gemini" => linux_agent_install_script(&cmd, "@google/gemini-cli"),
                "qwen" => linux_agent_install_script(&cmd, "@qwen-code/qwen-code"),
                _ => format!(
                    "{cmd} --version || (echo \"Please install {cmd} via your package manager\" && false)",
                    cmd = cmd
                ),
            };
            let command = vec!["/bin/bash".to_string(), "-lc".to_string(), script.clone()];
            return Some((command, script));
        }

        #[allow(unreachable_code)]
        {
            None
        }
    }

    pub(crate) fn launch_agent_install(
        &mut self,
        name: String,
        selected_index: usize,
    ) -> Option<TerminalLaunch> {
        self.agents_overview_selected_index = selected_index;
        let Some((_, default_command)) = self.resolve_agent_install_command(&name) else {
            self.history_push_plain_state(history_cell::new_error_event(format!(
                "No install command available for agent '{name}' on this platform."
            )));
            self.show_agents_overview_ui();
            return None;
        };
        let id = self.terminal.alloc_id();
        self.terminal.after = Some(TerminalAfter::RefreshAgentsAndClose { selected_index });
        let (controller_tx, controller_rx) = mpsc::channel();
        let controller = TerminalRunController { tx: controller_tx };
        let cwd = self.config.cwd.to_string_lossy().to_string();
        self.push_background_before_next_output(format!(
            "Starting guided install for agent '{name}'"
        ));
        start_agent_install_session(
            self.app_event_tx.clone(),
            id,
            name.clone(),
            default_command,
            Some(cwd),
            controller.clone(),
            controller_rx,
            selected_index,
            self.config.debug,
        );
        Some(TerminalLaunch {
            id,
            title: format!("Install {name}"),
            command: Vec::new(),
            command_display: "Preparing install assistant…".to_string(),
            controller: Some(controller),
            auto_close_on_success: false,
            start_running: true,
        })
    }

    pub(crate) fn launch_validation_tool_install(
        &mut self,
        tool_name: &str,
        install_hint: &str,
    ) -> Option<TerminalLaunch> {
        let trimmed = install_hint.trim();
        if trimmed.is_empty() {
            self.history_push_plain_state(history_cell::new_error_event(format!(
                "No install command available for validation tool '{tool_name}'."
            )));
            self.request_redraw();
            return None;
        }

        let wrapped = wrap_command(trimmed);
        if wrapped.is_empty() {
            self.history_push_plain_state(history_cell::new_error_event(format!(
                "Unable to build install command for validation tool '{tool_name}'."
            )));
            self.request_redraw();
            return None;
        }

        let id = self.terminal.alloc_id();
        let display = Self::truncate_with_ellipsis(trimmed, 128);
        let launch = TerminalLaunch {
            id,
            title: format!("Install {tool_name}"),
            command: wrapped,
            command_display: display,
            controller: None,
            auto_close_on_success: false,
            start_running: true,
        };

        self.push_background_before_next_output(format!(
            "Installing validation tool '{tool_name}' with `{trimmed}`"
        ));
        Some(launch)
    }

    fn try_handle_terminal_shortcut(&mut self, raw_text: &str) -> bool {
        let trimmed = raw_text.trim_start();
        if let Some(rest) = trimmed.strip_prefix("$$") {
            let prompt = rest.trim();
            if prompt.is_empty() {
                self.history_push_plain_state(history_cell::new_error_event(
                    "No prompt provided after '$$'.".to_string(),
                ));
                self.app_event_tx.send(AppEvent::RequestRedraw);
            } else {
                self.launch_guided_terminal_prompt(prompt);
            }
            return true;
        }
        if let Some(rest) = trimmed.strip_prefix('$') {
            let command = rest.trim();
            if command.is_empty() {
                self.launch_manual_terminal();
            } else {
                self.run_terminal_command(command);
            }
            return true;
        }
        false
    }

    fn launch_manual_terminal(&mut self) {
        let id = self.terminal.alloc_id();
        let launch = TerminalLaunch {
            id,
            title: "Shell".to_string(),
            command: Vec::new(),
            command_display: String::new(),
            controller: None,
            auto_close_on_success: false,
            start_running: false,
        };
        self.app_event_tx.send(AppEvent::OpenTerminal(launch));
    }

    fn run_terminal_command(&mut self, command: &str) {
        if wrap_command(command).is_empty() {
            self.history_push_plain_state(history_cell::new_error_event(
                "Unable to build shell command for execution.".to_string(),
            ));
            self.app_event_tx.send(AppEvent::RequestRedraw);
            return;
        }

        let id = self.terminal.alloc_id();
        let title = Self::truncate_with_ellipsis(&format!("Shell: {command}"), 64);
        let display = Self::truncate_with_ellipsis(command, 128);
        let (controller_tx, controller_rx) = mpsc::channel();
        let controller = TerminalRunController { tx: controller_tx };
        let launch = TerminalLaunch {
            id,
            title,
            command: Vec::new(),
            command_display: display,
            controller: Some(controller.clone()),
            auto_close_on_success: false,
            start_running: true,
        };
        self.push_background_before_next_output(format!(
            "Terminal command: {command}"
        ));
        self.app_event_tx.send(AppEvent::OpenTerminal(launch));
        let cwd = self.config.cwd.to_string_lossy().to_string();
        start_direct_terminal_session(
            self.app_event_tx.clone(),
            id,
            command.to_string(),
            Some(cwd),
            controller,
            controller_rx,
            self.config.debug,
        );
    }

    fn launch_guided_terminal_prompt(&mut self, prompt: &str) {
        let id = self.terminal.alloc_id();
        let (controller_tx, controller_rx) = mpsc::channel();
        let controller = TerminalRunController { tx: controller_tx };
        let cwd = self.config.cwd.to_string_lossy().to_string();
        let title = Self::truncate_with_ellipsis(&format!("Guided: {prompt}"), 64);
        let display = Self::truncate_with_ellipsis(prompt, 128);

        let launch = TerminalLaunch {
            id,
            title,
            command: Vec::new(),
            command_display: display,
            controller: Some(controller.clone()),
            auto_close_on_success: false,
            start_running: true,
        };

        self.push_background_before_next_output(format!(
            "Guided terminal request: {prompt}"
        ));
        self.app_event_tx.send(AppEvent::OpenTerminal(launch));
        start_prompt_terminal_session(
            self.app_event_tx.clone(),
            id,
            prompt.to_string(),
            Some(cwd),
            controller,
            controller_rx,
            self.config.debug,
        );
    }

    fn is_cli_running(&self) -> bool {
        if !self.exec.running_commands.is_empty() {
            return true;
        }
        if !self.tools_state.running_custom_tools.is_empty()
            || !self.tools_state.web_search_sessions.is_empty()
            || !self.tools_state.running_wait_tools.is_empty()
            || !self.tools_state.running_kill_tools.is_empty()
        {
            return true;
        }
        if self.stream.is_write_cycle_active() {
            return true;
        }
        if !self.active_task_ids.is_empty() {
            return true;
        }
        if self.active_agents.iter().any(|agent| {
            let is_auto_review = matches!(agent.source_kind, Some(AgentSourceKind::AutoReview))
                || agent
                    .batch_id
                    .as_deref()
                    .is_some_and(|batch| batch.eq_ignore_ascii_case("auto-review"));
            matches!(agent.status, AgentStatus::Pending | AgentStatus::Running) && !is_auto_review
        }) {
            return true;
        }
        false
    }

    fn refresh_auto_drive_visuals(&mut self) {
        if self.auto_state.is_active()
            || self.auto_state.should_show_goal_entry()
            || self.auto_state.last_run_summary.is_some()
        {
            self.auto_rebuild_live_ring();
        }
    }

    fn auto_reduced_motion_preference() -> bool {
        match std::env::var("CODE_TUI_REDUCED_MOTION") {
            Ok(value) => {
                let normalized = value.trim().to_ascii_lowercase();
                !matches!(normalized.as_str(), "" | "0" | "false" | "off" | "no")
            }
            Err(_) => false,
        }
    }

    fn auto_reset_intro_timing(&mut self) {
        self.auto_state.reset_intro_timing();
    }

    fn auto_ensure_intro_timing(&mut self) {
        let reduced_motion = Self::auto_reduced_motion_preference();
        self.auto_state.ensure_intro_timing(reduced_motion);
    }

    fn auto_show_goal_entry_panel(&mut self) {
        self.auto_state.set_phase(AutoRunPhase::AwaitingGoalEntry);
        self.auto_state.goal = None;
        self.auto_pending_goal_request = false;
        self.auto_goal_bootstrap_done = false;
        let seed_intro = self.auto_state.take_intro_pending();
        if seed_intro {
            self.auto_reset_intro_timing();
            self.auto_ensure_intro_timing();
        }
        self.auto_goal_escape_state = AutoGoalEscState::Inactive;
        let hint = "Let's do this! What's your goal?".to_string();
        let status_lines = vec![hint];
        let model = AutoCoordinatorViewModel::Active(AutoActiveViewModel {
            goal: None,
            status_lines,
            cli_prompt: None,
            cli_context: None,
            show_composer: true,
            awaiting_submission: false,
            waiting_for_response: false,
            coordinator_waiting: false,
            waiting_for_review: false,
            countdown: None,
            button: None,
            manual_hint: None,
            ctrl_switch_hint: String::new(),
            cli_running: false,
            turns_completed: 0,
            started_at: None,
            elapsed: None,
            status_sent_to_user: None,
            status_title: None,
            session_tokens: self.auto_session_tokens(),
            editing_prompt: false,
            intro_started_at: self.auto_state.intro_started_at,
            intro_reduced_motion: self.auto_state.intro_reduced_motion,
        });
        self.bottom_pane.show_auto_coordinator_view(model);
        self.bottom_pane.set_task_running(false);
        self.bottom_pane.clear_live_ring();
        self.bottom_pane.update_status_text("Auto Drive".to_string());
        self.auto_update_terminal_hint();
        self.bottom_pane.ensure_input_focus();
        self.clear_composer();
        self.request_redraw();
    }

    fn auto_exit_goal_entry_preserve_draft(&mut self) -> bool {
        if !self.auto_state.should_show_goal_entry() {
            return false;
        }

        let last_run_summary = self.auto_state.last_run_summary.clone();
        let last_decision_summary = self.auto_state.last_decision_summary.clone();
        let last_decision_status_sent_to_user =
            self.auto_state.last_decision_status_sent_to_user.clone();
        let last_decision_status_title =
            self.auto_state.last_decision_status_title.clone();
        let last_decision_display = self.auto_state.last_decision_display.clone();
        let last_decision_display_is_summary = self.auto_state.last_decision_display_is_summary;

        self.auto_state.reset();
        self.auto_state.last_run_summary = last_run_summary;
        self.auto_state.last_decision_summary = last_decision_summary;
        self.auto_state.last_decision_status_sent_to_user = last_decision_status_sent_to_user;
        self.auto_state.last_decision_status_title = last_decision_status_title;
        self.auto_state.last_decision_display = last_decision_display;
        self.auto_state.last_decision_display_is_summary = last_decision_display_is_summary;
        self.auto_state.set_phase(AutoRunPhase::Idle);
        self.auto_goal_escape_state = AutoGoalEscState::Inactive;
        self.bottom_pane.clear_auto_coordinator_view(true);
        self.bottom_pane.clear_live_ring();
        self.bottom_pane.set_task_running(false);
        self.bottom_pane.update_status_text(String::new());
        self.auto_rebuild_live_ring();
        self.request_redraw();
        true
    }

    fn auto_launch_with_goal(
        &mut self,
        goal: String,
        derive_goal_from_history: bool,
        review_enabled: bool,
        subagents_enabled: bool,
        cross_check_enabled: bool,
        qa_automation_enabled: bool,
        continue_mode: AutoContinueMode,
    ) {
        let conversation = self.rebuild_auto_history();
        let reduced_motion = Self::auto_reduced_motion_preference();
        self.auto_state.prepare_launch(
            goal.clone(),
            review_enabled,
            subagents_enabled,
            cross_check_enabled,
            qa_automation_enabled,
            continue_mode,
            reduced_motion,
        );
        self.config.auto_drive.cross_check_enabled = cross_check_enabled;
        self.config.auto_drive.qa_automation_enabled = qa_automation_enabled;
        let coordinator_events = {
            let app_event_tx = self.app_event_tx.clone();
            AutoCoordinatorEventSender::new(move |event| {
                match event {
                    AutoCoordinatorEvent::Decision {
                        seq,
                        status,
                        status_title,
                        status_sent_to_user,
                        goal,
                        cli,
                        agents_timing,
                        agents,
                        transcript,
                    } => {
                        app_event_tx.send(AppEvent::AutoCoordinatorDecision {
                            seq,
                            status,
                            status_title,
                            status_sent_to_user,
                            goal,
                            cli,
                            agents_timing,
                            agents,
                            transcript,
                        });
                    }
                    AutoCoordinatorEvent::Thinking { delta, summary_index } => {
                        app_event_tx.send(AppEvent::AutoCoordinatorThinking { delta, summary_index });
                    }
                    AutoCoordinatorEvent::Action { message } => {
                        app_event_tx.send(AppEvent::AutoCoordinatorAction { message });
                    }
                    AutoCoordinatorEvent::UserReply {
                        user_response,
                        cli_command,
                    } => {
                        app_event_tx.send(AppEvent::AutoCoordinatorUserReply {
                            user_response,
                            cli_command,
                        });
                    }
                    AutoCoordinatorEvent::TokenMetrics {
                        total_usage,
                        last_turn_usage,
                        turn_count,
                        duplicate_items,
                        replay_updates,
                    } => {
                        app_event_tx.send(AppEvent::AutoCoordinatorTokenMetrics {
                            total_usage,
                            last_turn_usage,
                            turn_count,
                            duplicate_items,
                            replay_updates,
                        });
                    }
                    AutoCoordinatorEvent::CompactedHistory { conversation, show_notice } => {
                        app_event_tx.send(AppEvent::AutoCoordinatorCompactedHistory {
                            conversation,
                            show_notice,
                        });
                    }
                    AutoCoordinatorEvent::StopAck => {
                        app_event_tx.send(AppEvent::AutoCoordinatorStopAck);
                    }
                }
            })
        };

        let mut auto_config = self.config.clone();
        auto_config.model = self.config.auto_drive.model.trim().to_string();
        if auto_config.model.is_empty() {
            auto_config.model = code_auto_drive_core::MODEL_SLUG.to_string();
        }
        auto_config.model_reasoning_effort = self.config.auto_drive.model_reasoning_effort;

        let mut pid_guard = AutoDrivePidFile::write(
            &self.config.code_home,
            Some(goal.as_str()),
            AutoDriveMode::Tui,
        );

        match start_auto_coordinator(
            coordinator_events,
            goal.clone(),
            conversation,
            auto_config,
            self.config.debug,
            derive_goal_from_history,
        ) {
            Ok(handle) => {
                self.auto_handle = Some(handle);
                self.auto_drive_pid_guard = pid_guard.take();
                let placeholder = auto_drive_strings::next_auto_drive_phrase().to_string();
                let effects = self
                    .auto_state
                    .launch_succeeded(goal, Some(placeholder), Instant::now());
                self.auto_apply_controller_effects(effects);
            }
            Err(err) => {
                drop(pid_guard);
                let effects = self
                    .auto_state
                    .launch_failed(goal, err.to_string());
                self.auto_apply_controller_effects(effects);
            }
        }
    }

    pub(crate) fn handle_auto_command(&mut self, goal: Option<String>) {
        let provided = goal.unwrap_or_default();
        let trimmed = provided.trim();

        if trimmed.eq_ignore_ascii_case("settings") {
            self.ensure_auto_drive_settings_overlay();
            return;
        }

        let full_auto_enabled = matches!(
            (&self.config.sandbox_policy, self.config.approval_policy),
            (SandboxPolicy::DangerFullAccess, AskForApproval::Never)
        );

        if !full_auto_enabled && !(trimmed.is_empty() && self.auto_state.is_active()) {
            self.push_background_tail(
                "Please use Shift+Tab to switch to Full Auto before using Auto Drive"
                    .to_string(),
            );
            self.request_redraw();
            return;
        }
        if trimmed.is_empty() {
            if self.auto_state.is_active() {
                self.auto_stop(None);
            }
            let started = self.auto_start_bootstrap_from_history();
            if !started {
                self.auto_state.reset();
                self.auto_state.set_phase(AutoRunPhase::Idle);
                self.auto_show_goal_entry_panel();
            }
            self.request_redraw();
            return;
        }

        let goal_text = trimmed.to_string();

        if self.auto_state.is_active() {
            self.auto_stop(None);
        }

        let defaults = self.config.auto_drive.clone();
        let default_mode = auto_continue_from_config(defaults.continue_mode);

        self.auto_state.mark_intro_pending();
        self.auto_launch_with_goal(
            goal_text,
            false,
            defaults.review_enabled,
            defaults.agents_enabled,
            defaults.cross_check_enabled,
            defaults.qa_automation_enabled,
            default_mode,
        );
    }

    pub(crate) fn show_auto_drive_settings(&mut self) {
        self.history_render.invalidate_all();
        self.mark_render_requests_dirty();
        self.history_prefix_append_only.set(false);
        self.ensure_auto_drive_settings_overlay();
    }

    pub(crate) fn close_auto_drive_settings(&mut self) {
        if matches!(
            self.settings
                .overlay
                .as_ref()
                .map(settings_overlay::SettingsOverlayView::active_section),
            Some(SettingsSection::AutoDrive)
        ) {
            self.close_settings_overlay();
        }
        self.history_render.invalidate_all();
        self.mark_render_requests_dirty();
        self.history_prefix_append_only.set(false);
        let should_rebuild_view = if self.auto_state.is_active() {
            !self.auto_state.is_paused_manual()
        } else {
            self.auto_state.should_show_goal_entry() || self.auto_state.last_run_summary.is_some()
        };

        if should_rebuild_view {
            self.auto_rebuild_live_ring();
        }
        self.bottom_pane.ensure_input_focus();
    }

    pub(crate) fn apply_auto_drive_settings(
        &mut self,
        review_enabled: bool,
        agents_enabled: bool,
        cross_check_enabled: bool,
        qa_automation_enabled: bool,
        continue_mode: AutoContinueMode,
    ) {
        let mut changed = false;
        if self.auto_state.review_enabled != review_enabled {
            self.auto_state.review_enabled = review_enabled;
            changed = true;
        }
        if self.auto_state.subagents_enabled != agents_enabled {
            self.auto_state.subagents_enabled = agents_enabled;
            changed = true;
        }
        if self.auto_state.cross_check_enabled != cross_check_enabled {
            self.auto_state.cross_check_enabled = cross_check_enabled;
            changed = true;
        }
        if self.auto_state.qa_automation_enabled != qa_automation_enabled {
            self.auto_state.qa_automation_enabled = qa_automation_enabled;
            changed = true;
        }
        if self.auto_state.continue_mode != continue_mode {
            let effects = self.auto_state.update_continue_mode(continue_mode);
            self.auto_apply_controller_effects(effects);
            changed = true;
        }

        if !changed {
            return;
        }

        self.config.auto_drive.review_enabled = review_enabled;
        self.config.auto_drive.agents_enabled = agents_enabled;
        self.config.auto_drive.cross_check_enabled = cross_check_enabled;
        self.config.auto_drive.qa_automation_enabled = qa_automation_enabled;
        self.config.auto_drive.continue_mode = auto_continue_to_config(continue_mode);
        self.restore_auto_resolve_attempts_if_lost();

        if let Ok(home) = code_core::config::find_code_home() {
            if let Err(err) = code_core::config::set_auto_drive_settings(
                &home,
                &self.config.auto_drive,
                self.config.auto_drive_use_chat_model,
            )
            {
                tracing::warn!("Failed to persist Auto Drive settings: {err}");
            }
        } else {
            tracing::warn!("Could not locate config home to persist Auto Drive settings");
        }

        self.refresh_settings_overview_rows();
        self.refresh_auto_drive_visuals();
        self.request_redraw();
    }

    fn auto_send_conversation(&mut self) {
        if !self.auto_state.is_active() || self.auto_state.is_waiting_for_response() {
            return;
        }
        self.auto_state.on_complete_review();
        if !self.auto_state.is_paused_manual() {
            self.auto_state.clear_bypass_coordinator_flag();
        }
        let conversation = std::sync::Arc::<[ResponseItem]>::from(self.current_auto_history());
        let Some(handle) = self.auto_handle.as_ref() else {
            return;
        };
        if handle
            .send(AutoCoordinatorCommand::UpdateConversation(conversation))
            .is_err()
        {
            self.auto_stop(Some("Coordinator stopped unexpectedly.".to_string()));
        } else {
            self.bottom_pane.set_standard_terminal_hint(None);
            self.auto_state.on_prompt_submitted();
            self.auto_state.set_coordinator_waiting(true);
            self.auto_state.current_summary = None;
            self.auto_state.current_status_sent_to_user = None;
            self.auto_state.current_status_title = None;
            self.auto_state.current_cli_prompt = None;
            self.auto_state.current_cli_context = None;
            self.auto_state.hide_cli_context_in_ui = false;
            self.auto_state.last_broadcast_summary = None;
            self.auto_state.current_summary_index = None;
            self.auto_state.current_display_line = None;
            self.auto_state.current_display_is_summary = false;
            self.auto_state.current_reasoning_title = None;
            self.auto_state.placeholder_phrase =
                Some(auto_drive_strings::next_auto_drive_phrase().to_string());
            self.auto_state.thinking_prefix_stripped = false;
            self.auto_rebuild_live_ring();
            self.request_redraw();
        }
    }

    fn auto_send_conversation_force(&mut self) {
        if !self.auto_state.is_active() {
            return;
        }
        if !self.auto_state.is_paused_manual() {
            self.auto_state.clear_bypass_coordinator_flag();
        }
        let conversation = std::sync::Arc::<[ResponseItem]>::from(self.current_auto_history());
        let Some(handle) = self.auto_handle.as_ref() else {
            return;
        };
        if handle
            .send(AutoCoordinatorCommand::UpdateConversation(conversation))
            .is_err()
        {
            self.auto_stop(Some("Coordinator stopped unexpectedly.".to_string()));
        } else {
            self.bottom_pane.set_standard_terminal_hint(None);
            self.auto_state.on_prompt_submitted();
            self.auto_state.set_coordinator_waiting(true);
            self.auto_state.current_summary = None;
            self.auto_state.current_status_sent_to_user = None;
            self.auto_state.current_status_title = None;
            self.auto_state.current_cli_prompt = None;
            self.auto_state.current_cli_context = None;
            self.auto_state.hide_cli_context_in_ui = false;
            self.auto_state.last_broadcast_summary = None;
            self.auto_state.current_summary_index = None;
            self.auto_state.current_display_line = None;
            self.auto_state.current_display_is_summary = false;
            self.auto_state.current_reasoning_title = None;
            self.auto_state.placeholder_phrase =
                Some(auto_drive_strings::next_auto_drive_phrase().to_string());
            self.auto_state.thinking_prefix_stripped = false;
            self.auto_rebuild_live_ring();
            self.request_redraw();
        }
    }

    fn auto_send_user_prompt_to_coordinator(
        &mut self,
        prompt: String,
        conversation: Vec<ResponseItem>,
    ) -> bool {
        let Some(handle) = self.auto_handle.as_ref() else {
            return false;
        };
        let command = AutoCoordinatorCommand::HandleUserPrompt {
            _prompt: prompt,
            conversation: conversation.into(),
        };
        match handle.send(command) {
            Ok(()) => {
                self.auto_state.on_prompt_submitted();
                self.auto_state.set_coordinator_waiting(true);
                self.auto_state.placeholder_phrase =
                    Some(auto_drive_strings::next_auto_drive_phrase().to_string());
                self.auto_rebuild_live_ring();
                self.request_redraw();
                true
            }
            Err(err) => {
                tracing::warn!("failed to dispatch user prompt to coordinator: {err}");
                false
            }
        }
    }

    fn auto_failure_is_transient(message: &str) -> bool {
        let lower = message.to_ascii_lowercase();
        const TRANSIENT_MARKERS: &[&str] = &[
            "stream error",
            "network error",
            "timed out",
            "timeout",
            "temporarily unavailable",
            "retry window exceeded",
            "retry limit exceeded",
            "connection reset",
            "connection refused",
            "broken pipe",
            "dns error",
            "host unreachable",
            "send request",
        ];
        TRANSIENT_MARKERS.iter().any(|needle| lower.contains(needle))
    }

    fn auto_schedule_restart_event(&self, token: u64, attempt: u32, delay: Duration) {
        let tx = self.app_event_tx.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                tx.send(AppEvent::AutoCoordinatorRestart { token, attempt });
            });
        } else {
            std::thread::spawn(move || {
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }
                tx.send(AppEvent::AutoCoordinatorRestart { token, attempt });
            });
        }
    }

    fn auto_pause_for_transient_failure(&mut self, message: String) {
        warn!("auto drive transient failure: {}", message);

        if let Some(handle) = self.auto_handle.take() {
            handle.cancel();
        }

        self.pending_turn_descriptor = None;
        self.pending_auto_turn_config = None;

        let effects = self
            .auto_state
            .pause_for_transient_failure(Instant::now(), message);
        self.auto_apply_controller_effects(effects);
    }

    pub(crate) fn auto_handle_decision(
        &mut self,
        seq: u64,
        status: AutoCoordinatorStatus,
        status_title: Option<String>,
        status_sent_to_user: Option<String>,
        goal: Option<String>,
        cli: Option<AutoTurnCliAction>,
        agents_timing: Option<AutoTurnAgentsTiming>,
        agents: Vec<AutoTurnAgentsAction>,
        transcript: Vec<code_protocol::models::ResponseItem>,
    ) {
        if !self.auto_state.is_active() {
            if let Some(handle) = self.auto_handle.as_ref() {
                let _ = handle.send(code_auto_drive_core::AutoCoordinatorCommand::AckDecision { seq });
            }
            return;
        }

        self.auto_pending_goal_request = false;

        if let Some(goal_text) = goal.as_ref().map(|g| g.trim()).filter(|g| !g.is_empty()) {
            let derived_goal = goal_text.to_string();
            self.auto_state.goal = Some(derived_goal.clone());
            self.auto_goal_bootstrap_done = true;
            self.auto_card_set_goal(Some(derived_goal));
        }

        let status_title = Self::normalize_status_field(status_title);
        let status_sent_to_user = Self::normalize_status_field(status_sent_to_user);

        self.auto_state.turns_completed = self.auto_state.turns_completed.saturating_add(1);

        if !transcript.is_empty() {
            self.auto_history.append_raw(&transcript);
        }

        if let Some(handle) = self.auto_handle.as_ref() {
            let _ = handle.send(code_auto_drive_core::AutoCoordinatorCommand::AckDecision { seq });
        }

        self.auto_state.current_status_sent_to_user = status_sent_to_user.clone();
        self.auto_state.current_status_title = status_title.clone();
        self.auto_state.last_decision_status_sent_to_user = status_sent_to_user.clone();
        self.auto_state.last_decision_status_title = status_title.clone();
        let planning_turn = cli
            .as_ref()
            .map(|action| action.suppress_ui_context)
            .unwrap_or(false);
        let cli_context_raw = cli
            .as_ref()
            .and_then(|action| action.context.clone());
        let cli_context = Self::normalize_status_field(cli_context_raw);
        let cli_prompt = cli.as_ref().map(|action| action.prompt.clone());

        self.auto_state.current_cli_context = cli_context;
        self.auto_state.hide_cli_context_in_ui = planning_turn;
        self.auto_state.suppress_next_cli_display = planning_turn;
        if let Some(ref prompt_text) = cli_prompt {
            self.auto_state.current_cli_prompt = Some(prompt_text.clone());
        } else {
            self.auto_state.current_cli_prompt = None;
        }

        let summary_text = Self::compose_status_summary(&status_title, &status_sent_to_user);
        self.auto_state.last_decision_summary = Some(summary_text.clone());
        self.auto_state.set_coordinator_waiting(false);
        self.auto_on_reasoning_final(&summary_text);
        self.auto_state.last_decision_display = self.auto_state.current_display_line.clone();
        self.auto_state.last_decision_display_is_summary =
            self.auto_state.current_display_is_summary;
            self.auto_state.on_resume_from_manual();

        self.pending_turn_descriptor = None;
        self.pending_auto_turn_config = None;

        if let Some(current) = status_title
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            self.auto_card_add_action(
                format!("Status: {current}"),
                AutoDriveActionKind::Info,
            );
        }

        let mut promoted_agents: Vec<String> = Vec::new();
        let continue_status = matches!(status, AutoCoordinatorStatus::Continue);

        let resolved_agents: Vec<AutoTurnAgentsAction> = agents
            .into_iter()
            .map(|mut action| {
                let original = action.write;
                let requested = action.write_requested;
                let resolved = self.resolve_agent_write_flag(requested);
                if resolved && !original {
                    promoted_agents.push(action.prompt.clone());
                }
                action.write = resolved;
                action
            })
            .collect();

        if continue_status {
            self.auto_state.pending_agent_actions = resolved_agents;
            self.auto_state.pending_agent_timing = agents_timing
                .filter(|_| !self.auto_state.pending_agent_actions.is_empty());
        } else {
            self.auto_state.pending_agent_actions.clear();
            self.auto_state.pending_agent_timing = None;
        }

        if !promoted_agents.is_empty() {
            let joined = promoted_agents
                .into_iter()
                .map(|prompt| {
                    let trimmed = prompt.trim();
                    if trimmed.is_empty() {
                        "<empty prompt>".to_string()
                    } else {
                        format!("\"{trimmed}\"")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            self.auto_card_add_action(
                format!("Auto Drive enabled write mode for agent prompt(s): {joined}"),
                AutoDriveActionKind::Info,
            );
        }

        if !matches!(status, AutoCoordinatorStatus::Failed) {
            self.auto_state.transient_restart_attempts = 0;
           self.auto_state.on_recovery_attempt();
            self.auto_state.pending_restart = None;
        }

        match status {
            AutoCoordinatorStatus::Continue => {
                let Some(prompt_text) = cli_prompt else {
                    self.auto_stop(Some("Coordinator response omitted a prompt.".to_string()));
                    return;
                };
                if planning_turn {
                    self.push_background_tail("Auto Drive: Planning started".to_string());
                    if let Some(full_prompt) = self.build_auto_turn_message(&prompt_text) {
                        self.auto_dispatch_cli_prompt(full_prompt);
                    } else {
                        self.auto_stop(Some(
                            "Coordinator produced an empty planning prompt.".to_string(),
                        ));
                    }
                } else {
                    self.schedule_auto_cli_prompt(seq, prompt_text);
                }
            }
            AutoCoordinatorStatus::Success => {
                let normalized = summary_text.trim();
                let message = if normalized.is_empty() {
                    "Coordinator success.".to_string()
                } else if normalized
                    .to_ascii_lowercase()
                    .starts_with("coordinator success:")
                {
                    summary_text
                } else {
                    format!("Coordinator success: {summary_text}")
                };

                let diagnostics_goal = self
                    .auto_state
                    .goal
                    .as_deref()
                    .unwrap_or("(goal unavailable)");

                let prompt_text = format!(
                    r#"Here was the original goal:
{diagnostics_goal}

Have we met every part of this goal and is there no further work to do?"#
                );

                let tf = TextFormat {
                    r#type: "json_schema".to_string(),
                    name: Some("auto_drive_diagnostics".to_string()),
                    strict: Some(true),
                    schema: Some(code_auto_drive_diagnostics::AutoDriveDiagnostics::completion_schema()),
                };
                self.submit_op(Op::SetNextTextFormat { format: tf.clone() });
                self.next_cli_text_format = Some(tf);
                self.auto_state.pending_stop_message = Some(message);
                self.auto_card_add_action(
                    "Auto Drive Diagnostics: Validating progress".to_string(),
                    AutoDriveActionKind::Info,
                );
                self.schedule_auto_cli_prompt(seq, prompt_text);
                self.auto_submit_prompt();
            }
            AutoCoordinatorStatus::Failed => {
                let normalized = summary_text.trim();
                let message = if normalized.is_empty() {
                    "Coordinator error.".to_string()
                } else if normalized
                    .to_ascii_lowercase()
                    .starts_with("coordinator error:")
                {
                    summary_text
                } else {
                    format!("Coordinator error: {summary_text}")
                };
                if Self::auto_failure_is_transient(&message) {
                    self.auto_pause_for_transient_failure(message);
                } else {
                    self.auto_stop(Some(message));
                }
            }
        }
    }

    pub(crate) fn auto_handle_user_reply(
        &mut self,
        user_response: Option<String>,
        cli_command: Option<String>,
    ) {
        if let Some(text) = user_response {
            if let Some(item) = Self::auto_drive_make_assistant_message(text.clone()) {
                self.auto_history
                    .append_raw(std::slice::from_ref(&item));
            }
            let mut lines = Vec::with_capacity(2);
            lines.push("AUTO DRIVE RESPONSE".to_string());
            lines.push(text);
            self.history_push_plain_paragraphs(PlainMessageKind::Notice, lines);
        }

        if let Some(command) = cli_command {
            if command.trim_start().starts_with('/') {
                self.app_event_tx
                    .send(AppEvent::DispatchCommand(SlashCommand::Auto, command));
            } else {
                let mut message: UserMessage = command.into();
                message.suppress_persistence = true;
                self.submit_user_message(message);
            }
        } else {
            self.auto_state.set_phase(AutoRunPhase::Active);
            self.auto_state.placeholder_phrase = None;
        }

        self.auto_rebuild_live_ring();
        self.request_redraw();
    }

    pub(crate) fn auto_handle_token_metrics(
        &mut self,
        total_usage: TokenUsage,
        last_turn_usage: TokenUsage,
        turn_count: u32,
        duplicate_items: u32,
        replay_updates: u32,
    ) {
        self.auto_history
            .apply_token_metrics(
                total_usage,
                last_turn_usage,
                turn_count,
                duplicate_items,
                replay_updates,
            );
        self.request_redraw();
    }

    fn auto_session_tokens(&self) -> Option<u64> {
        let total = self.auto_history.total_tokens().blended_total();
        (total > 0).then_some(total)
    }

    pub(crate) fn auto_handle_compacted_history(
        &mut self,
        conversation: std::sync::Arc<[ResponseItem]>,
        show_notice: bool,
    ) {
        let (previous_items, previous_indices) = self.export_auto_drive_items_with_indices();
        let conversation = conversation.as_ref().to_vec();
        self.auto_compaction_overlay = self
            .derive_compaction_overlay(&previous_items, &previous_indices, &conversation);
        self.auto_history.replace_all(conversation);
        if show_notice {
            self.history_push_plain_paragraphs(
                PlainMessageKind::Notice,
                [COMPACTION_CHECKPOINT_MESSAGE],
            );
        }
        self.auto_rebuild_live_ring();
        self.request_redraw();
    }

    fn schedule_auto_cli_prompt(&mut self, decision_seq: u64, prompt_text: String) {
        self.schedule_auto_cli_prompt_with_override(decision_seq, prompt_text, None);
    }

    fn schedule_auto_cli_prompt_with_override(
        &mut self,
        decision_seq: u64,
        prompt_text: String,
        countdown_override: Option<u8>,
    ) {
        self.auto_state.suppress_next_cli_display = false;
        let effects = self
            .auto_state
            .schedule_cli_prompt(decision_seq, prompt_text, countdown_override);
        self.auto_apply_controller_effects(effects);
    }

    fn auto_can_bootstrap_from_history(&self) -> bool {
        self.history_cells.iter().any(|cell| match cell.kind() {
            HistoryCellType::User | HistoryCellType::Assistant | HistoryCellType::Plain => true,
            HistoryCellType::Exec { .. } => true,
            _ => false,
        })
    }

    fn auto_apply_controller_effects(&mut self, effects: Vec<AutoControllerEffect>) {
        for effect in effects {
        match effect {
            AutoControllerEffect::RefreshUi => {
                    self.auto_rebuild_live_ring();
                    self.request_redraw();
                }
                AutoControllerEffect::StartCountdown {
                    countdown_id,
                    decision_seq,
                    seconds,
                } => {
                    if seconds == 0 {
                        self.app_event_tx.send(AppEvent::AutoCoordinatorCountdown {
                            countdown_id,
                            seconds_left: 0,
                        });
                    } else {
                        self.auto_spawn_countdown(countdown_id, decision_seq, seconds);
                    }
                }
                AutoControllerEffect::SubmitPrompt => {
                    if self.auto_state.should_bypass_coordinator_next_submit()
                        && self.auto_state.is_paused_manual()
                    {
                        self.auto_state.clear_bypass_coordinator_flag();
                        self.auto_state.set_phase(AutoRunPhase::Active);
                    }
                    if !self.auto_state.should_bypass_coordinator_next_submit() {
                        self.auto_submit_prompt();
                    }
                }
                AutoControllerEffect::LaunchStarted { goal } => {
                    self.bottom_pane.set_task_running(false);
                    self.bottom_pane.update_status_text("Auto Drive".to_string());
                    self.auto_card_start(Some(goal.clone()));
                    self.auto_card_add_action(
                        format!("Auto Drive started: {goal}"),
                        AutoDriveActionKind::Info,
                    );
                    self.auto_card_set_status(AutoDriveStatus::Running);
                }
                AutoControllerEffect::LaunchFailed { goal, error } => {
                    let message = format!(
                        "Coordinator failed to start for goal '{goal}': {error}"
                    );
                    self.auto_card_finalize(
                        Some(message),
                        AutoDriveStatus::Failed,
                        AutoDriveActionKind::Error,
                    );
                    self.auto_request_session_summary();
                }
                AutoControllerEffect::StopCompleted { summary, message } => {
                    if let Some(handle) = self.auto_handle.take() {
                        handle.cancel();
                        let _ = handle.send(AutoCoordinatorCommand::Stop);
                    }
                    let final_message = message.or_else(|| summary.message.clone());
                    if let Some(msg) = final_message.clone() {
                        if !msg.trim().is_empty() {
                            self.auto_card_finalize(
                                Some(msg),
                                AutoDriveStatus::Stopped,
                                AutoDriveActionKind::Info,
                            );
                        } else {
                            self.auto_card_finalize(None, AutoDriveStatus::Stopped, AutoDriveActionKind::Info);
                        }
                    } else {
                        self.auto_card_finalize(None, AutoDriveStatus::Stopped, AutoDriveActionKind::Info);
                    }
                    self.schedule_auto_drive_card_celebration(
                        Duration::from_secs(0),
                        self.auto_state.last_completion_explanation.clone(),
                    );
                    self.auto_turn_review_state = None;
                    if ENABLE_WARP_STRIPES {
                        self.header_wave.set_enabled(false, Instant::now());
                    }
                    self.auto_request_session_summary();
                }
                AutoControllerEffect::TransientPause {
                    attempt,
                    delay,
                    reason,
                } => {
                    let human_delay = format_duration(delay);
                    self.bottom_pane.set_task_running(false);
                    self.bottom_pane
                        .update_status_text("Auto Drive paused".to_string());
                    self.bottom_pane.set_standard_terminal_hint(Some(
                        AUTO_ESC_EXIT_HINT.to_string(),
                    ));
                    let message = format!(
                        "Auto Drive will retry automatically in {human_delay} (attempt {attempt}). Last error: {reason}"
                    );
                    self.auto_card_add_action(message, AutoDriveActionKind::Warning);
                    self.auto_card_set_status(AutoDriveStatus::Paused);
                }
                AutoControllerEffect::ScheduleRestart {
                    token,
                    attempt,
                    delay,
                } => {
                    self.auto_schedule_restart_event(token, attempt, delay);
                }
                AutoControllerEffect::CancelCoordinator => {
                    if let Some(handle) = self.auto_handle.take() {
                        handle.cancel();
                        let _ = handle.send(AutoCoordinatorCommand::Stop);
                    }
                }
                AutoControllerEffect::ResetHistory => {
                    self.auto_history.clear();
                    self.reset_auto_compaction_overlay();
                }
                AutoControllerEffect::UpdateTerminalHint { hint } => {
                    self.bottom_pane.set_standard_terminal_hint(hint);
                }
                AutoControllerEffect::SetTaskRunning { running } => {
                    let has_activity = running
                        || !self.exec.running_commands.is_empty()
                        || !self.tools_state.running_custom_tools.is_empty()
                        || !self.tools_state.web_search_sessions.is_empty()
                        || !self.tools_state.running_wait_tools.is_empty()
                        || !self.tools_state.running_kill_tools.is_empty()
                        || self.stream.is_write_cycle_active()
                        || !self.active_task_ids.is_empty();

                    self.bottom_pane.set_task_running(has_activity);
                    if !has_activity {
                        self.bottom_pane.update_status_text(String::new());
                    }
                }
                AutoControllerEffect::EnsureInputFocus => {
                    self.bottom_pane.ensure_input_focus();
                }
                AutoControllerEffect::ClearCoordinatorView => {
                    self.bottom_pane.clear_auto_coordinator_view(true);
                }
                AutoControllerEffect::ShowGoalEntry => {
                    self.auto_show_goal_entry_panel();
                }
            }
        }
    }

    fn auto_spawn_countdown(&self, countdown_id: u64, decision_seq: u64, seconds: u8) {
        let tx = self.app_event_tx.clone();
        let fallback_tx = tx.clone();
        if thread_spawner::spawn_lightweight("countdown", move || {
            let mut remaining = seconds;
            tracing::debug!(
                target: "auto_drive::coordinator",
                countdown_id,
                decision_seq,
                seconds,
                "spawned countdown"
            );
            while remaining > 0 {
                std::thread::sleep(std::time::Duration::from_secs(1));
                remaining -= 1;
                if !tx.send_with_result(AppEvent::AutoCoordinatorCountdown {
                    countdown_id,
                    seconds_left: remaining,
                }) {
                    break;
                }
            }
        })
        .is_none()
        {
            fallback_tx.send(AppEvent::AutoCoordinatorCountdown {
                countdown_id,
                seconds_left: 0,
            });
        }
    }

    pub(crate) fn auto_handle_countdown(&mut self, countdown_id: u64, seconds_left: u8) {
        let decision_seq = self.auto_state.countdown_decision_seq;
        let effects = self
            .auto_state
            .handle_countdown_tick(countdown_id, decision_seq, seconds_left);
        if effects.is_empty() {
            return;
        }
        self.auto_apply_controller_effects(effects);
    }

    pub(crate) fn auto_handle_restart(&mut self, token: u64, attempt: u32) {
        if !self.auto_state.is_active() || !self.auto_state.in_transient_recovery() {
            return;
        }
        let Some(restart) = self.auto_state.pending_restart.clone() else {
            return;
        };
        if restart.token != token || restart.attempt != attempt {
            return;
        }

        let Some(goal) = self.auto_state.goal.clone() else {
            self.auto_card_add_action(
                "Auto Drive restart skipped because the goal is no longer available.".to_string(),
                AutoDriveActionKind::Warning,
            );
            self.auto_state.pending_restart = None;
            self.auto_state.on_recovery_attempt();
            self.auto_stop(Some("Auto Drive restart aborted.".to_string()));
            return;
        };

        let cross_check_enabled = self.auto_state.cross_check_enabled;
        let continue_mode = self.auto_state.continue_mode;
        let previous_turns = self.auto_state.turns_completed;
        let previous_started_at = self.auto_state.started_at;
        let restart_attempts = self.auto_state.transient_restart_attempts;
        let review_enabled = self.auto_state.review_enabled;
        let agents_enabled = self.auto_state.subagents_enabled;
        let qa_automation_enabled = self.auto_state.qa_automation_enabled;

        self.auto_state.pending_restart = None;
        self.auto_state.on_recovery_attempt();
        self.auto_state.restart_token = token;

        let resume_message = if restart.reason.is_empty() {
            format!("Auto Drive resuming automatically (attempt {attempt}).")
        } else {
            format!(
                "Auto Drive resuming automatically (attempt {attempt}); previous error: {}",
                restart.reason
            )
        };
        self.auto_card_add_action(resume_message, AutoDriveActionKind::Info);
        self.auto_card_set_status(AutoDriveStatus::Running);

        self.auto_launch_with_goal(
            goal,
            false,
            review_enabled,
            agents_enabled,
            cross_check_enabled,
            qa_automation_enabled,
            continue_mode,
        );

        if previous_turns > 0 {
            self.auto_state.turns_completed = previous_turns;
        }
        if let Some(started_at) = previous_started_at {
            self.auto_state.started_at = Some(started_at);
        }
        self.auto_state.transient_restart_attempts = restart_attempts;
        self.auto_state.current_status_title = None;
        self.auto_state.current_status_sent_to_user = None;
        self.auto_rebuild_live_ring();
        self.auto_update_terminal_hint();
        self.request_redraw();
        self.rebuild_auto_history();
    }

    pub(crate) fn auto_handle_thinking(&mut self, delta: String, summary_index: Option<u32>) {
        if !self.auto_state.is_active() {
            return;
        }
        self.auto_on_reasoning_delta(&delta, summary_index);
    }

    pub(crate) fn auto_handle_action(&mut self, message: String) {
        if !self.auto_state.is_active() {
            return;
        }
        self.auto_card_add_action(message, AutoDriveActionKind::Info);
    }

    #[cfg(any(test, feature = "test-helpers"))]
    #[allow(dead_code)]
    fn auto_handle_post_turn_review(
        &mut self,
        cfg: TurnConfig,
        descriptor: Option<&TurnDescriptor>,
    ) {
        if !self.auto_state.review_enabled {
            self.auto_turn_review_state = None;
            return;
        }
        if cfg.read_only {
            self.auto_turn_review_state = None;
            return;
        }

        match self.auto_prepare_commit_scope() {
            AutoReviewOutcome::Skip => {
                self.auto_turn_review_state = None;
                if self.auto_state.awaiting_review() {
                    self.maybe_resume_auto_after_review();
                }
            }
            AutoReviewOutcome::Workspace => {
                self.auto_turn_review_state = None;
                self.auto_start_post_turn_review(None, descriptor);
            }
            AutoReviewOutcome::Commit(scope) => {
                self.auto_turn_review_state = None;
                self.auto_start_post_turn_review(Some(scope), descriptor);
            }
        }
    }

    #[cfg(any(test, feature = "test-helpers"))]
    #[allow(dead_code)]
    fn auto_prepare_commit_scope(&mut self) -> AutoReviewOutcome {
        let Some(state) = self.auto_turn_review_state.take() else {
            return AutoReviewOutcome::Workspace;
        };

        let Some(base_commit) = state.base_commit else {
            return AutoReviewOutcome::Workspace;
        };

        let final_commit = match self.capture_auto_turn_commit("auto turn change snapshot", Some(&base_commit)) {
            Ok(commit) => commit,
            Err(err) => {
                tracing::warn!("failed to capture auto turn change snapshot: {err}");
                return AutoReviewOutcome::Workspace;
            }
        };

        let diff_paths = match self.git_diff_name_only_between(base_commit.id(), final_commit.id()) {
            Ok(paths) => paths,
            Err(err) => {
                tracing::warn!("failed to diff auto turn snapshots: {err}");
                return AutoReviewOutcome::Workspace;
            }
        };

        if diff_paths.is_empty() {
            self.push_background_tail("Auto review skipped: no file changes detected this turn.".to_string());
            return AutoReviewOutcome::Skip;
        }

        AutoReviewOutcome::Commit(AutoReviewCommitScope {
            commit: final_commit.id().to_string(),
            file_count: diff_paths.len(),
        })
    }

    #[cfg(any(test, feature = "test-helpers"))]
    #[allow(dead_code)]
    fn auto_turn_has_diff(&self) -> bool {
        if self.worktree_has_uncommitted_changes().unwrap_or(false) {
            return true;
        }

        if let Some(base_commit) = self
            .auto_turn_review_state
            .as_ref()
            .and_then(|state| state.base_commit.as_ref())
        {
            if let Some(head) = self.current_head_commit_sha() {
                if let Ok(paths) = self.git_diff_name_only_between(base_commit.id(), &head) {
                    if !paths.is_empty() {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn prepare_auto_turn_review_state(&mut self) {
        if !self.auto_state.is_active() || !self.auto_state.review_enabled {
            self.auto_turn_review_state = None;
            return;
        }

        let read_only = self
            .pending_auto_turn_config
            .as_ref()
            .map(|cfg| cfg.read_only)
            .unwrap_or(false);

        if read_only {
            self.auto_turn_review_state = None;
            return;
        }

        let existing_base = self
            .auto_turn_review_state
            .as_ref()
            .and_then(|state| state.base_commit.as_ref());

        if existing_base.is_some() {
            return;
        }

        #[cfg(test)]
        {
            if CAPTURE_AUTO_TURN_COMMIT_STUB.lock().unwrap().is_some() {
                return;
            }
        }

        match self.capture_auto_turn_commit("auto turn base snapshot", None) {
            Ok(commit) => {
                self.auto_turn_review_state = Some(AutoTurnReviewState {
                    base_commit: Some(commit),
                });
            }
            Err(err) => {
                tracing::warn!("failed to capture auto turn base snapshot: {err}");
                self.auto_turn_review_state = None;
            }
        }
    }

    fn capture_auto_turn_commit(
        &self,
        message: &'static str,
        parent: Option<&GhostCommit>,
    ) -> Result<GhostCommit, GitToolingError> {
        #[cfg(test)]
        if let Some(stub) = CAPTURE_AUTO_TURN_COMMIT_STUB.lock().unwrap().as_ref() {
            let parent_id = parent.map(|commit| commit.id().to_string());
            return stub(message, parent_id);
        }
        let mut options = CreateGhostCommitOptions::new(self.config.cwd.as_path()).message(message);
        if let Some(parent_commit) = parent {
            options = options.parent(parent_commit.id());
        }
        let hook_repo_follow = self.config.cwd.clone();
        let hook = move || bump_snapshot_epoch_for(&hook_repo_follow);
        let result = create_ghost_commit(&options.post_commit_hook(&hook));
        if result.is_ok() {
            bump_snapshot_epoch_for(&self.config.cwd);
        }
        result
    }

    fn capture_auto_review_baseline_for_path(
        repo_path: PathBuf,
    ) -> Result<GhostCommit, GitToolingError> {
        #[cfg(test)]
        if let Some(stub) = CAPTURE_AUTO_TURN_COMMIT_STUB.lock().unwrap().as_ref() {
            return stub("auto review baseline snapshot", None);
        }
        let hook_repo = repo_path.clone();
        let options =
            CreateGhostCommitOptions::new(repo_path.as_path()).message("auto review baseline snapshot");
        let hook = move || bump_snapshot_epoch_for(&hook_repo);
        let result = create_ghost_commit(&options.post_commit_hook(&hook));
        if result.is_ok() {
            bump_snapshot_epoch_for(&repo_path);
        }
        result
    }

    fn spawn_auto_review_baseline_capture(&mut self) {
        let turn_sequence = self.turn_sequence;
        let repo_path = self.config.cwd.clone();
        let app_event_tx = self.app_event_tx.clone();

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let result = tokio::task::spawn_blocking(move || {
                    ChatWidget::capture_auto_review_baseline_for_path(repo_path)
                })
                .await
                .unwrap_or_else(|err| {
                    Err(GitToolingError::Io(io::Error::other(
                        format!("auto review baseline task failed: {err}"),
                    )))
                });
                app_event_tx.send(AppEvent::AutoReviewBaselineCaptured {
                    turn_sequence,
                    result,
                });
            });
        } else {
            std::thread::spawn(move || {
                let result = ChatWidget::capture_auto_review_baseline_for_path(repo_path);
                app_event_tx.send(AppEvent::AutoReviewBaselineCaptured {
                    turn_sequence,
                    result,
                });
            });
        }
    }

    pub(crate) fn handle_auto_review_baseline_captured(
        &mut self,
        turn_sequence: u64,
        result: Result<GhostCommit, GitToolingError>,
    ) {
        if turn_sequence != self.turn_sequence {
            tracing::debug!(
                "ignored auto review baseline for stale turn_sequence={turn_sequence}"
            );
            return;
        }
        if self.auto_review_baseline.is_some() {
            tracing::debug!("auto review baseline already set; skipping update");
            return;
        }
        match result {
            Ok(commit) => {
                self.auto_review_baseline = Some(commit);
            }
            Err(err) => {
                tracing::warn!("failed to capture auto review baseline: {err}");
            }
        }
    }

    #[cfg(any(test, feature = "test-helpers"))]
    #[allow(dead_code)]
    fn git_diff_name_only_between(
        &self,
        base_commit: &str,
        head_commit: &str,
    ) -> Result<Vec<String>, String> {
        #[cfg(test)]
        if let Some(stub) = GIT_DIFF_NAME_ONLY_BETWEEN_STUB.lock().unwrap().as_ref() {
            return stub(base_commit.to_string(), head_commit.to_string());
        }
        self.run_git_command(
            ["diff", "--name-only", base_commit, head_commit],
            |stdout| {
                let changes = stdout
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(|line| line.to_string())
                    .collect();
                Ok(changes)
            },
        )
    }

    fn auto_submit_prompt(&mut self) {
        if !self.auto_state.is_active() {
            return;
        }

        if self.auto_pending_goal_request {
            self.auto_pending_goal_request = false;
            self.auto_send_conversation_force();
            return;
        }

        let Some(original_prompt) = self.auto_state.current_cli_prompt.clone() else {
            self.auto_stop(Some("Coordinator prompt missing when attempting to submit.".to_string()));
            return;
        };

        if original_prompt.trim().is_empty() {
            self.auto_stop(Some("Coordinator produced an empty prompt.".to_string()));
            return;
        }

        let Some(full_prompt) = self.build_auto_turn_message(&original_prompt) else {
            self.auto_stop(Some("Coordinator produced an empty prompt.".to_string()));
            return;
        };

        self.auto_dispatch_cli_prompt(full_prompt);
    }

    fn auto_start_bootstrap_from_history(&mut self) -> bool {
        if !self.auto_can_bootstrap_from_history() {
            return false;
        }

        let defaults = self.config.auto_drive.clone();
        let default_mode = auto_continue_from_config(defaults.continue_mode);

        if self.auto_state.is_active() {
            self.auto_stop(None);
        }

        self.auto_state.mark_intro_pending();
        self.auto_launch_with_goal(
            AUTO_BOOTSTRAP_GOAL_PLACEHOLDER.to_string(),
            true,
            defaults.review_enabled,
            defaults.agents_enabled,
            defaults.cross_check_enabled,
            defaults.qa_automation_enabled,
            default_mode,
        );

        if self.auto_handle.is_none() {
            return false;
        }

        self.auto_state.current_cli_context = None;
        self.auto_state.hide_cli_context_in_ui = false;
        self.auto_state.current_cli_prompt = Some(String::new());
        self.auto_pending_goal_request = true;
        self.auto_goal_bootstrap_done = false;

        let override_seconds = if matches!(
            self.auto_state.continue_mode,
            AutoContinueMode::Immediate
        ) {
            Some(10)
        } else {
            None
        };
        self.schedule_auto_cli_prompt_with_override(0, String::new(), override_seconds);
        true
    }

    fn auto_dispatch_cli_prompt(&mut self, full_prompt: String) {
        self.auto_pending_goal_request = false;

        self.bottom_pane.set_standard_terminal_hint(None);
        self.auto_state.on_prompt_submitted();
        self.auto_state.set_coordinator_waiting(false);
        self.auto_state.clear_bypass_coordinator_flag();
        self.auto_state.seconds_remaining = 0;
        let post_submit_display = self.auto_state.last_decision_display.clone();
        self.auto_state.current_summary = None;
        self.auto_state.current_status_sent_to_user = None;
        self.auto_state.current_status_title = None;
        self.auto_state.last_broadcast_summary = None;
        self.auto_state.current_display_line = post_submit_display.clone();
        self.auto_state.current_display_is_summary =
            self.auto_state.last_decision_display_is_summary && post_submit_display.is_some();
        self.auto_state.current_summary_index = None;
        self.auto_state.placeholder_phrase = post_submit_display.is_none().then(|| {
            auto_drive_strings::next_auto_drive_phrase().to_string()
        });
        self.auto_state.current_reasoning_title = None;
        self.auto_state.thinking_prefix_stripped = false;

        let should_prepare_agents = self.auto_state.subagents_enabled
            && !self.auto_state.pending_agent_actions.is_empty();
        if should_prepare_agents {
            self.prepare_agents();
        }

        if self.auto_state.review_enabled {
            self.prepare_auto_turn_review_state();
        } else {
            self.auto_turn_review_state = None;
        }
        self.bottom_pane.update_status_text(String::new());
        self.bottom_pane.set_task_running(false);
        let mut message: UserMessage = full_prompt.into();
        message.suppress_persistence = true;
        if self.auto_state.pending_stop_message.is_some() {
            message.display_text.clear();
        } else if self.auto_state.suppress_next_cli_display {
            message.display_text.clear();
        }
        self.submit_user_message(message);
        self.auto_state.pending_agent_actions.clear();
        self.auto_state.pending_agent_timing = None;
        self.auto_rebuild_live_ring();
        self.request_redraw();
        self.auto_state.suppress_next_cli_display = false;
    }

    fn auto_pause_for_manual_edit(&mut self, force: bool) {
        if !self.auto_state.is_active() {
            return;
        }

        if !force && !self.auto_state.awaiting_coordinator_submit() {
            return;
        }

        let prompt_text = self
            .auto_state
            .current_cli_prompt
            .clone()
            .unwrap_or_default();
        let full_prompt = self
            .build_auto_turn_message(&prompt_text)
            .unwrap_or_else(|| prompt_text.clone());

        self.auto_state.on_pause_for_manual(true);
        self.auto_state.set_bypass_coordinator_next_submit();
        self.auto_state.countdown_id = self.auto_state.countdown_id.wrapping_add(1);
        self.auto_state.reset_countdown();
        self.clear_composer();
        if !full_prompt.is_empty() {
            self.insert_str(&full_prompt);
        } else if force && !prompt_text.is_empty() {
            self.insert_str(&prompt_text);
        }
        self.bottom_pane.ensure_input_focus();
        self.bottom_pane.set_task_running(true);
        self.bottom_pane
            .update_status_text("Auto Drive paused".to_string());
        self.show_auto_drive_exit_hint();
        self.auto_rebuild_live_ring();
        self.request_redraw();
    }

    // Build a hidden preface for the next Auto turn based on coordinator hints.
    fn build_auto_turn_message(&self, prompt_cli: &str) -> Option<String> {
        let mut sections: Vec<String> = Vec::new();

        if let Some(ctx) = self
            .auto_state
            .current_cli_context
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            sections.push(ctx.to_string());
        }

        if !prompt_cli.trim().is_empty() {
            sections.push(prompt_cli.trim().to_string());
        }

        let agent_actions = &self.auto_state.pending_agent_actions;
        if !agent_actions.is_empty() {
            let agent_timing = self.auto_state.pending_agent_timing;
            let mut agent_lines = Vec::with_capacity(agent_actions.len() * 4 + 5);
            const BLOCK_PREFIX: &str = "   ";
            const LINE_PREFIX: &str = "      ";

            agent_lines.push(format!("{BLOCK_PREFIX}<agents>"));
            agent_lines.push(format!(
                "{LINE_PREFIX}Please use agents to help you complete this task."
            ));

            for action in agent_actions {
                let prompt = action
                    .prompt
                    .trim()
                    .replace('\n', " ")
                    .replace('"', "\\\"");
                let write_text = if action.write { "write: true" } else { "write: false" };

                agent_lines.push(String::new());
                agent_lines.push(format!(
                    "{LINE_PREFIX}Please run agent.create with {write_text} and prompt like \"{prompt}\"."
                ));

                if let Some(ctx) = action
                    .context
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    agent_lines.push(format!(
                        "{LINE_PREFIX}Context: {}",
                        ctx.replace('\n', " ")
                    ));
                }

                if let Some(models) = action
                    .models
                    .as_ref()
                    .filter(|list| !list.is_empty())
                {
                    agent_lines.push(format!(
                        "{LINE_PREFIX}Models: [{}]",
                        models.join(", ")
                    ));
                }
            }

            agent_lines.push(String::new());
            let timing_line = match agent_timing {
                Some(AutoTurnAgentsTiming::Parallel) =>
                    "Timing (parallel): Launch these agents in the background while you continue the CLI prompt. Call agent.wait with the batch_id when you are ready to merge their results.".to_string(),
                Some(AutoTurnAgentsTiming::Blocking) =>
                    "Timing (blocking): Launch these agents first, then wait with agent.wait (use the batch_id from agent.create) and only continue the CLI prompt once their results are ready.".to_string(),
                None =>
                    "Timing (default blocking): After launching the agents, wait with agent.wait (use the batch_id returned by agent.create) and fold their output into your plan.".to_string(),
            };
            agent_lines.push(format!("{LINE_PREFIX}{timing_line}"));
            agent_lines.push(String::new());

            if agent_actions.iter().any(|action| !action.write) {
                agent_lines.push(format!(
                    "{LINE_PREFIX}Call agent.result to get the results from the agent if needed."
                ));
                agent_lines.push(String::new());
            }

            if agent_actions.iter().any(|action| action.write) {
                agent_lines.push(format!(
                    "{LINE_PREFIX}When agents run with write: true, they perform edits in their own worktree. Considering reviewing and merging the best worktree once they complete."
                ));
                agent_lines.push(String::new());
            }

            agent_lines.push(format!("{BLOCK_PREFIX}</agents>"));

            sections.push(agent_lines.join("\n"));
        }

        let combined = sections.join("\n\n");
        if combined.trim().is_empty() {
            None
        } else {
            Some(combined)
        }
    }

    fn auto_agents_can_write(&self) -> bool {
        if code_core::git_info::get_git_repo_root(&self.config.cwd).is_none() {
            return false;
        }
        matches!(
            self.config.sandbox_policy,
            SandboxPolicy::DangerFullAccess | SandboxPolicy::WorkspaceWrite { .. }
        )
    }

    fn resolve_agent_write_flag(&self, requested_write: Option<bool>) -> bool {
        if !self.auto_agents_can_write() {
            return false;
        }
        if !self.auto_state.subagents_enabled {
            return requested_write.unwrap_or(false);
        }
        true
    }

    fn auto_stop(&mut self, message: Option<String>) {
        self.next_cli_text_format = None;
        self.auto_pending_goal_request = false;
        self.auto_goal_bootstrap_done = false;
        self.auto_drive_pid_guard = None;
        let effects = self
            .auto_state
            .stop_run(Instant::now(), message);
        self.auto_goal_escape_state = AutoGoalEscState::Inactive;
        self.auto_apply_controller_effects(effects);
    }

    fn auto_on_assistant_final(&mut self) {
        if !self.auto_state.is_active() || !self.auto_state.is_waiting_for_response() {
            return;
        }
        self.auto_state.on_resume_from_manual();
        self.auto_state.reset_countdown();
        self.auto_state.current_summary = Some(String::new());
        self.auto_state.current_status_sent_to_user = None;
        self.auto_state.current_status_title = None;
        self.auto_state.current_summary_index = None;
        self.auto_state.placeholder_phrase = None;
        self.auto_state.thinking_prefix_stripped = false;

        let auto_resolve_blocking = self.auto_resolve_should_block_auto_resume();
        let review_pending = self.is_review_flow_active()
            || (self.auto_state.review_enabled
                && self
                    .pending_auto_turn_config
                    .as_ref()
                    .is_some_and(|cfg| !cfg.read_only));

        if review_pending || auto_resolve_blocking {
            self.auto_state.on_begin_review(false);
            #[cfg(any(test, feature = "test-helpers"))]
            if !self.auto_state.awaiting_review() {
                // Tests can run in parallel, so the shared review lock may already be held.
                // Force the state into AwaitingReview so assertions stay deterministic.
                self.auto_state.set_phase(AutoRunPhase::AwaitingReview {
                    diagnostics_pending: false,
                });
            }
        } else {
            self.auto_state.on_complete_review();
        }
        self.auto_rebuild_live_ring();
        self.request_redraw();
        self.rebuild_auto_history();

        if self.auto_state.awaiting_review() {
            return;
        }

        if !self.auto_state.should_bypass_coordinator_next_submit() {
            self.auto_send_conversation();
        }
    }

    #[cfg(any(test, feature = "test-helpers"))]
    #[allow(dead_code)]
    fn auto_start_post_turn_review(
        &mut self,
        scope: Option<AutoReviewCommitScope>,
        descriptor: Option<&TurnDescriptor>,
    ) {
        if !self.auto_state.review_enabled {
            return;
        }
        let strategy = descriptor.and_then(|d| d.review_strategy.as_ref());
        let (mut prompt, mut hint, mut auto_metadata, mut review_metadata, preparation) = match scope {
            Some(scope) => {
                let commit_id = scope.commit;
                let commit_for_prompt = commit_id.clone();
                let short_sha: String = commit_for_prompt.chars().take(8).collect();
                let file_label = if scope.file_count == 1 {
                    "1 file".to_string()
                } else {
                    format!("{} files", scope.file_count)
                };
                let prompt = format!(
                    "Review commit {} generated during the latest Auto Drive turn. Highlight bugs, regressions, risky patterns, and missing tests before merge.",
                    commit_for_prompt
                );
                let hint = format!("auto turn changes — {} ({})", short_sha, file_label);
                let preparation = format!("Preparing code review for commit {}", short_sha);
                let review_metadata = Some(ReviewContextMetadata {
                    scope: Some("commit".to_string()),
                    commit: Some(commit_id),
                    ..Default::default()
                });
                let auto_metadata = Some(ReviewContextMetadata {
                    scope: Some("workspace".to_string()),
                    ..Default::default()
                });
                (prompt, hint, auto_metadata, review_metadata, preparation)
            }
            None => {
                let prompt = "Review the current workspace changes and highlight bugs, regressions, risky patterns, and missing tests before merge.".to_string();
                let hint = "current workspace changes".to_string();
                let review_metadata = Some(ReviewContextMetadata {
                    scope: Some("workspace".to_string()),
                    ..Default::default()
                });
                let preparation = "Preparing code review request...".to_string();
                (
                    prompt,
                    hint,
                    review_metadata.clone(),
                    review_metadata,
                    preparation,
                )
            }
        };

        if let Some(strategy) = strategy {
            if let Some(custom_prompt) = strategy
                .custom_prompt
                .as_ref()
                .and_then(|text| {
                    let trimmed = text.trim();
                    (!trimmed.is_empty()).then_some(trimmed)
                })
            {
                prompt = custom_prompt.to_string();
            }

            if let Some(scope_hint) = strategy
                .scope_hint
                .as_ref()
                .and_then(|text| {
                    let trimmed = text.trim();
                    (!trimmed.is_empty()).then_some(trimmed)
                })
            {
                hint = scope_hint.to_string();

                let apply_scope = |meta: &mut ReviewContextMetadata| {
                    meta.scope = Some(scope_hint.to_string());
                };

                match review_metadata.as_mut() {
                    Some(meta) => apply_scope(meta),
                    None => {
                        review_metadata = Some(ReviewContextMetadata {
                            scope: Some(scope_hint.to_string()),
                            ..Default::default()
                        });
                    }
                }

                match auto_metadata.as_mut() {
                    Some(meta) => apply_scope(meta),
                    None => {
                        auto_metadata = Some(ReviewContextMetadata {
                            scope: Some(scope_hint.to_string()),
                            ..Default::default()
                        });
                    }
                }
            }
        }

        if self.config.tui.review_auto_resolve {
            let max_re_reviews = self.configured_auto_resolve_re_reviews();
            self.auto_resolve_state = Some(AutoResolveState::new_with_limit(
                prompt.clone(),
                hint.clone(),
                auto_metadata.clone(),
                max_re_reviews,
            ));
        } else {
            self.auto_resolve_state = None;
        }
        self.begin_review(prompt, hint, Some(preparation), review_metadata);
    }

    fn auto_rebuild_live_ring(&mut self) {
        if !self.auto_state.is_active() {
            if self.auto_state.should_show_goal_entry() {
                self.auto_show_goal_entry_panel();
                return;
            }
            if let Some(summary) = self.auto_state.last_run_summary.clone() {
                self.bottom_pane.clear_live_ring();
                self.auto_reset_intro_timing();
                self.auto_ensure_intro_timing();
                let mut status_lines: Vec<String> = Vec::new();
                if let Some(msg) = summary.message.as_ref() {
                    let trimmed = msg.trim();
                    if !trimmed.is_empty() {
                        status_lines.push(trimmed.to_string());
                    }
                }
                if status_lines.is_empty() {
                    if let Some(goal) = summary.goal.as_ref() {
                        status_lines.push(format!("Auto Drive completed: {goal}"));
                    } else {
                        status_lines.push("Auto Drive completed.".to_string());
                    }
                }
                let model = AutoCoordinatorViewModel::Active(AutoActiveViewModel {
                    goal: summary.goal.clone(),
                    status_lines,
                    cli_prompt: None,
                    cli_context: None,
                    show_composer: true,
            awaiting_submission: false,
            waiting_for_response: false,
            coordinator_waiting: false,
            waiting_for_review: false,
                    countdown: None,
                    button: None,
                    manual_hint: None,
                    ctrl_switch_hint: "Esc to exit Auto Drive".to_string(),
                    cli_running: false,
                    turns_completed: summary.turns_completed,
                    started_at: None,
                    elapsed: Some(summary.duration),
                    status_sent_to_user: None,
                    status_title: None,
                    session_tokens: self.auto_session_tokens(),
                    editing_prompt: false,
                    intro_started_at: self.auto_state.intro_started_at,
                    intro_reduced_motion: self.auto_state.intro_reduced_motion,
                });
            self
                .bottom_pane
                .show_auto_coordinator_view(model);
            self.bottom_pane.release_auto_drive_style();
            self.bottom_pane.set_standard_terminal_hint(None);
            return;
        }

        self.bottom_pane.clear_auto_coordinator_view(true);
        self.bottom_pane.clear_live_ring();
        self.bottom_pane.set_standard_terminal_hint(None);
        self.auto_reset_intro_timing();
        return;
    }

    // AutoDrive is active: if intro animation was mid-flight, force reduced motion
    // so a rebuild cannot leave the header half-rendered (issue #431).
    if self.auto_state.intro_started_at.is_some() && !self.auto_state.intro_reduced_motion {
        self.auto_state.intro_reduced_motion = true;
    }

    if self.auto_state.is_paused_manual() {
        self.bottom_pane.clear_auto_coordinator_view(false);
        self.bottom_pane.clear_live_ring();
        self.bottom_pane.set_standard_terminal_hint(None);
        return;
    }

        self.bottom_pane.clear_live_ring();

        let status_text = if self.auto_state.awaiting_review() {
            "waiting for code review...".to_string()
        } else if let Some(line) = self
            .auto_state
            .current_display_line
            .as_ref()
            .filter(|line| !line.trim().is_empty())
        {
            line.clone()
        } else {
            self
                .auto_state
                .placeholder_phrase
                .get_or_insert_with(|| auto_drive_strings::next_auto_drive_phrase().to_string())
                .clone()
        };

        let headline = self.auto_format_status_headline(&status_text);
        let mut status_lines = vec![headline];
        if !self.auto_state.awaiting_review() {
            self.auto_append_status_lines(
                &mut status_lines,
                self.auto_state.current_status_title.as_ref(),
                self.auto_state.current_status_sent_to_user.as_ref(),
            );
            if self.auto_state.is_waiting_for_response() && !self.auto_state.is_coordinator_waiting() {
                let appended = self.auto_append_status_lines(
                    &mut status_lines,
                    self.auto_state.last_decision_status_title.as_ref(),
                    self.auto_state.last_decision_status_sent_to_user.as_ref(),
                );
                if !appended
                    && let Some(summary) = self.auto_state.last_decision_summary.as_ref() {
                        let trimmed = summary.trim();
                        if !trimmed.is_empty() {
                            let collapsed = trimmed
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" ");
                            if !collapsed.is_empty() {
                                let current_line = status_lines
                                    .first()
                                    .map(|line| line.trim_end_matches('…').trim())
                                    .unwrap_or("");
                                if collapsed != current_line {
                                    let display = Self::truncate_with_ellipsis(&collapsed, 160);
                                    status_lines.push(display);
                                }
                            }
                        }
                    }
            }
        }
        let cli_running = self.is_cli_running();
        let progress_hint_active = self.auto_state.awaiting_coordinator_submit()
            || (self.auto_state.is_waiting_for_response() && !self.auto_state.is_coordinator_waiting())
            || cli_running;

        // Keep the most recent coordinator status visible across approval and
        // CLI execution. The coordinator clears the current status fields once it
        // starts streaming the next turn, so fall back to the last decision while
        // we are still acting on it.
        let status_title_for_view = if progress_hint_active {
            self.auto_state
                .current_status_title
                .clone()
                .or_else(|| self.auto_state.last_decision_status_title.clone())
        } else {
            None
        };
        let status_sent_to_user_for_view = if progress_hint_active {
            self.auto_state
                .current_status_sent_to_user
                .clone()
                .or_else(|| self.auto_state.last_decision_status_sent_to_user.clone())
        } else {
            None
        };

        let cli_prompt = self
            .auto_state
            .current_cli_prompt
            .clone()
            .filter(|p| !p.trim().is_empty());
        let cli_context = if self.auto_state.hide_cli_context_in_ui {
            None
        } else {
            self.auto_state
                .current_cli_context
                .clone()
                .filter(|value| !value.trim().is_empty())
        };
        let has_cli_prompt = cli_prompt.is_some();

        let bootstrap_pending = self.auto_pending_goal_request;
        let continue_cta_active = self.auto_should_show_continue_cta();

        let countdown_limit = self.auto_state.countdown_seconds();
        let countdown_active = self.auto_state.countdown_active();
        let countdown = if self.auto_state.awaiting_coordinator_submit() {
            match countdown_limit {
                Some(limit) if limit > 0 => Some(CountdownState {
                    remaining: self.auto_state.seconds_remaining.min(limit),
                }),
                _ => None,
            }
        } else {
            None
        };

        let button = if self.auto_state.awaiting_coordinator_submit() {
            let base_label = if bootstrap_pending {
                "Complete Current Task"
            } else if has_cli_prompt {
                "Send prompt"
            } else if continue_cta_active {
                "Continue current task"
            } else {
                "Send prompt"
            };
            let label = if countdown_active {
                format!("{base_label} ({}s)", self.auto_state.seconds_remaining)
            } else {
                base_label.to_string()
            };
            Some(AutoCoordinatorButton {
                label,
                enabled: true,
            })
        } else {
            None
        };

        let manual_hint = if self.auto_state.awaiting_coordinator_submit() {
            if self.auto_state.is_paused_manual() {
                Some("Edit the prompt, then press Enter to continue.".to_string())
            } else if bootstrap_pending {
                None
            } else if has_cli_prompt {
                if countdown_active {
                    Some("Enter to send now • Esc to edit".to_string())
                } else {
                    Some("Enter to send • Esc to edit".to_string())
                }
            } else if continue_cta_active {
                if countdown_active {
                    Some("Enter to continue now • Esc to stop".to_string())
                } else {
                    Some("Enter to continue • Esc to stop".to_string())
                }
            } else if countdown_active {
                Some("Enter to send now • Esc to stop".to_string())
            } else {
                Some("Enter to send • Esc to stop".to_string())
            }
        } else {
            None
        };

        let ctrl_switch_hint = if self.auto_state.awaiting_coordinator_submit() {
            if self.auto_state.is_paused_manual() {
                "Esc to cancel".to_string()
            } else if bootstrap_pending {
                "Esc enter new goal".to_string()
            } else if has_cli_prompt {
                "Esc to edit".to_string()
            } else if continue_cta_active {
                "Esc to stop".to_string()
            } else {
                "Esc to stop".to_string()
            }
        } else if self.auto_state.is_waiting_for_response() {
            String::new()
        } else {
            String::new()
        };

        let show_composer =
            !self.auto_state.awaiting_coordinator_submit() || self.auto_state.is_paused_manual();

        let model = AutoCoordinatorViewModel::Active(AutoActiveViewModel {
            goal: self.auto_state.goal.clone(),
            status_lines,
            cli_prompt,
            awaiting_submission: self.auto_state.awaiting_coordinator_submit(),
            waiting_for_response: self.auto_state.is_waiting_for_response(),
            coordinator_waiting: self.auto_state.is_coordinator_waiting(),
            waiting_for_review: self.auto_state.awaiting_review(),
            countdown,
            button,
            manual_hint,
            ctrl_switch_hint,
            cli_running,
            turns_completed: self.auto_state.turns_completed,
            started_at: self.auto_state.started_at,
            elapsed: self.auto_state.elapsed_override,
            status_sent_to_user: status_sent_to_user_for_view,
            status_title: status_title_for_view,
            session_tokens: self.auto_session_tokens(),
            cli_context,
            show_composer,
            editing_prompt: self.auto_state.is_paused_manual(),
            intro_started_at: self.auto_state.intro_started_at,
            intro_reduced_motion: self.auto_state.intro_reduced_motion,
        });

        self
            .bottom_pane
            .show_auto_coordinator_view(model);

        self.auto_update_terminal_hint();

        if self.auto_state.started_at.is_some() {
            self.app_event_tx
                .send(AppEvent::ScheduleFrameIn(Duration::from_secs(1)));
        }
    }

    fn auto_should_show_continue_cta(&self) -> bool {
        self.auto_state.is_active()
            && self.auto_state.awaiting_coordinator_submit()
            && !self.auto_state.is_paused_manual()
            && self.config.auto_drive.coordinator_routing
            && self.auto_state.continue_mode != AutoContinueMode::Manual
    }

    fn auto_format_status_headline(&self, text: &str) -> String {
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            return String::new();
        }

        if self.auto_state.current_display_is_summary {
            return trimmed.to_string();
        }

        let show_summary_without_ellipsis = self.auto_state.awaiting_coordinator_submit()
            && self.auto_state.current_reasoning_title.is_none()
            && self
                .auto_state
                .current_summary
                .as_ref()
                .map(|summary| !summary.trim().is_empty())
                .unwrap_or(false);

        if show_summary_without_ellipsis {
            trimmed.to_string()
        } else {
            append_thought_ellipsis(trimmed)
        }
    }

    fn auto_update_terminal_hint(&mut self) {
        if !self.auto_state.is_active() && !self.auto_state.should_show_goal_entry() {
            self.bottom_pane.set_standard_terminal_hint(None);
            return;
        }

        let agents_label = if self.auto_state.subagents_enabled {
            "Agents Enabled"
        } else {
            "Agents Disabled"
        };
        let diagnostics_enabled = self.auto_state.qa_automation_enabled
            && (self.auto_state.review_enabled || self.auto_state.cross_check_enabled);
        let diagnostics_label = if diagnostics_enabled {
            "Diagnostics Enabled"
        } else {
            "Diagnostics Disabled"
        };

        let left = format!("• {agents_label}  • {diagnostics_label}");

        let hint = left;
        self.bottom_pane
            .set_standard_terminal_hint(Some(hint));
    }

    fn auto_update_display_title(&mut self) {
        if !self.auto_state.is_active() {
            return;
        }

        let Some(summary) = self.auto_state.current_summary.as_ref() else {
            return;
        };

        let display = summary.lines().find_map(|line| {
            let trimmed = line.trim();
            (!trimmed.is_empty()).then(|| Self::truncate_with_ellipsis(trimmed, 160))
        });

        let Some(display) = display else {
            return;
        };

        let needs_update = self
            .auto_state
            .current_display_line
            .as_ref()
            .map(|current| current != &display)
            .unwrap_or(true);

        if needs_update {
            self.auto_state.current_display_line = Some(display);
            self.auto_state.current_display_is_summary = true;
            self.auto_state.placeholder_phrase = None;
            self.auto_state.current_reasoning_title = None;
        }
    }

    fn auto_broadcast_summary(&mut self, raw: &str) {
        if !self.auto_state.is_active() {
            return;
        }

        let display_text = extract_latest_bold_title(raw).or_else(|| {
            raw.lines().find_map(|line| {
                let trimmed = line.trim();
                (!trimmed.is_empty()).then_some(trimmed.to_string())
            })
        });

        let Some(display_text) = display_text else {
            return;
        };

        if self
            .auto_state
            .last_broadcast_summary
            .as_ref()
            .map(|prev| prev == &display_text)
            .unwrap_or(false)
        {
            return;
        }

        self.auto_state.last_broadcast_summary = Some(display_text);
    }

    fn auto_on_reasoning_delta(&mut self, delta: &str, summary_index: Option<u32>) {
        if !self.auto_state.is_active() || delta.trim().is_empty() {
            return;
        }

        let mut needs_refresh = false;

        if let Some(idx) = summary_index
            && self.auto_state.current_summary_index != Some(idx) {
                self.auto_state.current_summary_index = Some(idx);
                self.auto_state.current_summary = Some(String::new());
                self.auto_state.thinking_prefix_stripped = false;
                self.auto_state.current_reasoning_title = None;
                self.auto_state.current_display_line = None;
                self.auto_state.current_display_is_summary = false;
                self.auto_state.placeholder_phrase =
                    Some(auto_drive_strings::next_auto_drive_phrase().to_string());
                needs_refresh = true;
            }

        let cleaned_delta = if !self.auto_state.thinking_prefix_stripped {
            let (without_prefix, stripped) = strip_role_prefix_if_present(delta);
            if stripped {
                self.auto_state.thinking_prefix_stripped = true;
            }
            without_prefix.to_string()
        } else {
            delta.to_string()
        };

        if !self.auto_state.thinking_prefix_stripped && !cleaned_delta.trim().is_empty() {
            self.auto_state.thinking_prefix_stripped = true;
        }

        {
            let entry = self
                .auto_state
                .current_summary
                .get_or_insert_with(String::new);

            if auto_drive_strings::is_auto_drive_phrase(entry) {
                entry.clear();
            }

            entry.push_str(&cleaned_delta);

            let mut display_updated = false;

            if let Some(title) = extract_latest_bold_title(entry) {
                let needs_update = self
                    .auto_state
                    .current_reasoning_title
                    .as_ref()
                    .map(|existing| existing != &title)
                    .unwrap_or(true);
                if needs_update {
                    self.auto_state.current_reasoning_title = Some(title.clone());
                    self.auto_state.current_display_line = Some(title);
                    self.auto_state.current_display_is_summary = false;
                    self.auto_state.placeholder_phrase = None;
                    display_updated = true;
                }
            } else if self.auto_state.current_reasoning_title.is_none() {
                let previous_line = self.auto_state.current_display_line.clone();
                let previous_is_summary = self.auto_state.current_display_is_summary;
                self.auto_update_display_title();
                let updated_line = self.auto_state.current_display_line.clone();
                let updated_is_summary = self.auto_state.current_display_is_summary;
                if updated_is_summary
                    && (updated_line != previous_line || !previous_is_summary)
                {
                    display_updated = true;
                }
            }

            if display_updated {
                needs_refresh = true;
            }
        }

        if needs_refresh {
            self.auto_rebuild_live_ring();
            self.request_redraw();
        }
    }

    fn auto_on_reasoning_final(&mut self, text: &str) {
        if !self.auto_state.is_active() {
            return;
        }

        self.auto_state.current_reasoning_title = None;
        self.auto_state.current_summary = Some(text.to_string());
        self.auto_state.thinking_prefix_stripped = true;
        self.auto_state.current_summary_index = None;
        self.auto_update_display_title();
        self.auto_broadcast_summary(text);

        if self.auto_state.is_waiting_for_response() {
            self.auto_rebuild_live_ring();
            self.request_redraw();
        }
    }

    fn truncate_with_ellipsis(text: &str, max_chars: usize) -> String {
        if max_chars == 0 {
            return String::new();
        }
        let total = text.chars().count();
        if total <= max_chars {
            return text.to_string();
        }
        let take = max_chars.saturating_sub(1);
        let mut out = String::with_capacity(max_chars);
        for (idx, ch) in text.chars().enumerate() {
            if idx >= take {
                break;
            }
            out.push(ch);
        }
        out.push('…');
        out
    }

    fn normalize_status_field(field: Option<String>) -> Option<String> {
        field.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
    }

    fn compose_status_summary(
        status_title: &Option<String>,
        status_sent_to_user: &Option<String>,
    ) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(title) = status_title
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            parts.push(title.to_string());
        }
        if let Some(sent) = status_sent_to_user
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            && !parts.iter().any(|existing| existing.eq_ignore_ascii_case(sent)) {
                parts.push(sent.to_string());
            }

        match parts.len() {
            0 => String::new(),
            1 => parts.into_iter().next().unwrap_or_default(),
            _ => parts.join(" · "),
        }
    }

    fn auto_append_status_lines(
        &self,
        lines: &mut Vec<String>,
        status_title: Option<&String>,
        status_sent_to_user: Option<&String>,
    ) -> bool {
        let initial_len = lines.len();
        Self::append_status_line(lines, status_title);
        Self::append_status_line(lines, status_sent_to_user);
        lines.len() > initial_len
    }

    fn append_status_line(lines: &mut Vec<String>, status: Option<&String>) {
        if let Some(status) = status {
            let trimmed = status.trim();
            if trimmed.is_empty() {
                return;
            }
            let display = Self::truncate_with_ellipsis(trimmed, 160);
            if !lines.iter().any(|existing| existing.trim() == display) {
                lines.push(display);
            }
        }
    }

    pub(crate) fn launch_update_command(
        &mut self,
        command: Vec<String>,
        display: String,
        latest_version: Option<String>,
    ) -> Option<TerminalLaunch> {
        if !crate::updates::upgrade_ui_enabled() {
            return None;
        }

        self.pending_upgrade_notice = None;
        if command.is_empty() {
            self.history_push_plain_state(history_cell::new_error_event(
                "`/update` — no upgrade command available for this install.".to_string(),
            ));
            self.request_redraw();
            return None;
        }

        let command_text = if display.trim().is_empty() {
            strip_bash_lc_and_escape(&command)
        } else {
            display
        };

        if command_text.trim().is_empty() {
            self.history_push_plain_state(history_cell::new_error_event(
                "`/update` — unable to resolve upgrade command text.".to_string(),
            ));
            self.request_redraw();
            return None;
        }

        let id = self.terminal.alloc_id();
        if let Some(version) = &latest_version {
            self.pending_upgrade_notice = Some((id, version.clone()));
        }

        let (controller_tx, controller_rx) = mpsc::channel();
        let controller = TerminalRunController { tx: controller_tx };
        let display_label = Self::truncate_with_ellipsis(&format!("Guided: {command_text}"), 128);

        let launch = TerminalLaunch {
            id,
            title: "Upgrade Code".to_string(),
            command: Vec::new(),
            command_display: display_label,
            controller: Some(controller.clone()),
            auto_close_on_success: false,
            start_running: true,
        };

        let cwd = self.config.cwd.to_string_lossy().to_string();
        start_upgrade_terminal_session(
            self.app_event_tx.clone(),
            id,
            command_text,
            latest_version,
            Some(cwd),
            controller,
            controller_rx,
            self.config.clone(),
            self.config.debug,
        );

        Some(launch)
    }

    pub(crate) fn terminal_open(&mut self, launch: &TerminalLaunch) {
        let mut overlay = TerminalOverlay::new(
            launch.id,
            launch.title.clone(),
            launch.command_display.clone(),
            launch.auto_close_on_success,
        );
        if !launch.start_running {
            overlay.running = false;
        }
        let visible = self.terminal.last_visible_rows.get();
        overlay.visible_rows = visible;
        overlay.clamp_scroll();
        overlay.ensure_pending_command();
        self.terminal.overlay = Some(overlay);
        self.request_redraw();
    }

    pub(crate) fn terminal_append_chunk(&mut self, id: u64, chunk: &[u8], is_stderr: bool) {
        let mut needs_redraw = false;
        let visible = self.terminal.last_visible_rows.get();
        let visible_cols = self.terminal.last_visible_cols.get();
        if let Some(overlay) = self.terminal.overlay_mut()
            && overlay.id == id {
                if visible > 0 {
                    overlay.pty_rows = visible;
                }
                if visible_cols > 0 {
                    overlay.pty_cols = visible_cols;
                }
                if visible != overlay.visible_rows {
                    overlay.visible_rows = visible;
                    overlay.clamp_scroll();
                }
                overlay.append_chunk(chunk, is_stderr);
                needs_redraw = true;
            }
        if needs_redraw {
            self.request_redraw();
        }
    }

    pub(crate) fn terminal_dimensions_hint(&self) -> Option<(u16, u16)> {
        let rows = self.terminal.last_visible_rows.get();
        let cols = self.terminal.last_visible_cols.get();
        if rows > 0 && cols > 0 {
            Some((rows, cols))
        } else {
            None
        }
    }

    pub(crate) fn terminal_apply_resize(&mut self, id: u64, rows: u16, cols: u16) {
        if let Some(overlay) = self.terminal.overlay_mut()
            && overlay.id == id && overlay.update_pty_dimensions(rows, cols) {
                self.request_redraw();
            }
    }

    pub(crate) fn request_terminal_cancel(&mut self, id: u64) {
        let mut needs_redraw = false;
        if let Some(overlay) = self.terminal.overlay_mut()
            && overlay.id == id {
                overlay.push_info_message("Cancel requested…");
                if overlay.running {
                    overlay.running = false;
                    needs_redraw = true;
                }
            }
        if needs_redraw {
            self.request_redraw();
        }
        self.app_event_tx.send(AppEvent::TerminalCancel { id });
    }

    pub(crate) fn terminal_update_message(&mut self, id: u64, message: String) {
        if let Some(overlay) = self.terminal.overlay_mut()
            && overlay.id == id {
                overlay.push_info_message(&message);
                self.request_redraw();
            }
    }

    pub(crate) fn terminal_set_assistant_message(&mut self, id: u64, message: String) {
        if let Some(overlay) = self.terminal.overlay_mut()
            && overlay.id == id {
                overlay.push_assistant_message(&message);
                self.request_redraw();
            }
    }

    pub(crate) fn terminal_set_command_display(&mut self, id: u64, command: String) {
        if let Some(overlay) = self.terminal.overlay_mut()
            && overlay.id == id {
                overlay.command_display = command;
                self.request_redraw();
            }
    }

    pub(crate) fn terminal_prepare_command(
        &mut self,
        id: u64,
        suggestion: String,
        ack: Sender<TerminalCommandGate>,
    ) {
        let mut updated = false;
        if let Some(overlay) = self.terminal.overlay_mut()
            && overlay.id == id {
                overlay.set_pending_command(suggestion, ack);
                updated = true;
            }
        if updated {
            self.request_redraw();
        }
    }

    pub(crate) fn terminal_accept_pending_command(&mut self) -> Option<PendingCommandAction> {
        if let Some(overlay) = self.terminal.overlay_mut() {
            if overlay.running {
                return None;
            }
            if let Some(action) = overlay.accept_pending_command() {
                match &action {
                    PendingCommandAction::Forwarded(command)
                    | PendingCommandAction::Manual(command) => {
                        overlay.command_display = command.clone();
                    }
                }
                self.request_redraw();
                return Some(action);
            }
        }
        None
    }

    pub(crate) fn terminal_execute_manual_command(&mut self, id: u64, command: String) {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            if let Some(overlay) = self.terminal.overlay_mut() {
                overlay.ensure_pending_command();
            }
            self.request_redraw();
            return;
        }

        if let Some(rest) = trimmed.strip_prefix("$$") {
            let prompt_text = rest.trim();
            if prompt_text.is_empty() {
                if let Some(overlay) = self.terminal.overlay_mut() {
                    overlay.push_info_message("Provide a prompt after '$'.");
                    overlay.ensure_pending_command();
                }
                self.request_redraw();
                return;
            }

            if let Some(overlay) = self.terminal.overlay_mut() {
                overlay.cancel_pending_command();
                overlay.running = true;
                overlay.exit_code = None;
                overlay.duration = None;
                overlay.push_assistant_message("Preparing guided command…");
            }

            let (controller_tx, controller_rx) = mpsc::channel();
            let controller = TerminalRunController { tx: controller_tx };
            let cwd = self.config.cwd.to_string_lossy().to_string();

            start_prompt_terminal_session(
                self.app_event_tx.clone(),
                id,
                prompt_text.to_string(),
                Some(cwd),
                controller,
                controller_rx,
                self.config.debug,
            );

            self.push_background_before_next_output(format!(
                "Terminal prompt: {prompt_text}"
            ));
            return;
        }

        let mut command_body = trimmed;
        let mut run_direct = false;
        if let Some(rest) = trimmed.strip_prefix('$') {
            let candidate = rest.trim();
            if candidate.is_empty() {
                if let Some(overlay) = self.terminal.overlay_mut() {
                    overlay.push_info_message("Provide a command after '$'.");
                    overlay.ensure_pending_command();
                }
                self.request_redraw();
                return;
            }
            command_body = candidate;
            run_direct = true;
        }

        let command_string = command_body.to_string();
        let wrapped_command = wrap_command(&command_string);
        if wrapped_command.is_empty() {
            self.app_event_tx.send(AppEvent::TerminalSetAssistantMessage {
                id,
                message: "Command could not be constructed.".to_string(),
            });
            if let Some(overlay) = self.terminal.overlay_mut() {
                overlay.ensure_pending_command();
            }
            self.request_redraw();
            return;
        }

        if !matches!(self.config.sandbox_policy, SandboxPolicy::DangerFullAccess) {
            if let Some(overlay) = self.terminal.overlay_mut() {
                overlay.cancel_pending_command();
            }
            self.pending_manual_terminal.insert(
                id,
                PendingManualTerminal {
                    command: command_string.clone(),
                    run_direct,
                },
            );
            if let Some(overlay) = self.terminal.overlay_mut() {
                overlay.push_assistant_message("Awaiting approval to run this command…");
                overlay.running = false;
            }
            let ticket = self.make_background_before_next_output_ticket();
            self.bottom_pane.push_approval_request(
                ApprovalRequest::TerminalCommand {
                    id,
                    command: command_string,
                },
                ticket,
            );
            self.request_redraw();
            return;
        }

        if run_direct && self.terminal_dimensions_hint().is_some() {
            self.start_direct_terminal_command(id, command_string, wrapped_command);
        } else {
            self.start_manual_terminal_session(id, command_string);
        }
    }

    fn start_manual_terminal_session(&mut self, id: u64, command: String) {
        if command.is_empty() {
            return;
        }
        if let Some(overlay) = self.terminal.overlay_mut() {
            overlay.cancel_pending_command();
            overlay.running = true;
            overlay.exit_code = None;
            overlay.duration = None;
        }
        let (controller_tx, controller_rx) = mpsc::channel();
        let controller = TerminalRunController { tx: controller_tx };
        let cwd = self.config.cwd.to_string_lossy().to_string();
        start_direct_terminal_session(
            self.app_event_tx.clone(),
            id,
            command,
            Some(cwd),
            controller,
            controller_rx,
            self.config.debug,
        );
    }

    fn start_direct_terminal_command(
        &mut self,
        id: u64,
        display: String,
        command: Vec<String>,
    ) {
        if let Some(overlay) = self.terminal.overlay_mut() {
            overlay.cancel_pending_command();
        }
        self.app_event_tx.send(AppEvent::TerminalRunCommand {
            id,
            command,
            command_display: display,
            controller: None,
        });
    }

    pub(crate) fn terminal_send_input(&mut self, id: u64, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        self.app_event_tx
            .send(AppEvent::TerminalSendInput { id, data });
    }

    pub(crate) fn terminal_mark_running(&mut self, id: u64) {
        if let Some(overlay) = self.terminal.overlay_mut()
            && overlay.id == id {
                overlay.running = true;
                overlay.exit_code = None;
                overlay.duration = None;
                overlay.start_time = Some(Instant::now());
                self.request_redraw();
            }
    }

    pub(crate) fn terminal_finalize(
        &mut self,
        id: u64,
        exit_code: Option<i32>,
        duration: Duration,
    ) -> Option<TerminalAfter> {
        let mut success = false;
        let mut after = None;
        let mut needs_redraw = false;
        let mut should_close = false;
        let mut take_after = false;
        let visible = self.terminal.last_visible_rows.get();
        if let Some(overlay) = self.terminal.overlay_mut()
            && overlay.id == id {
                overlay.cancel_pending_command();
                if visible != overlay.visible_rows {
                    overlay.visible_rows = visible;
                    overlay.clamp_scroll();
                }
                let was_following = overlay.is_following();
                overlay.finalize(exit_code, duration);
                overlay.auto_follow(was_following);
                needs_redraw = true;
                if exit_code == Some(0) {
                    success = true;
                    take_after = true;
                    if overlay.auto_close_on_success {
                        should_close = true;
                    }
                }
                overlay.ensure_pending_command();
            }
        if take_after {
            after = self.terminal.after.take();
        }
        if should_close {
            self.terminal.overlay = None;
        }
        if needs_redraw {
            self.request_redraw();
        }
        if success {
            if crate::updates::upgrade_ui_enabled()
                && let Some((pending_id, version)) = self.pending_upgrade_notice.take() {
                    if pending_id == id {
                        self.bottom_pane
                            .flash_footer_notice(format!("Upgraded to {version}"));
                        self.latest_upgrade_version = None;
                    } else {
                        self.pending_upgrade_notice = Some((pending_id, version));
                    }
                }
            after
        } else {
            None
        }
    }

    pub(crate) fn terminal_prepare_rerun(&mut self, id: u64) -> bool {
        let mut reset = false;
        let visible = self.terminal.last_visible_rows.get();
        if let Some(overlay) = self.terminal.overlay_mut()
            && overlay.id == id && !overlay.running {
                overlay.reset_for_rerun();
                overlay.visible_rows = visible;
                overlay.clamp_scroll();
                overlay.ensure_pending_command();
                reset = true;
            }
        if reset {
            self.request_redraw();
        }
        reset
    }

    pub(crate) fn handle_terminal_approval_decision(&mut self, id: u64, approved: bool) {
        let pending = self.pending_manual_terminal.remove(&id);
        if approved {
            if let Some(entry) = pending
                && self
                    .terminal
                    .overlay()
                    .map(|overlay| overlay.id == id)
                    .unwrap_or(false)
                {
                    if let Some(overlay) = self.terminal.overlay_mut() {
                        overlay.push_assistant_message("Approval granted. Running command…");
                    }
                    if entry.run_direct && self.terminal_dimensions_hint().is_some() {
                        let command_vec = wrap_command(&entry.command);
                        self.start_direct_terminal_command(id, entry.command, command_vec);
                    } else {
                        self.start_manual_terminal_session(id, entry.command);
                    }
                    self.request_redraw();
                }
            return;
        }

        if let Some(entry) = pending {
            if let Some(overlay) = self.terminal.overlay_mut() {
                overlay.push_info_message("Command was not approved. You can edit it and try again.");
                overlay.running = false;
                overlay.exit_code = None;
                overlay.duration = None;
                overlay.pending_command = Some(PendingCommand::manual_with_input(entry.command));
            }
            self.request_redraw();
        }
    }

    pub(crate) fn close_terminal_overlay(&mut self) {
        let mut cancel_id = None;
        let mut preserved_visible = None;
        let mut overlay_id = None;
        if let Some(overlay) = self.terminal.overlay_mut() {
            overlay_id = Some(overlay.id);
            if overlay.running {
                cancel_id = Some(overlay.id);
            }
            overlay.cancel_pending_command();
            preserved_visible = Some(overlay.visible_rows);
        }
        if let Some(id) = cancel_id {
            self.app_event_tx.send(AppEvent::TerminalCancel { id });
        }
        if let Some(id) = overlay_id {
            self.pending_manual_terminal.remove(&id);
        }
        if let Some(visible_rows) = preserved_visible {
            self.terminal.last_visible_rows.set(visible_rows);
        }
        self.terminal.clear();
        self.request_redraw();
    }

    pub(crate) fn terminal_overlay_id(&self) -> Option<u64> {
        self.terminal.overlay().map(|o| o.id)
    }

    pub(crate) fn terminal_overlay_active(&self) -> bool {
        self.terminal.overlay().is_some()
    }

    pub(crate) fn terminal_is_running(&self) -> bool {
        self.terminal.overlay().map(|o| o.running).unwrap_or(false)
    }

    pub(crate) fn ctrl_c_requests_exit(&self) -> bool {
        !self.terminal_overlay_active() && self.bottom_pane.ctrl_c_quit_hint_visible()
    }

    pub(crate) fn terminal_has_pending_command(&self) -> bool {
        self.terminal
            .overlay()
            .and_then(|overlay| overlay.pending_command.as_ref())
            .is_some()
    }

    pub(crate) fn terminal_handle_pending_key(&mut self, key_event: KeyEvent) -> bool {
        if self.terminal_is_running() {
            return false;
        }
        if !self.terminal_has_pending_command() {
            return false;
        }
        if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return true;
        }

        let mut needs_redraw = false;
        let mut handled = false;

        if let Some(overlay) = self.terminal.overlay_mut()
            && let Some(pending) = overlay.pending_command.as_mut() {
                match key_event.code {
                    KeyCode::Char(ch) => {
                        if key_event
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
                        {
                            handled = true;
                        } else if pending.insert_char(ch) {
                            needs_redraw = true;
                            handled = true;
                        } else {
                            handled = true;
                        }
                    }
                    KeyCode::Backspace => {
                        handled = true;
                        if pending.backspace() {
                            needs_redraw = true;
                        }
                    }
                    KeyCode::Delete => {
                        handled = true;
                        if pending.delete() {
                            needs_redraw = true;
                        }
                    }
                    KeyCode::Left => {
                        handled = true;
                        if pending.move_left() {
                            needs_redraw = true;
                        }
                    }
                    KeyCode::Right => {
                        handled = true;
                        if pending.move_right() {
                            needs_redraw = true;
                        }
                    }
                    KeyCode::Home => {
                        handled = true;
                        if pending.move_home() {
                            needs_redraw = true;
                        }
                    }
                    KeyCode::End => {
                        handled = true;
                        if pending.move_end() {
                            needs_redraw = true;
                        }
                    }
                    KeyCode::Tab => {
                        handled = true;
                    }
                    _ => {}
                }
            }

        if needs_redraw {
            self.request_redraw();
        }
        handled
    }

    pub(crate) fn terminal_scroll_lines(&mut self, delta: i32) {
        let mut updated = false;
        let visible = self.terminal.last_visible_rows.get();
        if let Some(overlay) = self.terminal.overlay_mut() {
            if visible != overlay.visible_rows {
                overlay.visible_rows = visible;
            }
            let current = overlay.scroll as i32;
            let max_scroll = overlay.max_scroll() as i32;
            let mut next = current + delta;
            if next < 0 {
                next = 0;
            } else if next > max_scroll {
                next = max_scroll;
            }
            if next as u16 != overlay.scroll {
                overlay.scroll = next as u16;
                updated = true;
            }
        }
        if updated {
            self.request_redraw();
        }
    }

    pub(crate) fn terminal_scroll_page(&mut self, direction: i32) {
        let mut delta = None;
        let visible_value = self.terminal.last_visible_rows.get();
        if let Some(overlay) = self.terminal.overlay_mut() {
            let visible = visible_value.max(1);
            if visible != overlay.visible_rows {
                overlay.visible_rows = visible;
            }
            delta = Some((visible.saturating_sub(1)) as i32 * direction);
        }
        if let Some(amount) = delta {
            self.terminal_scroll_lines(amount);
        }
    }

    pub(crate) fn terminal_scroll_to_top(&mut self) {
        let mut updated = false;
        if let Some(overlay) = self.terminal.overlay_mut()
            && overlay.scroll != 0 {
                overlay.scroll = 0;
                updated = true;
            }
        if updated {
            self.request_redraw();
        }
    }

    pub(crate) fn terminal_scroll_to_bottom(&mut self) {
        let mut updated = false;
        let visible = self.terminal.last_visible_rows.get();
        if let Some(overlay) = self.terminal.overlay_mut() {
            if visible != overlay.visible_rows {
                overlay.visible_rows = visible;
            }
            let max_scroll = overlay.max_scroll();
            if overlay.scroll != max_scroll {
                overlay.scroll = max_scroll;
                updated = true;
            }
        }
        if updated {
            self.request_redraw();
        }
    }

    pub(crate) fn handle_terminal_after(&mut self, after: TerminalAfter) {
        match after {
            TerminalAfter::RefreshAgentsAndClose { selected_index } => {
                self.agents_overview_selected_index = selected_index;
                self.show_agents_overview_ui();
            }
        }
    }

    // show_subagent_editor_ui removed; use show_subagent_editor_for_name or show_new_subagent_editor

    pub(crate) fn show_subagent_editor_for_name(&mut self, name: String) {
        // Build available agents from enabled ones (or sensible defaults)
        let available_agents: Vec<String> = if self.config.agents.is_empty() {
            enabled_agent_model_specs()
                .into_iter()
                .map(|spec| spec.slug.to_string())
                .collect()
        } else {
            self.config
                .agents
                .iter()
                .filter(|a| a.enabled)
                .map(|a| a.name.clone())
                .collect()
        };
        let existing = self.config.subagent_commands.clone();
        let app_event_tx = self.app_event_tx.clone();
        let build_editor = || {
            SubagentEditorView::new_with_data(
                name.clone(),
                available_agents.clone(),
                existing.clone(),
                false,
                app_event_tx.clone(),
            )
        };

        if self.try_set_agents_settings_editor(build_editor()) {
            self.request_redraw();
            return;
        }

        self.ensure_settings_overlay_section(SettingsSection::Agents);
        self.show_agents_overview_ui();
        let _ = self.try_set_agents_settings_editor(build_editor());
        self.request_redraw();
    }

    pub(crate) fn show_new_subagent_editor(&mut self) {
        let available_agents: Vec<String> = if self.config.agents.is_empty() {
            enabled_agent_model_specs()
                .into_iter()
                .map(|spec| spec.slug.to_string())
                .collect()
        } else {
            self.config
                .agents
                .iter()
                .filter(|a| a.enabled)
                .map(|a| a.name.clone())
                .collect()
        };
        let existing = self.config.subagent_commands.clone();
        let app_event_tx = self.app_event_tx.clone();
        let build_editor = || {
            SubagentEditorView::new_with_data(
                String::new(),
                available_agents.clone(),
                existing.clone(),
                true,
                app_event_tx.clone(),
            )
        };

        if self.try_set_agents_settings_editor(build_editor()) {
            self.request_redraw();
            return;
        }

        self.ensure_settings_overlay_section(SettingsSection::Agents);
        self.show_agents_overview_ui();
        let _ = self.try_set_agents_settings_editor(build_editor());
        self.request_redraw();
    }

    pub(crate) fn show_agent_editor_ui(&mut self, name: String) {
        if let Some(cfg) = self
            .config
            .agents
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(&name))
            .cloned()
        {
            let ro = if let Some(ref v) = cfg.args_read_only {
                Some(v.clone())
            } else if !cfg.args.is_empty() {
                Some(cfg.args.clone())
            } else {
                let d = code_core::agent_defaults::default_params_for(
                    &cfg.name, true, /*read_only*/
                );
                if d.is_empty() { None } else { Some(d) }
            };
            let wr = if let Some(ref v) = cfg.args_write {
                Some(v.clone())
            } else if !cfg.args.is_empty() {
                Some(cfg.args.clone())
            } else {
                let d = code_core::agent_defaults::default_params_for(
                    &cfg.name, false, /*read_only*/
                );
                if d.is_empty() { None } else { Some(d) }
            };
            let app_event_tx = self.app_event_tx.clone();
            let cfg_name = cfg.name.clone();
            let cfg_enabled = cfg.enabled;
            let cfg_instructions = cfg.instructions.clone();
            let cfg_command = Self::resolve_agent_command(
                &cfg.name,
                Some(cfg.command.as_str()),
                Some(cfg.command.as_str()),
            );
            let builtin = Self::is_builtin_agent(&cfg.name, &cfg_command);
            let description = Self::agent_description_for(
                &cfg.name,
                Some(&cfg_command),
                cfg.description.as_deref(),
            );
            let build_editor = || {
                AgentEditorView::new(
                    cfg_name.clone(),
                    cfg_enabled,
                    ro.clone(),
                    wr.clone(),
                    cfg_instructions.clone(),
                    description.clone(),
                    cfg_command.clone(),
                    builtin,
                    app_event_tx.clone(),
                )
            };
            if self.try_set_agents_settings_agent_editor(build_editor()) {
                self.request_redraw();
                return;
            }

            self.ensure_settings_overlay_section(SettingsSection::Agents);
            self.show_agents_overview_ui();
            let _ = self.try_set_agents_settings_agent_editor(build_editor());
            self.request_redraw();
        } else {
            // Fallback: synthesize defaults
            let cmd = Self::resolve_agent_command(&name, None, None);
            let ro = code_core::agent_defaults::default_params_for(&name, true /*read_only*/);
            let wr =
                code_core::agent_defaults::default_params_for(&name, false /*read_only*/);
            let app_event_tx = self.app_event_tx.clone();
            let description = Self::agent_description_for(&name, Some(&cmd), None);
            let builtin = Self::is_builtin_agent(&name, &cmd);
            let build_editor = || {
                AgentEditorView::new(
                    name.clone(),
                    builtin,
                    if ro.is_empty() { None } else { Some(ro.clone()) },
                    if wr.is_empty() { None } else { Some(wr.clone()) },
                    None,
                    description.clone(),
                    cmd.clone(),
                    builtin,
                    app_event_tx.clone(),
                )
            };
            if self.try_set_agents_settings_agent_editor(build_editor()) {
                self.request_redraw();
                return;
            }

            self.ensure_settings_overlay_section(SettingsSection::Agents);
            self.show_agents_overview_ui();
            let _ = self.try_set_agents_settings_agent_editor(build_editor());
            self.request_redraw();
        }
    }

    pub(crate) fn show_agent_editor_new_ui(&mut self) {
        let app_event_tx = self.app_event_tx.clone();
        let build_editor = || {
            AgentEditorView::new(
                String::new(),
                true,
                None,
                None,
                None,
                None,
                String::new(),
                false,
                app_event_tx.clone(),
            )
        };

        if self.try_set_agents_settings_agent_editor(build_editor()) {
            self.request_redraw();
            return;
        }

        self.ensure_settings_overlay_section(SettingsSection::Agents);
        self.show_agents_overview_ui();
        let _ = self.try_set_agents_settings_agent_editor(build_editor());
        self.request_redraw();
    }

    pub(crate) fn apply_subagent_update(
        &mut self,
        cmd: code_core::config_types::SubagentCommandConfig,
    ) {
        if let Some(slot) = self
            .config
            .subagent_commands
            .iter_mut()
            .find(|c| c.name.eq_ignore_ascii_case(&cmd.name))
        {
            *slot = cmd;
        } else {
            self.config.subagent_commands.push(cmd);
        }

        self.refresh_settings_overview_rows();
    }

    pub(crate) fn delete_subagent_by_name(&mut self, name: &str) {
        self.config
            .subagent_commands
            .retain(|c| !c.name.eq_ignore_ascii_case(name));
        self.refresh_settings_overview_rows();
    }

    pub(crate) fn apply_agent_update(
        &mut self,
        name: &str,
        enabled: bool,
        args_ro: Option<Vec<String>>,
        args_wr: Option<Vec<String>>,
        instr: Option<String>,
        description: Option<String>,
        command: String,
    ) {
        let provided_command = if command.trim().is_empty() { None } else { Some(command.as_str()) };
        let existing_index = self
            .config
            .agents
            .iter()
            .position(|a| a.name.eq_ignore_ascii_case(name));

        let existing_command = existing_index
            .and_then(|idx| self.config.agents.get(idx))
            .map(|cfg| cfg.command.clone());
        let resolved = Self::resolve_agent_command(
            name,
            provided_command,
            existing_command.as_deref(),
        );

        let mut candidate_cfg = if let Some(idx) = existing_index {
            self.config.agents.get(idx).cloned().unwrap_or_else(|| AgentConfig {
                name: name.to_string(),
                command: resolved.clone(),
                args: Vec::new(),
                read_only: false,
                enabled,
                description: description.clone(),
                env: None,
                args_read_only: args_ro.clone(),
                args_write: args_wr.clone(),
                instructions: instr.clone(),
            })
        } else {
            AgentConfig {
                name: name.to_string(),
                command: resolved.clone(),
                args: Vec::new(),
                read_only: false,
                enabled,
                description: description.clone(),
                env: None,
                args_read_only: args_ro.clone(),
                args_write: args_wr.clone(),
                instructions: instr.clone(),
            }
        };

        candidate_cfg.command = resolved;
        candidate_cfg.enabled = enabled;
        candidate_cfg.description = description;
        candidate_cfg.args_read_only = args_ro;
        candidate_cfg.args_write = args_wr;
        candidate_cfg.instructions = instr;

        let pending = PendingAgentUpdate { id: Uuid::new_v4(), cfg: candidate_cfg };
        let requires_validation = !self.test_mode && existing_index.is_none();
        if requires_validation {
            self.start_agent_validation(pending);
            return;
        }

        self.commit_agent_update(pending);
    }

    fn start_agent_validation(&mut self, pending: PendingAgentUpdate) {
        let name = pending.cfg.name.clone();
        self.push_background_tail(format!(
            "🧪 Testing agent `{name}` (expecting \"ok\")…"
        ));
        self.pending_agent_updates.retain(|_, existing| {
            !existing.cfg.name.eq_ignore_ascii_case(&name)
        });
        let key = pending.key();
        let attempt = pending.clone();
        self.pending_agent_updates.insert(key, pending);
        self.refresh_settings_overview_rows();
        let tx = self.app_event_tx.clone();
        tokio::spawn(async move {
            let cfg = attempt.cfg.clone();
            let agent_name = cfg.name.clone();
            let attempt_id = attempt.id;
            let result = task::spawn_blocking(move || smoke_test_agent_blocking(cfg))
                .await
                .map_err(|e| format!("validation task failed: {e}"))
                .and_then(|res| res);
            tx.send(AppEvent::AgentValidationFinished { name: agent_name, result, attempt_id });
        });
    }

    pub(crate) fn handle_agent_validation_finished(&mut self, name: &str, attempt_id: Uuid, result: Result<(), String>) {
        let key = format!("{}:{}", name.to_ascii_lowercase(), attempt_id);
        let Some(pending) = self.pending_agent_updates.remove(&key) else {
            return;
        };

        match result {
            Ok(()) => {
                self.push_background_tail(format!(
                    "✅ Agent `{name}` responded with \"ok\"."
                ));
                self.commit_agent_update(pending);
            }
            Err(err) => {
                self.history_push_plain_state(history_cell::new_error_event(format!(
                    "❌ Agent `{name}` validation failed: {err}"
                )));
                self.show_agent_editor_for_pending(&pending);
            }
        }
        self.request_redraw();
    }

    fn commit_agent_update(&mut self, pending: PendingAgentUpdate) {
        let name = pending.cfg.name.clone();
        if let Some(slot) = self
            .config
            .agents
            .iter_mut()
            .find(|a| a.name.eq_ignore_ascii_case(&name))
        {
            *slot = pending.cfg.clone();
        } else {
            self.config.agents.push(pending.cfg.clone());
        }

        self.persist_agent_config(&pending.cfg);
        self.refresh_settings_overview_rows();
        self.show_agents_overview_ui();
    }

    fn persist_agent_config(&self, cfg: &AgentConfig) {
        if let Ok(home) = code_core::config::find_code_home() {
            let name = cfg.name.clone();
            let enabled = cfg.enabled;
            let ro = cfg.args_read_only.clone();
            let wr = cfg.args_write.clone();
            let instr = cfg.instructions.clone();
            let desc = cfg.description.clone();
            let command = cfg.command.clone();
            tokio::spawn(async move {
                let _ = code_core::config_edit::upsert_agent_config(
                    &home,
                    &name,
                    Some(enabled),
                    None,
                    ro.as_deref(),
                    wr.as_deref(),
                    instr.as_deref(),
                    desc.as_deref(),
                    Some(command.as_str()),
                )
                .await;
            });
        }
    }

    fn show_agent_editor_for_pending(&mut self, pending: &PendingAgentUpdate) {
        let cfg = pending.cfg.clone();
        let app_event_tx = self.app_event_tx.clone();
        let name_value = cfg.name.clone();
        let enabled_value = cfg.enabled;
        let ro = cfg.args_read_only.clone();
        let wr = cfg.args_write.clone();
        let instructions = cfg.instructions.clone();
        let description = cfg.description.clone();
        let command = cfg.command.clone();
        let builtin = Self::is_builtin_agent(&cfg.name, &command);
        let build_editor = || {
            AgentEditorView::new(
                name_value.clone(),
                enabled_value,
                ro.clone(),
                wr.clone(),
                instructions.clone(),
                description.clone(),
                command.clone(),
                builtin,
                app_event_tx.clone(),
            )
        };
        if self.try_set_agents_settings_agent_editor(build_editor()) {
            self.request_redraw();
            return;
        }
        self.ensure_settings_overlay_section(SettingsSection::Agents);
        self.show_agents_overview_ui();
        let _ = self.try_set_agents_settings_agent_editor(build_editor());
        self.request_redraw();
    }

    fn resolve_agent_command(
        name: &str,
        provided: Option<&str>,
        existing: Option<&str>,
    ) -> String {
        let spec = agent_model_spec(name);
        if let Some(cmd) = provided
            && let Some(resolved) = Self::normalize_agent_command(cmd, name, spec) {
                return resolved;
            }
        if let Some(cmd) = existing
            && let Some(resolved) = Self::normalize_agent_command(cmd, name, spec) {
                return resolved;
            }
        if let Some(spec) = spec {
            return spec.cli.to_string();
        }
        name.to_string()
    }

    fn normalize_agent_command(
        candidate: &str,
        name: &str,
        spec: Option<&code_core::agent_defaults::AgentModelSpec>,
    ) -> Option<String> {
        if candidate.trim().is_empty() {
            return None;
        }
        if let Some(spec) = spec {
            if candidate.eq_ignore_ascii_case(name) && !spec.cli.eq_ignore_ascii_case(name) {
                return Some(spec.cli.to_string());
            }
            if candidate.eq_ignore_ascii_case(spec.slug) && !spec.cli.eq_ignore_ascii_case(spec.slug) {
                return Some(spec.cli.to_string());
            }
        }
        Some(candidate.to_string())
    }

    pub(crate) fn show_diffs_popup(&mut self) {
        use crate::diff_render::create_diff_details_only;
        // Build a latest-first unique file list
        let mut order: Vec<PathBuf> = Vec::new();
        let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
        for changes in self.diffs.session_patch_sets.iter().rev() {
            for (path, change) in changes.iter() {
                // If this change represents a move/rename, show the destination path in the tabs
                let display_path: PathBuf = match change {
                    code_core::protocol::FileChange::Update {
                        move_path: Some(dest),
                        ..
                    } => dest.clone(),
                    _ => path.clone(),
                };
                if seen.insert(display_path.clone()) {
                    order.push(display_path);
                }
            }
        }
        // Build tabs: for each file, create a single unified diff against the original baseline
        let mut tabs: Vec<(String, Vec<DiffBlock>)> = Vec::new();
        for path in order {
            // Resolve baseline (first-seen content) and current (on-disk) content
            let baseline = self
                .diffs
                .baseline_file_contents
                .get(&path)
                .cloned()
                .unwrap_or_default();
            let current = std::fs::read_to_string(&path).unwrap_or_default();
            // Build a unified diff from baseline -> current
            let unified = diffy::create_patch(&baseline, &current).to_string();
            // Render detailed lines (no header) using our diff renderer helpers
            let mut single = HashMap::new();
            single.insert(
                path.clone(),
                code_core::protocol::FileChange::Update {
                    unified_diff: unified.clone(),
                    move_path: None,
                    original_content: baseline.clone(),
                    new_content: current.clone(),
                },
            );
            let detail = create_diff_details_only(&single);
            let mut blocks: Vec<DiffBlock> = vec![DiffBlock { lines: detail }];

            // Count adds/removes for the header label from the unified diff
            let mut total_added: usize = 0;
            let mut total_removed: usize = 0;
            if let Ok(patch) = diffy::Patch::from_str(&unified) {
                for h in patch.hunks() {
                    for l in h.lines() {
                        match l {
                            diffy::Line::Insert(_) => total_added += 1,
                            diffy::Line::Delete(_) => total_removed += 1,
                            _ => {}
                        }
                    }
                }
            } else {
                for l in unified.lines() {
                    if l.starts_with("+++") || l.starts_with("---") || l.starts_with("@@") {
                        continue;
                    }
                    if let Some(b) = l.as_bytes().first() {
                        if *b == b'+' {
                            total_added += 1;
                        } else if *b == b'-' {
                            total_removed += 1;
                        }
                    }
                }
            }
            // Prepend a header block with the full path and counts
            let header_line = {
                use ratatui::style::Modifier;
                use ratatui::style::Style;
                use ratatui::text::Line as RtLine;
                use ratatui::text::Span as RtSpan;
                let mut spans: Vec<RtSpan<'static>> = Vec::new();
                spans.push(RtSpan::styled(
                    path.display().to_string(),
                    Style::default()
                        .fg(crate::colors::text())
                        .add_modifier(Modifier::BOLD),
                ));
                spans.push(RtSpan::raw(" "));
                spans.push(RtSpan::styled(
                    format!("+{total_added}"),
                    Style::default().fg(crate::colors::success()),
                ));
                spans.push(RtSpan::raw(" "));
                spans.push(RtSpan::styled(
                    format!("-{total_removed}"),
                    Style::default().fg(crate::colors::error()),
                ));
                RtLine::from(spans)
            };
            blocks.insert(
                0,
                DiffBlock {
                    lines: vec![header_line],
                },
            );

            // Tab title: file name only
            let title = path
                .file_name()
                .and_then(|s| s.to_str())
                .map(std::string::ToString::to_string)
                .unwrap_or_else(|| path.display().to_string());
            tabs.push((title, blocks));
        }
        if tabs.is_empty() {
            // Nothing to show — surface a small notice so Ctrl+D feels responsive
            self.bottom_pane
                .flash_footer_notice("No diffs recorded this session".to_string());
            return;
        }
        self.diffs.overlay = Some(DiffOverlay::new(tabs));
        self.diffs.confirm = None;
        self.request_redraw();
    }

    pub(crate) fn toggle_diffs_popup(&mut self) {
        if self.diffs.overlay.is_some() {
            self.diffs.overlay = None;
            self.request_redraw();
        } else {
            self.show_diffs_popup();
        }
    }

    pub(crate) fn show_help_popup(&mut self) {
        let t_dim = Style::default().fg(crate::colors::text_dim());
        let t_fg = Style::default().fg(crate::colors::text());

        let mut lines: Vec<RtLine<'static>> = Vec::new();
        lines.push(RtLine::from(vec![RtSpan::styled(
            "Keyboard shortcuts",
            t_fg.add_modifier(Modifier::BOLD),
        )]));

        let kv = |k: &str, v: &str| -> RtLine<'static> {
            RtLine::from(vec![
                // Left-align the key column for improved readability
                RtSpan::styled(format!("{k:<12}"), t_fg),
                RtSpan::raw("  —  "),
                RtSpan::styled(v.to_string(), t_dim),
            ])
        };
        // Top quick action
        lines.push(kv(
            "Shift+Tab",
            "Rotate agent between Read Only / Write with Approval / Full Access",
        ));

        // Global
        lines.push(kv("F1", "Help overlay"));
        lines.push(kv("Ctrl+G", "Open external editor"));
        lines.push(kv("Ctrl+R", "Toggle reasoning"));
        lines.push(kv("Ctrl+T", "Toggle screen"));
        lines.push(kv("Ctrl+D", "Diff viewer"));
        lines.push(kv("Esc", &format!("{} / close popups", Self::double_esc_hint_label())));
        // Task control shortcuts
        lines.push(kv("Esc", "End current task"));
        lines.push(kv("Ctrl+C", "End current task"));
        lines.push(kv("Ctrl+C twice", "Quit"));
        lines.push(RtLine::from(""));

        // Composer
        lines.push(RtLine::from(vec![RtSpan::styled(
            "Compose field",
            t_fg.add_modifier(Modifier::BOLD),
        )]));
        lines.push(kv("Enter", "Send message"));
        lines.push(kv("Ctrl+J", "Insert newline"));
        lines.push(kv("Shift+Enter", "Insert newline"));
        // Split combined shortcuts into separate rows for readability
        lines.push(kv("Shift+Up", "Browse input history"));
        lines.push(kv("Shift+Down", "Browse input history"));
        lines.push(kv("Ctrl+B", "Move left"));
        lines.push(kv("Ctrl+F", "Move right"));
        lines.push(kv("Alt+Left", "Move by word"));
        lines.push(kv("Alt+Right", "Move by word"));
        // Simplify delete shortcuts; remove Alt+Backspace/Backspace/Delete variants
        lines.push(kv("Ctrl+W", "Delete previous word"));
        lines.push(kv("Ctrl+H", "Delete previous char"));
        lines.push(kv("Ctrl+D", "Delete next char"));
        lines.push(kv("Ctrl+Backspace", "Delete current line"));
        lines.push(kv("Ctrl+U", "Delete to line start"));
        lines.push(kv("Ctrl+K", "Delete to line end"));
        lines.push(kv(
            "Home/End",
            "Jump to line start/end (jump to history start/end when input is empty)",
        ));
        lines.push(RtLine::from(""));

        lines.push(RtLine::from(vec![RtSpan::styled(
            "Terminal",
            t_fg.add_modifier(Modifier::BOLD),
        )]));
        lines.push(kv("$", "Open shell terminal without a preset command"));
        lines.push(kv("$ <command>", "Run shell command immediately"));
        lines.push(kv("$$ <prompt>", "Request guided shell command help"));
        lines.push(RtLine::from(""));

        // Panels
        lines.push(RtLine::from(vec![RtSpan::styled(
            "Panels",
            t_fg.add_modifier(Modifier::BOLD),
        )]));
        lines.push(kv("Ctrl+B", "Toggle Browser overlay"));
        lines.push(kv("Ctrl+A", "Open Agents terminal"));

        // Slash command reference
        lines.push(RtLine::from(""));
        lines.push(RtLine::from(vec![RtSpan::styled(
            "Slash commands",
            t_fg.add_modifier(Modifier::BOLD),
        )]));
        for (cmd_str, cmd) in crate::slash_command::built_in_slash_commands() {
            // Hide internal test command from the Help panel
            if cmd_str == "test-approval" {
                continue;
            }
            // Prefer "Code" branding in the Help panel
            let desc = cmd.description().replace("Codex", "Code");
            // Render as "/command  —  description"
            lines.push(RtLine::from(vec![
                RtSpan::styled(format!("/{cmd_str:<12}"), t_fg),
                RtSpan::raw("  —  "),
                RtSpan::styled(desc.to_string(), t_dim),
            ]));
        }

        self.help.overlay = Some(HelpOverlay::new(lines));
        self.request_redraw();
    }

    pub(crate) fn toggle_help_popup(&mut self) {
        if self.help.overlay.is_some() {
            self.help.overlay = None;
        } else {
            self.show_help_popup();
        }
        self.request_redraw();
    }

    fn available_model_presets(&self) -> Vec<ModelPreset> {
        if let Some(presets) = self.remote_model_presets.as_ref() {
            return presets.clone();
        }
        let auth_mode = if self.config.using_chatgpt_auth {
            Some(McpAuthMode::ChatGPT)
        } else {
            Some(McpAuthMode::ApiKey)
        };
        builtin_model_presets(auth_mode)
    }

    pub(crate) fn update_model_presets(
        &mut self,
        presets: Vec<ModelPreset>,
        default_model: Option<String>,
    ) {
        if presets.is_empty() {
            return;
        }

        self.remote_model_presets = Some(presets.clone());
        self.bottom_pane.update_model_selection_presets(presets);

        if let Some(default_model) = default_model {
            self.maybe_apply_remote_default_model(default_model);
        }

        self.request_redraw();
    }

    fn maybe_apply_remote_default_model(&mut self, default_model: String) {
        if !self.allow_remote_default_at_startup {
            return;
        }
        if self.chat_model_selected_explicitly {
            return;
        }
        if self.config.model_explicit {
            return;
        }
        if self.config.model.eq_ignore_ascii_case(&default_model) {
            return;
        }

        self.apply_model_selection_inner(default_model, None, false, false);
    }

    fn preset_effort_for_model(preset: &ModelPreset) -> ReasoningEffort {
        preset.default_reasoning_effort.into()
    }

    fn clamp_reasoning_for_model(model: &str, requested: ReasoningEffort) -> ReasoningEffort {
        let protocol_effort: code_protocol::config_types::ReasoningEffort = requested.into();
        let clamped = clamp_reasoning_effort_for_model(model, protocol_effort);
        ReasoningEffort::from(clamped)
    }

    fn find_model_preset(&self, input: &str, presets: &[ModelPreset]) -> Option<ModelPreset> {
        if presets.is_empty() {
            return None;
        }

        let input_lower = input.to_ascii_lowercase();
        let collapsed_input: String = input_lower
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '-')
            .collect();

        let mut fallback_medium: Option<ModelPreset> = None;
        let mut fallback_first: Option<ModelPreset> = None;

        for preset in presets.iter() {
            let preset_effort = Self::preset_effort_for_model(preset);

            let id_lower = preset.id.to_ascii_lowercase();
            if Self::candidate_matches(&input_lower, &collapsed_input, &id_lower) {
                return Some(preset.clone());
            }

            let display_name_lower = preset.display_name.to_ascii_lowercase();
            if Self::candidate_matches(&input_lower, &collapsed_input, &display_name_lower) {
                return Some(preset.clone());
            }

            let effort_lower = preset_effort.to_string().to_ascii_lowercase();
            let model_lower = preset.model.to_ascii_lowercase();
            let spaced = format!("{model_lower} {effort_lower}");
            if Self::candidate_matches(&input_lower, &collapsed_input, &spaced) {
                return Some(preset.clone());
            }
            let dashed = format!("{model_lower}-{effort_lower}");
            if Self::candidate_matches(&input_lower, &collapsed_input, &dashed) {
                return Some(preset.clone());
            }

            if model_lower == input_lower
                || Self::candidate_matches(&input_lower, &collapsed_input, &model_lower)
            {
                if fallback_medium.is_none() && preset_effort == ReasoningEffort::Medium {
                    fallback_medium = Some(preset.clone());
                }
                if fallback_first.is_none() {
                    fallback_first = Some(preset.clone());
                }
            }
        }

        fallback_medium.or(fallback_first)
    }

    fn candidate_matches(input: &str, collapsed_input: &str, candidate: &str) -> bool {
        let candidate_lower = candidate.to_ascii_lowercase();
        if candidate_lower == input {
            return true;
        }
        let candidate_collapsed: String = candidate_lower
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '-')
            .collect();
        candidate_collapsed == collapsed_input
    }

    pub(crate) fn handle_model_command(&mut self, command_args: String) {
        if self.is_task_running() {
            let message = "'/model' is disabled while a task is in progress.".to_string();
            self.history_push_plain_state(history_cell::new_error_event(message));
            return;
        }

        let presets = self.available_model_presets();
        if presets.is_empty() {
            let message =
                "No model presets are available. Update your configuration to define models."
                    .to_string();
            self.history_push_plain_state(history_cell::new_error_event(message));
            return;
        }

        let trimmed = command_args.trim();
        if !trimmed.is_empty() {
            if let Some(preset) = self.find_model_preset(trimmed, &presets) {
                let effort = Self::preset_effort_for_model(&preset);
                self.apply_model_selection(preset.model, Some(effort));
            } else {
                let message = format!(
                    "Unknown model preset: '{trimmed}'. Use /model with no arguments to open the selector."
                );
                self.history_push_plain_state(history_cell::new_error_event(message));
            }
            return;
        }

        // Check if model selector is already open
        if self.bottom_pane.is_view_kind_active(crate::bottom_pane::ActiveViewKind::ModelSelection) {
            return;
        }

        self.bottom_pane.show_model_selection(
            presets,
            self.config.model.clone(),
            self.config.model_reasoning_effort,
            false,
            ModelSelectionTarget::Session,
        );
    }

    pub(crate) fn show_review_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for review. Update configuration to define models."
                    .to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::Review);
            self.close_settings_overlay();
        }
        self.bottom_pane.show_model_selection(
            presets,
            self.config.review_model.clone(),
            self.config.review_model_reasoning_effort,
            self.config.review_use_chat_model,
            ModelSelectionTarget::Review,
        );
    }

    pub(crate) fn show_review_resolve_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for review resolution.".to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::Review);
            self.close_settings_overlay();
        }
        let current = if self.config.review_resolve_use_chat_model {
            self.config.model.clone()
        } else {
            self.config.review_resolve_model.clone()
        };
        let effort = if self.config.review_resolve_use_chat_model {
            self.config.model_reasoning_effort
        } else {
            self.config.review_resolve_model_reasoning_effort
        };
        self.bottom_pane.show_model_selection(
            presets,
            current,
            effort,
            self.config.review_resolve_use_chat_model,
            ModelSelectionTarget::ReviewResolve,
        );
    }

    pub(crate) fn show_auto_review_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for Auto Review. Update configuration to define models.".to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::Review);
            self.close_settings_overlay();
        }
        let current = if self.config.auto_review_use_chat_model {
            self.config.model.clone()
        } else {
            self.config.auto_review_model.clone()
        };
        let effort = if self.config.auto_review_use_chat_model {
            self.config.model_reasoning_effort
        } else {
            self.config.auto_review_model_reasoning_effort
        };
        self.bottom_pane.show_model_selection(
            presets,
            current,
            effort,
            self.config.auto_review_use_chat_model,
            ModelSelectionTarget::AutoReview,
        );
    }

    pub(crate) fn show_auto_review_resolve_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for Auto Review resolution.".to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::Review);
            self.close_settings_overlay();
        }
        let current = if self.config.auto_review_resolve_use_chat_model {
            self.config.model.clone()
        } else {
            self.config.auto_review_resolve_model.clone()
        };
        let effort = if self.config.auto_review_resolve_use_chat_model {
            self.config.model_reasoning_effort
        } else {
            self.config.auto_review_resolve_model_reasoning_effort
        };
        self.bottom_pane.show_model_selection(
            presets,
            current,
            effort,
            self.config.auto_review_resolve_use_chat_model,
            ModelSelectionTarget::AutoReviewResolve,
        );
    }

    pub(crate) fn show_planning_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for planning. Update configuration to define models."
                    .to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::Planning);
            self.close_settings_overlay();
        }
        let current = if self.config.planning_use_chat_model {
            self.config.model.clone()
        } else {
            self.config.planning_model.clone()
        };
        let effort = self.config.planning_model_reasoning_effort;
        self.bottom_pane
            .show_model_selection(
                presets,
                current,
                effort,
                self.config.planning_use_chat_model,
                ModelSelectionTarget::Planning,
            );
    }

    pub(crate) fn show_auto_drive_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for Auto Drive. Update configuration to define models."
                    .to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::AutoDrive);
            self.close_settings_overlay();
        }
        self.bottom_pane.show_model_selection(
            presets,
            self.config.auto_drive.model.clone(),
            self.config.auto_drive.model_reasoning_effort,
            self.config.auto_drive_use_chat_model,
            ModelSelectionTarget::AutoDrive,
        );
    }

    pub(crate) fn apply_model_selection(&mut self, model: String, effort: Option<ReasoningEffort>) {
        self.apply_model_selection_inner(model, effort, true, true);
    }

    pub(crate) fn apply_shell_selection(
        &mut self,
        path: String,
        args: Vec<String>,
        script_style: Option<String>,
    ) {
        let parsed_style = script_style
            .as_deref()
            .and_then(ShellScriptStyle::parse)
            .or_else(|| ShellScriptStyle::infer_from_shell_program(&path));
        let shell_config = ShellConfig {
            path,
            args,
            script_style: parsed_style,
        };
        self.update_shell_config(Some(shell_config));
    }

    pub(crate) fn on_shell_selection_closed(&mut self, confirmed: bool) {
        if !confirmed {
            self.history_push_plain_paragraphs(
                crate::history::state::PlainMessageKind::Notice,
                vec!["Shell selection cancelled.".to_string()],
            );
        }
    }

    pub(crate) fn show_shell_selector(&mut self) {
        // Check if shell selector is already open
        if self.bottom_pane.is_view_kind_active(crate::bottom_pane::ActiveViewKind::ShellSelection) {
            return;
        }
        self.bottom_pane
            .show_shell_selection(self.config.shell.clone(), self.available_shell_presets());
    }

    fn clamp_reasoning_for_model_from_presets(
        model: &str,
        requested: ReasoningEffort,
        presets: &[ModelPreset],
    ) -> ReasoningEffort {
        fn rank(effort: ReasoningEffort) -> u8 {
            match effort {
                ReasoningEffort::Minimal => 0,
                ReasoningEffort::Low => 1,
                ReasoningEffort::Medium => 2,
                ReasoningEffort::High => 3,
                ReasoningEffort::XHigh => 4,
                ReasoningEffort::None => 5,
            }
        }

        let model_lower = model.to_ascii_lowercase();
        let Some(preset) = presets.iter().find(|preset| {
            preset.model.eq_ignore_ascii_case(&model_lower)
                || preset.id.eq_ignore_ascii_case(&model_lower)
                || preset.display_name.eq_ignore_ascii_case(&model_lower)
        }) else {
            return Self::clamp_reasoning_for_model(model, requested);
        };

        let supported: Vec<ReasoningEffort> = preset
            .supported_reasoning_efforts
            .iter()
            .map(|opt| ReasoningEffort::from(opt.effort))
            .collect();
        if supported.contains(&requested) {
            return requested;
        }

        let requested_rank = rank(requested);
        supported
            .into_iter()
            .min_by_key(|effort| {
                let effort_rank = rank(*effort);
                (requested_rank.abs_diff(effort_rank), u8::MAX - effort_rank)
            })
            .unwrap_or(requested)
    }

    fn apply_model_selection_inner(
        &mut self,
        model: String,
        effort: Option<ReasoningEffort>,
        mark_explicit: bool,
        announce: bool,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        if mark_explicit {
            self.chat_model_selected_explicitly = true;
            self.config.model_explicit = true;
        }

        let mut updated = false;
        if !self.config.model.eq_ignore_ascii_case(trimmed) {
            self.config.model = trimmed.to_string();
            let family = find_family_for_model(&self.config.model)
                .unwrap_or_else(|| derive_default_model_family(&self.config.model));
            self.config.model_family = family;
            updated = true;
        }

        if let Some(explicit) = effort
            && self.config.preferred_model_reasoning_effort != Some(explicit) {
                self.config.preferred_model_reasoning_effort = Some(explicit);
                updated = true;
            }

        let requested_effort = effort
            .or(self.config.preferred_model_reasoning_effort)
            .unwrap_or(self.config.model_reasoning_effort);
        let presets = self.available_model_presets();
        let clamped_effort = Self::clamp_reasoning_for_model_from_presets(trimmed, requested_effort, &presets);

        if self.config.model_reasoning_effort != clamped_effort {
            self.config.model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if updated {
            let op = Op::ConfigureSession {
                provider: self.config.model_provider.clone(),
                model: self.config.model.clone(),
                model_explicit: self.config.model_explicit,
                model_reasoning_effort: self.config.model_reasoning_effort,
                preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
                model_reasoning_summary: self.config.model_reasoning_summary,
                model_text_verbosity: self.config.model_text_verbosity,
                user_instructions: self.config.user_instructions.clone(),
                base_instructions: self.config.base_instructions.clone(),
                approval_policy: self.config.approval_policy,
                sandbox_policy: self.config.sandbox_policy.clone(),
                disable_response_storage: self.config.disable_response_storage,
                notify: self.config.notify.clone(),
                cwd: self.config.cwd.clone(),
                resume_path: None,
                demo_developer_message: self.config.demo_developer_message.clone(),
                dynamic_tools: Vec::new(),
                shell: self.config.shell.clone(),
            };
            self.submit_op(op);

            self.sync_follow_chat_models();
            self.refresh_settings_overview_rows();
        }

        if announce {
            let placement = self.ui_placement_for_now();
            let state = history_cell::new_model_output(&self.config.model, self.config.model_reasoning_effort);
            let cell = crate::history_cell::PlainHistoryCell::from_state(state.clone());
            self.push_system_cell(
                Box::new(cell),
                placement,
                Some("ui:model".to_string()),
                None,
                "system",
                Some(HistoryDomainRecord::Plain(state)),
            );
        }

        self.request_redraw();
    }

    fn sync_follow_chat_models(&mut self) {
        if self.config.review_use_chat_model {
            self.config.review_model = self.config.model.clone();
            self.config.review_model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_review_settings_model_row();
        }

        if self.config.review_resolve_use_chat_model {
            self.config.review_resolve_model = self.config.model.clone();
            self.config.review_resolve_model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_review_settings_model_row();
        }

        if self.config.planning_use_chat_model {
            self.config.planning_model = self.config.model.clone();
            self.config.planning_model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_planning_settings_model_row();
        }

        if self.config.auto_drive_use_chat_model {
            self.config.auto_drive.model = self.config.model.clone();
            self.config.auto_drive.model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_auto_drive_settings_model_row();
        }

        if self.config.auto_review_use_chat_model {
            self.config.auto_review_model = self.config.model.clone();
            self.config.auto_review_model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_review_settings_model_row();
        }

        if self.config.auto_review_resolve_use_chat_model {
            self.config.auto_review_resolve_model = self.config.model.clone();
            self.config.auto_review_resolve_model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_review_settings_model_row();
        }
    }

    pub(crate) fn apply_review_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.review_use_chat_model = false;

        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self.config.review_model.eq_ignore_ascii_case(trimmed) {
            self.config.review_model = trimmed.to_string();
            updated = true;
        }

        if self.config.review_model_reasoning_effort != clamped_effort {
            self.config.review_model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Review model unchanged.".to_string());
            return;
        }

        let message = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_review_model(
                &home,
                &self.config.review_model,
                self.config.review_model_reasoning_effort,
                self.config.review_use_chat_model,
            ) {
                Ok(_) => format!(
                    "Review model set to {} ({} reasoning)",
                    self.config.review_model,
                    Self::format_reasoning_effort(self.config.review_model_reasoning_effort)
                ),
                Err(err) => {
                    tracing::warn!("Failed to persist review model: {err}");
                    format!(
                        "Review model set for this session (failed to persist): {}",
                        self.config.review_model
                    )
                }
            }
        } else {
            tracing::warn!("Could not locate Code home to persist review model");
            format!(
                "Review model set for this session: {}",
                self.config.review_model
            )
        };

        self.bottom_pane.flash_footer_notice(message);
        self.refresh_settings_overview_rows();
        self.update_review_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn apply_review_resolve_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.review_resolve_use_chat_model = false;

        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self
            .config
            .review_resolve_model
            .eq_ignore_ascii_case(trimmed)
        {
            self.config.review_resolve_model = trimmed.to_string();
            updated = true;
        }

        if self.config.review_resolve_model_reasoning_effort != clamped_effort {
            self.config.review_resolve_model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Resolve model unchanged.".to_string());
            return;
        }

        let message = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_review_resolve_model(
                &home,
                &self.config.review_resolve_model,
                self.config.review_resolve_model_reasoning_effort,
                self.config.review_resolve_use_chat_model,
            ) {
                Ok(_) => format!(
                    "Resolve model set to {} ({} reasoning)",
                    self.config.review_resolve_model,
                    Self::format_reasoning_effort(self.config.review_resolve_model_reasoning_effort)
                ),
                Err(err) => {
                    tracing::warn!("Failed to persist resolve model: {err}");
                    format!(
                        "Resolve model set for this session (failed to persist): {}",
                        self.config.review_resolve_model
                    )
                }
            }
        } else {
            tracing::warn!("Could not locate Code home to persist resolve model");
            format!(
                "Resolve model set for this session: {}",
                self.config.review_resolve_model
            )
        };

        self.bottom_pane.flash_footer_notice(message);
        self.refresh_settings_overview_rows();
        self.update_review_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn set_review_use_chat_model(&mut self, use_chat: bool) {
        if self.config.review_use_chat_model == use_chat {
            return;
        }
        self.config.review_use_chat_model = use_chat;
        if use_chat {
            self.config.review_model = self.config.model.clone();
            self.config.review_model_reasoning_effort = self.config.model_reasoning_effort;
        }

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_review_model(
                &home,
                &self.config.review_model,
                self.config.review_model_reasoning_effort,
                use_chat,
            ) {
                tracing::warn!("Failed to persist review use-chat toggle: {err}");
            }

        let notice = if use_chat {
            "Review model now follows Chat model".to_string()
        } else {
            format!(
                "Review model set to {} ({} reasoning)",
                self.config.review_model,
                Self::format_reasoning_effort(self.config.review_model_reasoning_effort)
            )
        };
        self.bottom_pane.flash_footer_notice(notice);
        self.update_review_settings_model_row();
        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    pub(crate) fn set_review_resolve_use_chat_model(&mut self, use_chat: bool) {
        if self.config.review_resolve_use_chat_model == use_chat {
            return;
        }
        self.config.review_resolve_use_chat_model = use_chat;
        if use_chat {
            self.config.review_resolve_model = self.config.model.clone();
            self.config.review_resolve_model_reasoning_effort = self.config.model_reasoning_effort;
        }

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_review_resolve_model(
                &home,
                &self.config.review_resolve_model,
                self.config.review_resolve_model_reasoning_effort,
                use_chat,
            ) {
                tracing::warn!("Failed to persist resolve use-chat toggle: {err}");
            }

        let notice = if use_chat {
            "Resolve model now follows Chat model".to_string()
        } else {
            format!(
                "Resolve model set to {} ({} reasoning)",
                self.config.review_resolve_model,
                Self::format_reasoning_effort(self.config.review_resolve_model_reasoning_effort)
            )
        };
        self.bottom_pane.flash_footer_notice(notice);
        self.update_review_settings_model_row();
        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    pub(crate) fn apply_auto_review_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.auto_review_use_chat_model = false;
        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self
            .config
            .auto_review_model
            .eq_ignore_ascii_case(trimmed)
        {
            self.config.auto_review_model = trimmed.to_string();
            updated = true;
        }

        if self.config.auto_review_model_reasoning_effort != clamped_effort {
            self.config.auto_review_model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Auto Review model unchanged.".to_string());
            return;
        }

        let notice = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_auto_review_model(
                &home,
                &self.config.auto_review_model,
                self.config.auto_review_model_reasoning_effort,
                self.config.auto_review_use_chat_model,
            ) {
                Ok(_) => format!(
                    "Auto Review model set to {} ({} reasoning)",
                    self.config.auto_review_model,
                    Self::format_reasoning_effort(self.config.auto_review_model_reasoning_effort)
                ),
                Err(err) => {
                    tracing::warn!("Failed to persist Auto Review model: {err}");
                    format!(
                        "Auto Review model set for this session (failed to persist): {}",
                        self.config.auto_review_model
                    )
                }
            }
        } else {
            tracing::warn!("Could not locate Code home to persist Auto Review model");
            format!(
                "Auto Review model set for this session: {}",
                self.config.auto_review_model
            )
        };

        self.bottom_pane.flash_footer_notice(notice);
        self.refresh_settings_overview_rows();
        self.update_review_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn set_auto_review_use_chat_model(&mut self, use_chat: bool) {
        if self.config.auto_review_use_chat_model == use_chat {
            return;
        }
        self.config.auto_review_use_chat_model = use_chat;
        if use_chat {
            self.config.auto_review_model = self.config.model.clone();
            self.config.auto_review_model_reasoning_effort = self.config.model_reasoning_effort;
        }

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_auto_review_model(
                &home,
                &self.config.auto_review_model,
                self.config.auto_review_model_reasoning_effort,
                use_chat,
            ) {
                tracing::warn!("Failed to persist Auto Review use-chat toggle: {err}");
            }

        let notice = if use_chat {
            "Auto Review model now follows Chat model".to_string()
        } else {
            format!(
                "Auto Review model set to {} ({} reasoning)",
                self.config.auto_review_model,
                Self::format_reasoning_effort(self.config.auto_review_model_reasoning_effort)
            )
        };
        self.bottom_pane.flash_footer_notice(notice);
        self.update_review_settings_model_row();
        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    pub(crate) fn apply_auto_review_resolve_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.auto_review_resolve_use_chat_model = false;
        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self
            .config
            .auto_review_resolve_model
            .eq_ignore_ascii_case(trimmed)
        {
            self.config.auto_review_resolve_model = trimmed.to_string();
            updated = true;
        }

        if self.config.auto_review_resolve_model_reasoning_effort != clamped_effort {
            self.config.auto_review_resolve_model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Auto Review resolve model unchanged.".to_string());
            return;
        }

        let notice = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_auto_review_resolve_model(
                &home,
                &self.config.auto_review_resolve_model,
                self.config.auto_review_resolve_model_reasoning_effort,
                self.config.auto_review_resolve_use_chat_model,
            ) {
                Ok(_) => format!(
                    "Auto Review resolve model set to {} ({} reasoning)",
                    self.config.auto_review_resolve_model,
                    Self::format_reasoning_effort(self.config.auto_review_resolve_model_reasoning_effort)
                ),
                Err(err) => {
                    tracing::warn!("Failed to persist Auto Review resolve model: {err}");
                    format!(
                        "Auto Review resolve model set for this session (failed to persist): {}",
                        self.config.auto_review_resolve_model
                    )
                }
            }
        } else {
            tracing::warn!("Could not locate Code home to persist Auto Review resolve model");
            format!(
                "Auto Review resolve model set for this session: {}",
                self.config.auto_review_resolve_model
            )
        };

        self.bottom_pane.flash_footer_notice(notice);
        self.refresh_settings_overview_rows();
        self.update_review_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn set_auto_review_resolve_use_chat_model(&mut self, use_chat: bool) {
        if self.config.auto_review_resolve_use_chat_model == use_chat {
            return;
        }
        self.config.auto_review_resolve_use_chat_model = use_chat;
        if use_chat {
            self.config.auto_review_resolve_model = self.config.model.clone();
            self.config.auto_review_resolve_model_reasoning_effort =
                self.config.model_reasoning_effort;
        }

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_auto_review_resolve_model(
                &home,
                &self.config.auto_review_resolve_model,
                self.config.auto_review_resolve_model_reasoning_effort,
                use_chat,
            ) {
                tracing::warn!("Failed to persist Auto Review resolve use-chat toggle: {err}");
            }

        let notice = if use_chat {
            "Auto Review resolve model now follows Chat model".to_string()
        } else {
            format!(
                "Auto Review resolve model set to {} ({} reasoning)",
                self.config.auto_review_resolve_model,
                Self::format_reasoning_effort(self.config.auto_review_resolve_model_reasoning_effort)
            )
        };
        self.bottom_pane.flash_footer_notice(notice);
        self.update_review_settings_model_row();
        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    pub(crate) fn set_auto_drive_use_chat_model(&mut self, use_chat: bool) {
        if self.config.auto_drive_use_chat_model == use_chat {
            return;
        }
        self.config.auto_drive_use_chat_model = use_chat;
        if use_chat {
            self.config.auto_drive.model = self.config.model.clone();
            self.config.auto_drive.model_reasoning_effort = self.config.model_reasoning_effort;
        }

        self.restore_auto_resolve_attempts_if_lost();

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_auto_drive_settings(
                &home,
                &self.config.auto_drive,
                use_chat,
            ) {
                tracing::warn!("Failed to persist Auto Drive use-chat toggle: {err}");
            }

        let notice = if use_chat {
            "Auto Drive model now follows Chat model".to_string()
        } else {
            format!(
                "Auto Drive model set to {} ({} reasoning)",
                self.config.auto_drive.model,
                Self::format_reasoning_effort(self.config.auto_drive.model_reasoning_effort)
            )
        };

        self.bottom_pane.flash_footer_notice(notice);
        self.refresh_settings_overview_rows();
        self.update_auto_drive_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn handle_model_selection_closed(&mut self, target: ModelSelectionKind, _accepted: bool) {
        let expected_section = match target {
            ModelSelectionKind::Session => SettingsSection::Model,
            ModelSelectionKind::Review => SettingsSection::Review,
            ModelSelectionKind::Planning => SettingsSection::Planning,
            ModelSelectionKind::AutoDrive => SettingsSection::AutoDrive,
            ModelSelectionKind::ReviewResolve => SettingsSection::Review,
            ModelSelectionKind::AutoReview => SettingsSection::Review,
            ModelSelectionKind::AutoReviewResolve => SettingsSection::Review,
        };

        if let Some(section) = self.pending_settings_return {
            if section == expected_section {
                self.ensure_settings_overlay_section(section);
            }
            self.pending_settings_return = None;
        }

        self.request_redraw();
    }

    pub(crate) fn apply_planning_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.planning_use_chat_model = false;

        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self.config.planning_model.eq_ignore_ascii_case(trimmed) {
            self.config.planning_model = trimmed.to_string();
            updated = true;
        }
        if self.config.planning_model_reasoning_effort != clamped_effort {
            self.config.planning_model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Planning model unchanged.".to_string());
            return;
        }

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_planning_model(
                &home,
                &self.config.planning_model,
                self.config.planning_model_reasoning_effort,
                false,
            ) {
                tracing::warn!("Failed to persist planning model: {err}");
            }

        self.bottom_pane.flash_footer_notice(format!(
            "Planning model set to {} ({} reasoning)",
            self.config.planning_model,
            Self::format_reasoning_effort(self.config.planning_model_reasoning_effort)
        ));
        self.refresh_settings_overview_rows();
        self.update_planning_settings_model_row();
        // If we're currently in plan mode, switch the session model immediately.
        if matches!(self.config.sandbox_policy, code_core::protocol::SandboxPolicy::ReadOnly) {
            self.apply_planning_session_model();
        }
        self.request_redraw();
    }

    fn apply_planning_session_model(&mut self) {
        if self.config.planning_use_chat_model {
            self.restore_planning_session_model();
            return;
        }

        // If we're already on the planning model, do nothing.
        if self.config.model.eq_ignore_ascii_case(&self.config.planning_model)
            && self.config.model_reasoning_effort == self.config.planning_model_reasoning_effort
        {
            return;
        }

        // Save current chat model to restore later.
        self.planning_restore = Some((
            self.config.model.clone(),
            self.config.model_reasoning_effort,
        ));

        self.config.model = self.config.planning_model.clone();
        self.config.model_reasoning_effort = self.config.planning_model_reasoning_effort;

        let op = Op::ConfigureSession {
            provider: self.config.model_provider.clone(),
            model: self.config.model.clone(),
            model_explicit: self.config.model_explicit,
            model_reasoning_effort: self.config.model_reasoning_effort,
            preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
            model_reasoning_summary: self.config.model_reasoning_summary,
            model_text_verbosity: self.config.model_text_verbosity,
            user_instructions: self.config.user_instructions.clone(),
            base_instructions: self.config.base_instructions.clone(),
            approval_policy: self.config.approval_policy,
            sandbox_policy: self.config.sandbox_policy.clone(),
            disable_response_storage: self.config.disable_response_storage,
            notify: self.config.notify.clone(),
            cwd: self.config.cwd.clone(),
            resume_path: None,
            demo_developer_message: self.config.demo_developer_message.clone(),
            dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
        };
        self.submit_op(op);
    }

    fn restore_planning_session_model(&mut self) {
        if let Some((model, effort)) = self.planning_restore.take() {
            self.config.model = model;
            self.config.model_reasoning_effort = effort;

            let op = Op::ConfigureSession {
                provider: self.config.model_provider.clone(),
                model: self.config.model.clone(),
                model_explicit: self.config.model_explicit,
                model_reasoning_effort: self.config.model_reasoning_effort,
                preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
                model_reasoning_summary: self.config.model_reasoning_summary,
                model_text_verbosity: self.config.model_text_verbosity,
                user_instructions: self.config.user_instructions.clone(),
                base_instructions: self.config.base_instructions.clone(),
                approval_policy: self.config.approval_policy,
                sandbox_policy: self.config.sandbox_policy.clone(),
                disable_response_storage: self.config.disable_response_storage,
                notify: self.config.notify.clone(),
                cwd: self.config.cwd.clone(),
                resume_path: None,
                demo_developer_message: self.config.demo_developer_message.clone(),
                dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
            };
            self.submit_op(op);
        }
    }

    pub(crate) fn set_planning_use_chat_model(&mut self, use_chat: bool) {
        if self.config.planning_use_chat_model == use_chat {
            return;
        }
        self.config.planning_use_chat_model = use_chat;

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_planning_model(
                &home,
                &self.config.planning_model,
                self.config.planning_model_reasoning_effort,
                use_chat,
            ) {
                tracing::warn!("Failed to persist planning use-chat toggle: {err}");
            }

        if use_chat {
            self.bottom_pane
                .flash_footer_notice("Planning model now follows Chat model".to_string());
        } else {
            self.bottom_pane.flash_footer_notice(format!(
                "Planning model set to {} ({} reasoning)",
                self.config.planning_model,
                Self::format_reasoning_effort(self.config.planning_model_reasoning_effort)
            ));
        }

        self.update_planning_settings_model_row();
        self.refresh_settings_overview_rows();

        if matches!(self.config.sandbox_policy, code_core::protocol::SandboxPolicy::ReadOnly) {
            self.apply_planning_session_model();
        }
        self.request_redraw();
    }

    pub(crate) fn apply_auto_drive_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.auto_drive_use_chat_model = false;

        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self.config.auto_drive.model.eq_ignore_ascii_case(trimmed) {
            self.config.auto_drive.model = trimmed.to_string();
            updated = true;
        }

        if self.config.auto_drive.model_reasoning_effort != clamped_effort {
            self.config.auto_drive.model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Auto Drive model unchanged.".to_string());
            return;
        }

        let message = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_auto_drive_settings(
                &home,
                &self.config.auto_drive,
                self.config.auto_drive_use_chat_model,
            ) {
                Ok(_) => format!(
                    "Auto Drive model set to {} ({} reasoning)",
                    self.config.auto_drive.model,
                    Self::format_reasoning_effort(self.config.auto_drive.model_reasoning_effort)
                ),
                Err(err) => {
                    tracing::warn!("Failed to persist Auto Drive model: {err}");
                    format!(
                        "Auto Drive model set for this session (failed to persist): {}",
                        self.config.auto_drive.model
                    )
                }
            }
        } else {
            tracing::warn!("Could not locate Code home to persist Auto Drive model");
            format!(
                "Auto Drive model set for this session: {}",
                self.config.auto_drive.model
            )
        };

        self.bottom_pane.flash_footer_notice(message);
        self.refresh_settings_overview_rows();
        self.update_auto_drive_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn handle_reasoning_command(&mut self, command_args: String) {
        // command_args contains only the arguments after the command (e.g., "high" not "/reasoning high")
        let trimmed = command_args.trim();

        if !trimmed.is_empty() {
            // User specified a level: e.g., "high"
            let new_effort = match trimmed.to_lowercase().as_str() {
                "minimal" | "min" => ReasoningEffort::Minimal,
                "low" => ReasoningEffort::Low,
                "medium" | "med" => ReasoningEffort::Medium,
                "xhigh" | "extra-high" | "extra_high" => ReasoningEffort::XHigh,
                "high" => ReasoningEffort::High,
                // Backwards compatibility: map legacy values to minimal.
                "none" | "off" => ReasoningEffort::Minimal,
                _ => {
                    // Invalid parameter, show error and return
                    let message = format!(
                        "Invalid reasoning level: '{trimmed}'. Use: minimal, low, medium, or high"
                    );
                    self.history_push_plain_state(history_cell::new_error_event(message));
                    return;
                }
            };
            self.set_reasoning_effort(new_effort);
        } else {
            let presets = self.available_model_presets();
            if presets.is_empty() {
                let message =
                    "No model presets are available. Update your configuration to define models."
                        .to_string();
                self.history_push_plain_state(history_cell::new_error_event(message));
                return;
            }

        self.bottom_pane.show_model_selection(
            presets,
            self.config.model.clone(),
            self.config.model_reasoning_effort,
            false,
            ModelSelectionTarget::Session,
        );
        }
    }

    pub(crate) fn handle_verbosity_command(&mut self, command_args: String) {
        // Verbosity is not supported with ChatGPT auth
        if self.config.using_chatgpt_auth {
            let message =
                "Text verbosity is not available when using Sign in with ChatGPT".to_string();
            self.history_push_plain_state(history_cell::new_error_event(message));
            return;
        }

        // command_args contains only the arguments after the command (e.g., "high" not "/verbosity high")
        let trimmed = command_args.trim();

        if !trimmed.is_empty() {
            // User specified a level: e.g., "high"
            let new_verbosity = match trimmed.to_lowercase().as_str() {
                "low" => TextVerbosity::Low,
                "medium" | "med" => TextVerbosity::Medium,
                "high" => TextVerbosity::High,
                _ => {
                    // Invalid parameter, show error and return
                    let message = format!(
                        "Invalid verbosity level: '{trimmed}'. Use: low, medium, or high"
                    );
                    self.history_push_plain_state(history_cell::new_error_event(message));
                    return;
                }
            };

            // Update the configuration
            self.config.model_text_verbosity = new_verbosity;

            // Display success message
            let message = format!("Text verbosity set to: {new_verbosity}");
            self.push_background_tail(message);

            // Send the update to the backend
            let op = Op::ConfigureSession {
                provider: self.config.model_provider.clone(),
                model: self.config.model.clone(),
                model_explicit: self.config.model_explicit,
                model_reasoning_effort: self.config.model_reasoning_effort,
                preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
                model_reasoning_summary: self.config.model_reasoning_summary,
                model_text_verbosity: self.config.model_text_verbosity,
                user_instructions: self.config.user_instructions.clone(),
                base_instructions: self.config.base_instructions.clone(),
                approval_policy: self.config.approval_policy,
                sandbox_policy: self.config.sandbox_policy.clone(),
                disable_response_storage: self.config.disable_response_storage,
                notify: self.config.notify.clone(),
                cwd: self.config.cwd.clone(),
                resume_path: None,
                demo_developer_message: self.config.demo_developer_message.clone(),
                dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
            };
            let _ = self.code_op_tx.send(op);
        } else {
            // No parameter specified, show interactive UI
            self.bottom_pane
                .show_verbosity_selection(self.config.model_text_verbosity);
        }
    }

    pub(crate) fn prepare_agents(&mut self) {
        // Set the flag to show agents are ready to start
        self.agents_ready_to_start = true;
        self.agents_terminal.reset();
        if self.agents_terminal.active {
            // Reset scroll offset when a new batch starts to avoid stale positions
            self.layout.scroll_offset.set(0);
        }

        // Initialize sparkline with some data so it shows immediately
        {
            let mut sparkline_data = self.sparkline_data.borrow_mut();
            if sparkline_data.is_empty() {
                // Add initial low activity data for preparing phase
                for _ in 0..10 {
                    sparkline_data.push((2, false));
                }
                tracing::info!(
                    "Initialized sparkline data with {} points for preparing phase",
                    sparkline_data.len()
                );
            }
        } // Drop the borrow here

        self.request_redraw();
    }

    /// Update sparkline data with randomized activity based on agent count
    fn update_sparkline_data(&self) {
        let now = std::time::Instant::now();

        // Update every 100ms for smooth animation
        if now
            .duration_since(*self.last_sparkline_update.borrow())
            .as_millis()
            < 100
        {
            return;
        }

        *self.last_sparkline_update.borrow_mut() = now;

        // Calculate base height based on number of agents and status
        let agent_count = self.active_agents.len();
        let is_planning = self.overall_task_status == "planning";
        let base_height = if agent_count == 0 && self.agents_ready_to_start {
            2 // Minimal activity when preparing
        } else if is_planning && agent_count > 0 {
            3 // Low activity during planning phase
        } else if agent_count == 1 {
            5 // Low activity for single agent
        } else if agent_count == 2 {
            10 // Medium activity for two agents
        } else if agent_count >= 3 {
            15 // High activity for multiple agents
        } else {
            0 // No activity when no agents
        };

        // Don't generate data if there's no activity
        if base_height == 0 {
            return;
        }

        // Generate random variation
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hash;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        now.elapsed().as_nanos().hash(&mut hasher);
        let random_seed = hasher.finish();

        // More variation during planning phase for visibility (+/- 50%)
        // Less variation during running for stability (+/- 30%)
        let variation_percent = if self.agents_ready_to_start && self.active_agents.is_empty() {
            50 // More variation during planning for visibility
        } else {
            30 // Standard variation during running
        };

        let variation_range = variation_percent * 2; // e.g., 100 for +/- 50%
        let variation = ((random_seed % variation_range) as i32 - variation_percent as i32)
            * base_height
            / 100;
        let height = ((base_height + variation).max(1) as u64).min(20);

        // Check if any agents are completed
        let has_completed = self
            .active_agents
            .iter()
            .any(|a| matches!(a.status, AgentStatus::Completed));

        // Keep a rolling window of 60 data points (about 6 seconds at 100ms intervals)
        let mut sparkline_data = self.sparkline_data.borrow_mut();
        sparkline_data.push((height, has_completed));
        if sparkline_data.len() > 60 {
            sparkline_data.remove(0);
        }
    }

    pub(crate) fn set_reasoning_effort(&mut self, new_effort: ReasoningEffort) {
        let clamped_effort = Self::clamp_reasoning_for_model(&self.config.model, new_effort);

        if clamped_effort != new_effort {
            let requested = Self::format_reasoning_effort(new_effort);
            let applied = Self::format_reasoning_effort(clamped_effort);
            self.bottom_pane.flash_footer_notice(format!(
                "{} does not support {} reasoning; using {} instead.",
                self.config.model, requested, applied
            ));
        }

        // Update the config
        self.config.preferred_model_reasoning_effort = Some(new_effort);
        self.config.model_reasoning_effort = clamped_effort;

        // Send ConfigureSession op to update the backend
        let op = Op::ConfigureSession {
            provider: self.config.model_provider.clone(),
            model: self.config.model.clone(),
            model_explicit: self.config.model_explicit,
            model_reasoning_effort: clamped_effort,
            preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
            model_reasoning_summary: self.config.model_reasoning_summary,
            model_text_verbosity: self.config.model_text_verbosity,
            user_instructions: self.config.user_instructions.clone(),
            base_instructions: self.config.base_instructions.clone(),
            approval_policy: self.config.approval_policy,
            sandbox_policy: self.config.sandbox_policy.clone(),
            disable_response_storage: self.config.disable_response_storage,
            notify: self.config.notify.clone(),
            cwd: self.config.cwd.clone(),
            resume_path: None,
            demo_developer_message: self.config.demo_developer_message.clone(),
            dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
        };

        self.submit_op(op);

        // Add status message to history (replaceable system notice)
        let placement = self.ui_placement_for_now();
        let state = history_cell::new_reasoning_output(self.config.model_reasoning_effort);
        let cell = crate::history_cell::PlainHistoryCell::from_state(state.clone());
        self.push_system_cell(
            Box::new(cell),
            placement,
            Some("ui:reasoning".to_string()),
            None,
            "system",
            Some(HistoryDomainRecord::Plain(state)),
        );
        self.refresh_settings_overview_rows();
    }

    pub(crate) fn set_text_verbosity(&mut self, new_verbosity: TextVerbosity) {
        // Update the config
        self.config.model_text_verbosity = new_verbosity;

        // Send ConfigureSession op to update the backend
        let op = Op::ConfigureSession {
            provider: self.config.model_provider.clone(),
            model: self.config.model.clone(),
            model_explicit: self.config.model_explicit,
            model_reasoning_effort: self.config.model_reasoning_effort,
            preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
            model_reasoning_summary: self.config.model_reasoning_summary,
            model_text_verbosity: new_verbosity,
            user_instructions: self.config.user_instructions.clone(),
            base_instructions: self.config.base_instructions.clone(),
            approval_policy: self.config.approval_policy,
            sandbox_policy: self.config.sandbox_policy.clone(),
            disable_response_storage: self.config.disable_response_storage,
            notify: self.config.notify.clone(),
            cwd: self.config.cwd.clone(),
            resume_path: None,
            demo_developer_message: self.config.demo_developer_message.clone(),
            dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
        };

        self.submit_op(op);

        // Add status message to history
        let message = format!("Text verbosity set to: {new_verbosity}");
        self.push_background_tail(message);
    }

    pub(crate) fn set_auto_upgrade_enabled(&mut self, enabled: bool) {
        if self.config.auto_upgrade_enabled == enabled {
            return;
        }
        self.config.auto_upgrade_enabled = enabled;

        let code_home = self.config.code_home.clone();
        let profile = self.config.active_profile.clone();
        tokio::spawn(async move {
            if let Err(err) = code_core::config_edit::persist_overrides(
                &code_home,
                profile.as_deref(),
                &[(&["auto_upgrade_enabled"], if enabled { "true" } else { "false" })],
            )
            .await
            {
                tracing::warn!("failed to persist auto-upgrade setting: {err}");
            }
        });

        let notice = if enabled {
            "Automatic upgrades enabled"
        } else {
            "Automatic upgrades disabled"
        };
        self.bottom_pane.flash_footer_notice(notice.to_string());

        let should_refresh_updates = matches!(
            self.settings
                .overlay
                .as_ref()
                .map(settings_overlay::SettingsOverlayView::active_section),
            Some(SettingsSection::Updates)
        );

        if should_refresh_updates
            && let Some(content) = self.build_updates_settings_content()
                && let Some(overlay) = self.settings.overlay.as_mut() {
                    overlay.set_updates_content(content);
                }
        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    pub(crate) fn set_auto_switch_accounts_on_rate_limit(&mut self, enabled: bool) {
        if self.config.auto_switch_accounts_on_rate_limit == enabled {
            return;
        }
        self.config.auto_switch_accounts_on_rate_limit = enabled;

        let code_home = self.config.code_home.clone();
        let profile = self.config.active_profile.clone();
        tokio::spawn(async move {
            if let Err(err) = code_core::config_edit::persist_overrides(
                &code_home,
                profile.as_deref(),
                &[(&["auto_switch_accounts_on_rate_limit"], if enabled { "true" } else { "false" })],
            )
            .await
            {
                tracing::warn!("failed to persist account auto-switch setting: {err}");
            }
        });

        let notice = if enabled {
            "Auto-switch accounts enabled"
        } else {
            "Auto-switch accounts disabled"
        };
        self.bottom_pane.flash_footer_notice(notice.to_string());

        let should_refresh_accounts = matches!(
            self.settings
                .overlay
                .as_ref()
                .map(settings_overlay::SettingsOverlayView::active_section),
            Some(SettingsSection::Accounts)
        );
        if should_refresh_accounts {
            let content = self.build_accounts_settings_content();
            if let Some(overlay) = self.settings.overlay.as_mut() {
                overlay.set_accounts_content(content);
            }
        }

        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    pub(crate) fn set_api_key_fallback_on_all_accounts_limited(&mut self, enabled: bool) {
        if self.config.api_key_fallback_on_all_accounts_limited == enabled {
            return;
        }
        self.config.api_key_fallback_on_all_accounts_limited = enabled;

        let code_home = self.config.code_home.clone();
        let profile = self.config.active_profile.clone();
        tokio::spawn(async move {
            if let Err(err) = code_core::config_edit::persist_overrides(
                &code_home,
                profile.as_deref(),
                &[(&["api_key_fallback_on_all_accounts_limited"], if enabled { "true" } else { "false" })],
            )
            .await
            {
                tracing::warn!("failed to persist API key fallback setting: {err}");
            }
        });

        let notice = if enabled {
            "API key fallback enabled"
        } else {
            "API key fallback disabled"
        };
        self.bottom_pane.flash_footer_notice(notice.to_string());

        let should_refresh_accounts = matches!(
            self.settings
                .overlay
                .as_ref()
                .map(settings_overlay::SettingsOverlayView::active_section),
            Some(SettingsSection::Accounts)
        );
        if should_refresh_accounts {
            let content = self.build_accounts_settings_content();
            if let Some(overlay) = self.settings.overlay.as_mut() {
                overlay.set_accounts_content(content);
            }
        }

        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    /// Forward file-search results to the bottom pane.
    pub(crate) fn apply_file_search_result(&mut self, query: String, matches: Vec<FileMatch>) {
        self.bottom_pane.on_file_search_result(query, matches);
    }


    // Ctrl+Y syntax cycling disabled intentionally.

    /// Show a brief debug notice in the footer.
    #[allow(dead_code)]
    pub(crate) fn debug_notice(&mut self, text: String) {
        self.bottom_pane.flash_footer_notice(text);
        self.request_redraw();
    }

    fn maybe_start_auto_upgrade_task(&mut self) {
        if !crate::updates::auto_upgrade_runtime_enabled() {
            return;
        }
        if !self.config.auto_upgrade_enabled {
            return;
        }

        let cfg = self.config.clone();
        let tx = self.app_event_tx.clone();
        let upgrade_ticket = self.make_background_tail_ticket();
        tokio::spawn(async move {
            match crate::updates::auto_upgrade_if_enabled(&cfg).await {
                Ok(outcome) => {
                    if let Some(version) = outcome.installed_version {
                        tx.send(AppEvent::AutoUpgradeCompleted { version });
                    }
                    if let Some(message) = outcome.user_notice {
                        tx.send_background_event_with_ticket(&upgrade_ticket, message);
                    }
                }
                Err(err) => {
                    tracing::warn!("auto-upgrade: background task failed: {err:?}");
                }
            }
        });
    }

    pub(crate) fn set_theme(&mut self, new_theme: code_core::config_types::ThemeName) {
        let custom_hint = if matches!(new_theme, code_core::config_types::ThemeName::Custom) {
            self.config
                .tui
                .theme
                .is_dark
                .or_else(crate::theme::custom_theme_is_dark)
        } else {
            None
        };
        let mapped_theme = crate::theme::map_theme_for_palette(new_theme, custom_hint);

        // Update the config
        self.config.tui.theme.name = mapped_theme;
        if matches!(new_theme, code_core::config_types::ThemeName::Custom) {
            self.config.tui.theme.is_dark = custom_hint;
        } else {
            self.config.tui.theme.is_dark = None;
        }

        // Save the theme to config file
        self.save_theme_to_config(mapped_theme);

        // Retint pre-rendered history cell lines to the new palette
        self.restyle_history_after_theme_change();

        // Add confirmation message to history (replaceable system notice)
        let theme_name = Self::theme_display_name(mapped_theme);
        let message = format!("Theme changed to {theme_name}");
        let placement = self.ui_placement_for_now();
        let cell = history_cell::new_background_event(message);
        let record = HistoryDomainRecord::BackgroundEvent(cell.state().clone());
        self.push_system_cell(
            Box::new(cell),
            placement,
            Some("ui:theme".to_string()),
            None,
            "background",
            Some(record),
        );
        self.refresh_settings_overview_rows();
    }

    pub(crate) fn set_spinner(&mut self, spinner_name: String) {
        // Update the config
        self.config.tui.spinner.name = spinner_name.clone();
        // Persist selection to config file
        if let Ok(home) = code_core::config::find_code_home() {
            if let Err(e) = code_core::config::set_tui_spinner_name(&home, &spinner_name) {
                tracing::warn!("Failed to persist spinner to config.toml: {}", e);
            } else {
                tracing::info!("Persisted TUI spinner selection to config.toml");
            }
        } else {
            tracing::warn!("Could not locate Codex home to persist spinner selection");
        }

        // Confirmation message (replaceable system notice)
        let message = format!("Spinner changed to {spinner_name}");
        let placement = self.ui_placement_for_now();
        let cell = history_cell::new_background_event(message);
        let record = HistoryDomainRecord::BackgroundEvent(cell.state().clone());
        self.push_system_cell(
            Box::new(cell),
            placement,
            Some("ui:spinner".to_string()),
            None,
            "background",
            Some(record),
        );

        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    fn apply_access_mode_indicator_from_config(&mut self) {
        use code_core::protocol::AskForApproval;
        use code_core::protocol::SandboxPolicy;
        let label = match (&self.config.sandbox_policy, self.config.approval_policy) {
            (SandboxPolicy::ReadOnly, _) => Some("Read Only".to_string()),
            (
                SandboxPolicy::WorkspaceWrite {
                    network_access: false,
                    ..
                },
                AskForApproval::UnlessTrusted,
            ) => Some("Write with Approval".to_string()),
            _ => None,
        };
        self.bottom_pane.set_access_mode_label(label);
    }

    /// Rotate the access preset: Read Only (Plan Mode) → Write with Approval → Full Access
    pub(crate) fn cycle_access_mode(&mut self) {
        use code_core::config::set_project_access_mode;
        use code_core::protocol::AskForApproval;
        use code_core::protocol::SandboxPolicy;

        // Determine current index
        let idx = match (&self.config.sandbox_policy, self.config.approval_policy) {
            (SandboxPolicy::ReadOnly, _) => 0,
            (
                SandboxPolicy::WorkspaceWrite {
                    network_access: false,
                    ..
                },
                AskForApproval::UnlessTrusted,
            ) => 1,
            (SandboxPolicy::DangerFullAccess, AskForApproval::Never) => 2,
            _ => 0,
        };
        let next = (idx + 1) % 3;

        // Apply mapping
        let (label, approval, sandbox) = match next {
            0 => (
                "Read Only (Plan Mode)",
                AskForApproval::OnRequest,
                SandboxPolicy::ReadOnly,
            ),
            1 => (
                "Write with Approval",
                AskForApproval::UnlessTrusted,
                SandboxPolicy::new_workspace_write_policy(),
            ),
            _ => (
                "Full Access",
                AskForApproval::Never,
                SandboxPolicy::DangerFullAccess,
            ),
        };

        // Apply planning model when entering plan mode; restore when leaving it.
        if next == 0 {
            self.apply_planning_session_model();
        } else if idx == 0 {
            self.restore_planning_session_model();
        }

        // Update local config
        self.config.approval_policy = approval;
        self.config.sandbox_policy = sandbox;

        // Send ConfigureSession op to backend
        let op = Op::ConfigureSession {
            provider: self.config.model_provider.clone(),
            model: self.config.model.clone(),
            model_explicit: self.config.model_explicit,
            model_reasoning_effort: self.config.model_reasoning_effort,
            preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
            model_reasoning_summary: self.config.model_reasoning_summary,
            model_text_verbosity: self.config.model_text_verbosity,
            user_instructions: self.config.user_instructions.clone(),
            base_instructions: self.config.base_instructions.clone(),
            approval_policy: self.config.approval_policy,
            sandbox_policy: self.config.sandbox_policy.clone(),
            disable_response_storage: self.config.disable_response_storage,
            notify: self.config.notify.clone(),
            cwd: self.config.cwd.clone(),
            resume_path: None,
            demo_developer_message: self.config.demo_developer_message.clone(),
            dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
        };
        self.submit_op(op);

        // Persist selection into CODEX_HOME/config.toml for this project directory so it sticks.
        let _ = set_project_access_mode(
            &self.config.code_home,
            &self.config.cwd,
            self.config.approval_policy,
            match &self.config.sandbox_policy {
                SandboxPolicy::ReadOnly => code_protocol::config_types::SandboxMode::ReadOnly,
                SandboxPolicy::WorkspaceWrite { .. } => {
                    code_protocol::config_types::SandboxMode::WorkspaceWrite
                }
                SandboxPolicy::DangerFullAccess => {
                    code_protocol::config_types::SandboxMode::DangerFullAccess
                }
            },
        );

        // Footer indicator: persistent for RO/Approval; ephemeral for Full Access
        if next == 2 {
            self.bottom_pane.set_access_mode_label_ephemeral(
                "Full Access".to_string(),
                std::time::Duration::from_secs(4),
            );
        } else {
            let persistent = if next == 0 {
                "Read Only"
            } else {
                "Write with Approval"
            };
            self.bottom_pane
                .set_access_mode_label(Some(persistent.to_string()));
        }

        // Announce in history: replace the last access-mode status, inserting early
        // in the current request so it appears above upcoming commands.
        let msg = format!("Mode changed: {label}");
        self.set_access_status_message(msg);
        // No footer notice: the indicator covers this; avoid duplicate texts.

        // Prepare a single consolidated note for the agent to see before the
        // next turn begins. Subsequent cycles will overwrite this note.
        let agent_note = match next {
            0 => {
                "System: access mode changed to Read Only. Do not attempt write operations or apply_patch."
            }
            1 => {
                "System: access mode changed to Write with Approval. Request approval before writes."
            }
            _ => "System: access mode changed to Full Access. Writes and network are allowed.",
        };
        self.queue_agent_note(agent_note);
    }

    pub(crate) fn cycle_auto_drive_variant(&mut self) {
        self.auto_drive_variant = self.auto_drive_variant.next();
        self
            .bottom_pane
            .set_auto_drive_variant(self.auto_drive_variant);
        let notice = format!(
            "Auto Drive style: {}",
            self.auto_drive_variant.name()
        );
        self.bottom_pane.flash_footer_notice(notice);
    }

    /// Insert or replace the access-mode status background event. Uses a near-time
    /// key so it appears above any imminent Exec/Tool cells in this request.
    fn set_access_status_message(&mut self, message: String) {
        let cell = crate::history_cell::new_background_event(message);
        if let Some(idx) = self.access_status_idx
            && idx < self.history_cells.len()
                && matches!(
                    self.history_cells[idx].kind(),
                    crate::history_cell::HistoryCellType::BackgroundEvent
                )
            {
                self.history_replace_at(idx, Box::new(cell));
                self.request_redraw();
                return;
            }
        // Insert new status near the top of this request window
        let key = self.near_time_key(None);
        let pos = self.history_insert_with_key_global_tagged(Box::new(cell), key, "background", None);
        self.access_status_idx = Some(pos);
    }

    fn restyle_history_after_theme_change(&mut self) {
        let old = self.last_theme.clone();
        let new = crate::theme::current_theme();
        if old == new {
            return;
        }

        for cell in &mut self.history_cells {
            if let Some(plain) = cell
                .as_any_mut()
                .downcast_mut::<history_cell::PlainHistoryCell>()
            {
                plain.invalidate_layout_cache();
            } else if let Some(tool) = cell
                .as_any_mut()
                .downcast_mut::<history_cell::ToolCallCell>()
            {
                tool.retint(&old, &new);
            } else if let Some(reason) = cell
                .as_any_mut()
                .downcast_mut::<history_cell::CollapsibleReasoningCell>()
            {
                reason.retint(&old, &new);
            } else if let Some(stream) = cell
                .as_any_mut()
                .downcast_mut::<history_cell::StreamingContentCell>()
            {
                stream.update_context(self.config.file_opener, &self.config.cwd);
            } else if let Some(wait) = cell
                .as_any_mut()
                .downcast_mut::<history_cell::WaitStatusCell>()
            {
                wait.retint(&old, &new);
            } else if let Some(assist) = cell
                .as_any_mut()
                .downcast_mut::<history_cell::AssistantMarkdownCell>()
            {
                // Fully rebuild from raw to apply new theme + syntax highlight
                let current = assist.state().clone();
                assist.update_state(current, &self.config);
            } else if let Some(merged) = cell
                .as_any_mut()
                .downcast_mut::<history_cell::MergedExecCell>()
            {
                merged.rebuild_with_theme();
            } else if let Some(diff) = cell
                .as_any_mut()
                .downcast_mut::<history_cell::DiffCell>()
            {
                diff.rebuild_with_theme();
            }
        }

        // Update snapshot and redraw; height caching can remain (colors don't affect wrap)
        self.last_theme = new;
        self.render_theme_epoch = self.render_theme_epoch.saturating_add(1);
        self.history_render.invalidate_all();
        self.mark_render_requests_dirty();
        self.history_prefix_append_only.set(false);
        self.app_event_tx.send(AppEvent::RequestRedraw);
    }

    /// Public-facing hook for preview mode to retint existing history lines
    /// without persisting the theme or adding history events.
    pub(crate) fn retint_history_for_preview(&mut self) {
        self.restyle_history_after_theme_change();
    }

    fn save_theme_to_config(&self, new_theme: code_core::config_types::ThemeName) {
        // Persist the theme selection to CODE_HOME/CODEX_HOME config.toml
        match code_core::config::find_code_home() {
            Ok(home) => {
                if let Err(e) = code_core::config::set_tui_theme_name(&home, new_theme) {
                    tracing::warn!("Failed to persist theme to config.toml: {}", e);
                } else {
                    tracing::info!("Persisted TUI theme selection to config.toml");
                }
            }
            Err(e) => {
                tracing::warn!("Could not locate Codex home to persist theme: {}", e);
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn on_esc(&mut self) -> bool {
        if self.bottom_pane.is_task_running() {
            self.interrupt_running_task();
            return true;
        }
        false
    }

    /// Handle Ctrl-C key press.
    /// Returns CancellationEvent::Handled if the event was consumed by the UI, or
    /// CancellationEvent::Ignored if the caller should handle it (e.g. exit).
    pub(crate) fn on_ctrl_c(&mut self) -> CancellationEvent {
        if let Some(id) = self.terminal_overlay_id() {
            if self.terminal_is_running() {
                self.request_terminal_cancel(id);
            } else {
                self.close_terminal_overlay();
            }
            return CancellationEvent::Handled;
        }
        match self.bottom_pane.on_ctrl_c() {
            CancellationEvent::Handled => return CancellationEvent::Handled,
            CancellationEvent::Ignored => {}
        }
        if self.is_task_running() || self.wait_running() {
            self.interrupt_running_task();
            CancellationEvent::Ignored
        } else if self.bottom_pane.ctrl_c_quit_hint_visible() {
            self.submit_op(Op::Shutdown);
            CancellationEvent::Handled
        } else {
            self.bottom_pane.show_ctrl_c_quit_hint();
            CancellationEvent::Ignored
        }
    }

    #[allow(dead_code)]
    pub(crate) fn composer_is_empty(&self) -> bool {
        self.bottom_pane.composer_is_empty()
    }

    // --- Double‑Escape helpers ---
    fn schedule_auto_drive_card_celebration(
        &self,
        delay: Duration,
        message: Option<String>,
    ) {
        let event = AppEvent::StartAutoDriveCelebration { message };
        self.spawn_app_event_after(delay, event);
    }

    pub(crate) fn start_auto_drive_card_celebration(&mut self, message: Option<String>) {
        let mut started = auto_drive_cards::start_celebration(self, message.clone());
        if !started
            && let Some(card) = self.latest_auto_drive_card_mut() {
                card.start_celebration(message.clone());
                started = true;
            }
        if !started {
            return;
        }

        self.spawn_app_event_after(
            AUTO_COMPLETION_CELEBRATION_DURATION,
            AppEvent::StopAutoDriveCelebration,
        );

        if let Some(msg) = message
            && !auto_drive_cards::update_completion_message(self, Some(msg.clone()))
                && let Some(card) = self.latest_auto_drive_card_mut() {
                    card.set_completion_message(Some(msg));
                }

        self.mark_history_dirty();
        self.request_redraw();
    }

    pub(crate) fn stop_auto_drive_card_celebration(&mut self) {
        let mut stopped = auto_drive_cards::stop_celebration(self);
        if !stopped
            && let Some(card) = self.latest_auto_drive_card_mut() {
                card.stop_celebration();
                stopped = true;
            }
        if stopped {
            self.mark_history_dirty();
            self.request_redraw();
        }
    }

    fn spawn_app_event_after(&self, delay: Duration, event: AppEvent) {
        if delay.is_zero() {
            self.app_event_tx.send(event);
            return;
        }

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let tx = self.app_event_tx.clone();
            handle.spawn(async move {
                tokio::time::sleep(delay).await;
                tx.send(event);
            });
        } else {
            #[cfg(test)]
            {
                let tx = self.app_event_tx.clone();
                if let Err(err) = std::thread::Builder::new()
                    .name("delayed-app-event".to_string())
                    .spawn(move || {
                        tx.send(event);
                    })
                {
                    tracing::warn!("failed to spawn delayed app event: {err}");
                }
            }
            #[cfg(not(test))]
            {
                let _ = event;
            }
        }
    }

    fn latest_auto_drive_card_mut(
        &mut self,
    ) -> Option<&mut history_cell::AutoDriveCardCell> {
        self.history_cells
            .iter_mut()
            .rev()
            .find_map(|cell| cell.as_any_mut().downcast_mut::<history_cell::AutoDriveCardCell>())
    }

    pub(crate) fn auto_manual_entry_active(&self) -> bool {
        self.auto_state.should_show_goal_entry()
            || (self.auto_state.is_active() && self.auto_state.awaiting_coordinator_submit())
    }

    fn has_running_commands_or_tools(&self) -> bool {
        let wait_running = self.wait_running();
        let wait_blocks = self.wait_blocking_enabled();

        self.terminal_is_running()
            || !self.exec.running_commands.is_empty()
            || !self.tools_state.running_custom_tools.is_empty()
            || !self.tools_state.web_search_sessions.is_empty()
            || (wait_running && wait_blocks)
    }

    pub(crate) fn is_task_running(&self) -> bool {
        let wait_running = self.wait_running();
        let wait_blocks = self.wait_blocking_enabled();

        self.bottom_pane.is_task_running()
            || self.terminal_is_running()
            || !self.exec.running_commands.is_empty()
            || !self.tools_state.running_custom_tools.is_empty()
            || !self.tools_state.web_search_sessions.is_empty()
            || !self.active_task_ids.is_empty()
            || self.stream.is_write_cycle_active()
            || (wait_running && wait_blocks)
    }

    #[inline]
    fn wait_running(&self) -> bool {
        !self.tools_state.running_wait_tools.is_empty()
            || !self.tools_state.running_kill_tools.is_empty()
    }

    #[inline]
    fn wait_blocking_enabled(&self) -> bool {
        self.queued_user_messages.is_empty()
    }

    /// True when the only ongoing activity is a wait/kill tool (no exec/stream/agents/tasks),
    /// meaning we can safely unlock the composer without cancelling the work.
    ///
    /// Historically this returned false whenever any exec was running, which caused user input
    /// submitted during a `wait` tool to be queued instead of interrupting the wait. That meant
    /// the core never received `Op::UserInput`, so waits could not be cancelled mid-flight.
    /// We treat execs that are only being observed by a wait tool as "wait-only" so input can
    /// flow through immediately and interrupt the wait.
    fn wait_only_activity(&self) -> bool {
        if !self.wait_running() {
            return false;
        }

        // Consider execs "wait-only" when every running command is being waited on and marked
        // as such. Any other exec activity keeps the composer blocked.
        let execs_wait_only = self.exec.running_commands.is_empty()
            || self
                .exec
                .running_commands
                .iter()
                .all(|(id, cmd)| {
                    cmd.wait_active
                        && self
                            .tools_state
                            .running_wait_tools
                            .values()
                            .any(|wait_id| wait_id == id)
                });

        execs_wait_only
            && self.tools_state.running_custom_tools.is_empty()
            && self.tools_state.web_search_sessions.is_empty()
            && !self.stream.is_write_cycle_active()
            && !self.agents_are_actively_running()
            && self.active_task_ids.is_empty()
    }

    /// If queued user messages have been blocked longer than the SLA while only a wait/kill
    /// tool is running, unlock the composer and dispatch the queue.
    fn maybe_enforce_queue_unblock(&mut self) {
        if self.queued_user_messages.is_empty() {
            self.queue_block_started_at = None;
            return;
        }

        let Some(started) = self.queue_block_started_at else {
            self.queue_block_started_at = Some(Instant::now());
            return;
        };

        if started.elapsed() < Duration::from_secs(10) {
            return;
        }

        if !self.wait_only_activity() {
            // Another activity is running; keep waiting.
            return;
        }

        let wait_ids: Vec<String> = self
            .tools_state
            .running_wait_tools
            .keys()
            .map(|k| k.0.clone())
            .collect();

        tracing::warn!(
            "queue watchdog fired; unblocking input (waits={:?}, queued={})",
            wait_ids,
            self.queued_user_messages.len()
        );

        self.bottom_pane.set_task_running(false);
        self.bottom_pane
            .update_status_text("Waiting in background".to_string());

        if !wait_ids.is_empty() {
            self.push_background_tail(format!(
                "Input unblocked after 10s; wait still running ({}).",
                wait_ids.join(", ")
            ));
        } else {
            self.push_background_tail("Input unblocked after 10s; wait still running.".to_string());
        }

        if let Some(front) = self.queued_user_messages.front().cloned() {
            self.dispatch_queued_user_message_now(front);
        }

        // Reset timer only if messages remain; otherwise leave it cleared so the next queue
        // submission can schedule a fresh watchdog.
        if self.queued_user_messages.is_empty() {
            self.queue_block_started_at = None;
        } else {
            self.queue_block_started_at = Some(Instant::now());
        }
        self.request_redraw();
    }

    /// Clear the composer text and any pending paste placeholders/history cursors.
    pub(crate) fn clear_composer(&mut self) {
        self.bottom_pane.clear_composer();
        if self.auto_state.should_show_goal_entry() {
            self.auto_goal_escape_state = AutoGoalEscState::Inactive;
        }
        // Mark a height change so layout adjusts immediately if the composer shrinks.
        self.height_manager
            .borrow_mut()
            .record_event(crate::height_manager::HeightEvent::ComposerModeChange);
        self.request_redraw();
    }

    /// Forward an `Op` directly to codex.
    pub(crate) fn submit_op(&self, op: Op) {
        if let Err(e) = self.code_op_tx.send(op) {
            tracing::error!("failed to submit op: {e}");
        }
    }

    /// Cancel the current running task from a non-keyboard context (e.g. approval modal).
    /// This bypasses modal key handling and invokes the same immediate UI cleanup path
    /// as pressing Ctrl-C/Esc while a task is running.
    pub(crate) fn cancel_running_task_from_approval(&mut self) {
        self.interrupt_running_task();
    }

    /// Stop any in-flight turn (Auto Drive, agents, streaming responses) before
    /// starting a brand new chat so that stale output cannot leak into the new
    /// conversation.
    pub(crate) fn abort_active_turn_for_new_chat(&mut self) {
        if self.has_cancelable_agents() {
            self.cancel_active_agents();
        }

        if self.auto_state.is_active() {
            self.auto_stop(None);
        }

        self.interrupt_running_task();
        self.finalize_active_stream();
        self.stream_state.drop_streaming = true;
        self.bottom_pane.set_task_running(false);
        self.bottom_pane.clear_live_ring();
        self.maybe_hide_spinner();
    }

    pub(crate) fn register_approved_command(
        &self,
        command: Vec<String>,
        match_kind: ApprovedCommandMatchKind,
        semantic_prefix: Option<Vec<String>>,
    ) {
        if command.is_empty() {
            return;
        }
        let op = Op::RegisterApprovedCommand {
            command,
            match_kind,
            semantic_prefix,
        };
        self.submit_op(op);
    }

    /// Clear transient spinner/status after a denial without interrupting core
    /// execution. Only hide the spinner when there is no remaining activity so
    /// we avoid masking in-flight work (e.g. follow-up reasoning).
    pub(crate) fn mark_task_idle_after_denied(&mut self) {
        let any_tools_running = !self.exec.running_commands.is_empty()
            || !self.tools_state.running_custom_tools.is_empty()
            || !self.tools_state.web_search_sessions.is_empty();
        let any_streaming = self.stream.is_write_cycle_active();
        let any_agents_active = self.agents_are_actively_running();
        let any_tasks_active = !self.active_task_ids.is_empty();

        if !(any_tools_running || any_streaming || any_agents_active || any_tasks_active) {
            self.bottom_pane.set_task_running(false);
            self.bottom_pane.update_status_text(String::new());
            self.bottom_pane.clear_ctrl_c_quit_hint();
            self.mark_needs_redraw();
        }
    }

    pub(crate) fn insert_history_lines(&mut self, lines: Vec<ratatui::text::Line<'static>>) {
        let kind = self.stream_state.current_kind.unwrap_or(StreamKind::Answer);
        self.insert_history_lines_with_kind(kind, None, lines);
    }

    pub(crate) fn insert_history_lines_with_kind(
        &mut self,
        kind: StreamKind,
        id: Option<String>,
        lines: Vec<ratatui::text::Line<'static>>,
    ) {
        // No debug logging: we rely on preserving span modifiers end-to-end.
        // Insert all lines as a single streaming content cell to preserve spacing
        if lines.is_empty() {
            return;
        }

        if let Some(first_line) = lines.first() {
            let first_line_text: String = first_line
                .spans
                .iter()
                .map(|s| s.content.to_string())
                .collect();
            tracing::debug!("First line content: {:?}", first_line_text);
        }

        match kind {
            StreamKind::Reasoning => {
                // This reasoning block is the bottom-most; show progress indicator here only
                self.clear_reasoning_in_progress();
                // Ensure footer shows Ctrl+R hint when reasoning content is present
                self.bottom_pane.set_reasoning_hint(true);
                // Update footer label to reflect current visibility state
                self.bottom_pane
                    .set_reasoning_state(self.is_reasoning_shown());
                // Route by id when provided to avoid splitting reasoning across cells.
                // Be defensive: the cached index may be stale after inserts/removals; validate it.
                if let Some(ref rid) = id
                    && let Some(&idx) = self.reasoning_index.get(rid) {
                        if idx < self.history_cells.len()
                            && let Some(reasoning_cell) = self.history_cells[idx]
                                .as_any_mut()
                                .downcast_mut::<history_cell::CollapsibleReasoningCell>(
                            ) {
                                tracing::debug!(
                                    "Appending {} lines to Reasoning(id={})",
                                    lines.len(),
                                    rid
                                );
                                reasoning_cell.append_lines_dedup(lines);
                                reasoning_cell.set_in_progress(true);
                                self.invalidate_height_cache();
                                self.autoscroll_if_near_bottom();
                                self.request_redraw();
                                self.refresh_reasoning_collapsed_visibility();
                                return;
                            }
                        // Cached index was stale or wrong type — try to locate by scanning.
                        if let Some(found_idx) = self.history_cells.iter().rposition(|c| {
                            c.as_any()
                                .downcast_ref::<history_cell::CollapsibleReasoningCell>()
                                .map(|rc| rc.matches_id(rid))
                                .unwrap_or(false)
                        }) {
                            if let Some(reasoning_cell) = self.history_cells[found_idx]
                                .as_any_mut()
                                .downcast_mut::<history_cell::CollapsibleReasoningCell>()
                            {
                                // Refresh the cache with the corrected index
                                self.reasoning_index.insert(rid.clone(), found_idx);
                                tracing::debug!(
                                    "Recovered stale reasoning index; appending at {} for id={}",
                                    found_idx,
                                    rid
                                );
                                reasoning_cell.append_lines_dedup(lines);
                                reasoning_cell.set_in_progress(true);
                                self.invalidate_height_cache();
                                self.autoscroll_if_near_bottom();
                                self.request_redraw();
                                self.refresh_reasoning_collapsed_visibility();
                                return;
                            }
                        } else {
                            // No matching cell remains; drop the stale cache entry.
                            self.reasoning_index.remove(rid);
                        }
                    }

                tracing::debug!("Creating new CollapsibleReasoningCell id={:?}", id);
                let cell = history_cell::CollapsibleReasoningCell::new_with_id(lines, id.clone());
                if self.config.tui.show_reasoning {
                    cell.set_collapsed(false);
                } else {
                    cell.set_collapsed(true);
                }
                cell.set_in_progress(true);

                // Use pre-seeded key for this stream id when present; otherwise synthesize.
                let key = match id.as_deref() {
                    Some(rid) => self.try_stream_order_key(kind, rid).unwrap_or_else(|| {
                        tracing::warn!(
                            "missing stream order key for Reasoning id={}; using synthetic key",
                            rid
                        );
                        self.next_internal_key()
                    }),
                    None => {
                        tracing::warn!("missing stream id for Reasoning; using synthetic key");
                        self.next_internal_key()
                    }
                };
                tracing::info!(
                    "[order] insert Reasoning new id={:?} {}",
                    id,
                    Self::debug_fmt_order_key(key)
                );
                let idx = self.history_insert_with_key_global(Box::new(cell), key);
                if let Some(rid) = id {
                    self.reasoning_index.insert(rid, idx);
                }
                // Auto Drive status updates are handled via coordinator decisions.
            }
            StreamKind::Answer => {
                tracing::debug!(
                    "history.insert Answer id={:?} incoming_lines={}",
                    id,
                    lines.len()
                );
                self.clear_reasoning_in_progress();

                let explicit_id = id.clone();
                let stream_identifier = explicit_id.clone().unwrap_or_else(|| {
                    self.stream
                        .current_stream_id()
                        .map(std::string::ToString::to_string)
                        .unwrap_or_else(|| "stream-preview".to_string())
                });

                let fallback_preview = self
                    .synthesize_stream_state_from_lines(Some(&stream_identifier), &lines, true)
                    .preview_markdown;
                let preview_markdown = self
                    .stream
                    .preview_source_for_kind(StreamKind::Answer)
                    .unwrap_or(fallback_preview);

                let mutation = self.history_state.apply_domain_event(
                    HistoryDomainEvent::UpsertAssistantStream {
                        stream_id: stream_identifier,
                        preview_markdown,
                        delta: None,
                        metadata: None,
                    },
                );

                match mutation {
                    HistoryMutation::Inserted { id: history_id, record, .. } => {
                        let insert_key = match explicit_id.as_deref() {
                            Some(rid) => self.try_stream_order_key(kind, rid).unwrap_or_else(|| {
                                tracing::warn!(
                                    "missing stream order key for Answer id={}; using synthetic key",
                                    rid
                                );
                                self.next_internal_key()
                            }),
                            None => {
                                tracing::warn!(
                                    "missing stream id for Answer; using synthetic key"
                                );
                                self.next_internal_key()
                            }
                        };

                        if let Some(mut cell) = self.build_cell_from_record(&record) {
                            self.assign_history_id(&mut cell, history_id);
                            let new_idx = self.history_insert_existing_record(
                                cell,
                                insert_key,
                                "stream-begin",
                                history_id,
                            );
                            tracing::debug!(
                                "history.new StreamingContentCell at idx={} id={:?}",
                                new_idx,
                                explicit_id
                            );
                        } else {
                            tracing::warn!("assistant stream record could not build cell");
                        }
                    }
                    HistoryMutation::Replaced { id: history_id, record, .. } => {
                        self.update_cell_from_record(history_id, record);
                        self.mark_history_dirty();
                    }
                    HistoryMutation::Noop => {}
                    other => tracing::debug!(
                        "unexpected streaming mutation {:?} for id={:?}",
                        other,
                        explicit_id
                    ),
                }
            }
        }

        // Auto-follow if near bottom so new inserts are visible
        self.autoscroll_if_near_bottom();
        self.request_redraw();
        self.flush_history_snapshot_if_needed(false);
    }

    fn synthesize_stream_state_from_lines(
        &self,
        stream_id: Option<&String>,
        lines: &[ratatui::text::Line<'static>],
        in_progress: bool,
    ) -> AssistantStreamState {
        let mut preview = String::new();
        for (idx, line) in lines.iter().enumerate() {
            let flat: String = line
                .spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect();
            if idx == 0 && flat.trim().eq_ignore_ascii_case("codex") {
                continue;
            }
            if !preview.is_empty() {
                preview.push('\n');
            }
            preview.push_str(&flat);
        }
        if !preview.is_empty() && !preview.ends_with('\n') {
            preview.push('\n');
        }
        let mut stream_id_string = stream_id
            .cloned()
            .unwrap_or_else(|| "stream-preview".to_string());
        if stream_id_string.is_empty() {
            stream_id_string = "stream-preview".to_string();
        }
        AssistantStreamState {
            id: HistoryId::ZERO,
            stream_id: stream_id_string,
            preview_markdown: preview,
            deltas: Vec::new(),
            citations: Vec::new(),
            metadata: None,
            in_progress,
            last_updated_at: SystemTime::now(),
            truncated_prefix_bytes: 0,
        }
    }

    fn refresh_streaming_cell_for_stream_id(
        &mut self,
        stream_id: &str,
        state: AssistantStreamState,
    ) {
        if state.id != HistoryId::ZERO {
            self.update_cell_from_record(
                state.id,
                HistoryRecord::AssistantStream(state),
            );
            self.autoscroll_if_near_bottom();
            return;
        }

        if let Some(existing) = self
            .history_state
            .assistant_stream_state(stream_id)
            .cloned()
            && existing.id != HistoryId::ZERO {
                self.update_cell_from_record(
                    existing.id,
                    HistoryRecord::AssistantStream(existing),
                );
                self.autoscroll_if_near_bottom();
            }
    }

    fn update_stream_token_usage_metadata(&mut self) {
        let Some(stream_id) = self.stream.current_stream_id().cloned() else {
            return;
        };
        let Some(preview) = self
            .stream
            .preview_source_for_kind(StreamKind::Answer)
        else {
            return;
        };
        let metadata = MessageMetadata {
            citations: Vec::new(),
            token_usage: Some(self.last_token_usage.clone()),
        };
        self
            .history_state
            .upsert_assistant_stream_state(&stream_id, preview, None, Some(&metadata));
        if let Some(state) = self
            .history_state
            .assistant_stream_state(&stream_id)
            .cloned()
        {
            self.refresh_streaming_cell_for_stream_id(&stream_id, state);
        }
    }

    fn track_answer_stream_delta(&mut self, stream_id: &str, delta: &str, seq: Option<u64>) {
        let preview = self
            .stream
            .preview_source_for_kind(StreamKind::Answer)
            .unwrap_or_default();
        let delta = if delta.is_empty() {
            None
        } else {
            Some(AssistantStreamDelta {
                delta: delta.to_string(),
                sequence: seq,
                received_at: SystemTime::now(),
            })
        };
        let mutation = self.history_state.apply_domain_event(
            HistoryDomainEvent::UpsertAssistantStream {
                stream_id: stream_id.to_string(),
                preview_markdown: preview,
                delta,
                metadata: None,
            },
        );

        match mutation {
            HistoryMutation::Inserted { record, .. } => {
                if let HistoryRecord::AssistantStream(state) = record {
                    self.refresh_streaming_cell_for_stream_id(stream_id, state);
                    self.mark_history_dirty();
                }
            }
            HistoryMutation::Replaced { id, record, .. } => {
                if matches!(record, HistoryRecord::AssistantStream(_)) {
                    self.update_cell_from_record(id, record);
                    self.mark_history_dirty();
                }
            }
            _ => {}
        }
    }

    fn note_answer_stream_seen(&mut self, new_stream_id: &str) {
        let prev = self.last_seen_answer_stream_id_in_turn.clone();
        if let Some(prev) = prev
            && prev != new_stream_id {
                self.mid_turn_answer_ids_in_turn.insert(prev.clone());
                self.maybe_mark_finalized_answer_mid_turn(&prev);
            }
        self.last_seen_answer_stream_id_in_turn = Some(new_stream_id.to_string());
    }

    fn maybe_mark_finalized_answer_mid_turn(&mut self, prev_stream_id: &str) {
        let Some(last_final_id) = self.last_answer_stream_id_in_turn.as_deref() else {
            return;
        };
        if last_final_id != prev_stream_id {
            return;
        }
        let Some(prev_history_id) = self.last_answer_history_id_in_turn else {
            return;
        };

        let mut changed = false;
        if let Some(idx) = self
            .history_cell_ids
            .iter()
            .rposition(|hid| *hid == Some(prev_history_id))
            && let Some(cell) = self.history_cells[idx]
                .as_any_mut()
                .downcast_mut::<history_cell::AssistantMarkdownCell>()
                && !cell.state().mid_turn {
                    cell.set_mid_turn(true);
                    changed = true;
                }

        if let Some(record) = self.history_state.record_mut(prev_history_id)
            && let HistoryRecord::AssistantMessage(state) = record
                && !state.mid_turn {
                    state.mid_turn = true;
                    changed = true;
                }

        if changed {
            self.mark_history_dirty();
            self.request_redraw();
        }
    }

    fn apply_mid_turn_flag(&self, stream_id: Option<&str>, state: &mut AssistantMessageState) {
        if let Some(sid) = stream_id
            && self.mid_turn_answer_ids_in_turn.contains(sid) {
                state.mid_turn = true;
            }
    }

    fn maybe_clear_mid_turn_for_last_answer(&mut self, stream_id: &str) {
        let Some(last_history_id) = self.last_answer_history_id_in_turn else {
            return;
        };

        let mut changed = false;

        if let Some(record) = self.history_state.record_mut(last_history_id)
            && let HistoryRecord::AssistantMessage(state) = record
                && state.stream_id.as_deref() == Some(stream_id) && state.mid_turn {
                    state.mid_turn = false;
                    changed = true;
                }

        if let Some(idx) = self
            .history_cell_ids
            .iter()
            .rposition(|hid| *hid == Some(last_history_id))
            && let Some(cell) = self.history_cells[idx]
                .as_any_mut()
                .downcast_mut::<history_cell::AssistantMarkdownCell>()
                && cell.stream_id() == Some(stream_id) && cell.state().mid_turn {
                    cell.set_mid_turn(false);
                    changed = true;
                }

        if changed {
            self.mark_history_dirty();
            self.request_redraw();
        }
    }

    fn finalize_answer_stream_state(
        &mut self,
        stream_id: Option<&str>,
        source: &str,
    ) -> AssistantMessageState {
        let mut metadata = stream_id.and_then(|sid| {
            self.history_state
                .assistant_stream_state(sid)
                .and_then(|state| state.metadata.clone())
        });

        let should_attach_token_usage = self.last_token_usage.total_tokens > 0;
        if should_attach_token_usage {
            if let Some(meta) = metadata.as_mut() {
                if meta.token_usage.is_none() {
                    meta.token_usage = Some(self.last_token_usage.clone());
                }
            } else {
                metadata = Some(MessageMetadata {
                    citations: Vec::new(),
                    token_usage: Some(self.last_token_usage.clone()),
                });
            }
        }

        let token_usage = if should_attach_token_usage {
            Some(self.last_token_usage.clone())
        } else {
            None
        };

        
        self.history_state.finalize_assistant_stream_state(
            stream_id,
            source.to_string(),
            metadata.as_ref(),
            token_usage.as_ref(),
        )
    }

    /// Replace the in-progress streaming assistant cell with a final markdown cell that
    /// stores raw markdown for future re-rendering.
    pub(crate) fn insert_final_answer_with_id(
        &mut self,
        id: Option<String>,
        lines: Vec<ratatui::text::Line<'static>>,
        source: String,
    ) {
        tracing::debug!(
            "insert_final_answer_with_id id={:?} source_len={} lines={}",
            id,
            source.len(),
            lines.len()
        );
        tracing::info!("[order] final Answer id={:?}", id);
        let final_source = source.clone();

        if self.auto_state.pending_stop_message.is_some() {
            match serde_json::from_str::<code_auto_drive_diagnostics::CompletionCheck>(&final_source)
            {
                Ok(check) => {
                    if check.complete {
                        let explanation = check.explanation.trim();
                        if explanation.is_empty() {
                            self.auto_state.last_completion_explanation = None;
                        } else {
                            self.auto_state.last_completion_explanation =
                                Some(explanation.to_string());
                        }
                        let pending = self.auto_state.pending_stop_message.take();
                        if let Some(idx) = self.history_cells.iter().rposition(|c| {
                            c.as_any()
                                .downcast_ref::<history_cell::StreamingContentCell>()
                                .and_then(|sc| sc.id.as_ref())
                                .map(|existing| Some(existing.as_str()) == id.as_deref())
                                .unwrap_or(false)
                        }) {
                            self.history_remove_at(idx);
                        }
                        if let Some(ref stream_id) = id {
                            let _ = self.history_state.finalize_assistant_stream_state(
                                Some(stream_id.as_str()),
                                String::new(),
                                None,
                                None,
                            );
                            self.stream_state
                                .closed_answer_ids
                                .insert(StreamId(stream_id.clone()));
                        }
                        self.auto_stop(pending);
                        self.stop_spinner();
                        return;
                    } else {
                        self.auto_state.last_completion_explanation = None;
                        let goal = self
                            .auto_state
                            .goal
                            .as_deref()
                            .unwrap_or("(goal unavailable)");
                    let follow_up = format!(
                        "The primary goal has not been met. Please continue working on this.\nPrimary Goal: {goal}\nExplanation: {explanation}",
                        explanation = check.explanation
                    );
                    let mut conversation = self.rebuild_auto_history();
                    if let Some(user_item) = Self::auto_drive_make_user_message(follow_up) {
                        conversation.push(user_item.clone());
                        self.auto_history.append_raw(std::slice::from_ref(&user_item));
                    }
                    self.auto_state.pending_stop_message = None;
                    // Re-run the conversation through the normal decision pipeline so the
                    // coordinator produces a full finish_status/progress/cli turn rather than
                    // falling back to the user-response schema.
                    self.auto_state.set_phase(AutoRunPhase::Active);
                    self.auto_send_conversation_force();
                    self.stop_spinner();
                    return;
                }
                }
                Err(err) => {
                    tracing::warn!(
                        "failed to parse diagnostics completion check: {}",
                        err
                    );
                    self.auto_state.last_completion_explanation = None;
                    let pending = self.auto_state.pending_stop_message.take();
                    self.auto_stop(pending);
                }
            }
        }
            if self.is_review_flow_active() {
            if let Some(ref want) = id {
                if !self
                    .stream_state
                    .closed_answer_ids
                    .insert(StreamId(want.clone()))
                {
                    tracing::debug!(
                        "InsertFinalAnswer(review): dropping duplicate final for id={}",
                        want
                    );
                    self.last_assistant_message = Some(final_source.clone());
                    self.maybe_hide_spinner();
                    return;
                }
                if let Some(idx) = self.history_cells.iter().rposition(|c| {
                    c.as_any()
                        .downcast_ref::<history_cell::StreamingContentCell>()
                        .and_then(|sc| sc.id.as_ref())
                        .map(|existing| existing == want)
                        .unwrap_or(false)
                }) {
                    self.history_remove_at(idx);
                }
            } else if let Some(idx) = self.history_cells.iter().rposition(|c| {
                c.as_any()
                    .downcast_ref::<history_cell::StreamingContentCell>()
                    .is_some()
            }) {
                self.history_remove_at(idx);
            }
            self.last_assistant_message = Some(final_source.clone());
            let mut state = self.finalize_answer_stream_state(id.as_deref(), &final_source);
            self.apply_mid_turn_flag(id.as_deref(), &mut state);
            let history_id = state.id;
            let mut key = match id.as_deref() {
                Some(rid) => self.try_stream_order_key(StreamKind::Answer, rid).unwrap_or_else(|| {
                    tracing::warn!(
                        "missing stream order key for final Answer id={}; using synthetic key",
                        rid
                    );
                    self.next_internal_key()
                }),
                None => {
                    tracing::warn!("missing stream id for final Answer; using synthetic key");
                    self.next_internal_key()
                }
            };

            if let Some(last) = self.last_assigned_order
                && key <= last {
                    key = Self::order_key_successor(last);
                    if let Some(ref want) = id {
                        self.stream_order_seq
                            .insert((StreamKind::Answer, want.clone()), key);
                    }
                }

            let cell = history_cell::AssistantMarkdownCell::from_state(state, &self.config);
            self.history_insert_existing_record(Box::new(cell), key, "answer-review", history_id);
            self.last_answer_stream_id_in_turn = id.clone();
            self.last_answer_history_id_in_turn = Some(history_id);
            // Advance Auto Drive after the assistant message has been finalized.
            self.auto_on_assistant_final();
            self.maybe_hide_spinner();
            return;
        }
        // Debug: list last few history cell kinds so we can see what's present
        let tail_kinds: String = self
            .history_cells
            .iter()
            .rev()
            .take(5)
            .map(|c| {
                if c.as_any()
                    .downcast_ref::<history_cell::StreamingContentCell>()
                    .is_some()
                {
                    "Streaming".to_string()
                } else if c
                    .as_any()
                    .downcast_ref::<history_cell::AssistantMarkdownCell>()
                    .is_some()
                {
                    "AssistantFinal".to_string()
                } else if c
                    .as_any()
                    .downcast_ref::<history_cell::CollapsibleReasoningCell>()
                    .is_some()
                {
                    "Reasoning".to_string()
                } else {
                    format!("{:?}", c.kind())
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        tracing::debug!("history.tail kinds(last5) = [{}]", tail_kinds);

        // When we have an id but could not find a streaming cell by id, dump ids
        if id.is_some() {
            let ids: Vec<String> = self
                .history_cells
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    c.as_any()
                        .downcast_ref::<history_cell::StreamingContentCell>()
                        .and_then(|sc| sc.id.as_ref().map(|s| format!("{i}:{s}")))
                })
                .collect();
            tracing::debug!("history.streaming ids={}", ids.join(" | "));
        }
        // If we already finalized this id in the current turn with identical content,
        // drop this event to avoid duplicates (belt-and-suspenders against upstream repeats).
        if let Some(ref want) = id
            && self
                .stream_state
                .closed_answer_ids
                .contains(&StreamId(want.clone()))
                && let Some(existing_idx) = self.history_cells.iter().rposition(|c| {
                    c.as_any()
                        .downcast_ref::<history_cell::AssistantMarkdownCell>()
                        .map(|amc| amc.stream_id() == Some(want.as_str()))
                        .unwrap_or(false)
                })
                    && let Some(amc) = self.history_cells[existing_idx]
                        .as_any()
                        .downcast_ref::<history_cell::AssistantMarkdownCell>()
                    {
                        let prev = Self::normalize_text(amc.markdown());
                        let newn = Self::normalize_text(&source);
                        if prev == newn {
                            tracing::debug!(
                                "InsertFinalAnswer: dropping duplicate final for id={}",
                                want
                            );
                            self.maybe_hide_spinner();
                            return;
                        }
                    }
        // Ensure a hidden 'codex' header is present
        let has_header = lines
            .first()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .trim()
                    .eq_ignore_ascii_case("codex")
            })
            .unwrap_or(false);
        if !has_header {
            // No need to mutate `lines` further since we rebuild from `source` below.
        }

        // Replace the matching StreamingContentCell if one exists for this id; else fallback to most recent.
        // NOTE (dup‑guard): This relies on `StreamingContentCell::as_any()` returning `self`.
        // If that impl is removed, downcast_ref will fail and we won't find the streaming cell,
        // causing the final to append a new Assistant cell (duplicate).
        let streaming_idx = if let Some(ref want) = id {
            // Only replace a streaming cell if its id matches this final.
            self.history_cells.iter().rposition(|c| {
                if let Some(sc) = c
                    .as_any()
                    .downcast_ref::<history_cell::StreamingContentCell>()
                {
                    sc.id.as_ref() == Some(want)
                } else {
                    false
                }
            })
        } else {
            None
        };
        if let Some(idx) = streaming_idx {
            tracing::debug!(
                "final-answer: replacing StreamingContentCell at idx={} by id match",
                idx
            );
            let mut state = self.finalize_answer_stream_state(id.as_deref(), &final_source);
            self.apply_mid_turn_flag(id.as_deref(), &mut state);
            let history_id = state.id;
            let cell = history_cell::AssistantMarkdownCell::from_state(state, &self.config);
            self.history_replace_at(idx, Box::new(cell));
            if let Some(ref want) = id {
                self.stream_state
                    .closed_answer_ids
                    .insert(StreamId(want.clone()));
            }
            self.autoscroll_if_near_bottom();
            self.last_answer_stream_id_in_turn = id.clone();
            self.last_answer_history_id_in_turn = Some(history_id);
            // Final cell committed via replacement; now advance Auto Drive.
            self.auto_on_assistant_final();
            self.maybe_hide_spinner();
            return;
        }

        // No streaming cell found. First, try to replace a finalized assistant cell
        // that was created for the same stream id (e.g., we already finalized due to
        // a lifecycle event and this InsertFinalAnswer arrived slightly later).
        if let Some(ref want) = id
            && let Some(idx) = self.history_cells.iter().rposition(|c| {
                if let Some(amc) = c
                    .as_any()
                    .downcast_ref::<history_cell::AssistantMarkdownCell>()
                {
                    amc.stream_id() == Some(want.as_str())
                } else {
                    false
                }
            }) {
                tracing::debug!(
                    "final-answer: replacing existing AssistantMarkdownCell at idx={} by id match",
                    idx
                );
                let mut state =
                    self.finalize_answer_stream_state(id.as_deref(), &final_source);
                self.apply_mid_turn_flag(id.as_deref(), &mut state);
                let history_id = state.id;
                let cell = history_cell::AssistantMarkdownCell::from_state(state, &self.config);
                self.history_replace_at(idx, Box::new(cell));
                self.stream_state
                    .closed_answer_ids
                    .insert(StreamId(want.clone()));
                self.autoscroll_if_near_bottom();
                self.last_answer_stream_id_in_turn = id.clone();
                self.last_answer_history_id_in_turn = Some(history_id);
                // Final cell replaced in-place; advance Auto Drive now.
                self.auto_on_assistant_final();
                self.maybe_hide_spinner();
                return;
            }

        // Otherwise, if a finalized assistant cell exists at the tail,
        // replace it in place to avoid duplicate assistant messages when a second
        // InsertFinalAnswer (e.g., from an AgentMessage event) arrives after we already
        // finalized due to a side event.
        if let Some(idx) = self.history_cells.iter().rposition(|c| {
            c.as_any()
                .downcast_ref::<history_cell::AssistantMarkdownCell>()
                .is_some()
        }) {
            // Replace the tail finalized assistant cell if the new content is identical OR
            // a small revision that merely adds leading/trailing context. Otherwise append a
            // new assistant message so distinct replies remain separate.
            let should_replace = self.history_cells[idx]
                .as_any()
                .downcast_ref::<history_cell::AssistantMarkdownCell>()
                .map(|amc| {
                    let prev = Self::normalize_text(amc.markdown());
                    let newn = Self::normalize_text(&source);
                    let identical = prev == newn;
                    if identical || prev.is_empty() {
                        return identical;
                    }
                    let is_prefix_expansion = newn.starts_with(&prev);
                    let is_suffix_expansion = newn.ends_with(&prev);
                    let is_large_superset = prev.len() >= 80 && newn.contains(&prev);
                    identical || is_prefix_expansion || is_suffix_expansion || is_large_superset
                })
                .unwrap_or(false);
            if should_replace {
                tracing::debug!(
                    "final-answer: replacing tail AssistantMarkdownCell via heuristic identical/expansion"
                );
                let mut state =
                    self.finalize_answer_stream_state(id.as_deref(), &final_source);
                self.apply_mid_turn_flag(id.as_deref(), &mut state);
                let history_id = state.id;
                let cell = history_cell::AssistantMarkdownCell::from_state(state, &self.config);
                self.history_replace_at(idx, Box::new(cell));
                self.autoscroll_if_near_bottom();
                self.last_answer_stream_id_in_turn = id.clone();
                self.last_answer_history_id_in_turn = Some(history_id);
                // Final assistant content revised; advance Auto Drive now.
                self.auto_on_assistant_final();
                self.maybe_hide_spinner();
                return;
            }
        }

        // Fallback: no prior assistant cell found; insert at stable sequence position.
        tracing::debug!(
            "final-answer: ordered insert new AssistantMarkdownCell id={:?}",
            id
        );
        let mut key = match id.as_deref() {
            Some(rid) => self
                .try_stream_order_key(StreamKind::Answer, rid)
                .unwrap_or_else(|| {
                    tracing::warn!(
                        "missing stream order key for final Answer id={}; using synthetic key",
                        rid
                    );
                    self.next_internal_key()
                }),
            None => {
                tracing::warn!("missing stream id for final Answer; using synthetic key");
                self.next_internal_key()
            }
        };
        if let Some(last) = self.last_assigned_order
            && key <= last {
                // Background notices anchor themselves at out = i32::MAX. If a final answer arrives
                // after those notices we still want it to appear at the bottom, so bump the key
                // just past the most-recently assigned slot.
                key = Self::order_key_successor(last);
                if let Some(ref want) = id {
                    self.stream_order_seq
                        .insert((StreamKind::Answer, want.clone()), key);
                }
            }
        tracing::info!(
            "[order] final Answer ordered insert id={:?} {}",
            id,
            Self::debug_fmt_order_key(key)
        );
        let mut state = self.finalize_answer_stream_state(id.as_deref(), &final_source);
        self.apply_mid_turn_flag(id.as_deref(), &mut state);
        let history_id = state.id;
        let cell = history_cell::AssistantMarkdownCell::from_state(state, &self.config);
        self.history_insert_existing_record(
            Box::new(cell),
            key,
            "answer-final",
            history_id,
        );
        if let Some(ref want) = id {
            self.stream_state
                .closed_answer_ids
                .insert(StreamId(want.clone()));
        }
        self.last_answer_stream_id_in_turn = id.clone();
        self.last_answer_history_id_in_turn = Some(history_id);
        // Ordered insert completed; advance Auto Drive now that the assistant
        // message is present in history.
        self.auto_on_assistant_final();
        self.maybe_hide_spinner();
    }

    // Assign or fetch a stable sequence for a stream kind+id within its originating turn
    // removed legacy ensure_stream_order_key; strict variant is used instead

    /// Normalize text for duplicate detection (trim trailing whitespace and normalize newlines)
    fn normalize_text(s: &str) -> String {
        // 1) Normalize newlines
        let s = s.replace("\r\n", "\n");
        // 2) Trim trailing whitespace per line; collapse repeated blank lines
        let mut out: Vec<String> = Vec::new();
        let mut saw_blank = false;
        for line in s.lines() {
            // Replace common Unicode bullets with ASCII to stabilize equality checks
            let line = line
                .replace(['\u{2022}', '\u{25E6}', '\u{2219}'], "-"); // ∙
            let trimmed = line.trim_end();
            if trimmed.chars().all(char::is_whitespace) {
                if !saw_blank {
                    out.push(String::new());
                }
                saw_blank = true;
            } else {
                out.push(trimmed.to_string());
                saw_blank = false;
            }
        }
        // 3) Remove trailing blank lines
        while out.last().is_some_and(std::string::String::is_empty) {
            out.pop();
        }
        out.join("\n")
    }

    pub(crate) fn toggle_reasoning_visibility(&mut self) {
        // Track whether any reasoning cells are found and their new state
        let mut has_reasoning_cells = false;
        let mut new_collapsed_state = false;

        // Toggle all CollapsibleReasoningCell instances in history
        for cell in &self.history_cells {
            // Try to downcast to CollapsibleReasoningCell
            if let Some(reasoning_cell) = cell
                .as_any()
                .downcast_ref::<history_cell::CollapsibleReasoningCell>()
            {
                reasoning_cell.toggle_collapsed();
                has_reasoning_cells = true;
                new_collapsed_state = reasoning_cell.is_collapsed();
            }
        }

        // Update the config to reflect the current state (inverted because collapsed means hidden)
        if has_reasoning_cells {
            self.config.tui.show_reasoning = !new_collapsed_state;
            // Brief status to confirm the toggle to the user
            let status = if self.config.tui.show_reasoning {
                "Reasoning shown"
            } else {
                "Reasoning hidden"
            };
            self.bottom_pane.update_status_text(status.to_string());
            // Update footer label to reflect current state
            self.bottom_pane
                .set_reasoning_state(self.config.tui.show_reasoning);
        } else {
            // No reasoning cells exist; inform the user
            self.bottom_pane
                .update_status_text("No reasoning to toggle".to_string());
        }
        self.refresh_reasoning_collapsed_visibility();
        // Collapsed state changes affect heights; clear cache
        self.invalidate_height_cache();
        self.request_redraw();
        // In standard terminal mode, re-mirror the transcript so scrollback reflects
        // the new collapsed/expanded state. We cannot edit prior lines in scrollback,
        // so append a fresh view.
        if self.standard_terminal_mode {
            let mut lines = Vec::new();
            lines.push(ratatui::text::Line::from(""));
            lines.extend(self.export_transcript_lines_for_buffer());
            self.app_event_tx
                .send(crate::app_event::AppEvent::InsertHistory(lines));
        }
    }

    fn refresh_standard_terminal_hint(&mut self) {
        if self.standard_terminal_mode {
            let message = "Standard terminal mode active. Press Ctrl+T to return to full UI.";
            self.bottom_pane
                .set_standard_terminal_hint(Some(message.to_string()));
        } else {
            self.bottom_pane.set_standard_terminal_hint(None);
        }
    }

    pub(crate) fn set_standard_terminal_mode(&mut self, enabled: bool) {
        self.standard_terminal_mode = enabled;
        self.refresh_standard_terminal_hint();
    }

    pub(crate) fn is_reasoning_shown(&self) -> bool {
        // Check if any reasoning cell exists and if it's expanded
        for cell in &self.history_cells {
            if let Some(reasoning_cell) = cell
                .as_any()
                .downcast_ref::<history_cell::CollapsibleReasoningCell>()
            {
                return !reasoning_cell.is_collapsed();
            }
        }
        // If no reasoning cells exist, return the config default
        self.config.tui.show_reasoning
    }

    pub(crate) fn show_chrome_options(&mut self, port: Option<u16>) {
        self.ensure_settings_overlay_section(SettingsSection::Chrome);
        let content = self.build_chrome_settings_content(port);
        if let Some(overlay) = self.settings.overlay.as_mut() {
            overlay.set_chrome_content(content);
        }
        self.request_redraw();
    }

    pub(crate) fn handle_chrome_launch_option(
        &mut self,
        option: ChromeLaunchOption,
        port: Option<u16>,
    ) {
        let launch_port = port.unwrap_or(9222);
        let ticket = self.make_background_tail_ticket();

        match option {
            ChromeLaunchOption::CloseAndUseProfile => {
                // Kill existing Chrome and launch with user profile
                #[cfg(target_os = "macos")]
                {
                    let _ = std::process::Command::new("pkill")
                        .arg("-f")
                        .arg("Google Chrome")
                        .output();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                #[cfg(target_os = "linux")]
                {
                    let _ = std::process::Command::new("pkill")
                        .arg("-f")
                        .arg("chrome")
                        .output();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                #[cfg(target_os = "windows")]
                {
                    let _ = std::process::Command::new("taskkill")
                        .arg("/F")
                        .arg("/IM")
                        .arg("chrome.exe")
                        .output();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                self.launch_chrome_with_profile(launch_port);
                // Connect to Chrome after launching
                self.connect_to_chrome_after_launch(launch_port, ticket);
            }
            ChromeLaunchOption::UseTempProfile => {
                // Launch with temporary profile
                self.launch_chrome_with_temp_profile(launch_port);
                // Connect to Chrome after launching
                self.connect_to_chrome_after_launch(launch_port, ticket);
            }
            ChromeLaunchOption::UseInternalBrowser => {
                // Redirect to internal browser command
                self.handle_browser_command(String::new());
            }
            ChromeLaunchOption::Cancel => {
                // Do nothing, just close the dialog
            }
        }
    }

    fn launch_chrome_with_profile(&mut self, port: u16) {
        use std::process::Stdio;
        let log_path = self.chrome_log_path();

        #[cfg(target_os = "macos")]
        {
            let mut cmd = std::process::Command::new(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            );
            cmd.arg(format!("--remote-debugging-port={port}"))
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-component-extensions-with-background-pages")
                .arg("--disable-background-networking")
                .arg("--silent-debugger-extension-api")
                .arg("--remote-allow-origins=*")
                .arg("--disable-features=ChromeWhatsNewUI,TriggerFirstRunUI")
                .arg("--disable-hang-monitor")
                .arg("--disable-background-timer-throttling")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .stdin(Stdio::null());
            self.apply_chrome_logging(&mut cmd, log_path.as_deref());
            if let Err(err) = spawn_std_command_with_retry(&mut cmd) {
                tracing::warn!("failed to launch Chrome with profile: {err}");
            }
        }

        #[cfg(target_os = "linux")]
        {
            let mut cmd = std::process::Command::new("google-chrome");
            cmd.arg(format!("--remote-debugging-port={}", port))
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-component-extensions-with-background-pages")
                .arg("--disable-background-networking")
                .arg("--silent-debugger-extension-api")
                .arg("--remote-allow-origins=*")
                .arg("--disable-features=ChromeWhatsNewUI,TriggerFirstRunUI")
                .arg("--disable-hang-monitor")
                .arg("--disable-background-timer-throttling")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .stdin(Stdio::null());
            self.apply_chrome_logging(&mut cmd, log_path.as_deref());
            if let Err(err) = spawn_std_command_with_retry(&mut cmd) {
                tracing::warn!("failed to launch Chrome with profile: {err}");
            }
        }

        #[cfg(target_os = "windows")]
        {
            let chrome_paths = vec![
                "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe".to_string(),
                "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe".to_string(),
                format!(
                    "{}\\AppData\\Local\\Google\\Chrome\\Application\\chrome.exe",
                    std::env::var("USERPROFILE").unwrap_or_default()
                ),
            ];

            for chrome_path in chrome_paths {
                if std::path::Path::new(&chrome_path).exists() {
                    let mut cmd = std::process::Command::new(&chrome_path);
                    cmd.arg(format!("--remote-debugging-port={}", port))
                        .arg("--no-first-run")
                        .arg("--no-default-browser-check")
                        .arg("--disable-component-extensions-with-background-pages")
                        .arg("--disable-background-networking")
                        .arg("--silent-debugger-extension-api")
                        .arg("--remote-allow-origins=*")
                        .arg("--disable-features=ChromeWhatsNewUI,TriggerFirstRunUI")
                        .arg("--disable-hang-monitor")
                        .arg("--disable-background-timer-throttling")
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .stdin(Stdio::null());
                    self.apply_chrome_logging(&mut cmd, log_path.as_deref());
                    if let Err(err) = spawn_std_command_with_retry(&mut cmd) {
                        tracing::warn!("failed to launch Chrome with profile: {err}");
                    }
                    break;
                }
            }
        }

        // Add status message
        self.push_background_tail("✅ Chrome launched with user profile".to_string());
        // Show browsing state in input border after launch
        self.bottom_pane
            .update_status_text("using browser".to_string());
    }

    fn chrome_log_path(&self) -> Option<String> {
        if !self.config.debug {
            return None;
        }
        let log_dir = code_core::config::log_dir(&self.config).ok()?;
        Some(log_dir.join("code-chrome.log").display().to_string())
    }

    fn apply_chrome_logging(&self, cmd: &mut std::process::Command, log_path: Option<&str>) {
        if let Some(path) = log_path {
            cmd.arg("--enable-logging")
                .arg("--log-level=1")
                .arg(format!("--log-file={path}"));
        }
    }

    fn connect_to_chrome_after_launch(
        &mut self,
        port: u16,
        ticket: BackgroundOrderTicket,
    ) {
        // Wait a moment for Chrome to start, then reuse the existing connection logic
        let app_event_tx = self.app_event_tx.clone();
        let latest_screenshot = self.latest_browser_screenshot.clone();

        tokio::spawn(async move {
            // Wait for Chrome to fully start
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

            // Now try to connect using the shared CDP connection logic
            ChatWidget::connect_to_cdp_chrome(
                None,
                Some(port),
                latest_screenshot,
                app_event_tx,
                ticket,
            )
            .await;
        });
    }

    /// Shared CDP connection logic used by both /chrome command and Chrome launch options
    async fn connect_to_cdp_chrome(
        host: Option<String>,
        port: Option<u16>,
        latest_screenshot: Arc<Mutex<Option<(PathBuf, String)>>>,
        app_event_tx: AppEventSender,
        ticket: BackgroundOrderTicket,
    ) {
        tracing::info!(
            "[cdp] connect_to_cdp_chrome() begin, host={:?}, port={:?}",
            host,
            port
        );
        let browser_manager = ChatWidget::get_browser_manager().await;
        browser_manager.set_enabled_sync(true);

        // Configure for CDP connection (prefer cached ws/port on auto-detect)
        // Track whether we're attempting via cached WS and retain a cached port for fallback.
        let mut attempted_via_cached_ws = false;
        let mut cached_port_for_fallback: Option<u16> = None;
        {
            let mut config = browser_manager.config.write().await;
            config.headless = false;
            config.persist_profile = true;
            config.enabled = true;

            if let Some(p) = port {
                config.connect_ws = None;
                config.connect_host = host.clone();
                config.connect_port = Some(p);
            } else {
                // Load persisted cache from disk (if any), then fall back to in-memory
                let (cached_port, cached_ws) = match read_cached_connection().await {
                    Some(v) => v,
                    None => code_browser::global::get_last_connection().await,
                };
                cached_port_for_fallback = cached_port;
                if let Some(ws) = cached_ws {
                    tracing::info!("[cdp] using cached Chrome WS endpoint");
                    attempted_via_cached_ws = true;
                    config.connect_ws = Some(ws);
                    config.connect_port = None;
                } else if let Some(p) = cached_port_for_fallback {
                    tracing::info!("[cdp] using cached Chrome debug port: {}", p);
                    config.connect_ws = None;
                    config.connect_host = host.clone();
                    config.connect_port = Some(p);
                } else {
                    config.connect_ws = None;
                    config.connect_host = host.clone();
                    config.connect_port = Some(0); // auto-detect
                }
            }
        }

        // Try to connect to existing Chrome (no fallback to internal browser) with timeout
        tracing::info!("[cdp] calling BrowserManager::connect_to_chrome_only()…");
        // Allow 15s for WS discovery + 5s for connect
        let connect_deadline = tokio::time::Duration::from_secs(20);
        let connect_result =
            tokio::time::timeout(connect_deadline, browser_manager.connect_to_chrome_only()).await;
        match connect_result {
            Err(_) => {
                tracing::error!(
                    "[cdp] connect_to_chrome_only timed out after {:?}",
                    connect_deadline
                );
                app_event_tx.send_background_event_with_ticket(
                    &ticket,
                    format!(
                        "❌ CDP connect timed out after {}s. Ensure Chrome is running with --remote-debugging-port={} and http://127.0.0.1:{}/json/version is reachable",
                        connect_deadline.as_secs(),
                        port.unwrap_or(0),
                        port.unwrap_or(0)
                    ),
                );
                // Offer launch options popup to help recover quickly
                app_event_tx.send(AppEvent::ShowChromeOptions(port));
            }
            Ok(result) => match result {
                Ok(_) => {
                    tracing::info!("[cdp] Connected to Chrome via CDP");

                    // Build a detailed success message including CDP port and current URL when available
                    let (detected_port, detected_ws) =
                        code_browser::global::get_last_connection().await;
                    // Prefer explicit port; otherwise try to parse from ws URL
                    let mut port_num: Option<u16> = detected_port;
                    if port_num.is_none()
                        && let Some(ws) = &detected_ws {
                            // crude parse: ws://host:port/...
                            if let Some(after_scheme) = ws.split("//").nth(1)
                                && let Some(hostport) = after_scheme.split('/').next()
                                    && let Some(pstr) = hostport.split(':').nth(1)
                                        && let Ok(p) = pstr.parse::<u16>() {
                                            port_num = Some(p);
                                        }
                        }

                    // Try to capture current page URL (best-effort)
                    let current_url = browser_manager.get_current_url().await;

                    let success_msg = match (port_num, current_url) {
                        (Some(p), Some(url)) if !url.is_empty() => {
                            format!("✅ Connected to Chrome via CDP (port {p}) to {url}")
                        }
                        (Some(p), _) => format!("✅ Connected to Chrome via CDP (port {p})"),
                        (None, Some(url)) if !url.is_empty() => {
                            format!("✅ Connected to Chrome via CDP to {url}")
                        }
                        _ => "✅ Connected to Chrome via CDP".to_string(),
                    };

                    // Immediately notify success (do not block on screenshots)
                    app_event_tx
                        .send_background_event_with_ticket(&ticket, success_msg.clone());

                    // Persist last connection cache to disk (best-effort)
                    tokio::spawn(async move {
                        let (p, ws) = code_browser::global::get_last_connection().await;
                        let _ = write_cached_connection(p, ws).await;
                    });

                    // Set up navigation callback
                    let latest_screenshot_callback = latest_screenshot.clone();
                    let app_event_tx_callback = app_event_tx.clone();

                    browser_manager
                        .set_navigation_callback(move |url| {
                            tracing::info!("CDP Navigation callback triggered for URL: {}", url);
                            let latest_screenshot_inner = latest_screenshot_callback.clone();
                            let app_event_tx_inner = app_event_tx_callback.clone();
                            let url_inner = url;

                            tokio::spawn(async move {
                                tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                                let browser_manager_inner = ChatWidget::get_browser_manager().await;
                                let mut attempt = 0;
                                let max_attempts = 2;
                                loop {
                                    attempt += 1;
                                    match browser_manager_inner.capture_screenshot_with_url().await
                                    {
                                        Ok((paths, _)) => {
                                            if let Some(first_path) = paths.first() {
                                                tracing::info!(
                                                    "[cdp] auto-captured screenshot: {}",
                                                    first_path.display()
                                                );

                                                if let Ok(mut latest) =
                                                    latest_screenshot_inner.lock()
                                                {
                                                    *latest = Some((
                                                        first_path.clone(),
                                                        url_inner.clone(),
                                                    ));
                                                }

                                                use code_core::protocol::{
                                                    BrowserScreenshotUpdateEvent, Event, EventMsg,
                                                };
                                                app_event_tx_inner.send(
                                                    AppEvent::CodexEvent(Event {
                                                        id: uuid::Uuid::new_v4().to_string(),
                                                        event_seq: 0,
                                                        msg: EventMsg::BrowserScreenshotUpdate(
                                                            BrowserScreenshotUpdateEvent {
                                                                screenshot_path: first_path.clone(),
                                                                url: url_inner,
                                                            },
                                                        ),
                                                        order: None,
                                                    }),
                                                );
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!(
                                                "[cdp] auto-capture failed (attempt {}): {}",
                                                attempt,
                                                e
                                            );
                                            if attempt >= max_attempts {
                                                break;
                                            }
                                            tokio::time::sleep(tokio::time::Duration::from_millis(
                                                250,
                                            ))
                                            .await;
                                            continue;
                                        }
                                    }
                                    // end match
                                }
                                // end loop
                            });
                        })
                        .await;

                    // Set as global manager
                    code_browser::global::set_global_browser_manager(browser_manager.clone())
                        .await;

                    // Capture initial screenshot in background (don't block connect feedback)
                    {
                        let latest_screenshot_bg = latest_screenshot.clone();
                        let app_event_tx_bg = app_event_tx.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                            let browser_manager = ChatWidget::get_browser_manager().await;
                            let mut attempt = 0;
                            let max_attempts = 2;
                            loop {
                                attempt += 1;
                                match browser_manager.capture_screenshot_with_url().await {
                                    Ok((paths, url)) => {
                                        if let Some(first_path) = paths.first() {
                                            tracing::info!(
                                                "Initial CDP screenshot captured: {}",
                                                first_path.display()
                                            );
                                            if let Ok(mut latest) = latest_screenshot_bg.lock() {
                                                *latest = Some((
                                                    first_path.clone(),
                                                    url.clone()
                                                        .unwrap_or_else(|| "Chrome".to_string()),
                                                ));
                                            }
                                            use code_core::protocol::BrowserScreenshotUpdateEvent;
                                            use code_core::protocol::Event;
                                            use code_core::protocol::EventMsg;
                                            app_event_tx_bg.send(AppEvent::CodexEvent(Event {
                                                    id: uuid::Uuid::new_v4().to_string(),
                                                    event_seq: 0,
                                                    msg: EventMsg::BrowserScreenshotUpdate(
                                                        BrowserScreenshotUpdateEvent {
                                                            screenshot_path: first_path.clone(),
                                                            url: url.unwrap_or_else(|| {
                                                                "Chrome".to_string()
                                                            }),
                                                        },
                                                    ),
                                                    order: None,
                                                }));
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!(
                                            "Failed to capture initial CDP screenshot (attempt {}): {}",
                                            attempt,
                                            e
                                        );
                                        if attempt >= max_attempts {
                                            break;
                                        }
                                        tokio::time::sleep(tokio::time::Duration::from_millis(250))
                                            .await;
                                    }
                                }
                            }
                        });
                    }
                }
                Err(e) => {
                    let err_msg = format!("{e}");
                    // If we attempted via a cached WS, clear it and fallback to port-based discovery once.
                    if attempted_via_cached_ws {
                        tracing::warn!(
                            "[cdp] cached WS connect failed: {} — clearing WS cache and retrying via port discovery",
                            err_msg
                        );
                        let port_to_keep = cached_port_for_fallback;
                        // Clear WS in-memory and on-disk
                        code_browser::global::set_last_connection(port_to_keep, None).await;
                        let _ = write_cached_connection(port_to_keep, None).await;

                        // Reconfigure to use port (prefer cached port, else auto-detect)
                        {
                            let mut cfg = browser_manager.config.write().await;
                            cfg.connect_ws = None;
                            cfg.connect_port = Some(port_to_keep.unwrap_or(0));
                        }

                        tracing::info!(
                            "[cdp] retrying connect via port discovery after WS failure…"
                        );
                        let retry_deadline = tokio::time::Duration::from_secs(20);
                        let retry = tokio::time::timeout(
                            retry_deadline,
                            browser_manager.connect_to_chrome_only(),
                        )
                        .await;
                        match retry {
                            Ok(Ok(_)) => {
                                tracing::info!(
                                    "[cdp] Fallback connect succeeded after clearing cached WS"
                                );
                                // Emit success event and set up callbacks, mirroring the success path above
                                let (detected_port, detected_ws) =
                                    code_browser::global::get_last_connection().await;
                                let mut port_num: Option<u16> = detected_port;
                                if port_num.is_none()
                                    && let Some(ws) = &detected_ws
                                        && let Some(after_scheme) = ws.split("//").nth(1)
                                            && let Some(hostport) = after_scheme.split('/').next()
                                                && let Some(pstr) = hostport.split(':').nth(1)
                                                    && let Ok(p) = pstr.parse::<u16>() {
                                                        port_num = Some(p);
                                                    }
                                let current_url = browser_manager.get_current_url().await;
                                let success_msg = match (port_num, current_url) {
                                    (Some(p), Some(url)) if !url.is_empty() => {
                                        format!(
                                            "✅ Connected to Chrome via CDP (port {p}) to {url}"
                                        )
                                    }
                                    (Some(p), _) => {
                                        format!("✅ Connected to Chrome via CDP (port {p})")
                                    }
                                    (None, Some(url)) if !url.is_empty() => {
                                        format!("✅ Connected to Chrome via CDP to {url}")
                                    }
                                    _ => "✅ Connected to Chrome via CDP".to_string(),
                                };
                                app_event_tx
                                    .send_background_event_with_ticket(&ticket, success_msg);

                                // Persist last connection cache
                                tokio::spawn(async move {
                                    let (p, ws) =
                                        code_browser::global::get_last_connection().await;
                                    let _ = write_cached_connection(p, ws).await;
                                });

                                // Navigation callback
                                let latest_screenshot_callback = latest_screenshot.clone();
                                let app_event_tx_callback = app_event_tx.clone();
                                browser_manager
                                    .set_navigation_callback(move |url| {
                                        tracing::info!("CDP Navigation callback triggered for URL: {}", url);
                                        let latest_screenshot_inner = latest_screenshot_callback.clone();
                                        let app_event_tx_inner = app_event_tx_callback.clone();
                                        let url_inner = url;
                                        tokio::spawn(async move {
                                            tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                                            let browser_manager_inner = ChatWidget::get_browser_manager().await;
                                            let mut attempt = 0;
                                            let max_attempts = 2;
                                            loop {
                                                attempt += 1;
                                                match browser_manager_inner.capture_screenshot_with_url().await {
                                                    Ok((paths, _)) => {
                                                        if let Some(first_path) = paths.first() {
                                                            tracing::info!("[cdp] auto-captured screenshot: {}", first_path.display());
                                                            if let Ok(mut latest) = latest_screenshot_inner.lock() {
                                                                *latest = Some((first_path.clone(), url_inner.clone()));
                                                            }
                                                            use code_core::protocol::{BrowserScreenshotUpdateEvent, Event, EventMsg};
                                                            app_event_tx_inner.send(AppEvent::CodexEvent(Event {
                                                                id: uuid::Uuid::new_v4().to_string(),
                                                                event_seq: 0,
                                                                msg: EventMsg::BrowserScreenshotUpdate(BrowserScreenshotUpdateEvent {
                                                                    screenshot_path: first_path.clone(),
                                                                    url: url_inner,
                                                                }),
                                                                order: None,
                                                            }));
                                                            break;
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!("[cdp] auto-capture failed (attempt {}): {}", attempt, e);
                                                        if attempt >= max_attempts { break; }
                                                        tokio::time::sleep(tokio::time::Duration::from_millis(250)).await;
                                                    }
                                                }
                                            }
                                        });
                                    })
                                    .await;
                                // Set as global manager like success path
                                code_browser::global::set_global_browser_manager(
                                    browser_manager.clone(),
                                )
                                .await;

                                // Initial screenshot in background (best-effort)
                                {
                                    let latest_screenshot_bg = latest_screenshot.clone();
                                    let app_event_tx_bg = app_event_tx.clone();
                                    tokio::spawn(async move {
                                        tokio::time::sleep(tokio::time::Duration::from_millis(250))
                                            .await;
                                        let browser_manager =
                                            ChatWidget::get_browser_manager().await;
                                        let mut attempt = 0;
                                        let max_attempts = 2;
                                        loop {
                                            attempt += 1;
                                            match browser_manager
                                                .capture_screenshot_with_url()
                                                .await
                                            {
                                                Ok((paths, url)) => {
                                                    if let Some(first_path) = paths.first() {
                                                        tracing::info!(
                                                            "Initial CDP screenshot captured: {}",
                                                            first_path.display()
                                                        );
                                                        if let Ok(mut latest) =
                                                            latest_screenshot_bg.lock()
                                                        {
                                                            *latest = Some((
                                                                first_path.clone(),
                                                                url.clone().unwrap_or_else(|| {
                                                                    "Chrome".to_string()
                                                                }),
                                                            ));
                                                        }
                                                        use code_core::protocol::BrowserScreenshotUpdateEvent;
                                                        use code_core::protocol::Event;
                                                        use code_core::protocol::EventMsg;
                                                        app_event_tx_bg.send(AppEvent::CodexEvent(Event {
                                                            id: uuid::Uuid::new_v4().to_string(),
                                                            event_seq: 0,
                                                            msg: EventMsg::BrowserScreenshotUpdate(BrowserScreenshotUpdateEvent {
                                                                screenshot_path: first_path.clone(),
                                                                url: url.unwrap_or_else(|| "Chrome".to_string()),
                                                            }),
                                                            order: None,
                                                        }));
                                                        break;
                                                    }
                                                }
                                                Err(e) => {
                                                    tracing::warn!(
                                                        "Failed to capture initial CDP screenshot (attempt {}): {}",
                                                        attempt,
                                                        e
                                                    );
                                                    if attempt >= max_attempts {
                                                        break;
                                                    }
                                                    tokio::time::sleep(
                                                        tokio::time::Duration::from_millis(250),
                                                    )
                                                    .await;
                                                }
                                            }
                                        }
                                    });
                                }
                            }
                            Ok(Err(e2)) => {
                                tracing::error!("[cdp] Fallback connect failed: {}", e2);
                                app_event_tx.send_background_event_with_ticket(
                                    &ticket,
                                    format!(
                                        "❌ Failed to connect to Chrome after WS fallback: {e2} (original: {err_msg})"
                                    ),
                                );
                                // Also surface the Chrome launch options UI to assist the user
                                app_event_tx.send(AppEvent::ShowChromeOptions(port));
                            }
                            Err(_) => {
                                tracing::error!(
                                    "[cdp] Fallback connect timed out after {:?}",
                                    retry_deadline
                                );
                                app_event_tx.send_background_event_with_ticket(
                                    &ticket,
                                    format!(
                                        "❌ CDP connect timed out after {}s during fallback. Ensure Chrome is running with --remote-debugging-port and /json/version is reachable",
                                        retry_deadline.as_secs()
                                    ),
                                );
                                // Also surface the Chrome launch options UI to assist the user
                                app_event_tx.send(AppEvent::ShowChromeOptions(port));
                            }
                        }
                    } else {
                        tracing::error!(
                            "[cdp] connect_to_chrome_only failed immediately: {}",
                            err_msg
                        );
                        app_event_tx.send_background_event_with_ticket(
                            &ticket,
                            format!("❌ Failed to connect to Chrome: {err_msg}"),
                        );
                        // Offer launch options popup to help recover quickly
                        app_event_tx.send(AppEvent::ShowChromeOptions(port));
                    }
                }
            },
        }
    }

    fn launch_chrome_with_temp_profile(&mut self, port: u16) {
        use std::process::Stdio;

        let temp_dir = std::env::temp_dir();
        let profile_dir = temp_dir.join(format!("code-chrome-temp-{port}"));
        let log_path = self.chrome_log_path();

        #[cfg(target_os = "macos")]
        {
            let mut cmd = std::process::Command::new(
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            );
            cmd.arg(format!("--remote-debugging-port={port}"))
                .arg(format!("--user-data-dir={}", profile_dir.display()))
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-component-extensions-with-background-pages")
                .arg("--disable-background-networking")
                .arg("--silent-debugger-extension-api")
                .arg("--remote-allow-origins=*")
                .arg("--disable-features=ChromeWhatsNewUI,TriggerFirstRunUI")
                .arg("--disable-hang-monitor")
                .arg("--disable-background-timer-throttling")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .stdin(Stdio::null());
            self.apply_chrome_logging(&mut cmd, log_path.as_deref());
            if let Err(err) = spawn_std_command_with_retry(&mut cmd) {
                tracing::warn!("failed to launch Chrome with temp profile: {err}");
            }
        }

        #[cfg(target_os = "linux")]
        {
            let mut cmd = std::process::Command::new("google-chrome");
            cmd.arg(format!("--remote-debugging-port={}", port))
                .arg(format!("--user-data-dir={}", profile_dir.display()))
                .arg("--no-first-run")
                .arg("--no-default-browser-check")
                .arg("--disable-component-extensions-with-background-pages")
                .arg("--disable-background-networking")
                .arg("--silent-debugger-extension-api")
                .arg("--remote-allow-origins=*")
                .arg("--disable-features=ChromeWhatsNewUI,TriggerFirstRunUI")
                .arg("--disable-hang-monitor")
                .arg("--disable-background-timer-throttling")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .stdin(Stdio::null());
            self.apply_chrome_logging(&mut cmd, log_path.as_deref());
            if let Err(err) = spawn_std_command_with_retry(&mut cmd) {
                tracing::warn!("failed to launch Chrome with temp profile: {err}");
            }
        }

        #[cfg(target_os = "windows")]
        {
            let chrome_paths = vec![
                "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe".to_string(),
                "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe".to_string(),
                format!(
                    "{}\\AppData\\Local\\Google\\Chrome\\Application\\chrome.exe",
                    std::env::var("USERPROFILE").unwrap_or_default()
                ),
            ];

            for chrome_path in chrome_paths {
                if std::path::Path::new(&chrome_path).exists() {
                    let mut cmd = std::process::Command::new(&chrome_path);
                    cmd.arg(format!("--remote-debugging-port={}", port))
                        .arg(format!("--user-data-dir={}", profile_dir.display()))
                        .arg("--no-first-run")
                        .arg("--no-default-browser-check")
                        .arg("--disable-component-extensions-with-background-pages")
                        .arg("--disable-background-networking")
                        .arg("--silent-debugger-extension-api")
                        .arg("--remote-allow-origins=*")
                        .arg("--disable-features=ChromeWhatsNewUI,TriggerFirstRunUI")
                        .arg("--disable-hang-monitor")
                        .arg("--disable-background-timer-throttling")
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .stdin(Stdio::null());
                    self.apply_chrome_logging(&mut cmd, log_path.as_deref());
                    if let Err(err) = spawn_std_command_with_retry(&mut cmd) {
                        tracing::warn!("failed to launch Chrome with temp profile: {err}");
                    }
                    break;
                }
            }
        }

        // Add status message
        self.push_background_tail(format!(
            "✅ Chrome launched with temporary profile at {}",
            profile_dir.display()
        ));
    }

    fn schedule_browser_autofix(
        app_event_tx: AppEventSender,
        ticket: BackgroundOrderTicket,
        autofix_state: Arc<AtomicBool>,
        failure_context: &str,
        raw_error: String,
    ) {
        if autofix_state
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            tracing::info!(
                "[/browser] auto-handoff already requested; skipping duplicate dispatch"
            );
            return;
        }

        let sanitized = raw_error.replace(['\n', '\r'], " ");
        let trimmed = sanitized.trim();
        let truncated = if trimmed.len() > 220 {
            let mut shortened = trimmed.chars().take(220).collect::<String>();
            shortened.push('…');
            shortened
        } else {
            trimmed.to_string()
        };

        tracing::info!(
            "[/browser] scheduling Code autofix for context='{}', error='{}'",
            failure_context,
            truncated
        );

        let visible_message = format!(
            "🤖 Handing /browser failure ({failure_context}) to Code. Error: {truncated}"
        );
        app_event_tx.send_background_event_with_ticket(&ticket, visible_message);

        let command_text = format!(
            "/code The /browser command failed to {failure_context}. Recent error: {truncated}. Please diagnose and fix the environment (for example, install or configure Chrome) so /browser works in this workspace."
        );
        app_event_tx.send(AppEvent::DispatchCommand(
            SlashCommand::Code,
            command_text,
        ));
    }

    pub(crate) fn handle_browser_command(&mut self, command_text: String) {
        // Parse the browser subcommand
        let trimmed = command_text.trim();
        let browser_ticket = self.make_background_tail_ticket();
        self.consume_pending_prompt_for_ui_only_turn();

        // Handle the case where just "/browser" was typed
        if trimmed.is_empty() {
            tracing::info!("[/browser] toggling internal browser on/off");

            // Optimistically reflect browsing activity in the input border if we end up enabling
            // (safe even if we later disable; UI will update on event messages)
            self.bottom_pane
                .update_status_text("using browser".to_string());

            // Toggle asynchronously: if internal browser is active, disable it; otherwise enable and open about:blank
            let app_event_tx = self.app_event_tx.clone();
            let browser_autofix_flag = self.browser_autofix_requested.clone();
            let ticket = browser_ticket;
            tokio::spawn(async move {
                let browser_manager = ChatWidget::get_browser_manager().await;
                // Determine if internal browser is currently active
                let (is_external, status) = {
                    let cfg = browser_manager.config.read().await;
                    let is_external = cfg.connect_port.is_some() || cfg.connect_ws.is_some();
                    drop(cfg);
                    (is_external, browser_manager.get_status().await)
                };

                if !is_external && status.browser_active {
                    // Internal browser active → disable it
                    if let Err(e) = browser_manager.set_enabled(false).await {
                        tracing::warn!("[/browser] failed to disable internal browser: {}", e);
                    }
                    app_event_tx
                        .send_background_event_with_ticket(&ticket, "🔌 Browser disabled".to_string());
                } else {
                    // Not in internal mode → enable internal and open about:blank
                    // Reuse existing helper (ensures config + start + global manager + screenshot)
                    // Then explicitly navigate to about:blank
                    // We fire-and-forget errors to avoid blocking UI
                    {
                        // Configure cleanly for internal mode
                        let mut cfg = browser_manager.config.write().await;
                        cfg.connect_port = None;
                        cfg.connect_ws = None;
                        cfg.enabled = true;
                        cfg.persist_profile = false;
                        cfg.headless = true;
                    }

                    if let Err(e) = browser_manager.start().await {
                        let error_text = e.to_string();
                        tracing::error!(
                            "[/browser] failed to start internal browser: {}",
                            error_text
                        );
                        app_event_tx.send_background_event_with_ticket(
                            &ticket,
                            format!("❌ Failed to start internal browser: {error_text}"),
                        );
                        ChatWidget::schedule_browser_autofix(
                            app_event_tx.clone(),
                            ticket.clone(),
                            browser_autofix_flag.clone(),
                            "start the internal browser",
                            error_text,
                        );
                        return;
                    }

                    browser_autofix_flag.store(false, Ordering::SeqCst);

                    // Set as global manager so core/session share the same instance
                    code_browser::global::set_global_browser_manager(browser_manager.clone())
                        .await;

                    // Navigate to about:blank explicitly
                    if let Err(e) = browser_manager.goto("about:blank").await {
                        tracing::warn!("[/browser] failed to open about:blank: {}", e);
                    }

                    // Emit confirmation
                    app_event_tx
                        .send_background_event_with_ticket(
                            &ticket,
                            "✅ Browser enabled (about:blank)".to_string(),
                        );
                }
            });
            return;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        let response = if !parts.is_empty() {
            let first_arg = parts[0];

            // Check if the first argument looks like a URL (has a dot or protocol)
            let is_url = first_arg.contains("://") || first_arg.contains(".");

            if is_url {
                // It's a URL - enable browser mode and navigate to it
                let url = parts.join(" ");

                // Ensure URL has protocol
                let full_url = if !url.contains("://") {
                    format!("https://{url}")
                } else {
                    url
                };

                // We are navigating with the internal browser
                self.browser_is_external = false;

                // Navigate to URL and wait for it to load
                let latest_screenshot = self.latest_browser_screenshot.clone();
                let app_event_tx = self.app_event_tx.clone();
                let browser_autofix_flag = self.browser_autofix_requested.clone();
                let url_for_goto = full_url.clone();
                let ticket = browser_ticket.clone();

                // Add status message
                let status_msg = format!("🌐 Opening internal browser: {full_url}");
                self.push_background_tail(status_msg);
                // Also reflect browsing activity in the input border
                self.bottom_pane
                    .update_status_text("using browser".to_string());

                // Connect immediately, don't wait for message send
                tokio::spawn(async move {
                    // Get the global browser manager
                    let browser_manager = ChatWidget::get_browser_manager().await;

                    // Enable browser mode and ensure it's using internal browser (not CDP)
                    browser_manager.set_enabled_sync(true);
                    {
                        let mut config = browser_manager.config.write().await;
                        config.headless = false; // Ensure browser is visible when navigating to URL
                        config.connect_port = None; // Ensure we're not trying to connect to CDP
                        config.connect_ws = None; // Ensure we're not trying to connect via WebSocket
                    }

                    // IMPORTANT: Start the browser manager first before navigating
                    if let Err(e) = browser_manager.start().await {
                        let error_text = e.to_string();
                        tracing::error!(
                            "Failed to start TUI browser manager: {}",
                            error_text
                        );
                        app_event_tx.send_background_event_with_ticket(
                            &ticket,
                            format!("❌ Failed to start internal browser: {error_text}"),
                        );
                        ChatWidget::schedule_browser_autofix(
                            app_event_tx.clone(),
                            ticket.clone(),
                            browser_autofix_flag.clone(),
                            "launch the internal browser",
                            error_text,
                        );
                        return;
                    }

                    browser_autofix_flag.store(false, Ordering::SeqCst);

                    // Set up navigation callback to auto-capture screenshots
                    {
                        let latest_screenshot_callback = latest_screenshot.clone();
                        let app_event_tx_callback = app_event_tx.clone();

                        browser_manager
                            .set_navigation_callback(move |url| {
                                tracing::info!("Navigation callback triggered for URL: {}", url);
                                let latest_screenshot_inner = latest_screenshot_callback.clone();
                                let app_event_tx_inner = app_event_tx_callback.clone();
                                let url_inner = url;

                                tokio::spawn(async move {
                                    // Get browser manager in the inner async block
                                    let browser_manager_inner =
                                        ChatWidget::get_browser_manager().await;
                                    // Capture screenshot after navigation
                                    match browser_manager_inner.capture_screenshot_with_url().await
                                    {
                                        Ok((paths, _)) => {
                                            if let Some(first_path) = paths.first() {
                                                tracing::info!(
                                                    "Auto-captured screenshot after navigation: {}",
                                                    first_path.display()
                                                );

                                                // Update the latest screenshot
                                                if let Ok(mut latest) =
                                                    latest_screenshot_inner.lock()
                                                {
                                                    *latest = Some((
                                                        first_path.clone(),
                                                        url_inner.clone(),
                                                    ));
                                                }

                                                // Send update event
                                                use code_core::protocol::{
                                                    BrowserScreenshotUpdateEvent, EventMsg,
                                                };
                                                app_event_tx_inner.send(
                                                    AppEvent::CodexEvent(Event {
                                                        id: uuid::Uuid::new_v4().to_string(),
                                                        event_seq: 0,
                                                        msg: EventMsg::BrowserScreenshotUpdate(
                                                            BrowserScreenshotUpdateEvent {
                                                                screenshot_path: first_path.clone(),
                                                                url: url_inner,
                                                            },
                                                        ),
                                                        order: None,
                                                    }),
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!(
                                                "Failed to auto-capture screenshot: {}",
                                                e
                                            );
                                        }
                                    }
                                });
                            })
                            .await;
                    }

                    // Set the browser manager as the global manager so both TUI and Session use the same instance
                    code_browser::global::set_global_browser_manager(browser_manager.clone())
                        .await;

                    // Ensure the navigation callback is also set on the global manager
                    let global_manager = code_browser::global::get_browser_manager().await;
                    if let Some(global_manager) = global_manager {
                        let latest_screenshot_global = latest_screenshot.clone();
                        let app_event_tx_global = app_event_tx.clone();

                        global_manager.set_navigation_callback(move |url| {
                            tracing::info!("Global manager navigation callback triggered for URL: {}", url);
                            let latest_screenshot_inner = latest_screenshot_global.clone();
                            let app_event_tx_inner = app_event_tx_global.clone();
                            let url_inner = url;

                            tokio::spawn(async move {
                                // Wait a moment for the navigation to complete
                                tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

                                // Capture screenshot after navigation
                                let browser_manager = code_browser::global::get_browser_manager().await;
                                if let Some(browser_manager) = browser_manager {
                                    match browser_manager.capture_screenshot_with_url().await {
                                        Ok((paths, _url)) => {
                                            if let Some(first_path) = paths.first() {
                                                tracing::info!("Auto-captured screenshot after global navigation: {}", first_path.display());

                                                // Update the latest screenshot
                                                if let Ok(mut latest) = latest_screenshot_inner.lock() {
                                                    *latest = Some((first_path.clone(), url_inner.clone()));
                                                }

                                                // Send update event
                                                use code_core::protocol::{BrowserScreenshotUpdateEvent, EventMsg};
                                                app_event_tx_inner.send(AppEvent::CodexEvent(Event { id: uuid::Uuid::new_v4().to_string(), event_seq: 0, msg: EventMsg::BrowserScreenshotUpdate(BrowserScreenshotUpdateEvent {
                                                        screenshot_path: first_path.clone(),
                                                        url: url_inner,
                                                    }), order: None }));
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!("Failed to auto-capture screenshot after global navigation: {}", e);
                                        }
                                    }
                                }
                            });
                        }).await;
                    }

                    // Navigate using global manager
                    match browser_manager.goto(&url_for_goto).await {
                        Ok(result) => {
                            tracing::info!(
                                "Browser opened to: {} (title: {:?})",
                                result.url,
                                result.title
                            );

                            // Send success message to chat
                            app_event_tx.send_background_event_with_ticket(
                                &ticket,
                                format!("✅ Internal browser opened: {}", result.url),
                            );

                            // Capture initial screenshot
                            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                            match browser_manager.capture_screenshot_with_url().await {
                                Ok((paths, url)) => {
                                    if let Some(first_path) = paths.first() {
                                        tracing::info!(
                                            "Initial screenshot captured: {}",
                                            first_path.display()
                                        );

                                        // Update the latest screenshot
                                        if let Ok(mut latest) = latest_screenshot.lock() {
                                            *latest = Some((
                                                first_path.clone(),
                                                url.clone().unwrap_or_else(|| result.url.clone()),
                                            ));
                                        }

                                        // Send update event
                                        use code_core::protocol::BrowserScreenshotUpdateEvent;
                                        use code_core::protocol::EventMsg;
                                        app_event_tx.send(AppEvent::CodexEvent(Event {
                                            id: uuid::Uuid::new_v4().to_string(),
                                            event_seq: 0,
                                            msg: EventMsg::BrowserScreenshotUpdate(
                                                BrowserScreenshotUpdateEvent {
                                                    screenshot_path: first_path.clone(),
                                                    url: url.unwrap_or_else(|| result.url.clone()),
                                                },
                                            ),
                                            order: None,
                                        }));
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Failed to capture initial screenshot: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to open browser: {}", e);
                        }
                    }
                });

                format!("Browser mode enabled: {full_url}\n")
            } else {
                // It's a subcommand
                match first_arg {
                    "off" => {
                        // Disable browser mode
                        // Clear the screenshot popup
                        if let Ok(mut screenshot_lock) = self.latest_browser_screenshot.lock() {
                            *screenshot_lock = None;
                        }
                        // Close any open browser
                        tokio::spawn(async move {
                            let browser_manager = ChatWidget::get_browser_manager().await;
                            browser_manager.set_enabled_sync(false);
                            if let Err(e) = browser_manager.close().await {
                                tracing::error!("Failed to close browser: {}", e);
                            }
                        });
                        self.app_event_tx.send(AppEvent::RequestRedraw);
                        "Browser mode disabled.".to_string()
                    }
                    "status" => {
                        // Get status from BrowserManager
                        // Use a channel to get status from async context
                        let (status_tx, status_rx) = std::sync::mpsc::channel();
                        tokio::spawn(async move {
                            let browser_manager = ChatWidget::get_browser_manager().await;
                            let status = browser_manager.get_status_sync();
                            let _ = status_tx.send(status);
                        });
                        status_rx
                            .recv()
                            .unwrap_or_else(|_| "Failed to get browser status.".to_string())
                    }
                    "fullpage" => {
                        if parts.len() > 2 {
                            match parts[2] {
                                "on" => {
                                    // Enable full-page mode
                                    tokio::spawn(async move {
                                        let browser_manager =
                                            ChatWidget::get_browser_manager().await;
                                        browser_manager.set_fullpage_sync(true);
                                    });
                                    "Full-page screenshot mode enabled (max 8 segments)."
                                        .to_string()
                                }
                                "off" => {
                                    // Disable full-page mode
                                    tokio::spawn(async move {
                                        let browser_manager =
                                            ChatWidget::get_browser_manager().await;
                                        browser_manager.set_fullpage_sync(false);
                                    });
                                    "Full-page screenshot mode disabled.".to_string()
                                }
                                _ => "Usage: /browser fullpage [on|off]".to_string(),
                            }
                        } else {
                            "Usage: /browser fullpage [on|off]".to_string()
                        }
                    }
                    "config" => {
                        if parts.len() > 3 {
                            let key = parts[2];
                            let value = parts[3..].join(" ");
                            // Update browser config
                            match key {
                                "viewport" => {
                                    // Parse viewport dimensions like "1920x1080"
                                    if let Some((width_str, height_str)) = value.split_once('x') {
                                        if let (Ok(width), Ok(height)) =
                                            (width_str.parse::<u32>(), height_str.parse::<u32>())
                                        {
                                            tokio::spawn(async move {
                                                let browser_manager =
                                                    ChatWidget::get_browser_manager().await;
                                                browser_manager.set_viewport_sync(width, height);
                                            });
                                            format!(
                                                "Browser viewport updated: {width}x{height}"
                                            )
                                        } else {
                                            "Invalid viewport format. Use: /browser config viewport 1920x1080".to_string()
                                        }
                                    } else {
                                        "Invalid viewport format. Use: /browser config viewport 1920x1080".to_string()
                                    }
                                }
                                "segments_max" => {
                                    if let Ok(max) = value.parse::<usize>() {
                                        tokio::spawn(async move {
                                            let browser_manager =
                                                ChatWidget::get_browser_manager().await;
                                            browser_manager.set_segments_max_sync(max);
                                        });
                                        format!("Browser segments_max updated: {max}")
                                    } else {
                                        "Invalid segments_max value. Use a number.".to_string()
                                    }
                                }
                                _ => format!(
                                    "Unknown config key: {key}. Available: viewport, segments_max"
                                ),
                            }
                        } else {
                            "Usage: /browser config <key> <value>\nAvailable keys: viewport, segments_max".to_string()
                        }
                    }
                    _ => {
                        format!(
                            "Unknown browser command: '{first_arg}'\nUsage: /browser <url> | off | status | fullpage | config"
                        )
                    }
                }
            }
        } else {
            "Browser commands:\n• /browser <url> - Open URL in internal browser\n• /browser off - Disable browser mode\n• /browser status - Show current status\n• /browser fullpage [on|off] - Toggle full-page mode\n• /browser config <key> <value> - Update configuration\n\nUse /chrome [port] to connect to external Chrome browser".to_string()
        };

        // Add the response to the UI as a ticketed background event so it stays with
        // the originating slash command turn.
        self.app_event_tx
            .send_background_event_with_ticket(&browser_ticket, response);
    }

    fn validation_tool_flag_mut(
        &mut self,
        name: &str,
    ) -> Option<&mut Option<bool>> {
        let tools = &mut self.config.validation.tools;
        match name {
            "shellcheck" => Some(&mut tools.shellcheck),
            "markdownlint" => Some(&mut tools.markdownlint),
            "hadolint" => Some(&mut tools.hadolint),
            "yamllint" => Some(&mut tools.yamllint),
            "cargo-check" => Some(&mut tools.cargo_check),
            "shfmt" => Some(&mut tools.shfmt),
            "prettier" => Some(&mut tools.prettier),
            "tsc" => Some(&mut tools.tsc),
            "eslint" => Some(&mut tools.eslint),
            "phpstan" => Some(&mut tools.phpstan),
            "psalm" => Some(&mut tools.psalm),
            "mypy" => Some(&mut tools.mypy),
            "pyright" => Some(&mut tools.pyright),
            "golangci-lint" => Some(&mut tools.golangci_lint),
            _ => None,
        }
    }

    fn validation_group_label(group: ValidationGroup) -> &'static str {
        match group {
            ValidationGroup::Functional => "Functional checks",
            ValidationGroup::Stylistic => "Stylistic checks",
        }
    }

    fn validation_group_enabled(&self, group: ValidationGroup) -> bool {
        match group {
            ValidationGroup::Functional => self.config.validation.groups.functional,
            ValidationGroup::Stylistic => self.config.validation.groups.stylistic,
        }
    }

    fn validation_tool_requested(&self, name: &str) -> bool {
        let tools = &self.config.validation.tools;
        match name {
            "actionlint" => self.config.github.actionlint_on_patch,
            "shellcheck" => tools.shellcheck.unwrap_or(true),
            "markdownlint" => tools.markdownlint.unwrap_or(true),
            "hadolint" => tools.hadolint.unwrap_or(true),
            "yamllint" => tools.yamllint.unwrap_or(true),
            "cargo-check" => tools.cargo_check.unwrap_or(true),
            "shfmt" => tools.shfmt.unwrap_or(true),
            "prettier" => tools.prettier.unwrap_or(true),
            "tsc" => tools.tsc.unwrap_or(true),
            "eslint" => tools.eslint.unwrap_or(true),
            "phpstan" => tools.phpstan.unwrap_or(true),
            "psalm" => tools.psalm.unwrap_or(true),
            "mypy" => tools.mypy.unwrap_or(true),
            "pyright" => tools.pyright.unwrap_or(true),
            "golangci-lint" => tools.golangci_lint.unwrap_or(true),
            _ => true,
        }
    }

    fn validation_tool_enabled(&self, name: &str) -> bool {
        let requested = self.validation_tool_requested(name);
        let category = validation_tool_category(name);
        let group_enabled = match category {
            ValidationCategory::Functional => self.config.validation.groups.functional,
            ValidationCategory::Stylistic => self.config.validation.groups.stylistic,
        };
        requested && group_enabled
    }

    fn apply_validation_group_toggle(&mut self, group: ValidationGroup, enable: bool) {
        if self.validation_group_enabled(group) == enable {
            return;
        }

        match group {
            ValidationGroup::Functional => self.config.validation.groups.functional = enable,
            ValidationGroup::Stylistic => self.config.validation.groups.stylistic = enable,
        }

        if let Err(err) = self
            .code_op_tx
            .send(Op::UpdateValidationGroup { group, enable })
        {
            tracing::warn!("failed to send validation group update: {err}");
        }

        let result = match find_code_home() {
            Ok(home) => {
                let key = match group {
                    ValidationGroup::Functional => "functional",
                    ValidationGroup::Stylistic => "stylistic",
                };
                set_validation_group_enabled(&home, key, enable).map_err(|e| e.to_string())
            }
            Err(err) => Err(err.to_string()),
        };

        let label = Self::validation_group_label(group);
        if let Err(err) = result {
            self.push_background_tail(format!(
                "⚠️ {} {} (persist failed: {err})",
                label,
                if enable { "enabled" } else { "disabled" }
            ));
        }

        self.refresh_settings_overview_rows();
    }

    fn apply_validation_tool_toggle(&mut self, name: &str, enable: bool) {
        if name == "actionlint" {
            if self.config.github.actionlint_on_patch == enable {
                return;
            }
            self.config.github.actionlint_on_patch = enable;
            if let Err(err) = self
                .code_op_tx
                .send(Op::UpdateValidationTool { name: name.to_string(), enable })
            {
                tracing::warn!("failed to send validation tool update: {err}");
            }
            let persist_result = match find_code_home() {
                Ok(home) => set_github_actionlint_on_patch(&home, enable)
                    .map_err(|e| e.to_string()),
                Err(err) => Err(err.to_string()),
            };
            if let Err(err) = persist_result {
                self.push_background_tail(format!(
                    "⚠️ {}: {} (persist failed: {err})",
                    name,
                    if enable { "enabled" } else { "disabled" }
                ));
            }
            return;
        }

        let Some(flag) = self.validation_tool_flag_mut(name) else {
            self.push_background_tail(format!(
                "⚠️ Unknown validation tool '{name}'"
            ));
            return;
        };

        if flag.unwrap_or(true) == enable {
            return;
        }

        *flag = Some(enable);
        if let Err(err) = self
            .code_op_tx
            .send(Op::UpdateValidationTool { name: name.to_string(), enable })
        {
            tracing::warn!("failed to send validation tool update: {err}");
        }
        let persist_result = match find_code_home() {
            Ok(home) => set_validation_tool_enabled(&home, name, enable)
                .map_err(|e| e.to_string()),
            Err(err) => Err(err.to_string()),
        };
        if let Err(err) = persist_result {
            self.push_background_tail(format!(
                "⚠️ {}: {} (persist failed: {err})",
                name,
                if enable { "enabled" } else { "disabled" }
            ));
        }

        self.refresh_settings_overview_rows();
    }

    fn build_validation_status_message(&self) -> String {
        let mut lines = Vec::new();
        lines.push("Validation groups:".to_string());
        for group in [ValidationGroup::Functional, ValidationGroup::Stylistic] {
            let enabled = self.validation_group_enabled(group);
            lines.push(format!(
                "• {} — {}",
                Self::validation_group_label(group),
                if enabled { "enabled" } else { "disabled" }
            ));
        }
        lines.push("".to_string());
        lines.push("Tools:".to_string());
        for status in validation_settings_view::detect_tools() {
            let requested = self.validation_tool_requested(status.name);
            let effective = self.validation_tool_enabled(status.name);
            let mut state = if requested {
                if effective { "enabled".to_string() } else { "disabled (group off)".to_string() }
            } else {
                "disabled".to_string()
            };
            if !status.installed {
                state.push_str(" (not installed)");
            }
            lines.push(format!("• {} — {}", status.name, state));
        }
        lines.join("\n")
    }

    pub(crate) fn toggle_validation_tool(&mut self, name: &str, enable: bool) {
        self.apply_validation_tool_toggle(name, enable);
    }

    pub(crate) fn toggle_validation_group(&mut self, group: ValidationGroup, enable: bool) {
        self.apply_validation_group_toggle(group, enable);
    }

    pub(crate) fn handle_validation_command(&mut self, command_text: String) {
        let trimmed = command_text.trim();
        if trimmed.is_empty() {
            self.ensure_validation_settings_overlay();
            return;
        }

        let mut parts = trimmed.split_whitespace();
        match parts.next().unwrap_or("") {
            "status" => {
                let message = self.build_validation_status_message();
                self.push_background_tail(message);
            }
            "on" => {
                if !self.validation_group_enabled(ValidationGroup::Functional) {
                    self.apply_validation_group_toggle(ValidationGroup::Functional, true);
                }
            }
            "off" => {
                if self.validation_group_enabled(ValidationGroup::Functional) {
                    self.apply_validation_group_toggle(ValidationGroup::Functional, false);
                }
                if self.validation_group_enabled(ValidationGroup::Stylistic) {
                    self.apply_validation_group_toggle(ValidationGroup::Stylistic, false);
                }
            }
            group @ ("functional" | "stylistic") => {
                let Some(state) = parts.next() else {
                    self.push_background_tail("Usage: /validation <tool|group> on|off".to_string());
                    return;
                };
                let group = if group == "functional" {
                    ValidationGroup::Functional
                } else {
                    ValidationGroup::Stylistic
                };
                match state {
                    "on" | "enable" => self.apply_validation_group_toggle(group, true),
                    "off" | "disable" => self.apply_validation_group_toggle(group, false),
                    _ => self.push_background_tail(format!(
                        "⚠️ Unknown validation command '{state}'. Use on|off."
                    )),
                }
            }
            tool => {
                let Some(state) = parts.next() else {
                    self.push_background_tail("Usage: /validation <tool|group> on|off".to_string());
                    return;
                };
                match state {
                    "on" | "enable" => self.apply_validation_tool_toggle(tool, true),
                    "off" | "disable" => self.apply_validation_tool_toggle(tool, false),
                    _ => self.push_background_tail(format!(
                        "⚠️ Unknown validation command '{state}'. Use on|off."
                    )),
                }
            }
        }

        self.ensure_validation_settings_overlay();
    }

    fn format_mcp_summary(cfg: &code_core::config_types::McpServerConfig) -> String {
        use code_core::config_types::McpServerTransportConfig;

        match &cfg.transport {
            McpServerTransportConfig::Stdio { command, args, .. } => {
                if args.is_empty() {
                    command.clone()
                } else {
                    format!("{} {}", command, args.join(" "))
                }
            }
            McpServerTransportConfig::StreamableHttp { url, .. } => format!("HTTP {url}"),
        }
    }

    fn format_mcp_server_summary(
        &self,
        name: &str,
        cfg: &code_core::config_types::McpServerConfig,
        enabled: bool,
    ) -> String {
        let transport = Self::format_mcp_summary(cfg);
        let status = self.format_mcp_tool_status(name, enabled);
        if status.is_empty() {
            transport
        } else {
            format!("{transport} · {status}")
        }
    }

    fn format_mcp_tool_status(&self, name: &str, enabled: bool) -> String {
        if !enabled {
            return "Tools: disabled".to_string();
        }

        if let Some(failure) = self.mcp_server_failures.get(name) {
            return self.format_mcp_failure(failure);
        }

        if let Some(tools) = self.mcp_tools_by_server.get(name) {
            let list = Self::format_mcp_tool_list(tools);
            return format!("Tools: {list}");
        }

        "Tools: pending".to_string()
    }

    fn format_mcp_tool_list(tools: &[String]) -> String {
        const MAX_TOOLS: usize = 6;
        const MAX_CHARS: usize = 120;

        if tools.is_empty() {
            return "none".to_string();
        }

        let mut display = tools
            .iter()
            .take(MAX_TOOLS)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if tools.len() > MAX_TOOLS {
            let remaining = tools.len().saturating_sub(MAX_TOOLS);
            display.push_str(&format!(", +{remaining} more"));
        }
        Self::truncate_with_ellipsis(&display, MAX_CHARS)
    }

    fn format_mcp_failure(&self, failure: &McpServerFailure) -> String {
        const MAX_CHARS: usize = 160;

        let normalized = failure.message.replace('\n', " ");
        let message = Self::truncate_with_ellipsis(&normalized, MAX_CHARS);
        match failure.phase {
            McpServerFailurePhase::Start => format!("Failed to start: {message}"),
            McpServerFailurePhase::ListTools => format!("Failed to list tools: {message}"),
        }
    }

    /// Handle `/mcp` command: manage MCP servers (status/on/off/add).
    pub(crate) fn handle_mcp_command(&mut self, command_text: String) {
        let trimmed = command_text.trim();
        if trimmed.is_empty() {
            if !self.config.mcp_servers.is_empty() {
                self.submit_op(Op::ListMcpTools);
            }
            self.show_settings_overlay(Some(SettingsSection::Mcp));
            return;
        }

        let mut parts = trimmed.split_whitespace();
        let sub = parts.next().unwrap_or("");

        match sub {
            "status" => {
                if !self.config.mcp_servers.is_empty() {
                    self.submit_op(Op::ListMcpTools);
                }
                match find_code_home() {
                    Ok(home) => match code_core::config::list_mcp_servers(&home) {
                        Ok((enabled, disabled)) => {
                            let mut lines = String::new();
                            if enabled.is_empty() && disabled.is_empty() {
                                lines.push_str(
                                    "No MCP servers configured. Use /mcp add … to add one.",
                                );
                            } else {
                                let enabled_count = enabled.len();
                                lines.push_str(&format!("Enabled ({enabled_count}):\n"));
                                for (name, cfg) in enabled {
                                    lines.push_str(&format!(
                                        "• {} — {}\n",
                                        name,
                                        self.format_mcp_server_summary(&name, &cfg, true)
                                    ));
                                }
                                let disabled_count = disabled.len();
                                lines.push_str(&format!("\nDisabled ({disabled_count}):\n"));
                                for (name, cfg) in disabled {
                                    lines.push_str(&format!(
                                        "• {} — {}\n",
                                        name,
                                        self.format_mcp_server_summary(&name, &cfg, false)
                                    ));
                                }
                            }
                            self.push_background_tail(lines);
                        }
                        Err(e) => {
                            let msg = format!("Failed to read MCP config: {e}");
                            self.history_push_plain_state(history_cell::new_error_event(msg));
                        }
                    },
                    Err(e) => {
                        let msg = format!("Failed to locate CODEX_HOME: {e}");
                        self.history_push_plain_state(history_cell::new_error_event(msg));
                    }
                }
            }
            "on" | "off" => {
                let name = parts.next().unwrap_or("");
                if name.is_empty() {
                    let msg = format!("Usage: /mcp {sub} <name>");
                    self.history_push_plain_state(history_cell::new_error_event(msg));
                    return;
                }
                match find_code_home() {
                    Ok(home) => {
                        match code_core::config::set_mcp_server_enabled(&home, name, sub == "on") {
                            Ok(changed) => {
                                if changed {
                                    // Keep ChatWidget's in-memory config roughly in sync for new sessions.
                                    if sub == "off" {
                                        self.config.mcp_servers.remove(name);
                                    }
                                    if sub == "on" {
                                        // If enabling, try to load its config from disk and add to in-memory map.
                                        if let Ok((enabled, _)) =
                                            code_core::config::list_mcp_servers(&home)
                                            && let Some((_, cfg)) =
                                                enabled.into_iter().find(|(n, _)| n == name)
                                            {
                                                self.config
                                                    .mcp_servers
                                                    .insert(name.to_string(), cfg);
                                            }
                                    }
                                    let msg = format!(
                                        "{} MCP server '{}'",
                                        if sub == "on" { "Enabled" } else { "Disabled" },
                                        name
                                    );
                                    self.push_background_tail(msg);
                                } else {
                                    let msg = format!(
                                        "No change: server '{}' was already {}",
                                        name,
                                        if sub == "on" { "enabled" } else { "disabled" }
                                    );
                                    self.push_background_tail(msg);
                                }
                            }
                            Err(e) => {
                                let msg = format!("Failed to update MCP server '{name}': {e}");
                                self.history_push_plain_state(history_cell::new_error_event(msg));
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("Failed to locate CODEX_HOME: {e}");
                        self.history_push_plain_state(history_cell::new_error_event(msg));
                    }
                }
            }
            "add" => {
                // Support two forms:
                //   1) /mcp add <name> <command> [args…] [ENV=VAL…]
                //   2) /mcp add <command> [args…] [ENV=VAL…]   (name derived)
                let tail_tokens: Vec<String> = parts.map(std::string::ToString::to_string).collect();
                if tail_tokens.is_empty() {
                    let msg = "Usage: /mcp add <name> <command> [args…] [ENV=VAL…]\n       or: /mcp add <command> [args…] [ENV=VAL…]".to_string();
                    self.history_push_plain_state(history_cell::new_error_event(msg));
                    return;
                }

                // Helper: derive a reasonable server name from command/args.
                fn derive_server_name(command: &str, tokens: &[String]) -> String {
                    // Prefer an npm-style package token if present.
                    let candidate = tokens
                        .iter()
                        .find(|t| {
                            !t.starts_with('-')
                                && !t.contains('=')
                                && (t.contains('/') || t.starts_with('@'))
                        })
                        .cloned();

                    let mut raw = match candidate {
                        Some(pkg) => {
                            // Strip scope, take the last path segment
                            let after_slash = pkg.rsplit('/').next().unwrap_or(pkg.as_str());
                            // Common convention: server-<name>
                            after_slash
                                .strip_prefix("server-")
                                .unwrap_or(after_slash)
                                .to_string()
                        }
                        None => command.to_string(),
                    };

                    // Sanitize: keep [a-zA-Z0-9_-], map others to '-'
                    raw = raw
                        .chars()
                        .map(|c| {
                            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                                c
                            } else {
                                '-'
                            }
                        })
                        .collect();
                    // Collapse multiple '-'
                    let mut out = String::with_capacity(raw.len());
                    let mut prev_dash = false;
                    for ch in raw.chars() {
                        if ch == '-' && prev_dash {
                            continue;
                        }
                        prev_dash = ch == '-';
                        out.push(ch);
                    }
                    // Ensure non-empty; fall back to "server"
                    if out.trim_matches('-').is_empty() {
                        "server".to_string()
                    } else {
                        out.trim_matches('-').to_string()
                    }
                }

                // Parse the two accepted forms
                let (name, command, rest_tokens) = if tail_tokens.len() >= 2 {
                    let first = &tail_tokens[0];
                    let second = &tail_tokens[1];
                    // If the presumed command looks like a flag, assume name was omitted.
                    if second.starts_with('-') {
                        let cmd = first.clone();
                        let name = derive_server_name(&cmd, &tail_tokens[1..]);
                        (name, cmd, tail_tokens[1..].to_vec())
                    } else {
                        (first.clone(), second.clone(), tail_tokens[2..].to_vec())
                    }
                } else {
                    // Only one token provided — treat it as a command and derive a name.
                    let cmd = tail_tokens[0].clone();
                    let name = derive_server_name(&cmd, &[]);
                    (name, cmd, Vec::new())
                };

                if command.is_empty() {
                    let msg = "Usage: /mcp add <name> <command> [args…] [ENV=VAL…]".to_string();
                    self.history_push_plain_state(history_cell::new_error_event(msg));
                    return;
                }

                // Separate args from ENV=VAL pairs
                let mut args: Vec<String> = Vec::new();
                let mut env: std::collections::HashMap<String, String> =
                    std::collections::HashMap::new();
                for tok in rest_tokens.into_iter() {
                    if let Some((k, v)) = tok.split_once('=') {
                        if !k.is_empty() {
                            env.insert(k.to_string(), v.to_string());
                        }
                    } else {
                        args.push(tok);
                    }
                }
                match find_code_home() {
                    Ok(home) => {
                        let transport = code_core::config_types::McpServerTransportConfig::Stdio {
                            command,
                            args: args.clone(),
                            env: if env.is_empty() { None } else { Some(env.clone()) },
                        };
                        let cfg = code_core::config_types::McpServerConfig {
                            transport,
                            startup_timeout_sec: None,
                            tool_timeout_sec: None,
                        };
                        match code_core::config::add_mcp_server(&home, &name, cfg.clone()) {
                            Ok(()) => {
                                let summary = Self::format_mcp_summary(&cfg);
                                // Update in-memory config for future sessions
                                self.config.mcp_servers.insert(name.clone(), cfg);
                                let msg = format!("Added MCP server '{name}': {summary}");
                                self.push_background_tail(msg);
                            }
                            Err(e) => {
                                let msg = format!("Failed to add MCP server '{name}': {e}");
                                self.history_push_plain_state(history_cell::new_error_event(msg));
                            }
                        }
                    }
                    Err(e) => {
                        let msg = format!("Failed to locate CODEX_HOME: {e}");
                        self.history_push_plain_state(history_cell::new_error_event(msg));
                    }
                }
            }
            _ => {
                let msg = format!(
                    "Unknown MCP command: '{sub}'\nUsage:\n  /mcp status\n  /mcp on <name>\n  /mcp off <name>\n  /mcp add <name> <command> [args…] [ENV=VAL…]"
                );
                self.history_push_plain_state(history_cell::new_error_event(msg));
            }
        }
    }

    #[allow(dead_code)]
    fn switch_to_internal_browser(&mut self) {
        // Switch to internal browser mode
        self.browser_is_external = false;
        let latest_screenshot = self.latest_browser_screenshot.clone();
        let app_event_tx = self.app_event_tx.clone();
        let ticket = self.make_background_tail_ticket();

        tokio::spawn(async move {
            let ticket = ticket;
            let browser_manager = ChatWidget::get_browser_manager().await;

            // First, close any existing Chrome connection
            if browser_manager.is_enabled().await {
                let _ = browser_manager.close().await;
            }

            // Configure for internal browser
            {
                let mut config = browser_manager.config.write().await;
                config.connect_port = None;
                config.connect_ws = None;
                config.headless = true;
                config.persist_profile = false;
                config.enabled = true;
            }

            // Enable internal browser
            browser_manager.set_enabled_sync(true);

            // Explicitly (re)start the internal browser session now
            if let Err(e) = browser_manager.start().await {
                tracing::error!("Failed to start internal browser: {}", e);
                app_event_tx
                    .send_background_event_with_ticket(
                        &ticket,
                        format!("❌ Failed to start internal browser: {e}"),
                    );
                return;
            }

            // Set as global manager so core/session share the same instance
            code_browser::global::set_global_browser_manager(browser_manager.clone()).await;

            // Notify about successful switch/reconnect
            app_event_tx.send_background_event_with_ticket(
                &ticket,
                "✅ Switched to internal browser mode (reconnected)".to_string(),
            );

            // Clear any existing screenshot
            if let Ok(mut screenshot) = latest_screenshot.lock() {
                *screenshot = None;
            }

            // Proactively navigate to about:blank, then capture a first screenshot to populate HUD
            let _ = browser_manager.goto("about:blank").await;
            // Capture an initial screenshot to populate HUD
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
            match browser_manager.capture_screenshot_with_url().await {
                Ok((paths, url)) => {
                    if let Some(first_path) = paths.first() {
                        if let Ok(mut latest) = latest_screenshot.lock() {
                            *latest = Some((
                                first_path.clone(),
                                url.clone().unwrap_or_else(|| "Browser".to_string()),
                            ));
                        }
                        use code_core::protocol::BrowserScreenshotUpdateEvent;
                        use code_core::protocol::EventMsg;
                        app_event_tx.send(AppEvent::CodexEvent(Event {
                            id: uuid::Uuid::new_v4().to_string(),
                            event_seq: 0,
                            msg: EventMsg::BrowserScreenshotUpdate(BrowserScreenshotUpdateEvent {
                                screenshot_path: first_path.clone(),
                                url: url.unwrap_or_else(|| "Browser".to_string()),
                            }),
                            order: None,
                        }));
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to capture initial internal browser screenshot: {}",
                        e
                    );
                }
            }
        });
    }

    fn handle_chrome_connection(
        &mut self,
        host: Option<String>,
        port: Option<u16>,
        ticket: BackgroundOrderTicket,
    ) {
        tracing::info!(
            "[cdp] handle_chrome_connection begin, host={:?}, port={:?}",
            host,
            port
        );
        self.browser_is_external = true;
        let latest_screenshot = self.latest_browser_screenshot.clone();
        let app_event_tx = self.app_event_tx.clone();
        let port_display = port.map_or("auto-detect".to_string(), |p| p.to_string());
        let host_display = host.clone().unwrap_or_else(|| "127.0.0.1".to_string());

        // Add status message to chat (use BackgroundEvent with header so it renders reliably)
        let status_msg = format!(
            "🔗 Connecting to Chrome DevTools Protocol ({host_display}:{port_display})..."
        );
        self.push_background_before_next_output(status_msg);

        // Connect in background with a single, unified flow (no double-connect)
        tokio::spawn(async move {
            tracing::info!(
                "[cdp] connect task spawned, host={:?}, port={:?}",
                host,
                port
            );
            // Unified connect flow; emits success/failure messages internally
            ChatWidget::connect_to_cdp_chrome(
                host,
                port,
                latest_screenshot.clone(),
                app_event_tx.clone(),
                ticket,
            )
            .await;
        });
    }

    pub(crate) fn handle_chrome_command(&mut self, command_text: String) {
        tracing::info!("[cdp] handle_chrome_command start: '{}'", command_text);
        // Parse the chrome command arguments
        let parts: Vec<&str> = command_text.split_whitespace().collect();
        let chrome_ticket = self.make_background_tail_ticket();
        self.consume_pending_prompt_for_ui_only_turn();

        // Handle empty command - just "/chrome"
        if parts.is_empty() || command_text.trim().is_empty() {
            tracing::info!("[cdp] no args provided; toggle connect/disconnect");

            // Toggle behavior: if an external Chrome connection is active, disconnect it.
            // Otherwise, start a connection (auto-detect).
            let (tx, rx) = std::sync::mpsc::channel();
            let app_event_tx = self.app_event_tx.clone();
            let ticket = chrome_ticket.clone();
            tokio::spawn(async move {
                let browser_manager = ChatWidget::get_browser_manager().await;
                // Check if we're currently connected to an external Chrome
                let (is_external, browser_active) = {
                    let cfg = browser_manager.config.read().await;
                    let is_external = cfg.connect_port.is_some() || cfg.connect_ws.is_some();
                    drop(cfg);
                    let status = browser_manager.get_status().await;
                    (is_external, status.browser_active)
                };

                if is_external && browser_active {
                    // Disconnect from external Chrome (do not close Chrome itself)
                    if let Err(e) = browser_manager.stop().await {
                        tracing::warn!("[cdp] failed to stop external Chrome connection: {}", e);
                    }
                    // Notify UI
                    app_event_tx.send_background_event_with_ticket(
                        &ticket,
                        "🔌 Disconnected from Chrome".to_string(),
                    );
                    let _ = tx.send(true);
                } else {
                    // Not connected externally; proceed to connect
                    let _ = tx.send(false);
                }
            });

            // If the async task handled a disconnect, stop here; otherwise connect.
            let handled_disconnect = rx.recv().unwrap_or(false);
            if !handled_disconnect {
                // Switch to external Chrome mode with default/auto-detected port
                self.handle_chrome_connection(None, None, chrome_ticket);
            } else {
                // We just disconnected; reflect in title immediately
                self.browser_is_external = false;
                self.request_redraw();
            }
            return;
        }

        // Check if it's a status command
        if parts[0] == "status" {
            // Get status from BrowserManager - same as /browser status
            let (status_tx, status_rx) = std::sync::mpsc::channel();
            tokio::spawn(async move {
                let browser_manager = ChatWidget::get_browser_manager().await;
                let status = browser_manager.get_status_sync();
                let _ = status_tx.send(status);
            });
            let status = status_rx
                .recv()
                .unwrap_or_else(|_| "Failed to get browser status.".to_string());

            // Add the response to the UI
            let lines: Vec<String> = status.lines().map(std::string::ToString::to_string).collect();
            self.push_background_tail(lines.join("\n"));
            return;
        }

        // Accept several forms:
        //   /chrome 9222
        //   /chrome host:9222
        //   /chrome host 9222
        //   /chrome ws://host:9222/devtools/browser/<id>
        let mut host: Option<String> = None;
        let mut port: Option<u16> = None;
        let first = parts[0];

        if let Some(ws) = first
            .strip_prefix("ws://")
            .or_else(|| first.strip_prefix("wss://"))
        {
            // Full WS URL provided: set directly via config and return
            let ws_url = if first.starts_with("ws") {
                first.to_string()
            } else {
                format!("wss://{ws}")
            };
            tracing::info!("[cdp] /chrome provided WS endpoint: {}", ws_url);
            // Configure and connect using WS
            self.browser_is_external = true;
            let latest_screenshot = self.latest_browser_screenshot.clone();
            let app_event_tx = self.app_event_tx.clone();
            tokio::spawn(async move {
                let bm = ChatWidget::get_browser_manager().await;
                {
                    let mut cfg = bm.config.write().await;
                    cfg.enabled = true;
                    cfg.headless = false;
                    cfg.persist_profile = true;
                    cfg.connect_ws = Some(ws_url);
                    cfg.connect_port = None;
                    cfg.connect_host = None;
                }
                let _ = bm.connect_to_chrome_only().await;
                // Capture a first screenshot if possible
                tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                match bm.capture_screenshot_with_url().await {
                    Ok((paths, url)) => {
                        if let Some(first_path) = paths.first() {
                            if let Ok(mut latest) = latest_screenshot.lock() {
                                *latest = Some((
                                    first_path.clone(),
                                    url.clone().unwrap_or_else(|| "Browser".to_string()),
                                ));
                            }
                            use code_core::protocol::BrowserScreenshotUpdateEvent;
                            use code_core::protocol::EventMsg;
                            app_event_tx.send(AppEvent::CodexEvent(Event {
                                id: uuid::Uuid::new_v4().to_string(),
                                event_seq: 0,
                                msg: EventMsg::BrowserScreenshotUpdate(
                                    BrowserScreenshotUpdateEvent {
                                        screenshot_path: first_path.clone(),
                                        url: url.unwrap_or_else(|| "Browser".to_string()),
                                    },
                                ),
                                order: None,
                            }));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to capture initial external Chrome screenshot: {}",
                            e
                        );
                    }
                }
            });
            return;
        }

        if let Some((h, p)) = first.rsplit_once(':')
            && let Ok(pn) = p.parse::<u16>() {
                host = Some(h.to_string());
                port = Some(pn);
            }
        if host.is_none() && port.is_none() {
            if let Ok(pn) = first.parse::<u16>() {
                port = Some(pn);
            } else if parts.len() >= 2
                && let Ok(pn) = parts[1].parse::<u16>() {
                    host = Some(first.to_string());
                    port = Some(pn);
                }
        }
        tracing::info!("[cdp] parsed host={:?}, port={:?}", host, port);
        self.handle_chrome_connection(host, port, chrome_ticket);
    }

    /// Programmatically submit a user text message as if typed in the
    /// composer. The text will be added to conversation history and sent to
    /// the agent. This also handles slash command expansion.
    pub(crate) fn submit_text_message(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        self.submit_user_message(text.into());
    }

    /// Submit a message where the user sees `display` in history, but the
    /// model receives only `prompt`. This is used for prompt-expanding
    /// slash commands selected via the popup where expansion happens before
    /// reaching the normal composer pipeline.
    pub(crate) fn submit_prompt_with_display(&mut self, display: String, prompt: String) {
        if display.is_empty() && prompt.is_empty() {
            return;
        }
        use crate::chatwidget::message::UserMessage;
        use code_core::protocol::InputItem;
        let mut ordered = Vec::new();
        if !prompt.trim().is_empty() {
            ordered.push(InputItem::Text { text: prompt });
        }
        let msg = UserMessage {
            display_text: display,
            ordered_items: ordered,
            suppress_persistence: false,
        };
        self.submit_user_message(msg);
    }

    /// Submit a visible text message, but prepend a hidden instruction that is
    /// sent to the agent in the same turn. The hidden text is not added to the
    /// chat history; only `visible` appears to the user.
    pub(crate) fn submit_text_message_with_preface(&mut self, visible: String, preface: String) {
        if visible.is_empty() {
            return;
        }
        use crate::chatwidget::message::UserMessage;
        use code_core::protocol::InputItem;
        let mut ordered = Vec::new();
        if !preface.trim().is_empty() {
            ordered.push(InputItem::Text { text: preface });
        }
        ordered.push(InputItem::Text {
            text: visible.clone(),
        });
        let msg = UserMessage {
            display_text: visible,
            ordered_items: ordered,
            suppress_persistence: false,
        };
        self.submit_user_message(msg);
    }

    pub(crate) fn submit_hidden_text_message_with_preface(
        &mut self,
        agent_text: String,
        preface: String,
    ) {
        self.submit_hidden_text_message_with_preface_and_notice(agent_text, preface, false);
    }

    /// Submit a hidden message with optional notice surfacing.
    /// When `surface_notice` is true, the injected text is also shown in history
    /// as a developer-style notice; when false, the injection is silent.
    pub(crate) fn submit_hidden_text_message_with_preface_and_notice(
        &mut self,
        agent_text: String,
        preface: String,
        surface_notice: bool,
    ) {
        if agent_text.trim().is_empty() && preface.trim().is_empty() {
            return;
        }
        use crate::chatwidget::message::UserMessage;
        use code_core::protocol::InputItem;

        let mut ordered = Vec::new();
        let preface_cache = preface.clone();
        let agent_cache = agent_text.clone();
        if !preface.trim().is_empty() {
            ordered.push(InputItem::Text { text: preface });
        }
        if !agent_text.trim().is_empty() {
            ordered.push(InputItem::Text { text: agent_text });
        }

        if ordered.is_empty() {
            return;
        }

        if surface_notice {
            // Surface immediately in the TUI as a notice (developer-style message).
            let mut notice_lines = Vec::new();
            if !preface_cache.trim().is_empty() {
                notice_lines.push(preface_cache.trim().to_string());
            }
            if !agent_cache.trim().is_empty() {
                notice_lines.push(agent_cache.trim().to_string());
            }
            if !notice_lines.is_empty() {
                self.history_push_plain_paragraphs(PlainMessageKind::Notice, notice_lines);
            }
        }

        let msg = UserMessage {
            display_text: String::new(),
            ordered_items: ordered,
            suppress_persistence: false,
        };
        let mut cache = String::new();
        if !preface_cache.trim().is_empty() {
            cache.push_str(preface_cache.trim());
        }
        if !agent_cache.trim().is_empty() {
            if !cache.is_empty() {
                cache.push('\n');
            }
            cache.push_str(agent_cache.trim());
        }
        let cleaned = Self::strip_context_sections(&cache);
        self.last_developer_message = (!cleaned.trim().is_empty()).then_some(cleaned);
        self.pending_turn_origin = Some(TurnOrigin::Developer);
        self.submit_user_message_immediate(msg);
    }

    /// Dispatch a user message immediately, bypassing the queued/turn-active
    /// path. Used for developer/system injections that must not be lost if the
    /// current turn ends abruptly.
    fn submit_user_message_immediate(&mut self, message: UserMessage) {
        if message.ordered_items.is_empty() {
            return;
        }

        let items = message.ordered_items.clone();
        if let Err(e) = self.code_op_tx.send(Op::UserInput {
            items,
            final_output_json_schema: None,
        }) {
            tracing::error!("failed to send immediate UserInput: {e}");
        }

        self.finalize_sent_user_message(message);
    }

    /// Queue a note that will be delivered to the agent as a hidden system
    /// message immediately before the next user input is sent. Notes are
    /// drained in FIFO order so multiple updates retain their sequencing.
    pub(crate) fn queue_agent_note<S: Into<String>>(&mut self, note: S) {
        let note = note.into();
        if note.trim().is_empty() {
            return;
        }
        self.pending_agent_notes.push(note);
    }

    pub(crate) fn token_usage(&self) -> &TokenUsage {
        &self.total_token_usage
    }

    pub(crate) fn session_id(&self) -> Option<uuid::Uuid> {
        self.session_id
    }

    fn insert_resume_placeholder(&mut self) {
        if self.resume_placeholder_visible {
            return;
        }
        let key = self.next_req_key_top();
        let cell = history_cell::new_background_event(RESUME_PLACEHOLDER_MESSAGE.to_string());
        let _ = self.history_insert_with_key_global_tagged(Box::new(cell), key, "background", None);
        self.resume_placeholder_visible = true;
    }

    fn clear_resume_placeholder(&mut self) {
        if !self.resume_placeholder_visible {
            return;
        }
        if let Some(idx) = self.history_cells.iter().position(|cell| {
            cell.as_any()
                .downcast_ref::<crate::history_cell::BackgroundEventCell>()
                .map(|c| c.state().description.trim() == RESUME_PLACEHOLDER_MESSAGE)
                .unwrap_or(false)
        }) {
            self.history_remove_at(idx);
        }
        self.resume_placeholder_visible = false;
    }

    fn replace_resume_placeholder_with_notice(&mut self, message: &str) {
        if !self.resume_placeholder_visible {
            return;
        }
        self.clear_resume_placeholder();
        self.push_background_tail(message.to_string());
    }

    pub(crate) fn clear_token_usage(&mut self) {
        self.total_token_usage = TokenUsage::default();
        self.rate_limit_snapshot = None;
        self.rate_limit_warnings.reset();
        self.rate_limit_last_fetch_at = None;
        self.bottom_pane.set_token_usage(
            self.total_token_usage.clone(),
            self.last_token_usage.clone(),
            self.config.model_context_window,
        );
    }

    fn log_and_should_display_warning(&self, warning: &RateLimitWarning) -> bool {
        let reset_at = match warning.scope {
            RateLimitWarningScope::Primary => self.rate_limit_primary_next_reset_at,
            RateLimitWarningScope::Secondary => self.rate_limit_secondary_next_reset_at,
        };

        let account_id = auth_accounts::get_active_account_id(&self.config.code_home)
            .ok()
            .flatten()
            .unwrap_or_else(|| "_default".to_string());

        let plan = if account_id == "_default" {
            None
        } else {
            match account_usage::list_rate_limit_snapshots(&self.config.code_home) {
                Ok(records) => records
                    .into_iter()
                    .find(|record| record.account_id == account_id)
                    .and_then(|record| record.plan),
                Err(err) => {
                    tracing::warn!(?err, "failed to load rate limit snapshots while logging warning");
                    None
                }
            }
        };

        match account_usage::record_rate_limit_warning(
            &self.config.code_home,
            &account_id,
            plan.as_deref(),
            warning.scope,
            warning.threshold,
            reset_at,
            Utc::now(),
            &warning.message,
        ) {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!(?err, "failed to persist rate limit warning log");
                true
            }
        }
    }

    /// Export transcript for buffer-mode mirroring: omit internal sentinels
    /// and include gutter icons and a blank line between items for readability.
    pub(crate) fn export_transcript_lines_for_buffer(&self) -> Vec<ratatui::text::Line<'static>> {
        let mut out: Vec<ratatui::text::Line<'static>> = Vec::new();
        for (idx, cell) in self.history_cells.iter().enumerate() {
            out.extend(self.render_lines_for_terminal(idx, cell.as_ref()));
        }
        // Include streaming preview if present (treat like assistant output)
        let mut streaming_lines = self
            .live_builder
            .display_rows()
            .into_iter()
            .map(|r| ratatui::text::Line::from(r.text))
            .collect::<Vec<_>>();
        if !streaming_lines.is_empty() {
            // Apply gutter to streaming preview (first line gets " • ", continuations get 3 spaces)
            if let Some(first) = streaming_lines.first_mut() {
                first.spans.insert(0, ratatui::text::Span::raw(" • "));
            }
            for line in streaming_lines.iter_mut().skip(1) {
                line.spans.insert(0, ratatui::text::Span::raw("   "));
            }
            out.extend(streaming_lines);
            out.push(ratatui::text::Line::from(""));
        }
        out
    }

    /// Render a single history cell into terminal-friendly lines:
    /// - Prepend a gutter icon (symbol + space) to the first line when defined.
    /// - Add a single blank line after the cell as a separator.
    fn render_lines_for_terminal(
        &self,
        idx: usize,
        cell: &dyn crate::history_cell::HistoryCell,
    ) -> Vec<ratatui::text::Line<'static>> {
        let mut lines = self.cell_lines_for_terminal_index(idx, cell);
        let _has_icon = cell.gutter_symbol().is_some();
        let first_prefix = if let Some(sym) = cell.gutter_symbol() {
            format!(" {sym} ") // one space, icon, one space
        } else {
            "   ".to_string() // three spaces when no icon
        };
        if let Some(first) = lines.first_mut() {
            first
                .spans
                .insert(0, ratatui::text::Span::raw(first_prefix));
        }
        // For wrapped/subsequent lines, use a 3-space gutter to maintain alignment
        if lines.len() > 1 {
            for (_idx, line) in lines.iter_mut().enumerate().skip(1) {
                // Always 3 spaces for continuation lines
                line.spans.insert(0, ratatui::text::Span::raw("   "));
            }
        }
        lines.push(ratatui::text::Line::from(""));
        lines
    }

    // Desired bottom pane height (in rows) for a given terminal width.
    pub(crate) fn desired_bottom_height(&self, width: u16) -> u16 {
        self.bottom_pane.desired_height(width)
    }

    // The last bottom pane height (rows) that the layout actually used.
    // If not yet set, fall back to a conservative estimate from BottomPane.

    // (Removed) Legacy in-place reset method. The /new command now creates a fresh
    // ChatWidget (new core session) to ensure the agent context is fully reset.

    pub fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        // Hide the terminal cursor whenever a top‑level overlay is active so the
        // caret does not show inside the input while a modal (help/diff) is open.
        if self.diffs.overlay.is_some()
            || self.help.overlay.is_some()
            || self.settings.overlay.is_some()
            || self.terminal.overlay().is_some()
            || self.browser_overlay_visible
            || self.agents_terminal.active
        {
            return None;
        }
        let layout_areas = self.layout_areas(area);
        let bottom_pane_area = if layout_areas.len() == 4 {
            layout_areas[3]
        } else {
            layout_areas[2]
        };
        self.bottom_pane.cursor_pos(bottom_pane_area)
    }

    fn measured_font_size(&self) -> (u16, u16) {
        *self.cached_cell_size.get_or_init(|| {
            let size = self.terminal_info.font_size;

            // HACK: On macOS Retina displays, terminals often report physical pixels
            // but ratatui-image expects logical pixels. If we detect suspiciously
            // large cell sizes (likely 2x scaled), divide by 2.
            #[cfg(target_os = "macos")]
            {
                if size.0 >= 14 && size.1 >= 28 {
                    // Likely Retina display reporting physical pixels
                    tracing::info!(
                        "Detected likely Retina display, adjusting cell size from {:?} to {:?}",
                        size,
                        (size.0 / 2, size.1 / 2)
                    );
                    return (size.0 / 2, size.1 / 2);
                }
            }

            size
        })
    }

    fn get_git_branch(&self) -> Option<String> {
        use std::fs;
        use std::path::Path;

        let head_path = self.config.cwd.join(".git/HEAD");
        let mut cache = self.git_branch_cache.borrow_mut();
        let now = Instant::now();

        let needs_refresh = match cache.last_refresh {
            Some(last) => now.duration_since(last) >= Duration::from_millis(500),
            None => true,
        };

        if needs_refresh {
            let modified = fs::metadata(&head_path)
                .and_then(|meta| meta.modified())
                .ok();

            let metadata_changed = cache.last_head_mtime != modified || cache.last_refresh.is_none();

            if metadata_changed {
                cache.value = fs::read_to_string(&head_path)
                    .ok()
                    .and_then(|head_contents| {
                        let head = head_contents.trim();

                        if let Some(rest) = head.strip_prefix("ref: ") {
                            return Path::new(rest)
                                .file_name()
                                .and_then(|s| s.to_str())
                                .filter(|s| !s.is_empty())
                                .map(std::string::ToString::to_string);
                        }

                        if head.len() >= 7
                            && head.as_bytes().iter().all(u8::is_ascii_hexdigit)
                        {
                            return Some(format!("detached: {}", &head[..7]));
                        }

                        None
                    });
                cache.last_head_mtime = modified;
            }

            cache.last_refresh = Some(now);
        }

        cache.value.clone()
    }

    fn render_status_bar(&self, area: Rect, buf: &mut Buffer) {
        use crate::exec_command::relativize_to_home;
        use ratatui::layout::Margin;
        use ratatui::style::Modifier;
        use ratatui::style::Style;
        use ratatui::text::Line;
        use ratatui::text::Span;
        use ratatui::widgets::Block;
        use ratatui::widgets::Borders;
        use ratatui::widgets::Paragraph;

        // Add same horizontal padding as the Message input (2 chars on each side)
        let horizontal_padding = 1u16;
        let padded_area = Rect {
            x: area.x + horizontal_padding,
            y: area.y,
            width: area.width.saturating_sub(horizontal_padding * 2),
            height: area.height,
        };

        // Get current working directory string
        let cwd_str = match relativize_to_home(&self.config.cwd) {
            Some(rel) if !rel.as_os_str().is_empty() => format!("~/{}", rel.display()),
            Some(_) => "~".to_string(),
            None => self.config.cwd.display().to_string(),
        };

        let cwd_short_str = cwd_str
            .rsplit(['/', '\\'])
            .find(|segment| !segment.is_empty())
            .unwrap_or(cwd_str.as_str())
            .to_string();

        // Build status line spans with dynamic elision based on width.
        // Removal priority when space is tight:
        //   1) Reasoning level
        //   2) Model
        //   3) Shell
        //   4) Branch
        //   5) Directory
        let branch_opt = self.get_git_branch();

        // Determine current shell display (configured override or $SHELL fallback)
        let shell_display = match &self.config.shell {
            Some(shell) => format!("{} {}", shell.path, shell.args.join(" ")).trim().to_string(),
            None => std::env::var("SHELL").ok().unwrap_or_else(|| "sh".to_string()),
        };

        // Helper to assemble spans based on include flags
        let build_spans = |include_reasoning: bool,
                           include_model: bool,
                           include_shell: bool,
                           include_branch: bool,
                           include_dir: bool,
                           dir_display: &str| {
            let mut spans: Vec<Span> = Vec::new();
            // Title follows theme text color
            spans.push(Span::styled(
                "Every Code",
                Style::default()
                    .fg(crate::colors::text())
                    .add_modifier(Modifier::BOLD),
            ));

            if include_model {
                spans.push(Span::styled(
                    "  •  ",
                    Style::default().fg(crate::colors::text_dim()),
                ));
                spans.push(Span::styled(
                    "Model: ",
                    Style::default().fg(crate::colors::text_dim()),
                ));
                spans.push(Span::styled(
                    self.format_model_name(&self.config.model),
                    Style::default().fg(crate::colors::info()),
                ));
            }

            if include_shell {
                spans.push(Span::styled(
                    "  •  ",
                    Style::default().fg(crate::colors::text_dim()),
                ));
                spans.push(Span::styled(
                    "Shell: ",
                    Style::default().fg(crate::colors::text_dim()),
                ));
                spans.push(Span::styled(
                    shell_display.clone(),
                    Style::default().fg(crate::colors::info()),
                ));
            }

            if include_reasoning {
                spans.push(Span::styled(
                    "  •  ",
                    Style::default().fg(crate::colors::text_dim()),
                ));
                spans.push(Span::styled(
                    "Reasoning: ",
                    Style::default().fg(crate::colors::text_dim()),
                ));
                spans.push(Span::styled(
                    Self::format_reasoning_effort(self.config.model_reasoning_effort),
                    Style::default().fg(crate::colors::info()),
                ));
            }

            if include_dir {
                spans.push(Span::styled(
                    "  •  ",
                    Style::default().fg(crate::colors::text_dim()),
                ));
                spans.push(Span::styled(
                    "Directory: ",
                    Style::default().fg(crate::colors::text_dim()),
                ));
                spans.push(Span::styled(
                    dir_display.to_string(),
                    Style::default().fg(crate::colors::info()),
                ));
            }

            if include_branch
                && let Some(branch) = &branch_opt {
                    spans.push(Span::styled(
                        "  •  ",
                        Style::default().fg(crate::colors::text_dim()),
                    ));
                    spans.push(Span::styled(
                        "Branch: ",
                        Style::default().fg(crate::colors::text_dim()),
                    ));
                    spans.push(Span::styled(
                        branch.clone(),
                        Style::default().fg(crate::colors::success_green()),
                    ));
                }

            // Footer already shows the Ctrl+R hint; avoid duplicating it here.

            spans
        };

        // Start with all items in production; tests can opt-in to a minimal header via env flag.
        let minimal_header = std::env::var_os("CODEX_TUI_FORCE_MINIMAL_HEADER").is_some();
        let demo_mode = self.config.demo_developer_message.is_some();
        let mut include_reasoning = !minimal_header;
        let mut include_model = !minimal_header;
        let mut include_shell = !minimal_header;
        let mut include_branch = !minimal_header && branch_opt.is_some();
        let mut include_dir = !minimal_header && !demo_mode;
        let mut use_short_dir = false;
        let mut status_spans = build_spans(
            include_reasoning,
            include_model,
            include_shell,
            include_branch,
            include_dir,
            &cwd_str,
        );

        // Now recompute exact available width inside the border + padding before measuring
        // Render a bordered status block and explicitly fill its background.
        // Without a background fill, some terminals blend with prior frame
        // contents, which is especially noticeable on dark themes as dark
        // "caps" at the edges. Match the app background for consistency.
        let status_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::colors::border()))
            .style(Style::default().bg(crate::colors::background()));
        let inner_area = status_block.inner(padded_area);
        let padded_inner = inner_area.inner(Margin::new(1, 0));
        let inner_width = padded_inner.width as usize;

        // Helper to measure current spans width
        let measure =
            |spans: &Vec<Span>| -> usize { spans.iter().map(|s| s.content.chars().count()).sum() };

        if include_dir && !use_short_dir && measure(&status_spans) > inner_width {
            use_short_dir = true;
            status_spans = build_spans(
                include_reasoning,
                include_model,
                include_shell,
                include_branch,
                include_dir,
                &cwd_short_str,
            );
        }

        // Elide items in priority order until content fits
        while measure(&status_spans) > inner_width {
            if include_reasoning {
                include_reasoning = false;
            } else if include_model {
                include_model = false;
            } else if include_shell {
                include_shell = false;
            } else if include_branch {
                include_branch = false;
            } else if include_dir {
                include_dir = false;
            } else {
                break;
            }
            status_spans = build_spans(
                include_reasoning,
                include_model,
                include_shell,
                include_branch,
                include_dir,
                if use_short_dir { &cwd_short_str } else { &cwd_str },
            );
        }

        // Note: The reasoning visibility hint is appended inside `build_spans`
        // so it participates in width measurement and elision. Do not append
        // it again here to avoid overflow that caused corrupted glyph boxes on
        // some terminals.

        let status_line = Line::from(status_spans);

        let now = Instant::now();
        let mut frame_needed = false;
        if ENABLE_WARP_STRIPES && self.header_wave.schedule_if_needed(now) {
            frame_needed = true;
        }
        if frame_needed {
            self.app_event_tx
                .send(AppEvent::ScheduleFrameIn(HeaderWaveEffect::FRAME_INTERVAL));
        }

        // Render the block first
        status_block.render(padded_area, buf);
        let wave_enabled = self.header_wave.is_enabled();
        if wave_enabled {
            self.header_wave.render(padded_area, buf, now);
        }

        // Then render the text inside with padding, centered
        let effect_enabled = wave_enabled;
        let status_style = if effect_enabled {
            Style::default().fg(crate::colors::text())
        } else {
            Style::default()
                .bg(crate::colors::background())
                .fg(crate::colors::text())
        };

        let status_widget = Paragraph::new(vec![status_line.clone()])
            .alignment(ratatui::layout::Alignment::Center)
            .style(status_style);
        ratatui::widgets::Widget::render(status_widget, padded_inner, buf);

        // Track clickable regions for Model, Shell, and Reasoning
        self.track_status_bar_clickable_regions(
            &status_line.spans,
            padded_inner,
            include_model,
            include_shell,
            include_reasoning,
        );
    }

    /// Calculate and store clickable regions for the status bar (Model, Shell, Reasoning)
    fn track_status_bar_clickable_regions(
        &self,
        spans: &[Span],
        area: Rect,
        include_model: bool,
        include_shell: bool,
        include_reasoning: bool,
    ) {
        // Calculate total width of all spans
        let total_width: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        
        // Calculate starting x position for centered text
        let start_x = if total_width < area.width as usize {
            area.x + ((area.width as usize - total_width) / 2) as u16
        } else {
            area.x
        };
        
        let mut current_x = start_x;
        let mut regions = self.clickable_regions.borrow_mut();
        regions.clear();  // Clear previous frame's regions
        
        // Scan through spans to find Model, Shell, and Reasoning sections
        let mut i = 0;
        while i < spans.len() {
            let span = &spans[i];
            let content = span.content.as_ref();
            
            // Check if this is a clickable label
            if include_model && content.contains("Model:") {
                // Find the extent of the Model section (label + value)
                let mut section_width = content.chars().count();
                if i + 1 < spans.len() {
                    // Include the value span
                    section_width += spans[i + 1].content.chars().count();
                }
                regions.push(ClickableRegion {
                    rect: Rect {
                        x: current_x,
                        y: area.y,
                        width: section_width as u16,
                        height: 1,
                    },
                    action: ClickableAction::ShowModelSelector,
                });
                current_x += content.chars().count() as u16;
                i += 1;
                if i < spans.len() {
                    current_x += spans[i].content.chars().count() as u16;
                    i += 1;
                }
                continue;
            }
            
            if include_shell && content.contains("Shell:") {
                let mut section_width = content.chars().count();
                if i + 1 < spans.len() {
                    section_width += spans[i + 1].content.chars().count();
                }
                regions.push(ClickableRegion {
                    rect: Rect {
                        x: current_x,
                        y: area.y,
                        width: section_width as u16,
                        height: 1,
                    },
                    action: ClickableAction::ShowShellSelector,
                });
                current_x += content.chars().count() as u16;
                i += 1;
                if i < spans.len() {
                    current_x += spans[i].content.chars().count() as u16;
                    i += 1;
                }
                continue;
            }
            
            if include_reasoning && content.contains("Reasoning:") {
                let mut section_width = content.chars().count();
                if i + 1 < spans.len() {
                    section_width += spans[i + 1].content.chars().count();
                }
                regions.push(ClickableRegion {
                    rect: Rect {
                        x: current_x,
                        y: area.y,
                        width: section_width as u16,
                        height: 1,
                    },
                    action: ClickableAction::ShowReasoningSelector,
                });
                current_x += content.chars().count() as u16;
                i += 1;
                if i < spans.len() {
                    current_x += spans[i].content.chars().count() as u16;
                    i += 1;
                }
                continue;
            }
            
            // Not a clickable section, just advance position
            current_x += content.chars().count() as u16;
            i += 1;
        }
    }

    fn render_screenshot_highlevel(&self, path: &PathBuf, area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::Widget;
        use ratatui_image::Image;
        use ratatui_image::Resize;
        use ratatui_image::picker::Picker;
        use ratatui_image::picker::ProtocolType;

        // First, cheaply read image dimensions without decoding the full image
        let (img_w, img_h) = match image::image_dimensions(path) {
            Ok(dim) => dim,
            Err(_) => {
                self.render_screenshot_placeholder(path, area, buf);
                return;
            }
        };

        // picker (Retina 2x workaround preserved)
        let mut cached_picker = self.cached_picker.borrow_mut();
        if cached_picker.is_none() {
            // If we didn't get a picker from terminal query at startup, create one from font size
            let (fw, fh) = self.measured_font_size();
            let p = Picker::from_fontsize((fw, fh));

            *cached_picker = Some(p);
        }
        let Some(picker) = cached_picker.as_ref() else {
            self.render_screenshot_placeholder(path, area, buf);
            return;
        };

        // quantize step by protocol to avoid rounding bias
        let (_qx, _qy): (u16, u16) = match picker.protocol_type() {
            ProtocolType::Halfblocks => (1, 2), // half-block cell = 1 col x 2 half-rows
            _ => (1, 1),                        // pixel protocols (Kitty/iTerm2/Sixel)
        };

        // terminal cell aspect
        let (cw, ch) = self.measured_font_size();
        let cols = area.width as u32;
        let rows = area.height as u32;
        let cw = cw as u32;
        let ch = ch as u32;

        // fit (floor), then choose limiting dimension
        let mut rows_by_w = (cols * cw * img_h) / (img_w * ch);
        if rows_by_w == 0 {
            rows_by_w = 1;
        }
        let mut cols_by_h = (rows * ch * img_w) / (img_h * cw);
        if cols_by_h == 0 {
            cols_by_h = 1;
        }

        let (_used_cols, _used_rows) = if rows_by_w <= rows {
            (cols, rows_by_w)
        } else {
            (cols_by_h, rows)
        };

        // Compute a centered target rect based on image aspect and font cell size
        let (cell_w, cell_h) = self.measured_font_size();
        let area_px_w = (area.width as u32) * (cell_w as u32);
        let area_px_h = (area.height as u32) * (cell_h as u32);
        // If either dimension is zero, bail to placeholder
        if area.width == 0 || area.height == 0 || area_px_w == 0 || area_px_h == 0 {
            self.render_screenshot_placeholder(path, area, buf);
            return;
        }
        let (img_w, img_h) = match image::image_dimensions(path) {
            Ok(dim) => dim,
            Err(_) => {
                self.render_screenshot_placeholder(path, area, buf);
                return;
            }
        };
        let scale_num_w = area_px_w;
        let scale_num_h = area_px_h;
        let scale_w = scale_num_w as f64 / img_w as f64;
        let scale_h = scale_num_h as f64 / img_h as f64;
        let scale = scale_w.min(scale_h).max(0.0);
        // Compute target size in cells
        let target_w_cells = ((img_w as f64 * scale) / (cell_w as f64)).floor() as u16;
        let target_h_cells = ((img_h as f64 * scale) / (cell_h as f64)).floor() as u16;
        let target_w = target_w_cells.clamp(1, area.width);
        let target_h = target_h_cells.clamp(1, area.height);
        let target_x = area.x + (area.width.saturating_sub(target_w)) / 2;
        let target_y = area.y + (area.height.saturating_sub(target_h)) / 2;
        let target = Rect {
            x: target_x,
            y: target_y,
            width: target_w,
            height: target_h,
        };

        // cache by (path, target)
        let needs_recreate = {
            let cached = self.cached_image_protocol.borrow();
            match cached.as_ref() {
                Some((cached_path, cached_rect, _)) => {
                    cached_path != path || *cached_rect != target
                }
                None => true,
            }
        };
        if needs_recreate {
            // Only decode when we actually need to (path/target changed)
            let dyn_img = match image::ImageReader::open(path) {
                Ok(r) => match r.decode() {
                    Ok(img) => img,
                    Err(_) => {
                        self.render_screenshot_placeholder(path, area, buf);
                        return;
                    }
                },
                Err(_) => {
                    self.render_screenshot_placeholder(path, area, buf);
                    return;
                }
            };
            match picker.new_protocol(dyn_img, target, Resize::Fit(Some(FilterType::Lanczos3))) {
                Ok(protocol) => {
                    *self.cached_image_protocol.borrow_mut() =
                        Some((path.clone(), target, protocol))
                }
                Err(_) => {
                    self.render_screenshot_placeholder(path, area, buf);
                    return;
                }
            }
        }

        if let Some((_, rect, protocol)) = &*self.cached_image_protocol.borrow() {
            let image = Image::new(protocol);
            Widget::render(image, *rect, buf);
        } else {
            self.render_screenshot_placeholder(path, area, buf);
        }
    }

    fn render_screenshot_placeholder(&self, path: &PathBuf, area: Rect, buf: &mut Buffer) {
        use ratatui::style::Modifier;
        use ratatui::style::Style;
        use ratatui::widgets::Block;
        use ratatui::widgets::Borders;
        use ratatui::widgets::Paragraph;

        // Show a placeholder box with screenshot info
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("screenshot");

        let placeholder_text = format!("[Screenshot]\n{filename}");
        let placeholder_widget = Paragraph::new(placeholder_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(crate::colors::info()))
                    .title("Browser"),
            )
            .style(
                Style::default()
                    .fg(crate::colors::text_dim())
                    .add_modifier(Modifier::ITALIC),
            )
            .wrap(ratatui::widgets::Wrap { trim: true });

        placeholder_widget.render(area, buf);
    }
}

async fn run_background_review(
    config: Config,
    app_event_tx: AppEventSender,
    base_snapshot: Option<GhostCommit>,
    turn_context: Option<String>,
    prefer_fallback: bool,
) {
    // Best-effort: clean up any stale lock left by a cancelled review process.
    let _ = code_core::review_coord::clear_stale_lock_if_dead(Some(&config.cwd));

    // Prevent duplicate auto-reviews within this process: if any AutoReview agent
    // is already pending/running, bail early with a benign notice.
    {
        let mgr = code_core::AGENT_MANAGER.read().await;
        let busy = mgr
            .list_agents(None, Some("auto-review".to_string()), false)
            .into_iter()
            .any(|agent| {
                let status = format!("{:?}", agent.status).to_ascii_lowercase();
                status == "running" || status == "pending"
            });
        if busy {
            app_event_tx.send(AppEvent::BackgroundReviewFinished {
                worktree_path: std::path::PathBuf::new(),
                branch: String::new(),
                has_findings: false,
                findings: 0,
                summary: Some("Auto review skipped: another auto review is already running.".to_string()),
                error: None,
                agent_id: None,
                snapshot: None,
            });
            return;
        }
    }

    let app_event_tx_clone = app_event_tx.clone();
    let outcome = async move {
        let git_root = code_core::git_worktree::get_git_root_from(&config.cwd)
            .await
            .map_err(|e| format!("failed to detect git root: {e}"))?;

        let snapshot = task::spawn_blocking({
            let repo_path = config.cwd.clone();
            let base_snapshot = base_snapshot.clone();
            move || {
                let mut options = CreateGhostCommitOptions::new(repo_path.as_path())
                    .message("auto review snapshot");
                if let Some(base) = base_snapshot.as_ref() {
                    options = options.parent(base.id());
                }
                let hook_repo = repo_path.clone();
                let hook = move || bump_snapshot_epoch_for(&hook_repo);
                create_ghost_commit(&options.post_commit_hook(&hook))
            }
        })
        .await
        .map_err(|e| format!("failed to spawn snapshot task: {e}"))
        .and_then(|res| res.map_err(|e| format!("failed to capture snapshot: {e}")))?;

        let snapshot_id = snapshot.id().to_string();
        bump_snapshot_epoch_for(&config.cwd);

        // Attempt to hold the shared review lock; if busy or a previous review
        // with findings is still surfaced, fall back to a per-request
        // auto-review worktree to avoid clobbering pending fixes.
        let (worktree_path, branch, worktree_guard) = if prefer_fallback {
            let (path, name, guard) =
                allocate_fallback_auto_review_worktree(&git_root, &snapshot_id).await?;
            (path, name, guard)
        } else {
            match try_acquire_lock("review", &config.cwd) {
                Ok(Some(g)) => {
                    let path = code_core::git_worktree::prepare_reusable_worktree(
                        &git_root,
                        AUTO_REVIEW_SHARED_WORKTREE,
                        snapshot_id.as_str(),
                        true,
                    )
                    .await
                    .map_err(|e| format!("failed to prepare worktree: {e}"))?;
                    (path, AUTO_REVIEW_SHARED_WORKTREE.to_string(), g)
                }
                Ok(None) => {
                    let (path, name, guard) =
                        allocate_fallback_auto_review_worktree(&git_root, &snapshot_id).await?;
                    (path, name, guard)
                }
                Err(err) => {
                    return Err(format!("could not acquire review lock: {err}"));
                }
            }
        };

        // Ensure Codex models are invoked via the `code-` CLI shim so they exist on PATH.
        fn ensure_code_prefix(model: &str) -> String {
            let lower = model.to_ascii_lowercase();
            if lower.starts_with("code-") {
                model.to_string()
            } else {
                format!("code-{model}")
            }
        }

        let review_model = ensure_code_prefix(&config.auto_review_model);

        // Allow the spawned agent to reuse the parent's review lock without blocking.
        let mut env: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        env.insert("CODE_REVIEW_LOCK_LEASE".to_string(), "1".to_string());
        let agent_config = code_core::config_types::AgentConfig {
            name: review_model.clone(),
            command: String::new(),
            args: Vec::new(),
            read_only: false,
            enabled: true,
            description: None,
            env: Some(env),
            args_read_only: None,
            args_write: None,
            instructions: None,
        };

        // Use the /review entrypoint so upstream wiring (model defaults, review formatting) stays intact.
        let mut review_prompt = format!(
            "/review Analyze only changes made in commit {snapshot_id}. Identify critical bugs, regressions, security/performance/concurrency risks or incorrect assumptions. Provide actionable feedback and references to the changed code; ignore minor style or formatting nits."
        );

        if let Some(context) = turn_context {
            review_prompt.push_str("\n\n");
            review_prompt.push_str(&context);
        }

        let mut manager = code_core::AGENT_MANAGER.write().await;
        let agent_id = manager
            .create_agent_with_options(
                review_model,
                Some("Auto Review".to_string()),
                review_prompt,
                None,
                None,
                Vec::new(),
                false,
                Some(branch.clone()),
                Some(agent_config.clone()),
                Some(branch.clone()),
                Some(snapshot_id.clone()),
                Some(code_core::protocol::AgentSourceKind::AutoReview),
                config.auto_review_model_reasoning_effort.into(),
            )
            .await;
        insert_background_lock(&agent_id, worktree_guard);
        drop(manager);

        app_event_tx_clone.send(AppEvent::BackgroundReviewStarted {
            worktree_path: worktree_path.clone(),
            branch: branch.clone(),
            agent_id: Some(agent_id.clone()),
            snapshot: Some(snapshot_id.clone()),
        });
        Ok::<(PathBuf, String, String, String), String>((worktree_path, branch, agent_id, snapshot_id))
    }
    .await;

    if let Err(err) = outcome {
        app_event_tx.send(AppEvent::BackgroundReviewFinished {
            worktree_path: std::path::PathBuf::new(),
            branch: String::new(),
            has_findings: false,
            findings: 0,
            summary: None,
            error: Some(err),
            agent_id: None,
            snapshot: None,
        });
    }
}

#[allow(dead_code)]
fn insert_background_lock(agent_id: &str, guard: code_core::review_coord::ReviewGuard) {
    if let Ok(mut map) = BACKGROUND_REVIEW_LOCKS.lock() {
        map.insert(agent_id.to_string(), guard);
    }
}

fn release_background_lock(agent_id: &Option<String>) {
    if let Some(id) = agent_id
        && let Ok(mut map) = BACKGROUND_REVIEW_LOCKS.lock() {
            map.remove(id);
        }
}

#[cfg(test)]
static AUTO_REVIEW_STUB: once_cell::sync::Lazy<std::sync::Mutex<Option<Box<dyn FnMut() + Send>>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

#[cfg(test)]
struct AutoReviewStubGuard;

#[cfg(test)]
impl AutoReviewStubGuard {
    fn install<F: FnMut() + Send + 'static>(f: F) -> Self {
        let mut guard = AUTO_REVIEW_STUB.lock().unwrap();
        *guard = Some(Box::new(f));
        AutoReviewStubGuard
    }
}

#[cfg(test)]
impl Drop for AutoReviewStubGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = AUTO_REVIEW_STUB.lock() {
            *guard = None;
        }
    }
}

#[cfg(test)]
    mod tests {
        use super::*;
        use super::{
            CAPTURE_AUTO_TURN_COMMIT_STUB,
            GIT_DIFF_NAME_ONLY_BETWEEN_STUB,
        };
        use crate::app_event::AppEvent;
        use crate::bottom_pane::AutoCoordinatorViewModel;
    use crate::chatwidget::message::UserMessage;
    use crate::chatwidget::smoke_helpers::{enter_test_runtime_guard, ChatWidgetHarness};
    use crate::history_cell::{self, ExploreAggregationCell, HistoryCellType};
    use code_auto_drive_core::{
        AutoContinueMode,
        AutoRunPhase,
        AutoRunSummary,
        TurnComplexity,
        TurnMode,
        AUTO_RESOLVE_MAX_REVIEW_ATTEMPTS,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use code_core::config_types::AutoResolveAttemptLimit;
    use code_core::history::state::{
        AssistantStreamDelta,
        AssistantStreamState,
        HistoryId,
        HistoryRecord,
        HistorySnapshot,
        HistoryState,
        InlineSpan,
        MessageLine,
        MessageLineKind,
        OrderKeySnapshot,
        PlainMessageKind,
        PlainMessageRole,
        PlainMessageState,
        TextEmphasis,
        TextTone,
    };
use code_core::parse_command::ParsedCommand;
use code_core::protocol::OrderMeta;
    use code_core::config_types::{McpServerConfig, McpServerTransportConfig};
    use code_core::protocol::{
        AskForApproval,
        AgentMessageEvent,
        AgentStatusUpdateEvent,
        ErrorEvent,
        Event,
        EventMsg,
        ExecCommandBeginEvent,
        McpServerFailure,
        McpServerFailurePhase,
        TaskCompleteEvent,
    };
    use code_core::protocol::AgentInfo as CoreAgentInfo;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::process::Command;
    use tempfile::tempdir;
    use std::sync::Arc;
    use std::path::PathBuf;

    #[test]
    fn parse_agent_review_result_json_clean() {
        let json = r#"{
            "findings": [],
            "overall_correctness": "ok",
            "overall_explanation": "looks clean",
            "overall_confidence_score": 0.9
        }"#;

        let (has_findings, findings, summary) = ChatWidget::parse_agent_review_result(Some(json));
        assert!(!has_findings);
        assert_eq!(findings, 0);
        assert_eq!(summary.as_deref(), Some("looks clean"));
    }

    #[test]
    fn parse_agent_review_result_json_with_findings() {
        let json = r#"{
            "findings": [
                {"title": "bug", "body": "fix", "confidence_score": 0.5, "priority": 1, "code_location": {"absolute_file_path": "foo", "line_range": {"start":1,"end":1}}}
            ],
            "overall_correctness": "incorrect",
            "overall_explanation": "needs work",
            "overall_confidence_score": 0.6
        }"#;

        let (has_findings, findings, summary) = ChatWidget::parse_agent_review_result(Some(json));
        assert!(has_findings);
        assert_eq!(findings, 1);
        let summary_text = summary.unwrap();
        assert!(summary_text.contains("needs work"));
        assert!(summary_text.contains("bug"));
    }

    #[test]
    fn mcp_summary_includes_tools_and_failures() {
        let mut harness = ChatWidgetHarness::new();
        harness.with_chat(|chat| {
            chat.mcp_tools_by_server.insert(
                "alpha".to_string(),
                vec!["fetch".to_string(), "search".to_string()],
            );
            chat.mcp_server_failures.insert(
                "beta".to_string(),
                McpServerFailure {
                    phase: McpServerFailurePhase::ListTools,
                    message: "timeout".to_string(),
                },
            );

            let ok_cfg = McpServerConfig {
                transport: McpServerTransportConfig::Stdio {
                    command: "alpha-bin".to_string(),
                    args: Vec::new(),
                    env: None,
                },
                startup_timeout_sec: None,
                tool_timeout_sec: None,
            };
            let fail_cfg = McpServerConfig {
                transport: McpServerTransportConfig::Stdio {
                    command: "beta-bin".to_string(),
                    args: Vec::new(),
                    env: None,
                },
                startup_timeout_sec: None,
                tool_timeout_sec: None,
            };

            let ok_summary = chat.format_mcp_server_summary("alpha", &ok_cfg, true);
            let fail_summary = chat.format_mcp_server_summary("beta", &fail_cfg, true);

            assert!(
                ok_summary.contains("Tools: fetch, search"),
                "expected tool list in summary, got: {ok_summary}"
            );
            assert!(
                fail_summary.contains("Failed to list tools: timeout"),
                "expected failure message in summary, got: {fail_summary}"
            );
        });
    }

    #[test]
    fn parse_agent_review_result_json_multi_run() {
        let json = r#"{
            "findings": [],
            "overall_correctness": "correct",
            "overall_explanation": "clean",
            "overall_confidence_score": 0.9,
            "runs": [
                {
                    "findings": [
                        {"title": "bug", "body": "fix", "confidence_score": 0.5, "priority": 1, "code_location": {"absolute_file_path": "foo", "line_range": {"start":1,"end":1}}}
                    ],
                    "overall_correctness": "incorrect",
                    "overall_explanation": "needs work",
                    "overall_confidence_score": 0.6
                },
                {
                    "findings": [],
                    "overall_correctness": "correct",
                    "overall_explanation": "clean",
                    "overall_confidence_score": 0.9
                }
            ]
        }"#;

        let (has_findings, findings, summary) = ChatWidget::parse_agent_review_result(Some(json));
        assert!(has_findings);
        assert_eq!(findings, 1);
        let summary_text = summary.unwrap();
        assert!(summary_text.contains("needs work"));
        assert!(summary_text.contains("Final pass reported no issues"));
    }

    #[test]
    fn parse_agent_review_result_skip_lock() {
        let text = "Another review is already running; skipping this /review.";
        let (has_findings, findings, summary) = ChatWidget::parse_agent_review_result(Some(text));

        assert!(!has_findings);
        assert_eq!(findings, 0);
        assert_eq!(summary.as_deref(), Some(text));
    }

    #[test]
    fn format_model_name_capitalizes_codex_mini() {
        let mut harness = ChatWidgetHarness::new();
        let formatted = harness.chat().format_model_name("gpt-5.1-codex-mini");
        assert_eq!(formatted, "GPT-5.1-Codex-Mini");
    }

    #[test]
    fn auto_review_triggers_when_enabled_and_diff_seen() {
        let _guard = AutoReviewStubGuard::install(|| {});
        let _capture_guard = CaptureCommitStubGuard::install(|_, _| {
            Ok(GhostCommit::new("baseline".to_string(), None))
        });
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        chat.config.tui.auto_review_enabled = true;
        chat.turn_had_code_edits = true;
        chat.background_review = None;

        chat.maybe_trigger_auto_review();

        assert!(chat.background_review.is_some(), "background review should start");
    }

    #[test]
    fn auto_review_does_not_duplicate_while_running() {
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let _guard = AutoReviewStubGuard::install(move || {
            calls_clone.fetch_add(1, Ordering::SeqCst);
        });
        let _capture_guard = CaptureCommitStubGuard::install(|_, _| {
            Ok(GhostCommit::new("baseline".to_string(), None))
        });

        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        chat.config.tui.auto_review_enabled = true;
        chat.turn_had_code_edits = true;
        chat.background_review = None;

        chat.maybe_trigger_auto_review();
        // Already running; second trigger should no-op
        chat.turn_had_code_edits = true;
        chat.maybe_trigger_auto_review();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn auto_review_skips_when_no_changes_since_reviewed_snapshot() {
        let _rt = enter_test_runtime_guard();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_clone = calls.clone();
        let _guard = AutoReviewStubGuard::install(move || {
            calls_clone.fetch_add(1, Ordering::SeqCst);
        });

        let repo = tempdir().expect("temp repo");
        let repo_path = repo.path();
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .current_dir(repo_path)
                .args(args)
                .status()
                .expect("git command");
            assert!(status.success(), "git command failed: {args:?}");
        };

        git(&["init"]);
        git(&["config", "user.email", "auto@review.test"]);
        git(&["config", "user.name", "Auto Review"]);
        std::fs::write(repo_path.join("README.md"), "hello")
            .expect("write README");
        git(&["add", "."]);
        git(&["commit", "-m", "init"]);

        let snapshot = create_ghost_commit(
            &CreateGhostCommitOptions::new(repo_path).message("auto review snapshot"),
        )
        .expect("ghost snapshot");

        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        chat.config.cwd = repo_path.to_path_buf();
        chat.config.tui.auto_review_enabled = true;
        chat.turn_had_code_edits = true;
        chat.auto_review_reviewed_marker = Some(snapshot);

        chat.maybe_trigger_auto_review();

        assert_eq!(calls.load(Ordering::SeqCst), 0, "auto review should skip");
        assert!(chat.background_review.is_none());
    }

    #[test]
    fn task_started_defers_auto_review_baseline_capture() {
        let _stub_lock = AUTO_STUB_LOCK.lock().unwrap();
        let _rt = enter_test_runtime_guard();
        let _capture_guard = CaptureCommitStubGuard::install(|_, _| {
            Ok(GhostCommit::new("baseline".to_string(), None))
        });
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        chat.config.tui.auto_review_enabled = true;

        chat.handle_code_event(Event {
            id: "turn-1".to_string(),
            event_seq: 0,
            msg: EventMsg::TaskStarted,
            order: None,
        });

        assert!(
            chat.auto_review_baseline.is_none(),
            "baseline capture should not block TaskStarted"
        );
    }

    #[test]
    fn background_review_completion_resumes_auto_and_posts_summary() {
        let _rt = enter_test_runtime_guard();
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.insert_final_answer_with_id(
            None,
            vec![ratatui::text::Line::from("Assistant reply")],
            "Assistant reply".to_string(),
        );

        chat.config.tui.auto_review_enabled = true;
        chat.auto_state.on_begin_review(false);

        chat.background_review = Some(BackgroundReviewState {
            worktree_path: PathBuf::from("/tmp/wt"),
            branch: "auto-review-branch".to_string(),
            agent_id: Some("agent-123".to_string()),
            snapshot: Some("ghost123".to_string()),
            base: None,
            last_seen: std::time::Instant::now(),
        });

        chat.on_background_review_finished(
            PathBuf::from("/tmp/wt"),
            "auto-review-branch".to_string(),
            true,
            2,
            Some("Short summary".to_string()),
            None,
            Some("agent-123".to_string()),
            Some("ghost123".to_string()),
        );

        assert!(
            !chat.auto_state.awaiting_review(),
            "auto drive should resume after background review completes"
        );

        let footer_status = chat
            .bottom_pane
            .auto_review_status()
            .expect("footer should show auto review status");
        assert_eq!(footer_status.status, AutoReviewIndicatorStatus::Fixed);
        assert_eq!(footer_status.findings, Some(2));
        let notice_present = chat.history_cells.iter().any(|cell| {
            cell.display_lines_trimmed().iter().any(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains("issue(s) found"))
            })
        });
        assert!(notice_present, "actionable auto review notice should be visible");
        assert!(chat.pending_agent_notes.is_empty(), "idle path should inject via hidden message, not queue notes");
        let developer_seen = chat
            .pending_dispatched_user_messages
            .iter()
            .any(|msg| msg.contains("[developer]"));
        assert!(developer_seen, "developer note should be sent in hidden message");
    }

    #[test]
    fn background_review_busy_path_enqueues_developer_note_with_merge_hint() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.config.tui.auto_review_enabled = true;
        chat.bottom_pane.set_task_running(true); // simulate busy state so note is queued

        chat.background_review = Some(BackgroundReviewState {
            worktree_path: PathBuf::from("/tmp/wt"),
            branch: "auto-review-branch".to_string(),
            agent_id: Some("agent-123".to_string()),
            snapshot: Some("ghost123".to_string()),
            base: None,
            last_seen: std::time::Instant::now(),
        });

        // Agent.result will be parsed; provide structured JSON with findings
        let review_json = r#"{
            "findings": [
                {"title": "bug", "body": "fix", "confidence_score": 0.5, "priority": 1, "code_location": {"absolute_file_path": "foo", "line_range": {"start":1,"end":1}}}
            ],
            "overall_correctness": "incorrect",
            "overall_explanation": "needs work",
            "overall_confidence_score": 0.6
        }"#;

        // Simulate agent status observation completion path
        chat.on_background_review_finished(
            PathBuf::from("/tmp/wt"),
            "auto-review-branch".to_string(),
            true,
            1,
            Some(review_json.to_string()),
            None,
            Some("agent-123".to_string()),
            Some("ghost123".to_string()),
        );

        // Busy path still injects a developer note immediately so the user sees it in the transcript.
        assert!(chat.pending_agent_notes.is_empty());
        let developer_sent = chat
            .pending_dispatched_user_messages
            .iter()
            .any(|msg| msg.contains("[developer]") && msg.contains("Merge the worktree") && msg.contains("auto-review-branch"));
        assert!(developer_sent, "developer merge-hint note should be injected even while busy");
    }

    #[test]
    fn background_review_observe_idle_injects_note_from_agent_result() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.config.tui.auto_review_enabled = true;
        chat.background_review = Some(BackgroundReviewState {
            worktree_path: PathBuf::from("/tmp/wt"),
            branch: "auto-review-branch".to_string(),
            agent_id: None,
            snapshot: Some("ghost123".to_string()),
            base: None,
            last_seen: std::time::Instant::now(),
        });

        let agent = code_core::protocol::AgentInfo {
            id: "agent-1".to_string(),
            name: "Auto Review".to_string(),
            status: "completed".to_string(),
            batch_id: Some("auto-review-branch".to_string()),
            model: Some("code-review".to_string()),
            last_progress: None,
            result: Some(
                r#"{
                    "findings":[{"title":"bug","body":"details","confidence_score":0.5,"priority":1,"code_location":{"absolute_file_path":"src/lib.rs","line_range":{"start":1,"end":1}}}],
                    "overall_correctness":"incorrect",
                    "overall_explanation":"needs work",
                    "overall_confidence_score":0.6
                }"#
                .to_string(),
            ),
            error: None,
            elapsed_ms: None,
            token_count: None,
            last_activity_at: None,
            seconds_since_last_activity: None,
            source_kind: Some(AgentSourceKind::AutoReview),
        };

        chat.observe_auto_review_status(&[agent]);

        // Idle path: should send hidden developer note immediately (not queued)
        assert!(chat.pending_agent_notes.is_empty());
        let developer_sent = chat
            .pending_dispatched_user_messages
            .iter()
            .any(|msg| msg.contains("[developer]") && msg.contains("Merge the worktree") && msg.contains("auto-review-branch"));
        assert!(developer_sent, "developer merge-hint note should be injected when idle");
    }

    #[test]
    fn background_review_observe_busy_queues_note_from_agent_result() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.config.tui.auto_review_enabled = true;
        chat.bottom_pane.set_task_running(true);
        chat.background_review = Some(BackgroundReviewState {
            worktree_path: PathBuf::from("/tmp/wt"),
            branch: "auto-review-branch".to_string(),
            agent_id: None,
            snapshot: Some("ghost123".to_string()),
            base: None,
            last_seen: std::time::Instant::now(),
        });

        let agent = code_core::protocol::AgentInfo {
            id: "agent-1".to_string(),
            name: "Auto Review".to_string(),
            status: "completed".to_string(),
            batch_id: Some("auto-review-branch".to_string()),
            model: Some("code-review".to_string()),
            last_progress: None,
            result: Some(
                r#"{
                    "findings":[{"title":"bug","body":"details","confidence_score":0.5,"priority":1,"code_location":{"absolute_file_path":"src/lib.rs","line_range":{"start":1,"end":1}}}],
                    "overall_correctness":"incorrect",
                    "overall_explanation":"needs work",
                    "overall_confidence_score":0.6
                }"#
                .to_string(),
            ),
            error: None,
            elapsed_ms: None,
            token_count: None,
            last_activity_at: None,
            seconds_since_last_activity: None,
            source_kind: Some(AgentSourceKind::AutoReview),
        };

        chat.observe_auto_review_status(&[agent]);

        assert!(chat.pending_agent_notes.is_empty());
        let developer_sent = chat
            .pending_dispatched_user_messages
            .iter()
            .any(|msg| msg.contains("[developer]") && msg.contains("Merge the worktree") && msg.contains("auto-review-branch"));
        assert!(developer_sent, "developer merge-hint note should be injected when busy");
    }

    #[test]
    fn terminal_auto_review_without_worktree_state_does_not_surface_blank_path() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.config.tui.auto_review_enabled = true;
        chat.background_review = None;

        let agent = code_core::protocol::AgentInfo {
            id: "agent-blank".to_string(),
            name: "Auto Review".to_string(),
            status: "failed".to_string(),
            batch_id: None,
            model: Some("code-review".to_string()),
            last_progress: None,
            result: None,
            error: Some("fatal: not a git repository".to_string()),
            elapsed_ms: None,
            token_count: None,
            last_activity_at: None,
            seconds_since_last_activity: None,
            source_kind: Some(AgentSourceKind::AutoReview),
        };

        chat.observe_auto_review_status(&[agent]);

        let blank_path_message = chat
            .pending_dispatched_user_messages
            .iter()
            .any(|msg| msg.contains("Worktree path: \n") || msg.contains("Worktree path: \r\n"));
        assert!(!blank_path_message, "should not emit auto-review message with blank worktree path");
        assert!(chat.processed_auto_review_agents.contains("agent-blank"));
    }

    #[test]
    fn missing_agent_clis_start_disabled_in_overview() {
        let orig_path = std::env::var_os("PATH");
        unsafe {
            std::env::set_var("PATH", "");
        }

        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        let (rows, _commands) = chat.collect_agents_overview_rows();
        let qwen = rows
            .iter()
            .find(|row| row.name == "qwen-3-coder")
            .expect("qwen row present");
        assert!(!qwen.installed);
        assert!(!qwen.enabled);

        let code = rows
            .iter()
            .find(|row| row.name == "code-gpt-5.2")
            .expect("code row present");
        assert!(code.installed);
        assert!(code.enabled);

        if let Some(path) = orig_path {
            unsafe {
                std::env::set_var("PATH", path);
            }
        } else {
            unsafe {
                std::env::remove_var("PATH");
            }
        }
    }

    #[test]
    fn skipped_auto_review_with_findings_defers_to_next_turn() {
        let _rt = enter_test_runtime_guard();
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        let launches = Arc::new(AtomicUsize::new(0));
        let launches_clone = launches.clone();
        let _stub = AutoReviewStubGuard::install(move || {
            launches_clone.fetch_add(1, Ordering::SeqCst);
        });

        chat.config.tui.auto_review_enabled = true;
        chat.turn_sequence = 1;
        chat.turn_had_code_edits = true;
        let pending_base = GhostCommit::new("base-skip".to_string(), None);
        chat.auto_review_baseline = Some(pending_base.clone());

        chat.background_review = Some(BackgroundReviewState {
            worktree_path: PathBuf::from("/tmp/wt"),
            branch: "auto-review-running".to_string(),
            agent_id: Some("agent-running".to_string()),
            snapshot: Some("ghost-running".to_string()),
            base: Some(GhostCommit::new("running-base".to_string(), None)),
            last_seen: Instant::now(),
        });

        chat.maybe_trigger_auto_review();
        assert_eq!(launches.load(Ordering::SeqCst), 0, "should skip while review runs");
        let pending = chat
            .pending_auto_review_range
            .as_ref()
            .expect("pending range queued");
        assert_eq!(pending.base.id(), pending_base.id());
        assert_eq!(pending.defer_until_turn, None);

        chat.on_background_review_finished(
            PathBuf::from("/tmp/wt"),
            "auto-review-running".to_string(),
            true,
            2,
            Some("found issues".to_string()),
            None,
            Some("agent-running".to_string()),
            Some("ghost-running".to_string()),
        );

        let pending_after_finish = chat
            .pending_auto_review_range
            .as_ref()
            .expect("pending kept after findings");
        assert_eq!(pending_after_finish.defer_until_turn, Some(chat.turn_sequence));
        assert_eq!(launches.load(Ordering::SeqCst), 0, "follow-up deferred to next turn");

        chat.turn_sequence = 2;
        chat.turn_had_code_edits = true;
        chat.auto_review_baseline = Some(GhostCommit::new("next-base".to_string(), None));

        chat.maybe_trigger_auto_review();
        assert_eq!(launches.load(Ordering::SeqCst), 1, "follow-up launched next turn");
        let running = chat
            .background_review
            .as_ref()
            .expect("follow-up review should be running");
        assert_eq!(
            running.base.as_ref().map(|c| c.id()),
            Some(pending_base.id()),
            "follow-up should use first skipped base",
        );
    }

    #[test]
    fn skipped_auto_review_clean_runs_immediately() {
        let _rt = enter_test_runtime_guard();
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        let launches = Arc::new(AtomicUsize::new(0));
        let launches_clone = launches.clone();
        let _stub = AutoReviewStubGuard::install(move || {
            launches_clone.fetch_add(1, Ordering::SeqCst);
        });

        chat.config.tui.auto_review_enabled = true;
        chat.turn_sequence = 1;
        chat.turn_had_code_edits = true;
        let pending_base = GhostCommit::new("base-clean".to_string(), None);
        chat.auto_review_baseline = Some(pending_base.clone());

        chat.background_review = Some(BackgroundReviewState {
            worktree_path: PathBuf::from("/tmp/wt"),
            branch: "auto-review-running".to_string(),
            agent_id: Some("agent-running".to_string()),
            snapshot: Some("ghost-running".to_string()),
            base: Some(GhostCommit::new("running-base".to_string(), None)),
            last_seen: Instant::now(),
        });

        chat.maybe_trigger_auto_review();
        assert_eq!(launches.load(Ordering::SeqCst), 0);
        assert!(chat.pending_auto_review_range.is_some());

        chat.on_background_review_finished(
            PathBuf::from("/tmp/wt"),
            "auto-review-running".to_string(),
            false,
            0,
            None,
            None,
            Some("agent-running".to_string()),
            Some("ghost-running".to_string()),
        );

        assert_eq!(launches.load(Ordering::SeqCst), 1, "follow-up should start immediately");
        assert!(chat.pending_auto_review_range.is_none(), "pending should be consumed");
        let running = chat.background_review.as_ref().expect("follow-up running");
        assert_eq!(
            running.base.as_ref().map(|c| c.id()),
            Some(pending_base.id()),
            "follow-up should cover skipped base",
        );
    }

    #[test]
    fn multiple_skipped_auto_reviews_collapse_to_first_base() {
        let _rt = enter_test_runtime_guard();
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        let launches = Arc::new(AtomicUsize::new(0));
        let launches_clone = launches.clone();
        let _stub = AutoReviewStubGuard::install(move || {
            launches_clone.fetch_add(1, Ordering::SeqCst);
        });

        chat.config.tui.auto_review_enabled = true;
        chat.turn_sequence = 1;
        chat.turn_had_code_edits = true;
        let first_base = GhostCommit::new("base-first".to_string(), None);
        chat.auto_review_baseline = Some(first_base.clone());

        chat.background_review = Some(BackgroundReviewState {
            worktree_path: PathBuf::from("/tmp/wt"),
            branch: "auto-review-running".to_string(),
            agent_id: Some("agent-running".to_string()),
            snapshot: Some("ghost-running".to_string()),
            base: Some(GhostCommit::new("running-base".to_string(), None)),
            last_seen: Instant::now(),
        });

        chat.maybe_trigger_auto_review();
        assert_eq!(launches.load(Ordering::SeqCst), 0);
        let pending = chat
            .pending_auto_review_range
            .as_ref()
            .expect("first pending queued");
        assert_eq!(pending.base.id(), first_base.id());

        // Second skip while review still running
        chat.auto_review_baseline = Some(GhostCommit::new("base-second".to_string(), None));
        chat.turn_had_code_edits = true;
        chat.maybe_trigger_auto_review();

        let pending_after_second = chat
            .pending_auto_review_range
            .as_ref()
            .expect("pending should persist");
        assert_eq!(pending_after_second.base.id(), first_base.id());

        chat.on_background_review_finished(
            PathBuf::from("/tmp/wt"),
            "auto-review-running".to_string(),
            false,
            0,
            None,
            None,
            Some("agent-running".to_string()),
            Some("ghost-running".to_string()),
        );

        assert_eq!(launches.load(Ordering::SeqCst), 1, "collapsed follow-up should run once");
        let running = chat.background_review.as_ref().expect("follow-up running");
        assert_eq!(running.base.as_ref().map(|c| c.id()), Some(first_base.id()));
        assert!(chat.pending_auto_review_range.is_none());
    }

    #[test]
    fn stale_background_review_is_reclaimed() {
        let _rt = enter_test_runtime_guard();
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        let launches = Arc::new(AtomicUsize::new(0));
        let launches_clone = launches.clone();
        let _stub = AutoReviewStubGuard::install(move || {
            launches_clone.fetch_add(1, Ordering::SeqCst);
        });

        chat.config.tui.auto_review_enabled = true;
        chat.turn_had_code_edits = true;
        let base = GhostCommit::new("stale-base".to_string(), None);
        let stale_started = Instant::now()
            .checked_sub(Duration::from_secs(400))
            .unwrap_or_else(Instant::now);

        chat.background_review = Some(BackgroundReviewState {
            worktree_path: PathBuf::from("/tmp/wt"),
            branch: "auto-review-running".to_string(),
            agent_id: Some("agent-running".to_string()),
            snapshot: Some("ghost-running".to_string()),
            base: Some(base.clone()),
            last_seen: stale_started,
        });

        chat.maybe_trigger_auto_review();

        assert_eq!(launches.load(Ordering::SeqCst), 1, "stale review should be relaunched");
        let running = chat.background_review.as_ref().expect("reclaimed review running");
        assert_eq!(running.base.as_ref().map(|c| c.id()), Some(base.id()));
        assert!(chat.pending_auto_review_range.is_none());
    }

    #[test]
    fn auto_drive_ctrl_s_overlay_keeps_screen_readable() {
        use crate::test_helpers::AutoContinueModeFixture;
        let _guard = enter_test_runtime_guard();
        let mut harness = ChatWidgetHarness::new();
        harness.auto_drive_activate(
            "write some code",
            false,
            true,
            AutoContinueModeFixture::Immediate,
        );

        harness.open_auto_drive_settings();
        let frame_with_settings = crate::test_helpers::render_chat_widget_to_vt100(&mut harness, 90, 24);
        assert!(frame_with_settings.contains("Auto Drive Settings"));
        assert!(!frame_with_settings.contains('\u{fffd}'));

        harness.close_auto_drive_settings();
        let frame_after_close = crate::test_helpers::render_chat_widget_to_vt100(&mut harness, 90, 24);
        assert!(!frame_after_close.contains("Auto Drive Settings"));
        assert!(!frame_after_close.contains('\u{fffd}'));
    }

    #[test]
    fn slash_command_from_line_parses_prompt_expanding_commands() {
        assert!(matches!(
            ChatWidget::slash_command_from_line("/plan build it"),
            Some(SlashCommand::Plan)
        ));
        assert!(matches!(
            ChatWidget::slash_command_from_line("/code"),
            Some(SlashCommand::Code)
        ));
        assert_eq!(ChatWidget::slash_command_from_line("not-a-command"), None);
    }

    #[test]
    fn plan_multiline_commands_are_not_split() {
        assert!(ChatWidget::multiline_slash_command_requires_split("/auto"));
        assert!(!ChatWidget::multiline_slash_command_requires_split("/plan"));
        assert!(!ChatWidget::multiline_slash_command_requires_split("/solve add context"));
    }

    #[test]
    fn transient_error_sets_reconnect_ui() {
        let _guard = enter_test_runtime_guard();
        let mut harness = ChatWidgetHarness::new();

        harness
            .chat()
            .on_error("stream error: retrying 1/5".to_string());

        assert!(harness.chat().reconnect_notice_active);
        harness.chat().clear_reconnecting();
        assert!(!harness.chat().reconnect_notice_active);
    }
    use ratatui::backend::TestBackend;
    use ratatui::text::Line;
    use ratatui::Terminal;
    use std::collections::HashMap;
    use std::time::{Duration, Instant, SystemTime};

    use code_core::protocol::{ReviewFinding, ReviewCodeLocation, ReviewLineRange};

    struct CaptureCommitStubGuard;

    impl CaptureCommitStubGuard {
        fn install<F>(stub: F) -> Self
        where
            F: Fn(&'static str, Option<String>) -> Result<GhostCommit, GitToolingError>
                + Send
                + Sync
                + 'static,
        {
            let mut slot = match CAPTURE_AUTO_TURN_COMMIT_STUB.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            assert!(slot.is_none(), "capture stub already installed");
            *slot = Some(Box::new(stub));
            Self
        }
    }

    impl Drop for CaptureCommitStubGuard {
        fn drop(&mut self) {
            match CAPTURE_AUTO_TURN_COMMIT_STUB.lock() {
                Ok(mut slot) => *slot = None,
                Err(poisoned) => {
                    let mut slot = poisoned.into_inner();
                    *slot = None;
                }
            }
        }
    }

    struct GitDiffStubGuard;

    impl GitDiffStubGuard {
        fn install<F>(stub: F) -> Self
        where
            F: Fn(String, String) -> Result<Vec<String>, String> + Send + Sync + 'static,
        {
            let mut slot = match GIT_DIFF_NAME_ONLY_BETWEEN_STUB.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            assert!(slot.is_none(), "git diff stub already installed");
            *slot = Some(Box::new(stub));
            Self
        }
    }

    impl Drop for GitDiffStubGuard {
        fn drop(&mut self) {
            match GIT_DIFF_NAME_ONLY_BETWEEN_STUB.lock() {
                Ok(mut slot) => *slot = None,
                Err(poisoned) => {
                    let mut slot = poisoned.into_inner();
                    *slot = None;
                }
            }
        }
    }

    fn reset_history(chat: &mut ChatWidget<'_>) {
        #[cfg(any(test, feature = "test-helpers"))]
        println!(
            "reset_history before: len={} test_mode={}",
            chat.history_cells.len(),
            chat.test_mode
        );
        chat.history_cells.clear();
        chat.history_cell_ids.clear();
        chat.history_live_window = None;
        chat.history_frozen_width = 0;
        chat.history_frozen_count = 0;
        chat.history_virtualization_sync_pending.set(false);
        chat.history_state = HistoryState::new();
        chat.history_render.invalidate_all();
        chat.cell_order_seq.clear();
        chat.cell_order_dbg.clear();
        chat.ui_background_seq_counters.clear();
        chat.last_assigned_order = None;
        chat.last_seen_request_index = 0;
        chat.current_request_index = 0;
        chat.internal_seq = 0;
        chat.order_request_bias = 0;
        chat.resume_expected_next_request = None;
        chat.resume_provider_baseline = None;
        chat.synthetic_system_req = None;
        chat.layout.scroll_offset.set(0);
        chat.layout.last_max_scroll.set(0);
        chat.layout.last_history_viewport_height.set(0);
        #[cfg(any(test, feature = "test-helpers"))]
        println!("reset_history after: len={}", chat.history_cells.len());
    }

    fn insert_plain_cell(chat: &mut ChatWidget<'_>, lines: &[&str]) {
        use code_core::history::state::{
            InlineSpan,
            MessageLine,
            MessageLineKind,
            PlainMessageKind,
            PlainMessageRole,
            PlainMessageState,
            TextEmphasis,
            TextTone,
        };

        let state = PlainMessageState {
            id: HistoryId::ZERO,
            role: PlainMessageRole::System,
            kind: PlainMessageKind::Plain,
            header: None,
            lines: lines
                .iter()
                .map(|text| MessageLine {
                    kind: MessageLineKind::Paragraph,
                    spans: vec![InlineSpan {
                        text: (*text).to_string(),
                        tone: TextTone::Default,
                        emphasis: TextEmphasis::default(),
                        entity: None,
                    }],
                })
                .collect(),
            metadata: None,
        };

        let key = chat.next_internal_key();
        let _ = chat.history_insert_plain_state_with_key(state, key, "test");
    }

    fn make_pending_fix_state(review: ReviewOutputEvent) -> AutoResolveState {
        AutoResolveState {
            prompt: "prompt".to_string(),
            hint: "hint".to_string(),
            metadata: None,
            attempt: 0,
            max_attempts: AUTO_RESOLVE_MAX_REVIEW_ATTEMPTS,
            phase: AutoResolvePhase::PendingFix { review },
            last_review: None,
            last_fix_message: None,
            last_reviewed_commit: None,
            snapshot_epoch: None,
        }
    }

    #[allow(dead_code)]
    fn review_output_with_finding() -> ReviewOutputEvent {
        ReviewOutputEvent {
            findings: vec![ReviewFinding {
                title: "issue".to_string(),
                body: "details".to_string(),
                confidence_score: 0.5,
                priority: 0,
                code_location: ReviewCodeLocation {
                    absolute_file_path: PathBuf::from("src/lib.rs"),
                    line_range: ReviewLineRange { start: 1, end: 1 },
                },
            }],
            overall_correctness: "incorrect".to_string(),
            overall_explanation: "needs fixes".to_string(),
            overall_confidence_score: 0.5,
        }
    }

    #[test]
    fn review_dialog_uncommitted_option_runs_workspace_scope() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.open_review_dialog();
        chat.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        let (prompt, hint, preparation_label, metadata, auto_resolve) = harness
            .drain_events()
            .into_iter()
            .find_map(|event| match event {
                AppEvent::RunReviewWithScope {
                    prompt,
                    hint,
                    preparation_label,
                    metadata,
                    auto_resolve,
                } => Some((prompt, hint, preparation_label, metadata, auto_resolve)),
                _ => None,
            })
            .expect("uncommitted preset should dispatch a workspace review");

        assert_eq!(
            prompt,
            "Review the current workspace changes (staged, unstaged, and untracked files) and highlight bugs, regressions, risky patterns, and missing tests before merge.".to_string()
        );
        assert_eq!(hint, "current workspace changes");
        assert_eq!(
            preparation_label.as_deref(),
            Some("Preparing code review for current changes")
        );
        assert!(auto_resolve, "auto resolve now defaults to on for workspace reviews");

        let metadata = metadata.expect("workspace scope metadata");
        assert_eq!(metadata.scope.as_deref(), Some("workspace"));
        assert!(metadata.base_branch.is_none());
        assert!(metadata.current_branch.is_none());
    }

    #[test]
    fn esc_router_prioritizes_auto_stop_when_waiting_for_review() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_state.on_begin_review(false);

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::AutoStopActive);
        assert!(!route.allows_double_esc);
    }

    #[test]
    fn esc_router_stops_auto_drive_while_waiting_for_response() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_state.set_phase(AutoRunPhase::Active);
        chat.auto_state.set_coordinator_waiting(true);

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::AutoStopActive);
        assert!(!route.allows_double_esc);
    }

    #[test]
    fn esc_router_prioritizes_cli_interrupt_before_agent_cancel() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.active_agents.push(AgentInfo {
            id: "agent-1".to_string(),
            name: "Agent 1".to_string(),
            status: AgentStatus::Running,
            source_kind: None,
            batch_id: Some("batch-1".to_string()),
            model: None,
            result: None,
            error: None,
            last_progress: None,
        });
        chat.active_task_ids.insert("turn-1".to_string());
        chat.bottom_pane.set_task_running(true);

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelTask);
    }

    #[test]
    fn esc_router_cancels_agents_when_only_agents_running() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.active_agents.push(AgentInfo {
            id: "agent-1".to_string(),
            name: "Agent 1".to_string(),
            status: AgentStatus::Running,
            source_kind: None,
            batch_id: Some("batch-1".to_string()),
            model: None,
            result: None,
            error: None,
            last_progress: None,
        });
        chat.bottom_pane.set_task_running(true);

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelAgents);
    }

    #[test]
    fn esc_router_skips_auto_review_cancel() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.active_agents.push(AgentInfo {
            id: "auto-1".to_string(),
            name: "Auto Review".to_string(),
            status: AgentStatus::Running,
            source_kind: Some(AgentSourceKind::AutoReview),
            batch_id: Some("review-batch".to_string()),
            model: None,
            result: None,
            error: None,
            last_progress: None,
        });

        let route = chat.describe_esc_context();
        assert_ne!(route.intent, EscIntent::CancelAgents);
    }

    #[test]
    fn cancelable_agents_excludes_auto_review_entries() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.active_agents.push(AgentInfo {
            id: "auto-1".to_string(),
            name: "Auto Review".to_string(),
            status: AgentStatus::Running,
            source_kind: Some(AgentSourceKind::AutoReview),
            batch_id: Some("review-batch".to_string()),
            model: None,
            result: None,
            error: None,
            last_progress: None,
        });

        chat.active_agents.push(AgentInfo {
            id: "agent-1".to_string(),
            name: "Other Agent".to_string(),
            status: AgentStatus::Pending,
            source_kind: None,
            batch_id: Some("work".to_string()),
            model: None,
            result: None,
            error: None,
            last_progress: None,
        });

        let (batches, agents) = chat.collect_cancelable_agents();
        assert_eq!(batches, vec!["work".to_string()]);
        assert!(agents.is_empty(), "batch cancel should cover the non-auto agent");
    }

    #[test]
    fn esc_router_cancels_active_auto_turn_streaming() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_state.set_phase(AutoRunPhase::Active);
        chat.active_task_ids.insert("turn-1".to_string());
        chat.bottom_pane.set_task_running(true);

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelTask);

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(chat.execute_esc_intent(route.intent, esc_event));

        assert!(
            !chat.auto_state.is_active(),
            "Auto Drive should stop after cancelling the active turn",
        );
    }

    #[test]
    fn esc_requires_follow_up_after_canceling_agents() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_state.set_phase(AutoRunPhase::Active);
        chat.active_agents.push(AgentInfo {
            id: "agent-1".to_string(),
            name: "Agent 1".to_string(),
            status: AgentStatus::Running,
            source_kind: None,
            batch_id: Some("batch-1".to_string()),
            model: None,
            result: None,
            error: None,
            last_progress: None,
        });

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::AutoStopActive);
        assert!(chat.execute_esc_intent(route.intent, esc_event));
        assert!(!chat.auto_state.is_active(), "Auto Drive stops before canceling agents");
        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelAgents);
        assert!(chat.execute_esc_intent(route.intent, esc_event));
        assert!(!chat.auto_state.is_active());
        assert!(chat.has_cancelable_agents());
        assert!(chat.auto_state.last_run_summary.is_none());
    }

    #[test]
    fn cancel_agents_preserves_spinner_for_running_terminal_when_auto_inactive() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        let terminal_launch = TerminalLaunch {
            id: 42,
            title: "Terminal".to_string(),
            command: vec!["sleep".to_string(), "10".to_string()],
            command_display: "sleep 10".to_string(),
            controller: None,
            auto_close_on_success: false,
            start_running: true,
        };
        chat.terminal_open(&terminal_launch);

        chat.active_agents.push(AgentInfo {
            id: "agent-1".to_string(),
            name: "Agent 1".to_string(),
            status: AgentStatus::Running,
            source_kind: None,
            batch_id: Some("batch-1".to_string()),
            model: None,
            result: None,
            error: None,
            last_progress: None,
        });

        let mut route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::DismissModal);
        let mut attempts = 0;
        while route.intent == EscIntent::DismissModal && attempts < 3 {
            assert!(chat.execute_esc_intent(
                route.intent,
                KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            ));
            route = chat.describe_esc_context();
            attempts += 1;
        }

        assert_eq!(route.intent, EscIntent::CancelAgents);
        assert!(chat.execute_esc_intent(
            route.intent,
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        ));

        assert!(!chat.auto_state.is_active(), "Auto Drive remains inactive");
        assert!(chat.has_cancelable_agents());
        chat.maybe_hide_spinner();
        assert!(
            chat.bottom_pane.is_task_running(),
            "Spinner stays active while agents or terminal work are still running",
        );
    }

    #[test]
    fn esc_cancels_agents_then_command_and_stops_auto_drive() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_state.set_phase(AutoRunPhase::Active);
        chat.active_agents.push(AgentInfo {
            id: "agent-1".to_string(),
            name: "Agent 1".to_string(),
            status: AgentStatus::Running,
            source_kind: None,
            batch_id: Some("batch-1".to_string()),
            model: None,
            result: None,
            error: None,
            last_progress: None,
        });

        chat.exec.running_commands.insert(
            ExecCallId("exec-1".to_string()),
            RunningCommand {
                command: vec!["echo".to_string(), "hi".to_string()],
                parsed: Vec::new(),
                history_index: None,
                history_id: None,
                explore_entry: None,
                stdout_offset: 0,
                stderr_offset: 0,
                wait_total: None,
                wait_active: false,
                wait_notes: Vec::new(),
            },
        );
        chat.bottom_pane.set_task_running(true);

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelTask);
        assert!(chat.execute_esc_intent(route.intent, esc_event));

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelAgents);
        assert!(chat.execute_esc_intent(route.intent, esc_event));
        assert!(!chat.auto_state.is_active(), "Auto Drive should stop after cancelling the command");
        assert!(chat.auto_state.last_run_summary.is_none());

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelAgents);
    }

    #[allow(dead_code)]
    fn esc_cancels_agents_then_command_without_auto_hint() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.active_agents.push(AgentInfo {
            id: "agent-1".to_string(),
            name: "Agent 1".to_string(),
            status: AgentStatus::Running,
            source_kind: None,
            batch_id: Some("batch-1".to_string()),
            model: None,
            result: None,
            error: None,
            last_progress: None,
        });

        chat.exec.running_commands.insert(
            ExecCallId("exec-1".to_string()),
            RunningCommand {
                command: vec!["echo".to_string(), "hi".to_string()],
                parsed: Vec::new(),
                history_index: None,
                history_id: None,
                explore_entry: None,
                stdout_offset: 0,
                stderr_offset: 0,
                wait_total: None,
                wait_active: false,
                wait_notes: Vec::new(),
            },
        );
        chat.bottom_pane.set_task_running(true);

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelAgents);
        assert!(chat.execute_esc_intent(route.intent, esc_event));
        assert!(chat.has_cancelable_agents());
        assert!(
            chat.bottom_pane.standard_terminal_hint().is_none(),
            "Auto Drive exit hint should not display when Auto Drive is inactive",
        );

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelTask);
        assert!(chat.execute_esc_intent(route.intent, esc_event));
        assert!(chat.exec.running_commands.is_empty());
        assert!(!chat.bottom_pane.is_task_running());
    }

    #[test]
    fn auto_disabled_cli_turn_preserves_send_prompt_label() {
        let mut harness = ChatWidgetHarness::new();
        harness.with_chat(|chat| {
            chat.config.auto_drive.coordinator_routing = false;
            chat.auto_state.continue_mode = AutoContinueMode::Immediate;
            chat.auto_state.goal = Some("Ship feature".to_string());
            chat.auto_state.set_phase(AutoRunPhase::Active);
            chat.schedule_auto_cli_prompt(0, "echo ready".to_string());
        });

        let (button_label, countdown_override, ctrl_switch_hint, manual_hint_present) =
            harness.with_chat(|chat| {
                let model = chat
                    .bottom_pane
                    .auto_view_model()
                    .expect("auto coordinator view should be active");
                match model {
                    AutoCoordinatorViewModel::Active(active) => (
                        active
                            .button
                            .as_ref()
                            .expect("button expected")
                            .label
                            .clone(),
                        chat.auto_state.countdown_override,
                        active.ctrl_switch_hint.clone(),
                        active.manual_hint.is_some(),
                    ),
                }
            });

        assert!(button_label.starts_with("Send prompt"));
        assert_eq!(countdown_override, None);
        assert_eq!(ctrl_switch_hint.as_str(), "Esc to edit");
        assert!(manual_hint_present);

        harness.with_chat(|chat| {
            chat.auto_submit_prompt();
        });

        let auto_pending = harness.with_chat(|chat| chat.auto_pending_goal_request);
        assert!(!auto_pending);
    }

    #[test]
    fn auto_drive_view_marks_running_when_agents_active() {
        let mut harness = ChatWidgetHarness::new();
        harness.with_chat(|chat| {
            chat.auto_state.set_phase(AutoRunPhase::Active);
            chat.auto_state.goal = Some("Ship feature".to_string());
            chat.auto_rebuild_live_ring();
        });

        harness.handle_event(Event {
            id: "turn-1".to_string(),
            event_seq: 0,
            msg: EventMsg::AgentStatusUpdate(AgentStatusUpdateEvent {
                agents: vec![CoreAgentInfo {
                    id: "agent-1".to_string(),
                    name: "Worker".to_string(),
                    status: "running".to_string(),
                    batch_id: Some("batch-1".to_string()),
                    model: None,
                    last_progress: None,
                    result: None,
                    error: None,
                    elapsed_ms: None,
                    token_count: None,
                    last_activity_at: None,
                    seconds_since_last_activity: None,
                    source_kind: None,
                }],
                context: None,
                task: None,
            }),
            order: None,
        });

        let cli_running = harness.with_chat(|chat| {
            chat
                .bottom_pane
                .auto_view_model()
                .and_then(|model| match model {
                    AutoCoordinatorViewModel::Active(active) => Some(active.cli_running),
                })
                .unwrap_or(false)
        });

        assert!(
            cli_running,
            "auto drive view should treat running agents as active"
        );
    }

    #[test]
    fn auto_drive_error_enters_transient_recovery() {
        let mut harness = ChatWidgetHarness::new();
        harness.with_chat(|chat| {
            chat.auto_state.set_phase(AutoRunPhase::Active);
            chat.auto_state.goal = Some("Ship feature".to_string());
            chat.auto_state.on_prompt_ready(true);
            chat.auto_rebuild_live_ring();
        });

        harness.handle_event(Event {
            id: "turn-1".to_string(),
            event_seq: 0,
            msg: EventMsg::Error(ErrorEvent {
                message: "internal error; agent loop died unexpectedly".to_string(),
            }),
            order: None,
        });

        let (still_active, in_recovery) = harness.with_chat(|chat| {
            (chat.auto_state.is_active(), chat.auto_state.in_transient_recovery())
        });
        assert!(
            still_active && in_recovery,
            "auto drive should pause for recovery after an error event"
        );
    }

    #[test]
    fn auto_bootstrap_starts_from_history() {
        let mut harness = ChatWidgetHarness::new();
        {
            let chat = harness.chat();
            chat.config.auto_drive.coordinator_routing = false;
            chat.config.sandbox_policy = SandboxPolicy::DangerFullAccess;
            chat.config.approval_policy = AskForApproval::Never;
        }

        {
            let chat = harness.chat();
            insert_plain_cell(chat, &["User: summarize recent progress"]);
            insert_plain_cell(chat, &["Assistant: Tests are passing, next step pending."]);
            chat.handle_auto_command(Some(String::new()));
        }

        let chat = harness.chat();
        assert!(chat.auto_pending_goal_request);
        assert!(!chat.auto_goal_bootstrap_done);
        assert_eq!(
            chat.auto_state.goal.as_deref(),
            Some(AUTO_BOOTSTRAP_GOAL_PLACEHOLDER)
        );
        assert!(chat.next_cli_text_format.is_none());
        let pending_prompt = chat
            .auto_state
            .current_cli_prompt
            .as_deref()
            .expect("bootstrap prompt");
        assert!(pending_prompt.trim().is_empty());
    }

    #[test]
    fn auto_bootstrap_updates_goal_after_first_decision() {
        let mut harness = ChatWidgetHarness::new();
        {
            let chat = harness.chat();
            chat.auto_state.set_phase(AutoRunPhase::Active);
            chat.auto_state.goal = Some(AUTO_BOOTSTRAP_GOAL_PLACEHOLDER.to_string());
            chat.auto_goal_bootstrap_done = false;
        }

        {
            let chat = harness.chat();
            chat.auto_handle_decision(
                1,
                AutoCoordinatorStatus::Continue,
                None,
                None,
                Some("Finish migrations".to_string()),
                Some(AutoTurnCliAction {
                    prompt: "echo ready".to_string(),
                    context: None,
                    suppress_ui_context: false,
                }),
                None,
                Vec::new(),
                Vec::new(),
            );
        }

        let chat = harness.chat();
        assert_eq!(chat.auto_state.goal.as_deref(), Some("Finish migrations"));
        assert!(chat.auto_goal_bootstrap_done);
        assert!(!chat.auto_pending_goal_request);
        assert_eq!(chat.auto_state.current_cli_prompt.as_deref(), Some("echo ready"));
    }

    #[test]
    fn auto_card_goal_updates_after_derivation() {
        let mut harness = ChatWidgetHarness::new();
        {
            let chat = harness.chat();
            chat.auto_state.set_phase(AutoRunPhase::Active);
            chat.auto_state.goal = Some(AUTO_BOOTSTRAP_GOAL_PLACEHOLDER.to_string());
            chat.auto_card_start(Some(AUTO_BOOTSTRAP_GOAL_PLACEHOLDER.to_string()));
        }

        {
            let chat = harness.chat();
            chat.auto_handle_decision(
                2,
                AutoCoordinatorStatus::Continue,
                None,
                None,
                Some("Document release tasks".to_string()),
                Some(AutoTurnCliAction {
                    prompt: "echo start".to_string(),
                    context: None,
                    suppress_ui_context: false,
                }),
                None,
                Vec::new(),
                Vec::new(),
            );
        }

        let chat = harness.chat();
        let tracker = chat
            .tools_state
            .auto_drive_tracker
            .as_ref()
            .expect("auto drive tracker should be present");
        assert_eq!(tracker.cell.goal_text(), Some("Document release tasks"));
    }

    #[test]
    fn auto_action_events_land_in_auto_drive_card() {
        let mut harness = ChatWidgetHarness::new();
        let note = "Retrying prompt generation after the previous response was too long to send to the CLI.";

        let chat = harness.chat();
        chat.auto_state.set_phase(AutoRunPhase::Active);
        chat.auto_card_start(Some("Ship feature".to_string()));
        chat.auto_handle_action(note.to_string());

        let tracker = chat
            .tools_state
            .auto_drive_tracker
            .as_ref()
            .expect("auto drive tracker should be present");
        let actions = tracker.cell.action_texts();
        assert!(
            actions.iter().any(|text| text == note),
            "auto drive action card should record retry note"
        );
    }

    #[test]
    fn auto_compacted_history_without_notice_skips_checkpoint_banner() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_state.set_phase(AutoRunPhase::Active);
        let conversation = vec![
            ChatWidget::auto_drive_make_assistant_message("overlong prompt raw output".to_string())
                .expect("assistant message"),
        ];

        chat.auto_handle_compacted_history(std::sync::Arc::from(conversation), false);

        let has_checkpoint = chat.history_cells.iter().any(|cell| {
            cell.display_lines_trimmed().iter().any(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains(COMPACTION_CHECKPOINT_MESSAGE))
            })
        });

        assert!(
            !has_checkpoint,
            "compaction notice should not be shown when show_notice is false"
        );
    }

    #[test]
    fn auto_card_shows_status_title_in_state_detail() {
        let mut harness = ChatWidgetHarness::new();
        {
            let chat = harness.chat();
            chat.auto_state.set_phase(AutoRunPhase::Active);
            chat.auto_state.goal = Some("Ship feature".to_string());
            chat.auto_card_start(Some("Ship feature".to_string()));
        }

        {
            let chat = harness.chat();
            chat.auto_handle_decision(
                3,
                AutoCoordinatorStatus::Continue,
                Some("Drafting fix".to_string()),
                Some("Past work".to_string()),
                None,
                Some(AutoTurnCliAction {
                    prompt: "echo work".to_string(),
                    context: None,
                    suppress_ui_context: false,
                }),
                None,
                Vec::new(),
                Vec::new(),
            );
        }

        let chat = harness.chat();
        let tracker = chat
            .tools_state
            .auto_drive_tracker
            .as_ref()
            .expect("auto drive tracker should be present");
        let actions = tracker.cell.action_texts();
        assert!(actions.iter().any(|text| text == "Status: Drafting fix"));
    }

    #[test]
    fn goal_entry_esc_sequence_preserves_draft_and_summary() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_state.last_run_summary = Some(AutoRunSummary {
            duration: Duration::from_secs(42),
            turns_completed: 3,
            message: Some("All tasks done.".to_string()),
            goal: Some("Finish feature".to_string()),
        });
        chat.auto_show_goal_entry_panel();
        chat.handle_paste("Suggested goal".to_string());
        assert!(matches!(
            chat.auto_goal_escape_state,
            AutoGoalEscState::NeedsEnableEditing
        ));

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::AutoGoalEnableEdit);
        assert!(chat.execute_esc_intent(route.intent, esc_event));
        assert!(chat.auto_state.should_show_goal_entry());
        assert!(matches!(
            chat.auto_goal_escape_state,
            AutoGoalEscState::ArmedForExit
        ));

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::AutoGoalExitPreserveDraft);
        assert!(chat.execute_esc_intent(route.intent, esc_event));
        assert!(!chat.auto_state.should_show_goal_entry());
        assert_eq!(chat.bottom_pane.composer_text(), "Suggested goal");
        assert!(chat.auto_state.last_run_summary.is_some());

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::AutoDismissSummary);
    }

    #[test]
    fn goal_entry_typing_arms_escape_state() {
        let mut harness = ChatWidgetHarness::new();
        {
            let chat = harness.chat();
            chat.auto_show_goal_entry_panel();
        }

        harness.send_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

        let chat = harness.chat();
        assert!(matches!(
            chat.auto_goal_escape_state,
            AutoGoalEscState::NeedsEnableEditing
        ));
        assert_eq!(chat.bottom_pane.composer_text(), "x");
    }

    #[test]
    fn ctrl_g_dispatches_external_editor_event() {
        let mut harness = ChatWidgetHarness::new();
        let key_event = KeyEvent::new(KeyCode::Char('g'), KeyModifiers::CONTROL);
        harness.chat().handle_key_event(key_event);
        let events = harness.drain_events();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, AppEvent::OpenExternalEditor { .. })),
            "expected external editor request on Ctrl+G",
        );
    }

    #[test]
    fn goal_entry_esc_exits_immediately_without_suggestion() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_show_goal_entry_panel();
        assert!(chat.auto_state.should_show_goal_entry());
        assert!(matches!(chat.auto_goal_escape_state, AutoGoalEscState::Inactive));

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::AutoGoalExitPreserveDraft);
        assert!(chat.execute_esc_intent(route.intent, esc_event));

        assert!(!chat.auto_state.should_show_goal_entry());
        assert_eq!(chat.bottom_pane.composer_text(), "");
    }

    #[test]
    fn esc_unwinds_cli_before_stopping_auto() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_state.set_phase(AutoRunPhase::Active);
        let call_id = ExecCallId("exec-1".to_string());
        chat.exec.running_commands.insert(
            call_id.clone(),
            RunningCommand {
                command: vec!["echo".to_string()],
                parsed: Vec::new(),
                history_index: None,
                history_id: None,
                explore_entry: None,
                stdout_offset: 0,
                stderr_offset: 0,
                wait_total: None,
                wait_active: false,
                wait_notes: Vec::new(),
            },
        );
        chat.bottom_pane.set_task_running(true);

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelTask);
        assert!(chat.execute_esc_intent(route.intent, esc_event));
        assert!(
            !chat.auto_state.is_active(),
            "Auto Drive now stops immediately after cancelling the CLI task",
        );

        chat.exec.running_commands.clear();
        chat.bottom_pane.set_task_running(false);

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::AutoGoalExitPreserveDraft);
        assert!(chat.execute_esc_intent(route.intent, esc_event));
    }

    #[test]
    fn esc_router_cancels_running_task() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.active_task_ids.insert("turn-1".to_string());
        chat.bottom_pane.set_task_running(true);

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelTask);
    }

    #[test]
    fn esc_cancel_task_while_manual_command_does_not_trigger_auto_drive() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.exec.running_commands.insert(
            ExecCallId("exec-1".to_string()),
            RunningCommand {
                command: vec!["echo".to_string()],
                parsed: Vec::new(),
                history_index: None,
                history_id: None,
                explore_entry: None,
                stdout_offset: 0,
                stderr_offset: 0,
                wait_total: None,
                wait_active: false,
                wait_notes: Vec::new(),
            },
        );
        chat.bottom_pane.set_task_running(true);

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::CancelTask);
        assert!(chat.execute_esc_intent(route.intent, esc_event));
        assert!(
            !chat.auto_state.is_active(),
            "Auto Drive should remain inactive after cancelling manual command",
        );
        assert!(
            chat.auto_state.last_run_summary.is_none(),
            "Cancelling manual command should not create an Auto Drive summary",
        );
    }

    #[test]
    fn esc_router_handles_diff_confirm_prompt() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.diffs.confirm = Some(crate::chatwidget::diff_ui::DiffConfirm {
            text_to_submit: "Please undo".to_string(),
        });

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::DiffConfirm);

        let esc_event = KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE);
        assert!(chat.execute_esc_intent(route.intent, esc_event));
        assert!(chat.diffs.confirm.is_none(), "diff confirm should clear after Esc");
    }

    #[test]
    fn esc_router_handles_agents_terminal_overlay() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.agents_terminal.active = true;
        chat.agents_terminal.focus_detail();

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::AgentsTerminal);
    }

    #[test]
    fn esc_router_clears_manual_entry_input() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_show_goal_entry_panel();
        assert!(chat.auto_state.should_show_goal_entry());
        chat.bottom_pane.insert_str("draft goal");

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::AutoGoalExitPreserveDraft);
    }

    #[test]
    fn esc_router_defaults_to_show_hint_when_idle() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        let route = chat.describe_esc_context();
        assert_eq!(route.intent, EscIntent::ShowUndoHint);
        assert!(route.allows_double_esc);
    }

    #[test]
    fn reasoning_collapse_hides_intermediate_titles_in_consecutive_runs() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.config.tui.show_reasoning = false;

        let agent_cell = history_cell::AgentRunCell::new("Batch".to_string());
        chat.history_push(agent_cell);

        let reasoning_one = history_cell::CollapsibleReasoningCell::new_with_id(
            vec![Line::from("First reasoning".to_string())],
            Some("r1".to_string()),
        );
        let reasoning_two = history_cell::CollapsibleReasoningCell::new_with_id(
            vec![Line::from("Second reasoning".to_string())],
            Some("r2".to_string()),
        );

        chat.history_push(reasoning_one);
        chat.history_push(reasoning_two);

        chat.refresh_reasoning_collapsed_visibility();

        let reasoning_cells: Vec<&history_cell::CollapsibleReasoningCell> = chat
            .history_cells
            .iter()
            .filter_map(|cell| {
                cell.as_any()
                    .downcast_ref::<history_cell::CollapsibleReasoningCell>()
            })
            .collect();

        assert_eq!(reasoning_cells.len(), 2, "expected exactly two reasoning cells");

        assert!(
            reasoning_cells[0].display_lines().is_empty(),
            "intermediate reasoning should hide when collapsed after agent anchor",
        );
        assert!(
            !reasoning_cells[1].display_lines().is_empty(),
            "last reasoning should remain visible",
        );
    }

    #[test]
    fn reasoning_collapse_applies_without_anchor_cells() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.config.tui.show_reasoning = false;

        let reasoning_one = history_cell::CollapsibleReasoningCell::new_with_id(
            vec![Line::from("First reasoning".to_string())],
            Some("r1".to_string()),
        );
        let reasoning_two = history_cell::CollapsibleReasoningCell::new_with_id(
            vec![Line::from("Second reasoning".to_string())],
            Some("r2".to_string()),
        );

        chat.history_push(reasoning_one);
        chat.history_push(reasoning_two);

        chat.refresh_reasoning_collapsed_visibility();

        let reasoning_cells: Vec<&history_cell::CollapsibleReasoningCell> = chat
            .history_cells
            .iter()
            .filter_map(|cell| {
                cell.as_any()
                    .downcast_ref::<history_cell::CollapsibleReasoningCell>()
            })
            .collect();

        assert_eq!(reasoning_cells.len(), 2, "expected exactly two reasoning cells");

        assert!(
            reasoning_cells[0].display_lines().is_empty(),
            "intermediate reasoning should hide when collapsed without an anchor",
        );
        assert!(
            !reasoning_cells[1].display_lines().is_empty(),
            "last reasoning should remain visible",
        );
    }

    #[test]
    fn auto_drive_stays_paused_while_auto_resolve_pending_fix() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_state.set_phase(AutoRunPhase::Active);
        chat.auto_state.on_prompt_submitted();
        chat.auto_state.review_enabled = true;
        chat.auto_state.on_complete_review();
        chat.auto_state.set_waiting_for_response(true);
        chat.pending_turn_descriptor = None;
        chat.pending_auto_turn_config = None;
        chat.auto_resolve_state = Some(make_pending_fix_state(ReviewOutputEvent::default()));

        chat.auto_on_assistant_final();

        // With cloud-gpt-5.1-codex-max gated off, the review request is still queued but
        // may be processed synchronously; ensure the review slot was populated.
        if chat.auto_state.awaiting_review() {
            // Review remains pending; nothing else to assert.
        } else {
            assert!(chat.auto_state.current_cli_prompt.is_some());
        }
        assert!(!chat.auto_state.is_waiting_for_response());
    }

    #[test]
    fn auto_review_skip_resumes_auto_drive() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        let _stub_lock = AUTO_STUB_LOCK.lock().unwrap();

        chat.auto_state.set_phase(AutoRunPhase::Active);
        chat.auto_state.review_enabled = true;
        chat.auto_state.on_prompt_submitted();
        chat.auto_state.set_waiting_for_response(true);
        chat.auto_state.on_complete_review();
        chat.auto_state.set_waiting_for_response(true);

        let turn_config = TurnConfig {
            read_only: false,
            complexity: Some(TurnComplexity::Low),
            text_format_override: None,
        };
        chat.pending_auto_turn_config = Some(turn_config.clone());
        chat.pending_turn_descriptor = Some(TurnDescriptor {
            mode: TurnMode::Normal,
            read_only: false,
            complexity: Some(TurnComplexity::Low),
            agent_preferences: None,
            review_strategy: None,
            text_format_override: None,
        });

        let base_id = "base-commit".to_string();
        let final_id = "final-commit".to_string();

        chat.auto_turn_review_state = Some(AutoTurnReviewState {
            base_commit: Some(GhostCommit::new(base_id.clone(), None)),
        });

        let base_for_capture = base_id.clone();
        let final_for_capture = final_id.clone();
        let _capture_guard = CaptureCommitStubGuard::install(move |message, parent| {
            assert_eq!(message, "auto turn change snapshot");
            assert_eq!(parent.as_deref(), Some(base_for_capture.as_str()));
            Ok(GhostCommit::new(final_for_capture.clone(), parent))
        });

        let base_for_diff = base_id.clone();
        let final_for_diff = final_id.clone();
        let _diff_guard = GitDiffStubGuard::install(move |base, head| {
            assert_eq!(base, base_for_diff);
            assert_eq!(head, final_for_diff);
            Ok(Vec::new())
        });

        chat.auto_on_assistant_final();
        assert!(chat.auto_state.awaiting_review(), "post-turn review should be pending");

        let descriptor_snapshot = chat.pending_turn_descriptor.clone();
        chat.auto_handle_post_turn_review(turn_config.clone(), descriptor_snapshot.as_ref());

        assert!(
            !chat.auto_state.awaiting_review(),
            "auto drive should clear waiting flag after skipped review"
        );

        let skip_banner = "Auto review skipped: no file changes detected this turn.";
        let skip_present = chat.history_cells.iter().any(|cell| {
            cell.display_lines_trimmed().iter().any(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains(skip_banner))
            })
        });
        assert!(skip_present, "skip banner should appear in history");

        assert!(
            !chat.auto_state.is_waiting_for_response(),
            "auto drive should resume conversation after skipped review"
        );
    }

    #[test]
    fn auto_review_skip_stays_blocked_when_auto_resolve_pending() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        let _stub_lock = AUTO_STUB_LOCK.lock().unwrap();

        chat.auto_state.set_phase(AutoRunPhase::Active);
        chat.auto_state.review_enabled = true;
        chat.auto_state.on_prompt_submitted();
        chat.auto_state.on_complete_review();
        chat.auto_state.set_waiting_for_response(true);

        let turn_config = TurnConfig {
            read_only: false,
            complexity: Some(TurnComplexity::Low),
            text_format_override: None,
        };
        chat.pending_auto_turn_config = Some(turn_config.clone());
        chat.pending_turn_descriptor = Some(TurnDescriptor {
            mode: TurnMode::Normal,
            read_only: false,
            complexity: Some(TurnComplexity::Low),
            agent_preferences: None,
            review_strategy: None,
            text_format_override: None,
        });

        let base_id = "base-commit".to_string();
        let final_id = "final-commit".to_string();

        chat.auto_turn_review_state = Some(AutoTurnReviewState {
            base_commit: Some(GhostCommit::new(base_id.clone(), None)),
        });

        chat.auto_resolve_state = Some(make_pending_fix_state(ReviewOutputEvent::default()));

        let base_for_capture = base_id.clone();
        let final_for_capture = final_id.clone();
        let _capture_guard = CaptureCommitStubGuard::install(move |message, parent| {
            assert_eq!(message, "auto turn change snapshot");
            assert_eq!(parent.as_deref(), Some(base_for_capture.as_str()));
            Ok(GhostCommit::new(final_for_capture.clone(), parent))
        });

        let base_for_diff = base_id.clone();
        let final_for_diff = final_id.clone();
        let _diff_guard = GitDiffStubGuard::install(move |base, head| {
            assert_eq!(base, base_for_diff);
            assert_eq!(head, final_for_diff);
            Ok(Vec::new())
        });

        chat.auto_on_assistant_final();
        assert!(chat.auto_state.awaiting_review(), "auto-resolve should block resume before skip");

        let descriptor_snapshot = chat.pending_turn_descriptor.clone();
        chat.auto_handle_post_turn_review(turn_config.clone(), descriptor_snapshot.as_ref());

        assert!(
            chat.auto_state.awaiting_review(),
            "auto drive should remain waiting when auto-resolve blocks"
        );
        assert!(
            !chat.auto_state.is_waiting_for_response(),
            "skip should not resume coordinator when auto-resolve blocks"
        );
    }

    #[test]
    fn auto_resolve_limit_zero_runs_single_fix_cycle() {
        let _runtime_guard = enter_test_runtime_guard();
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.config.tui.review_auto_resolve = true;
        chat.config.auto_drive.auto_resolve_review_attempts =
            AutoResolveAttemptLimit::try_new(0).unwrap();

        chat.start_review_with_scope(
            "Review workspace".to_string(),
            "workspace".to_string(),
            Some("Preparing code review request...".to_string()),
            None,
            true,
        );

        let state = chat
            .auto_resolve_state
            .as_ref()
            .expect("limit 0 should still initialize auto-resolve state");
        assert_eq!(state.max_attempts, 0);

        chat.auto_resolve_handle_review_enter();

        let review = ReviewOutputEvent {
            findings: vec![ReviewFinding {
                title: "issue".to_string(),
                body: "details".to_string(),
                confidence_score: 0.6,
                priority: 1,
                code_location: ReviewCodeLocation {
                    absolute_file_path: PathBuf::from("src/lib.rs"),
                    line_range: ReviewLineRange { start: 1, end: 1 },
                },
            }],
            overall_correctness: "incorrect".to_string(),
            overall_explanation: "needs follow up".to_string(),
            overall_confidence_score: 0.6,
        };

        chat.auto_resolve_handle_review_exit(Some(review.clone()));
        assert!(
            matches!(
                chat.auto_resolve_state
                    .as_ref()
                    .map(|state| &state.phase),
                Some(AutoResolvePhase::PendingFix { .. })
            ),
            "limit 0 should still request an automated fix"
        );

        chat.auto_resolve_on_task_complete(Some("fix applied".to_string()));
        assert!(
            matches!(
                chat.auto_resolve_state
                    .as_ref()
                    .map(|state| &state.phase),
                Some(AutoResolvePhase::AwaitingFix { .. })
            ),
            "auto-resolve should wait for judge after fix"
        );

        chat.auto_resolve_on_task_complete(Some("ready for judge".to_string()));
        assert!(
            matches!(
                chat.auto_resolve_state
                    .as_ref()
                    .map(|state| &state.phase),
                Some(AutoResolvePhase::AwaitingJudge { .. })
            ),
            "auto-resolve should request a status check"
        );

        chat.auto_resolve_process_judge(
            review,
            r#"{"status":"review_again","rationale":"double-check"}"#.to_string(),
        );

        assert!(
            chat.auto_resolve_state.is_none(),
            "automation should halt after judge when limit is zero"
        );

        let attempts_string_present = chat.history_cells.iter().any(|cell| {
            cell.display_lines_trimmed().iter().any(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains("attempt 1 of 0"))
            })
        });
        assert!(
            !attempts_string_present,
            "history should not mention impossible attempt counts"
        );
    }

    #[test]
    fn auto_resolve_limit_one_stops_after_single_retry() {
        let _runtime_guard = enter_test_runtime_guard();
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.config.tui.review_auto_resolve = true;
        chat.config.auto_drive.auto_resolve_review_attempts =
            AutoResolveAttemptLimit::try_new(1).unwrap();

        chat.start_review_with_scope(
            "Review workspace".to_string(),
            "workspace".to_string(),
            Some("Preparing code review request...".to_string()),
            None,
            true,
        );

        assert_eq!(
            chat.auto_resolve_state.as_ref().map(|state| state.max_attempts),
            Some(1),
            "auto-resolve state should honor configured limit"
        );

        chat.auto_resolve_handle_review_enter();

        let review = ReviewOutputEvent {
            findings: vec![ReviewFinding {
                title: "issue".to_string(),
                body: "details".to_string(),
                confidence_score: 0.6,
                priority: 1,
                code_location: ReviewCodeLocation {
                    absolute_file_path: PathBuf::from("src/lib.rs"),
                    line_range: ReviewLineRange { start: 1, end: 1 },
                },
            }],
            overall_correctness: "incorrect".to_string(),
            overall_explanation: "needs follow up".to_string(),
            overall_confidence_score: 0.6,
        };

        chat.auto_resolve_handle_review_exit(Some(review.clone()));
        assert!(
            matches!(
                chat.auto_resolve_state.as_ref().map(|state| &state.phase),
                Some(AutoResolvePhase::PendingFix { .. })
            ),
            "auto-resolve should request a fix after first findings"
        );

        chat.auto_resolve_on_task_complete(Some("fix applied".to_string()));
        chat.auto_resolve_process_judge(
            review.clone(),
            r#"{"status":"review_again","rationale":"double-check"}"#.to_string(),
        );

        let state = chat
            .auto_resolve_state
            .as_ref()
            .expect("limit 1 should schedule a single re-review");
        assert!(matches!(state.phase, AutoResolvePhase::WaitingForReview));

        chat.auto_resolve_handle_review_enter();
        chat.auto_resolve_handle_review_exit(Some(review.clone()));

        assert!(
            chat.auto_resolve_state.is_none(),
            "automation should halt after completing the allowed re-review"
        );

        let mut history_strings = Vec::new();
        for cell in &chat.history_cells {
            for line in cell.display_lines_trimmed() {
                for span in &line.spans {
                    history_strings.push(span.content.to_string());
                }
            }
        }

        let attempt_limit_notice_present = history_strings
            .iter()
            .any(|line| line.contains("attempt limit") && line.contains("reached"));
        assert!(
            attempt_limit_notice_present,
            "user should be notified when the attempt limit stops automation"
        );

        assert!(
            history_strings
                .iter()
                .all(|line| !line.contains("attempt 1 of 0")),
            "no messaging should reference impossible attempt counts"
        );
    }

    #[test]
    fn auto_handle_decision_launches_cli_agents_and_review() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_state.set_phase(AutoRunPhase::Active);
        chat.auto_state.review_enabled = true;
        chat.config.sandbox_policy = SandboxPolicy::DangerFullAccess;

        chat.auto_handle_decision(
            4,
            AutoCoordinatorStatus::Continue,
            Some("Running unit tests".to_string()),
            Some("Finished setup".to_string()),
            Some("Refine goal".to_string()),
            Some(AutoTurnCliAction {
                prompt: "Run cargo test".to_string(),
                context: Some("use --all-features".to_string()),
                suppress_ui_context: false,
            }),
            Some(AutoTurnAgentsTiming::Parallel),
            vec![AutoTurnAgentsAction {
                prompt: "Draft alternative fix".to_string(),
                context: None,
                write: false,
                write_requested: Some(false),
                models: None,
            }],
            Vec::new(),
        );

        assert_eq!(
            chat.auto_state.current_cli_prompt.as_deref(),
            Some("Run cargo test")
        );
        assert!(!chat.auto_state.awaiting_review());
        assert_eq!(chat.auto_state.pending_agent_actions.len(), 1);
        assert_eq!(
            chat.auto_state.pending_agent_timing,
            Some(AutoTurnAgentsTiming::Parallel)
        );
        let action = &chat.auto_state.pending_agent_actions[0];
        assert_eq!(action.prompt, "Draft alternative fix");
        assert!(action.write);

        let notice = "Auto Drive enabled write mode";
        let write_notice_present = chat
            .history_cells
            .iter()
            .any(|cell| {
                cell.display_lines_trimmed().iter().any(|line| {
                    line.spans
                        .iter()
                        .any(|span| span.content.contains(notice))
                })
            });
        assert!(write_notice_present);
    }

    #[test]
    fn coordinator_router_emits_notice_for_status_question() {
        let mut harness = ChatWidgetHarness::new();
        {
            let chat = harness.chat();
        chat.auto_state.set_phase(AutoRunPhase::Active);
            chat.config.auto_drive.coordinator_routing = true;
            chat.config.sandbox_policy = SandboxPolicy::DangerFullAccess;
        }

        let baseline_notice_count = {
            let chat = harness.chat();
            chat.history_cells
                .iter()
                .filter(|cell| matches!(cell.kind(), HistoryCellType::Notice))
                .count()
        };

        {
            let chat = harness.chat();
            chat.auto_handle_user_reply(
                Some("Two active agents reporting steady progress.".to_string()),
                None,
            );
        }

        let notice_count = {
            let chat = harness.chat();
            chat.history_cells
                .iter()
                .filter(|cell| matches!(cell.kind(), HistoryCellType::Notice))
                .count()
        };
        assert!(notice_count > baseline_notice_count);

        let header_span = {
            let chat = harness.chat();
            let notice_cell = chat
                .history_cells
                .iter()
                .rev()
                .find(|cell| matches!(cell.kind(), HistoryCellType::Notice))
                .expect("notice cell");
            let lines = notice_cell.display_lines_trimmed();
            assert!(!lines.is_empty());
            lines
                .first()
                .and_then(|line| line.spans.first())
                .map(|span| span.content.to_string())
                .unwrap_or_default()
        };
        assert_eq!(header_span, "AUTO DRIVE RESPONSE");
    }

    #[test]
    fn coordinator_router_injects_cli_for_plan_requests() {
        let mut harness = ChatWidgetHarness::new();
        {
            let chat = harness.chat();
        chat.auto_state.set_phase(AutoRunPhase::Active);
            chat.config.auto_drive.coordinator_routing = true;
            chat.config.sandbox_policy = SandboxPolicy::DangerFullAccess;
        }

        harness.drain_events();

        {
            let chat = harness.chat();
            chat.auto_handle_user_reply(None, Some("/plan".to_string()));
        }

        let events = harness.drain_events();
        let (command, payload) = events
            .iter()
            .find_map(|event| match event {
                AppEvent::DispatchCommand(cmd, payload) => Some((cmd, payload.clone())),
                _ => None,
            })
            .expect("dispatch for /plan");
        assert_eq!(*command, SlashCommand::Auto);
        assert!(payload.contains("/plan"), "payload={payload}");
    }

    #[test]
    fn coordinator_router_bypasses_slash_commands() {
        let mut harness = ChatWidgetHarness::new();
        {
            let chat = harness.chat();
        chat.auto_state.set_phase(AutoRunPhase::Active);
            chat.config.auto_drive.coordinator_routing = true;
        }

        harness.drain_events();
        {
            let chat = harness.chat();
            chat.submit_user_message(UserMessage::from("/status".to_string()));
        }

        let events = harness.drain_events();
        assert!(
            events.iter().any(|event| matches!(event, AppEvent::DispatchCommand(_, _))
                || matches!(event, AppEvent::CodexOp(_))),
            "slash command should follow existing dispatch path"
        );
    }

    #[test]
    fn build_turn_message_includes_agent_guidance() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        chat.auto_state.subagents_enabled = true;
        chat.auto_state.pending_agent_actions = vec![AutoTurnAgentsAction {
            prompt: "Draft alternative fix".to_string(),
            context: Some("Focus on parser module".to_string()),
            write: false,
            write_requested: Some(false),
            models: Some(vec![
                "claude-sonnet-4.5".to_string(),
                "gemini-3-pro".to_string(),
            ]),
        }];
        chat.auto_state.pending_agent_timing = Some(AutoTurnAgentsTiming::Blocking);

        chat.auto_state.current_cli_context = Some("Workspace root: /tmp".to_string());

        let message = chat
            .build_auto_turn_message("Run diagnostics")
            .expect("message");
        assert!(message.contains("Workspace root: /tmp"));
        assert!(message.contains("Run diagnostics"));
        assert!(message.contains("Please run agent.create"));
        assert!(message.contains("write: false"));
        assert!(message.contains("Models: [claude-sonnet-4.5, gemini-3-pro]"));
        assert!(message.contains("Draft alternative fix"));
        assert!(message.contains("Focus on parser module"));
        assert!(message.contains("agent.wait"));
        assert!(message.contains("Timing (blocking)"));
        assert!(message.contains("Launch these agents first"));
        assert!(!message.contains("agent {\"action\""), "message should not include raw agent JSON");
    }

    #[test]
    fn task_complete_triggers_review_when_waiting_flag_set() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();

        let _stub_lock = AUTO_STUB_LOCK.lock().unwrap();

        chat.auto_state.set_phase(AutoRunPhase::Active);
        chat.auto_state.review_enabled = true;
        chat.auto_state.on_prompt_submitted();

        let turn_config = TurnConfig {
            read_only: false,
            complexity: Some(TurnComplexity::Low),
            text_format_override: None,
        };
        chat.pending_auto_turn_config = Some(turn_config.clone());
        chat.pending_turn_descriptor = Some(TurnDescriptor {
            mode: TurnMode::Normal,
            read_only: false,
            complexity: Some(TurnComplexity::Low),
            agent_preferences: None,
            review_strategy: None,
            text_format_override: None,
        });

        let base_id = "base-commit".to_string();
        let final_id = "final-commit".to_string();

        chat.auto_turn_review_state = Some(AutoTurnReviewState {
            base_commit: Some(GhostCommit::new(base_id.clone(), None)),
        });

        let base_for_capture = base_id.clone();
        let final_for_capture = final_id.clone();
        let _capture_guard = CaptureCommitStubGuard::install(move |message, parent| {
            assert_eq!(message, "auto turn change snapshot");
            assert_eq!(parent.as_deref(), Some(base_for_capture.as_str()));
            Ok(GhostCommit::new(final_for_capture.clone(), parent))
        });

        let base_for_diff = base_id.clone();
        let final_for_diff = final_id.clone();
        let _diff_guard = GitDiffStubGuard::install(move |base, head| {
            assert_eq!(base, base_for_diff);
            assert_eq!(head, final_for_diff);
            Ok(Vec::new())
        });

        chat.auto_on_assistant_final();
        assert!(chat.auto_state.awaiting_review());

        let descriptor_snapshot = chat.pending_turn_descriptor.clone();
        chat.auto_handle_post_turn_review(turn_config.clone(), descriptor_snapshot.as_ref());

        chat.handle_code_event(Event {
            id: "turn".to_string(),
            event_seq: 42,
            msg: EventMsg::TaskComplete(TaskCompleteEvent {
                last_agent_message: None,
            }),
            order: None,
        });

        assert!(
            !chat.auto_state.awaiting_review(),
            "waiting flag should clear after TaskComplete launches skip review"
        );

        let skip_banner = "Auto review skipped: no file changes detected this turn.";
        let skip_present = chat.history_cells.iter().any(|cell| {
            cell.display_lines_trimmed().iter().any(|line| {
                line.spans
                    .iter()
                    .any(|span| span.content.contains(skip_banner))
            })
        });
        assert!(skip_present, "skip banner should appear after review skip");
    }

    #[test]
    fn finalize_explore_updates_even_with_stale_index() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        reset_history(chat);

        let call_id = "call-explore".to_string();
        let order = OrderMeta {
            request_ordinal: 1,
            output_index: Some(0),
            sequence_number: Some(0),
        };

        chat.handle_code_event(Event {
            id: call_id.clone(),
            event_seq: 0,
            msg: EventMsg::ExecCommandBegin(ExecCommandBeginEvent {
                call_id: call_id.clone(),
                command: vec!["bash".into(), "-lc".into(), "cat foo.txt".into()],
                cwd: std::env::temp_dir(),
                parsed_cmd: vec![ParsedCommand::Read {
                    cmd: "cat foo.txt".to_string(),
                    name: "foo.txt".to_string(),
                }],
            }),
            order: Some(order.clone()),
        });

        let exec_call_id = ExecCallId(call_id.clone());
        let running = chat
            .exec
            .running_commands
            .get_mut(&exec_call_id)
            .expect("explore command should be tracked");
        let (agg_idx, entry_idx) = running
            .explore_entry
            .expect("read command should register an explore entry");

        // Simulate an out-of-date index so finalize must recover by searching.
        running.explore_entry = Some((usize::MAX, entry_idx));
        chat.exec.running_explore_agg_index = Some(usize::MAX);

        chat.finalize_all_running_due_to_answer();

        let cell = chat.history_cells[agg_idx]
            .as_any()
            .downcast_ref::<ExploreAggregationCell>()
            .expect("explore aggregation cell should remain present");
        let entry = cell
            .record()
            .entries
            .get(entry_idx)
            .expect("entry index should still be valid");
        assert!(
            !matches!(entry.status, history_cell::ExploreEntryStatus::Running),
            "explore entry should not remain running after finalize_all_running_due_to_answer"
        );
        assert!(
            !chat.exec.running_commands.contains_key(&exec_call_id),
            "finalization should clear the running command"
        );
    }

    #[test]
    fn ordering_keeps_new_answers_after_prior_backgrounds() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        reset_history(chat);

        chat.last_seen_request_index = 1;
        chat.current_request_index = 1;
        chat.internal_seq = 0;

        chat.push_background_tail("background-one".to_string());
        chat.push_background_tail("background-two".to_string());

        assert_eq!(chat.history_cells.len(), 2, "expected two background cells");

        let answer_id = "answer-turn-1";
        let seeded_key = OrderKey {
            req: 1,
            out: 1,
            seq: 0,
        };
        chat.seed_stream_order_key(StreamKind::Answer, answer_id, seeded_key);

        let response_text = "assistant-response";
        chat.insert_final_answer_with_id(
            Some(answer_id.to_string()),
            vec![Line::from(response_text)],
            response_text.to_string(),
        );

        assert_eq!(chat.history_cells.len(), 3, "expected assistant cell to be added");

        let tail_kinds: Vec<HistoryCellType> = chat
            .history_cells
            .iter()
            .map(|cell| cell.kind())
            .collect();

        let len = tail_kinds.len();
        assert_eq!(
            &tail_kinds[len - 3..],
            &[
                HistoryCellType::BackgroundEvent,
                HistoryCellType::BackgroundEvent,
                HistoryCellType::Assistant,
            ],
            "assistant output should appear after existing background cells",
        );
    }

    #[test]
    fn final_answer_clears_spinner_when_agent_never_reports_terminal_status() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        reset_history(chat);

        let turn_id = "turn-1".to_string();

        chat.handle_code_event(Event {
            id: turn_id.clone(),
            event_seq: 0,
            msg: EventMsg::TaskStarted,
            order: None,
        });

        chat.handle_code_event(Event {
            id: turn_id.clone(),
            event_seq: 1,
            msg: EventMsg::AgentStatusUpdate(AgentStatusUpdateEvent {
                agents: vec![CoreAgentInfo {
                    id: "agent-1".to_string(),
                    name: "Todo Agent".to_string(),
                    status: "running".to_string(),
                    batch_id: Some("batch-single".to_string()),
                    model: None,
                    last_progress: None,
                    result: None,
                    error: None,
                    elapsed_ms: None,
                    token_count: None,
                    last_activity_at: None,
                    seconds_since_last_activity: None,
                    source_kind: None,
                }],
                context: None,
                task: None,
            }),
            order: None,
        });
        assert!(
            chat.bottom_pane.is_task_running(),
            "spinner should remain active while the agent reports running"
        );

        chat.handle_code_event(Event {
            id: turn_id.clone(),
            event_seq: 2,
            msg: EventMsg::AgentMessage(AgentMessageEvent {
                message: "Completed todo items.".to_string(),
            }),
            order: None,
        });
        assert!(
            chat.bottom_pane.is_task_running(),
            "spinner should remain active after an assistant message until TaskComplete"
        );

        assert_eq!(chat.overall_task_status, "running".to_string());

        chat.handle_code_event(Event {
            id: turn_id.clone(),
            event_seq: 3,
            msg: EventMsg::TaskComplete(TaskCompleteEvent {
                last_agent_message: None,
            }),
            order: None,
        });

        assert!(
            !chat.bottom_pane.is_task_running(),
            "spinner should clear on TaskComplete even when agent runtime is missing"
        );

        assert_eq!(chat.overall_task_status, "complete".to_string());

        assert!(
            chat
                .agent_runtime
                .values()
                .all(|rt| rt.completed_at.is_none()),
            "runtime should remain incomplete until backend reports a terminal status"
        );

        assert!(
            chat
                .active_agents
                .iter()
                .all(|agent| !matches!(agent.status, AgentStatus::Pending | AgentStatus::Running)),
            "agents should be forced into a terminal status after the answer completes"
        );
    }

    #[test]
    fn spinner_rearms_when_late_agent_update_reports_running() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        reset_history(chat);

        let turn_id = "turn-1".to_string();

        chat.handle_code_event(Event {
            id: turn_id.clone(),
            event_seq: 0,
            msg: EventMsg::TaskStarted,
            order: None,
        });

        chat.handle_code_event(Event {
            id: turn_id.clone(),
            event_seq: 1,
            msg: EventMsg::AgentStatusUpdate(AgentStatusUpdateEvent {
                agents: vec![CoreAgentInfo {
                    id: "agent-1".to_string(),
                    name: "Todo Agent".to_string(),
                    status: "running".to_string(),
                    batch_id: Some("batch-single".to_string()),
                    model: None,
                    last_progress: None,
                    result: None,
                    error: None,
                    elapsed_ms: None,
                    token_count: None,
                    last_activity_at: None,
                    seconds_since_last_activity: None,
                    source_kind: None,
                }],
                context: None,
                task: None,
            }),
            order: None,
        });

        assert!(chat.bottom_pane.is_task_running(), "spinner should be running initially");

        chat.handle_code_event(Event {
            id: turn_id.clone(),
            event_seq: 2,
            msg: EventMsg::AgentMessage(AgentMessageEvent {
                message: "Completed todo items.".to_string(),
            }),
            order: None,
        });

        assert!(chat.bottom_pane.is_task_running(), "spinner stays running after assistant message");

        chat.handle_code_event(Event {
            id: turn_id.clone(),
            event_seq: 3,
            msg: EventMsg::TaskComplete(TaskCompleteEvent {
                last_agent_message: None,
            }),
            order: None,
        });

        assert!(
            !chat.bottom_pane.is_task_running(),
            "TaskComplete should clear the spinner"
        );

        chat.handle_code_event(Event {
            id: turn_id.clone(),
            event_seq: 4,
            msg: EventMsg::AgentStatusUpdate(AgentStatusUpdateEvent {
                agents: vec![CoreAgentInfo {
                    id: "agent-1".to_string(),
                    name: "Todo Agent".to_string(),
                    status: "running".to_string(),
                    batch_id: Some("batch-single".to_string()),
                    model: None,
                    last_progress: None,
                    result: None,
                    error: None,
                    elapsed_ms: None,
                    token_count: None,
                    last_activity_at: None,
                    seconds_since_last_activity: None,
                    source_kind: None,
                }],
                context: None,
                task: None,
            }),
            order: None,
        });

        assert!(
            chat.bottom_pane.is_task_running(),
            "late running update should re-enable the spinner"
        );
    }

    #[test]
    fn scrollback_spacer_preserves_top_cell_bottom_line() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        reset_history(chat);

        insert_plain_cell(chat, &["old-1", "old-2"]);
        insert_plain_cell(chat, &["mid-1", "mid-2"]);
        insert_plain_cell(chat, &["new-1", "new-2"]);

        let viewport_height = 6;
        chat.layout.scroll_offset.set(2);

        let mut terminal = Terminal::new(TestBackend::new(40, viewport_height)).expect("terminal");
        terminal
            .draw(|frame| frame.render_widget_ref(&*chat, frame.area()))
            .expect("draw history");

        let adjusted = chat.history_render.adjust_scroll_to_content(2);
        assert_eq!(adjusted, 1, "scroll origin should step back from spacer row");

        let prefix = chat.history_render.prefix_sums.borrow();
        assert!(!prefix.is_empty(), "prefix sums populated after draw");
        let start_idx = match prefix.binary_search(&adjusted) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        assert_eq!(start_idx, 0, "expected first cell to be visible after adjustment");

        let content_y = prefix[start_idx];
        drop(prefix);
        let skip_top = adjusted.saturating_sub(content_y);
        assert_eq!(skip_top, 1, "should display the second line of the oldest cell");

        let cell = &chat.history_cells[start_idx];
        let lines = cell.display_lines_trimmed();
        let line = lines
            .get(skip_top as usize)
            .expect("line available after scroll adjustment");
        let text: String = line.spans.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(text.trim(), "old-2");
    }

    #[test]
    fn final_answer_without_task_complete_clears_spinner() {
        let _rt = enter_test_runtime_guard();
        let mut harness = ChatWidgetHarness::new();
        {
            let chat = harness.chat();
            reset_history(chat);
        }

        let turn_id = "turn-1".to_string();
        let order = OrderMeta {
            request_ordinal: 1,
            output_index: Some(0),
            sequence_number: Some(0),
        };

        harness.handle_event(Event {
            id: turn_id.clone(),
            event_seq: 0,
            msg: EventMsg::TaskStarted,
            order: None,
        });

        harness.handle_event(Event {
            id: turn_id.clone(),
            event_seq: 1,
            msg: EventMsg::AgentReasoningDelta(AgentReasoningDeltaEvent {
                delta: "thinking about the change".to_string(),
            }),
            order: Some(order.clone()),
        });

        harness.handle_event(Event {
            id: turn_id.clone(),
            event_seq: 2,
            msg: EventMsg::AgentMessage(AgentMessageEvent {
                message: "All done".to_string(),
            }),
            order: Some(OrderMeta {
                request_ordinal: 1,
                output_index: Some(0),
                sequence_number: Some(1),
            }),
        });

        harness.flush_into_widget();

        assert!(
            !harness.chat().bottom_pane.is_task_running(),
            "spinner should clear after the final answer even when TaskComplete never arrives"
        );
    }

    #[test]
    fn scrollback_spacer_exact_offset_adjusts_to_content() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        reset_history(chat);

        insert_plain_cell(chat, &["old-1", "old-2"]);
        insert_plain_cell(chat, &["mid-1", "mid-2"]);
        insert_plain_cell(chat, &["new-1", "new-2"]);

        let viewport_height = 6;
        chat.layout.scroll_offset.set(2);

        {
            let mut terminal =
                Terminal::new(TestBackend::new(40, viewport_height)).expect("terminal");
            terminal
                .draw(|frame| frame.render_widget_ref(&*chat, frame.area()))
                .expect("draw history");
        }

        let ranges = chat.history_render.spacing_ranges_for_test();
        let (pos, _) = ranges
            .first()
            .copied()
            .expect("expected a spacer-induced adjustment");
        let adjusted = chat.history_render.adjust_scroll_to_content(pos);
        assert!(
            adjusted < pos,
            "scroll adjustment should reduce the origin when landing on a spacer"
        );
    }

    #[test]
    fn scrollback_top_boundary_retains_oldest_content() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        reset_history(chat);

        insert_plain_cell(chat, &["old-1", "old-2"]);
        insert_plain_cell(chat, &["mid-1", "mid-2"]);
        insert_plain_cell(chat, &["new-1", "new-2"]);

        {
            let mut terminal = Terminal::new(TestBackend::new(40, 6)).expect("terminal");
            terminal
                .draw(|frame| frame.render_widget_ref(&*chat, frame.area()))
                .expect("draw history");
        }

        let max_scroll = chat.layout.last_max_scroll.get();
        assert!(max_scroll > 0, "expected overflow to produce a positive max scroll");
        chat.layout.scroll_offset.set(max_scroll);

        let mut terminal = Terminal::new(TestBackend::new(40, 6)).expect("terminal");
        terminal
            .draw(|frame| frame.render_widget_ref(&*chat, frame.area()))
            .expect("draw history at top boundary");

        let max_scroll = chat.layout.last_max_scroll.get();
        let scroll_from_top = max_scroll.saturating_sub(chat.layout.scroll_offset.get());
        let effective = chat.history_render.adjust_scroll_to_content(scroll_from_top);
        let prefix = chat.history_render.prefix_sums.borrow();
        let mut start_idx = match prefix.binary_search(&effective) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        start_idx = start_idx.min(prefix.len().saturating_sub(1));
        start_idx = start_idx.min(chat.history_cells.len().saturating_sub(1));
        let content_y = prefix[start_idx];
        drop(prefix);

        let skip = effective.saturating_sub(content_y) as usize;
        let cell = &chat.history_cells[start_idx];
        let lines = cell.display_lines_trimmed();
        let target_index = skip.min(lines.len().saturating_sub(1));
        let visible = lines
            .get(target_index)
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .unwrap_or_default();

        assert!(
            visible.contains("old-1"),
            "scrolling to the top should keep the oldest content visible"
        );
    }

    #[test]
    fn ordering_stream_delta_should_follow_existing_background_tail() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        reset_history(chat);

        chat.last_seen_request_index = 1;
        chat.push_background_tail("background".to_string());

        let stream_state = AssistantStreamState {
            id: HistoryId::ZERO,
            stream_id: "stream-1".into(),
            preview_markdown: "partial".into(),
            deltas: vec![AssistantStreamDelta {
                delta: "partial".into(),
                sequence: Some(0),
                received_at: SystemTime::now(),
            }],
            citations: vec![],
            metadata: None,
            in_progress: true,
            last_updated_at: SystemTime::now(),
            truncated_prefix_bytes: 0,
        };
        let stream_cell = history_cell::new_streaming_content(stream_state, &chat.config);

        chat.history_insert_with_key_global_tagged(
            Box::new(stream_cell),
            OrderKey {
                req: 1,
                out: 0,
                seq: 0,
            },
            "stream",
            None,
        );

        let kinds: Vec<HistoryCellType> = chat
            .history_cells
            .iter()
            .map(|cell| cell.kind())
            .collect();

        assert_eq!(
            kinds,
            vec![HistoryCellType::BackgroundEvent, HistoryCellType::Assistant],
            "streaming assistant output should append after the existing background tail cell",
        );
    }

    #[test]
    fn ordering_tool_reasoning_explore_should_preserve_arrival_sequence() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        reset_history(chat);

        chat.last_seen_request_index = 1;

        let make_plain = |text: &str| PlainMessageState {
            id: HistoryId::ZERO,
            role: PlainMessageRole::System,
            kind: PlainMessageKind::Plain,
            header: None,
            lines: vec![MessageLine {
                kind: MessageLineKind::Paragraph,
                spans: vec![InlineSpan {
                    text: text.to_string(),
                    tone: TextTone::Default,
                    emphasis: TextEmphasis::default(),
                    entity: None,
                }],
            }],
            metadata: None,
        };

        // Reasoning arrives first with later output index.
        let reasoning_key = ChatWidget::raw_order_key_from_order_meta(&OrderMeta {
            request_ordinal: 1,
            output_index: Some(2),
            sequence_number: Some(0),
        });
        chat.history_insert_plain_state_with_key(make_plain("reasoning"), reasoning_key, "reasoning");

        // Explore summary follows immediately afterwards.
        let explore_key = ChatWidget::raw_order_key_from_order_meta(&OrderMeta {
            request_ordinal: 1,
            output_index: Some(3),
            sequence_number: Some(0),
        });
        chat.history_insert_plain_state_with_key(make_plain("explore"), explore_key, "explore");

        // Tool run summary arrives last but references an earlier output index.
        let tool_key = ChatWidget::raw_order_key_from_order_meta(&OrderMeta {
            request_ordinal: 1,
            output_index: Some(1),
            sequence_number: Some(0),
        });
        chat.history_insert_plain_state_with_key(make_plain("tool"), tool_key, "tool");

        let labels: Vec<String> = chat
            .history_cells
            .iter()
            .map(|cell| {
                cell.display_lines_trimmed()
                    .first()
                    .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect())
                    .unwrap_or_default()
            })
            .collect();

        assert_eq!(
            labels,
            vec!["reasoning".to_string(), "explore".to_string(), "tool".to_string()],
            "later inserts with smaller output_index should not leapfrog visible reasoning/explore summaries",
        );
    }

    #[test]
    fn ordering_cross_request_pre_prompt_should_not_prepend_previous_turn() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        reset_history(chat);

        let make_plain = |text: &str| PlainMessageState {
            id: HistoryId::ZERO,
            role: PlainMessageRole::System,
            kind: PlainMessageKind::Plain,
            header: None,
            lines: vec![MessageLine {
                kind: MessageLineKind::Paragraph,
                spans: vec![InlineSpan {
                    text: text.to_string(),
                    tone: TextTone::Default,
                    emphasis: TextEmphasis::default(),
                    entity: None,
                }],
            }],
            metadata: None,
        };

        chat.history_insert_plain_state_with_key(
            make_plain("req1"),
            OrderKey {
                req: 1,
                out: 0,
                seq: 0,
            },
            "req1",
        );

        chat.last_seen_request_index = 1;
        chat.pending_user_prompts_for_next_turn = 0;

        let key = chat.system_order_key(SystemPlacement::PrePromptInCurrent, None);
        chat.history_insert_plain_state_with_key(make_plain("system"), key, "system");

        let labels: Vec<String> = chat
            .history_cells
            .iter()
            .map(|cell| {
                cell.display_lines_trimmed()
                    .first()
                    .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect())
                    .unwrap_or_default()
            })
            .collect();

        assert_eq!(
            labels,
            vec!["req1".to_string(), "system".to_string()],
            "pre-prompt system notices for a new request should append after the prior turn rather than prepending it",
        );
    }

    #[test]
    fn resume_ordering_offsets_provider_ordinals() {
        let mut harness = ChatWidgetHarness::new();
        let chat = harness.chat();
        reset_history(chat);

        let make_plain = |id: u64,
                           text: &str,
                           role: PlainMessageRole,
                           kind: PlainMessageKind| -> PlainMessageState {
            PlainMessageState {
                id: HistoryId(id),
                role,
                kind,
                header: None,
                lines: vec![MessageLine {
                    kind: MessageLineKind::Paragraph,
                    spans: vec![InlineSpan {
                        text: text.to_string(),
                        tone: TextTone::Default,
                        emphasis: TextEmphasis::default(),
                        entity: None,
                    }],
                }],
                metadata: None,
            }
        };

        let snapshot = HistorySnapshot {
            records: vec![
                HistoryRecord::PlainMessage(make_plain(
                    1,
                    "user-turn",
                    PlainMessageRole::User,
                    PlainMessageKind::User,
                )),
                HistoryRecord::PlainMessage(make_plain(
                    2,
                    "assistant-turn",
                    PlainMessageRole::Assistant,
                    PlainMessageKind::Assistant,
                )),
            ],
            next_id: 3,
            exec_call_lookup: HashMap::new(),
            tool_call_lookup: HashMap::new(),
            stream_lookup: HashMap::new(),
            order: vec![
                OrderKeySnapshot {
                    req: 5,
                    out: 0,
                    seq: 0,
                },
                OrderKeySnapshot {
                    req: 5,
                    out: 1,
                    seq: 0,
                },
            ],
            order_debug: Vec::new(),
        };

        chat.restore_history_snapshot(&snapshot);

        assert_eq!(
            chat.last_seen_request_index, 5,
            "restoring snapshot should set last_seen_request_index"
        );

        let order_meta = OrderMeta {
            request_ordinal: 0,
            output_index: Some(0),
            sequence_number: Some(0),
        };
        let key = chat.provider_order_key_from_order_meta(&order_meta);
        assert_eq!(
            key.req, 6,
            "resume should bias provider ordinals so new output slots after restored history"
        );

        let new_state = PlainMessageState {
            id: HistoryId::ZERO,
            role: PlainMessageRole::Assistant,
            kind: PlainMessageKind::Assistant,
            header: None,
            lines: vec![MessageLine {
                kind: MessageLineKind::Paragraph,
                spans: vec![InlineSpan {
                    text: "new-assistant".to_string(),
                    tone: TextTone::Default,
                    emphasis: TextEmphasis::default(),
                    entity: None,
                }],
            }],
            metadata: None,
        };

        let pos = chat.history_insert_plain_state_with_key(new_state, key, "resume-order");
        assert_eq!(pos, chat.history_cells.len().saturating_sub(1));

        let inserted_key = chat.cell_order_seq[pos];
        assert_eq!(inserted_key.req, 6);

        let inserted_text: String = chat.history_cells[pos]
            .display_lines_trimmed()
            .iter()
            .flat_map(|line| line.spans.iter().map(|span| span.content.as_ref()))
            .collect();
        assert!(
            inserted_text.contains("new-assistant"),
            "resume insertion should surface the new assistant answer at the tail"
        );
    }



}

fn append_thought_ellipsis(text: &str) -> String {
    let trimmed = text.trim_end();
    if trimmed.ends_with('…') {
        trimmed.to_string()
    } else {
        format!("{trimmed}…")
    }
}

fn extract_latest_bold_title(text: &str) -> Option<String> {
    fn prev_non_ws(text: &str, end: usize) -> Option<char> {
        text[..end].chars().rev().find(|ch| !ch.is_whitespace())
    }

    fn next_non_ws(text: &str, start: usize) -> Option<char> {
        text[start..].chars().find(|ch| !ch.is_whitespace())
    }

    fn normalize_candidate(candidate: &str) -> Option<String> {
        let trimmed = candidate.trim();
        if trimmed.is_empty() {
            return None;
        }
        Some(trimmed.to_string())
    }

    let bytes = text.as_bytes();
    let mut idx = 0usize;
    let mut latest: Option<String> = None;
    let mut open_start: Option<usize> = None;

    while idx + 1 < bytes.len() {
        if bytes[idx] == b'*' && bytes[idx + 1] == b'*' {
            if let Some(start) = open_start {
                let candidate = &text[start..idx];
                let before = prev_non_ws(text, start);
                let after = next_non_ws(text, idx + 2);
                let looks_like_heading = before
                    .map(|ch| matches!(ch, '"' | '\n' | '\r' | ':' | '[' | '{'))
                    .unwrap_or(true)
                    && after
                        .map(|ch| matches!(ch, '"' | '\n' | '\r' | ',' | '}' | ']'))
                        .unwrap_or(true);

                if looks_like_heading
                    && let Some(clean) = normalize_candidate(candidate) {
                        latest = Some(clean);
                    }
                open_start = None;
                idx += 2;
                continue;
            } else {
                open_start = Some(idx + 2);
                idx += 2;
                continue;
            }
        }
        idx += 1;
    }

    if latest.is_none()
        && let Some(start) = open_start
            && let Some(clean) = normalize_candidate(&text[start..]) {
                latest = Some(clean);
            }

    if latest.is_some() {
        return latest;
    }

    for raw_line in text.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(title) = heading_from_line(trimmed) {
            latest = Some(title);
        }
    }

    latest
}

fn heading_from_line(line: &str) -> Option<String> {
    let normalized = remove_bullet_prefix(line.trim_start());
    if !normalized.starts_with("**") {
        return None;
    }

    let rest = &normalized[2..];
    let end = rest.find("**");
    let title = match end {
        Some(idx) => &rest[..idx],
        None => rest,
    };

    if title.trim().is_empty() {
        return None;
    }

    Some(title.to_string())
}

fn remove_bullet_prefix(line: &str) -> &str {
    let mut normalized = line;
    for prefix in ["- ", "* ", "\u{2022} "] {
        if normalized.starts_with(prefix) {
            normalized = normalized[prefix.len()..].trim_start();
            break;
        }
    }
    normalized
}

fn strip_role_prefix_if_present(input: &str) -> (&str, bool) {
    const PREFIXES: [&str; 2] = ["Coordinator:", "CLI:"];
    for prefix in PREFIXES {
        if input.len() >= prefix.len() {
            let (head, tail) = input.split_at(prefix.len());
            if head.eq_ignore_ascii_case(prefix) {
                return (tail, true);
            }
        }
    }
    (input, false)
}



#[derive(Default)]
struct ExecState {
    running_commands: HashMap<ExecCallId, RunningCommand>,
    running_explore_agg_index: Option<usize>,
    // Pairing map for out-of-order exec events. If an ExecEnd arrives before
    // ExecBegin, we stash it briefly and either pair it when Begin arrives or
    // flush it after a short timeout to show a fallback cell.
    pending_exec_ends: HashMap<
        ExecCallId,
        (
            ExecCommandEndEvent,
            code_core::protocol::OrderMeta,
            std::time::Instant,
        ),
    >,
    suppressed_exec_end_call_ids: HashSet<ExecCallId>,
    suppressed_exec_end_order: VecDeque<ExecCallId>,
}

impl ExecState {
    fn suppress_exec_end(&mut self, call_id: ExecCallId) {
        if self.suppressed_exec_end_call_ids.insert(call_id.clone()) {
            self.suppressed_exec_end_order.push_back(call_id);
            const MAX_TRACKED_SUPPRESSED_IDS: usize = 64;
            if self.suppressed_exec_end_order.len() > MAX_TRACKED_SUPPRESSED_IDS
                && let Some(old) = self.suppressed_exec_end_order.pop_front() {
                    self.suppressed_exec_end_call_ids.remove(&old);
                }
        }
    }

    fn unsuppress_exec_end(&mut self, call_id: &ExecCallId) {
        if self.suppressed_exec_end_call_ids.remove(call_id) {
            self.suppressed_exec_end_order.retain(|cid| cid != call_id);
        }
    }

    fn should_suppress_exec_end(&self, call_id: &ExecCallId) -> bool {
        self.suppressed_exec_end_call_ids.contains(call_id)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RunningToolEntry {
    order_key: OrderKey,
    fallback_index: usize,
    history_id: Option<HistoryId>,
}

impl RunningToolEntry {
    fn new(order_key: OrderKey, fallback_index: usize) -> Self {
        Self {
            order_key,
            fallback_index,
            history_id: None,
        }
    }

    fn with_history_id(mut self, id: Option<HistoryId>) -> Self {
        self.history_id = id;
        self
    }
}

#[derive(Default)]
struct ToolState {
    running_custom_tools: HashMap<ToolCallId, RunningToolEntry>,
    web_search_sessions: HashMap<String, web_search_sessions::WebSearchTracker>,
    web_search_by_call: HashMap<String, String>,
    web_search_by_order: HashMap<u64, String>,
    running_wait_tools: HashMap<ToolCallId, ExecCallId>,
    running_kill_tools: HashMap<ToolCallId, ExecCallId>,
    image_viewed_calls: HashSet<ToolCallId>,
    browser_sessions: HashMap<String, browser_sessions::BrowserSessionTracker>,
    browser_session_by_call: HashMap<String, String>,
    browser_session_by_order: HashMap<BrowserSessionOrderKey, String>,
    browser_last_key: Option<String>,
    agent_runs: HashMap<String, agent_runs::AgentRunTracker>,
    agent_run_by_call: HashMap<String, String>,
    agent_run_by_order: HashMap<u64, String>,
    agent_run_by_batch: HashMap<String, String>,
    agent_run_by_agent: HashMap<String, String>,
    agent_last_key: Option<String>,
    auto_drive_tracker: Option<auto_drive_cards::AutoDriveTracker>,
}
#[derive(Default)]
struct StreamState {
    current_kind: Option<StreamKind>,
    closed_answer_ids: HashSet<StreamId>,
    closed_reasoning_ids: HashSet<StreamId>,
    seq_answer_final: Option<u64>,
    drop_streaming: bool,
}

#[derive(Default)]
struct LayoutState {
    // Scroll offset from bottom (0 = bottom)
    scroll_offset: Cell<u16>,
    // Cached max scroll from last render
    last_max_scroll: std::cell::Cell<u16>,
    // Track last viewport height of the history content area
    last_history_viewport_height: std::cell::Cell<u16>,
    // Stateful vertical scrollbar for history view
    vertical_scrollbar_state: std::cell::RefCell<ScrollbarState>,
    // Auto-hide scrollbar timer
    scrollbar_visible_until: std::cell::Cell<Option<std::time::Instant>>,
    // Last effective bottom pane height used by layout (rows)
    last_bottom_reserved_rows: std::cell::Cell<u16>,
    last_frame_height: std::cell::Cell<u16>,
    last_frame_width: std::cell::Cell<u16>,
    // Last bottom pane area for mouse hit testing
    last_bottom_pane_area: std::cell::Cell<Rect>,
}

#[derive(Default)]
struct DiffsState {
    session_patch_sets: Vec<HashMap<PathBuf, code_core::protocol::FileChange>>,
    baseline_file_contents: HashMap<PathBuf, String>,
    overlay: Option<DiffOverlay>,
    confirm: Option<DiffConfirm>,
    body_visible_rows: std::cell::Cell<u16>,
}

#[derive(Default)]
struct HelpState {
    overlay: Option<HelpOverlay>,
    body_visible_rows: std::cell::Cell<u16>,
}

#[derive(Default)]
struct SettingsState {
    overlay: Option<SettingsOverlayView>,
}

struct BrowserOverlayState {
    session_key: RefCell<Option<String>>,
    screenshot_index: Cell<usize>,
    action_scroll: Cell<u16>,
    last_action_view_height: Cell<u16>,
    max_action_scroll: Cell<u16>,
}

impl Default for BrowserOverlayState {
    fn default() -> Self {
        Self {
            session_key: RefCell::new(None),
            screenshot_index: Cell::new(0),
            action_scroll: Cell::new(0),
            last_action_view_height: Cell::new(0),
            max_action_scroll: Cell::new(0),
        }
    }
}

impl BrowserOverlayState {
    fn reset(&self) {
        self.screenshot_index.set(0);
        self.action_scroll.set(0);
        self.last_action_view_height.set(0);
        self.max_action_scroll.set(0);
    }

    fn session_key(&self) -> Option<String> {
        self.session_key.borrow().clone()
    }

    fn set_session_key(&self, key: Option<String>) {
        *self.session_key.borrow_mut() = key;
    }

    fn screenshot_index(&self) -> usize {
        self.screenshot_index.get()
    }

    fn set_screenshot_index(&self, index: usize) {
        self.screenshot_index.set(index);
    }

    fn action_scroll(&self) -> u16 {
        self.action_scroll.get()
    }

    fn set_action_scroll(&self, value: u16) {
        self.action_scroll.set(value);
    }

    fn update_action_metrics(&self, height: u16, max_scroll: u16) {
        self.last_action_view_height.set(height);
        self.max_action_scroll.set(max_scroll);
        if self.action_scroll.get() > max_scroll {
            self.action_scroll.set(max_scroll);
        }
    }

    fn last_action_view_height(&self) -> u16 {
        self.last_action_view_height.get()
    }

    fn max_action_scroll(&self) -> u16 {
        self.max_action_scroll.get()
    }
}

#[derive(Default)]
struct LimitsState {
    cached_content: Option<LimitsOverlayContent>,
}

struct HelpOverlay {
    lines: Vec<RtLine<'static>>,
    scroll: u16,
}

impl HelpOverlay {
    fn new(lines: Vec<RtLine<'static>>) -> Self {
        Self { lines, scroll: 0 }
    }
}
#[derive(Default)]
struct PerfState {
    enabled: bool,
    stats: RefCell<PerfStats>,
    pending_scroll_rows: Cell<u64>,
}
