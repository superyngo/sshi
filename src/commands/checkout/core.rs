//! Pure command core for `sshi checkout`.
//!
//! Splits the legacy `checkout::run` body (audit §2.2 HIGH) into a
//! non-interactive [`checkout_core`] that reads snapshots from the DB
//! and returns a typed [`CheckoutReport`]. The CLI wrapper module
//! ([`super`]) formats and prints; the TUI (Phase E) reads the same
//! report.
//!
//! [`HostSnapshot`], [`DisplayColumns`], [`fetch_latest_snapshots`], and
//! [`fetch_combined_snapshots`] remain `pub(crate)` and are re-exported
//! from [`super`] so existing TUI imports (`crate::commands::checkout::*`)
//! keep resolving.

use std::collections::HashSet;

use anyhow::Result;
use serde::Serialize;

use crate::commands::Context;

use super::report::CheckoutReport;

/// Snapshot row from the database.
#[derive(Clone, Debug, Serialize)]
pub struct HostSnapshot {
    pub(crate) host: String,
    pub(crate) collected_at: i64,
    pub(crate) online: bool,
    pub(crate) data: serde_json::Value,
    /// When the host was last confirmed online (from host_last_seen table).
    pub(crate) last_online: i64,
}

/// Columns to display, derived from enabled metrics in applicable check entries.
pub struct DisplayColumns {
    pub(crate) metrics: Vec<String>,
}

impl DisplayColumns {
    pub(crate) fn from_context(ctx: &Context) -> Self {
        let mut metrics: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for entry in &ctx.config.check {
            for m in &entry.enabled {
                if seen.insert(m.clone()) && m != "online" {
                    metrics.push(m.clone());
                }
            }
        }
        Self { metrics }
    }
}

/// Pure command core: resolves targets, reads the latest (or combined-view)
/// snapshots from the DB, and returns a typed [`CheckoutReport`]. No
/// `println!`, no `printer::*`, no `stdin`.
pub(crate) fn checkout_core(ctx: &Context, combined_view: bool) -> Result<CheckoutReport> {
    let executed_at = chrono::Utc::now().to_rfc3339();
    let hosts = ctx.resolve_hosts()?;
    let host_names: Vec<&str> = hosts.iter().map(|h| h.name.as_str()).collect();
    let columns = DisplayColumns::from_context(ctx);
    let targets: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();

    let snapshots = if combined_view {
        fetch_combined_snapshots(ctx, &host_names, &columns.metrics)?
    } else {
        fetch_latest_snapshots(ctx, &host_names)?
    };

    Ok(CheckoutReport {
        executed_at,
        combined_view,
        targets,
        columns: columns.metrics,
        hosts: snapshots,
    })
}

/// Fetch the latest snapshot for each host using batch queries.
pub(crate) fn fetch_latest_snapshots(
    ctx: &Context,
    host_names: &[&str],
) -> Result<Vec<HostSnapshot>> {
    if host_names.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: String = host_names
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(",");

    let snapshot_sql = format!(
        "SELECT host, collected_at, online, raw_json \
         FROM check_snapshots WHERE host IN ({}) \
         ORDER BY host, collected_at DESC",
        placeholders
    );
    let last_seen_sql = format!(
        "SELECT host, last_online FROM host_last_seen WHERE host IN ({})",
        placeholders
    );

    let params: Vec<&dyn rusqlite::types::ToSql> = host_names
        .iter()
        .map(|h| h as &dyn rusqlite::types::ToSql)
        .collect();

    let last_online_map: std::collections::HashMap<String, i64> =
        ctx.db.with_conn(|conn| -> Result<_> {
            let mut stmt = conn.prepare_cached(&last_seen_sql)?;
            let mut rows = stmt.query(params.as_slice())?;
            let mut map = std::collections::HashMap::new();
            while let Some(row) = rows.next()? {
                let host: String = row.get(0)?;
                let ts: i64 = row.get(1)?;
                map.insert(host, ts);
            }
            Ok(map)
        })?;

    let snapshot_rows: std::collections::HashMap<String, (i64, bool, String)> =
        ctx.db.with_conn(|conn| -> Result<_> {
            let mut stmt = conn.prepare_cached(&snapshot_sql)?;
            let mut rows = stmt.query(params.as_slice())?;
            let mut map = std::collections::HashMap::new();
            while let Some(row) = rows.next()? {
                let host: String = row.get(0)?;
                let ts: i64 = row.get(1)?;
                let online: bool = row.get(2)?;
                let json_str: String = row.get(3)?;
                map.entry(host).or_insert((ts, online, json_str));
            }
            Ok(map)
        })?;

    let mut snapshots = Vec::new();
    for host in host_names {
        let last_online = last_online_map.get(*host).copied().unwrap_or(0);
        match snapshot_rows.get(*host) {
            Some((ts, online, json_str)) => {
                let data = serde_json::from_str(json_str).unwrap_or(serde_json::Value::Null);
                snapshots.push(HostSnapshot {
                    host: host.to_string(),
                    collected_at: *ts,
                    online: *online,
                    data,
                    last_online,
                });
            }
            None => {
                snapshots.push(HostSnapshot {
                    host: host.to_string(),
                    collected_at: 0,
                    online: false,
                    data: serde_json::Value::Null,
                    last_online,
                });
            }
        }
    }
    Ok(snapshots)
}

