//! Russh-based session pool with concurrent connect, auth, and SFTP support.

use std::collections::HashMap;
use std::future::Future;
use std::net::ToSocketAddrs;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use russh::client::{self, Handle};
use russh_keys::key::PublicKey;
use russh_sftp::client::SftpSession;

use super::auth::{authenticate, PassphraseCache, SshAuthSender};
use crate::config::schema::HostEntry;
use crate::config::ssh_config::ResolvedHostConfig;

/// russh client handler: verifies server host keys against ~/.ssh/known_hosts.
pub struct SshHandler {
    /// Hostname used for known_hosts lookup (may differ from the ssh alias)
    pub hostname: String,
    pub port: u16,
}

impl client::Handler for SshHandler {
    type Error = anyhow::Error;

    #[allow(clippy::manual_async_fn)]
    fn check_server_key<'life0, 'life1, 'async_trait>(
        &'life0 mut self,
        server_public_key: &'life1 PublicKey,
    ) -> ::core::pin::Pin<
        Box<
            dyn ::core::future::Future<Output = Result<bool, Self::Error>>
                + ::core::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            let known_hosts_path = dirs::home_dir()
                .context("Cannot determine home directory")?
                .join(".ssh")
                .join("known_hosts");

            if !known_hosts_path.exists() {
                bail!(
                    "Unknown host key for {}:{} — run `sshi init` to add the host to known_hosts",
                    self.hostname,
                    self.port
                );
            }

            match russh_keys::check_known_hosts_path(
                &self.hostname,
                self.port,
                server_public_key,
                &known_hosts_path,
            ) {
                Ok(true) => Ok(true),
                Ok(false) => bail!(
                    "Unknown host key for {}:{} — run `sshi init` to accept the key first",
                    self.hostname,
                    self.port
                ),
                Err(russh_keys::Error::KeyChanged { line }) => bail!(
                    "HOST KEY MISMATCH for {}:{} at line {} — possible man-in-the-middle attack",
                    self.hostname,
                    self.port,
                    line
                ),
                Err(e) => Err(e.into()),
            }
        })
    }
}

/// Result of a remote command execution via a russh channel.
#[derive(Debug, Clone)]
pub struct RemoteOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
}

/// Per-host cache of an expensive-to-open resource (e.g. `SftpSession`).
///
/// `get_or_try_insert_with` performs double-checked locking: the mutex is
/// released across the opener's `await` so concurrent opens for different
/// hosts don't serialize. On a race (two opens for the same key), the
/// loser's value is dropped and the existing entry is returned.
struct LazyCache<V> {
    inner: tokio::sync::Mutex<HashMap<String, Arc<V>>>,
}

