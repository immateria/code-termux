use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::process::Stdio;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use anyhow::anyhow;
use futures::FutureExt;
use mcp_types::CallToolRequestParams;
use mcp_types::CallToolResult;
use mcp_types::InitializeRequestParams;
use mcp_types::InitializeResult;
use mcp_types::ListToolsRequestParams;
use mcp_types::ListToolsResult;
use mcp_types::MCP_SCHEMA_VERSION;
use oauth2::TokenResponse;
use reqwest::header::AUTHORIZATION;
use reqwest::header::HeaderMap;
use rmcp::model::CallToolRequestParam;
use rmcp::model::InitializeRequestParam;
use rmcp::model::PaginatedRequestParam;
use rmcp::service::RoleClient;
use rmcp::service::RunningService;
use rmcp::service::{self};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::auth::AuthClient;
use rmcp::transport::auth::AuthError;
use rmcp::transport::auth::OAuthState;
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time;
use tracing::info;
use tracing::warn;

use crate::oauth::OAuthPersistor;
use crate::oauth::StoredOAuthTokens;
use crate::oauth::load_oauth_tokens;
use crate::logging_client_handler::LoggingClientHandler;
use crate::utils::convert_call_tool_result;
use crate::utils::convert_to_mcp;
use crate::utils::convert_to_rmcp;
use crate::utils::create_env_for_mcp_server;
use crate::utils::apply_default_headers;
use crate::utils::build_default_headers;
use crate::utils::run_with_timeout;
use crate::oauth::OAuthCredentialsStoreMode;

enum PendingTransport {
    ChildProcess(TokioChildProcess),
    StreamableHttp(StreamableHttpClientTransport<reqwest::Client>),
    StreamableHttpWithOAuth {
        transport: StreamableHttpClientTransport<AuthClient<reqwest::Client>>,
        oauth_persistor: OAuthPersistor,
    },
}

enum ClientState {
    Connecting {
        transport: Option<PendingTransport>,
    },
    Ready {
        service: Arc<RunningService<RoleClient, LoggingClientHandler>>,
        oauth: Option<OAuthPersistor>,
    },
}

/// MCP client implemented on top of the official `rmcp` SDK.
/// https://github.com/modelcontextprotocol/rust-sdk
pub struct RmcpClient {
    state: Mutex<ClientState>,
}

