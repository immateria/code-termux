use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::{
    BottomPaneView,
    ConditionalUpdate,
    agent_editor_view::AgentEditorView,
    agents_settings_view::SubagentEditorView,
};

use super::SettingsContent;

#[derive(Clone, Debug)]
pub(crate) struct AgentOverviewRow {
    pub(crate) name: String,
    pub(crate) enabled: bool,
    pub(crate) installed: bool,
    pub(crate) description: Option<String>,
}

#[derive(Default)]
struct AgentsOverviewState {
    rows: Vec<AgentOverviewRow>,
    commands: Vec<String>,
    selected: usize,
}

impl AgentsOverviewState {
    fn total_rows(&self) -> usize {
        self.rows
            .len()
            .saturating_add(self.commands.len())
            .saturating_add(2)
    }

    fn clamp_selection(&mut self) {
        let total = self.total_rows();
        if total == 0 {
            self.selected = 0;
        } else if self.selected >= total {
            self.selected = total - 1;
        }
    }
}

enum AgentsPane {
    Overview(AgentsOverviewState),
    Subagent(SubagentEditorView),
    Agent(AgentEditorView),
}

pub(crate) struct AgentsSettingsContent {
    pane: AgentsPane,
    app_event_tx: AppEventSender,
}

impl AgentsSettingsContent {
    pub(crate) fn new_overview(
        rows: Vec<AgentOverviewRow>,
        commands: Vec<String>,
        selected: usize,
        app_event_tx: AppEventSender,
    ) -> Self {
        let mut overview = AgentsOverviewState {
            rows,
            commands,
            selected,
        };
        overview.clamp_selection();
        Self {
            pane: AgentsPane::Overview(overview),
            app_event_tx,
        }
    }

    pub(crate) fn set_overview(
        &mut self,
        rows: Vec<AgentOverviewRow>,
        commands: Vec<String>,
        selected: usize,
    ) {
        let mut overview = AgentsOverviewState {
            rows,
            commands,
            selected,
        };
        overview.clamp_selection();
        self.pane = AgentsPane::Overview(overview);
    }

    pub(crate) fn set_editor(&mut self, editor: SubagentEditorView) {
        self.pane = AgentsPane::Subagent(editor);
    }

    pub(crate) fn set_overview_selection(&mut self, selected: usize) {
        if let AgentsPane::Overview(state) = &mut self.pane {
            state.selected = selected;
            state.clamp_selection();
        }
    }

    pub(crate) fn set_agent_editor(&mut self, editor: AgentEditorView) {
        self.pane = AgentsPane::Agent(editor);
    }

    #[cfg_attr(not(any(test, feature = "test-helpers")), allow(dead_code))]
    pub(crate) fn is_agent_editor_active(&self) -> bool {
        matches!(self.pane, AgentsPane::Agent(_))
    }

    fn render_overview(&self, area: Rect, buf: &mut Buffer, state: &AgentsOverviewState) {
        use ratatui::widgets::Paragraph;

        let lines = Self::build_overview_lines(state, Some(area.width as usize));
        Paragraph::new(lines)
            .style(Style::default().bg(crate::colors::background()).fg(crate::colors::text()))
            .render(area, buf);
    }

    fn build_overview_lines(
        state: &AgentsOverviewState,
        available_width: Option<usize>,
    ) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        lines.push(Line::from(Span::styled(
            "Agents",
            Style::default().add_modifier(Modifier::BOLD),
        )));

        let max_name_chars = state
            .rows
            .iter()
            .map(|row| row.name.chars().count())
            .max()
            .unwrap_or(0);
        let max_name_width = state
            .rows
            .iter()
            .map(|row| UnicodeWidthStr::width(row.name.as_str()))
            .max()
            .unwrap_or(0);

