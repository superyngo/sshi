# State schema reference

This document describes the SQLite database architecture, schema migrations, table definitions, concurrency model, and data retention policy used in `sshi` for persisting metrics snapshots, sync state, and operation audit logs.

---

## Database location and path resolution

`sshi` stores its persistent SQLite state in `sshi.db`. The resolved state directory also houses TUI state files named `tui_state-{config_hash}.toml` (`tui::state::persist::state_file_path`).

### Path resolution order

1. **Config override**: If `[settings].state_dir` is configured in `config.toml`, `sshi` uses that directory directly (`state::db::resolved_state_dir`).
2. **Platform default**:
   - **Linux / macOS**: `~/.local/state/sshi/sshi.db` (following XDG Base Directory specification via `dirs::home_dir().join(".local/state/sshi")`).
   - **Windows**: `%LOCALAPPDATA%\sshi\sshi.db` (`dirs::data_local_dir().join("sshi")`).

When opening the database or saving TUI state, `sshi` ensures the parent directory hierarchy is created automatically (`std::fs::create_dir_all`).

---

## Connection and concurrency architecture

`sshi` uses `rusqlite` for SQLite access, wrapped in a thread-safe handle designed for concurrent asynchronous operations and synchronous UI renders.

### `DbHandle`

The primary database interface is `DbHandle` (`src/state/db.rs`):

```rust
pub struct DbHandle {
    conn: Arc<Mutex<Connection>>,
}
```

