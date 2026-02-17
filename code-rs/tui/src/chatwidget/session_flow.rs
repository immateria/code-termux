use super::*;

impl ChatWidget<'_> {
    pub(super) fn layout_areas(&self, area: Rect) -> Vec<Rect> {
        layout_scroll::layout_areas(self, area)
    }
    pub(super) fn finalize_active_stream(&mut self) {
        streaming::finalize_active_stream(self);
    }
    // Strict stream order key helpers
    pub(super) fn seed_stream_order_key(&mut self, kind: StreamKind, id: &str, key: OrderKey) {
        self.stream_order_seq.insert((kind, id.to_string()), key);
    }
    // Try to fetch a seeded stream order key. Callers must handle None.
    pub(super) fn try_stream_order_key(&self, kind: StreamKind, id: &str) -> Option<OrderKey> {
        self.stream_order_seq.get(&(kind, id.to_string())).copied()
    }
    pub(crate) fn new(args: ChatWidgetInit) -> Self {
        let ChatWidgetInit {
            mut config,
            app_event_tx,
            initial_prompt,
            initial_images,
            enhanced_keys_supported,
            terminal_info,
            show_order_overlay,
            latest_upgrade_version,
        } = args;
        let mapped_theme = crate::theme::map_theme_for_palette(
            config.tui.theme.name,
            config.tui.theme.is_dark,
        );
        config.tui.theme.name = mapped_theme;
        remember_cwd_history(&config.cwd);

        let (code_op_tx, code_op_rx) = unbounded_channel::<Op>();

        let auth_manager = AuthManager::shared_with_mode_and_originator(
            config.code_home.clone(),
            AuthMode::ApiKey,
            config.responses_originator_header.clone(),
            config.cli_auth_credentials_store_mode,
        );

        // Browser manager is now handled through the global state
        // The core session will use the same global manager when browser tools are invoked

        // Add initial animated welcome message to history (top of first request)
        let history_cells: Vec<Box<dyn HistoryCell>> = Vec::new();
        // Insert later via history_push_top_next_req once struct is constructed

        // Removed the legacy startup tip for /resume.

        // Initialize image protocol for rendering screenshots

        let auto_drive_variant = AutoDriveVariant::from_env();
        let test_mode = is_test_mode();

        let bottom_pane = BottomPane::new(BottomPaneParams {
            app_event_tx: app_event_tx.clone(),
            has_input_focus: true,
            enhanced_keys_supported,
            using_chatgpt_auth: config.using_chatgpt_auth,
            auto_drive_variant,
        });

        let mut new_widget = Self {
            app_event_tx,
            code_op_tx,
            bottom_pane,
            auth_manager: auth_manager.clone(),
            login_view_state: None,
            login_add_view_state: None,
            active_exec_cell: None,
            history_cells,
            config: config.clone(),
            mcp_tool_catalog_by_id: HashMap::new(),
            mcp_tools_by_server: HashMap::new(),
            mcp_disabled_tools_by_server: HashMap::new(),
            mcp_server_failures: HashMap::new(),
            startup_mcp_error_summary: None,
            remote_model_presets: None,
            allow_remote_default_at_startup: !config.model_explicit,
            chat_model_selected_explicitly: false,
            planning_restore: None,
            history_debug_events: if history_cell_logging_enabled() {
                Some(RefCell::new(Vec::new()))
            } else {
                None
            },
            latest_upgrade_version,
            reconnect_notice_active: false,
            initial_user_message: create_initial_user_message(
                initial_prompt.unwrap_or_default(),
                initial_images,
            ),
            total_token_usage: TokenUsage::default(),
            last_token_usage: TokenUsage::default(),
            rate_limit_snapshot: None,
            rate_limit_warnings: RateLimitWarningState::default(),
            rate_limit_fetch_inflight: false,
            rate_limit_last_fetch_at: None,
            rate_limit_primary_next_reset_at: None,
            rate_limit_secondary_next_reset_at: None,
            rate_limit_refresh_scheduled_for: None,
            rate_limit_refresh_schedule_id: Arc::new(AtomicU64::new(0)),
            content_buffer: String::new(),
            last_assistant_message: None,
            last_answer_stream_id_in_turn: None,
            last_answer_history_id_in_turn: None,
            last_seen_answer_stream_id_in_turn: None,
            mid_turn_answer_ids_in_turn: HashSet::new(),
            last_user_message: None,
            last_developer_message: None,
            pending_turn_origin: None,
            pending_request_user_input: None,
            current_turn_origin: None,
            cleared_lingering_execs_this_turn: true,
            exec: ExecState {
                running_commands: HashMap::new(),
                running_explore_agg_index: None,
                pending_exec_ends: HashMap::new(),
                suppressed_exec_end_call_ids: HashSet::new(),
                suppressed_exec_end_order: VecDeque::new(),
            },
            canceled_exec_call_ids: HashSet::new(),
            tools_state: ToolState::default(),
            // Use max width to disable wrapping during streaming
            // Text will be properly wrapped when displayed based on terminal width
            live_builder: RowBuilder::new(usize::MAX),
            header_wave: {
                let effect = HeaderWaveEffect::new();
                if ENABLE_WARP_STRIPES {
                    effect.set_enabled(true, Instant::now());
                } else {
                    effect.set_enabled(false, Instant::now());
                }
                effect
            },
            browser_overlay_visible: false,
            browser_overlay_state: BrowserOverlayState::default(),
            pending_images: HashMap::new(),
            welcome_shown: false,
            test_mode,
            latest_browser_screenshot: Arc::new(Mutex::new(None)),
            browser_autofix_requested: Arc::new(AtomicBool::new(false)),
            cached_image_protocol: RefCell::new(None),
            cached_picker: RefCell::new(terminal_info.picker.clone()),
            cached_cell_size: std::cell::OnceCell::new(),
            git_branch_cache: RefCell::new(GitBranchCache::default()),
            terminal_info,
            active_agents: Vec::new(),
            agents_ready_to_start: false,
            last_agent_prompt: None,
            agent_context: None,
            agent_task: None,
            recent_agent_hint: None,
            suppress_next_agent_hint: false,
            active_review_hint: None,
            active_review_prompt: None,
            auto_resolve_state: None,
            auto_resolve_attempts_baseline: config.auto_drive.auto_resolve_review_attempts.get(),
            turn_had_code_edits: false,
            background_review: None,
            auto_review_status: None,
            auto_review_notice: None,
            auto_review_baseline: None,
            auto_review_reviewed_marker: None,
            pending_auto_review_range: None,
            turn_sequence: 0,
            review_guard: None,
            background_review_guard: None,
            processed_auto_review_agents: HashSet::new(),
            pending_turn_descriptor: None,
            render_request_cache: RefCell::new(Vec::new()),
            render_request_cache_dirty: Cell::new(true),
            history_prefix_append_only: Cell::new(true),
            pending_auto_turn_config: None,
            overall_task_status: "preparing".to_string(),
            active_plan_title: None,
            agent_runtime: HashMap::new(),
            pending_agent_updates: HashMap::new(),
            sparkline_data: std::cell::RefCell::new(Vec::new()),
            last_sparkline_update: std::cell::RefCell::new(std::time::Instant::now()),
            stream: crate::streaming::controller::StreamController::new(config.clone()),
            stream_state: StreamState {
                current_kind: None,
                closed_answer_ids: HashSet::new(),
                closed_reasoning_ids: HashSet::new(),
                seq_answer_final: None,
                drop_streaming: false,
            },
            interrupts: interrupts::InterruptManager::new(),
            interrupt_flush_scheduled: false,
            ended_call_ids: HashSet::new(),
            diffs: DiffsState {
                session_patch_sets: Vec::new(),
                baseline_file_contents: HashMap::new(),
                overlay: None,
                confirm: None,
                body_visible_rows: std::cell::Cell::new(0),
            },
            help: HelpState {
                overlay: None,
                body_visible_rows: std::cell::Cell::new(0),
            },
            settings: SettingsState::default(),
            limits: LimitsState::default(),
            terminal: TerminalState::default(),
            pending_settings_return: None,
            pending_manual_terminal: HashMap::new(),
            agents_overview_selected_index: 0,
            agents_terminal: AgentsTerminalState::new(),
            pending_git_init_resume: None,
            git_init_inflight: false,
            git_init_declined: false,
            pending_upgrade_notice: None,
            history_render: HistoryRenderState::new(),
            last_render_settings: Cell::new(RenderSettings::new(0, 0, false)),
            render_theme_epoch: 0,
            history_state: HistoryState::new(),
            history_snapshot_dirty: false,
            history_snapshot_last_flush: None,
            context_cell_id: None,
            context_summary: None,
            context_last_sequence: None,
            context_browser_sequence: None,
            history_cell_ids: Vec::new(),
            history_live_window: None,
            history_frozen_width: 0,
            history_frozen_count: 0,
            height_manager: RefCell::new(HeightManager::new(
                crate::height_manager::HeightManagerConfig::default(),
            )),
            layout: LayoutState {
                scroll_offset: Cell::new(0),
                last_max_scroll: std::cell::Cell::new(0),
                last_history_viewport_height: std::cell::Cell::new(0),
                vertical_scrollbar_state: std::cell::RefCell::new(ScrollbarState::default()),
                scrollbar_visible_until: std::cell::Cell::new(None),
                last_bottom_reserved_rows: std::cell::Cell::new(0),
                last_frame_height: std::cell::Cell::new(0),
                last_frame_width: std::cell::Cell::new(0),
                last_bottom_pane_area: std::cell::Cell::new(Rect::default()),
            },
            last_theme: crate::theme::current_theme(),
            perf_state: PerfState {
                enabled: false,
                stats: RefCell::new(PerfStats::default()),
                pending_scroll_rows: Cell::new(0),
            },
            session_id: None,
            active_task_ids: HashSet::new(),
            queued_user_messages: std::collections::VecDeque::new(),
            pending_dispatched_user_messages: std::collections::VecDeque::new(),
            pending_user_prompts_for_next_turn: 0,
            queue_block_started_at: None,
            ghost_snapshots: Vec::new(),
            ghost_snapshots_disabled: false,
            ghost_snapshots_disabled_reason: None,
            ghost_snapshot_queue: VecDeque::new(),
            active_ghost_snapshot: None,
            next_ghost_snapshot_id: 0,
            history_virtualization_sync_pending: Cell::new(false),
            auto_drive_card_sequence: 0,
            auto_drive_variant,
            auto_state: AutoDriveController::default(),
            auto_goal_escape_state: AutoGoalEscState::Inactive,
            auto_handle: None,
            auto_drive_pid_guard: None,
            auto_history: AutoDriveHistory::new(),
            auto_compaction_overlay: None,
            auto_turn_review_state: None,
            auto_pending_goal_request: false,
            auto_goal_bootstrap_done: false,
            cloud_tasks_selected_env: None,
            cloud_tasks_environments: Vec::new(),
            cloud_tasks_last_tasks: Vec::new(),
            cloud_tasks_best_of_n: 1,
            cloud_tasks_creation_inflight: false,
            cloud_task_apply_tickets: HashMap::new(),
            cloud_task_create_ticket: None,
            browser_is_external: false,
            next_cli_text_format: None,
            // Stable ordering & routing init
            cell_order_seq: vec![OrderKey {
                req: 0,
                out: -1,
                seq: 0,
            }],
            cell_order_dbg: vec![None; 1],
            reasoning_index: HashMap::new(),
            stream_order_seq: HashMap::new(),
            order_request_bias: 0,
            resume_expected_next_request: None,
            resume_provider_baseline: None,
            last_seen_request_index: 0,
            current_request_index: 0,
            internal_seq: 0,
            show_order_overlay,
            scroll_history_hint_shown: false,
            access_status_idx: None,
            pending_agent_notes: Vec::new(),
            synthetic_system_req: None,
            system_cell_by_id: HashMap::new(),
            ui_background_seq_counters: HashMap::new(),
            last_assigned_order: None,
            standard_terminal_mode: !config.tui.alternate_screen,
            replay_history_depth: 0,
            resume_placeholder_visible: false,
            resume_picker_loading: false,
            clickable_regions: RefCell::new(Vec::new()),
            hovered_clickable_action: RefCell::new(None),
        };
        new_widget.load_auto_review_baseline_marker();
        new_widget.spawn_conversation_runtime(config.clone(), auth_manager, code_op_rx);
        if let Ok(Some(active_id)) = auth_accounts::get_active_account_id(&config.code_home)
            && let Ok(records) = account_usage::list_rate_limit_snapshots(&config.code_home)
                && let Some(record) = records.into_iter().find(|r| r.account_id == active_id) {
                    new_widget.rate_limit_primary_next_reset_at = record.primary_next_reset_at;
                    new_widget.rate_limit_secondary_next_reset_at = record.secondary_next_reset_at;
                    new_widget.maybe_schedule_rate_limit_refresh();
                }
        // Seed footer access indicator based on current config
        new_widget.apply_access_mode_indicator_from_config();
        // Insert the welcome cell as top-of-first-request so future model output
        // appears below it. Also insert the Popular commands immediately so users
        // don't wait for MCP initialization to finish.
        let mut w = new_widget;
        let auto_defaults = w.config.auto_drive.clone();
        w.auto_state.review_enabled = auto_defaults.review_enabled;
        w.auto_state.subagents_enabled = auto_defaults.agents_enabled;
        w.auto_state.cross_check_enabled = auto_defaults.cross_check_enabled;
        w.auto_state.qa_automation_enabled = auto_defaults.qa_automation_enabled;
        w.auto_state.continue_mode = auto_continue_from_config(auto_defaults.continue_mode);
        w.auto_state.reset_countdown();
        w.auto_goal_escape_state = AutoGoalEscState::Inactive;
        w.set_standard_terminal_mode(!config.tui.alternate_screen);
        let welcome_brand_title = w.config.tui.branding.title.as_deref();
        if config.experimental_resume.is_none() {
            w.history_push_top_next_req(history_cell::new_animated_welcome(
                welcome_brand_title,
            )); // tag: prelude
            if !w.config.auto_upgrade_enabled
                && let Some(upgrade_cell) =
                    history_cell::new_upgrade_prelude(w.latest_upgrade_version.as_deref())
                {
                    w.history_push_top_next_req(upgrade_cell);
                }
            let notice_state = history_cell::new_popular_commands_notice(
                false,
                w.latest_upgrade_version.as_deref(),
            );
            let notice_key = w.next_req_key_top();
            let _ = w.history_insert_plain_state_with_key(notice_state, notice_key, "prelude");
            // Mark welcome as shown to avoid duplicating the Popular commands section
            // when SessionConfigured arrives shortly after.
            w.welcome_shown = true;
        } else {
            w.welcome_shown = true;
            w.insert_resume_placeholder();
        }
        if w.test_mode {
            w.bottom_pane.set_task_running(false);
            w.bottom_pane.update_status_text(String::new());
            #[cfg(any(test, feature = "test-helpers"))]
            w.seed_test_mode_greeting();
        }
        w.maybe_start_auto_upgrade_task();
        w
    }

    /// Construct a ChatWidget from an existing conversation (forked session).
    pub(crate) fn new_from_existing(args: ForkedChatWidgetInit) -> Self {
        let ForkedChatWidgetInit {
            config,
            conversation,
            session_configured,
            app_event_tx,
            enhanced_keys_supported,
            terminal_info,
            show_order_overlay,
            latest_upgrade_version,
            auth_manager,
            show_welcome,
        } = args;
        remember_cwd_history(&config.cwd);
        let (code_op_tx, code_op_rx) = unbounded_channel::<Op>();

        let auto_drive_variant = AutoDriveVariant::from_env();

        // Forward events from existing conversation.
        agent::spawn_existing_conversation_runtime(
            conversation,
            session_configured,
            app_event_tx.clone(),
            code_op_rx,
        );

        // Basic widget state mirrors `new`
        let history_cells: Vec<Box<dyn HistoryCell>> = Vec::new();

        let bottom_pane = BottomPane::new(BottomPaneParams {
            app_event_tx: app_event_tx.clone(),
            has_input_focus: true,
            enhanced_keys_supported,
            using_chatgpt_auth: config.using_chatgpt_auth,
            auto_drive_variant,
        });

        let mut w = Self {
            app_event_tx,
            code_op_tx,
            bottom_pane,
            auth_manager,
            login_view_state: None,
            login_add_view_state: None,
            active_exec_cell: None,
            history_cells,
            config: config.clone(),
            mcp_tool_catalog_by_id: HashMap::new(),
            mcp_tools_by_server: HashMap::new(),
            mcp_disabled_tools_by_server: HashMap::new(),
            mcp_server_failures: HashMap::new(),
            startup_mcp_error_summary: None,
            remote_model_presets: None,
            allow_remote_default_at_startup: !config.model_explicit,
            chat_model_selected_explicitly: false,
            planning_restore: None,
            history_debug_events: if history_cell_logging_enabled() {
                Some(RefCell::new(Vec::new()))
            } else {
                None
            },
            latest_upgrade_version,
            reconnect_notice_active: false,
            initial_user_message: None,
            total_token_usage: TokenUsage::default(),
            last_token_usage: TokenUsage::default(),
            rate_limit_snapshot: None,
            rate_limit_warnings: RateLimitWarningState::default(),
            rate_limit_fetch_inflight: false,
            rate_limit_last_fetch_at: None,
            rate_limit_primary_next_reset_at: None,
            rate_limit_secondary_next_reset_at: None,
            rate_limit_refresh_scheduled_for: None,
            rate_limit_refresh_schedule_id: Arc::new(AtomicU64::new(0)),
            content_buffer: String::new(),
            last_assistant_message: None,
            last_answer_stream_id_in_turn: None,
            last_answer_history_id_in_turn: None,
            last_seen_answer_stream_id_in_turn: None,
            mid_turn_answer_ids_in_turn: HashSet::new(),
            last_user_message: None,
            last_developer_message: None,
            pending_turn_origin: None,
            pending_request_user_input: None,
            current_turn_origin: None,
            cleared_lingering_execs_this_turn: true,
            exec: ExecState {
                running_commands: HashMap::new(),
                running_explore_agg_index: None,
                pending_exec_ends: HashMap::new(),
                suppressed_exec_end_call_ids: HashSet::new(),
                suppressed_exec_end_order: VecDeque::new(),
            },
            canceled_exec_call_ids: HashSet::new(),
            tools_state: ToolState {
                running_custom_tools: HashMap::new(),
                web_search_sessions: HashMap::new(),
                web_search_by_call: HashMap::new(),
                web_search_by_order: HashMap::new(),
                running_wait_tools: HashMap::new(),
                running_kill_tools: HashMap::new(),
                image_viewed_calls: HashSet::new(),
                browser_sessions: HashMap::new(),
                browser_session_by_call: HashMap::new(),
                browser_session_by_order: HashMap::new(),
                browser_last_key: None,
                agent_runs: HashMap::new(),
                agent_run_by_call: HashMap::new(),
                agent_run_by_order: HashMap::new(),
                agent_run_by_batch: HashMap::new(),
                agent_run_by_agent: HashMap::new(),
                agent_last_key: None,
                auto_drive_tracker: None,
            },
            live_builder: RowBuilder::new(usize::MAX),
            header_wave: {
                let effect = HeaderWaveEffect::new();
                if ENABLE_WARP_STRIPES {
                    effect.set_enabled(true, Instant::now());
                } else {
                    effect.set_enabled(false, Instant::now());
                }
                effect
            },
            browser_overlay_visible: false,
            browser_overlay_state: BrowserOverlayState::default(),
            pending_images: HashMap::new(),
            welcome_shown: false,
            test_mode: is_test_mode(),
            latest_browser_screenshot: Arc::new(Mutex::new(None)),
            browser_autofix_requested: Arc::new(AtomicBool::new(false)),
            cached_image_protocol: RefCell::new(None),
            cached_picker: RefCell::new(terminal_info.picker.clone()),
            cached_cell_size: std::cell::OnceCell::new(),
            git_branch_cache: RefCell::new(GitBranchCache::default()),
            terminal_info,
            active_agents: Vec::new(),
            agents_ready_to_start: false,
            last_agent_prompt: None,
            agent_context: None,
            agent_task: None,
            recent_agent_hint: None,
            suppress_next_agent_hint: false,
            active_review_hint: None,
            active_review_prompt: None,
            auto_resolve_state: None,
            auto_resolve_attempts_baseline: config.auto_drive.auto_resolve_review_attempts.get(),
            turn_had_code_edits: false,
            background_review: None,
            auto_review_status: None,
            auto_review_notice: None,
            auto_review_baseline: None,
            auto_review_reviewed_marker: None,
            pending_auto_review_range: None,
            turn_sequence: 0,
            review_guard: None,
            background_review_guard: None,
            processed_auto_review_agents: HashSet::new(),
            pending_turn_descriptor: None,
            render_request_cache: RefCell::new(Vec::new()),
            render_request_cache_dirty: Cell::new(true),
            history_prefix_append_only: Cell::new(true),
            pending_auto_turn_config: None,
            overall_task_status: "preparing".to_string(),
            active_plan_title: None,
            agent_runtime: HashMap::new(),
            pending_agent_updates: HashMap::new(),
            sparkline_data: std::cell::RefCell::new(Vec::new()),
            last_sparkline_update: std::cell::RefCell::new(std::time::Instant::now()),
            stream: crate::streaming::controller::StreamController::new(config.clone()),
            stream_state: StreamState {
                current_kind: None,
                closed_answer_ids: HashSet::new(),
                closed_reasoning_ids: HashSet::new(),
                seq_answer_final: None,
                drop_streaming: false,
            },
            interrupts: interrupts::InterruptManager::new(),
            interrupt_flush_scheduled: false,
            ended_call_ids: HashSet::new(),
            diffs: DiffsState {
                session_patch_sets: Vec::new(),
                baseline_file_contents: HashMap::new(),
                overlay: None,
                confirm: None,
                body_visible_rows: std::cell::Cell::new(0),
            },
            help: HelpState {
                overlay: None,
                body_visible_rows: std::cell::Cell::new(0),
            },
            settings: SettingsState::default(),
            pending_settings_return: None,
            limits: LimitsState::default(),
            terminal: TerminalState::default(),
            pending_manual_terminal: HashMap::new(),
            agents_overview_selected_index: 0,
            agents_terminal: AgentsTerminalState::new(),
            pending_git_init_resume: None,
            git_init_inflight: false,
            git_init_declined: false,
            pending_upgrade_notice: None,
            history_render: HistoryRenderState::new(),
            last_render_settings: Cell::new(RenderSettings::new(0, 0, false)),
            render_theme_epoch: 0,
            history_state: HistoryState::new(),
            history_snapshot_dirty: false,
            history_snapshot_last_flush: None,
            context_cell_id: None,
            context_summary: None,
            context_last_sequence: None,
            context_browser_sequence: None,
            history_cell_ids: Vec::new(),
            history_live_window: None,
            history_frozen_width: 0,
            history_frozen_count: 0,
            height_manager: RefCell::new(HeightManager::new(
                crate::height_manager::HeightManagerConfig::default(),
            )),
            layout: LayoutState {
                scroll_offset: Cell::new(0),
                last_max_scroll: std::cell::Cell::new(0),
                last_history_viewport_height: std::cell::Cell::new(0),
                vertical_scrollbar_state: std::cell::RefCell::new(ScrollbarState::default()),
                scrollbar_visible_until: std::cell::Cell::new(None),
                last_bottom_reserved_rows: std::cell::Cell::new(0),
                last_frame_height: std::cell::Cell::new(0),
                last_frame_width: std::cell::Cell::new(0),
                last_bottom_pane_area: std::cell::Cell::new(Rect::default()),
            },
            last_theme: crate::theme::current_theme(),
            perf_state: PerfState {
                enabled: false,
                stats: RefCell::new(PerfStats::default()),
                pending_scroll_rows: Cell::new(0),
            },
            session_id: None,
            active_task_ids: HashSet::new(),
            queued_user_messages: std::collections::VecDeque::new(),
            pending_dispatched_user_messages: std::collections::VecDeque::new(),
            pending_user_prompts_for_next_turn: 0,
            queue_block_started_at: None,
            ghost_snapshots: Vec::new(),
            ghost_snapshots_disabled: false,
            ghost_snapshots_disabled_reason: None,
            ghost_snapshot_queue: VecDeque::new(),
            active_ghost_snapshot: None,
            next_ghost_snapshot_id: 0,
            history_virtualization_sync_pending: Cell::new(false),
            auto_drive_card_sequence: 0,
            auto_drive_variant,
            auto_state: AutoDriveController::default(),
            auto_goal_escape_state: AutoGoalEscState::Inactive,
            auto_handle: None,
            auto_drive_pid_guard: None,
            auto_history: AutoDriveHistory::new(),
            auto_compaction_overlay: None,
            auto_turn_review_state: None,
            auto_pending_goal_request: false,
            auto_goal_bootstrap_done: false,
            cloud_tasks_selected_env: None,
            cloud_tasks_environments: Vec::new(),
            cloud_tasks_last_tasks: Vec::new(),
            cloud_tasks_best_of_n: 1,
            cloud_tasks_creation_inflight: false,
            cloud_task_apply_tickets: HashMap::new(),
            cloud_task_create_ticket: None,
            browser_is_external: false,
            next_cli_text_format: None,
            // Strict ordering init for forked widget
            cell_order_seq: vec![OrderKey {
                req: 0,
                out: -1,
                seq: 0,
            }],
            cell_order_dbg: vec![None; 1],
            reasoning_index: HashMap::new(),
            stream_order_seq: HashMap::new(),
            order_request_bias: 0,
            resume_expected_next_request: None,
            resume_provider_baseline: None,
            last_seen_request_index: 0,
            current_request_index: 0,
            internal_seq: 0,
            show_order_overlay,
            scroll_history_hint_shown: false,
            access_status_idx: None,
            standard_terminal_mode: !config.tui.alternate_screen,
            pending_agent_notes: Vec::new(),
            synthetic_system_req: None,
            system_cell_by_id: HashMap::new(),
            ui_background_seq_counters: HashMap::new(),
            last_assigned_order: None,
            replay_history_depth: 0,
            resume_placeholder_visible: false,
            resume_picker_loading: false,
            clickable_regions: RefCell::new(Vec::new()),
            hovered_clickable_action: RefCell::new(None),
        };
        w.load_auto_review_baseline_marker();
        if let Ok(Some(active_id)) = auth_accounts::get_active_account_id(&config.code_home)
            && let Ok(records) = account_usage::list_rate_limit_snapshots(&config.code_home)
                && let Some(record) = records.into_iter().find(|r| r.account_id == active_id) {
                    w.rate_limit_primary_next_reset_at = record.primary_next_reset_at;
                    w.rate_limit_secondary_next_reset_at = record.secondary_next_reset_at;
                    w.maybe_schedule_rate_limit_refresh();
                }
        w.set_standard_terminal_mode(!config.tui.alternate_screen);
        let welcome_brand_title = w.config.tui.branding.title.as_deref();
        if show_welcome {
            w.history_push_top_next_req(history_cell::new_animated_welcome(
                welcome_brand_title,
            ));
        }
        if w.test_mode {
            w.bottom_pane.set_task_running(false);
            w.bottom_pane.update_status_text(String::new());
            #[cfg(any(test, feature = "test-helpers"))]
            w.seed_test_mode_greeting();
        }
        w.maybe_start_auto_upgrade_task();
        w
    }

    pub(super) fn auto_drive_lines_to_string(lines: Vec<Line<'static>>) -> String {
        let mut rows: Vec<String> = Vec::new();
        for line in lines {
            let mut row = String::new();
            for span in line.spans {
                row.push_str(span.content.as_ref());
            }
            rows.push(row);
        }
        while rows
            .last()
            .map(|line| line.trim().is_empty())
            .unwrap_or(false)
        {
            rows.pop();
        }
        rows.join("\n")
    }

    pub(super) fn auto_drive_role_for_kind(kind: HistoryCellType) -> Option<AutoDriveRole> {
        use AutoDriveRole::{Assistant, User};
        match kind {
            HistoryCellType::User => Some(Assistant),
            HistoryCellType::Assistant
            | HistoryCellType::Reasoning
            | HistoryCellType::Error
            | HistoryCellType::Exec { .. }
            | HistoryCellType::Patch { .. }
            | HistoryCellType::PlanUpdate
            | HistoryCellType::BackgroundEvent
            | HistoryCellType::Notice
            | HistoryCellType::CompactionSummary
            | HistoryCellType::Diff
            | HistoryCellType::Plain
            | HistoryCellType::Image => Some(User),
            HistoryCellType::Context => None,
            HistoryCellType::Tool { status } => match status {
                crate::history_cell::ToolCellStatus::Running => None,
                crate::history_cell::ToolCellStatus::Success
                | crate::history_cell::ToolCellStatus::Failed => Some(User),
            },
            HistoryCellType::AnimatedWelcome | HistoryCellType::Loading => None,
        }
    }

    pub(super) fn auto_drive_cell_text_for_index(&self, idx: usize, cell: &dyn HistoryCell) -> Option<String> {
        let lines = self.cell_lines_for_index(idx, cell);
        let text = Self::auto_drive_lines_to_string(lines);
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }

    pub(super) fn auto_drive_make_user_message(
        text: String,
    ) -> Option<code_protocol::models::ResponseItem> {
        if text.trim().is_empty() {
            return None;
        }
        use code_protocol::models::{ContentItem, ResponseItem};
        Some(ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText { text }],
            end_turn: None,
            phase: None,
        })
    }

    pub(super) fn auto_drive_browser_screenshot_items(
        cell: &BrowserSessionCell,
    ) -> Option<Vec<code_protocol::models::ContentItem>> {
        use code_protocol::models::ContentItem;

        let record = cell.screenshot_history().last()?;
        let bytes = match std::fs::read(&record.path) {
            Ok(bytes) if !bytes.is_empty() => bytes,
            Ok(_) => return None,
            Err(err) => {
                tracing::warn!(
                    "Failed to read browser screenshot for Auto Drive export: {} ({err})",
                    record.path.display()
                );
                return None;
            }
        };

        let mime = record
            .path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| {
                let ext_lower = ext.to_ascii_lowercase();
                match ext_lower.as_str() {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "gif" => "image/gif",
                    "bmp" => "image/bmp",
                    "webp" => "image/webp",
                    "svg" => "image/svg+xml",
                    "ico" => "image/x-icon",
                    "tif" | "tiff" => "image/tiff",
                    _ => "application/octet-stream",
                }
            })
            .unwrap_or("application/octet-stream")
            .to_string();
        let encoded = BASE64_STANDARD.encode(bytes);

        let timestamp_ms = record.timestamp.as_millis();
        let raw_url = record.url.as_deref().unwrap_or("browser");
        let sanitized = raw_url.replace(['\n', '\r'], " ");
        let trimmed = sanitized.trim();
        let mut url_meta = if trimmed.is_empty() {
            "browser".to_string()
        } else {
            trimmed.to_string()
        };
        if url_meta.len() > 240 {
            let mut truncated: String = url_meta.chars().take(240).collect();
            truncated.push_str("...");
            url_meta = truncated;
        }

        let metadata = format!("browser-screenshot:{timestamp_ms}:{url_meta}");

        let mut items = Vec::with_capacity(2);
        items.push(ContentItem::InputText {
            text: format!("[EPHEMERAL:{metadata}]"),
        });
        items.push(ContentItem::InputImage {
            image_url: format!("data:{mime};base64,{encoded}"),
        });

        Some(items)
    }

    pub(super) fn auto_drive_make_assistant_message(
        text: String,
    ) -> Option<code_protocol::models::ResponseItem> {
        if text.trim().is_empty() {
            return None;
        }
        use code_protocol::models::{ContentItem, ResponseItem};
        Some(ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText { text }],
            end_turn: None,
            phase: None,
        })
    }

    pub(super) fn reset_auto_compaction_overlay(&mut self) {
        self.auto_compaction_overlay = None;
    }

    pub(super) fn auto_drive_normalize_diff_path(raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        if trimmed == "/dev/null" {
            return None;
        }
        let normalized = trimmed
            .strip_prefix("a/")
            .or_else(|| trimmed.strip_prefix("b/"))
            .unwrap_or(trimmed);
        Some(normalized.to_string())
    }

    pub(super) fn auto_drive_diff_summary(record: &DiffRecord) -> Option<String> {
        use std::collections::BTreeMap;

        let mut stats: BTreeMap<String, (u32, u32)> = BTreeMap::new();
        let mut current_file: Option<String> = None;

        for hunk in &record.hunks {
            for line in &hunk.lines {
                match line.kind {
                    DiffLineKind::Context => {
                        let content = line.content.trim();
                        if let Some(rest) = content.strip_prefix("diff --git ") {
                            let mut parts = rest.split_whitespace();
                            let _old = parts.next();
                            if let Some(new_path) = parts.next()
                                && let Some(path) = Self::auto_drive_normalize_diff_path(new_path) {
                                    stats.entry(path.clone()).or_insert((0, 0));
                                    current_file = Some(path);
                                }
                        } else {
                            if let Some(rest) = content.strip_prefix("--- ")
                                && let Some(path) = Self::auto_drive_normalize_diff_path(rest) {
                                    stats.entry(path.clone()).or_insert((0, 0));
                                    current_file = Some(path);
                                }
                            if let Some(rest) = content.strip_prefix("+++ ")
                                && let Some(path) = Self::auto_drive_normalize_diff_path(rest) {
                                    stats.entry(path.clone()).or_insert((0, 0));
                                    current_file = Some(path);
                                }
                        }
                    }
                    DiffLineKind::Addition => {
                        if let Some(file) = current_file.as_ref() {
                            let entry = stats.entry(file.clone()).or_insert((0, 0));
                            entry.0 += 1;
                        }
                    }
                    DiffLineKind::Removal => {
                        if let Some(file) = current_file.as_ref() {
                            let entry = stats.entry(file.clone()).or_insert((0, 0));
                            entry.1 += 1;
                        }
                    }
                }
            }
        }

        if stats.is_empty() {
            return None;
        }

        let mut lines = Vec::with_capacity(stats.len() + 1);
        lines.push("Files changed".to_string());
        for (path, (added, removed)) in stats {
            lines.push(format!("- {path} (+{added} / -{removed})"));
        }
        Some(lines.join("\n"))
    }

    pub(crate) fn export_auto_drive_items(&self) -> Vec<code_protocol::models::ResponseItem> {
        let (items, _) = self.export_auto_drive_items_with_indices();
        items
    }

    pub(super) fn export_auto_drive_items_with_indices(
        &self,
    ) -> (
        Vec<code_protocol::models::ResponseItem>,
        Vec<Option<usize>>,
    ) {
        if let Some(overlay) = &self.auto_compaction_overlay {
            let mut items = overlay.prefix_items.clone();
            let mut indices = vec![None; overlay.prefix_items.len()];
            let tail = self.export_auto_drive_items_from_index_with_indices(overlay.tail_start_cell);
            for (cell_idx, item) in tail {
                indices.push(Some(cell_idx));
                items.push(item);
            }
            (items, indices)
        } else {
            let tail = self.export_auto_drive_items_from_index_with_indices(0);
            let mut items = Vec::with_capacity(tail.len());
            let mut indices = Vec::with_capacity(tail.len());
            for (cell_idx, item) in tail {
                indices.push(Some(cell_idx));
                items.push(item);
            }
            (items, indices)
        }
    }

    pub(super) fn export_auto_drive_items_from_index_with_indices(
        &self,
        start_idx: usize,
    ) -> Vec<(usize, code_protocol::models::ResponseItem)> {
        let mut items = Vec::new();
        for (idx, cell) in self.history_cells.iter().enumerate().skip(start_idx) {
            let Some(role) = Self::auto_drive_role_for_kind(cell.kind()) else {
                continue;
            };

            let text = match cell.kind() {
                HistoryCellType::Reasoning => self
                    .auto_drive_cell_text_for_index(idx, cell.as_ref())
                    .map(|text| (text, true)),
                HistoryCellType::PlanUpdate => {
                    if let Some(plan) = cell.as_any().downcast_ref::<PlanUpdateCell>() {
                        let state = plan.state();
                        let mut lines: Vec<String> = Vec::new();
                        if !state.name.trim().is_empty() {
                            lines.push(format!("Plan update: {}", state.name.trim()));
                        } else {
                            lines.push("Plan update".to_string());
                        }
                        if state.progress.total > 0 {
                            lines.push(format!(
                                "Progress: {}/{}",
                                state.progress.completed, state.progress.total
                            ));
                        }
                        if state.steps.is_empty() {
                            lines.push("(no steps recorded)".to_string());
                        } else {
                            for step in &state.steps {
                                let status_label = match step.status {
                                    StepStatus::Completed => "[completed]",
                                    StepStatus::InProgress => "[in_progress]",
                                    StepStatus::Pending => "[pending]",
                                };
                                lines.push(format!("{} {}", status_label, step.description));
                            }
                        }
                        let text = lines.join("\n");
                        Some((text, false))
                    } else {
                        self.auto_drive_cell_text_for_index(idx, cell.as_ref())
                            .map(|text| (text, false))
                    }
                }
                HistoryCellType::Diff => {
                    if let Some(diff_cell) = cell.as_any().downcast_ref::<DiffCell>() {
                        Self::auto_drive_diff_summary(diff_cell.record()).map(|text| (text, false))
                    } else {
                        self.auto_drive_cell_text_for_index(idx, cell.as_ref())
                            .map(|text| (text, false))
                    }
                }
                _ => self
                    .auto_drive_cell_text_for_index(idx, cell.as_ref())
                    .map(|text| (text, false)),
            };

            let Some((text, is_reasoning)) = text else {
                continue;
            };

            let mut extra_content = None;
            if !is_reasoning && matches!(role, AutoDriveRole::User)
                && let Some(browser_cell) = cell
                    .as_ref()
                    .as_any()
                    .downcast_ref::<BrowserSessionCell>()
                {
                    extra_content = Self::auto_drive_browser_screenshot_items(browser_cell);
                }

            let mut item = if is_reasoning {
                code_protocol::models::ResponseItem::Message {
                    id: Some("auto-drive-reasoning".to_string()),
                    role: "user".to_string(),
                    content: vec![code_protocol::models::ContentItem::InputText { text }],
                    end_turn: None,
                    phase: None,
                }
            } else {
                match role {
                    AutoDriveRole::Assistant => match Self::auto_drive_make_assistant_message(text) {
                        Some(item) => item,
                        None => continue,
                    },
                    AutoDriveRole::User => match Self::auto_drive_make_user_message(text) {
                        Some(item) => item,
                        None => continue,
                    },
                }
            };

            if let Some(extra) = extra_content
                && let code_protocol::models::ResponseItem::Message { content, .. } = &mut item {
                    content.extend(extra);
                }

            items.push((idx, item));
        }
        items
    }

    pub(super) fn derive_compaction_overlay(
        &self,
        previous_items: &[code_protocol::models::ResponseItem],
        previous_indices: &[Option<usize>],
        new_items: &[code_protocol::models::ResponseItem],
    ) -> Option<AutoCompactionOverlay> {
        if previous_items == new_items {
            return self.auto_compaction_overlay.clone();
        }

        if new_items.is_empty() {
            return Some(AutoCompactionOverlay {
                prefix_items: Vec::new(),
                tail_start_cell: self.history_cells.len(),
            });
        }

        let max_prefix = previous_items.len().min(new_items.len());
        let mut prefix_len = 0;
        while prefix_len < max_prefix && previous_items[prefix_len] == new_items[prefix_len] {
            prefix_len += 1;
        }

        let remaining_prev = previous_items.len().saturating_sub(prefix_len);
        let remaining_new = new_items.len().saturating_sub(prefix_len);
        let mut suffix_len = 0;
        while suffix_len < remaining_prev && suffix_len < remaining_new {
            let prev_idx = previous_items.len() - 1 - suffix_len;
            let new_idx = new_items.len() - 1 - suffix_len;
            if previous_items[prev_idx] != new_items[new_idx] {
                break;
            }
            suffix_len += 1;
        }

        let mut prefix_items_end = new_items.len().saturating_sub(suffix_len);

        let tail_start_cell = if suffix_len == 0 {
            self.history_cells.len()
        } else {
            let start = previous_items.len() - suffix_len;
            previous_indices[start..]
                .iter()
                .find_map(|idx| *idx)
                .unwrap_or(self.history_cells.len())
        };

        if suffix_len > 0 && tail_start_cell == self.history_cells.len() {
            // Suffix items no longer map to on-screen cells (e.g., after repeated compactions),
            // so treat the whole conversation as the overlay prefix.
            prefix_items_end = new_items.len();
        }

        let prefix_items = new_items[..prefix_items_end].to_vec();

        Some(AutoCompactionOverlay {
            prefix_items,
            tail_start_cell,
        })
    }

    pub(super) fn rebuild_auto_history(&mut self) -> Vec<code_protocol::models::ResponseItem> {
        let conversation = self.export_auto_drive_items();
        let tail = self
            .auto_history
            .replace_converted(conversation);
        if !tail.is_empty() {
            self.auto_history.append_converted_tail(&tail);
        }
        self.auto_history.raw_snapshot()
    }

    pub(super) fn current_auto_history(&mut self) -> Vec<code_protocol::models::ResponseItem> {
        if self.auto_history.converted_is_empty() {
            return self.rebuild_auto_history();
        }
        self.auto_history.raw_snapshot()
    }

    /// Export current user/assistant messages into ResponseItem list for forking.
    pub(crate) fn export_response_items(&self) -> Vec<code_protocol::models::ResponseItem> {
        use code_protocol::models::ContentItem;
        use code_protocol::models::ResponseItem;
        let mut items = Vec::new();
        for (idx, cell) in self.history_cells.iter().enumerate() {
            match cell.kind() {
                crate::history_cell::HistoryCellType::User => {
                    let text = self
                        .cell_lines_for_index(idx, cell.as_ref())
                        .iter()
                        .map(|l| {
                            l.spans
                                .iter()
                                .map(|s| s.content.to_string())
                                .collect::<String>()
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let prefixed = format!("Coordinator: {text}");
                    let content = ContentItem::InputText { text: prefixed };
                    items.push(ResponseItem::Message {
                        id: None,
                        role: "user".to_string(),
                        content: vec![content],
                        end_turn: None,
                        phase: None,
                    });
                }
                crate::history_cell::HistoryCellType::Assistant => {
                    let text = self
                        .cell_lines_for_index(idx, cell.as_ref())
                        .iter()
                        .map(|l| {
                            l.spans
                                .iter()
                                .map(|s| s.content.to_string())
                                .collect::<String>()
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    let prefixed = format!("CLI: {text}");
                    let content = ContentItem::OutputText { text: prefixed };
                    items.push(ResponseItem::Message {
                        id: None,
                        role: "assistant".to_string(),
                        content: vec![content],
                        end_turn: None,
                        phase: None,
                    });
                }
                crate::history_cell::HistoryCellType::PlanUpdate => {
                    if let Some(plan) = cell
                        .as_any()
                        .downcast_ref::<crate::history_cell::PlanUpdateCell>()
                    {
                        let state = plan.state();
                        let mut lines: Vec<String> = Vec::new();
                        if !state.name.trim().is_empty() {
                            lines.push(format!("Plan update: {}", state.name.trim()));
                        } else {
                            lines.push("Plan update".to_string());
                        }

                        if state.progress.total > 0 {
                            lines.push(format!(
                                "Progress: {}/{}",
                                state.progress.completed, state.progress.total
                            ));
                        }

                        if state.steps.is_empty() {
                            lines.push("(no steps recorded)".to_string());
                        } else {
                            for step in &state.steps {
                                let status_label = match step.status {
                                    StepStatus::Completed => "[completed]",
                                    StepStatus::InProgress => "[in_progress]",
                                    StepStatus::Pending => "[pending]",
                                };
                                lines.push(format!("- {} {}", status_label, step.description));
                            }
                        }

                        let text = lines.join("\n");
                        let content = ContentItem::OutputText { text };
                        items.push(ResponseItem::Message {
                            id: None,
                            role: "assistant".to_string(),
                            content: vec![content],
                            end_turn: None,
                            phase: None,
                        });
                    }
                }
                _ => {}
            }
        }
        items
    }

    pub(crate) fn config_ref(&self) -> &Config {
        &self.config
    }

    /// Check if there are any animations and trigger redraw if needed
    pub fn check_for_initial_animations(&mut self) {
        if self
            .history_cells
            .iter()
            .any(crate::history_cell::HistoryCell::is_animating)
        {
            if Self::auto_reduced_motion_preference() {
                return;
            }
            tracing::info!("Initial animation detected, scheduling frame");
            // Schedule initial frame for animations to ensure they start properly.
            // Use ScheduleFrameIn to avoid debounce issues with immediate RequestRedraw.
            self.app_event_tx
                .send(AppEvent::ScheduleFrameIn(HISTORY_ANIMATION_FRAME_INTERVAL));
        }
    }

    /// Format model name with proper capitalization (e.g., "gpt-4" -> "GPT-4")
    pub(super) fn format_model_name(&self, model_name: &str) -> String {
        fn format_segment(segment: &str) -> String {
            if segment.eq_ignore_ascii_case("codex") {
                return "Codex".to_string();
            }

            let mut chars = segment.chars();
            match chars.next() {
                Some(first) if first.is_ascii_alphabetic() => {
                    let mut formatted = String::new();
                    formatted.push(first.to_ascii_uppercase());
                    formatted.push_str(chars.as_str());
                    formatted
                }
                Some(first) => {
                    let mut formatted = String::new();
                    formatted.push(first);
                    formatted.push_str(chars.as_str());
                    formatted
                }
                None => String::new(),
            }
        }

        if let Some(rest) = model_name.strip_prefix("gpt-") {
            let formatted_rest = rest
                .split('-')
                .map(format_segment)
                .collect::<Vec<_>>()
                .join("-");
            format!("GPT-{formatted_rest}")
        } else {
            model_name.to_string()
        }
    }

    /// Calculate the maximum scroll offset based on current content size
    #[allow(dead_code)]
    pub(super) fn calculate_max_scroll_offset(&self, content_area_height: u16) -> u16 {
        let mut total_height = 0u16;

        // Calculate total content height (same logic as render method)
        for cell in &self.history_cells {
            let h = cell.desired_height(80); // Use reasonable width for height calculation
            total_height = total_height.saturating_add(h);
        }

        if let Some(ref cell) = self.active_exec_cell {
            let h = cell.desired_height(80);
            total_height = total_height.saturating_add(h);
        }

        // Max scroll is content height minus available height
        total_height.saturating_sub(content_area_height)
    }

    pub(super) fn try_append_prefix_fast(
        &self,
        render_requests: &[RenderRequest<'_>],
        render_settings: RenderSettings,
        prefix_width: u16,
    ) -> bool {
        if !self.history_prefix_append_only.get() {
            return false;
        }
        if !self
            .history_render
            .can_append_prefix(prefix_width, render_requests.len())
        {
            return false;
        }
        let prev_count = self.history_render.last_prefix_count();
        if prev_count == 0 || render_requests.len() != prev_count.saturating_add(1) {
            return false;
        }
        let history_count = self.history_cells.len();
        if history_count < 2 {
            return false;
        }
        if history_count != self
            .history_render
            .last_history_count()
            .saturating_add(1)
        {
            return false;
        }
        if render_requests.len() != history_count {
            return false;
        }
        let history_tail_start = history_count - 2;
        let tail = &render_requests[history_tail_start..history_count];
        if tail.len() != 2 {
            return false;
        }
        let cells = self
            .history_render
            .visible_cells(&self.history_state, tail, render_settings);
        if cells.len() != 2 {
            return false;
        }
        let prev = &cells[0];
        let next = &cells[1];
        if prev.height == 0 || next.height == 0 {
            return false;
        }
        let prev_is_reasoning = prev
            .cell
            .and_then(|cell| cell.as_any().downcast_ref::<crate::history_cell::CollapsibleReasoningCell>())
            .is_some();
        let next_is_reasoning = next
            .cell
            .and_then(|cell| cell.as_any().downcast_ref::<crate::history_cell::CollapsibleReasoningCell>())
            .is_some();
        if prev_is_reasoning || next_is_reasoning {
            return false;
        }
        let spacing = 1u16;
        let spacing_range = self
            .history_render
            .extend_prefix_for_append(prefix_width, spacing, next.height, history_count);
        if let Some(range) = spacing_range {
            self.history_render.append_spacing_range(range);
        }
        if next.height >= 2
            && next
                .cell
                .and_then(|cell| {
                    cell.as_any()
                        .downcast_ref::<crate::history_cell::AssistantMarkdownCell>()
                })
                .is_some()
        {
            let cell_end = self.history_render.last_total_height();
            let cell_start = cell_end.saturating_sub(next.height);
            self.history_render
                .append_spacing_range((cell_start, cell_start.saturating_add(1)));
            self.history_render
                .append_spacing_range((cell_end.saturating_sub(1), cell_end));
        }
        true
    }
}
