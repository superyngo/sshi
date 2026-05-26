# TUI Operate/View Refactor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restructure the TUI into a `Config` / `Operate` / `View` tab layout where Operate launches `check/run/exec/sync` (progress popup) and a new View tab hosts `checkout/list/log` (live-refresh result area), with all operation-specific params edited through the existing Config field interface.

**Architecture:** Single-column "Approach B" layout per tab (op selector → target summary → specific fields → trigger/result). Targets stay mutually exclusive via the existing `FilterPopup` (plus a new `skip` modifier). View commands gain structured `*_core` data functions; `checkout` reuses its existing `pub(crate)` helpers. `init` is intentionally excluded (CLI-only) — its `stdin` prompts cannot run in the raw-mode TUI.

**Tech Stack:** Rust, ratatui, tokio, rusqlite, serde/toml. Spec: `docs/superpowers/specs/2026-05-26-tui-operate-refactor-design.md`.

**Conventions:**
- Build/test with the `tui` feature: `cargo test --features tui` and `cargo build --features tui`.
- Commit after each task. Conventional commit messages.
- TDD where unit-testable (cores, schema, resolution, persistence); build + manual verification for rendering/navigation tasks.

---

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `src/commands/list.rs` | `list_core` (data) + `run` (print wrapper) | Modify |
| `src/commands/log.rs` | `log_core` (rows) + `run` (print wrapper) | Modify |
| `src/tui/state/persist.rs` | `TargetFilterState.skip`, `ActiveTab::View` alias, `OperateState` fields, `ViewOperationKind` | Modify |
| `src/tui/components/target_filter.rs` | `skip` row in `FilterPopup` | Modify |
| `src/tui/tabs/mod.rs` | `TabId::View` rename | Modify |
| `src/tui/tabs/operate_schema.rs` | `specific_fields`/`apply_specific` for op params | **Create** |
| `src/tui/tabs/operate_tab.rs` | Approach-B render (selector + summary + specific + execute) | Rewrite render |
| `src/tui/tabs/view_tab.rs` | View render + result renderers (checkout/list/log) | **Create** |
| `src/tui/app.rs` | state fields, key dispatch, render dispatch, live-refresh, removal of stale `run_yes`/Checkout-inline | Modify |
| `CHANGELOG.md` | Unreleased entry | Modify |
| `docs/tui_reconstruct_plan.md` | reconciliation note (Checkout→View) | Modify |

---

## Phase 0 — Prerequisite: View command cores (no TUI changes)

### Task 1: Extract `list_core` from `list::run`

**Files:**
- Modify: `src/commands/list.rs`

- [ ] **Step 1: Write the failing test**

Add to `src/commands/list.rs` (bottom):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_support::ctx_with_hosts; // see note below

    #[test]
    fn list_core_returns_resolved_collections() {
        let ctx = ctx_with_hosts(&[("h1", &["web"]), ("h2", &[])]);
        let data = list_core(&ctx);
        assert_eq!(data.hosts.len(), 2);
        // checks/syncs default-empty config → empty
        assert!(data.checks.is_empty());
        assert!(data.syncs.is_empty());
    }
}
```

> If no shared `test_support` ctx builder exists, construct a `Context` inline using the same pattern other command tests use (search `tests` modules in `src/commands/`). If none exists, gate this test behind constructing `AppConfig::default()` + an in-memory DB via `crate::state::db::open(None)`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --features tui -p sshi list_core_returns_resolved_collections`
Expected: FAIL — `list_core` not found.

- [ ] **Step 3: Extract the core**

In `src/commands/list.rs`, add the data struct + core, and reduce `run` to a printing wrapper. Replace the body of `run` that computes `hosts/checks/syncs` with a call to `list_core`:

```rust
use crate::config::schema::{CheckEntry, HostEntry, SyncEntry};

/// Structured result of `list`, for both stdout printing and the TUI View tab.
pub struct ListData {
    pub hosts: Vec<HostEntry>,
    pub checks: Vec<CheckEntry>,
    pub syncs: Vec<SyncEntry>,
}

/// Resolve hosts/checks/syncs with no I/O side effects.
pub fn list_core(ctx: &Context) -> ListData {
    ListData {
        hosts: ctx.resolve_hosts().unwrap_or_default(),
        checks: ctx.resolve_checks(),
        syncs: ctx.resolve_syncs(),
    }
}
```

> Match `resolve_checks`/`resolve_syncs` return types. If they return `Vec<&CheckEntry>`, change `ListData` fields to owned via `.into_iter().cloned().collect()` in `list_core`, or store the owned clones. Verify with `grep -n "fn resolve_checks\|fn resolve_syncs\|fn resolve_hosts" src/commands/mod.rs`.

Then rewrite `run` to print from `list_core`:

