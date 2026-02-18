use crate::config_loader::{load_config_as_toml_blocking, LoaderOverrides};
use crate::config_types::{
    AutoDriveContinueMode,
    AutoDriveSettings,
    CachedTerminalBackground,
    LimitsLayoutMode,
    McpServerConfig,
    McpServerTransportConfig,
    ReasoningEffort,
    ShellConfig,
    ShellScriptStyle,
    ThemeColors,
    ThemeName,
};
use crate::protocol::{ApprovedCommandMatchKind, AskForApproval};
use code_protocol::config_types::SandboxMode;
use dirs::home_dir;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::NamedTempFile;
use toml::Value as TomlValue;
use toml_edit::Array as TomlArray;
use toml_edit::ArrayOfTables as TomlArrayOfTables;
use toml_edit::DocumentMut;
use toml_edit::Item as TomlItem;
use toml_edit::Table as TomlTable;
use which::which;

use super::CONFIG_TOML_FILE;

pub fn load_config_as_toml(code_home: &Path) -> std::io::Result<TomlValue> {
    load_config_as_toml_blocking(code_home, LoaderOverrides::default())
}

pub fn load_global_mcp_servers(
    code_home: &Path,
) -> std::io::Result<BTreeMap<String, McpServerConfig>> {
    let root_value = load_config_as_toml(code_home)?;
    let Some(servers_value) = root_value.get("mcp_servers") else {
        return Ok(BTreeMap::new());
    };

    let servers: BTreeMap<String, McpServerConfig> = servers_value
        .clone()
        .try_into()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    for (name, cfg) in &servers {
        if let McpServerTransportConfig::Stdio { command, .. } = &cfg.transport {
            let command_looks_like_path = {
                let path = Path::new(command);
                path.components().count() > 1 || path.is_absolute()
            };
            if !command_looks_like_path && which(command).is_err() {
                let msg = format!(
                    "MCP server `{name}` command `{command}` not found on PATH. If the server is an npm package, set command = \"npx\" and keep the package name in args."
                );
                return Err(std::io::Error::new(ErrorKind::NotFound, msg));
            }
        }
    }

    Ok(servers)
}

pub fn write_global_mcp_servers(
    code_home: &Path,
    servers: &BTreeMap<String, McpServerConfig>,
) -> std::io::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents
            .parse::<DocumentMut>()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e),
    };

    doc.as_table_mut().remove("mcp_servers");

    if !servers.is_empty() {
        let mut table = TomlTable::new();
        table.set_implicit(true);
        doc["mcp_servers"] = TomlItem::Table(table);

        for (name, config) in servers {
            let mut entry = TomlTable::new();
            entry.set_implicit(false);
            match &config.transport {
                McpServerTransportConfig::Stdio { command, args, env } => {
                    entry["command"] = toml_edit::value(command.clone());

                    if !args.is_empty() {
                        let mut args_array = TomlArray::new();
                        for arg in args {
                            args_array.push(arg.clone());
                        }
                        entry["args"] = TomlItem::Value(args_array.into());
                    }

                    if let Some(env) = env
                        && !env.is_empty()
                    {
                        let mut env_table = TomlTable::new();
                        env_table.set_implicit(false);
                        let mut pairs: Vec<_> = env.iter().collect();
                        pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                        for (key, value) in pairs {
                            env_table.insert(key, toml_edit::value(value.clone()));
                        }
                        entry["env"] = TomlItem::Table(env_table);
                    }
                }
                McpServerTransportConfig::StreamableHttp {
                    url,
                    bearer_token,
                    bearer_token_env_var,
                    http_headers,
                    env_http_headers,
                } => {
                    entry["url"] = toml_edit::value(url.clone());
                    if let Some(token) = bearer_token {
                        entry["bearer_token"] = toml_edit::value(token.clone());
                    }
                    if let Some(env_var) = bearer_token_env_var {
                        entry["bearer_token_env_var"] = toml_edit::value(env_var.clone());
                    }

                    if let Some(http_headers) = http_headers
                        && !http_headers.is_empty()
                    {
                        let mut headers_table = TomlTable::new();
                        headers_table.set_implicit(false);
                        let mut pairs: Vec<_> = http_headers.iter().collect();
                        pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                        for (key, value) in pairs {
                            headers_table.insert(key, toml_edit::value(value.clone()));
                        }
                        entry["http_headers"] = TomlItem::Table(headers_table);
                    }

                    if let Some(env_http_headers) = env_http_headers
                        && !env_http_headers.is_empty()
                    {
                        let mut headers_table = TomlTable::new();
                        headers_table.set_implicit(false);
                        let mut pairs: Vec<_> = env_http_headers.iter().collect();
                        pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                        for (key, value) in pairs {
                            headers_table.insert(key, toml_edit::value(value.clone()));
                        }
                        entry["env_http_headers"] = TomlItem::Table(headers_table);
                    }
                }
            }

            if let Some(timeout) = config.startup_timeout_sec {
                entry["startup_timeout_sec"] = toml_edit::value(timeout.as_secs_f64());
            }

            if let Some(timeout) = config.tool_timeout_sec {
                entry["tool_timeout_sec"] = toml_edit::value(timeout.as_secs_f64());
            }

            doc["mcp_servers"][name.as_str()] = TomlItem::Table(entry);
        }
    }

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path).map_err(|err| err.error)?;

    Ok(())
}

/// Persist the currently active model selection back to `config.toml` so that it
/// becomes the default for future sessions.
pub async fn persist_model_selection(
    code_home: &Path,
    profile: Option<&str>,
    model: &str,
    effort: Option<ReasoningEffort>,
    preferred_effort: Option<ReasoningEffort>,
) -> anyhow::Result<()> {
    use tokio::fs;

    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let existing = match fs::read_to_string(&read_path).await {
        Ok(raw) => Some(raw),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };

    let mut doc = match existing {
        Some(raw) if raw.trim().is_empty() => DocumentMut::new(),
        Some(raw) => raw
            .parse::<DocumentMut>()
            .map_err(|e| anyhow::anyhow!("failed to parse config.toml: {e}"))?,
        None => DocumentMut::new(),
    };

    {
        let root = doc.as_table_mut();
        if let Some(profile_name) = profile {
            let profiles_item = root
                .entry("profiles")
                .or_insert_with(|| {
                    let mut table = TomlTable::new();
                    table.set_implicit(true);
                    TomlItem::Table(table)
                });

            let Some(profiles_table) = profiles_item.as_table_mut() else {
                return Err(anyhow::anyhow!("profiles table should be a table"));
            };

            let profile_item = profiles_table
                .entry(profile_name)
                .or_insert_with(|| {
                    let mut table = TomlTable::new();
                    table.set_implicit(false);
                    TomlItem::Table(table)
                });

            let Some(profile_table) = profile_item.as_table_mut() else {
                return Err(anyhow::anyhow!("profile entry should be a table"));
            };

            profile_table["model"] = toml_edit::value(model.to_string());

            if let Some(effort) = effort {
                profile_table["model_reasoning_effort"] =
                    toml_edit::value(effort.to_string());
            } else {
                profile_table.remove("model_reasoning_effort");
            }

            if let Some(preferred) = preferred_effort {
                profile_table["preferred_model_reasoning_effort"] =
                    toml_edit::value(preferred.to_string());
            } else {
                profile_table.remove("preferred_model_reasoning_effort");
            }
        } else {
            root["model"] = toml_edit::value(model.to_string());
            match effort {
                Some(effort) => {
                    root["model_reasoning_effort"] =
                        toml_edit::value(effort.to_string());
                }
                None => {
                    root.remove("model_reasoning_effort");
                }
            }

            match preferred_effort {
                Some(preferred) => {
                    root["preferred_model_reasoning_effort"] =
                        toml_edit::value(preferred.to_string());
                }
                None => {
                    root.remove("preferred_model_reasoning_effort");
                }
            }
        }
    }

    fs::create_dir_all(code_home).await?;
    let tmp_path = config_path.with_extension("tmp");
    fs::write(&tmp_path, doc.to_string()).await?;
    fs::rename(&tmp_path, &config_path).await?;

    Ok(())
}

