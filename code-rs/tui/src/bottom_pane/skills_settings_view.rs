use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use code_core::config::{
    find_code_home,
    set_shell_style_profile_mcp_servers,
    set_shell_style_profile_paths,
    set_shell_style_profile_skill_mode,
    ShellStyleSkillMode,
};
use code_core::config_types::{ShellScriptStyle, ShellStyleProfileConfig};
use code_core::protocol::Op;
use code_protocol::skills::{Skill, SkillScope};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::prelude::Widget;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::colors;

use super::bottom_pane_view::{BottomPaneView, ConditionalUpdate};
use super::BottomPane;
use crate::components::form_text_field::{FormTextField, InputFilter};
use crate::ui_interaction::{
    clipped_vertical_rect_with_scroll,
    next_scroll_top_with_delta,
    redraw_if,
    render_vertical_scrollbar,
    route_selectable_list_mouse_with_config,
    scroll_top_to_keep_visible,
    split_pinned_footer_layout,
    ScrollSelectionBehavior,
    SelectableListMouseConfig,
    SelectableListMouseResult,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    List,
    Name,
    Description,
    Style,
    StyleProfile,
    StyleReferences,
    StyleSkillRoots,
    StyleMcpInclude,
    StyleMcpExclude,
    Examples,
    Body,
    Generate,
    Save,
    Delete,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    List,
    Edit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActionButton {
    Generate,
    Save,
    Delete,
    Cancel,
}

impl ActionButton {
    fn focus(self) -> Focus {
        match self {
            Self::Generate => Focus::Generate,
            Self::Save => Focus::Save,
            Self::Delete => Focus::Delete,
            Self::Cancel => Focus::Cancel,
        }
    }
}

#[derive(Clone, Copy)]
struct SkillsFormLayout {
    viewport_inner: Rect,
    scroll_top: usize,
    max_scroll: usize,
    status_row: Rect,
    name_field: Rect,
    name_top: usize,
    name_h: usize,
    description_field: Rect,
    description_top: usize,
    description_h: usize,
    style_field: Rect,
    style_top: usize,
    style_h: usize,
    style_profile_row: Rect,
    style_profile_top: usize,
    style_profile_h: usize,
    style_references_outer: Rect,
    style_references_inner: Rect,
    style_references_top: usize,
    style_references_h: usize,
    style_skill_roots_outer: Rect,
    style_skill_roots_inner: Rect,
    style_skill_roots_top: usize,
    style_skill_roots_h: usize,
    style_mcp_include_outer: Rect,
    style_mcp_include_inner: Rect,
    style_mcp_include_top: usize,
    style_mcp_include_h: usize,
    style_mcp_exclude_outer: Rect,
    style_mcp_exclude_inner: Rect,
    style_mcp_exclude_top: usize,
    style_mcp_exclude_h: usize,
    examples_outer: Rect,
    examples_inner: Rect,
    examples_top: usize,
    examples_h: usize,
    body_outer: Rect,
    body_inner: Rect,
    body_top: usize,
    body_h: usize,
    buttons_row: Rect,
}

impl SkillsFormLayout {
    fn focus_bounds(self, focus: Focus) -> Option<(usize, usize)> {
        match focus {
            Focus::Name => Some((self.name_top, self.name_h)),
            Focus::Description => Some((self.description_top, self.description_h)),
            Focus::Style => Some((self.style_top, self.style_h)),
            Focus::StyleProfile => Some((self.style_profile_top, self.style_profile_h)),
            Focus::StyleReferences => Some((self.style_references_top, self.style_references_h)),
            Focus::StyleSkillRoots => Some((self.style_skill_roots_top, self.style_skill_roots_h)),
            Focus::StyleMcpInclude => Some((self.style_mcp_include_top, self.style_mcp_include_h)),
            Focus::StyleMcpExclude => Some((self.style_mcp_exclude_top, self.style_mcp_exclude_h)),
            Focus::Examples => Some((self.examples_top, self.examples_h)),
            Focus::Body => Some((self.body_top, self.body_h)),
            Focus::Generate | Focus::Save | Focus::Delete | Focus::Cancel => None,
            Focus::List => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StyleProfileMode {
    Inherit,
    Enable,
    Disable,
}

impl StyleProfileMode {
    fn label(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Enable => "enable for style",
            Self::Disable => "disable for style",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Inherit => "Skill follows style defaults (not pinned in this style profile).",
            Self::Enable => "Add skill to shell_style_profiles.<style>.skills allow-list.",
            Self::Disable => "Add skill to shell_style_profiles.<style>.disabled_skills override list.",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Inherit => Self::Enable,
            Self::Enable => Self::Disable,
            Self::Disable => Self::Inherit,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Inherit => Self::Disable,
            Self::Enable => Self::Inherit,
            Self::Disable => Self::Enable,
        }
    }

    fn into_config_mode(self) -> ShellStyleSkillMode {
        match self {
            Self::Inherit => ShellStyleSkillMode::Inherit,
            Self::Enable => ShellStyleSkillMode::Enabled,
            Self::Disable => ShellStyleSkillMode::Disabled,
        }
    }
}

pub(crate) struct SkillsSettingsView {
    skills: Vec<Skill>,
    shell_style_profiles: HashMap<ShellScriptStyle, ShellStyleProfileConfig>,
    selected: usize,
    focus: Focus,
    name_field: FormTextField,
    description_field: FormTextField,
    style_field: FormTextField,
    style_references_field: FormTextField,
    style_skill_roots_field: FormTextField,
    style_mcp_include_field: FormTextField,
    style_mcp_exclude_field: FormTextField,
    examples_field: FormTextField,
    body_field: FormTextField,
    style_references_dirty: bool,
    style_skill_roots_dirty: bool,
    style_mcp_include_dirty: bool,
    style_mcp_exclude_dirty: bool,
    status: Option<(String, Style)>,
    style_profile_mode: StyleProfileMode,
    hovered_button: Option<ActionButton>,
    app_event_tx: AppEventSender,
    is_complete: bool,
    mode: Mode,
    edit_scroll_top: usize,
    last_render_area: Cell<Option<Rect>>,
}

impl SkillsSettingsView {
    pub fn new(
        skills: Vec<Skill>,
        shell_style_profiles: HashMap<ShellScriptStyle, ShellStyleProfileConfig>,
        app_event_tx: AppEventSender,
    ) -> Self {
        let mut name_field = FormTextField::new_single_line();
        name_field.set_filter(InputFilter::Id);
        let mut description_field = FormTextField::new_single_line();
        description_field.set_placeholder("Describe when this skill should be used.");
        let style_field = FormTextField::new_single_line();
        let style_references_field = FormTextField::new_multi_line();
        let style_skill_roots_field = FormTextField::new_multi_line();
        let style_mcp_include_field = FormTextField::new_multi_line();
        let style_mcp_exclude_field = FormTextField::new_multi_line();
        let mut examples_field = FormTextField::new_multi_line();
        examples_field
            .set_placeholder("- User asks for ...\n- User needs ...\n- Trigger when ...");
        let mut body_field = FormTextField::new_multi_line();
        body_field.set_placeholder(
            "# Overview\n\nSummarize what this skill does and why.\n\n## Workflow\n\n1. Outline the first step.\n2. Outline the second step.\n",
        );
        Self {
            skills,
            shell_style_profiles,
            selected: 0,
            focus: Focus::List,
            name_field,
            description_field,
            style_field,
            style_references_field,
            style_skill_roots_field,
            style_mcp_include_field,
            style_mcp_exclude_field,
            examples_field,
            body_field,
            style_references_dirty: false,
            style_skill_roots_dirty: false,
            style_mcp_include_dirty: false,
            style_mcp_exclude_dirty: false,
            status: None,
            style_profile_mode: StyleProfileMode::Inherit,
            hovered_button: None,
            app_event_tx,
            is_complete: false,
            mode: Mode::List,
            edit_scroll_top: 0,
            last_render_area: Cell::new(None),
        }
    }

    pub fn handle_key_event_direct(&mut self, key: KeyEvent) -> bool {
        if self.is_complete {
            return true;
        }
        let handled = match self.mode {
            Mode::List => match key {
                KeyEvent { code: KeyCode::Esc, .. } => {
                    self.is_complete = true;
                    true
                }
                KeyEvent { code: KeyCode::Enter, modifiers: KeyModifiers::NONE, .. } => {
                    self.enter_editor();
                    true
                }
                KeyEvent { code: KeyCode::Char('n'), modifiers, .. }
                    if modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    self.start_new_skill();
                    true
                }
                other => self.handle_list_key(other),
            },
            Mode::Edit => match key {
                KeyEvent { code: KeyCode::Esc, .. } => {
                    self.mode = Mode::List;
                    self.focus = Focus::List;
                    self.hovered_button = None;
                    self.status = None;
                    true
                }
                KeyEvent { code: KeyCode::Tab, .. } => {
                    self.cycle_focus(true);
                    true
                }
                KeyEvent { code: KeyCode::BackTab, .. } => {
                    self.cycle_focus(false);
                    true
                }
                KeyEvent { code: KeyCode::Enter, modifiers: KeyModifiers::NONE, .. }
                    if matches!(
                        self.focus,
                        Focus::StyleProfile
                            | Focus::Generate
                            | Focus::Save
                            | Focus::Delete
                            | Focus::Cancel
                    ) =>
                {
                    match self.focus {
                        Focus::StyleProfile => self.cycle_style_profile_mode(true),
                        Focus::Generate => self.generate_draft(),
                        Focus::Save => self.save_current(),
                        Focus::Delete => self.delete_current(),
                        Focus::Cancel => {
                            self.mode = Mode::List;
                            self.focus = Focus::List;
                            self.hovered_button = None;
                            self.status = None;
                        }
                        Focus::List
                        | Focus::Name
                        | Focus::Description
                        | Focus::Style
                        | Focus::StyleReferences
                        | Focus::StyleSkillRoots
                        | Focus::StyleMcpInclude
                        | Focus::StyleMcpExclude
                        | Focus::Examples
                        | Focus::Body => {}
                    }
                    true
                }
                KeyEvent { code: KeyCode::Char('n'), modifiers, .. }
                    if modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    self.start_new_skill();
                    true
                }
                KeyEvent { code: KeyCode::Char('g'), modifiers, .. }
                    if modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    self.generate_draft();
                    true
                }
                _ => match self.focus {
                    Focus::Name => {
                        self.name_field.handle_key(key);
                        true
                    }
                    Focus::Description => {
                        self.description_field.handle_key(key);
                        true
                    }
                    Focus::Style => {
                        let previous_style =
                            ShellScriptStyle::parse(self.style_field.text().trim());
                        self.style_field.handle_key(key);
                        let next_style = ShellScriptStyle::parse(self.style_field.text().trim());
                        if next_style != previous_style
                            && next_style.is_some()
                            && !self.style_profile_fields_dirty()
                        {
                            self.set_style_resource_fields_from_profile(next_style);
                        }
                        true
                    }
                    Focus::StyleProfile => match key.code {
                        KeyCode::Left => {
                            self.cycle_style_profile_mode(false);
                            true
                        }
                        KeyCode::Right | KeyCode::Char(' ') => {
                            self.cycle_style_profile_mode(true);
                            true
                        }
                        _ => false,
                    },
                    Focus::StyleReferences => {
                        let before = self.style_references_field.text().to_string();
                        self.style_references_field.handle_key(key);
                        if self.style_references_field.text() != before {
                            self.style_references_dirty = true;
                        }
                        true
                    }
                    Focus::StyleSkillRoots => {
                        let before = self.style_skill_roots_field.text().to_string();
                        self.style_skill_roots_field.handle_key(key);
                        if self.style_skill_roots_field.text() != before {
                            self.style_skill_roots_dirty = true;
                        }
                        true
                    }
                    Focus::StyleMcpInclude => {
                        let before = self.style_mcp_include_field.text().to_string();
                        self.style_mcp_include_field.handle_key(key);
                        if self.style_mcp_include_field.text() != before {
                            self.style_mcp_include_dirty = true;
                        }
                        true
                    }
                    Focus::StyleMcpExclude => {
                        let before = self.style_mcp_exclude_field.text().to_string();
                        self.style_mcp_exclude_field.handle_key(key);
                        if self.style_mcp_exclude_field.text() != before {
                            self.style_mcp_exclude_dirty = true;
                        }
                        true
                    }
                    Focus::Examples => {
                        self.examples_field.handle_key(key);
                        true
                    }
                    Focus::Body => {
                        self.body_field.handle_key(key);
                        true
                    }
                    Focus::Generate | Focus::Save | Focus::Delete | Focus::Cancel => false,
                    Focus::List => self.handle_list_key(key),
                },
            },
        };

        if handled && matches!(self.mode, Mode::Edit) {
            self.ensure_edit_focus_visible_from_last_render();
        }

        handled
    }

    pub fn handle_paste_direct(&mut self, text: String) -> bool {
        if self.is_complete {
            return true;
        }

        if !matches!(self.mode, Mode::Edit) {
            return false;
        }

        match self.focus {
            Focus::Name => {
                self.name_field.handle_paste(text);
                true
            }
            Focus::Description => {
                self.description_field.handle_paste(text);
                true
            }
            Focus::Style => {
                let previous_style =
                    ShellScriptStyle::parse(self.style_field.text().trim());
                self.style_field.handle_paste(text);
                let next_style = ShellScriptStyle::parse(self.style_field.text().trim());
                if next_style != previous_style
                    && next_style.is_some()
                    && !self.style_profile_fields_dirty()
                {
                    self.set_style_resource_fields_from_profile(next_style);
                }
                true
            }
            Focus::StyleReferences => {
                let before = self.style_references_field.text().to_string();
                self.style_references_field.handle_paste(text);
                if self.style_references_field.text() != before {
                    self.style_references_dirty = true;
                }
                true
            }
            Focus::StyleSkillRoots => {
                let before = self.style_skill_roots_field.text().to_string();
                self.style_skill_roots_field.handle_paste(text);
                if self.style_skill_roots_field.text() != before {
                    self.style_skill_roots_dirty = true;
                }
                true
            }
            Focus::StyleMcpInclude => {
                let before = self.style_mcp_include_field.text().to_string();
                self.style_mcp_include_field.handle_paste(text);
                if self.style_mcp_include_field.text() != before {
                    self.style_mcp_include_dirty = true;
                }
                true
            }
            Focus::StyleMcpExclude => {
                let before = self.style_mcp_exclude_field.text().to_string();
                self.style_mcp_exclude_field.handle_paste(text);
                if self.style_mcp_exclude_field.text() != before {
                    self.style_mcp_exclude_dirty = true;
                }
                true
            }
            Focus::Examples => {
                self.examples_field.handle_paste(text);
                true
            }
            Focus::Body => {
                self.body_field.handle_paste(text);
                true
            }
            Focus::StyleProfile
            | Focus::Generate
            | Focus::Save
            | Focus::Delete
            | Focus::Cancel
            | Focus::List => false,
        }
    }

    fn scroll_edit_container_by(&mut self, delta: isize, max_scroll: usize) -> bool {
        if max_scroll == 0 || delta == 0 {
            return false;
        }
        let next = next_scroll_top_with_delta(self.edit_scroll_top, max_scroll, delta);
        if next == self.edit_scroll_top {
            false
        } else {
            self.edit_scroll_top = next;
            true
        }
    }

    fn ensure_edit_focus_visible(&mut self, layout: SkillsFormLayout) -> bool {
        if layout.max_scroll == 0 || layout.viewport_inner.height == 0 {
            return false;
        }
        let Some((focus_top, focus_h)) = layout.focus_bounds(self.focus) else {
            return false;
        };
        if focus_h == 0 {
            return false;
        }

        let viewport_h = layout.viewport_inner.height as usize;
        let next = scroll_top_to_keep_visible(
            self.edit_scroll_top,
            layout.max_scroll,
            viewport_h,
            focus_top,
            focus_h,
        );
        if next == self.edit_scroll_top {
            false
        } else {
            self.edit_scroll_top = next;
            true
        }
    }

    fn ensure_edit_focus_visible_from_last_render(&mut self) -> bool {
        let Some(area) = self.last_render_area.get() else {
            return false;
        };
        let Some(layout) = self.compute_form_layout(area) else {
            return false;
        };
        self.edit_scroll_top = layout.scroll_top;
        self.ensure_edit_focus_visible(layout)
    }

    pub fn handle_mouse_event_direct(&mut self, mouse_event: MouseEvent, area: Rect) -> bool {
        if self.mode == Mode::List {
            return self.handle_list_mouse_event(mouse_event, area);
        }

        match mouse_event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if !point_in_rect(area, mouse_event.column, mouse_event.row) {
                    return false;
                }
                self.handle_edit_click(mouse_event, area)
            }
            MouseEventKind::Moved => {
                if !point_in_rect(area, mouse_event.column, mouse_event.row) {
                    return self.set_hovered_button(None);
                }
                self.handle_edit_mouse_move(mouse_event, area)
            }
            MouseEventKind::ScrollUp => {
                if !point_in_rect(area, mouse_event.column, mouse_event.row) {
                    return false;
                }
                self.handle_edit_scroll(mouse_event, area, false)
            }
            MouseEventKind::ScrollDown => {
                if !point_in_rect(area, mouse_event.column, mouse_event.row) {
                    return false;
                }
                self.handle_edit_scroll(mouse_event, area, true)
            }
            _ => false,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.is_complete
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        self.last_render_area.set(Some(area));
        self.render_body(area, buf);
    }

    fn render_body(&self, area: Rect, buf: &mut Buffer) {
        match self.mode {
            Mode::List => self.render_list(area, buf),
            Mode::Edit => self.render_form(area, buf),
        }
    }

    fn render_list(&self, area: Rect, buf: &mut Buffer) {
        let mut lines: Vec<Line> = Vec::new();
        for (idx, skill) in self.skills.iter().enumerate() {
            let arrow = if idx == self.selected { ">" } else { " " };
            let name_style = if idx == self.selected {
                Style::default().fg(colors::primary()).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors::text())
            };
            let scope_text = match skill.scope {
                SkillScope::Repo => " [repo]",
                SkillScope::User => " [user]",
                SkillScope::System => " [system]",
            };
            let name_span = Span::styled(format!("{arrow} {name}", name = skill.name), name_style);
            let scope_span = Span::styled(scope_text, Style::default().fg(colors::text_dim()));
            let desc_span = Span::styled(
                format!("  {desc}", desc = skill.description),
                Style::default().fg(colors::text_dim()),
            );
            lines.push(Line::from(vec![name_span, scope_span, desc_span]));
        }
        if lines.is_empty() {
            lines.push(Line::from("No skills yet. Press Ctrl+N to create."));
        }

        let add_arrow = if self.selected == self.skills.len() { ">" } else { " " };
        let add_style = if self.selected == self.skills.len() {
            Style::default().fg(colors::primary()).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(colors::success()).add_modifier(Modifier::BOLD)
        };
        lines.push(Line::from(vec![Span::styled(
            format!("{add_arrow} Add new..."),
            add_style,
        )]));

        let title = Paragraph::new(vec![Line::from(Span::styled(
            "Skills are reusable instruction bundles stored as SKILL.md files. Use Enter to edit, Ctrl+N for guided create, and Ctrl+G in editor to generate a draft with per-style skill and resource overrides.",
            Style::default().fg(colors::text_dim()),
        ))])
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true })
        .style(Style::default().bg(colors::background()));

        let list = Paragraph::new(lines)
            .alignment(Alignment::Left)
            .style(Style::default().bg(colors::background()));

        let outer = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().bg(colors::background()));
        let inner = outer.inner(area);
        outer.render(area, buf);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(inner);

        title.render(chunks[0], buf);
        list.render(chunks[1], buf);
    }

    fn render_form(&self, area: Rect, buf: &mut Buffer) {
        let outer = Block::default()
            .borders(Borders::ALL)
            .title("Skill Creator / Editor")
            .style(Style::default().bg(colors::background()));
        outer.render(area, buf);
        let Some(layout) = self.compute_form_layout(area) else {
            return;
        };

        let label_style = Style::default().fg(colors::text_dim());

        render_labeled_row(
            buf,
            layout.viewport_inner,
            label_style,
            "Name (slug)",
            layout.name_field,
        );
        self.name_field
            .render(layout.name_field, buf, matches!(self.focus, Focus::Name));

        render_labeled_row(
            buf,
            layout.viewport_inner,
            label_style,
            "Description",
            layout.description_field,
        );
        self.description_field.render(
            layout.description_field,
            buf,
            matches!(self.focus, Focus::Description),
        );

        render_labeled_row(
            buf,
            layout.viewport_inner,
            label_style,
            "Shell style (optional)",
            layout.style_field,
        );
        self.style_field
            .render(layout.style_field, buf, matches!(self.focus, Focus::Style));

        render_labeled_row(
            buf,
            layout.viewport_inner,
            label_style,
            "Style profile behavior",
            layout.style_profile_row,
        );
        if layout.style_profile_row.height > 0 {
            let focused = matches!(self.focus, Focus::StyleProfile);
            let mode_style = if focused {
                Style::default()
                    .fg(colors::background())
                    .bg(colors::primary())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(colors::text())
                    .add_modifier(Modifier::BOLD)
            };
            let hint_style = Style::default().fg(colors::text_dim());
            let mode_text = self.style_profile_mode.label().to_string();
            let hint_text = if self.style_field.text().trim().is_empty() {
                "Set shell style first".to_string()
            } else {
                self.style_profile_mode.hint().to_string()
            };
            Paragraph::new(Line::from(vec![
                Span::styled(mode_text, mode_style),
                Span::raw("  "),
                Span::styled(hint_text, hint_style),
            ]))
            .render(layout.style_profile_row, buf);
        }

        let references_title = if self.style_references_dirty {
            "Style references [edited]"
        } else {
            "Style references"
        };
        let mut references_block = Block::default()
            .borders(Borders::ALL)
            .title(format!("{references_title} (one path per line)"));
        if matches!(self.focus, Focus::StyleReferences) {
            references_block = references_block.border_style(Style::default().fg(colors::primary()));
        }
        references_block.render(layout.style_references_outer, buf);
        self.style_references_field.render(
            layout.style_references_inner,
            buf,
            matches!(self.focus, Focus::StyleReferences),
        );

        let skill_roots_title = if self.style_skill_roots_dirty {
            "Style skill roots [edited]"
        } else {
            "Style skill roots"
        };
        let mut skill_roots_block = Block::default()
            .borders(Borders::ALL)
            .title(format!("{skill_roots_title} (one path per line)"));
        if matches!(self.focus, Focus::StyleSkillRoots) {
            skill_roots_block = skill_roots_block.border_style(Style::default().fg(colors::primary()));
        }
        skill_roots_block.render(layout.style_skill_roots_outer, buf);
        self.style_skill_roots_field.render(
            layout.style_skill_roots_inner,
            buf,
            matches!(self.focus, Focus::StyleSkillRoots),
        );

        let mcp_include_title = if self.style_mcp_include_dirty {
            "Style MCP include [edited]"
        } else {
            "Style MCP include"
        };
        let mut mcp_include_block = Block::default()
            .borders(Borders::ALL)
            .title(format!("{mcp_include_title} (one server per line)"));
        if matches!(self.focus, Focus::StyleMcpInclude) {
            mcp_include_block = mcp_include_block.border_style(Style::default().fg(colors::primary()));
        }
        mcp_include_block.render(layout.style_mcp_include_outer, buf);
        self.style_mcp_include_field.render(
            layout.style_mcp_include_inner,
            buf,
            matches!(self.focus, Focus::StyleMcpInclude),
        );

        let mcp_exclude_title = if self.style_mcp_exclude_dirty {
            "Style MCP exclude [edited]"
        } else {
            "Style MCP exclude"
        };
        let mut mcp_exclude_block = Block::default()
            .borders(Borders::ALL)
            .title(format!("{mcp_exclude_title} (one server per line)"));
        if matches!(self.focus, Focus::StyleMcpExclude) {
            mcp_exclude_block = mcp_exclude_block.border_style(Style::default().fg(colors::primary()));
        }
        mcp_exclude_block.render(layout.style_mcp_exclude_outer, buf);
        self.style_mcp_exclude_field.render(
            layout.style_mcp_exclude_inner,
            buf,
            matches!(self.focus, Focus::StyleMcpExclude),
        );

        let mut examples_block = Block::default()
            .borders(Borders::ALL)
            .title("Trigger Examples / User Requests");
        if matches!(self.focus, Focus::Examples) {
            examples_block = examples_block.border_style(Style::default().fg(colors::primary()));
        }
        examples_block.render(layout.examples_outer, buf);
        self.examples_field
            .render(layout.examples_inner, buf, matches!(self.focus, Focus::Examples));

        let mut body_block = Block::default().borders(Borders::ALL).title("SKILL.md Body");
        if matches!(self.focus, Focus::Body) {
            body_block = body_block.border_style(Style::default().fg(colors::primary()));
        }
        body_block.render(layout.body_outer, buf);
        self.body_field
            .render(layout.body_inner, buf, matches!(self.focus, Focus::Body));

        let generate_label = "Generate draft";
        let save_label = "Save";
        let delete_label = "Delete";
        let cancel_label = "Cancel";

        let btn_span = |label: &str, focus: Focus, color: Style| {
            if self.focus == focus {
                Span::styled(label.to_string(), color.bg(colors::primary()).fg(colors::background()))
            } else if self.hovered_button.map(ActionButton::focus) == Some(focus) {
                Span::styled(
                    label.to_string(),
                    color
                        .bg(colors::border())
                        .fg(colors::text())
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(label.to_string(), color)
            }
        };
        let line = Line::from(vec![
            btn_span(
                generate_label,
                Focus::Generate,
                Style::default().fg(colors::info()).add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            btn_span(save_label, Focus::Save, Style::default().fg(colors::success()).add_modifier(Modifier::BOLD)),
            Span::raw("   "),
            btn_span(delete_label, Focus::Delete, Style::default().fg(colors::error()).add_modifier(Modifier::BOLD)),
            Span::raw("   "),
            btn_span(cancel_label, Focus::Cancel, Style::default().fg(colors::text_dim()).add_modifier(Modifier::BOLD)),
            Span::raw("    Tab cycle - Enter activates - <-/-> mode - Ctrl+G generate"),
        ]);
        Paragraph::new(line).render(layout.buttons_row, buf);

        if layout.status_row.height > 0
            && let Some((msg, style)) = &self.status
        {
            Paragraph::new(Line::from(Span::styled(msg.clone(), *style)))
                .alignment(Alignment::Left)
                .render(layout.status_row, buf);
        }

        if layout.max_scroll > 0 {
            let viewport_len = layout.viewport_inner.height as usize;
            render_vertical_scrollbar(
                buf,
                layout.viewport_inner,
                layout.scroll_top,
                layout.max_scroll,
                viewport_len,
            );
        }
    }

    fn compute_form_layout(&self, area: Rect) -> Option<SkillsFormLayout> {
        if area.width == 0 || area.height == 0 {
            return None;
        }

        let outer = Block::default().borders(Borders::ALL);
        let inner = outer.inner(area);
        if inner.width == 0 || inner.height == 0 {
            return None;
        }

        const BASIC_ROW_H: usize = 3;
        const PATH_BLOCK_H: usize = 4;
        const PATH_BLOCK_H_FOCUSED: usize = 7;
        const EXAMPLES_H: usize = 5;
        const EXAMPLES_H_FOCUSED: usize = 8;
        const BODY_MIN_H: usize = 10;
        const BODY_MIN_H_FOCUSED: usize = 14;

        let footer_layout = split_pinned_footer_layout(inner, 1, 1, 4);
        let viewport_inner = footer_layout.viewport;
        let viewport_h = viewport_inner.height as usize;

        let low_height = viewport_h > 0 && viewport_h <= 24;
        let style_references_section_h = if low_height && matches!(self.focus, Focus::StyleReferences) {
            PATH_BLOCK_H_FOCUSED
        } else {
            PATH_BLOCK_H
        };
        let style_skill_roots_section_h = if low_height && matches!(self.focus, Focus::StyleSkillRoots) {
            PATH_BLOCK_H_FOCUSED
        } else {
            PATH_BLOCK_H
        };
        let style_mcp_include_section_h = if low_height && matches!(self.focus, Focus::StyleMcpInclude) {
            PATH_BLOCK_H_FOCUSED
        } else {
            PATH_BLOCK_H
        };
        let style_mcp_exclude_section_h = if low_height && matches!(self.focus, Focus::StyleMcpExclude) {
            PATH_BLOCK_H_FOCUSED
        } else {
            PATH_BLOCK_H
        };
        let examples_section_h = if low_height && matches!(self.focus, Focus::Examples) {
            EXAMPLES_H_FOCUSED
        } else {
            EXAMPLES_H
        };
        let body_min_h = if low_height && matches!(self.focus, Focus::Body) {
            BODY_MIN_H_FOCUSED
        } else {
            BODY_MIN_H
        };

        let static_without_body = (BASIC_ROW_H * 4)
            + style_references_section_h
            + style_skill_roots_section_h
            + style_mcp_include_section_h
            + style_mcp_exclude_section_h
            + examples_section_h;
        let base_total = static_without_body + body_min_h;
        let body_h = body_min_h + viewport_h.saturating_sub(base_total);
        let content_h = static_without_body + body_h;
        let max_scroll = content_h.saturating_sub(viewport_h);
        let scroll_top = self.edit_scroll_top.min(max_scroll);

        let mut top = 0usize;
        let mut next_section = |h: usize| {
            let section_top = top;
            top = top.saturating_add(h);
            (section_top, h)
        };

        let (name_top, name_h) = next_section(BASIC_ROW_H);
        let (description_top, description_h) = next_section(BASIC_ROW_H);
        let (style_top, style_h) = next_section(BASIC_ROW_H);
        let (style_profile_top, style_profile_h) = next_section(BASIC_ROW_H);
        let (style_references_top, style_references_h) = next_section(style_references_section_h);
        let (style_skill_roots_top, style_skill_roots_h) = next_section(style_skill_roots_section_h);
        let (style_mcp_include_top, style_mcp_include_h) = next_section(style_mcp_include_section_h);
        let (style_mcp_exclude_top, style_mcp_exclude_h) = next_section(style_mcp_exclude_section_h);
        let (examples_top, examples_h) = next_section(examples_section_h);
        let (body_top, body_h_section) = next_section(body_h);

        let name_row = clipped_vertical_rect_with_scroll(viewport_inner, name_top, name_h, scroll_top);
        let description_row = clipped_vertical_rect_with_scroll(
            viewport_inner,
            description_top,
            description_h,
            scroll_top,
        );
        let style_row = clipped_vertical_rect_with_scroll(viewport_inner, style_top, style_h, scroll_top);
        let style_profile_row_full = clipped_vertical_rect_with_scroll(
            viewport_inner,
            style_profile_top,
            style_profile_h,
            scroll_top,
        );
        let style_references_outer =
            clipped_vertical_rect_with_scroll(viewport_inner, style_references_top, style_references_h, scroll_top);
        let style_skill_roots_outer =
            clipped_vertical_rect_with_scroll(viewport_inner, style_skill_roots_top, style_skill_roots_h, scroll_top);
        let style_mcp_include_outer =
            clipped_vertical_rect_with_scroll(viewport_inner, style_mcp_include_top, style_mcp_include_h, scroll_top);
        let style_mcp_exclude_outer =
            clipped_vertical_rect_with_scroll(viewport_inner, style_mcp_exclude_top, style_mcp_exclude_h, scroll_top);
        let examples_outer =
            clipped_vertical_rect_with_scroll(viewport_inner, examples_top, examples_h, scroll_top);
        let body_outer = clipped_vertical_rect_with_scroll(viewport_inner, body_top, body_h_section, scroll_top);

        let buttons_row = footer_layout.action_row;
        let status_row = footer_layout.status_row;

        let split_row = |row: Rect| {
            Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(24), Constraint::Min(1)])
                .split(row)
        };

        let name_chunks = split_row(name_row);
        let description_chunks = split_row(description_row);
        let style_chunks = split_row(style_row);
        let style_profile_chunks = split_row(style_profile_row_full);

        Some(SkillsFormLayout {
            viewport_inner,
            scroll_top,
            max_scroll,
            status_row,
            name_field: name_chunks[1],
            name_top,
            name_h,
            description_field: description_chunks[1],
            description_top,
            description_h,
            style_field: style_chunks[1],
            style_top,
            style_h,
            style_profile_row: style_profile_chunks[1],
            style_profile_top,
            style_profile_h,
            style_references_outer,
            style_references_inner: Block::default().borders(Borders::ALL).inner(style_references_outer),
            style_references_top,
            style_references_h,
            style_skill_roots_outer,
            style_skill_roots_inner: Block::default().borders(Borders::ALL).inner(style_skill_roots_outer),
            style_skill_roots_top,
            style_skill_roots_h,
            style_mcp_include_outer,
            style_mcp_include_inner: Block::default().borders(Borders::ALL).inner(style_mcp_include_outer),
            style_mcp_include_top,
            style_mcp_include_h,
            style_mcp_exclude_outer,
            style_mcp_exclude_inner: Block::default().borders(Borders::ALL).inner(style_mcp_exclude_outer),
            style_mcp_exclude_top,
            style_mcp_exclude_h,
            examples_outer,
            examples_inner: Block::default().borders(Borders::ALL).inner(examples_outer),
            examples_top,
            examples_h,
            body_outer,
            body_inner: Block::default().borders(Borders::ALL).inner(body_outer),
            body_top,
            body_h: body_h_section,
            buttons_row,
        })
    }

    fn list_selection_at(&self, area: Rect, x: u16, y: u16) -> Option<usize> {
        let outer = Block::default().borders(Borders::ALL);
        let inner = outer.inner(area);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(1)])
            .split(inner);
        let list_area = chunks[1];
        if !point_in_rect(list_area, x, y) {
            return None;
        }

        let row = y.saturating_sub(list_area.y) as usize;
        if row < self.skills.len() {
            return Some(row);
        }
        if row == self.skills.len() {
            return Some(self.skills.len());
        }
        None
    }

    fn handle_list_mouse_event(&mut self, mouse_event: MouseEvent, area: Rect) -> bool {
        let mut selected = self.selected;
        let result = route_selectable_list_mouse_with_config(
            mouse_event,
            &mut selected,
            self.skills.len().saturating_add(1),
            |x, y| self.list_selection_at(area, x, y),
            SelectableListMouseConfig {
                require_pointer_hit_for_scroll: true,
                scroll_behavior: ScrollSelectionBehavior::Clamp,
                ..SelectableListMouseConfig::default()
            },
        );
        self.selected = selected;
        if matches!(result, SelectableListMouseResult::Activated) {
            if self.selected < self.skills.len() {
                self.enter_editor();
            } else {
                self.start_new_skill();
            }
        }
        result.handled()
    }

    fn handle_edit_click(&mut self, mouse_event: MouseEvent, area: Rect) -> bool {
        let Some(layout) = self.compute_form_layout(area) else {
            return false;
        };
        self.set_hovered_button(None);

        if point_in_rect(layout.name_field, mouse_event.column, mouse_event.row) {
            self.focus = Focus::Name;
            self.name_field
                .handle_mouse_click(mouse_event.column, mouse_event.row, layout.name_field);
            return true;
        }
        if point_in_rect(layout.description_field, mouse_event.column, mouse_event.row) {
            self.focus = Focus::Description;
            self.description_field.handle_mouse_click(
                mouse_event.column,
                mouse_event.row,
                layout.description_field,
            );
            return true;
        }
        if point_in_rect(layout.style_field, mouse_event.column, mouse_event.row) {
            self.focus = Focus::Style;
            self.style_field
                .handle_mouse_click(mouse_event.column, mouse_event.row, layout.style_field);
            return true;
        }
        if point_in_rect(layout.style_profile_row, mouse_event.column, mouse_event.row) {
            self.focus = Focus::StyleProfile;
            self.cycle_style_profile_mode(true);
            return true;
        }
        if point_in_rect(layout.style_references_outer, mouse_event.column, mouse_event.row) {
            self.focus = Focus::StyleReferences;
            if point_in_rect(layout.style_references_inner, mouse_event.column, mouse_event.row) {
                self.style_references_field.handle_mouse_click(
                    mouse_event.column,
                    mouse_event.row,
                    layout.style_references_inner,
                );
            }
            return true;
        }
        if point_in_rect(layout.style_skill_roots_outer, mouse_event.column, mouse_event.row) {
            self.focus = Focus::StyleSkillRoots;
            if point_in_rect(layout.style_skill_roots_inner, mouse_event.column, mouse_event.row) {
                self.style_skill_roots_field.handle_mouse_click(
                    mouse_event.column,
                    mouse_event.row,
                    layout.style_skill_roots_inner,
                );
            }
            return true;
        }
        if point_in_rect(layout.style_mcp_include_outer, mouse_event.column, mouse_event.row) {
            self.focus = Focus::StyleMcpInclude;
            if point_in_rect(layout.style_mcp_include_inner, mouse_event.column, mouse_event.row) {
                self.style_mcp_include_field.handle_mouse_click(
                    mouse_event.column,
                    mouse_event.row,
                    layout.style_mcp_include_inner,
                );
            }
            return true;
        }
        if point_in_rect(layout.style_mcp_exclude_outer, mouse_event.column, mouse_event.row) {
            self.focus = Focus::StyleMcpExclude;
            if point_in_rect(layout.style_mcp_exclude_inner, mouse_event.column, mouse_event.row) {
                self.style_mcp_exclude_field.handle_mouse_click(
                    mouse_event.column,
                    mouse_event.row,
                    layout.style_mcp_exclude_inner,
                );
            }
            return true;
        }
        if point_in_rect(layout.examples_outer, mouse_event.column, mouse_event.row) {
            self.focus = Focus::Examples;
            if point_in_rect(layout.examples_inner, mouse_event.column, mouse_event.row) {
                self.examples_field.handle_mouse_click(
                    mouse_event.column,
                    mouse_event.row,
                    layout.examples_inner,
                );
            }
            return true;
        }
        if point_in_rect(layout.body_outer, mouse_event.column, mouse_event.row) {
            self.focus = Focus::Body;
            if point_in_rect(layout.body_inner, mouse_event.column, mouse_event.row) {
                self.body_field.handle_mouse_click(
                    mouse_event.column,
                    mouse_event.row,
                    layout.body_inner,
                );
            }
            return true;
        }

        self.handle_edit_button_click(mouse_event, layout.buttons_row)
    }

    fn handle_edit_mouse_move(&mut self, mouse_event: MouseEvent, area: Rect) -> bool {
        let Some(layout) = self.compute_form_layout(area) else {
            return self.set_hovered_button(None);
        };
        self.set_hovered_button(self.edit_button_at(
            mouse_event.column,
            mouse_event.row,
            layout.buttons_row,
        ))
    }

    fn handle_edit_scroll(&mut self, mouse_event: MouseEvent, area: Rect, scroll_down: bool) -> bool {
        let Some(layout) = self.compute_form_layout(area) else {
            return false;
        };
        let container_delta = if scroll_down { 3 } else { -3 };

        if point_in_rect(layout.style_references_outer, mouse_event.column, mouse_event.row) {
            let previous_focus = self.focus;
            self.focus = Focus::StyleReferences;
            let moved = self.style_references_field.handle_mouse_scroll(scroll_down);
            return moved
                || previous_focus != self.focus
                || self.scroll_edit_container_by(container_delta, layout.max_scroll);
        }
        if point_in_rect(layout.style_skill_roots_outer, mouse_event.column, mouse_event.row) {
            let previous_focus = self.focus;
            self.focus = Focus::StyleSkillRoots;
            let moved = self.style_skill_roots_field.handle_mouse_scroll(scroll_down);
            return moved
                || previous_focus != self.focus
                || self.scroll_edit_container_by(container_delta, layout.max_scroll);
        }
        if point_in_rect(layout.style_mcp_include_outer, mouse_event.column, mouse_event.row) {
            let previous_focus = self.focus;
            self.focus = Focus::StyleMcpInclude;
            let moved = self.style_mcp_include_field.handle_mouse_scroll(scroll_down);
            return moved
                || previous_focus != self.focus
                || self.scroll_edit_container_by(container_delta, layout.max_scroll);
        }
        if point_in_rect(layout.style_mcp_exclude_outer, mouse_event.column, mouse_event.row) {
            let previous_focus = self.focus;
            self.focus = Focus::StyleMcpExclude;
            let moved = self.style_mcp_exclude_field.handle_mouse_scroll(scroll_down);
            return moved
                || previous_focus != self.focus
                || self.scroll_edit_container_by(container_delta, layout.max_scroll);
        }
        if point_in_rect(layout.examples_outer, mouse_event.column, mouse_event.row) {
            let previous_focus = self.focus;
            self.focus = Focus::Examples;
            let moved = self.examples_field.handle_mouse_scroll(scroll_down);
            return moved
                || previous_focus != self.focus
                || self.scroll_edit_container_by(container_delta, layout.max_scroll);
        }
        if point_in_rect(layout.body_outer, mouse_event.column, mouse_event.row) {
            let previous_focus = self.focus;
            self.focus = Focus::Body;
            let moved = self.body_field.handle_mouse_scroll(scroll_down);
            return moved
                || previous_focus != self.focus
                || self.scroll_edit_container_by(container_delta, layout.max_scroll);
        }

        self.scroll_edit_container_by(container_delta, layout.max_scroll)
    }

    fn handle_edit_button_click(&mut self, mouse_event: MouseEvent, row: Rect) -> bool {
        let Some(button) = self.edit_button_at(mouse_event.column, mouse_event.row, row) else {
            return false;
        };
        self.set_hovered_button(Some(button));

        match button {
            ActionButton::Generate => {
                self.focus = Focus::Generate;
                self.generate_draft();
            }
            ActionButton::Save => {
                self.focus = Focus::Save;
                self.save_current();
            }
            ActionButton::Delete => {
                self.focus = Focus::Delete;
                self.delete_current();
            }
            ActionButton::Cancel => {
                self.focus = Focus::Cancel;
                self.mode = Mode::List;
                self.focus = Focus::List;
                self.hovered_button = None;
                self.status = None;
            }
        }
        true
    }

    fn edit_button_at(&self, x: u16, y: u16, row: Rect) -> Option<ActionButton> {
        if !point_in_rect(row, x, y) {
            return None;
        }
        if y < row.y || y >= row.y.saturating_add(row.height) {
            return None;
        }

        let mut cursor_x = row.x;
        let generate_len = "Generate draft".len() as u16;
        if x >= cursor_x && x < cursor_x.saturating_add(generate_len) {
            return Some(ActionButton::Generate);
        }
        cursor_x = cursor_x.saturating_add(generate_len + 3);

        let save_len = "Save".len() as u16;
        if x >= cursor_x && x < cursor_x.saturating_add(save_len) {
            return Some(ActionButton::Save);
        }
        cursor_x = cursor_x.saturating_add(save_len + 3);

        let delete_len = "Delete".len() as u16;
        if x >= cursor_x && x < cursor_x.saturating_add(delete_len) {
            return Some(ActionButton::Delete);
        }
        cursor_x = cursor_x.saturating_add(delete_len + 3);

        let cancel_len = "Cancel".len() as u16;
        if x >= cursor_x && x < cursor_x.saturating_add(cancel_len) {
            return Some(ActionButton::Cancel);
        }
        None
    }

    fn set_hovered_button(&mut self, hovered: Option<ActionButton>) -> bool {
        if self.hovered_button == hovered {
            return false;
        }
        self.hovered_button = hovered;
        true
    }

    fn handle_list_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
                return true;
            }
            KeyCode::Down => {
                let max = self.skills.len();
                if self.selected < max {
                    self.selected += 1;
                }
                return true;
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.start_new_skill();
                return true;
            }
            _ => {}
        }
        false
    }

    fn start_new_skill(&mut self) {
        self.selected = self.skills.len();
        self.name_field.set_text("");
        self.description_field.set_text("");
        self.style_field.set_text("");
        self.set_style_resource_fields_from_profile(None);
        self.style_profile_mode = StyleProfileMode::Inherit;
        self.examples_field.set_text("");
        self.body_field.set_text("");
        self.focus = Focus::Name;
        self.edit_scroll_top = 0;
        self.hovered_button = None;
        self.status = Some((
            "New skill. Fill fields, then Generate draft or Save.".to_string(),
            Style::default().fg(colors::info()),
        ));
        self.mode = Mode::Edit;
    }

    fn load_selected_into_form(&mut self) {
        if let Some(skill) = self.skills.get(self.selected).cloned() {
            let slug = skill_slug(&skill);
            self.name_field.set_text(&slug);
            self.description_field
                .set_text(&frontmatter_value(&skill.content, "description").unwrap_or_default());
            let shell_style = frontmatter_value(&skill.content, "shell_style").unwrap_or_default();
            self.style_field.set_text(&shell_style);
            self.style_profile_mode = self.infer_style_profile_mode(&shell_style, &slug, &skill.name);
            self.set_style_resource_fields_from_profile(ShellScriptStyle::parse(&shell_style));
            self.examples_field.set_text("");
            self.body_field.set_text(&strip_frontmatter(&skill.content));
            self.focus = Focus::Name;
            self.edit_scroll_top = 0;
            self.hovered_button = None;
        }
    }

    fn enter_editor(&mut self) {
        if self.selected >= self.skills.len() {
            self.start_new_skill();
        } else {
            self.load_selected_into_form();
            self.mode = Mode::Edit;
        }
    }

    fn cycle_focus(&mut self, forward: bool) {
        let order = [
            Focus::Name,
            Focus::Description,
            Focus::Style,
            Focus::StyleProfile,
            Focus::StyleReferences,
            Focus::StyleSkillRoots,
            Focus::StyleMcpInclude,
            Focus::StyleMcpExclude,
            Focus::Examples,
            Focus::Body,
            Focus::Generate,
            Focus::Save,
            Focus::Delete,
            Focus::Cancel,
        ];
        let mut idx = order.iter().position(|f| *f == self.focus).unwrap_or(0);
        if forward {
            idx = (idx + 1) % order.len();
        } else {
            idx = idx.checked_sub(1).unwrap_or(order.len() - 1);
        }
        self.focus = order[idx];
    }

    fn cycle_style_profile_mode(&mut self, forward: bool) {
        self.style_profile_mode = if forward {
            self.style_profile_mode.next()
        } else {
            self.style_profile_mode.previous()
        };
    }

    fn style_resource_paths_dirty(&self) -> bool {
        self.style_references_dirty || self.style_skill_roots_dirty
    }

    fn style_mcp_filters_dirty(&self) -> bool {
        self.style_mcp_include_dirty || self.style_mcp_exclude_dirty
    }

    fn style_profile_fields_dirty(&self) -> bool {
        self.style_resource_paths_dirty() || self.style_mcp_filters_dirty()
    }

    fn set_style_resource_fields_from_profile(&mut self, style: Option<ShellScriptStyle>) {
        let (references, skill_roots, mcp_include, mcp_exclude) = if let Some(style) = style {
            if let Some(profile) = self.shell_style_profiles.get(&style) {
                (
                    profile.references.clone(),
                    profile.skill_roots.clone(),
                    profile.mcp_servers.include.clone(),
                    profile.mcp_servers.exclude.clone(),
                )
            } else {
                (Vec::new(), Vec::new(), Vec::new(), Vec::new())
            }
        } else {
            (Vec::new(), Vec::new(), Vec::new(), Vec::new())
        };

        self.style_references_field
            .set_text(&format_path_list(&references));
        self.style_skill_roots_field
            .set_text(&format_path_list(&skill_roots));
        self.style_mcp_include_field
            .set_text(&format_string_list(&mcp_include));
        self.style_mcp_exclude_field
            .set_text(&format_string_list(&mcp_exclude));
        self.style_references_dirty = false;
        self.style_skill_roots_dirty = false;
        self.style_mcp_include_dirty = false;
        self.style_mcp_exclude_dirty = false;
    }

    fn infer_style_profile_mode(
        &self,
        shell_style: &str,
        slug: &str,
        display_name: &str,
    ) -> StyleProfileMode {
        let Some(style) = ShellScriptStyle::parse(shell_style) else {
            return StyleProfileMode::Inherit;
        };

        let Some(profile) = self.shell_style_profiles.get(&style) else {
            return StyleProfileMode::Inherit;
        };

        let identifiers = [slug, display_name];
        if profile_list_contains_any(&profile.disabled_skills, &identifiers) {
            return StyleProfileMode::Disable;
        }
        if profile_list_contains_any(&profile.skills, &identifiers) {
            return StyleProfileMode::Enable;
        }
        StyleProfileMode::Inherit
    }

    fn parse_shell_style(&self, shell_style_raw: &str) -> Result<Option<ShellScriptStyle>, String> {
        let trimmed = shell_style_raw.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        ShellScriptStyle::parse(trimmed)
            .ok_or_else(|| "Invalid shell style. Use: posix-sh, bash-zsh-compatible, or zsh.".to_string())
            .map(Some)
    }

    fn persist_style_profile_mode(
        &mut self,
        code_home: &std::path::Path,
        style: Option<ShellScriptStyle>,
        skill_name: &str,
        aliases: &[String],
    ) -> Result<(), String> {
        if style.is_none() && self.style_profile_mode != StyleProfileMode::Inherit {
            return Err("Style profile behavior requires a shell style value.".to_string());
        }

        let Some(style) = style else {
            return Ok(());
        };

        let mut identifiers: Vec<&str> = Vec::new();
        identifiers.push(skill_name);
        for alias in aliases {
            identifiers.push(alias);
        }
        let deduped_identifiers = unique_profile_identifiers(identifiers);

        for identifier in &deduped_identifiers {
            set_shell_style_profile_skill_mode(
                code_home,
                style,
                identifier,
                ShellStyleSkillMode::Inherit,
            )
            .map_err(|err| format!("Failed to update shell_style_profiles: {err}"))?;
        }

        if self.style_profile_mode != StyleProfileMode::Inherit {
            set_shell_style_profile_skill_mode(
                code_home,
                style,
                skill_name,
                self.style_profile_mode.into_config_mode(),
            )
            .map_err(|err| format!("Failed to update shell_style_profiles: {err}"))?;
        }

        let profile = self.shell_style_profiles.entry(style).or_default();
        for identifier in &deduped_identifiers {
            remove_profile_skill(&mut profile.skills, identifier);
            remove_profile_skill(&mut profile.disabled_skills, identifier);
        }
        if self.style_profile_mode == StyleProfileMode::Enable {
            profile.skills.push(skill_name.trim().to_string());
        }
        if self.style_profile_mode == StyleProfileMode::Disable {
            profile.disabled_skills.push(skill_name.trim().to_string());
        }
        Ok(())
    }

    fn persist_style_profile_paths(
        &mut self,
        code_home: &std::path::Path,
        style: Option<ShellScriptStyle>,
    ) -> Result<(), String> {
        if !self.style_resource_paths_dirty() {
            return Ok(());
        }

        let references = parse_path_list(self.style_references_field.text());
        let skill_roots = parse_path_list(self.style_skill_roots_field.text());

        let Some(style) = style else {
            if references.is_empty() && skill_roots.is_empty() {
                self.style_references_dirty = false;
                self.style_skill_roots_dirty = false;
                return Ok(());
            }
            return Err("Style references/skill roots require a shell style value.".to_string());
        };

        set_shell_style_profile_paths(code_home, style, &references, &skill_roots)
            .map_err(|err| format!("Failed to update shell_style_profiles paths: {err}"))?;

        let should_remove = {
            let profile = self.shell_style_profiles.entry(style).or_default();
            profile.references = references;
            profile.skill_roots = skill_roots;
            style_profile_is_empty(profile)
        };
        if should_remove {
            self.shell_style_profiles.remove(&style);
        }

        self.style_references_dirty = false;
        self.style_skill_roots_dirty = false;
        Ok(())
    }

    fn persist_style_profile_mcp_servers(
        &mut self,
        code_home: &std::path::Path,
        style: Option<ShellScriptStyle>,
    ) -> Result<(), String> {
        if !self.style_mcp_filters_dirty() {
            return Ok(());
        }

        let include = parse_string_list(self.style_mcp_include_field.text());
        let exclude = parse_string_list(self.style_mcp_exclude_field.text());

        let Some(style) = style else {
            if include.is_empty() && exclude.is_empty() {
                self.style_mcp_include_dirty = false;
                self.style_mcp_exclude_dirty = false;
                return Ok(());
            }
            return Err("Style MCP include/exclude requires a shell style value.".to_string());
        };

        set_shell_style_profile_mcp_servers(code_home, style, &include, &exclude)
            .map_err(|err| format!("Failed to update shell_style_profiles mcp_servers: {err}"))?;

        let should_remove = {
            let profile = self.shell_style_profiles.entry(style).or_default();
            profile.mcp_servers.include = include;
            profile.mcp_servers.exclude = exclude;
            style_profile_is_empty(profile)
        };
        if should_remove {
            self.shell_style_profiles.remove(&style);
        }

        self.style_mcp_include_dirty = false;
        self.style_mcp_exclude_dirty = false;
        Ok(())
    }

    fn validate_name(&self, name: &str) -> Result<(), String> {
        let slug = name.trim();
        if slug.is_empty() {
            return Err("Name is required".to_string());
        }
        if !slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err("Name must use letters, numbers, '-', '_' or '.'".to_string());
        }

        let dup = self
            .skills
            .iter()
            .enumerate()
            .any(|(idx, skill)| idx != self.selected && skill_slug(skill).eq_ignore_ascii_case(slug));
        if dup {
            return Err("A skill with this name already exists".to_string());
        }
        Ok(())
    }

    fn validate_frontmatter(&self, body: &str) -> Result<(), String> {
        if extract_frontmatter(body).is_none() {
            return Err("SKILL.md must start with YAML frontmatter".to_string());
        }
        if frontmatter_value(body, "name").is_none() {
            return Err("Frontmatter must include name".to_string());
        }
        if frontmatter_value(body, "description").is_none() {
            return Err("Frontmatter must include description".to_string());
        }
        Ok(())
    }

    fn validate_description(&self, description: &str) -> Result<(), String> {
        if description.trim().is_empty() {
            return Err("Description is required".to_string());
        }
        Ok(())
    }

    fn generate_draft(&mut self) {
        let name = self.name_field.text().trim().to_string();
        let description = self.description_field.text().trim().to_string();
        if let Err(msg) = self.validate_name(&name) {
            self.status = Some((msg, Style::default().fg(colors::error())));
            return;
        }
        if let Err(msg) = self.validate_description(&description) {
            self.status = Some((msg, Style::default().fg(colors::error())));
            return;
        }

        let shell_style = self.style_field.text().trim();
        let trigger_examples = self.examples_field.text().trim();
        let title = name.replace('-', " ");

        let mut body = format!(
            "# {title}\n\n## Purpose\n\n{description}\n\n## Workflow\n\n1. Describe the first deterministic step.\n2. Describe conditional branches and constraints.\n3. Point to scripts/references/assets when needed.\n"
        );

        if !trigger_examples.is_empty() {
            body.push_str("\n## Trigger Examples\n\n");
            body.push_str(trigger_examples);
            body.push('\n');
        }

        if !shell_style.is_empty() {
            body.push_str("\n## Shell Style Integration\n\n");
            body.push_str(
                "This skill is intended for shell-style-aware loading. Configure it under `shell_style_profiles` when appropriate.\n\n",
            );
            body.push_str(&format!(
                "- Preferred shell style: `{shell_style}`\n- Consider wiring via `shell_style_profiles.{shell_style}.skill_roots`\n"
            ));
        }

        self.body_field.set_text(&body);
        self.status = Some((
            "Draft generated from guided fields. Review and Save.".to_string(),
            Style::default().fg(colors::success()),
        ));
    }

    fn save_current(&mut self) {
        if let Some(skill) = self.skills.get(self.selected)
            && skill.scope != SkillScope::User {
                self.status = Some((
                    "Only user skills can be saved".to_string(),
                    Style::default().fg(colors::error()),
                ));
                return;
            }

        let existing_skill = self.skills.get(self.selected).cloned();

        let name = self.name_field.text().trim().to_string();
        let description = self.description_field.text().trim().to_string();
        let shell_style_raw = self.style_field.text().trim().to_string();
        let trigger_examples = self.examples_field.text().trim().to_string();
        let body = self.body_field.text().to_string();
        if let Err(msg) = self.validate_name(&name) {
            self.status = Some((msg, Style::default().fg(colors::error())));
            return;
        }
        if let Err(msg) = self.validate_description(&description) {
            self.status = Some((msg, Style::default().fg(colors::error())));
            return;
        }
        let parsed_shell_style = match self.parse_shell_style(&shell_style_raw) {
            Ok(style) => style,
            Err(msg) => {
                self.status = Some((msg, Style::default().fg(colors::error())));
                return;
            }
        };
        if parsed_shell_style.is_none() && self.style_profile_mode != StyleProfileMode::Inherit {
            self.status = Some((
                "Style profile behavior requires a shell style value.".to_string(),
                Style::default().fg(colors::error()),
            ));
            return;
        }
        if parsed_shell_style.is_none() && self.style_resource_paths_dirty() {
            let references = parse_path_list(self.style_references_field.text());
            let skill_roots = parse_path_list(self.style_skill_roots_field.text());
            if !references.is_empty() || !skill_roots.is_empty() {
                self.status = Some((
                    "Style references/skill roots require a shell style value.".to_string(),
                    Style::default().fg(colors::error()),
                ));
                return;
            }
        }
        if parsed_shell_style.is_none() && self.style_mcp_filters_dirty() {
            let mcp_include = parse_string_list(self.style_mcp_include_field.text());
            let mcp_exclude = parse_string_list(self.style_mcp_exclude_field.text());
            if !mcp_include.is_empty() || !mcp_exclude.is_empty() {
                self.status = Some((
                    "Style MCP include/exclude requires a shell style value.".to_string(),
                    Style::default().fg(colors::error()),
                ));
                return;
            }
        }
        let shell_style = parsed_shell_style
            .map(|style| style.to_string())
            .unwrap_or_default();

        let mut document_body = strip_frontmatter(&body);
        let extra_frontmatter = extract_frontmatter(&body)
            .map(|frontmatter| {
                filter_frontmatter_excluding_keys(
                    frontmatter.as_str(),
                    &["name", "description", "shell_style"],
                )
            })
            .unwrap_or_default();
        if !trigger_examples.is_empty() && !document_body.contains("## Trigger Examples") {
            document_body.push_str("\n\n## Trigger Examples\n\n");
            document_body.push_str(&trigger_examples);
            document_body.push('\n');
        }

        let body = compose_skill_document(
            &name,
            &description,
            &shell_style,
            &extra_frontmatter,
            &document_body,
        );
        if let Err(msg) = self.validate_frontmatter(&body) {
            self.status = Some((msg, Style::default().fg(colors::error())));
            return;
        }

        let code_home = match find_code_home() {
            Ok(path) => path,
            Err(err) => {
                self.status = Some((
                    format!("CODE_HOME unavailable: {err}"),
                    Style::default().fg(colors::error()),
                ));
                return;
            }
        };
        let mut dir = code_home.clone();
        dir.push("skills");
        dir.push(&name);
        if let Err(err) = fs::create_dir_all(&dir) {
            self.status = Some((
                format!("Failed to create skill dir: {err}"),
                Style::default().fg(colors::error()),
            ));
            return;
        }
        let mut path = dir;
        path.push("SKILL.md");
        if let Err(err) = fs::write(&path, &body) {
            self.status = Some((
                format!("Failed to save: {err}"),
                Style::default().fg(colors::error()),
            ));
            return;
        }

        self.style_field.set_text(&shell_style);

        let mut profile_warning: Option<String> = None;
        let mut style_profile_aliases: Vec<String> = Vec::new();
        if let Some(previous_skill) = existing_skill.as_ref() {
            let previous_name = skill_slug(previous_skill);
            style_profile_aliases.push(previous_name.clone());
            style_profile_aliases.push(previous_skill.name.clone());
            let previous_style = frontmatter_value(&previous_skill.content, "shell_style")
                .and_then(|value| ShellScriptStyle::parse(&value));
            let changed_identity = previous_name != name || previous_style != parsed_shell_style;
            if changed_identity
                && let Some(previous_style) = previous_style {
                    let previous_identifiers =
                        unique_profile_identifiers([previous_name.as_str(), previous_skill.name.as_str()]);
                    for identifier in &previous_identifiers {
                        if let Err(err) = set_shell_style_profile_skill_mode(
                            &code_home,
                            previous_style,
                            identifier,
                            ShellStyleSkillMode::Inherit,
                        ) {
                            append_warning(
                                &mut profile_warning,
                                format!(
                                    "Failed to clear previous style profile mapping: {err}"
                                ),
                            );
                            continue;
                        }
                        if let Some(profile) = self.shell_style_profiles.get_mut(&previous_style) {
                            remove_profile_skill(&mut profile.skills, identifier);
                            remove_profile_skill(&mut profile.disabled_skills, identifier);
                        }
                    }
                }
        }

        if let Err(msg) = self.persist_style_profile_mode(
            &code_home,
            parsed_shell_style,
            &name,
            &style_profile_aliases,
        ) {
            append_warning(&mut profile_warning, msg);
        }
        if let Err(msg) = self.persist_style_profile_paths(&code_home, parsed_shell_style) {
            append_warning(&mut profile_warning, msg);
        }
        if let Err(msg) = self.persist_style_profile_mcp_servers(&code_home, parsed_shell_style) {
            append_warning(&mut profile_warning, msg);
        }

        if let Some(previous_skill) = existing_skill.as_ref()
            && previous_skill.path != path
            && previous_skill.scope == SkillScope::User
        {
            if let Err(err) = fs::remove_file(&previous_skill.path)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                append_warning(
                    &mut profile_warning,
                    format!("Failed to remove previous file: {err}"),
                );
            }
            if let Some(parent) = previous_skill.path.parent() {
                let _ = fs::remove_dir(parent);
            }
        }

        let display_name = name.clone();

        let mut updated = self.skills.clone();
        let new_entry = Skill {
            name: display_name,
            path,
            description,
            scope: SkillScope::User,
            content: body,
        };
        if self.selected < updated.len() {
            updated[self.selected] = new_entry;
        } else {
            updated.push(new_entry);
            self.selected = updated.len() - 1;
        }
        self.skills = updated;
        self.status = Some(match profile_warning {
            Some(msg) => (format!("Saved skill with warnings: {msg}"), Style::default().fg(colors::warning())),
            None => ("Saved.".to_string(), Style::default().fg(colors::success())),
        });

        self.app_event_tx.send(AppEvent::CodexOp(Op::ListSkills));
    }

    fn delete_current(&mut self) {
        if self.selected >= self.skills.len() {
            self.status = Some(("Nothing to delete".to_string(), Style::default().fg(colors::warning())));
            self.mode = Mode::List;
            self.focus = Focus::List;
            return;
        }
        let skill = self.skills[self.selected].clone();
        if skill.scope != SkillScope::User {
            self.status = Some((
                "Only user skills can be deleted".to_string(),
                Style::default().fg(colors::error()),
            ));
            return;
        }

        if let Err(err) = fs::remove_file(&skill.path)
            && err.kind() != std::io::ErrorKind::NotFound {
                self.status = Some((
                    format!("Delete failed: {err}"),
                    Style::default().fg(colors::error()),
                ));
                return;
            }

        if let Some(parent) = skill.path.parent() {
            let _ = fs::remove_dir(parent);
        }

        if self.selected < self.skills.len() {
            self.skills.remove(self.selected);
            if self.selected >= self.skills.len() && !self.skills.is_empty() {
                self.selected = self.skills.len() - 1;
            }
        }

        let mut delete_warning: Option<String> = None;
        let deleted_skill_name = skill_slug(&skill);
        if let Some(style) = frontmatter_value(&skill.content, "shell_style")
            .and_then(|value| ShellScriptStyle::parse(&value))
            && let Ok(code_home) = find_code_home()
        {
            let identifiers =
                unique_profile_identifiers([deleted_skill_name.as_str(), skill.name.as_str()]);
            for identifier in &identifiers {
                match set_shell_style_profile_skill_mode(
                    &code_home,
                    style,
                    identifier,
                    ShellStyleSkillMode::Inherit,
                ) {
                    Ok(_) => {
                        if let Some(profile) = self.shell_style_profiles.get_mut(&style) {
                            remove_profile_skill(&mut profile.skills, identifier);
                            remove_profile_skill(&mut profile.disabled_skills, identifier);
                        }
                    }
                    Err(err) => append_warning(
                        &mut delete_warning,
                        format!("Failed to clear style profile mapping: {err}"),
                    ),
                }
            }
        }

        self.mode = Mode::List;
        self.focus = Focus::List;
        self.status = Some(match delete_warning {
            Some(msg) => (
                format!("Deleted skill with warnings: {msg}"),
                Style::default().fg(colors::warning()),
            ),
            None => ("Deleted.".to_string(), Style::default().fg(colors::success())),
        });

        self.app_event_tx.send(AppEvent::CodexOp(Op::ListSkills));
    }
}

