//! Single-line text input component for the Operate tab param panel.
//!
//! Per docs/tui_reconstruct_plan.md §14.3: all global single-letter shortcuts
//! are suspended while `InputMode::Active`; callers must check the mode flag
//! before routing any key event to the rest of the app.
//!
//! Editing contract (audit §1 P10): Emacs-style line editing, kill ring,
//! undo ring, and grapheme-cluster cursor movement.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use unicode_segmentation::UnicodeSegmentation;

const RING_MAX: usize = 8;

/// Whether the field is currently capturing keyboard input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InputMode {
    #[default]
    Normal,
    Active,
}

/// A single-line text input with visible cursor and Esc-restore semantics.
#[derive(Debug, Clone, Default)]
pub struct InputField {
    /// Current content.
    pub value: String,
    /// Grapheme-cluster cursor position within `value`.
    cursor_pos: usize,
    /// Snapshot saved on `Enter` (active → normal) for Esc-restore.
    pub saved: String,
    pub mode: InputMode,
    kill_ring: Vec<String>,
    undo_ring: Vec<(String, usize)>,
}

impl InputField {
    pub fn new(initial: &str) -> Self {
        Self {
            value: initial.to_string(),
            cursor_pos: initial.graphemes(true).count(),
            saved: initial.to_string(),
            mode: InputMode::Normal,
            kill_ring: Vec::new(),
            undo_ring: Vec::new(),
        }
    }

    /// Activate the field, saving the current value for Esc-restore.
    pub fn activate(&mut self) {
        self.saved = self.value.clone();
        self.mode = InputMode::Active;
        self.cursor_pos = self.grapheme_count();
        self.kill_ring.clear();
        self.undo_ring.clear();
    }

    /// Deactivate and confirm (save current value as the new baseline).
    pub fn confirm(&mut self) {
        self.saved = self.value.clone();
        self.mode = InputMode::Normal;
    }

    /// Deactivate and revert to the value saved at `activate` time.
    pub fn cancel(&mut self) {
        self.value = self.saved.clone();
        self.cursor_pos = self.grapheme_count();
        self.mode = InputMode::Normal;
    }

