use super::*;

impl ChatWidget<'_> {
    pub(super) fn context_ui_enabled(&self) -> bool {
        self.config.env_ctx_v2
    }

    pub(super) fn set_context_summary(
        &mut self,
        mut summary: ContextSummary,
        sequence: Option<u64>,
        is_baseline: bool,
    ) {
        if let Some(seq) = sequence {
            if let Some(prev_seq) = self.context_last_sequence
                && seq < prev_seq {
                    return;
                }
            self.context_last_sequence = Some(seq);
        }

        let previous = self.context_summary.clone();

        if let Some(prev) = previous.as_ref() {
            summary.expanded = prev.expanded;
        }

        if is_baseline {
            summary.deltas.clear();
        } else if let Some(prev) = previous.as_ref() {
            summary.deltas = prev.deltas.clone();
            for delta in Self::compute_context_deltas(prev, &summary, sequence) {
                Self::push_context_delta(&mut summary.deltas, delta);
            }
        }

        if let Some(current_seq) = self.context_last_sequence {
            if let Some(browser_seq) = self.context_browser_sequence {
                if browser_seq < current_seq {
                    summary.browser_session_active = false;
                }
            } else if summary.browser_snapshot.is_none() {
                summary.browser_session_active = false;
            }
        }

        self.context_summary = Some(summary.clone());
        self.update_context_cell(summary);
    }

    pub(super) fn strict_stream_ids_enabled(&self) -> bool {
        self.config.env_ctx_v2 && (self.test_mode || cfg!(debug_assertions))
    }

    pub(super) fn warn_missing_stream_id(&mut self, stream_kind: &str) {
        tracing::warn!("missing stream id for {stream_kind}");
        if cfg!(debug_assertions) || self.test_mode {
            let warning = format!("Missing stream id for {stream_kind}");
            self.push_background_tail(warning);
        }
    }

    pub(crate) fn toggle_context_expansion(&mut self) {
        if !self.context_ui_enabled() {
            self.bottom_pane
                .update_status_text("Context UI disabled".to_string());
            return;
        }

        let Some(mut summary) = self.context_summary.clone() else {
            self.bottom_pane
                .update_status_text("No context available yet".to_string());
            return;
        };

        summary.expanded = !summary.expanded;
        let expanded = summary.expanded;
        self.context_summary = Some(summary.clone());
        self.update_context_cell(summary);
        self.invalidate_height_cache();
        self.request_redraw();

        let status = if expanded {
            "Context expanded"
        } else {
            "Context collapsed"
        };
        self.bottom_pane.update_status_text(status.to_string());

        if self.standard_terminal_mode {
            let mut lines = Vec::new();
            lines.push(ratatui::text::Line::from(""));
            lines.extend(self.export_transcript_lines_for_buffer());
            self.app_event_tx
                .send(crate::app_event::AppEvent::InsertHistory(lines));
        }
    }

    pub(super) fn update_context_cell(&mut self, summary: ContextSummary) {
        let record = ContextRecord {
            id: HistoryId::ZERO,
            cwd: summary.cwd.clone(),
            git_branch: summary.git_branch.clone(),
            reasoning_effort: summary.reasoning_effort.clone(),
            browser_session_active: summary.browser_session_active,
            deltas: summary.deltas.clone(),
            browser_snapshot: summary.browser_snapshot.clone(),
            expanded: summary.expanded,
        };

        if let Some(id) = self.context_cell_id
            && let Some(index) = self.history_state.index_of(id) {
                let mutation = self.history_state.apply_domain_event(HistoryDomainEvent::Replace {
                    index,
                    record: HistoryDomainRecord::Context(record),
                });
                if let Some(cell_idx) = self.cell_index_for_history_id(id)
                    && let Some(new_id) = self.apply_mutation_to_cell_index(cell_idx, mutation) {
                        self.context_cell_id = Some(new_id);
                    }
                self.mark_history_dirty();
                self.request_redraw();
                return;
            }

        let insertion = self.history_state.apply_domain_event(HistoryDomainEvent::Insert {
            index: 0,
            record: HistoryDomainRecord::Context(record),
        });

        if let HistoryMutation::Inserted { id, record, .. } = insertion
            && let Some(mut cell) = self.build_cell_from_record(&record) {
                self.assign_history_id(&mut cell, id);
                let idx = self.history_insert_existing_record(
                    cell,
                    Self::context_order_key(),
                    "context",
                    id,
                );
                if idx < self.history_cell_ids.len() {
                    self.history_cell_ids[idx] = Some(id);
                } else {
                    self.history_cell_ids.push(Some(id));
                }
                self.context_cell_id = Some(id);
                self.mark_history_dirty();
                self.request_redraw();
            }
    }

    pub(super) fn compute_context_deltas(
        previous: &ContextSummary,
        current: &ContextSummary,
        sequence: Option<u64>,
    ) -> Vec<ContextDeltaRecord> {
        let mut deltas = Vec::new();

        if previous.cwd != current.cwd {
            deltas.push(ContextDeltaRecord {
                field: ContextDeltaField::Cwd,
                previous: previous.cwd.clone(),
                current: current.cwd.clone(),
                sequence,
            });
        }

        if previous.git_branch != current.git_branch {
            deltas.push(ContextDeltaRecord {
                field: ContextDeltaField::GitBranch,
                previous: previous.git_branch.clone(),
                current: current.git_branch.clone(),
                sequence,
            });
        }

        if previous.reasoning_effort != current.reasoning_effort {
            deltas.push(ContextDeltaRecord {
                field: ContextDeltaField::ReasoningEffort,
                previous: previous.reasoning_effort.clone(),
                current: current.reasoning_effort.clone(),
                sequence,
            });
        }

        if previous.browser_snapshot != current.browser_snapshot {
            let prev_label = previous
                .browser_snapshot
                .as_ref()
                .and_then(Self::context_snapshot_label);
            let next_label = current
                .browser_snapshot
                .as_ref()
                .and_then(Self::context_snapshot_label);
            deltas.push(ContextDeltaRecord {
                field: ContextDeltaField::BrowserSnapshot,
                previous: prev_label,
                current: next_label,
                sequence,
            });
        }

        deltas
    }

    pub(super) fn push_context_delta(deltas: &mut Vec<ContextDeltaRecord>, mut delta: ContextDeltaRecord) {
        if delta.previous == delta.current {
            return;
        }

        if let Some(last) = deltas.last()
            && last.field == delta.field && last.current == delta.current {
                return;
            }

        if deltas.len() >= CONTEXT_DELTA_HISTORY {
            let excess = deltas.len() + 1 - CONTEXT_DELTA_HISTORY;
            deltas.drain(0..excess);
        }

        if delta.field == ContextDeltaField::BrowserSnapshot
            && delta.current.is_none()
            && delta.previous.is_some()
        {
            delta.current = Some("inactive".to_string());
        }

        deltas.push(delta);
    }

    pub(super) fn context_snapshot_label(snapshot: &ContextBrowserSnapshotRecord) -> Option<String> {
        if let Some(title) = snapshot.title.as_ref().filter(|s| !s.is_empty()) {
            Some(title.clone())
        } else {
            snapshot.url.clone()
        }
    }

    pub(super) fn handle_environment_context_full_event(
        &mut self,
        payload: &EnvironmentContextFullEvent,
    ) {
        if !self.context_ui_enabled() {
            return;
        }

        let mut summary = ContextSummary::default();
        if let Some(obj) = payload.snapshot.as_object() {
            if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
                summary.cwd = Some(cwd.to_string());
            }
            if let Some(branch) = obj.get("git_branch").and_then(|v| v.as_str()) {
                summary.git_branch = Some(branch.to_string());
            }
            if let Some(reason) = obj.get("reasoning_effort").and_then(|v| v.as_str()) {
                summary.reasoning_effort = Some(reason.to_string());
            }
        }

        summary.browser_session_active = false;
        self.context_browser_sequence = None;
        summary.deltas.clear();
        summary.browser_snapshot = None;
        self.set_context_summary(summary, payload.sequence, true);
    }

    pub(super) fn handle_environment_context_delta_event(
        &mut self,
        payload: &EnvironmentContextDeltaEvent,
    ) {
        if !self.context_ui_enabled() {
            return;
        }

        let mut summary = self.context_summary.clone().unwrap_or_default();
        if let Some(obj) = payload.delta.as_object()
            && let Some(changes) = obj.get("changes").and_then(|v| v.as_object()) {
                self.apply_context_changes(&mut summary, changes);
            }

        self.set_context_summary(summary, payload.sequence, false);
    }

    pub(super) fn handle_browser_snapshot_event(&mut self, payload: &BrowserSnapshotEvent) {
        if !self.context_ui_enabled() {
            return;
        }

        let mut summary = self.context_summary.clone().unwrap_or_default();
        summary.browser_session_active = true;
        summary.browser_snapshot = Some(Self::browser_snapshot_from_event(payload));
        self.context_browser_sequence = self.context_last_sequence;
        self.set_context_summary(summary, None, false);
    }

    pub(super) fn apply_context_changes(
        &self,
        summary: &mut ContextSummary,
        changes: &serde_json::Map<String, serde_json::Value>,
    ) {
        if let Some(value) = changes.get("cwd") {
            summary.cwd = Self::value_to_optional_string(value);
        }
        if let Some(value) = changes.get("git_branch") {
            summary.git_branch = Self::value_to_optional_string(value);
        }
        if let Some(value) = changes.get("reasoning_effort") {
            summary.reasoning_effort = Self::value_to_optional_string(value);
        }
    }

    pub(super) fn value_to_optional_string(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::Null => None,
            serde_json::Value::String(s) => Some(s.clone()),
            other => Some(other.to_string()),
        }
    }

    pub(super) fn browser_snapshot_from_event(payload: &BrowserSnapshotEvent) -> ContextBrowserSnapshotRecord {
        use std::collections::BTreeMap;

        let mut record = ContextBrowserSnapshotRecord::default();

        if let Some(obj) = payload.snapshot.as_object() {
            if let Some(version_url) = obj.get("url").and_then(|v| v.as_str()) {
                record.url = Some(version_url.to_string());
            }
            if let Some(title) = obj.get("title").and_then(|v| v.as_str()) {
                record.title = Some(title.to_string());
            }
            if let Some(captured) = obj.get("captured_at").and_then(|v| v.as_str()) {
                record.captured_at = Some(captured.to_string());
            }
            if let Some(viewport) = obj.get("viewport").and_then(|v| v.as_object()) {
                record.width = viewport
                    .get("width")
                    .and_then(serde_json::Value::as_u64)
                    .map(|v| v as u32);
                record.height = viewport
                    .get("height")
                    .and_then(serde_json::Value::as_u64)
                    .map(|v| v as u32);
            }
            if let Some(meta) = obj.get("metadata").and_then(|v| v.as_object()) {
                let mut map = BTreeMap::new();
                for (key, value) in meta.iter() {
                    if let Some(v) = value.as_str() {
                        map.insert(key.clone(), v.to_string());
                    } else {
                        map.insert(key.clone(), value.to_string());
                    }
                }
                if !map.is_empty() {
                    record.metadata = map;
                }
            }
        }

        if record.url.is_none() {
            record.url = payload.url.clone();
        }

        if record.captured_at.is_none() {
            record.captured_at = payload.captured_at.clone();
        }

        record
    }
}