/// Persist the shell setting back to `config.toml`.
pub async fn persist_shell(code_home: &Path, shell: Option<&ShellConfig>) -> anyhow::Result<()> {
    use tokio::fs;

    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let existing = match fs::read_to_string(&read_path).await {
        Ok(raw) => Some(raw),
        Err(err) if err.kind() == ErrorKind::NotFound => None,
        Err(err) => return Err(err.into()),
    };

    let mut doc = match existing {
        Some(raw) if raw.trim().is_empty() => DocumentMut::new(),
        Some(raw) => raw
            .parse::<DocumentMut>()
            .map_err(|e| anyhow::anyhow!("failed to parse config.toml: {e}"))?,
        None => DocumentMut::new(),
    };

    {
        let root = doc.as_table_mut();
        if let Some(shell_config) = shell {
            let shell_table = root
                .entry("shell")
                .or_insert_with(|| {
                    let mut table = TomlTable::new();
                    table.set_implicit(false);
                    TomlItem::Table(table)
                });

            let Some(shell_table) = shell_table.as_table_mut() else {
                return Err(anyhow::anyhow!("shell entry should be a table"));
            };

            shell_table["path"] = toml_edit::value(shell_config.path.clone());
            if !shell_config.args.is_empty() {
                let mut args_array = toml_edit::Array::new();
                for arg in &shell_config.args {
                    args_array.push(arg.as_str());
                }
                shell_table["args"] = toml_edit::value(args_array);
            } else {
                shell_table.remove("args");
            }
            if let Some(style) = shell_config.script_style {
                shell_table["script_style"] = toml_edit::value(style.to_string());
            } else {
                shell_table.remove("script_style");
            }
        } else {
            root.remove("shell");
        }
    }

    fs::create_dir_all(code_home).await?;
    let tmp_path = config_path.with_extension("tmp");
    fs::write(&tmp_path, doc.to_string()).await?;
    fs::rename(&tmp_path, &config_path).await?;

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellStyleSkillMode {
    Inherit,
    Enabled,
    Disabled,
}

/// Update the membership of `skill_name` in a shell style profile.
///
/// - `Enabled`: add to `shell_style_profiles.<style>.skills` and remove from
///   `disabled_skills`.
/// - `Disabled`: add to `disabled_skills` and remove from `skills`.
/// - `Inherit`: remove from both lists.
///
/// Returns `true` when `config.toml` changed.
pub fn set_shell_style_profile_skill_mode(
    code_home: &Path,
    style: ShellScriptStyle,
    skill_name: &str,
    mode: ShellStyleSkillMode,
) -> anyhow::Result<bool> {
    let normalized_skill = skill_name.trim();
    if normalized_skill.is_empty() {
        return Err(anyhow::anyhow!("skill name cannot be empty"));
    }

    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    let style_key = style.to_string();
    let mut changed = false;

    {
        let root = doc.as_table_mut();
        match root.get("shell_style_profiles") {
            Some(item) => {
                if item.as_table().is_none() {
                    return Err(anyhow::anyhow!(
                        "`shell_style_profiles` must be a TOML table"
                    ));
                }
            }
            None => {
                if matches!(mode, ShellStyleSkillMode::Inherit) {
                    return Ok(false);
                }
                let mut table = TomlTable::new();
                table.set_implicit(true);
                root.insert("shell_style_profiles", TomlItem::Table(table));
                changed = true;
            }
        }
    }

    {
        let root = doc.as_table_mut();
        let profiles_table = root
            .get_mut("shell_style_profiles")
            .and_then(|item| item.as_table_mut())
            .ok_or_else(|| anyhow::anyhow!("failed to prepare shell_style_profiles table"))?;

        let mut resolved_style_key = find_shell_style_profile_key(profiles_table, style)?;
        match resolved_style_key.as_deref() {
            Some(existing_key) => {
                if profiles_table
                    .get(existing_key)
                    .and_then(|item| item.as_table())
                    .is_none()
                {
                    return Err(anyhow::anyhow!(
                        "`shell_style_profiles.{existing_key}` must be a TOML table"
                    ));
                }
            }
            None => {
                if matches!(mode, ShellStyleSkillMode::Inherit) {
                    return Ok(changed);
                }
                let mut style_table = TomlTable::new();
                style_table.set_implicit(false);
                profiles_table.insert(style_key.as_str(), TomlItem::Table(style_table));
                resolved_style_key = Some(style_key.clone());
                changed = true;
            }
        }
        let resolved_style_key = resolved_style_key
            .ok_or_else(|| anyhow::anyhow!("failed to resolve shell style profile key"))?;

        let style_table = profiles_table
            .get_mut(resolved_style_key.as_str())
            .and_then(|item| item.as_table_mut())
            .ok_or_else(|| anyhow::anyhow!("failed to prepare shell style profile table"))?;

        let mut skills = read_string_array(style_table, "skills")?;
        let mut disabled = read_string_array(style_table, "disabled_skills")?;
        let normalized_target = normalize_skill_name(normalized_skill);

        let removed_from_skills = remove_skill_name(&mut skills, &normalized_target);
        let removed_from_disabled = remove_skill_name(&mut disabled, &normalized_target);
        changed |= removed_from_skills || removed_from_disabled;

        match mode {
            ShellStyleSkillMode::Inherit => {}
            ShellStyleSkillMode::Enabled => {
                if push_unique_skill_name(&mut skills, normalized_skill) {
                    changed = true;
                }
            }
            ShellStyleSkillMode::Disabled => {
                if push_unique_skill_name(&mut disabled, normalized_skill) {
                    changed = true;
                }
            }
        }

        changed |= write_string_array(style_table, "skills", &skills)?;
        changed |= write_string_array(style_table, "disabled_skills", &disabled)?;

        if style_table.is_empty() {
            profiles_table.remove(resolved_style_key.as_str());
            changed = true;
        }

        if profiles_table.is_empty() {
            root.remove("shell_style_profiles");
            changed = true;
        }
    }

    if changed {
        std::fs::create_dir_all(code_home)?;
        let tmp_path = config_path.with_extension("tmp");
        std::fs::write(&tmp_path, doc.to_string())?;
        std::fs::rename(&tmp_path, &config_path)?;
    }

    Ok(changed)
}

/// Update shell-style profile path lists for `references` and `skill_roots`.
///
/// Empty lists remove their corresponding keys. If the resulting style profile
/// table is empty it is removed.
pub fn set_shell_style_profile_paths(
    code_home: &Path,
    style: ShellScriptStyle,
    references: &[PathBuf],
    skill_roots: &[PathBuf],
) -> anyhow::Result<bool> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    let style_key = style.to_string();
    let mut changed = false;

    {
        let root = doc.as_table_mut();
        match root.get("shell_style_profiles") {
            Some(item) => {
                if item.as_table().is_none() {
                    return Err(anyhow::anyhow!(
                        "`shell_style_profiles` must be a TOML table"
                    ));
                }
            }
            None => {
                if references.is_empty() && skill_roots.is_empty() {
                    return Ok(false);
                }
                let mut table = TomlTable::new();
                table.set_implicit(true);
                root.insert("shell_style_profiles", TomlItem::Table(table));
                changed = true;
            }
        }
    }

    {
        let root = doc.as_table_mut();
        let profiles_table = root
            .get_mut("shell_style_profiles")
            .and_then(|item| item.as_table_mut())
            .ok_or_else(|| anyhow::anyhow!("failed to prepare shell_style_profiles table"))?;

        let mut resolved_style_key = find_shell_style_profile_key(profiles_table, style)?;
        match resolved_style_key.as_deref() {
            Some(existing_key) => {
                if profiles_table
                    .get(existing_key)
                    .and_then(|item| item.as_table())
                    .is_none()
                {
                    return Err(anyhow::anyhow!(
                        "`shell_style_profiles.{existing_key}` must be a TOML table"
                    ));
                }
            }
            None => {
                if references.is_empty() && skill_roots.is_empty() {
                    return Ok(changed);
                }
                let mut style_table = TomlTable::new();
                style_table.set_implicit(false);
                profiles_table.insert(style_key.as_str(), TomlItem::Table(style_table));
                resolved_style_key = Some(style_key.clone());
                changed = true;
            }
        }
        let resolved_style_key = resolved_style_key
            .ok_or_else(|| anyhow::anyhow!("failed to resolve shell style profile key"))?;

        let style_table = profiles_table
            .get_mut(resolved_style_key.as_str())
            .and_then(|item| item.as_table_mut())
            .ok_or_else(|| anyhow::anyhow!("failed to prepare shell style profile table"))?;

        changed |= write_path_array(style_table, "references", references)?;
        changed |= write_path_array(style_table, "skill_roots", skill_roots)?;

        if style_table.is_empty() {
            profiles_table.remove(resolved_style_key.as_str());
            changed = true;
        }

        if profiles_table.is_empty() {
            root.remove("shell_style_profiles");
            changed = true;
        }
    }

    if changed {
        std::fs::create_dir_all(code_home)?;
        let tmp_path = config_path.with_extension("tmp");
        std::fs::write(&tmp_path, doc.to_string())?;
        std::fs::rename(&tmp_path, &config_path)?;
    }

    Ok(changed)
}

