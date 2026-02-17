use super::*;
use code_core::protocol::AgentStatusUpdateEvent;
use code_core::protocol::BrowserScreenshotUpdateEvent;
use code_core::protocol::ExitedReviewModeEvent;
use code_core::protocol::OrderMeta;
use code_protocol::protocol::ReviewRequest;

impl ChatWidget<'_> {
    pub(super) fn handle_background_event_event(
        &mut self,
        id: String,
        message: String,
        order: Option<&OrderMeta>,
    ) {
        info!("BackgroundEvent: {message}");
        if browser_sessions::handle_background_event(self, order, &message) {
            return;
        }
        let is_agent_hint = message.starts_with("🤖 Agent");
        if is_agent_hint && self.suppress_next_agent_hint {
            self.suppress_next_agent_hint = false;
            self.clear_resume_placeholder();
            return;
        }
        self.clear_resume_placeholder();
        // Route through unified system notice helper. If the core ties the
        // event to a turn (order present), prefer placing it before the next
        // provider output; else append to the tail. Use the event.id for
        // in-place replacement.
        let placement = match order.and_then(|om| om.output_index) {
            Some(v) if v == i32::MAX as u32 => SystemPlacement::Tail,
            Some(_) => SystemPlacement::Early,
            None => SystemPlacement::Tail,
        };
        let id_for_replace = Some(id);
        let message_clone = message.clone();
        let cell = history_cell::new_background_event(message_clone);
        let record = HistoryDomainRecord::BackgroundEvent(cell.state().clone());
        self.push_system_cell(
            Box::new(cell),
            placement,
            id_for_replace,
            order,
            "background",
            Some(record),
        );
        // If we inserted during streaming, keep the reasoning ellipsis visible.
        self.restore_reasoning_in_progress_if_streaming();

        // Also reflect CDP connect success in the status line.
        if message.starts_with("✅ Connected to Chrome via CDP") {
            self.bottom_pane
                .update_status_text("using browser (CDP)".to_string());
        }

        if is_agent_hint
            || message.starts_with("⚠️ Agent reuse")
            || message.starts_with("⚠️ Agent prompt")
        {
            self.recent_agent_hint = Some(message);
        }
    }

    pub(super) fn handle_agent_status_update_event(&mut self, event: AgentStatusUpdateEvent) {
        agent_runs::handle_status_update(self, &event);
        let AgentStatusUpdateEvent {
            agents,
            context,
            task,
        } = event;
        // Update the active agents list from the event and track timing
        self.active_agents.clear();
        let now = Instant::now();
        let mut saw_running = false;
        let mut has_running_non_auto_review = false;
        let mut has_running_auto_review = false;
        for agent in agents.iter() {
            let parsed_status = agent_status_from_str(agent.status.as_str());
            // Update runtime map
            let entry = self.agent_runtime.entry(agent.id.clone()).or_default();
            entry.last_update = Some(now);
            match parsed_status {
                AgentStatus::Running => {
                    if entry.started_at.is_none() {
                        entry.started_at = Some(now);
                    }
                    saw_running = true;
                }
                AgentStatus::Completed | AgentStatus::Failed | AgentStatus::Cancelled => {
                    if entry.completed_at.is_none() {
                        entry.completed_at = entry.completed_at.or(Some(now));
                    }
                }
                _ => {}
            }

            // Mirror agent list for rendering
            self.active_agents.push(AgentInfo {
                id: agent.id.clone(),
                name: agent.name.clone(),
                status: parsed_status.clone(),
                source_kind: agent.source_kind.clone(),
                batch_id: agent.batch_id.clone(),
                model: agent.model.clone(),
                result: agent.result.clone(),
                error: agent.error.clone(),
                last_progress: agent.last_progress.clone(),
            });

            let is_auto_review = Self::is_auto_review_agent(agent);

            if matches!(parsed_status, AgentStatus::Pending | AgentStatus::Running) {
                if is_auto_review {
                    has_running_auto_review = true;
                } else {
                    has_running_non_auto_review = true;
                }
            }
        }

        self.update_agents_terminal_state(&agents, context.clone(), task.clone());
        self.observe_auto_review_status(&agents);

        let agent_hint_label = if has_running_auto_review && !has_running_non_auto_review {
            AgentHintLabel::Review
        } else {
            AgentHintLabel::Agents
        };
        self.bottom_pane.set_agent_hint_label(agent_hint_label);

        // Store shared context and task
        self.agent_context = context;
        self.agent_task = task;

        // Fallback: if every agent we know about has reached a terminal state and
        // there is no active streaming or tooling, clear the spinner even if the
        // backend hasn't sent TaskComplete yet. This prevents the footer from
        // getting stuck on "Responding..." after multi-agent runs that yield early.
        if self.bottom_pane.is_task_running() {
            let all_agents_terminal = !self.agent_runtime.is_empty()
                && self
                    .agent_runtime
                    .values()
                    .all(|rt| rt.completed_at.is_some());
            if all_agents_terminal {
                let any_tools_running = !self.exec.running_commands.is_empty()
                    || !self.tools_state.running_custom_tools.is_empty()
                    || !self.tools_state.web_search_sessions.is_empty();
                let any_streaming = self.stream.is_write_cycle_active();
                if !(any_tools_running || any_streaming) {
                    self.bottom_pane.set_task_running(false);
                    self.bottom_pane.update_status_text(String::new());
                }
            }
        }

        if saw_running && has_running_non_auto_review && !self.bottom_pane.is_task_running() {
            self.bottom_pane.set_task_running(true);
            self.bottom_pane.update_status_text("Running...".to_string());
            self.refresh_auto_drive_visuals();
            self.request_redraw();
        }

        // Update overall task status based on agent states
        let status = Self::overall_task_status_for(&self.active_agents);
        self.overall_task_status = status.to_string();

        let agents_still_active = self
            .active_agents
            .iter()
            .any(|a| matches!(a.status, AgentStatus::Pending | AgentStatus::Running));
        if agents_still_active && has_running_non_auto_review {
            self.bottom_pane.set_task_running(true);
        } else if agents_still_active && !has_running_non_auto_review {
            // Auto Review-only runs should not drive the spinner.
            if !self.has_running_commands_or_tools()
                && !self.stream.is_write_cycle_active()
                && self.active_task_ids.is_empty()
            {
                self.bottom_pane.set_task_running(false);
                self.bottom_pane.update_status_text(String::new());
            }
        }

        // Reflect concise agent status in the input border
        if has_running_non_auto_review {
            let count = self.active_agents.len();
            let msg = match status {
                "preparing" => format!("agents: preparing ({count} ready)"),
                "running" => format!("agents: running ({count})"),
                "complete" => format!("agents: complete ({count} ok)"),
                "failed" => "agents: failed".to_string(),
                "cancelled" => "agents: cancelled".to_string(),
                _ => "agents: planning".to_string(),
            };
            self.bottom_pane.update_status_text(msg);
        } else if has_running_auto_review {
            // Let the dedicated Auto Review footer drive messaging; avoid
            // clobbering it with a generic agents status.
            self.bottom_pane.update_status_text(String::new());
        }

        // Keep agents visible after completion so users can see final messages/errors.
        // HUD will be reset automatically when a new agent batch starts.

        // Reset ready to start flag when we get actual agent updates
        if !self.active_agents.is_empty() {
            self.agents_ready_to_start = false;
        }
        // Re-evaluate spinner visibility now that agent states changed.
        self.maybe_hide_spinner();
        self.request_redraw();
    }

    pub(super) fn handle_browser_screenshot_update_event(
        &mut self,
        payload: BrowserScreenshotUpdateEvent,
        order: Option<&OrderMeta>,
    ) {
        #[cfg(feature = "code-fork")]
        handle_browser_screenshot(&payload, &self.app_event_tx);

        let BrowserScreenshotUpdateEvent {
            screenshot_path,
            url,
        } = payload;
        let update =
            browser_sessions::handle_screenshot_update(self, order, &screenshot_path, &url);
        tracing::info!(
            "Received browser screenshot update: {} at URL: {}",
            screenshot_path.display(),
            url
        );

        // Update the latest screenshot and URL for display
        if let Ok(mut latest) = self.latest_browser_screenshot.lock() {
            let old_url = latest.as_ref().map(|(_, u)| u.clone());
            *latest = Some((screenshot_path.clone(), url.clone()));
            if old_url.as_ref() != Some(&url) {
                tracing::info!("Browser URL changed from {:?} to {}", old_url, url);
            }
            tracing::debug!(
                "Updated browser screenshot display with path: {} and URL: {}",
                screenshot_path.display(),
                url
            );
        } else {
            tracing::warn!("Failed to acquire lock for browser screenshot update");
        }

        if let Some(key) = update.session_key.as_ref() {
            self.browser_overlay_state.set_session_key(Some(key.clone()));
            if let Some(tracker) = self.tools_state.browser_sessions.get(key) {
                let len = tracker.cell.screenshot_history().len();
                if len > 0 {
                    let last_index = len.saturating_sub(1);
                    let current_index = self.browser_overlay_state.screenshot_index();
                    if !self.browser_overlay_visible || current_index >= last_index {
                        self.browser_overlay_state.set_screenshot_index(last_index);
                    }
                }
            }
        }

        // Request a redraw to update the display immediately
        self.app_event_tx.send(AppEvent::RequestRedraw);

        if update.grouped {
            self.bottom_pane.update_status_text("using browser".to_string());
        }
    }

    pub(super) fn handle_turn_aborted_event(&mut self) {
        self.pending_request_user_input = None;
    }

    pub(super) fn handle_entered_review_mode_event(&mut self, review_request: ReviewRequest) {
        if self.auto_resolve_enabled() {
            self.auto_resolve_handle_review_enter();
        }
        let hint = review_request
            .user_facing_hint
            .as_deref()
            .unwrap_or("")
            .trim();
        let banner = if hint.is_empty() {
            ">> Code review started <<".to_string()
        } else {
            format!(">> Code review started: {hint} <<")
        };
        self.active_review_hint = review_request.user_facing_hint.clone();
        self.active_review_prompt = Some(review_request.prompt.clone());
        self.push_background_before_next_output(banner);

        let prompt_text = review_request.prompt.trim();
        if !prompt_text.is_empty() {
            let mut lines: Vec<Line<'static>> = Vec::new();
            lines.push(Line::from(vec![RtSpan::styled(
                "Review focus",
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            lines.push(Line::from(""));
            for line in prompt_text.lines() {
                lines.push(Line::from(line.to_string()));
            }
            let state = history_cell::plain_message_state_from_lines(
                lines,
                history_cell::HistoryCellType::Notice,
            );
            self.history_push_plain_state(state);
        }
        if self.auto_state.is_active() {
            self.auto_state.on_begin_review(false);
            self.auto_rebuild_live_ring();
        }
        self.request_redraw();
    }

    pub(super) fn handle_exited_review_mode_event(
        &mut self,
        review_event: ExitedReviewModeEvent,
    ) {
        if self.auto_resolve_enabled() {
            self.auto_resolve_handle_review_exit(review_event.review_output.clone());
        }
        self.review_guard = None;
        let hint = self.active_review_hint.take();
        let prompt = self.active_review_prompt.take();
        match review_event.review_output {
            Some(output) => {
                let summary_cell =
                    self.build_review_summary_cell(hint.as_deref(), prompt.as_deref(), &output);
                self.history_push(summary_cell);
                let finish_banner = match hint.as_deref() {
                    Some(h) if !h.trim().is_empty() => {
                        let trimmed = h.trim();
                        format!("<< Code review finished: {trimmed} >>")
                    }
                    _ => "<< Code review finished >>".to_string(),
                };
                self.push_background_tail(finish_banner);
            }
            None => {
                let banner = match hint.as_deref() {
                    Some(h) if !h.trim().is_empty() => {
                        let trimmed = h.trim();
                        format!("<< Code review finished without a final response ({trimmed}) >>")
                    }
                    _ => "<< Code review finished without a final response >>".to_string(),
                };
                self.push_background_tail(banner);
                self.history_push_plain_state(history_cell::new_warning_event(
                    "Review session ended without returning findings. Try `/review` again if you still need feedback.".to_string(),
                ));
            }
        }
        if self.auto_state.is_active() && self.auto_state.awaiting_review() {
            if self.auto_resolve_should_block_auto_resume() {
                self.request_redraw();
            } else {
                self.maybe_resume_auto_after_review();
            }
        } else {
            self.request_redraw();
        }
    }
}
