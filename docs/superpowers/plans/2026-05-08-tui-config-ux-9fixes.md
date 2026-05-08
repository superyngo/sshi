# TUI Config UX — 9 Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 9 TUI Config tab UX issues: Bool toggle, Tab cycling, GroupPicker Add, shell/enum single-select, cancel banner, focus preservation, popup key-blocking, and direct vec sub-popup.

**Architecture:** All changes are confined to `src/tui/tabs/config_tab.rs` (primary) and `src/tui/app.rs` (secondary). No new files needed. Each fix is independent and can be committed separately.

**Tech Stack:** Rust, ratatui 0.29, crossterm

> **Line-number drift:** All line references are accurate against the initial codebase state. Each Task modifies `config_tab.rs`, causing subsequent Tasks' line numbers to shift. When executing, re-locate targets by searching for function/variable names, not by absolute line number.

**Execution order:** Task 0 must land first (it fixes the pre-existing `editing_field_index` bug that every subsequent task depends on for correct write targeting). Tasks 1–7 are mostly independent **except**: Task 4+5 Step 2 modifies the same `activate_inline_edit` match arm as Task 1 Step 1 — execute Task 1 first. Task 8 must land **before** Task 9 — the popup guard (`is_any_popup_open`) needs to exist before Task 9 adds `direct_vec_editor` / `direct_group_picker` states that depend on it. Task 9 Step 10 then extends `is_any_popup_open` for the new states.

---

## File Map

- **Modify:** `src/tui/tabs/config_tab.rs` — all config tab state, key handling, rendering
- **Modify:** `src/tui/app.rs` — Tab key routing, popup global key blocking

---

## Task 0: Fix pre-existing TriBool stale `editing_field_index` bug (Pre-fix)

**Files:**
- Modify: `src/tui/tabs/config_tab.rs` — Left/Right/Enter handlers in `ConfigZone::FieldTable`

**Root cause:** `commit_inline_edit` reads `self.editing_field_index` to decide which field to write. For `TriBool`, `activate_inline_edit` returns `false` early (correct — no text popup), but the caller never sets `editing_field_index` before calling `commit_inline_edit`. A stale value from a previous text edit therefore writes to the wrong field.

This bug is currently present in three places (Left, Right, Enter handler arms for TriBool in `ConfigZone::FieldTable`). All subsequent tasks depend on the correct index being set; fixing it once in Task 0 avoids scattered notes in each later task.

- [ ] **Step 1: Add `editing_field_index` assignment to Left/Right/Enter TriBool arms**

  In `KeyCode::Left | KeyCode::BackTab` arm (line ~583), before `commit_inline_edit`:
  ```rust
  // Before:
  let new_val = tribool_cycle_back(&f.display_value);
  self.commit_inline_edit(new_val, config);

  // After:
  let new_val = tribool_cycle_back(&f.display_value);
  self.editing_field_index = self.field_vp.selected;   // ← fix stale index
  self.commit_inline_edit(new_val, config);
  ```

  In `KeyCode::Right` arm (line ~597), same fix:
  ```rust
  let new_val = tribool_cycle_fwd(&f.display_value);
  self.editing_field_index = self.field_vp.selected;   // ← fix stale index
  self.commit_inline_edit(new_val, config);
  ```

  In `KeyCode::Char('e') | KeyCode::Enter` arm (line ~614):
  ```rust
  FieldKind::TriBool => {
      let new_val = tribool_cycle_fwd(&f.display_value);
      self.editing_field_index = field_idx;             // ← fix stale index
      self.commit_inline_edit(new_val, config);
      self.config_dirty = true;
      self.pending_save = true;
      return true;
  }
  ```

- [ ] **Step 2: Build and verify**
  ```bash
  cargo build 2>&1 | grep -E "^error"
  ```
  Expected: no errors.

- [ ] **Step 3: Commit**
  ```bash
  git add src/tui/tabs/config_tab.rs
  git commit -m "fix(tui): set editing_field_index before TriBool commit to prevent stale-index write"
  ```

---

## Task 1: Bool fields toggle inline (Req 1)

**Files:**
- Modify: `src/tui/tabs/config_tab.rs:609-644` (FieldTable Enter handler)
- Modify: `src/tui/tabs/config_tab.rs:670-690` (activate_inline_edit)

Currently `Bool` fields open a text input box when Enter is pressed. They should toggle like `TriBool` but binary (true↔false), instantly saving.

- [ ] **Step 1: Exclude `Bool` from inline text input**

  In `activate_inline_edit` (line ~681), change:
  ```rust
  match &field.kind {
      FieldKind::VecString | FieldKind::VecCheckPath | FieldKind::TriBool => return false,
      _ => {}
  }
  ```
  to:
  ```rust
  match &field.kind {
      FieldKind::VecString | FieldKind::VecCheckPath | FieldKind::TriBool | FieldKind::Bool => return false,
      _ => {}
  }
  ```

- [ ] **Step 2: Add Bool toggle to FieldTable Enter/Space handler**

  In the `KeyCode::Char('e') | KeyCode::Enter` arm of `ConfigZone::FieldTable` (line ~609), before the `_ => {}` fallthrough, add after the TriBool case:
  ```rust
  FieldKind::Bool => {
      let current = &f.display_value;
      let new_val = if current == "true" { "false" } else { "true" };
      self.editing_field_index = field_idx;
      self.commit_inline_edit(new_val, config);
      self.config_dirty = true;
      self.pending_save = true;
      return true;
  }
  ```

- [ ] **Step 3: Add Space key toggle for Bool in FieldTable zone**

  In the `ConfigZone::FieldTable` match block (after `KeyCode::Right` arm, before `KeyCode::Char('e')`), add:
  ```rust
  KeyCode::Char(' ') => {
      let fields = self.current_descriptors(config);
      if let Some(f) = fields.get(self.field_vp.selected) {
          if matches!(f.kind, FieldKind::Bool) {
              let new_val = if f.display_value == "true" { "false" } else { "true" };
              self.editing_field_index = self.field_vp.selected;
              self.commit_inline_edit(new_val, config);
              self.config_dirty = true;
              self.pending_save = true;
              return true;
          }
      }
      false
  }
  ```

  > **Note:** The `editing_field_index = field_idx` assignment for `TriBool` in this handler arm was fixed in Task 0. No additional TriBool changes needed here.

- [ ] **Step 4: Build and verify**
  ```bash
  cd /Volumes/Home/Users/wen/repos/ssync
  cargo build 2>&1 | grep -E "^error"
  ```
  Expected: no errors.

- [ ] **Step 5: Commit**
  ```bash
  git add src/tui/tabs/config_tab.rs
  git commit -m "feat(tui): Bool fields toggle inline with Space/Enter in FieldTable"
  ```

---

## Task 2: Tab key cycles main tabs when navbar focused (Req 2)

**Files:**
- Modify: `src/tui/app.rs:1193-1202` (Tab/BackTab handler)

Currently when `navbar_focused = true`, Tab cycles the tab **and** drops navbar focus. The user wants Tab to keep cycling through tabs while staying in navbar.