- **Thread-safe sharing**: `rusqlite::Connection` is `!Sync`. Wrapping the connection in `Arc<Mutex<Connection>>` allows the handle to be cloned and shared across Tokio tasks and commands without compiler errors.
- **Offloading blocking I/O**: The async methods `execute` and `transaction` wrap SQLite calls in `tokio::task::spawn_blocking`. This ensures database writes and disk I/O run on Tokio's blocking thread pool, leaving the async runtime free to drive concurrent SSH operations unimpeded.
- **Owned parameters**: Because references from a caller's stack cannot cross the `'static` boundary into `spawn_blocking`, parameters are boxed via `boxed_param` (`Box<dyn rusqlite::ToSql + Send + Sync>`).
- **Atomic transactions**: `DbHandle::transaction` executes a closure taking `&rusqlite::Transaction` inside `spawn_blocking`. Returning `Ok` commits the transaction; returning `Err` triggers an automatic rollback.
- **Synchronous access**: `DbHandle::with_conn` provides a synchronous escape hatch for short-lived queries run directly on the TUI main thread (e.g., refreshing snapshot tables and log views).

### Connection pragmas

When `db::open` initializes a connection, it executes the following pragmas:

```sql
PRAGMA journal_mode=WAL;
PRAGMA busy_timeout=5000;
PRAGMA synchronous=NORMAL;
```

- **`journal_mode=WAL`**: Enables Write-Ahead Logging. Readers (such as the TUI view refresh or CLI log queries) do not block writers (such as metric collection tasks or sync operations), and writers do not block readers.
- **`busy_timeout=5000`**: Sets a 5000ms (5 second) busy handler. If another process or thread holds a write lock, queries wait up to 5 seconds before returning an `SQLITE_BUSY` error.
- **`synchronous=NORMAL`**: In WAL mode, `NORMAL` synchronous reduces disk sync operations while maintaining full database integrity against application crashes.

---

## Schema migration mechanism

Database migrations are embedded into the compiled binary and managed sequentially using SQLite's built-in `PRAGMA user_version` (`src/state/db.rs`).

### Migration lifecycle

- **Current schema version**: Tracked by the constant `CURRENT_VERSION = 2` in `src/state/db.rs`.
- **Version inspection**: On startup, `db::open` queries `PRAGMA user_version`.
- **Sequential application**: If `user_version` is less than `CURRENT_VERSION`, pending migrations are applied in ascending order:
  - **Version 0 → 1**: Executes `src/state/migrations/001_init.sql` (creates `check_snapshots`, `host_last_seen`, `sync_state`, `operation_log`, and associated indexes).
  - **Version 1 → 2**: Executes `src/state/migrations/002_log_stdout.sql` (adds `stdout` column to `operation_log`). To guard against race conditions across concurrent processes, migration 2 performs an idempotent column check via `pragma_table_info('operation_log')` before running `ALTER TABLE`.
- **Version update**: After all migrations succeed, `PRAGMA user_version` is set to `CURRENT_VERSION`.

### Adding a new migration

To add migration `N` (e.g., version 3):

1. Create a new SQL migration file: `src/state/migrations/003_<description>.sql`.
2. Increment `CURRENT_VERSION` in `src/state/db.rs`:
   ```rust
   const CURRENT_VERSION: u32 = 3;
   ```
3. Add a new conditional block in `db::migrate`:
   ```rust
   if version < 3 {
       conn.execute_batch(include_str!("migrations/003_<description>.sql"))?;
   }
   ```
4. If the migration alters existing tables, include defensive checks (e.g., inspecting `pragma_table_info`) where necessary to handle concurrent execution safely.

---

## Table schemas

The current database schema (v2) consists of four tables.

### 1. `check_snapshots`

Stores historical metric snapshots collected during `sshi check` and TUI metric runs.

```sql
CREATE TABLE IF NOT EXISTS check_snapshots (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    host         TEXT    NOT NULL,
    collected_at INTEGER NOT NULL,
    online       INTEGER NOT NULL,
    raw_json     TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_check_snapshots_host_time
    ON check_snapshots (host, collected_at DESC);
```

| Column | Type | Nullable | Description |
|---|---|---|---|
| `id` | `INTEGER` | No | Auto-incrementing primary key. |
| `host` | `TEXT` | No | Host name alias as configured in `[[host]]`. |
| `collected_at` | `INTEGER` | No | Unix epoch timestamp (seconds) when the metrics were collected. |
| `online` | `INTEGER` | No | Host reachability status: `1` = online, `0` = unreachable/offline. |
| `raw_json` | `TEXT` | No | JSON-serialized string of the full system metric probe results (CPU, memory, disk, network, etc.). Stored as `"{}"` when unreachable. |

#### Indexes
- `idx_check_snapshots_host_time`: Composite index on `(host, collected_at DESC)` optimizing historical lookups and trend reports by host.

---

### 2. `host_last_seen`

Maintains the high-water mark timestamp of when each host was last probed and last confirmed online.

```sql
CREATE TABLE IF NOT EXISTS host_last_seen (
    host        TEXT    PRIMARY KEY,
    last_seen   INTEGER NOT NULL,
    last_online INTEGER NOT NULL
);
```

| Column | Type | Nullable | Description |
|---|---|---|---|
| `host` | `TEXT` | No | Host name alias (primary key). |
| `last_seen` | `INTEGER` | No | Unix epoch timestamp (seconds) of the most recent check attempt (online or offline). |
| `last_online` | `INTEGER` | No | Unix epoch timestamp (seconds) when the host was last confirmed online (`0` if never seen online). |

#### Upsert semantics
`check_core` updates this table after every probe:
- **Host online**:
  ```sql
  INSERT INTO host_last_seen (host, last_seen, last_online) VALUES (?1, ?2, ?2)
  ON CONFLICT(host) DO UPDATE SET last_seen = ?2, last_online = ?2
  ```
- **Host offline**:
  ```sql
  INSERT INTO host_last_seen (host, last_seen, last_online) VALUES (?1, ?2, 0)
  ON CONFLICT(host) DO UPDATE SET last_seen = ?2
  ```

---

### 3. `sync_state`

Records file synchronization events across sync entries and target hosts. Currently, the sync engine operates without consulting this table (change detection queries hosts live), making `sync_state` an audit log of synced paths.

```sql
CREATE TABLE IF NOT EXISTS sync_state (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    sync_group   TEXT    NOT NULL,
    host         TEXT    NOT NULL,
    path         TEXT    NOT NULL,
    mtime        INTEGER NOT NULL,
    size_bytes   INTEGER NOT NULL,
    blake3       TEXT    NOT NULL,
    synced_at    INTEGER NOT NULL,
    UNIQUE (sync_group, host, path)
);

CREATE INDEX IF NOT EXISTS idx_sync_state_group
    ON sync_state (sync_group, host);
```

| Column | Type | Nullable | Description |
|---|---|---|---|
| `id` | `INTEGER` | No | Auto-incrementing primary key. |
| `sync_group` | `TEXT` | No | Sync entry name/label from `[[sync]]`. |
| `host` | `TEXT` | No | Target remote host name. |
| `path` | `TEXT` | No | Remote file path. |
| `mtime` | `INTEGER` | No | Placeholder timestamp; currently inserted as `0`. |
| `size_bytes` | `INTEGER` | No | Placeholder file size; currently inserted as `0`. |
| `blake3` | `TEXT` | No | Legacy hash column; currently inserted as `""` (empty string). |
| `synced_at` | `INTEGER` | No | Unix epoch timestamp (seconds) when sync succeeded. |

#### Constraints & Indexes
- `UNIQUE (sync_group, host, path)`: Ensures an upsert updates the record for a given file and target host.
- `idx_sync_state_group`: Index on `(sync_group, host)` defined in schema; currently unused by the query engine as `sync_state` is not read.

---

### 4. `operation_log`

Audit trail recording every command executed against remote hosts (`check`, `sync`, `run`, `exec`, `cp`).

```sql
CREATE TABLE IF NOT EXISTS operation_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp   INTEGER NOT NULL,
    command     TEXT    NOT NULL,
    host        TEXT    NOT NULL,
    action      TEXT    NOT NULL,
    status      TEXT    NOT NULL,
    duration_ms INTEGER,
    note        TEXT,
    stdout      TEXT
);

