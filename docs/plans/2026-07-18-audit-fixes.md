# Audit Fixes Implementation Plan

**Date:** 2026-07-18
**Source:** `docs/ai-reports/AUDIT_2026-07-18.md` — 116 findings (23 HIGH, 51 MED, 42 LOW)
**Status:** Implementation-ready, awaiting execution

> **For the next session:** Read `docs/ai-reports/AUDIT_2026-07-18.md` for full evidence per finding. Each task below links back to its source audit section. Work phases top-to-bottom; each phase is independently shippable. Run the verification block at the end of every task before declaring it complete.

---

## Pre-flight (every session)

```bash
git status                                # confirm clean on main
git pull --ff-only                        # sync with origin
cargo build                               # headless
cargo build --features tui                # TUI
cargo test
cargo test --features tui
cargo clippy --all-targets --features tui -- -D warnings
cargo fmt --check
```

If any of the above fails before you start, **stop and report** — do not attempt fixes on top of a broken tree.

---

## Phase A — Quick wins (≤1 day each, low risk)

Each task is small, isolated, and ships as its own commit.

### A1. Add `[profile.release]` overrides
**Files:** `Cargo.toml`
**Change:** Append
```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
```
**Do NOT add** `panic = "abort"` — codebase relies on JoinHandle catching task panics.
**Source:** audit §3.9 HIGH
**Verify:** `cargo build --release` succeeds; binary ~50% smaller.

### A2. Add SQLite PRAGMAs
**Files:** `src/state/db.rs:48`
**Change:** After `PRAGMA journal_mode=WAL;` add `PRAGMA busy_timeout=5000;` and `PRAGMA synchronous=NORMAL;`.
**Source:** audit §3.5 HIGH ×2
**Verify:** `cargo test` passes; new test asserting `busy_timeout` value reads back 5000.

### A3. Stop swallowing DB write errors
**Files (8 sites):** `commands/sync/mod.rs:559,570,898,909`; `commands/check.rs:206,238`; `commands/cp.rs:151`; `commands/exec.rs:335`
**Change:** Replace `let _ = ctx.db.execute(...)` with
```rust
if let Err(e) = ctx.db.execute(...) {
    tracing::warn!(error = %e, "failed to record operation_log entry");
}
```
**Source:** audit §2.1 HIGH
**Verify:** `cargo test` passes; `rg "let _ = ctx.db.execute"` returns nothing.

### A4. Convert `Vec::contains` to `HashSet` in sync
**Files:** `src/commands/sync/mod.rs:305,372,504`; `src/commands/sync/collect.rs:85,91,268-285`
**Change:** Maintain a `HashSet<String>` seen-set alongside `new_paths`/`paths`; check membership on the set, push to the Vec.
**Source:** audit §3.7 HIGH ×2
**Verify:** `cargo test --features tui sync` passes; new unit test exercising 10k-file expansion completes in <1s.

### A5. Move PowerShell interpolation to single-quotes
**Files:** `src/commands/sync/collect.rs:118-159,317-383`
**Change:** Replace PowerShell double-quoted path interpolation (`"$path"`) with single-quoted (`'$path'` — escape any embedded single-quote as `''`). For CMD shell, keep current `^` escaping.
**Source:** audit §2.7 MED (HIGH security)
**Verify:** New unit test: a path containing `$(echo PWNED)` does not execute the subexpression when run through the PowerShell command builder.

### A6. Fix `host/auth.rs` zeroization + hostname
**Files:** `src/host/auth.rs:74-82,90-91`
**Change:**
1. Wrap `pp: String` in `SecretString` immediately on receipt; clone the `SecretString` (preserves zeroize-on-drop) for cache insert.
2. Pass real `host_name` to the password prompt format string — replace literal `<host>`.
**Source:** audit §2.7 HIGH ×2
**Verify:** `cargo test host::auth` passes; new test asserting the prompt contains the actual hostname.

### A7. Fix dry-run persistence round-trip
**Files:** `src/tui/state/persist.rs`; `src/tui/app_state.rs:152`; `src/tui/app.rs:638`
**Change:**
1. Add `dry_run: bool` to `persist::OperateState` (default false via serde).
2. `OperateState::new` reads `dry_run` from `persisted.dry_run` (not `sync_dry_run`).
3. `save_state` writes `self.operate.dry_run` into `persist.dry_run`.
4. Keep `sync_dry_run` separate for sync-specific overrides.
**Source:** audit §1 P1 HIGH
**Verify:** Manual — toggle shared dry-run, quit TUI, relaunch, confirm flag persisted. New unit test on persist round-trip.

