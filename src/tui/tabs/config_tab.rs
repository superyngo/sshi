//! Config tab — 3-level browser (section → entry → field) + inline editing.
//!
//! Phase 4: read-only browsing + external editor.
//! Phase 7: Case A inline scalar edit, Case B entry forms, toml_edit write-back,
//! `S` save, `a`/`d` add/delete, Vec sub-editors, dirty guard.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
    Frame,
};
use unicode_width::UnicodeWidthStr;

use crate::config::schema::{
    generate_entry_id, AppConfig, CheckEntry, HostEntry, ShellType, SyncEntry,
};

use super::super::components::input_field::{InputField, InputMode};
use super::super::components::viewport::Viewport;
use super::super::theme::Theme;
use super::config_schema::{
    apply_check, apply_host, apply_settings, apply_sync, check_fields, host_fields,
    parse_bracket_list, settings_fields, sync_fields, FieldDescriptor, FieldKind,
    CHECK_ENABLED_OPTIONS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigZone {
    Sidebar,
    FieldTable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarItem {
    SectionSettings,
    SectionHosts,
    Host(usize),
    SectionChecks,
    Check(usize),
    SectionSyncs,
    Sync(usize),
}

/// Collapse state for the three child-bearing sidebar sections. When a section
/// is collapsed, `build_sidebar_items` omits its child entries. Settings has no
/// children, so it is never collapsible.
#[derive(Debug, Clone, Default)]
pub struct CollapsedSections {
    pub hosts: bool,
    pub checks: bool,
    pub syncs: bool,
}

// FieldKind, FieldDescriptor, and CHECK_ENABLED_OPTIONS now live in
// `super::config_schema` (the unified field schema).

// ── Entry form state (Case B) ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryFormKind {
    Host,
    Check,
    Sync,
}

#[derive(Debug)]
pub struct EntryFormState {
    pub kind: EntryFormKind,
    pub edit_index: Option<usize>,
    pub fields: Vec<FieldDescriptor>,
    pub field_vp: Viewport,
    pub active_input: Option<usize>,
    pub input: InputField,
    pub vec_editor: Option<VecEditorState>,
    pub group_picker: Option<GroupPickerState>,
    pub dirty: bool,
}

#[derive(Debug)]
pub struct VecEditorState {
    pub field_index: usize,
    pub items: Vec<String>,
    pub vp: Viewport,
    pub input: InputField,
    pub input_active: bool,
    /// Set by the handler when 's'/Esc commits — the caller (which holds the
    /// taken-out instance) checks this and drops it instead of restoring.
    /// Without this flag the vec editor cannot close because the caller
    /// always restores `form.vec_editor = Some(ve)` after handling the key.
    pub closing: bool,
}

#[derive(Debug)]
pub struct GroupPickerState {
    pub field_index: usize,
    pub available: Vec<String>,
    pub checked: Vec<bool>,
    pub vp: Viewport,
    pub closing: bool,
    pub descriptions: Vec<String>, // empty = no descriptions shown
    pub allow_add: bool,
    pub add_input: InputField,
    pub add_input_active: bool,
}

// New direct sub-popup states (Step 1)
#[derive(Debug, Clone)]
pub struct DirectVecEditorState {
    pub field_index: usize,
    pub sidebar_item: SidebarItem,
    pub field_key: String,
    pub items: Vec<String>,
    pub vp: Viewport,
    pub input: InputField,
    pub input_active: bool,
}

#[derive(Debug, Clone)]
pub struct DirectGroupPickerState {
    pub field_index: usize,
    pub sidebar_item: SidebarItem,
    pub field_key: String,
    pub available: Vec<String>,
    pub checked: Vec<bool>,
    /// Per-row description shown next to each option. Empty = no descriptions
    /// (matches groups-mode picker; Check.enabled mode populates it from
    /// `CHECK_ENABLED_OPTIONS`).
    pub descriptions: Vec<String>,
    /// `false` for fixed catalogs (e.g. `CheckEnabled`); `a` is a no-op then.
    pub allow_add: bool,
    pub vp: Viewport,
    pub add_input: InputField,
    pub add_input_active: bool,
}

impl EntryFormState {
    pub fn new_host(template: &HostEntry) -> Self {
        let fields = host_fields(template);
        let count = fields.len();
        let mut vp = Viewport::new();
        vp.set_dims(count, 0);
        Self {
            kind: EntryFormKind::Host,
            edit_index: None,
            fields,
            field_vp: vp,
            active_input: None,
            input: InputField::new(""),
            vec_editor: None,
            group_picker: None,
            dirty: false,
        }
    }

    pub fn new_check(template: &CheckEntry) -> Self {
        let fields = check_fields(template);
        let count = fields.len();
        let mut vp = Viewport::new();
        vp.set_dims(count, 0);
        Self {
            kind: EntryFormKind::Check,
            edit_index: None,
            fields,
            field_vp: vp,
            active_input: None,
            input: InputField::new(""),
            vec_editor: None,
            group_picker: None,
            dirty: false,
        }
    }

    pub fn new_sync(template: &SyncEntry) -> Self {
        let fields = sync_fields(template);
        let count = fields.len();
        let mut vp = Viewport::new();
        vp.set_dims(count, 0);
        Self {
            kind: EntryFormKind::Sync,
            edit_index: None,
            fields,
            field_vp: vp,
            active_input: None,
            input: InputField::new(""),
            vec_editor: None,
            group_picker: None,
            dirty: false,
        }
    }
}

// ── Confirm dialog state ──────────────────────────────────────────────────

#[derive(Debug)]
pub struct ConfirmState {
    pub prompt: String,
    pub action: ConfirmAction,
    pub hints: &'static str,
}

#[derive(Debug)]
pub enum ConfirmAction {
    DeleteEntry { kind: EntryFormKind, index: usize },
    DiscardDirty,
}

// ── Config tab state ──────────────────────────────────────────────────────

pub struct ConfigTabState {
    pub zone: ConfigZone,
    pub items: Vec<SidebarItem>,
    /// Which child-bearing sections are collapsed (children hidden).
    pub collapsed: CollapsedSections,
    pub sidebar_vp: Viewport,
    pub field_vp: Viewport,
    pub reload_banner_until: Option<Instant>,
    pub config_mtime: Option<std::time::SystemTime>,
    pub config_dirty: bool,
    pub editing_field: Option<InputField>,
    pub editing_field_index: usize,
    pub entry_form: Option<EntryFormState>,
    pub confirm: Option<ConfirmState>,
    pub pending_delete: Option<(EntryFormKind, usize)>,
    pub pending_open_editor: bool,
    pub pending_save: bool,
    /// Cursor snapshot captured at every commit site (entry form, direct
    /// popup, inline edit, cycle). Consumed by `app.rs` once after save+reload
    /// via [`consume_pending_snapshot`]. Using a stored snapshot (instead of
    /// re-capturing in `save_config`) is the only way to survive the
    /// commit→reload pipeline, which wipes viewports.
    pub pending_restore_snapshot: Option<ConfigSelectionSnapshot>,
    // direct popups
    pub direct_vec_editor: Option<DirectVecEditorState>,
    pub direct_group_picker: Option<DirectGroupPickerState>,
}

/// Cursor-position snapshot captured before save+reload, restored after.
///
/// `sidebar_idx` covers what spec §7.2 calls `section_idx + entry_idx`: the real
/// state uses a single flat `sidebar_vp` over `Vec<SidebarItem>` (which interleaves
/// section headers and entries), so one index suffices — clamping against
/// `self.items.len()` is equivalent to per-section clamping in the spec's model.
///
/// `entry_form_open` and `vec_editor_field_index` are defensive guards: a save+reload
/// may close the entry form or invalidate the in-form vec editor, in which case the
/// captured indices must NOT be reapplied to a different form.
///
/// Each field is clamped against the post-reload state in `restore_selection`.
#[derive(Default, Clone, Debug)]
pub struct ConfigSelectionSnapshot {
    sidebar_idx: usize,
    /// Outer right-panel field cursor. Restored when the entry form is closed
    /// at the time of restoration (i.e. user committed a direct popup or
    /// inline edit and we want to land back on the same row).
    field_vp_idx: usize,
    /// Entry-form internal field cursor (only meaningful if `entry_form_open`).
    field_idx: Option<usize>,
    entry_form_open: bool,
    vec_editor_idx: Option<usize>,
    vec_editor_field_index: Option<usize>,
    direct_vec_idx: Option<usize>,
}

impl ConfigTabState {
    pub fn new(config: &AppConfig, config_path: Option<&std::path::Path>) -> Self {
        let collapsed = CollapsedSections::default();
        let items = build_sidebar_items(config, &collapsed);
        let mut sidebar_vp = Viewport::new();
        sidebar_vp.set_dims(items.len(), 0);

        let config_mtime =
            config_path.and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());

        Self {
            zone: ConfigZone::Sidebar,
            items,
            collapsed,
            sidebar_vp,
            field_vp: Viewport::new(),
            reload_banner_until: None,
            config_mtime,
            config_dirty: false,
            editing_field: None,
            editing_field_index: 0,
            entry_form: None,
            confirm: None,
            pending_delete: None,
            pending_open_editor: false,
            pending_save: false,
            pending_restore_snapshot: None,
            direct_vec_editor: None,
            direct_group_picker: None,
        }
    }

    /// Take the pending snapshot for `save_config` to consume.
    ///
    /// `app.rs` MUST use this rather than re-capturing — capturing inside
    /// `save_config` happens after `commit_*` already wiped the viewports.
    pub fn consume_pending_snapshot(&mut self) -> Option<ConfigSelectionSnapshot> {
        self.pending_restore_snapshot.take()
    }

    /// Mark config dirty and capture the cursor state for post-reload restore.
    /// Call at every site that mutates `AppConfig` from the UI. This is the
    /// single chokepoint that protects the user's cursor across save+reload.
    ///
    /// Does NOT trigger save — the user explicitly presses `s` from the main
    /// view to flush changes. (Quit also flushes via `flush_dirty_config_to_disk`
    /// as a safety net.) Earlier versions also set `pending_save`; that
    /// autosave-on-mutate path was removed because it masked persistence bugs
    /// that only surfaced on next program start.
    fn mark_dirty(&mut self) {
        if self.pending_restore_snapshot.is_none() {
            self.pending_restore_snapshot = Some(self.capture_selection());
        }
        self.config_dirty = true;
    }

    /// Explicit save trigger from main-view `s` key. Returns true when a
    /// save should be performed (i.e. config is dirty). Used by app.rs to
    /// route the save through its existing save_config() path.
    pub fn request_save_if_dirty(&mut self) -> bool {
        if self.config_dirty {
            self.pending_save = true;
            true
        } else {
            false
        }
    }

    pub(super) fn capture_selection(&self) -> ConfigSelectionSnapshot {
        let mut snap = ConfigSelectionSnapshot {
            sidebar_idx: self.sidebar_vp.selected,
            field_vp_idx: self.field_vp.selected,
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

    pub fn restore_selection(&mut self, snap: ConfigSelectionSnapshot, config: &AppConfig) {
        let clamp = |idx: usize, len: usize| -> usize {
            if len == 0 {
                0
            } else {
                idx.min(len - 1)
            }
        };
        let sidebar_len = self.items.len();
        self.sidebar_vp.selected = clamp(snap.sidebar_idx, sidebar_len);
        self.sidebar_vp
            .set_dims(sidebar_len, self.sidebar_vp.visible_height);

        // Outer right-panel field cursor: restored whenever the entry form is
        // closed at restore time (direct popup commit, inline edit, cycle).
        // The post-reload field list length depends on the current sidebar
        // selection, so derive it before clamping.
        if self.entry_form.is_none() {
            let field_len = self.outer_field_len(config);
            self.field_vp.selected = clamp(snap.field_vp_idx, field_len);
            self.field_vp
                .set_dims(field_len, self.field_vp.visible_height);
        }

        if snap.entry_form_open {
            if let (Some(form), Some(fi)) = (self.entry_form.as_mut(), snap.field_idx) {
                let flen = form.fields.len();
                form.field_vp.selected = clamp(fi, flen);
                form.field_vp.set_dims(flen, form.field_vp.visible_height);
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
        if let (Some(dve), Some(didx)) = (self.direct_vec_editor.as_mut(), snap.direct_vec_idx) {
            let ilen = dve.items.len();
            dve.vp.selected = clamp(didx, ilen);
            dve.vp.set_dims(ilen, dve.vp.visible_height);
        }
    }

    /// Number of fields in the right-panel for the currently-selected sidebar
    /// item. Used by `restore_selection` to clamp `field_vp_idx`.
    fn outer_field_len(&self, config: &AppConfig) -> usize {
        match self.items.get(self.sidebar_vp.selected) {
            Some(SidebarItem::SectionSettings) => settings_fields(&config.settings).len(),
            Some(SidebarItem::Host(i)) => config
                .host
                .get(*i)
                .map(host_fields)
                .map(|f| f.len())
                .unwrap_or(0),
            Some(SidebarItem::Check(i)) => config
                .check
                .get(*i)
                .map(check_fields)
                .map(|f| f.len())
                .unwrap_or(0),
            Some(SidebarItem::Sync(i)) => config
                .sync
                .get(*i)
                .map(sync_fields)
                .map(|f| f.len())
                .unwrap_or(0),
            _ => 0,
        }
    }

    /// Toggle collapse on the focused section header (Hosts/Checks/Syncs).
    /// Rebuilds the sidebar and keeps the cursor on the same section header.
    /// Returns false (no-op) when the focused item is not a collapsible section.
    pub fn toggle_section_collapse(&mut self, config: &AppConfig) -> bool {
        let item = match self.items.get(self.sidebar_vp.selected) {
            Some(it) => it.clone(),
            None => return false,
        };
        match item {
            SidebarItem::SectionHosts => self.collapsed.hosts = !self.collapsed.hosts,
            SidebarItem::SectionChecks => self.collapsed.checks = !self.collapsed.checks,
            SidebarItem::SectionSyncs => self.collapsed.syncs = !self.collapsed.syncs,
            _ => return false,
        }
        self.items = build_sidebar_items(config, &self.collapsed);
        // Section headers keep a stable identity across rebuild, so the cursor
        // never lands on a now-hidden child.
        let new_idx = self.items.iter().position(|it| *it == item).unwrap_or(0);
        self.sidebar_vp
            .set_dims(self.items.len(), self.sidebar_vp.visible_height);
        self.sidebar_vp.selected = new_idx;
        self.reset_field_vp(config);
        true
    }

    pub fn reload(&mut self, config: &AppConfig, config_path: Option<&std::path::Path>) {
        // Preserve viewport selections across rebuild — `restore_selection`
        // (called by `save_config` after this) clamps them against the new
        // dimensions. Previously this wiped `field_vp`, which is why the
        // right-panel cursor jumped to the first row after every save.
        self.items = build_sidebar_items(config, &self.collapsed);
        let new_len = self.items.len();
        self.sidebar_vp
            .set_dims(new_len, self.sidebar_vp.visible_height);
        // field_vp dims depend on the (potentially new) selection; leave them
        // for restore_selection to recompute.
        self.config_mtime =
            config_path.and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
        self.config_dirty = false;
        self.editing_field = None;
        self.entry_form = None;
        self.confirm = None;
        self.pending_open_editor = false;
    }

    pub fn breadcrumb(&self, config: &AppConfig) -> String {
        if self.entry_form.is_some() {
            let suffix = if self.editing_field.is_some() {
                " [EDIT]"
            } else {
                ""
            };
            return format!("Config > Entry Form{suffix}");
        }
        if self.confirm.is_some() {
            return "Config > Confirm".to_string();
        }
        match self.items.get(self.sidebar_vp.selected) {
            None => "Config".to_string(),
            Some(SidebarItem::SectionSettings) => {
                if self.zone == ConfigZone::FieldTable {
                    let fields = settings_fields(&config.settings);
                    let name = fields
                        .get(self.field_vp.selected)
                        .map(|f| f.key.as_str())
                        .unwrap_or("?");
                    let edit = if self.editing_field.is_some() {
                        " [EDIT]"
                    } else {
                        ""
                    };
                    format!("Config > Settings > {name}{edit}")
                } else {
                    "Config > Settings".to_string()
                }
            }
            Some(SidebarItem::SectionHosts) => "Config > Hosts".to_string(),
            Some(SidebarItem::Host(i)) => {
                let name = config.host.get(*i).map(|h| h.name.as_str()).unwrap_or("?");
                if self.zone == ConfigZone::FieldTable {
                    let fields = host_fields(&config.host[*i]);
                    let fname = fields
                        .get(self.field_vp.selected)
                        .map(|f| f.key.as_str())
                        .unwrap_or("?");
                    let edit = if self.editing_field.is_some() {
                        " [EDIT]"
                    } else {
                        ""
                    };
                    format!("Config > Hosts > {name} > {fname}{edit}")
                } else {
                    format!("Config > Hosts > {name}")
                }
            }
            Some(SidebarItem::SectionChecks) => "Config > Checks".to_string(),
            Some(SidebarItem::Check(i)) => {
                let label = entry_label_check(config, *i);
                if self.zone == ConfigZone::FieldTable {
                    let fields = check_fields(&config.check[*i]);
                    let fname = fields
                        .get(self.field_vp.selected)
                        .map(|f| f.key.as_str())
                        .unwrap_or("?");
                    let edit = if self.editing_field.is_some() {
                        " [EDIT]"
                    } else {
                        ""
                    };
                    format!("Config > Checks > {label} > {fname}{edit}")
                } else {
                    format!("Config > Checks > {label}")
                }
            }
            Some(SidebarItem::SectionSyncs) => "Config > Syncs".to_string(),
            Some(SidebarItem::Sync(i)) => {
                let label = entry_label_sync(config, *i);
                if self.zone == ConfigZone::FieldTable {
                    let fields = sync_fields(&config.sync[*i]);
                    let fname = fields
                        .get(self.field_vp.selected)
                        .map(|f| f.key.as_str())
                        .unwrap_or("?");
                    let edit = if self.editing_field.is_some() {
                        " [EDIT]"
                    } else {
                        ""
                    };
                    format!("Config > Syncs > {label} > {fname}{edit}")
                } else {
                    format!("Config > Syncs > {label}")
                }
            }
        }
    }

    /// Returns true when the FieldTable zone cursor is at the first row.
    pub fn field_vp_at_top(&self) -> bool {
        self.field_vp.selected == 0
    }

    /// Returns true when a text input is currently active in the config tab
    /// (inline scalar edit, entry form field input, or vec editor input).
    /// Used by app.rs to suspend global hotkeys.
    #[allow(dead_code)]
    pub fn is_editing_active(&self) -> bool {
        // Previously this only checked for active text inputs. We extend it to
        // return true whenever any popup or interactive input is present so
        // global hotkeys are reliably blocked while config-related popups are
        // open (even if the popup is idle).
        if self.editing_field.is_some() {
            return true;
        }
        if self.entry_form.is_some() {
            return true;
        }
        if self.confirm.is_some() {
            return true;
        }
        if self.direct_vec_editor.is_some() {
            return true;
        }
        if self.direct_group_picker.is_some() {
            return true;
        }
        false
    }

    pub fn is_any_popup_open(&self) -> bool {
        self.entry_form.is_some()
            || self.confirm.is_some()
            || self.editing_field.is_some()
            || self.direct_vec_editor.is_some()
            || self.direct_group_picker.is_some()
    }

    /// Handle keypress. Returns true if dirty/redraw needed.
    /// `config` is mutable here for inline edits.
    pub fn handle_key(&mut self, key: KeyEvent, config: &mut AppConfig) -> bool {
        // Route direct sub-popups first (Step 6)
        if self.direct_group_picker.is_some() {
            return self.handle_direct_group_picker_key(key, config);
        }
        if self.direct_vec_editor.is_some() {
            return self.handle_direct_vec_editor_key(key, config);
        }

        if self.entry_form.is_some() {
            return self.handle_entry_form_key(key, config);
        }
        if self.confirm.is_some() {
            return self.handle_confirm_key(key);
        }
        if let Some(mut input) = self.editing_field.take() {
            let handled = self.handle_inline_edit_key(key, &mut input, config);
            if input.mode == InputMode::Active {
                self.editing_field = Some(input);
            }
            return handled;
        }
        match self.zone {
            ConfigZone::Sidebar => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.sidebar_vp.move_up();
                    self.reset_field_vp(config);
                    true
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.sidebar_vp.move_down();
                    self.reset_field_vp(config);
                    true
                }
                KeyCode::PageUp => {
                    self.sidebar_vp.page_up();
                    self.reset_field_vp(config);
                    true
                }
                KeyCode::PageDown => {
                    self.sidebar_vp.page_down();
                    self.reset_field_vp(config);
                    true
                }
                KeyCode::Home => {
                    self.sidebar_vp.home();
                    self.reset_field_vp(config);
                    true
                }
                KeyCode::End => {
                    self.sidebar_vp.end();
                    self.reset_field_vp(config);
                    true
                }
                KeyCode::Right | KeyCode::Tab => {
                    self.zone = ConfigZone::FieldTable;
                    true
                }
                KeyCode::Char('e') => {
                    self.start_edit_entry(config);
                    true
                }
                KeyCode::Char('s') => {
                    self.request_save_if_dirty();
                    true
                }
                KeyCode::Char(' ') => {
                    // Space toggles collapse on a section header; no-op elsewhere.
                    self.toggle_section_collapse(config)
                }
                KeyCode::Enter => {
                    match self.items.get(self.sidebar_vp.selected).cloned() {
                        Some(
                            SidebarItem::Host(_) | SidebarItem::Check(_) | SidebarItem::Sync(_),
                        ) => {
                            self.zone = ConfigZone::FieldTable;
                        }
                        Some(
                            SidebarItem::SectionHosts
                            | SidebarItem::SectionChecks
                            | SidebarItem::SectionSyncs,
                        ) => {
                            self.toggle_section_collapse(config);
                        }
                        _ => {}
                    }
                    true
                }
                _ => false,
            },
            ConfigZone::FieldTable => match key.code {
                KeyCode::Char('s') => {
                    // Explicit save (matches Sidebar 's'). Only fires if
                    // there's a dirty change to flush.
                    self.request_save_if_dirty();
                    true
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.field_vp.move_up();
                    true
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.field_vp.move_down();
                    true
                }
                KeyCode::PageUp => {
                    self.field_vp.page_up();
                    true
                }
                KeyCode::PageDown => {
                    self.field_vp.page_down();
                    true
                }
                KeyCode::Home => {
                    self.field_vp.home();
                    true
                }
                KeyCode::End => {
                    self.field_vp.end();
                    true
                }
                KeyCode::Left | KeyCode::BackTab => {
                    // Left/Right no longer cycle option fields (Space/Enter do
                    // that now); Left returns to the sidebar zone.
                    self.zone = ConfigZone::Sidebar;
                    true
                }
                KeyCode::Char(' ') => {
                    let fields = self.current_descriptors(config);
                    if let Some(f) = fields.get(self.field_vp.selected) {
                        if let Some(new_val) = cycle_option_value(&f.kind, &f.display_value) {
                            self.editing_field_index = self.field_vp.selected;
                            self.commit_inline_edit(&new_val, config);
                            self.mark_dirty();
                            return true;
                        }
                    }
                    false
                }
                KeyCode::Char('e') | KeyCode::Enter => {
                    let field_idx = self.field_vp.selected;
                    let fields = self.current_descriptors(config);
                    if let Some(f) = fields.get(field_idx) {
                        if let Some(new_val) = cycle_option_value(&f.kind, &f.display_value) {
                            self.editing_field_index = field_idx;
                            self.commit_inline_edit(&new_val, config);
                            self.mark_dirty();
                            return true;
                        }
                        match &f.kind {
                            FieldKind::VecString
                            | FieldKind::VecCheckPath
                            | FieldKind::CheckEnabled => {
                                // Direct sub-popup. Three modes:
                                //   - CheckEnabled  → group picker over fixed CHECK_ENABLED_OPTIONS
                                //   - VecString/groups → group picker over collect_known_groups
                                //   - anything else → free-text vec editor
                                let field_key = f.key.clone();
                                let current_val = f.display_value.clone();
                                let sidebar_item = self.items[self.sidebar_vp.selected].clone();
                                let field_index = self.field_vp.selected;
                                let current = parse_bracket_list(&current_val);
                                match f.kind {
                                    FieldKind::CheckEnabled => {
                                        let available: Vec<String> = CHECK_ENABLED_OPTIONS
                                            .iter()
                                            .map(|(k, _)| (*k).to_string())
                                            .collect();
                                        let descriptions: Vec<String> = CHECK_ENABLED_OPTIONS
                                            .iter()
                                            .map(|(_, d)| (*d).to_string())
                                            .collect();
                                        let checked: Vec<bool> =
                                            available.iter().map(|a| current.contains(a)).collect();
                                        let mut vp = Viewport::new();
                                        vp.set_dims(available.len(), 0);
                                        self.direct_group_picker = Some(DirectGroupPickerState {
                                            field_index,
                                            sidebar_item,
                                            field_key,
                                            available,
                                            checked,
                                            descriptions,
                                            allow_add: false,
                                            vp,
                                            add_input: InputField::new(""),
                                            add_input_active: false,
                                        });
                                    }
                                    FieldKind::VecString if field_key == "groups" => {
                                        let (available, checked) =
                                            collect_known_groups(config, &current);
                                        let mut vp = Viewport::new();
                                        vp.set_dims(available.len(), 0);
                                        self.direct_group_picker = Some(DirectGroupPickerState {
                                            field_index,
                                            sidebar_item,
                                            field_key,
                                            available,
                                            checked,
                                            descriptions: vec![],
                                            allow_add: true,
                                            vp,
                                            add_input: InputField::new(""),
                                            add_input_active: false,
                                        });
                                    }
                                    _ => {
                                        let items = current;
                                        let mut vp = Viewport::new();
                                        vp.set_dims(items.len(), 0);
                                        self.direct_vec_editor = Some(DirectVecEditorState {
                                            field_index,
                                            sidebar_item,
                                            field_key,
                                            items,
                                            vp,
                                            input: InputField::new(""),
                                            input_active: false,
                                        });
                                    }
                                }
                                return true;
                            }
                            _ => {}
                        }
                    }
                    self.activate_inline_edit(config)
                }
                _ => false,
            },
        }
    }

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
                self.mark_dirty();
            }
            return true;
        }
        if key.code == KeyCode::Esc {
            self.editing_field = None;
            return true;
        }
        false
    }

    fn activate_inline_edit(&mut self, config: &AppConfig) -> bool {
        let fields = self.current_descriptors(config);
        let idx = self.field_vp.selected;
        if idx >= fields.len() {
            return false;
        }
        let field = &fields[idx];
        if !field.editable {
            return false;
        }
        match &field.kind {
            FieldKind::VecString
            | FieldKind::VecCheckPath
            | FieldKind::TriBool
            | FieldKind::Bool
            | FieldKind::ShellEnum
            | FieldKind::Enum { .. } => return false,
            _ => {}
        }
        let raw_value = strip_unit(&field.display_value);
        let mut input = InputField::new(&raw_value);
        input.activate();
        self.editing_field = Some(input);
        self.editing_field_index = idx;
        true
    }

    fn commit_inline_edit(&mut self, new_value: &str, config: &mut AppConfig) {
        let item = match self.items.get(self.sidebar_vp.selected) {
            Some(i) => i.clone(),
            None => return,
        };
        let idx = self.editing_field_index;
        // Look up the key from the current schema. The index→key indirection
        // is the bridge between the viewport (index-based) and apply (key-based).
        match item {
            SidebarItem::SectionSettings => {
                if let Some(key) = settings_fields(&config.settings)
                    .get(idx)
                    .map(|f| f.key.clone())
                {
                    apply_settings(config, &key, new_value);
                }
            }
            SidebarItem::Host(i) => {
                let key = config
                    .host
                    .get(i)
                    .and_then(|h| host_fields(h).get(idx).map(|f| f.key.clone()));
                if let (Some(h), Some(key)) = (config.host.get_mut(i), key) {
                    apply_host(h, &key, new_value);
                }
            }
            SidebarItem::Check(i) => {
                let key = config
                    .check
                    .get(i)
                    .and_then(|c| check_fields(c).get(idx).map(|f| f.key.clone()));
                if let (Some(c), Some(key)) = (config.check.get_mut(i), key) {
                    apply_check(c, &key, new_value);
                }
            }
            SidebarItem::Sync(i) => {
                let key = config
                    .sync
                    .get(i)
                    .and_then(|s| sync_fields(s).get(idx).map(|f| f.key.clone()));
                if let (Some(s), Some(key)) = (config.sync.get_mut(i), key) {
                    apply_sync(s, &key, new_value);
                }
            }
            _ => {}
        }
    }

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
            let mut ve = self.entry_form.as_mut().unwrap().vec_editor.take().unwrap();
            let handled = self.handle_vec_editor_key(key, &mut ve);
            if !ve.closing {
                if let Some(form) = self.entry_form.as_mut() {
                    form.vec_editor = Some(ve);
                }
            }
            return handled;
        }

        if self
            .entry_form
            .as_ref()
            .map(|f| f.group_picker.is_some())
            .unwrap_or(false)
        {
            let mut gp = self
                .entry_form
                .as_mut()
                .unwrap()
                .group_picker
                .take()
                .unwrap();
            let handled = self.handle_group_picker_key(key, &mut gp);
            if !gp.closing {
                if let Some(form) = self.entry_form.as_mut() {
                    form.group_picker = Some(gp);
                }
            }
            return handled;
        }

        let form = self.entry_form.as_mut().unwrap();

        if form.active_input.is_some() {
            let is_cancel = key.code == KeyCode::Esc;
            form.input.handle_key(key);
            if form.input.mode == InputMode::Normal {
                let idx = form.active_input.unwrap();
                form.fields[idx].display_value = form.input.value.clone();
                if !is_cancel {
                    form.dirty = true;
                }
                form.active_input = None;
            }
            return true;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                form.field_vp.move_up();
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                form.field_vp.move_down();
                true
            }
            KeyCode::Char(' ') => {
                // Space cycles any option field (Bool, TriBool, ShellEnum,
                // Enum); Left/Right are left free for navigation.
                let idx = form.field_vp.selected;
                if idx < form.fields.len() && form.fields[idx].editable {
                    if let Some(new_val) =
                        cycle_option_value(&form.fields[idx].kind, &form.fields[idx].display_value)
                    {
                        form.fields[idx].display_value = new_val;
                        form.dirty = true;
                        return true;
                    }
                }
                false
            }
            KeyCode::Char('e') | KeyCode::Enter => {
                let idx = form.field_vp.selected;
                if idx < form.fields.len() {
                    let field = &form.fields[idx];
                    if field.editable {
                        if let Some(new_val) = cycle_option_value(&field.kind, &field.display_value)
                        {
                            form.fields[idx].display_value = new_val;
                            form.dirty = true;
                            return true;
                        }
                        match &field.kind {
                            FieldKind::CheckEnabled => {
                                let current = parse_bracket_list(&form.fields[idx].display_value);
                                let available: Vec<String> = CHECK_ENABLED_OPTIONS
                                    .iter()
                                    .map(|(k, _)| k.to_string())
                                    .collect();
                                let checked: Vec<bool> =
                                    available.iter().map(|k| current.contains(k)).collect();
                                let mut vp = Viewport::new();
                                vp.set_dims(available.len(), 0);
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
                                    allow_add: false,
                                    add_input: InputField::new(""),
                                    add_input_active: false,
                                });
                            }
                            FieldKind::VecString | FieldKind::VecCheckPath => {
                                if field.key == "groups" {
                                    let current =
                                        parse_bracket_list(&form.fields[idx].display_value);
                                    let (available, checked) =
                                        collect_known_groups(config, &current);
                                    let mut vp = Viewport::new();
                                    vp.set_dims(available.len(), 0);
                                    form.group_picker = Some(GroupPickerState {
                                        field_index: idx,
                                        available,
                                        checked,
                                        vp,
                                        closing: false,
                                        descriptions: vec![],
                                        allow_add: true,
                                        add_input: InputField::new(""),
                                        add_input_active: false,
                                    });
                                } else {
                                    let items = parse_bracket_list(&field.display_value);
                                    let mut ve = VecEditorState {
                                        field_index: idx,
                                        items,
                                        vp: Viewport::new(),
                                        input: InputField::new(""),
                                        input_active: false,
                                        closing: false,
                                    };
                                    ve.vp.set_dims(ve.items.len(), 0);
                                    form.vec_editor = Some(ve);
                                }
                            }
                            _ => {
                                let raw = strip_unit(&field.display_value);
                                form.input = InputField::new(&raw);
                                form.input.activate();
                                form.active_input = Some(idx);
                            }
                        }
                    }
                }
                true
            }
            KeyCode::Char('s') => {
                // commit_entry_form already marks dirty (sets pending_save +
                // captures snapshot) and takes self.entry_form.
                self.commit_entry_form(config);
                true
            }
            KeyCode::Esc => {
                if form.dirty {
                    self.confirm = Some(ConfirmState {
                        prompt: "Discard unsaved changes?".to_string(),
                        action: ConfirmAction::DiscardDirty,
                        hints: "[y/Enter] Yes   [n/Esc] No",
                    });
                } else {
                    self.entry_form = None;
                }
                true
            }
            _ => false,
        }
    }

    fn handle_vec_editor_key(&mut self, key: KeyEvent, ve: &mut VecEditorState) -> bool {
        if ve.input_active {
            ve.input.handle_key(key);
            if ve.input.mode == InputMode::Normal {
                if !ve.input.value.is_empty() {
                    ve.items.push(std::mem::take(&mut ve.input.value));
                    ve.vp.set_dims(ve.items.len(), 0);
                }
                ve.input_active = false;
            }
            return true;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                ve.vp.move_up();
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                ve.vp.move_down();
                true
            }
            KeyCode::PageUp => {
                ve.vp.page_up();
                true
            }
            KeyCode::PageDown => {
                ve.vp.page_down();
                true
            }
            KeyCode::Home => {
                ve.vp.home();
                true
            }
            KeyCode::End => {
                ve.vp.end();
                true
            }
            KeyCode::Char('a') | KeyCode::Enter => {
                ve.input = InputField::new("");
                ve.input.activate();
                ve.input_active = true;
                true
            }
            KeyCode::Char('d') => {
                let idx = ve.vp.selected;
                if idx < ve.items.len() {
                    ve.items.remove(idx);
                    ve.vp.set_dims(ve.items.len(), 0);
                    if ve.vp.selected >= ve.items.len() && ve.vp.selected > 0 {
                        ve.vp.move_up();
                    }
                }
                true
            }
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
                // Setting form.vec_editor = None here is a no-op because the
                // caller already `take()`d it; flag closing so the caller
                // drops the local instead of restoring it.
                ve.closing = true;
                true
            }
            _ => true, // swallow — prevent global keys (q, ?) from firing
        }
    }

    fn handle_group_picker_key(&mut self, key: KeyEvent, gp: &mut GroupPickerState) -> bool {
        // If add-input is active, route keys to it first
        if gp.add_input_active {
            // If the add-input is active, handle Esc as an explicit cancel before
            // routing the key to the input field. This avoids treating Esc the
            // same as Enter (both flip mode -> Normal) and accidentally applying
            // the pending value when the user intended to cancel.
            if key.code == KeyCode::Esc {
                gp.add_input = InputField::new("");
                gp.add_input_active = false;
                return true;
            }
            gp.add_input.handle_key(key);
            if gp.add_input.mode == InputMode::Normal {
                apply_add_input_to_picker(
                    &gp.add_input.value,
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
            KeyCode::PageUp => {
                gp.vp.page_up();
                true
            }
            KeyCode::PageDown => {
                gp.vp.page_down();
                true
            }
            KeyCode::Home => {
                gp.vp.home();
                true
            }
            KeyCode::End => {
                gp.vp.end();
                true
            }
            KeyCode::Char(' ') => {
                let idx = gp.vp.selected;
                if idx < gp.checked.len() {
                    gp.checked[idx] = !gp.checked[idx];
                }
                true
            }
            KeyCode::Char('a') if gp.allow_add => {
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
            _ => true, // swallow — prevent global keys (q, ?) from firing
        }
    }

    fn handle_confirm_key(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let confirm = self.confirm.take().unwrap();
                match confirm.action {
                    ConfirmAction::DeleteEntry { kind, index } => {
                        // Delete is handled by execute_delete which is called from App
                        // Re-store for App to pick up.
                        self.pending_delete = Some((kind, index));
                    }
                    ConfirmAction::DiscardDirty => {
                        self.entry_form = None;
                        self.editing_field = None;
                    }
                }
                true
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.confirm = None;
                true
            }
            _ => false,
        }
    }

    fn commit_entry_form(&mut self, config: &mut AppConfig) {
        // Snapshot BEFORE any state mutation. After this point the entry form
        // closes and the sidebar/field viewports get rebuilt; capturing later
        // would lose the user's cursor position.
        self.mark_dirty();
        let form = self.entry_form.take().unwrap();
        // Single dispatch: feed every form field through the unified apply().
        // Same code path as right-panel inline edits.
        match form.kind {
            EntryFormKind::Host => {
                let mut h = if let Some(idx) = form.edit_index {
                    config.host[idx].clone()
                } else {
                    HostEntry {
                        name: String::new(),
                        ssh_host: String::new(),
                        shell: ShellType::Sh,
                        groups: vec![],
                        proxy_jump: None,
                    }
                };
                for f in &form.fields {
                    apply_host(&mut h, &f.key, &f.display_value);
                }
                if let Some(idx) = form.edit_index {
                    config.host[idx] = h;
                } else {
                    config.host.push(h);
                }
            }
            EntryFormKind::Check => {
                let mut c = if let Some(idx) = form.edit_index {
                    config.check[idx].clone()
                } else {
                    CheckEntry {
                        name: None,
                        id: generate_entry_id("check"),
                        enabled: vec![],
                        path: vec![],
                        groups: vec![],
                        enable_hosts: true,
                        enable_all: true,
                    }
                };
                for f in &form.fields {
                    apply_check(&mut c, &f.key, &f.display_value);
                }
                if let Some(idx) = form.edit_index {
                    config.check[idx] = c;
                } else {
                    config.check.push(c);
                }
            }
            EntryFormKind::Sync => {
                let mut s = if let Some(idx) = form.edit_index {
                    config.sync[idx].clone()
                } else {
                    SyncEntry {
                        name: None,
                        id: generate_entry_id("sync"),
                        paths: vec![],
                        groups: vec![],
                        enable_hosts: true,
                        enable_all: true,
                        recursive: false,
                        mode: None,
                        propagate_deletes: None,
                        source: None,
                    }
                };
                for f in &form.fields {
                    apply_sync(&mut s, &f.key, &f.display_value);
                }
                if let Some(idx) = form.edit_index {
                    config.sync[idx] = s;
                } else {
                    config.sync.push(s);
                }
            }
        }
        // A freshly added entry must be visible — expand its section so the
        // new row isn't hidden by a prior collapse.
        if form.edit_index.is_none() {
            match form.kind {
                EntryFormKind::Host => self.collapsed.hosts = false,
                EntryFormKind::Check => self.collapsed.checks = false,
                EntryFormKind::Sync => self.collapsed.syncs = false,
            }
        }
        // Rebuild sidebar items but preserve viewport selections (clamp later
        // in restore_selection). `mark_dirty` above already captured the
        // pre-rebuild indices into pending_restore_snapshot.
        let items = build_sidebar_items(config, &self.collapsed);
        self.items = items;
        self.sidebar_vp
            .set_dims(self.items.len(), self.sidebar_vp.visible_height);
    }

    pub fn execute_delete(&mut self, config: &mut AppConfig, kind: EntryFormKind, index: usize) {
        self.mark_dirty();
        match kind {
            EntryFormKind::Host => {
                if index < config.host.len() {
                    config.host.remove(index);
                }
            }
            EntryFormKind::Check => {
                if index < config.check.len() {
                    config.check.remove(index);
                }
            }
            EntryFormKind::Sync => {
                if index < config.sync.len() {
                    config.sync.remove(index);
                }
            }
        }
        let items = build_sidebar_items(config, &self.collapsed);
        self.items = items;
        self.sidebar_vp
            .set_dims(self.items.len(), self.sidebar_vp.visible_height);
    }

    pub fn start_add_entry(&mut self, kind: EntryFormKind) {
        let form = match kind {
            EntryFormKind::Host => EntryFormState::new_host(&HostEntry {
                name: String::new(),
                ssh_host: String::new(),
                shell: ShellType::Sh,
                groups: vec![],
                proxy_jump: None,
            }),
            EntryFormKind::Check => EntryFormState::new_check(&CheckEntry {
                name: None,
                id: generate_entry_id("check"),
                enabled: vec![],
                path: vec![],
                groups: vec![],
                enable_hosts: true,
                enable_all: true,
            }),
            EntryFormKind::Sync => EntryFormState::new_sync(&SyncEntry {
                name: None,
                id: generate_entry_id("sync"),
                paths: vec![],
                groups: vec![],
                enable_hosts: true,
                enable_all: true,
                recursive: false,
                mode: None,
                propagate_deletes: None,
                source: None,
            }),
        };
        self.entry_form = Some(form);
    }

    pub fn start_edit_entry(&mut self, config: &AppConfig) {
        let item = match self.items.get(self.sidebar_vp.selected) {
            Some(i) => i.clone(),
            None => return,
        };
        let form = match item {
            SidebarItem::Host(i) => {
                if let Some(h) = config.host.get(i) {
                    let mut f = EntryFormState::new_host(h);
                    f.edit_index = Some(i);
                    Some(f)
                } else {
                    None
                }
            }
            SidebarItem::Check(i) => {
                if let Some(c) = config.check.get(i) {
                    let mut f = EntryFormState::new_check(c);
                    f.edit_index = Some(i);
                    Some(f)
                } else {
                    None
                }
            }
            SidebarItem::Sync(i) => {
                if let Some(s) = config.sync.get(i) {
                    let mut f = EntryFormState::new_sync(s);
                    f.edit_index = Some(i);
                    Some(f)
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(form) = form {
            self.entry_form = Some(form);
        }
    }

    pub fn request_delete(&mut self) {
        let item = match self.items.get(self.sidebar_vp.selected) {
            Some(i) => i.clone(),
            None => return,
        };
        match item {
            SidebarItem::Host(i) => {
                self.confirm = Some(ConfirmState {
                    prompt: format!("Delete host entry #{},?", i + 1),
                    action: ConfirmAction::DeleteEntry {
                        kind: EntryFormKind::Host,
                        index: i,
                    },
                    hints: "[y/Enter] Yes   [n/Esc] No",
                });
            }
            SidebarItem::Check(i) => {
                self.confirm = Some(ConfirmState {
                    prompt: format!("Delete check entry #{},?", i + 1),
                    action: ConfirmAction::DeleteEntry {
                        kind: EntryFormKind::Check,
                        index: i,
                    },
                    hints: "[y/Enter] Yes   [n/Esc] No",
                });
            }
            SidebarItem::Sync(i) => {
                self.confirm = Some(ConfirmState {
                    prompt: format!("Delete sync entry #{},?", i + 1),
                    action: ConfirmAction::DeleteEntry {
                        kind: EntryFormKind::Sync,
                        index: i,
                    },
                    hints: "[y/Enter] Yes   [n/Esc] No",
                });
            }
            _ => {}
        }
    }

    pub fn render(
        &mut self,
        area: Rect,
        frame: &mut Frame,
        theme: &Theme,
        config: &AppConfig,
        config_path: Option<&std::path::Path>,
        navbar_focused: bool,
    ) {
        // If direct sub-popups are open, render base screen + direct popup (Step 11)
        if self.direct_vec_editor.is_some() || self.direct_group_picker.is_some() {
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
            if let Some(ref dve) = self.direct_vec_editor {
                self.render_direct_vec_editor(area, frame, theme, dve);
            } else if let Some(ref dgp) = self.direct_group_picker {
                self.render_direct_group_picker(area, frame, theme, dgp);
            }
            return;
        }

        if let Some(ref form) = self.entry_form {
            self.render_entry_form(area, frame, theme, form);
            // Confirm dialog overlays the form (e.g. discard prompt on ESC).
            if let Some(ref confirm) = self.confirm {
                self.render_confirm(area, frame, theme, confirm);
            }
            return;
        }
        if let Some(ref confirm) = self.confirm {
            self.render_confirm(area, frame, theme, confirm);
            return;
        }

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
    }

    pub fn banner_active(&self) -> bool {
        self.reload_banner_until
            .map(|t| std::time::Instant::now() < t)
            .unwrap_or(false)
    }

    fn render_entry_form(
        &self,
        area: Rect,
        frame: &mut Frame,
        theme: &Theme,
        form: &EntryFormState,
    ) {
        let popup_area = centered_rect(70, 70, area);
        frame.render_widget(Clear, popup_area);

        let title = match form.kind {
            EntryFormKind::Host => " Add/Edit Host ",
            EntryFormKind::Check => " Add/Edit Check ",
            EntryFormKind::Sync => " Add/Edit Sync ",
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.accent_config))
            .title(title);
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let visible_h = inner.height as usize;
        let count = form.fields.len();
        let mut vp = form.field_vp.clone();
        vp.set_dims(count, visible_h);

        let (start, end) = vp.visible_range();

        let mut lines: Vec<Line> = Vec::new();

        if let Some(ref gp) = form.group_picker {
            let picker_title = if gp.descriptions.is_empty() {
                "  Pick groups  (Space:toggle  a:add  Enter/s:apply  Esc:cancel)".to_string()
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
            lines.push(Line::from(""));

            let gp_visible_h = visible_h.saturating_sub(5);
            let mut gp_vp = gp.vp.clone();
            gp_vp.set_dims(gp.available.len(), gp_visible_h);
            if gp.available.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  (no known groups — define groups on hosts first)",
                    Style::default().fg(theme.inactive),
                )));
            } else {
                for (rel, group) in gp_vp.visible_slice(&gp.available).iter().enumerate() {
                    let abs = gp_vp.scroll_y + rel;
                    let is_sel = abs == gp_vp.selected;
                    let checked = gp.checked.get(abs).copied().unwrap_or(false);
                    let mark = if checked { "◉" } else { "○" };
                    let style = if is_sel {
                        Style::default()
                            .fg(theme.accent_config)
                            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                    } else {
                        Style::default()
                    };
                    let desc = gp.descriptions.get(abs).map(|d| d.as_str()).unwrap_or("");
                    if desc.is_empty() {
                        lines.push(Line::from(Span::styled(format!("  {mark} {group}"), style)));
                    } else {
                        let dim = Style::default().fg(theme.border_inactive);
                        lines.push(Line::from(vec![
                            Span::styled(format!("  {mark} {group}"), style),
                            Span::styled(format!("  — {desc}"), dim),
                        ]));
                    }
                }
            }
            if gp.add_input_active {
                lines.push(Line::from(""));
                let accent = Style::default()
                    .fg(theme.accent_config)
                    .add_modifier(Modifier::BOLD);
                lines.push(input_cursor_line(
                    &gp.add_input,
                    Span::styled("  New group: ", accent),
                    accent,
                ));
            }
        } else if let Some(ref ve) = form.vec_editor {
            lines.push(Line::from(Span::styled(
                format!(
                    "  Editing: {} (a:add d:del s/Esc:done)",
                    form.fields[ve.field_index].key
                ),
                Style::default().fg(theme.warning),
            )));
            lines.push(Line::from(""));

            let ve_visible_h = visible_h.saturating_sub(6);
            let mut ve_vp = ve.vp.clone();
            ve_vp.set_dims(ve.items.len(), ve_visible_h);
            let scroll_y = ve_vp.scroll_y;
            for (rel, item) in ve_vp.visible_slice(&ve.items).iter().enumerate() {
                let abs = scroll_y + rel;
                let is_sel = abs == ve_vp.selected;
                let style = if is_sel {
                    Style::default()
                        .fg(theme.accent_config)
                        .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                } else {
                    Style::default()
                };
                let prefix = if is_sel { "▶ " } else { "  " };
                lines.push(Line::from(Span::styled(format!("{prefix}{item}"), style)));
            }

            if ve.items.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  (empty)",
                    Style::default().fg(theme.inactive),
                )));
            }

            if ve.input_active {
                lines.push(Line::from(""));
                let accent = Style::default()
                    .fg(theme.accent_config)
                    .add_modifier(Modifier::BOLD);
                lines.push(input_cursor_line(
                    &ve.input,
                    Span::styled("  New: ", accent),
                    accent,
                ));
            }
        } else {
            for rel in 0..(end - start) {
                let abs = start + rel;
                if abs >= form.fields.len() {
                    break;
                }
                let field = &form.fields[abs];
                let is_sel = abs == vp.selected;
                let is_editing = form.active_input == Some(abs);

                let key_style = if is_sel {
                    Style::default()
                        .fg(theme.accent_config)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.inactive)
                };

                let val = if is_editing {
                    format!("{}▏", form.input.value)
                } else {
                    field.display_value.clone()
                };
                let val_style = if is_editing {
                    Style::default()
                        .fg(theme.accent_config)
                        .add_modifier(Modifier::BOLD)
                } else if is_sel {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                let prefix = if is_sel { "▶ " } else { "  " };
                let max_w = inner.width as usize;
                let key_str = trunc(&format!("{prefix}{} = ", field.key), max_w / 3);
                let val_str = trunc(&val, max_w.saturating_sub(key_str.width()));

                lines.push(Line::from(vec![
                    Span::styled(key_str, key_style),
                    Span::styled(val_str, val_style),
                ]));
            }
        }

        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  [Enter/e] Edit  [Space] Cycle option  [s] Save  [Esc] Cancel",
            Style::default().fg(theme.inactive),
        )));

        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_confirm(&self, area: Rect, frame: &mut Frame, theme: &Theme, confirm: &ConfirmState) {
        let popup_area = centered_rect(50, 20, area);
        frame.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.warning))
            .title(" Confirm ");
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", confirm.prompt),
                Style::default()
                    .fg(theme.warning)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", confirm.hints),
                Style::default().fg(theme.inactive),
            )),
        ];
        frame.render_widget(Paragraph::new(lines), inner);
    }

    pub fn reset_field_vp(&mut self, config: &AppConfig) {
        let count = self.current_descriptors(config).len();
        self.field_vp = Viewport::new();
        self.field_vp.set_dims(count, 0);
    }

    pub fn current_descriptors(&self, config: &AppConfig) -> Vec<FieldDescriptor> {
        match self.items.get(self.sidebar_vp.selected) {
            None => vec![],
            Some(SidebarItem::SectionSettings) => settings_fields(&config.settings),
            Some(SidebarItem::SectionHosts) => {
                vec![FieldDescriptor::readonly(
                    "hosts",
                    format!("{} configured", config.host.len()),
                )]
            }
            Some(SidebarItem::Host(i)) => config.host.get(*i).map(host_fields).unwrap_or_default(),
            Some(SidebarItem::SectionChecks) => {
                vec![FieldDescriptor::readonly(
                    "checks",
                    format!("{} configured", config.check.len()),
                )]
            }
            Some(SidebarItem::Check(i)) => {
                config.check.get(*i).map(check_fields).unwrap_or_default()
            }
            Some(SidebarItem::SectionSyncs) => {
                vec![FieldDescriptor::readonly(
                    "syncs",
                    format!("{} configured", config.sync.len()),
                )]
            }
            Some(SidebarItem::Sync(i)) => config.sync.get(*i).map(sync_fields).unwrap_or_default(),
        }
    }

    fn render_sidebar(
        &mut self,
        area: Rect,
        frame: &mut Frame,
        theme: &Theme,
        config: &AppConfig,
        navbar_focused: bool,
    ) {
        let focused = !navbar_focused && self.zone == ConfigZone::Sidebar;
        let border_style = Style::default().fg(if focused {
            theme.accent_config
        } else {
            theme.border_inactive
        });
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(" Config ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let visible_h = inner.height as usize;
        self.sidebar_vp.set_dims(self.items.len(), visible_h);

        let max_w = inner.width.saturating_sub(1) as usize;
        let (start, end) = self.sidebar_vp.visible_range();

        let lines: Vec<Line> = self.items[start..end]
            .iter()
            .enumerate()
            .map(|(rel, item)| {
                let abs = start + rel;
                let is_sel = abs == self.sidebar_vp.selected;

                let (prefix, text, is_header) = sidebar_item_display(item, config);
                // Collapsible section headers show a ▼/▶ disclosure triangle in
                // the cursor column; selection is conveyed by style alone
                // (reverse when focused, bold otherwise). Other rows keep the
                // normal selection cursor.
                let glyph = if let Some(g) = collapse_glyph(item, &self.collapsed) {
                    g
                } else if is_sel && focused {
                    "▶ "
                } else if is_sel {
                    "> "
                } else {
                    "  "
                };
                let label = trunc(&format!("{glyph}{prefix}{text}"), max_w);

                let style = if is_sel && focused {
                    Style::default()
                        .fg(theme.accent_config)
                        .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                } else if is_sel {
                    Style::default().add_modifier(Modifier::BOLD)
                } else if is_header {
                    Style::default().fg(theme.accent_config)
                } else {
                    Style::default()
                };

                Line::from(Span::styled(label, style))
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_field_table(
        &mut self,
        area: Rect,
        frame: &mut Frame,
        theme: &Theme,
        config: &AppConfig,
        navbar_focused: bool,
    ) {
        let focused = !navbar_focused && self.zone == ConfigZone::FieldTable;
        let border_style = Style::default().fg(if focused {
            theme.accent_config
        } else {
            theme.border_inactive
        });
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let fields = self.current_descriptors(config);
        let visible_h = inner.height as usize;
        self.field_vp.set_dims(fields.len(), visible_h);

        if fields.is_empty() {
            let msg = match self.items.get(self.sidebar_vp.selected) {
                Some(SidebarItem::SectionHosts) if config.host.is_empty() => {
                    "(no hosts configured — press 'a' to add)"
                }
                Some(SidebarItem::SectionHosts) => "(select a host entry in the sidebar  ↑↓)",
                Some(SidebarItem::SectionChecks) if config.check.is_empty() => {
                    "(no [[check]] entries — press 'a' to add)"
                }
                Some(SidebarItem::SectionChecks) => "(select a check entry in the sidebar  ↑↓)",
                Some(SidebarItem::SectionSyncs) if config.sync.is_empty() => {
                    "(no [[sync]] entries — press 'a' to add)"
                }
                Some(SidebarItem::SectionSyncs) => "(select a sync entry in the sidebar  ↑↓)",
                _ => "(nothing to show)",
            };
            frame.render_widget(
                Paragraph::new(Span::styled(msg, Style::default().fg(theme.inactive))),
                inner,
            );
            return;
        }

        let key_w = fields
            .iter()
            .map(|f| f.key.width())
            .max()
            .unwrap_or(10)
            .min(30) as u16;

        let (start, end) = self.field_vp.visible_range();
        let rows: Vec<Row> = fields[start..end]
            .iter()
            .enumerate()
            .map(|(rel, f)| {
                let abs = start + rel;
                let is_sel = abs == self.field_vp.selected && focused;
                let is_editing = self.editing_field.is_some() && self.editing_field_index == abs;

                let key_style = if is_sel {
                    Style::default()
                        .fg(theme.accent_config)
                        .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                } else {
                    Style::default().fg(theme.inactive)
                };

                if is_editing {
                    if let Some(ref input) = self.editing_field {
                        let accent = Style::default()
                            .fg(theme.accent_config)
                            .add_modifier(Modifier::BOLD);
                        let val_cell = if input.mode == InputMode::Active {
                            let (before, after) = input.split_at_cursor();
                            let cursor_ch = after.chars().next().unwrap_or(' ').to_string();
                            let after_cursor: String = after.chars().skip(1).collect();
                            Cell::from(Line::from(vec![
                                Span::styled(before, accent),
                                Span::styled(
                                    cursor_ch,
                                    Style::default()
                                        .fg(Color::Black)
                                        .bg(Color::Yellow)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Span::styled(after_cursor, accent),
                            ]))
                        } else {
                            Cell::from(input.value.clone()).style(accent)
                        };
                        return Row::new(vec![
                            Cell::from(f.key.as_str()).style(key_style),
                            Cell::from(" = ").style(Style::default().fg(theme.warning)),
                            val_cell,
                        ]);
                    }
                }

                let val_style = if is_sel {
                    Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
                } else if !f.editable {
                    Style::default().fg(theme.inactive)
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(f.key.as_str()).style(key_style),
                    Cell::from(" = ").style(Style::default().fg(theme.inactive)),
                    Cell::from(f.display_value.as_str()).style(val_style),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(key_w),
                Constraint::Length(3),
                Constraint::Min(0),
            ],
        );
        frame.render_widget(table, inner);
    }

    // --- Direct popup handlers (Step 7-9)

    fn commit_direct_popup_field(
        &mut self,
        item: SidebarItem,
        field_index: usize,
        display_value: &str,
        config: &mut AppConfig,
    ) {
        // `item` identifies the sidebar entry being edited. Since is_any_popup_open()
        // blocks sidebar navigation while this popup is open, self.sidebar_vp.selected
        // always points to the same entry as `item`. commit_inline_edit uses the
        // viewport selection, so both paths are consistent.
        let _ = item; // documented invariant — not needed at runtime
                      // Snapshot BEFORE mutation; the popup will be closed by the caller
                      // right after this returns, and save_config's reload would otherwise
                      // wipe the field_vp cursor.
        self.mark_dirty();
        self.editing_field_index = field_index;
        self.commit_inline_edit(display_value, config);
    }

    fn handle_direct_vec_editor_key(&mut self, key: KeyEvent, config: &mut AppConfig) -> bool {
        let input_active = self.direct_vec_editor.as_ref().unwrap().input_active;
        if input_active {
            if key.code == KeyCode::Esc {
                let ve = self.direct_vec_editor.as_mut().unwrap();
                ve.input = InputField::new("");
                ve.input_active = false;
                return true;
            }
            let ve = self.direct_vec_editor.as_mut().unwrap();
            ve.input.handle_key(key);
            if ve.input.mode == InputMode::Normal {
                if !ve.input.value.is_empty() {
                    let val = std::mem::take(&mut ve.input.value);
                    ve.items.push(val);
                    ve.vp.set_dims(ve.items.len(), 0);
                }
                ve.input_active = false;
            }
            return true;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.direct_vec_editor.as_mut().unwrap().vp.move_up();
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.direct_vec_editor.as_mut().unwrap().vp.move_down();
                true
            }
            KeyCode::PageUp => {
                self.direct_vec_editor.as_mut().unwrap().vp.page_up();
                true
            }
            KeyCode::PageDown => {
                self.direct_vec_editor.as_mut().unwrap().vp.page_down();
                true
            }
            KeyCode::Home => {
                self.direct_vec_editor.as_mut().unwrap().vp.home();
                true
            }
            KeyCode::End => {
                self.direct_vec_editor.as_mut().unwrap().vp.end();
                true
            }
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
                    ve.vp.set_dims(ve.items.len(), 0);
                    if ve.vp.selected >= ve.items.len() && ve.vp.selected > 0 {
                        ve.vp.move_up();
                    }
                }
                true
            }
            KeyCode::Char('s') => {
                let (sidebar_item, field_index, display) = {
                    let ve = self.direct_vec_editor.as_ref().unwrap();
                    (
                        ve.sidebar_item.clone(),
                        ve.field_index,
                        if ve.items.is_empty() {
                            "(none)".to_string()
                        } else {
                            format!("[{}]", ve.items.join(", "))
                        },
                    )
                };
                self.commit_direct_popup_field(sidebar_item, field_index, &display, config);
                self.direct_vec_editor = None;
                true
            }
            KeyCode::Esc => {
                self.direct_vec_editor = None;
                true
            }
            _ => true,
        }
    }

    fn handle_direct_group_picker_key(&mut self, key: KeyEvent, config: &mut AppConfig) -> bool {
        let add_input_active = self.direct_group_picker.as_ref().unwrap().add_input_active;
        if add_input_active {
            if key.code == KeyCode::Esc {
                let gp = self.direct_group_picker.as_mut().unwrap();
                gp.add_input = InputField::new("");
                gp.add_input_active = false;
                return true;
            }
            let gp = self.direct_group_picker.as_mut().unwrap();
            gp.add_input.handle_key(key);
            if gp.add_input.mode == InputMode::Normal {
                apply_add_input_to_picker(
                    &gp.add_input.value,
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
                self.direct_group_picker.as_mut().unwrap().vp.move_up();
                true
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.direct_group_picker.as_mut().unwrap().vp.move_down();
                true
            }
            KeyCode::PageUp => {
                self.direct_group_picker.as_mut().unwrap().vp.page_up();
                true
            }
            KeyCode::PageDown => {
                self.direct_group_picker.as_mut().unwrap().vp.page_down();
                true
            }
            KeyCode::Home => {
                self.direct_group_picker.as_mut().unwrap().vp.home();
                true
            }
            KeyCode::End => {
                self.direct_group_picker.as_mut().unwrap().vp.end();
                true
            }
            KeyCode::Char(' ') => {
                let gp = self.direct_group_picker.as_mut().unwrap();
                let idx = gp.vp.selected;
                if idx < gp.checked.len() {
                    gp.checked[idx] = !gp.checked[idx];
                }
                true
            }
            KeyCode::Char('a') => {
                let gp = self.direct_group_picker.as_mut().unwrap();
                if gp.allow_add {
                    gp.add_input = InputField::new("");
                    gp.add_input.activate();
                    gp.add_input_active = true;
                }
                true
            }
            KeyCode::Enter | KeyCode::Char('s') => {
                let (sidebar_item, field_index, display) = {
                    let gp = self.direct_group_picker.as_ref().unwrap();
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
                    (gp.sidebar_item.clone(), gp.field_index, display)
                };
                self.commit_direct_popup_field(sidebar_item, field_index, &display, config);
                self.direct_group_picker = None;
                true
            }
            KeyCode::Esc => {
                self.direct_group_picker = None;
                true
            }
            _ => true,
        }
    }

    // --- Rendering helpers for direct popups (Step 12)

    fn render_direct_vec_editor(
        &self,
        area: Rect,
        frame: &mut Frame,
        theme: &Theme,
        dve: &DirectVecEditorState,
    ) {
        use super::super::components::popup::centered_rect;
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
        vp.set_dims(dve.items.len(), visible_h.saturating_sub(3));
        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(
                "  (a:add  d:del  s:save  Esc:cancel)",
                Style::default().fg(theme.warning),
            )),
            Line::from(""),
        ];
        for (rel, item) in vp.visible_slice(&dve.items).iter().enumerate() {
            let abs = vp.scroll_y + rel;
            let is_sel = abs == vp.selected;
            let style = if is_sel {
                Style::default()
                    .fg(theme.accent_config)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::default()
            };
            let prefix = if is_sel { "▶ " } else { "  " };
            lines.push(Line::from(Span::styled(format!("{prefix}{item}"), style)));
        }
        if dve.items.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (empty)",
                Style::default().fg(theme.inactive),
            )));
        }
        if dve.input_active {
            lines.push(Line::from(""));
            let accent = Style::default()
                .fg(theme.accent_config)
                .add_modifier(Modifier::BOLD);
            lines.push(input_cursor_line(
                &dve.input,
                Span::styled("  New: ", accent),
                accent,
            ));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_direct_group_picker(
        &self,
        area: Rect,
        frame: &mut Frame,
        theme: &Theme,
        dgp: &DirectGroupPickerState,
    ) {
        use super::super::components::popup::centered_rect;
        let popup_area = centered_rect(60, 70, area);
        frame.render_widget(Clear, popup_area);
        let title = format!(" Pick: {} ", dgp.field_key);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.accent_config))
            .title(title.as_str());
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let visible_h = inner.height as usize;
        let mut vp = dgp.vp.clone();
        let extra = if dgp.add_input_active { 4 } else { 2 };
        vp.set_dims(dgp.available.len(), visible_h.saturating_sub(extra + 2));
        let hints = if dgp.allow_add {
            "  (Space:toggle  a:add  Enter/s:apply  Esc:cancel)"
        } else {
            "  (Space:toggle  Enter/s:apply  Esc:cancel)"
        };
        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(hints, Style::default().fg(theme.warning))),
            Line::from(""),
        ];
        if dgp.available.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no options)",
                Style::default().fg(theme.inactive),
            )));
        } else {
            for (rel, group) in vp.visible_slice(&dgp.available).iter().enumerate() {
                let abs = vp.scroll_y + rel;
                let is_sel = abs == vp.selected;
                let checked = dgp.checked.get(abs).copied().unwrap_or(false);
                let mark = if checked { "◉" } else { "○" };
                let style = if is_sel {
                    Style::default()
                        .fg(theme.accent_config)
                        .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                } else {
                    Style::default()
                };
                let label = match dgp.descriptions.get(abs) {
                    Some(d) if !d.is_empty() => format!("  {mark} {group}  — {d}"),
                    _ => format!("  {mark} {group}"),
                };
                lines.push(Line::from(Span::styled(label, style)));
            }
        }
        if dgp.add_input_active {
            lines.push(Line::from(""));
            let accent = Style::default()
                .fg(theme.accent_config)
                .add_modifier(Modifier::BOLD);
            lines.push(input_cursor_line(
                &dgp.add_input,
                Span::styled("  New group: ", accent),
                accent,
            ));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

// ── Sidebar construction ─────────────────────────────────────────────────────

fn build_sidebar_items(config: &AppConfig, collapsed: &CollapsedSections) -> Vec<SidebarItem> {
    let mut items = vec![SidebarItem::SectionSettings, SidebarItem::SectionHosts];
    if !collapsed.hosts {
        for i in 0..config.host.len() {
            items.push(SidebarItem::Host(i));
        }
    }
    items.push(SidebarItem::SectionChecks);
    if !collapsed.checks {
        for i in 0..config.check.len() {
            items.push(SidebarItem::Check(i));
        }
    }
    items.push(SidebarItem::SectionSyncs);
    if !collapsed.syncs {
        for i in 0..config.sync.len() {
            items.push(SidebarItem::Sync(i));
        }
    }
    items
}

/// Disclosure triangle for collapsible section headers: `▼ ` expanded, `▶ `
/// collapsed. Returns `None` for non-collapsible rows (Settings + children),
/// which fall back to the normal selection cursor.
fn collapse_glyph(item: &SidebarItem, collapsed: &CollapsedSections) -> Option<&'static str> {
    let c = match item {
        SidebarItem::SectionHosts => collapsed.hosts,
        SidebarItem::SectionChecks => collapsed.checks,
        SidebarItem::SectionSyncs => collapsed.syncs,
        _ => return None,
    };
    Some(if c { "▶ " } else { "▼ " })
}

