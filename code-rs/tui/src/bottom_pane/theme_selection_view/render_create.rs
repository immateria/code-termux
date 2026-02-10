use super::*;

impl ThemeSelectionView {
    fn append_delta_with_line_wrap(
        current: &mut String,
        history: &mut Vec<String>,
        delta: &str,
    ) {
        current.push_str(delta);
        if let Some(pos) = current.rfind('\n') {
            let (complete, remainder) = current.split_at(pos);
            if !complete.trim().is_empty() {
                history.push(complete.trim().to_string());
            }
            *current = remainder.trim_start_matches('\n').to_string();
            let keep = 10usize;
            let len = history.len();
            if len > keep {
                history.drain(0..len - keep);
            }
        }
    }

    fn latest_progress_line(current: &str, history: &[String]) -> String {
        if !current.trim().is_empty() {
            current.trim().to_string()
        } else {
            history
                .iter()
                .rev()
                .find(|line| !line.trim().is_empty())
                .cloned()
                .unwrap_or_else(|| "Waiting for model…".to_string())
        }
    }

    pub(super) fn render_create_spinner_mode(
        &self,
        body_area: Rect,
        theme: &crate::theme::Theme,
        buf: &mut Buffer,
    ) {
        let Mode::CreateSpinner(s) = &self.mode else {
            return;
        };

        if let Some(rx) = &s.rx {
            for _ in 0..100 {
                // Limit per render to keep UI responsive.
                match rx.try_recv() {
                    Ok(ProgressMsg::ThinkingDelta(delta)) | Ok(ProgressMsg::OutputDelta(delta)) => {
                        if let Mode::CreateSpinner(state) = &self.mode {
                            let mut current = state.thinking_current.borrow_mut();
                            let mut history = state.thinking_lines.borrow_mut();
                            Self::append_delta_with_line_wrap(&mut current, &mut history, &delta);
                        }
                        self.app_event_tx.send(AppEvent::RequestRedraw);
                    }
                    Ok(ProgressMsg::RawOutput(raw)) => {
                        if let Mode::CreateSpinner(state) = &self.mode {
                            state.last_raw_output.replace(Some(raw));
                        }
                    }
                    Ok(ProgressMsg::SetStatus(status)) => {
                        if let Mode::CreateSpinner(state) = &self.mode {
                            let mut current = state.thinking_current.borrow_mut();
                            current.clear();
                            current.push_str(&status);
                        }
                        self.app_event_tx.send(AppEvent::RequestRedraw);
                    }
                    Ok(ProgressMsg::CompletedOk {
                        name,
                        interval,
                        frames,
                    }) => {
                        if let Mode::CreateSpinner(state) = &self.mode {
                            state.is_loading.set(false);
                            state.step.set(CreateStep::Review);
                            state.proposed_interval.set(Some(interval));
                            state.proposed_frames.replace(Some(frames));
                            state.proposed_name.replace(Some(name));
                        }
                        self.app_event_tx.send(AppEvent::RequestRedraw);
                    }
                    Ok(ProgressMsg::CompletedErr {
                        error,
                        _raw_snippet: _,
                    }) => {
                        if let Mode::CreateSpinner(state) = &self.mode {
                            state.is_loading.set(false);
                            state.step.set(CreateStep::Action);
                            state
                                .thinking_lines
                                .borrow_mut()
                                .push(format!("Error: {error}"));
                            state.thinking_current.borrow_mut().clear();
                        }
                        self.app_event_tx.send(AppEvent::RequestRedraw);
                    }
                    Ok(ProgressMsg::CompletedThemeOk(..)) => {}
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
        }

        let mut form_lines = Vec::new();

        if s.is_loading.get() {
            form_lines.push(Line::from(Span::styled(
                "Overview » Change Spinner » Create Custom",
                Style::default()
                    .fg(theme.text_bright)
                    .add_modifier(Modifier::BOLD),
            )));
            form_lines.push(Line::default());

            use std::time::SystemTime;
            use std::time::UNIX_EPOCH;
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let frames = ["◇", "◆", "◇", "◆"];
            let frame = frames[((now_ms / 120) as usize) % frames.len()].to_string();

            form_lines.push(Line::from(vec![
                Span::styled(frame, Style::default().fg(crate::colors::info())),
                Span::styled(
                    " Generating spinner with AI…",
                    Style::default().fg(theme.text_bright),
                ),
            ]));

            let current = s.thinking_current.borrow();
            let history = s.thinking_lines.borrow();
            let mut latest = Self::latest_progress_line(&current, &history);
            if !latest.ends_with('…') {
                latest.push_str(" …");
            }
            form_lines.push(Line::from(Span::styled(
                latest,
                Style::default().fg(theme.text_dim),
            )));

            self.app_event_tx
                .send(AppEvent::ScheduleFrameIn(std::time::Duration::from_millis(
                    120,
                )));
            Paragraph::new(form_lines)
                .alignment(Alignment::Left)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .render(body_area, buf);
            return;
        }

        if matches!(s.step.get(), CreateStep::Review) {
            form_lines.push(Line::from(Span::styled(
                "Overview » Change Spinner » Create Custom",
                Style::default()
                    .fg(theme.text_bright)
                    .add_modifier(Modifier::BOLD),
            )));
            form_lines.push(Line::default());

            if let (Some(interval), Some(frames)) =
                (s.proposed_interval.get(), s.proposed_frames.borrow().as_ref())
            {
                use std::time::SystemTime;
                use std::time::UNIX_EPOCH;
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let idx = if frames.is_empty() {
                    0
                } else {
                    ((now_ms / interval) as usize) % frames.len()
                };
                let preview = frames.get(idx).cloned().unwrap_or_default();

                let max_frame_len: u16 = preview.chars().count() as u16;
                let border = Style::default().fg(crate::colors::border());
                let fg = Style::default().fg(crate::colors::info());
                let x: u16 = max_frame_len.saturating_add(8);
                let border_len = x.saturating_sub(max_frame_len);
                let mut spans: Vec<Span> = Vec::new();
                spans.push(Span::styled("─".repeat(border_len as usize), border));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(preview, fg));
                spans.push(Span::raw(" "));
                spans.push(Span::styled("Preview...", fg));
                spans.push(Span::raw(" "));
                spans.push(Span::styled("─".repeat(border_len as usize), border));
                form_lines.push(Line::from(spans));

                self.app_event_tx
                    .send(AppEvent::ScheduleFrameIn(std::time::Duration::from_millis(
                        interval,
                    )));
            }

            form_lines.push(Line::default());
            let mut buttons: Vec<Span> = Vec::new();
            let save_selected = s.action_idx == 0;
            let retry_selected = s.action_idx == 1;
            let style = |selected: bool| {
                if selected {
                    Style::default()
                        .fg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                }
            };
            buttons.push(Span::styled("[ Save ]", style(save_selected)));
            buttons.push(Span::raw("  "));
            buttons.push(Span::styled("[ Retry ]", style(retry_selected)));
            form_lines.push(Line::from(buttons));

            Paragraph::new(form_lines)
                .alignment(Alignment::Left)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .render(body_area, buf);
            return;
        }

        form_lines.push(Line::from(Span::styled(
            "Overview » Change Spinner » Create Custom",
            Style::default()
                .fg(theme.text_bright)
                .add_modifier(Modifier::BOLD),
        )));
        form_lines.push(Line::default());
        form_lines.push(Line::from(Span::styled(
            "Code can generate a custom spinner just for you!",
            Style::default().fg(theme.text),
        )));
        form_lines.push(Line::from(Span::styled(
            "What sort of spinner would you like? (e.g. bouncing dot party, cat eating a pizza)",
            Style::default().fg(theme.text_dim),
        )));
        form_lines.push(Line::default());

        if let Some(last) = s.thinking_lines.borrow().last().cloned()
            && last.starts_with("Error:") {
                form_lines.push(Line::from(Span::styled(
                    last,
                    Style::default().fg(crate::colors::error()),
                )));
                if let Some(raw) = s.last_raw_output.borrow().as_ref() {
                    form_lines.push(Line::from(Span::styled(
                        "Model output (raw):",
                        Style::default().fg(theme.text_dim),
                    )));
                    for line in raw.split('\n') {
                        form_lines.push(Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(theme.text),
                        )));
                    }
                }
                form_lines.push(Line::default());
            }

