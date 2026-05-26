# TUI Operate Refactor (Operate / View split, unified field interface)

- **Date:** 2026-05-26
- **Status:** Draft (brainstorming output, grilled against `tui_reconstruct_plan.md`)
- **Scope:** Requirement #4 of the sshi modernization batch. This spec covers
  the **interactive TUI** surface for launching subcommands. It depends on the
  CLI surface defined in `2026-05-26-cli-interface-unification-design.md`
  (shared `--skip`, `--dry-run`, removal of `-y/--yes`). Config-tab internals,
  command-core logic, and output formats are out of scope.

## Problem

The Operate tab today hosts only `check / run / exec / sync` and diverges from
the rest of the TUI:

- It cannot launch `init`, `list`, `log`, or `checkout` (the latter lives in a
  separate, hand-rolled Checkout tab).
- Its parameter widgets are bespoke (`ParamPanelField` enum, manual rendering)
  rather than the unified `FieldDescriptor`/`FieldKind` schema the Config tab
  uses — so editing an Operate param feels different from editing a Config
  field.
- The target-filter popup lacks `--skip`, which the CLI now has on every
  host-operating command.
- It still renders a `--yes` flag for `run`, which the CLI has removed.

## Goals

1. Make Operate the single launcher for **all non-Config subcommands**, split
   by result type into two tabs that share one UI grammar.
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
  entry points for `checkout`/`list`/`log` is acknowledged prerequisite work —
  see "Prerequisite: View command cores".)
- No combinable target model. TUI targets stay mutually exclusive (see
  "Target model" below) — this is an intentional, documented divergence from
  the CLI's combinable `TargetArgs`.
- No editable config-path field (`-c/--config` is fixed at TUI launch).

## Tab structure

| Tab | Operations | Result model |
|---|---|---|
| `1: Config` | *(unchanged)* | config editor |
| `2: Operate` | `check, run, exec, sync, init` | explicit Execute → per-host progress popup |
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
  `f`/Enter opens the **popup picker** to edit it. For ops where targeting is
  inert (`init`, `log`) the line renders **greyed** and the popup is **not
  openable** (`f` disabled).
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
| **init** | **inert** (greyed, popup not openable) | `update`, `dry_run`, `skip`†, `timeout`† |
| **checkout** | target + skip (serial/timeout greyed) | `history`, `since`, `out` |
| **list** | target + skip (serial/timeout greyed) | *(none)* |
| **log** | **inert** (greyed, popup not openable) | `last`, `since`, `host`(filter)‡, `action`(enum), `errors` |

\* `command`/`script` are required text fields.
† `init`'s `skip`/`timeout` are **specific** fields, independent of the common
  popup — different host universe (ssh-config import vs. configured hosts) and
  intent (skip-import vs. skip-target). Not shared.
‡ `log --host` is a single substring filter on log entries, distinct from the
  common `host` target.

Notes:
- `out` (`-o/--out`) appears on `check/run/exec/sync/checkout` only; it is a
  **specific** field.
- `run --yes` is **dropped** (CLI removed it); the stale TUI `run_yes` is deleted.
- sync `source` activates the existing dormant `_sync_source_input: InputField`
  in `App` (currently underscore-prefixed and not surfaced in the param panel).

## init in Operate — deliberate grammar break (documented trade-off)

`init` is kept in Operate per the chosen scope, but it shares **none** of the
common target model: it imports from `~/.ssh/config` and always acts on that
universe. To make the break intentional rather than accidental:

- **Keep-in (chosen):** `init` lives in Operate for one launcher surface.
  Its target/common summary is **inert** — greyed line, `f` keybind disabled,
  popup not openable — and its `update/dry_run/skip/timeout` are specific fields.
  Execution reuses the per-host progress popup (init connects per-host for shell
  detection).
- **Alternative (rejected): CLI-only.** Leaving `init` out of the TUI would keep
  the Operate grammar uniform (every op targets hosts), at the cost of a missing
  launcher for a command users do run interactively after first setup.

