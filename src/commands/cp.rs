//! `cp` command — copy local files or directories to remote hosts (scp-style
//! fan-out). The local argument may be a single file, a directory (copied
//! recursively), or a single-level wildcard pattern (`dir/*.ext`) expanded by
//! sshi itself. The optional remote argument defaults to the remote home
//! directory; a leading `~` is expanded per host.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Result};

use crate::host::pool::SshPool;
use crate::output::printer;
use crate::output::summary::Summary;

use super::report::{CommandReport, CpHostResult, CpReport, HostStatus, ProgressSink};
use super::Context;

/// A single planned file transfer: local source → remote destination path.
#[derive(Debug, Clone)]
struct Transfer {
    local: PathBuf,
    remote: String,
}

/// Pure command core: plans the transfers, uploads them to each reachable
/// SFTP-capable host, writes the operation log, and returns a typed `CpReport`.
pub async fn cp_core(
    ctx: &Context,
    local: &str,
    remote: Option<&str>,
    progress: Option<&dyn ProgressSink>,
) -> Result<CommandReport> {
    let transfers = plan_transfers(local, remote)?;
    let remote_base = remote.unwrap_or("~").to_string();

    let hosts = ctx.resolve_hosts()?;
    let executed_at = chrono::Utc::now().to_rfc3339();
    let targets: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();

    // Probe SCP/SFTP capability during setup (cp transfers exclusively via SFTP).
    let (pool, _connected) = SshPool::setup_with_options(
        &hosts,
        ctx.timeout,
        ctx.concurrency(),
        ctx.per_host_concurrency(),
        true,
    )
    .await?;

    let mut host_results: Vec<CpHostResult> = Vec::new();

    for (name, err) in pool.failed_hosts() {
        let detail = format!("unreachable — {}", err);
        if let Some(p) = progress {
            p.host_completed(&name, HostStatus::Unreachable, &detail, 0);
        }
        host_results.push(CpHostResult {
            host: name.clone(),
            status: HostStatus::Unreachable,
            duration_ms: None,
            detail,
            files_copied: 0,
            files_failed: transfers.len(),
            errors: vec![err],
        });
    }
    for (name, err) in pool.sftp_failed_hosts() {
        let detail = format!("sftp unavailable — {}", err);
        if let Some(p) = progress {
            p.host_completed(&name, HostStatus::Error, &detail, 0);
        }
        host_results.push(CpHostResult {
            host: name.clone(),
            status: HostStatus::Error,
            duration_ms: None,
            detail,
            files_copied: 0,
            files_failed: transfers.len(),
            errors: vec![format!("sftp probe failed: {}", err)],
        });
    }

    let reachable = pool.filter_sftp_capable(&hosts);

    let mut handles = Vec::new();
    for host in &reachable {
        let host = (*host).clone();
        let transfers = transfers.clone();
        let timeout = ctx.timeout;
        let sessions = pool.session_pool.clone();
        let global_sem = pool.limiter.global_semaphore();
        if let Some(p) = progress {
            p.host_started(&host.name);
        }

        handles.push(tokio::spawn(async move {
            let _permit = global_sem.acquire_owned().await.unwrap();
            let start = Instant::now();
            let mut copied = 0usize;
            let mut errors: Vec<String> = Vec::new();
            for t in &transfers {
                match sessions.upload(&host, &t.local, &t.remote, timeout).await {
                    Ok(()) => copied += 1,
                    Err(e) => errors.push(format!("{}: {}", t.remote, e)),
                }
            }
            let elapsed = start.elapsed();
            (host, copied, errors, elapsed)
        }));
    }

    for handle in handles {
        let (host, copied, errors, elapsed) = handle.await?;
        let ms = elapsed.as_millis() as u64;
        let now = chrono::Utc::now().timestamp();
        let failed = errors.len();

        let (status, detail) = if failed == 0 {
            (
                HostStatus::Online,
                format!("{} file(s) copied ({:.1}s)", copied, elapsed.as_secs_f64()),
            )
        } else if copied == 0 {
            (
                HostStatus::Error,
                format!(
                    "all {} transfer(s) failed ({:.1}s) — {}",
                    failed,
                    elapsed.as_secs_f64(),
                    errors.first().map(|s| s.as_str()).unwrap_or("unknown")
                ),
            )
        } else {
            (
                HostStatus::Partial,
                format!(
                    "{} copied, {} failed ({:.1}s) — {}",
                    copied,
                    failed,
                    elapsed.as_secs_f64(),
                    errors.first().map(|s| s.as_str()).unwrap_or("unknown")
                ),
            )
        };

        let status_str = if failed == 0 { "ok" } else { "error" };
        let _ = ctx.db.execute(
            "INSERT INTO operation_log (timestamp, command, host, action, status, duration_ms) \
             VALUES (?1, 'cp', ?2, ?3, ?4, ?5)",
            rusqlite::params![
                now,
                host.name,
                format!("cp {} -> {}", local, remote_base),
                status_str,
                ms as i64
            ],
        );

        if let Some(p) = progress {
            p.host_completed(&host.name, status, &detail, ms);
        }
        host_results.push(CpHostResult {
            host: host.name.clone(),
            status,
            duration_ms: Some(ms),
            detail,
            files_copied: copied,
            files_failed: failed,
            errors,
        });
    }

    pool.shutdown().await;

    Ok(CommandReport::Cp(CpReport {
        executed_at,
        local: local.to_string(),
        remote: remote_base,
        planned_files: transfers.len(),
        targets,
        hosts: host_results,
    }))
}

