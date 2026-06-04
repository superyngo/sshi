//! Operate tab — operation selection, parameter panels, target filter, execution.
//!
//! Phase 5 scope: check/run/exec/sync operations with param panels.
//! Phase 3 adds: applicable entries panel with scroll, ad-hoc banner, conflict detection.

use std::collections::HashMap;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::commands::report::HostStatus;
use crate::config::schema::{AppConfig, SyncEntry};

use super::super::components::input_field::InputField;
use super::super::state::persist::{
    OperationKind, ShellMode, SyncMode, TargetFilterMode, TargetFilterState,
};
use super::super::theme::Theme;

/// A single focusable element on the Operate tab, walked linearly with ↑↓.
///
/// The order is computed per-operation by [`operate_fields`]; the same list
/// drives both rendering focus and keyboard navigation so the two never drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpField {
    /// Operation radio (check/run/exec/sync) — ←→ changes the operation.
    OpRadio,
    // ── Common zone (shared across operations) ──
    /// Target mode radio (All/Groups/Hosts/Shell) — ←→ changes the mode.
    TargetMode,
    /// Group/host/shell membership — Enter opens the picker.
    TargetMembers,
    /// Skip-host list — Enter opens the picker.
    Skip,
    /// Serial execution toggle — Space toggles.
    Serial,
    /// Dry-run toggle — Space (or `d`) toggles.
    DryRun,
    /// Per-host timeout — ←→ adjusts by ±5s.
    Timeout,
    // ── Per-command zone ──
    /// run: command text. Enter activates the input.
    Command,
    /// exec: script path. Enter activates the input.
    Script,
    /// run/exec: sudo toggle.
    Sudo,
    /// exec: --keep toggle.
    Keep,
    /// sync: config/ad-hoc mode — ←→ toggles.
    SyncModeToggle,
    /// sync ad-hoc: add-path input.
    SyncAdhocInput,
    /// sync: source override input.
    SyncSource,
    /// cp: local path input (file/dir/wildcard).
    CpLocal,
    /// cp: remote destination input (optional).
    CpRemote,
    /// `-o/--out` report path (all operations).
    Out,
    /// Read-only applicable [[check]]/[[sync]] entries (scrolls with ↑↓).
    Entries,
    /// Execute button.
    Execute,
}

/// Whether the operation has a scrollable applicable-entries panel right now.
pub fn has_entries(op: OperationKind, sync_mode: SyncMode, config: &AppConfig) -> bool {
    match op {
        OperationKind::Check => !config.check.is_empty(),
        OperationKind::Sync => sync_mode == SyncMode::ConfigEntries && !config.sync.is_empty(),
        _ => false,
    }
}

/// Ordered list of focusable fields for the current operation/mode.
pub fn operate_fields(
    op: OperationKind,
    target_mode: TargetFilterMode,
    sync_mode: SyncMode,
    config: &AppConfig,
) -> Vec<OpField> {
    let mut v = vec![OpField::OpRadio, OpField::TargetMode];
    if target_mode != TargetFilterMode::All {
        v.push(OpField::TargetMembers);
    }
    v.push(OpField::Skip);
    v.push(OpField::Timeout);
    v.push(OpField::Serial);
    v.push(OpField::DryRun);
    // -o/--out lives at the bottom of the Common zone (applies to every op).
    v.push(OpField::Out);
    match op {
        OperationKind::Check => {}
        OperationKind::Run => {
            v.push(OpField::Command);
            v.push(OpField::Sudo);
        }
        OperationKind::Exec => {
            v.push(OpField::Script);
            v.push(OpField::Sudo);
            v.push(OpField::Keep);
        }
        OperationKind::Sync => {
            // Order must match the render order: mode, then Source override
            // (anchored on top), then the ad-hoc add-path input below it.
            v.push(OpField::SyncModeToggle);
            v.push(OpField::SyncSource);
            if sync_mode == SyncMode::AdHoc {
                v.push(OpField::SyncAdhocInput);
            }
        }
        OperationKind::Cp => {
            v.push(OpField::CpLocal);
            v.push(OpField::CpRemote);
        }
    }
    if has_entries(op, sync_mode, config) {
        v.push(OpField::Entries);
    }
    v.push(OpField::Execute);
    v
}

