//! Per-frame mouse hit map (B83). Renderers record the rects they actually
//! draw, in paint order, so click targets can never drift from the screen.
//! An overlay calls [`HitMap::barrier`] before drawing: lookups then ignore
//! everything painted underneath it, so a click outside the top popup does
//! nothing.

use std::cell::RefCell;

use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::text::Line;

use crate::tui::app_state::ViewFocus;
use crate::tui::tabs::operate_tab::OpField;
use crate::tui::tabs::TabId;

/// A scrollable list whose rows can be clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum List {
    ViewResult,
    ConfigSidebar,
    ConfigFields,
    /// Config entry form: field rows.
    FormFields,
    /// Config entry form: nested group picker / list editor rows.
    FormPicker,
    FormVec,
    /// Config direct list editor / group picker over the field table.
    DirectVec,
    DirectPicker,
    /// The Operate / View member picker.
    MemberPicker,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hit {
    Tab(TabId),
    /// Click acts as this key press (popup buttons).
    Key(KeyCode),
    /// Popup body: swallows clicks; the wheel scrolls it with ↑/↓.
    Inert,
    /// Operate field: focus it, then activate (toggle / open / run).
    Op(OpField),
    /// Operate radio option: focus the row, then press → `n` times.
    OpCycle(OpField, usize),
    View(ViewFocus),
    ViewCycle(ViewFocus, usize),
    /// Absolute row index in a list.
    Row(List, usize),
    /// List area (wheel target) without a row under the pointer.
    Area(List),
}

#[derive(Debug, Default)]
pub struct HitMap {
    entries: RefCell<Vec<(Rect, Hit)>>,
    floor: std::cell::Cell<usize>,
}

impl HitMap {
    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
        self.floor.set(0);
    }

    pub fn push(&self, rect: Rect, hit: Hit) {
        if rect.width > 0 && rect.height > 0 {
            self.entries.borrow_mut().push((rect, hit));
        }
    }

    /// Start an overlay: everything recorded so far becomes unreachable.
    pub fn barrier(&self) {
        self.floor.set(self.entries.borrow().len());
    }

    /// Topmost target at a cell, ignoring anything under the top overlay.
    pub fn at(&self, x: u16, y: u16) -> Option<Hit> {
        let entries = self.entries.borrow();
        entries[self.floor.get()..]
            .iter()
            .rev()
            .find(|(r, _)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom())
            .map(|(_, h)| *h)
    }

    /// The rect recorded for `hit`, if any (tests click through this).
    pub fn rect_of(&self, hit: Hit) -> Option<Rect> {
        self.entries
            .borrow()
            .iter()
            .rev()
            .find(|(_, h)| *h == hit)
            .map(|(r, _)| *r)
    }

    /// One hit per row of a list drawn top-down in `area` from row `first`.
    pub fn rows(&self, list: List, area: Rect, first: usize, count: usize) {
        self.push(area, Hit::Area(list));
        for i in 0..count.min(area.height as usize) {
            self.push(
                Rect::new(area.x, area.y + i as u16, area.width, 1),
                Hit::Row(list, first + i),
            );
        }
    }

    /// Radio options of a one-line row: every span containing `◉` / `○`
    /// becomes a target that presses → the steps from the selected option.
    pub fn radio(&self, line: &Line, rect: Rect, mk: impl Fn(usize) -> Hit) {
        let mut x = rect.x;
        let mut opts = Vec::new();
        let mut selected = 0;
        for span in &line.spans {
            let w = span.width() as u16;
            if span.content.contains('◉') || span.content.contains('○') {
                if span.content.contains('◉') {
                    selected = opts.len();
                }
                opts.push((x, w));
            }
            x = x.saturating_add(w);
        }
        let n = opts.len();
        for (i, (ox, w)) in opts.into_iter().enumerate() {
            let w = w.min(rect.right().saturating_sub(ox));
            self.push(
                Rect::new(ox, rect.y, w, 1),
                mk((i + n - selected) % n.max(1)),
            );
        }
    }

    /// Clickable `[y] …` / `[n…] …` buttons on a confirm hint row: the
    /// part before `[n` sends `y`, the rest sends `n`.
    pub fn yes_no(&self, row: Rect, text: &str) {
        let split = text
            .find("[n")
            .map(|b| unicode_width::UnicodeWidthStr::width(&text[..b]) as u16)
            .unwrap_or(row.width);
        let split = split.min(row.width);
        self.push(
            Rect::new(row.x, row.y, split, 1),
            Hit::Key(KeyCode::Char('y')),
        );
        self.push(
            Rect::new(row.x + split, row.y, row.width - split, 1),
            Hit::Key(KeyCode::Char('n')),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::text::Span;

    #[test]
    fn barrier_hides_lower_layers_and_top_wins() {
        let m = HitMap::default();
        m.push(Rect::new(0, 0, 10, 10), Hit::Tab(TabId::Config));
        m.push(Rect::new(0, 0, 5, 5), Hit::Tab(TabId::View));
        assert_eq!(m.at(1, 1), Some(Hit::Tab(TabId::View)));
        assert_eq!(m.at(7, 7), Some(Hit::Tab(TabId::Config)));
        m.barrier();
        m.push(Rect::new(2, 2, 2, 2), Hit::Inert);
        assert_eq!(m.at(2, 2), Some(Hit::Inert));
        assert_eq!(m.at(7, 7), None);
    }

    #[test]
    fn radio_steps_count_from_selected() {
        let m = HitMap::default();
        let line = Line::from(vec![
            Span::raw(" T: "),
            Span::raw("○ a"),
            Span::raw(" "),
            Span::raw("◉ b"),
            Span::raw(" "),
            Span::raw("○ c"),
        ]);
        m.radio(&line, Rect::new(0, 3, 40, 1), |n| {
            Hit::OpCycle(OpField::TargetMode, n)
        });
        assert_eq!(m.at(4, 3), Some(Hit::OpCycle(OpField::TargetMode, 2)));
        assert_eq!(m.at(8, 3), Some(Hit::OpCycle(OpField::TargetMode, 0)));
        assert_eq!(m.at(12, 3), Some(Hit::OpCycle(OpField::TargetMode, 1)));
        assert_eq!(m.at(7, 3), None);
    }
}