fn sidebar_item_display(item: &SidebarItem, config: &AppConfig) -> (&'static str, String, bool) {
    match item {
        SidebarItem::SectionSettings => ("", "Settings".to_string(), true),
        SidebarItem::SectionHosts => ("", format!("Hosts ({})", config.host.len()), true),
        SidebarItem::Host(i) => {
            let name = config.host.get(*i).map(|h| h.name.as_str()).unwrap_or("?");
            ("  ", name.to_string(), false)
        }
        SidebarItem::SectionChecks => ("", format!("Checks ({})", config.check.len()), true),
        SidebarItem::Check(i) => ("  ", entry_label_check(config, *i), false),
        SidebarItem::SectionSyncs => ("", format!("Syncs ({})", config.sync.len()), true),
        SidebarItem::Sync(i) => ("  ", entry_label_sync(config, *i), false),
    }
}

// ── Label helpers ────────────────────────────────────────────────────────────

pub fn entry_label_check(config: &AppConfig, i: usize) -> String {
    config
        .check
        .get(i)
        .map(|c| {
            if c.groups.is_empty() {
                format!("Check #{}", i + 1)
            } else {
                format!("Check #{} [{}]", i + 1, c.groups.join(","))
            }
        })
        .unwrap_or_else(|| format!("Check #{}", i + 1))
}

