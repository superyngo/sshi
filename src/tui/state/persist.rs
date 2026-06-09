//! Persisted TUI state schema + load/save (per docs/tui_reconstruct_plan.md
//! §16, AD-8, AD-16).
//!
//! - Path: `{resolved_state_dir}/tui_state-{config_hash}.toml`.
//! - `config_hash` = first 8 hex chars of blake3 over the resolved + (where
//!   possible) canonicalised config path string.
//! - Atomic write via `tempfile::NamedTempFile::persist()` (cross-platform safe).
//! - Missing / unreadable / parse-failed files start with defaults — the TUI
//!   never crashes on persistence read.
//!
//! Filter validation rules (§16.2): unknown active_tab → Config; groups /
//! hosts not present in current config are silently dropped; if Groups
//! mode ends up empty after filtering, mode falls back to All.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use crate::config::schema::{AppConfig, ShellType};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiPersistedState {
    pub tui_state: TuiSection,
    pub target_filter: TargetFilterState,
    pub operate: OperateState,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiSection {
    pub active_tab: ActiveTab,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveTab {
    Config,
    Operate,
    #[default]
    #[serde(alias = "Checkout")]
    View,
}

impl ActiveTab {
    pub fn from_tab_id(t: crate::tui::tabs::TabId) -> Self {
        match t {
            crate::tui::tabs::TabId::Config => ActiveTab::Config,
            crate::tui::tabs::TabId::Operate => ActiveTab::Operate,
            crate::tui::tabs::TabId::View => ActiveTab::View,
        }
    }

    pub fn to_tab_id(self) -> crate::tui::tabs::TabId {
        match self {
            ActiveTab::Config => crate::tui::tabs::TabId::Config,
            ActiveTab::Operate => crate::tui::tabs::TabId::Operate,
            ActiveTab::View => crate::tui::tabs::TabId::View,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TargetFilterState {
    pub mode: TargetFilterMode,
    pub groups: Vec<String>,
    pub hosts: Vec<String>,
    pub skip: Vec<String>,
    pub shell: ShellMode,
    pub serial: bool,
    pub timeout: u64,
}

impl Default for TargetFilterState {
    fn default() -> Self {
        Self {
            mode: TargetFilterMode::default(),
            groups: Vec::new(),
            hosts: Vec::new(),
            skip: Vec::new(),
            shell: ShellMode::default(),
            serial: false,
            timeout: 30,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TargetFilterMode {
    #[default]
    All,
    Groups,
    Hosts,
    Shell,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellMode {
    #[default]
    Sh,
    PowerShell,
    Cmd,
}

impl ShellMode {
    pub fn to_shell_type(self) -> ShellType {
        match self {
            ShellMode::Sh => ShellType::Sh,
            ShellMode::PowerShell => ShellType::PowerShell,
            ShellMode::Cmd => ShellType::Cmd,
        }
    }

    #[allow(dead_code)]
    pub fn from_shell_type(s: ShellType) -> Self {
        match s {
            ShellType::Sh => ShellMode::Sh,
            ShellType::PowerShell => ShellMode::PowerShell,
            ShellType::Cmd => ShellMode::Cmd,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OperateState {
    pub operation: OperationKind,
    /// Whether to use sudo when running remote commands (Run/Exec operations).
    pub run_sudo: bool,
    pub exec_sudo: bool,
    /// Keep uploaded script on remote after execution.
    pub exec_keep: bool,
    /// Deprecated: the Sync tab no longer has a config/ad-hoc mode toggle
    /// (config entries and ad-hoc paths are now used together). Retained so
    /// older state files still deserialize; no longer read.
    pub sync_mode: SyncMode,
    /// Sync tab: whether to do a dry run (no files transferred).
    pub sync_dry_run: bool,
    /// Check tab: whether to do a dry run.
    pub check_dry_run: bool,
    /// Run tab: whether to do a dry run.
    pub run_dry_run: bool,
    /// Exec tab: whether to do a dry run.
    pub exec_dry_run: bool,
    /// View tab: selected view operation (checkout/list/log).
    pub view_operation: ViewOperationKind,
    /// View tab: include history when running checkout.
    pub checkout_history: bool,
    /// View tab: show combined (per-metric latest) instead of single-snapshot checkout.
    pub checkout_combined: bool,
    /// View tab: number of log entries to fetch (0 → App default of 20).
    pub log_last: usize,
    /// View tab: restrict log results to error rows.
    pub log_errors: bool,
    /// Operate tab: persisted input field values.
    pub run_command: String,
    pub exec_script: String,
    pub cp_local: String,
    pub cp_remote: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewOperationKind {
    #[default]
    Checkout,
    List,
    Log,
}

/// Sync params panel mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncMode {
    #[default]
    ConfigEntries,
    AdHoc,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationKind {
    #[default]
    Check,
    Run,
    Exec,
    Sync,
    Cp,
}

// ---------- path computation (AD-16) ----------

/// Compute the 8-hex-char `config_hash` for a config path.
///
/// `effective_path_str` is derived per AD-16:
///   1. resolve_path(custom_path) — applies default + tilde expansion.
///   2. canonicalize() if the file exists; fall back to to_string_lossy().
fn config_hash(custom_path: Option<&Path>) -> String {
    let resolved = match crate::config::app::resolve_path(custom_path) {
        Ok(p) => p,
        Err(_) => return "00000000".to_string(),
    };
    let s = match std::fs::canonicalize(&resolved) {
        Ok(canon) => canon.to_string_lossy().into_owned(),
        Err(_) => resolved.to_string_lossy().into_owned(),
    };
    let bytes = blake3::hash(s.as_bytes());
    let h = bytes.as_bytes();
    format!("{:02x}{:02x}{:02x}{:02x}", h[0], h[1], h[2], h[3])
}

/// Full path to the TUI state file for the given config.
pub fn state_file_path(config: &AppConfig, config_path: Option<&Path>) -> Result<PathBuf> {
    let dir = crate::state::db::resolved_state_dir(config.settings.state_dir.as_deref())?;
    let hash = config_hash(config_path);
    Ok(dir.join(format!("tui_state-{}.toml", hash)))
}

// ---------- load / save ----------

/// Load persisted state from disk.
///
/// Behavior on failure (per §16.1):
/// - File missing → return default state silently.
/// - Read or parse error → return default state, emit `tracing::warn!`,
///   never panic.
pub fn load(path: &Path) -> TuiPersistedState {
    match std::fs::read_to_string(path) {
        Ok(content) => match toml::from_str::<TuiPersistedState>(&content) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    "TUI state file at {} is malformed; starting fresh: {e}",
                    path.display()
                );
                TuiPersistedState::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => TuiPersistedState::default(),
        Err(e) => {
            tracing::warn!(
                "TUI state file at {} could not be read; starting fresh: {e}",
                path.display()
            );
            TuiPersistedState::default()
        }
    }
}

/// Atomically write persisted state to disk.
///
/// Failures emit `tracing::warn!` but do not crash the TUI.
pub fn save(path: &Path, state: &TuiPersistedState) -> Result<()> {
    let serialized = toml::to_string_pretty(state).context("Failed to serialize TUI state")?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("State path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create {}", parent.display()))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".tui-state-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .with_context(|| format!("Failed to create temp file in {}", parent.display()))?;
    tmp.as_file_mut()
        .write_all(serialized.as_bytes())
        .context("Failed to write TUI state temp file")?;
    tmp.as_file_mut()
        .flush()
        .context("Failed to flush TUI state temp file")?;
    tmp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("Failed to persist {}", path.display()))?;
    Ok(())
}

// ---------- §16.2 validation ----------

/// Sanitise persisted filter state against the current AppConfig.
///
/// - groups not referenced anywhere in the config (host/check/sync) are dropped.
/// - hosts not in `config.host[].name` are dropped.
/// - The selected mode is preserved even when its list ends up empty: an empty
///   Groups/Hosts selection means "no targets", which is clearer (and safer)
///   than silently widening to All. Shell mode is always valid.
pub fn validate_filter(state: &mut TargetFilterState, config: &AppConfig) {
    let known_groups: std::collections::BTreeSet<String> = config
        .host
        .iter()
        .flat_map(|h| h.groups.iter().cloned())
        .filter(|g| !g.is_empty())
        .collect();
    let known_hosts: std::collections::BTreeSet<String> =
        config.host.iter().map(|h| h.name.clone()).collect();

    state.groups.retain(|g| known_groups.contains(g));
    state.hosts.retain(|h| known_hosts.contains(h));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{HostEntry, ShellType};

    #[test]
    fn empty_string_loads_as_default() {
        let s: TuiPersistedState = toml::from_str("").unwrap();
        assert_eq!(s.tui_state.active_tab, ActiveTab::View);
        assert_eq!(s.target_filter.mode, TargetFilterMode::All);
        assert_eq!(s.operate.operation, OperationKind::Check);
    }

    #[test]
    fn round_trip_preserves_values() {
        let mut s = TuiPersistedState::default();
        s.tui_state.active_tab = ActiveTab::Operate;
        s.target_filter.mode = TargetFilterMode::Groups;
        s.target_filter.groups = vec!["web".to_string(), "db".to_string()];
        s.target_filter.timeout = 90;
        let serialized = toml::to_string(&s).unwrap();
        let parsed: TuiPersistedState = toml::from_str(&serialized).unwrap();
        assert_eq!(parsed.tui_state.active_tab, ActiveTab::Operate);
        assert_eq!(parsed.target_filter.mode, TargetFilterMode::Groups);
        assert_eq!(parsed.target_filter.groups, vec!["web", "db"]);
        assert_eq!(parsed.target_filter.timeout, 90);
    }

    #[test]
    fn missing_keys_load_as_defaults() {
        let s: TuiPersistedState = toml::from_str(
            r#"
[tui_state]
active_tab = "Config"
"#,
        )
        .unwrap();
        assert_eq!(s.tui_state.active_tab, ActiveTab::Config);
        assert_eq!(s.target_filter.mode, TargetFilterMode::All);
    }

    #[test]
    fn malformed_file_returns_default() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "this = is not valid [toml").unwrap();
        let s = load(tmp.path());
        // Default values, not panic.
        assert_eq!(s.tui_state.active_tab, ActiveTab::View);
    }

    #[test]
    fn missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("no-such-file.toml");
        let s = load(&path);
        assert_eq!(s.tui_state.active_tab, ActiveTab::View);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        let mut s = TuiPersistedState::default();
        s.tui_state.active_tab = ActiveTab::Config;
        s.target_filter.mode = TargetFilterMode::Hosts;
        s.target_filter.hosts = vec!["web1".to_string()];
        s.target_filter.skip = vec!["h9".to_string()];
        s.operate.view_operation = ViewOperationKind::Log;
        s.operate.log_last = 75;
        s.operate.log_errors = true;
        save(&path, &s).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.tui_state.active_tab, ActiveTab::Config);
        assert_eq!(loaded.target_filter.mode, TargetFilterMode::Hosts);
        assert_eq!(loaded.target_filter.hosts, vec!["web1"]);
        assert_eq!(loaded.target_filter.skip, vec!["h9"]);
        assert_eq!(loaded.operate.view_operation, ViewOperationKind::Log);
        assert_eq!(loaded.operate.log_last, 75);
        assert!(loaded.operate.log_errors);
    }

    fn cfg_with_hosts(specs: &[(&str, &[&str])]) -> AppConfig {
        let mut cfg = AppConfig::default();
        for (name, groups) in specs {
            cfg.host.push(HostEntry {
                name: name.to_string(),
                ssh_host: name.to_string(),
                shell: ShellType::Sh,
                groups: groups.iter().map(|s| s.to_string()).collect(),
                proxy_jump: None,
            });
        }
        cfg
    }

    #[test]
    fn validate_drops_unknown_groups() {
        let cfg = cfg_with_hosts(&[("h1", &["web"])]);
        let mut f = TargetFilterState {
            mode: TargetFilterMode::Groups,
            groups: vec!["web".to_string(), "ghost".to_string()],
            ..Default::default()
        };
        validate_filter(&mut f, &cfg);
        assert_eq!(f.groups, vec!["web"]);
        assert_eq!(f.mode, TargetFilterMode::Groups);
    }

    #[test]
    fn validate_keeps_groups_mode_when_list_becomes_empty() {
        let cfg = cfg_with_hosts(&[("h1", &["web"])]);
        let mut f = TargetFilterState {
            mode: TargetFilterMode::Groups,
            groups: vec!["ghost".to_string()],
            ..Default::default()
        };
        validate_filter(&mut f, &cfg);
        // Unknown group is dropped, but the mode is preserved (empty = 0 hosts,
        // not a silent widen to All).
        assert!(f.groups.is_empty());
        assert_eq!(f.mode, TargetFilterMode::Groups);
    }

    #[test]
    fn validate_drops_unknown_hosts() {
        let cfg = cfg_with_hosts(&[("h1", &[]), ("h2", &[])]);
        let mut f = TargetFilterState {
            mode: TargetFilterMode::Hosts,
            hosts: vec!["h1".to_string(), "h99".to_string()],
            ..Default::default()
        };
        validate_filter(&mut f, &cfg);
        assert_eq!(f.hosts, vec!["h1"]);
    }

    #[test]
    fn operate_state_extended_round_trips() {
        let s = OperateState {
            log_last: 50,
            log_errors: true,
            checkout_history: true,
            check_dry_run: true,
            run_dry_run: true,
            exec_dry_run: true,
            view_operation: ViewOperationKind::Log,
            ..Default::default()
        };
        let ser = toml::to_string(&s).unwrap();
        let back: OperateState = toml::from_str(&ser).unwrap();
        assert_eq!(back.log_last, 50);
        assert!(back.log_errors);
        assert!(back.checkout_history);
        assert!(back.check_dry_run);
        assert!(back.run_dry_run);
        assert!(back.exec_dry_run);
        assert_eq!(back.view_operation, ViewOperationKind::Log);
    }

    #[test]
    fn view_operation_kind_defaults_checkout() {
        assert_eq!(ViewOperationKind::default(), ViewOperationKind::Checkout);
    }

    #[test]
    fn view_operation_kind_non_default_variants_round_trip() {
        for v in [ViewOperationKind::List, ViewOperationKind::Log] {
            let s = OperateState {
                view_operation: v,
                ..Default::default()
            };
            let back: OperateState = toml::from_str(&toml::to_string(&s).unwrap()).unwrap();
            assert_eq!(back.view_operation, v);
        }
    }

    #[test]
    fn config_hash_is_deterministic() {
        let p = std::path::Path::new("/tmp/dummy-cfg-that-does-not-exist.toml");
        let h1 = config_hash(Some(p));
        let h2 = config_hash(Some(p));
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 8);
    }

    #[test]
    fn config_hash_differs_per_path() {
        let a = config_hash(Some(std::path::Path::new("/tmp/a.toml")));
        let b = config_hash(Some(std::path::Path::new("/tmp/b.toml")));
        assert_ne!(a, b);
    }

    #[test]
    fn legacy_checkout_tab_loads_as_view() {
        let s: TuiPersistedState = toml::from_str(
            r#"
[tui_state]
active_tab = "Checkout"
"#,
        )
        .unwrap();
        assert_eq!(s.tui_state.active_tab, ActiveTab::View);
    }

    #[test]
    fn skip_field_round_trips_and_defaults_empty() {
        let s: TuiPersistedState = toml::from_str("").unwrap();
        assert!(s.target_filter.skip.is_empty());

        let t = TargetFilterState {
            skip: vec!["h9".into()],
            ..Default::default()
        };
        let ser = toml::to_string(&t).unwrap();
        let back: TargetFilterState = toml::from_str(&ser).unwrap();
        assert_eq!(back.skip, vec!["h9".to_string()]);
    }
}
