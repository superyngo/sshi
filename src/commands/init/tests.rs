//! Integration tests for [`core::init_core`] driven end-to-end via a
//! [`MockSessionPool`] (audit §2.8 HIGH ×2 — closes the "0% coverage" gap).
//!
//! These tests do not touch the network: `MockSessionPool::exec` returns
//! canned responses for the three shell-detection commands issued by
//! [`host::shell::detect_russh`] (`$PSVersionTable.PSVersion.Major`,
//! `ver`, `echo ok`), so every host can be steered to a specific
//! [`ShellType`] without a live SSH server.

use std::sync::Arc;

use tempfile::TempDir;

use crate::commands::init::core::{init_core, InitPools};
use crate::commands::report::{HostStatus, ProgressSink};
use crate::commands::Context;
use crate::config::ssh_config::SshHostEntry;
use crate::host::session_pool::{RemoteOutput, SessionPool};
use crate::host::session_pool_mock::MockSessionPool;

use super::report::{InitDetectedHost, InitFailedHost, InitPlan};

/// Sink that records per-host completion events for assertion.
#[derive(Default)]
struct RecordingSink {
    events: std::sync::Mutex<Vec<(String, HostStatus, String)>>,
}

impl ProgressSink for RecordingSink {
    fn host_started(&self, _host: &str) {}
    fn host_completed(&self, host: &str, status: HostStatus, detail: &str, _ms: u64) {
        self.events
            .lock()
            .unwrap()
            .push((host.to_string(), status, detail.to_string()));
    }
}

impl RecordingSink {
    fn take(&self) -> Vec<(String, HostStatus, String)> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

fn make_remote_output(stdout: &str) -> RemoteOutput {
    RemoteOutput {
        stdout: stdout.to_string(),
        stderr: String::new(),
        exit_code: Some(0),
        success: true,
    }
}

fn build_ctx(tmp: &TempDir) -> Context {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::state::db::migrate_for_test(&conn);
    Context {
        config: Arc::new(crate::config::schema::AppConfig::default()),
        config_path: Some(tmp.path().join("config.toml")),
        db: crate::state::db::DbHandle::new(conn),
        timeout: 5,
        mode: crate::commands::TargetMode::All,
        serial: false,
        skip: vec![],
        verbose: false,
        auth_sender: None,
    }
}

fn ssh_host(name: &str) -> SshHostEntry {
    SshHostEntry {
        name: name.to_string(),
        hostname: None,
        user: None,
        port: None,
        identity_file: None,
        proxy_jump: None,
    }
}

/// Three-host happy path: sh / powershell / cmd detection via mock exec.
/// `plan.dry_run = true` so we don't write to the test's config_path.
#[tokio::test]
async fn init_core_happy_path_detects_mixed_shells() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = build_ctx(&tmp);

    let mock = MockSessionPool::new(vec!["h-sh".into(), "h-ps".into(), "h-cmd".into()])
        // h-sh: PSVersionTable not registered → exec errors → fall through;
        //       ver not registered → exec errors → fall through;
        //       echo ok registered → success → ShellType::Sh.
        .with_exec("h-sh", "echo ok", make_remote_output("ok\n"))
        // h-ps: PSVersionTable returns non-empty → ShellType::PowerShell
        //       (ver / echo ok never called).
        .with_exec("h-ps", "PSVersionTable", make_remote_output("7\n"))
        // h-cmd: PSVersionTable exec errors (no response registered) → fall
        //        through; ver returns stdout containing "Windows" →
        //        ShellType::Cmd.
        .with_exec(
            "h-cmd",
            "ver",
            make_remote_output("Microsoft Windows [Version 10.0]"),
        );

    let pools = InitPools {
        session: &mock,
        retry: None,
        auth_retry: None,
    };
    let plan = InitPlan {
        dry_run: true,
        ..Default::default()
    };
    let sink = RecordingSink::default();
    let ssh_hosts = vec![ssh_host("h-sh"), ssh_host("h-ps"), ssh_host("h-cmd")];

    let report = init_core(&ctx, &ssh_hosts, pools, &plan, Some(&sink))
        .await
        .unwrap();

    assert_eq!(report.ssh_hosts_count, 3);
    assert!(report.skipped_hosts.is_empty());
    assert!(report.stale_host_names.is_empty());
    assert!(!report.stale_hosts_removed);
    assert_eq!(report.detected_hosts.len(), 3);
    assert!(report.failed_hosts.is_empty());
    assert_eq!(report.new_hosts.len(), 3);

    let shells_by_host: std::collections::HashMap<&str, &str> = report
        .detected_hosts
        .iter()
        .map(|h| (h.host.as_str(), h.shell.as_str()))
        .collect();
    assert_eq!(shells_by_host["h-sh"], "sh");
    assert_eq!(shells_by_host["h-ps"], "powershell");
    assert_eq!(shells_by_host["h-cmd"], "cmd");

    // Dry-run: nothing persisted.
    assert!(report.persisted_path.is_none());
    assert!(report.dry_run);

