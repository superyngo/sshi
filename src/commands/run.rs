//! Run a shell command on remote hosts.

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result};

use crate::host::pool::SshPool;
use crate::host::shell;
use crate::output::printer;
use crate::output::report::maybe_write_report;
use crate::output::summary::Summary;

use super::report::{
    default_printer_sink, CommandReport, HostOutcome, HostStatus, ProgressSink, RunHostResult,
    RunReport,
};
use super::Context;

/// Pure command core: resolves targets, spawns per-host `run` tasks, writes
/// to the operation log, and returns a typed `RunReport`.
///
/// No `println!`, no `output::printer` calls — those belong to the CLI
/// wrapper. Per-host events are surfaced via the optional `ProgressSink`.
pub async fn run_core(
    ctx: &Context,
    command: &str,
    sudo: bool,
    progress: Option<&dyn ProgressSink>,
) -> Result<CommandReport> {
    let hosts = ctx.resolve_hosts()?;
    let executed_at = chrono::Utc::now().to_rfc3339();
    let targets: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();

    let (pool, _connected) = SshPool::setup(
        &hosts,
        ctx.timeout,
        ctx.concurrency(),
        ctx.per_host_concurrency(),
        ctx.auth_sender.clone(),
    )
    .await?;

    let mut host_results: Vec<RunHostResult> = Vec::new();

    // Report unreachable hosts.
    for (name, err) in pool.failed_hosts() {
        let detail = format!("unreachable — {}", err);
        if let Some(p) = progress {
            p.host_completed(&name, HostStatus::Unreachable, &detail, 0);
        }
        host_results.push(RunHostResult {
            host: name.clone(),
            status: HostStatus::Unreachable,
            duration_ms: None,
            detail,
            stdout: String::new(),
            stderr: err.clone(),
        });
    }

    let reachable = pool.filter_reachable(&hosts);

    let mut set = tokio::task::JoinSet::new();
    for host in &reachable {
        let host = Arc::clone(host);
        let cmd = if sudo {
            match shell::sudo_wrap(host.shell, command) {
                Ok(cmd) => cmd,
                // Windows hosts: refused up front rather than reporting a
                // success whose elevated exit status was never seen (B31).
                Err(e) => {
                    let detail = format!("error — {e}");
                    if let Some(p) = progress {
                        p.host_completed(&host.name, HostStatus::Error, &detail, 0);
                    }
                    crate::commands::report::record_operation_log(
                        &ctx.db,
                        chrono::Utc::now().timestamp(),
                        "run",
                        &host.name,
                        command,
                        HostStatus::Error,
                        0,
                        Some(&e.to_string()),
                        None,
                    )
                    .await;
                    host_results.push(RunHostResult {
                        host: host.name.clone(),
                        status: HostStatus::Error,
                        duration_ms: None,
                        detail,
                        stdout: String::new(),
                        stderr: e.to_string(),
                    });
                    continue;
                }
            }
        } else {
            command.to_string()
        };
        let timeout = ctx.timeout;
        let sessions = pool.session_pool.clone();
        let global_sem = pool.limiter.global_semaphore();
        if let Some(p) = progress {
            p.host_started(&host.name);
        }

        set.spawn(async move {
            let _permit = global_sem.acquire_owned().await.unwrap();
            let start = Instant::now();
            let result = sessions.exec(&host.ssh_host, &cmd, timeout).await;
            let elapsed = start.elapsed();
            (host, result, elapsed)
        });
    }

    while let Some(joined) = set.join_next().await {
        let (host, result, elapsed) = joined.context("task panic")?;
        let ms = elapsed.as_millis() as u64;
        let now = chrono::Utc::now().timestamp();

        match result {
            Ok(exec_output) => {
                let (status, detail) = if exec_output.success {
                    let first_line = exec_output.stdout.lines().next().unwrap_or("").to_string();
                    let detail = if first_line.is_empty() {
                        format!("ok ({:.1}s)", elapsed.as_secs_f64())
                    } else {
                        format!("ok ({:.1}s) — {}", elapsed.as_secs_f64(), first_line)
                    };
                    (HostStatus::Online, detail)
                } else {
                    let msg = exec_output.stderr.trim().to_string();
                    (
                        HostStatus::Error,
                        format!("error ({:.1}s) — {}", elapsed.as_secs_f64(), msg),
                    )
                };

                if let Some(p) = progress {
                    p.host_completed(&host.name, status, &detail, ms);
                }

                let note = if matches!(status, HostStatus::Error) {
                    Some(exec_output.stderr.trim())
                } else {
                    None
                };
                let stdout_preview = exec_output
                    .stdout
                    .lines()
                    .next()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| l.trim_end());

                crate::commands::report::record_operation_log(
                    &ctx.db,
                    now,
                    "run",
                    &host.name,
                    command,
                    status,
                    elapsed.as_millis() as i64,
                    note,
                    stdout_preview,
                )
                .await;

                host_results.push(RunHostResult {
                    host: host.name.clone(),
                    status,
                    duration_ms: Some(ms),
                    detail,
                    stdout: exec_output.stdout,
                    stderr: exec_output.stderr,
                });
            }
            Err(e) => {
                let detail = format!("error ({:.1}s) — {}", elapsed.as_secs_f64(), e);
                if let Some(p) = progress {
                    p.host_completed(&host.name, HostStatus::Error, &detail, ms);
                }
                crate::commands::report::record_operation_log(
                    &ctx.db,
                    now,
                    "run",
                    &host.name,
                    command,
                    HostStatus::Error,
                    elapsed.as_millis() as i64,
                    Some(&e.to_string()),
                    None,
                )
                .await;
                host_results.push(RunHostResult {
                    host: host.name.clone(),
                    status: HostStatus::Error,
                    duration_ms: Some(ms),
                    detail,
                    stdout: String::new(),
                    stderr: e.to_string(),
                });
            }
        }
    }

    pool.shutdown().await;

    Ok(CommandReport::Run(RunReport {
        executed_at,
        command: command.to_string(),
        targets,
        hosts: host_results,
    }))
}

/// Thin CLI wrapper: invokes `run_core` with a printer-driven `ProgressSink`,
/// prints the run summary, and writes `--out` reports.
pub async fn run(
    ctx: &Context,
    command: &str,
    sudo: bool,
    dry_run: bool,
    output: &crate::cli::OutputArgs,
) -> Result<HostOutcome> {
    if dry_run {
        println!("[dry-run] Command: {}", command);
        let hosts = ctx.resolve_hosts()?;
        for host in &hosts {
            // Preview the command as each host's shell would receive it (B31).
            match sudo.then(|| shell::sudo_wrap(host.shell, command)) {
                None => printer::print_host_line(&host.name, "ok", "would execute"),
                Some(Ok(wrapped)) => {
                    printer::print_host_line(&host.name, "ok", &format!("would execute: {wrapped}"))
                }
                Some(Err(e)) => printer::print_host_line(&host.name, "error", &e.to_string()),
            }
        }
        return Ok(HostOutcome::default());
    }

    let sink = default_printer_sink();
    let raw = run_core(ctx, command, sudo, Some(&sink)).await?;
    let CommandReport::Run(report) = &raw else {
        unreachable!("run_core always returns CommandReport::Run")
    };

    let mut summary = Summary::default();
    for h in &report.hosts {
        crate::commands::report::update_summary(&mut summary, &h.host, h.status, &h.detail);
    }

    summary.print();

    maybe_write_report(
        &raw,
        &ctx.mode,
        output.out.as_deref(),
        "run",
        ctx.config.settings.default_output_format.as_deref(),
    )?;

    Ok(raw.host_outcome())
}