pub fn entry_label_sync(config: &AppConfig, i: usize) -> String {
    config
        .sync
        .get(i)
        .map(|s| {
            let path_hint = s.paths.first().map(|p| trunc(p, 10)).unwrap_or_default();
            if path_hint.is_empty() {
                format!("Sync #{}", i + 1)
            } else {
                format!("Sync #{}: {}", i + 1, path_hint)
            }
        })
        .unwrap_or_else(|| format!("Sync #{}", i + 1))
}

// ── Utilities ────────────────────────────────────────────────────────────────

fn apply_add_input_to_picker(
    value: &str,
    available: &mut Vec<String>,
    checked: &mut Vec<bool>,
    vp: &mut Viewport,
) {
    let new_group = value.trim().to_string();
    if !new_group.is_empty() && !available.contains(&new_group) {
        let pos = available.partition_point(|g| g.as_str() < new_group.as_str());
        available.insert(pos, new_group);
        checked.insert(pos, true);
        // Update viewport dims first (clamps selection if needed), then set the
        // selected index to the insertion position. Viewport::set_dims may
        // adjust `selected` when item_count==0 or selected >= item_count, so
        // explicitly assigning `vp.selected = pos` afterwards guarantees the
        // desired selection.
        vp.set_dims(available.len(), 0);
        vp.selected = pos;
    }
}

