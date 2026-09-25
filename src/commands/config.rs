//! Open or print the path to the sshi configuration file.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

use crate::config::app;

pub async fn run(config_path: Option<&Path>) -> Result<()> {
    let path = app::resolve_path(config_path)?;

    if !path.exists() {
        bail!(
            "Config file not found at {}\nRun 'sshi init' first to create it.",
            path.display()
        );
    }

    let editor = resolve_editor();

    Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("Failed to open editor '{}'", editor))?;

    Ok(())
}

/// The editor for `sshi config` and the TUI `E` key: `$VISUAL`, then
/// `$EDITOR` (empty values skipped), then `notepad` on Windows / `vi`
/// elsewhere — one order for both entry points (B17).
pub fn resolve_editor() -> String {
    editor_from(|name| std::env::var(name).ok())
}

fn editor_from(var: impl Fn(&str) -> Option<String>) -> String {
    ["VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(var)
        .find(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::editor_from;

    #[test]
    fn visual_then_editor_then_default() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| v.to_string())
            }
        };
        assert_eq!(
            editor_from(env(&[("VISUAL", "code"), ("EDITOR", "nano")])),
            "code"
        );
        assert_eq!(editor_from(env(&[("EDITOR", "nano")])), "nano");
        assert_eq!(
            editor_from(env(&[("VISUAL", " "), ("EDITOR", "nano")])),
            "nano"
        );
        let default = if cfg!(windows) { "notepad" } else { "vi" };
        assert_eq!(editor_from(env(&[])), default);
    }
}