impl<V> LazyCache<V> {
    fn new() -> Self {
        Self {
            inner: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    async fn get_or_try_insert_with<F, Fut, E>(&self, key: &str, opener: F) -> Result<Arc<V>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<V, E>>,
    {
        {
            let guard = self.inner.lock().await;
            if let Some(v) = guard.get(key) {
                return Ok(Arc::clone(v));
            }
        }
        let value = Arc::new(opener().await?);
        let mut guard = self.inner.lock().await;
        if let Some(existing) = guard.get(key) {
            return Ok(Arc::clone(existing));
        }
        guard.insert(key.to_string(), Arc::clone(&value));
        Ok(value)
    }

    /// Seed the cache with an already-open value (used after the SFTP probe
    /// successfully opens a session that subsequent uploads can re-use).
    async fn insert(&self, key: String, value: Arc<V>) {
        self.inner.lock().await.insert(key, value);
    }
}

/// Pool of authenticated russh sessions, one per host alias.
pub struct RusshSessionPool {
    /// host alias → open authenticated session handle
    sessions: HashMap<String, Arc<Handle<SshHandler>>>,
    /// hosts that failed to connect: (alias, error message)
    failed: Vec<(String, String)>,
    /// hosts that failed the SFTP probe: (name, error message)
    sftp_failed: Vec<(String, String)>,
    /// cached remote home directories: host alias → home path
    home_dirs: tokio::sync::Mutex<HashMap<String, String>>,
    /// cached `SftpSession` per host alias, opened on first use and re-used
    /// for subsequent `upload`/`download` calls. The cache lives for the
    /// pool's lifetime; sessions are torn down together in `shutdown`.
    sftp_cache: LazyCache<SftpSession>,
    /// cancel senders for proxy keepalive tasks (one per proxied connection)
    proxy_cancels: Vec<tokio::sync::oneshot::Sender<()>>,
}

impl RusshSessionPool {
    /// Connect to all hosts concurrently; unreachable hosts are recorded in `failed`.
    ///
    /// `auth_sender`, when `Some`, routes SSH credential prompts (passphrase
    /// / password fallback) through the TUI auth bridge instead of blocking
    /// `rpassword`. The CLI passes `None`.
    pub async fn setup(
        hosts: &[&HostEntry],
        timeout_secs: u64,
        concurrency: usize,
        auth_sender: Option<SshAuthSender>,
    ) -> Result<Self> {
        let timeout = Duration::from_secs(timeout_secs);
        let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));
        let ssh_config = Arc::new(crate::config::ssh_config::load_ssh_config()?);
        let mut handles = Vec::new();

        for host in hosts {
            let alias = host.ssh_host.clone();
            let sem = sem.clone();
            let config = ssh_config.clone();
            let auth_sender = auth_sender.clone();

            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire_owned().await.unwrap();
                let mut cache = PassphraseCache::new();
                let result =
                    connect_one(&alias, timeout, &mut cache, &config, auth_sender.as_ref()).await;
                (alias, result)
            }));
        }

        let mut sessions: HashMap<String, Arc<Handle<SshHandler>>> = HashMap::new();
        let mut failed: Vec<(String, String)> = Vec::new();
        let mut proxy_cancels: Vec<tokio::sync::oneshot::Sender<()>> = Vec::new();

        for jh in handles {
            let (alias, result) = jh.await.context("task panic")?;
            match result {
                Ok((handle, cancel_tx)) => {
                    sessions.insert(alias, Arc::new(handle));
                    if let Some(tx) = cancel_tx {
                        proxy_cancels.push(tx);
                    }
                }
                Err(e) => {
                    // Use the alternate formatter to surface the full anyhow
                    // cause chain (e.g. host-key rejection behind "Failed to
                    // connect"), not just the outermost context.
                    failed.push((alias, format!("{:#}", e)));
                }
            }
        }

        Ok(Self {
            sessions,
            failed,
            sftp_failed: Vec::new(),
            home_dirs: tokio::sync::Mutex::new(HashMap::new()),
            sftp_cache: LazyCache::new(),
            proxy_cancels,
        })
    }

    /// Names of hosts that failed to connect (with error messages).
    pub fn failed_hosts(&self) -> Vec<(String, String)> {
        self.failed.clone()
    }

    /// Names of all successfully connected hosts.
    pub fn reachable_hosts(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    /// Execute a command on a connected host.
    pub async fn exec(
        &self,
        host_alias: &str,
        cmd: &str,
        timeout_secs: u64,
    ) -> Result<RemoteOutput> {
        let handle = self
            .sessions
            .get(host_alias)
            .ok_or_else(|| anyhow::anyhow!("Host '{}' is not connected", host_alias))?
            .clone();

        exec_on_handle(&handle, cmd, Duration::from_secs(timeout_secs)).await
    }

    /// Get the remote home directory for a host, caching the result.
    async fn home_dir(
        &self,
        host_alias: &str,
        shell: crate::config::schema::ShellType,
        timeout: Duration,
    ) -> Result<String> {
        {
            let cache = self.home_dirs.lock().await;
            if let Some(home) = cache.get(host_alias) {
                return Ok(home.clone());
            }
        }
        let handle = self
            .sessions
            .get(host_alias)
            .ok_or_else(|| anyhow::anyhow!("Host '{}' not connected", host_alias))?
            .clone();
        let home = crate::host::sftp::remote_home_dir(&handle, shell, timeout).await?;
        self.home_dirs
            .lock()
            .await
            .insert(host_alias.to_string(), home.clone());
        Ok(home)
    }

    /// Get a cached `SftpSession` for `host_alias`, opening one on first use.
    /// Subsequent calls return the cached channel, avoiding a fresh SFTP
    /// subsystem negotiation per file transfer.
    async fn sftp_session(&self, host_alias: &str) -> Result<Arc<SftpSession>> {
        let handle = self
            .sessions
            .get(host_alias)
            .ok_or_else(|| anyhow::anyhow!("Host '{}' is not connected", host_alias))?
            .clone();
        let alias = host_alias.to_string();
        self.sftp_cache
            .get_or_try_insert_with(host_alias, move || {
                let alias = alias.clone();
                async move {
                    tracing::debug!("Opening SFTP channel for {}", alias);
                    crate::host::sftp::open_sftp(&handle).await
                }
            })
            .await
    }

    /// Upload a local file to a remote host via SFTP.
    pub async fn upload(
        &self,
        host: &crate::config::schema::HostEntry,
        local_path: &std::path::Path,
        remote_path: &str,
        timeout_secs: u64,
    ) -> Result<()> {
        let sftp = self.sftp_session(&host.ssh_host).await?;
        let home = self
            .home_dir(
                &host.ssh_host,
                host.shell,
                Duration::from_secs(timeout_secs),
            )
            .await?;
        crate::host::sftp::upload(
            &sftp,
            local_path,
            remote_path,
            &home,
            Duration::from_secs(timeout_secs),
        )
        .await
    }

    /// Download a remote file via SFTP.
    pub async fn download(
        &self,
        host: &crate::config::schema::HostEntry,
        remote_path: &str,
        local_path: &std::path::Path,
        timeout_secs: u64,
    ) -> Result<()> {
        let sftp = self.sftp_session(&host.ssh_host).await?;
        let home = self
            .home_dir(
                &host.ssh_host,
                host.shell,
                Duration::from_secs(timeout_secs),
            )
            .await?;
        crate::host::sftp::download(
            &sftp,
            remote_path,
            local_path,
            &home,
            Duration::from_secs(timeout_secs),
        )
        .await
    }

    /// Run SFTP probe on all connected hosts. Records failures in `sftp_failed`.
    /// Successfully probed hosts have their open SFTP channel cached so the
    /// subsequent `upload`/`download` reuses it.
    pub async fn run_sftp_probe(
        &mut self,
        hosts: &[&crate::config::schema::HostEntry],
        timeout_secs: u64,
    ) {
        let timeout = Duration::from_secs(timeout_secs);

        let tasks: Vec<_> = hosts
            .iter()
            .filter_map(|host| {
                self.sessions
                    .get(&host.ssh_host)
                    .map(|h| (host.ssh_host.clone(), host.shell, Arc::clone(h)))
            })
            .collect();

        let mut set = tokio::task::JoinSet::new();
        for (ssh_host, shell, handle) in tasks {
            set.spawn(async move {
                let home = crate::host::sftp::remote_home_dir(&handle, shell, timeout).await;
                match home {
                    Err(e) => (ssh_host, None, None, Some(format!("home dir: {:#}", e))),
                    Ok(home_dir) => match crate::host::sftp::open_sftp(&handle).await {
                        Err(e) => (ssh_host, Some(home_dir), None, Some(format!("{:#}", e))),
                        Ok(sftp) => {
                            match crate::host::sftp::sftp_probe(&sftp, &home_dir, timeout).await {
                                Ok(()) => (ssh_host, Some(home_dir), Some(Arc::new(sftp)), None),
                                Err(e) => {
                                    (ssh_host, Some(home_dir), None, Some(format!("{:#}", e)))
                                }
                            }
                        }
                    },
                }
            });
        }

        while let Some(result) = set.join_next().await {
            match result {
                Ok((ssh_host, home_dir, sftp, failure)) => {
                    if let Some(home) = home_dir {
                        self.home_dirs.lock().await.insert(ssh_host.clone(), home);
                    }
                    if let Some(s) = sftp {
                        self.sftp_cache.insert(ssh_host.clone(), s).await;
                    }
                    if let Some(err) = failure {
                        self.sftp_failed.push((ssh_host, err));
                    }
                }
                Err(e) => {
                    tracing::warn!("SFTP probe task panicked: {}", e);
                }
            }
        }
    }

    /// Names and errors of hosts that failed the SFTP probe.
    pub fn sftp_failed_hosts(&self) -> Vec<(String, String)> {
        self.sftp_failed.clone()
    }

    /// Hosts that passed the SFTP probe (by session key / alias).
    pub fn sftp_capable_hosts(&self) -> Vec<String> {
        let failed: std::collections::HashSet<&str> =
            self.sftp_failed.iter().map(|(n, _)| n.as_str()).collect();
        self.sessions
            .keys()
            .filter(|n| !failed.contains(n.as_str()))
            .cloned()
            .collect()
    }

    /// Close all sessions gracefully.
    pub async fn shutdown(self) {
        for tx in self.proxy_cancels {
            let _ = tx.send(());
        }
        for (_, handle) in self.sessions {
            let _ = handle
                .disconnect(russh::Disconnect::ByApplication, "", "en")
                .await;
        }
    }
}