/// Focus layers for the Operate tab (§8.6 layer model). Tab/BackTab cycle
/// peers *within* a layer; arrow keys cross layer boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpLayer {
    /// The operation radio row.
    Op,
    /// Shared settings (target/skip/timeout/serial/dry-run/out).
    Common,
    /// Per-operation fields (command/script/sudo/keep/sync inputs).
    CommandSpecific,
    /// Read-only applicable-entries panel.
    Entries,
    /// Execute button.
    Execute,
}

/// Which layer a focusable field belongs to.
pub fn layer_of(field: OpField) -> OpLayer {
    match field {
        OpField::OpRadio => OpLayer::Op,
        OpField::TargetMode
        | OpField::TargetMembers
        | OpField::Skip
        | OpField::Serial
        | OpField::DryRun
        | OpField::Timeout
        | OpField::Out => OpLayer::Common,
        OpField::Command
        | OpField::Script
        | OpField::Sudo
        | OpField::Keep
        | OpField::SyncModeToggle
        | OpField::SyncAdhocInput
        | OpField::SyncSource
        | OpField::CpLocal
        | OpField::CpRemote => OpLayer::CommandSpecific,
        OpField::Entries => OpLayer::Entries,
        OpField::Execute => OpLayer::Execute,
    }
}

/// Rendering data for the Operate tab, passed from App.
pub struct OperateRenderData<'a> {
    pub focus: OpField,
    pub operation: OperationKind,
    pub sync_mode: SyncMode,
    pub dry_run: bool,
    pub sync_adhoc_files: &'a [String],
    pub sync_adhoc_input: &'a InputField,
    pub sync_source_input: &'a InputField,
    pub run_command: &'a InputField,
    pub exec_script: &'a InputField,
    pub cp_local_input: &'a InputField,
    pub cp_remote_input: &'a InputField,
    pub out_input: &'a InputField,
    pub run_sudo: bool,
    pub exec_sudo: bool,
    pub exec_keep: bool,
    pub entries_scroll: usize,
    pub config: &'a AppConfig,
    pub theme: &'a Theme,
    pub is_running: bool,
    pub target_filter: &'a TargetFilterState,
    pub target_count: usize,
    pub navbar_focused: bool,
}

