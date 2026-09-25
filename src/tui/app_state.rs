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
        let mut input = InputField::new_secret();
        input.activate();
        Self {
            prompt: req.prompt,
            input,
            responder: Some(req.responder),
        }
    }

    pub fn submit(&mut self) {
        // Moved, not copied: the auth side wraps it in a zeroizing `SecretString`.
        let mut credential = std::mem::take(&mut self.input.value);
        if let Some(tx) = self.responder.take() {
            if let Err(mut unsent) = tx.send(credential) {
                zeroize::Zeroize::zeroize(&mut unsent);
            }
        } else {
            zeroize::Zeroize::zeroize(&mut credential);
        }
        self.input.wipe();
    }

    pub fn cancel(&mut self) {
        self.input.wipe();
        self.responder = None;
    }
}

impl Drop for AuthPopup {
    fn drop(&mut self) {
        self.input.wipe();
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
    /// Credential requests waiting behind the open `auth` popup.
    pub auth_queue: std::collections::VecDeque<SshAuthRequest>,
    pub export: Option<ExportPopup>,
    pub member_picker: Option<MemberPicker>,
    pub reopen_name_picker: Option<PickerTarget>,
    pub reopen_view_edit: Option<usize>,
    pub progress_scroll: Option<usize>,
    pub completed_report_scroll: usize,
}

impl PopupState {
    /// Show `req` now, or queue it if a credential popup is already open
    /// (replacing it would drop the first host's responder).
    pub fn push_auth(&mut self, req: SshAuthRequest) {
        if self.auth.is_some() {
            self.auth_queue.push_back(req);
        } else {
            self.auth = Some(AuthPopup::new(req));
        }
    }

    /// Close the current credential popup and show the next queued one.
    pub fn next_auth(&mut self) {
        self.auth = self.auth_queue.pop_front().map(AuthPopup::new);
    }
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
            auth_queue: Default::default(),
            export: None,
            member_picker: None,
            reopen_name_picker: None,
            reopen_view_edit: None,
            progress_scroll: None,
            completed_report_scroll: 0,
        }
    }
}

#[cfg(test)]
mod auth_queue_tests {
    use super::*;

    fn req(p: &str) -> (SshAuthRequest, tokio::sync::oneshot::Receiver<String>) {
        let (responder, rx) = tokio::sync::oneshot::channel();
        (
            SshAuthRequest {
                prompt: p.into(),
                responder,
            },
            rx,
        )
    }

    #[test]
    fn second_request_is_queued_not_replacing_first() {
        let mut s = PopupState::new();
        let (r1, mut rx1) = req("first");
        let (r2, mut rx2) = req("second");
        s.push_auth(r1);
        s.push_auth(r2);
        assert_eq!(s.auth.as_ref().unwrap().prompt, "first");
        s.auth.as_mut().unwrap().input.value = "a".into();
        s.auth.as_mut().unwrap().submit();
        s.next_auth();
        assert_eq!(rx1.try_recv().unwrap(), "a", "first host got its answer");
        assert_eq!(s.auth.as_ref().unwrap().prompt, "second");
        assert!(rx2.try_recv().is_err(), "second still pending, not dropped");
        s.auth.as_mut().unwrap().cancel();
        s.next_auth();
        assert!(s.auth.is_none());
    }

    #[test]
    fn auth_popup_keeps_no_copies_and_wipes_on_close() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let key = |c, m| KeyEvent::new(c, m);
        for submit in [false, true] {
            let (r, mut rx) = req("p");
            let mut p = AuthPopup::new(r);
            let cap = p.input.value.capacity();
            for ch in "Zq7-b2-SECRET".chars() {
                p.input
                    .handle_key(key(KeyCode::Char(ch), KeyModifiers::NONE));
            }
            assert_eq!(p.input.value.capacity(), cap, "buffer reallocated");
            p.input
                .handle_key(key(KeyCode::Char('w'), KeyModifiers::CONTROL)); // kill word
            p.input
                .handle_key(key(KeyCode::Char('y'), KeyModifiers::CONTROL)); // yank: nothing kept
            p.input
                .handle_key(key(KeyCode::Char('_'), KeyModifiers::CONTROL)); // undo: nothing kept
            let dbg = format!("{:?}", p.input);
            assert!(!dbg.contains("SECRET"), "history kept a copy: {dbg}");
            if submit {
                p.submit();
                assert!(rx.try_recv().is_ok());
            } else {
                p.cancel();
            }
            assert!(p.input.value.is_empty() && p.input.saved.is_empty());
        }
    }
}
