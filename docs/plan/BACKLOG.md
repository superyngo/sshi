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
| B53 | 2026-09-25 | 2026-09-25 | P3 | Operation scaffolding duplicated (five `App::execute_*`, four command cores); `*_core` returns an enum callers `unreachable!`; `App::handle_key` 1010 lines | `src/tui/app.rs`; `src/commands/{exec,run,cp,check}.rs` | L | One launch helper and one fan-out helper; typed core returns; `handle_key` split by tab/popup |
| B54 | 2026-09-25 | 2026-09-25 | P3 | Parallel implementations: TUI export vs CLI report builders (checkout `task` differs), `resolve_target_names` vs `Context::resolve_hosts`, `Summary`/`SyncSummary` printing, Operate/View target rows, `parse_ssh_config`/`load_ssh_config` | `src/tui/app.rs`; `src/output/summary.rs`; `src/tui/tabs/{operate_tab,view_tab}.rs`; `src/config/ssh_config.rs` | M | Each pair reduced to one implementation; TUI and CLI checkout exports byte-identical |
| B55 | 2026-09-25 | 2026-09-25 | P3 | Dead code and misleading comments (list in the 2026-09-25 code audit) | `src/tui/tabs/operate_schema.rs`; `src/commands/sync/collect.rs`; `src/tui/event.rs`; `src/commands/init/report.rs`; `src/tui/async_bridge.rs`; `src/host/sftp.rs`; `src/config/app.rs` | S | Items removed or comments match code |

## Pending verification

Landed, but the check needs a platform or environment not available locally.

| Item | Closed by | Verifies when | Fallback |
|---|---|---|---|
| Closing the Windows console window quits the TUI cleanly and restores the terminal (B12) | `67d823b` | Run `sshi` in Windows Terminal / conhost and click the window's close button; the next shell prompt is usable. The branch type-checks for `x86_64-pc-windows-msvc` (scratch crate); the full crate cannot be cross-checked here (C build scripts) | Register `SetConsoleCtrlHandler` directly via `windows-sys` |
| 30 s SSH keepalive prevents idle drops | `e4a3ebe` | Run against a server with `ClientAliveInterval 10`, `ClientAliveCountMax 0` | Lower the interval |
| WAL + `busy_timeout=5000` removes lock contention | `268cbc6` | CLI commands run while the TUI writes operation logs | Single-writer DB actor |
| One cached SFTP channel per host across many files | `cce33f7` | Trace log of a 100+ file `sshi cp` shows one channel per host | Revert to channel-per-op |

## Awaiting external

Blocked on a person or third party. **Not counted as open.**

