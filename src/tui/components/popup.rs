//! Centered modal overlay helper (per docs/spec/2026-05-06-tui-reconstruct.md §6.3).

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// A `width` × `height` rectangle centred in `area`, clamped to fit (saturating).
pub fn fixed_centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

/// Centered popup whose *inner* area is `inner_w` × `inner_h` (plus the
/// border), clamped to `area` less a 1-cell margin on every side.
pub fn content_rect(inner_w: usize, inner_h: usize, area: Rect) -> Rect {
    let w = (inner_w.saturating_add(2)).min(area.width.saturating_sub(2) as usize) as u16;
    let h = (inner_h.saturating_add(2)).min(area.height.saturating_sub(2) as usize) as u16;
    fixed_centered_rect(w, h, area)
}

/// Compute a centered rectangle with `percent_x` × `percent_y` of `area`.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::content_rect;
    use ratatui::layout::Rect;

    /// B82: popups take their content size plus border, never past the
    /// screen less a 1-cell margin.
    #[test]
    fn content_rect_fits_content_then_clamps() {
        let area = Rect::new(0, 0, 80, 24);
        assert_eq!(content_rect(20, 3, area), Rect::new(29, 9, 22, 5));
        assert_eq!(content_rect(500, 500, area), Rect::new(1, 1, 78, 22));
        assert_eq!(content_rect(5, 5, Rect::new(0, 0, 1, 1)).width, 0);
    }
}
