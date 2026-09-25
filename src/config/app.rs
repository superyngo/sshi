//! TOML config file I/O with structured editing via toml_edit.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table, Value};

use super::schema::{AppConfig, CheckEntry, HostEntry, SyncEntry};

/// Returns the platform-appropriate config directory for sshi
/// (see [`crate::util::app_dir`]).
pub fn config_dir() -> Result<PathBuf> {
    crate::util::app_dir(crate::util::AppDir::Config)
}

/// Returns the path to config.toml.
pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// Resolve the effective config file path (AD-16): expands `~` so explicit and
/// default paths hash identically.
pub fn resolve_path(custom_path: Option<&Path>) -> Result<PathBuf> {
    match custom_path {
        Some(p) => Ok(crate::util::expand_tilde(p)),
        None => config_path(),
    }
}

/// Strip a leading UTF-8 BOM from the input if present.
fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// Load config from disk. Returns None if file doesn't exist.
pub fn load(custom_path: Option<&Path>) -> Result<Option<AppConfig>> {
    let path = resolve_path(custom_path)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let content = strip_bom(&raw);
    let config: AppConfig =
        toml::from_str(content).with_context(|| format!("Failed to parse {}", path.display()))?;
    warn_legacy_entry_names(&config);
    Ok(Some(config))
}

/// Non-fatal: warn about legacy `[[check]]`/`[[sync]]` entries with empty or
/// duplicate names. Names are used as selection keys, so collisions/blanks are
/// ambiguous, but we don't reject the file (back-compat).
fn warn_legacy_entry_names(config: &AppConfig) {
    use std::collections::HashSet;
    let mut check_seen: HashSet<&str> = HashSet::new();
    for c in &config.check {
        match c.name.as_deref().map(str::trim) {
            None | Some("") => tracing::warn!("[[check]] entry has an empty name"),
            Some(n) if !check_seen.insert(n) => {
                tracing::warn!("[[check]] name '{n}' is duplicated")
            }
            _ => {}
        }
    }
    let mut sync_seen: HashSet<&str> = HashSet::new();
    for s in &config.sync {
        match s.name.as_deref().map(str::trim) {
            None | Some("") => tracing::warn!("[[sync]] entry has an empty name"),
            Some(n) if !sync_seen.insert(n) => {
                tracing::warn!("[[sync]] name '{n}' is duplicated")
            }
            _ => {}
        }
    }
}

