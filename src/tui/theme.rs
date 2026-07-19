//! Canonical TUI palette (per docs/tui_reconstruct_plan.md §10).
//!
//! 16-color compatible: only ratatui named `Color` variants — no Rgb / Indexed.
//!
//! `Theme::from_env()` honours `NO_COLOR` (per https://no-color.org) and
//! `TERM=linux` (Linux console) to drop colour / fall back to ASCII glyphs
//! (audit §1 P20 HIGH ×2).

use crate::commands::report::HostStatus;
use ratatui::style::Color;

/// Status / outcome glyphs. `Unicode` is the default; `Ascii` is used when
/// `TERM=linux` or another dumb-Unicode terminal is detected so the Linux
/// console doesn't render `?` / empty boxes (audit §1 P20 HIGH).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlyphSet {
    pub ok: &'static str,
    pub error: &'static str,
    pub skip: &'static str,
    pub warn: &'static str,
}

impl GlyphSet {
    pub const fn unicode() -> Self {
        Self {
            ok: "✓",
            error: "✗",
            skip: "⊘",
            warn: "⚠",
        }
    }

    pub const fn ascii() -> Self {
        Self {
            ok: "+",
            error: "x",
            skip: "o",
            warn: "!",
        }
    }

    /// Pick the status glyph for a `HostStatus`. `TimedOut` has no ASCII
    /// pair in the E3 spec — its `⏱` literal is returned unchanged so
    /// `TERM=linux` degrades only the four spec'd glyphs.
    pub fn for_status(&self, status: HostStatus) -> &'static str {
        match status {
            HostStatus::Online => self.ok,
            HostStatus::Partial => self.warn,
            HostStatus::Offline | HostStatus::Error => self.error,
            HostStatus::Unreachable | HostStatus::Skipped => self.skip,
            HostStatus::TimedOut => "⏱",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub accent_config: Color,
    pub accent_operate: Color,
    pub accent_checkout: Color,
    pub error: Color,
    pub warning: Color,
    pub inactive: Color,
    pub border_active: Color,
    pub border_inactive: Color,
    pub glyphs: GlyphSet,
}

impl Theme {
    pub const fn default_palette() -> Self {
        Self {
            accent_config: Color::Yellow,
            accent_operate: Color::Cyan,
            accent_checkout: Color::Green,
            error: Color::Red,
            warning: Color::Yellow,
            inactive: Color::DarkGray,
            border_active: Color::Cyan,
            border_inactive: Color::DarkGray,
            glyphs: GlyphSet::unicode(),
        }
    }

    /// Build a Theme from process environment, honouring:
    /// - `NO_COLOR` (per https://no-color.org): if present and non-empty,
    ///   all colours become `Color::Reset` (no ANSI colour escapes).
    /// - `TERM=linux`: the Linux console has poor Unicode coverage; switch
    ///   to the ASCII glyph set.
    ///
    /// The two signals are independent: `NO_COLOR=` (empty) does not trigger
    /// the no-colour path; a coloured theme can still use ASCII glyphs and
    /// vice versa.
    pub fn from_env() -> Self {
        let no_color = std::env::var_os("NO_COLOR")
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        let ascii_glyphs = matches!(std::env::var("TERM").ok().as_deref(), Some("linux"));

        let mut theme = if no_color {
            Self::no_color()
        } else {
            Self::default_palette()
        };
        if ascii_glyphs {
            theme.glyphs = GlyphSet::ascii();
        }
        theme
    }

