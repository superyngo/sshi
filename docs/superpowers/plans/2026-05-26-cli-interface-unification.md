# CLI Interface Unification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unify the argument surface of the five host-operating commands (`sync`, `run`, `exec`, `check`, `checkout`): add a common `--skip`, extend `--dry-run`, and remove dead/low-value flags (`--no-push-missing`, `-y/--yes`).

**Architecture:** `clap` derive structs in `src/cli.rs` define the surface; `src/main.rs` dispatches to per-command handlers in `src/commands/*.rs`. Target selection is resolved once in `Context::resolve_hosts` (`src/commands/mod.rs`), so a single `--skip` filter there is inherited by all five commands. `--dry-run` is handled in each command's thin CLI wrapper (mirroring `exec.rs:196`), returning before any SSH/DB side effect.

**Tech Stack:** Rust, `clap` (derive), `tokio`, `rusqlite`, `anyhow`.

**Spec:** `docs/superpowers/specs/2026-05-26-cli-interface-unification-design.md`

**Baseline:** `cargo test` is green at 152 tests before starting.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/cli.rs` | clap arg definitions | add `skip` to `TargetArgs`; add `dry_run` to `Run`/`Check`; remove `no_push_missing` (Sync) + `yes` (Run/Exec); new `#[cfg(test)]` parse tests |
| `src/commands/mod.rs` | `Context`, target resolution | add `skip` field + `filter_skipped` helper; set field in 3 constructors; filter in `resolve_hosts`; unit test |
| `src/main.rs` | command dispatch | update Sync/Run/Exec/Check match arms |
| `src/commands/sync.rs` | sync handler/core | drop `no_push_missing` param; hardwire `push_missing = true` |
| `src/commands/run.rs` | run handler/core | drop `_yes`; add `dry_run` preview wrapper |
| `src/commands/exec.rs` | exec handler/core | drop `_yes` (dry_run already present) |
| `src/commands/check.rs` | check handler/core | add `dry_run` preview wrapper (per-host applicable checks) |
| `src/tui/app.rs` | TUI operate driver | drop `yes`/`false` arg from core call sites; remove orphaned local |

---

## Task 1: Common `--skip` target filter

**Files:**
- Modify: `src/cli.rs` (`TargetArgs`, ~line 41) + new test module
- Modify: `src/commands/mod.rs` (`Context` struct ~line 34; constructors ~46/74/95; `resolve_hosts` ~117; new helper + test)

- [ ] **Step 1: Write the failing parse test for `--skip`**

Append to `src/cli.rs` (create a `#[cfg(test)]` module at end of file):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn skip_parses_as_comma_list_on_check() {
        let cli = Cli::try_parse_from(["ssync", "check", "--all", "--skip", "h1,h2"]).unwrap();
        match cli.command.unwrap() {
            Commands::Check { target, .. } => {
                assert_eq!(target.skip, vec!["h1".to_string(), "h2".to_string()]);
            }
            _ => panic!("expected Check"),
        }
    }
}
```

- [ ] **Step 2: Run it — verify it fails (no `skip` field yet)**

Run: `cargo test --lib skip_parses_as_comma_list_on_check`
Expected: FAIL — compile error, `no field skip on type TargetArgs`.

- [ ] **Step 3: Add the `skip` field to `TargetArgs`**

In `src/cli.rs`, inside `pub struct TargetArgs`, after the `shell` field (line 43) add:

```rust
    /// Skip specific hosts (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub skip: Vec<String>,
