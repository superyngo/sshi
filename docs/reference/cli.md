# CLI reference

Exhaustive reference for the **sshi** command-line interface. For quickstart examples, see `README.md`. Canonical terms used here are defined in [glossary.md](glossary.md).

---

## Invocation & TUI fallback

Running `sshi` without a subcommand launches the full-screen terminal user interface (TUI) when the default `tui` feature is enabled.

```bash
sshi [GLOBAL_OPTIONS] [SUBCOMMAND]
```

### Pre-launch environment checks

When invoked without a subcommand (`sshi`), the entry point performs environment checks before launching:

1. **TTY verification**: Both `stdin` and `stdout` must be interactive terminals (`is_terminal()`).
2. **Terminal capabilities**: On Unix, the `$TERM` environment variable must be present and not set to `""` or `"dumb"`.

If either check fails (e.g. invoked in a non-interactive pipeline, cron job, or unsupported terminal):
- A diagnostic message is printed to `stderr` (if `$TERM` is unsuitable).
- The standard CLI `--help` text is printed to `stdout`.
- The process terminates immediately with exit code `2` (following standard non-TTY CLI conventions).

If the binary was compiled without the `tui` feature (`--no-default-features`), running `sshi` without a subcommand prints an error to `stderr` and exits with code `1`.

---

## Global options

These options apply globally before subcommand dispatch:

| Option | Description |
|---|---|
| `-c, --config <PATH>` | Explicit path to the configuration file (default: `config.toml` in the platform config directory — see [config-schema.md](config-schema.md)). An explicit path that does not exist is an error (exit 1) for every command except `init`, which creates it, and the TUI (no subcommand), which starts empty and can save there. A missing default path means an empty config. |
| `-v, --verbose` | Print debug diagnostics to stderr: per host the resolved `user@host:port` and ProxyJump, the authentication method that succeeded (agent, key file, password), and each remote command with its exit status and duration (filter `sshi=debug,russh=info,info`). `RUST_LOG` overrides the filter; russh's own `log` records are bridged into it (e.g. `RUST_LOG=russh=debug`). Global: accepted before or after the subcommand (e.g. `sshi -v check --all` or `sshi check --all -v`). |
| `-h, --help` | Print top-level help and exit with code `0`. |
| `-V, --version` | Print version information and exit with code `0`. |

---

## Target selection (`TargetArgs`)

Commands that perform operations on remote hosts (`check`, `checkout`, `sync`, `cp`, `run`, `exec`, `list`) require targeting arguments.

### Target mode selectors

Target-operating commands require **exactly one** target mode selector. Specifying multiple mode selectors or omitting them results in a validation error (exiting with code `1` via `commands::resolve_target_mode`):

| Flag | Mode | Description |
|---|---|---|
| `-a, --all` | `TargetMode::All` | Target all configured **HostEntry** records in `config.toml`. |
| `-g, --group <GROUPS>` | `TargetMode::Groups` | Target hosts belonging to any of the specified group names (comma-separated list, e.g. `-g web,db`). |
| `-h, --host <HOSTS>` | `TargetMode::Hosts` | Target specific hosts by their configured name (comma-separated list, e.g. `-h srv1,srv2`). |
| `-s, --shell <SHELLS>` | `TargetMode::Shell` | Target hosts matching the detected **ShellType** (`sh`, `powershell`, `cmd`, comma-separated list, e.g. `-s sh,powershell`). |

### Execution & filtering modifiers

These optional flags modify how targeted hosts are filtered and how connections execute:

| Flag | Description |
|---|---|
| `--skip <HOSTS>` | Comma-separated list of host names to exclude from the resolved target list. Unknown host names in `--skip` are ignored. |
| `--serial` | Execute operations sequentially one host at a time (sets global concurrency and per-host concurrency to `1`). Overrides `settings.max_concurrency` and `settings.max_per_host_concurrency`. |
| `--timeout <SECS>` | Override per-host connection and execution timeout in seconds. Overrides `settings.default_timeout`. |
| `-H, --help` | Print subcommand-specific help. Subcommands use `-H` because `-h` is reserved for `--host`. |

---

## Structured report output (`OutputArgs`)

Commands that produce operational or inspection data (`check`, `checkout`, `sync`, `cp`, `run`, `exec`, `list`, `log`) accept the `-o, --out` flag to write structured **OperationReport** output to disk.

```bash
sshi <command> [TARGETS] --out [PATH]
```

### Path and format resolution

