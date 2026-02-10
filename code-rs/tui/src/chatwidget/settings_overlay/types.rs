use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::bottom_pane::SettingsSection;

pub(super) const LABEL_COLUMN_WIDTH: usize = 18;

pub(crate) trait SettingsContent {
    fn render(&self, area: Rect, buf: &mut Buffer);
    fn handle_key(&mut self, key: KeyEvent) -> bool;
    fn is_complete(&self) -> bool;
    fn on_close(&mut self) {}
    fn handle_paste(&mut self, _text: String) -> bool {
        false
    }
    /// Handle mouse events in the content area. Returns true if handled/needs redraw.
    fn handle_mouse(&mut self, _mouse_event: MouseEvent, _area: Rect) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MenuState {
    selected: SettingsSection,
}

impl MenuState {
    pub(super) fn new(selected: SettingsSection) -> Self {
        Self { selected }
    }

    pub(super) fn selected(self) -> SettingsSection {
        self.selected
    }

    pub(super) fn set_selected(&mut self, section: SettingsSection) {
        self.selected = section;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SectionState {
    active: SettingsSection,
}

impl SectionState {
    pub(super) fn new(active: SettingsSection) -> Self {
        Self { active }
    }

    pub(super) fn active(self) -> SettingsSection {
        self.active
    }

    pub(super) fn set_active(&mut self, section: SettingsSection) {
        self.active = section;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SettingsOverlayMode {
    Menu(MenuState),
    Section(SectionState),
}

#[derive(Clone, Debug)]
pub(crate) struct SettingsOverviewRow {
    pub(crate) section: SettingsSection,
    pub(crate) summary: Option<String>,
}

impl SettingsOverviewRow {
    pub(crate) fn new(section: SettingsSection, summary: Option<String>) -> Self {
        Self { section, summary }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SettingsHelpOverlay {
    pub(super) lines: Vec<Line<'static>>,
}

impl SettingsHelpOverlay {
    pub(super) fn overview() -> Self {
        let title = Style::default()
            .fg(crate::colors::text())
            .add_modifier(Modifier::BOLD);
        let hint = Style::default().fg(crate::colors::text_dim());
        let mut lines = vec![Line::from(vec![Span::styled("Settings Overview", title)]), Line::default()];
        for text in [
            "• ↑/↓  Move between sections",
            "• Enter  Open selected section",
            "• Tab    Jump forward between sections",
            "• Esc    Close settings",
            "• ?      Toggle this help",
        ] {
            lines.push(Line::from(vec![Span::styled(text.to_string(), hint)]));
        }
        lines.push(Line::default());
        lines.push(Line::from(vec![Span::styled(
            "Press Esc to close",
            Style::default().fg(crate::colors::text_dim()),
        )]));
        Self { lines }
    }

    pub(super) fn section(section: SettingsSection) -> Self {
        let title = Style::default()
            .fg(crate::colors::text())
            .add_modifier(Modifier::BOLD);
        let hint = Style::default().fg(crate::colors::text_dim());
        let mut lines = vec![
            Line::from(vec![Span::styled(
                format!("{} Shortcuts", section.label()),
                title,
            )]),
            Line::default(),
            Line::from(vec![Span::styled("• Esc    Return to overview", hint)]),
            Line::from(vec![Span::styled("• Tab    Cycle sections", hint)]),
            Line::from(vec![Span::styled("• Shift+Tab  Cycle backwards", hint)]),
        ];
        if matches!(
            section,
            SettingsSection::Agents
                | SettingsSection::Mcp
                | SettingsSection::Accounts
                | SettingsSection::Skills
        ) {
            lines.push(Line::from(vec![Span::styled(
                "• Enter  Activate focused action",
                hint,
            )]));
        }
        lines.push(Line::from(vec![Span::styled("• ?      Toggle this help", hint)]));
        lines.push(Line::default());
        lines.push(Line::from(vec![Span::styled(
            "Press Esc to close",
            Style::default().fg(crate::colors::text_dim()),
        )]));
        Self { lines }
    }
}
