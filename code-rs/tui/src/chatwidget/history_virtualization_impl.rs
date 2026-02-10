use super::*;

impl ChatWidget<'_> {
    pub(super) fn invalidate_height_cache(&mut self) {
        self.history_render.invalidate_height_cache();
        self.mark_render_requests_dirty();
        self.history_prefix_append_only.set(false);
        self.history_virtualization_sync_pending.set(true);
    }

    pub(super) fn mark_render_requests_dirty(&self) {
        self.render_request_cache_dirty.set(true);
    }

    pub(super) fn update_welcome_height_hint(&self, height: u16) {
        for cell in self.history_cells.iter() {
            if let Some(welcome) = cell
                .as_any()
                .downcast_ref::<crate::history_cell::AnimatedWelcomeCell>()
            {
                welcome.set_available_height(height);
            }
        }
    }

    pub(super) fn is_frozen_cell(cell: &dyn HistoryCell) -> bool {
        cell.as_any().downcast_ref::<FrozenHistoryCell>().is_some()
    }

    pub(super) fn history_record_for_index(&self, idx: usize) -> Option<&HistoryRecord> {
        if let Some(record) = self
            .history_cell_ids
            .get(idx)
            .and_then(|entry| entry.map(|id| self.history_state.record(id)))
            .flatten()
        {
            return Some(record);
        }

        self.history_cells
            .get(idx)
            .and_then(|cell| cell.as_any().downcast_ref::<FrozenHistoryCell>())
            .and_then(|frozen| self.history_state.record(frozen.history_id()))
    }

    pub(super) fn record_from_cell_or_state(&self, idx: usize, cell: &dyn HistoryCell) -> Option<HistoryRecord> {
        history_cell::record_from_cell(cell)
            .or_else(|| self.history_record_for_index(idx).cloned())
    }

    pub(super) fn render_request_seed_for_cell(&self, idx: usize, cell: &dyn HistoryCell) -> RenderRequestSeed {
        let (history_id, has_record) = if let Some(Some(id)) = self.history_cell_ids.get(idx) {
            let exists = self.history_state.index_of(*id).is_some();
            (*id, exists)
        } else {
            (HistoryId::ZERO, false)
        };

        let cell_has_custom_render = cell.has_custom_render();
        let is_streaming = cell
            .as_any()
            .downcast_ref::<crate::history_cell::StreamingContentCell>()
            .is_some();

        let mut use_cache = history_id != HistoryId::ZERO
            && has_record
            && !cell_has_custom_render
            && !cell.is_animating()
            && !is_streaming;

        let is_frozen = Self::is_frozen_cell(cell);
        let mut kind = RenderRequestKind::Legacy;
        if history_id != HistoryId::ZERO && !is_frozen
            && let Some(record) = self.history_state.record(history_id) {
                match record {
                    HistoryRecord::Exec(_) => {
                        kind = RenderRequestKind::Exec { id: history_id };
                    }
                    HistoryRecord::MergedExec(_) => {
                        kind = RenderRequestKind::MergedExec { id: history_id };
                    }
                    HistoryRecord::Explore(_) => {
                        let hold_header = self.rendered_explore_should_hold(idx);
                        kind = RenderRequestKind::Explore {
                            id: history_id,
                            hold_header,
                            full_detail: self.is_reasoning_shown(),
                        };
                    }
                    HistoryRecord::Diff(_) => {
                        kind = RenderRequestKind::Diff { id: history_id };
                    }
                    HistoryRecord::AssistantStream(stream_state) => {
                        kind = RenderRequestKind::Streaming { id: history_id };
                        if stream_state.in_progress {
                            use_cache = false;
                        }
                    }
                    HistoryRecord::AssistantMessage(_) => {
                        kind = RenderRequestKind::Assistant { id: history_id };
                    }
                    _ => {}
                }
            }

        let mut fallback_lines: Option<Rc<Vec<Line<'static>>>> = None;
        if !cell_has_custom_render && !is_streaming {
            if history_id != HistoryId::ZERO {
                if let Some(record) = self.history_state.record(history_id)
                    && let Some(lines) = self.fallback_lines_for_record(cell, record) {
                        let cached = self.history_render.cached_fallback_lines(history_id, || lines);
                        fallback_lines = Some(cached);
                    }
            } else {
                let lines = cell.display_lines_trimmed();
                if !lines.is_empty() {
                    fallback_lines = Some(Rc::new(lines));
                }
            }
        }

        RenderRequestSeed {
            history_id,
            use_cache,
            fallback_lines,
            kind,
        }
    }

    pub(super) fn update_render_request_seed(&self, idx: usize) {
        let cache_len = self.render_request_cache.borrow().len();
        if cache_len != self.history_cells.len() {
            self.render_request_cache_dirty.set(true);
            return;
        }

        if let Some(cell) = self.history_cells.get(idx) {
            let seed = self.render_request_seed_for_cell(idx, cell.as_ref());
            self.render_request_cache.borrow_mut()[idx] = seed;
        }
    }

    pub(super) fn rebuild_render_request_cache(&self) {
        let mut cache = self.render_request_cache.borrow_mut();
        cache.clear();
        cache.reserve(self.history_cells.len());

        for (idx, cell) in self.history_cells.iter().enumerate() {
            let seed = self.render_request_seed_for_cell(idx, cell.as_ref());
            cache.push(seed);
        }

        self.render_request_cache_dirty.set(false);
    }

    pub(super) fn ensure_render_request_cache(&self) {
        if self.render_request_cache_dirty.get()
            || self.render_request_cache.borrow().len() != self.history_cells.len()
        {
            self.rebuild_render_request_cache();
        }
    }

    pub(super) fn fallback_lines_for_record(
        &self,
        cell: &dyn HistoryCell,
        record: &HistoryRecord,
    ) -> Option<Vec<Line<'static>>> {
        if cell.has_custom_render() {
            return None;
        }

        let lines = cell.display_lines_trimmed();
        if !lines.is_empty() || matches!(record, HistoryRecord::Reasoning(_)) {
            Some(lines)
        } else {
            Some(history_cell::lines_from_record(record, &self.config))
        }
    }

    pub(super) fn cell_lines_for_index(&self, idx: usize, cell: &dyn HistoryCell) -> Vec<Line<'static>> {
        if Self::is_frozen_cell(cell)
            && let Some(record) = self.history_record_for_index(idx) {
                return history_cell::lines_from_record(record, &self.config);
            }
        cell.display_lines()
    }

    pub(super) fn cell_lines_trimmed_is_empty(&self, idx: usize, cell: &dyn HistoryCell) -> bool {
        if Self::is_frozen_cell(cell)
            && let Some(record) = self.history_record_for_index(idx) {
                return history_cell::lines_from_record(record, &self.config).is_empty();
            }
        cell.display_lines_trimmed().is_empty()
    }

    pub(super) fn cell_lines_for_terminal_index(
        &self,
        idx: usize,
        cell: &dyn HistoryCell,
    ) -> Vec<Line<'static>> {
        if Self::is_frozen_cell(cell)
            && let Some(record) = self.history_record_for_index(idx) {
                return history_cell::cell_from_record(record, &self.config).display_lines();
            }
        cell.display_lines()
    }

    pub(super) fn freeze_eligible_record(&self, record: &HistoryRecord) -> bool {
        match record {
            HistoryRecord::PlainMessage(_)
            | HistoryRecord::AssistantMessage(_)
            | HistoryRecord::BackgroundEvent(_)
            | HistoryRecord::Notice(_)
            | HistoryRecord::Diff(_)
            | HistoryRecord::Patch(_)
            | HistoryRecord::Image(_)
            | HistoryRecord::UpgradeNotice(_)
            | HistoryRecord::RateLimits(_) => true,
            HistoryRecord::Exec(state) => !matches!(state.status, ExecStatus::Running),
            HistoryRecord::MergedExec(_) => true,
            HistoryRecord::ToolCall(state) => state.status != ToolStatus::Running,
            HistoryRecord::AssistantStream(state) => !state.in_progress,
            _ => false,
        }
    }

    pub(super) fn freeze_history_cell_at(&mut self, idx: usize, render_settings: RenderSettings) -> bool {
        if idx >= self.history_cells.len() {
            return false;
        }
        let Some(Some(history_id)) = self.history_cell_ids.get(idx) else {
            return false;
        };
        let Some(record) = self.history_state.record(*history_id) else {
            return false;
        };
        if !self.freeze_eligible_record(record) {
            return false;
        }
        let cell = &self.history_cells[idx];
        if Self::is_frozen_cell(cell.as_ref()) || cell.is_animating() {
            return false;
        }

        let cached_height = self
            .history_render
            .cached_height(*history_id, render_settings)
            .unwrap_or_else(|| cell.desired_height(render_settings.width));
        let frozen = FrozenHistoryCell::new(
            *history_id,
            cell.kind(),
            render_settings.width,
            cached_height,
        );
        self.history_cells[idx] = Box::new(frozen);
        self.history_frozen_count = self.history_frozen_count.saturating_add(1);
        self.update_render_request_seed(idx);
        true
    }

    pub(super) fn thaw_history_cell_at(&mut self, idx: usize) -> bool {
        if idx >= self.history_cells.len() {
            return false;
        }
        let Some(frozen) = self.history_cells[idx]
            .as_any()
            .downcast_ref::<FrozenHistoryCell>()
        else {
            return false;
        };

        let history_id = frozen.history_id();
        let Some(record) = self.history_state.record(history_id).cloned() else {
            return false;
        };
        let mut cell = match self.build_cell_from_record(&record) {
            Some(cell) => cell,
            None => return false,
        };
        Self::assign_history_id_inner(&mut cell, history_id);
        self.history_cells[idx] = cell;
        if idx < self.history_cell_ids.len() {
            self.history_cell_ids[idx] = Some(history_id);
        }
        self.history_frozen_count = self.history_frozen_count.saturating_sub(1);
        self.update_render_request_seed(idx);
        true
    }

    pub(super) fn refresh_frozen_heights(&mut self, render_settings: RenderSettings) {
        let width = render_settings.width;
        if self.history_frozen_width == width {
            return;
        }

        for idx in 0..self.history_cells.len() {
            let (history_id, cached_width, cached_height) = match self.history_cells[idx]
                .as_any()
                .downcast_ref::<FrozenHistoryCell>()
            {
                Some(frozen) => (frozen.history_id(), frozen.cached_width(), frozen.cached_height()),
                None => continue,
            };

            if cached_width == width {
                continue;
            }

            let height = self
                .history_render
                .cached_height(history_id, render_settings)
                .or_else(|| {
                    self.history_state.record(history_id).cloned().and_then(|record| {
                        self.build_cell_from_record(&record)
                            .map(|cell| cell.desired_height(width))
                    })
                })
                .unwrap_or(cached_height);

            if let Some(frozen) = self.history_cells[idx]
                .as_any_mut()
                .downcast_mut::<FrozenHistoryCell>()
            {
                frozen.update_cached_height(width, height);
            }
        }

        self.history_frozen_width = width;
    }

    pub(super) fn thaw_range(&mut self, start: usize, end: usize) -> bool {
        let mut changed = false;
        let upper = end.min(self.history_cells.len());
        for idx in start.min(self.history_cells.len())..upper {
            if self.thaw_history_cell_at(idx) {
                changed = true;
            }
        }
        changed
    }

    pub(super) fn freeze_range(&mut self, start: usize, end: usize, render_settings: RenderSettings) -> bool {
        let mut changed = false;
        let upper = end.min(self.history_cells.len());
        for idx in start.min(self.history_cells.len())..upper {
            if self.freeze_history_cell_at(idx, render_settings) {
                changed = true;
            }
        }
        changed
    }

    pub(super) fn update_history_live_window(
        &mut self,
        scroll_pos: u16,
        viewport_rows: u16,
        total_height: u16,
        render_settings: RenderSettings,
    ) -> bool {
        if self.history_cells.is_empty() || viewport_rows == 0 {
            self.history_live_window = None;
            return false;
        }
        let history_len = self.history_cells.len();
        let new_range = if scroll_pos >= total_height {
            // Tail-only content fills the viewport; keep all history frozen.
            (history_len, history_len)
        } else {
            let live_margin = viewport_rows / 2;
            let live_start = scroll_pos.saturating_sub(live_margin);
            let live_end = scroll_pos
                .saturating_add(viewport_rows)
                .saturating_add(live_margin)
                .min(total_height);

            let (mut start_idx, mut end_idx) = {
                let ps_ref = self.history_render.prefix_sums.borrow();
                let ps: &Vec<u16> = &ps_ref;
                if ps.len() <= 1 {
                    return false;
                }

                let start_idx = match ps.binary_search(&live_start) {
                    Ok(i) => i,
                    Err(i) => i.saturating_sub(1),
                };
                let end_idx = match ps.binary_search(&live_end) {
                    Ok(i) => i,
                    Err(i) => i,
                };

                (start_idx, end_idx)
            };

            start_idx = start_idx.min(history_len);
            end_idx = end_idx.min(history_len);
            if start_idx > end_idx {
                start_idx = end_idx;
            }

            (start_idx, end_idx)
        };
        if self.history_live_window == Some(new_range) {
            return false;
        }

        let mut changed = false;
        if let Some((prev_start, prev_end)) = self.history_live_window {
            if new_range.0 < prev_start {
                changed |= self.thaw_range(new_range.0, prev_start);
            }
            if prev_end < new_range.1 {
                changed |= self.thaw_range(prev_end, new_range.1);
            }
            if prev_start < new_range.0 {
                changed |= self.freeze_range(prev_start, new_range.0, render_settings);
            }
            if new_range.1 < prev_end {
                changed |= self.freeze_range(new_range.1, prev_end, render_settings);
            }
        } else {
            changed |= self.thaw_range(new_range.0, new_range.1);
            changed |= self.freeze_range(0, new_range.0, render_settings);
            changed |= self.freeze_range(new_range.1, history_len, render_settings);
        }

        self.history_live_window = Some(new_range);
        changed
    }

    pub(crate) fn sync_history_virtualization(&mut self) {
        self.history_virtualization_sync_pending.set(false);
        self.ensure_render_request_cache();
        let render_settings = self.last_render_settings.get();
        if render_settings.width == 0 {
            self.history_virtualization_sync_pending.set(true);
            return;
        }

        if self.history_cells.is_empty() {
            self.history_live_window = None;
            return;
        }

        self.refresh_frozen_heights(render_settings);

        let viewport_rows = self.layout.last_history_viewport_height.get();
        if viewport_rows == 0 {
            self.history_virtualization_sync_pending.set(true);
            return;
        }
        let total_height = self.history_render.last_total_height();
        let max_scroll = total_height.saturating_sub(viewport_rows);
        let clamped_offset = self.layout.scroll_offset.get().min(max_scroll);
        let scroll_pos = max_scroll.saturating_sub(clamped_offset);
        let history_len = self.history_cells.len();
        let history_total = {
            let ps_ref = self.history_render.prefix_sums.borrow();
            if ps_ref.len() <= history_len {
                self.history_virtualization_sync_pending.set(true);
                return;
            }
            ps_ref[history_len]
        };
        self.update_history_live_window(
            scroll_pos,
            viewport_rows,
            history_total,
            render_settings,
        );
    }
}
