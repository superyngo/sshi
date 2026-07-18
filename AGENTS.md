# AGENTS.md - Shared AI Agent Prompt for sshi

## Build, Test, and Quality Commands

```bash
# Build (single sshi binary; TUI enabled via --features tui)
cargo build                                       # headless (default)
cargo build --features tui                        # with TUI
cargo build --release                             # Release headless
cargo build --release --features tui              # Release with TUI

# Check without building (faster)
cargo check
cargo check --features tui

# Run tests (both feature configurations must pass)
cargo test
cargo test --features tui
cargo test test_name                              # Run single test
cargo test -- --nocapture                         # Show print! output

# Linting (run for both feature configs)
cargo clippy --all-targets
cargo clippy --all-targets --features tui

# Formatting
cargo fmt
cargo fmt --check
```

## TUI contributor rules (per docs/tui_reconstruct_plan.md §7.3)

- No `eprintln!` / `println!` / `print!` / `eprint!` anywhere in `src/tui/`
  or in any code path reachable while the TUI is running. Use `tracing`
  macros (`error!`, `warn!`, `debug!`) instead.
- `commands::*_core` functions must never call `output::printer`. They
  receive a `ProgressSink` impl or return a `CommandReport` variant;
  printing is the CLI wrapper's responsibility.
- Each phase merges into `feat/tui` only after `cargo test`,
  `cargo test --features tui`, `cargo clippy --all-targets`,
  `cargo clippy --all-targets --features tui`, and `cargo fmt --check`
  all pass. To revert a regression, revert the merge commit on
  `feat/tui` — the branch history is the rollback.

## Code Style Guidelines

### Error Handling
- Use `anyhow::Result<T>` throughout command handlers and library modules
- Propagate with `?` and add context: `.context("description")?`
- Example: `fs::read_to_string(path).context("Failed to read config")?`

### Imports
- Group imports: std extern crates -> third-party -> local modules
- Use `use crate::module::item` for local imports
- Keep imports at file top, sorted alphabetically within groups
- Example:
  ```rust
  use std::path::PathBuf;
  use anyhow::{Context, Result};
  use tokio::process::Command;
  use crate::config::schema::HostEntry;
  ```

### Types and Naming
- Use `snake_case` for functions, modules, variables
- Use `PascalCase` for structs, enums, types
- Use `SCREAMING_SNAKE_CASE` for constants
- Prefer explicit types over `impl Trait` in public APIs
- Use `&str` for borrowable data, `String` for owned data

### Async Concurrency
- All command handlers are `async fn` returning `Result<()>`
- Use `tokio::time::timeout` for SSH operations with timeout
- Control concurrency with `tokio::sync::Semaphore` (default: 10 permits)
- Use `tokio::process::Command` for spawning ssh/scp subprocesses
- Parallelize host operations with `futures::future::join_all` or stream

### Testing
- Place tests in `#[cfg(test)]` modules at file bottom
- Write helper functions for test data setup
- Use in-memory SQLite for DB tests: `Connection::open_in_memory()`
- Test public APIs, not implementation details
- Prefix test functions with `test_`

### Shell Compatibility
- Support three shells: `Sh`, `PowerShell`, `Cmd` (from `host::shell` module)
- Use `host::shell::ShellType` enum for shell detection
- Commands must account for shell-specific syntax (paths, quoting, operators)
- Use `host::shell` module for command wrapping and temp paths

### Feature Flags
- TUI features guarded with `#[cfg(feature = "tui")]`
- Default feature set includes `tui` (ratatui, crossterm)
- Test builds with `--no-default-features` for TUI-less configs

### SSH Transport
- Use `russh` (with `russh-keys` and `russh-sftp`) as the SSH transport; see
  `docs/adr/0002-russh-migration.md` for the decision, trade-offs, and
  follow-ups.
- `~/.ssh/config` is parsed with `ssh2-config` (not via `ssh -G`) in
  `host::session_pool::load_ssh_config`. Niche directives (`Match exec`,
  `CanonicalizeHostname`, out-of-tree `Include`) may not be honoured — see
  the evaluation docs referenced in the ADR.
- `ssh-keyscan`, `ssh-keygen`, and `ssh-copy-id` remain subprocesses in
  `commands/init.rs` (key-management workflows outside russh's scope).
- Live sessions are owned by `host::session_pool::RusshSessionPool`; file
  transfer goes through `host::sftp::SftpSession`; the auth chain
  (public-key + passphrase cache + password fallback, with a TUI popup
  bridge) lives in `host::auth`.

### Database
- Use SQLite with `rusqlite` and `bundled` feature
- Enable WAL mode: `PRAGMA journal_mode=WAL;`
- Migrations are embedded via `include_str!("migrations/NXX_name.sql")`
- Track version with `PRAGMA user_version`

### Paths
- Use `dirs` crate for cross-platform paths
- Config: `dirs::config_dir()/sshi/`
- State: `dirs::state_dir()/sshi/` (fallback: `dirs::data_local_dir()/sshi/`)
- SSH config: `~/.ssh/config`

### Output Formatting
- Use `output::printer` for host-prefixed colored terminal output
- Symbols: ✓ (green success), ✗ (red error), ⊘ (yellow skip)
- Use `output::summary` for execution summaries
- Use `indicatif` for progress bars

### Logging
- Use `tracing` for structured logging
- Levels: `DEBUG` (verbose mode), `INFO` (default)
- Set filter via `tracing_subscriber::EnvFilter::from_default_env()`

### CLI Arguments
- Use `clap` with derive macros
- Use `-v` as `--version` short option
- Common args: `--group`, `--host`, `--all`, `--serial`, `--timeout`
- Flatten shared args with `#[command(flatten)]`

### Comments and Documentation
- Document public APIs with `///` doc comments
- Keep comments concise and purpose-focused
- Avoid obvious comments, add for "why" not "what"

## Architecture Overview

sshi is a CLI tool managing remote hosts over SSH. Single binary, no embedded SSH.

**Module Structure:**
- `cli.rs` - Clap CLI definitions; pre-TUI fallback help printers
- `commands/` - Subcommand handlers (one file each: init, check, run, exec, cp, log, config, checkout, list) plus `commands/sync/` (collect, decide, distribute, report, types submodules) and shared `commands/report.rs` (CommandReport type, ProgressSink)
- `config/` - Config schema, file I/O, `ssh2-config` parser
- `host/` - russh SSH transport: `session_pool.rs` (connection pool + known_hosts check), `sftp.rs` (file transfer), `auth.rs` (auth chain + `SecretString`), `concurrency.rs` (dual-level limiter), `pool.rs` (`SshPool` wrapper), `shell.rs` (shell-type detection), `filter.rs`
- `metrics/` - System metrics collection, parsing, shell-specific probes
- `state/` - SQLite DB, migrations, retention cleanup
- `output/` - Terminal printer, execution summary

**Key Data Flow:**
1. CLI args parsed → main.rs dispatches to command handler
2. Hosts filtered by --group/--host/--all via `host::filter`
3. Remote operations parallelized via Tokio (semaphore-limited)
4. All operations logged to `operation_log` table

**Sync Strategy:**
3-stage: (1) collect metadata (mtime + SHA-256), (2) decide source (newest/skip), (3) distribute via local relay
