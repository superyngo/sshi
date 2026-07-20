//! Shared SSH connection pool: wraps `RusshSessionPool` (russh-based) with
//! `ConcurrencyLimiter` and `SyncProgress`. Used by every SSH-using
//! subcommand for consistent connection reuse, concurrency control, and
//! progress reporting.

use std::sync::Arc;

use anyhow::Result;

use crate::config::schema::HostEntry;
use crate::output::progress::SyncProgress;

use super::auth::SshAuthSender;
use super::concurrency::ConcurrencyLimiter;
use super::session_pool::RusshSessionPool;

/// Shared SSH connection pool: wraps RusshSessionPool + ConcurrencyLimiter + SyncProgress.
/// Used by all SSH-using subcommands for consistent connection pooling, concurrency control,
/// and progress display.
pub struct SshPool {
    pub(crate) session_pool: Arc<RusshSessionPool>,
    pub limiter: ConcurrencyLimiter,
    pub progress: SyncProgress,
}

impl SshPool {
    /// Set up the pool: create ControlMaster connections, build ConcurrencyLimiter,
    /// initialize progress bars. Returns (pool, connected_count).
    pub async fn setup(
        hosts: &[Arc<HostEntry>],
        timeout: u64,
        global_concurrency: usize,
        per_host_concurrency: usize,
        auth_sender: Option<SshAuthSender>,
    ) -> Result<(Self, usize)> {
        Self::setup_with_options(
            hosts,
            timeout,
            global_concurrency,
            per_host_concurrency,
            false,
            auth_sender,
        )
        .await
    }

    /// Set up the pool with optional SFTP probe.
    /// When `probe_sftp` is true, reachable hosts are also tested for SFTP capability.
    /// The progress bar reflects both SSH + SFTP checks.
    pub async fn setup_with_options(
        hosts: &[Arc<HostEntry>],
        timeout: u64,
        global_concurrency: usize,
        per_host_concurrency: usize,
        probe_sftp: bool,
        auth_sender: Option<SshAuthSender>,
    ) -> Result<(Self, usize)> {
        let host_names: Vec<String> = hosts.iter().map(|h| h.name.clone()).collect();
        let limiter =
            ConcurrencyLimiter::new(global_concurrency, per_host_concurrency, &host_names);
        let mut progress = SyncProgress::new();

        progress.start_host_check(hosts.len());
        let mut session_pool =
            RusshSessionPool::setup(hosts, timeout, global_concurrency, auth_sender).await?;
        let connected = session_pool.reachable_hosts().len();

        if probe_sftp && connected > 0 {
            session_pool.run_sftp_probe(hosts, timeout).await;
            let sftp_failed = session_pool.sftp_failed_hosts().len();
            let effective_ok = connected.saturating_sub(sftp_failed);
            progress.finish_host_check(effective_ok, hosts.len() - effective_ok);
        } else {
            let failed = hosts.len() - connected;
            progress.finish_host_check(connected, failed);
        }

        Ok((
            Self {
                session_pool: Arc::new(session_pool),
                limiter,
                progress,
            },
            connected,
        ))
    }

    /// Get names and errors of all failed hosts.
    pub fn failed_hosts(&self) -> Vec<(String, String)> {
        self.session_pool.failed_hosts()
    }

    /// Filter a host list to only reachable hosts.
    pub fn filter_reachable(&self, hosts: &[Arc<HostEntry>]) -> Vec<Arc<HostEntry>> {
        let reachable: std::collections::HashSet<String> =
            self.session_pool.reachable_hosts().into_iter().collect();
        hosts
            .iter()
            .filter(|h| reachable.contains(&h.ssh_host))
            .cloned()
            .collect()
    }

    /// Get names and errors of hosts that failed the SFTP probe.
    pub fn sftp_failed_hosts(&self) -> Vec<(String, String)> {
        self.session_pool.sftp_failed_hosts()
    }

    /// Filter a host list to only hosts that passed the SFTP probe.
    pub fn filter_sftp_capable(&self, hosts: &[Arc<HostEntry>]) -> Vec<Arc<HostEntry>> {
        let capable = self.session_pool.sftp_capable_hosts();
        hosts
            .iter()
            .filter(|h| capable.contains(&h.ssh_host))
            .cloned()
            .collect()
    }

    /// Gracefully shut down all sessions and clear progress bars.
    pub async fn shutdown(self) {
        self.progress.clear();
        match Arc::try_unwrap(self.session_pool) {
            Ok(pool) => pool.shutdown().await,
            Err(arc) => {
                tracing::warn!(
                    "session_pool has {} strong references at shutdown; \
                     sessions may not be cleanly closed",
                    Arc::strong_count(&arc)
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::HostEntry;

    /// Smoke test: `SshPool::setup` with an empty host list produces an
    /// empty pool (zero reachable, zero failed) and shuts down cleanly.
    /// Closes the Phase F carry-over where `host::pool` had zero tests
    /// (audit §2.8 MED — `host::pool.rs:23,89 PoolHostResult/reachable_hosts`
    /// was deleted in F1, leaving no test module at all).
    #[tokio::test]
    async fn ssh_pool_setup_empty_hosts_produces_empty_pool() {
        let (pool, connected) = SshPool::setup(&[], 5, 4, 4, None).await.unwrap();
        assert_eq!(connected, 0);
        assert!(pool.failed_hosts().is_empty());
        assert!(pool.sftp_failed_hosts().is_empty());
        assert!(pool.filter_reachable(&[]).is_empty());
        assert!(pool.filter_sftp_capable(&[]).is_empty());
        pool.shutdown().await;
    }

    /// `filter_reachable` / `filter_sftp_capable` on an empty pool return
    /// empty Vecs even when given a non-empty host list (no sessions to
    /// match against).
    #[tokio::test]
    async fn ssh_pool_empty_pool_filters_return_empty() {
        let (pool, _) = SshPool::setup(&[], 5, 4, 4, None).await.unwrap();
        let hosts = vec![
            Arc::new(HostEntry::placeholder("a", "a")),
            Arc::new(HostEntry::placeholder("b", "b")),
        ];
        assert_eq!(pool.filter_reachable(&hosts).len(), 0);
        assert_eq!(pool.filter_sftp_capable(&hosts).len(), 0);
        pool.shutdown().await;
    }
}
