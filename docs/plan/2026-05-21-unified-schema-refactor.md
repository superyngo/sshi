# Unified Schema Refactor Plan — TUI Config Tab
Status: Shipped (2026-06-03)

Status: REVISED v2 — incorporates external review feedback
Author: collaborative session (2026-05-21)
Scope: `src/tui/tabs/config_tab.rs` (~3436 LOC)

## Revision history

- **v1**: initial draft
- **v2 (this)**: incorporates external code review:
  - path key changed from `path:{label}` to `path:{index}` (labels not unique per `src/config/schema.rs:179–183`)
  - explicit raw-vs-display value contract added to apply() spec
  - cursor snapshot moved to a dedicated `pending_restore_snapshot` field on state (not a local var)
  - task order: cursor-preservation work moved AFTER apply rewrite is green
  - schema extracted to `src/tui/tabs/config_tab/config_schema.rs` submodule

## Context: the bugs this targets

Three persistent bugs in the TUI Config tab share one root cause:

1. **Hosts/Checks/Syncs edits don't persist.** Settings does. Root cause: `apply_host_field` / `apply_check_field` / `apply_sync_field` (config_tab.rs:2748–2810) match only scalar keys; Vec keys (`groups`, `enabled`, `paths`) fall through `_ =>` and are silently dropped. Direct popup commits (`enabled`/`groups` editors) flow:
   `commit_direct_popup_field → commit_inline_edit → apply_*_field` → drop → save_config writes unchanged config. "Save succeeded but nothing changed."

2. **Two parallel field schemas diverge:**
   - Right panel: `host_descriptors` / `check_descriptors` / `sync_descriptors` (line 2588+)
   - Entry-form popup (add/edit): `host_form_fields` / `check_form_fields` / `sync_form_fields` (line 238+)

   Observed divergences:
   - `host_descriptors` hides `proxy_jump` when `None` (line 2603 conditional push); form always shows it.
   - `check_descriptors` types `enabled` as `FieldKind::VecString` (generic vec editor); `check_form_fields` types it as `FieldKind::CheckEnabled` (group picker multi-select). Two editors for one field.

3. **Cursor jumps to first row after edits.**
   - **Sidebar:** `commit_entry_form` (line 1507) wipes `sidebar_vp = Viewport::new()` BEFORE `save_config` captures snapshot. Capture happens too late — sees `selected = 0`.
   - **Field (right panel):** `restore_selection` (line 425) only restores `field_vp.selected` when `snap.entry_form_open == true`. Direct popup path never sets that flag; `reload()` (line 461) wipes `field_vp = Viewport::new()`. Field cursor has no restoration path.

Settings works because (a) it has no Vec fields through direct popup, and (b) inline edits run through `apply_settings_field` which covers all keys.

## User-confirmed decisions

1. Right panel and entry-form show the **same** field list. Optional fields (`proxy_jump`, `mode`, `source`) always visible, rendered empty when None.
2. Save timing **stays as-is**: mutation → `config_dirty = true` → immediate `save_config`. Autosave-on-quit is NOT added; the previously proposed switch was orthogonal to the real bug.
3. Check `enabled` uses the **group-picker multi-select** (current entry-form behavior). Right panel switches to this picker.

## Design

### Single source of truth per entry kind

Lives in a new submodule: `src/tui/tabs/config_tab/config_schema.rs`.

For each of `HostEntry`, `CheckEntry`, `SyncEntry`, exactly two functions:

```rust
fn fields(entry: &T) -> Vec<FieldDescriptor>;
fn apply(entry: &mut T, key: &str, raw_value: &str);
```

Used by ALL three call sites:
- Right-panel render and inline edit
- Direct popup commit
- Entry-form commit

The `key` in `FieldDescriptor` is the contract between read and write. Apply is a `match key` table, not a `match index` — eliminating the current fragile index-to-fields[idx] coupling.

### Raw vs display value contract (new, per review feedback)

`FieldDescriptor` carries TWO value-related strings:
- `display_value: String` — what the renderer shows (`"(none)"`, `"[a, b, c]"`, `"true"`)
- `raw_value: String` — what gets fed to `apply()` (`""`, `"a,b,c"`, `"true"`)

