//! Collect and store system metrics from remote hosts.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use anyhow::Result;

use crate::config::schema::HostEntry;
use crate::host::pool::SshPool;
use crate::metrics::collector;
use crate::output::printer;
use crate::output::report::maybe_write_report;
use crate::output::summary::Summary;
use crate::state::retention;

use super::report::{
    printer_sink_with_partial, CheckHostResult, CheckReport, CommandReport, HostStatus,
    ProgressSink,
};
use super::Context;

/// Per-host check configuration: (enabled_metrics, check_paths).
type HostCheckConfig = (Vec<String>, Vec<(String, String)>);

/// Pure command core: resolves targets, dispatches per-host metric collection,
/// writes snapshot rows to the DB, and returns a typed `CheckReport`.
///
/// No `println!`, no `output::printer` calls — those belong to the CLI
/// wrapper. Per-host events are surfaced via the optional `ProgressSink`
/// callbacks (called for both unreachable hosts from `SshPool::setup`
/// and reachable hosts after their `collect_pooled` future resolves).
///
/// DB writes (snapshot inserts, host_last_seen upserts) and retention
/// cleanup stay here because the TUI also needs them.
pub async fn check_core(
    ctx: &Context,
    names: &[String],
    progress: Option<&dyn ProgressSink>,
) -> Result<CommandReport> {
    let hosts = ctx.resolve_hosts()?;
    let host_configs = build_host_check_configs(ctx, &hosts, names);

    let run_start = chrono::Utc::now();
    let now_ts = run_start.timestamp();
    let executed_at = run_start.to_rfc3339();
    let targets: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();
    let enabled_metrics: Vec<String> = host_configs
        .values()
        .flat_map(|(enabled, _)| enabled.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();

    if host_configs.is_empty() {
        return Ok(CommandReport::Check(CheckReport {
            executed_at,
            enabled_metrics,
            targets,
            hosts: Vec::new(),
        }));
    }

    let (pool, _connected) = SshPool::setup(
        &hosts,
        ctx.timeout,
        ctx.concurrency(),
        ctx.per_host_concurrency(),
    )
    .await?;

    let mut results: Vec<CheckHostResult> = Vec::new();

    // Unreachable hosts (from pool setup): report immediately, write offline rows.
    for (name, err) in pool.failed_hosts() {
        ctx.db.execute(
            "INSERT INTO check_snapshots (host, collected_at, online, raw_json) VALUES (?1, ?2, 0, '{}')",
            vec![
                crate::state::db::boxed_param(name.clone()),
                crate::state::db::boxed_param(now_ts),
            ],
        )
        .await?;
        ctx.db
            .execute(
                "INSERT INTO host_last_seen (host, last_seen, last_online) VALUES (?1, ?2, 0) \
             ON CONFLICT(host) DO UPDATE SET last_seen = ?2",
                vec![
                    crate::state::db::boxed_param(name.clone()),
                    crate::state::db::boxed_param(now_ts),
                ],
            )
            .await?;
        let detail = format!("unreachable — {}", err);
        if let Some(p) = progress {
            p.host_completed(&name, HostStatus::Unreachable, &detail, 0);
        }
        results.push(CheckHostResult {
            host: name.clone(),
            status: HostStatus::Unreachable,
            duration_ms: None,
            detail,
            metrics_succeeded: 0,
            metrics_failed: 0,
            data: serde_json::json!({}),
            raw_stdout: String::new(),
            raw_stderr: String::new(),
        });
    }

    let reachable = pool.filter_reachable(&hosts);

    let mut handles = Vec::new();
    for host in &reachable {
        let (enabled, check_paths) = match host_configs.get(&host.name) {
            Some(config) => config.clone(),
            None => continue,
        };
        if let Some(p) = progress {
            p.host_started(&host.name);
        }
        let host = (*host).clone();
        let timeout = ctx.timeout;
        let sessions = pool.session_pool.clone();
        let global_sem = pool.limiter.global_semaphore();

        handles.push(tokio::spawn(async move {
            let _permit = global_sem.acquire_owned().await.unwrap();
            let start = Instant::now();
            let result =
                collector::collect_pooled(&host, &enabled, &check_paths, timeout, sessions).await;
            let elapsed = start.elapsed();
            (host, result, elapsed)
        }));
    }

    // Pending per-host DB writes collected as handles resolve. Executing
    // them inside a single transaction at the end turns N auto-commit
    // fsyncs into one (audit §3.5 MED).
    #[allow(clippy::type_complexity)]
    let mut pending_writes: Vec<(String, i64, i64, String, bool, String, i64, Option<String>)> =
        Vec::new();

    for handle in handles {
        let (host, result, elapsed) = handle.await?;
        let now = chrono::Utc::now().timestamp();
        let ms = elapsed.as_millis() as u64;

        match result {
            Ok(cr) => {
                let json_str = serde_json::to_string(&cr.data)?;
                let total = cr.succeeded + cr.failed;

                let (status, online_int, detail) = if cr.succeeded == 0 {
                    let err_detail = cr
                        .errors
                        .first()
                        .map(|s| s.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    (
                        HostStatus::Offline,
                        0_i64,
                        format!("failed ({:.1}s) — {}", elapsed.as_secs_f64(), err_detail),
                    )
                } else if cr.failed > 0 {
                    let warn_detail = cr
                        .errors
                        .first()
                        .map(|s| s.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    (
                        HostStatus::Partial,
                        1,
                        format!(
                            "partial ({}/{} metrics, {:.1}s) — warn: {}",
                            cr.succeeded,
                            total,
                            elapsed.as_secs_f64(),
                            warn_detail
                        ),
                    )
                } else {
                    (
                        HostStatus::Online,
                        1,
                        format!(
                            "collected ({} metrics, {:.1}s)",
                            cr.succeeded,
                            elapsed.as_secs_f64()
                        ),
                    )
                };

                let status_str = if matches!(status, HostStatus::Online | HostStatus::Partial) {
                    "ok"
                } else {
                    "error"
                };

                if let Some(p) = progress {
                    p.host_completed(&host.name, status, &detail, ms);
                }

                pending_writes.push((
                    host.name.clone(),
                    now,
                    online_int,
                    json_str,
                    online_int == 1,
                    status_str.to_string(),
                    ms as i64,
                    None,
                ));

                results.push(CheckHostResult {
                    host: host.name.clone(),
                    status,
                    duration_ms: Some(ms),
                    detail,
                    metrics_succeeded: cr.succeeded,
                    metrics_failed: cr.failed,
                    data: cr.data,
                    raw_stdout: cr.metrics_raw_stdout,
                    raw_stderr: cr.metrics_raw_stderr,
                });
            }
            Err(e) => {
                let detail = e.to_string();
                if let Some(p) = progress {
                    p.host_completed(&host.name, HostStatus::Error, &detail, ms);
                }

                pending_writes.push((
                    host.name.clone(),
                    now,
                    0,
                    "{}".to_string(),
                    false,
                    "error".to_string(),
                    ms as i64,
                    Some(detail.clone()),
                ));

                results.push(CheckHostResult {
                    host: host.name.clone(),
                    status: HostStatus::Error,
                    duration_ms: Some(ms),
                    detail,
                    metrics_succeeded: 0,
                    metrics_failed: 0,
                    data: serde_json::json!({}),
                    raw_stdout: String::new(),
                    raw_stderr: String::new(),
                });
            }
        }
    }

    ctx.db
        .transaction(move |tx| -> Result<()> {
            for (host, now, online_int, json_str, online_set_to_now, status_str, ms, note) in
                &pending_writes
            {
                tx.execute(
                    "INSERT INTO check_snapshots (host, collected_at, online, raw_json) \
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![host, now, online_int, json_str],
                )?;
                if *online_set_to_now {
                    tx.execute(
                        "INSERT INTO host_last_seen (host, last_seen, last_online) \
                         VALUES (?1, ?2, ?2) \
                         ON CONFLICT(host) DO UPDATE SET last_seen = ?2, last_online = ?2",
                        rusqlite::params![host, now],
                    )?;
                } else {
                    tx.execute(
                        "INSERT INTO host_last_seen (host, last_seen, last_online) \
                         VALUES (?1, ?2, 0) \
                         ON CONFLICT(host) DO UPDATE SET last_seen = ?2",
                        rusqlite::params![host, now],
                    )?;
                }
                if let Err(e) = tx.execute(
                    "INSERT INTO operation_log \
                     (timestamp, command, host, action, status, duration_ms, note) \
                     VALUES (?1, 'check', ?2, 'metrics_batch', ?3, ?4, ?5)",
                    rusqlite::params![now, host, status_str, ms, note],
                ) {
                    tracing::warn!(error = %e, "failed to record operation_log entry");
                }
            }
            Ok(())
        })
        .await?;

    pool.shutdown().await;
    retention::cleanup(&ctx.db, ctx.config.settings.data_retention_days).await?;

    Ok(CommandReport::Check(CheckReport {
        executed_at,
        enabled_metrics,
        targets,
        hosts: results,
    }))
}

/// Thin CLI wrapper: invokes `check_core` with a printer-driven
/// `ProgressSink`, prints the run summary, and writes `--out` reports.
pub async fn run(
    ctx: &Context,
    names: &[String],
    dry_run: bool,
    output: &crate::cli::OutputArgs,
) -> Result<()> {
    if dry_run {
        let hosts = ctx.resolve_hosts()?;
        let configs = build_host_check_configs(ctx, &hosts, names);
        for host in &hosts {
            match configs.get(&host.name) {
                Some((enabled, _paths)) if !enabled.is_empty() => {
                    printer::print_host_line(
                        &host.name,
                        "ok",
                        &format!("would collect: {}", enabled.join(", ")),
                    );
                }
                _ => printer::print_host_line(&host.name, "skip", "no checks apply"),
            }
        }
        return Ok(());
    }

    let host_configs_empty_hint = {
        // We need to detect the "no entries" case before invoking core to
        // print the same hint as before. Cheap: re-derive host_configs.
        let hosts = ctx.resolve_hosts()?;
        build_host_check_configs(ctx, &hosts, names).is_empty()
    };
    if host_configs_empty_hint {
        println!("No check entries matched the current filter. Add [[check]] to config.toml.");
        return Ok(());
    }

    let sink = printer_sink_with_partial();
    let raw = check_core(ctx, names, Some(&sink)).await?;
    let CommandReport::Check(report) = &raw else {
        unreachable!("check_core always returns CommandReport::Check")
    };

    // Build the legacy Summary from the typed CheckReport for stdout.
    let mut summary = Summary::default();
    for h in &report.hosts {
        match h.status {
            HostStatus::Online | HostStatus::Partial => summary.add_success(),
            HostStatus::Offline | HostStatus::Error | HostStatus::TimedOut => {
                summary.add_failure(&h.host, &h.detail);
            }
            HostStatus::Unreachable => {
                let err = h.detail.strip_prefix("unreachable — ").unwrap_or(&h.detail);
                summary.add_failure(&h.host, err);
            }
            HostStatus::Skipped => {
                summary.add_skip();
            }
        }
    }

    summary.print();

    maybe_write_report(
        &raw,
        &ctx.mode,
        output.out.as_deref(),
        "check",
        ctx.config.settings.default_output_format.as_deref(),
    )?;

    Ok(())
}

/// Build per-host check configuration from the `--name`-selected entries.
/// Target mode only selects hosts; every targeted host gets the same merged
/// metrics/paths from the named (or "default") [[check]] entries.
fn build_host_check_configs(
    ctx: &Context,
    hosts: &[&HostEntry],
    names: &[String],
) -> HashMap<String, HostCheckConfig> {
    let checks = ctx.resolve_checks(names);
    let mut enabled: Vec<String> = Vec::new();
    let mut check_paths: Vec<(String, String)> = Vec::new();
    for entry in &checks {
        for m in &entry.enabled {
            if !enabled.contains(m) {
                enabled.push(m.clone());
            }
        }
        for p in &entry.path {
            let key = (p.path.clone(), p.label.clone());
            if !check_paths.contains(&key) {
                check_paths.push(key);
            }
        }
    }
    let mut configs = HashMap::new();
    if !enabled.is_empty() || !check_paths.is_empty() {
        for host in hosts {
            configs.insert(host.name.clone(), (enabled.clone(), check_paths.clone()));
        }
    }
    configs
}
