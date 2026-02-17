use super::*;
use code_login::AuthMode;

impl ThemeSelectionView {
    pub fn new(
        current_theme: ThemeName,
        app_event_tx: AppEventSender,
        tail_ticket: BackgroundOrderTicket,
        before_ticket: BackgroundOrderTicket,
    ) -> Self {
        let current_theme = map_theme_for_palette(current_theme, custom_theme_is_dark());
        let selected_theme_index = Self::theme_index_for(current_theme);

        // Initialize spinner selection from current runtime spinner
        let spinner_names = crate::spinner::spinner_names();
        let current_spinner_name = crate::spinner::current_spinner().name.clone();
        let selected_spinner_index = spinner_names
            .iter()
            .position(|n| *n == current_spinner_name)
            .unwrap_or(0);

        Self {
            original_theme: current_theme,
            current_theme,
            selected_theme_index,
            hovered_theme_index: None,
            _original_spinner: current_spinner_name.clone(),
            current_spinner: current_spinner_name.clone(),
            selected_spinner_index,
            mode: Mode::Overview,
            overview_selected_index: 0,
            revert_theme_on_back: current_theme,
            revert_spinner_on_back: current_spinner_name,
            just_entered_themes: false,
            just_entered_spinner: false,
            app_event_tx,
            tail_ticket,
            before_ticket,
            is_complete: false,
        }
    }

    pub(super) fn custom_theme_display_name() -> String {
        let Some(label0) = crate::theme::custom_theme_label() else {
            return "Custom".to_string();
        };

        // Sanitize any leading Light/Dark prefix the model may have included.
        let mut label = label0.trim().to_string();
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

    pub(super) fn get_theme_options() -> Vec<(ThemeName, Cow<'static, str>, &'static str)> {
        let builtins = Self::builtin_theme_options();
        let mut out: Vec<(ThemeName, Cow<'static, str>, &'static str)> =
            Vec::with_capacity(builtins.len() + 1);
        out.extend(
            builtins
                .iter()
                .map(|(theme, name, description)| (*theme, Cow::Borrowed(*name), *description)),
        );

        if !matches!(palette_mode(), PaletteMode::Ansi16)
            && crate::theme::custom_theme_label().is_some()
        {
            out.push((
                ThemeName::Custom,
                Cow::Owned(Self::custom_theme_display_name()),
                "Your saved custom theme",
            ));
        }

        out
    }

    pub(super) fn builtin_theme_options() -> &'static [ThemeOption] {
        if matches!(palette_mode(), PaletteMode::Ansi16) {
            THEME_OPTIONS_ANSI16
        } else {
            THEME_OPTIONS_ANSI256
        }
    }

    pub(super) fn has_custom_theme_option() -> bool {
        !matches!(palette_mode(), PaletteMode::Ansi16)
            && crate::theme::custom_theme_label().is_some()
    }

    pub(super) fn theme_option_count() -> usize {
        Self::builtin_theme_options().len() + if Self::has_custom_theme_option() { 1 } else { 0 }
    }

    pub(super) fn theme_index_for(theme_name: ThemeName) -> usize {
        if let Some(idx) = Self::builtin_theme_options()
            .iter()
            .position(|(theme, _, _)| *theme == theme_name)
        {
            return idx;
        }

        if matches!(theme_name, ThemeName::Custom) && Self::has_custom_theme_option() {
            return Self::builtin_theme_options().len();
        }

        0
    }

    pub(super) fn theme_name_for_option_index(index: usize) -> Option<ThemeName> {
        let builtins = Self::builtin_theme_options();
        if index < builtins.len() {
            return Some(builtins[index].0);
        }

        if Self::has_custom_theme_option() && index == builtins.len() {
            Some(ThemeName::Custom)
        } else {
            None
        }
    }

    pub(super) fn allow_custom_theme_generation() -> bool {
        !matches!(palette_mode(), PaletteMode::Ansi16)
    }

    pub(super) fn clear_hovered_theme_preview(&mut self) -> bool {
        if self.hovered_theme_index.take().is_some() {
            self.send_theme_split_preview();
            true
        } else {
            false
        }
    }

    pub(super) fn theme_list_count(options_len: usize) -> usize {
        options_len + if Self::allow_custom_theme_generation() { 1 } else { 0 }
    }

    pub(super) fn visible_theme_rows(list_height: u16) -> usize {
        (list_height as usize).saturating_sub(1).clamp(1, 9)
    }

    pub(super) fn theme_mode_areas(body_area: Rect) -> (Rect, Rect) {
        if body_area.width < 2 {
            return (body_area, Rect::default());
        }
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(body_area);
        if cols.len() >= 2 {
            (cols[0], cols[1])
        } else {
            (body_area, Rect::default())
        }
    }

