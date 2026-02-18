//! Connection manager for Model Context Protocol (MCP) servers.
//!
//! The [`McpConnectionManager`] owns one [`code_rmcp_client::RmcpClient`] per
//! configured server (keyed by the *server name*). It offers convenience
//! helpers to query the available tools across *all* servers and returns them
//! in a single aggregated map using the fully-qualified tool name
//! `"<server><MCP_TOOL_NAME_DELIMITER><tool>"` as the key.

use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::RwLock as StdRwLock;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use code_protocol::protocol::McpAuthStatus;
use code_rmcp_client::OAuthCredentialsStoreMode;
use code_rmcp_client::RmcpClient;
use futures::future::join_all;
use mcp_types::ClientCapabilities;
use mcp_types::Implementation;
use mcp_types::Tool;

use serde_json::json;
use sha1::Digest;
use sha1::Sha1;
use tokio::sync::RwLock as TokioRwLock;
use tokio::task::JoinSet;
use tracing::info;
use tracing::warn;

use crate::config_types::McpServerConfig;
use crate::config_types::McpServerTransportConfig;
use crate::protocol::{McpServerFailure, McpServerFailurePhase};

/// Delimiter used to separate the server name from the tool name in a fully
/// qualified tool name.
///
/// OpenAI requires tool names to conform to `^[a-zA-Z0-9_-]+$`, so we must
/// choose a delimiter from this character set.
const MCP_TOOL_NAME_DELIMITER: &str = "__";
const MAX_TOOL_NAME_LENGTH: usize = 64;

/// The Responses API requires tool names to match `^[a-zA-Z0-9_-]+$`.
/// MCP server/tool names are user-controlled, so sanitize the fully-qualified
/// name we expose to the model by replacing any disallowed character with `_`.
fn sanitize_responses_api_tool_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            sanitized.push(c);
        } else {
            sanitized.push('_');
        }
    }

    if sanitized.is_empty() {
        "_".to_string()
    } else {
        sanitized
    }
}

fn sha1_hex(s: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(s.as_bytes());
    let sha1 = hasher.finalize();
    format!("{sha1:x}")
}

/// Append a deterministic SHA1 suffix while keeping the name within the maximum length.
fn append_sha1_suffix(base: &str, raw: &str) -> String {
    let sha1_str = sha1_hex(raw);
    let prefix_len = MAX_TOOL_NAME_LENGTH.saturating_sub(sha1_str.len());
    let prefix = if base.len() > prefix_len {
        &base[..prefix_len]
    } else {
        base
    };
    format!("{prefix}{sha1_str}")
}

fn resolve_streamable_http_bearer_token(
    server_name: &str,
    bearer_token: Option<String>,
    bearer_token_env_var: Option<&str>,
) -> Result<Option<String>> {
    let Some(env_var) = bearer_token_env_var else {
        return Ok(bearer_token);
    };

    match std::env::var(env_var) {
        Ok(value) => {
            if value.is_empty() {
                Err(anyhow!(
                    "Environment variable {env_var} for MCP server '{server_name}' is empty"
                ))
            } else {
                Ok(Some(value))
            }
        }
        Err(std::env::VarError::NotPresent) => Err(anyhow!(
            "Environment variable {env_var} for MCP server '{server_name}' is not set"
        )),
        Err(std::env::VarError::NotUnicode(_)) => Err(anyhow!(
            "Environment variable {env_var} for MCP server '{server_name}' contains invalid Unicode"
        )),
    }
}

/// Default timeout for initializing MCP server & initially listing tools.
const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Map that holds startup or tool-list errors for MCP servers.
pub type ClientStartErrors = HashMap<String, McpServerFailure>;

