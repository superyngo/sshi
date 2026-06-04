# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

## [v0.9.0] - 2026-05-06

### Added
- Full TUI interface: Phase 0 through Phase 6 (scaffolding, checkout, persistence, operate, config, sync, help popup)
- Single `sshi` binary — TUI launches when run without subcommand; `--no-tui` for headless mode
- Arrow-key navigation: Up from top escapes to NavBar, Left/Right switches tabs
- Config tab: inline edit, vec editor, entry form with group picker
- Operate tab: run/exec support, check execution, progress popup
- Sync tab: two-mode panel, checkout filter wiring
- Focus model with adaptive arrow navigation and escape-to-parent zone resolution
- Async operation bridge with cancellation token and host outcome tracking
- Log overlay ring buffer for TUI diagnostics
- CI workflow (GitHub Actions)
- ADR: SSH auth TUI popup design

### Changed
- Unified into single binary — `sshi-tui` removed, TUI is a default feature
- Navbar focus reset on running-op tab switch
- Inline edit suspends global hotkeys (q, Esc) while active

### Fixed
- Resolve all clippy warnings: dead code, collapsible conditions, unnecessary unwrap, redundant closures
- Fix build errors for clean `cargo build`
- Vec field pressing `e` now opens entry form vec editor
- Viewport visible_height uses 0 sentinel when opening vec editor
- Operate tab: Up from ApplicableEntries at scroll=0 escapes to TargetRow
- Use canonical `input_active` flag and unqualified `InputMode` in `is_editing_active`

### Docs
- TUI reconstruction plan v5 with architecture decisions
- TUI navigation and bugfix plan

## [v0.8.0] - 2026-04-29

### Added
- `--shell/-s` target filter to select hosts by detected shell type (sh, powershell, cmd)
- `--out` report output for `run`, `exec`, `check`, `sync`, and `checkout` commands (JSON and HTML)
- `default_output_format` config setting to set default report format when `--out` path has no extension
- Per-host raw output JSON in HTML reports via collapsible details
- Auto-generated report filenames now respect `default_output_format`

### Changed
- CLI short flags reassigned for consistency: `--shell/-s`, `run|exec --sudo/-S`, `sync --source/-S`
- Removed `--format` from checkout; `--out` now handles both JSON and HTML output
- Removed "Collecting" progress bar from check and sync commands for cleaner output
- Deleted temporary test scripts (test.ps1, test.sh)

### Fixed
- Raw probe output strings now use move instead of clone for efficiency
- Unified `Utc::now()` timestamp handling in check command

## [v0.7.3] - 2026-04-28

### Fixed
- musl cross-compilation builds now succeed: removed `ssh2-config` dependency (which
  unconditionally pulled in `git2 → libgit2-sys → libssh2-sys → openssl-sys`) and replaced
  it with an enhanced pure-Rust SSH config parser that supports `Host *` wildcard inheritance
- Added `Cross.toml` with `pre-build` commands as a belt-and-suspenders guard to install
  `libssl-dev` in the cross Docker containers for all four musl targets
- Reverted the incorrect v0.7.2 vendored-OpenSSL workaround (target-conditional dependencies
  do not affect build-dependency compilation inside cross containers)

### Changed
- `ssh2-config` crate removed; SSH config parsing is now handled entirely by the built-in
  pure-Rust parser in `src/config/ssh_config.rs`. `ParsedSshConfig` replaces
  `ssh2_config::SshConfig` as the shared config handle in `session_pool`

## [v0.7.2] - 2026-04-28

### Fixed
- Vendor OpenSSL for musl targets to fix CI build failures (`ssh2-config` 0.7.1 transitively requires `openssl-sys` via `git2`; musl cross-compilation containers lack system OpenSSL headers)

## [v0.7.1] - 2026-04-28

### Fixed
- SFTP probe and upload now use `sftp.create()` instead of `sftp.write()` to correctly create non-existent files (russh-sftp `write()` opens with `WRITE` flag only, no `CREATE`)
- Removed `inactivity_timeout` from russh client config; the timeout was killing idle sessions between `setup()` and subsequent `exec`/shell-detection calls, causing all shell detections to fail

## [v0.7.0] - 2026-04-27

### Added
- Embedded russh SSH transport: all SSH/SCP subprocess calls replaced with pure-Rust russh library
- Multi-alias SSH host parsing: correctly handles `Host bastion alias1 alias2` entries in `~/.ssh/config`
- SFTP-based file transfers in `sync` command (replaces external `scp`)
- ProxyJump support via russh (single-hop; config-driven)
- Windows SSH connection multiplexing via connection pool (no ControlMaster required)
- `detect_russh` shell detection using established russh sessions
- SFTP upload/download helpers with 64 MB size guard and early stat check
- Parallel SFTP probe with JoinSet and home-dir caching in session pool