- **`--out` (no path argument)**: Automatically generates a timestamped report file in the current working directory:
  `sshi-<command>-<YYYYMMDD-HHmmss>.<ext>`
  where `<ext>` is taken from `settings.default_output_format` (default: `json`).
- **`--out <path>.json`**: Writes a formatted JSON file containing metadata, target filter info, summary statistics, and per-host results.
- **`--out <path>.html`**: Writes a self-contained HTML report with CSS styling and interactive layout.
- **Tilde expansion**: Paths beginning with `~` are expanded to the user's home directory.
- **Unsupported extensions**: Any file extension other than `.json` or `.html` returns an immediate error.

---

## Flag matrix

The following table summarizes all flags across every subcommand. The global `-v, --verbose` and `-c, --config` options are accepted both before and after subcommands.

| Flag | `init` | `check` | `checkout` | `sync` | `cp` | `run` | `exec` | `config` | `list` | `log` |
|---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| `-c, --config` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| `-a, --all` | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | — |
| `-g, --group` | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | — |
| `-h, --host` | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | ✓*1 |
| `-s, --shell` | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | — |
| `--skip` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | — |
| `--serial` | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | — |
| `--timeout` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | — |
| `--dry-run` | ✓ | ✓ | — | ✓ | ✓ | ✓ | ✓ | — | — | — |
| `-o, --out` | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — | ✓ | ✓ |
| `-n, --name` | — | ✓ | — | ✓ | — | — | — | — | — | — |
| `-S, --sudo` | — | — | — | — | — | ✓ | ✓ | — | — | — |
| `-S, --source` | — | — | — | ✓ | — | — | — | — | — | — |
| `--keep` | — | — | — | — | — | — | ✓ | — | — | — |
| `--combined-view` | — | — | ✓ | — | — | — | — | — | — | — |
| `--since` | — | — | — | — | — | — | — | — | — | ✓ |
| `--last` | — | — | — | — | — | — | — | — | — | ✓ |
| `--action` | — | — | — | — | — | — | — | — | — | ✓ |
| `--errors` | — | — | — | — | — | — | — | — | — | ✓ |
| `-H, --help` | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |

*\*1 Note: In `log`, `-h, --host` is a log-filtering option rather than a target selector.*  
---

## Subcommand reference

### `init`

Scans `~/.ssh/config`, probes connectivity to discovered hosts, handles missing host keys and authentication setup, detects remote **ShellType**, and generates or updates `config.toml`.

```bash
sshi init [OPTIONS]
```

#### Options
- `--dry-run`: Preview imported and stale hosts without writing changes to `config.toml` or executing keyscan/key-copy retries.
- `--skip <HOSTS>`: Skip specific hosts from connectivity testing and shell detection (comma-separated).
- `--timeout <SECS>`: Connection timeout in seconds during host discovery.
- `-H, --help`: Print help.

#### Interactive workflow
1. **Stale host cleanup**: Prompts to remove hosts from `config.toml` that no longer exist in `~/.ssh/config`.
2. **Host key acceptance**: If unknown SSH host keys are encountered, prompts to run `ssh-keyscan` and append keys to `~/.ssh/known_hosts`.
3. **Key generation & deployment**: If key authentication fails and no default private key (`~/.ssh/id_ed25519`, `~/.ssh/id_rsa`, etc.) exists, offers to generate one via `ssh-keygen -t ed25519`. Then offers to copy the public key to remote hosts via `ssh-copy-id`.

---

### `check`

Probes remote hosts for system health metrics (**Snapshot** records) and executes custom check paths. Records results in the `check_snapshots`, `host_last_seen`, and `operation_log` SQLite tables.

```bash
sshi check <TARGETS> [OPTIONS]
```

#### Target arguments
Accepts `-a/--all`, `-g/--group`, `-h/--host`, `-s/--shell`, `--skip`, `--serial`, `--timeout`.

#### Options
- `-n, --name <NAMES>`: Comma-separated list of `[[check]]` entry names to apply. If omitted, applies the entry named `"default"` (if defined in `config.toml`). A name that matches no entry is an error (exit 1) listing the available names.
- `--dry-run`: Display which hosts and check metrics would execute without connecting or modifying the database.
- `-o, --out [PATH]`: Write structured **OperationReport** to `.json` or `.html`.
- `-H, --help`: Print help.

#### Retention cleanup
After collection completes, `check` automatically triggers retention pruning (`retention::cleanup`) to remove snapshot records older than `settings.data_retention_days`.

---

### `checkout`

Inspects historical metrics stored in SQLite and renders tabular reports or export documents.

```bash
sshi checkout <TARGETS> [OPTIONS]
```