fn skill_slug(skill: &Skill) -> String {
    skill
        .path
        .parent()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| skill.name.clone())
}

fn extract_frontmatter(body: &str) -> Option<String> {
    let mut lines = body.lines();
    if lines.next()? != "---" {
        return None;
    }
    let mut frontmatter = String::new();
    for line in lines {
        if line.trim() == "---" {
            return Some(frontmatter);
        }
        frontmatter.push_str(line);
        frontmatter.push('\n');
    }
    None
}

fn strip_frontmatter(body: &str) -> String {
    let mut lines = body.lines();
    if lines.next() != Some("---") {
        return body.to_string();
    }

    for line in lines.by_ref() {
        if line.trim() == "---" {
            let rest: String = lines.collect::<Vec<_>>().join("\n");
            return rest.trim_start_matches('\n').to_string();
        }
    }

    body.to_string()
}

fn yaml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn compose_skill_document(
    name: &str,
    description: &str,
    shell_style: &str,
    extra_frontmatter: &str,
    body: &str,
) -> String {
    let escaped_name = yaml_escape(name);
    let escaped_description = yaml_escape(description);
    let mut header = format!(
        "---\nname: \"{escaped_name}\"\ndescription: \"{escaped_description}\"\n"
    );
    let shell_style = shell_style.trim();
    if !shell_style.is_empty() {
        let escaped_style = yaml_escape(shell_style);
        header.push_str(&format!("shell_style: \"{escaped_style}\"\n"));
    }
    let extra_frontmatter = extra_frontmatter.trim();
    if !extra_frontmatter.is_empty() {
        header.push_str(extra_frontmatter.trim_end_matches('\n'));
        header.push('\n');
    }
    header.push_str("---\n\n");

    let body = body.trim_start_matches('\n');
    if body.is_empty() {
        header
    } else {
        format!("{header}{body}")
    }
}

