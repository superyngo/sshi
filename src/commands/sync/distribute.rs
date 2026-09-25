use std::sync::Arc;

use anyhow::{Context as _, Result};
use tokio::sync::Semaphore;

use crate::config::schema::HostEntry;
use crate::host::concurrency::ConcurrencyLimiter;
use crate::host::session_pool::SessionPool;

use super::types::SyncDecision;

pub(crate) async fn distribute(
    hosts: &[Arc<HostEntry>],
    decision: &SyncDecision,
    timeout: u64,
    concurrency: usize,
    sessions: Arc<dyn SessionPool>,
) -> Result<(Vec<String>, Vec<(String, String)>)> {
    let source = hosts
        .iter()
        .find(|h| h.name == decision.source_host)
        .ok_or_else(|| anyhow::anyhow!("Source host not found: {}", decision.source_host))?;

    let temp_dir = tempfile::tempdir()?;
    let local_temp = temp_dir.path().join("sshi_relay");
    sessions
        .download(source, &decision.path, &local_temp, timeout)
        .await?;

    let semaphore = Arc::new(Semaphore::new(concurrency));
    let mut set = tokio::task::JoinSet::new();

    for target_name in &decision.target_hosts {
        let target = hosts
            .iter()
            .find(|h| h.name == *target_name)
            .ok_or_else(|| anyhow::anyhow!("Target host not found: {}", target_name))?;

        let sem = semaphore.clone();
        let target = Arc::clone(target);
        let local_temp = local_temp.clone();
        let remote_path = decision.path.clone();
        let target_name = target_name.clone();
        let sessions = Arc::clone(&sessions);

        set.spawn(async move {
            let _permit = sem.acquire().await.unwrap();

            let result = sessions
                .upload(&target, &local_temp, &remote_path, timeout)
                .await;
            (target_name, result)
        });
    }

    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    while let Some(joined) = set.join_next().await {
        let (target_name, result) = joined.context("task panic")?;
        match result {
            Ok(()) => succeeded.push(target_name),
            Err(e) => failed.push((target_name, e.to_string())),
        }
    }

    Ok((succeeded, failed))
}

pub(crate) async fn distribute_pooled(
    hosts: &[Arc<HostEntry>],
    decision: &SyncDecision,
    timeout: u64,
    limiter: &ConcurrencyLimiter,
    sessions: &Arc<dyn SessionPool>,
) -> Result<(Vec<String>, Vec<(String, String)>)> {
    let source = hosts
        .iter()
        .find(|h| h.name == decision.source_host)
        .ok_or_else(|| anyhow::anyhow!("Source host not found: {}", decision.source_host))?;

    let temp_dir = tempfile::tempdir()?;
    let local_temp = temp_dir.path().join("sshi_relay");
    {
        let _permit = limiter.acquire(&source.name).await;
        sessions
            .download(source, &decision.path, &local_temp, timeout)
            .await?;
    }

    let mut set = tokio::task::JoinSet::new();

    for target_name in &decision.target_hosts {
        let target = hosts
            .iter()
            .find(|h| h.name == *target_name)
            .ok_or_else(|| anyhow::anyhow!("Target host not found: {}", target_name))?;

        let target = Arc::clone(target);
        let local_temp = local_temp.clone();
        let remote_path = decision.path.clone();
        let target_name = target_name.clone();
        let sessions = Arc::clone(sessions);

        let limiter = limiter.clone();

        set.spawn(async move {
            let _permit = limiter.acquire(&target_name).await;

            let result = sessions
                .upload(&target, &local_temp, &remote_path, timeout)
                .await;
            (target_name, result)
        });
    }

    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    while let Some(joined) = set.join_next().await {
        let (target_name, result) = joined.context("task panic")?;
        match result {
            Ok(()) => {
                succeeded.push(target_name.clone());
            }
            Err(e) => failed.push((target_name, e.to_string())),
        }
    }

    Ok((succeeded, failed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::session_pool_mock::MockSessionPool;

    fn make_host(name: &str) -> Arc<HostEntry> {
        Arc::new(HostEntry::placeholder(name, name))
    }

    #[tokio::test]
    async fn test_distribute_pooled_acquires_per_host_first() {
        let hosts = vec![make_host("src"), make_host("h1"), make_host("h2")];
        let host_names = vec!["src".to_string(), "h1".to_string(), "h2".to_string()];
        // Global limit = 1, per-host = 1.
        let limiter = ConcurrencyLimiter::new(1, 1, &host_names);

        // Pre-acquire h1 permit to simulate a saturated host.
        let h1_sem = limiter.per_host_semaphore("h1").unwrap();
        let _h1_hold = h1_sem.acquire_owned().await.unwrap();

        let pool = Arc::new(MockSessionPool::new(host_names.clone())) as Arc<dyn SessionPool>;

        // Target h2 has its per-host permit free.
        let decision = SyncDecision {
            path: "/tmp/foo".to_string(),
            source_host: "src".to_string(),
            target_hosts: vec!["h2".to_string()],
            synced_hosts: vec![],
            reason: "test".to_string(),
        };

        let result = distribute_pooled(&hosts, &decision, 5, &limiter, &pool).await;
        assert!(result.is_ok());
        let (succeeded, failed) = result.unwrap();
        assert_eq!(succeeded, vec!["h2".to_string()]);
        assert!(failed.is_empty());
    }

    #[tokio::test]
    async fn test_distribute_pooled_multiple_targets() {
        let hosts = vec![make_host("src"), make_host("h1"), make_host("h2")];
        let host_names = vec!["src".to_string(), "h1".to_string(), "h2".to_string()];
        let limiter = ConcurrencyLimiter::new(2, 2, &host_names);
        let pool = Arc::new(MockSessionPool::new(host_names.clone())) as Arc<dyn SessionPool>;

        let decision = SyncDecision {
            path: "/tmp/bar".to_string(),
            source_host: "src".to_string(),
            target_hosts: vec!["h1".to_string(), "h2".to_string()],
            synced_hosts: vec![],
            reason: "test".to_string(),
        };

        let result = distribute_pooled(&hosts, &decision, 5, &limiter, &pool).await;
        assert!(result.is_ok());
        let (mut succeeded, failed) = result.unwrap();
        succeeded.sort();
        assert_eq!(succeeded, vec!["h1".to_string(), "h2".to_string()]);
        assert!(failed.is_empty());
    }
}