fn tribool_cycle_fwd(s: &str) -> &'static str {
    match s {
        "inherit" => "yes",
        "yes" => "no",
        _ => "inherit",
    }
}

/// Backward cycle, retained for symmetry with the forward helpers; no key
/// currently triggers it since Left/Right were freed for navigation.
#[allow(dead_code)]
fn tribool_cycle_back(s: &str) -> &'static str {
    match s {
        "no" => "yes",
        "yes" => "inherit",
        _ => "no",
    }
}

fn enum_cycle(variants: &[&str], current: &str, forward: bool) -> String {
    if variants.is_empty() {
        return current.to_string();
    }
    let pos = variants.iter().position(|&v| v == current).unwrap_or(0);
    let next = if forward {
        (pos + 1) % variants.len()
    } else {
        (pos + variants.len() - 1) % variants.len()
    };
    variants[next].to_string()
}

const SHELL_VARIANTS: &[&str] = &["sh", "powershell", "cmd"];

fn shell_cycle_fwd(s: &str) -> String {
    if !SHELL_VARIANTS.contains(&s) {
        tracing::warn!(shell = s, "unknown shell value, defaulting to sh");
    }
    enum_cycle(SHELL_VARIANTS, s, true)
}

#[allow(dead_code)]
fn shell_cycle_back(s: &str) -> String {
    if !SHELL_VARIANTS.contains(&s) {
        tracing::warn!(shell = s, "unknown shell value, defaulting to sh");
    }
    enum_cycle(SHELL_VARIANTS, s, false)
}

