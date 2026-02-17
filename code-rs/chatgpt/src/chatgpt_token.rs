use code_core::CodexAuth;
use code_app_server_protocol::AuthMode;
use code_core::auth::AuthCredentialsStoreMode;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::RwLock;

use code_core::token_data::TokenData;

static CHATGPT_TOKEN: LazyLock<RwLock<Option<TokenData>>> = LazyLock::new(|| RwLock::new(None));

pub fn get_chatgpt_token_data() -> Option<TokenData> {
    CHATGPT_TOKEN.read().ok()?.clone()
}

pub fn set_chatgpt_token_data(value: TokenData) {
    if let Ok(mut guard) = CHATGPT_TOKEN.write() {
        *guard = Some(value);
    }
}

/// Initialize the ChatGPT token from auth.json file
pub async fn init_chatgpt_token_from_auth(
    code_home: &Path,
    auth_credentials_store_mode: AuthCredentialsStoreMode,
    originator: &str,
) -> std::io::Result<()> {
    let auth = CodexAuth::from_code_home_with_store_mode(
        code_home,
        auth_credentials_store_mode,
        AuthMode::ChatGPT,
        originator,
    )?;
    if let Some(auth) = auth {
        let token_data = auth.get_token_data().await?;
        set_chatgpt_token_data(token_data);
    }
    Ok(())
}
