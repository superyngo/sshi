# TUI UX Popup Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix 7 TUI UX issues in the config tab: unify status message position, fix confirm dialog routing bug, unify sub-popup key behaviour, add input cursor in vec_editor, add Bool Space-toggle, and change `check.enabled` to a fixed multi-select picker.

**Architecture:** All changes are confined to `src/tui/tabs/config_tab.rs`. The file contains `ConfigTab` state + key handlers + renderers. Changes follow existing patterns (GroupPickerState for multi-select, InputField cursor rendering). No new files needed.

**Tech Stack:** Rust, ratatui 0.29, crossterm

---

## File Map

| File | Changes |
|------|---------|
| `src/tui/tabs/config_tab.rs` | All 7 fixes |

---

### Task 1: Move `✓ Config saved` banner to bottom status area

**Problem:** `✓ Config saved` appears as a top-of-content banner (`vert[0]`). Error messages appear at `render_status_bar()` → `chunks[0]` (bottom). They should be in the same location.

**Fix:** Remove the top banner layout from `ConfigTab::render()`. Instead, expose `reload_banner_until` to `App::render_config()`, which already calls `render_status_bar`. Show the success message in `render_status_bar` (`chunks[0]`) when `config_tab.reload_banner_until` is set and unexpired — overriding the error slot.

**Files:**
- Modify: `src/tui/tabs/config_tab.rs` (ConfigTab::render, render_config in app.rs)

- [ ] **Step 1: Remove top banner from ConfigTab::render()**

In `config_tab.rs` around line 1303, replace the 3-constraint layout + banner paragraph:

```rust
// BEFORE (lines ~1303–1320):
let banner_active = self
    .reload_banner_until
    .map(|t| Instant::now() < t)
    .unwrap_or(false);
let banner_h: u16 = if banner_active { 1 } else { 0 };

let vert = Layout::default()
    .direction(Direction::Vertical)
    .constraints([
        Constraint::Length(banner_h),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

if banner_active {
    let p = Paragraph::new(Span::styled(
        "  ✓ Config saved",
        Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(p, vert[0]);
}

let horiz = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Length(22), Constraint::Min(0)])
    .split(vert[1]);

self.render_sidebar(horiz[0], frame, theme, config, navbar_focused);
self.render_field_table(horiz[1], frame, theme, config, navbar_focused);

let crumb = self.breadcrumb(config);
let dirty_star = if self.config_dirty { " *" } else { "" };
let path_hint = config_path
    .map(|p| format!("  [{}]", p.display()))
    .unwrap_or_default();
let crumb_line = Line::from(vec![
    Span::styled(crumb, Style::default().fg(theme.inactive)),
    Span::styled(
        dirty_star.to_string(),
        Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
    ),
    Span::styled(path_hint, Style::default().fg(theme.border_inactive)),
]);
frame.render_widget(Paragraph::new(crumb_line), vert[2]);
```

Replace with (2-constraint layout, no top banner):

```rust
// AFTER:
let vert = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Min(0), Constraint::Length(1)])
    .split(area);

let horiz = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Length(22), Constraint::Min(0)])
    .split(vert[0]);

self.render_sidebar(horiz[0], frame, theme, config, navbar_focused);
self.render_field_table(horiz[1], frame, theme, config, navbar_focused);

let crumb = self.breadcrumb(config);
let dirty_star = if self.config_dirty { " *" } else { "" };
let path_hint = config_path
    .map(|p| format!("  [{}]", p.display()))
    .unwrap_or_default();
let crumb_line = Line::from(vec![
    Span::styled(crumb, Style::default().fg(theme.inactive)),
    Span::styled(
        dirty_star.to_string(),
        Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
    ),
    Span::styled(path_hint, Style::default().fg(theme.border_inactive)),
]);
frame.render_widget(Paragraph::new(crumb_line), vert[1]);
```

- [ ] **Step 2: Add `banner_active()` helper to ConfigTab**

After the `render` function, add:

```rust
pub fn banner_active(&self) -> bool {
    self.reload_banner_until
        .map(|t| std::time::Instant::now() < t)
        .unwrap_or(false)
}
```

- [ ] **Step 3: Show saved banner in render_status_bar (app.rs)**

In `app.rs::render_status_bar()` (~line 2383), replace:

```rust
if let Some(err) = &self.error {
    let p = Paragraph::new(err.as_str()).style(Style::default().fg(self.theme.error));
    frame.render_widget(p, chunks[0]);
}
```

With:

```rust
if self.active_tab == TabId::Config && self.config_tab.banner_active() {
    let p = Paragraph::new("  ✓ Config saved")
        .style(Style::default().fg(self.theme.warning).add_modifier(Modifier::BOLD));
    frame.render_widget(p, chunks[0]);
} else if let Some(err) = &self.error {
    let p = Paragraph::new(err.as_str()).style(Style::default().fg(self.theme.error));
    frame.render_widget(p, chunks[0]);
}
```

- [ ] **Step 4: Build and verify**

```bash
cargo build 2>&1 | head -30
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src/tui/tabs/config_tab.rs src/tui/app.rs
git commit -m "fix: move Config saved banner to bottom status bar"
```

---

### Task 2: Fix confirm dialog y/n routing when entry_form is open

**Problem:** `handle_key()` checks `entry_form.is_some()` before `confirm.is_some()`. When a confirm dialog is triggered from within `handle_entry_form_key()` (e.g. Esc on dirty form), subsequent keypresses still route to `handle_entry_form_key()` which does not check `self.confirm`. Result: y/n have no effect.

**Fix:** Add a `self.confirm` check at the very top of `handle_entry_form_key()`.

**Files:**
- Modify: `src/tui/tabs/config_tab.rs:699`

- [ ] **Step 1: Add confirm check at top of handle_entry_form_key()**

At line 699, `handle_entry_form_key` currently starts:

```rust
fn handle_entry_form_key(&mut self, key: KeyEvent, config: &mut AppConfig) -> bool {
    if self
        .entry_form
        .as_ref()
        .map(|f| f.vec_editor.is_some())
        .unwrap_or(false)
    {
```

Insert before the first `if`:

```rust
fn handle_entry_form_key(&mut self, key: KeyEvent, config: &mut AppConfig) -> bool {
    // Confirm dialog overlays the form — route to it first.
    if self.confirm.is_some() {
        return self.handle_confirm_key(key);
    }

    if self
        .entry_form
        .as_ref()
        .map(|f| f.vec_editor.is_some())
        .unwrap_or(false)
    {
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | head -20
```

- [ ] **Step 3: Commit**

```bash
git add src/tui/tabs/config_tab.rs
git commit -m "fix: route confirm y/n when entry_form is open"
```

---

### Task 3: Unify sub-popup key capture (vec_editor & group_picker)

**Problem:** `handle_vec_editor_key()` and `handle_group_picker_key()` return `false` for unhandled keys (e.g. `q`, `'s'`). This lets global handlers in `app.rs` fire (q → quit, etc.). Sub-popups must capture all keys they don't explicitly forward.

Additionally: vec_editor has no `'s'` commit path; group_picker's `'s'` is already handled but `_` returns `false`.

**Files:**
- Modify: `src/tui/tabs/config_tab.rs:874` (`handle_vec_editor_key`)
- Modify: `src/tui/tabs/config_tab.rs:930` (`handle_group_picker_key`)

- [ ] **Step 1: Add 's' commit + swallow unknown keys in handle_vec_editor_key()**

Find `handle_vec_editor_key` (~line 887). The current match is:

```rust
match key.code {
    KeyCode::Up | KeyCode::Char('k') => { ... }
    KeyCode::Down | KeyCode::Char('j') => { ... }
    KeyCode::Char('a') | KeyCode::Enter => { ... }
    KeyCode::Char('d') => { ... }
    KeyCode::Esc => { ... }
    _ => false,
}
```

Replace the `KeyCode::Esc` arm and `_ => false` with:

```rust
            KeyCode::Char('s') | KeyCode::Esc => {
                // Both 's' and Esc commit the vec_editor back to the form field.
                let display = if ve.items.is_empty() {
                    "(none)".to_string()
                } else {
                    format!("[{}]", ve.items.join(", "))
                };
                let idx = ve.field_index;
                let form = self.entry_form.as_mut().unwrap();
                form.fields[idx].display_value = display;
                form.dirty = true;
                form.vec_editor = None;
                true
            }
            _ => true, // swallow — prevent global keys (q, ?) from firing
        }
```

