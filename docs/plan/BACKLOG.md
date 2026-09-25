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
| B28 | 2026-09-25 | 2026-09-25 | P2 | Credential prompts: per-host `PassphraseCache`; concurrent blocking `rpassword` prompts on CLI; TUI replaces an open `AuthPopup`, failing the first host; passphrase asked for rejected unencrypted keys | `src/host/session_pool.rs` `RusshSessionPool::setup`; `src/host/auth.rs` `authenticate`, `try_pubkey`; `src/tui/app.rs` `App::handle_tui_event` | M | Three hosts sharing an encrypted key prompt once; prompts serialized on CLI and queued in TUI |
| B29 | 2026-09-25 | 2026-09-25 | P2 | `SecretString` derives `Debug`, printing the secret | `src/host/auth.rs` `SecretString` | S | `format!("{:?}")` prints a redacted placeholder; test asserts the secret is absent |
| B30 | 2026-09-25 | 2026-09-25 | P2 | Dead SSH/SFTP sessions are never evicted or reconnected | `src/host/session_pool.rs` `LazyCache`, `RusshSessionPool` | M | After a dropped connection the next op on that host reconnects once |
| B31 | 2026-09-25 | 2026-09-25 | P2 | Windows `--sudo` never observes the elevated command's exit status; `run --sudo --dry-run` previews the sh form | `src/host/shell.rs` `sudo_wrap`; `src/commands/run.rs` `run` | M | Windows `--sudo` either reports the real exit status or is refused with an error; preview uses the host shell |
| B32 | 2026-09-25 | 2026-09-25 | P2 | Batch metadata collection silently drops a host whose batch exits non-zero | `src/commands/sync/collect.rs` `batch_collect_all_metadata`, `collect_file_metadata` | S | Failed host recorded in the summary; one unreadable file does not abort the host's batch |
| B33 | 2026-09-25 | 2026-09-25 | P2 | Newest source choice is nondeterministic on equal mtimes | `src/commands/sync/decide.rs` `make_decisions` | S | Deterministic tie-break (mtime, then hash, then host) or tie reported as conflict |
| B34 | 2026-09-25 | 2026-09-25 | P2 | Recursive sync runs one exec per host per file and uses legacy `distribute`, bypassing per-host limits | `src/commands/sync/mod.rs` `run_recursive_entries`, `sync_path_across`; `src/commands/sync/distribute.rs` `distribute` | M | Recursive entries use the batch collector and `distribute_pooled`; exec count O(hosts), not O(hosts × files) |
| B36 | 2026-09-25 | 2026-09-25 | P2 | Unknown `-n` names exit 0 with a wrong hint; missing explicit `-c` path silently becomes an empty config | `src/commands/mod.rs` `select_named`; `src/config/app.rs` `load` | S | `check -a -n typo` exits 1 naming available entries; `-c missing.toml` errors except for `init` |
| B37 | 2026-09-25 | 2026-09-25 | P2 | Per-host outcome policy diverges: log-write failure aborts `exec`/`run` but warns in `cp`/`check`; `Partial` = skip/success/failure by command; `cp` log rows omit errors; `check` unreachable writes outside its transaction | `src/commands/{exec,run,cp,check}.rs`; `src/commands/report.rs` `printer_sink_with_partial` | M | One shared log-write helper and one `Partial` mapping used by all four commands |
| B38 | 2026-09-25 | 2026-09-25 | P2 | Recursive `cp` silently skips symlinks and unreadable entries | `src/commands/cp.rs` `walk_files` | S | Symlinks followed or reported; unreadable entries reported, not dropped |
| B39 | 2026-09-25 | 2026-09-25 | P2 | sh probes misread macOS/BSD output: load shifted, paths `MISSING`, disk ×1024, memory/battery empty | `src/metrics/probes/sh.rs` `command_for`, `batch_path_command`; `src/metrics/parser.rs` | M | Parser tests with captured macOS output for load, disk, memory, battery, path size |
| B40 | 2026-09-25 | 2026-09-25 | P2 | Windows path probes: cmd `dir` output unparsed (0, success); PowerShell empty dir `MISSING` | `src/metrics/probes/{cmd,powershell}.rs` `batch_path_command`; `src/metrics/parser.rs` `parse_path_size` | M | Parser tests with captured cmd and PowerShell output, including an empty directory |
| B41 | 2026-09-25 | 2026-09-25 | P2 | `fetch_latest_snapshots` reads the whole snapshot history | `src/commands/checkout/core.rs` `fetch_latest_snapshots` | S | Query returns one row per host (window function or correlated max) |
| B42 | 2026-09-25 | 2026-09-25 | P2 | ssh_config parser: `Match` directives overwrite the prior `Host`; duplicate blocks don't merge; case-sensitive; `Include` ignored | `src/config/ssh_config.rs` `parse_ssh_config_content`, `ParsedSshConfig::query` | M | Probe config in the code audit resolves `web1` to `alice`/22; `Include` either followed or warned |
| B43 | 2026-09-25 | 2026-09-25 | P2 | Config save panics on inline `settings = {…}`, replaces a symlinked config, skips fsync, drops unknown per-entry keys | `src/config/app.rs` `save`, `apply_config_to_doc`, `write_aot` | S | Inline table saved; symlink preserved (write through target); unknown per-entry keys kept or warned |
| B45 | 2026-09-25 | 2026-09-25 | P2 | Config tab discards edits silently: `path:{i}` check rows, unparseable numeric settings | `src/tui/tabs/config_schema.rs` `check_fields`, `apply_check`, `apply_settings` | S | Path rows read-only until an editor exists; bad numbers show an error and keep the editor open |
| B46 | 2026-09-25 | 2026-09-25 | P2 | Config tab editors: Esc commits in form, discards in direct popup; entry-form viewport height 0 and ignores hint rows; form/direct editors duplicated; mode state as `Option::unwrap()` | `src/tui/tabs/config_tab.rs` `handle_vec_editor_key`, `handle_direct_vec_editor_key`, `render_entry_form` | M | One vec/group editor used by both paths with one Esc rule; long forms scroll with a sticky cursor |
| B47 | 2026-09-25 | 2026-09-25 | P2 | View → List: sync rows not editable when there are no checks; layout mirrored in four functions | `src/tui/tabs/view_tab.rs` `list_entry_at_line`, `list_selectable_lines`, `list_line_count`, `render_list_result` | S | One layout model drives all four; `e` on a sync row works with zero checks |
| B48 | 2026-09-25 | 2026-09-25 | P2 | `InputField` deletes one char, not one grapheme; no horizontal scroll | `src/tui/components/input_field.rs` `InputField` | M | ZWJ emoji and combining accents delete whole; cursor visible past field width |
| B49 | 2026-09-25 | 2026-09-25 | P3 | One unknown enum value resets all persisted TUI state | `src/tui/state/persist.rs` `load` | S | Unknown values fall back per field; other fields survive |
| B50 | 2026-09-25 | 2026-09-25 | P2 | UI-thread work: SQLite in `render`, `write_report` on event thread, TOML write per arrow key, report clone + line rebuild per frame, per-frame target resolution | `src/tui/app.rs` `App::render`, `refresh_view`, `save_state`, `render_results_popup` | M | `render` performs no I/O; results lines cached on arrival; state saves debounced |
| B51 | 2026-09-25 | 2026-09-25 | P2 | `maybe_reload_checkout` clears `db_stale` when the snapshot fetch fails | `src/tui/app.rs` `App::maybe_reload_checkout` | S | Flag stays set on fetch error |
| B52 | 2026-09-25 | 2026-09-25 | P2 | Help text documents an `f` filter popup with no handler; `components/target_filter.rs` never compiled | `src/tui/app.rs` `render_help_body`, `render_tab_info_body`; `src/tui/components/target_filter.rs` | S | Help matches handled keys; orphan file removed |
| B53 | 2026-09-25 | 2026-09-25 | P3 | Operation scaffolding duplicated (five `App::execute_*`, four command cores); `*_core` returns an enum callers `unreachable!`; `App::handle_key` 1010 lines | `src/tui/app.rs`; `src/commands/{exec,run,cp,check}.rs` | L | One launch helper and one fan-out helper; typed core returns; `handle_key` split by tab/popup |
| B54 | 2026-09-25 | 2026-09-25 | P3 | Parallel implementations: TUI export vs CLI report builders (checkout `task` differs), `resolve_target_names` vs `Context::resolve_hosts`, `Summary`/`SyncSummary` printing, Operate/View target rows, `parse_ssh_config`/`load_ssh_config` | `src/tui/app.rs`; `src/output/summary.rs`; `src/tui/tabs/{operate_tab,view_tab}.rs`; `src/config/ssh_config.rs` | M | Each pair reduced to one implementation; TUI and CLI checkout exports byte-identical |
| B55 | 2026-09-25 | 2026-09-25 | P3 | Dead code and misleading comments (list in the 2026-09-25 code audit) | `src/tui/tabs/operate_schema.rs`; `src/commands/sync/collect.rs`; `src/tui/event.rs`; `src/commands/init/report.rs`; `src/tui/async_bridge.rs`; `src/host/sftp.rs`; `src/config/app.rs` | S | Items removed or comments match code |
| B56 | 2026-09-25 | 2026-09-25 | P2 | No CI since `83aea4d`; headless build warns (unused imports in `commands::checkout`) | `.github/workflows/`; `src/commands/checkout/mod.rs` | S | CI runs fmt, clippy `-D warnings` and tests for default and `--no-default-features` |
| B58 | 2026-09-25 | 2026-09-25 | P3 | `migrate` rewrites `user_version` downward under an older binary | `src/state/db.rs` `migrate` | S | Newer schema version refused or left untouched |
| B59 | 2026-09-25 | 2026-09-25 | P3 | PowerShell swap collected but never displayable | `src/commands/checkout/mod.rs` `extract_metric_value`; `src/metrics/parser.rs` | S | Swap shows for a PowerShell host |
| B60 | 2026-09-25 | 2026-09-25 | P3 | `output::printer` writes ANSI colours with no TTY/`NO_COLOR` gate | `src/output/printer.rs` `print_host_line` | S | Piped output has no escape codes; `NO_COLOR` honoured |
| B61 | 2026-09-25 | 2026-09-25 | P3 | Enums/catalogs re-spelled: shell strings in Config tab, `ShellMode` label ×3, check catalog ×2, script-extension mapping ×2 | `src/tui/tabs/config_tab.rs` `SHELL_VARIANTS`; `src/tui/tabs/config_schema.rs` `CHECK_ENABLED_OPTIONS`; `src/config/schema.rs` `AppConfig::default`; `src/commands/exec.rs` | S | Each derived from one source |
| B62 | 2026-09-25 | 2026-09-25 | P3 | Sync "source does not have path" lines call `printer::print_host_line("skip", &source, …)` with host and status swapped, so the host lands in the status slot and renders as a `·` with no name (found while fixing B25) | `src/commands/sync/mod.rs` fixed-source skip branches | S | Source-skip lines render `⊘` and name the source host |
| B64 | 2026-09-25 | 2026-09-25 | P3 | `cargo audit` after B57: rsa Marvin RUSTSEC-2023-0071 (no upstream fix; via russh/ssh-key), anyhow 1.0.102 unsound `downcast_mut` RUSTSEC-2026-0190 (not called by sshi), lru RUSTSEC-2026-0002/0253 + paste RUSTSEC-2024-0436 (via ratatui 0.29), number_prefix RUSTSEC-2025-0119 (via indicatif 0.17) | `Cargo.toml` ratatui, indicatif | M | ratatui and indicatif upgraded; rsa and anyhow recorded as accepted until upstream fixes |
| B65 | 2026-09-25 | 2026-09-25 | P3 | Remote SFTP overwrite is remove-then-rename (brief window with no file) because russh-sftp 2.1.1 lacks `posix-rename@openssh.com` | `src/host/sftp.rs` `upload` | S | Upgrade russh-sftp (or send the extension) and replace atomically; SIGKILL-left `.sshi-tmp` files swept |

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
