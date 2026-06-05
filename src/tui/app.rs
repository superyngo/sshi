//! `App` struct + main event loop + render dispatch.
//!
//! Phase 1a scope (per docs/tui_reconstruct_plan.md §19): tab bar,
//! Config/Operate placeholders, minimal Checkout host table, status bar
//! with red `app.error`, terminal-size guard, minimal `?` help popup,
//! signal handlers (SIGHUP/SIGTERM on Unix, ctrl_c on Windows).
//!
//! Phase 4 (§19): Config tab 3-level read-only browser (section → entry → field)
//! + external editor 4-stage flow (§7.4) with config_mtime change detection.

use std::io::{self, Write as _};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, terminal,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Tabs, Wrap},
    Terminal,
};

use crate::cli::ActionFilter;
use crate::commands::checkout::{fetch_latest_snapshots, DisplayColumns, HostSnapshot};
use crate::commands::report::{CommandReport, HostStatus};
use crate::commands::{Context, TargetMode};
use crate::config::schema::AppConfig;
use crate::tui::state::persist::ViewOperationKind;

use super::async_bridge::{EventSender, RunningOp, TuiEvent};
use super::components::input_field::{InputField, InputMode};
use super::components::member_picker::{MemberPicker, PickerResult, PickerTarget};
use super::components::popup::centered_rect;
use super::components::viewport::Viewport;
use super::log_layer::LogBufferHandle;
use super::state::persist::{
    self, ActiveTab, OperationKind, TargetFilterMode, TargetFilterState, TuiPersistedState,
};
use super::tabs::config_tab::trunc;
use super::tabs::config_tab::ConfigTabState;
use super::tabs::config_tab::ConfigZone;
use super::tabs::operate_schema;
use super::tabs::operate_tab::{self, OpField, OperateRenderData};
use super::tabs::TabId;
use super::theme::Theme;
use crate::host::auth::{SshAuthRequest, SshAuthSender};
use operate_tab::truncate;

/// Persist `config` to `path` if `dirty` is set; clear `dirty` on success.
/// On failure prints to stderr — the caller is presumed to be the shutdown
/// path where no UI is available to display errors.
fn flush_config_if_dirty(dirty: &mut bool, config: &AppConfig, path: Option<&std::path::Path>) {
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

const MIN_COLS: u16 = 60;
const MIN_ROWS: u16 = 20;
const POLL_INTERVAL_MS: u64 = 50;

/// Focus zone within the View tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewFocus {
    OpSelector,
    /// Inline Common-zone target mode radio (Checkout/List only).
    TargetMode,
    /// Inline Common-zone group/host members (Checkout/List, non-All modes).
    TargetMembers,
    /// Inline Common-zone skip-host list (Checkout/List only).
    Skip,
    Specific(usize),
    Result,
}

impl ViewFocus {
    /// Return the ordered focus stops for the given view operation.
    ///
    /// Checkout/List expose an inline Common zone (target mode → members →
    /// skip) in place of the old `f` filter popup; Log has no target.
    fn stops(op: ViewOperationKind, mode: TargetFilterMode) -> Vec<ViewFocus> {
        match op {
            ViewOperationKind::Checkout | ViewOperationKind::List => {
                let mut v = vec![ViewFocus::OpSelector, ViewFocus::TargetMode];
                if mode != TargetFilterMode::All {
                    v.push(ViewFocus::TargetMembers);
                }
                v.push(ViewFocus::Skip);
                v.push(ViewFocus::Result);
                v
            }
            ViewOperationKind::Log => vec![
                ViewFocus::OpSelector,
                ViewFocus::Specific(0),
                ViewFocus::Specific(1),
                ViewFocus::Specific(2),
                ViewFocus::Specific(3),
                ViewFocus::Specific(4),
                ViewFocus::Result,
            ],
        }
    }
}

/// State for the masked SSH auth credential popup.
struct AuthPopup {
    prompt: String,
    input: InputField,
    responder: Option<tokio::sync::oneshot::Sender<String>>,
}

impl AuthPopup {
    fn new(req: SshAuthRequest) -> Self {
        let mut input = InputField::new("");
        input.activate();
        Self {
            prompt: req.prompt,
            input,
            responder: Some(req.responder),
        }
    }

    /// Consume the popup, sending the credential. Zeroizes the input buffer.
    fn submit(&mut self) {
        let credential = std::mem::take(&mut self.input.value);
        if let Some(tx) = self.responder.take() {
            let _ = tx.send(credential);
        }
    }

    /// Dismiss without sending (drops sender → auth failure).
    fn cancel(&mut self) {
        self.input.value.clear();
        self.responder = None;
    }
}

/// State for the "Export to file" popup in View tab.
struct ExportPopup {
    input: InputField,
    source: ViewOperationKind,
}

impl ExportPopup {
    fn new(source: ViewOperationKind) -> Self {
        let mut input = InputField::new("");
        input.activate();
        Self { input, source }
    }
}

pub struct App {
    pub active_tab: TabId,
    export_popup: Option<ExportPopup>,
    navbar_focused: bool,
    pub theme: Theme,
    pub error: Option<String>,
    pub help_open: bool,
    pub should_quit: bool,
    pub info_open: bool,
    pub checkout_viewport: Viewport,
    pub checkout_snapshots: Vec<HostSnapshot>,
    /// Unfiltered cache for Checkout; `checkout_snapshots` is the filtered view.
    checkout_all_snapshots: Vec<HostSnapshot>,
    pub checkout_columns: DisplayColumns,
    pub config: AppConfig,
    pub config_path: Option<PathBuf>,
    /// 3-level Config tab browser state (Phase 4).
    config_tab: ConfigTabState,
    /// Set by `handle_key` when `E` is pressed; drained by `run()` after each event.
    needs_editor_open: bool,
    pub state_file_path: PathBuf,
    pub target_filter: TargetFilterState,
    operate_focus: OpField,
    /// Open member picker (groups/hosts/skip/shell), if any. Focus root.
    member_picker: Option<MemberPicker>,
    /// When a name picker triggered `a` (add entry), the picker target to reopen
    /// on the Operate tab once the Config add-entry form closes.
    reopen_name_picker: Option<PickerTarget>,
    /// Shared dry-run toggle, shown by the Execute button (`d` toggles).
    op_dry_run: bool,
    /// Currently-selected operation on the Operate tab.
    operate_operation: OperationKind,
    /// Text input for the `run` command field (NOT persisted per AD-12).
    run_command: InputField,
    /// Text input for the `exec` script path field (NOT persisted per AD-12).
    exec_script: InputField,
    /// Text inputs for the `cp` local/remote path fields (NOT persisted per AD-12).
    cp_local: InputField,
    cp_remote: InputField,
    /// Text inputs for check/sync config-entry name selection (NOT persisted).
    check_name: InputField,
    sync_name: InputField,
    /// Sudo / keep boolean params (persisted per AD-12).
    run_sudo: bool,
    exec_sudo: bool,
    exec_keep: bool,
    /// Sync params (sync_dry_run persisted; adhoc_files NOT persisted per AD-12).
    sync_dry_run: bool,
    sync_adhoc_files: Vec<String>,
    sync_adhoc_input: InputField,
    /// Source host override for sync (NOT persisted per AD-12).
    sync_source_input: InputField,
    /// `-o/--out` report path, shared across operations (NOT persisted).
    out_input: InputField,
    /// Currently-running operation, if any. Mutually exclusive with starting
    /// a new one (concurrency guard per Phase 3 step 10).
    running_op: Option<RunningOp>,
    /// Bridge channel sender. Spawned tasks clone this via `EventSender`.
    event_tx: tokio::sync::mpsc::UnboundedSender<TuiEvent>,
    /// Bridge receiver, drained by the main loop.
    event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<TuiEvent>>,
    /// Final report from the most recently completed operation, shown in the
    /// results popup until dismissed.
    completed_report: Option<CommandReport>,
    /// True when a snapshot DB write happened in this session and the
    /// Checkout tab needs to reload before its next render (§18.3).
    db_stale: bool,
    /// View tab: currently selected operation.
    view_op: super::state::persist::ViewOperationKind,
    /// View tab: cached list result.
    view_list: Option<crate::commands::list::ListData>,
    /// View tab: cached log result rows.
    view_log: Vec<crate::commands::log::LogRow>,
    /// View tab: stale flag — triggers refresh on next render.
    view_dirty: bool,
    /// View tab: loading spinner (unused stub for 11a).
    view_loading: bool,
    /// View tab: log_last parameter (0 → 20 default).
    log_last: usize,
    /// View tab: log_errors filter flag.
    log_errors: bool,
    /// View tab: log action filter.
    log_action: Option<ActionFilter>,
    /// View tab: text inputs for log params.
    log_last_input: InputField,
    log_since_input: InputField,
    log_host_input: InputField,
    /// View tab: which zone currently has focus.
    view_focus: ViewFocus,
    /// Tracks the most recent timeout used (filter timeout or default).
    last_timeout_secs: u64,
    /// Log overlay open state (Phase 7, §17.3 item 3).
    log_overlay_open: bool,
    /// Log overlay viewport for scrolling.
    log_overlay_vp: Viewport,
    /// Log buffer: in-memory ring of tracing events (§17.2).
    log_buffer: Option<LogBufferHandle>,
    /// Active SSH auth popup, if any. Takes highest key-routing priority after Ctrl+C.
    auth_popup: Option<AuthPopup>,
    /// Sender side of the auth bridge channel; cloned into each execute operation.
    auth_bridge_tx: Option<SshAuthSender>,
    /// User-controlled scroll offset for the progress popup (None = auto-scroll to bottom).
    progress_popup_scroll: Option<usize>,
}

