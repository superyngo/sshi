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
    generate_entry_id, AppConfig, CheckEntry, ConflictStrategy, HostEntry, Settings, ShellType,
    SyncEntry,
};

use super::super::components::input_field::{InputField, InputMode};
use super::super::components::viewport::Viewport;
use super::super::theme::Theme;

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

// ── Field type descriptors (Phase 7) ──────────────────────────────────────

pub const CHECK_ENABLED_OPTIONS: &[(&str, &str)] = &[
    ("online", "Check if host is online"),
    ("system_info", "System info (uname / systeminfo)"),
    ("cpu_arch", "CPU architecture"),
    ("memory", "Memory usage"),
    ("swap", "Swap usage"),
    ("disk", "Disk usage"),
    ("cpu_load", "CPU load"),
    ("network", "Network interface info"),
    ("battery", "Battery status"),
    ("ip_address", "IP address"),
];

#[derive(Debug, Clone)]
pub enum FieldKind {
    U64,
    Bool,
    String,
    OptionalString,
    Enum {
        variants: Vec<&'static str>,
    },
    VecString,
    #[allow(dead_code)]
    VecCheckPath,
    CheckEnabled, // fixed multi-select for check.enabled
    ShellEnum,
    TriBool, // Option<bool>: "inherit" | "yes" | "no"
}

#[derive(Debug, Clone)]
pub struct FieldDescriptor {
    pub key: String,
    pub display_value: String,
    pub kind: FieldKind,
    pub editable: bool,
}

impl FieldDescriptor {
    fn scalar(key: &str, value: String, kind: FieldKind) -> Self {
        Self {
            key: key.to_string(),
            display_value: value,
            kind,
            editable: true,
        }
    }

    fn readonly(key: &str, value: String) -> Self {
        Self {
            key: key.to_string(),
            display_value: value,
            kind: FieldKind::String,
            editable: false,
        }
    }

    fn vec_field(key: &str, display: String, kind: FieldKind) -> Self {
        Self {
            key: key.to_string(),
            display_value: display,
            kind,
            editable: true,
        }
    }
}

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
    pub vp: Viewport,
    pub add_input: InputField,
    pub add_input_active: bool,
}

