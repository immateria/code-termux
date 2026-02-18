use super::*;

pub(super) struct McpTurnAllowGuard {
    sess: Arc<Session>,
    turn_id: String,
}

impl McpTurnAllowGuard {
    pub(super) fn new(sess: Arc<Session>, turn_id: String) -> Self {
        sess.set_mcp_turn_allow_servers(turn_id.as_str(), HashSet::new());
        Self { sess, turn_id }
    }
}

impl Drop for McpTurnAllowGuard {
    fn drop(&mut self) {
        self.sess
            .clear_mcp_turn_allow_servers(self.turn_id.as_str());
    }
}

async fn send_warning_event(sess: &Session, turn_id: &str, message: String) {
    let event = sess.make_event(
        turn_id,
        EventMsg::Warning(crate::protocol::WarningEvent { message }),
    );
    sess.send_event(event).await;
}

fn mcp_access_prompt_options(
    style_active: bool,
) -> Vec<code_protocol::request_user_input::RequestUserInputQuestionOption> {
    use code_protocol::request_user_input::RequestUserInputQuestionOption;

    let mut options: Vec<RequestUserInputQuestionOption> = Vec::new();
    options.push(RequestUserInputQuestionOption {
        label: "Allow once".to_string(),
        description: "Allow this server for the current turn only.".to_string(),
    });
    options.push(RequestUserInputQuestionOption {
        label: "Allow for session".to_string(),
        description: "Allow this server until you restart the app.".to_string(),
    });
    if style_active {
        options.push(RequestUserInputQuestionOption {
            label: "Allow and persist for style".to_string(),
            description: "Update the active shell style MCP include/exclude filters.".to_string(),
        });
    }
    options.push(RequestUserInputQuestionOption {
        label: "Deny".to_string(),
        description: "Keep it blocked for now.".to_string(),
    });
    options.push(RequestUserInputQuestionOption {
        label: "Deny for session (don't ask again)".to_string(),
        description: "Keep it blocked for this session and skip future prompts.".to_string(),
    });
    if style_active {
        options.push(RequestUserInputQuestionOption {
            label: "Deny and persist for style".to_string(),
            description: "Add this server to the active shell style MCP exclude list.".to_string(),
        });
    }
    options.push(RequestUserInputQuestionOption {
        label: "Cancel".to_string(),
        description: "Abort this prompt without changing settings.".to_string(),
    });
    options
}

pub(super) fn mcp_access_question(
    question: String,
    style_active: bool,
) -> code_protocol::request_user_input::RequestUserInputQuestion {
    code_protocol::request_user_input::RequestUserInputQuestion {
        id: "mcp_access".to_string(),
        header: "MCP access".to_string(),
        question,
        is_other: false,
        is_secret: false,
        options: Some(mcp_access_prompt_options(style_active)),
    }
}

async fn persist_style_mcp_server_filters(
    sess: &Session,
    turn_id: &str,
    style: crate::config_types::ShellScriptStyle,
    style_label: Option<String>,
    include: HashSet<String>,
    exclude: HashSet<String>,
) {
    sess.set_mcp_style_filters(Some(style), style_label, include.clone(), exclude.clone());

    let mut include_vec: Vec<String> = include.into_iter().collect();
    let mut exclude_vec: Vec<String> = exclude.into_iter().collect();
    include_vec.sort();
    exclude_vec.sort();

    let code_home = sess.client.code_home().to_path_buf();
    let persisted = tokio::task::spawn_blocking(move || {
        crate::config::set_shell_style_profile_mcp_servers(
            &code_home,
            style,
            include_vec.as_slice(),
            exclude_vec.as_slice(),
        )
    })
    .await;

    match persisted {
        Ok(Ok(_)) => {}
        Ok(Err(err)) => {
            send_warning_event(
                sess,
                turn_id,
                format!("Failed to persist MCP style filters for `{style}`: {err:#}"),
            )
            .await;
        }
        Err(err) => {
            send_warning_event(
                sess,
                turn_id,
                format!("Failed to persist MCP style filters for `{style}`: {err:#}"),
            )
            .await;
        }
    }
}