/// Forward-cycle a rotating/toggle option field's value, or `None` for kinds
/// that aren't cycleable options (text, vec, path, …).
///
/// Single shared path behind both Space and Enter so every option kind (Bool,
/// TriBool, ShellEnum, Enum) advances identically; Left/Right stay free for
/// navigation.
fn cycle_option_value(kind: &FieldKind, current: &str) -> Option<String> {
    match kind {
        FieldKind::Bool => Some(if current == "true" { "false" } else { "true" }.to_string()),
        FieldKind::TriBool => Some(tribool_cycle_fwd(current).to_string()),
        FieldKind::ShellEnum => Some(shell_cycle_fwd(current)),
        FieldKind::Enum { variants } => Some(enum_cycle(variants, current, true)),
        _ => None,
    }
}

pub(crate) fn trunc(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let mut w = 0usize;
    let mut out = String::new();
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if w + cw > max.saturating_sub(1) {
            break;
        }
        out.push(ch);
        w += cw;
    }
    out.push('…');
    out
}

fn strip_unit(s: &str) -> String {
    s.trim_end_matches('s')
        .trim_end_matches('d')
        .trim_end_matches('%')
        .to_string()
}

// Collect known groups across config (Step 3)
fn collect_known_groups(config: &AppConfig, current: &[String]) -> (Vec<String>, Vec<bool>) {
    let mut known: std::collections::BTreeSet<String> = config
        .host
        .iter()
        .flat_map(|h| h.groups.iter().cloned())
        .chain(config.check.iter().flat_map(|c| c.groups.iter().cloned()))
        .chain(config.sync.iter().flat_map(|s| s.groups.iter().cloned()))
        .collect();
    for item in current {
        known.insert(item.clone());
    }
    let available: Vec<String> = known.into_iter().collect();
    let checked: Vec<bool> = available.iter().map(|g| current.contains(g)).collect();
    (available, checked)
}

