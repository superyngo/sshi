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

/// Which target-filter field the picker edits when applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerTarget {
    Groups,
    Hosts,
    Skip,
    Shell,
}

/// Outcome of a key handed to the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerResult {
    Continue,
    Cancelled,
    Applied,
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
    pub fn new(target: PickerTarget, options: Vec<String>, current: &[String], accent: Color) -> Self {
        let multi = !matches!(target, PickerTarget::Shell);
        let selected = options.iter().map(|o| current.iter().any(|c| c == o)).collect();
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
            KeyCode::Char(' ') | KeyCode::Char('x') => {
                self.toggle();
                PickerResult::Continue
            }
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

        let lines: Vec<Line> = if self.options.is_empty() {
            vec![Line::from(Span::styled(
                "  (nothing available — add entries in Config)",
                Style::default().fg(theme.inactive),
            ))]
        } else {
            self.options
                .iter()
                .enumerate()
                .map(|(i, opt)| {
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