impl App {
    pub fn from_context(ctx: &Context, log_buffer: Option<LogBufferHandle>) -> Self {
        let columns = DisplayColumns::from_context(ctx);
        let host_names: Vec<&str> = ctx.config.host.iter().map(|h| h.name.as_str()).collect();
        let snapshots = if host_names.is_empty() {
            Vec::new()
        } else {
            fetch_latest_snapshots(ctx, &host_names).unwrap_or_default()
        };
        let mut viewport = Viewport::new();
        viewport.set_dims(snapshots.len(), 0);

        // Resolve TUI state file path; on failure fall back to a path in the
        // OS temp dir so save/load remain functional even with unusual configs.
        let state_file_path = persist::state_file_path(&ctx.config, ctx.config_path.as_deref())
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to resolve TUI state path; using temp dir: {e}");
                std::env::temp_dir().join("sshi_tui_state.toml")
            });

        // Load persisted state and validate against current config (§16.2).
        let mut persisted = persist::load(&state_file_path);
        persist::validate_filter(&mut persisted.target_filter, &ctx.config);

        let active_tab = persisted.tui_state.active_tab.to_tab_id();
        let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
        let config_tab = ConfigTabState::new(&ctx.config, ctx.config_path.as_deref());
        let timeout = if persisted.target_filter.timeout > 0 {
            persisted.target_filter.timeout
        } else {
            ctx.config.settings.default_timeout
        };
        // Normalize the persisted field to the resolved value so the Timeout
        // row displays the real default (not "0s") and last_timeout_secs stays
        // in sync when execute reads target_filter.timeout.
        persisted.target_filter.timeout = timeout;

        Self {
            active_tab,
            navbar_focused: false,
            theme: Theme::default_palette(),
            error: None,
            help_open: false,
            should_quit: false,
            info_open: false,
            checkout_viewport: viewport,
            checkout_snapshots: snapshots.clone(),
            checkout_all_snapshots: snapshots,
            checkout_columns: columns,
            config: ctx.config.clone(),
            config_path: ctx.config_path.clone(),
            config_tab,
            needs_editor_open: false,
            state_file_path,
            target_filter: persisted.target_filter,
            operate_focus: OpField::OpRadio,
            member_picker: None,
            reopen_name_picker: None,
            op_dry_run: persisted.operate.sync_dry_run,
            operate_operation: persisted.operate.operation,
            run_command: InputField::new(""),
            exec_script: InputField::new(""),
            cp_local: InputField::new(""),
            cp_remote: InputField::new(""),
            check_name: InputField::new(""),
            sync_name: InputField::new(""),
            run_sudo: persisted.operate.run_sudo,
            exec_sudo: persisted.operate.exec_sudo,
            exec_keep: persisted.operate.exec_keep,
            sync_dry_run: persisted.operate.sync_dry_run,
            sync_adhoc_files: Vec::new(),
            sync_adhoc_input: InputField::new(""),
            sync_source_input: InputField::new(""),
            out_input: InputField::new(""),
            running_op: None,
            event_tx,
            event_rx: Some(event_rx),
            completed_report: None,
            db_stale: false,
            view_op: persisted.operate.view_operation,
            view_list: None,
            view_log: Vec::new(),
            view_dirty: true,
            view_loading: false,
            log_last: if persisted.operate.log_last == 0 {
                20
            } else {
                persisted.operate.log_last
            },
            log_errors: persisted.operate.log_errors,
            log_action: None,
            log_last_input: InputField::new(&if persisted.operate.log_last == 0 {
                "20".to_string()
            } else {
                persisted.operate.log_last.to_string()
            }),
            log_since_input: InputField::new(""),
            log_host_input: InputField::new(""),
            view_focus: ViewFocus::OpSelector,
            last_timeout_secs: timeout,
            log_overlay_open: false,
            log_overlay_vp: Viewport::new(),
            log_buffer,
            auth_popup: None,
            export_popup: None,
            auth_bridge_tx: None,
            progress_popup_scroll: None,
        }
    }

    /// Ordered list of focusable Operate fields for the current op/mode.
    fn operate_field_list(&self) -> Vec<OpField> {
        operate_tab::operate_fields(self.operate_operation, self.target_filter.mode)
    }

    /// Move Operate focus by `delta` steps through the field list. Moving up
    /// past the first field escapes to the NavBar; moving down past the last
    /// stays put.
    fn operate_move_focus(&mut self, delta: i32) {
        let list = self.operate_field_list();
        let pos = list
            .iter()
            .position(|f| *f == self.operate_focus)
            .unwrap_or(0) as i32;
        let next = pos + delta;
        if next < 0 {
            self.navbar_focused = true;
        } else if (next as usize) < list.len() {
            self.operate_focus = list[next as usize];
        }
    }

    /// Cycle the Operate operation radio (run/exec/sync/cp/check). Shared by ←→
    /// and Tab/BackTab while the operation radio holds focus.
    fn operate_cycle_operation(&mut self, forward: bool) {
        self.operate_operation = cycle_operation(self.operate_operation, forward);
        self.save_state();
    }

    /// Cycle the Operate target mode (All → Groups → Hosts → Shell). Shared by
    /// ←→ and Tab/BackTab while the target row holds focus. Doesn't validate the
    /// mode here — that would force an empty Groups/Hosts selection back to All
    /// before the user can pick members.
    fn operate_cycle_target_mode(&mut self, forward: bool) {
        self.target_filter.mode = cycle_target_mode(self.target_filter.mode, forward);
        // The Members row only exists for non-All modes; if we just switched to
        // All while focused there, keep focus on the Target row.
        if self.target_filter.mode == TargetFilterMode::All {
            self.operate_focus = OpField::TargetMode;
        }
        self.save_state();
        self.apply_checkout_filter();
        self.view_dirty = true;
    }

    /// Run the currently-selected operation. Shared by Enter-on-Execute and the
    /// `e` hotkey. Returns whether the frame should redraw.
    fn trigger_execute(&mut self) -> bool {
        // Feed the shared dry-run toggle into the sync path.
        self.sync_dry_run = self.op_dry_run;
        // Ensure the per-host timeout reflects the Timeout field.
        self.last_timeout_secs = self.target_filter.timeout;
        // sync_core honours dry-run internally; check/run/exec cores do not, so
        // preview them synthetically without contacting any host.
        if self.op_dry_run && !matches!(self.operate_operation, OperationKind::Sync) {
            return self.dry_run_preview();
        }
        match self.operate_operation {
            OperationKind::Check => self.execute_check(),
            OperationKind::Run => self.execute_run(),
            OperationKind::Exec => self.execute_exec(),
            OperationKind::Sync => self.execute_sync(),
            OperationKind::Cp => self.execute_cp(),
        }
    }

    /// Tab/BackTab: cycle the focus among peers in the *current layer* only,
    /// wrapping at the layer's ends. Arrow keys (operate_move_focus) cross
    /// layer boundaries; Tab never leaves the layer (mirrors Config zones).
    fn operate_tab_cycle(&mut self, forward: bool) {
        // Radios cycle their value on Tab (matching ←→): the operation radio is
        // alone in its layer, and the target row cycles its mode in place while
        // ↑↓ still steps to the other Common fields.
        match self.operate_focus {
            OpField::OpRadio => {
                self.operate_cycle_operation(forward);
                return;
            }
            OpField::TargetMode | OpField::TargetMembers => {
                self.operate_cycle_target_mode(forward);
                return;
            }
            _ => {}
        }
        let layer = operate_tab::layer_of(self.operate_focus);
        let peers: Vec<OpField> = self
            .operate_field_list()
            .into_iter()
            .filter(|f| operate_tab::layer_of(*f) == layer)
            .collect();
        if peers.len() < 2 {
            return;
        }
        let pos = peers
            .iter()
            .position(|f| *f == self.operate_focus)
            .unwrap_or(0);
        let next = if forward {
            (pos + 1) % peers.len()
        } else {
            (pos + peers.len() - 1) % peers.len()
        };
        self.operate_focus = peers[next];
    }

    /// All group names referenced anywhere in the config (hosts, check and
    /// sync entries), plus any currently-selected groups, sorted + de-duped.
    /// Mirrors `config_tab::collect_known_groups` so the Operate picker offers
    /// the same set the Config tab does.
    fn available_groups(&self) -> Vec<String> {
        let mut known: std::collections::BTreeSet<String> = self
            .config
            .host
            .iter()
            .flat_map(|h| h.groups.iter().cloned())
            .filter(|g| !g.is_empty())
            .collect();
        for g in &self.target_filter.groups {
            known.insert(g.clone());
        }
        known.into_iter().collect()
    }

    /// All configured host names.
    fn available_hosts(&self) -> Vec<String> {
        self.config.host.iter().map(|h| h.name.clone()).collect()
    }

    /// Cycle the View op (Checkout → List → Log) and refresh. Shared by ←→ and
    /// Tab/BackTab while the Op selector holds focus.
    fn cycle_view_op(&mut self, forward: bool) {
        self.view_op = if forward {
            match self.view_op {
                ViewOperationKind::Checkout => ViewOperationKind::List,
                ViewOperationKind::List => ViewOperationKind::Log,
                ViewOperationKind::Log => ViewOperationKind::Checkout,
            }
        } else {
            match self.view_op {
                ViewOperationKind::Checkout => ViewOperationKind::Log,
                ViewOperationKind::List => ViewOperationKind::Checkout,
                ViewOperationKind::Log => ViewOperationKind::List,
            }
        };
        self.view_focus = ViewFocus::OpSelector;
        self.view_dirty = true;
    }

    /// Cycle the View target mode through All → Groups → Hosts → Shell
    /// (Shell filters hosts by detected shell type), keeping focus sane.
    fn view_cycle_target_mode(&mut self, forward: bool) {
        let order = [
            TargetFilterMode::All,
            TargetFilterMode::Groups,
            TargetFilterMode::Hosts,
            TargetFilterMode::Shell,
        ];
        let pos = order
            .iter()
            .position(|m| *m == self.target_filter.mode)
            .unwrap_or(0);
        let next = if forward {
            (pos + 1) % order.len()
        } else {
            (pos + order.len() - 1) % order.len()
        };
        self.target_filter.mode = order[next];
        if self.target_filter.mode == TargetFilterMode::All
            && self.view_focus == ViewFocus::TargetMembers
        {
            self.view_focus = ViewFocus::TargetMode;
        }
        self.save_state();
        self.apply_checkout_filter();
        self.view_dirty = true;
    }

    /// Accent colour of the currently-active tab (drives borders, focus, and
    /// any popup the tab opens, for a consistent per-tab visual identity).
    fn tab_accent(&self) -> ratatui::style::Color {
        match self.active_tab {
            TabId::Config => self.theme.accent_config,
            TabId::Operate => self.theme.accent_operate,
            TabId::View => self.theme.accent_checkout,
        }
    }

    /// Open the member picker appropriate to the current target mode.
    fn open_member_picker(&mut self) {
        let accent = self.tab_accent();
        let picker = match self.target_filter.mode {
            TargetFilterMode::Groups => MemberPicker::new(
                PickerTarget::Groups,
                self.available_groups(),
                &self.target_filter.groups,
                accent,
            ),
            TargetFilterMode::Hosts => MemberPicker::new(
                PickerTarget::Hosts,
                self.available_hosts(),
                &self.target_filter.hosts,
                accent,
            ),
            TargetFilterMode::Shell => {
                let current = match self.target_filter.shell {
                    super::state::persist::ShellMode::Sh => "sh",
                    super::state::persist::ShellMode::PowerShell => "powershell",
                    super::state::persist::ShellMode::Cmd => "cmd",
                };
                MemberPicker::new(
                    PickerTarget::Shell,
                    vec!["sh".into(), "powershell".into(), "cmd".into()],
                    &[current.to_string()],
                    accent,
                )
            }
            // All mode has no TargetMembers field; nothing to open.
            TargetFilterMode::All => return,
        };
        self.member_picker = Some(picker);
    }

    /// Names of all `[[check]]` / `[[sync]]` entries, in config order.
    fn config_entry_names(&self, target: PickerTarget) -> Vec<String> {
        match target {
            PickerTarget::CheckNames => self
                .config
                .check
                .iter()
                .filter_map(|c| c.name.clone())
                .filter(|n| !n.is_empty())
                .collect(),
            PickerTarget::SyncNames => self
                .config
                .sync
                .iter()
                .filter_map(|s| s.name.clone())
                .filter(|n| !n.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Per-entry detail hints shown after each name in the picker (metrics for
    /// check entries; paths + source for sync entries).
    fn config_entry_descriptions(&self, target: PickerTarget) -> Vec<String> {
        match target {
            PickerTarget::CheckNames => self
                .config
                .check
                .iter()
                .filter(|c| c.name.as_deref().is_some_and(|n| !n.is_empty()))
                .map(|c| {
                    if c.enabled.is_empty() {
                        "(no metrics)".to_string()
                    } else {
                        format!("— {}", c.enabled.join(","))
                    }
                })
                .collect(),
            PickerTarget::SyncNames => self
                .config
                .sync
                .iter()
                .filter(|s| s.name.as_deref().is_some_and(|n| !n.is_empty()))
                .map(|s| {
                    let paths = s
                        .paths
                        .iter()
                        .take(3)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ");
                    let src = s
                        .source
                        .as_deref()
                        .filter(|v| !v.is_empty())
                        .map(|v| format!("  src:{v}"))
                        .unwrap_or_default();
                    format!("— {paths}{src}")
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Open the multi-select name picker for the check/sync entry field,
    /// pre-checking whatever the field's comma value already holds.
    fn open_name_picker(&mut self, target: PickerTarget) {
        let current = match target {
            PickerTarget::CheckNames => comma_names(&self.check_name.value),
            PickerTarget::SyncNames => comma_names(&self.sync_name.value),
            _ => return,
        };
        let accent = self.tab_accent();
        self.member_picker = Some(
            MemberPicker::new(target, self.config_entry_names(target), &current, accent)
                .with_descriptions(self.config_entry_descriptions(target)),
        );
    }

    /// Cycle the sync source override in place: none → host1 → … → none.
    /// Mirrors the Space-to-cycle behaviour of the Target Shell value.
    fn cycle_sync_source(&mut self) {
        let hosts = self.available_hosts();
        let cur = self.sync_source_input.value.trim().to_string();
        // Slot 0 = "(none)"; slots 1.. map to hosts.
        let pos = if cur.is_empty() {
            0
        } else {
            hosts.iter().position(|h| h == &cur).map_or(0, |i| i + 1)
        };
        let next = (pos + 1) % (hosts.len() + 1);
        self.sync_source_input.value = if next == 0 {
            String::new()
        } else {
            hosts[next - 1].clone()
        };
    }

    /// Open the single-select source-host picker for sync. A leading "(none)"
    /// option clears the override; the current value is pre-selected.
    fn open_source_picker(&mut self) {
        let mut options = vec!["(none)".to_string()];
        options.extend(self.available_hosts());
        let cur = self.sync_source_input.value.trim();
        let current = if cur.is_empty() {
            vec!["(none)".to_string()]
        } else {
            vec![cur.to_string()]
        };
        let accent = self.tab_accent();
        self.member_picker = Some(MemberPicker::new(
            PickerTarget::SyncSource,
            options,
            &current,
            accent,
        ));
    }

    /// Jump from a name picker to the Config add-entry form. The picker is
    /// reopened on the Operate tab once the form closes (commit or cancel),
    /// via the `reopen_name_picker` flag drained after Config key handling.
    fn open_add_entry_from_picker(&mut self, target: PickerTarget) {
        use super::tabs::config_tab::EntryFormKind;
        let kind = match target {
            PickerTarget::CheckNames => EntryFormKind::Check,
            PickerTarget::SyncNames => EntryFormKind::Sync,
            _ => return,
        };
        self.reopen_name_picker = Some(target);
        self.active_tab = TabId::Config;
        self.navbar_focused = false;
        self.config_tab.start_add_entry(kind);
    }

    fn save_state(&self) {
        let state = TuiPersistedState {
            tui_state: super::state::persist::TuiSection {
                active_tab: ActiveTab::from_tab_id(self.active_tab),
            },
            target_filter: self.target_filter.clone(),
            operate: super::state::persist::OperateState {
                operation: self.operate_operation,
                run_sudo: self.run_sudo,
                exec_sudo: self.exec_sudo,
                exec_keep: self.exec_keep,
                sync_dry_run: self.sync_dry_run,
                view_operation: self.view_op,
                log_last: self.log_last,
                log_errors: self.log_errors,
                ..Default::default()
            },
        };
        if let Err(e) = persist::save(&self.state_file_path, &state) {
            tracing::warn!(
                "Failed to save TUI state to {}: {e}",
                self.state_file_path.display()
            );
        }
    }

    /// Run the main event loop. Returns when the user quits cleanly.
    pub async fn run(&mut self) -> Result<()> {
        let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        terminal.clear()?;

        // Set up an async signal listener that flips should_quit.
        // This lives for the duration of the loop.
        let (sig_tx, mut sig_rx) = tokio::sync::mpsc::channel::<()>(4);
        spawn_signal_listener(sig_tx);

        // Move the event_rx out of self for the loop's lifetime. Rebuild before any
        // future call would need it (we never re-enter this method).
        let mut event_rx = self
            .event_rx
            .take()
            .expect("event_rx is Some after construction");

        // Bridge: convert SshAuthRequest events to TuiEvent::SshAuthRequired.
        let (auth_tx, mut auth_rx) = tokio::sync::mpsc::unbounded_channel::<SshAuthRequest>();
        self.auth_bridge_tx = Some(auth_tx);
        let event_tx_for_bridge = self.event_tx.clone();
        tokio::spawn(async move {
            while let Some(req) = auth_rx.recv().await {
                let _ = event_tx_for_bridge.send(TuiEvent::SshAuthRequired(req));
            }
        });

        let mut dirty = true;
        loop {
            if self.should_quit {
                self.save_state();
                self.flush_dirty_config_to_disk();
                break;
            }

            // Drain any pending signals (non-blocking).
            while let Ok(()) = sig_rx.try_recv() {
                self.should_quit = true;
                dirty = true;
            }

            // Drain bridge events (non-blocking) before rendering.
            while let Ok(ev) = event_rx.try_recv() {
                if self.handle_tui_event(ev) {
                    dirty = true;
                }
            }

            if dirty {
                terminal.draw(|f| self.render(f.area(), f))?;
                dirty = false;
            }

            // Poll crossterm with a short timeout so signal & dirty paths stay
            // responsive without busy-looping.
            if event::poll(Duration::from_millis(POLL_INTERVAL_MS))? {
                let ev = event::read()?;
                if self.handle_event(ev)? {
                    dirty = true;
                }
            }

            if self.config_tab.pending_open_editor {
                self.config_tab.pending_open_editor = false;
                self.needs_editor_open = true;
                dirty = true;
            }

            // Open external editor if requested (§7.4 4-stage flow).
            if self.needs_editor_open {
                self.needs_editor_open = false;
                self.do_open_editor(&mut terminal)?;
                dirty = true;
            }

            // Expire the "Config reloaded" banner so it disappears after 2s.
            if let Some(until) = self.config_tab.reload_banner_until {
                if Instant::now() >= until {
                    self.config_tab.reload_banner_until = None;
                    dirty = true;
                }
            }
        }

        Ok(())
    }

    /// Handle an inbound TuiEvent from a running operation. Returns true if
    /// state changed and a redraw is needed.
    fn handle_tui_event(&mut self, ev: TuiEvent) -> bool {
        match ev {
            TuiEvent::HostStarted(_host) => {
                // Phase 3: rendering reads from running_op.host_outcomes; the
                // started signal is informational. Future phases may track
                // in-flight hosts explicitly.
                true
            }
            TuiEvent::HostCompleted {
                host,
                status,
                detail,
                duration_ms,
            } => {
                if let Some(op) = self.running_op.as_mut() {
                    op.record_completed(&host, status, &detail, duration_ms);
                }
                true
            }
            TuiEvent::OperationFinished(report) => {
                // Write a `-o/--out` report file if one was requested for this run.
                if let Some(op) = self.running_op.as_ref() {
                    if let Some(out) = op.out.clone() {
                        let command = match &report {
                            CommandReport::Check(_) => "check",
                            CommandReport::Run(_) => "run",
                            CommandReport::Exec(_) => "exec",
                            CommandReport::Sync(_) => "sync",
                            CommandReport::Cp(_) => "cp",
                            CommandReport::Log(_) => "log",
                            CommandReport::List(_) => "list",
                        };
                        let op_report =
                            crate::output::report::to_operation_report(&report, &op.mode);
                        match crate::output::report::write_report(
                            &op_report,
                            &out,
                            command,
                            self.config.settings.default_output_format.as_deref(),
                        ) {
                            Ok(path) => self.error = Some(format!("Report written to {path}")),
                            Err(e) => self.error = Some(format!("Report write failed: {e}")),
                        }
                    }
                }
                self.running_op = None;
                self.db_stale = true;
                // View tab data is now stale — force a refresh on next render.
                self.view_dirty = true;
                self.completed_report = Some(report);
                true
            }
            TuiEvent::OperationCancelled => {
                self.running_op = None;
                self.db_stale = true;
                self.error = Some("Operation cancelled".to_string());
                true
            }
            TuiEvent::OperationError(msg) => {
                self.running_op = None;
                self.error = Some(format!("Operation failed: {msg}"));
                true
            }
            TuiEvent::SshAuthRequired(req) => {
                self.auth_popup = Some(AuthPopup::new(req));
                true
            }
        }
    }

    /// Trimmed `-o/--out` report path, or None when the field is empty.
    fn out_path(&self) -> Option<String> {
        let v = self.out_input.value.trim().to_string();
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    }

    fn execute_view_export(&mut self, path: &str) -> Result<()> {
        let target_mode = build_target_mode(&self.target_filter, &self.config);
        let executed_at = chrono::Local::now().to_rfc3339();

        let (op_report, command) = match self.view_op {
            ViewOperationKind::Checkout => {
                use crate::output::report::{
                    FilterInfo, HostResult, OperationReport, ReportSummary,
                };

                let report_results: Vec<HostResult> = self
                    .checkout_snapshots
                    .iter()
                    .map(|snap| {
                        let collected_at_str = if snap.collected_at > 0 {
                            chrono::DateTime::from_timestamp(snap.collected_at, 0)
                                .map(|dt| dt.to_rfc3339())
                                .unwrap_or_else(|| snap.collected_at.to_string())
                        } else {
                            "never".to_string()
                        };
                        HostResult {
                            host: snap.host.clone(),
                            status: if snap.online { "success" } else { "error" }.to_string(),
                            duration_ms: None,
                            output: serde_json::json!({
                                "collected_at": collected_at_str,
                                "online": snap.online,
                                "snapshot": snap.data,
                            }),
                        }
                    })
                    .collect();

                let rep_summary = ReportSummary {
                    total: report_results.len(),
                    success: report_results
                        .iter()
                        .filter(|r| r.status == "success")
                        .count(),
                    failed: report_results
                        .iter()
                        .filter(|r| r.status == "error")
                        .count(),
                    skipped: 0,
                };

                let targets: Vec<String> = self
                    .checkout_snapshots
                    .iter()
                    .map(|s| s.host.clone())
                    .collect();

                let op_report = OperationReport {
                    executed_at,
                    command: "checkout".to_string(),
                    filter: FilterInfo::from_mode(&target_mode),
                    task: serde_json::json!({}),
                    targets,
                    results: report_results,
                    summary: rep_summary,
                };
                (op_report, "checkout")
            }
            ViewOperationKind::Log => {
                use crate::commands::report::{
                    CommandReport, HostStatus, LogHostResult, LogQueryParams, LogReport,
                };

                let entries: Vec<LogHostResult> = self
                    .view_log
                    .iter()
                    .map(|r| {
                        let status = match r.status.as_str() {
                            "ok" => HostStatus::Online,
                            "skipped" => HostStatus::Skipped,
                            _ => HostStatus::Error,
                        };
                        let time_str = chrono::DateTime::from_timestamp(r.ts, 0)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| r.ts.to_string());
                        LogHostResult {
                            host: r.host.clone(),
                            status,
                            duration_ms: r.duration_ms,
                            timestamp: time_str,
                            command: r.command.clone(),
                            action: r.action.clone(),
                            note: r.note.clone(),
                        }
                    })
                    .collect();

                let action_filter_str = self.log_action.as_ref().map(|a| match a {
                    ActionFilter::Sync => "sync".to_string(),
                    ActionFilter::Run => "run".to_string(),
                    ActionFilter::Exec => "exec".to_string(),
                    ActionFilter::Check => "check".to_string(),
                });

                let report = CommandReport::Log(LogReport {
                    executed_at: executed_at.clone(),
                    query_params: LogQueryParams {
                        last: self.log_last,
                        since: {
                            let val = self.log_since_input.value.trim().to_string();
                            if val.is_empty() {
                                None
                            } else {
                                Some(val)
                            }
                        },
                        host: {
                            let val = self.log_host_input.value.trim().to_string();
                            if val.is_empty() {
                                None
                            } else {
                                Some(val)
                            }
                        },
                        action: action_filter_str,
                        errors: self.log_errors,
                    },
                    entries,
                });

                let op_report = crate::output::report::to_operation_report(&report, &target_mode);
                (op_report, "log")
            }
            ViewOperationKind::List => {
                use crate::commands::report::{CommandReport, ListHostResult, ListReport};

                let list_data = self
                    .view_list
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("No list data loaded"))?;

                let list_hosts: Vec<ListHostResult> = list_data
                    .hosts
                    .iter()
                    .map(|h| ListHostResult {
                        host: h.name.clone(),
                        ssh_host: h.ssh_host.clone(),
                        shell: h.shell.to_string(),
                        groups: h.groups.clone(),
                    })
                    .collect();

                let report = CommandReport::List(ListReport {
                    executed_at: executed_at.clone(),
                    targets: list_data.hosts.iter().map(|h| h.name.clone()).collect(),
                    hosts: list_hosts,
                    checks: list_data.checks.clone(),
                    syncs: list_data.syncs.clone(),
                });

                let op_report = crate::output::report::to_operation_report(&report, &target_mode);
                (op_report, "list")
            }
        };

        let default_fmt = self.config.settings.default_output_format.as_deref();
        let final_path =
            crate::output::report::write_report(&op_report, path, command, default_fmt)?;
        self.error = Some(format!("Report written to {}", final_path));
        Ok(())
    }

    /// Build a synthetic completed report previewing which hosts a
    /// check/run/exec dry-run would touch, without contacting any host. Sync is
    /// excluded (its core honours `dry_run` itself). Returns true (redraw).
    fn dry_run_preview(&mut self) -> bool {
        use crate::commands::report::{
            CheckHostResult, CheckReport, CpHostResult, CpReport, ExecHostResult, ExecReport,
            RunHostResult, RunReport,
        };

        if self.running_op.is_some() {
            self.error = Some("Operation already running".to_string());
            return true;
        }
        let target_mode = build_target_mode(&self.target_filter, &self.config);
        let targets: Vec<String> =
            match resolve_target_names(&target_mode, &self.config, &self.target_filter.skip) {
                Ok(t) if !t.is_empty() => t,
                Ok(_) => {
                    self.error = Some("No hosts matched the current filter.".to_string());
                    return true;
                }
                Err(e) => {
                    self.error = Some(format!("Filter error: {e}"));
                    return true;
                }
            };

        let executed_at = chrono::Utc::now().to_rfc3339();
        let detail = "would execute (dry-run)".to_string();
        let report = match self.operate_operation {
            OperationKind::Check => CommandReport::Check(CheckReport {
                executed_at,
                enabled_metrics: Vec::new(),
                targets: targets.clone(),
                hosts: targets
                    .iter()
                    .map(|h| CheckHostResult {
                        host: h.clone(),
                        status: HostStatus::Skipped,
                        duration_ms: None,
                        detail: "would collect metrics (dry-run)".to_string(),
                        metrics_succeeded: 0,
                        metrics_failed: 0,
                        data: serde_json::json!({}),
                        raw_stdout: String::new(),
                        raw_stderr: String::new(),
                    })
                    .collect(),
            }),
            OperationKind::Run => {
                let command = self.run_command.value.trim().to_string();
                if command.is_empty() {
                    self.error = Some("Command field is empty.".to_string());
                    return true;
                }
                CommandReport::Run(RunReport {
                    executed_at,
                    command,
                    targets: targets.clone(),
                    hosts: targets
                        .iter()
                        .map(|h| RunHostResult {
                            host: h.clone(),
                            status: HostStatus::Skipped,
                            duration_ms: None,
                            detail: detail.clone(),
                            stdout: String::new(),
                            stderr: String::new(),
                        })
                        .collect(),
                })
            }
            OperationKind::Exec => {
                let script = self.exec_script.value.trim().to_string();
                if script.is_empty() {
                    self.error = Some("Script path field is empty.".to_string());
                    return true;
                }
                CommandReport::Exec(ExecReport {
                    executed_at,
                    script,
                    targets: targets.clone(),
                    hosts: targets
                        .iter()
                        .map(|h| ExecHostResult {
                            host: h.clone(),
                            status: HostStatus::Skipped,
                            duration_ms: None,
                            detail: detail.clone(),
                            stdout: String::new(),
                            stderr: String::new(),
                        })
                        .collect(),
                })
            }
            OperationKind::Sync => unreachable!("sync dry-run is handled by sync_core"),
            OperationKind::Cp => {
                let local = self.cp_local.value.trim().to_string();
                if local.is_empty() {
                    self.error = Some("Local path field is empty.".to_string());
                    return true;
                }
                let remote = {
                    let r = self.cp_remote.value.trim();
                    if r.is_empty() {
                        "~".to_string()
                    } else {
                        r.to_string()
                    }
                };
                CommandReport::Cp(CpReport {
                    executed_at,
                    local,
                    remote,
                    planned_files: 0,
                    targets: targets.clone(),
                    hosts: targets
                        .iter()
                        .map(|h| CpHostResult {
                            host: h.clone(),
                            status: HostStatus::Skipped,
                            duration_ms: None,
                            detail: detail.clone(),
                            files_copied: 0,
                            files_failed: 0,
                            errors: Vec::new(),
                        })
                        .collect(),
                })
            }
        };

        let n = targets.len();
        self.progress_popup_scroll = None;
        self.completed_report = Some(report);
        self.error = Some(format!(
            "Dry-run preview — {n} target(s), no hosts contacted"
        ));
        true
    }

    /// Execute a `check` operation against the current target filter. Returns
    /// false (no-op) if an operation is already running (concurrency guard).
    fn execute_check(&mut self) -> bool {
        if self.running_op.is_some() {
            self.error = Some("Operation already running".to_string());
            return true;
        }

        let target_mode = build_target_mode(&self.target_filter, &self.config);
        let targets: Vec<String> =
            match resolve_target_names(&target_mode, &self.config, &self.target_filter.skip) {
                Ok(t) if !t.is_empty() => t,
                Ok(_) => {
                    self.error = Some("No hosts matched the current filter.".to_string());
                    return true;
                }
                Err(e) => {
                    self.error = Some(format!("Filter error: {e}"));
                    return true;
                }
            };

        let serial = self.target_filter.serial;
        let timeout = self.last_timeout_secs;
        let mode_for_op = target_mode.clone();
        let out_for_op = self.out_path();
        let verbose = false;
        let names = comma_names(&self.check_name.value);
        let cfg = self.config.clone();
        let cfg_path = self.config_path.clone();
        let event_tx = self.event_tx.clone();
        let _auth_sender = self.auth_bridge_tx.clone();
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_for_task = cancel.clone();

        // Run the operation on a dedicated OS thread with its own
        // current-thread tokio runtime. This sidesteps the Send constraint
        // imposed by tokio::spawn on the main multi-thread runtime
        // (rusqlite::Connection is Send but !Sync, so &Context is !Send and
        // check_core's future cannot be sent between threads). A current-
        // thread runtime never moves the future across threads.
        let _ = std::thread::Builder::new()
            .name("sshi-op".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                        return;
                    }
                };
                rt.block_on(async move {
                    let ctx = match Context::from_tui_parts(
                        cfg,
                        cfg_path,
                        target_mode,
                        serial,
                        timeout,
                        verbose,
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                            return;
                        }
                    };
                    let sink = EventSender::new(event_tx.clone());
                    let outcome = tokio::select! {
                        res = crate::commands::check::check_core(&ctx, &names, Some(&sink)) => res,
                        _ = cancel_for_task.cancelled() => {
                            let _ = event_tx.send(TuiEvent::OperationCancelled);
                            return;
                        }
                    };
                    match outcome {
                        Ok(report) => {
                            let _ = event_tx.send(TuiEvent::OperationFinished(report));
                        }
                        Err(e) => {
                            let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                        }
                    }
                });
            });

        self.progress_popup_scroll = None;
        self.running_op = Some(RunningOp {
            cancel,
            started_at: std::time::Instant::now(),
            targets,
            host_outcomes: Vec::new(),
            mode: mode_for_op,
            out: out_for_op,
        });
        true
    }

    /// Execute a `run` command against the current target filter.
    fn execute_run(&mut self) -> bool {
        if self.running_op.is_some() {
            self.error = Some("Operation already running".to_string());
            return true;
        }
        let command = self.run_command.value.trim().to_string();
        if command.is_empty() {
            self.error = Some("Command field is empty.".to_string());
            return true;
        }
        let target_mode = build_target_mode(&self.target_filter, &self.config);
        let targets: Vec<String> =
            match resolve_target_names(&target_mode, &self.config, &self.target_filter.skip) {
                Ok(t) if !t.is_empty() => t,
                Ok(_) => {
                    self.error = Some("No hosts matched the current filter.".to_string());
                    return true;
                }
                Err(e) => {
                    self.error = Some(format!("Filter error: {e}"));
                    return true;
                }
            };
        let serial = self.target_filter.serial;
        let timeout = self.last_timeout_secs;
        let mode_for_op = target_mode.clone();
        let out_for_op = self.out_path();
        let cfg = self.config.clone();
        let cfg_path = self.config_path.clone();
        let event_tx = self.event_tx.clone();
        let _auth_sender = self.auth_bridge_tx.clone();
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_for_task = cancel.clone();
        let sudo = self.run_sudo;

        let _ = std::thread::Builder::new()
            .name("sshi-op".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                        return;
                    }
                };
                rt.block_on(async move {
                    let ctx = match Context::from_tui_parts(
                        cfg, cfg_path, target_mode, serial, timeout, false,
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                            return;
                        }
                    };
                    let sink = EventSender::new(event_tx.clone());
                    let outcome = tokio::select! {
                        res = crate::commands::run::run_core(&ctx, &command, sudo, Some(&sink)) => res,
                        _ = cancel_for_task.cancelled() => {
                            let _ = event_tx.send(TuiEvent::OperationCancelled);
                            return;
                        }
                    };
                    match outcome {
                        Ok(report) => {
                            let _ = event_tx.send(TuiEvent::OperationFinished(report));
                        }
                        Err(e) => {
                            let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                        }
                    }
                });
            });

        self.progress_popup_scroll = None;
        self.running_op = Some(RunningOp {
            cancel,
            started_at: std::time::Instant::now(),
            targets,
            host_outcomes: Vec::new(),
            mode: mode_for_op,
            out: out_for_op,
        });
        true
    }

    /// Execute an `exec` (script upload + run) against the current target filter.
    fn execute_exec(&mut self) -> bool {
        if self.running_op.is_some() {
            self.error = Some("Operation already running".to_string());
            return true;
        }
        let script = self.exec_script.value.trim().to_string();
        if script.is_empty() {
            self.error = Some("Script path field is empty.".to_string());
            return true;
        }
        let target_mode = build_target_mode(&self.target_filter, &self.config);
        let targets: Vec<String> =
            match resolve_target_names(&target_mode, &self.config, &self.target_filter.skip) {
                Ok(t) if !t.is_empty() => t,
                Ok(_) => {
                    self.error = Some("No hosts matched the current filter.".to_string());
                    return true;
                }
                Err(e) => {
                    self.error = Some(format!("Filter error: {e}"));
                    return true;
                }
            };
        let serial = self.target_filter.serial;
        let timeout = self.last_timeout_secs;
        let mode_for_op = target_mode.clone();
        let out_for_op = self.out_path();
        let cfg = self.config.clone();
        let cfg_path = self.config_path.clone();
        let event_tx = self.event_tx.clone();
        let _auth_sender = self.auth_bridge_tx.clone();
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_for_task = cancel.clone();
        let sudo = self.exec_sudo;
        let keep = self.exec_keep;

        let _ = std::thread::Builder::new()
            .name("sshi-op".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                        return;
                    }
                };
                rt.block_on(async move {
                    let ctx = match Context::from_tui_parts(
                        cfg, cfg_path, target_mode, serial, timeout, false,
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                            return;
                        }
                    };
                    let sink = EventSender::new(event_tx.clone());
                    let outcome = tokio::select! {
                        res = crate::commands::exec::exec_core(&ctx, &script, sudo, keep, Some(&sink)) => res,
                        _ = cancel_for_task.cancelled() => {
                            let _ = event_tx.send(TuiEvent::OperationCancelled);
                            return;
                        }
                    };
                    match outcome {
                        Ok(report) => {
                            let _ = event_tx.send(TuiEvent::OperationFinished(report));
                        }
                        Err(e) => {
                            let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                        }
                    }
                });
            });

        self.progress_popup_scroll = None;
        self.running_op = Some(RunningOp {
            cancel,
            started_at: std::time::Instant::now(),
            targets,
            host_outcomes: Vec::new(),
            mode: mode_for_op,
            out: out_for_op,
        });
        true
    }

    /// Execute a `cp` operation in a background thread, following the same
    /// pattern as `execute_exec`.
    fn execute_cp(&mut self) -> bool {
        if self.running_op.is_some() {
            self.error = Some("Operation already running".to_string());
            return true;
        }
        let local = self.cp_local.value.trim().to_string();
        if local.is_empty() {
            self.error = Some("Local path field is empty.".to_string());
            return true;
        }
        let remote_raw = self.cp_remote.value.trim().to_string();
        let remote: Option<String> = if remote_raw.is_empty() {
            None
        } else {
            Some(remote_raw)
        };

        let target_mode = build_target_mode(&self.target_filter, &self.config);
        let targets: Vec<String> =
            match resolve_target_names(&target_mode, &self.config, &self.target_filter.skip) {
                Ok(t) if !t.is_empty() => t,
                Ok(_) => {
                    self.error = Some("No hosts matched the current filter.".to_string());
                    return true;
                }
                Err(e) => {
                    self.error = Some(format!("Filter error: {e}"));
                    return true;
                }
            };
        let serial = self.target_filter.serial;
        let timeout = self.last_timeout_secs;
        let mode_for_op = target_mode.clone();
        let out_for_op = self.out_path();
        let cfg = self.config.clone();
        let cfg_path = self.config_path.clone();
        let event_tx = self.event_tx.clone();
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_for_task = cancel.clone();

        let _ = std::thread::Builder::new()
            .name("sshi-op".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                        return;
                    }
                };
                rt.block_on(async move {
                    let ctx = match Context::from_tui_parts(
                        cfg, cfg_path, target_mode, serial, timeout, false,
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                            return;
                        }
                    };
                    let sink = EventSender::new(event_tx.clone());
                    let outcome = tokio::select! {
                        res = crate::commands::cp::cp_core(&ctx, &local, remote.as_deref(), Some(&sink)) => res,
                        _ = cancel_for_task.cancelled() => {
                            let _ = event_tx.send(TuiEvent::OperationCancelled);
                            return;
                        }
                    };
                    match outcome {
                        Ok(report) => {
                            let _ = event_tx.send(TuiEvent::OperationFinished(report));
                        }
                        Err(e) => {
                            let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                        }
                    }
                });
            });

        self.progress_popup_scroll = None;
        self.running_op = Some(RunningOp {
            cancel,
            started_at: std::time::Instant::now(),
            targets,
            host_outcomes: Vec::new(),
            mode: mode_for_op,
            out: out_for_op,
        });
        true
    }

    /// Execute a `sync` operation in a background thread, following the same
    /// pattern as `execute_check`/`execute_run`/`execute_exec`.
    fn execute_sync(&mut self) -> bool {
        if self.running_op.is_some() {
            self.error = Some("An operation is already running.".to_string());
            return true;
        }
        let target_mode = build_target_mode(&self.target_filter, &self.config);
        let targets: Vec<String> =
            match resolve_target_names(&target_mode, &self.config, &self.target_filter.skip) {
                Ok(t) if !t.is_empty() => t,
                Ok(_) => {
                    self.error = Some("No hosts matched the current filter.".to_string());
                    return true;
                }
                Err(e) => {
                    self.error = Some(format!("Filter error: {e}"));
                    return true;
                }
            };
        let serial = self.target_filter.serial;
        let timeout = self.last_timeout_secs;
        let mode_for_op = target_mode.clone();
        let out_for_op = self.out_path();
        let cfg = self.config.clone();
        let cfg_path = self.config_path.clone();
        let event_tx = self.event_tx.clone();
        let _auth_sender = self.auth_bridge_tx.clone();
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_for_task = cancel.clone();
        let dry_run = self.sync_dry_run;
        // Config-entry names and ad-hoc paths are passed together; the sync core
        // merges both (plus the optional source override).
        let adhoc_files = self.sync_adhoc_files.clone();
        let names = comma_names(&self.sync_name.value);
        let source_override: Option<String> = {
            let v = self.sync_source_input.value.trim().to_string();
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        };

        let _ = std::thread::Builder::new()
            .name("sshi-op".to_string())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                        return;
                    }
                };
                rt.block_on(async move {
                    let ctx = match Context::from_tui_parts(
                        cfg, cfg_path, target_mode, serial, timeout, false,
                    ) {
                        Ok(c) => c,
                        Err(e) => {
                            let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                            return;
                        }
                    };
                    let sink = EventSender::new(event_tx.clone());
                    let outcome = tokio::select! {
                        res = crate::commands::sync::sync_core(&ctx, &adhoc_files, &names, dry_run, source_override.as_deref(), Some(&sink)) => res,
                        _ = cancel_for_task.cancelled() => {
                            let _ = event_tx.send(TuiEvent::OperationCancelled);
                            return;
                        }
                    };
                    match outcome {
                        Ok(report) => {
                            let _ = event_tx.send(TuiEvent::OperationFinished(report));
                        }
                        Err(e) => {
                            let _ = event_tx.send(TuiEvent::OperationError(e.to_string()));
                        }
                    }
                });
            });

        self.progress_popup_scroll = None;
        self.running_op = Some(RunningOp {
            cancel,
            started_at: std::time::Instant::now(),
            targets,
            host_outcomes: Vec::new(),
            mode: mode_for_op,
            out: out_for_op,
        });
        true
    }
    fn maybe_reload_checkout(&mut self) {
        if !self.db_stale {
            return;
        }
        match crate::state::db::open(self.config.settings.state_dir.as_deref()) {
            Ok(conn) => {
                let host_names: Vec<&str> =
                    self.config.host.iter().map(|h| h.name.as_str()).collect();
                // Build a temporary minimal Context for fetch_latest_snapshots.
                let tmp_ctx = Context {
                    config: self.config.clone(),
                    config_path: self.config_path.clone(),
                    db: conn,
                    timeout: self.last_timeout_secs,
                    mode: TargetMode::All,
                    serial: false,
                    skip: Vec::new(),
                    verbose: false,
                };
                if let Ok(snaps) = fetch_latest_snapshots(&tmp_ctx, &host_names) {
                    self.checkout_all_snapshots = snaps;
                    self.apply_checkout_filter();
                }
                self.db_stale = false;
            }
            Err(e) => {
                tracing::warn!("Checkout DB reload failed: {e}");
                // Leave db_stale true so the next OperationFinished retries.
            }
        }
    }

    /// Filter `checkout_all_snapshots` by the current `target_filter` host/group
    /// selection and write the result into `checkout_snapshots`.
    fn apply_checkout_filter(&mut self) {
        let target_mode = build_target_mode(&self.target_filter, &self.config);
        let visible: std::collections::HashSet<String> =
            resolve_target_names(&target_mode, &self.config, &self.target_filter.skip)
                .unwrap_or_default()
                .into_iter()
                .collect();
        if visible.is_empty() {
            // No filter active (or filter matches nothing): show all.
            self.checkout_snapshots = self.checkout_all_snapshots.clone();
        } else {
            self.checkout_snapshots = self
                .checkout_all_snapshots
                .iter()
                .filter(|s| visible.contains(&s.host))
                .cloned()
                .collect();
        }
        self.checkout_viewport
            .set_dims(self.checkout_snapshots.len(), 0);
    }

    /// Per-line selectable flags for the List result, or None when every row is
    /// selectable (Checkout/Log). Lets the result cursor skip decorative lines.
    fn list_selectable(&self) -> Option<Vec<bool>> {
        if self.view_op == ViewOperationKind::List {
            self.view_list
                .as_ref()
                .map(super::tabs::view_tab::list_selectable_lines)
        } else {
            None
        }
    }

    /// Move the result-row cursor one step (`down`/up), skipping decorative
    /// List rows. Stops at the first/last selectable row (no wrap). The next
    /// render's `set_dims` scrolls to keep `selected` visible.
    fn result_move(&mut self, down: bool) {
        match self.list_selectable() {
            Some(sel) if sel.iter().any(|&b| b) => {
                let n = sel.len();
                let cur = self.checkout_viewport.selected.min(n - 1);
                let next = if down {
                    (cur + 1..n).find(|&i| sel[i])
                } else {
                    (0..cur).rev().find(|&i| sel[i])
                };
                if let Some(i) = next {
                    self.checkout_viewport.selected = i;
                }
            }
            _ => {
                if down {
                    self.checkout_viewport.move_down();
                } else {
                    self.checkout_viewport.move_up();
                }
            }
        }
    }

    /// Ensure the result cursor sits on a selectable List row (searching `down`
    /// first), used when focus first enters the Result zone. No-op otherwise.
    fn snap_result_selection(&mut self, down: bool) {
        let Some(sel) = self.list_selectable() else {
            return;
        };
        if !sel.iter().any(|&b| b) {
            return;
        }
        let n = sel.len();
        let cur = self.checkout_viewport.selected.min(n - 1);
        if sel[cur] {
            return;
        }
        let fwd = (cur..n).find(|&i| sel[i]);
        let bwd = || (0..cur).rev().find(|&i| sel[i]);
        let next = if down {
            fwd.or_else(bwd)
        } else {
            bwd().or(fwd)
        };
        if let Some(i) = next {
            self.checkout_viewport.selected = i;
        }
    }

    /// Cycle the result cursor one step with wrap-around (Tab/BackTab),
    /// skipping decorative List rows. No-op when the result is empty.
    fn result_cursor_cycle(&mut self, forward: bool) {
        let count = self.checkout_viewport.item_count;
        if count == 0 {
            return;
        }
        match self.list_selectable() {
            Some(sel) if sel.iter().any(|&b| b) => {
                let n = sel.len();
                let cur = self.checkout_viewport.selected.min(n - 1);
                let next = if forward {
                    (cur + 1..n).chain(0..=cur).find(|&i| sel[i])
                } else {
                    (0..cur).rev().chain((cur..n).rev()).find(|&i| sel[i])
                };
                if let Some(i) = next {
                    self.checkout_viewport.selected = i;
                }
            }
            _ => {
                let sel = self.checkout_viewport.selected;
                if forward {
                    if sel + 1 >= count {
                        self.checkout_viewport.home();
                    } else {
                        self.checkout_viewport.move_down();
                    }
                } else if sel == 0 {
                    self.checkout_viewport.end();
                } else {
                    self.checkout_viewport.move_up();
                }
            }
        }
    }

    /// Tab/BackTab on the View tab: cycle peers within the *current layer*.
    /// Layers are OpSelector (cycles the op value, like ←→), Settings (the stops
    /// between OpSelector and Result), and Result (cycles data rows). Arrow keys
    /// cross layers via view_focus_up/down; Tab never leaves the layer.
    fn view_tab_cycle(&mut self, forward: bool) {
        match self.view_focus {
            ViewFocus::Result => self.result_cursor_cycle(forward),
            ViewFocus::OpSelector => self.cycle_view_op(forward),
            // The target row cycles its mode in place (matching ←→); ↑↓ still
            // steps to the other Settings fields (members / skip).
            ViewFocus::TargetMode | ViewFocus::TargetMembers => {
                self.view_cycle_target_mode(forward)
            }
            _ => {
                // Settings layer: cycle the stops excluding OpSelector/Result.
                let settings: Vec<ViewFocus> =
                    ViewFocus::stops(self.view_op, self.target_filter.mode)
                        .into_iter()
                        .filter(|s| !matches!(s, ViewFocus::OpSelector | ViewFocus::Result))
                        .collect();
                if settings.len() < 2 {
                    return;
                }
                let pos = settings
                    .iter()
                    .position(|s| *s == self.view_focus)
                    .unwrap_or(0);
                let next = if forward {
                    (pos + 1) % settings.len()
                } else {
                    (pos + settings.len() - 1) % settings.len()
                };
                self.view_focus = settings[next];
            }
        }
    }

    /// True when no selectable row lies above the current List cursor (or, for
    /// Checkout/Log, when the cursor is at row 0).
    fn result_at_top(&self) -> bool {
        match self.list_selectable() {
            Some(sel) => {
                let cur = self
                    .checkout_viewport
                    .selected
                    .min(sel.len().saturating_sub(1));
                !(0..cur).any(|i| sel[i])
            }
            None => self.checkout_viewport.selected == 0,
        }
    }

    fn view_focus_up(&mut self) {
        if self.view_focus == ViewFocus::Result {
            if self.result_at_top() && self.checkout_viewport.scroll_y == 0 {
                // At top of result — move focus to previous stop.
                let stops = ViewFocus::stops(self.view_op, self.target_filter.mode);
                let idx = stops
                    .iter()
                    .position(|s| *s == self.view_focus)
                    .unwrap_or(0);
                if idx > 0 {
                    self.view_focus = stops[idx - 1];
                }
            } else {
                self.result_move(false);
            }
        } else {
            let stops = ViewFocus::stops(self.view_op, self.target_filter.mode);
            let idx = stops
                .iter()
                .position(|s| *s == self.view_focus)
                .unwrap_or(0);
            if idx > 0 {
                self.view_focus = stops[idx - 1];
            } else {
                // At first stop (OpSelector) — escape to navbar.
                self.navbar_focused = true;
            }
        }
    }

    fn view_focus_down(&mut self) {
        if self.view_focus == ViewFocus::Result {
            self.result_move(true);
        } else {
            let stops = ViewFocus::stops(self.view_op, self.target_filter.mode);
            let idx = stops
                .iter()
                .position(|s| *s == self.view_focus)
                .unwrap_or(0);
            if idx + 1 < stops.len() {
                self.view_focus = stops[idx + 1];
                // Entering the Result zone: land the cursor on a real data row.
                if self.view_focus == ViewFocus::Result {
                    self.snap_result_selection(true);
                }
            }
        }
    }

    /// Refresh the View tab result data for the active `view_op`.
    fn refresh_view(&mut self) {
        // Clear the dirty flag only on a successful read; a failed db::open
        // leaves it set so the next render retries instead of showing stale/empty.
        match self.view_op {
            super::state::persist::ViewOperationKind::Checkout => {
                self.maybe_reload_checkout();
                self.apply_checkout_filter();
                self.view_dirty = false;
            }
            super::state::persist::ViewOperationKind::List => {
                if let Ok(conn) = crate::state::db::open(self.config.settings.state_dir.as_deref())
                {
                    // List honors the target filter (the `f` popup applies to it).
                    let ctx = Context {
                        config: self.config.clone(),
                        config_path: self.config_path.clone(),
                        db: conn,
                        timeout: self.last_timeout_secs,
                        mode: build_target_mode(&self.target_filter, &self.config),
                        serial: false,
                        skip: self.target_filter.skip.clone(),
                        verbose: false,
                    };
                    self.view_list =
                        Some(crate::commands::list::list_core(&ctx).unwrap_or_default());
                    self.view_dirty = false;
                }
            }
            super::state::persist::ViewOperationKind::Log => {
                if let Ok(conn) = crate::state::db::open(self.config.settings.state_dir.as_deref())
                {
                    let ctx = Context {
                        config: self.config.clone(),
                        config_path: self.config_path.clone(),
                        db: conn,
                        timeout: self.last_timeout_secs,
                        mode: TargetMode::All,
                        serial: false,
                        skip: Vec::new(),
                        verbose: false,
                    };
                    let since = {
                        let v = self.log_since_input.value.trim().to_string();
                        if v.is_empty() {
                            None
                        } else {
                            Some(v)
                        }
                    };
                    let host = {
                        let v = self.log_host_input.value.trim().to_string();
                        if v.is_empty() {
                            None
                        } else {
                            Some(v)
                        }
                    };
                    self.view_log = crate::commands::log::log_core(
                        &ctx,
                        self.log_last,
                        since,
                        host,
                        self.log_action.clone(),
                        self.log_errors,
                    )
                    .unwrap_or_default();
                    self.view_dirty = false;
                }
            }
        }
    }

    /// Returns true if the event mutated state (frame should redraw).
    fn handle_event(&mut self, ev: Event) -> Result<bool> {
        match ev {
            Event::Resize(_, _) => Ok(true),
            Event::Key(key) if key.kind == KeyEventKind::Press => self.handle_key(key),
            _ => Ok(false),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        // Ctrl+C always quits.
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            self.should_quit = true;
            return Ok(true);
        }

        // Auth popup takes highest priority after Ctrl+C.
        if let Some(popup) = self.auth_popup.as_mut() {
            match key.code {
                KeyCode::Enter => {
                    popup.submit();
                    self.auth_popup = None;
                }
                KeyCode::Esc => {
                    popup.cancel();
                    self.auth_popup = None;
                }
                _ => {
                    popup.input.handle_key(key);
                }
            }
            return Ok(true);
        }

        // Export popup routing.
        if let Some(popup) = self.export_popup.as_mut() {
            match key.code {
                KeyCode::Enter => {
                    popup.input.confirm();
                    let path = popup.input.value.clone();
                    self.export_popup = None;
                    if let Err(e) = self.execute_view_export(&path) {
                        self.error = Some(format!("Export failed: {}", e));
                    }
                }
                KeyCode::Esc => {
                    popup.input.cancel();
                    self.export_popup = None;
                }
                _ => {
                    popup.input.handle_key(key);
                }
            }
            return Ok(true);
        }

        // §14.3: while an input field is active, suspend ALL other routing.
        if self.active_tab == TabId::Operate {
            let active_field: Option<&mut InputField> = match self.operate_focus {
                OpField::Command if self.run_command.mode == InputMode::Active => {
                    Some(&mut self.run_command)
                }
                OpField::Script if self.exec_script.mode == InputMode::Active => {
                    Some(&mut self.exec_script)
                }
                OpField::SyncAdhocInput if self.sync_adhoc_input.mode == InputMode::Active => {
                    Some(&mut self.sync_adhoc_input)
                }
                OpField::SyncSource if self.sync_source_input.mode == InputMode::Active => {
                    Some(&mut self.sync_source_input)
                }
                OpField::CpLocal if self.cp_local.mode == InputMode::Active => {
                    Some(&mut self.cp_local)
                }
                OpField::CpRemote if self.cp_remote.mode == InputMode::Active => {
                    Some(&mut self.cp_remote)
                }
                OpField::CheckName if self.check_name.mode == InputMode::Active => {
                    Some(&mut self.check_name)
                }
                OpField::SyncName if self.sync_name.mode == InputMode::Active => {
                    Some(&mut self.sync_name)
                }
                OpField::Out if self.out_input.mode == InputMode::Active => {
                    Some(&mut self.out_input)
                }
                _ => None,
            };
            if let Some(field) = active_field {
                let changed = field.handle_key(key);
                // If sync adhoc input just committed (Enter → mode Normal), add path to list.
                if self.operate_operation == OperationKind::Sync
                    && self.sync_adhoc_input.mode == InputMode::Normal
                    && !self.sync_adhoc_input.value.is_empty()
                {
                    let path = std::mem::take(&mut self.sync_adhoc_input.value);
                    self.sync_adhoc_files.push(path);
                }
                return Ok(changed);
            }
        }

        // §14.3: while a View text input is active, suspend ALL other routing.
        if self.active_tab == TabId::View && self.view_op == ViewOperationKind::Log {
            let active_view_field: Option<&mut InputField> = match self.view_focus {
                ViewFocus::Specific(0) if self.log_last_input.mode == InputMode::Active => {
                    Some(&mut self.log_last_input)
                }
                ViewFocus::Specific(3) if self.log_since_input.mode == InputMode::Active => {
                    Some(&mut self.log_since_input)
                }
                ViewFocus::Specific(4) if self.log_host_input.mode == InputMode::Active => {
                    Some(&mut self.log_host_input)
                }
                _ => None,
            };
            // Route the key to the active field, then end its borrow before
            // touching other `self` fields (commit handling needs `self`).
            let committed = if let Some(field) = active_view_field {
                field.handle_key(key);
                Some(field.mode == InputMode::Normal)
            } else {
                None
            };
            if let Some(committed) = committed {
                // Only Enter commits; Esc cancels (value already restored) and
                // must not trigger a redundant refresh.
                if committed && key.code == KeyCode::Enter {
                    if matches!(self.view_focus, ViewFocus::Specific(0)) {
                        let trimmed = self.log_last_input.value.trim();
                        if trimmed.is_empty() {
                            // Empty input means "no limit" → 0 (all entries).
                            self.log_last = 0;
                            self.log_last_input.value = "0".to_string();
                        } else {
                            match trimmed.parse::<usize>() {
                                Ok(v) => self.log_last = v,
                                // Non-numeric input — revert to the active value.
                                Err(_) => self.log_last_input.value = self.log_last.to_string(),
                            }
                        }
                    }
                    self.view_dirty = true;
                }
                return Ok(true);
            }
        }

        // Member picker (Operate target groups/hosts/skip/shell) is a focus
        // root: while open it consumes all keys until applied or cancelled.
        if let Some(picker) = self.member_picker.as_mut() {
            match picker.handle_key(key) {
                PickerResult::Continue => return Ok(true),
                PickerResult::Cancelled => {
                    self.member_picker = None;
                    return Ok(true);
                }
                PickerResult::Add => {
                    // Name picker → jump to the Config add-entry form; remember
                    // to reopen this picker once the entry is committed.
                    let target = self.member_picker.take().unwrap().target;
                    self.open_add_entry_from_picker(target);
                    return Ok(true);
                }
                PickerResult::Applied => {
                    let picker = self.member_picker.take().unwrap();
                    let chosen = picker.chosen();
                    match picker.target {
                        PickerTarget::Groups => self.target_filter.groups = chosen,
                        PickerTarget::Hosts => self.target_filter.hosts = chosen,
                        PickerTarget::Skip => self.target_filter.skip = chosen,
                        PickerTarget::Shell => {
                            if let Some(name) = chosen.first() {
                                self.target_filter.shell = match name.as_str() {
                                    "powershell" => super::state::persist::ShellMode::PowerShell,
                                    "cmd" => super::state::persist::ShellMode::Cmd,
                                    _ => super::state::persist::ShellMode::Sh,
                                };
                            }
                        }
                        // Name pickers write the comma-separated value back into
                        // the field the executor already reads; no target-filter
                        // post-processing applies.
                        PickerTarget::CheckNames => {
                            self.check_name.value = chosen.join(", ");
                            return Ok(true);
                        }
                        PickerTarget::SyncNames => {
                            self.sync_name.value = chosen.join(", ");
                            return Ok(true);
                        }
                        // Single source host; "(none)" or nothing clears it.
                        PickerTarget::SyncSource => {
                            self.sync_source_input.value = match chosen.first() {
                                Some(h) if h != "(none)" => h.clone(),
                                _ => String::new(),
                            };
                            return Ok(true);
                        }
                    }
                    // Deliberately no validate_filter() here: it would force an
                    // emptied Groups/Hosts selection back to All. The picker only
                    // offers valid options, so there is nothing to sanitise.
                    self.save_state();
                    self.apply_checkout_filter();
                    self.view_dirty = true;
                    return Ok(true);
                }
            }
        }

        // Help popup intercepts: only Esc/? close it.
        if self.help_open {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') => {
                    self.help_open = false;
                    return Ok(true);
                }
                _ => return Ok(false),
            }
        }

        // Info popup intercepts: only Esc/i close it.
        if self.info_open {
            match key.code {
                KeyCode::Esc | KeyCode::Char('i') => {
                    self.info_open = false;
                    return Ok(true);
                }
                _ => return Ok(false),
            }
        }

        // Log overlay intercepts: Esc/L close it, scroll inside.
        if self.log_overlay_open {
            return self.handle_log_overlay_key(key);
        }

        // Completed report popup: Esc / Enter dismisses it.
        if self.completed_report.is_some() {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.completed_report = None;
                    return Ok(true);
                }
                _ => return Ok(false),
            }
        }

        // Running operation: Esc cancels (cooperatively).
        if let Some(op) = self.running_op.as_ref() {
            if key.code == KeyCode::Esc {
                op.cancel.cancel();
                return Ok(true);
            }
            // While running, ignore most keys except 1/2/3 tab switches, Ctrl+C,
            // and Up/Down which scroll the progress popup.
            match key.code {
                KeyCode::Char('1') => {
                    self.goto_tab(TabId::Config);
                    return Ok(true);
                }
                KeyCode::Char('2') => {
                    self.goto_tab(TabId::Operate);
                    return Ok(true);
                }
                KeyCode::Char('3') => {
                    self.goto_tab(TabId::View);
                    return Ok(true);
                }
                KeyCode::Tab => {
                    self.navbar_focused = false;
                    self.goto_tab(self.active_tab.next());
                    return Ok(true);
                }
                KeyCode::BackTab => {
                    self.navbar_focused = false;
                    self.goto_tab(self.active_tab.prev());
                    return Ok(true);
                }
                KeyCode::Up | KeyCode::Char('k') if self.active_tab == TabId::Operate => {
                    // Enable manual scroll: lock to current position if auto-scrolling.
                    let outcomes_len = self
                        .running_op
                        .as_ref()
                        .map_or(0, |o| o.host_outcomes.len());
                    let current = self
                        .progress_popup_scroll
                        .unwrap_or(outcomes_len.saturating_sub(12));
                    self.progress_popup_scroll = Some(current.saturating_sub(1));
                    return Ok(true);
                }
                KeyCode::Down | KeyCode::Char('j') if self.active_tab == TabId::Operate => {
                    let outcomes_len = self
                        .running_op
                        .as_ref()
                        .map_or(0, |o| o.host_outcomes.len());
                    let current = self
                        .progress_popup_scroll
                        .unwrap_or(outcomes_len.saturating_sub(12));
                    let max_start = outcomes_len.saturating_sub(12);
                    let next = (current + 1).min(max_start);
                    // If scrolled to the auto-scroll position, clear manual scroll.
                    if next >= max_start {
                        self.progress_popup_scroll = None;
                    } else {
                        self.progress_popup_scroll = Some(next);
                    }
                    return Ok(true);
                }
                KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End
                    if self.active_tab == TabId::Operate =>
                {
                    // Page size matches the 12-row progress window.
                    let outcomes_len = self
                        .running_op
                        .as_ref()
                        .map_or(0, |o| o.host_outcomes.len());
                    let max_start = outcomes_len.saturating_sub(12);
                    let current = self.progress_popup_scroll.unwrap_or(max_start);
                    match key.code {
                        KeyCode::PageUp => {
                            self.progress_popup_scroll = Some(current.saturating_sub(12));
                        }
                        KeyCode::PageDown => {
                            let next = (current + 12).min(max_start);
                            if next >= max_start {
                                self.progress_popup_scroll = None;
                            } else {
                                self.progress_popup_scroll = Some(next);
                            }
                        }
                        KeyCode::Home => self.progress_popup_scroll = Some(0),
                        // End resumes auto-scroll to the latest output.
                        KeyCode::End => self.progress_popup_scroll = None,
                        _ => {}
                    }
                    return Ok(true);
                }
                _ => return Ok(false),
            }
        }

        // §popup-guard: while any config popup is open, suspend all global shortcuts.
        if self.active_tab == TabId::Config && self.config_tab.is_any_popup_open() {
            let handled = self.config_tab.handle_key(key, &mut self.config);
            self.after_config_key();
            return Ok(handled);
        }

        // NavBar focus: intercept keys when tab bar has focus.
        if self.navbar_focused {
            match key.code {
                KeyCode::Left | KeyCode::Char('h') => {
                    self.active_tab = self.active_tab.prev();
                    return Ok(true);
                }
                KeyCode::Right | KeyCode::Char('l') => {
                    self.active_tab = self.active_tab.next();
                    return Ok(true);
                }
                KeyCode::Tab => {
                    self.active_tab = self.active_tab.next();
                    return Ok(true);
                }
                KeyCode::BackTab => {
                    self.active_tab = self.active_tab.prev();
                    return Ok(true);
                }
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Enter => {
                    self.navbar_focused = false;
                    return Ok(true);
                }
                KeyCode::Esc => {
                    self.navbar_focused = false;
                    return Ok(true);
                }
                KeyCode::Char('1') => {
                    self.goto_tab(TabId::Config);
                    self.navbar_focused = false;
                    return Ok(true);
                }
                KeyCode::Char('2') => {
                    self.goto_tab(TabId::Operate);
                    self.navbar_focused = false;
                    return Ok(true);
                }
                KeyCode::Char('3') => {
                    self.goto_tab(TabId::View);
                    self.navbar_focused = false;
                    return Ok(true);
                }
                KeyCode::Char('q') => {
                    self.should_quit = true;
                    return Ok(true);
                }
                _ => return Ok(false),
            }
        }

        match key.code {
            // ── Global keys (always first; work from any tab) ──────────────
            KeyCode::Char('q') => {
                self.should_quit = true;
                Ok(true)
            }
            KeyCode::Char('?') => {
                self.help_open = true;
                Ok(true)
            }
            KeyCode::Char('i') => {
                self.info_open = true;
                Ok(true)
            }
            KeyCode::Char('L') => {
                self.log_overlay_open = !self.log_overlay_open;
                if self.log_overlay_open {
                    self.log_overlay_vp = Viewport::new();
                    let len = self
                        .log_buffer
                        .as_ref()
                        .map_or(0, |b: &LogBufferHandle| b.len());
                    self.log_overlay_vp.set_dims(len, 0);
                }
                Ok(true)
            }
            KeyCode::Esc => {
                if self.error.is_some() {
                    self.error = None;
                    return Ok(true);
                }
                // Any other position: Esc jumps focus to NavBar.
                if !self.navbar_focused {
                    self.navbar_focused = true;
                    return Ok(true);
                }
                Ok(false)
            }
            KeyCode::Char('1') => {
                if self.active_tab == TabId::Config && self.config_tab.config_dirty {
                    self.error = Some(
                        "Config save failed — fix the error before switching tabs.".to_string(),
                    );
                    return Ok(true);
                }
                self.goto_tab(TabId::Config);
                Ok(true)
            }
            KeyCode::Char('2') => {
                if self.active_tab == TabId::Config && self.config_tab.config_dirty {
                    self.error = Some(
                        "Config save failed — fix the error before switching tabs.".to_string(),
                    );
                    return Ok(true);
                }
                self.goto_tab(TabId::Operate);
                Ok(true)
            }
            KeyCode::Char('3') => {
                if self.active_tab == TabId::Config && self.config_tab.config_dirty {
                    self.error = Some(
                        "Config save failed — fix the error before switching tabs.".to_string(),
                    );
                    return Ok(true);
                }
                self.goto_tab(TabId::View);
                Ok(true)
            }
            // Tab/BackTab: context-aware cycling within the focused layer.
            KeyCode::Tab | KeyCode::BackTab => {
                let forward = key.code == KeyCode::Tab;
                if self.navbar_focused {
                    self.goto_tab(if forward {
                        self.active_tab.next()
                    } else {
                        self.active_tab.prev()
                    });
                } else {
                    match self.active_tab {
                        TabId::Config => match self.config_tab.zone {
                            ConfigZone::Sidebar => {
                                let count = self.config_tab.items.len();
                                if count > 0 {
                                    if forward {
                                        let sel = self.config_tab.sidebar_vp.selected;
                                        if sel + 1 >= count {
                                            self.config_tab.sidebar_vp.home();
                                        } else {
                                            self.config_tab.sidebar_vp.move_down();
                                        }
                                    } else {
                                        let sel = self.config_tab.sidebar_vp.selected;
                                        if sel == 0 {
                                            self.config_tab.sidebar_vp.end();
                                        } else {
                                            self.config_tab.sidebar_vp.move_up();
                                        }
                                    }
                                    self.config_tab.reset_field_vp(&self.config);
                                }
                            }
                            ConfigZone::FieldTable => {
                                let count = self.config_tab.current_descriptors(&self.config).len();
                                if count > 0 {
                                    if forward {
                                        let sel = self.config_tab.field_vp.selected;
                                        if sel + 1 >= count {
                                            self.config_tab.field_vp.home();
                                        } else {
                                            self.config_tab.field_vp.move_down();
                                        }
                                    } else {
                                        let sel = self.config_tab.field_vp.selected;
                                        if sel == 0 {
                                            self.config_tab.field_vp.set_dims(
                                                count,
                                                self.config_tab.field_vp.visible_height,
                                            );
                                            self.config_tab.field_vp.end();
                                        } else {
                                            self.config_tab.field_vp.move_up();
                                        }
                                    }
                                }
                            }
                        },
                        TabId::Operate => {
                            // Tab/BackTab cycle peers within the current layer;
                            // arrows cross layer boundaries.
                            self.operate_tab_cycle(forward);
                        }
                        TabId::View => {
                            // Tab/BackTab cycle peers within the current layer
                            // (OpSelector / Settings / Result); arrows cross
                            // layers via view_focus_up/down.
                            self.view_tab_cycle(forward);
                        }
                    }
                }
                Ok(true)
            }

            // ── Config tab (§8.6, §12.2, Phase 4+7) ───────────────────────────
            // E opens external editor (§7.4 4-stage flow).
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
            // 'a' adds a new entry (Phase 7 Case B).
            KeyCode::Char('a')
                if self.active_tab == TabId::Config
                    && self.config_tab.entry_form.is_none()
                    && self.config_tab.confirm.is_none() =>
            {
                let kind = self.config_add_kind();
                if let Some(kind) = kind {
                    self.config_tab.start_add_entry(kind);
                }
                Ok(true)
            }
            // 'd' deletes focused entry (Phase 7).
            KeyCode::Char('d')
                if self.active_tab == TabId::Config
                    && self.config_tab.entry_form.is_none()
                    && self.config_tab.confirm.is_none() =>
            {
                self.config_tab.request_delete();
                Ok(true)
            }
            // Up at top of Config Sidebar escapes to NavBar.
            KeyCode::Up | KeyCode::Char('k')
                if self.active_tab == TabId::Config
                    && self.config_tab.zone == ConfigZone::Sidebar
                    && self.config_tab.sidebar_vp.selected == 0 =>
            {
                self.navbar_focused = true;
                Ok(true)
            }
            // Up at top of Config FieldTable also escapes to NavBar.
            KeyCode::Up | KeyCode::Char('k')
                if self.active_tab == TabId::Config
                    && self.config_tab.zone == ConfigZone::FieldTable
                    && self.config_tab.field_vp_at_top() =>
            {
                self.navbar_focused = true;
                Ok(true)
            }
            // All other Config tab keys routed to ConfigTabState (including 'e'/Enter for inline edit).
            _ if self.active_tab == TabId::Config => {
                let handled = self.config_tab.handle_key(key, &mut self.config);
                self.after_config_key();
                Ok(handled)
            }

            // ── Operate tab (unified linear field walk) ─────────────────────
            KeyCode::Up | KeyCode::Char('k') if self.active_tab == TabId::Operate => {
                self.operate_move_focus(-1);
                Ok(true)
            }
            KeyCode::Down | KeyCode::Char('j') if self.active_tab == TabId::Operate => {
                self.operate_move_focus(1);
                Ok(true)
            }
            // ←→ change the value of the focused field.
            KeyCode::Left | KeyCode::Right if self.active_tab == TabId::Operate => {
                let right = key.code == KeyCode::Right;
                match self.operate_focus {
                    OpField::OpRadio => self.operate_cycle_operation(right),
                    // The mode row and its value row are linked: ←→ cycles the
                    // target mode from either.
                    OpField::TargetMode | OpField::TargetMembers => {
                        self.operate_cycle_target_mode(right)
                    }
                    OpField::Timeout => {
                        let cur = self.target_filter.timeout;
                        self.target_filter.timeout = if right {
                            cur.saturating_add(5)
                        } else {
                            cur.saturating_sub(5).max(1)
                        };
                        // Keep the value used at execution time in sync with the
                        // edited field (last_timeout_secs feeds execute_*).
                        self.last_timeout_secs = self.target_filter.timeout;
                        self.save_state();
                    }
                    _ => {}
                }
                Ok(true)
            }
            // Enter activates / opens the focused field.
            KeyCode::Enter if self.active_tab == TabId::Operate => {
                match self.operate_focus {
                    // Enter on the mode row or the value row opens the picker.
                    OpField::TargetMode | OpField::TargetMembers => self.open_member_picker(),
                    OpField::Skip => {
                        let hosts = self.available_hosts();
                        let accent = self.tab_accent();
                        self.member_picker = Some(MemberPicker::new(
                            PickerTarget::Skip,
                            hosts,
                            &self.target_filter.skip,
                            accent,
                        ));
                    }
                    OpField::Command => self.run_command.activate(),
                    OpField::Script => self.exec_script.activate(),
                    OpField::SyncAdhocInput => self.sync_adhoc_input.activate(),
                    OpField::SyncSource => self.open_source_picker(),
                    OpField::CpLocal => self.cp_local.activate(),
                    OpField::CpRemote => self.cp_remote.activate(),
                    OpField::CheckName => self.open_name_picker(PickerTarget::CheckNames),
                    OpField::SyncName => self.open_name_picker(PickerTarget::SyncNames),
                    OpField::Out => self.out_input.activate(),
                    OpField::Execute => return Ok(self.trigger_execute()),
                    _ => {}
                }
                Ok(true)
            }
            // Space toggles the focused boolean.
            KeyCode::Char(' ') if self.active_tab == TabId::Operate => {
                match self.operate_focus {
                    OpField::Serial => {
                        self.target_filter.serial = !self.target_filter.serial;
                        self.save_state();
                    }
                    OpField::Sudo => {
                        match self.operate_operation {
                            OperationKind::Run => self.run_sudo = !self.run_sudo,
                            OperationKind::Exec => self.exec_sudo = !self.exec_sudo,
                            _ => {}
                        }
                        self.save_state();
                    }
                    OpField::Keep => {
                        self.exec_keep = !self.exec_keep;
                        self.save_state();
                    }
                    OpField::DryRun => {
                        self.op_dry_run = !self.op_dry_run;
                    }
                    // In Shell mode the members field is a single fixed choice,
                    // so Space cycles it inline (from either the mode or value row).
                    OpField::TargetMode | OpField::TargetMembers
                        if self.target_filter.mode == TargetFilterMode::Shell =>
                    {
                        use super::state::persist::ShellMode;
                        self.target_filter.shell = match self.target_filter.shell {
                            ShellMode::Sh => ShellMode::PowerShell,
                            ShellMode::PowerShell => ShellMode::Cmd,
                            ShellMode::Cmd => ShellMode::Sh,
                        };
                        self.save_state();
                    }
                    // Sync source cycles in place (mirrors Shell); Enter still
                    // opens the full picker.
                    OpField::SyncSource => self.cycle_sync_source(),
                    _ => {}
                }
                Ok(true)
            }
            // 'e' executes the current operation from anywhere in the Operate
            // tab (shortcut for focusing Execute + Enter).
            KeyCode::Char('e') if self.active_tab == TabId::Operate => Ok(self.trigger_execute()),
            // 'd' toggles the shared dry-run flag (shown by the Execute button).
            KeyCode::Char('d') if self.active_tab == TabId::Operate => {
                self.op_dry_run = !self.op_dry_run;
                Ok(true)
            }
            // 's' toggles serial execution from anywhere in the Operate tab.
            KeyCode::Char('s') if self.active_tab == TabId::Operate => {
                self.target_filter.serial = !self.target_filter.serial;
                self.save_state();
                Ok(true)
            }
            // Del quick-clears the focused optional field (chip lists, optional
            // text inputs, single-select source). Required fields (Command,
            // Script, CpLocal) are intentionally absent so Del can't blank them.
            // For the ad-hoc list Del removes the last path (incremental).
            KeyCode::Delete if self.active_tab == TabId::Operate => {
                match self.operate_focus {
                    OpField::TargetMembers | OpField::TargetMode => {
                        match self.target_filter.mode {
                            TargetFilterMode::Groups => self.target_filter.groups.clear(),
                            TargetFilterMode::Hosts => self.target_filter.hosts.clear(),
                            // Shell is a fixed single value; All has no members.
                            _ => {}
                        }
                        self.save_state();
                        self.apply_checkout_filter();
                        self.view_dirty = true;
                    }
                    OpField::Skip => {
                        self.target_filter.skip.clear();
                        self.save_state();
                        self.apply_checkout_filter();
                        self.view_dirty = true;
                    }
                    OpField::CheckName => self.check_name.value.clear(),
                    OpField::SyncName => self.sync_name.value.clear(),
                    OpField::SyncSource => self.sync_source_input.value.clear(),
                    OpField::CpRemote => self.cp_remote.value.clear(),
                    OpField::Out => self.out_input.value.clear(),
                    OpField::SyncAdhocInput => {
                        self.sync_adhoc_files.pop();
                    }
                    _ => {}
                }
                Ok(true)
            }

            // ── View tab ───────────────────────────────────────────────
            KeyCode::Left if self.active_tab == TabId::View => {
                if self.view_focus == ViewFocus::OpSelector {
                    self.cycle_view_op(false);
                } else if matches!(
                    self.view_focus,
                    ViewFocus::TargetMode | ViewFocus::TargetMembers
                ) {
                    self.view_cycle_target_mode(false);
                }
                Ok(true)
            }
            KeyCode::Right if self.active_tab == TabId::View => {
                if self.view_focus == ViewFocus::OpSelector {
                    self.cycle_view_op(true);
                } else if matches!(
                    self.view_focus,
                    ViewFocus::TargetMode | ViewFocus::TargetMembers
                ) {
                    self.view_cycle_target_mode(true);
                }
                Ok(true)
            }
            // Enter activates inputs / opens the inline target pickers.
            KeyCode::Enter if self.active_tab == TabId::View => {
                match self.view_focus {
                    // Enter on the mode row or the value row opens the picker.
                    ViewFocus::TargetMode | ViewFocus::TargetMembers => self.open_member_picker(),
                    ViewFocus::Skip => {
                        let hosts = self.available_hosts();
                        let accent = self.tab_accent();
                        self.member_picker = Some(MemberPicker::new(
                            PickerTarget::Skip,
                            hosts,
                            &self.target_filter.skip,
                            accent,
                        ));
                    }
                    ViewFocus::Specific(0) => {
                        self.log_last_input.activate();
                    }
                    ViewFocus::Specific(3) => {
                        self.log_since_input.activate();
                    }
                    ViewFocus::Specific(4) => {
                        self.log_host_input.activate();
                    }
                    _ => {}
                }
                Ok(true)
            }
            // Space toggles the Log errors checkbox, cycles the action filter,
            // or cycles the shell value.
            KeyCode::Char(' ') if self.active_tab == TabId::View => {
                if self.view_focus == ViewFocus::Specific(1) {
                    self.log_errors = !self.log_errors;
                    self.view_dirty = true;
                } else if self.view_focus == ViewFocus::Specific(2) {
                    // action enum: cycle forward (None → check → run → exec → sync → None)
                    self.log_action = match self.log_action {
                        None => Some(ActionFilter::Check),
                        Some(ActionFilter::Check) => Some(ActionFilter::Run),
                        Some(ActionFilter::Run) => Some(ActionFilter::Exec),
                        Some(ActionFilter::Exec) => Some(ActionFilter::Sync),
                        Some(ActionFilter::Sync) => None,
                    };
                    self.view_dirty = true;
                } else if matches!(
                    self.view_focus,
                    ViewFocus::TargetMode | ViewFocus::TargetMembers
                ) && self.target_filter.mode == TargetFilterMode::Shell
                {
                    use super::state::persist::ShellMode;
                    self.target_filter.shell = match self.target_filter.shell {
                        ShellMode::Sh => ShellMode::PowerShell,
                        ShellMode::PowerShell => ShellMode::Cmd,
                        ShellMode::Cmd => ShellMode::Sh,
                    };
                    self.save_state();
                    self.apply_checkout_filter();
                    self.view_dirty = true;
                }
                Ok(true)
            }
            KeyCode::Up | KeyCode::Char('k') if self.active_tab == TabId::View => {
                self.view_focus_up();
                Ok(true)
            }
            KeyCode::Down | KeyCode::Char('j') if self.active_tab == TabId::View => {
                self.view_focus_down();
                Ok(true)
            }
            KeyCode::PageUp if self.active_tab == TabId::View => {
                self.checkout_viewport.page_up();
                Ok(true)
            }
            KeyCode::PageDown if self.active_tab == TabId::View => {
                self.checkout_viewport.page_down();
                Ok(true)
            }
            KeyCode::Home if self.active_tab == TabId::View => {
                self.checkout_viewport.home();
                Ok(true)
            }
            KeyCode::End if self.active_tab == TabId::View => {
                self.checkout_viewport.end();
                Ok(true)
            }

            KeyCode::Char('o') if self.active_tab == TabId::View => {
                let has_data = match self.view_op {
                    ViewOperationKind::Checkout => !self.checkout_snapshots.is_empty(),
                    ViewOperationKind::Log => !self.view_log.is_empty(),
                    ViewOperationKind::List => self.view_list.is_some(),
                };
                if has_data {
                    self.export_popup = Some(ExportPopup::new(self.view_op));
                } else {
                    self.error = Some("No data to export".to_string());
                }
                Ok(true)
            }

            _ => Ok(false),
        }
    }

    fn render(&mut self, area: Rect, frame: &mut ratatui::Frame) {
        // Terminal-size guard (§7.8): below threshold, render only the warning.
        if area.width < MIN_COLS || area.height < MIN_ROWS {
            let msg = format!(
                "Terminal too small (need {}×{}+; have {}×{})\n\nResize the terminal to continue.",
                MIN_COLS, MIN_ROWS, area.width, area.height
            );
            let p = Paragraph::new(msg)
                .style(Style::default().fg(self.theme.error))
                .wrap(Wrap { trim: false });
            frame.render_widget(p, area);
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(2),
            ])
            .split(area);

        self.render_tab_bar(chunks[0], frame);
        match self.active_tab {
            TabId::Config => self.render_config(chunks[1], frame),
            TabId::Operate => self.render_operate(chunks[1], frame),
            TabId::View => {
                if self.view_dirty {
                    self.refresh_view();
                }
                // Chrome rows: " View " block border (2) + op selector (2) +
                // Results block border (2) = 6 base.
                // Checkout/List add the inline Common zone (2 rows, +1 when a
                // Members row is shown for Groups/Hosts); Checkout adds 1 more
                // for the table header. Log adds the 1-row summary + 5 specific.
                let common_zone = if self.target_filter.mode != TargetFilterMode::All {
                    3
                } else {
                    2
                };
                let chrome = match self.view_op {
                    ViewOperationKind::Checkout => 6 + common_zone + 1,
                    ViewOperationKind::List => 6 + common_zone,
                    ViewOperationKind::Log => 12, // 6 base + 1 summary + 5 specific
                };
                let view_h = chunks[1].height.saturating_sub(chrome as u16) as usize;
                let row_count = match self.view_op {
                    ViewOperationKind::Checkout => self.checkout_snapshots.len(),
                    ViewOperationKind::List => self
                        .view_list
                        .as_ref()
                        .map(super::tabs::view_tab::list_line_count)
                        .unwrap_or(0),
                    ViewOperationKind::Log => self.view_log.len(),
                };
                self.checkout_viewport.set_dims(row_count, view_h);
                let checkout = if matches!(self.view_op, ViewOperationKind::Checkout) {
                    Some((&*self.checkout_snapshots, &self.checkout_columns))
                } else {
                    None
                };
                let list = if matches!(self.view_op, ViewOperationKind::List) {
                    self.view_list.as_ref()
                } else {
                    None
                };
                let log = if matches!(self.view_op, ViewOperationKind::Log) {
                    Some(&*self.view_log)
                } else {
                    None
                };
                let specific_focused = match self.view_focus {
                    ViewFocus::Specific(i) if self.view_op == ViewOperationKind::Log => Some(i),
                    _ => None,
                };
                let active = !self.navbar_focused;
                let op_selector_focused = active && self.view_focus == ViewFocus::OpSelector;
                let result_focused = active && self.view_focus == ViewFocus::Result;
                let target_mode_focused = active && self.view_focus == ViewFocus::TargetMode;
                let target_members_focused = active && self.view_focus == ViewFocus::TargetMembers;
                let skip_focused = active && self.view_focus == ViewFocus::Skip;
                let view_target_count = resolve_target_names(
                    &build_target_mode(&self.target_filter, &self.config),
                    &self.config,
                    &self.target_filter.skip,
                )
                .map(|t| t.len())
                .unwrap_or(0);
                let data = super::tabs::view_tab::ViewRenderData {
                    view_op: self.view_op,
                    theme: &self.theme,
                    navbar_focused: self.navbar_focused,
                    op_selector_focused,
                    result_focused,
                    target_filter: &self.target_filter,
                    target_count: view_target_count,
                    target_mode_focused,
                    target_members_focused,
                    skip_focused,
                    loading: self.view_loading,
                    checkout,
                    list,
                    log,
                    result_scroll: self.checkout_viewport.scroll_y,
                    checkout_selected: self.checkout_viewport.selected,
                    log_last_input: &self.log_last_input,
                    log_since_input: &self.log_since_input,
                    log_host_input: &self.log_host_input,
                    log_errors: self.log_errors,
                    log_action: operate_schema::action_str(self.log_action.as_ref()),
                    specific_focused,
                };
                super::tabs::view_tab::render_view(&data, chunks[1], frame);
            }
        }
        self.render_status_bar(chunks[2], frame);

        if self.help_open {
            self.render_help_popup(area, frame);
        }
        if self.info_open {
            self.render_info_popup(area, frame);
        }
        if let Some(picker) = &self.member_picker {
            picker.render(area, &self.theme, frame);
        }
        if self.running_op.is_some() {
            self.render_progress_popup(area, frame);
        }
        if let Some(report) = self.completed_report.clone() {
            self.render_results_popup(area, frame, &report);
        }
        if self.log_overlay_open {
            self.render_log_overlay(area, frame);
        }
        if self.auth_popup.is_some() {
            self.render_auth_popup(area, frame);
        }
        if self.export_popup.is_some() {
            self.render_export_popup(area, frame);
        }
    }

    fn render_tab_bar(&self, area: Rect, frame: &mut ratatui::Frame) {
        let titles: Vec<&str> = TabId::ALL.iter().map(|t| t.label()).collect();
        let selected = TabId::ALL
            .iter()
            .position(|t| *t == self.active_tab)
            .unwrap_or(0);
        let accent = match self.active_tab {
            TabId::Config => self.theme.accent_config,
            TabId::Operate => self.theme.accent_operate,
            TabId::View => self.theme.accent_checkout,
        };
        // Focus principle: reverse-video only while the NavBar holds focus;
        // once focus moves into a panel the active tab is bold/accent only.
        let highlight = if self.navbar_focused {
            Style::default()
                .fg(accent)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default().fg(accent).add_modifier(Modifier::BOLD)
        };
        let block_border_style = if self.navbar_focused {
            Style::default().fg(accent)
        } else {
            Style::default().fg(self.theme.border_inactive)
        };
        let tabs = Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" sshi ")
                    .title(Line::from(format!("v{} ", env!("CARGO_PKG_VERSION"))).right_aligned())
                    .border_style(block_border_style),
            )
            .select(selected)
            .style(Style::default().fg(self.theme.inactive))
            .highlight_style(highlight);
        frame.render_widget(tabs, area);
    }

    fn render_config(&mut self, area: Rect, frame: &mut ratatui::Frame) {
        self.config_tab.render(
            area,
            frame,
            &self.theme,
            &self.config,
            self.config_path.as_deref(),
            self.navbar_focused,
        );
    }

    /// Post-process state after a key was delegated to the Config tab. Shared by
    /// the popup-guard and general Config key paths so both flush pending
    /// deletes, autosave, validation errors, and the name-picker reopen
    /// consistently (entry-form keys go through the popup-guard path).
    fn after_config_key(&mut self) {
        if let Some((kind, index)) = self.config_tab.pending_delete.take() {
            self.config_tab
                .execute_delete(&mut self.config, kind, index);
        }
        // Autosave: every committed mutation marks the config dirty + pending,
        // which we flush to disk here (no explicit save key).
        if self.config_tab.pending_save {
            self.config_tab.pending_save = false;
            self.save_config();
        }
        if let Some(err) = self.config_tab.pending_error.take() {
            self.error = Some(err);
        }
        // The add-entry form was opened from an Operate name picker: once it
        // closes (commit or cancel), return to Operate and reopen the picker
        // with the (possibly new) entry available.
        if self.reopen_name_picker.is_some() && self.config_tab.entry_form.is_none() {
            let target = self.reopen_name_picker.take().unwrap();
            self.active_tab = TabId::Operate;
            self.open_name_picker(target);
        }
    }

    /// Switch the active tab, clearing any transient error banner so it does
    /// not stay stuck on screen after the user navigates away.
    fn goto_tab(&mut self, tab: TabId) {
        if self.active_tab != tab {
            self.error = None;
        }
        self.active_tab = tab;
    }

    fn save_config(&mut self) {
        // Consume the snapshot captured at the commit site. Re-capturing here
        // would be too late — `commit_entry_form` etc. already wiped viewports.
        let snap = self.config_tab.consume_pending_snapshot();
        let explicit_path = self.config_path.clone();
        let path_arg = explicit_path.as_deref();
        match crate::config::app::save(&self.config, path_arg) {
            Ok(()) => {
                self.config_tab.config_dirty = false;
                self.config_tab.reload_banner_until = Some(Instant::now() + Duration::from_secs(2));
                // Resolve the actual saved path so reload sees the correct mtime.
                let resolved_path = crate::config::app::resolve_path(path_arg).ok();
                if let Some(resolved) = resolved_path.as_ref() {
                    if self.config_path.is_none() {
                        self.config_path = Some(resolved.clone());
                    }
                    self.config_tab.reload(&self.config, Some(resolved));
                } else {
                    self.config_tab.reload(&self.config, path_arg);
                }
                if let Some(snap) = snap {
                    self.config_tab.restore_selection(snap, &self.config);
                }
            }
            Err(e) => {
                self.error = Some(format!("Config save failed: {e}"));
            }
        }
    }

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

    fn config_add_kind(&self) -> Option<super::tabs::config_tab::EntryFormKind> {
        use super::tabs::config_tab::EntryFormKind;
        let item = self
            .config_tab
            .items
            .get(self.config_tab.sidebar_vp.selected);
        match item {
            Some(super::tabs::config_tab::SidebarItem::SectionHosts)
            | Some(super::tabs::config_tab::SidebarItem::Host(_)) => Some(EntryFormKind::Host),
            Some(super::tabs::config_tab::SidebarItem::SectionChecks)
            | Some(super::tabs::config_tab::SidebarItem::Check(_)) => Some(EntryFormKind::Check),
            Some(super::tabs::config_tab::SidebarItem::SectionSyncs)
            | Some(super::tabs::config_tab::SidebarItem::Sync(_)) => Some(EntryFormKind::Sync),
            _ => None,
        }
    }

    fn handle_log_overlay_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('L') => {
                self.log_overlay_open = false;
                Ok(true)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.log_overlay_vp.move_up();
                Ok(true)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.log_overlay_vp.move_down();
                Ok(true)
            }
            KeyCode::PageUp => {
                self.log_overlay_vp.page_up();
                Ok(true)
            }
            KeyCode::PageDown => {
                self.log_overlay_vp.page_down();
                Ok(true)
            }
            KeyCode::Home => {
                self.log_overlay_vp.home();
                Ok(true)
            }
            KeyCode::End => {
                self.log_overlay_vp.end();
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn render_log_overlay(&mut self, area: Rect, frame: &mut ratatui::Frame) {
        let popup_area = centered_rect(80, 60, area);
        frame.render_widget(Clear, popup_area);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.warning))
            .title(" Log (L to close) ");
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let buf: &LogBufferHandle = match &self.log_buffer {
            Some(b) => b,
            None => {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        "(log capture not available)",
                        Style::default().fg(self.theme.inactive),
                    )),
                    inner,
                );
                return;
            }
        };

        let entries = buf.snapshot();
        let visible_h = inner.height as usize;
        self.log_overlay_vp.set_dims(entries.len(), visible_h);

        if entries.is_empty() {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "(no log entries yet)",
                    Style::default().fg(self.theme.inactive),
                )),
                inner,
            );
            return;
        }

        let (start, end) = self.log_overlay_vp.visible_range();
        let lines: Vec<Line> = entries[start..end.min(entries.len())]
            .iter()
            .enumerate()
            .map(|(rel, entry)| {
                let abs = start + rel;
                let is_sel = abs == self.log_overlay_vp.selected;
                let level_color = match entry.level.as_str() {
                    "ERROR" => self.theme.error,
                    "WARN" => self.theme.warning,
                    "INFO" => self.theme.accent_checkout,
                    _ => self.theme.inactive,
                };
                let style = if is_sel {
                    Style::default()
                        .fg(level_color)
                        .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                } else {
                    Style::default().fg(level_color)
                };
                let prefix = if is_sel { "▶ " } else { "  " };
                let text = trunc(
                    &format!(
                        "{}{:5} {} {}",
                        prefix, entry.level, entry.target, entry.text
                    ),
                    inner.width as usize,
                );
                Line::from(Span::styled(text, style))
            })
            .collect();

        frame.render_widget(Paragraph::new(lines), inner);
    }

    fn render_auth_popup(&mut self, area: Rect, frame: &mut ratatui::Frame) {
        use ratatui::style::Color;
        let popup_area = centered_rect(60, 30, area);
        frame.render_widget(Clear, popup_area);

        let popup = match &self.auth_popup {
            Some(p) => p,
            None => return,
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" SSH Authentication ");
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let chunks = ratatui::layout::Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(inner);

        let prompt_text = Paragraph::new(popup.prompt.as_str())
            .style(Style::default().fg(self.theme.inactive))
            .wrap(Wrap { trim: true });
        frame.render_widget(prompt_text, chunks[0]);

        // Render masked input (show '*' for each character).
        let masked: String = "*".repeat(popup.input.value.chars().count());
        let masked_field = InputField::new(&masked);
        masked_field.render(frame, chunks[1], "Credential", true);

        let hint = Paragraph::new(Span::styled(
            "Enter to confirm · Esc to cancel",
            Style::default().fg(self.theme.inactive),
        ));
        frame.render_widget(hint, chunks[2]);
    }

    fn render_export_popup(&mut self, area: Rect, frame: &mut ratatui::Frame) {
        use ratatui::style::Color;
        let popup_area = centered_rect(60, 25, area);
        frame.render_widget(Clear, popup_area);

        let popup = match &self.export_popup {
            Some(p) => p,
            None => return,
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Export Report to File ");
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let chunks = ratatui::layout::Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(inner);

        let command_name = match popup.source {
            ViewOperationKind::Checkout => "checkout",
            ViewOperationKind::Log => "log",
            ViewOperationKind::List => "list",
        };

        let prompt = format!(
            "Enter path to save {} report (.json or .html).\nLeave empty for auto-generated name.",
            command_name
        );
        let prompt_text = Paragraph::new(prompt.as_str())
            .style(Style::default().fg(self.theme.inactive))
            .wrap(Wrap { trim: true });
        frame.render_widget(prompt_text, chunks[0]);

        popup
            .input
            .render(frame, chunks[1], "Output File Path", true);

        let hint = Paragraph::new(Span::styled(
            "Enter to confirm · Esc to cancel",
            Style::default().fg(self.theme.inactive),
        ));
        frame.render_widget(hint, chunks[2]);
    }

    /// §7.4 external editor 4-stage flow.
    ///
    /// Called from `run()` when `needs_editor_open` is set — giving access to
    /// the `Terminal` object needed for `terminal.clear()` after restore.
    fn do_open_editor(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        let path = match &self.config_path {
            Some(p) => p.clone(),
            None => match crate::config::app::resolve_path(None) {
                Ok(p) => p,
                Err(e) => {
                    self.error = Some(format!("Cannot resolve config path: {e}"));
                    return Ok(());
                }
            },
        };

        // Resolve editor: $VISUAL → $EDITOR → platform default.
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| {
                if cfg!(windows) {
                    "notepad".to_string()
                } else {
                    "vi".to_string()
                }
            });

        // Stage 1 — PAUSE: leave alternate screen + disable raw mode.
        let _ = execute!(io::stdout(), terminal::LeaveAlternateScreen);
        let _ = terminal::disable_raw_mode();
        let _ = io::stdout().flush();

        // Stage 2 — EXECUTE.
        let status = std::process::Command::new(&editor).arg(&path).status();

        // Stage 3 — RESTORE: re-enter alternate screen.
        let _ = terminal::enable_raw_mode();
        let _ = execute!(io::stdout(), terminal::EnterAlternateScreen);
        terminal.clear()?;

        if let Err(e) = &status {
            self.error = Some(format!("Failed to launch '{editor}': {e}"));
            return Ok(());
        }

        // Detect mtime change and reload config if file was modified.
        let new_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();

        let should_reload = match (self.config_tab.config_mtime, new_mtime) {
            (Some(old), Some(new)) => old != new,
            // On Windows mtime granularity is 2s; also treat a successful exit as reload signal.
            _ => status.map(|s| s.success()).unwrap_or(false),
        };

        if should_reload {
            match crate::config::app::load(Some(&path)) {
                Ok(Some(new_config)) => {
                    self.config = new_config;
                    self.config_tab.reload(&self.config, Some(&path));
                    self.config_tab.reload_banner_until =
                        Some(Instant::now() + Duration::from_secs(2));
                }
                Ok(None) => {
                    self.error = Some("Config file disappeared after editor exit.".to_string());
                }
                Err(e) => {
                    self.error = Some(format!("Config reload failed: {e}"));
                }
            }
        }

        Ok(())
    }

    fn render_operate(&self, area: Rect, frame: &mut ratatui::Frame) {
        let target_count = match resolve_target_names(
            &build_target_mode(&self.target_filter, &self.config),
            &self.config,
            &self.target_filter.skip,
        ) {
            Ok(t) => t.len(),
            Err(_) => 0,
        };
        let data = OperateRenderData {
            focus: self.operate_focus,
            operation: self.operate_operation,
            dry_run: self.op_dry_run,
            sync_adhoc_files: &self.sync_adhoc_files,
            sync_adhoc_input: &self.sync_adhoc_input,
            sync_source_input: &self.sync_source_input,
            run_command: &self.run_command,
            exec_script: &self.exec_script,
            cp_local_input: &self.cp_local,
            cp_remote_input: &self.cp_remote,
            check_name_input: &self.check_name,
            sync_name_input: &self.sync_name,
            out_input: &self.out_input,
            run_sudo: self.run_sudo,
            exec_sudo: self.exec_sudo,
            exec_keep: self.exec_keep,
            theme: &self.theme,
            is_running: self.running_op.is_some(),
            target_filter: &self.target_filter,
            target_count,
            navbar_focused: self.navbar_focused,
        };
        operate_tab::render_operate(&data, area, frame);
    }

    fn render_progress_popup(&self, area: Rect, frame: &mut ratatui::Frame) {
        let Some(op) = &self.running_op else {
            return;
        };
        let op_name = match self.operate_operation {
            OperationKind::Check => "check",
            OperationKind::Run => "run",
            OperationKind::Exec => "exec",
            OperationKind::Sync => "sync",
            OperationKind::Cp => "cp",
        };
        operate_tab::render_progress_popup(
            &self.theme,
            op_name,
            &op.host_outcomes,
            &op.targets,
            op.started_at.elapsed().as_secs(),
            op.completed_count(),
            self.progress_popup_scroll,
            area,
            frame,
        );
    }

    fn render_results_popup(&self, area: Rect, frame: &mut ratatui::Frame, report: &CommandReport) {
        let popup_area = centered_rect(75, 75, area);
        frame.render_widget(Clear, popup_area);

        // Extract common fields for any variant.
        let (host_count, executed_at, header_detail): (usize, &str, String) = match report {
            CommandReport::Check(r) => (r.hosts.len(), r.executed_at.as_str(), String::new()),
            CommandReport::Run(r) => (
                r.hosts.len(),
                r.executed_at.as_str(),
                format!("  cmd: {}", truncate(&r.command, 50)),
            ),
            CommandReport::Exec(r) => (
                r.hosts.len(),
                r.executed_at.as_str(),
                format!("  script: {}", truncate(&r.script, 50)),
            ),
            CommandReport::Sync(r) => (
                r.hosts.len(),
                r.executed_at.as_str(),
                format!(
                    "  mode:{} dry-run:{} total_synced:{}",
                    r.mode, r.dry_run, r.total_files_synced
                ),
            ),
            CommandReport::Cp(r) => (
                r.hosts.len(),
                r.executed_at.as_str(),
                format!(
                    "  {} → {}  ({} file(s)/host)",
                    truncate(&r.local, 30),
                    truncate(&r.remote, 24),
                    r.planned_files
                ),
            ),
            CommandReport::Log(r) => (
                r.entries.len(),
                r.executed_at.as_str(),
                format!(
                    "  query: last={} errors={}",
                    r.query_params.last, r.query_params.errors
                ),
            ),
            CommandReport::List(r) => (
                r.hosts.len(),
                r.executed_at.as_str(),
                format!("  checks:{} syncs:{}", r.checks.len(), r.syncs.len()),
            ),
        };

        let title = format!(" Results — {host_count} hosts  (Enter / Esc to dismiss) ");
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border_active))
            .title(title);
        let inner = block.inner(popup_area);
        frame.render_widget(block, popup_area);

        let mut lines: Vec<Line> = Vec::new();

        // Helper closure to render a per-host row.
        let render_row = |host: &str, status: HostStatus, detail: &str, ms: Option<u64>| {
            let glyph = match status {
                HostStatus::Online => "✓",
                HostStatus::Partial => "⚠",
                HostStatus::Offline | HostStatus::Error => "✗",
                HostStatus::Unreachable => "⊘",
                HostStatus::TimedOut => "⏱",
                HostStatus::Skipped => "⊘",
            };
            let color = match status {
                HostStatus::Online => self.theme.accent_checkout,
                HostStatus::Partial => self.theme.warning,
                HostStatus::Skipped => self.theme.inactive,
                _ => self.theme.error,
            };
            let ms_val = ms.unwrap_or(0);
            let line = format!(
                "  {} {:<16} ({:>4}ms) — {}",
                glyph,
                truncate(host, 16),
                ms_val,
                truncate(detail, 80),
            );
            Line::from(Span::styled(line, Style::default().fg(color)))
        };

        match report {
            CommandReport::Check(r) => {
                let ok = r
                    .hosts
                    .iter()
                    .filter(|h| matches!(h.status, HostStatus::Online | HostStatus::Partial))
                    .count();
                let skipped = r
                    .hosts
                    .iter()
                    .filter(|h| h.status == HostStatus::Skipped)
                    .count();
                let fail = r.hosts.len() - ok - skipped;
                lines.push(Line::from(format!(
                    "Summary: {ok} ok / {fail} fail / {skipped} skipped    Executed: {executed_at}"
                )));
                lines.push(Line::from(""));
                for h in &r.hosts {
                    lines.push(render_row(&h.host, h.status, &h.detail, h.duration_ms));
                }
            }
            CommandReport::Run(r) => {
                let ok = r
                    .hosts
                    .iter()
                    .filter(|h| h.status == HostStatus::Online)
                    .count();
                let skipped = r
                    .hosts
                    .iter()
                    .filter(|h| h.status == HostStatus::Skipped)
                    .count();
                let fail = r.hosts.len() - ok - skipped;
                lines.push(Line::from(format!(
                    "Summary: {ok} ok / {fail} fail / {skipped} skipped    Executed: {executed_at}"
                )));
                lines.push(Line::from(header_detail));
                lines.push(Line::from(""));
                for h in &r.hosts {
                    lines.push(render_row(&h.host, h.status, &h.detail, h.duration_ms));
                    // Show first line of stdout for context.
                    if !h.stdout.is_empty() {
                        let first = h.stdout.lines().next().unwrap_or("").trim();
                        if !first.is_empty() {
                            lines.push(Line::from(format!("     ↳ {}", truncate(first, 70))));
                        }
                    }
                }
            }
            CommandReport::Exec(r) => {
                let ok = r
                    .hosts
                    .iter()
                    .filter(|h| h.status == HostStatus::Online)
                    .count();
                let skipped = r
                    .hosts
                    .iter()
                    .filter(|h| h.status == HostStatus::Skipped)
                    .count();
                let fail = r.hosts.len() - ok - skipped;
                lines.push(Line::from(format!(
                    "Summary: {ok} ok / {fail} fail / {skipped} skipped    Executed: {executed_at}"
                )));
                lines.push(Line::from(header_detail));
                lines.push(Line::from(""));
                for h in &r.hosts {
                    lines.push(render_row(&h.host, h.status, &h.detail, h.duration_ms));
                    if !h.stdout.is_empty() {
                        let first = h.stdout.lines().next().unwrap_or("").trim();
                        if !first.is_empty() {
                            lines.push(Line::from(format!("     ↳ {}", truncate(first, 70))));
                        }
                    }
                }
            }
            CommandReport::Sync(r) => {
                let ok = r
                    .hosts
                    .iter()
                    .filter(|h| matches!(h.status, HostStatus::Online))
                    .count();
                let fail = r
                    .hosts
                    .iter()
                    .filter(|h| !matches!(h.status, HostStatus::Online))
                    .count();
                lines.push(Line::from(format!(
                    "Summary: {ok} ok / {fail} fail    synced:{} skipped:{}    Executed: {executed_at}",
                    r.total_files_synced, r.total_files_skipped
                )));
                lines.push(Line::from(header_detail));
                lines.push(Line::from(""));
                for h in &r.hosts {
                    let detail = if h.files_synced > 0 || h.files_skipped > 0 {
                        format!("{} synced, {} skipped", h.files_synced, h.files_skipped)
                    } else {
                        h.detail.clone()
                    };
                    lines.push(render_row(&h.host, h.status, &detail, h.duration_ms));
                    for err in h.errors.iter().take(2) {
                        lines.push(Line::from(format!("     ↳ {}", truncate(err, 70))));
                    }
                }
            }
            CommandReport::Cp(r) => {
                let ok = r
                    .hosts
                    .iter()
                    .filter(|h| h.status == HostStatus::Online)
                    .count();
                let skipped = r
                    .hosts
                    .iter()
                    .filter(|h| h.status == HostStatus::Skipped)
                    .count();
                let fail = r.hosts.len() - ok - skipped;
                lines.push(Line::from(format!(
                    "Summary: {ok} ok / {fail} fail / {skipped} skipped    Executed: {executed_at}"
                )));
                lines.push(Line::from(header_detail));
                lines.push(Line::from(""));
                for h in &r.hosts {
                    lines.push(render_row(&h.host, h.status, &h.detail, h.duration_ms));
                    for err in h.errors.iter().take(2) {
                        lines.push(Line::from(format!("     ↳ {}", truncate(err, 70))));
                    }
                }
            }
            CommandReport::Log(r) => {
                lines.push(Line::from(format!(
                    "Log Entries: {} queried    Executed: {executed_at}",
                    r.entries.len()
                )));
                lines.push(Line::from(header_detail));
                lines.push(Line::from(""));
                for e in &r.entries {
                    lines.push(render_row(
                        &e.host,
                        e.status,
                        &format!(
                            "{} {} - {}",
                            e.command,
                            e.action,
                            e.note.as_deref().unwrap_or("")
                        ),
                        e.duration_ms.map(|d| d as u64),
                    ));
                }
            }
            CommandReport::List(r) => {
                lines.push(Line::from(format!(
                    "List Entries: {} hosts    Executed: {executed_at}",
                    r.hosts.len()
                )));
                lines.push(Line::from(header_detail));
                lines.push(Line::from(""));
                for h in &r.hosts {
                    lines.push(render_row(
                        &h.host,
                        HostStatus::Online,
                        &format!(
                            "ssh: {} shell: {} groups: {}",
                            h.ssh_host,
                            h.shell,
                            h.groups.join(",")
                        ),
                        None,
                    ));
                }
            }
        }

        let p = Paragraph::new(lines).wrap(Wrap { trim: false });
        frame.render_widget(p, inner);
    }

    fn render_info_popup(&self, area: Rect, frame: &mut ratatui::Frame) {
        let popup_area = centered_rect(60, 50, area);
        frame.render_widget(Clear, popup_area);
        let body = match self.active_tab {
            TabId::Operate => format!(
                "Operate tab\n\nSelect an operation with ← → on the Operation row.\n\ncheck — collect host metrics and write to DB.\nrun   — execute a shell command on all targets.\nexec  — upload and run a local script on targets.\nsync  — sync files between hosts.\ncp    — copy a local file/dir to targets (defaults to ~).\n\nUse `f` to change the target filter; press Enter on [Execute] to run.\nSet the Out field to write a .json/.html report (auto-named if left bare).\n`d` toggles dry-run: a preview that contacts no hosts and writes no report.\nEsc cancels a running operation (may take up to {}s per host).\n\nTab cycles fields within a section; ↑↓ move across sections.\nPgUp/PgDn/Home/End scroll the applicable-entries list and the progress popup.\n\nResults appear in a popup when the operation completes.",
                self.last_timeout_secs
            ),
            TabId::View => "View tab\n\nView checkout snapshots, host/config list, or operation log.\nUse ↑↓ to move between fields; ←→ switches op (on Op row) or target mode.\nEnter on Members/Skip opens the picker; PgUp/PgDn/Home/End scroll results.\no     — export the currently viewed data to a report file.\nData refreshes automatically on op switch and after operations.".to_string(),
            TabId::Config => format!(
                "Config tab (read-only browser)\n\n\
                 Sidebar: ↑↓ / jk to move between sections and entries.\n\
                 Space / Enter on a ▼/▶ section header collapses or expands it.\n\
                 Field table: → or Tab to enter, ← to return to sidebar.\n\
                 Within each pane: ↑↓ / jk / PgUp / PgDn / Home / End.\n\n\
                 E  — open config in $VISUAL / $EDITOR / vi (TUI suspends,\n\
                      resumes after exit; config reloads if file was changed).\n\n\
                 Config path: {}",
                self.config_path
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(default — ~/.config/sshi/config.toml)".to_string())
            ),
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border_active))
            .title(" Info (i) ");
        let p = Paragraph::new(body).block(block).wrap(Wrap { trim: false });
        frame.render_widget(p, popup_area);
    }

    fn render_status_bar(&self, area: Rect, frame: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(area);

        if self.active_tab == TabId::Config && self.config_tab.banner_active() {
            let p = Paragraph::new("  ✓ Config saved").style(
                Style::default()
                    .fg(self.theme.warning)
                    .add_modifier(Modifier::BOLD),
            );
            frame.render_widget(p, chunks[0]);
        } else if let Some(err) = &self.error {
            let p = Paragraph::new(err.as_str()).style(Style::default().fg(self.theme.error));
            frame.render_widget(p, chunks[0]);
        }

        let hints = match self.active_tab {
            TabId::Config => {
                "↑↓:Rows ←→:Zones e:Edit Del:Clear E:Editor a:Add d:DelEntry L:Log i:Info ?:Help q:Quit"
            }
            TabId::Operate => {
                "↑↓:Fields ←→/Tab:Value Enter:Pick Space:Toggle Del:Clear e:Exec s:Serial d:Dry ?:Help q:Quit"
            }
            TabId::View => {
                "↑↓:Fields ←→/Tab:Show/Mode Enter:Pick PgUp/PgDn o:Export L:Log i:Info ?:Help q:Quit"
            }
        };
        let p = Paragraph::new(Line::from(vec![Span::styled(
            hints,
            Style::default().fg(self.theme.inactive),
        )]));
        frame.render_widget(p, chunks[1]);
    }

    fn render_help_popup(&self, area: Rect, frame: &mut ratatui::Frame) {
        let popup_area = centered_rect(60, 70, area);
        frame.render_widget(Clear, popup_area);
        let body = "\
Global keys
  1 / 2 / 3   Switch to Config / Operate / View
  Tab         Cycle to next tab
  Shift+Tab   Cycle to previous tab
  q           Quit (state saved)
  Ctrl+C      Quit immediately (state saved)
  Esc         Close popup / clear error
  ?           Toggle this help
  L           Toggle log overlay
  i           Toggle contextual info popup

Operate tab
  ↑↓ / j k   Navigate zones: OpRadio → ParamPanel → TargetRow → Execute
  ← → / Tab   (OpRadio / Target row) cycle the selected option
  f           Open Target Filter popup
  Enter       (ParamPanel text field) activate input; (Execute) run operation
  e           Run the current operation (from anywhere on the tab)
  Space       (checkbox) toggle sudo / keep / dry-run; (Source) cycle host
  Del         Clear the focused optional field (members/skip/names/source/
              out/cp-remote); on the ad-hoc input, remove the last path
  Esc         Dismiss results popup / cancel running operation
  (while typing) Enter to confirm, Esc to revert

Sync operation (ParamPanel)
  Enter on Entries / Source  Open the multi/single-select popup
  Space on Source            Cycle source host (none → host → … → none)
  Enter on Ad-hoc input      Add typed path to the list
  Del on Ad-hoc input        Remove last path from the list
  Space on Dry-run       Toggle dry-run flag

View tab
  ← → / Tab   (Show row) cycle checkout / list / log
  ↑↓ / j k    Move row selection
  PgUp/PgDn   Page navigation
  Home/End    Jump to top / bottom
  f           Open Filter popup (disabled for Log)

Filter popup
  ↑↓ / Tab    Move between fields
  Space/Enter Toggle / select
  Enter on [Apply]   Commit + persist filter
  Esc                Cancel

Config tab
  ↑↓ / j k    Move sidebar / field rows
  ← / →       Switch zones (Sidebar ↔ FieldTable)
  Tab         Switch zone (Sidebar → FieldTable)
  PgUp/PgDn   Page navigation
  Home/End    Jump to top / bottom
  e / Enter   Edit text field inline; cycle option fields (bool/shell/enum)
  Space       Cycle the focused option field (bool/shell/tri-bool/enum)
  Del         Clear the focused optional field (required names are kept)
  a           Add new entry (host / check / sync)
  d           Delete focused entry
              (changes autosave to disk, format-preserving via toml_edit)
  E           Open config in $VISUAL/$EDITOR (TUI suspends, reloads on change)

Log overlay
  L           Open / close
  ↑↓ / j k    Scroll
  PgUp/PgDn   Page navigation
  Home/End    Jump to top / bottom
";
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.border_active))
            .title(" Keybindings (?) ");
        let p = Paragraph::new(body).block(block).wrap(Wrap { trim: false });
        frame.render_widget(p, popup_area);
    }
}

