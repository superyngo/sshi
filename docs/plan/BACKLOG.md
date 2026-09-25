# Backlog
Status: In progress

The one living record of open work. Rows move to Done with the commit that closed them and are
never deleted. Evidence is file + symbol, never a line number. `Verified` is the date the row was
last checked against the tree — not when it was opened.

Seeded 2026-09-25 from the scope gaps in [2026-07-18-audit-fixes.md](2026-07-18-audit-fixes.md),
ADR 0001/0002 follow-ups, [the 2026-05-21 TUI audit](../audit/2026-05-21-tui-audit.md) deferred list,
the one `TODO` in `src/`, and code defects found by
[the 2026-09-25 documentation audit](../audit/2026-09-25-documentation-audit.md) and
[the 2026-09-25 code audit](../audit/2026-09-25-code-audit.md) (B20–B61). Items closed before
this file existed are recorded in those source documents, not here.

## Open

| ID | Opened | Verified | Pri | Finding | Evidence | Effort | Acceptance |
|---|---|---|---|---|---|---|---|
| B4 | 2026-07-18 | 2026-09-25 | P2 | `HostEntry` has no stable `id`; Config selection restore after delete is positional | `src/config/schema.rs` `HostEntry`; `src/tui/tabs/config_tab.rs` `restore_selection` | M | Deleting host 2 of 5 restores the cursor by identity |
| B6 | 2026-07-18 | 2026-09-25 | P2 | Unused focus-model types kept alive by `#![allow(dead_code)]` | `src/tui/focus.rs` | S | Types wired in or deleted; allow removed |
| B11 | 2026-07-18 | 2026-09-25 | P3 | Kill ring is per-`InputField`; yank does not cross fields | `src/tui/components/input_field.rs` `InputField` | M | Text killed in one field can be yanked in another |
| B12 | 2026-07-18 | 2026-09-25 | P3 | Windows close button (`CTRL_CLOSE_EVENT`) not handled; `TODO(post-MVP windows)` | `src/tui/app.rs` `spawn_signal_listener` | M | Terminal restored when the console window is closed |
| B15 | 2026-09-25 | 2026-09-25 | P3 | `init --update` is a no-op whenever `config.toml` exists (`effective_update = update \|\| config_exists`) | `src/commands/init/mod.rs` `run` | S | Flag has a distinct effect, or is removed |
| B17 | 2026-09-25 | 2026-09-25 | P3 | Editor precedence differs: `sshi config` tries `$EDITOR` first, TUI `E` tries `$VISUAL` first | `src/commands/config.rs` `run`; `src/tui/app.rs` `App::do_open_editor` | S | One shared resolver, `$VISUAL` then `$EDITOR` |
| B19 | 2026-09-25 | 2026-09-25 | P3 | `sync_state` rows are written with placeholder `mtime`/`size_bytes`/`blake3` (0/0/"") and never read | `src/commands/sync/mod.rs` (inserts into `sync_state`) | M | Either real values are written and used, or the table is dropped by migration |
| B30 | 2026-09-25 | 2026-09-25 | P2 | Dead SSH/SFTP sessions are never evicted or reconnected | `src/host/session_pool.rs` `LazyCache`, `RusshSessionPool` | M | After a dropped connection the next op on that host reconnects once |
| B31 | 2026-09-25 | 2026-09-25 | P2 | Windows `--sudo` never observes the elevated command's exit status; `run --sudo --dry-run` previews the sh form | `src/host/shell.rs` `sudo_wrap`; `src/commands/run.rs` `run` | M | Windows `--sudo` either reports the real exit status or is refused with an error; preview uses the host shell |
| B34 | 2026-09-25 | 2026-09-25 | P2 | Recursive sync runs one exec per host per file and uses legacy `distribute`, bypassing per-host limits | `src/commands/sync/mod.rs` `run_recursive_entries`, `sync_path_across`; `src/commands/sync/distribute.rs` `distribute` | M | Recursive entries use the batch collector and `distribute_pooled`; exec count O(hosts), not O(hosts × files) |
| B37 | 2026-09-25 | 2026-09-25 | P2 | Per-host outcome policy diverges: log-write failure aborts `exec`/`run` but warns in `cp`/`check`; `Partial` = skip/success/failure by command; `cp` log rows omit errors; `check` unreachable writes outside its transaction | `src/commands/{exec,run,cp,check}.rs`; `src/commands/report.rs` `printer_sink_with_partial` | M | One shared log-write helper and one `Partial` mapping used by all four commands |
| B39 | 2026-09-25 | 2026-09-25 | P2 | sh probes misread macOS/BSD output: load shifted, paths `MISSING`, disk ×1024, memory/battery empty | `src/metrics/probes/sh.rs` `command_for`, `batch_path_command`; `src/metrics/parser.rs` | M | Parser tests with captured macOS output for load, disk, memory, battery, path size |
| B40 | 2026-09-25 | 2026-09-25 | P2 | Windows path probes: cmd `dir` output unparsed (0, success); PowerShell empty dir `MISSING` | `src/metrics/probes/{cmd,powershell}.rs` `batch_path_command`; `src/metrics/parser.rs` `parse_path_size` | M | Parser tests with captured cmd and PowerShell output, including an empty directory |
| B42 | 2026-09-25 | 2026-09-25 | P2 | ssh_config parser: `Match` directives overwrite the prior `Host`; duplicate blocks don't merge; case-sensitive; `Include` ignored | `src/config/ssh_config.rs` `parse_ssh_config_content`, `ParsedSshConfig::query` | M | Probe config in the code audit resolves `web1` to `alice`/22; `Include` either followed or warned |
| B46 | 2026-09-25 | 2026-09-25 | P2 | Config tab editors: Esc commits in form, discards in direct popup; entry-form viewport height 0 and ignores hint rows; form/direct editors duplicated; mode state as `Option::unwrap()` | `src/tui/tabs/config_tab.rs` `handle_vec_editor_key`, `handle_direct_vec_editor_key`, `render_entry_form` | M | One vec/group editor used by both paths with one Esc rule; long forms scroll with a sticky cursor |
| B47 | 2026-09-25 | 2026-09-25 | P2 | View → List: sync rows not editable when there are no checks; layout mirrored in four functions | `src/tui/tabs/view_tab.rs` `list_entry_at_line`, `list_selectable_lines`, `list_line_count`, `render_list_result` | S | One layout model drives all four; `e` on a sync row works with zero checks |
| B48 | 2026-09-25 | 2026-09-25 | P2 | `InputField` deletes one char, not one grapheme; no horizontal scroll | `src/tui/components/input_field.rs` `InputField` | M | ZWJ emoji and combining accents delete whole; cursor visible past field width |
| B49 | 2026-09-25 | 2026-09-25 | P3 | One unknown enum value resets all persisted TUI state | `src/tui/state/persist.rs` `load` | S | Unknown values fall back per field; other fields survive |
| B50 | 2026-09-25 | 2026-09-25 | P2 | UI-thread work: SQLite in `render`, `write_report` on event thread, TOML write per arrow key, report clone + line rebuild per frame, per-frame target resolution | `src/tui/app.rs` `App::render`, `refresh_view`, `save_state`, `render_results_popup` | M | `render` performs no I/O; results lines cached on arrival; state saves debounced |
| B52 | 2026-09-25 | 2026-09-25 | P2 | Help text documents an `f` filter popup with no handler; `components/target_filter.rs` never compiled | `src/tui/app.rs` `render_help_body`, `render_tab_info_body`; `src/tui/components/target_filter.rs` | S | Help matches handled keys; orphan file removed |
| B53 | 2026-09-25 | 2026-09-25 | P3 | Operation scaffolding duplicated (five `App::execute_*`, four command cores); `*_core` returns an enum callers `unreachable!`; `App::handle_key` 1010 lines | `src/tui/app.rs`; `src/commands/{exec,run,cp,check}.rs` | L | One launch helper and one fan-out helper; typed core returns; `handle_key` split by tab/popup |
| B54 | 2026-09-25 | 2026-09-25 | P3 | Parallel implementations: TUI export vs CLI report builders (checkout `task` differs), `resolve_target_names` vs `Context::resolve_hosts`, `Summary`/`SyncSummary` printing, Operate/View target rows, `parse_ssh_config`/`load_ssh_config` | `src/tui/app.rs`; `src/output/summary.rs`; `src/tui/tabs/{operate_tab,view_tab}.rs`; `src/config/ssh_config.rs` | M | Each pair reduced to one implementation; TUI and CLI checkout exports byte-identical |
| B55 | 2026-09-25 | 2026-09-25 | P3 | Dead code and misleading comments (list in the 2026-09-25 code audit) | `src/tui/tabs/operate_schema.rs`; `src/commands/sync/collect.rs`; `src/tui/event.rs`; `src/commands/init/report.rs`; `src/tui/async_bridge.rs`; `src/host/sftp.rs`; `src/config/app.rs` | S | Items removed or comments match code |
| B58 | 2026-09-25 | 2026-09-25 | P3 | `migrate` rewrites `user_version` downward under an older binary | `src/state/db.rs` `migrate` | S | Newer schema version refused or left untouched |
| B61 | 2026-09-25 | 2026-09-25 | P3 | Enums/catalogs re-spelled: shell strings in Config tab, `ShellMode` label ×3, check catalog ×2, script-extension mapping ×2 | `src/tui/tabs/config_tab.rs` `SHELL_VARIANTS`; `src/tui/tabs/config_schema.rs` `CHECK_ENABLED_OPTIONS`; `src/config/schema.rs` `AppConfig::default`; `src/commands/exec.rs` | S | Each derived from one source |
| B65 | 2026-09-25 | 2026-09-25 | P3 | Remote SFTP overwrite is remove-then-rename (brief window with no file) because russh-sftp 2.1.1 lacks `posix-rename@openssh.com` | `src/host/sftp.rs` `upload` | S | Upgrade russh-sftp (or send the extension) and replace atomically; SIGKILL-left `.sshi-tmp` files swept |
| B69 | 2026-09-25 | 2026-09-25 | P3 | `-v/--verbose` has almost no observable CLI effect: `Context::verbose` is never read outside tests; the sync `tracing::debug!` events only fire in the TUI-only `SyncOutputStyle::Quiet` path; russh logs via the `log` crate, which the tracing subscriber does not bridge. Even `RUST_LOG=trace` shows only WARN lines on a real `sync` run | `src/main.rs` `init_tracing`; `src/commands/mod.rs` `Context::verbose`; `src/commands/sync/mod.rs` | S | Decide what `-v` should show (e.g. bridge `log` via `tracing-log`, emit per-host connect/auth debug); a `-v` run shows extra diagnostics |
| B70 | 2026-09-25 | 2026-09-25 | P3 | TUI Operate view persists a `checkout_history` toggle that nothing reads (the CLI flag was removed in B14) | `src/tui/tabs/operate_schema.rs`; `src/tui/state/persist.rs` `checkout_history` | S | Toggle removed (old state files still load), or wired to a real history view |
| B72 | 2026-09-25 | 2026-09-25 | P3 | Every non-trivial `sync` run logs `WARN session_pool has 2 strong references at shutdown; sessions may not be cleanly closed`: `sync_inner` still holds the `sessions: Arc<dyn SessionPool>` clone when it calls `SshPool::shutdown`, so SSH sessions are never disconnected gracefully; `?` early returns skip `shutdown` entirely (found while verifying B62) | `src/commands/sync/mod.rs` `sync_inner`; `src/host/pool.rs` `SshPool::shutdown` | S | Real-binary `sync` prints no shutdown warning; sessions closed on every return path |

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
| `rsa` Marvin timing side channel RUSTSEC-2023-0071 (via `russh` → `ssh-key`; accepted after B64) | Upstream: no fixed `rsa` release; `russh` pins `rsa` 0.10 pre-release | A `russh` release depends on a fixed `rsa`; re-run `cargo audit` |

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
| B38 | Recursive `cp` silently skipped symlinks and unreadable entries | `fe2d12e` — `walk_files` warns on symlinks/special files, errors on unreadable entries; `plan_transfers` checks single files; tests; real binary: symlink warned, chmod 000 file → exit 1 |
| B59 | PowerShell swap collected but never displayable | `19c79a7` — `parser::parse_ps_swap` + shared `ps_swap_totals` used by `extract_metric_value` (legacy raw snapshots too); tests |
| B68 | TUI tests resolved the real state dir via `App::new` and could run the legacy migration | `aa6cbcd` — `App::from_context_with_state_path`; `minimal_app` uses a per-process temp dir; test asserts the explicit path is used |
| B51 | `maybe_reload_checkout` cleared `db_stale` when the snapshot fetch failed | `7019a32` — `db_stale = false` only on `Ok`; test `checkout_reload_keeps_db_stale_on_fetch_error` |
| B45 | Config tab discarded edits silently: `path:{i}` check rows, unparseable numeric settings | `28aefbf` — `FieldDescriptor::readonly` for check paths; `apply_settings` returns `Result` and the editor stays open with the message; tests |
| B16 | New-config comment template documented removed `groups`/`enable_hosts`/`enable_all` fields | `e9c5728` — `inject_config_comments` rewritten to current `CheckEntry`/`CheckPath`/`SyncEntry`/`Settings` fields; test `b16_template_mentions_only_existing_schema_fields` |
| B10 | Checkout metric extractors had no unit tests | `8e6d4c8` — tests in `checkout::tests` for each metric (sh and PowerShell samples, fallbacks); no behaviour change |
| B62 | Sync source-skip and `-v` unreachable/sftp-failed lines swapped host and status in `print_host_line` | `80a187f` — arguments swapped back in `decide_batch`, `sync_path_across`, `sync_inner`; real binary: `[h1 ] ⊘ does not have …` |
| B60 | `output::printer` (and `log`/`checkout` tables) wrote ANSI colours with no TTY/`NO_COLOR` gate | `6fd0848` — `printer::should_color` (TTY + `NO_COLOR`), `format_host_line`; `log` and `checkout` gated too; real binary: piped `list`/`log`/`checkout` contain no `^[[` |
| B18 | `distribute_pooled` acquired the global permit before the per-host one | `ae57ce5` — `distribute_pooled` uses `ConcurrencyLimiter::acquire`; test `test_distribute_pooled_acquires_per_host_first`; `ssh-transport.md` note updated |
| B3 | Config tab breadcrumb indexed `config.host/check/sync[*i]` directly; stale index panicked | `613d3c7` — `.get(*i)` with `?` fallback in `ConfigTabState::breadcrumb`; test `breadcrumb_stale_index_does_not_panic` |
| B8 | `batch_keyscan_and_accept` panicked if the home directory could not be resolved | `1bb5769` — `append_keys_to_known_hosts` returns `anyhow` context errors; caller propagates; unit tests for missing home and append |
| B41 | `fetch_latest_snapshots` read the whole snapshot history | `4f7de17` — `latest_snapshot_sql` window query (newest `collected_at`, then `id`); tests: 15 history rows → 2, missing host kept |
| B71 | CI/release actions on deprecated Node 20/16 runtimes (found while closing B36) | `340b9c9` — checkout@v5, upload-artifact@v6, download-artifact@v7, action-gh-release@v3 (smallest node24 majors; inputs unchanged, actionlint clean); CI annotation checked after push |
| B64 | `cargo audit`: rsa Marvin, anyhow unsound `downcast_mut`, lru ×2 + paste (via ratatui 0.29), number_prefix (via indicatif 0.17) | `8c4d5ee` — ratatui 0.30 / crossterm 0.29 / indicatif 0.18 / anyhow 1.0.104, no code changes; `cargo audit`: only rsa left (Awaiting external); real binary: TUI tabs render identically (pyte), `check -a` progress OK |
| B9 | HTML report templating lived inside the general report module | `9841414` — `render_html_report` and helpers moved to `output::html`; 391/240 tests unchanged |
| B36 | Unknown `-n` exited 0 with a wrong hint; missing explicit `-c` became an empty config | `752b3c8` — `ensure_check_names` / `ensure_sync_names` + `load_config` (init and TUI exempt); real binary: typo/missing path exit 1, `init` still creates |
| B14 | `checkout --history` / `--since` parsed but ignored | `f5f2424` — flags removed from CLI, docs and README (option A); real binary rejects them with exit 2 |
| B13 | `-v/--verbose` rejected after the subcommand | `e53fbe9` — `global = true` on `Cli::verbose`; real binary `check -a -v` accepted |
| B43 | Config save panicked on inline `settings`, replaced a symlinked config, skipped fsync, dropped unknown per-entry keys | `45fee17` — inline table converted; symlink canonicalized; `sync_all` before persist; unknown keys merged by `id`/`name`; real binary `init` verified |
| B66 | Recursive `[[sync]]` without `source` never expanded the directory (sync failed or copied nothing) | `3148ae4` — per-host recursive expansion + `union_dir_expansions`; real binary: 500/500 files copied, split-content case converges |
| B67 | Tests opened the real per-user state DB (`db::open(None)`); parallel migrations raced on a fresh file (CI failure) | `91583f5` — `list` / `navbar_focus_tests` use `open_in_memory` + `migrate_for_test`; 40/40 parallel runs pass (were 38/40 failing) |
| B5 | Recursive sync wrote DB rows one auto-commit each | `3d44f34` — `SyncRows` + shared `flush_sync_rows` (one transaction); 500-file run: same rows, time unchanged (~9.9 s) |
| B33 | "newest" picked the source by host reply order on equal mtimes | `1f22bfe` — tie with different contents is a conflict (`newest_tie_hosts` via `skip_conflict_hosts`); option A chosen by user |
| B32 | Sync metadata collection silently dropped a host whose query failed | `59fe57a` — `failed` in `CollectResult`/`BatchCollectResult`, `record_collect_failures`; PS/cmd `Get-FileHash -ErrorAction SilentlyContinue` → `NOHASH` |
| B7 | TUI auth-bridge wait had no timeout; stale popups stayed open | `e237fc7` — `await_credential` with `AUTH_POPUP_TIMEOUT` (120 s); `PopupState::prune_stale_auth` |
| B2 | TUI auth popup kept the typed credential in plain `String`s (value, undo/kill rings), never zeroized | `99be018` — `InputField::new_secret` + `wipe`; `AuthPopup` wipes on submit/cancel/drop; unit-test verified only |
| B56 | No CI since `83aea4d`; headless build warned (unused imports in `commands::checkout`) | `f51974e` — `.github/workflows/ci.yml` (ubuntu+macos × default/headless, `-D warnings`); re-exports gated on `tui` |
| B28 | Per-host `PassphraseCache`, overlapping CLI prompts, TUI popup replaced by a second request (rejected-unencrypted-key prompts were already fixed by B20) | `f885002` — `SharedPassphraseCache` + `unlock_key` under one lock; `PopupState::push_auth`/`next_auth` queue |
| B29 | `SecretString` derived `Debug`, printing the secret | `84a2cbf` — redacting `Debug` impl; `test_secret_string_debug` |
| B27 | Auth, `open_sftp` and DNS escaped the connect timeout; DNS blocked a worker thread | `966c95c` — `resolve_addr` (`lookup_host`), `auth::net` per round-trip, `open_sftp_bounded` |
| B24 | SFTP transfers wrote in place; interrupted transfer truncated the destination; whole transfer bounded by `default_timeout`; close errors discarded | `252cbc7` — temp + rename (`temp_sibling`, `write_local_atomic`), idle timeout per step (`copy_idle`) |
| B26 | No shared remote-quoting layer (sync Cmd batch, sh path probes, `exec` chmod/rm, PowerShell `sudo_wrap`) | `8dbf80d` — `host::quote` used at every site; parity tests per `ShellType` |
| B1 | PowerShell directory expansion interpolated paths in double quotes, so `$(...)` executed | `8dbf80d` — closed by B26 |
| B20 | Auth tried neither ssh-agent nor default keys; one `IdentityFile` kept; `IdentitiesOnly` ignored | `0bc35f2` — OpenSSH order in `auth::authenticate`; `ssh_config` keeps all `IdentityFile`s, parses `IdentitiesOnly` |
| B57 | `cargo audit`: russh 0.44 advisories (RUSTSEC-2026-0153/0154), yanked spin | `27422dc` — russh 0.63 (russh-keys folded into `russh::keys`); remaining advisories moved to B64 |
| B44 | `XDG_CONFIG_HOME`/`XDG_STATE_HOME` ignored; macOS used hard-coded `~/.config` / `~/.local/state` | `684a99d` — `util::app_dir`: XDG → platform default; legacy dirs copied forward once |
| B63 | Sync report marked a failed host `online` when its `name` differed from `ssh_host` (pool keys failures by `ssh_host`) | `a2072e4` — `sync::by_host_name` re-keys failures by config name |
| B35 | Exit status was 0 when every host failed | `dae7226` — ADR 0004: `3` some hosts failed, `4` all failed |
| B25 | Sync with `conflict_strategy = skip` counted conflicting files as in sync | `2ae1102` — `skip_conflict_hosts` + skip summary entry |
| B23 | TUI Config tab stripped trailing `s`/`d`/`%` from every text field on edit (`prod` → `pro`) | `fa9ee9d` — `strip_unit` limited to U64 fields |
| B22 | `checkout --combined-view` applied the 50-snapshot lookback globally, so busy hosts starved others into "offline" | `980b31f` — per-host `ROW_NUMBER()` window |
| B21 | Stdout previews byte-sliced: `sshi log` and View → Log panicked on non-ASCII output | `d143b31` — shared `util::truncate` |