The keep-in choice accepts a localized grammar break (inert common area) in
exchange for completeness. This is intentional and isolated to `init`/`log`.

## Focus / navigation

Reuse the existing `FocusZone`/`FocusPath` axis framework, staying within
`§8.4`. Per-tab zones (vertical stack):

```
OpSelector → TargetSummary → SpecificPanel → Trigger(Operate only)
```

- ←/→ on `OpSelector` cycle the operation.
- ↑/↓ move between zones / within the specific field list, skipping greyed
  fields.
- `f`/Enter on `TargetSummary` opens the popup (disabled for `init`/`log`).
- Editing specific fields uses the Config model (`InputField`,
  `cycle_option_value`, Esc).
- Enter on `Trigger` executes (Operate). View has no Trigger zone — it
  auto-refreshes.

## Prerequisite: command cores for init + view ops

`check`/`run`/`exec`/`sync` already expose `*_core` functions returning
structured data with a progress sink, and the TUI already has `execute_*` paths
for them. Two gaps must be closed first:

- **`init`** exposes only `commands::init::run(ctx, update, dry_run, skip)` — no
  `init_core` and no TUI `execute_init`. Needs an `init_core` (per-host progress
  sink, structured return) plus a new `execute_init` wiring it to the progress
  popup.
- **View ops** expose `commands::list::run(ctx)`, `commands::log::run(ctx, last,
  since, host, action, errors)`, and `commands::checkout::run(...)`, each
  printing to stdout via the printer. The TUI result area needs **structured
  return values**, not stdout — so extract `checkout_core`/`list_core`/`log_core`
  returning data with no printer side effects.

In all cases the existing `run()` functions become thin wrappers that call the
new core and print. This is a pure refactor of plumbing, not a behavior change
(acknowledged in Non-Goals).

## Execution & results

- **Operate ops** reuse the existing `execute_*` path (new `execute_init` for
  init): build `Context`, resolve targets, call
  `commands::{check,run,exec,sync,init}::*_core` with the progress sink →
  per-host **progress popup**.
- **View ops** call the new `commands::{checkout,list,log}::*_core` and render
  returned data in the **result area**, refreshed live on op/param change. A minimal
  debounce and a simple "loading…" indicator are sufficient — no elaborate
  async state machine. Scrolling reuses the existing viewport handling.
- The current Checkout-tab rendering is extracted into a reusable result
  renderer for the View tab (plan-level detail).

## State & persistence

- `TargetFilterState` gains `skip: Vec<String>` (exclusive `mode` unchanged).
- `OperateState` gains per-op specific values: `init` (`update`/`dry_run`/`skip`/
  `timeout`), `log` (`last`/`since`/`host`/`action`/`errors`), `checkout`
  (`history`/`since`), plus existing `run`/`exec`/`sync` fields (minus `run_yes`).
- `OperationKind` extends with `Init`; View ops use a `ViewOperationKind`
  (`Checkout`/`List`/`Log`) tagged by tab.
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
- Greyed-field traversal: ↑/↓ skip disabled fields; `f` is inert for
  `init`/`log`.
- Persistence: `OperateState`/`TargetFilterState` round-trip; a legacy state
  file with `active_tab = "Checkout"` loads as `View` (alias) without reset.

Manual (real binary, per bug-fix protocol):
- Launch each of the 8 ops; confirm correct field set, greying/inert behavior,
  execution vs. live-refresh, popup open/disabled state, and that target +
  modifiers persist across op **and** tab switches.

## Open items (deferred to plan)

- **Width on narrow terminals:** single-column B has no horizontal-fit risk; the
  result area for View tables should clip/scroll gracefully.
- **View renderer extraction:** decoupling Checkout-tab rendering into a shared
  renderer for `checkout`/`list`/`log`.
- **DB-unavailable handling:** `§6.1`'s db_healthy banner now applies to the
  View tab (`checkout`/`log` need the DB; `list` reads config only).
