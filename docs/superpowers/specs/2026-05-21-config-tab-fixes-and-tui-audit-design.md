# Config-Tab Fixes & TUI Audit — Design Spec

**Date:** 2026-05-21
**Status:** Draft (pending review)
**Scope:** `src/tui/` only. (Atomic save in `src/config/app.rs` was checked during review and is already implemented — no change needed there.)

---

## 1. Background

Four issues reported against the Config tab:

1. **Selection lost after edit.** After editing any config entry, the selection cursor jumps back to the first item across the section column, entry list, field column, and vec-editor cursor.
2. **Hosts / Checks / Syncs edits never persist.** Edits round-trip in memory (the TUI reflects them) but on next launch the file reverts. Only the Settings section reliably writes to disk.
3. **Three reproducible panics**, all `range end index 1 out of range for slice of length 0`:
   - `config_tab.rs:1778` — editing a Sync's `paths` field via the entry-form vec editor.
   - `config_tab.rs:2332` — editing `settings.skipped_hosts` via the direct vec editor.
   - `config_tab.rs:2332` — editing a Check's `enabled` field via the direct vec editor.
4. **Whole-TUI audit** to find other latent issues.

Root cause for items 3 is shared and visible: 14 call sites pass `items.len().max(1)` to `Viewport::set_dims`, lying about an empty vec. `visible_range()` then returns a non-empty range while the underlying slice has length 0, and the render loop's `items[vs..ve_end]` panics.

For item 2, the save plumbing exists (`save_config()` → `crate::config::app::save`) and is wired to a `pending_save` flag and the explicit `s` key. Symptom evidence (in-session UI reflects edits; on-disk file does not) suggests not every section's edit path triggers the save, or commit-vs-save is inconsistent. The chosen remediation does not depend on identifying the exact gap — it replaces the per-section trigger with autosave-on-quit + an explicit-save affordance, eliminating the gap by construction.

For item 1, the current `pending_field_restore: Option<usize>` only restores the entry-form field cursor; other selection state is reset by `ConfigTabState::reload()`.

## 2. Goals

- No render-time panic when any edited vec is empty.
- Edits to Hosts / Checks / Syncs / Settings persist to the config file with the same reliability as Settings does today.
- Cursor position in the Config tab is preserved across save + reload cycles.
- A documented audit of `src/tui/` exists, with blocker findings folded into this work and lower-severity findings recorded in a deferred backlog.

## 3. Non-Goals

- Changes to anything outside `src/tui/`.
- Refactoring unrelated to the four deliverables.
- New features.
- Fixing audit findings classified Major or Minor — those are deferred.

## 4. Architecture & Deliverables

Four deliverables on one branch:

1. **Viewport contract fix** (root-cause fix for item 3).
2. **Autosave-on-quit** (item 2).
3. **Selection snapshot** (item 1).
4. **TUI audit** (item 4) — methodology, findings doc, blocker fixes folded into this spec.

Ordering during implementation: deliverables 1 → 2 → 3 → 4. Audit runs after the first three are drafted so blocker findings can join the same PR.

## 5. Viewport contract fix

### 5.1 Files touched

- `src/tui/components/viewport.rs` — add helper, no behavior change to existing methods.
- `src/tui/tabs/config_tab.rs` — 14 call sites updated, ~4 render sites switch to helper.

### 5.2 Helper added

```rust
// src/tui/components/viewport.rs
impl Viewport {
    /// Safe-slice helper: returns the visible window of `items`, clamped to the
    /// actual slice length. Never panics, even if `set_dims` was called with a
    /// stale length.
    pub fn visible_slice<'a, T>(&self, items: &'a [T]) -> &'a [T] {
        let (start, end) = self.visible_range();
        let end = end.min(items.len());
        let start = start.min(end);
        &items[start..end]
    }
}
```

### 5.3 Caller migration

All 14 occurrences of `vp.set_dims(<expr>.len().max(1), <h>)` change to `vp.set_dims(<expr>.len(), <h>)`. Locations (from `rg -n "len\\(\\)\\.max\\(1\\)" src/tui/tabs/config_tab.rs`):

`766, 780, 1072, 1095, 1116, 1159, 1185, 1719, 1775, 2163, 2190, 2322, 2387, 2817`. (Note: `766`/`780`/`1072`/`1095` etc. occur in event handlers with `visible_height = 0` and never reach a panicking render slice, but are corrected for consistency.)

The four (possibly five) render sites that hand-slice `items[vs..ve_end]` (currently confirmed at `1778`, `2332`; suspected `1719`/`2387`/`2817` — to be verified during implementation):

```rust
// before
let (vs, ve_end) = vp.visible_range();
for (rel, item) in items[vs..ve_end].iter().enumerate() {
    let abs = vs + rel;
    ...
}

// after
let scroll_y = vp.scroll_y();
for (rel, item) in vp.visible_slice(&items).iter().enumerate() {
    let abs = scroll_y + rel;
    ...
}
```

If `Viewport` does not already expose `scroll_y` publicly, add a getter (`pub fn scroll_y(&self) -> usize { self.scroll_y }`).

### 5.4 Tests

