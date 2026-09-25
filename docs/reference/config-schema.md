# Config schema reference

This document describes the complete TOML configuration schema for `sshi` (`config.toml`), the comment-preservation behavior on save, and the rules used to parse and resolve `~/.ssh/config`.

---

## Configuration file locations

The configuration file is resolved in the following order:

1. Custom path passed via CLI `--config <path>` (supports `~` tilde expansion).
2. `$XDG_CONFIG_HOME/sshi/config.toml`, if `XDG_CONFIG_HOME` is set to an absolute path.
3. Default platform config path (`util::resolve_app_dir`):
   - **Linux**: `~/.config/sshi/config.toml`
   - **macOS**: `~/Library/Application Support/sshi/config.toml`
   - **Windows**: `%APPDATA%\sshi\config.toml`

**Legacy migration** (`util::app_dir`, `util::migrate_dir`): if the pre-B44 directory `~/.config/sshi` exists and the resolved directory has no `.migrated-config` marker, its files are copied in (existing files are never overwritten, the legacy directory is never modified), the marker is written, and one `sshi: migrated … → …` line goes to stderr. If the copy fails, sshi keeps using the legacy directory. No-op on Windows and whenever the legacy and resolved directories coincide (Linux default).

If the file does not exist, `sshi` operates with default settings or prompts for initialization (`sshi init`).

---

## `[settings]` table

The `[settings]` table contains global application settings. All fields are optional and fall back to their built-in defaults.

| Field | Type | Default | Description |
|---|---|---|---|
| `default_timeout` | `integer` (`u64`) | `30` | Default timeout in seconds for SSH commands and operations. |
| `data_retention_days` | `integer` (`u64`) | `90` | Number of days to retain historical snapshot and operation log data in SQLite. |
| `conflict_strategy` | `string` (`"newest"` \| `"skip"`) | `"newest"` | Default conflict resolution strategy during file sync when file timestamps differ. |
| `propagate_deletes` | `boolean` (`bool`) | `false` | When `true`, deleting a file on the source will propagate the deletion to target hosts during sync. |
| `max_concurrency` | `integer` (`usize`) | `10` | Global maximum number of concurrent operations across all hosts. |
| `max_per_host_concurrency` | `integer` (`usize`) | `4` | Maximum number of concurrent operations permitted against a single host. |
| `skipped_hosts` | `array of strings` (`Vec<String>`) | `[]` | List of host aliases to skip during `sshi init` host-key scanning and shell probing. Persisted across re-initialization runs. |
| `state_dir` | `string` (`Option<PathBuf>`) | `None` | Custom path override for the state directory where SQLite database (`sshi.db`) is stored. Defaults to the platform state directory (see [state-schema.md](state-schema.md)). |
| `default_output_format` | `string` (`Option<String>`) | `None` | Default format when `--out` is specified without an explicit file extension. Precedence: path extension > this setting > `"json"`. |

### Example

```toml
[settings]
default_timeout = 30
data_retention_days = 90
conflict_strategy = "newest"
propagate_deletes = false
max_concurrency = 10
max_per_host_concurrency = 4
skipped_hosts = ["backup-server"]
state_dir = "/var/lib/sshi"
default_output_format = "json"
```

---

## `[[host]]` table array

Each `[[host]]` entry represents a managed remote host.

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | `string` | *(required)* | Unique display name and selection identifier for the host (e.g. `web-1`). |
| `ssh_host` | `string` | *(required)* | SSH hostname, IP address, or `~/.ssh/config` host alias used to connect. |
| `shell` | `string` (`"sh"` \| `"powershell"` \| `"cmd"`) | *(required)* | Remote execution environment and shell type. Dictates metric probe command syntax, directory path formatting, and escaping. |
| `groups` | `array of strings` | `[]` | List of group names the host belongs to (used for `--group` / `-g` filtering). |
| `proxy_jump` | `string` (`Option<String>`) | `None` | Optional first-hop ProxyJump alias. `None` indicates a direct connection. |

### Example

```toml
[[host]]
name = "web-prod-1"
ssh_host = "192.168.1.10"
shell = "sh"
groups = ["web", "production"]
proxy_jump = "bastion"

[[host]]
name = "win-build"
ssh_host = "win-builder.corp.internal"
shell = "powershell"
groups = ["ci", "windows"]
```

---

## `[[check]]` table array

Each `[[check]]` entry configures a periodic metric inspection or health check task.

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | `string` (`Option<String>`) | `None` | Selection identifier (`sshi check -n <name>`) and TUI sidebar label. When `-n` is omitted on the CLI, `sshi check` selects the entry explicitly named `"default"`. Entries with an omitted or empty `name` cannot be selected by CLI commands and trigger a warning on load. |
| `id` | `string` | `""` | Stable 8-hex-character identifier used for TUI persistence state. Automatically generated via BLAKE3 on creation; falls back to vector index if empty. |
| `enabled` | `array of strings` | `[]` | List of system metric probes enabled for this check task. |
| `path` | `array of tables` (`[[check.path]]`) | `[]` | Custom path monitoring rules for tracking directory and file status. |

### Built-in probe names (`enabled`)

- `"online"` — Host reachability check
- `"system_info"` — Operating system and kernel details (`uname` / `systeminfo`)
- `"cpu_arch"` — CPU architecture
- `"memory"` — Total, used, and free RAM
- `"swap"` — Swap memory statistics
- `"disk"` — Disk partition usage
- `"cpu_load"` — CPU load averages
- `"network"` — Network interface statistics and status
- `"battery"` — Battery charge level and status (where available)
- `"ip_address"` — Network interface IP addresses