fn qualify_tools(tools: Vec<ToolInfo>) -> HashMap<String, ToolInfo> {
    let mut used_names = HashSet::new();
    let mut seen_raw_names = HashSet::new();
    let mut qualified_tools = HashMap::new();
    for tool in tools {
        let qualified_name_raw = format!(
            "{}{}{}",
            tool.server_name, MCP_TOOL_NAME_DELIMITER, tool.tool_name
        );
        if !seen_raw_names.insert(qualified_name_raw.clone()) {
            warn!("skipping duplicated tool {}", qualified_name_raw);
            continue;
        }

        // Start from a "pretty" name (sanitized), then deterministically disambiguate on
        // collisions by appending a hash of the *raw* (unsanitized) qualified name. This
        // ensures tools like `foo.bar` and `foo_bar` don't collapse to the same key.
        let mut qualified_name = sanitize_responses_api_tool_name(&qualified_name_raw);

        // Enforce length constraints early; use the raw name for the hash input so the
        // output remains stable even when sanitization changes.
        if qualified_name.len() > MAX_TOOL_NAME_LENGTH {
            qualified_name = append_sha1_suffix(&qualified_name, &qualified_name_raw);
        }

        if used_names.contains(&qualified_name) {
            let disambiguated_name = append_sha1_suffix(&qualified_name, &qualified_name_raw);
            if used_names.contains(&disambiguated_name) {
                warn!("skipping duplicated tool {}", disambiguated_name);
                continue;
            }
            qualified_name = disambiguated_name;
        }

        used_names.insert(qualified_name.clone());
        qualified_tools.insert(qualified_name, tool);
    }

    qualified_tools
}

struct ToolInfo {
    server_name: String,
    tool_name: String,
    tool: Tool,
}

#[derive(Clone)]
struct ManagedClient {
    client: McpClientAdapter,
    startup_timeout: Duration,
    tool_timeout: Option<Duration>,
}

#[derive(Clone)]
enum McpClientAdapter {
    Rmcp(Arc<RmcpClient>),
}

struct StreamableHttpClientArgs<'a> {
    code_home: PathBuf,
    server_name: &'a str,
    url: String,
    bearer_token: Option<String>,
    http_headers: Option<HashMap<String, String>>,
    env_http_headers: Option<HashMap<String, String>>,
    oauth_store_mode: OAuthCredentialsStoreMode,
    params: mcp_types::InitializeRequestParams,
    startup_timeout: Duration,
}

impl McpClientAdapter {
    async fn new_stdio_client(
        program: OsString,
        args: Vec<OsString>,
        env: Option<HashMap<String, String>>,
        params: mcp_types::InitializeRequestParams,
        startup_timeout: Duration,
    ) -> Result<Self> {
        tracing::debug!(
            "new_stdio_client program: {program:?} args: {args:?} env: {env:?} params: {params:?} startup_timeout: {startup_timeout:?}"
        );
        let client = Arc::new(RmcpClient::new_stdio_client(program, args, env).await?);
        client.initialize(params, Some(startup_timeout)).await?;
        Ok(McpClientAdapter::Rmcp(client))
    }

    async fn new_streamable_http_client(args: StreamableHttpClientArgs<'_>) -> Result<Self> {
        let StreamableHttpClientArgs {
            code_home,
            server_name,
            url,
            bearer_token,
            http_headers,
            env_http_headers,
            oauth_store_mode,
            params,
            startup_timeout,
        } = args;
        let client = Arc::new(
            RmcpClient::new_streamable_http_client(
                code_home,
                server_name,
                &url,
                bearer_token,
                http_headers,
                env_http_headers,
                oauth_store_mode,
            )
            .await?,
        );
        client.initialize(params, Some(startup_timeout)).await?;
        Ok(McpClientAdapter::Rmcp(client))
    }

    async fn list_tools(
        &self,
        params: Option<mcp_types::ListToolsRequestParams>,
        timeout: Option<Duration>,
    ) -> Result<mcp_types::ListToolsResult> {
        match self {
            McpClientAdapter::Rmcp(client) => client.list_tools(params, timeout).await,
        }
    }

    async fn call_tool(
        &self,
        name: String,
        arguments: Option<serde_json::Value>,
        timeout: Option<Duration>,
    ) -> Result<mcp_types::CallToolResult> {
        match self {
            McpClientAdapter::Rmcp(client) => client.call_tool(name, arguments, timeout).await,
        }
    }

    async fn into_shutdown(self) {
        match self {
            McpClientAdapter::Rmcp(client) => {
                client.shutdown().await;
            }
        }
    }
}