/// Thin CLI wrapper: handles dry-run, calls `cp_core`, prints summary, writes
/// `--out` reports.
pub async fn run(
    ctx: &Context,
    local: &str,
    remote: Option<&str>,
    dry_run: bool,
    output: &crate::cli::OutputArgs,
) -> Result<()> {
    if dry_run {
        let transfers = plan_transfers(local, remote)?;
        let remote_base = remote.unwrap_or("~");
        println!(
            "[dry-run] local: {}  →  remote base: {}",
            local, remote_base
        );
        println!("[dry-run] {} file(s) per host:", transfers.len());
        for t in &transfers {
            println!("  {} → {}", t.local.display(), t.remote);
        }
        let hosts = ctx.resolve_hosts()?;
        for host in &hosts {
            printer::print_host_line(&host.name, "ok", "would copy");
        }
        return Ok(());
    }

    let sink = PrinterSink;
    let raw = cp_core(ctx, local, remote, Some(&sink)).await?;
    let CommandReport::Cp(report) = &raw else {
        unreachable!("cp_core always returns CommandReport::Cp")
    };

    let mut summary = Summary::default();
    for h in &report.hosts {
        match h.status {
            HostStatus::Online => summary.add_success(),
            HostStatus::Skipped => summary.add_skip(),
            _ => summary.add_failure(&h.host, &h.detail),
        }
    }
    summary.print();

    if let Some(out) = &output.out {
        let rep = crate::output::report::to_operation_report(&raw, &ctx.mode);
        let path = crate::output::report::write_report(
            &rep,
            out,
            "cp",
            ctx.config.settings.default_output_format.as_deref(),
        )?;
        println!("Report written to {}", path);
    }

    Ok(())
}

/// `ProgressSink` impl that prints to stdout via the existing `output::printer`.
struct PrinterSink;

impl ProgressSink for PrinterSink {
    fn host_started(&self, _host: &str) {}

    fn host_completed(&self, host: &str, status: HostStatus, detail: &str, _ms: u64) {
        let kind = match status {
            HostStatus::Online => "ok",
            HostStatus::Partial => "skip",
            HostStatus::Skipped => "skip",
            _ => "error",
        };
        printer::print_host_line(host, kind, detail);
    }
}

/// Expand the local argument into concrete per-file transfers with their remote
/// destination paths.
///
/// Rules (scp-like):
/// - The remote base is the remote argument, or `~` when omitted.
/// - A single regular file with an explicit remote that does NOT end in `/` is
///   copied to that exact path (rename); otherwise files land under the base by
///   name.
/// - A directory is copied recursively, preserving its own name under the base.
fn plan_transfers(local: &str, remote: Option<&str>) -> Result<Vec<Transfer>> {
    let matches = expand_glob(local)?;
    let multi = matches.len() > 1;
    let base = remote.unwrap_or("~");
    let rename_target = !multi && remote.map(|r| !r.ends_with('/')).unwrap_or(false);

    let mut transfers = Vec::new();
    for m in matches {
        let meta = std::fs::metadata(&m)
            .map_err(|e| anyhow::anyhow!("Cannot stat {}: {}", m.display(), e))?;
        if meta.is_dir() {
            let parent = m.parent().unwrap_or_else(|| Path::new(""));
            let files = walk_files(&m)?;
            if files.is_empty() {
                continue;
            }
            for f in files {
                let rel = f.strip_prefix(parent).unwrap_or(&f);
                let rel_str = path_to_remote_rel(rel);
                transfers.push(Transfer {
                    local: f.clone(),
                    remote: join_remote(base, &rel_str),
                });
            }
        } else {
            let name = m
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| anyhow::anyhow!("Invalid file name: {}", m.display()))?;
            let dest = if rename_target {
                base.to_string()
            } else {
                join_remote(base, name)
            };
            transfers.push(Transfer {
                local: m.clone(),
                remote: dest,
            });
        }
    }

    if transfers.is_empty() {
        bail!("No files to copy for '{}'", local);
    }
    Ok(transfers)
}