/// Open and authenticate a single session to `alias`.
/// Resolves the alias via the provided SshConfig and handles a single ProxyJump hop.
async fn connect_one(
    alias: &str,
    timeout: Duration,
    cache: &mut PassphraseCache,
    ssh_config: &crate::config::ssh_config::ParsedSshConfig,
    auth_sender: Option<&SshAuthSender>,
) -> Result<(Handle<SshHandler>, Option<tokio::sync::oneshot::Sender<()>>)> {
    let resolved = crate::config::ssh_config::resolve_host_with_config(alias, ssh_config)?;

    match &resolved.proxy_jump.clone() {
        Some(proxy_alias) => {
            let proxy_resolved =
                crate::config::ssh_config::resolve_host_with_config(proxy_alias, ssh_config)?;
            let (handle, cancel_tx) =
                connect_via_proxy(&proxy_resolved, &resolved, timeout, cache, auth_sender).await?;
            Ok((handle, Some(cancel_tx)))
        }
        None => {
            let handle = connect_direct(&resolved, timeout, cache, auth_sender).await?;
            Ok((handle, None))
        }
    }
}

/// Open a direct TCP connection to `config.hostname:config.port` and authenticate.
async fn connect_direct(
    config: &ResolvedHostConfig,
    timeout: Duration,
    cache: &mut PassphraseCache,
    auth_sender: Option<&SshAuthSender>,
) -> Result<Handle<SshHandler>> {
    let russh_config = Arc::new(client::Config {
        // Send keepalive packets every 30s of inactivity so aggressive
        // `ClientAliveInterval` / NAT idle timeouts don't silently drop the
        // session between setup and use. `inactivity_timeout` stays `None`
        // (we never want to proactively close an idle session); per-op
        // timeouts are still enforced by callers via `tokio::time::timeout`.
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        ..<client::Config as Default>::default()
    });

    let handler = SshHandler {
        hostname: config.hostname.clone(),
        port: config.port,
    };

    let addr = format!("{}:{}", config.hostname, config.port);
    let addr = addr
        .to_socket_addrs()
        .with_context(|| format!("Cannot resolve {}", addr))?
        .next()
        .with_context(|| format!("No address resolved for {}", addr))?;

    let mut handle = tokio::time::timeout(timeout, client::connect(russh_config, addr, handler))
        .await
        .context("SSH connect timeout")?
        .with_context(|| format!("Failed to connect to {}:{}", config.hostname, config.port))?;

    authenticate(
        &mut handle,
        &config.user,
        &config.alias,
        &config.identity_files,
        config.identities_only,
        cache,
        auth_sender,
    )
    .await
    .with_context(|| {
        format!(
            "Authentication failed for {}@{}:{}",
            config.user, config.hostname, config.port
        )
    })?;

    Ok(handle)
}

