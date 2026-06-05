# Config / Operate / Init improvements

Date: 2026-06-05

## Decisions
- Autosave on commit (not per-keystroke).
- Check/Sync names stay `Option<String>` + validation (no type change).
- Task 7 is CLI-only, key type `ed25519`, sequential interactive prompts.

## Phases & Tasks

### Phase A — Names as keys
- **Task 2 — Require non-empty + unique check/sync names**
  - `config_tab::commit_entry_form`: reject empty or colliding name; keep form open, show error.
  - On load: non-fatal `tracing::warn` on legacy empty/duplicate names.
  - Tests: empty blocked, duplicate blocked, unique passes.

### Phase B — Name selection UX
- **Task 3 — Check/Sync name fields → multi-select popup**
  - Add `PickerTarget::CheckNames` / `SyncNames` to `member_picker`.
  - Enter on `CheckName`/`SyncName` opens picker; apply joins back into comma value.
  - Render those fields as read-only chip rows (like `Skip`).
- **Task 4 — `a:add` inside the picker**
  - Add `PickerResult::Add`; `a` triggers it (CheckNames/SyncNames only).
  - On `Add`: stash reopen flag, switch to Config, `start_add_entry`; reopen picker after commit.

### Phase C — Sync layout simplification
- **Task 5 — Remove sync mode; show path + entries together**
  - Drop `SyncModeToggle` / `SyncMode` / `toggle_sync_mode`; always show both inputs.
  - `execute_sync` passes `adhoc_files` + `names` + `source_override` unconditionally.
  - `has_entries` keys on `config.sync` non-empty only.
  - `persist.rs`: keep `sync_mode` deserializing-with-default, stop reading it.
- **Task 6 — Move Source-override to bottom of sync params**
  - Reorder `operate_fields` and render rows so `SyncSource` is last.

### Phase D — Standalone
- **Task 1 — Autosave config; remove `s:Save`**
  - `mark_dirty()` also sets `pending_save = true`.
  - Remove `s` key path, `s:Save` footer + help text, dead dirty-guard tab-switch checks.
  - Keep quit-time `flush_dirty_config_to_disk`.
- **Task 7 — Init: ssh-copy-id / keygen for auth failures**
  - New auth-failure partition in `init.rs` (detect "All authentication methods exhausted").
  - Sequentially on real TTY: key exists → prompt `ssh-copy-id`; none → `ssh-keygen -t ed25519` then `ssh-copy-id`; retry connection.

## Verification
- `cargo build` + `cargo test` after each phase.
- Manual TUI smoke-test for B/C; manual real-binary `sshi init` for Task 7.
- CHANGELOG `Unreleased` + README/docs update at the end.

## Sequencing
A → B → C → D, pausing after each phase for review.
