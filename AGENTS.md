# AGENTS.md - Shared AI Agent Prompt for sshi

## Build, Test, and Quality Commands

```bash
# Build (single sshi binary; TUI enabled by default)
cargo build                                       # with TUI (default)
cargo build --no-default-features                 # headless
cargo build --release                             # Release with TUI (default)
cargo build --release --no-default-features       # Release headless

# Check without building (faster)
cargo check
cargo check --no-default-features

# Run tests (both feature configurations must pass)
cargo test
cargo test --no-default-features
cargo test test_name                              # Run single test
cargo test -- --nocapture                         # Show print! output

# Linting (run for both feature configs)
cargo clippy --all-targets
cargo clippy --all-targets --no-default-features

# Formatting
cargo fmt
cargo fmt --check
```

## TUI contributor rules

- No `eprintln!` / `println!` / `print!` / `eprint!` anywhere in `src/tui/`
  or in any code path reachable while the TUI is running. Use `tracing`
  macros (`error!`, `warn!`, `debug!`) instead.
- `commands::*_core` functions must never call `output::printer`. They
  receive a `ProgressSink` impl or return a `CommandReport` variant;
  printing is the CLI wrapper's responsibility.
- Every change merges into `main` only after `cargo test`,
  `cargo test --no-default-features`, `cargo clippy --all-targets`,
  `cargo clippy --all-targets --no-default-features`, and `cargo fmt --check`
  all pass with no warnings (CI, `.github/workflows/ci.yml`, enforces this with
  `RUSTFLAGS=-D warnings` on ubuntu and macos). To revert a regression, revert the merge commit on
  `main` — the branch history is the rollback.

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

### Testing
- Place tests in `#[cfg(test)]` modules at file bottom
- Write helper functions for test data setup
- Use in-memory SQLite for DB tests: `Connection::open_in_memory()`
- Test public APIs, not implementation details
- Use descriptive test function names; `test_` prefix is optional

### Feature Flags
- TUI features guarded with `#[cfg(feature = "tui")]`
- Default feature set includes `tui` (ratatui, crossterm)
- Test builds with `--no-default-features` for TUI-less configs

### Comments and Documentation
- Document public APIs with `///` doc comments
- Keep comments concise and purpose-focused
- Avoid obvious comments, add for "why" not "what"

See [CONTEXT.md](CONTEXT.md) for architecture, config, transport, sync, state, and TUI reference documentation.

## Changelog

- Root `CHANGELOG.md` holds `[Unreleased]` plus the current major series only; older series
  live verbatim in `docs/reference/changelog/`. Never archive the series the next tag belongs to.
