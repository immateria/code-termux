use ratatui::layout::Rect;
use std::cell::RefCell;

use crate::bottom_pane::SettingsSection;

mod agents;
mod chrome;
mod contents;
mod limits;
mod overlay_input;
mod overlay_render;
mod types;

pub(crate) use self::agents::{AgentOverviewRow, AgentsSettingsContent};
pub(crate) use self::chrome::ChromeSettingsContent;
pub(crate) use self::contents::{
    AccountsSettingsContent,
    AutoDriveSettingsContent,
    McpSettingsContent,
    ModelSettingsContent,
    NotificationsSettingsContent,
    PlanningSettingsContent,
    PromptsSettingsContent,
    ReviewSettingsContent,
    SkillsSettingsContent,
    ThemeSettingsContent,
    UpdatesSettingsContent,
    ValidationSettingsContent,
};
pub(crate) use self::limits::LimitsSettingsContent;
pub(crate) use self::types::{SettingsContent, SettingsOverviewRow};

use self::types::{MenuState, SectionState, SettingsHelpOverlay, SettingsOverlayMode};

pub(crate) struct SettingsOverlayView {
    overview_rows: Vec<SettingsOverviewRow>,
    mode: SettingsOverlayMode,
    last_section: SettingsSection,
    help: Option<SettingsHelpOverlay>,
    model_content: Option<ModelSettingsContent>,
    planning_content: Option<PlanningSettingsContent>,
    theme_content: Option<ThemeSettingsContent>,
    updates_content: Option<UpdatesSettingsContent>,
    notifications_content: Option<NotificationsSettingsContent>,
    accounts_content: Option<AccountsSettingsContent>,
    prompts_content: Option<PromptsSettingsContent>,
    skills_content: Option<SkillsSettingsContent>,
    mcp_content: Option<McpSettingsContent>,
    agents_content: Option<AgentsSettingsContent>,
    review_content: Option<ReviewSettingsContent>,
    validation_content: Option<ValidationSettingsContent>,
    auto_drive_content: Option<AutoDriveSettingsContent>,
    limits_content: Option<LimitsSettingsContent>,
    chrome_content: Option<ChromeSettingsContent>,
    /// Last overlay content area for mouse calculations
    last_content_area: RefCell<Rect>,
    /// Exact sidebar rectangle from the last render pass for hover hit testing.
    last_sidebar_area: RefCell<Rect>,
    /// Last panel inner area where content is rendered (for mouse forwarding)
    last_panel_inner_area: RefCell<Rect>,
    /// Currently hovered section in sidebar (for visual feedback)
    hovered_section: RefCell<Option<SettingsSection>>,
}

impl SettingsOverlayView {
    pub(crate) fn new(section: SettingsSection) -> Self {
        let section_state = SectionState::new(section);
        Self {
            overview_rows: Vec::new(),
            mode: SettingsOverlayMode::Section(section_state),
            last_section: section,
            help: None,
            model_content: None,
            planning_content: None,
            theme_content: None,
            updates_content: None,
            notifications_content: None,
            accounts_content: None,
            prompts_content: None,
            skills_content: None,
            mcp_content: None,
            agents_content: None,
            review_content: None,
            validation_content: None,
            auto_drive_content: None,
            limits_content: None,
            chrome_content: None,
            last_content_area: RefCell::new(Rect::default()),
            last_sidebar_area: RefCell::new(Rect::default()),
            last_panel_inner_area: RefCell::new(Rect::default()),
            hovered_section: RefCell::new(None),
        }
    }

    pub(crate) fn active_section(&self) -> SettingsSection {
        match self.mode {
            SettingsOverlayMode::Menu(state) => state.selected(),
            SettingsOverlayMode::Section(state) => state.active(),
        }
    }

    pub(crate) fn is_menu_active(&self) -> bool {
        matches!(self.mode, SettingsOverlayMode::Menu(_))
    }

