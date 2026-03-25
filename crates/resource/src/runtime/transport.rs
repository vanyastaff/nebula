//! Transport runtime — shared connection, multiplexed sessions via semaphore.
//!
//! The transport runtime holds a persistent `Arc<R::Runtime>` and gates
//! concurrent sessions with a semaphore. Each acquire opens a session on
//! the shared transport; the guarded handle closes the session and drops
//! the semaphore permit on release.

use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::ctx::Ctx;
use crate::error::Error;
use crate::handle::ResourceHandle;
use crate::metrics::ResourceMetrics;
use crate::options::AcquireOptions;
use crate::release_queue::ReleaseQueue;
use crate::resource::Resource;
use crate::topology::transport::Transport;
use crate::topology::transport::config::Config;
use crate::topology_tag::TopologyTag;

/// Runtime state for a transport topology.
///
/// Holds a shared transport connection and a semaphore limiting
/// the number of concurrent sessions.
pub struct TransportRuntime<R: Resource> {
    runtime: Arc<R::Runtime>,
    session_semaphore: Arc<Semaphore>,
    config: Config,
}

impl<R: Resource> TransportRuntime<R> {
    /// Creates a new transport runtime wrapping an existing runtime instance.
    ///
    /// The semaphore is initialized with `config.max_sessions` permits.
    pub fn new(runtime: R::Runtime, config: Config) -> Self {
        let session_semaphore = Arc::new(Semaphore::new(config.max_sessions as usize));
        Self {
            runtime: Arc::new(runtime),
            session_semaphore,
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

impl<R> TransportRuntime<R>
where
    R: Transport + Clone + Send + Sync + 'static,
    R::Lease: Send + 'static,
    R::Runtime: Send + Sync + 'static,
{
    /// Acquires a session on the shared transport.
    ///
    /// 1. Acquires a semaphore permit (limiting concurrency to `max_sessions`).
    /// 2. Calls `resource.open_session(runtime, ctx)`.
    /// 3. Returns a guarded handle whose drop submits `close_session()` +
    ///    permit release to the [`ReleaseQueue`].
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Permanent`] if the semaphore is closed.
    /// - Propagates errors from `open_session`.
    pub async fn acquire(
        &self,
        resource: &R,
        ctx: &dyn Ctx,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        _options: &AcquireOptions,
        metrics: Arc<ResourceMetrics>,
    ) -> Result<ResourceHandle<R>, Error> {
        let permit = self
            .session_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::permanent("transport session semaphore closed"))?;

        let session = resource
            .open_session(&self.runtime, ctx)
            .await
            .map_err(Into::into)?;

        let runtime = self.runtime.clone();
        let resource_clone = resource.clone();
        let rq = release_queue.clone();

        Ok(ResourceHandle::guarded_with_permit(
            session,
            R::key(),
            TopologyTag::Transport,
            generation,
            move |lease, tainted| {
                metrics.record_release();
                rq.submit(move || {
                    Box::pin(release_transport_session(
                        resource_clone,
                        runtime,
                        lease,
                        !tainted,
                    ))
                });
            },
            Some(permit),
        ))
    }
}

/// Async helper for releasing a transport session.
///
/// Calls `close_session`. The semaphore permit is **not** held here — it
/// was already returned when the handle dropped (it lives in
/// `HandleInner::Guarded`, not in the callback closure).
async fn release_transport_session<R>(
    resource: R,
    runtime: Arc<R::Runtime>,
    session: R::Lease,
    healthy: bool,
) where
    R: Transport + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
{
    let _ = resource.close_session(&runtime, session, healthy).await;
}
