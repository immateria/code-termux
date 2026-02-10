use super::*;

impl ThemeSelectionView {
    pub(super) fn render_spinner_mode(
        &self,
        body_area: Rect,
        available_height: usize,
        theme: &crate::theme::Theme,
        buf: &mut Buffer,
    ) {
        use std::time::SystemTime;
        use std::time::UNIX_EPOCH;

        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let names = crate::spinner::spinner_names();

        // Include an extra pseudo-row for "Generate your own…".
        let count = names.len() + 1;

        // Reserve two rows (header + spacer).
        let visible = available_height.saturating_sub(2).clamp(1, 9);
        let (start, _vis, _mid) = crate::util::list_window::anchored_window(
            self.selected_spinner_index,
            count,
            visible,
        );
        let end = (start + visible).min(count);

        // Compute fixed column width globally so rows never jump when scrolling.
        let max_frame_len: u16 = crate::spinner::global_max_frame_len() as u16;

        // Render header (left-aligned) and spacer row.
        let header_rect = Rect {
            x: body_area.x,
            y: body_area.y,
            width: body_area.width,
            height: 1,
        };
        let header = Line::from(Span::styled(
            "Overview » Change Spinner",
            Style::default()
                .fg(theme.text_bright)
                .add_modifier(Modifier::BOLD),
        ));
        Paragraph::new(header)
            .alignment(Alignment::Left)
            .render(header_rect, buf);
        if header_rect.y + 1 < body_area.y + body_area.height {
            let spacer = Rect {
                x: body_area.x,
                y: body_area.y + 1,
                width: body_area.width,
                height: 1,
            };
            Paragraph::new(Line::default()).render(spacer, buf);
        }

        for row_idx in 0..(end - start) {
            let index = start + row_idx;
            // Rows start two below (header + spacer).
            let y = body_area.y + 2 + row_idx as u16;
            if y >= body_area.y + body_area.height {
                break;
            }

            let row_rect = Rect {
                x: body_area.x,
                y,
                width: body_area.width,
                height: 1,
            };

            if index >= names.len() {
                let mut spans = Vec::new();
                let is_selected = index == self.selected_spinner_index;
                spans.push(Span::styled(
                    if is_selected { "› " } else { "  " }.to_string(),
                    Style::default().fg(if is_selected {
                        theme.keyword
                    } else {
                        theme.text
                    }),
                ));
                let label_style = if is_selected {
                    Style::default()
                        .fg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_dim)
                };
                spans.push(Span::styled("Generate your own…", label_style));
                Paragraph::new(Line::from(spans)).render(row_rect, buf);
                continue;
            }

            let name = &names[index];
            let is_selected = index == self.selected_spinner_index;
            let def =
                crate::spinner::find_spinner_by_name(name).unwrap_or(crate::spinner::current_spinner());
            let frame = crate::spinner::frame_at_time(def, now_ms);

            // Aligned columns:
            // selector (2) | left_rule | space | spinner | space | label | right_rule
            let border = if is_selected {
                Style::default().fg(crate::colors::border())
            } else {
                Style::default()
                    .fg(theme.text_dim)
                    .add_modifier(Modifier::DIM)
            };
            let fg = if is_selected {
                Style::default().fg(crate::colors::info())
            } else {
                Style::default()
                    .fg(theme.text_dim)
                    .add_modifier(Modifier::DIM)
            };
            let label = crate::spinner::spinner_label_for(name);

            let spinner_len = frame.chars().count() as u16;
            let text_len = (label.chars().count() as u16).saturating_add(3); // label + "..."
            let x: u16 = max_frame_len.saturating_add(8);
            let left_rule = x.saturating_sub(spinner_len);
            let right_rule = x.saturating_sub(text_len);

            let mut spans: Vec<Span> = Vec::new();
            spans.push(Span::styled(
                if is_selected { "› " } else { "  " }.to_string(),
                Style::default().fg(if is_selected {
                    theme.keyword
                } else {
                    theme.text
                }),
            ));
            spans.push(Span::styled("─".repeat(left_rule as usize), border));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(frame, fg));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(format!("{label}... "), fg));
            spans.push(Span::styled("─".repeat(right_rule as usize), border));
            Paragraph::new(Line::from(spans))
                .alignment(Alignment::Left)
                .render(row_rect, buf);
        }

        // Animate spinner previews while this mode is open.
        self.app_event_tx
            .send(AppEvent::ScheduleFrameIn(std::time::Duration::from_millis(
                100,
            )));
    }
}
