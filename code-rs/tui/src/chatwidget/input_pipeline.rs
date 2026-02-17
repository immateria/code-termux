use super::*;
use code_protocol::num_format::format_with_separators_u64;

type NumstatRow = (Option<u32>, Option<u32>, String);

impl ChatWidget<'_> {
    pub(crate) fn handle_key_event(&mut self, key_event: KeyEvent) {
        if settings_handlers::handle_settings_key(self, key_event) {
            self.sync_limits_layout_mode_preference();
            return;
        }
        if self.settings.overlay.is_some() {
            return;
        }
        if terminal_handlers::handle_terminal_key(self, key_event) {
            return;
        }
        if self.terminal.overlay.is_some() {
            // Block background input while the terminal overlay is visible.
            return;
        }
        // Intercept keys for overlays when active (help first, then diff)
        if help_handlers::handle_help_key(self, key_event) {
            return;
        }
        if self.help.overlay.is_some() {
            return;
        }
        if diff_handlers::handle_diff_key(self, key_event) {
            return;
        }
        if self.diffs.overlay.is_some() {
            return;
        }
        if self.browser_overlay_visible {
            let is_ctrl_b = matches!(
                key_event,
                KeyEvent {
                    code: crossterm::event::KeyCode::Char('b'),
                    modifiers: crossterm::event::KeyModifiers::CONTROL,
                    kind: KeyEventKind::Press | KeyEventKind::Repeat,
                    ..
                }
            );
            if is_ctrl_b {
                self.toggle_browser_overlay();
                return;
            }
            if self.handle_browser_overlay_key(key_event) {
                return;
            }
        }
        if key_event.kind == KeyEventKind::Press {
            self.bottom_pane.clear_ctrl_c_quit_hint();
        }

        if self.auto_state.awaiting_coordinator_submit()
            && !self.auto_state.is_paused_manual()
            && matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat)
        {
            match key_event.code {
                crossterm::event::KeyCode::Enter
                | crossterm::event::KeyCode::Char(' ') if key_event.modifiers.is_empty() => {
                    if !self.auto_state.should_bypass_coordinator_next_submit() {
                        self.auto_submit_prompt();
                    }
                    return;
                }
                crossterm::event::KeyCode::Char('e') | crossterm::event::KeyCode::Char('E')
                    if key_event.modifiers.is_empty() =>
                {
                    self.auto_pause_for_manual_edit(false);
                    return;
                }
                _ => {}
            }
        }

        // Global overlays (avoid conflicting with common editor keys):
        // - Ctrl+B: toggle Browser overlay
        // - Ctrl+A: toggle Agents terminal mode
        if let KeyEvent {
            code: crossterm::event::KeyCode::Char('b'),
            modifiers: crossterm::event::KeyModifiers::CONTROL,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        } = key_event
        {
            self.toggle_browser_overlay();
            return;
        }
        if let KeyEvent {
            code: crossterm::event::KeyCode::Char('a'),
            modifiers: crossterm::event::KeyModifiers::CONTROL,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        } = key_event
        {
            self.toggle_agents_hud();
            return;
        }
        if self.agents_terminal.active {
            use crossterm::event::KeyCode;
            if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                return;
            }

            if let Some(pending) = self.agents_terminal.pending_stop.clone() {
                match key_event.code {
                    KeyCode::Esc | KeyCode::Char('n') => {
                        self.agents_terminal.clear_stop_prompt();
                        self.request_redraw();
                    }
                    KeyCode::Enter | KeyCode::Char('y') => {
                        self.cancel_agent_by_id(&pending.agent_id);
                        self.agents_terminal.clear_stop_prompt();
                    }
                    _ => {}
                }
                return;
            }
            match key_event.code {
                KeyCode::Esc => {
                    if self.agents_terminal.focus() == AgentsTerminalFocus::Detail {
                        self.agents_terminal.focus_sidebar();
                        self.request_redraw();
                    } else {
                        self.exit_agents_terminal_mode();
                    }
                    return;
                }
                KeyCode::Right | KeyCode::Enter => {
                    if self.agents_terminal.focus() == AgentsTerminalFocus::Sidebar {
                        self.agents_terminal.focus_detail();
                        self.request_redraw();
                    }
                    return;
                }
                KeyCode::Left => {
                    if self.agents_terminal.focus() == AgentsTerminalFocus::Detail {
                        self.agents_terminal.focus_sidebar();
                        self.request_redraw();
                    }
                    return;
                }
                KeyCode::Tab => {
                    match self.agents_terminal.focus() {
                        AgentsTerminalFocus::Sidebar => self.agents_terminal.focus_detail(),
                        AgentsTerminalFocus::Detail => self.agents_terminal.focus_sidebar(),
                    }
                    self.request_redraw();
                    return;
                }
                KeyCode::BackTab => {
                    match self.agents_terminal.focus() {
                        AgentsTerminalFocus::Sidebar => self.agents_terminal.focus_detail(),
                        AgentsTerminalFocus::Detail => self.agents_terminal.focus_sidebar(),
                    }
                    self.request_redraw();
                    return;
                }
                KeyCode::Char('1') => {
                    self.agents_terminal.set_tab(AgentsTerminalTab::All);
                    self.request_redraw();
                    return;
                }
                KeyCode::Char('2') => {
                    self.agents_terminal.set_tab(AgentsTerminalTab::Running);
                    self.request_redraw();
                    return;
                }
                KeyCode::Char('3') => {
                    self.agents_terminal.set_tab(AgentsTerminalTab::Failed);
                    self.request_redraw();
                    return;
                }
                KeyCode::Char('4') => {
                    self.agents_terminal.set_tab(AgentsTerminalTab::Completed);
                    self.request_redraw();
                    return;
                }
                KeyCode::Char('5') => {
                    self.agents_terminal.set_tab(AgentsTerminalTab::Review);
                    self.request_redraw();
                    return;
                }
                KeyCode::Char('[') => {
                    self.agents_terminal.jump_batch(-1);
                    self.request_redraw();
                    return;
                }
                KeyCode::Char(']') => {
                    self.agents_terminal.jump_batch(1);
                    self.request_redraw();
                    return;
                }
                KeyCode::Char('s') => {
                    let current = self.agents_terminal.current_sidebar_entry();
                    self.agents_terminal.cycle_sort_mode();
                    self.agents_terminal.reselect_entry(current);
                    self.request_redraw();
                    return;
                }
                KeyCode::Char('S') => {
                    self.prompt_stop_selected_agent();
                    return;
                }
                KeyCode::Char('h') => {
                    self.agents_terminal.toggle_highlights();
                    self.request_redraw();
                    return;
                }
                KeyCode::Char('a') => {
                    self.agents_terminal.toggle_actions();
                    self.request_redraw();
                    return;
                }
                KeyCode::Up => {
                    if self.agents_terminal.focus() == AgentsTerminalFocus::Detail {
                        self.sync_agents_terminal_scroll();
                        layout_scroll::line_up(self);
                        self.record_current_agent_scroll();
                    } else {
                        self.navigate_agents_terminal_selection(-1);
                    }
                    return;
                }
                KeyCode::Down => {
                    if self.agents_terminal.focus() == AgentsTerminalFocus::Detail {
                        self.sync_agents_terminal_scroll();
                        layout_scroll::line_down(self);
                        self.record_current_agent_scroll();
                    } else {
                        self.navigate_agents_terminal_selection(1);
                    }
                    return;
                }
                KeyCode::PageUp => {
                    if self.agents_terminal.focus() == AgentsTerminalFocus::Detail {
                        self.sync_agents_terminal_scroll();
                        layout_scroll::page_up(self);
                        self.record_current_agent_scroll();
                    } else {
                        self.navigate_agents_terminal_page(-1);
                    }
                    return;
                }
                KeyCode::PageDown => {
                    if self.agents_terminal.focus() == AgentsTerminalFocus::Detail {
                        self.sync_agents_terminal_scroll();
                        layout_scroll::page_down(self);
                        self.record_current_agent_scroll();
                    } else {
                        self.navigate_agents_terminal_page(1);
                    }
                    return;
                }
                KeyCode::Home => {
                    if self.agents_terminal.focus() == AgentsTerminalFocus::Detail {
                        self.sync_agents_terminal_scroll();
                        layout_scroll::to_top(self);
                        self.record_current_agent_scroll();
                    } else {
                        self.navigate_agents_terminal_home();
                    }
                    return;
                }
                KeyCode::End => {
                    if self.agents_terminal.focus() == AgentsTerminalFocus::Detail {
                        self.sync_agents_terminal_scroll();
                        layout_scroll::to_bottom(self);
                        self.record_current_agent_scroll();
                    } else {
                        self.navigate_agents_terminal_end();
                    }
                    return;
                }
                _ => {
                    return;
                }
            }
        }

        if let KeyEvent {
            code: crossterm::event::KeyCode::Char('g'),
            modifiers: crossterm::event::KeyModifiers::CONTROL,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        } = key_event
        {
            if !self.bottom_pane.has_active_modal_view() {
                let initial = self.bottom_pane.composer_text();
                self.app_event_tx
                    .send(AppEvent::OpenExternalEditor { initial });
            }
            return;
        }

        // Fast-path PageUp/PageDown to scroll the transcript by a viewport at a time.
        if let crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::PageUp,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        } = key_event
        {
            layout_scroll::page_up(self);
            return;
        }
        if let crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::PageDown,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        } = key_event
        {
            layout_scroll::page_down(self);
            return;
        }
        // Home/End: when the composer is empty, jump the history to start/end
        if let crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::Home,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        } = key_event
            && self.composer_is_empty() {
                layout_scroll::to_top(self);
                return;
            }
        if let crossterm::event::KeyEvent {
            code: crossterm::event::KeyCode::End,
            kind: KeyEventKind::Press | KeyEventKind::Repeat,
            ..
        } = key_event
            && self.composer_is_empty() {
                layout_scroll::to_bottom(self);
                return;
            }

        let composer_was_empty = self.bottom_pane.composer_is_empty();
        let input_result = self.bottom_pane.handle_key_event(key_event);
        let composer_is_empty = self.bottom_pane.composer_is_empty();
        if composer_was_empty && !composer_is_empty {
            for cell in &self.history_cells {
                cell.trigger_fade();
            }
            self.request_redraw();
        }
        self.auto_sync_goal_escape_state_from_composer();

        match input_result {
            InputResult::Submitted(text) => {
                if let Some(pending) = self.pending_request_user_input.take() {
                    self.submit_request_user_input_answer(pending, text);
                    return;
                }
                self.pending_turn_origin = Some(TurnOrigin::User);
                let cleaned = Self::strip_context_sections(&text);
                self.last_user_message = (!cleaned.trim().is_empty()).then_some(cleaned);
                if self.auto_state.should_show_goal_entry() {
                    for cell in &self.history_cells {
                        cell.trigger_fade();
                    }
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        self.bottom_pane.set_task_running(true);
                        self.bottom_pane
                            .update_status_text("Auto Drive Goal".to_string());
                        self.clear_composer();
                        self.request_redraw();
                        return;
                    }
                    self.clear_composer();
                    self.bottom_pane.update_status_text(String::new());
                    self.bottom_pane.set_task_running(false);
                    self.handle_auto_command(Some(trimmed.to_string()));
                    return;
                }
                if self.try_handle_terminal_shortcut(&text) {
                    return;
                }
                let user_message = self.parse_message_with_images(text);
                self.submit_user_message(user_message);
            }
            InputResult::Command(_cmd) => {
                // Command was dispatched at the App layer; request redraw.
                self.app_event_tx.send(AppEvent::RequestRedraw);
            }
            InputResult::ScrollUp => {
                let before = self.layout.scroll_offset.get();
                // Only allow Up to navigate command history when the top view
                // cannot be scrolled at all (no scrollback available).
                if self.layout.last_max_scroll.get() == 0
                    && self.bottom_pane.try_history_up() {
                        self.perf_track_scroll_delta(before, self.layout.scroll_offset.get());
                        return;
                    }
                // Scroll up in chat history (increase offset, towards older content)
                // Use last_max_scroll computed during the previous render to avoid overshoot
                let new_offset = self
                    .layout
                    .scroll_offset
                    .get()
                    .saturating_add(3)
                    .min(self.layout.last_max_scroll.get());
                self.layout.scroll_offset.set(new_offset);
                self.flash_scrollbar();
                self.sync_history_virtualization();
                // Enable compact mode so history can use the spacer line
                if self.layout.scroll_offset.get() > 0 {
                    self.bottom_pane.set_compact_compose(true);
                    self.height_manager
                        .borrow_mut()
                        .record_event(HeightEvent::ComposerModeChange);
                    // Mark that the very next Down should continue scrolling chat (sticky)
                    self.bottom_pane.mark_next_down_scrolls_history();
                }
                self.app_event_tx.send(AppEvent::RequestRedraw);
                self.height_manager
                    .borrow_mut()
                    .record_event(HeightEvent::UserScroll);
                self.maybe_show_history_nav_hint_on_first_scroll();
                self.perf_track_scroll_delta(before, self.layout.scroll_offset.get());
            }
            InputResult::ScrollDown => {
                let before = self.layout.scroll_offset.get();
                // Only allow Down to navigate command history when the top view
                // cannot be scrolled at all (no scrollback available).
                if self.layout.last_max_scroll.get() == 0 && self.bottom_pane.history_is_browsing()
                    && self.bottom_pane.try_history_down() {
                        self.perf_track_scroll_delta(before, self.layout.scroll_offset.get());
                        return;
                    }
                // Scroll down in chat history (decrease offset, towards bottom)
                if self.layout.scroll_offset.get() == 0 {
                    // Already at bottom: ensure spacer above input is enabled.
                    self.bottom_pane.set_compact_compose(false);
                    self.sync_history_virtualization();
                    self.app_event_tx.send(AppEvent::RequestRedraw);
                    self.height_manager
                        .borrow_mut()
                        .record_event(HeightEvent::UserScroll);
                    self.maybe_show_history_nav_hint_on_first_scroll();
                    self.height_manager
                        .borrow_mut()
                        .record_event(HeightEvent::ComposerModeChange);
                    self.perf_track_scroll_delta(before, self.layout.scroll_offset.get());
                } else if self.layout.scroll_offset.get() >= 3 {
                    // Move towards bottom but do NOT toggle spacer yet; wait until
                    // the user confirms by pressing Down again at bottom.
                    self.layout
                        .scroll_offset
                        .set(self.layout.scroll_offset.get().saturating_sub(3));
                    self.sync_history_virtualization();
                    self.app_event_tx.send(AppEvent::RequestRedraw);
                    self.height_manager
                        .borrow_mut()
                        .record_event(HeightEvent::UserScroll);
                    self.maybe_show_history_nav_hint_on_first_scroll();
                    self.perf_track_scroll_delta(before, self.layout.scroll_offset.get());
                } else if self.layout.scroll_offset.get() > 0 {
                    // Land exactly at bottom without toggling spacer yet; require
                    // a subsequent Down to re-enable the spacer so the input
                    // doesn't move when scrolling into the line above it.
                    self.layout.scroll_offset.set(0);
                    self.sync_history_virtualization();
                    self.app_event_tx.send(AppEvent::RequestRedraw);
                    self.height_manager
                        .borrow_mut()
                        .record_event(HeightEvent::UserScroll);
                    self.maybe_show_history_nav_hint_on_first_scroll();
                    self.perf_track_scroll_delta(before, self.layout.scroll_offset.get());
                }
                self.flash_scrollbar();
            }
            InputResult::None => {
                // Trigger redraw so input wrapping/height reflects immediately
                self.app_event_tx.send(AppEvent::RequestRedraw);
            }
        }
    }

    pub(super) fn toggle_browser_overlay(&mut self) {
        let new_state = !self.browser_overlay_visible;
        self.browser_overlay_visible = new_state;
        if new_state {
            if self.agents_terminal.active {
                self.exit_agents_terminal_mode();
            }
            self.browser_overlay_state.reset();
            let session_key = self
                .tools_state
                .browser_last_key
                .clone()
                .or_else(|| self.tools_state.browser_sessions.keys().next().cloned());
            self.browser_overlay_state.set_session_key(session_key.clone());
            if let Some(key) = session_key
                && let Some(tracker) = self.tools_state.browser_sessions.get(&key) {
                    let history_len = tracker.cell.screenshot_history().len();
                    if history_len > 0 {
                        self
                            .browser_overlay_state
                            .set_screenshot_index(history_len.saturating_sub(1));
                    }
                }
        } else {
            self.browser_overlay_state.reset();
        }
        self.request_redraw();
    }

    pub(super) fn handle_browser_overlay_key(&mut self, key_event: KeyEvent) -> bool {
        if !self.browser_overlay_visible {
            return false;
        }
        if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return true;
        }

        let shift = key_event.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key_event.modifiers.contains(KeyModifiers::CONTROL);

        match key_event.code {
            KeyCode::Esc => {
                self.browser_overlay_visible = false;
                self.browser_overlay_state.reset();
                self.request_redraw();
            }
            KeyCode::Up if shift => {
                self.adjust_browser_overlay_action_scroll(-1);
                self.request_redraw();
            }
            KeyCode::Down if shift => {
                self.adjust_browser_overlay_action_scroll(1);
                self.request_redraw();
            }
            KeyCode::Up => {
                if self.move_browser_overlay_screenshot(-1) {
                    self.request_redraw();
                }
            }
            KeyCode::Down => {
                if self.move_browser_overlay_screenshot(1) {
                    self.request_redraw();
                }
            }
            KeyCode::Left => {
                if self.move_browser_overlay_screenshot(-1) {
                    self.request_redraw();
                }
            }
            KeyCode::Right => {
                if self.move_browser_overlay_screenshot(1) {
                    self.request_redraw();
                }
            }
            KeyCode::PageUp => {
                let step = self.browser_overlay_state.last_action_view_height().max(1) as i16;
                self.adjust_browser_overlay_action_scroll(-step);
                self.request_redraw();
            }
            KeyCode::PageDown => {
                let step = self.browser_overlay_state.last_action_view_height().max(1) as i16;
                self.adjust_browser_overlay_action_scroll(step);
                self.request_redraw();
            }
            KeyCode::Home => {
                if self.set_browser_overlay_screenshot_index(0) {
                    self.request_redraw();
                }
            }
            KeyCode::End => {
                if let Some((_, tracker)) = self.browser_overlay_tracker() {
                    let len = tracker.cell.screenshot_history().len();
                    if len > 0 && self.set_browser_overlay_screenshot_index(len - 1) {
                        self.request_redraw();
                    }
                }
            }
            KeyCode::Char('j') if key_event.modifiers.is_empty() => {
                self.adjust_browser_overlay_action_scroll(1);
                self.request_redraw();
            }
            KeyCode::Char('k') if key_event.modifiers.is_empty() => {
                self.adjust_browser_overlay_action_scroll(-1);
                self.request_redraw();
            }
            KeyCode::Char('g') if ctrl => {
                if self.set_browser_overlay_screenshot_index(0) {
                    self.request_redraw();
                }
            }
            KeyCode::Char('G') if key_event.modifiers.is_empty() => {
                if let Some((_, tracker)) = self.browser_overlay_tracker() {
                    let len = tracker.cell.screenshot_history().len();
                    if len > 0 && self.set_browser_overlay_screenshot_index(len - 1) {
                        self.request_redraw();
                    }
                }
            }
            _ => {}
        }

        true
    }

    pub(super) fn browser_overlay_session_key(&self) -> Option<String> {
        if let Some(key) = self.browser_overlay_state.session_key()
            && self.tools_state.browser_sessions.contains_key(&key) {
                return Some(key);
            }
        if let Some(last) = self.tools_state.browser_last_key.clone()
            && self.tools_state.browser_sessions.contains_key(&last) {
                self.browser_overlay_state
                    .set_session_key(Some(last.clone()));
                return Some(last);
            }
        if let Some((key, _)) = self.tools_state.browser_sessions.iter().next() {
            let owned = key.clone();
            self.browser_overlay_state
                .set_session_key(Some(owned.clone()));
            return Some(owned);
        }
        None
    }

    pub(super) fn browser_overlay_tracker(
        &self,
    ) -> Option<(String, &browser_sessions::BrowserSessionTracker)> {
        let key = self.browser_overlay_session_key()?;
        self.tools_state
            .browser_sessions
            .get(&key)
            .map(|tracker| (key, tracker))
    }

    pub(super) fn set_browser_overlay_screenshot_index(&self, index: usize) -> bool {
        let Some((_, tracker)) = self.browser_overlay_tracker() else {
            return false;
        };
        let history = tracker.cell.screenshot_history();
        if history.is_empty() {
            return false;
        }
        let clamped = index.min(history.len().saturating_sub(1));
        if self.browser_overlay_state.screenshot_index() != clamped {
            self.browser_overlay_state.set_screenshot_index(clamped);
            return true;
        }
        false
    }

    pub(super) fn move_browser_overlay_screenshot(&self, delta: isize) -> bool {
        let Some((_, tracker)) = self.browser_overlay_tracker() else {
            return false;
        };
        let history = tracker.cell.screenshot_history();
        if history.is_empty() {
            return false;
        }
        let last_index = history.len() as isize - 1;
        let mut current = self.browser_overlay_state.screenshot_index() as isize;
        if current > last_index {
            current = last_index;
        }
        let mut new_index = current + delta;
        if new_index < 0 {
            new_index = 0;
        }
        if new_index > last_index {
            new_index = last_index;
        }
        if new_index != current {
            self.browser_overlay_state
                .set_screenshot_index(new_index as usize);
            return true;
        }
        false
    }

    pub(super) fn adjust_browser_overlay_action_scroll(&self, delta: i16) {
        let current = self.browser_overlay_state.action_scroll() as i32;
        let max = self.browser_overlay_state.max_action_scroll() as i32;
        let mut updated = current + delta as i32;
        if updated < 0 {
            updated = 0;
        } else if updated > max {
            updated = max;
        }
        self.browser_overlay_state
            .set_action_scroll(updated as u16);
    }

    pub(super) fn toggle_agents_hud(&mut self) {
        if self.agents_terminal.active {
            self.exit_agents_terminal_mode();
        } else {
            self.enter_agents_terminal_mode();
        }
    }

    pub(super) fn set_limits_overlay_content(&mut self, content: LimitsOverlayContent) {
        let handled_by_settings = self.update_limits_settings_content(content.clone());
        if handled_by_settings {
            self.limits.cached_content = None;
        } else {
            self.limits.cached_content = Some(content);
        }
    }

    pub(super) fn update_limits_settings_content(&mut self, content: LimitsOverlayContent) -> bool {
        if let Some(overlay) = self.settings.overlay.as_mut() {
            if let Some(view) = overlay.limits_content_mut() {
                view.set_content(content);
            } else {
                overlay.set_limits_content(LimitsSettingsContent::new(
                    content,
                    self.config.tui.limits.layout_mode,
                ));
            }
            self.request_redraw();
            true
        } else {
            false
        }
    }

    pub(super) fn set_limits_overlay_tabs(&mut self, tabs: Vec<LimitsTab>) {
        let content = if tabs.is_empty() {
            LimitsOverlayContent::Placeholder
        } else {
            LimitsOverlayContent::Tabs(tabs)
        };
        self.set_limits_overlay_content(content);
    }

    pub(super) fn build_limits_tabs(
        &self,
        current_snapshot: Option<RateLimitSnapshotEvent>,
        current_reset: RateLimitResetInfo,
    ) -> Vec<LimitsTab> {
        use std::collections::HashSet;

        let code_home = self.config.code_home.clone();
        let accounts = auth_accounts::list_accounts(&code_home).unwrap_or_default();
        let account_map: HashMap<String, StoredAccount> = accounts
            .into_iter()
            .map(|account| (account.id.clone(), account))
            .collect();

        let active_id = auth_accounts::get_active_account_id(&code_home)
            .ok()
            .flatten();

        let usage_records = account_usage::list_rate_limit_snapshots(&code_home).unwrap_or_default();
        let mut snapshot_map: HashMap<String, StoredRateLimitSnapshot> = usage_records
            .into_iter()
            .filter(|record| account_map.contains_key(&record.account_id))
            .map(|record| (record.account_id.clone(), record))
            .collect();

        let mut usage_summary_map: HashMap<String, StoredUsageSummary> = HashMap::new();
        for id in account_map.keys() {
            if let Ok(Some(summary)) = account_usage::load_account_usage(&code_home, id) {
                usage_summary_map.insert(id.clone(), summary);
            }
        }

        if let Some(active_id) = active_id.as_ref()
            && !usage_summary_map.contains_key(active_id)
                && let Ok(Some(summary)) = account_usage::load_account_usage(&code_home, active_id) {
                    usage_summary_map.insert(active_id.clone(), summary);
                }

        let mut tabs: Vec<LimitsTab> = Vec::new();
        let mut seen_ids: HashSet<String> = HashSet::new();

        if let Some(snapshot) = current_snapshot {
            let account_ref = active_id
                .as_ref()
                .and_then(|id| account_map.get(id));
            let snapshot_ref = active_id
                .as_ref()
                .and_then(|id| snapshot_map.get(id));
            let summary_ref = active_id
                .as_ref()
                .and_then(|id| usage_summary_map.get(id));

            let title = account_ref
                .map(account_display_label)
                .or_else(|| active_id.clone())
                .unwrap_or_else(|| "Current session".to_string());
            let header = Self::account_header_lines(account_ref, snapshot_ref, summary_ref);
            let is_api_key_account = matches!(
                account_ref.map(|acc| acc.mode),
                Some(AuthMode::ApiKey)
            );
            let extra = Self::usage_history_lines(summary_ref, is_api_key_account);
            let display = Self::rate_limit_display_config_for_account(account_ref);
            let view = build_limits_view(
                &snapshot,
                current_reset,
                DEFAULT_GRID_CONFIG,
                display,
            );
            tabs.push(LimitsTab::view(title, header, view, extra));

            if let Some(active_id) = active_id.as_ref()
                && account_map.contains_key(active_id) {
                    seen_ids.insert(active_id.clone());
                    snapshot_map.remove(active_id);
                    usage_summary_map.remove(active_id);
                }
        }

        let mut remaining_ids: Vec<String> = account_map
            .keys()
            .filter(|id| !seen_ids.contains(*id))
            .cloned()
            .collect();

        let account_sort_key = |id: &String| {
            if let Some(account) = account_map.get(id) {
                let label = account_display_label(account);
                (
                    account_mode_priority(account.mode),
                    label.to_ascii_lowercase(),
                    label,
                )
            } else {
                (u8::MAX, id.to_ascii_lowercase(), id.clone())
            }
        };

        remaining_ids.sort_by(|a, b| {
            let (a_priority, a_lower, a_label) = account_sort_key(a);
            let (b_priority, b_lower, b_label) = account_sort_key(b);
            a_priority
                .cmp(&b_priority)
                .then_with(|| a_lower.cmp(&b_lower))
                .then_with(|| a_label.cmp(&b_label))
                .then_with(|| a.cmp(b))
        });

        for id in remaining_ids {
            let account = account_map.get(&id);
            let record = snapshot_map.remove(&id);
            let usage_summary = usage_summary_map.remove(&id);
            let title = account
                .map(account_display_label)
                .unwrap_or_else(|| id.clone());
            match record {
                Some(record) => {
                    if let Some(snapshot) = record.snapshot.clone() {
                        let view_snapshot = snapshot.clone();
                        let view_reset = RateLimitResetInfo {
                            primary_next_reset: record.primary_next_reset_at,
                            secondary_next_reset: record.secondary_next_reset_at,
                            ..RateLimitResetInfo::default()
                        };
                        let display = Self::rate_limit_display_config_for_account(account);
                        let view = build_limits_view(
                            &view_snapshot,
                            view_reset,
                            DEFAULT_GRID_CONFIG,
                            display,
                        );
                        let header = Self::account_header_lines(
                            account,
                            Some(&record),
                            usage_summary.as_ref(),
                        );
                        let is_api_key_account = matches!(
                            account.map(|acc| acc.mode),
                            Some(AuthMode::ApiKey)
                        );
                        let extra = Self::usage_history_lines(
                            usage_summary.as_ref(),
                            is_api_key_account,
                        );
                        tabs.push(LimitsTab::view(title, header, view, extra));
                    } else {
                        let is_api_key_account = matches!(
                            account.map(|acc| acc.mode),
                            Some(AuthMode::ApiKey)
                        );
                        let mut lines = Self::usage_history_lines(
                            usage_summary.as_ref(),
                            is_api_key_account,
                        );
                        lines.push(Self::dim_line(
                            " Rate limit snapshot not yet available.",
                        ));
                        let header = Self::account_header_lines(
                            account,
                            Some(&record),
                            usage_summary.as_ref(),
                        );
                        tabs.push(LimitsTab::message(title, header, lines));
                    }
                }
                None => {
                    let is_api_key_account = matches!(
                        account.map(|acc| acc.mode),
                        Some(AuthMode::ApiKey)
                    );
                    let mut lines = Self::usage_history_lines(
                        usage_summary.as_ref(),
                        is_api_key_account,
                    );
                    lines.push(Self::dim_line(
                        " Rate limit snapshot not yet available.",
                    ));
                    let header = Self::account_header_lines(
                        account,
                        None,
                        usage_summary.as_ref(),
                    );
                    tabs.push(LimitsTab::message(title, header, lines));
                }
            }
        }

        if tabs.is_empty() {
            let mut lines = Self::usage_history_lines(None, false);
            lines.push(Self::dim_line(
                " Rate limit snapshot not yet available.",
            ));
            tabs.push(LimitsTab::message("Usage", Vec::new(), lines));
        }

        tabs
    }

    pub(super) fn usage_cost_usd_from_totals(totals: &TokenTotals) -> f64 {
        let non_cached_input = totals
            .input_tokens
            .saturating_sub(totals.cached_input_tokens);
        let input_cost = (non_cached_input as f64 / TOKENS_PER_MILLION)
            * INPUT_COST_PER_MILLION_USD;
        let cached_cost = (totals.cached_input_tokens as f64 / TOKENS_PER_MILLION)
            * CACHED_INPUT_COST_PER_MILLION_USD;
        let output_cost = (totals.output_tokens as f64 / TOKENS_PER_MILLION)
            * OUTPUT_COST_PER_MILLION_USD;
        input_cost + cached_cost + output_cost
    }

    pub(super) fn format_usd(amount: f64) -> String {
        let cents = (amount * 100.0).round().max(0.0);
        let cents_u128 = cents as u128;
        let dollars_u128 = cents_u128 / 100;
        let cents_part = (cents_u128 % 100) as u8;
        let dollars = dollars_u128.min(i64::MAX as u128) as i64;
        if cents_part == 0 {
            format!("${} USD", format_with_separators(dollars))
        } else {
            format!(
                "${}.{:02} USD",
                format_with_separators(dollars),
                cents_part
            )
        }
    }

    pub(super) fn accumulate_token_totals(target: &mut TokenTotals, delta: &TokenTotals) {
        target.input_tokens = target
            .input_tokens
            .saturating_add(delta.input_tokens);
        target.cached_input_tokens = target
            .cached_input_tokens
            .saturating_add(delta.cached_input_tokens);
        target.output_tokens = target
            .output_tokens
            .saturating_add(delta.output_tokens);
        target.reasoning_output_tokens = target
            .reasoning_output_tokens
            .saturating_add(delta.reasoning_output_tokens);
        target.total_tokens = target
            .total_tokens
            .saturating_add(delta.total_tokens);
    }

    pub(super) fn account_header_lines(
        account: Option<&StoredAccount>,
        record: Option<&StoredRateLimitSnapshot>,
        usage: Option<&StoredUsageSummary>,
    ) -> Vec<RtLine<'static>> {
        let mut lines: Vec<RtLine<'static>> = Vec::new();

        let account_type = account
            .map(|acc| match acc.mode {
                AuthMode::ChatGPT | AuthMode::ChatgptAuthTokens => "ChatGPT account",
                AuthMode::ApiKey => "API key",
            })
            .unwrap_or("Unknown account");

        let plan = record
            .and_then(|r| r.plan.as_deref())
            .or_else(|| usage.and_then(|u| u.plan.as_deref()))
            .unwrap_or("Unknown");

        let value_style = Style::default().fg(crate::colors::text_dim());
        let is_api_key = matches!(account.map(|acc| acc.mode), Some(AuthMode::ApiKey));
        let totals = usage
            .map(|u| u.totals.clone())
            .unwrap_or_default();
        let non_cached_input = totals
            .input_tokens
            .saturating_sub(totals.cached_input_tokens);
        let cached_input = totals.cached_input_tokens;
        let output_tokens = totals.output_tokens;
        let reasoning_tokens = totals.reasoning_output_tokens;
        let total_tokens = totals.total_tokens;

        let cost_usd = Self::usage_cost_usd_from_totals(&totals);
        let formatted_total = format_with_separators_u64(total_tokens);
        let formatted_cost = Self::format_usd(cost_usd);
        let cost_suffix = if is_api_key {
            format!("({formatted_cost})")
        } else {
            format!("(API would cost {formatted_cost})")
        };

        lines.push(RtLine::from(String::new()));

        lines.push(RtLine::from(vec![
            RtSpan::raw(status_field_prefix("Type")),
            RtSpan::styled(account_type.to_string(), value_style),
        ]));
        lines.push(RtLine::from(vec![
            RtSpan::raw(status_field_prefix("Plan")),
            RtSpan::styled(plan.to_string(), value_style),
        ]));
        let tokens_prefix = status_field_prefix("Tokens");
        let tokens_summary = format!("{formatted_total} total {cost_suffix}");
        lines.push(RtLine::from(vec![
            RtSpan::raw(tokens_prefix.clone()),
            RtSpan::styled(tokens_summary, value_style),
        ]));

        let indent = " ".repeat(tokens_prefix.len());
        let counts = [
            (format_with_separators_u64(cached_input), "cached"),
            (format_with_separators_u64(non_cached_input), "input"),
            (format_with_separators_u64(output_tokens), "output"),
            (format_with_separators_u64(reasoning_tokens), "reasoning"),
        ];
        let max_width = counts
            .iter()
            .map(|(count, _)| count.len())
            .max()
            .unwrap_or(0);
        for (count, label) in counts.iter() {
            let number = format!("{count:>max_width$}");
            lines.push(RtLine::from(vec![
                RtSpan::raw(indent.clone()),
                RtSpan::styled(number, value_style),
                RtSpan::styled(format!(" {label}"), value_style),
            ]));
        }
        lines
    }

    pub(super) fn hourly_usage_lines(
        summary: Option<&StoredUsageSummary>,
        is_api_key_account: bool,
    ) -> Vec<RtLine<'static>> {
        const WIDTH: usize = 14;
        let now = Local::now();
        let anchor = now
            - ChronoDuration::minutes(now.minute() as i64)
            - ChronoDuration::seconds(now.second() as i64)
            - ChronoDuration::nanoseconds(now.nanosecond() as i64);

        let hourly_totals = Self::aggregate_hourly_totals(summary);
        let series: Vec<(DateTime<Local>, TokenTotals)> = (0..12)
            .map(|offset| anchor - ChronoDuration::hours(offset as i64))
            .map(|dt| {
                let utc_key = Self::truncate_utc_hour(dt.with_timezone(&Utc));
                let totals = hourly_totals
                    .get(&utc_key)
                    .cloned()
                    .unwrap_or_default();
                (dt, totals)
            })
            .collect();

        let max_total = series
            .iter()
            .map(|(_, totals)| totals.total_tokens)
            .max()
            .unwrap_or(0);

        let mut lines: Vec<RtLine<'static>> = Vec::new();
        lines.push(RtLine::from(vec![RtSpan::styled(
            "12 Hour History",
            Style::default().add_modifier(Modifier::BOLD),
        )]));

        let prefix = status_content_prefix();
        let tokens_width = series
            .iter()
            .map(|(_, totals)| format_with_separators_u64(totals.total_tokens).len())
            .max()
            .unwrap_or(0);
        let cached_width = series
            .iter()
            .map(|(_, totals)| format_with_separators_u64(totals.cached_input_tokens).len())
            .max()
            .unwrap_or(0);
        let cost_width = series
            .iter()
            .map(|(_, totals)| Self::format_usd(Self::usage_cost_usd_from_totals(totals)).len())
            .max()
            .unwrap_or(0);
        let column_divider = RtSpan::styled(
            " │ ",
            Style::default().fg(crate::colors::text_dim()),
        );
        for (dt, totals) in series.iter() {
            let label = Self::format_hour_label(*dt);
            let bar = Self::bar_segment(totals.total_tokens, max_total, WIDTH);
            let tokens = format_with_separators_u64(totals.total_tokens);
            let padding = tokens_width.saturating_sub(tokens.len());
            let formatted_tokens = format!("{space}{tokens}", space = " ".repeat(padding), tokens = tokens);
            let cached_tokens = format_with_separators_u64(totals.cached_input_tokens);
            let cached_padding = cached_width.saturating_sub(cached_tokens.len());
            let cached_display = format!(
                "{space}{cached_tokens}",
                space = " ".repeat(cached_padding),
                cached_tokens = cached_tokens
            );
            let cost_text = Self::format_usd(Self::usage_cost_usd_from_totals(totals));
            let cost_display = if is_api_key_account {
                format!(
                    "{space}{cost_text}",
                    space = " ".repeat(cost_width.saturating_sub(cost_text.len())),
                    cost_text = cost_text
                )
            } else {
                let saved = Self::format_usd(Self::usage_cost_usd_from_totals(totals));
                format!(
                    "{space}{saved}",
                    space = " ".repeat(cost_width.saturating_sub(saved.len())),
                    saved = saved
                )
            };
            lines.push(RtLine::from(vec![
                RtSpan::raw(prefix.clone()),
                RtSpan::styled(
                    format!("{label} "),
                    Style::default().fg(crate::colors::text_dim()),
                ),
                RtSpan::styled("│ ", Style::default().fg(crate::colors::text_dim())),
                RtSpan::styled(bar, Style::default().fg(crate::colors::primary())),
                RtSpan::raw(format!(" {formatted_tokens} tokens")),
                column_divider.clone(),
                RtSpan::styled(
                    format!("{cached_display} cached"),
                    Style::default().fg(crate::colors::text_dim()),
                ),
                column_divider.clone(),
                RtSpan::styled(
                    format!(
                        "{cost_display} {}",
                        if is_api_key_account { "cost" } else { "saved" }
                    ),
                    Style::default().fg(crate::colors::text_dim()),
                ),
            ]));
        }
        lines
    }

    pub(super) fn daily_usage_lines(
        summary: Option<&StoredUsageSummary>,
        is_api_key_account: bool,
    ) -> Vec<RtLine<'static>> {
        const WIDTH: usize = 14;
        let today = Local::now().date_naive();
        let day_totals = Self::aggregate_daily_totals(summary);
        let daily: Vec<(chrono::NaiveDate, TokenTotals)> = (0..7)
            .map(|offset| today - ChronoDuration::days(offset as i64))
            .map(|day| {
                let totals = day_totals.get(&day).cloned().unwrap_or_default();
                (day, totals)
            })
            .collect();

        let max_total = daily
            .iter()
            .map(|(_, totals)| totals.total_tokens)
            .max()
            .unwrap_or(0);
        let mut lines: Vec<RtLine<'static>> = Vec::new();
        lines.push(Self::dim_line(String::new()));
        lines.push(RtLine::from(vec![RtSpan::styled(
            "7 Day History",
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        let prefix = status_content_prefix();
        let tokens_width = daily
            .iter()
            .map(|(_, totals)| format_with_separators_u64(totals.total_tokens).len())
            .max()
            .unwrap_or(0);
        let cached_width = daily
            .iter()
            .map(|(_, totals)| format_with_separators_u64(totals.cached_input_tokens).len())
            .max()
            .unwrap_or(0);
        let cost_width = daily
            .iter()
            .map(|(_, totals)| Self::format_usd(Self::usage_cost_usd_from_totals(totals)).len())
            .max()
            .unwrap_or(0);
        let column_divider = RtSpan::styled(
            " │ ",
            Style::default().fg(crate::colors::text_dim()),
        );
        for (day, totals) in daily.iter() {
            let label = Self::format_daily_label(*day);
            let bar = Self::bar_segment(totals.total_tokens, max_total, WIDTH);
            let tokens = format_with_separators_u64(totals.total_tokens);
            let padding = tokens_width.saturating_sub(tokens.len());
            let formatted_tokens = format!("{space}{tokens}", space = " ".repeat(padding), tokens = tokens);
            let cached_tokens = format_with_separators_u64(totals.cached_input_tokens);
            let cached_padding = cached_width.saturating_sub(cached_tokens.len());
            let cached_display = format!(
                "{space}{cached_tokens}",
                space = " ".repeat(cached_padding),
                cached_tokens = cached_tokens
            );
            let daily_cost = Self::usage_cost_usd_from_totals(totals);
            let cost_text = Self::format_usd(daily_cost);
            let cost_display = if is_api_key_account {
                format!(
                    "{space}{cost_text}",
                    space = " ".repeat(cost_width.saturating_sub(cost_text.len())),
                    cost_text = cost_text
                )
            } else {
                let saved = Self::format_usd(Self::usage_cost_usd_from_totals(totals));
                format!(
                    "{space}{saved}",
                    space = " ".repeat(cost_width.saturating_sub(saved.len())),
                    saved = saved
                )
            };
            lines.push(RtLine::from(vec![
                RtSpan::raw(prefix.clone()),
                RtSpan::styled(
                    format!("{label} "),
                    Style::default().fg(crate::colors::text_dim()),
                ),
                RtSpan::styled("│ ", Style::default().fg(crate::colors::text_dim())),
                RtSpan::styled(bar, Style::default().fg(crate::colors::primary())),
                RtSpan::raw(format!(" {formatted_tokens} tokens")),
                column_divider.clone(),
                RtSpan::styled(
                    format!("{cached_display} cached"),
                    Style::default().fg(crate::colors::text_dim()),
                ),
                column_divider.clone(),
                RtSpan::styled(
                    format!(
                        "{cost_display} {}",
                        if is_api_key_account { "cost" } else { "saved" }
                    ),
                    Style::default().fg(crate::colors::text_dim()),
                ),
            ]));
        }
        lines
    }

    pub(super) fn day_suffix(day: u32) -> &'static str {
        if (11..=13).contains(&(day % 100)) {
            return "th";
        }
        match day % 10 {
            1 => "st",
            2 => "nd",
            3 => "rd",
            _ => "th",
        }
    }

    pub(super) fn format_daily_label(date: chrono::NaiveDate) -> String {
        let suffix = Self::day_suffix(date.day());
        format!("{} {:>2}{}", date.format("%b"), date.day(), suffix)
    }

    pub(super) fn format_hour_label(dt: DateTime<Local>) -> String {
        let (is_pm, hour) = dt.hour12();
        let meridiem = if is_pm { "pm" } else { "am" };
        format!("{} {:>2}{}", dt.format("%a"), hour, meridiem)
    }

    pub(super) fn usage_history_lines(
        summary: Option<&StoredUsageSummary>,
        is_api_key_account: bool,
    ) -> Vec<RtLine<'static>> {
        let mut lines = Self::hourly_usage_lines(summary, is_api_key_account);
        lines.extend(Self::daily_usage_lines(summary, is_api_key_account));
        lines.extend(Self::six_month_usage_lines(summary, is_api_key_account));
        lines
    }

    pub(super) fn six_month_usage_lines(
        summary: Option<&StoredUsageSummary>,
        is_api_key_account: bool,
    ) -> Vec<RtLine<'static>> {
        const WIDTH: usize = 14;
        const MONTHS: usize = 6;

        let today = Local::now().date_naive();
        let mut year = today.year();
        let mut month = today.month();

        let month_totals = Self::aggregate_monthly_totals(summary);
        let mut months: Vec<(chrono::NaiveDate, TokenTotals)> = Vec::with_capacity(MONTHS);
        for _ in 0..MONTHS {
            let Some(start) = chrono::NaiveDate::from_ymd_opt(year, month, 1) else {
                break;
            };
            let key = (start.year(), start.month());
            let totals = month_totals
                .get(&key)
                .cloned()
                .unwrap_or_default();
            months.push((start, totals));
            if month == 1 {
                month = 12;
                year -= 1;
            } else {
                month -= 1;
            }
        }

        let max_total = months
            .iter()
            .map(|(_, totals)| totals.total_tokens)
            .max()
            .unwrap_or(0);

        let mut lines: Vec<RtLine<'static>> = Vec::new();
        lines.push(Self::dim_line(String::new()));
        lines.push(RtLine::from(vec![RtSpan::styled(
            "6 Month History",
            Style::default().add_modifier(Modifier::BOLD),
        )]));

        let prefix = status_content_prefix();
        let tokens_width = months
            .iter()
            .map(|(_, totals)| format_with_separators_u64(totals.total_tokens).len())
            .max()
            .unwrap_or(0);
        let cached_width = months
            .iter()
            .map(|(_, totals)| format_with_separators_u64(totals.cached_input_tokens).len())
            .max()
            .unwrap_or(0);
        let cost_width = months
            .iter()
            .map(|(_, totals)| Self::format_usd(Self::usage_cost_usd_from_totals(totals)).len())
            .max()
            .unwrap_or(0);
        let column_divider = RtSpan::styled(
            " │ ",
            Style::default().fg(crate::colors::text_dim()),
        );
        for (start, totals) in months.iter() {
            let label = start.format("%b %Y").to_string();
            let bar = Self::bar_segment(totals.total_tokens, max_total, WIDTH);
            let tokens = format_with_separators_u64(totals.total_tokens);
            let padding = tokens_width.saturating_sub(tokens.len());
            let formatted_tokens = format!("{space}{tokens}", space = " ".repeat(padding), tokens = tokens);
            let cached_tokens = format_with_separators_u64(totals.cached_input_tokens);
            let cached_padding = cached_width.saturating_sub(cached_tokens.len());
            let cached_display = format!(
                "{space}{cached_tokens}",
                space = " ".repeat(cached_padding),
                cached_tokens = cached_tokens
            );
            let cost_text = Self::format_usd(Self::usage_cost_usd_from_totals(totals));
            let cost_display = if is_api_key_account {
                format!(
                    "{space}{cost_text}",
                    space = " ".repeat(cost_width.saturating_sub(cost_text.len())),
                    cost_text = cost_text
                )
            } else {
                let saved = Self::format_usd(Self::usage_cost_usd_from_totals(totals));
                format!(
                    "{space}{saved}",
                    space = " ".repeat(cost_width.saturating_sub(saved.len())),
                    saved = saved
                )
            };
            lines.push(RtLine::from(vec![
                RtSpan::raw(prefix.clone()),
                RtSpan::styled(
                    format!("{label} "),
                    Style::default().fg(crate::colors::text_dim()),
                ),
                RtSpan::styled("│ ", Style::default().fg(crate::colors::text_dim())),
                RtSpan::styled(bar, Style::default().fg(crate::colors::primary())),
                RtSpan::raw(format!(" {formatted_tokens} tokens")),
                column_divider.clone(),
                RtSpan::styled(
                    format!("{cached_display} cached"),
                    Style::default().fg(crate::colors::text_dim()),
                ),
                column_divider.clone(),
                RtSpan::styled(
                    format!(
                        "{cost_display} {}",
                        if is_api_key_account { "cost" } else { "saved" }
                    ),
                    Style::default().fg(crate::colors::text_dim()),
                ),
            ]));
        }
        lines
    }

    pub(super) fn bar_segment(value: u64, max: u64, width: usize) -> String {
        const FILL: &str = "▇";
        if max == 0 {
            return format!("{}{}", FILL, " ".repeat(width.saturating_sub(1)));
        }
        if value == 0 {
            return format!("{}{}", FILL, " ".repeat(width.saturating_sub(1)));
        }
        let ratio = value as f64 / max as f64;
        let filled = (ratio * width as f64).ceil().clamp(1.0, width as f64) as usize;
        format!(
            "{}{}",
            FILL.repeat(filled),
            " ".repeat(width.saturating_sub(filled))
        )
    }

    pub(super) fn dim_line(text: impl Into<String>) -> RtLine<'static> {
        RtLine::from(vec![RtSpan::styled(
            text.into(),
            Style::default().fg(crate::colors::text_dim()),
        )])
    }

    pub(super) fn truncate_utc_hour(ts: DateTime<Utc>) -> DateTime<Utc> {
        let naive = ts.naive_utc();
        let Some(trimmed) = naive
            .with_minute(0)
            .and_then(|dt| dt.with_second(0))
            .and_then(|dt| dt.with_nanosecond(0))
        else {
            return ts;
        };
        Utc.from_utc_datetime(&trimmed)
    }

    pub(super) fn aggregate_hourly_totals(
        summary: Option<&StoredUsageSummary>,
    ) -> HashMap<DateTime<Utc>, TokenTotals> {
        let mut totals = HashMap::new();
        if let Some(summary) = summary {
            for entry in &summary.hourly_entries {
                let key = Self::truncate_utc_hour(entry.timestamp);
                let slot = totals.entry(key).or_insert_with(TokenTotals::default);
                Self::accumulate_token_totals(slot, &entry.tokens);
            }
            for bucket in &summary.hourly_buckets {
                let slot = totals
                    .entry(bucket.period_start)
                    .or_insert_with(TokenTotals::default);
                Self::accumulate_token_totals(slot, &bucket.tokens);
            }
        }
        totals
    }

    pub(super) fn aggregate_daily_totals(
        summary: Option<&StoredUsageSummary>,
    ) -> HashMap<chrono::NaiveDate, TokenTotals> {
        let mut totals = HashMap::new();
        if let Some(summary) = summary {
            for bucket in &summary.daily_buckets {
                let key = bucket.period_start.date_naive();
                let slot = totals.entry(key).or_insert_with(TokenTotals::default);
                Self::accumulate_token_totals(slot, &bucket.tokens);
            }
            for bucket in &summary.hourly_buckets {
                let key = bucket.period_start.date_naive();
                let slot = totals.entry(key).or_insert_with(TokenTotals::default);
                Self::accumulate_token_totals(slot, &bucket.tokens);
            }
            for entry in &summary.hourly_entries {
                let key = entry.timestamp.date_naive();
                let slot = totals.entry(key).or_insert_with(TokenTotals::default);
                Self::accumulate_token_totals(slot, &entry.tokens);
            }
        }
        totals
    }

    pub(super) fn aggregate_monthly_totals(
        summary: Option<&StoredUsageSummary>,
    ) -> HashMap<(i32, u32), TokenTotals> {
        let mut totals = HashMap::new();
        if let Some(summary) = summary {
            let mut accumulate = |dt: DateTime<Utc>, tokens: &TokenTotals| {
                let date = dt.date_naive();
                let key = (date.year(), date.month());
                let slot = totals.entry(key).or_insert_with(TokenTotals::default);
                Self::accumulate_token_totals(slot, tokens);
            };

            for bucket in &summary.monthly_buckets {
                accumulate(bucket.period_start, &bucket.tokens);
            }
            for bucket in &summary.daily_buckets {
                accumulate(bucket.period_start, &bucket.tokens);
            }
            for bucket in &summary.hourly_buckets {
                accumulate(bucket.period_start, &bucket.tokens);
            }
            for entry in &summary.hourly_entries {
                accumulate(entry.timestamp, &entry.tokens);
            }
        }
        totals
    }

    // dispatch_command() removed — command routing is handled at the App layer via AppEvent::DispatchCommand

    pub(crate) fn handle_paste(&mut self, text: String) {
        if settings_handlers::handle_settings_paste(self, text.clone()) {
            return;
        }
        // Check if the pasted text is a file path to an image
        let trimmed = text.trim();

        tracing::info!("Paste received: {:?}", trimmed);

        const IMAGE_EXTENSIONS: &[&str] = &[
            ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp", ".svg", ".ico", ".tiff", ".tif",
        ];

        // Check if it looks like a file path
        let is_likely_path = trimmed.starts_with("file://")
            || trimmed.starts_with("/")
            || trimmed.starts_with("~/")
            || trimmed.starts_with("./");

        if is_likely_path {
            // Remove escape backslashes that terminals add for special characters
            let unescaped = trimmed
                .replace("\\ ", " ")
                .replace("\\(", "(")
                .replace("\\)", ")");

            // Handle file:// URLs (common when dragging from Finder)
            let path_str = if unescaped.starts_with("file://") {
                // URL decode to handle spaces and special characters
                // Simple decoding for common cases (spaces as %20, etc.)
                unescaped
                    .strip_prefix("file://")
                    .map(|s| {
                        s.replace("%20", " ")
                            .replace("%28", "(")
                            .replace("%29", ")")
                            .replace("%5B", "[")
                            .replace("%5D", "]")
                            .replace("%2C", ",")
                            .replace("%27", "'")
                            .replace("%26", "&")
                            .replace("%23", "#")
                            .replace("%40", "@")
                            .replace("%2B", "+")
                            .replace("%3D", "=")
                            .replace("%24", "$")
                            .replace("%21", "!")
                            .replace("%2D", "-")
                            .replace("%2E", ".")
                    })
                    .unwrap_or_else(|| unescaped.clone())
            } else {
                unescaped
            };

            tracing::info!("Decoded path: {:?}", path_str);

            // Check if it has an image extension
            let is_image = IMAGE_EXTENSIONS
                .iter()
                .any(|ext| path_str.to_lowercase().ends_with(ext));

            if is_image {
                let path = PathBuf::from(&path_str);
                tracing::info!("Checking if path exists: {:?}", path);
                if path.exists() {
                    tracing::info!("Image file dropped/pasted: {:?}", path);
                    // Get just the filename for display
                    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("image");

                    // Add a placeholder to the compose field instead of submitting
                    let placeholder = format!("[image: {filename}]");

                    let persisted = self.persist_user_image_if_needed(&path).unwrap_or(path);

                    // Store the image path for later submission
                    self.pending_images.insert(placeholder.clone(), persisted);

                    // Add the placeholder text to the compose field
                    self.bottom_pane.handle_paste(placeholder);
                    self.auto_sync_goal_escape_state_from_composer();
                    // Force immediate redraw to reflect input growth/wrap
                    self.request_redraw();
                    return;
                } else {
                    tracing::warn!("Image path does not exist: {:?}", path);
                }
            } else {
                // For non-image files, paste the decoded path as plain text.
                let path = PathBuf::from(&path_str);
                if path.exists() && path.is_file() {
                    self.bottom_pane.handle_paste(path_str);
                    self.auto_sync_goal_escape_state_from_composer();
                    self.request_redraw();
                    return;
                }
            }
        }

        // Otherwise handle as regular text paste
        self.bottom_pane.handle_paste(text);
        self.auto_sync_goal_escape_state_from_composer();
        // Force immediate redraw so compose height matches new content
        self.request_redraw();
    }

    /// Briefly show the vertical scrollbar and schedule a redraw to hide it.
    pub(super) fn flash_scrollbar(&self) {
        layout_scroll::flash_scrollbar(self);
    }

    pub(super) fn ensure_image_cell_picker(&self, cell: &dyn HistoryCell) {
        if let Some(image) = cell
            .as_any()
            .downcast_ref::<crate::history_cell::ImageOutputCell>()
        {
            let picker = self.terminal_info.picker.clone();
            let font_size = self.terminal_info.font_size;
            image.ensure_picker_initialized(picker, font_size);
        }
    }

    pub(super) fn history_insert_with_key_global(
        &mut self,
        cell: Box<dyn HistoryCell>,
        key: OrderKey,
    ) -> usize {
        self.history_insert_with_key_global_tagged(cell, key, "untagged", None)
    }

    // Internal: same as above but with a short tag for debug overlays.
    pub(super) fn history_insert_with_key_global_tagged(
        &mut self,
        cell: Box<dyn HistoryCell>,
        key: OrderKey,
        tag: &'static str,
        record: Option<HistoryDomainRecord>,
    ) -> usize {
        #[cfg(debug_assertions)]
        {
            let cell_kind = cell.kind();
            if cell_kind == HistoryCellType::BackgroundEvent {
                debug_assert!(
                    tag == "background",
                    "Background events must use the background helper (tag={tag})"
                );
            }
        }
        self.ensure_image_cell_picker(cell.as_ref());
        // Any ordered insert of a non-reasoning cell means reasoning is no longer the
        // bottom-most active block; drop the in-progress ellipsis on collapsed titles.
        let is_reasoning_cell = cell
            .as_any()
            .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
            .is_some();
        if !is_reasoning_cell {
            self.clear_reasoning_in_progress();
        }
        let is_background_cell = matches!(cell.kind(), HistoryCellType::BackgroundEvent);
        let mut key = key;
        let mut key_bumped = false;
        if !is_background_cell
            && let Some(last) = self.last_assigned_order
                && key <= last {
                    key = Self::order_key_successor(last);
                    key_bumped = true;
                }

        // Determine insertion position across the entire history.
        // Most ordered inserts are monotonic tail-appends (we bump non-background
        // keys to keep them strictly increasing), so avoid an O(n) scan in the
        // common case.
        //
        // Exception: some early, non-background system cells (e.g. the context
        // summary) are inserted with a low order key before any ordering state
        // has been established. In that phase, we must still respect the order.
        let mut pos = self.history_cells.len();
        if is_background_cell || self.last_assigned_order.is_none() {
            for i in 0..self.history_cells.len() {
                if let Some(existing) = self.cell_order_seq.get(i)
                    && *existing > key {
                        pos = i;
                        break;
                    }
            }
        }

        // Keep auxiliary order vector in lockstep with history before inserting
        if self.cell_order_seq.len() < self.history_cells.len() {
            let missing = self.history_cells.len() - self.cell_order_seq.len();
            for _ in 0..missing {
                self.cell_order_seq.push(OrderKey {
                    req: 0,
                    out: -1,
                    seq: 0,
                });
            }
        }

        tracing::info!(
            "[order] insert: {} pos={} len_before={} order_len_before={} tag={}",
            Self::debug_fmt_order_key(key),
            pos,
            self.history_cells.len(),
            self.cell_order_seq.len(),
            tag
        );
        // If order overlay is enabled, compute a short, inline debug summary for
        // reasoning titles so we can spot mid‑word character drops quickly.
        // We intentionally do this before inserting so we can attach the
        // composed string alongside the standard order debug info.
        let reasoning_title_dbg: Option<String> = if self.show_order_overlay {
            // CollapsibleReasoningCell shows a collapsed "title" line; extract
            // the first visible line and summarize its raw text/lengths.
            if let Some(rc) = cell
                .as_any()
                .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
            {
                let lines = rc.display_lines_trimmed();
                let first = lines.first();
                if let Some(line) = first {
                    // Collect visible text and basic metrics
                    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                    let bytes = text.len();
                    let chars = text.chars().count();
                    let width = unicode_width::UnicodeWidthStr::width(text.as_str());
                    let spans = line.spans.len();
                    // Per‑span byte lengths to catch odd splits inside words
                    let span_lens: Vec<usize> =
                        line.spans.iter().map(|s| s.content.len()).collect();
                    // Truncate preview to avoid overflow in narrow panes
                    let mut preview = text;
                    // Truncate preview by display width, not bytes, to avoid splitting
                    // a multi-byte character at an invalid boundary.
                    {
                        use unicode_width::UnicodeWidthStr as _;
                        let maxw = 120usize;
                        if preview.width() > maxw {
                            preview = format!(
                                "{}…",
                                crate::live_wrap::take_prefix_by_width(
                                    &preview,
                                    maxw.saturating_sub(1)
                                )
                                .0
                            );
                        }
                    }
                    Some(format!(
                        "title='{preview}' bytes={bytes} chars={chars} width={width} spans={spans} span_bytes={span_lens:?}"
                    ))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let mut cell = cell;

        let mutation = if let Some(domain_record) = record {
            let record_index = if pos == self.history_cells.len() {
                self.history_state.records.len()
            } else {
                self.record_index_for_position(pos)
            };
            let event = match domain_record {
                HistoryDomainRecord::Exec(ref exec_record) => {
                    HistoryDomainEvent::StartExec {
                        index: record_index,
                        call_id: exec_record.call_id.clone(),
                        command: exec_record.command.clone(),
                        parsed: exec_record.parsed.clone(),
                        action: exec_record.action,
                        started_at: exec_record.started_at,
                        working_dir: exec_record.working_dir.clone(),
                        env: exec_record.env.clone(),
                        tags: exec_record.tags.clone(),
                    }
                }
                other => HistoryDomainEvent::Insert {
                    index: record_index,
                    record: other,
                },
            };
            Some(self.history_state.apply_domain_event(event))
        } else if let Some(record) = history_cell::record_from_cell(cell.as_ref()) {
            let record_index = if pos == self.history_cells.len() {
                self.history_state.records.len()
            } else {
                self.record_index_for_position(pos)
            };
            let event = match HistoryDomainRecord::from(record) {
                HistoryDomainRecord::Exec(exec_record) => HistoryDomainEvent::StartExec {
                    index: record_index,
                    call_id: exec_record.call_id.clone(),
                    command: exec_record.command.clone(),
                    parsed: exec_record.parsed.clone(),
                    action: exec_record.action,
                    started_at: exec_record.started_at,
                    working_dir: exec_record.working_dir.clone(),
                    env: exec_record.env.clone(),
                    tags: exec_record.tags,
                },
                other => HistoryDomainEvent::Insert {
                    index: record_index,
                    record: other,
                },
            };
            Some(self.history_state.apply_domain_event(event))
        } else {
            None
        };

        let mut maybe_id = None;
        if let Some(mutation) = mutation
            && let Some(id) = self.apply_mutation_to_cell(&mut cell, mutation) {
                maybe_id = Some(id);
            }

        let append = pos == self.history_cells.len();
        if !append {
            self.history_prefix_append_only.set(false);
        }
        if append {
            self.history_cells.push(cell);
            self.history_cell_ids.push(maybe_id);
        } else {
            self.history_cells.insert(pos, cell);
            self.history_cell_ids.insert(pos, maybe_id);
        }
        // In terminal mode, App mirrors history lines into the native buffer.
        // Ensure order vector is also long enough for position after cell insert
        if self.cell_order_seq.len() < pos {
            self.cell_order_seq.resize(
                pos,
                OrderKey {
                    req: 0,
                    out: -1,
                    seq: 0,
                },
            );
        }
        if append {
            self.cell_order_seq.push(key);
        } else {
            self.cell_order_seq.insert(pos, key);
        }
        if key_bumped
            && let Some(stream) = self.history_cells[pos]
                .as_any()
                .downcast_ref::<crate::history_cell::StreamingContentCell>()
            {
                self.stream_order_seq
                    .insert((StreamKind::Answer, stream.state().stream_id.clone()), key);
            }
        self.last_assigned_order = Some(match self.last_assigned_order {
            Some(prev) => prev.max(key),
            None => key,
        });
        // Insert debug info aligned with cell insert
        let ordered = "ordered";
        let req_dbg = format!("{}", key.req);
        let dbg = if let Some(tdbg) = reasoning_title_dbg {
            format!(
                "insert: {} req={} key={} {} pos={} tag={} | {}",
                ordered,
                req_dbg,
                0,
                Self::debug_fmt_order_key(key),
                pos,
                tag,
                tdbg
            )
        } else {
            format!(
                "insert: {} req={} {} pos={} tag={}",
                ordered,
                req_dbg,
                Self::debug_fmt_order_key(key),
                pos,
                tag
            )
        };
        if self.cell_order_dbg.len() < pos {
            self.cell_order_dbg.resize(pos, None);
        }
        if append {
            self.cell_order_dbg.push(Some(dbg));
        } else {
            self.cell_order_dbg.insert(pos, Some(dbg));
        }
        if let Some(id) = maybe_id {
            if id != HistoryId::ZERO {
                self.history_render.invalidate_history_id(id);
            } else {
                self.history_render.invalidate_prefix_only();
            }
        } else {
            self.history_render.invalidate_prefix_only();
        }
        self.mark_render_requests_dirty();
        self.autoscroll_if_near_bottom();
        self.bottom_pane.set_has_chat_history(true);
        self.process_animation_cleanup();
        // Maintain input focus when new history arrives unless a modal overlay owns it
        if !self.agents_terminal.active {
            self.bottom_pane.ensure_input_focus();
        }
        self.app_event_tx.send(AppEvent::RequestRedraw);
        self.refresh_explore_trailing_flags();
        self.refresh_reasoning_collapsed_visibility();
        self.mark_history_dirty();
        pos
    }

    pub(super) fn history_insert_existing_record(
        &mut self,
        mut cell: Box<dyn HistoryCell>,
        mut key: OrderKey,
        tag: &'static str,
        id: HistoryId,
    ) -> usize {
        #[cfg(debug_assertions)]
        {
            let cell_kind = cell.kind();
            if cell_kind == HistoryCellType::BackgroundEvent {
                debug_assert!(
                    tag == "background",
                    "Background events must use the background helper (tag={tag})"
                );
            }
        }

        let is_reasoning_cell = cell
            .as_any()
            .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
            .is_some();
        if !is_reasoning_cell {
                self.clear_reasoning_in_progress();
        }

        let is_background_cell = matches!(cell.kind(), HistoryCellType::BackgroundEvent);
        let mut key_bumped = false;
        if !is_background_cell
            && let Some(last) = self.last_assigned_order
                && key <= last {
                    key = Self::order_key_successor(last);
                    key_bumped = true;
                }

        let mut pos = self.history_cells.len();
        if is_background_cell || self.last_assigned_order.is_none() {
            for i in 0..self.history_cells.len() {
                if let Some(existing) = self.cell_order_seq.get(i)
                    && *existing > key {
                        pos = i;
                        break;
                    }
            }
        }

        if self.cell_order_seq.len() < self.history_cells.len() {
            let missing = self.history_cells.len() - self.cell_order_seq.len();
            for _ in 0..missing {
                self.cell_order_seq.push(OrderKey {
                    req: 0,
                    out: -1,
                    seq: 0,
                });
            }
        }

        tracing::info!(
            "[order] insert(existing): {} pos={} len_before={} order_len_before={} tag={}",
            Self::debug_fmt_order_key(key),
            pos,
            self.history_cells.len(),
            self.cell_order_seq.len(),
            tag
        );

        let reasoning_title_dbg: Option<String> = if self.show_order_overlay {
            if let Some(rc) = cell
                .as_any()
                .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
            {
                let lines = rc.display_lines_trimmed();
                if let Some(line) = lines.first() {
                    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                    let bytes = text.len();
                    let chars = text.chars().count();
                    let width = unicode_width::UnicodeWidthStr::width(text.as_str());
                    let spans = line.spans.len();
                    let span_lens: Vec<usize> =
                        line.spans.iter().map(|s| s.content.len()).collect();
                    let mut preview = text;
                    {
                        use unicode_width::UnicodeWidthStr as _;
                        let maxw = 120usize;
                        if preview.width() > maxw {
                            preview = format!(
                                "{}…",
                                crate::live_wrap::take_prefix_by_width(
                                    &preview,
                                    maxw.saturating_sub(1)
                                )
                                .0
                            );
                        }
                    }
                    Some(format!(
                        "title='{preview}' bytes={bytes} chars={chars} width={width} spans={spans} span_bytes={span_lens:?}"
                    ))
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        Self::assign_history_id_inner(&mut cell, id);

        let append = pos == self.history_cells.len();
        if !append {
            self.history_prefix_append_only.set(false);
        }
        if append {
            self.history_cells.push(cell);
            self.history_cell_ids.push(Some(id));
        } else {
            self.history_cells.insert(pos, cell);
            self.history_cell_ids.insert(pos, Some(id));
        }
        if self.cell_order_seq.len() < pos {
            self.cell_order_seq.resize(
                pos,
                OrderKey {
                    req: 0,
                    out: -1,
                    seq: 0,
                },
            );
        }
        if append {
            self.cell_order_seq.push(key);
        } else {
            self.cell_order_seq.insert(pos, key);
        }
        if key_bumped
            && let Some(stream) = self.history_cells[pos]
                .as_any()
                .downcast_ref::<crate::history_cell::StreamingContentCell>()
            {
                self.stream_order_seq
                    .insert((StreamKind::Answer, stream.state().stream_id.clone()), key);
            }
        self.last_assigned_order = Some(match self.last_assigned_order {
            Some(prev) => prev.max(key),
            None => key,
        });

        let ordered = "existing";
        let req_dbg = format!("{}", key.req);
        let dbg = if let Some(tdbg) = reasoning_title_dbg {
            format!(
                "insert: {} req={} {} pos={} tag={} | {}",
                ordered,
                req_dbg,
                Self::debug_fmt_order_key(key),
                pos,
                tag,
                tdbg
            )
        } else {
            format!(
                "insert: {} req={} {} pos={} tag={}",
                ordered,
                req_dbg,
                Self::debug_fmt_order_key(key),
                pos,
                tag
            )
        };
        if self.cell_order_dbg.len() < pos {
            self.cell_order_dbg.resize(pos, None);
        }
        if append {
            self.cell_order_dbg.push(Some(dbg));
        } else {
            self.cell_order_dbg.insert(pos, Some(dbg));
        }
        self.history_render.invalidate_history_id(id);
        self.mark_render_requests_dirty();
        self.autoscroll_if_near_bottom();
        self.bottom_pane.set_has_chat_history(true);
        self.process_animation_cleanup();
        if !self.agents_terminal.active {
            self.bottom_pane.ensure_input_focus();
        }
        self.app_event_tx.send(AppEvent::RequestRedraw);
        self.refresh_explore_trailing_flags();
        self.refresh_reasoning_collapsed_visibility();
        self.mark_history_dirty();
        pos
    }

    pub(super) fn append_wait_pairs(target: &mut Vec<(String, bool)>, additions: &[(String, bool)]) {
        for (text, is_error) in additions {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if target
                .last()
                .map(|(existing, existing_err)| existing == trimmed && *existing_err == *is_error)
                .unwrap_or(false)
            {
                continue;
            }
            target.push((trimmed.to_string(), *is_error));
        }
    }

    pub(super) fn wait_pairs_from_exec_notes(notes: &[ExecWaitNote]) -> Vec<(String, bool)> {
        notes
            .iter()
            .map(|note| {
                (
                    note.message.clone(),
                    matches!(note.tone, TextTone::Error),
                )
            })
            .collect()
    }

    pub(super) fn update_exec_wait_state_with_pairs(
        &mut self,
        history_id: HistoryId,
        total_wait: Option<Duration>,
        wait_active: bool,
        notes: &[(String, bool)],
    ) -> bool {
        let Some(record_idx) = self.history_state.index_of(history_id) else {
            return false;
        };
        let note_records: Vec<ExecWaitNote> = notes
            .iter()
            .filter_map(|(text, is_error)| {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(ExecWaitNote {
                        message: trimmed.to_string(),
                        tone: if *is_error {
                            TextTone::Error
                        } else {
                            TextTone::Info
                        },
                        timestamp: SystemTime::now(),
                    })
                }
            })
            .collect();
        let mutation = self.history_state.apply_domain_event(HistoryDomainEvent::UpdateExecWait {
            index: record_idx,
            total_wait,
            wait_active,
            notes: note_records,
        });
        match mutation {
            HistoryMutation::Replaced {
                id,
                record: HistoryRecord::Exec(exec_record),
                ..
            }
            | HistoryMutation::Inserted {
                id,
                record: HistoryRecord::Exec(exec_record),
                ..
            } => {
                self.update_cell_from_record(id, HistoryRecord::Exec(exec_record));
                self.mark_history_dirty();
                true
            }
            _ => false,
        }
    }

    pub(super) fn merge_tool_arguments(existing: &mut Vec<ToolArgument>, updates: Vec<ToolArgument>) {
        for update in updates {
            if let Some(existing_arg) = existing.iter_mut().find(|arg| arg.name == update.name) {
                *existing_arg = update;
            } else {
                existing.push(update);
            }
        }
    }

    pub(super) fn apply_custom_tool_update(
        &mut self,
        call_id: &str,
        parameters: Option<serde_json::Value>,
    ) {
        let Some(params) = parameters else {
            return;
        };
        let updates = history_cell::arguments_from_json(&params);
        if updates.is_empty() {
            return;
        }

        let running_entry = self
            .tools_state
            .running_custom_tools
            .get(&ToolCallId(call_id.to_string()))
            .copied();
        let resolved_idx = running_entry
            .as_ref()
            .and_then(|entry| running_tools::resolve_entry_index(self, entry, call_id))
            .or_else(|| running_tools::find_by_call_id(self, call_id));

        let Some(idx) = resolved_idx else {
            return;
        };
        if idx >= self.history_cells.len() {
            return;
        }
        let Some(running_cell) = self.history_cells[idx]
            .as_any()
            .downcast_ref::<history_cell::RunningToolCallCell>()
        else {
            return;
        };

        let mut state = running_cell.state().clone();
        Self::merge_tool_arguments(&mut state.arguments, updates);
        let mut updated_cell = history_cell::RunningToolCallCell::from_state(state.clone());
        updated_cell.state_mut().call_id = Some(call_id.to_string());
        self.history_replace_with_record(
            idx,
            Box::new(updated_cell),
            HistoryDomainRecord::from(state),
        );
    }

    pub(super) fn hydrate_cell_from_record(
        &self,
        cell: &mut Box<dyn HistoryCell>,
        record: &HistoryRecord,
    ) -> bool {
        Self::hydrate_cell_from_record_inner(cell, record, &self.config)
    }

    pub(super) fn hydrate_cell_from_record_inner(
        cell: &mut Box<dyn HistoryCell>,
        record: &HistoryRecord,
        config: &Config,
    ) -> bool {
        match record {
            HistoryRecord::PlainMessage(state) => {
                if let Some(plain) = cell
                    .as_any_mut()
                    .downcast_mut::<crate::history_cell::PlainHistoryCell>()
                {
                    *plain.state_mut() = state.clone();
                    return true;
                }
            }
            HistoryRecord::WaitStatus(state) => {
                if let Some(wait) = cell
                    .as_any_mut()
                    .downcast_mut::<crate::history_cell::WaitStatusCell>()
                {
                    *wait.state_mut() = state.clone();
                    return true;
                }
            }
            HistoryRecord::Loading(state) => {
                if let Some(loading) = cell
                    .as_any_mut()
                    .downcast_mut::<crate::history_cell::LoadingCell>()
                {
                    *loading.state_mut() = state.clone();
                    return true;
                }
            }
            HistoryRecord::BackgroundEvent(state) => {
                if let Some(background) = cell
                    .as_any_mut()
                    .downcast_mut::<crate::history_cell::BackgroundEventCell>()
                {
                    *background.state_mut() = state.clone();
                    return true;
                }
            }
            HistoryRecord::Exec(state) => {
                if let Some(exec) = cell
                    .as_any_mut()
                    .downcast_mut::<crate::history_cell::ExecCell>()
                {
                    exec.sync_from_record(state);
                    return true;
                }
            }
            HistoryRecord::AssistantStream(state) => {
                if let Some(stream) = cell
                    .as_any_mut()
                    .downcast_mut::<crate::history_cell::StreamingContentCell>()
                {
                    stream.set_state(state.clone());
                    stream.update_context(config.file_opener, &config.cwd);
                    return true;
                }
            }
            HistoryRecord::RateLimits(state) => {
                if let Some(rate_limits) = cell
                    .as_any_mut()
                    .downcast_mut::<crate::history_cell::RateLimitsCell>()
                {
                    *rate_limits.record_mut() = state.clone();
                    return true;
                }
            }
            HistoryRecord::Patch(state) => {
                if let Some(patch) = cell
                    .as_any_mut()
                    .downcast_mut::<crate::history_cell::PatchSummaryCell>()
                {
                    patch.update_record(state.clone());
                    return true;
                }
            }
            HistoryRecord::Image(state) => {
                if let Some(image) = cell
                    .as_any_mut()
                    .downcast_mut::<crate::history_cell::ImageOutputCell>()
                {
                    *image.record_mut() = state.clone();
                    return true;
                }
            }
            HistoryRecord::Context(state) => {
                if let Some(context) = cell
                    .as_any_mut()
                    .downcast_mut::<crate::history_cell::ContextCell>()
                {
                    context.update(state.clone());
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    pub(super) fn build_cell_from_record(&self, record: &HistoryRecord) -> Option<Box<dyn HistoryCell>> {
        use crate::history_cell;

        match record {
            HistoryRecord::PlainMessage(state) => Some(Box::new(
                history_cell::PlainHistoryCell::from_state(state.clone()),
            )),
            HistoryRecord::WaitStatus(state) => {
                Some(Box::new(history_cell::WaitStatusCell::from_state(state.clone())))
            }
            HistoryRecord::Loading(state) => {
                Some(Box::new(history_cell::LoadingCell::from_state(state.clone())))
            }
            HistoryRecord::RunningTool(state) => Some(Box::new(
                history_cell::RunningToolCallCell::from_state(state.clone()),
            )),
            HistoryRecord::ToolCall(state) => Some(Box::new(
                history_cell::ToolCallCell::from_state(state.clone()),
            )),
            HistoryRecord::PlanUpdate(state) => Some(Box::new(
                history_cell::PlanUpdateCell::from_state(state.clone()),
            )),
            HistoryRecord::UpgradeNotice(state) => Some(Box::new(
                history_cell::UpgradeNoticeCell::from_state(state.clone()),
            )),
            HistoryRecord::Reasoning(state) => Some(Box::new(
                history_cell::CollapsibleReasoningCell::from_state(state.clone()),
            )),
            HistoryRecord::Exec(state) => {
                Some(Box::new(history_cell::ExecCell::from_record(state.clone())))
            }
            HistoryRecord::MergedExec(state) => Some(Box::new(
                history_cell::MergedExecCell::from_state(state.clone()),
            )),
            HistoryRecord::AssistantStream(state) => Some(Box::new(
                history_cell::StreamingContentCell::from_state(
                    state.clone(),
                    self.config.file_opener,
                    self.config.cwd.clone(),
                ),
            )),
            HistoryRecord::AssistantMessage(state) => Some(Box::new(
                history_cell::AssistantMarkdownCell::from_state(state.clone(), &self.config),
            )),
            HistoryRecord::Diff(state) => {
                Some(Box::new(history_cell::DiffCell::from_record(state.clone())))
            }
            HistoryRecord::Patch(state) => {
                Some(Box::new(history_cell::PatchSummaryCell::from_record(state.clone())))
            }
            HistoryRecord::Explore(state) => {
                Some(Box::new(history_cell::ExploreAggregationCell::from_record(state.clone())))
            }
            HistoryRecord::RateLimits(state) => Some(Box::new(
                history_cell::RateLimitsCell::from_record(state.clone()),
            )),
            HistoryRecord::BackgroundEvent(state) => {
                Some(Box::new(history_cell::BackgroundEventCell::new(state.clone())))
            }
            HistoryRecord::Image(state) => {
                let cell = history_cell::ImageOutputCell::from_record(state.clone());
                self.ensure_image_cell_picker(&cell);
                Some(Box::new(cell))
            }
            HistoryRecord::Context(state) => Some(Box::new(
                history_cell::ContextCell::new(state.clone()),
            )),
            HistoryRecord::Notice(state) => Some(Box::new(
                history_cell::PlainHistoryCell::from_notice_record(state.clone()),
            )),
        }
    }

    pub(super) fn apply_mutation_to_cell(
        &self,
        cell: &mut Box<dyn HistoryCell>,
        mutation: HistoryMutation,
    ) -> Option<HistoryId> {
        match mutation {
            HistoryMutation::Inserted { id, record, .. }
            | HistoryMutation::Replaced { id, record, .. } => {
                if let Some(mut new_cell) = self.build_cell_from_record(&record) {
                    self.assign_history_id(&mut new_cell, id);
                    *cell = new_cell;
                } else if !self.hydrate_cell_from_record(cell, &record) {
                    self.assign_history_id(cell, id);
                }
                Some(id)
            }
            _ => None,
        }
    }

    pub(super) fn apply_mutation_to_cell_index(
        &mut self,
        idx: usize,
        mutation: HistoryMutation,
    ) -> Option<HistoryId> {
        if idx >= self.history_cells.len() {
            return None;
        }
        match mutation {
            HistoryMutation::Inserted { id, record, .. }
            | HistoryMutation::Replaced { id, record, .. } => {
                self.update_cell_from_record(id, record);
                Some(id)
            }
            _ => None,
        }
    }

    pub(super) fn cell_index_for_history_id(&self, id: HistoryId) -> Option<usize> {
        if let Some(idx) = self
            .history_cell_ids
            .iter()
            .position(|maybe| maybe.map(|stored| stored == id).unwrap_or(false))
        {
            return Some(idx);
        }

        self.history_cells.iter().enumerate().find_map(|(idx, cell)| {
        history_cell::record_from_cell(cell.as_ref())
                .map(|record| record.id() == id)
                .filter(|matched| *matched)
                .map(|_| idx)
        })
    }

    pub(super) fn update_cell_from_record(&mut self, id: HistoryId, record: HistoryRecord) {
        if id == HistoryId::ZERO {
            tracing::debug!("skip update_cell_from_record: zero id");
            return;
        }

        self.history_render.invalidate_history_id(id);

        if let Some(idx) = self.cell_index_for_history_id(id) {
            if let Some(mut rebuilt) = self.build_cell_from_record(&record) {
                Self::assign_history_id_inner(&mut rebuilt, id);
                self.history_cells[idx] = rebuilt;
            } else if let Some(cell_slot) = self.history_cells.get_mut(idx)
                && !Self::hydrate_cell_from_record_inner(cell_slot, &record, &self.config) {
                    Self::assign_history_id_inner(cell_slot, id);
                }

            if idx < self.history_cell_ids.len() {
                self.history_cell_ids[idx] = Some(id);
            }
            self.invalidate_height_cache();
            self.request_redraw();
        } else {
            tracing::warn!(
                "history-state mismatch: unable to locate cell for id {:?}",
                id
            );
        }
    }

    pub(super) fn assign_history_id(&self, cell: &mut Box<dyn HistoryCell>, id: HistoryId) {
        Self::assign_history_id_inner(cell, id);
    }

    pub(super) fn assign_history_id_inner(cell: &mut Box<dyn HistoryCell>, id: HistoryId) {
        if let Some(tool_call) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::ToolCallCell>()
        {
            tool_call.state_mut().id = id;
        } else if let Some(running_tool) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::RunningToolCallCell>()
        {
            running_tool.state_mut().id = id;
        } else if let Some(plan) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::PlanUpdateCell>()
        {
            plan.state_mut().id = id;
        } else if let Some(upgrade) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::UpgradeNoticeCell>()
        {
            upgrade.state_mut().id = id;
        } else if let Some(reasoning) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::CollapsibleReasoningCell>()
        {
            reasoning.set_history_id(id);
        } else if let Some(exec) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::ExecCell>()
        {
            exec.record.id = id;
        } else if let Some(merged) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::MergedExecCell>()
        {
            merged.set_history_id(id);
        } else if let Some(stream) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::StreamingContentCell>()
        {
            stream.state_mut().id = id;
        } else if let Some(assistant) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::AssistantMarkdownCell>()
        {
            assistant.state_mut().id = id;
        } else if let Some(diff) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::DiffCell>()
        {
            diff.record_mut().id = id;
        } else if let Some(image) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::ImageOutputCell>()
        {
            image.record_mut().id = id;
        } else if let Some(patch) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::PatchSummaryCell>()
        {
            patch.record_mut().id = id;
        } else if let Some(explore) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::ExploreAggregationCell>()
        {
            explore.record_mut().id = id;
        } else if let Some(rate_limits) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::RateLimitsCell>()
        {
            rate_limits.record_mut().id = id;
        } else if let Some(plain) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::PlainHistoryCell>()
        {
            plain.state_mut().id = id;
        } else if let Some(wait) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::WaitStatusCell>()
        {
            wait.state_mut().id = id;
        } else if let Some(loading) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::LoadingCell>()
        {
            loading.state_mut().id = id;
        } else if let Some(background) = cell
            .as_any_mut()
            .downcast_mut::<crate::history_cell::BackgroundEventCell>()
        {
            background.state_mut().id = id;
        }
    }

    /// Push a cell using a synthetic global order key at the bottom of the current request.
    pub(crate) fn history_push(&mut self, cell: impl HistoryCell + 'static) {
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                cell.kind() != HistoryCellType::BackgroundEvent,
                "Background events must use push_background_* helpers"
            );
        }
        let key = self.next_internal_key();
        let _ = self.history_insert_with_key_global_tagged(Box::new(cell), key, "epilogue", None);
    }

    pub(super) fn history_insert_plain_state_with_key(
        &mut self,
        state: PlainMessageState,
        key: OrderKey,
        tag: &'static str,
    ) -> usize {
        let cell = crate::history_cell::PlainHistoryCell::from_state(state.clone());
        self.history_insert_with_key_global_tagged(
            Box::new(cell),
            key,
            tag,
            Some(HistoryDomainRecord::Plain(state)),
        )
    }

    pub(crate) fn history_push_plain_state(&mut self, state: PlainMessageState) {
        let key = self.next_internal_key();
        let _ = self.history_insert_plain_state_with_key(state, key, "epilogue");
    }

    pub(super) fn history_push_plain_paragraphs<I, S>(
        &mut self,
        kind: PlainMessageKind,
        lines: I,
    ) where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let role = history_cell::plain_role_for_kind(kind);
        let state = history_cell::plain_message_state_from_paragraphs(kind, role, lines);
        self.history_push_plain_state(state);
    }

    pub(super) fn history_push_diff(&mut self, title: Option<String>, diff_output: String) {
        let record = history_cell::diff_record_from_string(
            title.unwrap_or_default(),
            &diff_output,
        );
        let key = self.next_internal_key();
        let _ = self.history_insert_with_key_global_tagged(
            Box::new(history_cell::DiffCell::from_record(record.clone())),
            key,
            "diff",
            Some(HistoryDomainRecord::Diff(record)),
        );
    }
    /// Insert a background event near the top of the current request so it appears
    /// before imminent provider output (e.g. Exec begin).
    pub(crate) fn insert_background_event_early(&mut self, message: String) {
        let ticket = self.make_background_before_next_output_ticket();
        self.insert_background_event_with_placement(
            message,
            BackgroundPlacement::BeforeNextOutput,
            Some(ticket.next_order()),
        );
    }
    /// Insert a background event using the specified placement semantics.
    pub(crate) fn insert_background_event_with_placement(
        &mut self,
        message: String,
        placement: BackgroundPlacement,
        order: Option<code_core::protocol::OrderMeta>,
    ) {
        if order.is_none() {
            if matches!(placement, BackgroundPlacement::Tail) {
                tracing::error!(
                    target: "code_order",
                    "missing order metadata for tail background event; dropping message"
                );
                return;
            } else {
                tracing::warn!(
                    target: "code_order",
                    "background event without order metadata placement={:?}",
                    placement
                );
            }
        }
        let system_placement = match placement {
            BackgroundPlacement::Tail => SystemPlacement::Tail,
            BackgroundPlacement::BeforeNextOutput => {
                if self.pending_user_prompts_for_next_turn > 0 {
                    SystemPlacement::Early
                } else {
                    SystemPlacement::PrePrompt
                }
            }
        };
        let cell = history_cell::new_background_event(message);
        let record = HistoryDomainRecord::BackgroundEvent(cell.state().clone());
        self.push_system_cell(
            Box::new(cell),
            system_placement,
            None,
            order.as_ref(),
            "background",
            Some(record),
        );
    }

    pub(crate) fn push_background_tail(&mut self, message: impl Into<String>) {
        let ticket = self.make_background_tail_ticket();
        self.insert_background_event_with_placement(
            message.into(),
            BackgroundPlacement::Tail,
            Some(ticket.next_order()),
        );
    }

    pub(crate) fn push_background_before_next_output(&mut self, message: impl Into<String>) {
        let ticket = self.make_background_before_next_output_ticket();
        self.insert_background_event_with_placement(
            message.into(),
            BackgroundPlacement::BeforeNextOutput,
            Some(ticket.next_order()),
        );
    }

    pub(super) fn history_debug(&self, message: impl Into<String>) {
        if !history_cell_logging_enabled() {
            return;
        }
        let message = message.into();
        tracing::trace!(target: "code_history", "{message}");
        if let Some(buffer) = &self.history_debug_events {
            buffer.borrow_mut().push(message);
        }
    }

    pub(super) fn rehydrate_system_order_cache(&mut self, preserved: &[(String, HistoryId)]) {
        let prev = self.system_cell_by_id.len();
        self.system_cell_by_id.clear();

        for (key, hid) in preserved {
            if let Some(idx) = self
                .history_cell_ids
                .iter()
                .position(|maybe| maybe.map(|stored| stored == *hid).unwrap_or(false))
            {
                self.system_cell_by_id.insert(key.clone(), idx);
            }
        }

        self.history_debug(format!(
            "system_order_cache.rehydrate prev={} restored={} entries={}",
            prev,
            preserved.len(),
            self.system_cell_by_id.len()
        ));
    }

    /// Push a cell using a synthetic key at the TOP of the NEXT request.
    pub(super) fn history_push_top_next_req(&mut self, cell: impl HistoryCell + 'static) {
        let key = self.next_req_key_top();
        let _ = self.history_insert_with_key_global_tagged(Box::new(cell), key, "prelude", None);
    }
    pub(super) fn history_replace_with_record(
        &mut self,
        idx: usize,
        mut cell: Box<dyn HistoryCell>,
        record: HistoryDomainRecord,
    ) {
        if idx >= self.history_cells.len() {
            return;
        }

        let record_idx = self
            .record_index_for_cell(idx)
            .unwrap_or_else(|| self.record_index_for_position(idx));

        let mutation = self.history_state.apply_domain_event(HistoryDomainEvent::Replace {
            index: record_idx,
            record,
        });

        if let Some(id) = self.apply_mutation_to_cell(&mut cell, mutation)
            && idx < self.history_cell_ids.len() {
                self.history_cell_ids[idx] = Some(id);
            }

        self.ensure_image_cell_picker(cell.as_ref());
        self.history_cells[idx] = cell;
        self.invalidate_height_cache();
        self.request_redraw();
        self.refresh_explore_trailing_flags();
        self.mark_history_dirty();
    }

    pub(super) fn history_replace_at(&mut self, idx: usize, mut cell: Box<dyn HistoryCell>) {
        if idx >= self.history_cells.len() {
            return;
        }

        let old_id = self.history_cell_ids.get(idx).and_then(|id| *id);
        let record = history_cell::record_from_cell(cell.as_ref());
        let mut maybe_id = None;

        match (record.map(HistoryDomainRecord::from), self.record_index_for_cell(idx)) {
            (Some(record), Some(record_idx)) => {
                let mutation = self
                    .history_state
                    .apply_domain_event(HistoryDomainEvent::Replace {
                        index: record_idx,
                        record,
                    });
                if let Some(id) = self.apply_mutation_to_cell(&mut cell, mutation) {
                    maybe_id = Some(id);
                }
            }
            (Some(record), None) => {
                let record_idx = self.record_index_for_position(idx);
                let mutation = self
                    .history_state
                    .apply_domain_event(HistoryDomainEvent::Insert {
                        index: record_idx,
                        record,
                    });
                if let Some(id) = self.apply_mutation_to_cell(&mut cell, mutation) {
                    maybe_id = Some(id);
                }
            }
            (None, Some(record_idx)) => {
                let _ = self
                    .history_state
                    .apply_domain_event(HistoryDomainEvent::Remove { index: record_idx });
            }
            (None, None) => {}
        }

        self.ensure_image_cell_picker(cell.as_ref());
        self.history_cells[idx] = cell;
        if idx < self.history_cell_ids.len() {
            self.history_cell_ids[idx] = maybe_id;
        }
        if let Some(id) = old_id {
            self.history_render.invalidate_history_id(id);
        } else {
            self.history_render.invalidate_prefix_only();
        }
        if let Some(id) = maybe_id
            && Some(id) != old_id {
                self.history_render.invalidate_history_id(id);
            }
        self.mark_render_requests_dirty();
        self.history_prefix_append_only.set(false);
        self.request_redraw();
        self.refresh_explore_trailing_flags();
        // Keep debug info for this cell index as-is.
        self.mark_history_dirty();
    }

    pub(super) fn history_remove_at(&mut self, idx: usize) {
        if idx >= self.history_cells.len() {
            return;
        }

        let removed_id = self.history_cell_ids.get(idx).and_then(|id| *id);
        if let Some(record_idx) = self.record_index_for_cell(idx) {
            let _ = self
                .history_state
                .apply_domain_event(HistoryDomainEvent::Remove { index: record_idx });
        }

        self.history_cells.remove(idx);
        if idx < self.history_cell_ids.len() {
            self.history_cell_ids.remove(idx);
        }
        if idx < self.cell_order_seq.len() {
            self.cell_order_seq.remove(idx);
        }
        if idx < self.cell_order_dbg.len() {
            self.cell_order_dbg.remove(idx);
        }
        if let Some(id) = removed_id {
            self.history_render.invalidate_history_id(id);
        } else {
            self.history_render.invalidate_prefix_only();
        }
        self.mark_render_requests_dirty();
        self.history_prefix_append_only.set(false);
        self.request_redraw();
        self.refresh_explore_trailing_flags();
        self.mark_history_dirty();
    }

    pub(super) fn history_replace_and_maybe_merge(&mut self, idx: usize, cell: Box<dyn HistoryCell>) {
        // Replace at index, then attempt standard exec merge with previous cell.
        self.history_replace_at(idx, cell);
        // Merge only if the new cell is an Exec with output (completed) or a MergedExec.
        crate::chatwidget::exec_tools::try_merge_completed_exec_at(self, idx);
    }

    // Merge adjacent tool cells with the same header (e.g., successive Web Search blocks)
    #[allow(dead_code)]
    pub(super) fn history_maybe_merge_tool_with_previous(&mut self, idx: usize) {
        if idx == 0 || idx >= self.history_cells.len() {
            return;
        }
        let new_lines = self.history_cells[idx].display_lines();
        let new_header = new_lines
            .first()
            .and_then(|l| l.spans.first())
            .map(|s| s.content.clone().to_string())
            .unwrap_or_default();
        if new_header.is_empty() {
            return;
        }
        let prev_lines = self.history_cells[idx - 1].display_lines();
        let prev_header = prev_lines
            .first()
            .and_then(|l| l.spans.first())
            .map(|s| s.content.clone().to_string())
            .unwrap_or_default();
        if new_header != prev_header {
            return;
        }
        let mut combined = prev_lines.clone();
        while combined
            .last()
            .map(|l| crate::render::line_utils::is_blank_line_trim(l))
            .unwrap_or(false)
        {
            combined.pop();
        }
        let mut body: Vec<ratatui::text::Line<'static>> = new_lines.into_iter().skip(1).collect();
        while body
            .first()
            .map(|l| crate::render::line_utils::is_blank_line_trim(l))
            .unwrap_or(false)
        {
            body.remove(0);
        }
        while body
            .last()
            .map(|l| crate::render::line_utils::is_blank_line_trim(l))
            .unwrap_or(false)
        {
            body.pop();
        }
        if let Some(first_line) = body.first_mut()
            && let Some(first_span) = first_line.spans.get_mut(0)
                && (first_span.content == "  └ " || first_span.content == "└ ") {
                    first_span.content = "  ".into();
                }
        combined.extend(body);
        let state = history_cell::plain_message_state_from_lines(
            combined,
            crate::history_cell::HistoryCellType::Plain,
        );
        self.history_replace_with_record(
            idx - 1,
            Box::new(crate::history_cell::PlainHistoryCell::from_state(state.clone())),
            HistoryDomainRecord::Plain(state),
        );
        self.history_remove_at(idx);
    }

    pub(super) fn record_index_for_position(&self, ui_index: usize) -> usize {
        if let Some(Some(id)) = self.history_cell_ids.get(ui_index)
            && let Some(idx) = self.history_state.index_of(*id) {
                return idx;
            }
        self.history_cell_ids
            .iter()
            .take(ui_index)
            .filter(|entry| entry.is_some())
            .count()
    }

    pub(super) fn record_index_for_cell(&self, idx: usize) -> Option<usize> {
        self.history_cell_ids
            .get(idx)
            .and_then(|entry| entry.map(|_| self.record_index_for_position(idx)))
    }

    /// Clean up faded-out animation cells
    pub(super) fn process_animation_cleanup(&mut self) {
        // With trait-based cells, we can't easily detect and clean up specific cell types
        // Animation cleanup is now handled differently
    }

    pub(super) fn refresh_explore_trailing_flags(&mut self) -> bool {
        let mut updated = false;
        for idx in 0..self.history_cells.len() {
            let is_explore = self.history_cells[idx]
                .as_any()
                .downcast_ref::<history_cell::ExploreAggregationCell>()
                .is_some();
            if !is_explore {
                continue;
            }

            let hold_title = self.rendered_explore_should_hold(idx);

            if let Some(explore_cell) = self.history_cells[idx]
                .as_any_mut()
                .downcast_mut::<history_cell::ExploreAggregationCell>()
                && explore_cell.set_force_exploring_header(hold_title) {
                    updated = true;
                    if let Some(Some(id)) = self.history_cell_ids.get(idx) {
                        self.history_render.invalidate_history_id(*id);
                    }
                }
        }

        if updated {
            self.invalidate_height_cache();
            self.request_redraw();
        }

        updated
    }

    pub(super) fn rendered_explore_should_hold(&self, idx: usize) -> bool {
        if idx >= self.history_cells.len() {
            return true;
        }

        let mut next = idx + 1;
        while next < self.history_cells.len() {
            let cell = &self.history_cells[next];

            if cell.should_remove() {
                next += 1;
                continue;
            }

            match cell.kind() {
                history_cell::HistoryCellType::Reasoning
                | history_cell::HistoryCellType::Loading
                | history_cell::HistoryCellType::PlanUpdate => {
                    next += 1;
                    continue;
                }
                _ => {}
            }

            if cell
                .as_any()
                .downcast_ref::<history_cell::WaitStatusCell>()
                .is_some()
            {
                next += 1;
                continue;
            }

            if self.cell_lines_trimmed_is_empty(next, cell.as_ref()) {
                next += 1;
                continue;
            }

            return false;
        }

        true
    }

    pub(super) fn try_coordinator_route(
        &mut self,
        original_text: &str,
    ) -> Option<CoordinatorRouterResponse> {
        let trimmed = original_text.trim();
        if trimmed.is_empty() {
            return None;
        }
        if !self.auto_state.is_active() {
            return None;
        }
        if self.auto_state.is_paused_manual()
            && self.auto_state.should_bypass_coordinator_next_submit()
        {
            return None;
        }
        if !self.config.auto_drive.coordinator_routing {
            return None;
        }
        if trimmed.starts_with('/') {
            return None;
        }

        let mut updates = Vec::new();
        if let Some(summary) = self.auto_state.last_decision_summary.clone()
            && !summary.trim().is_empty() {
                updates.push(summary);
            }
        if let Some(current) = self.auto_state.current_summary.clone()
            && !current.trim().is_empty() && updates.iter().all(|existing| existing != &current) {
                updates.push(current);
            }

        let context = CoordinatorContext::new(self.auto_state.pending_agent_actions.len(), updates);
        let response = route_user_message(trimmed, &context);
        if response.user_response.is_some() || response.cli_command.is_some() {
            Some(response)
        } else {
            None
        }
    }

    pub(super) fn submit_request_user_input_answer(&mut self, pending: PendingRequestUserInput, raw: String) {
        use code_protocol::request_user_input::RequestUserInputAnswer;
        use code_protocol::request_user_input::RequestUserInputResponse;

        tracing::info!(
            "[request_user_input] answer turn_id={} call_id={}",
            pending.turn_id,
            pending.call_id
        );

        let response = serde_json::from_str::<RequestUserInputResponse>(&raw).unwrap_or_else(|_| {
            let question_count = pending.questions.len();
            let mut lines: Vec<String> = raw
                .lines()
                .map(|line| line.trim_end().to_string())
                .collect();

            if question_count <= 1 {
                lines = vec![raw.trim().to_string()];
            } else if lines.len() > question_count {
                let tail = lines.split_off(question_count - 1);
                lines.push(tail.join("\n"));
            }

            while lines.len() < question_count {
                lines.push(String::new());
            }

            let mut answers = std::collections::HashMap::new();
            for (idx, question) in pending.questions.iter().enumerate() {
                let value = lines.get(idx).cloned().unwrap_or_default();
                answers.insert(
                    question.id.clone(),
                    RequestUserInputAnswer {
                        answers: vec![value],
                    },
                );
            }
            RequestUserInputResponse { answers }
        });

        let display_text =
            Self::format_request_user_input_display(&pending.questions, &response);
        if !display_text.trim().is_empty() {
            let key = Self::order_key_successor(pending.anchor_key);
            let state = history_cell::new_user_prompt(display_text);
            let _ =
                self.history_insert_plain_state_with_key(state, key, "request_user_input_answer");
            self.restore_reasoning_in_progress_if_streaming();
        }

        if let Err(e) = self.code_op_tx.send(Op::UserInputAnswer {
            id: pending.turn_id,
            response,
        }) {
            tracing::error!("failed to send Op::UserInputAnswer: {e}");
        }

        self.clear_composer();
        self.bottom_pane
            .update_status_text("waiting for model".to_string());
        self.request_redraw();
    }

    pub(super) fn format_request_user_input_display(
        questions: &[code_protocol::request_user_input::RequestUserInputQuestion],
        response: &code_protocol::request_user_input::RequestUserInputResponse,
    ) -> String {
        let mut lines = Vec::new();
        for question in questions {
            let answer: &[String] = response
                .answers
                .get(&question.id)
                .map(|a| a.answers.as_slice())
                .unwrap_or(&[]);
            let value = answer.first().map(String::as_str).unwrap_or("");
            let value = if value.trim().is_empty() {
                "(skipped)"
            } else if question.is_secret {
                "[hidden]"
            } else {
                value
            };

            if questions.len() == 1 {
                lines.push(value.to_string());
            } else {
                let header = question.header.trim();
                if header.is_empty() {
                    lines.push(value.to_string());
                } else {
                    lines.push(format!("{header}: {value}"));
                }
            }
        }
        lines.join("\n")
    }

    pub(crate) fn on_request_user_input_answer(
        &mut self,
        turn_id: String,
        response: code_protocol::request_user_input::RequestUserInputResponse,
    ) {
        let Some(pending) = self.pending_request_user_input.take() else {
            tracing::warn!(
                "[request_user_input] received UI answer but no request is pending (turn_id={turn_id})"
            );
            return;
        };

        if pending.turn_id != turn_id {
            tracing::warn!(
                "[request_user_input] received UI answer for unexpected turn_id (expected={}, got={turn_id})",
                pending.turn_id,
            );
        }

        self.bottom_pane.close_request_user_input_view();

        let display_text =
            Self::format_request_user_input_display(&pending.questions, &response);

        if !display_text.trim().is_empty() {
            let key = Self::order_key_successor(pending.anchor_key);
            let state = history_cell::new_user_prompt(display_text);
            let _ =
                self.history_insert_plain_state_with_key(state, key, "request_user_input_answer");
            self.restore_reasoning_in_progress_if_streaming();
        }

        if let Err(e) = self.code_op_tx.send(Op::UserInputAnswer {
            id: pending.turn_id,
            response,
        }) {
            tracing::error!("failed to send Op::UserInputAnswer: {e}");
        }

        self.clear_composer();
        self.bottom_pane
            .update_status_text("waiting for model".to_string());
        self.request_redraw();
    }

    pub(super) fn submit_user_message(&mut self, user_message: UserMessage) {
        if self.layout.scroll_offset.get() > 0 {
            layout_scroll::to_bottom(self);
        }
        // Surface a local diagnostic note and anchor it to the NEXT turn,
        // placing it directly after the user prompt so ordering is stable.
        // (debug message removed)
        // Fade the welcome cell only when a user actually posts a message.
        for cell in &self.history_cells {
            cell.trigger_fade();
        }
        let mut message = user_message;
        // If our configured cwd no longer exists (e.g., a worktree folder was
        // deleted outside the app), try to automatically recover to the repo
        // root for worktrees and re-submit the same message there.
        if !self.config.cwd.exists() {
            let missing = self.config.cwd.clone();
            let mut fallback: Option<(PathBuf, &'static str)> =
                worktree_root_hint_for(&missing).map(|p| (p, "recorded repo root"));
            if fallback.is_none()
                && let Some(parent) = missing.parent().and_then(worktree_root_hint_for) {
                    fallback = Some((parent, "recorded repo root"));
                }
            if fallback.is_none()
                && let Some(prev) = last_existing_cwd(&missing) {
                    fallback = Some((prev, "last known directory"));
                }
            let missing_s = missing.display().to_string();
            if fallback.is_none() && missing_s.contains("/.code/branches/") {
                let mut current = missing.as_path();
                let mut first_existing: Option<PathBuf> = None;
                while let Some(parent) = current.parent() {
                    current = parent;
                    if !current.exists() {
                        continue;
                    }
                    if first_existing.is_none() {
                        first_existing = Some(current.to_path_buf());
                    }
                    if let Some(repo_root) =
                        code_core::git_info::resolve_root_git_project_for_trust(current)
                    {
                        fallback = Some((repo_root, "repository root"));
                        break;
                    }
                }
                if fallback.is_none()
                    && let Some(existing) = first_existing {
                        fallback = Some((existing, "parent directory"));
                    }
            }
            if fallback.is_none()
                && let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
                    fallback = Some((home, "home directory"));
                }
            if let Some((fallback_root, label)) = fallback {
                let msg = format!(
                    "⚠️ Worktree directory is missing: {}\nSwitching to {}: {}",
                    missing.display(),
                    label,
                    fallback_root.display()
                );
                self.send_background_tail_ordered(msg);
                self.app_event_tx.send(AppEvent::SwitchCwd(
                    fallback_root,
                    Some(message.display_text.clone()),
                ));
                return;
            }
            self.history_push_plain_state(history_cell::new_error_event(format!(
                "Working directory is missing: {}",
                self.config.cwd.display()
            )));
            return;
        }
        let original_text = message.display_text.clone();

        let mut submitted_cli = false;
        let manual_edit_pending = self.auto_state.is_paused_manual()
            && self.auto_state.resume_after_submit();
        let manual_override_active = self.auto_state.is_paused_manual();
        let bypass_active = self.auto_state.should_bypass_coordinator_next_submit();
        let coordinator_routing_allowed = if bypass_active {
            manual_edit_pending || manual_override_active
        } else {
            true
        };

        let should_route_through_coordinator = !message.suppress_persistence
            && !original_text.trim().starts_with('/')
            && self.auto_state.is_active()
            && self.config.auto_drive.coordinator_routing
            && coordinator_routing_allowed;

        if should_route_through_coordinator
        {
            let mut conversation = self.current_auto_history();
            if let Some(user_item) = Self::auto_drive_make_user_message(original_text.clone()) {
                conversation.push(user_item.clone());
                if self.auto_send_user_prompt_to_coordinator(original_text.clone(), conversation) {
                    self.finalize_sent_user_message(message);
                    self.consume_pending_prompt_for_ui_only_turn();
                    self.auto_history.append_raw(std::slice::from_ref(&user_item));
                    return;
                }
            }
        }

        if !message.suppress_persistence
            && self.auto_state.is_active()
            && self.config.auto_drive.coordinator_routing
            && coordinator_routing_allowed
            && let Some(mut routed) = self.try_coordinator_route(&original_text) {
                self.finalize_sent_user_message(message);
                self.consume_pending_prompt_for_ui_only_turn();

                if let Some(notice_text) = routed.user_response.take() {
                    if let Some(item) =
                        Self::auto_drive_make_assistant_message(notice_text.clone())
                    {
                        self.auto_history.append_raw(std::slice::from_ref(&item));
                    }
                    let lines = vec!["AUTO DRIVE RESPONSE".to_string(), notice_text];
                    self.history_push_plain_paragraphs(PlainMessageKind::Notice, lines);
                }

                let _ = self.rebuild_auto_history();

                if let Some(cli_command) = routed.cli_command {
                    let mut synthetic: UserMessage = cli_command.into();
                    synthetic.suppress_persistence = true;
                    self.submit_user_message(synthetic);
                    submitted_cli = true;
                }

                if !submitted_cli {
                    self.auto_send_conversation_force();
                }

                return;
            }

        let only_text_items = message
            .ordered_items
            .iter()
            .all(|item| matches!(item, InputItem::Text { .. }));
        if only_text_items
            && let Some((command_line, rest_text)) =
                Self::split_leading_slash_command(&original_text)
                && Self::multiline_slash_command_requires_split(&command_line) {
                    let preview = crate::slash_command::process_slash_command_message(
                        command_line.as_str(),
                    );
                    match preview {
                        ProcessedCommand::RegularCommand(SlashCommand::Auto, canonical_text) => {
                            let goal = rest_text.trim();
                            let command_text = if goal.is_empty() {
                                canonical_text
                            } else {
                                format!("{canonical_text} {goal}")
                            };
                            self.app_event_tx
                                .send(AppEvent::DispatchCommand(SlashCommand::Auto, command_text));
                            return;
                        }
                        ProcessedCommand::NotCommand(_) => {}
                        _ => {
                            self.submit_user_message(command_line.into());
                            let trimmed_rest = rest_text.trim();
                            if !trimmed_rest.is_empty() {
                                self.submit_user_message(rest_text.into());
                            }
                            return;
                        }
                    }
                }
        // Build a combined string view of the text-only parts to process slash commands
        let mut text_only = String::new();
        for it in &message.ordered_items {
            if let InputItem::Text { text } = it {
                if !text_only.is_empty() {
                    text_only.push('\n');
                }
                text_only.push_str(text);
            }
        }

        // Expand user-defined custom prompts, supporting both "/prompts:name" and "/name" forms.
        match prompt_args::expand_custom_prompt(&text_only, self.bottom_pane.custom_prompts()) {
            Ok(Some(expanded)) => {
                text_only = expanded.clone();
                message
                    .ordered_items
                    .clear();
                message
                    .ordered_items
                    .push(InputItem::Text { text: expanded });
            }
            Ok(None) => {}
            Err(err) => {
                self.history_push_plain_state(history_cell::new_error_event(err.user_message()));
                return;
            }
        }

        // Save the prompt if it's a multi-agent command
        let original_trimmed = original_text.trim();
        if original_trimmed.starts_with("/plan ")
            || original_trimmed.starts_with("/solve ")
            || original_trimmed.starts_with("/code ")
        {
            self.last_agent_prompt = Some(original_text.clone());
        }

        // Process slash commands and expand them if needed
        // First, allow custom subagent commands: if the message starts with a slash and the
        // command name matches a saved subagent in config, synthesize a unified prompt using
        // format_subagent_command and replace the message with that prompt.
        if let Some(first) = original_text.trim().strip_prefix('/') {
            let mut parts = first.splitn(2, ' ');
            let cmd_name = parts.next().unwrap_or("").trim();
            let args = parts.next().unwrap_or("").trim().to_string();
            if !cmd_name.is_empty() {
                let has_custom = self
                    .config
                    .subagent_commands
                    .iter()
                    .any(|c| c.name.eq_ignore_ascii_case(cmd_name));
                // Treat built-ins via the standard path below to preserve existing ack flow,
                // but allow any other saved subagent command to be executed here.
                let is_builtin = matches!(
                    cmd_name.to_ascii_lowercase().as_str(),
                    "plan" | "solve" | "code"
                );
                if has_custom && !is_builtin {
                    let res = code_core::slash_commands::format_subagent_command(
                        cmd_name,
                        &args,
                        Some(&self.config.agents),
                        Some(&self.config.subagent_commands),
                    );
                    if !res.read_only
                        && self.ensure_git_repo_for_action(
                            GitInitResume::SubmitText {
                                text: original_text.clone(),
                            },
                            "Write-enabled agents require a git repository.",
                        )
                    {
                        return;
                    }
                    // Acknowledge configuration
                    let mode = if res.read_only { "read-only" } else { "write" };
                    let agents = if res.models.is_empty() {
                        "<none>".to_string()
                    } else {
                        res.models.join(", ")
                    };
                    let lines = vec![
                        format!("/{} configured", res.name),
                        format!("mode: {}", mode),
                        format!("agents: {}", agents),
                        format!("command: {}", original_text.trim()),
                    ];
                    self.history_push_plain_paragraphs(PlainMessageKind::Notice, lines);

                    message
                        .ordered_items
                        .clear();
                    message
                        .ordered_items
                        .push(InputItem::Text { text: res.prompt });
                    // Continue with normal submission after this match block
                }
            }
        }

        let processed = crate::slash_command::process_slash_command_message(&text_only);
        match processed {
            crate::slash_command::ProcessedCommand::ExpandedPrompt(_expanded) => {
                // If a built-in multi-agent slash command was used, resolve
                // configured subagent settings and feed the synthesized prompt
                // without echoing an additional acknowledgement cell.
                let trimmed = original_trimmed;
                let (cmd_name, args_opt) = if let Some(rest) = trimmed.strip_prefix("/plan ") {
                    ("plan", Some(rest.trim().to_string()))
                } else if let Some(rest) = trimmed.strip_prefix("/solve ") {
                    ("solve", Some(rest.trim().to_string()))
                } else if let Some(rest) = trimmed.strip_prefix("/code ") {
                    ("code", Some(rest.trim().to_string()))
                } else {
                    ("", None)
                };

                if let Some(task) = args_opt {
                    let res = code_core::slash_commands::format_subagent_command(
                        cmd_name,
                        &task,
                        Some(&self.config.agents),
                        Some(&self.config.subagent_commands),
                    );
                    if !res.read_only
                        && self.ensure_git_repo_for_action(
                            GitInitResume::SubmitText {
                                text: original_text.clone(),
                            },
                            "Write-enabled agents require a git repository.",
                        )
                    {
                        return;
                    }

                    // Replace the message with the resolved prompt and suppress the
                    // agent launch hint that would otherwise echo back immediately.
                    self.suppress_next_agent_hint = true;
                    message
                        .ordered_items
                        .clear();
                    message
                        .ordered_items
                        .push(InputItem::Text { text: res.prompt });
                } else {
                    // Fallback to default expansion behavior
                    let expanded = _expanded;
                    message
                        .ordered_items
                        .clear();
                    message
                        .ordered_items
                        .push(InputItem::Text { text: expanded });
                }
            }
            crate::slash_command::ProcessedCommand::RegularCommand(cmd, command_text) => {
                if cmd == SlashCommand::Undo {
                    self.handle_undo_command();
                    return;
                }
                // This is a regular slash command, dispatch it normally
                self.app_event_tx
                    .send(AppEvent::DispatchCommand(cmd, command_text));
                return;
            }
            crate::slash_command::ProcessedCommand::Error(error_msg) => {
                // Show error in history
                self.history_push_plain_state(history_cell::new_error_event(error_msg));
                return;
            }
            crate::slash_command::ProcessedCommand::NotCommand(_) => {
                // Not a slash command, process normally
            }
        }

        let mut items: Vec<InputItem> = Vec::new();

        // Check if browser mode is enabled and capture screenshot
        // IMPORTANT: Always use global browser manager for consistency
        // The global browser manager ensures both TUI and agent tools use the same instance

        // Start async screenshot capture in background (non-blocking)
        {
            let latest_browser_screenshot_clone = Arc::clone(&self.latest_browser_screenshot);

            tokio::spawn(async move {
                tracing::info!("Evaluating background screenshot capture...");

                // Rate-limit: skip if a capture ran very recently (< 4000ms)
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let last = BG_SHOT_LAST_START_MS.load(Ordering::Relaxed);
                if now_ms.saturating_sub(last) < 4000 {
                    tracing::info!("Skipping background screenshot: rate-limited");
                    return;
                }

                // Single-flight: skip if another capture is in progress
                if BG_SHOT_IN_FLIGHT.swap(true, Ordering::AcqRel) {
                    tracing::info!("Skipping background screenshot: already in-flight");
                    return;
                }
                // Ensure we always clear the flag
                struct ShotGuard;
                impl Drop for ShotGuard {
                    fn drop(&mut self) {
                        BG_SHOT_IN_FLIGHT.store(false, Ordering::Release);
                    }
                }
                let _guard = ShotGuard;

                // Short settle to allow page to reach a stable state; keep it small
                tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;

                let Some(browser_manager) = code_browser::global::get_browser_manager().await else {
                    tracing::info!("Skipping background screenshot: browser manager unavailable");
                    return;
                };

                if !browser_manager.is_enabled().await {
                    tracing::info!("Skipping background screenshot: browser disabled");
                    return;
                }

                if browser_manager.idle_elapsed_past_timeout().await.is_some() {
                    tracing::info!("Skipping background screenshot: browser idle");
                    return;
                }

                BG_SHOT_LAST_START_MS.store(now_ms, Ordering::Relaxed);

                tracing::info!("Screenshot capture attempt 1 of 1");

                // Add timeout to screenshot capture
                let capture_result = tokio::time::timeout(
                    tokio::time::Duration::from_secs(5),
                    browser_manager.capture_screenshot_with_url(),
                )
                .await;

                match capture_result {
                    Ok(Ok((screenshot_paths, url))) => {
                        tracing::info!(
                            "Background screenshot capture succeeded with {} images on attempt 1",
                            screenshot_paths.len()
                        );

                        // Save the first screenshot path and URL for display in the TUI
                        if let Some(first_path) = screenshot_paths.first()
                            && let Ok(mut latest) = latest_browser_screenshot_clone.lock() {
                                let url_string = url.clone().unwrap_or_else(|| "Browser".to_string());
                                *latest = Some((first_path.clone(), url_string));
                            }

                        // Create screenshot items
                        let mut screenshot_items = Vec::new();
                        for path in screenshot_paths {
                            if path.exists() {
                                tracing::info!("Adding browser screenshot: {}", path.display());
                                let timestamp = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_secs();
                                let metadata = format!(
                                    "screenshot:{}:{}",
                                    timestamp,
                                    url.as_deref().unwrap_or("unknown")
                                );
                                screenshot_items.push(InputItem::EphemeralImage {
                                    path,
                                    metadata: Some(metadata),
                                });
                            }
                        }

                        // Do not enqueue screenshots as messages.
                        // They are now injected per-turn by the core session.
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("Background screenshot capture failed (attempt 1): {}", e);
                    }
                    Err(_timeout_err) => {
                        tracing::warn!("Background screenshot capture timed out (attempt 1)");
                    }
                }
            });
        }

        // Use the ordered items (text + images interleaved with markers)
        items.extend(message.ordered_items.clone());
        message.ordered_items = items;

        if message.ordered_items.is_empty() {
            return;
        }

        let wait_only_active = self.wait_only_activity();
        let turn_active = (self.is_task_running()
            || !self.active_task_ids.is_empty()
            || self.stream.is_write_cycle_active()
            || !self.queued_user_messages.is_empty())
            && !wait_only_active;

        if turn_active {
            tracing::info!(
                "[queue] Enqueuing user input while turn is active (queue_size={}, task_running={}, stream_active={}, active_tasks={})",
                self.queued_user_messages.len() + 1,
                self.is_task_running(),
                self.stream.is_write_cycle_active(),
                self.active_task_ids.len()
            );
            let queued_clone = message.clone();
            self.queued_user_messages.push_back(queued_clone);
            self.refresh_queued_user_messages(true);

            let prompt_summary = if message.display_text.trim().is_empty() {
                None
            } else {
                Some(message.display_text.clone())
            };

            let should_capture_snapshot = self.active_ghost_snapshot.is_none()
                && self.ghost_snapshot_queue.is_empty();
            if should_capture_snapshot {
                let _ = self.capture_ghost_snapshot(prompt_summary);
            }
            self.dispatch_queued_user_message_now(message);
            return;
        }

        if wait_only_active {
            // Keep long waits running but do not block user input.
            self.bottom_pane.set_task_running(false);
            self.bottom_pane
                .update_status_text("Waiting in background".to_string());
        }

        tracing::info!(
            "[queue] Turn idle, enqueuing and preparing to drain (auto_active={}, queue_size={})",
            self.auto_state.is_active(),
            self.queued_user_messages.len() + 1
        );

        let queued_clone = message.clone();
        self.queued_user_messages.push_back(queued_clone);
        self.refresh_queued_user_messages(false);

        let batch: Vec<UserMessage> = self.queued_user_messages.iter().cloned().collect();
        let summary = batch
            .last()
            .and_then(|msg| {
                let trimmed = msg.display_text.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(msg.display_text.clone())
                }
            });

        let _ = self.capture_ghost_snapshot(summary);

        if self.auto_state.is_active() {
            tracing::info!(
                "[queue] Draining via coordinator path for Auto Drive (batch_size={})",
                batch.len()
            );
            self.dispatch_queued_batch_via_coordinator(batch);
        } else {
            tracing::info!(
                "[queue] Draining via direct batch dispatch (batch_size={})",
                batch.len()
            );
            self.dispatch_queued_batch(batch);
        }

        // (debug watchdog removed)
    }

    pub(super) fn split_leading_slash_command(text: &str) -> Option<(String, String)> {
        if !text.starts_with('/') {
            return None;
        }
        let mut parts = text.splitn(2, '\n');
        let first_line = parts.next().unwrap_or_default();
        let rest = parts.next().unwrap_or("");
        if rest.is_empty() {
            return None;
        }
        let command = first_line.trim_end_matches('\r');
        if command.is_empty() {
            return None;
        }
        if rest.trim().is_empty() {
            return None;
        }
        Some((command.to_string(), rest.to_string()))
    }

    pub(super) fn slash_command_from_line(line: &str) -> Option<SlashCommand> {
        let trimmed = line.trim();
        let command_portion = trimmed.strip_prefix('/')?;
        let name = command_portion.split_whitespace().next()?;
        let canonical = name.to_ascii_lowercase();
        SlashCommand::from_str(&canonical).ok()
    }

    pub(super) fn multiline_slash_command_requires_split(command_line: &str) -> bool {
        Self::slash_command_from_line(command_line)
            .map(|cmd| !cmd.is_prompt_expanding())
            .unwrap_or(true)
    }

    pub(super) fn capture_ghost_snapshot(&mut self, summary: Option<String>) -> GhostSnapshotJobHandle {
        if self.ghost_snapshots_disabled {
            return GhostSnapshotJobHandle::Skipped;
        }

        let request = GhostSnapshotRequest::new(
            summary,
            self.current_conversation_snapshot(),
            self.history_snapshot_for_persistence(),
        );
        self.enqueue_ghost_snapshot(request)
    }

    pub(super) fn capture_ghost_snapshot_blocking(&mut self, summary: Option<String>) -> Option<GhostSnapshot> {
        if self.ghost_snapshots_disabled {
            return None;
        }

        let request = GhostSnapshotRequest::new(
            summary,
            self.current_conversation_snapshot(),
            self.history_snapshot_for_persistence(),
        );
        let repo_path = self.config.cwd.clone();
        let started_at = request.started_at;
        let hook_repo = repo_path.clone();
        let result = create_ghost_commit(
            &CreateGhostCommitOptions::new(repo_path.as_path())
                .post_commit_hook(&move || bump_snapshot_epoch_for(&hook_repo)),
        );
        let elapsed = started_at.elapsed();
        
        self.finalize_ghost_snapshot(request, result, elapsed)
    }

    pub(super) fn dispatch_queued_batch(&mut self, batch: Vec<UserMessage>) {
        if batch.is_empty() {
            return;
        }

        let mut messages: Vec<UserMessage> = Vec::with_capacity(batch.len());

        for message in batch {
            let Some(message) = self.take_queued_user_message(&message) else {
                tracing::info!("Skipping queued user input removed before dispatch");
                continue;
            };
            messages.push(message);
        }

        if messages.is_empty() {
            return;
        }

        let mut combined_items: Vec<InputItem> = Vec::new();

        for (idx, message) in messages.iter().enumerate() {
            if idx > 0 && !combined_items.is_empty() && !message.ordered_items.is_empty() {
                combined_items.push(InputItem::Text {
                    text: "\n\n".to_string(),
                });
            }
            combined_items.extend(message.ordered_items.clone());
        }

        let total_items = combined_items.len();
        let ephemeral_count = combined_items
            .iter()
            .filter(|item| matches!(item, InputItem::EphemeralImage { .. }))
            .count();
        if ephemeral_count > 0 {
            tracing::info!(
                "Sending {} items to model (including {} ephemeral images)",
                total_items,
                ephemeral_count
            );
        }

        if !combined_items.is_empty() {
            self.flush_pending_agent_notes();
            if let Err(e) = self
                .code_op_tx
                .send(Op::UserInput {
                    items: combined_items,
                    final_output_json_schema: None,
                })
            {
                tracing::error!("failed to send Op::UserInput: {e}");
            }
        }

        for message in messages {
            self.finalize_sent_user_message(message);
        }
    }

    pub(super) fn dispatch_queued_user_message_now(&mut self, message: UserMessage) {
        let message = self.take_queued_user_message(&message).unwrap_or(message);
        let items = message.ordered_items.clone();
        tracing::info!(
            "[queue] Dispatching single queued message via coordinator (queue_remaining={})",
            self.queued_user_messages.len()
        );
        match self.code_op_tx.send(Op::QueueUserInput { items }) {
            Ok(()) => {
                self.finalize_sent_user_message(message);
            }
            Err(err) => {
                tracing::error!("failed to send QueueUserInput op: {err}");
                self.queued_user_messages.push_front(message);
                self.refresh_queued_user_messages(true);
            }
        }
    }

    pub(super) fn dispatch_queued_batch_via_coordinator(&mut self, batch: Vec<UserMessage>) {
        if batch.is_empty() {
            return;
        }

        tracing::info!(
            "[queue] Draining batch via coordinator path (batch_size={}, auto_active={})",
            batch.len(),
            self.auto_state.is_active()
        );

        for message in batch {
            let Some(message) = self.take_queued_user_message(&message) else {
                tracing::info!("[queue] Skipping queued user input removed before dispatch");
                continue;
            };

            let items = message.ordered_items.clone();
            match self.code_op_tx.send(Op::QueueUserInput { items }) {
                Ok(()) => {
                    self.finalize_sent_user_message(message);
                }
                Err(err) => {
                    tracing::error!("[queue] Failed to send QueueUserInput op: {err}");
                    self.queued_user_messages.push_front(message);
                    self.refresh_queued_user_messages(true);
                    break;
                }
            }
        }
    }

    pub(super) fn take_queued_user_message(&mut self, target: &UserMessage) -> Option<UserMessage> {
        let position = self
            .queued_user_messages
            .iter()
            .position(|message| message == target)?;
        let removed = self.queued_user_messages.remove(position)?;
        self.refresh_queued_user_messages(false);
        Some(removed)
    }

    pub(super) fn enqueue_ghost_snapshot(&mut self, request: GhostSnapshotRequest) -> GhostSnapshotJobHandle {
        let job_id = self.next_ghost_snapshot_id;
        self.next_ghost_snapshot_id = self.next_ghost_snapshot_id.wrapping_add(1);
        self.ghost_snapshot_queue.push_back((job_id, request));
        self.spawn_next_ghost_snapshot();
        GhostSnapshotJobHandle::Scheduled(job_id)
    }

    pub(super) fn spawn_next_ghost_snapshot(&mut self) {
        if self.ghost_snapshots_disabled {
            self.ghost_snapshot_queue.clear();
            return;
        }
        if self.active_ghost_snapshot.is_some() {
            return;
        }
        let Some((job_id, request)) = self.ghost_snapshot_queue.pop_front() else {
            return;
        };

        let repo_path = self.config.cwd.clone();
        let app_event_tx = self.app_event_tx.clone();
        let notice_ticket = self.make_background_tail_ticket();
        let started_at = request.started_at;
        self.active_ghost_snapshot = Some((job_id, request));

        tokio::spawn(async move {
            let handle = tokio::task::spawn_blocking(move || {
                let hook_repo = repo_path.clone();
                let options = CreateGhostCommitOptions::new(repo_path.as_path());
                create_ghost_commit(&options.post_commit_hook(&move || bump_snapshot_epoch_for(&hook_repo)))
            });
            tokio::pin!(handle);

            let mut notice_sent = false;
            let notice_sleep = tokio::time::sleep(GHOST_SNAPSHOT_NOTICE_THRESHOLD);
            tokio::pin!(notice_sleep);
            let timeout_sleep = tokio::time::sleep(GHOST_SNAPSHOT_TIMEOUT);
            tokio::pin!(timeout_sleep);

            let join_result = loop {
                tokio::select! {
                    res = &mut handle => break res,
                    _ = &mut timeout_sleep => {
                        handle.as_mut().abort();
                        let elapsed = started_at.elapsed();
                        let err = GitToolingError::Io(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!(
                                "ghost snapshot exceeded {}",
                                format_duration(GHOST_SNAPSHOT_TIMEOUT)
                            ),
                        ));
                        let event = AppEvent::GhostSnapshotFinished {
                            job_id,
                            result: Err(err),
                            elapsed,
                        };
                        app_event_tx.send(event);
                        return;
                    }
                    _ = &mut notice_sleep, if !notice_sent => {
                        notice_sent = true;
                        let elapsed = started_at.elapsed();
                        let message = format!(
                            "Git snapshot still running… {} elapsed.",
                            format_duration(elapsed)
                        );
                        app_event_tx.send_background_event_with_ticket(&notice_ticket, message);
                    }
                }
            };

            let elapsed = started_at.elapsed();
            let event = match join_result {
                Ok(Ok(commit)) => AppEvent::GhostSnapshotFinished {
                    job_id,
                    result: Ok(commit),
                    elapsed,
                },
                Ok(Err(err)) => AppEvent::GhostSnapshotFinished {
                    job_id,
                    result: Err(err),
                    elapsed,
                },
                Err(join_err) => {
                    let err = GitToolingError::Io(io::Error::other(
                        format!("ghost snapshot task failed: {join_err}"),
                    ));
                    AppEvent::GhostSnapshotFinished {
                        job_id,
                        result: Err(err),
                        elapsed,
                    }
                }
            };

            app_event_tx.send(event);
        });
    }

    pub(super) fn finalize_ghost_snapshot(
        &mut self,
        request: GhostSnapshotRequest,
        result: Result<GhostCommit, GitToolingError>,
        elapsed: Duration,
    ) -> Option<GhostSnapshot> {
        match result {
            Ok(commit) => {
                self.ghost_snapshots_disabled = false;
                self.ghost_snapshots_disabled_reason = None;
                let snapshot = GhostSnapshot::new(
                    commit,
                    request.summary,
                    request.conversation,
                    request.history,
                );
                self.ghost_snapshots.push(snapshot.clone());
                session_log::log_history_snapshot(
                    snapshot.commit().id(),
                    snapshot.summary.as_deref(),
                    &snapshot.history,
                );
                if self.ghost_snapshots.len() > MAX_TRACKED_GHOST_COMMITS {
                    self.ghost_snapshots.remove(0);
                }
                if elapsed >= GHOST_SNAPSHOT_NOTICE_THRESHOLD {
                    self.push_background_tail(format!(
                        "Git snapshot captured in {}.",
                        format_duration(elapsed)
                    ));
                }
                Some(snapshot)
            }
            Err(err) => {
                if let GitToolingError::Io(io_err) = &err
                    && io_err.kind() == io::ErrorKind::TimedOut {
                        self.push_background_tail(format!(
                            "Git snapshot timed out after {}. Try again once the repository is less busy.",
                            format_duration(elapsed)
                        ));
                        tracing::warn!(
                            elapsed = %format_duration(elapsed),
                            "ghost snapshot timed out"
                        );
                        return None;
                    }
                self.ghost_snapshots_disabled = true;
                let (message, hint) = match &err {
                    GitToolingError::NotAGitRepository { .. } => (
                        "Snapshots disabled: this workspace is not inside a Git repository.".to_string(),
                        None,
                    ),
                    _ => (
                        format!("Snapshots disabled after Git error: {err}"),
                        Some(
                            "Restart Code after resolving the issue to re-enable snapshots.".to_string(),
                        ),
                    ),
                };
                self.ghost_snapshots_disabled_reason = Some(GhostSnapshotsDisabledReason {
                    message: message.clone(),
                    hint: hint.clone(),
                });
                self.push_background_tail(message);
                if let Some(hint) = hint {
                    self.push_background_tail(hint);
                }
                tracing::warn!("failed to create ghost snapshot: {err}");
                self.ghost_snapshot_queue.clear();
                None
            }
        }
    }

    pub(crate) fn handle_ghost_snapshot_finished(
        &mut self,
        job_id: u64,
        result: Result<GhostCommit, GitToolingError>,
        elapsed: Duration,
    ) {
        let Some((active_id, request)) = self.active_ghost_snapshot.take() else {
            tracing::warn!("ghost snapshot finished without active job (id={job_id})");
            return;
        };

        if active_id != job_id {
            tracing::warn!(
                "ghost snapshot job id mismatch: expected {active_id}, got {job_id}"
            );
            self.active_ghost_snapshot = Some((active_id, request));
            return;
        }

        let _ = self.finalize_ghost_snapshot(request, result, elapsed);
        self.request_redraw();
        self.spawn_next_ghost_snapshot();
    }

    pub(super) fn current_conversation_snapshot(&self) -> ConversationSnapshot {
        use crate::history_cell::HistoryCellType;
        let mut user_turns = 0usize;
        let mut assistant_turns = 0usize;
        for cell in &self.history_cells {
            match cell.kind() {
                HistoryCellType::User => user_turns = user_turns.saturating_add(1),
                HistoryCellType::Assistant => {
                    assistant_turns = assistant_turns.saturating_add(1)
                }
                _ => {}
            }
        }
        let mut snapshot = ConversationSnapshot::new(user_turns, assistant_turns);
        snapshot.history_len = self.history_cells.len();
        snapshot.order_len = self.cell_order_seq.len();
        snapshot.order_dbg_len = self.cell_order_dbg.len();
        snapshot
    }

    pub(super) fn conversation_delta_since(
        &self,
        snapshot: &ConversationSnapshot,
    ) -> (usize, usize) {
        let current = self.current_conversation_snapshot();
        let user_delta = current
            .user_turns
            .saturating_sub(snapshot.user_turns);
        let assistant_delta = current
            .assistant_turns
            .saturating_sub(snapshot.assistant_turns);
        (user_delta, assistant_delta)
    }

    pub(super) fn history_snapshot_for_persistence(&self) -> HistorySnapshot {
        let order: Vec<OrderKeySnapshot> = self
            .cell_order_seq
            .iter()
            .map(|key| (*key).into())
            .collect();
        let order_debug = self.cell_order_dbg.clone();
        self.history_state
            .snapshot()
            .with_order(order, order_debug)
    }

    pub(super) fn mark_history_dirty(&mut self) {
        self.history_snapshot_dirty = true;
        self.render_request_cache_dirty.set(true);
        self.flush_history_snapshot_if_needed(false);
        self.sync_history_virtualization();
    }

    pub(super) fn flush_history_snapshot_if_needed(&mut self, force: bool) {
        if !self.history_snapshot_dirty {
            return;
        }
        if !force
            && let Some(last) = self.history_snapshot_last_flush
                && last.elapsed() < Duration::from_millis(400) {
                    return;
                }
        let snapshot = self.history_snapshot_for_persistence();
        match serde_json::to_value(&snapshot) {
            Ok(snapshot_value) => {
                let send_result = self
                    .code_op_tx
                    .send(Op::PersistHistorySnapshot { snapshot: snapshot_value });
                if send_result.is_err() {
                    tracing::warn!("failed to send history snapshot to core");
                } else {
                    self.history_snapshot_dirty = false;
                }
                self.history_snapshot_last_flush = Some(Instant::now());
            }
            Err(err) => {
                tracing::warn!("failed to serialize history snapshot: {err}");
            }
        }
    }

    pub(crate) fn snapshot_ghost_state(&self) -> GhostState {
        GhostState {
            snapshots: self.ghost_snapshots.clone(),
            disabled: self.ghost_snapshots_disabled,
            disabled_reason: self.ghost_snapshots_disabled_reason.clone(),
            queue: self.ghost_snapshot_queue.clone(),
            active: self.active_ghost_snapshot.clone(),
            next_id: self.next_ghost_snapshot_id,
            queued_user_messages: self.queued_user_messages.clone(),
        }
    }

    pub(crate) fn adopt_ghost_state(&mut self, state: GhostState) {
        self.ghost_snapshots = state.snapshots;
        if self.ghost_snapshots.len() > MAX_TRACKED_GHOST_COMMITS {
            self.ghost_snapshots
                .truncate(MAX_TRACKED_GHOST_COMMITS);
        }
        self.ghost_snapshots_disabled = state.disabled;
        self.ghost_snapshots_disabled_reason = state.disabled_reason;
        self.ghost_snapshot_queue = state.queue;
        self.active_ghost_snapshot = state.active;
        self.next_ghost_snapshot_id = state.next_id;
        self.queued_user_messages = state.queued_user_messages;
        let blocked = self.is_task_running()
            || !self.active_task_ids.is_empty()
            || self.stream.is_write_cycle_active();
        self.refresh_queued_user_messages(blocked);
        self.spawn_next_ghost_snapshot();
    }

    pub(crate) fn handle_undo_command(&mut self) {
        if self.ghost_snapshots_disabled {
            let reason = self
                .ghost_snapshots_disabled_reason
                .as_ref()
                .map(|reason| reason.message.clone())
                .unwrap_or_else(|| "Snapshots are currently disabled.".to_string());
            self.push_background_tail(format!("/undo unavailable: {reason}"));
            self.show_undo_snapshots_disabled();
            return;
        }

        if self.ghost_snapshots.is_empty() {
            self.push_background_tail(
                "/undo unavailable: no snapshots captured yet. Run a file-modifying command to create one.".to_string(),
            );
            self.show_undo_empty_state();
            return;
        }

        self.show_undo_snapshot_picker();
    }

    pub(super) fn show_undo_snapshots_disabled(&mut self) {
        let mut lines: Vec<String> = Vec::new();
        if let Some(reason) = &self.ghost_snapshots_disabled_reason {
            lines.push(reason.message.clone());
            if let Some(hint) = &reason.hint {
                lines.push(hint.clone());
            }
        } else {
            lines.push(
                "Snapshots are currently disabled. Resolve the Git issue and restart Code to re-enable them.".to_string(),
            );
        }

        self.show_undo_status_popup(
            "Snapshots unavailable",
            Some(
                "Restores workspace files only. Conversation history remains unchanged.".to_string(),
            ),
            Some("Automatic snapshotting failed, so /undo cannot restore the workspace.".to_string()),
            lines,
        );
    }

    pub(super) fn show_undo_empty_state(&mut self) {
        self.show_undo_status_popup(
            "No snapshots yet",
            Some(
                "Restores workspace files only. Conversation history remains unchanged.".to_string(),
            ),
            Some("Snapshots appear once Code captures a Git checkpoint.".to_string()),
            vec![
                "No snapshot is available to restore.".to_string(),
                "Run a command that modifies files to create the first snapshot.".to_string(),
            ],
        );
    }

    pub(super) fn show_undo_status_popup(
        &mut self,
        title: &str,
        scope_hint: Option<String>,
        subtitle: Option<String>,
        mut lines: Vec<String>,
    ) {
        if lines.is_empty() {
            lines.push("No snapshot information available.".to_string());
        }

        let headline = lines.remove(0);
        let description = if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        };

        let mut composed_subtitle = Vec::new();
        if let Some(hint) = scope_hint {
            composed_subtitle.push(hint);
        }
        if let Some(extra) = subtitle {
            composed_subtitle.push(extra);
        }
        let subtitle_for_view = if composed_subtitle.is_empty() {
            None
        } else {
            Some(composed_subtitle.join("\n"))
        };

        let items = vec![SelectionItem {
            name: headline,
            description,
            is_current: true,
            actions: Vec::new(),
        }];

        let view = ListSelectionView::new(
            format!(" {title} "),
            subtitle_for_view,
            Some("Esc close".to_string()),
            items,
            self.app_event_tx.clone(),
            1,
        );

        self.bottom_pane.show_list_selection(
            title.to_string(),
            None,
            Some("Esc close".to_string()),
            view,
        );
    }

    pub(super) fn show_undo_snapshot_picker(&mut self) {
        let entries = self.build_undo_timeline_entries();
        if entries.len() <= 1 {
            self.push_background_tail(
                "/undo unavailable: no snapshots captured yet. Run a file-modifying command to create one.".to_string(),
            );
            self.show_undo_empty_state();
            return;
        }

        let current_index = entries.len().saturating_sub(1);
        let view = UndoTimelineView::new(entries, current_index, self.app_event_tx.clone());
        self.bottom_pane.show_undo_timeline_view(view);
    }

    pub(super) fn build_undo_timeline_entries(&self) -> Vec<UndoTimelineEntry> {
        let mut entries: Vec<UndoTimelineEntry> = Vec::with_capacity(self.ghost_snapshots.len().saturating_add(1));
        for snapshot in self.ghost_snapshots.iter() {
            entries.push(self.timeline_entry_for_snapshot(snapshot));
        }
        entries.push(self.timeline_entry_for_current());
        entries
    }

    pub(super) fn timeline_entry_for_snapshot(&self, snapshot: &GhostSnapshot) -> UndoTimelineEntry {
        let short_id = snapshot.short_id();
        let label = format!("Snapshot {short_id}");
        let summary = snapshot.summary.clone();
        let timestamp_line = Some(snapshot.captured_at.format("%Y-%m-%d %H:%M:%S").to_string());
        let relative_time = snapshot
            .age_from(Local::now())
            .map(|age| format!("captured {} ago", format_duration(age)));
        let (user_delta, assistant_delta) = self.conversation_delta_since(&snapshot.conversation);
        let stats_line = if user_delta == 0 && assistant_delta == 0 {
            Some("conversation already matches current state".to_string())
        } else if assistant_delta == 0 {
            Some(format!(
                "rewind {} user turn{}",
                user_delta,
                if user_delta == 1 { "" } else { "s" }
            ))
        } else {
            Some(format!(
                "rewind {} user turn{} and {} assistant repl{}",
                user_delta,
                if user_delta == 1 { "" } else { "s" },
                assistant_delta,
                if assistant_delta == 1 { "y" } else { "ies" }
            ))
        };

        let conversation_lines = Self::conversation_preview_lines_from_snapshot(&snapshot.history);
        let file_lines = self.timeline_file_lines_for_commit(snapshot.commit().id());

        UndoTimelineEntry {
            label,
            summary,
            timestamp_line,
            relative_time,
            stats_line,
            commit_line: Some(format!("commit {short_id}")),
            conversation_lines,
            file_lines,
            conversation_available: user_delta > 0,
            files_available: true,
            kind: UndoTimelineEntryKind::Snapshot {
                commit: snapshot.commit().id().to_string(),
            },
        }
    }

    pub(super) fn timeline_entry_for_current(&self) -> UndoTimelineEntry {
        let history_snapshot = self.history_snapshot_for_persistence();
        let conversation_lines = Self::conversation_preview_lines_from_snapshot(&history_snapshot);
        let file_lines = self.timeline_file_lines_for_current();
        UndoTimelineEntry {
            label: "Current workspace".to_string(),
            summary: None,
            timestamp_line: Some(Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
            relative_time: Some("current point".to_string()),
            stats_line: Some("Already at this point in time".to_string()),
            commit_line: None,
            conversation_lines,
            file_lines,
            conversation_available: false,
            files_available: false,
            kind: UndoTimelineEntryKind::Current,
        }
    }

    pub(super) fn conversation_preview_lines_from_snapshot(snapshot: &HistorySnapshot) -> Vec<Line<'static>> {
        let mut state = HistoryState::new();
        state.restore(snapshot);
        let mut messages: Vec<(UndoPreviewRole, String)> = Vec::new();
        for record in &state.records {
            match record {
                HistoryRecord::PlainMessage(msg) => match msg.kind {
                    PlainMessageKind::User => {
                        let text = Self::message_lines_to_plain_preview(&msg.lines);
                        if !text.is_empty() {
                            messages.push((UndoPreviewRole::User, text));
                        }
                    }
                    PlainMessageKind::Assistant => {
                        let text = Self::message_lines_to_plain_preview(&msg.lines);
                        if !text.is_empty() {
                            messages.push((UndoPreviewRole::Assistant, text));
                        }
                    }
                    _ => {}
                },
                HistoryRecord::AssistantMessage(msg) => {
                    let text = Self::markdown_to_plain_preview(&msg.markdown);
                    if !text.is_empty() {
                        messages.push((UndoPreviewRole::Assistant, text));
                    }
                }
                _ => {}
            }
        }

        if messages.is_empty() {
            return vec![Line::from(Span::styled(
                "No conversation captured in this snapshot.",
                Style::default().fg(crate::colors::text_dim()),
            ))];
        }

        let len = messages.len();
        let start = len.saturating_sub(Self::MAX_UNDO_CONVERSATION_MESSAGES);
        messages[start..]
            .iter()
            .map(|(role, text)| Self::conversation_line(*role, text.as_str()))
            .collect()
    }

    pub(super) fn conversation_line(role: UndoPreviewRole, text: &str) -> Line<'static> {
        let (label, color) = match role {
            UndoPreviewRole::User => ("You", crate::colors::text_bright()),
            UndoPreviewRole::Assistant => ("Code", crate::colors::primary()),
        };
        let label_span = Span::styled(
            format!("{label}: "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        );
        let content_span = Span::styled(text.to_string(), Style::default().fg(crate::colors::text()));
        Line::from(vec![label_span, content_span])
    }

    pub(super) fn message_lines_to_plain_preview(lines: &[MessageLine]) -> String {
        let mut segments: Vec<String> = Vec::new();
        for line in lines {
            match line.kind {
                MessageLineKind::Blank => continue,
                MessageLineKind::Metadata => continue,
                _ => {
                    let mut text = String::new();
                    for span in &line.spans {
                        text.push_str(&span.text);
                    }
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        segments.push(trimmed.to_string());
                    }
                }
            }
            if segments.len() >= Self::MAX_UNDO_CONVERSATION_MESSAGES {
                break;
            }
        }
        let joined = segments.join(" ");
        Self::truncate_preview_text(joined, Self::MAX_UNDO_PREVIEW_CHARS)
    }

    pub(super) fn markdown_to_plain_preview(markdown: &str) -> String {
        let mut segments: Vec<String> = Vec::new();
        for line in markdown.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('#') {
                segments.push(trimmed.trim_start_matches('#').trim().to_string());
            } else {
                segments.push(trimmed.to_string());
            }
            if segments.len() >= Self::MAX_UNDO_CONVERSATION_MESSAGES {
                break;
            }
        }
        if segments.is_empty() {
            return String::new();
        }
        let joined = segments.join(" ");
        Self::truncate_preview_text(joined, Self::MAX_UNDO_PREVIEW_CHARS)
    }

    pub(super) fn truncate_preview_text(text: String, limit: usize) -> String {
        crate::text_formatting::truncate_chars_with_ellipsis(&text, limit)
    }

    pub(super) fn timeline_file_lines_for_commit(&self, commit_id: &str) -> Vec<Line<'static>> {
        match self.git_numstat(["show", "--numstat", "--format=", commit_id]) {
            Ok(entries) => Self::file_change_lines(entries),
            Err(err) => vec![Line::from(Span::styled(
                err,
                Style::default().fg(crate::colors::error()),
            ))],
        }
    }

    pub(super) fn timeline_file_lines_for_current(&self) -> Vec<Line<'static>> {
        match self.git_numstat(["diff", "--numstat", "HEAD"]) {
            Ok(entries) => {
                if entries.is_empty() {
                    vec![Line::from(Span::styled(
                        "Working tree clean",
                        Style::default().fg(crate::colors::text_dim()),
                    ))]
                } else {
                    Self::file_change_lines(entries)
                }
            }
            Err(err) => vec![Line::from(Span::styled(
                err,
                Style::default().fg(crate::colors::error()),
            ))],
        }
    }

    pub(super) fn git_numstat<I, S>(
        &self,
        args: I,
    ) -> Result<Vec<NumstatRow>, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.run_git_command(args, |stdout| {
            let mut out = Vec::new();
            for line in stdout.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let mut parts = trimmed.splitn(3, '\t');
                let added = parts.next();
                let removed = parts.next();
                let path = parts.next();
                if let (Some(added), Some(removed), Some(path)) = (added, removed, path) {
                    out.push((
                        Self::parse_numstat_count(added),
                        Self::parse_numstat_count(removed),
                        path.to_string(),
                    ));
                }
            }
            Ok(out)
        })
    }

    pub(super) fn run_git_command<I, S, F, T>(&self, args: I, parser: F) -> Result<T, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
        F: FnOnce(String) -> Result<T, String>,
    {
        let args_vec: Vec<String> = args.into_iter().map(|s| s.as_ref().to_string()).collect();
        let output = Command::new("git")
            .current_dir(&self.config.cwd)
            .args(&args_vec)
            .output()
            .map_err(|err| format!("git {} failed: {err}", args_vec.join(" ")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let msg = stderr.trim();
            if msg.is_empty() {
                Err(format!(
                    "git {} exited with status {}",
                    args_vec.join(" "),
                    output.status
                ))
            } else {
                Err(msg.to_string())
            }
        } else {
            if args_vec
                .iter()
                .any(|arg| matches!(arg.as_str(), "pull" | "checkout" | "merge" | "apply"))
            {
                bump_snapshot_epoch_for(&self.config.cwd);
            }
            parser(String::from_utf8_lossy(&output.stdout).to_string())
        }
    }

    pub(super) fn parse_numstat_count(raw: &str) -> Option<u32> {
        if raw == "-" {
            None
        } else {
            raw.parse::<u32>().ok()
        }
    }

    pub(super) fn file_change_lines(entries: Vec<(Option<u32>, Option<u32>, String)>) -> Vec<Line<'static>> {
        if entries.is_empty() {
            return vec![Line::from(Span::styled(
                "No file changes recorded for this snapshot.",
                Style::default().fg(crate::colors::text_dim()),
            ))];
        }

        let max_entries = (Self::MAX_UNDO_FILE_LINES / 2).max(1);
        let mut lines: Vec<Line<'static>> = Vec::new();
        for (idx, (added, removed, path)) in entries.iter().enumerate() {
            if idx >= max_entries {
                break;
            }
            lines.push(Line::from(Span::styled(
                path.clone(),
                Style::default().fg(crate::colors::text()),
            )));

            let added_text = added.map_or("-".to_string(), |v| v.to_string());
            let removed_text = removed.map_or("-".to_string(), |v| v.to_string());
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    format!("+{added_text}"),
                    Style::default().fg(crate::colors::success()),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("-{removed_text}"),
                    Style::default().fg(crate::colors::error()),
                ),
            ]));
        }

        if entries.len() > max_entries {
            let remaining = entries.len() - max_entries;
            lines.push(Line::from(Span::styled(
                format!("… and {remaining} more file{}", if remaining == 1 { "" } else { "s" }),
                Style::default().fg(crate::colors::text_dim()),
            )));
        }

        lines
    }

    pub(super) fn reset_resume_order_anchor(&mut self) {
        if self.history_cells.is_empty() {
            self.resume_expected_next_request = None;
        } else {
            let max_req = self
                .cell_order_seq
                .iter()
                .map(|key| key.req)
                .max()
                .unwrap_or(0);
            self.resume_expected_next_request = Some(max_req.saturating_add(1));
        }
        self.order_request_bias = 0;
        self.resume_provider_baseline = None;
    }

    pub(crate) fn restore_history_snapshot(&mut self, snapshot: &HistorySnapshot) {
        let perf_timer = self.perf_state.enabled.then(Instant::now);
        let preserved_system_entries: Vec<(String, HistoryId)> = self
            .system_cell_by_id
            .iter()
            .filter_map(|(key, &idx)| {
                self.history_cell_ids
                    .get(idx)
                    .and_then(|maybe| maybe.map(|hid| (key.clone(), hid)))
            })
            .collect();
        self.history_debug(format!(
            "restore_history_snapshot.start records={} cells_before={} order_before={}",
            snapshot.records.len(),
            self.history_cells.len(),
            self.cell_order_seq.len()
        ));
        self.history_state.restore(snapshot);

        self.history_render.invalidate_all();
        self.mark_render_requests_dirty();
        self.history_prefix_append_only.set(false);

        self.history_cells.clear();
        self.history_cell_ids.clear();
        self.history_live_window = None;
        self.history_frozen_width = 0;
        self.history_frozen_count = 0;
        self.history_virtualization_sync_pending.set(false);
        self.cell_order_seq.clear();
        self.cell_order_dbg.clear();

        for record in &self.history_state.records {
            if let Some(mut cell) = self.build_cell_from_record(record) {
                let id = record.id();
                Self::assign_history_id_inner(&mut cell, id);
                self.history_cells.push(cell);
                self.history_cell_ids.push(Some(id));
            } else {
                tracing::warn!("unable to rebuild history cell for record id {:?}", record.id());
                let fallback = history_cell::new_background_event(format!(
                    "Restored snapshot missing renderer for record {:?}",
                    record.id()
                ));
                self.history_cells.push(Box::new(fallback));
                self.history_cell_ids.push(None);
            }
        }

        if !snapshot.order.is_empty() {
            self.cell_order_seq = snapshot
                .order
                .iter()
                .copied()
                .map(OrderKey::from)
                .collect();
        } else {
            self.cell_order_seq = self
                .history_cells
                .iter()
                .enumerate()
                .map(|(idx, _)| OrderKey {
                    req: (idx as u64).saturating_add(1),
                    out: i32::MAX,
                    seq: (idx as u64).saturating_add(1),
                })
                .collect();
        }

        if self.cell_order_seq.len() < self.history_cells.len() {
            let mut next_req = self
                .cell_order_seq
                .iter()
                .map(|key| key.req)
                .max()
                .unwrap_or(0);
            let mut next_seq = self
                .cell_order_seq
                .iter()
                .map(|key| key.seq)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            while self.cell_order_seq.len() < self.history_cells.len() {
                next_req = next_req.saturating_add(1);
                self.cell_order_seq.push(OrderKey {
                    req: next_req,
                    out: i32::MAX,
                    seq: next_seq,
                });
                next_seq = next_seq.saturating_add(1);
            }
        }

        if !snapshot.order_debug.is_empty() {
            self.cell_order_dbg = snapshot.order_debug.clone();
        }
        if self.cell_order_dbg.len() < self.history_cells.len() {
            self.cell_order_dbg
                .resize(self.history_cells.len(), None);
        }

        let max_req = self.cell_order_seq.iter().map(|key| key.req).max().unwrap_or(0);
        let max_seq = self.cell_order_seq.iter().map(|key| key.seq).max().unwrap_or(0);

        self.last_seen_request_index = max_req;
        self.current_request_index = max_req;
        self.internal_seq = max_seq;
        self.last_assigned_order = self.cell_order_seq.iter().copied().max();
        self.reset_resume_order_anchor();

        self.rebuild_ui_background_seq_counters();

        running_tools::rehydrate(self);
        self.rehydrate_system_order_cache(&preserved_system_entries);

        self.bottom_pane
            .set_has_chat_history(!self.history_cells.is_empty());
        self.refresh_reasoning_collapsed_visibility();
        self.refresh_explore_trailing_flags();
        self.invalidate_height_cache();
        self.request_redraw();

        if let (true, Some(started)) = (self.perf_state.enabled, perf_timer) {
            let elapsed = started.elapsed().as_nanos();
            self.perf_state
                .stats
                .borrow_mut()
                .record_undo_restore(elapsed);
        }
        self.history_snapshot_dirty = true;
        self.history_snapshot_last_flush = None;

        self.history_debug(format!(
            "restore_history_snapshot.done cells={} order={} system_cells={}",
            self.history_cells.len(),
            self.cell_order_seq.len(),
            self.system_cell_by_id.len()
        ));
    }

    pub(crate) fn perform_undo_restore(
        &mut self,
        commit: Option<&str>,
        restore_files: bool,
        restore_conversation: bool,
    ) {
        let Some(commit_id) = commit else {
            self.push_background_tail("No snapshot selected.".to_string());
            return;
        };

        let Some((index, snapshot)) = self
            .ghost_snapshots
            .iter()
            .enumerate()
            .find(|(_, snap)| snap.commit().id() == commit_id)
            .map(|(idx, snap)| (idx, snap.clone()))
        else {
            self.push_background_tail(
                "Selected snapshot is no longer available.".to_string(),
            );
            return;
        };

        if !restore_files && !restore_conversation {
            self.push_background_tail("No restore options selected.".to_string());
            return;
        }

        let mut files_restored = false;
        let mut conversation_rewind_requested = false;
        let mut errors: Vec<String> = Vec::new();
        let mut pre_restore_snapshot: Option<GhostSnapshot> = None;

        if restore_files {
            let previous_len = self.ghost_snapshots.len();
            let pre_summary = Some("Pre-undo checkpoint".to_string());
            let captured_snapshot = self.capture_ghost_snapshot_blocking(pre_summary);
            let added_snapshot = self.ghost_snapshots.len() > previous_len;
            if let Some(snapshot) = captured_snapshot {
                pre_restore_snapshot = Some(snapshot);
            }

            match restore_ghost_commit(&self.config.cwd, snapshot.commit()) {
                Ok(()) => {
                    files_restored = true;
                    self.ghost_snapshots.truncate(index);
                    if let Some(pre) = pre_restore_snapshot {
                        self.ghost_snapshots.push(pre);
                        if self.ghost_snapshots.len() > MAX_TRACKED_GHOST_COMMITS {
                            self.ghost_snapshots.remove(0);
                        }
                    }
                }
                Err(err) => {
                    if added_snapshot && !self.ghost_snapshots.is_empty() {
                        self.ghost_snapshots.pop();
                    }
                    errors.push(format!("Failed to restore workspace files: {err}"));
                }
            }
        }

        if restore_conversation {
            let (user_delta, assistant_delta) =
                self.conversation_delta_since(&snapshot.conversation);
            if user_delta == 0 {
                self.push_background_tail(
                    "Conversation already matches selected snapshot; nothing to rewind.".to_string(),
                );
            } else {
                self.app_event_tx.send(AppEvent::JumpBack {
                    nth: user_delta,
                    prefill: String::new(),
                    history_snapshot: Some(snapshot.history.clone()),
                });
                if assistant_delta > 0 {
                    self.push_background_tail(format!(
                        "Rewinding conversation by {} user turn{} and {} assistant repl{}",
                        user_delta,
                        if user_delta == 1 { "" } else { "s" },
                        assistant_delta,
                        if assistant_delta == 1 { "y" } else { "ies" }
                    ));
                } else {
                    self.push_background_tail(format!(
                        "Rewinding conversation by {} user turn{}",
                        user_delta,
                        if user_delta == 1 { "" } else { "s" }
                    ));
                }
                conversation_rewind_requested = true;
            }
        }

        for err in errors {
            self.history_push_plain_state(history_cell::new_error_event(err));
        }

        if files_restored {
            let mut message = format!("Restored workspace files to snapshot {}", snapshot.short_id());
            if let Some(snippet) = snapshot.summary_snippet(60) {
                message.push_str(&format!(" • {snippet}"));
            }
            if let Some(age) = snapshot.age_from(Local::now()) {
                message.push_str(&format!(" • captured {} ago", format_duration(age)));
            }
            if !restore_conversation {
                message.push_str(" • chat history unchanged");
            }
            self.push_background_tail(message);
        }

        if conversation_rewind_requested {
            // Ensure Auto Drive state does not point at the old session after a conversation rewind.
            // If we leave it active, subsequent user messages may be routed to a stale coordinator
            // handle and appear to "not go through".
            if self.auto_state.is_active() || self.auto_handle.is_some() {
                self.auto_stop(Some("Auto Drive reset after /undo restore.".to_string()));
                self.auto_handle = None;
                self.auto_history.clear();
            }

            // Conversation rewind will reload the chat widget via AppEvent::JumpBack.
            self.reset_after_conversation_restore();
        } else {
            // Even when only files are restored, clear any pending user prompts or transient state
            // so subsequent messages flow normally.
            self.reset_after_conversation_restore();
        }

        self.request_redraw();
    }

    pub(super) fn reset_after_conversation_restore(&mut self) {
        self.pending_dispatched_user_messages.clear();
        self.pending_user_prompts_for_next_turn = 0;
        self.queued_user_messages.clear();
        self.refresh_queued_user_messages(false);
        self.bottom_pane.clear_composer();
        self.bottom_pane.clear_ctrl_c_quit_hint();
        self.bottom_pane.clear_live_ring();
        self.bottom_pane.set_task_running(false);
        self.active_task_ids.clear();
        if !self.agents_terminal.active {
            self.bottom_pane.ensure_input_focus();
        }
    }

    pub(super) fn flush_pending_agent_notes(&mut self) {
        for note in self.pending_agent_notes.drain(..) {
            if let Err(e) = self.code_op_tx.send(Op::AddToHistory { text: note }) {
                tracing::error!("failed to send AddToHistory op: {e}");
            }
        }
    }

    pub(super) fn finalize_sent_user_message(&mut self, message: UserMessage) {
        let UserMessage {
            display_text,
            ordered_items,
            suppress_persistence,
        } = message;

        let combined_message_text = {
            let mut buffer = String::new();
            for item in &ordered_items {
                if let InputItem::Text { text } = item {
                    if !buffer.is_empty() {
                        buffer.push('\n');
                    }
                    buffer.push_str(text);
                }
            }
            let trimmed = buffer.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };

        if !display_text.is_empty() {
            let key = self.next_req_key_prompt();
            let state = history_cell::new_user_prompt(display_text.clone());
            let _ = self.history_insert_plain_state_with_key(state, key, "prompt");
            self.pending_user_prompts_for_next_turn =
                self.pending_user_prompts_for_next_turn.saturating_add(1);
        }

        self.flush_pending_agent_notes();

        if let Some(model_echo) = combined_message_text {
            self.pending_dispatched_user_messages.push_back(model_echo);
        }

        let suppress_history = suppress_persistence;

        if !display_text.is_empty() && !suppress_history
            && let Err(e) = self
                .code_op_tx
                .send(Op::AddToHistory { text: display_text })
            {
                tracing::error!("failed to send AddHistory op: {e}");
            }

        if self.auto_state.is_active() && self.auto_state.resume_after_submit() {
            self.auto_state.on_prompt_submitted();
            self.auto_state.seconds_remaining = 0;
            self.auto_rebuild_live_ring();
            self.bottom_pane.update_status_text(String::new());
            self.bottom_pane.set_task_running(false);
        }

        self.request_redraw();
    }

    pub(super) fn refresh_queued_user_messages(&mut self, schedule_watchdog: bool) {
        let mut scheduled_watchdog = false;
        if self.queued_user_messages.is_empty() {
            self.queue_block_started_at = None;
        } else if schedule_watchdog
            && self.queue_block_started_at.is_none() {
                self.queue_block_started_at = Some(Instant::now());
                scheduled_watchdog = true;
            }

        if scheduled_watchdog {
            let tx = self.app_event_tx.clone();
            // Fire a CommitTick after ~10s to ensure the watchdog runs even when
            // no streaming/animation is active.
            if thread_spawner::spawn_lightweight("queue-watchdog", move || {
                std::thread::sleep(Duration::from_secs(10));
                tx.send(crate::app_event::AppEvent::CommitTick);
            })
            .is_none()
            {
                // If we cannot spawn another lightweight thread (e.g., thread cap reached),
                // fall back to a non-threaded timer using tokio when available, or a best-effort
                // regular thread; as a last resort mark the timer expired and send immediately so
                // the queue cannot remain blocked.
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    let tx = self.app_event_tx.clone();
                    handle.spawn(async move {
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        tx.send(crate::app_event::AppEvent::CommitTick);
                    });
                } else {
                    let tx = self.app_event_tx.clone();
                    if std::thread::Builder::new()
                        .name("queue-watchdog-fallback".to_string())
                        .spawn(move || {
                            std::thread::sleep(Duration::from_secs(10));
                            tx.send(crate::app_event::AppEvent::CommitTick);
                        })
                        .is_err()
                    {
                        // No way to schedule a delayed tick; force the timer to appear expired
                        // and emit a tick now to avoid indefinite blocking.
                        self.queue_block_started_at = Some(Instant::now() - Duration::from_secs(10));
                        self.app_event_tx.send(crate::app_event::AppEvent::CommitTick);
                    }
                }
            }
        }

        self.request_redraw();
    }

    #[allow(dead_code)]
    pub(crate) fn set_mouse_status_message(&mut self, message: &str) {
        self.bottom_pane.update_status_text(message.to_string());
    }

    pub(crate) fn handle_mouse_event(&mut self, mouse_event: crossterm::event::MouseEvent) {
        use crossterm::event::KeyModifiers;
        use crossterm::event::MouseEventKind;

        // Check if Shift is held - if so, let the terminal handle selection
        if mouse_event.modifiers.contains(KeyModifiers::SHIFT) {
            // Don't handle any mouse events when Shift is held
            // This allows the terminal's native text selection to work
            return;
        }

        // Settings overlay is modal: while visible, mouse input must be consumed
        // by the overlay layer and must not leak to global history/pane scrolling.
        if let Some(overlay) = self.settings.overlay.as_mut() {
            // Settings overlay covers the full screen - compute terminal area from layout
            let terminal_area = Rect {
                x: 0,
                y: 0,
                width: self.layout.last_frame_width.get(),
                height: self.layout.last_frame_height.get(),
            };
            let changed = overlay.handle_mouse_event(mouse_event, terminal_area);
            if changed {
                self.sync_limits_layout_mode_preference();
                self.request_redraw();
            }
            return;
        }

        let bottom_pane_area = self.layout.last_bottom_pane_area.get();
        let mouse_pos = (mouse_event.column, mouse_event.row);

        // Helper: check if mouse is inside bottom pane area
        let in_bottom_pane = mouse_event.row >= bottom_pane_area.y
            && mouse_event.row < bottom_pane_area.y + bottom_pane_area.height
            && mouse_event.column >= bottom_pane_area.x
            && mouse_event.column < bottom_pane_area.x + bottom_pane_area.width;

        match mouse_event.kind {
            MouseEventKind::ScrollUp => {
                // If scroll is in the bottom pane area, forward it there first
                if in_bottom_pane {
                    let (input_result, needs_redraw) = self.bottom_pane.handle_mouse_event(mouse_event, bottom_pane_area);
                    if needs_redraw {
                        self.process_mouse_input_result(input_result);
                        return;
                    }
                }
                // Otherwise, scroll the history
                layout_scroll::mouse_scroll(self, true)
            }
            MouseEventKind::ScrollDown => {
                // If scroll is in the bottom pane area, forward it there first
                if in_bottom_pane {
                    let (input_result, needs_redraw) = self.bottom_pane.handle_mouse_event(mouse_event, bottom_pane_area);
                    if needs_redraw {
                        self.process_mouse_input_result(input_result);
                        return;
                    }
                }
                // Otherwise, scroll the history
                layout_scroll::mouse_scroll(self, false)
            }
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                // First check if click is inside the bottom pane area
                if in_bottom_pane {
                    // Forward click to bottom pane
                    let (input_result, needs_redraw) = self.bottom_pane.handle_mouse_event(mouse_event, bottom_pane_area);
                    if needs_redraw {
                        self.process_mouse_input_result(input_result);
                        return;
                    }
                }
                // Handle left click by checking clickable regions (header bar, etc.)
                self.handle_click(mouse_pos);
            }
            MouseEventKind::Moved => {
                let mut needs_redraw = false;
                if self.update_header_hover_state(mouse_pos) {
                    needs_redraw = true;
                }
                // Update hover state in bottom pane.
                if in_bottom_pane
                    && self.bottom_pane.update_hover(mouse_pos, bottom_pane_area)
                {
                    needs_redraw = true;
                }
                if needs_redraw {
                    self.request_redraw();
                }
            }
            _ => {
                // Ignore other mouse events for now
            }
        }
    }

    /// Process InputResult from mouse events (similar to key event handling).
    pub(super) fn process_mouse_input_result(&mut self, input_result: InputResult) {
        match input_result {
            InputResult::Submitted(text) => {
                if let Some(pending) = self.pending_request_user_input.take() {
                    self.submit_request_user_input_answer(pending, text);
                    return;
                }
                self.pending_turn_origin = Some(TurnOrigin::User);
                let cleaned = Self::strip_context_sections(&text);
                self.last_user_message = (!cleaned.trim().is_empty()).then_some(cleaned);
                let user_message = self.parse_message_with_images(text);
                self.submit_user_message(user_message);
            }
            InputResult::Command(_cmd) => {
                // Command was dispatched at the App layer; request redraw.
                self.app_event_tx.send(AppEvent::RequestRedraw);
            }
            InputResult::None | InputResult::ScrollUp | InputResult::ScrollDown => {
                self.request_redraw();
            }
        }
    }

    pub(super) fn handle_click(&mut self, pos: (u16, u16)) {
        let (x, y) = pos;
        
        // Check clickable regions from last render and find matching action
        let action_opt: Option<ClickableAction> = {
            let regions = self.clickable_regions.borrow();
            
            regions.iter().find_map(|region| {
                // Check if click is inside this region
                if x >= region.rect.x
                    && x < region.rect.x + region.rect.width
                    && y >= region.rect.y
                    && y < region.rect.y + region.rect.height
                {
                    Some(region.action.clone())
                } else {
                    None
                }
            })
        };
        
        // Execute the action after dropping the borrow
        if let Some(action) = action_opt {
            match action {
                ClickableAction::ShowModelSelector => {
                    // Open model selector with empty args (opens selector UI)
                    self.handle_model_command(String::new());
                }
                ClickableAction::ShowShellSelector => {
                    self.app_event_tx.send(AppEvent::ShowShellSelector);
                }
                ClickableAction::ShowReasoningSelector => {
                    // Cycle through reasoning efforts
                    use code_core::config_types::ReasoningEffort;
                    let current = self.config.model_reasoning_effort;
                    let next = match current {
                        ReasoningEffort::None => ReasoningEffort::Minimal,
                        ReasoningEffort::Minimal => ReasoningEffort::Low,
                        ReasoningEffort::Low => ReasoningEffort::Medium,
                        ReasoningEffort::Medium => ReasoningEffort::High,
                        ReasoningEffort::High => ReasoningEffort::XHigh,
                        ReasoningEffort::XHigh => ReasoningEffort::None,
                    };
                    self.set_reasoning_effort(next);
                }
                ClickableAction::ExecuteCommand(cmd) => {
                    // Parse and dispatch the slash command
                    let trimmed = cmd.trim_start_matches('/').trim();
                    if let Ok(slash_cmd) = trimmed.parse::<SlashCommand>() {
                        self.app_event_tx.send(AppEvent::DispatchCommand(slash_cmd, cmd));
                    }
                }
            }
        }
    }

    fn update_header_hover_state(&mut self, pos: (u16, u16)) -> bool {
        let (x, y) = pos;
        let hovered = {
            let regions = self.clickable_regions.borrow();
            regions.iter().find_map(|region| {
                if x >= region.rect.x
                    && x < region.rect.x + region.rect.width
                    && y >= region.rect.y
                    && y < region.rect.y + region.rect.height
                {
                    Some(region.action.clone())
                } else {
                    None
                }
            })
        };
        let mut current = self.hovered_clickable_action.borrow_mut();
        if *current == hovered {
            false
        } else {
            *current = hovered;
            true
        }
    }
}
