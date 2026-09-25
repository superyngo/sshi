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
- **Multiplexing**: A single authenticated `russh::client::Handle` per host alias handles all subsequent operations. Multiple concurrent channels (command execution channels and SFTP subsystem sessions) are opened over this shared connection without additional TCP handshakes.
- **Keepalive Policy**: Configured with `keepalive_interval: Some(Duration::from_secs(30))` and `inactivity_timeout: None` in `russh::client::Config`. This sends periodic keepalive packets to prevent NAT and firewall timeouts during idle intervals while leaving operation timeouts to per-call `tokio::time::timeout` wrappers.
- **ProxyJump Support**: Supports single-hop jump hosts (`proxy_jump` directive). `connect_via_proxy` connects to the bastion host, opens a `direct-tcpip` tunnel channel via `channel_open_direct_tcpip`, spawns a background keepalive task holding the proxy handle, and establishes the target session via `client::connect_stream` over the channel stream.
- **Graceful Shutdown**: On `shutdown()`, background proxy keepalive tasks are signaled via oneshot channels, and every active handle sends `Disconnect::ByApplication` before disconnection.

### Server Host Key Verification (`known_hosts`)

Server public key verification is implemented in `SshHandler::check_server_key` (`src/host/session_pool.rs`), which implements `russh::client::Handler`:

- Verification checks the host key against `~/.ssh/known_hosts` using `russh_keys::check_known_hosts_path(&self.hostname, self.port, server_public_key, &known_hosts_path)`.
- **Missing File**: If `~/.ssh/known_hosts` does not exist, connection bails: `Unknown host key for <host>:<port> — run sshi init to add the host to known_hosts`.
- **Unknown Key**: If the key is not present in `known_hosts` (`Ok(false)`), connection bails: `Unknown host key for <host>:<port> — run sshi init to accept the key first`.
- **Key Mismatch / MITM Protection**: If the key differs from the stored record (`Err(russh_keys::Error::KeyChanged { line })`), connection bails with a high-visibility warning: `HOST KEY MISMATCH for <host>:<port> at line <line> — possible man-in-the-middle attack`.
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

**Caller Consistency Note**: Callers using `ConcurrencyLimiter::acquire` follow this per-host-first ordering, preventing circular wait across those operations. However, `sync::distribute::distribute_pooled` directly acquires the global semaphore before the per-host semaphore; in practice, sync upload tasks only compete against other uploads within the same distribution phase, avoiding deadlocks under current usage.

Both permits are held inside an RAII `ConcurrencyPermit` guard and released on drop.

---

## SFTP Streaming Transfers

File operations in `src/host/sftp.rs` are built on `russh_sftp::client::SftpSession`.

### Streaming I/O and No Fixed Size Cap

- **Streaming Implementation**: File uploads (`upload`) and downloads (`download`) stream data between local `tokio::fs::File` instances and remote `sftp.create()` / `sftp.open()` file handles via `tokio::io::copy`.
- **No File Size Cap**: Data is transferred incrementally in memory-efficient stream chunks (Tokio 8 KB copy buffer written through SFTP protocol frames). There is **no fixed file size limit** and no whole-file memory buffering.
- **Explicit Flush and Shutdown**: Remote file handles explicitly execute `remote_file.flush()` and `remote_file.shutdown()`. Because `russh-sftp`'s `Drop` implementation is fire-and-forget, explicit shutdown ensures write and close errors on the remote server are surfaced back to `sshi` rather than silently ignored.

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

1. **Unencrypted Public Keys**: Iterates over all configured `identity_files` in order, attempting `russh_keys::load_secret_key(path, None)` and `handle.authenticate_publickey`. Unencrypted keys authenticate immediately without prompting.
2. **Encrypted Public Keys with Passphrase**: For keys that failed unencrypted loading, attempts authentication using passphrases:
   - Checks `PassphraseCache` (`HashMap<PathBuf, SecretString>`), an in-memory process-scoped cache.
   - If uncached, prompts the user for the passphrase and caches the resulting `SecretString`.
   - Calls `russh_keys::load_secret_key(path, Some(passphrase))` and `handle.authenticate_publickey`.
3. **Password Fallback**: If all identity files fail and `IdentitiesOnly` is not enabled in SSH config (`!identities_only`), prompts the user with `<user>@<host> password: ` and calls `handle.authenticate_password`.