        for (idx, row) in state.rows.iter().enumerate() {
            let selected = idx == state.selected;
            let status = if !row.enabled {
                ("disabled", crate::colors::error())
            } else if !row.installed {
                ("not installed", crate::colors::warning())
            } else {
                ("enabled", crate::colors::success())
            };

            let mut spans = Vec::new();
            spans.push(Span::styled(
                if selected { "› " } else { "  " },
                if selected {
                    Style::default().fg(crate::colors::primary())
                } else {
                    Style::default()
                },
            ));
            spans.push(Span::styled(
                format!("{:<width$}", row.name, width = max_name_chars),
                if selected {
                    Style::default()
                        .fg(crate::colors::primary())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            ));
            spans.push(Span::raw("  "));
            spans.push(Span::styled("•", Style::default().fg(status.1)));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(status.0.to_string(), Style::default().fg(status.1)));

            let mut showed_desc = false;
            if let Some(desc) = row
                .description
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                && let Some(width) = available_width {
                    let status_width = UnicodeWidthStr::width(status.0);
                    let prefix_width = 2 + max_name_width + 2 + 2 + status_width;
                    if width > prefix_width + 3 {
                        let desc_width = width - prefix_width - 3;
                        if desc_width > 0 {
                            let truncated = Self::truncate_to_width(desc, desc_width);
                            if !truncated.is_empty() {
                                spans.push(Span::raw("  "));
                                spans.push(Span::styled(
                                    truncated,
                                    Style::default().fg(crate::colors::text_dim()),
                                ));
                                showed_desc = true;
                            }
                        }
                    }
                }

            if selected && !showed_desc {
                spans.push(Span::raw("  "));
                let hint = if !row.installed {
                    "Enter to install"
                } else {
                    "Enter to configure"
                };
                spans.push(Span::styled(hint, Style::default().fg(crate::colors::text_dim())));
            }

            lines.push(Line::from(spans));
        }

        lines.push(Line::from(""));

        let add_agent_idx = state.rows.len();
        let add_agent_selected = add_agent_idx == state.selected;
        let mut add_spans: Vec<Span<'static>> = Vec::new();
        add_spans.push(Span::styled(
            if add_agent_selected { "› " } else { "  " },
            if add_agent_selected {
                Style::default().fg(crate::colors::primary())
            } else {
                Style::default()
            },
        ));
        add_spans.push(Span::styled(
            "Add new agent…",
            if add_agent_selected {
                Style::default()
                    .fg(crate::colors::primary())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        ));
        if add_agent_selected {
            add_spans.push(Span::raw("  "));
            add_spans.push(Span::styled(
                "Enter to configure",
                Style::default().fg(crate::colors::text_dim()),
            ));
        }
        lines.push(Line::from(add_spans));

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Commands",
            Style::default().add_modifier(Modifier::BOLD),
        )));

        for (offset, cmd) in state.commands.iter().enumerate() {
            let idx = state.rows.len() + 1 + offset;
            let selected = idx == state.selected;
            let mut spans = Vec::new();
            spans.push(Span::styled(
                if selected { "› " } else { "  " },
                if selected {
                    Style::default().fg(crate::colors::primary())
                } else {
                    Style::default()
                },
            ));
            spans.push(Span::styled(
                format!("/{cmd}"),
                if selected {
                    Style::default()
                        .fg(crate::colors::primary())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            ));
            if selected {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    "Enter to configure",
                    Style::default().fg(crate::colors::text_dim()),
                ));
            }
            lines.push(Line::from(spans));
        }

        let add_idx = state.rows.len() + 1 + state.commands.len();
        let add_selected = add_idx == state.selected;
        let mut add_spans = Vec::new();
        add_spans.push(Span::styled(
            if add_selected { "› " } else { "  " },
            if add_selected {
                Style::default().fg(crate::colors::primary())
            } else {
                Style::default()
            },
        ));
        add_spans.push(Span::styled(
            "Add new…",
            if add_selected {
                Style::default()
                    .fg(crate::colors::primary())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            },
        ));
        if add_selected {
            add_spans.push(Span::raw("  "));
            add_spans.push(Span::styled(
                "Enter to create",
                Style::default().fg(crate::colors::text_dim()),
            ));
        }
        lines.push(Line::from(add_spans));

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("↑↓", Style::default().fg(crate::colors::function())),
            Span::styled(" Navigate  ", Style::default().fg(crate::colors::text_dim())),
            Span::styled("Enter", Style::default().fg(crate::colors::success())),
            Span::styled(" Open", Style::default().fg(crate::colors::text_dim())),
            Span::styled("  Esc", Style::default().fg(crate::colors::error())),
            Span::styled(" Close", Style::default().fg(crate::colors::text_dim())),
        ]));

        lines
    }

    fn truncate_to_width(text: &str, max_width: usize) -> String {
        if max_width == 0 {
            return String::new();
        }
        let mut width = 0;
        let mut truncated = String::new();
        for ch in text.chars() {
            let ch_width = ch.width().unwrap_or(0);
            if width + ch_width > max_width {
                break;
            }
            truncated.push(ch);
            width += ch_width;
            if width == max_width {
                break;
            }
        }
        truncated
    }

    fn overview_selection_at(
        state: &AgentsOverviewState,
        area: Rect,
        mouse_event: MouseEvent,
    ) -> Option<usize> {
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

        let rel_y = mouse_event.row.saturating_sub(area.y) as usize;
        let rows_len = state.rows.len();
        let command_len = state.commands.len();

        if rel_y >= 1 && rel_y < 1 + rows_len {
            return Some(rel_y - 1);
        }

        let add_agent_line = rows_len + 2;
        if rel_y == add_agent_line {
            return Some(rows_len);
        }

        let command_start = rows_len + 5;
        if rel_y >= command_start && rel_y < command_start + command_len {
            return Some(rows_len + 1 + (rel_y - command_start));
        }

        let add_command_line = command_start + command_len;
        if rel_y == add_command_line {
            return Some(rows_len + 1 + command_len);
        }

        None
    }

    fn handle_overview_key(
        state: &mut AgentsOverviewState,
        key: KeyEvent,
        app_event_tx: &AppEventSender,
    ) -> bool {
        match key.code {
            KeyCode::Up => {
                if state.total_rows() == 0 {
                    return true;
                }
                if state.selected == 0 {
                    state.selected = state.total_rows().saturating_sub(1);
                } else {
                    state.selected -= 1;
                }
                app_event_tx.send(AppEvent::AgentsOverviewSelectionChanged {
                    index: state.selected,
                });
                true
            }
            KeyCode::Down => {
                let total = state.total_rows();
                if total == 0 {
                    return true;
                }
                state.selected = (state.selected + 1) % total;
                app_event_tx.send(AppEvent::AgentsOverviewSelectionChanged {
                    index: state.selected,
                });
                true
            }
            KeyCode::Enter => {
                let idx = state.selected;
                let add_agent_idx = state.rows.len();
                if idx == add_agent_idx {
                    app_event_tx.send(AppEvent::ShowAgentEditorNew);
                } else if idx < add_agent_idx {
                    let row = &state.rows[idx];
                    if !row.installed {
                        app_event_tx.send(AppEvent::RequestAgentInstall {
                            name: row.name.clone(),
                            selected_index: idx,
                        });
                    } else {
                        app_event_tx.send(AppEvent::ShowAgentEditor {
                            name: row.name.clone(),
                        });
                    }
                } else {
                    let cmd_idx = idx.saturating_sub(state.rows.len() + 1);
                    if cmd_idx < state.commands.len() {
                        if let Some(name) = state.commands.get(cmd_idx) {
                            app_event_tx.send(AppEvent::ShowSubagentEditorForName {
                                name: name.clone(),
                            });
                        }
                    } else {
                        app_event_tx.send(AppEvent::ShowSubagentEditorNew);
                    }
                }
                true
            }
            _ => false,
        }
    }
}

