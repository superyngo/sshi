use anyhow::Result;

use crate::config::schema::{CheckEntry, HostEntry, SyncEntry};

use super::Context;

/// Structured result of `list`, for both stdout printing and the TUI View tab.
#[derive(Default)]
pub struct ListData {
    pub hosts: Vec<HostEntry>,
    pub checks: Vec<CheckEntry>,
    pub syncs: Vec<SyncEntry>,
}

/// Resolve hosts/checks/syncs with no I/O side effects.
///
/// Fallible because `resolve_hosts` surfaces a diagnostic error when no hosts
/// match the filter; `run` propagates it to preserve CLI behavior, while the
/// TUI View tab calls `.unwrap_or_default()` at its own call site.
pub fn list_core(ctx: &Context) -> Result<ListData> {
    Ok(ListData {
        hosts: ctx.resolve_hosts()?.into_iter().cloned().collect(),
        checks: ctx.resolve_checks().into_iter().cloned().collect(),
        syncs: ctx.resolve_syncs().into_iter().cloned().collect(),
    })
}

pub async fn run(ctx: &Context) -> Result<()> {
    let ListData {
        hosts,
        checks,
        syncs,
    } = list_core(ctx)?;

    // ── Hosts ──
    println!("── Hosts ({}) ──", hosts.len());
    println!("  {:<16} {:<20} {:<12} Groups", "Name", "SSH Host", "Shell");
    println!("  {}", "-".repeat(64));
    for h in &hosts {
        let groups = if h.groups.is_empty() {
            "-".to_string()
        } else {
            h.groups.join(", ")
        };
        println!(
            "  {:<16} {:<20} {:<12} {}",
            h.name, h.ssh_host, h.shell, groups
        );
    }

    // ── Checks ──
    println!("\n── Applicable Checks ({}) ──", checks.len());
    if checks.is_empty() {
        println!("  (none)");
    } else {
        for (i, entry) in checks.iter().enumerate() {
            let scope = format_scope(&entry.groups, entry.enable_hosts, entry.enable_all);
            println!("  [{}] scope: {}", i + 1, scope);
            if !entry.enabled.is_empty() {
                println!("      enabled: {}", entry.enabled.join(", "));
            }
            for p in &entry.path {
                println!("      path: {} ({})", p.path, p.label);
            }
        }
    }

    // ── Sync ──
    println!("\n── Applicable Sync Entries ({}) ──", syncs.len());
    if syncs.is_empty() {
        println!("  (none)");
    } else {
        for (i, entry) in syncs.iter().enumerate() {
            let scope = format_scope(&entry.groups, entry.enable_hosts, entry.enable_all);
            println!(
                "  [{}] scope: {}  paths: {}",
                i + 1,
                scope,
                entry.paths.join(", ")
            );
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::TargetMode;
    use crate::config::schema::{AppConfig, HostEntry, ShellType};

    fn make_ctx(hosts: &[(&str, &[&str])]) -> Context {
        let mut config = AppConfig::default();
        for (name, groups) in hosts {
            config.host.push(HostEntry {
                name: name.to_string(),
                ssh_host: name.to_string(),
                groups: groups.iter().map(|g| g.to_string()).collect(),
                shell: ShellType::Sh,
                proxy_jump: None,
            });
        }
        let db = crate::state::db::open(None).unwrap();
        Context {
            config,
            config_path: None,
            db,
            timeout: 30,
            mode: TargetMode::All,
            serial: false,
            skip: vec![],
            verbose: false,
        }
    }

    #[test]
    fn list_core_returns_resolved_collections() {
        let ctx = make_ctx(&[("h1", &["web"]), ("h2", &[])]);
        let data = list_core(&ctx).unwrap();
        assert_eq!(data.hosts.len(), 2);
        // checks/syncs default-empty config → empty
        assert!(data.checks.is_empty());
        assert!(data.syncs.is_empty());
    }
}

fn format_scope(groups: &[String], enable_hosts: bool, enable_all: bool) -> String {
    let mut parts = Vec::new();
    if !groups.is_empty() {
        parts.push(format!("groups=[{}]", groups.join(", ")));
    }
    if !enable_hosts {
        parts.push("hosts=off".to_string());
    }
    if !enable_all {
        parts.push("all=off".to_string());
    }
    if parts.is_empty() {
        "global".to_string()
    } else {
        parts.join(" ")
    }
}
