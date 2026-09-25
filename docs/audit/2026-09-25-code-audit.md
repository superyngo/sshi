# Code audit
Status: In progress

Whole-crate audit of `src/` (80 files, ~30 k lines) for bugs, optimization, simplicity, clarity,
and integration. Eight read-only reviewers each took one module group; the controller ran the
tooling, re-read the code behind every P1/P2 claim, and reproduced what could be reproduced.
Every actionable finding is filed in [`plan/BACKLOG.md`](../plan/BACKLOG.md) as B20–B61; this
record keeps the evidence. Per `wens-dev-principles docs 20`.

**Verification legend** (column `V`): **R** reproduced on the built binary or a library probe with
the input shown; **C** controller re-read the code and confirmed the mechanism; **A** reviewer
quoted the code, not independently re-read. Per `wens-dev-principles debug 2`, re-measure an **A**
row before fixing it.

## Baseline

| Check | Result |
|---|---|
| `cargo test` | 351 + 1 pass, 0.2 s |
| `cargo clippy --all-targets` (default features) | 0 warnings |
| `cargo clippy --all-targets --no-default-features` | 1 warning: unused imports `fetch_combined_snapshots`, `fetch_latest_snapshots` in `commands::checkout` |
| `cargo fmt --check` | clean |
| `cargo audit` | 3 vulnerabilities: russh 0.44.1 RUSTSEC-2026-0154 and russh-cryptovec 0.7.3 RUSTSEC-2026-0153 (fixed in russh ≥ 0.60.3); rsa 0.9.10 RUSTSEC-2023-0071 Marvin (no fix). Unsound: anyhow 1.0.102 RUSTSEC-2026-0190; lru 0.12.5 RUSTSEC-2026-0253 / -0002 (via ratatui 0.29). Unmaintained: paste, number_prefix. Yanked: spin 0.9.8 |
| CI | None. `ci.yml` deleted in `83aea4d`; `release.yml` builds only (no test/clippy); the untracked `ci.yml.backup` still treats `cargo build` as headless |
| Pedantic/nursery clippy | Mostly style (`uninlined_format_args` 158, `must_use_candidate` 125). Notable: 28 functions over 100 lines — `App::handle_key` is 1010 lines; `init::run` 290; `App::render_results_popup` 251; `check::run` 245. 7 `unused_async` (`Context::new`, `Context::new_without_targets`, `log::run`, `list::run`, `config::run`, `checkout::run`, one in `init::core`) |

## Reproductions

Temp config = the README example with `state_dir = "/tmp/rs"`; `B = target/debug/sshi`.

- **B21 — `sshi log` panics.** After `B log` created the DB:
  `sqlite3 /tmp/rs/sshi.db "INSERT INTO operation_log(timestamp,command,host,action,status,duration_ms,note,stdout) VALUES (strftime('%s','now'),'run','server1','uptime','success',12,NULL,'x系統負載正常，目前有二十五位使用者登入中並且磁碟空間充足可以繼續運行')"`
  → `B log` exits 101: `byte index 72 is not a char boundary; it is inside '碟'`. Without the
  leading `x` the cut lands on a boundary and it prints — the bug depends on content.
- **B22 — combined view drops hosts.** 60 snapshots for `server1`, 1 for `server2` (30 s old).
  `B checkout -a` shows `server2 ✓ online 7.70`; `B checkout -a --combined-view` shows
  `server2 ✗ offline -`.
- **B35 — exit status.** Both README hosts unresolvable: `B check -a` and `B run -a 'echo hi'`
  print `0 succeeded 2 failed` and exit 0 (as `reference/cli.md` documents).
- **B36.** `B -c /tmp/does-not-exist.toml list --all` → `No hosts matched the specified filter.`
  (no mention of the missing file). `B check -a -n typo` → exit 0 with hint
  `Add [[check]] to config.toml.` although a `[[check]]` exists.
