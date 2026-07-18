# ADR 0002 — russh as SSH transport

**Status:** Accepted
**Date:** 2026-04-27 (migration landed); recorded retroactively 2026-07-18
**Supersedes:** the "NEVER use embedded SSH libraries" guidance previously in `AGENTS.md` (originally written for the subprocess-based transport)

## Context

sshi's original transport shelled out to the system `ssh` and `scp` binaries via `tokio::process::Command`. The stated rationale (recorded in `AGENTS.md`) was automatic `~/.ssh/config` compatibility — including `ProxyJump`, `ssh-agent`, `IdentityFile`, and per-host option overrides — without re-implementing an SSH config parser.

This approach worked for POSIX-flavoured development but had three structural problems that surfaced as the project grew:

1. **Windows multiplexing was effectively unavailable.** OpenSSH for Windows does not expose a `ControlMaster` equivalent. Every `ssh` invocation paid the full TCP + TLS + auth handshake, making `check`/`sync` against Windows hosts several times slower than against Linux hosts and breaking the per-host concurrency budget.
2. **File-transfer behaviour diverged across `scp` implementations.** The OpenSSH 9.0+ `scp` switched to SFTP protocol by default; older systems and PuTTY's `pscp` did not. Quoting, recursive-transfer semantics, and error reporting were inconsistent.
3. **`ProxyJump` and `IdentityFile` resolution depended on the user's local `ssh` version.** Some users on older system-ssh builds could not use modern `~/.ssh/config` features; on Windows, the bundled OpenSSH sometimes disagreed with system `ssh`.

`docs/russh-migration-evaluation.md` and `docs/openssh-migration-evaluation.md` (both in this repo) capture the full evaluation that led to this decision.

## Decision

**Adopt `russh` (with `russh-keys` and `russh-sftp`) as the SSH transport**, replacing all `tokio::process::Command::new("ssh" | "scp")` calls in the steady-state command path.

Concretely:

- Add `russh = "0.44"`, `russh-keys = "0.44"`, `russh-sftp = "2.1"` as direct dependencies.
- Introduce `host::session_pool::RusshSessionPool` as the single owner of live SSH sessions, with `connect`, `exec`, `upload`, `download`, and SFTP-probe APIs.
- Parse `~/.ssh/config` with `ssh2-config` rather than relying on `ssh -G` — `host::session_pool::load_ssh_config` is the canonical entry point.
- Keep `ssh-keyscan`, `ssh-keygen`, and `ssh-copy-id` as subprocesses in `commands/init.rs`. These cover key-management workflows that russh does not aim to replace; the `init` flow is interactive and infrequent, so the cost of spawning processes is acceptable.

## Consequences

**Positive:**

- Cross-platform behaviour is identical: Windows, macOS, and Linux all use the same Rust transport.
- `ProxyJump` and `IdentityFile` are resolved by `ssh2-config` and applied inside russh — no dependence on the user's local `ssh` version.
- SFTP replaces `scp` for file transfer, giving consistent quoting, recursive-transfer semantics, and a single error-reporting path.
- One persistent session per host enables future SFTP-channel caching, key re-use, and keepalive policies that subprocess-per-op could not support.

**Negative:**

- `~/.ssh/config` compatibility is now bounded by what `ssh2-config` parses, not by what the user's `ssh` would have accepted. Niche directives (e.g. `Match exec`, `CanonicalizeHostname`, `Include` of files outside `~/.ssh/`) may not be honoured. The team accepted this trade-off; evaluation doc lists known gaps.
- Known-hosts matching changed semantics. russh's `check_known_hosts` matches plain `host` / `[host]:port` tokens by string equality and does not match hashed (`|1|`) entries — `init` was updated to write unhashed entries (see CHANGELOG `[v1.6.0] - 2026-06-11` fix).
- The russh `Handler` trait requires boxed async fns (`Pin<Box<...>>`); the `#[allow(clippy::manual_async_fn)]` annotation at `host/session_pool.rs:26` is permanent.
- `rpassword::prompt_password` (used for passphrase and password prompts) is a blocking call. In TUI mode this is plumbed through an auth-bridge channel (`SshAuthRequest`); at time of writing the bridge is wired but not yet emitted from `host::auth` — see audit `docs/ai-reports/AUDIT_2026-07-18.md` §3.1 for the open task.

## Implementation references

- Migration plan (task-by-task): `docs/superpowers/plans/2026-04-27-russh-migration.md`
- Pre-decision evaluations: `docs/russh-migration-evaluation.md`, `docs/openssh-migration-evaluation.md`
- Implementation: `src/host/{session_pool.rs, sftp.rs, auth.rs}`
- Related ADR: `docs/adr/ssh-auth-tui-popup.md` (the TUI auth popup contract)
- Release that shipped the migration: `[v1.6.0]` family (CHANGELOG)

## Follow-ups

The 2026-07-18 audit surfaced three russh-adjacent items that should be tracked separately:

1. Cache `SftpSession` per host instead of re-opening the subsystem on every op (audit §3.4).
2. Configure SSH keepalive (audit §3.1).
3. Wire the TUI auth bridge end-to-end (audit §3.1) — passphrase/password auth is non-functional in TUI mode until this lands.

These are scheduled in Phases C1–C3 of `docs/plans/2026-07-18-audit-fixes.md`.
