use std::cell::RefCell;
use std::rc::Rc;

use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

use crate::bottom_pane::{
    AccountSwitchSettingsView,
    ConditionalUpdate,
    LoginAccountsState,
    LoginAddAccountState,
};
use crate::{app_event_sender::AppEventSender, colors};

use super::super::SettingsContent;

enum AccountsSubmenuMode {
    Switch(AccountSwitchSettingsView),
    Manage(Rc<RefCell<LoginAccountsState>>),
    Add(Rc<RefCell<LoginAddAccountState>>),
}

pub(crate) struct AccountsSettingsContent {
    mode: AccountsSubmenuMode,
    app_event_tx: AppEventSender,
    auto_switch_enabled: bool,
    api_key_fallback_enabled: bool,
    auth_credentials_store_mode: code_core::config_types::AuthCredentialsStoreMode,
}

impl AccountsSettingsContent {
    pub(crate) fn new(
        app_event_tx: AppEventSender,
        auto_switch_enabled: bool,
        api_key_fallback_enabled: bool,
        auth_credentials_store_mode: code_core::config_types::AuthCredentialsStoreMode,
    ) -> Self {
        let view = AccountSwitchSettingsView::new(
            app_event_tx.clone(),
            auto_switch_enabled,
            api_key_fallback_enabled,
            auth_credentials_store_mode,
        );
        Self {
            mode: AccountsSubmenuMode::Switch(view),
            app_event_tx,
            auto_switch_enabled,
            api_key_fallback_enabled,
            auth_credentials_store_mode,
        }
    }

    pub(crate) fn show_manage_accounts(&mut self, state: Rc<RefCell<LoginAccountsState>>) {
        self.mode = AccountsSubmenuMode::Manage(state);
    }

    pub(crate) fn show_add_account(&mut self, state: Rc<RefCell<LoginAddAccountState>>) {
        self.mode = AccountsSubmenuMode::Add(state);
    }

    fn reset_to_switch_mode(&mut self) {
        self.mode = AccountsSubmenuMode::Switch(AccountSwitchSettingsView::new(
            self.app_event_tx.clone(),
            self.auto_switch_enabled,
            self.api_key_fallback_enabled,
            self.auth_credentials_store_mode,
        ));
    }

    fn render_embedded_accounts_state(
        mode_label: &str,
        area: Rect,
        buf: &mut Buffer,
        render: impl Fn(Rect, &mut Buffer),
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let title = Paragraph::new(Line::from(vec![
            Span::styled(
                mode_label.to_string(),
                Style::default()
                    .fg(colors::primary())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  (Esc to return)".to_string(),
                Style::default().fg(colors::text_dim()),
            ),
        ]));
        let title_height = 1;
        title.render(
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: title_height,
            },
            buf,
        );

        let content_area = Rect {
            x: area.x,
            y: area.y.saturating_add(title_height),
            width: area.width,
            height: area.height.saturating_sub(title_height),
        };
        render(content_area, buf);
    }
}

impl SettingsContent for AccountsSettingsContent {
    fn render(&self, area: Rect, buf: &mut Buffer) {
        match &self.mode {
            AccountsSubmenuMode::Switch(view) => view.render_without_frame(area, buf),
            AccountsSubmenuMode::Manage(state) => {
                Self::render_embedded_accounts_state("Manage Accounts", area, buf, |inner, buffer| {
                    state.borrow().render(inner, buffer);
                });
            }
            AccountsSubmenuMode::Add(state) => {
                Self::render_embedded_accounts_state("Add Account", area, buf, |inner, buffer| {
                    state.borrow().render(inner, buffer);
                });
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> bool {
        let mut handled = true;
        match &mut self.mode {
            AccountsSubmenuMode::Switch(view) => {
                handled = view.handle_key_event_direct(key);
            }
            AccountsSubmenuMode::Manage(state) => {
                state.borrow_mut().handle_key_event(key);
                if state.borrow().is_complete() {
                    state.borrow_mut().clear_complete();
                    self.reset_to_switch_mode();
                }
            }
            AccountsSubmenuMode::Add(state) => {
                state.borrow_mut().handle_key_event(key);
                if state.borrow().is_complete() {
                    state.borrow_mut().clear_complete();
                    self.reset_to_switch_mode();
                }
            }
        }
        handled
    }

    fn is_complete(&self) -> bool {
        match &self.mode {
            AccountsSubmenuMode::Switch(view) => view.is_view_complete(),
            AccountsSubmenuMode::Manage(_) | AccountsSubmenuMode::Add(_) => false,
        }
    }

    fn handle_mouse(&mut self, mouse_event: MouseEvent, area: Rect) -> bool {
        match &mut self.mode {
            AccountsSubmenuMode::Switch(view) => view.handle_mouse_event_direct(mouse_event, area),
            AccountsSubmenuMode::Manage(state) => {
                let content_area = Rect {
                    x: area.x,
                    y: area.y.saturating_add(1),
                    width: area.width,
                    height: area.height.saturating_sub(1),
                };
                state
                    .borrow_mut()
                    .handle_mouse_event(mouse_event, content_area)
            }
            AccountsSubmenuMode::Add(_) => false,
        }
    }

    fn handle_paste(&mut self, text: String) -> bool {
        match &mut self.mode {
            AccountsSubmenuMode::Switch(_) => false,
            AccountsSubmenuMode::Manage(state) => matches!(
                state.borrow_mut().handle_paste(text),
                ConditionalUpdate::NeedsRedraw
            ),
            AccountsSubmenuMode::Add(state) => matches!(
                state.borrow_mut().handle_paste(text),
                ConditionalUpdate::NeedsRedraw
            ),
        }
    }
}
