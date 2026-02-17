use chrono::{DateTime, Utc};
use code_app_server_protocol::AuthMode;
use code_keyring_store::DefaultKeyringStore;
use code_keyring_store::KeyringStore;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use uuid::Uuid;

use crate::config_types::AuthCredentialsStoreMode;
use crate::keyring_keys::store_key_for_code_home;
use crate::token_data::TokenData;

const ACCOUNTS_FILE_NAME: &str = "auth_accounts.json";
const ACCOUNTS_CONFIG_TABLE: &str = "accounts";
const ACCOUNTS_READ_PATHS_KEY: &str = "read_paths";
const ACCOUNTS_WRITE_PATH_KEY: &str = "write_path";
const KEYRING_SERVICE: &str = "Codex Auth Accounts";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredAccount {
    pub id: String,
    pub mode: AuthMode,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_api_key: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenData>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<DateTime<Utc>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct AccountsFile {
    #[serde(default = "default_version")]
    version: u32,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    active_account_id: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    accounts: Vec<StoredAccount>,
}

impl Default for AccountsFile {
    fn default() -> Self {
        Self {
            version: default_version(),
            active_account_id: None,
            accounts: Vec::new(),
        }
    }
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Clone)]
struct AccountStorePaths {
    read_paths: Vec<PathBuf>,
    write_path: PathBuf,
}

fn resolve_store_path(code_home: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        code_home.join(path)
    }
}

fn configured_account_store_paths(code_home: &Path) -> Option<AccountStorePaths> {
    let root = match crate::config::load_config_as_toml(code_home) {
        Ok(value) => value,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::warn!("failed to read config while resolving account store paths: {err}");
            return None;
        }
    };

    let accounts = root
        .get(ACCOUNTS_CONFIG_TABLE)
        .and_then(toml::Value::as_table)?;

    let read_paths = accounts
        .get(ACCOUNTS_READ_PATHS_KEY)
        .and_then(toml::Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| resolve_store_path(code_home, &path))
        .collect::<Vec<_>>();

    let write_path = accounts
        .get(ACCOUNTS_WRITE_PATH_KEY)
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| resolve_store_path(code_home, &path));

    if read_paths.is_empty() && write_path.is_none() {
        return None;
    }

    let write_path = write_path.unwrap_or_else(|| code_home.join(ACCOUNTS_FILE_NAME));
    Some(AccountStorePaths {
        read_paths,
        write_path,
    })
}

fn account_store_paths(code_home: &Path) -> AccountStorePaths {
    let default_write_path = code_home.join(ACCOUNTS_FILE_NAME);
    let default_read_path =
        crate::config::resolve_code_path_for_read(code_home, Path::new(ACCOUNTS_FILE_NAME));

    let mut paths = configured_account_store_paths(code_home).unwrap_or(AccountStorePaths {
        read_paths: vec![default_read_path.clone()],
        write_path: default_write_path,
    });

    if paths.read_paths.is_empty() {
        paths.read_paths.push(default_read_path);
    }

    if !paths.read_paths.iter().any(|path| path == &paths.write_path) {
        paths.read_paths.insert(0, paths.write_path.clone());
    }

    let mut seen = HashSet::new();
    paths.read_paths.retain(|path| seen.insert(path.clone()));
    paths
}

fn accounts_store_key(code_home: &Path) -> String {
    store_key_for_code_home("cli-accounts", code_home)
}

