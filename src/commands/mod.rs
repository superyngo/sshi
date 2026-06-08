pub mod check;
pub mod checkout;
pub mod config;
pub mod cp;
pub mod exec;
pub mod init;
pub mod list;
pub mod log;
pub mod report;
pub mod run;
pub mod sync;

use anyhow::{bail, Result};
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::cli::TargetArgs;
use crate::config::schema::{AppConfig, CheckEntry, HostEntry, SyncEntry};

/// Target mode derived from CLI flags.
#[derive(Debug, Clone, PartialEq)]
pub enum TargetMode {
    /// --all: all configured hosts
    All,
    /// --host: specific hosts by name
    Hosts(Vec<String>),
    /// --group: hosts belonging to named groups
    Groups(Vec<String>),
    /// --shell: hosts with the specified shell type(s)
    Shell(Vec<crate::config::schema::ShellType>),
}

/// Shared context available to all commands.
pub struct Context {
    pub config: AppConfig,
    pub config_path: Option<PathBuf>,
    pub db: Connection,
    pub timeout: u64,
    pub mode: TargetMode,
    pub serial: bool,
    pub skip: Vec<String>,
    #[allow(dead_code)]
    pub verbose: bool,
}

impl Context {
    pub async fn new(
        verbose: bool,
        target: &TargetArgs,
        config_path: Option<&Path>,
    ) -> Result<Self> {
        let config = crate::config::app::load(config_path)?.unwrap_or_default();
        let db = crate::state::db::open(config.settings.state_dir.as_deref())?;
        let timeout = target.timeout.unwrap_or(config.settings.default_timeout);
        let mode = resolve_target_mode(target, &config)?;

        Ok(Self {
            config,
            config_path: config_path.map(|p| p.to_path_buf()),
            db,
            timeout,
            mode,
            serial: target.serial,
            skip: target.skip.clone(),
            verbose,
        })
    }

    /// Build a `Context` for a single TUI-driven operation (per
    /// docs/tui_reconstruct_plan.md §6.4 and AD-5/AD-6/AD-16).
    ///
    /// The caller has already cloned `config` from `App.config` (AD-6 ownership
    /// rule). A fresh `rusqlite::Connection` is opened per call against the
    /// resolved state directory; `App.db` is never moved or shared (AD-5).
    #[cfg(feature = "tui")]
    pub fn from_tui_parts(
        config: AppConfig,
        config_path: Option<PathBuf>,
        mode: TargetMode,
        serial: bool,
        timeout: u64,
        verbose: bool,
        skip: Vec<String>,
    ) -> Result<Self> {
        let db = crate::state::db::open(config.settings.state_dir.as_deref())?;
        Ok(Self {
            config,
            config_path,
            db,
            timeout,
            mode,
            serial,
            skip,
            verbose,
        })
    }

    /// Create a context without target args (for commands like init, config, log).
    pub async fn new_without_targets(
        verbose: bool,
        config_path: Option<&Path>,
        timeout_override: Option<u64>,
    ) -> Result<Self> {
        let config = crate::config::app::load(config_path)?.unwrap_or_default();
        let db = crate::state::db::open(config.settings.state_dir.as_deref())?;
        let timeout = timeout_override.unwrap_or(config.settings.default_timeout);

        Ok(Self {
            config,
            config_path: config_path.map(|p| p.to_path_buf()),
            db,
            timeout,
            mode: TargetMode::All,
            serial: false,
            skip: Vec::new(),
            verbose,
        })
    }

