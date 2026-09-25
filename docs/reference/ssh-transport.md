# SSH transport reference

This document describes the `russh`-based SSH transport layer in `sshi`. All remote execution, metric collection, and file synchronization operations run over embedded pure-Rust SSH sessions managed by `RusshSessionPool` and orchestrated via `SshPool`.

## Overview

The transport stack is structured across five primary modules and testing abstractions:

1. **`SessionPool` trait (`src/host/session_pool.rs`) & `MockSessionPool` (`src/host/session_pool_mock.rs`)**: Async trait abstracting remote command execution and SFTP transfers across production SSH sessions and headless test mocks.
2. **`RusshSessionPool` (`src/host/session_pool.rs`)**: Thread-safe pool implementing `SessionPool`, managing authenticated `russh` client sessions (`Handle<SshHandler>`), channel multiplexing, `known_hosts` verification, and lazy SFTP session caching.
3. **`SshPool` (`src/host/pool.rs`)**: High-level orchestrator wrapping `RusshSessionPool`, `ConcurrencyLimiter`, and `SyncProgress` for subcommand execution.
4. **`ConcurrencyLimiter` (`src/host/concurrency.rs`)**: Two-tier semaphore enforcing global and per-host concurrency limits without head-of-line blocking.
5. **`SftpSession` (`src/host/sftp.rs`)**: Streaming file upload and download operations built on `russh-sftp`.
6. **Authentication & TUI Bridge (`src/host/auth.rs`)**: Multi-step credential resolution chain supporting unencrypted keys, encrypted keys with passphrase caching, and password fallback across CLI and TUI surfaces.

---

## RusshSessionPool & Connection Multiplexing

### Architecture

`RusshSessionPool` (`src/host/session_pool.rs`) maintains a map of open, authenticated SSH client handles keyed by host alias:

```rust
pub struct RusshSessionPool {
    sessions: HashMap<String, Arc<Handle<SshHandler>>>,
    failed: Vec<(String, String)>,
    sftp_failed: Vec<(String, String)>,
    home_dirs: tokio::sync::Mutex<HashMap<String, String>>,
    sftp_cache: LazyCache<SftpSession>,
    proxy_cancels: Vec<tokio::sync::oneshot::Sender<()>>,
}
```

- **Concurrent Connect**: Established during `RusshSessionPool::setup` using a `tokio::task::JoinSet` bounded by a connection semaphore. Unreachable hosts are captured in `failed` with their full error chain (`{:#}`) rather than aborting setup.
- **Connect timeouts**: `timeout_secs` (`default_timeout` / `--timeout`) bounds every network step of a connection separately: DNS (`resolve_addr`, via `tokio::net::lookup_host` so a slow resolver never blocks a runtime worker), TCP connect + handshake, the ProxyJump tunnel open, each server round-trip of `authenticate` (not the time spent at a passphrase/password prompt), and SFTP subsystem negotiation (`open_sftp_bounded`, in both `run_sftp_probe` and `sftp_session`).
- **Multiplexing**: A single authenticated `russh::client::Handle` per host alias handles all subsequent operations. Multiple concurrent channels (command execution channels and SFTP subsystem sessions) are opened over this shared connection without additional TCP handshakes.
- **Keepalive Policy**: Configured with `keepalive_interval: Some(Duration::from_secs(30))` and `inactivity_timeout: None` in `russh::client::Config`. This sends periodic keepalive packets to prevent NAT and firewall timeouts during idle intervals while leaving operation timeouts to per-call `tokio::time::timeout` wrappers.
- **ProxyJump Support**: Supports single-hop jump hosts (`proxy_jump` directive). `connect_via_proxy` connects to the bastion host, opens a `direct-tcpip` tunnel channel via `channel_open_direct_tcpip`, spawns a background keepalive task holding the proxy handle, and establishes the target session via `client::connect_stream` over the channel stream.
- **Reconnect on a dropped connection**: every operation fetches its handle through `RusshSessionPool::handle`. If russh reports the connection closed (`Handle::is_closed`, e.g. the server or network dropped it mid-run), the pool reconnects that host once with the same ssh_config resolution, passphrase cache and auth bridge, drops the SFTP and rename channels opened on the old connection, and continues; the operation that was in flight when the drop happened still fails. A host whose reconnect fails keeps failing fast for the rest of the run. Reconnects are serialized.
- **Graceful Shutdown**: On `shutdown()`, background proxy keepalive tasks are signaled via oneshot channels, and every active handle sends `Disconnect::ByApplication` before disconnection.

