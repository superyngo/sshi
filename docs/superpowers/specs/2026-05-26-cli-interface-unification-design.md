# CLI Interface Unification (sync / run / exec / check / checkout)

- **Date:** 2026-05-26
- **Status:** Draft (brainstorming output)
- **Scope:** Requirement #3 of the sshi modernization batch. This spec covers
  **only** the CLI surface of the five host-operating commands. The project
  rename (#1), TUI Config fixes (#2), and TUI Operate refactor (#4) are
  separate efforts with their own specs.

## Problem

The five host-operating subcommands (`sync`, `run`, `exec`, `check`,
`checkout`) have drifted into an inconsistent argument surface:

- `--dry-run` exists only on `sync` and `exec`, though it is meaningful for
  `run` and `check` too.
- `sync` carries `--no-push-missing`, an opt-out flag whose only effect is to
  flip a default the user never needs to flip in practice.
- `run` and `exec` carry `-y/--yes` ("auto-respond yes to interactive prompts,
  serial mode only"). This flag is a **complete no-op**: `run_core`/`exec_core`
  take the value as `_yes` and never read it, and the only interactive prompts
  in the codebase are SSH passphrase/password prompts (`src/host/auth.rs`),
  which `--yes` does not gate.
- There is no general way to exclude specific hosts from a resolved target set,
  even though `init` already has `--skip` for exactly this.

## Goals

1. One consistent shared target/mode argument surface across all five commands.
2. Add a common `--skip` to arbitrarily exclude hosts from the resolved set.
3. Make `--dry-run` available on every command where it is meaningful.
4. Remove dead / low-value flags (`--no-push-missing`, `-y/--yes`).

## Non-Goals

- No changes to `init`, `config`, `list`, `log` interfaces.
- No change to global flags (`-c/--config`, `-v/--verbose` stay global).
- No change to `-o/--out` (structured report) — kept on all five commands.
- No integration of CLI `--skip` with the persisted `settings.skipped_hosts`
  list. `resolve_hosts` does **not** currently honor `settings.skipped_hosts`
  for these commands, and this spec does not change that. `--skip` is a
  pure CLI-only filter. (Reconcile persisted-skip behavior in a later effort
  if desired.)
- No project rename, no TUI work.

## Design

### Approach

Keep the single shared `TargetArgs` struct (already flattened into all five
commands). Add `--skip` to it. `--dry-run` stays an inline per-command field on
the four commands that get it, matching how `sync`/`exec` already declare it.
(The alternative — splitting into separate `TargetArgs` + `ModeArgs` structs —
was rejected as cosmetic churn; YAGNI.)

### Shared `TargetArgs` (all five commands)

| Flag        | Short | Status    | Notes |
|-------------|-------|-----------|-------|
| `--group`   | `-g`  | unchanged | comma-list |
| `--host`    | `-h`  | unchanged | comma-list |
| `--all`     | `-a`  | unchanged | |
| `--shell`   | `-s`  | unchanged | comma-list of `ShellType` |
| `--skip`    | —     | **NEW**   | long-only (avoids `-s` clash); comma-list; excludes named hosts from the resolved set |
| `--serial`  | —     | unchanged | |
| `--timeout` | —     | unchanged | seconds, overrides config |
| `--help`    | `-H`  | unchanged | (`-h` is taken by `--host`) |

### Global root flags (unchanged)

`-c/--config`, `-v/--verbose` remain global on the root `Cli`. Not duplicated
per-command.

### Shared `OutputArgs` (all five commands)

`-o/--out` (optional path; `.json`/`.html`; auto-named when bare) — **kept** on
`sync`, `run`, `exec`, `check`, `checkout`. Orthogonal to this unification.

### `--dry-run`

Added to **`sync`, `exec`, `run`, `check`**. **Not** `checkout` (read-only
viewer — dry-run is meaningless there).

- `sync`, `exec`: already present, unchanged.
- `run`: **NEW** — preview the target set and the command (sudo-wrapped form
  shown) without connecting or executing.
- `check`: **NEW** — preview the target set and, **per host, the applicable
  check kinds that would be collected**, without connecting or writing
  snapshots to the state DB.

Declared as an inline `--dry-run` bool on each of the four commands.

**Implementation pattern (mirror exec):** dry-run is handled in the
command-handler wrapper, not the `*_core` fn — exactly as `exec.rs:196` does
today. The wrapper resolves hosts (and, for `check`, applicable checks via
`Context::resolve_checks`), prints `[dry-run]` per-host lines through
`printer::print_host_line`, and returns `Ok(())` before any SSH connection or
DB write. `run_core`/`check`'s core stay unchanged apart from the `_yes`
removal. This keeps "no side effects in dry-run" structurally guaranteed (the
core that performs side effects is never reached).

### Per-command special args (unchanged)

| Command    | Special args |
|------------|--------------|
| `sync`     | `-S/--source`, `-f/--files` |
| `run`      | `--sudo` (`-S`), positional `command` |
| `exec`     | `--sudo` (`-S`), `--keep`, positional `script` |
| `checkout` | `--history`, `--since` |
| `check`    | (none) |

### Removals

1. **`sync --no-push-missing`** — delete the flag. In `sync.rs`, replace
   `let push_missing = !no_push_missing;` (`src/commands/sync.rs:67`) with
   `let push_missing = true;`. Remove the field from `cli.rs` and its
   threading through `main.rs` (`src/main.rs:161,170`) and the `sync` entry fn
   signature (`src/commands/sync.rs:59`). Net behavior: push-missing is always
   on, which is today's default.

2. **`run -y/--yes` and `exec -y/--yes`** — delete the flags from `cli.rs`.
   Remove the `yes` argument from the `run`/`exec` command handlers and the
   `_yes` parameters from `run_core` (`src/commands/run.rs:23`) and `exec_core`
   (`src/commands/exec.rs:24`), plus the `main.rs` wiring. No behavior change
   (the flag was a no-op).

   **TUI call sites (must update or it won't compile):** `src/tui/app.rs` calls
   both core fns — `run_core(&ctx, &command, sudo, yes, …)` at `app.rs:629` and
   `exec_core(&ctx, &script, sudo, false, keep, …)` at `app.rs:715`. Drop the
   `yes`/`false` positional arg from both. The local `let yes = self.run_yes;`
   (`app.rs:602`) becomes orphaned — remove it.

   **Deferred to #4 (TUI Operate), not this spec:** the TUI Operate tab keeps a
   `run_yes` state field + a visible "yes" checkbox (`app.rs:139`, render +
   persistence per AD-12, help text `app.rs:2442`). Since `_yes` was always a
   no-op, that checkbox **already does nothing** — leaving it inert is not a
   regression. Removing the `run_yes` field, its checkbox, and the help text is
   left for the #4 Operate refactor. This spec only stops passing the argument.

### `--skip` semantics & placement

Apply `--skip` centrally in `Context::resolve_hosts`
(`src/commands/mod.rs:117`): after the target mode produces a host list, drop
any host whose `name` is in the skip list. Because all five commands resolve
hosts through this one method, they inherit `--skip` uniformly.

**Data flow (concrete):** `Context` (`src/commands/mod.rs:34`) has no `skip`
field today — add `skip: Vec<String>`. All three constructors must set it:
- `Context::new` (`mod.rs:46`, receives `TargetArgs`) → `skip: target.skip.clone()`.
- `Context::new_without_targets` (`mod.rs:95`, no `TargetArgs`) → `skip: vec![]`.
- `Context::from_tui_parts` (`mod.rs:74`, TUI constructor, no `TargetArgs`) → `skip: vec![]`.

`resolve_hosts` then filters the resolved list with
`!self.skip.contains(&h.name)`.

Edge cases:
- A `--skip` name that matches no resolved host is a silent no-op (consistent
  with `init`).
- `--skip` combined with `--all` excludes the named hosts from the full set.
- Skipping every resolved host yields an empty target set; commands should
  handle "no hosts" exactly as they do today for an empty resolution.

## Affected Files

- `src/cli.rs` — add `skip` to `TargetArgs`; add `dry_run` to `run`/`check`;
  remove `no_push_missing` from `sync`; remove `yes` from `run`/`exec`.
- `src/main.rs` — update the match arms / call sites for the four commands
  (drop `no_push_missing`/`yes`, pass new `dry_run`, pass `skip`).
- `src/commands/mod.rs` — add `skip` field to `Context`; set it in `new`,
  `new_without_targets`, and `from_tui_parts`; filter in `resolve_hosts`.
- `src/tui/app.rs` — drop the `yes`/`false` arg from the `run_core`
  (`app.rs:629`) and `exec_core` (`app.rs:715`) call sites; remove the orphaned
  `let yes = self.run_yes;` (`app.rs:602`). Leave the `run_yes` field/checkbox
  for #4.
- `src/commands/sync.rs` — hardwire `push_missing = true`; drop param.
- `src/commands/run.rs` — drop `_yes`; honor `dry_run` (preview path).
- `src/commands/exec.rs` — drop `_yes` (dry_run already handled).
- `src/commands/check.rs` — honor `dry_run` (preview path).

## Testing

- `cli.rs` parses each command with the new/removed flags as expected
  (clap-level: `--skip a,b`, `--dry-run` on the 4, absence of removed flags is
  an error).
- `resolve_hosts` filters skipped hosts: unit test over a small config with
  `--all --skip h2` → resolved set excludes `h2`; skip of unknown host is a
  no-op; skip-all yields empty.
- `run --dry-run` and `check --dry-run` produce a preview and make no remote
  calls / no DB writes (assert via existing dry-run test patterns).
- Regression: full `cargo test` green (baseline 152 tests).

## Verification (real binary)

- `cargo run -- run --all --dry-run 'echo hi'` previews without executing.
- `cargo run -- check --all --dry-run` previews without writing snapshots.
- `cargo run -- sync --all --skip <one-host>` excludes that host.
- `cargo run -- run --help` / `exec --help` no longer list `-y/--yes`;
  `sync --help` no longer lists `--no-push-missing`.

## Open Questions

None blocking. Persisted `settings.skipped_hosts` reconciliation is
deliberately deferred (see Non-Goals).
