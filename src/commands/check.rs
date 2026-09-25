//! Collect and store system metrics from remote hosts.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use anyhow::Result;

use crate::config::schema::HostEntry;
use crate::host::pool::SshPool;
use crate::metrics::collector;
use crate::output::printer;
use crate::output::report::maybe_write_report;
use crate::output::summary::Summary;
use crate::state::retention;

use super::fanout::FanOut;
use super::report::{
    default_printer_sink, CheckHostResult, CheckReport, CommandReport, HostOutcome, HostStatus,
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
) -> Result<CheckReport> {
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
        return Ok(CheckReport {
            executed_at,
            enabled_metrics,
            targets,
            hosts: Vec::new(),
        });
    }

    let (pool, _connected) = SshPool::setup(
        &hosts,
        ctx.timeout,
        ctx.concurrency(),
        ctx.per_host_concurrency(),
        ctx.auth_sender.clone(),
    )
    .await?;

    let mut results: Vec<CheckHostResult> = Vec::new();

    // Pending per-host DB writes collected as handles resolve. Executing
    // them inside a single transaction at the end turns N auto-commit
    // fsyncs into one (audit §3.5 MED, B37).
    #[allow(clippy::type_complexity)]
    let mut pending_writes: Vec<(
        String,
        i64,
        i64,
        String,
        bool,
        HostStatus,
        i64,
        Option<String>,
    )> = Vec::new();

    // Unreachable hosts (from pool setup): report immediately, queue offline rows.
    for (name, err) in pool.failed_hosts() {
        let detail = format!("unreachable — {}", err);
        if let Some(p) = progress {
            p.host_completed(&name, HostStatus::Unreachable, &detail, 0);
        }
        pending_writes.push((
            name.clone(),
            now_ts,
            0,
            "{}".to_string(),
            false,
            HostStatus::Unreachable,
            0,
            Some(detail.clone()),
        ));
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

    let mut fan = FanOut::new(&pool);
    for host in &reachable {
        let (enabled, check_paths) = match host_configs.get(&host.name) {
            Some(config) => config.clone(),
            None => continue,
        };
        let task_host = Arc::clone(host);
        let timeout = ctx.timeout;
        let sessions = pool.session_pool.clone();
        fan.spawn(host, progress, async move {
            collector::collect_pooled(&task_host, &enabled, &check_paths, timeout, sessions).await
        });
    }

    while let Some(done) = fan.next().await {
        let (host, result, elapsed) = done?;
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

                if let Some(p) = progress {
                    p.host_completed(&host.name, status, &detail, ms);
                }

                pending_writes.push((
                    host.name.clone(),
                    now,
                    online_int,
                    json_str,
                    online_int == 1,
                    status,
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
                    HostStatus::Error,
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

    if let Err(e) = ctx.db
        .transaction(move |tx| -> Result<()> {
            for (host, now, online_int, json_str, online_set_to_now, status, ms, note) in
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
                if let Err(e) = crate::commands::report::record_operation_log_tx(
                    tx,
                    *now,
                    "check",
                    host,
                    "metrics_batch",
                    *status,
                    *ms,
                    note.as_deref(),
                    None,
                ) {
                    tracing::warn!(error = %e, host = %host, "failed to record operation_log entry");
                }
            }
            Ok(())
        })
        .await
    {
        tracing::warn!(error = %e, "failed to record check snapshots and operation_log");
    }

    pool.shutdown().await;
    retention::cleanup(&ctx.db, ctx.config.settings.data_retention_days).await?;

    Ok(CheckReport {
        executed_at,
        enabled_metrics,
        targets,
        hosts: results,
    })
}

/// Thin CLI wrapper: invokes `check_core` with a printer-driven
/// `ProgressSink`, prints the run summary, and writes `--out` reports.
pub async fn run(
    ctx: &Context,
    names: &[String],
    dry_run: bool,
    output: &crate::cli::OutputArgs,
) -> Result<HostOutcome> {
    ctx.ensure_check_names(names)?;
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
        return Ok(HostOutcome::default());
    }

    let host_configs_empty_hint = {
        // We need to detect the "no entries" case before invoking core to
        // print the same hint as before. Cheap: re-derive host_configs.
        let hosts = ctx.resolve_hosts()?;
        build_host_check_configs(ctx, &hosts, names).is_empty()
    };
    if host_configs_empty_hint {
        println!("No check entries matched the current filter. Add [[check]] to config.toml.");
        return Ok(HostOutcome::default());
    }

    let sink = default_printer_sink();
    let report = check_core(ctx, names, Some(&sink)).await?;

    // Build the legacy Summary from the typed CheckReport for stdout.
    let mut summary = Summary::default();
    for h in &report.hosts {
        crate::commands::report::update_summary(&mut summary, &h.host, h.status, &h.detail);
    }

    summary.print();

    let raw = CommandReport::from(report);
    maybe_write_report(
        &raw,
        &ctx.mode,
        output.out.as_deref(),
        "check",
        ctx.config.settings.default_output_format.as_deref(),
    )?;

    Ok(raw.host_outcome())
}

/// Build per-host check configuration from the `--name`-selected entries.
/// Target mode only selects hosts; every targeted host gets the same merged
/// metrics/paths from the named (or "default") [[check]] entries.
fn build_host_check_configs(
    ctx: &Context,
    hosts: &[Arc<HostEntry>],
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