    /// Resolve targeted hosts based on mode.
    /// For --all: all hosts. For --host: named hosts. For --group: hosts in group.
    pub fn resolve_hosts(&self) -> Result<Vec<&HostEntry>> {
        let hosts: Vec<&HostEntry> = match &self.mode {
            TargetMode::All => self.config.host.iter().collect(),
            TargetMode::Hosts(names) => self
                .config
                .host
                .iter()
                .filter(|h| names.contains(&h.name))
                .collect(),
            TargetMode::Groups(groups) => self
                .config
                .host
                .iter()
                .filter(|h| h.groups.iter().any(|g| groups.contains(g)))
                .collect(),
            TargetMode::Shell(shells) => self
                .config
                .host
                .iter()
                .filter(|h| shells.contains(&h.shell))
                .collect(),
        };

        let hosts = filter_skipped(hosts, &self.skip);

        if hosts.is_empty() {
            let mut hint = String::from("No hosts matched the specified filter.");
            if let TargetMode::Shell(shells) = &self.mode {
                hint = format!(
                    "No hosts matched shell type: {}",
                    shells
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                let mut shell_map: BTreeMap<String, Vec<String>> = BTreeMap::new();
                for h in &self.config.host {
                    shell_map
                        .entry(h.shell.to_string())
                        .or_default()
                        .push(h.name.clone());
                }
                if !shell_map.is_empty() {
                    let parts: Vec<String> = shell_map
                        .iter()
                        .map(|(shell, hosts)| format!("{} ({})", shell, hosts.join(", ")))
                        .collect();
                    hint.push_str(&format!("\nAvailable shells: {}", parts.join(", ")));
                }
            } else {
                append_available_hints(&self.config, &mut hint);
            }
            bail!("{}", hint);
        }

        Ok(hosts)
    }

    /// Get the global concurrency limit.
    pub fn concurrency(&self) -> usize {
        if self.serial {
            1
        } else {
            self.config.settings.max_concurrency
        }
    }

    /// Get the per-host concurrency limit.
    pub fn per_host_concurrency(&self) -> usize {
        if self.serial {
            1
        } else {
            self.config.settings.max_per_host_concurrency
        }
    }

    /// Resolve check entries selected by `--name`. With no names, falls back to
    /// the entry named `"default"` (if any). Target mode selects hosts, not
    /// entries.
    pub fn resolve_checks(&self, names: &[String]) -> Vec<&CheckEntry> {
        select_named(
            &self.config.check,
            |e| e.name.as_deref(),
            names,
            Some("default"),
            "check",
        )
    }

    /// Resolve sync entries selected by `--name`. No default: with no names this
    /// returns nothing (the caller combines named entries with positional paths).
    pub fn resolve_syncs(&self, names: &[String]) -> Vec<&SyncEntry> {
        select_named(
            &self.config.sync,
            |e| e.name.as_deref(),
            names,
            None,
            "sync",
        )
    }
}

/// Select config entries by `name`. When `names` is empty, `default_name` (if
/// any) is used. Unknown requested names and duplicate config names are logged
/// as warnings; all matching entries are returned.
fn select_named<'a, T>(
    entries: &'a [T],
    get_name: impl Fn(&T) -> Option<&str>,
    names: &[String],
    default_name: Option<&str>,
    kind: &str,
) -> Vec<&'a T> {
    let wanted: Vec<&str> = if names.is_empty() {
        default_name.into_iter().collect()
    } else {
        names.iter().map(|s| s.as_str()).collect()
    };
    if wanted.is_empty() {
        return Vec::new();
    }

    let mut selected: Vec<&T> = Vec::new();
    for name in &wanted {
        let matches: Vec<&T> = entries
            .iter()
            .filter(|e| get_name(e) == Some(*name))
            .collect();
        match matches.len() {
            0 => tracing::warn!("no {} entry named '{}'", kind, name),
            n => {
                if n > 1 {
                    tracing::warn!(
                        "{} {} entries named '{}' — all will be applied",
                        n,
                        kind,
                        name
                    );
                }
                selected.extend(matches);
            }
        }
    }
    selected
}

