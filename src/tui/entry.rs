//! TUI entry point: `run_or_fallback` performs §4 TTY/TERM detection and
//! either launches the TUI (exit 0 on clean quit) or prints clap help and
//! exits 2 (clap convention for non-TTY).

use std::io::IsTerminal;
use std::path::Path;
use std::process;

use anyhow::Result;

use crate::commands::Context;

use super::app::App;
use super::terminal::{install_panic_hook, TerminalGuard};

/// §4 entry: detect TTY/TERM, then either launch TUI or fall back to help.
///
/// Exit codes (stable contract):
///   0 — clean TUI quit
///   1 — TUI requested but unavailable (handled by caller in main.rs)
///   2 — non-TTY environment, help printed
pub async fn run_or_fallback(verbose: bool, config_path: Option<&Path>) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        crate::cli::print_help_with_newline();
        process::exit(2);
    }

    #[cfg(unix)]
    {
        let term = std::env::var("TERM").ok();
        let term_unsuitable = matches!(term.as_deref(), None | Some("") | Some("dumb"));
        if term_unsuitable {
            crate::cli::print_tui_unavailable_help(
                "Terminal does not support TUI (TERM=dumb or unset).",
            );
            process::exit(2);
        }
    }

    let _ = verbose; // tracing already initialised; verbose only affects level filter
    let config = crate::config::app::load(config_path)?.unwrap_or_default();
    let conn = crate::state::db::open(config.settings.state_dir.as_deref())?;
    let db = crate::state::db::DbHandle::new(conn);
    let timeout = config.settings.default_timeout;
    let ctx = Context {
        config: std::sync::Arc::new(config),
        config_path: config_path.map(|p| p.to_path_buf()),
        db,
        timeout,
        mode: crate::commands::TargetMode::All,
        serial: false,
        skip: Vec::new(),
        verbose,
        auth_sender: None,
    };

    // Background operations share this terminal; silence their CLI progress
    // bars so they don't draw over the alternate-screen TUI.
    crate::output::progress::suppress();

    install_panic_hook();
    let _guard = TerminalGuard::install()?;

    let mut app = App::from_context(&ctx, None);
    app.run().await
}
