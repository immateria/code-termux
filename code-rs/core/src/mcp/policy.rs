use std::collections::HashMap;

use crate::codex::McpAccessState;
use crate::mcp::ids::McpServerId;
use crate::mcp_connection_manager::McpConnectionManager;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum McpServerAccessDecision {
    Allowed,
    DeniedSession,
    DeniedStyleExclude,
    DeniedStyleIncludeOnly,
}

impl McpServerAccessDecision {
    pub(crate) fn is_allowed(self) -> bool {
        matches!(self, Self::Allowed)
    }

    pub(crate) fn is_session_denied(self) -> bool {
        matches!(self, Self::DeniedSession)
    }
}

pub(crate) fn server_access_for_turn(
    mcp_access: &McpAccessState,
    turn_id: &str,
    server: &McpServerId,
) -> McpServerAccessDecision {
    let server = server.as_str();
    if mcp_access.session_deny_servers.contains(server) {
        return McpServerAccessDecision::DeniedSession;
    }
    if mcp_access.turn_id.as_deref() == Some(turn_id)
        && mcp_access.turn_allow_servers.contains(server)
    {
        return McpServerAccessDecision::Allowed;
    }
    if mcp_access.session_allow_servers.contains(server) {
        return McpServerAccessDecision::Allowed;
    }
    if mcp_access.style_exclude_servers.contains(server) {
        return McpServerAccessDecision::DeniedStyleExclude;
    }
    if !mcp_access.style_include_servers.is_empty()
        && !mcp_access.style_include_servers.contains(server)
    {
        return McpServerAccessDecision::DeniedStyleIncludeOnly;
    }
    McpServerAccessDecision::Allowed
}

pub(crate) fn filter_tools_for_turn(
    mcp: &McpConnectionManager,
    mcp_access: &McpAccessState,
    turn_id: &str,
) -> HashMap<String, mcp_types::Tool> {
    let mut out: HashMap<String, mcp_types::Tool> = HashMap::new();
    for (qualified_name, server_name, tool) in mcp.list_all_tools_with_server_names() {
        let Some(server) = McpServerId::parse(server_name.as_str()) else {
            continue;
        };
        if !server_access_for_turn(mcp_access, turn_id, &server).is_allowed() {
            continue;
        }
        out.insert(qualified_name, tool);
    }
    out
}
