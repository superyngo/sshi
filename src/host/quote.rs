//! Shared remote-quoting layer (B26). Every value interpolated into a remote
//! command string goes through here, so a path or label containing spaces,
//! quotes, `$(...)`, backticks, `;`, `&` or `%` stays one literal argument.

use anyhow::{bail, Result};

use crate::config::schema::ShellType;

/// Quote `s` as one literal argument for `shell`. No expansion of any kind.
///
/// - sh: `'…'`, embedded `'` as `'\''`.
/// - PowerShell: `'…'`, embedded `'` as `''`.
/// - Cmd: `"…"`; values containing `"`, `%`, `!`, CR or LF are refused because
///   cmd has no reliable escape for them inside quotes.
pub fn quote_arg(shell: ShellType, s: &str) -> Result<String> {
    Ok(match shell {
        ShellType::Sh => format!("'{}'", s.replace('\'', "'\\''")),
        ShellType::PowerShell => format!("'{}'", s.replace('\'', "''")),
        ShellType::Cmd => {
            check_cmd_safe(s)?;
            format!("\"{s}\"")
        }
    })
}

/// Like [`quote_arg`], but a leading `~` or `~/` becomes the remote home
/// directory (sh `"$HOME"`, PowerShell `$HOME`, Cmd `%USERPROFILE%`); the rest
/// of the path stays literal, and `/` in it becomes `\` for PowerShell and Cmd.
/// Paths without a leading `~` are quoted exactly as [`quote_arg`] would.
pub fn quote_path(shell: ShellType, path: &str) -> Result<String> {
    let rest = if path == "~" {
        Some("")
    } else {
        path.strip_prefix("~/")
    };
    Ok(match (shell, rest) {
        (ShellType::Sh, None) => quote_arg(shell, path)?,
        (ShellType::Sh, Some("")) => "\"$HOME\"".to_string(),
        (ShellType::Sh, Some(r)) => format!("\"$HOME\"/{}", quote_arg(shell, r)?),
        (ShellType::PowerShell, None) => quote_arg(shell, path)?,
        (ShellType::PowerShell, Some("")) => "$HOME".to_string(),
        (ShellType::PowerShell, Some(r)) => {
            format!(
                "($HOME + {})",
                quote_arg(shell, &format!("\\{}", r.replace('/', "\\")))?
            )
        }
        (ShellType::Cmd, None) => quote_arg(shell, path)?,
        (ShellType::Cmd, Some(r)) => {
            check_cmd_safe(r)?;
            format!("\"%USERPROFILE%\\{}\"", r.replace('/', "\\"))
        }
    })
}

/// Escape `s` for a Cmd `echo` (unquoted): `^` before each of `^&|<>()`.
/// Refuses the same characters as [`quote_arg`].
pub fn cmd_echo_arg(s: &str) -> Result<String> {
    check_cmd_safe(s)?;
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "^&|<>()".contains(c) {
            out.push('^');
        }
        out.push(c);
    }
    Ok(out)
}

fn check_cmd_safe(s: &str) -> Result<()> {
    if let Some(c) = s
        .chars()
        .find(|c| matches!(c, '"' | '%' | '!' | '\r' | '\n'))
    {
        bail!("{s:?} contains {c:?}, which cannot be passed safely to a cmd.exe host");
    }
    Ok(())
}

/// Run a PowerShell `script` from cmd.exe without a second quoting layer:
/// `powershell -NoProfile -EncodedCommand <base64 UTF-16LE>`.
pub fn ps_in_cmd(script: &str) -> String {
    format!(
        "powershell -NoProfile -EncodedCommand {}",
        encode_ps(script)
    )
}

