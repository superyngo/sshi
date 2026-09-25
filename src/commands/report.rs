//! Typed command-result and progress-sink contracts shared by command-core
//! functions (`check_core`, future `run_core`/`exec_core`/`sync_core`/
//! `checkout_core`) and their consumers (CLI wrapper printing to stdout,
//! TUI bridging to a `tokio::mpsc` channel).
//!
//! Per docs/spec/2026-05-06-tui-reconstruct.md AD-14 and §7.5: putting these types
//! in `commands::report` keeps `*_core` functions independent of the
//! output layer; `output::report` is a thin downstream consumer.

use crate::config::schema::{CheckEntry, SyncEntry};
use serde::Serialize;

/// Per-host outcome of a command operation.
///
/// Variants are command-agnostic so the same enum serves as a progress signal
/// for `check`, `run`, `exec`, and `sync`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HostStatus {
    /// All probes succeeded / command exited 0.
    Online,
    /// Some probes succeeded, some failed (non-fatal).
    Partial,
    /// Host responded but reported offline / command exited non-zero.
    Offline,
    /// SshPool::setup could not establish a connection.
    Unreachable,
    /// Per-host timeout fired (`ctx.timeout`).
    TimedOut,
    /// Other transport / IO error during the run.
    Error,
    /// Host was intentionally skipped (e.g. shell mismatch in `exec`).
    Skipped,
}

/// Sink for per-host progress events emitted during a command-core run.
///
/// CLI wraps this with a printer impl; the TUI's AsyncBridge wraps it
/// with a channel sender impl. `*_core` functions should never call
/// `output::printer` directly.
pub trait ProgressSink: Send + Sync {
    fn host_started(&self, host: &str);
    fn host_completed(&self, host: &str, status: HostStatus, detail: &str, ms: u64);
}

/// `ProgressSink` impl that prints per-host lines via `output::printer`.
///
/// Construct with [`default_printer_sink`], which uses the one status→kind
/// mapping shared by every command ([`host_status_to_printer_kind`], B37).
pub struct PrinterSink {
    status_kind: fn(HostStatus) -> &'static str,
}

impl PrinterSink {
    fn new(status_kind: fn(HostStatus) -> &'static str) -> Self {
        Self { status_kind }
    }
}

impl ProgressSink for PrinterSink {
    fn host_started(&self, _host: &str) {}

    fn host_completed(&self, host: &str, status: HostStatus, detail: &str, _ms: u64) {
        let kind = (self.status_kind)(status);
        crate::output::printer::print_host_line(host, kind, detail);
    }
}

/// Map `HostStatus` to `output::printer` status kind.
///
/// Under ADR 0004:
/// - Online and Partial map to "ok" (green ✓)
/// - Skipped maps to "skip" (yellow ⊘)
/// - Offline, Unreachable, TimedOut, and Error map to "error" (red ✗)
pub fn host_status_to_printer_kind(status: HostStatus) -> &'static str {
    match status {
        HostStatus::Online | HostStatus::Partial => "ok",
        HostStatus::Skipped => "skip",
        HostStatus::Offline
        | HostStatus::Unreachable
        | HostStatus::TimedOut
        | HostStatus::Error => "error",
    }
}

/// Standard progress sink mapping HostStatus to printer status kinds (B37).
pub fn default_printer_sink() -> PrinterSink {
    PrinterSink::new(host_status_to_printer_kind)
}

/// Unified mapping from HostStatus to Summary entry across all commands (B37).
///
/// Under ADR 0004:
/// - Online and Partial count as success.
/// - Skipped counts as skipped.
/// - Offline, Error, TimedOut, and Unreachable count as failure (unreachable prefix stripped).
pub fn update_summary(
    summary: &mut crate::output::summary::Summary,
    host: &str,
    status: HostStatus,
    detail: &str,
) {
    match status {
        HostStatus::Online | HostStatus::Partial => summary.add_success(),
        HostStatus::Skipped => summary.add_skip(),
        HostStatus::Offline | HostStatus::Error | HostStatus::TimedOut => {
            summary.add_failure(host, detail);
        }
        HostStatus::Unreachable => {
            let err = detail.strip_prefix("unreachable — ").unwrap_or(detail);
            summary.add_failure(host, err);
        }
    }
}

/// Map HostStatus to the operation_log status column string ("ok", "error", "skipped").
///
/// Follows ADR 0004: Online and Partial map to "ok".
pub fn host_status_to_log_status(status: HostStatus) -> &'static str {
    match status {
        HostStatus::Online | HostStatus::Partial => "ok",
        HostStatus::Skipped => "skipped",
        HostStatus::Offline
        | HostStatus::Unreachable
        | HostStatus::TimedOut
        | HostStatus::Error => "error",
    }
}

