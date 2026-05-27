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

use super::super::state::persist::ViewOperationKind;
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
}

/// Entry point: render the entire View tab into `area`.
#[allow(dead_code)]
pub fn render_view(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    let border_col = if data.navbar_focused {
        data.theme.border_inactive
    } else {
        data.theme.border_active
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_col))
        .title(" View ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // op selector
            Constraint::Length(1), // target summary
            Constraint::Min(0),    // result area
        ])
        .split(inner);

    render_view_selector(data, chunks[0], frame);
    render_view_target_summary(data, chunks[1], frame);
    render_result_area(data, chunks[2], frame);
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
        let active = *op == data.view_op;
        let style = if active {
            Style::default()
                .fg(data.theme.accent_checkout)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED)
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

/// One-line summary of the active target filter. Greyed out for Log (log has no target).
#[allow(dead_code)]
fn render_view_target_summary(data: &ViewRenderData, area: Rect, frame: &mut Frame) {
    let text = if data.view_op == ViewOperationKind::Log {
        Span::styled(
            " (Log queries all hosts — target filter not applied)",
            Style::default().fg(data.theme.inactive),
        )
    } else {
        Span::styled(
            " [f] to set target filter",
            Style::default().fg(data.theme.inactive),
        )
    };
    frame.render_widget(Paragraph::new(Line::from(vec![text])), area);
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
    let mut constraints: Vec<Constraint> =
        vec![Constraint::Length(16), Constraint::Length(12)];
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
    for snap in &snapshots[start..end] {
        let status_text = if snap.online { "✓ online" } else { "✗ offline" };
        let status_style = Style::default().fg(if snap.online {
            data.theme.accent_checkout
        } else {
            data.theme.error
        });

        let mut cells: Vec<Cell> =
            vec![Cell::from(snap.host.clone()), Cell::from(status_text).style(status_style)];
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
        rows.push(Row::new(cells));
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
            let scope = format_scope(&entry.groups, entry.enable_hosts, entry.enable_all);
            lines.push(Line::from(Span::raw(format!("  [{}] scope: {}", i + 1, scope))));
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
            let scope = format_scope(&entry.groups, entry.enable_hosts, entry.enable_all);
            lines.push(Line::from(Span::raw(format!(
                "  [{}] scope: {}  paths: {}",
                i + 1,
                scope,
                entry.paths.join(", ")
            ))));
        }
    }

    // Apply scroll
    let skip = data.result_scroll.min(lines.len());
    let visible: Vec<Line> = lines.into_iter().skip(skip).collect();
    frame.render_widget(Paragraph::new(visible), area);
}

fn format_scope(groups: &[String], enable_hosts: bool, enable_all: bool) -> String {
    let mut parts = Vec::new();
    if !groups.is_empty() {
        parts.push(format!("groups=[{}]", groups.join(", ")));
    }
    if !enable_hosts {
        parts.push("hosts=off".to_string());
    }
    if !enable_all {
        parts.push("all=off".to_string());
    }
    if parts.is_empty() {
        "global".to_string()
    } else {
        parts.join(" ")
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
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  No log entries found.",
                Style::default().fg(data.theme.inactive),
            )),
            area,
        );
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

    let skip = data.result_scroll.min(lines.len());
    let visible: Vec<Line> = lines.into_iter().skip(skip).collect();
    frame.render_widget(Paragraph::new(visible), area);
}
