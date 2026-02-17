use sha2::Digest;
use sha2::Sha256;
use std::path::Path;

pub(crate) fn store_key_for_code_home(prefix: &str, code_home: &Path) -> String {
    let canonical = code_home
        .canonicalize()
        .unwrap_or_else(|_| code_home.to_path_buf());
    let path_str = canonical.to_string_lossy();
    let mut hasher = Sha256::new();
    hasher.update(path_str.as_bytes());
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    let truncated = hex.get(..16).unwrap_or(&hex);
    format!("{prefix}|{truncated}")
}

