use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::chrome_launch::{CHROME_LAUNCH_CHOICES, ChromeLaunchOption};
use super::SettingsContent;

pub(crate) struct ChromeSettingsContent {
    selected_index: usize,
    app_event_tx: AppEventSender,
    port: Option<u16>,
    is_complete: bool,
}

impl ChromeSettingsContent {
    pub(crate) fn new(app_event_tx: AppEventSender, port: Option<u16>) -> Self {
        Self {
            selected_index: 0,
            app_event_tx,
            port,
            is_complete: false,
        }
    }

    fn options() -> &'static [(ChromeLaunchOption, &'static str, &'static str)] {
        CHROME_LAUNCH_CHOICES
    }

    fn move_up(&mut self) {
        let len = Self::options().len();
        if self.selected_index == 0 {
            self.selected_index = len.saturating_sub(1);
        } else {
            self.selected_index -= 1;
        }
    }

    fn move_down(&mut self) {
        let len = Self::options().len();
        if len > 0 {
            self.selected_index = (self.selected_index + 1) % len;
        }
    }

    fn confirm(&mut self) {
        if let Some((option, _, _)) = Self::options().get(self.selected_index) {
            self
                .app_event_tx
                .send(AppEvent::ChromeLaunchOptionSelected(*option, self.port));
            self.is_complete = true;
        }
    }

    fn cancel(&mut self) {
        self.app_event_tx.send(AppEvent::ChromeLaunchOptionSelected(
            ChromeLaunchOption::Cancel,
            self.port,
        ));
        self.is_complete = true;
    }

    fn option_at(&self, area: Rect, mouse_event: MouseEvent) -> Option<usize> {
        if area.width == 0 || area.height == 0 {
            return None;
        }

        let inner = Block::default().borders(Borders::ALL).inner(area);
        let content_area = inner.inner(Margin::new(1, 1));
        if content_area.width == 0 || content_area.height == 0 {
            return None;
        }

        if mouse_event.column < content_area.x
            || mouse_event.column >= content_area.x.saturating_add(content_area.width)
            || mouse_event.row < content_area.y
            || mouse_event.row >= content_area.y.saturating_add(content_area.height)
        {
            return None;
        }

        // Header uses 4 lines, each option consumes 3 lines (title + description + spacer).
        let rel_y = mouse_event.row.saturating_sub(content_area.y) as usize;
        if rel_y < 4 {
            return None;
        }
        let option_index = (rel_y - 4) / 3;
        if option_index < Self::options().len() {
            Some(option_index)
        } else {
            None
        }
    }
}

impl SettingsContent for ChromeSettingsContent {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        Clear.render(area, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .title(Line::from(" Chrome Launch Options "))
            .title_alignment(Alignment::Center)
            .style(Style::default().bg(crate::colors::background()).fg(crate::colors::text()))
            .border_style(Style::default().fg(crate::colors::border()));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from(vec![Span::styled(
            "Chrome is already running or CDP connection failed",
            Style::default()
                .fg(crate::colors::warning())
                .add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));
        lines.push(Line::from("Select an option:"));
        lines.push(Line::from(""));

        for (idx, (_, label, description)) in Self::options().iter().enumerate() {
            let selected = idx == self.selected_index;
            if selected {
                lines.push(Line::from(vec![Span::styled(
                    format!("› {label}"),
                    Style::default()
                        .fg(crate::colors::success())
                        .add_modifier(Modifier::BOLD),
                )]));
                lines.push(Line::from(vec![Span::styled(
                    format!("  {description}"),
                    Style::default().fg(crate::colors::secondary()),
                )]));
            } else {
                lines.push(Line::from(vec![Span::styled(
                    format!("  {label}"),
                    Style::default().fg(crate::colors::text()),
                )]));
                lines.push(Line::from(vec![Span::styled(
                    format!("  {description}"),
                    Style::default().fg(crate::colors::text_dim()),
                )]));
            }
            lines.push(Line::from(""));
        }

        lines.push(Line::from(vec![
            Span::styled("↑↓/jk", Style::default().fg(crate::colors::function())),
            Span::styled(" move  ", Style::default().fg(crate::colors::text_dim())),
            Span::styled("Enter", Style::default().fg(crate::colors::function())),
            Span::styled(" select  ", Style::default().fg(crate::colors::text_dim())),
            Span::styled("Esc/q", Style::default().fg(crate::colors::function())),
            Span::styled(" cancel", Style::default().fg(crate::colors::text_dim())),
        ]));

        let content_area = inner.inner(Margin::new(1, 1));
        if content_area.width == 0 || content_area.height == 0 {
            return;
        }

        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .style(Style::default().bg(crate::colors::background()).fg(crate::colors::text()))
            .render(content_area, buf);
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up();
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down();
                true
            }
            KeyCode::Enter => {
                self.confirm();
                true
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.cancel();
                true
            }
            _ => false,
        }
    }

    fn handle_mouse(&mut self, mouse_event: MouseEvent, area: Rect) -> bool {
        match mouse_event.kind {
            MouseEventKind::Moved => {
                let Some(index) = self.option_at(area, mouse_event) else {
                    return false;
                };
                if self.selected_index == index {
                    return false;
                }
                self.selected_index = index;
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(index) = self.option_at(area, mouse_event) else {
                    return false;
                };
                self.selected_index = index;
                self.confirm();
                true
            }
            MouseEventKind::ScrollUp => {
                self.move_up();
                true
            }
            MouseEventKind::ScrollDown => {
                self.move_down();
                true
            }
            _ => false,
        }
    }

    fn is_complete(&self) -> bool {
        self.is_complete
    }
}
