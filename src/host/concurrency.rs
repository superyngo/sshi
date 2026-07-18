//! Semaphore-based concurrency control for parallel host operations.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Semaphore;

/// Dual-level concurrency limiter: global cap + per-host cap.
/// Acquire both permits before any SSH/SCP operation.
pub struct ConcurrencyLimiter {
    global: Arc<Semaphore>,
    per_host: HashMap<String, Arc<Semaphore>>,
}

impl ConcurrencyLimiter {
    /// Create a new limiter with global and per-host caps.
    /// `hosts` is the list of host names that will be used.
    pub fn new(global_limit: usize, per_host_limit: usize, hosts: &[String]) -> Self {
        let mut per_host = HashMap::new();
        for host in hosts {
            per_host.insert(host.clone(), Arc::new(Semaphore::new(per_host_limit)));
        }
        Self {
            global: Arc::new(Semaphore::new(global_limit)),
            per_host,
        }
    }

    /// Acquire both per-host and global permits.
    /// Order: per-host first, then global. Per-host-first avoids head-of-line
    /// blocking — a task waiting on a saturated host does not hold a global
    /// permit, so an independent host can still acquire its per-host slot and
    /// the global slot in parallel. Order is consistent across all callers,
    /// so there is no deadlock risk.
    /// Returns a guard that releases both permits on drop.
    pub async fn acquire(&self, host: &str) -> ConcurrencyPermit {
        let per_host_sem = self
            .per_host
            .get(host)
            .expect("host not registered in limiter");
        let per_host_permit = per_host_sem.clone().acquire_owned().await.unwrap();
        let global_permit = self.global.clone().acquire_owned().await.unwrap();
        ConcurrencyPermit {
            _global: global_permit,
            _per_host: per_host_permit,
        }
    }

    /// Get a clone of the global semaphore (for use in spawned tasks).
    pub fn global_semaphore(&self) -> Arc<Semaphore> {
        self.global.clone()
    }

    /// Get a clone of a per-host semaphore (for use in spawned tasks).
    pub fn per_host_semaphore(&self, host: &str) -> Option<Arc<Semaphore>> {
        self.per_host.get(host).cloned()
    }
}

/// RAII guard that holds both global and per-host semaphore permits.
pub struct ConcurrencyPermit {
    _global: tokio::sync::OwnedSemaphorePermit,
    _per_host: tokio::sync::OwnedSemaphorePermit,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Duration;

    #[tokio::test]
    async fn test_global_limit_respected() {
        let hosts = vec!["a".into(), "b".into(), "c".into()];
        let limiter = ConcurrencyLimiter::new(2, 10, &hosts);

        let _p1 = limiter.acquire("a").await;
        let _p2 = limiter.acquire("b").await;

        let result = tokio::time::timeout(Duration::from_millis(50), limiter.acquire("c")).await;
        assert!(
            result.is_err(),
            "Third acquire should block when global limit is 2"
        );
    }

    #[tokio::test]
    async fn test_per_host_limit_respected() {
        let hosts = vec!["a".into()];
        let limiter = ConcurrencyLimiter::new(10, 2, &hosts);

        let _p1 = limiter.acquire("a").await;
        let _p2 = limiter.acquire("a").await;

        let result = tokio::time::timeout(Duration::from_millis(50), limiter.acquire("a")).await;
        assert!(
            result.is_err(),
            "Third acquire on same host should block when per-host limit is 2"
        );
    }

    #[tokio::test]
    async fn test_permits_released_on_drop() {
        let hosts = vec!["a".into()];
        let limiter = ConcurrencyLimiter::new(1, 1, &hosts);

        {
            let _p = limiter.acquire("a").await;
        }
        let result = tokio::time::timeout(Duration::from_millis(50), limiter.acquire("a")).await;
        assert!(
            result.is_ok(),
            "Should acquire after previous permit dropped"
        );
    }

    #[tokio::test]
    async fn test_global_semaphore_accessor() {
        let hosts = vec!["x".into()];
        let limiter = ConcurrencyLimiter::new(5, 10, &hosts);
        let sem = limiter.global_semaphore();
        let permits = sem.available_permits();
        assert_eq!(permits, 5);
    }

    #[tokio::test]
    async fn test_per_host_semaphore_accessor() {
        let hosts = vec!["h1".into(), "h2".into()];
        let limiter = ConcurrencyLimiter::new(10, 3, &hosts);
        let sem = limiter.per_host_semaphore("h1").unwrap();
        assert_eq!(sem.available_permits(), 3);
    }

    #[tokio::test]
    async fn test_per_host_semaphore_unknown_host() {
        let hosts = vec!["h1".into()];
        let limiter = ConcurrencyLimiter::new(10, 3, &hosts);
        assert!(limiter.per_host_semaphore("unknown").is_none());
    }

    #[tokio::test]
    async fn test_concurrency_limiter_no_hosts() {
        let limiter = ConcurrencyLimiter::new(2, 5, &[]);
        let sem = limiter.global_semaphore();
        assert_eq!(sem.available_permits(), 2);
    }

    #[tokio::test]
    async fn test_independent_host_not_blocked_by_separate_host_queue() {
        // Head-of-line regression: 10 long-running tasks targeting host A
        // (per-host limit 1, so 9 queue on A's semaphore) plus 1 task
        // targeting host B must start B within 1s. With global-first acquire
        // the 9 queued A tasks would each hold a global permit, starving B.
        let hosts = vec!["a".into(), "b".into()];
        let limiter = Arc::new(ConcurrencyLimiter::new(10, 1, &hosts));

        let mut handles = Vec::new();
        for _ in 0..10 {
            let l = limiter.clone();
            handles.push(tokio::spawn(async move {
                let _p = l.acquire("a").await;
                tokio::time::sleep(Duration::from_secs(30)).await;
            }));
        }

        // Let the A tasks grab their permits and queue on host A's semaphore.
        tokio::time::sleep(Duration::from_millis(150)).await;

        let l = limiter.clone();
        let b_result = tokio::time::timeout(Duration::from_secs(1), async move {
            let _p = l.acquire("b").await;
        })
        .await;

        for h in handles {
            h.abort();
        }

        assert!(
            b_result.is_ok(),
            "host B's task should start within 1s of spawn; head-of-line blocking detected",
        );
    }
}
