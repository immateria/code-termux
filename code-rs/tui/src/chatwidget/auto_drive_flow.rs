use super::*;

use code_core::config_types::AutoDriveModelRoutingEntry;

mod decision_runtime;
mod review_runtime;
mod presentation;

impl ChatWidget<'_> {
    pub(super) fn is_cli_running(&self) -> bool {
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

    pub(super) fn refresh_auto_drive_visuals(&mut self) {
        if self.auto_state.is_active()
            || self.auto_state.should_show_goal_entry()
            || self.auto_state.last_run_summary.is_some()
        {
            self.auto_rebuild_live_ring();
        }
    }

    pub(super) fn auto_reduced_motion_preference() -> bool {
        match std::env::var("CODE_TUI_REDUCED_MOTION") {
            Ok(value) => {
                let normalized = value.trim().to_ascii_lowercase();
                !matches!(normalized.as_str(), "" | "0" | "false" | "off" | "no")
            }
            Err(_) => false,
        }
    }

    pub(super) fn auto_reset_intro_timing(&mut self) {
        self.auto_state.reset_intro_timing();
    }

    pub(super) fn auto_ensure_intro_timing(&mut self) {
        let reduced_motion = Self::auto_reduced_motion_preference();
        self.auto_state.ensure_intro_timing(reduced_motion);
    }

    pub(super) fn auto_show_goal_entry_panel(&mut self) {
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

    pub(super) fn auto_exit_goal_entry_preserve_draft(&mut self) -> bool {
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

    pub(super) fn auto_launch_with_goal(&mut self, request: AutoLaunchRequest) {
        let AutoLaunchRequest {
            goal,
            derive_goal_from_history,
            review_enabled,
            subagents_enabled,
            cross_check_enabled,
            qa_automation_enabled,
            continue_mode,
        } = request;
        let conversation = self.rebuild_auto_history();
        let reduced_motion = Self::auto_reduced_motion_preference();
        self.auto_state.prepare_launch(
            goal.clone(),
            code_auto_drive_core::AutoLaunchSettings {
                review_enabled,
                subagents_enabled,
                cross_check_enabled,
                qa_automation_enabled,
                continue_mode,
                reduced_motion,
            },
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

        if !(full_auto_enabled || (trimmed.is_empty() && self.auto_state.is_active())) {
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
        self.auto_launch_with_goal(AutoLaunchRequest {
            goal: goal_text,
            derive_goal_from_history: false,
            review_enabled: defaults.review_enabled,
            subagents_enabled: defaults.agents_enabled,
            cross_check_enabled: defaults.cross_check_enabled,
            qa_automation_enabled: defaults.qa_automation_enabled,
            continue_mode: default_mode,
        });
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
        model_routing_enabled: bool,
        model_routing_entries: Vec<AutoDriveModelRoutingEntry>,
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
        if self.config.auto_drive.model_routing_enabled != model_routing_enabled
            || self.config.auto_drive.model_routing_entries != model_routing_entries
        {
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
        self.config.auto_drive.model_routing_enabled = model_routing_enabled;
        self.config.auto_drive.model_routing_entries = model_routing_entries;
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

    pub(super) fn auto_send_conversation(&mut self) {
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

    pub(super) fn auto_send_conversation_force(&mut self) {
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

    pub(super) fn auto_send_user_prompt_to_coordinator(
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

    pub(super) fn auto_failure_is_transient(message: &str) -> bool {
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

    pub(super) fn auto_schedule_restart_event(&self, token: u64, attempt: u32, delay: Duration) {
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

    pub(super) fn auto_pause_for_transient_failure(&mut self, message: String) {
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

}
