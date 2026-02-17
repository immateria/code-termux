//! MCP OAuth token storage + persistence.
//!
//! This module stores per-server OAuth tokens either in the OS keyring or on disk under
//! `CODE_HOME/.credentials.json`.
//!
//! The implementation is adapted from upstream `codex-rs/rmcp-client/src/oauth.rs` but
//! takes `code_home: &Path` explicitly so dev-mode `CODE_HOME` overrides stay consistent
//! across the workspace.

use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Error, Result};
use oauth2::AccessToken;
use oauth2::EmptyExtraTokenFields;
use oauth2::RefreshToken;
use oauth2::Scope;
use oauth2::TokenResponse;
use oauth2::basic::BasicTokenType;
use rmcp::transport::auth::AuthorizationManager;
use rmcp::transport::auth::OAuthTokenResponse;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::map::Map as JsonMap;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tracing::warn;

use code_keyring_store::DefaultKeyringStore;
use code_keyring_store::KeyringStore;

const KEYRING_SERVICE: &str = "Code MCP Credentials";
const REFRESH_SKEW_MILLIS: u64 = 30_000;

const FALLBACK_FILENAME: &str = ".credentials.json";
const MCP_SERVER_TYPE: &str = "http";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredOAuthTokens {
    pub server_name: String,
    pub url: String,
    pub client_id: String,
    pub token_response: WrappedOAuthTokenResponse,
    #[serde(default)]
    pub expires_at: Option<u64>,
}

/// Determine where Code should store and read MCP OAuth credentials.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OAuthCredentialsStoreMode {
    /// `Keyring` when available; otherwise, `File`.
    #[default]
    Auto,
    /// `CODE_HOME/.credentials.json`
    File,
    /// Keyring when available, otherwise fail.
    Keyring,
}

/// Wrap OAuthTokenResponse to allow for partial equality comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WrappedOAuthTokenResponse(pub OAuthTokenResponse);

impl PartialEq for WrappedOAuthTokenResponse {
    fn eq(&self, other: &Self) -> bool {
        match (serde_json::to_string(self), serde_json::to_string(other)) {
            (Ok(s1), Ok(s2)) => s1 == s2,
            _ => false,
        }
    }
}

pub(crate) fn load_oauth_tokens(
    code_home: &Path,
    server_name: &str,
    url: &str,
    store_mode: OAuthCredentialsStoreMode,
) -> Result<Option<StoredOAuthTokens>> {
    let keyring_store = DefaultKeyringStore;
    match store_mode {
        OAuthCredentialsStoreMode::Auto => load_oauth_tokens_from_keyring_with_fallback_to_file(
            &keyring_store,
            code_home,
            server_name,
            url,
        ),
        OAuthCredentialsStoreMode::File => load_oauth_tokens_from_file(code_home, server_name, url),
        OAuthCredentialsStoreMode::Keyring => load_oauth_tokens_from_keyring(
            &keyring_store,
            code_home,
            server_name,
            url,
        )
        .with_context(|| "failed to read OAuth tokens from keyring".to_string()),
    }
}

pub(crate) fn has_oauth_tokens(
    code_home: &Path,
    server_name: &str,
    url: &str,
    store_mode: OAuthCredentialsStoreMode,
) -> Result<bool> {
    Ok(load_oauth_tokens(code_home, server_name, url, store_mode)?.is_some())
}

fn refresh_expires_in_from_timestamp(tokens: &mut StoredOAuthTokens) {
    let Some(expires_at) = tokens.expires_at else {
        return;
    };

    match expires_in_from_timestamp(expires_at) {
        Some(seconds) => {
            let duration = Duration::from_secs(seconds);
            tokens.token_response.0.set_expires_in(Some(&duration));
        }
        None => {
            tokens.token_response.0.set_expires_in(None);
        }
    }
}