#### Target arguments
Accepts `-a/--all`, `-g/--group`, `-h/--host`, `-s/--shell`, `--skip`, `--serial`, `--timeout`.

#### Options
- `--combined-view`: Per-metric combined view: displays the most recent recorded value for each metric column across the 50 most recent snapshots of each host rather than only the single latest snapshot.
- `-o, --out [PATH]`: Write structured report to `.json` or `.html`.
- `-H, --help`: Print help.

---

### `sync`

Synchronizes files across remote hosts using the 3-stage collect-decide-distribute (**Local Relay**) model.

```bash
sshi sync <TARGETS> [PATHS...] [OPTIONS]
```

#### Target arguments
Accepts `-a/--all`, `-g/--group`, `-h/--host`, `-s/--shell`, `--skip`, `--serial`, `--timeout`.

#### Arguments & Options
- `PATHS...`: Positional file or directory paths to synchronize across hosts.
- `-n, --name <NAMES>`: Comma-separated list of `[[sync]]` entry names from `config.toml` to apply. A name that matches no entry is an error (exit 1) listing the available names.
- `-S, --source <HOST>`: Force a specific host as the authoritative file source, bypassing automatic newest-mtime/hash decision logic.
- `--dry-run`: Preview file comparisons, conflict decisions, and planned transfers without transferring files.
- `-o, --out [PATH]`: Write structured **OperationReport** to `.json` or `.html`.
- `-H, --help`: Print help.

*Note: Positional `PATHS` and `-n/--name` can be combined freely. At least one path or named entry must be provided.*

---

### `cp`

Copies local files, directories, or wildcard patterns to remote hosts using SFTP streaming (scp-style fan-out).

```bash
sshi cp <TARGETS> <LOCAL> [REMOTE] [OPTIONS]
```

#### Target arguments
Accepts `-a/--all`, `-g/--group`, `-h/--host`, `-s/--shell`, `--skip`, `--serial`, `--timeout`.