/// Open an SSH session through a jump host (ProxyJump, single hop).
async fn connect_via_proxy(
    proxy: &ResolvedHostConfig,
    target: &ResolvedHostConfig,
    timeout: Duration,
    cache: &mut PassphraseCache,
    auth_sender: Option<&SshAuthSender>,
) -> Result<(Handle<SshHandler>, tokio::sync::oneshot::Sender<()>)> {
    // Step 1: connect and authenticate to the proxy
    let proxy_handle = connect_direct(proxy, timeout, cache, auth_sender)
        .await
        .with_context(|| format!("Failed to connect to proxy {}", proxy.alias))?;

    // Step 2: open a direct-tcpip tunnel through the proxy to the target
    let channel = tokio::time::timeout(
        timeout,
        proxy_handle.channel_open_direct_tcpip(
            target.hostname.as_str(),
            target.port as u32,
            "127.0.0.1",
            0u32,
        ),
    )
    .await
    .context("Proxy channel open timeout")?
    .with_context(|| {
        format!(
            "Failed to open direct-tcpip channel to {}:{} via {}",
            target.hostname, target.port, proxy.alias
        )
    })?;

    // Step 3: establish a second SSH session over the channel stream.
    // Keep proxy_handle alive in a background task until shutdown() cancels it.
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let _keep_alive = proxy_handle;
        let _ = cancel_rx.await;
    });

    let russh_config = Arc::new(client::Config {
        // Same keepalive policy as the direct connection — the tunneled
        // session is equally susceptible to idle-timeout disconnects.
        inactivity_timeout: None,
        keepalive_interval: Some(Duration::from_secs(30)),
        ..<client::Config as Default>::default()
    });

    let handler = SshHandler {
        hostname: target.hostname.clone(),
        port: target.port,
    };

    let mut target_handle = tokio::time::timeout(
        timeout,
        client::connect_stream(russh_config, channel.into_stream(), handler),
    )
    .await
    .context("SSH-through-proxy connect timeout")?
    .context("Failed to establish SSH session through proxy")?;

    authenticate(
        &mut target_handle,
        &target.user,
        &target.alias,
        &target.identity_files,
        target.identities_only,
        cache,
        auth_sender,
    )
    .await
    .with_context(|| {
        format!(
            "Authentication failed for {}@{} (via proxy {})",
            target.user, target.alias, proxy.alias
        )
    })?;

    Ok((target_handle, cancel_tx))
}

