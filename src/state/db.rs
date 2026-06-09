use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::Connection;

const CURRENT_VERSION: u32 = 2;

/// Returns the platform-appropriate state directory for sshi.
pub fn state_dir() -> Result<PathBuf> {
    // On macOS/Linux: ~/.local/state/sshi
    // On Windows: %LOCALAPPDATA%/sshi
    #[cfg(target_os = "windows")]
    let base = dirs::data_local_dir().context("Cannot determine local data directory")?;
    // Use state subdirectory on Linux/macOS for XDG compliance
    #[cfg(not(target_os = "windows"))]
    let base = {
        let home = dirs::home_dir().context("Cannot determine home directory")?;
        home.join(".local").join("state")
    };
    Ok(base.join("sshi"))
}

/// Returns the path to sshi.db.
#[allow(dead_code)]
pub fn db_path() -> Result<PathBuf> {
    Ok(state_dir()?.join("sshi.db"))
}

/// Resolve the effective state directory (AD-16): honors the
/// `[settings].state_dir` override or falls back to the OS default.
/// Single source of truth used by both DB open and TUI state file path.
pub fn resolved_state_dir(override_dir: Option<&std::path::Path>) -> Result<PathBuf> {
    match override_dir {
        Some(dir) => Ok(dir.to_path_buf()),
        None => state_dir(),
    }
}

/// Open or create the SQLite database with migrations applied.
/// If `override_dir` is provided, uses that directory instead of the default.
pub fn open(override_dir: Option<&std::path::Path>) -> Result<Connection> {
    let path = resolved_state_dir(override_dir)?.join("sshi.db");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let conn = Connection::open(&path)
        .with_context(|| format!("Failed to open database {}", path.display()))?;

    // Enable WAL mode for better concurrent reads
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;

    migrate(&conn)?;
    Ok(conn)
}

#[cfg(test)]
pub fn migrate_for_test(conn: &Connection) {
    migrate(conn).unwrap();
}

fn migrate(conn: &Connection) -> Result<()> {
    let version: u32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;

    if version < 1 {
        conn.execute_batch(include_str!("migrations/001_init.sql"))?;
    }
    if version < 2 {
        // ALTER TABLE is not idempotent; check first in case two connections
        // race to apply this migration against the same on-disk DB.
        let has_col: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('operation_log') WHERE name = 'stdout'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if has_col == 0 {
            conn.execute_batch(include_str!("migrations/002_log_stdout.sql"))?;
        }
    }
    conn.pragma_update(None, "user_version", CURRENT_VERSION)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_in_memory() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
        migrate(&conn).unwrap();

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);
    }
}
