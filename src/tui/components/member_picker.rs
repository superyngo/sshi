//! Member picker popup — the working replacement for the broken `cycle_chip`
//! logic in the old filter popup.
//!
//! Lets the user actually choose *which* groups / hosts / skip-hosts (multi
//! select) or *which* shell (single select) the target filter uses. Opened
//! from the Operate tab by pressing Enter on the Members / Skip field.
//!
//!   ↑↓ / jk : move cursor
//!   Space   : toggle membership (multi) / select (single)
//!   Enter   : apply
//!   Esc     : cancel

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

use crate::tui::theme::Theme;

use super::popup::centered_rect;

/// Fixed page step for PageUp/PageDown (the popup height is dynamic; a fixed
/// step keeps the handler stateless and matches the other scroll popups).
const PICKER_PAGE: usize = 10;

/// Which target-filter field the picker edits when applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerTarget {
    Groups,
    Hosts,
    Skip,
    Shell,
    /// `[[check]]` entry names to apply (Operate tab; empty = "default").
    CheckNames,
    /// `[[sync]]` entry names to apply (Operate tab).
    SyncNames,
}

impl PickerTarget {
    /// Whether this picker offers the `a` = add-entry shortcut (name pickers
    /// can jump to the Config add-entry form).
    pub fn allows_add(self) -> bool {
        matches!(self, PickerTarget::CheckNames | PickerTarget::SyncNames)
    }
}

/// Outcome of a key handed to the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerResult {
    Continue,
    Cancelled,
    Applied,
    /// `a` pressed on a name picker — caller opens the Config add-entry form.
    Add,
}

pub struct MemberPicker {
    pub target: PickerTarget,
    options: Vec<String>,
    selected: Vec<bool>,
    cursor: usize,
    multi: bool,
    /// Accent colour of the tab that opened the picker (keeps the popup visually
    /// consistent with its origin — green for View, cyan for Operate, …).
    accent: Color,
}

impl MemberPicker {
    /// Build a picker. `options` are all available values; `current` are the
    /// ones already chosen (pre-checked). `accent` themes the popup to match the
    /// originating tab.
    pub fn new(
        target: PickerTarget,
        options: Vec<String>,
        current: &[String],
        accent: Color,
    ) -> Self {
        let multi = !matches!(target, PickerTarget::Shell);
        let selected = options
            .iter()
            .map(|o| current.iter().any(|c| c == o))
            .collect();
        // Place the cursor on the first already-selected option, if any.
        let cursor = options
            .iter()
            .position(|o| current.iter().any(|c| c == o))
            .unwrap_or(0);
        Self {
            target,
            options,
            selected,
            cursor,
            multi,
            accent,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PickerResult {
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            return PickerResult::Cancelled;
        }
        match key.code {
            KeyCode::Esc => PickerResult::Cancelled,
            KeyCode::Enter => PickerResult::Applied,
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.options.is_empty() && self.cursor > 0 {
                    self.cursor -= 1;
                }
                PickerResult::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.cursor + 1 < self.options.len() {
                    self.cursor += 1;
                }
                PickerResult::Continue
            }
            KeyCode::PageUp => {
                self.cursor = self.cursor.saturating_sub(PICKER_PAGE);
                PickerResult::Continue
            }
            KeyCode::PageDown => {
                if !self.options.is_empty() {
                    self.cursor = (self.cursor + PICKER_PAGE).min(self.options.len() - 1);
                }
                PickerResult::Continue
            }
            KeyCode::Home => {
                self.cursor = 0;
                PickerResult::Continue
            }
            KeyCode::End => {
                self.cursor = self.options.len().saturating_sub(1);
                PickerResult::Continue
            }
            KeyCode::Char(' ') | KeyCode::Char('x') => {
                self.toggle();
                PickerResult::Continue
            }
            KeyCode::Char('a') if self.target.allows_add() => PickerResult::Add,
            _ => PickerResult::Continue,
        }
    }

