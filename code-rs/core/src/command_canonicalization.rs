use std::path::Path;

use crate::bash;
use crate::util::is_shell_like_executable;

const CANONICAL_SHELL_SCRIPT_PREFIX: &str = "__code_shell_script__";
const CANONICAL_POWERSHELL_SCRIPT_PREFIX: &str = "__code_powershell_script__";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalApprovalCommandKind {
    /// A plain argv vector that can be compared directly (e.g. `["cargo", "test"]`).
    Argv,
    /// A shell wrapper command where we can't safely recover a tokenized argv.
    ShellScript,
    /// A PowerShell wrapper command where we canonicalize to the script text.
    PowerShellScript,
}

pub fn canonical_approval_command_kind(canonical: &[String]) -> CanonicalApprovalCommandKind {
    match canonical.first().map(String::as_str) {
        Some(CANONICAL_SHELL_SCRIPT_PREFIX) => CanonicalApprovalCommandKind::ShellScript,
        Some(CANONICAL_POWERSHELL_SCRIPT_PREFIX) => CanonicalApprovalCommandKind::PowerShellScript,
        _ => CanonicalApprovalCommandKind::Argv,
    }
}

/// Canonicalize command argv for approval-cache matching.
///
/// This keeps approval decisions stable across wrapper-path differences (for
/// example `/bin/bash -lc` vs `bash -lc`) and across shell wrapper tools while
/// preserving exact script text for complex scripts where we cannot safely
/// recover a tokenized command sequence.
pub fn canonicalize_command_for_approval(command: &[String]) -> Vec<String> {
    if let Some(commands) = parse_shell_lc_plain_commands(command)
        && let [single_command] = commands.as_slice()
    {
        return single_command.clone();
    }

    if let Some(script) = extract_shell_wrapper_script(command) {
        let shell_mode = command.get(1).cloned().unwrap_or_default();
        return vec![
            CANONICAL_SHELL_SCRIPT_PREFIX.to_string(),
            shell_mode,
            script,
        ];
    }

    if let Some(script) = extract_powershell_script(command) {
        return vec![CANONICAL_POWERSHELL_SCRIPT_PREFIX.to_string(), script];
    }

    command.to_vec()
}

pub(crate) fn normalize_command_for_persistence(command: &[String]) -> Vec<String> {
    let canonical = canonicalize_command_for_approval(command);
    match canonical_approval_command_kind(&canonical) {
        CanonicalApprovalCommandKind::Argv => canonical,
        CanonicalApprovalCommandKind::ShellScript => {
            let mode = canonical.get(1).cloned().unwrap_or_default();
            let script = canonical.get(2).cloned().unwrap_or_default();
            let shell = command
                .first()
                .and_then(|shell| file_name_only(shell))
                .unwrap_or_else(|| "bash".to_string());
            vec![shell, mode, script]
        }
        CanonicalApprovalCommandKind::PowerShellScript => {
            let script = canonical.get(1).cloned().unwrap_or_default();
            let shell = command
                .first()
                .and_then(|shell| file_name_only(shell))
                .unwrap_or_else(|| "pwsh".to_string());
            vec![shell, "-Command".to_string(), script]
        }
    }
}

fn parse_shell_lc_plain_commands(command: &[String]) -> Option<Vec<Vec<String>>> {
    let script = extract_shell_wrapper_script(command)?;
    let tree = bash::try_parse_bash(&script)?;
    bash::try_parse_word_only_commands_sequence(&tree, &script)
}

fn extract_shell_wrapper_script(command: &[String]) -> Option<String> {
    let [shell, flag, script] = command else {
        return None;
    };
    if !is_shell_like_executable(shell) || !(flag == "-lc" || flag == "-c") {
        return None;
    }

    Some(strip_rc_source_wrapper(script).unwrap_or_else(|| script.trim().to_string()))
}

fn strip_rc_source_wrapper(script: &str) -> Option<String> {
    let trimmed = script.trim();
    if !trimmed.starts_with("source ") {
        return None;
    }

    let start = trimmed.find("&& (")?;
    let inner_start = start + "&& (".len();
    let end = trimmed.rfind(')')?;
    if end <= inner_start {
        return None;
    }
    Some(trimmed[inner_start..end].trim().to_string())
}

fn extract_powershell_script(command: &[String]) -> Option<String> {
    let (exe, rest) = command.split_first()?;
    if !is_powershell_executable(exe) {
        return None;
    }
    if rest.is_empty() {
        return None;
    }

    let mut idx = 0;
    while idx < rest.len() {
        let arg = &rest[idx];
        let lower = arg.to_ascii_lowercase();
        match lower.as_str() {
            "-command" | "/command" | "-c" => {
                return rest.get(idx + 1).cloned().map(|s| s.trim().to_string());
            }
            _ if lower.starts_with("-command:") || lower.starts_with("/command:") => {
                let script = arg.split_once(':')?.1;
                return Some(script.trim().to_string());
            }
            // Benign, no-arg flags we tolerate.
            "-nologo" | "-noprofile" | "-noninteractive" | "-mta" | "-sta" => {
                idx += 1;
                continue;
            }
            // Unknown switch -> skip it conservatively.
            _ if lower.starts_with('-') => {
                idx += 1;
                continue;
            }
            _ => {
                return Some(join_arguments_as_script(&rest[idx..]));
            }
        }
    }

    None
}

