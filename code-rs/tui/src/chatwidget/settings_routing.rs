use super::*;

impl ChatWidget<'_> {
    #[allow(dead_code)]
    pub(crate) fn show_theme_selection(&mut self) {
        let tail_ticket = self.make_background_tail_ticket();
        let before_ticket = self.make_background_before_next_output_ticket();
        self.bottom_pane.show_theme_selection(
            crate::theme::current_theme_name(),
            tail_ticket,
            before_ticket,
        );
    }

    pub(crate) fn show_settings_overlay(&mut self, section: Option<SettingsSection>) {
        // TODO(responsive-settings): Route between overlay and bottom pane based on
        // terminal width and a user-configurable preference. Today we route to
        // bottom pane when possible and fall back to the overlay for unsupported
        // sections.
        if let Some(section) = section
            && self.open_settings_section_in_bottom_pane(section) {
                return;
            }

        let initial_section = section
            .or_else(|| {
                self.settings
                    .overlay
                    .as_ref()
                    .map(super::settings_overlay::SettingsOverlayView::active_section)
            })
            .unwrap_or(SettingsSection::Model);

        let mut overlay = SettingsOverlayView::new(initial_section);
        overlay.set_model_content(self.build_model_settings_content());
        overlay.set_planning_content(self.build_planning_settings_content());
        overlay.set_theme_content(self.build_theme_settings_content());
        if let Some(update_content) = self.build_updates_settings_content() {
            overlay.set_updates_content(update_content);
        }
        overlay.set_accounts_content(self.build_accounts_settings_content());
        overlay.set_notifications_content(self.build_notifications_settings_content());
        overlay.set_prompts_content(self.build_prompts_settings_content());
        overlay.set_skills_content(self.build_skills_settings_content());
        if let Some(mcp_content) = self.build_mcp_settings_content() {
            overlay.set_mcp_content(mcp_content);
        }
        overlay.set_agents_content(self.build_agents_settings_content());
        overlay.set_auto_drive_content(self.build_auto_drive_settings_content());
        overlay.set_review_content(self.build_review_settings_content());
        overlay.set_validation_content(self.build_validation_settings_content());
        overlay.set_limits_content(self.build_limits_settings_content());
        overlay.set_chrome_content(self.build_chrome_settings_content(None));
        let overview_rows = self.build_settings_overview_rows();
        overlay.set_overview_rows(overview_rows);

        match section {
            Some(section) => overlay.set_mode_section(section),
            None => overlay.set_mode_menu(None),
        }

        self.settings.overlay = Some(overlay);
        self.request_redraw();
    }

    pub(crate) fn ensure_settings_overlay_section(&mut self, section: SettingsSection) {
        match self.settings.overlay.as_mut() {
            Some(overlay) => {
                let was_menu = overlay.is_menu_active();
                let changed_section = overlay.active_section() != section;
                overlay.set_mode_section(section);
                if was_menu || changed_section {
                    self.request_redraw();
                }
            }
            None => {
                self.show_settings_overlay(Some(section));
            }
        }
    }

    pub(super) fn build_model_settings_view(&self) -> ModelSelectionView {
        let presets = self.available_model_presets();
        let current_model = self.config.model.clone();
        let current_effort = self.config.model_reasoning_effort;
        ModelSelectionView::new(
            presets,
            current_model,
            current_effort,
            false,
            ModelSelectionTarget::Session,
            self.app_event_tx.clone(),
        )
    }

    pub(super) fn build_model_settings_content(&self) -> ModelSettingsContent {
        ModelSettingsContent::new(self.build_model_settings_view())
    }

    pub(super) fn build_theme_settings_content(&mut self) -> ThemeSettingsContent {
        let tail_ticket = self.make_background_tail_ticket();
        let before_ticket = self.make_background_before_next_output_ticket();
        let view = ThemeSelectionView::new(
            crate::theme::current_theme_name(),
            self.app_event_tx.clone(),
            tail_ticket,
            before_ticket,
        );
        ThemeSettingsContent::new(view)
    }

    pub(super) fn build_notifications_settings_view(&mut self) -> NotificationsSettingsView {
        let mode = match &self.config.tui.notifications {
            Notifications::Enabled(enabled) => NotificationsMode::Toggle { enabled: *enabled },
            Notifications::Custom(entries) => NotificationsMode::Custom { entries: entries.clone() },
        };
        let ticket = self.make_background_tail_ticket();
        NotificationsSettingsView::new(mode, self.app_event_tx.clone(), ticket)
    }

    pub(super) fn build_notifications_settings_content(&mut self) -> NotificationsSettingsContent {
        NotificationsSettingsContent::new(self.build_notifications_settings_view())
    }

    pub(super) fn build_prompts_settings_view(&mut self) -> PromptsSettingsView {
        let prompts = self.bottom_pane.custom_prompts().to_vec();
        PromptsSettingsView::new(prompts, self.app_event_tx.clone())
    }

    pub(super) fn build_prompts_settings_content(&mut self) -> PromptsSettingsContent {
        PromptsSettingsContent::new(self.build_prompts_settings_view())
    }

    pub(super) fn build_skills_settings_view(&mut self) -> SkillsSettingsView {
        let skills = self.bottom_pane.skills().to_vec();
        SkillsSettingsView::new(
            skills,
            self.config.shell_style_profiles.clone(),
            self.app_event_tx.clone(),
        )
    }

    pub(super) fn build_skills_settings_content(&mut self) -> SkillsSettingsContent {
        SkillsSettingsContent::new(self.build_skills_settings_view())
    }

    pub(super) fn build_chrome_settings_content(&self, port: Option<u16>) -> ChromeSettingsContent {
        ChromeSettingsContent::new(self.app_event_tx.clone(), port)
    }

    pub(super) fn build_mcp_server_rows(&mut self) -> Option<McpServerRows> {
        let home = match code_core::config::find_code_home() {
            Ok(home) => home,
            Err(e) => {
                let msg = format!("Failed to locate CODE_HOME: {e}");
                self.history_push_plain_state(history_cell::new_error_event(msg));
                return None;
            }
        };

        let (enabled, disabled) = match code_core::config::list_mcp_servers(&home) {
            Ok(result) => result,
            Err(e) => {
                let msg = format!("Failed to read MCP config: {e}");
                self.history_push_plain_state(history_cell::new_error_event(msg));
                return None;
            }
        };

        let mut rows: McpServerRows = Vec::new();
        for (name, cfg) in enabled.into_iter() {
            let summary = self.format_mcp_server_summary(&name, &cfg, true);
            rows.push(McpServerRow {
                name,
                enabled: true,
                summary,
            });
        }
        for (name, cfg) in disabled.into_iter() {
            let summary = self.format_mcp_server_summary(&name, &cfg, false);
            rows.push(McpServerRow {
                name,
                enabled: false,
                summary,
            });
        }
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        Some(rows)
    }

    pub(super) fn build_mcp_settings_content(&mut self) -> Option<McpSettingsContent> {
        let rows = self.build_mcp_server_rows()?;
        let view = McpSettingsView::new(rows, self.app_event_tx.clone());
        Some(McpSettingsContent::new(view))
    }

    pub(super) fn is_builtin_agent(name: &str, command: &str) -> bool {
        if let Some(spec) = agent_model_spec(name).or_else(|| agent_model_spec(command)) {
            return matches!(spec.family, "code" | "codex" | "cloud");
        }

        name.eq_ignore_ascii_case("code")
            || name.eq_ignore_ascii_case("codex")
            || name.eq_ignore_ascii_case("cloud")
            || name.eq_ignore_ascii_case("coder")
            || command.eq_ignore_ascii_case("code")
            || command.eq_ignore_ascii_case("codex")
            || command.eq_ignore_ascii_case("cloud")
            || command.eq_ignore_ascii_case("coder")
    }

    pub(super) fn collect_agents_overview_rows(&self) -> (Vec<AgentOverviewRow>, Vec<String>) {
        fn command_exists(cmd: &str) -> bool {
            if cmd.contains(std::path::MAIN_SEPARATOR) || cmd.contains('/') || cmd.contains('\\') {
                return std::fs::metadata(cmd).map(|m| m.is_file()).unwrap_or(false);
            }
            #[cfg(target_os = "windows")]
            {
                which::which(cmd).map(|p| p.is_file()).unwrap_or(false)
            }
            #[cfg(not(target_os = "windows"))]
            {
                use std::os::unix::fs::PermissionsExt;
                let Some(path_os) = std::env::var_os("PATH") else {
                    return false;
                };
                for dir in std::env::split_paths(&path_os) {
                    if dir.as_os_str().is_empty() {
                        continue;
                    }
                    let candidate = dir.join(cmd);
                    if let Ok(meta) = std::fs::metadata(&candidate)
                        && meta.is_file() && (meta.permissions().mode() & 0o111 != 0) {
                            return true;
                        }
                }
                false
            }
        }

        fn command_for_check(command: &str) -> String {
            let (base, _) = split_command_and_args(command);
            if base.trim().is_empty() {
                command.trim().to_string()
            } else {
                base
            }
        }

        let mut agent_rows: Vec<AgentOverviewRow> = Vec::new();
        let mut ordered: Vec<String> = enabled_agent_model_specs()
            .into_iter()
            .map(|spec| spec.slug.to_string())
            .collect();
        let mut extras: Vec<String> = Vec::new();
        for agent in &self.config.agents {
            if !ordered.iter().any(|name| agent.name.eq_ignore_ascii_case(name)) {
                extras.push(agent.name.to_ascii_lowercase());
            }
        }
        let mut pending_agents: HashMap<String, AgentConfig> = HashMap::new();
        for pending in self.pending_agent_updates.values() {
            let lower = pending.cfg.name.to_ascii_lowercase();
            pending_agents.insert(lower.clone(), pending.cfg.clone());
            if !ordered.iter().any(|name| name.eq_ignore_ascii_case(&lower)) {
                extras.push(lower);
            }
        }
        extras.sort();
        for extra in extras {
            if !ordered.iter().any(|name| name.eq_ignore_ascii_case(&extra)) {
                ordered.push(extra);
            }
        }

        for name in ordered.iter() {
            let name_lower = name.to_ascii_lowercase();
            if let Some(cfg) = self
                .config
                .agents
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case(name))
            {
                let builtin = Self::is_builtin_agent(&cfg.name, &cfg.command);
                    let spec_cli = agent_model_spec(&cfg.name)
                        .or_else(|| agent_model_spec(&cfg.command))
                        .map(|spec| spec.cli);
                let command_to_check = command_for_check(&cfg.command);
                let installed = if builtin {
                    true
                } else if command_exists(&command_to_check) {
                    true
                } else if let Some(cli) = spec_cli {
                    command_exists(cli)
                } else {
                    false
                };
                agent_rows.push(AgentOverviewRow {
                    name: cfg.name.clone(),
                    enabled: cfg.enabled && installed,
                    installed,
                    description: Self::agent_description_for(
                        &cfg.name,
                        Some(&cfg.command),
                        cfg.description.as_deref(),
                    ),
                });
            } else if let Some(cfg) = pending_agents.get(&name_lower) {
                let builtin = Self::is_builtin_agent(&cfg.name, &cfg.command);
                let spec_cli = agent_model_spec(&cfg.name)
                    .or_else(|| agent_model_spec(&cfg.command))
                    .map(|spec| spec.cli);
                let command_to_check = command_for_check(&cfg.command);
                let installed = if builtin {
                    true
                } else if command_exists(&command_to_check) {
                    true
                } else if let Some(cli) = spec_cli {
                    command_exists(cli)
                } else {
                    false
                };
                agent_rows.push(AgentOverviewRow {
                    name: cfg.name.clone(),
                    enabled: cfg.enabled && installed,
                    installed,
                    description: Self::agent_description_for(
                        &cfg.name,
                        Some(&cfg.command),
                        cfg.description.as_deref(),
                    ),
                });
            } else {
                let cmd = name.clone();
                let builtin = Self::is_builtin_agent(name, &cmd);
                let spec_cli = agent_model_spec(name).map(|spec| spec.cli);
                let installed = if builtin {
                    true
                } else if let Some(cli) = spec_cli {
                    command_exists(cli)
                } else {
                    command_exists(&cmd)
                };
                agent_rows.push(AgentOverviewRow {
                    name: name.clone(),
                    enabled: installed,
                    installed,
                    description: Self::agent_description_for(name, Some(&cmd), None),
                });
            }
        }

        let mut commands: Vec<String> = vec!["plan".into(), "solve".into(), "code".into()];
        let custom: Vec<String> = self
            .config
            .subagent_commands
            .iter()
            .map(|c| c.name.clone())
            .filter(|name| !commands.iter().any(|builtin| builtin.eq_ignore_ascii_case(name)))
            .collect();
        commands.extend(custom);

        (agent_rows, commands)
    }

    pub(super) fn agent_description_for(
        name: &str,
        command: Option<&str>,
        config_description: Option<&str>,
    ) -> Option<String> {
        if let Some(desc) = config_description {
            let trimmed = desc.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
        agent_model_spec(name)
            .or_else(|| command.and_then(agent_model_spec))
            .map(|spec| spec.description.trim().to_string())
            .filter(|desc| !desc.is_empty())
    }

    pub(super) fn build_agents_settings_content(&mut self) -> AgentsSettingsContent {
        let (rows, commands) = self.collect_agents_overview_rows();
        let total = rows
            .len()
            .saturating_add(commands.len())
            .saturating_add(AGENTS_OVERVIEW_STATIC_ROWS);
        let selected = if total == 0 {
            0
        } else {
            self.agents_overview_selected_index.min(total.saturating_sub(1))
        };
        self.agents_overview_selected_index = selected;
        AgentsSettingsContent::new_overview(rows, commands, selected, self.app_event_tx.clone())
    }

    pub(super) fn build_limits_settings_content(&mut self) -> LimitsSettingsContent {
        let snapshot = self.rate_limit_snapshot.clone();
        let needs_refresh = self.should_refresh_limits();

        let content = if self.rate_limit_fetch_inflight || needs_refresh {
            LimitsOverlayContent::Loading
        } else {
            let reset_info = self.rate_limit_reset_info();
            let tabs = self.build_limits_tabs(snapshot.clone(), reset_info);
            if tabs.is_empty() {
                LimitsOverlayContent::Placeholder
            } else {
                LimitsOverlayContent::Tabs(tabs)
            }
        };

        if needs_refresh {
            self.request_latest_rate_limits(snapshot.is_none());
        }

        LimitsSettingsContent::new(content)
    }

    pub(super) fn build_settings_overview_rows(&mut self) -> Vec<SettingsOverviewRow> {
        SettingsSection::ALL
            .iter()
            .copied()
            .map(|section| {
                let summary = match section {
                    SettingsSection::Model => self.settings_summary_model(),
                    SettingsSection::Theme => self.settings_summary_theme(),
                    SettingsSection::Planning => self.settings_summary_planning(),
                    SettingsSection::Updates => self.settings_summary_updates(),
                    SettingsSection::Accounts => self.settings_summary_accounts(),
                    SettingsSection::Agents => self.settings_summary_agents(),
                    SettingsSection::Prompts => self.settings_summary_prompts(),
                    SettingsSection::Skills => self.settings_summary_skills(),
                    SettingsSection::AutoDrive => self.settings_summary_auto_drive(),
                    SettingsSection::Review => self.settings_summary_review(),
                    SettingsSection::Validation => self.settings_summary_validation(),
                    SettingsSection::Chrome => self.settings_summary_chrome(),
                    SettingsSection::Mcp => self.settings_summary_mcp(),
                    SettingsSection::Notifications => self.settings_summary_notifications(),
                    SettingsSection::Limits => self.settings_summary_limits(),
                };
                SettingsOverviewRow::new(section, summary)
            })
            .collect()
    }

    pub(super) fn settings_summary_model(&self) -> Option<String> {
        let model = self.config.model.trim();
        let model_display_storage;
        let model_display = if model.is_empty() {
            "—"
        } else {
            model_display_storage = Self::format_model_label(model);
            &model_display_storage
        };
        let effort = Self::format_reasoning_effort(self.config.model_reasoning_effort);
        let mut parts: Vec<String> = vec![format!("Model: {} ({})", model_display, effort)];
        if let Some(profile) = self
            .config
            .active_profile
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!("Profile: {profile}"));
        }
        Some(parts.join(" · "))
    }

    pub(super) fn settings_summary_planning(&self) -> Option<String> {
        if self.config.planning_use_chat_model {
            return Some("Model: Follow Chat Mode".to_string());
        }
        let model = self.config.planning_model.trim();
        let model_display_storage;
        let model_display = if model.is_empty() {
            "(default)"
        } else {
            model_display_storage = Self::format_model_label(model);
            &model_display_storage
        };
        let effort = Self::format_reasoning_effort(self.config.planning_model_reasoning_effort);
        Some(format!("Model: {model_display} ({effort})"))
    }

    pub(super) fn settings_summary_theme(&self) -> Option<String> {
        let theme_label = Self::theme_display_name(self.config.tui.theme.name);
        let spinner_name = &self.config.tui.spinner.name;
        let spinner_label = spinner::spinner_label_for(spinner_name);
        Some(format!("Theme: {theme_label} · Spinner: {spinner_label}"))
    }

    pub(super) fn settings_summary_updates(&self) -> Option<String> {
        if !crate::updates::upgrade_ui_enabled() {
            return Some("Auto update: Disabled".to_string());
        }
        let status = if self.config.auto_upgrade_enabled {
            "Enabled"
        } else {
            "Disabled"
        };
        let mut parts = vec![format!("Auto update: {}", status)];
        if let Some(latest) = self
            .latest_upgrade_version
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            parts.push(format!("Latest available: {latest}"));
        }
        Some(parts.join(" · "))
    }

    pub(super) fn settings_summary_accounts(&self) -> Option<String> {
        let auto_switch = if self.config.auto_switch_accounts_on_rate_limit {
            "Auto-switch: On"
        } else {
            "Auto-switch: Off"
        };

        let api_key_fallback = if self.config.api_key_fallback_on_all_accounts_limited {
            "API key fallback: On"
        } else {
            "API key fallback: Off"
        };

        Some(format!("{auto_switch} · {api_key_fallback}"))
    }

    pub(super) fn settings_summary_agents(&self) -> Option<String> {
        let (enabled, total) = agent_summary_counts(&self.config.agents);
        let commands = self.config.subagent_commands.len();
        let mut parts = vec![format!("Enabled: {}/{}", enabled, total)];
        if commands > 0 {
            parts.push(format!("Custom commands: {commands}"));
        }
        Some(parts.join(" · "))
    }

    pub(super) fn settings_summary_auto_drive(&self) -> Option<String> {
        let diagnostics_enabled = self.auto_state.qa_automation_enabled
            && (self.auto_state.review_enabled || self.auto_state.cross_check_enabled);
        let (model_text, effort_text) = if self.config.auto_drive_use_chat_model {
            ("Follow Chat Mode".to_string(), None)
        } else {
            let model_label = if self.config.auto_drive.model.trim().is_empty() {
                "(default)".to_string()
            } else {
                Self::format_model_label(&self.config.auto_drive.model)
            };
            let effort = Self::format_reasoning_effort(self.config.auto_drive.model_reasoning_effort);
            (model_label, Some(effort))
        };
        let model_segment = if let Some(effort) = effort_text {
            format!("Model: {model_text} ({effort})")
        } else {
            format!("Model: {model_text}")
        };
        Some(format!(
            "{} · Agents: {} · Diagnostics: {} · Continue: {}",
            model_segment,
            Self::on_off_label(self.auto_state.subagents_enabled),
            Self::on_off_label(diagnostics_enabled),
            self.auto_state.continue_mode.label()
        ))
    }

    pub(super) fn settings_summary_validation(&self) -> Option<String> {
        let groups = &self.config.validation.groups;
        Some(format!(
            "Functional: {} · Stylistic: {}",
            Self::on_off_label(groups.functional),
            Self::on_off_label(groups.stylistic)
        ))
    }

    pub(super) fn settings_summary_review(&self) -> Option<String> {
        let attempts = self.configured_auto_resolve_re_reviews();
        let auto_followups = self.config.auto_drive.auto_review_followup_attempts.get();

        let review_model_label = if self.config.review_use_chat_model {
            "Chat".to_string()
        } else {
            format!(
                "{} ({})",
                Self::format_model_label(&self.config.review_model),
                Self::format_reasoning_effort(self.config.review_model_reasoning_effort)
            )
        };

        let review_resolve_label = if self.config.review_resolve_use_chat_model {
            "Chat".to_string()
        } else {
            format!(
                "{} ({})",
                Self::format_model_label(&self.config.review_resolve_model),
                Self::format_reasoning_effort(self.config.review_resolve_model_reasoning_effort)
            )
        };

        let auto_review_model_label = if self.config.auto_review_use_chat_model {
            "Chat".to_string()
        } else {
            format!(
                "{} ({})",
                Self::format_model_label(&self.config.auto_review_model),
                Self::format_reasoning_effort(self.config.auto_review_model_reasoning_effort)
            )
        };

        let auto_review_resolve_label = if self.config.auto_review_resolve_use_chat_model {
            "Chat".to_string()
        } else {
            format!(
                "{} ({})",
                Self::format_model_label(&self.config.auto_review_resolve_model),
                Self::format_reasoning_effort(self.config.auto_review_resolve_model_reasoning_effort)
            )
        };

        Some(format!(
            "/review: {} · Resolve: {} · Follow-ups: {} · Auto Review: {} ({} · resolve {} · follow-ups {})",
            review_model_label,
            review_resolve_label,
            attempts,
            Self::on_off_label(self.config.tui.auto_review_enabled),
            auto_review_model_label,
            auto_review_resolve_label,
            auto_followups
        ))
    }

    pub(super) fn settings_summary_limits(&self) -> Option<String> {
        if let Some(snapshot) = &self.rate_limit_snapshot {
            let primary = snapshot.primary_used_percent.clamp(0.0, 100.0).round() as i64;
            let secondary = snapshot.secondary_used_percent.clamp(0.0, 100.0).round() as i64;
            Some(format!("Primary: {primary}% · Secondary: {secondary}%"))
        } else if self.rate_limit_fetch_inflight {
            Some("Refreshing usage...".to_string())
        } else {
            Some("Usage data not loaded".to_string())
        }
    }

    pub(super) fn settings_summary_chrome(&self) -> Option<String> {
        if self.browser_is_external {
            Some("Browser: external".to_string())
        } else {
            Some("Browser: available".to_string())
        }
    }

    pub(super) fn settings_summary_mcp(&self) -> Option<String> {
        Some(format!(
            "Servers configured: {}",
            self.config.mcp_servers.len()
        ))
    }

    pub(super) fn settings_summary_notifications(&self) -> Option<String> {
        match &self.config.tui.notifications {
            Notifications::Enabled(enabled) => {
                Some(format!("Desktop alerts: {}", Self::on_off_label(*enabled)))
            }
            Notifications::Custom(entries) => Some(format!("Custom rules: {}", entries.len())),
        }
    }

    pub(super) fn settings_summary_prompts(&self) -> Option<String> {
        let count = self.bottom_pane.custom_prompts().len();
        Some(format!("Prompts enabled: {count}"))
    }

    pub(super) fn settings_summary_skills(&self) -> Option<String> {
        let count = self.bottom_pane.skills().len();
        Some(format!("Skills loaded: {count}"))
    }

    pub(super) fn refresh_mcp_settings_overlay(&mut self) {
        let content = self.build_mcp_settings_content();
        let Some(content) = content else {
            return;
        };
        let Some(overlay) = self.settings.overlay.as_mut() else {
            return;
        };
        overlay.set_mcp_content(content);
        self.request_redraw();
    }

    pub(super) fn refresh_settings_overview_rows(&mut self) {
        if self.settings.overlay.is_none() {
            return;
        }
        let rows = self.build_settings_overview_rows();
        if let Some(overlay) = self.settings.overlay.as_mut() {
            overlay.set_overview_rows(rows);
        }
        self.request_redraw();
    }

    pub(super) fn format_reasoning_effort(effort: ReasoningEffort) -> &'static str {
        match effort {
            ReasoningEffort::Minimal | ReasoningEffort::None => "Minimal",
            ReasoningEffort::Low => "Low",
            ReasoningEffort::Medium => "Medium",
            ReasoningEffort::High => "High",
            ReasoningEffort::XHigh => "XHigh",
        }
    }

    pub(super) fn format_model_label(model: &str) -> String {
        // Strip the internal "code-" prefix from agent models so user-facing labels
        // display the canonical model name (e.g., code-gpt-5.1-codex-mini -> GPT-5.1-Codex-Mini).
        let model = if model.to_ascii_lowercase().starts_with("code-") {
            &model[5..]
        } else {
            model
        };

        let mut parts = Vec::new();
        for (idx, part) in model.split('-').enumerate() {
            if idx == 0 {
                parts.push(part.to_ascii_uppercase());
                continue;
            }
            let mut chars = part.chars();
            let formatted = match chars.next() {
                Some(first) if first.is_ascii_alphabetic() => {
                    let mut s = String::new();
                    s.push(first.to_ascii_uppercase());
                    s.push_str(chars.as_str());
                    s
                }
                Some(first) => {
                    let mut s = String::new();
                    s.push(first);
                    s.push_str(chars.as_str());
                    s
                }
                None => String::new(),
            };
            parts.push(formatted);
        }
        parts.join("-")
    }

    pub(super) fn on_off_label(value: bool) -> &'static str {
        if value { "On" } else { "Off" }
    }

    pub(super) fn theme_display_name(theme: code_core::config_types::ThemeName) -> String {
        match theme {
            code_core::config_types::ThemeName::LightPhoton => "Light - Photon".to_string(),
            code_core::config_types::ThemeName::LightPhotonAnsi16 => {
                "Light - Photon (16-color)".to_string()
            }
            code_core::config_types::ThemeName::LightPrismRainbow => {
                "Light - Prism Rainbow".to_string()
            }
            code_core::config_types::ThemeName::LightVividTriad => {
                "Light - Vivid Triad".to_string()
            }
            code_core::config_types::ThemeName::LightPorcelain => "Light - Porcelain".to_string(),
            code_core::config_types::ThemeName::LightSandbar => "Light - Sandbar".to_string(),
            code_core::config_types::ThemeName::LightGlacier => "Light - Glacier".to_string(),
            code_core::config_types::ThemeName::DarkCarbonNight => {
                "Dark - Carbon Night".to_string()
            }
            code_core::config_types::ThemeName::DarkCarbonAnsi16 => {
                "Dark - Carbon (16-color)".to_string()
            }
            code_core::config_types::ThemeName::DarkShinobiDusk => {
                "Dark - Shinobi Dusk".to_string()
            }
            code_core::config_types::ThemeName::DarkOledBlackPro => {
                "Dark - OLED Black Pro".to_string()
            }
            code_core::config_types::ThemeName::DarkAmberTerminal => {
                "Dark - Amber Terminal".to_string()
            }
            code_core::config_types::ThemeName::DarkAuroraFlux => "Dark - Aurora Flux".to_string(),
            code_core::config_types::ThemeName::DarkCharcoalRainbow => {
                "Dark - Charcoal Rainbow".to_string()
            }
            code_core::config_types::ThemeName::DarkZenGarden => "Dark - Zen Garden".to_string(),
            code_core::config_types::ThemeName::DarkPaperLightPro => {
                "Dark - Paper Light Pro".to_string()
            }
            code_core::config_types::ThemeName::Custom => {
                let mut label =
                    crate::theme::custom_theme_label().unwrap_or_else(|| "Custom".to_string());
                for pref in ["Light - ", "Dark - ", "Light ", "Dark "] {
                    if label.starts_with(pref) {
                        label = label[pref.len()..].trim().to_string();
                        break;
                    }
                }
                if crate::theme::custom_theme_is_dark().unwrap_or(false) {
                    format!("Dark - {label}")
                } else {
                    format!("Light - {label}")
                }
            }
        }
    }

    pub(crate) fn close_settings_overlay(&mut self) {
        if let Some(overlay) = self.settings.overlay.as_mut() {
            overlay.notify_close();
        }
        self.settings.overlay = None;
        self.request_redraw();
    }

    pub(crate) fn open_settings_section_in_bottom_pane(
        &mut self,
        section: SettingsSection,
    ) -> bool {
        match section {
            SettingsSection::Model => {
                let presets = self.available_model_presets();
                let current_model = self.config.model.clone();
                let current_effort = self.config.model_reasoning_effort;
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.bottom_pane.show_model_selection(
                    presets,
                    current_model,
                    current_effort,
                    false,
                    ModelSelectionTarget::Session,
                );
                true
            }
            SettingsSection::Theme => {
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.show_theme_selection();
                true
            }
            SettingsSection::Updates => {
                let Some(view) = self.prepare_update_settings_view() else {
                    return false;
                };
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.bottom_pane.show_update_settings(view);
                true
            }
            SettingsSection::Accounts => {
                let view = self.build_accounts_settings_view();
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.bottom_pane.show_account_switch_settings(view);
                true
            }
            SettingsSection::Prompts => {
                let view = self.build_prompts_settings_view();
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.bottom_pane.show_prompts_settings(view);
                true
            }
            SettingsSection::Skills => {
                let view = self.build_skills_settings_view();
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.bottom_pane.show_skills_settings(view);
                true
            }
            SettingsSection::AutoDrive => {
                let view = self.build_auto_drive_settings_view();
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.bottom_pane.show_auto_drive_settings_panel(view);
                true
            }
            SettingsSection::Review => {
                let view = self.build_review_settings_view();
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.bottom_pane.show_review_settings(view);
                true
            }
            SettingsSection::Planning => {
                let view = self.build_planning_settings_view();
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.bottom_pane.show_planning_settings(view);
                true
            }
            SettingsSection::Validation => {
                let view = self.build_validation_settings_view();
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.bottom_pane.show_validation_settings(view);
                true
            }
            SettingsSection::Notifications => {
                let view = self.build_notifications_settings_view();
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.bottom_pane.show_notifications_settings(view);
                true
            }
            SettingsSection::Mcp => {
                let Some(rows) = self.build_mcp_server_rows() else {
                    return false;
                };
                if self.settings.overlay.is_some() {
                    self.close_settings_overlay();
                }
                self.bottom_pane.show_mcp_settings(rows);
                true
            }
            SettingsSection::Agents | SettingsSection::Limits | SettingsSection::Chrome => false,
        }
    }

    pub(crate) fn activate_current_settings_section(&mut self) -> bool {
        let section = match self
            .settings
            .overlay
            .as_ref()
            .map(super::settings_overlay::SettingsOverlayView::active_section)
        {
            Some(section) => section,
            None => return false,
        };

        if self.open_settings_section_in_bottom_pane(section) {
            return true;
        }

        let handled = match section {
            SettingsSection::Agents => {
                self.show_agents_overview_ui();
                false
            }
            SettingsSection::Limits => {
                self.show_limits_settings_ui();
                false
            }
            SettingsSection::Chrome => {
                self.show_chrome_options(None);
                true
            }
            SettingsSection::Model
            | SettingsSection::Theme
            | SettingsSection::Planning
            | SettingsSection::Updates
            | SettingsSection::Review
            | SettingsSection::Validation
            | SettingsSection::AutoDrive
            | SettingsSection::Mcp
            | SettingsSection::Notifications
            | SettingsSection::Prompts
            | SettingsSection::Accounts
            | SettingsSection::Skills => false,
        };

        if handled {
            self.close_settings_overlay();
        }

        handled
    }

    pub(crate) fn settings_section_from_hint(section: &str) -> Option<SettingsSection> {
        SettingsSection::from_hint(section)
    }
}
