# Glossary

Canonical vocabulary for `sshi`. Every other document, code identifier, and commit message uses
these terms. Introducing a new term means adding its entry here in the same commit.

**sshi**:
The project and binary name (`Cargo.toml` `[package]`/`[[bin]]`, `README.md` H1).
_Avoid_: ssync — the project's former name; still appears stale in historical `CHANGELOG.md`
entries (left as-is, the changelog is a historical record) but must not appear in any current
document.

**AppConfig**:
The root configuration structure deserialized from `config.toml`, holding **Settings** (`[settings]`), **HostEntry** items (`[[host]]`), **CheckEntry** items (`[[check]]`), and **SyncEntry** items (`[[sync]]`) (`src/config/schema.rs`).
_Avoid_: Config, Configuration, RootConfig.

**Settings**:
The global operational configuration table (`[settings]`) within **AppConfig**, defining process defaults such as timeouts, data retention days, **ConflictStrategy**, and concurrency limits (`src/config/schema.rs`).
_Avoid_: Preferences, Options, GlobalConfig.

**HostEntry**:
A configured remote host (`[[host]]` table), populated by import from `~/.ssh/config` or added
directly in `config.toml` (`src/config/schema.rs`).
_Avoid_: server, node, machine, target (as a config table).

**TargetMode**:
The host-filtering strategy that resolves which **HostEntry** values a command runs against, via
`--all`/`--group`/`--host`/`--shell` (`src/commands/mod.rs`, `src/cli.rs`).
_Avoid_: filter_mode, host_filter, target_args.

**Context**:
The shared execution bundle passed to all command core functions (`check`, `sync`, `run`, `exec`, `cp`, `init`), aggregating `Arc<`**AppConfig**`>`, **DbHandle**, **TargetMode**, timeouts, and execution flags (`src/commands/mod.rs`).
_Avoid_: CommandContext, ExecutionContext, SessionContext, Env.

**CheckEntry**:
A periodic system-metric inspection task (`[[check]]` table) applying named probe sets and paths
to hosts (`src/config/schema.rs`, `src/commands/check.rs`).
_Avoid_: monitor, probe_task, snapshot_job, inspection.

**SyncEntry**:
A file-synchronization rule (`[[sync]]` table) specifying paths, recursion (`recursive`), and source overrides (`src/config/schema.rs`, `src/commands/sync/mod.rs`). Global conflict resolution is configured under **Settings** via **ConflictStrategy**.
_Avoid_: sync_group, sync_task, file_sync.

**ConflictStrategy**:
The resolution rule (`newest` or `skip`) applied when the same file differs across hosts during
sync (`src/config/schema.rs`, `src/commands/sync/decide.rs`).
_Avoid_: conflict_resolution, sync_strategy, overwrite_policy.

**Local Relay**:
The sync transfer pattern: download the source file to a local temp path, then upload it to every
target (`src/commands/sync/distribute.rs`).
_Avoid_: p2p_sync, direct_scp, remote_copy.

**Snapshot**:
A stored point-in-time record of system metrics and host reachability, persisted to the
`check_snapshots` SQLite table (`src/state/db.rs`, `src/commands/checkout/core.rs`).
_Avoid_: metric_record, host_status, telemetry_row.

**Checkout**:
The subcommand and TUI view for querying historical **Snapshot** data and generating trend
reports (`src/cli.rs`, `src/commands/checkout/mod.rs`).
_Avoid_: dashboard, status, inspect, history_viewer.

**SessionPool**:
The async trait (`src/host/session_pool.rs`) and thread-safe implementation (`RusshSessionPool`) that owns multiplexed russh SSH client handles, one per host. Substituted with `MockSessionPool` (`src/host/session_pool_mock.rs`) in tests.
_Avoid_: ConnectionPool, SshManager, ClientCache.

**SshPool**:
The high-level wrapper combining **SessionPool**, a dual-level **ConcurrencyLimiter**, and sync
progress reporting (`src/host/pool.rs`).
_Avoid_: ExecutorPool, Pool, MasterPool.

**ConcurrencyLimiter**:
The two-tier semaphore enforcing global and per-host concurrent-operation limits
(`src/host/concurrency.rs`).
_Avoid_: Throttle, RateLimiter, PermitManager.

