use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Semaphore;

use crate::config::schema::HostEntry;
use crate::host::concurrency::ConcurrencyLimiter;
use crate::host::session_pool::RusshSessionPool;

use super::types::SyncDecision;

pub(crate) async fn distribute(
    hosts: &[&HostEntry],
    decision: &SyncDecision,
    timeout: u64,
    concurrency: usize,
    sessions: Arc<RusshSessionPool>,
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
    let mut handles = Vec::new();

    for target_name in &decision.target_hosts {
        let target = hosts
            .iter()
            .find(|h| h.name == *target_name)
            .ok_or_else(|| anyhow::anyhow!("Target host not found: {}", target_name))?;

        let sem = semaphore.clone();
        let target = (*target).clone();
        let local_temp = local_temp.clone();
        let remote_path = decision.path.clone();
        let target_name = target_name.clone();
        let sessions = Arc::clone(&sessions);

        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.unwrap();

            let result = sessions
                .upload(&target, &local_temp, &remote_path, timeout)
                .await;
            (target_name, result)
        }));
    }

    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    for handle in handles {
        let (target_name, result) = handle.await?;
        match result {
            Ok(()) => succeeded.push(target_name),
            Err(e) => failed.push((target_name, e.to_string())),
        }
    }

    Ok((succeeded, failed))
}

pub(crate) async fn distribute_pooled(
    hosts: &[&HostEntry],
    decision: &SyncDecision,
    timeout: u64,
    limiter: &ConcurrencyLimiter,
    sessions: &Arc<RusshSessionPool>,
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

    let mut handles = Vec::new();

    for target_name in &decision.target_hosts {
        let target = hosts
            .iter()
            .find(|h| h.name == *target_name)
            .ok_or_else(|| anyhow::anyhow!("Target host not found: {}", target_name))?;

        let target = (*target).clone();
        let local_temp = local_temp.clone();
        let remote_path = decision.path.clone();
        let target_name = target_name.clone();
        let sessions = Arc::clone(sessions);

        let limiter_global = limiter.global_semaphore();
        let limiter_per_host = limiter
            .per_host_semaphore(&target.name)
            .expect("target not registered");

        handles.push(tokio::spawn(async move {
            let _global_permit = limiter_global.acquire().await.unwrap();
            let _per_host_permit = limiter_per_host.acquire().await.unwrap();

            let result = sessions
                .upload(&target, &local_temp, &remote_path, timeout)
                .await;
            (target_name, result)
        }));
    }

    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    for handle in handles {
        let (target_name, result) = handle.await?;
        match result {
            Ok(()) => {
                succeeded.push(target_name.clone());
            }
            Err(e) => failed.push((target_name, e.to_string())),
        }
    }

    Ok((succeeded, failed))
}
