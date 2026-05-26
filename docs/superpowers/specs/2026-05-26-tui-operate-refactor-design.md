# TUI Operate Refactor (Operate / View split, unified field interface)

- **Date:** 2026-05-26
- **Status:** Draft (brainstorming output, grilled against `tui_reconstruct_plan.md`;
  revised to exclude `init` after the stdin-prompt finding — see "init excluded")
- **Scope:** Requirement #4 of the sshi modernization batch. This spec covers
  the **interactive TUI** surface for launching subcommands. It depends on the
  CLI surface defined in `2026-05-26-cli-interface-unification-design.md`
  (shared `--skip`, `--dry-run`, removal of `-y/--yes`). Config-tab internals,
  command-core logic, and output formats are out of scope.

## Problem

The Operate tab today hosts only `check / run / exec / sync` and diverges from
the rest of the TUI:

- It cannot launch `list`, `log`, or `checkout` (the latter lives in a
  separate, hand-rolled Checkout tab). (`init` stays CLI-only — see "init
  excluded".)
- Its parameter widgets are bespoke (`ParamPanelField` enum, manual rendering)
  rather than the unified `FieldDescriptor`/`FieldKind` schema the Config tab
  uses — so editing an Operate param feels different from editing a Config
  field.
- The target-filter popup lacks `--skip`, which the CLI now has on every
  host-operating command.
- It still renders a `--yes` flag for `run`, which the CLI has removed.

## Goals

1. Make Operate the single launcher for the host-operating and view subcommands
   (every non-Config subcommand **except `init`**), split by result type into two
   tabs that share one UI grammar.
2. Reuse the Config tab's field interface (`FieldDescriptor`, `FieldKind`,
   `cycle_option_value`, `InputField`, vec/multi-select editors) for **every**
   operation-specific parameter.
3. Keep the **exclusive** target model and its popup picker (per
   `tui_reconstruct_plan.md` AD / §13), adding `--skip`; surface an
   always-visible summary so target/modifier values are glanceable and persist
   across operation and tab switches.

## Non-Goals

- No changes to the Config tab internals or its schema.
- No **semantic** changes to existing command behavior, **target-resolution
  semantics**, or output/report formats. (Extracting new `*_core` data-returning
  entry points for `list`/`log` is acknowledged prerequisite work — see
  "Prerequisite: View command cores".)
- No combinable target model. TUI targets stay mutually exclusive (see
  "Target model" below) — this is an intentional, documented divergence from
  the CLI's combinable `TargetArgs`.
- No editable config-path field (`-c/--config` is fixed at TUI launch).
- **No TUI `init`.** `init` stays CLI-only this cycle (see "init excluded");
  CLI `init` is untouched.

## Tab structure

| Tab | Operations | Result model |
|---|---|---|
| `1: Config` | *(unchanged)* | config editor |
| `2: Operate` | `check, run, exec, sync` | explicit Execute → per-host progress popup |
| `3: View` | `checkout, list, log` | live auto-refresh → rendered result area |

The `View` tab **absorbs and replaces** the current Checkout tab. The
`ActiveTab::Checkout` persisted variant becomes `ActiveTab::View` (with a serde
alias so existing state files do not reset — see "State & persistence").

> **Terminology:** `tui_reconstruct_plan.md` calls tab 3 the "Checkout tab".
> This spec renames it the **"View tab"** (it now also hosts `list`/`log`). The
> plan doc's §12.4 / §6.1 references are reconciled during implementation.

## Shared layout grammar (Approach B — single vertical column)

Both Operate and View use the same vertical stack inside the tab body:

```
Operation selector row              (horizontal radio: ◉/○ per op; ←/→ cycles)
Target summary line                 (read-only; f/Enter opens the popup)
┌ <op> parameters ──────────────────────────────────────────┐
│ specific field list (inline, Config field interface)       │
└────────────────────────────────────────────────────────────┘
Trigger / result area
```

- **Operation selector**: one horizontal radio row; ←/→ change the active op
  (matches `§8.4` radio/toggle priority — no new horizontal zone navigation).
- **Target summary line**: a read-only summary of the current exclusive target
  + modifiers (e.g. `Target: groups:[web,db] · skip:[h9] · serial · 30s`).
  `f`/Enter opens the **popup picker** to edit it. For `log` (which has no target
  concept) the line renders **greyed** and the popup is **not openable**
  (`f` disabled).
- **Specific-params block**: the active op's unique fields, edited inline via
  the Config field interface. Collapses to a one-line placeholder when an op
  has none (`list`).