fn load_oauth_tokens_from_keyring_with_fallback_to_file<K: KeyringStore>(
    keyring_store: &K,
    code_home: &Path,
    server_name: &str,
    url: &str,
) -> Result<Option<StoredOAuthTokens>> {
    match load_oauth_tokens_from_keyring(keyring_store, code_home, server_name, url) {
        Ok(Some(tokens)) => Ok(Some(tokens)),
        Ok(None) => load_oauth_tokens_from_file(code_home, server_name, url),
        Err(error) => {
            warn!("failed to read OAuth tokens from keyring: {error}");
            load_oauth_tokens_from_file(code_home, server_name, url)
                .with_context(|| format!("failed to read OAuth tokens from keyring: {error}"))
        }
    }
}

fn load_oauth_tokens_from_keyring<K: KeyringStore>(
    keyring_store: &K,
    code_home: &Path,
    server_name: &str,
    url: &str,
) -> Result<Option<StoredOAuthTokens>> {
    let key = compute_keyring_account(code_home, server_name, url)?;
    match keyring_store.load(KEYRING_SERVICE, &key) {
        Ok(Some(serialized)) => {
            let mut tokens: StoredOAuthTokens = serde_json::from_str(&serialized)
                .context("failed to deserialize OAuth tokens from keyring")?;
            refresh_expires_in_from_timestamp(&mut tokens);
            Ok(Some(tokens))
        }
        Ok(None) => Ok(None),
        Err(error) => Err(Error::new(error.into_error())),
    }
}

pub fn save_oauth_tokens(
    code_home: &Path,
    server_name: &str,
    tokens: &StoredOAuthTokens,
    store_mode: OAuthCredentialsStoreMode,
) -> Result<()> {
    let keyring_store = DefaultKeyringStore;
    match store_mode {
        OAuthCredentialsStoreMode::Auto => save_oauth_tokens_with_keyring_with_fallback_to_file(
            &keyring_store,
            code_home,
            server_name,
            tokens,
        ),
        OAuthCredentialsStoreMode::File => save_oauth_tokens_to_file(code_home, tokens),
        OAuthCredentialsStoreMode::Keyring => {
            save_oauth_tokens_with_keyring(&keyring_store, code_home, server_name, tokens)
        }
    }
}

fn save_oauth_tokens_with_keyring<K: KeyringStore>(
    keyring_store: &K,
    code_home: &Path,
    server_name: &str,
    tokens: &StoredOAuthTokens,
) -> Result<()> {
    let serialized = serde_json::to_string(tokens).context("failed to serialize OAuth tokens")?;

    let key = compute_keyring_account(code_home, server_name, &tokens.url)?;
    match keyring_store.save(KEYRING_SERVICE, &key, &serialized) {
        Ok(()) => {
            if let Err(error) = delete_oauth_tokens_from_file(code_home, &compute_store_key(server_name, &tokens.url)?) {
                warn!("failed to remove OAuth tokens from fallback storage: {error:?}");
            }
            Ok(())
        }
        Err(error) => {
            let message = format!("failed to write OAuth tokens to keyring: {}", error.message());
            warn!("{message}");
            Err(Error::new(error.into_error()).context(message))
        }
    }
}

fn save_oauth_tokens_with_keyring_with_fallback_to_file<K: KeyringStore>(
    keyring_store: &K,
    code_home: &Path,
    server_name: &str,
    tokens: &StoredOAuthTokens,
) -> Result<()> {
    match save_oauth_tokens_with_keyring(keyring_store, code_home, server_name, tokens) {
        Ok(()) => Ok(()),
        Err(error) => {
            let message = error.to_string();
            warn!("falling back to file storage for OAuth tokens: {message}");
            save_oauth_tokens_to_file(code_home, tokens)
                .with_context(|| format!("failed to write OAuth tokens to keyring: {message}"))
        }
    }
}

pub fn delete_oauth_tokens(
    code_home: &Path,
    server_name: &str,
    url: &str,
    store_mode: OAuthCredentialsStoreMode,
) -> Result<bool> {
    let keyring_store = DefaultKeyringStore;
    delete_oauth_tokens_from_keyring_and_file(&keyring_store, code_home, store_mode, server_name, url)
}

