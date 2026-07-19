//! Pure command core for `sshi init`.
//!
//! Splits the legacy 400-line `run()` body (audit §2.2 HIGH, §2.5 HIGH) into
//! reusable non-interactive helpers ([`partition_host_key_failures`],
//! [`partition_auth_failures`], [`batch_keyscan_and_accept`],
//! [`detect_shells`], [`persist_init_result`]) and a thin orchestrator
//! ([`init_core`]) that runs the final shell-detection + persistence phase
//! after the CLI wrapper has collected all interactive answers.
//!
//! Behaviour is byte-identical to the legacy `init::run`: same partitioning
//! of failures, same keyscan / ssh-copy-id retry order, same persist
//! conditions. Per-host prompts and `printer::*` narration live in the CLI
//! wrapper ([`super`]); the helpers here take pre-authorised inputs and
//! surface per-host events via the optional [`ProgressSink`] (only
//! [`batch_keyscan_and_accept`] and [`detect_shells`] actually emit progress).
//!
//! The audit's suggested `offer_keyscan_retry` / `offer_ssh_copy_id_retry`
//! helpers are intentionally NOT extracted: the legacy byte stream
//! interleaves per-host prompts with per-host ssh-copy-id output, and any
//! batch helper would reorder that stream. The wrapper owns the prompt +
//! work interleaving; [`batch_keyscan_and_accept`] is the only keyscan-side
//! extraction (it parallelises cleanly with no per-host prompt).

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};

use crate::commands::report::{HostStatus, ProgressSink};
use crate::commands::Context;
use crate::config::schema::HostEntry;
use crate::config::ssh_config::SshHostEntry;
use crate::host::session_pool::RusshSessionPool;
use crate::host::shell;
use crate::output::summary::Summary;

use super::report::{InitDetectedHost, InitFailedHost, InitPlan, InitReport};

/// Partition connection failures into host-key verification errors and other errors.
#[allow(clippy::type_complexity)]
pub(crate) fn partition_host_key_failures(
    failures: Vec<(String, String)>,
) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let mut host_key_failures = Vec::new();
    let mut other_failures = Vec::new();
    for (name, err) in failures {
        if err.contains("Unknown host key") || err.contains("Host key verification failed") {
            host_key_failures.push((name, err));
        } else {
            other_failures.push((name, err));
        }
    }
    (host_key_failures, other_failures)
}

/// Split connection failures into authentication failures (key rejected / no
/// key accepted) and everything else. Auth failures surface as anyhow's outer
/// context "Authentication failed for …" from `connect_direct`.
#[allow(clippy::type_complexity)]
pub(crate) fn partition_auth_failures(
    failures: Vec<(String, String)>,
) -> (Vec<(String, String)>, Vec<(String, String)>) {
    failures
        .into_iter()
        .partition(|(_, err)| err.contains("Authentication failed"))
}

/// Whether the user has any default SSH key pair (`~/.ssh/id_*.pub`).
pub(crate) fn default_ssh_key_exists() -> bool {
    let Some(ssh_dir) = dirs::home_dir().map(|h| h.join(".ssh")) else {
        return false;
    };
    ["id_ed25519", "id_rsa", "id_ecdsa", "id_dsa"]
        .iter()
        .any(|name| ssh_dir.join(format!("{name}.pub")).exists())
}

/// Run an interactive command on the real TTY (inherited stdio), so prompts like
/// passphrases and remote passwords work. Returns true on success.
///
/// Lives in `core` rather than the CLI wrapper because it is dispatched by
/// [`InitPlan`] decisions: when the plan authorises the action, the core
/// helper invokes the subprocess. The TUI never authorises these policies,
/// so the inherited-TTY subprocess is never spawned from TUI mode.
pub(crate) fn run_interactive(program: &str, args: &[&str]) -> bool {
    match std::process::Command::new(program).args(args).status() {
        Ok(status) => status.success(),
        Err(e) => {
            tracing::warn!("Failed to run {program}: {e}");
            false
        }
    }
}