fn configured_auth_credentials_store_mode(code_home: &Path) -> Option<AuthCredentialsStoreMode> {
    let root = match crate::config::load_config_as_toml(code_home) {
        Ok(value) => value,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return None,
        Err(err) => {
            tracing::warn!(
                "failed to read config while resolving auth credentials store mode: {err}"
            );
            return None;
        }
    };

    let raw = root
        .get("cli_auth_credentials_store")
        .and_then(toml::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

    match raw.to_ascii_lowercase().as_str() {
        "file" => Some(AuthCredentialsStoreMode::File),
        "keyring" => Some(AuthCredentialsStoreMode::Keyring),
        "auto" => Some(AuthCredentialsStoreMode::Auto),
        "ephemeral" => Some(AuthCredentialsStoreMode::Ephemeral),
        other => {
            tracing::warn!("unknown cli_auth_credentials_store value '{other}', using default");
            None
        }
    }
}

fn auth_credentials_store_mode(code_home: &Path) -> AuthCredentialsStoreMode {
    configured_auth_credentials_store_mode(code_home).unwrap_or_default()
}

trait AccountsStorageBackend: Send + Sync {
    fn load(&self) -> io::Result<AccountsFile>;
    fn save(&self, data: &AccountsFile) -> io::Result<()>;
}

#[derive(Clone)]
struct FileAccountsStorage {
    paths: AccountStorePaths,
}

impl FileAccountsStorage {
    fn new(paths: AccountStorePaths) -> Self {
        Self { paths }
    }
}

impl AccountsStorageBackend for FileAccountsStorage {
    fn load(&self) -> io::Result<AccountsFile> {
        load_accounts_file(&self.paths)
    }

    fn save(&self, data: &AccountsFile) -> io::Result<()> {
        write_accounts_file(&self.paths.write_path, data)
    }
}

fn delete_file_if_exists(path: &Path) -> io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

fn delete_accounts_files_if_exists(code_home: &Path, paths: &AccountStorePaths) -> io::Result<bool> {
    let legacy_path =
        crate::config::resolve_code_path_for_read(code_home, Path::new(ACCOUNTS_FILE_NAME));

    let mut removed = delete_file_if_exists(&paths.write_path)?;
    if legacy_path != paths.write_path {
        removed |= delete_file_if_exists(&legacy_path)?;
    }
    Ok(removed)
}

#[derive(Clone)]
struct KeyringAccountsStorage {
    code_home: PathBuf,
    paths: AccountStorePaths,
    keyring_store: Arc<dyn KeyringStore>,
}

impl KeyringAccountsStorage {
    fn new(code_home: PathBuf, paths: AccountStorePaths, keyring_store: Arc<dyn KeyringStore>) -> Self {
        Self {
            code_home,
            paths,
            keyring_store,
        }
    }

    fn load_from_keyring(&self, key: &str) -> io::Result<Option<AccountsFile>> {
        match self.keyring_store.load(KEYRING_SERVICE, key) {
            Ok(Some(serialized)) => serde_json::from_str(&serialized)
                .map(Some)
                .map_err(|err| std::io::Error::other(format!(
                    "failed to deserialize accounts from keyring: {err}"
                ))),
            Ok(None) => Ok(None),
            Err(error) => Err(std::io::Error::other(format!(
                "failed to load accounts from keyring: {}",
                error.message()
            ))),
        }
    }

    fn save_to_keyring(&self, key: &str, value: &str) -> io::Result<()> {
        match self.keyring_store.save(KEYRING_SERVICE, key, value) {
            Ok(()) => Ok(()),
            Err(error) => Err(std::io::Error::other(format!(
                "failed to save accounts to keyring: {}",
                error.message()
            ))),
        }
    }
}

impl AccountsStorageBackend for KeyringAccountsStorage {
    fn load(&self) -> io::Result<AccountsFile> {
        let key = accounts_store_key(&self.code_home);
        Ok(self.load_from_keyring(&key)?.unwrap_or_default())
    }

    fn save(&self, data: &AccountsFile) -> io::Result<()> {
        let key = accounts_store_key(&self.code_home);
        let serialized = serde_json::to_string(data).map_err(std::io::Error::other)?;
        self.save_to_keyring(&key, &serialized)?;
        if let Err(error) = delete_accounts_files_if_exists(&self.code_home, &self.paths) {
            tracing::warn!("failed to remove auth accounts fallback file: {error}");
        }
        Ok(())
    }
}

#[derive(Clone)]
struct AutoAccountsStorage {
    keyring_storage: Arc<KeyringAccountsStorage>,
    file_storage: Arc<FileAccountsStorage>,
}

impl AutoAccountsStorage {
    fn new(code_home: PathBuf, paths: AccountStorePaths, keyring_store: Arc<dyn KeyringStore>) -> Self {
        Self {
            keyring_storage: Arc::new(KeyringAccountsStorage::new(
                code_home,
                paths.clone(),
                keyring_store,
            )),
            file_storage: Arc::new(FileAccountsStorage::new(paths)),
        }
    }
}

impl AccountsStorageBackend for AutoAccountsStorage {
    fn load(&self) -> io::Result<AccountsFile> {
        let key = accounts_store_key(&self.keyring_storage.code_home);
        match self.keyring_storage.load_from_keyring(&key) {
            Ok(Some(data)) => Ok(data),
            Ok(None) => self.file_storage.load(),
            Err(error) => {
                tracing::warn!(
                    "failed to load accounts from keyring, falling back to file: {error}"
                );
                self.file_storage.load()
            }
        }
    }

    fn save(&self, data: &AccountsFile) -> io::Result<()> {
        match self.keyring_storage.save(data) {
            Ok(()) => Ok(()),
            Err(error) => {
                tracing::warn!(
                    "failed to save accounts to keyring, falling back to file: {error}"
                );
                self.file_storage.save(data)
            }
        }
    }
}

static EPHEMERAL_ACCOUNTS_STORE: Lazy<Mutex<HashMap<String, AccountsFile>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
struct EphemeralAccountsStorage {
    code_home: PathBuf,
    paths: AccountStorePaths,
}

impl EphemeralAccountsStorage {
    fn new(code_home: PathBuf, paths: AccountStorePaths) -> Self {
        Self { code_home, paths }
    }

    fn with_store<F, T>(&self, action: F) -> io::Result<T>
    where
        F: FnOnce(&mut HashMap<String, AccountsFile>, String) -> io::Result<T>,
    {
        let key = accounts_store_key(&self.code_home);
        let mut store = EPHEMERAL_ACCOUNTS_STORE
            .lock()
            .map_err(|_| io::Error::other("failed to lock ephemeral accounts storage"))?;
        action(&mut store, key)
    }
}

impl AccountsStorageBackend for EphemeralAccountsStorage {
    fn load(&self) -> io::Result<AccountsFile> {
        self.with_store(|store, key| Ok(store.get(&key).cloned().unwrap_or_default()))
    }

    fn save(&self, data: &AccountsFile) -> io::Result<()> {
        self.with_store(|store, key| {
            store.insert(key, data.clone());
            Ok(())
        })?;
        if let Err(error) = delete_accounts_files_if_exists(&self.code_home, &self.paths) {
            tracing::warn!("failed to remove auth accounts fallback file: {error}");
        }
        Ok(())
    }
}

fn accounts_storage_with_mode(
    code_home: &Path,
    mode: AuthCredentialsStoreMode,
) -> Arc<dyn AccountsStorageBackend> {
    let paths = account_store_paths(code_home);
    match mode {
        AuthCredentialsStoreMode::File => Arc::new(FileAccountsStorage::new(paths)),
        AuthCredentialsStoreMode::Keyring => Arc::new(KeyringAccountsStorage::new(
            code_home.to_path_buf(),
            paths,
            Arc::new(DefaultKeyringStore),
        )),
        AuthCredentialsStoreMode::Auto => Arc::new(AutoAccountsStorage::new(
            code_home.to_path_buf(),
            paths,
            Arc::new(DefaultKeyringStore),
        )),
        AuthCredentialsStoreMode::Ephemeral => Arc::new(EphemeralAccountsStorage::new(
            code_home.to_path_buf(),
            paths,
        )),
    }
}

fn accounts_storage(code_home: &Path) -> Arc<dyn AccountsStorageBackend> {
    accounts_storage_with_mode(code_home, auth_credentials_store_mode(code_home))
}

pub fn migrate_accounts_store_mode(
    code_home: &Path,
    from: AuthCredentialsStoreMode,
    to: AuthCredentialsStoreMode,
) -> io::Result<()> {
    if from == to {
        return Ok(());
    }

    let from_storage = accounts_storage_with_mode(code_home, from);
    let to_storage = accounts_storage_with_mode(code_home, to);
    let data = from_storage.load()?;
    to_storage.save(&data)?;
    Ok(())
}

fn read_accounts_file(path: &Path) -> io::Result<Option<AccountsFile>> {
    match File::open(path) {
        Ok(mut file) => {
            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            let parsed: AccountsFile = serde_json::from_str(&contents)?;
            Ok(Some(parsed))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn load_accounts_file(paths: &AccountStorePaths) -> io::Result<AccountsFile> {
    for path in &paths.read_paths {
        if let Some(data) = read_accounts_file(path)? {
            return Ok(data);
        }
    }
    Ok(AccountsFile::default())
}

fn write_accounts_file(path: &Path, data: &AccountsFile) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }

    let json = serde_json::to_string_pretty(data)?;
    let mut options = OpenOptions::new();
    options.truncate(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(json.as_bytes())?;
    file.flush()?;
    Ok(())
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn now() -> DateTime<Utc> {
    Utc::now()
}

fn next_id() -> String {
    Uuid::new_v4().to_string()
}

fn match_chatgpt_account(existing: &StoredAccount, tokens: &TokenData) -> bool {
    if !existing.mode.is_chatgpt() {
        return false;
    }

    let existing_tokens = match &existing.tokens {
        Some(tokens) => tokens,
        None => return false,
    };

    let account_id_matches = match (&existing_tokens.account_id, &tokens.account_id) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };

    let email_matches = match (
        existing_tokens.id_token.email.as_ref(),
        tokens.id_token.email.as_ref(),
    ) {
        (Some(a), Some(b)) => normalize_email(a) == normalize_email(b),
        _ => false,
    };

    account_id_matches && email_matches
}

fn match_api_key_account(existing: &StoredAccount, api_key: &str) -> bool {
    existing.mode == AuthMode::ApiKey
        && existing
            .openai_api_key
            .as_ref()
            .is_some_and(|stored| stored == api_key)
}

fn touch_account(account: &mut StoredAccount, used: bool) {
    if account.created_at.is_none() {
        account.created_at = Some(now());
    }
    if used {
        account.last_used_at = Some(now());
    }
}

fn upsert_account(mut data: AccountsFile, mut new_account: StoredAccount) -> (AccountsFile, StoredAccount) {
    let existing_idx = match new_account.mode {
        AuthMode::ChatGPT | AuthMode::ChatgptAuthTokens => new_account
            .tokens
            .as_ref()
            .and_then(|tokens| data.accounts.iter().position(|acc| match_chatgpt_account(acc, tokens))),
        AuthMode::ApiKey => new_account
            .openai_api_key
            .as_ref()
            .and_then(|api_key| data.accounts.iter().position(|acc| match_api_key_account(acc, api_key))),
    };

    if let Some(idx) = existing_idx {
        let mut account = data.accounts[idx].clone();
        if new_account.label.is_some() {
            account.label = new_account.label;
        }
        if new_account.last_refresh.is_some() {
            account.last_refresh = new_account.last_refresh;
        }
        if let Some(tokens) = new_account.tokens {
            account.tokens = Some(tokens);
        }
        if let Some(api_key) = new_account.openai_api_key {
            account.openai_api_key = Some(api_key);
        }
        if let Some(last_used) = new_account.last_used_at {
            account.last_used_at = Some(last_used);
        }
        data.accounts[idx] = account.clone();
        return (data, account);
    }

    if new_account.created_at.is_none() {
        new_account.created_at = Some(now());
    }

    data.accounts.push(new_account.clone());
    (data, new_account)
}

pub fn list_accounts(code_home: &Path) -> io::Result<Vec<StoredAccount>> {
    let storage = accounts_storage(code_home);
    let data = storage.load()?;
    Ok(data.accounts)
}

pub fn get_active_account_id(code_home: &Path) -> io::Result<Option<String>> {
    let storage = accounts_storage(code_home);
    let data = storage.load()?;
    Ok(data.active_account_id)
}

pub fn find_account(code_home: &Path, account_id: &str) -> io::Result<Option<StoredAccount>> {
    let storage = accounts_storage(code_home);
    let data = storage.load()?;
    Ok(data
        .accounts
        .into_iter()
        .find(|acc| acc.id == account_id))
}

pub fn set_active_account_id(
    code_home: &Path,
    account_id: Option<String>,
) -> io::Result<Option<StoredAccount>> {
    let storage = accounts_storage(code_home);
    let mut data = storage.load()?;

    data.active_account_id = account_id.clone();

    let updated = account_id.and_then(|id| {
        data.accounts
            .iter_mut()
            .find(|account| account.id == id)
            .map(|account| {
                touch_account(account, true);
                account.clone()
            })
    });

    storage.save(&data)?;
    Ok(updated)
}

pub fn remove_account(code_home: &Path, account_id: &str) -> io::Result<Option<StoredAccount>> {
    let storage = accounts_storage(code_home);
    let mut data = storage.load()?;

    let removed = if let Some(pos) = data.accounts.iter().position(|acc| acc.id == account_id) {
        Some(data.accounts.remove(pos))
    } else {
        None
    };

    if data
        .active_account_id
        .as_ref()
        .is_some_and(|active| active == account_id)
    {
        data.active_account_id = None;
    }

    storage.save(&data)?;
    Ok(removed)
}

pub fn upsert_api_key_account(
    code_home: &Path,
    api_key: String,
    label: Option<String>,
    make_active: bool,
) -> io::Result<StoredAccount> {
    let storage = accounts_storage(code_home);
    let data = storage.load()?;

    let new_account = StoredAccount {
        id: next_id(),
        mode: AuthMode::ApiKey,
        label,
        openai_api_key: Some(api_key),
        tokens: None,
        last_refresh: None,
        created_at: None,
        last_used_at: None,
    };

    let (mut data, mut stored) = upsert_account(data, new_account);

    if make_active {
        data.active_account_id = Some(stored.id.clone());
        if let Some(account) = data
            .accounts
            .iter_mut()
            .find(|acc| acc.id == stored.id)
        {
            touch_account(account, true);
            stored = account.clone();
        }
    }

    storage.save(&data)?;
    Ok(stored)
}

pub fn upsert_chatgpt_account(
    code_home: &Path,
    tokens: TokenData,
    last_refresh: DateTime<Utc>,
    label: Option<String>,
    make_active: bool,
) -> io::Result<StoredAccount> {
    let storage = accounts_storage(code_home);
    let data = storage.load()?;

    let new_account = StoredAccount {
        id: next_id(),
        mode: AuthMode::ChatGPT,
        label,
        openai_api_key: None,
        tokens: Some(tokens),
        last_refresh: Some(last_refresh),
        created_at: None,
        last_used_at: None,
    };

    let (mut data, mut stored) = upsert_account(data, new_account);

    if make_active {
        data.active_account_id = Some(stored.id.clone());
        if let Some(account) = data
            .accounts
            .iter_mut()
            .find(|acc| acc.id == stored.id)
        {
            touch_account(account, true);
            stored = account.clone();
        }
    }

    storage.save(&data)?;
    Ok(stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use crate::token_data::{IdTokenInfo, TokenData};
    use std::fs;
    use tempfile::tempdir;

    fn make_chatgpt_tokens(account_id: Option<&str>, email: Option<&str>) -> TokenData {
        fn fake_jwt(account_id: Option<&str>, email: Option<&str>, plan: &str) -> String {
            #[derive(Serialize)]
            struct Header {
                alg: &'static str,
                typ: &'static str,
            }
            let header = Header {
                alg: "none",
                typ: "JWT",
            };
            let payload = serde_json::json!({
                "email": email,
                "https://api.openai.com/auth": {
                    "chatgpt_plan_type": plan,
                    "chatgpt_account_id": account_id.unwrap_or("acct"),
                    "chatgpt_user_id": "user-12345",
                    "user_id": "user-12345",
                }
            });
            let b64 = |value: &serde_json::Value| {
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .encode(serde_json::to_vec(value).expect("json to vec"))
            };
            let header_b64 = b64(&serde_json::to_value(header).expect("header value"));
            let payload_b64 = b64(&payload);
            let signature_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"sig");
            format!("{header_b64}.{payload_b64}.{signature_b64}")
        }

        TokenData {
            id_token: IdTokenInfo {
                email: email.map(std::string::ToString::to_string),
                chatgpt_plan_type: None,
                raw_jwt: fake_jwt(account_id, email, "pro"),
            },
            access_token: "access".to_string(),
            refresh_token: "refresh".to_string(),
            account_id: account_id.map(std::string::ToString::to_string),
        }
    }

    fn write_accounts_store(path: &Path, accounts: Vec<StoredAccount>) {
        let data = AccountsFile {
            version: 1,
            active_account_id: accounts.first().map(|account| account.id.clone()),
            accounts,
        };
        write_accounts_file(path, &data).expect("write accounts store");
    }

    #[test]
    fn uses_configured_account_store_paths() {
        let home = tempdir().expect("tempdir");
        let custom_store = home.path().join("custom/accounts_store.json");
        let existing = StoredAccount {
            id: "existing-account".to_string(),
            mode: AuthMode::ApiKey,
            label: Some("existing".to_string()),
            openai_api_key: Some("sk-existing".to_string()),
            tokens: None,
            last_refresh: None,
            created_at: Some(Utc::now()),
            last_used_at: Some(Utc::now()),
        };
        write_accounts_store(&custom_store, vec![existing.clone()]);

        fs::write(
            home.path().join("config.toml"),
            r#"
[accounts]
read_paths = ["custom/accounts_store.json"]
write_path = "custom/accounts_store.json"
"#,
        )
        .expect("write config");

        let loaded = list_accounts(home.path()).expect("list configured accounts");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, existing.id);

        upsert_api_key_account(home.path(), "sk-new".to_string(), None, false)
            .expect("upsert to configured path");

        let custom_contents =
            fs::read_to_string(&custom_store).expect("read configured store");
        assert!(
            custom_contents.contains("sk-new"),
            "new account should be written to configured path"
        );

        let default_store = home.path().join(ACCOUNTS_FILE_NAME);
        assert!(
            !default_store.exists(),
            "default account store should remain unused when write_path is configured"
        );
    }

    #[test]
    fn upsert_api_key_creates_and_updates() {
        let home = tempdir().expect("tempdir");
        let api_key = "sk-test".to_string();
        let stored = upsert_api_key_account(home.path(), api_key.clone(), None, true)
            .expect("upsert api key");

        assert_eq!(stored.mode, AuthMode::ApiKey);
        assert_eq!(stored.openai_api_key.as_deref(), Some("sk-test"));

        let again = upsert_api_key_account(home.path(), api_key, None, false)
            .expect("upsert same key");
        assert_eq!(stored.id, again.id);

        let accounts = list_accounts(home.path()).expect("list accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, stored.id);
    }

    #[test]
    fn upsert_chatgpt_dedupes_by_account_id() {
        let home = tempdir().expect("tempdir");
        let tokens = make_chatgpt_tokens(Some("acct-1"), Some("user@example.com"));
        let stored = upsert_chatgpt_account(
            home.path(),
            tokens,
            Utc::now(),
            None,
            true,
        )
        .expect("insert chatgpt");

        let tokens_updated = make_chatgpt_tokens(Some("acct-1"), Some("user@example.com"));
        let again = upsert_chatgpt_account(
            home.path(),
            tokens_updated,
            Utc::now(),
            None,
            false,
        )
        .expect("update chatgpt");

        assert_eq!(stored.id, again.id);
        let accounts = list_accounts(home.path()).expect("list accounts");
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].id, stored.id);
    }

    #[test]
    fn chatgpt_accounts_with_same_email_but_different_ids_are_distinct() {
        let home = tempdir().expect("tempdir");

        let personal = make_chatgpt_tokens(Some("acct-personal"), Some("user@example.com"));
        let personal_id = upsert_chatgpt_account(
            home.path(),
            personal,
            Utc::now(),
            None,
            true,
        )
        .expect("insert personal account")
        .id;

        let team = make_chatgpt_tokens(Some("acct-team"), Some("user@example.com"));
        let team_id = upsert_chatgpt_account(
            home.path(),
            team,
            Utc::now(),
            None,
            false,
        )
        .expect("insert team account")
        .id;

        assert_ne!(personal_id, team_id, "accounts with different IDs should not be merged");

        let accounts = list_accounts(home.path()).expect("list accounts");
        assert_eq!(accounts.len(), 2, "both accounts should remain listed");
    }

    #[test]
    fn remove_account_clears_active() {
        let home = tempdir().expect("tempdir");
        let tokens = make_chatgpt_tokens(Some("acct-remove"), Some("user@example.com"));
        let stored = upsert_chatgpt_account(
            home.path(),
            tokens,
            Utc::now(),
            None,
            true,
        )
        .expect("insert chatgpt");

        let active_before = get_active_account_id(home.path()).expect("active id");
        assert_eq!(active_before.as_deref(), Some(stored.id.as_str()));

        let removed = remove_account(home.path(), &stored.id).expect("remove");
        assert!(removed.is_some());

        let active_after = get_active_account_id(home.path()).expect("active id");
        assert!(active_after.is_none());
    }
}
