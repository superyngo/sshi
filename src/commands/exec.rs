//! Execute scripts or commands on remote hosts with shell wrapping.

use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Result};

use crate::config::schema::ShellType;
use crate::host::pool::SshPool;
use crate::host::shell;
use crate::output::printer;
use crate::output::report::maybe_write_report;
use crate::output::summary::Summary;

use super::fanout::FanOut;
use super::report::{
    default_printer_sink, CommandReport, ExecHostResult, ExecReport, HostOutcome, HostStatus,
    ProgressSink,
};
use super::Context;

/// Resolve the compatible remote shell for a script from its file extension.
pub fn script_extension_to_shell(path: &Path) -> Option<ShellType> {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    match extension.as_str() {
        "sh" => Some(ShellType::Sh),
        "ps1" => Some(ShellType::PowerShell),
        "bat" | "cmd" => Some(ShellType::Cmd),
        _ => None,
    }
}

/// Pure command core: uploads and executes a script on each host, writes to
/// the operation log, and returns a typed `ExecReport`.
///
/// `dry_run` is handled by the CLI wrapper before calling this.
pub async fn exec_core(
    ctx: &Context,
    script: &str,
    sudo: bool,
    keep: bool,
    progress: Option<&dyn ProgressSink>,
) -> Result<ExecReport> {
    let script_path = crate::util::expand_tilde(Path::new(script));
    if !script_path.exists() {
        bail!("Script not found: {}", script);
    }

    let compatible_shell = script_extension_to_shell(&script_path);

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

    let mut host_results: Vec<ExecHostResult> = Vec::new();

    // Report unreachable hosts.
    for (name, err) in pool.failed_hosts() {
        let detail = format!("unreachable — {}", err);
        if let Some(p) = progress {
            p.host_completed(&name, HostStatus::Unreachable, &detail, 0);
        }
        host_results.push(ExecHostResult {
            host: name.clone(),
            status: HostStatus::Unreachable,
            duration_ms: None,
            detail,
            stdout: String::new(),
            stderr: err.clone(),
        });
    }

    let reachable = pool.filter_reachable(&hosts);

    let mut fan = FanOut::new(&pool);
    for host in &reachable {
        // Check shell compatibility — skipped hosts are reported immediately.
        if let Some(required) = compatible_shell {
            if required != host.shell {
                let detail = format!(
                    "skipped (shell mismatch: need {:?}, have {:?})",
                    required, host.shell
                );
                if let Some(p) = progress {
                    p.host_completed(&host.name, HostStatus::Skipped, &detail, 0);
                }
                host_results.push(ExecHostResult {
                    host: host.name.clone(),
                    status: HostStatus::Skipped,
                    duration_ms: None,
                    detail,
                    stdout: String::new(),
                    stderr: String::new(),
                });
                continue;
            }
        }

        let task_host = Arc::clone(host);
        let script_path = script_path.to_path_buf();
        let timeout = ctx.timeout;
        let sessions = pool.session_pool.clone();
        fan.spawn(host, progress, async move {
            exec_on_host_pooled(&task_host, &script_path, timeout, keep, sudo, sessions).await
        });
    }

    while let Some(done) = fan.next().await {
        let (host, result, elapsed) = done?;
        let ms = elapsed.as_millis() as u64;
        let now = chrono::Utc::now().timestamp();

        match result {
            Ok(exec_stdout) => {
                let first_line = exec_stdout.lines().next().unwrap_or("").to_string();
                let detail = if first_line.is_empty() {
                    format!("ok ({:.1}s)", elapsed.as_secs_f64())
                } else {
                    format!("ok ({:.1}s) — {}", elapsed.as_secs_f64(), first_line)
                };
                if let Some(p) = progress {
                    p.host_completed(&host.name, HostStatus::Online, &detail, ms);
                }
                let stdout_preview = exec_stdout
                    .lines()
                    .next()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| l.trim_end());

                crate::commands::report::record_operation_log(
                    &ctx.db,
                    now,
                    "exec",
                    &host.name,
                    script,
                    HostStatus::Online,
                    elapsed.as_millis() as i64,
                    None,
                    stdout_preview,
                )
                .await;
                host_results.push(ExecHostResult {
                    host: host.name.clone(),
                    status: HostStatus::Online,
                    duration_ms: Some(ms),
                    detail,
                    stdout: exec_stdout,
                    stderr: String::new(),
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
                    "exec",
                    &host.name,
                    script,
                    HostStatus::Error,
                    elapsed.as_millis() as i64,
                    Some(&e.to_string()),
                    None,
                )
                .await;
                host_results.push(ExecHostResult {
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

    Ok(ExecReport {
        executed_at,
        script: script.to_string(),
        targets,
        hosts: host_results,
    })
}

/// Thin CLI wrapper: handles dry-run, calls `exec_core`, prints summary,
/// writes `--out` reports.
pub async fn run(
    ctx: &Context,
    script: &str,
    sudo: bool,
    keep: bool,
    dry_run: bool,
    output: &crate::cli::OutputArgs,
) -> Result<HostOutcome> {
    let script_path = crate::util::expand_tilde(Path::new(script));

    if dry_run {
        if !script_path.exists() {
            bail!("Script not found: {}", script);
        }
        let compatible_shell = script_extension_to_shell(&script_path);
        println!("[dry-run] Script: {}", script);
        println!("[dry-run] Compatible shell: {:?}", compatible_shell);
        let hosts = ctx.resolve_hosts()?;
        for host in &hosts {
            let compat = compatible_shell.is_none_or(|s| s == host.shell);
            if !compat {
                printer::print_host_line(&host.name, "skip", "shell mismatch");
            } else if let Some(Err(e)) = sudo.then(|| shell::sudo_wrap(host.shell, "")) {
                printer::print_host_line(&host.name, "error", &e.to_string());
            } else {
                printer::print_host_line(&host.name, "ok", "would execute");
            }
        }
        return Ok(HostOutcome::default());
    }

    let sink = default_printer_sink();
    let report = exec_core(ctx, script, sudo, keep, Some(&sink)).await?;

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
        "exec",
        ctx.config.settings.default_output_format.as_deref(),
    )?;

    Ok(raw.host_outcome())
}

async fn exec_on_host_pooled(
    host: &crate::config::schema::HostEntry,
    script_path: &Path,
    timeout: u64,
    keep: bool,
    sudo: bool,
    sessions: std::sync::Arc<crate::host::session_pool::RusshSessionPool>,
) -> Result<String> {
    if sudo {
        // Refuse before uploading anything: Windows `--sudo` is unsupported (B31).
        shell::sudo_wrap(host.shell, "")?;
    }
    let temp_dir = get_expanded_temp_dir_pooled(host, timeout, sessions.clone()).await?;
    let script_name = script_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("sshi_script");

    let suffix = format!("sshi_{}_{}", std::process::id(), script_name);

    // Upload — for Sh shells, try /tmp first then fall back to ~/ (like scp_probe)
    let remote_path = if host.shell == ShellType::Sh {
        let primary = format!("{}/{}", temp_dir, suffix);
        match sessions.upload(host, script_path, &primary, timeout).await {
            Ok(()) => primary,
            Err(_) => {
                let fallback = format!("~/{}", suffix);
                sessions
                    .upload(host, script_path, &fallback, timeout)
                    .await?;
                fallback
            }
        }
    } else {
        let path = format!("{}/{}", temp_dir, suffix);
        sessions.upload(host, script_path, &path, timeout).await?;
        path
    };

    // One literal argument for the remote shell (B26): the path embeds the
    // local script's file name, which may contain spaces or shell syntax.
    let remote_path_quoted = crate::host::quote::quote_path(host.shell, &remote_path)?;

    // Make executable (sh only)
    if host.shell == ShellType::Sh {
        sessions
            .exec(
                &host.ssh_host,
                &format!("chmod +x {}", remote_path_quoted),
                timeout,
            )
            .await?;
    }

    // Execute
    let exec_cmd = match host.shell {
        ShellType::Sh | ShellType::Cmd => remote_path_quoted.clone(),
        ShellType::PowerShell => format!("powershell -File {}", remote_path_quoted),
    };

    let exec_cmd = if sudo {
        shell::sudo_wrap(host.shell, &exec_cmd)?
    } else {
        exec_cmd
    };

    let output = sessions.exec(&host.ssh_host, &exec_cmd, timeout).await?;

    // Cleanup (unless --keep)
    if !keep {
        let rm_cmd = match host.shell {
            ShellType::Sh => format!("rm -f {}", remote_path_quoted),
            ShellType::PowerShell => format!("Remove-Item -Force {}", remote_path_quoted),
            ShellType::Cmd => format!("del /f {}", remote_path_quoted),
        };
        let _ = sessions.exec(&host.ssh_host, &rm_cmd, timeout).await;
    }

    if output.success {
        Ok(output.stdout)
    } else {
        bail!(
            "Script failed (exit {}): {}",
            output.exit_code.unwrap_or(-1),
            output.stderr.trim()
        );
    }
}

async fn get_expanded_temp_dir_pooled(
    host: &crate::config::schema::HostEntry,
    timeout: u64,
    sessions: std::sync::Arc<crate::host::session_pool::RusshSessionPool>,
) -> Result<String> {
    let temp_dir = shell::temp_dir(host.shell);

    // For sh, /tmp is already a literal path
    if host.shell == ShellType::Sh {
        return Ok(temp_dir.to_string());
    }

    // For PowerShell and Cmd, need to expand the variable
    let echo_cmd = match host.shell {
        ShellType::PowerShell => "echo $env:TEMP".to_string(),
        ShellType::Cmd => "echo %TEMP%".to_string(),
        ShellType::Sh => unreachable!(),
    };

    let output = sessions.exec(&host.ssh_host, &echo_cmd, timeout).await?;

    if !output.success {
        bail!("Failed to get temp directory: {}", output.stderr.trim());
    }

    Ok(output.stdout.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_script_extension_to_shell() {
        assert_eq!(
            script_extension_to_shell(Path::new("test.sh")),
            Some(ShellType::Sh)
        );
        assert_eq!(
            script_extension_to_shell(Path::new("TEST.SH")),
            Some(ShellType::Sh)
        );
        assert_eq!(
            script_extension_to_shell(Path::new("test.ps1")),
            Some(ShellType::PowerShell)
        );
        assert_eq!(
            script_extension_to_shell(Path::new("test.bat")),
            Some(ShellType::Cmd)
        );
        assert_eq!(
            script_extension_to_shell(Path::new("test.cmd")),
            Some(ShellType::Cmd)
        );
        assert_eq!(script_extension_to_shell(Path::new("test.py")), None);
        assert_eq!(script_extension_to_shell(Path::new("test")), None);
    }
}