/// Resolve SSH hostname and port for a host alias using ~/.ssh/config.
async fn resolve_ssh_host_port(alias: &str) -> Result<(String, u16)> {
    let resolved = crate::config::ssh_config::resolve_host(alias)?;
    Ok((resolved.hostname, resolved.port))
}

/// Run ssh-keyscan for a single host and return the output lines (key entries).
/// Returns Ok(output) on success, Err on failure or empty output.
///
/// Entries are written **unhashed** (no `-H`): russh's `check_known_hosts`
/// matcher only reliably matches plain `host`/`[host]:port` tokens by string
/// equality. Hashed (`|1|`) entries written by `ssh-keyscan -H` are not matched
/// by russh, so a host scanned with `-H` would still be reported as an unknown
/// host key (notably non-standard-port hosts looked up as `[host]:port`).
async fn keyscan_host(alias: &str, timeout_secs: u64) -> Result<String> {
    let (hostname, port) = resolve_ssh_host_port(alias).await?;

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        tokio::process::Command::new("ssh-keyscan")
            .arg("-p")
            .arg(port.to_string())
            .arg(&hostname)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output(),
    )
    .await
    .context("ssh-keyscan timeout")?
    .context("Failed to run ssh-keyscan")?;

    let stdout = String::from_utf8_lossy(&result.stdout).to_string();

    let key_lines: String = stdout
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");

    if key_lines.is_empty() {
        anyhow::bail!("ssh-keyscan returned no keys for {}", alias);
    }

    Ok(key_lines)
}

/// Run ssh-keyscan for multiple hosts in parallel and append results to
/// `~/.ssh/known_hosts`. Returns the list of host names that were
/// successfully keyscanned.
///
/// `progress` is notified per host so the CLI can stream `ok`/`error` lines
/// in the same order as the legacy printer calls.
pub(crate) async fn batch_keyscan_and_accept(
    hosts: &[(String, String)],
    timeout_secs: u64,
    concurrency: usize,
    progress: Option<&dyn ProgressSink>,
) -> Vec<String> {
    let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency));
    let mut handles = Vec::new();

    for (name, _err) in hosts {
        let sem = semaphore.clone();
        let alias = name.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();
            let result = keyscan_host(&alias, timeout_secs).await;
            (alias, result)
        }));
    }

    let mut all_keys = String::new();
    let mut succeeded = Vec::new();

    for handle in handles {
        match handle.await {
            Ok((alias, Ok(keys))) => {
                if !all_keys.is_empty() {
                    all_keys.push('\n');
                }
                all_keys.push_str(&keys);
                succeeded.push(alias.clone());
                if let Some(p) = progress {
                    p.host_completed(&alias, HostStatus::Online, "host key accepted", 0);
                }
            }
            Ok((alias, Err(e))) => {
                if let Some(p) = progress {
                    p.host_completed(
                        &alias,
                        HostStatus::Error,
                        &format!("keyscan failed: {}", e),
                        0,
                    );
                }
            }
            Err(e) => {
                tracing::warn!("keyscan task panicked: {}", e);
            }
        }
    }

    if !all_keys.is_empty() {
        let known_hosts_path = dirs::home_dir()
            .map(|h| h.join(".ssh").join("known_hosts"))
            .expect("Could not determine home directory");

        if let Some(parent) = known_hosts_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut content = String::new();
        if known_hosts_path.exists() {
            if let Ok(existing) = std::fs::read_to_string(&known_hosts_path) {
                if !existing.ends_with('\n') && !existing.is_empty() {
                    content.push('\n');
                }
            }
        }
        content.push_str(&all_keys);
        if !content.ends_with('\n') {
            content.push('\n');
        }

        use std::io::Write;
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&known_hosts_path)
        {
            Ok(mut f) => {
                if let Err(e) = f.write_all(content.as_bytes()) {
                    tracing::warn!("Failed to write known_hosts: {}", e);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to open known_hosts: {}", e);
            }
        }
    }

    succeeded
}