pub(super) async fn apply_mcp_access_selection(
    sess: &Session,
    turn_id: &str,
    server_id: &crate::mcp::ids::McpServerId,
    server_name: &str,
    mcp_access: &crate::codex::McpAccessState,
    selection: &str,
) {
    match selection {
        "Allow once" => {
            sess.allow_mcp_server_for_turn(turn_id, server_id.as_str());
        }
        "Allow for session" => {
            sess.allow_mcp_server_for_session(server_id.as_str());
        }
        "Allow and persist for style" => {
            let Some(style) = mcp_access.style else {
                return;
            };
            let mut include = mcp_access.style_include_servers.clone();
            let mut exclude = mcp_access.style_exclude_servers.clone();
            exclude.remove(server_id.as_str());
            if !include.is_empty() {
                include.insert(server_id.as_str().to_string());
            }
            persist_style_mcp_server_filters(
                sess,
                turn_id,
                style,
                mcp_access.style_label.clone(),
                include,
                exclude,
            )
            .await;
        }
        "Deny for session (don't ask again)" => {
            sess.deny_mcp_server_for_session(server_id.as_str());
        }
        "Deny and persist for style" => {
            let Some(style) = mcp_access.style else {
                return;
            };
            let mut include = mcp_access.style_include_servers.clone();
            let mut exclude = mcp_access.style_exclude_servers.clone();
            exclude.insert(server_id.as_str().to_string());
            include.remove(server_id.as_str());
            persist_style_mcp_server_filters(
                sess,
                turn_id,
                style,
                mcp_access.style_label.clone(),
                include,
                exclude,
            )
            .await;
        }
        "Deny" | "Cancel" => {}
        other => {
            send_warning_event(
                sess,
                turn_id,
                format!(
                    "Unknown MCP access selection `{other}`; keeping server `{server_name}` blocked."
                ),
            )
            .await;
        }
    }
}

pub(super) async fn ensure_skill_mcp_access_for_turn(
    sess: &Arc<Session>,
    turn_id: &str,
    config: &crate::config::Config,
    deps: &[crate::skills::injection::SkillMcpDependency],
) {
    use code_protocol::request_user_input::RequestUserInputEvent;

    if deps.is_empty() {
        return;
    }

    let mut config_server_by_id: HashMap<
        crate::mcp::ids::McpServerId,
        (String, crate::config_types::McpServerConfig),
    > = HashMap::new();
    for (server_name, cfg) in &config.mcp_servers {
        let Some(server_id) = crate::mcp::ids::McpServerId::parse(server_name) else {
            continue;
        };
        config_server_by_id
            .entry(server_id)
            .or_insert_with(|| (server_name.clone(), cfg.clone()));
    }

    let mut required_by_server: std::collections::BTreeMap<
        crate::mcp::ids::McpServerId,
        std::collections::BTreeSet<String>,
    > = std::collections::BTreeMap::new();
    for dep in deps {
        let Some(server) = crate::mcp::ids::McpServerId::parse(dep.server.as_str()) else {
            continue;
        };
        if !config_server_by_id.contains_key(&server) {
            continue;
        }
        let skill = dep.skill_name.trim();
        if skill.is_empty() {
            continue;
        }
        required_by_server
            .entry(server)
            .or_default()
            .insert(skill.to_string());
    }

    if required_by_server.is_empty() {
        return;
    }

    for (server_id, skills) in required_by_server {
        let (server_name, cfg) = match config_server_by_id.get(&server_id) {
            Some((name, cfg)) => (name.clone(), cfg.clone()),
            None => continue,
        };

        let mcp_access = sess.mcp_access_snapshot();
        if mcp_access.session_deny_servers.contains(server_id.as_str()) {
            continue;
        }

        if crate::mcp::policy::server_access_for_turn(&mcp_access, turn_id, &server_id).is_allowed()
        {
            if let Err(err) = sess
                .mcp_connection_manager
                .ensure_server_started(&server_name, &cfg)
                .await
            {
                send_warning_event(
                    sess.as_ref(),
                    turn_id,
                    format!("Failed to start MCP server `{server_name}` required by skills: {err:#}"),
                )
                .await;
            }
            continue;
        }

        let style_label = mcp_access.style_label.clone();
        let skill_list = skills
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ");

        let mut question_text = format!(
            "{skill_list} require MCP server `{server_name}`, but it is blocked by your current MCP filters."
        );
        if let Some(style_label) = style_label.as_deref() {
            question_text.push_str(&format!(" Active shell style: `{style_label}`."));
        }
        question_text.push_str("\n\nHow do you want to proceed?");

        let call_id = format!("mcp_access:{turn_id}:{}", server_id.as_str());
        let rx_response = match sess.register_pending_user_input(turn_id.to_string()) {
            Ok(rx) => rx,
            Err(err) => {
                send_warning_event(sess.as_ref(), turn_id, err).await;
                continue;
            }
        };

        sess.send_event(sess.make_event(
            turn_id,
            EventMsg::RequestUserInput(RequestUserInputEvent {
                call_id: call_id.clone(),
                turn_id: turn_id.to_string(),
                questions: vec![mcp_access_question(
                    question_text,
                    mcp_access.style.is_some(),
                )],
            }),
        ))
        .await;

        let response = match rx_response.await {
            Ok(response) => response,
            Err(_) => continue,
        };
        let selection = response
            .answers
            .get("mcp_access")
            .and_then(|answer| answer.answers.first())
            .map(|value| value.trim().to_string())
            .unwrap_or_default();

        apply_mcp_access_selection(
            sess.as_ref(),
            turn_id,
            &server_id,
            server_name.as_str(),
            &mcp_access,
            selection.as_str(),
        )
        .await;

        let mcp_access_after = sess.mcp_access_snapshot();
        if crate::mcp::policy::server_access_for_turn(&mcp_access_after, turn_id, &server_id)
            .is_allowed()
            && let Err(err) = sess
                .mcp_connection_manager
                .ensure_server_started(&server_name, &cfg)
                .await
        {
            send_warning_event(
                sess.as_ref(),
                turn_id,
                format!(
                    "Failed to start MCP server `{server_name}` after allowing it: {err:#}"
                ),
            )
            .await;
        }
    }
}