fn delete_oauth_tokens_from_keyring_and_file<K: KeyringStore>(
    keyring_store: &K,
    code_home: &Path,
    store_mode: OAuthCredentialsStoreMode,
    server_name: &str,
    url: &str,
) -> Result<bool> {
    let file_key = compute_store_key(server_name, url)?;
    let keyring_key = compute_keyring_account(code_home, server_name, url)?;

    let keyring_result = keyring_store.delete(KEYRING_SERVICE, &keyring_key);
    let keyring_removed = match keyring_result {
        Ok(removed) => removed,
        Err(error) => {
            let message = error.message();
            warn!("failed to delete OAuth tokens from keyring: {message}");
            match store_mode {
                OAuthCredentialsStoreMode::Auto | OAuthCredentialsStoreMode::Keyring => {
                    return Err(error.into_error())
                        .context("failed to delete OAuth tokens from keyring");
                }
                OAuthCredentialsStoreMode::File => false,
            }
        }
    };

    let file_removed = delete_oauth_tokens_from_file(code_home, &file_key)?;
    Ok(keyring_removed || file_removed)
}

#[derive(Clone)]
pub(crate) struct OAuthPersistor {
    inner: Arc<OAuthPersistorInner>,
}

struct OAuthPersistorInner {
    code_home: PathBuf,
    server_name: String,
    url: String,
    authorization_manager: Arc<Mutex<AuthorizationManager>>,
    store_mode: OAuthCredentialsStoreMode,
    last_credentials: Mutex<Option<StoredOAuthTokens>>,
}

impl OAuthPersistor {
    pub(crate) fn new(
        code_home: PathBuf,
        server_name: String,
        url: String,
        authorization_manager: Arc<Mutex<AuthorizationManager>>,
        store_mode: OAuthCredentialsStoreMode,
        initial_credentials: Option<StoredOAuthTokens>,
    ) -> Self {
        Self {
            inner: Arc::new(OAuthPersistorInner {
                code_home,
                server_name,
                url,
                authorization_manager,
                store_mode,
                last_credentials: Mutex::new(initial_credentials),
            }),
        }
    }

    /// Persists the latest stored credentials if they have changed.
    /// Deletes the credentials if they are no longer present.
    pub(crate) async fn persist_if_needed(&self) -> Result<()> {
        let (client_id, maybe_credentials) = {
            let manager = self.inner.authorization_manager.clone();
            let guard = manager.lock().await;
            guard.get_credentials().await
        }?;

        match maybe_credentials {
            Some(credentials) => {
                let mut last_credentials = self.inner.last_credentials.lock().await;
                let new_token_response = WrappedOAuthTokenResponse(credentials.clone());
                let same_token = last_credentials
                    .as_ref()
                    .map(|prev| prev.token_response == new_token_response)
                    .unwrap_or(false);
                let expires_at = if same_token {
                    last_credentials.as_ref().and_then(|prev| prev.expires_at)
                } else {
                    compute_expires_at_millis(&credentials)
                };
                let stored = StoredOAuthTokens {
                    server_name: self.inner.server_name.clone(),
                    url: self.inner.url.clone(),
                    client_id,
                    token_response: new_token_response,
                    expires_at,
                };
                if last_credentials.as_ref() != Some(&stored) {
                    save_oauth_tokens(
                        &self.inner.code_home,
                        &self.inner.server_name,
                        &stored,
                        self.inner.store_mode,
                    )?;
                    *last_credentials = Some(stored);
                }
            }
            None => {
                let mut last_serialized = self.inner.last_credentials.lock().await;
                if last_serialized.take().is_some()
                    && let Err(error) = delete_oauth_tokens(
                        &self.inner.code_home,
                        &self.inner.server_name,
                        &self.inner.url,
                        self.inner.store_mode,
                    )
                {
                    warn!(
                        "failed to remove OAuth tokens for server {}: {error}",
                        self.inner.server_name
                    );
                }
            }
        }

        Ok(())
    }

    pub(crate) async fn refresh_if_needed(&self) -> Result<()> {
        let expires_at = {
            let guard = self.inner.last_credentials.lock().await;
            guard.as_ref().and_then(|tokens| tokens.expires_at)
        };

        if !token_needs_refresh(expires_at) {
            return Ok(());
        }

        {
            let manager = self.inner.authorization_manager.clone();
            let guard = manager.lock().await;
            guard.refresh_token().await.with_context(|| {
                format!(
                    "failed to refresh OAuth tokens for server {}",
                    self.inner.server_name
                )
            })?;
        }

        self.persist_if_needed().await
    }
}

