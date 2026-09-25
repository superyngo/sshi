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

#[cfg(feature = "tui")]
pub(crate) use core::{fetch_combined_snapshots, fetch_latest_snapshots};
pub(crate) use core::{DisplayColumns, HostSnapshot};
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
            task: serde_json::json!({}),
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
        // PowerShell format: JSON string or empty (desktop without battery)
        if let Some(s) = bat.as_str() {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return "N/A".to_string();
            }
            if let Ok(p) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if let Some(pct) = p.get("EstimatedChargeRemaining").and_then(|v| v.as_u64()) {
                    return format!("{}%", pct);
                }
            }
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
                    if let Ok(p) = serde_json::from_str::<serde_json::Value>(s) {
                        if let Some(os) = p.get("OsName").and_then(|u| u.as_str()) {
                            let short: String =
                                os.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
                            return (short, false);
                        }
                    }
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

    let color = crate::output::printer::should_color();
    for snap in snapshots {
        let (status, status_width) = if color {
            if snap.online {
                ("\x1b[32m✓ online\x1b[0m", 20)
            } else {
                ("\x1b[31m✗ offline\x1b[0m", 20)
            }
        } else if snap.online {
            ("✓ online", 12)
        } else {
            ("✗ offline", 12)
        };
        let last_seen = format_relative_time(snap.last_online);

        let mut line = format!("{:<16} {:<width$}", snap.host, status, width = status_width);
        for metric in &columns.metrics {
            let w = metric_width(metric);
            let (val, critical) = extract_metric_value(&snap.data, metric);
            if critical && color {
                line.push_str(&format!(" \x1b[31m{:<width$}\x1b[0m", val, width = w));
            } else {
                line.push_str(&format!(" {:<width$}", val, width = w));
            }
        }
        line.push_str(&format!(" {}", last_seen));
        println!("{}", line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_extract_cpu_load() {
        // sh sample: object with load1, load5, load15
        let data = json!({ "cpu_load": { "load1": 1.25, "load5": 0.85, "load15": 0.55 } });
        assert_eq!(
            extract_metric_value(&data, "cpu_load"),
            ("1.25".to_string(), false)
        );

        // PowerShell sample: string representation of float
        let data = json!({ "cpu_load": "21.18" });
        assert_eq!(
            extract_metric_value(&data, "cpu_load"),
            ("21.18".to_string(), false)
        );

        // Raw float sample
        let data = json!({ "cpu_load": 2.75 });
        assert_eq!(
            extract_metric_value(&data, "cpu_load"),
            ("2.75".to_string(), false)
        );

        // Fallbacks: empty, null, invalid string
        assert_eq!(
            extract_metric_value(&json!({}), "cpu_load"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({ "cpu_load": null }), "cpu_load"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({ "cpu_load": "not_a_number" }), "cpu_load"),
            ("-".to_string(), false)
        );
    }

    #[test]
    fn test_extract_memory() {
        // sh sample: normal usage (< 90%)
        let data =
            json!({ "memory": { "total_bytes": 16000000000u64, "used_bytes": 8000000000u64 } });
        assert_eq!(
            extract_metric_value(&data, "memory"),
            ("50%".to_string(), false)
        );

        // sh sample: critical usage (> 90%)
        let data = json!({ "memory": { "total_bytes": 10000u64, "used_bytes": 9500u64 } });
        assert_eq!(
            extract_metric_value(&data, "memory"),
            ("95%".to_string(), true)
        );

        // PowerShell sample: JSON string with TotalVisibleMemorySize and FreePhysicalMemory (KB)
        let data = json!({ "memory": r#"{"TotalVisibleMemorySize": 16777216, "FreePhysicalMemory": 8388608}"# });
        assert_eq!(
            extract_metric_value(&data, "memory"),
            ("50%".to_string(), false)
        );

        // PowerShell sample: critical usage (> 90%)
        let data =
            json!({ "memory": r#"{"TotalVisibleMemorySize": 10000, "FreePhysicalMemory": 500}"# });
        assert_eq!(
            extract_metric_value(&data, "memory"),
            ("95%".to_string(), true)
        );

        // Fallbacks: total 0, invalid json, null, missing
        let zero_total = json!({ "memory": { "total_bytes": 0, "used_bytes": 0 } });
        assert_eq!(
            extract_metric_value(&zero_total, "memory"),
            ("-".to_string(), false)
        );
        let bad_json = json!({ "memory": "not_json" });
        assert_eq!(
            extract_metric_value(&bad_json, "memory"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({ "memory": null }), "memory"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({}), "memory"),
            ("-".to_string(), false)
        );
    }

    #[test]
    fn test_extract_disk() {
        // sh sample: array of mounts, prioritizes root "/"
        let data = json!({
            "disk": [
                { "mount": "/boot", "total_bytes": 1000000, "used_bytes": 990000 },
                { "mount": "/", "total_bytes": 100000000u64, "used_bytes": 40000000u64 }
            ]
        });
        assert_eq!(
            extract_metric_value(&data, "disk"),
            ("40%".to_string(), false)
        );

        // sh sample: critical usage (> 90%)
        let data = json!({
            "disk": [{ "mount": "/", "total_bytes": 100000u64, "used_bytes": 95000u64 }]
        });
        assert_eq!(
            extract_metric_value(&data, "disk"),
            ("95%".to_string(), true)
        );

        // sh sample: non-root fallback when no "/" mount
        let data = json!({
            "disk": [{ "mount": "/data", "total_bytes": 200u64, "used_bytes": 100u64 }]
        });
        assert_eq!(
            extract_metric_value(&data, "disk"),
            ("50%".to_string(), false)
        );

        // PowerShell sample: JSON string array of drives
        let data = json!({
            "disk": r#"[{"Name": "C", "Used": 40000000, "Free": 60000000}]"#
        });
        assert_eq!(
            extract_metric_value(&data, "disk"),
            ("40%".to_string(), false)
        );

        // PowerShell sample: critical usage (> 90%)
        let data = json!({
            "disk": r#"[{"Name": "C", "Used": 95000, "Free": 5000}]"#
        });
        assert_eq!(
            extract_metric_value(&data, "disk"),
            ("95%".to_string(), true)
        );

        // Fallbacks: empty array, invalid json, null, missing
        assert_eq!(
            extract_metric_value(&json!({ "disk": [] }), "disk"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({ "disk": "invalid" }), "disk"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({ "disk": null }), "disk"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({}), "disk"),
            ("-".to_string(), false)
        );
    }

    #[test]
    fn test_extract_battery() {
        // sh sample: battery present with percent
        let data = json!({ "battery": { "present": true, "percent": 88 } });
        assert_eq!(
            extract_metric_value(&data, "battery"),
            ("88%".to_string(), false)
        );

        // sh sample: battery not present (desktop)
        let data = json!({ "battery": { "present": false } });
        assert_eq!(
            extract_metric_value(&data, "battery"),
            ("N/A".to_string(), false)
        );

        // sh sample: present without percent
        let data = json!({ "battery": { "present": true } });
        assert_eq!(
            extract_metric_value(&data, "battery"),
            ("-".to_string(), false)
        );

        // PowerShell sample: JSON string with EstimatedChargeRemaining
        let data = json!({ "battery": r#"{"EstimatedChargeRemaining": 85, "BatteryStatus": 2}"# });
        assert_eq!(
            extract_metric_value(&data, "battery"),
            ("85%".to_string(), false)
        );

        // PowerShell sample: empty string for desktop
        let data = json!({ "battery": "   " });
        assert_eq!(
            extract_metric_value(&data, "battery"),
            ("N/A".to_string(), false)
        );

        // Fallback: missing or null
        assert_eq!(
            extract_metric_value(&json!({}), "battery"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({ "battery": null }), "battery"),
            ("-".to_string(), false)
        );
    }

    #[test]
    fn test_extract_ip_address() {
        // sh sample
        let data = json!({ "ip_address": "192.168.1.50" });
        assert_eq!(
            extract_metric_value(&data, "ip_address"),
            ("192.168.1.50".to_string(), false)
        );

        // PowerShell sample: space-separated IPs with whitespace
        let data = json!({ "ip_address": " 192.168.1.50 10.0.0.5 \n" });
        assert_eq!(
            extract_metric_value(&data, "ip_address"),
            ("192.168.1.50 10.0.0.5".to_string(), false)
        );

        // Fallback: empty, null, missing
        assert_eq!(
            extract_metric_value(&json!({ "ip_address": "" }), "ip_address"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({ "ip_address": "   " }), "ip_address"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({ "ip_address": null }), "ip_address"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({}), "ip_address"),
            ("-".to_string(), false)
        );
    }

    #[test]
    fn test_extract_swap() {
        // sh sample: normal usage
        let data = json!({ "swap": { "total_bytes": 2048000u64, "used_bytes": 512000u64 } });
        assert_eq!(
            extract_metric_value(&data, "swap"),
            ("25%".to_string(), false)
        );

        // sh sample: critical usage (> 90%)
        let data = json!({ "swap": { "total_bytes": 1000u64, "used_bytes": 950u64 } });
        assert_eq!(
            extract_metric_value(&data, "swap"),
            ("95%".to_string(), true)
        );

        // Fallback: total 0, null, missing
        let zero = json!({ "swap": { "total_bytes": 0, "used_bytes": 0 } });
        assert_eq!(
            extract_metric_value(&zero, "swap"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({ "swap": null }), "swap"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({}), "swap"),
            ("-".to_string(), false)
        );
    }

    #[test]
    fn test_extract_system_info() {
        // sh sample: uname in object
        let data = json!({
            "system_info": { "uname": "Linux debian 5.10.0-8-amd64 #1 SMP PREEMPT Debian 5.10.46-4 (2021-08-03)" }
        });
        assert_eq!(
            extract_metric_value(&data, "system_info"),
            ("Linux debian 5.10.0-8-amd64".to_string(), false)
        );

        // PowerShell sample: JSON string with OsName
        let data = json!({
            "system_info": r#"{"OsName": "Microsoft Windows 11 Pro", "CsName": "PC", "OsVersion": "10.0.22631"}"#
        });
        assert_eq!(
            extract_metric_value(&data, "system_info"),
            ("Microsoft Windows 11".to_string(), false)
        );

        // Plain string sample
        let data = json!({ "system_info": "FreeBSD 13.0-RELEASE FreeBSD" });
        assert_eq!(
            extract_metric_value(&data, "system_info"),
            ("FreeBSD 13.0-RELEASE FreeBSD".to_string(), false)
        );

        // Fallbacks
        assert_eq!(
            extract_metric_value(&json!({ "system_info": null }), "system_info"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({}), "system_info"),
            ("-".to_string(), false)
        );
    }

    #[test]
    fn test_extract_cpu_arch() {
        // sh sample
        let data = json!({ "cpu_arch": "x86_64" });
        assert_eq!(
            extract_metric_value(&data, "cpu_arch"),
            ("x86_64".to_string(), false)
        );

        // PowerShell sample
        let data = json!({ "cpu_arch": "  AMD64 \n" });
        assert_eq!(
            extract_metric_value(&data, "cpu_arch"),
            ("AMD64".to_string(), false)
        );

        // Fallback
        assert_eq!(
            extract_metric_value(&json!({ "cpu_arch": null }), "cpu_arch"),
            ("-".to_string(), false)
        );
        assert_eq!(
            extract_metric_value(&json!({}), "cpu_arch"),
            ("-".to_string(), false)
        );
    }

    #[test]
    fn test_extract_network() {
        let data = json!({});
        assert_eq!(
            extract_metric_value(&data, "network"),
            ("…".to_string(), false)
        );
    }

    #[test]
    fn test_extract_custom_and_fallback_metric() {
        // Custom string metric
        let data = json!({ "docker_status": " active \n" });
        assert_eq!(
            extract_metric_value(&data, "docker_status"),
            ("active".to_string(), false)
        );

        // Custom numeric metric
        let data = json!({ "active_workers": 42 });
        assert_eq!(
            extract_metric_value(&data, "active_workers"),
            ("42".to_string(), false)
        );

        // Missing custom metric
        assert_eq!(
            extract_metric_value(&json!({}), "unknown_metric"),
            ("-".to_string(), false)
        );
    }

    #[test]
    fn test_metric_header_and_width() {
        assert_eq!(metric_header("cpu_load"), "CPU Load");
        assert_eq!(metric_header("memory"), "Memory");
        assert_eq!(metric_header("disk"), "Disk");
        assert_eq!(metric_header("battery"), "Battery");
        assert_eq!(metric_header("ip_address"), "IP Address");
        assert_eq!(metric_header("swap"), "Swap");
        assert_eq!(metric_header("system_info"), "System");
        assert_eq!(metric_header("cpu_arch"), "Arch");
        assert_eq!(metric_header("network"), "Network");
        assert_eq!(metric_header("custom_name"), "custom_name");

        assert_eq!(metric_width("ip_address"), 18);
        assert_eq!(metric_width("system_info"), 20);
        assert_eq!(metric_width("cpu_load"), 10);
        assert_eq!(metric_width("memory"), 8);
        assert_eq!(metric_width("disk"), 8);
        assert_eq!(metric_width("swap"), 8);
        assert_eq!(metric_width("battery"), 8);
        assert_eq!(metric_width("cpu_arch"), 8);
        assert_eq!(metric_width("other"), 12);
    }

    #[test]
    fn test_format_relative_time() {
        assert_eq!(format_relative_time(0), "never");
        let now = chrono::Utc::now().timestamp();
        assert_eq!(format_relative_time(now - 10), "10s ago");
        assert_eq!(format_relative_time(now - 120), "2m ago");
        assert_eq!(format_relative_time(now - 7200), "2h ago");
        assert_eq!(format_relative_time(now - 172800), "2d ago");
    }
}
