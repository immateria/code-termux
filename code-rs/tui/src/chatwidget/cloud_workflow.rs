use super::*;

impl ChatWidget<'_> {
    pub(crate) fn handle_cloud_command(&mut self, args: String) {
        let trimmed = args.trim();
        self.consume_pending_prompt_for_ui_only_turn();
        if trimmed.is_empty() {
            self.open_cloud_menu();
            return;
        }

        let mut parts = trimmed.splitn(2, ' ');
        let head = parts.next().unwrap_or("").to_ascii_lowercase();
        let rest = parts.next().map(str::trim).unwrap_or("");

        match head.as_str() {
            "list" => {
                if rest.is_empty() {
                    self.request_cloud_task_refresh(None);
                } else {
                    self.request_cloud_task_refresh(Some(rest.to_string()));
                }
            }
            "env" => {
                self.app_event_tx.send(AppEvent::FetchCloudEnvironments);
            }
            "new" => {
                if rest.is_empty() {
                    self.show_cloud_task_create_prompt();
                } else if let Some(env) = self.cloud_tasks_selected_env.clone() {
                    if self.cloud_tasks_creation_inflight {
                        self.bottom_pane.flash_footer_notice(
                            "Cloud task creation already in progress".to_string(),
                        );
                    } else {
                        self.cloud_tasks_creation_inflight = true;
                        self.cloud_task_create_ticket = Some(self.make_background_tail_ticket());
                        self.app_event_tx.send(AppEvent::SubmitCloudTaskCreate {
                            env_id: env.id,
                            prompt: rest.to_string(),
                            best_of_n: self.cloud_tasks_best_of_n,
                        });
                        self.show_cloud_task_create_progress();
                    }
                } else {
                    self.show_cloud_tasks_error(
                        "Select an environment before creating a cloud task".to_string(),
                    );
                    self.app_event_tx.send(AppEvent::FetchCloudEnvironments);
                }
            }
            _ => {
                self.history_push_plain_state(history_cell::new_error_event(format!(
                    "`/cloud` — unknown option '{head}'. Try `/cloud`, `/cloud list`, `/cloud new`, or `/cloud env`."
                )));
                self.request_redraw();
            }
        }
    }

    pub(super) fn open_cloud_menu(&mut self) {
        let current = self.cloud_env_label();
        let env_id = self
            .cloud_tasks_selected_env
            .as_ref()
            .map(|env| env.id.clone());
        let fetch_action_env = env_id;
        let mut items: Vec<SelectionItem> = Vec::new();
        items.push(SelectionItem {
            name: "Browse cloud tasks".to_string(),
            description: Some(format!("Current filter: {current}")),
            is_current: false,
            actions: vec![Box::new(move |tx: &AppEventSender| {
                tx.send(AppEvent::FetchCloudTasks {
                    environment: fetch_action_env.clone(),
                });
            })],
        });
        items.push(SelectionItem {
            name: "Select environment".to_string(),
            description: Some("Choose which environment to browse".to_string()),
            is_current: false,
            actions: vec![Box::new(|tx: &AppEventSender| {
                tx.send(AppEvent::FetchCloudEnvironments);
            })],
        });
        items.push(SelectionItem {
            name: "Create new task".to_string(),
            description: Some("Open the composer to submit a new cloud task".to_string()),
            is_current: false,
            actions: vec![Box::new(|tx: &AppEventSender| {
                tx.send(AppEvent::OpenCloudTaskCreate);
            })],
        });

        let view = ListSelectionView::new(
            " Cloud tasks ".to_string(),
            Some("Choose an action".to_string()),
            Some("Enter select · Esc cancel".to_string()),
            items,
            self.app_event_tx.clone(),
            6,
        );

        self.bottom_pane.show_list_selection(
            "Cloud tasks".to_string(),
            None,
            None,
            view,
        );
    }

    pub(crate) fn show_cloud_tasks_loading(&mut self) {
        let loading_item = SelectionItem {
            name: "Loading cloud tasks…".to_string(),
            description: Some("Fetching latest tasks from Codex Cloud".to_string()),
            is_current: true,
            actions: Vec::new(),
        };
        let view = ListSelectionView::new(
            " Cloud tasks ".to_string(),
            Some(self.cloud_env_label()),
            Some("Esc cancel".to_string()),
            vec![loading_item],
            self.app_event_tx.clone(),
            6,
        );
        self.bottom_pane.show_list_selection(
            "Loading cloud tasks".to_string(),
            None,
            None,
            view,
        );
    }

    pub(crate) fn present_cloud_tasks(
        &mut self,
        environment: Option<String>,
        tasks: Vec<TaskSummary>,
    ) {
        self.cloud_tasks_last_tasks = tasks.clone();
        let env_label = match environment {
            Some(ref id) => self
                .cloud_tasks_selected_env
                .as_ref()
                .filter(|env| env.id == *id)
                .map(|env| self.display_name_for_env(env))
                .unwrap_or_else(|| format!("Environment {id}")),
            None => "All environments".to_string(),
        };
        let view = CloudTasksView::new(
            tasks,
            Some(env_label),
            environment,
            self.app_event_tx.clone(),
        );
        self.bottom_pane.show_cloud_tasks(view);
        self.request_redraw();
    }

    pub(crate) fn show_cloud_tasks_error(&mut self, message: String) {
        self.bottom_pane.flash_footer_notice(message.clone());
        self.history_push_plain_state(history_cell::new_error_event(format!(
            "`/cloud` — {message}"
        )));
        self.request_redraw();
    }

    pub(crate) fn show_cloud_environment_loading(&mut self) {
        let loading_item = SelectionItem {
            name: "Loading environments…".to_string(),
            description: Some("Fetching available Codex Cloud environments".to_string()),
            is_current: true,
            actions: Vec::new(),
        };
        let view = ListSelectionView::new(
            " Select environment ".to_string(),
            Some("Choose which environment to browse".to_string()),
            Some("Esc cancel".to_string()),
            vec![loading_item],
            self.app_event_tx.clone(),
            8,
        );
        self.bottom_pane.show_list_selection(
            "Select environment".to_string(),
            None,
            None,
            view,
        );
    }

    pub(crate) fn present_cloud_environment_picker(
        &mut self,
        environments: Vec<CloudEnvironment>,
    ) {
        if environments.is_empty() {
            self.show_cloud_tasks_error("No environments available".to_string());
            return;
        }
        self.cloud_tasks_environments = environments.clone();

        let mut items: Vec<SelectionItem> = Vec::with_capacity(environments.len() + 1);
        items.push(SelectionItem {
            name: "All environments".to_string(),
            description: Some("Show tasks across every environment".to_string()),
            is_current: self.cloud_tasks_selected_env.is_none(),
            actions: vec![Box::new(|tx: &AppEventSender| {
                tx.send(AppEvent::SetCloudEnvironment { environment: None });
            })],
        });

        for env in environments {
            let env_clone = env.clone();
            let display = self.display_name_for_env(&env_clone);
            let repo_hint = env_clone
                .repo_hints
                .clone()
                .map(|hint| format!("Repo: {hint}"));
            items.push(SelectionItem {
                name: display,
                description: repo_hint,
                is_current: self
                    .cloud_tasks_selected_env
                    .as_ref()
                    .map(|selected| selected.id == env_clone.id)
                    .unwrap_or(false),
                actions: vec![Box::new(move |tx: &AppEventSender| {
                    tx.send(AppEvent::SetCloudEnvironment {
                        environment: Some(env_clone.clone()),
                    });
                })],
            });
        }

        let view = ListSelectionView::new(
            " Select environment ".to_string(),
            Some("Pick the environment to browse".to_string()),
            Some("Enter select · Esc cancel".to_string()),
            items,
            self.app_event_tx.clone(),
            10,
        );

        self.bottom_pane.show_list_selection(
            "Select environment".to_string(),
            None,
            None,
            view,
        );
    }

    pub(crate) fn set_cloud_environment(&mut self, environment: Option<CloudEnvironment>) {
        self.cloud_tasks_selected_env = environment.clone();
        let label = environment
            .as_ref()
            .map(|env| self.display_name_for_env(env))
            .unwrap_or_else(|| "All environments".to_string());
        self.bottom_pane
            .flash_footer_notice(format!("Cloud tasks filter set to {label}"));
        self.request_cloud_task_refresh(None);
    }

    pub(super) fn display_name_for_env(&self, env: &CloudEnvironment) -> String {
        match env.label.as_ref() {
            Some(label) if !label.is_empty() => format!("{label} ({})", env.id),
            _ => env.id.clone(),
        }
    }

    pub(super) fn request_cloud_task_refresh(&mut self, env_override: Option<String>) {
        let selected = env_override.or_else(|| self.current_cloud_env_id());
        self.app_event_tx
            .send(AppEvent::FetchCloudTasks { environment: selected });
    }

    pub(super) fn cloud_env_label(&self) -> String {
        self.cloud_tasks_selected_env
            .as_ref()
            .map(|env| self.display_name_for_env(env))
            .unwrap_or_else(|| "All environments".to_string())
    }

    pub(super) fn current_cloud_env_id(&self) -> Option<String> {
        self.cloud_tasks_selected_env
            .as_ref()
            .map(|env| env.id.clone())
    }

    pub(crate) fn show_cloud_task_actions(&mut self, task_id: String) {
        let Some(task) = self.find_cloud_task(&task_id) else {
            self.show_cloud_tasks_error(format!("Task {task_id} no longer available"));
            return;
        };

        let status_label = format!("Status: {:?}", task.status);
        let env_display = task
            .environment_label
            .as_ref()
            .or(task.environment_id.as_ref())
            .cloned()
            .unwrap_or_else(|| "Unknown environment".to_string());

        let mut items: Vec<SelectionItem> = Vec::new();
        let diff_id = task.id.0.clone();
        items.push(SelectionItem {
            name: "View diff".to_string(),
            description: Some("Open the unified diff in history".to_string()),
            is_current: true,
            actions: vec![Box::new(move |tx: &AppEventSender| {
                tx.send(AppEvent::FetchCloudTaskDiff {
                    task_id: diff_id.clone(),
                });
            })],
        });

        let msg_id = task.id.0.clone();
        items.push(SelectionItem {
            name: "View assistant output".to_string(),
            description: Some("Show assistant messages associated with this task".to_string()),
            is_current: false,
            actions: vec![Box::new(move |tx: &AppEventSender| {
                tx.send(AppEvent::FetchCloudTaskMessages {
                    task_id: msg_id.clone(),
                });
            })],
        });

        let preflight_id = task.id.0.clone();
        items.push(SelectionItem {
            name: "Preflight apply".to_string(),
            description: Some("Check whether the patch applies cleanly".to_string()),
            is_current: false,
            actions: vec![Box::new(move |tx: &AppEventSender| {
                tx.send(AppEvent::ApplyCloudTask {
                    task_id: preflight_id.clone(),
                    preflight: true,
                });
            })],
        });

        let apply_id = task.id.0.clone();
        items.push(SelectionItem {
            name: "Apply task".to_string(),
            description: Some("Apply the diff to the working tree".to_string()),
            is_current: false,
            actions: vec![Box::new(move |tx: &AppEventSender| {
                tx.send(AppEvent::ApplyCloudTask {
                    task_id: apply_id.clone(),
                    preflight: false,
                });
            })],
        });

        let subtitle = format!("{status_label} • Env: {env_display}");
        let view = ListSelectionView::new(
            format!(" Task {} ", task.title),
            Some(subtitle),
            Some("Enter choose · Esc cancel".to_string()),
            items,
            self.app_event_tx.clone(),
            8,
        );

        self.bottom_pane.show_list_selection(
            format!("Cloud task: {}", task.title),
            None,
            None,
            view,
        );
    }

    pub(crate) fn show_cloud_task_create_prompt(&mut self) {
        let Some(env) = self.cloud_tasks_selected_env.clone() else {
            self.show_cloud_tasks_error(
                "Select an environment before creating a cloud task".to_string(),
            );
            return;
        };

        let env_display = self.display_name_for_env(&env);
        let submit_tx = self.app_event_tx.clone();
        let env_id = env.id;
        let best_of = self.cloud_tasks_best_of_n;
        let on_submit: Box<dyn Fn(String) + Send + Sync> = Box::new(move |text: String| {
            if text.trim().is_empty() {
                return;
            }
            submit_tx.send(AppEvent::SubmitCloudTaskCreate {
                env_id: env_id.clone(),
                prompt: text,
                best_of_n: best_of,
            });
        });

        let view = CustomPromptView::new(
            format!("Create cloud task ({env_display})"),
            "Describe the change you want Codex to implement".to_string(),
            Some("Press Enter to submit · Esc cancel".to_string()),
            self.app_event_tx.clone(),
            None,
            on_submit,
        );
        self.bottom_pane.show_custom_prompt(view);
    }

    pub(crate) fn show_cloud_task_create_progress(&mut self) {
        self.cloud_tasks_creation_inflight = true;
        self.bottom_pane
            .flash_footer_notice("Submitting cloud task…".to_string());
        self.request_redraw();
    }

    pub(crate) fn handle_cloud_task_created(
        &mut self,
        env_id: String,
        result: Result<CreatedTask, CloudTaskError>,
    ) {
        self.cloud_tasks_creation_inflight = false;
        match result {
            Ok(created) => {
                let ticket = self
                    .cloud_task_create_ticket
                    .take()
                    .unwrap_or_else(|| self.make_background_tail_ticket());
                self.app_event_tx.send_background_event_with_ticket(
                    &ticket,
                    format!("✅ Created cloud task {} in {env_id}", created.id.0),
                );
                self.request_cloud_task_refresh(None);
            }
            Err(err) => {
                self.show_cloud_tasks_error(format!(
                    "Failed to create cloud task in {env_id}: {}",
                    describe_cloud_error(&err)
                ));
            }
        }
    }

    pub(crate) fn show_cloud_task_apply_status(&mut self, task_id: &str, preflight: bool) {
        let key = (task_id.to_string(), preflight);
        if !self.cloud_task_apply_tickets.contains_key(&key) {
            let ticket = self.make_background_tail_ticket();
            self.cloud_task_apply_tickets.insert(key.clone(), ticket);
        }
        let Some(ticket) = self.cloud_task_apply_tickets.get_mut(&key) else {
            return;
        };
        if preflight {
            self.app_event_tx.send_background_event_with_ticket(
                ticket,
                format!("Preflighting cloud task {task_id}…"),
            );
        } else {
            self.app_event_tx.send_background_event_with_ticket(
                ticket,
                format!("Applying cloud task {task_id}…"),
            );
        }
    }

    pub(crate) fn handle_cloud_task_apply_finished(
        &mut self,
        task_id: String,
        outcome: Result<ApplyOutcome, CloudTaskError>,
        preflight: bool,
    ) {
        match outcome {
            Ok(result) => {
                let mut message = if preflight {
                    format!("Preflight result for {task_id}: {}", result.message)
                } else {
                    format!("Apply result for {task_id}: {}", result.message)
                };
                if !result.skipped_paths.is_empty() {
                    message.push_str("\nSkipped: ");
                    message.push_str(&result.skipped_paths.join(", "));
                }
                if !result.conflict_paths.is_empty() {
                    message.push_str("\nConflicts: ");
                    message.push_str(&result.conflict_paths.join(", "));
                }
                let key = (task_id, preflight);
                let ticket = self
                    .cloud_task_apply_tickets
                    .remove(&key)
                    .unwrap_or_else(|| self.make_background_tail_ticket());
                self.app_event_tx
                    .send_background_event_with_ticket(&ticket, message);
                if !preflight {
                    self.request_cloud_task_refresh(None);
                }
            }
            Err(err) => {
                self.show_cloud_tasks_error(format!(
                    "Cloud task {task_id} failed: {}",
                    describe_cloud_error(&err)
                ));
            }
        }
    }

    pub(super) fn find_cloud_task(&self, task_id: &str) -> Option<&TaskSummary> {
        self.cloud_tasks_last_tasks
            .iter()
            .find(|task| task.id.0 == task_id)
    }

    pub(super) fn ensure_git_repo_for_action(&mut self, resume: GitInitResume, reason: &str) -> bool {
        if code_core::git_info::get_git_repo_root(&self.config.cwd).is_some() {
            if self.git_init_declined {
                self.git_init_declined = false;
            }
            return false;
        }

        if self.git_init_inflight {
            self.bottom_pane
                .flash_footer_notice("Initializing git repository...".to_string());
            return true;
        }

        if self.git_init_declined {
            let notice = format!(
                "{reason} Run `git init` in {} to enable write-enabled agents and worktrees.",
                self.config.cwd.display()
            );
            self.history_push_plain_paragraphs(
                PlainMessageKind::Notice,
                vec!["Git repository not initialized.".to_string(), notice],
            );
            self.request_redraw();
            return true;
        }

        self.show_git_init_prompt(resume, reason);
        true
    }

    pub(super) fn show_git_init_prompt(&mut self, resume: GitInitResume, reason: &str) {
        let subtitle = format!(
            "{reason}\nInitialize a git repository in {}?",
            self.config.cwd.display()
        );
        let resume_init = resume;
        let items = vec![
            SelectionItem {
                name: "Initialize git repository".to_string(),
                description: Some("Run `git init` in this folder (recommended).".to_string()),
                is_current: true,
                actions: vec![Box::new(move |tx: &AppEventSender| {
                    tx.send(AppEvent::ConfirmGitInit {
                        resume: resume_init.clone(),
                    });
                })],
            },
            SelectionItem {
                name: "Continue without git".to_string(),
                description: Some("Write-enabled agents and worktrees will be unavailable.".to_string()),
                is_current: false,
                actions: vec![Box::new(|tx: &AppEventSender| {
                    tx.send(AppEvent::DeclineGitInit);
                })],
            },
        ];

        let view = ListSelectionView::new(
            " Git repository required ".to_string(),
            Some(subtitle),
            Some("Enter select - Esc cancel".to_string()),
            items,
            self.app_event_tx.clone(),
            6,
        );
        self.bottom_pane.show_list_selection(
            "Git repository required".to_string(),
            None,
            None,
            view,
        );
        self.request_redraw();
    }

    pub(crate) fn confirm_git_init(&mut self, resume: GitInitResume) {
        if self.git_init_inflight {
            self.bottom_pane
                .flash_footer_notice("Git init already running...".to_string());
            return;
        }

        self.pending_git_init_resume = Some(resume);
        self.git_init_inflight = true;

        let cwd = self.config.cwd.clone();
        let ticket = self.make_background_tail_ticket();
        self.app_event_tx.send_background_event_with_ticket(
            &ticket,
            format!("Initializing git repository in {}...", cwd.display()),
        );

        let tx = self.app_event_tx.clone();
        tokio::spawn(async move {
            let output = tokio::process::Command::new("git")
                .current_dir(&cwd)
                .arg("init")
                .output()
                .await;

            let (ok, message) = match output {
                Ok(out) if out.status.success() => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let trimmed = stdout.trim();
                    let msg = if trimmed.is_empty() {
                        format!("Initialized git repository in {}.", cwd.display())
                    } else {
                        trimmed.to_string()
                    };
                    (true, msg)
                }
                Ok(out) => {
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let detail = if !stderr.trim().is_empty() {
                        stderr.trim().to_string()
                    } else {
                        stdout.trim().to_string()
                    };
                    let msg = if detail.is_empty() {
                        "git init failed.".to_string()
                    } else {
                        format!("git init failed: {detail}")
                    };
                    (false, msg)
                }
                Err(e) => (false, format!("Failed to run git init: {e}")),
            };

            tx.send(AppEvent::GitInitFinished { ok, message });
        });
    }

    pub(crate) fn decline_git_init(&mut self) {
        self.git_init_declined = true;
        let notice = format!(
            "Write-enabled agents and worktrees are unavailable until you run `git init` in {}.",
            self.config.cwd.display()
        );
        self.history_push_plain_paragraphs(
            PlainMessageKind::Notice,
            vec!["Git repository not initialized.".to_string(), notice],
        );
        self.request_redraw();
    }

    pub(crate) fn handle_git_init_finished(&mut self, ok: bool, message: String) {
        self.git_init_inflight = false;
        if ok {
            self.git_init_declined = false;
            let notice = if message.trim().is_empty() {
                format!("Initialized git repository in {}.", self.config.cwd.display())
            } else {
                message
            };
            self.push_background_tail(notice);
            if let Some(resume) = self.pending_git_init_resume.take() {
                match resume {
                    GitInitResume::SubmitText { text } => {
                        self.app_event_tx.send(AppEvent::SubmitTextWithPreface {
                            visible: text,
                            preface: String::new(),
                        });
                    }
                    GitInitResume::DispatchCommand {
                        command,
                        command_text,
                    } => {
                        self.app_event_tx
                            .send(AppEvent::DispatchCommand(command, command_text));
                    }
                }
            }
        } else {
            let err = if message.trim().is_empty() {
                "git init failed.".to_string()
            } else {
                message
            };
            self.history_push_plain_state(history_cell::new_error_event(err));
            self.pending_git_init_resume = None;
        }
        self.request_redraw();
    }

}
