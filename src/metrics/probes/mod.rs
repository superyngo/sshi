//! Shell-specific probe command registry (sh, PowerShell, cmd).

pub mod cmd;
pub mod powershell;
pub mod sh;

use crate::config::schema::ShellType;

/// Build a single batched command that collects all metrics with `---METRIC:` markers.
pub fn batch_command(shell: ShellType, metrics: &[String]) -> String {
    match shell {
        ShellType::Sh => sh::batch_command(metrics),
        ShellType::PowerShell => powershell::batch_command(metrics),
        ShellType::Cmd => cmd::batch_command(metrics),
    }
}

/// Build a single batched command that measures all path sizes with `---PATH:` markers.
pub fn batch_path_command(shell: ShellType, paths: &[(String, String)]) -> String {
    match shell {
        ShellType::Sh => sh::batch_path_command(paths),
        ShellType::PowerShell => powershell::batch_path_command(paths),
        ShellType::Cmd => cmd::batch_path_command(paths),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_command_sh_markers() {
        let metrics = vec!["online".into(), "memory".into()];
        let cmd = batch_command(ShellType::Sh, &metrics);
        assert!(cmd.contains("---METRIC:online"));
        assert!(cmd.contains("---METRIC:memory"));
    }

    #[test]
    fn test_batch_command_ps_markers() {
        let metrics = vec!["online".into()];
        let cmd = batch_command(ShellType::PowerShell, &metrics);
        assert!(cmd.contains("---METRIC:online"));
    }

    #[test]
    fn test_batch_command_cmd_markers() {
        let metrics = vec!["online".into(), "cpu_arch".into()];
        let cmd = batch_command(ShellType::Cmd, &metrics);
        assert!(cmd.contains("---METRIC:online"));
        assert!(cmd.contains("---METRIC:cpu_arch"));
    }

    #[test]
    fn test_batch_command_empty() {
        assert!(batch_command(ShellType::Sh, &[]).is_empty());
        assert!(batch_command(ShellType::PowerShell, &[]).is_empty());
        assert!(batch_command(ShellType::Cmd, &[]).is_empty());
    }

    #[test]
    fn test_batch_path_command_sh() {
        let paths = vec![
            ("~/docs".into(), "docs".into()),
            ("/var".into(), "var".into()),
        ];
        let cmd = batch_path_command(ShellType::Sh, &paths);
        assert!(cmd.contains("---PATH:docs"));
        assert!(cmd.contains("---PATH:var"));
    }

    #[test]
    fn test_batch_path_command_empty() {
        assert!(batch_path_command(ShellType::Sh, &[]).is_empty());
    }
}