- **B20, B42, B43 — library probe** (scratch crate depending on `sshi` by path; `HOME` pointed at a
  temp dir holding `.ssh/id_ed25519` and `.ssh/config` =
  `Host web1 / User alice / HostName 10.0.0.1` then `Match host web* / User deploy / Port 2222`):
  `resolve_host("web1")` → `user=deploy port=2222 identity_files=[]`; `resolve_host("Web1")` →
  no match. `config::app::save` on a file containing `settings = { default_timeout = 5 }` →
  panic `settings must be a table`; `save` through a symlinked config → the link becomes a
  regular file.
- **B39.** On macOS: `sysctl -n vm.loadavg` → `{ 2.26 2.26 2.18 }`; `du -sb` → `invalid option -- b`;
  `df -B1` → `invalid option -- B`; `free` → not found.

## Findings by theme

### Data loss and silent wrong results (P1)

| Row | V | Finding |
|---|---|---|
| B21 | R | Stdout previews are byte-sliced: `commands::log::run` `&trimmed[..72]`, `tui::tabs::view_tab::render_log_result` `&trimmed[..50]`. One stored non-ASCII row makes `sshi log` and the View → Log tab panic on every run until retention purges it |
| B22 | R | `commands::checkout::core::fetch_combined_snapshots` applies `LIMIT 50` to the whole result ordered by host, not per host, contradicting its doc comment |
| B23 | C | `tui::tabs::config_tab::strip_unit` (`trim_end_matches` `s`/`d`/`%`) runs on every editable String/OptionalString field when editing opens (`activate_inline_edit`, entry-form Enter): `prod` → `pro`, `nas` → `na`; Enter without typing saves the corruption |
| B24 | C | `host::sftp::upload` opens the destination with `sftp.create` (truncate) and streams in place; `download` truncates the local file first. Drop, timeout (one `default_timeout` for the whole transfer), or cancel leaves a truncated file. The comment says `shutdown()` surfaces close errors; the result is discarded with `let _` |
| B25 | C | `commands::sync::decide::make_decisions` with `ConflictStrategy::Skip` returns no decisions on a hash conflict; the caller treats that as "all in sync" and calls `summary.file_in_sync` |
| B26 | C | No shared remote-quoting layer. Sites: sync `build_batch_metadata_cmd` Cmd branch (PowerShell double quotes, `$()` expands — sibling of B1), sh path probe `du -sb {}` unquoted, `exec` `chmod +x {remote_path}` / `rm -f` unquoted, PowerShell `sudo_wrap` single quotes unescaped |
| B20 | R | Authentication never tries `ssh-agent` or OpenSSH's default keys: `ParsedSshConfig::query` fills `identity_files` only from an explicit `IdentityFile`, then `authenticate` falls back to a password. `identities_only` is hard-coded `false`; one `IdentityFile` is kept. `reference/config-schema.md` claims "agent" auth |

### Transport robustness (P2)

| Row | V | Finding |
|---|---|---|
| B27 | C | Not every remote step is time-bounded: `authenticate` after the connect timeout (`connect_direct`, `connect_via_proxy`); `open_sftp` in `RusshSessionPool::sftp_session` and `run_sftp_probe` although its doc requires callers to bound it; blocking `to_socket_addrs` on a worker thread outside the timeout |
| B28 | C | `RusshSessionPool::setup` builds a fresh `PassphraseCache` per host task, so a shared key is prompted once per host; the CLI prompts with blocking `rpassword` from up to `concurrency` tasks at once; the TUI's `SshAuthRequired` handler replaces an open `AuthPopup`, dropping the first host's responder; step 2 of `authenticate` asks for a passphrase for keys the server merely rejected |
| B29 | C | `host::auth::SecretString` derives `Debug`, printing the secret |
| B30 | C | No eviction or reconnect: `LazyCache` entries and `RusshSessionPool::sessions` live for the process, so a dropped session fails every later op in a long TUI session |
| B31 | C/A | Windows `--sudo`: `sudo_wrap` PowerShell form has no `-Wait`/`-PassThru` and cmd uses `runas`, so the elevated command's exit status is never observed; `run --sudo --dry-run` always previews the sh form |

