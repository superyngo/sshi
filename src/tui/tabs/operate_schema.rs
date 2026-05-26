//! Field schema for operation-specific params (Operate + View tabs).
//!
//! Produces `FieldDescriptor`s for each operation's toggle/enum/number params
//! and applies edits back via an `OpSpecific` state-view struct. Text inputs
//! (command, script, source, files, since, host, out) are handled by
//! `InputField` directly in the tab renderers.

use super::config_schema::{FieldDescriptor, FieldKind};
use crate::cli::ActionFilter;
use crate::tui::state::persist::{OperationKind, SyncMode, ViewOperationKind};

/// Mutable references to the operation-specific fields the schema reads/writes.
#[allow(dead_code)]
pub struct OpSpecific<'a> {
    pub sudo: &'a mut bool,
    pub keep: &'a mut bool,
    pub dry_run: &'a mut bool,
    pub sync_mode: &'a mut SyncMode,
    pub checkout_history: &'a mut bool,
    pub log_last: &'a mut usize,
    pub log_errors: &'a mut bool,
    pub log_action: &'a mut Option<ActionFilter>,
}

#[allow(dead_code)]
pub fn check_specific_fields(dry_run: bool) -> Vec<FieldDescriptor> {
    vec![FieldDescriptor::scalar(
        "dry_run",
        dry_run.to_string(),
        FieldKind::Bool,
    )]
}