/// Save config to disk, creating parent directories if needed.
///
/// First-create path: serialize fresh and inject `inject_config_comments` guidance.
/// Edit path: parse existing file with `toml_edit::DocumentMut`, mutate scalars
/// in place, and write back — preserving user comments and key order.
/// Atomic write via `tempfile::persist()` (cross-platform safe).
pub fn save(config: &AppConfig, custom_path: Option<&Path>) -> Result<()> {
    let path = resolve_path(custom_path)?;
    // B43: write through a symlinked config (rename would replace the link).
    let path = match std::fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(&path)
            .with_context(|| format!("Failed to resolve symlink {}", path.display()))?,
        _ => path,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let new_content = match std::fs::read_to_string(&path) {
        Ok(original) => {
            let trimmed = strip_bom(&original);
            match trimmed.parse::<DocumentMut>() {
                Ok(mut doc) => {
                    apply_config_to_doc(&mut doc, config)?;
                    let candidate = doc.to_string();
                    // Round-trip validate: catch apply_config_to_doc bugs before writing.
                    toml::from_str::<AppConfig>(&candidate)
                        .context("apply_config_to_doc produced invalid TOML; aborting write")?;
                    candidate
                }
                Err(e) => {
                    tracing::warn!(
                        "Config comments lost — file contained non-standard formatting \
                         that toml_edit could not parse: {e}"
                    );
                    toml::to_string_pretty(config).context("Failed to serialize config")?
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let serialized =
                toml::to_string_pretty(config).context("Failed to serialize config")?;
            inject_config_comments(&serialized)
        }
        Err(e) => return Err(e).context("Failed to read existing config"),
    };

    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::Builder::new()
        .prefix(".sshi-config-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .with_context(|| format!("Failed to create temp file in {}", parent.display()))?;
    tmp.as_file_mut()
        .write_all(new_content.as_bytes())
        .context("Failed to write temp config file")?;
    tmp.as_file_mut()
        .flush()
        .context("Failed to flush temp config file")?;
    tmp.as_file()
        .sync_all()
        .context("Failed to fsync temp config file")?;
    tmp.persist(&path)
        .map_err(|e| e.error)
        .with_context(|| format!("Failed to persist {}", path.display()))?;
    Ok(())
}

/// Set a scalar key in `table`, preserving the existing item's decor
/// (whitespace and inline comments) if the key already exists.
fn set_scalar<V: Into<Value>>(table: &mut Table, key: &str, v: V) {
    let new_val: Value = v.into();
    match table.get_mut(key) {
        Some(Item::Value(existing)) => {
            let decor = existing.decor().clone();
            let mut replacement = new_val;
            *replacement.decor_mut() = decor;
            *existing = replacement;
        }
        Some(slot) => {
            *slot = Item::Value(new_val);
        }
        None => {
            table.insert(key, Item::Value(new_val));
        }
    }
}

/// Mutate a parsed TOML document in place to reflect the in-memory `AppConfig`.
///
/// `[settings]` is updated in place via `set_scalar`, preserving per-key inline
/// comments and decor. `[[host]]` / `[[check]]` / `[[sync]]` array-of-tables
/// are fully rebuilt: per-entry inline comments are lost but top-level section
/// comments survive; unknown keys inside an entry are carried over from the
/// matching old entry (`write_aot`, B43). Unknown top-level keys are preserved
/// automatically by `toml_edit`.
fn apply_config_to_doc(doc: &mut DocumentMut, config: &AppConfig) -> Result<()> {
    // Ensure [settings] exists as a table; B43: accept inline `settings = {…}`.
    let replacement = match doc.get_mut("settings") {
        None => Some(Table::new()),
        Some(Item::Table(_)) => None,
        Some(Item::Value(Value::InlineTable(t))) => Some(std::mem::take(t).into_table()),
        Some(_) => anyhow::bail!("`settings` in the config file must be a table"),
    };
    if let Some(t) = replacement {
        doc.insert("settings", Item::Table(t));
    }
    let settings = doc["settings"].as_table_mut().expect("settings is a table");

    set_scalar(
        settings,
        "default_timeout",
        config.settings.default_timeout as i64,
    );
    set_scalar(
        settings,
        "data_retention_days",
        config.settings.data_retention_days as i64,
    );
    set_scalar(
        settings,
        "conflict_strategy",
        match config.settings.conflict_strategy {
            super::schema::ConflictStrategy::Newest => "newest",
            super::schema::ConflictStrategy::Skip => "skip",
        },
    );
    set_scalar(
        settings,
        "propagate_deletes",
        config.settings.propagate_deletes,
    );
    set_scalar(
        settings,
        "max_concurrency",
        config.settings.max_concurrency as i64,
    );
    set_scalar(
        settings,
        "max_per_host_concurrency",
        config.settings.max_per_host_concurrency as i64,
    );

    // skipped_hosts: write as inline array; remove if empty.
    if config.settings.skipped_hosts.is_empty() {
        settings.remove("skipped_hosts");
    } else {
        let mut arr = toml_edit::Array::new();
        for h in &config.settings.skipped_hosts {
            arr.push(h.as_str());
        }
        settings["skipped_hosts"] = value(Value::Array(arr));
    }

    match &config.settings.state_dir {
        Some(dir) => {
            set_scalar(settings, "state_dir", dir.to_string_lossy().into_owned());
        }
        None => {
            settings.remove("state_dir");
        }
    }

    match &config.settings.default_output_format {
        Some(fmt) => {
            set_scalar(settings, "default_output_format", fmt.as_str());
        }
        None => {
            settings.remove("default_output_format");
        }
    }

    // Host/Check/Sync sections: rebuild the entire ArrayOfTables. This
    // preserves the top-level [settings] in-place edits (with comments) but
    // loses per-entry inline comments — acceptable trade-off given the file
    // is tool-managed and inline comments inside [[host]]/[[check]]/[[sync]]
    // entries are rare in practice. Previously these sections were never
    // written back, so any Hosts/Checks/Syncs edit through the TUI was
    // silently dropped on save (only [settings] persisted).
    write_aot(
        doc,
        "host",
        &config.host,
        HOST_KEYS,
        |h: &Arc<HostEntry>| host_to_table(h),
    );
    write_aot(doc, "check", &config.check, CHECK_KEYS, check_to_table);
    write_aot(doc, "sync", &config.sync, SYNC_KEYS, sync_to_table);
    Ok(())
}

/// Schema-known keys per entry type; anything else found in the existing
/// entry is carried over on save (B43).
const HOST_KEYS: &[&str] = &["name", "ssh_host", "shell", "groups", "proxy_jump"];
const CHECK_KEYS: &[&str] = &["name", "id", "enabled", "path"];
const SYNC_KEYS: &[&str] = &[
    "name",
    "id",
    "paths",
    "recursive",
    "mode",
    "propagate_deletes",
    "source",
];

/// Same logical entry: matching non-empty `id`, else matching `name`.
fn same_entry(a: &Table, b: &Table) -> bool {
    let get = |t: &Table, k: &str| {
        t.get(k)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    match (get(a, "id"), get(b, "id")) {
        (Some(x), Some(y)) => x == y,
        _ => get(a, "name").is_some() && get(a, "name") == get(b, "name"),
    }
}

fn write_aot<T, F>(doc: &mut DocumentMut, key: &str, items: &[T], known: &[&str], to_table: F)
where
    F: Fn(&T) -> Table,
{
    if items.is_empty() {
        doc.remove(key);
        return;
    }
    let old: Vec<Table> = doc
        .get(key)
        .and_then(|i| i.as_array_of_tables())
        .map(|a| a.iter().cloned().collect())
        .unwrap_or_default();
    let mut aot = ArrayOfTables::new();
    for item in items {
        let mut t = to_table(item);
        if let Some(prev) = old.iter().find(|o| same_entry(o, &t)) {
            for (k, v) in prev.iter() {
                if !known.contains(&k) && !t.contains_key(k) {
                    t.insert(k, v.clone());
                }
            }
        }
        aot.push(t);
    }
    doc.insert(key, Item::ArrayOfTables(aot));
}

fn string_array(values: &[String]) -> Array {
    let mut arr = Array::new();
    for v in values {
        arr.push(v.as_str());
    }
    arr
}

fn host_to_table(h: &HostEntry) -> Table {
    let mut t = Table::new();
    t.insert("name", value(h.name.as_str()));
    t.insert("ssh_host", value(h.ssh_host.as_str()));
    t.insert(
        "shell",
        value(match h.shell {
            super::schema::ShellType::Sh => "sh",
            super::schema::ShellType::PowerShell => "powershell",
            super::schema::ShellType::Cmd => "cmd",
        }),
    );
    if !h.groups.is_empty() {
        t.insert("groups", value(Value::Array(string_array(&h.groups))));
    }
    if let Some(pj) = &h.proxy_jump {
        if !pj.is_empty() {
            t.insert("proxy_jump", value(pj.as_str()));
        }
    }
    t
}

fn check_to_table(c: &CheckEntry) -> Table {
    let mut t = Table::new();
    if let Some(n) = &c.name {
        t.insert("name", value(n.as_str()));
    }
    if !c.id.is_empty() {
        t.insert("id", value(c.id.as_str()));
    }
    t.insert("enabled", value(Value::Array(string_array(&c.enabled))));
    if !c.path.is_empty() {
        let mut aot = ArrayOfTables::new();
        for p in &c.path {
            let mut pt = Table::new();
            pt.insert("path", value(p.path.as_str()));
            pt.insert("label", value(p.label.as_str()));
            aot.push(pt);
        }
        t.insert("path", Item::ArrayOfTables(aot));
    }
    t
}

fn sync_to_table(s: &SyncEntry) -> Table {
    let mut t = Table::new();
    if let Some(n) = &s.name {
        t.insert("name", value(n.as_str()));
    }
    if !s.id.is_empty() {
        t.insert("id", value(s.id.as_str()));
    }
    t.insert("paths", value(Value::Array(string_array(&s.paths))));
    t.insert("recursive", value(s.recursive));
    if let Some(m) = &s.mode {
        t.insert("mode", value(m.as_str()));
    }
    if let Some(pd) = s.propagate_deletes {
        t.insert("propagate_deletes", value(pd));
    }
    if let Some(src) = &s.source {
        t.insert("source", value(src.as_str()));
    }
    t
}

/// Inject helpful comments into TOML config for [settings], [[check]] and [[sync]] sections.
fn inject_config_comments(toml_str: &str) -> String {
    let settings_comment = "\
# [settings] Global settings:
#   state_dir = \"/custom/path/to/state\"  # Custom DB storage location
#                                          # Default: $XDG_STATE_HOME/sshi, else ~/.local/state/sshi (Linux),
#                                          #   ~/Library/Application Support/sshi (macOS), %LOCALAPPDATA%/sshi (Windows)
";

    // The probe list comes from `DEFAULT_CHECK_ENABLED` so the template can't
    // drift from the catalog (B61).
    let probes: String = crate::config::schema::DEFAULT_CHECK_ENABLED
        .iter()
        .map(|(key, desc)| format!("#       {:<21}# {desc}\n", format!("\"{key}\",")))
        .collect();
    let check_comment = format!(
        "\
# [[check]] Check tasks (selected with `check -n <name>`, or default when omitted)
#
# Fields:
#   name = \"default\"         # Selection identifier and TUI label (optional)
#   enabled = [              # Available metric probes:
{probes}#   ]
#
# [[check.path]] Custom path monitoring:
#   path  = \"/var/log\"       # Path to monitor
#   label = \"Logs\"           # Display label
#
# Examples:
# [[check]]
# name = \"default\"
# enabled = [\"online\", \"memory\", \"disk\", \"cpu_load\", \"ip_address\"]
#
# [[check]]
# name = \"web-logs\"
# enabled = [\"online\", \"disk\"]
# [[check.path]]
# path = \"/var/log/nginx\"
# label = \"Nginx Logs\"
"
    );

    let sync_comment = "\
# [[sync]] Sync tasks (selected with `sync -n <name>`, or default when omitted)
#
# Fields:
#   name = \"nginx-config\"              # Selection identifier and TUI label (optional)
#   paths = [\"/etc/timezone\"]          # File paths to sync (multiple allowed)
#   recursive = false                  # Recursive directory sync (default: false)
#   mode = \"0644\"                      # File permissions (optional)
#   propagate_deletes = false          # Sync deletions (optional, default: false)
#   source = \"myhost\"                  # Fixed source host (optional, skips auto-selection)
#
# Example:
# [[sync]]
# name = \"nginx-config\"
# paths = [\"/etc/nginx/nginx.conf\", \"/etc/nginx/conf.d\"]
# recursive = true
# mode = \"0644\"
# propagate_deletes = true
# source = \"web-prod-1\"
";

    let mut result = String::new();
    let mut has_check = false;
    let mut has_sync = false;
    for line in toml_str.lines() {
        if line.trim() == "[settings]" {
            result.push_str(settings_comment);
        } else if line.trim() == "[[check]]" && !has_check {
            result.push_str(&check_comment);
            has_check = true;
        } else if line.trim() == "[[sync]]" && !has_sync {
            result.push_str(sync_comment);
            has_sync = true;
        }
        result.push_str(line);
        result.push('\n');
    }

    // Append comment blocks for sections that are empty / absent in the TOML
    if !has_check {
        result.push('\n');
        result.push_str(&check_comment);
    }
    if !has_sync {
        result.push('\n');
        result.push_str(sync_comment);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // Returns TempPath (not NamedTempFile) so the file handle is closed before
    // callers invoke save(), which needs an atomic rename on Windows. Windows
    // rejects renaming over an open file with "Access denied".
    fn write_tmp(content: &str) -> tempfile::TempPath {
        let mut f = tempfile::Builder::new()
            .prefix("sshi-cfg-test-")
            .suffix(".toml")
            .tempfile()
            .unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f.flush().unwrap();
        f.into_temp_path()
    }

    #[test]
    fn bom_is_stripped_on_load() {
        let with_bom = "\u{feff}[settings]\ndefault_timeout = 7\n".to_string();
        let f = write_tmp(&with_bom);
        let cfg = load(Some(&*f)).unwrap().unwrap();
        assert_eq!(cfg.settings.default_timeout, 7);
    }

    #[test]
    fn tilde_resolution() {
        let p = std::path::Path::new("~/foo/bar.toml");
        let resolved = resolve_path(Some(p)).unwrap();
        let home = dirs::home_dir().unwrap();
        assert_eq!(resolved, home.join("foo/bar.toml"));
    }

    /// T-WB-2: unknown top-level key inside [settings] survives a save round-trip.
    #[test]
    fn t_wb_2_preserve_unknown_key() {
        let original = "\
[settings]
default_timeout = 30
unknown_future_option = true
";
        let f = write_tmp(original);
        let cfg = load(Some(&*f)).unwrap().unwrap();
        save(&cfg, Some(&*f)).unwrap();
        let after = std::fs::read_to_string(&*f).unwrap();
        assert!(
            after.contains("unknown_future_option = true"),
            "unknown key dropped:\n{after}"
        );
    }

    /// B43: inline `settings = {…}` must save (was a panic), and reload.
    #[test]
    fn b43_inline_settings_table_saves() {
        let f = write_tmp("settings = { default_timeout = 9, custom = 1 }\n");
        let cfg = load(Some(&*f)).unwrap().unwrap();
        save(&cfg, Some(&*f)).unwrap();
        let after = std::fs::read_to_string(&*f).unwrap();
        assert!(after.contains("custom = 1"), "{after}");
        let again = load(Some(&*f)).unwrap().unwrap();
        assert_eq!(again.settings.default_timeout, 9);
    }

    /// B43: saving through a symlinked config updates the target and keeps the link.
    #[cfg(unix)]
    #[test]
    fn b43_save_writes_through_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real.toml");
        let link = dir.path().join("link.toml");
        std::fs::write(&real, "[settings]\ndefault_timeout = 3\n").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let mut cfg = load(Some(&link)).unwrap().unwrap();
        cfg.settings.default_timeout = 11;
        save(&cfg, Some(&link)).unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        let real_after = std::fs::read_to_string(&real).unwrap();
        assert!(real_after.contains("default_timeout = 11"), "{real_after}");
    }

    /// B43: unknown per-entry keys survive a save; removed known keys stay removed.
    #[test]
    fn b43_unknown_entry_keys_preserved() {
        let f = write_tmp(
            "[[host]]\nname = \"h1\"\nssh_host = \"h1\"\nshell = \"sh\"\nproxy_jump = \"b\"\nnote = \"keep me\"\n\n\
             [[sync]]\nname = \"t\"\npaths = [\"~/x\"]\nfuture_key = 7\n",
        );
        let mut cfg = load(Some(&*f)).unwrap().unwrap();
        let mut h = (*cfg.host[0]).clone();
        h.proxy_jump = None;
        cfg.host[0] = Arc::new(h);
        save(&cfg, Some(&*f)).unwrap();
        let after = std::fs::read_to_string(&*f).unwrap();
        assert!(after.contains("note = \"keep me\""), "{after}");
        assert!(after.contains("future_key = 7"), "{after}");
        assert!(!after.contains("proxy_jump"), "{after}");
    }

    /// Regression: editing a Host/Check/Sync field must persist across a
    /// reload. Pre-fix `apply_config_to_doc` only wrote [settings] back, so
    /// any change to host.groups / check.enabled / sync.paths was silently
    /// dropped — the file on disk was the original.
    #[test]
    fn host_check_sync_edits_round_trip_through_save() {
        use super::super::schema::{CheckEntry, CheckPath, HostEntry, ShellType, SyncEntry};
        let original = "\
[settings]
default_timeout = 30

[[host]]
name = \"h1\"
ssh_host = \"1.2.3.4\"
shell = \"sh\"
groups = [\"old\"]

[[check]]
enabled = [\"online\"]
enable_hosts = true
enable_all = true

[[sync]]
paths = [\"/etc\"]
enable_hosts = true
enable_all = true
recursive = false
";
        let f = write_tmp(original);
        // Mutate one field of each kind through the same path the TUI uses.
        let mut cfg = load(Some(&*f)).unwrap().unwrap();
        Arc::make_mut(&mut cfg.host[0]).groups = vec!["new1".into(), "new2".into()];
        Arc::make_mut(&mut cfg.host[0]).proxy_jump = Some("bastion".into());
        cfg.check[0].enabled = vec!["online".into(), "cpu_load".into()];
        cfg.check[0].path = vec![CheckPath {
            path: "/var/log".into(),
            label: "Logs".into(),
        }];
        cfg.sync[0].paths = vec!["/srv".into(), "/opt".into()];
        cfg.sync[0].propagate_deletes = Some(true);

        save(&cfg, Some(&*f)).unwrap();
        let reloaded = load(Some(&*f)).unwrap().unwrap();

        assert_eq!(reloaded.host[0].groups, vec!["new1", "new2"]);
        assert_eq!(reloaded.host[0].proxy_jump.as_deref(), Some("bastion"));
        assert_eq!(reloaded.check[0].enabled, vec!["online", "cpu_load"]);
        assert_eq!(reloaded.check[0].path.len(), 1);
        assert_eq!(reloaded.check[0].path[0].label, "Logs");
        assert_eq!(reloaded.sync[0].paths, vec!["/srv", "/opt"]);
        assert_eq!(reloaded.sync[0].propagate_deletes, Some(true));

        // Settings still untouched.
        assert_eq!(reloaded.settings.default_timeout, 30);

        // Silence unused imports if the schema types change.
        let _ = (
            HostEntry {
                name: String::new(),
                ssh_host: String::new(),
                shell: ShellType::Sh,
                groups: vec![],
                proxy_jump: None,
            },
            CheckEntry {
                name: None,
                id: String::new(),
                enabled: vec![],
                path: vec![],
            },
            SyncEntry {
                name: None,
                id: String::new(),
                paths: vec![],
                recursive: false,
                mode: None,
                propagate_deletes: None,
                source: None,
            },
        );
    }

    /// T-WB-3: an inline comment on a scalar key survives an edit to that scalar.
    #[test]
    fn t_wb_3_inline_comment_survives_scalar_edit() {
        let original = "\
[settings]
max_concurrency = 10  # max 50
";
        let f = write_tmp(original);
        let mut cfg = load(Some(&*f)).unwrap().unwrap();
        cfg.settings.max_concurrency = 20;
        save(&cfg, Some(&*f)).unwrap();
        let after = std::fs::read_to_string(&*f).unwrap();
        assert!(
            after.contains("max_concurrency = 20"),
            "value not updated:\n{after}"
        );
        assert!(
            after.contains("# max 50"),
            "inline comment dropped:\n{after}"
        );
    }

    /// B16: template comments must only mention fields that actually exist in
    /// the schema (CheckEntry, CheckPath, SyncEntry, Settings).
    #[test]
    fn b16_template_mentions_only_existing_schema_fields() {
        use crate::config::schema::{CheckEntry, CheckPath, Settings, SyncEntry};
        use std::collections::HashSet;

        let template = inject_config_comments("");

        // Removed legacy fields must not be present in the template.
        assert!(
            !template.contains("enable_hosts"),
            "template contains enable_hosts"
        );
        assert!(
            !template.contains("enable_all"),
            "template contains enable_all"
        );

        // Collect all valid field names for each section.
        let check_sample = CheckEntry {
            name: Some("test".into()),
            id: "123".into(),
            enabled: vec!["online".into()],
            path: vec![CheckPath {
                path: "/var/log".into(),
                label: "logs".into(),
            }],
        };
        let check_json = serde_json::to_value(&check_sample).unwrap();
        let mut check_valid_keys: HashSet<String> =
            check_json.as_object().unwrap().keys().cloned().collect();
        let check_path_json = serde_json::to_value(&check_sample.path[0]).unwrap();
        check_valid_keys.extend(check_path_json.as_object().unwrap().keys().cloned());

        let sync_sample = SyncEntry {
            name: Some("test".into()),
            id: "123".into(),
            paths: vec!["/tmp".into()],
            recursive: true,
            mode: Some("0644".into()),
            propagate_deletes: Some(false),
            source: Some("host1".into()),
        };
        let sync_json = serde_json::to_value(&sync_sample).unwrap();
        let sync_valid_keys: HashSet<String> =
            sync_json.as_object().unwrap().keys().cloned().collect();

        let settings_sample = Settings {
            state_dir: Some(std::path::PathBuf::from("/custom/path")),
            default_output_format: Some("json".into()),
            ..Default::default()
        };
        let settings_json = serde_json::to_value(&settings_sample).unwrap();
        let settings_valid_keys: HashSet<String> =
            settings_json.as_object().unwrap().keys().cloned().collect();

        enum Section {
            None,
            Settings,
            Check,
            Sync,
        }
        let mut current_section = Section::None;

        for line in template.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("# [settings]") {
                current_section = Section::Settings;
                continue;
            } else if trimmed.starts_with("# [[check]]") {
                current_section = Section::Check;
                continue;
            } else if trimmed.starts_with("# [[sync]]") {
                current_section = Section::Sync;
                continue;
            }

            if let Some(comment_body) = trimmed.strip_prefix('#') {
                let code_part = comment_body.split('#').next().unwrap_or("").trim();
                if let Some((key, _val)) = code_part.split_once('=') {
                    let key = key.trim();
                    if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        match current_section {
                            Section::Settings => {
                                assert!(
                                    settings_valid_keys.contains(key),
                                    "Field '{key}' in settings template does not exist in Settings schema"
                                );
                            }
                            Section::Check => {
                                assert!(
                                    check_valid_keys.contains(key),
                                    "Field '{key}' in check template does not exist in CheckEntry/CheckPath schema"
                                );
                            }
                            Section::Sync => {
                                assert!(
                                    sync_valid_keys.contains(key),
                                    "Field '{key}' in sync template does not exist in SyncEntry schema"
                                );
                            }
                            Section::None => {}
                        }
                    }
                }
            }
        }

        // Verify the example blocks in comments parse cleanly.
        let check_example = r#"
name = "default"
enabled = ["online", "memory", "disk", "cpu_load", "ip_address"]
"#;
        let parsed_check: CheckEntry = toml::from_str(check_example).unwrap();
        assert_eq!(parsed_check.name.as_deref(), Some("default"));

        let sync_example = r#"
name = "nginx-config"
paths = ["/etc/nginx/nginx.conf", "/etc/nginx/conf.d"]
recursive = true
mode = "0644"
propagate_deletes = true
source = "web-prod-1"
"#;
        let parsed_sync: SyncEntry = toml::from_str(sync_example).unwrap();
        assert_eq!(parsed_sync.name.as_deref(), Some("nginx-config"));
        assert!(parsed_sync.recursive);
    }
}
