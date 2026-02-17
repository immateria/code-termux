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
use code_protocol::dynamic_tools::DynamicToolResponse;
use code_protocol::num_format::format_with_separators;
use code_core::split_command_and_args;
use serde_json::Value as JsonValue;


mod diff_handlers;
mod agent_summary;
mod agent_editor_flow;
mod esc;
mod modals;
mod agent;
mod agent_install;
mod internals;
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
mod shell_config_flow;
mod session_tuning_flow;
mod streaming;
mod terminal_handlers;
mod terminal;
mod terminal_flow;
mod terminal_surface_image;
mod terminal_surface_header;
mod terminal_surface_render;
mod tools;
mod browser_sessions;
mod agent_runs;
mod web_search_sessions;
mod auto_drive_cards;
mod auto_drive_flow;
pub(crate) mod tool_cards;
mod running_tools;
#[cfg(test)]
mod tests;
#[cfg(any(test, feature = "test-helpers"))]
pub mod smoke_helpers;

#[cfg(test)]
pub(crate) use self::esc::EscIntent;
use self::agent_summary::agent_summary_counts;
use self::esc::AutoGoalEscState;
use self::agent_install::{
    AgentInstallSessionArgs,
    GuidedTerminalControl,
    UpgradeTerminalSessionArgs,
    start_agent_install_session,
    start_direct_terminal_session,
    start_prompt_terminal_session,
    start_upgrade_terminal_session,
    wrap_command,
};
use self::internals::preamble::*;
use self::internals::state::*;
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
use code_core::protocol::ApplyPatchApprovalRequestEvent;
use code_core::protocol::BackgroundEventEvent;
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
use code_core::protocol::ReviewRequest;
use code_core::protocol::PatchApplyBeginEvent;
use code_core::protocol::PatchApplyEndEvent;
use code_core::protocol::TaskCompleteEvent;
use code_core::protocol::TokenUsage;
use code_core::protocol::TurnDiffEvent;
use code_core::protocol::ViewImageToolCallEvent;
use code_core::review_coord::{bump_snapshot_epoch_for, try_acquire_lock, ReviewGuard};
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
    agent_editor_view::{AgentEditorInit, AgentEditorView},
    AutoDriveSettingsInit,
    AutoDriveSettingsView,
    PlanningSettingsView,
    UpdateSettingsInit,
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

        let spans: Vec<Span> = vec![
            Span::styled(prefix.to_string(), Style::default().fg(crate::colors::text())),
            Span::styled(
                slider,
                Style::default()
                    .fg(crate::colors::primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(suffix, Style::default().fg(crate::colors::text())),
        ];

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

include!("chatwidget/shared_defs.rs");
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
            SystemPlacement::Early => i32::MIN + 2,
            SystemPlacement::Tail => i32::MAX,
            SystemPlacement::PrePrompt => i32::MIN,
        };

        if order.is_none()
            && self.pending_user_prompts_for_next_turn > 0
            && matches!(placement, SystemPlacement::Early)
        {
            out = i32::MIN;
        }

        let mut key = OrderKey {
            req,
            out,
            seq: self.internal_seq,
        };

        if matches!(placement, SystemPlacement::Tail) {
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

    pub(super) fn is_startup_mcp_error(&self, message: &str) -> bool {
        if self.last_seen_request_index != 0 || self.pending_user_prompts_for_next_turn > 0 {
            return false;
        }

        let lower = message.to_ascii_lowercase();
        lower.contains("mcp server")
            && (lower.contains("failed to start") || lower.contains("failed to list tools"))
    }

    fn extract_mcp_server_name(message: &str) -> Option<&str> {
        for (marker, terminator) in [("MCP server `", '`'), ("MCP server '", '\'')] {
            let start = message.find(marker).map(|idx| idx + marker.len());
            if let Some(start) = start {
                let rest = &message[start..];
                if let Some(end) = rest.find(terminator) {
                    let name = &rest[..end];
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
        }
        None
    }

    pub(super) fn summarize_startup_mcp_error(message: &str) -> String {
        if let Some(name) = Self::extract_mcp_server_name(message) {
            return format!("❌ MCP server '{name}' failed to initialize. Run /mcp status for diagnostics.");
        }
        "❌ MCP server failed to initialize. Run /mcp status for diagnostics.".to_string()
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
        agent::spawn_new_conversation_runtime(
            config,
            self.app_event_tx.clone(),
            auth_manager,
            code_op_rx,
            ticket,
        );
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
            SystemPlacement::PrePrompt
        } else {
            SystemPlacement::Tail
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
                    explanation: None,
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
                    explanation: None,
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
                None,
                None,
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
        let completed_preview = history_cell::AgentStatusPreview {
            id: "demo-completed".to_string(),
            name: "Docs Scout".to_string(),
            status: "Completed".to_string(),
            model: Some("gpt-5.1-large".to_string()),
            details: vec![history_cell::AgentDetail::Result(
                "Summarized API changes".to_string(),
            )],
            status_kind: history_cell::AgentStatusKind::Completed,
            step_progress: Some(history_cell::StepProgress {
                completed: 3,
                total: 3,
            }),
            elapsed: Some(Duration::from_secs(32)),
            last_update: Some("Wrapped up summary".to_string()),
            ..history_cell::AgentStatusPreview::default()
        };
        let running_preview = history_cell::AgentStatusPreview {
            id: "demo-running".to_string(),
            name: "Lint Fixer".to_string(),
            status: "Running".to_string(),
            model: Some("code-gpt-5.2".to_string()),
            details: vec![history_cell::AgentDetail::Progress(
                "Refining suggested fixes".to_string(),
            )],
            status_kind: history_cell::AgentStatusKind::Running,
            step_progress: Some(history_cell::StepProgress {
                completed: 1,
                total: 3,
            }),
            elapsed: Some(Duration::from_secs(18)),
            last_update: Some("Step 2 of 3".to_string()),
            ..history_cell::AgentStatusPreview::default()
        };
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
        let pending_preview = history_cell::AgentStatusPreview {
            id: "demo-read-pending".to_string(),
            name: "Doc Harvester".to_string(),
            status: "Pending".to_string(),
            model: Some("gpt-4.5".to_string()),
            details: vec![history_cell::AgentDetail::Info(
                "Waiting for search index".to_string(),
            )],
            status_kind: history_cell::AgentStatusKind::Pending,
            ..history_cell::AgentStatusPreview::default()
        };
        let running_read = history_cell::AgentStatusPreview {
            id: "demo-read-running".to_string(),
            name: "Spec Parser".to_string(),
            status: "Running".to_string(),
            model: Some("code-gpt-3.5".to_string()),
            details: vec![history_cell::AgentDetail::Progress(
                "Scanning RFC summaries".to_string(),
            )],
            status_kind: history_cell::AgentStatusKind::Running,
            step_progress: Some(history_cell::StepProgress {
                completed: 2,
                total: 5,
            }),
            elapsed: Some(Duration::from_secs(22)),
            ..history_cell::AgentStatusPreview::default()
        };
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
            None,
            None,
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
        if matches!(account.map(|acc| acc.mode), Some(AuthMode::ApiKey)) {
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
            self.config.cli_auth_credentials_store_mode,
        );
        self.login_view_state = Some(LoginAccountsState::weak_handle(&state_rc));
        self.login_add_view_state = None;

        let showing_accounts_in_overlay = self.settings.overlay.as_ref().is_some_and(|overlay| {
            !overlay.is_menu_active() && overlay.active_section() == SettingsSection::Accounts
        });
        if showing_accounts_in_overlay
            && let Some(overlay) = self.settings.overlay.as_mut()
            && let Some(content) = overlay.accounts_content_mut() {
                content.show_manage_accounts(state_rc);
                self.request_redraw();
                return;
            }

        self.bottom_pane.show_login_accounts(view);
        self.request_redraw();
    }

    pub(crate) fn show_login_add_account_view(&mut self) {
        let ticket = self.make_background_tail_ticket();
        let (view, state_rc) = LoginAddAccountView::new(
            self.config.code_home.clone(),
            self.app_event_tx.clone(),
            ticket,
            self.config.cli_auth_credentials_store_mode,
        );
        self.login_add_view_state = Some(LoginAddAccountState::weak_handle(&state_rc));
        self.login_view_state = None;

        let showing_accounts_in_overlay = self.settings.overlay.as_ref().is_some_and(|overlay| {
            !overlay.is_menu_active() && overlay.active_section() == SettingsSection::Accounts
        });
        if showing_accounts_in_overlay
            && let Some(overlay) = self.settings.overlay.as_mut()
            && let Some(content) = overlay.accounts_content_mut() {
                content.show_add_account(state_rc);
                self.request_redraw();
                return;
            }

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

        let view = UpdateSettingsView::new(UpdateSettingsInit {
            app_event_tx: self.app_event_tx.clone(),
            ticket: self.make_background_tail_ticket(),
            current_version: code_version::version().to_string(),
            auto_enabled: self.config.auto_upgrade_enabled,
            command,
            command_display: display,
            manual_instructions: instructions,
            shared: shared_state.clone(),
        });

        if allow_refresh {
            self.spawn_update_refresh(shared_state);
        }
        Some(view)
    }

    fn build_updates_settings_content(&mut self) -> Option<UpdatesSettingsContent> {
        self.prepare_update_settings_view()
            .map(UpdatesSettingsContent::new)
    }

    fn build_accounts_settings_content(&self) -> AccountsSettingsContent {
        AccountsSettingsContent::new(
            self.app_event_tx.clone(),
            self.config.auto_switch_accounts_on_rate_limit,
            self.config.api_key_fallback_on_all_accounts_limited,
            self.config.cli_auth_credentials_store_mode,
        )
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
        let model_routing_enabled = self.config.auto_drive.model_routing_enabled;
        let model_routing_entries = self.config.auto_drive.model_routing_entries.clone();
        let routing_model_options = self
            .available_model_presets()
            .into_iter()
            .map(|preset| preset.model)
            .collect();
        let mode = self.auto_state.continue_mode;
        AutoDriveSettingsView::new(AutoDriveSettingsInit {
            app_event_tx: self.app_event_tx.clone(),
            model,
            model_reasoning: model_effort,
            use_chat_model,
            review_enabled: review,
            agents_enabled: agents,
            cross_check_enabled: cross,
            qa_automation_enabled: qa,
            model_routing_enabled,
            model_routing_entries,
            routing_model_options,
            continue_mode: mode,
        })
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
        start_agent_install_session(AgentInstallSessionArgs {
            app_event_tx: self.app_event_tx.clone(),
            terminal_id: id,
            agent_name: name.clone(),
            default_command,
            cwd: Some(cwd),
            control: GuidedTerminalControl {
                controller: controller.clone(),
                controller_rx,
            },
            selected_index,
            debug_enabled: self.config.debug,
        });
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

    pub(crate) fn flash_footer_notice(&mut self, text: String) {
        self.bottom_pane.flash_footer_notice(text);
        self.request_redraw();
    }

    pub(crate) fn refresh_accounts_settings_content(&mut self) {
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

    pub(crate) fn set_cli_auth_credentials_store_mode(
        &mut self,
        mode: code_core::config_types::AuthCredentialsStoreMode,
    ) {
        if self.config.cli_auth_credentials_store_mode == mode {
            self.refresh_accounts_settings_content();
            return;
        }
        self.config.cli_auth_credentials_store_mode = mode;

        let label = match mode {
            code_core::config_types::AuthCredentialsStoreMode::File => "file",
            code_core::config_types::AuthCredentialsStoreMode::Keyring => "keyring",
            code_core::config_types::AuthCredentialsStoreMode::Auto => "auto",
            code_core::config_types::AuthCredentialsStoreMode::Ephemeral => "ephemeral",
        };
        self.bottom_pane
            .flash_footer_notice(format!("Credential store: {label}"));

        self.refresh_accounts_settings_content();
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
            HistoryMutation::Inserted {
                record: HistoryRecord::AssistantStream(state),
                ..
            } => {
                self.refresh_streaming_cell_for_stream_id(stream_id, state);
                self.mark_history_dirty();
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
                            disabled_tools: Vec::new(),
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
            account_usage::RateLimitWarningEvent::new(
                warning.scope,
                warning.threshold,
                reset_at,
                Utc::now(),
                &warning.message,
            ),
        ) {
            Ok(result) => result,
            Err(err) => {
                tracing::warn!(?err, "failed to persist rate limit warning log");
                true
            }
        }
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
            .create_agent_with_options(code_core::AgentCreateRequest {
                model: review_model,
                name: Some("Auto Review".to_string()),
                prompt: review_prompt,
                context: None,
                output_goal: None,
                files: Vec::new(),
                read_only: false,
                batch_id: Some(branch.clone()),
                config: Some(agent_config.clone()),
                worktree_branch: Some(branch.clone()),
                worktree_base: Some(snapshot_id.clone()),
                source_kind: Some(code_core::protocol::AgentSourceKind::AutoReview),
                reasoning_effort: config.auto_review_model_reasoning_effort.into(),
            })
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
type AutoReviewStub = Option<Box<dyn FnMut() + Send>>;

#[cfg(test)]
type AutoReviewStubSlot = std::sync::Mutex<AutoReviewStub>;

#[cfg(test)]
static AUTO_REVIEW_STUB: once_cell::sync::Lazy<AutoReviewStubSlot> =
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