### `[[check.path]]` sub-table

| Field | Type | Description |
|---|---|---|
| `path` | `string` | Remote file or directory path to inspect. |
| `label` | `string` | Display label for reports and TUI metrics view. |

### Example

```toml
[[check]]
name = "default"
id = "a1b2c3d4"
enabled = [
    "online",
    "system_info",
    "cpu_arch",
    "memory",
    "swap",
    "disk",
    "cpu_load",
    "network",
    "battery",
    "ip_address",
]

[[check]]
name = "web-logs"
id = "e5f6a7b8"
enabled = ["online", "disk"]

[[check.path]]
path = "/var/log/nginx"
label = "Nginx Logs"
```

---

## `[[sync]]` table array

Each `[[sync]]` entry defines a file synchronization task.

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | `string` (`Option<String>`) | `None` | Selection identifier (`sshi sync -n <name>`) and TUI sidebar label. |
| `id` | `string` | `""` | Stable 8-hex-character identifier used for TUI persistence state. |
| `paths` | `array of strings` | *(required)* | File or directory paths to synchronize across hosts. |
| `recursive` | `boolean` (`bool`) | `false` | Whether to synchronize directories recursively. |
| `mode` | `string` (`Option<String>`) | `None` | Optional file permissions mode string (e.g. `"0644"`). |
| `propagate_deletes` | `boolean` (`Option<bool>`) | `None` | Override for the global `propagate_deletes` setting for this specific sync task. |
| `source` | `string` (`Option<String>`) | `None` | Fixed source host alias. If specified, bypasses automatic source selection (e.g. newest mtime). |

### Example

```toml
[[sync]]
name = "nginx-config"
id = "10203040"
paths = ["/etc/nginx/nginx.conf", "/etc/nginx/conf.d"]
recursive = true
mode = "0644"
propagate_deletes = true
source = "web-prod-1"
```

---

## Comment preservation and file save behavior

Configuration file I/O is managed by `src/config/app.rs` using `toml_edit::DocumentMut` for structured editing:

1. **UTF-8 BOM Stripping**: On load, leading UTF-8 Byte Order Marks (`\u{feff}`) are stripped automatically.
2. **First-time File Creation**: When no existing `config.toml` exists, the configuration is serialized and default explanatory comment templates (`inject_config_comments`) are added for `[settings]`, `[[check]]`, and `[[sync]]`.
3. **Structured In-Place Updates**:
   - `[settings]` scalars are mutated in place via `set_scalar`, preserving whitespace, formatting, and per-key inline comments (e.g. `max_concurrency = 10  # max 50`).
   - Unknown top-level keys inside `[settings]` or the document root are retained across saves.
   - Array-of-tables (`[[host]]`, `[[check]]`, `[[sync]]`) are fully reconstructed from in-memory structs on write: top-level section comments survive, but individual per-entry inline comments inside table entries are replaced.
4. **Validation**: Before writing to disk, the generated TOML is round-trip validated with `toml::from_str::<AppConfig>` to ensure syntactic validity.
5. **Atomic Write**: Saves write to a temporary file (`.sshi-config-*.tmp`) in the configuration directory and atomically rename it (`tempfile::persist`) to prevent corrupted or partial writes.

---

## `~/.ssh/config` parsing and resolution

SSH host aliases and connection parameters are parsed directly from `~/.ssh/config` via `src/config/ssh_config.rs`.

### Parser implementation

- **Hand-rolled parser**: `sshi` uses a custom, self-contained parser (`parse_ssh_config_content`) implemented in `src/config/ssh_config.rs`.
- **No external parser crate**: The `ssh2-config` crate was removed to avoid a transitive C library dependency (`openssl-sys` via `git2`/`libgit2-sys`). No external SSH configuration parsing dependency is used.

### Supported directives

The parser matches directives case-insensitively and supports both space-delimited (`Key Value`) and equal-delimited (`Key=Value`) formats:

| Directive | Behavior | Default / Fallback |
|---|---|---|
| `Host` | Defines a host block. Supports multiple whitespace-separated aliases (e.g. `Host s1 s2`). Names containing `*` or `?` are treated as wildcard default blocks. | — |
| `HostName` | Target DNS hostname or IP address. | Host alias name |
| `User` | Remote SSH login username. | Current system username (`whoami::username`) |
| `Port` | Remote SSH port number (`u16`). | `22` |
| `IdentityFile` | Path to private authentication key. Supports `~` tilde expansion. | Direct connection / agent / password auth |
| `ProxyJump` | Jump host proxy alias. Comma-separated multi-hop chains are parsed to extract the first hop. | `None` (direct connection) |

### Parsing and inheritance rules

1. **Wildcard inheritance (`Host *`)**: Directives defined in wildcard blocks (e.g. `Host *`) are accumulated into `wildcard_defaults`. When resolving a host alias with `query(alias)`, any field not explicitly set in the host-specific block inherits from the wildcard defaults.
2. **Multi-alias expansion**: `Host alias1 alias2` creates distinct lookup entries for each alias sharing the block's parameters.
3. **Comments and unknown directives**: Lines starting with `#` and empty lines are ignored. Directives not recognized by `sshi` (such as `ServerAliveInterval`, `ForwardAgent`, `Match`, or `Include`) are ignored without error.