/// Highlight a focused field. When `active` (the tab itself holds focus) the
/// field is reverse-highlighted; when focus has moved up to the NavBar it is
/// shown bold/accent only, so the panel visibly relinquishes focus.
fn focus_style(focused: bool, active: bool, theme: &Theme) -> Style {
    if focused && active {
        Style::default()
            .fg(theme.accent_operate)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else if focused {
        Style::default()
            .fg(theme.accent_operate)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

fn shell_label(s: ShellMode) -> &'static str {
    match s {
        ShellMode::Sh => "sh",
        ShellMode::PowerShell => "powershell",
        ShellMode::Cmd => "cmd",
    }
}

/// One renderable row in the fixed (non-entries) region of the tab.
enum RowItem<'a> {
    /// A single text line.
    Plain(Line<'a>),
    /// A bordered text input (occupies 3 rows).
    Field(&'a InputField, &'a str, bool),
}

impl RowItem<'_> {
    fn height(&self) -> u16 {
        match self {
            RowItem::Plain(_) => 1,
            RowItem::Field(..) => 3,
        }
    }
}

/// Render the entire Operate tab.
#[allow(clippy::vec_init_then_push)] // rows are appended conditionally per op
pub fn render_operate(data: &OperateRenderData, area: Rect, frame: &mut Frame) {
    // Config-style layout: no outer wrapper. The body block (" Operate ") holds
    // OpRadio/Common/Command-specific/Entries; Execute is its own block below.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(3)])
        .split(area);
    let body_area = outer[0];
    let exec_area = outer[1];

    let body_focused = !data.navbar_focused && data.focus != OpField::Execute;
    let border_col = if body_focused {
        data.theme.accent_operate // Operate identity colour (cyan)
    } else {
        data.theme.border_inactive
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_col))
        .title(" Operate ");
    let inner = block.inner(body_area);
    frame.render_widget(block, body_area);

    let theme = data.theme;
    let active = !data.navbar_focused;
    let tf = data.target_filter;
    let mut rows: Vec<RowItem> = Vec::new();

    // ── Operation radio ──
    rows.push(RowItem::Plain(op_radio_line(data)));
    rows.push(RowItem::Plain(Line::from("")));

    // ── Common zone ──
    rows.push(RowItem::Plain(zone_header("── Common ──", theme)));
    rows.push(RowItem::Plain(target_mode_line(data)));
    if tf.mode != TargetFilterMode::All {
        rows.push(RowItem::Plain(members_line(data)));
    }
    rows.push(RowItem::Plain(skip_line(data)));
    rows.push(RowItem::Plain(timeout_line(data)));
    rows.push(RowItem::Plain(serial_line(data)));
    rows.push(RowItem::Plain(dry_run_line(data)));
    rows.push(RowItem::Field(
        data.out_input,
        "Output report (.json/.html, optional)",
        data.focus == OpField::Out,
    ));
    rows.push(RowItem::Plain(Line::from("")));

    // ── Per-command zone ──
    let op_label = op_name(data.operation);
    rows.push(RowItem::Plain(zone_header(
        &format!("── {op_label} params ──"),
        theme,
    )));
    match data.operation {
        OperationKind::Check => {
            rows.push(RowItem::Plain(Line::from(Span::styled(
                "  (none)",
                Style::default().fg(theme.inactive),
            ))));
        }
        OperationKind::Run => {
            rows.push(RowItem::Field(
                data.run_command,
                "Command",
                data.focus == OpField::Command,
            ));
            rows.push(RowItem::Plain(toggle_line(
                "sudo",
                data.run_sudo,
                data.focus == OpField::Sudo,
                active,
                theme,
            )));
        }
        OperationKind::Exec => {
            rows.push(RowItem::Field(
                data.exec_script,
                "Script path",
                data.focus == OpField::Script,
            ));
            rows.push(RowItem::Plain(toggle_line(
                "sudo",
                data.exec_sudo,
                data.focus == OpField::Sudo,
                active,
                theme,
            )));
            rows.push(RowItem::Plain(toggle_line(
                "--keep",
                data.exec_keep,
                data.focus == OpField::Keep,
                active,
                theme,
            )));
        }
        OperationKind::Sync => {
            rows.push(RowItem::Plain(sync_mode_line(data)));
            // Source override stays anchored on top for visual stability; the
            // ad-hoc add-path input + file list appear below it.
            rows.push(RowItem::Field(
                data.sync_source_input,
                "Source override (optional)",
                data.focus == OpField::SyncSource,
            ));
            if data.sync_mode == SyncMode::AdHoc {
                rows.push(RowItem::Field(
                    data.sync_adhoc_input,
                    "Add path",
                    data.focus == OpField::SyncAdhocInput,
                ));
                if data.sync_adhoc_files.is_empty() {
                    rows.push(RowItem::Plain(Line::from(Span::styled(
                        "  (no paths)",
                        Style::default().fg(theme.inactive),
                    ))));
                } else {
                    for p in data.sync_adhoc_files.iter().rev().take(4) {
                        rows.push(RowItem::Plain(Line::from(format!("  · {p}"))));
                    }
                }
            }
        }
        OperationKind::Cp => {
            rows.push(RowItem::Field(
                data.cp_local_input,
                "Local path (file / dir / 'dir/*.ext')",
                data.focus == OpField::CpLocal,
            ));
            rows.push(RowItem::Field(
                data.cp_remote_input,
                "Remote destination (optional, defaults to ~)",
                data.focus == OpField::CpRemote,
            ));
        }
    }

    // ── Layout: fixed rows, entries (Min 0), execute bar (2) ──
    let mut constraints: Vec<Constraint> = rows
        .iter()
        .map(|r| Constraint::Length(r.height()))
        .collect();
    constraints.push(Constraint::Min(0)); // entries
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (i, row) in rows.iter().enumerate() {
        match row {
            RowItem::Plain(line) => {
                frame.render_widget(Paragraph::new(line.clone()), chunks[i]);
            }
            RowItem::Field(field, label, focused) => {
                field.render(frame, chunks[i], label, *focused);
            }
        }
    }
    let entries_area = chunks[rows.len()];
    render_applicable_entries(data, entries_area, frame);
    render_execute_bar(data, exec_area, frame);
}