- **Trigger / result area**:
  - *Operate:* `[ Execute <op> ⏎ ]` button + target count → per-host progress
    popup on run.
  - *View:* no execute button; the result area **auto-refreshes** whenever the
    op or a param changes.

The layout stays a vertical stack, so the documented `§8.4` navigation model
(horizontal ←/→ sealed at zone level; ←/→ only cycles the op radio) is
preserved unchanged.

## Target model (exclusive — unchanged invariant)

Targets remain **mutually exclusive**, exactly as `tui_reconstruct_plan.md`
specifies: one of `All | Groups | Hosts | Shell` at a time. This is reaffirmed,
not changed. The popup picker (`§13`; `FilterPopup` in
`src/tui/components/target_filter.rs`) stays the editing surface; the only
additions are:

- **`skip`** (`Vec<String>`) — hosts to exclude from the resolved set, edited in
  the popup via the vec editor. Currently missing from the TUI.
- `serial` (bool) and `timeout` (u64) remain popup-editable modifiers.

These are **modifiers**, independent of the exclusive mode. The Common-area
summary line reflects the current mode + modifiers and persists across op/tab
switches. Target *resolution* semantics (`build_target_mode` /
`resolve_target_names`) are unchanged apart from applying `skip` as a final
subtraction.

> **Intentional divergence:** the CLI's `TargetArgs` is combinable
> (`-g`+`-h`+`-s`+`--skip`); the TUI is exclusive. This is a deliberate UX
> simplification covered by the existing TUI architecture decision; TUI and CLI
> may therefore resolve different host sets for "equivalent" inputs.

## Field interface reuse (specific params)

A new `src/tui/tabs/operate_schema.rs` mirrors `config_schema.rs` for the
**operation-specific** fields (the common target lives in the popup, not here):

- `specific_fields(op, state) -> Vec<FieldDescriptor>`
- `apply_specific(state, op, key, val)`

Each param maps to an existing `FieldKind` (no new kinds expected; `log --action`
reuses `FieldKind::Enum`). Editing — text input, enum/bool cycling, vec editor,
Esc-to-cancel — is identical to the Config tab.

## Per-operation field map

**Common (popup-edited):** exclusive target (`all|groups|hosts|shell`) + `skip`
+ `serial` + `timeout`. Inapplicable modifiers render greyed in the popup.

| Op | Common / target | Specific params |
|---|---|---|
| **check** | target + skip + serial + timeout | `dry_run`, `out` |
| **run** | target + skip + serial + timeout | `command`*, `sudo`, `dry_run`, `out` |
| **exec** | target + skip + serial + timeout | `script`*, `sudo`, `keep`, `dry_run`, `out` |
| **sync** | target + skip + serial + timeout | `mode`(config/adhoc), `files`(adhoc), `source`, `dry_run`, `out` |
| **checkout** | target + skip (serial/timeout greyed) | `history`, `since`, `out` |
| **list** | target + skip (serial/timeout greyed) | *(none)* |
| **log** | **inert** (greyed, popup not openable) | `last`, `since`, `host`(filter)‡, `action`(enum), `errors` |

\* `command`/`script` are required text fields.
‡ `log --host` is a single substring filter on log entries, distinct from the
  common `host` target.

Notes:
- `out` (`-o/--out`) appears on `check/run/exec/sync/checkout` only; it is a
  **specific** field.
- `run --yes` is **dropped** (CLI removed it); the stale TUI `run_yes` is deleted.
- sync `source` activates the existing dormant `_sync_source_input: InputField`
  in `App` (currently underscore-prefixed and not surfaced in the param panel).

## init excluded from Operate (CLI-only)

An earlier draft kept `init` in Operate with an inert target area. Planning
surfaced a blocking finding that flipped this decision:

- **`init::run` blocks on interactive `stdin` prompts** — stale-host removal
  (`init.rs:204`, "Remove these N host(s)? [y/N]") and host-key acceptance
  (`init.rs:302`, "Add to known_hosts and retry? [y/N]"). Inside the TUI the
  terminal is in raw mode and ratatui owns the screen, so a blocking
  `stdin().read_line()` cannot work.
- Making `init` work in the TUI therefore requires a non-interactive `init_core`
  **plus** dedicated confirmation popups for both prompts — one of which
  (host-key acceptance) is **security-sensitive**. That is a feature in its own
  right, not a slot-in.

**Decision: `init` stays CLI-only this cycle.** This keeps the Operate grammar
uniform (every Operate op targets hosts) and keeps this refactor scoped. CLI
`init` is untouched.

> **Deferred:** "TUI `init`" is a separate future feature. It must carry its own
> ADR covering the interactive host-key-acceptance decision (security-sensitive,
> hard to reverse, a real trade-off) before implementation.

