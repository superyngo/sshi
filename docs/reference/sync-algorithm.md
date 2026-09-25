# Sync algorithm reference

This document describes the file synchronization protocol implemented in `sshi` (`src/commands/sync/`). The synchronization engine coordinates file consistency across multiple remote hosts over SSH/SFTP using a centralized coordination model.

## Overview

The sync workflow runs across four primary phases:

```
┌────────────────────────────────────────────────────────┐
│ 1. Directory Expansion (commands::sync::collect)       │
│    - Expand directory paths on remote hosts (find -L)  │
│    - Rewrite path lists with expanded file paths       │
└──────────────────────────┬─────────────────────────────┘
                           │
                           ▼
┌────────────────────────────────────────────────────────┐
│ 2. Decide Batch (commands::sync::decide)               │
│    - Batch-gather mtime, size, SHA-256 hashes          │
│    - Compare file hashes across reachable hosts        │
│    - Build SyncDecision per file (source, targets)     │
└──────────────────────────┬─────────────────────────────┘
                           │
                           ▼
┌────────────────────────────────────────────────────────┐
│ 3. Distribute Batch (commands::sync::distribute)       │
│    - Local Relay: Download source file to local temp   │
│    - Concurrently upload local temp to target hosts    │
│    - Record results in SQLite sync_state & log         │
└──────────────────────────┬─────────────────────────────┘
                           │
                           ▼
┌────────────────────────────────────────────────────────┐
│ 4. Recursive Entries (commands::sync::mod)             │
│    - Iterate recursive sync entries (recursive = true) │
│    - Run per-file sync flow via sync_path_across       │
└────────────────────────────────────────────────────────┘
```

The sync pipeline is coordinated by `sync_core` and `sync_inner` in `src/commands/sync/mod.rs`. Before executing the pipeline, `sshi` establishes an `SshPool` managing multiplexed `russh` client sessions, tests SFTP capability on all target hosts, and filters out unreachable or SFTP-incapable hosts. Sync requires at least 2 reachable SFTP-capable hosts.

---

## Phase 1: Directory Expansion & Path Resolution

Before metadata collection, configured sync paths are resolved and expanded into concrete file lists:

1. **Path Resolution (`collect_sync_paths`)**:
   - Gathers paths from CLI positional arguments (ad-hoc mode) or matching `[[sync]]` configuration entries (selected via `-n/--name`).
   - Distinguishes between flat batch sync paths and recursive sync entries (`recursive = true`).
2. **Directory Expansion (`expand_paths`, `expand_directory_paths`)**:
   - For entries with a fixed source: inspects the directory on the specified source host.
   - For entries without a fixed source: queries all reachable hosts and computes the union of expanded paths (`union_dir_expansions`).
   - Directory paths are replaced with their constituent file paths.

### Shell-Specific Directory Expansion Commands

Directory expansion queries remote files using `build_dir_expand_cmd`:

- **POSIX (`Sh`)**:
  ```sh
  for p in <files>; do
    orig=$(echo "$p" | sed "s|^$HOME/|~/|;s|^$HOME$|~|");
    echo "---PATH:$orig";
    if [ -d "$p" ]; then
      echo "DIR";
      find -L "$p" -maxdepth 1 -type f 2>/dev/null | sed "s|^$HOME/|~/|" | sort;
    elif [ -e "$p" ]; then
      echo "FILE";
    else
      echo "MISSING";
    fi;
  done
  ```
  *(Note: For recursive entries, `-maxdepth 1` is omitted).*
  **Symlink behavior**: POSIX directory expansion uses `find -L`, which **follows symbolic links** during expansion.

- **Windows PowerShell (`PowerShell`)**:
  ```powershell
  $h=$HOME;
  foreach ($p in @(<files>)) {
    $orig=$p -replace [regex]::Escape($h),'~';
    "---PATH:$orig";
    if (Test-Path $p -PathType Container) {
      "DIR";
      Get-ChildItem $p -File | ForEach-Object {
        $_.FullName -replace [regex]::Escape($h),'~'
      }
    } elseif (Test-Path $p) { "FILE" }
    else { "MISSING" }
  }
  ```
  *(Note: Appends `-Recurse` when recursive mode is enabled).*

