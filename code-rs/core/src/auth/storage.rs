use std::collections::HashMap;
use std::fmt::Debug;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tracing::warn;

use crate::config::resolve_code_path_for_read;
use crate::config_types::AuthCredentialsStoreMode;
use code_keyring_store::DefaultKeyringStore;
use code_keyring_store::KeyringStore;
use code_keyring_store::store_key_for_code_home;
use once_cell::sync::Lazy;

use super::AuthDotJson;

pub(super) fn get_auth_file(code_home: &Path) -> PathBuf {
    code_home.join("auth.json")
}

fn delete_file_if_exists(path: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

fn delete_auth_files_if_exists(code_home: &Path) -> std::io::Result<bool> {
    let write_path = get_auth_file(code_home);
    let read_path = resolve_code_path_for_read(code_home, Path::new("auth.json"));
    let write_removed = delete_file_if_exists(&write_path)?;
    let read_removed = if read_path != write_path {
        delete_file_if_exists(&read_path)?
    } else {
        false
    };
    Ok(write_removed || read_removed)
}

pub(super) trait AuthStorageBackend: Debug + Send + Sync {
    fn load(&self) -> std::io::Result<Option<AuthDotJson>>;
    fn save(&self, auth: &AuthDotJson) -> std::io::Result<()>;
    fn delete(&self) -> std::io::Result<bool>;
}

#[derive(Clone, Debug)]
pub(super) struct FileAuthStorage {
    code_home: PathBuf,
}

impl FileAuthStorage {
    pub(super) fn new(code_home: PathBuf) -> Self {
        Self { code_home }
    }

    fn try_read_auth_json(&self, auth_file: &Path) -> std::io::Result<AuthDotJson> {
        let mut file = File::open(auth_file)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        let auth_dot_json: AuthDotJson = serde_json::from_str(&contents)?;
        Ok(auth_dot_json)
    }
}

impl AuthStorageBackend for FileAuthStorage {
    fn load(&self) -> std::io::Result<Option<AuthDotJson>> {
        let auth_file = resolve_code_path_for_read(&self.code_home, Path::new("auth.json"));
        let auth_dot_json = match self.try_read_auth_json(&auth_file) {
            Ok(auth) => auth,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        Ok(Some(auth_dot_json))
    }

    fn save(&self, auth_dot_json: &AuthDotJson) -> std::io::Result<()> {
        let auth_file = get_auth_file(&self.code_home);

        if let Some(parent) = auth_file.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json_data = serde_json::to_string_pretty(auth_dot_json)?;
        let mut options = OpenOptions::new();
        options.truncate(true).write(true).create(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut file = options.open(auth_file)?;
        file.write_all(json_data.as_bytes())?;
        file.flush()?;
        Ok(())
    }

    fn delete(&self) -> std::io::Result<bool> {
        delete_auth_files_if_exists(&self.code_home)
    }
}

const KEYRING_SERVICE: &str = "Codex Auth";

fn compute_store_key(code_home: &Path) -> String {
    store_key_for_code_home("cli", code_home)
}

#[derive(Clone, Debug)]
struct KeyringAuthStorage {
    code_home: PathBuf,
    keyring_store: Arc<dyn KeyringStore>,
}

impl KeyringAuthStorage {
    fn new(code_home: PathBuf, keyring_store: Arc<dyn KeyringStore>) -> Self {
        Self {
            code_home,
            keyring_store,
        }
    }

    fn load_from_keyring(&self, key: &str) -> std::io::Result<Option<AuthDotJson>> {
        match self.keyring_store.load(KEYRING_SERVICE, key) {
            Ok(Some(serialized)) => serde_json::from_str(&serialized).map(Some).map_err(|err| {
                std::io::Error::other(format!(
                    "failed to deserialize CLI auth from keyring: {err}"
                ))
            }),
            Ok(None) => Ok(None),
            Err(error) => Err(std::io::Error::other(format!(
                "failed to load CLI auth from keyring: {}",
                error.message()
            ))),
        }
    }

    fn save_to_keyring(&self, key: &str, value: &str) -> std::io::Result<()> {
        match self.keyring_store.save(KEYRING_SERVICE, key, value) {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = format!("failed to save CLI auth to keyring: {}", error.message());
                warn!("{message}");
                Err(std::io::Error::other(message))
            }
        }
    }
}

impl AuthStorageBackend for KeyringAuthStorage {
    fn load(&self) -> std::io::Result<Option<AuthDotJson>> {
        let key = compute_store_key(&self.code_home);
        self.load_from_keyring(&key)
    }

    fn save(&self, auth: &AuthDotJson) -> std::io::Result<()> {
        let key = compute_store_key(&self.code_home);
        let serialized = serde_json::to_string(auth).map_err(std::io::Error::other)?;
        self.save_to_keyring(&key, &serialized)?;
        if let Err(err) = delete_auth_files_if_exists(&self.code_home) {
            warn!("failed to remove CLI auth fallback file: {err}");
        }
        Ok(())
    }

    fn delete(&self) -> std::io::Result<bool> {
        let key = compute_store_key(&self.code_home);
        let keyring_removed = self
            .keyring_store
            .delete(KEYRING_SERVICE, &key)
            .map_err(|err| {
                std::io::Error::other(format!("failed to delete auth from keyring: {err}"))
            })?;
        let file_removed = delete_auth_files_if_exists(&self.code_home)?;
        Ok(keyring_removed || file_removed)
    }
}

#[derive(Clone, Debug)]
struct AutoAuthStorage {
    keyring_storage: Arc<KeyringAuthStorage>,
    file_storage: Arc<FileAuthStorage>,
}

impl AutoAuthStorage {
    fn new(code_home: PathBuf, keyring_store: Arc<dyn KeyringStore>) -> Self {
        Self {
            keyring_storage: Arc::new(KeyringAuthStorage::new(code_home.clone(), keyring_store)),
            file_storage: Arc::new(FileAuthStorage::new(code_home)),
        }
    }
}

impl AuthStorageBackend for AutoAuthStorage {
    fn load(&self) -> std::io::Result<Option<AuthDotJson>> {
        match self.keyring_storage.load() {
            Ok(Some(auth)) => Ok(Some(auth)),
            Ok(None) => self.file_storage.load(),
            Err(err) => {
                warn!("failed to load auth from keyring, falling back to file: {err}");
                self.file_storage.load()
            }
        }
    }

    fn save(&self, auth: &AuthDotJson) -> std::io::Result<()> {
        match self.keyring_storage.save(auth) {
            Ok(()) => Ok(()),
            Err(err) => {
                warn!("failed to save auth to keyring, falling back to file: {err}");
                self.file_storage.save(auth)
            }
        }
    }

    fn delete(&self) -> std::io::Result<bool> {
        // Keyring storage deletes fallback files as well.
        self.keyring_storage.delete()
    }
}

static EPHEMERAL_AUTH_STORE: Lazy<Mutex<HashMap<String, AuthDotJson>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug)]
struct EphemeralAuthStorage {
    code_home: PathBuf,
}