// input cursor helper for rendering (Step 10)
fn input_cursor_line<'a>(input: &'a InputField, prefix: Span<'a>, style: Style) -> Line<'a> {
    let (before, after) = input.split_at_cursor();
    let cursor_ch = after.chars().next().unwrap_or(' ').to_string();
    let after_cursor: String = after.chars().skip(1).collect();
    Line::from(vec![
        prefix,
        Span::styled(before, style),
        Span::styled(
            cursor_ch,
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(after_cursor, style),
    ])
}

use super::super::components::popup::centered_rect;

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;
    use ratatui::{backend::TestBackend, Terminal};

    fn render_once(state: &mut ConfigTabState, config: &AppConfig) {
        // Create a 80x24 test terminal and render one frame.
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::default_palette();
        terminal
            .draw(|f| state.render(Rect::new(0, 0, 80, 24), f, &theme, config, None, false))
            .unwrap();
    }

    #[test]
    fn test_enum_cycle_forward() {
        assert_eq!(enum_cycle(&["a", "b", "c"], "a", true), "b");
        assert_eq!(enum_cycle(&["a", "b", "c"], "c", true), "a");
    }

    #[test]
    fn test_enum_cycle_backward() {
        assert_eq!(enum_cycle(&["a", "b", "c"], "c", false), "b");
        assert_eq!(enum_cycle(&["a", "b", "c"], "a", false), "c");
    }

    #[test]
    fn test_enum_cycle_unknown_defaults_to_first() {
        assert_eq!(enum_cycle(&["a", "b"], "z", true), "b");
    }

    #[test]
    fn test_enum_cycle_unknown_backward_defaults_to_last() {
        assert_eq!(enum_cycle(&["a", "b", "c"], "z", false), "c");
    }

    #[test]
    fn test_shell_cycle_fwd() {
        assert_eq!(shell_cycle_fwd("sh"), "powershell");
        assert_eq!(shell_cycle_fwd("powershell"), "cmd");
        assert_eq!(shell_cycle_fwd("cmd"), "sh");
    }

    #[test]
    fn test_shell_cycle_back() {
        assert_eq!(shell_cycle_back("sh"), "cmd");
        assert_eq!(shell_cycle_back("cmd"), "powershell");
        assert_eq!(shell_cycle_back("powershell"), "sh");
    }

    #[test]
    fn test_cycle_option_value_advances_each_option_kind() {
        assert_eq!(
            cycle_option_value(&FieldKind::Bool, "true").as_deref(),
            Some("false")
        );
        assert_eq!(
            cycle_option_value(&FieldKind::Bool, "false").as_deref(),
            Some("true")
        );
        assert_eq!(
            cycle_option_value(&FieldKind::TriBool, "inherit").as_deref(),
            Some("yes")
        );
        assert_eq!(
            cycle_option_value(&FieldKind::ShellEnum, "sh").as_deref(),
            Some("powershell")
        );
        assert_eq!(
            cycle_option_value(
                &FieldKind::Enum {
                    variants: vec!["newest", "skip"]
                },
                "newest"
            )
            .as_deref(),
            Some("skip")
        );
    }

    #[test]
    fn test_cycle_option_value_none_for_non_option_kinds() {
        assert_eq!(cycle_option_value(&FieldKind::String, "x"), None);
        assert_eq!(cycle_option_value(&FieldKind::U64, "5"), None);
        assert_eq!(cycle_option_value(&FieldKind::VecString, "[a]"), None);
        assert_eq!(
            cycle_option_value(&FieldKind::CheckEnabled, "[online]"),
            None
        );
    }

    #[test]
    fn toggle_section_collapse_hides_children_and_keeps_cursor() {
        let mut config = AppConfig::default();
        for n in ["h1", "h2", "h3"] {
            config.host.push(HostEntry {
                name: n.to_string(),
                ssh_host: "1.1.1.1".to_string(),
                shell: ShellType::Sh,
                groups: vec![],
                proxy_jump: None,
            });
        }
        let mut state = ConfigTabState::new(&config, None);
        // Focus the Hosts section header.
        let hosts_idx = state
            .items
            .iter()
            .position(|i| matches!(i, SidebarItem::SectionHosts))
            .unwrap();
        state.sidebar_vp.selected = hosts_idx;
        let expanded_len = state.items.len();

        // Collapse: the 3 host children disappear, cursor stays on the header.
        assert!(state.toggle_section_collapse(&config));
        assert!(state.collapsed.hosts);
        assert_eq!(state.items.len(), expanded_len - 3);
        assert!(!state
            .items
            .iter()
            .any(|i| matches!(i, SidebarItem::Host(_))));
        assert!(matches!(
            state.items.get(state.sidebar_vp.selected),
            Some(SidebarItem::SectionHosts)
        ));

        // Expand again: children return.
        assert!(state.toggle_section_collapse(&config));
        assert!(!state.collapsed.hosts);
        assert_eq!(state.items.len(), expanded_len);
    }

    #[test]
    fn toggle_section_collapse_noop_on_non_section() {
        let config = AppConfig::default();
        let mut state = ConfigTabState::new(&config, None);
        // Settings header is not collapsible.
        let settings_idx = state
            .items
            .iter()
            .position(|i| matches!(i, SidebarItem::SectionSettings))
            .unwrap();
        state.sidebar_vp.selected = settings_idx;
        assert!(!state.toggle_section_collapse(&config));
    }

    #[test]
    fn render_does_not_panic_empty_sync_paths() {
        let mut config = AppConfig::default();
        config.sync.push(SyncEntry {
            name: None,
            id: generate_entry_id("sync"),
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
        let sid = state
            .items
            .iter()
            .position(|i| matches!(i, SidebarItem::Sync(_)))
            .unwrap();
        state.sidebar_vp.selected = sid;
        state.start_edit_entry(&config);
        if let Some(ref mut form) = state.entry_form {
            let fi = form.fields.iter().position(|f| f.key == "paths").unwrap();
            form.vec_editor = Some(VecEditorState {
                field_index: fi,
                items: vec![],
                vp: Viewport::new(),
                input: InputField::new(""),
                input_active: false,
                closing: false,
            });
        } else {
            panic!("entry_form not set");
        }
        render_once(&mut state, &config);
    }

    #[test]
    fn render_does_not_panic_empty_skipped_hosts_direct_vec() {
        let config = AppConfig::default();
        let mut state = ConfigTabState::new(&config, None);
        let sid = state
            .items
            .iter()
            .position(|i| matches!(i, SidebarItem::SectionSettings))
            .unwrap();
        state.sidebar_vp.selected = sid;
        state.direct_vec_editor = Some(DirectVecEditorState {
            field_index: 0,
            sidebar_item: SidebarItem::SectionSettings,
            field_key: "skipped_hosts".to_string(),
            items: vec![],
            vp: Viewport::new(),
            input: InputField::new(""),
            input_active: false,
        });
        render_once(&mut state, &config);
    }

    #[test]
    fn render_does_not_panic_empty_check_enabled_direct_vec() {
        let mut config = AppConfig::default();
        config.check.push(CheckEntry {
            name: None,
            id: generate_entry_id("check"),
            enabled: vec![],
            path: vec![],
            groups: vec![],
            enable_hosts: true,
            enable_all: true,
        });
        let mut state = ConfigTabState::new(&config, None);
        let sid = state
            .items
            .iter()
            .position(|i| matches!(i, SidebarItem::Check(_)))
            .unwrap();
        state.sidebar_vp.selected = sid;
        state.direct_vec_editor = Some(DirectVecEditorState {
            field_index: 0,
            sidebar_item: state.items[sid].clone(),
            field_key: "enabled".to_string(),
            items: vec![],
            vp: Viewport::new(),
            input: InputField::new(""),
            input_active: false,
        });
        render_once(&mut state, &config);
    }

    #[test]
    fn snapshot_round_trip_no_form_no_popup() {
        let mut config = AppConfig::default();
        config.host.push(HostEntry {
            name: "h1".to_string(),
            ssh_host: "1.1.1.1".to_string(),
            shell: ShellType::Sh,
            groups: vec![],
            proxy_jump: None,
        });
        config.host.push(HostEntry {
            name: "h2".to_string(),
            ssh_host: "2.2.2.2".to_string(),
            shell: ShellType::Sh,
            groups: vec![],
            proxy_jump: None,
        });
        let mut state = ConfigTabState::new(&config, None);
        state.sidebar_vp.move_down();
        state.sidebar_vp.move_down();
        let captured_sidebar = state.sidebar_vp.selected;
        let snap = state.capture_selection();
        state.sidebar_vp = Viewport::new();
        state.sidebar_vp.set_dims(state.items.len(), 0);
        state.restore_selection(snap, &config);
        assert_eq!(state.sidebar_vp.selected, captured_sidebar);
    }

    #[test]
    fn snapshot_clamps_when_entry_deleted() {
        let mut config = AppConfig::default();
        for i in 0..3 {
            config.host.push(HostEntry {
                name: format!("h{i}"),
                ssh_host: format!("{i}.{i}.{i}.{i}"),
                shell: ShellType::Sh,
                groups: vec![],
                proxy_jump: None,
            });
        }
        let mut state = ConfigTabState::new(&config, None);
        let last = state.items.len() - 1;
        for _ in 0..last {
            state.sidebar_vp.move_down();
        }
        let snap = state.capture_selection();
        config.host.pop();
        state.items = build_sidebar_items(&config, &state.collapsed);
        state.sidebar_vp = Viewport::new();
        state.sidebar_vp.set_dims(state.items.len(), 0);
        state.restore_selection(snap, &config);
        let expected = state.items.len().saturating_sub(1);
        assert_eq!(state.sidebar_vp.selected, expected);
    }

    #[test]
    fn snapshot_clamps_on_empty_list() {
        // Spec §7.4 empty-list edge case: post-reload length 0 → cursor at 0, no panic.
        let mut config = AppConfig::default();
        config.host.push(HostEntry {
            name: "h1".to_string(),
            ssh_host: "1.1.1.1".to_string(),
            shell: ShellType::Sh,
            groups: vec![],
            proxy_jump: None,
        });
        let mut state = ConfigTabState::new(&config, None);
        // Move cursor off zero.
        state.sidebar_vp.move_down();
        let snap = state.capture_selection();
        // Simulate a reload that ends up with zero items.
        state.items = vec![];
        state.sidebar_vp = Viewport::new();
        state.sidebar_vp.set_dims(0, 0);
        state.restore_selection(snap, &config);
        assert_eq!(state.sidebar_vp.selected, 0);
    }

    #[test]
    fn snapshot_restores_entry_form_field_cursor() {
        let mut config = AppConfig::default();
        config.host.push(crate::config::schema::HostEntry {
            name: "h1".to_string(),
            ssh_host: "1.1.1.1".to_string(),
            shell: crate::config::schema::ShellType::Sh,
            groups: vec![],
            proxy_jump: None,
        });
        let mut state = ConfigTabState::new(&config, None);
        let form = EntryFormState::new_host(&config.host[0]);
        state.entry_form = Some(form);
        // Move the field cursor inside the form to a non-zero position.
        if let Some(form) = state.entry_form.as_mut() {
            assert!(
                form.fields.len() >= 2,
                "host form should have multiple fields"
            );
            form.field_vp.set_dims(form.fields.len(), 0);
            form.field_vp.move_down();
            form.field_vp.move_down();
        }
        let captured_field = state
            .entry_form
            .as_ref()
            .map(|f| f.field_vp.selected)
            .unwrap();
        let snap = state.capture_selection();
        // Simulate reload that resets the form's field cursor to 0.
        if let Some(form) = state.entry_form.as_mut() {
            form.field_vp = Viewport::new();
            form.field_vp.set_dims(form.fields.len(), 0);
        }
        state.restore_selection(snap, &config);
        let restored_field = state
            .entry_form
            .as_ref()
            .map(|f| f.field_vp.selected)
            .unwrap();
        assert_eq!(restored_field, captured_field);
    }

    #[test]
    fn snapshot_restores_in_form_vec_editor_cursor() {
        let mut config = AppConfig::default();
        config.sync.push(crate::config::schema::SyncEntry {
            name: Some("test".to_string()),
            id: "sync-test".to_string(),
            paths: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            groups: vec![],
            enable_hosts: true,
            enable_all: true,
            recursive: false,
            mode: None,
            propagate_deletes: None,
            source: None,
        });
        let mut state = ConfigTabState::new(&config, None);
        let form = EntryFormState::new_sync(&config.sync[0]);
        state.entry_form = Some(form);
        let paths_field_idx = state
            .entry_form
            .as_ref()
            .unwrap()
            .fields
            .iter()
            .position(|f| f.key == "paths")
            .expect("paths field must exist");
        if let Some(form) = state.entry_form.as_mut() {
            form.field_vp.selected = paths_field_idx;
            let mut ve = VecEditorState {
                field_index: paths_field_idx,
                items: vec!["a".to_string(), "b".to_string(), "c".to_string()],
                vp: Viewport::new(),
                input_active: false,
                input: InputField::new(""),
                closing: false,
            };
            ve.vp.set_dims(3, 0);
            ve.vp.move_down();
            ve.vp.move_down();
            form.vec_editor = Some(ve);
        }
        let captured_ve = state
            .entry_form
            .as_ref()
            .and_then(|f| f.vec_editor.as_ref())
            .map(|ve| ve.vp.selected)
            .unwrap();
        assert_eq!(captured_ve, 2);
        let snap = state.capture_selection();
        // Reset the vec editor's cursor to 0.
        if let Some(form) = state.entry_form.as_mut() {
            if let Some(ve) = form.vec_editor.as_mut() {
                ve.vp = Viewport::new();
                ve.vp.set_dims(ve.items.len(), 0);
            }
        }
        state.restore_selection(snap, &config);
        let restored_ve = state
            .entry_form
            .as_ref()
            .and_then(|f| f.vec_editor.as_ref())
            .map(|ve| ve.vp.selected)
            .unwrap();
        assert_eq!(restored_ve, captured_ve);
    }

    #[test]
    fn snapshot_vec_editor_field_index_guard_prevents_wrong_apply() {
        let mut config = AppConfig::default();
        config.sync.push(crate::config::schema::SyncEntry {
            name: Some("test".to_string()),
            id: "sync-test".to_string(),
            paths: vec!["a".to_string(), "b".to_string()],
            groups: vec!["g1".to_string(), "g2".to_string()],
            enable_hosts: true,
            enable_all: true,
            recursive: false,
            mode: None,
            propagate_deletes: None,
            source: None,
        });
        let mut state = ConfigTabState::new(&config, None);
        let form = EntryFormState::new_sync(&config.sync[0]);
        state.entry_form = Some(form);
        let paths_idx = state
            .entry_form
            .as_ref()
            .unwrap()
            .fields
            .iter()
            .position(|f| f.key == "paths")
            .unwrap();
        let groups_idx = state
            .entry_form
            .as_ref()
            .unwrap()
            .fields
            .iter()
            .position(|f| f.key == "groups")
            .unwrap();
        assert_ne!(paths_idx, groups_idx);
        // Capture with vec_editor on `paths` at cursor 1.
        if let Some(form) = state.entry_form.as_mut() {
            let mut ve = VecEditorState {
                field_index: paths_idx,
                items: vec!["a".to_string(), "b".to_string()],
                vp: Viewport::new(),
                input_active: false,
                input: InputField::new(""),
                closing: false,
            };
            ve.vp.set_dims(2, 0);
            ve.vp.move_down();
            form.vec_editor = Some(ve);
        }
        let snap = state.capture_selection();
        // Between capture and restore, the form's vec_editor switches to `groups`.
        if let Some(form) = state.entry_form.as_mut() {
            form.vec_editor = Some(VecEditorState {
                field_index: groups_idx,
                items: vec!["g1".to_string(), "g2".to_string()],
                vp: Viewport::new(),
                input_active: false,
                input: InputField::new(""),
                closing: false,
            });
        }
        state.restore_selection(snap, &config);
        // Restored cursor must remain 0 — guard rejected the cross-field apply.
        let ve_sel = state
            .entry_form
            .as_ref()
            .and_then(|f| f.vec_editor.as_ref())
            .map(|ve| ve.vp.selected)
            .unwrap();
        assert_eq!(ve_sel, 0, "guard must prevent cross-field cursor apply");
    }

    #[test]
    fn snapshot_restores_direct_vec_editor_cursor() {
        let mut config = AppConfig::default();
        config.settings.skipped_hosts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let mut state = ConfigTabState::new(&config, None);
        let mut dve = DirectVecEditorState {
            field_index: 0,
            sidebar_item: SidebarItem::SectionSettings,
            field_key: "skipped_hosts".to_string(),
            items: vec!["a".to_string(), "b".to_string(), "c".to_string()],
            vp: Viewport::new(),
            input: InputField::new(""),
            input_active: false,
        };
        dve.vp.set_dims(3, 0);
        dve.vp.move_down();
        dve.vp.move_down();
        state.direct_vec_editor = Some(dve);
        let snap = state.capture_selection();
        if let Some(dve) = state.direct_vec_editor.as_mut() {
            dve.vp = Viewport::new();
            dve.vp.set_dims(dve.items.len(), 0);
        }
        state.restore_selection(snap, &config);
        let restored = state
            .direct_vec_editor
            .as_ref()
            .map(|d| d.vp.selected)
            .unwrap();
        assert_eq!(restored, 2);
    }

    // ── New tests covering the unified-schema refactor (2026-05-21) ──────────
    //
    // These reproduce the three production bugs that the refactor targets:
    //   1. Hosts/Checks/Syncs Vec edits via direct popup were silently dropped
    //   2. Cursor jumped to first row after entry-form commit (sidebar)
    //   3. Cursor jumped to first row after direct popup commit (field_vp)

    fn make_host_config() -> AppConfig {
        let mut c = AppConfig::default();
        c.host.push(HostEntry {
            name: "h1".into(),
            ssh_host: "1.2.3.4".into(),
            shell: ShellType::Sh,
            groups: vec!["old".into()],
            proxy_jump: None,
        });
        c
    }

    #[test]
    fn direct_popup_commit_persists_host_groups() {
        let mut config = make_host_config();
        let mut state = ConfigTabState::new(&config, None);
        let sid = state
            .items
            .iter()
            .position(|i| matches!(i, SidebarItem::Host(_)))
            .unwrap();
        state.sidebar_vp.selected = sid;
        // Simulate: open direct vec editor on "groups" field (index 3 in host_fields)
        state.direct_vec_editor = Some(DirectVecEditorState {
            field_index: 3,
            sidebar_item: state.items[sid].clone(),
            field_key: "groups".to_string(),
            items: vec!["a".into(), "b".into()],
            vp: Viewport::new(),
            input: InputField::new(""),
            input_active: false,
        });
        // Commit via the same code path the 's' key takes.
        state.commit_direct_popup_field(state.items[sid].clone(), 3, "[a, b]", &mut config);
        // The Vec field MUST be written through to config (the original bug).
        assert_eq!(config.host[0].groups, vec!["a", "b"]);
    }

    #[test]
    fn direct_popup_commit_preserves_field_vp_cursor() {
        let mut config = make_host_config();
        let mut state = ConfigTabState::new(&config, None);
        let sid = state
            .items
            .iter()
            .position(|i| matches!(i, SidebarItem::Host(_)))
            .unwrap();
        state.sidebar_vp.selected = sid;
        state
            .field_vp
            .set_dims(host_fields(&config.host[0]).len(), 0);
        // Move field cursor to "groups" (index 3) before opening popup.
        state.field_vp.selected = 3;

        state.direct_vec_editor = Some(DirectVecEditorState {
            field_index: 3,
            sidebar_item: state.items[sid].clone(),
            field_key: "groups".to_string(),
            items: vec!["x".into()],
            vp: Viewport::new(),
            input: InputField::new(""),
            input_active: false,
        });
        state.commit_direct_popup_field(state.items[sid].clone(), 3, "[x]", &mut config);
        state.direct_vec_editor = None;
        // Snapshot must have been captured by commit_direct_popup_field's mark_dirty.
        let snap = state.consume_pending_snapshot().expect("snapshot captured");
        assert_eq!(
            snap.field_vp_idx, 3,
            "snapshot must remember field_vp cursor"
        );

        // Simulate save_config's reload+restore (without touching disk).
        state.reload(&config, None);
        state.restore_selection(snap, &config);
        assert_eq!(
            state.field_vp.selected, 3,
            "field cursor must stay on the edited row, not jump to 0"
        );
    }

    #[test]
    fn entry_form_commit_preserves_sidebar_cursor() {
        let mut config = make_host_config();
        let mut state = ConfigTabState::new(&config, None);
        let sid = state
            .items
            .iter()
            .position(|i| matches!(i, SidebarItem::Host(_)))
            .unwrap();
        state.sidebar_vp.selected = sid;
        // Open entry form (edit-in-place via `e`).
        state.entry_form = Some(EntryFormState::new_host(&config.host[0]));
        if let Some(form) = state.entry_form.as_mut() {
            form.edit_index = Some(0);
        }
        // Commit (== pressing 's' in the form).
        state.commit_entry_form(&mut config);
        let snap = state.consume_pending_snapshot().expect("snapshot captured");
        assert_eq!(snap.sidebar_idx, sid);

        state.reload(&config, None);
        state.restore_selection(snap, &config);
        assert_eq!(
            state.sidebar_vp.selected, sid,
            "sidebar cursor must stay on the edited entry, not jump to 0"
        );
    }

    #[test]
    fn check_enabled_via_direct_popup_persists() {
        let mut config = AppConfig::default();
        config.check.push(CheckEntry {
            name: None,
            id: generate_entry_id("check"),
            enabled: vec![],
            path: vec![],
            groups: vec![],
            enable_hosts: true,
            enable_all: true,
        });
        let mut state = ConfigTabState::new(&config, None);
        let sid = state
            .items
            .iter()
            .position(|i| matches!(i, SidebarItem::Check(_)))
            .unwrap();
        state.sidebar_vp.selected = sid;
        // enabled is index 0 in check_fields.
        state.direct_group_picker = Some(DirectGroupPickerState {
            field_index: 0,
            sidebar_item: state.items[sid].clone(),
            field_key: "enabled".to_string(),
            available: vec!["online".into(), "cpu_load".into()],
            checked: vec![true, true],
            descriptions: vec![],
            allow_add: false,
            vp: Viewport::new(),
            add_input: InputField::new(""),
            add_input_active: false,
        });
        state.commit_direct_popup_field(
            state.items[sid].clone(),
            0,
            "[online, cpu_load]",
            &mut config,
        );
        assert_eq!(config.check[0].enabled, vec!["online", "cpu_load"]);
    }
}
