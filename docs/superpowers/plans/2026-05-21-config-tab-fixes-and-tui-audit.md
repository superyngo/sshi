# Config-Tab Fixes & TUI Audit — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix three reproducible Config-tab panics, unify config persistence via autosave-on-quit, preserve selection state across save/reload, and produce a TUI audit folding blocker findings into this PR.

**Architecture:** Root-cause-fix the panic class by introducing a `Viewport::visible_slice` helper and scrubbing all 14 `len().max(1)` lie sites in `config_tab.rs`. Promote save from per-keypress `pending_save` to a lifecycle event triggered on app quit and before external editor open. Replace ad-hoc `pending_field_restore` with a unified `ConfigSelectionSnapshot` captured/restored around save+reload. Then run a scripted audit of `src/tui/`, fold blocker findings, defer the rest.

**Tech Stack:** Rust, ratatui 0.29 (`tui` feature, default on), tokio, crossterm. Tests via `cargo test --features tui` (default). Smoke render tests use `ratatui::backend::TestBackend`.

**Spec reference:** `docs/superpowers/specs/2026-05-21-config-tab-fixes-and-tui-audit-design.md`.

**Branch model:** Single feature branch off `main`. Commit per task. Final commit appends `CHANGELOG.md` Unreleased entry per project rules.

---

## File Map

| File | Action | Responsibility |
|---|---|---|
| `src/tui/components/viewport.rs` | Modify | Add `visible_slice` helper + tests |
| `src/tui/tabs/config_tab.rs` | Modify | Scrub `len().max(1)` sites, switch render sites to `visible_slice`, add `ConfigSelectionSnapshot` + capture/restore methods, remove `pending_field_restore` field, change `commit_entry_form` return type, add panic-reproduction smoke tests |
| `src/tui/app.rs` | Modify | Add `flush_dirty_config_to_disk()` helper + call it on quit; rewrite `E` key handler to autosave instead of confirm popup; bracket `save_config()` reload with snapshot capture/restore; delete `pending_field_restore` restore blocks |
| `docs/superpowers/audits/2026-05-21-tui-audit.md` | Create | Audit findings doc |
| `CHANGELOG.md` | Modify | Single Unreleased entry summarizing this work |

`ConfirmAction::OpenEditorDirty` (in `config_tab.rs:324`) is removed; no other file references it (verified by grep in Task 6).

---

## Task 1: Add `Viewport::visible_slice` helper + tests

**Files:**
- Modify: `src/tui/components/viewport.rs` — insert helper after the existing `visible_range` method; add unit tests inside the existing `#[cfg(test)] mod tests`.

- [ ] **Step 1.1: Write the failing tests**

Edit `src/tui/components/viewport.rs`, inside the existing `mod tests` block (after the existing `empty_list_is_safe` test, before the closing `}` on line 145), append:

```rust
    #[test]
    fn visible_slice_returns_empty_for_empty_list() {
        let mut v = Viewport::new();
        v.set_dims(0, 5);
        let items: Vec<i32> = vec![];
        assert!(v.visible_slice(&items).is_empty());
    }

    #[test]
    fn visible_slice_clamps_when_caller_lies_about_length() {
        let mut v = Viewport::new();
        // Caller lies: claims 10 items, but the real slice has 0.
        v.set_dims(10, 5);
        let items: Vec<i32> = vec![];
        assert!(v.visible_slice(&items).is_empty());
    }

    #[test]
    fn visible_slice_returns_window_when_caller_truthful() {
        let mut v = Viewport::new();
        v.set_dims(10, 3);
        let items: Vec<i32> = (0..10).collect();
        let slice = v.visible_slice(&items);
        assert_eq!(slice, &[0, 1, 2]);
    }
```

- [ ] **Step 1.2: Run tests to verify they fail**

Run: `cargo test --features tui -p sshi --lib tui::components::viewport -- --nocapture`

Expected: three new tests fail to compile with `error[E0599]: no method named visible_slice found`.

- [ ] **Step 1.3: Add the helper**

Edit `src/tui/components/viewport.rs`. Insert after the existing `visible_range` method (after line 96, before the `at_top` method at line 98):

```rust
    /// Safe-slice helper: returns the visible window of `items`, clamped to the
    /// actual slice length. Never panics, even if `set_dims` was called with a
    /// stale length.
    pub fn visible_slice<'a, T>(&self, items: &'a [T]) -> &'a [T] {
        let (start, end) = self.visible_range();
        let end = end.min(items.len());
        let start = start.min(end);
        &items[start..end]
    }
```

- [ ] **Step 1.4: Run tests to verify they pass**

Run: `cargo test --features tui -p sshi --lib tui::components::viewport`

Expected: all viewport tests pass (the three new ones plus the three existing ones).

- [ ] **Step 1.5: Commit**

```bash
git add src/tui/components/viewport.rs
git commit -m "feat(tui): add Viewport::visible_slice helper

Safe-slice wrapper around visible_range that clamps against the
actual slice length. Lays groundwork for replacing fragile
items[vs..ve_end] sites in config_tab.rs."
```

---

## Task 2: Scrub all 14 `len().max(1)` sites in `config_tab.rs`

**Files:**
- Modify: `src/tui/tabs/config_tab.rs` — 14 occurrences across event handlers and render paths.

These are not render-crash fixes on their own (most are in event handlers with `visible_height = 0`). They remove the lie that makes a future render-site crash possible.

- [ ] **Step 2.1: Verify the 14 site list before editing**

Run: `rg -n "len\(\)\.max\(1\)" src/tui/tabs/config_tab.rs`

