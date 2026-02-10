use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::bottom_pane::{
    settings_panel::{render_panel, PanelFrameStyle},
    SettingsSection,
};
use crate::live_wrap::take_prefix_by_width;
use crate::util::buffer::fill_rect;

use super::types::{LABEL_COLUMN_WIDTH, SettingsHelpOverlay};
use super::{SettingsContent, SettingsOverlayView};

impl SettingsOverlayView {
    pub(crate) fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let block = Block::default()
            .title(self.block_title_line())
            .title_alignment(Alignment::Left)
            .borders(Borders::ALL)
            .style(Style::default().bg(crate::colors::background()))
            .border_style(
                Style::default()
                    .fg(crate::colors::border())
                    .bg(crate::colors::background()),
            );
        let inner = block.inner(area);
        block.render(area, buf);

        let bg = Style::default().bg(crate::colors::background());
        for y in inner.y..inner.y.saturating_add(inner.height) {
            for x in inner.x..inner.x.saturating_add(inner.width) {
                buf[(x, y)].set_style(bg);
            }
        }

        let content = inner.inner(Margin::new(1, 1));
        if content.width == 0 || content.height == 0 {
            return;
        }

        // Store content area for mouse hit testing
        *self.last_content_area.borrow_mut() = content;

        if self.is_menu_active() {
            self.render_overview(content, buf);
        } else {
            self.render_section_layout(content, buf);
        }

