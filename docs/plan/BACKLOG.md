# Backlog
Status: In progress

The one living record of open work. Rows move to Done with the commit that closed them and are
never deleted. Evidence is file + symbol, never a line number. `Verified` is the date the row was
last checked against the tree — not when it was opened.

Seeded 2026-09-25 from the scope gaps in [2026-07-18-audit-fixes.md](2026-07-18-audit-fixes.md),
ADR 0001/0002 follow-ups, [the 2026-05-21 TUI audit](../audit/2026-05-21-tui-audit.md) deferred list,
the one `TODO` in `src/`, and code defects found by
[the 2026-09-25 documentation audit](../audit/2026-09-25-documentation-audit.md). Items closed before
this file existed are recorded in those source documents, not here.

## Open

| ID | Opened | Verified | Pri | Finding | Evidence | Effort | Acceptance |
|---|---|---|---|---|---|---|---|
| B1 | 2026-07-18 | 2026-09-25 | P1 | PowerShell directory expansion interpolates remote paths inside double quotes, so `$(...)` subexpressions in a path execute | `src/commands/sync/collect.rs` `build_dir_expand_cmd` | S | Paths single-quoted with `''` escaping; a path containing `$(echo PWNED)` is not evaluated |
| B2 | 2026-07-18 | 2026-09-25 | P1 | TUI auth popup holds the typed credential in a plain `String`, not zeroized after submit/cancel (ADR 0001 §d) | `src/tui/app_state.rs` `AuthPopup` | S | Buffer zeroized via `zeroize` on submit and cancel |
| B3 | 2026-05-21 | 2026-09-25 | P2 | Config tab breadcrumb indexes `config.host/check/sync[*i]` directly in the FieldTable branch; stale index panics | `src/tui/tabs/config_tab.rs` `breadcrumb` | S | Uses `.get(*i)` with fallback; stale index renders without panic |
| B4 | 2026-07-18 | 2026-09-25 | P2 | `HostEntry` has no stable `id`; Config selection restore after delete is positional | `src/config/schema.rs` `HostEntry`; `src/tui/tabs/config_tab.rs` `restore_selection` | M | Deleting host 2 of 5 restores the cursor by identity |
| B5 | 2026-07-18 | 2026-09-25 | P2 | Recursive-sync drain in `sync_path_across` writes DB rows one by one outside a transaction | `src/commands/sync/mod.rs` `sync_path_across` | S | Inserts batched in one `ctx.db.transaction` |
| B6 | 2026-07-18 | 2026-09-25 | P2 | Unused focus-model types kept alive by `#![allow(dead_code)]` | `src/tui/focus.rs` | S | Types wired in or deleted; allow removed |
| B7 | 2026-07-18 | 2026-09-25 | P2 | Auth-bridge oneshot await has no timeout or cancellation (ADR 0001 §c) | `src/host/auth.rs` `authenticate` | S | `rx.await` raced against a timeout and cancel token |
| B8 | 2026-07-18 | 2026-09-25 | P3 | `batch_keyscan_and_accept` panics if the home directory cannot be resolved | `src/commands/init/core.rs` `batch_keyscan_and_accept` | S | Returns an `anyhow` error instead of `.expect` |
| B9 | 2026-07-18 | 2026-09-25 | P3 | HTML report templating lives inside the general report module | `src/output/report.rs` `render_html_report` | M | HTML rendering in its own module |
| B10 | 2026-07-18 | 2026-09-25 | P3 | Checkout metric extractors have no unit tests | `src/commands/checkout/mod.rs` `extract_metric_value` | S | Tests cover each metric for sh and PowerShell samples plus fallbacks |
| B11 | 2026-07-18 | 2026-09-25 | P3 | Kill ring is per-`InputField`; yank does not cross fields | `src/tui/components/input_field.rs` `InputField` | M | Text killed in one field can be yanked in another |
| B12 | 2026-07-18 | 2026-09-25 | P3 | Windows close button (`CTRL_CLOSE_EVENT`) not handled; `TODO(post-MVP windows)` | `src/tui/app.rs` `spawn_signal_listener` | M | Terminal restored when the console window is closed |
| B13 | 2026-09-25 | 2026-09-25 | P2 | `-v/--verbose` is not `global`, so `sshi check -a -v` is rejected | `src/cli.rs` `Cli::verbose` | S | `-v` accepted before or after the subcommand |
| B14 | 2026-09-25 | 2026-09-25 | P2 | `checkout --history` and `--since` are parsed but ignored | `src/commands/checkout/mod.rs` `run` (`_history`, `_since`) | M | Flags change output, or are removed from the CLI and docs |
| B15 | 2026-09-25 | 2026-09-25 | P3 | `init --update` is a no-op whenever `config.toml` exists (`effective_update = update \|\| config_exists`) | `src/commands/init/mod.rs` `run` | S | Flag has a distinct effect, or is removed |
| B16 | 2026-09-25 | 2026-09-25 | P3 | New-config comment template documents removed `groups`/`enable_hosts`/`enable_all` fields | `src/config/app.rs` `inject_config_comments` | S | Template mentions only fields present in `CheckEntry`/`SyncEntry` |
| B17 | 2026-09-25 | 2026-09-25 | P3 | Editor precedence differs: `sshi config` tries `$EDITOR` first, TUI `E` tries `$VISUAL` first | `src/commands/config.rs` `run`; `src/tui/app.rs` `App::do_open_editor` | S | One shared resolver, `$VISUAL` then `$EDITOR` |
| B18 | 2026-09-25 | 2026-09-25 | P2 | `distribute_pooled` acquires the global permit before the per-host one, opposite to `ConcurrencyLimiter::acquire` | `src/commands/sync/distribute.rs` `distribute_pooled` | S | Uses `ConcurrencyLimiter::acquire` (per-host first) |
| B19 | 2026-09-25 | 2026-09-25 | P3 | `sync_state` rows are written with placeholder `mtime`/`size_bytes`/`blake3` (0/0/"") and never read | `src/commands/sync/mod.rs` (inserts into `sync_state`) | M | Either real values are written and used, or the table is dropped by migration |

