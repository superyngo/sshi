use crate::cli::ActionFilter;
use crate::commands::list::ListData;
use crate::commands::log::LogRow;
use crate::tui::state::persist::ViewOperationKind;

use super::components::input_field::InputField;
use super::components::member_picker::{MemberPicker, PickerTarget};
use super::state::persist::{OperationKind, TargetFilterMode};
use super::tabs::operate_tab::OpField;
use crate::host::auth::SshAuthRequest;

/// ESC-toggle level for Operate and View tabs (3-way cycling).
///
/// Config tab is excluded — it keeps the original 2-way NavBar toggle.
/// The cycle is: NavBar → TopField → Content → NavBar → …
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscLevel {
    NavBar,
    TopField,
    Content,
}

impl EscLevel {
    pub fn next(self) -> EscLevel {
        match self {
            EscLevel::NavBar => EscLevel::TopField,
            EscLevel::TopField => EscLevel::Content,
            EscLevel::Content => EscLevel::NavBar,
        }
    }
}

/// Focus zone within the View tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewFocus {
    OpSelector,
    TargetMode,
    TargetMembers,
    Skip,
    CombinedToggle,
    Specific(usize),
    Result,
}

impl ViewFocus {
    pub fn stops(op: ViewOperationKind, mode: TargetFilterMode) -> Vec<ViewFocus> {
        match op {
            ViewOperationKind::Checkout => {
                let mut v = vec![ViewFocus::OpSelector, ViewFocus::TargetMode];
                if mode != TargetFilterMode::All {
                    v.push(ViewFocus::TargetMembers);
                }
                v.push(ViewFocus::Skip);
                v.push(ViewFocus::CombinedToggle);
                v.push(ViewFocus::Result);
                v
            }
            ViewOperationKind::List => {
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
pub struct AuthPopup {
    pub prompt: String,
    pub input: InputField,
    pub responder: Option<tokio::sync::oneshot::Sender<String>>,
}

impl AuthPopup {
    pub fn new(req: SshAuthRequest) -> Self {
        let mut input = InputField::new("");
        input.activate();
        Self {
            prompt: req.prompt,
            input,
            responder: Some(req.responder),
        }
    }

    pub fn submit(&mut self) {
        let credential = std::mem::take(&mut self.input.value);
        if let Some(tx) = self.responder.take() {
            let _ = tx.send(credential);
        }
    }

    pub fn cancel(&mut self) {
        self.input.value.clear();
        self.responder = None;
    }
}

/// State for the "Export to file" popup in View tab.
pub struct ExportPopup {
    pub input: InputField,
    pub source: ViewOperationKind,
}

impl ExportPopup {
    pub fn new(source: ViewOperationKind) -> Self {
        let mut input = InputField::new("");
        input.activate();
        Self { input, source }
    }
}

/// Operate tab state: operation selection, text inputs, dry-run toggle.
pub struct OperateState {
    pub operation: OperationKind,
    pub focus: OpField,
    pub dry_run: bool,
    pub run_command: InputField,
    pub exec_script: InputField,
    pub cp_local: InputField,
    pub cp_remote: InputField,
    pub check_name: InputField,
    pub sync_name: InputField,
    pub run_sudo: bool,
    pub exec_sudo: bool,
    pub exec_keep: bool,
    pub sync_dry_run: bool,
    pub sync_adhoc_files: Vec<String>,
    pub sync_adhoc_input: InputField,
    pub sync_source_input: InputField,
    pub out_input: InputField,
    pub esc_level: EscLevel,
}

impl OperateState {
    pub fn new(persisted: &super::state::persist::OperateState) -> Self {
        Self {
            operation: persisted.operation,
            focus: OpField::OpRadio,
            dry_run: persisted.dry_run,
            run_command: InputField::new(&persisted.run_command),
            exec_script: InputField::new(&persisted.exec_script),
            cp_local: InputField::new(&persisted.cp_local),
            cp_remote: InputField::new(&persisted.cp_remote),
            check_name: InputField::new(""),
            sync_name: InputField::new(""),
            run_sudo: persisted.run_sudo,
            exec_sudo: persisted.exec_sudo,
            exec_keep: persisted.exec_keep,
            sync_dry_run: persisted.sync_dry_run,
            sync_adhoc_files: Vec::new(),
            sync_adhoc_input: InputField::new(""),
            sync_source_input: InputField::new(""),
            out_input: InputField::new(""),
            esc_level: EscLevel::NavBar,
        }
    }
}

/// View tab state: cached results, filters, log params.
pub struct ViewState {
    pub op: ViewOperationKind,
    pub list: Option<ListData>,
    pub log: Vec<LogRow>,
    pub checkout_combined: bool,
    pub dirty: bool,
    pub loading: bool,
    pub log_last: usize,
    pub log_errors: bool,
    pub log_action: Option<ActionFilter>,
    pub log_last_input: InputField,
    pub log_since_input: InputField,
    pub log_host_input: InputField,
    pub focus: ViewFocus,
    pub esc_level: EscLevel,
}

impl ViewState {
    pub fn new(persisted: &super::state::persist::OperateState) -> Self {
        let log_last = if persisted.log_last == 0 {
            20
        } else {
            persisted.log_last
        };
        Self {
            op: persisted.view_operation,
            list: None,
            log: Vec::new(),
            checkout_combined: persisted.checkout_combined,
            dirty: true,
            loading: false,
            log_last,
            log_errors: persisted.log_errors,
            log_action: None,
            log_last_input: InputField::new(&log_last.to_string()),
            log_since_input: InputField::new(""),
            log_host_input: InputField::new(""),
            focus: ViewFocus::OpSelector,
            esc_level: EscLevel::NavBar,
        }
    }
}

/// Popup state: auth, export, member picker, results.
pub struct PopupState {
    pub auth: Option<AuthPopup>,
    pub export: Option<ExportPopup>,
    pub member_picker: Option<MemberPicker>,
    pub reopen_name_picker: Option<PickerTarget>,
    pub reopen_view_edit: Option<usize>,
    pub progress_scroll: Option<usize>,
    pub completed_report_scroll: usize,
}

impl Default for PopupState {
    fn default() -> Self {
        Self::new()
    }
}

impl PopupState {
    pub fn new() -> Self {
        Self {
            auth: None,
            export: None,
            member_picker: None,
            reopen_name_picker: None,
            reopen_view_edit: None,
            progress_scroll: None,
            completed_report_scroll: 0,
        }
    }
}
