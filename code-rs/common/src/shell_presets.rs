//! Shell presets for the shell selection UI.
//! 
//! This module provides built-in shell definitions that can be extended
//! or overridden by user configuration.

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// Metadata describing a shell option.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellPreset {
    /// Unique identifier for the shell
    pub id: String,
    /// The shell command/binary name (e.g., "zsh", "bash")
    pub command: String,
    /// Display name shown in the UI
    pub display_name: String,
    /// Description of the shell
    pub description: String,
    /// Default arguments to pass to the shell
    #[serde(default)]
    pub default_args: Vec<String>,
    /// Preferred shell scripting style for model-generated commands.
    #[serde(default)]
    pub script_style: Option<String>,
    /// Whether this shell should be shown in the picker
    #[serde(default = "default_show_in_picker")]
    pub show_in_picker: bool,
}

fn default_show_in_picker() -> bool {
    true
}

/// Built-in shell presets. These cover the most common shells across platforms.
static BUILTIN_PRESETS: Lazy<Vec<ShellPreset>> = Lazy::new(|| {
    vec![
        ShellPreset {
            id: "zsh".to_string(),
            command: "zsh".to_string(),
            display_name: "Zsh".to_string(),
            description: "Z shell - modern shell with powerful features".to_string(),
            default_args: vec![],
            script_style: Some("zsh".to_string()),
            show_in_picker: true,
        },
        ShellPreset {
            id: "bash".to_string(),
            command: "bash".to_string(),
            display_name: "Bash".to_string(),
            description: "Bourne Again Shell - most common Unix shell".to_string(),
            default_args: vec![],
            script_style: Some("bash-zsh-compatible".to_string()),
            show_in_picker: true,
        },
        ShellPreset {
            id: "fish".to_string(),
            command: "fish".to_string(),
            display_name: "Fish".to_string(),
            description: "Friendly Interactive Shell - modern shell with great defaults".to_string(),
            default_args: vec![],
            script_style: None,
            show_in_picker: true,
        },
        ShellPreset {
            id: "nushell".to_string(),
            command: "nu".to_string(),
            display_name: "Nushell".to_string(),
            description: "Nu shell - modern shell with structured data".to_string(),
            default_args: vec![],
            script_style: None,
            show_in_picker: true,
        },
        ShellPreset {
            id: "elvish".to_string(),
            command: "elvish".to_string(),
            display_name: "Elvish".to_string(),
            description: "Elvish - expressive programming language and shell".to_string(),
            default_args: vec![],
            script_style: None,
            show_in_picker: true,
        },
        ShellPreset {
            id: "powershell".to_string(),
            command: "pwsh".to_string(),
            display_name: "PowerShell".to_string(),
            description: "PowerShell Core - cross-platform automation shell".to_string(),
            default_args: vec![],
            script_style: None,
            show_in_picker: true,
        },
        ShellPreset {
            id: "dash".to_string(),
            command: "dash".to_string(),
            display_name: "Dash".to_string(),
            description: "Debian Almquist Shell - lightweight POSIX shell".to_string(),
            default_args: vec![],
            script_style: Some("posix-sh".to_string()),
            show_in_picker: true,
        },
        ShellPreset {
            id: "sh".to_string(),
            command: "sh".to_string(),
            display_name: "Sh".to_string(),
            description: "Bourne Shell - original Unix shell".to_string(),
            default_args: vec![],
            script_style: Some("posix-sh".to_string()),
            show_in_picker: true,
        },
        ShellPreset {
            id: "xonsh".to_string(),
            command: "xonsh".to_string(),
            display_name: "Xonsh".to_string(),
            description: "Xonsh - Python-powered shell".to_string(),
            default_args: vec![],
            script_style: None,
            show_in_picker: true,
        },
        ShellPreset {
            id: "oil".to_string(),
            command: "osh".to_string(),
            display_name: "Oil Shell".to_string(),
            description: "Oil shell - our upgrade path from bash".to_string(),
            default_args: vec![],
            script_style: None,
            show_in_picker: true,
        },
    ]
});

/// Returns the built-in shell presets.
pub fn builtin_shell_presets() -> Vec<ShellPreset> {
    BUILTIN_PRESETS.clone()
}

/// Merges user-defined shell presets with built-in ones.
/// User presets with the same ID override built-in presets.
/// User presets with new IDs are appended.
pub fn merge_shell_presets(user_presets: Vec<ShellPreset>) -> Vec<ShellPreset> {
    let mut result = BUILTIN_PRESETS.clone();
    
    for user_preset in user_presets {
        if let Some(idx) = result.iter().position(|p| p.id == user_preset.id) {
            // Override existing preset
            result[idx] = user_preset;
        } else {
            // Add new preset
            result.push(user_preset);
        }
    }
    
    result
}

/// Filter presets to only those that should be shown in the picker.
pub fn picker_shell_presets(presets: &[ShellPreset]) -> Vec<ShellPreset> {
    presets
        .iter()
        .filter(|p| p.show_in_picker)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_presets_not_empty() {
        let presets = builtin_shell_presets();
        assert!(!presets.is_empty());
    }

    #[test]
    fn test_merge_override() {
        let user = vec![ShellPreset {
            id: "zsh".to_string(),
            command: "zsh".to_string(),
            display_name: "My Custom Zsh".to_string(),
            description: "Custom description".to_string(),
            default_args: vec!["-i".to_string()],
            script_style: Some("zsh".to_string()),
            show_in_picker: true,
        }];
        
        let merged = merge_shell_presets(user);
        let zsh = merged.iter().find(|p| p.id == "zsh").unwrap();
        assert_eq!(zsh.display_name, "My Custom Zsh");
    }

    #[test]
    fn test_merge_add_new() {
        let user = vec![ShellPreset {
            id: "custom".to_string(),
            command: "my-shell".to_string(),
            display_name: "My Shell".to_string(),
            description: "A custom shell".to_string(),
            default_args: vec![],
            script_style: None,
            show_in_picker: true,
        }];
        
        let merged = merge_shell_presets(user);
        assert!(merged.iter().any(|p| p.id == "custom"));
    }
}
