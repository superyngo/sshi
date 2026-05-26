use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "ssync",
    version,
    about = "SSH-config-based cross-platform remote management tool",
    subcommand_required = false,
    arg_required_else_help = false
)]
pub struct Cli {
    /// Enable verbose output
    #[arg(short = 'v', long)]
    pub verbose: bool,

    /// Path to config file (default: ~/.config/ssync/config.toml)
    #[arg(short = 'c', long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Common target selection arguments for commands that operate on remote hosts.
#[derive(Args, Clone, Debug)]
pub struct TargetArgs {
    /// Specify groups (comma-separated)
    #[arg(short, long, value_delimiter = ',')]
    pub group: Vec<String>,

    /// Specify hosts (comma-separated)
    #[arg(short, long, value_delimiter = ',')]
    pub host: Vec<String>,

    /// Target all hosts
    #[arg(short, long)]
    pub all: bool,

    /// Filter by remote shell type (comma-separated: sh, powershell, cmd)
    #[arg(short = 's', long, value_delimiter = ',')]
    pub shell: Vec<crate::config::schema::ShellType>,

    /// Skip specific hosts (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub skip: Vec<String>,

    /// Execute sequentially instead of in parallel
    #[arg(long)]
    pub serial: bool,

    /// Connection timeout in seconds (overrides config)
    #[arg(long)]
    pub timeout: Option<u64>,

    /// Print help
    #[arg(short = 'H', long, action = clap::ArgAction::HelpLong)]
    pub help: Option<bool>,
}

/// Output arguments for writing structured reports to file.
#[derive(Args, Clone, Debug, Default)]
pub struct OutputArgs {
    /// Write structured report to file (.json or .html).
    /// Omit path for auto-named file: ssync-{command}-{YYYYMMDD-HHmmss}.json
    /// Examples: --out  |  --out report.json  |  --out report.html
    #[arg(short = 'o', long, num_args = 0..=1, default_missing_value = "")]
    pub out: Option<String>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Import hosts from ~/.ssh/config and detect remote shell types
    #[command(disable_help_flag = true)]
    Init {
        /// Re-detect shell type for existing hosts
        #[arg(long)]
        update: bool,

        /// Show what would be imported without writing
        #[arg(long)]
        dry_run: bool,

        /// Skip specific hosts (comma-separated)
        #[arg(long, value_delimiter = ',')]
        skip: Vec<String>,

        /// Connection timeout in seconds (overrides config)
        #[arg(long)]
        timeout: Option<u64>,

        /// Print help
        #[arg(short = 'H', long, action = clap::ArgAction::HelpLong)]
        help: Option<bool>,
    },

