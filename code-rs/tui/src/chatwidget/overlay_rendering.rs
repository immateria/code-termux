use super::*;

impl ChatWidget<'_> {
    fn render_browser_overlay(
        &self,
        frame_area: Rect,
        history_area: Rect,
        bottom_pane_area: Rect,
        buf: &mut Buffer,
    ) {
        use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect as RtRect};
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line as RLine, Span};
        use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
        use ratatui::widgets::Widget;

        let scrim_style = Style::default()
            .bg(crate::colors::overlay_scrim())
            .fg(crate::colors::text_dim());
        fill_rect(buf, frame_area, None, scrim_style);

        let padding = 1u16;
        let footer_reserved = bottom_pane_area.height.min(1);
        let overlay_bottom = (bottom_pane_area.y + bottom_pane_area.height).saturating_sub(footer_reserved);
        let overlay_height = overlay_bottom
            .saturating_sub(history_area.y)
            .max(1)
            .min(frame_area.height);

        let window_area = Rect {
            x: history_area.x + padding,
            y: history_area.y,
            width: history_area.width.saturating_sub(padding * 2),
            height: overlay_height,
        };
        Clear.render(window_area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(RLine::from(vec![
                Span::styled(
                    format!(" {} ", self.browser_title()),
                    Style::default().fg(crate::colors::text()),
                ),
                Span::styled(
                    "— Ctrl+B to close",
                    Style::default().fg(crate::colors::text_dim()),
                ),
            ]))
            .style(Style::default().bg(crate::colors::background()))
            .border_style(
                Style::default()
                    .fg(crate::colors::border())
                    .bg(crate::colors::background()),
            );
        let inner = block.inner(window_area);
        block.render(window_area, buf);

        let inner_bg = Style::default().bg(crate::colors::background());
        for y in inner.y..inner.y + inner.height {
            for x in inner.x..inner.x + inner.width {
                buf[(x, y)].set_style(inner_bg);
            }
        }

        let content = inner.inner(Margin::new(1, 0));
        if content.width == 0 || content.height == 0 {
            return;
        }

        let overlay_tracker = self.browser_overlay_tracker();
        let cell_opt = overlay_tracker.as_ref().map(|(_, tracker)| &tracker.cell);

        let (screenshot_history, mut selected_index) = if let Some(cell) = cell_opt {
            let history = cell.screenshot_history();
            if history.is_empty() {
                (None, 0usize)
            } else {
                let mut index = self.browser_overlay_state.screenshot_index();
                if index >= history.len() {
                    index = history.len().saturating_sub(1);
                    self.browser_overlay_state.set_screenshot_index(index);
                }
                (Some(history), index)
            }
        } else {
            (None, 0usize)
        };

        let screenshot_count = screenshot_history.map_or(0, <[_]>::len);
        if screenshot_count == 0 {
            selected_index = 0;
        }

        let mut screenshot_path = screenshot_history
            .and_then(|history| history.get(selected_index))
            .map(|record| record.path.clone());
        let mut screenshot_url = screenshot_history
            .and_then(|history| history.get(selected_index))
            .and_then(|record| record.url.clone());

        if screenshot_path.is_none()
            && let Ok(latest) = self.latest_browser_screenshot.lock()
                && let Some((path, url)) = latest.as_ref() {
                    screenshot_path = Some(path.clone());
                    if screenshot_url.is_none() {
                        screenshot_url = Some(url.clone());
                    }
                }

        let summary_label = cell_opt
            .map(crate::history_cell::BrowserSessionCell::summary_label)
            .unwrap_or_else(|| self.browser_title().to_string());
        let summary_value = screenshot_url
            
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| summary_label.clone());

        let screenshot_info = if screenshot_count > 0 {
            format!("Shot {}/{}", selected_index + 1, screenshot_count)
        } else {
            "No screenshots yet".to_string()
        };

        let is_active = screenshot_path.is_some();
        let key_hint_style = Style::default().fg(crate::colors::function());
        let label_style = Style::default().fg(crate::colors::text_dim());
        let dot_style = if is_active {
            Style::default().fg(crate::colors::success_green())
        } else {
            Style::default().fg(crate::colors::text_dim())
        };

        let header_height = if content.height >= 3 { 1 } else { 0 };
        if header_height > 0 {
            let header_area = Rect {
                x: content.x,
                y: content.y,
                width: content.width,
                height: 1,
            };

            let mut left_spans: Vec<Span> = Vec::new();
            left_spans.push(Span::styled("•", dot_style));
            if !summary_value.is_empty() {
                left_spans.push(Span::raw(" "));
                left_spans.push(Span::raw(summary_value));
            }
            left_spans.push(Span::raw("  "));
            left_spans.push(Span::styled(screenshot_info, label_style));

            let right_spans: Vec<Span> = vec![
                Span::from("Ctrl+B").style(key_hint_style),
                Span::styled(" close", label_style),
            ];

            let measure = |spans: &Vec<Span>| -> usize {
                spans.iter().map(|s| s.content.chars().count()).sum()
            };
            let left_len = measure(&left_spans);
            let right_len = measure(&right_spans);
            let total_width = header_area.width as usize;
            if total_width > left_len + right_len {
                let spacer = " ".repeat(total_width - left_len - right_len);
                left_spans.push(Span::from(spacer));
            }
            let mut spans = left_spans;
            spans.extend(right_spans);
            Paragraph::new(RLine::from(spans))
                .alignment(Alignment::Left)
                .render(header_area, buf);
        }

        let mut body_y = content.y + header_height;
        let mut body_height = content.height.saturating_sub(header_height);
        if header_height > 0 && body_height > 0 {
            body_y = body_y.saturating_add(1);
            body_height = body_height.saturating_sub(1);
        }

        if body_height == 0 {
            return;
        }

        let body_area = RtRect {
            x: content.x,
            y: body_y,
            width: content.width,
            height: body_height,
        };

        let column_constraints = if body_area.width <= 50 {
            [
                Constraint::Length(body_area.width.saturating_sub(24).max(20)),
                Constraint::Length(24),
            ]
        } else {
            [Constraint::Percentage(62), Constraint::Percentage(38)]
        };
        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(column_constraints)
            .split(body_area);

        let screenshot_column = columns[0];
        let info_column = if columns.len() > 1 { columns[1] } else { columns[0] };

        let progress_height = if screenshot_column.height > 3 { 1 } else { 0 };
        let screenshot_display_height = screenshot_column.height.saturating_sub(progress_height);
        let screenshot_display_area = Rect {
            x: screenshot_column.x,
            y: screenshot_column.y,
            width: screenshot_column.width,
            height: screenshot_display_height,
        };
        let progress_area = if progress_height > 0 {
            Some(Rect {
                x: screenshot_column.x,
                y: screenshot_column
                    .y
                    .saturating_add(screenshot_column.height.saturating_sub(progress_height)),
                width: screenshot_column.width,
                height: progress_height,
            })
        } else {
            None
        };

        if screenshot_display_area.width > 0 && screenshot_display_area.height > 0 {
            if let Some(path) = screenshot_path.as_ref() {
                self.render_screenshot_highlevel(path, screenshot_display_area, buf);
            } else {
                let message = Paragraph::new(RLine::from(vec![Span::raw(
                    "No browser session captured yet.",
                )]))
                .alignment(Alignment::Center)
                .style(Style::default().fg(crate::colors::text_dim()));
                Widget::render(message, screenshot_display_area, buf);
            }
        }

        let current_time = screenshot_history
            .and_then(|history| history.get(selected_index))
            .map(|record| record.timestamp)
            .unwrap_or_else(|| Duration::ZERO);
        let mut total_time = overlay_tracker
            .as_ref()
            .map(|(_, tracker)| tracker.elapsed)
            .unwrap_or_else(|| Duration::ZERO);
        if let Some(history) = screenshot_history
            && let Some(last) = history.last() {
                total_time = total_time.max(last.timestamp);
            }
        if let Some(cell) = cell_opt {
            total_time = total_time.max(cell.total_duration());
        }

        if let Some(area) = progress_area
            && area.height > 0 && area.width > 0 {
                let progress_line = self.browser_overlay_progress_line(area.width, current_time, total_time);
                Paragraph::new(progress_line)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(crate::colors::text()))
                    .render(area, buf);
            }

        if info_column.width == 0 || info_column.height == 0 {
            return;
        }

        let header_style = Style::default()
            .fg(crate::colors::text())
            .add_modifier(Modifier::BOLD);
        let secondary_style = Style::default().fg(crate::colors::text_dim());
        let primary_style = Style::default().fg(crate::colors::text());

        let mut info_lines: Vec<RLine<'static>> = Vec::new();

        info_lines.push(RLine::from(vec![Span::styled("Screenshots", header_style)]));

        if let Some(history) = screenshot_history {
            if history.is_empty() {
                info_lines.push(RLine::from(vec![Span::styled(
                    "No screenshots yet",
                    secondary_style,
                )]));
            } else {
                for (idx, entry) in history.iter().enumerate() {
                    let mut spans: Vec<Span> = Vec::new();
                    let marker = if idx == selected_index { "◉" } else { "•" };
                    let marker_style = if idx == selected_index {
                        Style::default().fg(crate::colors::primary())
                    } else {
                        secondary_style
                    };
                    spans.push(Span::styled(marker.to_string(), marker_style));
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        self.format_overlay_mm_ss(entry.timestamp),
                        secondary_style,
                    ));
                    if let Some(url) = entry.url.as_ref()
                        && !url.trim().is_empty() {
                            spans.push(Span::raw("  "));
                            spans.push(Span::styled(url.clone(), primary_style));
                        }
                    info_lines.push(RLine::from(spans));
                }
            }
        } else {
            info_lines.push(RLine::from(vec![Span::styled(
                "No browser session yet",
                secondary_style,
            )]));
        }

        info_lines.push(RLine::from(vec![Span::raw(String::new())]));
        info_lines.push(RLine::from(vec![Span::styled("Actions", header_style)]));

        if let Some(cell) = cell_opt {
            let entries = cell.full_action_entries();
            if entries.is_empty() {
                info_lines.push(RLine::from(vec![Span::styled(
                    "No browser actions yet",
                    secondary_style,
                )]));
            } else {
                for (time, label, detail) in entries {
                    let mut spans: Vec<Span> = Vec::new();
                    spans.push(Span::styled("•", secondary_style));
                    spans.push(Span::raw(" "));
                    spans.push(Span::styled(
                        self.normalize_action_time_label(time.as_str()),
                        secondary_style,
                    ));
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(label.clone(), primary_style));
                    let detail_trimmed = detail.trim();
                    if !detail_trimmed.is_empty() {
                        spans.push(Span::raw(" "));
                        spans.push(Span::styled(detail_trimmed.to_string(), secondary_style));
                    }
                    info_lines.push(RLine::from(spans));
                }
            }
        } else {
            info_lines.push(RLine::from(vec![Span::styled(
                "No browser session yet",
                secondary_style,
            )]));
        }

        info_lines.push(RLine::from(vec![Span::raw(String::new())]));
        info_lines.push(RLine::from(vec![Span::styled(
            "Controls: ←/→ or ↑/↓ select screenshot • Shift+↑/↓ or j/k scroll actions",
            secondary_style,
        )]));

        let max_scroll = info_lines.len().saturating_sub(info_column.height as usize);
        let max_scroll_u16 = max_scroll.min(u16::MAX as usize) as u16;
        self.browser_overlay_state
            .update_action_metrics(info_column.height, max_scroll_u16);
        let scroll = self
            .browser_overlay_state
            .action_scroll()
            .min(max_scroll_u16);

        let paragraph = Paragraph::new(info_lines)
            .style(Style::default().fg(crate::colors::text()))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));
        Widget::render(paragraph, info_column, buf);
    }

    fn render_settings_overlay(
        &self,
        frame_area: Rect,
        history_area: Rect,
        buf: &mut Buffer,
        overlay: &SettingsOverlayView,
    ) {
        use ratatui::widgets::Clear;

        let scrim_style = Style::default()
            .bg(crate::colors::overlay_scrim())
            .fg(crate::colors::text_dim());
        fill_rect(buf, frame_area, None, scrim_style);

        let padding = 1u16;
        let overlay_area = Rect {
            x: history_area.x + padding,
            y: history_area.y,
            width: history_area.width.saturating_sub(padding * 2),
            height: history_area.height,
        };

        Clear.render(overlay_area, buf);

        let bg_style = Style::default().bg(crate::colors::overlay_scrim());
        fill_rect(buf, overlay_area, None, bg_style);

        overlay.render(overlay_area, buf);
    }

    fn browser_title(&self) -> &'static str {
        if self.browser_is_external {
            "Chrome"
        } else {
            "Browser"
        }
    }

    fn render_agents_terminal_overlay(
        &self,
        frame_area: Rect,
        history_area: Rect,
        bottom_pane_area: Rect,
        buf: &mut Buffer,
    ) {
        use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect as RtRect};
        use ratatui::style::{Modifier, Style};
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{
            Block, Borders, Clear, HighlightSpacing, List, ListItem, ListState, Paragraph,
        };

        let scrim_style = Style::default()
            .bg(crate::colors::overlay_scrim())
            .fg(crate::colors::text_dim());
        fill_rect(buf, frame_area, None, scrim_style);

        let padding = 1u16;
        let footer_reserved = bottom_pane_area.height.min(1);
        let overlay_bottom = (bottom_pane_area.y + bottom_pane_area.height).saturating_sub(footer_reserved);
        let overlay_height = overlay_bottom
            .saturating_sub(history_area.y)
            .max(1)
            .min(frame_area.height);

        let window_area = Rect {
            x: history_area.x + padding,
            y: history_area.y,
            width: history_area.width.saturating_sub(padding * 2),
            height: overlay_height,
        };
        Clear.render(window_area, buf);

        let title_spans = vec![
            Span::styled(" Agents ", Style::default().fg(crate::colors::text())),
            Span::styled("— Ctrl+A to close", Style::default().fg(crate::colors::text_dim())),
        ];

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(title_spans))
            .style(Style::default().bg(crate::colors::background()))
            .border_style(
                Style::default()
                    .fg(crate::colors::border())
                    .bg(crate::colors::background()),
            );
        let inner = block.inner(window_area);
        block.render(window_area, buf);

        let inner_bg = Style::default().bg(crate::colors::background());
        for y in inner.y..inner.y + inner.height {
            for x in inner.x..inner.x + inner.width {
                buf[(x, y)].set_style(inner_bg);
            }
        }

        // Remove vertical padding so the filter row sits directly below the title.
        let content = inner.inner(Margin {
            horizontal: 1,
            vertical: 0,
        });
        if content.width == 0 || content.height == 0 {
            return;
        }

        let tab_height = if content.height >= 3 { 1 } else { 0 };
        let hint_height = if content.height >= 2 { 1 } else { 0 };
        let body_height = content
            .height
            .saturating_sub(hint_height + tab_height);
        let tabs_area = RtRect {
            x: content.x,
            y: content.y,
            width: content.width,
            height: tab_height,
        };
        let body_area = RtRect {
            x: content.x,
            y: content.y.saturating_add(tab_height),
            width: content.width,
            height: body_height,
        };
        let hint_area = RtRect {
            x: content.x,
            y: content.y.saturating_add(tab_height + body_height),
            width: content.width,
            height: hint_height,
        };

        let sidebar_has_focus = self.agents_terminal.focus() == AgentsTerminalFocus::Sidebar;
        let sidebar_border_color = if sidebar_has_focus {
            crate::colors::primary()
        } else {
            crate::colors::border()
        };
        let filter_title_style = Style::default().fg(crate::colors::text_dim());

        if tab_height > 0 {
            let filter_row = tabs_area;
            let mut spans: Vec<Span> = Vec::new();
            spans.push(Span::styled("Filter", filter_title_style));
            spans.push(Span::raw("   "));
            let tabs = [
                (AgentsTerminalTab::All, "1", "All"),
                (AgentsTerminalTab::Running, "2", "Running"),
                (AgentsTerminalTab::Failed, "3", "Failed"),
                (AgentsTerminalTab::Completed, "4", "Done"),
                (AgentsTerminalTab::Review, "5", "Review"),
            ];
            for (idx, (tab, number, label)) in tabs.iter().enumerate() {
                if idx > 0 {
                    spans.push(Span::styled(
                        " - ",
                        Style::default().fg(crate::colors::text_dim()),
                    ));
                }
                let active = *tab == self.agents_terminal.active_tab;
                let style = if active {
                    Style::default()
                        .fg(crate::colors::primary())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(crate::colors::text_dim())
                };
                spans.push(Span::styled(format!("{number} {label}"), style));
            }
            let sort_label = match self.agents_terminal.sort_mode {
                AgentsSortMode::Recent => "Recent",
                AgentsSortMode::RunningFirst => "Running",
                AgentsSortMode::Name => "Name",
            };
            let sort_spans = vec![
                Span::styled("Sort: ", Style::default().fg(crate::colors::text_dim())),
                Span::raw("( "),
                Span::styled(
                    format!("{sort_label} ▼"),
                    Style::default()
                        .fg(crate::colors::primary())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" )"),
            ];

            let filter_width: u16 = spans.iter().map(|span| span.width() as u16).sum();
            let sort_width: u16 = sort_spans.iter().map(|span| span.width() as u16).sum();
            let gap = filter_row
                .width
                .saturating_sub(filter_width + sort_width)
                .max(1);
            spans.push(Span::raw(" ".repeat(gap as usize)));
            spans.extend(sort_spans);

            Paragraph::new(Line::from(spans))
                .alignment(Alignment::Left)
                .render(filter_row, buf);
        }

        let longest_name_width: u16 = self
            .agents_terminal
            .entries
            .values()
            .map(|entry| {
                let label = entry
                    .model
                    .as_ref()
                    .map(|m| Self::format_model_label(m))
                    .unwrap_or_else(|| Self::format_model_label(&entry.name));
                UnicodeWidthStr::width(label.as_str()) as u16
            })
            .max()
            .unwrap_or(10);
        let status_icon_width = UnicodeWidthStr::width(agent_status_icon(AgentStatus::Running)) as u16;
        let desired_sidebar = longest_name_width
            .saturating_add(status_icon_width)
            .saturating_add(8);
        let sidebar_width = if body_area.width <= 30 {
            body_area.width
        } else {
            let max_allowed = body_area.width.saturating_sub(30).max(18);
            let min_allowed = 24.min(max_allowed);
            desired_sidebar.clamp(min_allowed, max_allowed)
        };

        let constraints = if body_area.width <= sidebar_width {
            [Constraint::Length(body_area.width), Constraint::Length(0)]
        } else {
            [Constraint::Length(sidebar_width), Constraint::Min(12)]
        };

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(body_area);

        // Sidebar list of agents grouped by batch id
        let mut items: Vec<ListItem> = Vec::new();
        let mut row_entries: Vec<Option<AgentsSidebarEntry>> = Vec::new();
        let groups = self.agents_terminal.sidebar_groups();
        let last_group_idx = groups.len().saturating_sub(1);

        for (group_idx, group) in groups.into_iter().enumerate() {
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    group.label.clone(),
                    Style::default()
                        .fg(crate::colors::text())
                        .add_modifier(Modifier::BOLD),
                ),
            ])));
            row_entries.push(None);

            let selected_entry = self.agents_terminal.current_sidebar_entry();

            for agent_id in group.agent_ids {
                if let Some(entry) = self.agents_terminal.entries.get(&agent_id) {
                    let model_label = entry
                        .model
                        .as_ref()
                        .map(|value| Self::format_model_label(value))
                        .unwrap_or_else(|| Self::format_model_label(&entry.name));
                    let status = entry.status.clone();
                    let status_icon = agent_status_icon(status.clone());
                    let name_room = sidebar_width
                        .saturating_sub((UnicodeWidthStr::width(status_icon) as u16).saturating_add(5))
                        .max(4) as usize;
                    let mut display_name = model_label.clone();
                    if display_name.chars().count() > name_room {
                        display_name = display_name
                            .chars()
                            .take(name_room.saturating_sub(1))
                            .collect::<String>();
                        display_name.push('…');
                    }
                    let color = agent_status_color(status);
                    let is_selected = selected_entry
                        .as_ref()
                        .map(|entry| entry == &AgentsSidebarEntry::Agent(agent_id.clone()))
                        .unwrap_or(false);
                    let prefix_span = if is_selected {
                        Span::styled(
                            "› ",
                            Style::default().fg(crate::colors::primary()),
                        )
                    } else {
                        Span::raw("  ")
                    };

                    let line = Line::from(vec![
                        prefix_span,
                        Span::styled(
                            display_name,
                            Style::default().fg(crate::colors::text()),
                        ),
                        Span::raw(" "),
                        Span::styled(status_icon, Style::default().fg(color)),
                    ]);
                    items.push(ListItem::new(line));
                    row_entries.push(Some(AgentsSidebarEntry::Agent(agent_id.clone())));
                }
            }

            if group_idx < last_group_idx {
                items.push(ListItem::new(Line::from(vec![Span::raw(" ")])));
                row_entries.push(None);
            }
        }

        if items.is_empty() {
            let empty_text = if self.agents_terminal.order.is_empty() {
                "No agents yet"
            } else {
                "No agents match filters"
            };
            items.push(ListItem::new(Line::from(vec![Span::styled(
                empty_text,
                Style::default().fg(crate::colors::text_dim()),
            )])));
            row_entries.push(None);
        }

        let mut list_state = ListState::default();
        if let Some(selected_entry) = self.agents_terminal.current_sidebar_entry()
            && let Some(row_idx) = row_entries
                .iter()
                .position(|entry| entry.as_ref() == Some(&selected_entry))
            {
                list_state.select(Some(row_idx));
            }

        // Keep the selected agent vivid even when detail pane holds focus so users
        // don’t lose their place while reading logs.
        let highlight_style = Style::default()
            .fg(crate::colors::primary())
            .add_modifier(Modifier::BOLD);
        let sidebar = List::new(items)
            .highlight_style(highlight_style)
            .highlight_spacing(HighlightSpacing::Never);

        let sidebar_block = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(crate::colors::background()))
            .border_style(Style::default().fg(sidebar_border_color));

        let sidebar_area = chunks[0];
        let sidebar_inner = sidebar_block.inner(sidebar_area);
        sidebar_block.render(sidebar_area, buf);

        fill_rect(
            buf,
            sidebar_inner,
            None,
            Style::default().bg(crate::colors::background()),
        );

        ratatui::widgets::StatefulWidget::render(sidebar, sidebar_inner, buf, &mut list_state);

        let right_area = if chunks.len() > 1 { chunks[1] } else { chunks[0] };
        let detail_width = right_area.width.saturating_sub(2).max(1);
        let mut lines: Vec<Line> = Vec::new();

        match self.agents_terminal.current_sidebar_entry() {
            Some(AgentsSidebarEntry::Agent(agent_id)) => {
                if let Some(entry) = self.agents_terminal.entries.get(agent_id.as_str()) {
                    let status = entry.status.clone();
                    let status_color = agent_status_color(status.clone());
                    let display_name = entry
                        .model
                        .as_ref()
                        .map(|m| Self::format_model_label(m))
                        .unwrap_or_else(|| Self::format_model_label(&entry.name));
                    let title_text = entry
                        .batch_label
                        .as_ref()
                        .and_then(|b| {
                            let trimmed = b.trim();
                            (!trimmed.is_empty()).then(|| trimmed.to_string())
                        })
                        .map(|batch| format!("{batch} / {display_name}"))
                        .unwrap_or_else(|| display_name.clone());

                    lines.push(Line::from(vec![
                        Span::raw(" "),
                        Span::styled(
                            title_text,
                            Style::default()
                                .fg(crate::colors::text())
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]));

                    let id_short = format!("#{}", agent_id.chars().take(7).collect::<String>());
                    let status_chip = format!("{} {}", agent_status_icon(status.clone()), agent_status_label(status));
                    let model_meta = entry
                        .model
                        .as_ref()
                        .map(|m| Self::format_model_label(m))
                        .unwrap_or_else(|| display_name.clone());
                    let mut meta_line: Vec<Span> = vec![
                        Span::raw(" "),
                        Span::styled("Status:", Style::default().fg(crate::colors::text_dim())),
                        Span::raw(" "),
                        Span::styled(status_chip, Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
                        Span::raw("   "),
                        Span::styled("Model:", Style::default().fg(crate::colors::text_dim())),
                        Span::raw(" "),
                        Span::styled(
                            model_meta,
                            Style::default().fg(crate::colors::text()),
                        ),
                        Span::raw("   "),
                        Span::styled("ID:", Style::default().fg(crate::colors::text_dim())),
                        Span::raw(" "),
                        Span::styled(id_short, Style::default().fg(crate::colors::text_dim())),
                    ];
                    if let Some(batch_id) = entry.batch_id.as_ref() {
                        meta_line.push(Span::raw("   "));
                        meta_line.push(Span::styled(
                            format!("Batch: {}", short_batch_label(batch_id)),
                            Style::default().fg(crate::colors::text_dim()),
                        ));
                    }
                    lines.push(Line::from(meta_line));

                    self.ensure_trailing_blank_line(&mut lines);

                    self.append_agent_highlights(
                        &mut lines,
                        entry,
                        detail_width,
                        self.agents_terminal.highlights_collapsed,
                    );

                    if let Some(context_text) = entry
                        .batch_context
                        .as_ref()
                        .filter(|value| !value.trim().is_empty())
                    {
                        self.ensure_trailing_blank_line(&mut lines);
                        self.append_agents_overlay_section(&mut lines, "Context", context_text);
                    }

                    self.ensure_trailing_blank_line(&mut lines);

                    // Action log box
                    let action_header_style = Style::default()
                        .fg(crate::colors::text())
                        .add_modifier(Modifier::BOLD);
                    let chevron = if self.agents_terminal.actions_collapsed { "▶" } else { "▼" };
                    let header_text = format!(
                        "╭ Action Log (a) {chevron} — {} entries ",
                        entry.logs.len()
                    );
                    let header_width = UnicodeWidthStr::width(header_text.as_str()) as u16;
                    let pad = detail_width
                        .saturating_sub(header_width)
                        .saturating_sub(1);
                    let mut action_header = header_text;
                    action_header.push_str(&"─".repeat(pad as usize));
                    action_header.push('╮');
                    lines.push(Line::from(Span::styled(action_header, action_header_style)));

                    if self.agents_terminal.actions_collapsed {
                        let mut footer = String::from("╰");
                        footer.push_str(&"─".repeat(detail_width.saturating_sub(1) as usize));
                        lines.push(Line::from(footer));
                        self.ensure_trailing_blank_line(&mut lines);
                    } else if entry.logs.is_empty() {
                        lines.push(Line::from(vec![
                            Span::raw("│   "),
                            Span::styled(
                                "No updates yet",
                                Style::default().fg(crate::colors::text_dim()),
                            ),
                        ]));
                        let mut footer = String::from("╰");
                        footer.push_str(&"─".repeat(detail_width.saturating_sub(1) as usize));
                        lines.push(Line::from(footer));
                        self.ensure_trailing_blank_line(&mut lines);
                    } else {
                        let mut log_lines: Vec<Line> = Vec::new();
                        let mut last_kind: Option<AgentLogKind> = None;
                        for (idx, log) in entry.logs.iter().enumerate() {
                            let is_new_kind = last_kind != Some(log.kind);
                            self.append_agent_log_lines(
                                &mut log_lines,
                                idx,
                                log,
                                detail_width.saturating_sub(4),
                                is_new_kind,
                            );
                            last_kind = Some(log.kind);
                        }
                        for mut line in log_lines.into_iter() {
                            line.spans.insert(0, Span::raw("│   "));
                            lines.push(line);
                        }
                        let mut footer = String::from("╰");
                        footer.push_str(&"─".repeat(detail_width.saturating_sub(1) as usize));
                        lines.push(Line::from(footer));
                        self.ensure_trailing_blank_line(&mut lines);
                    }
                } else {
                    lines.push(Line::from(vec![
                        Span::raw(" "),
                        Span::styled(
                            "No data for selected agent",
                            Style::default().fg(crate::colors::text_dim()),
                        ),
                    ]));
                }
            }
            None => {
                lines.push(Line::from(vec![
                    Span::raw(" "),
                    Span::styled(
                        "No agents available",
                        Style::default().fg(crate::colors::text_dim()),
                    ),
                ]));
            }
        }

        let content_width = right_area.width.saturating_sub(2).max(1);
        let wrapped_lines = word_wrap_lines(&lines, content_width as u16);
        let viewport_height = right_area.height.saturating_sub(2).max(1);
        let total_lines = wrapped_lines.len() as u16;
        let max_scroll = total_lines.saturating_sub(viewport_height);
        self.layout.last_history_viewport_height.set(viewport_height);
        self.layout.last_max_scroll.set(max_scroll);

        // scroll_offset is bottom‑anchored; Paragraph expects top‑anchored scroll.
        let preferred_offset = self
            .agents_terminal
            .current_sidebar_entry()
            .and_then(|entry| {
                self.agents_terminal
                    .scroll_offsets
                    .get(&entry.scroll_key())
                    .copied()
            })
            .unwrap_or(max_scroll);
        let clamped_offset = preferred_offset.min(max_scroll);
        self
            .agents_terminal
            .last_render_scroll
            .set(clamped_offset);
        let scroll_from_top = max_scroll.saturating_sub(clamped_offset);

        let detail_has_focus = self.agents_terminal.focus() == AgentsTerminalFocus::Detail;
        let detail_border_color = if detail_has_focus {
            crate::colors::primary()
        } else {
            crate::colors::border()
        };
        let history_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(detail_border_color));

        Paragraph::new(wrapped_lines)
            .block(history_block)
            .scroll((scroll_from_top, 0))
            .render(right_area, buf);

        if hint_height == 1 {
            let hint_line = if let Some(pending) = self.agents_terminal.pending_stop.as_ref() {
                Line::from(vec![
                    Span::styled(
                        "Stop agent? ",
                        Style::default()
                            .fg(crate::colors::error())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        pending.agent_name.clone(),
                        Style::default().fg(crate::colors::text()),
                    ),
                    Span::styled(
                        " — Enter/Y stop  ",
                        Style::default().fg(crate::colors::text_dim()),
                    ),
                    Span::styled(
                        "Esc/N cancel",
                        Style::default().fg(crate::colors::text_dim()),
                    ),
                ])
            } else {
                Line::from(vec![
                    Span::styled("[↑/↓/←/→]", Style::default().fg(crate::colors::function())),
                    Span::styled(" Navigate   ", Style::default().fg(crate::colors::text_dim())),
                    Span::styled("[1-5]", Style::default().fg(crate::colors::function())),
                    Span::styled(" Filter   ", Style::default().fg(crate::colors::text_dim())),
                    Span::styled("[S]", Style::default().fg(crate::colors::function())),
                    Span::styled(" Sort   ", Style::default().fg(crate::colors::text_dim())),
                    Span::styled("[H/A]", Style::default().fg(crate::colors::function())),
                    Span::styled(" Toggle Details   ", Style::default().fg(crate::colors::text_dim())),
                    Span::styled("[X]", Style::default().fg(crate::colors::function())),
                    Span::styled(" Stop   ", Style::default().fg(crate::colors::text_dim())),
                    Span::styled("[Ctrl+A]", Style::default().fg(crate::colors::function())),
                    Span::styled(" Exit", Style::default().fg(crate::colors::text_dim())),
                ])
            };
            Paragraph::new(hint_line)
                .style(Style::default().bg(crate::colors::background()))
                .alignment(ratatui::layout::Alignment::Center)
                .render(hint_area, buf);
        }
    }

    #[allow(dead_code)]
    /// Render the agent status panel in the HUD
    fn render_agent_panel(&self, area: Rect, buf: &mut Buffer) {
        use ratatui::text::Line as RLine;
        use ratatui::text::Span;
        use ratatui::text::Text;
        use ratatui::widgets::Block;
        use ratatui::widgets::Borders;
        use ratatui::widgets::Paragraph;
        use ratatui::widgets::Sparkline;
        use ratatui::widgets::SparklineBar;
        use ratatui::widgets::Widget;
        use ratatui::widgets::Wrap;

        // Update sparkline data for animation
        if !self.active_agents.is_empty() || self.agents_ready_to_start {
            self.update_sparkline_data();
        }

        let short_id = |id: &str| -> String { id.chars().take(8).collect() };
        let mut rendered_batches = std::collections::HashSet::new();

        // Agent status block
        let agent_block = Block::default()
            .borders(Borders::ALL)
            .title(" Agents ")
            .border_style(Style::default().fg(crate::colors::border()));

        let inner_agent = agent_block.inner(area);
        agent_block.render(area, buf);
        // Render a one-line collapsed header inside expanded panel
        use ratatui::layout::Margin;
        let header_pad = inner_agent.inner(Margin::new(1, 0));
        let header_line = Rect {
            x: header_pad.x,
            y: header_pad.y,
            width: header_pad.width,
            height: 1,
        };
        let key_hint_style = Style::default().fg(crate::colors::function());
        let label_style = Style::default().dim();
        let is_active = !self.active_agents.is_empty() || self.agents_ready_to_start;
        let dot_style = if is_active {
            Style::default().fg(crate::colors::success_green())
        } else {
            Style::default().fg(crate::colors::text_dim())
        };
        // Build summary like collapsed header
        let count = self.active_agents.len();
        let summary = if count == 0 && self.agents_ready_to_start {
            "Starting...".to_string()
        } else if count == 0 {
            "no active agents".to_string()
        } else {
            let mut parts: Vec<String> = Vec::new();
            for a in self.active_agents.iter().take(3) {
                let s = match a.status {
                    AgentStatus::Pending => "pending",
                    AgentStatus::Running => "running",
                    AgentStatus::Completed => "done",
                    AgentStatus::Failed => "failed",
                    AgentStatus::Cancelled => "cancelled",
                };
                parts.push(format!("{} ({})", a.name, s));
            }
            let extra = if count > 3 {
                format!(" +{}", count - 3)
            } else {
                String::new()
            };
            format!("{}{}", parts.join(", "), extra)
        };
        let mut left_spans: Vec<Span> = Vec::new();
        left_spans.push(Span::styled("•", dot_style));
        // no status text; dot conveys status
        // single space between dot and summary; no label/separator
        left_spans.push(Span::raw(" "));
        left_spans.push(Span::raw(summary));
        let right_spans: Vec<Span> = vec![
            Span::from("Ctrl+A").style(key_hint_style),
            Span::styled(" open terminal", label_style),
        ];
        let measure =
            |spans: &Vec<Span>| -> usize { spans.iter().map(|s| s.content.chars().count()).sum() };
        let left_len = measure(&left_spans);
        let right_len = measure(&right_spans);
        let total_width = header_line.width as usize;
        if total_width > left_len + right_len {
            left_spans.push(Span::from(" ".repeat(total_width - left_len - right_len)));
        }
        let mut spans = left_spans;
        spans.extend(right_spans);
        Paragraph::new(RLine::from(spans)).render(header_line, buf);

        // Body area excludes the header line and a spacer line
        let inner_agent = Rect {
            x: inner_agent.x,
            y: inner_agent.y + 2,
            width: inner_agent.width,
            height: inner_agent.height.saturating_sub(2),
        };

        // Dynamically calculate sparkline height based on agent activity
        // More agents = taller sparkline area
        let agent_count = self.active_agents.len();
        let sparkline_height = if agent_count == 0 && self.agents_ready_to_start {
            1u16 // Minimal height when preparing
        } else if agent_count == 0 {
            0u16 // No sparkline when no agents
        } else {
            (agent_count as u16 + 1).min(4) // 2-4 lines based on agent count
        };

        // Ensure we have enough space for both content and sparkline
        // Reserve at least 3 lines for content (status + blank + message)
        let min_content_height = 3u16;
        let available_height = inner_agent.height;

        let (actual_content_height, actual_sparkline_height) = if sparkline_height > 0 {
            if available_height > min_content_height + sparkline_height {
                // Enough space for both
                (
                    available_height.saturating_sub(sparkline_height),
                    sparkline_height,
                )
            } else if available_height > min_content_height {
                // Limited space - give minimum to content, rest to sparkline
                (
                    min_content_height,
                    available_height
                        .saturating_sub(min_content_height)
                        .min(sparkline_height),
                )
            } else {
                // Very limited space - content only
                (available_height, 0)
            }
        } else {
            // No sparkline needed
            (available_height, 0)
        };

        let content_area = Rect {
            x: inner_agent.x,
            y: inner_agent.y,
            width: inner_agent.width,
            height: actual_content_height,
        };
        let sparkline_area = Rect {
            x: inner_agent.x,
            y: inner_agent.y + actual_content_height,
            width: inner_agent.width,
            height: actual_sparkline_height,
        };

        // Build all content into a single Text structure for proper wrapping
        let mut text_content = vec![];

        // Add blank line at the top
        text_content.push(RLine::from(" "));

        // Add overall task status at the top
        let status_color = match self.overall_task_status.as_str() {
            "planning" => crate::colors::warning(),
            "running" => crate::colors::info(),
            "consolidating" => crate::colors::warning(),
            "complete" => crate::colors::success(),
            "failed" => crate::colors::error(),
            "cancelled" => crate::colors::warning(),
            _ => crate::colors::text_dim(),
        };

        text_content.push(RLine::from(vec![
            Span::from(" "),
            Span::styled(
                "Status: ",
                Style::default()
                    .fg(crate::colors::text())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(&self.overall_task_status, Style::default().fg(status_color)),
        ]));

        // Add blank line
        text_content.push(RLine::from(" "));

        // Display agent statuses
        if self.agents_ready_to_start && self.active_agents.is_empty() {
            // Show "Building context..." message when agents are expected
            text_content.push(RLine::from(vec![
                Span::from(" "),
                Span::styled(
                    "Building context...",
                    Style::default()
                        .fg(crate::colors::text_dim())
                        .add_modifier(Modifier::ITALIC),
                ),
            ]));
        } else if self.active_agents.is_empty() {
            text_content.push(RLine::from(vec![
                Span::from(" "),
                Span::styled(
                    "No active agents",
                    Style::default().fg(crate::colors::text_dim()),
                ),
            ]));
        } else {
            // Show agent names/models and final messages
            for agent in &self.active_agents {
                let status_color = match agent.status {
                    AgentStatus::Pending => crate::colors::warning(),
                    AgentStatus::Running => crate::colors::info(),
                    AgentStatus::Completed => crate::colors::success(),
                    AgentStatus::Failed => crate::colors::error(),
                    AgentStatus::Cancelled => crate::colors::warning(),
                };

                // Build status + timing suffix where available
                let status_text = match agent.status {
                    AgentStatus::Pending => "pending".to_string(),
                    AgentStatus::Running => {
                        if let Some(rt) = self.agent_runtime.get(&agent.id) {
                            if let Some(start) = rt.started_at {
                                let now = Instant::now();
                                let elapsed = now.saturating_duration_since(start);
                                format!("running {}", self.fmt_short_duration(elapsed))
                            } else {
                                "running".to_string()
                            }
                        } else {
                            "running".to_string()
                        }
                    }
                    AgentStatus::Completed | AgentStatus::Failed | AgentStatus::Cancelled => {
                        if let Some(rt) = self.agent_runtime.get(&agent.id) {
                            if let (Some(start), Some(done)) = (rt.started_at, rt.completed_at) {
                                let dur = done.saturating_duration_since(start);
                                let base = match agent.status {
                                    AgentStatus::Completed => "completed",
                                    AgentStatus::Failed => "failed",
                                    AgentStatus::Cancelled => "cancelled",
                                    _ => unreachable!(),
                                };
                                format!("{} {}", base, self.fmt_short_duration(dur))
                            } else {
                                match agent.status {
                                    AgentStatus::Completed => "completed".to_string(),
                                    AgentStatus::Failed => "failed".to_string(),
                                    AgentStatus::Cancelled => "cancelled".to_string(),
                                    _ => unreachable!(),
                                }
                            }
                        } else {
                            match agent.status {
                                AgentStatus::Completed => "completed".to_string(),
                                AgentStatus::Failed => "failed".to_string(),
                                AgentStatus::Cancelled => "cancelled".to_string(),
                                _ => unreachable!(),
                            }
                        }
                    }
                };

                let mut line_spans: Vec<Span> = Vec::new();
                line_spans.push(Span::from(" "));
                line_spans.push(
                    Span::styled(
                        agent.name.to_string(),
                        Style::default()
                            .fg(crate::colors::text())
                            .add_modifier(Modifier::BOLD),
                    ),
                );
                line_spans.push(Span::styled(
                    format!(" [{}]", short_id(&agent.id)),
                    Style::default().fg(crate::colors::text_dim()),
                ));
                if let Some(ref model) = agent.model
                    && !model.is_empty() {
                        line_spans.push(Span::styled(
                            format!(" ({model})"),
                            Style::default().fg(crate::colors::text_dim()),
                        ));
                    }
                line_spans.push(Span::from(": "));
                line_spans.push(Span::styled(status_text, Style::default().fg(status_color)));
                text_content.push(RLine::from(line_spans));

                // For running agents, show latest progress hint if available
                if matches!(agent.status, AgentStatus::Running)
                    && let Some(ref lp) = agent.last_progress {
                        let mut lp_trim = lp.trim().to_string();
                        if lp_trim.len() > 120 {
                            lp_trim.truncate(120);
                            lp_trim.push('…');
                        }
                        text_content.push(RLine::from(vec![
                            Span::from("   "),
                            Span::styled(
                                lp_trim,
                                Style::default().fg(crate::colors::text_dim()),
                            ),
                        ]));
                    }

                // For completed/failed agents, show their final message or error
                match agent.status {
                    AgentStatus::Completed => {
                        if let Some(ref msg) = agent.result {
                            text_content.push(RLine::from(vec![
                                Span::from("   "),
                                Span::styled(msg, Style::default().fg(crate::colors::text_dim())),
                            ]));
                        }
                    }
                    AgentStatus::Failed => {
                        if let Some(ref err) = agent.error {
                            text_content.push(RLine::from(vec![
                                Span::from("   "),
                                Span::styled(
                                    err,
                                    Style::default()
                                        .fg(crate::colors::error())
                                        .add_modifier(Modifier::ITALIC),
                                ),
                            ]));
                        }
                    }
                    AgentStatus::Cancelled => {
                        if let Some(ref err) = agent.error {
                            text_content.push(RLine::from(vec![
                                Span::from("   "),
                                Span::styled(
                                    err,
                                    Style::default()
                                        .fg(crate::colors::warning())
                                        .add_modifier(Modifier::ITALIC),
                                ),
                            ]));
                        }
                    }
                    _ => {}
                }

                if let Some(ref batch) = agent.batch_id
                    && rendered_batches.insert(batch.clone()) {
                        let batch_line = format!(
                            "Batch {} — use agent {{\"action\":\"wait\",\"wait\":{{\"batch_id\":\"{}\"}}}}",
                            short_id(batch),
                            batch
                        );
                        text_content.push(RLine::from(vec![
                            Span::from("   "),
                            Span::styled(
                                batch_line,
                                Style::default().fg(crate::colors::text_dim()),
                            ),
                        ]));
                    }
            }
        }

        // Calculate how much vertical space the fixed content takes
        let fixed_content_height = text_content.len() as u16;

        // Create the first paragraph for the fixed content (status and agents) without wrapping
        let fixed_paragraph = Paragraph::new(Text::from(text_content));

        // Render the fixed content first
        let fixed_area = Rect {
            x: content_area.x,
            y: content_area.y,
            width: content_area.width,
            height: fixed_content_height.min(content_area.height),
        };
        fixed_paragraph.render(fixed_area, buf);

        // Calculate remaining area for wrapped content
        let remaining_height = content_area.height.saturating_sub(fixed_content_height);
        if remaining_height > 0 {
            let wrapped_area = Rect {
                x: content_area.x,
                y: content_area.y + fixed_content_height,
                width: content_area.width,
                height: remaining_height,
            };

            // Add context and task sections with proper wrapping in the remaining area
            let mut wrapped_content = vec![];

            if let Some(ref task) = self.agent_task {
                wrapped_content.push(RLine::from(" ")); // Empty line separator
                wrapped_content.push(RLine::from(vec![
                    Span::from(" "),
                    Span::styled(
                        "Task:",
                        Style::default()
                            .fg(crate::colors::text())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::from(" "),
                    Span::styled(task, Style::default().fg(crate::colors::text_dim())),
                ]));
            }

            if let Some(ref hint) = self.recent_agent_hint {
                wrapped_content.push(RLine::from(" "));
                wrapped_content.push(RLine::from(vec![
                    Span::from(" "),
                    Span::styled(
                        "Next steps:",
                        Style::default()
                            .fg(crate::colors::text())
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
                for line in hint.lines() {
                    wrapped_content.push(RLine::from(vec![
                        Span::from("   "),
                        Span::styled(
                            line.trim_end(),
                            Style::default().fg(crate::colors::text_dim()),
                        ),
                    ]));
                }
            }

            if !wrapped_content.is_empty() {
                // Create paragraph with wrapping enabled for the long text content
                let wrapped_paragraph =
                    Paragraph::new(Text::from(wrapped_content)).wrap(Wrap { trim: false });
                wrapped_paragraph.render(wrapped_area, buf);
            }
        }

        // Render sparkline at the bottom if we have data and agents are active
        let sparkline_data = self.sparkline_data.borrow();

        // Debug logging
        tracing::debug!(
            "Sparkline render check: data_len={}, agents={}, ready={}, height={}, actual_height={}, area={:?}",
            sparkline_data.len(),
            self.active_agents.len(),
            self.agents_ready_to_start,
            sparkline_height,
            actual_sparkline_height,
            sparkline_area
        );

        if !sparkline_data.is_empty()
            && (!self.active_agents.is_empty() || self.agents_ready_to_start)
            && actual_sparkline_height > 0
        {
            // Convert data to SparklineBar with colors based on completion status
            let bars: Vec<SparklineBar> = sparkline_data
                .iter()
                .map(|(value, is_completed)| {
                    let color = if *is_completed {
                        crate::colors::success() // Green for completed
                    } else {
                        crate::colors::border() // Border color for normal activity
                    };
                    SparklineBar::from(*value).style(Style::default().fg(color))
                })
                .collect();

            // Use dynamic max based on the actual data for better visibility
            // During preparing/planning, values are small (2-3), during running they're larger (5-15)
            // For planning phase with single line, use smaller max for better visibility
            let max_value = if self.agents_ready_to_start && self.active_agents.is_empty() {
                // Planning phase - use smaller max for better visibility of 1-3 range
                sparkline_data
                    .iter()
                    .map(|(v, _)| *v)
                    .max()
                    .unwrap_or(4)
                    .max(4)
            } else {
                // Running phase - use larger max
                sparkline_data
                    .iter()
                    .map(|(v, _)| *v)
                    .max()
                    .unwrap_or(10)
                    .max(10)
            };

            let sparkline = Sparkline::default().data(bars).max(max_value); // Dynamic max for better visibility
            sparkline.render(sparkline_area, buf);
        }
    }
}

impl WidgetRef for &ChatWidget<'_> {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        // Top-level widget render timing
        let _perf_widget_start = if self.perf_state.enabled {
            Some(std::time::Instant::now())
        } else {
            None
        };

        // Ensure a consistent background even when individual widgets skip
        // painting unchanged regions. Without this, gutters and inter‑cell
        // spacing can show through after we reduced full clears.
        // Cost: one Block render across the frame (O(area)); acceptable and
        // fixes visual artifacts reported after redraw reductions.
        if !self.standard_terminal_mode {
            use ratatui::style::Style;
            use ratatui::widgets::Block;
            let bg = Block::default().style(Style::default().bg(crate::colors::background()));
            bg.render(area, buf);
        }

        // Remember full frame height for HUD sizing logic
        self.layout.last_frame_height.set(area.height);
        self.layout.last_frame_width.set(area.width);

        let layout_areas = self.layout_areas(area);
        let status_bar_area = layout_areas.first().copied().unwrap_or(area);
        let history_area = layout_areas.get(1).copied().unwrap_or(area);
        let bottom_pane_area = layout_areas.get(2).copied().unwrap_or(area);

        // Record the effective bottom pane height for buffer-mode scrollback inserts.
        self.layout
            .last_bottom_reserved_rows
            .set(bottom_pane_area.height);
        // Store the bottom pane area for mouse hit testing
        self.layout.last_bottom_pane_area.set(bottom_pane_area);

        // Render status bar and HUD only in full TUI mode
        if !self.standard_terminal_mode {
            self.render_status_bar(status_bar_area, buf);
        }

        // In standard-terminal mode, do not paint the history region: committed
        // content is appended to the terminal's own scrollback via
        // insert_history_lines and repainting here would overwrite it.
        if self.standard_terminal_mode {
            // Render only the bottom pane (composer or its active view) without painting
            // backgrounds to preserve the terminal's native theme.
            ratatui::widgets::WidgetRef::render_ref(&(&self.bottom_pane), bottom_pane_area, buf);
            // Scrub backgrounds in the bottom pane region so any widget-set bg becomes transparent.
            self.clear_backgrounds_in(buf, bottom_pane_area);
            return;
        }

        // Create a unified scrollable container for all chat content
        // Use consistent padding throughout
        let padding = 1u16;
        let content_area = Rect {
            x: history_area.x + padding,
            y: history_area.y,
            width: history_area.width.saturating_sub(padding * 2),
            height: history_area.height,
        };

        self.update_welcome_height_hint(content_area.height);

        // Reset the full history region to the baseline theme background once per frame.
        // Individual cells only repaint when their visuals differ (e.g., assistant tint),
        // which keeps overdraw minimal while ensuring stale characters disappear.
        let base_style = Style::default()
            .bg(crate::colors::background())
            .fg(crate::colors::text());
        fill_rect(buf, history_area, Some(' '), base_style);

        // Add live streaming content if present
        let streaming_lines = self
            .live_builder
            .display_rows()
            .into_iter()
            .map(|r| ratatui::text::Line::from(r.text))
            .collect::<Vec<_>>();

        let streaming_cell = if !streaming_lines.is_empty() {
            let state =
                self.synthesize_stream_state_from_lines(None, &streaming_lines, true);
            Some(history_cell::new_streaming_content(state, &self.config))
        } else {
            None
        };

        // Append any queued user messages as sticky preview cells at the very
        // end so they always render at the bottom until they are dispatched.
        let mut queued_preview_cells: Vec<crate::history_cell::PlainHistoryCell> = Vec::new();
        if !self.queued_user_messages.is_empty() {
            for qm in &self.queued_user_messages {
                let state = history_cell::new_queued_user_prompt(qm.display_text.clone());
                queued_preview_cells.push(crate::history_cell::PlainHistoryCell::from_state(state));
            }
        }

        self.ensure_render_request_cache();

        let extra_count = (self.active_exec_cell.is_some() as usize)
            .saturating_add(streaming_cell.is_some() as usize)
            .saturating_add(queued_preview_cells.len());
        let request_count = self.history_cells.len().saturating_add(extra_count);

        let mut render_requests_full: Option<Vec<RenderRequest>> = None;

        // Calculate total content height using prefix sums; build if needed
        let spacing = 1u16; // Standard spacing between cells
        const GUTTER_WIDTH: u16 = 2; // Same as in render loop
        let reasoning_visible = self.is_reasoning_shown();
        let cache_width = content_area.width.saturating_sub(GUTTER_WIDTH);

        // Opportunistically clear height cache if width changed
        self.history_render.handle_width_change(cache_width);

        // Perf: count a frame
        if self.perf_state.enabled {
            let mut p = self.perf_state.stats.borrow_mut();
            p.frames = p.frames.saturating_add(1);
        }

        let render_settings = RenderSettings::new(cache_width, self.render_theme_epoch, reasoning_visible);
        self.last_render_settings.set(render_settings);
        if self.history_frozen_count > 0
            && self.history_frozen_width != render_settings.width
            && !self.history_virtualization_sync_pending.get()
        {
            self.history_virtualization_sync_pending.set(true);
            self.app_event_tx.send(AppEvent::SyncHistoryVirtualization);
        }
        let perf_enabled = self.perf_state.enabled;
        let needs_prefix_rebuild =
            self.history_render
                .should_rebuild_prefix(content_area.width, request_count);
        let mut rendered_cells_full: Option<Vec<VisibleCell>> = None;
        if needs_prefix_rebuild {
            if render_requests_full.is_none() {
                let render_request_cache = self.render_request_cache.borrow();
                let mut render_requests = Vec::with_capacity(request_count);
                for (cell, seed) in self
                    .history_cells
                    .iter()
                    .zip(render_request_cache.iter())
                {
                    let assistant = cell
                        .as_any()
                        .downcast_ref::<crate::history_cell::AssistantMarkdownCell>();
                    render_requests.push(RenderRequest {
                        history_id: seed.history_id,
                        cell: Some(cell.as_ref()),
                        assistant,
                        use_cache: seed.use_cache,
                        fallback_lines: seed.fallback_lines.clone(),
                        kind: seed.kind,
                        config: &self.config,
                    });
                }

                if let Some(ref cell) = self.active_exec_cell {
                    render_requests.push(RenderRequest {
                        history_id: HistoryId::ZERO,
                        cell: Some(cell as &dyn HistoryCell),
                        assistant: None,
                        use_cache: false,
                        fallback_lines: None,
                        kind: RenderRequestKind::Legacy,
                        config: &self.config,
                    });
                }

                if let Some(ref cell) = streaming_cell {
                    render_requests.push(RenderRequest {
                        history_id: HistoryId::ZERO,
                        cell: Some(cell as &dyn HistoryCell),
                        assistant: None,
                        use_cache: false,
                        fallback_lines: None,
                        kind: RenderRequestKind::Legacy,
                        config: &self.config,
                    });
                }

                for c in &queued_preview_cells {
                    render_requests.push(RenderRequest {
                        history_id: HistoryId::ZERO,
                        cell: Some(c as &dyn HistoryCell),
                        assistant: None,
                        use_cache: false,
                        fallback_lines: None,
                        kind: RenderRequestKind::Legacy,
                        config: &self.config,
                    });
                }

                if perf_enabled {
                    let mut p = self.perf_state.stats.borrow_mut();
                    p.render_requests_full =
                        p.render_requests_full.saturating_add(render_requests.len() as u64);
                }

                render_requests_full = Some(render_requests);
            }

            let Some(render_requests) = render_requests_full.as_ref() else {
                return;
            };
            let mut used_fast_append = false;
            if self.try_append_prefix_fast(render_requests, render_settings, content_area.width) {
                used_fast_append = true;
                self.history_prefix_append_only.set(true);
            }
            if used_fast_append {
                // Prefix sums already updated; skip the full rebuild path.
                rendered_cells_full = None;
            } else {
            if perf_enabled {
                let mut p = self.perf_state.stats.borrow_mut();
                p.prefix_rebuilds = p.prefix_rebuilds.saturating_add(1);
            }

            let prefix_start = perf_enabled.then(std::time::Instant::now);
            let cells = self.history_render.visible_cells(
                &self.history_state,
                render_requests,
                render_settings,
            );

            let mut prefix: Vec<u16> = Vec::with_capacity(cells.len().saturating_add(1));
            prefix.push(0);
            let mut acc = 0u16;
            let content_width = content_area.width.saturating_sub(GUTTER_WIDTH);
            let mut spacing_ranges: Vec<(u16, u16)> = Vec::new();

            for (idx, vis) in cells.iter().enumerate() {
                let Some(cell) = vis.cell else {
                    continue;
                };
                let line_count = vis.height;
                if self.perf_state.enabled
                    && matches!(vis.height_source, history_render::HeightSource::DesiredHeight)
                {
                    let mut p = self.perf_state.stats.borrow_mut();
                    p.height_misses_render = p.height_misses_render.saturating_add(1);
                    if let Some(ns) = vis.height_measure_ns {
                        let label = self.perf_label_for_item(cell);
                        p.record_render((idx, content_width), label.as_str(), ns);
                    }
                }
                let cell_start = acc;
                acc = acc.saturating_add(line_count);
                let cell_end = acc;

                if cell
                    .as_any()
                    .is::<crate::history_cell::AssistantMarkdownCell>()
                    && line_count >= 2
                {
                    spacing_ranges.push((cell_start, cell_start.saturating_add(1)));
                    spacing_ranges.push((cell_end.saturating_sub(1), cell_end));
                }

                let mut should_add_spacing = idx < cells.len().saturating_sub(1) && line_count > 0;
                if should_add_spacing {
                    let prev_visible_idx = (0..idx).rev().find(|j| cells[*j].height > 0);
                    let next_visible_idx = ((idx + 1)..cells.len()).find(|j| cells[*j].height > 0);

                    if next_visible_idx.is_none() {
                        should_add_spacing = false;
                    } else {
                        let this_collapsed = cell
                            .as_any()
                            .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
                            .map(crate::history_cell::CollapsibleReasoningCell::is_collapsed)
                            .unwrap_or(false);
                        if this_collapsed {
                            let prev_collapsed = prev_visible_idx
                                .and_then(|j| cells[j]
                                    .cell
                                    .and_then(|c| {
                                        c.as_any()
                                            .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
                                            .map(crate::history_cell::CollapsibleReasoningCell::is_collapsed)
                                    }))
                                .unwrap_or(false);
                            let next_collapsed = next_visible_idx
                                .and_then(|j| cells[j]
                                    .cell
                                    .and_then(|c| {
                                        c.as_any()
                                            .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
                                            .map(crate::history_cell::CollapsibleReasoningCell::is_collapsed)
                                    }))
                                .unwrap_or(false);
                            if prev_collapsed && next_collapsed {
                                should_add_spacing = false;
                            }
                        }
                    }
                }
                if should_add_spacing {
                    let spacing_start = acc;
                    acc = acc.saturating_add(spacing);
                    // Track the spacer interval so scroll adjustments can skip over it later.
                    spacing_ranges.push((spacing_start, acc));
                }
                prefix.push(acc);
            }

            let total_height = *prefix.last().unwrap_or(&0);
            if let (true, Some(t0)) = (perf_enabled, prefix_start) {
                let elapsed = t0.elapsed().as_nanos();
                let mut p = self.perf_state.stats.borrow_mut();
                p.ns_total_height = p.ns_total_height.saturating_add(elapsed);
            }
            self.history_render.update_prefix_cache(
                content_area.width,
                prefix,
                total_height,
                cells.len(),
                self.history_cells.len(),
            );
            self.history_render.update_spacing_ranges(spacing_ranges);
            rendered_cells_full = Some(cells);
            self.history_prefix_append_only.set(true);
            }
        }

        if self.history_virtualization_sync_pending.get()
            && !self.history_cells.is_empty()
            && render_settings.width > 0
            && content_area.height > 0
        {
            let prefix_ready = self.history_render.prefix_sums.borrow().len()
                > self.history_cells.len();
            if prefix_ready {
                self.history_virtualization_sync_pending.set(false);
                self.app_event_tx.send(AppEvent::SyncHistoryVirtualization);
            }
        }

        let mut total_height = self.history_render.last_total_height();
        let base_total_height = total_height;
        let viewport_rows = content_area.height;
        let mut requested_spacer_lines = 0u16;
        let mut remainder_for_log: Option<u16> = None;

        if total_height > 0 && viewport_rows > 0 && request_count > 0
            && base_total_height > viewport_rows {
                let remainder = base_total_height % viewport_rows;
                remainder_for_log = Some(remainder);
                if remainder == 0 {
                    requested_spacer_lines = if base_total_height == viewport_rows { 1 } else { 2 };
                } else if remainder <= 2 || remainder >= viewport_rows.saturating_sub(2) {
                    requested_spacer_lines = 1;
                }
            }

        let composer_rows = self.layout.last_bottom_reserved_rows.get();
        let ensure_footer_space = self.layout.scroll_offset.get() == 0
            && composer_rows > 0
            && base_total_height >= viewport_rows
            && request_count > 0;
        if ensure_footer_space {
            requested_spacer_lines = requested_spacer_lines.max(1);
        }

        let (spacer_lines, spacer_pending_shrink) = self
            .history_render
            .select_bottom_spacer_lines(requested_spacer_lines);

        if spacer_pending_shrink {
            // Force a follow-up frame so the spacer can settle back to the newly
            // requested height even if no additional history events arrive. Without
            // this, we'd keep the stale overscan row on-screen until the user types
            // or resizes the window again.
            self.app_event_tx.send(AppEvent::ScheduleFrameIn(
                HISTORY_ANIMATION_FRAME_INTERVAL,
            ));
        }

        if spacer_lines > 0 {
            total_height = total_height.saturating_add(spacer_lines);
            self.history_render
                .set_bottom_spacer_range(Some((base_total_height, total_height)));
            tracing::debug!(
                target: "code_tui::history_render",
                lines = spacer_lines,
                base_height = base_total_height,
                padded_height = total_height,
                viewport = viewport_rows,
                remainder = remainder_for_log,
                footer_padding = ensure_footer_space,
                "history overscan: adding bottom spacer",
            );
        } else {
            self.history_render.set_bottom_spacer_range(None);
        }
        let overscan_extra = total_height.saturating_sub(base_total_height);
        // Calculate scroll position and vertical alignment
        // Preserve a stable viewport anchor when history grows while the user is scrolled up.
        let prev_viewport_h = self.layout.last_history_viewport_height.get();
        let prev_max_scroll = self.layout.last_max_scroll.get();
        let prev_scroll_offset = self.layout.scroll_offset.get().min(prev_max_scroll);
        let prev_scroll_from_top = prev_max_scroll.saturating_sub(prev_scroll_offset);
        if prev_viewport_h == 0 {
            // Initialize on first render
            self.layout
                .last_history_viewport_height
                .set(content_area.height);
        }

        let (start_y, scroll_pos) = if total_height <= content_area.height {
            // Content fits - always align to bottom so "Popular commands" stays at the bottom
            let start_y = content_area.y + content_area.height.saturating_sub(total_height);
            // Update last_max_scroll cache
            self.layout.last_max_scroll.set(0);
            (start_y, 0u16) // No scrolling needed
        } else {
            // Content overflows - calculate scroll position
            // scroll_offset is measured from the bottom (0 = bottom/newest)
            // Convert to distance from the top for rendering math.
            let max_scroll = total_height.saturating_sub(content_area.height);
            if self.layout.scroll_offset.get() > 0 && max_scroll != prev_max_scroll {
                // If the user has scrolled up and the history height changes (e.g. new output
                // arrives while streaming), keep the same content anchored at the top of the
                // viewport by adjusting our bottom-anchored scroll offset.
                self.layout
                    .scroll_offset
                    .set(max_scroll.saturating_sub(prev_scroll_from_top));
            }

            // Update cache and clamp for display only.
            self.layout.last_max_scroll.set(max_scroll);
            let clamped_scroll_offset = self.layout.scroll_offset.get().min(max_scroll);
            let mut scroll_from_top = max_scroll.saturating_sub(clamped_scroll_offset);

            if overscan_extra > 0 && clamped_scroll_offset == 0 {
                scroll_from_top = scroll_from_top.saturating_sub(overscan_extra);
            }

            if clamped_scroll_offset == 0 && content_area.height == 1 {
                scroll_from_top = self
                    .history_render
                    .adjust_scroll_to_content(scroll_from_top);
            }

            // NOTE: when pinned to the bottom, avoid guessing at cell-internal padding.
            // Only skip known spacer intervals recorded by the history render cache.

            tracing::debug!(
                target: "code_tui::scrollback",
                total_height,
                base_total_height,
                viewport = content_area.height,
                overscan_extra,
                max_scroll,
                scroll_offset = clamped_scroll_offset,
                initial_scroll_from_top = scroll_from_top,
                "scrollback pre-adjust scroll position",
            );

            // If our scroll origin landed on a spacer row between cells, nudge it up so
            // the viewport starts with real content instead of an empty separator.
            let scroll_pos = if clamped_scroll_offset > 0 {
                let adjusted = self
                    .history_render
                    .adjust_scroll_to_content(scroll_from_top);
                tracing::debug!(
                    target: "code_tui::scrollback",
                    adjusted_scroll_from_top = adjusted,
                    scroll_from_top,
                    "scrollback adjusted scroll position",
                );
                adjusted
            } else {
                scroll_from_top
            };

            (content_area.y, scroll_pos)
        };

        // Record current viewport height for the next frame
        self.layout
            .last_history_viewport_height
            .set(content_area.height);

        let _perf_hist_clear_start = if self.perf_state.enabled {
            Some(std::time::Instant::now())
        } else {
            None
        };

        // Render the scrollable content with spacing using prefix sums
        let mut screen_y = start_y; // Position on screen
        let spacing = 1u16; // Spacing between cells
        let viewport_bottom = scroll_pos.saturating_add(content_area.height);
        let ps_ref = self.history_render.prefix_sums.borrow();
        let ps: &Vec<u16> = &ps_ref;
        let mut start_idx = match ps.binary_search(&scroll_pos) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        start_idx = start_idx.min(request_count);
        let mut end_idx = match ps.binary_search(&viewport_bottom) {
            Ok(i) => i,
            Err(i) => i,
        };
        end_idx = end_idx.saturating_add(1).min(request_count);

        enum VisibleRequests<'a> {
            Full(&'a [RenderRequest<'a>]),
            Owned(Vec<RenderRequest<'a>>),
        }

        let history_len = self.history_cells.len();
        let visible_requests = if let Some(ref full_requests) = render_requests_full {
            VisibleRequests::Full(&full_requests[start_idx..end_idx])
        } else {
            let render_request_cache = self.render_request_cache.borrow();
            let mut requests = Vec::with_capacity(end_idx.saturating_sub(start_idx));
            for idx in start_idx..end_idx {
                if idx < history_len {
                    let cell = &self.history_cells[idx];
                    let seed = &render_request_cache[idx];
                    let assistant = cell
                        .as_any()
                        .downcast_ref::<crate::history_cell::AssistantMarkdownCell>();
                    requests.push(RenderRequest {
                        history_id: seed.history_id,
                        cell: Some(cell.as_ref()),
                        assistant,
                        use_cache: seed.use_cache,
                        fallback_lines: seed.fallback_lines.clone(),
                        kind: seed.kind,
                        config: &self.config,
                    });
                    continue;
                }

                let extra_idx = idx.saturating_sub(history_len);
                let mut extra_cursor = 0usize;
                if let Some(ref cell) = self.active_exec_cell {
                    if extra_idx == extra_cursor {
                        requests.push(RenderRequest {
                            history_id: HistoryId::ZERO,
                            cell: Some(cell as &dyn HistoryCell),
                            assistant: None,
                            use_cache: false,
                            fallback_lines: None,
                            kind: RenderRequestKind::Legacy,
                            config: &self.config,
                        });
                        continue;
                    }
                    extra_cursor = extra_cursor.saturating_add(1);
                }

                if let Some(ref cell) = streaming_cell {
                    if extra_idx == extra_cursor {
                        requests.push(RenderRequest {
                            history_id: HistoryId::ZERO,
                            cell: Some(cell as &dyn HistoryCell),
                            assistant: None,
                            use_cache: false,
                            fallback_lines: None,
                            kind: RenderRequestKind::Legacy,
                            config: &self.config,
                        });
                        continue;
                    }
                    extra_cursor = extra_cursor.saturating_add(1);
                }

                let queued_idx = extra_idx.saturating_sub(extra_cursor);
                if let Some(cell) = queued_preview_cells.get(queued_idx) {
                    requests.push(RenderRequest {
                        history_id: HistoryId::ZERO,
                        cell: Some(cell as &dyn HistoryCell),
                        assistant: None,
                        use_cache: false,
                        fallback_lines: None,
                        kind: RenderRequestKind::Legacy,
                        config: &self.config,
                    });
                }
            }
            VisibleRequests::Owned(requests)
        };

        let visible_requests_slice = match &visible_requests {
            VisibleRequests::Full(slice) => *slice,
            VisibleRequests::Owned(vec) => vec.as_slice(),
        };

        if perf_enabled {
            let mut p = self.perf_state.stats.borrow_mut();
            p.render_requests_visible = p
                .render_requests_visible
                .saturating_add(visible_requests_slice.len() as u64);
        }

        let mut _subset_rendered: Option<Vec<VisibleCell>> = None;
        let visible_slice: &[VisibleCell] = if let Some(ref full) = rendered_cells_full {
            &full[start_idx..end_idx]
        } else {
            _subset_rendered = Some(self.history_render.visible_cells(
                &self.history_state,
                visible_requests_slice,
                render_settings,
            ));
            _subset_rendered.as_deref().unwrap_or(&[])
        };

        // Only schedule animation frames if an animating cell is actually visible.
        let has_visible_animation = visible_slice.iter().any(|visible| {
            visible
                .cell
                .map(crate::history_cell::HistoryCell::is_animating)
                .unwrap_or(false)
        });
        if has_visible_animation && !ChatWidget::auto_reduced_motion_preference() {
            tracing::debug!("Visible animation detected, scheduling next frame");
            self.app_event_tx
                .send(AppEvent::ScheduleFrameIn(HISTORY_ANIMATION_FRAME_INTERVAL));
        }

        let render_loop_start = if self.perf_state.enabled {
            Some(std::time::Instant::now())
        } else {
            None
        };
        #[derive(Debug)]
        struct HeightMismatch {
            history_id: HistoryId,
            cached: u16,
            recomputed: u16,
            idx: usize,
            preview: String,
        }

        let mut height_mismatches: Vec<HeightMismatch> = Vec::new();
        let is_collapsed_reasoning_at = |idx: usize| {
            if idx >= request_count {
                return false;
            }
            if idx < history_len {
                return self.history_cells[idx]
                    .as_any()
                    .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
                    .map(crate::history_cell::CollapsibleReasoningCell::is_collapsed)
                    .unwrap_or(false);
            }
            false
        };

        for (offset, visible) in visible_slice.iter().enumerate() {
            let idx = start_idx + offset;
            let Some(item) = visible.cell else {
                continue;
            };
            // Calculate height with reduced width due to gutter
            const GUTTER_WIDTH: u16 = 2;
            let content_width = content_area.width.saturating_sub(GUTTER_WIDTH);
            let maybe_assistant = item
                .as_any()
                .downcast_ref::<crate::history_cell::AssistantMarkdownCell>();
            let is_streaming = item
                .as_any()
                .downcast_ref::<crate::history_cell::StreamingContentCell>()
                .is_some();

            let mut layout_for_render: Option<Rc<CachedLayout>> = visible
                .layout
                .as_ref()
                .map(super::history_render::LayoutRef::layout);

            let item_height = visible.height;
            if content_area.width > 0
                && let Some(req) = visible_requests_slice.get(offset)
                    && req.history_id != HistoryId::ZERO
                        && matches!(item.kind(), history_cell::HistoryCellType::Reasoning)
                    {
                        if item_height == 0 && content_width == 0 {
                            // Zero-width viewport leaves both cached and computed heights at 0.
                            // Skip to avoid false positives during aggressive window resizes.
                            continue;
                        }

                        #[cfg(debug_assertions)]
                        {
                            let mut preview: Option<String> = None;
                            let fresh = item.desired_height(content_width);
                            if fresh != item_height {
                                if preview.is_none() {
                                    let lines = item.display_lines_trimmed();
                                    if !lines.is_empty() {
                                        preview = Some(ChatWidget::reasoning_preview(&lines));
                                    }
                                }
                                height_mismatches.push(HeightMismatch {
                                    history_id: req.history_id,
                                    cached: item_height,
                                    recomputed: fresh,
                                    idx,
                                    preview: preview.unwrap_or_default(),
                                });
                            }
                        }
                    }
            if self.perf_state.enabled
                && rendered_cells_full.is_none()
                && matches!(visible.height_source, history_render::HeightSource::DesiredHeight)
            {
                let mut p = self.perf_state.stats.borrow_mut();
                p.height_misses_render = p.height_misses_render.saturating_add(1);
                if let Some(ns) = visible.height_measure_ns {
                    let label = self.perf_label_for_item(item);
                    p.record_render((idx, content_width), label.as_str(), ns);
                }
            }

            let content_y = ps[idx];

            // The prefix sums already account for spacer rows between cells (and omit the
            // trailing spacer). Avoid additional compensation when rendering the final
            // visible cell; otherwise we double-count spacing and trim the last line.
            let skip_top = scroll_pos.saturating_sub(content_y);

            // Stop if we've gone past the bottom of the screen
            if screen_y >= content_area.y + content_area.height {
                break;
            }

            // Calculate how much height is available for this item
            let available_height = (content_area.y + content_area.height).saturating_sub(screen_y);
            let visible_height = item_height.saturating_sub(skip_top).min(available_height);


            if visible_height > 0 {
                // Define gutter width (2 chars: symbol + space)
                const GUTTER_WIDTH: u16 = 2;

                // Split area into gutter and content
                let gutter_area = Rect {
                    x: content_area.x,
                    y: screen_y,
                    width: GUTTER_WIDTH.min(content_area.width),
                    height: visible_height,
                };

                let item_area = Rect {
                    x: content_area.x + GUTTER_WIDTH.min(content_area.width),
                    y: screen_y,
                    width: content_area.width.saturating_sub(GUTTER_WIDTH),
                    height: visible_height,
                };

                if history_cell_logging_enabled() {
                    let row_start = item_area.y;
                    let row_end = item_area
                        .y
                        .saturating_add(visible_height)
                        .saturating_sub(1);
                    let cache_hit = layout_for_render.is_some();
                    tracing::info!(
                        target: "code_tui::history_cells",
                        idx,
                        kind = ?item.kind(),
                        row_start,
                        row_end,
                        height = visible_height,
                        width = item_area.width,
                        skip_rows = skip_top,
                        item_height,
                        content_y,
                        cache_hit,
                        assistant = maybe_assistant.is_some(),
                        streaming = is_streaming,
                        custom = item.has_custom_render(),
                        animating = item.is_animating(),
                        "history cell render",
                    );
                }

                // Paint gutter background. For Assistant and Auto Review, extend the tint under the
                // gutter and also one extra column to the left (so the • has color on both sides),
                // without changing layout or symbol positions.
                let is_assistant =
                    matches!(item.kind(), crate::history_cell::HistoryCellType::Assistant);
                let is_auto_review = ChatWidget::is_auto_review_cell(item);
                let auto_review_bg = crate::history_cell::PlainHistoryCell::auto_review_bg();
                let gutter_bg = if is_assistant {
                    crate::colors::assistant_bg()
                } else if is_auto_review {
                    auto_review_bg
                } else {
                    crate::colors::background()
                };

                // Paint gutter background for assistant/auto-review cells so the tinted
                // strip appears contiguous with the message body. This avoids
                // the light "hole" seen after we reduced redraws. For other
                // cell types keep the default background (already painted by
                // the frame bg fill above).
                if (is_assistant || is_auto_review) && gutter_area.width > 0 && gutter_area.height > 0 {
                    let _perf_gutter_start = if self.perf_state.enabled {
                        Some(std::time::Instant::now())
                    } else {
                        None
                    };
                    let style = Style::default().bg(gutter_bg);
                    let mut tint_x = gutter_area.x;
                    let mut tint_width = gutter_area.width;
                    if content_area.x > history_area.x {
                        tint_x = content_area.x.saturating_sub(1);
                        tint_width = tint_width.saturating_add(1);
                    }
                    let tint_rect = Rect::new(tint_x, gutter_area.y, tint_width, gutter_area.height);
                    fill_rect(buf, tint_rect, Some(' '), style);
                    // Also tint one column immediately to the right of the content area
                    // so the assistant block is visually bookended. This column lives in the
                    // right padding stripe; when the scrollbar is visible it will draw over
                    // the far-right edge, which is fine.
                    let right_col_x = content_area.x.saturating_add(content_area.width);
                    let history_right = history_area.x.saturating_add(history_area.width);
                    if right_col_x < history_right {
                        let right_rect = Rect::new(right_col_x, item_area.y, 1, item_area.height);
                        fill_rect(buf, right_rect, Some(' '), style);
                    }
                    if let Some(t0) = _perf_gutter_start {
                        let dt = t0.elapsed().as_nanos();
                        let mut p = self.perf_state.stats.borrow_mut();
                        p.ns_gutter_paint = p.ns_gutter_paint.saturating_add(dt);
                        // Rough accounting: area of gutter rectangle (clamped to u64)
                        let area_cells: u64 =
                            (gutter_area.width as u64).saturating_mul(gutter_area.height as u64);
                        p.cells_gutter_paint = p.cells_gutter_paint.saturating_add(area_cells);
                    }
                }

                // Render gutter symbol if present
                if let Some(symbol) = item.gutter_symbol() {
                    // Choose color based on symbol/type
                    let color = if is_auto_review {
                        crate::colors::success()
                    } else if symbol == "❯" {
                        // Executed arrow – color reflects exec state
                        if let Some(exec) = item
                            .as_any()
                            .downcast_ref::<crate::history_cell::ExecCell>()
                        {
                            match &exec.output {
                                None => crate::colors::text(), // Running...
                                // Successful runs use the theme success color so the arrow stays visible on all themes
                                Some(o) if o.exit_code == 0 => crate::colors::text(),
                                Some(_) => crate::colors::error(),
                            }
                        } else {
                            // Handle merged exec cells (multi-block "Ran") the same as single execs
                            match item.kind() {
                                crate::history_cell::HistoryCellType::Exec {
                                    kind: crate::history_cell::ExecKind::Run,
                                    status: crate::history::state::ExecStatus::Success,
                                } => crate::colors::text(),
                                crate::history_cell::HistoryCellType::Exec {
                                    kind: crate::history_cell::ExecKind::Run,
                                    status: crate::history::state::ExecStatus::Error,
                                } => crate::colors::error(),
                                crate::history_cell::HistoryCellType::Exec { .. } => {
                                    crate::colors::text()
                                }
                                _ => crate::colors::text(),
                            }
                        }
                    } else if symbol == "↯" {
                        // Patch/Updated arrow color – match the header text color
                        match item.kind() {
                            crate::history_cell::HistoryCellType::Patch {
                                kind: crate::history_cell::PatchKind::ApplySuccess,
                            } => crate::colors::success(),
                            crate::history_cell::HistoryCellType::Patch {
                                kind: crate::history_cell::PatchKind::ApplyBegin,
                            } => crate::colors::success(),
                            crate::history_cell::HistoryCellType::Patch {
                                kind: crate::history_cell::PatchKind::Proposed,
                            } => crate::colors::primary(),
                            crate::history_cell::HistoryCellType::Patch {
                                kind: crate::history_cell::PatchKind::ApplyFailure,
                            } => crate::colors::error(),
                            _ => crate::colors::primary(),
                        }
                    } else if matches!(symbol, "◐" | "◓" | "◑" | "◒")
                        && item
                            .as_any()
                            .downcast_ref::<crate::history_cell::RunningToolCallCell>()
                            .is_some_and(|cell| cell.has_title("Waiting"))
                    {
                        crate::colors::text_bright()
                    } else if matches!(symbol, "○" | "◔" | "◑" | "◕" | "●") {
                        if let Some(plan_cell) = item
                            .as_any()
                            .downcast_ref::<crate::history_cell::PlanUpdateCell>()
                        {
                            if plan_cell.is_complete() {
                                crate::colors::success()
                            } else {
                                crate::colors::info()
                            }
                        } else {
                            crate::colors::success()
                        }
                    } else {
                        match symbol {
                            "›" => crate::colors::text(),        // user
                            "⋮" => crate::colors::primary(),     // thinking
                            "•" => crate::colors::text_bright(), // codex/agent
                            "⚙" => crate::colors::info(),        // tool working
                            "✔" => crate::colors::success(),     // tool complete
                            "✖" => crate::colors::error(),       // error
                            "★" => crate::colors::text_bright(), // notice/popular
                            _ => crate::colors::text_dim(),
                        }
                    };

                    // Draw the symbol anchored to the top of the message (not the viewport).
                    // "Top of the message" accounts for any intentional top padding per cell type.
                    // As you scroll past that anchor, the icon scrolls away with the message.
                    if gutter_area.width >= 2 {
                        // Anchor offset counted from the very start of the item's painted area
                        // to the first line of its content that the icon should align with.
                        let anchor_offset: u16 = match item.kind() {
                            // Assistant messages render with one row of top padding so that
                            // the content visually aligns; anchor to that second row.
                            crate::history_cell::HistoryCellType::Assistant => 1,
                            _ if is_auto_review => {
                                crate::history_cell::PlainHistoryCell::auto_review_padding().0
                            }
                            _ => 0,
                        };

                        // If we've scrolled past the anchor line, don't render the icon.
                        if skip_top <= anchor_offset {
                            let rel = anchor_offset - skip_top; // rows from current viewport top
                            let symbol_y = gutter_area.y.saturating_add(rel);
                            if symbol_y < gutter_area.y.saturating_add(gutter_area.height) {
                                let symbol_style = Style::default().fg(color).bg(gutter_bg);
                                let symbol_x = gutter_area.x;
                                buf.set_string(symbol_x, symbol_y, symbol, symbol_style);
                            }
                        }
                    }
                }

                // Render only the visible window of the item using vertical skip
                let skip_rows = skip_top;

                // Log all cells being rendered
                let is_animating = item.is_animating();
                let has_custom = item.has_custom_render();

                if is_animating || has_custom {
                    tracing::debug!(
                        ">>> RENDERING ANIMATION Cell[{}]: area={:?}, skip_rows={}",
                        idx,
                        item_area,
                        skip_rows
                    );
                }

                // Render the cell content first
                let mut handled_assistant = false;
                if let Some(plan) = visible.assistant_plan.as_ref()
                    && let Some(assistant) = visible
                        .cell
                        .and_then(|c| c.as_any().downcast_ref::<crate::history_cell::AssistantMarkdownCell>())
                    {
                        if skip_rows < plan.total_rows() && item_area.height > 0 {
                            assistant.render_with_layout(plan.as_ref(), item_area, buf, skip_rows);
                        }
                        handled_assistant = true;
                        layout_for_render = None;
                    }

                if !handled_assistant {
                    if let Some(layout_rc) = layout_for_render.as_ref() {
                        self.render_cached_lines(
                            item,
                            layout_rc.as_ref(),
                            item_area,
                            buf,
                            skip_rows,
                        );
                    } else {
                        item.render_with_skip(item_area, buf, skip_rows);
                    }
                }

                // Debug: overlay order info on the spacing row below (or above if needed).
                if self.show_order_overlay
                    && let Some(Some(info)) = self.cell_order_dbg.get(idx) {
                        let mut text = format!("⟦{info}⟧");
                        // Live reasoning diagnostics: append current title detection snapshot
                        if let Some(rc) = item
                            .as_any()
                            .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
                        {
                            let snap = rc.debug_title_overlay();
                            text.push_str(" | ");
                            text.push_str(&snap);
                        }
                        let style = Style::default().fg(crate::colors::text_dim());
                        // Prefer below the item in the one-row spacing area
                        let below_y = item_area.y.saturating_add(visible_height);
                        let bottom_y = content_area.y.saturating_add(content_area.height);
                        let maxw = item_area.width as usize;
                        // Truncate safely by display width, not by bytes, to avoid
                        // panics on non-UTF-8 boundaries (e.g., emoji/CJK). Use the
                        // same width logic as our live wrap utilities.
                        let draw_text = {
                            use unicode_width::UnicodeWidthStr as _;
                            if text.width() > maxw {
                                crate::live_wrap::take_prefix_by_width(&text, maxw).0
                            } else {
                                text.clone()
                            }
                        };
                        if item_area.width > 0 {
                            if below_y < bottom_y {
                                buf.set_string(item_area.x, below_y, draw_text.clone(), style);
                            } else if item_area.y > content_area.y {
                                // Fall back to above the item if no space below
                                let above_y = item_area.y.saturating_sub(1);
                                buf.set_string(item_area.x, above_y, draw_text.clone(), style);
                            }
                        }
                    }
                screen_y += visible_height;
            }

            // Add spacing only if something was actually rendered for this item.
            // Prevent a stray blank when zero-height, and suppress spacing between
            // consecutive collapsed reasoning titles so they appear as a tight list.
            if idx == request_count.saturating_sub(1) {
                let viewport_top = content_area.y;
                let viewport_bottom = content_area.y.saturating_add(content_area.height);
                tracing::debug!(
                    target: "code_tui::scrollback",
                    idx,
                    request_count,
                    content_y,
                    scroll_pos,
                    viewport_top,
                    viewport_bottom,
                    skip_top,
                    item_height,
                    available_height,
                    visible_height,
                    screen_y,
                    spacing,
                    "last visible history cell metrics"
                );
            }

            let mut should_add_spacing = idx < request_count.saturating_sub(1) && visible_height > 0;
            if should_add_spacing {
                // Special-case: two adjacent collapsed reasoning cells → no spacer.
                let this_is_collapsed_reasoning = item
                    .as_any()
                    .downcast_ref::<crate::history_cell::CollapsibleReasoningCell>()
                    .map(crate::history_cell::CollapsibleReasoningCell::is_collapsed)
                    .unwrap_or(false);
                if this_is_collapsed_reasoning {
                    let prev_is_collapsed_reasoning = idx
                        .checked_sub(1)
                        .map(is_collapsed_reasoning_at)
                        .unwrap_or(false);
                    let next_is_collapsed_reasoning = is_collapsed_reasoning_at(idx + 1);
                    if prev_is_collapsed_reasoning && next_is_collapsed_reasoning {
                        should_add_spacing = false;
                    }
                }
            }
            if should_add_spacing {
                let bottom = content_area.y + content_area.height;
                if screen_y < bottom {
                    // Maintain the single-row spacer between cells (critical for explore →
                    // reasoning bundles) while respecting the visible viewport height. This keeps
                    // the rendered gaps consistent with the cached prefix sums even after scroll
                    // adjustments.
                    let spacing_rows = spacing.min(bottom.saturating_sub(screen_y));
                    screen_y = screen_y.saturating_add(spacing_rows);
                }
            }
        }

        drop(ps_ref);

        if let Some(first) = height_mismatches.first() {
            for mismatch in &height_mismatches {
                tracing::error!(
                    target: "code_tui::history_cells",
                    history_id = ?mismatch.history_id,
                    idx = mismatch.idx,
                    cached = mismatch.cached,
                    recomputed = mismatch.recomputed,
                    preview = %mismatch.preview,
                    "History cell height mismatch detected; aborting to capture repro",
                );
            }
            panic!(
                "history cell height mismatch ({} cases); first id={:?} cached={} recomputed={} preview={}",
                height_mismatches.len(),
                first.history_id,
                first.cached,
                first.recomputed,
                first.preview
            );
        }
        if let Some(start) = render_loop_start
            && self.perf_state.enabled {
                let elapsed = start.elapsed().as_nanos();
                let pending_scroll = self.perf_state.pending_scroll_rows.get();
                {
                    let mut p = self.perf_state.stats.borrow_mut();
                    p.ns_render_loop = p.ns_render_loop.saturating_add(elapsed);
                    if pending_scroll > 0 {
                        p.record_scroll_render(pending_scroll, elapsed);
                    }
                }
                if pending_scroll > 0 {
                    self.perf_state.pending_scroll_rows.set(0);
                }
            }

        // Clear any bottom gap inside the content area that wasn’t covered by items
        if screen_y < content_area.y + content_area.height {
            let _perf_hist_clear2 = if self.perf_state.enabled {
                Some(std::time::Instant::now())
            } else {
                None
            };
            let gap_height = (content_area.y + content_area.height).saturating_sub(screen_y);
            if gap_height > 0 {
                let gap_rect = Rect::new(content_area.x, screen_y, content_area.width, gap_height);
                fill_rect(buf, gap_rect, Some(' '), base_style);
            }
            if let Some(t0) = _perf_hist_clear2 {
                let dt = t0.elapsed().as_nanos();
                let mut p = self.perf_state.stats.borrow_mut();
                p.ns_history_clear = p.ns_history_clear.saturating_add(dt);
                let cells = (content_area.width as u64)
                    * ((content_area.y + content_area.height - screen_y) as u64);
                p.cells_history_clear = p.cells_history_clear.saturating_add(cells);
            }
        }

        // Render vertical scrollbar when content is scrollable and currently visible
        // Auto-hide after a short delay to avoid copying it along with text.
        let now = std::time::Instant::now();
        let show_scrollbar = total_height > content_area.height
            && self
                .layout
                .scrollbar_visible_until
                .get()
                .map(|t| now < t)
                .unwrap_or(false);
        if show_scrollbar {
            let mut sb_state = self.layout.vertical_scrollbar_state.borrow_mut();
            // Scrollbar expects number of scroll positions, not total rows.
            // For a viewport of H rows and content of N rows, there are
            // max_scroll = N - H positions; valid positions = [0, max_scroll].
            let max_scroll = total_height.saturating_sub(content_area.height);
            let scroll_positions = max_scroll.saturating_add(1).max(1) as usize;
            let pos = scroll_pos.min(max_scroll) as usize;
            *sb_state = sb_state.content_length(scroll_positions).position(pos);
            // Theme-aware scrollbar styling (line + block)
            // Track: thin line using border color; Thumb: block using border_focused.
            let theme = crate::theme::current_theme();
            let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .symbols(scrollbar_symbols::VERTICAL)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .track_style(
                    Style::default()
                        .fg(crate::colors::border())
                        .bg(crate::colors::background()),
                )
                .thumb_symbol("█")
                .thumb_style(
                    Style::default()
                        .fg(theme.border_focused)
                        .bg(crate::colors::background()),
                );
            // To avoid a small jump at the bottom due to spacer toggling,
            // render the scrollbar in a slightly shorter area (reserve 1 row).
            let sb_area = Rect {
                x: history_area.x,
                y: history_area.y,
                width: history_area.width,
                height: history_area.height.saturating_sub(1),
            };
            StatefulWidget::render(sb, sb_area, buf, &mut sb_state);
        }

        if self.terminal.overlay().is_some() {
            let bg_style = Style::default().bg(crate::colors::background());
            fill_rect(buf, bottom_pane_area, Some(' '), bg_style);
        } else if self.agents_terminal.active {
            let bg_style = Style::default().bg(crate::colors::background());
            fill_rect(buf, bottom_pane_area, Some(' '), bg_style);
        } else {
            // Render the bottom pane directly without a border for now
            // The composer has its own layout with hints at the bottom
            (&self.bottom_pane).render(bottom_pane_area, buf);
        }

        if let Some(overlay) = self.terminal.overlay() {
            let scrim_style = Style::default()
                .bg(crate::colors::overlay_scrim())
                .fg(crate::colors::text_dim());
            fill_rect(buf, area, None, scrim_style);

            let padding = 1u16;
            let footer_reserved = 1.min(bottom_pane_area.height);
            let overlay_bottom = (bottom_pane_area.y + bottom_pane_area.height)
                .saturating_sub(footer_reserved);
            let overlay_height = overlay_bottom
                .saturating_sub(history_area.y)
                .max(1)
                .min(area.height);
            let window_area = Rect {
                x: history_area.x + padding,
                y: history_area.y,
                width: history_area.width.saturating_sub(padding * 2),
                height: overlay_height,
            };
            Clear.render(window_area, buf);

            let block = Block::default()
                .borders(Borders::ALL)
                .title(ratatui::text::Line::from(vec![
                    ratatui::text::Span::styled(
                        format!(" Terminal - {} ", overlay.title),
                        Style::default().fg(crate::colors::text()),
                    ),
                ]))
                .style(Style::default().bg(crate::colors::background()))
                .border_style(
                    Style::default()
                        .fg(crate::colors::border())
                        .bg(crate::colors::background()),
                );
            let inner = block.inner(window_area);
            block.render(window_area, buf);

            let inner_bg = Style::default().bg(crate::colors::background());
            for y in inner.y..inner.y + inner.height {
                for x in inner.x..inner.x + inner.width {
                    buf[(x, y)].set_style(inner_bg);
                }
            }

            let content = inner.inner(ratatui::layout::Margin::new(1, 0));
            if content.height == 0 || content.width == 0 {
                self.terminal.last_visible_rows.set(0);
                self.terminal.last_visible_cols.set(0);
            } else {
                let header_height = 1.min(content.height);
                let footer_height = if content.height >= 2 { 2 } else { 0 };

                let header_area = Rect {
                    x: content.x,
                    y: content.y,
                    width: content.width,
                    height: header_height,
                };
                let footer_area = if footer_height > 0 {
                    Rect {
                        x: content.x,
                        y: content
                            .y
                            .saturating_add(content.height.saturating_sub(footer_height)),
                        width: content.width,
                        height: footer_height,
                    }
                } else {
                    header_area
                };

                if header_height > 0 {
                    fill_rect(buf, header_area, Some(' '), inner_bg);
                    let width_limit = header_area.width as usize;
                    let mut header_spans: Vec<ratatui::text::Span<'static>> = Vec::new();
                    let mut consumed_width: usize = 0;

                    if overlay.running {
                        let now_ms = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis();
                        let frame = crate::spinner::frame_at_time(
                            crate::spinner::current_spinner(),
                            now_ms,
                        );
                        if !frame.is_empty() {
                            consumed_width += frame.chars().count();
                            header_spans.push(ratatui::text::Span::styled(
                                frame,
                                Style::default().fg(crate::colors::spinner()),
                            ));
                            header_spans.push(ratatui::text::Span::raw(" "));
                            consumed_width = consumed_width.saturating_add(1);
                        }

                        let status_text = overlay
                            .start_time
                            .map(|start| format!("Running… ({})", format_duration(start.elapsed())))
                            .unwrap_or_else(|| "Running…".to_string());
                        consumed_width = consumed_width
                            .saturating_add(UnicodeWidthStr::width(status_text.as_str()));
                        header_spans.push(ratatui::text::Span::styled(
                            status_text,
                            Style::default().fg(crate::colors::text_dim()),
                        ));

                        let interval = crate::spinner::current_spinner().interval_ms.max(50);
                        self.app_event_tx
                            .send(AppEvent::ScheduleFrameIn(Duration::from_millis(interval)));
                    } else {
                        let (icon, color, status_text) = match overlay.exit_code {
                            Some(0) => (
                                "✔",
                                crate::colors::success(),
                                overlay
                                    .duration
                                    .map(|d| format!("Completed in {}", format_duration(d)))
                                    .unwrap_or_else(|| "Completed".to_string()),
                            ),
                            Some(code) => (
                                "✖",
                                crate::colors::error(),
                                overlay
                                    .duration
                                    .map(|d| format!("Exit {code} in {}", format_duration(d)))
                                    .unwrap_or_else(|| format!("Exit {code}")),
                            ),
                            None => (
                                "⚠",
                                crate::colors::warning(),
                                overlay
                                    .duration
                                    .map(|d| format!("Stopped after {}", format_duration(d)))
                                    .unwrap_or_else(|| "Stopped".to_string()),
                            ),
                        };

                        header_spans.push(ratatui::text::Span::styled(
                            format!("{icon} "),
                            Style::default().fg(color),
                        ));
                        consumed_width = consumed_width.saturating_add(icon.chars().count() + 1);

                        consumed_width = consumed_width
                            .saturating_add(UnicodeWidthStr::width(status_text.as_str()));
                        header_spans.push(ratatui::text::Span::styled(
                            status_text,
                            Style::default().fg(crate::colors::text_dim()),
                        ));
                    }

                    if !overlay.command_display.is_empty() && width_limit > consumed_width + 5 {
                        let remaining = width_limit.saturating_sub(consumed_width + 5);
                        if remaining > 0 {
                            let truncated = ChatWidget::truncate_with_ellipsis(
                                &overlay.command_display,
                                remaining,
                            );
                            if !truncated.is_empty() {
                                header_spans.push(ratatui::text::Span::styled(
                                    "  •  ",
                                    Style::default().fg(crate::colors::text_dim()),
                                ));
                                header_spans.push(ratatui::text::Span::styled(
                                    truncated,
                                    Style::default().fg(crate::colors::text()),
                                ));
                            }
                        }
                    }

                    let header_line = ratatui::text::Line::from(header_spans);
                    Paragraph::new(RtText::from(vec![header_line]))
                        .wrap(ratatui::widgets::Wrap { trim: true })
                        .render(header_area, buf);
                }

                let mut body_space = content
                    .height
                    .saturating_sub(header_height.saturating_add(footer_height));
                let body_top = header_area.y.saturating_add(header_area.height);
                let mut bottom_cursor = body_top.saturating_add(body_space);

                let mut pending_visible = false;
                let mut pending_box: Option<(Rect, Vec<RtLine<'static>>)> = None;
                if let Some(pending) = overlay.pending_command.as_ref()
                    && let Some((pending_lines, pending_height)) =
                        pending_command_box_lines(pending, content.width)
                        && pending_height <= body_space && pending_height > 0 {
                            bottom_cursor = bottom_cursor.saturating_sub(pending_height);
                            let pending_area = Rect {
                                x: content.x,
                                y: bottom_cursor,
                                width: content.width,
                                height: pending_height,
                            };
                            body_space = body_space.saturating_sub(pending_height);
                            pending_box = Some((pending_area, pending_lines));
                            pending_visible = true;
                        }

                let body_area = Rect {
                    x: content.x,
                    y: body_top,
                    width: content.width,
                    height: body_space,
                };

                // Body content
                let rows = body_area.height;
                let cols = body_area.width;
                let prev_rows = self.terminal.last_visible_rows.replace(rows);
                let prev_cols = self.terminal.last_visible_cols.replace(cols);
                if rows > 0 && cols > 0 && (prev_rows != rows || prev_cols != cols) {
                    self.app_event_tx.send(AppEvent::TerminalResize {
                        id: overlay.id,
                        rows,
                        cols,
                    });
                }

                if rows > 0 && cols > 0 {
                    let mut rendered_rows: Vec<RtLine<'static>> = Vec::new();
                    if overlay.truncated {
                        rendered_rows.push(ratatui::text::Line::from(vec![
                            ratatui::text::Span::styled(
                                "… output truncated (showing last 10,000 lines)",
                                Style::default().fg(crate::colors::text_dim()),
                            ),
                        ]));
                    }
                    rendered_rows.extend(overlay.lines.iter().cloned());
                    let total = rendered_rows.len();
                    let visible = rows as usize;
                    if visible > 0 {
                        let max_scroll = total.saturating_sub(visible);
                        let scroll = (overlay.scroll as usize).min(max_scroll);
                        let end = (scroll + visible).min(total);
                        let window = rendered_rows.get(scroll..end).unwrap_or(&[]);
                        Paragraph::new(RtText::from(window.to_vec()))
                            .wrap(ratatui::widgets::Wrap { trim: false })
                            .render(body_area, buf);
                    }
                }

                if let Some((pending_area, pending_lines)) = pending_box {
                    render_text_box(
                        pending_area,
                        " Command ",
                        crate::colors::function(),
                        pending_lines,
                        buf,
                    );
                }

                // Footer hints
                let mut footer_spans = vec![
                    ratatui::text::Span::styled(
                        "↑↓",
                        Style::default().fg(crate::colors::function()),
                    ),
                    ratatui::text::Span::styled(
                        " Scroll  ",
                        Style::default().fg(crate::colors::text_dim()),
                    ),
                    ratatui::text::Span::styled(
                        "Esc",
                        Style::default().fg(crate::colors::error()),
                    ),
                    ratatui::text::Span::styled(
                        if overlay.running { " Cancel  " } else { " Close  " },
                        Style::default().fg(crate::colors::text_dim()),
                    ),
                ];
                if overlay.running {
                    footer_spans.push(ratatui::text::Span::styled(
                        "Ctrl+C",
                        Style::default().fg(crate::colors::warning()),
                    ));
                    footer_spans.push(ratatui::text::Span::styled(
                        " Cancel",
                        Style::default().fg(crate::colors::text_dim()),
                    ));
                } else if pending_visible {
                    footer_spans.push(ratatui::text::Span::styled(
                        "Enter",
                        Style::default().fg(crate::colors::primary()),
                    ));
                    footer_spans.push(ratatui::text::Span::styled(
                        " Run",
                        Style::default().fg(crate::colors::text_dim()),
                    ));
                }
                if footer_height > 1 {
                    let spacer_area = Rect {
                        x: footer_area.x,
                        y: footer_area.y,
                        width: footer_area.width,
                        height: footer_area.height.saturating_sub(1),
                    };
                    fill_rect(buf, spacer_area, Some(' '), inner_bg);
                }

                let instructions_area = Rect {
                    x: footer_area.x,
                    y: footer_area.y.saturating_add(footer_area.height.saturating_sub(1)),
                    width: footer_area.width,
                    height: 1,
                };

                Paragraph::new(RtText::from(vec![ratatui::text::Line::from(footer_spans)]))
                    .wrap(ratatui::widgets::Wrap { trim: true })
                    .alignment(ratatui::layout::Alignment::Left)
                    .render(instructions_area, buf);
            }
        }

        if self.terminal.overlay().is_none() && self.browser_overlay_visible {
            self.render_browser_overlay(area, history_area, bottom_pane_area, buf);
            return;
        }

        if self.terminal.overlay().is_none() && self.agents_terminal.active {
            self.render_agents_terminal_overlay(area, history_area, bottom_pane_area, buf);
        }

        // Terminal overlay takes precedence over other overlays

        // Welcome animation is kept as a normal cell in history; no overlay.

        // The welcome animation is no longer rendered as an overlay.

        let terminal_overlay_none = self.terminal.overlay().is_none();
        let agents_terminal_active = self.agents_terminal.active;
        if terminal_overlay_none && !agents_terminal_active {
            if let Some(overlay) = self.settings.overlay.as_ref() {
                self.render_settings_overlay(area, history_area, buf, overlay);
            } else if let Some(overlay) = &self.diffs.overlay {
                // Global scrim: dim the whole background to draw focus to the viewer
                // We intentionally do this across the entire widget area rather than just the
                // history area so the viewer stands out even with browser HUD or status bars.
                let scrim_bg = Style::default()
                    .bg(crate::colors::overlay_scrim())
                    .fg(crate::colors::text_dim());
                let _perf_scrim_start = if self.perf_state.enabled {
                    Some(std::time::Instant::now())
                } else {
                    None
                };
                fill_rect(buf, area, None, scrim_bg);
                if let Some(t0) = _perf_scrim_start {
                    let dt = t0.elapsed().as_nanos();
                    let mut p = self.perf_state.stats.borrow_mut();
                    p.ns_overlay_scrim = p.ns_overlay_scrim.saturating_add(dt);
                    let cells = (area.width as u64) * (area.height as u64);
                    p.cells_overlay_scrim = p.cells_overlay_scrim.saturating_add(cells);
                }
                // Match the horizontal padding used by status bar and input
                let padding = 1u16;
                let area = Rect {
                    x: history_area.x + padding,
                    y: history_area.y,
                    width: history_area.width.saturating_sub(padding * 2),
                    height: history_area.height,
                };

                // Clear and repaint the overlay area with theme scrim background
                Clear.render(area, buf);
                let bg_style = Style::default().bg(crate::colors::overlay_scrim());
                let _perf_overlay_area_bg_start = if self.perf_state.enabled {
                    Some(std::time::Instant::now())
                } else {
                    None
                };
                fill_rect(buf, area, None, bg_style);
                if let Some(t0) = _perf_overlay_area_bg_start {
                    let dt = t0.elapsed().as_nanos();
                    let mut p = self.perf_state.stats.borrow_mut();
                    p.ns_overlay_body_bg = p.ns_overlay_body_bg.saturating_add(dt);
                    let cells = (area.width as u64) * (area.height as u64);
                    p.cells_overlay_body_bg = p.cells_overlay_body_bg.saturating_add(cells);
                }

                // Build a styled title: keys/icons in normal text color; descriptors and dividers dim
                let t_dim = Style::default().fg(crate::colors::text_dim());
                let t_fg = Style::default().fg(crate::colors::text());
                let has_tabs = overlay.tabs.len() > 1;
                let mut title_spans: Vec<ratatui::text::Span<'static>> = vec![
                    ratatui::text::Span::styled(" ", t_dim),
                    ratatui::text::Span::styled("Diff viewer", t_fg),
                ];
                if has_tabs {
                    title_spans.extend_from_slice(&[
                        ratatui::text::Span::styled(" ——— ", t_dim),
                        ratatui::text::Span::styled("◂ ▸", t_fg),
                        ratatui::text::Span::styled(" change tabs ", t_dim),
                    ]);
                }
                title_spans.extend_from_slice(&[
                    ratatui::text::Span::styled("——— ", t_dim),
                    ratatui::text::Span::styled("e", t_fg),
                    ratatui::text::Span::styled(" explain ", t_dim),
                    ratatui::text::Span::styled("——— ", t_dim),
                    ratatui::text::Span::styled("u", t_fg),
                    ratatui::text::Span::styled(" undo ", t_dim),
                    ratatui::text::Span::styled("——— ", t_dim),
                    ratatui::text::Span::styled("Esc", t_fg),
                    ratatui::text::Span::styled(" close ", t_dim),
                ]);
                let block = Block::default()
                    .borders(Borders::ALL)
                    .title(ratatui::text::Line::from(title_spans))
                    // Use normal background for the window itself so it contrasts against the
                    // dimmed scrim behind
                    .style(Style::default().bg(crate::colors::background()))
                    .border_style(
                        Style::default()
                            .fg(crate::colors::border())
                            .bg(crate::colors::background()),
                    );
                let inner = block.inner(area);
                block.render(area, buf);

                // Paint inner content background as the normal theme background
                let inner_bg = Style::default().bg(crate::colors::background());
                let _perf_overlay_inner_bg_start = if self.perf_state.enabled {
                    Some(std::time::Instant::now())
                } else {
                    None
                };
                for y in inner.y..inner.y + inner.height {
                    for x in inner.x..inner.x + inner.width {
                        buf[(x, y)].set_style(inner_bg);
                    }
                }
                if let Some(t0) = _perf_overlay_inner_bg_start {
                    let dt = t0.elapsed().as_nanos();
                    let mut p = self.perf_state.stats.borrow_mut();
                    p.ns_overlay_body_bg = p.ns_overlay_body_bg.saturating_add(dt);
                    let cells = (inner.width as u64) * (inner.height as u64);
                    p.cells_overlay_body_bg = p.cells_overlay_body_bg.saturating_add(cells);
                }

                // Split into header tabs and body/footer
                // Add one cell padding around the entire inside of the window
                let padded_inner = inner.inner(ratatui::layout::Margin::new(1, 1));
                let [tabs_area, body_area] = if has_tabs {
                    Layout::vertical([Constraint::Length(2), Constraint::Fill(1)])
                        .areas(padded_inner)
                } else {
                    // Keep a small header row to show file path and counts
                    let [t, b] = Layout::vertical([Constraint::Length(2), Constraint::Fill(1)])
                        .areas(padded_inner);
                    [t, b]
                };

                // Render tabs only if we have more than one file
                if has_tabs {
                    let labels: Vec<String> = overlay
                        .tabs
                        .iter()
                        .map(|(t, _)| format!("  {t}  "))
                        .collect();
                    let mut constraints: Vec<Constraint> = Vec::new();
                    let mut total: u16 = 0;
                    for label in &labels {
                        let w = (label.chars().count() as u16)
                            .min(tabs_area.width.saturating_sub(total));
                        constraints.push(Constraint::Length(w));
                        total = total.saturating_add(w);
                        if total >= tabs_area.width.saturating_sub(4) {
                            break;
                        }
                    }
                    constraints.push(Constraint::Fill(1));
                    let chunks = Layout::horizontal(constraints).split(tabs_area);
                    // Draw a light bottom border across the entire tabs strip
                    let tabs_bottom_rule = Block::default()
                        .borders(Borders::BOTTOM)
                        .border_style(Style::default().fg(crate::colors::border()));
                    tabs_bottom_rule.render(tabs_area, buf);
                    for i in 0..labels.len() {
                        // last chunk is filler; guard below
                        if i >= chunks.len().saturating_sub(1) {
                            break;
                        }
                        let rect = chunks[i];
                        if rect.width == 0 {
                            continue;
                        }
                        let selected = i == overlay.selected;

                        // Both selected and unselected tabs use the normal background
                        let tab_bg = crate::colors::background();
                        let bg_style = Style::default().bg(tab_bg);
                        for y in rect.y..rect.y + rect.height {
                            for x in rect.x..rect.x + rect.width {
                                buf[(x, y)].set_style(bg_style);
                            }
                        }

                        // Render label at the top line, with padding
                        let label_rect = Rect {
                            x: rect.x + 1,
                            y: rect.y,
                            width: rect.width.saturating_sub(2),
                            height: 1,
                        };
                        let label_style = if selected {
                            Style::default()
                                .fg(crate::colors::text())
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(crate::colors::text_dim())
                        };
                        let line = ratatui::text::Line::from(ratatui::text::Span::styled(
                            labels[i].clone(),
                            label_style,
                        ));
                        Paragraph::new(RtText::from(vec![line]))
                            .wrap(ratatui::widgets::Wrap { trim: true })
                            .render(label_rect, buf);
                        // Selected tab: thin underline using text_bright under the label width
                        if selected {
                            let label_len = labels[i].chars().count() as u16;
                            let accent_w = label_len.min(rect.width.saturating_sub(2)).max(1);
                            let accent_rect = Rect {
                                x: label_rect.x,
                                y: rect.y + rect.height.saturating_sub(1),
                                width: accent_w,
                                height: 1,
                            };
                            let underline = Block::default()
                                .borders(Borders::BOTTOM)
                                .border_style(Style::default().fg(crate::colors::text_bright()));
                            underline.render(accent_rect, buf);
                        }
                    }
                } else {
                    // Single-file header: show full path with (+adds -dels)
                    if let Some((label, _)) = overlay.tabs.get(overlay.selected) {
                        let header_line = ratatui::text::Line::from(ratatui::text::Span::styled(
                            label.clone(),
                            Style::default()
                                .fg(crate::colors::text())
                                .add_modifier(Modifier::BOLD),
                        ));
                        let para = Paragraph::new(RtText::from(vec![header_line]))
                            .wrap(ratatui::widgets::Wrap { trim: true });
                        ratatui::widgets::Widget::render(para, tabs_area, buf);
                    }
                }

                // Render selected tab with vertical scroll and highlight current diff block
                if let Some((_, blocks)) = overlay.tabs.get(overlay.selected) {
                    // Flatten blocks into lines and record block start indices
                    let mut all_lines: Vec<ratatui::text::Line<'static>> = Vec::new();
                    let mut block_starts: Vec<(usize, usize)> = Vec::new(); // (start_index, len)
                    for b in blocks {
                        let start = all_lines.len();
                        block_starts.push((start, b.lines.len()));
                        all_lines.extend(b.lines.clone());
                    }

                    let raw_skip = overlay
                        .scroll_offsets
                        .get(overlay.selected)
                        .copied()
                        .unwrap_or(0) as usize;
                    let visible_rows = body_area.height as usize;
                    // Cache visible rows so key handler can clamp
                    self.diffs.body_visible_rows.set(body_area.height);
                    let max_off = all_lines.len().saturating_sub(visible_rows.max(1));
                    let skip = raw_skip.min(max_off);
                    let body_inner = body_area;
                    let visible_rows = body_inner.height as usize;

                    // Collect visible slice
                    let end = (skip + visible_rows).min(all_lines.len());
                    let visible = if skip < all_lines.len() {
                        &all_lines[skip..end]
                    } else {
                        &[]
                    };
                    // Fill body background with a slightly lighter paper-like background
                    let bg = crate::colors::background();
                    let paper_color = crate::colors::mix_toward(bg, ratatui::style::Color::White, 0.06);
                    let body_bg = Style::default().bg(paper_color);
                    let _perf_overlay_body_bg2 = if self.perf_state.enabled {
                        Some(std::time::Instant::now())
                    } else {
                        None
                    };
                    for y in body_inner.y..body_inner.y + body_inner.height {
                        for x in body_inner.x..body_inner.x + body_inner.width {
                            buf[(x, y)].set_style(body_bg);
                        }
                    }
                    if let Some(t0) = _perf_overlay_body_bg2 {
                        let dt = t0.elapsed().as_nanos();
                        let mut p = self.perf_state.stats.borrow_mut();
                        p.ns_overlay_body_bg = p.ns_overlay_body_bg.saturating_add(dt);
                        let cells = (body_inner.width as u64) * (body_inner.height as u64);
                        p.cells_overlay_body_bg = p.cells_overlay_body_bg.saturating_add(cells);
                    }
                    let paragraph = Paragraph::new(RtText::from(visible.to_vec()))
                        .wrap(ratatui::widgets::Wrap { trim: false });
                    ratatui::widgets::Widget::render(paragraph, body_inner, buf);

                    // No explicit current-block highlight for a cleaner look

                    // Render confirmation dialog if active
                    if self.diffs.confirm.is_some() {
                        // Centered small box
                        let w = (body_inner.width as i16 - 10).max(20) as u16;
                        let h = 5u16;
                        let x = body_inner.x + (body_inner.width.saturating_sub(w)) / 2;
                        let y = body_inner.y + (body_inner.height.saturating_sub(h)) / 2;
                        let dialog = Rect {
                            x,
                            y,
                            width: w,
                            height: h,
                        };
                        Clear.render(dialog, buf);
                        let dlg_block = Block::default()
                            .borders(Borders::ALL)
                            .title("Confirm Undo")
                            .style(
                                Style::default()
                                    .bg(crate::colors::background())
                                    .fg(crate::colors::text()),
                            )
                            .border_style(Style::default().fg(crate::colors::border()));
                        let dlg_inner = dlg_block.inner(dialog);
                        dlg_block.render(dialog, buf);
                        // Fill dialog inner area with theme background for consistent look
                        let dlg_bg = Style::default().bg(crate::colors::background());
                        for y in dlg_inner.y..dlg_inner.y + dlg_inner.height {
                            for x in dlg_inner.x..dlg_inner.x + dlg_inner.width {
                                buf[(x, y)].set_style(dlg_bg);
                            }
                        }
                        let lines = vec![
                            ratatui::text::Line::from("Are you sure you want to undo this diff?"),
                            ratatui::text::Line::from(
                                "Press Enter to confirm • Esc to cancel".to_string().dim(),
                            ),
                        ];
                        let para = Paragraph::new(RtText::from(lines))
                            .style(
                                Style::default()
                                    .bg(crate::colors::background())
                                    .fg(crate::colors::text()),
                            )
                            .wrap(ratatui::widgets::Wrap { trim: true });
                        ratatui::widgets::Widget::render(para, dlg_inner, buf);
                    }
                }
            }

            // Render help overlay (covering the history area) if active
            if self.settings.overlay.is_none()
                && let Some(overlay) = &self.help.overlay {
                    // Global scrim across widget
                    let scrim_bg = Style::default()
                        .bg(crate::colors::overlay_scrim())
                        .fg(crate::colors::text_dim());
                    for y in area.y..area.y + area.height {
                        for x in area.x..area.x + area.width {
                            buf[(x, y)].set_style(scrim_bg);
                        }
                    }
                    let padding = 1u16;
                    let window_area = Rect {
                        x: history_area.x + padding,
                        y: history_area.y,
                        width: history_area.width.saturating_sub(padding * 2),
                        height: history_area.height,
                    };
                    Clear.render(window_area, buf);
                    let block = Block::default()
                        .borders(Borders::ALL)
                        .title(ratatui::text::Line::from(vec![
                            ratatui::text::Span::styled(
                                " ",
                                Style::default().fg(crate::colors::text_dim()),
                            ),
                            ratatui::text::Span::styled(
                                "Guide",
                                Style::default().fg(crate::colors::text()),
                            ),
                            ratatui::text::Span::styled(
                                " ——— ",
                                Style::default().fg(crate::colors::text_dim()),
                            ),
                            ratatui::text::Span::styled(
                                "Esc",
                                Style::default().fg(crate::colors::text()),
                            ),
                            ratatui::text::Span::styled(
                                " close ",
                                Style::default().fg(crate::colors::text_dim()),
                            ),
                        ]))
                        .style(Style::default().bg(crate::colors::background()))
                        .border_style(
                            Style::default()
                                .fg(crate::colors::border())
                                .bg(crate::colors::background()),
                        );
                    let inner = block.inner(window_area);
                    block.render(window_area, buf);

                    // Paint inner bg
                    let inner_bg = Style::default().bg(crate::colors::background());
                    for y in inner.y..inner.y + inner.height {
                        for x in inner.x..inner.x + inner.width {
                            buf[(x, y)].set_style(inner_bg);
                        }
                    }

                    // Body area with one cell padding
                    let body = inner.inner(ratatui::layout::Margin::new(1, 1));

                    // Compute visible slice
                    let visible_rows = body.height as usize;
                    self.help.body_visible_rows.set(body.height);
                    let max_off = overlay.lines.len().saturating_sub(visible_rows.max(1));
                    let skip = (overlay.scroll as usize).min(max_off);
                    let end = (skip + visible_rows).min(overlay.lines.len());
                    let visible = if skip < overlay.lines.len() {
                        &overlay.lines[skip..end]
                    } else {
                        &[]
                    };
                    let paragraph = Paragraph::new(RtText::from(visible.to_vec()))
                        .wrap(ratatui::widgets::Wrap { trim: false });
                    ratatui::widgets::Widget::render(paragraph, body, buf);
                }
        }
        // Finalize widget render timing
        if let Some(t0) = _perf_widget_start {
            let dt = t0.elapsed().as_nanos();
            let mut p = self.perf_state.stats.borrow_mut();
            p.ns_widget_render_total = p.ns_widget_render_total.saturating_add(dt);
        }
    }
}

// Coalesce adjacent Read entries of the same file with contiguous ranges in a rendered lines vector.
// Expects the vector to contain a header line at index 0 (e.g., "Read"). Modifies in place.
#[allow(dead_code)]
fn coalesce_read_ranges_in_lines(lines: &mut Vec<ratatui::text::Line<'static>>) {
    use ratatui::style::Modifier;
    use ratatui::style::Style;
    use ratatui::text::Line;
    use ratatui::text::Span;

    if lines.len() <= 1 {
        return;
    }

    // Helper to parse a content line into (filename, start, end, prefix)
    fn parse_read_line(line: &Line<'_>) -> Option<(String, u32, u32, String)> {
        if line.spans.is_empty() {
            return None;
        }
        let prefix = line.spans[0].content.to_string();
        if !(prefix == "└ " || prefix == "  ") {
            return None;
        }
        let rest: String = line
            .spans
            .iter()
            .skip(1)
            .map(|s| s.content.as_ref())
            .collect();
        if let Some(idx) = rest.rfind(" (lines ") {
            let fname = rest[..idx].to_string();
            let tail = &rest[idx + 1..];
            if tail.starts_with("(lines ") && tail.ends_with(")") {
                let inner = &tail[7..tail.len() - 1];
                if let Some((s1, s2)) = inner.split_once(" to ")
                    && let (Ok(start), Ok(end)) =
                        (s1.trim().parse::<u32>(), s2.trim().parse::<u32>())
                    {
                        return Some((fname, start, end, prefix));
                    }
            }
        }
        None
    }

    // Merge overlapping or touching ranges for the same file, regardless of adjacency.
    let mut i: usize = 0; // works for vectors with or without a header line
    while i < lines.len() {
        let Some((fname_a, mut a1, mut a2, prefix_a)) = parse_read_line(&lines[i]) else {
            i += 1;
            continue;
        };
        let mut k = i + 1;
        while k < lines.len() {
            if let Some((fname_b, b1, b2, _prefix_b)) = parse_read_line(&lines[k])
                && fname_b == fname_a {
                    let touch_or_overlap = b1 <= a2.saturating_add(1) && b2.saturating_add(1) >= a1;
                    if touch_or_overlap {
                        a1 = a1.min(b1);
                        a2 = a2.max(b2);
                        let new_spans: Vec<Span<'static>> = vec![
                            Span::styled(
                                prefix_a.clone(),
                                Style::default().add_modifier(Modifier::DIM),
                            ),
                            Span::styled(
                                fname_a.clone(),
                                Style::default().fg(crate::colors::text()),
                            ),
                            Span::styled(
                                format!(" (lines {a1} to {a2})"),
                                Style::default().fg(crate::colors::text_dim()),
                            ),
                        ];
                        lines[i] = Line::from(new_spans);
                        lines.remove(k);
                        continue;
                    }
                }
            k += 1;
        }
        i += 1;
    }
}

struct CommandDisplayLine {
    text: String,
    start: usize,
    end: usize,
}

fn wrap_pending_command_lines(input: &str, width: usize) -> Vec<CommandDisplayLine> {
    if width == 0 {
        return vec![CommandDisplayLine {
            text: String::new(),
            start: 0,
            end: input.len(),
        }];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    let mut current_start = 0usize;

    for (byte_idx, grapheme) in input.grapheme_indices(true) {
        let g_width = UnicodeWidthStr::width(grapheme);
        if current_width + g_width > width && !current.is_empty() {
            lines.push(CommandDisplayLine {
                text: current,
                start: current_start,
                end: byte_idx,
            });
            current = String::new();
            current_width = 0;
            current_start = byte_idx;
        }
        current.push_str(grapheme);
        current_width += g_width;
    }

    let end = input.len();
    lines.push(CommandDisplayLine {
        text: current,
        start: current_start,
        end,
    });

    if lines.is_empty() {
        lines.push(CommandDisplayLine {
            text: String::new(),
            start: 0,
            end: 0,
        });
    }

    lines
}

fn pending_command_box_lines(
    pending: &PendingCommand,
    width: u16,
) -> Option<(Vec<RtLine<'static>>, u16)> {
    if width <= 4 {
        return None;
    }
    let inner_width = width.saturating_sub(2);
    if inner_width <= 4 {
        return None;
    }

    let padded_width = inner_width.saturating_sub(2).max(1) as usize;
    let command_width = inner_width.saturating_sub(4).max(1) as usize;

    const INSTRUCTION_TEXT: &str =
        "Press Enter to run this command. Press Esc to cancel.";
    let instruction_segments = wrap(INSTRUCTION_TEXT, padded_width);
    let instruction_style = Style::default().fg(crate::colors::text_dim());
    let mut lines: Vec<RtLine<'static>> = instruction_segments
        .into_iter()
        .map(|segment| {
            ratatui::text::Line::from(vec![
                ratatui::text::Span::raw(" "),
                ratatui::text::Span::styled(segment.into_owned(), instruction_style),
                ratatui::text::Span::raw(" "),
            ])
        })
        .collect();

    let command_lines = wrap_pending_command_lines(pending.input(), command_width);
    let cursor_line_idx = command_line_index_for_cursor(&command_lines, pending.cursor());
    let prefix_style = Style::default().fg(crate::colors::primary());
    let text_style = Style::default().fg(crate::colors::text());
    let cursor_style = Style::default()
        .bg(crate::colors::primary())
        .fg(crate::colors::background());

    if !lines.is_empty() {
        lines.push(ratatui::text::Line::from(vec![ratatui::text::Span::raw(String::new())]));
    }

    for (idx, line) in command_lines.iter().enumerate() {
        let mut spans = Vec::new();
        spans.push(ratatui::text::Span::raw(" "));
        if idx == 0 {
            spans.push(ratatui::text::Span::styled("$ ", prefix_style));
        } else {
            spans.push(ratatui::text::Span::raw("  "));
        }

        if idx == cursor_line_idx {
            let cursor_offset = pending.cursor().saturating_sub(line.start);
            let cursor_offset = cursor_offset.min(line.text.len());
            let (before, cursor_span, after) = split_line_for_cursor(&line.text, cursor_offset);
            if !before.is_empty() {
                spans.push(ratatui::text::Span::styled(before, text_style));
            }
            match cursor_span {
                Some(token) => spans.push(ratatui::text::Span::styled(token, cursor_style)),
                None => spans.push(ratatui::text::Span::styled(" ", cursor_style)),
            }
            if let Some(after_text) = after
                && !after_text.is_empty() {
                    spans.push(ratatui::text::Span::styled(after_text, text_style));
                }
        } else {
            spans.push(ratatui::text::Span::styled(line.text.clone(), text_style));
        }

        spans.push(ratatui::text::Span::raw(" "));
        lines.push(ratatui::text::Line::from(spans));
    }

    let height = (lines.len() as u16).saturating_add(2).max(3);
    Some((lines, height))
}

fn command_line_index_for_cursor(lines: &[CommandDisplayLine], cursor: usize) -> usize {
    if lines.is_empty() {
        return 0;
    }
    for (idx, line) in lines.iter().enumerate() {
        if cursor < line.end {
            return idx;
        }
        if cursor == line.end {
            return (idx + 1).min(lines.len().saturating_sub(1));
        }
    }
    lines.len().saturating_sub(1)
}

fn split_line_for_cursor(text: &str, cursor_offset: usize) -> (String, Option<String>, Option<String>) {
    if cursor_offset >= text.len() {
        return (text.to_string(), None, None);
    }

    let (before, remainder) = text.split_at(cursor_offset);
    let mut graphemes = remainder.graphemes(true);
    if let Some(first) = graphemes.next() {
        let after = graphemes.collect::<String>();
        (
            before.to_string(),
            Some(first.to_string()),
            if after.is_empty() { None } else { Some(after) },
        )
    } else {
        (before.to_string(), None, None)
    }
}

fn render_text_box(
    area: Rect,
    title: &str,
    border_color: ratatui::style::Color,
    lines: Vec<RtLine<'static>>,
    buf: &mut Buffer,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(crate::colors::background()))
        .border_style(Style::default().fg(border_color))
        .title(ratatui::text::Span::styled(
            title.to_string(),
            Style::default().fg(border_color),
        ));
    block.render(area, buf);

    let inner = area.inner(ratatui::layout::Margin::new(1, 1));
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let inner_bg = Style::default().bg(crate::colors::background());
    for y in inner.y..inner.y + inner.height {
        for x in inner.x..inner.x + inner.width {
            buf[(x, y)].set_style(inner_bg);
        }
    }

    Paragraph::new(RtText::from(lines))
        .wrap(ratatui::widgets::Wrap { trim: false })
        .render(inner, buf);
}