- [ ] **Step 1: Remove `navbar_focused = false` from Tab handler when navbar is focused**

  In app.rs, the `KeyCode::Tab | KeyCode::BackTab` branch (line ~1193):
  ```rust
  KeyCode::Tab | KeyCode::BackTab => {
      let forward = key.code == KeyCode::Tab;
      if self.navbar_focused {
          self.active_tab = if forward {
              self.active_tab.next()
          } else {
              self.active_tab.prev()
          };
          self.navbar_focused = false;  // ← REMOVE THIS LINE
      } else {
  ```
  Change to:
  ```rust
  KeyCode::Tab | KeyCode::BackTab => {
      let forward = key.code == KeyCode::Tab;
      if self.navbar_focused {
          self.active_tab = if forward {
              self.active_tab.next()
          } else {
              self.active_tab.prev()
          };
          // navbar_focused stays true — Tab keeps cycling tabs
      } else {
  ```

- [ ] **Step 2: Build and verify**
  ```bash
  cargo build 2>&1 | grep -E "^error"
  ```

- [ ] **Step 3: Commit**
  ```bash
  git add src/tui/app.rs
  git commit -m "feat(tui): Tab keeps cycling main tabs while navbar is focused"
  ```

---

## Task 3: GroupPicker — Add new group name (Req 3)

**Files:**
- Modify: `src/tui/tabs/config_tab.rs:145-153` (GroupPickerState struct)
- Modify: `src/tui/tabs/config_tab.rs:1008-1052` (handle_group_picker_key)
- Modify: `src/tui/tabs/config_tab.rs:1437-1489` (render GroupPicker section)

Add `a` key in GroupPicker to type a new group name → adds to available list + auto-selects it.

- [ ] **Step 1: Extend GroupPickerState with add-input fields**

  Change struct at line ~145:
  ```rust
  pub struct GroupPickerState {
      pub field_index: usize,
      pub available: Vec<String>,
      pub checked: Vec<bool>,
      pub vp: Viewport,
      pub closing: bool,
      pub descriptions: Vec<String>,
      pub add_input: InputField,       // ← new
      pub add_input_active: bool,      // ← new
  }
  ```

- [ ] **Step 2: Update all GroupPickerState construction sites**

  There are two construction sites:
  
  **CheckEnabled picker** (line ~856):
  ```rust
  form.group_picker = Some(GroupPickerState {
      field_index: idx,
      available,
      checked,
      vp,
      closing: false,
      descriptions: CHECK_ENABLED_OPTIONS
          .iter()
          .map(|(_, d)| d.to_string())
          .collect(),
      add_input: InputField::new(""),
      add_input_active: false,
  });
  ```
  
  **Groups picker** (line ~897):
  ```rust
  form.group_picker = Some(GroupPickerState {
      field_index: idx,
      available,
      checked,
      vp,
      closing: false,
      descriptions: vec![],
      add_input: InputField::new(""),
      add_input_active: false,
  });
  ```

- [ ] **Step 3: Handle `a` key and input in handle_group_picker_key**

  > **Extract shared helper:** The add-input logic (normalize, deduplicate, insert sorted, auto-select) is identical to Task 9's `DirectGroupPickerState`. Extract it as a free function:
  > ```rust
  > fn apply_add_input_to_picker(
  >     value: &str,
  >     available: &mut Vec<String>,
  >     checked: &mut Vec<bool>,
  >     vp: &mut Viewport,
  > ) {
  >     let new_group = value.trim().to_string();
  >     if !new_group.is_empty() && !available.contains(&new_group) {
  >         let pos = available.partition_point(|g| g.as_str() < new_group.as_str());
  >         available.insert(pos, new_group);
  >         checked.insert(pos, true);
  >         vp.set_dims(available.len().max(1), 0);
  >         vp.selected = pos;
  >     }
  > }
  > ```
  > Both `handle_group_picker_key` (Task 3) and `handle_direct_group_picker_key` (Task 9) call this helper instead of duplicating the logic.

  Replace `handle_group_picker_key` (line ~1008) with:
  ```rust
  fn handle_group_picker_key(&mut self, key: KeyEvent, gp: &mut GroupPickerState) -> bool {
      // If add-input is active, route keys to it first
      if gp.add_input_active {
          gp.add_input.handle_key(key);
          if gp.add_input.mode == InputMode::Normal {
              apply_add_input_to_picker(
                  &gp.add_input.value.clone(),
                  &mut gp.available,
                  &mut gp.checked,
                  &mut gp.vp,
              );
              gp.add_input = InputField::new("");
              gp.add_input_active = false;
          }
          return true;
      }
      match key.code {
          KeyCode::Up | KeyCode::Char('k') => {
              gp.vp.move_up();
              true
          }
          KeyCode::Down | KeyCode::Char('j') => {
              gp.vp.move_down();
              true
          }
          KeyCode::Char(' ') => {
              let idx = gp.vp.selected;
              if idx < gp.checked.len() {
                  gp.checked[idx] = !gp.checked[idx];
              }
              true
          }
          KeyCode::Char('a') if gp.descriptions.is_empty() => {
              // Only allow Add for groups picker (not check enabled picker)
              gp.add_input = InputField::new("");
              gp.add_input.activate();
              gp.add_input_active = true;
              true
          }
          KeyCode::Enter | KeyCode::Char('s') => {
              let selected: Vec<String> = gp
                  .available
                  .iter()
                  .zip(gp.checked.iter())
                  .filter(|(_, &c)| c)
                  .map(|(g, _)| g.clone())
                  .collect();
              let display = if selected.is_empty() {
                  "(none)".to_string()
              } else {
                  format!("[{}]", selected.join(", "))
              };
              let fi = gp.field_index;
              if let Some(form) = self.entry_form.as_mut() {
                  form.fields[fi].display_value = display;
                  form.dirty = true;
              }
              gp.closing = true;
              true
          }
          KeyCode::Esc => {
              gp.closing = true;
              true
          }
          _ => true,
      }
  }
  ```

- [ ] **Step 4: Update GroupPicker render to show add-input and updated hint**

  In the render section for group picker (line ~1437), update the picker title for groups (no descriptions):
  ```rust
  let picker_title = if gp.descriptions.is_empty() {
      "  Pick groups  (Space:toggle  a:add  Enter/s:apply  Esc:cancel)".to_string()
  } else {
      format!(
          "  Editing: {}  (Space:toggle  Enter/s:apply  Esc:cancel)",
          form.fields[gp.field_index].key
      )
  };
  ```

  After the group list rendering block (after line ~1488), add the add-input line:
  ```rust
  if gp.add_input_active {
      lines.push(Line::from(""));
      let accent = Style::default()
          .fg(theme.accent_config)
          .add_modifier(Modifier::BOLD);
      let prefix = Span::styled("  New group: ", accent);
      let input_line = if gp.add_input.mode == InputMode::Active {
          let (before, after) = gp.add_input.split_at_cursor();
          let cursor_ch = after.chars().next().unwrap_or(' ').to_string();
          let after_cursor: String = after.chars().skip(1).collect();
          Line::from(vec![
              prefix,
              Span::styled(before, accent),
              Span::styled(
                  cursor_ch,
                  Style::default()
                      .fg(Color::Black)
                      .bg(Color::Yellow)
                      .add_modifier(Modifier::BOLD),
              ),
              Span::styled(after_cursor, accent),
          ])
      } else {
          Line::from(vec![prefix, Span::styled(gp.add_input.value.clone(), accent)])
      };
      lines.push(input_line);
  }
  ```

