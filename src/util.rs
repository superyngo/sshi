//! Small helpers shared across commands and the TUI.

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

/// CJK-aware truncation: if `s` fits within `max` display cells, return
/// it unchanged; otherwise stop on the last character that fits and
/// append `…`. Never splits a character, so it is safe on any UTF-8 input.
/// Shared by the CLI and the TUI.
pub fn truncate(s: &str, max: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if s.width() <= max {
        return s.to_string();
    }
    let mut w = 0;
    let mut out = String::new();
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_never_splits_multibyte_chars() {
        let s = "x系統負載正常，目前有二十五位使用者登入中並且磁碟空間充足可以繼續運行".repeat(2);
        for max in [1, 2, 50, 51, 72] {
            let out = truncate(&s, max);
            assert!(
                unicode_width::UnicodeWidthStr::width(out.as_str()) <= max,
                "max={max}"
            );
            assert!(out.ends_with('…'));
        }
    }

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
