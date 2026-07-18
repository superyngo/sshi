//! Database initialization, migration, and connection management.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::Connection;

const CURRENT_VERSION: u32 = 2;

/// Handle to the shared SQLite connection.
///
/// Wraps `Arc<Mutex<Connection>>` so multiple async tasks on the same
/// per-op runtime can share one connection without `Connection: !Sync`
/// poisoning cross-thread sends. Async methods (`execute`, `transaction`)
/// wrap their rusqlite work in `tokio::task::spawn_blocking` so the
/// current-thread runtime is free to drive concurrent SSH tasks while the
/// DB write is on a blocking-pool thread.
///
/// The sync `with_conn` escape hatch is intended only for short-lived
/// helpers (e.g. `log_core`, `fetch_latest_snapshots`) that are called
/// from sync TUI main-thread code and never run on the per-op runtime —
/// for those callers, blocking the thread is the same as today.
pub struct DbHandle {
    conn: Arc<Mutex<Connection>>,
}

impl DbHandle {
    pub fn new(conn: Connection) -> Self {
        Self {
            conn: Arc::new(Mutex::new(conn)),
        }
    }

    /// Execute a single parameterised statement on a blocking-pool thread.
    /// Owned boxed params are required because borrows from the caller's
    /// stack cannot be moved into `spawn_blocking` (no `'static` lifetime).
    /// Use [`boxed_param`] for ergonomic construction.
    pub async fn execute(
        &self,
        sql: &str,
        params: Vec<Box<dyn rusqlite::ToSql + Send + Sync>>,
    ) -> Result<usize> {
        let conn = self.conn.clone();
        let sql = sql.to_string();
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().expect("DB mutex poisoned");
            let refs: Vec<&dyn rusqlite::ToSql> = params
                .iter()
                .map(|b| -> &dyn rusqlite::ToSql { &**b })
                .collect();
            guard.execute(&sql, rusqlite::params_from_iter(refs))
        })
        .await
        .context("DB execute task failed")?
        .map_err(Into::into)
    }

    /// Run a closure inside one SQLite transaction on a blocking-pool
    /// thread. The closure receives a `&Transaction`; returning `Ok`
    /// commits, returning `Err` rolls back. Used to batch drain loops
    /// (audit §3.5 MED).
    pub async fn transaction<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&rusqlite::Transaction) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let mut guard = conn.lock().expect("DB mutex poisoned");
            let tx = guard.transaction()?;
            let r = f(&tx)?;
            tx.commit()?;
            Ok(r)
        })
        .await
        .context("DB transaction task failed")?
    }

    /// Sync escape hatch: run `f` against the underlying connection while
    /// holding the mutex. Use only from sync code paths that cannot move
    /// onto the blocking pool (e.g. TUI main-thread helpers).
    pub fn with_conn<F, T>(&self, f: F) -> T
    where
        F: FnOnce(&Connection) -> T,
    {
        let guard = self.conn.lock().expect("DB mutex poisoned");
        f(&guard)
    }
}

/// Box a value as a type-erased `ToSql` parameter for [`DbHandle::execute`].
pub fn boxed_param<T: rusqlite::ToSql + Send + Sync + 'static>(
    v: T,
) -> Box<dyn rusqlite::ToSql + Send + Sync> {
    Box::new(v)
}

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

    // Enable WAL mode for better concurrent reads; raise the busy timeout so
    // concurrent CLI + TUI access waits instead of returning SQLITE_BUSY, and
    // drop synchronous from FULL to NORMAL (documented fast/safe combo under WAL).
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;\nPRAGMA busy_timeout=5000;\nPRAGMA synchronous=NORMAL;",
    )?;

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

    #[test]
    fn test_pragmas_set_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open(Some(dir.path())).unwrap();
        let busy: i64 = conn
            .pragma_query_value(None, "busy_timeout", |r| r.get(0))
            .unwrap();
        assert_eq!(busy, 5000);
        let synchronous: i64 = conn
            .pragma_query_value(None, "synchronous", |r| r.get(0))
            .unwrap();
        assert_eq!(synchronous, 1);
    }
}