/// Cycle the operation radio (→ forward, ← backward).
/// Split a comma-separated name input into trimmed, non-empty names.
fn comma_names(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

fn cycle_operation(op: OperationKind, forward: bool) -> OperationKind {
    let order = [
        OperationKind::Run,
        OperationKind::Exec,
        OperationKind::Sync,
        OperationKind::Cp,
        OperationKind::Check,
    ];
    let pos = order.iter().position(|o| *o == op).unwrap_or(0);
    let next = if forward {
        (pos + 1) % order.len()
    } else {
        (pos + order.len() - 1) % order.len()
    };
    order[next]
}

/// Cycle the target-mode radio (→ forward, ← backward).
fn cycle_target_mode(mode: TargetFilterMode, forward: bool) -> TargetFilterMode {
    let order = [
        TargetFilterMode::All,
        TargetFilterMode::Groups,
        TargetFilterMode::Hosts,
        TargetFilterMode::Shell,
    ];
    let pos = order.iter().position(|m| *m == mode).unwrap_or(0);
    let next = if forward {
        (pos + 1) % order.len()
    } else {
        (pos + order.len() - 1) % order.len()
    };
    order[next]
}

/// Build a `TargetMode` from the persisted filter state and current config.
/// Empty Groups/Hosts → falls back to All.
fn build_target_mode(filter: &TargetFilterState, _config: &AppConfig) -> TargetMode {
    match filter.mode {
        TargetFilterMode::All => TargetMode::All,
        // Empty Groups/Hosts resolve to *zero* targets, not All — silently
        // widening an empty group selection to every host is dangerous and
        // confusing. The mode also stays as the user set it.
        TargetFilterMode::Groups => TargetMode::Groups(filter.groups.clone()),
        TargetFilterMode::Hosts => TargetMode::Hosts(filter.hosts.clone()),
        TargetFilterMode::Shell => TargetMode::Shell(vec![filter.shell.to_shell_type()]),
    }
}

/// Resolve the matching host names for a TargetMode against a config.
fn resolve_target_names(
    mode: &TargetMode,
    config: &AppConfig,
    skip: &[String],
) -> anyhow::Result<Vec<String>> {
    let mut names: Vec<String> = match mode {
        TargetMode::All => config.host.iter().map(|h| h.name.clone()).collect(),
        TargetMode::Hosts(specs) => config
            .host
            .iter()
            .filter(|h| specs.contains(&h.name))
            .map(|h| h.name.clone())
            .collect(),
        TargetMode::Groups(groups) => config
            .host
            .iter()
            .filter(|h| h.groups.iter().any(|g| groups.contains(g)))
            .map(|h| h.name.clone())
            .collect(),
        TargetMode::Shell(shells) => config
            .host
            .iter()
            .filter(|h| shells.contains(&h.shell))
            .map(|h| h.name.clone())
            .collect(),
    };
    names.retain(|n| !skip.iter().any(|s| s == n));
    Ok(names)
}

/// Spawn a background task that listens for OS signals and pushes a unit into
/// the channel for each. The main loop drains this channel each iteration.
///
/// Unix: SIGHUP, SIGTERM, SIGINT.
/// Windows: ctrl_c (covers Ctrl+C and CTRL_BREAK_EVENT).
/// CTRL_CLOSE_EVENT (Windows close-button) deferred to post-MVP — see §7.9.
fn spawn_signal_listener(tx: tokio::sync::mpsc::Sender<()>) {
    #[cfg(unix)]
    {
        tokio::spawn(async move {
            use tokio::signal::unix::{signal, SignalKind};
            let mut sighup = match signal(SignalKind::hangup()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to install SIGHUP handler: {e}");
                    return;
                }
            };
            let mut sigterm = match signal(SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to install SIGTERM handler: {e}");
                    return;
                }
            };
            let mut sigint = match signal(SignalKind::interrupt()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Failed to install SIGINT handler: {e}");
                    return;
                }
            };
            loop {
                tokio::select! {
                    _ = sighup.recv() => { let _ = tx.send(()).await; }
                    _ = sigterm.recv() => { let _ = tx.send(()).await; }
                    _ = sigint.recv() => { let _ = tx.send(()).await; }
                }
            }
        });
    }
    #[cfg(windows)]
    {
        tokio::spawn(async move {
            // TODO(post-MVP windows): CTRL_CLOSE_EVENT via windows-sys for
            // close-button shutdown on Windows.
            loop {
                if tokio::signal::ctrl_c().await.is_ok() {
                    let _ = tx.send(()).await;
                }
            }
        });
    }
}