### Server Host Key Verification (`known_hosts`)

Server public key verification is implemented in `SshHandler::check_server_key` (`src/host/session_pool.rs`), which implements `russh::client::Handler`:

- Verification checks the host key against `~/.ssh/known_hosts` using `russh::keys::check_known_hosts_path(&self.hostname, self.port, server_public_key, &known_hosts_path)`.
- **Missing File**: If `~/.ssh/known_hosts` does not exist, connection bails: `Unknown host key for <host>:<port> — run sshi init to add the host to known_hosts`.
- **Unknown Key**: If the key is not present in `known_hosts` (`Ok(false)`), connection bails: `Unknown host key for <host>:<port> — run sshi init to accept the key first`.
- **Certificates**: `check_server_key` receives a `PublicKeyOrCertificate`; a host that presents an SSH certificate is refused (`certificate host keys are not supported`).
- **Key Mismatch / MITM Protection**: If the key differs from the stored record (`Err(russh::keys::Error::KeyChanged { line })`), connection bails with a high-visibility warning: `HOST KEY MISMATCH for <host>:<port> at line <line> — possible man-in-the-middle attack`.
- **Unhashed Entries Requirement**: `russh` matches host keys by plain string comparison (`host` or `[host]:port`). Hashed entries (`|1|...`) produced by `ssh-keyscan -H` are not matched; `sshi init` writes unhashed entries specifically to maintain compatibility with `russh`.

### Command Execution (`exec_on_handle`)

Remote execution runs via `exec_on_handle` (`src/host/session_pool.rs`):
1. Opens an SSH channel via `handle.channel_open_session()`.
2. Requests command execution via `channel.exec(true, cmd)`.
3. Loops over `channel.wait()`, collecting `ChannelMsg::Data` into `stdout`, `ChannelMsg::ExtendedData { ext: 1 }` into `stderr`, and `ChannelMsg::ExitStatus` into `exit_code`.
4. Returns `RemoteOutput { stdout, stderr, exit_code, success }` where `success` is `exit_code == Some(0)`.

### Lazy SFTP Channel Caching

SFTP sessions are managed through `LazyCache<SftpSession>`:
- Uses double-checked locking over `tokio::sync::Mutex<HashMap<String, Arc<SftpSession>>>`. The lock is released across the async channel opening call so concurrent opens for different hosts do not serialize.
- Once opened during `run_sftp_probe` or first transfer, the `SftpSession` is cached for the pool's lifetime and reused across file operations.

---

## SshPool High-Level Wrapper

`SshPool` (`src/host/pool.rs`) is the central handle used by subcommands (`check`, `sync`, `run`, `exec`, `cp`). `init` manages `RusshSessionPool` instances directly via `InitPools` (`commands::init::core::InitPools`).

```rust
pub struct SshPool {
    pub(crate) session_pool: Arc<RusshSessionPool>,
    pub limiter: ConcurrencyLimiter,
    pub progress: SyncProgress,
}
```

- **Setup (`SshPool::setup` / `setup_with_options`)**:
  - Initializes `ConcurrencyLimiter` with global and per-host caps.
  - Initializes terminal `SyncProgress` bars.
  - Connects to all hosts concurrently via `RusshSessionPool::setup`.
  - When `probe_sftp` is enabled (e.g. for `sync` and `cp`), executes `run_sftp_probe` against all reachable hosts and updates progress counts with effectively capable hosts.
- **Host Filtering**:
  - `filter_reachable(&hosts)`: Returns only `HostEntry` items whose SSH connection succeeded.
  - `failed_hosts()`: Returns `(host_alias, error_message)` for connection failures.
  - `filter_sftp_capable(&hosts)`: Returns only `HostEntry` items that passed the SFTP capability probe.
  - `sftp_failed_hosts()`: Returns `(host_alias, error_message)` for SFTP probe failures.
- **Shutdown**: Clears progress displays and unwraps `session_pool` to execute `RusshSessionPool::shutdown()`.

---

## Concurrency Limiting

The `ConcurrencyLimiter` (`src/host/concurrency.rs`) controls concurrency using a dual-level semaphore architecture.

### Dual-Level Semaphores

```rust
pub struct ConcurrencyLimiter {
    global: Arc<Semaphore>,
    per_host: HashMap<String, Arc<Semaphore>>,
}
```