fn filter_frontmatter_excluding_keys(frontmatter: &str, excluded_keys: &[&str]) -> String {
    if excluded_keys.is_empty() {
        return frontmatter.to_string();
    }

    let mut out: Vec<&str> = Vec::new();
    let lines: Vec<&str> = frontmatter.lines().collect();
    let mut idx = 0;
    while idx < lines.len() {
        let line = lines[idx];
        let is_top_level = !line.starts_with(|c: char| c.is_whitespace());
        if is_top_level {
            let trimmed = line.trim_start();
            let matches_excluded = excluded_keys.iter().any(|key| {
                let needle = format!("{key}:");
                trimmed.starts_with(needle.as_str())
            });
            if matches_excluded {
                idx += 1;
                // Skip continuation lines for multi-line / nested values (indented or blank).
                while idx < lines.len() {
                    let next = lines[idx];
                    if next.trim().is_empty() || next.starts_with(|c: char| c.is_whitespace()) {
                        idx += 1;
                        continue;
                    }
                    break;
                }
                continue;
            }
        }

        out.push(line);
        idx += 1;
    }

    out.join("\n")
}

fn frontmatter_value(body: &str, key: &str) -> Option<String> {
    let frontmatter = extract_frontmatter(body)?;
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&format!("{key}:")) {
            let value = rest
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn normalize_profile_skill_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn remove_profile_skill(values: &mut Vec<String>, skill_name: &str) {
    let normalized_target = normalize_profile_skill_name(skill_name);
    values.retain(|entry| normalize_profile_skill_name(entry) != normalized_target);
}

fn profile_list_contains_any(values: &[String], identifiers: &[&str]) -> bool {
    let normalized_values: Vec<String> = values
        .iter()
        .map(|entry| normalize_profile_skill_name(entry))
        .collect();
    identifiers
        .iter()
        .map(|identifier| normalize_profile_skill_name(identifier))
        .any(|normalized| normalized_values.iter().any(|value| value == &normalized))
}

fn unique_profile_identifiers<'a, I>(identifiers: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut deduped: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for identifier in identifiers {
        let trimmed = identifier.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = normalize_profile_skill_name(trimmed);
        if seen.insert(normalized) {
            deduped.push(trimmed.to_string());
        }
    }
    deduped
}