/// Per-metric combined snapshot: for each host and each metric, find the most
/// recent snapshot that has a non-null value for that metric, then assemble a
/// synthetic `HostSnapshot` from those best-available values.
///
/// Looks back through at most `LOOKBACK` snapshots per host so that the scan
/// stays O(hosts × LOOKBACK) and doesn't read the entire history.
pub(crate) fn fetch_combined_snapshots(
    ctx: &Context,
    host_names: &[&str],
    metrics: &[String],
) -> Result<Vec<HostSnapshot>> {
    const LOOKBACK: i64 = 50;
    if host_names.is_empty() {
        return Ok(Vec::new());
    }

    let placeholders: String = host_names
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 1))
        .collect::<Vec<_>>()
        .join(",");

    let snapshot_sql = format!(
        "SELECT host, collected_at, online, raw_json \
         FROM check_snapshots WHERE host IN ({}) \
         ORDER BY host, collected_at DESC LIMIT ?{}",
        placeholders,
        host_names.len() + 1
    );
    let last_seen_sql = format!(
        "SELECT host, last_online FROM host_last_seen WHERE host IN ({})",
        placeholders
    );

    let params: Vec<&dyn rusqlite::types::ToSql> = host_names
        .iter()
        .map(|h| h as &dyn rusqlite::types::ToSql)
        .collect();

    let last_online_map: std::collections::HashMap<String, i64> =
        ctx.db.with_conn(|conn| -> Result<_> {
            let mut stmt = conn.prepare_cached(&last_seen_sql)?;
            let mut rows = stmt.query(params.as_slice())?;
            let mut map = std::collections::HashMap::new();
            while let Some(row) = rows.next()? {
                let host: String = row.get(0)?;
                let ts: i64 = row.get(1)?;
                map.insert(host, ts);
            }
            Ok(map)
        })?;

    let per_host: std::collections::HashMap<String, Vec<(i64, bool, serde_json::Value)>> =
        ctx.db.with_conn(|conn| -> Result<_> {
            let mut stmt = conn.prepare_cached(&snapshot_sql)?;
            let mut all_params: Vec<&dyn rusqlite::types::ToSql> = params.clone();
            all_params.push(&LOOKBACK);
            let mut rows = stmt.query(all_params.as_slice())?;
            let mut map: std::collections::HashMap<String, Vec<(i64, bool, serde_json::Value)>> =
                std::collections::HashMap::new();
            while let Some(row) = rows.next()? {
                let host: String = row.get(0)?;
                let ts: i64 = row.get(1)?;
                let online: bool = row.get(2)?;
                let json_str: String = row.get(3)?;
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    map.entry(host).or_default().push((ts, online, v));
                }
            }
            Ok(map)
        })?;

    let mut snapshots = Vec::new();
    for host in host_names {
        let last_online = last_online_map.get(*host).copied().unwrap_or(0);
        let history = match per_host.get(*host) {
            Some(h) => h,
            None => {
                snapshots.push(HostSnapshot {
                    host: host.to_string(),
                    collected_at: 0,
                    online: false,
                    data: serde_json::Value::Null,
                    last_online,
                });
                continue;
            }
        };

        let (latest_ts, latest_online, _) = &history[0];

        let mut combined = serde_json::Map::new();
        for metric in metrics {
            for (_, _, data) in history {
                if let Some(val) = data.get(metric) {
                    if !val.is_null() {
                        combined.insert(metric.clone(), val.clone());
                        break;
                    }
                }
            }
        }

        snapshots.push(HostSnapshot {
            host: host.to_string(),
            collected_at: *latest_ts,
            online: *latest_online,
            data: serde_json::Value::Object(combined),
            last_online,
        });
    }
    Ok(snapshots)
}