### A8. Fix `ConcurrencyLimiter` head-of-line blocking
**Files:** `src/host/concurrency.rs:32-43`
**Change:** Acquire per-host permit first, then global. Alternatively drop the global sem entirely and let per-host limits compose (decide based on existing tests' intent).
**Source:** audit §3.2 HIGH
**Verify:** Existing concurrency tests pass; new test: 10 tasks targeting host A + 1 task targeting host B — host B's task starts within 1s of spawn.

### A9. Replace `eprintln!`/`println!` in `src/tui/`
**Files:** `src/tui/app.rs:70`; `src/tui/entry.rs:28,37,40`
**Change:**
1. `app.rs:70` → `tracing::error!(error = %e, "failed to save config on quit");`
2. `entry.rs:28,40` — these are clap help trailing newlines before exit. Move the `println!` out of `src/tui/` into a helper in `src/main.rs` or `src/cli.rs`, OR call them before tracing init so they go directly to stdout intentionally (less ideal). Recommended: relocate to `src/cli.rs::print_help_with_newline()`.
3. `entry.rs:37` — same; relocate or rewrite as `tracing::warn!` emitted *before* fmt-writer swap.
**Source:** audit §1 P21 HIGH/MED ×4
**Verify:** `rg "println!|eprintln!|print!|eprint!" src/tui/` returns empty (or only pre-TUI paths outside the tracing-swap window, with a comment justifying each).

### A10. Update AGENTS.md + add russh ADR
**Files:** `AGENTS.md`; new `docs/adr/0002-russh-migration.md`
**Change:**
1. AGENTS.md §"SSH Transport" — rewrite to reflect russh as transport; note `ssh-keyscan`/`ssh-keygen`/`ssh-copy-id` remain subprocesses in `init`.
2. AGENTS.md §"Error Handling" — drop the thiserror mandate OR add `thiserror` to `Cargo.toml` and adopt (decide: recommend adopting per audit §2.1).
3. AGENTS.md §"Sync Strategy" — change "BLAKE3" to "SHA-256" (or complete the BLAKE3 migration — separate task).
4. AGENTS.md §"Build, Test, and Quality Commands" — remove `cargo build --bin sshi-tui`; there is one `[[bin]]` named `sshi`.
5. AGENTS.md §"Module Structure" — add `host/session_pool.rs`, `host/auth.rs`, `host/sftp.rs`, `host/concurrency.rs`, `host/pool.rs`, `commands/sync/`, `commands/report.rs`.
6. `host/pool.rs:1` doc-comment — fix inverted description.
7. Create `docs/adr/0002-russh-migration.md` (see task A11).
**Source:** audit §2.9 HIGH
**Verify:** `cargo build` still passes; AGENTS.md cross-references match actual files.

### A11. Write `docs/adr/0002-russh-migration.md`
**File:** new `docs/adr/0002-russh-migration.md`
**Content:** Standard ADR format — Context, Decision, Status (Accepted), Consequences. Reference the existing implementation plan at `docs/superpowers/plans/2026-04-27-russh-migration.md` and the evaluation at `docs/russh-migration-evaluation.md`. Note: `ssh-keyscan`/`ssh-keygen`/`ssh-copy-id` remain subprocesses (out of russh scope). Consequences: enables Windows multiplexing, consistent ProxyJump, SFTP-based transfers; loses `~/.ssh/config` automatic compatibility (now parsed via `ssh2-config`).
**Source:** audit §2.9 HIGH
**Verify:** File exists; referenced from AGENTS.md.

---

## Phase B — DB layer hardening (1–2 days)

Larger but cohesive: blocks Phase F TUI fixes.

### B1. Wrap all `rusqlite` calls in `spawn_blocking`
**Files:** `src/state/db.rs`; every call site in `commands/{check,sync,run,exec,cp,checkout,log}.rs`
**Change:**
1. Introduce `state::DbHandle` newtype wrapping `Arc<Mutex<Connection>>` (or own channel-based command pattern). Each method: `tokio::task::spawn_blocking(move || { let conn = self.conn.lock().unwrap(); ... })`.
2. Replace `ctx.db.execute(...)` → `ctx.db.execute(...).await` at all 25+ sites.
3. Wrap multi-statement batches in `conn.transaction()` via the handle.
**Source:** audit §3.1 HIGH
**Verify:** `cargo test` passes. Manual TUI test: run a 50-host `check`; observe no UI freeze when each result is written.

### B2. Wrap drain loops in `conn.transaction()`
**Files:** `src/commands/check.rs:127`; `src/commands/sync/mod.rs:515`
**Change:** Move the per-result DB writes inside one transaction.
**Source:** audit §3.5 MED
**Verify:** `cargo test` passes; benchmark `sshi check --group big` shows ~5–20× faster DB phase.

### B3. Switch to `prepare_cached` for hot queries
**Files:** `src/commands/checkout.rs:161,173,255,267`; `src/commands/log.rs:65`
**Change:** `db.prepare(...)` → `db.prepare_cached(...)`.
**Source:** audit §3.5 MED
**Verify:** `cargo test` passes.

---

## Phase C — SSH efficiency (1–2 days)

### C1. Cache `SftpSession` per host
**Files:** `src/host/session_pool.rs`; `src/host/sftp.rs`
**Change:**
1. Add `sftp: HashMap<String, Arc<SftpSession>>` to `RusshSessionPool` behind `tokio::sync::Mutex`.
2. `open_sftp(host)` checks cache first; if miss, open + cache.
3. Refactor `upload`/`download`/`probe` to take the cached `SftpSession`.
4. Invalidate on session disconnect.
**Source:** audit §3.4 HIGH
**Verify:** Manual `sshi cp` of 100-file directory shows ~100× fewer SFTP negotiations (verify via `tracing::debug!` log count).

### C2. Set SSH keepalive
**Files:** `src/host/session_pool.rs:384,464`
**Change:** `inactivity_timeout: Some(Duration::from_secs(30))` on russh `Config` (or whichever field actually sends keepalive packets per russh 0.44 API — verify).
**Source:** audit §3.1 MED
**Verify:** Manual long-running sync against aggressive-`ClientAliveInterval` host stays connected.

### C3. Wire the auth bridge (currently broken in TUI)
**Files:** `src/host/auth.rs:77,91`; `src/tui/app.rs:1173,1275,1366,1551`; new `SshAuthSender` plumbing through `connect_one` → `authenticate` → `try_pubkey`.
**Change:**
1. Add `Option<SshAuthSender>` param to `authenticate`.
2. If Some, emit `SshAuthRequest` and await oneshot reply (TUI popup path).
3. If None, fall back to `rpassword` (CLI path).
4. Plumb through `connect_one` → `setup` → `setup_with_options`.
**Source:** audit §3.1 HIGH
**Verify:** Manual TUI test: passphrase-protected key triggers popup (not block).

---

## Phase D — Command-core extraction (2–3 days)

Unblocks TUI parity for `init` and `checkout`.

### D1. Extract `init_core`
**Files:** `src/commands/init.rs` (split), new `src/commands/init/core.rs` + `init/report.rs` + `init.rs` (CLI wrapper)
**Change:**
1. Define `InitReport` typed struct (similar to `CheckReport`/`RunReport`).
2. Extract `init_core(ctx, opts, progress: &dyn ProgressSink) -> Result<InitReport>` from current `run()`.
3. Move all `printer::print_host_line` and `println!` calls into the thin CLI wrapper `run()` that calls `init_core` and prints from the report.
4. Split `run` (~400 lines) into `detect_shells`, `offer_keyscan_retry`, `offer_ssh_copy_id_retry`, `persist_init_result`.
**Source:** audit §2.2 HIGH, §2.5 HIGH
**Verify:** CLI `sshi init` output unchanged (regression test). `rg "printer::|println!" src/commands/init/` returns hits only in the wrapper module.

### D2. Extract `checkout_core`
**Files:** `src/commands/checkout.rs` (split), new `src/commands/checkout/core.rs` + `checkout.rs` wrapper
**Change:** Same pattern as D1. `checkout_core` returns a `CheckoutReport`. All `println!`/`print_table_report` move to the wrapper.
**Source:** audit §2.2 HIGH
**Verify:** CLI `sshi checkout` output unchanged.

### D3. Split `sync_inner` into phase helpers
**Files:** `src/commands/sync/mod.rs:80-758`
**Change:** Extract four helpers (each ~150 lines max):
- `expand_paths(ctx, config, names, adhoc) -> Vec<PathExpansion>`
- `decide_batch(ctx, expansions, infos) -> Vec<Decision>`
- `distribute_batch(ctx, decisions, pool, progress) -> DistributedReport`
- `run_recursive_entries(ctx, entries, pool, progress) -> RecursiveReport`
`sync_inner` becomes a thin orchestrator calling these in order.
**Source:** audit §2.5 HIGH, §3.7 HIGH
**Verify:** `cargo test sync` passes; manual `sshi sync` output unchanged. New integration test for the orchestrator.

---

## Phase E — TUI fixes (2–3 days)

After Phase D, the TUI can drive init/checkout.

### E1. Make Help/Info/Export popups scrollable
**Files:** `src/tui/app.rs:3992-4022` (Info), `:4060-4133` (Help), `:3527` (Export)
**Change:** Wrap body in a `Viewport` (pattern from log overlay `app.rs:3379-3402`). Add `↑↓/PgUp/PgDn/Home/End` handling. Cap popup size with content-aware lower bound.
**Source:** audit §1 P11 HIGH ×2, MED
**Verify:** Manual — shrink terminal to 24 rows; full help text reachable.

### E2. About panel
**Files:** `Cargo.toml` `[package]`; `src/tui/app.rs:3992-4022`
**Change:**
1. Add `homepage`, `repository`, `license`, `authors` to `Cargo.toml [package]`.
2. Convert `i` Info popup to switchable sections: Help / About / active-tab-info.
3. About content: description, version (from `env!("CARGO_PKG_VERSION")`), author, project URL, privacy policy, license.
**Source:** audit §1 P18 HIGH ×3
**Verify:** Manual — `i` shows About section with all checklist items.

### E3. `NO_COLOR` + ASCII glyph fallback
**Files:** `src/tui/theme.rs`; `src/tui/entry.rs`; possibly `src/output/printer.rs`
**Change:**
1. Detect `NO_COLOR` env var (per https://no-color.org) — if present and non-empty, use a no-colour `Theme`.
2. Add a `GlyphSet` enum {Unicode, Ascii} with each glyph paired (e.g. `✓` → `+`, `✗` → `x`, `⊘` → `o`, `⚠` → `!`).
3. Detect `TERM=linux` / `--ascii` flag → use Ascii set.
**Source:** audit §1 P20 HIGH ×2
**Verify:** `NO_COLOR=1 cargo run --features tui -- tui` — no ANSI colour codes emitted. `TERM=linux` shows ASCII glyphs.

### E4. Bring `InputField` up to editing contract
**Files:** `src/tui/components/input_field.rs`
**Change:**
1. Add Emacs-style keys: `Ctrl+A`/`Ctrl+E` (line start/end), `Ctrl+K`/`Ctrl+U` (kill-to-end / kill-to-start), `Ctrl+W` (delete-word-back), `Ctrl+Y` (yank last kill), `Meta+Backspace` (delete-word-back alt), `Ctrl+Left`/`Ctrl+Right` (word jumps).
2. Implement kill ring: `Vec<String>` with last N kills; `Ctrl+Y` yanks the most recent.
3. Implement undo ring: snapshot value + caret on each mutating op; `Ctrl+_` (or `Ctrl+Z` if reliable in target terminals) pops.
4. Add `unicode-segmentation` dep; cursor moves by grapheme, not codepoint.
5. (Defer: Shift+arrow selection — separate larger task.)
**Source:** audit §1 P10 HIGH ×3, MED ×3
**Verify:** New unit tests for each key binding; manual test with emoji ZWJ sequence.

### E5. Restore selection by identity in Config tab
**Files:** `src/tui/tabs/config_tab.rs:357`; depends on entry `id` field (already added per reconstruct plan AD-18)
**Change:**
1. `ConfigSelectionSnapshot` captures entry `id` (not just `sidebar_idx`).
2. `restore_selection` finds the entry by `id`; falls back to position only if `id` empty (legacy).
3. `App::do_open_editor` (`app.rs:3634-3648`) calls `capture_selection`/`restore_selection` like the save path.
**Source:** audit §1 P8 MED
**Verify:** Manual — delete host #2 of 5; cursor lands on next host (by id), not on the host that took position 2.

---

## Phase F — Dead code & duplication cleanup (1 day)

### F1. Delete dead code
**Files:** `src/host/filter.rs` (+ tests); `src/commands/sync/collect.rs:386` `parse_batch_metadata_output` (+ tests at 387+); `src/metrics/probes/mod.rs:10,20` `command_for`/`path_size_command`; `src/host/pool.rs:23,89` `PoolHostResult`/`reachable_hosts` (+ tests); stale `#[allow(dead_code)]` on `SshHostEntry` (`config/ssh_config.rs:6`).
**Change:** Delete each; remove `#[allow(dead_code)]` annotations that are no longer needed.
**Source:** audit §2.4 HIGH + MED ×4
**Verify:** `cargo build --features tui` passes with no warnings.

### F2. Consolidate triplicated helpers
**Files:** new `src/tui/components/shared.rs`; modify `target_filter.rs`, `operate_tab.rs`, `view_tab.rs`, `config_tab.rs`
**Change:**
1. Move `shell_label` to shared; import in all 3 sites.
2. Move group-collection logic to `App::available_groups` (already exists) — call from `target_filter.rs` and `config_tab.rs` instead of their local copies.
3. Move `focus_style`/`chips` to shared (parameterised by theme).
4. Collapse `ShellMode` into `ShellType` (or derive via single `From`).
5. Move `truncate` helper (CJK-aware) to shared.
**Source:** audit §1 P1 MED ×3, LOW ×2
**Verify:** `cargo test --features tui` passes.

### F3. Delete or wire `Focusable` trait
**Files:** `src/tui/focus.rs`
**Change:** Decision required — either (a) delete the trait + adapter tests, or (b) route one canonical list through it to justify existence.
**Source:** audit §1 P4 LOW
**Verify:** Decision documented in PR description.

---

## Phase G — Performance refactor (2–4 days)

### G1. `Arc<HostEntry>` end-to-end
**Files:** `src/config/schema.rs`; every call site in `commands/`
**Change:** Change `AppConfig::host: Vec<HostEntry>` → `Vec<Arc<HostEntry>>`. Propagate `Arc::clone` into every per-host task instead of deep clone.
**Source:** audit §3.3 HIGH
**Verify:** `cargo test` passes. Microbenchmark: 500-host `check` shows measurable allocation reduction.

### G2. `Arc<AppConfig>` in TUI
**Files:** `src/tui/app.rs:1169,1271,1362,1463,1540`
**Change:** `App.config: Arc<AppConfig>`; hand `Arc::clone` to `from_tui_parts`.
**Source:** audit §3.3 MED
**Verify:** Manual — large-config TUI click latency reduced.

### G3. Switch to `JoinSet::join_next()`
**Files:** 9 sites — `session_pool.rs:132,288`; `sync/collect.rs:194,247`; `distribute.rs:58,122`; `cp.rs:116`; `run.rs:86`; `exec.rs:124`; `check.rs:127`
**Change:** `Vec<JoinHandle>` + sequential `await` → `tokio::task::JoinSet`; drain via `join_next()`.
**Source:** audit §3.2 MED
**Verify:** `cargo test` passes.

### G4. Event-driven TUI loop
**Files:** `src/tui/app.rs:77,713`
**Change:** Replace `event::poll(Duration::from_millis(50))` with `crossterm::event::EventStream` + `tokio::select!` between terminal events, signal channel, and bridge channel.
**Source:** audit §3.6 MED
**Verify:** Manual — TUI idle CPU drops from ~5% to ~0.

### G5. Streaming SFTP (largest effort)
**Files:** `src/host/sftp.rs:85,129`
**Change:** Replace whole-file buffering with `AsyncRead`/`AsyncWrite` against russh-sftp streaming API. Removes `MAX_SFTP_FILE_SIZE` 64 MB cap.
**Source:** audit §3.3 MED, §3.8 HIGH
**Verify:** New test transferring >64 MB file succeeds.

---

## Phase H — Tests & docs (1–2 days)

### H1. Integration tests for `sync_inner` and `init::run`
**Files:** new `src/commands/sync/integration_tests.rs`; `src/commands/init/tests.rs`
**Change:** Introduce a `SessionPool` trait; mock impl returns canned exec/sftp results. Drive `sync_core` and `init_core` end-to-end.
**Source:** audit §2.8 HIGH ×2
**Verify:** New tests pass; coverage of `sync_inner` and `init::run` goes from 0% to >60%.

### H2. CHANGELOG + README updates
**Files:** `CHANGELOG.md`; `README.md`
**Change:** Append `Unreleased Update` entries for each shipped phase per `~/.config/opencode/AGENTS.md` "After Each Development Task".
**Source:** global AGENTS.md mandate
**Verify:** CHANGELOG entries match shipped commits.

---

## Sequencing

```
Phase A (quick wins) ──┬──> shippable as one PR per task
                      │
Phase B (DB layer) ────┼──> blocks Phase E (TUI needs responsive DB)
                      │
Phase C (SSH)  ────────┤
                      │
Phase D (cores) ───────┼──> blocks Phase E (TUI init/checkout parity)
                      │
Phase E (TUI) ─────────┤
                      │
Phase F (cleanup) ─────┤   (can run in parallel with D/E)
                      │
Phase G (perf) ────────┤   (after B + D land)
                      │
Phase H (tests+docs) ──┘   (last; references final state)
```

Pause after each phase for review. Each phase = one PR (or one batch of PRs).

## Definition of done (every task)

- [ ] Code change implemented per spec.
- [ ] `cargo build --features tui` clean.
- [ ] `cargo test --features tui` passes.
- [ ] `cargo clippy --all-targets --features tui -- -D warnings` clean.
- [ ] `cargo fmt --check` clean.
- [ ] Verification block (per task) shows expected result.
- [ ] CHANGELOG `Unreleased` entry appended.
- [ ] Commit message follows conventional style (e.g. `fix(sync): replace Vec::contains with HashSet for path dedup`).

---

## Phase A execution notes (landed 2026-07-19)

Phase A shipped as 10 commits (`583064f`..`feb3032`). 257 tests pass (+4 new
regression tests across A5/A6/A7/A8). Four trade-offs / deferrals surfaced
during execution — recorded here so later phases know what was *not* done
and why:

1. **A9 — visibility regression on TUI shutdown.** `flush_config_if_dirty`
   errors (config save on quit) now go to `tracing::error!`. Because the
   tracing fmt writer is swapped to a sink at process start
   (`src/main.rs:50-57`), these errors land in the ring buffer but are
   **no longer printed to stderr** after alt-screen teardown.
   *Fix options for a later phase:* (a) flush the ring buffer to stderr on
   app drop; (b) un-swap the fmt writer during shutdown; (c) move
   `flush_config_if_dirty` itself out of `src/tui/` into `src/main.rs`.
   Pick one during Phase B or H.

2. **A5 — incomplete PowerShell hardening scope.** Only the two sites
   cited by the audit (`collect.rs:118-159`, `collect.rs:317-383`) were
   migrated to single-quote interpolation. The same double-quote pattern
   exists at `collect.rs:468-498` (`build_dir_expand_cmd`) and is the
   same vulnerability class. Audit did not flag it; Phase A did not touch
   it. Phase F or a dedicated security pass should close it for
   consistency.

3. **A8 — kept the global semaphore.** Chose Option 1 (acquire per-host
   permit first, then global) over Option 2 (drop the global sem
   entirely). Option 2 would have broken `test_global_limit_respected`
   and `test_global_semaphore_accessor` and required rethinking the
   contract. If a later phase wants the simpler single-level limiter,
   those tests need to be rethought first.

4. **A10 — dropped the `thiserror` mandate instead of adopting.** Audit
   §2.1 HIGH specifically calls out that library modules deserve typed
   error enums. A10 was a docs-only task; adopting `thiserror` properly
   is a multi-day refactor. The mandate was removed from AGENTS.md to
   stop contradicting reality, but **no code was migrated**. Phase B
   (DB layer) or a dedicated phase should pick this up — the DB layer is
   the most natural first consumer of a typed error enum.

Other minor notes:
- A6 passes `ResolvedHostConfig::alias` (the sshi config name like
  `web-prod-1`) to the password prompt, not `ResolvedHostConfig::hostname`
  (which can be a raw IP). Matches the ssh-with-Host-alias convention.
- A10 rewrote `host/pool.rs:1` doc-comment from 1 line to 4 lines for
  accuracy (the prior comment was inverted).

---

## Phase B execution notes (landed 2026-07-19)

Phase B shipped as 3 commits (`356c26b` `9664882` `8bdbcae`). 257 tests
pass (no delta from Phase A — these are behavioural refactors covered by
existing tests). Three deviations from spec + two scope gaps recorded
here so later phases know what was *not* done:

1. **B1 — sync `with_conn` escape hatch on `DbHandle`.** Plan B1 spec
   said "each method: `spawn_blocking`". The 5 `.prepare(...)` sites in
   `log_core` / `fetch_latest_snapshots` / `fetch_latest_snapshots` /
   `fetch_combined_snapshots` could not be individually `.await`-ified
   because their helpers are sync and converting them would cascade
   through the TUI's sync event-handler chain (`App::from_context`,
   `maybe_reload_checkout`, `refresh_view` match arms at
   `app.rs:1900-1995`) — explicitly out of scope per B1 guidance. A
   sync `with_conn` method was added to `DbHandle` that just locks the
   mutex without spawning. All 18 command-handler sites use proper
   `spawn_blocking`. The audit's HIGH finding was about the
   per-op current-thread runtime; that's fully addressed. Documented in
   `state/db.rs` doc-comment. *Tighten later:* if/when TUI event
   handlers become async, remove `with_conn` and route the 5 sites
   through `spawn_blocking`.

2. **B2 — behavior change in `check_core` write timing.** Snapshot /
   last_seen rows are now buffered during the await drain loop and
   flushed inside one transaction *after* the loop completes. Before,
   each row was written as its handle resolved. Strictly better
   atomicity. Operational change: if `check_core` errors mid-loop, no
   rows persist at all (vs. before, where earlier hosts' rows survived).
   The `?` propagation was already all-or-nothing in spirit.

3. **B1 — `DbHandle` methods beyond the spec.** Added `transaction`
   (anticipated by B2), `with_conn` (deviation 1 above), and a
   `boxed_param` free function in `state::db`. All minimal, all
   documented. No scope creep into other areas.

### Scope gaps (intentionally not fixed)

- **B2 — second sync drain loop untouched.** `src/commands/sync/mod.rs:862`
  (`for decision in &decisions` inside `sync_path_across`) has the same
  per-row DB-write pattern but was not cited in B2's spec. Left
  untouched. Worth a follow-up cleanup task.
- **TUI dedicated-OS-thread workaround now redundant.** Now that
  `Context` holds `Arc<Mutex<Connection>>` (Send + Sync) instead of bare
  `Connection`, `&Context` is `Send + Sync` and the workaround at
  `app.rs:1184-1230` (and 4 similar sites) is no longer technically
  necessary. Removing it is a behaviour change (per-op work would run
  on the main multi-thread runtime instead of a dedicated OS thread).
  Out of scope for B; flagged for Phase F or G cleanup.

### thiserror question (audit §2.1 HIGH)

Phase B did **not** make thiserror adoption more urgent. The `DbHandle`
API uses `anyhow::Result<T>` throughout; no place where a typed enum
would have been cleaner. Recommend deferring thiserror adoption to the
security-critical modules (`host::auth`, `host::session_pool`) in
Phase C or later.

---

## Phase C execution notes (landed 2026-07-19)

Phase C shipped as 3 commits (`cce33f7` `e4a3ebe` `81c21df`). 263
tests pass (+6 from Phase B: C1 added 3 LazyCache tests, C2 added 1
keepalive test, C3 added 2 auth-bridge tests). Three deviations from
spec + two deferred ADR items recorded here:

1. **C2 — different field name than spec.** Plan said
   `inactivity_timeout: Some(Duration::from_secs(30))`. Verified against
   russh 0.44: `inactivity_timeout` is what *closes* idle connections;
   `keepalive_interval` is what actually sends keepalive packets.
   Implementation uses `keepalive_interval: Some(Duration::from_secs(30))`
   with `inactivity_timeout: None` (we never want to proactively close
   idle sessions). Documented inline in `session_pool.rs`. The audit
   cited the wrong field name; flagged here so a future re-audit doesn't
   re-flag it.

2. **C3 — `host::auth::authenticate` signature is a breaking change.**
   Gained required `auth_sender: Option<&SshAuthSender>` parameter. The
   CLI is the only in-tree caller and uniformly passes `None`, so CLI
   behaviour is unchanged. If/when this crate becomes a library, the
   signature change must be reflected in its public API. Per audit §3.1
   HIGH this breakage was expected.

3. **C3 — files touched beyond spec's explicit list.** The WIP's new
   `auth_sender` field on `Context` broke every `Context { ... }` struct
   literal in the tree. Fixups (each got `auth_sender: None`):
   `src/commands/list.rs`, `src/commands/log.rs`, `src/tui/entry.rs`
   (the TUI launcher). No behavioural change at any of these — they're
   test-only or non-operation contexts.

4. **C3 — clippy `too_many_arguments` allow.** `Context::from_tui_parts`
   now takes 8 args; added `#[allow(clippy::too_many_arguments)]` to
   match the established pattern (`sync/mod.rs:790`,
   `sync/report.rs:5`, `operate_tab.rs:626`). Not a refactor target.

### Deferred ADR items (security follow-up)

Two items from `docs/adr/ssh-auth-tui-popup.md` were **not** implemented
by C3 and remain open:

- **§(c) AUTH_POPUP_TIMEOUT = 120s.** The ADR specifies a `tokio::select!`
  wrapper around `receiver.await` in `prompt_credential` so an
  unresponsive user doesn't hang the operation indefinitely. C3 does a
  bare `receiver.await`. Cancellation still works via the host
  `CancellationToken` (Esc on the *progress* popup), but there's no
  hard timeout. Recommend follow-up in Phase E or a dedicated security
  pass.
- **§(d) credential lifetime.** `SecretString` zeroizes on drop
  (Phase A6) but the TUI-side input buffer (`AuthPopup::input.value:
  String`) is **not** wrapped in `SecretString`; it's `std::mem::take`'n
  on submit but not zeroized. Same follow-up slot.

### What C3 unblocked

The TUI auth bridge is now functional end-to-end. The chain closes:
TUI operation → `Context::auth_sender` → `SshPool::setup` →
`RusshSessionPool::setup` → `connect_one` → `connect_direct` /
`connect_via_proxy` → `authenticate` → `prompt_credential` →
`SshAuthRequest` over mpsc → forwarder task (`app.rs:680-684`) →
`TuiEvent::SshAuthRequired` → `AuthPopup` (`app_state.rs:81-109`) →
oneshot reply. Passphrase-protected keys now work in TUI mode (was
previously blocking `rpassword`, which dead-locks the alt-screen).

### thiserror question — still not urgent

Phase C did not make thiserror adoption more urgent. `prompt_credential`
returns `anyhow::Result<SecretString>` and the three error cases (bridge
closed, responder dropped, `rpassword` failure) read fine as ad-hoc
context. The `?` operator plus `.context()` is consistent with the rest
of `host::auth`. Recommendation stands: defer to Phase F/G and only if a
typed enum is needed for caller-side matching.

---

## Phase D execution notes (landed 2026-07-19)

Phase D shipped as 3 commits (`7866f7f` `9462f23` `967ba9a`). 270 tests
pass (+7 from Phase C: D1 added 6, D3 added 1, D2 added 0). Two
deviations from spec + three scope gaps recorded here:

1. **D1 — partial deviation from spec's helper list.** The spec called
   for splitting `init::run` into `detect_shells`,
   `offer_keyscan_retry`, `offer_ssh_copy_id_retry`, and
   `persist_init_result`. Only `detect_shells` and
   `persist_init_result` were extracted into `core.rs`. The two
   `offer_*_retry` helpers were **not** extracted because their logic
   is interleaved with per-host interactive prompts (ssh-keygen prompt
   → ssh-keygen subprocess; per-host ssh-copy-id prompt → ssh-copy-id
   subprocess). Extracting them into `init_core` would either (a)
   force double-SSH-connect (re-establishing pools inside the helper)
   or (b) reorder the byte stream (all prompts first, then all work).
   Neither preserves byte-identity. The CLI wrapper (`init/mod.rs`)
   owns the prompt→work interleaving inline; `init_core` runs only the
   post-prompt detect-and-persist phase and takes an `InitPools<'a>`
   borrowing wrapper-established pools. *Phase E will need a different
   UX for init from the TUI* — see scope gap below.

2. **D3 — `sync_path_across` left intact.** The function at
   `sync/mod.rs:791-1003` (~210 lines) has the same structure as the
   original `sync_inner` and shares mutable state with the recursive
   loop. The D3 spec listed only the 4 phase helpers (`expand_paths`,
   `decide_batch`, `distribute_batch`, `run_recursive_entries`);
   extracting `sync_path_across` would have been a behavioural change.
   Left intact; flagged for Phase F/G cleanup.

3. **D3 — `#[allow(clippy::too_many_arguments)]` on all 4 helpers.**
   Each helper takes 6–10 params (`summary`, `host_file_map`, etc.
   threaded via `&mut`). A `SyncPhaseContext<'a>` struct-parameter
   refactor would clean this up but is out of scope. Matches the
   established pattern (`sync/mod.rs:790`, `Context::from_tui_parts`).

### Scope gaps (Phase E or follow-up)

- **TUI cannot fully drive `init` yet.** `init_core` is non-interactive
  and callable, but expects `InitPools` already established by the
  caller, and the retry flows (keyscan, ssh-copy-id) live in the CLI
  wrapper because they require inherited-TTY subprocesses that cannot
  run from TUI mode. Phase E will need to design an init popup flow
  (likely: TUI runs `init_core` with a plan that declines all retries,
  surfaces the failure partition as a popup, lets the user pick
  "keyscan all" / "skip", re-runs `init_core` with the resulting plan
  and an internally-established retry pool). The current `init_core` +
  helpers support this flow, but Phase E has to drive it.
- **`checkout` from TUI is fully unblocked.** `checkout_core` is
  non-interactive and returns a typed `CheckoutReport`; Phase E can
  call it directly or keep using the existing DB path
  (`fetch_combined_snapshots`).
- **AGENTS.md §"Module Structure" needs follow-up update.** Three
  bullets need refinement:
  - `commands/init.rs` → `commands/init/{core,report,mod}.rs`
  - `commands/checkout.rs` → `commands/checkout/{core,report,mod}.rs`
  - `commands/sync/mod.rs` note should mention the 4 phase helpers
    (`expand_paths`, `decide_batch`, `distribute_batch`,
    `run_recursive_entries`) plus `sync_path_across`.

### thiserror question — still not urgent

Phase D did not make thiserror adoption more urgent. All four new
sync helpers return `anyhow::Result<()>` or `Result<Vec<SyncDecision>>`.
The `*Report` typed structs (`InitReport`, `CheckoutReport`) carry the
structured data callers need. Recommendation stands: defer to Phase F/G.

---

## Phase E execution notes (landed 2026-07-19)

Phase E shipped as 5 commits (`efdd337` `2857b88` E1/E2 + `9183470`
E3 + `dcfb74f` E4 + `b385310` E5). 306 tests pass (+36 from Phase D's
270: E1 +7, E2 +2, E3 +9, E4 +16, E5 +2). Three deviations from spec
+ two scope gaps recorded here:

1. **E3 — `GlyphSet::for_status` helper beyond spec.** Spec listed
   the 4 status-glyph pairs (`✓ ✗ ⊘ ⚠`) and explicit routing sites.
   Implementation added a small `pub fn for_status(&self, HostStatus)
   -> &'static str` method on `GlyphSet` to DRY the 3 HostStatus→glyph
   match sites (`app.rs::render_row`, `operate_tab::render_progress_popup`,
   `view_tab::log-row match`) and make the routing unit-testable. The
   helper returns the `⏱` literal for `HostStatus::TimedOut` since
   the spec defines no ASCII pair for it — that variant degrades only
   when `TERM=linux` is set if a future spec extends the glyph set.

2. **E3 — glyph scope decision: left non-status glyphs hardcoded.**
   The audit §1 P20 lists `▶ ◉ ○ ▼ ☑ ☐ ↳ ↵ … ⏱` as also-problematic.
   The E3 spec only defines pairs for the 4 status glyphs, so non-status
   UI glyphs (radio-button indicators, disclosure triangles, subline
   prefixes, truncation markers) remain Unicode unconditionally. Touched
   sites cover all 4 spec'd glyphs across `app.rs`, `member_picker.rs`,
   `view_tab.rs`, `operate_tab.rs`. CLI output (`src/output/printer.rs`)
   intentionally untouched — CLI keeps Unicode glyphs unconditionally.

3. **E4 — kill ring is per-field.** Spec implied a single kill ring;
   implementation puts `kill_ring: Vec<String>` on each `InputField`,
   cleared on `activate()`. Cross-field yank (kill from one field,
   navigate to another field, yank) does not work; making it work
   would require threading the ring through `App` (out of scope).
   `Ctrl+W` and `Meta+Backspace` are aliases (Emacs distinguishes
   `unix-word-rubout` from `backward-kill-word`; modern users expect
   both to delete a word back). `Ctrl+_` and `Ctrl+Z` are both wired
   to undo since crossterm's `Ctrl+_` delivery is terminal-dependent.

4. **E4 — deferred: Shift+arrow selection.** Per spec; remains audit
   §1 P10 HIGH ×1. The grapheme infrastructure added here is the
   foundation for it.

5. **E5 — `HostEntry` has no `id` field.** The spec assumed all 3
   entry types have `id` per reconstruct plan AD-18; in reality only
   `CheckEntry` and `SyncEntry` do. Per the task contract ("do not
   add it speculatively"), `HostEntry` was not modified. The
   identity-based restore therefore fires only for Check/Sync
   selections; **host deletions still fall back to positional
   clamping — the audit's literal example (host deletion) is not
   fully fixed by E5.** Codified by the
   `snapshot_falls_back_to_positional_when_entry_id_missing` test so
   a future `HostEntry.id` addition doesn't silently regress.

### Scope gaps (Phase F or follow-up)

- **E5 follow-up: add `id` to `HostEntry`.** Mirror the
  `CheckEntry`/`SyncEntry` pattern: `pub id: String` with
  `#[serde(default, skip_serializing_if = "String::is_empty")]`,
  generated via `generate_entry_id(&host.name)` at host-creation
  sites. Once added, the E5 identity-restore path will start firing
  for host deletions automatically (no code change in `config_tab.rs`
  beyond updating the `selected_entry_id` / `find_sidebar_idx_by_id`
  match arms to include `Host(i)`). This is the single highest-value
  follow-up from Phase E.
- **E3 follow-up: extend `GlyphSet` for `⏱` (and possibly `▶ ◉ ○`).**
  Add fields as audit findings resurface. Punt until a real
  `TERM=linux` user complains.
- **E4 follow-up: Shift+arrow selection.** Build on the grapheme
  infrastructure. Separate larger task.
- **E4 follow-up: cross-field kill ring.** Thread `kill_ring: &mut
  Vec<String>` through `App` if user demand materialises.

### thiserror question — still not urgent

Phase E did not make thiserror adoption more urgent. The kill ring
returns `()`, the undo ring returns `()`, the glyph lookup returns
`&'static str`. No place where a typed enum would have been cleaner.
Recommendation stands: defer to Phase F/G.

---

## Phase F execution notes (landed 2026-07-19)

Phase F shipped as 3 commits (`4903fc0` F1 + `bc4a592` F2 + `75c869f`
F3). 307 tests pass (delta from Phase E's 306: F1 −3, F2 +11, F3 −7
= net +1, but the count is 307 because the test arithmetic includes
the +1 rounding into the F2 verification window; the actual final
count is 307). Three deviations from spec + four scope gaps / future
cleanups recorded here:

1. **F1 — `parse_batch_metadata_output` deviation.** F1 spec listed
   it for deletion (audit §2.4 MED flagged the `#[allow(dead_code)]`).
   Live caller discovered at `collect.rs:229` (batch-metadata collect
   path). The function is wired in and exercised by every multi-file
   sync; the allow annotation was stale. Only the `#[allow(dead_code)]`
   was removed; the body is preserved. **Audit evidence was wrong on
   this one finding** — the function gained a caller after the audit
   snapshot. Deviation recorded in F1's commit body.

2. **F2 — `ShellMode` collapse deferred.** F2 spec called for
   "Collapse `ShellMode` into `ShellType` (or derive via single
   `From`)." `ShellMode` lives in the TUI persistence schema
   (`persist.rs:71` inside `TargetFilterState`) and serializes via
   serde as `"Sh"` / `"PowerShell"` / `"Cmd"` (default PascalCase
   enum representation). `ShellType` (`config/schema.rs:171`) has
   `#[serde(rename_all = "lowercase")]` + a `powershell` rename and
   serializes as `"sh"` / `"powershell"` / `"cmd"`. Collapsing would
   silently change the on-disk TOML format — existing
   `tui_state-*.toml` files would either fail to deserialize (strict
   mode) or silently fall back to default (the current
   `#[serde(default)]`-everywhere policy makes this the likely path,
   which would reset every user's shell filter to `Sh`). Per Phase F
   brief's "STOP and report" guidance, the collapse is **deferred to
   a dedicated behaviour-change task** that would need either a serde
   migration layer or a state-schema major-version bump. The other 4
   parts of F2 (`shell_label`, `chips`, `truncate`, `focus_style`,
   group-collection) shipped.

3. **F3 — option (a) taken (delete).** The `Focusable` trait had
   zero non-test callers. Production arrow dispatch is implemented
   per-tab. Deleted the trait + `ArrowResult` enum + 7 adapter tests
   (`ListAdapter` + 4 tests; `RadioAdapter` + 3 tests). Kept the
   other types in `focus.rs` (`Direction`, `Axis`, `AxisFreedom`,
   `FocusZone`, `EscapeOutcome`, `escape_to_parent`, `FocusPath`)
   plus their 2 surviving tests — they describe the focus model a
   future dispatch could be rebuilt on, and the audit didn't flag
   them. Module-level `#![allow(dead_code)]` stays.

### Scope gaps (Phase G or follow-up)

- **A5 scope gap (carried forward): `build_dir_expand_cmd` still
  uses PowerShell double-quote interpolation.** `collect.rs:469-498`
  has the same `"$path"` / `"$HOME\…"` pattern that A5 fixed
  elsewhere. Phase A execution notes flagged it; F1 did not touch it
  per spec. Same vulnerability class. **Phase G or a dedicated
  security pass should close it.**

- **Phase B carry-over: redundant TUI OS-thread workaround.** After
  Phase B made `Context` `Send + Sync`, the dedicated-OS-thread
  workaround at `app.rs:1184-1230` + 4 similar sites is no longer
  technically necessary. Removing it is a behaviour change (per-op
  work would run on the main multi-thread runtime instead of a
  dedicated OS thread). Phase B execution notes flagged it for F/G
  cleanup. **F1/F2/F3 did not remove it.** Flag for Phase G4
  (event-driven TUI loop) — that's the natural place to revisit it.

- **`host::pool` test coverage now zero.** F1 deleted the 3
  `PoolHostResult` tests, leaving `host/pool.rs` with no test module
  at all. Construction requires a live `RusshSessionPool` (real SSH
  connection), so unit tests aren't really meaningful — integration
  coverage comes from the rest of the test suite that *uses*
  `SshPool::setup`. **Acceptable for now**; flagged for Phase H1
  (integration tests via SessionPool trait mock).

- **`focus.rs` is now mostly dead types.** F3 deleted the trait but
  kept `Direction`/`Axis`/`AxisFreedom`/`FocusZone`/`EscapeOutcome`/
  `FocusPath`/`escape_to_parent`. These are silenced by
  `#![allow(dead_code)]`. Either (a) wire them into actual production
  dispatch (the original §8.6 plan) or (b) delete the module
  entirely. **Phase G4 (event-driven TUI loop) or a dedicated focus
  refactor.**

### Test-count delta

| Phase | Tests | Delta |
|---|---|---|
| A | 257 | +4 (from 253 baseline) |
| B | 257 | 0 |
| C | 263 | +6 |
| D | 270 | +7 |
| E | 306 | +36 |
| F1 | 303 | −3 |
| F2 | 314 | +11 |
| F3 | 307 | −7 |
| **F total** | **307** | **+1** |

### thiserror question — still not urgent

Phase F did not make thiserror adoption more urgent. The `shared`
helpers return `String` / `Style` / `Vec<String>`; the `Focusable`
deletion removed a place that returned `()`. No place where a typed
enum would have been cleaner. Recommendation stands: defer to Phase
G/H and only if a typed enum is needed for caller-side matching.

---

## Phase G execution notes (landed 2026-07-20)

Phase G shipped as 5 task commits (`4be3484` G1 + `b2f8a3b` G2 +
`a3dc59e` G3 + `7102d28` G4 + `a799f49` G5) plus one flake-fix chore
(`34977f5`). 313 tests pass (+6 from Phase F's 307). One pre-existing
flake fixed + five deviations from spec recorded here:

0. **Pre-flight flake fix.** Phase E3's env-mutating theme tests
   (`set_var`/`remove_var` on `NO_COLOR` and `TERM`) raced under
   cargo's parallel test runner. The 7 affected tests in
   `src/tui/theme.rs::tests` now take a static `ENV_TEST_LOCK:
   std::sync::Mutex<()>` returned by `clear_env()`; this serialises
   them without introducing a new dev-dep (`serial_test` was the
   alternative). Verified clean across 10 consecutive
   `cargo test --features tui` runs.

1. **G1 — `serde = { features = ["rc"] }` required.** Wrapping
   `HostEntry` in `Arc` end-to-end meant `AppConfig` (which embeds
   `Vec<Arc<HostEntry>>`) now serializes through serde's `rc`
   primitive. Without the feature flag, serde emits a compile error.
   The feature is transparent for existing TOML files (Arc round-trips
   identically to the inner value).

2. **G1 — mutation sites resolved via `Arc::make_mut`.** Two paths
   mutate `HostEntry` after construction:
   - `init/core.rs::persist_init_result` (set `.shell` from probe
     result) — rewritten as `iter().position()` + `Arc::make_mut`.
   - `config_tab.rs` inline field edit — `apply_host` now takes
     `Arc::make_mut(h)`.
   - Entry-form save path wraps the mutated `HostEntry` in `Arc::new`
     before reassigning.
   - `ListData.hosts` (viewer-only) deliberately kept as
     `Vec<HostEntry>` deep-clone — it's not on the spawn path so
     Arc-buying would not help.

3. **G2 — cascaded to `Context.config`.** Spec said `App.config:
   Arc<AppConfig>`; reality required `Context.config: Arc<AppConfig>`
   too (the TUI hands config to operations via `Context`). All
   mutation sites use `Arc::make_mut(&mut self.config)` (COW).
   Reload-from-editor wraps in `Arc::new`.

4. **G3 — JoinSet drop semantics differ (latent improvement).**
   `JoinSet` aborts remaining tasks on drop; the prior `Vec<JoinHandle>`
   left them detached. No drain loop in tree early-returns today, so
   the difference is invisible — but if a future code path adds early
   return on first error, JoinSet's auto-abort is the desired
   behaviour.

5. **G4 — two new TUI deps.** `crossterm = { features = ["event-stream"] }`
   for `EventStream`, plus `futures = "0.3"` for `StreamExt`. Both are
   TUI-only (under `[features] tui`). Banner-expiry moved from
   `event::poll(50ms)` loop to `tokio::time::sleep_until` future
   composed into the same `tokio::select!`. Idle CPU should drop to
   ~0 (can't verify manually). All 5 OS-thread workaround sites
   replaced with direct `tokio::spawn`. Compile-time test
   `context_is_send_sync` guards the Send bound that motivated the
   workaround.

6. **G5 — `MAX_SFTP_FILE_SIZE` cap genuinely deleted.** Not just
   hidden — the constant is gone. `upload` opens the local file via
   `tokio::fs::File::open` and streams via
   `tokio::io::copy(&mut local_file, &mut remote_file)`. `download`
   symmetrically streams via
   `tokio::io::copy(&mut remote_file, &mut local_file)`. Explicit
   `File::shutdown().await` added so close errors surface (russh-sftp's
   Drop is fire-and-forget). Test exercises the streaming primitive at
   64 MiB + 1 byte to prove the legacy cap is gone. Real upload/download
   can't be exercised from `cargo test` (need live SSH server) — Phase
   H1 will add mock-based integration coverage.

### Carry-overs now closed

- **Phase B redundant TUI OS-thread workaround** — fully removed by G4.
  All 5 spawn sites now use direct `tokio::spawn` on the main
  multi-thread runtime. The `context_is_send_sync` compile-time test
  guards against future regressions of the Send bound.

### Carry-overs still open

- **Phase A A5 carry-forward: `build_dir_expand_cmd`** PowerShell
  double-quote interpolation. Still untouched. Same vulnerability
  class as the sites A5 fixed. **Phase H or a dedicated security
  pass** — it's the last open audit §2.7 MED item.
- **Phase D `sync_path_across` (~210 lines)** — G3 didn't refactor it;
  it's not a drain loop. Still flagged for a future cleanup pass.
- **Phase E5 `HostEntry.id`** — G1 did not add it speculatively.
  Still the single highest-value follow-up.
- **Phase F `host::pool` zero tests** — Phase H1 scope.

### Test-count delta

| Phase | Tests | Delta |
|---|---|---|
| A | 257 | +4 (from 253 baseline) |
| B | 257 | 0 |
| C | 263 | +6 |
| D | 270 | +7 |
| E | 306 | +36 |
| F | 307 | +1 |
| **G** | **313** | **+6** (flake-fix 0, G1 +2, G2 +1, G3 +1, G4 +1, G5 +1) |

### thiserror question — still not urgent

Phase G introduced no place where a typed enum would read more cleanly
than `anyhow::Result<T>` + `.context()`. `Arc::make_mut` returns
`&mut T`, JoinSet drain returns `Option<Result<T, JoinError>>`
(handled inline), streaming primitives return `io::Result<()>`.
Recommendation stands: defer to Phase H or beyond.

---

## Phase H execution notes (landed 2026-07-20)

Phase H shipped as 2 commits (`d8f5564` H1 + `78d0d6f` H2). 344 tests
pass (+31 from Phase G's 313). One design trade-off surfaced during
H1 and was resolved per the spec's explicit "STOP and report" guidance
— recorded here so the trait design is reproducible.

1. **H1 deviation from spec step 1 — `async-trait` is a regular dep,
   not a dev-dep.** The user's H1 scope said "Add `async-trait` as a
   **dev-dependency**" with the rationale "It's test-only
   infrastructure (production code keeps the concrete
   `RusshSessionPool`)." That mental model conflicts with steps 2–5 of
   the same scope, which refactor production sites (`shell::detect_russh`,
   `init::InitPools`, the 4 sync phase helpers + `sync_path_across`) to
   take `&dyn SessionPool`. Once production code takes the trait
   object, the trait definition + `#[async_trait]` impl for
   `RusshSessionPool` must be visible in non-test builds, so the macro
   must be a regular dep. Used `[dependencies]` instead; flagged
   prominently in the commit body + final report. The alternative
   (dev-dep + cfg-gated trait) would have required either duplicating
   every refactored function behind `#[cfg(test)]` (untenable) or
   keeping production code 100% concrete (which would have left no
   seam for the mock to enter — defeating H1's purpose).

2. **H1 — Option C (hybrid trait refactor) chosen over Option A (full).**
   The spec listed 10 production sites needing trait conversion; final
   count was exactly 10 (5 in `sync/mod.rs`, 3 in `collect.rs`, 2 in
   `distribute.rs`). No sites exploded beyond budget. The
   `host::shell::detect_russh` and `init::core::InitPools` sites
   converted cleanly. `SshPool` itself was NOT refactored to hold
   `Arc<dyn SessionPool>` — doing so would require adding `shutdown` to
   the trait (RusshSessionPool::shutdown currently consumes `self`),
   which is a separate refactor. As a result, `SshPool` filter methods
   still hold concrete `Arc<RusshSessionPool>` internally; only smoke
   tests added (closes Phase F carry-over literally, but deep coverage
   of `SshPool::filter_*` with non-empty data is deferred).

3. **H1 — `sync_inner` itself kept concrete.** Per the user's
   instruction ("NOT `sync_inner` itself — keep it concrete, just have
   it call the helpers via the trait"), the orchestrator still calls
   `SshPool::setup_with_options` which does real SSH. Tests cover the
   early-return paths (no paths, single host, zero-host error) but
   cannot drive the happy path without intercepting `SshPool::setup`.
   The 4 phase helpers + `sync_path_across` are tested directly via
   the mock — that's where the substantive logic lives.

4. **H1 — `Arc::clone` is type-specific, can't auto-coerce.** The
   first attempt at `let sessions: Arc<dyn SessionPool> =
   Arc::clone(&pool.session_pool);` failed: `Arc::clone` returns
   `Arc<RusshSessionPool>` and the compiler doesn't insert unsized
   coercion through function return values. Fixed via two-step let
   binding: `let concrete: Arc<RusshSessionPool> = Arc::clone(&pool.session_pool);
   let sessions: Arc<dyn SessionPool> = concrete;` — `CoerceUnsized`
   fires on the second assignment because the destination type is
   inferred from the annotation.

5. **H1 — InitPools Option fields needed explicit `.map()`.** Rust's
   unsized coercion doesn't auto-propagate through `Option<&T>` →
   `Option<&dyn Trait>`. The bare `session: &session_pool` field
   coerced automatically; the `retry: retry_pool.as_ref()` field
   didn't. Fixed by `.map(|p| p as &dyn SessionPool)` on each Option
   field.

6. **H2 — README `cp` section had outdated `64 MB cap` mention.** The
   streaming-SFTP refactor (G5) lifted the cap, but the README still
   said "Per-file transfers use SFTP and are capped at 64 MB each;
   oversized files are reported and skipped." Fixed as part of H2
   since the user spec for H2 included README updates.

### Carry-overs now closed by Phase H

- **Phase D `sync_path_across` (~210 lines) untested** — covered by
  `sync_path_across_distributes_newest_to_older` +
  `sync_path_across_in_sync_no_io` in `commands::sync::integration_tests`.
- **Phase F `host::pool` zero tests** — 2 smoke tests added in
  `host::pool::tests` (full filter-logic coverage still deferred).

### Carry-overs still open after Phase H

- **Phase A A5 carry-forward: `build_dir_expand_cmd`** PowerShell
  double-quote interpolation. **NOT touched in Phase H** per the task
  contract. Still the last open audit §2.7 MED item. Same vulnerability
  class as the sites A5 fixed in `collect.rs::collect_file_metadata`
  and `collect.rs::build_batch_metadata_cmd`. Recommend a dedicated
  security follow-up.
- **Phase E5 `HostEntry.id`** — not added speculatively. Still the
  highest-value follow-up; once added, the E5 identity-restore path
  will start firing for host deletions automatically.
- **`sync_inner` happy path** — can't be driven past `SshPool::setup`
  without intercepting a concrete static method. Deeper coverage
  requires refactoring `sync_inner` to take a pre-built pool OR
  refactoring `SshPool::setup_with_options` to be generic — both out
  of H1 scope.
- **`SshPool::filter_*` deep coverage** — `SshPool.session_pool` is
  still concrete. Adding `shutdown` to the trait would let
  `SshPool.session_pool` become `Arc<dyn SessionPool>` and unblock
  mock injection. Trivial follow-up if anyone needs it.
- **`focus.rs` dead types** — `Direction` / `Axis` / `FocusZone` /
  `EscapeOutcome` / `FocusPath` / `escape_to_parent` still silenced by
  `#![allow(dead_code)]` after F3. Either wire them or delete the
  module.
- **B1 sync `with_conn` escape hatch** — the 5 sync helpers
  (`log_core`, `fetch_latest_snapshots`, `fetch_combined_snapshots`)
  still use the sync `with_conn` because they're called from the TUI
  main thread. Tightening requires the TUI event handlers to become
  async.
- **B2 second sync drain loop** — `sync_path_across` per-row DB writes
  still not wrapped in a transaction. Flagged by Phase B execution
  notes.
- **`metrics::collector` / `host::auth` tests** — `metrics::collector`
  still has no test for the batch metadata assembly; `host::auth` has
  no test for the `authenticate` flow (mock would need to fake russh
  `Handle<SshHandler>`, which is what H1's `MockSessionPool` worked
  around by abstracting at a higher level).

### Test-count delta

| Phase | Tests | Delta |
|---|---|---|
| A | 257 | +4 |
| B | 257 | 0 |
| C | 263 | +6 |
| D | 270 | +7 |
| E | 306 | +36 |
| F | 307 | +1 |
| G | 313 | +6 |
| **H** | **344** | **+31** (H1 +31: init 9, sync 13, mock 7, host::pool 2; H2 0) |

### thiserror question — still not urgent

Phase H introduced `SessionPool` (a trait with async methods returning
`anyhow::Result<T>`) and `MockSessionPool` (returns `anyhow::Result`
with `.bail!()` for unexpected calls). No place where a typed error
enum would read more cleanly — the mock's "no canned response" error
is a test-time assertion failure, not a recoverable production error.
**Final recommendation:** thiserror remains not urgent. Adopt only if
a future caller needs to `match` on error variants; for the
 foreseeable future, `anyhow::Result<T>` + `.context()` remains the
 right call.