```

- [ ] **Step 4: Run the test — verify it passes**

Run: `cargo test --lib skip_parses_as_comma_list_on_check`
Expected: PASS.

- [ ] **Step 5: Write the failing unit test for `filter_skipped`**

In `src/commands/mod.rs`, add a test module at end of file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{HostEntry, ShellType};

    fn host(name: &str) -> HostEntry {
        HostEntry {
            name: name.to_string(),
            ssh_host: name.to_string(),
            groups: vec![],
            shell: ShellType::Sh,
            proxy_jump: None,
        }
    }

    #[test]
    fn filter_skipped_removes_named_hosts() {
        let h1 = host("h1");
        let h2 = host("h2");
        let h3 = host("h3");
        let all: Vec<&HostEntry> = vec![&h1, &h2, &h3];

        let kept = filter_skipped(all.clone(), &["h2".to_string()]);
        let names: Vec<&str> = kept.iter().map(|h| h.name.as_str()).collect();
        assert_eq!(names, vec!["h1", "h3"]);

        // unknown skip is a no-op
        let kept = filter_skipped(all.clone(), &["nope".to_string()]);
        assert_eq!(kept.len(), 3);

        // skip-all yields empty
        let kept = filter_skipped(all, &["h1".into(), "h2".into(), "h3".into()]);
        assert!(kept.is_empty());
    }
}
```

> Verified against `src/config/schema.rs:117`: `HostEntry { name, ssh_host, shell, groups, proxy_jump }` — the `host()` helper above sets exactly these five fields, so it compiles as written.

- [ ] **Step 6: Run it — verify it fails (`filter_skipped` undefined)**

Run: `cargo test --lib filter_skipped_removes_named_hosts`
Expected: FAIL — `cannot find function filter_skipped`.

- [ ] **Step 7: Add the `filter_skipped` helper**

In `src/commands/mod.rs`, add a free function near `resolve_target_mode` (it is a module-private helper):

```rust
/// Drop any host whose name appears in `skip`. Unknown names are ignored.
fn filter_skipped<'a>(hosts: Vec<&'a HostEntry>, skip: &[String]) -> Vec<&'a HostEntry> {
    if skip.is_empty() {
        return hosts;
    }
    hosts
        .into_iter()
        .filter(|h| !skip.iter().any(|s| s == &h.name))
        .collect()
}
```

- [ ] **Step 8: Run the test — verify it passes**

Run: `cargo test --lib filter_skipped_removes_named_hosts`
Expected: PASS.

- [ ] **Step 9: Add the `skip` field to `Context` and wire all three constructors**

In `src/commands/mod.rs`:

Add to the `pub struct Context` (after `pub serial: bool,`):

```rust
    pub skip: Vec<String>,
```

In `Context::new`, add to the returned struct literal (after `serial: target.serial,`):

```rust
            skip: target.skip.clone(),
```

In `Context::from_tui_parts`, add to the struct literal (after `serial,`):

```rust
            skip: Vec::new(),
```

In `Context::new_without_targets`, add to the struct literal (after `serial: false,`):

```rust
            skip: Vec::new(),
```

- [ ] **Step 10: Apply the filter in `resolve_hosts`**

In `src/commands/mod.rs`, in `resolve_hosts`, immediately after the `match &self.mode { ... };` block that binds `hosts` and **before** the `if hosts.is_empty()` check, insert:

```rust
        let hosts = filter_skipped(hosts, &self.skip);
```

This keeps the existing empty-set `bail!` as the "no hosts after skip" behavior (per spec).

- [ ] **Step 11: Run the full suite — verify green**

Run: `cargo test`
Expected: PASS — baseline 152 + 2 new tests.

- [ ] **Step 12: Commit**

```bash
git add src/cli.rs src/commands/mod.rs
git commit -m "feat(cli): add common --skip host filter across the 5 host commands

--skip is added to shared TargetArgs and applied centrally in
resolve_hosts via a new Context.skip field, so sync/run/exec/check/checkout
all inherit it. Unknown names are no-ops; skipping all hosts triggers the
existing empty-set error.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 2: Remove `sync --no-push-missing`

**Files:**
- Modify: `src/cli.rs` (`Commands::Sync`, lines 135-137) + test
- Modify: `src/main.rs` (`Commands::Sync` arm, lines 157-175)
- Modify: `src/commands/sync.rs` (handler signature line 59; `push_missing` line 67)

- [ ] **Step 1: Write the failing test — flag must be rejected**

Add to the `#[cfg(test)] mod tests` in `src/cli.rs`:

```rust
    #[test]
    fn sync_rejects_no_push_missing() {
        let err = Cli::try_parse_from(["ssync", "sync", "--all", "--no-push-missing"]);
        assert!(err.is_err(), "--no-push-missing should no longer be accepted");
    }

    #[test]
    fn sync_still_parses_without_removed_flag() {
        assert!(Cli::try_parse_from(["ssync", "sync", "--all"]).is_ok());
    }
```

- [ ] **Step 2: Run it — verify it fails**

Run: `cargo test --lib sync_rejects_no_push_missing`
Expected: FAIL — the flag is still accepted, so `is_err()` is false.

- [ ] **Step 3: Remove the flag from `cli.rs`**

In `src/cli.rs`, in `Commands::Sync`, delete:

```rust
        /// Don't push files to hosts that are missing them
        #[arg(long)]
        no_push_missing: bool,
```

- [ ] **Step 4: Update `main.rs` dispatch**

In `src/main.rs`, change the `Commands::Sync` arm to drop `no_push_missing`:

```rust
        Commands::Sync {
            target,
            dry_run,
            files,
            source,
            output,
        } => {
            let ctx = commands::Context::new(cli.verbose, &target, cfg).await?;
            commands::sync::run(&ctx, dry_run, &files, source.as_deref(), &output).await
        }
```

- [ ] **Step 5: Update `sync::run` signature and hardwire `push_missing`**

In `src/commands/sync.rs`, change the handler signature (around line 55-60) to remove the `no_push_missing: bool` parameter, and change line 67:

```rust
    let push_missing = true;
```

(Delete the old `let push_missing = !no_push_missing;`. The `no_push_missing` parameter is gone from the signature; everything downstream already takes `push_missing: bool`.)

- [ ] **Step 6: Build + run the new tests**

Run: `cargo test --lib sync_rejects_no_push_missing sync_still_parses_without_removed_flag`
Expected: PASS (and the crate compiles — confirms `main.rs`/`sync.rs` call sites match).

- [ ] **Step 7: Run the full suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add src/cli.rs src/main.rs src/commands/sync.rs
git commit -m "refactor(cli): remove sync --no-push-missing (push-missing always on)

The flag only flipped a default no one disables in practice. push_missing
is now hardwired true, matching prior default behavior.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 3: Remove `-y/--yes` from `run` and `exec`

**Files:**
- Modify: `src/cli.rs` (`Commands::Run` lines 159-162, `Commands::Exec` lines 180-182) + test
- Modify: `src/main.rs` (`Commands::Run` ~176, `Commands::Exec` ~186)
- Modify: `src/commands/run.rs` (`run_core` `_yes` line 23; handler `run` `yes` line 173; call site line 177)
- Modify: `src/commands/exec.rs` (`exec_core` `_yes` line 24; handler `run` `yes` line 189; call site line 226)
- Modify: `src/tui/app.rs` (line 602 orphan; line 629 `run_core` call; line 715 `exec_core` call)

- [ ] **Step 1: Write the failing test — `-y/--yes` must be rejected**

Add to `#[cfg(test)] mod tests` in `src/cli.rs`:

```rust
    #[test]
    fn run_rejects_yes() {
        assert!(Cli::try_parse_from(["ssync", "run", "--all", "--yes", "echo hi"]).is_err());
        assert!(Cli::try_parse_from(["ssync", "run", "--all", "-y", "echo hi"]).is_err());
    }

    #[test]
    fn exec_rejects_yes() {
        assert!(Cli::try_parse_from(["ssync", "exec", "--all", "--yes", "s.sh"]).is_err());
    }
```

- [ ] **Step 2: Run it — verify it fails**

Run: `cargo test --lib run_rejects_yes exec_rejects_yes`
Expected: FAIL — `--yes` still accepted.

- [ ] **Step 3: Remove the flags from `cli.rs`**

In `Commands::Run`, delete:

```rust
        /// Auto-respond yes to interactive prompts (serial mode only)
        #[arg(short, long)]
        yes: bool,
```

In `Commands::Exec`, delete the identical `yes` block.

- [ ] **Step 4: Drop `_yes` from `run_core` and the `run` handler**

