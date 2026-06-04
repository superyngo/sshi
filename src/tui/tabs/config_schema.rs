//! Unified field schema for the Config tab.
//!
//! Single source of truth for both right-panel inline editing and entry-form
//! popups. Two free functions per entry kind:
//!
//! - `fields(entry) -> Vec<FieldDescriptor>` — what to render and what `key` to
//!   pass to `apply()`. Optional fields (proxy_jump, mode, source) are ALWAYS
//!   included; they render with an empty `display_value` when unset.
//! - `apply(entry, key, val) -> ()` — write `val` (raw form) into the field
//!   identified by `key`. Unknown keys are no-ops.
//!
//! ### Raw vs display contract
//! `display_value` on `FieldDescriptor` is human-formatted (`"(none)"`,
//! `"[a, b]"`). `apply()` accepts the **raw** form: empty string for unset
//! optionals; for Vec keys the input is either bracketed display (`"[a, b]"`,
//! `"(none)"`) or a comma-joined string — `parse_bracket_list()` normalises
//! both, so callers can safely pass display values for Vec keys.
//!
//! ### Path key
//! Check.path entries are keyed by index: `path:0`, `path:1`, … Indices are
//! stable for the duration of a single edit (no reorder happens between
//! `fields()` and `apply()` within one mutation).

use crate::config::schema::{
    AppConfig, CheckEntry, ConflictStrategy, HostEntry, Settings, ShellType, SyncEntry,
};

// ── Field descriptor types ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum FieldKind {
    U64,
    Bool,
    String,
    OptionalString,
    Enum {
        variants: Vec<&'static str>,
    },
    VecString,
    #[allow(dead_code)]
    VecCheckPath,
    /// Fixed multi-select for `Check.enabled`.
    CheckEnabled,
    ShellEnum,
    /// `Option<bool>`: "inherit" | "yes" | "no".
    TriBool,
}

#[derive(Debug, Clone)]
pub struct FieldDescriptor {
    pub key: String,
    pub display_value: String,
    pub kind: FieldKind,
    pub editable: bool,
}

impl FieldDescriptor {
    pub fn scalar(key: &str, value: String, kind: FieldKind) -> Self {
        Self {
            key: key.to_string(),
            display_value: value,
            kind,
            editable: true,
        }
    }

    #[allow(dead_code)]
    pub fn readonly(key: &str, value: String) -> Self {
        Self {
            key: key.to_string(),
            display_value: value,
            kind: FieldKind::String,
            editable: false,
        }
    }

    pub fn vec_field(key: &str, display: String, kind: FieldKind) -> Self {
        Self {
            key: key.to_string(),
            display_value: display,
            kind,
            editable: true,
        }
    }
}

// ── Constants ────────────────────────────────────────────────────────────────

pub const CHECK_ENABLED_OPTIONS: &[(&str, &str)] = &[
    ("online", "Check if host is online"),
    ("system_info", "System info (uname / systeminfo)"),
    ("cpu_arch", "CPU architecture"),
    ("memory", "Memory usage"),
    ("swap", "Swap usage"),
    ("disk", "Disk usage"),
    ("cpu_load", "CPU load"),
    ("network", "Network interface info"),
    ("battery", "Battery status"),
    ("ip_address", "IP address"),
];

// ── Field definitions (canonical) ────────────────────────────────────────────

pub fn settings_fields(s: &Settings) -> Vec<FieldDescriptor> {
    let mut f = vec![
        FieldDescriptor::scalar(
            "default_timeout",
            format!("{}s", s.default_timeout),
            FieldKind::U64,
        ),
        FieldDescriptor::scalar(
            "data_retention_days",
            format!("{}d", s.data_retention_days),
            FieldKind::U64,
        ),
        FieldDescriptor::scalar(
            "conflict_strategy",
            format!("{:?}", s.conflict_strategy).to_lowercase(),
            FieldKind::Enum {
                variants: vec!["newest", "skip"],
            },
        ),
        FieldDescriptor::scalar(
            "propagate_deletes",
            s.propagate_deletes.to_string(),
            FieldKind::Bool,
        ),
        FieldDescriptor::scalar(
            "max_concurrency",
            s.max_concurrency.to_string(),
            FieldKind::U64,
        ),
        FieldDescriptor::scalar(
            "max_per_host_concurrency",
            s.max_per_host_concurrency.to_string(),
            FieldKind::U64,
        ),
        FieldDescriptor::scalar(
            "state_dir",
            s.state_dir
                .as_ref()
                .map(|d| d.display().to_string())
                .unwrap_or_default(),
            FieldKind::OptionalString,
        ),
        FieldDescriptor::scalar(
            "default_output_format",
            s.default_output_format.clone().unwrap_or_default(),
            FieldKind::OptionalString,
        ),
    ];
    f.push(FieldDescriptor::vec_field(
        "skipped_hosts",
        fmt_vec(&s.skipped_hosts),
        FieldKind::VecString,
    ));
    f
}