fn zone_header<'a>(text: &str, theme: &Theme) -> Line<'a> {
    Line::from(Span::styled(
        format!(" {text}"),
        Style::default()
            .fg(theme.inactive)
            .add_modifier(Modifier::BOLD),
    ))
}

fn op_name(op: OperationKind) -> &'static str {
    match op {
        OperationKind::Check => "check",
        OperationKind::Run => "run",
        OperationKind::Exec => "exec",
        OperationKind::Sync => "sync",
        OperationKind::Cp => "cp",
    }
}

/// Style for a selected radio/option, dimming to bold-only when focus has
/// moved to the NavBar (`active == false`).
fn radio_style(selected: bool, focused: bool, active: bool, theme: &Theme) -> Style {
    if selected && focused && active {
        Style::default()
            .fg(theme.accent_operate)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else if selected {
        Style::default()
            .fg(theme.accent_operate)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.inactive)
    }
}

fn op_radio_line<'a>(data: &OperateRenderData) -> Line<'a> {
    let focused = data.focus == OpField::OpRadio;
    let active = !data.navbar_focused;
    let ops = [
        (OperationKind::Run, "run"),
        (OperationKind::Exec, "exec"),
        (OperationKind::Sync, "sync"),
        (OperationKind::Cp, "cp"),
        (OperationKind::Check, "check"),
    ];
    let mut spans = vec![Span::raw(" Operation: ")];
    for (kind, label) in ops {
        let selected = kind == data.operation;
        let prefix = if selected { "◉ " } else { "○ " };
        spans.push(Span::styled(
            format!("[{prefix}{label}]"),
            radio_style(selected, focused, active, data.theme),
        ));
        spans.push(Span::raw("  "));
    }
    Line::from(spans)
}

fn target_mode_line<'a>(data: &OperateRenderData) -> Line<'a> {
    let focused = data.focus == OpField::TargetMode;
    let active = !data.navbar_focused;
    let modes = [
        (TargetFilterMode::All, "All"),
        (TargetFilterMode::Groups, "Groups"),
        (TargetFilterMode::Hosts, "Hosts"),
        (TargetFilterMode::Shell, "Shell"),
    ];
    let mut spans = vec![Span::raw(" Target:  ")];
    for (m, label) in modes {
        let selected = m == data.target_filter.mode;
        let prefix = if selected { "◉ " } else { "○ " };
        spans.push(Span::styled(
            format!("{prefix}{label}"),
            radio_style(selected, focused, active, data.theme),
        ));
        spans.push(Span::raw("   "));
    }
    spans.push(Span::styled(
        format!("({} hosts)", data.target_count),
        Style::default().fg(data.theme.inactive),
    ));
    Line::from(spans)
}

fn members_line<'a>(data: &OperateRenderData) -> Line<'a> {
    let focused = data.focus == OpField::TargetMembers;
    let active = !data.navbar_focused;
    let tf = data.target_filter;
    let (label, value) = match tf.mode {
        TargetFilterMode::Groups => ("Members", chips(&tf.groups, "no groups")),
        TargetFilterMode::Hosts => ("Members", chips(&tf.hosts, "no hosts")),
        TargetFilterMode::Shell => ("Shell", shell_label(tf.shell).to_string()),
        TargetFilterMode::All => ("Members", String::new()),
    };
    Line::from(vec![
        Span::raw(format!(" {label}: ")),
        Span::styled(value, focus_style(focused, active, data.theme)),
    ])
}