```rust
pub async fn run(ctx: &Context) -> Result<()> {
    let ListData { hosts, checks, syncs } = list_core(ctx);

    println!("── Hosts ({}) ──", hosts.len());
    println!("  {:<16} {:<20} {:<12} Groups", "Name", "SSH Host", "Shell");
    println!("  {}", "-".repeat(64));
    for h in &hosts {
        let groups = if h.groups.is_empty() { "-".to_string() } else { h.groups.join(", ") };
        println!("  {:<16} {:<20} {:<12} {}", h.name, h.ssh_host, h.shell, groups);
    }

    println!("\n── Applicable Checks ({}) ──", checks.len());
    if checks.is_empty() {
        println!("  (none)");
    } else {
        for (i, entry) in checks.iter().enumerate() {
            let scope = format_scope(&entry.groups, entry.enable_hosts, entry.enable_all);
            println!("  [{}] scope: {}", i + 1, scope);
            if !entry.enabled.is_empty() {
                println!("      enabled: {}", entry.enabled.join(", "));
            }
            for p in &entry.path {
                println!("      path: {} ({})", p.path, p.label);
            }
        }
    }

    println!("\n── Applicable Sync Entries ({}) ──", syncs.len());
    if syncs.is_empty() {
        println!("  (none)");
    } else {
        for (i, entry) in syncs.iter().enumerate() {
            let scope = format_scope(&entry.groups, entry.enable_hosts, entry.enable_all);
            println!("  [{}] scope: {}  paths: {}", i + 1, scope, entry.paths.join(", "));
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Run test + full suite**

Run: `cargo test --features tui -p sshi list_core_returns_resolved_collections && cargo test --features tui`
Expected: PASS, no regressions.

- [ ] **Step 5: Commit**

```bash
git add src/commands/list.rs
git commit -m "refactor(list): extract list_core data fn from run"
```

---

### Task 2: Extract `log_core` from `log::run`

**Files:**
- Modify: `src/commands/log.rs`

- [ ] **Step 1: Write the failing test**

Add to `src/commands/log.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_relative_days() {
        let now = chrono::Utc::now().timestamp();
        let ts = parse_since("7d").unwrap();
        assert!((now - ts - 7 * 86400).abs() < 5);
    }

    #[test]
    fn log_core_empty_db_returns_no_rows() {
        let db = crate::state::db::open(None).unwrap();
        let ctx = crate::commands::Context::for_test(db); // see note
        let rows = log_core(&ctx, 20, None, None, None, false).unwrap();
        assert!(rows.is_empty());
    }
}
```

> If `Context::for_test` does not exist, build a `Context` inline matching the struct's public fields (config `AppConfig::default()`, `db`, `timeout`, `mode: TargetMode::All`, etc. — copy the pattern from an existing command test). The key assertion is that `log_core` returns `Ok(vec![])` against an empty in-memory DB.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --features tui -p sshi log_core_empty_db_returns_no_rows`
Expected: FAIL — `log_core` not found.

- [ ] **Step 3: Extract the core**

In `src/commands/log.rs`, add the row struct + `log_core` returning rows; rewrite `run` to format/print them:

```rust
/// One operation-log row, for stdout and the TUI View tab.
pub struct LogRow {
    pub ts: i64,
    pub command: String,
    pub host: String,
    pub action: String,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub note: Option<String>,
}

/// Query the operation log with no I/O side effects (returns rows newest-first).
pub fn log_core(
    ctx: &Context,
    last: usize,
    since: Option<String>,
    host: Option<String>,
    action: Option<ActionFilter>,
    errors: bool,
) -> Result<Vec<LogRow>> {
    let mut query = String::from(
        "SELECT timestamp, command, host, action, status, duration_ms, note FROM operation_log WHERE 1=1",
    );
    let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref h) = host {
        query.push_str(&format!(" AND host = ?{}", bind_values.len() + 1));
        bind_values.push(Box::new(h.clone()));
    }
    if let Some(ref a) = action {
        let action_str = match a {
            ActionFilter::Sync => "sync",
            ActionFilter::Run => "run",
            ActionFilter::Exec => "exec",
            ActionFilter::Check => "check",
        };
        query.push_str(&format!(" AND command = ?{}", bind_values.len() + 1));
        bind_values.push(Box::new(action_str.to_string()));
    }
    if errors {
        query.push_str(" AND status = 'error'");
    }
    if let Some(ref s) = since {
        let since_ts = parse_since(s)?;
        query.push_str(&format!(" AND timestamp >= ?{}", bind_values.len() + 1));
        bind_values.push(Box::new(since_ts));
    }
    query.push_str(" ORDER BY timestamp DESC");
    query.push_str(&format!(" LIMIT {}", last));

    let mut stmt = ctx.db.prepare(&query)?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> =
        bind_values.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(params_refs.as_slice(), |row| {
        Ok(LogRow {
            ts: row.get(0)?,
            command: row.get(1)?,
            host: row.get(2)?,
            action: row.get(3)?,
            status: row.get(4)?,
            duration_ms: row.get(5)?,
            note: row.get(6)?,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
}
```

Rewrite `run` to print from rows:

```rust
pub async fn run(
    ctx: &Context,
    last: usize,
    since: Option<String>,
    host: Option<String>,
    action: Option<ActionFilter>,
    errors: bool,
) -> Result<()> {
    let rows = log_core(ctx, last, since, host, action, errors)?;
    if rows.is_empty() {
        println!("No log entries found.");
        return Ok(());
    }
    for r in &rows {
        let time = chrono::DateTime::from_timestamp(r.ts, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| r.ts.to_string());
        let duration = r.duration_ms.map(|ms| format!(" ({:.1}s)", ms as f64 / 1000.0)).unwrap_or_default();
        let note_str = r.note.as_deref().map(|n| format!(" — {}", n)).unwrap_or_default();
        let status_icon = match r.status.as_str() {
            "ok" => "\x1b[32m✓\x1b[0m",
            "error" => "\x1b[31m✗\x1b[0m",
            "skipped" => "\x1b[33m⊘\x1b[0m",
            _ => "·",
        };
        println!("{} {} [{}] {} {}{}{}", time, status_icon, r.host, r.command, r.action, duration, note_str);
    }
    Ok(())
}
```