/// Execute a command on an open session handle and collect stdout/stderr/exit code.
pub async fn exec_on_handle(
    handle: &Handle<SshHandler>,
    cmd: &str,
    timeout: Duration,
) -> Result<RemoteOutput> {
    tokio::time::timeout(timeout, async {
        let mut channel = handle
            .channel_open_session()
            .await
            .context("Failed to open SSH channel")?;

        channel
            .exec(true, cmd)
            .await
            .context("Failed to exec command")?;

        let mut stdout: Vec<u8> = Vec::new();
        let mut stderr: Vec<u8> = Vec::new();
        let mut exit_code: Option<u32> = None;

        loop {
            match channel.wait().await {
                Some(russh::ChannelMsg::Data { data }) => stdout.extend_from_slice(&data),
                Some(russh::ChannelMsg::ExtendedData { data, ext: 1 }) => {
                    stderr.extend_from_slice(&data);
                }
                Some(russh::ChannelMsg::ExitStatus { exit_status }) => {
                    exit_code = Some(exit_status);
                }
                Some(russh::ChannelMsg::Eof) | Some(russh::ChannelMsg::Close) => {}
                None => break,
                _ => {}
            }
        }

        let exit_code = exit_code.map(|c| c as i32);
        let success = exit_code == Some(0);

        Ok::<RemoteOutput, anyhow::Error>(RemoteOutput {
            stdout: String::from_utf8_lossy(&stdout).to_string(),
            stderr: String::from_utf8_lossy(&stderr).to_string(),
            exit_code,
            success,
        })
    })
    .await
    .context("Command execution timeout")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_remote_output_success_flag() {
        let out = RemoteOutput {
            stdout: "hello\n".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            success: true,
        };
        assert!(out.success);
        assert_eq!(out.stdout.trim(), "hello");
    }

    #[test]
    fn test_remote_output_failure_flag() {
        let out = RemoteOutput {
            stdout: String::new(),
            stderr: "not found\n".to_string(),
            exit_code: Some(127),
            success: false,
        };
        assert!(!out.success);
        assert_eq!(out.exit_code, Some(127));
    }

    #[test]
    fn test_remote_output_no_exit_code() {
        let out = RemoteOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            success: false,
        };
        assert!(!out.success);
        assert!(out.exit_code.is_none());
    }

    #[test]
    fn test_remote_output_debug_clone() {
        let out = RemoteOutput {
            stdout: "data".to_string(),
            stderr: "err".to_string(),
            exit_code: Some(1),
            success: false,
        };
        let cloned = out.clone();
        assert_eq!(cloned.stdout, "data");
        assert_eq!(cloned.stderr, "err");
        let debug_str = format!("{:?}", out);
        assert!(debug_str.contains("data"));
    }

    #[test]
    fn test_ssh_handler_fields() {
        let handler = SshHandler {
            hostname: "example.com".to_string(),
            port: 2222,
        };
        assert_eq!(handler.hostname, "example.com");
        assert_eq!(handler.port, 2222);
    }

    #[test]
    fn test_ssh_handler_with_default_port() {
        let handler = SshHandler {
            hostname: "myhost".to_string(),
            port: 22,
        };
        assert_eq!(handler.port, 22);
    }

    #[test]
    fn test_sftp_capable_excludes_failed_from_all_sessions() {
        let session_keys: Vec<String> = vec!["alpha".into(), "beta".into(), "gamma".into()];
        let sftp_failed: Vec<(String, String)> =
            vec![("beta".into(), "sftp subsystem not found".into())];
        let failed_set: std::collections::HashSet<&str> =
            sftp_failed.iter().map(|(n, _)| n.as_str()).collect();
        let capable: Vec<String> = session_keys
            .iter()
            .filter(|n| !failed_set.contains(n.as_str()))
            .cloned()
            .collect();
        assert_eq!(capable.len(), 2);
        assert!(capable.contains(&String::from("alpha")));
        assert!(capable.contains(&String::from("gamma")));
        assert!(!capable.contains(&String::from("beta")));
    }

    #[test]
    fn test_sftp_capable_empty_failed_list() {
        let session_keys: Vec<String> = vec!["x".into(), "y".into()];
        let sftp_failed: Vec<(String, String)> = Vec::new();
        let failed_set: std::collections::HashSet<&str> =
            sftp_failed.iter().map(|(n, _)| n.as_str()).collect();
        let capable: Vec<String> = session_keys
            .iter()
            .filter(|n| !failed_set.contains(n.as_str()))
            .cloned()
            .collect();
        assert_eq!(capable.len(), 2);
    }

    #[test]
    fn test_sftp_capable_all_failed() {
        let session_keys: Vec<String> = vec!["only-one".into()];
        let sftp_failed: Vec<(String, String)> = vec![("only-one".into(), "error".into())];
        let failed_set: std::collections::HashSet<&str> =
            sftp_failed.iter().map(|(n, _)| n.as_str()).collect();
        let capable: Vec<String> = session_keys
            .iter()
            .filter(|n| !failed_set.contains(n.as_str()))
            .cloned()
            .collect();
        assert!(capable.is_empty());
    }

    #[test]
    fn test_proxy_alias_resolved_from_config() {
        let resolved = crate::config::ssh_config::resolve_host("nonexistent-xyz-direct");
        let r = resolved.unwrap();
        assert!(
            r.proxy_jump.is_none(),
            "Host not in config should have no ProxyJump"
        );
    }

    #[tokio::test]
    async fn test_lazy_cache_reuses_value_for_same_key() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let cache: LazyCache<u32> = LazyCache::new();
        let counter = Arc::new(AtomicUsize::new(0));

        let c = Arc::clone(&counter);
        let v1 = cache
            .get_or_try_insert_with("host-a", move || {
                let c = Arc::clone(&c);
                async move {
                    let n = c.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, std::convert::Infallible>(n as u32 + 100)
                }
            })
            .await
            .expect("first open");

        let c = Arc::clone(&counter);
        let v2 = cache
            .get_or_try_insert_with("host-a", move || {
                let c = Arc::clone(&c);
                async move {
                    let n = c.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, std::convert::Infallible>(n as u32 + 100)
                }
            })
            .await
            .expect("cache hit");

        // Same Arc contents returned both times; opener ran exactly once.
        assert_eq!(*v1, 100);
        assert_eq!(*v2, 100);
        assert!(Arc::ptr_eq(&v1, &v2));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_lazy_cache_distinct_keys_get_distinct_values() {
        let cache: LazyCache<String> = LazyCache::new();

        let a = cache
            .get_or_try_insert_with("a", || async {
                Ok::<_, std::convert::Infallible>("A".to_string())
            })
            .await
            .unwrap();
        let b = cache
            .get_or_try_insert_with("b", || async {
                Ok::<_, std::convert::Infallible>("B".to_string())
            })
            .await
            .unwrap();

        assert_eq!(a.as_str(), "A");
        assert_eq!(b.as_str(), "B");
        assert_eq!(cache.inner.lock().await.len(), 2);
    }

    #[tokio::test]
    async fn test_lazy_cache_propagates_opener_error() {
        let cache: LazyCache<u32> = LazyCache::new();

        let err: Result<Arc<u32>, &'static str> = cache
            .get_or_try_insert_with("missing", || async { Err("boom") })
            .await;

        assert!(err.is_err());
        assert_eq!(err.unwrap_err(), "boom");
        assert!(cache.inner.lock().await.is_empty());
    }

    #[test]
    fn test_keepalive_config_sends_packets_every_30s() {
        let config = client::Config {
            inactivity_timeout: None,
            keepalive_interval: Some(Duration::from_secs(30)),
            ..<client::Config as Default>::default()
        };
        assert_eq!(
            config.keepalive_interval,
            Some(Duration::from_secs(30)),
            "keepalive_interval must be set so aggressive ClientAliveInterval hosts stay connected"
        );
        assert_eq!(
            config.inactivity_timeout, None,
            "inactivity_timeout must remain None — we never proactively close idle sessions"
        );
    }
}