- **Global Cap**: Limits simultaneous active operations across the entire process.
- **Per-Host Cap**: Limits simultaneous active operations targeting the same host.

### Default Limits

- **`max_concurrency`**: Default `10` (configured in `[settings]` in `config.toml`).
- **`max_per_host_concurrency`**: Default `4` (configured in `[settings]` in `config.toml`).
- **`--serial` Flag**: Overrides both limits to `1` across all command contexts (`Context::concurrency()`, `Context::per_host_concurrency()`).

### Permit Acquisition Order & Deadlock Prevention

Permit acquisition in `ConcurrencyLimiter::acquire(host)` strictly follows this order:

1. **Per-host semaphore permit acquired first.**
2. **Global semaphore permit acquired second.**

**Why per-host first**: This prevents **head-of-line blocking**. If tasks acquired the global permit first, multiple tasks queued for a single saturated host would each hold a global permit while waiting for that host's lock, starving tasks for other completely idle hosts. Acquiring per-host first ensures tasks only consume a global permit when their target host has an available execution slot.

**Caller Consistency Note**: Callers follow this per-host-first ordering, preventing circular wait across operations. All callers, including `sync::distribute::distribute_pooled`, acquire permits via `ConcurrencyLimiter::acquire` (per-host first, then global).

Both permits are held inside an RAII `ConcurrencyPermit` guard and released on drop.

---

## SFTP Streaming Transfers

File operations in `src/host/sftp.rs` are built on `russh_sftp::client::SftpSession`.

### Streaming I/O and No Fixed Size Cap