In `src/commands/run.rs`:
- Remove the `_yes: bool,` parameter from `run_core` (line 23).
- Remove the `yes: bool,` parameter from the `run` handler (line 173).
- Change the call site (line 177) from `run_core(ctx, command, sudo, yes, Some(&sink))` to `run_core(ctx, command, sudo, Some(&sink))`.

- [ ] **Step 5: Drop `_yes` from `exec_core` and the `exec` handler**

In `src/commands/exec.rs`:
- Remove the `_yes: bool,` parameter from `exec_core` (line 24).
- Remove the `yes: bool,` parameter from the `run` handler (line 189).
- Change the call site (line 226) from `exec_core(ctx, script, sudo, yes, keep, Some(&sink))` to `exec_core(ctx, script, sudo, keep, Some(&sink))`.

- [ ] **Step 6: Update `main.rs` dispatch arms**

`Commands::Run` arm:

```rust
        Commands::Run {
            target,
            command,
            sudo,
            output,
        } => {
            let ctx = commands::Context::new(cli.verbose, &target, cfg).await?;
            commands::run::run(&ctx, &command, sudo, &output).await
        }
```

`Commands::Exec` arm:

```rust
        Commands::Exec {
            target,
            script,
            sudo,
            keep,
            dry_run,
            output,
        } => {
            let ctx = commands::Context::new(cli.verbose, &target, cfg).await?;
            commands::exec::run(&ctx, &script, sudo, keep, dry_run, &output).await
        }
```

- [ ] **Step 7: Update the TUI call sites in `app.rs`**

In `src/tui/app.rs`:
- Delete line 602: `let yes = self.run_yes;`
- Line 629: change `run_core(&ctx, &command, sudo, yes, Some(&sink))` to `run_core(&ctx, &command, sudo, Some(&sink))`.
- Line 715: change `exec_core(&ctx, &script, sudo, false, keep, Some(&sink))` to `exec_core(&ctx, &script, sudo, keep, Some(&sink))`.

> Leave the `run_yes` field, its checkbox, persistence, and help text untouched — that inert toggle is scoped to the #4 TUI Operate refactor (see spec). `self.run_yes` remains read elsewhere (render/persistence), so removing only the local `let yes` binding will not leave `run_yes` unused.

- [ ] **Step 8: Build (with TUI feature) + run the new tests**

Run: `cargo test --lib run_rejects_yes exec_rejects_yes`
Then: `cargo build --features tui`
Expected: tests PASS; build succeeds (confirms all call sites updated, including the TUI path).

- [ ] **Step 9: Run the full suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add src/cli.rs src/main.rs src/commands/run.rs src/commands/exec.rs src/tui/app.rs
git commit -m "refactor(cli): remove run/exec -y/--yes (was a no-op)

The flag was never read (run_core/exec_core took _yes and ignored it; the
only prompts are SSH passphrase/password, which it never gated). Drops the
flag, the dead params, and the arg at TUI call sites. The TUI Operate yes
checkbox is left inert for the #4 refactor.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 4: Add `--dry-run` to `run`

**Files:**
- Modify: `src/cli.rs` (`Commands::Run`) + test
- Modify: `src/main.rs` (`Commands::Run` arm)
- Modify: `src/commands/run.rs` (handler `run`)

- [ ] **Step 1: Write the failing parse test**

Add to `#[cfg(test)] mod tests` in `src/cli.rs`:

```rust
    #[test]
    fn run_parses_dry_run() {
        let cli = Cli::try_parse_from(["ssync", "run", "--all", "--dry-run", "echo hi"]).unwrap();
        match cli.command.unwrap() {
            Commands::Run { dry_run, .. } => assert!(dry_run),
            _ => panic!("expected Run"),
        }
    }
```

- [ ] **Step 2: Run it — verify it fails**

Run: `cargo test --lib run_parses_dry_run`
Expected: FAIL — `no field dry_run on Run` / `--dry-run` unexpected.

- [ ] **Step 3: Add `dry_run` to `Commands::Run` in `cli.rs`**

Inside `Commands::Run`, after the `sudo` field, add:

```rust
        /// Preview the command and target set without executing
        #[arg(long)]
        dry_run: bool,
```

- [ ] **Step 4: Thread `dry_run` through `main.rs`**