## Focus / navigation

Reuse the existing `FocusZone`/`FocusPath` axis framework, staying within
`§8.4`. Per-tab zones (vertical stack):

```
OpSelector → TargetSummary → SpecificPanel → Trigger(Operate only)
```

- ←/→ on `OpSelector` cycle the operation.
- ↑/↓ move between zones / within the specific field list, skipping greyed
  fields.
- `f`/Enter on `TargetSummary` opens the popup (disabled for `log`).
- Editing specific fields uses the Config model (`InputField`,
  `cycle_option_value`, Esc).
- Enter on `Trigger` executes (Operate). View has no Trigger zone — it
  auto-refreshes.

## Prerequisite: View command cores

`check`/`run`/`exec`/`sync` already expose `*_core` functions returning
structured data with a progress sink, and the TUI already has `execute_*` paths
for them. For the View ops:

- **`checkout`** already exposes `pub(crate)` data helpers
  (`fetch_latest_snapshots`, `DisplayColumns`, `extract_metric_value`,
  `metric_header`/`metric_width`, `format_relative_time`) — the current Checkout
  tab already uses them. **No new core needed**; the View tab reuses these
  directly. Only the table-rendering currently in `checkout::print_table_report`
  (stdout) is reproduced as a ratatui renderer.
- **`list`** exposes only `commands::list::run(ctx)` (prints to stdout). Extract
  `list_core(ctx) -> ListData { hosts, checks, syncs }` (structured) and make
  `run` a thin printing wrapper.
- **`log`** exposes only `commands::log::run(ctx, last, since, host, action,
  errors)` (prints to stdout). Extract `log_core(ctx, last, since, host, action,
  errors) -> Vec<LogRow>` and make `run` a thin printing wrapper.

The extracted `run()` wrappers preserve current stdout behavior exactly — a pure
plumbing refactor, no behavior change (acknowledged in Non-Goals).

## Execution & results

- **Operate ops** reuse the existing `execute_*` path unchanged: build
  `Context`, resolve targets, call `commands::{check,run,exec,sync}::*_core` with
  the progress sink → per-host **progress popup**.
- **View ops** read via the checkout helpers / new `list_core`/`log_core` and
  render returned data in the **result area**, refreshed live on op/param change.
  A minimal
  debounce and a simple "loading…" indicator are sufficient — no elaborate
  async state machine. Scrolling reuses the existing viewport handling.
- The current Checkout-tab rendering is extracted into a reusable result
  renderer for the View tab (plan-level detail).

## State & persistence

- `TargetFilterState` gains `skip: Vec<String>` (exclusive `mode` unchanged).
- `OperateState` gains per-op specific values: `log` (`last`/`since`/`host`/
  `action`/`errors`), `checkout` (`history`/`since`), plus existing
  `run`/`exec`/`sync` fields (minus `run_yes`).
- `OperationKind` is unchanged (`Check`/`Run`/`Exec`/`Sync`); View ops use a
  `ViewOperationKind` (`Checkout`/`List`/`Log`) tagged by tab.
- `ActiveTab::Checkout` → `ActiveTab::View`, with `#[serde(alias = "Checkout")]`
  so existing state files load without resetting (per the existing "never crash
  on persistence read" contract). The non-persisted `TabId::Checkout` variant
  (`src/tui/tabs/mod.rs`) is renamed to `TabId::View` as well; all match arms in
  `app.rs`/`operate_tab.rs` that dispatch on it are updated.
- CHANGELOG notes the View-tab rename and the dropped `run_yes`.

## Testing

Unit:
- `operate_schema` round-trips: `specific_fields` ↔ `apply_specific` for every
  op and field kind.
- `skip` subtraction applied to the resolved target set.
- Greyed-field traversal: ↑/↓ skip disabled fields; `f` is inert for `log`.
- Persistence: `OperateState`/`TargetFilterState` round-trip; a legacy state
  file with `active_tab = "Checkout"` loads as `View` (alias) without reset.

Manual (real binary, per bug-fix protocol):
- Launch each of the 7 ops; confirm correct field set, greying/inert behavior,
  execution vs. live-refresh, popup open/disabled state, and that target +
  modifiers persist across op **and** tab switches.

## Open items (deferred to plan)

- **Width on narrow terminals:** single-column B has no horizontal-fit risk; the
  result area for View tables should clip/scroll gracefully.
- **View renderer extraction:** decoupling Checkout-tab rendering into a shared
  renderer for `checkout`/`list`/`log`.
- **DB-unavailable handling:** `§6.1`'s db_healthy banner now applies to the
  View tab (`checkout`/`log` need the DB; `list` reads config only).
