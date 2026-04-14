//! Exclusive runtime — one caller at a time via semaphore(1).
//!
//! The exclusive runtime holds a shared `Arc<R::Runtime>` behind a
//! binary semaphore. Only one caller can hold the lease at any time.
//! On release, the runtime is reset before the permit is returned.

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::{
    error::Error,
    handle::ResourceHandle,
    metrics::ResourceOpsMetrics,
    options::AcquireOptions,
    release_queue::ReleaseQueue,
    resource::Resource,
    topology::exclusive::{Exclusive, config::Config},
    topology_tag::TopologyTag,
};

/// Runtime state for an exclusive topology.
///
/// Wraps a shared runtime in a binary semaphore, ensuring at most one
/// caller holds the lease at any time.
pub struct ExclusiveRuntime<R: Resource> {
    runtime: Arc<R::Runtime>,
    semaphore: Arc<Semaphore>,
    config: Config,
}

impl<R: Resource> ExclusiveRuntime<R> {
    /// Creates a new exclusive runtime wrapping an existing runtime instance.
    ///
    /// The semaphore is initialized with exactly 1 permit.
    pub fn new(runtime: R::Runtime, config: Config) -> Self {
        Self {
            runtime: Arc::new(runtime),
            semaphore: Arc::new(Semaphore::new(1)),
            config,
        }
    }

    /// Returns the current configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Returns a reference to the underlying runtime.
    pub fn runtime(&self) -> &R::Runtime {
        &self.runtime
    }
}

impl<R> ExclusiveRuntime<R>
where
    R: Exclusive + Clone + Send + Sync + 'static,
    R::Lease: Send + 'static,
    R::Runtime: Clone + Send + Sync + 'static,
{
    /// Acquires exclusive access to the resource.
    ///
    /// 1. Acquires the single semaphore permit.
    /// 2. Returns a shared handle wrapping `Arc<R::Lease>`.
    /// 3. On drop, submits `reset()` + permit release to the [`ReleaseQueue`].
    ///
    /// # Errors
    ///
    /// Returns an error if the semaphore is closed or the acquire times out.
    pub async fn acquire(
        &self,
        resource: &R,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        options: &AcquireOptions,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Result<ResourceHandle<R>, Error>
    where
        R::Runtime: Into<R::Lease>,
    {
        let timeout = options.remaining().unwrap_or(self.config.acquire_timeout);
        let permit =
            match tokio::time::timeout(timeout, self.semaphore.clone().acquire_owned()).await {
                Ok(Ok(permit)) => permit,
                Ok(Err(_)) => return Err(Error::permanent("exclusive semaphore closed")),
                Err(_) => return Err(Error::backpressure("exclusive: timed out waiting for lock")),
            };

        let lease: R::Lease = (*self.runtime).clone().into();
        let runtime = self.runtime.clone();
        let resource_clone = resource.clone();
        let rq = release_queue.clone();

        // #384: the permit must stay alive for the entire `reset()` window,
        // otherwise the next acquirer can enter while the previous reset is
        // still running. We move the permit into the release closure and
        // then into the submitted future, so it is dropped AFTER reset
        // resolves (see `release_exclusive`).
        Ok(ResourceHandle::guarded(
            lease,
            R::key(),
            TopologyTag::Exclusive,
            generation,
            move |_returned_lease, _tainted| {
                if let Some(m) = &metrics {
                    m.record_release();
                }
                rq.submit(move || Box::pin(release_exclusive(resource_clone, runtime, permit)));
            },
        ))
    }
}

/// Async helper for releasing an exclusive lease.
///
/// Calls `reset()` on the resource and then drops the semaphore permit.
/// Holding the permit until after `reset()` resolves is the contract
/// documented on `Exclusive::reset`: the next caller cannot acquire the
/// exclusive lock until the previous reset has finished (#384).
async fn release_exclusive<R>(
    resource: R,
    runtime: Arc<R::Runtime>,
    permit: tokio::sync::OwnedSemaphorePermit,
) where
    R: Exclusive + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
{
    let _ = resource.reset(&runtime).await;
    drop(permit);
}
