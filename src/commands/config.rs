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

    editor_command(&editor, &path)
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

/// Command that opens `path` in `editor`. On Unix a value with arguments
/// (`code --wait`) runs through `sh -c '<editor> "$1"'`, as git does; a
/// plain program name runs directly (B74).
pub fn editor_command(editor: &str, path: &Path) -> Command {
    if cfg!(unix) && editor.trim().contains(char::is_whitespace) {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(format!("{editor} \"$1\""))
            .arg("sh")
            .arg(path);
        cmd
    } else {
        let mut cmd = Command::new(editor);
        cmd.arg(path);
        cmd
    }
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
    use super::{editor_command, editor_from};

    /// B74: an editor value with arguments runs through `sh -c`.
    #[cfg(unix)]
    #[test]
    fn editor_with_arguments_runs_through_sh() {
        let path = std::path::Path::new("/tmp/my config.toml");
        let cmd = editor_command("code --wait", path);
        assert_eq!(cmd.get_program(), "sh");
        let args: Vec<_> = cmd.get_args().collect();
        assert_eq!(
            args,
            ["-c", "code --wait \"$1\"", "sh", "/tmp/my config.toml"]
        );
        let plain = editor_command("vi", path);
        assert_eq!(plain.get_program(), "vi");
        assert_eq!(
            plain.get_args().collect::<Vec<_>>(),
            ["/tmp/my config.toml"]
        );
    }

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