/// Update shell-style profile MCP server include/exclude filters.
///
/// Empty lists remove their corresponding keys. If the resulting `mcp_servers`
/// table becomes empty it is removed.
pub fn set_shell_style_profile_mcp_servers(
    code_home: &Path,
    style: ShellScriptStyle,
    include: &[String],
    exclude: &[String],
) -> anyhow::Result<bool> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    let style_key = style.to_string();
    let mut changed = false;

    {
        let root = doc.as_table_mut();
        match root.get("shell_style_profiles") {
            Some(item) => {
                if item.as_table().is_none() {
                    return Err(anyhow::anyhow!(
                        "`shell_style_profiles` must be a TOML table"
                    ));
                }
            }
            None => {
                if include.is_empty() && exclude.is_empty() {
                    return Ok(false);
                }
                let mut table = TomlTable::new();
                table.set_implicit(true);
                root.insert("shell_style_profiles", TomlItem::Table(table));
                changed = true;
            }
        }
    }

    {
        let root = doc.as_table_mut();
        let profiles_table = root
            .get_mut("shell_style_profiles")
            .and_then(|item| item.as_table_mut())
            .ok_or_else(|| anyhow::anyhow!("failed to prepare shell_style_profiles table"))?;

        let mut resolved_style_key = find_shell_style_profile_key(profiles_table, style)?;
        match resolved_style_key.as_deref() {
            Some(existing_key) => {
                if profiles_table
                    .get(existing_key)
                    .and_then(|item| item.as_table())
                    .is_none()
                {
                    return Err(anyhow::anyhow!(
                        "`shell_style_profiles.{existing_key}` must be a TOML table"
                    ));
                }
            }
            None => {
                if include.is_empty() && exclude.is_empty() {
                    return Ok(changed);
                }
                let mut style_table = TomlTable::new();
                style_table.set_implicit(false);
                profiles_table.insert(style_key.as_str(), TomlItem::Table(style_table));
                resolved_style_key = Some(style_key.clone());
                changed = true;
            }
        }
        let resolved_style_key = resolved_style_key
            .ok_or_else(|| anyhow::anyhow!("failed to resolve shell style profile key"))?;

        let style_table = profiles_table
            .get_mut(resolved_style_key.as_str())
            .and_then(|item| item.as_table_mut())
            .ok_or_else(|| anyhow::anyhow!("failed to prepare shell style profile table"))?;

        let mcp_key = "mcp_servers";
        let mut has_mcp_table = false;
        match style_table.get(mcp_key) {
            Some(item) => {
                if item.as_table().is_none() {
                    return Err(anyhow::anyhow!(
                        "`shell_style_profiles.{resolved_style_key}.{mcp_key}` must be a TOML table"
                    ));
                }
                has_mcp_table = true;
            }
            None => {
                if !include.is_empty() || !exclude.is_empty() {
                    let mut mcp_table = TomlTable::new();
                    mcp_table.set_implicit(false);
                    style_table.insert(mcp_key, TomlItem::Table(mcp_table));
                    changed = true;
                    has_mcp_table = true;
                }
            }
        }

        if has_mcp_table {
            let mcp_table = style_table
                .get_mut(mcp_key)
                .and_then(|item| item.as_table_mut())
                .ok_or_else(|| anyhow::anyhow!("failed to prepare mcp_servers table"))?;
            changed |= write_string_array(mcp_table, "include", include)?;
            changed |= write_string_array(mcp_table, "exclude", exclude)?;

            if mcp_table.is_empty() {
                style_table.remove(mcp_key);
                changed = true;
            }
        }

        if style_table.is_empty() {
            profiles_table.remove(resolved_style_key.as_str());
            changed = true;
        }

        if profiles_table.is_empty() {
            root.remove("shell_style_profiles");
            changed = true;
        }
    }

    if changed {
        std::fs::create_dir_all(code_home)?;
        let tmp_path = config_path.with_extension("tmp");
        std::fs::write(&tmp_path, doc.to_string())?;
        std::fs::rename(&tmp_path, &config_path)?;
    }

    Ok(changed)
}

fn normalize_skill_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn find_shell_style_profile_key(
    table: &TomlTable,
    style: ShellScriptStyle,
) -> anyhow::Result<Option<String>> {
    let mut match_key: Option<String> = None;
    for (key, _) in table.iter() {
        if ShellScriptStyle::parse(key) == Some(style) {
            if let Some(existing) = &match_key {
                return Err(anyhow::anyhow!(
                    "multiple shell_style_profiles entries map to `{style}` (`{existing}` and `{key}`); keep only one"
                ));
            }
            match_key = Some(key.to_string());
        }
    }
    Ok(match_key)
}

fn read_string_array(table: &TomlTable, key: &str) -> anyhow::Result<Vec<String>> {
    let Some(item) = table.get(key) else {
        return Ok(Vec::new());
    };
    let array = item
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("`{key}` must be a TOML array"))?;
    let mut out: Vec<String> = Vec::new();
    for value in array.iter() {
        let as_str = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("`{key}` entries must be TOML strings"))?;
        let trimmed = as_str.trim();
        if !trimmed.is_empty() {
            out.push(trimmed.to_string());
        }
    }
    Ok(out)
}

fn remove_skill_name(values: &mut Vec<String>, normalized_target: &str) -> bool {
    let original_len = values.len();
    values.retain(|entry| normalize_skill_name(entry) != normalized_target);
    original_len != values.len()
}

fn push_unique_skill_name(values: &mut Vec<String>, skill_name: &str) -> bool {
    let normalized_target = normalize_skill_name(skill_name);
    if values
        .iter()
        .any(|entry| normalize_skill_name(entry) == normalized_target)
    {
        return false;
    }
    values.push(skill_name.trim().to_string());
    true
}

fn write_string_array(table: &mut TomlTable, key: &str, values: &[String]) -> anyhow::Result<bool> {
    if values.is_empty() {
        return Ok(table.remove(key).is_some());
    }

    let mut deduped: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let normalized = normalize_skill_name(trimmed);
        if seen.insert(normalized) {
            deduped.push(trimmed.to_string());
        }
    }

    if deduped.is_empty() {
        return Ok(table.remove(key).is_some());
    }

    let existing = read_string_array(table, key)?;
    if existing == deduped {
        return Ok(false);
    }

    let mut array = TomlArray::new();
    for value in &deduped {
        array.push(value.as_str());
    }
    table[key] = toml_edit::value(array);
    Ok(true)
}

fn read_path_array(table: &TomlTable, key: &str) -> anyhow::Result<Vec<PathBuf>> {
    let Some(item) = table.get(key) else {
        return Ok(Vec::new());
    };
    let array = item
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("`{key}` must be a TOML array"))?;
    let mut out: Vec<PathBuf> = Vec::new();
    for value in array.iter() {
        let as_str = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("`{key}` entries must be TOML strings"))?;
        let trimmed = as_str.trim();
        if !trimmed.is_empty() {
            out.push(PathBuf::from(trimmed));
        }
    }
    Ok(out)
}

fn write_path_array(table: &mut TomlTable, key: &str, values: &[PathBuf]) -> anyhow::Result<bool> {
    if values.is_empty() {
        return Ok(table.remove(key).is_some());
    }

    let mut deduped: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for value in values {
        let rendered = value.to_string_lossy().trim().to_string();
        if rendered.is_empty() {
            continue;
        }
        if seen.insert(rendered.clone()) {
            deduped.push(rendered);
        }
    }

    if deduped.is_empty() {
        return Ok(table.remove(key).is_some());
    }

    let existing = read_path_array(table, key)?
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if existing == deduped {
        return Ok(false);
    }

    let mut array = TomlArray::new();
    for value in &deduped {
        array.push(value.as_str());
    }
    table[key] = toml_edit::value(array);
    Ok(true)
}

/// Patch `CODEX_HOME/config.toml` project state.
/// Use with caution.
pub fn set_project_trusted(code_home: &Path, project_path: &Path) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    // Parse existing config if present; otherwise start a new document.
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    set_project_trusted_inner(&mut doc, project_path)?;

    // ensure code_home exists
    std::fs::create_dir_all(code_home)?;

    // create a tmp_file
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;

    // atomically move the tmp file into config.toml
    tmp_file.persist(config_path)?;

    Ok(())
}

fn set_project_trusted_inner(doc: &mut DocumentMut, project_path: &Path) -> anyhow::Result<()> {
    // Ensure we render a human-friendly structure:
    //
    // [projects]
    // [projects."/path/to/project"]
    // trust_level = "trusted"
    //
    // rather than inline tables like:
    //
    // [projects]
    // "/path/to/project" = { trust_level = "trusted" }
    let project_key = project_path.to_string_lossy().to_string();

    // Ensure top-level `projects` exists as a non-inline, explicit table. If it
    // exists but was previously represented as a non-table (e.g., inline),
    // replace it with an explicit table.
    let mut created_projects_table = false;
    {
        let root = doc.as_table_mut();
        let needs_table = !root.contains_key("projects")
            || root.get("projects").and_then(|i| i.as_table()).is_none();
        if needs_table {
            root.insert("projects", toml_edit::table());
            created_projects_table = true;
        }
    }
    let Some(projects_tbl) = doc["projects"].as_table_mut() else {
        return Err(anyhow::anyhow!(
            "projects table missing after initialization"
        ));
    };

    // If we created the `projects` table ourselves, keep it implicit so we
    // don't render a standalone `[projects]` header.
    if created_projects_table {
        projects_tbl.set_implicit(true);
    }

    // Ensure the per-project entry is its own explicit table. If it exists but
    // is not a table (e.g., an inline table), replace it with an explicit table.
    let needs_proj_table = !projects_tbl.contains_key(project_key.as_str())
        || projects_tbl
            .get(project_key.as_str())
            .and_then(|i| i.as_table())
            .is_none();
    if needs_proj_table {
        projects_tbl.insert(project_key.as_str(), toml_edit::table());
    }
    let Some(proj_tbl) = projects_tbl
        .get_mut(project_key.as_str())
        .and_then(|i| i.as_table_mut())
    else {
        return Err(anyhow::anyhow!("project table missing for {project_key}"));
    };
    proj_tbl.set_implicit(false);
    proj_tbl["trust_level"] = toml_edit::value("trusted");

    Ok(())
}