Or equivalently: a single value field plus a `parse_for_apply(display) -> raw` helper. Implementation choice TBD during execution, but the **contract is mandatory**: callers MUST pass raw form, not display form, into `apply()`. `commit_entry_form` is the easiest place to get this wrong because it currently passes `f.display_value` straight through; that path must convert via the schema.

For Vec fields, the existing `parse_bracket_list()` helper already covers the display-to-raw direction; canonicalise on it.

### Unified field list per kind

| Kind | Fields (in order) |
|---|---|
| Host | name, ssh_host, shell, groups, proxy_jump |
| Check | enabled, groups, enable_hosts, enable_all, path:0, path:1, … |
| Sync | paths, groups, enable_hosts, enable_all, recursive, mode, propagate_deletes, source |

`proxy_jump`, `mode`, `source` always rendered (empty string when None).

**Path key choice (per review):** `CheckPath` (`src/config/schema.rs:179–183`) does not guarantee unique `label` within a Check. Use `path:{index}` (e.g. `path:0`, `path:1`). Indices are stable for the duration of a single edit session — the form operates on a live copy of the entry and no reorder happens between fields() and apply() within one mutation. If reorder support is added later, schema must be re-derived.

### FieldKind → editor routing

| Kind | Editor |
|---|---|
| String / OptionalString | inline text input |
| Bool | space toggle |
| TriBool / Enum / ShellEnum | left/right cycle |
| VecString | direct vec editor popup |
| CheckEnabled | direct group picker popup (replaces VecString path for Check.enabled) |
| VecCheckPath | dedicated path editor (existing) |

### Cursor preservation

**Snapshot must persist across the commit→save→reload boundary on state, not as a local.** Add a field:

```rust
pub struct ConfigTabState {
    // ...
    pub pending_restore_snapshot: Option<ConfigSelectionSnapshot>,
}
```

**Commit-path inventory (all sites that set `pending_save = true`):**
- `commit_entry_form` (entry-form save) — line ~1202
- `commit_direct_popup_field` (direct vec/group popup save) — lines ~2273, ~2350
- `handle_inline_edit_key` (scalar inline edit confirm) — line ~891
- TriBool / Enum / ShellEnum / Bool cycle handlers — lines ~709, 717, 725, 740, 747, 754, 771, 787, 799, 807, 816

ALL of these must set `pending_restore_snapshot` before the save kicks in. Centralise via a single helper: `fn mark_dirty(&mut self) { self.pending_restore_snapshot = Some(self.capture_selection()); self.config_dirty = true; self.pending_save = true; }` — replaces the current three-line incantation at every commit site.

Flow:
1. Every commit site calls `self.mark_dirty()` instead of setting the three flags separately.
2. `save_config` (in app.rs) **stops** calling `capture_selection()` itself; instead reads `self.config_tab.pending_restore_snapshot.take()` after reload and passes that to `restore_selection`.
3. `reload()` and `commit_entry_form` switch from `viewport = Viewport::new()` to `set_dims(new_len, height)` so `selected` survives the rebuild (then clamp inside `restore_selection`).
4. `restore_selection` adds a branch for "snap.entry_form_open == false but field_vp.selected should still be restored when direct popup just closed". Discriminator: introduce `snap.field_vp_was_active: bool` (or similar), set true whenever a direct popup commits.

This addresses the reviewer's explicit warning: forgetting to consume the stored snapshot silently reverts to the original bug.

### Enforced invariant: `app.rs` cannot self-capture (per v2 review)

Verified call sites of `capture_selection()`:
- `src/tui/app.rs:1772` — the **only production caller**; gets replaced by `consume_pending_snapshot()`
- `src/tui/tabs/config_tab.rs:3173, 3197, 3221, 3255, 3317, 3383, 3423` — all inside `#[cfg(test)]` blocks, exercising `capture_selection` directly to test snapshot behavior. These stay.