CREATE INDEX IF NOT EXISTS idx_operation_log_time
    ON operation_log (timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_operation_log_host
    ON operation_log (host, timestamp DESC);
```

| Column | Type | Nullable | Description |
|---|---|---|---|
| `id` | `INTEGER` | No | Auto-incrementing primary key. |
| `timestamp` | `INTEGER` | No | Unix epoch timestamp (seconds) when the operation started. |
| `command` | `TEXT` | No | Subcommand name (`"check"`, `"sync"`, `"run"`, `"exec"`, `"cp"`). |
| `host` | `TEXT` | No | Target host alias. |
| `action` | `TEXT` | No | Action detail (e.g., `"metrics_batch"`, command line string, sync file path). |
| `status` | `TEXT` | No | Execution status (`"ok"`, `"error"`). |
| `duration_ms` | `INTEGER` | Yes | Total execution duration in milliseconds (`NULL` if unavailable; `sync` records `0`). |
| `note` | `TEXT` | Yes | Optional error message, failure reason, or detail note (`NULL` on success). |
| `stdout` | `TEXT` | Yes | Captured stdout preview for `exec` and `run` commands (added in schema v2, `NULL` otherwise). |

#### Indexes
- `idx_operation_log_time`: Index on `(timestamp DESC)` for global log viewing and reverse-chronological pagination.
- `idx_operation_log_host`: Composite index on `(host, timestamp DESC)` for filtering logs by host.

---

## Retention and cleanup policy

Historical data is pruned according to the time-based retention policy implemented in `src/state/retention.rs`.

### Execution trigger

Retention cleanup is invoked automatically at the end of each `sshi check` execution (`commands::check::check_core`):

```rust
retention::cleanup(&ctx.db, ctx.config.settings.data_retention_days).await?;
```

Because `check` can be executed periodically via cron, CLI, or TUI, cleanup runs regularly without requiring a separate background daemon.

### Configuration

Data retention is governed by `[settings].data_retention_days` in `config.toml`:
- **Default**: `90` days (`config::schema::default_retention`).
- **Disable cleanup**: Setting `data_retention_days = 0` disables cleanup entirely, retaining all historical records indefinitely.

### Pruning logic

When `retention_days > 0`, `cleanup` computes `cutoff_secs = retention_days * 86400` and executes two deletion queries:

```sql
DELETE FROM check_snapshots WHERE collected_at < (strftime('%s', 'now') - ?1);
DELETE FROM operation_log WHERE timestamp < (strftime('%s', 'now') - ?1);
```

### Table retention summary

| Table | Subject to Retention Cleanup | Rationale |
|---|---|---|
| `check_snapshots` | **Yes** | Time-series metrics data grows proportionally with check frequency. |
| `operation_log` | **Yes** | Audit log grows with every executed command. |
| `host_last_seen` | **No** | Stores exactly one high-water mark row per host. |
| `sync_state` | **No** | Represents file synchronization audit records; currently write-only and never read by sync planning. |