#[cfg(test)]
mod flush_tests {
    use super::flush_config_if_dirty;
    use crate::config::app as cfg_app;
    use crate::config::schema::AppConfig;
    use tempfile::NamedTempFile;

    #[test]
    fn flush_writes_when_dirty_and_clears_flag() {
        let mut config = AppConfig::default();
        config.settings.skipped_hosts = vec!["host-a".to_string(), "host-b".to_string()];
        // Use into_temp_path() to close the file handle: on Windows, save() uses
        // an atomic rename which fails with "Access denied" if the destination
        // file is still open.
        let tmp = NamedTempFile::new().expect("temp file").into_temp_path();
        let path = tmp.to_path_buf();

        let mut dirty = true;
        flush_config_if_dirty(&mut dirty, &config, Some(&path));

        assert!(!dirty, "dirty flag must be cleared on successful save");
        let loaded = cfg_app::load(Some(&path))
            .expect("load")
            .expect("config should exist");
        assert_eq!(loaded.settings.skipped_hosts, vec!["host-a", "host-b"]);
    }

    #[test]
    fn flush_is_noop_when_not_dirty() {
        let config = AppConfig::default();
        let tmp = NamedTempFile::new().expect("temp file");
        let path = tmp.path().to_path_buf();
        std::fs::write(&path, b"# pre-existing\n").unwrap();
        let mtime_before = std::fs::metadata(&path).unwrap().modified().unwrap();

        let mut dirty = false;
        flush_config_if_dirty(&mut dirty, &config, Some(&path));

        assert!(!dirty);
        let mtime_after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(
            mtime_before, mtime_after,
            "file must not be touched when not dirty"
        );
    }
}

