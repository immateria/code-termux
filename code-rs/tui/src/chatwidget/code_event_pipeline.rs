use super::*;

impl ChatWidget<'_> {
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
                // Remove stale "Connecting MCP servers…" status from the startup notice
                // now that MCP initialization has completed in core.
                self.remove_connecting_mcp_notice();
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
                self.mcp_tools_by_server.clear();
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
                // If the user requested an interrupt, ignore late final answers.
                if self.stream_state.drop_streaming {
                    tracing::debug!("Ignoring AgentMessage after interrupt");
                    self.stop_spinner();
                    return;
                }

                // Allow a fresh lingering-exec sweep even if the per-turn guard
                // was tripped before any commands started.
                self.cleared_lingering_execs_this_turn = false;
                self.ensure_lingering_execs_cleared();

                self.stream_state.seq_answer_final = Some(event.event_seq);
                if !id.trim().is_empty() {
                    self.note_answer_stream_seen(&id);
                    // Any Answer item that completes before TaskComplete is considered
                    // mid‑turn until we later determine it was the final Answer.
                    if !self.active_task_ids.is_empty() {
                        self.mid_turn_answer_ids_in_turn.insert(id.clone());
                    }
                }
                // Strict order for the stream id
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!("missing OrderMeta on AgentMessage; using synthetic key");
                        self.next_internal_key()
                    }
                };
                self.seed_stream_order_key(StreamKind::Answer, &id, ok);

                tracing::debug!(
                    "AgentMessage final id={} bytes={} preview={:?}",
                    id,
                    message.len(),
                    message.chars().take(80).collect::<String>()
                );

                // Route final message through streaming controller so AppEvent::InsertFinalAnswer
                // is the single source of truth for assistant content.
                let sink = AppEventHistorySink(self.app_event_tx.clone());
                streaming::begin(self, StreamKind::Answer, Some(id.clone()));
                let _ = self.stream.apply_final_answer(&message, &sink);

                // Track last message for potential dedup heuristics.
                let cleaned = Self::strip_context_sections(&message);
                self.last_assistant_message = (!cleaned.trim().is_empty()).then_some(cleaned);
                // Mark this Answer stream id as closed for the rest of the turn so any late
                // AgentMessageDelta for the same id is ignored. In the full App runtime,
                // the InsertFinalAnswer path also marks closed; setting it here makes
                // unit tests (which do not route AppEvents back) behave identically.
                self.stream_state
                    .closed_answer_ids
                    .insert(StreamId(id.clone()));
                // Do not quiesce the global spinner here. `AgentMessage` is emitted for every
                // completed assistant output item, and modern models may send multiple assistant
                // messages mid-turn (progress updates, tool interleavings, etc.). We only clear
                // the turn spinner on `TaskComplete` or when all other activity drains.
                // Important: do not advance Auto Drive here. The StreamController will emit
                // AppEvent::InsertFinalAnswer, and the App thread will finalize the assistant
                // cell slightly later. Advancing at this point can start the next Auto Drive
                // step before the final answer is actually inserted, which appears as a
                // mid-turn re-trigger. We instead advance immediately after insertion inside
                // insert_final_answer_with_id().
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
                tracing::debug!("AgentMessageDelta: {:?}", delta);
                // If the user requested an interrupt, ignore late deltas.
                if self.stream_state.drop_streaming {
                    tracing::debug!("Ignoring Answer delta after interrupt");
                    self.stop_spinner();
                    return;
                }

                self.ensure_lingering_execs_cleared();

                if self.strict_stream_ids_enabled() && id.trim().is_empty() {
                    self.warn_missing_stream_id("assistant answer delta");
                    return;
                }
                // Ignore late deltas for ids that have already finalized in this turn
                if self
                    .stream_state
                    .closed_answer_ids
                    .contains(&StreamId(id.clone()))
                {
                    tracing::debug!("Ignoring Answer delta for closed id={}", id);
                    return;
                }
                self.note_answer_stream_seen(&id);
                // Seed/refresh order key for this Answer stream id (must have OrderMeta)
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!(
                            "missing OrderMeta on AgentMessageDelta; using synthetic key"
                        );
                        self.next_internal_key()
                    }
                };
                self.seed_stream_order_key(StreamKind::Answer, &id, ok);
                // Stream answer delta through StreamController
                streaming::delta_text(
                    self,
                    StreamKind::Answer,
                    id.clone(),
                    delta,
                    event.order.as_ref().and_then(|o| o.sequence_number),
                );
                self.ensure_spinner_for_activity("assistant-delta");
                // Show responding state while assistant streams
                self.bottom_pane
                    .update_status_text("responding".to_string());
            }
            EventMsg::AgentReasoning(AgentReasoningEvent { text }) => {
                // Ignore late reasoning if we've dropped streaming due to interrupt.
                if self.stream_state.drop_streaming {
                    tracing::debug!("Ignoring AgentReasoning after interrupt");
                    self.stop_spinner();
                    return;
                }
                tracing::debug!(
                    "AgentReasoning event with text: {:?}...",
                    text.chars().take(100).collect::<String>()
                );
                // Guard duplicates for this id within the task
                if self
                    .stream_state
                    .closed_reasoning_ids
                    .contains(&StreamId(id.clone()))
                {
                    tracing::warn!("Ignoring duplicate AgentReasoning for closed id={}", id);
                    return;
                }
                // Seed strict order key for this Reasoning stream
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!("missing OrderMeta on AgentReasoning; using synthetic key");
                        self.next_internal_key()
                    }
                };
                tracing::info!("[order] EventMsg::AgentReasoning id={} key={:?}", id, ok);
                self.seed_stream_order_key(StreamKind::Reasoning, &id, ok);
                // Use StreamController for final reasoning
                let sink = AppEventHistorySink(self.app_event_tx.clone());
                streaming::begin(self, StreamKind::Reasoning, Some(id.clone()));

                // The StreamController now properly handles duplicate detection and prevents
                // re-injecting content when we're already finishing a stream
                let _finished = self.stream.apply_final_reasoning(&text, &sink);
                // Stream finishing is handled by StreamController
                // Mark this id closed for further reasoning deltas in this turn
                self.stream_state
                    .closed_reasoning_ids
                    .insert(StreamId(id.clone()));
                // Clear in-progress flags on the most recent reasoning cell(s)
                if let Some(last) = self.history_cells.iter().rposition(|c| {
                    c.as_any()
                        .downcast_ref::<history_cell::CollapsibleReasoningCell>()
                        .is_some()
                })
                    && let Some(reason) = self.history_cells[last]
                        .as_any()
                        .downcast_ref::<history_cell::CollapsibleReasoningCell>()
                    {
                        reason.set_in_progress(false);
                    }
                self.mark_needs_redraw();
            }
            EventMsg::AgentReasoningDelta(AgentReasoningDeltaEvent { delta }) => {
                tracing::debug!("AgentReasoningDelta: {:?}", delta);
                if self.stream_state.drop_streaming {
                    tracing::debug!("Ignoring Reasoning delta after interrupt");
                    self.stop_spinner();
                    return;
                }
                if self.strict_stream_ids_enabled() && id.trim().is_empty() {
                    self.warn_missing_stream_id("assistant reasoning delta");
                    return;
                }
                // Ignore late deltas for ids that have already finalized in this turn
                if self
                    .stream_state
                    .closed_reasoning_ids
                    .contains(&StreamId(id.clone()))
                {
                    tracing::debug!("Ignoring Reasoning delta for closed id={}", id);
                    return;
                }
                // Seed strict order key for this Reasoning stream
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!(
                            "missing OrderMeta on AgentReasoningDelta; using synthetic key"
                        );
                        self.next_internal_key()
                    }
                };
                tracing::info!(
                    "[order] EventMsg::AgentReasoningDelta id={} key={:?}",
                    id,
                    ok
                );
                self.seed_stream_order_key(StreamKind::Reasoning, &id, ok);
                streaming::delta_text(
                    self,
                    StreamKind::Reasoning,
                    id.clone(),
                    delta,
                    event.order.as_ref().and_then(|o| o.sequence_number),
                );
                self.ensure_spinner_for_activity("reasoning-delta");
                // Show thinking state while reasoning streams
                self.bottom_pane.update_status_text("thinking".to_string());
            }
            EventMsg::AgentReasoningSectionBreak(AgentReasoningSectionBreakEvent {}) => {
                // Insert section break in reasoning stream
                let sink = AppEventHistorySink(self.app_event_tx.clone());
                self.stream.insert_reasoning_section_break(&sink);
            }
            EventMsg::TaskStarted => {
                // Defensive: if the previous turn never emitted TaskComplete (e.g. dropped event
                // due to reconnect), `active_task_ids` can stay non-empty. That makes every
                // subsequent Answer look like "mid-turn" forever and keeps the footer spinner
                // stuck.
                if let Some(last_id) = self.last_seen_answer_stream_id_in_turn.clone()
                    && (self.mid_turn_answer_ids_in_turn.contains(&last_id)
                        || !self.active_task_ids.is_empty())
                    {
                        self.mid_turn_answer_ids_in_turn.remove(&last_id);
                        self.maybe_clear_mid_turn_for_last_answer(&last_id);
                    }
                if !self.active_task_ids.is_empty() {
                    tracing::warn!(
                        "TaskStarted id={} while {} task(s) still active; assuming stale turn state",
                        id,
                        self.active_task_ids.len()
                    );
                    self.active_task_ids.clear();
                }
                // Reset per-turn cleanup guard and clear any lingering running
                // exec/tool cells if the prior turn never sent TaskComplete.
                // This runs once per turn and is intentionally later than
                // ToolEnd to avoid the earlier regression where we finalized
                // after every tool call.
                self.turn_sequence = self.turn_sequence.saturating_add(1);
                self.turn_had_code_edits = false;
                self.current_turn_origin = self.pending_turn_origin.take();
                self.cleared_lingering_execs_this_turn = false;
                self.ensure_lingering_execs_cleared();

                self.clear_reconnecting();
                // This begins the new turn; clear the pending prompt anchor count
                // so subsequent background events use standard placement.
                self.pending_user_prompts_for_next_turn = 0;
                self.pending_request_user_input = None;
                // Reset stream headers for new turn
                self.stream.reset_headers_for_new_turn();
                self.stream_state.current_kind = None;
                self.stream_state.seq_answer_final = None;
                self.last_answer_stream_id_in_turn = None;
                self.last_answer_history_id_in_turn = None;
                self.last_seen_answer_stream_id_in_turn = None;
                self.mid_turn_answer_ids_in_turn.clear();
                // New turn: clear closed id guards
                self.stream_state.closed_answer_ids.clear();
                self.stream_state.closed_reasoning_ids.clear();
                self.ended_call_ids.clear();
                self.bottom_pane.clear_ctrl_c_quit_hint();
                // Accept streaming again for this turn
                self.stream_state.drop_streaming = false;
                // Mark this task id as active and ensure the status stays visible
                self.active_task_ids.insert(id.clone());
                // Reset per-turn UI indicators; ordering is now global-only
                self.reasoning_index.clear();
                self.bottom_pane.set_task_running(true);
                self.bottom_pane
                    .update_status_text("waiting for model".to_string());
                self.ensure_spinner_for_activity("task-started");
                tracing::info!("[order] EventMsg::TaskStarted id={}", id);

                // Capture a baseline snapshot for this turn so background auto review only
                // covers changes made during the turn, not pre-existing local edits.
                self.auto_review_baseline = None;
                if self.config.tui.auto_review_enabled {
                    self.spawn_auto_review_baseline_capture();
                }

                // Don't add loading cell - we have progress in the input area
                // self.add_to_history(history_cell::new_loading_cell("waiting for model".to_string()));

                self.mark_needs_redraw();
            }
            EventMsg::TaskComplete(TaskCompleteEvent { last_agent_message }) => {
                self.clear_reconnecting();
                self.pending_request_user_input = None;
                let had_running_execs = !self.exec.running_commands.is_empty();
                // Finalize any active streams
                let finalizing_streams = self.stream.is_write_cycle_active();
                if finalizing_streams {
                    // Finalize both streams via streaming facade
                    streaming::finalize(self, StreamKind::Reasoning, true);
                    streaming::finalize(self, StreamKind::Answer, true);
                }
                // Remove this id from the active set (it may be a sub‑agent)
                self.active_task_ids.remove(&id);
                if !finalizing_streams && self.active_task_ids.is_empty()
                    && let Some(last_id) = self.last_seen_answer_stream_id_in_turn.clone() {
                        self.mid_turn_answer_ids_in_turn.remove(&last_id);
                        self.maybe_clear_mid_turn_for_last_answer(&last_id);
                    }
                if self.auto_resolve_enabled() {
                    self.auto_resolve_on_task_complete(last_agent_message.clone());
                }
                // Defensive: mark any lingering agent state as complete so the spinner can quiesce
                self.finalize_agent_activity();
                // Convert any lingering running exec/tool cells to completed so the UI doesn't hang
                self.finalize_all_running_due_to_answer();
                // Mark any running web searches as completed
                web_search_sessions::finalize_all_failed(
                    self,
                    "Search cancelled before completion",
                );
                if had_running_execs {
                    self.insert_background_event_with_placement(
                        "Running commands finalized after turn end.".to_string(),
                        BackgroundPlacement::Tail,
                        event.order.clone(),
                    );
                }
                // Now that streaming is complete, flush any queued interrupts
        self.flush_interrupt_queue();

        // Only drop the working status if nothing is actually running.
        let any_tools_running = !self.exec.running_commands.is_empty()
            || !self.tools_state.running_custom_tools.is_empty()
            || !self.tools_state.web_search_sessions.is_empty();
                let any_streaming = self.stream.is_write_cycle_active();
                let any_agents_active = self.agents_are_actively_running();
                let any_tasks_active = !self.active_task_ids.is_empty();

                if !(any_tools_running || any_streaming || any_agents_active || any_tasks_active) {
                    self.bottom_pane.set_task_running(false);
                    // Ensure any transient footer text like "responding" is cleared when truly idle
                    self.bottom_pane.update_status_text(String::new());
                }
                self.stream_state.current_kind = None;
                // Final re-check for idle state
                self.maybe_hide_spinner();
                self.maybe_trigger_auto_review();
                self.emit_turn_complete_notification(last_agent_message);
                self.suppress_next_agent_hint = false;
                self.mark_needs_redraw();
                self.flush_history_snapshot_if_needed(true);

            }
            EventMsg::AgentReasoningRawContentDelta(AgentReasoningRawContentDeltaEvent {
                delta,
            }) => {
                if self.stream_state.drop_streaming {
                    tracing::debug!("Ignoring RawContent delta after interrupt");
                    self.stop_spinner();
                    return;
                }
                if self.strict_stream_ids_enabled() && id.trim().is_empty() {
                    self.warn_missing_stream_id("assistant raw reasoning delta");
                    return;
                }
                // Treat raw reasoning content the same as summarized reasoning
                if self
                    .stream_state
                    .closed_reasoning_ids
                    .contains(&StreamId(id.clone()))
                {
                    tracing::debug!("Ignoring RawContent delta for closed id={}", id);
                    return;
                }
                // Seed strict order key for this reasoning stream id
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!(
                            "missing OrderMeta on Tools::PlanUpdate; using synthetic key"
                        );
                        self.next_internal_key()
                    }
                };
                self.seed_stream_order_key(StreamKind::Reasoning, &id, ok);

                streaming::delta_text(
                    self,
                    StreamKind::Reasoning,
                    id.clone(),
                    delta,
                    event.order.as_ref().and_then(|o| o.sequence_number),
                );
            }
            EventMsg::AgentReasoningRawContent(AgentReasoningRawContentEvent { text }) => {
                if self.stream_state.drop_streaming {
                    tracing::debug!("Ignoring AgentReasoningRawContent after interrupt");
                    self.stop_spinner();
                    return;
                }
                tracing::debug!(
                    "AgentReasoningRawContent event with text: {:?}...",
                    text.chars().take(100).collect::<String>()
                );
                if self
                    .stream_state
                    .closed_reasoning_ids
                    .contains(&StreamId(id.clone()))
                {
                    tracing::warn!(
                        "Ignoring duplicate AgentReasoningRawContent for closed id={}",
                        id
                    );
                    return;
                }
                // Seed strict order key now so upcoming insert uses the correct key.
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!(
                            "missing OrderMeta on Tools::ReasoningBegin; using synthetic key"
                        );
                        self.next_internal_key()
                    }
                };
                self.seed_stream_order_key(StreamKind::Reasoning, &id, ok);
                // Use StreamController for final raw reasoning
                let sink = AppEventHistorySink(self.app_event_tx.clone());
                streaming::begin(self, StreamKind::Reasoning, Some(id.clone()));
                let _finished = self.stream.apply_final_reasoning(&text, &sink);
                // Stream finishing is handled by StreamController
                self.stream_state
                    .closed_reasoning_ids
                    .insert(StreamId(id.clone()));
                if let Some(last) = self.history_cells.iter().rposition(|c| {
                    c.as_any()
                        .downcast_ref::<history_cell::CollapsibleReasoningCell>()
                        .is_some()
                })
                    && let Some(reason) = self.history_cells[last]
                        .as_any()
                        .downcast_ref::<history_cell::CollapsibleReasoningCell>()
                    {
                        reason.set_in_progress(false);
                    }
                self.mark_needs_redraw();
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
                let id2 = id.clone();
                let ev2 = ev.clone();
                let seq = event.event_seq;
                self.defer_or_handle(
                    move |interrupts| interrupts.push_exec_approval(seq, id, ev),
                    |this| {
                        this.finalize_active_stream();
                        this.flush_interrupt_queue();
                        this.handle_exec_approval_now(id2, ev2);
                        this.request_redraw();
                    },
                );
            }
            EventMsg::RequestUserInput(ev) => {
                let key = self.near_time_key_current_req(event.order.as_ref());
                let mut lines: Vec<String> = Vec::new();
                lines.push("Model requested user input".to_string());

                for question in &ev.questions {
                    let header = &question.header;
                    let id = &question.id;
                    let question_text = &question.question;
                    lines.push(format!("\n{header} ({id})\n{question_text}"));
                    if let Some(options) = &question.options {
                        lines.push("Options:".to_string());
                        for option in options {
                            let label = &option.label;
                            let description = &option.description;
                            lines.push(format!("- {label}: {description}"));
                        }
                    }
                }
                let auto_answer = self.auto_state.is_active() && !self.auto_state.is_paused_manual();
                if auto_answer {
                    lines.push("\nAuto Drive is active; continuing automatically.".to_string());
                } else {
                    lines.push(
                        "\nUse the picker below to continue (Esc to type in the composer).".to_string(),
                    );
                }

                let role = history_cell::plain_role_for_kind(PlainMessageKind::Notice);
                let state =
                    history_cell::plain_message_state_from_paragraphs(PlainMessageKind::Notice, role, lines);
                let _ = self.history_insert_plain_state_with_key(state, key, "request_user_input");
                self.restore_reasoning_in_progress_if_streaming();

                if auto_answer {
                    use code_protocol::request_user_input::RequestUserInputAnswer;
                    use code_protocol::request_user_input::RequestUserInputResponse;

                    fn choose_option_label(
                        question: &code_protocol::request_user_input::RequestUserInputQuestion,
                    ) -> Option<String> {
                        let options = question.options.as_ref()?;
                        if options.is_empty() {
                            return None;
                        }

                        let recommended = options.iter().position(|opt| {
                            opt.label.contains("(Recommended)")
                                || opt.label.contains("Recommended")
                                || opt.label.contains("recommended")
                        });
                        let idx = recommended.unwrap_or(0);
                        options.get(idx).map(|opt| opt.label.clone())
                    }

                    fn choose_freeform_value(
                        question: &code_protocol::request_user_input::RequestUserInputQuestion,
                    ) -> String {
                        let key = format!("{} {}", question.id, question.header).to_ascii_lowercase();
                        if key.contains("confirm") || key.contains("proceed") {
                            "yes".to_string()
                        } else if key.contains("name") {
                            "Auto Drive".to_string()
                        } else {
                            "auto".to_string()
                        }
                    }

                    let mut answers = std::collections::HashMap::new();
                    for question in &ev.questions {
                        let answer_value = if let Some(label) = choose_option_label(question) {
                            vec![label]
                        } else {
                            vec![choose_freeform_value(question)]
                        };
                        answers.insert(
                            question.id.clone(),
                            RequestUserInputAnswer {
                                answers: answer_value,
                            },
                        );
                    }
                    let response = RequestUserInputResponse { answers };

                    let summary = {
                        let mut parts = Vec::new();
                        for question in &ev.questions {
                            let label = response
                                .answers
                                .get(&question.id)
                                .and_then(|a| a.answers.first())
                                .map(String::as_str)
                                .unwrap_or("(skipped)");
                            if ev.questions.len() == 1 {
                                parts.push(label.to_string());
                            } else {
                                let header = question.header.trim();
                                if header.is_empty() {
                                    parts.push(label.to_string());
                                } else {
                                    parts.push(format!("{header}: {label}"));
                                }
                            }
                        }
                        parts.join("\n")
                    };

                    if !summary.trim().is_empty() {
                        let key = Self::order_key_successor(key);
                        let role = history_cell::plain_role_for_kind(PlainMessageKind::Notice);
                        let state = history_cell::plain_message_state_from_paragraphs(
                            PlainMessageKind::Notice,
                            role,
                            vec![format!("Auto Drive answered user input:\n{summary}")],
                        );
                        let _ = self
                            .history_insert_plain_state_with_key(state, key, "request_user_input_auto_answer");
                        self.restore_reasoning_in_progress_if_streaming();
                    }

                    if let Err(e) = self.code_op_tx.send(Op::UserInputAnswer {
                        id: ev.turn_id,
                        response,
                    }) {
                        tracing::error!("failed to send Op::UserInputAnswer: {e}");
                    }

                    self.bottom_pane
                        .update_status_text("waiting for model".to_string());
                    self.bottom_pane.set_task_running(true);
                } else {
                    self.pending_request_user_input = Some(PendingRequestUserInput {
                        turn_id: ev.turn_id.clone(),
                        call_id: ev.call_id.clone(),
                        anchor_key: key,
                        questions: ev.questions.clone(),
                    });
                    self.bottom_pane
                        .update_status_text("waiting for user input".to_string());
                    self.bottom_pane.set_task_running(true);
                    self.bottom_pane.ensure_input_focus();
                    self.bottom_pane
                        .show_request_user_input(crate::bottom_pane::RequestUserInputView::new(
                            ev.turn_id.clone(),
                            ev.questions,
                            self.app_event_tx.clone(),
                        ));
                }
                self.request_redraw();
            }
            EventMsg::DynamicToolCallRequest(ev) => {
                let key = self.near_time_key_current_req(event.order.as_ref());
                let tool = &ev.tool;
                let call_id = &ev.call_id;
                let lines = vec![
                    format!("Dynamic tool call requested: {tool}"),
                    format!("call_id: {call_id}"),
                    "Dynamic tools are not supported in this UI; returning a failure response."
                        .to_string(),
                ];
                let role = history_cell::plain_role_for_kind(PlainMessageKind::Notice);
                let state = history_cell::plain_message_state_from_paragraphs(
                    PlainMessageKind::Notice,
                    role,
                    lines,
                );
                let _ = self.history_insert_plain_state_with_key(state, key, "dynamic_tool_call");
                self.restore_reasoning_in_progress_if_streaming();

                let response = DynamicToolResponse {
                    content_items: vec![
                        code_protocol::dynamic_tools::DynamicToolCallOutputContentItem::InputText {
                            text: "dynamic tools are not supported in this UI".to_string(),
                        },
                    ],
                    success: false,
                };
                if let Err(e) = self.code_op_tx.send(Op::DynamicToolResponse {
                    id: ev.call_id.clone(),
                    response,
                }) {
                    tracing::error!("failed to send Op::DynamicToolResponse: {e}");
                }

                self.bottom_pane
                    .update_status_text("waiting for model".to_string());
                self.bottom_pane.set_task_running(true);
                self.request_redraw();
            }
            EventMsg::ApplyPatchApprovalRequest(ev) => {
                let id2 = id.clone();
                let ev2 = ev.clone();
                self.defer_or_handle(
                    move |interrupts| interrupts.push_apply_patch_approval(event.event_seq, id, ev),
                    |this| {
                        this.finalize_active_stream();
                        this.flush_interrupt_queue();
                        // Push approval UI state to bottom pane and surface the patch summary there.
                        // (Avoid inserting a duplicate summary here; handle_apply_patch_approval_now
                        // is responsible for rendering the proposed patch once.)
                        this.handle_apply_patch_approval_now(id2, ev2);
                        this.request_redraw();
                    },
                );
            }
            EventMsg::ExecCommandBegin(ev) => {
                let seq = event.event_seq;
                let om_begin = event.order.clone().unwrap_or_else(|| {
                    tracing::warn!("missing OrderMeta for ExecCommandBegin; using synthetic order");
                    code_core::protocol::OrderMeta {
                        request_ordinal: self.last_seen_request_index,
                        output_index: Some(i32::MAX as u32),
                        sequence_number: Some(seq),
                    }
                });
                self.handle_exec_begin_ordered(ev, om_begin, seq);
            }
            EventMsg::ExecCommandOutputDelta(ev) => {
                let call_id = ExecCallId(ev.call_id.clone());
                if self.exec.running_commands.contains_key(&call_id) {
                    self.ensure_spinner_for_activity("exec-output");
                }
                if let Some(running) = self.exec.running_commands.get_mut(&call_id) {
                    let chunk = String::from_utf8_lossy(&ev.chunk).to_string();
                    let chunk_len = chunk.len();
                    let (stdout_chunk, stderr_chunk) = match ev.stream {
                        ExecOutputStream::Stdout => {
                            let offset = running.stdout_offset;
                            running.stdout_offset = running.stdout_offset.saturating_add(chunk_len);
                            (
                                Some(crate::history::state::ExecStreamChunk {
                                    offset,
                                    content: chunk,
                                }),
                                None,
                            )
                        }
                        ExecOutputStream::Stderr => {
                            let offset = running.stderr_offset;
                            running.stderr_offset = running.stderr_offset.saturating_add(chunk_len);
                            (
                                None,
                                Some(crate::history::state::ExecStreamChunk {
                                    offset,
                                    content: chunk,
                                }),
                            )
                        }
                    };
                    let history_id = running.history_id.or_else(|| {
                        let mapped = self
                            .history_state
                            .history_id_for_exec_call(call_id.as_ref())
                            .or_else(|| {
                                running.history_index.and_then(|idx| {
                                    self.history_cell_ids
                                        .get(idx)
                                        .and_then(|slot| *slot)
                                })
                            });
                        running.history_id = mapped;
                        mapped
                    });
                    if let Some(history_id) = history_id
                        && let Some(record_idx) = self.history_state.index_of(history_id) {
                            let mutation = self.history_state.apply_domain_event(
                                HistoryDomainEvent::UpdateExecStream {
                                    index: record_idx,
                                    stdout_chunk,
                                    stderr_chunk,
                                },
                            );
                            if let HistoryMutation::Replaced {
                                id,
                                record: HistoryRecord::Exec(exec_record),
                                ..
                            } = mutation
                            {
                                self.update_cell_from_record(id, HistoryRecord::Exec(exec_record));
                            }
                        }
                    self.invalidate_height_cache();
                    self.autoscroll_if_near_bottom();
                    self.request_redraw();
                }
            }
            EventMsg::PatchApplyBegin(PatchApplyBeginEvent {
                call_id,
                auto_approved,
                changes,
            }) => {
                let exec_call_id = ExecCallId(call_id);
                self.exec.suppress_exec_end(exec_call_id);
                // Store for session diff popup (clone before moving into history)
                self.diffs.session_patch_sets.push(changes.clone());
                // Capture/adjust baselines, including rename moves
                if let Some(last) = self.diffs.session_patch_sets.last() {
                    for (src_path, chg) in last.iter() {
                        match chg {
                            code_core::protocol::FileChange::Update {
                                move_path: Some(dest_path),
                                ..
                            } => {
                                // Prefer to carry forward existing baseline from src to dest.
                                if let Some(baseline) =
                                    self.diffs.baseline_file_contents.remove(src_path)
                                {
                                    self.diffs
                                        .baseline_file_contents
                                        .insert(dest_path.clone(), baseline);
                                } else if !self.diffs.baseline_file_contents.contains_key(dest_path)
                                {
                                    // Fallback: snapshot current contents of src (pre-apply) under dest key.
                                    let baseline =
                                        std::fs::read_to_string(src_path).unwrap_or_default();
                                    self.diffs
                                        .baseline_file_contents
                                        .insert(dest_path.clone(), baseline);
                                }
                            }
                            _ => {
                                if !self.diffs.baseline_file_contents.contains_key(src_path) {
                                    let baseline =
                                        std::fs::read_to_string(src_path).unwrap_or_default();
                                    self.diffs
                                        .baseline_file_contents
                                        .insert(src_path.clone(), baseline);
                                }
                            }
                        }
                    }
                }
                // Enable Ctrl+D footer hint now that we have diffs to show
                self.bottom_pane.set_diffs_hint(true);
                // Strict order
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!("missing OrderMeta on ExecEnd flush; using synthetic key");
                        self.next_internal_key()
                    }
                };
                let cell = history_cell::new_patch_event(
                    PatchEventType::ApplyBegin { auto_approved },
                    changes,
                );
                let _ = self.history_insert_with_key_global(Box::new(cell), ok);
            }
            EventMsg::PatchApplyEnd(ev) => {
                let ev2 = ev.clone();
                self.defer_or_handle(
                    move |interrupts| interrupts.push_patch_end(event.event_seq, ev),
                    |this| this.handle_patch_apply_end_now(ev2),
                );
            }
            EventMsg::ExecCommandEnd(ev) => {
                let ev2 = ev.clone();
                let seq = event.event_seq;
                let order_meta_end = event.order.clone().unwrap_or_else(|| {
                    tracing::warn!("missing OrderMeta for ExecCommandEnd; using synthetic order");
                    code_core::protocol::OrderMeta {
                        request_ordinal: self.last_seen_request_index,
                        output_index: Some(i32::MAX as u32),
                        sequence_number: Some(seq),
                    }
                });
                let om_for_send = order_meta_end.clone();
                self.defer_or_handle(
                    move |interrupts| interrupts.push_exec_end(seq, ev, Some(om_for_send)),
                    move |this| {
                        tracing::info!(
                            "[order] ExecCommandEnd call_id={} seq={}",
                            ev2.call_id,
                            seq
                        );
                        this.enqueue_or_handle_exec_end(ev2, order_meta_end);
                    },
                );
            }
            EventMsg::McpToolCallBegin(ev) => {
                let seq = event.event_seq;
                let order_ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!("missing OrderMeta on McpBegin; using synthetic key");
                        self.next_internal_key()
                    }
                };
                self.finalize_active_stream();
                tracing::info!(
                    "[order] McpToolCallBegin call_id={} seq={}",
                    ev.call_id,
                    seq
                );
                self.ensure_spinner_for_activity("mcp-begin");
                tools::mcp_begin(self, ev, order_ok);
                if self.interrupts.has_queued() {
                    self.flush_interrupt_queue();
                }
            }
            EventMsg::McpToolCallEnd(ev) => {
                let ev2 = ev.clone();
                let seq = event.event_seq;
                let order_ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!("missing OrderMeta on McpEnd; using synthetic key");
                        self.next_internal_key()
                    }
                };
                self.defer_or_handle(
                    move |interrupts| interrupts.push_mcp_end(seq, ev, event.order),
                    |this| {
                        tracing::info!(
                            "[order] McpToolCallEnd call_id={} seq={}",
                            ev2.call_id,
                            seq
                        );
                        tools::mcp_end(this, ev2, order_ok)
                    },
                );
            }

            EventMsg::CustomToolCallBegin(CustomToolCallBeginEvent {
                call_id,
                tool_name,
                parameters,
            }) => {
                self.ensure_spinner_for_activity("tool-begin");
                // Any custom tool invocation should fade out the welcome animation
                for cell in &self.history_cells {
                    cell.trigger_fade();
                }
                self.finalize_active_stream();
                // Flush any queued interrupts when streaming ends
                self.flush_interrupt_queue();
                // Show an active entry immediately for all custom tools so the user sees progress
                let params_json = parameters;
                let params_string = params_json.clone().map(|p| p.to_string());
                if agent_runs::is_agent_tool(&tool_name)
                    && agent_runs::handle_custom_tool_begin(
                        self,
                        event.order.as_ref(),
                        &call_id,
                        &tool_name,
                        params_json.clone(),
                    ) {
                        self.bottom_pane
                            .update_status_text("agents coordinating".to_string());
                        return;
                    }
                if tool_name.starts_with("browser_")
                    && browser_sessions::handle_custom_tool_begin(
                        self,
                        event.order.as_ref(),
                        &call_id,
                        &tool_name,
                        params_json,
                    ) {
                        self.bottom_pane
                            .update_status_text("using browser".to_string());
                        return;
                    }

                if tool_name == "wait"

                    && let Some(exec_call_id) = wait_exec_call_id_from_params(params_string.as_ref()) {
                        // Only treat this as an exec-scoped wait when the target exec is still running.
                        // Background waits (e.g., waiting on a shell call_id) also carry `call_id`.
                        if self.exec.running_commands.contains_key(&exec_call_id) {
                            self.tools_state
                                .running_wait_tools
                                .insert(ToolCallId(call_id.clone()), exec_call_id.clone());


                            let mut wait_update: Option<(
                                HistoryId,
                                Option<Duration>,
                                Vec<(String, bool)>,
                            )> = None;
                            if let Some(running) = self.exec.running_commands.get_mut(&exec_call_id) {
                                running.wait_active = true;
                                running.wait_notes.clear();
                                let history_id = running.history_id.or_else(|| {
                                    running.history_index.and_then(|idx| {
                                        self.history_cell_ids
                                            .get(idx)
                                            .and_then(|slot| *slot)
                                    })
                                });
                                running.history_id = history_id;
                                if let Some(id) = history_id {
                                    wait_update = Some((id, running.wait_total, running.wait_notes.clone()));
                                }
                            }
                            if let Some((history_id, total, notes)) = wait_update {
                                let _ = self.update_exec_wait_state_with_pairs(history_id, total, true, &notes);
                            }
                            self.bottom_pane
                                .update_status_text("waiting for command".to_string());
                            self.invalidate_height_cache();
                            self.request_redraw();
                            return;
                        }
                    }

                if tool_name == "kill"

                    && let Some(exec_call_id) = wait_exec_call_id_from_params(params_string.as_ref())
                        && self.exec.running_commands.contains_key(&exec_call_id) {
                            self.tools_state
                                .running_kill_tools
                                .insert(ToolCallId(call_id.clone()), exec_call_id);
                            self.bottom_pane
                                .update_status_text("cancelling command".to_string());
                            self.invalidate_height_cache();
                            self.request_redraw();
                            return;
                        }
                // Animated running cell with live timer and formatted args
                let mut cell = if tool_name.starts_with("browser_") {
                    history_cell::new_running_browser_tool_call(
                        tool_name.clone(),
                        params_string,
                    )
                } else if tool_name.starts_with("agent_") {
                    history_cell::new_running_custom_tool_call(
                        tool_name.clone(),
                        params_string,
                    )
                } else {
                    history_cell::new_running_custom_tool_call(
                        tool_name.clone(),
                        params_string,
                    )
                };
                cell.state_mut().call_id = Some(call_id.clone());
                // Enforce ordering for custom tool begin
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!(
                            "missing OrderMeta on CustomToolCallBegin; using synthetic key"
                        );
                        self.next_internal_key()
                    }
                };
                let idx = self.history_insert_with_key_global(Box::new(cell), ok);
                let history_id = self
                    .history_state
                    .history_id_for_tool_call(&call_id)
                    .or_else(|| self.history_cell_ids.get(idx).and_then(|slot| *slot));
                // Track index so we can replace it on completion
                if idx < self.history_cells.len() {
                    self.tools_state
                        .running_custom_tools
                        .insert(
                            ToolCallId(call_id.clone()),
                            RunningToolEntry::new(ok, idx).with_history_id(history_id),
                        );
                }

                // Update border status based on tool
                if tool_name.starts_with("browser_") {
                    self.bottom_pane
                        .update_status_text("using browser".to_string());
                } else if agent_runs::is_agent_tool(&tool_name) {
                    self.bottom_pane
                        .update_status_text("agents coordinating".to_string());
                } else {
                    self.bottom_pane
                        .update_status_text(format!("using tool: {tool_name}"));
                }
            }
            EventMsg::CustomToolCallUpdate(CustomToolCallUpdateEvent {
                call_id,
                tool_name: _,
                parameters,
            }) => {
                self.apply_custom_tool_update(&call_id, parameters);
            }
            EventMsg::CustomToolCallEnd(CustomToolCallEndEvent {
                call_id,
                tool_name,
                parameters,
                duration,
                result,
            }) => {
                let params_json = parameters;
                if agent_runs::is_agent_tool(&tool_name)
                    && agent_runs::handle_custom_tool_end(
                        self,
                        event.order.as_ref(),
                        &call_id,
                        &tool_name,
                        params_json.clone(),
                        duration,
                        &result,
                    ) {
                        self.bottom_pane
                            .update_status_text("responding".to_string());
                        return;
                    }
                if tool_name.starts_with("browser_")
                    && browser_sessions::handle_custom_tool_end(
                        self,
                        event.order.as_ref(),
                        &call_id,
                        &tool_name,
                        params_json.clone(),
                        duration,
                        &result,
                    ) {
                        if tool_name == "browser_close" {
                            self.bottom_pane
                                .update_status_text("responding".to_string());
                        } else {
                            self.bottom_pane
                                .update_status_text("using browser".to_string());
                        }
                        return;
                    }
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!(
                            "missing OrderMeta on CustomToolCallEnd; using synthetic key"
                        );
                        self.next_internal_key()
                    }
                };
                let image_view_seen = if tool_name == "image_view" {
                    self.tools_state
                        .image_viewed_calls
                        .remove(&ToolCallId(call_id.clone()))
                } else {
                    false
                };
                let image_view_path = if tool_name == "image_view" && !image_view_seen {
                    params_json
                        .as_ref()
                        .and_then(|value| image_view_path_from_params(value, &self.config.cwd))
                } else {
                    None
                };
                tracing::info!(
                    "[order] CustomToolCallEnd call_id={} tool={} seq={}",
                    call_id,
                    tool_name,
                    event.event_seq
                );
                // Convert parameters to String if present
                let params_string = params_json.map(|p| p.to_string());
                // Determine success and content from Result
                let (success, content) = match result {
                    Ok(content) => (true, content),
                    Err(error) => (false, error),
                };
                if tool_name == "wait"
                    && let Some(exec_call_id) = self
                        .tools_state
                        .running_wait_tools
                        .remove(&ToolCallId(call_id.clone()))
                    {
                        let trimmed = content.trim();
                        let wait_missing_job = wait_result_missing_background_job(trimmed);
                        let wait_interrupted = wait_result_interrupted(trimmed);
                        let mut wait_still_pending =
                            !success && trimmed != "Cancelled by user." && !wait_missing_job;
                        let mut exec_running = false;
                        let mut exec_completed = false;
                        let mut note_lines: Vec<(String, bool)> = Vec::new();
                        let suppress_json_notes = serde_json::from_str::<serde_json::Value>(
                            trimmed,
                        )
                        .ok()
                        .and_then(|value| {
                            value.as_object().map(|obj| {
                                obj.contains_key("output") || obj.contains_key("metadata")
                            })
                        })
                        .unwrap_or(false);
                        if !suppress_json_notes {
                            for line in content.lines() {
                                let note_text = line.trim();
                                if note_text.is_empty() {
                                    continue;
                                }
                                let is_error_note = note_text == "Cancelled by user.";
                                note_lines.push((note_text.to_string(), is_error_note));
                            }
                        }
                        let mut history_id: Option<HistoryId> = None;
                        let mut wait_total: Option<Duration> = None;
                        let mut wait_notes_snapshot: Vec<(String, bool)> = Vec::new();
                        if let Some(running) = self.exec.running_commands.get_mut(&exec_call_id) {
                            exec_running = true;
                            let base = running.wait_total.unwrap_or_default();
                            let total = base.saturating_add(duration);
                            running.wait_total = Some(total);
                            running.wait_active = wait_still_pending;
                            Self::append_wait_pairs(&mut running.wait_notes, &note_lines);
                            wait_notes_snapshot = running.wait_notes.clone();
                            wait_total = running.wait_total;
                            history_id = running.history_id.or_else(|| {
                                running.history_index.and_then(|idx| {
                                    self.history_cell_ids
                                        .get(idx)
                                        .and_then(|slot| *slot)
                                })
                            });
                            running.history_id = history_id;
                        } else {
                            Self::append_wait_pairs(&mut wait_notes_snapshot, &note_lines);
                        }

                        if history_id.is_none()
                            && let Some((idx, _)) = self.history_cells.iter().enumerate().rev().find(|(_, cell)| {
                                cell.as_any()
                                    .downcast_ref::<history_cell::ExecCell>()
                                    .is_some()
                            })
                                && let Some(id) = self.history_cell_ids.get(idx).and_then(|slot| *slot) {
                                    history_id = Some(id);
                                    if let Some(running) =
                                        self.exec.running_commands.get_mut(&exec_call_id)
                                    {
                                        running.history_index = Some(idx);
                                        running.history_id = Some(id);
                                    }
                                }

                        if let Some(id) = history_id {
                            let exec_record = self
                                .history_state
                                .index_of(id)
                                .and_then(|idx| self.history_state.get(idx).cloned());
                            if let Some(HistoryRecord::Exec(record)) = exec_record {
                                exec_completed = !matches!(record.status, ExecStatus::Running);
                                if wait_total.is_none() {
                                    let base = record.wait_total.unwrap_or_default();
                                    wait_total = Some(base.saturating_add(duration));
                                }
                                if wait_notes_snapshot.is_empty() {
                                    wait_notes_snapshot =
                                        Self::wait_pairs_from_exec_notes(&record.wait_notes);
                                    Self::append_wait_pairs(&mut wait_notes_snapshot, &note_lines);
                                }
                            } else {
                                if wait_total.is_none() {
                                    wait_total = Some(duration);
                                }
                                if wait_notes_snapshot.is_empty() {
                                    Self::append_wait_pairs(&mut wait_notes_snapshot, &note_lines);
                                }
                            }
                            if exec_completed {
                                wait_still_pending = false;
                            } else if wait_interrupted && !exec_running {
                                wait_still_pending = false;
                            }

                            if !exec_completed {
                                let _ = self.update_exec_wait_state_with_pairs(
                                    id,
                                    wait_total,
                                    wait_still_pending,
                                    &wait_notes_snapshot,
                                );
                            }
                        }

                        if exec_completed {
                            self.bottom_pane
                                .update_status_text("responding".to_string());
                            self.maybe_hide_spinner();
                            self.invalidate_height_cache();
                            self.request_redraw();
                            return;
                        }

                        if success {
                            self.remove_background_completion_message(&call_id);
                            self.bottom_pane
                                .update_status_text("responding".to_string());
                            self.maybe_hide_spinner();
                        } else if trimmed == "Cancelled by user." {
                            self.bottom_pane
                                .update_status_text("wait cancelled".to_string());
                        } else if wait_missing_job || (wait_interrupted && !exec_running) {
                            let finalized = exec_tools::finalize_wait_missing_exec(
                                self,
                                exec_call_id.clone(),
                                trimmed,
                            );
                            if finalized {
                                self.bottom_pane.update_status_text(
                                    "command finished (output unavailable)".to_string(),
                                );
                            } else {
                                self.bottom_pane.update_status_text(
                                    "command status unavailable".to_string(),
                                );
                            }
                        } else {
                            self.bottom_pane
                                .update_status_text("waiting for command".to_string());
                        }
                        self.invalidate_height_cache();
                        self.request_redraw();
                        return;
                    }
                let running_entry = self
                    .tools_state
                    .running_custom_tools
                    .remove(&ToolCallId(call_id.clone()));
                let resolved_idx = running_entry
                    .as_ref()
                    .and_then(|entry| running_tools::resolve_entry_index(self, entry, &call_id))
                    .or_else(|| running_tools::find_by_call_id(self, &call_id));

                if tool_name == "apply_patch" && success {
                    if let Some(idx) = resolved_idx
                        && idx < self.history_cells.len() {
                            let is_running_tool = self.history_cells[idx]
                                .as_any()
                                .downcast_ref::<history_cell::RunningToolCallCell>()
                                .is_some();
                            if is_running_tool {
                                self.history_remove_at(idx);
                            }
                        }
                    self.bottom_pane
                        .update_status_text("responding".to_string());
                    self.maybe_hide_spinner();
                    return;
                }

                if tool_name == "wait" && success {
                    let target = wait_target_from_params(params_string.as_ref(), &call_id);
                    let wait_cell = history_cell::new_completed_wait_tool_call(target, duration);
                    let wait_state = wait_cell.state().clone();
                    if let Some(idx) = resolved_idx {
                        self.history_replace_with_record(
                            idx,
                            Box::new(wait_cell),
                            HistoryDomainRecord::WaitStatus(wait_state),
                        );
                    } else {
                        let _ = self.history_insert_with_key_global_tagged(
                            Box::new(wait_cell),
                            ok,
                            "untagged",
                            Some(HistoryDomainRecord::WaitStatus(wait_state)),
                        );
                    }
                    self.remove_background_completion_message(&call_id);
                    self.bottom_pane
                        .update_status_text("responding".to_string());
                    self.maybe_hide_spinner();
                    return;
                }
                if tool_name == "wait" && !success && content.trim() == "Cancelled by user." {
                    let mut emphasis = TextEmphasis::default();
                    emphasis.bold = true;
                    let wait_state = PlainMessageState {
                        id: HistoryId::ZERO,
                        role: PlainMessageRole::Error,
                        kind: PlainMessageKind::Error,
                        header: None,
                        lines: vec![MessageLine {
                            kind: MessageLineKind::Paragraph,
                            spans: vec![InlineSpan {
                                text: "Wait cancelled".into(),
                                tone: TextTone::Error,
                                emphasis,
                                entity: None,
                            }],
                        }],
                        metadata: None,
                    };

                    if let Some(idx) = resolved_idx {
                        self.history_replace_with_record(
                            idx,
                            Box::new(history_cell::PlainHistoryCell::from_state(wait_state.clone())),
                            HistoryDomainRecord::Plain(wait_state),
                        );
                    } else {
                        let _ = self.history_insert_plain_state_with_key(wait_state, ok, "untagged");
                    }

                    self.bottom_pane
                        .update_status_text("responding".to_string());
                    self.maybe_hide_spinner();
                    return;
                }
                if tool_name == "kill" {
                    let _ = self
                        .tools_state
                        .running_kill_tools
                        .remove(&ToolCallId(call_id.clone()));
                    if success {
                        self.remove_background_completion_message(&call_id);
                        self.bottom_pane
                            .update_status_text("responding".to_string());
                    } else {
                        let trimmed = content.trim();
                        if !trimmed.is_empty() {
                            self.push_background_tail(trimmed.to_string());
                        }
                        self.bottom_pane
                            .update_status_text("kill failed".to_string());
                    }
                    self.maybe_hide_spinner();
                    self.invalidate_height_cache();
                    self.request_redraw();
                    return;
                }
                // Special-case browser/web fetch to render returned markdown nicely.
                if tool_name == "web_fetch" || tool_name == "browser_fetch" {
                    let completed = history_cell::new_completed_web_fetch_tool_call(
                        &self.config,
                        params_string,
                        duration,
                        success,
                        content,
                    );
                    if let Some(idx) = resolved_idx {
                        self.history_replace_at(idx, Box::new(completed));
                    } else {
                        running_tools::collapse_spinner(self, &call_id);
                        let _ = self.history_insert_with_key_global(Box::new(completed), ok);
                    }

                    // After tool completes, likely transitioning to response
                    self.bottom_pane
                        .update_status_text("responding".to_string());
                    self.maybe_hide_spinner();
                    return;
                }
                let mut completed = history_cell::new_completed_custom_tool_call(
                    tool_name,
                    params_string,
                    duration,
                    success,
                    content,
                );
                completed.state_mut().call_id = Some(call_id.clone());
                if let Some(idx) = resolved_idx {
                    self.history_debug(format!(
                        "custom_tool_end.in_place call_id={} idx={} order=({}, {}, {})",
                        call_id,
                        idx,
                        ok.req,
                        ok.out,
                        ok.seq
                    ));
                    self.history_replace_at(idx, Box::new(completed));
                } else {
                    self.history_debug(format!(
                        "custom_tool_end.fallback_insert call_id={} order=({}, {}, {})",
                        call_id,
                        ok.req,
                        ok.out,
                        ok.seq
                    ));
                    running_tools::collapse_spinner(self, &call_id);
                    let _ = self.history_insert_with_key_global(Box::new(completed), ok);
                }

                if let Some(path) = image_view_path.as_ref()
                    && let Some(record) = image_record_from_path(path) {
                        let cell = Box::new(history_cell::ImageOutputCell::from_record(record));
                        let _ = self.history_insert_with_key_global(cell, ok);
                    }

                // After tool completes, likely transitioning to response
                self.bottom_pane
                    .update_status_text("responding".to_string());
                self.maybe_hide_spinner();
            }
            EventMsg::ViewImageToolCall(ViewImageToolCallEvent { call_id, path }) => {
                let ok = match event.order.as_ref() {
                    Some(om) => self.provider_order_key_from_order_meta(om),
                    None => {
                        tracing::warn!("missing OrderMeta on ViewImageToolCall; using synthetic key");
                        self.next_internal_key()
                    }
                };
                if let Some(record) = image_record_from_path(&path) {
                    let cell = Box::new(history_cell::ImageOutputCell::from_record(record));
                    let _ = self.history_insert_with_key_global(cell, ok);
                    self.tools_state
                        .image_viewed_calls
                        .insert(ToolCallId(call_id));
                }
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
                self.mcp_tools_by_server = ev.server_tools.unwrap_or_default();
                self.mcp_server_failures = ev.server_failures.unwrap_or_default();
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
                info!("BackgroundEvent: {message}");
                if browser_sessions::handle_background_event(
                    self,
                    event.order.as_ref(),
                    &message,
                ) {
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
                let placement = match event.order.as_ref().and_then(|om| om.output_index) {
                    Some(v) if v == i32::MAX as u32 => SystemPlacement::EndOfCurrent,
                    Some(_) => SystemPlacement::EarlyInCurrent,
                    None => SystemPlacement::EndOfCurrent,
                };
                let id_for_replace = Some(id.clone());
                let message_clone = message.clone();
                let cell = history_cell::new_background_event(message_clone);
                let record = HistoryDomainRecord::BackgroundEvent(cell.state().clone());
                self.push_system_cell(
                    Box::new(cell),
                    placement,
                    id_for_replace,
                    event.order.as_ref(),
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
            EventMsg::AgentStatusUpdate(event) => {
                agent_runs::handle_status_update(self, &event);
                let AgentStatusUpdateEvent { agents, context, task } = event;
                // Update the active agents list from the event and track timing
                self.active_agents.clear();
                let now = Instant::now();
                let mut saw_running = false;
                let mut has_running_non_auto_review = false;
                let mut has_running_auto_review = false;
                for agent in agents.iter() {
                    let parsed_status = agent_status_from_str(agent.status.as_str());
                    // Update runtime map
                    let entry = self
                        .agent_runtime
                        .entry(agent.id.clone())
                        .or_default();
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
                // getting stuck on "Responding..." after multi-agent runs that yield
                // early.
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

                if saw_running
                    && has_running_non_auto_review
                    && !self.bottom_pane.is_task_running()
                {
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
            EventMsg::BrowserScreenshotUpdate(payload) => {
                #[cfg(feature = "code-fork")]
                handle_browser_screenshot(&payload, &self.app_event_tx);

                let BrowserScreenshotUpdateEvent { screenshot_path, url } = payload;
                let update = browser_sessions::handle_screenshot_update(
                    self,
                    event.order.as_ref(),
                    &screenshot_path,
                    &url,
                );
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
                    self.browser_overlay_state
                        .set_session_key(Some(key.clone()));
                    if let Some(tracker) = self.tools_state.browser_sessions.get(key) {
                        let len = tracker.cell.screenshot_history().len();
                        if len > 0 {
                            let last_index = len.saturating_sub(1);
                            let current_index = self.browser_overlay_state.screenshot_index();
                            if !self.browser_overlay_visible || current_index >= last_index {
                                self.browser_overlay_state
                                    .set_screenshot_index(last_index);
                            }
                        }
                    }
                }

                // Request a redraw to update the display immediately
                self.app_event_tx.send(AppEvent::RequestRedraw);

                if update.grouped {
                    self.bottom_pane
                        .update_status_text("using browser".to_string());
                }
            }
            // Newer protocol variants we currently ignore in the TUI
            EventMsg::UserMessage(_) => {}
            EventMsg::TurnAborted(_) => {
                self.pending_request_user_input = None;
            }
            EventMsg::ConversationPath(_) => {}
            EventMsg::EnteredReviewMode(review_request) => {
                if self.auto_resolve_enabled() {
                    self.auto_resolve_handle_review_enter();
                }
                let hint = review_request.user_facing_hint.trim();
                let banner = if hint.is_empty() {
                    ">> Code review started <<".to_string()
                } else {
                    format!(">> Code review started: {hint} <<")
                };
                self.active_review_hint = Some(review_request.user_facing_hint.clone());
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
            EventMsg::ExitedReviewMode(review_event) => {
                if self.auto_resolve_enabled() {
                    self.auto_resolve_handle_review_exit(review_event.review_output.clone());
                }
                self.review_guard = None;
                let hint = self.active_review_hint.take();
                let prompt = self.active_review_prompt.take();
                match review_event.review_output {
                    Some(output) => {
                        let summary_cell = self.build_review_summary_cell(
                            hint.as_deref(),
                            prompt.as_deref(),
                            &output,
                        );
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
                                format!(
                                    "<< Code review finished without a final response ({trimmed}) >>"
                                )
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
    }

}