- Unit, `viewport.rs`: `set_dims(0, 10)` → `visible_range()` returns `(0, 0)` and `selected == 0`.
- Unit, `viewport.rs`: `visible_slice(&[])` on a viewport configured with `set_dims(0, 5)` returns `&[]`.
- Smoke test for the three reproductions:
  - Build a `ConfigTabState`, open an entry form on a Sync with empty `paths`, render one frame to a test backend, assert no panic.
  - Open the direct vec editor on `settings.skipped_hosts` when empty, render one frame, assert no panic.
  - Open the direct vec editor on a Check's empty `enabled`, render one frame, assert no panic.

### 5.5 Acceptance

- All 14 `.max(1)` sites removed.
- All confirmed render sites use `visible_slice`.
- New tests pass; existing tests unaffected.
- Manual reproduction of all three crashes in section 1 no longer panics.

## 6. Autosave-on-quit

### 6.1 Behavioral contract

| Event | Action |
|---|---|
| User presses `s` in any section / entry form / vec popup | Save immediately. Existing path unchanged. Explicit-save affordance preserved. |
| User quits the app (`q`, `Ctrl-C`, terminal close) | If `config_dirty`, autosave once. No prompt. |
| User opens external editor (`E`) | If `config_dirty`, autosave first. The current `OpenEditorDirty` confirm popup is removed. |
| User switches tab (`1`/`2`/`3`) while dirty | Keep current behavior: block with `"Config save failed — fix the error before switching tabs."` (this is a save-failure guard, not a dirty-guard, and remains relevant). |
| Crash / panic | Pending edits lost. Acceptable — mitigated by §5 panic fixes. |

### 6.2 Implementation points

- `src/tui/app.rs`, in `App::run` shutdown path (after the event loop returns, before `disable_raw_mode` / terminal restore):
  ```rust
  if self.config_tab.config_dirty {
      if let Err(e) = crate::config::app::save(&self.config, self.config_path.as_deref()) {
          eprintln!("sshi: failed to save config on quit: {e}");
      } else {
          self.config_tab.config_dirty = false;
      }
  }
  ```
  This bypasses `self.save_config()` because the latter triggers `reload()` and banner state that the dying UI cannot show. Justified: the call sites and the responsibilities diverge.
- `Ctrl-C` / signal handling: confirm during implementation that signals route through the graceful shutdown path (where the autosave block runs). If `std::process::exit` is used anywhere, replace with a graceful return. (Audit item — §8.)
- `E` key handler in `app.rs:1299` — replace:
  ```rust
  } else if self.config_tab.config_dirty {
      use crate::tui::tabs::config_tab::{ConfirmAction, ConfirmState};
      self.config_tab.confirm = Some(ConfirmState {
          prompt: "Unsaved changes will be lost.".to_string(),
          action: ConfirmAction::OpenEditorDirty,
          hints: "  [y/Enter] Open editor   [Esc] Cancel",
      });
  } else {
      self.needs_editor_open = true;
  }
  ```
  with:
  ```rust
  } else {
      if self.config_tab.config_dirty {
          self.save_config();
      }
      if !self.config_tab.config_dirty {
          self.needs_editor_open = true;
      }
      // If save failed, `config_dirty` is still true and `self.error` is set;
      // skip the editor open so user can react to the error.
  }
  ```
- Delete `ConfirmAction::OpenEditorDirty` and any unreachable code that handled it.
- `crate::config::app::save` already writes atomically (`tempfile::Builder` + `persist()`, confirmed at `src/config/app.rs:117–130`). No change needed.

### 6.3 Tests

- Persistence test (free-function level): exercise the autosave logic the quit path calls. Constructing a full `App` requires a terminal backend; instead the autosave logic is extracted into a free function `flush_config_if_dirty(dirty, config, path)` that the `App::flush_dirty_config_to_disk` method delegates to 1-for-1. Tests cover the dirty→write→clear-flag path and the not-dirty no-op path against a temp-file config. The quit-path *wiring* (two lines inserting `flush_dirty_config_to_disk` into the `should_quit` branch) is reviewed by eye and covered by `cargo check`; an automated end-to-end `App::run` driver is intentionally deferred to avoid building a synthetic terminal-backend harness.
- Behavior test for `E` key: with `config_dirty = true`, simulate `E`, assert `save_config` was called and `needs_editor_open` is true.

### 6.4 Acceptance

- After any edit (Hosts / Checks / Syncs / Settings) followed by `q`, the on-disk file reflects the edit.
- After any edit followed by `E`, the external editor opens against the updated file.
- Explicit `s` still works as before.
- The dirty-confirm popup before external editor open is gone.

## 7. Selection snapshot

### 7.1 Files touched

- `src/tui/tabs/config_tab.rs` — new struct, two methods, removal of `pending_field_restore`.
- `src/tui/app.rs` — `save_config()` brackets `reload` with capture / restore.

### 7.2 New struct & methods