pub fn host_fields(h: &HostEntry) -> Vec<FieldDescriptor> {
    vec![
        FieldDescriptor::scalar("name", h.name.clone(), FieldKind::String),
        FieldDescriptor::scalar("ssh_host", h.ssh_host.clone(), FieldKind::String),
        FieldDescriptor::scalar("shell", h.shell.to_string(), FieldKind::ShellEnum),
        FieldDescriptor::vec_field("groups", fmt_vec(&h.groups), FieldKind::VecString),
        FieldDescriptor::scalar(
            "proxy_jump",
            h.proxy_jump.clone().unwrap_or_default(),
            FieldKind::OptionalString,
        ),
    ]
}

pub fn check_fields(c: &CheckEntry) -> Vec<FieldDescriptor> {
    let mut f = vec![
        FieldDescriptor::scalar(
            "name",
            c.name.clone().unwrap_or_default(),
            FieldKind::OptionalString,
        ),
        FieldDescriptor::vec_field("enabled", fmt_vec(&c.enabled), FieldKind::CheckEnabled),
    ];
    for (i, p) in c.path.iter().enumerate() {
        f.push(FieldDescriptor::scalar(
            &format!("path:{i}"),
            format!("{} → {}", p.label, p.path),
            FieldKind::String,
        ));
    }
    f
}

pub fn sync_fields(s: &SyncEntry) -> Vec<FieldDescriptor> {
    vec![
        FieldDescriptor::scalar(
            "name",
            s.name.clone().unwrap_or_default(),
            FieldKind::OptionalString,
        ),
        FieldDescriptor::vec_field("paths", fmt_vec(&s.paths), FieldKind::VecString),
        FieldDescriptor::scalar("recursive", s.recursive.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar(
            "mode",
            s.mode.clone().unwrap_or_default(),
            FieldKind::OptionalString,
        ),
        FieldDescriptor::scalar(
            "propagate_deletes",
            tribool_from_opt(s.propagate_deletes).to_string(),
            FieldKind::TriBool,
        ),
        FieldDescriptor::scalar(
            "source",
            s.source.clone().unwrap_or_default(),
            FieldKind::OptionalString,
        ),
    ]
}

// ── Apply (key-routed, complete) ────────────────────────────────────────────

pub fn apply_settings(config: &mut AppConfig, key: &str, val: &str) {
    let s = &mut config.settings;
    match key {
        "default_timeout" => {
            if let Ok(v) = strip_suffix(val, 's').parse::<u64>() {
                s.default_timeout = v;
            }
        }
        "data_retention_days" => {
            if let Ok(v) = strip_suffix(val, 'd').parse::<u64>() {
                s.data_retention_days = v;
            }
        }
        "conflict_strategy" => {
            s.conflict_strategy = match val {
                "skip" => ConflictStrategy::Skip,
                _ => ConflictStrategy::Newest,
            };
        }
        "propagate_deletes" => s.propagate_deletes = val == "true",
        "max_concurrency" => {
            if let Ok(v) = val.parse::<usize>() {
                s.max_concurrency = v;
            }
        }
        "max_per_host_concurrency" => {
            if let Ok(v) = val.parse::<usize>() {
                s.max_per_host_concurrency = v;
            }
        }
        "state_dir" => {
            s.state_dir = if val.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(val))
            };
        }
        "default_output_format" => {
            s.default_output_format = if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            };
        }
        "skipped_hosts" => s.skipped_hosts = parse_bracket_list(val),
        _ => {}
    }
}

pub fn apply_host(host: &mut HostEntry, key: &str, val: &str) {
    match key {
        "name" => host.name = val.to_string(),
        "ssh_host" => host.ssh_host = val.to_string(),
        "shell" => {
            host.shell = match val {
                "powershell" => ShellType::PowerShell,
                "cmd" => ShellType::Cmd,
                _ => ShellType::Sh,
            };
        }
        "groups" => host.groups = parse_bracket_list(val),
        "proxy_jump" => {
            host.proxy_jump = if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            };
        }
        _ => {}
    }
}

pub fn apply_check(check: &mut CheckEntry, key: &str, val: &str) {
    match key {
        "name" => check.name = opt_string(val),
        "enabled" => check.enabled = parse_bracket_list(val),
        k if k.starts_with("path:") => {
            // path entries are edited via dedicated form, not single-string apply.
        }
        _ => {}
    }
}

