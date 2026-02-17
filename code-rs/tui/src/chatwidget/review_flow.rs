use super::*;
use code_protocol::protocol::ReviewTarget;

impl ChatWidget<'_> {
    pub(super) fn auto_resolve_enabled(&self) -> bool {
        self.auto_resolve_state.is_some()
    }

    pub(super) fn configured_auto_resolve_re_reviews(&self) -> u32 {
        self.config
            .auto_drive
            .auto_resolve_review_attempts
            .get()
    }

    pub(super) fn auto_resolve_clear(&mut self) {
        self.auto_resolve_state = None;
        self.maybe_resume_auto_after_review();
    }

    pub(super) fn auto_resolve_notice<S: Into<String>>(&mut self, message: S) {
        self.push_background_tail(message.into());
        self.request_redraw();
    }

    pub(super) fn auto_resolve_commit_sha(&self) -> Option<String> {
        self.auto_resolve_state
            .as_ref()
            .and_then(|state| match &state.target {
                ReviewTarget::Commit { sha, .. } => Some(sha.clone()),
                _ => None,
            })
    }

    pub(super) fn worktree_has_uncommitted_changes(&self) -> Option<bool> {
        let output = Command::new("git")
            .current_dir(&self.config.cwd)
            .args(["status", "--short"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        Some(!stdout.trim().is_empty())
    }

    pub(super) fn current_head_commit_sha(&self) -> Option<String> {
        let output = Command::new("git")
            .current_dir(&self.config.cwd)
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            None
        } else {
            Some(stdout)
        }
    }

    pub(super) fn commit_subject_for(&self, commit: &str) -> Option<String> {
        let output = Command::new("git")
            .current_dir(&self.config.cwd)
            .args(["show", "-s", "--format=%s", commit])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if stdout.is_empty() {
            None
        } else {
            Some(stdout)
        }
    }

    pub(super) fn strip_context_sections(text: &str) -> String {
        // Remove any <context>...</context> blocks. If a closing tag is missing,
        // drop everything from the opening tag to the end of the string so we
        // never leak a stray <context> marker back into the next prompt.
        const START: &str = "<context"; // allow attributes or whitespace before '>'
        const END: &str = "</context>";

        let lower = text.to_ascii_lowercase();
        let mut cleaned = String::with_capacity(text.len());
        let mut cursor: usize = 0;

        while let Some(start_rel) = lower[cursor..].find(START) {
            let start = cursor + start_rel;
            cleaned.push_str(&text[cursor..start]);

            // Advance past the opening tag terminator '>' if present; otherwise
            // treat the rest of the string as part of the unclosed context block.
            let after_start = match text[start..].find('>') {
                Some(off) => start + off + 1,
                None => return cleaned, // Unclosed start tag: drop the remainder
            };

            // Look for the matching closing tag. If not found, drop the tail.
            if let Some(end_rel) = lower[after_start..].find(END) {
                let end = after_start + end_rel + END.len();
                cursor = end;
            } else {
                return cleaned;
            }
        }

        // Append any trailing text after the last removed block.
        cleaned.push_str(&text[cursor..]);

        // Clean up any stray closing tags that had no opener.
        if cleaned.contains(END) {
            cleaned = cleaned.replace(END, "");
        }

        cleaned
    }

    pub(super) fn turn_context_block(&self) -> Option<String> {
        let mut lines: Vec<String> = Vec::new();
        let mut any = false;
        lines.push("<context>".to_string());
        lines.push("Below are the most recent messages related to this code change.".to_string());
        if let Some(user) = self
            .last_user_message
            .as_ref()
            .map(|msg| Self::strip_context_sections(msg))
            .map(|msg| msg.trim().to_string())
            .filter(|msg| !msg.is_empty())
        {
            any = true;
            lines.push(format!("<user>{user}</user>"));
        }
        if let Some(dev) = self
            .last_developer_message
            .as_ref()
            .map(|msg| Self::strip_context_sections(msg))
            .map(|msg| msg.trim().to_string())
            .filter(|msg| !msg.is_empty())
        {
            any = true;
            lines.push(format!("<developer>{dev}</developer>"));
        }
        if let Some(assistant) = self
            .last_assistant_message
            .as_ref()
            .map(|msg| Self::strip_context_sections(msg))
            .map(|msg| msg.trim().to_string())
            .filter(|msg| !msg.is_empty())
        {
            any = true;
            lines.push(format!("<assistant>{assistant}</assistant>"));
        }
        lines.push("</context>".to_string());

        if any {
            Some(lines.join("\n"))
        } else {
            None
        }
    }

    pub(super) fn auto_resolve_should_block_auto_resume(&self) -> bool {
        match self.auto_resolve_state.as_ref().map(|state| &state.phase) {
            Some(AutoResolvePhase::PendingFix { .. })
            | Some(AutoResolvePhase::AwaitingFix { .. })
            | Some(AutoResolvePhase::AwaitingJudge { .. }) => true,
            Some(AutoResolvePhase::WaitingForReview) => self.is_review_flow_active(),
            None => false,
        }
    }

    pub(super) fn maybe_resume_auto_after_review(&mut self) {
        if !self.auto_state.is_active() || !self.auto_state.awaiting_review() {
            return;
        }
        if self.is_review_flow_active() || self.auto_resolve_should_block_auto_resume() {
            return;
        }
        self.auto_state.on_complete_review();
        if !self.auto_state.should_bypass_coordinator_next_submit() {
            self.auto_send_conversation();
        }
        self.request_redraw();
    }

    pub(super) fn auto_resolve_format_findings(review: &ReviewOutputEvent) -> String {
        let mut sections: Vec<String> = Vec::new();
        if !review.findings.is_empty() {
            sections.push(format_review_findings_block(&review.findings, None));
        }
        let explanation = review.overall_explanation.trim();
        if !explanation.is_empty() {
            sections.push(explanation.to_string());
        }
        sections
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub(super) fn auto_resolve_handle_review_enter(&mut self) {
        if let Some(state) = self.auto_resolve_state.as_mut() {
            state.phase = AutoResolvePhase::WaitingForReview;
            state.last_review = None;
            state.last_fix_message = None;
        }
    }

    pub(super) fn auto_resolve_handle_review_exit(&mut self, review_output: Option<ReviewOutputEvent>) {
        if self.auto_resolve_state.is_none() {
            return;
        }

        let notice: Option<String>;
        let mut should_clear = false;
        {
            let Some(state) = self.auto_resolve_state.as_mut() else {
                return;
            };
            match review_output {
                Some(ref output) => {
                    state.attempt = state.attempt.saturating_add(1);
                    state.last_review = Some(output.clone());
                    state.last_fix_message = None;

                    if output.findings.is_empty() {
                        notice = Some("Auto-resolve: review reported no actionable findings. Exiting.".to_string());
                        should_clear = true;
                    } else if state.max_attempts > 0 && state.attempt > state.max_attempts {
                        let limit = state.max_attempts;
                        notice = Some(match limit {
                            0 => "Auto-resolve: attempt limit is set to 0, so automation stopped after the initial review.".to_string(),
                            1 => "Auto-resolve: reached the review attempt limit (1 allowed review). Handing control back to you.".to_string(),
                            _ => format!(
                                "Auto-resolve: reached the review attempt limit ({limit} allowed reviews). Handing control back to you."
                            ),
                        });
                        should_clear = true;
                    } else {
                        state.phase = AutoResolvePhase::PendingFix {
                            review: output.clone(),
                        };
                        notice = Some("Auto-resolve: review found issues. Preparing follow-up fix request.".to_string());
                    }
                }
                None => {
                    notice = Some(
                        "Auto-resolve: review ended without findings. Please inspect manually.".to_string(),
                    );
                    should_clear = true;
                }
            }
        }

        if should_clear {
            self.auto_resolve_clear();
        }
        if let Some(message) = notice {
            self.auto_resolve_notice(message);
        }
    }

    pub(super) fn auto_resolve_on_task_complete(&mut self, last_agent_message: Option<String>) {
        let Some(state_snapshot) = self.auto_resolve_state.clone() else {
            return;
        };

        match state_snapshot.phase {
            AutoResolvePhase::PendingFix { review } => {
                if let Some(state) = self.auto_resolve_state.as_mut() {
                    state.phase = AutoResolvePhase::AwaitingFix {
                        review: review.clone(),
                    };
                }
                self.dispatch_auto_fix(&review);
            }
            AutoResolvePhase::AwaitingFix { review } => {
                if let Some(state) = self.auto_resolve_state.as_mut() {
                    state.last_fix_message = last_agent_message.clone();
                    state.phase = AutoResolvePhase::AwaitingJudge {
                        review: review.clone(),
                    };
                }
                self.dispatch_auto_judge(&review, last_agent_message);
            }
            AutoResolvePhase::AwaitingJudge { review } => {
                let message = last_agent_message.unwrap_or_default();
                self.auto_resolve_process_judge(review, message);
            }
            AutoResolvePhase::WaitingForReview => {}
        }
    }

    pub(super) fn dispatch_auto_fix(&mut self, review: &ReviewOutputEvent) {
        let summary = Self::auto_resolve_format_findings(review);
        let mut preface = String::from(
            "You are continuing an automated /review resolution loop. Review the listed findings and determine whether they represent real issues introduced by our changes. If they are, apply the necessary fixes and resolve any similar issues you can identify before responding."
        );
        if !summary.is_empty() {
            preface.push_str("\n\nFindings:\n");
            preface.push_str(&summary);
        }
        if let Some(commit) = self.auto_resolve_commit_sha() {
                let short_sha: String = commit.chars().take(7).collect();
                preface.push_str("\n\nCommit under review: ");
                preface.push_str(&commit);
                preface.push_str(" (short SHA ");
                preface.push_str(&short_sha);
                preface.push_str(
                    "). If you make changes to address these findings, amend this commit before responding so the review target reflects your fixes.",
                );
            }

        // Pass the full structured output so the resolving agent sees file paths and line ranges.
        if let Ok(raw_json) = serde_json::to_string_pretty(review) {
            preface.push_str("\n\nFull review JSON (includes file paths and line ranges):\n");
            preface.push_str(&raw_json);
        }

        if let Some(context) = self.turn_context_block() {
            preface.push_str("\n\n");
            preface.push_str(&context);
        }

        self.auto_resolve_notice("Auto-resolve: asking the agent to verify and address the review findings.");
        self.submit_hidden_text_message_with_preface(
            "Is this a real issue introduced by our changes? If so, please fix and resolve all similar issues.".to_string(),
            preface,
        );
    }

    pub(super) fn dispatch_auto_judge(&mut self, review: &ReviewOutputEvent, fix_message: Option<String>) {
        let summary = Self::auto_resolve_format_findings(review);
        let mut preface = String::from(
            "You are evaluating whether the latest fixes resolved the findings from `/review`. Respond with a strict JSON object containing `status` and optional `rationale`. Valid `status` values: `review_again`, `no_issue`, `continue_fix`. Do not include any additional text before or after the JSON."
        );
        if !summary.is_empty() {
            preface.push_str("\n\nOriginal findings:\n");
            preface.push_str(&summary);
        }
        if let Some(fix) = fix_message.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
            preface.push_str("\n\nLatest agent response:\n");
            preface.push_str(fix);
        }
        preface.push_str("\n\nReturn JSON: {\"status\": \"...\", \"rationale\": \"optional explanation\"}.");
        if let Some(commit) = self.auto_resolve_commit_sha() {
                let short_sha: String = commit.chars().take(7).collect();
                preface.push_str("\n\nCommit under review: ");
                preface.push_str(&commit);
                preface.push_str(" (short SHA ");
                preface.push_str(&short_sha);
                preface.push_str(
                    "). Confirm that any fixes have been committed (amend the commit if necessary) before returning `no_issue`.",
                );
            }

        if let Some(context) = self.turn_context_block() {
            preface.push_str("\n\n");
            preface.push_str(&context);
        }

        self.auto_resolve_notice("Auto-resolve: requesting status JSON from the agent.");
        self.submit_hidden_text_message_with_preface("Auto-resolve status check".to_string(), preface);
    }

    pub(super) fn dispatch_auto_continue(&mut self, review: &ReviewOutputEvent) {
        let summary = Self::auto_resolve_format_findings(review);
        let mut preface = String::from(
            "The previous status check indicated more work is required on the review findings. Continue addressing the remaining issues before responding."
        );
        if !summary.is_empty() {
            preface.push_str("\n\nOutstanding findings:\n");
            preface.push_str(&summary);
        }
        if let Some(context) = self.turn_context_block() {
            preface.push_str("\n\n");
            preface.push_str(&context);
        }
        self.auto_resolve_notice("Auto-resolve: asking the agent to continue working on the findings.");
        self.submit_hidden_text_message_with_preface("Please continue".to_string(), preface);
    }

    pub(super) fn restart_auto_resolve_review(&mut self) {
        let Some(state_snapshot) = self.auto_resolve_state.clone() else {
            return;
        };
        let next_attempt = state_snapshot.attempt.saturating_add(1);
        let re_reviews_allowed = state_snapshot.max_attempts;
        let total_allowed = re_reviews_allowed.saturating_add(1);
        let attempt_label = if re_reviews_allowed == 0 {
            "attempt limit reached".to_string()
        } else {
            format!("attempt {next_attempt} of {total_allowed}")
        };
        let prep_label = format!("Preparing follow-up code review ({attempt_label})");
        let mut base_prompt = state_snapshot.prompt.trim_end().to_string();
        if let Some(idx) = base_prompt.find(AUTO_RESOLVE_REVIEW_FOLLOWUP) {
            base_prompt = base_prompt[..idx].trim_end().to_string();
        }

        let mut next_hint = state_snapshot.hint.clone();
        let mut next_target = state_snapshot.target.clone();

        if matches!(next_target, ReviewTarget::Commit { .. })
            && let Some(new_commit) = self.current_head_commit_sha()
        {
            let short_sha: String = new_commit.chars().take(7).collect();
            let subject = self.commit_subject_for(&new_commit);
            base_prompt = match subject.as_deref() {
                Some(subject) => format!(
                    "Review the code changes introduced by commit {new_commit} (\"{subject}\"). Provide prioritized, actionable findings."
                ),
                None => format!(
                    "Review the code changes introduced by commit {new_commit}. Provide prioritized, actionable findings."
                ),
            };
            next_hint = format!("commit {short_sha}");
            next_target = ReviewTarget::Commit {
                sha: new_commit,
                title: subject,
            };
        }

        let mut continued_prompt = base_prompt.clone();
        if let Some(last_review) = state_snapshot.last_review.as_ref() {
            let recap = Self::auto_resolve_format_findings(last_review);
            if !recap.is_empty() {
                continued_prompt.push_str("\n\nPreviously reported findings to re-validate:\n");
                continued_prompt.push_str(&recap);
            }
        }
        if let ReviewTarget::Commit { sha, .. } = &state_snapshot.target
            && let Some(true) = self.worktree_has_uncommitted_changes()
        {
            continued_prompt.push_str(
                "\n\nNote: there are uncommitted changes in the working tree since commit ",
            );
            continued_prompt.push_str(sha);
            continued_prompt.push_str(
                ". Ensure the review covers the updated workspace rather than only the original commit snapshot.",
            );
        }
        continued_prompt.push_str("\n\n");
        continued_prompt.push_str(AUTO_RESOLVE_REVIEW_FOLLOWUP);
        let hint = (!next_hint.trim().is_empty()).then(|| next_hint.clone());
        self.begin_review(next_target.clone(), continued_prompt, hint, Some(prep_label));
        if let Some(state) = self.auto_resolve_state.as_mut() {
            state.phase = AutoResolvePhase::WaitingForReview;
            state.target = next_target;
            state.prompt = base_prompt;
            state.hint = next_hint;
            state.last_review = None;
            state.last_fix_message = None;
        }
    }

    pub(super) fn auto_resolve_process_judge(&mut self, review: ReviewOutputEvent, message: String) {
        let trimmed = message.trim();
        let Some(decision) = Self::auto_resolve_parse_decision(trimmed) else {
            self.auto_resolve_notice("Auto-resolve: expected JSON status but received something else. Stopping automation.");
            self.auto_resolve_clear();
            return;
        };

        let status = decision.status.to_ascii_lowercase();
        let rationale = decision.rationale.unwrap_or_default();

        match status.as_str() {
            "no_issue" => {
                let rationale_text = rationale.trim();
                let attempt_limit_reached = self
                    .auto_resolve_state
                    .as_ref()
                    .is_some_and(|state| {
                        let allowed = state.max_attempts.saturating_add(1);
                        state.attempt >= allowed
                    });

                if attempt_limit_reached {
                    let limit = self
                        .auto_resolve_state
                        .as_ref()
                        .map(|state| state.max_attempts)
                        .unwrap_or(0);
                    let message = if rationale_text.is_empty() {
                        match limit {
                            0 => "Auto-resolve: agent reported no remaining issues but automation is disabled (limit 0). Please inspect manually.".to_string(),
                            1 => "Auto-resolve: agent reported no remaining issues but hit the single allowed review. Please inspect manually.".to_string(),
                            _ => format!(
                                "Auto-resolve: agent reported no remaining issues but hit the review attempt limit ({limit}). Please inspect manually."
                            ),
                        }
                    } else {
                        match limit {
                            0 => format!(
                                "Auto-resolve: no remaining issues. {rationale_text} Automation is disabled (limit 0); handing control back to you."
                            ),
                            1 => format!(
                                "Auto-resolve: no remaining issues. {rationale_text} The single allowed review is complete; handing control back to you."
                            ),
                            _ => format!(
                                "Auto-resolve: no remaining issues. {rationale_text} Review attempt limit ({limit}) reached; handing control back to you."
                            ),
                        }
                    };
                    self.auto_resolve_notice(message);
                    self.auto_resolve_clear();
                } else {
                    if rationale_text.is_empty() {
                        self.auto_resolve_notice(
                            "Auto-resolve: agent reported no remaining issues. Running follow-up /review to confirm.".to_string(),
                        );
                    } else {
                        self.auto_resolve_notice(format!(
                            "Auto-resolve: no remaining issues. {rationale_text} Running follow-up /review to confirm."
                        ));
                    }
                    if let Some(state) = self.auto_resolve_state.as_mut() {
                        state.phase = AutoResolvePhase::WaitingForReview;
                    }
                    self.restart_auto_resolve_review();
                }
            }
            "continue_fix" => {
                if let Some(state) = self.auto_resolve_state.as_mut() {
                    state.phase = AutoResolvePhase::AwaitingFix {
                        review: review.clone(),
                    };
                }
                self.dispatch_auto_continue(&review);
            }
            "review_again" => {
                let stop = self
                    .auto_resolve_state
                    .as_ref()
                    .is_some_and(|state| {
                        let allowed = state.max_attempts.saturating_add(1);
                        state.attempt >= allowed
                    });
                if stop {
                    let limit = self
                        .auto_resolve_state
                        .as_ref()
                        .map(|state| state.max_attempts)
                        .unwrap_or(0);
                    let message = if limit == 0 {
                        "Auto-resolve: review-again requested but automation is disabled (limit 0). Stopping.".to_string()
                    } else if limit == 1 {
                        "Auto-resolve: review-again requested but the attempt limit has been reached (1 allowed review). Stopping.".to_string()
                    } else {
                        format!(
                            "Auto-resolve: review-again requested but the attempt limit has been reached ({limit} allowed reviews). Stopping."
                        )
                    };
                    self.auto_resolve_notice(message);
                    self.auto_resolve_clear();
                } else {
                    if rationale.trim().is_empty() {
                        self.auto_resolve_notice("Auto-resolve: running another /review pass.".to_string());
                    } else {
                        let rationale_text = rationale.trim();
                        self.auto_resolve_notice(format!(
                            "Auto-resolve: running another /review pass. {rationale_text}"
                        ));
                    }
                    self.restart_auto_resolve_review();
                }
            }
            other => {
                self.auto_resolve_notice(format!(
                    "Auto-resolve: unexpected status '{other}'. Stopping automation."
                ));
                self.auto_resolve_clear();
            }
        }
    }

    pub(super) fn auto_resolve_parse_decision(raw: &str) -> Option<AutoResolveDecision> {
        if let Ok(decision) = serde_json::from_str::<AutoResolveDecision>(raw) {
            return Some(decision);
        }

        if let Some(start) = raw.find("{" )
            && let Some(end) = raw.rfind("}") {
                let slice = &raw[start..=end];
                if let Ok(decision) = serde_json::from_str::<AutoResolveDecision>(slice) {
                    return Some(decision);
                }
            }

        // try to strip ```json fences
        if let Some(json_start) = raw.find("```")
            && let Some(rest) = raw[json_start + 3..].split_once("```") {
                let candidate = rest.0.trim_start_matches("json").trim();
                if let Ok(decision) = serde_json::from_str::<AutoResolveDecision>(candidate) {
                    return Some(decision);
                }
            }

        None
    }

    pub(crate) fn open_review_dialog(&mut self) {
        if self.is_task_running() {
            self.history_push_plain_state(crate::history_cell::new_error_event(
                "`/review` — complete or cancel the current task before starting a new review.".to_string(),
            ));
            self.request_redraw();
            return;
        }

        let mut items: Vec<SelectionItem> = Vec::new();

        let max_attempts = self.configured_auto_resolve_re_reviews();
        let auto_note = if self.config.tui.review_auto_resolve {
            if max_attempts == 0 {
                "Auto Resolve is enabled (no automatic re-reviews)."
            } else if max_attempts == 1 {
                "Auto Resolve is enabled (max 1 re-review)."
            } else {
                "Auto Resolve is enabled."
            }
        } else {
            "Auto Resolve is disabled."
        };
        items.push(SelectionItem {
            name: "Auto Resolve settings moved to /settings".to_string(),
            description: Some(format!(
                "{auto_note} Manage Auto Resolve reviews and max re-reviews via `/settings review`."
            )),
            is_current: false,
            actions: vec![Box::new(|tx: &crate::app_event_sender::AppEventSender| {
                tx.send(crate::app_event::AppEvent::DispatchCommand(
                    SlashCommand::Settings,
                    "review".to_string(),
                ));
            })],
        });

        let workspace_prompt = "Review the current workspace changes (staged, unstaged, and untracked files) and highlight bugs, regressions, risky patterns, and missing tests before merge.".to_string();
        let workspace_hint = "current workspace changes".to_string();
        let workspace_preparation = "Preparing code review for current changes".to_string();
        let workspace_auto_resolve = self.config.tui.review_auto_resolve;
        items.push(SelectionItem {
            name: "Review uncommitted changes".to_string(),
            description: Some("Look at staged, unstaged, and untracked files".to_string()),
            is_current: false,
            actions: vec![Box::new({
                let prompt = workspace_prompt;
                let hint = workspace_hint;
                let preparation = workspace_preparation;
                move |tx: &crate::app_event_sender::AppEventSender| {
                    tx.send(crate::app_event::AppEvent::RunReviewWithScope {
                        target: ReviewTarget::UncommittedChanges,
                        prompt: prompt.clone(),
                        hint: Some(hint.clone()),
                        preparation_label: Some(preparation.clone()),
                        auto_resolve: workspace_auto_resolve,
                    });
                }
            })],
        });

        items.push(SelectionItem {
            name: "Review /branch changes".to_string(),
            description: Some("Compare your worktree branch against its merge target".to_string()),
            is_current: false,
            actions: vec![Box::new(|tx: &crate::app_event_sender::AppEventSender| {
                tx.send(crate::app_event::AppEvent::RunReviewCommand(String::new()));
            })],
        });

        items.push(SelectionItem {
            name: "Review a specific commit".to_string(),
            description: Some("Pick from recent commits".to_string()),
            is_current: false,
            actions: vec![Box::new(|tx: &crate::app_event_sender::AppEventSender| {
                tx.send(crate::app_event::AppEvent::StartReviewCommitPicker);
            })],
        });

        items.push(SelectionItem {
            name: "Review against a base branch".to_string(),
            description: Some("Diff current branch against another".to_string()),
            is_current: false,
            actions: vec![Box::new(|tx: &crate::app_event_sender::AppEventSender| {
                tx.send(crate::app_event::AppEvent::StartReviewBranchPicker);
            })],
        });

        items.push(SelectionItem {
            name: "Custom review instructions".to_string(),
            description: Some("Describe exactly what to audit".to_string()),
            is_current: false,
            actions: vec![Box::new(|tx: &crate::app_event_sender::AppEventSender| {
                tx.send(crate::app_event::AppEvent::OpenReviewCustomPrompt);
            })],
        });

        let view: ListSelectionView = ListSelectionView::new(
            " Review options ".to_string(),
            Some("Choose what scope to review".to_string()),
            Some("Enter select · Esc cancel".to_string()),
            items,
            self.app_event_tx.clone(),
            6,
        );

        self.bottom_pane.show_list_selection(
            "Review options".to_string(),
            None,
            None,
            view,
        );
    }

    pub(crate) fn show_review_custom_prompt(&mut self) {
        let submit_tx = self.app_event_tx.clone();
        let on_submit: Box<dyn Fn(String) + Send + Sync> = Box::new(move |text: String| {
            submit_tx.send(crate::app_event::AppEvent::RunReviewCommand(text));
        });
        let view = CustomPromptView::new(
            "Custom review instructions".to_string(),
            "Describe the files or changes you want reviewed".to_string(),
            Some("Press Enter to submit · Esc cancel".to_string()),
            self.app_event_tx.clone(),
            None,
            on_submit,
        );
        self.bottom_pane.show_custom_prompt(view);
    }

    pub(crate) fn set_review_auto_resolve_enabled(&mut self, enabled: bool) {
        if self.config.tui.review_auto_resolve == enabled {
            return;
        }

        self.config.tui.review_auto_resolve = enabled;
        if !enabled {
            self.auto_resolve_clear();
        }

        let message = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_tui_review_auto_resolve(&home, enabled) {
                Ok(_) => {
                    tracing::info!("Persisted review auto resolve toggle: {}", enabled);
                    if enabled {
                        "Auto Resolve reviews enabled."
                    } else {
                        "Auto Resolve reviews disabled."
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to persist review auto resolve toggle: {}", e);
                    if enabled {
                        "Auto Resolve enabled for this session (failed to persist)."
                    } else {
                        "Auto Resolve disabled for this session (failed to persist)."
                    }
                }
            }
        } else {
            tracing::warn!("Could not locate Codex home to persist review auto resolve toggle");
            if enabled {
                "Auto Resolve enabled for this session."
            } else {
                "Auto Resolve disabled for this session."
            }
        };

        self.bottom_pane.flash_footer_notice(message.to_string());
        self.refresh_settings_overview_rows();
        self.update_review_settings_model_row();
        self.request_redraw();
    }

    pub(crate) fn set_auto_review_enabled(&mut self, enabled: bool) {
        if self.config.tui.auto_review_enabled == enabled {
            return;
        }

        self.config.tui.auto_review_enabled = enabled;

        let message = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_tui_auto_review_enabled(&home, enabled) {
                Ok(_) => {
                    tracing::info!("Persisted auto review toggle: {}", enabled);
                    if enabled {
                        "Auto Review enabled."
                    } else {
                        "Auto Review disabled."
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to persist auto review toggle: {}", e);
                    if enabled {
                        "Auto Review enabled for this session (failed to persist)."
                    } else {
                        "Auto Review disabled for this session (failed to persist)."
                    }
                }
            }
        } else {
            tracing::warn!("Could not locate Codex home to persist auto review toggle");
            if enabled {
                "Auto Review enabled for this session."
            } else {
                "Auto Review disabled for this session."
            }
        };

        self.bottom_pane.flash_footer_notice(message.to_string());
        self.refresh_settings_overview_rows();
        self.update_review_settings_model_row();
        self.request_redraw();
    }

    pub(super) fn auto_review_git_root(&self) -> Option<PathBuf> {
        self.run_git_command(["rev-parse", "--show-toplevel"], |stdout| {
            let root = stdout.lines().next().unwrap_or("").trim();
            if root.is_empty() {
                Err("auto review git root unavailable".to_string())
            } else {
                Ok(root.to_string())
            }
        })
        .ok()
        .map(PathBuf::from)
    }

    pub(super) fn auto_review_baseline_path(&self) -> Option<PathBuf> {
        let git_root = self.auto_review_git_root()?;
        match auto_review_baseline_path_for_repo(&git_root) {
            Ok(path) => Some(path),
            Err(err) => {
                tracing::warn!("failed to resolve auto review baseline path: {err}");
                None
            }
        }
    }

    pub(super) fn load_auto_review_baseline_marker(&mut self) {
        if self.auto_review_reviewed_marker.is_some() {
            return;
        }
        let Some(path) = self.auto_review_baseline_path() else {
            return;
        };
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return;
        };
        let commit_id = contents.lines().next().unwrap_or("").trim();
        if commit_id.is_empty() {
            return;
        }
        self.auto_review_reviewed_marker = Some(GhostCommit::new(commit_id.to_string(), None));
    }

    pub(super) fn persist_auto_review_baseline_marker(&self, commit_id: &str) {
        let commit_id = commit_id.trim();
        if commit_id.is_empty() {
            return;
        }
        let Some(path) = self.auto_review_baseline_path() else {
            return;
        };
        if let Some(parent) = path.parent()
            && let Err(err) = std::fs::create_dir_all(parent) {
                tracing::warn!("failed to create auto review baseline dir: {err}");
                return;
            }
        if let Err(err) = std::fs::write(&path, format!("{commit_id}\n")) {
            tracing::warn!("failed to persist auto review baseline: {err}");
        }
    }

    pub(super) fn maybe_trigger_auto_review(&mut self) {
        if !self.config.tui.auto_review_enabled {
            return;
        }
        self.recover_stuck_background_review();

        if !self.turn_had_code_edits && self.pending_auto_review_range.is_none() {
            return;
        }
        if matches!(self.current_turn_origin, Some(TurnOrigin::Developer)) {
            return;
        }

        if self.pending_auto_review_deferred_for_current_turn() {
            return;
        }

        if let Some(reviewed) = self.auto_review_reviewed_marker.as_ref()
            && !self.auto_review_has_changes_since(reviewed) {
                return;
            }

        if self.background_review.is_some() || self.is_review_flow_active() {
            if let Some(base) = self.take_or_capture_auto_review_baseline() {
                self.queue_skipped_auto_review(base);
            }
            return;
        }

        let base_snapshot = if let Some(base) = self.take_ready_pending_range_base() {
            Some(base)
        } else {
            self.take_or_capture_auto_review_baseline()
        };

        if let Some(base) = base_snapshot {
            self.launch_background_review(Some(base));
        }
    }

    pub(super) fn auto_review_has_changes_since(&self, reviewed: &GhostCommit) -> bool {
        let reviewed_id = reviewed.id();
        let tracked_changes = match self.run_git_command(
            ["diff", "--name-only", reviewed_id],
            |stdout| {
                Ok(stdout.lines().any(|line| !line.trim().is_empty()))
            },
        ) {
            Ok(changed) => changed,
            Err(err) => {
                tracing::warn!("auto review diff failed for {reviewed_id}: {err}");
                return true;
            }
        };

        if tracked_changes {
            return true;
        }

        let snapshot_paths = match self.run_git_command(
            ["ls-tree", "-r", "--name-only", reviewed_id],
            |stdout| {
                let mut paths = HashSet::new();
                for line in stdout.lines().map(str::trim) {
                    if !line.is_empty() {
                        paths.insert(line.to_string());
                    }
                }
                Ok(paths)
            },
        ) {
            Ok(paths) => paths,
            Err(err) => {
                tracing::warn!("auto review snapshot listing failed for {reviewed_id}: {err}");
                return true;
            }
        };

        

        match self.run_git_command(
            ["ls-files", "--others", "--exclude-standard"],
            |stdout| {
                Ok(stdout
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .any(|path| !snapshot_paths.contains(path)))
            },
        ) {
            Ok(changed) => changed,
            Err(err) => {
                tracing::warn!("auto review untracked check failed: {err}");
                true
            }
        }
    }

    pub(super) fn pending_auto_review_deferred_for_current_turn(&self) -> bool {
        matches!(
            self.pending_auto_review_range.as_ref(),
            Some(range)
                if matches!(range.defer_until_turn, Some(turn) if turn == self.turn_sequence)
        )
    }

    pub(super) fn take_ready_pending_range_base(&mut self) -> Option<GhostCommit> {
        if let Some(range) = self.pending_auto_review_range.as_ref()
            && let Some(turn) = range.defer_until_turn
                && turn == self.turn_sequence {
                    return None;
                }
        self.pending_auto_review_range.take().map(|range| range.base)
    }

    pub(super) fn take_or_capture_auto_review_baseline(&mut self) -> Option<GhostCommit> {
        if let Some(existing) = self.auto_review_baseline.take() {
            return Some(existing);
        }
        match self.capture_auto_turn_commit("auto review baseline snapshot", None) {
            Ok(commit) => Some(commit),
            Err(err) => {
                tracing::warn!("failed to capture auto review baseline: {err}");
                self.auto_review_reviewed_marker.clone()
            }
        }
    }

    pub(super) fn queue_skipped_auto_review(&mut self, base: GhostCommit) {
        if self.pending_auto_review_range.is_some() {
            return;
        }
        self.pending_auto_review_range = Some(PendingAutoReviewRange {
            base,
            defer_until_turn: None,
        });
    }

    pub(super) fn recover_stuck_background_review(&mut self) {
        let Some(state) = self.background_review.as_ref() else {
            return;
        };

        let elapsed = state.last_seen.elapsed();
        if elapsed.as_secs() < AUTO_REVIEW_STALE_SECS {
            return;
        }

        let stale = self.background_review.take();
        self.background_review_guard = None;

        let Some(stale) = stale else {
            return;
        };

        if self.pending_auto_review_range.is_none()
            && let Some(base) = stale.base {
                self.pending_auto_review_range = Some(PendingAutoReviewRange {
                    base,
                    defer_until_turn: None,
                });
            }
    }

    pub(super) fn launch_background_review(&mut self, base_snapshot: Option<GhostCommit>) {
        // Record state immediately to avoid duplicate launches when multiple
        // TaskComplete events fire in quick succession.
        self.turn_had_code_edits = false;
        let had_notice = self.auto_review_notice.is_some();
        let had_fixed_indicator = matches!(
            self.auto_review_status,
            Some(AutoReviewStatus { status: AutoReviewIndicatorStatus::Fixed, .. })
        );
        self.background_review = Some(BackgroundReviewState {
            worktree_path: std::path::PathBuf::new(),
            branch: String::new(),
            agent_id: None,
            snapshot: None,
            base: base_snapshot.clone(),
            last_seen: std::time::Instant::now(),
        });
        self.auto_review_status = None;
        self.bottom_pane.set_auto_review_status(None);
        self.auto_review_notice = None;
        self.set_auto_review_indicator(
            AutoReviewIndicatorStatus::Running,
            None,
            AutoReviewPhase::Reviewing,
        );

        #[cfg(test)]
        if let Some(stub) = AUTO_REVIEW_STUB.lock().unwrap().as_mut() {
            (stub)();
            return;
        }

        let config = self.config.clone();
        let app_event_tx = self.app_event_tx.clone();
        let base_snapshot_for_task = base_snapshot;
        let turn_context = self.turn_context_block();
        let prefer_fallback = had_notice || had_fixed_indicator;
        tokio::spawn(async move {
            run_background_review(
                config,
                app_event_tx,
                base_snapshot_for_task,
                turn_context,
                prefer_fallback,
            )
            .await;
        });
    }

    pub(super) fn observe_auto_review_status(&mut self, agents: &[code_core::protocol::AgentInfo]) {
        let now = Instant::now();
        for agent in agents {
            if !Self::is_auto_review_agent(agent) {
                continue;
            }

            if let Some(progress) = agent.last_progress.as_deref()
                && progress.contains(SKIP_REVIEW_PROGRESS_SENTINEL) {
                    // Treat skipped review as benign: clear indicator and state, do not surface.
                    self.clear_auto_review_indicator();
                    self.background_review = None;
                    self.background_review_guard = None;
                    self.processed_auto_review_agents.insert(agent.id.clone());
                    continue;
                }

            if let Some(state) = self.background_review.as_mut() {
                state.last_seen = now;
                if state.agent_id.is_none() {
                    state.agent_id = Some(agent.id.clone());
                }
                if state.branch.is_empty()
                    && let Some(batch) = agent.batch_id.as_ref() {
                        state.branch = batch.clone();
                    }
            }

            let status = agent_status_from_str(agent.status.as_str());
            let is_terminal = matches!(
                status,
                AgentStatus::Completed | AgentStatus::Failed | AgentStatus::Cancelled
            );
            let phase = detect_auto_review_phase(agent.last_progress.as_deref());

            if matches!(status, AgentStatus::Running | AgentStatus::Pending) {
                let findings = self.auto_review_status.and_then(|s| s.findings);
                self.set_auto_review_indicator(
                    AutoReviewIndicatorStatus::Running,
                    findings,
                    phase,
                );
                continue;
            }

            if let Some(mut state) = self.auto_review_status {
                state.phase = phase;
                self.auto_review_status = Some(state);
                self.bottom_pane.set_auto_review_status(Some(AutoReviewFooterStatus {
                    status: state.status,
                    findings: state.findings,
                    phase,
                }));
            }

            if is_terminal && self.processed_auto_review_agents.contains(&agent.id) {
                continue;
            }
            if !is_terminal {
                continue;
            }

            let (worktree_path, branch, snapshot) = if let Some(state) = self.background_review.as_ref() {
                (
                    state.worktree_path.clone(),
                    state.branch.clone(),
                    state.snapshot.clone(),
                )
            } else {
                let Some(branch) = agent
                    .batch_id
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                else {
                    // We sometimes observe the same terminal auto-review agent multiple
                    // times (especially after cancellation/resend). If the background
                    // review state is already cleared and we cannot resolve the worktree,
                    // do not surface a misleading blank-path error.
                    self.processed_auto_review_agents.insert(agent.id.clone());
                    continue;
                };
                let Some(worktree_path) =
                    resolve_auto_review_worktree_path(&self.config.cwd, &branch)
                else {
                    self.processed_auto_review_agents.insert(agent.id.clone());
                    continue;
                };
                (worktree_path, branch, None)
            };

            let (has_findings, findings, summary) = Self::parse_agent_review_result(agent.result.as_deref());

            self.processed_auto_review_agents.insert(agent.id.clone());
            self.on_background_review_finished(BackgroundReviewFinishedEvent {
                worktree_path,
                branch,
                has_findings,
                findings,
                summary,
                error: agent.error.clone(),
                agent_id: Some(agent.id.clone()),
                snapshot,
            });
        }
    }

    /// Parse the auto-review agent result to derive findings count and a concise summary.
    /// Tries to deserialize `ReviewOutputEvent` JSON (direct or fenced). Falls back to heuristics.
    pub(super) fn parse_agent_review_result(raw: Option<&str>) -> (bool, usize, Option<String>) {
        let Some(text) = raw else { return (false, 0, None); };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return (false, 0, None);
        }

        #[derive(serde::Deserialize)]
        struct MultiRunReview {
            #[serde(flatten)]
            latest: ReviewOutputEvent,
            #[serde(default)]
            runs: Vec<ReviewOutputEvent>,
        }

        // Try multi-run JSON first (our /review output that preserves all passes).
        if let Ok(wrapper) = serde_json::from_str::<MultiRunReview>(trimmed) {
            let mut runs = wrapper.runs;
            if runs.is_empty() {
                runs.push(wrapper.latest);
            }
            return Self::review_result_from_runs(&runs);
        }

        // Try direct JSON first.
        if let Ok(output) = serde_json::from_str::<ReviewOutputEvent>(trimmed) {
            return Self::review_result_from_output(&output);
        }

        // Try to extract JSON from fenced code blocks.
        if let Some(start) = trimmed.find("```")
            && let Some((body, _)) = trimmed[start + 3..].split_once("```") {
                let candidate = body.trim_start_matches("json").trim();
                if let Ok(output) = serde_json::from_str::<ReviewOutputEvent>(candidate) {
                    return Self::review_result_from_output(&output);
                }
            }

        // Heuristic: treat plain text as summary; infer findings only when the text
        // explicitly mentions issues. Avoid false positives for skip/lock messages.
        let lowered = trimmed.to_ascii_lowercase();
        let clean_phrases = ["no issues", "no findings", "clean", "looks good", "nothing to fix"];
        let skip_phrases = ["already running", "another review", "skipping this", "skip this"];
        let issue_markers = ["issue", "issues", "finding", "findings", "bug", "bugs", "problem", "problems", "error", "errors"]; // keep broad but guarded

        if skip_phrases.iter().any(|p| lowered.contains(p)) {
            return (false, 0, Some(trimmed.to_string()));
        }

        if clean_phrases.iter().any(|p| lowered.contains(p)) {
            return (false, 0, Some(trimmed.to_string()));
        }

        let has_findings = issue_markers.iter().any(|p| lowered.contains(p));
        (has_findings, 0, Some(trimmed.to_string()))
    }

    pub(super) fn review_result_from_runs(outputs: &[ReviewOutputEvent]) -> (bool, usize, Option<String>) {
        if outputs.is_empty() {
            return (false, 0, None);
        }

        let Some(last) = outputs.last() else {
            return (false, 0, None);
        };
        let last_with_findings_idx = outputs
            .iter()
            .rposition(|o| !o.findings.is_empty());

        let (mut has_findings, mut findings_len, mut summary) = Self::review_result_from_output(last);

        if let Some(idx) = last_with_findings_idx {
            let with_findings = &outputs[idx];
            let (has, len, summary_with_findings) = Self::review_result_from_output(with_findings);
            has_findings |= has;
            findings_len = len;
            summary = summary_with_findings.or(summary);

            // If fixes cleared the final pass, note that in the summary.
            if last.findings.is_empty() {
                let tail = "Final pass reported no issues after auto-resolve.";
                summary = Some(match summary {
                    Some(existing) if existing.contains(tail) => existing,
                    Some(existing) => format!("{existing} \n{tail}"),
                    None => tail.to_string(),
                });
            }
        }

        (has_findings, findings_len, summary)
    }

    pub(super) fn review_result_from_output(output: &ReviewOutputEvent) -> (bool, usize, Option<String>) {
        let findings_len = output.findings.len();
        let has_findings = findings_len > 0;

        let mut summary_parts: Vec<String> = Vec::new();
        if !output.overall_explanation.trim().is_empty() {
            summary_parts.push(output.overall_explanation.trim().to_string());
        }
        if findings_len > 0 {
            let titles: Vec<String> = output
                .findings
                .iter()
                .filter_map(|f| {
                    let title = f.title.trim();
                    (!title.is_empty()).then_some(title.to_string())
                })
                .collect();
            if !titles.is_empty() {
                summary_parts.push(format!("Findings: {}", titles.join("; ")));
            }
        }

        let summary = if summary_parts.is_empty() {
            None
        } else {
            Some(summary_parts.join(" \n"))
        };

        (has_findings, findings_len, summary)
    }

    pub(super) fn set_auto_review_indicator(
        &mut self,
        status: AutoReviewIndicatorStatus,
        findings: Option<usize>,
        phase: AutoReviewPhase,
    ) {
        let state = AutoReviewStatus {
            status,
            findings,
            phase,
        };
        self.auto_review_status = Some(state);
        self.bottom_pane
            .set_auto_review_status(Some(AutoReviewFooterStatus {
                status,
                findings,
                phase,
            }));
        self.request_redraw();
    }

    pub(super) fn clear_auto_review_indicator(&mut self) {
        self.auto_review_status = None;
        self.bottom_pane.set_auto_review_status(None);
    }

    pub(super) fn last_assistant_cell_index(&self) -> Option<usize> {
        self.history_cells.iter().enumerate().rev().find_map(|(idx, cell)| {
            cell.as_any()
                .downcast_ref::<history_cell::AssistantMarkdownCell>()
                .map(|_| idx)
        })
    }

    pub(super) fn insert_auto_review_notice(
        &mut self,
        branch: &str,
        worktree_path: &std::path::Path,
        summary: Option<&str>,
        findings: usize,
    ) {
        let path_text = format!("{}", worktree_path.display());
        let has_path = !path_text.is_empty();

        let summary_text = summary.and_then(|text| {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_string())
        });

        let mut line = format!("Auto Review: {findings} issue(s) found");
        if let Some(summary_text) = summary_text {
            line.push_str(". ");
            line.push_str(&summary_text);
        } else {
            line.push_str(&format!(" in '{branch}'"));
        }
        if has_path {
            line.push(' ');
            line.push_str(&format!("Merge {path_text} to apply fixes."));
        }
        line.push_str(" [Ctrl+A] Show");

        let message_lines = vec![MessageLine {
            kind: MessageLineKind::Paragraph,
            spans: vec![InlineSpan {
                text: line,
                tone: TextTone::Default,
                emphasis: TextEmphasis::default(),
                entity: None,
            }],
        }];

        let state = PlainMessageState {
            id: HistoryId::ZERO,
            role: PlainMessageRole::System,
            kind: PlainMessageKind::Notice,
            header: Some(MessageHeader { label: "Auto Review".to_string(), badge: None }),
            lines: message_lines,
            metadata: None,
        };

        // Replace existing notice if present
        if let Some(notice) = self.auto_review_notice.clone()
            && let Some(idx) = self
                .history_cell_ids
                .iter()
                .position(|maybe| maybe.map(|id| id == notice.history_id).unwrap_or(false))
            {
                let cell = crate::history_cell::PlainHistoryCell::from_state(state);
                self.history_replace_at(idx, Box::new(cell));
                return;
            }

        // Insert after the indicator if present; otherwise after the assistant cell
        let base_key = self
            .last_assistant_cell_index()
            .and_then(|idx| self.cell_order_seq.get(idx).copied())
            .unwrap_or(OrderKey {
                req: 0,
                out: -1,
                seq: 0,
            });

        let insert_key = Self::order_key_successor(base_key);
        let pos = self.history_insert_plain_state_with_key(state, insert_key, "auto-review-notice");
        if let Some(Some(id)) = self.history_cell_ids.get(pos) {
            self.auto_review_notice = Some(AutoReviewNotice { history_id: *id });
        }
    }

    pub(crate) fn on_background_review_started(
        &mut self,
        worktree_path: std::path::PathBuf,
        branch: String,
        agent_id: Option<String>,
        snapshot: Option<String>,
    ) {
        if let Some(state) = self.background_review.as_mut() {
            state.worktree_path = worktree_path;
            state.branch = branch;
            state.agent_id = agent_id;
            state.snapshot = snapshot;
            state.last_seen = Instant::now();
        }
        if self.auto_review_status.is_none() {
            self.set_auto_review_indicator(
                AutoReviewIndicatorStatus::Running,
                None,
                AutoReviewPhase::Reviewing,
            );
        }
        // Ensure the main status spinner is cleared once the foreground turn ends;
        // background auto review should not keep the composer in a "running" state.
        self.bottom_pane.set_task_running(false);
        self.bottom_pane.update_status_text(String::new());
        self.auto_review_notice = None;
        self.request_redraw();
    }

    pub(crate) fn on_background_review_finished(
        &mut self,
        event: BackgroundReviewFinishedEvent,
    ) {
        let BackgroundReviewFinishedEvent {
            worktree_path,
            branch,
            has_findings,
            findings,
            summary,
            error,
            agent_id,
            snapshot,
        } = event;
        // Normalize zero-count "issues" so the indicator and developer notes stay
        // aligned with the overlay: if the parser could not produce a findings list,
        // treat the run as clean instead of "fixed".
        let mut has_findings = has_findings;
        if has_findings && findings == 0 {
            has_findings = false;
        }

        let inflight_base = self
            .background_review
            .as_ref()
            .and_then(|state| state.base.clone());
        let inflight_snapshot = snapshot.or_else(|| {
            self.background_review
                .as_ref()
                .and_then(|state| state.snapshot.clone())
        });
        // Clear flags up front so subsequent auto reviews can start even if this finishes with an error
        self.background_review = None;
        self.background_review_guard = None;
        release_background_lock(&agent_id);
        let mut developer_note: Option<String> = None;
        let snapshot_note = inflight_snapshot
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("Snapshot: {s} (review target)"))
            .unwrap_or_else(|| "Snapshot: (unknown)".to_string());
        let agent_note = agent_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|id| {
                let short = id.chars().take(8).collect::<String>();
                format!("Agent: #{short} (auto-review)")
            })
            .unwrap_or_else(|| "Agent: (unknown)".to_string());
        let summary_note = summary
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.replace('\n', " "))
            .map(|s| format!("Summary: {s}"));
        let errored = error.is_some();
        let indicator_status = if let Some(err) = error {
            developer_note = Some(format!(
                "[developer] Background auto-review failed.\n\nThis auto-review ran in an isolated git worktree and did not modify your current workspace.\n\nWorktree: '{branch}'\nWorktree path: {}\n{snapshot_note}\n{agent_note}\nError: {err}",
                worktree_path.display(),
            ));
            AutoReviewIndicatorStatus::Failed
        } else if has_findings {
            let mut note = format!(
                "[developer] Background auto-review completed and reported {findings} issue(s).\n\nA separate LLM ran /review (and may have run auto-resolve) in an isolated git worktree. Any proposed fixes live only in that worktree until you merge them.\n\nNext: Decide if the findings are genuine. If yes, Merge the worktree '{branch}' to apply the changes (or cherry-pick selectively). If not, do not merge.\n\nWorktree path: {}\n{snapshot_note}\n{agent_note}",
                worktree_path.display(),
            );
            if let Some(summary_note) = summary_note {
                note.push('\n');
                note.push_str(&summary_note);
            }
            developer_note = Some(note);
            AutoReviewIndicatorStatus::Fixed
        } else {
            AutoReviewIndicatorStatus::Clean
        };

        let findings_for_indicator =
            matches!(indicator_status, AutoReviewIndicatorStatus::Fixed).then_some(findings.max(1));
        let phase = self
            .auto_review_status
            .map(|s| s.phase)
            .unwrap_or(AutoReviewPhase::Reviewing);
        self.set_auto_review_indicator(indicator_status, findings_for_indicator, phase);
        if matches!(indicator_status, AutoReviewIndicatorStatus::Fixed) {
            self.insert_auto_review_notice(
                &branch,
                &worktree_path,
                summary.as_deref(),
                findings.max(1),
            );
        }

        if let Some(note) = developer_note {
            // Immediately inject as a developer message so the user sees it in the
            // transcript, even if tasks/streams are still running. Do not defer to
            // pending_agent_notes; that path can be lost if the session ends.
            self.submit_hidden_text_message_with_preface(String::new(), note);
        }

        // Auto review completion should never leave the composer spinner active.
        self.bottom_pane.set_task_running(false);
        self.bottom_pane.update_status_text(String::new());

        self.handle_auto_review_completion_state(
            has_findings,
            errored,
            inflight_base,
            inflight_snapshot,
        );
        self.maybe_resume_auto_after_review();
        self.request_redraw();
    }

    pub(super) fn handle_auto_review_completion_state(
        &mut self,
        has_findings: bool,
        errored: bool,
        inflight_base: Option<GhostCommit>,
        snapshot: Option<String>,
    ) {
        let was_skipped = !has_findings && !errored && snapshot.is_none();

        if !errored
            && let Some(id) = snapshot.as_ref() {
                self.auto_review_reviewed_marker = Some(GhostCommit::new(id.clone(), None));
                self.persist_auto_review_baseline_marker(id);
            }

        if was_skipped || errored {
            if let Some(base) = inflight_base {
                self.queue_skipped_auto_review(base);
            }
            return;
        }

        if has_findings {
            if let Some(range) = self.pending_auto_review_range.as_mut() {
                if range.defer_until_turn.is_none() {
                    range.defer_until_turn = Some(self.turn_sequence);
                }
            } else if let Some(base) = inflight_base {
                self.pending_auto_review_range = Some(PendingAutoReviewRange {
                    base,
                    defer_until_turn: Some(self.turn_sequence),
                });
            }
            return;
        }

        if let Some(pending) = self.pending_auto_review_range.take() {
            self.launch_background_review(Some(pending.base));
        }
    }

    pub(crate) fn set_review_auto_resolve_attempts(&mut self, attempts: u32) {
        use code_core::config_types::AutoResolveAttemptLimit;

        let Ok(limit) = AutoResolveAttemptLimit::try_new(attempts) else {
            tracing::warn!("Ignoring invalid auto-resolve attempt value: {}", attempts);
            return;
        };

        self.auto_resolve_attempts_baseline = limit.get();

        if self
            .config
            .auto_drive
            .auto_resolve_review_attempts
            .get()
            == limit.get()
        {
            return;
        }

        self.config.auto_drive.auto_resolve_review_attempts = limit;
        if let Some(state) = self.auto_resolve_state.as_mut() {
            state.max_attempts = limit.get();
            let allowed_total = state.max_attempts.saturating_add(1);
            if state.attempt >= allowed_total {
                self.auto_resolve_clear();
            }
        }

        let message = if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_auto_drive_settings(
                &home,
                &self.config.auto_drive,
                self.config.auto_drive_use_chat_model,
            ) {
                Ok(_) => {
                    tracing::info!(
                        "Persisted auto resolve attempt limit: {}",
                        limit.get()
                    );
                    format!("Max re-reviews set to {}.", limit.get())
                }
                Err(err) => {
                    tracing::warn!("Failed to persist auto resolve attempts: {err}");
                    format!(
                        "Max re-reviews set to {} for this session (failed to persist).",
                        limit.get()
                    )
                }
            }
        } else {
            tracing::warn!("Could not locate Codex home to persist auto resolve attempts");
            format!("Max re-reviews set to {} for this session.", limit.get())
        };

        self.bottom_pane.flash_footer_notice(message);
        self.update_review_settings_model_row();
        self.refresh_settings_overview_rows();
        self.request_redraw();
    }

    pub(crate) fn set_auto_review_followup_attempts(&mut self, attempts: u32) {
        use code_core::config_types::AutoResolveAttemptLimit;

        let Ok(limit) = AutoResolveAttemptLimit::try_new(attempts) else {
            tracing::warn!("Ignoring invalid auto-review follow-up value: {}", attempts);
            return;
        };

        if self
            .config
            .auto_drive
            .auto_review_followup_attempts
            .get()
            == limit.get()
        {
            return;
        }

        self.config.auto_drive.auto_review_followup_attempts = limit;

        if let Ok(home) = code_core::config::find_code_home() {
            match code_core::config::set_auto_drive_settings(
                &home,
                &self.config.auto_drive,
                self.config.auto_drive_use_chat_model,
            ) {
                Ok(_) => {
                    tracing::info!(
                        "Persisted auto-review follow-up limit: {}",
                        limit.get()
                    );
                    self.bottom_pane.flash_footer_notice(format!(
                        "Auto Review follow-ups set to {}.",
                        limit.get()
                    ));
                }
                Err(err) => {
                    tracing::warn!("Failed to persist auto-review follow-up attempts: {err}");
                    self.bottom_pane.flash_footer_notice(format!(
                        "Auto Review follow-ups set to {} for this session (failed to persist).",
                        limit.get()
                    ));
                }
            }
        }

        self.refresh_settings_overview_rows();
        self.update_review_settings_model_row();
        self.request_redraw();
    }

    pub(super) fn restore_auto_resolve_attempts_if_lost(&mut self) {
        if self.auto_resolve_attempts_baseline == 0 {
            return;
        }
        let current = self
            .config
            .auto_drive
            .auto_resolve_review_attempts
            .get();
        if current == 0
            && let Ok(limit) = code_core::config_types::AutoResolveAttemptLimit::try_new(
                self.auto_resolve_attempts_baseline,
            ) {
                self.config.auto_drive.auto_resolve_review_attempts = limit;
            }

        self.background_review = None;
    }

    pub(super) fn update_review_settings_model_row(&mut self) {
        if let Some(overlay) = self.settings.overlay.as_mut()
            && let Some(content) = overlay.review_content_mut() {
                content.update_review_model(
                    self.config.review_model.clone(),
                    self.config.review_model_reasoning_effort,
                );
                content.set_review_use_chat_model(self.config.review_use_chat_model);
                content.update_review_resolve_model(
                    self.config.review_resolve_model.clone(),
                    self.config.review_resolve_model_reasoning_effort,
                );
                content.set_review_resolve_use_chat_model(self.config.review_resolve_use_chat_model);
                content.update_auto_review_model(
                    self.config.auto_review_model.clone(),
                    self.config.auto_review_model_reasoning_effort,
                );
                content.set_auto_review_use_chat_model(self.config.auto_review_use_chat_model);
                content.update_auto_review_resolve_model(
                    self.config.auto_review_resolve_model.clone(),
                    self.config.auto_review_resolve_model_reasoning_effort,
                );
                content.set_auto_review_resolve_use_chat_model(
                    self.config.auto_review_resolve_use_chat_model,
                );
                content.set_review_followups(self.config.auto_drive.auto_resolve_review_attempts.get());
                content.set_auto_review_followups(
                    self.config.auto_drive.auto_review_followup_attempts.get(),
                );
            }
    }

    pub(super) fn update_planning_settings_model_row(&mut self) {
        if let Some(overlay) = self.settings.overlay.as_mut()
            && let Some(content) = overlay.planning_content_mut() {
                content.update_planning_model(
                    self.config.planning_model.clone(),
                    self.config.planning_model_reasoning_effort,
                );
                content.set_use_chat_model(self.config.planning_use_chat_model);
            }
    }

    pub(super) fn update_auto_drive_settings_model_row(&mut self) {
        if let Some(overlay) = self.settings.overlay.as_mut()
            && let Some(content) = overlay.auto_drive_content_mut() {
                content.update_model(
                    self.config.auto_drive.model.clone(),
                    self.config.auto_drive.model_reasoning_effort,
                );
                content.set_use_chat_model(
                    self.config.auto_drive_use_chat_model,
                    self.config.model.clone(),
                    self.config.model_reasoning_effort,
                );
            }
    }

    pub(crate) fn show_review_commit_loading(&mut self) {
        let loading_item = SelectionItem {
            name: "Loading recent commits…".to_string(),
            description: None,
            is_current: true,
            actions: Vec::new(),
        };
        let view = ListSelectionView::new(
            " Select a commit ".to_string(),
            Some("Fetching recent commits from git".to_string()),
            Some("Esc cancel".to_string()),
            vec![loading_item],
            self.app_event_tx.clone(),
            6,
        );
        self.bottom_pane.show_list_selection(
            "Select a commit".to_string(),
            None,
            None,
            view,
        );
    }

    pub(crate) fn present_review_commit_picker(&mut self, commits: Vec<CommitLogEntry>) {
        if commits.is_empty() {
            self.bottom_pane
                .flash_footer_notice("No recent commits found for review".to_string());
            self.request_redraw();
            return;
        }

        let auto_resolve = self.config.tui.review_auto_resolve;
        let mut items: Vec<SelectionItem> = Vec::with_capacity(commits.len());
        for entry in commits {
            let subject = entry.subject.trim().to_string();
            let sha = entry.sha.trim().to_string();
            if sha.is_empty() {
                continue;
            }
            let short_sha: String = sha.chars().take(7).collect();
            let title = if subject.is_empty() {
                short_sha.clone()
            } else {
                format!("{short_sha} — {subject}")
            };
            let prompt = if subject.is_empty() {
                format!(
                    "Review the code changes introduced by commit {sha}. Provide prioritized, actionable findings."
                )
            } else {
                format!(
                    "Review the code changes introduced by commit {sha} (\"{subject}\"). Provide prioritized, actionable findings."
                )
            };
            let hint = format!("commit {short_sha}");
            let preparation = format!("Preparing code review for commit {short_sha}");
            let prompt_closure = prompt.clone();
            let hint_closure = hint.clone();
            let prep_closure = preparation.clone();
            let target_closure = ReviewTarget::Commit {
                sha: sha.clone(),
                title: (!subject.is_empty()).then_some(subject.clone()),
            };
            let auto_flag = auto_resolve;
            items.push(SelectionItem {
                name: title,
                description: None,
                is_current: false,
                actions: vec![Box::new(move |tx: &crate::app_event_sender::AppEventSender| {
                    tx.send(crate::app_event::AppEvent::RunReviewWithScope {
                        target: target_closure.clone(),
                        prompt: prompt_closure.clone(),
                        hint: Some(hint_closure.clone()),
                        preparation_label: Some(prep_closure.clone()),
                        auto_resolve: auto_flag,
                    });
                })],
            });
        }

        if items.is_empty() {
            self.bottom_pane
                .flash_footer_notice("No recent commits found for review".to_string());
            self.request_redraw();
            return;
        }

        let view = ListSelectionView::new(
            " Select a commit ".to_string(),
            Some("Choose a commit to review".to_string()),
            Some("Enter select · Esc cancel".to_string()),
            items,
            self.app_event_tx.clone(),
            10,
        );

        self.bottom_pane.show_list_selection(
            "Select a commit to review".to_string(),
            None,
            None,
            view,
        );
    }

    pub(crate) fn show_review_branch_loading(&mut self) {
        let loading_item = SelectionItem {
            name: "Loading local branches…".to_string(),
            description: None,
            is_current: true,
            actions: Vec::new(),
        };
        let view = ListSelectionView::new(
            " Select a base branch ".to_string(),
            Some("Fetching local branches".to_string()),
            Some("Esc cancel".to_string()),
            vec![loading_item],
            self.app_event_tx.clone(),
            6,
        );
        self.bottom_pane.show_list_selection(
            "Select a base branch".to_string(),
            None,
            None,
            view,
        );
    }

    pub(crate) fn present_review_branch_picker(
        &mut self,
        current_branch: Option<String>,
        branches: Vec<String>,
    ) {
        let current_trimmed = current_branch.as_ref().map(|s| s.trim().to_string());
        let mut items: Vec<SelectionItem> = Vec::new();
        let auto_resolve = self.config.tui.review_auto_resolve;
        for branch in branches {
            let branch_trimmed = branch.trim();
            if branch_trimmed.is_empty() {
                continue;
            }
            if current_trimmed
                .as_ref()
                .is_some_and(|current| current == branch_trimmed)
            {
                continue;
            }

            let title = if let Some(current) = current_trimmed.as_ref() {
                format!("{current} → {branch_trimmed}")
            } else {
                format!("Compare against {branch_trimmed}")
            };

            let prompt = if let Some(current) = current_trimmed.as_ref() {
                format!(
                    "Review the code changes between the current branch '{current}' and '{branch_trimmed}'. Identify the intent of the changes in '{current}' and ensure no obvious gaps remain. Find all geniune bugs or regressions which need to be addressed before merging. Return ALL issues which need to be addressed, not just the first one you find."
                )
            } else {
                format!(
                    "Review the code changes that would merge into '{branch_trimmed}'. Identify bugs, regressions, risky patterns, and missing tests before merge."
                )
            };
            let hint = format!("against {branch_trimmed}");
            let preparation = format!("Preparing code review against {branch_trimmed}");
            let prompt_closure = prompt.clone();
            let hint_closure = hint.clone();
            let prep_closure = preparation.clone();
            let target_closure =
                ReviewTarget::BaseBranch { branch: branch_trimmed.to_string() };
            let auto_flag = auto_resolve;
            items.push(SelectionItem {
                name: title,
                description: None,
                is_current: false,
                actions: vec![Box::new(move |tx: &crate::app_event_sender::AppEventSender| {
                    tx.send(crate::app_event::AppEvent::RunReviewWithScope {
                        target: target_closure.clone(),
                        prompt: prompt_closure.clone(),
                        hint: Some(hint_closure.clone()),
                        preparation_label: Some(prep_closure.clone()),
                        auto_resolve: auto_flag,
                    });
                })],
            });
        }

        if items.is_empty() {
            self.bottom_pane
                .flash_footer_notice("No alternative branches found for review".to_string());
            self.request_redraw();
            return;
        }

        let subtitle = current_trimmed
            .as_ref()
            .map(|current| format!("Current branch: {current}"));

        let view = ListSelectionView::new(
            " Select a base branch ".to_string(),
            subtitle,
            Some("Enter select · Esc cancel".to_string()),
            items,
            self.app_event_tx.clone(),
            10,
        );

        self.bottom_pane.show_list_selection(
            "Compare against a branch".to_string(),
            None,
            None,
            view,
        );
    }

    /// Handle `/review [focus]` command by starting a dedicated review session.
    pub(crate) fn handle_review_command(&mut self, args: String) {
        if self.is_task_running() {
            self.history_push_plain_state(crate::history_cell::new_error_event(
                "`/review` — complete or cancel the current task before starting a new review.".to_string(),
            ));
            self.request_redraw();
            return;
        }

        let trimmed = args.trim();
        let auto_resolve = self.config.tui.review_auto_resolve;
        if trimmed.is_empty() {
            if Self::is_branch_worktree_path(&self.config.cwd)
                && let Some(git_root) =
                    code_core::git_info::resolve_root_git_project_for_trust(&self.config.cwd)
                {
                    let worktree_cwd = self.config.cwd.clone();
                    let tx = self.app_event_tx.clone();
                    let auto_flag = auto_resolve;
                    tokio::spawn(async move {
                        let branch_metadata =
                            code_core::git_worktree::load_branch_metadata(&worktree_cwd);
                        let metadata_base = branch_metadata.as_ref().and_then(|meta| {
                            meta.remote_ref.clone().or_else(|| {
                                if let (Some(remote_name), Some(base_branch)) =
                                    (meta.remote_name.clone(), meta.base_branch.clone())
                                {
                                    Some(format!("{remote_name}/{base_branch}"))
                                } else {
                                    None
                                }
                            })
                            .or_else(|| meta.base_branch.clone())
                        });
                        let default_branch = match metadata_base {
                            Some(value) => Some(value),
                            None => code_core::git_worktree::detect_default_branch(&git_root)
                                .await
                                .map(|name| name.trim().to_string())
                                .filter(|name| !name.is_empty()),
                        };
                        let current_branch = code_core::git_info::current_branch_name(&worktree_cwd)
                            .await
                            .map(|name| name.trim().to_string())
                            .filter(|name| !name.is_empty());

                        if let (Some(base_branch), Some(current_branch)) =
                            (default_branch, current_branch)
                            && base_branch != current_branch {
                                let prompt = format!(
                                    "Review the code changes between the current branch '{current_branch}' and '{base_branch}'. Identify the intent of the changes in '{current_branch}' and ensure no obvious gaps remain. Find all geniune bugs or regressions which need to be addressed before merging. Return ALL issues which need to be addressed, not just the first one you find."
                                );
                                let hint = Some(format!("against {base_branch}"));
                                let preparation_label =
                                    Some(format!("Preparing code review against {base_branch}"));
                                let target = ReviewTarget::Custom { instructions: prompt.clone() };
                                tx.send(crate::app_event::AppEvent::RunReviewWithScope {
                                    target,
                                    prompt,
                                    hint,
                                    preparation_label,
                                    auto_resolve: auto_flag,
                                });
                                return;
                            }

                        let prompt = "Review the current workspace changes and highlight bugs, regressions, risky patterns, and missing tests before merge.".to_string();
                        tx.send(crate::app_event::AppEvent::RunReviewWithScope {
                            target: ReviewTarget::Custom { instructions: prompt.clone() },
                            prompt,
                            hint: Some("current workspace changes".to_string()),
                            preparation_label: Some("Preparing code review request...".to_string()),
                            auto_resolve: auto_flag,
                        });
                    });
                    return;
                }

            let prompt = "Review the current workspace changes and highlight bugs, regressions, risky patterns, and missing tests before merge.".to_string();
            self.start_review_with_scope(
                ReviewTarget::Custom { instructions: prompt.clone() },
                prompt,
                Some("current workspace changes".to_string()),
                Some("Preparing code review request...".to_string()),
                auto_resolve,
            );
        } else {
            let value = trimmed.to_string();
            let preparation = format!("Preparing code review for {value}");
            self.start_review_with_scope(
                ReviewTarget::Custom { instructions: value.clone() },
                value.clone(),
                Some(value),
                Some(preparation),
                auto_resolve,
            );
        }
    }

    pub(crate) fn start_review_with_scope(
        &mut self,
        target: ReviewTarget,
        prompt: String,
        hint: Option<String>,
        preparation_label: Option<String>,
        auto_resolve: bool,
    ) {
        if auto_resolve {
            let max_re_reviews = self.configured_auto_resolve_re_reviews();
            self.auto_resolve_state = Some(AutoResolveState::new_with_limit(
                target.clone(),
                prompt.clone(),
                hint.clone().unwrap_or_default(),
                None,
                max_re_reviews,
            ));
        } else {
            self.auto_resolve_state = None;
        }

        self.begin_review(target, prompt, hint, preparation_label);
    }

    pub(super) fn begin_review(
        &mut self,
        target: ReviewTarget,
        prompt: String,
        hint: Option<String>,
        preparation_label: Option<String>,
    ) {
        self.active_review_hint = None;
        self.active_review_prompt = None;

        let trimmed_hint = hint.as_deref().unwrap_or("").trim();
        let preparation_notice = preparation_label.unwrap_or_else(|| {
            if trimmed_hint.is_empty() {
                "Preparing code review request...".to_string()
            } else {
                format!("Preparing code review for {trimmed_hint}")
            }
        });

        self.insert_background_event_early(preparation_notice);
        self.request_redraw();

        let review_request = ReviewRequest {
            target,
            prompt,
            user_facing_hint: hint,
        };
        match try_acquire_lock("review", &self.config.cwd) {
            Ok(Some(guard)) => {
                self.review_guard = Some(guard);
                self.submit_op(Op::Review { review_request });
            }
            Ok(None) => {
                self.push_background_tail("Review skipped: another review is already running.".to_string());
            }
            Err(err) => {
                self.push_background_tail(format!("Review skipped: could not acquire review lock ({err})"));
            }
        }
    }

    pub(super) fn is_review_flow_active(&self) -> bool {
        self.active_review_hint.is_some() || self.active_review_prompt.is_some()
    }

    pub(super) fn build_review_summary_cell(
        &self,
        hint: Option<&str>,
        prompt: Option<&str>,
        output: &ReviewOutputEvent,
    ) -> history_cell::AssistantMarkdownCell {
        let mut sections: Vec<String> = Vec::new();
        let title = match hint {
            Some(h) if !h.trim().is_empty() => {
                let trimmed = h.trim();
                format!("**Review summary — {trimmed}**")
            }
            _ => "**Review summary**".to_string(),
        };
        sections.push(title);

        if let Some(p) = prompt {
            let trimmed_prompt = p.trim();
            if !trimmed_prompt.is_empty() {
                sections.push(format!("**Prompt:** {trimmed_prompt}"));
            }
        }

        let explanation = output.overall_explanation.trim();
        if !explanation.is_empty() {
            sections.push(explanation.to_string());
        }
        if !output.findings.is_empty() {
            sections.push(format_review_findings_block(&output.findings, None).trim().to_string());
        }
        let correctness = output.overall_correctness.trim();
        if !correctness.is_empty() {
            sections.push(format!("**Overall correctness:** {correctness}"));
        }
        if output.overall_confidence_score > 0.0 {
            let score = output.overall_confidence_score;
            sections.push(format!("**Confidence score:** {score:.1}"));
        }
        if sections.len() == 1 {
            sections.push("No detailed findings were provided.".to_string());
        }

        let markdown = sections
            .into_iter()
            .map(|part| part.trim().to_string())
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");

        let state = AssistantMessageState {
            id: HistoryId::ZERO,
            stream_id: None,
            markdown,
            citations: Vec::new(),
            metadata: None,
            token_usage: None,
            mid_turn: false,
            created_at: SystemTime::now(),
        };
        history_cell::AssistantMarkdownCell::from_state(state, &self.config)
    }
}