    fn toggle(&mut self) {
        if self.options.is_empty() {
            return;
        }
        if self.multi {
            self.selected[self.cursor] = !self.selected[self.cursor];
        } else {
            // Single select: clear all, set cursor.
            for s in self.selected.iter_mut() {
                *s = false;
            }
            self.selected[self.cursor] = true;
        }
    }

    /// The chosen values (in option order).
    pub fn chosen(&self) -> Vec<String> {
        self.options
            .iter()
            .zip(&self.selected)
            .filter(|(_, s)| **s)
            .map(|(o, _)| o.clone())
            .collect()
    }

    pub fn render(&self, area: Rect, theme: &Theme, frame: &mut Frame) {
        let title = match self.target {
            PickerTarget::Groups => " Pick groups · Space=toggle · Enter=apply · Esc=cancel ",
            PickerTarget::Hosts => " Pick hosts · Space=toggle · Enter=apply · Esc=cancel ",
            PickerTarget::Skip => " Pick hosts to skip · Space=toggle · Enter=apply · Esc=cancel ",
            PickerTarget::Shell => " Pick shell · Space=select · Enter=apply · Esc=cancel ",
            PickerTarget::CheckNames => {
                " Pick [[check]] entries · Space=toggle · a=add · Enter=apply · Esc=cancel "
            }
            PickerTarget::SyncNames => {
                " Pick [[sync]] entries · Space=toggle · a=add · Enter=apply · Esc=cancel "
            }
        };
        let popup = centered_rect(60, 60, area);
        frame.render_widget(Clear, popup);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.accent))
            .title(title);
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0)])
            .split(inner)[0];

        // Scroll window: keep the cursor visible when options overflow the popup.
        let vis = (rows.height as usize).max(1);
        let n = self.options.len();
        let start = if self.cursor < vis {
            0
        } else {
            (self.cursor + 1 - vis).min(n.saturating_sub(vis))
        };
        let end = (start + vis).min(n);

        let lines: Vec<Line> = if self.options.is_empty() {
            vec![Line::from(Span::styled(
                "  (nothing available — add entries in Config)",
                Style::default().fg(theme.inactive),
            ))]
        } else {
            self.options[start..end]
                .iter()
                .enumerate()
                .map(|(rel, opt)| {
                    let i = start + rel;
                    let glyph = if self.multi {
                        if self.selected[i] {
                            "[✓]"
                        } else {
                            "[ ]"
                        }
                    } else if self.selected[i] {
                        "◉"
                    } else {
                        "○"
                    };
                    let focused = i == self.cursor;
                    let style = if focused {
                        Style::default()
                            .fg(self.accent)
                            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
                    } else {
                        Style::default()
                    };
                    Line::from(Span::styled(format!(" {glyph} {opt}"), style))
                })
                .collect()
        };
        frame.render_widget(Paragraph::new(lines), rows);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;
    use ratatui::style::Color;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn a_adds_only_for_name_pickers() {
        let mut p = MemberPicker::new(
            PickerTarget::CheckNames,
            vec!["a".into(), "b".into()],
            &[],
            Color::Cyan,
        );
        assert_eq!(p.handle_key(key('a')), PickerResult::Add);

        let mut hosts = MemberPicker::new(PickerTarget::Hosts, vec!["h1".into()], &[], Color::Cyan);
        // 'a' is inert for non-name pickers.
        assert_eq!(hosts.handle_key(key('a')), PickerResult::Continue);
    }

    #[test]
    fn name_picker_is_multi_and_prechecks_current() {
        let mut p = MemberPicker::new(
            PickerTarget::SyncNames,
            vec!["x".into(), "y".into(), "z".into()],
            &["y".into()],
            Color::Cyan,
        );
        assert_eq!(p.chosen(), vec!["y".to_string()]);
        // toggle another on (multi-select).
        p.handle_key(key('j')); // cursor -> z? starts on 'y' (index1)
        p.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(p.chosen(), vec!["y".to_string(), "z".to_string()]);
    }
}