/// Detect shell type on every host in `reachable` using the right pool.
///
/// `pools.session` is the initial setup pool; `pools.retry` is the keyscan
/// retry pool; `pools.auth_retry` is the ssh-copy-id retry pool. The three
/// are searched in that order per host (matches the legacy `Vec::contains`
/// selection).
///
/// Returns the typed `(detected, failed)` pair: detected hosts get a fresh
/// `HostEntry` with the discovered [`ShellType`]; failed hosts get an
/// [`InitFailedHost`] entry. Per-host `Online` / `Error` events stream via
/// `progress`.
pub(crate) async fn detect_shells(
    reachable: &[String],
    pools: &InitPools<'_>,
    timeout: u64,
    progress: Option<&dyn ProgressSink>,
) -> (Vec<HostEntry>, Vec<InitFailedHost>) {
    let mut detected = Vec::new();
    let mut failed = Vec::new();
    for host_name in reachable {
        let host_entry = HostEntry::placeholder(host_name, host_name);
        let pool_ref = if pools.session.reachable_hosts().contains(host_name) {
            pools.session
        } else if pools
            .retry
            .is_some_and(|rp| rp.reachable_hosts().contains(host_name))
        {
            pools.retry.unwrap()
        } else if let Some(rp) = pools.auth_retry {
            rp
        } else {
            continue;
        };
        match shell::detect_russh(&host_entry, pool_ref, timeout).await {
            Ok(shell_type) => {
                if let Some(p) = progress {
                    p.host_completed(
                        host_name,
                        HostStatus::Online,
                        &format!("detected: {}", shell_type),
                        0,
                    );
                }
                let mut entry = HostEntry::placeholder(host_name, host_name);
                entry.shell = shell_type;
                detected.push(entry);
            }
            Err(e) => {
                let detail = e.to_string();
                if let Some(p) = progress {
                    p.host_completed(host_name, HostStatus::Error, &detail, 0);
                }
                failed.push(InitFailedHost {
                    host: host_name.clone(),
                    detail,
                });
            }
        }
    }
    (detected, failed)
}

/// Apply the post-run persistence step: load existing config (or default),
/// optionally drop stale hosts, upsert detected hosts, dedupe `skipped_hosts`,
/// save. Returns the saved path on success, or `None` when nothing changed
/// (or `plan.dry_run` is set).
pub(crate) fn persist_init_result(
    ctx: &Context,
    new_hosts: &[HostEntry],
    plan: &InitPlan,
    stale_host_names: &[String],
) -> Result<Option<PathBuf>> {
    if plan.dry_run {
        return Ok(None);
    }
    let stale_hosts_removed = plan.remove_stale_hosts && !stale_host_names.is_empty();
    if !stale_hosts_removed && new_hosts.is_empty() && plan.skip.is_empty() {
        return Ok(None);
    }
    let mut config = crate::config::app::load(ctx.config_path.as_deref())?.unwrap_or_default();
    if stale_hosts_removed {
        config
            .host
            .retain(|h| !stale_host_names.contains(&h.ssh_host));
    }
    for host in new_hosts {
        if let Some(idx) = config.host.iter().position(|h| h.ssh_host == host.ssh_host) {
            Arc::make_mut(&mut config.host[idx]).shell = host.shell;
        } else {
            config.host.push(Arc::new(host.clone()));
        }
    }
    for s in &plan.skip {
        if !config.settings.skipped_hosts.contains(s) {
            config.settings.skipped_hosts.push(s.clone());
        }
    }
    crate::config::app::save(&config, ctx.config_path.as_deref())?;
    let saved_path = crate::config::app::resolve_path(ctx.config_path.as_deref())?;
    Ok(Some(saved_path))
}