- [ ] **Step 5: Update is_editing_active to include group picker add-input**

  In `is_editing_active` (line ~466), add:
  ```rust
  if let Some(ref form) = self.entry_form {
      // ... existing checks ...
      if let Some(ref gp) = form.group_picker {
          if gp.add_input_active {
              return true;
          }
      }
  }
  ```

- [ ] **Step 6: Build and verify**
  ```bash
  cargo build 2>&1 | grep -E "^error"
  ```

- [ ] **Step 7: Commit**
  ```bash
  git add src/tui/tabs/config_tab.rs
  git commit -m "feat(tui): GroupPicker Add — type new group name with 'a' key"
  ```

---

## Task 4+5: Shell and Enum fields cycle with Left/Right/Enter (Req 4+5)

**Files:**
- Modify: `src/tui/tabs/config_tab.rs` — add cycle helpers, update FieldTable and entry form key handlers, exclude from text input

`ShellEnum` (sh/powershell/cmd) and `Enum` (e.g., conflict_strategy: newest/skip) should cycle with Left/Right/Enter/Space instead of opening a text input.

- [ ] **Step 1: Add cycle helper functions near line ~2229**

  After `tribool_to_opt`, add:
  ```rust
  /// Generic cycle over a fixed list of variants (forward or back).
  /// Used for ShellEnum, Enum, TriBool, and Bool to avoid separate helper functions.
  fn enum_cycle(variants: &[&str], current: &str, forward: bool) -> String {
      let pos = variants.iter().position(|&v| v == current).unwrap_or(0);
      let next = if forward {
          (pos + 1) % variants.len()
      } else {
          (pos + variants.len() - 1) % variants.len()
      };
      variants[next].to_string()
  }

  fn shell_cycle_fwd(s: &str) -> String {
      match s {
          "sh" => "powershell".to_string(),
          "powershell" => "cmd".to_string(),
          other => {
              tracing::warn!(shell = other, "unknown shell value, defaulting to sh");
              "sh".to_string()
          }
      }
  }

  fn shell_cycle_back(s: &str) -> String {
      match s {
          "sh" => "cmd".to_string(),
          "cmd" => "powershell".to_string(),
          other => {
              tracing::warn!(shell = other, "unknown shell value, defaulting to sh");
              "sh".to_string()
          }
      }
  }
  ```

  > **Unification note:** `tribool_cycle_fwd/back` can now be replaced by `enum_cycle(&["true", "false", "none"], current, true/false)`. Similarly Bool toggle becomes `enum_cycle(&["true", "false"], current, true)`. ShellEnum keeps dedicated helpers to preserve the `tracing::warn!` on unknown values. This reduces cycle-specific code to one generic function.

- [ ] **Step 2: Exclude ShellEnum and Enum from text input in activate_inline_edit**

  > **Depends on Task 1 Step 1** — Task 1 already adds `FieldKind::Bool` to this match arm. Apply this change after Task 1 is committed, so both exclusions land in the correct order.

  In `activate_inline_edit` (line ~681):
  ```rust
  match &field.kind {
      FieldKind::VecString
      | FieldKind::VecCheckPath
      | FieldKind::TriBool
      | FieldKind::Bool
      | FieldKind::ShellEnum
      | FieldKind::Enum { .. } => return false,
      _ => {}
  }
  ```

- [ ] **Step 3: Add cycle in FieldTable Left/Right handlers**

  In `ConfigZone::FieldTable`, extend the `KeyCode::Left | KeyCode::BackTab` arm (line ~583):
  ```rust
  KeyCode::Left | KeyCode::BackTab => {
      let fields = self.current_descriptors(config);
      if let Some(f) = fields.get(self.field_vp.selected) {
          match &f.kind {
              FieldKind::TriBool => {
                  let new_val = tribool_cycle_back(&f.display_value);
                  self.editing_field_index = self.field_vp.selected;
                  self.commit_inline_edit(new_val, config);
                  self.config_dirty = true;
                  self.pending_save = true;
                  return true;
              }
              FieldKind::ShellEnum => {
                  let new_val = shell_cycle_back(&f.display_value);
                  self.editing_field_index = self.field_vp.selected;
                  self.commit_inline_edit(new_val, config);
                  self.config_dirty = true;
                  self.pending_save = true;
                  return true;
              }
              FieldKind::Enum { variants } => {
                  let new_val = enum_cycle(variants, &f.display_value, false);
                  self.editing_field_index = self.field_vp.selected;
                  self.commit_inline_edit(&new_val, config);
                  self.config_dirty = true;
                  self.pending_save = true;
                  return true;
              }
              _ => {}
          }
      }
      self.zone = ConfigZone::Sidebar;
      true
  }
  ```

  > **Note:** `editing_field_index` assignments for TriBool were fixed in Task 0. The ShellEnum/Enum arms above already include the assignment, which is the same pattern.

  Extend `KeyCode::Right` arm (line ~597):
  ```rust
  KeyCode::Right => {
      let fields = self.current_descriptors(config);
      if let Some(f) = fields.get(self.field_vp.selected) {
          match &f.kind {
              FieldKind::TriBool => {
                  let new_val = tribool_cycle_fwd(&f.display_value);
                  self.editing_field_index = self.field_vp.selected;
                  self.commit_inline_edit(new_val, config);
                  self.config_dirty = true;
                  self.pending_save = true;
              }
              FieldKind::ShellEnum => {
                  let new_val = shell_cycle_fwd(&f.display_value);
                  self.editing_field_index = self.field_vp.selected;
                  self.commit_inline_edit(new_val, config);
                  self.config_dirty = true;
                  self.pending_save = true;
              }
              FieldKind::Enum { variants } => {
                  let new_val = enum_cycle(variants, &f.display_value, true);
                  self.editing_field_index = self.field_vp.selected;
                  self.commit_inline_edit(&new_val, config);
                  self.config_dirty = true;
                  self.pending_save = true;
              }
              _ => {}
          }
      }
      true
  }
  ```

  > **Note:** `editing_field_index` for the existing TriBool Right handler case was fixed in Task 0.

  Extend `KeyCode::Char('e') | KeyCode::Enter` arm — after `FieldKind::Bool` and before the `VecString` case, add:
  ```rust
  FieldKind::ShellEnum => {
      let new_val = shell_cycle_fwd(&f.display_value);
      self.editing_field_index = field_idx;
      self.commit_inline_edit(new_val, config);
      self.config_dirty = true;
      self.pending_save = true;
      return true;
  }
  FieldKind::Enum { variants } => {
      let new_val = enum_cycle(variants, &f.display_value, true);
      self.editing_field_index = field_idx;
      self.commit_inline_edit(&new_val, config);
      self.config_dirty = true;
      self.pending_save = true;
      return true;
  }
  ```

  > **Note:** `editing_field_index` for the existing TriBool Enter handler case was fixed in Task 0.

