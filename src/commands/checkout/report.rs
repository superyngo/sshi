//! Typed contracts for `checkout_core`: a typed
//! [`CheckoutReport`] carrying the snapshots read from the DB plus the
//! resolved display columns.
//!
//! Per audit §2.2 HIGH: `checkout_core` returns this typed struct so the
//! CLI wrapper (and future TUI Phase E popup) can format without reaching
//! into raw DB rows.

use serde::Serialize;

use super::core::HostSnapshot;

/// Typed outcome of a `checkout_core` invocation.
#[derive(Debug, Clone, Serialize)]
pub struct CheckoutReport {
    /// RFC 3339 timestamp captured at the start of the read.
    pub executed_at: String,
    /// `true` when the caller asked for the per-metric combined view.
    pub combined_view: bool,
    /// Names of all targeted hosts (in resolve order).
    pub targets: Vec<String>,
    /// Display-column metric names (excludes `online`).
    pub columns: Vec<String>,
    /// One snapshot row per target host.
    pub hosts: Vec<HostSnapshot>,
}
