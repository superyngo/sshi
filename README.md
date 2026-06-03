# sshi

SSH-config-based cross-platform remote management tool.

## Features

- **Host Discovery**: Import hosts from `~/.ssh/config` with automatic shell type detection
- **System Snapshots**: Collect and store system information for historical tracking
- **File Synchronization**: Sync files across multiple hosts using collect-decide-distribute model
- **Remote Execution**: Run commands or scripts on multiple hosts in parallel
- **TUI Interface**: Interactive terminal UI (`sshi`) for browsing snapshot data, configuring filters, and running checks

## Installation

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

One binary is produced. Source builds default to headless; release downloads include a TUI-enabled build.

| Binary | Built with | What it does |
|--------|-----------|--------------|
| `sshi` | always | All CLI subcommands. Invoked without a subcommand → launches TUI (if built with `--features tui`), otherwise prints "Interactive TUI not available" and exits 1. |

```bash
cargo build --bin sshi                            # headless
cargo build --bin sshi --features tui             # TUI build
```

> Running multiple `sshi` instances against the same config simultaneously
> is not supported; they share a single state file with last-write-wins
> semantics.

## TUI keybindings (Phase 7)

| Scope | Key | Action |
|-------|-----|--------|
| Global | `1` / `2` / `3` | Switch to Config / Operate / Checkout |
| Global | `q` | Quit (state saved) |
| Global | `Ctrl+C` | Quit immediately (state saved; cancels running op) |
| Global | `Esc` | Close popup / clear error / cancel running op |
| Global | `?` | Toggle keybindings help popup |
| Global | `i` | Toggle contextual info popup |
| Global | `L` | Toggle log overlay |
| Config | `↑` `↓` `j` `k` | Move within sidebar or field table |
| Config | `←` / `→` | Switch zones (Sidebar ↔ FieldTable); also cycles radio/toggle fields |
| Config | `Tab` | Sidebar → FieldTable (within Config tab only) |
| Config | `PgUp` `PgDn` `Home` `End` | Page / jump navigation |
| Config | `e` / `Enter` | Edit selected field inline; cycle radio fields; open group picker for `groups` |
| Config | `E` | Open config file in `$VISUAL` / `$EDITOR` / `vi` — prompts if unsaved changes; reloads on change |
| Config | `S` | Save config (preserves comments and unknown keys via `toml_edit`) |
| Config | `a` | Add new entry (host / check / sync based on sidebar selection) |
| Config | `d` | Delete selected entry (with confirmation) |
| Config (group picker) | `Space` | Toggle group selection |
| Config (group picker) | `Enter` / `s` | Apply group selection |
| Config (group picker) | `Esc` | Cancel group picker |
| Operate | `↑` / `↓` (or `j`/`k`) | Move between zones |
| Operate | `←` / `→` | Cycle operation radio (check / run / exec / sync) |
| Operate | `f` | Open Target Filter popup |
| Operate | `Enter` on `[Execute]` | Run the selected operation |
| Operate | `Tab` / `Shift+Tab` | Cycle tabs (Operate and Checkout only) |
| Checkout | `↑` `↓` `j` `k` | Move row selection |
| Checkout | `PgUp` `PgDn` `Home` `End` | Page / jump navigation |
| Checkout | `Tab` / `Shift+Tab` | Cycle tabs |
| View (all) | `o` | Export the currently viewed data to a report file (.json or .html) |

> **Note:** On the Config tab, `Tab` switches between the Sidebar and FieldTable zones rather than cycling to the next tab. Use `1` / `2` / `3` to switch tabs from Config.
>
> **toml_edit comment preservation:** `S` saves config using `toml_edit`, which preserves all comments and unknown keys. The one known limitation: when an entry (host/check/sync) is deleted, any inline comments attached to that entry's keys are lost. All other comments survive edits.

## Usage

### Initialize

Import hosts from `~/.ssh/config`:

```bash
sshi init
```

Re-detect shell types for existing hosts:

```bash
sshi init --update
```

### Check

Collect system snapshots from hosts:

```bash
# All hosts
sshi check --all

# Specific group
sshi check -g servers

# Specific hosts
sshi check -h host1,host2

# Sequential execution
sshi check --all --serial
```

### Sync

Synchronize files across hosts:

```bash
# Sync configured files
sshi sync --all

# Preview without changes
sshi sync --all --dry-run

# Sync specific files
sshi sync --all -f /etc/hosts,/etc/resolv.conf

# Use fixed source host
sshi sync --all -S host1

# Don't push to hosts missing files
sshi sync --all --no-push-missing
```

### Run

Execute commands on remote hosts:

```bash
# Run command on all hosts
sshi run --all "uptime"

# Run with sudo
sshi run --all "apt update" -S

# Auto-confirm prompts (serial mode)
sshi run --all "systemctl restart nginx" --yes
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

### Checkout

View historical data and generate reports:

```bash
# Interactive TUI
sshi checkout --all

# HTML report
sshi checkout --all --out report.html

# Show trend history
sshi checkout --all --history

# History from specific date
sshi checkout --all --history --since "2025-01-01"
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

Open configuration file in `$EDITOR`:

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

The default config location is `~/.config/sshi/config.toml`, and state (the
snapshot database `sshi.db`) lives in `~/.local/state/sshi/`.

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
state_dir = "~/.local/share/sshi"
# default_output_format = "html"   # json (default) or html

[[host]]
name = "server1"
hostname = "192.168.1.10"
user = "admin"
port = 22
groups = ["production", "web"]

[[host]]
name = "server2"
hostname = "192.168.1.11"
user = "admin"
groups = ["production", "db"]

[[host.file]]
path = "/etc/hosts"
description = "Hosts file"

[[host.file]]
path = "/etc/resolv.conf"
description = "DNS configuration"
```

## License

MIT