- [ ] **Step 4: Add cycle in entry form Left/Right/Enter handlers**

  In `handle_entry_form_key`, in the `KeyCode::Left` arm (line ~787), add after `TriBool`:
  ```rust
  KeyCode::Left => {
      let idx = form.field_vp.selected;
      if idx < form.fields.len() {
          match &form.fields[idx].kind.clone() {
              FieldKind::TriBool => {
                  let new_val = tribool_cycle_back(&form.fields[idx].display_value.clone());
                  form.fields[idx].display_value = new_val.to_string();
                  form.dirty = true;
                  return true;
              }
              FieldKind::ShellEnum => {
                  let new_val = shell_cycle_back(&form.fields[idx].display_value);
                  form.fields[idx].display_value = new_val.to_string();
                  form.dirty = true;
                  return true;
              }
              FieldKind::Enum { variants } => {
                  let new_val = enum_cycle(&variants.clone(), &form.fields[idx].display_value.clone(), false);
                  form.fields[idx].display_value = new_val;
                  form.dirty = true;
                  return true;
              }
              _ => {}
          }
      }
      false
  }
  ```

  Similarly extend `KeyCode::Right` arm, and add `ShellEnum`/`Enum` cases inside `KeyCode::Char('e') | KeyCode::Enter` in the entry form handler (line ~824), between the `TriBool` and `Bool` cases:
  ```rust
  FieldKind::ShellEnum => {
      let new_val = shell_cycle_fwd(&form.fields[idx].display_value);
      form.fields[idx].display_value = new_val.to_string();
      form.dirty = true;
  }
  FieldKind::Enum { variants } => {
      let new_val = enum_cycle(&variants.clone(), &form.fields[idx].display_value.clone(), true);
      form.fields[idx].display_value = new_val;
      form.dirty = true;
  }
  ```

- [ ] **Step 5: Build and verify**
  ```bash
  cargo build 2>&1 | grep -E "^error"
  ```

- [ ] **Step 6: Commit**
  ```bash
  git add src/tui/tabs/config_tab.rs
  git commit -m "feat(tui): shell and enum fields cycle with Left/Right/Enter (no text input)"
  ```

---

## Task 6: Esc cancel / type error — no "Config saved" banner (Req 6)

**Files:**
- Modify: `src/tui/tabs/config_tab.rs:648-668` (handle_inline_edit_key)

**Root cause:** When Esc is pressed during inline edit, `InputField::cancel()` sets `mode = Normal` (restored value). The caller then sees `mode == Normal` and calls `commit_inline_edit + pending_save = true`, triggering the "Config saved" banner even though no save happened.

- [ ] **Step 1: Fix handle_inline_edit_key to not save on Esc**

  Replace the function body (line ~654):
  ```rust
  fn handle_inline_edit_key(
      &mut self,
      key: KeyEvent,
      input: &mut InputField,
      config: &mut AppConfig,
  ) -> bool {
      if input.mode == InputMode::Active {
          if key.code == KeyCode::Esc {
              // Cancel: restore original value, do NOT save
              input.cancel();
              // mode is now Normal so editing_field won't be restored by caller
              return true;
          }
          input.handle_key(key);
          if input.mode == InputMode::Normal {
              // Confirmed via Enter
              self.commit_inline_edit(&input.value, config);
              self.config_dirty = true;
              self.pending_save = true;
          }
          return true;
      }
      if key.code == KeyCode::Esc {
          self.editing_field = None;
          return true;
      }
      false
  }
  ```

  The old Esc handler at the bottom (lines 663-666) is now only reached when `mode != Active`, which handles the case where the field is in an intermediate state.

- [ ] **Step 2: Build and verify**
  ```bash
  cargo build 2>&1 | grep -E "^error"
  ```

- [ ] **Step 3: Commit**
  ```bash
  git add src/tui/tabs/config_tab.rs
  git commit -m "fix(tui): Esc on inline edit cancels without triggering Config saved banner"
  ```

---

## Task 7: Preserve field_vp selection after save (Req 7)

**Files:**
- Modify: `src/tui/tabs/config_tab.rs:1082-1211` (commit_entry_form — change return type)
- Modify: `src/tui/app.rs` (save_config — consume returned index)

After pressing `s` to save an entry form popup, `reload()` is called which resets `field_vp` to index 0, losing the user's scroll position.

**Approach:** Change `commit_entry_form` to return `Option<usize>` (the field index to restore), then consume it in `save_config`. This avoids polluting `ConfigTabState` with a transient communication field.

- [ ] **Step 1: Change commit_entry_form signature to return Option\<usize\>**

  Change the function signature at line ~1082:
  ```rust
  // Before:
  fn commit_entry_form(&mut self, config: &mut AppConfig) {

  // After:
  fn commit_entry_form(&mut self, config: &mut AppConfig) -> Option<usize> {
  ```

  At the tail of the function (line ~1205), capture and return the selection:
  ```rust
  // Before:
  self.config_dirty = true;
  let items = build_sidebar_items(config);
  self.items = items;
  self.sidebar_vp = Viewport::new();
  self.sidebar_vp.set_dims(self.items.len(), 0);
  self.field_vp = Viewport::new();

  // After:
  self.config_dirty = true;
  let saved_sel = self.field_vp.selected;   // capture before reset
  let items = build_sidebar_items(config);
  self.items = items;
  self.sidebar_vp = Viewport::new();
  self.sidebar_vp.set_dims(self.items.len(), 0);
  self.field_vp = Viewport::new();
  Some(saved_sel)
  ```

- [ ] **Step 2: Update all call sites of commit_entry_form**

  Search for `self.commit_entry_form(` in `config_tab.rs`. Each call site should now capture the return value:
  ```rust
  // Before:
  self.commit_entry_form(config);

  // After:
  let _saved_sel = self.commit_entry_form(config);
  ```
  The `_saved_sel` is ignored at the call site inside `config_tab.rs` — the caller in `app.rs` is the one that uses it. If `commit_entry_form` is only called from the `pending_save` path in `app.rs`, the change is simpler (see Step 3).