fn skip_line<'a>(data: &OperateRenderData) -> Line<'a> {
    let focused = data.focus == OpField::Skip;
    let active = !data.navbar_focused;
    Line::from(vec![
        Span::raw(" Skip:    "),
        Span::styled(
            chips(&data.target_filter.skip, "none"),
            focus_style(focused, active, data.theme),
        ),
    ])
}

fn serial_line<'a>(data: &OperateRenderData) -> Line<'a> {
    let focused = data.focus == OpField::Serial;
    let active = !data.navbar_focused;
    let glyph = if data.target_filter.serial {
        "[✓] Serial (s)"
    } else {
        "[ ] Serial (s)"
    };
    Line::from(vec![
        Span::raw(" "),
        Span::styled(glyph, focus_style(focused, active, data.theme)),
    ])
}

fn dry_run_line<'a>(data: &OperateRenderData) -> Line<'a> {
    let focused = data.focus == OpField::DryRun;
    let active = !data.navbar_focused;
    let glyph = if data.dry_run {
        "[✓] dry-run (d)"
    } else {
        "[ ] dry-run (d)"
    };
    Line::from(vec![
        Span::raw(" "),
        Span::styled(glyph, focus_style(focused, active, data.theme)),
    ])
}

fn timeout_line<'a>(data: &OperateRenderData) -> Line<'a> {
    let focused = data.focus == OpField::Timeout;
    let active = !data.navbar_focused;
    Line::from(vec![
        Span::raw(" Timeout: "),
        Span::styled(
            format!("{}s", data.target_filter.timeout),
            focus_style(focused, active, data.theme),
        ),
    ])
}

fn toggle_line<'a>(label: &str, on: bool, focused: bool, active: bool, theme: &Theme) -> Line<'a> {
    let glyph = if on {
        format!("[✓] {label}")
    } else {
        format!("[ ] {label}")
    };
    Line::from(vec![
        Span::raw("  "),
        Span::styled(glyph, focus_style(focused, active, theme)),
    ])
}

fn sync_mode_line<'a>(data: &OperateRenderData) -> Line<'a> {
    let focused = data.focus == OpField::SyncModeToggle;
    let active = !data.navbar_focused;
    let (config_glyph, adhoc_glyph) = match data.sync_mode {
        SyncMode::ConfigEntries => ("◉", "○"),
        SyncMode::AdHoc => ("○", "◉"),
    };
    let style = focus_style(focused, active, data.theme);
    Line::from(vec![
        Span::raw("  Mode: "),
        Span::styled(format!("{config_glyph} Config entries"), style),
        Span::raw("   "),
        Span::styled(format!("{adhoc_glyph} Ad-hoc files"), style),
    ])
}

fn chips(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        format!("({empty})")
    } else {
        items.join(", ")
    }
}

fn render_execute_bar(data: &OperateRenderData, area: Rect, frame: &mut Frame) {
    let active = !data.navbar_focused;
    let exec_focused = data.focus == OpField::Execute;
    // Execute lives in its own bordered block; border lights up with the
    // Operate accent when focused (matches the Config per-zone convention).
    let border_col = if exec_focused && active {
        data.theme.accent_operate
    } else {
        data.theme.border_inactive
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_col))
        .title(" Execute ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let exec_label = if data.is_running {
        " [ running… — Esc to cancel ] ".to_string()
    } else {
        format!(" [ Execute {} (Enter) ] ", op_name(data.operation))
    };
    let exec_style = if exec_focused && active && !data.is_running {
        Style::default()
            .fg(data.theme.accent_operate)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else if exec_focused && !data.is_running {
        Style::default()
            .fg(data.theme.accent_operate)
            .add_modifier(Modifier::BOLD)
    } else if data.is_running {
        Style::default().fg(data.theme.warning)
    } else {
        Style::default().fg(data.theme.inactive)
    };
    // dry-run now lives in the Common zone; the bar is just the Execute button.
    let line = Line::from(vec![Span::styled(exec_label, exec_style)]);
    frame.render_widget(Paragraph::new(line), inner);
}

