use std::path::Path;
use std::path::PathBuf;

use shlex::try_join;

pub(crate) fn escape_command(command: &[String]) -> String {
    try_join(command.iter().map(String::as_str)).unwrap_or_else(|_| command.join(" "))
}

pub(crate) fn strip_bash_lc_and_escape(command: &[String]) -> String {
    match command {
        // exactly three items
        [first, second, third]
            // first two must be "bash", "-lc"
            if is_bash_like(first) && second == "-lc" =>
        {
            third.clone()        // borrow `third`
        }
        _ => escape_command(command),
    }
}

fn is_bash_like(cmd: &str) -> bool {
    let trimmed = cmd.trim_matches('"').trim_matches('\'');
    let lowered = Path::new(trimmed)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(trimmed)
        .to_ascii_lowercase();
    matches!(
        lowered.as_str(),
        "bash"
            | "bash.exe"
            | "sh"
            | "sh.exe"
            | "dash"
            | "dash.exe"
            | "zsh"
            | "zsh.exe"
            | "ksh"
            | "ksh.exe"
            | "busybox"
    )
}

/// If `path` is absolute and inside $HOME, return the part *after* the home
/// directory; otherwise, return the path as-is. Note if `path` is the homedir,
/// this will return and empty path.
pub(crate) fn relativize_to_home<P>(path: P) -> Option<PathBuf>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();
    if !path.is_absolute() {
        // If the path is not absolute, we can’t do anything with it.
        return None;
    }

    if let Some(home_dir) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(rel) = path.strip_prefix(&home_dir) {
            return Some(rel.to_path_buf());
        }

    None
}
