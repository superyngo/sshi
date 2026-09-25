//! Helpers shared across TUI tabs (Operate / View / Config) and the
//! target-filter popup. Anything triplicated across those sites lives
//! here so the rendering surface stays in sync.
//!
//! Per audit §1 P1 MED ×3 + LOW ×2 (F2). The 3-arg
//! `operate_tab::focus_style(focused, active, theme)` is intentionally
//! NOT moved here — its `active` parameter carries panel-activity
//! semantics that the 2-arg sites don't have.

use std::collections::BTreeSet;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::config::schema::AppConfig;
use crate::tui::state::persist::{ShellMode, TargetFilterMode, TargetFilterState};

/// Human-readable lowercase label for a `ShellMode` variant ("sh",
/// "powershell", "cmd"). Used by Operate, View, and the target-filter
/// popup.
pub fn shell_label(s: ShellMode) -> &'static str {
    s.as_str()
}

/// Join a list of strings with `", "`. If the list is empty, return
/// `"({empty})"` so the row keeps a visible placeholder. Replaces the
/// forked `chips` / `view_chips` / `format_chips` helpers.
pub fn chips(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        format!("({empty})")
    } else {
        items.join(", ")
    }
}

pub use crate::util::truncate;

/// Reverse-video bold style for a focused row, parameterised by the
/// accent colour the caller picks from its `Theme` (`accent_operate`
/// for the Operate tab and target-filter popup, `accent_checkout` for
/// the View tab). Unfocused rows get the default style. Replaces the
/// forked 2-arg `focus_style` / `view_focus_style` helpers.
pub fn focus_accent(focused: bool, accent: Color) -> Style {
    if focused {
        Style::default()
            .fg(accent)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::default()
    }
}

/// "Target: ◉ All ○ Groups ○ Hosts ○ Shell (N hosts)" — the target-mode row
/// shared by the Operate and View tabs (B54). `focused` is true only while
/// the row has keyboard focus in the active panel.
pub fn target_mode_line(
    filter: &TargetFilterState,
    host_count: usize,
    focused: bool,
    accent: Color,
    inactive: Color,
) -> Line<'static> {
    let modes = [
        (TargetFilterMode::All, "All"),
        (TargetFilterMode::Groups, "Groups"),
        (TargetFilterMode::Hosts, "Hosts"),
        (TargetFilterMode::Shell, "Shell"),
    ];
    let mut spans = vec![Span::raw(" Target:  ")];
    for (mode, label) in modes {
        let selected = mode == filter.mode;
        let prefix = if selected { "◉ " } else { "○ " };
        let style = match (selected, focused) {
            (true, true) => Style::default()
                .fg(accent)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            (true, false) => Style::default().fg(accent).add_modifier(Modifier::BOLD),
            (false, _) => Style::default().fg(inactive),
        };
        spans.push(Span::styled(format!("{prefix}{label}"), style));
        spans.push(Span::raw("   "));
    }
    spans.push(Span::styled(
        format!("({host_count} hosts)"),
        Style::default().fg(inactive),
    ));
    Line::from(spans)
}

/// "Members: …" (or "Shell: …") row under the target mode; `value_style`
/// carries the tab's focus styling (B54).
pub fn target_members_line(filter: &TargetFilterState, value_style: Style) -> Line<'static> {
    let (label, value) = match filter.mode {
        TargetFilterMode::Groups => ("Members", chips(&filter.groups, "no groups")),
        TargetFilterMode::Hosts => ("Members", chips(&filter.hosts, "no hosts")),
        TargetFilterMode::Shell => ("Shell", shell_label(filter.shell).to_string()),
        TargetFilterMode::All => ("Members", String::new()),
    };
    Line::from(vec![
        Span::raw(format!(" {label}: ")),
        Span::styled(value, value_style),
    ])
}

/// "Skip: …" row; `value_style` carries the tab's focus styling (B54).
pub fn target_skip_line(filter: &TargetFilterState, value_style: Style) -> Line<'static> {
    Line::from(vec![
        Span::raw(" Skip:    "),
        Span::styled(chips(&filter.skip, "none"), value_style),
    ])
}

/// All group names referenced by any host in `config`, plus any
/// entries of `current` that aren't on a host (so the picker still
/// shows a previously-selected group whose host was just deleted).
/// Empty strings are filtered, the result is de-duped and sorted.
/// Replaces `App::available_groups`, `target_filter::collect_groups`,
/// and the host-scan portion of `config_tab::collect_known_groups`.
pub fn collect_groups(config: &AppConfig, current: &[String]) -> Vec<String> {
    let mut known: BTreeSet<String> = config
        .host
        .iter()
        .flat_map(|h| h.groups.iter().cloned())
        .filter(|g| !g.is_empty())
        .collect();
    for g in current {
        if !g.is_empty() {
            known.insert(g.clone());
        }
    }
    known.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_label_covers_all_variants() {
        assert_eq!(shell_label(ShellMode::Sh), "sh");
        assert_eq!(shell_label(ShellMode::PowerShell), "powershell");
        assert_eq!(shell_label(ShellMode::Cmd), "cmd");
    }

    #[test]
    fn chips_empty_returns_placeholder() {
        assert_eq!(chips(&[], "none"), "(none)");
    }

    #[test]
    fn chips_non_empty_joins_with_comma_space() {
        assert_eq!(
            chips(&["a".into(), "b".into(), "c".into()], "none"),
            "a, b, c"
        );
    }

    #[test]
    fn truncate_keeps_short_input_unchanged() {
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn truncate_appends_ellipsis_when_too_long() {
        assert_eq!(truncate("abcdef", 4), "abc…");
    }

    #[test]
    fn truncate_handles_wide_chars() {
        assert_eq!(truncate("中文测试", 5), "中文…");
    }

    #[test]
    fn focus_accent_unfocused_returns_default() {
        assert_eq!(focus_accent(false, Color::Blue), Style::default());
    }

    #[test]
    fn focus_accent_focused_carries_accent_and_reverse_bold() {
        let s = focus_accent(true, Color::Blue);
        assert_eq!(s.fg, Some(Color::Blue));
        assert!(s.add_modifier & (Modifier::BOLD | Modifier::REVERSED) != Modifier::empty());
    }

    #[test]
    fn collect_groups_dedupes_and_sorts() {
        let mut host = crate::config::schema::HostEntry::placeholder("h1", "h1");
        host.groups = vec!["web".into(), "db".into(), "web".into()];
        let config = AppConfig {
            host: vec![std::sync::Arc::new(host)],
            ..Default::default()
        };
        assert_eq!(
            collect_groups(&config, &[]),
            vec!["db".to_string(), "web".to_string()]
        );
    }

    #[test]
    fn collect_groups_includes_current_even_if_not_on_any_host() {
        let config = AppConfig::default();
        let current = vec!["orphan".to_string()];
        assert_eq!(
            collect_groups(&config, &current),
            vec!["orphan".to_string()]
        );
    }

    #[test]
    fn collect_groups_filters_empty_strings() {
        let mut host = crate::config::schema::HostEntry::placeholder("h1", "h1");
        host.groups = vec!["".to_string(), "web".to_string()];
        let config = AppConfig {
            host: vec![std::sync::Arc::new(host)],
            ..Default::default()
        };
        assert_eq!(
            collect_groups(&config, &["".into()]),
            vec!["web".to_string()]
        );
    }
}
