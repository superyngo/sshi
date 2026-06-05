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
        // Viewer: list every configured entry (selection now happens by --name).
        checks: ctx.config.check.clone(),
        syncs: ctx.config.sync.clone(),
    })
}

pub async fn run(ctx: &Context, output: &crate::cli::OutputArgs) -> Result<()> {
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
    println!("\n── Check Entries ({}) ──", checks.len());
    if checks.is_empty() {
        println!("  (none)");
    } else {
        for (i, entry) in checks.iter().enumerate() {
            println!("  [{}] name: {}", i + 1, entry_name(&entry.name));
            if !entry.enabled.is_empty() {
                println!("      enabled: {}", entry.enabled.join(", "));
            }
            for p in &entry.path {
                println!("      path: {} ({})", p.path, p.label);
            }
        }
    }

    // ── Sync ──
    println!("\n── Sync Entries ({}) ──", syncs.len());
    if syncs.is_empty() {
        println!("  (none)");
    } else {
        for (i, entry) in syncs.iter().enumerate() {
            println!(
                "  [{}] name: {}  paths: {}",
                i + 1,
                entry_name(&entry.name),
                entry.paths.join(", ")
            );
        }
    }

    if let Some(ref out) = output.out {
        use crate::commands::report::{CommandReport, ListHostResult, ListReport};

        let list_hosts: Vec<ListHostResult> = hosts
            .iter()
            .map(|h| ListHostResult {
                host: h.name.clone(),
                ssh_host: h.ssh_host.clone(),
                shell: h.shell.to_string(),
                groups: h.groups.clone(),
            })
            .collect();

        let report = CommandReport::List(ListReport {
            executed_at: chrono::Local::now().to_rfc3339(),
            targets: ctx
                .resolve_hosts()?
                .iter()
                .map(|h| h.name.clone())
                .collect(),
            hosts: list_hosts,
            checks: checks.clone(),
            syncs: syncs.clone(),
        });

        let op_report = crate::output::report::to_operation_report(&report, &ctx.mode);
        let path = crate::output::report::write_report(
            &op_report,
            out,
            "list",
            ctx.config.settings.default_output_format.as_deref(),
        )?;
        println!("Report written to {}", path);
    }

    Ok(())
}

fn entry_name(name: &Option<String>) -> String {
    match name {
        Some(n) if !n.is_empty() => n.clone(),
        _ => "(unnamed)".to_string(),
    }
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
        // checks: default config has one "default" entry; syncs still empty
        assert_eq!(data.checks.len(), 1);
        assert!(data.syncs.is_empty());
    }

    #[tokio::test]
    async fn test_list_run_with_output() {
        let ctx = make_ctx(&[("h1", &["web"]), ("h2", &[])]);
        let temp_dir = tempfile::TempDir::new().unwrap();
        let out_path = temp_dir.path().join("list_report.json");
        let output = crate::cli::OutputArgs {
            out: Some(out_path.to_str().unwrap().to_string()),
        };

        run(&ctx, &output).await.unwrap();

        assert!(out_path.exists());
        let content = std::fs::read_to_string(&out_path).unwrap();
        let v: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(v["command"], "list");
        assert_eq!(v["results"][0]["host"], "h1");
        assert_eq!(v["results"][0]["ssh_host"], "h1");
        assert_eq!(v["results"][0]["shell"], "sh");
        assert_eq!(v["results"][1]["host"], "h2");
    }
}
