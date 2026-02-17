use std::collections::HashMap;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use code_common::CliConfigOverrides;
use code_core::config::Config;
use code_core::config::ConfigOverrides;
use code_core::config::find_code_home;
use code_core::config::load_global_mcp_servers;
use code_core::config::write_global_mcp_servers;
use code_core::config_types::McpServerConfig;
use code_core::config_types::McpServerTransportConfig;
use code_rmcp_client::delete_oauth_tokens;
use code_rmcp_client::perform_oauth_login;
use code_rmcp_client::supports_oauth_login;

/// Subcommands:
/// - `serve`  — run the MCP server on stdio
/// - `list`   — list configured servers (with `--json`)
/// - `get`    — show a single server (with `--json`)
/// - `add`    — add a server launcher entry to `~/.code/config.toml` (Code also reads legacy `~/.codex/config.toml`)
/// - `remove` — delete a server entry
/// - `login`  — authenticate with MCP server using OAuth
/// - `logout` — remove OAuth credentials for MCP server
#[derive(Debug, clap::Parser)]
pub struct McpCli {
    #[clap(flatten)]
    pub config_overrides: CliConfigOverrides,

    #[command(subcommand)]
    pub subcommand: McpSubcommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum McpSubcommand {
    List(ListArgs),

    Get(GetArgs),

    Add(AddArgs),

    Remove(RemoveArgs),

    Login(LoginArgs),

    Logout(LogoutArgs),
}

#[derive(Debug, clap::Parser)]
pub struct ListArgs {
    /// Output the configured servers as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, clap::Parser)]
pub struct GetArgs {
    /// Name of the MCP server to display.
    pub name: String,

    /// Output the server configuration as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, clap::Parser)]
pub struct AddArgs {
    /// Name for the MCP server configuration.
    pub name: String,

    /// URL of a remote MCP server.
    #[arg(long)]
    pub url: Option<String>,

    /// Optional bearer token to use with `--url` for static authentication.
    #[arg(long)]
    pub bearer_token: Option<String>,

    /// Optional environment variable to read for a bearer token.
    ///
    /// Only valid with `--url`.
    #[arg(long = "bearer-token-env-var", value_name = "ENV_VAR")]
    pub bearer_token_env_var: Option<String>,

    /// Environment variables to set when launching the server.
    ///
    /// Only valid with stdio servers.
    #[arg(long, value_parser = parse_env_pair, value_name = "KEY=VALUE")]
    pub env: Vec<(String, String)>,

    /// Command to launch the MCP server.
    #[arg(trailing_var_arg = true, num_args = 0..)]
    pub command: Vec<String>,
}

#[derive(Debug, clap::Parser)]
pub struct RemoveArgs {
    /// Name of the MCP server configuration to remove.
    pub name: String,
}

#[derive(Debug, clap::Parser)]
pub struct LoginArgs {
    /// Name of the MCP server to authenticate with OAuth.
    pub name: String,

    /// Comma-separated list of OAuth scopes to request.
    #[arg(long, value_delimiter = ',', value_name = "SCOPE,SCOPE")]
    pub scopes: Vec<String>,
}

#[derive(Debug, clap::Parser)]
pub struct LogoutArgs {
    /// Name of the MCP server to deauthenticate.
    pub name: String,
}

impl McpCli {
    pub async fn run(self) -> Result<()> {
        let McpCli {
            config_overrides,
            subcommand,
        } = self;

        match subcommand {
            McpSubcommand::List(args) => {
                run_list(&config_overrides, args)?;
            }
            McpSubcommand::Get(args) => {
                run_get(&config_overrides, args)?;
            }
            McpSubcommand::Add(args) => {
                run_add(&config_overrides, args).await?;
            }
            McpSubcommand::Remove(args) => {
                run_remove(&config_overrides, args)?;
            }
            McpSubcommand::Login(args) => {
                run_login(&config_overrides, args).await?;
            }
            McpSubcommand::Logout(args) => {
                run_logout(&config_overrides, args).await?;
            }
        }

        Ok(())
    }
}

fn build_mcp_transport_for_add(
    url: Option<String>,
    bearer_token: Option<String>,
    bearer_token_env_var: Option<String>,
    env: Option<HashMap<String, String>>,
    command: Vec<String>,
) -> Result<McpServerTransportConfig> {
    if let Some(url) = url {
        if !command.is_empty() {
            bail!("--url cannot be combined with a command");
        }
        if env.is_some() {
            bail!("--env is only supported for stdio servers");
        }
        if bearer_token.is_some() && bearer_token_env_var.is_some() {
            bail!("--bearer-token cannot be combined with --bearer-token-env-var");
        }
        return Ok(McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token,
            bearer_token_env_var,
            http_headers: None,
            env_http_headers: None,
        });
    }