type FallbackFile = BTreeMap<String, FallbackTokenEntry>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FallbackTokenEntry {
    server_name: String,
    server_url: String,
    client_id: String,
    access_token: String,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scopes: Vec<String>,
}

fn load_oauth_tokens_from_file(code_home: &Path, server_name: &str, url: &str) -> Result<Option<StoredOAuthTokens>> {
    let Some(store) = read_fallback_file(code_home)? else {
        return Ok(None);
    };

    let key = compute_store_key(server_name, url)?;

    for entry in store.values() {
        let entry_key = compute_store_key(&entry.server_name, &entry.server_url)?;
        if entry_key != key {
            continue;
        }

        let mut token_response = OAuthTokenResponse::new(
            AccessToken::new(entry.access_token.clone()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );

        if let Some(refresh) = entry.refresh_token.clone() {
            token_response.set_refresh_token(Some(RefreshToken::new(refresh)));
        }

        let scopes = entry.scopes.clone();
        if !scopes.is_empty() {
            token_response.set_scopes(Some(scopes.into_iter().map(Scope::new).collect()));
        }

        let mut stored = StoredOAuthTokens {
            server_name: entry.server_name.clone(),
            url: entry.server_url.clone(),
            client_id: entry.client_id.clone(),
            token_response: WrappedOAuthTokenResponse(token_response),
            expires_at: entry.expires_at,
        };
        refresh_expires_in_from_timestamp(&mut stored);

        return Ok(Some(stored));
    }

    Ok(None)
}

fn save_oauth_tokens_to_file(code_home: &Path, tokens: &StoredOAuthTokens) -> Result<()> {
    let key = compute_store_key(&tokens.server_name, &tokens.url)?;
    let mut store = read_fallback_file(code_home)?.unwrap_or_default();

    let token_response = &tokens.token_response.0;
    let expires_at = tokens
        .expires_at
        .or_else(|| compute_expires_at_millis(token_response));
    let refresh_token = token_response
        .refresh_token()
        .map(|token| token.secret().to_string());
    let scopes = token_response
        .scopes()
        .map(|s| s.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();
    let entry = FallbackTokenEntry {
        server_name: tokens.server_name.clone(),
        server_url: tokens.url.clone(),
        client_id: tokens.client_id.clone(),
        access_token: token_response.access_token().secret().to_string(),
        expires_at,
        refresh_token,
        scopes,
    };

    store.insert(key, entry);
    write_fallback_file(code_home, &store)
}

fn delete_oauth_tokens_from_file(code_home: &Path, key: &str) -> Result<bool> {
    let mut store = match read_fallback_file(code_home)? {
        Some(store) => store,
        None => return Ok(false),
    };

    let removed = store.remove(key).is_some();

    if removed {
        write_fallback_file(code_home, &store)?;
    }

    Ok(removed)
}

pub(crate) fn compute_expires_at_millis(response: &OAuthTokenResponse) -> Option<u64> {
    let expires_in = response.expires_in()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    let expiry = now.checked_add(expires_in)?;
    let millis = expiry.as_millis();
    if millis > u128::from(u64::MAX) {
        Some(u64::MAX)
    } else {
        Some(millis as u64)
    }
}

fn expires_in_from_timestamp(expires_at: u64) -> Option<u64> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    let now_ms = now.as_millis() as u64;

    if expires_at <= now_ms {
        None
    } else {
        Some((expires_at - now_ms) / 1000)
    }
}

fn token_needs_refresh(expires_at: Option<u64>) -> bool {
    let Some(expires_at) = expires_at else {
        return false;
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64;

    now.saturating_add(REFRESH_SKEW_MILLIS) >= expires_at
}

fn compute_store_key(server_name: &str, server_url: &str) -> Result<String> {
    let mut payload = JsonMap::new();
    payload.insert("type".to_string(), Value::String(MCP_SERVER_TYPE.to_string()));
    payload.insert("url".to_string(), Value::String(server_url.to_string()));
    payload.insert("headers".to_string(), Value::Object(JsonMap::new()));

    let truncated = sha_256_prefix(&Value::Object(payload))?;
    Ok(format!("{server_name}|{truncated}"))
}

fn compute_keyring_account(code_home: &Path, server_name: &str, server_url: &str) -> Result<String> {
    let base = compute_store_key(server_name, server_url)?;
    let home_prefix = code_keyring_store::store_key_for_code_home("mcp-oauth", code_home);
    Ok(format!("{home_prefix}|{base}"))
}

fn fallback_file_path(code_home: &Path) -> PathBuf {
    code_home.join(FALLBACK_FILENAME)
}

fn read_fallback_file(code_home: &Path) -> Result<Option<FallbackFile>> {
    let path = fallback_file_path(code_home);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).context(format!(
                "failed to read credentials file at {}",
                path.display()
            ));
        }
    };

    match serde_json::from_str::<FallbackFile>(&contents) {
        Ok(store) => Ok(Some(store)),
        Err(e) => Err(e).context(format!(
            "failed to parse credentials file at {}",
            path.display()
        )),
    }
}