/// Persist the selected TUI theme into `CODEX_HOME/config.toml` at `[tui.theme].name`.
pub fn set_tui_theme_name(code_home: &Path, theme: ThemeName) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);

    // Parse existing config if present; otherwise start a new document.
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    // Map enum to kebab-case string used in config
    let theme_str = match theme {
        ThemeName::LightPhoton => "light-photon",
        ThemeName::LightPhotonAnsi16 => "light-photon-ansi16",
        ThemeName::LightPrismRainbow => "light-prism-rainbow",
        ThemeName::LightVividTriad => "light-vivid-triad",
        ThemeName::LightPorcelain => "light-porcelain",
        ThemeName::LightSandbar => "light-sandbar",
        ThemeName::LightGlacier => "light-glacier",
        ThemeName::DarkCarbonNight => "dark-carbon-night",
        ThemeName::DarkCarbonAnsi16 => "dark-carbon-ansi16",
        ThemeName::DarkShinobiDusk => "dark-shinobi-dusk",
        ThemeName::DarkOledBlackPro => "dark-oled-black-pro",
        ThemeName::DarkAmberTerminal => "dark-amber-terminal",
        ThemeName::DarkAuroraFlux => "dark-aurora-flux",
        ThemeName::DarkCharcoalRainbow => "dark-charcoal-rainbow",
        ThemeName::DarkZenGarden => "dark-zen-garden",
        ThemeName::DarkPaperLightPro => "dark-paper-light-pro",
        ThemeName::Custom => "custom",
    };

    // Write `[tui.theme].name = "…"`
    doc["tui"]["theme"]["name"] = toml_edit::value(theme_str);
    // When switching away from the Custom theme, clear any lingering custom
    // overrides so built-in themes render true to spec on next startup.
    if theme != ThemeName::Custom
        && let Some(tbl) = doc["tui"]["theme"].as_table_mut() {
            tbl.remove("label");
            tbl.remove("colors");
        }

    // ensure code_home exists
    std::fs::create_dir_all(code_home)?;

    // create a tmp_file
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;

    // atomically move the tmp file into config.toml
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Record the most recent terminal background autodetect result under `[tui.cached_terminal_background]`.
pub fn set_cached_terminal_background(
    code_home: &Path,
    cache: &CachedTerminalBackground,
) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    let mut tbl = toml_edit::Table::new();
    tbl.set_implicit(false);
    tbl.insert("is_dark", toml_edit::value(cache.is_dark));
    if let Some(term) = &cache.term {
        tbl.insert("term", toml_edit::value(term.as_str()));
    }
    if let Some(term_program) = &cache.term_program {
        tbl.insert("term_program", toml_edit::value(term_program.as_str()));
    }
    if let Some(term_program_version) = &cache.term_program_version {
        tbl.insert(
            "term_program_version",
            toml_edit::value(term_program_version.as_str()),
        );
    }
    if let Some(colorfgbg) = &cache.colorfgbg {
        tbl.insert("colorfgbg", toml_edit::value(colorfgbg.as_str()));
    }
    if let Some(source) = &cache.source {
        tbl.insert("source", toml_edit::value(source.as_str()));
    }
    if let Some(rgb) = &cache.rgb {
        tbl.insert("rgb", toml_edit::value(rgb.as_str()));
    }

    doc["tui"]["cached_terminal_background"] = toml_edit::Item::Table(tbl);

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;
    Ok(())
}

/// Persist the selected spinner into `CODEX_HOME/config.toml` at `[tui.spinner].name`.
pub fn set_tui_spinner_name(code_home: &Path, spinner_name: &str) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);

    // Parse existing config if present; otherwise start a new document.
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    // Write `[tui.spinner].name = "…"`
    doc["tui"]["spinner"]["name"] = toml_edit::value(spinner_name);

    // ensure code_home exists
    std::fs::create_dir_all(code_home)?;

    // create a tmp_file
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;

    // atomically move the tmp file into config.toml
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Save or update a custom spinner under `[tui.spinner.custom.<id>]` with a display `label`,
/// and set it active by writing `[tui.spinner].name = <id>`.
pub fn set_custom_spinner(
    code_home: &Path,
    id: &str,
    label: &str,
    interval: u64,
    frames: &[String],
) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };
    // Write custom spinner
    let node = &mut doc["tui"]["spinner"]["custom"][id];
    node["interval"] = toml_edit::value(interval as i64);
    let mut arr = toml_edit::Array::default();
    for s in frames { arr.push(s.as_str()); }
    node["frames"] = toml_edit::value(arr);
    node["label"] = toml_edit::value(label);

    // Set as active
    doc["tui"]["spinner"]["name"] = toml_edit::value(id);

    std::fs::create_dir_all(code_home)?;
    let tmp = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp.path(), doc.to_string())?;
    tmp.persist(config_path)?;
    Ok(())
}

/// Save or update a custom theme with a display `label` and color overrides
/// under `[tui.theme]`, and set it active by writing `[tui.theme].name = "custom"`.
pub fn set_custom_theme(
    code_home: &Path,
    label: &str,
    colors: &ThemeColors,
    set_active: bool,
    is_dark: Option<bool>,
) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    // Optionally activate custom theme and persist label
    if set_active {
        doc["tui"]["theme"]["name"] = toml_edit::value("custom");
    }
    doc["tui"]["theme"]["label"] = toml_edit::value(label);
    if let Some(d) = is_dark { doc["tui"]["theme"]["is_dark"] = toml_edit::value(d); }

    // Ensure colors table exists and write provided keys
    {
        use toml_edit::Item as It;
        if !doc["tui"]["theme"].is_table() {
            doc["tui"]["theme"] = It::Table(toml_edit::Table::new());
        }
        let Some(theme_tbl) = doc["tui"]["theme"].as_table_mut() else {
            return Err(anyhow::anyhow!("tui.theme must be a table"));
        };
        if !theme_tbl.contains_key("colors") {
            theme_tbl.insert("colors", It::Table(toml_edit::Table::new()));
        }
        let Some(colors_tbl) = theme_tbl["colors"].as_table_mut() else {
            return Err(anyhow::anyhow!("tui.theme.colors must be a table"));
        };
        macro_rules! set_opt {
            ($key:ident) => {
                if let Some(ref v) = colors.$key { colors_tbl.insert(stringify!($key), toml_edit::value(v.clone())); }
            };
        }
        set_opt!(primary);
        set_opt!(secondary);
        set_opt!(background);
        set_opt!(foreground);
        set_opt!(border);
        set_opt!(border_focused);
        set_opt!(selection);
        set_opt!(cursor);
        set_opt!(success);
        set_opt!(warning);
        set_opt!(error);
        set_opt!(info);
        set_opt!(text);
        set_opt!(text_dim);
        set_opt!(text_bright);
        set_opt!(keyword);
        set_opt!(string);
        set_opt!(comment);
        set_opt!(function);
        set_opt!(spinner);
        set_opt!(progress);
    }

    std::fs::create_dir_all(code_home)?;
    let tmp = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp.path(), doc.to_string())?;
    tmp.persist(config_path)?;
    Ok(())
}