pub fn apply_sync(sync: &mut SyncEntry, key: &str, val: &str) {
    match key {
        "name" => sync.name = opt_string(val),
        "paths" => sync.paths = parse_bracket_list(val),
        "recursive" => sync.recursive = val == "true",
        "mode" => {
            sync.mode = if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            }
        }
        "propagate_deletes" => sync.propagate_deletes = tribool_to_opt(val),
        "source" => {
            sync.source = if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            }
        }
        _ => {}
    }
}

// ── Helpers (shared) ─────────────────────────────────────────────────────────

/// Empty string → None, otherwise Some(trimmed-as-is).
fn opt_string(val: &str) -> Option<String> {
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

pub fn fmt_vec(v: &[String]) -> String {
    if v.is_empty() {
        "(none)".to_string()
    } else {
        format!("[{}]", v.join(", "))
    }
}

pub fn parse_bracket_list(s: &str) -> Vec<String> {
    let inner = s.trim_start_matches('[').trim_end_matches(']');
    if inner.is_empty() || inner == "(none)" || inner == "(unscoped)" {
        return vec![];
    }
    inner
        .split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

pub fn tribool_from_opt(v: Option<bool>) -> &'static str {
    match v {
        None => "inherit",
        Some(true) => "yes",
        Some(false) => "no",
    }
}

pub fn tribool_to_opt(s: &str) -> Option<bool> {
    match s {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

fn strip_suffix(s: &str, c: char) -> &str {
    s.strip_suffix(c).unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::CheckPath;

    fn empty_host() -> HostEntry {
        HostEntry {
            name: "h1".into(),
            ssh_host: "1.2.3.4".into(),
            shell: ShellType::Sh,
            groups: vec![],
            proxy_jump: None,
        }
    }

    fn empty_check() -> CheckEntry {
        CheckEntry {
            name: None,
            id: "c1".into(),
            enabled: vec![],
            path: vec![],
        }
    }

    fn empty_sync() -> SyncEntry {
        SyncEntry {
            name: None,
            id: "s1".into(),
            paths: vec![],
            recursive: false,
            mode: None,
            propagate_deletes: None,
            source: None,
        }
    }

    #[test]
    fn host_groups_round_trip_via_apply() {
        let mut h = empty_host();
        apply_host(&mut h, "groups", "[a, b, c]");
        assert_eq!(h.groups, vec!["a", "b", "c"]);

        // Display value form also accepted.
        apply_host(&mut h, "groups", "(none)");
        assert!(h.groups.is_empty());
    }

    #[test]
    fn host_proxy_jump_empty_clears() {
        let mut h = empty_host();
        apply_host(&mut h, "proxy_jump", "bastion");
        assert_eq!(h.proxy_jump.as_deref(), Some("bastion"));
        apply_host(&mut h, "proxy_jump", "");
        assert_eq!(h.proxy_jump, None);
    }

    #[test]
    fn check_enabled_and_name_apply() {
        let mut c = empty_check();
        apply_check(&mut c, "enabled", "[online, cpu_load]");
        assert_eq!(c.enabled, vec!["online", "cpu_load"]);
        apply_check(&mut c, "name", "default");
        assert_eq!(c.name.as_deref(), Some("default"));
        apply_check(&mut c, "name", "");
        assert_eq!(c.name, None);
    }

    #[test]
    fn sync_paths_and_name_apply() {
        let mut s = empty_sync();
        apply_sync(&mut s, "paths", "[/etc, /var/log]");
        assert_eq!(s.paths, vec!["/etc", "/var/log"]);
        apply_sync(&mut s, "name", "dotfiles");
        assert_eq!(s.name.as_deref(), Some("dotfiles"));
    }

    #[test]
    fn host_fields_always_includes_proxy_jump() {
        let h = empty_host();
        let f = host_fields(&h);
        assert!(f.iter().any(|d| d.key == "proxy_jump"));
    }

    #[test]
    fn check_enabled_kind_is_check_enabled() {
        let c = empty_check();
        let f = check_fields(&c);
        let enabled = f.iter().find(|d| d.key == "enabled").unwrap();
        assert!(matches!(enabled.kind, FieldKind::CheckEnabled));
    }

    #[test]
    fn check_path_keys_are_indexed() {
        let mut c = empty_check();
        c.path = vec![
            CheckPath {
                label: "a".into(),
                path: "/a".into(),
            },
            CheckPath {
                label: "a".into(),
                path: "/b".into(),
            }, // duplicate label OK
        ];
        let f = check_fields(&c);
        assert!(f.iter().any(|d| d.key == "path:0"));
        assert!(f.iter().any(|d| d.key == "path:1"));
    }
}