    /// Collect system snapshots from hosts and store in state DB
    #[command(disable_help_flag = true)]
    Check {
        #[command(flatten)]
        target: TargetArgs,

        /// Preview which hosts/checks would run without collecting or writing
        #[arg(long)]
        dry_run: bool,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// View historical data and generate reports from state DB
    #[command(disable_help_flag = true)]
    Checkout {
        #[command(flatten)]
        target: TargetArgs,

        /// Show trend history
        #[arg(long)]
        history: bool,

        /// History start point (e.g. "2025-01-01" or "7d")
        #[arg(long)]
        since: Option<String>,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// Synchronize files across hosts using collect-decide-distribute model
    #[command(disable_help_flag = true)]
    Sync {
        #[command(flatten)]
        target: TargetArgs,

        /// Preview sync decisions without making changes
        #[arg(long)]
        dry_run: bool,

        /// Ad-hoc file paths to sync (comma-separated)
        #[arg(short = 'f', long, value_delimiter = ',')]
        files: Vec<String>,

        /// Use a specific host as file source (bypasses auto-detection)
        #[arg(short = 'S', long)]
        source: Option<String>,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// Execute a command string on remote hosts
    #[command(disable_help_flag = true)]
    Run {
        #[command(flatten)]
        target: TargetArgs,

        /// Command to execute
        command: String,

        /// Run with sudo
        #[arg(short = 'S', long)]
        sudo: bool,

        /// Preview without executing
        #[arg(long)]
        dry_run: bool,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// Upload and execute a local script on remote hosts
    #[command(disable_help_flag = true)]
    Exec {
        #[command(flatten)]
        target: TargetArgs,

        /// Local script path
        script: String,

        /// Run with sudo
        #[arg(short = 'S', long)]
        sudo: bool,

        /// Keep remote temp script after execution
        #[arg(long)]
        keep: bool,

        /// Preview without executing
        #[arg(long)]
        dry_run: bool,

        #[command(flatten)]
        output: OutputArgs,
    },

    /// Open config file in $EDITOR
    #[command(disable_help_flag = true)]
    Config {
        /// Print help
        #[arg(short = 'H', long, action = clap::ArgAction::HelpLong)]
        help: Option<bool>,
    },

    /// List hosts, applicable checks, and sync paths
    #[command(disable_help_flag = true)]
    List {
        #[command(flatten)]
        target: TargetArgs,
    },

    /// View operation logs
    #[command(disable_help_flag = true)]
    Log {
        /// Show last N entries (default: 20)
        #[arg(long, default_value = "20")]
        last: usize,

        /// Show entries since datetime
        #[arg(long)]
        since: Option<String>,

        /// Filter by host name
        #[arg(short, long)]
        host: Option<String>,

        /// Filter by action type
        #[arg(long)]
        action: Option<ActionFilter>,

        /// Show only error entries
        #[arg(long)]
        errors: bool,

        /// Print help
        #[arg(short = 'H', long, action = clap::ArgAction::HelpLong)]
        help: Option<bool>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn skip_parses_as_comma_list_on_check() {
        let cli = Cli::try_parse_from(["ssync", "check", "--all", "--skip", "h1,h2"]).unwrap();
        match cli.command.unwrap() {
            Commands::Check { target, .. } => {
                assert_eq!(target.skip, vec!["h1".to_string(), "h2".to_string()]);
            }
            _ => panic!("expected Check"),
        }
    }

    #[test]
    fn sync_rejects_no_push_missing() {
        let result = Cli::try_parse_from(["ssync", "sync", "--all", "--no-push-missing"]);
        assert!(result.is_err(), "--no-push-missing should be rejected");
    }

    #[test]
    fn sync_still_parses_without_removed_flag() {
        let cli = Cli::try_parse_from(["ssync", "sync", "--all"]).unwrap();
        assert!(matches!(cli.command.unwrap(), Commands::Sync { .. }));
    }

    #[test]
    fn run_rejects_yes() {
        assert!(Cli::try_parse_from(["ssync", "run", "--all", "--yes", "echo hi"]).is_err());
        assert!(Cli::try_parse_from(["ssync", "run", "--all", "-y", "echo hi"]).is_err());
    }

    #[test]
    fn exec_rejects_yes() {
        assert!(Cli::try_parse_from(["ssync", "exec", "--all", "--yes", "s.sh"]).is_err());
    }

    #[test]
    fn run_parses_dry_run() {
        let cli = Cli::try_parse_from(["ssync", "run", "--all", "--dry-run", "echo hi"]).unwrap();
        match cli.command.unwrap() {
            Commands::Run { dry_run, .. } => assert!(dry_run),
            _ => panic!("expected Run"),
        }
    }

    #[test]
    fn check_parses_dry_run() {
        let cli = Cli::try_parse_from(["ssync", "check", "--all", "--dry-run"]).unwrap();
        match cli.command.unwrap() {
            Commands::Check { dry_run, .. } => assert!(dry_run),
            _ => panic!("expected Check"),
        }
    }
}

#[derive(Clone, clap::ValueEnum)]
pub enum ActionFilter {
    Sync,
    Run,
    Exec,
    Check,
}
