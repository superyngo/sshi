//! View cached host metrics from the database (checkout / dashboard).
//!
//! This is the CLI wrapper: it owns the `println!` narration, the
//! `print_table_report` rendering, and the `--out` report writing. The
//! non-interactive DB read + typed report construction live in
//! [`core::checkout_core`]. Existing TUI imports of `DisplayColumns`,
//! `HostSnapshot`, and `fetch_combined_snapshots` keep resolving via the
//! `pub(crate) use` re-exports below.

mod core;
mod report;

pub(crate) use core::{
    fetch_combined_snapshots, fetch_latest_snapshots, DisplayColumns, HostSnapshot,
};
pub use report::CheckoutReport;

use anyhow::Result;

use crate::output::report::{FilterInfo, HostResult, OperationReport, ReportSummary};

use super::Context;

use core::checkout_core;

/// Thin CLI wrapper: invokes [`checkout_core`] for the typed snapshot
/// report, then renders the table to stdout and (optionally) writes an
/// `--out` JSON/HTML report.
pub async fn run(
    ctx: &Context,
    _history: bool,
    _since: Option<String>,
    combined_view: bool,
    output: &crate::cli::OutputArgs,
) -> Result<()> {
    let report = checkout_core(ctx, combined_view)?;
    let columns = DisplayColumns {
        metrics: report.columns.clone(),
    };

    if combined_view {
        println!("(combined view — each metric shows its most recent recorded value)");
    }
    print_table_report(&report.hosts, &columns);

    if let Some(out) = &output.out {
        let report_results: Vec<HostResult> = report
            .hosts
            .iter()
            .map(|snap| {
                let collected_at_str = if snap.collected_at > 0 {
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
                        "collected_at": collected_at_str,
                        "online": snap.online,
                        "snapshot": snap.data,
                    }),
                }
            })
            .collect();

        let rep_summary = ReportSummary {
            total: report_results.len(),
            success: report_results
                .iter()
                .filter(|r| r.status == "success")
                .count(),
            failed: report_results
                .iter()
                .filter(|r| r.status == "error")
                .count(),
            skipped: 0,
        };

        let cr = OperationReport {
            executed_at: report.executed_at.clone(),
            command: "checkout".to_string(),
            filter: FilterInfo::from_mode(&ctx.mode),
            task: serde_json::json!({
                "history": _history,
                "since": _since,
            }),
            targets: report.targets.clone(),
            results: report_results,
            summary: rep_summary,
        };
        let path = crate::output::report::write_report(
            &cr,
            out,
            "checkout",
            ctx.config.settings.default_output_format.as_deref(),
        )?;
        println!("Report written to {}", path);
    }

    Ok(())
}

fn extract_cpu_load(data: &serde_json::Value) -> String {
    let v = match data.get("cpu_load") {
        Some(v) => v,
        None => return "-".to_string(),
    };

    // Sh format: {load1: f, load5: f, load15: f}
    if let Some(obj) = v.as_object() {
        for key in &["load1", "load5", "load15"] {
            if let Some(f) = obj.get(*key).and_then(|n| n.as_f64()) {
                return format!("{:.2}", f);
            }
        }
    }
    // PowerShell / raw string format: "21.18..."
    if let Some(s) = v.as_str() {
        if let Ok(f) = s.trim().parse::<f64>() {
            return format!("{:.2}", f);
        }
    }
    if let Some(f) = v.as_f64() {
        return format!("{:.2}", f);
    }
    "-".to_string()
}

fn extract_memory(data: &serde_json::Value) -> (String, bool) {
    let v = match data.get("memory") {
        Some(v) => v,
        None => return ("-".to_string(), false),
    };

    // Sh format: {total_bytes, used_bytes}
    if let (Some(total), Some(used)) = (
        v.get("total_bytes").and_then(|n| n.as_u64()),
        v.get("used_bytes").and_then(|n| n.as_u64()),
    ) {
        if total > 0 {
            let pct = used as f64 / total as f64 * 100.0;
            return (format!("{:.0}%", pct), pct > 90.0);
        }
    }
    // PowerShell format: JSON string {TotalVisibleMemorySize, FreePhysicalMemory} (KB)
    if let Some(s) = v.as_str() {
        if let Ok(p) = serde_json::from_str::<serde_json::Value>(s) {
            if let (Some(total), Some(free)) = (
                p.get("TotalVisibleMemorySize").and_then(|n| n.as_u64()),
                p.get("FreePhysicalMemory").and_then(|n| n.as_u64()),
            ) {
                if total > 0 {
                    let pct = (total - free) as f64 / total as f64 * 100.0;
                    return (format!("{:.0}%", pct), pct > 90.0);
                }
            }
        }
    }
    ("-".to_string(), false)
}

fn extract_disk(data: &serde_json::Value) -> (String, bool) {
    let v = match data.get("disk") {
        Some(v) => v,
        None => return ("-".to_string(), false),
    };

    // Sh format: [{total_bytes, used_bytes, mount}, ...]
    if let Some(arr) = v.as_array() {
        let entry = arr
            .iter()
            .find(|e| e.get("mount").and_then(|m| m.as_str()) == Some("/"))
            .or_else(|| arr.iter().find(|e| e.get("total_bytes").is_some()));
        if let Some(e) = entry {
            if let (Some(total), Some(used)) = (
                e.get("total_bytes").and_then(|n| n.as_u64()),
                e.get("used_bytes").and_then(|n| n.as_u64()),
            ) {
                if total > 0 {
                    let pct = used as f64 / total as f64 * 100.0;
                    return (format!("{:.0}%", pct), pct > 90.0);
                }
            }
        }
    }
    // PowerShell format: JSON string [{Name, Used, Free}, ...] (bytes)
    if let Some(s) = v.as_str() {
        if let Ok(p) = serde_json::from_str::<serde_json::Value>(s) {
            if let Some(e) = p.as_array().and_then(|a| a.first()) {
                if let (Some(used), Some(free)) = (
                    e.get("Used").and_then(|n| n.as_u64()),
                    e.get("Free").and_then(|n| n.as_u64()),
                ) {
                    let total = used + free;
                    if total > 0 {
                        let pct = used as f64 / total as f64 * 100.0;
                        return (format!("{:.0}%", pct), pct > 90.0);
                    }
                }
            }
        }
    }
    ("-".to_string(), false)
}

