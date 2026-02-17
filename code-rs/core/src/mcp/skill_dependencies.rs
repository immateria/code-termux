use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;

use crate::config::Config;
use crate::config_types::ShellScriptStyle;
use crate::mcp_connection_manager::McpConnectionManager;
use crate::protocol::McpServerFailure;
use crate::protocol::McpServerFailurePhase;
use crate::skills::injection::SkillMcpDependency;

pub(crate) fn build_skill_mcp_dependency_warnings(
    deps: &[SkillMcpDependency],
    mcp: &McpConnectionManager,
    config: &Config,
    style: Option<ShellScriptStyle>,
) -> Vec<String> {
    if deps.is_empty() {
        return Vec::new();
    }

    let tools_by_server = mcp.list_tools_by_server();
    let disabled_by_server = mcp.list_disabled_tools_by_server();
    let failures = mcp.list_server_failures();

    let ServerMaps {
        server_name_by_lower,
        tool_names_by_server_lower,
        disabled_tool_names_by_server_lower,
    } = build_server_maps(&tools_by_server, &disabled_by_server, &failures);

    let StyleFilters {
        style_label,
        include_servers,
        exclude_servers,
    } = build_style_filters(config, style);

    let mut grouped: BTreeMap<(String, Option<String>), BTreeSet<String>> = BTreeMap::new();
    for dep in deps {
        let server = dep.server.trim().to_ascii_lowercase();
        if server.is_empty() {
            continue;
        }
        let tool = dep
            .tool
            .as_deref()
            .map(str::trim)
            .filter(|tool| !tool.is_empty())
            .map(str::to_ascii_lowercase);
        grouped
            .entry((server, tool))
            .or_default()
            .insert(dep.skill_name.trim().to_string());
    }

    let mut warnings = Vec::new();
    for ((server_lower, tool_lower), skills) in grouped {
        let skill_list = format_skill_list(&skills);
        let Some(server_name) = server_name_by_lower.get(&server_lower) else {
            warnings.push(missing_server_warning(
                &skill_list,
                server_lower.as_str(),
                style_label.as_deref(),
                &include_servers,
                &exclude_servers,
            ));
            continue;
        };

        match tool_lower.as_deref() {
            None => {
                if let Some(failure) = failures.get(server_name) {
                    warnings.push(server_failed_warning(&skill_list, server_name, failure));
                }
            }
            Some(tool_lower) => {
                if disabled_tool_names_by_server_lower
                    .get(&server_lower)
                    .is_some_and(|disabled| disabled.contains(tool_lower))
                {
                    warnings.push(disabled_tool_warning(
                        &skill_list,
                        server_name.as_str(),
                        tool_lower,
                    ));
                    continue;
                }

                if tool_names_by_server_lower
                    .get(&server_lower)
                    .is_some_and(|tools| tools.contains(tool_lower))
                {
                    continue;
                }

                if let Some(failure) = failures.get(server_name) {
                    warnings.push(server_failed_warning(&skill_list, server_name, failure));
                    continue;
                }

                warnings.push(missing_tool_warning(
                    &skill_list,
                    server_name.as_str(),
                    tool_lower,
                ));
            }
        }
    }

    warnings
}

struct ServerMaps {
    server_name_by_lower: HashMap<String, String>,
    tool_names_by_server_lower: HashMap<String, HashSet<String>>,
    disabled_tool_names_by_server_lower: HashMap<String, HashSet<String>>,
}

fn build_server_maps(
    tools_by_server: &HashMap<String, Vec<String>>,
    disabled_by_server: &HashMap<String, Vec<String>>,
    failures: &HashMap<String, McpServerFailure>,
) -> ServerMaps {
    let mut server_name_by_lower: HashMap<String, String> = HashMap::new();
    for name in tools_by_server
        .keys()
        .chain(disabled_by_server.keys())
        .chain(failures.keys())
    {
        server_name_by_lower
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| name.clone());
    }

    let mut tool_names_by_server_lower: HashMap<String, HashSet<String>> = HashMap::new();
    for (server, tools) in tools_by_server {
        let key = server.to_ascii_lowercase();
        let set = tool_names_by_server_lower.entry(key).or_default();
        for tool in tools {
            set.insert(tool.trim().to_ascii_lowercase());
        }
    }

    let mut disabled_tool_names_by_server_lower: HashMap<String, HashSet<String>> = HashMap::new();
    for (server, tools) in disabled_by_server {
        let key = server.to_ascii_lowercase();
        let set = disabled_tool_names_by_server_lower.entry(key).or_default();
        for tool in tools {
            set.insert(tool.trim().to_ascii_lowercase());
        }
    }

    ServerMaps {
        server_name_by_lower,
        tool_names_by_server_lower,
        disabled_tool_names_by_server_lower,
    }
}