If all available methods are exhausted without success, authentication fails with `All authentication methods exhausted for user '<user>'`.

### Credential Memory Security (`SecretString`)

Passphrases and passwords are stored in `SecretString` (`src/host/auth.rs`), which implements `zeroize::Zeroize` on `Drop`. When dropped, the underlying buffer memory is zeroed out.

### TUI Auth Bridge

Interactive prompts (`prompt_credential`) support two operating modes:

- **CLI Path (`auth_sender: None`)**: Prompts directly on the terminal via `rpassword::prompt_password`.
- **TUI Bridge Path (`auth_sender: Some(&SshAuthSender)`)**: Non-blocking asynchronous bridge connecting the background SSH task to the TUI event loop:
  1. Spawns a `tokio::sync::oneshot::channel::<String>()`.
  2. Sends `SshAuthRequest { prompt, responder: tx }` over an unbounded `mpsc` channel to the TUI main loop.
  3. The TUI displays a modal input popup with masked text entry.
  4. On Enter, the TUI sends the entered credential string across the oneshot channel to unblock the SSH authentication task. If the user cancels (Escape), the responder is dropped, surfacing an error to the auth task.
  5. Architecture and lifecycle decisions are documented in [ADR 0001: SSH Auth TUI Popup](../adr/0001-ssh-auth-tui-popup.md).

---

## Remote Shell Detection

Remote shell detection (`src/host/shell.rs::detect_russh`) executes probe commands over the established session to determine the remote OS and shell type:

1. **PowerShell**: Runs `$PSVersionTable.PSVersion.Major`. If stdout is non-empty and exit status is successful, returns `ShellType::PowerShell`.
2. **Cmd (Windows)**: Runs `ver`. If stdout contains `"Windows"` and exit status is successful, returns `ShellType::Cmd`.
3. **POSIX Sh**: Runs `echo ok`. If exit status is successful, returns `ShellType::Sh`.
4. **Fallback**: If all command attempts return errors, bails. If commands succeed without matching specific markers, defaults to `ShellType::Sh`.

The detected `ShellType` controls temporary directory paths (`temp_dir`: `/tmp`, `$env:TEMP`, `%TEMP%`) and privilege elevation wrapping (`sudo_wrap`: `sudo <cmd>`, `Start-Process powershell -ArgumentList '-Command <cmd>' -Verb RunAs`, `runas /user:Administrator "<cmd>"`).

---

## Pure-Rust russh vs Subprocesses

`sshi` uses pure-Rust `russh` / `russh-sftp` for all operational workflows while retaining legacy OpenSSH CLI utilities for initial host setup and onboarding:

| Subsystem / Operation | Implementation Type | Component | Description |
|---|---|---|---|
| **Session & Connection Multiplexing** | Pure-Rust (`russh`) | `src/host/session_pool.rs` | TCP connect, session caching, keepalive, and ProxyJump tunnel stream |
| **Server Host Key Verification** | Pure-Rust (`russh_keys`) | `src/host/session_pool.rs` | `known_hosts` checking and MITM mismatch detection |
| **Authentication Chain** | Pure-Rust (`russh`, `russh_keys`) | `src/host/auth.rs` | Public key, passphrase caching, and password fallback |
| **Command Execution (`check`, `run`, `exec`)** | Pure-Rust (`russh`) | `src/host/session_pool.rs` | Channel session open, command exec, stdout/stderr/exit code capture |
| **File Transfer (`sync`, `cp`)** | Pure-Rust (`russh-sftp`) | `src/host/sftp.rs` | SFTP subsystem session, directory creation, streaming file I/O |
| **Shell Detection** | Pure-Rust (`russh`) | `src/host/shell.rs` | Remote probe execution via session channel |
| **Host Key Discovery (`init`)** | System Subprocess (`ssh-keyscan`) | `src/commands/init/core.rs` | Executes `ssh-keyscan -p <port> <host>` to discover and append unhashed keys to `known_hosts` |
| **SSH Key Generation (`init`)** | System Subprocess (`ssh-keygen`) | `src/commands/init/mod.rs` | Runs interactive `ssh-keygen -t ed25519` when no default private key is found |
| **Public Key Deployment (`init`)** | System Subprocess (`ssh-copy-id`) | `src/commands/init/mod.rs` | Runs interactive `ssh-copy-id <host>` to install public keys on auth-failed hosts |