Update the `Commands::Run` arm:

```rust
        Commands::Run {
            target,
            command,
            sudo,
            dry_run,
            output,
        } => {
            let ctx = commands::Context::new(cli.verbose, &target, cfg).await?;
            commands::run::run(&ctx, &command, sudo, dry_run, &output).await
        }
```

- [ ] **Step 5: Add the `dry_run` preview to the `run` handler**

In `src/commands/run.rs`, add `dry_run: bool` to the `run` handler signature (after `sudo: bool,`). At the top of the function body, before `let sink = PrinterSink;`, insert (mirrors `exec.rs:196`):

```rust
    if dry_run {
        let display = if sudo {
            shell::sudo_wrap(ShellType::Sh, command)
        } else {
            command.to_string()
        };
        println!("[dry-run] Command: {}", display);
        let hosts = ctx.resolve_hosts()?;
        for host in &hosts {
            printer::print_host_line(&host.name, "ok", "would execute");
        }
        return Ok(());
    }
```

Ensure the needed imports exist in `run.rs`: `use crate::host::shell;` (already present, line 6), `use crate::output::printer;` (present, line 7), and `use crate::config::schema::ShellType;` — add this import if missing (check with `rg -n "ShellType" src/commands/run.rs`; the preview only needs a placeholder shell for the sudo-wrapped display, since the real per-host shell isn't connected in dry-run).

> Design note: in dry-run we show one representative sudo-wrapped form using `ShellType::Sh` rather than per-host wrapping, because the point is to preview intent, not exact per-shell quoting. If you prefer exact per-host wrapping, wrap inside the host loop using `host.shell`; either is acceptable per spec. Default to the simple single-line form above.

- [ ] **Step 6: Run the parse test + build**

Run: `cargo test --lib run_parses_dry_run`
Then: `cargo build`
Expected: PASS + compiles.

- [ ] **Step 7: Verify on the real binary (no side effects)**

Run: `cargo run -- run --all --dry-run 'echo hi'`
Expected: prints `[dry-run] Command: echo hi` and a `would execute` line per host; **no SSH connection, no execution, no operation_log row**. (If you have no hosts configured, it prints the "No hosts matched" error from `resolve_hosts` — that is correct.)

- [ ] **Step 8: Run the full suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/cli.rs src/main.rs src/commands/run.rs
git commit -m "feat(cli): add run --dry-run preview

Previews the command and resolved target set without connecting or
executing, mirroring exec's dry-run wrapper (returns before run_core).

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 5: Add `--dry-run` to `check` (per-host applicable checks)

**Files:**
- Modify: `src/cli.rs` (`Commands::Check`) + test
- Modify: `src/main.rs` (`Commands::Check` arm)
- Modify: `src/commands/check.rs` (handler `run`, line 250)

- [ ] **Step 1: Write the failing parse test**

Add to `#[cfg(test)] mod tests` in `src/cli.rs`:

```rust
    #[test]
    fn check_parses_dry_run() {
        let cli = Cli::try_parse_from(["ssync", "check", "--all", "--dry-run"]).unwrap();
        match cli.command.unwrap() {
            Commands::Check { dry_run, .. } => assert!(dry_run),
            _ => panic!("expected Check"),
        }
    }
```

- [ ] **Step 2: Run it — verify it fails**

Run: `cargo test --lib check_parses_dry_run`
Expected: FAIL — `Commands::Check` has no `dry_run` field.

- [ ] **Step 3: Add `dry_run` to `Commands::Check` in `cli.rs`**

`Commands::Check` currently only flattens `target` + `output`. Add a `dry_run` field:

```rust
    /// Collect system snapshots from hosts and store in state DB
    #[command(disable_help_flag = true)]
    Check {
        #[command(flatten)]
        target: TargetArgs,

        /// Preview which hosts/checks would run without collecting or writing
        #[arg(long)]
        dry_run: bool,

        #[command(flatten)]
        output: OutputArgs,
    },
```

- [ ] **Step 4: Thread `dry_run` through `main.rs`**

Update the `Commands::Check` arm:

```rust
        Commands::Check { target, dry_run, output } => {
            let ctx = commands::Context::new(cli.verbose, &target, cfg).await?;
            commands::check::run(&ctx, dry_run, &output).await
        }
```

- [ ] **Step 5: Add `dry_run` preview to the `check` handler**

In `src/commands/check.rs`, add `dry_run: bool` to the `run` handler signature (between `ctx` and `output`):

```rust
pub async fn run(ctx: &Context, dry_run: bool, output: &crate::cli::OutputArgs) -> Result<()> {
```

At the very top of the body (before the existing `host_configs_empty_hint` block), insert the preview, reusing the existing `build_host_check_configs` helper:

```rust
    if dry_run {
        let hosts = ctx.resolve_hosts()?;
        let configs = build_host_check_configs(ctx, &hosts);
        for host in &hosts {
            match configs.get(&host.name) {
                Some((enabled, _paths)) if !enabled.is_empty() => {
                    printer::print_host_line(
                        &host.name,
                        "ok",
                        &format!("would collect: {}", enabled.join(", ")),
                    );
                }
                _ => printer::print_host_line(&host.name, "skip", "no checks apply"),
            }
        }
        return Ok(());
    }
```

(`printer` is already imported at `check.rs:9`; `build_host_check_configs` and `HostCheckConfig` are defined in the same file.)

- [ ] **Step 6: Run the parse test + build**

Run: `cargo test --lib check_parses_dry_run`
Then: `cargo build`
Expected: PASS + compiles.

- [ ] **Step 7: Verify on the real binary (no DB writes)**

Run: `cargo run -- check --all --dry-run`
Expected: one line per host showing `would collect: <metric,…>` (or `no checks apply`); **no SSH connection, no snapshot rows inserted**. Cross-check with `cargo run -- log --last 5` that no new `check` entries appeared from the dry-run.

- [ ] **Step 8: Run the full suite**

Run: `cargo test`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add src/cli.rs src/main.rs src/commands/check.rs
git commit -m "feat(cli): add check --dry-run preview with per-host applicable checks

Lists each target host and the check kinds that would be collected (via
build_host_check_configs) without connecting or writing to the state DB.

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Task 6: Update CHANGELOG

**Files:**
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add an Unreleased entry**

Under `## [Unreleased]`, add a dated block:

```markdown
### 2026-05-26 — CLI interface unification (sync/run/exec/check/checkout)
- feat(cli): common `--skip <hosts>` on all five host commands; filtered
  centrally in `resolve_hosts` (unknown names no-op; skip-all → no-hosts error).
- feat(cli): `--dry-run` added to `run` (preview command + targets) and
  `check` (preview per-host applicable checks); both return before any SSH/DB
  side effect. `sync`/`exec` dry-run unchanged. `checkout` unaffected (read-only).
- refactor(cli): removed `sync --no-push-missing` (push-missing always on,
  matching prior default).
- refactor(cli): removed `run`/`exec` `-y/--yes` (was a no-op; dead params and
  TUI call-site args dropped). The TUI Operate "yes" checkbox is left inert
  pending the #4 Operate refactor.
- `-o/--out` retained on all five commands.
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(changelog): CLI interface unification (req #3)

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

## Final Verification

- [ ] **Full suite green:** `cargo test` → all pass (baseline 152 + 6 new CLI/helper tests).
- [ ] **Clippy clean:** `cargo clippy --all-targets --features tui -- -D warnings` (the project's lint bar; fix any orphan-import warnings from removed params).
- [ ] **Help surfaces correct (real binary):**
  - `cargo run -- sync -H` → no `--no-push-missing`.
  - `cargo run -- run -H` → no `-y/--yes`; has `--dry-run`, `--skip`.
  - `cargo run -- exec -H` → no `-y/--yes`.
  - `cargo run -- check -H` → has `--dry-run`, `--skip`.
  - `cargo run -- checkout -H` → has `--skip`; **no** `--dry-run`.
- [ ] **Skip works (real binary):** `cargo run -- list --all --skip <one-host-name>` (or `check --all --dry-run --skip <host>`) excludes that host from the resolved set.