    pub(crate) fn set_mode_menu(&mut self, selected: Option<SettingsSection>) {
        let section = selected.unwrap_or(self.last_section);
        self.mode = SettingsOverlayMode::Menu(MenuState::new(section));
        if self.help.is_some() {
            self.show_help(true);
        }
    }

    pub(crate) fn set_mode_section(&mut self, section: SettingsSection) {
        self.mode = SettingsOverlayMode::Section(SectionState::new(section));
        self.last_section = section;
        if self.help.is_some() {
            self.show_help(false);
        }
    }

    pub(crate) fn is_help_visible(&self) -> bool {
        self.help.is_some()
    }

    pub(crate) fn show_help(&mut self, menu_active: bool) {
        self.help = Some(if menu_active {
            SettingsHelpOverlay::overview()
        } else {
            SettingsHelpOverlay::section(self.active_section())
        });
    }

    pub(crate) fn hide_help(&mut self) {
        self.help = None;
    }

    pub(crate) fn set_overview_rows(&mut self, rows: Vec<SettingsOverviewRow>) {
        let fallback = rows.first().map(|row| row.section).unwrap_or(self.last_section);
        if let SettingsOverlayMode::Menu(state) = &mut self.mode
            && !rows.iter().any(|row| row.section == state.selected()) {
                state.set_selected(fallback);
            }
        self.overview_rows = rows;
    }

    pub(crate) fn set_model_content(&mut self, content: ModelSettingsContent) {
        self.model_content = Some(content);
    }

    pub(crate) fn set_planning_content(&mut self, content: PlanningSettingsContent) {
        self.planning_content = Some(content);
    }

    pub(crate) fn set_theme_content(&mut self, content: ThemeSettingsContent) {
        self.theme_content = Some(content);
    }

    pub(crate) fn set_updates_content(&mut self, content: UpdatesSettingsContent) {
        self.updates_content = Some(content);
    }

    pub(crate) fn set_notifications_content(&mut self, content: NotificationsSettingsContent) {
        self.notifications_content = Some(content);
    }

    pub(crate) fn set_accounts_content(&mut self, content: AccountsSettingsContent) {
        self.accounts_content = Some(content);
    }

    pub(crate) fn set_prompts_content(&mut self, content: PromptsSettingsContent) {
        self.prompts_content = Some(content);
    }

    pub(crate) fn set_skills_content(&mut self, content: SkillsSettingsContent) {
        self.skills_content = Some(content);
    }

    pub(crate) fn set_mcp_content(&mut self, content: McpSettingsContent) {
        self.mcp_content = Some(content);
    }

    pub(crate) fn set_agents_content(&mut self, content: AgentsSettingsContent) {
        self.agents_content = Some(content);
    }

    pub(crate) fn set_review_content(&mut self, content: ReviewSettingsContent) {
        self.review_content = Some(content);
    }

    pub(crate) fn set_validation_content(&mut self, content: ValidationSettingsContent) {
        self.validation_content = Some(content);
    }

    pub(crate) fn set_auto_drive_content(&mut self, content: AutoDriveSettingsContent) {
        self.auto_drive_content = Some(content);
    }

    pub(crate) fn set_limits_content(&mut self, content: LimitsSettingsContent) {
        self.limits_content = Some(content);
    }

    pub(crate) fn set_chrome_content(&mut self, content: ChromeSettingsContent) {
        self.chrome_content = Some(content);
    }

    #[cfg_attr(not(any(test, feature = "test-helpers")), allow(dead_code))]
    pub(crate) fn agents_content(&self) -> Option<&AgentsSettingsContent> {
        self.agents_content.as_ref()
    }

    pub(crate) fn agents_content_mut(&mut self) -> Option<&mut AgentsSettingsContent> {
        self.agents_content.as_mut()
    }

    pub(crate) fn review_content_mut(&mut self) -> Option<&mut ReviewSettingsContent> {
        self.review_content.as_mut()
    }