/// A thin wrapper around a set of running [`RmcpClient`] instances.
#[derive(Default)]
pub struct McpConnectionManager {
    /// Directory containing all Code state (used for MCP OAuth token storage).
    code_home: PathBuf,
    mcp_oauth_credentials_store_mode: OAuthCredentialsStoreMode,
    server_transports: StdRwLock<HashMap<String, McpServerTransportConfig>>,

    /// Server-name -> client instance.
    ///
    /// The server name originates from the keys of the `mcp_servers` map in
    /// the user configuration.
    clients: TokioRwLock<HashMap<String, ManagedClient>>,

    /// Fully qualified tool name -> tool instance.
    tools: StdRwLock<HashMap<String, ToolInfo>>,
    excluded_tools: StdRwLock<HashSet<(String, String)>>,
    server_names: StdRwLock<Vec<String>>,
    failures: StdRwLock<HashMap<String, McpServerFailure>>,
}

impl McpConnectionManager {
    /// Spawn a [`RmcpClient`] for each configured server.
    ///
    /// * `mcp_servers` – Map loaded from the user configuration where *keys*
    ///   are human-readable server identifiers and *values* are the spawn
    ///   instructions.
    ///
    /// Servers that fail to start or list tools are reported in `ClientStartErrors`:
    /// the user should be informed about these errors.
    pub async fn new(
        code_home: PathBuf,
        mcp_oauth_credentials_store_mode: OAuthCredentialsStoreMode,
        mcp_servers: HashMap<String, McpServerConfig>,
        excluded_tools: HashSet<(String, String)>,
    ) -> Result<(Self, ClientStartErrors)> {
        // Early exit if no servers are configured.
        if mcp_servers.is_empty() {
            return Ok((
                Self {
                    code_home,
                    mcp_oauth_credentials_store_mode,
                    server_transports: StdRwLock::new(HashMap::new()),
                    clients: TokioRwLock::new(HashMap::new()),
                    tools: StdRwLock::new(HashMap::new()),
                    excluded_tools: StdRwLock::new(excluded_tools),
                    server_names: StdRwLock::new(Vec::new()),
                    failures: StdRwLock::new(HashMap::new()),
                },
                ClientStartErrors::default(),
            ));
        }

        // Launch all configured servers concurrently.
        let mut join_set = JoinSet::new();
        let mut errors = ClientStartErrors::new();
        let mut server_transports = HashMap::with_capacity(mcp_servers.len());

        for (server_name, cfg) in mcp_servers {
            // Validate server name before spawning
            if !is_valid_mcp_server_name(&server_name) {
                let message = format!(
                    "invalid server name '{server_name}': must match pattern ^[a-zA-Z0-9_-]+$"
                );
                errors.insert(
                    server_name,
                    McpServerFailure { phase: McpServerFailurePhase::Start, message },
                );
                continue;
            }

            server_transports.insert(server_name.clone(), cfg.transport.clone());

            let startup_timeout = cfg.startup_timeout_sec.unwrap_or(DEFAULT_STARTUP_TIMEOUT);
            let tool_timeout = cfg.tool_timeout_sec;
            let code_home_for_server = code_home.clone();
            let oauth_store_mode = mcp_oauth_credentials_store_mode;

            join_set.spawn(async move {
                let McpServerConfig { transport, .. } = cfg;
                let server_name_for_error = server_name.clone();
                let params = mcp_types::InitializeRequestParams {
                    capabilities: ClientCapabilities {
                        experimental: None,
                        roots: None,
                        sampling: None,
                        // https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation#capabilities
                        // indicates this should be an empty object.
                        elicitation: Some(json!({})),
                    },
                    client_info: Implementation {
                        name: "codex-mcp-client".to_owned(),
                        version: env!("CARGO_PKG_VERSION").to_owned(),
                        title: Some("Codex".into()),
                        // This field is used by Codex when it is an MCP
                        // server: it should not be used when Codex is
                        // an MCP client.
                        user_agent: None,
                    },
                    protocol_version: mcp_types::MCP_SCHEMA_VERSION.to_owned(),
                };

                let client = match transport {
                    McpServerTransportConfig::Stdio { command, args, env } => {
                        let command_for_error = command.clone();
                        let args_for_error = args.clone();
                        let command_os: OsString = command.into();
                        let args_os: Vec<OsString> = args.into_iter().map(Into::into).collect();
                        McpClientAdapter::new_stdio_client(
                            command_os,
                            args_os,
                            env,
                            params.clone(),
                            startup_timeout,
                        )
                        .await
                        .with_context(|| {
                            if args_for_error.is_empty() {
                                format!(
                                    "failed to spawn MCP server `{server_name_for_error}` using command `{command_for_error}`"
                                )
                            } else {
                                format!(
                                    "failed to spawn MCP server `{server_name_for_error}` using command `{command_for_error}` with args {args_for_error:?}"
                                )
                            }
                        })
                    }
                    McpServerTransportConfig::StreamableHttp {
                        url,
                        bearer_token,
                        bearer_token_env_var,
                        http_headers,
                        env_http_headers,
                    } => {
                        match resolve_streamable_http_bearer_token(
                            &server_name_for_error,
                            bearer_token,
                            bearer_token_env_var.as_deref(),
                        ) {
                            Ok(bearer_token) => {
                                McpClientAdapter::new_streamable_http_client(StreamableHttpClientArgs {
                                    code_home: code_home_for_server,
                                    server_name: &server_name_for_error,
                                    url,
                                    bearer_token,
                                    http_headers,
                                    env_http_headers,
                                    oauth_store_mode,
                                    params,
                                    startup_timeout,
                                })
                                .await
                            }
                            Err(err) => Err(err),
                        }
                    }
                }
                .map(|c| (c, startup_timeout));

                ((server_name, tool_timeout), client)
            });
        }

        let mut clients: HashMap<String, ManagedClient> = HashMap::with_capacity(join_set.len());

        while let Some(res) = join_set.join_next().await {
            let ((server_name, tool_timeout), client_res) = match res {
                Ok(result) => result,
                Err(e) => {
                    warn!("Task panic when starting MCP server: {e:#}");
                    continue;
                }
            };

            match client_res {
                Ok((client, startup_timeout)) => {
                    clients.insert(
                        server_name,
                        ManagedClient {
                            client,
                            startup_timeout,
                            tool_timeout,
                        },
                    );
                }
                Err(e) => {
                    let message = format!("server '{server_name}': {e:#}");
                    errors.insert(
                        server_name,
                        McpServerFailure {
                            phase: McpServerFailurePhase::Start,
                            message,
                        },
                    );
                }
            }
        }

        let all_tools = list_all_tools(&clients, &excluded_tools, &mut errors).await;

        let tools = qualify_tools(all_tools);

        let mut server_names: Vec<String> = clients.keys().cloned().collect();
        server_names.sort();
        let failures = errors.clone();

        Ok((
            Self {
            code_home,
            mcp_oauth_credentials_store_mode,
            server_transports: StdRwLock::new(server_transports),
            clients: TokioRwLock::new(clients),
            tools: StdRwLock::new(tools),
            excluded_tools: StdRwLock::new(excluded_tools),
            server_names: StdRwLock::new(server_names),
            failures: StdRwLock::new(failures),
        },
            errors,
        ))
    }

