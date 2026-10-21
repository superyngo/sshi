# TUI mouse support + UX assessment (2026-10-21)
Status: In progress

Assessment run; no code changed. Evidence: real binary (`target/debug/sshi`, v1.7.0) driven in a pty
by `docs/tmp/claude-scratch/2026-10-21-ux-probe/probe.py` (scratch, gitignored) at 100×30 and 80×24,
plus source reading. Line numbers below are as of commit `4552405`. Findings are tracked in
[`../plan/BACKLOG.md`](../plan/BACKLOG.md) as B78–B83. Phase A (B78–B81) and B (B82) landed 2026-10-21.

## 1. Mouse support

### Current state
- Zero mouse handling. `App::handle_event` (src/tui/app.rs:1798-1803) only matches `Resize` and `Key`.
- `TerminalGuard::install` (src/tui/terminal.rs:17-28) never enables mouse capture; panic hook and
  `do_open_editor` pause/resume (app.rs:3618-3628) would all need matching enable/disable.
- Prior decision: spec 2026-05-06-tui-reconstruct §22 item 11 deferred mouse as opt-in `[tui].mouse = true`;
  §23 rejected default-on. No `[tui]` config section exists today.

### Architecture gaps
1. No frame hit map. Geometry is computed inline in render (e.g. View chrome magic numbers app.rs:3108-3127,
   88 `centered_rect`/viewport sites in config_tab.rs). Mouse handlers recomputing it would drift.
2. ~13 overlays are scattered `Option`/`bool` fields (auth, export, member picker, help, info, log overlay,
   results, running op, entry form + nested vec editor/group picker, confirm, direct vec editor, direct group
   picker, inline edit). Priority order exists only in `handle_popup_key` (app.rs:1870-1965); mouse routing
   would have to mirror it by hand.

### Phased plan (self-written, no new crate; HitMap ≈ 40 lines per skill contract C2)
| Phase | Scope | Rough size |
|---|---|---|
| M0 infra | Enable/disable capture in guard, panic hook, editor pause/resume; per-frame `HitMap<Target>` filled in paint order; `handle_mouse` routed through the same layer order; on/off switch | 1–1.5 d |
| M1 | Tab bar click; wheel scroll for region under pointer (help/info/log/results/progress/picker/checkout/list/log/sidebar/field table); click-to-select list rows | ~1 d |
| M2 | Operate/View: click radio options (op, target mode, Show), checkboxes, fields (focus + activate), Execute button, picker items | 1.5–2 d |
| M3 | Config: sidebar select, section header collapse, field row select, double-click edit, form rows; Confirm gets real `[Yes] [No]` buttons | ~2 d |
| Skip | Drag dividers, hover, right-click menus — low value for this app | — |

Tests: TestBackend render → click at centre of `hit_map.rect_of(target)`; one pty run with SGR sequences.

### Main tradeoff
Mouse capture disables native terminal text selection (Shift-drag / Option-drag still works in most
terminals). For an ops tool where users copy hostnames, errors, output this matters. Mitigation: runtime
toggle key + config default, and ship `y` yank (spec §22 item 12) alongside.

## 2. UX findings

### Defects (each needs its own Bug-Fix Protocol report before fixing)
- **D1 Operate has no scrolling; inputs collapse at 80×24.** Body inner height is 14 rows; check+Groups needs
  15, cp needs ~19. Ratatui squeezes the 3-row `Field` boxes to a bare border: Output / Local path / Remote /
  Script fields become invisible (`operate-cp-80x24` dump). Source: operate_tab.rs:360-369 (all `Length`,
  no viewport).
- **D2 Confirm popup clips its key hints.** `centered_rect(50, 20, area)` (config_tab.rs:1895): at 30 rows
  the box is 5 rows, the `[y/Enter] Yes [n/Esc] No` line is cut; at 24 rows even less. User sees
  "Delete host …?" with no keys.
- **D3 Status-bar hints overflow 80 cols.** Operate hint ≈ 93 cols, Config ≈ 88, View ≈ 84 (app.rs:4154-4164);
  `?:Help q:Quit` — the escape hatches — are what gets cut.
- **D4 Help text stale/wrong.** Says `Tab` cycles tabs (it cycles fields on Operate/View); Operate zones named
  `OpRadio → ParamPanel → TargetRow → Execute` (no longer matches layout); wrapped lines lose indent.

### Structural issues
- **U1 Focus model.** Extra NavBar focus layer + Esc cycles focus NavBar→TopField→Content→NavBar. Esc also
  clears errors and closes popups — overloaded; `1/2/3` already switch tabs so the NavBar layer adds a step.
  `Tab` means four different things (tabs / zones / OpLayer / popup sections).
- **U2 Key overloading across tabs.** `e` = edit (Config) / **execute** (Operate, from any field) / jump to config
  (View List); `d` = delete entry / dry-run; `s` = save / serial. Config's `e` muscle memory can launch a run
  on all hosts. No confirmation before broad non-dry-run run/exec (spec §22 item 8).
- **U3 Hints are static per tab**, not per focus/overlay (Confirm popup shows `d:DelEntry`, picker shows Operate keys).
- **U4 Percent-based popups.** Picker 60%×60% mostly empty, title truncated ("Es"); add-entry form large empty
  body; info popup narrow and heavily wrapped. `content_aware_popup_rect` exists but is not used everywhere.
- **U5 Checkout shows `✗ offline` + `never` for hosts never checked** — unknown rendered as failure.
- **U6 Empty values inconsistent in Config:** `proxy_jump =` / `default_output_format =` blank vs `(none)` elsewhere.
- **U7 Chrome cost:** tab bar 3 rows for 1 line, status 2, Execute block 3 → 8 of 24 rows (33%) on Operate.
- **U8 Breadcrumb prints full absolute config path**, truncated; use `~` + middle ellipsis.
- **U9 Destructive confirm defaults Enter = Yes.**
- Missing affordances already in spec §22: `/` filter, toasts, yank, confirm-before-broad-op.

## 3. Recommended order
1. **A — Defects D1–D4** (small, independent; each via Bug-Fix Protocol).
2. **B — Structural UX** (spec first): unified overlay stack, shared geometry fns (single source for render
   and input), context hints with priority truncation, content-aware popups, 1-row tab bar, key map cleanup.
3. **C — Mouse M0–M3**, built on B's overlay stack + geometry so hit-testing cannot drift.

B before C because mouse needs exactly the two things B creates; doing C first means building the hit map on
top of scattered state and inline magic numbers, then redoing it.

## Decisions (user, 2026-10-21)
- Mouse capture is **always on** — no config switch, no toggle key. This reverses spec
  2026-05-06-tui-reconstruct §22 item 11 (opt-in) and the §23 rejection of default-on. Native
  text selection remains available via the terminal's modifier-drag (Shift / Option); ship `y`
  yank with the mouse work to compensate.
- Scope: **A + B + C**, in that order.
- Key-map breaking changes: **accepted, as far as useful** (`e` on Operate, Esc/NavBar model,
  per-tab key overloads). Record each change in `tui.md` and the changelog.
