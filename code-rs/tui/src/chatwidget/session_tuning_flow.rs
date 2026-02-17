use super::*;

impl ChatWidget<'_> {
    pub(super) fn available_model_presets(&self) -> Vec<ModelPreset> {
        if let Some(presets) = self.remote_model_presets.as_ref() {
            return presets.clone();
        }
        let auth_mode = self
            .auth_manager
            .auth()
            .map(|auth| auth.mode)
            .or_else(|| {
                if self.config.using_chatgpt_auth {
                    Some(AuthMode::ChatGPT)
                } else {
                    Some(AuthMode::ApiKey)
                }
            });
        let supports_pro_only_models = self.auth_manager.supports_pro_only_models();
        builtin_model_presets(auth_mode, supports_pro_only_models)
    }

    pub(crate) fn update_model_presets(
        &mut self,
        presets: Vec<ModelPreset>,
        default_model: Option<String>,
    ) {
        if presets.is_empty() {
            return;
        }

        self.remote_model_presets = Some(presets.clone());
        self.bottom_pane.update_model_selection_presets(presets);

        if let Some(default_model) = default_model {
            self.maybe_apply_remote_default_model(default_model);
        }

        self.request_redraw();
    }

    fn maybe_apply_remote_default_model(&mut self, default_model: String) {
        if !self.allow_remote_default_at_startup {
            return;
        }
        if self.chat_model_selected_explicitly {
            return;
        }
        if self.config.model_explicit {
            return;
        }
        if self.config.model.eq_ignore_ascii_case(&default_model) {
            return;
        }

        self.apply_model_selection_inner(default_model, None, false, false);
    }

    fn preset_effort_for_model(preset: &ModelPreset) -> ReasoningEffort {
        preset.default_reasoning_effort.into()
    }

    fn clamp_reasoning_for_model(model: &str, requested: ReasoningEffort) -> ReasoningEffort {
        let protocol_effort: code_protocol::config_types::ReasoningEffort = requested.into();
        let clamped = clamp_reasoning_effort_for_model(model, protocol_effort);
        ReasoningEffort::from(clamped)
    }

    fn find_model_preset(&self, input: &str, presets: &[ModelPreset]) -> Option<ModelPreset> {
        if presets.is_empty() {
            return None;
        }

        let input_lower = input.to_ascii_lowercase();
        let collapsed_input: String = input_lower
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '-')
            .collect();

        let mut fallback_medium: Option<ModelPreset> = None;
        let mut fallback_first: Option<ModelPreset> = None;

        for preset in presets.iter() {
            let preset_effort = Self::preset_effort_for_model(preset);

            let id_lower = preset.id.to_ascii_lowercase();
            if Self::candidate_matches(&input_lower, &collapsed_input, &id_lower) {
                return Some(preset.clone());
            }

            let display_name_lower = preset.display_name.to_ascii_lowercase();
            if Self::candidate_matches(&input_lower, &collapsed_input, &display_name_lower) {
                return Some(preset.clone());
            }

            let effort_lower = preset_effort.to_string().to_ascii_lowercase();
            let model_lower = preset.model.to_ascii_lowercase();
            let spaced = format!("{model_lower} {effort_lower}");
            if Self::candidate_matches(&input_lower, &collapsed_input, &spaced) {
                return Some(preset.clone());
            }
            let dashed = format!("{model_lower}-{effort_lower}");
            if Self::candidate_matches(&input_lower, &collapsed_input, &dashed) {
                return Some(preset.clone());
            }

            if model_lower == input_lower
                || Self::candidate_matches(&input_lower, &collapsed_input, &model_lower)
            {
                if fallback_medium.is_none() && preset_effort == ReasoningEffort::Medium {
                    fallback_medium = Some(preset.clone());
                }
                if fallback_first.is_none() {
                    fallback_first = Some(preset.clone());
                }
            }
        }

        fallback_medium.or(fallback_first)
    }

    fn candidate_matches(input: &str, collapsed_input: &str, candidate: &str) -> bool {
        let candidate_lower = candidate.to_ascii_lowercase();
        if candidate_lower == input {
            return true;
        }
        let candidate_collapsed: String = candidate_lower
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != '-')
            .collect();
        candidate_collapsed == collapsed_input
    }

    pub(crate) fn handle_model_command(&mut self, command_args: String) {
        if self.is_task_running() {
            let message = "'/model' is disabled while a task is in progress.".to_string();
            self.history_push_plain_state(history_cell::new_error_event(message));
            return;
        }

        let presets = self.available_model_presets();
        if presets.is_empty() {
            let message =
                "No model presets are available. Update your configuration to define models."
                    .to_string();
            self.history_push_plain_state(history_cell::new_error_event(message));
            return;
        }

        let trimmed = command_args.trim();
        if !trimmed.is_empty() {
            if let Some(preset) = self.find_model_preset(trimmed, &presets) {
                let effort = Self::preset_effort_for_model(&preset);
                self.apply_model_selection(preset.model, Some(effort));
            } else {
                let message = format!(
                    "Unknown model preset: '{trimmed}'. Use /model with no arguments to open the selector."
                );
                self.history_push_plain_state(history_cell::new_error_event(message));
            }
            return;
        }

        // Check if model selector is already open
        if self.bottom_pane.is_view_kind_active(crate::bottom_pane::ActiveViewKind::ModelSelection) {
            return;
        }

        self.bottom_pane.show_model_selection(
            presets,
            self.config.model.clone(),
            self.config.model_reasoning_effort,
            false,
            ModelSelectionTarget::Session,
        );
    }

    pub(crate) fn show_review_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for review. Update configuration to define models."
                    .to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::Review);
            self.close_settings_overlay();
        }
        self.bottom_pane.show_model_selection(
            presets,
            self.config.review_model.clone(),
            self.config.review_model_reasoning_effort,
            self.config.review_use_chat_model,
            ModelSelectionTarget::Review,
        );
    }

    pub(crate) fn show_review_resolve_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for review resolution.".to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::Review);
            self.close_settings_overlay();
        }
        let current = if self.config.review_resolve_use_chat_model {
            self.config.model.clone()
        } else {
            self.config.review_resolve_model.clone()
        };
        let effort = if self.config.review_resolve_use_chat_model {
            self.config.model_reasoning_effort
        } else {
            self.config.review_resolve_model_reasoning_effort
        };
        self.bottom_pane.show_model_selection(
            presets,
            current,
            effort,
            self.config.review_resolve_use_chat_model,
            ModelSelectionTarget::ReviewResolve,
        );
    }

    pub(crate) fn show_auto_review_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for Auto Review. Update configuration to define models.".to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::Review);
            self.close_settings_overlay();
        }
        let current = if self.config.auto_review_use_chat_model {
            self.config.model.clone()
        } else {
            self.config.auto_review_model.clone()
        };
        let effort = if self.config.auto_review_use_chat_model {
            self.config.model_reasoning_effort
        } else {
            self.config.auto_review_model_reasoning_effort
        };
        self.bottom_pane.show_model_selection(
            presets,
            current,
            effort,
            self.config.auto_review_use_chat_model,
            ModelSelectionTarget::AutoReview,
        );
    }

    pub(crate) fn show_auto_review_resolve_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for Auto Review resolution.".to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::Review);
            self.close_settings_overlay();
        }
        let current = if self.config.auto_review_resolve_use_chat_model {
            self.config.model.clone()
        } else {
            self.config.auto_review_resolve_model.clone()
        };
        let effort = if self.config.auto_review_resolve_use_chat_model {
            self.config.model_reasoning_effort
        } else {
            self.config.auto_review_resolve_model_reasoning_effort
        };
        self.bottom_pane.show_model_selection(
            presets,
            current,
            effort,
            self.config.auto_review_resolve_use_chat_model,
            ModelSelectionTarget::AutoReviewResolve,
        );
    }

    pub(crate) fn show_planning_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for planning. Update configuration to define models."
                    .to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::Planning);
            self.close_settings_overlay();
        }
        let current = if self.config.planning_use_chat_model {
            self.config.model.clone()
        } else {
            self.config.planning_model.clone()
        };
        let effort = self.config.planning_model_reasoning_effort;
        self.bottom_pane
            .show_model_selection(
                presets,
                current,
                effort,
                self.config.planning_use_chat_model,
                ModelSelectionTarget::Planning,
            );
    }

    pub(crate) fn show_auto_drive_model_selector(&mut self) {
        let presets = self.available_model_presets();
        if presets.is_empty() {
            self.bottom_pane.flash_footer_notice(
                "No model presets are available for Auto Drive. Update configuration to define models."
                    .to_string(),
            );
            return;
        }
        if self.settings.overlay.is_some() {
            self.pending_settings_return = Some(SettingsSection::AutoDrive);
            self.close_settings_overlay();
        }
        self.bottom_pane.show_model_selection(
            presets,
            self.config.auto_drive.model.clone(),
            self.config.auto_drive.model_reasoning_effort,
            self.config.auto_drive_use_chat_model,
            ModelSelectionTarget::AutoDrive,
        );
    }

    pub(crate) fn apply_model_selection(&mut self, model: String, effort: Option<ReasoningEffort>) {
        self.apply_model_selection_inner(model, effort, true, true);
    }

    pub(crate) fn apply_shell_selection(
        &mut self,
        path: String,
        args: Vec<String>,
        script_style: Option<String>,
    ) {
        let parsed_style = script_style
            .as_deref()
            .and_then(ShellScriptStyle::parse)
            .or_else(|| ShellScriptStyle::infer_from_shell_program(&path));
        let shell_config =
            Self::build_shell_config(path, args, parsed_style, self.config.shell.as_ref());
        self.update_shell_config(Some(shell_config));
    }

    pub(crate) fn on_shell_selection_closed(&mut self, confirmed: bool) {
        if !confirmed {
            self.history_push_plain_paragraphs(
                crate::history::state::PlainMessageKind::Notice,
                vec!["Shell selection cancelled.".to_string()],
            );
        }
    }

    pub(crate) fn show_shell_selector(&mut self) {
        // Check if shell selector is already open
        if self.bottom_pane.is_view_kind_active(crate::bottom_pane::ActiveViewKind::ShellSelection) {
            return;
        }
        self.bottom_pane
            .show_shell_selection(self.config.shell.clone(), self.available_shell_presets());
    }

    fn clamp_reasoning_for_model_from_presets(
        model: &str,
        requested: ReasoningEffort,
        presets: &[ModelPreset],
    ) -> ReasoningEffort {
        fn rank(effort: ReasoningEffort) -> u8 {
            match effort {
                ReasoningEffort::Minimal => 0,
                ReasoningEffort::Low => 1,
                ReasoningEffort::Medium => 2,
                ReasoningEffort::High => 3,
                ReasoningEffort::XHigh => 4,
                ReasoningEffort::None => 5,
            }
        }

        let model_lower = model.to_ascii_lowercase();
        let Some(preset) = presets.iter().find(|preset| {
            preset.model.eq_ignore_ascii_case(&model_lower)
                || preset.id.eq_ignore_ascii_case(&model_lower)
                || preset.display_name.eq_ignore_ascii_case(&model_lower)
        }) else {
            return Self::clamp_reasoning_for_model(model, requested);
        };

        let supported: Vec<ReasoningEffort> = preset
            .supported_reasoning_efforts
            .iter()
            .map(|opt| ReasoningEffort::from(opt.effort))
            .collect();
        if supported.contains(&requested) {
            return requested;
        }

        let requested_rank = rank(requested);
        supported
            .into_iter()
            .min_by_key(|effort| {
                let effort_rank = rank(*effort);
                (requested_rank.abs_diff(effort_rank), u8::MAX - effort_rank)
            })
            .unwrap_or(requested)
    }

    fn apply_model_selection_inner(
        &mut self,
        model: String,
        effort: Option<ReasoningEffort>,
        mark_explicit: bool,
        announce: bool,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        if mark_explicit {
            self.chat_model_selected_explicitly = true;
            self.config.model_explicit = true;
        }

        let mut updated = false;
        if !self.config.model.eq_ignore_ascii_case(trimmed) {
            self.config.model = trimmed.to_string();
            let family = find_family_for_model(&self.config.model)
                .unwrap_or_else(|| derive_default_model_family(&self.config.model));
            self.config.model_family = family;
            updated = true;
        }

        if let Some(explicit) = effort
            && self.config.preferred_model_reasoning_effort != Some(explicit) {
                self.config.preferred_model_reasoning_effort = Some(explicit);
                updated = true;
            }

        let requested_effort = effort
            .or(self.config.preferred_model_reasoning_effort)
            .unwrap_or(self.config.model_reasoning_effort);
        let presets = self.available_model_presets();
        let clamped_effort = Self::clamp_reasoning_for_model_from_presets(trimmed, requested_effort, &presets);

        if self.config.model_reasoning_effort != clamped_effort {
            self.config.model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if updated {
            let op = Op::ConfigureSession {
                provider: self.config.model_provider.clone(),
                model: self.config.model.clone(),
                model_explicit: self.config.model_explicit,
                model_reasoning_effort: self.config.model_reasoning_effort,
                preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
                model_reasoning_summary: self.config.model_reasoning_summary,
                model_text_verbosity: self.config.model_text_verbosity,
                user_instructions: self.config.user_instructions.clone(),
                base_instructions: self.config.base_instructions.clone(),
                approval_policy: self.config.approval_policy,
                sandbox_policy: self.config.sandbox_policy.clone(),
                disable_response_storage: self.config.disable_response_storage,
                notify: self.config.notify.clone(),
                cwd: self.config.cwd.clone(),
                resume_path: None,
                demo_developer_message: self.config.demo_developer_message.clone(),
                dynamic_tools: Vec::new(),
                shell: self.config.shell.clone(),
            };
            self.submit_op(op);

            self.sync_follow_chat_models();
            self.refresh_settings_overview_rows();
        }

        if announce {
            let placement = self.ui_placement_for_now();
            let state = history_cell::new_model_output(&self.config.model, self.config.model_reasoning_effort);
            let cell = crate::history_cell::PlainHistoryCell::from_state(state.clone());
            self.push_system_cell(
                Box::new(cell),
                placement,
                Some("ui:model".to_string()),
                None,
                "system",
                Some(HistoryDomainRecord::Plain(state)),
            );
        }

        self.request_redraw();
    }

    fn sync_follow_chat_models(&mut self) {
        if self.config.review_use_chat_model {
            self.config.review_model = self.config.model.clone();
            self.config.review_model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_review_settings_model_row();
        }

        if self.config.review_resolve_use_chat_model {
            self.config.review_resolve_model = self.config.model.clone();
            self.config.review_resolve_model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_review_settings_model_row();
        }

        if self.config.planning_use_chat_model {
            self.config.planning_model = self.config.model.clone();
            self.config.planning_model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_planning_settings_model_row();
        }

        if self.config.auto_drive_use_chat_model {
            self.config.auto_drive.model = self.config.model.clone();
            self.config.auto_drive.model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_auto_drive_settings_model_row();
        }

        if self.config.auto_review_use_chat_model {
            self.config.auto_review_model = self.config.model.clone();
            self.config.auto_review_model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_review_settings_model_row();
        }

        if self.config.auto_review_resolve_use_chat_model {
            self.config.auto_review_resolve_model = self.config.model.clone();
            self.config.auto_review_resolve_model_reasoning_effort = self.config.model_reasoning_effort;
            self.update_review_settings_model_row();
        }
    }

    pub(crate) fn apply_review_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.review_use_chat_model = false;

        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self.config.review_model.eq_ignore_ascii_case(trimmed) {
            self.config.review_model = trimmed.to_string();
            updated = true;
        }

        if self.config.review_model_reasoning_effort != clamped_effort {
            self.config.review_model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Review model unchanged.".to_string());
            return;
        }

        let message = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_review_model(
                &home,
                &self.config.review_model,
                self.config.review_model_reasoning_effort,
                self.config.review_use_chat_model,
            ) {
                Ok(_) => format!(
                    "Review model set to {} ({} reasoning)",
                    self.config.review_model,
                    Self::format_reasoning_effort(self.config.review_model_reasoning_effort)
                ),
                Err(err) => {
                    tracing::warn!("Failed to persist review model: {err}");
                    format!(
                        "Review model set for this session (failed to persist): {}",
                        self.config.review_model
                    )
                }
            }
        } else {
            tracing::warn!("Could not locate Code home to persist review model");
            format!(
                "Review model set for this session: {}",
                self.config.review_model
            )
        };

        self.bottom_pane.flash_footer_notice(message);
        self.refresh_settings_overview_rows();
        self.update_review_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn apply_review_resolve_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.review_resolve_use_chat_model = false;

        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self
            .config
            .review_resolve_model
            .eq_ignore_ascii_case(trimmed)
        {
            self.config.review_resolve_model = trimmed.to_string();
            updated = true;
        }

        if self.config.review_resolve_model_reasoning_effort != clamped_effort {
            self.config.review_resolve_model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Resolve model unchanged.".to_string());
            return;
        }

        let message = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_review_resolve_model(
                &home,
                &self.config.review_resolve_model,
                self.config.review_resolve_model_reasoning_effort,
                self.config.review_resolve_use_chat_model,
            ) {
                Ok(_) => format!(
                    "Resolve model set to {} ({} reasoning)",
                    self.config.review_resolve_model,
                    Self::format_reasoning_effort(self.config.review_resolve_model_reasoning_effort)
                ),
                Err(err) => {
                    tracing::warn!("Failed to persist resolve model: {err}");
                    format!(
                        "Resolve model set for this session (failed to persist): {}",
                        self.config.review_resolve_model
                    )
                }
            }
        } else {
            tracing::warn!("Could not locate Code home to persist resolve model");
            format!(
                "Resolve model set for this session: {}",
                self.config.review_resolve_model
            )
        };

        self.bottom_pane.flash_footer_notice(message);
        self.refresh_settings_overview_rows();
        self.update_review_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn set_review_use_chat_model(&mut self, use_chat: bool) {
        if self.config.review_use_chat_model == use_chat {
            return;
        }
        self.config.review_use_chat_model = use_chat;
        if use_chat {
            self.config.review_model = self.config.model.clone();
            self.config.review_model_reasoning_effort = self.config.model_reasoning_effort;
        }

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_review_model(
                &home,
                &self.config.review_model,
                self.config.review_model_reasoning_effort,
                use_chat,
            ) {
                tracing::warn!("Failed to persist review use-chat toggle: {err}");
            }

        let notice = if use_chat {
            "Review model now follows Chat model".to_string()
        } else {
            format!(
                "Review model set to {} ({} reasoning)",
                self.config.review_model,
                Self::format_reasoning_effort(self.config.review_model_reasoning_effort)
            )
        };
        self.bottom_pane.flash_footer_notice(notice);
        self.update_review_settings_model_row();
        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    pub(crate) fn set_review_resolve_use_chat_model(&mut self, use_chat: bool) {
        if self.config.review_resolve_use_chat_model == use_chat {
            return;
        }
        self.config.review_resolve_use_chat_model = use_chat;
        if use_chat {
            self.config.review_resolve_model = self.config.model.clone();
            self.config.review_resolve_model_reasoning_effort = self.config.model_reasoning_effort;
        }

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_review_resolve_model(
                &home,
                &self.config.review_resolve_model,
                self.config.review_resolve_model_reasoning_effort,
                use_chat,
            ) {
                tracing::warn!("Failed to persist resolve use-chat toggle: {err}");
            }

        let notice = if use_chat {
            "Resolve model now follows Chat model".to_string()
        } else {
            format!(
                "Resolve model set to {} ({} reasoning)",
                self.config.review_resolve_model,
                Self::format_reasoning_effort(self.config.review_resolve_model_reasoning_effort)
            )
        };
        self.bottom_pane.flash_footer_notice(notice);
        self.update_review_settings_model_row();
        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    pub(crate) fn apply_auto_review_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.auto_review_use_chat_model = false;
        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self
            .config
            .auto_review_model
            .eq_ignore_ascii_case(trimmed)
        {
            self.config.auto_review_model = trimmed.to_string();
            updated = true;
        }

        if self.config.auto_review_model_reasoning_effort != clamped_effort {
            self.config.auto_review_model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Auto Review model unchanged.".to_string());
            return;
        }

        let notice = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_auto_review_model(
                &home,
                &self.config.auto_review_model,
                self.config.auto_review_model_reasoning_effort,
                self.config.auto_review_use_chat_model,
            ) {
                Ok(_) => format!(
                    "Auto Review model set to {} ({} reasoning)",
                    self.config.auto_review_model,
                    Self::format_reasoning_effort(self.config.auto_review_model_reasoning_effort)
                ),
                Err(err) => {
                    tracing::warn!("Failed to persist Auto Review model: {err}");
                    format!(
                        "Auto Review model set for this session (failed to persist): {}",
                        self.config.auto_review_model
                    )
                }
            }
        } else {
            tracing::warn!("Could not locate Code home to persist Auto Review model");
            format!(
                "Auto Review model set for this session: {}",
                self.config.auto_review_model
            )
        };

        self.bottom_pane.flash_footer_notice(notice);
        self.refresh_settings_overview_rows();
        self.update_review_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn set_auto_review_use_chat_model(&mut self, use_chat: bool) {
        if self.config.auto_review_use_chat_model == use_chat {
            return;
        }
        self.config.auto_review_use_chat_model = use_chat;
        if use_chat {
            self.config.auto_review_model = self.config.model.clone();
            self.config.auto_review_model_reasoning_effort = self.config.model_reasoning_effort;
        }

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_auto_review_model(
                &home,
                &self.config.auto_review_model,
                self.config.auto_review_model_reasoning_effort,
                use_chat,
            ) {
                tracing::warn!("Failed to persist Auto Review use-chat toggle: {err}");
            }

        let notice = if use_chat {
            "Auto Review model now follows Chat model".to_string()
        } else {
            format!(
                "Auto Review model set to {} ({} reasoning)",
                self.config.auto_review_model,
                Self::format_reasoning_effort(self.config.auto_review_model_reasoning_effort)
            )
        };
        self.bottom_pane.flash_footer_notice(notice);
        self.update_review_settings_model_row();
        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    pub(crate) fn apply_auto_review_resolve_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.auto_review_resolve_use_chat_model = false;
        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self
            .config
            .auto_review_resolve_model
            .eq_ignore_ascii_case(trimmed)
        {
            self.config.auto_review_resolve_model = trimmed.to_string();
            updated = true;
        }

        if self.config.auto_review_resolve_model_reasoning_effort != clamped_effort {
            self.config.auto_review_resolve_model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Auto Review resolve model unchanged.".to_string());
            return;
        }

        let notice = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_auto_review_resolve_model(
                &home,
                &self.config.auto_review_resolve_model,
                self.config.auto_review_resolve_model_reasoning_effort,
                self.config.auto_review_resolve_use_chat_model,
            ) {
                Ok(_) => format!(
                    "Auto Review resolve model set to {} ({} reasoning)",
                    self.config.auto_review_resolve_model,
                    Self::format_reasoning_effort(self.config.auto_review_resolve_model_reasoning_effort)
                ),
                Err(err) => {
                    tracing::warn!("Failed to persist Auto Review resolve model: {err}");
                    format!(
                        "Auto Review resolve model set for this session (failed to persist): {}",
                        self.config.auto_review_resolve_model
                    )
                }
            }
        } else {
            tracing::warn!("Could not locate Code home to persist Auto Review resolve model");
            format!(
                "Auto Review resolve model set for this session: {}",
                self.config.auto_review_resolve_model
            )
        };

        self.bottom_pane.flash_footer_notice(notice);
        self.refresh_settings_overview_rows();
        self.update_review_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn set_auto_review_resolve_use_chat_model(&mut self, use_chat: bool) {
        if self.config.auto_review_resolve_use_chat_model == use_chat {
            return;
        }
        self.config.auto_review_resolve_use_chat_model = use_chat;
        if use_chat {
            self.config.auto_review_resolve_model = self.config.model.clone();
            self.config.auto_review_resolve_model_reasoning_effort =
                self.config.model_reasoning_effort;
        }

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_auto_review_resolve_model(
                &home,
                &self.config.auto_review_resolve_model,
                self.config.auto_review_resolve_model_reasoning_effort,
                use_chat,
            ) {
                tracing::warn!("Failed to persist Auto Review resolve use-chat toggle: {err}");
            }

        let notice = if use_chat {
            "Auto Review resolve model now follows Chat model".to_string()
        } else {
            format!(
                "Auto Review resolve model set to {} ({} reasoning)",
                self.config.auto_review_resolve_model,
                Self::format_reasoning_effort(self.config.auto_review_resolve_model_reasoning_effort)
            )
        };
        self.bottom_pane.flash_footer_notice(notice);
        self.update_review_settings_model_row();
        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    pub(crate) fn set_auto_drive_use_chat_model(&mut self, use_chat: bool) {
        if self.config.auto_drive_use_chat_model == use_chat {
            return;
        }
        self.config.auto_drive_use_chat_model = use_chat;
        if use_chat {
            self.config.auto_drive.model = self.config.model.clone();
            self.config.auto_drive.model_reasoning_effort = self.config.model_reasoning_effort;
        }

        self.restore_auto_resolve_attempts_if_lost();

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_auto_drive_settings(
                &home,
                &self.config.auto_drive,
                use_chat,
            ) {
                tracing::warn!("Failed to persist Auto Drive use-chat toggle: {err}");
            }

        let notice = if use_chat {
            "Auto Drive model now follows Chat model".to_string()
        } else {
            format!(
                "Auto Drive model set to {} ({} reasoning)",
                self.config.auto_drive.model,
                Self::format_reasoning_effort(self.config.auto_drive.model_reasoning_effort)
            )
        };

        self.bottom_pane.flash_footer_notice(notice);
        self.refresh_settings_overview_rows();
        self.update_auto_drive_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn handle_model_selection_closed(&mut self, target: ModelSelectionKind, _accepted: bool) {
        let expected_section = match target {
            ModelSelectionKind::Session => SettingsSection::Model,
            ModelSelectionKind::Review => SettingsSection::Review,
            ModelSelectionKind::Planning => SettingsSection::Planning,
            ModelSelectionKind::AutoDrive => SettingsSection::AutoDrive,
            ModelSelectionKind::ReviewResolve => SettingsSection::Review,
            ModelSelectionKind::AutoReview => SettingsSection::Review,
            ModelSelectionKind::AutoReviewResolve => SettingsSection::Review,
        };

        if let Some(section) = self.pending_settings_return {
            if section == expected_section {
                self.ensure_settings_overlay_section(section);
            }
            self.pending_settings_return = None;
        }

        self.request_redraw();
    }

    pub(crate) fn apply_planning_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.planning_use_chat_model = false;

        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self.config.planning_model.eq_ignore_ascii_case(trimmed) {
            self.config.planning_model = trimmed.to_string();
            updated = true;
        }
        if self.config.planning_model_reasoning_effort != clamped_effort {
            self.config.planning_model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Planning model unchanged.".to_string());
            return;
        }

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_planning_model(
                &home,
                &self.config.planning_model,
                self.config.planning_model_reasoning_effort,
                false,
            ) {
                tracing::warn!("Failed to persist planning model: {err}");
            }

        self.bottom_pane.flash_footer_notice(format!(
            "Planning model set to {} ({} reasoning)",
            self.config.planning_model,
            Self::format_reasoning_effort(self.config.planning_model_reasoning_effort)
        ));
        self.refresh_settings_overview_rows();
        self.update_planning_settings_model_row();
        // If we're currently in plan mode, switch the session model immediately.
        if matches!(self.config.sandbox_policy, code_core::protocol::SandboxPolicy::ReadOnly) {
            self.apply_planning_session_model();
        }
        self.request_redraw();
    }

    pub(super) fn apply_planning_session_model(&mut self) {
        if self.config.planning_use_chat_model {
            self.restore_planning_session_model();
            return;
        }

        // If we're already on the planning model, do nothing.
        if self.config.model.eq_ignore_ascii_case(&self.config.planning_model)
            && self.config.model_reasoning_effort == self.config.planning_model_reasoning_effort
        {
            return;
        }

        // Save current chat model to restore later.
        self.planning_restore = Some((
            self.config.model.clone(),
            self.config.model_reasoning_effort,
        ));

        self.config.model = self.config.planning_model.clone();
        self.config.model_reasoning_effort = self.config.planning_model_reasoning_effort;

        let op = Op::ConfigureSession {
            provider: self.config.model_provider.clone(),
            model: self.config.model.clone(),
            model_explicit: self.config.model_explicit,
            model_reasoning_effort: self.config.model_reasoning_effort,
            preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
            model_reasoning_summary: self.config.model_reasoning_summary,
            model_text_verbosity: self.config.model_text_verbosity,
            user_instructions: self.config.user_instructions.clone(),
            base_instructions: self.config.base_instructions.clone(),
            approval_policy: self.config.approval_policy,
            sandbox_policy: self.config.sandbox_policy.clone(),
            disable_response_storage: self.config.disable_response_storage,
            notify: self.config.notify.clone(),
            cwd: self.config.cwd.clone(),
            resume_path: None,
            demo_developer_message: self.config.demo_developer_message.clone(),
            dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
        };
        self.submit_op(op);
    }

    pub(super) fn restore_planning_session_model(&mut self) {
        if let Some((model, effort)) = self.planning_restore.take() {
            self.config.model = model;
            self.config.model_reasoning_effort = effort;

            let op = Op::ConfigureSession {
                provider: self.config.model_provider.clone(),
                model: self.config.model.clone(),
                model_explicit: self.config.model_explicit,
                model_reasoning_effort: self.config.model_reasoning_effort,
                preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
                model_reasoning_summary: self.config.model_reasoning_summary,
                model_text_verbosity: self.config.model_text_verbosity,
                user_instructions: self.config.user_instructions.clone(),
                base_instructions: self.config.base_instructions.clone(),
                approval_policy: self.config.approval_policy,
                sandbox_policy: self.config.sandbox_policy.clone(),
                disable_response_storage: self.config.disable_response_storage,
                notify: self.config.notify.clone(),
                cwd: self.config.cwd.clone(),
                resume_path: None,
                demo_developer_message: self.config.demo_developer_message.clone(),
                dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
            };
            self.submit_op(op);
        }
    }

    pub(crate) fn set_planning_use_chat_model(&mut self, use_chat: bool) {
        if self.config.planning_use_chat_model == use_chat {
            return;
        }
        self.config.planning_use_chat_model = use_chat;

        if let Ok(home) = code_core::config::find_code_home()
            && let Err(err) = code_core::config::set_planning_model(
                &home,
                &self.config.planning_model,
                self.config.planning_model_reasoning_effort,
                use_chat,
            ) {
                tracing::warn!("Failed to persist planning use-chat toggle: {err}");
            }

        if use_chat {
            self.bottom_pane
                .flash_footer_notice("Planning model now follows Chat model".to_string());
        } else {
            self.bottom_pane.flash_footer_notice(format!(
                "Planning model set to {} ({} reasoning)",
                self.config.planning_model,
                Self::format_reasoning_effort(self.config.planning_model_reasoning_effort)
            ));
        }

        self.update_planning_settings_model_row();
        self.refresh_settings_overview_rows();

        if matches!(self.config.sandbox_policy, code_core::protocol::SandboxPolicy::ReadOnly) {
            self.apply_planning_session_model();
        }
        self.request_redraw();
    }

    pub(crate) fn apply_auto_drive_model_selection(
        &mut self,
        model: String,
        effort: ReasoningEffort,
    ) {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return;
        }

        self.config.auto_drive_use_chat_model = false;

        let clamped_effort = Self::clamp_reasoning_for_model(trimmed, effort);

        let mut updated = false;
        if !self.config.auto_drive.model.eq_ignore_ascii_case(trimmed) {
            self.config.auto_drive.model = trimmed.to_string();
            updated = true;
        }

        if self.config.auto_drive.model_reasoning_effort != clamped_effort {
            self.config.auto_drive.model_reasoning_effort = clamped_effort;
            updated = true;
        }

        if !updated {
            self.bottom_pane
                .flash_footer_notice("Auto Drive model unchanged.".to_string());
            return;
        }

        let message = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_auto_drive_settings(
                &home,
                &self.config.auto_drive,
                self.config.auto_drive_use_chat_model,
            ) {
                Ok(_) => format!(
                    "Auto Drive model set to {} ({} reasoning)",
                    self.config.auto_drive.model,
                    Self::format_reasoning_effort(self.config.auto_drive.model_reasoning_effort)
                ),
                Err(err) => {
                    tracing::warn!("Failed to persist Auto Drive model: {err}");
                    format!(
                        "Auto Drive model set for this session (failed to persist): {}",
                        self.config.auto_drive.model
                    )
                }
            }
        } else {
            tracing::warn!("Could not locate Code home to persist Auto Drive model");
            format!(
                "Auto Drive model set for this session: {}",
                self.config.auto_drive.model
            )
        };

        self.bottom_pane.flash_footer_notice(message);
        self.refresh_settings_overview_rows();
        self.update_auto_drive_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn handle_reasoning_command(&mut self, command_args: String) {
        // command_args contains only the arguments after the command (e.g., "high" not "/reasoning high")
        let trimmed = command_args.trim();

        if !trimmed.is_empty() {
            // User specified a level: e.g., "high"
            let new_effort = match trimmed.to_lowercase().as_str() {
                "minimal" | "min" => ReasoningEffort::Minimal,
                "low" => ReasoningEffort::Low,
                "medium" | "med" => ReasoningEffort::Medium,
                "xhigh" | "extra-high" | "extra_high" => ReasoningEffort::XHigh,
                "high" => ReasoningEffort::High,
                // Backwards compatibility: map legacy values to minimal.
                "none" | "off" => ReasoningEffort::Minimal,
                _ => {
                    // Invalid parameter, show error and return
                    let message = format!(
                        "Invalid reasoning level: '{trimmed}'. Use: minimal, low, medium, or high"
                    );
                    self.history_push_plain_state(history_cell::new_error_event(message));
                    return;
                }
            };
            self.set_reasoning_effort(new_effort);
        } else {
            let presets = self.available_model_presets();
            if presets.is_empty() {
                let message =
                    "No model presets are available. Update your configuration to define models."
                        .to_string();
                self.history_push_plain_state(history_cell::new_error_event(message));
                return;
            }

        self.bottom_pane.show_model_selection(
            presets,
            self.config.model.clone(),
            self.config.model_reasoning_effort,
            false,
            ModelSelectionTarget::Session,
        );
        }
    }

    pub(crate) fn handle_verbosity_command(&mut self, command_args: String) {
        // Verbosity is not supported with ChatGPT auth
        if self.config.using_chatgpt_auth {
            let message =
                "Text verbosity is not available when using Sign in with ChatGPT".to_string();
            self.history_push_plain_state(history_cell::new_error_event(message));
            return;
        }

        // command_args contains only the arguments after the command (e.g., "high" not "/verbosity high")
        let trimmed = command_args.trim();

        if !trimmed.is_empty() {
            // User specified a level: e.g., "high"
            let new_verbosity = match trimmed.to_lowercase().as_str() {
                "low" => TextVerbosity::Low,
                "medium" | "med" => TextVerbosity::Medium,
                "high" => TextVerbosity::High,
                _ => {
                    // Invalid parameter, show error and return
                    let message = format!(
                        "Invalid verbosity level: '{trimmed}'. Use: low, medium, or high"
                    );
                    self.history_push_plain_state(history_cell::new_error_event(message));
                    return;
                }
            };

            // Update the configuration
            self.config.model_text_verbosity = new_verbosity;

            // Display success message
            let message = format!("Text verbosity set to: {new_verbosity}");
            self.push_background_tail(message);

            // Send the update to the backend
            let op = Op::ConfigureSession {
                provider: self.config.model_provider.clone(),
                model: self.config.model.clone(),
                model_explicit: self.config.model_explicit,
                model_reasoning_effort: self.config.model_reasoning_effort,
                preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
                model_reasoning_summary: self.config.model_reasoning_summary,
                model_text_verbosity: self.config.model_text_verbosity,
                user_instructions: self.config.user_instructions.clone(),
                base_instructions: self.config.base_instructions.clone(),
                approval_policy: self.config.approval_policy,
                sandbox_policy: self.config.sandbox_policy.clone(),
                disable_response_storage: self.config.disable_response_storage,
                notify: self.config.notify.clone(),
                cwd: self.config.cwd.clone(),
                resume_path: None,
                demo_developer_message: self.config.demo_developer_message.clone(),
                dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
            };
            let _ = self.code_op_tx.send(op);
        } else {
            // No parameter specified, show interactive UI
            self.bottom_pane
                .show_verbosity_selection(self.config.model_text_verbosity);
        }
    }

    pub(crate) fn prepare_agents(&mut self) {
        // Set the flag to show agents are ready to start
        self.agents_ready_to_start = true;
        self.agents_terminal.reset();
        if self.agents_terminal.active {
            // Reset scroll offset when a new batch starts to avoid stale positions
            self.layout.scroll_offset.set(0);
        }

        // Initialize sparkline with some data so it shows immediately
        {
            let mut sparkline_data = self.sparkline_data.borrow_mut();
            if sparkline_data.is_empty() {
                // Add initial low activity data for preparing phase
                for _ in 0..10 {
                    sparkline_data.push((2, false));
                }
                tracing::info!(
                    "Initialized sparkline data with {} points for preparing phase",
                    sparkline_data.len()
                );
            }
        } // Drop the borrow here

        self.request_redraw();
    }

    /// Update sparkline data with randomized activity based on agent count
    pub(super) fn update_sparkline_data(&self) {
        let now = std::time::Instant::now();

        // Update every 100ms for smooth animation
        if now
            .duration_since(*self.last_sparkline_update.borrow())
            .as_millis()
            < 100
        {
            return;
        }

        *self.last_sparkline_update.borrow_mut() = now;

        // Calculate base height based on number of agents and status
        let agent_count = self.active_agents.len();
        let is_planning = self.overall_task_status == "planning";
        let base_height = if agent_count == 0 && self.agents_ready_to_start {
            2 // Minimal activity when preparing
        } else if is_planning && agent_count > 0 {
            3 // Low activity during planning phase
        } else if agent_count == 1 {
            5 // Low activity for single agent
        } else if agent_count == 2 {
            10 // Medium activity for two agents
        } else if agent_count >= 3 {
            15 // High activity for multiple agents
        } else {
            0 // No activity when no agents
        };

        // Don't generate data if there's no activity
        if base_height == 0 {
            return;
        }

        // Generate random variation
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hash;
        use std::hash::Hasher;
        let mut hasher = DefaultHasher::new();
        now.elapsed().as_nanos().hash(&mut hasher);
        let random_seed = hasher.finish();

        // More variation during planning phase for visibility (+/- 50%)
        // Less variation during running for stability (+/- 30%)
        let variation_percent = if self.agents_ready_to_start && self.active_agents.is_empty() {
            50 // More variation during planning for visibility
        } else {
            30 // Standard variation during running
        };

        let variation_range = variation_percent * 2; // e.g., 100 for +/- 50%
        let variation = ((random_seed % variation_range) as i32 - variation_percent as i32)
            * base_height
            / 100;
        let height = ((base_height + variation).max(1) as u64).min(20);

        // Check if any agents are completed
        let has_completed = self
            .active_agents
            .iter()
            .any(|a| matches!(a.status, AgentStatus::Completed));

        // Keep a rolling window of 60 data points (about 6 seconds at 100ms intervals)
        let mut sparkline_data = self.sparkline_data.borrow_mut();
        sparkline_data.push((height, has_completed));
        if sparkline_data.len() > 60 {
            sparkline_data.remove(0);
        }
    }

    pub(crate) fn set_reasoning_effort(&mut self, new_effort: ReasoningEffort) {
        let clamped_effort = Self::clamp_reasoning_for_model(&self.config.model, new_effort);

        if clamped_effort != new_effort {
            let requested = Self::format_reasoning_effort(new_effort);
            let applied = Self::format_reasoning_effort(clamped_effort);
            self.bottom_pane.flash_footer_notice(format!(
                "{} does not support {} reasoning; using {} instead.",
                self.config.model, requested, applied
            ));
        }

        // Update the config
        self.config.preferred_model_reasoning_effort = Some(new_effort);
        self.config.model_reasoning_effort = clamped_effort;

        // Send ConfigureSession op to update the backend
        let op = Op::ConfigureSession {
            provider: self.config.model_provider.clone(),
            model: self.config.model.clone(),
            model_explicit: self.config.model_explicit,
            model_reasoning_effort: clamped_effort,
            preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
            model_reasoning_summary: self.config.model_reasoning_summary,
            model_text_verbosity: self.config.model_text_verbosity,
            user_instructions: self.config.user_instructions.clone(),
            base_instructions: self.config.base_instructions.clone(),
            approval_policy: self.config.approval_policy,
            sandbox_policy: self.config.sandbox_policy.clone(),
            disable_response_storage: self.config.disable_response_storage,
            notify: self.config.notify.clone(),
            cwd: self.config.cwd.clone(),
            resume_path: None,
            demo_developer_message: self.config.demo_developer_message.clone(),
            dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
        };

        self.submit_op(op);

        // Add status message to history (replaceable system notice)
        let placement = self.ui_placement_for_now();
        let state = history_cell::new_reasoning_output(self.config.model_reasoning_effort);
        let cell = crate::history_cell::PlainHistoryCell::from_state(state.clone());
        self.push_system_cell(
            Box::new(cell),
            placement,
            Some("ui:reasoning".to_string()),
            None,
            "system",
            Some(HistoryDomainRecord::Plain(state)),
        );
        self.refresh_settings_overview_rows();
    }

    pub(crate) fn set_text_verbosity(&mut self, new_verbosity: TextVerbosity) {
        // Update the config
        self.config.model_text_verbosity = new_verbosity;

        // Send ConfigureSession op to update the backend
        let op = Op::ConfigureSession {
            provider: self.config.model_provider.clone(),
            model: self.config.model.clone(),
            model_explicit: self.config.model_explicit,
            model_reasoning_effort: self.config.model_reasoning_effort,
            preferred_model_reasoning_effort: self.config.preferred_model_reasoning_effort,
            model_reasoning_summary: self.config.model_reasoning_summary,
            model_text_verbosity: new_verbosity,
            user_instructions: self.config.user_instructions.clone(),
            base_instructions: self.config.base_instructions.clone(),
            approval_policy: self.config.approval_policy,
            sandbox_policy: self.config.sandbox_policy.clone(),
            disable_response_storage: self.config.disable_response_storage,
            notify: self.config.notify.clone(),
            cwd: self.config.cwd.clone(),
            resume_path: None,
            demo_developer_message: self.config.demo_developer_message.clone(),
            dynamic_tools: Vec::new(),
            shell: self.config.shell.clone(),
        };

        self.submit_op(op);

        // Add status message to history
        let message = format!("Text verbosity set to: {new_verbosity}");
        self.push_background_tail(message);
    }

}
