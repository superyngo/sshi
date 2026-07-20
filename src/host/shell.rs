//! Remote shell type detection (sh, PowerShell, cmd).

use super::session_pool::SessionPool;
use crate::config::schema::ShellType;

/// Detect shell type using an established session pool.
///
/// Takes a `&dyn SessionPool` so that test code can supply a `MockSessionPool`
/// with canned `exec` responses (audit §2.8 HIGH ×2 — see `host::session_pool`).
pub async fn detect_russh(
    host: &crate::config::schema::HostEntry,
    sessions: &dyn SessionPool,
    timeout: u64,
) -> anyhow::Result<crate::config::schema::ShellType> {
    let mut any_exec_ok = false;

    // Try PowerShell
    if let Ok(o) = sessions
        .exec(&host.ssh_host, "$PSVersionTable.PSVersion.Major", timeout)
        .await
    {
        any_exec_ok = true;
        if o.success && !o.stdout.trim().is_empty() {
            return Ok(ShellType::PowerShell);
        }
    }

    // Try CMD (Windows)
    if let Ok(o) = sessions.exec(&host.ssh_host, "ver", timeout).await {
        any_exec_ok = true;
        if o.success && o.stdout.contains("Windows") {
            return Ok(ShellType::Cmd);
        }
    }

    // Confirm POSIX shell is reachable before defaulting
    if let Ok(o) = sessions.exec(&host.ssh_host, "echo ok", timeout).await {
        any_exec_ok = true;
        if o.success {
            return Ok(ShellType::Sh);
        }
    }

    if !any_exec_ok {
        anyhow::bail!(
            "shell detection failed for {}: all exec attempts returned errors (session may be dropped)",
            host.ssh_host
        );
    }

    // exec succeeded but none of the markers matched — default to Sh
    Ok(ShellType::Sh)
}

/// Get the temporary directory path for a given shell type.
pub fn temp_dir(shell: ShellType) -> &'static str {
    match shell {
        ShellType::Sh => "/tmp",
        ShellType::PowerShell => "$env:TEMP",
        ShellType::Cmd => "%TEMP%",
    }
}

/// Wrap a command for sudo execution based on shell type.
pub fn sudo_wrap(shell: ShellType, command: &str) -> String {
    match shell {
        ShellType::Sh => format!("sudo {}", command),
        ShellType::PowerShell => {
            format!(
                "Start-Process powershell -ArgumentList '-Command {}' -Verb RunAs",
                command
            )
        }
        ShellType::Cmd => format!("runas /user:Administrator \"{}\"", command),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::ShellType;

    #[test]
    fn test_temp_dir_sh() {
        assert_eq!(temp_dir(ShellType::Sh), "/tmp");
    }

    #[test]
    fn test_temp_dir_powershell() {
        assert_eq!(temp_dir(ShellType::PowerShell), "$env:TEMP");
    }

    #[test]
    fn test_temp_dir_cmd() {
        assert_eq!(temp_dir(ShellType::Cmd), "%TEMP%");
    }

    #[test]
    fn test_sudo_wrap_sh() {
        let wrapped = sudo_wrap(ShellType::Sh, "apt update");
        assert_eq!(wrapped, "sudo apt update");
    }

    #[test]
    fn test_sudo_wrap_powershell() {
        let wrapped = sudo_wrap(ShellType::PowerShell, "Install-Module Foo");
        assert!(wrapped.contains("Start-Process powershell"));
        assert!(wrapped.contains("Install-Module Foo"));
        assert!(wrapped.contains("-Verb RunAs"));
    }

    #[test]
    fn test_sudo_wrap_cmd() {
        let wrapped = sudo_wrap(ShellType::Cmd, "net stop thing");
        assert!(wrapped.contains("runas /user:Administrator"));
        assert!(wrapped.contains("net stop thing"));
    }

    #[test]
    fn test_sudo_wrap_sh_empty_command() {
        let wrapped = sudo_wrap(ShellType::Sh, "");
        assert_eq!(wrapped, "sudo ");
    }

    #[test]
    fn test_sudo_wrap_all_variants_return_string() {
        for shell in [ShellType::Sh, ShellType::PowerShell, ShellType::Cmd] {
            let s = sudo_wrap(shell, "echo hi");
            assert!(!s.is_empty());
        }
    }
}