pub(super) async fn preflight_turn_skill_input(
    sess: &Arc<Session>,
    turn_context: &Arc<TurnContext>,
    turn_id: &str,
    initial_user_item: Option<&ResponseItem>,
    pending_input_tail: &[ResponseItem],
    input: &mut Vec<ResponseItem>,
) {
    let mut mention_messages: Vec<String> = Vec::new();
    for item in initial_user_item.iter().copied().chain(pending_input_tail.iter()) {
        if let ResponseItem::Message { role, content, .. } = item
            && role == "user"
        {
            for entry in content {
                if let ContentItem::InputText { text } = entry {
                    mention_messages.push(text.clone());
                }
            }
        }
    }

    if mention_messages.is_empty() {
        return;
    }

    let mention_outcome = {
        let skills = sess.skills.read().await;
        if skills.is_empty() {
            None
        } else {
            Some(crate::skills::injection::collect_explicit_skill_mentions(
                mention_messages.as_slice(),
                skills.as_slice(),
            ))
        }
    };

    let Some(mention_outcome) = mention_outcome else {
        return;
    };

    for warning in mention_outcome.warnings {
        send_warning_event(sess.as_ref(), turn_id, warning).await;
    }

    let injections = crate::skills::injection::build_skill_injections(&mention_outcome.mentioned).await;
    for warning in injections.warnings {
        send_warning_event(sess.as_ref(), turn_id, warning).await;
    }

    let config_snapshot = turn_context.client.config();
    ensure_skill_mcp_access_for_turn(
        sess,
        turn_id,
        config_snapshot.as_ref(),
        injections.mcp_dependencies.as_slice(),
    )
    .await;

    let mcp_access = sess.mcp_access_snapshot();
    for warning in crate::mcp::skill_dependencies::build_skill_mcp_dependency_warnings(
        injections.mcp_dependencies.as_slice(),
        &sess.mcp_connection_manager,
        mcp_access.style_label.as_deref(),
        &mcp_access.style_include_servers,
        &mcp_access.style_exclude_servers,
    ) {
        send_warning_event(sess.as_ref(), turn_id, warning).await;
    }

    if !injections.items.is_empty() {
        input.extend(injections.items);
    }
}