/// Resolve which target mode the user intended, or show helpful error.
fn resolve_target_mode(target: &TargetArgs, config: &AppConfig) -> Result<TargetMode> {
    let has_all = target.all;
    let has_hosts = !target.host.is_empty();
    let has_groups = !target.group.is_empty();
    let has_shell = !target.shell.is_empty();

    let count = has_all as u8 + has_hosts as u8 + has_groups as u8 + has_shell as u8;

    if count == 0 {
        let mut hint = String::from(
            "Target required. Use --group/-g, --host/-h, --shell/-s, or --all/-a to specify targets.",
        );
        if config.host.is_empty() {
            hint.push_str("\nHint: Run 'sshi init' first to import hosts from ~/.ssh/config.");
        } else {
            append_available_hints(config, &mut hint);
        }
        bail!("{}", hint);
    }

    if count > 1 {
        bail!("Only one of --all/-a, --host/-h, --group/-g, or --shell/-s can be used at a time.");
    }

    if has_all {
        Ok(TargetMode::All)
    } else if has_hosts {
        Ok(TargetMode::Hosts(target.host.clone()))
    } else if has_groups {
        Ok(TargetMode::Groups(target.group.clone()))
    } else {
        Ok(TargetMode::Shell(target.shell.clone()))
    }
}

/// Append available groups and hosts to hint message.
fn append_available_hints(config: &AppConfig, hint: &mut String) {
    let groups = collect_available_groups(config);
    if !groups.is_empty() {
        hint.push_str(&format!(
            "\n\nAvailable groups: {}",
            groups.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if !config.host.is_empty() {
        let names: Vec<&str> = config.host.iter().map(|h| h.name.as_str()).collect();
        hint.push_str(&format!("\nAvailable hosts: {}", names.join(", ")));
    }
}

/// Collect available group names from host[].groups tags.
fn collect_available_groups(config: &AppConfig) -> BTreeSet<String> {
    let mut groups = BTreeSet::new();
    for h in &config.host {
        for g in &h.groups {
            if !g.is_empty() {
                groups.insert(g.clone());
            }
        }
    }
    groups
}

/// Drop any host whose name appears in `skip`. Unknown names are ignored.
fn filter_skipped<'a>(hosts: Vec<&'a HostEntry>, skip: &[String]) -> Vec<&'a HostEntry> {
    if skip.is_empty() {
        return hosts;
    }
    hosts
        .into_iter()
        .filter(|h| !skip.iter().any(|s| s == &h.name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{HostEntry, ShellType};

    fn host(name: &str) -> HostEntry {
        HostEntry {
            name: name.to_string(),
            ssh_host: name.to_string(),
            groups: vec![],
            shell: ShellType::Sh,
            proxy_jump: None,
        }
    }

    #[test]
    fn select_named_defaults_and_matches() {
        let entries = vec![
            ("default".to_string(), 1),
            ("extra".to_string(), 2),
            ("default".to_string(), 3),
        ];
        fn get(e: &(String, i32)) -> Option<&str> {
            Some(e.0.as_str())
        }

        // No names → fall back to "default" (matches both entries named default).
        let got: Vec<i32> = select_named(&entries, get, &[], Some("default"), "x")
            .iter()
            .map(|e| e.1)
            .collect();
        assert_eq!(got, vec![1, 3]);

        // Explicit name selects only that entry.
        let got: Vec<i32> =
            select_named(&entries, get, &["extra".to_string()], Some("default"), "x")
                .iter()
                .map(|e| e.1)
                .collect();
        assert_eq!(got, vec![2]);

        // No names and no default → empty (sync semantics).
        assert!(select_named(&entries, get, &[], None, "x").is_empty());

        // Unknown name → empty.
        assert!(select_named(&entries, get, &["nope".to_string()], None, "x").is_empty());
    }

    #[test]
    fn filter_skipped_removes_named_hosts() {
        let h1 = host("h1");
        let h2 = host("h2");
        let h3 = host("h3");
        let all: Vec<&HostEntry> = vec![&h1, &h2, &h3];

        let kept = filter_skipped(all.clone(), &["h2".to_string()]);
        let names: Vec<&str> = kept.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["h1", "h3"]);

        // unknown skip is a no-op
        let kept = filter_skipped(all.clone(), &["nope".to_string()]);
        assert_eq!(kept.len(), 3);

        // skip-all yields empty
        let kept = filter_skipped(all, &["h1".into(), "h2".into(), "h3".into()]);
        assert!(kept.is_empty());
    }
}