#[allow(dead_code)]
pub fn run_specific_fields(sudo: bool, dry_run: bool) -> Vec<FieldDescriptor> {
    vec![
        FieldDescriptor::scalar("sudo", sudo.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("dry_run", dry_run.to_string(), FieldKind::Bool),
    ]
}

#[allow(dead_code)]
pub fn exec_specific_fields(sudo: bool, keep: bool, dry_run: bool) -> Vec<FieldDescriptor> {
    vec![
        FieldDescriptor::scalar("sudo", sudo.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("keep", keep.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar("dry_run", dry_run.to_string(), FieldKind::Bool),
    ]
}

#[allow(dead_code)]
pub fn sync_specific_fields(mode: SyncMode, dry_run: bool) -> Vec<FieldDescriptor> {
    let mode_str = match mode {
        SyncMode::ConfigEntries => "config",
        SyncMode::AdHoc => "adhoc",
    };
    vec![
        FieldDescriptor::scalar(
            "mode",
            mode_str.into(),
            FieldKind::Enum {
                variants: vec!["config", "adhoc"],
            },
        ),
        FieldDescriptor::scalar("dry_run", dry_run.to_string(), FieldKind::Bool),
    ]
}

#[allow(dead_code)]
pub fn checkout_specific_fields(history: bool) -> Vec<FieldDescriptor> {
    vec![FieldDescriptor::scalar(
        "history",
        history.to_string(),
        FieldKind::Bool,
    )]
}

#[allow(dead_code)]
pub fn action_str(action: Option<&ActionFilter>) -> &'static str {
    match action {
        None => "all",
        Some(ActionFilter::Sync) => "sync",
        Some(ActionFilter::Run) => "run",
        Some(ActionFilter::Exec) => "exec",
        Some(ActionFilter::Check) => "check",
    }
}

#[allow(dead_code)]
pub fn log_specific_fields(
    last: usize,
    errors: bool,
    action: Option<&ActionFilter>,
) -> Vec<FieldDescriptor> {
    vec![
        FieldDescriptor::scalar("last", last.to_string(), FieldKind::U64),
        FieldDescriptor::scalar("errors", errors.to_string(), FieldKind::Bool),
        FieldDescriptor::scalar(
            "action",
            action_str(action).into(),
            FieldKind::Enum {
                variants: vec!["all", "sync", "run", "exec", "check"],
            },
        ),
    ]
}

#[allow(dead_code)]
pub fn apply_specific(s: &mut OpSpecific, op: OperationKind, key: &str, val: &str) {
    match (op, key) {
        (OperationKind::Run | OperationKind::Exec, "sudo") => *s.sudo = val == "true",
        (OperationKind::Exec, "keep") => *s.keep = val == "true",
        (_, "dry_run") => *s.dry_run = val == "true",
        (OperationKind::Sync, "mode") => {
            *s.sync_mode = if val == "adhoc" {
                SyncMode::AdHoc
            } else {
                SyncMode::ConfigEntries
            };
        }
        _ => {}
    }
}

#[allow(dead_code)]
pub fn apply_view_specific(view_op: ViewOperationKind, s: &mut OpSpecific, key: &str, val: &str) {
    match (view_op, key) {
        (ViewOperationKind::Checkout, "history") => *s.checkout_history = val == "true",
        (ViewOperationKind::Log, "last") => {
            if let Ok(v) = val.parse::<usize>() {
                *s.log_last = v;
            }
        }
        (ViewOperationKind::Log, "errors") => *s.log_errors = val == "true",
        (ViewOperationKind::Log, "action") => {
            *s.log_action = match val {
                "sync" => Some(ActionFilter::Sync),
                "run" => Some(ActionFilter::Run),
                "exec" => Some(ActionFilter::Exec),
                "check" => Some(ActionFilter::Check),
                _ => None,
            };
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scratch state backing an `OpSpecific` for tests.
    #[derive(Default)]
    struct Scratch {
        sudo: bool,
        keep: bool,
        dry_run: bool,
        sync_mode: SyncMode,
        checkout_history: bool,
        log_last: usize,
        log_errors: bool,
        log_action: Option<ActionFilter>,
    }

    impl Scratch {
        fn view(&mut self) -> OpSpecific<'_> {
            OpSpecific {
                sudo: &mut self.sudo,
                keep: &mut self.keep,
                dry_run: &mut self.dry_run,
                sync_mode: &mut self.sync_mode,
                checkout_history: &mut self.checkout_history,
                log_last: &mut self.log_last,
                log_errors: &mut self.log_errors,
                log_action: &mut self.log_action,
            }
        }
    }

    #[test]
    fn check_specific_fields_reflects_dry_run() {
        let f = check_specific_fields(true);
        let d = f.iter().find(|d| d.key == "dry_run").unwrap();
        assert_eq!(d.display_value, "true");
    }

    #[test]
    fn apply_run_sudo_toggles() {
        let mut sc = Scratch::default();
        apply_specific(&mut sc.view(), OperationKind::Run, "sudo", "true");
        assert!(sc.sudo);
    }

    #[test]
    fn apply_exec_keep_and_dry_run() {
        let mut sc = Scratch::default();
        apply_specific(&mut sc.view(), OperationKind::Exec, "keep", "true");
        apply_specific(&mut sc.view(), OperationKind::Exec, "dry_run", "true");
        assert!(sc.keep);
        assert!(sc.dry_run);
    }

    #[test]
    fn apply_sync_mode_enum() {
        let mut sc = Scratch::default();
        apply_specific(&mut sc.view(), OperationKind::Sync, "mode", "adhoc");
        assert_eq!(sc.sync_mode, SyncMode::AdHoc);
        apply_specific(&mut sc.view(), OperationKind::Sync, "mode", "config");
        assert_eq!(sc.sync_mode, SyncMode::ConfigEntries);
    }

    #[test]
    fn apply_specific_unknown_key_is_noop() {
        let mut sc = Scratch::default();
        apply_specific(&mut sc.view(), OperationKind::Run, "bogus", "true");
        assert!(!sc.sudo && !sc.dry_run);
    }

    #[test]
    fn apply_view_checkout_history_and_log_fields() {
        let mut sc = Scratch::default();
        apply_view_specific(ViewOperationKind::Checkout, &mut sc.view(), "history", "true");
        apply_view_specific(ViewOperationKind::Log, &mut sc.view(), "last", "50");
        apply_view_specific(ViewOperationKind::Log, &mut sc.view(), "errors", "true");
        apply_view_specific(ViewOperationKind::Log, &mut sc.view(), "action", "exec");
        assert!(sc.checkout_history);
        assert_eq!(sc.log_last, 50);
        assert!(sc.log_errors);
        assert!(matches!(sc.log_action, Some(ActionFilter::Exec)));
    }

    #[test]
    fn apply_view_log_action_all_clears() {
        let mut sc = Scratch {
            log_action: Some(ActionFilter::Run),
            ..Default::default()
        };
        apply_view_specific(ViewOperationKind::Log, &mut sc.view(), "action", "all");
        assert!(sc.log_action.is_none());
    }

    #[test]
    fn log_action_round_trips_through_descriptor() {
        let fields = log_specific_fields(10, false, Some(&ActionFilter::Sync));
        let action = fields.iter().find(|d| d.key == "action").unwrap();
        assert_eq!(action.display_value, "sync");
    }
}