fn format_path_list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_path_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn parse_string_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn render_labeled_row(
    buf: &mut Buffer,
    viewport_inner: Rect,
    label_style: Style,
    label: &str,
    field_rect: Rect,
) {
    if field_rect.height == 0 || viewport_inner.width == 0 {
        return;
    }
    let label_w = 24u16.min(viewport_inner.width);
    let label_rect = Rect {
        x: viewport_inner.x,
        y: field_rect.y,
        width: label_w,
        height: field_rect.height,
    };
    Paragraph::new(Line::from(Span::styled(label, label_style))).render(label_rect, buf);
}

fn point_in_rect(rect: Rect, x: u16, y: u16) -> bool {
    if rect.width == 0 || rect.height == 0 {
        return false;
    }
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn style_profile_is_empty(profile: &ShellStyleProfileConfig) -> bool {
    profile.references.is_empty()
        && profile.prepend_developer_messages.is_empty()
        && profile.skills.is_empty()
        && profile.disabled_skills.is_empty()
        && profile.skill_roots.is_empty()
        && profile.mcp_servers.include.is_empty()
        && profile.mcp_servers.exclude.is_empty()
}

fn append_warning(current: &mut Option<String>, message: String) {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return;
    }
    match current {
        Some(existing) => {
            if !existing.contains(trimmed) {
                existing.push_str("; ");
                existing.push_str(trimmed);
            }
        }
        None => *current = Some(trimmed.to_string()),
    }
}