#[cfg(test)]
mod resolve_tests {
    use super::{build_target_mode, resolve_target_names};
    use crate::config::schema::{AppConfig, HostEntry, ShellType};
    use crate::tui::state::persist::{TargetFilterMode, TargetFilterState};

    fn cfg_with_group(hosts: &[(&str, &[&str])]) -> AppConfig {
        let mut cfg = AppConfig::default();
        for (name, groups) in hosts {
            cfg.host.push(HostEntry {
                name: name.to_string(),
                ssh_host: name.to_string(),
                shell: ShellType::Sh,
                groups: groups.iter().map(|s| s.to_string()).collect(),
                proxy_jump: None,
            });
        }
        cfg
    }

    #[test]
    fn skip_subtracts_from_resolved_targets() {
        let cfg = cfg_with_group(&[("h1", &["g"]), ("h2", &["g"]), ("h3", &["g"])]);
        let filter = TargetFilterState {
            mode: TargetFilterMode::All,
            skip: vec!["h2".to_string()],
            ..Default::default()
        };
        let mode = build_target_mode(&filter, &cfg);
        let names = resolve_target_names(&mode, &cfg, &filter.skip).unwrap();
        assert_eq!(names, vec!["h1".to_string(), "h3".to_string()]);
    }
}

#[cfg(test)]
mod view_focus_tests {
    use super::{TargetFilterMode, ViewFocus, ViewOperationKind};