fn is_powershell_executable(exe: &str) -> bool {
    let executable_name = Path::new(exe)
        .file_name()
        .and_then(|osstr| osstr.to_str())
        .unwrap_or(exe)
        .to_ascii_lowercase();

    matches!(
        executable_name.as_str(),
        "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
    )
}

fn join_arguments_as_script(args: &[String]) -> String {
    args.join(" ").trim().to_string()
}

fn file_name_only(path: &str) -> Option<String> {
    Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .map(std::string::ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn canonicalizes_word_only_shell_scripts_to_inner_command() {
        let command_a = vec![
            "/bin/bash".to_string(),
            "-lc".to_string(),
            "cargo test -p code-core".to_string(),
        ];
        let command_b = vec![
            "bash".to_string(),
            "-lc".to_string(),
            "cargo   test   -p code-core".to_string(),
        ];

        assert_eq!(
            canonicalize_command_for_approval(&command_a),
            vec![
                "cargo".to_string(),
                "test".to_string(),
                "-p".to_string(),
                "code-core".to_string(),
            ]
        );
        assert_eq!(
            canonicalize_command_for_approval(&command_a),
            canonicalize_command_for_approval(&command_b)
        );
    }

    #[test]
    fn canonicalizes_shell_scripts_wrapped_in_rc_source_to_inner_command() {
        let script = "source /tmp/.bashrc && (cargo test -p code-core)";
        let command = vec!["bash".to_string(), "-lc".to_string(), script.to_string()];

        assert_eq!(
            canonicalize_command_for_approval(&command),
            vec![
                "cargo".to_string(),
                "test".to_string(),
                "-p".to_string(),
                "code-core".to_string(),
            ]
        );
    }

    #[test]
    fn canonicalizes_heredoc_scripts_to_stable_script_key() {
        let script = "python3 <<'PY'\nprint('hello')\nPY";
        let command_a = vec![
            "/bin/zsh".to_string(),
            "-lc".to_string(),
            script.to_string(),
        ];
        let command_b = vec!["zsh".to_string(), "-lc".to_string(), script.to_string()];

        assert_eq!(
            canonicalize_command_for_approval(&command_a),
            vec![
                "__code_shell_script__".to_string(),
                "-lc".to_string(),
                script.to_string(),
            ]
        );
        assert_eq!(
            canonicalize_command_for_approval(&command_a),
            canonicalize_command_for_approval(&command_b)
        );
    }

    #[test]
    fn canonicalizes_powershell_wrappers_to_stable_script_key() {
        let script = "Write-Host hi";
        let command_a = vec![
            "powershell.exe".to_string(),
            "-NoProfile".to_string(),
            "-Command".to_string(),
            script.to_string(),
        ];
        let command_b = vec![
            "pwsh".to_string(),
            "-Command".to_string(),
            script.to_string(),
        ];

        assert_eq!(
            canonicalize_command_for_approval(&command_a),
            vec!["__code_powershell_script__".to_string(), script.to_string(),]
        );
        assert_eq!(
            canonicalize_command_for_approval(&command_a),
            canonicalize_command_for_approval(&command_b)
        );
    }

    #[test]
    fn preserves_non_shell_commands() {
        let command = vec!["cargo".to_string(), "fmt".to_string()];
        assert_eq!(canonicalize_command_for_approval(&command), command);
    }

    #[test]
    fn normalizes_shell_commands_for_persistence_without_rc_paths() {
        let script = "source /Users/me/.bashrc && (cargo test -p code-core)";
        let command = vec![
            "/bin/bash".to_string(),
            "-lc".to_string(),
            script.to_string(),
        ];
        assert_eq!(
            normalize_command_for_persistence(&command),
            vec![
                "cargo".to_string(),
                "test".to_string(),
                "-p".to_string(),
                "code-core".to_string(),
            ]
        );
    }

    #[test]
    fn normalizes_complex_shell_scripts_to_shell_wrapper_for_persistence() {
        let script = "source /Users/me/.bashrc && (python3 <<'PY'\nprint('hi')\nPY)";
        let command = vec![
            "/bin/bash".to_string(),
            "-lc".to_string(),
            script.to_string(),
        ];

        assert_eq!(
            normalize_command_for_persistence(&command),
            vec![
                "bash".to_string(),
                "-lc".to_string(),
                "python3 <<'PY'\nprint('hi')\nPY".to_string(),
            ]
        );
    }
}