/// Shared helper to record an operation_log row (B37).
///
/// Log-write failure: the remote op already happened, so this warns and continues
/// (never aborts after hosts ran) consistently across all commands.
#[allow(clippy::too_many_arguments)]
pub async fn record_operation_log(
    db: &crate::state::db::DbHandle,
    timestamp: i64,
    command: &str,
    host: &str,
    action: &str,
    status: HostStatus,
    duration_ms: i64,
    note: Option<&str>,
    stdout: Option<&str>,
) {
    let status_str = host_status_to_log_status(status);
    if let Err(e) = db
        .execute(
            "INSERT INTO operation_log (timestamp, command, host, action, status, duration_ms, note, stdout) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            vec![
                crate::state::db::boxed_param(timestamp),
                crate::state::db::boxed_param(command.to_string()),
                crate::state::db::boxed_param(host.to_string()),
                crate::state::db::boxed_param(action.to_string()),
                crate::state::db::boxed_param(status_str.to_string()),
                crate::state::db::boxed_param(duration_ms),
                crate::state::db::boxed_param(note.map(|s| s.to_string())),
                crate::state::db::boxed_param(stdout.map(|s| s.to_string())),
            ],
        )
        .await
    {
        tracing::warn!(error = %e, host = %host, command = %command, "failed to record operation_log entry");
    }
}

/// Shared helper to record an operation_log row inside an active transaction (B37).
#[allow(clippy::too_many_arguments)]
pub fn record_operation_log_tx(
    tx: &rusqlite::Transaction<'_>,
    timestamp: i64,
    command: &str,
    host: &str,
    action: &str,
    status: HostStatus,
    duration_ms: i64,
    note: Option<&str>,
    stdout: Option<&str>,
) -> rusqlite::Result<()> {
    let status_str = host_status_to_log_status(status);
    tx.execute(
        "INSERT INTO operation_log (timestamp, command, host, action, status, duration_ms, note, stdout) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            timestamp,
            command,
            host,
            action,
            status_str,
            duration_ms,
            note,
            stdout,
        ],
    )?;
    Ok(())
}

/// Per-host typed result of a `check_core` run.
///
/// Carries a `serde_json::Value` for the dynamic metrics blob (the metrics
/// schema varies by host shell), but all top-level host attributes are
/// typed so consumers do not need to reach into `serde_json::Value` to
/// determine status, duration, or detail.
#[derive(Debug, Clone, Serialize)]
pub struct CheckHostResult {
    pub host: String,
    pub status: HostStatus,
    pub duration_ms: Option<u64>,
    /// Human-readable error / warning / summary detail.
    pub detail: String,
    pub metrics_succeeded: usize,
    pub metrics_failed: usize,
    /// Raw metrics blob (may be empty for unreachable / errored hosts).
    pub data: serde_json::Value,
    pub raw_stdout: String,
    pub raw_stderr: String,
}

/// Typed return value of `check_core`.
#[derive(Debug, Clone, Serialize)]
pub struct CheckReport {
    /// RFC 3339 timestamp captured at the start of the run.
    pub executed_at: String,
    /// Distinct metrics enabled across all matched [[check]] entries.
    pub enabled_metrics: Vec<String>,
    /// Names of all targeted hosts (reachable and unreachable).
    pub targets: Vec<String>,
    pub hosts: Vec<CheckHostResult>,
}

/// Top-level typed result enum returned by `*_core` functions.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "command", rename_all = "lowercase")]
pub enum CommandReport {
    Check(CheckReport),
    Run(RunReport),
    Exec(ExecReport),
    Sync(SyncReport),
    Cp(CpReport),
    Log(LogReport),
    List(ListReport),
}

/// Tally of per-host results for a finished multi-host command; decides the
/// process exit code (ADR 0004).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostOutcome {
    pub succeeded: usize,
    pub failed: usize,
}

impl HostOutcome {
    fn add(&mut self, status: HostStatus) {
        match status {
            HostStatus::Online | HostStatus::Partial => self.succeeded += 1,
            HostStatus::Offline
            | HostStatus::Unreachable
            | HostStatus::TimedOut
            | HostStatus::Error => self.failed += 1,
            HostStatus::Skipped => {}
        }
    }

    /// `0` no host failed, `3` some failed, `4` all failed (ADR 0004).
    pub fn exit_code(self) -> i32 {
        match (self.failed, self.succeeded) {
            (0, _) => 0,
            (_, 0) => 4,
            _ => 3,
        }
    }
}