- [ ] **Step 3: Restore field_vp selection in save_config**

  In app.rs `save_config` (line ~1774), after `reload()`, add the restoration:
  ```rust
  fn save_config(&mut self) {
      // ...existing save logic...
      match config_io::save_config(&self.config, path_arg) {
          Ok(resolved) => {
              self.config_tab.reload_banner_until = ...;
              self.config_tab.reload(&self.config, Some(&resolved));
              // Restore field selection if coming from entry form commit
              if let Some(saved_sel) = self.pending_field_restore.take() {
                  let count = self.config_tab.current_descriptors(&self.config).len();
                  if saved_sel < count {
                      self.config_tab.field_vp.selected = saved_sel;
                      self.config_tab.field_vp.set_dims(count, 0);
                  }
              }
          }
          Err(e) => { ... }
      }
  }
  ```

  Store the returned value on `App` before calling `save_config`. In the `pending_save` handler in app.rs (line ~1074), change:
  ```rust
  // Before:
  if self.config_tab.pending_save {
      self.config_tab.pending_save = false;
      self.save_config();
  }

  // After:
  if self.config_tab.pending_save {
      self.config_tab.pending_save = false;
      self.pending_field_restore = self.config_tab.last_committed_field_sel.take();
      self.save_config();
  }
  ```

  Add `last_committed_field_sel: Option<usize>` to `ConfigTabState` and `pending_field_restore: Option<usize>` to `App`. `commit_entry_form` writes `self.last_committed_field_sel = Some(saved_sel)` in addition to returning it.

  > **Simpler alternative:** If `commit_entry_form` is called from exactly one place (the `pending_save` guard in app.rs), pass the return value directly without the extra `App` field. Check the actual call graph before choosing.

  > **Why return value over state field:** `saved_field_sel_after_commit: Option<usize>` in `ConfigTabState` would be a transient communication channel — it holds data that is only meaningful for one tick and doesn't represent any real config-tab state. A return value makes the data flow explicit and avoids polluting the struct.

- [ ] **Step 4: Build and verify**
  ```bash
  cargo build 2>&1 | grep -E "^error"
  ```

- [ ] **Step 5: Commit**
  ```bash
  git add src/tui/tabs/config_tab.rs src/tui/app.rs
  git commit -m "fix(tui): preserve field selection after saving entry form popup"
  ```

---

## Task 8: Unified popup key blocking (Req 8)

**Files:**
- Modify: `src/tui/tabs/config_tab.rs:466-483` (is_editing_active → rename to is_any_popup_open)
- Modify: `src/tui/app.rs:1068-1079` and `1119-1163` (global key routing)

**Root cause:** `is_editing_active()` only blocks global keys when a text input is actively typing. When `entry_form` or `confirm` is open but idle, global shortcuts (`q`, `?`, `i`, `L`) still fire. Need a broader check.

- [ ] **Step 1: Add is_any_popup_open to ConfigTabState**

  After `is_editing_active()` (line ~483), add:
  ```rust
  pub fn is_any_popup_open(&self) -> bool {
      self.entry_form.is_some() || self.confirm.is_some() || self.editing_field.is_some()
  }
  ```

- [ ] **Step 2: Extend the edit guard in app.rs to cover all popup states**

  In app.rs, the edit guard section (line ~1068):
  ```rust
  // §edit-guard: while config tab has an active text input, suspend all
  // global shortcuts and route directly to the config tab.
  if self.active_tab == TabId::Config && self.config_tab.is_editing_active() {
  ```
  
  Change to:
  ```rust
  // §popup-guard: while any config popup is open, suspend all global shortcuts.
  if self.active_tab == TabId::Config && self.config_tab.is_any_popup_open() {
  ```

  The rest of the block (lines 1069-1078) stays the same — it already routes keys to config_tab and handles pending_save and pending_delete.

- [ ] **Step 3: Remove now-redundant Esc special-case**

  The block at app.rs lines ~1150-1157:
  ```rust
  // Entry form must handle Esc before the global NavBar escape below.
  if self.active_tab == TabId::Config
      && (self.config_tab.entry_form.is_some()
          || self.config_tab.confirm.is_some())
  {
      let handled = self.config_tab.handle_key(key, &mut self.config);
      return Ok(handled);
  }
  ```
  This is now covered by the broader popup guard. Remove it (replace with just a comment if needed for clarity).

