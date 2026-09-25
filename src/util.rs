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

/// Kind of per-user application directory (`wens-dev-principles` cli 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppDir {
    /// User-authored `config.toml`.
    Config,
    /// Application-authored `sshi.db` and TUI state files.
    State,
}

impl AppDir {
    fn xdg_var(self) -> &'static str {
        match self {
            AppDir::Config => "XDG_CONFIG_HOME",
            AppDir::State => "XDG_STATE_HOME",
        }
    }

    fn marker(self) -> &'static str {
        match self {
            AppDir::Config => ".migrated-config",
            AppDir::State => ".migrated-state",
        }
    }

    /// Location sshi used before B44 (`~/.config/sshi`, `~/.local/state/sshi`).
    /// `None` on Windows, whose paths never changed.
    pub fn legacy_dir(self, home: &Path) -> Option<PathBuf> {
        if cfg!(windows) {
            return None;
        }
        Some(match self {
            AppDir::Config => home.join(".config").join("sshi"),
            AppDir::State => home.join(".local").join("state").join("sshi"),
        })
    }
}

/// Resolve an sshi directory without touching the filesystem:
/// (1) the XDG variable if set to an absolute path, else (2) the platform
/// default — Linux `~/.config` / `~/.local/state`, macOS
/// `~/Library/Application Support`, Windows `%APPDATA%` / `%LOCALAPPDATA%`.
pub fn resolve_app_dir(kind: AppDir, home: &Path, xdg: Option<&std::ffi::OsStr>) -> PathBuf {
    if let Some(v) = xdg.map(Path::new).filter(|p| p.is_absolute()) {
        return v.join("sshi");
    }
    let base = if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else if cfg!(windows) {
        match kind {
            AppDir::Config => dirs::config_dir(),
            AppDir::State => dirs::data_local_dir(),
        }
        .unwrap_or_else(|| home.to_path_buf())
    } else {
        match kind {
            AppDir::Config => home.join(".config"),
            AppDir::State => home.join(".local").join("state"),
        }
    };
    base.join("sshi")
}

/// Step 3 of the cli 2 order: copy the `legacy` directory's files into
/// `target` once, then drop a per-kind marker so it never repeats. Files
/// already in `target` are never overwritten and `legacy` is never modified.
/// Each file is copied to a temporary name then renamed, so an interrupted
/// copy never leaves a truncated file. Returns whether a migration happened.
pub fn migrate_dir(kind: AppDir, legacy: &Path, target: &Path) -> std::io::Result<bool> {
    let marker = target.join(kind.marker());
    if legacy == target || !legacy.is_dir() || marker.exists() {
        return Ok(false);
    }
    copy_missing(legacy, target)?;
    std::fs::write(&marker, format!("migrated from {}\n", legacy.display()))?;
    Ok(true)
}

fn copy_missing(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let dest = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_missing(&entry.path(), &dest)?;
        } else if !dest.exists() {
            let tmp = to.join(format!(".sshi-migrate-{}", std::process::id()));
            std::fs::copy(entry.path(), &tmp)?;
            std::fs::rename(&tmp, &dest)?;
        }
    }
    Ok(())
}

/// The sshi directory of `kind`, migrating the legacy location forward on
/// first use (prints one line to stderr when it does).
pub fn app_dir(kind: AppDir) -> anyhow::Result<PathBuf> {
    use anyhow::Context;
    let home = dirs::home_dir().context("Cannot determine home directory")?;
    let target = resolve_app_dir(kind, &home, std::env::var_os(kind.xdg_var()).as_deref());
    if let Some(legacy) = kind.legacy_dir(&home) {
        match migrate_dir(kind, &legacy, &target) {
            Ok(true) => eprintln!(
                "sshi: migrated {} → {} (old copy left in place)",
                legacy.display(),
                target.display()
            ),
            Ok(false) => {}
            Err(e) => {
                // Keep working from the legacy location rather than failing.
                eprintln!("sshi: could not migrate {}: {e}", legacy.display());
                return Ok(legacy);
            }
        }
    }
    Ok(target)
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
    fn app_dir_prefers_absolute_xdg_then_platform_default() {
        let home = Path::new("/h");
        let xdg = std::ffi::OsStr::new("/x");
        assert_eq!(
            resolve_app_dir(AppDir::Config, home, Some(xdg)),
            PathBuf::from("/x/sshi")
        );
        assert_eq!(
            resolve_app_dir(AppDir::State, home, Some(xdg)),
            PathBuf::from("/x/sshi")
        );
        // A relative XDG value is invalid per the spec and ignored.
        let rel = std::ffi::OsStr::new("rel");
        let dflt = resolve_app_dir(AppDir::Config, home, None);
        assert_eq!(resolve_app_dir(AppDir::Config, home, Some(rel)), dflt);
        if cfg!(target_os = "macos") {
            assert_eq!(dflt, PathBuf::from("/h/Library/Application Support/sshi"));
            assert_eq!(
                resolve_app_dir(AppDir::State, home, None),
                PathBuf::from("/h/Library/Application Support/sshi")
            );
        } else if cfg!(unix) {
            assert_eq!(dflt, PathBuf::from("/h/.config/sshi"));
            assert_eq!(
                resolve_app_dir(AppDir::State, home, None),
                PathBuf::from("/h/.local/state/sshi")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn migrate_dir_copies_once_merges_and_keeps_legacy() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let target = home.join("new/sshi");
        let lc = AppDir::Config.legacy_dir(home).unwrap();
        let ls = AppDir::State.legacy_dir(home).unwrap();
        std::fs::create_dir_all(&lc).unwrap();
        std::fs::create_dir_all(&ls).unwrap();
        std::fs::write(lc.join("config.toml"), "cfg").unwrap();
        std::fs::write(ls.join("sshi.db"), "db").unwrap();

        // Config and state share one target (the macOS layout): both merge in.
        assert!(migrate_dir(AppDir::Config, &lc, &target).unwrap());
        assert!(migrate_dir(AppDir::State, &ls, &target).unwrap());
        assert_eq!(
            std::fs::read_to_string(target.join("config.toml")).unwrap(),
            "cfg"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("sshi.db")).unwrap(),
            "db"
        );
        assert!(lc.join("config.toml").exists() && ls.join("sshi.db").exists());

        // Second run is a no-op and never overwrites newer target files.
        std::fs::write(target.join("config.toml"), "edited").unwrap();
        assert!(!migrate_dir(AppDir::Config, &lc, &target).unwrap());
        assert_eq!(
            std::fs::read_to_string(target.join("config.toml")).unwrap(),
            "edited"
        );

        // Same legacy and target (Linux default) is a no-op.
        assert!(!migrate_dir(AppDir::Config, &lc, &lc).unwrap());
    }

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
