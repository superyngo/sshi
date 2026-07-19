//! Focus model + adaptive arrow navigation (per docs/tui_reconstruct_plan.md
//! §8.2 / §8.3 / §8.6).
//!
//! - Arrow keys drive cross-level transitions via `escape_to_parent`.
//! - Tab / Shift+Tab cycle peers within the **current level only** and never
//!   escape level boundaries.
//!
//! Note: the `Focusable` trait that previously lived here was deleted
//! (audit §1 P4 LOW, Phase F3) — it had zero non-test callers. The
//! surviving types (`Direction`, `Axis`, `AxisFreedom`, `FocusZone`,
//! `EscapeOutcome`, `escape_to_parent`, `FocusPath`) describe the focus
//! model a future dispatch could be rebuilt on; they remain dead in
//! the meantime and are silenced by the module-level allow below.

#![allow(dead_code)]

use super::tabs::TabId;

/// Direction of an arrow keypress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    pub fn axis(self) -> Axis {
        match self {
            Direction::Up | Direction::Down => Axis::Y,
            Direction::Left | Direction::Right => Axis::X,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

/// What kinds of arrow keys a focused element absorbs internally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisFreedom {
    /// No internal movement — any arrow escapes immediately.
    None,
    /// Only ↑↓ move internally (e.g. vertical list).
    Y,
    /// Only ←→ move internally (e.g. horizontal radio row).
    X,
    /// Both axes move internally (e.g. grid).
    XY,
}

impl AxisFreedom {
    /// Does this element absorb the given direction internally (when not at
    /// a boundary)? If false, the keypress always escapes.
    pub fn absorbs(self, dir: Direction) -> bool {
        match self {
            AxisFreedom::None => false,
            AxisFreedom::Y => matches!(dir, Direction::Up | Direction::Down),
            AxisFreedom::X => matches!(dir, Direction::Left | Direction::Right),
            AxisFreedom::XY => true,
        }
    }
}

/// Per-tab logical zones (per §8.6). Zone IDs map to render layout.
///
/// MVP scope: only the zones needed by Phase 1a's tabs are populated.
/// Later phases extend this enum (Operate sub-zones in Phase 3, Config
/// sub-zones in Phase 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusZone {
    // Checkout tab
    CheckoutControls,
    CheckoutHostTable,
    // Operate tab placeholder
    OperatePlaceholder,
    // Config tab placeholder
    ConfigPlaceholder,
}

impl FocusZone {
    pub fn for_tab(tab: TabId) -> FocusZone {
        match tab {
            TabId::Config => FocusZone::ConfigPlaceholder,
            TabId::Operate => FocusZone::OperatePlaceholder,
            TabId::View => FocusZone::CheckoutHostTable,
        }
    }

    /// Human-readable label for breadcrumb display.
    pub fn label(self) -> &'static str {
        match self {
            FocusZone::CheckoutControls => "Controls",
            FocusZone::CheckoutHostTable => "Rows",
            FocusZone::OperatePlaceholder => "(placeholder)",
            FocusZone::ConfigPlaceholder => "(placeholder)",
        }
    }
}

/// Outcome of an `escape_to_parent` resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeOutcome {
    /// Move to a sibling zone within the same tab.
    SwitchZone(FocusZone),
    /// Wrap at L0 (tab bar) — change the active tab.
    SwitchTab(TabId),
    /// No movement — boundary is sealed (popup root, side edge of zone table).
    Stop,
}

/// Resolve an escape from `from` zone in `dir`, given the current active tab.
///
/// Implements the §8.6 zone neighbour tables. Tab-bar wrap (L0) is handled
/// elsewhere (Tab/Shift+Tab paths); this function only handles arrow-driven
/// zone transitions.
pub fn escape_to_parent(tab: TabId, from: FocusZone, dir: Direction) -> EscapeOutcome {
    match (tab, from, dir) {
        // Checkout tab: Controls ↔ HostTable on ↑↓.
        (TabId::View, FocusZone::CheckoutControls, Direction::Down) => {
            EscapeOutcome::SwitchZone(FocusZone::CheckoutHostTable)
        }
        (TabId::View, FocusZone::CheckoutHostTable, Direction::Up) => {
            EscapeOutcome::SwitchZone(FocusZone::CheckoutControls)
        }
        // All other arrows in Checkout zones are sealed.
        // Placeholder tabs have no sub-zones to cross to.
        _ => EscapeOutcome::Stop,
    }
}

/// `FocusPath` ties the active focus zone to its breadcrumb trail
/// (per §6.2). Breadcrumb updates only on zone change, never on plain ↑↓.
#[derive(Debug, Clone)]
pub struct FocusPath {
    pub zone: FocusZone,
    pub breadcrumb: Vec<String>,
}

impl FocusPath {
    pub fn for_tab(tab: TabId) -> Self {
        let zone = FocusZone::for_tab(tab);
        let breadcrumb = vec![tab.label().to_string(), zone.label().to_string()];
        Self { zone, breadcrumb }
    }

    pub fn switch_zone(&mut self, tab: TabId, zone: FocusZone) {
        self.zone = zone;
        self.breadcrumb = vec![tab.label().to_string(), zone.label().to_string()];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_to_parent_checkout_table() {
        // Checkout tab: HostTable + Up → Controls; HostTable + Down → Stop.
        assert_eq!(
            escape_to_parent(TabId::View, FocusZone::CheckoutHostTable, Direction::Up),
            EscapeOutcome::SwitchZone(FocusZone::CheckoutControls),
        );
        assert_eq!(
            escape_to_parent(TabId::View, FocusZone::CheckoutHostTable, Direction::Down),
            EscapeOutcome::Stop,
        );
        assert_eq!(
            escape_to_parent(TabId::View, FocusZone::CheckoutHostTable, Direction::Left),
            EscapeOutcome::Stop,
        );
        assert_eq!(
            escape_to_parent(TabId::View, FocusZone::CheckoutControls, Direction::Down),
            EscapeOutcome::SwitchZone(FocusZone::CheckoutHostTable),
        );
        assert_eq!(
            escape_to_parent(TabId::View, FocusZone::CheckoutControls, Direction::Up),
            EscapeOutcome::Stop,
        );
    }

    #[test]
    fn breadcrumb_updates_on_zone_change() {
        let mut fp = FocusPath::for_tab(TabId::View);
        let initial = fp.breadcrumb.clone();
        fp.switch_zone(TabId::View, FocusZone::CheckoutControls);
        assert_ne!(initial, fp.breadcrumb);
        assert_eq!(fp.breadcrumb.last().unwrap(), "Controls");
    }
}