impl<'a> BottomPaneView<'a> for SkillsSettingsView {
    fn handle_key_event(&mut self, _pane: &mut BottomPane<'a>, key_event: KeyEvent) {
        let _ = self.handle_key_event_direct(key_event);
    }

    fn handle_key_event_with_result(
        &mut self,
        _pane: &mut BottomPane<'a>,
        key_event: KeyEvent,
    ) -> ConditionalUpdate {
        redraw_if(self.handle_key_event_direct(key_event))
    }

    fn handle_mouse_event(
        &mut self,
        _pane: &mut BottomPane<'a>,
        mouse_event: MouseEvent,
        area: Rect,
    ) -> ConditionalUpdate {
        redraw_if(self.handle_mouse_event_direct(mouse_event, area))
    }

    fn is_complete(&self) -> bool {
        self.is_complete()
    }

    fn desired_height(&self, _width: u16) -> u16 {
        28
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        SkillsSettingsView::render(self, area, buf);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use std::sync::mpsc::channel;

    fn make_view(
        profiles: HashMap<ShellScriptStyle, ShellStyleProfileConfig>,
    ) -> SkillsSettingsView {
        let (tx, _rx) = channel();
        SkillsSettingsView::new(Vec::new(), profiles, AppEventSender::new(tx))
    }

    fn mouse_left_click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn mouse_scroll_down(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    fn mouse_move(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: MouseEventKind::Moved,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn paste_is_ignored_in_list_mode() {
        let mut view = make_view(HashMap::new());
        assert!(!view.handle_paste_direct("zsh".to_string()));
    }

    #[test]
    fn paste_marks_style_resource_fields_dirty() {
        let mut view = make_view(HashMap::new());
        view.start_new_skill();

        view.focus = Focus::StyleReferences;
        assert!(view.handle_paste_direct("docs/shell/zsh.md".to_string()));
        assert_eq!(view.style_references_field.text(), "docs/shell/zsh.md");
        assert!(view.style_references_dirty);

        view.focus = Focus::StyleSkillRoots;
        assert!(view.handle_paste_direct("skills/zsh".to_string()));
        assert_eq!(view.style_skill_roots_field.text(), "skills/zsh");
        assert!(view.style_skill_roots_dirty);
    }

    #[test]
    fn style_paste_loads_profile_paths_when_not_dirty() {
        let mut profiles: HashMap<ShellScriptStyle, ShellStyleProfileConfig> = HashMap::new();
        profiles.insert(
            ShellScriptStyle::Zsh,
            ShellStyleProfileConfig {
                references: vec![PathBuf::from("docs/shell/zsh.md")],
                skill_roots: vec![PathBuf::from("skills/zsh")],
                mcp_servers: code_core::config_types::ShellStyleMcpConfig {
                    include: vec!["termux".to_string()],
                    exclude: vec!["legacy".to_string()],
                },
                ..Default::default()
            },
        );

        let mut view = make_view(profiles);
        view.start_new_skill();
        view.focus = Focus::Style;

        assert!(view.handle_paste_direct("zsh".to_string()));
        assert_eq!(view.style_field.text(), "zsh");
        assert_eq!(view.style_references_field.text(), "docs/shell/zsh.md");
        assert_eq!(view.style_skill_roots_field.text(), "skills/zsh");
        assert_eq!(view.style_mcp_include_field.text(), "termux");
        assert_eq!(view.style_mcp_exclude_field.text(), "legacy");
        assert!(!view.style_references_dirty);
        assert!(!view.style_skill_roots_dirty);
        assert!(!view.style_mcp_include_dirty);
        assert!(!view.style_mcp_exclude_dirty);
    }

    #[test]
    fn style_paste_does_not_override_manual_paths_when_dirty() {
        let mut profiles: HashMap<ShellScriptStyle, ShellStyleProfileConfig> = HashMap::new();
        profiles.insert(
            ShellScriptStyle::Zsh,
            ShellStyleProfileConfig {
                references: vec![PathBuf::from("docs/shell/zsh.md")],
                skill_roots: vec![PathBuf::from("skills/zsh")],
                mcp_servers: code_core::config_types::ShellStyleMcpConfig {
                    include: vec!["termux".to_string()],
                    exclude: vec!["legacy".to_string()],
                },
                ..Default::default()
            },
        );

        let mut view = make_view(profiles);
        view.start_new_skill();
        view.focus = Focus::StyleMcpInclude;
        assert!(view.handle_paste_direct("custom-server".to_string()));
        assert!(view.style_mcp_include_dirty);

        view.focus = Focus::Style;
        assert!(view.handle_paste_direct("zsh".to_string()));
        assert_eq!(view.style_field.text(), "zsh");
        assert_eq!(view.style_mcp_include_field.text(), "custom-server");
        assert_eq!(view.style_mcp_exclude_field.text(), "");
        assert_eq!(view.style_references_field.text(), "");
        assert_eq!(view.style_skill_roots_field.text(), "");
    }

    #[test]
    fn new_skill_fields_start_empty_for_visual_placeholders() {
        let mut view = make_view(HashMap::new());
        view.start_new_skill();

        assert_eq!(view.description_field.text(), "");
        assert_eq!(view.examples_field.text(), "");
        assert_eq!(view.body_field.text(), "");
    }

    #[test]
    fn list_click_add_new_enters_edit_mode() {
        let mut view = make_view(HashMap::new());
        let area = Rect::new(0, 0, 120, 40);

        // List row starts after border + title block.
        let click = mouse_left_click(2, 4);
        assert!(view.handle_mouse_event_direct(click, area));
        assert!(matches!(view.mode, Mode::Edit));
        assert!(matches!(view.focus, Focus::Name));
    }

    #[test]
    fn edit_click_focuses_style_mcp_include_field() {
        let mut profiles: HashMap<ShellScriptStyle, ShellStyleProfileConfig> = HashMap::new();
        profiles.insert(
            ShellScriptStyle::Zsh,
            ShellStyleProfileConfig {
                mcp_servers: code_core::config_types::ShellStyleMcpConfig {
                    include: vec!["termux".to_string()],
                    exclude: vec!["legacy".to_string()],
                },
                ..Default::default()
            },
        );

        let mut view = make_view(profiles);
        view.start_new_skill();
        view.style_field.set_text("zsh");
        view.set_style_resource_fields_from_profile(Some(ShellScriptStyle::Zsh));

        let area = Rect::new(0, 0, 140, 48);
        let layout = view.compute_form_layout(area).expect("layout should exist");
        let click = mouse_left_click(
            layout.style_mcp_include_inner.x.saturating_add(1),
            layout.style_mcp_include_inner.y.saturating_add(1),
        );
        assert!(view.handle_mouse_event_direct(click, area));
        assert!(matches!(view.focus, Focus::StyleMcpInclude));
    }

    #[test]
    fn scrolling_body_field_with_mouse_moves_cursor() {
        let mut view = make_view(HashMap::new());
        view.start_new_skill();
        let long_body = (0..60)
            .map(|idx| format!("line {idx}"))
            .collect::<Vec<_>>()
            .join("\n");
        view.body_field.set_text(&long_body);

        let area = Rect::new(0, 0, 140, 48);
        let layout = view.compute_form_layout(area).expect("layout should exist");

        let click = mouse_left_click(
            layout.body_inner.x.saturating_add(1),
            layout.body_inner.y.saturating_add(1),
        );
        assert!(view.handle_mouse_event_direct(click, area));
        assert!(matches!(view.focus, Focus::Body));

        let before = view.body_field.cursor();
        let scroll_down = mouse_scroll_down(
            layout.body_inner.x.saturating_add(1),
            layout.body_inner.y.saturating_add(1),
        );
        assert!(view.handle_mouse_event_direct(scroll_down, area));
        let after = view.body_field.cursor();
        assert!(after > before);
    }

    #[test]
    fn mouse_move_updates_button_hover_state() {
        let mut view = make_view(HashMap::new());
        view.start_new_skill();
        let area = Rect::new(0, 0, 140, 48);
        let layout = view.compute_form_layout(area).expect("layout should exist");

        let save_x = layout
            .buttons_row
            .x
            .saturating_add("Generate draft".len() as u16 + 3)
            .saturating_add(1);
        let hover_save = mouse_move(save_x, layout.buttons_row.y);
        assert!(view.handle_mouse_event_direct(hover_save, area));
        assert_eq!(view.hovered_button, Some(ActionButton::Save));

        let hover_body = mouse_move(
            layout.body_inner.x.saturating_add(1),
            layout.body_inner.y.saturating_add(1),
        );
        assert!(view.handle_mouse_event_direct(hover_body, area));
        assert_eq!(view.hovered_button, None);
    }

    #[test]
    fn short_height_editor_scrolls_focus_into_view() {
        let mut view = make_view(HashMap::new());
        view.start_new_skill();
        let area = Rect::new(0, 0, 80, 14);
        view.last_render_area.set(Some(area));
        assert_eq!(view.edit_scroll_top, 0);

        for _ in 0..9 {
            assert!(view.handle_key_event_direct(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)));
        }

        assert!(matches!(view.focus, Focus::Body));
        assert!(view.edit_scroll_top > 0);

        let layout = view.compute_form_layout(area).expect("layout should exist");
        assert!(layout.body_outer.height > 0);
    }
}
