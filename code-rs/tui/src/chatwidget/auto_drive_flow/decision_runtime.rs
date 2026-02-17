use super::*;

impl ChatWidget<'_> {
    pub(crate) fn auto_handle_decision(&mut self, event: AutoDecisionEvent) {
        let AutoDecisionEvent {
            seq,
            status,
            status_title,
            status_sent_to_user,
            goal,
            cli,
            agents_timing,
            agents,
            transcript,
        } = event;
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
            let lines = vec!["AUTO DRIVE RESPONSE".to_string(), text];
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

    pub(crate) fn auto_session_tokens(&self) -> Option<u64> {
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

    pub(crate) fn schedule_auto_cli_prompt(&mut self, decision_seq: u64, prompt_text: String) {
        self.schedule_auto_cli_prompt_with_override(decision_seq, prompt_text, None);
    }

    pub(crate) fn schedule_auto_cli_prompt_with_override(
        &mut self,
        decision_seq: u64,
        prompt_text: String,
        countdown_override: Option<u8>,
    ) {
        self.auto_state.suppress_next_cli_display = false;
        let effects = self
            .auto_state
            .schedule_cli_prompt(decision_seq, prompt_text, None, None, countdown_override);
        self.auto_apply_controller_effects(effects);
    }

    pub(crate) fn auto_can_bootstrap_from_history(&self) -> bool {
        self.history_cells.iter().any(|cell| {
            matches!(
                cell.kind(),
                HistoryCellType::User
                    | HistoryCellType::Assistant
                    | HistoryCellType::Plain
                    | HistoryCellType::Exec { .. }
            )
        })
    }

    pub(crate) fn auto_apply_controller_effects(&mut self, effects: Vec<AutoControllerEffect>) {
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

    pub(crate) fn auto_spawn_countdown(&self, countdown_id: u64, decision_seq: u64, seconds: u8) {
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

        self.auto_launch_with_goal(AutoLaunchRequest {
            goal,
            derive_goal_from_history: false,
            review_enabled,
            subagents_enabled: agents_enabled,
            cross_check_enabled,
            qa_automation_enabled,
            continue_mode,
        });

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

}