- **Streaming Implementation**: `upload` and `download` stream between a local `tokio::fs::File` and the remote `sftp.create()` / `sftp.open()` handle through `copy_idle`, in 256 KiB chunks. There is **no fixed file size limit** and no whole-file buffering.
- **Temp file + rename**: data is written to a sibling temp file `.<name>.sshi-tmp.<pid>` (`temp_sibling`) and renamed over the destination only after every byte arrived and the file closed cleanly. An interrupted, failed or timed-out transfer leaves the existing destination untouched and removes the temp file (locally also on cancellation, via `TempGuard`).
- **Atomic replace**: when the server advertises `posix-rename@openssh.com` (OpenSSH does), the rename goes over a second, raw SFTP channel per host (`RenameChannel`; russh-sftp's `SftpSession` cannot send extended requests) and replaces an existing destination atomically — readers never see it missing. Without the extension, an existing destination is removed and the temp file renamed (a brief gap).
- **Abandoned temp files**: a process killed with SIGKILL can leave `.<name>.sshi-tmp.<pid>` behind. The first upload into a directory in a run lists it and removes such files whose pid is not the current process and whose mtime is at least an hour old (`sweep_stale_temps`, `STALE_TEMP_SECS`), so a concurrent run's live temp file is never touched.
  - Remote: SFTP v3 rename refuses an existing target on OpenSSH, so `upload` retries after removing the destination — a brief window where the file is absent, never a truncated one.
  - Local (`write_local_atomic`): `rename` replaces the destination atomically.
- **Close errors surface**: the remote handle is flushed and `shutdown()` explicitly (`russh-sftp`'s `Drop` close is fire-and-forget); a flush or close failure fails the transfer before the rename.
- **Idle timeout**: `default_timeout` / `--timeout` bounds each step (open, each chunk read/write, flush, close, rename), not the whole transfer — a slow transfer that keeps moving never times out; one stalled for the timeout fails with `… timed out (no progress for Ns)`.

### Path Resolution and Auto-Directory Creation

- **Remote Home Directory Detection**: `remote_home_dir` evaluates remote home paths by running shell-appropriate commands (`echo $HOME` for POSIX `Sh`, `Write-Output $env:USERPROFILE` for `PowerShell`, `echo %USERPROFILE%` for `Cmd`).
- **Tilde Expansion**: `resolve_remote_path` resolves leading `~` or `~/` prefixes against the detected home directory.
- **Recursive Directory Creation**: `mkdir_p_sftp` splits remote destination paths and creates parent directories recursively via `sftp.create_dir()`, ignoring existing directory errors.

### SFTP Health Probe

`sftp_probe` verifies remote SFTP read/write capability by creating `~/.sshi_probe`, writing byte `b"0"`, and removing the file. Successful probe results are cached in the session pool to eliminate redundant negotiation during subsequent transfers.

---

## Authentication Chain & TUI Auth Bridge

Authentication is orchestrated by `authenticate()` in `src/host/auth.rs`.

### Authentication Order

For each connection, credentials are evaluated in the following sequence:

1. **ssh-agent** (Unix): if `SSH_AUTH_SOCK` is set, each agent public key is offered via `handle.authenticate_publickey_with` with the agent as signer (`auth::try_agent`). With `IdentitiesOnly`, only agent keys whose key data matches a listed identity file's `.pub` are offered. Agent errors are non-fatal. The Windows agent pipe is not used.
2. **Unencrypted Public Keys**: Iterates over all configured `identity_files` in order, attempting `russh::keys::load_secret_key(path, None)` and `handle.authenticate_publickey` (wrapped in `PrivateKeyWithHashAlg` with the server's `best_supported_rsa_hash`, so RSA keys sign with `rsa-sha2-*`). Unencrypted keys authenticate immediately without prompting.
   `identity_files` holds every `IdentityFile`, or the OpenSSH default keys when none is configured (see [config-schema.md](config-schema.md)).
3. **Encrypted Public Keys with Passphrase**: Only for identity files that exist and are encrypted (`auth::is_encrypted_key`: OpenSSH-format key with a cipher, or legacy PEM/PKCS#8 encrypted header). Missing or unparsable files never trigger a prompt:
   - `auth::unlock_key` locks the `SharedPassphraseCache` (`Arc<Mutex<HashMap<PathBuf, SecretString>>>`), created once per `RusshSessionPool::setup` and shared by every host in the batch.
   - If the key's passphrase is cached it is reused; otherwise the user is prompted **while the lock is held**, so hosts sharing a key are asked once and other hosts wait for that answer.
   - Only a passphrase that actually decrypts the key (`load_secret_key(path, Some(pp))`) is cached; a wrong one is not reused, so the next host needing the key asks again. An empty answer skips the key.
   - The unlocked key is then offered with `handle.authenticate_publickey`.
4. **Password Fallback**: If all identity files fail and `IdentitiesOnly` is not enabled in SSH config (`!identities_only`), prompts the user with `<user>@<host> password: ` (also under the shared lock, so prompts never overlap; passwords are not cached) and calls `handle.authenticate_password`.

If all available methods are exhausted without success, authentication fails with `All authentication methods exhausted for user '<user>'`.

### Credential Memory Security (`SecretString`)

Passphrases and passwords are stored in `SecretString` (`src/host/auth.rs`), which implements `zeroize::Zeroize` on `Drop`. When dropped, the underlying buffer memory is zeroed out. Its `Debug` output is redacted (`SecretString(***)`).

### TUI Auth Bridge

Interactive prompts (`prompt_credential`) support two operating modes:

- **CLI Path (`auth_sender: None`)**: Prompts directly on the terminal via `rpassword::prompt_password`, run in `spawn_blocking` so the TTY read never blocks an async worker.
- **TUI Bridge Path (`auth_sender: Some(&SshAuthSender)`)**: Non-blocking asynchronous bridge connecting the background SSH task to the TUI event loop:
  1. Spawns a `tokio::sync::oneshot::channel::<String>()`.
  2. Sends `SshAuthRequest { prompt, responder: tx }` over an unbounded `mpsc` channel to the TUI main loop.
  3. The TUI displays a modal input popup with masked text entry. A request that arrives while a popup is open is queued (`PopupState::push_auth`) and shown when the current one is submitted or cancelled (`PopupState::next_auth`); it never replaces the open popup.
  4. On Enter, the TUI moves (not copies) the entered credential string across the oneshot channel to unblock the SSH authentication task. If the user cancels (Escape), the responder is dropped, surfacing an error to the auth task.
  5. An unanswered popup fails that host after `AUTH_POPUP_TIMEOUT` (120 s) with `credential prompt timed out after 120s` (`await_credential`), releasing the shared prompt lock. Debug builds accept `SSHI_AUTH_PROMPT_TIMEOUT_SECS` to shorten it for tests. When the auth task stops waiting (timeout or operation cancelled), the TUI drops that popup and any such queued request (`PopupState::prune_stale_auth`) before the next draw. The CLI terminal prompt has no timeout, like `ssh`.
  6. Architecture and lifecycle decisions are documented in [ADR 0001: SSH Auth TUI Popup](../adr/0001-ssh-auth-tui-popup.md).

---

## Remote Shell Detection

Remote shell detection (`src/host/shell.rs::detect_russh`) executes probe commands over the established session to determine the remote OS and shell type:

1. **PowerShell**: Runs `$PSVersionTable.PSVersion.Major`. If stdout is non-empty and exit status is successful, returns `ShellType::PowerShell`.
2. **Cmd (Windows)**: Runs `ver`. If stdout contains `"Windows"` and exit status is successful, returns `ShellType::Cmd`.
3. **POSIX Sh**: Runs `echo ok`. If exit status is successful, returns `ShellType::Sh`.
4. **Fallback**: If all command attempts return errors, bails. If commands succeed without matching specific markers, defaults to `ShellType::Sh`.

The detected `ShellType` controls temporary directory paths (`temp_dir`: `/tmp`, `$env:TEMP`, `%TEMP%`) and privilege elevation wrapping (`sudo_wrap`: `sudo <cmd>`, `Start-Process powershell -ArgumentList '-NoProfile','-EncodedCommand','<base64>' -Verb RunAs`, `runas /user:Administrator "<cmd>"`).

## Remote Quoting

Every value interpolated into a remote command string — sync paths, `[[check]]` path probes and labels, the `exec` script path, `sudo_wrap` — goes through `src/host/quote.rs`:

| Function | sh | PowerShell | Cmd |
|---|---|---|---|
| `quote_arg` (literal) | `'…'`, `'` → `'\''` | `'…'`, `'` → `''` | `"…"`; refuses `"` `%` `!` CR LF |
| `quote_path` (leading `~`/`~/` → home) | `"$HOME"/'rest'` | `($HOME + '\rest')` | `"%USERPROFILE%\rest"` |
| `cmd_echo_arg` (unquoted `echo`) | — | — | `^` before `^&\|<>()`; same refusals |

- PowerShell run from a cmd.exe host (sync Cmd branches) and elevated PowerShell (`sudo_wrap`) use `-EncodedCommand` with base64 UTF-16LE (`ps_in_cmd`, `encode_ps`), so the script never passes through a second quoting layer.
- A value cmd.exe cannot quote safely fails that operation with `… cannot be passed safely to a cmd.exe host` instead of running.
- Parity tests (`host::quote::tests`) loop every `ShellType` over values containing spaces, quotes, `$(...)`, backticks, `;`, `&`, `%`; generated sh commands run in a real `sh` and must not execute the payload.

---

## Pure-Rust russh vs Subprocesses

`sshi` uses pure-Rust `russh` / `russh-sftp` for all operational workflows while retaining legacy OpenSSH CLI utilities for initial host setup and onboarding:

| Subsystem / Operation | Implementation Type | Component | Description |
|---|---|---|---|
| **Session & Connection Multiplexing** | Pure-Rust (`russh`) | `src/host/session_pool.rs` | TCP connect, session caching, keepalive, and ProxyJump tunnel stream |
| **Server Host Key Verification** | Pure-Rust (`russh::keys`) | `src/host/session_pool.rs` | `known_hosts` checking and MITM mismatch detection |
| **Authentication Chain** | Pure-Rust (`russh`, `russh::keys`) | `src/host/auth.rs` | ssh-agent, public key, passphrase caching, and password fallback |
| **Command Execution (`check`, `run`, `exec`)** | Pure-Rust (`russh`) | `src/host/session_pool.rs` | Channel session open, command exec, stdout/stderr/exit code capture |
| **File Transfer (`sync`, `cp`)** | Pure-Rust (`russh-sftp`) | `src/host/sftp.rs` | SFTP subsystem session, directory creation, streaming file I/O |
| **Shell Detection** | Pure-Rust (`russh`) | `src/host/shell.rs` | Remote probe execution via session channel |
| **Host Key Discovery (`init`)** | System Subprocess (`ssh-keyscan`) | `src/commands/init/core.rs` | Executes `ssh-keyscan -p <port> <host>` to discover and append unhashed keys to `known_hosts` |
| **SSH Key Generation (`init`)** | System Subprocess (`ssh-keygen`) | `src/commands/init/mod.rs` | Runs interactive `ssh-keygen -t ed25519` when no default private key is found |
| **Public Key Deployment (`init`)** | System Subprocess (`ssh-copy-id`) | `src/commands/init/mod.rs` | Runs interactive `ssh-copy-id <host>` to install public keys on auth-failed hosts |
