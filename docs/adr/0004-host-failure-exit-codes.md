# ADR 0004 — Non-zero exit codes when hosts fail

**Status:** Implemented (2026-09-25)
**Date:** 2026-09-25

## Context

Multi-host commands (`check`, `run`, `exec`, `cp`, `sync`) exited `0` whenever orchestration finished, even when every targeted host was unreachable or every command failed (backlog B35, reproduced in the 2026-09-25 code audit: `check -a` and `run -a 'echo hi'` exit 0 with all hosts down). Scripts, cron jobs and CI could not detect a failed run without parsing the summary text or the `--out` report.

The existing codes were already taken: `1` fatal/setup error, `2` usage error (clap) or non-TTY bare `sshi`.

## Decision

After a multi-host command completes, derive the exit code from the per-host `HostStatus` values in its `CommandReport` (`CommandReport::host_outcome`):

| Exit | Condition |
|---|---|
| `0` | No host failed (skipped hosts do not count as failures) |
| `3` | At least one host failed and at least one succeeded |
| `4` | At least one host failed and none succeeded |

- **Failed:** `Offline` (including a remote command that exited non-zero), `Unreachable`, `TimedOut`, `Error`.
- **Succeeded:** `Online`, `Partial` (some check probes failed, host answered).
- **Neutral:** `Skipped`.

`1` and `2` keep their meaning and take precedence: a fatal error before or during orchestration still exits `1`.

The code is decided in one place, from the typed report every command already builds, so all five commands share the same rule.

## Alternatives rejected

- **Keep exit 0 (status quo):** the user decided on 2026-09-25 that failures must be detectable.
- **Exit `1` for any host failure:** conflates "sshi could not run" with "sshi ran, some hosts failed"; callers retrying on setup errors would retry partial failures too.
- **A single failure code:** loses the useful "partial vs total" distinction for monitoring.
- **Opt-in flag (`--strict`):** makes the safe behaviour the non-default one.

## Consequences

- Breaking change for scripts that relied on exit 0 after host failures; recorded in `CHANGELOG.md`.
- `init`, `checkout`, `list`, `log` and `config` are not host-fan-out operations with a `CommandReport` host outcome and keep their existing exit behaviour.
- `--dry-run` for `check`, `run`, `exec` and `cp` returns before connecting and exits `0`. `sync --dry-run` connects to compare files, so host failures there count.