/// Base64 of the UTF-16LE bytes of `script`, as PowerShell's
/// `-EncodedCommand` expects. The alphabet (`A-Za-z0-9+/=`) needs no quoting
/// in any shell.
pub fn encode_ps(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const NASTY: &[&str] = &[
        "plain",
        "a b",
        "it's",
        "say \"hi\"",
        "$(touch PWNED)",
        "`touch PWNED`",
        "x;touch PWNED",
        "x & echo PWNED",
        "%PATH%",
        "a|b>c",
    ];

    /// Parity: every shell either quotes each nasty value or refuses it.
    /// sh output is executed by a real `sh`, which must echo it back intact
    /// and never run the injected command.
    #[test]
    fn every_shell_quotes_or_refuses_every_nasty_value() {
        for shell in [ShellType::Sh, ShellType::PowerShell, ShellType::Cmd] {
            for v in NASTY {
                match quote_arg(shell, v) {
                    Ok(q) => match shell {
                        ShellType::Sh => assert_eq!(run_sh(&format!("printf %s {q}")), *v),
                        ShellType::PowerShell => {
                            assert!(q.starts_with('\'') && q.ends_with('\''));
                            assert!(!q[1..q.len() - 1].replace("''", "").contains('\''), "{q}");
                        }
                        ShellType::Cmd => {
                            assert!(!q[1..q.len() - 1].contains(['"', '%', '!']), "{q}")
                        }
                    },
                    Err(_) => assert_eq!(shell, ShellType::Cmd, "only Cmd may refuse {v:?}"),
                }
            }
        }
    }

    /// Site parity (B26): every command builder that interpolates a remote
    /// path, for every shell. sh commands run in a real `sh` (HOME and cwd a
    /// temp dir) and must not create PWNED; PowerShell and Cmd output must not
    /// contain the payload outside a quoted literal / base64 blob.
    #[test]
    fn every_site_neutralises_injection_for_every_shell() {
        use crate::commands::sync::collect::{build_batch_metadata_cmd, build_dir_expand_cmd};
        use crate::metrics::probes::batch_path_command;
        let path = "~/a b/$(touch PWNED);`touch PWNED`".to_string();
        let probe = [(path.clone(), "l$(touch PWNED)".to_string())];
        for shell in [ShellType::Sh, ShellType::PowerShell, ShellType::Cmd] {
            let mut cmds = vec![
                build_batch_metadata_cmd(std::slice::from_ref(&path), shell),
                build_dir_expand_cmd(std::slice::from_ref(&path), true, shell),
            ];
            match batch_path_command(shell, &probe) {
                Ok(c) => cmds.push(c),
                Err(_) => assert_eq!(shell, ShellType::Cmd),
            }
            for c in cmds {
                match shell {
                    // Echoed back verbatim (path or label), never executed.
                    ShellType::Sh if cfg!(unix) => {
                        let out = run_sh(&c);
                        assert!(out.contains("$(touch PWNED)"), "{c}\n{out}");
                    }
                    ShellType::Sh => {}
                    ShellType::PowerShell => {
                        // Payload only ever appears inside a '…' literal.
                        let outside: String = c.split('\'').step_by(2).collect::<Vec<_>>().join("");
                        assert!(!outside.contains("touch PWNED"), "{c}");
                    }
                    // PowerShell-in-cmd is pure base64; the native probe keeps the
                    // path in one "…" (cmd has no $() / backticks) and ^-escapes
                    // the label's metacharacters.
                    ShellType::Cmd if c.starts_with("powershell") => {
                        assert!(!c.contains("PWNED"), "{c}")
                    }
                    ShellType::Cmd => {
                        assert!(
                            c.contains("\"%USERPROFILE%\\a b\\$(touch PWNED);`touch PWNED`\""),
                            "{c}"
                        );
                        assert!(c.contains("---PATH:l$^(touch PWNED^) &"), "{c}");
                    }
                }
            }
        }
    }

    #[cfg(unix)]
    fn run_sh(script: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let out = std::process::Command::new("sh")
            .arg("-c")
            .arg(script)
            .current_dir(dir.path())
            .env("HOME", dir.path())
            .output()
            .unwrap();
        assert!(
            !dir.path().join("PWNED").exists(),
            "injection ran: {script}"
        );
        String::from_utf8(out.stdout).unwrap()
    }

    #[cfg(not(unix))]
    fn run_sh(_: &str) -> String {
        String::new()
    }

    #[cfg(unix)]
    #[test]
    fn sh_quote_path_expands_only_leading_tilde() {
        let out = run_sh(&format!(
            "HOME=/h; printf '%s|' {} {} {}",
            quote_path(ShellType::Sh, "~/a b/$(touch PWNED)").unwrap(),
            quote_path(ShellType::Sh, "~").unwrap(),
            quote_path(ShellType::Sh, "/x/~/y").unwrap()
        ));
        assert_eq!(out, "/h/a b/$(touch PWNED)|/h|/x/~/y|");
    }

    #[test]
    fn ps_and_cmd_quote_path_forms() {
        assert_eq!(
            quote_path(ShellType::PowerShell, "~/d/it's").unwrap(),
            "($HOME + '\\d\\it''s')"
        );
        assert_eq!(quote_path(ShellType::PowerShell, "C:/x").unwrap(), "'C:/x'");
        assert_eq!(
            quote_path(ShellType::Cmd, "~/a b").unwrap(),
            "\"%USERPROFILE%\\a b\""
        );
        assert!(quote_path(ShellType::Cmd, "~/%TEMP%").is_err());
        assert_eq!(cmd_echo_arg("a&b(c)").unwrap(), "a^&b^(c^)");
    }

    #[test]
    fn encode_ps_matches_powershell_encoding() {
        // [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes('dir'))
        assert_eq!(encode_ps("dir"), "ZABpAHIA");
        assert_eq!(encode_ps("a"), "YQA=");
        assert_eq!(encode_ps(""), "");
        assert!(ps_in_cmd("$x=\"%P%\"").ends_with(&encode_ps("$x=\"%P%\"")));
    }
}