/// Persist the alternate screen preference into `CODEX_HOME/config.toml` at `[tui].alternate_screen`.
pub fn set_tui_alternate_screen(code_home: &Path, enabled: bool) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);

    // Parse existing config if present; otherwise start a new document.
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    // Write `[tui].alternate_screen = true/false`
    doc["tui"]["alternate_screen"] = toml_edit::value(enabled);

    // ensure code_home exists
    std::fs::create_dir_all(code_home)?;

    // create a tmp_file
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;

    // atomically move the tmp file into config.toml
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist the limits layout mode into `CODEX_HOME/config.toml` at `[tui.limits].layout_mode`.
pub fn set_tui_limits_layout_mode(
    code_home: &Path,
    layout_mode: LimitsLayoutMode,
) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    let mode = match layout_mode {
        LimitsLayoutMode::Auto => "auto",
        LimitsLayoutMode::SingleColumn => "single-column",
    };
    doc["tui"]["limits"]["layout_mode"] = toml_edit::value(mode);

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist the TUI notifications preference into `CODEX_HOME/config.toml` at `[tui].notifications`.
pub fn set_tui_notifications(
    code_home: &Path,
    notifications: crate::config_types::Notifications,
) -> anyhow::Result<()> {
    use crate::config_types::Notifications;

    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    match notifications {
        Notifications::Enabled(value) => {
            doc["tui"]["notifications"] = toml_edit::value(value);
        }
        Notifications::Custom(values) => {
            let mut array = TomlArray::default();
            for value in values {
                array.push(value);
            }
            doc["tui"]["notifications"] = TomlItem::Value(array.into());
        }
    }

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist account-store path preferences into `CODEX_HOME/config.toml` at `[accounts]`.
///
/// - `read_paths` is an ordered list of candidate files to read from.
/// - `write_path` is the file used for writes/updates.
/// - If both are empty/unset, the `[accounts]` table is removed so defaults apply.
pub fn set_account_store_paths(
    code_home: &Path,
    read_paths: &[String],
    write_path: Option<&str>,
) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    let normalized_read_paths = read_paths
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>();

    let normalized_write_path = write_path
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(std::string::ToString::to_string);

    if normalized_read_paths.is_empty() && normalized_write_path.is_none() {
        doc.as_table_mut().remove("accounts");
    } else {
        if !doc["accounts"].is_table() {
            doc["accounts"] = TomlItem::Table(TomlTable::new());
        }
        let Some(accounts_table) = doc["accounts"].as_table_mut() else {
            return Err(anyhow::anyhow!(
                "failed to configure account store paths: [accounts] is not a table"
            ));
        };

        if normalized_read_paths.is_empty() {
            accounts_table.remove("read_paths");
        } else {
            let mut array = TomlArray::new();
            for path in normalized_read_paths {
                array.push(path);
            }
            accounts_table["read_paths"] = TomlItem::Value(array.into());
        }

        if let Some(path) = normalized_write_path {
            accounts_table["write_path"] = toml_edit::value(path);
        } else {
            accounts_table.remove("write_path");
        }
    }

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist the review auto-resolve preference into `CODEX_HOME/config.toml` at `[tui].review_auto_resolve`.
pub fn set_tui_review_auto_resolve(code_home: &Path, enabled: bool) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));

    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    doc["tui"]["review_auto_resolve"] = toml_edit::value(enabled);

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist the auto review preference into `CODEX_HOME/config.toml` at `[tui].auto_review_enabled`.
pub fn set_tui_auto_review_enabled(code_home: &Path, enabled: bool) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));

    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    doc["tui"]["auto_review_enabled"] = toml_edit::value(enabled);

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist the review model + reasoning effort into `CODEX_HOME/config.toml`.
pub fn set_review_model(
    code_home: &Path,
    model: &str,
    effort: ReasoningEffort,
    use_chat_model: bool,
) -> anyhow::Result<()> {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("review model cannot be empty"));
    }

    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    doc["review_use_chat_model"] = toml_edit::value(use_chat_model);
    if !use_chat_model {
        doc["review_model"] = toml_edit::value(trimmed);
        doc["review_model_reasoning_effort"] =
            toml_edit::value(effort.to_string().to_ascii_lowercase());
    }

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist the resolve model + reasoning effort for `/review` auto-resolve flows.
pub fn set_review_resolve_model(
    code_home: &Path,
    model: &str,
    effort: ReasoningEffort,
    use_chat_model: bool,
) -> anyhow::Result<()> {
    let trimmed = model.trim();
    if !use_chat_model && trimmed.is_empty() {
        return Err(anyhow::anyhow!("review resolve model cannot be empty"));
    }

    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    doc["review_resolve_use_chat_model"] = toml_edit::value(use_chat_model);
    if !use_chat_model {
        doc["review_resolve_model"] = toml_edit::value(trimmed);
        doc["review_resolve_model_reasoning_effort"] =
            toml_edit::value(effort.to_string().to_ascii_lowercase());
    }

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist the planning model + reasoning effort into `CODEX_HOME/config.toml`.
pub fn set_planning_model(
    code_home: &Path,
    model: &str,
    effort: ReasoningEffort,
    use_chat_model: bool,
) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));

    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    doc["planning_use_chat_model"] = toml_edit::value(use_chat_model);
    if !use_chat_model {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            return Err(anyhow::anyhow!("planning model cannot be empty"));
        }
        doc["planning_model"] = toml_edit::value(trimmed);
        doc["planning_model_reasoning_effort"] =
            toml_edit::value(effort.to_string().to_ascii_lowercase());
    }

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist the Auto Review review model + reasoning effort.
pub fn set_auto_review_model(
    code_home: &Path,
    model: &str,
    effort: ReasoningEffort,
    use_chat_model: bool,
) -> anyhow::Result<()> {
    let trimmed = model.trim();
    if !use_chat_model && trimmed.is_empty() {
        return Err(anyhow::anyhow!("auto review model cannot be empty"));
    }

    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));

    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    doc["auto_review_use_chat_model"] = toml_edit::value(use_chat_model);
    if !use_chat_model {
        doc["auto_review_model"] = toml_edit::value(trimmed);
        doc["auto_review_model_reasoning_effort"] =
            toml_edit::value(effort.to_string().to_ascii_lowercase());
    }

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist the Auto Review resolve model + reasoning effort.
pub fn set_auto_review_resolve_model(
    code_home: &Path,
    model: &str,
    effort: ReasoningEffort,
    use_chat_model: bool,
) -> anyhow::Result<()> {
    let trimmed = model.trim();
    if !use_chat_model && trimmed.is_empty() {
        return Err(anyhow::anyhow!("auto review resolve model cannot be empty"));
    }

    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));

    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    doc["auto_review_resolve_use_chat_model"] = toml_edit::value(use_chat_model);
    if !use_chat_model {
        doc["auto_review_resolve_model"] = toml_edit::value(trimmed);
        doc["auto_review_resolve_model_reasoning_effort"] =
            toml_edit::value(effort.to_string().to_ascii_lowercase());
    }

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist Auto Drive defaults under `[auto_drive]`.
pub fn set_auto_drive_settings(
    code_home: &Path,
    settings: &AutoDriveSettings,
    use_chat_model: bool,
) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));

    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(contents) => contents.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    if let Some(tui_tbl) = doc["tui"].as_table_mut() {
        tui_tbl.remove("auto_drive");
    }

    if !doc.as_table().contains_key("auto_drive") || !doc["auto_drive"].is_table() {
        doc["auto_drive"] = TomlItem::Table(TomlTable::new());
    }

    doc["auto_drive_use_chat_model"] = toml_edit::value(use_chat_model);

    doc["auto_drive"]["review_enabled"] = toml_edit::value(settings.review_enabled);
    doc["auto_drive"]["agents_enabled"] = toml_edit::value(settings.agents_enabled);
    doc["auto_drive"]["qa_automation_enabled"] =
        toml_edit::value(settings.qa_automation_enabled);
    doc["auto_drive"]["cross_check_enabled"] =
        toml_edit::value(settings.cross_check_enabled);
    doc["auto_drive"]["observer_enabled"] =
        toml_edit::value(settings.observer_enabled);
    doc["auto_drive"]["coordinator_routing"] =
        toml_edit::value(settings.coordinator_routing);
    doc["auto_drive"]["model_routing_enabled"] =
        toml_edit::value(settings.model_routing_enabled);
    if settings.model_routing_entries.is_empty() {
        if let Some(auto_drive_tbl) = doc["auto_drive"].as_table_mut() {
            auto_drive_tbl.remove("model_routing_entries");
        }
    } else {
        let mut routing_entries = TomlArrayOfTables::new();
        for entry in &settings.model_routing_entries {
            let mut table = TomlTable::new();
            table.insert("model", TomlItem::Value(entry.model.trim().into()));
            table.insert("enabled", TomlItem::Value(entry.enabled.into()));

            let mut reasoning_levels = TomlArray::new();
            for level in &entry.reasoning_levels {
                reasoning_levels.push(level.to_string().to_ascii_lowercase());
            }
            table.insert(
                "reasoning_levels",
                TomlItem::Value(toml_edit::Value::Array(reasoning_levels)),
            );

            table.insert(
                "description",
                TomlItem::Value(entry.description.trim().into()),
            );

            routing_entries.push(table);
        }
        doc["auto_drive"]["model_routing_entries"] = TomlItem::ArrayOfTables(routing_entries);
    }
    doc["auto_drive"]["model"] = toml_edit::value(settings.model.trim());
    doc["auto_drive"]["model_reasoning_effort"] = toml_edit::value(
        settings
            .model_reasoning_effort
            .to_string()
            .to_ascii_lowercase(),
    );
    doc["auto_drive"]["auto_resolve_review_attempts"] =
        toml_edit::value(settings.auto_resolve_review_attempts.get() as i64);
    doc["auto_drive"]["auto_review_followup_attempts"] =
        toml_edit::value(settings.auto_review_followup_attempts.get() as i64);
    doc["auto_drive"]["coordinator_turn_cap"] =
        toml_edit::value(settings.coordinator_turn_cap as i64);

    let mode_str = match settings.continue_mode {
        AutoDriveContinueMode::Immediate => "immediate",
        AutoDriveContinueMode::TenSeconds => "ten-seconds",
        AutoDriveContinueMode::SixtySeconds => "sixty-seconds",
        AutoDriveContinueMode::Manual => "manual",
    };
    doc["auto_drive"]["continue_mode"] = toml_edit::value(mode_str);

    std::fs::create_dir_all(code_home)?;
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Legacy helper: persist Auto Drive defaults under `[auto_drive]` while
/// accepting the former API surface.
#[deprecated(note = "use set_auto_drive_settings instead")]
pub fn set_tui_auto_drive_settings(
    code_home: &Path,
    settings: &AutoDriveSettings,
    use_chat_model: bool,
) -> anyhow::Result<()> {
    set_auto_drive_settings(code_home, settings, use_chat_model)
}