impl EntryFormState {
    pub fn new_host(template: &HostEntry) -> Self {
        let fields = host_form_fields(template);
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
        let fields = check_form_fields(template);
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
        let fields = sync_form_fields(template);
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

fn host_form_fields(h: &HostEntry) -> Vec<FieldDescriptor> {
    vec![
        FieldDescriptor::scalar("name", h.name.clone(), FieldKind::String),
        FieldDescriptor::scalar("ssh_host", h.ssh_host.clone(), FieldKind::String),
        FieldDescriptor::scalar("shell", h.shell.to_string(), FieldKind::ShellEnum),
        FieldDescriptor::vec_field("groups", fmt_vec(&h.groups), FieldKind::VecString),
        FieldDescriptor::scalar(
            "proxy_jump",
            h.proxy_jump.clone().unwrap_or_default(),
            FieldKind::OptionalString,
        ),
    ]
}

fn check_form_fields(c: &CheckEntry) -> Vec<FieldDescriptor> {
    let mut fields = vec![
        FieldDescriptor::vec_field("enabled", fmt_vec(&c.enabled), FieldKind::CheckEnabled),
        FieldDescriptor::vec_field("groups", fmt_vec(&c.groups), FieldKind::VecString),
        FieldDescriptor::scalar("enable_hosts", c.enable_hosts.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("enable_all", c.enable_all.to_string(), FieldKind::Bool),
    ];
    for p in &c.path {
        fields.push(FieldDescriptor::scalar(
            &format!("path:{}:{}", p.label, p.path),
            format!("{} → {}", p.label, p.path),
            FieldKind::String,
        ));
    }
    fields
}

fn sync_form_fields(s: &SyncEntry) -> Vec<FieldDescriptor> {
    let mut fields = vec![
        FieldDescriptor::vec_field("paths", fmt_vec(&s.paths), FieldKind::VecString),
        FieldDescriptor::vec_field("groups", fmt_vec(&s.groups), FieldKind::VecString),
        FieldDescriptor::scalar("enable_hosts", s.enable_hosts.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("enable_all", s.enable_all.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("recursive", s.recursive.to_string(), FieldKind::Bool),
    ];
    if let Some(m) = &s.mode {
        fields.push(FieldDescriptor::scalar(
            "mode",
            m.clone(),
            FieldKind::String,
        ));
    } else {
        fields.push(FieldDescriptor::scalar(
            "mode",
            String::new(),
            FieldKind::OptionalString,
        ));
    }
    fields.push(FieldDescriptor::scalar(
        "propagate_deletes",
        tribool_from_opt(s.propagate_deletes).to_string(),
        FieldKind::TriBool,
    ));
    fields.push(FieldDescriptor::scalar(
        "source",
        s.source.clone().unwrap_or_default(),
        FieldKind::OptionalString,
    ));
    fields
}

fn fmt_vec(v: &[String]) -> String {
    if v.is_empty() {
        "(none)".to_string()
    } else {
        format!("[{}]", v.join(", "))
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
    OpenEditorDirty,
}

// ── Config tab state ──────────────────────────────────────────────────────

pub struct ConfigTabState {
    pub zone: ConfigZone,
    pub items: Vec<SidebarItem>,
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
    pub pending_field_restore: Option<usize>,
    // direct popups
    pub direct_vec_editor: Option<DirectVecEditorState>,
    pub direct_group_picker: Option<DirectGroupPickerState>,
}

impl ConfigTabState {
    pub fn new(config: &AppConfig, config_path: Option<&std::path::Path>) -> Self {
        let items = build_sidebar_items(config);
        let mut sidebar_vp = Viewport::new();
        sidebar_vp.set_dims(items.len(), 0);

        let config_mtime =
            config_path.and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());

        Self {
            zone: ConfigZone::Sidebar,
            items,
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
            pending_field_restore: None,
            direct_vec_editor: None,
            direct_group_picker: None,
        }
    }

    pub fn reload(&mut self, config: &AppConfig, config_path: Option<&std::path::Path>) {
        self.items = build_sidebar_items(config);
        let new_len = self.items.len();
        let old_sel = self.sidebar_vp.selected;
        self.sidebar_vp = Viewport::new();
        self.sidebar_vp.set_dims(new_len, 0);
        if old_sel < new_len {
            for _ in 0..old_sel {
                self.sidebar_vp.move_down();
            }
        }
        self.field_vp = Viewport::new();
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
                    let fields = settings_descriptors(&config.settings);
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
                    let fields = host_descriptors(&config.host[*i]);
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
                    let fields = check_descriptors(&config.check[*i]);
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
                    let fields = sync_descriptors(&config.sync[*i]);
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
                KeyCode::Enter => {
                    if let Some(
                        SidebarItem::Host(_) | SidebarItem::Check(_) | SidebarItem::Sync(_),
                    ) = self.items.get(self.sidebar_vp.selected).cloned()
                    {
                        self.zone = ConfigZone::FieldTable;
                    }
                    true
                }
                _ => false,
            },
            ConfigZone::FieldTable => match key.code {
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
                    let fields = self.current_descriptors(config);
                    if let Some(f) = fields.get(self.field_vp.selected) {
                        if matches!(f.kind, FieldKind::TriBool) {
                            let new_val = tribool_cycle_back(&f.display_value);
                            self.editing_field_index = self.field_vp.selected;
                            self.commit_inline_edit(new_val, config);
                            self.config_dirty = true;
                            self.pending_save = true;
                            return true;
                        }
                        if matches!(f.kind, FieldKind::ShellEnum) {
                            let new_val = shell_cycle_back(&f.display_value);
                            self.editing_field_index = self.field_vp.selected;
                            self.commit_inline_edit(&new_val, config);
                            self.config_dirty = true;
                            self.pending_save = true;
                            return true;
                        }
                        if let FieldKind::Enum { variants } = &f.kind {
                            let new_val = enum_cycle(variants, &f.display_value, false);
                            self.editing_field_index = self.field_vp.selected;
                            self.commit_inline_edit(&new_val, config);
                            self.config_dirty = true;
                            self.pending_save = true;
                            return true;
                        }
                    }
                    self.zone = ConfigZone::Sidebar;
                    true
                }
                KeyCode::Right => {
                    let fields = self.current_descriptors(config);
                    if let Some(f) = fields.get(self.field_vp.selected) {
                        if matches!(f.kind, FieldKind::TriBool) {
                            let new_val = tribool_cycle_fwd(&f.display_value);
                            self.editing_field_index = self.field_vp.selected;
                            self.commit_inline_edit(new_val, config);
                            self.config_dirty = true;
                            self.pending_save = true;
                        }
                        if matches!(f.kind, FieldKind::ShellEnum) {
                            let new_val = shell_cycle_fwd(&f.display_value);
                            self.editing_field_index = self.field_vp.selected;
                            self.commit_inline_edit(&new_val, config);
                            self.config_dirty = true;
                            self.pending_save = true;
                        }
                        if let FieldKind::Enum { variants } = &f.kind {
                            let new_val = enum_cycle(variants, &f.display_value, true);
                            self.editing_field_index = self.field_vp.selected;
                            self.commit_inline_edit(&new_val, config);
                            self.config_dirty = true;
                            self.pending_save = true;
                        }
                    }
                    true
                }
                KeyCode::Char(' ') => {
                    let fields = self.current_descriptors(config);
                    if let Some(f) = fields.get(self.field_vp.selected) {
                        if matches!(f.kind, FieldKind::Bool) {
                            let new_val = if f.display_value == "true" {
                                "false"
                            } else {
                                "true"
                            };
                            self.editing_field_index = self.field_vp.selected;
                            self.commit_inline_edit(new_val, config);
                            self.config_dirty = true;
                            self.pending_save = true;
                            return true;
                        }
                    }
                    false
                }
                KeyCode::Char('e') | KeyCode::Enter => {
                    let field_idx = self.field_vp.selected;
                    let fields = self.current_descriptors(config);
                    if let Some(f) = fields.get(field_idx) {
                        match &f.kind {
                            FieldKind::TriBool => {
                                let new_val = tribool_cycle_fwd(&f.display_value);
                                self.editing_field_index = field_idx;
                                self.commit_inline_edit(new_val, config);
                                self.config_dirty = true;
                                self.pending_save = true;
                                return true;
                            }
                            FieldKind::Bool => {
                                let new_val = if f.display_value == "true" {
                                    "false"
                                } else {
                                    "true"
                                };
                                self.editing_field_index = field_idx;
                                self.commit_inline_edit(new_val, config);
                                self.config_dirty = true;
                                self.pending_save = true;
                                return true;
                            }
                            FieldKind::ShellEnum => {
                                let new_val = shell_cycle_fwd(&f.display_value);
                                self.editing_field_index = field_idx;
                                self.commit_inline_edit(&new_val, config);
                                self.config_dirty = true;
                                self.pending_save = true;
                                return true;
                            }
                            FieldKind::Enum { variants } => {
                                let new_val =
                                    enum_cycle(variants.as_slice(), &f.display_value, true);
                                self.editing_field_index = field_idx;
                                self.commit_inline_edit(&new_val, config);
                                self.config_dirty = true;
                                self.pending_save = true;
                                return true;
                            }
                            FieldKind::VecString
                            | FieldKind::VecCheckPath
                            | FieldKind::CheckEnabled => {
                                // New behavior: open direct sub-popup instead of full entry form (Step 5)
                                let field_key = f.key.clone();
                                let current_val = f.display_value.clone();
                                let sidebar_item = self.items[self.sidebar_vp.selected].clone();
                                let field_index = self.field_vp.selected;
                                let use_group_picker = matches!(f.kind, FieldKind::CheckEnabled)
                                    || (matches!(f.kind, FieldKind::VecString)
                                        && field_key == "groups");
                                if use_group_picker {
                                    let current = parse_bracket_list(&current_val);
                                    let (available, checked) =
                                        collect_known_groups(config, &current);
                                    let mut vp = Viewport::new();
                                    vp.set_dims(available.len().max(1), 0);
                                    self.direct_group_picker = Some(DirectGroupPickerState {
                                        field_index,
                                        sidebar_item,
                                        field_key,
                                        available,
                                        checked,
                                        vp,
                                        add_input: InputField::new(""),
                                        add_input_active: false,
                                    });
                                } else {
                                    let items = parse_bracket_list(&current_val);
                                    let mut vp = Viewport::new();
                                    vp.set_dims(items.len().max(1), 0);
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
        match item {
            SidebarItem::SectionSettings => {
                apply_settings_field(config, self.editing_field_index, new_value);
            }
            SidebarItem::Host(i) => {
                if let Some(h) = config.host.get_mut(i) {
                    apply_host_field(h, self.editing_field_index, new_value);
                }
            }
            SidebarItem::Check(i) => {
                if let Some(c) = config.check.get_mut(i) {
                    apply_check_field(c, self.editing_field_index, new_value);
                }
            }
            SidebarItem::Sync(i) => {
                if let Some(s) = config.sync.get_mut(i) {
                    apply_sync_field(s, self.editing_field_index, new_value);
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
            if let Some(form) = self.entry_form.as_mut() {
                form.vec_editor = Some(ve);
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
            KeyCode::Left => {
                let idx = form.field_vp.selected;
                if idx < form.fields.len() && matches!(form.fields[idx].kind, FieldKind::TriBool) {
                    let new_val = tribool_cycle_back(&form.fields[idx].display_value.clone());
                    form.fields[idx].display_value = new_val.to_string();
                    form.dirty = true;
                    return true;
                }
                if idx < form.fields.len() && matches!(form.fields[idx].kind, FieldKind::ShellEnum)
                {
                    let new_val = shell_cycle_back(&form.fields[idx].display_value);
                    form.fields[idx].display_value = new_val.to_string();
                    form.dirty = true;
                    return true;
                }
                if idx < form.fields.len() {
                    if let FieldKind::Enum { variants } = &form.fields[idx].kind {
                        let new_val = enum_cycle(
                            variants.as_slice(),
                            &form.fields[idx].display_value.clone(),
                            false,
                        );
                        form.fields[idx].display_value = new_val;
                        form.dirty = true;
                        return true;
                    }
                }
                false
            }
            KeyCode::Right => {
                let idx = form.field_vp.selected;
                if idx < form.fields.len() && matches!(form.fields[idx].kind, FieldKind::TriBool) {
                    let new_val = tribool_cycle_fwd(&form.fields[idx].display_value.clone());
                    form.fields[idx].display_value = new_val.to_string();
                    form.dirty = true;
                    return true;
                }
                if idx < form.fields.len() && matches!(form.fields[idx].kind, FieldKind::ShellEnum)
                {
                    let new_val = shell_cycle_fwd(&form.fields[idx].display_value);
                    form.fields[idx].display_value = new_val.to_string();
                    form.dirty = true;
                    return true;
                }
                if idx < form.fields.len() {
                    if let FieldKind::Enum { variants } = &form.fields[idx].kind {
                        let new_val = enum_cycle(
                            variants.as_slice(),
                            &form.fields[idx].display_value.clone(),
                            true,
                        );
                        form.fields[idx].display_value = new_val;
                        form.dirty = true;
                        return true;
                    }
                }
                false
            }
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
            KeyCode::Char('e') | KeyCode::Enter => {
                let idx = form.field_vp.selected;
                if idx < form.fields.len() {
                    let field = &form.fields[idx];
                    if field.editable {
                        match &field.kind {
                            FieldKind::TriBool => {
                                let new_val = tribool_cycle_fwd(&form.fields[idx].display_value);
                                form.fields[idx].display_value = new_val.to_string();
                                form.dirty = true;
                            }
                            FieldKind::Bool => {
                                let toggled = if form.fields[idx].display_value == "true" {
                                    "false"
                                } else {
                                    "true"
                                };
                                form.fields[idx].display_value = toggled.to_string();
                                form.dirty = true;
                            }
                            FieldKind::ShellEnum => {
                                let new_val = shell_cycle_fwd(&form.fields[idx].display_value);
                                form.fields[idx].display_value = new_val.to_string();
                                form.dirty = true;
                            }
                            FieldKind::Enum { variants } => {
                                let new_val = enum_cycle(
                                    variants.as_slice(),
                                    &form.fields[idx].display_value.clone(),
                                    true,
                                );
                                form.fields[idx].display_value = new_val;
                                form.dirty = true;
                            }
                            FieldKind::CheckEnabled => {
                                let current = parse_bracket_list(&form.fields[idx].display_value);
                                let available: Vec<String> = CHECK_ENABLED_OPTIONS
                                    .iter()
                                    .map(|(k, _)| k.to_string())
                                    .collect();
                                let checked: Vec<bool> =
                                    available.iter().map(|k| current.contains(k)).collect();
                                let mut vp = Viewport::new();
                                vp.set_dims(available.len().max(1), 0);
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
                                    vp.set_dims(available.len().max(1), 0);
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
                                    };
                                    ve.vp.set_dims(ve.items.len().max(1), 0);
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
                self.pending_field_restore = self.commit_entry_form(config);
                self.entry_form = None;
                self.pending_save = true;
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
                    ve.vp.set_dims(ve.items.len().max(1), 0);
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
                    ve.vp.set_dims(ve.items.len().max(1), 0);
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
                form.vec_editor = None;
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
                    ConfirmAction::OpenEditorDirty => {
                        self.pending_open_editor = true;
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

    fn commit_entry_form(&mut self, config: &mut AppConfig) -> Option<usize> {
        let form = self.entry_form.take().unwrap();
        let saved_sel = form.field_vp.selected;
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
                    match f.key.as_str() {
                        "name" => h.name = f.display_value.clone(),
                        "ssh_host" => h.ssh_host = f.display_value.clone(),
                        "shell" => {
                            h.shell = match f.display_value.as_str() {
                                "powershell" => ShellType::PowerShell,
                                "cmd" => ShellType::Cmd,
                                _ => ShellType::Sh,
                            }
                        }
                        "groups" => h.groups = parse_bracket_list(&f.display_value),
                        "proxy_jump" => {
                            h.proxy_jump = if f.display_value.is_empty() {
                                None
                            } else {
                                Some(f.display_value.clone())
                            }
                        }
                        _ => {}
                    }
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
                    match f.key.as_str() {
                        "enabled" => c.enabled = parse_bracket_list(&f.display_value),
                        "groups" => c.groups = parse_bracket_list(&f.display_value),
                        "enable_hosts" => c.enable_hosts = f.display_value == "true",
                        "enable_all" => c.enable_all = f.display_value == "true",
                        _ => {}
                    }
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
                    match f.key.as_str() {
                        "paths" => s.paths = parse_bracket_list(&f.display_value),
                        "groups" => s.groups = parse_bracket_list(&f.display_value),
                        "enable_hosts" => s.enable_hosts = f.display_value == "true",
                        "enable_all" => s.enable_all = f.display_value == "true",
                        "recursive" => s.recursive = f.display_value == "true",
                        "mode" => {
                            s.mode = if f.display_value.is_empty() {
                                None
                            } else {
                                Some(f.display_value.clone())
                            }
                        }
                        "propagate_deletes" => {
                            s.propagate_deletes = tribool_to_opt(&f.display_value);
                        }
                        "source" => {
                            s.source = if f.display_value.is_empty() {
                                None
                            } else {
                                Some(f.display_value.clone())
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(idx) = form.edit_index {
                    config.sync[idx] = s;
                } else {
                    config.sync.push(s);
                }
            }
        }
        self.config_dirty = true;
        let items = build_sidebar_items(config);
        self.items = items;
        self.sidebar_vp = Viewport::new();
        self.sidebar_vp.set_dims(self.items.len(), 0);
        self.field_vp = Viewport::new();
        Some(saved_sel)
    }

    pub fn execute_delete(&mut self, config: &mut AppConfig, kind: EntryFormKind, index: usize) {
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
        self.config_dirty = true;
        let items = build_sidebar_items(config);
        self.items = items;
        self.sidebar_vp = Viewport::new();
        self.sidebar_vp.set_dims(self.items.len(), 0);
        self.field_vp = Viewport::new();
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
            gp_vp.set_dims(gp.available.len().max(1), gp_visible_h);
            let (gs, ge) = gp_vp.visible_range();

            if gp.available.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  (no known groups — define groups on hosts first)",
                    Style::default().fg(theme.inactive),
                )));
            } else {
                for (rel, group) in gp.available[gs..ge].iter().enumerate() {
                    let abs = gs + rel;
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
            ve_vp.set_dims(ve.items.len().max(1), ve_visible_h);
            let (vs, ve_end) = ve_vp.visible_range();

            for (rel, item) in ve.items[vs..ve_end].iter().enumerate() {
                let abs = vs + rel;
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
            "  [Enter/e] Edit  [Space] Toggle bool  [s] Save  [Esc] Cancel",
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
            Some(SidebarItem::SectionSettings) => settings_descriptors(&config.settings),
            Some(SidebarItem::SectionHosts) => {
                vec![FieldDescriptor::readonly(
                    "hosts",
                    format!("{} configured", config.host.len()),
                )]
            }
            Some(SidebarItem::Host(i)) => config
                .host
                .get(*i)
                .map(host_descriptors)
                .unwrap_or_default(),
            Some(SidebarItem::SectionChecks) => {
                vec![FieldDescriptor::readonly(
                    "checks",
                    format!("{} configured", config.check.len()),
                )]
            }
            Some(SidebarItem::Check(i)) => config
                .check
                .get(*i)
                .map(check_descriptors)
                .unwrap_or_default(),
            Some(SidebarItem::SectionSyncs) => {
                vec![FieldDescriptor::readonly(
                    "syncs",
                    format!("{} configured", config.sync.len()),
                )]
            }
            Some(SidebarItem::Sync(i)) => config
                .sync
                .get(*i)
                .map(sync_descriptors)
                .unwrap_or_default(),
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
                let glyph = if is_sel && focused {
                    "▶"
                } else if is_sel {
                    ">"
                } else {
                    " "
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
        self.editing_field_index = field_index;
        self.commit_inline_edit(display_value, config);
        self.config_dirty = true;
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
                    ve.vp.set_dims(ve.items.len().max(1), 0);
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
                gp.add_input = InputField::new("");
                gp.add_input.activate();
                gp.add_input_active = true;
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
        vp.set_dims(dve.items.len().max(1), visible_h.saturating_sub(3));
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
        vp.set_dims(
            dgp.available.len().max(1),
            visible_h.saturating_sub(extra + 2),
        );
        let (gs, ge) = vp.visible_range();

        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(
                "  (Space:toggle  a:add  Enter/s:apply  Esc:cancel)",
                Style::default().fg(theme.warning),
            )),
            Line::from(""),
        ];
        if dgp.available.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no known groups)",
                Style::default().fg(theme.inactive),
            )));
        } else {
            for (rel, group) in dgp.available[gs..ge].iter().enumerate() {
                let abs = gs + rel;
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
                lines.push(Line::from(Span::styled(format!("  {mark} {group}"), style)));
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

fn build_sidebar_items(config: &AppConfig) -> Vec<SidebarItem> {
    let mut items = vec![SidebarItem::SectionSettings, SidebarItem::SectionHosts];
    for i in 0..config.host.len() {
        items.push(SidebarItem::Host(i));
    }
    items.push(SidebarItem::SectionChecks);
    for i in 0..config.check.len() {
        items.push(SidebarItem::Check(i));
    }
    items.push(SidebarItem::SectionSyncs);
    for i in 0..config.sync.len() {
        items.push(SidebarItem::Sync(i));
    }
    items
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

// ── Field descriptor builders (Phase 7 typed version) ────────────────────────

fn settings_descriptors(s: &Settings) -> Vec<FieldDescriptor> {
    let mut f = vec![
        FieldDescriptor::scalar(
            "default_timeout",
            format!("{}s", s.default_timeout),
            FieldKind::U64,
        ),
        FieldDescriptor::scalar(
            "data_retention_days",
            format!("{}d", s.data_retention_days),
            FieldKind::U64,
        ),
        FieldDescriptor::scalar(
            "conflict_strategy",
            format!("{:?}", s.conflict_strategy).to_lowercase(),
            FieldKind::Enum {
                variants: vec!["newest", "skip"],
            },
        ),
        FieldDescriptor::scalar(
            "propagate_deletes",
            s.propagate_deletes.to_string(),
            FieldKind::Bool,
        ),
        FieldDescriptor::scalar(
            "max_concurrency",
            s.max_concurrency.to_string(),
            FieldKind::U64,
        ),
        FieldDescriptor::scalar(
            "max_per_host_concurrency",
            s.max_per_host_concurrency.to_string(),
            FieldKind::U64,
        ),
    ];
    if let Some(d) = &s.state_dir {
        f.push(FieldDescriptor::scalar(
            "state_dir",
            d.display().to_string(),
            FieldKind::OptionalString,
        ));
    }
    if let Some(fmt) = &s.default_output_format {
        f.push(FieldDescriptor::scalar(
            "default_output_format",
            fmt.clone(),
            FieldKind::OptionalString,
        ));
    }
    if !s.skipped_hosts.is_empty() {
        f.push(FieldDescriptor::vec_field(
            "skipped_hosts",
            format!("[{}]", s.skipped_hosts.join(", ")),
            FieldKind::VecString,
        ));
    }
    f
}

fn host_descriptors(h: &HostEntry) -> Vec<FieldDescriptor> {
    let mut f = vec![
        FieldDescriptor::scalar("name", h.name.clone(), FieldKind::String),
        FieldDescriptor::scalar("ssh_host", h.ssh_host.clone(), FieldKind::String),
        FieldDescriptor::scalar("shell", h.shell.to_string(), FieldKind::ShellEnum),
        FieldDescriptor::vec_field(
            "groups",
            if h.groups.is_empty() {
                "(none)".to_string()
            } else {
                format!("[{}]", h.groups.join(", "))
            },
            FieldKind::VecString,
        ),
    ];
    if let Some(pj) = &h.proxy_jump {
        f.push(FieldDescriptor::scalar(
            "proxy_jump",
            pj.clone(),
            FieldKind::OptionalString,
        ));
    }
    f
}

fn check_descriptors(c: &CheckEntry) -> Vec<FieldDescriptor> {
    let mut f = vec![
        FieldDescriptor::vec_field(
            "enabled",
            if c.enabled.is_empty() {
                "(none)".to_string()
            } else {
                format!("[{}]", c.enabled.join(", "))
            },
            FieldKind::VecString,
        ),
        FieldDescriptor::vec_field(
            "groups",
            if c.groups.is_empty() {
                "(unscoped)".to_string()
            } else {
                format!("[{}]", c.groups.join(", "))
            },
            FieldKind::VecString,
        ),
        FieldDescriptor::scalar("enable_hosts", c.enable_hosts.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("enable_all", c.enable_all.to_string(), FieldKind::Bool),
    ];
    for (i, p) in c.path.iter().enumerate() {
        f.push(FieldDescriptor::scalar(
            &format!("path[{i}]"),
            format!("{} → {}", p.label, p.path),
            FieldKind::String,
        ));
    }
    f
}

fn sync_descriptors(s: &SyncEntry) -> Vec<FieldDescriptor> {
    let mut f = vec![
        FieldDescriptor::vec_field(
            "paths",
            if s.paths.is_empty() {
                "(none)".to_string()
            } else {
                format!("[{}]", s.paths.join(", "))
            },
            FieldKind::VecString,
        ),
        FieldDescriptor::vec_field(
            "groups",
            if s.groups.is_empty() {
                "(unscoped)".to_string()
            } else {
                format!("[{}]", s.groups.join(", "))
            },
            FieldKind::VecString,
        ),
        FieldDescriptor::scalar("enable_hosts", s.enable_hosts.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("enable_all", s.enable_all.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("recursive", s.recursive.to_string(), FieldKind::Bool),
    ];
    if let Some(m) = &s.mode {
        f.push(FieldDescriptor::scalar(
            "mode",
            m.clone(),
            FieldKind::String,
        ));
    }
    f.push(FieldDescriptor::scalar(
        "propagate_deletes",
        tribool_from_opt(s.propagate_deletes).to_string(),
        FieldKind::TriBool,
    ));
    if let Some(src) = &s.source {
        f.push(FieldDescriptor::scalar(
            "source",
            src.clone(),
            FieldKind::OptionalString,
        ));
    }
    f
}

// ── Inline-edit field writers (mutate AppConfig in place) ────────────────────

fn apply_settings_field(config: &mut AppConfig, idx: usize, val: &str) {
    let fields = settings_descriptors(&config.settings);
    if idx >= fields.len() {
        return;
    }
    let key = &fields[idx].key;
    match key.as_str() {
        "default_timeout" => {
            if let Ok(v) = val.parse::<u64>() {
                config.settings.default_timeout = v;
            }
        }
        "data_retention_days" => {
            if let Ok(v) = val.parse::<u64>() {
                config.settings.data_retention_days = v;
            }
        }
        "conflict_strategy" => {
            config.settings.conflict_strategy = match val {
                "skip" => ConflictStrategy::Skip,
                _ => ConflictStrategy::Newest,
            };
        }
        "propagate_deletes" => {
            config.settings.propagate_deletes = val == "true";
        }
        "max_concurrency" => {
            if let Ok(v) = val.parse::<usize>() {
                config.settings.max_concurrency = v;
            }
        }
        "max_per_host_concurrency" => {
            if let Ok(v) = val.parse::<usize>() {
                config.settings.max_per_host_concurrency = v;
            }
        }
        "state_dir" => {
            config.settings.state_dir = if val.is_empty() {
                None
            } else {
                Some(std::path::PathBuf::from(val))
            }
        }
        "default_output_format" => {
            config.settings.default_output_format = if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            }
        }
        _ => {}
    }
}

fn apply_host_field(host: &mut HostEntry, idx: usize, val: &str) {
    let fields = host_descriptors(host);
    if idx >= fields.len() {
        return;
    }
    match fields[idx].key.as_str() {
        "name" => host.name = val.to_string(),
        "ssh_host" => host.ssh_host = val.to_string(),
        "shell" => {
            host.shell = match val {
                "powershell" => ShellType::PowerShell,
                "cmd" => ShellType::Cmd,
                _ => ShellType::Sh,
            }
        }
        "proxy_jump" => {
            host.proxy_jump = if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            }
        }
        _ => {}
    }
}

fn apply_check_field(check: &mut CheckEntry, idx: usize, val: &str) {
    let fields = check_descriptors(check);
    if idx >= fields.len() {
        return;
    }
    match fields[idx].key.as_str() {
        "enable_hosts" => check.enable_hosts = val == "true",
        "enable_all" => check.enable_all = val == "true",
        k if k.starts_with("path[") => {
            // path editing is complex; handled via forms
        }
        _ => {}
    }
}

fn apply_sync_field(sync: &mut SyncEntry, idx: usize, val: &str) {
    let fields = sync_descriptors(sync);
    if idx >= fields.len() {
        return;
    }
    match fields[idx].key.as_str() {
        "enable_hosts" => sync.enable_hosts = val == "true",
        "enable_all" => sync.enable_all = val == "true",
        "recursive" => sync.recursive = val == "true",
        "mode" => {
            sync.mode = if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            }
        }
        "propagate_deletes" => {
            sync.propagate_deletes = tribool_to_opt(val);
        }
        "source" => {
            sync.source = if val.is_empty() {
                None
            } else {
                Some(val.to_string())
            }
        }
        _ => {}
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

fn tribool_from_opt(v: Option<bool>) -> &'static str {
    match v {
        None => "inherit",
        Some(true) => "yes",
        Some(false) => "no",
    }
}

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
        vp.set_dims(available.len().max(1), 0);
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

fn tribool_cycle_back(s: &str) -> &'static str {
    match s {
        "no" => "yes",
        "yes" => "inherit",
        _ => "no",
    }
}

fn tribool_to_opt(s: &str) -> Option<bool> {
    match s {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
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

fn shell_cycle_back(s: &str) -> String {
    if !SHELL_VARIANTS.contains(&s) {
        tracing::warn!(shell = s, "unknown shell value, defaulting to sh");
    }
    enum_cycle(SHELL_VARIANTS, s, false)
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

fn parse_bracket_list(s: &str) -> Vec<String> {
    let inner = s.trim_start_matches('[').trim_end_matches(']');
    if inner.is_empty() || inner == "(none)" || inner == "(unscoped)" {
        return vec![];
    }
    inner
        .split(',')
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
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
}
