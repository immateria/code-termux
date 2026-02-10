use super::*;

impl ChatWidget<'_> {
    pub(crate) fn handle_branch_command(&mut self, args: String) {
        self.consume_pending_prompt_for_ui_only_turn();
        let command_text = if args.trim().is_empty() {
            "/branch".to_string()
        } else {
            format!("/branch {}", args.trim())
        };
        if self.ensure_git_repo_for_action(
            GitInitResume::DispatchCommand {
                command: SlashCommand::Branch,
                command_text,
            },
            "Creating a branch worktree requires a git repository.",
        ) {
            return;
        }
        if Self::is_branch_worktree_path(&self.config.cwd) {
            self.history_push_plain_state(crate::history_cell::new_error_event(
                "`/branch` — already inside a branch worktree; switch to the repo root before creating another branch."
                    .to_string(),
            ));
            self.request_redraw();
            return;
        }
        let args_trim = args.trim().to_string();
        let cwd = self.config.cwd.clone();
        let tx = self.app_event_tx.clone();
        let branch_tail_ticket = self.make_background_tail_ticket();
        // Add a quick notice into history, include task preview if provided
        if args_trim.is_empty() {
            self.insert_background_event_with_placement(
                "Creating branch worktree...".to_string(),
                BackgroundPlacement::BeforeNextOutput,
                None,
            );
        } else {
            self.insert_background_event_with_placement(
                format!("Creating branch worktree... Task: {args_trim}"),
                BackgroundPlacement::BeforeNextOutput,
                None,
            );
        }
        self.request_redraw();

        tokio::spawn(async move {
            use tokio::process::Command;
            let ticket = branch_tail_ticket;
            // Resolve git root
            let git_root = match code_core::git_worktree::get_git_root_from(&cwd).await {
                Ok(p) => p,
                Err(e) => {
                    tx.send_background_event_with_ticket(
                        &ticket,
                        format!("`/branch` — not a git repo: {e}"),
                    );
                    return;
                }
            };
            let current_base_branch = Command::new("git")
                .current_dir(&git_root)
                .args(["branch", "--show-current"])
                .output()
                .await
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| {
                    let name = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if name.is_empty() { None } else { Some(name) }
                });
            // Determine branch name
            let task_opt = if args.trim().is_empty() {
                None
            } else {
                Some(args.trim())
            };
            let branch_name = code_core::git_worktree::generate_branch_name_from_task(task_opt);
            // Create worktree
            let (worktree, used_branch) =
                match code_core::git_worktree::setup_worktree(&git_root, &branch_name, None).await {
                    Ok((p, b)) => (p, b),
                    Err(e) => {
                        tx.send_background_event_with_ticket(
                            &ticket,
                            format!("`/branch` — failed to create worktree: {e}"),
                        );
                        return;
                    }
                };
            remember_worktree_root_hint(&worktree, &git_root);
            // Copy uncommitted changes from the source root into the new worktree
            let copied =
                match code_core::git_worktree::copy_uncommitted_to_worktree(&git_root, &worktree)
                    .await
                {
                    Ok(n) => n,
                    Err(e) => {
                        tx.send_background_event_with_ticket(
                            &ticket,
                            format!("`/branch` — failed to copy changes: {e}"),
                        );
                        // Still switch to the branch even if copy fails
                        0
                    }
                };

            let mut branch_metadata: Option<code_core::git_worktree::BranchMetadata> = None;
            match code_core::git_worktree::ensure_local_default_remote(
                &git_root,
                current_base_branch.as_deref(),
            )
            .await
            {
                Ok(meta_option) => {
                    if let Some(meta) = meta_option.clone() {
                        if let Err(e) = code_core::git_worktree::write_branch_metadata(&worktree, &meta).await
                        {
                            tx.send_background_event_with_ticket(
                                &ticket,
                                format!("`/branch` — failed to record branch metadata: {e}"),
                            );
                        }
                        branch_metadata = meta_option;
                    }
                }
                Err(err) => {
                    tx.send_background_event_with_ticket(
                        &ticket,
                        format!(
                            "`/branch` — failed to configure local-default remote: {err}"
                        ),
                    );
                }
            }

            // Attempt to set upstream for the new branch to match the source branch's upstream,
            // falling back to origin/<default> when available. Also ensure origin/HEAD is set.
            let mut _upstream_msg: Option<String> = None;
            // Discover source branch upstream like 'origin/main'
            let src_upstream = Command::new("git")
                .current_dir(&git_root)
                .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
                .output()
                .await
                .ok()
                .filter(|o| o.status.success())
                .and_then(|o| {
                    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if s.is_empty() { None } else { Some(s) }
                });
            // Ensure origin/HEAD points at the remote default, if origin exists.
            let _ = Command::new("git")
                .current_dir(&git_root)
                .args(["remote", "set-head", "origin", "-a"])
                .output()
                .await;
            // Compute fallback remote default
            let fallback_remote = code_core::git_worktree::detect_default_branch(&git_root)
                .await
                .map(|d| format!("origin/{d}"));
            let target_upstream = src_upstream.clone().or(fallback_remote);
            if let Some(up) = target_upstream {
                let set = Command::new("git")
                    .current_dir(&worktree)
                    .args([
                        "branch",
                        "--set-upstream-to",
                        up.as_str(),
                        used_branch.as_str(),
                    ])
                    .output()
                    .await;
                if let Ok(o) = set {
                    if o.status.success() {
                        _upstream_msg =
                            Some(format!("Set upstream for '{used_branch}' to {up}"));
                    } else {
                        let e = String::from_utf8_lossy(&o.stderr).trim().to_string();
                        if !e.is_empty() {
                            _upstream_msg = Some(format!("Upstream not set ({e})."));
                        }
                    }
                }
            }

            // Build clean multi-line output as a BackgroundEvent (not streaming Answer)
            let base_summary = branch_metadata
                .as_ref()
                .and_then(|meta| {
                    if let Some(remote_ref) = meta.remote_ref.as_ref() {
                        Some(format!("\n  Base: {remote_ref}"))
                    } else if let (Some(remote_name), Some(base_branch)) =
                        (meta.remote_name.as_ref(), meta.base_branch.as_ref())
                    {
                        Some(format!("\n  Base: {remote_name}/{base_branch}"))
                    } else { meta.remote_name.as_ref().map(|remote_name| format!("\n  Base remote: {remote_name}")) }
                })
                .unwrap_or_default();
            let msg = if let Some(task_text) = task_opt {
                format!(
                    "Created worktree '{used}'\n  Path: {path}\n  Copied {copied} changed files{base}\n  Task: {task}\n  Starting task...",
                    used = used_branch,
                    path = worktree.display(),
                    copied = copied,
                    task = task_text,
                    base = base_summary
                )
            } else {
                format!(
                    "Created worktree '{used}'\n  Path: {path}\n  Copied {copied} changed files{base}\n  Type your task when ready.",
                    used = used_branch,
                    path = worktree.display(),
                    copied = copied,
                    base = base_summary
                )
            };
            tx.send_background_event_with_ticket(&ticket, msg);

            // Switch cwd and optionally submit the task
            // Prefix the auto-submitted task so it's obvious it started in the new branch
            let initial_prompt = task_opt.map(|s| format!("[branch created] {s}"));
            tx.send(AppEvent::SwitchCwd(worktree, initial_prompt));
        });
    }

    pub(crate) fn handle_push_command(&mut self) {
        self.consume_pending_prompt_for_ui_only_turn();
        if self.ensure_git_repo_for_action(
            GitInitResume::DispatchCommand {
                command: SlashCommand::Push,
                command_text: "/push".to_string(),
            },
            "Pushing changes requires a git repository.",
        ) {
            return;
        }
        let Some(git_root) =
            code_core::git_info::resolve_root_git_project_for_trust(&self.config.cwd)
        else {
            self.push_background_tail("`/push` — run this command inside a git repository.".to_string());
            self.request_redraw();
            return;
        };

        self.push_background_tail("Commit, push and monitor workflows.".to_string());
        self.request_redraw();

        let tx = self.app_event_tx.clone();
        let ticket = self.make_background_tail_ticket();
        let worktree = git_root;

        tokio::spawn(async move {
            use std::fmt::Write as _;
            use tokio::{fs, process::Command};

            let short_status = match ChatWidget::git_short_status(&worktree).await {
                Ok(output) => output,
                Err(err) => {
                    tx.send_background_event_with_ticket(
                        &ticket,
                        format!("`/push` — failed to read git status: {err}"),
                    );
                    return;
                }
            };
            let has_dirty_changes = short_status.lines().any(|line| !line.trim().is_empty());

            let gh_available = Command::new("gh")
                .arg("--version")
                .output()
                .await
                .map(|out| out.status.success())
                .unwrap_or(false);

            let workflow_dir = worktree.join(".github").join("workflows");
            let workflows_exist = if fs::metadata(&workflow_dir)
                .await
                .map(|meta| meta.is_dir())
                .unwrap_or(false)
            {
                match fs::read_dir(&workflow_dir).await {
                    Ok(mut dir) => {
                        let mut found = false;
                        loop {
                            match dir.next_entry().await {
                                Ok(Some(entry)) => {
                                    if entry
                                        .file_type()
                                        .await
                                        .map(|ft| ft.is_file())
                                        .unwrap_or(false)
                                    {
                                        found = true;
                                        break;
                                    }
                                }
                                Ok(None) | Err(_) => {
                                    break;
                                }
                            }
                        }
                        found
                    }
                    Err(_) => false,
                }
            } else {
                false
            };

            let status_snippet = if short_status.trim().is_empty() {
                "(clean working tree)".to_string()
            } else {
                short_status.trim_end().to_string()
            };

            let diff_output = Command::new("git")
                .current_dir(&worktree)
                .args(["diff", "--cached"])
                .output()
                .await;
            let diff_snippet = match diff_output {
                Ok(out) if out.status.success() => {
                    let diff_text = String::from_utf8_lossy(&out.stdout);
                    if diff_text.trim().is_empty() {
                        "(no staged changes)".to_string()
                    } else {
                        const MAX_LINES: usize = 200;
                        const MAX_CHARS: usize = 16_000;
                        let mut preview = String::new();
                        let mut truncated = false;
                        let mut emitted = 0usize;
                        let mut chars = 0usize;
                        for line in diff_text.lines() {
                            if emitted >= MAX_LINES || chars >= MAX_CHARS {
                                truncated = true;
                                break;
                            }
                            preview.push_str(line);
                            preview.push('\n');
                            emitted += 1;
                            chars += line.len() + 1;
                        }
                        if truncated {
                            preview.push_str("…\n(truncated)\n");
                        }
                        preview.trim_end().to_string()
                    }
                }
                Ok(out) => {
                    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
                    if err.is_empty() {
                        "(failed to read staged diff)".to_string()
                    } else {
                        format!("(failed to read staged diff: {err})")
                    }
                }
                Err(err) => format!("(failed to run git diff --cached: {err})"),
            };

            let mut steps = Vec::new();
            if has_dirty_changes {
                steps.push(
                    "Briefly clean this repo (add working files/secrets to .gitignore, delete any temporary files) if neccessary, then commit all remaining dirty files."
                        .to_string(),
                );
            }
            steps.push(
                "Run git pull and merge any remote changes, carefully. Ensure conflicts are resolved line-by-line, do not bulk checkout or prefer changes from one side or the other."
                    .to_string(),
            );
            steps.push("Perform a git push.".to_string());
            if gh_available && workflows_exist {
                steps.push(
                    "Use gh_run_wait to monitor any triggered workflows."
                        .to_string(),
                );
                steps.push(
                    "If the workflow fails, then view errors, commit, push, monitor and repeat until the workflow succeeds."
                        .to_string(),
                );
            }

            let mut message = String::from("You have permission to commit and push.\n");
            message.push_str("\nRepository snapshot:\n");
            message.push_str("`git status --short`:\n");
            message.push_str(&status_snippet);
            message.push_str("\n\n`git diff --cached` (first 200 lines, 16k chars):\n");
            message.push_str(&diff_snippet);
            message.push_str("\n\n");
            for (idx, step) in steps.iter().enumerate() {
                let _ = writeln!(message, "{}. {}", idx + 1, step);
            }
            message.push_str(
                "You must ensure all workflows complete successfully. Do not yeild or respond until this has been completed.",
            );

            tx.send(AppEvent::SubmitHiddenTextWithPreface {
                agent_text: message,
                preface: String::new(),
                surface_notice: false,
            });
        });
    }

    pub(crate) fn handle_project_command(&mut self, args: String) {
        let name = args.trim();
        if name.is_empty() {
            self.history_push_plain_state(crate::history_cell::new_error_event(
                "`/cmd` — provide a project command name".to_string(),
            ));
            self.request_redraw();
            return;
        }

        if self.config.project_commands.is_empty() {
            self.history_push_plain_state(crate::history_cell::new_error_event(
                "No project commands configured for this workspace.".to_string(),
            ));
            self.request_redraw();
            return;
        }

        if let Some(cmd) = self
            .config
            .project_commands
            .iter()
            .find(|command| command.matches(name))
            .cloned()
        {
            let notice = if let Some(desc) = &cmd.description {
                format!("Running project command `{}` — {}", cmd.name, desc)
            } else {
                format!("Running project command `{}`", cmd.name)
            };
            self.insert_background_event_with_placement(
                notice,
                BackgroundPlacement::BeforeNextOutput,
                None,
            );
            self.request_redraw();
            self.submit_op(Op::RunProjectCommand { name: cmd.name });
        } else {
            let available: Vec<String> = self
                .config
                .project_commands
                .iter()
                .map(|cmd| cmd.name.clone())
                .collect();
            let suggestion = if available.is_empty() {
                "".to_string()
            } else {
                format!(" Available commands: {}", available.join(", "))
            };
            self.history_push_plain_state(crate::history_cell::new_error_event(format!(
                "Unknown project command `{name}`.{suggestion}"
            )));
            self.request_redraw();
        }
    }

    pub(crate) fn switch_cwd(
        &mut self,
        new_cwd: std::path::PathBuf,
        initial_prompt: Option<String>,
    ) {
        let previous_cwd = self.config.cwd.clone();
        self.config.cwd = new_cwd.clone();
        remember_cwd_history(&self.config.cwd);
        let ticket = self.make_background_tail_ticket();

        let msg = format!(
            "✅ Working directory changed\n  from: {}\n  to:   {}",
            previous_cwd.display(),
            new_cwd.display()
        );
        self.app_event_tx
            .send_background_event_with_ticket(&ticket, msg);

        let worktree_hint = new_cwd
            .file_name()
            .and_then(|n| n.to_str())
            .map(|name| format!(" (worktree: {name})"))
            .unwrap_or_default();
        let default_branch_note = format!(
            "System: Working directory changed from {} to {}{}. Use {} for subsequent commands.",
            previous_cwd.display(),
            new_cwd.display(),
            worktree_hint,
            new_cwd.display()
        );
        let branch_note = if Self::is_branch_worktree_path(&new_cwd) {
            if let Some(meta) = code_core::git_worktree::load_branch_metadata(&new_cwd) {
                let branch_name = new_cwd
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(std::string::ToString::to_string)
                    .unwrap_or_else(|| new_cwd.display().to_string());
                let base_descriptor = meta
                    .remote_ref
                    .clone()
                    .or_else(|| {
                        if let (Some(remote_name), Some(base_branch)) =
                            (meta.remote_name.clone(), meta.base_branch.clone())
                        {
                            Some(format!("{remote_name}/{base_branch}"))
                        } else {
                            None
                        }
                    })
                    .or(meta.base_branch.clone())
                    .unwrap_or_else(|| code_core::git_worktree::LOCAL_DEFAULT_REMOTE.to_string());
                let mut note = format!(
                    "System: Working directory changed from {} to {}{}. You are now working on branch '{}' checked out at {}. Compare against '{}' for the parent branch and run all commands from this directory.",
                    previous_cwd.display(),
                    new_cwd.display(),
                    worktree_hint,
                    branch_name,
                    new_cwd.display(),
                    base_descriptor
                );
                if let (Some(remote_name), Some(remote_url)) =
                    (meta.remote_name.as_ref(), meta.remote_url.as_ref())
                {
                    note.push_str(&format!(
                        " The remote '{remote_name}' points to {remote_url}."
                    ));
                }
                note
            } else {
                default_branch_note
            }
        } else {
            default_branch_note
        };
        self.queue_agent_note(branch_note);

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

        if let Some(prompt) = initial_prompt
            && !prompt.is_empty() {
                let preface = "[internal] When you finish this task, ask the user if they want any changes. If they are happy, offer to merge the branch back into the repository's default branch and delete the worktree. Use '/merge' (or an equivalent git worktree remove + switch) rather than deleting the folder directly so the UI can switch back cleanly. Wait for explicit confirmation before merging.".to_string();
                self.submit_text_message_with_preface(prompt, preface);
            }

        self.request_redraw();
    }

    /// Handle `/merge` for branch worktrees. Attempts a clean fast-forward
    /// when both checkouts are pristine; otherwise it hands the work to the agent
    /// with explicit manual instructions.
    pub(crate) fn handle_merge_command(&mut self) {
        self.consume_pending_prompt_for_ui_only_turn();
        if self.ensure_git_repo_for_action(
            GitInitResume::DispatchCommand {
                command: SlashCommand::Merge,
                command_text: "/merge".to_string(),
            },
            "Merging a branch worktree requires a git repository.",
        ) {
            return;
        }
        if !Self::is_branch_worktree_path(&self.config.cwd) {
            self.history_push_plain_state(crate::history_cell::new_error_event(
                "`/merge` — run this command from inside a branch worktree created with '/branch'.".to_string(),
            ));
            self.request_redraw();
            return;
        }

        let merge_ticket = self.make_background_tail_ticket();
        let tx = self.app_event_tx.clone();
        let work_cwd = self.config.cwd.clone();
        let ticket = merge_ticket;
        self.push_background_before_next_output(
            "Evaluating repository state before merging current branch...".to_string(),
        );
        self.request_redraw();

        tokio::spawn(async move {

            fn send_background(
                tx: &AppEventSender,
                ticket: &BackgroundOrderTicket,
                message: String,
            ) {
                tx.send_background_event_with_ticket(ticket, message);
            }

            fn send_background_late(
                tx: &AppEventSender,
                ticket: &BackgroundOrderTicket,
                message: String,
            ) {
                tx.send_background_event_with_ticket(ticket, message);
            }

            fn handoff_to_agent(
                tx: &AppEventSender,
                ticket: &BackgroundOrderTicket,
                state: MergeRepoState,
                mut reasons: Vec<String>,
            ) {
                if reasons.is_empty() {
                    reasons.push("manual follow-up requested".to_string());
                }
                let reason_text = reasons.join(", ");
                if state.git_root != state.worktree_path {
                    tx.send(AppEvent::SwitchCwd(state.git_root.clone(), None));
                }
                send_background(
                    tx,
                    ticket,
                    format!("`/merge` — handing off to agent ({reason_text})"),
                );
                let worktree_branch = state.worktree_branch.as_str();
                let visible =
                    format!("Finalize branch '{worktree_branch}' via /merge (agent merge required)");
                let preface = state.agent_preface(&reason_text);
                tx.send(AppEvent::SubmitTextWithPreface { visible, preface });
            }

            let git_root = match code_core::git_info::resolve_root_git_project_for_trust(&work_cwd) {
                Some(p) => p,
                None => {
                    send_background(&tx, &ticket, "`/merge` — not a git repo".to_string());
                    return;
                }
            };
            let merge_lock = ChatWidget::merge_lock_for_repo(&git_root);
            let _merge_guard = match merge_lock.try_lock() {
                Ok(guard) => guard,
                Err(_) => {
                    send_background(
                        &tx,
                        &ticket,
                        "`/merge` — waiting for an in-progress merge to finish".to_string(),
                    );
                    merge_lock.lock().await
                }
            };

            let state = match MergeRepoState::gather(work_cwd.clone(), git_root.clone()).await {
                Ok(state) => state,
                Err(err) => {
                    send_background(&tx, &ticket, format!("`/merge` — {err}"));
                    return;
                }
            };

            send_background(&tx, &ticket, state.snapshot_summary());

            let mut blockers = state.auto_fast_forward_blockers();
            if blockers.is_empty() {
                send_background(
                    &tx,
                    &ticket,
                    format!(
                        "`/merge` — attempting clean fast-forward of '{}' into '{}'",
                        state.worktree_branch,
                        state.default_branch_label()
                    ),
                );
                match run_fast_forward_merge(&state).await {
                    Ok(()) => {
                        send_background_late(
                            &tx,
                            &ticket,
                            format!(
                                "`/merge` — fast-forwarded '{}' into '{}' and removed the worktree",
                                state.worktree_branch,
                                state.default_branch_label()
                            ),
                        );
                        tx.send(AppEvent::SwitchCwd(state.git_root.clone(), None));
                        return;
                    }
                    Err(err) => {
                        blockers.push(err);
                    }
                }
            }

            handoff_to_agent(&tx, &ticket, state, blockers);
        });
    }
}