| Item | Blocked on | Ready when |
|---|---|---|
| Drop the legacy `sync_state` table (B19 follow-up): writes stopped, the table and old placeholder rows remain in existing DBs | User approval — a `DROP TABLE` migration deletes stored rows irreversibly | User approves the drop; add migration 003 `DROP TABLE IF EXISTS sync_state` and bump `CURRENT_VERSION` |
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
| B46 | Config tab editors: Esc committed in the form, discarded in the direct popup; entry-form viewport height 0 and ignored hint rows; form/direct editors duplicated; mode state as `Option::unwrap()` | `7fd4641` — `list_editor_key` / `picker_key` + `EditorOutcome` shared by form and direct editors (duplicated handlers and `closing` flags removed); `form_list_height` / `editor_list_height` set by render, used by key handling; non-test `unwrap()`s in config_tab 32 → 0; tests `vec_editors_share_esc_discards_s_saves`, `entry_form_scrolls_with_sticky_cursor`; real TUI: form paths editor `d` then Esc keeps `[~/INTEG/only2.txt]` (old: saved `(none)`) |
| B11 | Kill ring was per-`InputField`; yank did not cross fields | `dbba5b1` — `input_field::KILL_RING` (capped at `RING_MAX`), `activate` no longer clears it; secret fields skip `push_kill`/`yank`; test `kill_ring_is_shared_across_fields_but_not_secrets` (+ `kill_ring_test_guard` serializing ring tests); real TUI: kill "def", confirm, re-activate, Ctrl+Y → "abc def" (old: "abc ") |
| B48 | `InputField` deleted one char, not one grapheme; no horizontal scroll | `efecc04` — `remove_grapheme_at_cursor`, recount in `insert_char`; `visible_parts`/`cursor_spans` (unicode-width, wide graphemes never split) used by `InputField::render`, `config_tab::input_cursor_line`, the inline value cell and `view_tab::log_input_line`; 4 tests; real TUI: 120-char Output path shows its tail `…END.json` with the cursor (old: showed the head, cursor off-screen) |
| B4 | `HostEntry` has no stable `id`; Config selection restore after delete was positional | `0b56fe6` — `EntryKey::{Host(name), Id}` in `ConfigSelectionSnapshot`; `selected_entry_key` / `find_sidebar_idx_by_key`; test `snapshot_restores_host_by_name` (replaces the test that pinned the positional gap); real TUI: external edit removing `h1` keeps the cursor on `h3` (old: moved to `h4`) |
| B50 | UI-thread work: SQLite in `render`, `write_report` on the event thread, TOML write per arrow key, report clone + line rebuild per frame, per-frame target resolution | `a7f7287` — view refresh moved to the event loop; `spawn_report_write` + `TuiEvent::Notice`; `save_state` schedules / `write_state` writes (`STATE_SAVE_DEBOUNCE`); `results_view`/`ResultsView` cached in `completed_view`; `target_count` memo; tests `render_does_not_refresh_view`, `save_state_is_debounced`, `results_view_built_on_arrival`, `target_count_memoized_until_filter_changes`; real TUI: List loads, export notice "Report written", state persisted on quit |
| B12 | Windows close button (`CTRL_CLOSE_EVENT`) not handled; `TODO(post-MVP windows)` | `67d823b` — `tokio::signal::windows::{ctrl_close, ctrl_break}` added to `spawn_signal_listener`; the branch type-checks for `x86_64-pc-windows-msvc` (scratch crate, verbatim copy); real-Windows check under Pending verification |
| B47 | View → List: sync rows not editable when there were no checks; layout mirrored in four functions | `b8c1d9c` — `view_tab::list_layout` (`ListRow`) drives `render_list_result`, `list_line_count`, `list_selectable_lines`, `list_entry_at_line`; tests `list_entry_at_line_sync_without_checks`, `e_on_list_sync_row_edits_it_without_checks` (fails on the old layout); real TUI (pyte): `e` on the sync row opens "Add/Edit Sync" (old: nothing) |
| B74 | `$VISUAL`/`$EDITOR` values with arguments failed (whole value used as the program name) | `310bf46` — `commands::config::editor_command` (Unix `sh -c` for values with whitespace) used by `config::run` and `App::do_open_editor`; test `editor_with_arguments_runs_through_sh`; real binary: `VISUAL="fake --wait"` → editor got `--wait c.toml` (old: "Failed to open editor") |
| B17 | Editor precedence differed: `sshi config` tried `$EDITOR` first, TUI `E` tried `$VISUAL` first | `0615950` — `commands::config::resolve_editor` (`editor_from`, pure, tested) used by `config::run` and `App::do_open_editor`; `cli.md`/`tui.md`/README updated; real binary: `VISUAL=fake EDITOR=false sshi config` opens fake (old binary ran `false`) |
| B6 | Unused focus-model types kept alive by `#![allow(dead_code)]` | `107cc41` — `src/tui/focus.rs` deleted (no callers; its own tests went with it), `pub mod focus` removed; 447/289 tests pass |
| B73 | sh `swap` probe was `free -b` only; macOS/BSD hosts never reported swap | `2f6ff2a` — `sh::command_for("swap")` falls back to `sysctl -n vm.swapusage`; `parse_bsd_swapusage` (K/M/G units); fixtures `macos_swap*.txt` + test; real binary: snapshot `swap` = `{"total_bytes":0,"used_bytes":0}` on the macOS rig (was `{}`) |
| B30 | Dead SSH/SFTP sessions were never evicted or reconnected | `24df437` — `RusshSessionPool::handle` checks `Handle::is_closed`, reconnects once via `connect_one` (serialized; `Reconnect` keeps config/passphrase cache/auth bridge), `LazyCache::remove` drops old SFTP/rename channels; test `test_lazy_cache_remove_forces_reopen`; real binary: rig connections killed mid-sync → "reconnected" h1/h2, 499/500 synced (old: 71/500) |
| B69 | `-v/--verbose` had almost no observable CLI effect; russh `log` records never reached the subscriber | `2f81e32` — `VERBOSE_FILTER` (`sshi=debug,russh=info,info`); `try_init` installs the `log`→tracing bridge; debug events in `connect_one`, `authenticate`, `RusshSessionPool::exec`; `Context::verbose` and its constructor params removed; real binary: `run -a -v` 5 → 13 lines (connect/auth/exec), `RUST_LOG=russh=debug` 151 lines (was WARN only) |
| B31 | Windows `--sudo` never observed the elevated command's exit status; `run --sudo --dry-run` previewed the sh form | `5e480b6` — `shell::sudo_wrap` returns `Result` (refuses PowerShell/cmd); `run_core` records a per-host error, `exec_on_host_pooled` refuses before upload; dry-runs preview per host; real binary with h1 as `powershell`: dry-run `✗ --sudo is not supported…` / `✓ would execute: sudo id -u`, run exit 3 with the refusal (old: sent `Start-Process` to the host) |
| B65 | Remote SFTP overwrite was remove-then-rename (brief window with no file); SIGKILL-left `.sshi-tmp` files never cleaned | `984d79d` — `sftp::RenameChannel` (`open_rename_channel`, `posix_rename_payload`) cached per host in `RusshSessionPool::rename_channel`; `sweep_stale_temps`/`is_stale_temp` once per (host, dir); no crate upgrade needed (3.0 lacks it too); real binary: 60 overwrites watched by a tight existence loop — 3153 "missing" samples → 0; stale temps removed, fresh and non-matching kept |
| B19 | `sync_state` rows written with placeholder `mtime`/`size_bytes`/`blake3` and never read | `c224f2f` — writes removed from `distribute_batch`/`sync_path_across`/`flush_sync_rows` (and the now-unused `label`/`group_name` params); table kept — DROP awaits user approval (Awaiting external); docs updated; real binary: sync adds 1 `operation_log` row, 0 `sync_state` rows |
| B58 | `migrate` rewrote `user_version` downward under an older binary | `a94f11c` — `migrate` bails when `user_version > CURRENT_VERSION`; `open` adds the DB path as context; test `migrate_refuses_newer_schema_and_keeps_version`; real binary: v99 DB → exit 1, version stays 99 (old binary: rewrote it to 2) |
| B72 | Every `sync` logged "session_pool has 2 strong references at shutdown"; sessions never closed gracefully; `?` exits skipped shutdown | `c0bfcce` — phases 1–4 in one async block in `sync_inner`, `drop(sessions)` then `SshPool::shutdown` before `phases?`; real binary: warning gone for recursive, fixed-source skip and dry-run syncs |
| B34 | Recursive sync ran one exec per host per file and used legacy `distribute`, bypassing per-host limits | `4920bba` — `sync_path_across` takes all expanded paths → `batch_collect_all_metadata` with `chunk_paths`/`batch_cmd_budget` and per-chunk timeout; `distribute_pooled`; legacy `distribute` and per-file `collect_file_metadata` removed; tests: exec count O(hosts), 5000-path chunking under budget for sh/PowerShell/cmd; real binary (timeout 1 s): 500 files synced both with and without `source` |
| B37 | Per-host outcome policy diverged: log-write failure aborted `exec`/`run` but warned in `cp`/`check`; `Partial` meant skip/success/failure by command; `cp` log rows omitted errors; `check` unreachable writes outside its transaction | `40efecd` — `report::{record_operation_log, record_operation_log_tx, host_status_to_printer_kind, host_status_to_log_status, update_summary, default_printer_sink}` used by all four; tests `mapping_and_summary_follows_adr_0004`, `record_operation_log_writes_and_does_not_abort_on_db_error`; real binary with `operation_log` dropped: `run -a` warns and exits 0 (was exit 1 after hosts ran) |
| B42 | `ssh_config` parser: `Match` overwrote the prior `Host`; duplicate blocks didn't merge; case-sensitive; `Include` ignored | `146dfc8` — `ParsedSshConfig::query` first-wins over matching `Block`s (`block_matches_host`, negation, case-insensitive); `Match` skipped unless `all`; `Include` followed with continuation block; tests incl. the audit probe (`web1` → `alice`/22); real binary: host defined via `Include conf.d/*` + case-different `Host` connects (old binary: DNS failure) |
| B40 | Windows path probes: cmd `dir` output unparsed (0, success); PowerShell empty dir `MISSING` | `144de58` — `---SIZE:` marker in `powershell::batch_path_command`; `parse_path_size` parses cmd `dir /s /-c` totals; cmd/PowerShell fixtures incl. empty dir and missing path (not run on a real Windows host) |
| B39 | sh probes misread macOS/BSD output: load shifted, paths `MISSING`, disk ×1024, memory/battery empty | `c4cb461` — `sh::command_for` memory fallback (`sysctl hw.memsize` + `vm_stat`), `batch_path_command` `du -sk` fallback; BSD-aware parsers; fixtures `tests/fixtures/probes/{linux,macos}_*`; real binary: `checkout -a` load/memory/disk/path sane on macOS rig |
| B52 | Help documented an `f` filter popup with no handler; `components/target_filter.rs` never compiled | `df702a9` — help/tab-info audited against `handle_key`; orphan file removed; test `help_and_tab_info_do_not_document_unhandled_f_filter_key` |
| B49 | One unknown enum value reset all persisted TUI state | `de3859f` — `persist::deserialize_enum_or_default` on the six enum fields; test `unknown_enum_value_falls_back_per_field_while_other_fields_survive` |
| B61 | Enums/catalogs re-spelled: shell strings in Config tab, `ShellMode` label ×3, check catalog ×2 (×3 with the B16 template), script-extension mapping ×2 | `5412320` — one source each; `inject_config_comments` builds the probe list from `DEFAULT_CHECK_ENABLED`; real binary: `init` template byte-identical probe list |
| B70 | TUI persisted a `checkout_history` toggle nothing read | `3f49a5e` — field removed from `OperateState`/`OpSpecific`; test `old_state_file_with_checkout_history_loads_successfully` |
| B15 | `init --update` was a no-op whenever `config.toml` existed (and had nothing to skip otherwise) | `2c78612` — flag, `InitPlan::update` and `effective_update` removed; README and `cli.md` updated; real binary: `init --update` exit 2, `init --dry-run` unchanged |
| B38 | Recursive `cp` silently skipped symlinks and unreadable entries | `fe2d12e` — `walk_files` warns on symlinks/special files, errors on unreadable entries; `plan_transfers` checks single files; tests; real binary: symlink warned, chmod 000 file → exit 1 |
| B59 | PowerShell swap collected but never displayable | `19c79a7` — `parser::parse_ps_swap` + shared `ps_swap_totals` used by `extract_metric_value` (legacy raw snapshots too); tests |
| B68 | TUI tests resolved the real state dir via `App::new` and could run the legacy migration | `aa6cbcd` — `App::from_context_with_state_path`; `minimal_app` uses a per-process temp dir; test asserts the explicit path is used |
| B51 | `maybe_reload_checkout` cleared `db_stale` when the snapshot fetch failed | `7019a32` — `db_stale = false` only on `Ok`; test `checkout_reload_keeps_db_stale_on_fetch_error` |
| B45 | Config tab discarded edits silently: `path:{i}` check rows, unparseable numeric settings | `28aefbf` — `FieldDescriptor::readonly` for check paths; `apply_settings` returns `Result` and the editor stays open with the message; tests |
| B16 | New-config comment template documented removed `groups`/`enable_hosts`/`enable_all` fields | `e9c5728` — `inject_config_comments` rewritten to current `CheckEntry`/`CheckPath`/`SyncEntry`/`Settings` fields; test `b16_template_mentions_only_existing_schema_fields` |
| B10 | Checkout metric extractors had no unit tests | `8e6d4c8` — tests in `checkout::tests` for each metric (sh and PowerShell samples, fallbacks); PowerShell battery/system-info extraction fixed on the way |
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