## Pending verification

Landed, but the check needs a platform or environment not available locally.

| Item | Closed by | Verifies when | Fallback |
|---|---|---|---|
| 30 s SSH keepalive prevents idle drops | `e4a3ebe` | Run against a server with `ClientAliveInterval 10`, `ClientAliveCountMax 0` | Lower the interval |
| WAL + `busy_timeout=5000` removes lock contention | `268cbc6` | CLI commands run while the TUI writes operation logs | Single-writer DB actor |
| One cached SFTP channel per host across many files | `cce33f7` | Trace log of a 100+ file `sshi cp` shows one channel per host | Revert to channel-per-op |

## Awaiting external

Blocked on a person or third party. **Not counted as open.**

| Item | Blocked on | Ready when |
|---|---|---|
| — | | |

## Watching

| Item | Why not now | Trigger | Re-read |
|---|---|---|---|
| Keyboard-interactive auth (ADR 0001 §a) | Password and key auth cover all reported hosts; russh client API support unverified | A user host requires keyboard-interactive, or russh API confirmed | 2026-09-25 |
| Shift+arrow selection in `InputField` | Deferred in audit-fixes E4; needs selection ranges and rendering | User request | 2026-09-25 |
| Collapse TUI `ShellMode` into `ShellType` | Persisted TUI state uses `ShellMode`; collapsing breaks saved state | A state migration layer exists, or 2.0.0 | 2026-09-25 |
| `thiserror` typed errors | `anyhow` is consistent and sufficient for a binary crate | A library crate is extracted | 2026-09-25 |
| ASCII fallback for `⏱ ▶ ◉ ○` in `GlyphSet` | Status glyphs already fall back; these rarely break | Garbled-glyph report on `TERM=linux` | 2026-09-25 |
| Streaming sync relay instead of local temp file | Disk relay is reliable at current sizes | Benchmarks show local I/O is the bottleneck | 2026-09-25 |
| UI localization | No i18n decision made; ~150 hard-coded strings | Decision to support a second language | 2026-09-25 |

## Done

| ID | Finding | Closed by |
|---|---|---|
