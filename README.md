# sshi

SSH-config-based cross-platform remote management tool.

## Recent changes

See [CHANGELOG.md](CHANGELOG.md) for recent changes and release notes.

## Features

- **Host Discovery**: Import hosts from `~/.ssh/config` with automatic shell type detection
- **System Snapshots**: Collect and store system information for historical tracking
- **File Synchronization**: Sync files across multiple hosts using collect-decide-distribute model
- **File Copy**: Push local files or directories to many hosts at once (`cp`, scp-style)
- **Remote Execution**: Run commands or scripts on multiple hosts in parallel
- **TUI Interface**: Interactive terminal UI (`sshi`) for browsing snapshot data, configuring filters, and running checks

## Installation

### With Wenget

```bash
wenget install sshi
```

### Windows

```powershell
$env:APP_NAME="sshi"; $env:REPO="superyngo/sshi"; irm https://gist.githubusercontent.com/superyngo/a6b786af38b8b4c2ce15a70ae5387bd7/raw/gpinstall.ps1 | iex
```

### macOS / Linux

```bash
cargo install sshi
```

Or build from source:

```bash
git clone https://github.com/superyngo/sshi.git
cd sshi
cargo install --path .
```

## Binaries

One binary is produced. Source builds include the TUI by default; headless builds can be compiled using `--no-default-features`. Release downloads include a TUI-enabled build.

| Binary | Built with | What it does |
|--------|-----------|--------------|
| `sshi` | always | All CLI subcommands. Invoked without a subcommand → launches TUI (if built with `tui` feature, default), otherwise prints "TUI not compiled in. Rebuild with --features tui." and exits 1. |

```bash
cargo build                                       # TUI build (default)
cargo build --no-default-features                 # headless
```

> Running multiple `sshi` instances against the same config simultaneously
> is not supported; they share a single state file with last-write-wins
> semantics.

## TUI keybindings

See [docs/reference/tui.md](docs/reference/tui.md) for complete TUI documentation and keybindings.

## Usage

### Initialize

Import hosts from `~/.ssh/config`:

```bash
sshi init
```

### Check

Collect system snapshots from hosts:

```bash
# All hosts — applies the [[check]] entry named "default"
sshi check --all

# Apply specific named [[check]] entries (comma-separated)
sshi check --all -n cpu,disk

# Specific group
sshi check -g servers

# Specific hosts
sshi check -h host1,host2

# Sequential execution
sshi check --all --serial
```

Target flags (`-a`/`-g`/`-h`/`-s`) select **which hosts** to act on; `-n/--name`
selects **which `[[check]]` entries** to apply. With no `-n`, the entry named
`"default"` is used (if present).

### Sync

Synchronize files across hosts:

```bash
# Sync paths directly (positional, space-separated)
sshi sync --all /etc/hosts /etc/resolv.conf

# Apply named [[sync]] entries from config
sshi sync --all -n dotfiles,nginx

# Combine named entries with extra ad-hoc paths
sshi sync --all -n dotfiles /etc/hosts

# Preview without changes
sshi sync --all -n dotfiles --dry-run

# Use fixed source host
sshi sync --all -n dotfiles -S host1
```

Positional paths and `-n/--name` combine. Passing neither is an error — there is
nothing to sync. (`[[sync]]`/`[[check]]` entries are selected by their `name`;
the former `groups` / `enable_hosts` / `enable_all` entry fields were removed.)

### Run

Execute commands on remote hosts:

```bash
# Run command on all hosts
sshi run --all "uptime"

# Run with sudo
sshi run --all "apt update" -S
```

### Exec

Upload and execute local scripts:

```bash
# Execute script
sshi exec --all ./deploy.sh

# Execute with sudo
sshi exec --all ./install.sh -S

# Keep remote script after execution
sshi exec --all ./script.sh --keep

# Preview without executing
sshi exec --all ./deploy.sh --dry-run
```

### Cp

Copy local files or directories to remote hosts, fanning out to every target (scp-style):

