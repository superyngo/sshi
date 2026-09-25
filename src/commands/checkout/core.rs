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

    let snapshot_sql = latest_snapshot_sql(&placeholders);
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

/// SQL for the newest snapshot of *each* host in `placeholders`.
fn latest_snapshot_sql(placeholders: &str) -> String {
    format!(
        "SELECT host, collected_at, online, raw_json FROM ( \
           SELECT host, collected_at, online, raw_json, \
                  ROW_NUMBER() OVER (PARTITION BY host ORDER BY collected_at DESC, id DESC) AS rn \
           FROM check_snapshots WHERE host IN ({placeholders}) \
         ) WHERE rn = 1 \
         ORDER BY host"
    )
}

/// SQL for the newest `?{limit_param}` snapshots of *each* host in
/// `placeholders` (a global `LIMIT` would let one busy host starve the rest).
fn combined_snapshot_sql(placeholders: &str, limit_param: usize) -> String {
    format!(
        "SELECT host, collected_at, online, raw_json FROM ( \
           SELECT host, collected_at, online, raw_json, \
                  ROW_NUMBER() OVER (PARTITION BY host ORDER BY collected_at DESC) AS rn \
           FROM check_snapshots WHERE host IN ({placeholders}) \
         ) WHERE rn <= ?{limit_param} \
         ORDER BY host, collected_at DESC"
    )
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

    let snapshot_sql = combined_snapshot_sql(&placeholders, host_names.len() + 1);
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

#[cfg(test)]
mod snapshot_sql_tests {
    use super::{combined_snapshot_sql, fetch_latest_snapshots, latest_snapshot_sql};
    use crate::commands::Context;
    use crate::config::schema::AppConfig;
    use std::sync::Arc;

    #[test]
    fn lookback_applies_per_host_not_globally() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::state::db::migrate_for_test(&conn);
        for i in 0..60 {
            conn.execute(
                "INSERT INTO check_snapshots(host, collected_at, online, raw_json) VALUES ('server1', ?1, 1, '{}')",
                [1000 - i],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO check_snapshots(host, collected_at, online, raw_json) VALUES ('server2', 500, 1, '{}')",
            [],
        )
        .unwrap();
        let sql = combined_snapshot_sql("?1,?2", 3);
        let mut stmt = conn.prepare(&sql).unwrap();
        let hosts: Vec<String> = stmt
            .query_map(rusqlite::params!["server1", "server2", 50i64], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(hosts.iter().filter(|h| *h == "server1").count(), 50);
        assert_eq!(hosts.iter().filter(|h| *h == "server2").count(), 1);
    }

    #[test]
    fn latest_snapshot_sql_returns_one_row_per_host() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::state::db::migrate_for_test(&conn);
        for i in 0..10 {
            let json = format!("{{\"idx\":{}}}", 1000 + i);
            conn.execute(
                "INSERT INTO check_snapshots(host, collected_at, online, raw_json) VALUES ('server1', ?1, 1, ?2)",
                rusqlite::params![1000 + i, json],
            )
            .unwrap();
        }
        for i in 0..5 {
            let json = format!("{{\"idx\":{}}}", 2000 + i);
            conn.execute(
                "INSERT INTO check_snapshots(host, collected_at, online, raw_json) VALUES ('server2', ?1, 1, ?2)",
                rusqlite::params![2000 + i, json],
            )
            .unwrap();
        }
        let sql = latest_snapshot_sql("?1,?2");
        let mut stmt = conn.prepare(&sql).unwrap();
        let rows: Vec<(String, i64)> = stmt
            .query_map(rusqlite::params!["server1", "server2"], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        // Query returns strictly 1 row per host from SQL, not all 15 history rows.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], ("server1".to_string(), 1009));
        assert_eq!(rows[1], ("server2".to_string(), 2004));
    }

    #[test]
    fn fetch_latest_snapshots_with_history_and_missing_host() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::state::db::migrate_for_test(&conn);
        conn.execute(
            "INSERT INTO host_last_seen(host, last_seen, last_online) VALUES ('h1', 1050, 1050), ('h2', 2050, 2050)",
            [],
        )
        .unwrap();
        for i in 0..5 {
            let json = format!("{{\"seq\":{}}}", 1000 + i);
            conn.execute(
                "INSERT INTO check_snapshots(host, collected_at, online, raw_json) VALUES ('h1', ?1, 1, ?2)",
                rusqlite::params![1000 + i, json],
            )
            .unwrap();
        }
        for i in 0..3 {
            let json = format!("{{\"seq\":{}}}", 2000 + i);
            conn.execute(
                "INSERT INTO check_snapshots(host, collected_at, online, raw_json) VALUES ('h2', ?1, 1, ?2)",
                rusqlite::params![2000 + i, json],
            )
            .unwrap();
        }
        let ctx = Context {
            config: Arc::new(AppConfig::default()),
            config_path: None,
            db: crate::state::db::DbHandle::new(conn),
            timeout: 30,
            mode: crate::commands::TargetMode::All,
            serial: false,
            skip: vec![],
            verbose: false,
            auth_sender: None,
        };
        let snapshots = fetch_latest_snapshots(&ctx, &["h1", "h2", "h3"]).unwrap();
        assert_eq!(snapshots.len(), 3);
        assert_eq!(snapshots[0].host, "h1");
        assert_eq!(snapshots[0].collected_at, 1004);
        assert!(snapshots[0].online);
        assert_eq!(snapshots[0].last_online, 1050);
        assert_eq!(snapshots[0].data["seq"], 1004);

        assert_eq!(snapshots[1].host, "h2");
        assert_eq!(snapshots[1].collected_at, 2002);
        assert!(snapshots[1].online);
        assert_eq!(snapshots[1].last_online, 2050);
        assert_eq!(snapshots[1].data["seq"], 2002);

        assert_eq!(snapshots[2].host, "h3");
        assert_eq!(snapshots[2].collected_at, 0);
        assert!(!snapshots[2].online);
        assert_eq!(snapshots[2].last_online, 0);
        assert!(snapshots[2].data.is_null());
    }
}