    // Per-host progress events: 3 Online, 0 Error.
    let events = sink.take();
    let online = events
        .iter()
        .filter(|(_, s, _)| *s == HostStatus::Online)
        .count();
    assert_eq!(online, 3);
}

/// All hosts failed to connect: `pools.session.reachable_hosts()` is empty,
/// so `detect_shells` iterates nothing; `init_core` returns a report with
/// zero detected, zero failed (failure partition is the wrapper's job —
/// `init_core` only sees the post-filter reachable list).
#[tokio::test]
async fn init_core_all_unreachable_skips_detect() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = build_ctx(&tmp);

    // Mock with NO reachable hosts and the three hosts in `failed`.
    let mock = MockSessionPool::new(vec![])
        .with_failed_host("h-a", "connection refused")
        .with_failed_host("h-b", "host key mismatch")
        .with_failed_host("h-c", "auth failed");

    let pools = InitPools {
        session: &mock,
        retry: None,
        auth_retry: None,
    };
    let plan = InitPlan::default();
    let sink = RecordingSink::default();
    let ssh_hosts = vec![ssh_host("h-a"), ssh_host("h-b"), ssh_host("h-c")];

    let report = init_core(&ctx, &ssh_hosts, pools, &plan, Some(&sink))
        .await
        .unwrap();

    assert_eq!(report.ssh_hosts_count, 3);
    assert!(report.detected_hosts.is_empty());
    assert!(report.failed_hosts.is_empty());
    assert!(report.new_hosts.is_empty());
    // Nothing to persist (no detections, no skips, no stale removal).
    assert!(report.persisted_path.is_none());
    assert!(report.no_changes);
    // No progress events (no hosts reached detect_shells).
    assert!(sink.take().is_empty());
}

/// Mixed: 2 reachable + 1 unreachable. The unreachable host simply doesn't
/// appear in the detect loop (the CLI wrapper's failure partitioning has
/// already moved it out of `reachable_hosts` before `init_core` runs).
#[tokio::test]
async fn init_core_mixed_reachable_and_failed() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = build_ctx(&tmp);

    let mock = MockSessionPool::new(vec!["h-up1".into(), "h-up2".into()])
        .with_failed_host("h-down", "Network unreachable")
        .with_exec("h-up1", "echo ok", make_remote_output("ok\n"))
        .with_exec("h-up2", "PSVersionTable", make_remote_output("5\n"));

    let pools = InitPools {
        session: &mock,
        retry: None,
        auth_retry: None,
    };
    let plan = InitPlan {
        dry_run: true,
        ..Default::default()
    };
    let ssh_hosts = vec![ssh_host("h-up1"), ssh_host("h-up2"), ssh_host("h-down")];
    let sink = RecordingSink::default();

    let report = init_core(&ctx, &ssh_hosts, pools, &plan, Some(&sink))
        .await
        .unwrap();

    assert_eq!(report.ssh_hosts_count, 3);
    assert_eq!(report.detected_hosts.len(), 2);
    let detected_names: Vec<&str> = report
        .detected_hosts
        .iter()
        .map(|h| h.host.as_str())
        .collect();
    assert!(detected_names.contains(&"h-up1"));
    assert!(detected_names.contains(&"h-up2"));
    assert!(!detected_names.contains(&"h-down"));
    // The unreachable host doesn't appear in `failed_hosts` either — that
    // partition is the CLI wrapper's responsibility (it folds `failed_hosts`
    // from the pool into `summary.add_failure` before calling init_core).
    assert!(report.failed_hosts.is_empty());
}

/// A host skipped via `plan.skip` is counted as skipped, not detected.
#[tokio::test]
async fn init_core_skipped_hosts_excluded_from_detect() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = build_ctx(&tmp);

    let mock = MockSessionPool::new(vec!["h-keep".into(), "h-skip".into()]).with_exec(
        "h-keep",
        "echo ok",
        make_remote_output("ok\n"),
    );

    let pools = InitPools {
        session: &mock,
        retry: None,
        auth_retry: None,
    };
    let plan = InitPlan {
        dry_run: true,
        skip: vec!["h-skip".to_string()],
        ..Default::default()
    };
    let ssh_hosts = vec![ssh_host("h-keep"), ssh_host("h-skip")];

    let report = init_core(&ctx, &ssh_hosts, pools, &plan, None)
        .await
        .unwrap();

    assert_eq!(report.detected_hosts.len(), 1);
    assert_eq!(report.detected_hosts[0].host, "h-keep");
    assert_eq!(report.skipped_hosts, vec!["h-skip".to_string()]);
}