        if let Some(help) = &self.help {
            self.render_help_overlay(inner, buf, help);
        }
    }

    fn block_title_line(&self) -> Line<'static> {
        if self.is_menu_active() {
            Line::from(vec![
                Span::styled("Settings", Style::default().fg(crate::colors::text())),
                Span::styled(" ▸ ", Style::default().fg(crate::colors::text_dim())),
                Span::styled("Overview", Style::default().fg(crate::colors::text())),
            ])
        } else {
            Line::from(vec![
                Span::styled("Settings", Style::default().fg(crate::colors::text())),
                Span::styled(" ▸ ", Style::default().fg(crate::colors::text_dim())),
                Span::styled(
                    self.active_section().label(),
                    Style::default().fg(crate::colors::text()),
                ),
            ])
        }
    }

    fn render_overview(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let (list_area, hint_area) = match area.height {
            0 => return,
            1 => (area, None),
            _ => {
                let [list, hint] =
                    Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
                (list, Some(hint))
            }
        };

        self.render_overview_list(list_area, buf);
        if let Some(hint_area) = hint_area {
            self.render_footer_hints(hint_area, buf);
        }
    }

    fn render_overview_list(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        fill_rect(
            buf,
            area,
            Some(' '),
            Style::default().bg(crate::colors::background()),
        );

        if self.overview_rows.is_empty() {
            let line = Line::from(vec![Span::styled(
                "No settings available.",
                Style::default().fg(crate::colors::text_dim()),
            )]);
            Paragraph::new(line)
                .style(Style::default().bg(crate::colors::background()))
                .render(area, buf);
            return;
        }

        let active_section = self.active_section();
        let content_width = area.width as usize;
        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut selected_range: Option<(usize, usize)> = None;

        for (idx, row) in self.overview_rows.iter().enumerate() {
            let is_active = row.section == active_section;
            let indicator = if is_active { "›" } else { " " };

            let row_start = lines.len();

            if row.section == SettingsSection::Limits && !lines.is_empty() {
                lines.push(Line::from(""));
                let dash_count = content_width.saturating_sub(2);
                if dash_count > 0 {
                    lines.push(Line::from(vec![Span::styled(
                        format!("  {}", "─".repeat(dash_count)),
                        Style::default().fg(crate::colors::border_dim()),
                    )]));
                    lines.push(Line::from(""));
                }
            }

            let label_style = if is_active {
                Style::default()
                    .fg(crate::colors::text_bright())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(crate::colors::text_mid())
            };

            let label_text = format!("{:<width$}", row.section.label(), width = LABEL_COLUMN_WIDTH);

            let summary_src = row.summary.as_deref().unwrap_or("—");
            let base_width = 1 + 1 + LABEL_COLUMN_WIDTH;
            let available_tail = content_width.saturating_sub(base_width);

            let mut summary_line = Line::from(vec![
                Span::styled(indicator.to_string(), Style::default().fg(crate::colors::text())),
                Span::raw(" "),
                Span::styled(label_text, label_style),
            ]);

            if available_tail > 0 {
                summary_line.spans.push(Span::raw(" "));
                let summary_budget = available_tail.saturating_sub(1);

                if summary_budget > 0 {
                    let summary_trimmed = self.trim_with_ellipsis(summary_src, summary_budget);
                    if !summary_trimmed.is_empty() {
                        self.push_summary_spans(&mut summary_line, &summary_trimmed);
                    }
                }
            }

            if is_active {
                summary_line = summary_line.style(
                    Style::default()
                        .bg(crate::colors::selection())
                        .fg(crate::colors::text()),
                );
            }
            lines.push(summary_line);

            let info_text = row.section.help_line();
            let info_trimmed = self.trim_with_ellipsis(info_text, content_width.saturating_sub(8));
            let info_style = Style::default().fg(crate::colors::text_dim());
            let mut info_line = Line::from(vec![
                Span::raw("  "),
                Span::styled("└ ", info_style),
                Span::styled(info_trimmed, info_style),
            ]);
            if is_active {
                info_line = info_line.style(
                    Style::default()
                        .bg(crate::colors::selection())
                        .fg(crate::colors::text()),
                );
            }
            lines.push(info_line);

            if is_active {
                let row_end = lines.len().saturating_sub(1);
                selected_range = Some((row_start, row_end));
            }

            if idx != self.overview_rows.len().saturating_sub(1) {
                lines.push(Line::from(""));
                if matches!(row.section, SettingsSection::Updates) {
                    let dash_count = content_width.saturating_sub(2);
                    if dash_count > 0 {
                        lines.push(Line::from(vec![Span::styled(
                            format!("  {}", "─".repeat(dash_count)),
                            Style::default().fg(crate::colors::border_dim()),
                        )]));
                        lines.push(Line::from(""));
                    }
                }
            }
        }

        let total_lines = lines.len();
        let visible_lines = area.height as usize;
        let mut scroll = 0usize;
        if visible_lines > 0 && total_lines > visible_lines {
            if let Some((start, end)) = selected_range {
                let max_scroll = total_lines.saturating_sub(visible_lines);
                let mut candidate = end.saturating_add(1).saturating_sub(visible_lines);
                if candidate > max_scroll {
                    candidate = max_scroll;
                }
                if start < candidate {
                    candidate = start.min(max_scroll);
                }
                if end >= candidate.saturating_add(visible_lines) {
                    candidate = end
                        .saturating_add(1)
                        .saturating_sub(visible_lines)
                        .min(max_scroll);
                }
                scroll = candidate;
            } else {
                scroll = total_lines.saturating_sub(visible_lines);
            }
        }
        let scroll = scroll.min(u16::MAX as usize) as u16;

        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .style(Style::default().bg(crate::colors::background()))
            .scroll((scroll, 0))
            .render(area, buf);
    }

    fn trim_with_ellipsis(&self, text: &str, max_width: usize) -> String {
        if max_width == 0 || text.is_empty() {
            return String::new();
        }
        if UnicodeWidthStr::width(text) <= max_width {
            return text.to_string();
        }
        if max_width <= 3 {
            return "...".chars().take(max_width).collect();
        }
        let keep = max_width.saturating_sub(3);
        let (prefix, _, _) = take_prefix_by_width(text, keep);
        let mut result = prefix;
        result.push_str("...");
        result
    }

    fn push_summary_spans(&self, line: &mut Line<'static>, summary: &str) {
        let label_style = Style::default().fg(crate::colors::text_mid());
        let dim_style = Style::default().fg(crate::colors::text_dim());
        let mut first = true;
        for raw_segment in summary.split(" · ") {
            let segment = raw_segment.trim();
            if segment.is_empty() {
                continue;
            }
            if !first {
                line.spans.push(Span::styled(" · ".to_string(), dim_style));
            }
            first = false;

            if let Some((label, value)) = segment.split_once(':') {
                let label_trim = label.trim_end();
                let value_trim = value.trim_start();
                line.spans
                    .push(Span::styled(format!("{label_trim}:"), label_style));
                if !value_trim.is_empty() {
                    line.spans.push(Span::styled(" ".to_string(), dim_style));
                    let value_style = self.summary_value_style(value_trim);
                    line.spans
                        .push(Span::styled(value_trim.to_string(), value_style));
                }
            } else {
                let value_style = self.summary_value_style(segment);
                line.spans
                    .push(Span::styled(segment.to_string(), value_style));
            }
        }
    }

    fn summary_value_style(&self, value: &str) -> Style {
        let trimmed = value.trim();
        let normalized = trimmed
            .trim_end_matches(['.', '!', ','])
            .to_ascii_lowercase();
        if matches!(normalized.as_str(), "on" | "enabled" | "yes") {
            Style::default().fg(crate::colors::success())
        } else if matches!(normalized.as_str(), "off" | "disabled" | "no") {
            Style::default().fg(crate::colors::error())
        } else {
            Style::default().fg(crate::colors::info())
        }
    }

    fn render_footer_hints(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let line = Line::from(vec![
            Span::styled("↑ ↓", Style::default().fg(crate::colors::text())),
            Span::styled(" Move    ", Style::default().fg(crate::colors::text_dim())),
            Span::styled("Enter", Style::default().fg(crate::colors::text())),
            Span::styled(" Open    ", Style::default().fg(crate::colors::text_dim())),
            Span::styled("Esc", Style::default().fg(crate::colors::text())),
            Span::styled(" Close    ", Style::default().fg(crate::colors::text_dim())),
            Span::styled("?", Style::default().fg(crate::colors::text())),
            Span::styled(" Help", Style::default().fg(crate::colors::text_dim())),
        ]);

        Paragraph::new(line)
            .style(Style::default().bg(crate::colors::background()))
            .alignment(Alignment::Left)
            .render(area, buf);
    }

    fn render_section_layout(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let (main_area, hint_area) = if area.height <= 1 {
            (area, None)
        } else {
            let [main, hint] =
                Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
            (main, Some(hint))
        };

        self.render_section_main(main_area, buf);
        if let Some(hint_area) = hint_area {
            self.render_footer_hints(hint_area, buf);
        }
    }

    fn render_section_main(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let [sidebar, main] =
            Layout::horizontal([Constraint::Length(22), Constraint::Fill(1)]).areas(area);
        *self.last_sidebar_area.borrow_mut() = sidebar;

        self.render_sidebar(sidebar, buf);
        self.render_section_panel(main, buf);
    }

    fn render_section_panel(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let title = Self::section_panel_title(self.active_section());
        render_panel(
            area,
            buf,
            title,
            PanelFrameStyle::overlay().with_margin(Margin::new(1, 1)),
            |inner, buf| {
                self.render_content(inner, buf);
                self.strip_child_border(inner, buf);
            },
        );
    }

    fn section_panel_title(section: SettingsSection) -> &'static str {
        match section {
            SettingsSection::Model => "Select Model & Reasoning",
            SettingsSection::Theme => "Theme Settings",
            SettingsSection::Planning => "Planning Settings",
            SettingsSection::Updates => "Upgrade",
            SettingsSection::Accounts => "Account Switching",
            SettingsSection::Agents => "Agents",
            SettingsSection::Skills => "Skills",
            SettingsSection::AutoDrive => "Auto Drive Settings",
            SettingsSection::Review => "Review Settings",
            SettingsSection::Validation => "Validation Settings",
            SettingsSection::Limits => "Rate Limits",
            SettingsSection::Chrome => "Chrome Launch Options",
            SettingsSection::Notifications => "Notifications",
            SettingsSection::Mcp => "MCP Servers",
            SettingsSection::Prompts => "Custom Prompts",
        }
    }

    fn strip_child_border(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let background = Style::default()
            .bg(crate::colors::background())
            .fg(crate::colors::text());
        let end_x = area.x + area.width - 1;
        let end_y = area.y + area.height - 1;

        let top_left_symbol = buf[(area.x, area.y)].symbol();
        let top_right_symbol = buf[(end_x, area.y)].symbol();
        let bottom_left_symbol = buf[(area.x, end_y)].symbol();
        let bottom_right_symbol = buf[(end_x, end_y)].symbol();

        let top_has_corners =
            Self::is_corner_symbol(top_left_symbol) && Self::is_corner_symbol(top_right_symbol);
        let bottom_has_corners = Self::is_corner_symbol(bottom_left_symbol)
            && Self::is_corner_symbol(bottom_right_symbol);

        let top_is_frame = top_has_corners
            && (area.x..=end_x).all(|x| {
                let symbol = buf[(x, area.y)].symbol();
                Self::is_border_symbol(symbol) || Self::is_corner_symbol(symbol)
            });
        let bottom_is_frame = if area.height > 1 {
            Some(bottom_has_corners
                && (area.x..=end_x).all(|x| {
                    let symbol = buf[(x, end_y)].symbol();
                    Self::is_border_symbol(symbol) || Self::is_corner_symbol(symbol)
                }))
        } else {
            None
        };

        let left_has_corners =
            Self::is_corner_symbol(top_left_symbol) && Self::is_corner_symbol(bottom_left_symbol);
        let right_has_corners =
            Self::is_corner_symbol(top_right_symbol) && Self::is_corner_symbol(bottom_right_symbol);

        let left_is_frame = left_has_corners
            && (area.y..=end_y).all(|y| {
                let symbol = buf[(area.x, y)].symbol();
                Self::is_border_symbol(symbol) || Self::is_corner_symbol(symbol)
            });
        let right_is_frame = if area.width > 1 {
            Some(right_has_corners
                && (area.y..=end_y).all(|y| {
                    let symbol = buf[(end_x, y)].symbol();
                    Self::is_border_symbol(symbol) || Self::is_corner_symbol(symbol)
                }))
        } else {
            None
        };

        if top_is_frame {
            for x in area.x..=end_x {
                let cell = &mut buf[(x, area.y)];
                cell.set_symbol(" ");
                cell.set_style(background);
            }
        }

        if let Some(true) = bottom_is_frame {
            for x in area.x..=end_x {
                let cell = &mut buf[(x, end_y)];
                cell.set_symbol(" ");
                cell.set_style(background);
            }
        }

        if left_is_frame {
            for y in area.y..=end_y {
                let cell = &mut buf[(area.x, y)];
                cell.set_symbol(" ");
                cell.set_style(background);
            }
        }

        if let Some(true) = right_is_frame {
            for y in area.y..=end_y {
                let cell = &mut buf[(end_x, y)];
                cell.set_symbol(" ");
                cell.set_style(background);
            }
        }
    }

    fn is_border_symbol(symbol: &str) -> bool {
        matches!(
            symbol,
            "│" | "┃" | "║" | "╎" | "┆" | "┊" | "┇" | "╏" | "╿"
                | "─" | "━" | "═" | "╼" | "╾" | "┄" | "┈" | "╍"
                | "┬" | "┴" | "├" | "┤" | "┼" | "╞" | "╡" | "╪" | "╫"
        )
    }

    fn is_corner_symbol(symbol: &str) -> bool {
        matches!(symbol, "┌" | "┐" | "└" | "┘" | "╭" | "╮" | "╰" | "╯")
    }

    fn render_help_overlay(&self, area: Rect, buf: &mut Buffer, help: &SettingsHelpOverlay) {
        if area.width < 4 || area.height < 4 {
            return;
        }

        fill_rect(
            buf,
            area,
            None,
            Style::default().bg(crate::colors::overlay_scrim()),
        );

        let content_width = help.lines.iter().map(Line::width).max().unwrap_or(0);
        let content_height = help.lines.len() as u16;

        let max_box_width = area.width.saturating_sub(2);
        let mut box_width = content_width
            .saturating_add(4)
            .min(max_box_width as usize)
            .max(20.min(max_box_width as usize));
        if box_width == 0 {
            box_width = max_box_width as usize;
        }
        let box_width = box_width.min(area.width as usize) as u16;

        let max_box_height = area.height.saturating_sub(2);
        let mut box_height = content_height.saturating_add(2).min(max_box_height);
        if box_height < 4 {
            box_height = max_box_height.min(4);
        }
        if box_height == 0 {
            box_height = area.height;
        }

        let box_x = area.x + (area.width.saturating_sub(box_width)) / 2;
        let box_y = area.y + (area.height.saturating_sub(box_height)) / 2;
        let box_area = Rect::new(box_x, box_y, box_width, box_height);

        fill_rect(
            buf,
            box_area,
            Some(' '),
            Style::default().bg(crate::colors::background()),
        );

        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::colors::border()))
            .style(Style::default().bg(crate::colors::background()))
            .render(box_area, buf);

        let inner = box_area.inner(Margin::new(1, 1));
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        Paragraph::new(help.lines.clone())
            .alignment(Alignment::Left)
            .style(Style::default().bg(crate::colors::background()).fg(crate::colors::text()))
            .wrap(Wrap { trim: true })
            .render(inner, buf);
    }

    fn render_sidebar(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let sections: Vec<SettingsSection> = if self.overview_rows.is_empty() {
            SettingsSection::ALL.to_vec()
        } else {
            self.overview_rows.iter().map(|row| row.section).collect()
        };

        fill_rect(
            buf,
            area,
            Some(' '),
            Style::default().bg(crate::colors::background()),
        );

        if sections.is_empty() {
            return;
        }

        let visible = area.height as usize;
        if visible == 0 {
            return;
        }

        let selected_idx = sections
            .iter()
            .position(|section| *section == self.active_section())
            .unwrap_or(0);

        let total = sections.len();
        let mut start = 0usize;
        if total > visible {
            let half = visible / 2;
            if selected_idx > half {
                start = selected_idx - half;
            }
            if start + visible > total {
                start = total - visible;
            }
        }
        let end = (start + visible).min(total);

        // Get current hover state
        let hovered = *self.hovered_section.borrow();

        let mut lines: Vec<Line<'static>> = Vec::new();
        for idx in start..end {
            let section = sections[idx];
            let is_active = idx == selected_idx;
            let is_hovered = hovered == Some(section) && !is_active;
            let is_first_visible = idx == start;
            let is_last_visible = idx + 1 == end;

            let selection_indicator = if is_active {
                "›"
            } else if is_hovered {
                "▸"
            } else {
                " "
            };
            let overflow_indicator = if is_first_visible && start > 0 {
                "↑"
            } else if is_last_visible && end < total {
                "↓"
            } else {
                " "
            };

            let mut spans: Vec<Span<'static>> = Vec::new();
            spans.push(Span::styled(
                selection_indicator.to_string(),
                Style::default().fg(crate::colors::text()),
            ));
            spans.push(Span::styled(
                overflow_indicator.to_string(),
                Style::default().fg(crate::colors::text_dim()),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                section.label(),
                if is_active {
                    Style::default()
                        .fg(crate::colors::text())
                        .add_modifier(Modifier::BOLD)
                } else if is_hovered {
                    Style::default()
                        .fg(crate::colors::text_bright())
                        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
                } else {
                    Style::default().fg(crate::colors::text_dim())
                },
            ));

            let line = if is_active {
                Line::from(spans).style(
                    Style::default()
                        .bg(crate::colors::selection())
                        .fg(crate::colors::text()),
                )
            } else if is_hovered {
                Line::from(spans).style(
                    Style::default()
                        .bg(crate::colors::border_focused())
                        .fg(crate::colors::text()),
                )
            } else {
                Line::from(spans)
            };
            lines.push(line);
        }

        while lines.len() < visible {
            lines.push(Line::from(" "));
        }

        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .style(Style::default().bg(crate::colors::background()))
            .render(area, buf);
    }

    fn render_content(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Cache the panel inner area for mouse event forwarding
        *self.last_panel_inner_area.borrow_mut() = area;

        match self.active_section() {
            SettingsSection::Model => {
                if let Some(content) = self.model_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Model.placeholder());
            }
            SettingsSection::Planning => {
                if let Some(content) = self.planning_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Planning.placeholder());
            }
            SettingsSection::Theme => {
                if let Some(content) = self.theme_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Theme.placeholder());
            }
            SettingsSection::Updates => {
                if let Some(content) = self.updates_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Updates.placeholder());
            }
            SettingsSection::Agents => {
                if let Some(content) = self.agents_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Agents.placeholder());
            }
            SettingsSection::Accounts => {
                if let Some(content) = self.accounts_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Accounts.placeholder());
            }
            SettingsSection::Prompts => {
                if let Some(content) = self.prompts_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Prompts.placeholder());
            }
            SettingsSection::Skills => {
                if let Some(content) = self.skills_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Skills.placeholder());
            }
            SettingsSection::AutoDrive => {
                if let Some(content) = self.auto_drive_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::AutoDrive.placeholder());
            }
            SettingsSection::Review => {
                if let Some(content) = self.review_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Review.placeholder());
            }
            SettingsSection::Validation => {
                if let Some(content) = self.validation_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Validation.placeholder());
            }
            SettingsSection::Limits => {
                if let Some(content) = self.limits_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Limits.placeholder());
            }
            SettingsSection::Chrome => {
                if let Some(content) = self.chrome_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Chrome.placeholder());
            }
            SettingsSection::Notifications => {
                if let Some(content) = self.notifications_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Notifications.placeholder());
            }
            SettingsSection::Mcp => {
                if let Some(content) = self.mcp_content.as_ref() {
                    content.render(area, buf);
                    return;
                }
                self.render_placeholder(area, buf, SettingsSection::Mcp.placeholder());
            }
        }
    }

    fn render_placeholder(&self, area: Rect, buf: &mut Buffer, text: &'static str) {
        let paragraph = Paragraph::new(text)
            .wrap(ratatui::widgets::Wrap { trim: true })
            .style(Style::default().fg(crate::colors::text_dim()));
        paragraph.render(area, buf);
    }
}