    fn server_transports_read(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, HashMap<String, McpServerTransportConfig>> {
        match self.server_transports.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("MCP server transports lock poisoned; recovering inner state");
                poisoned.into_inner()
            }
        }
    }

    fn server_transports_write(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, HashMap<String, McpServerTransportConfig>> {
        match self.server_transports.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("MCP server transports lock poisoned; recovering inner state");
                poisoned.into_inner()
            }
        }
    }

    fn server_names_read(&self) -> std::sync::RwLockReadGuard<'_, Vec<String>> {
        match self.server_names.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("MCP server names lock poisoned; recovering inner state");
                poisoned.into_inner()
            }
        }
    }

    fn server_names_write(&self) -> std::sync::RwLockWriteGuard<'_, Vec<String>> {
        match self.server_names.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("MCP server names lock poisoned; recovering inner state");
                poisoned.into_inner()
            }
        }
    }

    fn tools_read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, ToolInfo>> {
        match self.tools.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("MCP tools lock poisoned; recovering inner state");
                poisoned.into_inner()
            }
        }
    }

    fn tools_write(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, ToolInfo>> {
        match self.tools.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("MCP tools lock poisoned; recovering inner state");
                poisoned.into_inner()
            }
        }
    }

    fn excluded_tools_read(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, HashSet<(String, String)>> {
        match self.excluded_tools.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("MCP excluded-tools lock poisoned; recovering inner state");
                poisoned.into_inner()
            }
        }
    }

    fn excluded_tools_write(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, HashSet<(String, String)>> {
        match self.excluded_tools.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("MCP excluded-tools lock poisoned; recovering inner state");
                poisoned.into_inner()
            }
        }
    }

    fn failures_read(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, McpServerFailure>> {
        match self.failures.read() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("MCP failures lock poisoned; recovering inner state");
                poisoned.into_inner()
            }
        }
    }

    fn failures_write(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, HashMap<String, McpServerFailure>> {
        match self.failures.write() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!("MCP failures lock poisoned; recovering inner state");
                poisoned.into_inner()
            }
        }
    }

    /// Returns a single map that contains **all** tools. Each key is the
    /// fully-qualified name for the tool.
    pub fn list_all_tools(&self) -> HashMap<String, Tool> {
        self.tools_read()
            .iter()
            .map(|(name, tool)| (name.clone(), tool.tool.clone()))
            .collect()
    }

    pub fn list_all_tools_with_server_names(&self) -> Vec<(String, String, Tool)> {
        self.tools_read()
            .iter()
            .map(|(qualified_name, tool_info)| {
                (
                    qualified_name.clone(),
                    tool_info.server_name.clone(),
                    tool_info.tool.clone(),
                )
            })
            .collect()
    }

    pub fn list_tools_by_server(&self) -> HashMap<String, Vec<String>> {
        let mut tools_by_server: HashMap<String, Vec<String>> = HashMap::new();
        for tool in self.tools_read().values() {
            tools_by_server
                .entry(tool.server_name.clone())
                .or_default()
                .push(tool.tool_name.clone());
        }

        for server_name in self.server_names_read().iter() {
            tools_by_server.entry(server_name.clone()).or_default();
        }

        for tools in tools_by_server.values_mut() {
            tools.sort();
            tools.dedup();
        }

        tools_by_server
    }

    pub fn list_disabled_tools_by_server(&self) -> HashMap<String, Vec<String>> {
        let mut disabled_by_server: HashMap<String, Vec<String>> = HashMap::new();
        for (server, tool) in self.excluded_tools_read().iter() {
            disabled_by_server
                .entry(server.clone())
                .or_default()
                .push(tool.clone());
        }

        for server_name in self.server_names_read().iter() {
            disabled_by_server.entry(server_name.clone()).or_default();
        }

        for tools in disabled_by_server.values_mut() {
            tools.sort();
            tools.dedup();
        }

        disabled_by_server
    }

    pub fn list_server_failures(&self) -> HashMap<String, McpServerFailure> {
        self.failures_read().clone()
    }

    pub async fn list_auth_statuses(&self) -> HashMap<String, McpAuthStatus> {
        let store_mode = self.mcp_oauth_credentials_store_mode;
        let code_home = self.code_home.clone();

        let join = {
            let transports = self.server_transports_read();
            join_all(transports.iter().map(|(server_name, transport)| {
                let code_home = code_home.clone();
                let server_name = server_name.clone();
                let transport = transport.clone();
                async move {
                let status = match transport {
                    McpServerTransportConfig::Stdio { .. } => McpAuthStatus::Unsupported,
                    McpServerTransportConfig::StreamableHttp {
                        url,
                        bearer_token,
                        bearer_token_env_var,
                        http_headers,
                        env_http_headers,
                    } => match code_rmcp_client::determine_streamable_http_auth_status(
                        &code_home,
                        &server_name,
                        &url,
                        bearer_token.as_deref(),
                        bearer_token_env_var.as_deref(),
                        http_headers,
                        env_http_headers,
                        store_mode,
                    )
                    .await
                    {
                        Ok(status) => status,
                        Err(error) => {
                            warn!(
                                "failed to determine auth status for MCP server `{server_name}`: {error:#}"
                            );
                            McpAuthStatus::Unsupported
                        }
                    },
                };
                (server_name, status)
                }
            }))
        };

        join.await.into_iter().collect()
    }

    /// Start an MCP server on-demand when it is needed for the current session.
    ///
    /// Returns `true` when a new client was started, `false` when the server was already running.
    pub async fn ensure_server_started(
        &self,
        server_name: &str,
        cfg: &McpServerConfig,
    ) -> Result<bool> {
        if !is_valid_mcp_server_name(server_name) {
            return Err(anyhow!(
                "invalid server name '{server_name}': must match pattern ^[a-zA-Z0-9_-]+$"
            ));
        }

        {
            let clients = self.clients.read().await;
            if clients.contains_key(server_name) {
                return Ok(false);
            }
        }

        let cfg = cfg.clone();
        let startup_timeout = cfg.startup_timeout_sec.unwrap_or(DEFAULT_STARTUP_TIMEOUT);
        let tool_timeout = cfg.tool_timeout_sec;

        let params = mcp_types::InitializeRequestParams {
            capabilities: ClientCapabilities {
                experimental: None,
                roots: None,
                sampling: None,
                elicitation: Some(json!({})),
            },
            client_info: Implementation {
                name: "codex-mcp-client".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                title: Some("Codex".into()),
                user_agent: None,
            },
            protocol_version: mcp_types::MCP_SCHEMA_VERSION.to_owned(),
        };

        let transport = cfg.transport.clone();
        {
            let mut transports = self.server_transports_write();
            transports.insert(server_name.to_string(), transport.clone());
        }

        let code_home = self.code_home.clone();
        let oauth_store_mode = self.mcp_oauth_credentials_store_mode;

        let client = match transport {
            McpServerTransportConfig::Stdio { command, args, env } => {
                let command_os: OsString = command.into();
                let args_os: Vec<OsString> = args.into_iter().map(Into::into).collect();
                McpClientAdapter::new_stdio_client(
                    command_os,
                    args_os,
                    env,
                    params,
                    startup_timeout,
                )
                .await?
            }
            McpServerTransportConfig::StreamableHttp {
                url,
                bearer_token,
                bearer_token_env_var,
                http_headers,
                env_http_headers,
            } => {
                let bearer_token = resolve_streamable_http_bearer_token(
                    server_name,
                    bearer_token,
                    bearer_token_env_var.as_deref(),
                )?;
                McpClientAdapter::new_streamable_http_client(StreamableHttpClientArgs {
                    code_home,
                    server_name,
                    url,
                    bearer_token,
                    http_headers,
                    env_http_headers,
                    oauth_store_mode,
                    params,
                    startup_timeout,
                })
                .await?
            }
        };

        let managed = ManagedClient {
            client,
            startup_timeout,
            tool_timeout,
        };

        let inserted = {
            let mut clients = self.clients.write().await;
            if clients.contains_key(server_name) {
                false
            } else {
                clients.insert(server_name.to_string(), managed.clone());
                true
            }
        };

        if !inserted {
            managed.shutdown().await;
            return Ok(false);
        }

        {
            let mut names = self.server_names_write();
            if !names.iter().any(|name| name.eq_ignore_ascii_case(server_name)) {
                names.push(server_name.to_string());
                names.sort_by_key(|name| name.to_ascii_lowercase());
                names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            }
        }

        {
            let mut failures = self.failures_write();
            failures.remove(server_name);
        }

        self.refresh_tools().await;
        Ok(true)
    }

    /// Invoke the tool indicated by the (server, tool) pair.
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<serde_json::Value>,
        timeout_override: Option<Duration>,
    ) -> Result<mcp_types::CallToolResult> {
        let (client, timeout) = {
            let clients = self.clients.read().await;
            let managed = clients
                .get(server)
                .ok_or_else(|| anyhow!("unknown MCP server '{server}'"))?;
            let timeout = timeout_override.or(managed.tool_timeout);
            (managed.client.clone(), timeout)
        };

        client
            .call_tool(tool.to_string(), arguments, timeout)
            .await
            .with_context(|| format!("tool call failed for `{server}/{tool}`"))
    }

    pub fn parse_tool_name(&self, tool_name: &str) -> Option<(String, String)> {
        self.tools_read()
            .get(tool_name)
            .map(|tool| (tool.server_name.clone(), tool.tool_name.clone()))
    }

    pub async fn shutdown_all(&self) {
        let mut clients = self.clients.write().await;
        let drained: Vec<ManagedClient> = clients.drain().map(|(_, managed)| managed).collect();
        drop(clients);

        for managed in drained {
            managed.shutdown().await;
        }
    }

    pub async fn refresh_tools(&self) {
        let clients_snapshot = {
            let clients = self.clients.read().await;
            clients.clone()
        };
        let excluded_tools = self.excluded_tools_read().clone();
        let mut errors = HashMap::new();
        let all_tools = list_all_tools(&clients_snapshot, &excluded_tools, &mut errors).await;
        let tools = qualify_tools(all_tools);
        *self.tools_write() = tools;
        *self.failures_write() = errors;
    }

    pub async fn set_tool_enabled(&self, server: &str, tool: &str, enable: bool) -> bool {
        let key = (server.to_string(), tool.to_string());
        let changed = {
            let mut excluded = self.excluded_tools_write();
            if enable {
                excluded.remove(&key)
            } else {
                excluded.insert(key)
            }
        };

        if changed {
            self.refresh_tools().await;
        }
        changed
    }

}