/// Expand a possibly-wildcarded local path into matching paths.
///
/// Only the final path component may contain `*` / `?` wildcards (single
/// directory level — the documented `dir/*.ext` form). A literal path must
/// exist or this errors.
fn expand_glob(local: &str) -> Result<Vec<PathBuf>> {
    let path = Path::new(local);
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    if !file_name.contains('*') && !file_name.contains('?') {
        if !path.exists() {
            bail!("Local path not found: {}", local);
        }
        return Ok(vec![path.to_path_buf()]);
    }

    let dir = path.parent().filter(|p| !p.as_os_str().is_empty());
    let dir = dir.unwrap_or_else(|| Path::new("."));
    let mut matches: Vec<PathBuf> = Vec::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("Cannot read directory {}: {}", dir.display(), e))?;
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            if wildcard_match(file_name, name) {
                matches.push(entry.path());
            }
        }
    }
    matches.sort();
    if matches.is_empty() {
        bail!("No files matched pattern: {}", local);
    }
    Ok(matches)
}

/// Recursively collect all regular files under `dir`.
fn walk_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d)
            .map_err(|e| anyhow::anyhow!("Cannot read directory {}: {}", d.display(), e))?;
        for entry in entries.flatten() {
            let p = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(p),
                Ok(ft) if ft.is_file() => out.push(p),
                _ => {}
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Join a remote base directory with a relative path using forward slashes
/// (SFTP path separator), trimming a trailing slash on the base.
fn join_remote(base: &str, rel: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), rel)
}

/// Render a relative path with forward-slash separators for the remote side.
fn path_to_remote_rel(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

/// Match a file name against a simple wildcard pattern supporting `*` (any
/// run, including empty) and `?` (exactly one character). Backtracking glob.
fn wildcard_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    let (mut pi, mut ni) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn wildcard_basic() {
        assert!(wildcard_match("*.toml", "config.toml"));
        assert!(wildcard_match("*.toml", ".toml"));
        assert!(!wildcard_match("*.toml", "config.txt"));
        assert!(wildcard_match("file?.log", "file1.log"));
        assert!(!wildcard_match("file?.log", "file12.log"));
        assert!(wildcard_match("*", "anything"));
        assert!(wildcard_match("a*b*c", "axxbyyc"));
    }

    #[test]
    fn single_file_default_remote_uses_name() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("app.conf");
        fs::write(&f, b"x").unwrap();
        let t = plan_transfers(f.to_str().unwrap(), None).unwrap();
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].remote, "~/app.conf");
    }

    #[test]
    fn single_file_explicit_path_is_rename() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("app.conf");
        fs::write(&f, b"x").unwrap();
        let t = plan_transfers(f.to_str().unwrap(), Some("/etc/app/app.conf")).unwrap();
        assert_eq!(t[0].remote, "/etc/app/app.conf");
    }

    #[test]
    fn single_file_trailing_slash_remote_is_dir() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("app.conf");
        fs::write(&f, b"x").unwrap();
        let t = plan_transfers(f.to_str().unwrap(), Some("~/configs/")).unwrap();
        assert_eq!(t[0].remote, "~/configs/app.conf");
    }

    #[test]
    fn directory_is_recursive_and_preserves_name() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("assets");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), b"a").unwrap();
        fs::write(src.join("sub/b.txt"), b"b").unwrap();
        let t = plan_transfers(src.to_str().unwrap(), Some("~/dest")).unwrap();
        let mut remotes: Vec<String> = t.iter().map(|x| x.remote.clone()).collect();
        remotes.sort();
        assert_eq!(
            remotes,
            vec!["~/dest/assets/a.txt", "~/dest/assets/sub/b.txt"]
        );
    }

    #[test]
    fn wildcard_matches_multiple_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.toml"), b"a").unwrap();
        fs::write(dir.path().join("b.toml"), b"b").unwrap();
        fs::write(dir.path().join("c.txt"), b"c").unwrap();
        let pattern = format!("{}/*.toml", dir.path().display());
        let t = plan_transfers(&pattern, Some("~/configs/")).unwrap();
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|x| x.remote.starts_with("~/configs/")));
    }

    #[test]
    fn missing_local_errors() {
        assert!(plan_transfers("/no/such/path/xyz", None).is_err());
    }
}