```bash
# Copy a file to the remote home directory (~)
sshi cp --all ./app.conf

# Copy to an explicit remote path
sshi cp --all ./app.conf /etc/app/app.conf

# Copy a directory recursively (file vs. directory is auto-detected)
sshi cp -g web ./assets ~/assets

# Wildcards — quote the pattern so sshi expands it (not your shell)
sshi cp --all './configs/*.toml' ~/configs/

# Preview without transferring
sshi cp --all ./app.conf --dry-run
```

- The **local path** (required, first positional) may be a file, a directory
  (copied recursively), or a quoted wildcard pattern expanded by sshi itself.
- The **remote path** (optional, second positional) defaults to the remote home
  directory, mirroring `scp`. A leading `~` is expanded per host/shell.
- Per-file transfers stream via SFTP (no size cap; the previous 64 MB limit
  was lifted by the streaming-SFTP refactor).

### Checkout

View historical data and generate reports:

```bash
# Interactive TUI
sshi checkout --all

# HTML report
sshi checkout --all --out report.html
```

### List

List configured hosts, shell types, assigned groups, and checks:

```bash
# List all configured hosts
sshi list --all

# List hosts in a specific group
sshi list -g web

# Export host list to JSON report
sshi list --all --out hosts.json
```

### Log

View operation logs:

```bash
# Show last 20 entries
sshi log

# Show last 50 entries
sshi log --last 50

# Filter by host
sshi log --host server1

# Filter by action type
sshi log --action sync

# Show only errors
sshi log --errors

# Export logs to HTML report
sshi log --out report.html
```

### Config

Open configuration file in `$VISUAL` / `$EDITOR`:

```bash
sshi config
```

## Target Selection

All commands that operate on remote hosts support the following target options:

| Flag | Description |
|------|-------------|
| `-a, --all` | Target all configured hosts |
| `-g, --group` | Target hosts by group (comma-separated) |
| `-h, --host` | Target specific hosts (comma-separated) |
| `-s, --shell` | Target hosts by detected shell type (`sh`, `powershell`, `cmd`) |
| `--serial` | Execute sequentially instead of in parallel |
| `--timeout` | Connection timeout in seconds |

## Configuration

`config.toml` lives in `$XDG_CONFIG_HOME/sshi/` if set, otherwise
`~/.config/sshi/` (Linux), `~/Library/Application Support/sshi/` (macOS) or
`%APPDATA%\sshi\` (Windows). State (the snapshot database `sshi.db`) lives in
`$XDG_STATE_HOME/sshi/` if set, otherwise `~/.local/state/sshi/` (Linux),
`~/Library/Application Support/sshi/` (macOS) or `%LOCALAPPDATA%\sshi\`
(Windows). On first run sshi copies files from the older `~/.config/sshi/` and
`~/.local/state/sshi/` locations into the new ones and leaves the originals in
place.

> **Migrating from `ssync`:** this project was previously named `ssync` and used
> `~/.config/ssync/` and `~/.local/state/ssync/`. The new paths are not read
> automatically — move your existing files:
>
> ```sh
> mv ~/.config/ssync ~/.config/sshi
> mv ~/.local/state/ssync ~/.local/state/sshi
> ```

Example configuration:

```toml
[settings]
default_timeout = 30
max_concurrency = 10
state_dir = "~/.local/state/sshi"
# default_output_format = "html"   # json (default) or html

[[host]]
name = "server1"
ssh_host = "server1"
shell = "sh"
groups = ["production", "web"]

[[host]]
name = "server2"
ssh_host = "server2"
shell = "sh"
groups = ["production", "db"]

[[check]]
name = "default"
enabled = ["online", "cpu_load", "memory", "disk"]

[[check.path]]
path = "/var/log/nginx"
label = "Nginx Logs"
```

## Documentation

See [CONTEXT.md](CONTEXT.md) for architectural overview, design documents, and reading order across reference specifications.

## License

MIT