Refactor:
- `capture_selection` downgraded to `pub(super)` or private — visible to tests in the same module, not to `app.rs`.
- New `pub fn consume_pending_snapshot(&mut self) -> Option<ConfigSelectionSnapshot>` — only API `app.rs` can reach.
- `save_config` in `app.rs:1772` switches from `capture_selection()` to `consume_pending_snapshot()`. If `None` (means no commit happened — e.g. Settings inline edit that already routes through `commit_inline_edit` without a popup), fall back gracefully: just skip restore_selection (the inline edit path doesn't disturb selection anyway, so nothing to restore).

This makes "forgetting to set `pending_restore_snapshot`" a compile error from `app.rs`, not a silent runtime regression.

## Task breakdown

Ordering deliberate: schema + apply land first and are verified green BEFORE touching cursor logic, so any cursor regression isn't tangled with the apply rewrite during bisect.

1. **Create submodule** `src/tui/tabs/config_tab/config_schema.rs`:
   - Move existing `*_form_fields` in as canonical `fields()`
   - Add raw/display value contract (extend `FieldDescriptor` or parallel helpers)
   - Delete `*_descriptors`; redirect callers (lines 506, 525, 544, 961, 1974, 1985, 1996)
   - `cargo build` + `cargo test` checkpoint — no behavior change expected yet for Vec save bug
2. **Rewrite apply_* as complete key-routed tables**:
   - `apply_host_field`: add `groups`
   - `apply_check_field`: add `enabled`, `groups`, `path:{index}` pattern
   - `apply_sync_field`: add `paths`, `groups`
   - All keyed by string key, not by index
   - **Checkpoint**: `cargo test` + manual smoke test — Vec save bug should now be gone
3. **Right-panel call sites** switched to unified schema (descriptor types deleted)
4. **Check `enabled` editor**: simplify lines 828–846 — `FieldKind::CheckEnabled` always routes to group_picker regardless of entry point
5. **commit_entry_form refactor**: replace per-kind match with `for f in form.fields { apply(entry, &f.key, raw_value(f)) }`, sharing the path with the right panel
6. **Cursor preservation (last, isolated)**:
   - Add `pending_restore_snapshot: Option<ConfigSelectionSnapshot>` field
   - Capture in commit_entry_form / commit_direct_popup_field
   - `save_config` consumes that field after reload (stop self-capturing)
   - `reload()` and `commit_entry_form` preserve selected via `set_dims` + clamp
   - Extend `restore_selection` for direct-popup-closed case
7. **Tests**:
   - Per kind, per Vec field: edit via direct popup → assert config mutated AND `save_config` wrote to disk
   - After commit: assert sidebar_vp.selected and field_vp.selected preserved
   - `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`
8. **CHANGELOG**: add Unreleased entry per `~/.claude/CLAUDE.md` workflow

## Risks and mitigations

- **Behavior change (visible to user)**: proxy_jump always shown; Check.enabled editor swapped to group picker. Both are intentional improvements; document in CHANGELOG.
- **Stable keys for path[i]**: use `path:{index}` (see Path key choice above); index is stable within a single edit session.
- **Size**: ~400 LOC concentrated in config_tab.rs; no cross-file ripple.

## Acceptance criteria

- Edit Hosts `groups` from right panel → change visible immediately AND written to disk
- Check `enabled` editor identical whether opened from right panel or entry-form
- After any commit from any entry point, sidebar and field cursors stay in place
- New unit tests pass; existing tests still pass
- `cargo clippy -- -D warnings` clean

## Open questions — RESOLVED in v2

1. ~~Submodule vs same file?~~ → New submodule `src/tui/tabs/config_tab/config_schema.rs`.
2. ~~`path:{label}` uniqueness?~~ → NOT unique; switched to `path:{index}`.
3. ~~Existing tests coupled to index-based apply?~~ → Reviewer found none externally; `cargo test` checkpoint after Task 2 catches any internal coupling.

## v2 review feedback — RESOLVED in v2.1

1. Stale `path:{label}` text in Risks → fixed (now reads `path:{index}`).
2. `capture_selection` call-site enumeration → done; 1 production caller (app.rs:1772) + 7 test-only callers (all in `#[cfg(test)]`); table added.
3. Stronger invariant adopted → `capture_selection` downgraded to `pub(super)`; new `consume_pending_snapshot()` is the only API exposed to `app.rs`.
4. Bonus: added centralised `mark_dirty()` helper covering ALL commit paths (inline edit, cycle, entry form, direct popup), so no commit site can forget to snapshot.
