use super::*;

impl ChatWidget<'_> {
    /// Render a single recorded ResponseItem into history without executing tools
    pub(in super::super) fn render_replay_item(&mut self, item: ResponseItem) {
        match item {
            ResponseItem::Message { id, role, content, .. } => {
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
                let mut content = output.body.to_text().unwrap_or_default();
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

    pub(in super::super) fn is_auto_review_cell(item: &dyn HistoryCell) -> bool {
        item.as_any()
            .downcast_ref::<crate::history_cell::PlainHistoryCell>()
            .map(crate::history_cell::PlainHistoryCell::is_auto_review_notice)
            .unwrap_or(false)
    }

    pub(in super::super) fn render_cached_lines(
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
    pub(in super::super) fn autoscroll_if_near_bottom(&mut self) {
        layout_scroll::autoscroll_if_near_bottom(self);
    }

    pub(in super::super) fn clear_reasoning_in_progress(&mut self) {
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

    #[cfg(debug_assertions)]
    pub(in super::super) fn reasoning_preview(lines: &[Line<'static>]) -> String {
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

    pub(in super::super) fn refresh_reasoning_collapsed_visibility(&mut self) {
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
}
