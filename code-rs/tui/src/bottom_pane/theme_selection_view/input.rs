use super::*;

impl ThemeSelectionView {
    pub(super) fn process_key_event(&mut self, key_event: KeyEvent) {
        match key_event {
            KeyEvent { code: KeyCode::Up, modifiers: KeyModifiers::NONE, .. } => {
                if let Mode::CreateSpinner(ref mut s) = self.mode {
                    let new_step = match s.step.get() {
                        CreateStep::Prompt => CreateStep::Action,
                        CreateStep::Action => {
                            if s.action_idx > 0 {
                                s.action_idx -= 1;
                            }
                            CreateStep::Action
                        }
                        CreateStep::Review => {
                            if s.action_idx > 0 {
                                s.action_idx -= 1;
                            }
                            CreateStep::Review
                        }
                    };
                    s.step.set(new_step);
                } else if let Mode::CreateTheme(ref mut s) = self.mode {
                    match s.step.get() {
                        CreateStep::Prompt => s.step.set(CreateStep::Action),
                        CreateStep::Action => {
                            if s.action_idx > 0 {
                                s.action_idx -= 1;
                            }
                        }
                        CreateStep::Review => {
                            s.review_focus_is_toggle.set(true);
                        }
                    }
                } else {
                    match self.mode {
                        Mode::Overview => {
                            self.overview_selected_index =
                                self.overview_selected_index.saturating_sub(1) % 3;
                        }
                        _ => self.move_selection_up(),
                    }
                }
            }
            KeyEvent { code: KeyCode::Down, modifiers: KeyModifiers::NONE, .. } => {
                if let Mode::CreateSpinner(ref mut s) = self.mode {
                    let new_step = match s.step.get() {
                        CreateStep::Prompt => CreateStep::Action,
                        CreateStep::Action => {
                            if s.action_idx < 1 {
                                s.action_idx += 1;
                            }
                            CreateStep::Action
                        }
                        CreateStep::Review => {
                            if s.action_idx < 1 {
                                s.action_idx += 1;
                            }
                            CreateStep::Review
                        }
                    };
                    s.step.set(new_step);
                } else if let Mode::CreateTheme(ref mut s) = self.mode {
                    match s.step.get() {
                        CreateStep::Prompt => s.step.set(CreateStep::Action),
                        CreateStep::Action => {
                            if s.action_idx < 1 {
                                s.action_idx += 1;
                            }
                        }
                        CreateStep::Review => {
                            s.review_focus_is_toggle.set(false);
                        }
                    }
                } else {
                    match &self.mode {
                        Mode::Overview => {
                            self.overview_selected_index = (self.overview_selected_index + 1) % 3;
                        }
                        _ => self.move_selection_down(),
                    }
                }
            }
            KeyEvent { code: KeyCode::Left, modifiers: KeyModifiers::NONE, .. } => {
                if let Mode::CreateSpinner(ref mut s) = self.mode {
                    let new_step = match s.step.get() {
                        CreateStep::Prompt => CreateStep::Action,
                        CreateStep::Action => {
                            if s.action_idx > 0 {
                                s.action_idx -= 1;
                            }
                            CreateStep::Action
                        }
                        CreateStep::Review => {
                            if s.action_idx > 0 {
                                s.action_idx -= 1;
                            }
                            CreateStep::Review
                        }
                    };
                    s.step.set(new_step);
                } else if let Mode::CreateTheme(ref mut s) = self.mode {
                    match s.step.get() {
                        CreateStep::Prompt => s.step.set(CreateStep::Action),
                        CreateStep::Action => {
                            if s.action_idx > 0 {
                                s.action_idx -= 1;
                            }
                        }
                        CreateStep::Review => {
                            s.review_focus_is_toggle.set(true);
                        }
                    }
                } else {
                    match self.mode {
                        Mode::Overview => {
                            self.overview_selected_index =
                                self.overview_selected_index.saturating_sub(1) % 3;
                        }
                        _ => self.move_selection_up(),
                    }
                }
            }
            KeyEvent { code: KeyCode::Right, modifiers: KeyModifiers::NONE, .. } => {
                if let Mode::CreateSpinner(ref mut s) = self.mode {
                    let new_step = match s.step.get() {
                        CreateStep::Prompt => CreateStep::Action,
                        CreateStep::Action => {
                            if s.action_idx < 1 {
                                s.action_idx += 1;
                            }
                            CreateStep::Action
                        }
                        CreateStep::Review => {
                            if s.action_idx < 1 {
                                s.action_idx += 1;
                            }
                            CreateStep::Review
                        }
                    };
                    s.step.set(new_step);
                } else if let Mode::CreateTheme(ref mut s) = self.mode {
                    match s.step.get() {
                        CreateStep::Prompt => s.step.set(CreateStep::Action),
                        CreateStep::Action => {
                            if s.action_idx < 1 {
                                s.action_idx += 1;
                            }
                        }
                        CreateStep::Review => {
                            s.review_focus_is_toggle.set(false);
                        }
                    }
                } else {
                    match &self.mode {
                        Mode::Overview => {
                            self.overview_selected_index = (self.overview_selected_index + 1) % 3;
                        }
                        _ => self.move_selection_down(),
                    }
                }
            }
            KeyEvent { code: KeyCode::Enter, modifiers: KeyModifiers::NONE, .. } => {
                let current_mode = std::mem::replace(&mut self.mode, Mode::Overview);
                match current_mode {
                    Mode::Overview => {
                        match self.overview_selected_index {
                            0 => {
                                self.revert_theme_on_back = self.current_theme;
                                self.hovered_theme_index = None;
                                self.mode = Mode::Themes;
                                self.just_entered_themes = true;
                                self.send_theme_split_preview();
                            }
                            1 => {
                                self.revert_spinner_on_back = self.current_spinner.clone();
                                self.mode = Mode::Spinner;
                                self.app_event_tx.send(AppEvent::ScheduleFrameIn(
                                    std::time::Duration::from_millis(120),
                                ));
                                self.just_entered_spinner = true;
                            }
                            _ => {
                                self.is_complete = true;
                                self.mode = Mode::Overview;
                            }
                        }
                    }
                    Mode::Themes => {
                        let count = Self::theme_option_count();
                        if Self::allow_custom_theme_generation()
                            && self.selected_theme_index >= count
                        {
                            self.app_event_tx
                                .send(AppEvent::PreviewTheme(self.revert_theme_on_back));
                            self.clear_theme_split_preview();
                            self.mode = Mode::CreateTheme(CreateThemeState {
                                step: std::cell::Cell::new(CreateStep::Prompt),
                                prompt: String::new(),
                                is_loading: std::cell::Cell::new(false),
                                action_idx: 0,
                                rx: None,
                                thinking_lines: std::cell::RefCell::new(Vec::new()),
                                thinking_current: std::cell::RefCell::new(String::new()),
                                proposed_name: std::cell::RefCell::new(None),
                                proposed_colors: std::cell::RefCell::new(None),
                                preview_on: std::cell::Cell::new(true),
                                review_focus_is_toggle: std::cell::Cell::new(true),
                                last_raw_output: std::cell::RefCell::new(None),
                                proposed_is_dark: std::cell::Cell::new(None),
                            });
                        } else {
                            self.confirm_theme()
                        }
                    }
                    Mode::Spinner => {
                        let names = crate::spinner::spinner_names();
                        if self.selected_spinner_index > names.len() {
                            self.selected_spinner_index = names.len().saturating_sub(1);
                        }
                        if self.selected_spinner_index >= names.len() {
                            self.mode = Mode::CreateSpinner(CreateState {
                                step: std::cell::Cell::new(CreateStep::Prompt),
                                prompt: String::new(),
                                is_loading: std::cell::Cell::new(false),
                                action_idx: 0,
                                rx: None,
                                thinking_lines: std::cell::RefCell::new(Vec::new()),
                                thinking_current: std::cell::RefCell::new(String::new()),
                                proposed_interval: std::cell::Cell::new(None),
                                proposed_frames: std::cell::RefCell::new(None),
                                proposed_name: std::cell::RefCell::new(None),
                                last_raw_output: std::cell::RefCell::new(None),
                            });
                        } else {
                            self.confirm_spinner()
                        }
                    }
                    Mode::CreateSpinner(mut s) => {
                        let mut go_overview = false;
                        match s.step.get() {
                            CreateStep::Prompt => {
                                if !s.is_loading.get() {
                                    let user_prompt = s.prompt.clone();
                                    s.is_loading.set(true);
                                    s.thinking_lines.borrow_mut().clear();
                                    s.thinking_current.borrow_mut().clear();
                                    s.proposed_interval.set(None);
                                    s.proposed_frames.replace(None);
                                    let (txp, rxp) = std::sync::mpsc::channel::<ProgressMsg>();
                                    s.rx = Some(rxp);
                                    self.kickoff_spinner_creation(user_prompt, txp);
                                    self.app_event_tx.send(AppEvent::RequestRedraw);
                                }
                            }
                            CreateStep::Action => {
                                if s.action_idx == 0 && !s.is_loading.get() {
                                    let user_prompt = s.prompt.clone();
                                    s.is_loading.set(true);
                                    s.thinking_lines.borrow_mut().clear();
                                    s.thinking_current.borrow_mut().clear();
                                    s.proposed_interval.set(None);
                                    s.proposed_frames.replace(None);
                                    let (txp, rxp) = std::sync::mpsc::channel::<ProgressMsg>();
                                    s.rx = Some(rxp);
                                    self.kickoff_spinner_creation(user_prompt, txp);
                                    self.app_event_tx.send(AppEvent::RequestRedraw);
                                } else {
                                    go_overview = true;
                                }
                            }
                            CreateStep::Review => {
                                if s.action_idx == 0 {
                                    if let (Some(interval), Some(frames)) = (
                                        s.proposed_interval.get(),
                                        s.proposed_frames.borrow().clone(),
                                    ) {
                                        let display_name = s
                                            .proposed_name
                                            .borrow()
                                            .as_ref()
                                            .cloned()
                                            .unwrap_or_else(|| "Custom".to_string());
                                        if let Ok(home) = code_core::config::find_code_home() {
                                            let _ = code_core::config::set_custom_spinner(
                                                &home,
                                                "custom",
                                                &display_name,
                                                interval,
                                                &frames,
                                            );
                                        }
                                        crate::spinner::add_custom_spinner(
                                            "custom".to_string(),
                                            display_name,
                                            interval,
                                            frames,
                                        );
                                        crate::spinner::switch_spinner("custom");
                                        self.revert_spinner_on_back = "custom".to_string();
                                        self.current_spinner = "custom".to_string();
                                        self.app_event_tx
                                            .send(AppEvent::UpdateSpinner("custom".to_string()));
                                        self.send_tail("Custom spinner saved".to_string());
                                        go_overview = true;
                                    }
                                } else {
                                    s.thinking_lines.borrow_mut().clear();
                                    s.thinking_current.borrow_mut().clear();
                                    s.proposed_interval.set(None);
                                    s.proposed_frames.replace(None);
                                    s.step.set(CreateStep::Prompt);
                                    self.app_event_tx.send(AppEvent::RequestRedraw);
                                }
                            }
                        }
                        if go_overview {
                            self.mode = Mode::Overview;
                        } else {
                            self.mode = Mode::CreateSpinner(s);
                        }
                    }
                    Mode::CreateTheme(mut s) => {
                        let mut go_overview = false;
                        match s.step.get() {
                            CreateStep::Prompt => {
                                if !s.is_loading.get() {
                                    let user_prompt = s.prompt.clone();
                                    s.is_loading.set(true);
                                    s.thinking_lines.borrow_mut().clear();
                                    s.thinking_current.borrow_mut().clear();
                                    s.proposed_name.replace(None);
                                    s.proposed_colors.replace(None);
                                    let (txp, rxp) = std::sync::mpsc::channel::<ProgressMsg>();
                                    s.rx = Some(rxp);
                                    self.kickoff_theme_creation(user_prompt, txp);
                                    self.app_event_tx.send(AppEvent::RequestRedraw);
                                }
                            }
                            CreateStep::Action => {
                                if s.action_idx == 0 && !s.is_loading.get() {
                                    let user_prompt = s.prompt.clone();
                                    s.is_loading.set(true);
                                    s.thinking_lines.borrow_mut().clear();
                                    s.thinking_current.borrow_mut().clear();
                                    s.proposed_name.replace(None);
                                    s.proposed_colors.replace(None);
                                    let (txp, rxp) = std::sync::mpsc::channel::<ProgressMsg>();
                                    s.rx = Some(rxp);
                                    self.kickoff_theme_creation(user_prompt, txp);
                                    self.app_event_tx.send(AppEvent::RequestRedraw);
                                } else {
                                    go_overview = true;
                                }
                            }
                            CreateStep::Review => {
                                if s.review_focus_is_toggle.get() {
                                    let now_on = !s.preview_on.get();
                                    s.preview_on.set(now_on);
                                    if now_on {
                                        if let (Some(name), Some(colors)) = (
                                            s.proposed_name.borrow().clone(),
                                            s.proposed_colors.borrow().clone(),
                                        ) {
                                            crate::theme::set_custom_theme_colors(colors.clone());
                                            crate::theme::set_custom_theme_label(name.clone());
                                            crate::theme::init_theme(
                                                &code_core::config_types::ThemeConfig {
                                                    name: ThemeName::Custom,
                                                    colors,
                                                    label: Some(name),
                                                    is_dark: s.proposed_is_dark.get(),
                                                },
                                            );
                                        }
                                    } else {
                                        let fallback = if self.revert_theme_on_back == ThemeName::Custom {
                                            ThemeName::LightPhoton
                                        } else {
                                            self.revert_theme_on_back
                                        };
                                        self.app_event_tx.send(AppEvent::PreviewTheme(fallback));
                                    }
                                    self.app_event_tx.send(AppEvent::RequestRedraw);
                                } else if s.action_idx == 0 {
                                    if let (Some(name), Some(colors)) = (
                                        s.proposed_name.borrow().clone(),
                                        s.proposed_colors.borrow().clone(),
                                    ) {
                                        if let Ok(home) = code_core::config::find_code_home() {
                                            let _ = code_core::config::set_custom_theme(
                                                &home,
                                                &name,
                                                &colors,
                                                s.preview_on.get(),
                                                s.proposed_is_dark.get(),
                                            );
                                        }
                                        crate::theme::set_custom_theme_label(name.clone());
                                        crate::theme::set_custom_theme_colors(colors.clone());
                                        crate::theme::set_custom_theme_is_dark(
                                            s.proposed_is_dark.get(),
                                        );
                                        if s.preview_on.get() {
                                        crate::theme::init_theme(
                                            &code_core::config_types::ThemeConfig {
                                                name: ThemeName::Custom,
                                                colors,
                                                label: Some(name.clone()),
                                                is_dark: s.proposed_is_dark.get(),
                                            },
                                        );
                                            self.revert_theme_on_back = ThemeName::Custom;
                                            self.current_theme = ThemeName::Custom;
                                            self.app_event_tx
                                                .send(AppEvent::UpdateTheme(ThemeName::Custom));
                                        } else {
                                            self.app_event_tx
                                                .send(AppEvent::PreviewTheme(self.revert_theme_on_back));
                                        }
                                        if s.preview_on.get() {
                                            self.send_before_next_output(format!("Set theme to {name}"));
                                        } else {
                                            self.send_before_next_output(format!(
                                                "Saved custom theme {name} (not active)"
                                            ));
                                        }
                                        go_overview = true;
                                    }
                                } else {
                                    s.thinking_lines.borrow_mut().clear();
                                    s.thinking_current.borrow_mut().clear();
                                    s.proposed_name.replace(None);
                                    s.proposed_colors.replace(None);
                                    s.step.set(CreateStep::Prompt);
                                    self.app_event_tx.send(AppEvent::RequestRedraw);
                                    self.app_event_tx
                                        .send(AppEvent::PreviewTheme(self.revert_theme_on_back));
                                }
                            }
                        }
                        if go_overview {
                            self.mode = Mode::Overview;
                        } else {
                            self.mode = Mode::CreateTheme(s);
                        }
                    }
                }
            }
            KeyEvent { code: KeyCode::Esc, modifiers: KeyModifiers::NONE, .. } => match self.mode {
                Mode::Overview => self.is_complete = true,
                Mode::CreateSpinner(_) => {
                    self.mode = Mode::Spinner;
                }
                Mode::CreateTheme(_) => {
                    self.app_event_tx
                        .send(AppEvent::PreviewTheme(self.revert_theme_on_back));
                    self.hovered_theme_index = None;
                    self.mode = Mode::Themes;
                    self.send_theme_split_preview();
                }
                _ => self.cancel_detail(),
            },
            KeyEvent { code: KeyCode::Char(c), modifiers, .. } => {
                if let Mode::CreateSpinner(ref mut s) = self.mode {
                    if s.is_loading.get() {
                        return;
                    }
                    if matches!(modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
                        match s.step.get() {
                            CreateStep::Prompt => s.prompt.push(c),
                            CreateStep::Action | CreateStep::Review => {}
                        }
                    }
                } else if let Mode::CreateTheme(ref mut s) = self.mode {
                    if s.is_loading.get() {
                        return;
                    }
                    if matches!(modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
                        match s.step.get() {
                            CreateStep::Prompt => s.prompt.push(c),
                            CreateStep::Action | CreateStep::Review => {}
                        }
                    }
                }
            }
            KeyEvent { code: KeyCode::Backspace, .. } => {
                if let Mode::CreateSpinner(ref mut s) = self.mode {
                    if s.is_loading.get() {
                        return;
                    }
                    match s.step.get() {
                        CreateStep::Prompt => {
                            if let Some((idx, _)) = s.prompt.grapheme_indices(true).next_back() {
                                s.prompt.truncate(idx);
                            } else {
                                s.prompt.clear();
                            }
                        }
                        CreateStep::Action | CreateStep::Review => {}
                    }
                } else if let Mode::CreateTheme(ref mut s) = self.mode {
                    if s.is_loading.get() {
                        return;
                    }
                    match s.step.get() {
                        CreateStep::Prompt => {
                            if let Some((idx, _)) = s.prompt.grapheme_indices(true).next_back() {
                                s.prompt.truncate(idx);
                            } else {
                                s.prompt.clear();
                            }
                        }
                        CreateStep::Action | CreateStep::Review => {}
                    }
                }
            }
            _ => {}
        }
    }

