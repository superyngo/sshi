//! Host-prefixed colored line printer for CLI output.

use std::io::IsTerminal;

/// Determine whether ANSI colors should be used for stdout output.
///
/// Returns true if stdout is an interactive terminal and `NO_COLOR` is unset or empty.
pub fn should_color() -> bool {
    std::io::stdout().is_terminal() && no_color_allows()
}

/// Check if the `NO_COLOR` environment variable permits color.
///
/// Per https://no-color.org, any non-empty value disables color.
pub fn no_color_allows() -> bool {
    match std::env::var("NO_COLOR") {
        Ok(val) => val.is_empty(),
        Err(_) => true,
    }
}

/// Format a host-prefixed line with or without ANSI color.
pub fn format_host_line(host: &str, status: &str, detail: &str, color: bool) -> String {
    let max_name_len = 12;
    let padded = format!("{:width$}", host, width = max_name_len);
    let (symbol, color_code) = match status {
        "ok" => ("✓", "\x1b[32m"),    // green
        "error" => ("✗", "\x1b[31m"), // red
        "skip" => ("⊘", "\x1b[33m"),  // yellow
        _ => ("·", "\x1b[37m"),       // white
    };

    if color {
        format!("[{padded}]  {color_code}{symbol}\x1b[0m {detail}")
    } else {
        format!("[{padded}]  {symbol} {detail}")
    }
}

/// Print a host-prefixed line with color gated on stdout being a TTY and `NO_COLOR`.
pub fn print_host_line(host: &str, status: &str, detail: &str) {
    println!("{}", format_host_line(host, status, detail, should_color()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_host_line_colored() {
        let line = format_host_line("h1", "ok", "done", true);
        assert!(line.contains("\x1b[32m✓\x1b[0m"));
        assert!(line.starts_with("[h1          ]"));

        let line_err = format_host_line("server2", "error", "failed", true);
        assert!(line_err.contains("\x1b[31m✗\x1b[0m"));

        let line_skip = format_host_line("server3", "skip", "skipped", true);
        assert!(line_skip.contains("\x1b[33m⊘\x1b[0m"));

        let line_other = format_host_line("server4", "other", "unknown", true);
        assert!(line_other.contains("\x1b[37m·\x1b[0m"));
    }

    #[test]
    fn format_host_line_uncolored() {
        let line = format_host_line("h1", "ok", "done", false);
        assert!(!line.contains("\x1b"));
        assert!(line.contains("✓"));
        assert_eq!(line, "[h1          ]  ✓ done");

        let line_err = format_host_line("server2", "error", "failed", false);
        assert!(!line_err.contains("\x1b"));
        assert!(line_err.contains("✗"));
        assert_eq!(line_err, "[server2     ]  ✗ failed");

        let line_skip = format_host_line("server3", "skip", "skipped", false);
        assert!(!line_skip.contains("\x1b"));
        assert_eq!(line_skip, "[server3     ]  ⊘ skipped");

        let line_other = format_host_line("server4", "other", "unknown", false);
        assert!(!line_other.contains("\x1b"));
        assert_eq!(line_other, "[server4     ]  · unknown");
    }
}
