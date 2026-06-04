//! View tab — checkout / list / log result renderers.
//!
//! Pure rendering functions that take `ViewRenderData` and draw into ratatui
//! frames. No side effects; all data is passed in from App.

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use crate::commands::checkout::{
    extract_metric_value, format_relative_time, metric_header, metric_width, DisplayColumns,
    HostSnapshot,
};
use crate::commands::list::ListData;
use crate::commands::log::LogRow;
use crate::tui::components::input_field::InputField;

use super::super::state::persist::{TargetFilterMode, TargetFilterState, ViewOperationKind};
use super::super::theme::Theme;

/// All data needed to render the View tab — pure snapshot of App state.
pub struct ViewRenderData<'a> {
    pub view_op: ViewOperationKind,
    pub theme: &'a Theme,
    pub navbar_focused: bool,
    pub loading: bool,
    /// Checkout result payload (host snapshots + display columns).
    pub checkout: Option<(&'a [HostSnapshot], &'a DisplayColumns)>,
    /// List result payload.
    pub list: Option<&'a ListData>,
    /// Log result payload.
    pub log: Option<&'a [LogRow]>,
    /// Number of lines to skip at the top of the result area (scroll offset).
    pub result_scroll: usize,
    /// Selected row index for the Checkout table (highlighted with ▶ + reverse).
    pub checkout_selected: usize,
    /// Log specific-params: text inputs.
    pub log_last_input: &'a InputField,
    pub log_since_input: &'a InputField,
    pub log_host_input: &'a InputField,
    /// Log specific-params: errors checkbox state.
    pub log_errors: bool,
    /// Log specific-params: action display string ("all", "sync", etc.).
    pub log_action: &'static str,
    /// Which specific field index (0..4) is focused, or None.
    pub specific_focused: Option<usize>,
    /// True when the Op selector row holds the focus cursor (reverse-video);
    /// otherwise it shows bold/accent only (focus principle).
    pub op_selector_focused: bool,
    /// True when the result list holds the focus cursor.
    pub result_focused: bool,
    /// Active target filter (drives the inline Common zone for Checkout/List).
    pub target_filter: &'a TargetFilterState,
    /// Number of hosts the target filter currently resolves to.
    pub target_count: usize,
    /// Which Common-zone target field holds the focus cursor, if any.
    pub target_mode_focused: bool,
    pub target_members_focused: bool,
    pub skip_focused: bool,
}

/// Entry point: render the entire View tab into `area`.
#[allow(dead_code)]
pub fn render_view(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    let is_log = data.view_op == ViewOperationKind::Log;
    // Log has no target zone; Checkout/List get the inline Common zone
    // (mode row, optional members row, skip row) that replaced the `f` popup.
    let target_height: u16 = if is_log {
        1 // greyed "log has no target" summary
    } else if data.target_filter.mode != TargetFilterMode::All {
        3
    } else {
        2
    };
    let specific_height: u16 = if is_log { 5 } else { 0 };
    // Config-style layout: no outer wrapper. The " View " block holds the op
    // selector (2 rows) + target/common (+ Log-specific); Results is its own
    // block below.
    let settings_inner_h = 2 + target_height + specific_height;
    let settings_block_h = settings_inner_h + 2;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(settings_block_h), // " View " block (bordered)
            Constraint::Min(0),                   // Results block (bordered)
        ])
        .split(area);

    // ── " View " settings block (op selector + target/common) ──
    let settings_focused = !data.navbar_focused
        && (data.op_selector_focused
            || data.target_mode_focused
            || data.target_members_focused
            || data.skip_focused
            || data.specific_focused.is_some());
    let s_border = if settings_focused {
        data.theme.accent_checkout
    } else {
        data.theme.border_inactive
    };
    let s_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(s_border))
        .title(" View ");
    let s_inner = s_block.inner(chunks[0]);
    frame.render_widget(s_block, chunks[0]);
    let s_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),               // op selector
            Constraint::Length(target_height),   // target / common (or Log summary)
            Constraint::Length(specific_height), // Log-specific params (0 otherwise)
        ])
        .split(s_inner);
    render_view_selector(data, s_chunks[0], frame);
    if is_log {
        render_view_target_summary(data, s_chunks[1], frame);
        render_log_specific_params(data, s_chunks[2], frame);
    } else {
        render_view_common(data, s_chunks[1], frame);
    }

    // ── Results block ──
    let results_focused = !data.navbar_focused && data.result_focused;
    let r_border = if results_focused {
        data.theme.accent_checkout
    } else {
        data.theme.border_inactive
    };
    let r_title = match data.view_op {
        ViewOperationKind::Checkout => " Checkout ",
        ViewOperationKind::List => " List ",
        ViewOperationKind::Log => " Log ",
    };
    let r_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(r_border))
        .title(r_title);
    let r_inner = r_block.inner(chunks[1]);
    frame.render_widget(r_block, chunks[1]);
    render_result_area(data, r_inner, frame);
}