#### Arguments & Options
- `LOCAL` (required): Local path to copy. Supports single files, directories (copied recursively), or single-level wildcard patterns (e.g. `'configs/*.toml'`). Wildcards should be quoted to allow `sshi` to expand them.
- `REMOTE` (optional): Destination path on remote hosts. Defaults to `"~"` (the remote user's home directory). Leading `~` is expanded per host.
- `--dry-run`: Preview planned file transfers without uploading.
- `-o, --out [PATH]`: Write structured **OperationReport** to `.json` or `.html`.
- `-H, --help`: Print help.

#### Directory recursion and symlinks
When copying directories recursively:
- Symbolic links are not followed (to avoid filesystem recursion loops) and are reported as warnings.
- Unreadable directory entries or files produce an immediate operational error (exit code `1`) instead of being silently skipped.

---

### `run`

Executes a command string on remote hosts.

```bash
sshi run <TARGETS> <COMMAND> [OPTIONS]
```

#### Target arguments
Accepts `-a/--all`, `-g/--group`, `-h/--host`, `-s/--shell`, `--skip`, `--serial`, `--timeout`.

#### Arguments & Options
- `COMMAND` (required): Shell command string to execute on remote hosts.
- `-S, --sudo`: Execute the command with `sudo` (sh hosts). PowerShell and cmd hosts are refused with a per-host error (they count as failed hosts): Windows elevation (`Start-Process -Verb RunAs`, `runas`) cannot report the command's exit status or output. `--dry-run` previews the wrapped command per host.
- `--dry-run`: Preview the wrapped command string and targeted hosts without executing.
- `-o, --out [PATH]`: Write structured **OperationReport** to `.json` or `.html`.
- `-H, --help`: Print help.

---

### `exec`

Uploads a local script file to remote hosts and executes it via the host's native shell.

```bash
sshi exec <TARGETS> <SCRIPT> [OPTIONS]
```

#### Target arguments
Accepts `-a/--all`, `-g/--group`, `-h/--host`, `-s/--shell`, `--skip`, `--serial`, `--timeout`.

#### Arguments & Options
- `SCRIPT` (required): Local path to the script file.
- `-S, --sudo`: Execute the script with `sudo` (sh hosts only; PowerShell and cmd hosts are refused with a per-host error before anything is uploaded, as for `run --sudo`).
- `--keep`: Retain the temporary script file on the remote host after execution instead of deleting it.
- `--dry-run`: Preview execution and shell compatibility without uploading or running.
- `-o, --out [PATH]`: Write structured **OperationReport** to `.json` or `.html`.
- `-H, --help`: Print help.

#### Shell compatibility matching
The script file extension dictates the required remote **ShellType**:
- `.sh` → requires `Sh`
- `.ps1` → requires `PowerShell`
- `.bat`, `.cmd` → requires `Cmd`

Targeted hosts with mismatched shell environments are marked as `Skipped` and will not execute the script.

---

### `config`

Resolves the active `config.toml` path and opens it in the user's preferred text editor.

```bash
sshi config [OPTIONS]
```

#### Options
- `-c, --config <PATH>`: Custom configuration path to edit.
- `-H, --help`: Print help.

#### Editor resolution
Resolves the editor from `$VISUAL`, then `$EDITOR` (empty values skipped), then `vi` on Unix / `notepad` on Windows (`commands::config::resolve_editor`, shared with the TUI `E` key). On Unix a value with arguments such as `code --wait` runs through `sh -c`, as git does (`editor_command`).

---

### `list`

Lists configured hosts, shell types, assigned groups, and configured check and sync rules.

```bash
sshi list <TARGETS> [OPTIONS]
```

#### Target arguments
Accepts `-a/--all`, `-g/--group`, `-h/--host`, `-s/--shell`, `--skip`, `--serial`, `--timeout`.

#### Options
- `-o, --out [PATH]`: Write structured host and task list report to `.json` or `.html`.
- `-H, --help`: Print help.

---

### `log`

Queries and filters historical execution records from the SQLite `operation_log` table.

```bash
sshi log [OPTIONS]
```

#### Options
- `--last <N>`: Maximum number of log entries to display (default: `20`; pass `0` for all records).
- `--since <TIME>`: Filter entries since a given timestamp or relative duration (`YYYY-MM-DD`, `7d`, `24h`).
- `-h, --host <HOST>`: Filter entries for a specific host name.
- `--action <ACTION>`: Filter by command/action type (`sync`, `run`, `exec`, `check`, `cp`).
- `--errors`: Filter for entries with status `error`.
- `-o, --out [PATH]`: Write query results to `.json` or `.html`.
- `-H, --help`: Print help.

---

## Exit codes & error handling

The CLI adheres to the following exit code contract:

| Exit code | Condition | Description |
|---|---|---|
| `0` | Success | Normal successful completion, clean TUI exit, dry-run completion, or `--help`/`--version` display. |
| `1` | Operational failure | Fatal error during command setup, semantic target validation failure (e.g. missing target selector, mutually exclusive target flags), runtime execution failure (e.g. configuration file unreadable, local script missing), or TUI feature disabled. |
| `2` | Usage / Non-TTY error | Command-line syntax error (unrecognized flags, missing required positional arguments), or bare `sshi` invoked in a non-TTY or unsuitable terminal environment (`$TERM` empty or `"dumb"`). |
| `3` | Some hosts failed | A multi-host command (`check`, `run`, `exec`, `cp`, `sync`) finished, at least one host failed and at least one succeeded. See [Partial host failures](#partial-host-failures). |
| `4` | All hosts failed | A multi-host command finished, at least one host failed and none succeeded. |

### Partial host failures

In multi-host operations (`check`, `sync`, `cp`, `run`, `exec`), an individual host's connectivity failure, command error or probe timeout does **not** abort the run; every other host still completes.

Instead:
1. Per-host progress and errors stream to `stdout` / `stderr`.
2. A post-execution summary is printed (e.g. `2 succeeded, 1 failed, 0 skipped`).
3. Individual failures are recorded in the SQLite database and included in `-o/--out` reports.
4. The exit code reflects the per-host results ([ADR 0004](../adr/0004-host-failure-exit-codes.md), `CommandReport::host_outcome`):
   - `0`: no host failed. Skipped hosts do not count.
   - `3`: some hosts failed and at least one succeeded.
   - `4`: every host that ran failed.

   A host fails when its status is `offline` (including a remote command that exited non-zero), `unreachable`, `timedout` or `error`; `online` and `partial` count as success. `--dry-run` for `check`, `run`, `exec` and `cp` contacts no host and exits `0`; `sync --dry-run` connects to compare files, so its host failures count. `init`, `checkout`, `list`, `log` and `config` never use `3` or `4`.

---

## Color and terminal formatting

CLI progress lines (e.g. `[host        ]  ✓ detail`) format status indicators with ANSI color codes when written to an interactive terminal (`std::io::IsTerminal`).

Colors are suppressed (plain text output) when:
- Standard output (`stdout`) is not an interactive terminal (e.g. piped to another program or redirected to a file).
- The `NO_COLOR` environment variable is set to any non-empty value (per [no-color.org](https://no-color.org)).
