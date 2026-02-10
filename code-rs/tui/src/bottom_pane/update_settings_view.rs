use std::sync::{Arc, Mutex};

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::chatwidget::BackgroundOrderTicket;
use crate::colors;
use crate::util::buffer::fill_rect;
use super::bottom_pane_view::BottomPaneView;
use super::bottom_pane_view::ConditionalUpdate;
use super::BottomPane;
use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Margin, Rect};
use ratatui::prelude::Widget;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::settings_panel::{render_panel, PanelFrameStyle};

#[derive(Debug, Clone, Default)]
pub struct UpdateSharedState {
    pub checking: bool,
    pub latest_version: Option<String>,
    pub error: Option<String>,
}

pub(crate) struct UpdateSettingsView {
    app_event_tx: AppEventSender,
    ticket: BackgroundOrderTicket,
    field: usize,
    is_complete: bool,
    auto_enabled: bool,
    shared: Arc<Mutex<UpdateSharedState>>,
    current_version: String,
    command: Option<Vec<String>>,
    command_display: Option<String>,
    manual_instructions: Option<String>,
}

impl UpdateSettingsView {
    const PANEL_TITLE: &'static str = "Upgrade";

    pub fn new(
        app_event_tx: AppEventSender,
        ticket: BackgroundOrderTicket,
        current_version: String,
        auto_enabled: bool,
        command: Option<Vec<String>>,
        command_display: Option<String>,
        manual_instructions: Option<String>,
        shared: Arc<Mutex<UpdateSharedState>>,
    ) -> Self {
        Self {
            app_event_tx,
            ticket,
            field: 0,
            is_complete: false,
            auto_enabled,
            shared,
            current_version,
            command,
            command_display,
            manual_instructions,
        }
    }

    fn toggle_auto(&mut self) {
        self.auto_enabled = !self.auto_enabled;
        self.app_event_tx
            .send(AppEvent::SetAutoUpgradeEnabled(self.auto_enabled));
    }

    fn invoke_run_upgrade(&mut self) {
        let state = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();

            if self.command.is_none() {
                if let Some(instructions) = &self.manual_instructions {
                    self.app_event_tx
                        .send_background_event_with_ticket(&self.ticket, instructions.clone());
                }
                return;
            }

            if state.checking {
                self.app_event_tx.send_background_event_with_ticket(
                    &self.ticket,
                    "Still checking for updates…".to_string(),
                );
                return;
            }
            if let Some(err) = &state.error {
                self.app_event_tx.send_background_event_with_ticket(
                    &self.ticket,
                    format!("❌ /update failed: {err}"),
                );
                return;
            }
            let Some(latest) = state.latest_version else {
                self.app_event_tx.send_background_event_with_ticket(
                    &self.ticket,
                    "✅ Code is already up to date.".to_string(),
                );
                return;
            };

        let Some(command) = self.command.clone() else {
            return;
        };
        let display = self
            .command_display
            .clone()
            .unwrap_or_else(|| command.join(" "));

        self.app_event_tx.send_background_event_with_ticket(
            &self.ticket,
            format!(
                "⬆️ Update available: {} → {}. Opening guided upgrade with `{}`…",
                self.current_version, latest, display
            ),
        );
        self.app_event_tx.send(AppEvent::RunUpdateCommand {
            command,
            display: display.clone(),
            latest_version: Some(latest.clone()),
        });
        self.app_event_tx.send_background_event_with_ticket(
            &self.ticket,
            format!(
                "↻ Complete the guided terminal steps for `{display}` then restart Code to finish upgrading to {latest}."
            ),
        );
        self.is_complete = true;
    }