/// Inline Common zone for Checkout/List: target mode radio, optional members
/// row, and skip row — the flat replacement for the old filter popup.
fn render_view_common(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    let tf = data.target_filter;
    let mut rows: Vec<Line> = vec![view_target_mode_line(data)];
    if tf.mode != TargetFilterMode::All {
        rows.push(view_members_line(data));
    }
    rows.push(view_skip_line(data));
    let constraints: Vec<Constraint> = rows.iter().map(|_| Constraint::Length(1)).collect();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);
    for (i, line) in rows.into_iter().enumerate() {
        frame.render_widget(Paragraph::new(line), chunks[i]);
    }
}

fn view_focus_style(focused: bool, theme: &Theme) -> Style {
    if focused {
        Style::default()
            .fg(theme.accent_checkout)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::default()
    }
}

fn view_target_mode_line<'a>(data: &ViewRenderData) -> Line<'a> {
    let focused = data.target_mode_focused;
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
        let style = if selected && focused {
            Style::default()
                .fg(data.theme.accent_checkout)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else if selected {
            Style::default()
                .fg(data.theme.accent_checkout)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(data.theme.inactive)
        };
        spans.push(Span::styled(format!("{prefix}{label}"), style));
        spans.push(Span::raw("   "));
    }
    spans.push(Span::styled(
        format!("({} hosts)", data.target_count),
        Style::default().fg(data.theme.inactive),
    ));
    Line::from(spans)
}

fn view_members_line<'a>(data: &ViewRenderData) -> Line<'a> {
    let tf = data.target_filter;
    let (label, value) = match tf.mode {
        TargetFilterMode::Groups => ("Members", view_chips(&tf.groups, "no groups")),
        TargetFilterMode::Hosts => ("Members", view_chips(&tf.hosts, "no hosts")),
        TargetFilterMode::Shell => ("Shell", shell_label(tf.shell).to_string()),
        TargetFilterMode::All => ("Members", String::new()),
    };
    Line::from(vec![
        Span::raw(format!(" {label}: ")),
        Span::styled(
            value,
            view_focus_style(data.target_members_focused, data.theme),
        ),
    ])
}

fn shell_label(s: super::super::state::persist::ShellMode) -> &'static str {
    use super::super::state::persist::ShellMode;
    match s {
        ShellMode::Sh => "sh",
        ShellMode::PowerShell => "powershell",
        ShellMode::Cmd => "cmd",
    }
}

fn view_skip_line<'a>(data: &ViewRenderData) -> Line<'a> {
    Line::from(vec![
        Span::raw(" Skip:    "),
        Span::styled(
            view_chips(&data.target_filter.skip, "none"),
            view_focus_style(data.skip_focused, data.theme),
        ),
    ])
}

fn view_chips(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        format!("({empty})")
    } else {
        items.join(", ")
    }
}