### Sync correctness and cost (P2)

| Row | V | Finding |
|---|---|---|
| B32 | A | `batch_collect_all_metadata` drops a host whose batch exits non-zero (one unreadable file under PowerShell) without recording it; the run reports success and Newest may pick a stale source |
| B33 | C | Newest uses `max_by_key(mtime)`, which returns the last of equal maxima in `JoinSet` completion order — same-second edits pick a nondeterministic source |
| B34 | A | Recursive entries run per-file `sync_path_across` (one exec per host per file, fresh temp dir per file) and the legacy `distribute`, bypassing `ConcurrencyLimiter` per-host caps |

### CLI contract and per-command consistency (P2)

| Row | V | Finding |
|---|---|---|
| B35 | R | Exit 0 when every host fails (documented). Scripts and CI cannot detect a failed fleet run — needs a decision |
| B36 | R | Unknown `-n` names warn and exit 0 with a wrong hint; a missing explicit `-c` path silently becomes an empty default config (`init -c typo.toml` would write a new file) |
| B37 | A/C | Same event, different policy: `operation_log` write failure aborts `exec`/`run` after all hosts ran, but only warns in `cp`/`check`; `HostStatus::Partial` is printed `skip`, counted success in `check`, failure in `cp`; `cp` log rows omit errors; `check` unreachable-host writes are outside its transaction |
| B38 | C | `cp::walk_files` silently skips symlinked files/dirs and unreadable entries (`entries.flatten()`) |

### Metrics and state (P2)

| Row | V | Finding |
|---|---|---|
| B39 | R | sh probes on macOS/BSD: `vm.loadavg` braces shift load values; `du -sb` fails → every path `MISSING`; `df -k` fallback stored as bytes (×1024); `vm_stat`/`pmset` output never parsed, stored as empty success |
| B40 | A | Windows path probes: cmd `dir /s` output does not match `parse_path_size` (always 0, success); PowerShell empty directory reported `MISSING` |
| B41 | A | `fetch_latest_snapshots` reads every historical row (with `raw_json`) to keep one per host |

### Config and ssh_config (P2)

| Row | V | Finding |
|---|---|---|
| B42 | R | `parse_ssh_config_content`: `Match` block directives overwrite the preceding `Host`; duplicate `Host` blocks do not merge; matching is case-sensitive; `Include` ignored |
| B43 | R | `config::app::save`: panics on inline `settings = {…}`; replaces a symlinked config with a regular file; no fsync before rename; temp-file mode replaces the original; unknown per-entry keys are dropped by `write_aot` rebuilds |
| B44 | C | `config::app::config_dir` and `state::db::state_dir` ignore `XDG_CONFIG_HOME`/`XDG_STATE_HOME` and hard-code `~/.config` / `~/.local/state` on macOS — `wens-dev-principles cli 2` (MUST): fix or ADR |

### TUI (P2)

| Row | V | Finding |
|---|---|---|
| B45 | C | Config tab drops edits silently: `path:{i}` check rows are editable but `apply_check` ignores them; unparseable numeric settings are discarded in `apply_settings`; both still autosave and reload |
| B46 | C/A | Config tab editors disagree: vec-editor Esc commits in the entry form, discards in the direct popup; entry-form viewport built with height 0 (top-pinned cursor, `wens-dev-principles ui 7`) and sized without its hint rows; form and direct editors are parallel copies; ~30 `Option::unwrap()` encode mode state |
| B47 | A | View → List: `list_entry_at_line` returns early when there are no checks, so sync rows cannot be edited; the layout is hand-mirrored in four functions |
| B48 | A | `InputField`: Backspace/Delete remove one `char`, not one grapheme; no horizontal scroll, so the cursor disappears past the field width (`ui 10`) |
| B50 | A | Work on the UI thread: `render` calls `refresh_view` (SQLite); `write_report` on the event thread; `save_state` (atomic TOML write) on every arrow-key cycle; results popup clones the whole report and rebuilds all lines each frame; target names re-resolved each frame (`ui 16`) |
| B51 | C | `maybe_reload_checkout` clears `db_stale` even when the snapshot fetch fails |
| B52 | C | Help and tab-info text document an `f` filter popup with no handler; its component `components/target_filter.rs` (321 lines) is not in `components/mod.rs` and never compiles |