/// Persist the GitHub workflow check preference under `[github].check_workflows_on_push`.
pub fn set_github_check_on_push(code_home: &Path, enabled: bool) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);

    // Parse existing config if present; otherwise start a new document.
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    // Write `[github].check_workflows_on_push = <enabled>`
    doc["github"]["check_workflows_on_push"] = toml_edit::value(enabled);

    // ensure code_home exists
    std::fs::create_dir_all(code_home)?;

    // create a tmp_file
    let tmp_file = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp_file.path(), doc.to_string())?;

    // atomically move the tmp file into config.toml
    tmp_file.persist(config_path)?;

    Ok(())
}

/// Persist `github.actionlint_on_patch = <enabled>`.
pub fn set_github_actionlint_on_patch(
    code_home: &Path,
    enabled: bool,
) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    doc["github"]["actionlint_on_patch"] = toml_edit::value(enabled);

    std::fs::create_dir_all(code_home)?;
    let tmp = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp.path(), doc.to_string())?;
    tmp.persist(config_path)?;
    Ok(())
}

/// Persist `[validation.groups.<group>] = <enabled>`.
pub fn set_validation_group_enabled(
    code_home: &Path,
    group: &str,
    enabled: bool,
) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    doc["validation"]["groups"][group] = toml_edit::value(enabled);

    std::fs::create_dir_all(code_home)?;
    let tmp = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp.path(), doc.to_string())?;
    tmp.persist(config_path)?;
    Ok(())
}

/// Persist `[validation.tools.<tool>] = <enabled>`.
pub fn set_validation_tool_enabled(
    code_home: &Path,
    tool: &str,
    enabled: bool,
) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    doc["validation"]["tools"][tool] = toml_edit::value(enabled);

    std::fs::create_dir_all(code_home)?;
    let tmp = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp.path(), doc.to_string())?;
    tmp.persist(config_path)?;
    Ok(())
}

/// Persist per-project access mode under `[projects."<path>"]` with
/// `approval_policy` and `sandbox_mode`.
pub fn set_project_access_mode(
    code_home: &Path,
    project_path: &Path,
    approval: AskForApproval,
    sandbox_mode: SandboxMode,
) -> anyhow::Result<()> {
    let config_path = code_home.join(CONFIG_TOML_FILE);

    // Parse existing config if present; otherwise start a new document.
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    // Ensure projects table and the per-project table exist
    let project_key = project_path.to_string_lossy().to_string();
    // Ensure `projects` is a table; if key exists but is not a table, replace it.
    let has_projects_table = doc
        .as_table()
        .get("projects")
        .and_then(|i| i.as_table())
        .is_some();
    if !has_projects_table {
        doc["projects"] = TomlItem::Table(toml_edit::Table::new());
    }
    let Some(projects_tbl) = doc["projects"].as_table_mut() else {
        return Err(anyhow::anyhow!("failed to prepare projects table"));
    };
    // Ensure per-project entry exists and is a table; replace if wrong type.
    let needs_proj_table = projects_tbl
        .get(project_key.as_str())
        .and_then(|i| i.as_table())
        .is_none();
    if needs_proj_table {
        projects_tbl.insert(project_key.as_str(), TomlItem::Table(toml_edit::Table::new()));
    }
    let proj_tbl = projects_tbl
        .get_mut(project_key.as_str())
        .and_then(|i| i.as_table_mut())
        .ok_or_else(|| anyhow::anyhow!(format!("failed to create projects.{project_key} table")))?;

    // Write fields
    proj_tbl.insert(
        "approval_policy",
        TomlItem::Value(toml_edit::Value::from(format!("{approval}"))),
    );
    proj_tbl.insert(
        "sandbox_mode",
        TomlItem::Value(toml_edit::Value::from(format!("{sandbox_mode}"))),
    );

    // Harmonize trust_level with selected access mode:
    // - Full Access (Never + DangerFullAccess): set trust_level = "trusted" so future runs
    //   default to non-interactive behavior when no overrides are present.
    // - Other modes: remove trust_level to avoid conflicting with per-project policy.
    let full_access = matches!(
        (approval, sandbox_mode),
        (AskForApproval::Never, SandboxMode::DangerFullAccess)
    );
    if full_access {
        proj_tbl.insert(
            "trust_level",
            TomlItem::Value(toml_edit::Value::from("trusted")),
        );
    } else {
        proj_tbl.remove("trust_level");
    }

    // Ensure home exists; write atomically
    std::fs::create_dir_all(code_home)?;
    let tmp = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp.path(), doc.to_string())?;
    tmp.persist(config_path)?;

    Ok(())
}

/// Append a command pattern to `[projects."<path>"].always_allow_commands`.
pub fn add_project_allowed_command(
    code_home: &Path,
    project_path: &Path,
    command: &[String],
    match_kind: ApprovedCommandMatchKind,
) -> anyhow::Result<()> {
    let command = crate::command_canonicalization::normalize_command_for_persistence(command);
    if command.is_empty() {
        return Ok(());
    }

    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    let project_key = project_path.to_string_lossy().to_string();
    if doc
        .as_table()
        .get("projects")
        .and_then(|i| i.as_table())
        .is_none()
    {
        doc["projects"] = TomlItem::Table(TomlTable::new());
    }

    let Some(projects_tbl) = doc["projects"].as_table_mut() else {
        return Err(anyhow::anyhow!("failed to prepare projects table"));
    };

    if projects_tbl
        .get(project_key.as_str())
        .and_then(|i| i.as_table())
        .is_none()
    {
        projects_tbl.insert(project_key.as_str(), TomlItem::Table(TomlTable::new()));
    }

    let project_tbl = projects_tbl
        .get_mut(project_key.as_str())
        .and_then(|i| i.as_table_mut())
        .ok_or_else(|| anyhow::anyhow!(format!("failed to create projects.{project_key} table")))?;

    let mut argv_array = TomlArray::new();
    for arg in &command {
        argv_array.push(arg.clone());
    }

    let mut table = TomlTable::new();
    table.insert("argv", TomlItem::Value(toml_edit::Value::Array(argv_array)));
    let match_str = match match_kind {
        ApprovedCommandMatchKind::Exact => "exact",
        ApprovedCommandMatchKind::Prefix => "prefix",
    };
    table.insert(
        "match_kind",
        TomlItem::Value(toml_edit::Value::from(match_str)),
    );

    if let Some(existing) = project_tbl
        .get_mut("always_allow_commands")
        .and_then(|item| item.as_array_of_tables_mut())
    {
        let exists = existing.iter().any(|tbl| {
            let argv_match = tbl
                .get("argv")
                .and_then(|item| item.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let match_kind = tbl
                .get("match_kind")
                .and_then(|item| item.as_str())
                .unwrap_or("exact");
            argv_match == command && match_kind.eq_ignore_ascii_case(match_str)
        });
        if !exists {
            existing.push(table);
        }
    } else {
        let mut arr = TomlArrayOfTables::new();
        arr.push(table);
        project_tbl.insert("always_allow_commands", TomlItem::ArrayOfTables(arr));
    }

    std::fs::create_dir_all(code_home)?;
    let tmp = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp.path(), doc.to_string())?;
    tmp.persist(config_path)?;

    Ok(())
}

/// List MCP servers from `CODEX_HOME/config.toml`.
/// Returns `(enabled, disabled)` lists of `(name, McpServerConfig)`.
type NamedMcpServer = (String, McpServerConfig);
type McpServerListPair = (Vec<NamedMcpServer>, Vec<NamedMcpServer>);