fn write_fallback_file(code_home: &Path, store: &FallbackFile) -> Result<()> {
    let path = fallback_file_path(code_home);

    if store.is_empty() {
        if path.exists() {
            fs::remove_file(path)?;
        }
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let serialized = serde_json::to_string(store)?;
    fs::write(&path, serialized)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&path, perms)?;
    }

    Ok(())
}

fn sha_256_prefix(value: &Value) -> Result<String> {
    let serialized = serde_json::to_string(&value).context("failed to serialize MCP OAuth key payload")?;
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    let truncated = &hex[..16];
    Ok(truncated.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use keyring::Error as KeyringError;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    use code_keyring_store::tests::MockKeyringStore;

    fn sample_tokens() -> StoredOAuthTokens {
        let mut response = OAuthTokenResponse::new(
            AccessToken::new("access".to_string()),
            BasicTokenType::Bearer,
            EmptyExtraTokenFields {},
        );
        response.set_refresh_token(Some(RefreshToken::new("refresh".to_string())));
        response.set_scopes(Some(vec![Scope::new("scope".to_string())]));
        StoredOAuthTokens {
            server_name: "server".to_string(),
            url: "https://example.com/mcp".to_string(),
            client_id: "client".to_string(),
            token_response: WrappedOAuthTokenResponse(response),
            expires_at: None,
        }
    }

    fn assert_tokens_match_without_expiry(loaded: &StoredOAuthTokens, expected: &StoredOAuthTokens) {
        assert_eq!(loaded.server_name, expected.server_name);
        assert_eq!(loaded.url, expected.url);
        assert_eq!(loaded.client_id, expected.client_id);
        assert_eq!(
            loaded.token_response.0.access_token().secret(),
            expected.token_response.0.access_token().secret()
        );
        assert_eq!(
            loaded
                .token_response
                .0
                .refresh_token()
                .map(|t| t.secret().to_string()),
            expected
                .token_response
                .0
                .refresh_token()
                .map(|t| t.secret().to_string())
        );
    }

    #[test]
    fn load_oauth_tokens_reads_from_keyring_when_available() -> Result<()> {
        let code_home = tempdir()?;
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let expected = tokens.clone();
        let serialized = serde_json::to_string(&tokens)?;
        let key = super::compute_keyring_account(code_home.path(), &tokens.server_name, &tokens.url)?;
        store.save(KEYRING_SERVICE, &key, &serialized)?;

        let loaded = super::load_oauth_tokens_from_keyring(&store, code_home.path(), &tokens.server_name, &tokens.url)?
            .expect("tokens should load from keyring");
        assert_tokens_match_without_expiry(&loaded, &expected);
        Ok(())
    }

    #[test]
    fn load_oauth_tokens_falls_back_when_missing_in_keyring() -> Result<()> {
        let code_home = tempdir()?;
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let expected = tokens.clone();

        super::save_oauth_tokens_to_file(code_home.path(), &tokens)?;

        let loaded = super::load_oauth_tokens_from_keyring_with_fallback_to_file(
            &store,
            code_home.path(),
            &tokens.server_name,
            &tokens.url,
        )?
        .expect("tokens should load from fallback");
        assert_tokens_match_without_expiry(&loaded, &expected);
        Ok(())
    }

    #[test]
    fn load_oauth_tokens_falls_back_when_keyring_errors() -> Result<()> {
        let code_home = tempdir()?;
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let expected = tokens.clone();
        let key = super::compute_keyring_account(code_home.path(), &tokens.server_name, &tokens.url)?;
        store.set_error(&key, KeyringError::Invalid("error".into(), "load".into()));

        super::save_oauth_tokens_to_file(code_home.path(), &tokens)?;

        let loaded = super::load_oauth_tokens_from_keyring_with_fallback_to_file(
            &store,
            code_home.path(),
            &tokens.server_name,
            &tokens.url,
        )?
        .expect("tokens should load from fallback");
        assert_tokens_match_without_expiry(&loaded, &expected);
        Ok(())
    }

    #[test]
    fn save_oauth_tokens_prefers_keyring_when_available() -> Result<()> {
        let code_home = tempdir()?;
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let key = super::compute_keyring_account(code_home.path(), &tokens.server_name, &tokens.url)?;

        super::save_oauth_tokens_to_file(code_home.path(), &tokens)?;

        super::save_oauth_tokens_with_keyring_with_fallback_to_file(
            &store,
            code_home.path(),
            &tokens.server_name,
            &tokens,
        )?;

        let fallback_path = super::fallback_file_path(code_home.path());
        assert!(!fallback_path.exists(), "fallback file should be removed");
        let stored = store.saved_value(&key).expect("value saved to keyring");
        assert_eq!(serde_json::from_str::<StoredOAuthTokens>(&stored)?, tokens);
        Ok(())
    }

    #[test]
    fn save_oauth_tokens_writes_fallback_when_keyring_fails() -> Result<()> {
        let code_home = tempdir()?;
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let key = super::compute_keyring_account(code_home.path(), &tokens.server_name, &tokens.url)?;
        store.set_error(&key, KeyringError::Invalid("error".into(), "save".into()));

        super::save_oauth_tokens_with_keyring_with_fallback_to_file(
            &store,
            code_home.path(),
            &tokens.server_name,
            &tokens,
        )?;

        let fallback_path = super::fallback_file_path(code_home.path());
        assert!(fallback_path.exists(), "fallback file should be created");
        let saved = super::read_fallback_file(code_home.path())?.expect("fallback file should load");
        let file_key = super::compute_store_key(&tokens.server_name, &tokens.url)?;
        let entry = saved.get(&file_key).expect("entry for key");
        assert_eq!(entry.server_name, tokens.server_name);
        assert_eq!(entry.server_url, tokens.url);
        assert_eq!(entry.client_id, tokens.client_id);
        assert_eq!(
            entry.access_token,
            tokens.token_response.0.access_token().secret().as_str()
        );
        assert!(store.saved_value(&key).is_none());
        Ok(())
    }

    #[test]
    fn delete_oauth_tokens_removes_all_storage() -> Result<()> {
        let code_home = tempdir()?;
        let store = MockKeyringStore::default();
        let tokens = sample_tokens();
        let serialized = serde_json::to_string(&tokens)?;
        let keyring_key = super::compute_keyring_account(code_home.path(), &tokens.server_name, &tokens.url)?;
        store.save(KEYRING_SERVICE, &keyring_key, &serialized)?;
        super::save_oauth_tokens_to_file(code_home.path(), &tokens)?;

        let removed = super::delete_oauth_tokens_from_keyring_and_file(
            &store,
            code_home.path(),
            OAuthCredentialsStoreMode::Auto,
            &tokens.server_name,
            &tokens.url,
        )?;
        assert!(removed);
        assert!(!super::fallback_file_path(code_home.path()).exists());
        assert!(store.saved_value(&keyring_key).is_none());
        Ok(())
    }
}

