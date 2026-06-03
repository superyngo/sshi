use anyhow::Result;

use crate::cli::ActionFilter;

use super::Context;

/// One operation-log row, for stdout and the TUI View tab.
pub struct LogRow {
    pub ts: i64,
    pub command: String,
    pub host: String,
    pub action: String,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub note: Option<String>,
}

/// Query the operation log with no I/O side effects (returns rows newest-first).
pub fn log_core(
    ctx: &Context,
    last: usize,
    since: Option<String>,
    host: Option<String>,
    action: Option<ActionFilter>,
    errors: bool,
) -> Result<Vec<LogRow>> {
    let mut query = String::from(
        "SELECT timestamp, command, host, action, status, duration_ms, note FROM operation_log WHERE 1=1",
    );
    let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref h) = host {
        query.push_str(&format!(" AND host = ?{}", bind_values.len() + 1));
        bind_values.push(Box::new(h.clone()));
    }
    if let Some(ref a) = action {
        let action_str = match a {
            ActionFilter::Sync => "sync",
            ActionFilter::Run => "run",
            ActionFilter::Exec => "exec",
            ActionFilter::Check => "check",
        };
        query.push_str(&format!(" AND command = ?{}", bind_values.len() + 1));
        bind_values.push(Box::new(action_str.to_string()));
    }
    if errors {
        query.push_str(" AND status = 'error'");
    }
    if let Some(ref s) = since {
        let since_ts = parse_since(s)?;
        query.push_str(&format!(" AND timestamp >= ?{}", bind_values.len() + 1));
        bind_values.push(Box::new(since_ts));
    }
    query.push_str(" ORDER BY timestamp DESC");
    query.push_str(&format!(" LIMIT {}", last));

    let mut stmt = ctx.db.prepare(&query)?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        bind_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok(LogRow {
            ts: row.get(0)?,
            command: row.get(1)?,
            host: row.get(2)?,
            action: row.get(3)?,
            status: row.get(4)?,
            duration_ms: row.get(5)?,
            note: row.get(6)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}

pub async fn run(
    ctx: &Context,
    last: usize,
    since: Option<String>,
    host: Option<String>,
    action: Option<ActionFilter>,
    errors: bool,
) -> Result<()> {
    let rows = log_core(ctx, last, since, host, action, errors)?;
    if rows.is_empty() {
        println!("No log entries found.");
        return Ok(());
    }
    for r in &rows {
        let time = chrono::DateTime::from_timestamp(r.ts, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| r.ts.to_string());
        let duration = r
            .duration_ms
            .map(|ms| format!(" ({:.1}s)", ms as f64 / 1000.0))
            .unwrap_or_default();
        let note_str = r
            .note
            .as_deref()
            .map(|n| format!(" — {}", n))
            .unwrap_or_default();
        let status_icon = match r.status.as_str() {
            "ok" => "\x1b[32m✓\x1b[0m",
            "error" => "\x1b[31m✗\x1b[0m",
            "skipped" => "\x1b[33m⊘\x1b[0m",
            _ => "·",
        };
        println!(
            "{} {} [{}] {} {}{}{}",
            time, status_icon, r.host, r.command, r.action, duration, note_str
        );
    }
    Ok(())
}

fn parse_since(s: &str) -> Result<i64> {
    let now = chrono::Utc::now().timestamp();
    if let Some(days) = s.strip_suffix('d') {
        if let Ok(n) = days.parse::<i64>() {
            return Ok(now - n * 86400);
        }
    }
    if let Some(hours) = s.strip_suffix('h') {
        if let Ok(n) = hours.parse::<i64>() {
            return Ok(now - n * 3600);
        }
    }
    if let Ok(dt) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(dt.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp());
    }
    anyhow::bail!("Invalid --since value: {}", s);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_relative_days() {
        let now = chrono::Utc::now().timestamp();
        let ts = parse_since("7d").unwrap();
        assert!((now - ts - 7 * 86400).abs() < 5);
    }

    #[test]
    fn log_core_empty_db_returns_no_rows() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        crate::state::db::migrate_for_test(&db);
        let ctx = crate::commands::Context {
            config: crate::config::schema::AppConfig::default(),
            config_path: None,
            db,
            timeout: 30,
            mode: crate::commands::TargetMode::All,
            serial: false,
            skip: vec![],
            verbose: false,
        };
        let rows = log_core(&ctx, 20, None, None, None, false).unwrap();
        assert!(rows.is_empty());
    }
}
