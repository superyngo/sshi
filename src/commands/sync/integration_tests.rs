//! Integration tests for the sync phase helpers + `sync_path_across` driven
//! end-to-end via a [`MockSessionPool`] (audit §2.8 HIGH ×2). Closes the
//! Phase D carry-over: `sync_path_across` (~210 lines) had zero direct
//! coverage after the D3 split.
//!
//! These tests do NOT drive the `sync_inner` orchestrator past its
//! early-return paths — that requires intercepting `SshPool::setup_with_options`
//! which lives outside the `SessionPool` trait (see H1 scope notes). The
//! orchestrator's early returns are covered; the four phase helpers +
//! `sync_path_across` are covered via direct invocation with the mock.

use std::collections::HashMap;
use std::sync::Arc;

use crate::commands::sync::{decide_batch, distribute_batch, run_recursive_entries, sync_inner};
use crate::commands::Context;
use crate::config::schema::{AppConfig, HostEntry, ShellType};
use crate::host::concurrency::ConcurrencyLimiter;
use crate::host::session_pool::{RemoteOutput, SessionPool};
use crate::host::session_pool_mock::MockSessionPool;
use crate::output::summary::SyncSummary;

use super::types::SyncDecision;

fn build_ctx() -> Context {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::state::db::migrate_for_test(&conn);
    Context {
        config: Arc::new(AppConfig::default()),
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

fn sh_host(name: &str) -> Arc<HostEntry> {
    Arc::new(HostEntry {
        name: name.to_string(),
        ssh_host: name.to_string(),
        shell: ShellType::Sh,
        groups: vec![],
        proxy_jump: None,
    })
}

/// Build a canned batch-metadata output that mimics what the production
/// `build_batch_metadata_cmd` would produce on a sh host. The mock matches
/// by substring "---FILE:" so we don't have to reconstruct the exact command.
fn sh_batch_output(path: &str, mtime: i64, size: u64, hash: &str) -> RemoteOutput {
    RemoteOutput {
        stdout: format!("---FILE:{path}\n{mtime} {size}\n{hash}\n"),
        stderr: String::new(),
        exit_code: Some(0),
        success: true,
    }
}

/// Build a canned per-file metadata output that mimics what the production
/// `collect_file_metadata` would produce on a sh host. The mock matches by
/// substring "stat " so we don't have to reconstruct the exact command.
/// Format: `<mtime> <size>\n<hash>\n`.
fn sh_per_file_output(mtime: i64, size: u64, hash: &str) -> RemoteOutput {
    RemoteOutput {
        stdout: format!("{mtime} {size}\n{hash}\n"),
        stderr: String::new(),
        exit_code: Some(0),
        success: true,
    }
}

/// Build a `SyncDecision` for `decide_batch`/`distribute_batch` tests.
fn make_decision(path: &str, source: &str, targets: &[&str]) -> SyncDecision {
    SyncDecision {
        path: path.to_string(),
        source_host: source.to_string(),
        target_hosts: targets.iter().map(|s| s.to_string()).collect(),
        synced_hosts: vec![],
        reason: "test decision".to_string(),
    }
}

/// `decide_batch` with conflicting mtimes: host-b has the newest mtime and
/// becomes the source. Asserts the audit's "newest-wins" conflict strategy.
#[tokio::test]
async fn decide_batch_newest_wins_with_mock_exec() {
    let ctx = build_ctx();
    let hosts = vec![sh_host("host-a"), sh_host("host-b")];
    let sessions: Arc<dyn SessionPool> = Arc::new(
        MockSessionPool::new(vec!["host-a".into(), "host-b".into()])
            .with_exec(
                "host-a",
                "---FILE:",
                sh_batch_output("$HOME/.bashrc", 1000, 100, "hash-a"),
            )
            .with_exec(
                "host-b",
                "---FILE:",
                sh_batch_output("$HOME/.bashrc", 3000, 100, "hash-b"),
            ),
    );
    let path_source_map: HashMap<String, Option<&str>> = HashMap::new();
    let mut summary = SyncSummary::default();

    let decisions = decide_batch(
        &ctx,
        &hosts,
        &["~/.bashrc".to_string()],
        &path_source_map,
        &None,
        &sessions,
        None,
        true,
        false,
        &mut summary,
    )
    .await
    .unwrap();

    assert_eq!(decisions.len(), 1);
    let d = &decisions[0];
    assert_eq!(
        d.source_host, "host-b",
        "host-b (newer mtime) must be selected as source"
    );
    assert_eq!(d.target_hosts, vec!["host-a".to_string()]);
    assert!(d.reason.contains("3000"));
}

/// Same hash on all hosts → no decisions (file is in sync).
#[tokio::test]
async fn decide_batch_all_in_sync_with_mock_exec() {
    let ctx = build_ctx();
    let hosts = vec![sh_host("host-a"), sh_host("host-b")];
    let sessions: Arc<dyn SessionPool> = Arc::new(
        MockSessionPool::new(vec!["host-a".into(), "host-b".into()])
            .with_exec(
                "host-a",
                "---FILE:",
                sh_batch_output("$HOME/.bashrc", 1000, 100, "same-hash"),
            )
            .with_exec(
                "host-b",
                "---FILE:",
                sh_batch_output("$HOME/.bashrc", 1000, 100, "same-hash"),
            ),
    );
    let path_source_map: HashMap<String, Option<&str>> = HashMap::new();
    let mut summary = SyncSummary::default();

    let decisions = decide_batch(
        &ctx,
        &hosts,
        &["~/.bashrc".to_string()],
        &path_source_map,
        &None,
        &sessions,
        None,
        true,
        false,
        &mut summary,
    )
    .await
    .unwrap();

    assert!(decisions.is_empty(), "in-sync file → no decisions");
}

/// `decide_batch` with no reachable hosts → empty (no IO attempted).
#[tokio::test]
async fn decide_batch_no_reachable_hosts_returns_empty() {
    let ctx = build_ctx();
    let hosts: Vec<Arc<HostEntry>> = vec![];
    let sessions: Arc<dyn SessionPool> = Arc::new(MockSessionPool::new(vec![]));
    let path_source_map: HashMap<String, Option<&str>> = HashMap::new();
    let mut summary = SyncSummary::default();

    let decisions = decide_batch(
        &ctx,
        &hosts,
        &["~/.bashrc".to_string()],
        &path_source_map,
        &None,
        &sessions,
        None,
        true,
        false,
        &mut summary,
    )
    .await
    .unwrap();

    assert!(decisions.is_empty());
}

/// `distribute_batch` happy path: 1 decision with one target; mock records
/// the download (from source) and upload (to target). Asserts both recorded.
#[tokio::test]
async fn distribute_batch_happy_path_records_upload_and_download() {
    let ctx = build_ctx();
    let hosts = vec![sh_host("host-src"), sh_host("host-dst")];
    let mock = Arc::new(
        MockSessionPool::new(vec!["host-src".into(), "host-dst".into()]).with_download_bytes(
            "host-src",
            "~/.bashrc",
            b"file-bytes".to_vec(),
        ),
    );
    let sessions: Arc<dyn SessionPool> = mock.clone();
    let limiter = ConcurrencyLimiter::new(10, 4, &["host-src".to_string(), "host-dst".to_string()]);
    let decisions = vec![make_decision("~/.bashrc", "host-src", &["host-dst"])];
    let mut summary = SyncSummary::default();
    let mut host_file_map: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
    host_file_map.insert("host-src".to_string(), (Vec::new(), Vec::new()));
    host_file_map.insert("host-dst".to_string(), (Vec::new(), Vec::new()));

    distribute_batch(
        &ctx,
        &hosts,
        &decisions,
        &limiter,
        &sessions,
        false, // dry_run = false → real distribute
        false,
        "test-group",
        &mut summary,
        &mut host_file_map,
    )
    .await
    .unwrap();

    // One download from source, one upload to target.
    assert_eq!(mock.downloads().len(), 1, "source download recorded");
    assert_eq!(mock.uploads().len(), 1, "target upload recorded");
    assert_eq!(mock.uploads()[0].0, "host-dst");
    assert_eq!(mock.uploads()[0].2, "~/.bashrc");

    // host_file_map updated: dst got the file in .0 (synced), src in .1 (passed-along)
    let dst_entry = &host_file_map["host-dst"];
    assert!(
        dst_entry.0.contains(&"~/.bashrc".to_string()),
        "dst .0 (synced) should contain the path"
    );
}

/// `distribute_batch` dry-run: no uploads, no downloads — just summary
/// accounting.
#[tokio::test]
async fn distribute_batch_dry_run_skips_all_io() {
    let ctx = build_ctx();
    let hosts = vec![sh_host("host-src"), sh_host("host-dst")];
    let mock = Arc::new(MockSessionPool::new(vec![
        "host-src".to_string(),
        "host-dst".to_string(),
    ]));
    let sessions: Arc<dyn SessionPool> = mock.clone();
    let limiter = ConcurrencyLimiter::new(10, 4, &["host-src".to_string(), "host-dst".to_string()]);
    let decisions = vec![make_decision("~/.bashrc", "host-src", &["host-dst"])];
    let mut summary = SyncSummary::default();
    let mut host_file_map: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();

    distribute_batch(
        &ctx,
        &hosts,
        &decisions,
        &limiter,
        &sessions,
        true, // dry_run = true → skip distribute entirely
        false,
        "test-group",
        &mut summary,
        &mut host_file_map,
    )
    .await
    .unwrap();

    assert!(mock.uploads().is_empty(), "dry-run must not upload");
    assert!(mock.downloads().is_empty(), "dry-run must not download");
}

/// `distribute_batch` records failed uploads when the mock returns an error.
#[tokio::test]
async fn distribute_batch_records_failed_upload() {
    let ctx = build_ctx();
    let hosts = vec![sh_host("host-src"), sh_host("host-dst")];
    let mock = Arc::new(
        MockSessionPool::new(vec!["host-src".into(), "host-dst".into()])
            .with_download_bytes("host-src", "~/.bashrc", b"x".to_vec())
            .with_upload_error("host-dst", "~/.bashrc", "permission denied"),
    );
    let sessions: Arc<dyn SessionPool> = mock.clone();
    let limiter = ConcurrencyLimiter::new(10, 4, &["host-src".to_string(), "host-dst".to_string()]);
    let decisions = vec![make_decision("~/.bashrc", "host-src", &["host-dst"])];
    let mut summary = SyncSummary::default();
    let mut host_file_map: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();

    distribute_batch(
        &ctx,
        &hosts,
        &decisions,
        &limiter,
        &sessions,
        false,
        false,
        "test-group",
        &mut summary,
        &mut host_file_map,
    )
    .await
    .unwrap();

    // Upload attempted but failed; download from source still happened.
    assert_eq!(mock.downloads().len(), 1);
    assert_eq!(
        mock.uploads().len(),
        1,
        "upload was attempted (then errored)"
    );
}

/// `sync_path_across` happy path with mock pool: 2 hosts, conflicting
/// mtimes → newest-wins distribute. Closes Phase D carry-over (the ~210-line
/// `sync_path_across` had zero coverage).
#[tokio::test]
async fn sync_path_across_distributes_newest_to_older() {
    use crate::commands::sync::sync_path_across;

    let ctx = build_ctx();
    let hosts = vec![sh_host("host-a"), sh_host("host-b")];
    let mock = Arc::new(
        MockSessionPool::new(vec!["host-a".into(), "host-b".into()])
            // collect_file_metadata uses the per-file sh command starting
            // with "stat " (NOT the batch ---FILE: command).
            .with_exec("host-a", "stat ", sh_per_file_output(1000, 100, "hash-a"))
            .with_exec("host-b", "stat ", sh_per_file_output(3000, 100, "hash-b"))
            // download from host-b (the selected source) writes bytes to temp.
            .with_download_bytes("host-b", "~/.bashrc", b"newer-content".to_vec()),
    );
    let sessions: Arc<dyn SessionPool> = mock.clone();
    let mut summary = SyncSummary::default();

    sync_path_across(
        &ctx,
        &hosts,
        "~/.bashrc",
        "test-group",
        false, // dry_run
        true,  // push_missing
        None,  // source_override
        sessions,
        &mut summary,
        true, // quiet
    )
    .await
    .unwrap();

    // Download from host-b (newer = source); upload to host-a.
    assert_eq!(mock.downloads().len(), 1);
    assert_eq!(mock.downloads()[0].0, "host-b");
    assert_eq!(mock.uploads().len(), 1);
    assert_eq!(mock.uploads()[0].0, "host-a");
}

/// `sync_path_across` all-in-sync: no uploads, no downloads.
#[tokio::test]
async fn sync_path_across_in_sync_no_io() {
    use crate::commands::sync::sync_path_across;

    let ctx = build_ctx();
    let hosts = vec![sh_host("host-a"), sh_host("host-b")];
    let mock = Arc::new(
        MockSessionPool::new(vec!["host-a".into(), "host-b".into()])
            .with_exec("host-a", "stat ", sh_per_file_output(1000, 100, "same"))
            .with_exec("host-b", "stat ", sh_per_file_output(1000, 100, "same")),
    );
    let sessions: Arc<dyn SessionPool> = mock.clone();
    let mut summary = SyncSummary::default();

    sync_path_across(
        &ctx,
        &hosts,
        "~/.bashrc",
        "test-group",
        false,
        true,
        None,
        sessions,
        &mut summary,
        true,
    )
    .await
    .unwrap();

    assert!(mock.uploads().is_empty());
    assert!(mock.downloads().is_empty());
}

/// `run_recursive_entries` with empty entries → no-op (early return path).
#[tokio::test]
async fn run_recursive_entries_empty_returns_noop() {
    let ctx = build_ctx();
    let hosts = vec![sh_host("host-a"), sh_host("host-b")];
    let sessions: Arc<dyn SessionPool> =
        Arc::new(MockSessionPool::new(vec!["host-a".into(), "host-b".into()]));
    let mut summary = SyncSummary::default();

    run_recursive_entries(
        &ctx,
        &hosts,
        &[],
        &sessions,
        false,
        true,
        "test-group",
        false,
        &mut summary,
    )
    .await
    .unwrap();

    assert_eq!(summary.files_synced, 0);
}

/// `sync_inner` early-return: 2 hosts in config but NO paths/names supplied
/// → "No sync paths resolved" branch (no SSH attempted).
#[tokio::test]
async fn sync_inner_no_paths_returns_empty_report() {
    let mut config = AppConfig::default();
    config
        .host
        .push(Arc::new(HostEntry::placeholder("h-a", "h-a")));
    config
        .host
        .push(Arc::new(HostEntry::placeholder("h-b", "h-b")));
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::state::db::migrate_for_test(&conn);
    let ctx = Context {
        config: Arc::new(config),
        config_path: None,
        db: crate::state::db::DbHandle::new(conn),
        timeout: 5,
        mode: crate::commands::TargetMode::All,
        serial: false,
        skip: vec![],
        verbose: false,
        auth_sender: None,
    };

    let report = sync_inner(
        &ctx,
        false,
        &[], // no positional paths
        &[], // no -n entries
        None,
        crate::commands::sync::SyncOutputStyle::Quiet,
        None,
    )
    .await
    .unwrap();

    match report {
        crate::commands::report::CommandReport::Sync(r) => {
            assert_eq!(r.mode, "config_entries");
            // No paths resolved → no work done; every host shows files_synced == 0.
            assert!(r.hosts.iter().all(|h| h.files_synced == 0));
        }
        other => panic!("expected CommandReport::Sync, got {:?}", other),
    }
}

/// `sync_inner` early-return: paths supplied but only 1 host in config →
/// "need at least 2 hosts" branch (no SSH attempted).
#[tokio::test]
async fn sync_inner_single_host_returns_at_least_2_branch() {
    let mut config = AppConfig::default();
    config
        .host
        .push(Arc::new(HostEntry::placeholder("lonely", "lonely")));
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::state::db::migrate_for_test(&conn);
    let ctx = Context {
        config: Arc::new(config),
        config_path: None,
        db: crate::state::db::DbHandle::new(conn),
        timeout: 5,
        mode: crate::commands::TargetMode::All,
        serial: false,
        skip: vec![],
        verbose: false,
        auth_sender: None,
    };

    let report = sync_inner(
        &ctx,
        false,
        &["~/.bashrc".to_string()],
        &[],
        None,
        crate::commands::sync::SyncOutputStyle::Quiet,
        None,
    )
    .await
    .unwrap();

    match report {
        crate::commands::report::CommandReport::Sync(r) => {
            // Single-host path doesn't error; it returns a Sync report with
            // no operations performed.
            assert!(!r.executed_at.is_empty(), "executed_at should be set");
            assert!(r.hosts.iter().all(|h| h.files_synced == 0));
        }
        other => panic!("expected CommandReport::Sync, got {:?}", other),
    }
}

/// `sync_inner` with 0 hosts in config + paths → `resolve_hosts` returns an
/// Err ("No hosts matched the specified filter"). This is the production
/// error path — `sync_inner` propagates it. The test pins that contract.
#[tokio::test]
async fn sync_inner_zero_hosts_errors_on_resolve() {
    let ctx = build_ctx(); // empty config
    let err = sync_inner(
        &ctx,
        false,
        &["~/.bashrc".to_string()],
        &[],
        None,
        crate::commands::sync::SyncOutputStyle::Quiet,
        None,
    )
    .await
    .unwrap_err();

    assert!(
        err.to_string().contains("No hosts matched"),
        "expected resolve_hosts error, got: {}",
        err
    );
}

/// MockSessionPool upholds Send + Sync (required by `async_trait` +
/// `tokio::spawn` paths in the sync helpers).
#[test]
fn mock_session_pool_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MockSessionPool>();
}