impl CommandReport {
    /// Per-host success/failure tally; `Log` and `List` have no host outcome.
    pub fn host_outcome(&self) -> HostOutcome {
        let statuses: Vec<HostStatus> = match self {
            CommandReport::Check(r) => r.hosts.iter().map(|h| h.status).collect(),
            CommandReport::Run(r) => r.hosts.iter().map(|h| h.status).collect(),
            CommandReport::Exec(r) => r.hosts.iter().map(|h| h.status).collect(),
            CommandReport::Sync(r) => r.hosts.iter().map(|h| h.status).collect(),
            CommandReport::Cp(r) => r.hosts.iter().map(|h| h.status).collect(),
            CommandReport::Log(_) | CommandReport::List(_) => Vec::new(),
        };
        let mut outcome = HostOutcome::default();
        for s in statuses {
            outcome.add(s);
        }
        outcome
    }
}

// ── Log ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct LogQueryParams {
    pub last: usize,
    pub since: Option<String>,
    pub host: Option<String>,
    pub action: Option<String>,
    pub errors: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogHostResult {
    pub host: String,
    pub status: HostStatus,
    pub duration_ms: Option<i64>,
    pub timestamp: String,
    pub command: String,
    pub action: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogReport {
    pub executed_at: String,
    pub query_params: LogQueryParams,
    pub entries: Vec<LogHostResult>,
}

// ── List ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ListHostResult {
    pub host: String,
    pub ssh_host: String,
    pub shell: String,
    pub groups: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListReport {
    pub executed_at: String,
    pub targets: Vec<String>,
    pub hosts: Vec<ListHostResult>,
    pub checks: Vec<CheckEntry>,
    pub syncs: Vec<SyncEntry>,
}

// ── Run ──────────────────────────────────────────────────────────────────────

/// Per-host result of a `run_core` invocation.
#[derive(Debug, Clone, Serialize)]
pub struct RunHostResult {
    pub host: String,
    pub status: HostStatus,
    pub duration_ms: Option<u64>,
    /// Human-readable summary (success: truncated stdout, failure: stderr).
    pub detail: String,
    pub stdout: String,
    pub stderr: String,
}

/// Typed return value of `run_core`.
#[derive(Debug, Clone, Serialize)]
pub struct RunReport {
    pub executed_at: String,
    pub command: String,
    pub targets: Vec<String>,
    pub hosts: Vec<RunHostResult>,
}

// ── Exec ─────────────────────────────────────────────────────────────────────

/// Per-host result of an `exec_core` invocation.
#[derive(Debug, Clone, Serialize)]
pub struct ExecHostResult {
    pub host: String,
    pub status: HostStatus,
    pub duration_ms: Option<u64>,
    /// Human-readable summary.
    pub detail: String,
    pub stdout: String,
    pub stderr: String,
}

/// Typed return value of `exec_core`.
#[derive(Debug, Clone, Serialize)]
pub struct ExecReport {
    pub executed_at: String,
    pub script: String,
    pub targets: Vec<String>,
    pub hosts: Vec<ExecHostResult>,
}

// ── Cp ───────────────────────────────────────────────────────────────────────

/// Per-host result of a `cp_core` invocation.
#[derive(Debug, Clone, Serialize)]
pub struct CpHostResult {
    pub host: String,
    pub status: HostStatus,
    pub duration_ms: Option<u64>,
    /// Human-readable summary (e.g. "3 files copied" or an error).
    pub detail: String,
    pub files_copied: usize,
    pub files_failed: usize,
    pub errors: Vec<String>,
}

/// Typed return value of `cp_core`.
#[derive(Debug, Clone, Serialize)]
pub struct CpReport {
    pub executed_at: String,
    /// The local argument as given on the CLI (file / dir / wildcard).
    pub local: String,
    /// The resolved remote destination base (or `~`).
    pub remote: String,
    /// Number of planned per-file transfers (per host).
    pub planned_files: usize,
    pub targets: Vec<String>,
    pub hosts: Vec<CpHostResult>,
}

// ── Sync ─────────────────────────────────────────────────────────────────────

/// Per-host aggregated result of a `sync_core` invocation.
#[derive(Debug, Clone, Serialize)]
pub struct SyncHostResult {
    pub host: String,
    pub status: HostStatus,
    pub duration_ms: Option<u64>,
    pub detail: String,
    pub files_synced: usize,
    pub files_skipped: usize,
    /// Distributed file paths (length == `files_synced`). Carried so the
    /// `--out` report / HTML renderer can list paths, not just counts.
    pub synced_paths: Vec<String>,
    /// Already-in-sync file paths (length == `files_skipped`).
    pub skipped_paths: Vec<String>,
    pub errors: Vec<String>,
}

/// Typed return value of `sync_core`.
#[derive(Debug, Clone, Serialize)]
pub struct SyncReport {
    pub executed_at: String,
    /// "config_entries" | "adhoc"
    pub mode: String,
    pub dry_run: bool,
    pub total_files_synced: usize,
    pub total_files_skipped: usize,
    /// Configured / ad-hoc paths requested for this sync (the report `task`).
    pub paths: Vec<String>,
    pub targets: Vec<String>,
    pub hosts: Vec<SyncHostResult>,
}

#[cfg(test)]
mod host_outcome_tests {
    use super::{HostOutcome, HostStatus};

    fn outcome(statuses: &[HostStatus]) -> HostOutcome {
        let mut o = HostOutcome::default();
        for s in statuses {
            o.add(*s);
        }
        o
    }

    #[test]
    fn exit_code_follows_adr_0004() {
        use HostStatus::*;
        assert_eq!(outcome(&[]).exit_code(), 0);
        assert_eq!(outcome(&[Online, Partial, Skipped]).exit_code(), 0);
        assert_eq!(outcome(&[Online, Unreachable]).exit_code(), 3);
        assert_eq!(
            outcome(&[Unreachable, TimedOut, Error, Offline]).exit_code(),
            4
        );
        assert_eq!(outcome(&[Skipped, Offline]).exit_code(), 4);
    }

    #[test]
    fn mapping_and_summary_follows_adr_0004() {
        use super::*;
        use crate::output::summary::Summary;

        // 1. Printer kinds
        assert_eq!(host_status_to_printer_kind(HostStatus::Online), "ok");
        assert_eq!(host_status_to_printer_kind(HostStatus::Partial), "ok");
        assert_eq!(host_status_to_printer_kind(HostStatus::Skipped), "skip");
        assert_eq!(host_status_to_printer_kind(HostStatus::Offline), "error");
        assert_eq!(
            host_status_to_printer_kind(HostStatus::Unreachable),
            "error"
        );
        assert_eq!(host_status_to_printer_kind(HostStatus::TimedOut), "error");
        assert_eq!(host_status_to_printer_kind(HostStatus::Error), "error");

        // 2. Log status
        assert_eq!(host_status_to_log_status(HostStatus::Online), "ok");
        assert_eq!(host_status_to_log_status(HostStatus::Partial), "ok");
        assert_eq!(host_status_to_log_status(HostStatus::Skipped), "skipped");
        assert_eq!(host_status_to_log_status(HostStatus::Offline), "error");
        assert_eq!(host_status_to_log_status(HostStatus::Unreachable), "error");
        assert_eq!(host_status_to_log_status(HostStatus::TimedOut), "error");
        assert_eq!(host_status_to_log_status(HostStatus::Error), "error");

        // 3. Update summary
        let mut summary = Summary::default();
        update_summary(&mut summary, "h1", HostStatus::Online, "ok");
        update_summary(&mut summary, "h2", HostStatus::Partial, "partial warning");
        update_summary(&mut summary, "h3", HostStatus::Skipped, "skipped");
        update_summary(&mut summary, "h4", HostStatus::Offline, "offline");
        update_summary(
            &mut summary,
            "h5",
            HostStatus::Unreachable,
            "unreachable — connection refused",
        );
        update_summary(&mut summary, "h6", HostStatus::TimedOut, "timed out");
        update_summary(&mut summary, "h7", HostStatus::Error, "error");

        assert_eq!(summary.succeeded, 2); // h1 (Online) + h2 (Partial)
        assert_eq!(summary.skipped, 1); // h3
        assert_eq!(summary.failed, 4); // h4, h5, h6, h7
        assert_eq!(summary.errors.len(), 4);
        // Verify unreachable prefix stripping
        assert_eq!(summary.errors[1].message, "connection refused");
    }

    #[tokio::test]
    async fn record_operation_log_writes_and_does_not_abort_on_db_error() {
        use super::*;
        use crate::state::db::{migrate_for_test, DbHandle};

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        migrate_for_test(&conn);
        let db = DbHandle::new(conn);

        // Successful write
        record_operation_log(
            &db,
            1717000000,
            "run",
            "h1",
            "echo 1",
            HostStatus::Online,
            100,
            None,
            Some("1"),
        )
        .await;

        // Verify row was written
        let count: i64 = db
            .transaction(|tx| {
                let c: i64 =
                    tx.query_row("SELECT COUNT(*) FROM operation_log", [], |r| r.get(0))?;
                Ok(c)
            })
            .await
            .unwrap();
        assert_eq!(count, 1);

        // Now break the database by dropping the table
        db.execute("DROP TABLE operation_log", vec![])
            .await
            .unwrap();

        // Calling record_operation_log must NOT abort or panic (warn and continue)
        record_operation_log(
            &db,
            1717000001,
            "run",
            "h2",
            "echo 2",
            HostStatus::Error,
            200,
            Some("failed"),
            None,
        )
        .await;
    }
}
