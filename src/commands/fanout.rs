//! Per-host fan-out shared by the `exec`, `run`, `cp` and `check` cores (B53).

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use super::report::ProgressSink;
use crate::config::schema::HostEntry;
use crate::host::pool::SshPool;

/// Concurrent per-host work: each task runs under the pool's global permit,
/// is announced to the progress sink when queued, and is timed. Results come
/// back in completion order so callers can report each host as it finishes.
pub(crate) struct FanOut<T> {
    set: JoinSet<(Arc<HostEntry>, T, Duration)>,
    global: Arc<Semaphore>,
}

impl<T: Send + 'static> FanOut<T> {
    pub(crate) fn new(pool: &SshPool) -> Self {
        Self {
            set: JoinSet::new(),
            global: pool.limiter.global_semaphore(),
        }
    }

    /// Queue `work` for `host`, reporting `host_started` to `progress`.
    pub(crate) fn spawn<F>(
        &mut self,
        host: &Arc<HostEntry>,
        progress: Option<&dyn ProgressSink>,
        work: F,
    ) where
        F: Future<Output = T> + Send + 'static,
    {
        if let Some(p) = progress {
            p.host_started(&host.name);
        }
        let host = Arc::clone(host);
        let global = Arc::clone(&self.global);
        self.set.spawn(async move {
            let _permit = global.acquire_owned().await.expect("semaphore closed");
            let start = Instant::now();
            let out = work.await;
            (host, out, start.elapsed())
        });
    }

    /// The next host to finish: `(host, output, elapsed)`; `None` when all
    /// queued work is done. A panicked task is an error.
    pub(crate) async fn next(&mut self) -> Option<Result<(Arc<HostEntry>, T, Duration)>> {
        let joined = self.set.join_next().await?;
        Some(joined.context("task panic"))
    }
}