impl RmcpClient {
    pub async fn new_stdio_client(
        program: OsString,
        args: Vec<OsString>,
        env: Option<HashMap<String, String>>,
    ) -> io::Result<Self> {
        let program_name = program.to_string_lossy().into_owned();
        let mut last_err: Option<io::Error> = None;
        let mut spawned: Option<(TokioChildProcess, Option<tokio::process::ChildStderr>)> = None;

        for delay_ms in [0_u64, 10, 50] {
            let mut command = Command::new(&program);
            command
                .kill_on_drop(true)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .env_clear()
                .envs(create_env_for_mcp_server(env.clone()))
                .args(&args);

            match TokioChildProcess::builder(command)
                .stderr(Stdio::piped())
                .spawn()
            {
                Ok(child) => {
                    spawned = Some(child);
                    break;
                }
                Err(err)
                    if err.kind() == io::ErrorKind::WouldBlock
                        || matches!(err.raw_os_error(), Some(35) | Some(12)) =>
                {
                    last_err = Some(err);
                    if delay_ms > 0 {
                        time::sleep(Duration::from_millis(delay_ms)).await;
                    }
                }
                Err(err) => return Err(err),
            }
        }

        let (transport, stderr) = spawned
            .ok_or_else(|| last_err.unwrap_or_else(|| io::Error::other("failed to spawn rmcp server")))?;

        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut reader = BufReader::new(stderr).lines();
                loop {
                    match reader.next_line().await {
                        Ok(Some(line)) => {
                            info!("MCP server stderr ({program_name}): {line}");
                        }
                        Ok(None) => break,
                        Err(error) => {
                            warn!("Failed to read MCP server stderr ({program_name}): {error}");
                            break;
                        }
                    }
                }
            });
        }

        Ok(Self {
            state: Mutex::new(ClientState::Connecting {
                transport: Some(PendingTransport::ChildProcess(transport)),
            }),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new_streamable_http_client(
        code_home: PathBuf,
        server_name: &str,
        url: &str,
        bearer_token: Option<String>,
        http_headers: Option<HashMap<String, String>>,
        env_http_headers: Option<HashMap<String, String>>,
        store_mode: OAuthCredentialsStoreMode,
    ) -> Result<Self> {
        let default_headers = build_default_headers(http_headers, env_http_headers)?;

        let initial_oauth_tokens =
            if bearer_token.is_none() && !default_headers.contains_key(AUTHORIZATION) {
                match load_oauth_tokens(&code_home, server_name, url, store_mode) {
                    Ok(tokens) => tokens,
                    Err(err) => {
                        warn!("failed to read tokens for server `{server_name}`: {err}");
                        None
                    }
                }
            } else {
                None
            };

        let transport = if let Some(initial_tokens) = initial_oauth_tokens.clone() {
            match create_oauth_transport_and_runtime(
                code_home,
                server_name,
                url,
                initial_tokens.clone(),
                store_mode,
                default_headers.clone(),
            )
            .await
            {
                Ok((transport, oauth_persistor)) => PendingTransport::StreamableHttpWithOAuth {
                    transport,
                    oauth_persistor,
                },
                Err(err)
                    if err.downcast_ref::<AuthError>().is_some_and(|auth_err| {
                        matches!(auth_err, AuthError::NoAuthorizationSupport)
                    }) =>
                {
                    let access_token = initial_tokens
                        .token_response
                        .0
                        .access_token()
                        .secret()
                        .to_string();
                    warn!(
                        "OAuth metadata discovery is unavailable for MCP server `{server_name}`; falling back to stored bearer token authentication"
                    );
                    let http_config =
                        StreamableHttpClientTransportConfig::with_uri(url.to_string())
                            .auth_header(access_token);
                    let http_client =
                        apply_default_headers(reqwest::Client::builder(), &default_headers)
                            .build()?;
                    let transport =
                        StreamableHttpClientTransport::with_client(http_client, http_config);
                    PendingTransport::StreamableHttp(transport)
                }
                Err(err) => return Err(err),
            }
        } else {
            let mut http_config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
            if let Some(bearer_token) = bearer_token.clone() {
                http_config = http_config.auth_header(bearer_token);
            }

            let http_client =
                apply_default_headers(reqwest::Client::builder(), &default_headers).build()?;
            let transport = StreamableHttpClientTransport::with_client(http_client, http_config);
            PendingTransport::StreamableHttp(transport)
        };

        Ok(Self {
            state: Mutex::new(ClientState::Connecting {
                transport: Some(transport),
            }),
        })
    }

    /// Perform the initialization handshake with the MCP server.
    /// https://modelcontextprotocol.io/specification/2025-06-18/basic/lifecycle#initialization
    pub async fn initialize(
        &self,
        params: InitializeRequestParams,
        timeout: Option<Duration>,
    ) -> Result<InitializeResult> {
        let transport = {
            let mut guard = self.state.lock().await;
            match &mut *guard {
                ClientState::Connecting { transport } => transport
                    .take()
                    .ok_or_else(|| anyhow!("client already initializing"))?,
                ClientState::Ready { .. } => {
                    return Err(anyhow!("client already initialized"));
                }
            }
        };

        let client_info = convert_to_rmcp::<_, InitializeRequestParam>(params.clone())?;
        let client_handler = LoggingClientHandler::new(client_info);
        let (service_future, oauth_persistor) = match transport {
            PendingTransport::ChildProcess(transport) => (
                service::serve_client(client_handler.clone(), transport).boxed(),
                None,
            ),
            PendingTransport::StreamableHttp(transport) => (
                service::serve_client(client_handler, transport).boxed(),
                None,
            ),
            PendingTransport::StreamableHttpWithOAuth {
                transport,
                oauth_persistor,
            } => (
                service::serve_client(client_handler, transport).boxed(),
                Some(oauth_persistor),
            ),
        };

        let service = match timeout {
            Some(duration) => match time::timeout(duration, service_future).await {
                Ok(Ok(service)) => service,
                Ok(Err(err)) => return Err(handshake_failed_error(err)),
                Err(_) => return Err(handshake_timeout_error(duration)),
            },
            None => match service_future.await {
                Ok(service) => service,
                Err(err) => return Err(handshake_failed_error(err)),
            },
        };

        let initialize_result_rmcp = service
            .peer()
            .peer_info()
            .ok_or_else(|| anyhow!("handshake succeeded but server info was missing"))?;
        let initialize_result: InitializeResult = convert_to_mcp(initialize_result_rmcp)?;

        if initialize_result.protocol_version != MCP_SCHEMA_VERSION {
            let reported_version = initialize_result.protocol_version.clone();
            return Err(anyhow!(
                "MCP server reported protocol version {reported_version}, but this client expects {MCP_SCHEMA_VERSION}. Update either side so both speak the same schema."
            ));
        }

        {
            let mut guard = self.state.lock().await;
            *guard = ClientState::Ready {
                service: Arc::new(service),
                oauth: oauth_persistor,
            };
        }

        Ok(initialize_result)
    }

    pub async fn list_tools(
        &self,
        params: Option<ListToolsRequestParams>,
        timeout: Option<Duration>,
    ) -> Result<ListToolsResult> {
        self.refresh_oauth_if_needed().await;
        let service = self.service().await?;
        let rmcp_params = params
            .map(convert_to_rmcp::<_, PaginatedRequestParam>)
            .transpose()?;

        let fut = service.list_tools(rmcp_params);
        let result = run_with_timeout(fut, timeout, "tools/list").await?;
        self.persist_oauth_tokens().await;
        convert_to_mcp(result)
    }

    pub async fn call_tool(
        &self,
        name: String,
        arguments: Option<serde_json::Value>,
        timeout: Option<Duration>,
    ) -> Result<CallToolResult> {
        self.refresh_oauth_if_needed().await;
        let service = self.service().await?;
        let params = CallToolRequestParams { arguments, name };
        let rmcp_params: CallToolRequestParam = convert_to_rmcp(params)?;
        let fut = service.call_tool(rmcp_params);
        let rmcp_result = run_with_timeout(fut, timeout, "tools/call").await?;
        self.persist_oauth_tokens().await;
        convert_call_tool_result(rmcp_result)
    }

    async fn service(&self) -> Result<Arc<RunningService<RoleClient, LoggingClientHandler>>> {
        let guard = self.state.lock().await;
        match &*guard {
            ClientState::Ready { service, .. } => Ok(Arc::clone(service)),
            ClientState::Connecting { .. } => Err(anyhow!("MCP client not initialized")),
        }
    }

    async fn oauth_persistor(&self) -> Option<OAuthPersistor> {
        let guard = self.state.lock().await;
        match &*guard {
            ClientState::Ready {
                oauth: Some(runtime),
                ..
            } => Some(runtime.clone()),
            _ => None,
        }
    }

    /// This should be called after every MCP request so that if a given request triggered
    /// a refresh of the OAuth tokens, they are persisted.
    async fn persist_oauth_tokens(&self) {
        if let Some(runtime) = self.oauth_persistor().await
            && let Err(error) = runtime.persist_if_needed().await
        {
            warn!("failed to persist OAuth tokens: {error}");
        }
    }

    async fn refresh_oauth_if_needed(&self) {
        if let Some(runtime) = self.oauth_persistor().await
            && let Err(error) = runtime.refresh_if_needed().await
        {
            warn!("failed to refresh OAuth tokens: {error}");
        }
    }

    pub async fn shutdown(&self) {
        if let Ok(service) = self.service().await {
            service.cancellation_token().cancel();
        }
    }
}

async fn create_oauth_transport_and_runtime(
    code_home: PathBuf,
    server_name: &str,
    url: &str,
    initial_tokens: StoredOAuthTokens,
    credentials_store: OAuthCredentialsStoreMode,
    default_headers: HeaderMap,
) -> Result<(StreamableHttpClientTransport<AuthClient<reqwest::Client>>, OAuthPersistor)> {
    let http_client =
        apply_default_headers(reqwest::Client::builder(), &default_headers).build()?;
    let mut oauth_state = OAuthState::new(url.to_string(), Some(http_client.clone())).await?;

    oauth_state
        .set_credentials(
            &initial_tokens.client_id,
            initial_tokens.token_response.0.clone(),
        )
        .await?;

    let manager = match oauth_state {
        OAuthState::Authorized(manager) => manager,
        OAuthState::Unauthorized(manager) => manager,
        OAuthState::Session(_) | OAuthState::AuthorizedHttpClient(_) => {
            return Err(anyhow!("unexpected OAuth state during client setup"));
        }
    };

    let auth_client = AuthClient::new(http_client, manager);
    let auth_manager = auth_client.auth_manager.clone();

    let transport = StreamableHttpClientTransport::with_client(
        auth_client,
        StreamableHttpClientTransportConfig::with_uri(url.to_string()),
    );

    let runtime = OAuthPersistor::new(
        code_home,
        server_name.to_string(),
        url.to_string(),
        auth_manager,
        credentials_store,
        Some(initial_tokens),
    );

    Ok((transport, runtime))
}

fn handshake_failed_error(err: impl Into<anyhow::Error>) -> anyhow::Error {
    let err = err.into();
    anyhow!(
        "handshaking with MCP server failed: {err} (this client supports MCP schema version {MCP_SCHEMA_VERSION})"
    )
}

fn handshake_timeout_error(duration: Duration) -> anyhow::Error {
    anyhow!(
        "timed out handshaking with MCP server after {duration:?} (expected MCP schema version {MCP_SCHEMA_VERSION})"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_schema_version_is_well_formed() {
        assert!(!MCP_SCHEMA_VERSION.is_empty());
        let parts: Vec<&str> = MCP_SCHEMA_VERSION.split('-').collect();
        assert_eq!(
            parts.len(),
            3,
            "MCP_SCHEMA_VERSION should be in YYYY-MM-DD format"
        );
        assert!(parts.iter().all(|segment| !segment.trim().is_empty()));
    }
}