**ShellType**:
The detected remote execution environment (`Sh`, `PowerShell`, or `Cmd`) that dictates probe
syntax and path conventions (`src/config/schema.rs`, `src/host/shell.rs`).
_Avoid_: RemoteShell, OsType, ShellKind.

**ShellMode**:
The TUI-specific enum for shell-based target filtering, persisted in UI state
(`src/tui/state/persist.rs`). Distinct from **ShellType**, which is the transport-layer enum.
_Avoid_: ShellType (reserved for the transport-layer enum), ShellFilter.

**SecretString**:
A memory-safe wrapper holding a password or passphrase that zeroizes its contents on drop and prints as `SecretString(***)` under `{:?}`
(`src/host/auth.rs`).
_Avoid_: Password, SecureBuffer, Credential.

**PassphraseCache**:
An in-memory, process-lifetime cache mapping private-key file paths to their decrypted
passphrases, avoiding repeat prompts (`src/host/auth.rs`).
_Avoid_: Keyring, AgentCache, IdentityCache.

**HostStatus**:
The command-agnostic status enum (`Online`, `Partial`, `Offline`, `Unreachable`, `TimedOut`, `Error`, `Skipped`) emitted via **ProgressSink** during execution and rendered across reporting and TUI views (`src/commands/report.rs`).
_Avoid_: NodeStatus, MachineState, ConnectionStatus.

**ProgressSink**:
The trait that decouples command-execution progress reporting from CLI printing or the TUI event
channel, so the same command code drives both surfaces (`src/commands/report.rs`).
_Avoid_: ProgressListener, ProgressCallback, EventSubscriber.

**CommandReport**:
The polymorphic enum representing the final execution summary for `check`, `run`, `exec`, `cp`,
or `sync` (`src/commands/report.rs`).
_Avoid_: ExecutionResult, CommandOutput, OperationResult.

**OperationReport**:
The serialized JSON/HTML structure written by `-o/--out`, recording operation metadata, the
target filter, tasks, and per-host results (`src/output/report.rs`).
_Avoid_: RunReport, JsonOutput, ReportDocument.

**DbHandle**:
The thread-safe handle wrapping `Arc<Mutex<rusqlite::Connection>>`, executed via
`tokio::task::spawn_blocking` (`src/state/db.rs`).
_Avoid_: Database, SqlitePool, DbConnection.

**Viewport**:
The TUI component that manages scroll offset and list-selection window invariants for scrollable
lists (`src/tui/components/viewport.rs`).
_Avoid_: ScrollState, ListState, WindowOffset.

**InputField**:
The single-line text input widget with visible cursor, grapheme-cluster navigation, Emacs keybindings, undo/kill rings, and Escape-restore semantics (`src/tui/components/input_field.rs`).
_Avoid_: TextInput, TextBox, EntryLine.

**MemberPicker**:
The modal selection popup supporting single-select or multi-select lists for groups, hosts, check entries, sync entries, or sync sources, backed by **Viewport** scrolling (`src/tui/components/member_picker.rs`).
_Avoid_: ItemSelector, CheckboxModal, SelectList, OptionPicker.

**GlyphSet**:
The status symbol mapping struct supplying Unicode icons (`✓`, `✗`, `⊘`, `⚠`) and ASCII fallback characters (`+`, `x`, `o`, `!`) for Linux virtual consoles (`TERM=linux`), mapped to **HostStatus** variants (`src/tui/theme.rs`).
_Avoid_: IconSet, SymbolTable, StatusIcons.

**EscLevel**:
The three-way TUI focus-cycling state (`NavBar` → `TopField` → `Content`) advanced by pressing
Escape (`src/tui/app_state.rs`).
_Avoid_: FocusDepth, NavigationLayer, EscapeState.

**OperationKind**:
The enum representing operations selectable on the TUI Operate tab (`Check`, `Run`, `Exec`, `Sync`, `Cp`), persisted in UI state (`src/tui/state/persist.rs`).
_Avoid_: ActionKind, SubcommandKind, OpType.

**ViewOperationKind**:
The enum representing sub-views selectable on the TUI View tab (`Checkout`, `List`, `Log`), persisted in UI state (`src/tui/state/persist.rs`).
_Avoid_: ViewMode, SubView, ViewType.
