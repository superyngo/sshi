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
///
/// Refuses Windows hosts (PowerShell and Cmd) because `Start-Process -Verb RunAs`
/// and `runas` cannot observe the elevated process exit status or capture stdout/stderr (B31).
pub fn sudo_wrap(shell: ShellType, command: &str) -> anyhow::Result<String> {
    match shell {
        ShellType::Sh => Ok(format!("sudo {}", command)),
        ShellType::PowerShell | ShellType::Cmd => {
            anyhow::bail!(
                "--sudo is not supported on {} hosts (cannot observe elevated exit status)",
                shell
            )
        }
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
        let wrapped = sudo_wrap(ShellType::Sh, "apt update").unwrap();
        assert_eq!(wrapped, "sudo apt update");
    }

    #[test]
    fn test_sudo_wrap_powershell_refused() {
        let res = sudo_wrap(ShellType::PowerShell, "Install-Module Foo");
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("not supported on powershell"));
    }

    #[test]
    fn test_sudo_wrap_cmd_refused() {
        let res = sudo_wrap(ShellType::Cmd, "net stop thing");
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("not supported on cmd"));
    }

    #[test]
    fn test_sudo_wrap_sh_empty_command() {
        let wrapped = sudo_wrap(ShellType::Sh, "").unwrap();
        assert_eq!(wrapped, "sudo ");
    }
}