    pub(super) fn theme_preview_list_area(preview_area: Rect) -> Rect {
        if preview_area.width == 0 || preview_area.height == 0 {
            return Rect::default();
        }
        let inner = Block::default().borders(Borders::ALL).inner(preview_area);
        if inner.width == 0 || inner.height == 0 {
            return Rect::default();
        }
        let list_height = inner.height.saturating_sub(5).max(3);
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: list_height,
        }
    }

    pub(super) fn theme_index_at_mouse_position(
        &self,
        mouse_event: MouseEvent,
        list_area: Rect,
        options_len: usize,
    ) -> Option<usize> {
        if list_area.width == 0 || list_area.height == 0 {
            return None;
        }
        if mouse_event.column < list_area.x
            || mouse_event.column >= list_area.x.saturating_add(list_area.width)
            || mouse_event.row < list_area.y
            || mouse_event.row >= list_area.y.saturating_add(list_area.height)
        {
            return None;
        }

        let rel_y = mouse_event.row.saturating_sub(list_area.y) as usize;
        if rel_y == 0 {
            return None;
        }
        let visible = Self::visible_theme_rows(list_area.height);
        let row = rel_y - 1;
        if row >= visible {
            return None;
        }

        let count = Self::theme_list_count(options_len);
        if count == 0 {
            return None;
        }
        let (start, _, _) =
            crate::util::list_window::anchored_window(self.selected_theme_index, count, visible);
        let idx = start + row;
        if idx < count {
            Some(idx)
        } else {
            None
        }
    }

    pub(super) fn theme_display_name(theme_name: ThemeName) -> String {
        if matches!(theme_name, ThemeName::Custom) {
            return crate::theme::custom_theme_label().unwrap_or_else(|| "Custom".to_string());
        }
        Self::builtin_theme_options()
            .iter()
            .find(|(candidate, _, _)| *candidate == theme_name)
            .map(|(_, name, _)| (*name).to_string())
            .unwrap_or_else(|| "Theme".to_string())
    }

    pub(super) fn render_theme_preview_column(
        area: Rect,
        title: &str,
        palette: &crate::theme::Theme,
        buf: &mut Buffer,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title.to_string())
            .border_style(Style::default().fg(palette.border));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let base = Style::default().bg(palette.background).fg(palette.text);
        for y in inner.y..inner.y.saturating_add(inner.height) {
            for x in inner.x..inner.x.saturating_add(inner.width) {
                let cell = &mut buf[(x, y)];
                cell.set_char(' ');
                cell.set_style(base);
            }
        }

        let lines = vec![
            Line::from(Span::styled(
                "Aa Sample Header",
                Style::default()
                    .bg(palette.background)
                    .fg(palette.text_bright)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(vec![
                Span::styled(
                    "Normal",
                    Style::default().bg(palette.background).fg(palette.text),
                ),
                Span::raw(" "),
                Span::styled(
                    "Dim",
                    Style::default().bg(palette.background).fg(palette.text_dim),
                ),
                Span::raw(" "),
                Span::styled(
                    "Bright",
                    Style::default().bg(palette.background).fg(palette.text_bright),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    "Primary",
                    Style::default()
                        .bg(palette.background)
                        .fg(palette.primary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    "Info",
                    Style::default().bg(palette.background).fg(palette.info),
                ),
                Span::raw(" "),
                Span::styled(
                    "Warn",
                    Style::default().bg(palette.background).fg(palette.warning),
                ),
                Span::raw(" "),
                Span::styled(
                    "Err",
                    Style::default().bg(palette.background).fg(palette.error),
                ),
            ]),
            Line::from(vec![
                Span::styled("   ", Style::default().bg(palette.primary)),
                Span::raw(" "),
                Span::styled("   ", Style::default().bg(palette.secondary)),
                Span::raw(" "),
                Span::styled("   ", Style::default().bg(palette.selection)),
                Span::raw(" "),
                Span::styled("   ", Style::default().bg(palette.border)),
            ]),
            Line::from(vec![
                Span::styled(
                    "kw",
                    Style::default().bg(palette.background).fg(palette.keyword),
                ),
                Span::raw(" "),
                Span::styled(
                    "\"str\"",
                    Style::default().bg(palette.background).fg(palette.string),
                ),
                Span::raw(" "),
                Span::styled(
                    "fn()",
                    Style::default().bg(palette.background).fg(palette.function),
                ),
                Span::raw(" "),
                Span::styled(
                    "# comment",
                    Style::default().bg(palette.background).fg(palette.comment),
                ),
            ]),
        ];

        Paragraph::new(lines)
            .style(base)
            .wrap(ratatui::widgets::Wrap { trim: true })
            .render(inner, buf);
    }

    pub(super) fn render_theme_option_lines_for_palette(
        &self,
        area: Rect,
        palette: &crate::theme::Theme,
        options: &[(ThemeName, Cow<'static, str>, &'static str)],
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            "Choose Theme",
            Style::default()
                .bg(palette.background)
                .fg(palette.text_bright)
                .add_modifier(Modifier::BOLD),
        )));

        let allow_custom = Self::allow_custom_theme_generation();
        let count = options.len() + if allow_custom { 1 } else { 0 };
        if count == 0 {
            return lines;
        }

        let visible = Self::visible_theme_rows(area.height);
        let (start, _, _) =
            crate::util::list_window::anchored_window(self.selected_theme_index, count, visible);
        let end = (start + visible).min(count);
        let hovered = self.hovered_theme_index;

        for i in start..end {
            let is_selected = i == self.selected_theme_index;
            let is_hovered = hovered == Some(i);
            if allow_custom && i >= options.len() {
                let mut spans = vec![Span::raw(" ")];
                if is_selected {
                    spans.push(Span::styled("› ", Style::default().fg(palette.keyword)));
                } else if is_hovered {
                    spans.push(Span::styled("• ", Style::default().fg(palette.info)));
                } else {
                    spans.push(Span::raw("  "));
                }
                let label_style = if is_selected || is_hovered {
                    Style::default()
                        .fg(palette.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(palette.text_dim)
                };
                spans.push(Span::styled("Generate your own…", label_style));
                lines.push(Line::from(spans));
                continue;
            }

            let (theme_enum, name, description) = &options[i];
            let is_original = *theme_enum == self.original_theme;
            let mut spans = vec![Span::raw(" ")];
            if is_selected {
                spans.push(Span::styled("› ", Style::default().fg(palette.keyword)));
            } else if is_hovered {
                spans.push(Span::styled("• ", Style::default().fg(palette.info)));
            } else {
                spans.push(Span::raw("  "));
            }

            if is_selected || is_hovered {
                spans.push(Span::styled(
                    name.clone(),
                    Style::default()
                        .fg(palette.primary)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled(name.clone(), Style::default().fg(palette.text)));
            }

            if is_original {
                spans.push(Span::styled(" (original)", Style::default().fg(palette.text_dim)));
                spans.push(Span::raw(" "));
            } else {
                spans.push(Span::raw("  "));
            }

            spans.push(Span::styled(
                *description,
                Style::default().fg(palette.text_dim),
            ));
            lines.push(Line::from(spans));
        }

        lines
    }

    pub(super) fn render_preview_theme_half(
        &self,
        area: Rect,
        preview_name: ThemeName,
        options: &[(ThemeName, Cow<'static, str>, &'static str)],
        buf: &mut Buffer,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        let palette = resolved_theme(preview_name);
        let title = format!("Preview: {}", Self::theme_display_name(preview_name));
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(palette.border));
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let base = Style::default().bg(palette.background).fg(palette.text);
        for y in inner.y..inner.y.saturating_add(inner.height) {
            for x in inner.x..inner.x.saturating_add(inner.width) {
                let cell = &mut buf[(x, y)];
                cell.set_char(' ');
                cell.set_style(base);
            }
        }

        let list_area = Self::theme_preview_list_area(area);
        let list_lines = self.render_theme_option_lines_for_palette(list_area, &palette, options);
        Paragraph::new(list_lines)
            .style(base)
            .render(list_area, buf);

        let samples_area = Rect {
            x: inner.x,
            y: list_area.y.saturating_add(list_area.height).min(inner.y.saturating_add(inner.height)),
            width: inner.width,
            height: inner
                .y
                .saturating_add(inner.height)
                .saturating_sub(list_area.y.saturating_add(list_area.height)),
        };
        if samples_area.width > 0 && samples_area.height > 0 {
            let lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "Live preview sample",
                    Style::default()
                        .bg(palette.background)
                        .fg(palette.text_bright)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(vec![
                    Span::styled("Primary", Style::default().bg(palette.background).fg(palette.primary)),
                    Span::raw(" "),
                    Span::styled("Info", Style::default().bg(palette.background).fg(palette.info)),
                    Span::raw(" "),
                    Span::styled("Warn", Style::default().bg(palette.background).fg(palette.warning)),
                    Span::raw(" "),
                    Span::styled("Err", Style::default().bg(palette.background).fg(palette.error)),
                ]),
                Line::from(vec![
                    Span::styled("   ", Style::default().bg(palette.primary)),
                    Span::raw(" "),
                    Span::styled("   ", Style::default().bg(palette.secondary)),
                    Span::raw(" "),
                    Span::styled("   ", Style::default().bg(palette.selection)),
                    Span::raw(" "),
                    Span::styled("   ", Style::default().bg(palette.border)),
                ]),
            ];
            Paragraph::new(lines)
                .style(base)
                .wrap(ratatui::widgets::Wrap { trim: true })
                .render(samples_area, buf);
        }
    }

    pub(super) fn render_theme_side_by_side(
        &self,
        area: Rect,
        options: &[(ThemeName, Cow<'static, str>, &'static str)],
        buf: &mut Buffer,
    ) {
        let (current_half, preview_half) = Self::theme_mode_areas(area);
        let current_name = self.revert_theme_on_back;
        let preview_name = self
            .hovered_theme_index
            .and_then(|idx| options.get(idx).map(|(name, _, _)| *name))
            .or_else(|| {
                options
                    .get(self.selected_theme_index)
                    .map(|(name, _, _)| *name)
            })
            .unwrap_or(self.current_theme);

        let current_palette = resolved_theme(current_name);
        let left_title = format!("Current: {}", Self::theme_display_name(current_name));
        Self::render_theme_preview_column(current_half, &left_title, &current_palette, buf);
        self.render_preview_theme_half(preview_half, preview_name, options, buf);
    }

    pub(super) fn active_preview_theme(&self) -> ThemeName {
        self.hovered_theme_index
            .and_then(Self::theme_name_for_option_index)
            .or_else(|| Self::theme_name_for_option_index(self.selected_theme_index))
            .unwrap_or(self.current_theme)
    }

    pub(super) fn send_theme_split_preview(&self) {
        if !matches!(self.mode, Mode::Themes) {
            return;
        }
        let preview = self.active_preview_theme();
        self.app_event_tx.send(AppEvent::SetThemeSplitPreview {
            current: self.revert_theme_on_back,
            preview,
        });
    }

    pub(super) fn clear_theme_split_preview(&self) {
        self.app_event_tx.send(AppEvent::ClearThemeSplitPreview);
    }

    pub(super) fn move_selection_up(&mut self) {
        if matches!(self.mode, Mode::Themes) {
            self.hovered_theme_index = None;
            if self.selected_theme_index > 0 {
                self.selected_theme_index -= 1;
                if let Some(theme) = Self::theme_name_for_option_index(self.selected_theme_index) {
                    self.current_theme = theme;
                }
            }
            self.send_theme_split_preview();
        } else {
            let names = crate::spinner::spinner_names();
            if self.selected_spinner_index > 0 {
                self.selected_spinner_index -= 1;
                if let Some(name) = names.get(self.selected_spinner_index) {
                    self.current_spinner = name.clone();
                    self.app_event_tx
                        .send(AppEvent::PreviewSpinner(self.current_spinner.clone()));
                }
            }
        }
    }

    pub(super) fn move_selection_down(&mut self) {
        if matches!(self.mode, Mode::Themes) {
            self.hovered_theme_index = None;
            let options_len = Self::theme_option_count();
            let allow_extra_row = Self::allow_custom_theme_generation();
            let limit = if allow_extra_row {
                options_len
            } else {
                options_len.saturating_sub(1)
            };
            if self.selected_theme_index < limit {
                self.selected_theme_index += 1;
                if self.selected_theme_index < options_len
                    && let Some(theme) = Self::theme_name_for_option_index(self.selected_theme_index)
                {
                    self.current_theme = theme;
                }
            }
            self.send_theme_split_preview();
        } else {
            let names = crate::spinner::spinner_names();
            // Allow moving onto the extra pseudo-row (Generate your own…)
            if self.selected_spinner_index < names.len() {
                self.selected_spinner_index += 1;
                if self.selected_spinner_index < names.len() {
                    if let Some(name) = names.get(self.selected_spinner_index) {
                        self.current_spinner = name.clone();
                        self.app_event_tx
                            .send(AppEvent::PreviewSpinner(self.current_spinner.clone()));
                    }
                } else {
                    // On the pseudo-row: do not change current spinner preview
                }
            }
        }
    }

    pub(super) fn confirm_theme(&mut self) {
        self.hovered_theme_index = None;
        self.app_event_tx
            .send(AppEvent::UpdateTheme(self.current_theme));
        self.clear_theme_split_preview();
        self.revert_theme_on_back = self.current_theme;
        self.mode = Mode::Overview;
    }

    pub(super) fn confirm_spinner(&mut self) {
        self.app_event_tx
            .send(AppEvent::UpdateSpinner(self.current_spinner.clone()));
        self.revert_spinner_on_back = self.current_spinner.clone();
        self.mode = Mode::Overview;
    }

    pub(super) fn cancel_detail(&mut self) {
        match self.mode {
            Mode::Themes => {
                self.hovered_theme_index = None;
                if self.current_theme != self.revert_theme_on_back {
                    self.current_theme = self.revert_theme_on_back;
                    self.app_event_tx
                        .send(AppEvent::PreviewTheme(self.current_theme));
                }
                self.clear_theme_split_preview();
            }
            Mode::Spinner => {
                if self.current_spinner != self.revert_spinner_on_back {
                    self.current_spinner = self.revert_spinner_on_back.clone();
                    self.app_event_tx
                        .send(AppEvent::PreviewSpinner(self.current_spinner.clone()));
                }
            }
            Mode::Overview => {}
            Mode::CreateSpinner(_) => {}
            Mode::CreateTheme(_) => {}
        }
        self.mode = Mode::Overview;
    }

    pub(super) fn send_tail(&self, message: impl Into<String>) {
        self.app_event_tx
            .send_background_event_with_ticket(&self.tail_ticket, message);
    }

    pub(super) fn send_before_next_output(&self, message: impl Into<String>) {
        self.app_event_tx.send_background_before_next_output_with_ticket(
            &self.before_ticket,
            message,
        );
    }

    /// Spawn a background task that creates a custom spinner using the LLM with a JSON schema
    pub(super) fn kickoff_spinner_creation(
        &self,
        user_prompt: String,
        progress_tx: std::sync::mpsc::Sender<ProgressMsg>,
    ) {
        let tx = self.app_event_tx.clone();
        let before_ticket = self.before_ticket.clone();
        let fallback_tx = self.app_event_tx.clone();
        let fallback_ticket = self.before_ticket.clone();
        let completion_tx = progress_tx.clone();
        if thread_spawner::spawn_lightweight("spinner-create", move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tx.send_background_before_next_output_with_ticket(
                        &before_ticket,
                        format!("Failed to start runtime: {e}"),
                    );
                    return;
                }
            };
            rt.block_on(async move {
                // Load current config (CLI-style) and construct a one-off ModelClient
                let cfg = match code_core::config::Config::load_with_cli_overrides(vec![], code_core::config::ConfigOverrides::default()) {
                    Ok(c) => c,
                    Err(e) => {
                        tx.send_background_before_next_output_with_ticket(
                            &before_ticket,
                            format!("Config error: {e}"),
                        );
                        return;
                    }
                };
                // Use the same auth preference as the active Codex session.
                // When logged in with ChatGPT, prefer ChatGPT auth; otherwise fall back to API key.
                let preferred_auth = if cfg.using_chatgpt_auth {
                    AuthMode::ChatGPT
                } else {
                    AuthMode::ApiKey
                };
                let auth_mgr = code_core::AuthManager::shared_with_mode_and_originator(
                    cfg.code_home.clone(),
                    preferred_auth,
                    cfg.responses_originator_header.clone(),
                );
                let debug_logger = match code_core::debug_logger::DebugLogger::new(true) {
                    Ok(logger) => logger,
                    Err(err) => {
                        tracing::warn!("spinner debug logger init failed: {err}");
                        match code_core::debug_logger::DebugLogger::new(false) {
                            Ok(logger) => logger,
                            Err(disabled_err) => {
                                tx.send_background_before_next_output_with_ticket(
                                    &before_ticket,
                                    format!(
                                        "Failed to initialize debug logger: {err}; fallback failed: {disabled_err}"
                                    ),
                                );
                                return;
                            }
                        }
                    }
                };
                let client = code_core::ModelClient::new(code_core::ModelClientInit {
                    config: std::sync::Arc::new(cfg.clone()),
                    auth_manager: Some(auth_mgr),
                    otel_event_manager: None,
                    provider: cfg.model_provider.clone(),
                    effort: code_core::config_types::ReasoningEffort::Low,
                    summary: cfg.model_reasoning_summary,
                    verbosity: cfg.model_text_verbosity,
                    session_id: uuid::Uuid::new_v4(),
                    // Enable debug logs for targeted triage of stream issues
                    debug_logger: std::sync::Arc::new(std::sync::Mutex::new(debug_logger)),
                });

                // Build developer guidance and input
                let developer = "You are performing a custom task to create a terminal spinner.\n\nRequirements:\n- Output JSON ONLY, no prose.\n- `interval` is the delay in milliseconds between frames; MUST be between 50 and 300 inclusive.\n- `frames` is an array of strings; each element is a frame displayed sequentially at the given interval.\n- The spinner SHOULD have between 2 and 60 frames.\n- Each frame SHOULD be between 1 and 30 characters wide. ALL frames MUST be the SAME width (same number of characters). If you propose frames with varying widths, PAD THEM ON THE LEFT with spaces so they are uniform.\n- You MAY use both ASCII and Unicode characters (e.g., box drawing, braille, arrows). Use EMOJIS ONLY if the user explicitly requests emojis in their prompt.\n- Be creative! You have the full range of Unicode to play with!\n".to_string();
                let input: Vec<code_protocol::models::ResponseItem> = vec![
                    code_protocol::models::ResponseItem::Message {
                        id: None,
                        role: "developer".to_string(),
                        content: vec![code_protocol::models::ContentItem::InputText {
                            text: developer,
                        }],
                        end_turn: None,
                        phase: None,
                    },
                    code_protocol::models::ResponseItem::Message {
                        id: None,
                        role: "user".to_string(),
                        content: vec![code_protocol::models::ContentItem::InputText {
                            text: user_prompt,
                        }],
                        end_turn: None,
                        phase: None,
                    },
                ];

                // JSON schema for structured output
                let schema = serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "minLength": 1, "maxLength": 40, "description": "Display name for the spinner (1 - 3 words, shown in the UI)."},
                        "interval": {"type": "integer", "minimum": 50, "maximum": 300, "description": "Delay between frames in milliseconds (50 - 300)."},
                        "frames": {
                            "type": "array",
                            "items": {"type": "string", "minLength": 1, "maxLength": 30},
                            "minItems": 2,
                            "maxItems": 60,
                            "description": "2 - 60 frames, 1 - 30 characters each (every frame should be the same length of characters)."
                        }
                    },
                    "required": ["name", "interval", "frames"],
                    "additionalProperties": false
                });
                let format = code_core::TextFormat { r#type: "json_schema".to_string(), name: Some("custom_spinner".to_string()), strict: Some(true), schema: Some(schema) };

                let mut prompt = code_core::Prompt::default();
                prompt.input = input;
                prompt.store = true;
                prompt.text_format = Some(format);
                prompt.set_log_tag("ui/theme_spinner");

                // Stream and collect final JSON
                use futures::StreamExt;
                let _ = progress_tx.send(ProgressMsg::ThinkingDelta("(connecting to model)".to_string()));
                let mut stream = match client.stream(&prompt).await {
                    Ok(s) => s,
                    Err(e) => {
                        tx.send_background_before_next_output_with_ticket(
                            &before_ticket,
                            format!("Request error: {e}"),
                        );
                        tracing::info!("spinner request error: {e}");
                        return;
                    }
                };
                let mut out = String::new();
                let mut think_sum = String::new();
                // Capture the last transport/stream error so we can surface it to the UI
                let mut last_err: Option<String> = None;
                while let Some(ev) = stream.next().await {
                    match ev {
                        Ok(code_core::ResponseEvent::Created { .. }) => { tracing::info!("LLM: created"); let _ = progress_tx.send(ProgressMsg::SetStatus("(starting generation)".to_string())); }
                        Ok(code_core::ResponseEvent::ReasoningSummaryDelta { delta, .. }) => { tracing::info!(target: "spinner", "LLM[thinking]: {}", delta); let _ = progress_tx.send(ProgressMsg::ThinkingDelta(delta.clone())); think_sum.push_str(&delta); }
                        Ok(code_core::ResponseEvent::ReasoningContentDelta { delta, .. }) => { tracing::info!(target: "spinner", "LLM[reasoning]: {}", delta); }
                        Ok(code_core::ResponseEvent::OutputTextDelta { delta, .. }) => { tracing::info!(target: "spinner", "LLM[delta]: {}", delta); let _ = progress_tx.send(ProgressMsg::OutputDelta(delta.clone())); out.push_str(&delta); }
                        Ok(code_core::ResponseEvent::OutputItemDone { item, .. }) => {
                            if let code_protocol::models::ResponseItem::Message { content, .. } = item {
                                for c in content { if let code_protocol::models::ContentItem::OutputText { text } = c { out.push_str(&text); } }
                            }
                            tracing::info!(target: "spinner", "LLM[item_done]");
                        }
                        Ok(code_core::ResponseEvent::Completed { .. }) => { tracing::info!("LLM: completed"); break; }
                        Err(e) => {
                            let msg = e.to_string();
                            tracing::info!("LLM stream error: {msg}");
                            last_err = Some(msg);
                            break; // Stop consuming after a terminal transport error
                        }
                        _ => {}
                    }
                }

                let _ = progress_tx.send(ProgressMsg::RawOutput(out.clone()));

                // If we received no content at all, surface the transport error explicitly
                if out.trim().is_empty() {
                    let err = last_err
                        .map(|e| format!(
                            "model stream error: {} | raw_out_len={} think_len={}",
                            e,
                            out.len(),
                            think_sum.len()
                        ))
                        .unwrap_or_else(|| format!(
                            "model stream returned no content | raw_out_len={} think_len={}",
                            out.len(),
                            think_sum.len()
                        ));
                    let _ = progress_tx.send(ProgressMsg::CompletedErr { error: err, _raw_snippet: String::new() });
                    return;
                }

                // Parse JSON; on failure, attempt to salvage a top-level object and log raw output
                let v: serde_json::Value = match serde_json::from_str(&out) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::info!(target: "spinner", "Strict JSON parse failed: {}", e);
                        tracing::info!(target: "spinner", "Raw output: {}", out);
                        fn extract_first_json_object(s: &str) -> Option<String> {
                            let mut depth = 0usize;
                            let mut in_str = false;
                            let mut esc = false;
                            let mut start: Option<usize> = None;
                            for (i, ch) in s.char_indices() {
                                if in_str {
                                    if esc { esc = false; }
                                    else if ch == '\\' { esc = true; }
                                    else if ch == '"' { in_str = false; }
                                    continue;
                                }
                                match ch {
                                    '"' => in_str = true,
                                    '{' => { if depth == 0 { start = Some(i); } depth += 1; },
                                    '}' => { if depth > 0 { depth -= 1; if depth == 0 { let end = i + ch.len_utf8(); return start.map(|st| s[st..end].to_string()); } } },
                                    _ => {}
                                }
                            }
                            None
                        }
                        if let Some(obj) = extract_first_json_object(&out) {
                            match serde_json::from_str::<serde_json::Value>(&obj) {
                                Ok(v) => v,
                                Err(e2) => {
                                    let _ = progress_tx.send(ProgressMsg::CompletedErr {
                                        error: e2.to_string(),
                                        _raw_snippet: out.chars().take(200).collect::<String>(),
                                    });
                                    return;
                                }
                            }
                        } else {
                            // Prefer a clearer message if we saw a transport error
                            let msg = last_err
                                .map(|le| format!("model stream error: {le}"))
                                .unwrap_or_else(|| e.to_string());
                            let _ = progress_tx.send(ProgressMsg::CompletedErr {
                                error: msg,
                                _raw_snippet: out.chars().take(200).collect::<String>(),
                            });
                            return;
                        }
                    }
                };
                let interval = v
                    .get("interval")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(120)
                    .clamp(50, 300);
                let display_name = v
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Custom")
                    .to_string();
                let mut frames: Vec<String> = v
                    .get("frames")
                    .and_then(serde_json::Value::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();

                // Enforce frame width limit (truncate to first 20 characters)
                const MAX_CHARS: usize = 20;
                frames = frames
                    .into_iter()
                    .map(|f| f.chars().take(MAX_CHARS).collect::<String>())
                    .filter(|f| !f.is_empty())
                    .collect();

                // Enforce count 2–50
                if frames.len() > 50 { frames.truncate(50); }
                if frames.len() < 2 { let _ = progress_tx.send(ProgressMsg::CompletedErr { error: "too few frames after validation".to_string(), _raw_snippet: out.chars().take(200).collect::<String>() }); return; }

                // Normalize: left-pad frames to equal length if needed
                let max_len = frames.iter().map(|f| f.chars().count()).max().unwrap_or(0);
                let norm_frames: Vec<String> = frames
                    .into_iter()
                    .map(|f| {
                        let cur = f.chars().count();
                        if cur >= max_len { f } else { format!("{}{}", " ".repeat(max_len - cur), f) }
                    })
                    .collect();

                // Persist + activate
                let _ = progress_tx.send(ProgressMsg::CompletedOk { name: display_name, interval, frames: norm_frames });
            });
        })
        .is_none()
        {
            let _ = completion_tx.send(ProgressMsg::CompletedErr {
                error: "background worker unavailable".to_string(),
                _raw_snippet: String::new(),
            });
            fallback_tx.send_background_before_next_output_with_ticket(
                &fallback_ticket,
                "Failed to generate spinner preview: background worker unavailable".to_string(),
            );
        }
    }

    /// Spawn a background task that creates a custom theme using the LLM.
    pub(super) fn kickoff_theme_creation(
        &self,
        user_prompt: String,
        progress_tx: std::sync::mpsc::Sender<ProgressMsg>,
    ) {
        let tx = self.app_event_tx.clone();
        // Capture a compact example of the current theme as guidance
        fn color_to_hex(c: ratatui::style::Color) -> Option<String> {
            match c {
                ratatui::style::Color::Rgb(r, g, b) => {
                    Some(format!("#{r:02X}{g:02X}{b:02X}"))
                }
                _ => None,
            }
        }
        let cur = crate::theme::current_theme();
        let mut example = serde_json::json!({"name": "Current", "colors": {}});
        if let Some(v) = color_to_hex(cur.primary) {
            example["colors"]["primary"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.secondary) {
            example["colors"]["secondary"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.background) {
            example["colors"]["background"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.foreground) {
            example["colors"]["foreground"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.border) {
            example["colors"]["border"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.border_focused) {
            example["colors"]["border_focused"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.selection) {
            example["colors"]["selection"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.cursor) {
            example["colors"]["cursor"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.success) {
            example["colors"]["success"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.warning) {
            example["colors"]["warning"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.error) {
            example["colors"]["error"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.info) {
            example["colors"]["info"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.text) {
            example["colors"]["text"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.text_dim) {
            example["colors"]["text_dim"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.text_bright) {
            example["colors"]["text_bright"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.keyword) {
            example["colors"]["keyword"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.string) {
            example["colors"]["string"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.comment) {
            example["colors"]["comment"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.function) {
            example["colors"]["function"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.spinner) {
            example["colors"]["spinner"] = serde_json::Value::String(v);
        }
        if let Some(v) = color_to_hex(cur.progress) {
            example["colors"]["progress"] = serde_json::Value::String(v);
        }

        let before_ticket = self.before_ticket.clone();
        let fallback_tx = self.app_event_tx.clone();
        let fallback_ticket = self.before_ticket.clone();
        let completion_tx = progress_tx.clone();
        if thread_spawner::spawn_lightweight("theme-create", move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tx.send_background_before_next_output_with_ticket(
                        &before_ticket,
                        format!("Failed to start runtime: {e}"),
                    );
                    return;
                }
            };
            rt.block_on(async move {
                let cfg = match code_core::config::Config::load_with_cli_overrides(vec![], code_core::config::ConfigOverrides::default()) {
                    Ok(c) => c,
                    Err(e) => {
                        tx.send_background_before_next_output_with_ticket(
                            &before_ticket,
                            format!("Config error: {e}"),
                        );
                        return;
                    }
                };
                let auth_mgr = code_core::AuthManager::shared_with_mode_and_originator(
                    cfg.code_home.clone(),
                    AuthMode::ApiKey,
                    cfg.responses_originator_header.clone(),
                );
                let debug_logger = match code_core::debug_logger::DebugLogger::new(false) {
                    Ok(logger) => logger,
                    Err(err) => {
                        tracing::warn!("theme debug logger init failed: {err}");
                        match code_core::debug_logger::DebugLogger::new(false) {
                            Ok(logger) => logger,
                            Err(disabled_err) => {
                                tx.send_background_before_next_output_with_ticket(
                                    &before_ticket,
                                    format!(
                                        "Failed to initialize debug logger: {err}; fallback failed: {disabled_err}"
                                    ),
                                );
                                return;
                            }
                        }
                    }
                };
                let client = code_core::ModelClient::new(code_core::ModelClientInit {
                    config: std::sync::Arc::new(cfg.clone()),
                    auth_manager: Some(auth_mgr),
                    otel_event_manager: None,
                    provider: cfg.model_provider.clone(),
                    effort: cfg.model_reasoning_effort,
                    summary: cfg.model_reasoning_summary,
                    verbosity: cfg.model_text_verbosity,
                    session_id: uuid::Uuid::new_v4(),
                    debug_logger: std::sync::Arc::new(std::sync::Mutex::new(debug_logger)),
                });

                // Prompt with example and detailed field usage to help the model choose appropriate colors
                let developer = format!(
                    "You are designing a TUI color theme for a terminal UI.\n\nOutput: Strict JSON only. Include fields: `name` (string), `is_dark` (boolean), and `colors` (object of hex strings #RRGGBB).\n\nImportant rules:\n- Include EVERY `colors` key below. If you are not changing a value, copy it from the Current example.\n- Ensure strong contrast and readability for text vs background and for dim/bright variants.\n- Favor accessible color contrast (WCAG-ish) where possible.\n\nColor semantics (how the UI uses them):\n- background: main screen background.\n- foreground: primary foreground accents for widgets.\n- text: normal body text; must be readable on background.\n- text_dim: secondary/description text; slightly lower contrast than text.\n- text_bright: headings/emphasis; higher contrast than text.\n- primary: primary action/highlight color for selected items/buttons.\n- secondary: secondary accents (less prominent than primary).\n- border: container borders/dividers; should be visible but subtle against background.\n- border_focused: border when focused/active; slightly stronger than border.\n- selection: background for selected list rows; must contrast with text.\n- cursor: text caret color in input fields; must contrast with background.\n- success/warning/error/info: status badges and notices.\n- keyword/string/comment/function: syntax highlight accents in code blocks.\n- spinner: glyph color for loading animations; should be visible on background.\n- progress: progress-bar foreground color.\n\nCurrent theme example (copy unchanged values from here):\n{example}",
                );
                let input: Vec<code_protocol::models::ResponseItem> = vec![
                    code_protocol::models::ResponseItem::Message {
                        id: None,
                        role: "developer".to_string(),
                        content: vec![code_protocol::models::ContentItem::InputText {
                            text: developer,
                        }],
                        end_turn: None,
                        phase: None,
                    },
                    code_protocol::models::ResponseItem::Message {
                        id: None,
                        role: "user".to_string(),
                        content: vec![code_protocol::models::ContentItem::InputText {
                            text: user_prompt,
                        }],
                        end_turn: None,
                        phase: None,
                    },
                ];

                let schema = serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string", "minLength": 1, "maxLength": 40},
                        "is_dark": {"type": "boolean"},
                        "colors": {
                            "type": "object",
                            "properties": {
                                "primary": {"type": "string"},
                                "secondary": {"type": "string"},
                                "background": {"type": "string"},
                                "foreground": {"type": "string"},
                                "border": {"type": "string"},
                                "border_focused": {"type": "string"},
                                "selection": {"type": "string"},
                                "cursor": {"type": "string"},
                                "success": {"type": "string"},
                                "warning": {"type": "string"},
                                "error": {"type": "string"},
                                "info": {"type": "string"},
                                "text": {"type": "string"},
                                "text_dim": {"type": "string"},
                                "text_bright": {"type": "string"},
                                "keyword": {"type": "string"},
                                "string": {"type": "string"},
                                "comment": {"type": "string"},
                                "function": {"type": "string"},
                                "spinner": {"type": "string"},
                                "progress": {"type": "string"}
                            },
                            "required": [
                                "primary", "secondary", "background", "foreground", "border",
                                "border_focused", "selection", "cursor", "success", "warning",
                                "error", "info", "text", "text_dim", "text_bright", "keyword",
                                "string", "comment", "function", "spinner", "progress"
                            ],
                            "additionalProperties": false
                        }
                    },
                    "required": ["name", "is_dark", "colors"],
                    "additionalProperties": false
                });
                let format = code_core::TextFormat { r#type: "json_schema".to_string(), name: Some("custom_theme".to_string()), strict: Some(true), schema: Some(schema) };

                let mut prompt = code_core::Prompt::default();
                prompt.input = input;
                prompt.store = true;
                prompt.text_format = Some(format);
                prompt.set_log_tag("ui/theme_builder");

                use futures::StreamExt;
                let _ = progress_tx.send(ProgressMsg::ThinkingDelta("(connecting to model)".to_string()));
                let mut stream = match client.stream(&prompt).await {
                    Ok(s) => s,
                    Err(e) => {
                        tx.send_background_before_next_output_with_ticket(
                            &before_ticket,
                            format!("Request error: {e}"),
                        );
                        return;
                    }
                };
                let mut out = String::new();
                // Capture the last transport/stream error so we can surface it to the UI
                let mut last_err: Option<String> = None;
                while let Some(ev) = stream.next().await {
                    match ev {
                        Ok(code_core::ResponseEvent::Created { .. }) => {
                            let _ = progress_tx.send(ProgressMsg::SetStatus("(starting generation)".to_string()));
                        }
                        Ok(code_core::ResponseEvent::ReasoningSummaryDelta { delta, .. }) => {
                            let _ = progress_tx.send(ProgressMsg::ThinkingDelta(delta));
                        }
                        Ok(code_core::ResponseEvent::ReasoningContentDelta { delta, .. }) => {
                            let _ = progress_tx.send(ProgressMsg::ThinkingDelta(delta));
                        }
                        Ok(code_core::ResponseEvent::OutputTextDelta { delta, .. }) => {
                            let _ = progress_tx.send(ProgressMsg::OutputDelta(delta.clone()));
                            out.push_str(&delta);
                        }
                        Ok(code_core::ResponseEvent::OutputItemDone {
                            item: code_protocol::models::ResponseItem::Message { content, .. },
                            ..
                        }) => {
                            for c in content {
                                if let code_protocol::models::ContentItem::OutputText {
                                    text,
                                } = c
                                {
                                    out.push_str(&text);
                                }
                            }
                        }
                        Ok(code_core::ResponseEvent::Completed { .. }) => break,
                        Err(e) => {
                            let msg = e.to_string();
                            let _ = progress_tx.send(ProgressMsg::ThinkingDelta(format!(
                                "(stream error: {msg})"
                            )));
                            last_err = Some(msg);
                            break; // Stop consuming after a terminal transport error
                        }
                        _ => {}
                    }
                }

                let _ = progress_tx.send(ProgressMsg::RawOutput(out.clone()));
                // If we received no content at all, surface the transport error explicitly
                if out.trim().is_empty() {
                    let err = last_err
                        .map(|e| format!("model stream error: {e}"))
                        .unwrap_or_else(|| "model stream returned no content".to_string());
                    let _ = progress_tx.send(ProgressMsg::CompletedErr {
                        error: err,
                        _raw_snippet: String::new(),
                    });
                    return;
                }
                // Try strict parse first; if that fails, salvage the first JSON object in the text.
                let v: serde_json::Value = match serde_json::from_str(&out) {
                    Ok(v) => v,
                    Err(e) => {
                        // Attempt to extract the first top-level JSON object from the stream text
                        fn extract_first_json_object(s: &str) -> Option<String> {
                            let mut depth = 0usize;
                            let mut in_str = false;
                            let mut esc = false;
                            let mut start: Option<usize> = None;
                            for (i, ch) in s.char_indices() {
                                if in_str {
                                    if esc { esc = false; }
                                    else if ch == '\\' { esc = true; }
                                    else if ch == '"' { in_str = false; }
                                    continue;
                                }
                                match ch {
                                    '"' => in_str = true,
                                    '{' => { if depth == 0 { start = Some(i); } depth += 1; },
                                    '}' => { if depth > 0 { depth -= 1; if depth == 0 { let end = i + ch.len_utf8(); return start.map(|st| s[st..end].to_string()); } } },
                                    _ => {}
                                }
                            }
                            None
                        }
                        if let Some(obj) = extract_first_json_object(&out) {
                            match serde_json::from_str::<serde_json::Value>(&obj) {
                                Ok(v) => v,
                                Err(e2) => {
                                    let _ = progress_tx.send(ProgressMsg::CompletedErr {
                                        error: e2.to_string(),
                                        _raw_snippet: out.chars().take(200).collect(),
                                    });
                                    return;
                                }
                            }
                        } else {
                            // Prefer a clearer message if we saw a transport error
                            let msg = last_err
                                .map(|le| format!("model stream error: {le}"))
                                .unwrap_or_else(|| e.to_string());
                            let _ = progress_tx.send(ProgressMsg::CompletedErr {
                                error: msg,
                                _raw_snippet: out.chars().take(200).collect(),
                            });
                            return;
                        }
                    }
                };
                let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("Custom").trim().to_string();
                let is_dark = v.get("is_dark").and_then(serde_json::Value::as_bool);
                let mut colors = code_core::config_types::ThemeColors::default();
                if let Some(map) = v.get("colors").and_then(|x| x.as_object()) {
                    let get = |k: &str| {
                        map.get(k)
                            .and_then(serde_json::Value::as_str)
                            .map(str::trim)
                            .map(str::to_string)
                    };
                    colors.primary = get("primary");
                    colors.secondary = get("secondary");
                    colors.background = get("background");
                    colors.foreground = get("foreground");
                    colors.border = get("border");
                    colors.border_focused = get("border_focused");
                    colors.selection = get("selection");
                    colors.cursor = get("cursor");
                    colors.success = get("success");
                    colors.warning = get("warning");
                    colors.error = get("error");
                    colors.info = get("info");
                    colors.text = get("text");
                    colors.text_dim = get("text_dim");
                    colors.text_bright = get("text_bright");
                    colors.keyword = get("keyword");
                    colors.string = get("string");
                    colors.comment = get("comment");
                    colors.function = get("function");
                    colors.spinner = get("spinner");
                    colors.progress = get("progress");
                }
                let _ = progress_tx.send(ProgressMsg::CompletedThemeOk(Box::new(
                    ThemeGenerationResult {
                        name,
                        colors,
                        is_dark,
                    },
                )));
            });
        })
        .is_none()
        {
            let _ = completion_tx.send(ProgressMsg::CompletedErr {
                error: "background worker unavailable".to_string(),
                _raw_snippet: String::new(),
            });
            fallback_tx.send_background_before_next_output_with_ticket(
                &fallback_ticket,
                "Failed to generate theme: background worker unavailable".to_string(),
            );
        }
    }
}