struct StyleFilters {
    style_label: Option<String>,
    include_servers: HashSet<String>,
    exclude_servers: HashSet<String>,
}

fn build_style_filters(config: &Config, style: Option<ShellScriptStyle>) -> StyleFilters {
    let Some(style) = style else {
        return StyleFilters {
            style_label: None,
            include_servers: HashSet::new(),
            exclude_servers: HashSet::new(),
        };
    };

    let Some(profile) = config.shell_style_profiles.get(&style) else {
        return StyleFilters {
            style_label: Some(style.to_string()),
            include_servers: HashSet::new(),
            exclude_servers: HashSet::new(),
        };
    };

    let include_servers: HashSet<String> = profile
        .mcp_servers
        .include
        .iter()
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();

    let exclude_servers: HashSet<String> = profile
        .mcp_servers
        .exclude
        .iter()
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();

    StyleFilters {
        style_label: Some(style.to_string()),
        include_servers,
        exclude_servers,
    }
}

fn format_skill_list(skills: &BTreeSet<String>) -> String {
    let mut entries: Vec<String> = skills
        .iter()
        .filter_map(|name| {
            let trimmed = name.trim();
            (!trimmed.is_empty()).then(|| format!("`${trimmed}`"))
        })
        .collect();

    if entries.is_empty() {
        return "A skill".to_string();
    }

    entries.sort();
    if entries.len() <= 3 {
        return entries.join(", ");
    }

    let remaining = entries.len().saturating_sub(3);
    entries.truncate(3);
    format!("{} (and {remaining} more)", entries.join(", "))
}

fn missing_server_warning(
    skills: &str,
    server: &str,
    style_label: Option<&str>,
    include_servers: &HashSet<String>,
    exclude_servers: &HashSet<String>,
) -> String {
    let mut message = format!(
        "{skills} require MCP server `{server}`, but it is not enabled for this session."
    );

    if let Some(style_label) = style_label {
        if !include_servers.is_empty() && !include_servers.contains(server) {
            let mut included: Vec<&str> = include_servers.iter().map(String::as_str).collect();
            included.sort();
            message.push_str(&format!(
                " Active shell style `{style_label}` includes only: {}.",
                included.join(", ")
            ));
        } else if exclude_servers.contains(server) {
            message.push_str(&format!(
                " Active shell style `{style_label}` explicitly excludes it."
            ));
        }
    }

    message.push_str(" Enable it in Settings -> MCP, or update your shell style MCP filters.");
    message
}

fn disabled_tool_warning(skills: &str, server: &str, tool: &str) -> String {
    format!(
        "{skills} require MCP tool `{server}/{tool}`, but it is currently disabled. Enable it in Settings -> MCP (or remove it from `disabled_tools`)."
    )
}

fn missing_tool_warning(skills: &str, server: &str, tool: &str) -> String {
    format!(
        "{skills} require MCP tool `{server}/{tool}`, but it was not reported by the server. Try refreshing tools/status in Settings -> MCP, or verify the tool exists."
    )
}

fn server_failed_warning(skills: &str, server: &str, failure: &McpServerFailure) -> String {
    let phase = match failure.phase {
        McpServerFailurePhase::Start => "start",
        McpServerFailurePhase::ListTools => "list tools",
    };
    format!(
        "{skills} require MCP server `{server}`, but it failed to {phase}: {}. Open Settings -> MCP for details (or run `/mcp status`).",
        failure.message
    )
}