    #[test]
    fn checkout_list_have_common_zone_stops() {
        // All mode: OpSelector → TargetMode → Skip → Result (no Members row).
        let stops = ViewFocus::stops(ViewOperationKind::Checkout, TargetFilterMode::All);
        assert_eq!(
            stops,
            vec![
                ViewFocus::OpSelector,
                ViewFocus::TargetMode,
                ViewFocus::Skip,
                ViewFocus::Result,
            ]
        );

        // Groups mode adds the Members stop.
        let stops = ViewFocus::stops(ViewOperationKind::List, TargetFilterMode::Groups);
        assert_eq!(
            stops,
            vec![
                ViewFocus::OpSelector,
                ViewFocus::TargetMode,
                ViewFocus::TargetMembers,
                ViewFocus::Skip,
                ViewFocus::Result,
            ]
        );
    }

    #[test]
    fn log_has_seven_stops_with_five_specifics() {
        let stops = ViewFocus::stops(ViewOperationKind::Log, TargetFilterMode::All);
        assert_eq!(stops.len(), 7);
        assert_eq!(stops[0], ViewFocus::OpSelector);
        for i in 0..5 {
            assert_eq!(stops[i + 1], ViewFocus::Specific(i));
        }
        assert_eq!(stops[6], ViewFocus::Result);
    }
}