- [ ] **Step 4: Update the second catch-all block (app.rs ~1341-1354)**

  There is a second catch-all in the `match key.code` block:
  ```rust
  _ if self.active_tab == TabId::Config
      && (self.config_tab.entry_form.is_some()
          || self.config_tab.confirm.is_some()) =>
  {
      self.config_tab.handle_key(key, &mut self.config);
      ...
  }
  ```
  This block handles `pending_save` and `pending_delete` for popup states. **Keep this block** but update its condition to use `is_any_popup_open()` for consistency, so it also covers `editing_field` (and later, Task 9's direct sub-popups):
  ```rust
  _ if self.active_tab == TabId::Config
      && self.config_tab.is_any_popup_open() =>
  {
      // ... body unchanged ...
  }
  ```

- [ ] **Step 5: Build and verify**
  ```bash
  cargo build 2>&1 | grep -E "^error"
  ```

- [ ] **Step 6: Commit**
  ```bash
  git add src/tui/tabs/config_tab.rs src/tui/app.rs
  git commit -m "fix(tui): block all global shortcuts when any config popup is open"
  ```

---

## Task 9: Vec/groups fields open sub-popup directly from main screen (Req 9)

**Files:**
- Modify: `src/tui/tabs/config_tab.rs` — add DirectVecEditorState/DirectGroupPickerState, handle_key, render, commit

Currently pressing Enter on a `VecString`/`VecCheckPath` field in the FieldTable opens the whole entry form, requiring the user to navigate to the field again. This task makes Enter directly open the sub-popup.

- [ ] **Step 1: Add DirectVecEditorState and DirectGroupPickerState structs**

  After the `GroupPickerState` struct (line ~153), add:
  ```rust
  #[derive(Debug)]
  pub struct DirectVecEditorState {
      pub field_key: String,
      pub sidebar_item: SidebarItem,
      pub items: Vec<String>,
      pub vp: Viewport,
      pub input: InputField,
      pub input_active: bool,
  }

  #[derive(Debug)]
  pub struct DirectGroupPickerState {
      pub field_key: String,
      pub sidebar_item: SidebarItem,
      pub available: Vec<String>,
      pub checked: Vec<bool>,
      pub vp: Viewport,
      pub closing: bool,
      pub add_input: InputField,
      pub add_input_active: bool,
  }
  ```

- [ ] **Step 2: Add fields to ConfigTabState**

  In `ConfigTabState` struct (near line ~290):
  ```rust
  pub direct_vec_editor: Option<DirectVecEditorState>,
  pub direct_group_picker: Option<DirectGroupPickerState>,
  ```
  
  In `ConfigTabState::new()`, initialize both to `None`.

- [ ] **Step 3: Replace VecString/VecCheckPath Enter handler in FieldTable**

  In `handle_key` `ConfigZone::FieldTable`, the `KeyCode::Char('e') | KeyCode::Enter` arm, replace the `VecString | VecCheckPath` case (line ~621-636):
  ```rust
  FieldKind::VecString | FieldKind::VecCheckPath | FieldKind::CheckEnabled => {
      let field_key = f.key.clone();
      let current_val = f.display_value.clone();
      let sidebar_item = self.items[self.sidebar_vp.selected].clone();
      if field_key == "groups" || matches!(f.kind, FieldKind::VecCheckPath | FieldKind::CheckEnabled) {
          // Open DirectGroupPicker for groups (and check enabled)
          let mut known: std::collections::BTreeSet<String> = config
              .host.iter().flat_map(|h| h.groups.iter().cloned())
              .chain(config.check.iter().flat_map(|c| c.groups.iter().cloned()))
              .chain(config.sync.iter().flat_map(|s| s.groups.iter().cloned()))
              .collect();
          let current = parse_bracket_list(&current_val);
          for item in &current { known.insert(item.clone()); }
          let available: Vec<String> = known.into_iter().collect();
          let checked: Vec<bool> = available.iter().map(|g| current.contains(g)).collect();
          let mut vp = Viewport::new();
          vp.set_dims(available.len().max(1), 0);
          self.direct_group_picker = Some(DirectGroupPickerState {
              field_key,
              sidebar_item,
              available,
              checked,
              vp,
              closing: false,
              add_input: InputField::new(""),
              add_input_active: false,
          });
      } else {
          // Open DirectVecEditor for paths, skipped_hosts, etc.
          let items = parse_bracket_list(&current_val);
          let mut vp = Viewport::new();
          vp.set_dims(items.len().max(1), 0);
          self.direct_vec_editor = Some(DirectVecEditorState {
              field_key,
              sidebar_item,
              items,
              vp,
              input: InputField::new(""),
              input_active: false,
          });
      }
      return true;
  }
  ```

  > **CheckEnabled:** `FieldKind::CheckEnabled` is included in the match so that `check.enabled` fields also open a group-picker directly instead of the full entry form. If `CheckEnabled` should continue to use the entry form, remove it from the match.

- [ ] **Step 4: Route keys to direct sub-popups in handle_key**

  At the top of `handle_key` (line ~493), before the `entry_form` check, add:
  ```rust
  if self.direct_group_picker.is_some() {
      return self.handle_direct_group_picker_key(key, config);
  }
  if self.direct_vec_editor.is_some() {
      return self.handle_direct_vec_editor_key(key, config);
  }
  ```

- [ ] **Step 5: Implement handle_direct_vec_editor_key**

  ```rust
  fn handle_direct_vec_editor_key(&mut self, key: KeyEvent, config: &mut AppConfig) -> bool {
      let ve = self.direct_vec_editor.as_mut().unwrap();
      if ve.input_active {
          ve.input.handle_key(key);
          if ve.input.mode == InputMode::Normal {
              if !ve.input.value.is_empty() {
                  ve.items.push(std::mem::take(&mut ve.input.value));
                  ve.vp.set_dims(ve.items.len().max(1), 0);
              }
              ve.input_active = false;
          }
          return true;
      }
      match key.code {
          KeyCode::Up | KeyCode::Char('k') => { ve.vp.move_up(); true }
          KeyCode::Down | KeyCode::Char('j') => { ve.vp.move_down(); true }
          KeyCode::Char('a') | KeyCode::Enter => {
              let ve = self.direct_vec_editor.as_mut().unwrap();
              ve.input = InputField::new("");
              ve.input.activate();
              ve.input_active = true;
              true
          }
          KeyCode::Char('d') => {
              let ve = self.direct_vec_editor.as_mut().unwrap();
              let idx = ve.vp.selected;
              if idx < ve.items.len() {
                  ve.items.remove(idx);
                  ve.vp.set_dims(ve.items.len().max(1), 0);
                  if ve.vp.selected >= ve.items.len() && ve.vp.selected > 0 {
                      ve.vp.move_up();
                  }
              }
              true
          }
          KeyCode::Char('s') => {
              self.commit_direct_vec_editor(config);
              self.direct_vec_editor = None;
              self.pending_save = true;
              true
          }
          KeyCode::Esc => {
              self.direct_vec_editor = None;
              true
          }
          _ => true,
      }
  }
  ```

- [ ] **Step 6: Implement commit_direct_vec_editor**

  ```rust
  fn commit_direct_vec_editor(&mut self, config: &mut AppConfig) {
      let ve = match self.direct_vec_editor.as_ref() {
          Some(v) => v,
          None => return,
      };
      let key = ve.field_key.clone();
      let item = ve.sidebar_item.clone();
      self.config_dirty = true;
      let items = ve.items.clone();
      match item {
          SidebarItem::SectionSettings => {
              match key.as_str() {
                  "skipped_hosts" => config.settings.skipped_hosts = items,
                  other => tracing::warn!(key = other, "commit_direct_vec_editor: unhandled SectionSettings key"),
              }
          }
          SidebarItem::Host(i) => {
              if let Some(h) = config.host.get_mut(i) {
                  match key.as_str() {
                      "groups" => h.groups = items,
                      other => tracing::warn!(key = other, "commit_direct_vec_editor: unhandled Host key"),
                  }
              }
          }
          SidebarItem::Check(i) => {
              if let Some(c) = config.check.get_mut(i) {
                  match key.as_str() {
                      "groups" => c.groups = items,
                      "enabled" => c.enabled = items,
                      other => tracing::warn!(key = other, "commit_direct_vec_editor: unhandled Check key"),
                  }
              }
          }
          SidebarItem::Sync(i) => {
              if let Some(s) = config.sync.get_mut(i) {
                  match key.as_str() {
                      "groups" => s.groups = items,
                      "paths" => s.paths = items,
                      other => tracing::warn!(key = other, "commit_direct_vec_editor: unhandled Sync key"),
                  }
              }
          }
          other => tracing::warn!(item = ?other, "commit_direct_vec_editor: unhandled SidebarItem variant"),
      }
  }
  ```

- [ ] **Step 7: Implement handle_direct_group_picker_key and commit_direct_group_picker**

  > **Uses shared helper** `apply_add_input_to_picker` defined in Task 3 Step 3 — call it instead of duplicating the insert logic.

  ```rust
  fn handle_direct_group_picker_key(&mut self, key: KeyEvent, config: &mut AppConfig) -> bool {
      let gp = self.direct_group_picker.as_mut().unwrap();
      if gp.add_input_active {
          gp.add_input.handle_key(key);
          if gp.add_input.mode == InputMode::Normal {
              apply_add_input_to_picker(
                  &gp.add_input.value.clone(),
                  &mut gp.available,
                  &mut gp.checked,
                  &mut gp.vp,
              );
              gp.add_input = InputField::new("");
              gp.add_input_active = false;
          }
          return true;
      }
      match key.code {
          KeyCode::Up | KeyCode::Char('k') => { self.direct_group_picker.as_mut().unwrap().vp.move_up(); true }
          KeyCode::Down | KeyCode::Char('j') => { self.direct_group_picker.as_mut().unwrap().vp.move_down(); true }
          KeyCode::Char(' ') => {
              let gp = self.direct_group_picker.as_mut().unwrap();
              let idx = gp.vp.selected;
              if idx < gp.checked.len() { gp.checked[idx] = !gp.checked[idx]; }
              true
          }
          KeyCode::Char('a') => {
              let gp = self.direct_group_picker.as_mut().unwrap();
              gp.add_input = InputField::new("");
              gp.add_input.activate();
              gp.add_input_active = true;
              true
          }
          KeyCode::Enter | KeyCode::Char('s') => {
              self.commit_direct_group_picker(config);
              self.direct_group_picker = None;
              self.pending_save = true;
              true
          }
          KeyCode::Esc => {
              self.direct_group_picker = None;
              true
          }
          _ => true,
      }
  }

  fn commit_direct_group_picker(&mut self, config: &mut AppConfig) {
      let gp = match self.direct_group_picker.as_ref() {
          Some(g) => g,
          None => return,
      };
      let selected: Vec<String> = gp.available.iter()
          .zip(gp.checked.iter())
          .filter(|(_, &c)| c)
          .map(|(g, _)| g.clone())
          .collect();
      let key = gp.field_key.clone();
      let item = gp.sidebar_item.clone();
      self.config_dirty = true;
      match item {
          SidebarItem::Host(i) => {
              if let Some(h) = config.host.get_mut(i) {
                  if key == "groups" { h.groups = selected; }
              }
          }
          SidebarItem::Check(i) => {
              if let Some(c) = config.check.get_mut(i) {
                  match key.as_str() {
                      "groups" => c.groups = selected,
                      "enabled" => c.enabled = selected,
                      _ => {}
                  }
              }
          }
          SidebarItem::Sync(i) => {
              if let Some(s) = config.sync.get_mut(i) {
                  if key == "groups" { s.groups = selected; }
              }
          }
          _ => {}
      }
  }
  ```

- [ ] **Step 8: Add rendering for direct sub-popups**

  In `render()` (line ~1353), before the existing `entry_form` check, add:
  ```rust
  // Direct sub-popup overlays (req 9: vec fields open sub-popup without entry form)
  if self.direct_vec_editor.is_some() || self.direct_group_picker.is_some() {
      // Render the main screen underneath
      let vert = Layout::default()
          .direction(Direction::Vertical)
          .constraints([Constraint::Min(0), Constraint::Length(1)])
          .split(area);
      let horiz = Layout::default()
          .direction(Direction::Horizontal)
          .constraints([Constraint::Length(22), Constraint::Min(0)])
          .split(vert[0]);
      self.render_sidebar(horiz[0], frame, theme, config, false);
      self.render_field_table(horiz[1], frame, theme, config, false);
      let crumb = self.breadcrumb(config);
      frame.render_widget(
          Paragraph::new(Span::styled(crumb, Style::default().fg(theme.inactive))),
          vert[1],
      );
      // Render popup overlay
      if let Some(ref dve) = self.direct_vec_editor {
          self.render_direct_vec_editor(area, frame, theme, dve);
      } else if let Some(ref dgp) = self.direct_group_picker {
          self.render_direct_group_picker(area, frame, theme, dgp);
      }
      return;
  }
  ```

- [ ] **Step 9: Implement render_direct_vec_editor and render_direct_group_picker**

  > **Extract cursor rendering helper** — the cursor character rendering pattern below appears identically in Task 3's GroupPicker add-input render and here (4+ total occurrences). Extract it before implementing these renderers:
  > ```rust
  > /// Renders an active InputField with a block-cursor character into a Vec<Line>.
  > /// `prefix` is the label span shown before the input content.
  > fn input_cursor_line<'a>(input: &InputField, prefix: Span<'a>, style: Style) -> Line<'a> {
  >     let (before, after) = input.split_at_cursor();
  >     let cursor_ch = after.chars().next().unwrap_or(' ').to_string();
  >     let after_cursor: String = after.chars().skip(1).collect();
  >     Line::from(vec![
  >         prefix,
  >         Span::styled(before, style),
  >         Span::styled(
  >             cursor_ch,
  >             Style::default()
  >                 .fg(Color::Black)
  >                 .bg(Color::Yellow)
  >                 .add_modifier(Modifier::BOLD),
  >         ),
  >         Span::styled(after_cursor, style),
  >     ])
  > }
  > ```
  > Then replace all inline cursor-rendering blocks with `input_cursor_line(...)`. Update Task 3 Step 4 render as well.

  These mirror the existing `render_entry_form` popup structure but show only the vec/group-picker content.

  ```rust
  fn render_direct_vec_editor(
      &self, area: Rect, frame: &mut Frame, theme: &Theme, dve: &DirectVecEditorState,
  ) {
      let popup_area = centered_rect(60, 60, area);
      frame.render_widget(Clear, popup_area);
      let title = format!(" Edit: {} ", dve.field_key);
      let block = Block::default()
          .borders(Borders::ALL)
          .border_style(Style::default().fg(theme.accent_config))
          .title(title.as_str());
      let inner = block.inner(popup_area);
      frame.render_widget(block, popup_area);

      let visible_h = inner.height as usize;
      let mut vp = dve.vp.clone();
      vp.set_dims(dve.items.len().max(1), visible_h.saturating_sub(3));
      let (vs, ve_end) = vp.visible_range();

      let mut lines: Vec<Line> = vec![
          Line::from(Span::styled(
              format!("  (a:add  d:del  s:save  Esc:cancel)"),
              Style::default().fg(theme.warning),
          )),
          Line::from(""),
      ];
      for (rel, item) in dve.items[vs..ve_end].iter().enumerate() {
          let abs = vs + rel;
          let is_sel = abs == vp.selected;
          let style = if is_sel {
              Style::default().fg(theme.accent_config).add_modifier(Modifier::BOLD | Modifier::REVERSED)
          } else { Style::default() };
          let prefix = if is_sel { "▶ " } else { "  " };
          lines.push(Line::from(Span::styled(format!("{prefix}{item}"), style)));
      }
      if dve.items.is_empty() {
          lines.push(Line::from(Span::styled("  (empty)", Style::default().fg(theme.inactive))));
      }
      if dve.input_active {
          lines.push(Line::from(""));
          let accent = Style::default().fg(theme.accent_config).add_modifier(Modifier::BOLD);
          let prefix = Span::styled("  New: ", accent);
          let (before, after) = dve.input.split_at_cursor();
          let cursor_ch = after.chars().next().unwrap_or(' ').to_string();
          let after_cursor: String = after.chars().skip(1).collect();
          lines.push(Line::from(vec![
              prefix,
              Span::styled(before, accent),
              Span::styled(cursor_ch, Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
              Span::styled(after_cursor, accent),
          ]));
      }
      frame.render_widget(Paragraph::new(lines), inner);
  }

  fn render_direct_group_picker(
      &self, area: Rect, frame: &mut Frame, theme: &Theme, dgp: &DirectGroupPickerState,
  ) {
      let popup_area = centered_rect(60, 70, area);
      frame.render_widget(Clear, popup_area);
      let title = format!(" Pick groups: {} ", dgp.field_key);
      let block = Block::default()
          .borders(Borders::ALL)
          .border_style(Style::default().fg(theme.accent_config))
          .title(title.as_str());
      let inner = block.inner(popup_area);
      frame.render_widget(block, popup_area);

      let visible_h = inner.height as usize;
      let mut vp = dgp.vp.clone();
      let extra = if dgp.add_input_active { 4 } else { 2 };
      vp.set_dims(dgp.available.len().max(1), visible_h.saturating_sub(extra + 2));
      let (gs, ge) = vp.visible_range();

      let mut lines: Vec<Line> = vec![
          Line::from(Span::styled(
              "  (Space:toggle  a:add  Enter/s:apply  Esc:cancel)",
              Style::default().fg(theme.warning),
          )),
          Line::from(""),
      ];
      if dgp.available.is_empty() {
          lines.push(Line::from(Span::styled("  (no known groups)", Style::default().fg(theme.inactive))));
      } else {
          for (rel, group) in dgp.available[gs..ge].iter().enumerate() {
              let abs = gs + rel;
              let is_sel = abs == vp.selected;
              let checked = dgp.checked.get(abs).copied().unwrap_or(false);
              let mark = if checked { "◉" } else { "○" };
              let style = if is_sel {
                  Style::default().fg(theme.accent_config).add_modifier(Modifier::BOLD | Modifier::REVERSED)
              } else { Style::default() };
              lines.push(Line::from(Span::styled(format!("  {mark} {group}"), style)));
          }
      }
      if dgp.add_input_active {
          lines.push(Line::from(""));
          let accent = Style::default().fg(theme.accent_config).add_modifier(Modifier::BOLD);
          let prefix = Span::styled("  New group: ", accent);
          let (before, after) = dgp.add_input.split_at_cursor();
          let cursor_ch = after.chars().next().unwrap_or(' ').to_string();
          let after_cursor: String = after.chars().skip(1).collect();
          lines.push(Line::from(vec![
              prefix,
              Span::styled(before, accent),
              Span::styled(cursor_ch, Style::default().fg(Color::Black).bg(Color::Yellow).add_modifier(Modifier::BOLD)),
              Span::styled(after_cursor, accent),
          ]));
      }
      frame.render_widget(Paragraph::new(lines), inner);
  }
  ```

- [ ] **Step 10: Update is_any_popup_open to include direct sub-popups**

  ```rust
  pub fn is_any_popup_open(&self) -> bool {
      self.entry_form.is_some()
          || self.confirm.is_some()
          || self.editing_field.is_some()
          || self.direct_vec_editor.is_some()
          || self.direct_group_picker.is_some()
  }
  ```

  Also update `is_editing_active()` to include direct sub-popup input states.

  > **Note:** `is_editing_active` is modified twice — once in Task 3 Step 5 (adds `group_picker.add_input_active` check for the entry-form's GroupPicker) and once here for the direct sub-popups. The version below is the **final state** combining both:
  ```rust
  pub fn is_editing_active(&self) -> bool {
      // ... existing checks for editing_field, entry_form inline input ...
      if let Some(ref form) = self.entry_form {
          if let Some(ref gp) = form.group_picker {
              if gp.add_input_active { return true; }  // ← added by Task 3 Step 5
          }
      }
      if let Some(ref dve) = self.direct_vec_editor {
          if dve.input_active { return true; }          // ← added by Task 9 Step 10
      }
      if let Some(ref dgp) = self.direct_group_picker {
          if dgp.add_input_active { return true; }      // ← added by Task 9 Step 10
      }
      false
  }
  ```

- [ ] **Step 11: Verify app.rs catch-all covers direct sub-popups**

  The catch-all block updated in Task 8 Step 4 already uses `is_any_popup_open()`, which now includes `direct_vec_editor` / `direct_group_picker`. Verify the `pending_save` flow explicitly:

  In app.rs, the popup guard (line ~1068):
  ```rust
  if self.active_tab == TabId::Config && self.config_tab.is_any_popup_open() {
      let handled = self.config_tab.handle_key(key, &mut self.config);
      if self.config_tab.pending_save {      // ← this check must be present
          self.config_tab.pending_save = false;
          self.save_config();
      }
      // ... pending_delete check ...
      return Ok(handled);
  }
  ```

  `handle_direct_vec_editor_key` sets `self.pending_save = true` on `KeyCode::Char('s')`. The guard above then calls `save_config()`. Confirm these two lines are indeed present in the guard body — if the guard body only contains `return Ok(handled)` without the `pending_save` check, add it.

- [ ] **Step 12: Build and verify**
  ```bash
  cargo build 2>&1 | grep -E "^error"
  ```

- [ ] **Step 13: Commit**
  ```bash
  git add src/tui/tabs/config_tab.rs
  git commit -m "feat(tui): Vec/groups fields open sub-popup directly from main Config screen"
  ```

> **Code duplication note:** `DirectVecEditorState` / `DirectGroupPickerState` and their handlers/renderers share logic with the entry form's `VecEditorState` / `GroupPickerState`. This plan mitigates the duplication via the `apply_add_input_to_picker` helper (Task 3 Step 3) and the `input_cursor_line` helper (Task 9 Step 9). A deeper refactor into a shared trait or generic popup module can follow once all 9 fixes are stable.

---

## Final: Changelog + verify

- [ ] **Append to CHANGELOG.md**

  Add under `## [Unreleased]`:
  ```markdown
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
  ```

- [ ] **Run Clippy (both feature configs) — must pass before final build**
  ```bash
  cargo clippy --all-targets -- -D warnings 2>&1 | head -60
  cargo clippy --all-targets --features tui -- -D warnings 2>&1 | head -60
  ```

- [ ] **Run fmt check**
  ```bash
  cargo fmt --check
  ```

- [ ] **Final build check**
  ```bash
  cargo build 2>&1 | grep -E "^error|^warning.*unused"
  cargo build --features tui 2>&1 | grep -E "^error|^warning.*unused"
  ```

---

## Manual Test Checklist

Since TUI code is difficult to unit-test, verify each feature manually after all tasks land:

- [ ] **Task 0 — TriBool stale index:** Edit a String field at index 3 → confirm → navigate to a TriBool at index 7 → press Left/Right/Enter → verify the TriBool field (not field 3) changes value
- [ ] **Task 1 — Bool toggle:** Config tab → select a host → navigate to `sudo` or `sudo_password` (any Bool field) → press Enter → value toggles true↔false → press Space → toggles again → no text input opens
- [ ] **Task 2 — Tab cycling:** Focus navbar (press Esc from sidebar) → press Tab → tab cycles, navbar stays focused → press BackTab → cycles back
- [ ] **Task 3 — GroupPicker Add:** Open entry form on a host → navigate to `groups` → press Enter → press `a` → type group name → press Enter → new group appears in list, checked
- [ ] **Task 4+5 — Shell/Enum cycling:** Config tab → select host with `shell` field → press Left/Right → cycles sh↔powershell↔cmd → press Enter → cycles forward → no text input. Similarly test `conflict_strategy` in Settings
- [ ] **Task 6 — Esc cancel banner:** Start editing a text field (e.g. `hostname`) → type something → press Esc → "Config saved" banner should NOT appear → value restored
- [ ] **Task 7 — Focus preservation:** Open entry form → navigate to field 5 → press `s` to save → after save completes, field table should still highlight row 5 (not reset to 0)
- [ ] **Task 8 — Popup key blocking:** Open entry form (idle, no input active) → press `q` → should NOT quit → press `?` → should NOT open help → press Esc to close popup → then `q` works again
- [ ] **Task 9 — Direct sub-popup:** Config tab → select host → navigate to `groups` field → press Enter → group picker popup opens directly (no full entry form) → toggle groups → press `s` → saves and returns to field table
- [ ] **Task 9 — Direct vec editor:** Config tab → select a sync entry → navigate to `paths` field → press Enter → vec editor popup opens directly → press `a` → add path → press `d` → delete → press `s` → saves
- [ ] **Regression — TriBool editing_field_index (covered by Task 0 test above)**