fn render_applicable_entries(data: &OperateRenderData, area: Rect, frame: &mut Frame) {
    let entries_focused = data.focus == OpField::Entries;
    let page_size = 6usize;

    if data.operation == OperationKind::Check {
        let mut lines: Vec<Line> = Vec::new();
        let total = data.config.check.len();
        let scroll = data.entries_scroll.min(total.saturating_sub(page_size));
        let scroll_hint = if total > page_size {
            format!(
                " [{}-{}/{}] ↑↓",
                scroll + 1,
                (scroll + page_size).min(total),
                total
            )
        } else {
            String::new()
        };
        let header_style = if entries_focused {
            Style::default()
                .fg(data.theme.accent_operate)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(data.theme.inactive)
        };
        lines.push(Line::from(Span::styled(
            format!("─ Applicable [[check]] entries ─{scroll_hint}"),
            header_style,
        )));
        if data.config.check.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no [[check]] entries — add one to config.toml)",
                Style::default().fg(data.theme.inactive),
            )));
        } else {
            for (i, entry) in data
                .config
                .check
                .iter()
                .enumerate()
                .skip(scroll)
                .take(page_size)
            {
                let label = entry
                    .name
                    .clone()
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("Check #{}", i + 1));
                let metrics = if entry.enabled.is_empty() {
                    "(no metrics)".to_string()
                } else {
                    entry.enabled.join(",")
                };
                let groups = if entry.groups.is_empty() {
                    "unscoped".to_string()
                } else {
                    format!("groups:[{}]", entry.groups.join(","))
                };
                lines.push(Line::from(format!(
                    "  ▸ {label} — {groups}  metrics:{metrics}"
                )));
            }
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    } else if data.operation == OperationKind::Sync && data.sync_mode == SyncMode::ConfigEntries {
        let mut lines: Vec<Line> = Vec::new();
        let total = data.config.sync.len();
        let scroll = data.entries_scroll.min(total.saturating_sub(page_size));
        let scroll_hint = if total > page_size {
            format!(
                " [{}-{}/{}] ↑↓",
                scroll + 1,
                (scroll + page_size).min(total),
                total
            )
        } else {
            String::new()
        };
        let header_style = if entries_focused {
            Style::default()
                .fg(data.theme.accent_operate)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(data.theme.inactive)
        };
        lines.push(Line::from(Span::styled(
            format!("─ Applicable [[sync]] entries ─{scroll_hint}"),
            header_style,
        )));
        let conflicts = detect_sync_source_conflicts(&data.config.sync);
        if !conflicts.is_empty() {
            lines.push(Line::from(Span::styled(
                format!(
                    "  ⚠ Source conflict: {} entries share the same source path",
                    conflicts.len()
                ),
                Style::default().fg(data.theme.warning),
            )));
        }
        if data.config.sync.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no [[sync]] entries — add one to config.toml)",
                Style::default().fg(data.theme.inactive),
            )));
        } else {
            for entry in data.config.sync.iter().skip(scroll).take(page_size) {
                let paths = entry
                    .paths
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                let groups = if entry.groups.is_empty() {
                    "unscoped".to_string()
                } else {
                    format!("groups:[{}]", entry.groups.join(","))
                };
                let src = entry
                    .source
                    .as_deref()
                    .map(|s| format!("  src:{s}"))
                    .unwrap_or_default();
                lines.push(Line::from(format!("  ▸ {paths}  {groups}{src}")));
            }
        }
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
    }
}