```rust
// src/tui/tabs/config_tab.rs
#[derive(Default, Clone)]
struct ConfigSelectionSnapshot {
    section_idx: usize,
    entry_idx: Option<usize>,
    field_idx: Option<usize>,
    vec_editor_idx: Option<usize>,
    direct_vec_idx: Option<usize>,
}

impl ConfigTabState {
    fn capture_selection(&self) -> ConfigSelectionSnapshot { ... }
    fn restore_selection(&mut self, snap: ConfigSelectionSnapshot, config: &AppConfig) {
        // For each idx, clamp against the post-reload length of the corresponding
        // collection; if out of range, fall back to (len - 1) or 0.
    }
}
```

Clamping rules in `restore_selection`:

- `section_idx` — clamped against the static section list (always valid; defensive `min(section_count - 1)`).
- `entry_idx` — clamped against `config.host.len()` / `config.check.len()` / `config.sync.len()` for the restored section. If the restored section has no entries, the field becomes `None`.
- `field_idx` — clamped against the post-reload `entry_form.fields.len()` if the form is still open; ignored otherwise.
- `vec_editor_idx` — clamped against `entry_form.vec_editor.items.len()` if a vec editor is still open in the same field; ignored otherwise.
- `direct_vec_idx` — clamped against `direct_vec_editor.items.len()` if open; ignored otherwise.

Any `idx` exceeding the new length falls back to `len - 1` (or 0 if `len == 0`).

### 7.3 Call site changes

- `src/tui/app.rs`, in `save_config()`:
  ```rust
  let snap = self.config_tab.capture_selection();
  // existing save + reload
  self.config_tab.restore_selection(snap, &self.config);
  ```
- Remove `pending_field_restore: Option<usize>` field and the two restore blocks at `app.rs:1076–1081` and `app.rs:1365–1370` — subsumed by the snapshot.
- `commit_entry_form` return type changes from `Option<usize>` to `()`. Its one call site updated.

### 7.4 Tests

- Unit: capture / restore round-trip on a constructed state.
- Clamping: capture with `entry_idx = 3`, post-reload list length 2 → restored cursor at 1.
- Empty-list edge case: post-reload length 0 → cursor at 0, no panic.

### 7.5 Acceptance

- After editing any entry and saving, the section / entry / field / vec-editor cursors remain on the item that was just edited (or the nearest valid neighbor if it was deleted).

## 8. TUI audit

### 8.1 Output

`docs/superpowers/audits/2026-05-21-tui-audit.md`, committed in the same PR.

### 8.2 Methodology

Scripted greps + targeted reads:

| Pattern | Rationale |
|---|---|
| `\.len\(\)\.max\(1\)` | Same bug class as §5. |
| `\.unwrap\(\)` / `\.expect\(` | Review each; convert when invariant not locally provable. |
| Slice / index ops (`[vs..ve]`, `config\.host\[`, etc.) | Bounds-unchecked indexing. |
| `pending_save`, `config_dirty` | Verify autosave-on-quit covers every mutation path. |
| `std::process::exit`, `panic!`, `unreachable!` | Force-quit paths that skip autosave. |
| `tokio::signal`, `ctrlc`, `SIGINT` | Confirm signals route through graceful shutdown. |
| File length (`wc -l src/tui/**/*.rs`) | Flag files >1500 LOC as "doing too much" — note only. |

Targeted reads: `app.rs`, all `tabs/*.rs`, `widgets/`, `components/`, `state/persist.rs`.

### 8.3 Finding format

```markdown
### [BLOCKER|MAJOR|MINOR] <one-line title>
**File:** `path:line`
**Symptom:** What goes wrong, when.
**Fix sketch:** One paragraph.
**Disposition:** fixed-in-this-spec | deferred-backlog
```

### 8.4 Severity rubric

- **Blocker:** Crashes the app or corrupts config. Fix in this spec.
- **Major:** Wrong behavior under realistic input, no crash. Default deferred; fold in only if the fix is < ~20 LOC and touches a file already in this spec's diff.
- **Minor:** Style, dead code, hint typos. Always deferred.

### 8.5 Acceptance

- Audit doc exists at the path above.
- Every blocker has a corresponding test / fix in this spec.
- Major / minor findings appear in a "Deferred Backlog" section of the audit doc.

## 9. Testing summary

- Unit tests added under `src/tui/components/viewport.rs` and `src/tui/tabs/config_tab.rs`.
- Smoke / render tests for the three panic reproductions.
- Integration test for autosave-on-quit (build `App`, mark dirty, shutdown, read file).
- Existing tests must continue to pass.

## 10. Risks & open questions

- **Signal handling.** The exact signal-routing behavior is unverified; the audit (§8) confirms it. If `Ctrl-C` currently bypasses graceful shutdown, the fix is in scope; it is not pre-specified here.
- **`scroll_y` getter on `Viewport`.** Spec assumes it can be added if missing; trivial.
- **Hidden render sites.** The five candidate render sites (1719, 1778, 2332, 2387, 2817) are confirmed from grep; if implementation finds additional render paths that hand-slice without `visible_slice`, they are converted in scope.

## 11. Out-of-scope follow-ups

- Refactoring `config_tab.rs` (currently large — exact LOC noted by audit).
- Major / minor audit findings.
- Any persistence redesign beyond autosave-on-quit + explicit save.