impl SettingsContent for AgentsSettingsContent {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        match &self.pane {
            AgentsPane::Overview(state) => {
                self.render_overview(area, buf, state);
            }
            AgentsPane::Subagent(view) => {
                view.render(area, buf);
            }
            AgentsPane::Agent(view) => {
                view.render(area, buf);
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        match &mut self.pane {
            AgentsPane::Overview(state) => Self::handle_overview_key(state, key, &self.app_event_tx),
            AgentsPane::Subagent(view) => view.handle_key_event_direct(key),
            AgentsPane::Agent(view) => view.handle_key_event_direct(key),
        }
    }

    fn is_complete(&self) -> bool {
        false
    }

    fn handle_paste(&mut self, text: String) -> bool {
        match &mut self.pane {
            AgentsPane::Agent(view) => {
                matches!(view.handle_paste(text), ConditionalUpdate::NeedsRedraw)
            }
            _ => false,
        }
    }

    fn handle_mouse(&mut self, mouse_event: MouseEvent, area: Rect) -> bool {
        match (&mut self.pane, mouse_event.kind) {
            (AgentsPane::Overview(state), MouseEventKind::Moved) => {
                let Some(next) = Self::overview_selection_at(state, area, mouse_event) else {
                    return false;
                };
                if state.selected == next {
                    return false;
                }
                state.selected = next;
                self.app_event_tx.send(AppEvent::AgentsOverviewSelectionChanged {
                    index: state.selected,
                });
                true
            }
            (AgentsPane::Overview(state), MouseEventKind::Down(MouseButton::Left)) => {
                let Some(next) = Self::overview_selection_at(state, area, mouse_event) else {
                    return false;
                };
                state.selected = next;
                self.app_event_tx.send(AppEvent::AgentsOverviewSelectionChanged {
                    index: state.selected,
                });
                Self::handle_overview_key(
                    state,
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    &self.app_event_tx,
                )
            }
            (AgentsPane::Overview(state), MouseEventKind::ScrollUp) => Self::handle_overview_key(
                state,
                KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
                &self.app_event_tx,
            ),
            (AgentsPane::Overview(state), MouseEventKind::ScrollDown) => Self::handle_overview_key(
                state,
                KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                &self.app_event_tx,
            ),
            _ => false,
        }
    }
}