impl ManagedClient {
    async fn shutdown(self) {
        self.client.into_shutdown().await;
    }
}

/// Query every server for its available tools and return a single map that
/// contains **all** tools. Each key is the fully-qualified name for the tool.
async fn list_all_tools(
    clients: &HashMap<String, ManagedClient>,
    excluded_tools: &HashSet<(String, String)>,
    errors: &mut ClientStartErrors,
) -> Vec<ToolInfo> {
    let mut join_set = JoinSet::new();

    // Spawn one task per server so we can query them concurrently. This
    // keeps the overall latency roughly at the slowest server instead of
    // the cumulative latency.
    for (server_name, managed_client) in clients {
        let server_name_cloned = server_name.clone();
        let client_clone = managed_client.client.clone();
        let startup_timeout = managed_client.startup_timeout;
        join_set.spawn(async move {
            let res = client_clone.list_tools(None, Some(startup_timeout)).await;
            (server_name_cloned, res)
        });
    }

    let mut aggregated: Vec<ToolInfo> = Vec::with_capacity(join_set.len());

    while let Some(join_res) = join_set.join_next().await {
        let (server_name, list_result) = if let Ok(result) = join_res {
            result
        } else {
            warn!("Task panic when listing tools for MCP server: {join_res:#?}");
            continue;
        };

        match list_result {
            Ok(result) => {
                for tool in result.tools {
                    if excluded_tools.contains(&(server_name.clone(), tool.name.clone())) {
                        continue;
                    }
                    let tool_info = ToolInfo {
                        server_name: server_name.clone(),
                        tool_name: tool.name.clone(),
                        tool,
                    };
                    aggregated.push(tool_info);
                }
            }
            Err(err) => {
                warn!(
                    "Failed to list tools for MCP server '{server_name}': {err:#?}"
                );
                errors.insert(
                    server_name,
                    McpServerFailure {
                        phase: McpServerFailurePhase::ListTools,
                        message: format!("{err:#}"),
                    },
                );
            }
        }
    }

    info!(
        "aggregated {} tools from {} servers",
        aggregated.len(),
        clients.len()
    );

    aggregated
}