/// `plan.remove_stale_hosts = true` + a host in ctx.config that's no longer
/// in ssh_hosts → report.stale_host_names populated + stale_hosts_removed true.
#[tokio::test]
async fn init_core_marks_stale_hosts_for_removal() {
    let tmp = tempfile::tempdir().unwrap();
    let config_path = tmp.path().join("config.toml");

    // Seed config with one host that's NOT in ssh_hosts (stale) and one that is.
    let mut config = crate::config::schema::AppConfig::default();
    config
        .host
        .push(Arc::new(crate::config::schema::HostEntry::placeholder(
            "stale-host",
            "stale-host",
        )));
    config
        .host
        .push(Arc::new(crate::config::schema::HostEntry::placeholder(
            "live-host",
            "live-host",
        )));
    // Write to disk so init_core's `config_exists` check returns true and
    // the stale-host detection branch fires.
    crate::config::app::save(&config, Some(config_path.as_path())).unwrap();

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::state::db::migrate_for_test(&conn);
    let ctx = Context {
        config: Arc::new(config),
        config_path: Some(config_path.clone()),
        db: crate::state::db::DbHandle::new(conn),
        timeout: 5,
        mode: crate::commands::TargetMode::All,
        serial: false,
        skip: vec![],
        verbose: false,
        auth_sender: None,
    };

    let mock = MockSessionPool::new(vec!["live-host".into()]).with_exec(
        "live-host",
        "echo ok",
        make_remote_output("ok\n"),
    );

    let pools = InitPools {
        session: &mock,
        retry: None,
        auth_retry: None,
    };
    let plan = InitPlan {
        dry_run: true,
        remove_stale_hosts: true,
        ..Default::default()
    };
    let ssh_hosts = vec![ssh_host("live-host")];

    let report = init_core(&ctx, &ssh_hosts, pools, &plan, None)
        .await
        .unwrap();

    assert_eq!(report.stale_host_names, vec!["stale-host".to_string()]);
    assert!(report.stale_hosts_removed);
}

/// When `detect_russh` fails for a reachable host (e.g. all exec calls
/// error), the host lands in `failed_hosts` with the shell-detect error.
#[tokio::test]
async fn init_core_detect_failure_recorded_in_failed_hosts() {
    let tmp = tempfile::tempdir().unwrap();
    let ctx = build_ctx(&tmp);

    // Mock claims host is reachable, but registers no exec responses — so
    // every detect_russh exec call errors and detect_russh bails.
    let mock = MockSessionPool::new(vec!["h-broken".into()]);

    let pools = InitPools {
        session: &mock,
        retry: None,
        auth_retry: None,
    };
    let plan = InitPlan {
        dry_run: true,
        ..Default::default()
    };
    let ssh_hosts = vec![ssh_host("h-broken")];

    let report = init_core(&ctx, &ssh_hosts, pools, &plan, None)
        .await
        .unwrap();

    assert!(report.detected_hosts.is_empty());
    assert_eq!(report.failed_hosts.len(), 1);
    assert_eq!(report.failed_hosts[0].host, "h-broken");
    assert!(
        report.failed_hosts[0]
            .detail
            .contains("shell detection failed"),
        "detail should mention shell detection failure, got: {}",
        report.failed_hosts[0].detail
    );
}

/// Smoke test of the InitReport shape when fully populated.
#[test]
fn init_report_round_trip_serializes_all_fields() {
    let report = super::report::InitReport {
        executed_at: "2026-07-20T00:00:00Z".to_string(),
        ssh_hosts_count: 2,
        skipped_hosts: vec!["skip1".to_string()],
        detected_hosts: vec![InitDetectedHost {
            host: "h".to_string(),
            shell: "sh".to_string(),
        }],
        stale_host_names: vec!["old".to_string()],
        stale_hosts_removed: true,
        failed_hosts: vec![InitFailedHost {
            host: "bad".to_string(),
            detail: "timeout".to_string(),
        }],
        summary_should_print: true,
        no_changes: false,
        persisted_path: None,
        dry_run: false,
        summary: crate::output::summary::Summary::default(),
        new_hosts: vec![],
    };
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["ssh_hosts_count"], 2);
    assert_eq!(json["stale_hosts_removed"], true);
    assert_eq!(json["skipped_hosts"][0], "skip1");
}

#[tokio::test]
async fn mock_session_pool_trait_object_compiles_and_runs() {
    // Confirms `&dyn SessionPool` is usable from InitPools — i.e. the trait
    // dispatch design works end-to-end with the mock as the trait object.
    let mock = MockSessionPool::new(vec!["x".into()]);
    let _pools = InitPools {
        session: &mock as &dyn SessionPool,
        retry: None,
        auth_retry: None,
    };
    // Also confirm Send + Sync bounds required by async_trait hold.
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MockSessionPool>();
}

/// Ensures InitPools accepts a `&dyn SessionPool` constructed from a real
/// `RusshSessionPool` (the production path) — i.e. the coercion from
/// `&RusshSessionPool` to `&dyn SessionPool` works in struct initialisation.
#[tokio::test]
async fn init_pools_accepts_real_russh_pool_via_coercion() {
    use crate::host::session_pool::RusshSessionPool;
    let pool = RusshSessionPool::setup(&[], 5, 1, None).await.unwrap();
    let _pools = InitPools {
        session: &pool,
        retry: None,
        auth_retry: None,
    };
    pool.shutdown().await;
}