/// Horizontal radio selector for checkout / list / log.
#[allow(dead_code)]
fn render_view_selector(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    let ops = [
        (ViewOperationKind::Checkout, "Checkout"),
        (ViewOperationKind::List, "List"),
        (ViewOperationKind::Log, "Log"),
    ];

    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::raw(" Op: "));
    for (op, label) in &ops {
        let selected = *op == data.view_op;
        let style = if selected && data.op_selector_focused {
            Style::default()
                .fg(data.theme.accent_checkout)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else if selected {
            Style::default()
                .fg(data.theme.accent_checkout)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(data.theme.inactive)
        };
        spans.push(Span::styled(format!(" {} ", label), style));
        spans.push(Span::raw("  "));
    }
    spans.push(Span::styled(
        " ←/→ to switch ",
        Style::default().fg(data.theme.inactive),
    ));

    let line = Line::from(spans);
    let p = Paragraph::new(line);
    frame.render_widget(p, area);
}

/// One-line greyed note shown for Log (which has no target filter).
fn render_view_target_summary(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    let count = data.log.map(|r| r.len()).unwrap_or(0);
    let text = Span::styled(
        format!(
            " Log: {count} entr{} below (all hosts) — ↑↓/Tab scroll · Enter edits a field · Space toggles errors/action",
            if count == 1 { "y" } else { "ies" }
        ),
        Style::default().fg(data.theme.inactive),
    );
    frame.render_widget(Paragraph::new(Line::from(vec![text])), area);
}

/// Render the 5-row Log specific-params panel (last, errors, action, since,
/// host). Every field is a uniform single line — the focused field's value is
/// reverse-highlighted (focus principle) and text inputs show an inline cursor
/// when active, rather than drawing a bordered box into a 1-row slot.
fn render_log_specific_params(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1); 5])
        .split(area);

    let f = |i: usize| data.specific_focused == Some(i);
    let lines = [
        log_input_line("last", data.log_last_input, f(0), data.theme),
        log_value_line(
            "errors",
            if data.log_errors { "[x]" } else { "[ ]" },
            f(1),
            data.theme,
        ),
        log_value_line("action", data.log_action, f(2), data.theme),
        log_input_line("since", data.log_since_input, f(3), data.theme),
        log_input_line("host", data.log_host_input, f(4), data.theme),
    ];
    for (i, line) in lines.into_iter().enumerate() {
        frame.render_widget(Paragraph::new(line), rows[i]);
    }
}

/// A `label: value` line for a non-input log field (errors / action).
fn log_value_line<'a>(label: &str, value: &str, focused: bool, theme: &Theme) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!(" {label}: "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(value.to_string(), view_focus_style(focused, theme)),
    ])
}

/// A `label: value` line for a text-input log field, with an inline cursor when
/// the field is active.
fn log_input_line<'a>(
    label: &str,
    input: &'a InputField,
    focused: bool,
    theme: &Theme,
) -> Line<'a> {
    let label_span = Span::styled(
        format!(" {label}: "),
        Style::default().add_modifier(Modifier::BOLD),
    );
    if input.mode == crate::tui::components::input_field::InputMode::Active {
        let (before, after) = input.split_at_cursor();
        let cursor_ch = after.chars().next().unwrap_or(' ').to_string();
        let rest: String = after.chars().skip(1).collect();
        Line::from(vec![
            label_span,
            Span::raw(before.to_string()),
            Span::styled(
                cursor_ch,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(rest),
        ])
    } else {
        let value = if input.value.is_empty() {
            "(empty)".to_string()
        } else {
            input.value.clone()
        };
        Line::from(vec![
            label_span,
            Span::styled(value, view_focus_style(focused, theme)),
        ])
    }
}

/// Dispatch to the appropriate result renderer.
#[allow(dead_code)]
fn render_result_area(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    if data.loading {
        let p = Paragraph::new(Span::styled(
            "  loading…",
            Style::default().fg(data.theme.inactive),
        ));
        frame.render_widget(p, area);
        return;
    }
    match data.view_op {
        ViewOperationKind::Checkout => render_checkout_result(data, area, frame),
        ViewOperationKind::List => render_list_result(data, area, frame),
        ViewOperationKind::Log => render_log_result(data, area, frame),
    }
}

// ── Checkout result ──────────────────────────────────────────────────────────