fn is_valid_mcp_server_name(server_name: &str) -> bool {
    !server_name.is_empty()
        && server_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_types::ToolInputSchema;

    fn create_test_tool(server_name: &str, tool_name: &str) -> ToolInfo {
        ToolInfo {
            server_name: server_name.to_string(),
            tool_name: tool_name.to_string(),
            tool: Tool {
                annotations: None,
                description: Some(format!("Test tool: {tool_name}")),
                input_schema: ToolInputSchema {
                    properties: None,
                    required: None,
                    r#type: "object".to_string(),
                },
                name: tool_name.to_string(),
                output_schema: None,
                title: None,
            },
        }
    }

    #[test]
    fn test_qualify_tools_short_non_duplicated_names() {
        let tools = vec![
            create_test_tool("server1", "tool1"),
            create_test_tool("server1", "tool2"),
        ];

        let qualified_tools = qualify_tools(tools);

        assert_eq!(qualified_tools.len(), 2);
        assert!(qualified_tools.contains_key("server1__tool1"));
        assert!(qualified_tools.contains_key("server1__tool2"));
    }

    #[test]
    fn test_qualify_tools_duplicated_names_skipped() {
        let tools = vec![
            create_test_tool("server1", "duplicate_tool"),
            create_test_tool("server1", "duplicate_tool"),
        ];

        let qualified_tools = qualify_tools(tools);

        // Only the first tool should remain, the second is skipped
        assert_eq!(qualified_tools.len(), 1);
        assert!(qualified_tools.contains_key("server1__duplicate_tool"));
    }

    #[test]
    fn test_qualify_tools_long_names_same_server() {
        let server_name = "my_server";

        let tools = vec![
            create_test_tool(
                server_name,
                "extremely_lengthy_function_name_that_absolutely_surpasses_all_reasonable_limits",
            ),
            create_test_tool(
                server_name,
                "yet_another_extremely_lengthy_function_name_that_absolutely_surpasses_all_reasonable_limits",
            ),
        ];

        let qualified_tools = qualify_tools(tools);

        assert_eq!(qualified_tools.len(), 2);

        let mut keys: Vec<_> = qualified_tools.keys().cloned().collect();
        keys.sort();

        assert_eq!(keys[0].len(), 64);
        assert_eq!(
            keys[0],
            "my_server__extremely_lena02e507efc5a9de88637e436690364fd4219e4ef"
        );

        assert_eq!(keys[1].len(), 64);
        assert_eq!(
            keys[1],
            "my_server__yet_another_e1c3987bd9c50b826cbe1687966f79f0c602d19ca"
        );
    }

    #[test]
    fn test_qualify_tools_sanitizes_invalid_characters() {
        let tools = vec![create_test_tool("server.one", "tool.two")];

        let qualified_tools = qualify_tools(tools);

        assert_eq!(qualified_tools.len(), 1);
        let (qualified_name, tool) = qualified_tools.into_iter().next().expect("one tool");
        assert_eq!(qualified_name, "server_one__tool_two");

        // The key is sanitized for OpenAI, but we keep original parts for the actual MCP call.
        assert_eq!(tool.server_name, "server.one");
        assert_eq!(tool.tool_name, "tool.two");

        assert!(
            qualified_name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            "qualified name must be Responses API compatible: {qualified_name:?}"
        );
    }

    #[tokio::test]
    async fn stdio_spawn_error_mentions_server_and_command() {
        let mut servers = HashMap::new();
        servers.insert(
            "context7-mcp".to_string(),
            McpServerConfig {
                transport: McpServerTransportConfig::Stdio {
                    command: "nonexistent-cmd".to_string(),
                    args: Vec::new(),
                    env: None,
                },
                startup_timeout_sec: None,
                tool_timeout_sec: None,
                disabled_tools: Vec::new(),
            },
        );

        let code_home = tempfile::tempdir().expect("code home");
        let (_manager, errors) = McpConnectionManager::new(
            code_home.path().to_path_buf(),
            OAuthCredentialsStoreMode::Auto,
            servers,
            HashSet::new(),
        )
            .await
            .expect("manager creation should succeed even when servers fail");

        let err = errors
            .get("context7-mcp")
            .expect("missing executable should be reported under server name");
        let msg = err.message.as_str();

        assert!(msg.contains("context7-mcp"), "error should mention the server name");
        assert!(
            msg.contains("nonexistent-cmd"),
            "error should include the missing command, got: {msg}"
        );
    }
}
