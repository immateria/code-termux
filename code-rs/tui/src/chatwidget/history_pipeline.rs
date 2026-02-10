use super::*;

impl ChatWidget<'_> {
    pub(crate) fn show_resume_picker(&mut self) {
        if self.resume_picker_loading {
            self.bottom_pane
                .flash_footer_notice("Still loading past sessions…".to_string());
            return;
        }
        self.resume_picker_loading = true;
        self.bottom_pane.flash_footer_notice_for(
            "Loading past sessions…".to_string(),
            std::time::Duration::from_secs(30),
        );
        self.request_redraw();

        let cwd = self.config.cwd.clone();
        let code_home = self.config.code_home.clone();
        let exclude_path = self.config.experimental_resume.clone();
        let tx = self.app_event_tx.clone();

        tokio::spawn(async move {
            let fetch_cwd = cwd.clone();
            let fetch_code_home = code_home.clone();
            let fetch_exclude = exclude_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                crate::resume::discovery::list_sessions_for_cwd(
                    &fetch_cwd,
                    &fetch_code_home,
                    fetch_exclude.as_deref(),
                )
            })
            .await;

            match result {
                Ok(candidates) => {
                    tx.send(AppEvent::ResumePickerLoaded { cwd, candidates });
                }
                Err(err) => {
                    tx.send(AppEvent::ResumePickerLoadFailed {
                        message: format!("Failed to load past sessions: {err}"),
                    });
                }
            }
        });
    }

    pub(super) fn resume_rows_from_candidates(
        candidates: Vec<crate::resume::discovery::ResumeCandidate>,
    ) -> Vec<crate::bottom_pane::resume_selection_view::ResumeRow> {
        fn human_ago(ts: &str) -> String {
            use chrono::{DateTime, Local};
            if let Ok(dt) = DateTime::parse_from_rfc3339(ts) {
                let local_dt = dt.with_timezone(&Local);
                let now = Local::now();
                let delta = now.signed_duration_since(local_dt);
                let secs = delta.num_seconds().max(0);
                let mins = secs / 60;
                let hours = mins / 60;
                let days = hours / 24;
                if days >= 7 {
                    return local_dt.format("%Y-%m-%d %H:%M").to_string();
                }
                if days >= 1 {
                    return format!("{days}d ago");
                }
                if hours >= 1 {
                    return format!("{hours}h ago");
                }
                if mins >= 1 {
                    return format!("{mins}m ago");
                }
                return "just now".to_string();
            }
            ts.to_string()
        }

        candidates
            .into_iter()
            .map(|c| {
                let modified = human_ago(&c.modified_ts.unwrap_or_default());
                let created = human_ago(&c.created_ts.unwrap_or_default());
                let user_message_count = c.user_message_count;
                let user_msgs = format!("{user_message_count}");
                let branch = c.branch.unwrap_or_else(|| "-".to_string());
                let nickname = c
                    .nickname
                    .and_then(|name| {
                        let trimmed = name.trim();
                        (!trimmed.is_empty()).then(|| trimmed.to_string())
                    });
                let snippet = c.snippet.or(c.subtitle);
                let mut summary = match (nickname, snippet) {
                    (Some(name), Some(snippet)) => format!("{name} - {snippet}"),
                    (Some(name), None) => name,
                    (None, Some(snippet)) => snippet,
                    (None, None) => String::new(),
                };
                const SNIPPET_MAX: usize = 64;
                if summary.chars().count() > SNIPPET_MAX {
                    summary = summary.chars().take(SNIPPET_MAX).collect::<String>() + "…";
                }
                crate::bottom_pane::resume_selection_view::ResumeRow {
                    modified,
                    created,
                    user_msgs,
                    branch,
                    last_user_message: summary,
                    path: c.path,
                }
            })
            .collect()
    }

    pub(crate) fn present_resume_picker(
        &mut self,
        cwd: std::path::PathBuf,
        candidates: Vec<crate::resume::discovery::ResumeCandidate>,
    ) {
        self.resume_picker_loading = false;
        if candidates.is_empty() {
            self.bottom_pane
                .flash_footer_notice("No past sessions found for this folder".to_string());
            self.request_redraw();
            return;
        }
        let rows = Self::resume_rows_from_candidates(candidates);
        let count = rows.len();
        let title = format!("Resume Session — {}", cwd.display());
        self.bottom_pane
            .show_resume_selection(title, Some(String::new()), rows);
        self.bottom_pane
            .flash_footer_notice(format!("Loaded {count} past sessions."));
        self.request_redraw();
    }

    pub(crate) fn handle_resume_picker_load_failed(&mut self, message: String) {
        self.resume_picker_loading = false;
        self.bottom_pane.flash_footer_notice(message);
        self.request_redraw();
    }

    /// Render a single recorded ResponseItem into history without executing tools
    pub(super) fn render_replay_item(&mut self, item: ResponseItem) {
        match item {
            ResponseItem::Message { id, role, content } => {
                let message_id = id;
                let mut text = String::new();
                for c in content {
                    match c {
                        ContentItem::OutputText { text: t }
                        | ContentItem::InputText { text: t } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(&t);
                        }
                        _ => {}
                    }
                }
                let text = text.trim();
                if text.is_empty() {
                    return;
                }
                if role == "user" {
                    if text.starts_with("<user_action>") {
                        return;
                    }
                    if let Some(expected) = self.pending_dispatched_user_messages.front()
                        && expected.trim() == text {
                            self.pending_dispatched_user_messages.pop_front();
                            return;
                        }
                }
                if text.starts_with("== System Status ==") {
                    return;
            }
            if role == "assistant" {
                let normalized_new = Self::normalize_text(text);
                if let Some(last_cell) = self.history_cells.last()
                    && let Some(existing) = last_cell
                        .as_any()
                        .downcast_ref::<crate::history_cell::AssistantMarkdownCell>()
                    {
                        let normalized_existing =
                            Self::normalize_text(existing.markdown());
                        if normalized_existing == normalized_new {
                            tracing::debug!(
                                "replay: skipping duplicate assistant message"
                            );
                            return;
                        }
                    }
                let mut lines: Vec<ratatui::text::Line<'static>> = Vec::new();
                crate::markdown::append_markdown(text, &mut lines, &self.config);
                self.insert_final_answer_with_id(message_id, lines, text.to_string());
                return;
            }
                if role == "user" {
                    let key = self.next_internal_key();
                    let state = history_cell::new_user_prompt(text.to_string());
                    let _ = self.history_insert_plain_state_with_key(state, key, "epilogue");

                    if let Some(front) = self.queued_user_messages.front()
                        && front.display_text.trim() == text.trim() {
                            self.queued_user_messages.pop_front();
                            self.refresh_queued_user_messages(false);
                        }
                } else {
                    let mut lines = Vec::new();
                    crate::markdown::append_markdown(text, &mut lines, &self.config);
                    let key = self.next_internal_key();
                    let state = history_cell::plain_message_state_from_lines(
                        lines,
                        history_cell::HistoryCellType::Assistant,
                    );
                    let _ = self.history_insert_plain_state_with_key(state, key, "epilogue");
                }
            }
            ResponseItem::FunctionCall { name, arguments, call_id, .. } => {
                let mut message = self
                    .format_tool_call_preview(&name, &arguments)
                    .unwrap_or_else(|| {
                        let pretty_args = serde_json::from_str::<JsonValue>(&arguments)
                            .and_then(|v| serde_json::to_string_pretty(&v))
                            .unwrap_or_else(|_| arguments.clone());
                        let mut m = format!("🔧 Tool call: {name}");
                        if !pretty_args.trim().is_empty() {
                            m.push('\n');
                            m.push_str(&pretty_args);
                        }
                        m
                    });
                if !call_id.is_empty() {
                    message.push_str(&format!("\ncall_id: {call_id}"));
                }
                let key = self.next_internal_key();
                let _ = self.history_insert_with_key_global_tagged(
                    Box::new(crate::history_cell::new_background_event(message)),
                    key,
                    "background",
                    None,
                );
            }
            ResponseItem::Reasoning { summary, .. } => {
                for s in summary {
                    let code_protocol::models::ReasoningItemReasoningSummary::SummaryText { text } =
                        s;
                    // Reasoning cell – use the existing reasoning output styling
                    let sink = crate::streaming::controller::AppEventHistorySink(
                        self.app_event_tx.clone(),
                    );
                    streaming::begin(self, StreamKind::Reasoning, None);
                    let _ = self.stream.apply_final_reasoning(&text, &sink);
                    // finalize immediately for static replay
                    self.stream
                        .finalize(crate::streaming::StreamKind::Reasoning, true, &sink);
                }
            }
            ResponseItem::FunctionCallOutput { output, call_id, .. } => {
                let mut content = output.content;
                let mut metadata_summary = String::new();
                if let Ok(v) = serde_json::from_str::<JsonValue>(&content) {
                    if let Some(s) = v.get("output").and_then(|x| x.as_str()) {
                        content = s.to_string();
                    }
                    if let Some(meta) = v.get("metadata").and_then(|m| m.as_object()) {
                        let mut parts = Vec::new();
                        if let Some(code) = meta.get("exit_code").and_then(serde_json::Value::as_i64) {
                            parts.push(format!("exit_code={code}"));
                        }
                        if let Some(duration) =
                            meta.get("duration_seconds").and_then(serde_json::Value::as_f64)
                        {
                            parts.push(format!("duration={duration:.2}s"));
                        }
                        if !parts.is_empty() {
                            metadata_summary = parts.join(", ");
                        }
                    }
                }
                let mut message = String::new();
                if !content.trim().is_empty() {
                    message.push_str(content.trim_end());
                }
                if !metadata_summary.is_empty() {
                    if !message.is_empty() {
                        message.push_str("\n\n");
                    }
                    message.push_str(&format!("({metadata_summary})"));
                }
                if !call_id.is_empty() {
                    if !message.is_empty() {
                        message.push('\n');
                    }
                    message.push_str(&format!("call_id: {call_id}"));
                }
                if message.trim().is_empty() {
                    return;
                }
                let key = self.next_internal_key();
                let _ = self.history_insert_with_key_global_tagged(
                    Box::new(crate::history_cell::new_background_event(message)),
                    key,
                    "background",
                    None,
                );
            }
            _ => {
                // Ignore other item kinds for replay (tool calls, etc.)
            }
        }
    }

    pub(super) fn is_auto_review_cell(item: &dyn HistoryCell) -> bool {
        item.as_any()
            .downcast_ref::<crate::history_cell::PlainHistoryCell>()
            .map(crate::history_cell::PlainHistoryCell::is_auto_review_notice)
            .unwrap_or(false)
    }

    pub(super) fn render_cached_lines(
        &self,
        item: &dyn HistoryCell,
        layout: &CachedLayout,
        area: Rect,
        buf: &mut Buffer,
        skip_rows: u16,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let total = layout.lines.len() as u16;
        if skip_rows >= total {
            return;
        }

        debug_assert_eq!(layout.lines.len(), layout.rows.len());

        let is_assistant = matches!(item.kind(), crate::history_cell::HistoryCellType::Assistant);
        let is_auto_review = ChatWidget::is_auto_review_cell(item);
        let cell_bg = if is_assistant {
            crate::colors::assistant_bg()
        } else if is_auto_review {
            crate::history_cell::PlainHistoryCell::auto_review_bg()
        } else {
            crate::colors::background()
        };

        if is_assistant || is_auto_review {
            let bg_style = Style::default()
                .bg(cell_bg)
                .fg(crate::colors::text());
            fill_rect(buf, area, Some(' '), bg_style);
        }

        let max_rows = area.height.min(total.saturating_sub(skip_rows));
        let buf_width = buf.area.width as usize;
        let offset_x = area.x.saturating_sub(buf.area.x) as usize;
        let offset_y = area.y.saturating_sub(buf.area.y) as usize;
        let row_width = area.width as usize;

        for (visible_offset, src_index) in (skip_rows as usize..skip_rows as usize + max_rows as usize)
            .enumerate()
        {
            let src_row = layout
                .rows
                .get(src_index)
                .map(std::convert::AsRef::as_ref)
                .unwrap_or(&[]);

            let dest_y = offset_y + visible_offset;
            if dest_y >= buf.area.height as usize {
                break;
            }
            let start = dest_y * buf_width + offset_x;
            if start >= buf.content.len() {
                break;
            }
            let max_width = row_width.min(buf_width.saturating_sub(offset_x));
            let end = (start + max_width).min(buf.content.len());
            if end <= start {
                continue;
            }
            let dest_slice = &mut buf.content[start..end];

            let copy_len = src_row.len().min(dest_slice.len());
            if copy_len == dest_slice.len() {
                if copy_len > 0 {
                    dest_slice.clone_from_slice(&src_row[..copy_len]);
                }
            } else {
                for (dst, src) in dest_slice.iter_mut().zip(src_row.iter()).take(copy_len) {
                    dst.clone_from(src);
                }
                for cell in dest_slice.iter_mut().skip(copy_len) {
                    cell.reset();
                }
            }

            for cell in dest_slice.iter_mut() {
                if cell.bg == ratatui::style::Color::Reset {
                    cell.bg = cell_bg;
                }
            }
        }
    }
    /// Trigger fade on the welcome cell when the composer expands (e.g., slash popup).
    pub(crate) fn on_composer_expanded(&mut self) {
        for cell in &self.history_cells {
            cell.trigger_fade();
        }
        self.request_redraw();
    }
    /// If the user is at the bottom, keep following new messages.
    pub(super) fn autoscroll_if_near_bottom(&mut self) {
        layout_scroll::autoscroll_if_near_bottom(self);
    }

    pub(super) fn clear_reasoning_in_progress(&mut self) {
        let last_reasoning_index = self
            .history_cells
            .iter()
            .enumerate()
            .rev()
            .find_map(|(idx, cell)| {
                cell.as_any()
                    .downcast_ref::<history_cell::CollapsibleReasoningCell>()
                    .map(|_| idx)
            });

        let mut changed = false;
        for (idx, cell) in self.history_cells.iter().enumerate() {
            if let Some(reasoning_cell) = cell
                .as_any()
                .downcast_ref::<history_cell::CollapsibleReasoningCell>()
            {
                if !reasoning_cell.is_in_progress() {
                    continue;
                }

                let keep_in_progress = !self.config.tui.show_reasoning
                    && Some(idx) == last_reasoning_index
                    && reasoning_cell.is_collapsed()
                    && !reasoning_cell.collapsed_has_summary();

                if keep_in_progress {
                    continue;
                }

                reasoning_cell.set_in_progress(false);
                changed = true;
            }
        }

        if changed {
            self.invalidate_height_cache();
        }
    }

    pub(super) fn reasoning_preview(lines: &[Line<'static>]) -> String {
        const MAX_LINES: usize = 3;
        const MAX_CHARS: usize = 120;
        let mut previews: Vec<String> = Vec::new();
        for line in lines.iter().take(MAX_LINES) {
            let mut text = String::new();
            for span in &line.spans {
                text.push_str(span.content.as_ref());
            }
            if text.chars().count() > MAX_CHARS {
                let mut truncated: String = text.chars().take(MAX_CHARS).collect();
                truncated.push('…');
                previews.push(truncated);
            } else {
                previews.push(text);
            }
        }
        if previews.is_empty() {
            String::new()
        } else {
            previews.join(" ⏐ ")
        }
    }

    pub(super) fn refresh_reasoning_collapsed_visibility(&mut self) {
        let show = self.config.tui.show_reasoning;
        let mut needs_invalidate = false;
        if show {
            for cell in &self.history_cells {
                if let Some(reasoning_cell) = cell
                    .as_any()
                    .downcast_ref::<history_cell::CollapsibleReasoningCell>()
                    && reasoning_cell.set_hide_when_collapsed(false) {
                        needs_invalidate = true;
                    }
            }
        } else {
            // When reasoning is hidden (collapsed), we still show a single summary
            // line for the most recent reasoning in any consecutive run. Earlier
            // reasoning cells in the run are hidden entirely.
            use std::collections::HashSet;
            let mut hide_indices: HashSet<usize> = HashSet::new();
            let len = self.history_cells.len();
            let mut idx = 0usize;
            while idx < len {
                let cell = &self.history_cells[idx];
                let is_reasoning = cell
                    .as_any()
                    .downcast_ref::<history_cell::CollapsibleReasoningCell>()
                    .is_some();
                if !is_reasoning {
                    idx += 1;
                    continue;
                }

                let mut reasoning_indices: Vec<usize> = vec![idx];
                let mut j = idx + 1;
                while j < len {
                    let cell = &self.history_cells[j];

                    if cell.should_remove() {
                        j += 1;
                        continue;
                    }

                    if cell
                        .as_any()
                        .downcast_ref::<history_cell::CollapsibleReasoningCell>()
                        .is_some()
                    {
                        reasoning_indices.push(j);
                        j += 1;
                        continue;
                    }

                    match cell.kind() {
                        history_cell::HistoryCellType::PlanUpdate
                        | history_cell::HistoryCellType::Loading => {
                            j += 1;
                            continue;
                        }
                        _ => {}
                    }

                    if cell
                        .as_any()
                        .downcast_ref::<history_cell::WaitStatusCell>()
                        .is_some()
                    {
                        j += 1;
                        continue;
                    }

                    if self.cell_lines_trimmed_is_empty(j, cell.as_ref()) {
                        j += 1;
                        continue;
                    }

                    break;
                }

                if reasoning_indices.len() > 1 {
                    for &ri in &reasoning_indices[..reasoning_indices.len() - 1] {
                        hide_indices.insert(ri);
                    }
                }

                idx = j;
            }

            for (i, cell) in self.history_cells.iter().enumerate() {
                if let Some(reasoning_cell) = cell
                    .as_any()
                    .downcast_ref::<history_cell::CollapsibleReasoningCell>()
                {
                    let hide = hide_indices.contains(&i);
                    if reasoning_cell.set_hide_when_collapsed(hide) {
                        needs_invalidate = true;
                    }
                }
            }
        }

        if needs_invalidate {
            self.invalidate_height_cache();
            self.request_redraw();
        }

        self.refresh_explore_trailing_flags();
    }

    // Handle streaming delta for both answer and reasoning.
    // Legacy helper removed: streaming now requires explicit sequence numbers.
    // Call sites should invoke `streaming::delta_text(self, kind, id, delta, seq)` directly.

    /// Defer or handle an interrupt based on whether we're streaming
    pub(super) fn defer_or_handle<F1, F2>(&mut self, defer_fn: F1, handle_fn: F2)
    where
        F1: FnOnce(&mut interrupts::InterruptManager),
        F2: FnOnce(&mut Self),
    {
        if self.is_write_cycle_active() {
            defer_fn(&mut self.interrupts);
            self.schedule_interrupt_flush_check();
        } else {
            handle_fn(self);
        }
    }

    // removed: next_sequence; plan updates are inserted immediately

    // Removed order-adjustment helpers; ordering now uses stable order keys on insert.

    /// Mark that the widget needs to be redrawn
    pub(super) fn mark_needs_redraw(&mut self) {
        // Clean up fully faded cells before redraw. If any are removed,
        // invalidate the height cache since indices shift and our cache is
        // keyed by (idx,width).
        let before_len = self.history_cells.len();
        if before_len > 0 {
            let old_cells = std::mem::take(&mut self.history_cells);
            let old_ids = std::mem::take(&mut self.history_cell_ids);
            debug_assert_eq!(
                old_cells.len(),
                old_ids.len(),
                "history ids out of sync with cells"
            );
            let mut removed_any = false;
            let mut kept_cells = Vec::with_capacity(old_cells.len());
            let mut kept_ids = Vec::with_capacity(old_ids.len());
            for (cell, id) in old_cells.into_iter().zip(old_ids.into_iter()) {
                if cell.should_remove() {
                    removed_any = true;
                    continue;
                }
                kept_ids.push(id);
                kept_cells.push(cell);
            }
            self.history_cells = kept_cells;
            self.history_cell_ids = kept_ids;
            if removed_any {
                self.invalidate_height_cache();
            }
        } else if !self.history_cell_ids.is_empty() {
            self.history_cell_ids.clear();
        }

        // Send a redraw event to trigger UI update
        self.app_event_tx.send(AppEvent::RequestRedraw);
    }

    /// Clear memoized cell heights (called when history/content changes)
    /// Handle exec approval request immediately
    pub(super) fn handle_exec_approval_now(&mut self, _id: String, ev: ExecApprovalRequestEvent) {
        // Use call_id as the approval correlation id so responses map to the
        // exact pending approval in core (supports multiple approvals per turn).
        let approval_id = ev.call_id.clone();
        let ticket = self.make_background_before_next_output_ticket();
        self.bottom_pane
            .push_approval_request(ApprovalRequest::Exec {
                id: approval_id,
                command: ev.command,
                reason: ev.reason,
            }, ticket);
    }

    /// Handle apply patch approval request immediately
    pub(super) fn handle_apply_patch_approval_now(&mut self, _id: String, ev: ApplyPatchApprovalRequestEvent) {
        let ApplyPatchApprovalRequestEvent {
            call_id,
            changes,
            reason,
            grant_root,
        } = ev;

        // Clone for session storage before moving into history
        let changes_clone = changes.clone();
        // Surface the patch summary in the main conversation
        let key = self.next_internal_key();
        let _ = self.history_insert_with_key_global(
            Box::new(history_cell::new_patch_event(
                history_cell::PatchEventType::ApprovalRequest,
                changes,
            )),
            key,
        );
        // Record change set for session diff popup (latest last)
        self.diffs.session_patch_sets.push(changes_clone);
        // For any new paths, capture an original baseline snapshot the first time we see them
        if let Some(last) = self.diffs.session_patch_sets.last() {
            for (src_path, chg) in last.iter() {
                match chg {
                    code_core::protocol::FileChange::Update {
                        move_path: Some(dest_path),
                        ..
                    } => {
                        if let Some(baseline) =
                            self.diffs.baseline_file_contents.get(src_path).cloned()
                        {
                            // Mirror baseline under destination so tabs use the new path
                            self.diffs
                                .baseline_file_contents
                                .entry(dest_path.clone())
                                .or_insert(baseline);
                        } else if !self.diffs.baseline_file_contents.contains_key(dest_path) {
                            // Snapshot from source (pre-apply)
                            let baseline = std::fs::read_to_string(src_path).unwrap_or_default();
                            self.diffs
                                .baseline_file_contents
                                .insert(dest_path.clone(), baseline);
                        }
                    }
                    _ => {
                        if !self.diffs.baseline_file_contents.contains_key(src_path) {
                            let baseline = std::fs::read_to_string(src_path).unwrap_or_default();
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

        // Push the approval request to the bottom pane, keyed by call_id
        let request = ApprovalRequest::ApplyPatch {
            id: call_id,
            reason,
            grant_root,
        };
        let ticket = self.make_background_before_next_output_ticket();
        self.bottom_pane.push_approval_request(request, ticket);
    }

    /// Handle exec command begin immediately
    pub(super) fn handle_exec_begin_now(
        &mut self,
        ev: ExecCommandBeginEvent,
        order: &code_core::protocol::OrderMeta,
    ) {
        exec_tools::handle_exec_begin_now(self, ev, order);
    }

    /// Common exec-begin handling used for both immediate and deferred paths.
    /// Ensures we finalize any active stream, create the running cell, and
    /// immediately apply a pending end if it arrived first.
    pub(super) fn handle_exec_begin_ordered(
        &mut self,
        ev: ExecCommandBeginEvent,
        order: code_core::protocol::OrderMeta,
        seq: u64,
    ) {
        self.finalize_active_stream();
        tracing::info!(
            "[order] ExecCommandBegin call_id={} seq={}",
            ev.call_id,
            seq
        );
        self.handle_exec_begin_now(ev.clone(), &order);
        self.ensure_spinner_for_activity("exec-begin");
        if let Some((pending_end, order2, _ts)) = self
            .exec
            .pending_exec_ends
            .remove(&ExecCallId(ev.call_id))
        {
            self.handle_exec_end_now(pending_end, &order2);
        }
        if self.interrupts.has_queued() {
            self.flush_interrupt_queue();
        }
    }

    /// Handle exec command end immediately
    pub(super) fn handle_exec_end_now(
        &mut self,
        ev: ExecCommandEndEvent,
        order: &code_core::protocol::OrderMeta,
    ) {
        exec_tools::handle_exec_end_now(self, ev, order);
    }

    /// Handle or defer an exec end based on whether the matching begin has
    /// already been seen. When no running entry exists yet, stash the end so
    /// it can be paired once the begin arrives, falling back to a timed flush.
    pub(super) fn enqueue_or_handle_exec_end(
        &mut self,
        ev: ExecCommandEndEvent,
        order: code_core::protocol::OrderMeta,
    ) {
        let call_id = ExecCallId(ev.call_id.clone());
        let has_running = self.exec.running_commands.contains_key(&call_id);
        if has_running {
            self.handle_exec_end_now(ev, &order);
            return;
        }

        // If the history already knows about this call_id (e.g., Begin was handled
        // but running_commands was cleared), finish it immediately to avoid leaving
        // the cell stuck in a running state.
        if self
            .history_state
            .history_id_for_exec_call(call_id.as_ref())
            .is_some()
        {
            self.handle_exec_end_now(ev, &order);
            return;
        }

        self.exec
            .pending_exec_ends
            .insert(call_id, (ev, order.clone(), std::time::Instant::now()));
        let tx = self.app_event_tx.clone();
        let fallback_tx = tx.clone();
        if thread_spawner::spawn_lightweight("exec-flush", move || {
            std::thread::sleep(std::time::Duration::from_millis(120));
            tx.send(crate::app_event::AppEvent::FlushPendingExecEnds);
        })
        .is_none()
        {
            fallback_tx.send(crate::app_event::AppEvent::FlushPendingExecEnds);
        }
    }

    pub(super) fn build_patch_failure_metadata(stdout: &str, stderr: &str) -> PatchFailureMetadata {
        fn sanitize(text: &str) -> String {
            let normalized = history_cell::normalize_overwrite_sequences(text);
            sanitize_for_tui(
                &normalized,
                SanitizeMode::AnsiPreserving,
                SanitizeOptions {
                    expand_tabs: true,
                    tabstop: 4,
                    debug_markers: false,
                },
            )
        }

        fn excerpt(input: &str) -> Option<String> {
            let trimmed = input.trim();
            if trimmed.is_empty() {
                return None;
            }
            const MAX_CHARS: usize = 2_000;
            const MAX_LINES: usize = 20;
            let mut excerpt = String::new();
            let mut remaining = MAX_CHARS;
            for (idx, line) in trimmed.lines().enumerate() {
                if idx >= MAX_LINES || remaining == 0 {
                    break;
                }
                let line = line.trim_end_matches('\r');
                let mut line_chars = line.chars();
                let mut chunk = String::new();
                while remaining > 0 {
                    if let Some(ch) = line_chars.next() {
                        let width = ch.len_utf8();
                        if width > remaining {
                            break;
                        }
                        chunk.push(ch);
                        remaining -= width;
                    } else {
                        break;
                    }
                }
                if chunk.len() < line.len() {
                    chunk.push('…');
                    remaining = 0;
                }
                if !excerpt.is_empty() {
                    excerpt.push('\n');
                }
                excerpt.push_str(&chunk);
                if remaining == 0 {
                    break;
                }
            }
            Some(excerpt)
        }

        let sanitized_stdout = sanitize(stdout);
        let sanitized_stderr = sanitize(stderr);
        let message = sanitized_stderr
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(std::string::ToString::to_string)
            .unwrap_or_else(|| "Patch application failed".to_string());

        PatchFailureMetadata {
            message,
            stdout_excerpt: excerpt(&sanitized_stdout),
            stderr_excerpt: excerpt(&sanitized_stderr),
        }
    }

    // If a completed exec cell sits at `idx`, attempt to merge it into the
    // previous cell when they represent the same action header (e.g., Search, Read).

    // MCP tool call handlers now live in chatwidget::tools

    /// Handle patch apply end immediately
    pub(super) fn handle_patch_apply_end_now(&mut self, ev: PatchApplyEndEvent) {
        if ev.success {
            if let Some(idx) = self.history_cells.iter().rposition(|cell| {
                matches!(
                    cell.kind(),
                    crate::history_cell::HistoryCellType::Patch {
                        kind: crate::history_cell::PatchKind::ApplyBegin
                    } | crate::history_cell::HistoryCellType::Patch {
                        kind: crate::history_cell::PatchKind::Proposed
                    }
                )
            })
                && let Some(record) = self
                    .history_cells
                    .get(idx)
                    .and_then(|existing| self.record_from_cell_or_state(idx, existing.as_ref()))
                    && let HistoryRecord::Patch(mut patch_record) = record {
                        patch_record.patch_type = HistoryPatchEventType::ApplySuccess;
                        let record_index = self
                            .record_index_for_cell(idx)
                            .unwrap_or_else(|| self.record_index_for_position(idx));
                        let mutation = self
                            .history_state
                            .apply_domain_event(HistoryDomainEvent::Replace {
                                index: record_index,
                                record: HistoryDomainRecord::Patch(patch_record),
                            });
                        if let Some(id) = self.apply_mutation_to_cell_index(idx, mutation) {
                            if idx < self.history_cell_ids.len() {
                                self.history_cell_ids[idx] = Some(id);
                            }
                            self.maybe_hide_spinner();
                            return;
                        }
                    }
            self.maybe_hide_spinner();
            return;
        }

        let failure_meta = Self::build_patch_failure_metadata(&ev.stdout, &ev.stderr);
        if let Some(idx) = self.history_cells.iter().rposition(|cell| {
            matches!(
                cell.kind(),
                crate::history_cell::HistoryCellType::Patch {
                    kind: crate::history_cell::PatchKind::ApplyBegin
                } | crate::history_cell::HistoryCellType::Patch {
                    kind: crate::history_cell::PatchKind::Proposed
                }
            )
        })
            && let Some(record) = self
                .history_cells
                .get(idx)
                .and_then(|existing| self.record_from_cell_or_state(idx, existing.as_ref()))
                && let HistoryRecord::Patch(mut patch_record) = record {
                    patch_record.patch_type = HistoryPatchEventType::ApplyFailure;
                    patch_record.failure = Some(failure_meta.clone());
                    let record_index = self
                        .record_index_for_cell(idx)
                        .unwrap_or_else(|| self.record_index_for_position(idx));
                    let mutation = self
                        .history_state
                        .apply_domain_event(HistoryDomainEvent::Replace {
                            index: record_index,
                            record: HistoryDomainRecord::Patch(patch_record),
                        });
                    if let Some(_id) = self.apply_mutation_to_cell_index(idx, mutation) {
                        self.maybe_hide_spinner();
                        return;
                    }
                }

        let record = PatchRecord {
            id: HistoryId::ZERO,
            patch_type: HistoryPatchEventType::ApplyFailure,
            changes: HashMap::new(),
            failure: Some(failure_meta),
        };
        let cell = history_cell::PatchSummaryCell::from_record(record.clone());
        let key = self.next_internal_key();
        let _ = self.history_insert_with_key_global_tagged(
            Box::new(cell),
            key,
            "patch-failure",
            Some(HistoryDomainRecord::Patch(record)),
        );
        self.maybe_hide_spinner();
    }


    /// Get or create the global browser manager
    pub(super) async fn get_browser_manager() -> Arc<BrowserManager> {
        code_browser::global::get_or_create_browser_manager().await
    }

    pub(crate) fn insert_str(&mut self, s: &str) {
        if self.auto_state.should_show_goal_entry()
            && matches!(self.auto_goal_escape_state, AutoGoalEscState::Inactive)
            && !s.trim().is_empty()
        {
            self.auto_goal_escape_state = AutoGoalEscState::NeedsEnableEditing;
        }
        self.bottom_pane.insert_str(s);
    }

    pub(crate) fn set_composer_text(&mut self, text: String) {
        if self.auto_state.should_show_goal_entry()
            && matches!(self.auto_goal_escape_state, AutoGoalEscState::Inactive)
            && !text.trim().is_empty()
        {
            self.auto_goal_escape_state = AutoGoalEscState::NeedsEnableEditing;
        }
        self.bottom_pane.set_composer_text(text);
    }

    // Removed: pending insert sequencing is not used under strict ordering.

    pub(crate) fn register_pasted_image(&mut self, placeholder: String, path: std::path::PathBuf) {
        let persisted = self
            .persist_user_image_if_needed(&path)
            .unwrap_or_else(|| path.clone());
        if persisted.exists() && persisted.is_file() {
            self.pending_images.insert(placeholder, persisted);
            self.request_redraw();
            return;
        }

        // Some terminals (notably on macOS) can drop a "promised" file path
        // (e.g., from Preview) that doesn't actually exist on disk. Fall back
        // to reading the image directly from the clipboard.
        match crate::clipboard_paste::paste_image_to_temp_png() {
            Ok((clipboard_path, _info)) => {
                let clipboard_persisted = self
                    .persist_user_image_if_needed(&clipboard_path)
                    .unwrap_or(clipboard_path);
                self.pending_images.insert(placeholder, clipboard_persisted);
                self.push_background_tail("Used clipboard image (dropped file path was missing).");
                self.request_redraw();
            }
            Err(err) => {
                tracing::warn!(
                    "dropped/pasted image path missing ({}); clipboard fallback failed: {}",
                    persisted.display(),
                    err
                );
            }
        }
    }

    pub(super) fn persist_user_image_if_needed(&self, path: &std::path::Path) -> Option<PathBuf> {
        if !path.exists() || !path.is_file() {
            return None;
        }

        let temp_dir = std::env::temp_dir();
        let path_lossy = path.to_string_lossy();
        let looks_ephemeral = path.starts_with(&temp_dir)
            || path_lossy.contains("/TemporaryItems/")
            || path_lossy.contains("\\TemporaryItems\\");
        if !looks_ephemeral {
            return None;
        }

        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("png")
            .to_string();

        let mut dir = self
            .config
            .code_home
            .join("working")
            .join("_pasted_images");
        if let Some(session_id) = self.session_id {
            dir = dir.join(session_id.to_string());
        }

        if let Err(err) = std::fs::create_dir_all(&dir) {
            tracing::warn!(
                "failed to create pasted image dir {}: {}",
                dir.display(),
                err
            );
            return None;
        }

        let file_name = format!("dropped-{}.{}", Uuid::new_v4(), ext);
        let dest = dir.join(file_name);

        match std::fs::copy(path, &dest) {
            Ok(_) => Some(dest),
            Err(err) => {
                tracing::warn!(
                    "failed to persist dropped image {} -> {}: {}",
                    path.display(),
                    dest.display(),
                    err
                );
                None
            }
        }
    }

    pub(super) fn parse_message_with_images(&mut self, text: String) -> UserMessage {
        use std::path::Path;

        // Common image extensions
        const IMAGE_EXTENSIONS: &[&str] = &[
            ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".svg", ".ico", ".tiff", ".tif",
        ];
        // We keep a visible copy of the original (normalized) text for history
        let mut display_text = text.clone();
        let mut ordered_items: Vec<InputItem> = Vec::new();

        // First, handle [image: ...] placeholders from drag-and-drop
        let Ok(placeholder_regex) = regex_lite::Regex::new(r"\[image: [^\]]+\]") else {
            return UserMessage {
                display_text: text.clone(),
                ordered_items: vec![InputItem::Text { text }],
                suppress_persistence: false,
            };
        };
        let mut cursor = 0usize;
        for mat in placeholder_regex.find_iter(&text) {
            // Push preceding text as a text item (if any)
            if mat.start() > cursor {
                let chunk = &text[cursor..mat.start()];
                if !chunk.trim().is_empty() {
                    ordered_items.push(InputItem::Text {
                        text: chunk.to_string(),
                    });
                }
            }


            let placeholder = mat.as_str();
            if placeholder.starts_with("[image:") {
                if let Some(path) = self.pending_images.remove(placeholder) {
                    if path.exists() && path.is_file() {
                        // Emit the placeholder marker verbatim followed by the image so the LLM sees placement
                        ordered_items.push(InputItem::Text {
                            text: placeholder.to_string(),
                        });
                        ordered_items.push(InputItem::LocalImage { path });
                    } else {
                        tracing::warn!(
                            "pending image placeholder {} resolved to missing path {}",
                            placeholder,
                            path.display()
                        );
                        self.push_background_tail(format!(
                            "Dropped image file went missing; not attaching ({})",
                            path.display()
                        ));
                        ordered_items.push(InputItem::Text {
                            text: placeholder.to_string(),
                        });
                    }
                } else {
                    // Unknown placeholder: preserve as text
                    ordered_items.push(InputItem::Text {
                        text: placeholder.to_string(),
                    });
                }
            } else {
                // Unknown placeholder type; preserve
                ordered_items.push(InputItem::Text {
                    text: placeholder.to_string(),
                });
            }
            cursor = mat.end();
        }
        // Push any remaining trailing text
        if cursor < text.len() {
            let chunk = &text[cursor..];
            if !chunk.trim().is_empty() {
                ordered_items.push(InputItem::Text {
                    text: chunk.to_string(),
                });
            }
        }

        // Then check for direct file paths typed into the message (no placeholder).
        // We conservatively append these at the end to avoid mis-ordering text.
        // This keeps the behavior consistent while still including the image.
        // We do NOT strip them from display_text so the user sees what they typed.
        let words: Vec<String> = text.split_whitespace().map(String::from).collect();
        for word in &words {
            if word.starts_with("[image:") {
                continue;
            }
            let is_image_path = IMAGE_EXTENSIONS
                .iter()
                .any(|ext| word.to_lowercase().ends_with(ext));
            if !is_image_path {
                continue;
            }
            let path = Path::new(word);
            if path.exists() {
                // Add a marker then the image so the LLM has contextual placement info
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("image");
                let persisted_path = self
                    .persist_user_image_if_needed(path)
                    .unwrap_or_else(|| path.to_path_buf());
                ordered_items.push(InputItem::Text {
                    text: format!("[image: {filename}]"),
                });
                ordered_items.push(InputItem::LocalImage {
                    path: persisted_path,
                });
            }
        }

        // Non-image paths are left as-is in the text; the model may choose to read them.

        // Preserve user formatting (retain newlines) but normalize whitespace:
        // - Normalize CRLF -> LF
        // - Trim trailing spaces per line
        // - Remove any completely blank lines at the start and end
        display_text = display_text.replace("\r\n", "\n");
        let mut _lines_tmp: Vec<String> = display_text
            .lines()
            .map(|l| l.trim_end().to_string())
            .collect();
        while _lines_tmp.first().is_some_and(|s| s.trim().is_empty()) {
            _lines_tmp.remove(0);
        }
        while _lines_tmp.last().is_some_and(|s| s.trim().is_empty()) {
            _lines_tmp.pop();
        }
        display_text = _lines_tmp.join("\n");

        UserMessage {
            display_text,
            ordered_items,
            suppress_persistence: false,
        }
    }

    /// Periodic tick to commit at most one queued line to history,
    /// animating the output.
    pub(crate) fn on_commit_tick(&mut self) {
        streaming::on_commit_tick(self);
    }
    pub(super) fn is_write_cycle_active(&self) -> bool {
        streaming::is_write_cycle_active(self)
    }

    pub(super) fn flush_interrupt_queue(&mut self) {
        let mut mgr = std::mem::take(&mut self.interrupts);
        mgr.flush_all(self);
        self.interrupts = mgr;
    }

    pub(super) fn on_error(&mut self, message: String) {
        // Treat transient stream errors (which the core will retry) differently
        // from fatal errors so the status spinner remains visible while we wait.
        let lower = message.to_lowercase();
        let is_transient = lower.contains("retrying")
            || lower.contains("reconnecting")
            || lower.contains("disconnected")
            || lower.contains("stream error")
            || lower.contains("stream closed")
            || lower.contains("timeout")
            || lower.contains("temporar")
            || lower.contains("transport")
            || lower.contains("network")
            || lower.contains("connection")
            || lower.contains("failed to start stream");

        if is_transient {
            self.mark_reconnecting(message);
            return;
        }

        // Ensure reconnect banners are cleared once we pivot to a fatal error
        // without emitting the "Reconnected" toast (which would be misleading).
        if self.reconnect_notice_active {
            self.reconnect_notice_active = false;
            self.bottom_pane.update_status_text(String::new());
            self.request_redraw();
        }

        // Error path: show an error cell and clear running state.
        self.clear_resume_placeholder();
        let key = self.next_internal_key();
        let state = history_cell::new_error_event(message.clone());
        let cell = crate::history_cell::PlainHistoryCell::from_state(state.clone());
        let _ = self.history_insert_with_key_global_tagged(
            Box::new(cell),
            key,
            "epilogue",
            Some(HistoryDomainRecord::Plain(state)),
        );
        let should_recover_auto = self.auto_state.is_active();
        self.bottom_pane.set_task_running(false);
        // Ensure any running exec/tool cells are finalized so spinners don't linger
        // after errors.
        self.finalize_all_running_as_interrupted();
        self.stream.clear_all();
        self.stream_state.drop_streaming = false;
        self.agents_ready_to_start = false;
        self.active_task_ids.clear();
        self.maybe_hide_spinner();
        if should_recover_auto {
            self.auto_pause_for_transient_failure(message);
        }
        self.mark_needs_redraw();
    }

    pub(super) fn mark_reconnecting(&mut self, message: String) {
        // Keep task running and surface a concise status in the input header.
        self.bottom_pane.set_task_running(true);
        self.bottom_pane.update_status_text("Retrying...".to_string());

        if !self.reconnect_notice_active {
            self.reconnect_notice_active = true;
            self.push_background_tail(format!("Auto-retrying… ({message})"));
        }

        // Do NOT clear running state or streams; the retry will resume them.
        self.request_redraw();
    }

    pub(super) fn clear_reconnecting(&mut self) {
        if !self.reconnect_notice_active {
            return;
        }
        self.reconnect_notice_active = false;
        self.bottom_pane.update_status_text(String::new());
        self.bottom_pane
            .flash_footer_notice_for("Resuming".to_string(), Duration::from_secs(2));
        self.request_redraw();
    }

    pub(super) fn interrupt_running_task(&mut self) {
        let bottom_running = self.bottom_pane.is_task_running();
        let wait_running = self.wait_running();
        if !self.is_task_running() && !wait_running {
            return;
        }

        // If the user cancels mid-turn while Auto Review is enabled, preserve the
        // captured baseline so a review still runs after the next turn completes.
        if self.config.tui.auto_review_enabled
            && self.pending_auto_review_range.is_none()
            && self.background_review.is_none()
            && let Some(base) = self.auto_review_baseline.take() {
                self.pending_auto_review_range = Some(PendingAutoReviewRange {
                    base,
                    // Defer to the next turn so cancellation doesn’t immediately
                    // trigger auto-review in the same (cancelled) turn.
                    defer_until_turn: Some(self.turn_sequence),
                });
            }

        let mut has_wait_running = false;
        for (call_id, entry) in self.tools_state.running_custom_tools.iter() {
            if let Some(idx) = running_tools::resolve_entry_index(self, entry, &call_id.0)
                && let Some(cell) = self.history_cells.get(idx).and_then(|c| c
                    .as_any()
                    .downcast_ref::<history_cell::RunningToolCallCell>())
                    && cell.has_title("Waiting") {
                        has_wait_running = true;
                        break;
                    }
        }

        self.active_exec_cell = None;
        // Finalize any visible running indicators as interrupted (Exec/Web/Custom)
        self.finalize_all_running_as_interrupted();
        if bottom_running {
            self.bottom_pane.clear_ctrl_c_quit_hint();
        }
        // Stop any active UI streams immediately so output ceases at once.
        self.finalize_active_stream();
        self.stream_state.drop_streaming = true;
        // Surface an explicit notice in history so users see confirmation.
        if !has_wait_running {
            self.push_background_tail("Cancelled by user.".to_string());
        }
        self.submit_op(Op::Interrupt);
        // Immediately drop the running status so the next message can be typed/run,
        // even if backend cleanup (and Error event) arrives slightly later.
        self.bottom_pane.set_task_running(false);
        self.bottom_pane.clear_live_ring();
        // Reset with max width to disable wrapping
        self.live_builder = RowBuilder::new(usize::MAX);
        // Stream state is now managed by StreamController
        self.content_buffer.clear();
        // Defensive: clear transient flags so UI can quiesce
        self.agents_ready_to_start = false;
        self.active_task_ids.clear();
        // Restore any queued messages back into the composer so the user can
        // immediately press Enter to resume the conversation where they left off.
        if !self.queued_user_messages.is_empty() {
            let existing_input = self.bottom_pane.composer_text();
            let mut segments: Vec<String> = Vec::new();

            let mut queued_block = String::new();
            for (i, qm) in self.queued_user_messages.iter().enumerate() {
                if i > 0 {
                    queued_block.push_str("\n\n");
                }
                queued_block.push_str(qm.display_text.trim_end());
            }
            if !queued_block.trim().is_empty() {
                segments.push(queued_block);
            }

            if !existing_input.trim().is_empty() {
                segments.push(existing_input);
            }

            let combined = segments.join("\n\n");
            self.clear_composer();
            if !combined.is_empty() {
                self.insert_str(&combined);
            }
            self.queued_user_messages.clear();
            self.bottom_pane.update_status_text(String::new());
            self.pending_dispatched_user_messages.clear();
            self.refresh_queued_user_messages(false);
        }
        self.maybe_hide_spinner();
        self.request_redraw();
    }
}
