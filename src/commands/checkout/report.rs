//! Typed contracts for `checkout_core`: a typed
//! [`CheckoutReport`] carrying the snapshots read from the DB plus the
//! resolved display columns.
//!
//! Per audit §2.2 HIGH: `checkout_core` returns this typed struct so the
//! CLI wrapper (and future TUI Phase E popup) can format without reaching
//! into raw DB rows.

use serde::Serialize;

use super::core::HostSnapshot;
use crate::commands::TargetMode;
use crate::output::report::{FilterInfo, HostResult, OperationReport, ReportSummary};

/// Typed outcome of a `checkout_core` invocation.
#[derive(Debug, Clone, Serialize)]
pub struct CheckoutReport {
    /// RFC 3339 timestamp captured at the start of the read.
    pub executed_at: String,
    /// `true` when the caller asked for the per-metric combined view.
    pub combined_view: bool,
    /// Names of all targeted hosts (in resolve order).
    pub targets: Vec<String>,
    /// Display-column metric names (excludes `online`).
    pub columns: Vec<String>,
    /// One snapshot row per target host.
    pub hosts: Vec<HostSnapshot>,
}

/// The `--out` / export document for checkout rows: one builder for the CLI
/// (`checkout --out`) and the TUI View export, so both write the same JSON
/// for the same rows (B54).
pub fn checkout_operation_report(
    executed_at: &str,
    mode: &TargetMode,
    targets: &[String],
    hosts: &[HostSnapshot],
) -> OperationReport {
    let results: Vec<HostResult> = hosts
        .iter()
        .map(|snap| {
            let collected_at = if snap.collected_at > 0 {
                chrono::DateTime::from_timestamp(snap.collected_at, 0)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| snap.collected_at.to_string())
            } else {
                "never".to_string()
            };
            HostResult {
                host: snap.host.clone(),
                status: if snap.online { "success" } else { "error" }.to_string(),
                duration_ms: None,
                output: serde_json::json!({
                    "collected_at": collected_at,
                    "online": snap.online,
                    "snapshot": snap.data,
                }),
            }
        })
        .collect();
    let success = results.iter().filter(|r| r.status == "success").count();
    let summary = ReportSummary {
        total: results.len(),
        success,
        failed: results.len() - success,
        skipped: 0,
    };
    OperationReport {
        executed_at: executed_at.to_string(),
        command: "checkout".to_string(),
        filter: FilterInfo::from_mode(mode),
        task: serde_json::json!({}),
        targets: targets.to_vec(),
        results,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// B54: one builder for CLI and TUI checkout exports.
    #[test]
    fn checkout_operation_report_rows_and_summary() {
        let snap = |host: &str, at: i64, online: bool| HostSnapshot {
            host: host.into(),
            collected_at: at,
            online,
            data: serde_json::json!({ "memory": "x" }),
            last_online: 0,
        };
        let hosts = [snap("h1", 1_700_000_000, true), snap("h2", 0, false)];
        let targets = ["h1".to_string(), "h2".to_string()];
        let r = checkout_operation_report(
            "2026-09-25T00:00:00+00:00",
            &TargetMode::All,
            &targets,
            &hosts,
        );
        assert_eq!(r.command, "checkout");
        assert_eq!(r.task, serde_json::json!({}));
        assert_eq!(r.targets, targets);
        assert_eq!(
            (r.summary.total, r.summary.success, r.summary.failed),
            (2, 1, 1)
        );
        assert_eq!(r.results[0].status, "success");
        assert_eq!(
            r.results[0].output["collected_at"],
            "2023-11-14T22:13:20+00:00"
        );
        assert_eq!(r.results[1].output["collected_at"], "never");
        assert_eq!(r.results[1].output["snapshot"]["memory"], "x");
    }
}
