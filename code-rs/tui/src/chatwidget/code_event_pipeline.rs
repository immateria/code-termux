use super::*;
use code_core::protocol::WarningEvent;

mod custom_tool_events;
mod stream_events;
mod task_events;
mod approval_events;
mod exec_events;
mod runtime_events;

type WaitHistoryUpdate = (HistoryId, Option<Duration>, Vec<(String, bool)>);

impl ChatWidget<'_> {
    fn mcp_tool_from_protocol(tool: code_protocol::mcp::Tool) -> mcp_types::Tool {
        let code_protocol::mcp::Tool {
            name,
            title,
            description,
            input_schema,
            output_schema,
            annotations,
            ..
        } = tool;

        let annotations = annotations.and_then(|value| match serde_json::from_value(value) {
            Ok(annotations) => Some(annotations),
            Err(err) => {
                tracing::warn!("failed to parse annotations for MCP tool '{name}': {err}");
                None
            }
        });

        let input_schema = match serde_json::from_value::<mcp_types::ToolInputSchema>(input_schema)
        {
            Ok(schema) => schema,
            Err(err) => {
                tracing::warn!("failed to parse input schema for MCP tool '{name}': {err}");
                mcp_types::ToolInputSchema {
                    properties: None,
                    required: None,
                    r#type: "object".to_string(),
                }
            }
        };

        let output_schema = output_schema.and_then(|value| match serde_json::from_value(value) {
            Ok(schema) => Some(schema),
            Err(err) => {
                tracing::warn!("failed to parse output schema for MCP tool '{name}': {err}");
                None
            }
        });

        mcp_types::Tool {
            annotations,
            description,
            input_schema,
            name,
            output_schema,
            title,
        }
    }

    pub(crate) fn handle_code_event(&mut self, event: Event) {
        tracing::debug!(
            "handle_code_event({})",
            serde_json::to_string_pretty(&event).unwrap_or_default()
        );

        if self.session_id.is_none()
            && !self.test_mode
            && !matches!(&event.msg, EventMsg::SessionConfigured(_))
        {
            tracing::debug!(
                "Ignoring stale event {:?} (seq={}) while waiting for SessionConfigured",
                &event.msg,
                event.event_seq
            );
            return;
        }
        // Strict ordering: all LLM/tool events must carry OrderMeta; internal events use synthetic keys.
        // Track provider order to anchor internal inserts at the bottom of the active request.
        self.note_order(event.order.as_ref());

        let Event { id, msg, .. } = event.clone();
        match msg {
            EventMsg::EnvironmentContextFull(ev) => {
                self.handle_environment_context_full_event(&ev);
            }
            EventMsg::EnvironmentContextDelta(ev) => {
                self.handle_environment_context_delta_event(&ev);
            }
            EventMsg::BrowserSnapshot(ev) => {
                self.handle_browser_snapshot_event(&ev);
            }
            EventMsg::CompactionCheckpointWarning(event) => {
                self.history_push_plain_paragraphs(PlainMessageKind::Notice, [event.message]);
            }
            EventMsg::SessionConfigured(event) => {
                // Record session id for potential future fork/backtrack features
                self.session_id = Some(event.session_id);
                self.bottom_pane
                    .set_history_metadata(event.history_log_id, event.history_entry_count);
                // Record session information at the top of the conversation.
                // If we already showed the startup prelude (Popular commands),
                // avoid inserting a duplicate. Still surface a notice if the
                // model actually changed from the requested one.
                let is_first = !self.welcome_shown;
                let should_insert_session_info =
                    (!self.test_mode && is_first) || self.config.model != event.model;
                if should_insert_session_info {
                    if is_first {
                        self.welcome_shown = true;
                    }
                    let session_state = history_cell::new_session_info(
                        &self.config,
                        event.clone(),
                        is_first,
                        self.latest_upgrade_version.as_deref(),
                    );
                    let key = self.next_req_key_top();
                    let _ = self
                        .history_insert_plain_state_with_key(session_state, key, "prelude");
                }

                if let Some(user_message) = self.initial_user_message.take() {
                    // If the user provided an initial message, add it to the
                    // conversation history.
                    self.submit_user_message(user_message);
                }

                // Ask core for custom prompts so the slash menu can show them.
                self.submit_op(Op::ListCustomPrompts);
                self.submit_op(Op::ListSkills);
                self.mcp_tool_catalog_by_id.clear();
                self.mcp_tools_by_server.clear();
                self.mcp_disabled_tools_by_server.clear();
                self.mcp_server_failures.clear();
                if !self.config.mcp_servers.is_empty() {
                    self.submit_op(Op::ListMcpTools);
                }

                if self.resume_placeholder_visible && event.history_entry_count == 0 {
                    self.replace_resume_placeholder_with_notice(RESUME_NO_HISTORY_NOTICE);
                }

                self.request_redraw();
                self.flush_history_snapshot_if_needed(true);
            }
            EventMsg::WebSearchBegin(ev) => {
                self.ensure_spinner_for_activity("web-search-begin");
                // Enforce order presence (tool events should carry it)
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!("missing OrderMeta on WebSearchBegin; using synthetic key");
                        self.next_internal_key()
                    }
                };
                tracing::info!(
                    "[order] WebSearchBegin call_id={} seq={}",
                    ev.call_id,
                    event.event_seq
                );
                tools::web_search_begin(self, ev.call_id, ev.query, event.order.as_ref(), ok)
            }
            EventMsg::AgentMessage(AgentMessageEvent { message }) => {
                self.handle_agent_message_event(event.order.as_ref(), event.event_seq, id, message);
            }
            EventMsg::ReplayHistory(ev) => {
                self.clear_resume_placeholder();
                let code_core::protocol::ReplayHistoryEvent { items, history_snapshot } = ev;
                self.replay_history_depth = self.replay_history_depth.saturating_add(1);
                let max_req = self.last_seen_request_index;
                let mut processed_snapshot = false;
                if let Some(snapshot_value) = history_snapshot {
                    match serde_json::from_value::<HistorySnapshot>(snapshot_value) {
                        Ok(snapshot) => {
                            self.restore_history_snapshot(&snapshot);
                            self.flush_history_snapshot_if_needed(true);
                            processed_snapshot = true;
                        }
                        Err(err) => {
                            tracing::warn!("failed to deserialize replay snapshot: {err}");
                        }
                    }
                }
                if !processed_snapshot {
                    for item in &items {
                        self.render_replay_item(item.clone());
                    }
                    if !items.is_empty() {
                        self.last_seen_request_index =
                            self.last_seen_request_index.max(self.current_request_index);
                    }
                }
                if max_req > 0 {
                    self.last_seen_request_index = self.last_seen_request_index.max(max_req);
                    self.current_request_index = self.last_seen_request_index;
                }
                if processed_snapshot || !items.is_empty() {
                    self.reset_resume_order_anchor();
                }
                self.request_redraw();
                self.replay_history_depth = self.replay_history_depth.saturating_sub(1);
            }
            EventMsg::WebSearchComplete(ev) => {
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!("missing OrderMeta on WebSearchComplete; using synthetic key");
                        self.next_internal_key()
                    }
                };
                tools::web_search_complete(self, ev.call_id, ev.query, event.order.as_ref(), ok)
            }
            EventMsg::AgentMessageDelta(AgentMessageDeltaEvent { delta }) => {
                self.handle_agent_message_delta_event(event.order.as_ref(), id, delta);
            }
            EventMsg::AgentReasoning(AgentReasoningEvent { text }) => {
                self.handle_agent_reasoning_event(event.order.as_ref(), id, text);
            }
            EventMsg::AgentReasoningDelta(AgentReasoningDeltaEvent { delta }) => {
                self.handle_agent_reasoning_delta_event(event.order.as_ref(), id, delta);
            }
            EventMsg::AgentReasoningSectionBreak(AgentReasoningSectionBreakEvent {}) => {
                self.handle_agent_reasoning_section_break_event();
            }
            EventMsg::TaskStarted => {
                self.handle_task_started_event(id);
            }
            EventMsg::TaskComplete(TaskCompleteEvent { last_agent_message }) => {
                self.handle_task_complete_event(id, last_agent_message, event.order.clone());
            }
            EventMsg::AgentReasoningRawContentDelta(AgentReasoningRawContentDeltaEvent {
                delta,
            }) => {
                self.handle_agent_reasoning_raw_content_delta_event(event.order.as_ref(), id, delta);
            }
            EventMsg::AgentReasoningRawContent(AgentReasoningRawContentEvent { text }) => {
                self.handle_agent_reasoning_raw_content_event(event.order.as_ref(), id, text);
            }
            EventMsg::TokenCount(event) => {
                if let Some(info) = &event.info {
                    self.total_token_usage = info.total_token_usage.clone();
                    self.last_token_usage = info.last_token_usage.clone();
                }
                if let Some(snapshot) = event.rate_limits {
                    self.update_rate_limit_resets(&snapshot);
                    let warnings = self
                        .rate_limit_warnings
                        .take_warnings(snapshot.secondary_used_percent, snapshot.primary_used_percent);
                    let mut legend_entries: Vec<RateLimitLegendEntry> = Vec::new();
                    for warning in warnings {
                        if self.log_and_should_display_warning(&warning) {
                            let label = match warning.scope {
                                RateLimitWarningScope::Primary => {
                                    format!("Hourly usage ≥ {:.0}%", warning.threshold)
                                }
                                RateLimitWarningScope::Secondary => {
                                    format!("Weekly usage ≥ {:.0}%", warning.threshold)
                                }
                            };
                            legend_entries.push(RateLimitLegendEntry {
                                label,
                                description: warning.message.clone(),
                                tone: TextTone::Warning,
                            });
                        }
                    }
                    if !legend_entries.is_empty() {
                        let record = RateLimitsRecord {
                            id: HistoryId::ZERO,
                            snapshot: snapshot.clone(),
                            legend: legend_entries,
                        };
                        let cell = history_cell::RateLimitsCell::from_record(record.clone());
                        let key = self.next_internal_key();
                        let _ = self.history_insert_with_key_global_tagged(
                            Box::new(cell),
                            key,
                            "rate-limits",
                            Some(HistoryDomainRecord::RateLimits(record)),
                        );
                        self.request_redraw();
                    }

                    self.rate_limit_snapshot = Some(snapshot);
                    self.rate_limit_last_fetch_at = Some(Utc::now());
                    self.rate_limit_fetch_inflight = false;
                    self.refresh_settings_overview_rows();
                    let refresh_limits_settings = self
                        .settings
                        .overlay
                        .as_ref()
                        .map(|overlay| {
                            overlay.active_section() == SettingsSection::Limits
                                && !overlay.is_menu_active()
                        })
                        .unwrap_or(false);
                    if refresh_limits_settings {
                        self.show_limits_settings_ui();
                    }
                }
                self.bottom_pane.set_token_usage(
                    self.total_token_usage.clone(),
                    self.last_token_usage.clone(),
                    self.config.model_context_window,
                );
                self.update_stream_token_usage_metadata();
            }
            EventMsg::Error(ErrorEvent { message }) => {
                self.on_error(message);
            }
            EventMsg::Warning(WarningEvent { message }) => {
                self.history_push_plain_state(history_cell::new_warning_event(message));
                self.request_redraw();
            }
            EventMsg::PlanUpdate(update) => {
                let (plan_title, plan_active) = {
                    let title = update
                        .name
                        .as_ref()
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .map(std::string::ToString::to_string);
                    let total = update.plan.len();
                    let completed = update
                        .plan
                        .iter()
                        .filter(|p| matches!(p.status, StepStatus::Completed))
                        .count();
                    let active = total > 0 && completed < total;
                    (title, active)
                };
                // Insert plan updates at the time they occur. If the provider
                // supplied OrderMeta, honor it. Otherwise, derive a key within
                // the current (last-seen) request — do NOT advance to the next
                // request when a prompt is already queued, since these belong
                // to the in-flight turn.
                let key = self.near_time_key_current_req(event.order.as_ref());
                let _ = self.history_insert_with_key_global(
                    Box::new(history_cell::new_plan_update(update)),
                    key,
                );
                // If we inserted during streaming, keep the reasoning ellipsis visible.
                self.restore_reasoning_in_progress_if_streaming();
                let desired_title = if plan_active {
                    Some(plan_title.unwrap_or_else(|| "Plan".to_string()))
                } else {
                    None
                };
                self.apply_plan_terminal_title(desired_title);
            }
            EventMsg::ExecApprovalRequest(ev) => {
                self.handle_exec_approval_request_event(id, ev, event.event_seq);
            }
            EventMsg::RequestUserInput(ev) => {
                self.handle_request_user_input_event(event.order.as_ref(), ev);
            }
            EventMsg::DynamicToolCallRequest(ev) => {
                self.handle_dynamic_tool_call_request_event(event.order.as_ref(), ev);
            }
            EventMsg::ApplyPatchApprovalRequest(ev) => {
                self.handle_apply_patch_approval_request_event(id, ev, event.event_seq);
            }
            EventMsg::ExecCommandBegin(ev) => {
                self.handle_exec_command_begin_event(ev, event.order.clone(), event.event_seq);
            }
            EventMsg::ExecCommandOutputDelta(ev) => {
                self.handle_exec_command_output_delta_event(ev);
            }
            EventMsg::PatchApplyBegin(ev) => {
                self.handle_patch_apply_begin_event(ev, event.order.as_ref());
            }
            EventMsg::PatchApplyEnd(ev) => {
                self.handle_patch_apply_end_event(ev, event.event_seq);
            }
            EventMsg::ExecCommandEnd(ev) => {
                self.handle_exec_command_end_event(ev, event.order.clone(), event.event_seq);
            }
            EventMsg::McpToolCallBegin(ev) => {
                self.handle_mcp_tool_call_begin_event(ev, event.order.as_ref(), event.event_seq);
            }
            EventMsg::McpToolCallEnd(ev) => {
                self.handle_mcp_tool_call_end_event(ev, event.order.clone(), event.event_seq);
            }

            EventMsg::CustomToolCallBegin(CustomToolCallBeginEvent {
                call_id,
                tool_name,
                parameters,
            }) => {
                self.handle_custom_tool_call_begin_event(
                    event.order.as_ref(),
                    call_id,
                    tool_name,
                    parameters,
                );
            }
            EventMsg::CustomToolCallUpdate(CustomToolCallUpdateEvent {
                call_id,
                tool_name: _,
                parameters,
            }) => {
                self.apply_custom_tool_update(&call_id, parameters);
            }
            EventMsg::CustomToolCallEnd(end_event) => {
                self.handle_custom_tool_call_end_event(
                    event.order.as_ref(),
                    event.event_seq,
                    end_event,
                );
            }
            EventMsg::ViewImageToolCall(ViewImageToolCallEvent { call_id, path }) => {
                self.handle_view_image_tool_call_event(call_id, path, event.order.as_ref());
            }
            EventMsg::GetHistoryEntryResponse(event) => {
                let code_core::protocol::GetHistoryEntryResponseEvent {
                    offset,
                    log_id,
                    entry,
                } = event;

                // Inform bottom pane / composer.
                self.bottom_pane
                    .on_history_entry_response(log_id, offset, entry.map(|e| e.text));
            }
            EventMsg::ListCustomPromptsResponse(ev) => {
                let len = ev.custom_prompts.len();
                debug!("received {len} custom prompts");
                self.bottom_pane.set_custom_prompts(ev.custom_prompts);
            }
            EventMsg::McpListToolsResponse(ev) => {
                self.mcp_tool_catalog_by_id = ev
                    .tools
                    .into_iter()
                    .map(|(id, tool)| (id, Self::mcp_tool_from_protocol(tool)))
                    .collect();
                self.mcp_tools_by_server = ev.server_tools.unwrap_or_default();
                self.mcp_disabled_tools_by_server =
                    ev.server_disabled_tools.unwrap_or_default();
                self.mcp_server_failures = ev.server_failures.unwrap_or_default();
                if self.mcp_server_failures.is_empty() {
                    self.startup_mcp_error_summary = None;
                }
                self.refresh_mcp_settings_overlay();
            }
            EventMsg::ListSkillsResponse(ev) => {
                let len = ev.skills.len();
                debug!("received {len} skills");
                self.bottom_pane.set_skills(ev.skills);
                self.refresh_settings_overview_rows();
            }
            EventMsg::ShutdownComplete => {
                self.push_background_tail("🟡 ShutdownComplete".to_string());
                self.app_event_tx.send(AppEvent::ExitRequest);
            }
            EventMsg::TurnDiff(TurnDiffEvent { unified_diff }) => {
                info!("TurnDiffEvent: {unified_diff}");
                self.turn_had_code_edits = true;
            }
            EventMsg::BackgroundEvent(BackgroundEventEvent { message }) => {
                self.handle_background_event_event(id, message, event.order.as_ref());
            }
            EventMsg::AgentStatusUpdate(event) => {
                self.handle_agent_status_update_event(event);
            }
            EventMsg::BrowserScreenshotUpdate(payload) => {
                self.handle_browser_screenshot_update_event(payload, event.order.as_ref());
            }
            // Newer protocol variants we currently ignore in the TUI
            EventMsg::UserMessage(_) => {}
            EventMsg::TurnAborted(_) => {
                self.handle_turn_aborted_event();
            }
            EventMsg::ConversationPath(_) => {}
            EventMsg::EnteredReviewMode(review_request) => {
                self.handle_entered_review_mode_event(review_request);
            }
            EventMsg::ExitedReviewMode(review_event) => {
                self.handle_exited_review_mode_event(review_event);
            }
        }
    }

}
