# TUI reference

`sshi` includes an interactive terminal user interface built with [ratatui](https://ratatui.rs) and [crossterm](https://github.com/crossterm-rs/crossterm). The TUI is gated behind the Cargo feature `tui`, which is enabled by default in `Cargo.toml`. Standard builds (`cargo build`) include the TUI; headless CLI-only builds use `cargo build --no-default-features`. When launched without subcommands (`sshi`), the application enters TUI mode by default (`src/main.rs`, `src/tui/entry.rs`).

## Tab structure

The TUI is organized into three primary tabs defined in `src/tui/tabs/mod.rs` (`TabId`):

| Tab | Label | Key | Purpose |
|-----|-------|-----|---------|
| **Config** | `1:Config` | `1` | 3-level browser (sections, entries, fields) with inline editing, direct picker popups, entry creation/deletion, and external `$EDITOR` integration. |
| **Operate** | `2:Operate` | `2` | Interactive operation dispatcher for `check`, `run`, `exec`, `sync`, and `cp` with linear parameter walk, target filtering, and live progress monitoring. |
| **View** | `3:View` | `3` | Multi-view inspection surface displaying historical snapshot metrics (`Checkout`), configuration listings (`List`), and execution audit logs (`Log`). |

### Layout & Frame Allocation

The screen layout is divided vertically into three zones (`src/tui/app.rs`):
1. **Tab bar** (`Length(3)`): Displays the application title `sshi`, version number right-aligned, and the active tab selection widget (`1:Config`, `2:Operate`, `3:View`).
2. **Main content** (`Min(0)`): Houses the active tab widget and all nested viewports.
3. **Status bar** (`Length(2)`): Displays contextual keybinding hints and transient notifications or error messages (`app.error`, styled in bold red).

### Terminal Size Guard

The TUI enforces a hard terminal size threshold of **60 columns × 20 rows** (`MIN_COLS = 60`, `MIN_ROWS = 20` in `src/tui/app.rs`). If the terminal window is smaller than 60×20, standard rendering is suspended and a full-screen warning paragraph is displayed requesting a resize.

---

## Global shortcuts & navigation

Global keybindings are evaluated first and remain active across all tabs, unless an interactive text input or modal popup is currently capturing keyboard input:

| Key | Scope | Action |
|-----|-------|--------|
| `1` | Global | Switch directly to the **Config** tab (`TabId::Config`). |
| `2` | Global | Switch directly to the **Operate** tab (`TabId::Operate`). |
| `3` | Global | Switch directly to the **View** tab (`TabId::View`). |
| `q` | Global | Quit the application cleanly; automatically flushes pending config and state changes to disk. |
| `Ctrl+C` | Global | Immediate emergency abort; cancels any active background operation, flushes pending state, and exits. The same clean exit (terminal restored) runs on SIGHUP/SIGTERM/SIGINT on Unix, and on Ctrl+Break or closing the console window on Windows (`spawn_signal_listener`). |
| `Esc` | Global | Context-dependent escape action (see [Escape Level Cycling](#escape-level-cycling)). |
| `?` | Global | Toggle the modal **Keybindings & Help** popup. |
| `i` | Global | Toggle the contextual **Info** popup. |
| `L` | Global | Toggle the slide-over **Log buffer overlay** (`LogBufferHandle`). |

### Escape Level Cycling

`Esc` behavior is context-sensitive depending on active popups, errors, and the current tab (`EscLevel` in `src/tui/app_state.rs`):

1. **Error / Notification Active**: Dismisses `app.error` immediately without changing focus.
2. **Modal Popup / Overlay Open**: Closes the active popup (Help, Info, Log Overlay, Completed Report, Member Picker) or cancels the running operation.
3. **Config Tab**: Toggles focus between the top navigation bar (`NavBar`) and content zones.
4. **Operate & View Tabs**: Cycles focus through a 3-way hierarchy:
   $$\text{NavBar} \longrightarrow \text{TopField} \longrightarrow \text{Content} \longrightarrow \text{NavBar}$$
   - **Operate**: `NavBar` $\rightarrow$ `OpRadio` (Top Field) $\rightarrow$ first parameter field after OpRadio $\rightarrow$ `NavBar`.
   - **View**: `NavBar` $\rightarrow$ `OpSelector` (Top Field) $\rightarrow$ first settings field or Result area $\rightarrow$ `NavBar`.

### NavBar Focus

When the top navigation bar is focused (`navbar_focused == true`):
- `←` / `→` / `h` / `l` / `Tab` / `BackTab`: Cycle active tab selection.
- `↓` / `j` / `Enter`: Descend focus into the active tab content.
- `1` / `2` / `3`: Switch to tab and immediately descend focus.

---

## Popups & overlays

### Help Popup (`?`)
- **Activation**: Press `?` from any non-input context.
- **Sections**: Cycles between `Help` (keybinding tables) and `About` (version and build metadata) via `Tab` or `BackTab`.
- **Navigation**: `↑` / `↓` / `j` / `k` scroll line-by-line; `PgUp` / `PgDn` page scroll; `Home` / `End` jump to edges.
- **Dismissal**: `Esc` or `?`.

### Info Popup (`i`)
- **Activation**: Press `i` from any non-input context.
- **Sections**: 3-stage cycle (`TabInfo` $\rightarrow$ `About` $\rightarrow$ `Help` $\rightarrow$ `TabInfo`) via `Tab`, `BackTab`, or `i`. `TabInfo` displays documentation and tips specific to the active tab.
- **Navigation**: `↑` / `↓` / `j` / `k`, `PgUp` / `PgDn`, `Home` / `End`.
- **Dismissal**: `Esc`.

### Log Overlay (`L`)
- **Activation**: Press `L` (Shift+L) from any tab.
- **Content**: Displays live in-memory tracing logs captured by `src/tui/log_layer.rs` (`LogBufferHandle`).
- **Navigation**: `↑` / `↓` / `j` / `k` scroll line-by-line; `PgUp` / `PgDn` page; `Home` / `End` jump to oldest/newest log entries.
- **Dismissal**: `Esc` or `L`.

### SSH Authentication Popup (`AuthPopup`)
- **Activation**: Triggered automatically when an SSH operation requests a password or decrypted key passphrase via `SshAuthRequest` (`src/host/auth.rs`).
- **Behavior**: Highest priority modal; intercepts all keystrokes. Input is masked with asterisks. The field is `InputField::new_secret()`: no undo/kill history, a pre-reserved 256-byte buffer, and `wipe()` zeroizes it on `Enter`, `Esc` and drop.
- **Actions**: `Enter` submits the credential over a Tokio oneshot channel (`responder`); `Esc` cancels the request.
- **Queueing**: Requests arriving while the popup is open wait in `PopupState::auth_queue` and appear one at a time after each `Enter`/`Esc`. A popup left unanswered for 120 s fails its host and closes on its own; popups whose operation was cancelled are dropped the same way. Hosts sharing an encrypted key are asked once per connect batch (see [ssh-transport.md](ssh-transport.md#authentication-order)).

### Export Popup (`ExportPopup`)
- **Activation**: Press `o` on the View tab when snapshots, log rows, or list entries are present.
- **Behavior**: Prompts for an output file path (defaults to timestamped `.json` or `.html`).
- **Actions**: `Enter` confirms and exports the report; `Esc` cancels.

### Member & Name Pickers (`MemberPicker`)
- **Activation**: Press `Enter` on multi-select/single-select fields in Operate or View tabs (e.g., Target Groups, Target Hosts, Skip Hosts, Shell Mode, Check Names, Sync Names, Sync Source).
- **Navigation**: `↑` / `↓` / `j` / `k` move highlight; `Space` or `x` toggles item selection; `a` (where supported) jumps to the Config tab to create a missing entry; `Enter` commits the selection; `Esc` cancels without applying.

### Progress & Report Popups
- **Running Operation**: Displays real-time per-host outcome streaming with auto-scrolling progress. `Esc` requests graceful cooperative cancellation. `↑`/`↓`/`PgUp`/`PgDn` engage manual scrolling; `End` resumes auto-scroll tracking.
- **Completed Report**: Displays final execution statistics and host summaries. `Enter` or `Esc` dismisses; `↑`/`↓`/`PgUp`/`PgDn` scroll.

---

## Tab reference

### 1. Config Tab

The Config tab (`src/tui/tabs/config_tab.rs`) provides an interactive interface for viewing, editing, adding, and deleting configuration records (`settings`, `hosts`, `checks`, `syncs`).

```
┌─ Config: Sidebar ──────────┐┌─ Field Table: host[0] (web-prod-1) ──────────┐
│ ▼ Settings                 ││ name              web-prod-1                 │
│ ▼ Hosts (3)                ││ ssh_host          192.168.1.10               │
│   ▶ web-prod-1             ││ shell             sh                         │
│     web-prod-2             ││ groups            [web, prod]                │
│     db-primary             ││ proxy_jump        (none)                     │
│ ▶ Checks (2)               ││                                              │
│ ▶ Syncs (1)                ││                                              │
└────────────────────────────┘└──────────────────────────────────────────────┘
```

#### Zones & Navigation
- **Sidebar (Left Zone)**: Hierarchical tree showing sections and child entries.
  - Sections: `Settings`, `Hosts` (`[[host]]`), `Checks` (`[[check]]`), `Syncs` (`[[sync]]`).
  - `Space` / `Enter` on section headers: Toggles section collapse (`CollapsedSections`).
  - `↑` / `↓` / `j` / `k`: Navigate sidebar items.
  - `→` / `Tab` / `Enter` on an entry: Shifts focus to the **FieldTable** zone.
- **FieldTable (Right Zone)**: Unified table displaying field keys, kinds, and current values.
  - `↑` / `↓` / `j` / `k`: Move row selection across field descriptors.
  - `←` / `BackTab`: Returns focus to the **Sidebar**.

#### Editing & Value Mutation
- **Inline Scalar Edit**: Press `e` or `Enter` on a scalar field (`String`, `OptionalString`, `U64`) to activate inline `InputField`. Press `Enter` to commit or `Esc` to cancel.
- **Option / Enum Cycling**: Press `Space`, `e`, or `Enter` on an enum, boolean, or tri-bool field (`FieldKind::Enum`, `FieldKind::Bool`, `FieldKind::TriBool`, `FieldKind::ShellEnum`) to cycle variants in place.
- **Quick-Clear Optional**: Press `Delete` on an `OptionalString` field to instantly clear its value (required fields like `name` cannot be cleared).
- **Direct Sub-Popups**:
  - `groups` (`FieldKind::VecString`): Opens `DirectGroupPickerState` showing all known groups across the config with toggle checkboxes.
  - `enabled` (`FieldKind::CheckEnabled`): Opens `DirectGroupPickerState` over fixed probe options (`online`, `system_info`, `cpu_arch`, `memory`, `swap`, `disk`, `cpu_load`, `network`, `battery`, `ip_address`).
  - Vector fields / custom lists: Opens `DirectVecEditorState` for list item management (`a` or `Enter` to add, `d` to delete selected, `s` to save, `Esc` to cancel).
  - The same editors open inside the Add/Edit entry form (`VecEditorState`, `GroupPickerState`) with the same keys: both paths run one key map (`list_editor_key`, `picker_key`), so `Esc` always discards the pending list change and `s` (or `Enter` in pickers) applies it; `Esc` while typing a new item cancels just that item.

#### Entry Management
- **Add Entry (`a`)**: Press `a` while an entry or section is selected to open `EntryFormState` pre-populated with required and default fields for that type. A form taller than its popup scrolls: the field list gets the popup height minus its two hint rows, and the cursor stays put while moving inside the visible window.
- **Delete Entry (`d`)**: Press `d` to request deletion of the selected `host`, `check`, or `sync` entry. A confirmation popup (`ConfirmState`) prompts before deletion.

#### External Editor & Comment Preservation
- **External Editor (`E`)**: Press `E` to suspend the TUI and open `config.toml` in `$VISUAL`, `$EDITOR`, or `vi` (`notepad` on Windows) — the same order as `sshi config` (`commands::config::resolve_editor`). The TUI tracks file modification timestamps (`config_mtime`) and automatically reloads and re-validates the configuration upon editor exit.
- **Comment Preservation**: All programmatic writes use `toml_edit` (`src/config/app.rs`), preserving user comments, whitespace, formatting, and unrecognized top-level tables.

---

### 2. Operate Tab

The Operate tab (`src/tui/tabs/operate_tab.rs`) is the operational command center. It unifies operation configuration, target resolution, and command execution into a linear focus flow (`OpField`).

```
┌─ Operate ──────────────────────────────────────────────────────────────────┐
│ Operation: [◉ run]  [○ exec]  [○ sync]  [○ cp]  [○ check]                  │
│                                                                            │
│ ── Common ──                                                               │
│ Target:  ◉ All   ○ Groups   ○ Hosts   ○ Shell   (3 hosts)                  │
│ Skip:    (none)                                                            │
│ Timeout: 30s                                                               │
│ [ ] Serial (s)                                                             │
│ [ ] dry-run (d)                                                            │
│ ┌─ Output report (.json/.html, optional) ────────────────────────────────┐ │
│ │                                                                        │ │
│ └────────────────────────────────────────────────────────────────────────┘ │
│                                                                            │
│ ── check params ──                                                         │
│ Entries: (default)  (Enter: choose)                                        │
└────────────────────────────────────────────────────────────────────────────┘
┌─ Execute ──────────────────────────────────────────────────────────────────┐
│ [ Execute check (Enter) ] (e)                                              │
└────────────────────────────────────────────────────────────────────────────┘
```

#### Linear Field Walk (`operate_fields`)
Focus moves linearly through focusable elements using `↑` / `↓` / `j` / `k`:
1. `OpRadio`: Operation selector (`Run`, `Exec`, `Sync`, `Cp`, `Check`). `←` / `→` cycles operations in this order.
2. `TargetMode`: Filter strategy (`All`, `Groups`, `Hosts`, `Shell`). `←` / `→` cycles mode.
3. `TargetMembers`: Visible when mode is not `All`. `Enter` opens member picker (`Space` cycles shell when mode is `Shell`).
4. `Skip`: Excluded hosts. `Enter` opens host picker; `Delete` clears.
5. `Timeout`: Per-host execution timeout in seconds. `←` / `→` adjusts by $\pm 5\text{s}$ (minimum 1s).
6. `Serial`: Toggle parallel vs serial ($1$ host at a time) execution. `Space` or `s` toggles.
7. `DryRun`: Toggle dry-run preview mode. `Space` or `d` toggles.
8. `Out`: Path to write report output (`-o/--out`). `Enter` activates input; `Delete` clears.
9. **Command-Specific Fields**:
   - `Check`: `CheckName` (multi-select picker or comma-separated names).
   - `Run`: `Command` (command line string), `Sudo` (boolean toggle).
   - `Exec`: `Script` (local script path), `Sudo` (boolean toggle), `Keep` (keep remote temp script).
   - `Sync`: `SyncName` (sync entry picker), `SyncAdhocInput` (ad-hoc file path input; Enter appends to list), `SyncSource` (source host override picker/cycler).
   - `Cp`: `CpLocal` (local file/dir/glob), `CpRemote` (remote destination path).
10. `Execute`: Action button. Press `Enter` to run.

#### Global Operate Shortcuts
- `e`: Trigger execution immediately from any focused field on the tab.
- `d`: Quick-toggle dry-run state.
- `s`: Quick-toggle serial execution state.
- `Delete`: Quick-clear focused optional field or pop the last ad-hoc sync path.
- `Tab` / `BackTab`: Cycle focus between logical field layers (`OpLayer`: `Op`, `Common`, `CommandSpecific`, `Execute`).

---

### 3. View Tab

The View tab (`src/tui/tabs/view_tab.rs`) is a multi-mode inspection dashboard. The sub-view is selected via `ViewOperationKind` (`OpSelector`):

```
┌─ View ─────────────────────────────────────────────────────────────────────┐
│ Show:  Checkout   List   Log   ←/→ to switch                               │
│ Target:  ◉ All   ○ Groups   ○ Hosts   ○ Shell   (3 hosts)                  │
│ Skip:    none                                                              │
│ Combined: [ ]  c=toggle                                                    │
└────────────────────────────────────────────────────────────────────────────┘
┌─ Checkout ─────────────────────────────────────────────────────────────────┐
│ Host             Status      OS            CPU%   Mem%   Disk%   Last Seen │
│   web-prod-1     ✓ online    Ubuntu 24.04  4.2%   38.1%  52.0%   2m ago    │
│   web-prod-2     ✓ online    Ubuntu 24.04  6.8%   41.0%  54.2%   2m ago    │
│   db-primary     ✓ online    Debian 12     18.5%  82.4%  68.1%   1m ago    │
└────────────────────────────────────────────────────────────────────────────┘
```

#### Sub-Views
1. **Checkout (`ViewOperationKind::Checkout`)**:
   - Displays latest system metrics from SQLite (`check_snapshots` table).
   - **Columns**: `Host`, `Status` (`✓ online` / `✗ offline`), dynamic metric columns from configured check probes, and `Last Seen` relative timestamp.
   - **Combined View (`c` / Space)**: Toggles between point-in-time snapshot view and combined latest metric compilation across all check runs.
   - **Navigation**: `↑` / `↓` / `j` / `k` move row selection; `PgUp` / `PgDn` / `Home` / `End` scroll viewport.
2. **List (`ViewOperationKind::List`)**:
   - Displays configured inventory of hosts, checks, and syncs filtered by active target selection.
   - **Jump to Config (`e`)**: Pressing `e` on a selected host row immediately switches to the Config tab and focuses that host's field table.
3. **Log (`ViewOperationKind::Log`)**:
   - Queries and inspects historical execution records from `operation_log`.
   - **Filter Controls**:
     - `last` (input): Limit number of returned rows (default 20, 0 for unlimited).
     - `since` (input): ISO-8601 or relative time filter.
     - `host` (input): Target host filter string.
     - `action` (cycler): Filter by command kind (`None` $\rightarrow$ `check` $\rightarrow$ `run` $\rightarrow$ `exec` $\rightarrow$ `cp` $\rightarrow$ `sync` $\rightarrow$ `None`).
     - `errors` (toggle): Restrict to failed operations only.

#### Exporting Reports
Pressing `o` on the View tab opens the `ExportPopup`. Data from Checkout snapshots, List entries, or Log records can be serialized directly to a `.json` or `.html` file.

---

## Line editing & input fields

Text inputs throughout the TUI (`InputField` in `src/tui/components/input_field.rs`) implement an Emacs-compatible editing model with full Unicode grapheme-cluster awareness.

### Input Modes & Isolation
- `InputMode::Normal`: The field displays its current text; global single-key shortcuts operate normally.
- `InputMode::Active`: The field captures keyboard focus with a visible cursor block. **All global single-letter shortcuts (such as `q`, `1`, `2`, `3`, `?`, `i`, `L`) are suspended** to prevent accidental command dispatch while typing.

### Keybinding Reference

| Key | Operation | Description |
|-----|-----------|-------------|
| `Char(c)` | `insert_char` | Insert character at cursor position; advances cursor by 1 grapheme. |
| `Ctrl+A` | Move to start | Move cursor to beginning of the line (`cursor_pos = 0`). |
| `Ctrl+E` | Move to end | Move cursor to end of the line (`cursor_pos = grapheme_count`). |
| `Ctrl+K` | Kill to end | Delete text from cursor to end of line; pushes killed text to kill ring. |
| `Ctrl+U` | Kill to start | Delete text from cursor to start of line; pushes killed text to kill ring. |
| `Ctrl+W` / `Alt+Backspace` | Kill word back | Delete previous word; pushes deleted word to kill ring. |
| `Ctrl+Y` | Yank | Paste most recent entry from the kill ring at cursor position. |
| `Ctrl+Z` / `Ctrl+_` | Undo | Revert previous mutating operation from undo history stack. |
| `Ctrl+Left` | Prev word | Move cursor backward to start of previous word. |
| `Ctrl+Right` | Next word | Move cursor forward to end of next word. |
| `Left` | Move left | Move cursor left by 1 Unicode grapheme cluster. |
| `Right` | Move right | Move cursor right by 1 Unicode grapheme cluster. |
| `Home` | Move to start | Jump cursor to line start. |
| `End` | Move to end | Jump cursor to line end. |
| `Backspace` | Delete back | Delete 1 Unicode grapheme cluster behind cursor. |
| `Delete` | Delete forward | Delete 1 Unicode grapheme cluster in front of cursor. |
| `Enter` | Confirm | Commit edited value, transition to `InputMode::Normal`, and snapshot baseline. |
| `Esc` | Cancel | Revert field value to saved baseline snapshot and return to `InputMode::Normal`. |

### Kill Ring & Undo Buffer
- **Kill Ring**: Capacity of up to 8 entries (`RING_MAX = 8`). Sequential kills store string slices in memory for `Ctrl+Y` yank retrieval.
- **Undo Buffer**: Capacity of up to 8 states. Every mutation (`insert_char`, `delete_grapheme_*`, `kill_*`) records a `(String, usize)` tuple capturing the previous text and cursor position.

---

## Theming, colors & terminal compatibility

Theming and glyph resolution are managed in `src/tui/theme.rs` (`Theme`).

### 16-Color Palette
To ensure broad terminal compatibility across standard ANSI emulators and SSH sessions, `sshi` uses exclusively 16-color named `ratatui::style::Color` variants (no 24-bit TrueColor RGB or 256-color palette requirements):

| Color Slot | Default Value | Usage |
|------------|---------------|-------|
| `accent_config` | `Color::Yellow` | Config tab headers, active borders, and highlights. |
| `accent_operate` | `Color::Cyan` | Operate tab headers, radio focus, and buttons. |
| `accent_checkout` | `Color::Green` | View tab headers and snapshot column highlights. |
| `error` | `Color::Red` | Error banners, failed host status, offline indicators. |
| `warning` | `Color::Yellow` | Warnings, partial host status, pending state. |
| `inactive` | `Color::DarkGray` | Unfocused borders, secondary hints, disabled labels. |
| `border_active` | `Color::Cyan` | Focused block outline border. |
| `border_inactive` | `Color::DarkGray` | Unfocused block outline border. |

### NO_COLOR Support
Per the [NO_COLOR specification](https://no-color.org), if the `NO_COLOR` environment variable is set to any non-empty string, `Theme::from_env()` invokes `Theme::no_color()`. Every color slot is set to `Color::Reset`, ensuring no ANSI color escape sequences are written to stdout.

### Linux Console ASCII Fallback (`TERM=linux`)
When running on the Linux virtual console (`TERM=linux`), standard Unicode symbols can render as question marks or empty glyph boxes. `sshi` detects `TERM=linux` and switches from Unicode to pure ASCII glyphs:

| Status | Unicode (`GlyphSet::unicode()`) | ASCII (`GlyphSet::ascii()`) |
|--------|---------------------------------|-----------------------------|
| `HostStatus::Online` | `✓` | `+` |
| `HostStatus::Offline` / `Error` | `✗` | `x` |
| `HostStatus::Unreachable` / `Skipped` | `⊘` | `o` |
| `HostStatus::Partial` | `⚠` | `!` |
| `HostStatus::TimedOut` | `⏱` | `⏱` |

`NO_COLOR` and `TERM=linux` operate independently: a monochrome theme can render Unicode glyphs on modern terminals, while a colored terminal running under `TERM=linux` receives color escapes with ASCII status glyphs.

---

## Persisted state schema

The TUI automatically persists UI navigation and filter state on exit (`src/tui/state/persist.rs`).

### File Location & Naming
- **Path**: `{state_dir}/tui_state-{config_hash}.toml`
  - `state_dir`: Derived via `state::db::resolved_state_dir`, honoring the `[settings].state_dir` override if specified, otherwise the platform state directory (`state::db::state_dir`; see [state-schema.md](state-schema.md)). For the default config, a missing `tui_state-{hash}.toml` is seeded once from the file keyed by the legacy `~/.config/sshi/config.toml` path (`persist::state_file_path`), so the B44 directory move keeps saved TUI state.
  - `config_hash`: First 8 hex characters of the BLAKE3 hash of the canonicalized configuration file path string.
- **Atomic Persistence**: Written atomically via `tempfile::NamedTempFile::persist()` to prevent corruption on sudden termination.
- **Debounced writes**: a state change schedules one write once input has been idle for 500 ms (`STATE_SAVE_DEBOUNCE`), plus a final write on quit — not one write per keypress.
- **UI thread**: `render` does no I/O. View data (SQLite) loads in the event loop before a frame is drawn, `--out`/export report files are written on a blocking task (the result arrives as a footer notice), the results popup is built once when a report arrives, and the target-count badge is recomputed only when the config or filter changes.
- **Fault Tolerance**: Missing, unreadable, or invalid state files fall back to defaults silently without panicking.

### TOML Schema Structure

```toml
[tui_state]
active_tab = "View" # "Config" | "Operate" | "View" (alias "Checkout")

[target_filter]
mode = "All"        # "All" | "Groups" | "Hosts" | "Shell"
groups = ["web"]
hosts = []
skip = []
shell = "Sh"        # "Sh" | "PowerShell" | "Cmd"
serial = false
timeout = 30

[operate]
operation = "Check" # "Check" | "Run" | "Exec" | "Sync" | "Cp"
run_sudo = false
exec_sudo = false
exec_keep = false
dry_run = false
sync_dry_run = false
check_dry_run = false
run_dry_run = false
exec_dry_run = false
view_operation = "Checkout" # "Checkout" | "List" | "Log"
checkout_combined = false
log_last = 20
log_errors = false
run_command = "uptime"
exec_script = "./scripts/deploy.sh"
cp_local = "./dist/*"
cp_remote = "/var/www/app/"
```

### State Validation & Sanitization
On load, `validate_filter()` sanitizes persisted state against the active `AppConfig`:
1. **Group/Host Pruning**: Target groups or hosts not defined in the loaded `config.toml` are silently dropped.
2. **Explicit Zero-Target Preservation**: If a `Groups` or `Hosts` filter resolves to an empty list after pruning, the mode is preserved rather than widened to `All`, preventing accidental operations against all hosts.

---

See [AGENTS.md](../../AGENTS.md) for TUI contributor rules and merge gates.
