//! Typed contracts for `init_core`: the call plan (answers to interactive
//! questions, captured up-front by the CLI wrapper) and the typed
//! `InitReport` returned to the wrapper for summary printing and persistence
//! narration.
//!
//! Per audit §2.2 HIGH and §2.5 HIGH: `init_core` itself is non-interactive
//! — the TUI cannot answer y/n prompts. All yes/no decisions captured by the
//! CLI wrapper up-front are encoded as fields on [`InitPlan`].

use std::path::PathBuf;

use serde::Serialize;

use crate::config::schema::HostEntry;
use crate::output::summary::Summary;

/// Decisions collected by the CLI wrapper before invoking [`crate::commands::init::core::init_core`].
///
/// Persists the persistence-time decisions (`dry_run`, `skip`,
/// `remove_stale_hosts`, `update`) that `init_core`'s detect-and-persist
/// phase still needs after all per-host retry helpers have run. The
/// interactive answers (`accept_unknown_host_keys`,
/// `generate_ssh_key_if_missing`, `copy_id_targets`) are consumed directly
/// by the wrapper-orchestrated helpers and are carried here only so a
/// future TUI popup can populate the full plan from a single interaction
/// surface.
#[derive(Debug, Clone, Default)]
pub struct InitPlan {
    /// `--dry-run`: skips the persist step. When `true`, `init_core`
    /// returns a report describing what would have happened without
    /// writing `~/.config/sshi/config.toml`.
    pub dry_run: bool,
    /// `--update`: re-detect shell on hosts already present in sshi config.
    /// The CLI wrapper ORs this with the "config already exists" condition
    /// before passing it in.
    pub update: bool,
    /// `--skip` values merged with `settings.skipped_hosts` from the loaded
    /// config. New skips are persisted.
    pub skip: Vec<String>,
    /// User answered "y" to "Remove these N host(s) from sshi config?".
    /// Always `false` when `dry_run == true`.
    pub remove_stale_hosts: bool,
    /// User answered "y" to "Add to known_hosts and retry?". Consumed by
    /// [`offer_keyscan_retry`](super::core::offer_keyscan_retry); carried
    /// here for future TUI single-shot plan population.
    pub accept_unknown_host_keys: bool,
    /// User answered "y" to "No SSH key found. Create one?". Consumed by
    /// [`offer_ssh_copy_id_retry`](super::core::offer_ssh_copy_id_retry).
    pub generate_ssh_key_if_missing: bool,
    /// Subset of auth-failed hosts the user agreed to copy their public key
    /// to via `ssh-copy-id`. Consumed by
    /// [`offer_ssh_copy_id_retry`](super::core::offer_ssh_copy_id_retry).
    pub copy_id_targets: Vec<String>,
}

/// Typed outcome of an `init_core` invocation. The CLI wrapper consumes this
/// to print the post-run summary; a future TUI Phase E consumes it to render
/// a status popup.
#[derive(Debug, Clone, Serialize)]
pub struct InitReport {
    /// RFC 3339 timestamp captured at the start of the run.
    pub executed_at: String,
    /// Total hosts read from `~/.ssh/config` (after wildcard filtering).
    pub ssh_hosts_count: usize,
    /// Hosts skipped via `--skip` or `settings.skipped_hosts`.
    pub skipped_hosts: Vec<String>,
    /// Hosts whose shell type was successfully detected and that will be
    /// upserted into sshi config.
    pub detected_hosts: Vec<InitDetectedHost>,
    /// Host names in sshi config that no longer appear in `~/.ssh/config`.
    /// Captured for reporting even when not removed.
    pub stale_host_names: Vec<String>,
    /// True when `InitPlan::remove_stale_hosts` was honoured.
    pub stale_hosts_removed: bool,
    /// Hosts that failed shell detection (unreachable, keyscan failed, etc.).
    pub failed_hosts: Vec<InitFailedHost>,
    /// `true` when the legacy CLI path should emit `Summary::print()` before
    /// the "Config saved" line. Mirrors the original three save branches:
    /// paths A (detect non-empty) and B (skip-only) print the summary;
    /// path C (stale-only) does not.
    pub summary_should_print: bool,
    /// `true` when the CLI wrapper should print "No new hosts to add." and
    /// return early without saving.
    pub no_changes: bool,
    /// `Some(path)` when sshi config was saved; `None` when dry-run or
    /// nothing changed.
    pub persisted_path: Option<PathBuf>,
    /// Whether `--dry-run` was in effect (echoes `InitPlan::dry_run`).
    pub dry_run: bool,
    /// Accumulated legacy summary (succeeded/failed/skipped tallies + error
    /// entries). The CLI wrapper calls `.print()` on it; the TUI can ignore
    /// it in favour of the typed fields above.
    #[serde(skip)]
    pub summary: Summary,
    /// `HostEntry` rows upserted into sshi config (parallel to
    /// `detected_hosts`, but carrying the full typed entry). Used by the
    /// core's own persistence step; carried in the report so Phase E can
    /// show a diff.
    #[serde(skip)]
    pub new_hosts: Vec<HostEntry>,
}

/// Per-host successful shell detection entry.
#[derive(Debug, Clone, Serialize)]
pub struct InitDetectedHost {
    pub host: String,
    /// `Sh` | `PowerShell` | `Cmd` rendered as the canonical shell name.
    pub shell: String,
}

/// Per-host failure entry.
#[derive(Debug, Clone, Serialize)]
pub struct InitFailedHost {
    pub host: String,
    pub detail: String,
}