        let mut description_spans: Vec<Span> = Vec::new();
        description_spans.push(Span::styled(
            "Description: ",
            Style::default().fg(theme.keyword),
        ));
        let active = matches!(s.step.get(), CreateStep::Prompt);
        description_spans.push(Span::styled(
            s.prompt.clone(),
            Style::default().fg(theme.text_bright),
        ));
        if active {
            description_spans.push(Span::styled("▏", Style::default().fg(theme.info)));
        }
        form_lines.push(Line::from(description_spans));

        form_lines.push(Line::from(Span::styled(
            "─".repeat((body_area.width.saturating_sub(4)) as usize),
            Style::default().fg(crate::colors::border()),
        )));

        let mut buttons: Vec<Span> = Vec::new();
        let on_actions = matches!(s.step.get(), CreateStep::Action);
        let generate_selected = on_actions && s.action_idx == 0;
        let cancel_selected = on_actions && s.action_idx == 1;
        let style = |selected: bool| {
            if selected {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            }
        };
        buttons.push(Span::styled("[ Generate... ]", style(generate_selected)));
        buttons.push(Span::raw("  "));
        buttons.push(Span::styled("[ Cancel ]", style(cancel_selected)));
        form_lines.push(Line::from(buttons));

        Paragraph::new(form_lines)
            .alignment(Alignment::Left)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .render(body_area, buf);
    }

    pub(super) fn render_create_theme_mode(
        &self,
        body_area: Rect,
        theme: &crate::theme::Theme,
        buf: &mut Buffer,
    ) {
        let Mode::CreateTheme(s) = &self.mode else {
            return;
        };

        if let Some(rx) = &s.rx {
            for _ in 0..100 {
                match rx.try_recv() {
                    Ok(ProgressMsg::ThinkingDelta(delta)) | Ok(ProgressMsg::OutputDelta(delta)) => {
                        if let Mode::CreateTheme(state) = &self.mode {
                            let mut current = state.thinking_current.borrow_mut();
                            let mut history = state.thinking_lines.borrow_mut();
                            Self::append_delta_with_line_wrap(&mut current, &mut history, &delta);
                        }
                        self.app_event_tx.send(AppEvent::RequestRedraw);
                    }
                    Ok(ProgressMsg::SetStatus(status)) => {
                        if let Mode::CreateTheme(state) = &self.mode {
                            let mut current = state.thinking_current.borrow_mut();
                            current.clear();
                            current.push_str(&status);
                        }
                        self.app_event_tx.send(AppEvent::RequestRedraw);
                    }
                    Ok(ProgressMsg::CompletedThemeOk(name, colors, is_dark)) => {
                        if let Mode::CreateTheme(state) = &self.mode {
                            state.is_loading.set(false);
                            state.step.set(CreateStep::Review);
                            state.proposed_name.replace(Some(name.clone()));
                            state.proposed_colors.replace(Some(colors.clone()));
                            state.proposed_is_dark.set(is_dark);
                            crate::theme::set_custom_theme_label(name.clone());
                            crate::theme::set_custom_theme_is_dark(is_dark);
                            crate::theme::init_theme(&code_core::config_types::ThemeConfig {
                                name: ThemeName::Custom,
                                colors,
                                label: Some(name),
                                is_dark,
                            });
                        }
                        self.app_event_tx.send(AppEvent::RequestRedraw);
                    }
                    Ok(ProgressMsg::RawOutput(raw)) => {
                        if let Mode::CreateTheme(state) = &self.mode {
                            state.last_raw_output.replace(Some(raw));
                        }
                    }
                    Ok(ProgressMsg::CompletedErr { error, .. }) => {
                        if let Mode::CreateTheme(state) = &self.mode {
                            state.is_loading.set(false);
                            state.step.set(CreateStep::Action);
                            state
                                .thinking_lines
                                .borrow_mut()
                                .push(format!("Error: {error}"));
                            state.thinking_current.borrow_mut().clear();
                        }
                        self.app_event_tx.send(AppEvent::RequestRedraw);
                    }
                    Ok(ProgressMsg::CompletedOk { .. }) => {}
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
        }

        let mut form_lines = Vec::new();

        if s.is_loading.get() {
            form_lines.push(Line::from(Span::styled(
                "Overview » Change Theme » Create Custom",
                Style::default()
                    .fg(theme.text_bright)
                    .add_modifier(Modifier::BOLD),
            )));
            form_lines.push(Line::default());

            use std::time::SystemTime;
            use std::time::UNIX_EPOCH;
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let frames = ["◌", "◔", "◑", "◕", "●", "◕", "◑", "◔"];
            let frame = frames[((now_ms / 100) as usize) % frames.len()].to_string();
            form_lines.push(Line::from(vec![
                Span::styled(frame, Style::default().fg(crate::colors::info())),
                Span::styled(
                    " Generating theme with AI…",
                    Style::default().fg(theme.text_bright),
                ),
            ]));

            let current = s.thinking_current.borrow();
            let history = s.thinking_lines.borrow();
            let mut latest = Self::latest_progress_line(&current, &history);
            if !latest.ends_with('…') {
                latest.push_str(" …");
            }
            form_lines.push(Line::from(Span::styled(
                latest,
                Style::default().fg(theme.text_dim),
            )));

            self.app_event_tx
                .send(AppEvent::ScheduleFrameIn(std::time::Duration::from_millis(
                    100,
                )));
            Paragraph::new(form_lines)
                .alignment(Alignment::Left)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .render(body_area, buf);
            return;
        }

        if matches!(s.step.get(), CreateStep::Review) {
            form_lines.push(Line::from(Span::styled(
                "Overview » Change Theme » Create Custom",
                Style::default()
                    .fg(theme.text_bright)
                    .add_modifier(Modifier::BOLD),
            )));
            form_lines.push(Line::default());

            let name = s
                .proposed_name
                .borrow()
                .clone()
                .unwrap_or_else(|| "Custom".to_string());
            let onoff = if s.preview_on.get() { "on" } else { "off" };
            let toggle_style = if s.review_focus_is_toggle.get() {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            form_lines.push(Line::from(Span::styled(
                format!("Now showing {name} [{onoff}]"),
                toggle_style,
            )));
            form_lines.push(Line::default());

            let mut buttons: Vec<Span> = Vec::new();
            let save_selected = !s.review_focus_is_toggle.get() && s.action_idx == 0;
            let retry_selected = !s.review_focus_is_toggle.get() && s.action_idx == 1;
            let style = |selected: bool| {
                if selected {
                    Style::default()
                        .fg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                }
            };
            buttons.push(Span::styled("[ Save ]", style(save_selected)));
            buttons.push(Span::raw("  "));
            buttons.push(Span::styled("[ Retry ]", style(retry_selected)));
            form_lines.push(Line::from(buttons));

            Paragraph::new(form_lines)
                .alignment(Alignment::Left)
                .wrap(ratatui::widgets::Wrap { trim: false })
                .render(body_area, buf);
            return;
        }

        form_lines.push(Line::from(Span::styled(
            "Overview » Change Theme » Create Custom",
            Style::default()
                .fg(theme.text_bright)
                .add_modifier(Modifier::BOLD),
        )));
        form_lines.push(Line::default());

        if let Some(last) = s.thinking_lines.borrow().last().cloned()
            && last.starts_with("Error:") {
                form_lines.push(Line::from(Span::styled(
                    last,
                    Style::default().fg(crate::colors::error()),
                )));
                if let Some(raw) = s.last_raw_output.borrow().as_ref() {
                    form_lines.push(Line::from(Span::styled(
                        "Model output (raw):",
                        Style::default().fg(theme.text_dim),
                    )));
                    for line in raw.split('\n') {
                        form_lines.push(Line::from(Span::styled(
                            line.to_string(),
                            Style::default().fg(theme.text),
                        )));
                    }
                }
                form_lines.push(Line::default());
            }

        form_lines.push(Line::from(Span::styled(
            "Code can generate a custom theme just for you!",
            Style::default().fg(theme.text),
        )));
        form_lines.push(Line::from(Span::styled(
            "What should it look like? (e.g. Light Sunrise with Palm Trees, Dark River with Fireflies)",
            Style::default().fg(theme.text_dim),
        )));
        form_lines.push(Line::default());

        let mut description_spans: Vec<Span> = Vec::new();
        description_spans.push(Span::styled(
            "Description: ",
            Style::default().fg(theme.keyword),
        ));
        let active = matches!(s.step.get(), CreateStep::Prompt);
        description_spans.push(Span::styled(
            s.prompt.clone(),
            Style::default().fg(theme.text_bright),
        ));
        if active {
            description_spans.push(Span::styled("▏", Style::default().fg(theme.info)));
        }
        form_lines.push(Line::from(description_spans));

        form_lines.push(Line::from(Span::styled(
            "─".repeat((body_area.width.saturating_sub(4)) as usize),
            Style::default().fg(crate::colors::border()),
        )));

        let mut buttons: Vec<Span> = Vec::new();
        let on_actions = matches!(s.step.get(), CreateStep::Action);
        let generate_selected = on_actions && s.action_idx == 0;
        let cancel_selected = on_actions && s.action_idx == 1;
        let style = |selected: bool| {
            if selected {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            }
        };
        buttons.push(Span::styled("[ Generate... ]", style(generate_selected)));
        buttons.push(Span::raw("  "));
        buttons.push(Span::styled("[ Cancel ]", style(cancel_selected)));
        form_lines.push(Line::from(buttons));

        Paragraph::new(form_lines)
            .alignment(Alignment::Left)
            .wrap(ratatui::widgets::Wrap { trim: false })
            .render(body_area, buf);
    }
}
