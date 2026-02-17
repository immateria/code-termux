use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::ui_interaction::{
    RelativeHitRegion,
    redraw_if,
    route_selectable_regions_mouse_with_config,
    ScrollSelectionBehavior,
    SelectableListMouseConfig,
    SelectableListMouseResult,
    wrap_next,
    wrap_prev,
};
use crate::colors;
use code_core::config_types::AuthCredentialsStoreMode;
use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::bottom_pane_view::{BottomPaneView, ConditionalUpdate};
use super::BottomPane;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ViewMode {
    Main,
    ConfirmStoreChange {
        target: AuthCredentialsStoreMode,
        selected: usize,
    },
}

pub(crate) struct AccountSwitchSettingsView {
    app_event_tx: AppEventSender,
    selected_index: usize,
    auto_switch_enabled: bool,
    api_key_fallback_enabled: bool,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    view_mode: ViewMode,
    is_complete: bool,
}

impl AccountSwitchSettingsView {
    const MAIN_OPTION_COUNT: usize = 6;
    const MAIN_HIT_REGIONS: [RelativeHitRegion; Self::MAIN_OPTION_COUNT] = [
        RelativeHitRegion::new(0, 2, 2),
        RelativeHitRegion::new(1, 4, 2),
        RelativeHitRegion::new(2, 6, 2),
        RelativeHitRegion::new(3, 9, 2),
        RelativeHitRegion::new(4, 11, 2),
        RelativeHitRegion::new(5, 14, 1),
    ];

    const CONFIRM_OPTION_COUNT: usize = 3;
    const CONFIRM_HIT_REGIONS: [RelativeHitRegion; Self::CONFIRM_OPTION_COUNT] = [
        RelativeHitRegion::new(0, 4, 1),
        RelativeHitRegion::new(1, 5, 1),
        RelativeHitRegion::new(2, 6, 1),
    ];

    pub(crate) fn new(
        app_event_tx: AppEventSender,
        auto_switch_enabled: bool,
        api_key_fallback_enabled: bool,
        auth_credentials_store_mode: AuthCredentialsStoreMode,
    ) -> Self {
        Self {
            app_event_tx,
            selected_index: 0,
            auto_switch_enabled,
            api_key_fallback_enabled,
            auth_credentials_store_mode,
            view_mode: ViewMode::Main,
            is_complete: false,
        }
    }