/// Render the checkout snapshot table, mirroring `App::render_checkout`.
#[allow(dead_code)]
pub fn render_checkout_result(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    let (snapshots, columns) = match data.checkout {
        Some(pair) => pair,
        None => {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "  No checkout data.",
                    Style::default().fg(data.theme.inactive),
                )),
                area,
            );
            return;
        }
    };

    if snapshots.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  No snapshots available. Run `sshi check --all` to populate the database.",
                Style::default().fg(data.theme.inactive),
            )),
            area,
        );
        return;
    }

    let mut header_cells: Vec<Cell> = vec![Cell::from("Host"), Cell::from("Status")];
    let mut constraints: Vec<Constraint> = vec![Constraint::Length(16), Constraint::Length(12)];
    for metric in &columns.metrics {
        header_cells.push(Cell::from(metric_header(metric)));
        constraints.push(Constraint::Length(metric_width(metric) as u16));
    }
    header_cells.push(Cell::from("Last Seen"));
    constraints.push(Constraint::Min(10));

    let visible_height = area.height.saturating_sub(1) as usize; // minus header
    let start = data.result_scroll.min(snapshots.len().saturating_sub(1));
    let end = (start + visible_height).min(snapshots.len());

    let mut rows: Vec<Row> = Vec::new();
    for (i, snap) in snapshots[start..end].iter().enumerate() {
        let selected = start + i == data.checkout_selected;
        let status_text = if snap.online {
            "✓ online"
        } else {
            "✗ offline"
        };
        let status_style = Style::default().fg(if snap.online {
            data.theme.accent_checkout
        } else {
            data.theme.error
        });

        let host_cell = if selected && data.result_focused {
            format!("▶ {}", snap.host)
        } else if selected {
            format!("> {}", snap.host)
        } else {
            format!("  {}", snap.host)
        };
        let mut cells: Vec<Cell> = vec![
            Cell::from(host_cell),
            Cell::from(status_text).style(status_style),
        ];
        for metric in &columns.metrics {
            let (val, critical) = extract_metric_value(&snap.data, metric);
            let style = if critical {
                Style::default().fg(data.theme.error)
            } else {
                Style::default()
            };
            cells.push(Cell::from(val).style(style));
        }
        cells.push(Cell::from(format_relative_time(snap.last_online)));
        let mut row = Row::new(cells);
        if selected && data.result_focused {
            row = row.style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED));
        } else if selected {
            row = row.style(Style::default().add_modifier(Modifier::BOLD));
        }
        rows.push(row);
    }

    let table = Table::new(rows, &constraints)
        .header(Row::new(header_cells).style(Style::default().add_modifier(Modifier::BOLD)));
    frame.render_widget(table, area);
}

// ── List result ───────────────────────────────────────────────────────────────