    /// Handle a key event while the field is active.
    ///
    /// Returns `true` if the event was consumed and the field should be
    /// redrawn. Returns `false` for unhandled events.
    pub fn handle_key(&mut self, key: KeyEvent) -> bool {
        if self.mode != InputMode::Active {
            return false;
        }
        match key.code {
            KeyCode::Char(c)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.insert_char(c);
                true
            }
            KeyCode::Char('a') if key.modifiers == KeyModifiers::CONTROL => {
                self.cursor_pos = 0;
                true
            }
            KeyCode::Char('e') if key.modifiers == KeyModifiers::CONTROL => {
                self.cursor_pos = self.grapheme_count();
                true
            }
            KeyCode::Char('k') if key.modifiers == KeyModifiers::CONTROL => {
                self.kill_to_end();
                true
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                self.kill_to_start();
                true
            }
            KeyCode::Char('w') if key.modifiers == KeyModifiers::CONTROL => {
                self.kill_word_back();
                true
            }
            KeyCode::Char('y') if key.modifiers == KeyModifiers::CONTROL => {
                self.yank();
                true
            }
            KeyCode::Char('_') | KeyCode::Char('z') if key.modifiers == KeyModifiers::CONTROL => {
                self.undo();
                true
            }
            KeyCode::Backspace if key.modifiers == KeyModifiers::ALT => {
                self.kill_word_back();
                true
            }
            KeyCode::Backspace => {
                self.delete_grapheme_back();
                true
            }
            KeyCode::Delete => {
                self.delete_grapheme_forward();
                true
            }
            KeyCode::Left if key.modifiers == KeyModifiers::CONTROL => {
                self.cursor_pos = self.prev_word_start(self.cursor_pos);
                true
            }
            KeyCode::Right if key.modifiers == KeyModifiers::CONTROL => {
                self.cursor_pos = self.next_word_end(self.cursor_pos);
                true
            }
            KeyCode::Left => {
                if self.cursor_pos > 0 {
                    self.cursor_pos -= 1;
                }
                true
            }
            KeyCode::Right => {
                if self.cursor_pos < self.grapheme_count() {
                    self.cursor_pos += 1;
                }
                true
            }
            KeyCode::Home => {
                self.cursor_pos = 0;
                true
            }
            KeyCode::End => {
                self.cursor_pos = self.grapheme_count();
                true
            }
            KeyCode::Enter => {
                self.confirm();
                true
            }
            KeyCode::Esc => {
                self.cancel();
                true
            }
            _ => false,
        }
    }

    /// Render the field inside `area`. `focused` controls the border colour.
    pub fn render(&self, frame: &mut Frame, area: Rect, label: &str, focused: bool) {
        let border_style = if self.mode == InputMode::Active {
            Style::default().fg(Color::Yellow)
        } else if focused {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(
                format!(" {} ", label),
                Style::default().add_modifier(Modifier::BOLD),
            ));

        // Build the visible line with a cursor marker when active.
        let display = if self.mode == InputMode::Active {
            let (before, after) = self.split_at_cursor();
            let cursor_char = after.chars().next().unwrap_or(' ');
            let after_cursor: String = after.chars().skip(1).collect();
            Line::from(vec![
                Span::raw(before),
                Span::styled(
                    cursor_char.to_string(),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(after_cursor),
            ])
        } else {
            Line::from(Span::raw(self.value.clone()))
        };

        let para = Paragraph::new(display).block(block);
        frame.render_widget(para, area);
    }

    // ------ mutating primitives (each snapshots undo) ------

    fn insert_char(&mut self, c: char) {
        self.snapshot_undo();
        let byte_pos = self.grapheme_to_byte(self.cursor_pos);
        self.value.insert(byte_pos, c);
        self.cursor_pos += 1;
    }

    fn delete_grapheme_back(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        self.snapshot_undo();
        self.cursor_pos -= 1;
        let byte_pos = self.grapheme_to_byte(self.cursor_pos);
        self.value.remove(byte_pos);
    }

    fn delete_grapheme_forward(&mut self) {
        if self.cursor_pos >= self.grapheme_count() {
            return;
        }
        self.snapshot_undo();
        let byte_pos = self.grapheme_to_byte(self.cursor_pos);
        self.value.remove(byte_pos);
    }

    fn kill_to_end(&mut self) {
        let byte_pos = self.grapheme_to_byte(self.cursor_pos);
        if byte_pos >= self.value.len() {
            return;
        }
        self.snapshot_undo();
        let killed = self.value[byte_pos..].to_string();
        self.value.truncate(byte_pos);
        self.push_kill(killed);
    }

    fn kill_to_start(&mut self) {
        let byte_pos = self.grapheme_to_byte(self.cursor_pos);
        if byte_pos == 0 {
            return;
        }
        self.snapshot_undo();
        let killed = self.value[..byte_pos].to_string();
        self.value = self.value[byte_pos..].to_string();
        self.cursor_pos = 0;
        self.push_kill(killed);
    }

    fn kill_word_back(&mut self) {
        let new_pos = self.prev_word_start(self.cursor_pos);
        if new_pos == self.cursor_pos {
            return;
        }
        self.snapshot_undo();
        let start_byte = self.grapheme_to_byte(new_pos);
        let end_byte = self.grapheme_to_byte(self.cursor_pos);
        let killed = self.value[start_byte..end_byte].to_string();
        self.value.replace_range(start_byte..end_byte, "");
        self.cursor_pos = new_pos;
        self.push_kill(killed);
    }

    fn yank(&mut self) {
        let Some(text) = self.kill_ring.last().cloned() else {
            return;
        };
        self.snapshot_undo();
        let byte_pos = self.grapheme_to_byte(self.cursor_pos);
        self.value.insert_str(byte_pos, &text);
        self.cursor_pos += text.graphemes(true).count();
    }

    fn undo(&mut self) {
        if let Some((prev_value, prev_cursor)) = self.undo_ring.pop() {
            self.value = prev_value;
            self.cursor_pos = prev_cursor;
        }
    }

    // ------ ring helpers ------

    fn snapshot_undo(&mut self) {
        self.undo_ring.push((self.value.clone(), self.cursor_pos));
        if self.undo_ring.len() > RING_MAX {
            self.undo_ring.remove(0);
        }
    }

    fn push_kill(&mut self, killed: String) {
        if killed.is_empty() {
            return;
        }
        self.kill_ring.push(killed);
        if self.kill_ring.len() > RING_MAX {
            self.kill_ring.remove(0);
        }
    }

    // ------ grapheme helpers ------

    fn grapheme_count(&self) -> usize {
        self.value.graphemes(true).count()
    }

    fn grapheme_to_byte(&self, grapheme_idx: usize) -> usize {
        self.value
            .grapheme_indices(true)
            .nth(grapheme_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.value.len())
    }

    pub(crate) fn split_at_cursor(&self) -> (&str, &str) {
        let byte_pos = self.grapheme_to_byte(self.cursor_pos);
        self.value.split_at(byte_pos)
    }

    #[cfg(test)]
    pub(crate) fn cursor_pos_for_test(&self) -> usize {
        self.cursor_pos
    }

    #[cfg(test)]
    pub(crate) fn grapheme_count_for_test(&self) -> usize {
        self.grapheme_count()
    }

    fn prev_word_start(&self, mut i: usize) -> usize {
        let g: Vec<&str> = self.value.graphemes(true).collect();
        while i > 0 && !is_word_grapheme(g[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word_grapheme(g[i - 1]) {
            i -= 1;
        }
        i
    }

    fn next_word_end(&self, mut i: usize) -> usize {
        let g: Vec<&str> = self.value.graphemes(true).collect();
        let n = g.len();
        while i < n && !is_word_grapheme(g[i]) {
            i += 1;
        }
        while i < n && is_word_grapheme(g[i]) {
            i += 1;
        }
        i
    }
}

fn is_word_grapheme(g: &str) -> bool {
    g.chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::InputField;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn alt(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::ALT)
    }

    fn plain(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn activate_and_type(field: &mut InputField, text: &str) {
        field.activate();
        for c in text.chars() {
            field.handle_key(plain(c));
        }
    }

    #[test]
    fn ctrl_a_moves_cursor_to_start() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "hello");
        assert_eq!(f.cursor_pos_for_test(), 5);
        assert!(f.handle_key(ctrl('a')));
        assert_eq!(f.cursor_pos_for_test(), 0);
        assert_eq!(f.value, "hello");
    }

    #[test]
    fn ctrl_e_moves_cursor_to_end() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "hello");
        f.handle_key(key(KeyCode::Home));
        assert_eq!(f.cursor_pos_for_test(), 0);
        assert!(f.handle_key(ctrl('e')));
        assert_eq!(f.cursor_pos_for_test(), 5);
    }

    #[test]
    fn ctrl_k_kills_to_end_and_yanks_back() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "hello world");
        f.handle_key(ctrl('a'));
        f.handle_key(key(KeyCode::Right));
        f.handle_key(key(KeyCode::Right));
        assert_eq!(f.cursor_pos_for_test(), 2);
        assert!(f.handle_key(ctrl('k')));
        assert_eq!(f.value, "he");
        assert_eq!(f.cursor_pos_for_test(), 2);

        // Ctrl+Y yanks the kill back at cursor.
        f.handle_key(ctrl('a'));
        assert!(f.handle_key(ctrl('y')));
        assert_eq!(f.value, "llo worldhe");
    }

    #[test]
    fn ctrl_u_kills_to_start() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "hello");
        f.handle_key(key(KeyCode::Left));
        f.handle_key(key(KeyCode::Left));
        assert_eq!(f.cursor_pos_for_test(), 3);
        assert!(f.handle_key(ctrl('u')));
        assert_eq!(f.value, "lo");
        assert_eq!(f.cursor_pos_for_test(), 0);

        f.handle_key(ctrl('e'));
        f.handle_key(ctrl('y'));
        assert_eq!(f.value, "lohel");
    }

    #[test]
    fn ctrl_w_kills_word_back() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "foo bar baz");
        assert_eq!(f.cursor_pos_for_test(), 11);
        assert!(f.handle_key(ctrl('w')));
        assert_eq!(f.value, "foo bar ");
        assert_eq!(f.cursor_pos_for_test(), 8);

        // Yank pulls the most recent kill ("baz") back at cursor.
        assert!(f.handle_key(ctrl('y')));
        assert_eq!(f.value, "foo bar baz");
        assert_eq!(f.cursor_pos_for_test(), 11);
    }

    #[test]
    fn alt_backspace_kills_word_back_like_ctrl_w() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "alpha beta");
        assert!(f.handle_key(alt(KeyCode::Backspace)));
        assert_eq!(f.value, "alpha ");
    }

    #[test]
    fn ctrl_y_with_empty_kill_ring_is_noop() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "abc");
        f.handle_key(key(KeyCode::Home));
        let before = f.value.clone();
        assert!(f.handle_key(ctrl('y')));
        assert_eq!(f.value, before);
    }

    #[test]
    fn ctrl_left_jumps_word_back() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "foo bar baz");
        assert_eq!(f.cursor_pos_for_test(), 11);
        f.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(f.cursor_pos_for_test(), 8);
        f.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(f.cursor_pos_for_test(), 4);
        f.handle_key(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
        assert_eq!(f.cursor_pos_for_test(), 0);
    }

    #[test]
    fn ctrl_right_jumps_word_forward() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "foo bar baz");
        f.handle_key(key(KeyCode::Home));
        assert_eq!(f.cursor_pos_for_test(), 0);
        f.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(f.cursor_pos_for_test(), 3);
        f.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(f.cursor_pos_for_test(), 7);
        f.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
        assert_eq!(f.cursor_pos_for_test(), 11);
    }

    #[test]
    fn ctrl_z_undoes_last_mutating_op() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "hello");
        assert_eq!(f.value, "hello");
        assert!(f.handle_key(ctrl('z')));
        assert_eq!(f.value, "hell");
        assert!(f.handle_key(ctrl('z')));
        assert_eq!(f.value, "hel");
    }

    #[test]
    fn ctrl_underscore_is_also_undo() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "ab");
        assert_eq!(f.value, "ab");
        assert!(f.handle_key(ctrl('_')));
        assert_eq!(f.value, "a");
    }

    #[test]
    fn undo_restores_cursor_position() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "abc");
        f.handle_key(key(KeyCode::Home));
        f.handle_key(ctrl('k'));
        assert_eq!(f.value, "");
        assert_eq!(f.cursor_pos_for_test(), 0);
        f.handle_key(ctrl('z'));
        assert_eq!(f.value, "abc");
        assert_eq!(f.cursor_pos_for_test(), 0);
    }

    #[test]
    fn zwj_family_emoji_moves_as_one_grapheme() {
        let family = "a\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}b";
        let mut f = InputField::new(family);
        f.activate();
        f.handle_key(key(KeyCode::Home));
        assert_eq!(f.cursor_pos_for_test(), 0);
        f.handle_key(key(KeyCode::Right));
        assert_eq!(f.cursor_pos_for_test(), 1);
        f.handle_key(key(KeyCode::Right));
        // Cursor jumped past the 3-codepoint ZWJ family as a single grapheme.
        assert_eq!(f.cursor_pos_for_test(), 2);
        f.handle_key(key(KeyCode::Right));
        assert_eq!(f.cursor_pos_for_test(), 3);
        assert_eq!(f.grapheme_count_for_test(), 3);
    }

    #[test]
    fn zwj_family_emoji_backspace_deletes_one_grapheme() {
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}x";
        let mut f = InputField::new(family);
        f.activate();
        assert_eq!(f.grapheme_count_for_test(), 2);
        f.handle_key(key(KeyCode::Backspace));
        assert_eq!(f.value, "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}");
        assert_eq!(f.grapheme_count_for_test(), 1);
    }

    #[test]
    fn kill_ring_caps_at_ring_max() {
        let mut f = InputField::new("");
        activate_and_type(&mut f, "a b c d e f g h i j");
        for _ in 0..10 {
            f.handle_key(ctrl('w'));
        }
        // Field still usable after saturating the ring.
        f.handle_key(plain('z'));
        assert!(f.value.ends_with('z'));
    }

    #[test]
    fn plain_inserts_route_through_grapheme_indexing() {
        let mut f = InputField::new("café");
        f.activate();
        f.handle_key(key(KeyCode::Home));
        f.handle_key(plain('X'));
        assert_eq!(f.value, "Xcafé");
        assert_eq!(f.cursor_pos_for_test(), 1);
    }
}
