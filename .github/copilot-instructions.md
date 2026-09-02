# Copilot Instructions for sshi

## Build & Test

```bash
# Build
cargo build

# Build without TUI feature
cargo build --no-default-features

# Run tests
cargo test

# Run a single test
cargo test test_name

# Run tests in a specific module
cargo test config::ssh_config::tests

# Check without building
cargo check

# Lint
cargo clippy
```

## Architecture

sshi is documented in [CONTEXT.md](../CONTEXT.md) — start there. Current behavior (CLI surface,
config schema, SSH transport, sync algorithm, state schema, TUI) lives in `docs/reference/`.

## Conventions

See [AGENTS.md](../AGENTS.md) for code style, error handling, and contributor conventions.