    /// Monochrome palette: every colour set to `Color::Reset` so ratatui
    /// emits no ANSI colour escapes. Glyphs stay Unicode unless the caller
    /// swaps in `GlyphSet::ascii()`.
    pub const fn no_color() -> Self {
        Self {
            accent_config: Color::Reset,
            accent_operate: Color::Reset,
            accent_checkout: Color::Reset,
            error: Color::Reset,
            warning: Color::Reset,
            inactive: Color::Reset,
            border_active: Color::Reset,
            border_inactive: Color::Reset,
            glyphs: GlyphSet::unicode(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GlyphSet, Theme};
    use ratatui::style::Color;

    // Env-mutating tests share process-global state and race under cargo's
    // default parallel test runner. Serialise them through a module-level
    // mutex so the set_var/remove_var windows don't observe each other.
    static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn clear_env() -> std::sync::MutexGuard<'static, ()> {
        let guard = ENV_TEST_LOCK.lock().unwrap();
        std::env::remove_var("NO_COLOR");
        std::env::remove_var("TERM");
        guard
    }

    #[test]
    fn glyph_set_unicode_default_matches_legacy_strings() {
        let g = GlyphSet::unicode();
        assert_eq!(g.ok, "✓");
        assert_eq!(g.error, "✗");
        assert_eq!(g.skip, "⊘");
        assert_eq!(g.warn, "⚠");
    }

    #[test]
    fn glyph_set_ascii_fallback_pairs_match_spec() {
        // E3 spec: ✓→+, ✗→x, ⊘→o, ⚠→!
        let g = GlyphSet::ascii();
        assert_eq!(g.ok, "+");
        assert_eq!(g.error, "x");
        assert_eq!(g.skip, "o");
        assert_eq!(g.warn, "!");
    }

    #[test]
    fn no_color_env_switches_to_reset_palette() {
        let _guard = clear_env();
        std::env::set_var("NO_COLOR", "1");
        let t = Theme::from_env();
        assert_eq!(t.accent_config, Color::Reset);
        assert_eq!(t.error, Color::Reset);
        assert_eq!(t.border_active, Color::Reset);
        // NO_COLOR is orthogonal to glyph set: Unicode still default.
        assert_eq!(t.glyphs, GlyphSet::unicode());
    }

    #[test]
    fn no_color_empty_string_does_not_trigger_monochrome() {
        // Per https://no-color.org: NO_COLOR must be "present and not an
        // empty string" — empty value is the explicit opt-out.
        let _guard = clear_env();
        std::env::set_var("NO_COLOR", "");
        let t = Theme::from_env();
        assert_eq!(
            t.accent_config,
            Color::Yellow,
            "empty NO_COLOR keeps colour"
        );
    }

    #[test]
    fn term_linux_switches_to_ascii_glyphs() {
        let _guard = clear_env();
        std::env::set_var("TERM", "linux");
        let t = Theme::from_env();
        assert_eq!(t.glyphs, GlyphSet::ascii());
        // Colours are independent — TERM=linux alone keeps coloured palette.
        assert_eq!(t.accent_checkout, Color::Green);
    }

    #[test]
    fn term_xterm_256color_keeps_unicode_glyphs() {
        let _guard = clear_env();
        std::env::set_var("TERM", "xterm-256color");
        let t = Theme::from_env();
        assert_eq!(t.glyphs, GlyphSet::unicode());
    }

    #[test]
    fn no_color_plus_term_linux_combines_both() {
        let _guard = clear_env();
        std::env::set_var("NO_COLOR", "1");
        std::env::set_var("TERM", "linux");
        let t = Theme::from_env();
        assert_eq!(t.accent_config, Color::Reset);
        assert_eq!(t.glyphs, GlyphSet::ascii());
    }

    #[test]
    fn default_palette_keeps_unicode_and_colour_when_env_absent() {
        let _guard = clear_env();
        let t = Theme::from_env();
        assert_eq!(t.glyphs, GlyphSet::unicode());
        assert_eq!(t.accent_config, Color::Yellow);
        assert_eq!(t.error, Color::Red);
    }

    #[test]
    fn for_status_returns_correct_glyph_per_variant() {
        use crate::commands::report::HostStatus;
        let g = GlyphSet::unicode();
        assert_eq!(g.for_status(HostStatus::Online), "✓");
        assert_eq!(g.for_status(HostStatus::Partial), "⚠");
        assert_eq!(g.for_status(HostStatus::Offline), "✗");
        assert_eq!(g.for_status(HostStatus::Error), "✗");
        assert_eq!(g.for_status(HostStatus::Unreachable), "⊘");
        assert_eq!(g.for_status(HostStatus::Skipped), "⊘");
        assert_eq!(g.for_status(HostStatus::TimedOut), "⏱");

        let a = GlyphSet::ascii();
        assert_eq!(a.for_status(HostStatus::Online), "+");
        assert_eq!(a.for_status(HostStatus::Partial), "!");
        assert_eq!(a.for_status(HostStatus::Offline), "x");
        assert_eq!(a.for_status(HostStatus::Error), "x");
        assert_eq!(a.for_status(HostStatus::Unreachable), "o");
        assert_eq!(a.for_status(HostStatus::Skipped), "o");
        assert_eq!(a.for_status(HostStatus::TimedOut), "⏱");
    }
}