Expected output (line numbers must match these exactly; if they don't, re-locate before editing):
```
766:                                    vp.set_dims(available.len().max(1), 0);
780:                                    vp.set_dims(items.len().max(1), 0);
1072:                                vp.set_dims(available.len().max(1), 0);
1095:                                    vp.set_dims(available.len().max(1), 0);
1116:                                    ve.vp.set_dims(ve.items.len().max(1), 0);
1159:                    ve.vp.set_dims(ve.items.len().max(1), 0);
1185:                    ve.vp.set_dims(ve.items.len().max(1), 0);
1719:            gp_vp.set_dims(gp.available.len().max(1), gp_visible_h);
1775:            ve_vp.set_dims(ve.items.len().max(1), ve_visible_h);
2163:                    ve.vp.set_dims(ve.items.len().max(1), 0);
2190:                    ve.vp.set_dims(ve.items.len().max(1), 0);
2322:        vp.set_dims(dve.items.len().max(1), visible_h.saturating_sub(3));
2387:            dgp.available.len().max(1),
2817:        vp.set_dims(available.len().max(1), 0);
```

If line numbers differ, locate by grep before each edit; do not edit blindly.

- [ ] **Step 2.2: Apply the same replacement at every site**

For each of the 14 sites, replace `.len().max(1)` with `.len()`. Use 14 separate `Edit` operations (the surrounding context differs between sites; `replace_all` would also work since the substring is the same — prefer `replace_all` on `config_tab.rs` with `old_string: ".len().max(1)"` and `new_string: ".len()"`).

Run after the bulk edit: `rg -n "len\(\)\.max\(1\)" src/tui/tabs/config_tab.rs`

Expected output: empty (zero matches).

- [ ] **Step 2.3: Run `cargo check` to confirm no compile breakage**

Run: `cargo check --features tui -p sshi`

Expected: clean build (warnings allowed, no errors).

- [ ] **Step 2.4: Run the existing test suite to confirm no regression**

Run: `cargo test --features tui -p sshi --lib`

Expected: all existing tests pass.

- [ ] **Step 2.5: Commit**

```bash
git add src/tui/tabs/config_tab.rs
git commit -m "fix(tui): stop lying about item_count in Viewport::set_dims

The 14 .len().max(1) callers in config_tab.rs claimed a non-zero
item_count for empty vecs. Combined with the slice-based render
loop, this panics at items[vs..ve_end] when the vec is empty.
Pass real lengths; Viewport already handles zero correctly."
```

---

## Task 3: Migrate render sites to `visible_slice`

**Files:**
- Modify: `src/tui/tabs/config_tab.rs` — render-loop sites that hand-slice `items[vs..ve_end]`.

Candidate render sites from the spec: `1719`, `1778`, `2332`, `2387`, `2817`. Each is verified individually below; some may not hand-slice (group-picker render sites in particular may iterate differently).

- [ ] **Step 3.1: Identify exact hand-slice render sites**

Run: `rg -nP '\.iter\(\)\.enumerate\(\)' src/tui/tabs/config_tab.rs | head -20`

Then for each render site listed below, confirm the loop pattern before editing. The two confirmed-panicking sites are at `~1778` (entry-form vec editor) and `~2332` (direct vec editor). Run:

```sh
rg -nB1 -A1 'items\[vs\.\.ve_end\]\.iter\(\)\.enumerate\(\)' src/tui/tabs/config_tab.rs
```

Expected: at least two matches. Note their line numbers; the line numbers have shifted slightly from Task 2 (no shift expected since Task 2 was character-level replacement, but verify).

- [ ] **Step 3.2: Rewrite entry-form vec editor render (around line 1778)**

Locate the block matching:

```rust
            let ve_visible_h = visible_h.saturating_sub(6);
            let mut ve_vp = ve.vp.clone();
            ve_vp.set_dims(ve.items.len(), ve_visible_h);
            let (vs, ve_end) = ve_vp.visible_range();

            for (rel, item) in ve.items[vs..ve_end].iter().enumerate() {
                let abs = vs + rel;
                let is_sel = abs == ve_vp.selected;
```

Replace the four lines starting at `let (vs, ve_end)` with:

```rust
            let scroll_y = ve_vp.scroll_y;
            for (rel, item) in ve_vp.visible_slice(&ve.items).iter().enumerate() {
                let abs = scroll_y + rel;
                let is_sel = abs == ve_vp.selected;
```

(The old `let (vs, ve_end) = ve_vp.visible_range();` line is removed. The replacement preserves everything else in the loop body.)

- [ ] **Step 3.3: Rewrite direct vec editor render (around line 2332)**

Locate the block matching:

```rust
        let mut vp = dve.vp.clone();
        vp.set_dims(dve.items.len(), visible_h.saturating_sub(3));
        let (vs, ve_end) = vp.visible_range();

        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(
                "  (a:add  d:del  s:save  Esc:cancel)",
                Style::default().fg(theme.warning),
            )),
            Line::from(""),
        ];
        for (rel, item) in dve.items[vs..ve_end].iter().enumerate() {
            let abs = vs + rel;
            let is_sel = abs == vp.selected;
```

Replace:
- Remove the `let (vs, ve_end) = vp.visible_range();` line.
- Change the `for` line to `for (rel, item) in vp.visible_slice(&dve.items).iter().enumerate() {`.
- Change `let abs = vs + rel;` to `let abs = vp.scroll_y + rel;`.

- [ ] **Step 3.4: Check the remaining candidate sites (1719, 2387, 2817)**

For each of lines `~1719` (group-picker render), `~2387` (direct group-picker render), `~2817` (other group-picker render), Read the surrounding 20 lines and check whether the render loop uses `items[vs..ve_end]` or some other pattern (e.g. iterating `available` directly, or using `vp.visible_range()` differently).

If the loop matches the unsafe `slice[vs..ve_end].iter().enumerate()` pattern, rewrite it using the same scheme as Step 3.2 (use `vp.visible_slice(&slice)` and `vp.scroll_y` for absolute index). If not, leave the call site as-is (Task 2 already removed the `.max(1)` lie; the loop pattern is already safe).

Document in the commit message which sites were rewritten and which were left because they did not match the unsafe pattern.

- [ ] **Step 3.5: Run `cargo check`**

Run: `cargo check --features tui -p sshi`

Expected: clean build.

- [ ] **Step 3.6: Run the existing test suite**

Run: `cargo test --features tui -p sshi --lib`

Expected: all existing tests still pass.

- [ ] **Step 3.7: Commit**

```bash
git add src/tui/tabs/config_tab.rs
git commit -m "fix(tui): use Viewport::visible_slice in panic-prone render sites

Replaces items[vs..ve_end] hand-slicing with the new safe helper.
Eliminates the three reported render-time panics at the entry-form
vec editor (Sync paths) and direct vec editor (settings.skipped_hosts,
Check.enabled). Commit body documents which candidate sites were
rewritten vs. already safe."
```

---

## Task 4: Smoke tests for the three panic reproductions

**Files:**
- Modify: `src/tui/tabs/config_tab.rs` — append three tests to the existing `#[cfg(test)] mod tests` block.

These tests construct minimal state, open the affected editor with an empty vec, and render one frame to a `TestBackend`. Any panic from `items[vs..ve_end]` fails the test.

- [ ] **Step 4.1: Write the smoke-test scaffold first**

Edit `src/tui/tabs/config_tab.rs`, inside the existing `mod tests` block (right before its closing `}`), add the imports and a helper at the top of the module body:

```rust
    use crate::config::types::AppConfig;
    use crate::tui::theme::Theme;
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    fn render_once(state: &mut ConfigTabState, config: &AppConfig) {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test backend");
        let theme = Theme::default();
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 120, 40);
                state.render(area, f, &theme, config, None, false);
            })
            .expect("render must not panic");
    }
```

If `Theme::default()` does not exist, replace it with whichever constructor the codebase uses (check via `rg -n 'impl.*Theme|Theme::new' src/tui/`); if a constructor needs arguments, use a stub. The `render_once` helper is the single contract: rendering must not panic on the constructed state.

If `crate::config::types::AppConfig` is wrong (verify with `rg -nP 'pub (struct|enum) AppConfig\b' src/`), substitute the correct path. Same for `Theme`.

- [ ] **Step 4.2: Write the three failing tests**

Append (still inside `mod tests`):

```rust
    #[test]
    fn render_does_not_panic_on_empty_sync_paths_vec_editor() {
        // Reproduces config_tab.rs:1778 panic: editing a Sync's `paths`
        // field via the entry-form vec editor when paths is empty.
        let mut config = AppConfig::default();
        config.sync.push(crate::config::types::SyncEntry {
            name: Some("test".to_string()),
            id: "sync-test".to_string(),
            paths: vec![],
            groups: vec![],
            enable_hosts: true,
            enable_all: true,
            recursive: false,
            mode: None,
            propagate_deletes: None,
            source: None,
        });
        let mut state = ConfigTabState::new(&config, None);
        // Open the Sync entry form on index 0, navigate to the `paths` field,
        // and open the vec editor with an empty items list.
        let form = EntryFormState::new_sync(&config.sync[0]);
        state.entry_form = Some(form);
        if let Some(form) = state.entry_form.as_mut() {
            // Find the "paths" field index.
            let paths_field_idx = form
                .fields
                .iter()
                .position(|f| f.key == "paths")
                .expect("paths field must exist");
            form.field_vp.selected = paths_field_idx;
            form.vec_editor = Some(VecEditorState {
                field_index: paths_field_idx,
                items: vec![],
                vp: Viewport::new(),
                input_active: false,
                input: String::new(),
            });
        }
        render_once(&mut state, &config);
    }

    #[test]
    fn render_does_not_panic_on_empty_skipped_hosts_direct_editor() {
        // Reproduces config_tab.rs:2332 panic: editing settings.skipped_hosts
        // via the direct vec editor when it is empty.
        let mut config = AppConfig::default();
        config.settings.skipped_hosts = vec![];
        let mut state = ConfigTabState::new(&config, None);
        state.direct_vec_editor = Some(DirectVecEditorState {
            field_key: "skipped_hosts".to_string(),
            items: vec![],
            vp: Viewport::new(),
            input_active: false,
            input: String::new(),
        });
        render_once(&mut state, &config);
    }

    #[test]
    fn render_does_not_panic_on_empty_check_enabled_direct_editor() {
        // Reproduces config_tab.rs:2332 panic: editing a Check's `enabled`
        // via the direct vec editor when it is empty.
        let mut config = AppConfig::default();
        config.check.push(crate::config::types::CheckEntry {
            name: Some("test".to_string()),
            id: "check-test".to_string(),
            enabled: vec![],
            path: vec![],
            groups: vec![],
            enable_hosts: true,
            enable_all: true,
        });
        let mut state = ConfigTabState::new(&config, None);
        state.direct_vec_editor = Some(DirectVecEditorState {
            field_key: "enabled".to_string(),
            items: vec![],
            vp: Viewport::new(),
            input_active: false,
            input: String::new(),
        });
        render_once(&mut state, &config);
    }
```

**Important — verify field signatures before running.** The exact field names and types of `VecEditorState`, `DirectVecEditorState`, `SyncEntry`, `CheckEntry`, and `AppConfig::default()` must match what's currently in the codebase. Before writing each test, run:

```sh
rg -nA8 'pub struct VecEditorState|pub struct DirectVecEditorState' src/tui/tabs/config_tab.rs
rg -nA12 'pub struct SyncEntry|pub struct CheckEntry|pub struct AppConfig' src/config/types.rs
```

Adjust the literal field lists in the test code to match. If `AppConfig` does not implement `Default`, replace `AppConfig::default()` with whichever minimal constructor exists (e.g. `AppConfig::empty()` or build by literal).

- [ ] **Step 4.3: Run tests — expect first run BEFORE Tasks 2-3 fixes would have panicked; now (post-fix) they should pass**

Since Tasks 2 and 3 have already landed, these tests should pass on first run. They serve as regression coverage.

Run: `cargo test --features tui -p sshi --lib tui::tabs::config_tab::tests::render_does_not_panic -- --nocapture`

Expected: all three new tests pass. If any panics with `range end index 1 out of range`, Tasks 2–3 missed a site; re-locate via the backtrace and fix.

- [ ] **Step 4.4: Sanity — confirm tests would have caught the bug**

Optional but recommended: `git stash` the Task 2 and Task 3 changes, run the new tests, confirm they panic; then `git stash pop`. Skip this step if it complicates the workflow; the spec already documents the root cause.

- [ ] **Step 4.5: Commit**

```bash
git add src/tui/tabs/config_tab.rs
git commit -m "test(tui): regression smoke tests for the three reported render panics

Render-one-frame tests reproducing the panic conditions from the
bug report: empty Sync.paths in entry-form vec editor, empty
settings.skipped_hosts in direct vec editor, empty Check.enabled
in direct vec editor."
```

---

## Task 5: Autosave-on-quit via `flush_dirty_config_to_disk` helper

**Files:**
- Modify: `src/tui/app.rs` — add helper, call from quit path in `App::run`.

The autosave block goes inside the quit path (`if self.should_quit { ... break; }` at `app.rs:356`) and explicitly bypasses `self.save_config()` because the latter triggers a reload/banner that the dying UI cannot show.

- [ ] **Step 5.1: Add the testable free function + thin method wrapper**

Edit `src/tui/app.rs`. Find an appropriate spot — adjacent to `save_config()` at `app.rs:1762`. Insert immediately after the closing brace of `save_config()`:

```rust
    /// Best-effort flush of dirty config to disk during shutdown.
    ///
    /// Unlike `save_config()`, this does NOT trigger reload, banner state, or
    /// selection-snapshot bookkeeping — the UI is tearing down. Delegates to
    /// the free function `flush_config_if_dirty` so the persistence logic
    /// is unit-testable without constructing a full `App`.
    fn flush_dirty_config_to_disk(&mut self) {
        flush_config_if_dirty(
            &mut self.config_tab.config_dirty,
            &self.config,
            self.config_path.as_deref(),
        );
    }
```

Then add the free function near the top of `app.rs` (module-private), just below the `use` block:

```rust
/// Persist `config` to `path` if `dirty` is set; clear `dirty` on success.
/// On failure prints to stderr — the caller is presumed to be the shutdown
/// path where no UI is available to display errors.
fn flush_config_if_dirty(
    dirty: &mut bool,
    config: &crate::config::types::AppConfig,
    path: Option<&std::path::Path>,
) {
    if !*dirty {
        return;
    }
    match crate::config::app::save(config, path) {
        Ok(()) => {
            *dirty = false;
        }
        Err(e) => {
            eprintln!("sshi: failed to save config on quit: {e}");
        }
    }
}
```

If the `AppConfig` path differs, verify with `rg -nP 'pub struct AppConfig\b' src/`. The function signature observed during spec review is `pub fn save(config: &AppConfig, path: Option<&Path>) -> Result<()>` (atomic via tempfile + persist at `src/config/app.rs:117–130`).

- [ ] **Step 5.2: Call the helper in the quit path**

Locate the quit block in `App::run` at `app.rs:356–358`:

```rust
            if self.should_quit {
                self.save_state();
                break;
            }
```

Replace with:

```rust
            if self.should_quit {
                self.save_state();
                self.flush_dirty_config_to_disk();
                break;
            }
```

`save_state` first (it persists TUI state which is unrelated to config); `flush_dirty_config_to_disk` second.

- [ ] **Step 5.3: Add tests that exercise the autosave function the quit path calls**

In `src/tui/app.rs`, locate or create a `#[cfg(test)] mod tests` block at the bottom of the file. If one exists, append; if not, create:

```rust
#[cfg(test)]
mod flush_tests {
    // These tests cover the persistence logic invoked by the quit-path
    // wiring in `App::run` (Step 5.2). `App::flush_dirty_config_to_disk`
    // delegates 1-for-1 to the free function `flush_config_if_dirty`, so
    // testing the free function gives us the same coverage without
    // constructing a full App (which requires a real terminal backend).
    //
    // The wiring itself (calling flush_dirty_config_to_disk from the
    // should_quit branch in App::run) is two lines and reviewed by eye;
    // the substantive behavior — flag + save + flag-clear on success — is
    // exhaustively covered here.

    use super::flush_config_if_dirty;
    use crate::config::app as cfg_app;
    use crate::config::types::AppConfig;
    use tempfile::NamedTempFile;

    #[test]
    fn flush_writes_when_dirty_and_clears_flag() {
        let mut config = AppConfig::default();
        config.settings.skipped_hosts = vec!["host-a".to_string(), "host-b".to_string()];
        let tmp = NamedTempFile::new().expect("temp file");
        let path = tmp.path().to_path_buf();

        let mut dirty = true;
        flush_config_if_dirty(&mut dirty, &config, Some(&path));

        assert!(!dirty, "dirty flag must be cleared on successful save");
        let loaded = cfg_app::load(Some(&path)).expect("load");
        assert_eq!(loaded.settings.skipped_hosts, vec!["host-a", "host-b"]);
    }

    #[test]
    fn flush_is_noop_when_not_dirty() {
        let config = AppConfig::default();
        let tmp = NamedTempFile::new().expect("temp file");
        let path = tmp.path().to_path_buf();
        // Pre-write a marker; the no-op call must not overwrite it.
        std::fs::write(&path, b"# pre-existing\n").unwrap();
        let mtime_before = std::fs::metadata(&path).unwrap().modified().unwrap();

        let mut dirty = false;
        flush_config_if_dirty(&mut dirty, &config, Some(&path));

        assert!(!dirty);
        let mtime_after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after, "file must not be touched when not dirty");
    }
}
```

**Verify before running:**
- `Cargo.toml` lists `tempfile` (any section is fine — the helper crate is used widely in tests). If absent, add to `[dev-dependencies]` in this same step:

  ```toml
  [dev-dependencies]
  tempfile = "3"
  ```

  Run `grep -E '^tempfile' Cargo.toml` to check.

- Adjust `cfg_app::load` and `AppConfig::default` to actual API names if they differ (verified via `rg -n 'pub fn load|pub fn save' src/config/app.rs` and `rg -nP 'impl Default for AppConfig|AppConfig\s*\{' src/`).
- If `AppConfig::default()` does not exist, replace with whichever zero-value constructor the codebase exposes (e.g. `AppConfig::empty()`), or build by literal: `AppConfig { host: vec![], check: vec![], sync: vec![], settings: Default::default() }` (verify the field list with `rg -nA10 'pub struct AppConfig' src/config/types.rs`).

- [ ] **Step 5.4: Run the new tests**

Run: `cargo test --features tui -p sshi --lib flush_tests -- --nocapture`

Expected: both tests pass.

- [ ] **Step 5.5: Run the full suite**

Run: `cargo test --features tui -p sshi --lib`

Expected: all tests pass.

- [ ] **Step 5.6: Commit**

```bash
git add src/tui/app.rs
git commit -m "feat(tui): autosave dirty config to disk on quit

Adds App::flush_dirty_config_to_disk(), called from the quit path
just before breaking out of the event loop. Distinct from
save_config() because it skips reload/banner state during teardown.
Closes the persistence gap for Hosts/Checks/Syncs sections."
```

---

## Task 6: Rewrite `E` key handler to autosave; remove `OpenEditorDirty`

**Files:**
- Modify: `src/tui/app.rs` — the `E` key handler at line 1299.
- Modify: `src/tui/tabs/config_tab.rs` — remove `ConfirmAction::OpenEditorDirty` variant + any handler arm.

- [ ] **Step 6.1: Locate every reference to `OpenEditorDirty`**

Run: `rg -n "OpenEditorDirty" src/`

Expected: three or four sites — the enum variant definition (`config_tab.rs:324`), the construction site (`app.rs:1307`), and any match arm that handles it. Note them all.

- [ ] **Step 6.2: Rewrite the `E` key handler in `app.rs`**

Find the block at `app.rs:1299–1313`:

```rust
            KeyCode::Char('E') if self.active_tab == TabId::Config => {
                if self.running_op.is_some() {
                    self.error =
                        Some("Cannot edit config while an operation is running.".to_string());
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
                Ok(true)
            }
```

Replace with:

```rust
            KeyCode::Char('E') if self.active_tab == TabId::Config => {
                if self.running_op.is_some() {
                    self.error =
                        Some("Cannot edit config while an operation is running.".to_string());
                } else {
                    if self.config_tab.config_dirty {
                        self.save_config();
                    }
                    // Skip editor open if save failed: config_dirty stays true
                    // and self.error is set by save_config(); the user sees
                    // the error and can react.
                    if !self.config_tab.config_dirty {
                        self.needs_editor_open = true;
                    }
                }
                Ok(true)
            }
        }
```

(Be careful with the trailing `}` — match the existing brace structure exactly.)

- [ ] **Step 6.3: Remove the `OpenEditorDirty` variant + any match arm that handled it**

In `src/tui/tabs/config_tab.rs` at line 321–325:

```rust
#[derive(Debug)]
pub enum ConfirmAction {
    DeleteEntry { kind: EntryFormKind, index: usize },
    DiscardDirty,
    OpenEditorDirty,
}
```

Change to:

```rust
#[derive(Debug)]
pub enum ConfirmAction {
    DeleteEntry { kind: EntryFormKind, index: usize },
    DiscardDirty,
}
```

If any `match` on `ConfirmAction` had an `OpenEditorDirty` arm, remove it. To find: `rg -n "ConfirmAction::" src/`. Match arms can live in any of the confirm-handling functions.

- [ ] **Step 6.4: Verify no remaining references**

Run: `rg -n "OpenEditorDirty" src/`

Expected: no matches.

- [ ] **Step 6.5: Run `cargo check`**

Run: `cargo check --features tui -p sshi`

Expected: clean build. If a `match` is now non-exhaustive, the compiler will pinpoint it — add or remove the missing arm.

- [ ] **Step 6.6: Run the full test suite**

Run: `cargo test --features tui -p sshi --lib`

Expected: all tests pass.

- [ ] **Step 6.7: Commit**

```bash
git add src/tui/app.rs src/tui/tabs/config_tab.rs
git commit -m "feat(tui): autosave before opening external editor (key E)

Replaces the 'Unsaved changes will be lost' confirm popup with
an automatic save. If the save fails, config_dirty stays true and
the error is surfaced via self.error; editor open is skipped so
the user can react. Removes the now-unused
ConfirmAction::OpenEditorDirty variant."
```

---

## Task 7: Add `ConfigSelectionSnapshot` + capture/restore methods

**Files:**
- Modify: `src/tui/tabs/config_tab.rs` — add struct, two methods. (Wiring into `app.rs` is Task 8.)

The snapshot maps to the actual `ConfigTabState` fields verified during planning: a single `sidebar_vp` (not separate section/entry viewports), plus `field_vp`, `entry_form.vec_editor.vp` (when present), and `direct_vec_editor.vp` (when present). This deviates slightly from the spec's `section_idx + entry_idx` naming because the real state has a unified sidebar; behavior is identical.

- [ ] **Step 7.1: Add the struct**

Edit `src/tui/tabs/config_tab.rs`. Insert the struct just before `impl ConfigTabState` at `~line 350`:

```rust
/// Cursor-position snapshot captured before save+reload, restored after.
/// Each field is clamped against the post-reload state in `restore_selection`.
#[derive(Default, Clone, Debug)]
pub(super) struct ConfigSelectionSnapshot {
    sidebar_idx: usize,
    field_idx: Option<usize>,
    entry_form_open: bool,
    vec_editor_idx: Option<usize>,
    vec_editor_field_index: Option<usize>,
    direct_vec_idx: Option<usize>,
}
```

`pub(super)` is correct because `app.rs` is in `src/tui/`, the parent module of `src/tui/tabs/`. If the visibility isn't reachable in practice, raise to `pub`. Verify by `rg -n 'mod tabs' src/tui/mod.rs`.

- [ ] **Step 7.2: Write failing tests for capture/restore**

Inside the existing `mod tests` block in `config_tab.rs`, append:

```rust
    #[test]
    fn snapshot_round_trip_no_form_no_popup() {
        let mut config = AppConfig::default();
        config.host.push(crate::config::types::HostEntry {
            name: "h1".to_string(),
            ssh_host: "1.1.1.1".to_string(),
            shell: crate::config::types::ShellType::Sh,
            groups: vec![],
            proxy_jump: None,
        });
        config.host.push(crate::config::types::HostEntry {
            name: "h2".to_string(),
            ssh_host: "2.2.2.2".to_string(),
            shell: crate::config::types::ShellType::Sh,
            groups: vec![],
            proxy_jump: None,
        });
        let mut state = ConfigTabState::new(&config, None);
        // Move sidebar cursor down a few times so it's not at 0.
        state.sidebar_vp.move_down();
        state.sidebar_vp.move_down();
        let captured_sidebar = state.sidebar_vp.selected;
        let snap = state.capture_selection();
        // Simulate reload that resets sidebar to 0.
        state.sidebar_vp = Viewport::new();
        state.sidebar_vp.set_dims(state.items.len(), 0);
        state.restore_selection(snap, &config);
        assert_eq!(state.sidebar_vp.selected, captured_sidebar);
    }

    #[test]
    fn snapshot_clamps_when_entry_deleted() {
        let mut config = AppConfig::default();
        // Build a config with 3 hosts.
        for i in 0..3 {
            config.host.push(crate::config::types::HostEntry {
                name: format!("h{i}"),
                ssh_host: format!("{i}.{i}.{i}.{i}"),
                shell: crate::config::types::ShellType::Sh,
                groups: vec![],
                proxy_jump: None,
            });
        }
        let mut state = ConfigTabState::new(&config, None);
        // Cursor at last sidebar entry.
        let last = state.items.len() - 1;
        for _ in 0..last {
            state.sidebar_vp.move_down();
        }
        let snap = state.capture_selection();
        // Simulate deletion: remove a host, rebuild items.
        config.host.pop();
        state.items = build_sidebar_items(&config);
        state.sidebar_vp = Viewport::new();
        state.sidebar_vp.set_dims(state.items.len(), 0);
        state.restore_selection(snap, &config);
        // Should clamp to new len-1 (or 0 if items empty).
        let expected = state.items.len().saturating_sub(1);
        assert_eq!(state.sidebar_vp.selected, expected);
    }
```

If `build_sidebar_items` is not visible (private to module), this test sits inside the same module so it's reachable. Verify by grep: `rg -n 'fn build_sidebar_items' src/tui/tabs/config_tab.rs`.

- [ ] **Step 7.3: Run tests to verify they fail**

Run: `cargo test --features tui -p sshi --lib snapshot_ -- --nocapture`

Expected: compile errors — `capture_selection` and `restore_selection` are undefined.

- [ ] **Step 7.4: Implement `capture_selection`**

Inside `impl ConfigTabState`, add:

```rust
    pub(super) fn capture_selection(&self) -> ConfigSelectionSnapshot {
        let mut snap = ConfigSelectionSnapshot {
            sidebar_idx: self.sidebar_vp.selected,
            ..Default::default()
        };
        if let Some(form) = self.entry_form.as_ref() {
            snap.entry_form_open = true;
            snap.field_idx = Some(form.field_vp.selected);
            if let Some(ve) = form.vec_editor.as_ref() {
                snap.vec_editor_field_index = Some(ve.field_index);
                snap.vec_editor_idx = Some(ve.vp.selected);
            }
        }
        if let Some(dve) = self.direct_vec_editor.as_ref() {
            snap.direct_vec_idx = Some(dve.vp.selected);
        }
        snap
    }
```

The exact field names (`form.field_vp`, `form.vec_editor`, `ve.field_index`, `ve.vp`, `dve.vp`) were verified at planning time at `config_tab.rs:127–131` (EntryFormState) and the direct-vec editor struct. If any name differs at implementation time, locate via grep and adjust.

- [ ] **Step 7.5: Implement `restore_selection`**

```rust
    pub(super) fn restore_selection(&mut self, snap: ConfigSelectionSnapshot, _config: &AppConfig) {
        let clamp = |idx: usize, len: usize| -> usize {
            if len == 0 { 0 } else { idx.min(len - 1) }
        };
        // Sidebar.
        let sidebar_len = self.items.len();
        self.sidebar_vp.selected = clamp(snap.sidebar_idx, sidebar_len);
        // Re-run clamp() inside Viewport to keep scroll_y in sync.
        self.sidebar_vp.set_dims(sidebar_len, self.sidebar_vp.visible_height);
        // Entry form field.
        if snap.entry_form_open {
            if let (Some(form), Some(fi)) = (self.entry_form.as_mut(), snap.field_idx) {
                let flen = form.fields.len();
                form.field_vp.selected = clamp(fi, flen);
                form.field_vp.set_dims(flen, form.field_vp.visible_height);
                // Vec editor inside the form.
                if let (Some(ve), Some(target_field), Some(vidx)) = (
                    form.vec_editor.as_mut(),
                    snap.vec_editor_field_index,
                    snap.vec_editor_idx,
                ) {
                    if ve.field_index == target_field {
                        let ilen = ve.items.len();
                        ve.vp.selected = clamp(vidx, ilen);
                        ve.vp.set_dims(ilen, ve.vp.visible_height);
                    }
                }
            }
        }
        // Direct vec editor.
        if let (Some(dve), Some(didx)) = (self.direct_vec_editor.as_mut(), snap.direct_vec_idx) {
            let ilen = dve.items.len();
            dve.vp.selected = clamp(didx, ilen);
            dve.vp.set_dims(ilen, dve.vp.visible_height);
        }
    }
```

The unused `_config` parameter is retained because the spec's clamping rules reference `config.host.len()` etc.; in practice the `self.items` length already reflects post-reload state, so `_config` is not needed. Keep the signature with the parameter for forward compatibility, marked unused.

- [ ] **Step 7.6: Run tests to verify they pass**

Run: `cargo test --features tui -p sshi --lib snapshot_ -- --nocapture`

Expected: both tests pass.

- [ ] **Step 7.7: Commit**

```bash
git add src/tui/tabs/config_tab.rs
git commit -m "feat(tui): ConfigSelectionSnapshot for cursor preservation

Adds capture_selection / restore_selection on ConfigTabState that
snapshot sidebar_vp.selected, the entry-form field cursor, the
in-form vec editor cursor (with field-index guard), and the direct
vec editor cursor. Each restored index is clamped against the
post-reload length. Tested with round-trip and entry-deleted cases.
Wired into save_config in the next task."
```

---

## Task 8: Wire selection snapshot into `save_config`; remove `pending_field_restore`

**Files:**
- Modify: `src/tui/app.rs` — `save_config()` brackets save/reload with snapshot capture/restore; remove the two `pending_field_restore` restore blocks.
- Modify: `src/tui/tabs/config_tab.rs` — remove the `pending_field_restore` field; change `commit_entry_form` return type from `Option<usize>` to `()`.

- [ ] **Step 8.1: Bracket `save_config` with capture/restore**

In `src/tui/app.rs`, locate `fn save_config(&mut self)` at line 1762. The current body matches:

```rust
    fn save_config(&mut self) {
        let explicit_path = self.config_path.clone();
        let path_arg = explicit_path.as_deref();
        match crate::config::app::save(&self.config, path_arg) {
            Ok(()) => {
                self.config_tab.config_dirty = false;
                self.config_tab.reload_banner_until = Some(Instant::now() + Duration::from_secs(2));
                if let Ok(resolved) = crate::config::app::resolve_path(path_arg) {
                    if self.config_path.is_none() {
                        self.config_path = Some(resolved.clone());
                    }
                    self.config_tab.reload(&self.config, Some(&resolved));
                } else {
                    self.config_tab.reload(&self.config, path_arg);
                }
            }
            Err(e) => { ... }
        }
    }
```

Add `let snap = self.config_tab.capture_selection();` as the first line of the method, and `self.config_tab.restore_selection(snap, &self.config);` immediately after each of the two `reload(...)` calls inside the `Ok(())` arm.

Final shape:

```rust
    fn save_config(&mut self) {
        let snap = self.config_tab.capture_selection();
        let explicit_path = self.config_path.clone();
        let path_arg = explicit_path.as_deref();
        match crate::config::app::save(&self.config, path_arg) {
            Ok(()) => {
                self.config_tab.config_dirty = false;
                self.config_tab.reload_banner_until = Some(Instant::now() + Duration::from_secs(2));
                if let Ok(resolved) = crate::config::app::resolve_path(path_arg) {
                    if self.config_path.is_none() {
                        self.config_path = Some(resolved.clone());
                    }
                    self.config_tab.reload(&self.config, Some(&resolved));
                    self.config_tab.restore_selection(snap, &self.config);
                } else {
                    self.config_tab.reload(&self.config, path_arg);
                    self.config_tab.restore_selection(snap, &self.config);
                }
            }
            Err(e) => { /* unchanged */ }
        }
    }
```

(The `Err` arm is unchanged because on failure the state is not reloaded, so restoration is unnecessary.)

- [ ] **Step 8.2: Remove the two `pending_field_restore` restore blocks**

In `src/tui/app.rs`, locate and delete the post-save restore block at `app.rs:1074–1081` (popup save path):

```rust
                let restore = self.config_tab.pending_field_restore.take();
                self.save_config();
                if let Some(idx) = restore {
                    let count = self.config_tab.current_descriptors(&self.config).len();
                    if idx < count {
                        self.config_tab.field_vp.selected = idx;
                    }
                }
```

Replace with simply:

```rust
                self.save_config();
```

Same surgery at the second block (`app.rs:1361–1370`, main save path).

Both locations: the four lines after `self.save_config();` that consult `restore` go away. The `let restore = ...take();` line above goes away. Net: only `self.save_config();` remains where there were 8–9 lines.

- [ ] **Step 8.3: Remove the `pending_field_restore` field and its usages**

In `src/tui/tabs/config_tab.rs`:
- Delete line `344: pub pending_field_restore: Option<usize>,` from the `ConfigTabState` struct.
- Delete the corresponding init in `ConfigTabState::new` at line 374: `pending_field_restore: None,`.
- Change `commit_entry_form`'s return type from `-> Option<usize>` to `-> ()` (i.e. remove `-> Option<usize>` entirely so the implicit unit return applies).
- Inside `commit_entry_form`, remove `let saved_sel = form.field_vp.selected;` and any `return Some(saved_sel)` / `Some(saved_sel)` final expression.

To find all usages: `rg -n "pending_field_restore" src/`

The handler at `config_tab.rs:1132` will fail to compile because `commit_entry_form` no longer returns `Option<usize>`:

```rust
            KeyCode::Char('s') => {
                self.pending_field_restore = self.commit_entry_form(config);
                self.entry_form = None;
                self.pending_save = true;
                true
            }
```

Change to:

```rust
            KeyCode::Char('s') => {
                self.commit_entry_form(config);
                self.entry_form = None;
                self.pending_save = true;
                true
            }
```

- [ ] **Step 8.4: Verify no remaining references**

Run: `rg -n "pending_field_restore" src/`

Expected: no matches.

- [ ] **Step 8.5: Run `cargo check`**

Run: `cargo check --features tui -p sshi`

Expected: clean build. Compiler will flag any missed usage.

- [ ] **Step 8.6: Run the full test suite**

Run: `cargo test --features tui -p sshi --lib`

Expected: all tests pass (including the new snapshot tests from Task 7).

- [ ] **Step 8.7: Commit**

```bash
git add src/tui/app.rs src/tui/tabs/config_tab.rs
git commit -m "refactor(tui): unify save-time cursor restore via snapshot

save_config() now brackets reload() with capture_selection() /
restore_selection(), subsuming the ad-hoc pending_field_restore
mechanism. Removes the field, its restore blocks, and the
commit_entry_form return value. After saving, the sidebar / field /
in-form vec editor / direct vec editor cursors all preserve their
positions."
```

---

## Task 9: TUI audit — produce findings doc

**Files:**
- Create: `docs/superpowers/audits/2026-05-21-tui-audit.md`.

This task is an investigation, not a fix. Each finding is written into the audit doc with severity and disposition. Blocker findings get their own follow-up task (Task 10).

- [ ] **Step 9.1: Run the scripted greps**

Run each in sequence and capture output:

```sh
echo "=== len().max(1) sites (should be empty after Task 2) ==="
rg -n "len\(\)\.max\(1\)" src/tui/

echo "=== unwrap() in src/tui ==="
rg -n "\.unwrap\(\)" src/tui/ | wc -l
rg -n "\.unwrap\(\)" src/tui/ | head -30

echo "=== expect() in src/tui ==="
rg -n "\.expect\(" src/tui/ | wc -l
rg -n "\.expect\(" src/tui/ | head -30

echo "=== std::process::exit / panic! / unreachable! ==="
rg -n "std::process::exit|panic!|unreachable!" src/tui/

echo "=== Hardcoded config vec indexing in non-test code ==="
rg -n "config\.(host|check|sync)\[" src/tui/

echo "=== Signal handling ==="
rg -n "tokio::signal|ctrlc|SIGINT|SIGTERM|spawn_signal_listener" src/tui/

echo "=== File length heuristic ==="
wc -l src/tui/app.rs src/tui/tabs/*.rs src/tui/components/*.rs src/tui/widgets/*.rs 2>/dev/null
```

- [ ] **Step 9.2: Targeted reads**

Read each of these files and note any patterns the greps could miss:
- `src/tui/app.rs` — focus on event loop, signal handling, shutdown.
- `src/tui/tabs/operate_tab.rs` — sibling tab; check for parallel render anti-patterns.
- `src/tui/state/persist.rs` — TUI state save reliability.
- `src/tui/components/*.rs` — viewport already covered.

- [ ] **Step 9.3: Write findings into the audit doc**

Create `docs/superpowers/audits/2026-05-21-tui-audit.md` with this exact structure:

```markdown
# TUI Audit — 2026-05-21

**Spec:** `docs/superpowers/specs/2026-05-21-config-tab-fixes-and-tui-audit-design.md`
**Audit scope:** `src/tui/`.

## Methodology

Scripted greps + targeted reads, per the spec's §8.2. Findings classified
blocker / major / minor per §8.4.

## Findings

<!-- One block per finding. Use the spec's exact §8.3 format. -->

### [BLOCKER|MAJOR|MINOR] <one-line title>
**File:** `path:line`
**Symptom:** What goes wrong, when.
**Fix sketch:** One paragraph.
**Disposition:** fixed-in-this-spec | deferred-backlog

<!-- repeat -->

## Deferred Backlog

A consolidated list of MAJOR + MINOR findings whose disposition is
deferred-backlog. Each row links to the finding above.

| Severity | Title | File | Disposition |
|---|---|---|---|
| ... | ... | ... | deferred-backlog |
```

For each grep hit and targeted-read observation that constitutes a finding, add one block. **For every blocker finding, also add an entry to the table at the bottom of THIS plan in Task 10's "Blocker findings list" section** (the implementor will fold blocker fixes there).

Severity calls — use the §8.4 rubric verbatim:
- Blocker if it could crash the app or corrupt config.
- Major if wrong behavior under realistic input, no crash. Default deferred unless the fix is < ~20 LOC AND touches a file already in this PR's diff.
- Minor for style/dead code/typos. Always deferred.

When in doubt, downgrade. When unsure whether something is a real issue at all (e.g. an `.unwrap()` on a `Mutex::lock` where poisoning is genuinely fatal), do not record it — note your reasoning briefly in a "Reviewed but not recorded" trailing section.

- [ ] **Step 9.4: Commit the audit doc**

```bash
git add docs/superpowers/audits/2026-05-21-tui-audit.md
git commit -m "docs(audit): TUI audit 2026-05-21 findings

Scripted greps + targeted reads per spec §8. <N> findings total:
<X> blocker / <Y> major / <Z> minor. Blocker findings folded into
this PR via Task 10; major and minor deferred per rubric."
```

(Replace `<N>/<X>/<Y>/<Z>` with the actual counts before committing.)

---

## Task 10: Fold blocker audit findings (one sub-task per blocker)

**Files:** TBD per finding — listed in the audit doc.

This task is a placeholder. If Task 9 produced zero blocker findings, skip and commit nothing here.

For each blocker finding from Task 9:

- [ ] **Step 10.X.1: Write a failing test (if applicable)**

For findings that admit a regression test, write it first. Crash-class findings get a smoke render test similar to Task 4. Logic findings get a unit test.

- [ ] **Step 10.X.2: Apply the fix**

Show the exact diff in the audit doc's "Fix sketch" field. Apply it.

- [ ] **Step 10.X.3: Run tests**

Run: `cargo test --features tui -p sshi --lib`

Expected: all tests pass, including the new one.

- [ ] **Step 10.X.4: Update the audit doc**

Change the finding's `Disposition` from `fixed-in-this-spec` (planned) to `fixed-in-this-spec ✓ (commit <hash>)` after the commit lands.

- [ ] **Step 10.X.5: Commit**

```bash
git add <files>
git commit -m "fix(tui): <blocker-finding-title>

Folds audit finding from docs/superpowers/audits/2026-05-21-tui-audit.md.
<one-line description>"
```

---

## Task 11: CHANGELOG + finalization

**Files:**
- Modify: `CHANGELOG.md` — append a single Unreleased entry summarizing this PR.

- [ ] **Step 11.1: Read the current `CHANGELOG.md` to learn the format**

Run: `head -30 CHANGELOG.md`

Note the section header style (e.g. `## Unreleased` vs `## [Unreleased]`), the date convention, and the bullet style.

- [ ] **Step 11.2: Append the Unreleased entry**

Insert under the existing Unreleased section (or create one immediately after the top-level `# Changelog` heading if none exists). Use the exact style observed in Step 11.1. Content:

```markdown
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
```

- [ ] **Step 11.3: Final test sweep**

Run: `cargo test --features tui -p sshi`

Expected: all tests pass.

- [ ] **Step 11.4: Final manual reproduction (optional but recommended)**

Run: `cargo run --features tui` — exercise the three crash reproductions from the bug report:
1. Edit a Sync, navigate to `paths` field, open the vec editor with an empty list — should not crash.
2. Settings → `skipped_hosts` empty → open direct vec editor — should not crash.
3. Checks → `enabled` empty → open direct vec editor — should not crash.

Also verify:
4. Edit a Host's name, press `q` — quit, restart, check the on-disk config reflects the change.
5. Edit a Host's name, press `E` — external editor opens against the saved-on-disk file with the new name.
6. After saving via `s`, cursor position is preserved.

- [ ] **Step 11.5: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs(changelog): unreleased entry for config-tab fixes + TUI audit"
```

---

## Self-Review Notes

**Spec coverage check:**

| Spec section | Plan task |
|---|---|
| §5 Viewport contract fix | Tasks 1, 2, 3, 4 |
| §6 Autosave-on-quit | Task 5 (quit) + Task 6 (editor) |
| §7 Selection snapshot | Tasks 7, 8 |
| §8 TUI audit | Tasks 9, 10 |
| §9 Testing summary | covered by 1.1, 4.x, 5.3, 7.2; integration test for autosave-on-quit is the helper test in 5.3 |
| §10 Risks: signal handling | Audit Task 9 (grep `tokio::signal`); if a blocker emerges (e.g. `process::exit` short-circuiting save), Task 10 picks it up |
| §10 Risks: `scroll_y` getter | Resolved — `scroll_y` is already `pub`, no getter needed. Plan uses field access directly. |
| §10 Risks: hidden render sites | Task 3 Step 3.4 explicitly checks the candidate sites |

**Naming reconciliation:** the spec's snapshot fields `section_idx + entry_idx` are collapsed in the plan to a single `sidebar_idx` because the real state has one `sidebar_vp`. Behavior is equivalent; this is a verified deviation worth flagging in code review.

**Integration-test scope (Task 5) — intentional gap.** Spec §6.3 (as updated post-plan-verify R2) explicitly scopes the autosave persistence test to the free-function level. Rationale: constructing a real `App` requires a terminal backend and event channel; building a synthetic-backend harness for one end-to-end test is more work than the rest of this PR. Mitigations:

1. The persistence behavior is exhaustively tested at the free-function level (Step 5.3 covers dirty→write→clear-flag and not-dirty→no-op).
2. The method-to-function delegation in `App::flush_dirty_config_to_disk` is a single `flush_config_if_dirty(...)` call — no logic to bug.
3. The quit-path wiring (Step 5.2) is two lines inserted into `App::run`'s `should_quit` branch — reviewed by eye and validated by Step 11.4's manual reproduction (edit → `q` → restart → assert on-disk content).

§6.4 acceptance ("After any edit followed by `q`, the on-disk file reflects the edit") is covered by (1) + (3). The end-to-end automated coverage gap is an accepted risk, called out here and in spec §6.3.

**Spec §5.5 count typo:** Spec §5.5 was corrected from "15" to "14" `.max(1)` sites during plan-verify R1 (the verified count is 14, consistent with §5.3's list and the grep output the plan uses throughout).

**Placeholder scan:** no TBDs, every code step has the exact code, every command has expected output. The single `TBD` text appears in Task 10's Files line and is intentional — Task 10 is a per-finding placeholder; if Task 9 produces zero blockers, Task 10 is skipped.