- [ ] **Step 4: Run test + full suite**

Run: `cargo test --features tui -p sshi log_core && cargo test --features tui`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/commands/log.rs
git commit -m "refactor(log): extract log_core query fn from run"
```

---

## Phase 1 — Target & tab foundation

### Task 3: Add `skip` to `TargetFilterState` + apply as final subtraction

**Files:**
- Modify: `src/tui/state/persist.rs`
- Modify: `src/tui/app.rs` (the `resolve_target_names` helper)

- [ ] **Step 1: Write the failing test**

In `src/tui/state/persist.rs` tests module:

```rust
#[test]
fn skip_field_round_trips_and_defaults_empty() {
    let s: TuiPersistedState = toml::from_str("").unwrap();
    assert!(s.target_filter.skip.is_empty());

    let mut t = TargetFilterState::default();
    t.skip = vec!["h9".into()];
    let ser = toml::to_string(&t).unwrap();
    let back: TargetFilterState = toml::from_str(&ser).unwrap();
    assert_eq!(back.skip, vec!["h9".to_string()]);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --features tui -p sshi skip_field_round_trips`
Expected: FAIL — no field `skip`.

- [ ] **Step 3: Add the field**

In `TargetFilterState` (`persist.rs`), add `pub skip: Vec<String>,` and add `skip: Vec::new(),` to its `Default` impl.

- [ ] **Step 4: Apply skip in resolution**

Locate `fn resolve_target_names` in `src/tui/app.rs` (`grep -n "fn resolve_target_names" src/tui/app.rs`). After it builds the host-name list, subtract skip. The function takes the resolved `TargetMode`/config — thread `&self.target_filter.skip` (or pass skip in). Add at the end, before returning the names vec:

```rust
// Apply the TUI skip modifier as a final subtraction.
let skip = &self.target_filter.skip; // adjust to the function's access path
names.retain(|n| !skip.iter().any(|s| s == n));
```

> If `resolve_target_names` is a free function without `&self`, add a `skip: &[String]` parameter and update its call sites (`grep -n "resolve_target_names(" src/tui/app.rs`).

- [ ] **Step 5: Add a resolution test**

In `src/tui/app.rs` tests (or wherever `build_target_mode` is tested), add:

```rust
#[test]
fn skip_subtracts_from_resolved_targets() {
    // Build a config with h1,h2,h3 all in group "g"; mode=All; skip=[h2].
    // Assert resolved names == [h1, h3].
    // (Mirror the existing target-resolution test setup in this module.)
}
```

Fill in using the existing test helpers in that module (copy the closest existing resolution test and add `skip`).

- [ ] **Step 6: Run tests**

Run: `cargo test --features tui -p sshi skip_`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/tui/state/persist.rs src/tui/app.rs
git commit -m "feat(tui): add skip modifier to target filter + resolution"
```

---

### Task 4: Add the `skip` row to `FilterPopup`

**Files:**
- Modify: `src/tui/components/target_filter.rs`

- [ ] **Step 1: Read the popup to find the field list**

Run: `grep -n "enum\|struct\|fn render\|fn handle_key\|skip\|serial\|timeout" src/tui/components/target_filter.rs`
Identify how `serial`/`timeout` rows are represented (the popup's internal focus enum + render rows + commit back to `TargetFilterState`).

- [ ] **Step 2: Add `skip` as a vec-edited row**

Mirror the existing groups/hosts vec-entry handling (the popup already edits `groups`/`hosts` lists). Add a `skip` editable row using the same input/commit mechanism, writing into `TargetFilterState.skip` on commit. Render it in the same style as the other vec rows.

> No new test framework here — the popup has limited unit coverage. Verify via build + the manual checklist (Task 16). If the popup has a `handle_key` unit test, extend it to cover adding/removing a skip entry.

- [ ] **Step 3: Build**

Run: `cargo build --features tui`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src/tui/components/target_filter.rs
git commit -m "feat(tui): add skip row to target filter popup"
```

---

### Task 5: Rename `TabId::Checkout` → `TabId::View`; `ActiveTab::View` with serde alias

**Files:**
- Modify: `src/tui/tabs/mod.rs`
- Modify: `src/tui/state/persist.rs`
- Modify: `src/tui/app.rs` (all `TabId::Checkout` match arms)

- [ ] **Step 1: Write the failing persistence test**

In `persist.rs` tests:

```rust
#[test]
fn legacy_checkout_tab_loads_as_view() {
    let s: TuiPersistedState = toml::from_str(
        "[tui_state]\nactive_tab = \"Checkout\"\n",
    ).unwrap();
    assert_eq!(s.tui_state.active_tab, ActiveTab::View);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --features tui -p sshi legacy_checkout_tab_loads_as_view`
Expected: FAIL — no `ActiveTab::View`.

- [ ] **Step 3: Rename `ActiveTab` variant + alias**

In `persist.rs`, rename `Checkout` → `View` in `ActiveTab` and add the alias:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveTab {
    Config,
    Operate,
    #[default]
    #[serde(alias = "Checkout")]
    View,
}
```

Update `from_tab_id`/`to_tab_id` to use `TabId::View`/`ActiveTab::View`. Update the existing `persist.rs` tests that assert `ActiveTab::Checkout` (e.g. `empty_string_loads_as_default`, `malformed_file_returns_default`, `missing_file_returns_default`) to `ActiveTab::View`.

- [ ] **Step 4: Rename `TabId::Checkout` → `TabId::View`**

In `src/tui/tabs/mod.rs`, rename the variant, update `ALL`, `label` (`"3:View"`), `next`/`prev`. Then update every `TabId::Checkout` in `app.rs`:

Run: `grep -rn "TabId::Checkout\|ActiveTab::Checkout" src/`
Replace each occurrence with `::View`. (Render dispatch `app.rs:1692`, key nav `1036/1127/1204`, scroll branches `1624-1656`, tab-bar color `1731`.)

- [ ] **Step 5: Run tests + build**

Run: `cargo test --features tui -p sshi legacy_checkout_tab_loads_as_view && cargo build --features tui`
Expected: PASS + compiles.

- [ ] **Step 6: Commit**

```bash
git add src/tui/tabs/mod.rs src/tui/state/persist.rs src/tui/app.rs
git commit -m "refactor(tui): rename Checkout tab to View (serde alias for state)"
```

---

## Phase 2 — Operate state + field schema

### Task 6: Extend `OperateState`; add `ViewOperationKind`; drop `run_yes`

**Files:**
- Modify: `src/tui/state/persist.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn operate_state_extended_round_trips() {
    let mut s = OperateState::default();
    s.log_last = 50;
    s.log_errors = true;
    s.checkout_history = true;
    let ser = toml::to_string(&s).unwrap();
    let back: OperateState = toml::from_str(&ser).unwrap();
    assert_eq!(back.log_last, 50);
    assert!(back.log_errors);
    assert!(back.checkout_history);
}

#[test]
fn view_operation_kind_defaults_checkout() {
    assert_eq!(ViewOperationKind::default(), ViewOperationKind::Checkout);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --features tui -p sshi operate_state_extended_round_trips view_operation_kind_defaults`
Expected: FAIL.

- [ ] **Step 3: Extend state**

In `persist.rs`, remove `run_yes` from `OperateState`, and add the new fields + the view enum:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OperateState {
    pub operation: OperationKind,
    pub run_sudo: bool,
    pub exec_sudo: bool,
    pub exec_keep: bool,
    pub sync_mode: SyncMode,
    pub sync_dry_run: bool,

    // check/run/exec/sync dry-run + out are handled per-op (out is session-only
    // input; dry-run booleans below where persisted).
    pub check_dry_run: bool,
    pub run_dry_run: bool,
    pub exec_dry_run: bool,

    // View ops.
    pub view_operation: ViewOperationKind,
    pub checkout_history: bool,
    pub log_last: usize,
    pub log_errors: bool,
    // log_since / log_host / log_action and checkout_since are session-only
    // text/enum inputs (NOT persisted, per AD-12) and live in App.
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViewOperationKind {
    #[default]
    Checkout,
    List,
    Log,
}
```

> `log_last` default is `0`; the App initializes the `log` field to `20` when empty (matching the CLI default). Note this in Task 11.

- [ ] **Step 4: Fix `OperateState` construction in `app.rs`**

`from_context` (`app.rs:~239`) sets `run_yes: persisted.operate.run_yes`. Remove that line and the `run_yes` field from `App` (struct + constructor). Add constructor wiring for the new persisted fields (see Task 11 for the full set). For now, remove `run_yes` everywhere to keep it compiling:

Run: `grep -rn "run_yes" src/tui/`
Delete the `App.run_yes` field, its constructor init, and any read (the `OperateRenderData.run_yes` and its render usage are removed in Task 8 — for now leave `run_yes: false` literal at the render-data construction site if needed to compile, to be removed in Task 8).

- [ ] **Step 5: Run tests + build**

Run: `cargo test --features tui -p sshi operate_state view_operation && cargo build --features tui`
Expected: PASS + compiles.

- [ ] **Step 6: Commit**

```bash
git add src/tui/state/persist.rs src/tui/app.rs
git commit -m "feat(tui): extend OperateState for view ops; drop dead run_yes"
```

---

### Task 7: Create `operate_schema.rs` — `specific_fields` / `apply_specific`

**Files:**
- Create: `src/tui/tabs/operate_schema.rs`
- Modify: `src/tui/tabs/mod.rs` (add `pub mod operate_schema;`)

This module produces `FieldDescriptor`s (reusing `config_schema::{FieldDescriptor, FieldKind}`) for each operation's specific params and applies edits back into an `OpSpecific` view of state.

- [ ] **Step 1: Define the state-view struct passed to the schema**

To avoid coupling the schema to the whole `App`, define a small mutable view it edits. Add to `operate_schema.rs`:

```rust
use super::config_schema::{FieldDescriptor, FieldKind};
use super::super::state::persist::{OperationKind, SyncMode, ViewOperationKind};
use super::super::components::input_field::InputField;

/// Mutable references to the operation-specific fields the schema reads/writes.
/// Built by App from its own fields each frame.
pub struct OpSpecific<'a> {
    pub sudo: &'a mut bool,        // run/exec
    pub keep: &'a mut bool,        // exec
    pub dry_run: &'a mut bool,     // check/run/exec/sync
    pub sync_mode: &'a mut SyncMode,
    // Text inputs and out string are rendered directly by App (InputField),
    // so the schema covers only the bool/enum/number params.
    pub checkout_history: &'a mut bool,
    pub log_last: &'a mut usize,
    pub log_errors: &'a mut bool,
}
```

> Rationale: `InputField` (command/script/source/files/since/host/out) keeps its own focus + cursor handling and is rendered directly by the tab (as today). The schema unifies the **cycle/toggle/number** fields so they edit exactly like Config fields. This keeps the schema small and avoids re-implementing text editing.

- [ ] **Step 2: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_specific_fields_has_dry_run() {
        let f = check_specific_fields();
        assert!(f.iter().any(|d| d.key == "dry_run"));
    }

    #[test]
    fn apply_run_sudo_toggles() {
        let (mut sudo, mut keep, mut dry) = (false, false, false);
        let mut sm = SyncMode::ConfigEntries;
        let (mut h, mut last, mut err) = (false, 0usize, false);
        let mut s = OpSpecific {
            sudo: &mut sudo, keep: &mut keep, dry_run: &mut dry,
            sync_mode: &mut sm, checkout_history: &mut h,
            log_last: &mut last, log_errors: &mut err,
        };
        apply_specific(&mut s, OperationKind::Run, "sudo", "true");
        assert!(sudo);
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test --features tui -p sshi check_specific_fields_has_dry_run apply_run_sudo_toggles`
Expected: FAIL — module not found.

- [ ] **Step 4: Implement the schema**

```rust
pub fn check_specific_fields() -> Vec<FieldDescriptor> {
    vec![FieldDescriptor::scalar("dry_run", "false".into(), FieldKind::Bool)]
}

pub fn run_specific_fields(sudo: bool, dry_run: bool) -> Vec<FieldDescriptor> {
    vec![
        FieldDescriptor::scalar("sudo", sudo.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("dry_run", dry_run.to_string(), FieldKind::Bool),
    ]
}

pub fn exec_specific_fields(sudo: bool, keep: bool, dry_run: bool) -> Vec<FieldDescriptor> {
    vec![
        FieldDescriptor::scalar("sudo", sudo.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("keep", keep.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("dry_run", dry_run.to_string(), FieldKind::Bool),
    ]
}

pub fn sync_specific_fields(mode: SyncMode, dry_run: bool) -> Vec<FieldDescriptor> {
    let mode_str = match mode { SyncMode::ConfigEntries => "config", SyncMode::AdHoc => "adhoc" };
    vec![
        FieldDescriptor::scalar("mode", mode_str.into(),
            FieldKind::Enum { variants: vec!["config", "adhoc"] }),
        FieldDescriptor::scalar("dry_run", dry_run.to_string(), FieldKind::Bool),
    ]
}

pub fn checkout_specific_fields(history: bool) -> Vec<FieldDescriptor> {
    vec![FieldDescriptor::scalar("history", history.to_string(), FieldKind::Bool)]
}

pub fn log_specific_fields(last: usize, errors: bool) -> Vec<FieldDescriptor> {
    vec![
        FieldDescriptor::scalar("last", last.to_string(), FieldKind::U64),
        FieldDescriptor::scalar("errors", errors.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("action", "all".into(),
            FieldKind::Enum { variants: vec!["all", "sync", "run", "exec", "check"] }),
    ]
}

pub fn apply_specific(s: &mut OpSpecific, op: OperationKind, key: &str, val: &str) {
    match (op, key) {
        (OperationKind::Run | OperationKind::Exec, "sudo") => *s.sudo = val == "true",
        (OperationKind::Exec, "keep") => *s.keep = val == "true",
        (_, "dry_run") => *s.dry_run = val == "true",
        (OperationKind::Sync, "mode") => {
            *s.sync_mode = if val == "adhoc" { SyncMode::AdHoc } else { SyncMode::ConfigEntries };
        }
        _ => {}
    }
}
```

> `action` and view-specific applies (history/last/errors) follow the same pattern; add `apply_view_specific(view_op, key, val)` analogously when wiring Task 11. Keep apply functions total (unknown keys = no-op), matching `config_schema`.

Register the module in `src/tui/tabs/mod.rs`: add `pub mod operate_schema;`.

- [ ] **Step 5: Run tests + build**

Run: `cargo test --features tui -p sshi _specific_fields apply_ && cargo build --features tui`
Expected: PASS + compiles.

- [ ] **Step 6: Commit**

```bash
git add src/tui/tabs/operate_schema.rs src/tui/tabs/mod.rs
git commit -m "feat(tui): operate_schema for operation-specific fields"
```

---

## Phase 3 — Operate tab rebuild (Approach B)

### Task 8: Rewrite `render_operate` to selector → target summary → specific fields → execute

**Files:**
- Modify: `src/tui/tabs/operate_tab.rs`
- Modify: `src/tui/app.rs` (`render_operate` at `~2061`, `OperateRenderData`)

- [ ] **Step 1: Trim `OperateRenderData`**

In `operate_tab.rs`, remove `run_yes` from `OperateRenderData` and its render usage (`render_run_exec_params` second-flag block for run). The second flag now only applies to `exec` (`keep`); for `run` render only `sudo` + `dry_run`. Remove the `ParamPanelField::SecondFlag` reuse for `run --yes`.

- [ ] **Step 2: Replace the layout with the Approach-B stack**

Restructure `render_operate` to:
1. Op selector row (`render_op_radio`, unchanged — already a horizontal radio).
2. **Target summary line** (new `render_target_summary`): one line showing mode + skip + serial + timeout, styled active when `operate_focus == TargetRow`. Reuse `data.target_filter`. Example:

```rust
fn render_target_summary(data: &OperateRenderData, area: Rect, frame: &mut Frame) {
    let focused = data.focus == OperateFocus::TargetRow;
    let mode = match data.target_filter.mode {
        TargetFilterMode::All => "all hosts".into(),
        TargetFilterMode::Groups => format!("groups:[{}]", data.target_filter.groups.join(",")),
        TargetFilterMode::Hosts => format!("hosts:[{}]", data.target_filter.hosts.join(",")),
        TargetFilterMode::Shell => format!("shell:{:?}", data.target_filter.shell),
    };
    let skip = if data.target_filter.skip.is_empty() { String::new() }
               else { format!(" · skip:[{}]", data.target_filter.skip.join(",")) };
    let line = format!(" Target: {}  ({} hosts){} · serial={} · {}s    [f] filter",
        mode, data.target_count, skip, data.target_filter.serial, data.target_filter.timeout);
    let style = if focused { Style::default().fg(data.theme.accent_operate).add_modifier(Modifier::REVERSED) }
                else { Style::default() };
    frame.render_widget(Paragraph::new(line).style(style), area);
}
```

3. **Specific-params block**: keep the existing `render_run_exec_params` / `render_sync_params` (they already use `InputField` + toggles). They render `command`/`script`/`source`/adhoc `files` text inputs and the sudo/keep/dry_run/mode toggles. The toggles are now backed by `operate_schema` values (no behavior change to rendering).
4. **Applicable entries** stay for `check`/`sync` config-entries mode (unchanged `render_applicable_entries`).
5. **Execute button** (`render_execute_button`, unchanged).

Update the `Layout` constraints to the new vertical order (selector / target-summary(1 line) / params / entries / execute).

- [ ] **Step 3: Update `App::render_operate` data construction**

In `app.rs:~2061`, drop `run_yes` from the `OperateRenderData { .. }` literal. Confirm `target_count` is computed from the skip-aware resolver (Task 3).

- [ ] **Step 4: Build + manual smoke**

Run: `cargo build --features tui`
Then run the binary in a terminal (`cargo run --features tui`), open Operate, cycle ops with ←/→, confirm the target summary line shows and `[f]` opens the popup. (Full manual sweep is Task 16.)
Expected: compiles; Operate renders selector + summary + params + execute; no `--yes` row for run.

- [ ] **Step 5: Commit**

```bash
git add src/tui/tabs/operate_tab.rs src/tui/app.rs
git commit -m "feat(tui): Operate tab Approach-B layout (summary line, no --yes)"
```

---

### Task 9: Wire specific-field cycling through `operate_schema` + navigation

**Files:**
- Modify: `src/tui/app.rs` (Operate key handling `~1386-1620`)

- [ ] **Step 1: Route toggle/cycle keys through apply_specific**

Where the Operate tab handles Space/←/→ on param toggles (`param_field` handling around `app.rs:1407-1470`), replace ad-hoc toggling with building an `OpSpecific` view and calling `operate_schema::apply_specific(..)` / `cycle_option_value`-based changes, so behavior matches Config field cycling. Keep `InputField` text editing as-is for command/script/source/files.

> This is a behavior-preserving refactor of how toggles flip; verify the existing Operate toggle tests (if any) still pass and add one asserting Space toggles `run_sudo` via the new path.

- [ ] **Step 2: Confirm navigation zones**

Verify zone order matches the spec: `OpRadio(selector) → TargetRow(summary) → ParamPanel → ApplicableEntries → Execute`. The existing `OperateFocus` enum already has these. Ensure ↑/↓ skip the entries zone when the op has none, and `f` opens the popup only from any Operate zone (already at `app.rs:1617`).

- [ ] **Step 3: Build + test**

Run: `cargo build --features tui && cargo test --features tui`
Expected: compiles, tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/tui/app.rs
git commit -m "feat(tui): Operate param toggles via operate_schema (Config-consistent)"
```

---

## Phase 4 — View tab

### Task 10: Create `view_tab.rs` — state + render scaffold (selector + summary + specific + result area)

**Files:**
- Create: `src/tui/tabs/view_tab.rs`
- Modify: `src/tui/tabs/mod.rs` (`pub mod view_tab;`)

- [ ] **Step 1: Define the render-data struct and entry point**

```rust
use ratatui::{layout::{Constraint, Direction, Layout, Rect}, widgets::{Block, Borders, Paragraph}, Frame};
use super::super::state::persist::ViewOperationKind;
use super::super::theme::Theme;
use crate::commands::list::ListData;
use crate::commands::log::LogRow;
use crate::commands::checkout::{DisplayColumns, HostSnapshot};

pub struct ViewRenderData<'a> {
    pub view_op: ViewOperationKind,
    pub theme: &'a Theme,
    pub navbar_focused: bool,
    pub loading: bool,
    // Result payloads (only the active op's is populated):
    pub checkout: Option<(&'a [HostSnapshot], &'a DisplayColumns)>,
    pub list: Option<&'a ListData>,
    pub log: Option<&'a [LogRow]>,
    pub result_scroll: usize,
}

pub fn render_view(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    let block = Block::default().borders(Borders::ALL).title(" View ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chunks = Layout::default().direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    render_view_selector(data, chunks[0], frame);
    render_view_target_summary(data, chunks[1], frame); // greyed for Log
    render_result_area(data, chunks[2], frame);
}
```

- [ ] **Step 2: Implement the three result renderers**

- `render_checkout_result`: reproduce `checkout::print_table_report` as ratatui `Line`s (header + per-host rows). Reuse `extract_metric_value`, `metric_header`, `metric_width`, `format_relative_time` (all `pub(crate)`). This replaces the inline `App::render_checkout`.
- `render_list_result`: render `ListData` (hosts table, checks, syncs) as `Line`s mirroring `list::run`'s text layout.
- `render_log_result`: render `&[LogRow]` as `Line`s mirroring `log::run`'s formatting (status glyph via theme colors instead of raw ANSI).

Each respects `data.result_scroll` and shows a "loading…" placeholder when `data.loading`.

> No unit tests for rendering; correctness is verified by the manual checklist (Task 16). Keep renderers pure functions of the data structs.

- [ ] **Step 3: Register module + build**

Add `pub mod view_tab;` to `src/tui/tabs/mod.rs`.
Run: `cargo build --features tui`
Expected: compiles (renderers may be unused until Task 11 — allow with usage in Task 11, or `#[allow(dead_code)]` temporarily).

- [ ] **Step 4: Commit**

```bash
git add src/tui/tabs/view_tab.rs src/tui/tabs/mod.rs
git commit -m "feat(tui): view_tab module with result renderers"
```

---

### Task 11: Wire View tab into App — state, render dispatch, live refresh

**Files:**
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Add View state to `App`**

Add fields:

```rust
view_op: ViewOperationKind,
view_focus: ViewFocus,        // OpSelector | TargetRow | SpecificPanel
view_result_scroll: usize,
view_loading: bool,
view_dirty: bool,             // set on op/param change → triggers refresh
// payloads:
view_list: Option<ListData>,
view_log: Vec<LogRow>,
// checkout reuses existing self.checkout_snapshots / self.checkout_columns
// session-only inputs:
checkout_since_input: InputField,
log_since_input: InputField,
log_host_input: InputField,
log_action: Option<ActionFilter>,
```

Initialize them in `from_context` from `persisted.operate` (`view_operation`, `checkout_history`, `log_last` defaulting to 20 if 0, `log_errors`).

- [ ] **Step 2: Render dispatch**

At `app.rs:~1692`, replace the `TabId::View => { reload; render_checkout }` block with a `render_view` call that builds `ViewRenderData` from current state. Remove the old `App::render_checkout` method (its table logic now lives in `view_tab::render_checkout_result`). Keep the snapshot reload trigger (`db_stale`) but route the data into `ViewRenderData.checkout`.

- [ ] **Step 3: Live-refresh logic**

Add `fn refresh_view(&mut self)`:

```rust
fn refresh_view(&mut self) {
    let ctx = self.build_context(); // existing helper that builds Context from self
    match self.view_op {
        ViewOperationKind::Checkout => {
            let names: Vec<&str> = /* resolved target names */;
            self.checkout_snapshots = fetch_latest_snapshots(&ctx, &names).unwrap_or_default();
        }
        ViewOperationKind::List => {
            self.view_list = Some(crate::commands::list::list_core(&ctx));
        }
        ViewOperationKind::Log => {
            let last = self.operate_log_last(); // from persisted log_last (>=1)
            let since = self.log_since_input.value_opt();
            let host = self.log_host_input.value_opt();
            self.view_log = crate::commands::log::log_core(
                &ctx, last, since, host, self.log_action.clone(), self.operate_log_errors(),
            ).unwrap_or_default();
        }
    }
    self.view_dirty = false;
}
```

> `build_context`, target-name resolution, and `value_opt()` (InputField → Option<String> when non-empty) may need small helpers — add them mirroring existing patterns (`grep -n "fn build_context\|fn .*context" src/tui/app.rs`; if none, construct `Context` like `execute_check` does). View ops are synchronous DB/config reads, so call them directly on the main thread (no spawn). Set `view_loading` only if a read is slow enough to matter — for now omit the spinner (the reads are sub-millisecond); a `loading` flag stub is fine.

Call `refresh_view()` when: switching to the View tab, changing `view_op`, editing a View param (mark `view_dirty` then refresh at end of the key handler). Minimal debounce: refresh once per key event after handling (no timer needed given read speed).

- [ ] **Step 4: View key handling**

Replace the `TabId::View` (formerly Checkout) scroll-only branches (`app.rs:1624-1656`) with handling for: ←/→ cycle `view_op`; ↑/↓ navigate zones / scroll result; `f` opens the popup (disabled when `view_op == Log`); Space/enter toggle/cycle the active View specific field via `operate_schema`. After any param change, set `view_dirty=true` and call `refresh_view()`.

- [ ] **Step 5: Build + test**

Run: `cargo build --features tui && cargo test --features tui`
Expected: compiles; tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/tui/app.rs
git commit -m "feat(tui): wire View tab (checkout/list/log) with live refresh"
```

---

## Phase 5 — Persistence, docs, verification

### Task 12: Persist new state on save

**Files:**
- Modify: `src/tui/app.rs` (the state-save assembly)

- [ ] **Step 1: Find the save path**

Run: `grep -n "OperateState\|TuiPersistedState\|persist::save\|fn save_state\|fn persist" src/tui/app.rs`

- [ ] **Step 2: Populate new fields on save**

Where the app builds `TuiPersistedState` before `persist::save`, set `operate.view_operation`, `operate.checkout_history`, `operate.log_last`, `operate.log_errors`, the `check/run/exec_dry_run` booleans, and `tui_state.active_tab` (now `View`). Ensure `target_filter` (with `skip`) is included.

- [ ] **Step 3: Add a save/load round-trip test**

In `persist.rs` tests, extend `save_then_load_round_trips` to set and assert `target_filter.skip` and `operate.view_operation`/`operate.log_last`.

- [ ] **Step 4: Run tests**

Run: `cargo test --features tui -p sshi`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tui/app.rs src/tui/state/persist.rs
git commit -m "feat(tui): persist View tab + skip state"
```

---

### Task 13: CHANGELOG + plan-doc reconciliation

**Files:**
- Modify: `CHANGELOG.md`
- Modify: `docs/tui_reconstruct_plan.md`

- [ ] **Step 1: CHANGELOG entry**

Append an `Unreleased` entry (timestamp `2026-05-26`) summarizing: Operate tab now `check/run/exec/sync`; new View tab (`checkout/list/log`) replacing the Checkout tab with live refresh; target filter gains `--skip`; dead `--yes` removed from TUI; `init` remains CLI-only; persisted `[tui_state] active_tab="Checkout"` auto-migrates to `View`.

- [ ] **Step 2: Reconciliation note in the plan doc**

Add a short note at the top of `docs/tui_reconstruct_plan.md` §12.4: "The Checkout tab is superseded by the View tab (`checkout/list/log`); see `docs/superpowers/specs/2026-05-26-tui-operate-refactor-design.md`."

- [ ] **Step 3: Commit**

```bash
git add CHANGELOG.md docs/tui_reconstruct_plan.md
git commit -m "docs: changelog + plan reconciliation for Operate/View refactor"
```

---

### Task 14: Full verification (real binary)

**Files:** none (verification only)

- [ ] **Step 1: Build release-mode TUI**

Run: `cargo build --features tui`
Expected: clean build, no warnings about unused `run_yes`/`_sync_source_input` (source is now wired).

- [ ] **Step 2: Run the full suite**

Run: `cargo test --features tui`
Expected: all pass.

- [ ] **Step 3: Manual sweep in a real terminal**

Run `cargo run --features tui` and verify:
- Tabs are `1:Config 2:Operate 3:View`.
- **Operate**: cycle `check/run/exec/sync` with ←/→; target summary shows mode + skip + serial + timeout; `[f]` opens the popup and `skip` is editable; `run` has no `--yes`; sudo/keep/dry_run toggle like Config fields; Execute runs and shows the progress popup; `sync` source input is now reachable.
- **View**: cycle `checkout/list/log`; `checkout`/`list` honor the target filter and `[f]`; `log` greys the target line and `f` is inert; editing a View param refreshes the result area live; tables/log lines render and scroll.
- **Persistence**: set a group filter + skip on Operate, switch to View and back, quit and relaunch — values persist; a pre-existing state file with `active_tab="Checkout"` opens on the View tab.
- `init` is absent from the TUI; `sshi init` still works from the CLI.

- [ ] **Step 4: Final commit (if any cleanup)**

```bash
git add -A
git commit -m "chore(tui): cleanup after Operate/View refactor"
```

---

## Self-Review (completed by plan author)

- **Spec coverage:** tab split (Tasks 5,8,10,11) · field interface reuse (Task 7,9) · exclusive target + skip (Tasks 3,4) · summary line (Task 8) · View live-refresh (Task 11) · `log` inert targeting (Tasks 10,11) · `init` excluded (no task — intentional) · prerequisite cores (Tasks 1,2; checkout reuses helpers) · persistence + alias migration (Tasks 5,6,12) · CHANGELOG/docs (Task 13) · manual verification (Task 14). All spec sections map to a task.
- **Placeholder scan:** code provided for cores, schema, renderers, and state. App.rs surgery gives exact grep anchors + signatures rather than reproducing the 2643-line file; this is intentional for an existing-file refactor.
- **Type consistency:** `ListData`/`LogRow`/`OpSpecific`/`ViewOperationKind`/`ViewRenderData` names are used consistently across tasks; `FieldDescriptor`/`FieldKind` reused from `config_schema`.