pub fn list_mcp_servers(code_home: &Path) -> anyhow::Result<McpServerListPair> {
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let doc_str = std::fs::read_to_string(&read_path).unwrap_or_default();
    let doc = doc_str.parse::<DocumentMut>().unwrap_or_else(|_| DocumentMut::new());

    fn table_to_list(tbl: &toml_edit::Table) -> Vec<(String, McpServerConfig)> {
        let mut out = Vec::new();
        for (name, item) in tbl.iter() {
            if let Some(t) = item.as_table() {
                let transport = if let Some(command) = t.get("command").and_then(|v| v.as_str()) {
                    let args: Vec<String> = t
                        .get("args")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|i| i.as_str().map(std::string::ToString::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    let env = t
                        .get("env")
                        .and_then(|v| {
                            if let Some(tbl) = v.as_inline_table() {
                                Some(
                                    tbl.iter()
                                        .filter_map(|(k, v)| {
                                            v.as_str().map(|s| (k.to_string(), s.to_string()))
                                        })
                                        .collect::<HashMap<_, _>>(),
                                )
                            } else { v.as_table().map(|table| table
                                        .iter()
                                        .filter_map(|(k, v)| {
                                            v.as_str().map(|s| (k.to_string(), s.to_string()))
                                        })
                                        .collect::<HashMap<_, _>>()) }
                        });

                    McpServerTransportConfig::Stdio {
                        command: command.to_string(),
                        args,
                        env,
                    }
                } else if let Some(url) = t.get("url").and_then(|v| v.as_str()) {
                    let bearer_token = t
                        .get("bearer_token")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string);

                    let bearer_token_env_var = t
                        .get("bearer_token_env_var")
                        .and_then(|v| v.as_str())
                        .map(std::string::ToString::to_string);

                    let table_string_map = |item: &toml_edit::Item| {
                        item.as_inline_table()
                            .map(|tbl| {
                                tbl.iter()
                                    .filter_map(|(k, v)| {
                                        v.as_str().map(|s| (k.to_string(), s.to_string()))
                                    })
                                    .collect::<HashMap<_, _>>()
                            })
                            .or_else(|| {
                                item.as_table().map(|table| {
                                    table
                                        .iter()
                                        .filter_map(|(k, v)| {
                                            v.as_str().map(|s| (k.to_string(), s.to_string()))
                                        })
                                        .collect::<HashMap<_, _>>()
                                })
                            })
                    };

                    let http_headers = t.get("http_headers").and_then(table_string_map);
                    let env_http_headers = t.get("env_http_headers").and_then(table_string_map);

                    McpServerTransportConfig::StreamableHttp {
                        url: url.to_string(),
                        bearer_token,
                        bearer_token_env_var,
                        http_headers,
                        env_http_headers,
                    }
                } else {
                    continue;
                };

                let startup_timeout_sec = t
                    .get("startup_timeout_sec")
                    .and_then(|v| {
                        v.as_float()
                            .map(|f| Duration::try_from_secs_f64(f).ok())
                            .or_else(|| {
                                Some(v.as_integer().map(|i| Duration::from_secs(i as u64)))
                            })
                    })
                    .flatten()
                    .or_else(|| {
                        t.get("startup_timeout_ms")
                            .and_then(toml_edit::Item::as_integer)
                            .map(|ms| Duration::from_millis(ms as u64))
                    });

                let tool_timeout_sec = t
                    .get("tool_timeout_sec")
                    .and_then(|v| {
                        v.as_float()
                            .map(|f| Duration::try_from_secs_f64(f).ok())
                            .or_else(|| {
                                Some(v.as_integer().map(|i| Duration::from_secs(i as u64)))
                            })
                    })
                    .flatten();

                let mut disabled_tools: Vec<String> = t
                    .get("disabled_tools")
                    .and_then(toml_edit::Item::as_array)
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|item| item.as_str().map(str::trim))
                            .filter(|name| !name.is_empty())
                            .map(std::string::ToString::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                disabled_tools.sort();
                disabled_tools.dedup();

                out.push((
                    name.to_string(),
                    McpServerConfig {
                        transport,
                        startup_timeout_sec,
                        tool_timeout_sec,
                        disabled_tools,
                    },
                ));
            }
        }
        out
    }

    let enabled = doc
        .as_table()
        .get("mcp_servers")
        .and_then(|i| i.as_table())
        .map(table_to_list)
        .unwrap_or_default();

    let disabled = doc
        .as_table()
        .get("mcp_servers_disabled")
        .and_then(|i| i.as_table())
        .map(table_to_list)
        .unwrap_or_default();

    Ok((enabled, disabled))
}

/// Add or update an MCP server under `[mcp_servers.<name>]`. If the same
/// server exists under `mcp_servers_disabled`, it will be removed from there.
pub fn add_mcp_server(
    code_home: &Path,
    name: &str,
    cfg: McpServerConfig,
) -> anyhow::Result<()> {
    // Validate server name for safety and compatibility with MCP tool naming.
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(anyhow::anyhow!(
            "invalid server name '{name}': must match ^[a-zA-Z0-9_-]+$"
        ));
    }

    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    // Ensure target tables exist
    if !doc.as_table().contains_key("mcp_servers") {
        doc["mcp_servers"] = TomlItem::Table(toml_edit::Table::new());
    }
    let Some(tbl) = doc["mcp_servers"].as_table_mut() else {
        return Err(anyhow::anyhow!("mcp_servers must be a table"));
    };

    let McpServerConfig {
        transport,
        startup_timeout_sec,
        tool_timeout_sec,
        disabled_tools,
    } = cfg;

    // Build table for this server
    let mut server_tbl = toml_edit::Table::new();
    match transport {
        McpServerTransportConfig::Stdio { command, args, env } => {
            server_tbl.insert("command", toml_edit::value(command));
            if !args.is_empty() {
                let mut arr = toml_edit::Array::new();
                for a in args {
                    arr.push(toml_edit::Value::from(a));
                }
                server_tbl.insert("args", TomlItem::Value(toml_edit::Value::Array(arr)));
            }
            if let Some(env) = env {
                let mut it = toml_edit::InlineTable::new();
                for (k, v) in env {
                    it.insert(&k, toml_edit::Value::from(v));
                }
                server_tbl.insert("env", TomlItem::Value(toml_edit::Value::InlineTable(it)));
            }
        }
        McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        } => {
            server_tbl.insert("url", toml_edit::value(url));
            if let Some(token) = bearer_token {
                server_tbl.insert("bearer_token", toml_edit::value(token));
            }
            if let Some(env_var) = bearer_token_env_var {
                server_tbl.insert("bearer_token_env_var", toml_edit::value(env_var));
            }
            if let Some(http_headers) = http_headers
                && !http_headers.is_empty()
            {
                let mut it = toml_edit::InlineTable::new();
                for (k, v) in http_headers {
                    it.insert(&k, toml_edit::Value::from(v));
                }
                server_tbl.insert(
                    "http_headers",
                    TomlItem::Value(toml_edit::Value::InlineTable(it)),
                );
            }
            if let Some(env_http_headers) = env_http_headers
                && !env_http_headers.is_empty()
            {
                let mut it = toml_edit::InlineTable::new();
                for (k, v) in env_http_headers {
                    it.insert(&k, toml_edit::Value::from(v));
                }
                server_tbl.insert(
                    "env_http_headers",
                    TomlItem::Value(toml_edit::Value::InlineTable(it)),
                );
            }
        }
    }

    if let Some(duration) = startup_timeout_sec {
        server_tbl.insert("startup_timeout_sec", toml_edit::value(duration.as_secs_f64()));
    }
    if let Some(duration) = tool_timeout_sec {
        server_tbl.insert("tool_timeout_sec", toml_edit::value(duration.as_secs_f64()));
    }
    if !disabled_tools.is_empty() {
        let mut arr = toml_edit::Array::new();
        for tool in disabled_tools {
            arr.push(toml_edit::Value::from(tool));
        }
        server_tbl.insert(
            "disabled_tools",
            TomlItem::Value(toml_edit::Value::Array(arr)),
        );
    }

    // Write into enabled table
    tbl.insert(name, TomlItem::Table(server_tbl));

    // Remove from disabled if present
    if let Some(disabled_tbl) = doc["mcp_servers_disabled"].as_table_mut() {
        disabled_tbl.remove(name);
    }

    // ensure code_home exists
    std::fs::create_dir_all(code_home)?;
    let tmp = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp.path(), doc.to_string())?;
    tmp.persist(config_path)?;
    Ok(())
}

/// Enable/disable an MCP server by moving it between `[mcp_servers]` and
/// `[mcp_servers_disabled]`. Returns `true` if a change was made.
pub fn set_mcp_server_enabled(
    code_home: &Path,
    name: &str,
    enabled: bool,
) -> anyhow::Result<bool> {
    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    // Helper to ensure table exists
    fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &'a str) -> &'a mut toml_edit::Table {
        if !doc.as_table().contains_key(key) {
            doc[key] = TomlItem::Table(toml_edit::Table::new());
        }
        match doc[key].as_table_mut() {
            Some(table) => table,
            None => panic!("table key '{key}' should be a table"),
        }
    }

    let mut changed = false;
    if enabled {
        // Move from disabled -> enabled
        let moved = {
            let disabled_tbl = ensure_table(&mut doc, "mcp_servers_disabled");
            disabled_tbl.remove(name)
        };
        if let Some(item) = moved {
            let enabled_tbl = ensure_table(&mut doc, "mcp_servers");
            enabled_tbl.insert(name, item);
            changed = true;
        }
    } else {
        // Move from enabled -> disabled
        let moved = {
            let enabled_tbl = ensure_table(&mut doc, "mcp_servers");
            enabled_tbl.remove(name)
        };
        if let Some(item) = moved {
            let disabled_tbl = ensure_table(&mut doc, "mcp_servers_disabled");
            disabled_tbl.insert(name, item);
            changed = true;
        }
    }

    if changed {
        std::fs::create_dir_all(code_home)?;
        let tmp = NamedTempFile::new_in(code_home)?;
        std::fs::write(tmp.path(), doc.to_string())?;
        tmp.persist(config_path)?;
    }

    Ok(changed)
}