### Tooling and supply chain (P2)

| Row | V | Finding |
|---|---|---|
| B56 | C | No CI since `83aea4d`; the headless build already regressed unnoticed |
| B57 | R | `cargo audit` advisories above; russh upgrade is the only fix for the two vulnerabilities |

### Simplicity, clarity, single source (P3)

| Row | V | Finding |
|---|---|---|
| B49 | C | `tui::state::persist::load` resets all persisted state when one enum value is unknown (no per-field tolerance) |
| B53 | A/C | Operation scaffolding duplicated: five `App::execute_*` (~90 lines each), four command cores' fan-out/collect loop, `*_core` returning the `CommandReport` enum so wrappers `unreachable!` on other variants; `App::handle_key` is 1010 lines |
| B54 | A | Parallel implementations that already drifted or will: TUI export vs CLI report builders (checkout `task` differs), `App::resolve_target_names` vs `Context::resolve_hosts`, `Summary::print` vs `SyncSummary::print`, Operate vs View target rows, `parse_ssh_config` vs `load_ssh_config` |
| B55 | A/C | Dead code and misleading comments: `operate_schema.rs` (8/10 items dead), sync `host_applicable_paths`/`dirs_missing`, `event::drain_non_resize`, `InitPlan` answer fields with broken doc links, `async_bridge` docs (bounded channel, OS thread), `mkdir_p_sftp` swallows every error, `sync_state` failure logged as `operation_log`, `apply_config_to_doc` doc comment stranded above `set_scalar`, redundant clamp after vec delete |
| B58 | A | `state::db::migrate` writes `user_version` unconditionally; an older binary downgrades the marker |
| B59 | A | PowerShell swap is collected but never displayable |
| B60 | A | `output::printer` emits ANSI colours with no TTY/`NO_COLOR` gate (`ui 20`, `cli 3`) |
| B61 | A | Enums and catalogs re-spelled: shell variants as strings in the Config tab, `ShellMode`↔label in three places, check catalog in `CHECK_ENABLED_OPTIONS` and `AppConfig::default`, script-extension→shell mapping twice in `exec` |

Reviewer HOST-8 restates B18; CFG-1 restates B16 (plus the key-dropping half, filed under B43).

## Suggested order

1. **Quick wins (each < 2 h, P1):** B21, B22, B23, B25 — small, reproduced or confirmed, and each
   silently corrupts data or crashes.
2. **Safety layer (≤ 1 d each):** B26 (closes B1), B24, B27, B28 + B29, then B56 so the fixes stay
   fixed.
3. **Decisions first:** B20 (agent/default keys — scope), B35 (exit status), B44 (XDG vs ADR), B57
   (russh 0.44 → ≥ 0.60 is a large API move).
4. **Structural (multi-day):** B53/B54 extraction, B50 UI-thread work, B39/B40 probe portability.

## Appendix — reviewer findings → backlog rows