impl EphemeralAuthStorage {
    fn new(code_home: PathBuf) -> Self {
        Self { code_home }
    }

    fn with_store<F, T>(&self, action: F) -> std::io::Result<T>
    where
        F: FnOnce(&mut HashMap<String, AuthDotJson>, String) -> std::io::Result<T>,
    {
        let key = compute_store_key(&self.code_home);
        let mut store = EPHEMERAL_AUTH_STORE
            .lock()
            .map_err(|_| std::io::Error::other("failed to lock ephemeral auth storage"))?;
        action(&mut store, key)
    }
}

impl AuthStorageBackend for EphemeralAuthStorage {
    fn load(&self) -> std::io::Result<Option<AuthDotJson>> {
        self.with_store(|store, key| Ok(store.get(&key).cloned()))
    }

    fn save(&self, auth: &AuthDotJson) -> std::io::Result<()> {
        self.with_store(|store, key| {
            store.insert(key, auth.clone());
            Ok(())
        })
    }

    fn delete(&self) -> std::io::Result<bool> {
        self.with_store(|store, key| Ok(store.remove(&key).is_some()))
    }
}

pub(super) fn create_auth_storage(
    code_home: PathBuf,
    mode: AuthCredentialsStoreMode,
) -> Arc<dyn AuthStorageBackend> {
    let keyring_store: Arc<dyn KeyringStore> = Arc::new(DefaultKeyringStore);
    create_auth_storage_with_keyring_store(code_home, mode, keyring_store)
}