/// Render the progress popup showing running operation status.
#[allow(clippy::too_many_arguments)]
pub fn render_progress_popup(
    theme: &Theme,
    op_name: &str,
    host_outcomes: &[(String, HostStatus, String, u64)],
    targets: &[String],
    elapsed_secs: u64,
    completed_count: usize,
    progress_scroll: Option<usize>,
    area: Rect,
    frame: &mut Frame,
) {
    use super::super::components::popup::centered_rect;

    let popup_area = centered_rect(70, 70, area);
    frame.render_widget(ratatui::widgets::Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border_active))
        .title(format!(" Running {op_name} — Esc to cancel "));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let mut lines: Vec<Line> = Vec::new();
    let total_outcomes = host_outcomes.len();
    let take = 12usize;
    let auto_start = total_outcomes.saturating_sub(take);
    let start = progress_scroll.unwrap_or(auto_start).min(auto_start);
    let scroll_hint = if total_outcomes > take {
        format!("  [{}/{}] ↑↓ scroll", start + 1, total_outcomes)
    } else {
        String::new()
    };
    lines.push(Line::from(format!(
        "Targets: {}    Completed: {}    Elapsed: {}s{}",
        targets.len(),
        completed_count,
        elapsed_secs,
        scroll_hint,
    )));
    lines.push(Line::from(""));

    for (host, status, detail, ms) in &host_outcomes[start..(start + take).min(total_outcomes)] {
        let glyph = match status {
            HostStatus::Online => "✓",
            HostStatus::Partial => "⚠",
            HostStatus::Offline => "✗",
            HostStatus::Unreachable => "⊘",
            HostStatus::TimedOut => "⏱",
            HostStatus::Error => "✗",
            HostStatus::Skipped => "⊘",
        };
        let color = match status {
            HostStatus::Online => theme.accent_checkout,
            HostStatus::Partial => theme.warning,
            HostStatus::Skipped => theme.inactive,
            _ => theme.error,
        };
        let line = format!(
            "  {} {:<16} ({:>4}ms) — {}",
            glyph,
            truncate(host, 16),
            ms,
            truncate(detail, 60),
        );
        lines.push(Line::from(Span::styled(line, Style::default().fg(color))));
    }

    let p = Paragraph::new(lines).wrap(Wrap { trim: false });
    frame.render_widget(p, inner);
}

/// Detect sync entries that share the same (non-empty) source path.
pub fn detect_sync_source_conflicts(sync_entries: &[SyncEntry]) -> Vec<String> {
    let mut source_counts: HashMap<&str, usize> = HashMap::new();
    for entry in sync_entries {
        if let Some(src) = entry.source.as_deref() {
            if !src.is_empty() {
                *source_counts.entry(src).or_insert(0) += 1;
            }
        }
    }
    source_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(src, _)| src.to_string())
        .collect()
}

pub fn truncate(s: &str, max: usize) -> String {
    use unicode_width::UnicodeWidthStr;
    if s.width() <= max {
        return s.to_string();
    }
    let mut w = 0;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every OpField maps to a layer (exhaustive — a new variant without a
    /// mapping is a compile error in `layer_of`, this guards the values).
    #[test]
    fn layer_of_groups_fields() {
        assert_eq!(layer_of(OpField::OpRadio), OpLayer::Op);
        for f in [
            OpField::TargetMode,
            OpField::TargetMembers,
            OpField::Skip,
            OpField::Serial,
            OpField::DryRun,
            OpField::Timeout,
            OpField::Out,
        ] {
            assert_eq!(layer_of(f), OpLayer::Common, "{f:?} should be Common");
        }
        for f in [
            OpField::Command,
            OpField::Script,
            OpField::Sudo,
            OpField::Keep,
            OpField::SyncModeToggle,
            OpField::SyncAdhocInput,
            OpField::SyncSource,
        ] {
            assert_eq!(
                layer_of(f),
                OpLayer::CommandSpecific,
                "{f:?} should be CommandSpecific"
            );
        }
        assert_eq!(layer_of(OpField::Entries), OpLayer::Entries);
        assert_eq!(layer_of(OpField::Execute), OpLayer::Execute);
    }
}