    pub(crate) fn planning_content_mut(&mut self) -> Option<&mut PlanningSettingsContent> {
        self.planning_content.as_mut()
    }

    pub(crate) fn auto_drive_content_mut(&mut self) -> Option<&mut AutoDriveSettingsContent> {
        self.auto_drive_content.as_mut()
    }

    pub(crate) fn limits_content_mut(&mut self) -> Option<&mut LimitsSettingsContent> {
        self.limits_content.as_mut()
    }

    pub(crate) fn set_section(&mut self, section: SettingsSection) -> bool {
        if self.active_section() == section {
            return false;
        }
        self.last_section = section;
        match &mut self.mode {
            SettingsOverlayMode::Menu(state) => state.set_selected(section),
            SettingsOverlayMode::Section(state) => state.set_active(section),
        }
        if self.help.is_some() {
            self.show_help(self.is_menu_active());
        }
        true
    }

    pub(crate) fn select_next(&mut self) -> bool {
        if !self.overview_rows.is_empty() {
            let sections: Vec<SettingsSection> =
                self.overview_rows.iter().map(|row| row.section).collect();
            if let Some(idx) = sections
                .iter()
                .position(|section| *section == self.active_section())
            {
                let next = sections[(idx + 1) % sections.len()];
                return self.set_section(next);
            }
        }
        let mut idx = self.index_of(self.active_section());
        idx = (idx + 1) % SettingsSection::ALL.len();
        self.set_section(SettingsSection::ALL[idx])
    }

    pub(crate) fn select_previous(&mut self) -> bool {
        if !self.overview_rows.is_empty() {
            let sections: Vec<SettingsSection> =
                self.overview_rows.iter().map(|row| row.section).collect();
            if let Some(idx) = sections
                .iter()
                .position(|section| *section == self.active_section())
            {
                let new_idx = idx.checked_sub(1).unwrap_or(sections.len() - 1);
                return self.set_section(sections[new_idx]);
            }
        }
        let mut idx = self.index_of(self.active_section());
        idx = idx.checked_sub(1).unwrap_or(SettingsSection::ALL.len() - 1);
        self.set_section(SettingsSection::ALL[idx])
    }

    fn index_of(&self, section: SettingsSection) -> usize {
        SettingsSection::ALL
            .iter()
            .position(|s| *s == section)
            .unwrap_or(0)
    }

    pub(crate) fn active_content_mut(&mut self) -> Option<&mut dyn SettingsContent> {
        if self.is_menu_active() {
            return None;
        }

        match self.active_section() {
            SettingsSection::Model => self
                .model_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Planning => self
                .planning_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Theme => self
                .theme_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Updates => self
                .updates_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Agents => self
                .agents_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Accounts => self
                .accounts_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Prompts => self
                .prompts_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Skills => self
                .skills_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::AutoDrive => self
                .auto_drive_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Review => self
                .review_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Validation => self
                .validation_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Limits => self
                .limits_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Chrome => self
                .chrome_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Notifications => self
                .notifications_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
            SettingsSection::Mcp => self
                .mcp_content
                .as_mut()
                .map(|content| content as &mut dyn SettingsContent),
        }
    }

    pub(crate) fn notify_close(&mut self) {
        match self.active_section() {
            SettingsSection::Model => {
                if let Some(content) = self.model_content.as_mut() {
                    content.on_close();
                }
            }
            SettingsSection::Theme => {
                if let Some(content) = self.theme_content.as_mut() {
                    content.on_close();
                }
            }
            SettingsSection::Notifications => {
                if let Some(content) = self.notifications_content.as_mut() {
                    content.on_close();
                }
            }
            SettingsSection::Mcp => {
                if let Some(content) = self.mcp_content.as_mut() {
                    content.on_close();
                }
            }
            SettingsSection::Chrome => {
                if let Some(content) = self.chrome_content.as_mut() {
                    content.on_close();
                }
            }
            _ => {}
        }
    }
}