- [ ] **Step 2: Swallow unknown keys in handle_group_picker_key()**

Find `handle_group_picker_key` (~line 930). Change the final `_ => false` to `_ => true`.

- [ ] **Step 3: Build**

```bash
cargo build 2>&1 | head -20
```

- [ ] **Step 4: Commit**

```bash
git add src/tui/tabs/config_tab.rs
git commit -m "fix: sub-popups capture all keys; vec_editor gains s-commit"
```

---

### Task 4: Show cursor in vec_editor New: input line

**Problem:** When `ve.input_active` is true, the line renders as `"  New: {ve.input.value}"` with no visible cursor.

**Fix:** Render the cursor the same way as `render_field_table` does for inline edits — use `InputField::split_at_cursor()` to split the string and highlight the cursor character.

**Files:**
- Modify: `src/tui/tabs/config_tab.rs` (~line 1441, inside `render_entry_form`)

- [ ] **Step 1: Replace the New: text rendering with cursor-aware version**

Find the block around line 1441:

```rust
if ve.input_active {
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("  New: {}", ve.input.value),
        Style::default().fg(theme.accent_config),
    )));
}
```

Replace with:

```rust
if ve.input_active {
    lines.push(Line::from(""));
    let accent = Style::default().fg(theme.accent_config).add_modifier(Modifier::BOLD);
    let prefix = Span::styled("  New: ", accent);
    let input_line = if ve.input.mode == crate::tui::components::input_field::InputMode::Active {
        let (before, after) = ve.input.split_at_cursor();
        let cursor_ch = after.chars().next().unwrap_or(' ').to_string();
        let after_cursor: String = after.chars().skip(1).collect();
        Line::from(vec![
            prefix,
            Span::styled(before, accent),
            Span::styled(
                cursor_ch,
                Style::default()
                    .fg(ratatui::style::Color::Black)
                    .bg(ratatui::style::Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(after_cursor, accent),
        ])
    } else {
        Line::from(vec![
            prefix,
            Span::styled(ve.input.value.clone(), accent),
        ])
    };
    lines.push(input_line);
}
```

- [ ] **Step 2: Build**

```bash
cargo build 2>&1 | head -20
```

- [ ] **Step 3: Commit**

```bash
git add src/tui/tabs/config_tab.rs
git commit -m "fix: show cursor in vec_editor New: input line"
```

---

### Task 5: Bool fields — Space to toggle, update hints

**Problem:** `FieldKind::Bool` currently activates a text input (enter "true"/"false"). Should toggle with Space bar like TriBool uses Left/Right.

**Fix:** Handle `KeyCode::Char(' ')` for Bool in both `handle_entry_form_key()` (form context) and update hints to show `[Space] Toggle`.

**Files:**
- Modify: `src/tui/tabs/config_tab.rs`

- [ ] **Step 1: Add Space toggle in handle_entry_form_key() form field match**

In the `KeyCode::Char('e') | KeyCode::Enter` arm (~line 781), the Bool kind falls through to `activate_inline_edit`. Replace that fall-through by handling Bool explicitly:

Inside the `if field.editable { match &field.kind { ... } }` block, add before the `_ =>` arm:

```rust
FieldKind::Bool => {
    let toggled = if form.fields[idx].display_value == "true" {
        "false"
    } else {
        "true"
    };
    form.fields[idx].display_value = toggled.to_string();
    form.dirty = true;
}
```

- [ ] **Step 2: Also handle Space key for Bool in the top-level match in handle_entry_form_key()**

Add a new arm in the `match key.code` block (after the `Right` arm, before the `Char('e') | Enter` arm):

```rust
KeyCode::Char(' ') => {
    let idx = form.field_vp.selected;
    if idx < form.fields.len() {
        let field = &form.fields[idx];
        if field.editable && matches!(field.kind, FieldKind::Bool) {
            let toggled = if form.fields[idx].display_value == "true" {
                "false"
            } else {
                "true"
            };
            form.fields[idx].display_value = toggled.to_string();
            form.dirty = true;
            return true;
        }
    }
    false
}
```

- [ ] **Step 3: Update form hints line to include Space**

Around line 1494 in `render_entry_form`:

```rust
// BEFORE:
lines.push(Line::from(Span::styled(
    "  [Enter/e] Edit field  [s] Save  [Esc] Cancel",
    Style::default().fg(theme.inactive),
)));
```

Replace with:

```rust
// AFTER:
lines.push(Line::from(Span::styled(
    "  [Enter/e] Edit  [Space] Toggle bool  [s] Save  [Esc] Cancel",
    Style::default().fg(theme.inactive),
)));
```

- [ ] **Step 4: Build**

```bash
cargo build 2>&1 | head -20
```

- [ ] **Step 5: Commit**

```bash
git add src/tui/tabs/config_tab.rs
git commit -m "feat: Bool fields toggle with Space in entry form"
```

---

### Task 6: check.enabled — fixed multi-select picker

**Problem:** `check.enabled` is `FieldKind::VecString`, opening a free-text add/del editor. The valid values are a fixed set of 10 strings. Should use a GroupPicker-style multi-select.

**Fix:** Add `FieldKind::CheckEnabled` variant. Use it for the `enabled` field in `check_form_fields()`. In the Enter/e handler, open a `GroupPickerState` seeded with the 10 fixed options. The existing `GroupPickerState` commit path writes back a bracket-list display value, which `commit_entry_form()` already parses correctly — no further changes needed there.

**Files:**
- Modify: `src/tui/tabs/config_tab.rs`

- [ ] **Step 1: Add CHECK_ENABLED_OPTIONS constant and FieldKind::CheckEnabled**

Near the top of `config_tab.rs`, after the `FieldKind` enum definition (~line 61), add:

```rust
pub const CHECK_ENABLED_OPTIONS: &[(&str, &str)] = &[
    ("online",      "Check if host is online"),
    ("system_info", "System info (uname / systeminfo)"),
    ("cpu_arch",    "CPU architecture"),
    ("memory",      "Memory usage"),
    ("swap",        "Swap usage"),
    ("disk",        "Disk usage"),
    ("cpu_load",    "CPU load"),
    ("network",     "Network interface info"),
    ("battery",     "Battery status"),
    ("ip_address",  "IP address"),
];
```

In the `FieldKind` enum, add the variant:

```rust
pub enum FieldKind {
    U64,
    Bool,
    String,
    OptionalString,
    Enum {
        _variants: Vec<&'static str>,
    },
    VecString,
    #[allow(dead_code)]
    VecCheckPath,
    CheckEnabled, // ← new
    ShellEnum,
    TriBool,
}
```

- [ ] **Step 2: Use CheckEnabled in check_form_fields()**

In `check_form_fields()` (~line 212), change:

```rust
FieldDescriptor::vec_field("enabled", fmt_vec(&c.enabled), FieldKind::VecString),
```

To:

```rust
FieldDescriptor::vec_field("enabled", fmt_vec(&c.enabled), FieldKind::CheckEnabled),
```

- [ ] **Step 3: Open GroupPickerState for CheckEnabled in handle_entry_form_key()**

In the `KeyCode::Char('e') | KeyCode::Enter` arm, inside the `match &field.kind` block, currently `VecString | VecCheckPath` checks if `field.key == "groups"` to decide GroupPicker vs VecEditor. Add a new arm before `VecString`:

```rust
FieldKind::CheckEnabled => {
    let current = parse_bracket_list(&form.fields[idx].display_value);
    let available: Vec<String> = CHECK_ENABLED_OPTIONS
        .iter()
        .map(|(k, _)| k.to_string())
        .collect();
    let checked: Vec<bool> = available
        .iter()
        .map(|k| current.contains(k))
        .collect();
    let mut vp = Viewport::new();
    vp.set_dims(available.len().max(1), 0);
    form.group_picker = Some(GroupPickerState {
        field_index: idx,
        available,
        checked,
        vp,
        closing: false,
    });
}
```

- [ ] **Step 4: Update group_picker render hints to show descriptions for CheckEnabled**

The existing group_picker render loop (~line 1379) shows item names only. For `CheckEnabled`, we want to also show the description in dim style. We need to know which field triggered the picker. Add a boolean `show_descriptions` field to `GroupPickerState`:

```rust
pub struct GroupPickerState {
    pub field_index: usize,
    pub available: Vec<String>,
    pub checked: Vec<bool>,
    pub vp: Viewport,
    pub closing: bool,
    pub descriptions: Vec<String>, // ← new; empty = no descriptions
}
```

Update all `GroupPickerState` construction sites:

For the existing `groups` picker (~line 821):
```rust
form.group_picker = Some(GroupPickerState {
    field_index: idx,
    available,
    checked,
    vp,
    closing: false,
    descriptions: vec![],  // ← add
});
```

For the new `CheckEnabled` picker (Task 6 Step 3):
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
        .collect(),  // ← add
});
```

- [ ] **Step 5: Update group_picker render to show descriptions when present**

In `render_entry_form()`, the group_picker item loop (~line 1397):

```rust
// BEFORE:
lines.push(Line::from(Span::styled(format!("  {mark} {group}"), style)));
```

Replace with:

```rust
// AFTER:
let desc = gp.descriptions.get(abs).map(|d| d.as_str()).unwrap_or("");
if desc.is_empty() {
    lines.push(Line::from(Span::styled(
        format!("  {mark} {group}"),
        style,
    )));
} else {
    let dim = Style::default().fg(theme.border_inactive);
    lines.push(Line::from(vec![
        Span::styled(format!("  {mark} {group}", ), style),
        Span::styled(format!("  — {desc}"), dim),
    ]));
}
```

Also update the group_picker title hint line (~line 1381) to differentiate context:

```rust
// BEFORE:
"  Pick groups  (Space:toggle  Enter:apply  Esc:cancel)"
```

For the `enabled` field specifically, the field key tells us. Access `form.fields[gp.field_index].key` or simpler: check `!gp.descriptions.is_empty()`:

```rust
let picker_title = if gp.descriptions.is_empty() {
    "  Pick groups  (Space:toggle  Enter/s:apply  Esc:cancel)".to_string()
} else {
    format!(
        "  Editing: {}  (Space:toggle  Enter/s:apply  Esc:cancel)",
        form.fields[gp.field_index].key
    )
};
lines.push(Line::from(Span::styled(
    picker_title,
    Style::default().fg(theme.warning),
)));
```

- [ ] **Step 6: Build**

```bash
cargo build 2>&1 | head -30
```

- [ ] **Step 7: Commit**

```bash
git add src/tui/tabs/config_tab.rs
git commit -m "feat: check.enabled uses fixed multi-select picker"
```

---

### Task 7: Final build, clippy, and integration check

- [ ] **Step 1: Full build + clippy**

```bash
cargo build 2>&1
cargo clippy -- -D warnings 2>&1 | head -40
```

Expected: zero errors, zero warnings.

- [ ] **Step 2: Run**

```bash
cargo run -- tui
```

Manual verification checklist:
- Open Config tab → edit any entry → press Esc with dirty state → y/n respond correctly
- `✓ Config saved` appears at bottom (same row as errors), not top
- Open a check entry → edit `enabled` → multi-select picker shows 10 options with descriptions
- In any vec_editor (e.g. `paths` in sync) → press `s` commits; pressing `q` does NOT quit
- In vec_editor New: prompt → cursor (yellow block) is visible
- On a Bool field → Space toggles true/false
- Group picker and vec_editor both intercept all keys

- [ ] **Step 3: Update CHANGELOG.md**

Append to `CHANGELOG.md`:

```markdown
## Unreleased — 2026-05-08

### Fixed
- `✓ Config saved` banner unified to bottom status bar (same position as error messages)
- Confirm dialog y/n now responds correctly when triggered from within an entry form (routing bug)
- Sub-popups (vec_editor, group_picker) now capture all keys, preventing global shortcuts (q, ?) from firing while a sub-popup is open
- vec_editor commits correctly on `s` key (previously only Esc worked)
- Cursor (yellow block) now visible in vec_editor `New:` input prompt

### Added
- `check.enabled` field now uses a fixed multi-select picker (10 predefined check types with descriptions) instead of free-text add/del
- Bool fields toggle with Space bar; hint bar updated to show `[Space] Toggle bool`
```

- [ ] **Step 4: Final commit**

```bash
git add CHANGELOG.md
git commit -m "chore: update CHANGELOG for TUI popup UX fixes"
```