/// Render `ListData` mirroring the text layout of `list::run`.
#[allow(dead_code)]
pub fn render_list_result(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    let list_data = match data.list {
        Some(d) => d,
        None => {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "  No list data.",
                    Style::default().fg(data.theme.inactive),
                )),
                area,
            );
            return;
        }
    };

    let mut lines: Vec<Line> = Vec::new();

    // ── Hosts ──
    lines.push(Line::from(Span::styled(
        format!("── Hosts ({}) ──", list_data.hosts.len()),
        Style::default()
            .fg(data.theme.accent_checkout)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::raw(format!(
        "  {:<16} {:<20} {:<12} Groups",
        "Name", "SSH Host", "Shell"
    ))));
    lines.push(Line::from(Span::styled(
        format!("  {}", "-".repeat(64)),
        Style::default().fg(data.theme.inactive),
    )));
    for h in &list_data.hosts {
        let groups = if h.groups.is_empty() {
            "-".to_string()
        } else {
            h.groups.join(", ")
        };
        lines.push(Line::from(Span::raw(format!(
            "  {:<16} {:<20} {:<12} {}",
            h.name, h.ssh_host, h.shell, groups
        ))));
    }

    // ── Checks ──
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        format!("── Applicable Checks ({}) ──", list_data.checks.len()),
        Style::default()
            .fg(data.theme.accent_checkout)
            .add_modifier(Modifier::BOLD),
    )));
    if list_data.checks.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none)",
            Style::default().fg(data.theme.inactive),
        )));
    } else {
        for (i, entry) in list_data.checks.iter().enumerate() {
            lines.push(Line::from(Span::raw(format!(
                "  [{}] name: {}",
                i + 1,
                format_entry_name(&entry.name)
            ))));
            if !entry.enabled.is_empty() {
                lines.push(Line::from(Span::raw(format!(
                    "      enabled: {}",
                    entry.enabled.join(", ")
                ))));
            }
            for p in &entry.path {
                lines.push(Line::from(Span::raw(format!(
                    "      path: {} ({})",
                    p.path, p.label
                ))));
            }
        }
    }

    // ── Syncs ──
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        format!("── Applicable Sync Entries ({}) ──", list_data.syncs.len()),
        Style::default()
            .fg(data.theme.accent_checkout)
            .add_modifier(Modifier::BOLD),
    )));
    if list_data.syncs.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (none)",
            Style::default().fg(data.theme.inactive),
        )));
    } else {
        for (i, entry) in list_data.syncs.iter().enumerate() {
            lines.push(Line::from(Span::raw(format!(
                "  [{}] name: {}  paths: {}",
                i + 1,
                format_entry_name(&entry.name),
                entry.paths.join(", ")
            ))));
        }
    }

    // Apply scroll, and (when the result list holds focus) draw a row cursor
    // on the selected line so it's clear where focus is — mirroring Checkout.
    let skip = data.result_scroll.min(lines.len());
    let cursor_style = Style::default()
        .fg(data.theme.accent_checkout)
        .add_modifier(Modifier::BOLD | Modifier::REVERSED);
    let visible: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .filter(|(i, _)| *i >= skip)
        .map(|(abs, line)| {
            if data.result_focused && abs == data.checkout_selected {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                Line::from(Span::styled(text, cursor_style))
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(visible), area);
}

/// Number of lines `render_list_result` produces for this data, so the scroll
/// viewport can be dimensioned correctly. Must mirror the structure above.
pub fn list_line_count(list: &ListData) -> usize {
    // Hosts: title + column header + separator + one per host.
    let mut n = 3 + list.hosts.len();
    // Checks: blank + title + body.
    n += 2;
    if list.checks.is_empty() {
        n += 1;
    } else {
        for entry in &list.checks {
            n += 1; // scope line
            if !entry.enabled.is_empty() {
                n += 1;
            }
            n += entry.path.len();
        }
    }
    // Syncs: blank + title + body.
    n += 2;
    if list.syncs.is_empty() {
        n += 1;
    } else {
        n += list.syncs.len();
    }
    n
}

/// Per-line selectable flags for `render_list_result`, mirroring its structure
/// exactly so the result cursor can skip decorative lines (section titles,
/// column header, separator, blank spacers, and empty `(none)` placeholders).
/// `true` marks a data row the focus cursor may land on.
pub fn list_selectable_lines(list: &ListData) -> Vec<bool> {
    let mut sel: Vec<bool> = Vec::new();
    // ── Hosts ──: title, column header, separator (all decorative).
    sel.push(false); // title
    sel.push(false); // column header
    sel.push(false); // separator
    sel.extend(list.hosts.iter().map(|_| true)); // one host row each
                                                 // ── Checks ──: blank + title (decorative), then body.
    sel.push(false); // blank
    sel.push(false); // title
    if list.checks.is_empty() {
        sel.push(false); // "(none)" placeholder
    } else {
        for entry in &list.checks {
            sel.push(true); // scope line
            if !entry.enabled.is_empty() {
                sel.push(true); // enabled line
            }
            sel.extend(entry.path.iter().map(|_| true)); // one path line each
        }
    }
    // ── Syncs ──: blank + title (decorative), then body.
    sel.push(false); // blank
    sel.push(false); // title
    if list.syncs.is_empty() {
        sel.push(false); // "(none)" placeholder
    } else {
        sel.extend(list.syncs.iter().map(|_| true)); // one entry line each
    }
    sel
}

fn format_entry_name(name: &Option<String>) -> String {
    match name {
        Some(n) if !n.is_empty() => n.clone(),
        _ => "(unnamed)".to_string(),
    }
}

// ── Log result ────────────────────────────────────────────────────────────────

/// Render `&[LogRow]` mirroring `log::run`'s format, using theme colors instead
/// of raw ANSI escape codes.
#[allow(dead_code)]
pub fn render_log_result(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    let rows = match data.log {
        Some(r) => r,
        None => {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "  No log data.",
                    Style::default().fg(data.theme.inactive),
                )),
                area,
            );
            return;
        }
    };

    if rows.is_empty() {
        let dim = Style::default().fg(data.theme.inactive);
        let msg = vec![
            Line::from(Span::styled("  No log entries yet.", dim)),
            Line::from(Span::styled(
                "  Logs are recorded automatically when you run check / run / exec / sync.",
                dim,
            )),
            Line::from(Span::styled(
                "  If you expected entries, relax the filters above (errors / action / host / since).",
                dim,
            )),
        ];
        frame.render_widget(Paragraph::new(msg), area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for r in rows {
        let time = chrono::DateTime::from_timestamp(r.ts, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| r.ts.to_string());

        let duration = r
            .duration_ms
            .map(|ms| format!(" ({:.1}s)", ms as f64 / 1000.0))
            .unwrap_or_default();

        let note_str = r
            .note
            .as_deref()
            .map(|n| format!(" — {}", n))
            .unwrap_or_default();

        let (glyph, glyph_color) = match r.status.as_str() {
            "ok" => ("✓", data.theme.accent_checkout),
            "error" => ("✗", data.theme.error),
            "skipped" => ("⊘", data.theme.warning),
            _ => ("·", Color::Reset),
        };

        let line = Line::from(vec![
            Span::raw(format!("{} ", time)),
            Span::styled(glyph, Style::default().fg(glyph_color)),
            Span::raw(format!(
                " [{}] {} {}{}{}",
                r.host, r.command, r.action, duration, note_str
            )),
        ]);
        lines.push(line);
    }

    // When the result list holds focus, draw a row cursor on the selected line
    // so focus position is visible — mirroring Checkout/List.
    let skip = data.result_scroll.min(lines.len());
    let cursor_style = Style::default()
        .fg(data.theme.accent_checkout)
        .add_modifier(Modifier::BOLD | Modifier::REVERSED);
    let visible: Vec<Line> = lines
        .into_iter()
        .enumerate()
        .filter(|(i, _)| *i >= skip)
        .map(|(abs, line)| {
            if data.result_focused && abs == data.checkout_selected {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                Line::from(Span::styled(text, cursor_style))
            } else {
                line
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(visible), area);
}

#[cfg(test)]
mod tests {
    use super::{list_line_count, list_selectable_lines};
    use crate::commands::list::ListData;

    fn sample(populated: bool) -> ListData {
        let hosts = serde_json::from_value(serde_json::json!([
            { "name": "h1", "ssh_host": "h1.example", "shell": "sh", "groups": ["web"] },
            { "name": "h2", "ssh_host": "h2.example", "shell": "sh", "groups": [] },
        ]))
        .unwrap();
        let (checks, syncs) = if populated {
            (
                serde_json::from_value(serde_json::json!([
                    { "enabled": ["cpu", "mem"], "path": [{ "path": "/v", "label": "v" }] },
                ]))
                .unwrap(),
                serde_json::from_value(serde_json::json!([
                    { "paths": ["~/a", "~/b"] },
                ]))
                .unwrap(),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        ListData {
            hosts,
            checks,
            syncs,
        }
    }

    #[test]
    fn selectable_flags_match_line_count_empty_sections() {
        let d = sample(false);
        assert_eq!(list_selectable_lines(&d).len(), list_line_count(&d));
    }

    #[test]
    fn selectable_flags_match_line_count_populated() {
        let d = sample(true);
        assert_eq!(list_selectable_lines(&d).len(), list_line_count(&d));
    }

    #[test]
    fn host_rows_are_selectable_titles_are_not() {
        let d = sample(true);
        let sel = list_selectable_lines(&d);
        // Lines 0..3 are title / column header / separator → not selectable.
        assert!(!sel[0] && !sel[1] && !sel[2]);
        // Lines 3,4 are the two host rows → selectable.
        assert!(sel[3] && sel[4]);
    }
}
