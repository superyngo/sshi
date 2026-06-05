//! Small filesystem-path helpers shared across commands.

use std::path::{Path, PathBuf};

/// Expand a leading `~`, `~/…`, or `~\…` in `p` to the user's home directory.
///
/// Windows shells (cmd / PowerShell) do not expand `~`, and TUI text fields
/// have no shell at all — so sshi must expand it itself for any *local* path it
/// reads or writes (`--out` reports, `exec` scripts, `cp` sources, config
/// paths). Returns the path unchanged when there is no `~` prefix or the home
/// directory cannot be resolved.
pub fn expand_tilde(p: &Path) -> PathBuf {
    let s = match p.to_str() {
        Some(s) => s,
        None => return p.to_path_buf(),
    };
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| p.to_path_buf());
    }
    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    p.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_only_expands_to_home() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde(Path::new("~")), home);
    }

    #[test]
    fn tilde_slash_unix() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            expand_tilde(Path::new("~/test.html")),
            home.join("test.html")
        );
    }

    #[test]
    fn tilde_backslash_windows() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(
            expand_tilde(Path::new("~\\test.html")),
            home.join("test.html")
        );
    }

    #[test]
    fn no_tilde_unchanged() {
        assert_eq!(
            expand_tilde(Path::new("test.html")),
            PathBuf::from("test.html")
        );
        assert_eq!(
            expand_tilde(Path::new("/abs/path")),
            PathBuf::from("/abs/path")
        );
    }

    #[test]
    fn embedded_tilde_unchanged() {
        // Only a *leading* ~ is expanded.
        assert_eq!(expand_tilde(Path::new("/a/~/b")), PathBuf::from("/a/~/b"));
    }
}