- **Windows Cmd (`Cmd`)**:
  Runs the PowerShell `Get-ChildItem -File` logic via `powershell -NoProfile -EncodedCommand <base64>` (`host::quote::ps_in_cmd`), so paths never pass through a cmd quoting layer.

---

## Stage 1: Collect (Metadata Gathering)

The collection phase queries file metadata across all reachable hosts in parallel (`batch_collect_all_metadata` in `src/commands/sync/collect.rs`; per-file `collect_file_metadata` for recursive entries).

A host whose query errors or exits non-zero is returned in `failed` and recorded by `record_collect_failures` as a host failure (`metadata collection failed: exit N: <first stderr line>`, exit code 3 per ADR 0004). Its state is unknown, so it is neither a source nor a target for that run; it is never treated as `MISSING`. On PowerShell/Cmd, `Get-FileHash` runs with `-ErrorAction SilentlyContinue` and prints `NOHASH` for an unreadable file, so one locked file does not fail the whole batch.

To minimize SSH channel round-trips, `sshi` executes a single batched remote command per host querying all required paths, formatted using `---FILE:<path>` block delimiters.

`<files>` in both command sets is each path quoted by `host::quote::quote_path` (see [ssh-transport.md](ssh-transport.md#remote-quoting)): sh `"$HOME"/'rest'` / `'path'`, PowerShell `($HOME + '\rest')` / `'path'`.

### Remote Commands (`build_batch_metadata_cmd`)

- **POSIX (`Sh`)**:
  ```sh
  for f in <files>; do
    echo "---FILE:$f";
    stat -c '%Y %s' "$f" 2>/dev/null || stat -f '%m %z' "$f" 2>/dev/null || echo "MISSING";
    (sha256sum "$f" 2>/dev/null || shasum -a 256 "$f" 2>/dev/null) || echo "NOHASH";
  done
  ```
- **Windows PowerShell (`PowerShell`)**:
  ```powershell
  foreach ($f in @(<files>)) {
    "---FILE:$f";
    $i=Get-Item $f -ErrorAction SilentlyContinue;
    if ($i) {
      [int64](($i.LastWriteTimeUtc-[datetime]"1970-01-01").TotalSeconds), $i.Length -join " ";
      (Get-FileHash $f -Algorithm SHA256).Hash.ToLower()
    } else { "MISSING" }
  }
  ```
- **Windows Cmd (`Cmd`)**:
  Invokes the PowerShell script snippet via `powershell -NoProfile -EncodedCommand <base64>` (`host::quote::ps_in_cmd`).

### Hash Algorithm Confirmation: SHA-256 vs BLAKE3

- **Remote File Hashing**: The sync protocol uses **SHA-256** exclusively for remote file verification across all supported operating systems (`sha256sum` / `shasum -a 256` on POSIX, `Get-FileHash -Algorithm SHA256` on Windows).
- **`blake3` in `Cargo.toml`**: The `blake3` crate dependency in `Cargo.toml` is used for internal local utilities:
  - Generating stable random entry IDs (`generate_entry_id()` in `src/config/schema.rs`).
  - Hashing config paths for TUI state file persistence (`src/tui/state/persist.rs`).
- **Database Schema Column**: The SQLite `sync_state` table retains a column historically named `blake3`, but sync writes empty strings (`""`) to it; hashes are compared in memory during decision-making and are not persisted to the database. The active hash algorithm executed on remote hosts during synchronization is **SHA-256**.

### Collection Results

`parse_batch_metadata_output` converts remote stdout blocks into `FileInfo` records:
- `host`: Host name.
- `mtime`: Last modification time in seconds since Unix epoch.
- `hash`: Hex-encoded SHA-256 checksum (or empty string if unhashable).
- Missing files are tracked separately per host.

---

## Stage 2: Decide (Source Selection & Conflict Resolution)

The decision phase (`src/commands/sync/decide.rs`) analyzes `FileInfo` records across hosts for each path to determine if action is required.

### In-Sync Check

If all hosts possessing the file share identical non-empty SHA-256 hashes:
- If no hosts are missing the file (or `push_missing` is `false`), the file is marked **in sync** and skipped.
- If some hosts are missing the file and `push_missing` is `true`, a decision is created to replicate the existing file to the missing hosts.

### Conflict Strategies (`make_decisions`)

When hashes differ across reachable hosts, the configured `ConflictStrategy` dictates the resolution:

#### 1. `ConflictStrategy::Newest` (Default)
- **Source Selection**: The host with the highest modification time (`mtime`) is selected as the authoritative source (`file_infos.iter().max_by_key(|f| f.mtime)`).
- **Target Hosts**: Every reachable host with a differing hash (`f.hash != source.hash`), plus any missing hosts if `push_missing` is enabled.
- **Synced Hosts**: Hosts that already match the source hash (`f.hash == source.hash`).
- **Reason**: Formatted as `"newest mtime: <timestamp>"` (or `"in sync on reachable hosts, pushing to N missing"`).

#### 2. `ConflictStrategy::Skip`
- **Conflict Handling**: If more than one distinct hash exists across reachable hosts (`hashes.len() > 1`), `sshi` emits **no** `SyncDecision` and leaves every copy unmodified. The caller detects this case first via `decide::skip_conflict_hosts` and records the path as **skipped** in the summary with the reason "contents differ between hosts" and the hosts involved — it is never counted as in sync.
- **Missing File Propagation**: If all existing copies share the same hash and only missing hosts exist, the first host is chosen as source to push to the missing hosts.

### Fixed Source Selection (`make_decisions_fixed_source`)

When a source host is explicitly defined (via `--source <host>` CLI argument or `source = "<host>"` in `[[sync]]` configuration):
- The designated host is unconditionally selected as the source.
- Targets are all reachable hosts with differing hashes + missing hosts.
- If the fixed source host does not possess the file, the sync for that path is skipped.

---

## Stage 3: Distribute (Local Relay Pattern)

The distribution phase (`distribute_batch` in `src/commands/sync/mod.rs`, `distribute_pooled` in `src/commands/sync/distribute.rs`) applies the `SyncDecision`.

### Local Relay Transfer Pattern

`sshi` does not perform direct remote-to-remote (P2P/SCP) copies between target hosts. Instead, it utilizes a two-step **Local Relay**:

```
[Source Host] ──────(SFTP Download)──────► [Local Temp File]
                                                    │
                                                    ├──────(SFTP Upload)──────► [Target Host A]
                                                    ├──────(SFTP Upload)──────► [Target Host B]
                                                    └──────(SFTP Upload)──────► [Target Host C]
```

1. **Download Phase**:
   - Creates a temporary directory locally via `tempfile::tempdir()`.
   - Acquires concurrency permit for the source host.
   - Downloads the file from `source_host` to `<temp_dir>/sshi_relay` via SFTP (`sessions.download(...)`).
2. **Upload Phase**:
   - Concurrently spawns upload tasks for all `target_hosts` using a Tokio `JoinSet`.
   - For each target, acquires both the global concurrency permit and the target's per-host concurrency permit (`ConcurrencyLimiter`).
   - Streams the local temporary file to the remote target path via SFTP (`sessions.upload(...)`).
3. **Cleanup**:
   - When the `tempfile::TempDir` handle drops at the end of the distribution function, the local relay file is automatically deleted from local disk.

### Dry-Run Mode

When `--dry-run` is specified:
- No files are downloaded or uploaded.
- Planned transfers are logged and printed with their source, target list, and decision reason.
- Reports accurately reflect what transfers would have occurred.

### Persistence & Reporting

After successful distribution:
1. **SQLite Database Update**: Updates `sync_state` and appends entries to `operation_log` in a single SQLite transaction.
2. **Summary & Progress**: Updates `SyncSummary` counters (`files_synced`, `files_partial`, `files_failed`, `transfers_synced`, `transfers_failed`, etc.) and dispatches event notifications to `ProgressSink` (updating the CLI progress or TUI view).

---

## Phase 4: Recursive Entries (`run_recursive_entries`)

Entries configured with `recursive = true` bypass the batched collect/decide/distribute pipeline. In Phase 4 (`commands::sync::run_recursive_entries`):

1. **Directory Expansion**: Expands directory contents on the source host (or computes the union of expanded paths across reachable hosts when no fixed source is specified).
2. **Per-File Synchronization**: Executes `sync_path_across` sequentially for each expanded file path, collecting metadata (`collect_file_metadata`), making individual `SyncDecision` evaluations, and executing `distribute_pooled`.
3. **Incremental Recording**: Each synchronized file commits its outcome immediately to SQLite (`sync_state` and `operation_log`) and dispatches progress updates to the caller's `ProgressSink`.
