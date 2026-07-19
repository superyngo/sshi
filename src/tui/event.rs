//! Crossterm event helpers. The main loop uses `crossterm::event::EventStream`
//! (per G4, replacing the legacy 50ms `event::poll`). This module currently
//! only re-exports types and provides the `drain_non_resize` helper used by
//! the size-guard path (§7.8 step 2: discard non-Resize events while terminal
//! is too small).

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event};

/// Discard all pending crossterm events except `Resize` (§7.8 step 2).
#[allow(dead_code)]
pub fn drain_non_resize() -> Result<()> {
    while event::poll(Duration::ZERO)? {
        match event::read()? {
            Event::Resize(_, _) => {
                // Re-queue a synthetic resize? crossterm has already
                // delivered the event via read(); the caller handles
                // it via the next poll cycle. Just stop draining here.
                return Ok(());
            }
            _ => continue,
        }
    }
    Ok(())
}