    if bearer_token.is_some() || bearer_token_env_var.is_some() {
        bail!("--bearer-token and --bearer-token-env-var require --url");
    }

    let mut command_parts = command.into_iter();
    let command_bin = command_parts
        .next()
        .ok_or_else(|| anyhow!("command is required"))?;
    let command_args: Vec<String> = command_parts.collect();
    Ok(McpServerTransportConfig::Stdio {
        command: command_bin,
        args: command_args,
        env,
    })
}

async fn run_add(config_overrides: &CliConfigOverrides, add_args: AddArgs) -> Result<()> {
    // Validate any provided overrides even though they are not currently applied.
    let overrides = config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;
    let config = Config::load_with_cli_overrides(overrides, ConfigOverrides::default())
        .context("failed to load configuration")?;

    let AddArgs {
        name,
        url,
        bearer_token,
        bearer_token_env_var,
        env,
        command,
    } = add_args;

    validate_server_name(&name)?;

    let env_map = if env.is_empty() {
        None
    } else {
        let mut map = HashMap::new();
        for (key, value) in env {
            map.insert(key, value);
        }
        Some(map)
    };

    let code_home = config.code_home.clone();
    let mut servers = load_global_mcp_servers(&code_home)
        .with_context(|| format!("failed to load MCP servers from {}", code_home.display()))?;

    let transport =
        build_mcp_transport_for_add(url, bearer_token, bearer_token_env_var, env_map, command)?;

    let new_entry = McpServerConfig {
        transport: transport.clone(),
        startup_timeout_sec: None,
        tool_timeout_sec: None,
        disabled_tools: Vec::new(),
    };

    servers.insert(name.clone(), new_entry);

    write_global_mcp_servers(&code_home, &servers)
        .with_context(|| format!("failed to write MCP servers to {}", code_home.display()))?;

    println!("Added global MCP server '{name}'.");

    if let McpServerTransportConfig::StreamableHttp {
        url,
        bearer_token,
        bearer_token_env_var,
        http_headers,
        env_http_headers,
    } = &transport
        && bearer_token.is_none()
        && bearer_token_env_var.is_none()
    {
        match supports_oauth_login(url).await {
            Ok(true) => {
                println!("Detected OAuth support. Starting OAuth flow…");
                perform_oauth_login(
                    &code_home,
                    &name,
                    url,
                    config.mcp_oauth_credentials_store_mode,
                    http_headers.clone(),
                    env_http_headers.clone(),
                    &[],
                    config.mcp_oauth_callback_port,
                )
                .await?;
                println!("Successfully logged in.");
            }
            Ok(false) => {}
            Err(_) => println!(
                "MCP server may or may not require login. Run `codex mcp login {name}` to login."
            ),
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_with_url_defaults_to_streamable_http() {
        let transport = build_mcp_transport_for_add(
            Some("https://mcp.example.com/mcp".to_string()),
            None,
            None,
            None,
            Vec::new(),
        )
        .expect("transport");

        match transport {
            McpServerTransportConfig::StreamableHttp {
                url,
                bearer_token,
                bearer_token_env_var,
                ..
            } => {
                assert_eq!(url, "https://mcp.example.com/mcp");
                assert!(bearer_token.is_none());
                assert!(bearer_token_env_var.is_none());
            }
            _ => panic!("expected streamable http transport"),
        }
    }

    #[test]
    fn add_with_url_and_bearer_token_uses_streamable_http() {
        let transport = build_mcp_transport_for_add(
            Some("https://mcp.example.com/mcp".to_string()),
            Some("token".to_string()),
            None,
            None,
            Vec::new(),
        )
        .expect("transport");

        match transport {
            McpServerTransportConfig::StreamableHttp { url, bearer_token, .. } => {
                assert_eq!(url, "https://mcp.example.com/mcp");
                assert_eq!(bearer_token.as_deref(), Some("token"));
            }
            _ => panic!("expected streamable http transport"),
        }
    }

    #[test]
    fn add_with_url_and_bearer_token_env_var_uses_streamable_http() {
        let transport = build_mcp_transport_for_add(
            Some("https://mcp.example.com/mcp".to_string()),
            None,
            Some("MCP_BEARER_TOKEN".to_string()),
            None,
            Vec::new(),
        )
        .expect("transport");

        match transport {
            McpServerTransportConfig::StreamableHttp {
                url,
                bearer_token,
                bearer_token_env_var,
                ..
            } => {
                assert_eq!(url, "https://mcp.example.com/mcp");
                assert!(bearer_token.is_none());
                assert_eq!(bearer_token_env_var.as_deref(), Some("MCP_BEARER_TOKEN"));
            }
            _ => panic!("expected streamable http transport"),
        }
    }
}

fn run_remove(config_overrides: &CliConfigOverrides, remove_args: RemoveArgs) -> Result<()> {
    config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;

    let RemoveArgs { name } = remove_args;

    validate_server_name(&name)?;

    let code_home = find_code_home().context("failed to resolve CODEX_HOME")?;
    let mut servers = load_global_mcp_servers(&code_home)
        .with_context(|| format!("failed to load MCP servers from {}", code_home.display()))?;

    let removed = servers.remove(&name).is_some();

    if removed {
        write_global_mcp_servers(&code_home, &servers)
            .with_context(|| format!("failed to write MCP servers to {}", code_home.display()))?;
    }

    if removed {
        println!("Removed global MCP server '{name}'.");
    } else {
        println!("No MCP server named '{name}' found.");
    }

    Ok(())
}

async fn run_login(config_overrides: &CliConfigOverrides, login_args: LoginArgs) -> Result<()> {
    let overrides = config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;
    let config = Config::load_with_cli_overrides(overrides, ConfigOverrides::default())
        .context("failed to load configuration")?;

    let LoginArgs { name, scopes } = login_args;

    let Some(server) = config.mcp_servers.get(&name) else {
        bail!("No MCP server named '{name}' found.");
    };

    let (url, bearer_token, bearer_token_env_var, http_headers, env_http_headers) =
        match &server.transport {
            McpServerTransportConfig::StreamableHttp {
                url,
                bearer_token,
                bearer_token_env_var,
                http_headers,
                env_http_headers,
            } => (
                url.clone(),
                bearer_token.as_deref(),
                bearer_token_env_var.as_deref(),
                http_headers.clone(),
                env_http_headers.clone(),
            ),
            _ => bail!("OAuth login is only supported for streamable HTTP servers."),
        };

    if bearer_token.is_some() || bearer_token_env_var.is_some() {
        bail!(
            "OAuth login is not supported when a bearer token is configured. Remove bearer_token/bearer_token_env_var from the server config first."
        );
    }

    perform_oauth_login(
        &config.code_home,
        &name,
        &url,
        config.mcp_oauth_credentials_store_mode,
        http_headers,
        env_http_headers,
        &scopes,
        config.mcp_oauth_callback_port,
    )
    .await?;

    println!("Successfully logged in to MCP server '{name}'.");
    Ok(())
}

async fn run_logout(config_overrides: &CliConfigOverrides, logout_args: LogoutArgs) -> Result<()> {
    let overrides = config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;
    let config = Config::load_with_cli_overrides(overrides, ConfigOverrides::default())
        .context("failed to load configuration")?;

    let LogoutArgs { name } = logout_args;

    let server = config.mcp_servers.get(&name).ok_or_else(|| {
        anyhow!("No MCP server named '{name}' found in configuration.")
    })?;

    let url = match &server.transport {
        McpServerTransportConfig::StreamableHttp { url, .. } => url.clone(),
        _ => bail!("OAuth logout is only supported for streamable_http transports."),
    };

    match delete_oauth_tokens(
        &config.code_home,
        &name,
        &url,
        config.mcp_oauth_credentials_store_mode,
    ) {
        Ok(true) => println!("Removed OAuth credentials for '{name}'."),
        Ok(false) => println!("No OAuth credentials stored for '{name}'."),
        Err(err) => return Err(anyhow!("failed to delete OAuth credentials: {err}")),
    }

    Ok(())
}

fn run_list(config_overrides: &CliConfigOverrides, list_args: ListArgs) -> Result<()> {
    let overrides = config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;
    let config = Config::load_with_cli_overrides(overrides, ConfigOverrides::default())
        .context("failed to load configuration")?;

    let mut entries: Vec<_> = config.mcp_servers.iter().collect();
    entries.sort_by(|(a, _), (b, _)| a.cmp(b));

    if list_args.json {
        let json_entries: Vec<_> = entries
            .into_iter()
            .map(|(name, cfg)| {
                let transport = match &cfg.transport {
                    McpServerTransportConfig::Stdio { command, args, env } => serde_json::json!({
                        "type": "stdio",
                        "command": command,
                        "args": args,
                        "env": env,
                    }),
                    McpServerTransportConfig::StreamableHttp {
                        url,
                        bearer_token,
                        bearer_token_env_var,
                        http_headers,
                        env_http_headers,
                    } => {
                        serde_json::json!({
                            "type": "streamable_http",
                            "url": url,
                            "bearer_token": bearer_token,
                            "bearer_token_env_var": bearer_token_env_var,
                            "http_headers": http_headers,
                            "env_http_headers": env_http_headers,
                        })
                    }
                };

                serde_json::json!({
                    "name": name,
                    "transport": transport,
                    "startup_timeout_sec": cfg.startup_timeout_sec.map(|d| d.as_secs_f64()),
                    "tool_timeout_sec": cfg.tool_timeout_sec.map(|d| d.as_secs_f64()),
                })
            })
            .collect();
        let output = serde_json::to_string_pretty(&json_entries)?;
        println!("{output}");
        return Ok(());
    }

    if entries.is_empty() {
        println!("No MCP servers configured yet. Try `codex mcp add my-tool -- my-command`.");
        return Ok(());
    }

    let mut stdio_rows: Vec<[String; 4]> = Vec::new();
    let mut http_rows: Vec<[String; 3]> = Vec::new();

    for (name, cfg) in entries {
        match &cfg.transport {
            McpServerTransportConfig::Stdio { command, args, env } => {
                let args_display = if args.is_empty() {
                    "-".to_string()
                } else {
                    args.join(" ")
                };
                let env_display = match env.as_ref() {
                    None => "-".to_string(),
                    Some(map) if map.is_empty() => "-".to_string(),
                    Some(map) => {
                        let mut pairs: Vec<_> = map.iter().collect();
                        pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                        pairs
                            .into_iter()
                            .map(|(k, v)| format!("{k}={v}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                };
                stdio_rows.push([name.clone(), command.clone(), args_display, env_display]);
            }
            McpServerTransportConfig::StreamableHttp {
                url,
                bearer_token,
                bearer_token_env_var,
                ..
            } => {
                let has_bearer = if bearer_token.is_some() || bearer_token_env_var.is_some() {
                    "True"
                } else {
                    "False"
                };
                http_rows.push([name.clone(), url.clone(), has_bearer.into()]);
            }
        }
    }

    if !stdio_rows.is_empty() {
        let mut widths = ["Name".len(), "Command".len(), "Args".len(), "Env".len()];
        for row in &stdio_rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.len());
            }
        }

        println!(
            "{:<name_w$}  {:<cmd_w$}  {:<args_w$}  {:<env_w$}",
            "Name",
            "Command",
            "Args",
            "Env",
            name_w = widths[0],
            cmd_w = widths[1],
            args_w = widths[2],
            env_w = widths[3],
        );

        for row in &stdio_rows {
            println!(
                "{:<name_w$}  {:<cmd_w$}  {:<args_w$}  {:<env_w$}",
                row[0],
                row[1],
                row[2],
                row[3],
                name_w = widths[0],
                cmd_w = widths[1],
                args_w = widths[2],
                env_w = widths[3],
            );
        }
    }

    if !stdio_rows.is_empty() && !http_rows.is_empty() {
        println!();
    }

    if !http_rows.is_empty() {
        let mut widths = ["Name".len(), "Url".len(), "Has Bearer Token".len()];
        for row in &http_rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.len());
            }
        }

        println!(
            "{:<name_w$}  {:<url_w$}  {:<token_w$}",
            "Name",
            "Url",
            "Has Bearer Token",
            name_w = widths[0],
            url_w = widths[1],
            token_w = widths[2],
        );

        for row in &http_rows {
            println!(
                "{:<name_w$}  {:<url_w$}  {:<token_w$}",
                row[0],
                row[1],
                row[2],
                name_w = widths[0],
                url_w = widths[1],
                token_w = widths[2],
            );
        }
    }

    Ok(())
}

fn run_get(config_overrides: &CliConfigOverrides, get_args: GetArgs) -> Result<()> {
    let overrides = config_overrides.parse_overrides().map_err(|e| anyhow!(e))?;
    let config = Config::load_with_cli_overrides(overrides, ConfigOverrides::default())
        .context("failed to load configuration")?;

    let Some(server) = config.mcp_servers.get(&get_args.name) else {
        bail!("No MCP server named '{name}' found.", name = get_args.name);
    };

    if get_args.json {
        let transport = match &server.transport {
            McpServerTransportConfig::Stdio { command, args, env } => serde_json::json!({
                "type": "stdio",
                "command": command,
                "args": args,
                "env": env,
            }),
            McpServerTransportConfig::StreamableHttp {
                url,
                bearer_token,
                bearer_token_env_var,
                http_headers,
                env_http_headers,
            } => serde_json::json!({
                "type": "streamable_http",
                "url": url,
                "bearer_token": bearer_token,
                "bearer_token_env_var": bearer_token_env_var,
                "http_headers": http_headers,
                "env_http_headers": env_http_headers,
            }),
        };
        let output = serde_json::to_string_pretty(&serde_json::json!({
            "name": get_args.name,
            "transport": transport,
            "startup_timeout_sec": server.startup_timeout_sec.map(|d| d.as_secs_f64()),
            "tool_timeout_sec": server.tool_timeout_sec.map(|d| d.as_secs_f64()),
        }))?;
        println!("{output}");
        return Ok(());
    }

    println!("{}", get_args.name);
    match &server.transport {
        McpServerTransportConfig::Stdio { command, args, env } => {
            println!("  transport: stdio");
            println!("  command: {command}");
            let args_display = if args.is_empty() {
                "-".to_string()
            } else {
                args.join(" ")
            };
            println!("  args: {args_display}");
            let env_display = match env.as_ref() {
                None => "-".to_string(),
                Some(map) if map.is_empty() => "-".to_string(),
                Some(map) => {
                    let mut pairs: Vec<_> = map.iter().collect();
                    pairs.sort_by(|(a, _), (b, _)| a.cmp(b));
                    pairs
                        .into_iter()
                        .map(|(k, v)| format!("{k}={v}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            };
            println!("  env: {env_display}");
        }
        McpServerTransportConfig::StreamableHttp {
            url,
            bearer_token,
            bearer_token_env_var,
            http_headers,
            env_http_headers,
        } => {
            println!("  transport: streamable_http");
            println!("  url: {url}");
            let token_display = bearer_token
                .as_ref()
                .map(|_| "<redacted>".to_string())
                .unwrap_or_else(|| "-".to_string());
            println!("  bearer_token: {token_display}");
            let env_var_display = bearer_token_env_var.as_deref().unwrap_or("-");
            println!("  bearer_token_env_var: {env_var_display}");
            let headers_display = http_headers
                .as_ref()
                .map(|headers| format!("{} header(s)", headers.len()))
                .unwrap_or_else(|| "-".to_string());
            println!("  http_headers: {headers_display}");
            let env_headers_display = env_http_headers
                .as_ref()
                .map(|headers| format!("{} header(s)", headers.len()))
                .unwrap_or_else(|| "-".to_string());
            println!("  env_http_headers: {env_headers_display}");
        }
    }
    if let Some(timeout) = server.startup_timeout_sec {
        println!("  startup_timeout_sec: {:.3}", timeout.as_secs_f64());
    }
    if let Some(timeout) = server.tool_timeout_sec {
        println!("  tool_timeout_sec: {:.3}", timeout.as_secs_f64());
    }
    println!("  remove: codex mcp remove {}", get_args.name);

    Ok(())
}

fn parse_env_pair(raw: &str) -> Result<(String, String), String> {
    let mut parts = raw.splitn(2, '=');
    let key = parts
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "environment entries must be in KEY=VALUE form".to_string())?;
    let value = parts
        .next()
        .map(str::to_string)
        .ok_or_else(|| "environment entries must be in KEY=VALUE form".to_string())?;

    Ok((key.to_string(), value))
}

fn validate_server_name(name: &str) -> Result<()> {
    let is_valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');

    if is_valid {
        Ok(())
    } else {
        bail!("invalid server name '{name}' (use letters, numbers, '-', '_')");
    }
}