/// Enable/disable a specific MCP tool for a named server.
/// Returns `true` when the persisted config changed.
pub fn set_mcp_server_tool_enabled(
    code_home: &Path,
    server_name: &str,
    tool_name: &str,
    enabled: bool,
) -> anyhow::Result<bool> {
    let normalized_tool = tool_name.trim();
    if normalized_tool.is_empty() {
        return Err(anyhow::anyhow!("tool name cannot be empty"));
    }

    let config_path = code_home.join(CONFIG_TOML_FILE);
    let read_path = resolve_code_path_for_read(code_home, Path::new(CONFIG_TOML_FILE));
    let mut doc = match std::fs::read_to_string(&read_path) {
        Ok(s) => s.parse::<DocumentMut>()?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DocumentMut::new(),
        Err(e) => return Err(e.into()),
    };

    fn find_server_table_mut<'a>(
        doc: &'a mut DocumentMut,
        server_name: &str,
    ) -> Option<&'a mut toml_edit::Table> {
        let section_key = if doc
            .as_table()
            .get("mcp_servers")
            .and_then(toml_edit::Item::as_table)
            .and_then(|table| table.get(server_name))
            .is_some()
        {
            Some("mcp_servers")
        } else if doc
            .as_table()
            .get("mcp_servers_disabled")
            .and_then(toml_edit::Item::as_table)
            .and_then(|table| table.get(server_name))
            .is_some()
        {
            Some("mcp_servers_disabled")
        } else {
            None
        }?;

        doc.as_table_mut()
            .get_mut(section_key)
            .and_then(toml_edit::Item::as_table_mut)
            .and_then(|section| section.get_mut(server_name))
            .and_then(toml_edit::Item::as_table_mut)
    }

    let Some(server_table) = find_server_table_mut(&mut doc, server_name) else {
        return Err(anyhow::anyhow!("MCP server '{server_name}' not found"));
    };

    let mut disabled_tools: Vec<String> = server_table
        .get("disabled_tools")
        .and_then(toml_edit::Item::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item.as_str().map(str::trim))
                .filter(|name| !name.is_empty())
                .map(std::string::ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut changed = false;
    if enabled {
        let prev_len = disabled_tools.len();
        disabled_tools.retain(|name| name != normalized_tool);
        changed = prev_len != disabled_tools.len();
    } else if !disabled_tools
        .iter()
        .any(|name| name == normalized_tool)
    {
        disabled_tools.push(normalized_tool.to_string());
        changed = true;
    }

    if !changed {
        return Ok(false);
    }

    disabled_tools.sort();
    disabled_tools.dedup();
    if disabled_tools.is_empty() {
        server_table.remove("disabled_tools");
    } else {
        let mut arr = toml_edit::Array::new();
        for tool in disabled_tools {
            arr.push(toml_edit::Value::from(tool));
        }
        server_table["disabled_tools"] = TomlItem::Value(toml_edit::Value::Array(arr));
    }

    std::fs::create_dir_all(code_home)?;
    let tmp = NamedTempFile::new_in(code_home)?;
    std::fs::write(tmp.path(), doc.to_string())?;
    tmp.persist(config_path)?;

    Ok(true)
}

/// Apply a single dotted-path override onto a TOML value.
fn env_path(var: &str) -> std::io::Result<Option<PathBuf>> {
    match std::env::var(var) {
        Ok(val) if !val.trim().is_empty() => {
            let canonical = PathBuf::from(val).canonicalize()?;
            Ok(Some(canonical))
        }
        _ => Ok(None),
    }
}

fn env_overrides_present() -> bool {
    matches!(std::env::var("CODE_HOME"), Ok(ref v) if !v.trim().is_empty())
        || matches!(std::env::var("CODEX_HOME"), Ok(ref v) if !v.trim().is_empty())
}

fn default_code_home_dir() -> Option<PathBuf> {
    let mut path = home_dir()?;
    path.push(".code");
    Some(path)
}

fn compute_legacy_code_home_dir() -> Option<PathBuf> {
    if env_overrides_present() {
        return None;
    }
    let home = home_dir()?;
    let candidate = home.join(".codex");
    if path_exists(&candidate) {
        Some(candidate)
    } else {
        None
    }
}

fn legacy_code_home_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        compute_legacy_code_home_dir()
    }

    #[cfg(not(test))]
    {
        static LEGACY: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
        LEGACY
            .get_or_init(compute_legacy_code_home_dir)
            .clone()
    }
}

fn path_exists(path: &Path) -> bool {
    std::fs::metadata(path).is_ok()
}

fn find_repo_dev_code_home() -> Option<PathBuf> {
    let cwd = std::env::current_dir().ok()?;
    for ancestor in cwd.ancestors() {
        // Limit this fallback to local source checkouts so regular users are
        // unaffected unless they intentionally run from the codebase.
        let repo_marker = ancestor.join("code-rs/Cargo.toml");
        if !path_exists(&repo_marker) {
            continue;
        }

        let dev_code_home = ancestor.join(".code");
        let dev_config = dev_code_home.join(CONFIG_TOML_FILE);
        if path_exists(&dev_config) {
            return Some(dev_code_home);
        }
    }
    None
}

/// Resolve the filesystem path used for *reading* Codex state that may live in
/// a legacy `~/.codex` directory. Writes should continue targeting `code_home`.
pub fn resolve_code_path_for_read(code_home: &Path, relative: &Path) -> PathBuf {
    let default_path = code_home.join(relative);

    if env_overrides_present() {
        return default_path;
    }

    if path_exists(&default_path) {
        return default_path;
    }

    if let Some(default_home) = default_code_home_dir()
        && default_home != code_home {
            return default_path;
        }

    if let Some(legacy) = legacy_code_home_dir() {
        let candidate = legacy.join(relative);
        if path_exists(&candidate) {
            return candidate;
        }
    }

    default_path
}

/// Returns the path to the Code/Codex configuration directory, which can be
/// specified by the `CODE_HOME` or `CODEX_HOME` environment variables. If not set,
/// defaults to `~/.code` for the fork.
///
/// - If `CODE_HOME` or `CODEX_HOME` is set, the value will be canonicalized and this
///   function will Err if the path does not exist.
/// - If neither is set, this function does not verify that the directory exists.
pub fn find_code_home() -> std::io::Result<PathBuf> {
    if let Some(path) = env_path("CODE_HOME")? {
        return Ok(path);
    }

    if let Some(path) = env_path("CODEX_HOME")? {
        return Ok(path);
    }

    if let Some(dev_code_home) = find_repo_dev_code_home() {
        return Ok(dev_code_home);
    }

    let home = home_dir().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Could not find home directory",
        )
    })?;

    let mut write_path = home;
    write_path.push(".code");
    Ok(write_path)
}

pub(crate) fn load_instructions(code_dir: Option<&Path>) -> Option<String> {
    let code_home = code_dir?;
    let read_path = resolve_code_path_for_read(code_home, Path::new("AGENTS.md"));

    let contents = match std::fs::read_to_string(&read_path) {
        Ok(s) => s,
        Err(_) => {
            if env_overrides_present() {
                return None;
            }
            let legacy_home = legacy_code_home_dir()?;
            let legacy_path = legacy_home.join("AGENTS.md");
            match std::fs::read_to_string(&legacy_path) {
                Ok(s) => s,
                Err(_) => return None,
            }
        }
    };

    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn read_override_file(
    path: Option<&PathBuf>,
    cwd: &Path,
    description: &str,
) -> std::io::Result<Option<String>> {
    let p = match path.as_ref() {
        None => return Ok(None),
        Some(p) => p,
    };

    // Resolve relative paths against the provided cwd to make CLI
    // overrides consistent regardless of where the process was launched
    // from.
    let full_path = if p.is_relative() {
        cwd.join(p)
    } else {
        p.to_path_buf()
    };

    let contents = std::fs::read_to_string(&full_path).map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("failed to read {description} {}: {e}", full_path.display()),
        )
    })?;

    let s = contents.trim().to_string();
    if s.is_empty() {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("{description} is empty: {}", full_path.display()),
        ))
    } else {
        Ok(Some(s))
    }
}

pub(crate) fn get_base_instructions(
    path: Option<&PathBuf>,
    cwd: &Path,
) -> std::io::Result<Option<String>> {
    read_override_file(path, cwd, "experimental instructions file")
}

pub(crate) fn get_compact_prompt_override(
    path: Option<&PathBuf>,
    cwd: &Path,
) -> std::io::Result<Option<String>> {
    read_override_file(path, cwd, "compact prompt override file")
}
