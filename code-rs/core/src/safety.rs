use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

use code_apply_patch::ApplyPatchAction;
use code_apply_patch::ApplyPatchFileChange;

use crate::codex::ApprovedCommandPattern;
use crate::command_safety::context::CommandSafetyContext;
use crate::config_types::CommandSafetyOsProfileConfig;
use crate::config_types::CommandSafetyRuleConfig;
use crate::config_types::CommandSafetyRuleset;
use crate::config_types::ShellConfig;
use crate::config_types::ShellScriptStyle;
use crate::config_types::ShellStyleProfileConfig;
use crate::exec::SandboxType;
use crate::is_dangerous_command::command_might_be_dangerous_with_context_and_rules;
use crate::is_safe_command::is_known_safe_command_with_context_and_rules;
use crate::protocol::AskForApproval;
use crate::protocol::SandboxPolicy;
use crate::shell::Shell;

#[derive(Debug, PartialEq)]
pub enum SafetyCheck {
    AutoApprove {
        sandbox_type: SandboxType,
        user_explicitly_approved: bool,
    },
    AskUser,
    Reject { reason: String },
}

fn default_dangerous_command_detection_for_style(style: Option<ShellScriptStyle>) -> bool {
    matches!(
        style,
        Some(
            ShellScriptStyle::PosixSh
                | ShellScriptStyle::BashZshCompatible
                | ShellScriptStyle::Zsh
        )
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedCommandSafetyProfile {
    pub dangerous_command_detection_enabled: bool,
    pub safe_rules: CommandSafetyRuleset,
    pub dangerous_rules: CommandSafetyRuleset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandSafetyEvaluationConfig {
    pub context: CommandSafetyContext,
    pub safe_rules: CommandSafetyRuleset,
    pub dangerous_rules: CommandSafetyRuleset,
    pub dangerous_command_detection_enabled: bool,
}

fn apply_command_safety_rule_config(
    source: &CommandSafetyRuleConfig,
    target: &mut ResolvedCommandSafetyProfile,
) {
    if let Some(enabled) = source.dangerous_command_detection {
        target.dangerous_command_detection_enabled = enabled;
    }
    if let Some(rules) = source.safe_rules {
        target.safe_rules = rules;
    }
    if let Some(rules) = source.dangerous_rules {
        target.dangerous_rules = rules;
    }
}

#[cfg(target_os = "windows")]
fn current_os_command_safety_rule_config(os: &CommandSafetyOsProfileConfig) -> &CommandSafetyRuleConfig {
    &os.windows
}

#[cfg(target_os = "macos")]
fn current_os_command_safety_rule_config(os: &CommandSafetyOsProfileConfig) -> &CommandSafetyRuleConfig {
    &os.macos
}

#[cfg(target_os = "linux")]
fn current_os_command_safety_rule_config(os: &CommandSafetyOsProfileConfig) -> &CommandSafetyRuleConfig {
    &os.linux
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
fn current_os_command_safety_rule_config(os: &CommandSafetyOsProfileConfig) -> &CommandSafetyRuleConfig {
    &os.other
}

/// Resolve command-safety profile for the active shell/style and current OS.
///
/// Precedence (later items override earlier items):
/// 1) style default
/// 2) legacy `shell.dangerous_command_detection`
/// 3) legacy `shell_style_profiles.<style>.dangerous_command_detection`
/// 4) `shell.command_safety`
/// 5) `shell_style_profiles.<style>.command_safety`
/// 6) `shell.command_safety.os.<current-os>`
/// 7) `shell_style_profiles.<style>.command_safety.os.<current-os>`
pub fn resolve_command_safety_profile(
    shell: &Shell,
    shell_config: Option<&ShellConfig>,
    shell_style_profiles: &HashMap<ShellScriptStyle, ShellStyleProfileConfig>,
) -> ResolvedCommandSafetyProfile {
    let style = shell.script_style();
    let mut resolved = ResolvedCommandSafetyProfile {
        dangerous_command_detection_enabled: default_dangerous_command_detection_for_style(style),
        safe_rules: CommandSafetyRuleset::Auto,
        dangerous_rules: CommandSafetyRuleset::Auto,
    };

    if let Some(shell_legacy_override) = shell_config.and_then(|cfg| cfg.dangerous_command_detection)
    {
        resolved.dangerous_command_detection_enabled = shell_legacy_override;
    }

    let style_profile = style.and_then(|active_style| shell_style_profiles.get(&active_style));
    if let Some(profile_legacy_override) =
        style_profile.and_then(|profile| profile.dangerous_command_detection)
    {
        resolved.dangerous_command_detection_enabled = profile_legacy_override;
    }

    if let Some(shell_cfg) = shell_config {
        apply_command_safety_rule_config(&shell_cfg.command_safety.rules, &mut resolved);
    }
    if let Some(profile) = style_profile {
        apply_command_safety_rule_config(&profile.command_safety.rules, &mut resolved);
    }

    if let Some(shell_cfg) = shell_config {
        apply_command_safety_rule_config(
            current_os_command_safety_rule_config(&shell_cfg.command_safety.os),
            &mut resolved,
        );
    }
    if let Some(profile) = style_profile {
        apply_command_safety_rule_config(
            current_os_command_safety_rule_config(&profile.command_safety.os),
            &mut resolved,
        );
    }

    resolved
}

pub fn assess_patch_safety(
    action: &ApplyPatchAction,
    policy: AskForApproval,
    sandbox_policy: &SandboxPolicy,
    cwd: &Path,
) -> SafetyCheck {
    if action.is_empty() {
        return SafetyCheck::Reject {
            reason: "empty patch".to_string(),
        };
    }

    // In Read Only mode, we need explicit user approval before writing.
    if matches!(sandbox_policy, SandboxPolicy::ReadOnly) {
        return match policy {
            AskForApproval::Never => SafetyCheck::Reject {
                reason: "write operations require approval but approval policy is set to never".to_string(),
            },
            _ => SafetyCheck::AskUser,
        };
    }

    match policy {
        AskForApproval::OnFailure | AskForApproval::Never | AskForApproval::OnRequest => {
            // Continue to see if this can be auto-approved.
        }
        // TODO(ragona): I'm not sure this is actually correct? I believe in this case
        // we want to continue to the writable paths check before asking the user.
        AskForApproval::UnlessTrusted => {
            return SafetyCheck::AskUser;
        }
    }

    // Even though the patch *appears* to be constrained to writable paths, it
    // is possible that paths in the patch are hard links to files outside the
    // writable roots, so we should still run `apply_patch` in a sandbox in that
    // case.
    if is_write_patch_constrained_to_writable_paths(action, sandbox_policy, cwd)
        || policy == AskForApproval::OnFailure
    {
        // Only auto‑approve when we can actually enforce a sandbox. Otherwise
        // fall back to asking the user because the patch may touch arbitrary
        // paths outside the project.
        match get_platform_sandbox() {
            Some(sandbox_type) => SafetyCheck::AutoApprove {
                sandbox_type,
                user_explicitly_approved: false,
            },
            None if sandbox_policy == &SandboxPolicy::DangerFullAccess => {
                // If the user has explicitly requested DangerFullAccess, then
                // we can auto-approve even without a sandbox.
                SafetyCheck::AutoApprove {
                    sandbox_type: SandboxType::None,
                    user_explicitly_approved: false,
                }
            }
            None => SafetyCheck::AskUser,
        }
    } else if policy == AskForApproval::Never {
        SafetyCheck::Reject {
            reason: "writing outside of the project; rejected by user approval settings"
                .to_string(),
        }
    } else {
        SafetyCheck::AskUser
    }
}

/// For a command to be run _without_ a sandbox, one of the following must be
/// true:
///
/// - the user has explicitly approved the command
/// - the command is on the "known safe" list
/// - `DangerFullAccess` was specified and `UnlessTrusted` was not
pub fn assess_command_safety(
    command: &[String],
    safety_config: CommandSafetyEvaluationConfig,
    approval_policy: AskForApproval,
    sandbox_policy: &SandboxPolicy,
    approved: &HashSet<ApprovedCommandPattern>,
    with_escalated_permissions: bool,
) -> SafetyCheck {
    // A command is "trusted" because either:
    // - it belongs to a set of commands we consider "safe" by default, or
    // - the user has explicitly approved the command for this session
    //
    // Currently, whether a command is "trusted" is a simple boolean, but we
    // should include more metadata on this command test to indicate whether it
    // should be run inside a sandbox or not. (This could be something the user
    // defines as part of `execpolicy`.)
    //
    // For example, when `is_known_safe_command(command)` returns `true`, it
    // would probably be fine to run the command in a sandbox, but when
    // `approved.contains(command)` is `true`, the user may have approved it for
    // the session _because_ they know it needs to run outside a sandbox.
    let user_explicitly_approved = approved.iter().any(|pattern| pattern.matches(command));
    if is_known_safe_command_with_context_and_rules(
        command,
        safety_config.context,
        safety_config.safe_rules,
    )
        || user_explicitly_approved
    {
        return SafetyCheck::AutoApprove {
            sandbox_type: SandboxType::None,
            user_explicitly_approved,
        };
    }

    if safety_config.dangerous_command_detection_enabled
        && command_might_be_dangerous_with_context_and_rules(
            command,
            safety_config.context,
            safety_config.dangerous_rules,
        )
    {
        return if matches!(approval_policy, AskForApproval::Never) {
            SafetyCheck::Reject {
                reason: "auto-rejected because command is considered dangerous".to_string(),
            }
        } else {
            SafetyCheck::AskUser
        };
    }

    assess_safety_for_untrusted_command(approval_policy, sandbox_policy, with_escalated_permissions)
}

pub(crate) fn assess_safety_for_untrusted_command(
    approval_policy: AskForApproval,
    sandbox_policy: &SandboxPolicy,
    with_escalated_permissions: bool,
) -> SafetyCheck {
    use AskForApproval::*;
    use SandboxPolicy::*;

    match (approval_policy, sandbox_policy) {
        (UnlessTrusted, _) => {
            // Even though the user may have opted into DangerFullAccess,
            // they also requested that we ask for approval for untrusted
            // commands.
            SafetyCheck::AskUser
        }
        (OnFailure, DangerFullAccess)
        | (Never, DangerFullAccess)
        | (OnRequest, DangerFullAccess) => SafetyCheck::AutoApprove {
            sandbox_type: SandboxType::None,
            user_explicitly_approved: false,
        },
        (OnRequest, ReadOnly) | (OnRequest, WorkspaceWrite { .. }) => {
            if with_escalated_permissions {
                SafetyCheck::AskUser
            } else {
                match get_platform_sandbox() {
                    Some(sandbox_type) => SafetyCheck::AutoApprove {
                        sandbox_type,
                        user_explicitly_approved: false,
                    },
                    // Fall back to asking since the command is untrusted and
                    // we do not have a sandbox available
                    None => SafetyCheck::AskUser,
                }
            }
        }
        (Never, ReadOnly)
        | (Never, WorkspaceWrite { .. })
        | (OnFailure, ReadOnly)
        | (OnFailure, WorkspaceWrite { .. }) => {
            match get_platform_sandbox() {
                Some(sandbox_type) => SafetyCheck::AutoApprove {
                    sandbox_type,
                    user_explicitly_approved: false,
                },
                None => {
                    if matches!(approval_policy, OnFailure) {
                        // Since the command is not trusted, even though the
                        // user has requested to only ask for approval on
                        // failure, we will ask the user because no sandbox is
                        // available.
                        SafetyCheck::AskUser
                    } else {
                        // We are in non-interactive mode and lack approval, so
                        // all we can do is reject the command.
                        SafetyCheck::Reject {
                            reason: "auto-rejected because command is not on trusted list"
                                .to_string(),
                        }
                    }
                }
            }
        }
    }
}

pub fn get_platform_sandbox() -> Option<SandboxType> {
    if cfg!(target_os = "macos") {
        Some(SandboxType::MacosSeatbelt)
    } else if cfg!(target_os = "linux") {
        Some(SandboxType::LinuxSeccomp)
    } else {
        None
    }
}

fn is_write_patch_constrained_to_writable_paths(
    action: &ApplyPatchAction,
    sandbox_policy: &SandboxPolicy,
    cwd: &Path,
) -> bool {
    // Early‑exit if there are no declared writable roots.
    let writable_roots = match sandbox_policy {
        SandboxPolicy::ReadOnly => {
            return false;
        }
        SandboxPolicy::DangerFullAccess => {
            return true;
        }
        SandboxPolicy::WorkspaceWrite { .. } => sandbox_policy.get_writable_roots_with_cwd(cwd),
    };

    // Normalize a path by removing `.` and resolving `..` without touching the
    // filesystem (works even if the file does not exist).
    fn normalize(path: &Path) -> Option<PathBuf> {
        let mut out = PathBuf::new();
        for comp in path.components() {
            match comp {
                Component::ParentDir => {
                    out.pop();
                }
                Component::CurDir => { /* skip */ }
                other => out.push(other.as_os_str()),
            }
        }
        Some(out)
    }

    // Determine whether `path` is inside **any** writable root. Both `path`
    // and roots are converted to absolute, normalized forms before the
    // prefix check.
    let is_path_writable = |p: &PathBuf| {
        let abs = if p.is_absolute() {
            p.clone()
        } else {
            cwd.join(p)
        };
        let abs = match normalize(&abs) {
            Some(v) => v,
            None => return false,
        };

        writable_roots
            .iter()
            .any(|writable_root| writable_root.is_path_writable(&abs))
    };

    for (path, change) in action.changes() {
        match change {
            ApplyPatchFileChange::Add { .. } | ApplyPatchFileChange::Delete { .. } => {
                if !is_path_writable(path) {
                    return false;
                }
            }
            ApplyPatchFileChange::Update { move_path, .. } => {
                if !is_path_writable(path) {
                    return false;
                }
                if let Some(dest) = move_path
                    && !is_path_writable(dest) {
                        return false;
                    }
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn set_current_os_override(
        os_cfg: &mut crate::config_types::CommandSafetyOsProfileConfig,
        rule: crate::config_types::CommandSafetyRuleConfig,
    ) {
        #[cfg(target_os = "windows")]
        {
            os_cfg.windows = rule;
        }
        #[cfg(target_os = "macos")]
        {
            os_cfg.macos = rule;
        }
        #[cfg(target_os = "linux")]
        {
            os_cfg.linux = rule;
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            os_cfg.other = rule;
        }
    }

    #[test]
    fn test_writable_roots_constraint() {
        // Use a temporary directory as our workspace to avoid touching
        // the real current working directory.
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_path_buf();
        let parent = cwd.parent().unwrap().to_path_buf();

        // Helper to build a single‑entry patch that adds a file at `p`.
        let make_add_change = |p: PathBuf| ApplyPatchAction::new_add_for_test(&p, "".to_string());

        let add_inside = make_add_change(cwd.join("inner.txt"));
        let add_outside = make_add_change(parent.join("outside.txt"));

        // Policy limited to the workspace only; exclude system temp roots so
        // only `cwd` is writable by default.
        let policy_workspace_only = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
            allow_git_writes: true,
        };

        assert!(is_write_patch_constrained_to_writable_paths(
            &add_inside,
            &policy_workspace_only,
            &cwd,
        ));

        assert!(!is_write_patch_constrained_to_writable_paths(
            &add_outside,
            &policy_workspace_only,
            &cwd,
        ));

        // With the parent dir explicitly added as a writable root, the
        // outside write should be permitted.
        let policy_with_parent = SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![parent],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
            allow_git_writes: true,
        };
        assert!(is_write_patch_constrained_to_writable_paths(
            &add_outside,
            &policy_with_parent,
            &cwd,
        ));
    }

    #[test]
    fn test_read_only_patch_requests_approval() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_path_buf();
        let action = ApplyPatchAction::new_add_for_test(&cwd.join("file.txt"), "".to_string());

        let result = assess_patch_safety(
            &action,
            AskForApproval::OnRequest,
            &SandboxPolicy::ReadOnly,
            &cwd,
        );

        assert_eq!(result, SafetyCheck::AskUser);
    }

    #[test]
    fn test_read_only_patch_rejects_when_policy_never() {
        let tmp = TempDir::new().unwrap();
        let cwd = tmp.path().to_path_buf();
        let action = ApplyPatchAction::new_add_for_test(&cwd.join("file.txt"), "".to_string());

        let result = assess_patch_safety(
            &action,
            AskForApproval::Never,
            &SandboxPolicy::ReadOnly,
            &cwd,
        );

        match result {
            SafetyCheck::Reject { reason } => {
                assert_eq!(
                    reason,
                    "write operations require approval but approval policy is set to never"
                );
            }
            other => {
                panic!("expected rejection, got {other:?}");
            }
        }
    }

    #[test]
    fn test_request_escalated_privileges() {
        // Should not be a trusted command
        let command = vec!["git commit".to_string()];
        let approval_policy = AskForApproval::OnRequest;
        let sandbox_policy = SandboxPolicy::ReadOnly;
        let approved: HashSet<ApprovedCommandPattern> = HashSet::new();
        let request_escalated_privileges = true;
        let command_safety_context = CommandSafetyContext::current().with_command_shell(&command);
        let safety_config = CommandSafetyEvaluationConfig {
            context: command_safety_context,
            safe_rules: CommandSafetyRuleset::Auto,
            dangerous_rules: CommandSafetyRuleset::Auto,
            dangerous_command_detection_enabled: true,
        };

        let safety_check = assess_command_safety(
            &command,
            safety_config,
            approval_policy,
            &sandbox_policy,
            &approved,
            request_escalated_privileges,
        );

        assert_eq!(safety_check, SafetyCheck::AskUser);
    }

    #[test]
    fn test_request_escalated_privileges_no_sandbox_fallback() {
        let command = vec!["git".to_string(), "commit".to_string()];
        let approval_policy = AskForApproval::OnRequest;
        let sandbox_policy = SandboxPolicy::ReadOnly;
        let approved: HashSet<ApprovedCommandPattern> = HashSet::new();
        let request_escalated_privileges = false;
        let command_safety_context = CommandSafetyContext::current().with_command_shell(&command);
        let safety_config = CommandSafetyEvaluationConfig {
            context: command_safety_context,
            safe_rules: CommandSafetyRuleset::Auto,
            dangerous_rules: CommandSafetyRuleset::Auto,
            dangerous_command_detection_enabled: true,
        };

        let safety_check = assess_command_safety(
            &command,
            safety_config,
            approval_policy,
            &sandbox_policy,
            &approved,
            request_escalated_privileges,
        );

        let expected = match get_platform_sandbox() {
            Some(sandbox_type) => SafetyCheck::AutoApprove {
                sandbox_type,
                user_explicitly_approved: false,
            },
            None => SafetyCheck::AskUser,
        };
        assert_eq!(safety_check, expected);
    }

    #[test]
    fn dangerous_command_detection_toggle_controls_reject_path() {
        let command = vec!["git".to_string(), "reset".to_string(), "--hard".to_string()];
        let approved: HashSet<ApprovedCommandPattern> = HashSet::new();
        let command_safety_context = CommandSafetyContext::current().with_command_shell(&command);
        let auto_rules = CommandSafetyRuleset::Auto;

        let with_detection = assess_command_safety(
            &command,
            CommandSafetyEvaluationConfig {
                context: command_safety_context,
                safe_rules: auto_rules,
                dangerous_rules: auto_rules,
                dangerous_command_detection_enabled: true,
            },
            AskForApproval::Never,
            &SandboxPolicy::DangerFullAccess,
            &approved,
            false,
        );
        assert_eq!(
            with_detection,
            SafetyCheck::Reject {
                reason: "auto-rejected because command is considered dangerous".to_string(),
            }
        );

        let without_detection = assess_command_safety(
            &command,
            CommandSafetyEvaluationConfig {
                context: command_safety_context,
                safe_rules: auto_rules,
                dangerous_rules: auto_rules,
                dangerous_command_detection_enabled: false,
            },
            AskForApproval::Never,
            &SandboxPolicy::DangerFullAccess,
            &approved,
            false,
        );
        assert_eq!(
            without_detection,
            SafetyCheck::AutoApprove {
                sandbox_type: SandboxType::None,
                user_explicitly_approved: false,
            }
        );
    }

    #[test]
    fn dangerous_command_detection_resolution_respects_precedence() {
        use crate::shell::PowerShellConfig;

        let pwsh = Shell::PowerShell(PowerShellConfig {
            exe: "pwsh".to_string(),
            bash_exe_fallback: None,
        });
        let shell_cfg = ShellConfig {
            path: "pwsh".to_string(),
            args: vec!["-NoProfile".to_string()],
            script_style: None,
            command_safety: crate::config_types::CommandSafetyProfileConfig::default(),
            dangerous_command_detection: None,
        };

        assert!(
            !resolve_command_safety_profile(&pwsh, None, &HashMap::new())
                .dangerous_command_detection_enabled
        );

        let shell_enabled = ShellConfig {
            dangerous_command_detection: Some(true),
            ..shell_cfg
        };
        assert!(
            resolve_command_safety_profile(&pwsh, Some(&shell_enabled), &HashMap::new())
                .dangerous_command_detection_enabled
        );

        let zsh = Shell::Generic(crate::shell::GenericShell {
            command: vec!["zsh".to_string()],
            script_style: Some(ShellScriptStyle::Zsh),
        });
        let mut profiles: HashMap<ShellScriptStyle, ShellStyleProfileConfig> = HashMap::new();
        profiles.insert(
            ShellScriptStyle::Zsh,
            ShellStyleProfileConfig {
                dangerous_command_detection: Some(false),
                ..Default::default()
            },
        );

        assert!(
            !resolve_command_safety_profile(&zsh, Some(&shell_enabled), &profiles)
                .dangerous_command_detection_enabled
        );
    }

    #[test]
    fn command_safety_profile_resolution_supports_shell_profile_and_os_matrix() {
        let shell = Shell::Generic(crate::shell::GenericShell {
            command: vec!["zsh".to_string()],
            script_style: Some(ShellScriptStyle::Zsh),
        });

        let mut shell_cfg = ShellConfig {
            path: "/bin/zsh".to_string(),
            args: vec!["-lc".to_string()],
            script_style: Some(ShellScriptStyle::Zsh),
            command_safety: crate::config_types::CommandSafetyProfileConfig::default(),
            dangerous_command_detection: None,
        };

        // 1) shell
        shell_cfg.command_safety.rules.safe_rules = Some(CommandSafetyRuleset::Posix);
        shell_cfg.command_safety.rules.dangerous_rules = Some(CommandSafetyRuleset::Windows);
        shell_cfg.command_safety.rules.dangerous_command_detection = Some(false);

        let mut profile = ShellStyleProfileConfig::default();
        let mut profiles = HashMap::new();
        profiles.insert(ShellScriptStyle::Zsh, profile.clone());

        let shell_only = resolve_command_safety_profile(&shell, Some(&shell_cfg), &profiles);
        assert!(!shell_only.dangerous_command_detection_enabled);
        assert_eq!(shell_only.safe_rules, CommandSafetyRuleset::Posix);
        assert_eq!(shell_only.dangerous_rules, CommandSafetyRuleset::Windows);

        // 2) shell + profile
        profile.command_safety.rules.safe_rules = Some(CommandSafetyRuleset::Windows);
        profile.command_safety.rules.dangerous_rules = Some(CommandSafetyRuleset::Posix);
        profile.command_safety.rules.dangerous_command_detection = Some(true);
        profiles.insert(ShellScriptStyle::Zsh, profile.clone());

        let shell_and_profile = resolve_command_safety_profile(&shell, Some(&shell_cfg), &profiles);
        assert!(shell_and_profile.dangerous_command_detection_enabled);
        assert_eq!(shell_and_profile.safe_rules, CommandSafetyRuleset::Windows);
        assert_eq!(shell_and_profile.dangerous_rules, CommandSafetyRuleset::Posix);

        // 3) shell + os
        profile.command_safety.rules = crate::config_types::CommandSafetyRuleConfig::default();
        profiles.insert(ShellScriptStyle::Zsh, profile.clone());

        set_current_os_override(
            &mut shell_cfg.command_safety.os,
            crate::config_types::CommandSafetyRuleConfig {
                dangerous_command_detection: Some(false),
                safe_rules: Some(CommandSafetyRuleset::Auto),
                dangerous_rules: Some(CommandSafetyRuleset::Posix),
            },
        );
        let shell_and_os = resolve_command_safety_profile(&shell, Some(&shell_cfg), &profiles);
        assert!(!shell_and_os.dangerous_command_detection_enabled);
        assert_eq!(shell_and_os.safe_rules, CommandSafetyRuleset::Auto);
        assert_eq!(shell_and_os.dangerous_rules, CommandSafetyRuleset::Posix);

        // 4) shell + (profile + os)
        set_current_os_override(
            &mut profile.command_safety.os,
            crate::config_types::CommandSafetyRuleConfig {
                dangerous_command_detection: Some(true),
                safe_rules: Some(CommandSafetyRuleset::Windows),
                dangerous_rules: Some(CommandSafetyRuleset::Windows),
            },
        );
        profiles.insert(ShellScriptStyle::Zsh, profile);

        let shell_profile_and_os =
            resolve_command_safety_profile(&shell, Some(&shell_cfg), &profiles);
        assert!(shell_profile_and_os.dangerous_command_detection_enabled);
        assert_eq!(shell_profile_and_os.safe_rules, CommandSafetyRuleset::Windows);
        assert_eq!(shell_profile_and_os.dangerous_rules, CommandSafetyRuleset::Windows);
    }
}