/// Borrowed handles to the three pools that may carry reachable hosts at
/// detect-shell time. `session` is always set (initial setup); `retry` and
/// `auth_retry` are present only when the user authorised the corresponding
/// retry path and the retry produced at least one connection.
pub struct InitPools<'a> {
    pub session: &'a RusshSessionPool,
    pub retry: Option<&'a RusshSessionPool>,
    pub auth_retry: Option<&'a RusshSessionPool>,
}

/// Pure command core: runs the final shell-detection + persistence phase
/// after the CLI wrapper has collected all interactive answers and (when
/// authorised) driven the keyscan / ssh-copy-id retry flows.
///
/// `pools` borrows the session pool plus any retry pools the wrapper
/// created; their reachable-host union is what [`detect_shells`] walks.
/// `plan` carries the persistence-time decisions (`dry_run`, `skip`,
/// `remove_stale_hosts`); the interactive answers
/// (`accept_unknown_host_keys`, `copy_id_targets`, etc.) don't reach
/// `init_core` — they're consumed by the wrapper-orchestrated retry flows.
///
/// No `println!`, no `printer::*`, no `stdin` reads. Per-host detect events
/// stream via `progress`.
pub async fn init_core(
    ctx: &Context,
    ssh_hosts: &[SshHostEntry],
    pools: InitPools<'_>,
    plan: &InitPlan,
    progress: Option<&dyn ProgressSink>,
) -> Result<InitReport> {
    let executed_at = chrono::Utc::now().to_rfc3339();

    let mut report = InitReport {
        executed_at,
        ssh_hosts_count: ssh_hosts.len(),
        skipped_hosts: Vec::new(),
        detected_hosts: Vec::new(),
        stale_host_names: Vec::new(),
        stale_hosts_removed: false,
        failed_hosts: Vec::new(),
        summary_should_print: false,
        no_changes: false,
        persisted_path: None,
        dry_run: plan.dry_run,
        summary: Summary::default(),
        new_hosts: Vec::new(),
    };

    if ssh_hosts.is_empty() {
        return Ok(report);
    }

    let config_exists = crate::config::app::resolve_path(ctx.config_path.as_deref())?.exists();
    if config_exists {
        let ssh_host_names: std::collections::HashSet<&str> =
            ssh_hosts.iter().map(|h| h.name.as_str()).collect();
        report.stale_host_names = ctx
            .config
            .host
            .iter()
            .filter(|h| !ssh_host_names.contains(h.ssh_host.as_str()))
            .map(|h| h.ssh_host.clone())
            .collect();
    }
    report.stale_hosts_removed = plan.remove_stale_hosts && !report.stale_host_names.is_empty();

    let all_skips: Vec<String> = ctx
        .config
        .settings
        .skipped_hosts
        .iter()
        .cloned()
        .chain(plan.skip.iter().cloned())
        .collect();
    for ssh_host in ssh_hosts {
        if all_skips.iter().any(|s| s == &ssh_host.name) {
            report.skipped_hosts.push(ssh_host.name.clone());
            report.summary.add_skip();
        }
    }

    let mut reachable = pools.session.reachable_hosts();
    if let Some(rp) = pools.retry {
        reachable.extend(rp.reachable_hosts());
    }
    if let Some(rp) = pools.auth_retry {
        reachable.extend(rp.reachable_hosts());
    }

    let (new_hosts, failed_hosts) = detect_shells(&reachable, &pools, ctx.timeout, progress).await;
    for h in &new_hosts {
        report.summary.add_success();
        report.detected_hosts.push(InitDetectedHost {
            host: h.name.clone(),
            shell: h.shell.to_string(),
        });
    }
    for f in &failed_hosts {
        report.summary.add_failure(&f.host, &f.detail);
    }
    report.failed_hosts.extend(failed_hosts);
    report.new_hosts = new_hosts;

    report.summary_should_print = true;

    if plan.dry_run {
        return Ok(report);
    }

    if report.new_hosts.is_empty() && plan.skip.is_empty() && !report.stale_hosts_removed {
        report.no_changes = true;
        return Ok(report);
    }

    report.persisted_path =
        persist_init_result(ctx, &report.new_hosts, plan, &report.stale_host_names)?;

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_host_falls_back_to_alias() {
        let result = crate::config::ssh_config::resolve_host("nonexistent-test-host-xyz");
        let resolved = result.unwrap();
        assert_eq!(resolved.hostname, "nonexistent-test-host-xyz");
        assert_eq!(resolved.port, 22);
    }

    #[test]
    fn test_partition_auth_failures_splits_on_auth_context() {
        let failures = vec![
            (
                "host-a".to_string(),
                "Authentication failed for bob@host-a:22".to_string(),
            ),
            ("host-b".to_string(), "Connection refused".to_string()),
            (
                "host-c".to_string(),
                "Authentication failed for bob@host-c:22".to_string(),
            ),
        ];
        let (auth, other) = partition_auth_failures(failures);
        assert_eq!(auth.len(), 2);
        assert_eq!(auth[0].0, "host-a");
        assert_eq!(auth[1].0, "host-c");
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].0, "host-b");
    }

    #[test]
    fn test_partition_host_key_failures_mixed() {
        let failures = vec![
            (
                "host-a".to_string(),
                "ControlMaster failed: Host key verification failed.".to_string(),
            ),
            ("host-b".to_string(), "Connection refused".to_string()),
            (
                "host-c".to_string(),
                "ControlMaster failed: Host key verification failed.".to_string(),
            ),
        ];
        let (hk, other) = partition_host_key_failures(failures);
        assert_eq!(hk.len(), 2);
        assert_eq!(hk[0].0, "host-a");
        assert_eq!(hk[1].0, "host-c");
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].0, "host-b");
    }

    #[test]
    fn test_partition_host_key_failures_none() {
        let failures = vec![("host-a".to_string(), "Connection timeout".to_string())];
        let (hk, other) = partition_host_key_failures(failures);
        assert!(hk.is_empty());
        assert_eq!(other.len(), 1);
    }

    #[test]
    fn test_partition_host_key_failures_all() {
        let failures = vec![(
            "host-a".to_string(),
            "Host key verification failed.".to_string(),
        )];
        let (hk, other) = partition_host_key_failures(failures);
        assert_eq!(hk.len(), 1);
        assert!(other.is_empty());
    }

    #[test]
    fn test_partition_host_key_failures_unknown_host_key() {
        let failures = vec![
            (
                "host-a".to_string(),
                "Unknown host key for server.example.com".to_string(),
            ),
            ("host-b".to_string(), "Connection refused".to_string()),
        ];
        let (hk, other) = partition_host_key_failures(failures);
        assert_eq!(hk.len(), 1);
        assert_eq!(hk[0].0, "host-a");
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].0, "host-b");
    }

    #[test]
    fn test_partition_host_key_failures_both_error_strings() {
        let failures = vec![
            (
                "host-a".to_string(),
                "Unknown host key for server.example.com".to_string(),
            ),
            (
                "host-b".to_string(),
                "ControlMaster failed: Host key verification failed.".to_string(),
            ),
            ("host-c".to_string(), "Timeout".to_string()),
        ];
        let (hk, other) = partition_host_key_failures(failures);
        assert_eq!(hk.len(), 2);
        assert_eq!(hk[0].0, "host-a");
        assert_eq!(hk[1].0, "host-b");
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].0, "host-c");
    }

    fn build_test_ctx() -> Context {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::state::db::migrate_for_test(&conn);
        Context {
            config: crate::config::schema::AppConfig::default(),
            config_path: None,
            db: crate::state::db::DbHandle::new(conn),
            timeout: 5,
            mode: crate::commands::TargetMode::All,
            serial: false,
            skip: vec![],
            verbose: false,
            auth_sender: None,
        }
    }

    struct NullSink;
    impl ProgressSink for NullSink {
        fn host_started(&self, _host: &str) {}
        fn host_completed(&self, _host: &str, _status: HostStatus, _detail: &str, _ms: u64) {}
    }

    #[tokio::test]
    async fn init_core_no_ssh_hosts_returns_empty_report_without_io() {
        let ctx = build_test_ctx();
        let session_pool = RusshSessionPool::setup(&[], ctx.timeout, ctx.concurrency(), None)
            .await
            .unwrap();
        let pools = InitPools {
            session: &session_pool,
            retry: None,
            auth_retry: None,
        };
        let plan = InitPlan::default();
        let sink = NullSink;
        let report = init_core(&ctx, &[], pools, &plan, Some(&sink))
            .await
            .unwrap();
        session_pool.shutdown().await;
        assert_eq!(report.ssh_hosts_count, 0);
        assert!(report.detected_hosts.is_empty());
        assert!(report.stale_host_names.is_empty());
        assert!(!report.stale_hosts_removed);
        assert!(report.persisted_path.is_none());
        assert!(!report.dry_run);
        assert!(!report.summary_should_print);
    }

    #[test]
    fn init_plan_default_declines_all_interactive_decisions() {
        let plan = InitPlan::default();
        assert!(!plan.dry_run);
        assert!(!plan.update);
        assert!(plan.skip.is_empty());
        assert!(!plan.remove_stale_hosts);
        assert!(!plan.accept_unknown_host_keys);
        assert!(!plan.generate_ssh_key_if_missing);
        assert!(plan.copy_id_targets.is_empty());
    }

    #[test]
    fn init_report_serializes_with_expected_top_level_fields() {
        let report = InitReport {
            executed_at: "2026-07-19T00:00:00Z".to_string(),
            ssh_hosts_count: 3,
            skipped_hosts: vec!["a".to_string()],
            detected_hosts: vec![InitDetectedHost {
                host: "b".to_string(),
                shell: "sh".to_string(),
            }],
            stale_host_names: vec![],
            stale_hosts_removed: false,
            failed_hosts: vec![InitFailedHost {
                host: "c".to_string(),
                detail: "boom".to_string(),
            }],
            summary_should_print: true,
            no_changes: false,
            persisted_path: None,
            dry_run: true,
            summary: Summary::default(),
            new_hosts: vec![],
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["ssh_hosts_count"], 3);
        assert_eq!(json["skipped_hosts"][0], "a");
        assert_eq!(json["detected_hosts"][0]["host"], "b");
        assert_eq!(json["detected_hosts"][0]["shell"], "sh");
        assert_eq!(json["failed_hosts"][0]["host"], "c");
        assert_eq!(json["failed_hosts"][0]["detail"], "boom");
        assert_eq!(json["dry_run"], true);
    }

    #[test]
    fn persist_init_result_skips_when_dry_run() {
        let ctx = build_test_ctx();
        let plan = InitPlan {
            dry_run: true,
            ..Default::default()
        };
        let path = persist_init_result(&ctx, &[], &plan, &[]).unwrap();
        assert!(path.is_none());
    }

    #[test]
    fn persist_init_result_noop_when_nothing_to_persist() {
        let ctx = build_test_ctx();
        let plan = InitPlan::default();
        let path = persist_init_result(&ctx, &[], &plan, &[]).unwrap();
        assert!(path.is_none());
    }

    #[test]
    fn persist_init_result_writes_skip_list() {
        let ctx = build_test_ctx();
        let plan = InitPlan {
            skip: vec!["host-x".to_string()],
            ..Default::default()
        };
        let path = persist_init_result(&ctx, &[], &plan, &[]).unwrap();
        assert!(path.is_some(), "skip list change should persist");
        // Reload and verify the skip was recorded.
        let reloaded = crate::config::app::load(ctx.config_path.as_deref())
            .unwrap()
            .unwrap();
        assert!(reloaded
            .settings
            .skipped_hosts
            .iter()
            .any(|s| s == "host-x"));
    }
}