    fn build_lines(&self) -> Vec<Line<'static>> {
        let state = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();

        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from(vec![Span::styled(
            "Upgrade",
            Style::default().add_modifier(Modifier::BOLD),
        )]));

        let run_selected = self.field == 0;
        let run_style = if run_selected {
            Style::default().fg(colors::primary())
        } else {
            Style::default()
        };
        let version_summary = if state.checking {
            "checking…".to_string()
        } else if let Some(err) = &state.error {
            err.clone()
        } else if let Some(latest) = &state.latest_version {
            format!("{} → {}", self.current_version, latest)
        } else {
            self.current_version.to_string()
        };

        let run_prefix = if run_selected { "› " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(run_prefix, run_style),
            Span::styled("Run Upgrade", run_style),
            Span::raw("  "),
            Span::styled(version_summary, Style::default().fg(colors::text_dim())),
        ]));

        let toggle_selected = self.field == 1;
        let toggle_prefix = if toggle_selected { "› " } else { "  " };
        let toggle_label_style = if toggle_selected {
            Style::default().fg(colors::primary())
        } else {
            Style::default()
        };
        let enabled_box_style = if self.auto_enabled {
            Style::default().fg(colors::success())
        } else {
            Style::default().fg(colors::text_dim())
        };
        let disabled_box_style = if self.auto_enabled {
            Style::default().fg(colors::text_dim())
        } else {
            Style::default().fg(colors::error())
        };
        lines.push(Line::from(vec![
            Span::styled(toggle_prefix, toggle_label_style),
            Span::styled("Automatic Upgrades", toggle_label_style),
            Span::raw("  "),
            Span::styled(
                format!("[{}] Enabled", if self.auto_enabled { "x" } else { " " }),
                enabled_box_style,
            ),
            Span::raw("  "),
            Span::styled(
                format!("[{}] Disabled", if self.auto_enabled { " " } else { "x" }),
                disabled_box_style,
            ),
        ]));

        let close_selected = self.field == 2;
        let close_prefix = if close_selected { "› " } else { "  " };
        let close_style = if close_selected {
            Style::default().fg(colors::primary())
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled(close_prefix, close_style),
            Span::styled("Close", close_style),
        ]));

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" ↑↓", Style::default().fg(colors::function())),
            Span::styled(" Navigate  ", Style::default().fg(colors::text_dim())),
            Span::styled("Enter", Style::default().fg(colors::success())),
            Span::styled(" Configure  ", Style::default().fg(colors::text_dim())),
            Span::styled("Esc", Style::default().fg(colors::error())),
            Span::styled(" Close", Style::default().fg(colors::text_dim())),
        ]));

        // Colors for the enabled/disabled boxes already set; no extra lines needed.

        lines
    }

    fn selectable_row_at(&self, area: Rect, mouse_event: MouseEvent) -> Option<usize> {
        if area.width == 0 || area.height == 0 {
            return None;
        }
        if mouse_event.column < area.x
            || mouse_event.column >= area.x.saturating_add(area.width)
            || mouse_event.row < area.y
            || mouse_event.row >= area.y.saturating_add(area.height)
        {
            return None;
        }

        let rel_y = mouse_event.row.saturating_sub(area.y);
        match rel_y {
            1 => Some(0),
            2 => Some(1),
            3 => Some(2),
            _ => None,
        }
    }

    fn activate_selected(&mut self) {
        match self.field {
            0 => self.invoke_run_upgrade(),
            1 => self.toggle_auto(),
            _ => self.is_complete = true,
        }
    }

    pub fn handle_key_event_direct(&mut self, key_event: KeyEvent) {
        const FIELD_COUNT: usize = 3;

        match key_event.code {
            KeyCode::Esc => self.is_complete = true,
            KeyCode::Tab | KeyCode::Down => {
                self.field = (self.field + 1) % FIELD_COUNT;
            }
            KeyCode::BackTab | KeyCode::Up => {
                if self.field == 0 {
                    self.field = FIELD_COUNT - 1;
                } else {
                    self.field -= 1;
                }
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') if self.field == 1 => {
                self.toggle_auto();
            }
            KeyCode::Enter => match self.field {
                0 => self.invoke_run_upgrade(),
                1 => self.toggle_auto(),
                _ => self.is_complete = true,
            },
            _ => {}
        }
        self.app_event_tx.send(AppEvent::RequestRedraw);
    }

    pub fn handle_mouse_event_direct(&mut self, mouse_event: MouseEvent, area: Rect) -> bool {
        const FIELD_COUNT: usize = 3;
        match mouse_event.kind {
            MouseEventKind::Moved => {
                let Some(row) = self.selectable_row_at(area, mouse_event) else {
                    return false;
                };
                if self.field == row {
                    return false;
                }
                self.field = row;
                true
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(row) = self.selectable_row_at(area, mouse_event) else {
                    return false;
                };
                self.field = row;
                self.activate_selected();
                self.app_event_tx.send(AppEvent::RequestRedraw);
                true
            }
            MouseEventKind::ScrollUp => {
                if self.field == 0 {
                    self.field = FIELD_COUNT - 1;
                } else {
                    self.field -= 1;
                }
                true
            }
            MouseEventKind::ScrollDown => {
                self.field = (self.field + 1) % FIELD_COUNT;
                true
            }
            _ => false,
        }
    }

    pub fn is_view_complete(&self) -> bool {
        self.is_complete
    }

    fn render_panel_body(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let lines = self.build_lines();
        let bg_style = Style::default().bg(colors::background()).fg(colors::text());
        fill_rect(buf, area, Some(' '), bg_style);

        Paragraph::new(lines)
            .alignment(Alignment::Left)
            .style(bg_style)
            .wrap(Wrap { trim: false })
            .render(area, buf);
    }

    pub(crate) fn render_without_frame(&self, area: Rect, buf: &mut Buffer) {
        self.render_panel_body(area, buf);
    }

}

impl<'a> BottomPaneView<'a> for UpdateSettingsView {
    fn handle_key_event(&mut self, _pane: &mut BottomPane<'a>, key_event: KeyEvent) {
        self.handle_key_event_direct(key_event);
    }

    fn is_complete(&self) -> bool {
        self.is_complete
    }

    fn desired_height(&self, _width: u16) -> u16 {
        self.build_lines().len().saturating_add(2) as u16
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        render_panel(
            area,
            buf,
            Self::PANEL_TITLE,
            PanelFrameStyle::bottom_pane().with_margin(Margin::new(1, 0)),
            |inner, buf| self.render_panel_body(inner, buf),
        );
    }

    fn handle_paste(&mut self, _text: String) -> ConditionalUpdate {
        ConditionalUpdate::NoRedraw
    }
}