    pub(crate) fn handle_key_event_direct(&mut self, key_event: KeyEvent) -> bool {
        let handled = matches!(
            key_event,
            KeyEvent { code: KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Esc, .. }
                | KeyEvent { code: KeyCode::Backspace, .. }
        ) || matches!(
            key_event,
            KeyEvent { code: KeyCode::Char(_), modifiers: KeyModifiers::NONE | KeyModifiers::SHIFT, .. }
        );
        self.process_key_event(key_event);
        handled
    }

    pub(crate) fn handle_mouse_event_direct(&mut self, mouse_event: MouseEvent, area: Rect) -> bool {
        if area.width == 0 || area.height == 0 {
            return false;
        }

        let inner = Block::default().borders(Borders::ALL).inner(area);
        let body_area = inner.inner(Margin::new(1, 1));
        if body_area.width == 0 || body_area.height == 0 {
            return false;
        }

        let in_body = mouse_event.column >= body_area.x
            && mouse_event.column < body_area.x.saturating_add(body_area.width)
            && mouse_event.row >= body_area.y
            && mouse_event.row < body_area.y.saturating_add(body_area.height);

        match mouse_event.kind {
            MouseEventKind::Moved => {
                if !in_body {
                    if matches!(self.mode, Mode::Themes) {
                        return self.clear_hovered_theme_preview();
                    }
                    return false;
                }
                match self.mode {
                    Mode::Themes => self.handle_mouse_hover(mouse_event, body_area),
                    // Keep spinner list scrolling on wheel/click only.
                    Mode::Spinner => false,
                    Mode::Overview | Mode::CreateSpinner(_) | Mode::CreateTheme(_) => {
                        self.handle_mouse_hover(mouse_event, body_area)
                    }
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if !in_body {
                    return false;
                }
                self.handle_mouse_click(mouse_event, body_area)
            }
            MouseEventKind::ScrollUp => {
                if !in_body {
                    return false;
                }
                self.process_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
                true
            }
            MouseEventKind::ScrollDown => {
                if !in_body {
                    return false;
                }
                self.process_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
                true
            }
            _ => false,
        }
    }

    fn handle_mouse_hover(&mut self, mouse_event: MouseEvent, body_area: Rect) -> bool {
        let rel_y = mouse_event.row.saturating_sub(body_area.y) as usize;
        match self.mode {
            Mode::Overview => {
                let Some(next) = (match rel_y {
                    0 => Some(0),
                    1 => Some(1),
                    3 => Some(2),
                    _ => None,
                }) else {
                    return false;
                };
                if self.overview_selected_index == next {
                    return false;
                }
                self.overview_selected_index = next;
                true
            }
            Mode::Themes => {
                let list_area = body_area;
                let option_count = Self::theme_option_count();
                let Some(next) = self.theme_index_at_mouse_position(mouse_event, list_area, option_count)
                else {
                    return self.clear_hovered_theme_preview();
                };

                if next >= option_count {
                    return self.clear_hovered_theme_preview();
                }

                if self.hovered_theme_index == Some(next) {
                    return false;
                }

                self.hovered_theme_index = Some(next);
                self.send_theme_split_preview();
                true
            }
            Mode::Spinner => {
                if rel_y < 2 {
                    return false;
                }
                let names = crate::spinner::spinner_names();
                let count = names.len().saturating_add(1);
                if count == 0 {
                    return false;
                }
                let visible = (body_area.height as usize).saturating_sub(2).clamp(1, 9);
                let (start, _, _) = crate::util::list_window::anchored_window(
                    self.selected_spinner_index,
                    count,
                    visible,
                );
                let row = rel_y.saturating_sub(2);
                let next = start + row;
                if next >= count || self.selected_spinner_index == next {
                    return false;
                }
                self.selected_spinner_index = next;
                if let Some(name) = names.get(next) {
                    self.current_spinner = name.clone();
                    self.app_event_tx
                        .send(AppEvent::PreviewSpinner(self.current_spinner.clone()));
                }
                true
            }
            Mode::CreateSpinner(_) | Mode::CreateTheme(_) => false,
        }
    }

    fn handle_mouse_click(&mut self, mouse_event: MouseEvent, body_area: Rect) -> bool {
        match self.mode {
            Mode::Overview | Mode::Spinner => {
                let changed = self.handle_mouse_hover(mouse_event, body_area);
                self.process_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                let _ = changed;
                true
            }
            Mode::Themes => {
                let list_area = body_area;
                let option_count = Self::theme_option_count();
                let Some(next) = self.theme_index_at_mouse_position(mouse_event, list_area, option_count)
                else {
                    return false;
                };

                self.selected_theme_index = next;
                if let Some(theme_name) = Self::theme_name_for_option_index(next) {
                    self.current_theme = theme_name;
                    self.hovered_theme_index = Some(next);
                    self.send_theme_split_preview();
                } else {
                    let _ = self.clear_hovered_theme_preview();
                }

                self.process_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
                true
            }
            Mode::CreateSpinner(_) | Mode::CreateTheme(_) => {
                self.handle_mouse_hover(mouse_event, body_area)
            }
        }
    }
}

impl Drop for ThemeSelectionView {
    fn drop(&mut self) {
        self.clear_theme_split_preview();
    }
}