fn extract_battery(data: &serde_json::Value) -> String {
    if let Some(bat) = data.get("battery") {
        if bat.get("present").and_then(|v| v.as_bool()) == Some(false) {
            return "N/A".to_string();
        }
        if let Some(pct) = bat.get("percent").and_then(|v| v.as_u64()) {
            return format!("{}%", pct);
        }
        // present but no percent (desktop without battery info)
        return "-".to_string();
    }
    "-".to_string()
}

pub(crate) fn format_relative_time(ts: i64) -> String {
    if ts == 0 {
        return "never".to_string();
    }
    let now = chrono::Utc::now().timestamp();
    let diff = now - ts;
    if diff < 60 {
        format!("{}s ago", diff)
    } else if diff < 3600 {
        format!("{}m ago", diff / 60)
    } else if diff < 86400 {
        format!("{}h ago", diff / 3600)
    } else {
        format!("{}d ago", diff / 86400)
    }
}

fn extract_ip_address(data: &serde_json::Value) -> String {
    if let Some(v) = data.get("ip_address") {
        if let Some(s) = v.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "-".to_string()
}

/// Extract a generic metric value as a display string.
pub(crate) fn extract_metric_value(data: &serde_json::Value, metric: &str) -> (String, bool) {
    match metric {
        "cpu_load" => (extract_cpu_load(data), false),
        "memory" => extract_memory(data),
        "disk" => extract_disk(data),
        "battery" => (extract_battery(data), false),
        "ip_address" => (extract_ip_address(data), false),
        "swap" => {
            if let Some(v) = data.get("swap") {
                if let (Some(total), Some(used)) = (
                    v.get("total_bytes").and_then(|n| n.as_u64()),
                    v.get("used_bytes").and_then(|n| n.as_u64()),
                ) {
                    if total > 0 {
                        let pct = used as f64 / total as f64 * 100.0;
                        return (format!("{:.0}%", pct), pct > 90.0);
                    }
                }
            }
            ("-".to_string(), false)
        }
        "system_info" => {
            if let Some(v) = data.get("system_info") {
                if let Some(obj) = v.as_object() {
                    if let Some(uname) = obj.get("uname").and_then(|u| u.as_str()) {
                        let short: String = uname
                            .split_whitespace()
                            .take(3)
                            .collect::<Vec<_>>()
                            .join(" ");
                        return (short, false);
                    }
                }
                if let Some(s) = v.as_str() {
                    let short: String = s.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
                    return (short, false);
                }
            }
            ("-".to_string(), false)
        }
        "cpu_arch" => {
            if let Some(v) = data.get("cpu_arch") {
                if let Some(s) = v.as_str() {
                    return (s.trim().to_string(), false);
                }
            }
            ("-".to_string(), false)
        }
        "network" => ("…".to_string(), false),
        _ => {
            if let Some(v) = data.get(metric) {
                if let Some(s) = v.as_str() {
                    return (s.trim().to_string(), false);
                }
                return (v.to_string(), false);
            }
            ("-".to_string(), false)
        }
    }
}

/// Map metric name to a human-readable column header.
pub(crate) fn metric_header(metric: &str) -> &str {
    match metric {
        "cpu_load" => "CPU Load",
        "memory" => "Memory",
        "disk" => "Disk",
        "battery" => "Battery",
        "ip_address" => "IP Address",
        "swap" => "Swap",
        "system_info" => "System",
        "cpu_arch" => "Arch",
        "network" => "Network",
        _ => metric,
    }
}

/// Column width for a metric.
pub(crate) fn metric_width(metric: &str) -> usize {
    match metric {
        "ip_address" => 18,
        "system_info" => 20,
        "cpu_load" => 10,
        "memory" | "disk" | "swap" | "battery" => 8,
        "cpu_arch" => 8,
        _ => 12,
    }
}

/// Print a plain-text table report to stdout with dynamic columns.
fn print_table_report(snapshots: &[HostSnapshot], columns: &DisplayColumns) {
    // Build header
    let mut header = format!("{:<16} {:<12}", "Host", "Status");
    for metric in &columns.metrics {
        let w = metric_width(metric);
        header.push_str(&format!(" {:<width$}", metric_header(metric), width = w));
    }
    header.push_str(" Last Seen");
    println!("{}", header);
    println!("{}", "-".repeat(header.len().max(78)));

    for snap in snapshots {
        let status = if snap.online {
            "\x1b[32m✓ online\x1b[0m"
        } else {
            "\x1b[31m✗ offline\x1b[0m"
        };
        let last_seen = format_relative_time(snap.last_online);

        // Status has ANSI codes (8 extra chars), so pad wider
        let mut line = format!("{:<16} {:<20}", snap.host, status);
        for metric in &columns.metrics {
            let w = metric_width(metric);
            let (val, critical) = extract_metric_value(&snap.data, metric);
            if critical {
                line.push_str(&format!(" \x1b[31m{:<width$}\x1b[0m", val, width = w));
            } else {
                line.push_str(&format!(" {:<width$}", val, width = w));
            }
        }
        line.push_str(&format!(" {}", last_seen));
        println!("{}", line);
    }
}