    fn auth_store_mode_label(mode: AuthCredentialsStoreMode) -> &'static str {
        match mode {
            AuthCredentialsStoreMode::File => "file",
            AuthCredentialsStoreMode::Keyring => "keyring",
            AuthCredentialsStoreMode::Auto => "auto",
            AuthCredentialsStoreMode::Ephemeral => "ephemeral",
        }
    }

    fn next_auth_store_mode(mode: AuthCredentialsStoreMode) -> AuthCredentialsStoreMode {
        match mode {
            AuthCredentialsStoreMode::File => AuthCredentialsStoreMode::Keyring,
            AuthCredentialsStoreMode::Keyring => AuthCredentialsStoreMode::Auto,
            AuthCredentialsStoreMode::Auto => AuthCredentialsStoreMode::Ephemeral,
            AuthCredentialsStoreMode::Ephemeral => AuthCredentialsStoreMode::File,
        }
    }

    fn toggle_auto_switch(&mut self) {
        self.auto_switch_enabled = !self.auto_switch_enabled;
        self.app_event_tx
            .send(AppEvent::SetAutoSwitchAccountsOnRateLimit(
                self.auto_switch_enabled,
            ));
    }

    fn toggle_api_key_fallback(&mut self) {
        self.api_key_fallback_enabled = !self.api_key_fallback_enabled;
        self.app_event_tx
            .send(AppEvent::SetApiKeyFallbackOnAllAccountsLimited(
                self.api_key_fallback_enabled,
            ));
    }

    fn close(&mut self) {
        self.is_complete = true;
    }

    fn show_login_accounts(&self) {
        self.app_event_tx.send(AppEvent::ShowLoginAccounts);
    }

    fn show_login_add_account(&self) {
        self.app_event_tx.send(AppEvent::ShowLoginAddAccount);
    }

    fn request_store_mode_change(&mut self, target: AuthCredentialsStoreMode, migrate_existing: bool) {
        self.app_event_tx.send(AppEvent::RequestSetAuthCredentialsStoreMode {
            mode: target,
            migrate_existing,
        });
    }

    fn open_store_mode_confirm(&mut self) {
        let target = Self::next_auth_store_mode(self.auth_credentials_store_mode);
        self.view_mode = ViewMode::ConfirmStoreChange { target, selected: 0 };
    }

    fn activate_selected_main(&mut self) {
        match self.selected_index {
            0 => self.toggle_auto_switch(),
            1 => self.toggle_api_key_fallback(),
            2 => self.open_store_mode_confirm(),
            3 => self.show_login_accounts(),
            4 => self.show_login_add_account(),
            5 => self.close(),
            _ => {}
        }
    }

    fn confirm_selected_index(&self) -> usize {
        match self.view_mode {
            ViewMode::ConfirmStoreChange { selected, .. } => selected,
            ViewMode::Main => 0,
        }
    }

    fn set_confirm_selected_index(&mut self, selected: usize) {
        if let ViewMode::ConfirmStoreChange { target, .. } = self.view_mode {
            self.view_mode = ViewMode::ConfirmStoreChange { target, selected };
        }
    }

    fn activate_selected_confirm(&mut self) {
        let ViewMode::ConfirmStoreChange { target, selected } = self.view_mode else {
            return;
        };

        match selected {
            0 => {
                self.request_store_mode_change(target, true);
                self.view_mode = ViewMode::Main;
            }
            1 => {
                self.request_store_mode_change(target, false);
                self.view_mode = ViewMode::Main;
            }
            2 => {
                self.view_mode = ViewMode::Main;
            }
            _ => {}
        }
    }

    fn main_info_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(Line::from(vec![Span::styled(
            "Accounts",
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));

        let highlight = Style::default()
            .fg(colors::primary())
            .add_modifier(Modifier::BOLD);
        let normal = Style::default().fg(colors::text());
        let dim = Style::default().fg(colors::text_dim());

        let row = |idx: usize, label: &str, enabled: bool| -> Line<'static> {
            let selected = idx == self.selected_index;
            let indicator = if selected { ">" } else { " " };
            let style = if selected { highlight } else { normal };
            let state_style = if enabled {
                Style::default().fg(colors::success())
            } else {
                Style::default().fg(colors::text_dim())
            };
            Line::from(vec![
                Span::styled(format!("{indicator} "), style),
                Span::styled(label.to_string(), style),
                Span::raw("  "),
                Span::styled(
                    format!("[{}]", if enabled { "x" } else { " " }),
                    state_style,
                ),
            ])
        };

        lines.push(row(
            0,
            "Auto-switch on rate/usage limit",
            self.auto_switch_enabled,
        ));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                "Switches to another connected account on 429/usage_limit.",
                dim,
            ),
        ]));

        lines.push(row(
            1,
            "API key fallback when all accounts limited",
            self.api_key_fallback_enabled,
        ));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(
                "Only used if every connected ChatGPT account is limited.",
                dim,
            ),
        ]));

        let store_selected = self.selected_index == 2;
        let store_style = if store_selected { highlight } else { normal };
        let indicator = if store_selected { ">" } else { " " };
        let store_mode = Self::auth_store_mode_label(self.auth_credentials_store_mode);
        lines.push(Line::from(vec![
            Span::styled(format!("{indicator} "), store_style),
            Span::styled("Credential store".to_string(), store_style),
            Span::raw("  "),
            Span::styled(
                format!("[{store_mode}]"),
                Style::default().fg(colors::primary()),
            ),
        ]));
        let store_detail = match self.auth_credentials_store_mode {
            AuthCredentialsStoreMode::Ephemeral => {
                "In-memory only (will not persist across restarts)."
            }
            _ => "Where Code stores CLI auth credentials (auth.json payload).",
        };
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled(store_detail, dim),
        ]));

        lines.push(Line::from(""));

        let manage_selected = self.selected_index == 3;
        let manage_style = if manage_selected { highlight } else { normal };
        let manage_indicator = if manage_selected { ">" } else { " " };
        lines.push(Line::from(vec![
            Span::styled(format!("{manage_indicator} "), manage_style),
            Span::styled("Manage connected accounts", manage_style),
        ]));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled("View, switch, and remove stored accounts.", dim),
        ]));

        let add_selected = self.selected_index == 4;
        let add_style = if add_selected { highlight } else { normal };
        let add_indicator = if add_selected { ">" } else { " " };
        lines.push(Line::from(vec![
            Span::styled(format!("{add_indicator} "), add_style),
            Span::styled("Add account", add_style),
        ]));
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::styled("Start ChatGPT or API-key account setup.", dim),
        ]));

        lines.push(Line::from(""));

        let close_selected = self.selected_index == 5;
        let close_style = if close_selected { highlight } else { normal };
        let indicator = if close_selected { ">" } else { " " };
        lines.push(Line::from(vec![
            Span::styled(format!("{indicator} "), close_style),
            Span::styled("Close", close_style),
        ]));

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" Up/Down", Style::default().fg(colors::function())),
            Span::styled(" Navigate  ", dim),
            Span::styled("Enter", Style::default().fg(colors::success())),
            Span::styled(" Toggle/Open  ", dim),
            Span::styled("Esc", Style::default().fg(colors::error())),
            Span::styled(" Close", dim),
        ]));

        lines
    }

    fn confirm_info_lines(&self, target: AuthCredentialsStoreMode, selected: usize) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(Line::from(vec![Span::styled(
            "Credential store",
            Style::default().add_modifier(Modifier::BOLD),
        )]));
        lines.push(Line::from(""));

        let dim = Style::default().fg(colors::text_dim());
        let current = Self::auth_store_mode_label(self.auth_credentials_store_mode);
        let next = Self::auth_store_mode_label(target);
        lines.push(Line::from(vec![
            Span::styled("Current: ", dim),
            Span::styled(current, Style::default().fg(colors::text())),
            Span::styled("   New: ", dim),
            Span::styled(next, Style::default().fg(colors::primary()).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(""));

        let highlight = Style::default()
            .fg(colors::primary())
            .add_modifier(Modifier::BOLD);
        let normal = Style::default().fg(colors::text());

        let row = |idx: usize, label: &str| -> Line<'static> {
            let is_selected = idx == selected;
            let indicator = if is_selected { ">" } else { " " };
            let style = if is_selected { highlight } else { normal };
            Line::from(vec![
                Span::styled(format!("{indicator} "), style),
                Span::styled(label.to_string(), style),
            ])
        };

        lines.push(row(0, "Apply + migrate existing credentials"));
        lines.push(row(1, "Apply (do not migrate)  (may log you out)"));
        lines.push(row(2, "Cancel"));

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" Up/Down", Style::default().fg(colors::function())),
            Span::styled(" Select  ", dim),
            Span::styled("Enter", Style::default().fg(colors::success())),
            Span::styled(" Apply  ", dim),
            Span::styled("Esc", Style::default().fg(colors::error())),
            Span::styled(" Back", dim),
        ]));

        lines
    }

    fn info_lines(&self) -> Vec<Line<'static>> {
        match self.view_mode {
            ViewMode::Main => self.main_info_lines(),
            ViewMode::ConfirmStoreChange { target, selected } => {
                self.confirm_info_lines(target, selected)
            }
        }
    }

    pub(crate) fn render_without_frame(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        Paragraph::new(self.info_lines())
            .wrap(Wrap { trim: true })
            .style(Style::default().bg(colors::background()).fg(colors::text()))
            .render(area, buf);
    }

    pub(crate) fn handle_key_event_direct(&mut self, key_event: KeyEvent) -> bool {
        match self.view_mode {
            ViewMode::Main => match key_event.code {
                KeyCode::Esc => {
                    self.close();
                    true
                }
                KeyCode::Up => {
                    self.selected_index =
                        wrap_prev(self.selected_index, Self::MAIN_OPTION_COUNT);
                    true
                }
                KeyCode::Down | KeyCode::Tab => {
                    self.selected_index =
                        wrap_next(self.selected_index, Self::MAIN_OPTION_COUNT);
                    true
                }
                KeyCode::BackTab => {
                    self.selected_index =
                        wrap_prev(self.selected_index, Self::MAIN_OPTION_COUNT);
                    true
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    self.activate_selected_main();
                    true
                }
                _ => false,
            },
            ViewMode::ConfirmStoreChange { .. } => match key_event.code {
                KeyCode::Esc => {
                    self.view_mode = ViewMode::Main;
                    true
                }
                KeyCode::Up => {
                    let next = wrap_prev(
                        self.confirm_selected_index(),
                        Self::CONFIRM_OPTION_COUNT,
                    );
                    self.set_confirm_selected_index(next);
                    true
                }
                KeyCode::Down | KeyCode::Tab => {
                    let next = wrap_next(
                        self.confirm_selected_index(),
                        Self::CONFIRM_OPTION_COUNT,
                    );
                    self.set_confirm_selected_index(next);
                    true
                }
                KeyCode::BackTab => {
                    let next = wrap_prev(
                        self.confirm_selected_index(),
                        Self::CONFIRM_OPTION_COUNT,
                    );
                    self.set_confirm_selected_index(next);
                    true
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    self.activate_selected_confirm();
                    true
                }
                _ => false,
            },
        }
    }

    pub(crate) fn handle_mouse_event_direct(&mut self, mouse_event: MouseEvent, area: Rect) -> bool {
        match self.view_mode {
            ViewMode::Main => {
                let mut selected = self.selected_index;
                let result = route_selectable_regions_mouse_with_config(
                    mouse_event,
                    &mut selected,
                    Self::MAIN_OPTION_COUNT,
                    area,
                    &Self::MAIN_HIT_REGIONS,
                    SelectableListMouseConfig {
                        require_pointer_hit_for_scroll: true,
                        scroll_behavior: ScrollSelectionBehavior::Clamp,
                        ..SelectableListMouseConfig::default()
                    },
                );
                self.selected_index = selected;

                if matches!(result, SelectableListMouseResult::Activated) {
                    self.activate_selected_main();
                }
                result.handled()
            }
            ViewMode::ConfirmStoreChange { .. } => {
                let mut selected = self.confirm_selected_index();
                let result = route_selectable_regions_mouse_with_config(
                    mouse_event,
                    &mut selected,
                    Self::CONFIRM_OPTION_COUNT,
                    area,
                    &Self::CONFIRM_HIT_REGIONS,
                    SelectableListMouseConfig {
                        require_pointer_hit_for_scroll: true,
                        scroll_behavior: ScrollSelectionBehavior::Clamp,
                        ..SelectableListMouseConfig::default()
                    },
                );
                self.set_confirm_selected_index(selected);

                if matches!(result, SelectableListMouseResult::Activated) {
                    self.activate_selected_confirm();
                }
                result.handled()
            }
        }
    }

    pub(crate) fn is_view_complete(&self) -> bool {
        self.is_complete
    }
}

impl<'a> BottomPaneView<'a> for AccountSwitchSettingsView {
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
        self.is_complete
    }

    fn desired_height(&self, _width: u16) -> u16 {
        match self.view_mode {
            ViewMode::Main => 18,
            ViewMode::ConfirmStoreChange { .. } => 10,
        }
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        self.render_without_frame(area, buf);
    }
}
