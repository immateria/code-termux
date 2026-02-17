#[cfg(target_os = "windows")]
fn default_pathext_or_default() -> Vec<String> {
    std::env::var("PATHEXT")
        .ok()
        .filter(|v| !v.is_empty())
        .map(|v| {
            v.split(';')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_ascii_lowercase())
                .collect()
        })
        // Keep a sane default set even if PATHEXT is missing or empty. Include
        // .ps1 because PowerShell users can invoke scripts without specifying
        // the extension; CreateProcess still resolves fine when we provide the
        // full path with extension.
        .unwrap_or_else(|| vec![
            ".com".into(),
            ".exe".into(),
            ".bat".into(),
            ".cmd".into(),
            ".ps1".into(),
        ])
}

#[cfg(target_os = "windows")]
fn resolve_in_path(command: &str) -> Option<std::path::PathBuf> {
    use std::path::Path;

    let cmd_path = Path::new(command);

    // Absolute or contains separators: respect it directly if it points to a file.
    if cmd_path.is_absolute() || command.contains(['\\', '/']) {
        if cmd_path.is_file() {
            return Some(cmd_path.to_path_buf());
        }
    }

    // Search PATH with PATHEXT semantics and return the first hit.
    let exts = default_pathext_or_default();
    let Some(path_os) = std::env::var_os("PATH") else { return None; };
    let has_ext = cmd_path.extension().is_some();
    for dir in std::env::split_paths(&path_os) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        if has_ext {
            let candidate = dir.join(command);
            if candidate.is_file() {
                return Some(candidate);
            }
        } else {
            for ext in &exts {
                let candidate = dir.join(format!("{command}{ext}"));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn is_executable_file(path: &std::path::Path) -> bool {
    path.is_file()
}

#[cfg(not(target_os = "windows"))]
fn is_executable_file(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };

    meta.is_file() && (meta.permissions().mode() & 0o111 != 0)
}

#[cfg(target_os = "windows")]
fn resolve_existing_path_with_pathext(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if is_executable_file(path) {
        return Some(path.to_path_buf());
    }

    if path.extension().is_some() {
        return None;
    }

    let Some(file_name) = path.file_name() else {
        return None;
    };

    let base = file_name.to_os_string();
    for ext in default_pathext_or_default() {
        let mut name = base.clone();
        name.push(ext);
        let candidate = path.with_file_name(name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn resolve_explicit_command_path(path: &std::path::Path) -> Option<std::path::PathBuf> {
    resolve_existing_path_with_pathext(path)
}

#[cfg(not(target_os = "windows"))]
fn resolve_explicit_command_path(path: &std::path::Path) -> Option<std::path::PathBuf> {
    is_executable_file(path).then_some(path.to_path_buf())
}

#[cfg(target_os = "windows")]
fn resolve_command_on_path(command: &str) -> Option<std::path::PathBuf> {
    resolve_in_path(command)
}

#[cfg(not(target_os = "windows"))]
fn resolve_command_on_path(command: &str) -> Option<std::path::PathBuf> {
    let path_os = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_os) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(command);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn home_fallback_command_candidates(command: &str) -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();

    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    if let Some(home) = home.as_ref() {
        candidates.push(home.join(".local/bin").join(command));
        candidates.push(home.join(".n/bin").join(command));
        candidates.push(home.join(".npm-global/bin").join(command));
    }

    if command.eq_ignore_ascii_case("claude") {
        if let Some(claude_config_dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
            candidates.push(std::path::PathBuf::from(claude_config_dir).join("local").join(command));
        }
        if let Some(home) = home {
            candidates.push(home.join(".claude/local").join(command));
        }
    }

    candidates
}

pub(crate) fn resolve_external_agent_command_path(command: &str) -> Option<std::path::PathBuf> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.contains(std::path::MAIN_SEPARATOR) || trimmed.contains('/') || trimmed.contains('\\') {
        let path = std::path::PathBuf::from(trimmed);
        return resolve_explicit_command_path(&path);
    }

    if let Some(path) = resolve_command_on_path(trimmed) {
        return Some(path);
    }

    for candidate in home_fallback_command_candidates(trimmed) {
        if let Some(path) = resolve_explicit_command_path(&candidate) {
            return Some(path);
        }
    }

    None
}

pub fn external_agent_command_exists(command: &str) -> bool {
    resolve_external_agent_command_path(command).is_some()
}

use crate::agent_defaults::agent_model_spec;
use crate::config_types::AgentConfig;

pub(crate) fn current_code_binary_path() -> Result<std::path::PathBuf, String> {
    if let Ok(path) = std::env::var("CODE_BINARY_PATH") {
        let p = std::path::PathBuf::from(path);
        if !p.exists() {
            return Err(format!(
                "CODE_BINARY_PATH points to '{}' but that file is missing. Rebuild with ./build-fast.sh or update CODE_BINARY_PATH.",
                p.display()
            ));
        }
        return Ok(p);
    }
    let exe = std::env::current_exe().map_err(|e| format!("Failed to resolve current executable: {e}"))?;

    // If the kernel reports the path as "(deleted)", strip the suffix and prefer the live file
    // at the same location (common when a rebuild replaces the inode under a long-running process).
    let cleaned = strip_deleted_suffix(&exe);
    if cleaned.exists() {
        return Ok(cleaned);
    }

    if let Some(fallback) = fallback_code_binary_path() {
        return Ok(fallback);
    }

    Err(format!(
        "Current code binary is missing on disk ({}). It may have been deleted while running. Rebuild with ./build-fast.sh or reinstall 'code' to continue.",
        exe.display()
    ))
}

fn strip_deleted_suffix(path: &std::path::Path) -> std::path::PathBuf {
    const DELETED_SUFFIX: &str = " (deleted)";
    let s = path.to_string_lossy();
    if let Some(stripped) = s.strip_suffix(DELETED_SUFFIX) {
        return std::path::PathBuf::from(stripped);
    }
    path.to_path_buf()
}

fn fallback_code_binary_path() -> Option<std::path::PathBuf> {
    // If the running binary was pruned (e.g., shared target cache rotation), try to locate
    // a fresh dev build in the repository, and if missing, trigger a quick rebuild.
    let repo_root = find_repo_root(std::env::current_dir().ok()?)?;
    let workspace = repo_root.join("code-rs");

    // Probe likely build outputs in priority order.
    let mut candidates = vec![
        workspace.join("target/dev-fast/code"),
        workspace.join("target/debug/code"),
        workspace.join("target/release-prod/code"),
        workspace.join("target/release/code"),
        workspace.join("bin/code"),
    ];

    if let Some(found) = candidates.iter().find(|p| p.exists()).cloned() {
        return Some(found);
    }

    // Best-effort rebuild; swallow errors so caller can surface the original message.
    let status = std::process::Command::new("bash")
        .current_dir(&repo_root)
        .args(["-lc", "./build-fast.sh >/dev/null 2>&1"])
        .status()
        .ok();

    if status.map(|s| s.success()).unwrap_or(false) {
        candidates.retain(|p| p.exists());
        if let Some(found) = candidates.first().cloned() {
            return Some(found);
        }
    }

    None
}

fn find_repo_root(start: std::path::PathBuf) -> Option<std::path::PathBuf> {
    let mut dir = Some(start.as_path());
    while let Some(path) = dir {
        if path.join(".git").exists() {
            return Some(path.to_path_buf());
        }
        dir = path.parent();
    }
    None
}

/// Format a helpful error message when an agent command is not found.
/// Provides platform-specific guidance for resolving PATH issues.
pub(super) fn format_agent_not_found_error(agent_name: &str, command: &str) -> String {
    let mut msg = format!("Agent '{agent_name}' could not be found.");

    #[cfg(target_os = "windows")]
    {
        msg.push_str(&format!(
            "\n\nTroubleshooting steps:\n\
            1. Check if '{}' is installed and available in your PATH\n\
            2. Try using an absolute path in your config.toml:\n\
               [[agents]]\n\
               name = \"{}\"\n\
               command = \"C:\\\\Users\\\\YourUser\\\\AppData\\\\Roaming\\\\npm\\\\{}.cmd\"\n\
            3. Verify your PATH includes the directory containing '{}'\n\
            4. On Windows, ensure the file has a valid extension (.exe, .cmd, .bat, .com)\n\n\
            For more information, see: https://github.com/just-every/code/blob/main/code-rs/config.md",
            command, agent_name, command, command
        ));
    }

    #[cfg(not(target_os = "windows"))]
    {
        msg.push_str(&format!(
            "\n\nTroubleshooting steps:\n\
            1. Check if '{command}' is installed: which {command}\n\
            2. Verify '{command}' is in your PATH: echo $PATH\n\
            3. Try using an absolute path in your config.toml:\n\
               [[agents]]\n\
               name = \"{agent_name}\"\n\
               command = \"/absolute/path/to/{command}\"\n\n\
            For more information, see: https://github.com/just-every/code/blob/main/code-rs/config.md"
        ));
    }

    msg
}


pub(crate) fn should_use_current_exe_for_agent(
    family: &str,
    command_missing: bool,
    config: Option<&AgentConfig>,
) -> bool {
    if !matches!(family, "code" | "codex" | "cloud" | "coder") {
        return false;
    }

    // If the command is missing/empty, always use the current binary.
    if command_missing {
        return true;
    }

    if let Some(cfg) = config {
        let trimmed = cfg.command.trim();
        if trimmed.is_empty() {
            return true;
        }

        // If the configured command matches the canonical CLI for this spec, prefer self.
        if let Some(spec) = agent_model_spec(&cfg.name).or_else(|| agent_model_spec(trimmed))
            && trimmed.eq_ignore_ascii_case(spec.cli) {
                return true;
            }

        // Otherwise assume the user intentionally set a custom command; do not override.
        false
    } else {
        // No explicit config: built-in families should use the current binary.
        true
    }
}

pub(crate) fn resolve_program_path(use_current_exe: bool, command_for_spawn: &str) -> Result<std::path::PathBuf, String> {
    if use_current_exe {
        return current_code_binary_path();
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(p) = resolve_in_path(command_for_spawn) {
            return Ok(p);
        }
    }

    Ok(std::path::PathBuf::from(command_for_spawn))
}