### Changed
- `sshi init` migrated to `RusshSessionPool`; unknown-host-key flow matches russh error format
- `shell.rs` detect functions replaced with `detect_russh`
- `pool.rs` is now russh-only; `ConnectionManager` removed entirely
- `filter_reachable` now consistently keyed on `ssh_host` (matching `filter_sftp_capable`)
- `exec.rs` upload path uses SFTP instead of `scp` subprocess

### Fixed
- VirtualLock security warnings suppressed on Windows in non-verbose mode
- `partition_host_key_failures` matches both `"Unknown host key"` (russh) and legacy ControlMaster error strings
- SFTP download guards file size via `metadata()` before `read()` to prevent OOM

### Removed
- `connection.rs`, `executor.rs`, `process_transport.rs`, `transport.rs` legacy modules (~1,600 lines)
- `async-trait` dependency (no longer needed after transport abstraction removal)
- `socket_for` stub in `pool.rs`

## [v0.6.0] - 2026-04-14

### Added
- SshTransport trait definition with unified interface for SSH operations
- RemoteOutput struct for structured command execution results
- ProcessTransport implementation wrapping ConnectionManager with RwLock
- async-trait dependency for async trait support

### Changed
- SSH abstraction layer (Phase 1) enabling future transport backends

### Docs
- SshTransport trait abstraction design spec
- OpenSSH library migration evaluation
- russh library migration evaluation
- SSH transport trait implementation plan

### Tests
- ProcessTransport unit tests (send/sync, creation, initial state)

## [v0.5.0] - 2026-03-21

### Added
- Dual-mode ConnectionManager (Pooled/Direct) for Windows client support
- ANSI escape code support on Windows terminals via `SetConsoleMode`

### Fixed
- Shell-aware SCP probe paths for PowerShell and Cmd remote hosts
- Windows Cmd remote shell support in sync commands (metadata, batch, dir-expand)
- Defensive escaping and clippy/fmt fixes throughout

## [v0.4.0]- 2026-03-19

### Added
- SSH host key acceptance during init with interactive prompt
- SSH host resolution and keyscan helpers for batch operations
- Partition helper for host key failure handling
- Stale host detection and removal prompt during init
- Hostname display in sync summary transfer lines

### Changed
- Enhanced init workflow with host key management
- Improved sync summary with clearer host identification

### Docs
- Implementation plan for init host key acceptance
- Design spec for init host key acceptance feature
- Implementation plan for init stale hosts, summary hostnames, version verification
- Design spec for init stale host detection, summary hostnames, version verification

### CI
- Version verification step in release workflow

## [v0.3.0] - 2026-03-13

### Added
- SSH connection pooling for improved performance
- Batch metadata collection with parallel operations
- Per-host concurrency configuration
- Skip reasons tracking for sync operations
- Progress display enhancements
- ConcurrencyLimiter and pooled SSH executor functions
- SSH ConnectionManager with per-file skip on missing source
- Batched metadata command builder and parser

### Changed
- Complete sync pipeline optimization with batched collection and parallel distribution
- Rewrote check, exec, run, and init commands to use pooled executor
- Replaced inline ConnectionManager with SshPool

### Docs
- Sync pipeline optimization implementation plan
- Sync pipeline optimization design spec

## [v0.2.0] - 2026-03-11

### Added
- List command to display configured hosts and groups
- Enhanced CLI capabilities with improved command structure

### Changed
- Improved checkout command with better HTML export
- Enhanced sync command with collect-decide-distribute model refinement
- Refined app config schema for better flexibility
- Improved shell-specific probe implementations

## [v0.1.1] - 2026-03-08

### Fixed
- Corrected Windows state directory variable binding in `state_dir()` for proper platform-specific resolution.

## [v0.1.0] - 2026-03-06

### Added
- SSH-config-based host discovery and import
- Automatic shell type detection (sh, bash, zsh, PowerShell, cmd.exe)
- System snapshots (CPU, memory, disk, battery metrics)
- File synchronization with collect-decide-distribute model
- Remote command execution (`run`, `exec`)
- TUI for viewing historical data and trends
- Operation logging with SQLite state database
- Group-based host targeting
- Cross-platform GitHub release workflow

### Documentation
- Complete README with usage examples for all commands
- Configuration file examples
- Target selection reference