| Ref | Pri | Reviewer finding | File | Row |
|---|---|---|---|---|
| HOST-0 | P2 | SSH authentication has no timeout; pool setup can hang forever | `src/host/session_pool.rs` | B27 |
| HOST-1 | P2 | open_sftp is unbounded despite doc requiring callers to bound it | `src/host/session_pool.rs` | B27 |
| HOST-2 | P2 | Blocking DNS resolution outside the connect timeout window | `src/host/session_pool.rs` | B27 |
| HOST-3 | P2 | SecretString derives Debug and prints the secret in plaintext | `src/host/auth.rs` | B29 |
| HOST-4 | P2 | SFTP upload/download are non-atomic; failures leave truncated files | `src/host/sftp.rs` | B24 |
| HOST-5 | P2 | Broken SFTP sessions are cached forever (no eviction/reconnect) | `src/host/session_pool.rs` | B30 |
| HOST-6 | P2 | PowerShell sudo_wrap: unescaped command and meaningless exit code | `src/host/shell.rs` | B31 |
| HOST-7 | P2 | Passphrase prompted for unauthorized (not merely encrypted) keys | `src/host/auth.rs` | B28 |
| HOST-8 | P3 | Limiter doc claims consistent permit order; distribute_pooled acquires in reverse | `src/host/concurrency.rs` | B18 |
| HOST-9 | P3 | mkdir_p_sftp swallows all errors, not just already-exists | `src/host/sftp.rs` | B55 |
| HOST-10 | P3 | run --sudo dry-run preview always renders the sh form | `src/commands/run.rs` | B31 |
| SYNC-0 | P1 | SFTP upload truncates target before transfer; failure leaves corrupted remote file | `src/host/sftp.rs` | B24 |
| SYNC-1 | P1 | Skip-strategy conflicts are reported as all-in-sync and counted as files_synced | `src/commands/sync/mod.rs` | B25 |
| SYNC-2 | P2 | Batch metadata collection silently drops a host on non-zero exit, then syncs without it | `src/commands/sync/collect.rs` | B32 |
| SYNC-3 | P2 | Newest-mtime source selection is nondeterministic on second-granularity ties | `src/commands/sync/decide.rs` | B33 |
| SYNC-4 | P2 | Cmd-shell batch command interpolates paths in PowerShell double quotes, allowing subexpression execution | `src/commands/sync/collect.rs` | B26 |
| SYNC-5 | P2 | Recursive sync does one SSH exec per host per file (N+1) despite having a batch collector | `src/commands/sync/mod.rs` | B34 |
| SYNC-6 | P2 | Recursive entries bypass ConcurrencyLimiter: sync_path_across uses legacy distribute() | `src/commands/sync/mod.rs` | B34 |
| SYNC-7 | P3 | Dead scoping machinery: host_applicable_paths is always None and dirs_missing is never read | `src/commands/sync/collect.rs` | B55 |
| SYNC-8 | P3 | sync_state insert failure logged as failed to record operation_log entry | `src/commands/sync/mod.rs` | B55 |
| CMD-0 | P1 | log output preview panics on multi-byte UTF-8 stdout | `src/commands/log.rs` | B21 |
| CMD-1 | P2 | Remote script path interpolated unquoted into chmod/rm/exec (sh) and inconsistently quoted for PS/Cmd | `src/commands/exec.rs` | B26 |
| CMD-2 | P2 | --sudo on Windows hosts reports success without capturing the elevated command's exit status | `src/commands/exec.rs` | B31 |
| CMD-3 | P2 | operation_log write failures abort exec/run entirely while cp/check only warn | `src/commands/exec.rs` | B37 |
| CMD-4 | P2 | HostStatus::Partial summarized as success in check but failure in cp; run/exec silently drop variants | `src/commands/check.rs` | B37 |
| CMD-5 | P2 | Recursive cp silently skips symlinked files and directories | `src/commands/cp.rs` | B38 |
| CMD-6 | P2 | Commands exit 0 even when every targeted host fails | `src/commands/run.rs` | B35 |
| CMD-7 | P3 | Target-resolution/fan-out/result-collection scaffolding duplicated across exec, run, cp, check cores | `src/commands/run.rs` | B53 |
| CMD-8 | P3 | exec dry-run duplicates the script-extension to shell mapping from exec_core | `src/commands/exec.rs` | B61 |
| CMD-9 | P3 | cp operation_log rows omit per-file errors, unlike exec/run rows | `src/commands/cp.rs` | B37 |
| CMD-10 | P3 | Unreachable-host snapshot writes bypass the batched transaction in check_core | `src/commands/check.rs` | B37 |
| ICS-0 | P1 | Combined view LIMIT 50 is global, silently dropping most hosts | `src/commands/checkout/core.rs` | B22 |
| ICS-1 | P2 | parse_sh_cpu_load misparses sysctl vm.loadavg, shifting load values | `src/metrics/parser.rs` | B39 |
| ICS-2 | P2 | sh probe fallbacks for macOS produce output the parsers never read | `src/metrics/probes/sh.rs` | B39 |
| ICS-3 | P2 | sh path probe uses GNU-only du -sb; macOS remotes report MISSING for existing paths | `src/metrics/probes/sh.rs` | B39 |
| ICS-4 | P2 | cmd path probe output is incompatible with parse_path_size; every size recorded as 0 | `src/metrics/probes/cmd.rs` | B40 |
| ICS-5 | P2 | PowerShell path probe reports existing empty directories as MISSING/failed | `src/metrics/probes/powershell.rs` | B40 |
| ICS-6 | P2 | Probe commands interpolate config paths/labels unquoted; spaces silently corrupt path sizes | `src/metrics/probes/sh.rs` | B26 |
| ICS-7 | P2 | fetch_latest_snapshots scans and allocates the entire snapshot history to keep one row per host | `src/commands/checkout/core.rs` | B41 |
| ICS-8 | P3 | migrate() overwrites user_version unconditionally; downgrade then upgrade corrupts the schema state | `src/state/db.rs` | B58 |
| ICS-9 | P3 | PowerShell swap metric is collected but can never be displayed | `src/commands/checkout/mod.rs` | B59 |
| ICS-10 | P3 | InitPlan interactive-answer fields are dead code with docs linking to nonexistent functions | `src/commands/init/report.rs` | B55 |
| ICS-11 | P3 | Stale-host and skip-list detection duplicated between init CLI wrapper and init_core | `src/commands/init/mod.rs` | B55 |
| CFG-0 | P1 | config_dir ignores XDG_CONFIG_HOME and macOS platform default | `src/config/app.rs` | B44 |
| CFG-1 | P2 | First-run config template documents keys that saves delete | `src/config/app.rs` | B16 |
| CFG-2 | P2 | Match-block directives bleed into the preceding Host block | `src/config/ssh_config.rs` | B42 |
| CFG-3 | P2 | Unknown -n/--name selector exits 0 while --host typo exits 1 | `src/commands/mod.rs` | B36 |
| CFG-4 | P2 | Explicit --config path that doesn't exist is silently ignored | `src/config/app.rs` | B36 |
| CFG-5 | P2 | save() panics when [settings] is an inline table | `src/config/app.rs` | B43 |
| CFG-6 | P3 | Duplicate Host blocks don't merge across blocks | `src/config/ssh_config.rs` | B42 |
| CFG-7 | P3 | HostStatus::Partial labeled 'skip' live but success/failure in summary/report | `src/commands/report.rs` | B37 |
| CFG-8 | P3 | printer.rs emits ANSI colors unconditionally (no TTY/NO_COLOR gate) | `src/output/printer.rs` | B60 |
| CFG-9 | P3 | ~55-line error/skip rendering duplicated between Summary::print and SyncSummary::print | `src/output/summary.rs` | B54 |
| CFG-10 | P3 | parse_ssh_config duplicates load_ssh_config file-read logic | `src/config/ssh_config.rs` | B54 |
| CFG-11 | P3 | Clippy-reported duplication risks (see body) | `src/config/app.rs` | B43 |
| TAPP-0 | P2 | Checkout reload clears db_stale even when snapshot fetch fails | `src/tui/app.rs` | B51 |
| TAPP-1 | P2 | Blocking DB queries and report writes run inside render()/event thread | `src/tui/app.rs` | B50 |
| TAPP-2 | P2 | Results popup clones full report and rebuilds all lines every frame | `src/tui/app.rs` | B50 |
| TAPP-3 | P2 | Five execute_* methods are near-identical ~90-line copies | `src/tui/app.rs` | B53 |
| TAPP-4 | P2 | Help text documents a nonexistent `f` filter popup; 321-line component orphaned | `src/tui/app.rs` | B52 |
| TAPP-5 | P3 | New SSH auth prompt replaces open one, failing the in-flight host auth | `src/tui/app.rs` | B28 |
| TAPP-6 | P3 | async_bridge docs contradict implementation (bounded channel, OS thread) | `src/tui/async_bridge.rs` | B55 |
| TAPP-7 | P3 | TUI View export re-implements CLI report builders and has already diverged | `src/tui/app.rs` | B54 |
| TAPP-8 | P3 | TUI re-implements target resolution duplicating Context::resolve_hosts | `src/tui/app.rs` | B54 |
| TAPP-9 | P3 | drain_non_resize is dead code whose resize handling drops the event | `src/tui/event.rs` | B55 |
| TAPP-10 | P3 | Every arrow-key cycle performs an atomic TOML write on the UI thread | `src/tui/app.rs` | B50 |
| TCFG-0 | P1 | strip_unit corrupts plain string fields on inline edit and form open | `src/tui/tabs/config_tab.rs` | B23 |
| TCFG-1 | P2 | Check path:{i} rows are editable but apply silently discards edits | `src/tui/tabs/config_schema.rs` | B45 |
| TCFG-2 | P2 | Numeric settings edits fail silently: no validation error, value reverts | `src/tui/tabs/config_schema.rs` | B45 |
| TCFG-3 | P2 | Vec editor Esc commits in entry form but discards in direct popup | `src/tui/tabs/config_tab.rs` | B46 |
| TCFG-4 | P2 | Entry-form field list top-pins the cursor: stored viewport never gets real height | `src/tui/tabs/config_tab.rs` | B46 |
| TCFG-5 | P2 | Form-popup and direct-popup editors are parallel implementations of the same widgets | `src/tui/tabs/config_tab.rs` | B46 |
| TCFG-6 | P3 | Dead selection-correction code after vec-item delete (set_dims already clamps) | `src/tui/tabs/config_tab.rs` | B55 |
| TCFG-7 | P3 | Shell variants spelled out as strings, duplicating ShellType/ShellMode enums | `src/tui/tabs/config_tab.rs` | B61 |
| TCFG-8 | P3 | Check catalog duplicated between TUI picker options and AppConfig::default | `src/tui/tabs/config_schema.rs` | B61 |
| TCFG-9 | P3 | Entry-form viewport height ignores blank + hint rows, truncating the hints | `src/tui/tabs/config_tab.rs` | B46 |
| TVIEW-0 | P1 | Panic slicing UTF-8 log stdout preview at byte 50 | `src/tui/tabs/view_tab.rs` | B21 |
| TVIEW-1 | P2 | Sync list rows uneditable via `e` when config has no checks | `src/tui/tabs/view_tab.rs` | B47 |
| TVIEW-2 | P2 | InputField backspace/Delete remove one char, not one grapheme | `src/tui/components/input_field.rs` | B48 |
| TVIEW-3 | P2 | Unknown enum variant in state file silently wipes all persisted TUI state | `src/tui/state/persist.rs` | B49 |
| TVIEW-4 | P2 | InputField lacks horizontal scroll; cursor invisible past field width | `src/tui/components/input_field.rs` | B48 |
| TVIEW-5 | P2 | List/Log renderers rebuild and clone the full content every frame | `src/tui/tabs/view_tab.rs` | B50 |
| TVIEW-6 | P3 | target_filter.rs is an orphaned module that is never compiled | `src/tui/components/target_filter.rs` | B52 |
| TVIEW-7 | P3 | operate_schema.rs is a dead parallel schema layer (8/10 items dead_code) | `src/tui/tabs/operate_schema.rs` | B55 |
| TVIEW-8 | P3 | List layout hand-mirrored in four functions that must stay in sync | `src/tui/tabs/view_tab.rs` | B47 |
| TVIEW-9 | P3 | Target names re-resolved into fresh Vec<String> allocations every frame | `src/tui/app.rs` | B50 |
| TVIEW-10 | P3 | Target mode/members/skip rows duplicated between Operate and View tabs | `src/tui/tabs/operate_tab.rs` | B54 |
| TVIEW-11 | P3 | ShellMode↔label mapping split across three sites; typed converter dead | `src/tui/state/persist.rs` | B61 |
