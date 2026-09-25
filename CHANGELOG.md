# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]
### 2026-09-25
- docs: `AGENTS.md` now says `*_core` functions return typed reports convertible into `CommandReport` (B53), not a `CommandReport` variant.
- fix: `cp` to several hosts that share one filesystem (e.g. an NFS home) failed on all but one host, because every upload of a path used the same temp name `.<name>.sshi-tmp.<pid>`; temp names now carry a per-upload counter (`.<name>.sshi-tmp.<pid>-<n>`), and the stale-temp sweep still recognises the old form (B77).
- ci: the release workflow no longer splices the manual `version` input into its shell script (passed via `env`), checksums files with `sha256sum --`, and writes step outputs through one quoted `"$GITHUB_OUTPUT"` redirect; actionlint/shellcheck report nothing (B76).
- docs: `tui.md` now describes per-field fallback for unknown saved enum values (B49) and the read-only check-path rows / numeric-setting validation (B45).
- fix: `~/.ssh/config` options written before the first `Host` line (OpenSSH applies them to every host, e.g. a global `Port` or `IdentityFile`) were silently dropped, as were options at the top of a file `Include`d inside a `Host` block; both now apply as in OpenSSH. `config-schema.md` now documents the OpenSSH-style rules from B42 (first value wins across matching blocks, `Match`, `Include`), which it still described the old way (B75).
- docs: removed stale references (per-file sync collector, "currently inserted" `sync_state` columns) and recorded the verified "one SFTP channel per host" pending item as done.
- refactor: operation scaffolding is no longer copied per command: the `exec`/`run`/`cp`/`check` cores return their typed reports (the CLI wrappers lose their `unreachable!` matches) and share one per-host fan-out (`commands::fanout::FanOut`); the five TUI `execute_*` methods share one launch helper (`App::launch_operation`), which also fixes sync's differently worded "already running" message; and the 1,125-line `App::handle_key` is split into a short router plus one handler per popup layer, the tab bar, global keys and each tab (largest now 214 lines). No behaviour change otherwise (B53).
- refactor: five pairs of parallel implementations are now one each: the TUI checkout export and `checkout --out` share `checkout_operation_report` (the TUI stamped local time where the CLI used UTC; both now write the same document), target resolution (`commands::select_hosts` behind `Context::resolve_hosts` and the TUI), the summary "Errors:/Skipped:" block (`detail_lines` for `Summary` and `SyncSummary`), the Operate/View target rows (`shared::target_mode_line`/`target_members_line`/`target_skip_line`), and the `~/.ssh/config` reader (`parse_ssh_config` → `load_ssh_config`) (B54).
- refactor: removed dead code and fixed comments that contradicted the code: the unused `operate_schema` module and `event::drain_non_resize`; sync's always-`None` per-host scoping (`host_applicable_paths`, `scope_collect_result`, `dirs_missing`); `InitPlan`'s write-only answer fields whose docs linked to functions that don't exist; `async_bridge`'s bounded-channel/OS-thread docs and its unused bounded sender; a stranded `apply_config_to_doc` doc comment; the duplicated stale-host/skip-list logic in `init` (now `stale_hosts`/`skip_list`); `ActionFilter` strings spelled in four places (now `ActionFilter::as_str`). One behaviour fix: remote `mkdir -p` no longer swallows every error — a path blocked by a file or a permission problem now fails with "SFTP mkdir failed for <dir>" (B55).
- fix: Config tab list editors disagreed — in the Add/Edit entry form `Esc` *saved* the edited list (e.g. a deleted path), while the same editor opened directly from the field table discarded it; both now run one key map where `Esc` discards and `s` saves (Enter too in pickers). The entry form also scrolls properly when it is taller than its popup (the cursor used to pin to the top because the list was sized with height 0 and without its hint rows) (B46).
- fix: each TUI text field had its own kill ring, cleared whenever the field was activated, so text killed with Ctrl+K/U/W could only be yanked back in the same field during the same edit; one process-wide ring now serves every field (Ctrl+Y pastes the last kill anywhere). Password/passphrase fields neither write to nor read from it (B11).
- fix: TUI text fields deleted one code point instead of one grapheme (Backspace on a ZWJ emoji or an accented letter left fragments), typing a combining mark pushed the cursor past the end, and a value wider than the field hid the cursor; deletion now removes whole grapheme clusters, the cursor is recounted after insertion, and every active field (Operate, Config inline/popup editors, View log filters) scrolls horizontally by display width so the cursor stays visible (B48).
- fix: after a config reload (save, external editor), the Config tab put the cursor back on the same *position* for hosts, since `[[host]]` has no `id` — removing another host moved the cursor to a different one; hosts are now matched by `name` (checks/syncs by `id`, as before), falling back to the position only when the entry itself is gone. No config schema change (B4).
- fix: the TUI did work on every frame or keypress that belongs elsewhere: `render` loaded View data from SQLite, report files were written on the event thread, every arrow key rewrote the state file, the results popup cloned the whole report and rebuilt its lines each frame, and target names were re-resolved each frame. Now data loads before drawing, report writes run on a blocking task (result shown as a footer notice), state saves are debounced (500 ms idle, and on quit), the results popup is built once on arrival, and the target count is memoized per config/filter (B50).
- fix: on Windows, closing the console window (CTRL_CLOSE_EVENT) or pressing Ctrl+Break killed the TUI without restoring the terminal; `spawn_signal_listener` now listens for both next to Ctrl+C and quits through the normal clean-exit path within the ~5 s Windows allows (B12). Verified by type-checking for Windows; a run on real Windows is listed under Pending verification.
- fix: in View → List, `e` on a sync row did nothing when the config had no `[[check]]` entries (the edit lookup stopped at the checks section's `(none)` line); rendering, line count, cursor stops and `e`-to-edit now come from one layout model (`list_layout`) instead of four hand-mirrored functions (B47).
- fix: `$VISUAL`/`$EDITOR` values with arguments (`code --wait`, `emacsclient -t`) failed with "Failed to open editor" because the whole value was used as the program name; on Unix such values now run through `sh -c '<editor> "$1"'` like git, for both `sshi config` and the TUI `E` key (B74).
- fix: `sshi config` tried `$EDITOR` before `$VISUAL` while the TUI `E` key tried `$VISUAL` first, so the two could open different editors; both now use `commands::config::resolve_editor` — `$VISUAL`, then `$EDITOR` (empty values skipped), then `vi`/`notepad` (B17).
- refactor: removed `tui::focus` (`Direction`, `Axis`, `AxisFreedom`, `FocusZone`, `EscapeOutcome`, `escape_to_parent`, `FocusPath`): nothing outside the module used it and it compiled only behind `#![allow(dead_code)]`; no behaviour change (B6).
- fix: the sh swap probe only ran `free -b`, so macOS/BSD hosts never reported swap (stored `{}`, shown `-`); it now falls back to `sysctl -n vm.swapusage` and parses its `total = …M used = …M` line into total/used bytes (B73).
- fix: when a host's SSH connection dropped during a run, every later operation on it failed (the dead session and its SFTP channel stayed cached); the pool now notices the closed connection, reconnects that host once, reopens its SFTP channels and carries on — a sync of 500 files with all connections killed after ~100 uploads now finishes 499/500 instead of 71/500. A host whose reconnect fails keeps failing fast (B30).
- fix: `-v` changed almost nothing on the command line (the debug events it enabled were never emitted outside the TUI, and russh's `log` records were not bridged); `-v` now prints, per host, the resolved `user@host:port`/ProxyJump, the authentication method that succeeded and each remote command with its exit status and duration, and `RUST_LOG` can reach russh (e.g. `RUST_LOG=russh=debug`). The unused `Context::verbose` field is gone (B69).
- fix: `run`/`exec --sudo` on PowerShell or cmd hosts sent `Start-Process -Verb RunAs` / `runas`, which cannot report the elevated command's exit status or output, so a failure could show as success; such hosts are now refused with a per-host error (counted as failed, nothing uploaded for `exec`). `run --sudo --dry-run` previewed the sh form for every host; it now shows each host's wrapped command or the refusal (B31).
- fix: replacing an existing remote file removed it before renaming the upload into place, so readers could briefly see it missing; uploads now rename with `posix-rename@openssh.com` over a second raw SFTP channel when the server offers it (russh-sftp, even 3.0, cannot send extended requests), which replaces atomically. The first upload into a directory also removes `.sshi-tmp.<pid>` files abandoned by a killed sshi (other pid, over an hour old) (B65).
- fix: sync wrote a `sync_state` row per synced file and target host with placeholder `mtime`/`size_bytes`/`blake3` (0/0/"") that nothing ever read; those writes are removed (sync events stay in `operation_log`). The table and existing rows are left in place — dropping them is an irreversible migration held for your approval (Backlog → Awaiting external) (B19).
- fix: an older sshi opening a state database migrated by a newer one silently rewrote `user_version` downward and ran against the unknown schema; it now refuses with "schema vN is newer than this sshi supports", naming the file, and leaves it untouched (B58).
- fix: every `sync` run logged `WARN session_pool has 2 strong references at shutdown` and never closed its SSH sessions cleanly, because `sync_inner` still held a pool reference when shutting down (and a failing phase returned before shutdown at all); the phases now run in one block, the reference is dropped, and the pool is always shut down before an error propagates (B72).
- fix: recursive `[[sync]]` entries ran one metadata exec per host per file (and used the legacy `distribute`, bypassing per-host concurrency limits); they now use the batch collector — one exec per host per chunk, chunks kept under a per-shell command-size budget so Linux (128 KiB per argument) and Windows (`cmd.exe` 8 191 chars) hosts accept them, each chunk allowed `timeout` seconds per file — and `distribute_pooled`. A 500-file tree: 11.1 s → 9.6 s with a fixed source, rerun in sync 2.5 s (B34).
- fix: per-host outcome handling now matches across `exec`, `run`, `cp` and `check`: an `operation_log` write failure warns instead of aborting `exec`/`run` after every host already ran; `partial` prints ✓ and counts as success everywhere (was `skip` in `check`/`cp` output and a failure in `cp`'s summary), per ADR 0004; `cp` log rows now carry the per-file errors and say "N of M file(s) copied"; `check` writes unreachable-host rows inside its single transaction (B37).
- fix: the `~/.ssh/config` reader diverged from OpenSSH: `Match` directives overwrote the preceding `Host`, duplicate `Host` blocks did not merge, keywords and host patterns were case-sensitive, `!negated` patterns were ignored and `Include` was skipped; it now applies first-obtained-value-wins across all matching blocks in file order, matches case-insensitively, honours negation, skips `Match` blocks other than `Match all` (with a warning) and follows `Include` (globs, relative to `~/.ssh`, depth-limited, warning on unreadable files) (B42).
- fix: Windows path probes: cmd `dir` output was never parsed (size 0, reported success) and PowerShell reported an empty directory as `MISSING`; PowerShell now emits a locale-independent `---SIZE:<bytes>` marker (0 for an empty directory), cmd uses `dir /s /-c` and a missing path yields `MISSING` (B40).
- fix: sh probes misread macOS/BSD hosts — load average shifted by one field, disk sizes ×1024, memory and battery empty, path sizes reported `MISSING` (no `du -b`); probes now fall back to `sysctl`/`vm_stat`/`du -sk` and the parsers recognise both GNU and BSD output (captured fixtures for Linux and macOS) (B39).
- fix: TUI help and tab-info text documented an `f` filter popup that has no key handler; help now lists only handled keys (inline target rows, member picker), and the never-compiled `components/target_filter.rs` is removed (B52).
- fix: one unrecognised enum value in the saved TUI state (e.g. from a newer version) reset every saved setting; each enum field now falls back to its default on its own and the rest of the state survives (B49).
- refactor: shell variants, the `ShellMode` labels, the check-probe catalog and the script-extension → shell mapping are each defined once (`ShellType::VARIANTS`/`as_str`, `ShellMode::as_str`/`cycle`, `DEFAULT_CHECK_ENABLED`, `exec::script_extension_to_shell`); the new-config template now lists probes from the same catalog; no behaviour change (B61).
- fix: removed the TUI's persisted `checkout_history` toggle, which nothing read after B14; state files that still contain it load normally (B70).
- fix: removed `init --update`: it had no effect in any case (with a config present every host was already re-detected; without one there were no existing hosts to skip), so `init` behaves exactly as before and `--update` is now rejected (B15).
- fix: recursive `cp` silently skipped symlinks and unreadable files; symlinks (not followed, to avoid loops) and special files are now reported as warnings, and an unreadable file or directory entry stops the command with an error naming it (B38).
- fix: PowerShell hosts collected swap usage but `checkout` could never display it (the raw `Win32_PageFileUsage` JSON was stored unparsed); it is now parsed into total/used bytes and shown as a percentage, including for snapshots stored by older versions (B59).
- fix: TUI unit tests built `App` through the real state-path resolver, which could run the legacy state migration against the user's own `~/.local/state/sshi`; tests now pass an explicit temp state path (`App::from_context_with_state_path`) (B68).
- fix: the TUI cleared its "checkout data is stale" flag even when reloading snapshots failed, so it never retried; the flag now stays set on error (B51).
- fix: the Config tab silently discarded edits to `[[check.path]]` rows (no editor existed) and unparseable numeric settings; path rows are now shown read-only, and a bad number shows an error and keeps the editor open (B45).
- fix: the comment template written into a new `config.toml` documented removed `[[check]]`/`[[sync]]` fields (`groups`, `hosts`, `enable_hosts`, `enable_all`); it now lists only fields the schema accepts, and a test fails if the template names an unknown field (B16).
- fix: unit tests for `extract_metric_value` now cover every metric for sh and PowerShell sample outputs plus fallbacks; they exposed that PowerShell battery (`EstimatedChargeRemaining` JSON) and system-info (`OsName`) values were shown raw, which is fixed (B10).
- fix: sync's "source does not have path" lines and the `-v` unreachable/sftp-failed lines passed host and status to `print_host_line` in swapped order, printing the status word as the host name with a `·` glyph; they now name the host and show `⊘`/`✗` (B62).
- fix: host lines, `sshi log` status glyphs and `checkout` online/threshold colours wrote ANSI escape codes even when piped; colour is now used only when stdout is a terminal and `NO_COLOR` is unset or empty (`printer::should_color`) (B60).
- fix: sync uploads (`distribute_pooled`) took the global concurrency permit before the per-host one, the opposite of every other command, so a saturated host could hold global slots while idle hosts waited; they now use `ConcurrencyLimiter::acquire` (per-host first) (B18).
- fix: the Config tab breadcrumb indexed hosts/checks/syncs directly and could panic on a stale selection index; it now renders `?` instead (B3).
- fix: `init` panicked when the home directory could not be resolved while writing scanned host keys; `batch_keyscan_and_accept` now returns an error naming the problem (`append_keys_to_known_hosts`) (B8).
- fix: `checkout` fetched every stored snapshot of every selected host and kept the newest in Rust; `fetch_latest_snapshots` now asks SQLite for one row per host (`ROW_NUMBER() OVER (PARTITION BY host …)`), so the read no longer grows with history (B41).
- ci: bumped `actions/checkout` v4 → v5 (CI and release), `actions/upload-artifact` v4 → v6, `actions/download-artifact` v4 → v7 and `softprops/action-gh-release` v1 → v3 — the smallest majors that run on Node 24; CI runs no longer carry the Node 20 deprecation annotation (B71).
- chore: upgraded ratatui 0.29 → 0.30, crossterm 0.28 → 0.29, indicatif 0.17 → 0.18 and anyhow 1.0.102 → 1.0.104; `cargo audit` now reports only the `rsa` Marvin advisory (no upstream fix, tracked under Awaiting external), down from six (lru ×2, paste, number_prefix, anyhow, rsa) (B64).
- refactor: HTML report rendering (`render_html_report`, `render_output_html`, `html_escape`) moved from `output::report` into its own `output::html` module; output unchanged (B9).
- docs: backlog rows for B66, B43, B13, B14 and B36 had been inserted into the Open table instead of Done; moved to Done.
- fix: `check`/`sync -n <name>` with a name that matches no entry now exits 1 listing the available names (was exit 0 with a misleading hint); an explicit `-c <path>` that does not exist is now an error, except for `init` and the TUI (was a silent empty config) (B36).
- fix: removed the unimplemented `checkout --history` / `--since` flags (they were accepted but ignored, and `--since` never validated its value); `checkout --out` reports no longer carry them in `task` metadata (B14).
- fix: `-v/--verbose` is now a global flag, accepted after the subcommand (e.g. `sshi check -a -v`) as well as before it (B13).
- fix: config save accepts an inline `settings = {…}` table (was a panic), writes through a symlinked config instead of replacing the link, fsyncs before the rename, and keeps unknown keys inside `[[host]]` / `[[check]]` / `[[sync]]` entries (B43).
- fix: recursive `[[sync]]` entries without `source` now expand the directory on every host and sync the union of files; previously the directory itself was treated as one file, failing the download or reporting "synced" with nothing copied (B66).
- test: `list` and TUI navbar tests use an in-memory database instead of the real per-user state DB, fixing a CI race (`duplicate column name` / `file is not a database`) when parallel tests migrated the same fresh file (B67).
- fix: recursive `sync` records its database rows in one transaction per run instead of one auto-commit per row, sharing the batch path's writer; rows of transfers that completed are still written if a later file errors. Measured wall time unchanged (per-file remote round-trips dominate, B34) (B5).
- fix: with `conflict_strategy = "newest"`, hosts that share the newest mtime but hold different contents are now reported as a conflict and left untouched, instead of one being picked by reply order and silently overwriting the other (B33).
- fix: `sync` no longer silently drops a host whose metadata query fails — it is reported as failed (exit 3) with the reason, instead of the run claiming success while that host was never checked or updated; one unreadable file no longer fails a PowerShell/cmd host's whole batch (B32).
- fix: a TUI password/passphrase popup left unanswered now fails that host after 120 s instead of stalling the operation (and every host waiting to prompt) forever; popups whose operation stopped waiting close on their own (B7).
- fix(security): the TUI password/passphrase popup keeps no undo or kill-ring copies of the typed text and zeroizes its buffer on Enter, Esc and close. Verified by unit tests; process memory was not inspected on the real binary (B2).
- ci: restore CI — fmt, clippy (`-D warnings`) and tests for the default and `--no-default-features` builds on Linux and macOS; the headless build no longer warns about TUI-only re-exports in `commands::checkout` (B56).
- fix: hosts connecting together share one passphrase cache and prompt one at a time, so a key used by several hosts is asked for once; only a passphrase that decrypts the key is remembered; terminal prompts no longer block async workers; in the TUI a second credential request is queued instead of replacing the open popup and failing the first host (B28).
- fix(security): `SecretString` no longer prints the password/passphrase when debug-formatted (e.g. in logs); it shows `SecretString(***)` (B29).
- fix: `--timeout` / `default_timeout` now also bounds DNS lookup, each authentication round-trip and SFTP channel setup, so a host that stalls during login or SFTP negotiation fails after the timeout instead of hanging `sshi`; DNS no longer blocks a runtime thread. Time spent typing at a passphrase/password prompt is not counted (B27).
- fix: SFTP uploads and downloads write to a temp file and rename it into place, so an interrupted, failed or timed-out transfer no longer truncates the existing file; remote close errors now fail the transfer; `default_timeout` bounds each transfer step (no progress) instead of the whole transfer, so large files on slow links no longer time out (B24).
- fix(security): remote paths, check labels and `exec` script names are quoted by one shared layer (`host::quote`) for every shell, so spaces, quotes and `$(...)` can no longer split arguments or run commands on the remote host; Cmd hosts run PowerShell via `-EncodedCommand`, and values cmd.exe cannot quote safely are refused. sh verified end to end; PowerShell/Cmd verified by unit parity tests only (B26, B1).
- fix: SSH login follows OpenSSH order — ssh-agent keys, every `IdentityFile` (not just the last), default `~/.ssh/id_*` keys when none is listed, passphrase prompts only for encrypted keys, and `IdentitiesOnly yes` now honoured (no password prompt) (B20).
- fix(deps): upgrade russh 0.44 → 0.63 (russh-keys now `russh::keys`), clearing RUSTSEC-2026-0153/0154; RSA keys now sign with `rsa-sha2-*`, and SSH-certificate host keys are refused explicitly (B57).
- feat!: config and state directories follow `XDG_CONFIG_HOME`/`XDG_STATE_HOME`, then the platform default (macOS now `~/Library/Application Support/sshi`); on first run the old `~/.config/sshi` and `~/.local/state/sshi` are copied forward (originals kept) along with saved TUI state (B44).
- fix: `sync` reports an unreachable host under its config name and marks it failed in `--out` reports and the exit code, even when `name` differs from `ssh_host` (previously reported as online, exit 0) (B63).
- feat!: `check`, `run`, `exec`, `cp` and `sync` exit `3` when some hosts fail and `4` when all fail (previously always `0`); decided in ADR 0004 (B35). Scripts that relied on exit `0` after host failures must now handle `3`/`4`.
- fix: `sync` with `conflict_strategy = skip` now reports conflicting files as skipped (with the hosts and reason) instead of counting them as in sync (B25).
- fix: TUI Config tab no longer strips trailing `s`, `d` or `%` from text fields when editing opens (`prod` stayed `pro` on Enter); only unit-suffixed numeric fields (`30s`, `90d`) are stripped (B23).
- fix: `checkout --combined-view` looks back 50 snapshots per host instead of 50 in total, so a host with fewer snapshots is no longer shown offline (B22).
- fix: `sshi log` and the TUI View → Log preview no longer panic on non-ASCII command output; both truncate by display width via the shared `util::truncate` (B21).
- docs: add the 2026-09-25 code audit record `docs/audit/2026-09-25-code-audit.md` (whole-crate review for bugs, optimization, simplicity, clarity and integration, with reproductions) and file its 42 actionable findings as backlog rows B20–B61.
- chore: remove stray tracked root files (`CHANGELOG` duplicate, `temp`, `.claude/scheduled_tasks.lock`); ignore `.claude/`.
- docs: archive the v0.x changelog series verbatim to `docs/reference/changelog/v0.x.md` (root keeps `[Unreleased]` + v1.x); link the archive and the backlog from `CONTEXT.md`.
- docs: add the living backlog `docs/plan/BACKLOG.md` (19 open items consolidated from frozen records plus code defects found by this audit) and the audit record `docs/audit/2026-09-25-documentation-audit.md`; index both.
- docs: correct reference docs, README, and AGENTS.md against the code — TUI is built by default (headless is `--no-default-features`); fix invented TUI fields/keys/probe catalog/operation order, CLI exit codes and flag matrix, transport/sync/state claims, and README config example; add nine glossary terms; contributor rules live only in `AGENTS.md`; `.github/copilot-instructions.md` is now a pointer.
- docs: repair dead paths left by the layout migration (ADR 0002, 2026-05-21 readme-analysis audit, v1.7.0 changelog entry, 13 `src/` doc comments citing `docs/tui_reconstruct_plan.md`); mark ADR 0003 `Implemented (2026-09-02)`; drop the false "machine-checked" claim in `docs/reference/README.md`; replace a line-number citation in `state-schema.md`; fix the `SessionPool` glossary entry format; normalize the 2026-09-02 changelog sub-heading.

### 2026-09-02
- docs: adopted the fixed `docs/{reference,adr,spec,plan,debug,audit,tmp}/` layout. Added
  root `CONTEXT.md` index and `docs/reference/` (glossary + 6 subsystem reference docs:
  `cli.md`, `config-schema.md`, `ssh-transport.md`, `sync-algorithm.md`, `state-schema.md`,
  `tui.md`) as the single current-behavior source of truth.
- docs: moved and `Status:`-lined ~40 historical spec/plan/audit documents out of
  `docs/superpowers/{specs,plans,audits}/`, `docs/plans/`, `docs/ai-reports/`, and a
  gitignored `docs/tmp/` file into `docs/spec/`, `docs/plan/`, `docs/audit/`, each with a
  status derived from this changelog's own release history.
- docs: renumbered `docs/adr/ssh-auth-tui-popup.md` to `docs/adr/0001-ssh-auth-tui-popup.md`,
  added `docs/adr/0003-docs-layout-convention.md`, and added `docs/adr/README.md`.
- docs: trimmed `AGENTS.md` and rewrote `.github/copilot-instructions.md` to conduct-only,
  pointing at `CONTEXT.md`/`docs/reference/` instead of restating (and drifting from) current
  behavior; fixed stale `ssync` naming and other factual errors in the Copilot file.

## [v1.7.0] - 2026-07-20

Audit-fixes rollout covering 24 tasks across 8 phases (A–H) addressing
`docs/audit/2026-07-18-codebase-audit.md` (116 findings: 23 HIGH, 51 MED, 42 LOW).
Per-phase execution notes with deviations + scope gaps live in
`docs/plan/2026-07-18-audit-fixes.md`. Test count: 352 (up from 1.6.1's 253).

### Security

- **fix(auth):** wrap rpassword passphrase in `SecretString` immediately on receipt and pass the resolved host name (was literal `<host>`) to the password prompt. (A6, ca2a305)
- **feat(tui):** wire the SSH auth bridge end-to-end so passphrase-protected keys and password fallback work in TUI mode (was blocking on `rpassword`, dead-locking the alt-screen). (C3, 81c21df)
- **fix(sync):** move PowerShell path interpolation in `collect.rs` to single-quoted literals with `''` escaping — config-controlled paths like `$(rm -rf $HOME)` can no longer execute as PowerShell subexpressions. (A5, 9ae5d94)

### Performance

- **chore(build):** add `[profile.release]` overrides (`lto="thin"`, `codegen-units=1`, `strip="symbols"`) for ~50% smaller release binaries. (A1, 583064f)
- **perf(sync):** replace O(F²) `Vec::contains` membership scans with `HashSet<String>` seen-sets in path-expansion loops. (A4, 2094954)
- **perf(state):** wrap every `rusqlite` call from async command handlers in `tokio::task::spawn_blocking` via a new `state::DbHandle` newtype; under the TUI's per-op runtime, DB writes no longer freeze concurrent SSH tasks. (B1, 356c26b)
- **perf(state,sync):** batch the `check_core` and `sync_inner` drain loops' per-host DB writes into one `rusqlite::Transaction` — ~5–20× DB-phase speedup for a 500-host check. (B2, 9664882)
- **perf(checkout,log):** switch the 5 hot read queries from `prepare` to `prepare_cached`. (B3, 8bdbcae)
- **perf(host):** cache `SftpSession` per host alias via a new double-checked `LazyCache`; a 100-file `cp` now makes 1 SFTP negotiation per host instead of 100. (C1, cce33f7)
- **perf(host):** set `keepalive_interval = Some(30s)` on russh `client::Config` so aggressive `ClientAliveInterval` hosts and NAT idle timers no longer silently drop the session. (C2, e4a3ebe)
- **perf(config):** wrap `HostEntry` in `Arc` end-to-end; `resolve_hosts` returns owned `Vec<Arc<HostEntry>>` — eliminates 3–4 String allocations per host per command. Required enabling serde's `rc` feature. (G1, 4be3484)
- **perf(tui):** wrap `AppConfig` in `Arc`; the 5 TUI spawn sites hand off `Arc::clone` instead of deep-cloning the full config (multiple MB per click for a 500-host config → one atomic increment). (G2, b2f8a3b)
- **perf(commands):** switch from `Vec<JoinHandle>` + sequential `await` to `tokio::task::JoinSet::join_next()` at 9 drain sites. (G3, a3dc59e)
- **perf(tui):** replace `event::poll(50ms)` busy-loop with `crossterm::event::EventStream` + `tokio::select!`; idle CPU drops from ~5% to ~0. Removed the dedicated-OS-thread workaround at 5 spawn sites now that `Context` is `Send + Sync`. (G4, 7102d28)
- **perf(host):** rewrite `sftp::upload`/`download` to stream via `tokio::io::copy` against russh-sftp's `AsyncRead`/`AsyncWrite` impls — eliminates the 64 MB `MAX_SFTP_FILE_SIZE` cap and concurrent-transfer memory blowup. (G5, a799f49)

### TUI

- **fix(tui):** persist shared `dry_run` toggle across TUI restarts independently of `sync_dry_run`. (A7, 35f91d9)
- **fix(tui):** relocate the last `eprintln!`/`println!` calls out of `src/tui/` so AGENTS.md §7.3 stdio rule holds. (A9, 9438804)
- **feat(tui):** make Help (`?`), Info (`i`), and Export popups scrollable with content-aware geometry caps; previously the ~62-line Help body was unreachable past the bottom ~16 rows on a 24-row terminal. (E1, efdd337)
- **feat(tui):** convert the `i` Info popup into a 3-section switchable panel (Tab-info → About → Keybindings); About surfaces name, version, description, author, license, homepage, repository, and a privacy statement sourced from `env!("CARGO_PKG_*)`. `Cargo.toml [package]` gained the previously-missing `homepage`/`repository`/`license`/`authors` fields. (E2, 2857b88)
- **feat(tui):** honour `NO_COLOR` (per https://no-color.org) and `TERM=linux` independently; new `GlyphSet` carries the 4 spec'd status glyph pairs (`✓`/`+`, `✗`/`x`, `⊘`/`o`, `⚠`/`!`) routed through `theme.glyphs`. CLI output keeps Unicode glyphs unconditionally. (E3, 9183470)
- **feat(tui):** bring `InputField` up to the editing contract — Emacs-style keys (`Ctrl+A/E/K/U/W/Y`), word jumps, per-field kill ring (cap 8) and undo ring, grapheme-cluster cursor movement via `unicode-segmentation`. Deferred: Shift+arrow selection. (E4, dcfb74f)
- **fix(tui):** restore Config-tab selection by entry `id` (not just sidebar position) after deletion; `App::do_open_editor` now mirrors save-config's capture/restore flow. Scope gap: `HostEntry` has no `id` field, so host deletions still fall back to positional clamping. (E5, b385310)
- **fix(tui):** open the Help popup on `?` from the navbar-focused state — the navbar dispatcher trapped all keys via its `_` arm, dropping `?` before the global handler ran.
- **fix(tui):** same navbar-dispatcher trap also dropped `i` (Info popup) and `L` (Log overlay) — both are now mirrored into the navbar dispatcher and share their toggle/cycle logic with the global handlers via `cycle_info_popup`/`toggle_log_overlay` helpers.
- **feat(tui):** add Help/About toggle to the `?` popup — `Tab` cycles between the Keybindings body and the About section (reuses E2's About content). Header is a minimal `Help / About` with the active section bolded + underlined.

### Architecture / Refactor

- **refactor(init):** extract `commands::init::core::init_core` (non-interactive detect-and-persist phase) plus reusable helpers out of the 400-line `init::run`. New `commands/init/{core,report,mod}.rs` layout; CLI wrapper owns prompts + `printer::*` calls. (D1, 7866f7f)
- **refactor(checkout):** extract `commands::checkout::core::checkout_core` returning a typed `CheckoutReport`. New `commands/checkout/{core,report,mod}.rs` layout. (D2, 9462f23)
- **refactor(sync):** split the 678-line `sync_inner` orchestrator into 4 phase helpers (`expand_paths`, `decide_batch`, `distribute_batch`, `run_recursive_entries`); `sync_inner` is now a thin orchestrator. `sync_path_across` preserved verbatim. (D3, 967ba9a)
- **chore(cleanup):** delete dead code flagged by audit §2.4 — `host/filter.rs`, `probes::command_for`/`path_size_command`, `PoolHostResult`/`SshPool::reachable_hosts`, stale `#[allow(dead_code)]` on `SshHostEntry`. Deviation: `parse_batch_metadata_output` was kept (live caller discovered). (F1, 4903fc0)
- **refactor(tui):** consolidate triplicated helpers into `src/tui/components/shared.rs` (`shell_label`, `chips`, `truncate`, `focus_accent`, `collect_groups`). Deferred: `ShellMode` → `ShellType` collapse (would silently change on-disk TOML format). (F2, bc4a592)
- **chore(tui):** delete the dead `Focusable` trait + adapter tests (zero production callers). (F3, 75c869f)
- **test(host):** introduce `SessionPool` trait + `MockSessionPool` test helper; refactor `shell::detect_russh`, `init::InitPools`, and the 4 sync phase helpers + `sync_path_across` to take `&dyn SessionPool` / `Arc<dyn SessionPool>`. 31 new tests cover `init_core` happy + unreachable + mixed + skip + stale-removal, the 4 sync phase helpers, `sync_path_across`, and `host::pool` smoke. (H1, d8f5564)

### Bug Fixes

- **fix(sync,check,cp):** surface DB write errors at the 7 sites that silently dropped `operation_log`/`sync_state` inserts via `let _ = ctx.db.execute(...)` — now logged via `tracing::warn!`. (A3, 1330fa2)
- **fix(host):** acquire per-host permit before global in `ConcurrencyLimiter::acquire`, removing head-of-line blocking where N tasks queued on a saturated host starve an independent host. (A8, 2409b21)
- **fix(state):** set `PRAGMA busy_timeout=5000` and `PRAGMA synchronous=NORMAL` so concurrent CLI + TUI access waits instead of failing with `SQLITE_BUSY`. (A2, 268cbc6)
- **chore(tui):** serialise the 7 env-mutating tests in `src/tui/theme.rs` through a module-level `Mutex<()>` to fix an intermittent parallel-test flake. (34977f5)

### Documentation

- **docs(agents):** align AGENTS.md with the actual russh-based transport, the single `sshi` binary, and the actual module layout. Drop the stale `thiserror` mandate. Correct sync strategy from BLAKE3 to SHA-256. Fix `host/pool.rs` doc-comment. (A10, feb3032)
- **docs(adr):** add `docs/adr/0002-russh-migration.md` recording the russh migration decision, trade-offs, and consequences. (A11, included in feb3032)
- Per-phase execution notes with deviations, scope gaps, and carry-overs recorded in `docs/plan/2026-07-18-audit-fixes.md` (159e2fc, 75e0788, e4328bb, 990c22b, 90c44f1, d77d1e3, de44f6a).

## [v1.6.1] - 2026-07-08

### Changed
- **refactor:** extracted `src/lib.rs` with `[lib]` config; module declarations moved from `main.rs`, imports now use `sshi::` prefix
- **refactor:** split monolithic `sync.rs` (2,700+ lines) into `sync/` module with `collect`, `decide`, `distribute`, `report`, `tests`, `types` submodules
- **refactor:** extracted `tui/app_state.rs` from `tui/app.rs` for clearer state separation
- **refactor:** replaced N+1 checkout DB queries with batch parameterized queries
- **refactor:** `HostEntry::placeholder()` constructor for init-time use before shell detection
- Removed unused `thiserror` dependency and stale `#[allow(dead_code)]` annotations

## [v1.6.0] - 2026-06-11

### Fixed
- **fix(init):** `ssh-keyscan` now writes **unhashed** known_hosts entries (dropped `-H`). russh's `check_known_hosts` only matches plain `host`/`[host]:port` tokens by string equality and does not match hashed (`|1|`) entries, so hosts added with `-H` — especially non-standard-port hosts looked up as `[host]:port` (e.g. a local WSL sshd on `:8022`) — were still rejected as "Unknown host key" even after `init` accepted them
- **fix(ssh):** connection and SFTP-probe failures now report the full error cause chain (`{:#}`) instead of only the outermost context, so masked causes (e.g. an unknown host key behind "Failed to connect to host:port") are visible in the host error line and summary
- **fix(tui):** sync activity (`synced`, `all in sync`, dry-run `would sync`) is now logged at `info` level so it appears in the TUI log overlay — previously these were `debug` and filtered out by the default `info` tracing filter

## [v1.5.0] - 2026-06-10

### Added
- **feat(tui):** ESC key now cycles through 3 focus levels (NavBar → TopField → Content) on Operate and View tabs — Config tab retains the original 2-way toggle
- **feat(tui):** Press `e` on a View List result row to open the Config edit form for that entry; cursor returns to the same row after commit or cancel

### Changed
- **refactor:** `ShellType::fmt` uses `f.pad()` instead of explicit `write!` matches

## [v1.4.0] - 2026-06-09

### Added
- **feat(checkout):** `--combined-view` flag and TUI toggle (`c` shortcut) — each metric column shows the most recent recorded value across up to 50 historical snapshots instead of the single latest snapshot, so mixed probe coverage fills in from different collection times
- **feat(log):** `run` and `exec` now record the first non-empty line of stdout in `operation_log.stdout` (schema v2 migration); `sshi log` and the TUI View Log display a `↳` preview line for entries without a note
- **feat(tui):** completed-operation report popup now supports scrolling (↑↓/j/k/PgUp/PgDn/Home/End) for long outputs; previously only Esc/Enter to dismiss was supported
- **feat(tui):** `checkout_combined` preference is persisted across restarts

### Fixed
- **fix(tui):** member picker scroll no longer anchors the cursor to the bottom row when navigating backward — replaced the hand-rolled scroll window with the shared `Viewport` component so `move_up`/`move_down` update `scroll_y` independently of `selected`

## [v1.3.3] - 2026-06-08

### Fixed
- **fix(tui):** TUI 的 skip 清單（target filter）現在對所有操作指令（cp、run、exec、sync、check）正確生效——`from_tui_parts()` 先前硬編碼 `skip: Vec::new()`，導致在 TUI 中設定的 skip hosts 被完全忽略，每個 `execute_*` 函數現在會捕獲 `target_filter.skip` 並傳入 `Context`

## [v1.3.2] - 2026-06-05

### Fixed
- **fix(path):** `--out` 報告路徑與 `exec` 腳本路徑現在支援前導 `~` / `~/` / `~\` 展開——Windows shell（cmd/PowerShell）與 TUI 文字欄位不會自動展開 `~`，先前在 Windows 寫報告會出現 `Failed to write report to '~\test.html'`。同時將分散的三份 `expand_tilde` 收斂為共用的 `crate::util::expand_tilde`（config 路徑、cp 來源、ssh identity_file 一併改用）。

## [v1.3.1] - 2026-06-05

### Added
- **feat(config):** `AppConfig::default()` 預設包含一筆 `[[check]]` name="default"，enabled 所有 metrics（online/system_info/cpu_arch/memory/swap/disk/cpu_load/network/battery/ip_address）；首次 `sshi init` 或在無 config 環境下執行時會自動帶入這筆 entry
- **feat(log):** View Log action filter 新增 `cp` 選項（check/run/exec/cp/sync）
- **feat(operate):** Del 鍵可清空 Command、Script、CpLocal 欄位（同 CpRemote、Out 等）
- **feat(persist):** Operate tab 的 run command、exec script path、cp local/remote 欄位在重啟後保留上次輸入值

### Fixed
- **fix(view):** View tab List result panel 焦點不再卡在結果區塊中，↑ 在最頂端時可正確離開回到上方 stop（移除多餘的 `scroll_y == 0` 條件）
- **fix(check):** `check` 操作現在會寫入 `operation_log`，使 View Log 可顯示 check 執行記錄
- **fix(cp):** operate cp 的 local path 支援 `~/…` tilde 展開（Linux/Windows 都適用）

## [v1.3.0] - 2026-06-05

### 2026-06-05 — Del quick-clears optional fields; inline name validation
- feat(tui): **Del** clears the focused optional field — Operate: target
  members, skip, check/sync names, source, `-o/--out`, cp-remote (and removes
  the last ad-hoc path); Config: optional scalar fields (`proxy_jump`, sync
  `mode`/`source`). Required fields (Command/Script/cp-local, host name/ssh_host,
  `[[check]]`/`[[sync]]` names) are intentionally not clearable.
- fix(config): inline-editing a `[[check]]`/`[[sync]]` name to empty/duplicate is
  now rejected (previously only the add/edit form validated it).

### 2026-06-05 — Operate sync source picker, drop applicable-entries panel, error auto-dismiss
- change(tui): the sync **Source override** is now a single-select host popup
  (Enter to choose; "(none)" clears it) instead of a free-text field. Space
  cycles the value in place (none → host → … → none), mirroring Target Shell.
- change(tui): the read-only **"Applicable [[check]]/[[sync]] entries" panel is
  removed** — each entry's detail (metrics / paths + source) now shows inline as
  a dimmed hint after its name in the selection popup.
- fix(tui): the bottom error/warning banner now **clears when you switch tabs**,
  so a stale message (e.g. "name cannot be empty") no longer sticks.

### 2026-06-05 — Config autosave, name-based entry selection, sync simplification, init key-copy
- feat(tui): Config changes now **autosave** to disk on every committed edit
  (format-preserving via `toml_edit`); the main-view `s:Save` key and footer hint
  are removed.
- feat(config): `[[check]]`/`[[sync]]` entry names are now required to be
  **non-empty and unique** — the add/edit form rejects empty or duplicate names;
  legacy duplicates/blanks emit a non-fatal warning on load.
- feat(tui): the Operate Check/Sync **entry-name fields are now multi-select
  popups** (Enter to choose) instead of free-text; `a` inside the popup jumps to
  the Config add-entry form and returns to the picker afterward.
- change(tui): the sync **config/ad-hoc mode toggle is removed** — config-entry
  names and ad-hoc paths are now shown together and both feed the sync at once;
  the **Source override input moved to the bottom** of the sync params.
- feat(init): hosts that fail **key authentication** are now offered an
  interactive `ssh-copy-id` (and `ssh-keygen -t ed25519` first if no key exists),
  then retried — alongside the existing host-key handling.

### 2026-06-05 — TUI Operate fixes: Execute hotkey label, progress-bar bleed
- fix(tui): the Operate Execute button now advertises the `e` shortcut —
  `[ Execute check (Enter) ] (e)`.
- fix(tui): suppress the CLI indicatif progress bar while the TUI is running. It
  was drawing to the shared terminal (stderr) during an operation, corrupting the
  alternate-screen layout (stray `Hosts ░░… 0/6` / `█████ 6/6` artifacts).
- feat(log): `log --last 0` now returns all matching entries (maps to SQLite
  `LIMIT -1`); any non-zero value still caps the result as before.

## [v1.2.0] - 2026-06-05

### 2026-06-05 — TUI polish: entry names, `e` to execute, Tab-cycling radios, version header
- change(tui): the Config sidebar now shows each `[[check]]`/`[[sync]]` entry's
  `name` directly (falling back to `Check #n`/`Sync #n` when unnamed), instead of
  the old `Check #n [name]` / `Sync #n: path` labels.
- feat(tui): press `e` anywhere on the Operate tab to run the current operation
  (shortcut for focusing `[Execute]` + Enter).
- feat(tui): `Tab`/`Shift+Tab` now cycle the selected option in place on every
  radio — the Operate operation radio, the Operate target row, the Operate sync
  mode (Config entries ↔ Ad-hoc), the View "Show" selector, and the View target
  row — the same as `←`/`→`. `↑`/`↓` still steps between fields.
- change(tui): renamed the View tab's `Op:` selector label to `Show:` (Checkout /
  List / Log are views, not operations).
- feat(tui): the header bar shows the version number (`v<x.y.z>`) in the
  top-right corner.

### 2026-06-04 — Name-based [[check]]/[[sync]] selection (breaking config change)
- **breaking(config):** removed the `groups`, `enable_hosts`, and `enable_all`
  fields from `[[check]]` and `[[sync]]` entries. Entries are now selected by
  their `name`. Old configs still parse (unknown keys are ignored), but those
  fields no longer have any effect.
- change(cli): target flags (`-a`/`-g`/`-h`/`-s`) now only select **hosts**;
  `-n/--name` selects **which entries** to apply (orthogonal).
- feat(cli): `check` gains `-n/--name` (comma-separated). With no `-n`, the entry
  named `"default"` is applied (if present).
- change(cli): `sync` drops `-f/--files`; paths are now positional
  (space-separated) and combine with `-n/--name`. Passing neither errors.
- change(tui): the Operate tab gains entry-name inputs for check and sync
  (config-entries mode); the config editor now edits each entry's `name` and no
  longer shows the removed scope fields. The `list`/View panes show entry names
  instead of scope.

### 2026-06-04 — `cp` command: copy local files/dirs to hosts
- feat(cli): new `cp` subcommand copies a local file, directory (recursive), or
  quoted wildcard pattern to remote hosts. Two positional args — local path
  (required) and remote path (optional, defaults to the remote home directory,
  mirroring `scp`). A leading `~` in the remote path is expanded per host/shell.
  Supports the shared target / `--serial` / `--timeout` / `--dry-run` / `--out`
  arguments. Per-file SFTP transfers remain capped at 64 MB; oversized files are
  reported and skipped.
- feat(tui): new **cp** operation on the Operate tab with local-path and
  remote-path inputs.

### 2026-06-04 — Relax minimum terminal size
- feat(tui): lower minimum terminal size from 80×24 to 60×20 so the TUI is
  operable on phones (Termux landscape, mobile SSH clients); below the new
  threshold the "Terminal too small" guard still applies.

## [v1.1.0] - 2026-06-04

### 2026-06-04 — TUI universal scroll / jump keys
- feat(tui): `PageUp` / `PageDown` / `Home` / `End` now work in every scrollable
  vertical region that previously only had ↑↓: the Operate applicable-entries
  panel, the running-operation progress popup, the member/skip/group pickers, and
  the Config entry-form vec-editor and group-picker sub-popups.
- fix(tui): the member/skip/group picker popup now scrolls to keep the cursor
  visible instead of clipping options when the list overflows the popup height.

### 2026-06-04 — TUI collapsible Config sections
- feat(tui): the **Config** sidebar's Hosts/Checks/Syncs section headers are now
  collapsible. Each shows a ▼ (expanded) / ▶ (collapsed) disclosure triangle;
  press `Space` or `Enter` on a header to toggle. Collapsing hides that section's
  child entries and keeps the cursor on the header (never on a hidden child).
  Adding an entry auto-expands its section so the new row stays visible.

### 2026-06-04 — TUI block-division layout (Operate + View)
- change(tui): both tabs now use the Config tab's lighter per-zone block style
  (no outer wrapper; the tab-identity title sits on the primary zone block).
- change(tui): the **Operate** tab's Execute action now lives in its own bordered
  ` Execute ` block, separated from the ` Operate ` body block (OpRadio / Common /
  Command-specific / Entries); each border lights up with the Operate accent when
  its layer holds focus.
- change(tui): the **View** tab is now split into a bordered ` View ` block (op
  selector + target/common + Log-specific params) and a bordered Results block
  (titled per operation — Checkout/List/Log), each border accenting when its
  layer holds focus.

### 2026-06-04 — TUI Tab-key layer cycling (Operate + View)
- change(tui): on the **Operate** and **View** tabs, `Tab`/`Shift+Tab` now cycle
  focus among peers *within the current layer only* (wrapping at the layer ends),
  matching the Config tab's principle. Arrow keys continue to cross layer
  boundaries. Operate layers: Op → Common settings → Command-specific → Entries →
  Execute. View layers: OpSelector → Settings → Result.

## [v1.0.1] - 2026-06-04

### Fixed
- fix(tui): remove `DisableMouseCapture` from terminal setup — resolves spurious mouse events interfering with keyboard input on some terminals
- fix(test): Windows-compatible temp-file handling in config round-trip and TUI flush tests — use `TempPath` (closed handle) instead of `NamedTempFile` to avoid "Access denied" errors on atomic rename
- fix(test): clippy `field_reassign_with_default` — use struct literal initialisation in `operate_state_extended_round_trips` and `skip_field_round_trips_and_defaults_empty`

## [v1.0.0] - 2026-06-03

### 2026-06-03 — Log & List `--out` / TUI Export `o`
- feat: log and list subcommands now support the `-o/--out` parameter, allowing log queries and host/check lists to be exported as structured JSON or HTML reports.
- feat(tui): added the `o` hotkey to the View tab when viewing Checkout, List, or Log results. It prompts the user for an output path (or empty for auto-named) and exports the currently viewed data using the same serialization logic.

### 2026-06-03 — Operate operation order + View navigation consistency
- change(tui): the Operate **Operation radio is reordered to `run · exec · sync ·
  check`** (check stays the default selection, now shown last). `←→` cycles in
  that order.
- change(tui): the View **Log `action` filter is now toggled with `Space`** (it
  cycles all → check → run → exec → sync). `←→` no longer changes its value —
  arrow keys move the focus cursor only, matching the other Log fields.
- fix(tui): the View **List result cursor skips decorative lines** (section
  titles, the column header, the separator, blank spacers, and empty `(none)`
  placeholders). The focus cursor now only lands on real data rows, so it no
  longer appears to vanish onto a blank/black line.
- feat(tui): **Tab/BackTab now work in List and Log**, consistent with Checkout
  — they cycle the result-row cursor (wrapping, skipping decorative List rows).
  The **Log result now draws a row cursor** when the result zone holds focus, so
  the selection is visible there too.
- feat(tui): **View Log is more discoverable** — the summary line now reads
  `Log: N entries below (all hosts) — ↑↓/Tab scroll · Enter edits a field ·
  Space toggles errors/action`, and the empty state explains that logs are
  recorded automatically by check/run/exec/sync and suggests relaxing the
  filters.

### 2026-06-03 — Operate wiring: report export, dry-run preview, timeout
- feat(tui): the Operate **Out field now writes a report**. On completion, if a
  path is set, sshi writes a `.json`/`.html` report (auto-named when left bare,
  honouring the config `default_output_format`) and shows `Report written to …`;
  write failures surface in the status banner.
- feat: extracted a shared `output::report::to_operation_report(CommandReport,
  TargetMode)` so the CLI wrappers (check/run/exec/sync) and the TUI build the
  `--out` report from one place instead of four bespoke conversions.
- feat(tui): **dry-run now works for check/run/exec**, not just sync. With
  dry-run on, Execute shows a synthetic preview popup listing each resolved
  target as “would execute” (⊘) and contacts **no** hosts and writes no report —
  mirroring the CLI’s dry-run. Sync keeps its in-core dry-run.
- fix(tui): the **Timeout field now reaches execution** — editing it (`←→ ±5s`)
  updates the per-host timeout used by check/run/exec/sync, and the Timeout row
  shows the resolved default instead of `0s` on first run.
- change: `write_report` no longer prints internally; it returns the written
  path so the CLI prints it and the TUI shows a banner (avoids stray stdout
  corrupting the TUI). CLI report output is otherwise unchanged.
- change(cli): the `sync --out` report now reports real per-host status
  (unreachable/error) and carries the synced/skipped **file-path lists**, built
  via the same shared converter (previously every host was marked `success`).

### 2026-06-03 — dry-run placement, View Shell, per-tab accent colours
- change(tui): the Operate **dry-run** toggle moved out of the Execute bar into
  the Common zone, directly **below Serial** (`[ ] dry-run (d)`), so it reads
  consistently with the other toggles. Space toggles it when focused; `d` still
  works from anywhere. The Execute bar is now just the button.
- feat(tui): **View target now supports Shell** (All/Groups/Hosts/Shell), same
  as Operate — it filters hosts by detected shell type. `←→` cycles it, `Space`
  cycles the shell value, `Enter` opens the single-select picker.
- fix(tui): **View is now consistently green.** Each tab owns an accent colour
  (Config=yellow, Operate=cyan, View=green) used for its panel border and any
  popup it opens; previously the View frame and the member picker borrowed the
  shared cyan `border_active`, so View looked like Operate. The member picker is
  now themed to the tab that opened it.
- feat(tui): the View **List** result shows a green row cursor on the selected
  line when the list holds focus, mirroring Checkout, so focus position is
  visible. (Note: the cursor can still land on section headers; restricting it
  to data rows is a later refinement.)
- change(tui): the Operate output field is relabelled **`Output report
  (.json/.html, optional)`** (dropped the cryptic `-o`) and moved to the bottom
  of the **── Common ──** zone.
- feat(tui): pressing **Enter on the Target *mode* row** now opens the relevant
  picker (multi-select for Groups/Hosts, single-select for Shell), and **Space
  on the mode row** cycles the shell in Shell mode — previously only the value
  row responded. Applies to both Operate and the shared View interface.
- fix(tui): the View **Log** specific-params (`last`/`since`/`host`) no longer
  render broken `┌ … ┐` boxes — every Log field (last/errors/action/since/host)
  is now a uniform single line whose value reverse-highlights when focused, with
  an inline cursor while editing. Removed the now-stale `[f] to set target
  filter` note.
- feat(tui): Operate **Target mode and its value row are now linked** — `←→`
  cycles the target mode from either row, so the radio and the picked value
  behave as one control. Switching to All while on the value row keeps focus on
  the Target row instead of stranding it.
- feat(tui): Operate now exposes an **`-o/--out` report path** input (shared by
  check/run/exec/sync), placed just above the applicable-entries / Execute area.
  (Field + navigation wired here; report-file writing was hooked up later the
  same day — see the "report export" entry above.)
- feat(tui): **View Checkout/List replace the old `f` filter popup with an inline
  Common zone** (target mode radio → members → skip), mirroring Operate: `←→`
  cycles the mode, `Enter` opens the same multi-select picker for groups / hosts
  / skip. Log keeps its greyed "no target" summary. The `FilterPopup` component
  is retired (the `target_filter` module is unlinked; the file is left in place
  but no longer compiled).

### 2026-06-03 — TUI focus-highlight principle + Operate target fixes
- fix(tui): the Operate **Groups** picker now offers every group referenced in
  the config (host **and** check/sync entries, plus the current selection),
  matching the Config tab's `collect_known_groups`. Previously it only scanned
  host groups, so configs that scoped groups on check/sync showed nothing.
- fix(tui): **emptying a Groups/Hosts selection no longer snaps Target back to
  All.** The mode is preserved and an empty list now resolves to *zero* hosts
  (clearer and safer than silently targeting every host). `validate_filter` and
  `build_target_mode` were updated to stop the fallback at both edit and load
  time.
- change(tui): Operate **Serial** toggle moved below **Timeout**, and a new
  **`s`** shortcut toggles serial from anywhere in the tab (shown as
  `[ ] Serial (s)`).
- feat(tui): applied a consistent **focus-highlight principle** across the UI —
  only the element holding the focus cursor is reverse-video; every other
  "selected" element (active NavBar tab, selected row of an unfocused panel) is
  bold/accent only. Fixes the NavBar tab staying reversed after focus moved into
  a panel, and the View tab reverse-highlighting both the Op selector and the
  checkout row at once (so it was unclear where focus actually was).

### 2026-06-03 — TUI Operate/View redesign feedback fixes
- fix(tui): **Tab/Shift+Tab now cycle Config/Operate/View** while the NavBar
  holds focus (previously a no-op there).
- feat(tui): when focus moves up to the NavBar, the **Operate** and **View**
  panels now visibly relinquish focus — the selected radio/row drops its
  reverse-highlight to bold-accent and the row arrow changes `▶`→`>`, matching
  the existing Config behavior.
- fix(tui): Config sidebar selection arrow `▶` no longer overlaps the first
  letter of the entry name (added a trailing space, consistent with the field
  table).
- fix(tui): **Operate Target can now actually switch into Groups/Hosts** — the
  live `←→` mode change no longer runs `validate_filter`, which was snapping an
  empty Groups/Hosts selection straight back to All.
- feat(tui): in **Shell** target mode the shell value is cycled inline with
  **Space** (sh → powershell → cmd), since it is a fixed single choice.
- feat(tui): Operate **sync** params now keep `Source override` anchored on top
  with the ad-hoc `Add path` input + file list below it, for a stabler layout;
  focus-walk order matches the new visual order.
- change(tui): Operate execute bar reordered to `[ Execute … ]` first, then the
  `[ ] dry-run (d)` toggle.
- polish(tui): trimmed Operate chrome — `── Common ──` header, removed the
  redundant per-field inline key hints (the status row already lists them).

### 2026-06-02 — TUI Operate redesign (flat zoned layout MVP)
- feat(tui): redesigned the **Operate** tab around a single unified field walk
  (`OpField`) instead of nested focus zones + a sub-`ParamPanelField`. All
  parameters are now laid flat on the first layer in two zones: a **Common**
  zone (target mode, members, skip, serial, timeout) that is shared across and
  preserved when switching operations, and a **per-command** zone (command/
  script/sudo/keep, or sync mode/ad-hoc files/source) that swaps with the op.
- feat(tui): target **groups / hosts / shell / skip** are now actually editable
  via a working multi/single-select `member_picker` popup (Space toggles,
  Enter applies, Esc cancels), opened by pressing **Enter** on the Members or
  Skip field. This replaces the broken `cycle_chip` logic that could only ever
  push the first available item.
- feat(tui): **←→ changes the value** of any focused field — operation radio,
  target mode, **sync config/ad-hoc mode**, and timeout (±5s). Space toggles
  booleans (serial/sudo/keep); `d` toggles a single shared **dry-run** flag
  shown next to the Execute button.
- feat(tui): the `⚡ Ad-hoc mode` notice moved from a jarring top banner into
  the bottom status row, shown only while sync is in ad-hoc mode.
- note: this is a UI/navigation MVP for evaluation — Execute still dispatches
  the real operations via the existing pipeline; only the Operate-tab layout,
  navigation, and target editing were reworked. The legacy `FilterPopup`
  remains in use by the View tab.

### 2026-05-27 — TUI Operate/View refactor
- feat(tui): the **Operate** tab now launches `check`/`run`/`exec`/`sync` in a
  single-column "Approach-B" layout (op selector → target summary line →
  operation-specific params → Execute). Param toggles (sudo/keep/sync-mode/
  sync-dry-run) flip through `operate_schema::apply_specific` for Config-consistent
  behavior; the param panel itself still uses the existing renderer
  (`FieldDescriptor`-driven Operate rendering is deferred).
- feat(tui): new **View** tab hosts `checkout`/`list`/`log` with a live
  auto-refreshing result area (cycle ops with ←/→; refreshes on op switch and
  after each operation). The View tab absorbs and replaces the former Checkout
  tab; a persisted `[tui_state] active_tab = "Checkout"` auto-migrates to `View`
  (serde alias) without resetting.
- feat(tui): `log` in the View tab exposes editable `last`/`errors`/`action`/
  `since`/`host` params; `checkout`/`list` honor the target filter, while `log`
  queries all hosts (its target row is inert).
- feat(tui): the target-filter popup gains a `skip` modifier (hosts excluded
  from the resolved set, applied as a final subtraction in resolution).
- feat(tui): the `sync` operation's `source` override input is now wired
  (previously dormant) and passed through to `sync_core`.
- refactor(commands): extracted `list_core`/`log_core` data functions from
  `list::run`/`log::run`; the `run` wrappers print from them with identical
  output (and `list` still surfaces the no-hosts diagnostic).
- remove(tui): the dead `run --yes` toggle is gone (the CLI removed `-y/--yes`).
- note: `init` stays CLI-only (its interactive stdin prompts can't run in the
  raw-mode TUI). `dry_run` for `check`/`run`/`exec`, the `-o/--out` field, and
  `checkout` `history`/`since` are intentionally **not** surfaced in the TUI this
  cycle — those paths aren't wired in the cores (deferred to a follow-up).

### 2026-05-26 — TUI navbar quit fix + CLI help ordering
- fix(tui): pressing `q` while the top navigation bar has focus now quits
  (state saved). Previously the navbar key handler's catch-all swallowed `q`,
  so it was a no-op until you left the navbar. `src/tui/app.rs` navbar block.
- change(cli): per-subcommand help now displays options in a consistent
  grouped order via clap `display_order` — grouping flags (`-g/-h/-a/-s`),
  then common flags (`--skip/--serial/--timeout/-H/-c`), then command-specific
  flags (`--sudo`, `--dry-run`, …), and `-o/--out` last. Applied to the
  host-operating commands (`check`, `checkout`, `sync`, `run`, `exec`, `list`);
  `init`/`log`/`config` keep their existing order.

### 2026-05-26 — Rename project ssync → sshi
- **BREAKING:** the project, crate, and binary are renamed from `ssync` to
  `sshi`. Invoke the CLI as `sshi`; `cargo install sshi`; repo is now
  `github.com/superyngo/sshi`.
- **BREAKING (config/state paths):** default config dir moved
  `~/.config/ssync/` → `~/.config/sshi/` and state dir `~/.local/state/ssync/`
  → `~/.local/state/sshi/` (DB file `ssync.db` → `sshi.db`). The old paths are
  **not** migrated automatically — move your files manually (see README
  "Migrating from `ssync`").
- Mechanical sweep across `Cargo.toml`, all source identifiers/strings/help
  text, the clap command name, migration headers, temp-file prefixes, thread
  names, and docs.

### 2026-05-26 — TUI Config tab: save hint + unified option-field cycling
- fix(tui): main-view Config footer now shows the `s:Save` hint (was only on
  the entry-form footer). `src/tui/app.rs` Config hints line.
- change(tui): all rotating/toggle field kinds (`Bool`, `TriBool`, `ShellEnum`,
  `Enum`) now cycle via Enter/Space through one shared `cycle_option_value`
  helper; Left/Right no longer hijack value changes and are freed for
  navigation. Previously `ShellEnum`/`TriBool`/`Enum` cycled on Left/Right,
  unlike `Bool` (Space/Enter only). Applies to both the right-panel inline
  editor and the entry-form popup. Backward-cycle helpers retained behind
  `#[allow(dead_code)]` (no key triggers them now). Operate tab untouched.

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

### 2026-05-21 — TOML serializer fix, explicit-save UX, vec_editor close fix
- **fix(config): writer now persists `[[host]]` / `[[check]]` / `[[sync]]`
  sections.** Root cause of "edits look saved in TUI but vanish after
  restart": `apply_config_to_doc` (`src/config/app.rs:161`) was scoped to
  `[settings]` only; per-entry edits were silently dropped on every save. New
  `host_to_table` / `check_to_table` / `sync_to_table` helpers rebuild the
  array-of-tables on write. Per-entry inline comments lost (acceptable for a
  tool-managed file); top-level section comments preserved.
- **change(tui): autosave-on-mutate removed; explicit `s` saves from main
  view.** `mark_dirty` no longer sets `pending_save` — it just captures the
  cursor snapshot. New `request_save_if_dirty()` flips `pending_save` when
  the user presses `s` in Sidebar or FieldTable zone. Quit-time
  `flush_dirty_config_to_disk` retained as safety net. Rationale: autosave
  masked persistence bugs that only surfaced on next program start; explicit
  save makes "did this actually write?" visible at the moment of action.
- **fix(tui): entry-form vec editor (`Sync.paths` etc.) can be closed with
  `s` or Esc.** Pre-existing bug: caller `take()`s `form.vec_editor` then
  unconditionally restores it after handler runs, so the handler's
  `form.vec_editor = None` was a no-op. Added `closing: bool` flag mirroring
  `GroupPickerState`; caller now checks and drops instead of restoring.
- tests: new `host_check_sync_edits_round_trip_through_save` covers the
  writer regression.

### 2026-05-21 — Config-tab unified schema refactor (real fix for Vec save & cursor)
- fix(tui): Hosts/Checks/Syncs Vec edits (`groups`, `enabled`, `paths`) now
  actually persist to disk. Root cause: `apply_*_field` matched only scalar
  keys; Vec keys fell through `_ =>` and were silently dropped, so direct
  popup commits looked like they saved but wrote unchanged config.
- fix(tui): right-panel `field_vp` cursor stays put after any commit (direct
  popup, inline edit, cycle). Was always resetting to row 0 because
  `restore_selection` had no branch for "entry form closed, field cursor
  outstanding"; the snapshot also didn't carry `field_vp.selected`.
- fix(tui): sidebar cursor stays put after entry-form (`e`) commit. Was being
  wiped by `commit_entry_form` BEFORE `save_config` captured the snapshot;
  snapshot capture moved to mutation entry points (mark_dirty).
- refactor(tui): unified field schema in new `src/tui/tabs/config_schema.rs`.
  Single `fields()` + `apply()` per entry kind, used by right-panel inline
  edit, direct popup, and entry-form commit. Removes parallel
  `*_descriptors` / `*_form_fields` / `apply_*_field` definitions in
  `config_tab.rs`.
- refactor(tui): cursor preservation via
  `ConfigTabState::pending_restore_snapshot` + `mark_dirty()` helper at every
  commit site. `app.rs:save_config` now consumes via
  `consume_pending_snapshot()`; `capture_selection` downgraded to
  `pub(super)` so `app.rs` can't accidentally self-capture and bypass the
  stored snapshot.
- change(tui): `Host.proxy_jump` always visible in right panel (was hidden
  when `None`). Empty string when unset.
- change(tui): `Check.enabled` edited from the right panel now opens the
  group-picker over the fixed `CHECK_ENABLED_OPTIONS` catalog with
  descriptions (matching entry-form behaviour), instead of the free-text vec
  editor. Prevents typos in check kind ids.
- tests: 7 new schema unit tests + 4 new integration tests covering Vec
  persistence and cursor preservation across all entry points.

### 2026-05-21 — Config-tab crash fixes, autosave-on-quit, cursor preservation
- fix(tui): eliminate three render-time panics when editing empty
  vec fields (Sync.paths, Settings.skipped_hosts, Check.enabled).
  Root cause: viewport set_dims callers passed len().max(1) on empty
  lists, then render hand-sliced items[vs..ve_end]. Fixed by adding
  Viewport::visible_slice and removing the .max(1) lie everywhere.
- feat(tui): autosave dirty config on quit and before opening the
  external editor (E). Closes a persistence gap for Hosts/Checks/Syncs.
- refactor(tui): preserve cursor position across save+reload via
  ConfigSelectionSnapshot (subsumes pending_field_restore).
- docs(audit): TUI audit results at docs/superpowers/audits/2026-05-21-tui-audit.md.

### 2026-05-08 — TUI Config UX: 9 improvements
- Fix: TriBool stale editing_field_index no longer writes to wrong field (pre-fix)
- Bool fields now toggle with Space/Enter inline (no text input required)
- Tab key cycles main tabs while navbar is focused (stays in navbar)
- GroupPicker supports adding new group names with 'a' key
- shell and conflict_strategy fields cycle with Left/Right/Enter
- Esc on inline edit no longer shows "Config saved" banner
- Field selection preserved after saving entry form popup
- All global shortcuts blocked when any config popup is open
- Vec/groups fields open sub-popup directly from main Config screen


### Unreleased Update — 2026-05-08

#### Fixed
- `✓ Config saved` banner unified to bottom status bar (same position as error messages)
- Confirm dialog y/n now responds correctly when triggered from within an entry form (routing bug where keypresses were swallowed by the entry form handler)
- Sub-popups (vec_editor, group_picker) now capture all keys, preventing global shortcuts (`q`, `?`) from firing while a sub-popup is open
- `vec_editor`: `s` key now commits the list (previously only Esc worked)
- `vec_editor`: cursor (yellow block) now visible in `New:` input prompt
- Group picker and vec_editor hint text updated to show `s/Esc:done`

#### Added
- `check.enabled` field now uses a fixed 10-option multi-select picker with descriptions instead of free-text add/del editor
- Bool fields toggle with Space bar; Enter/e also toggles; hint bar updated to show `[Space] Toggle bool`


Older releases (v0.1.0 – v0.9.0): [docs/reference/changelog/v0.x.md](docs/reference/changelog/v0.x.md).