fn create_auth_storage_with_keyring_store(
    code_home: PathBuf,
    mode: AuthCredentialsStoreMode,
    keyring_store: Arc<dyn KeyringStore>,
) -> Arc<dyn AuthStorageBackend> {
    match mode {
        AuthCredentialsStoreMode::File => Arc::new(FileAuthStorage::new(code_home)),
        AuthCredentialsStoreMode::Keyring => {
            Arc::new(KeyringAuthStorage::new(code_home, keyring_store))
        }
        AuthCredentialsStoreMode::Auto => Arc::new(AutoAuthStorage::new(code_home, keyring_store)),
        AuthCredentialsStoreMode::Ephemeral => Arc::new(EphemeralAuthStorage::new(code_home)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token_data::IdTokenInfo;
    use base64::Engine;
    use pretty_assertions::assert_eq;
    use serde::Serialize;
    use serde_json::json;
    use tempfile::tempdir;

    use code_app_server_protocol::AuthMode;
    use code_keyring_store::tests::MockKeyringStore;
    use keyring::Error as KeyringError;

    #[test]
    fn file_storage_load_returns_auth_dot_json() -> anyhow::Result<()> {
        let code_home = tempdir()?;
        let storage = FileAuthStorage::new(code_home.path().to_path_buf());
        let auth_dot_json = AuthDotJson {
            auth_mode: Some(AuthMode::ApiKey),
            openai_api_key: Some("test-key".to_string()),
            tokens: None,
            last_refresh: Some(chrono::Utc::now()),
        };

        storage.save(&auth_dot_json)?;
        let loaded = storage.load()?;
        assert_eq!(Some(auth_dot_json), loaded);
        Ok(())
    }

    #[test]
    fn ephemeral_storage_save_load_delete_is_in_memory_only() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let storage = create_auth_storage(
            dir.path().to_path_buf(),
            AuthCredentialsStoreMode::Ephemeral,
        );
        let auth_dot_json = AuthDotJson {
            auth_mode: Some(AuthMode::ApiKey),
            openai_api_key: Some("sk-ephemeral".to_string()),
            tokens: None,
            last_refresh: Some(chrono::Utc::now()),
        };

        storage.save(&auth_dot_json)?;
        let loaded = storage.load()?;
        assert_eq!(Some(auth_dot_json), loaded);

        let removed = storage.delete()?;
        assert!(removed);
        let loaded = storage.load()?;
        assert_eq!(None, loaded);
        assert!(!get_auth_file(dir.path()).exists());
        Ok(())
    }

    fn id_token_with_prefix(prefix: &str) -> IdTokenInfo {
        #[derive(Serialize)]
        struct Header {
            alg: &'static str,
            typ: &'static str,
        }

        let header = Header {
            alg: "none",
            typ: "JWT",
        };
        let payload = json!({
            "email": format!("{prefix}@example.com"),
            "https://api.openai.com/auth": {
                "chatgpt_account_id": format!("{prefix}-account"),
            },
        });
        let encode = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let header_b64 = encode(&serde_json::to_vec(&header).expect("serialize header"));
        let payload_b64 = encode(&serde_json::to_vec(&payload).expect("serialize payload"));
        let signature_b64 = encode(b"sig");
        let fake_jwt = format!("{header_b64}.{payload_b64}.{signature_b64}");

        crate::token_data::parse_id_token(&fake_jwt).expect("fake JWT should parse")
    }

    fn auth_with_prefix(prefix: &str) -> AuthDotJson {
        AuthDotJson {
            auth_mode: Some(AuthMode::ApiKey),
            openai_api_key: Some(format!("{prefix}-api-key")),
            tokens: Some(crate::token_data::TokenData {
                id_token: id_token_with_prefix(prefix),
                access_token: format!("{prefix}-access"),
                refresh_token: format!("{prefix}-refresh"),
                account_id: Some(format!("{prefix}-account-id")),
            }),
            last_refresh: None,
        }
    }

    #[test]
    fn keyring_storage_load_returns_deserialized_auth() -> anyhow::Result<()> {
        let code_home = tempdir()?;
        let mock_keyring = MockKeyringStore::default();
        let storage = KeyringAuthStorage::new(
            code_home.path().to_path_buf(),
            Arc::new(mock_keyring.clone()),
        );
        let expected = auth_with_prefix("load");
        let key = compute_store_key(code_home.path());
        let serialized = serde_json::to_string(&expected)?;
        mock_keyring.save(KEYRING_SERVICE, &key, &serialized)?;

        let loaded = storage.load()?;
        assert_eq!(Some(expected), loaded);
        Ok(())
    }

    #[test]
    fn keyring_storage_save_persists_and_removes_fallback_file() -> anyhow::Result<()> {
        let code_home = tempdir()?;
        let mock_keyring = MockKeyringStore::default();
        let storage: Arc<dyn AuthStorageBackend> = Arc::new(KeyringAuthStorage::new(
            code_home.path().to_path_buf(),
            Arc::new(mock_keyring.clone()),
        ));

        let key = compute_store_key(code_home.path());
        let fallback = get_auth_file(code_home.path());
        std::fs::write(&fallback, "stale")?;

        let expected = auth_with_prefix("save");
        storage.save(&expected)?;

        let saved_value = mock_keyring
            .saved_value(&key)
            .expect("keyring entry should exist");
        let expected_serialized = serde_json::to_string(&expected)?;
        assert_eq!(saved_value, expected_serialized);
        assert!(!fallback.exists(), "fallback auth.json should be removed");
        Ok(())
    }

    #[test]
    fn auto_storage_load_falls_back_when_keyring_errors() -> anyhow::Result<()> {
        let code_home = tempdir()?;
        let mock_keyring = MockKeyringStore::default();
        let key = compute_store_key(code_home.path());
        mock_keyring.set_error(
            &key,
            KeyringError::NoStorageAccess(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "keyring unavailable",
            ))),
        );

        let storage: Arc<dyn AuthStorageBackend> = Arc::new(AutoAuthStorage::new(
            code_home.path().to_path_buf(),
            Arc::new(mock_keyring.clone()),
        ));

        let expected = auth_with_prefix("file");
        let file_storage = FileAuthStorage::new(code_home.path().to_path_buf());
        file_storage.save(&expected)?;

        let loaded = storage.load()?;
        assert_eq!(Some(expected), loaded);
        Ok(())
    }
}
