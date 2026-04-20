//! Service runtime — long-lived runtime, short-lived tokens.
//!
//! The service runtime holds a persistent `Arc<R::Runtime>` and hands
//! out lightweight tokens via [`Service::acquire_token`]. Depending on
//! [`TokenMode`], tokens are either owned (cloned, fire-and-forget) or
//! guarded (tracked, released via callback).

use std::sync::Arc;

use crate::{
    ctx::Ctx,
    error::Error,
    handle::ResourceHandle,
    metrics::ResourceOpsMetrics,
    options::AcquireOptions,
    release_queue::ReleaseQueue,
    resource::Resource,
    topology::service::{Service, TokenMode, config::Config},
    topology_tag::TopologyTag,
};

/// Runtime state for a service topology.
///
/// Holds the long-lived runtime and hands out short-lived tokens.
pub struct ServiceRuntime<R: Resource> {
    runtime: Arc<R::Runtime>,
    config: Config,
}

impl<R: Resource> ServiceRuntime<R> {
    /// Creates a new service runtime wrapping an existing runtime instance.
    pub fn new(runtime: R::Runtime, config: Config) -> Self {
        Self {
            runtime: Arc::new(runtime),
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

impl<R> ServiceRuntime<R>
where
    R: Service + Clone + Send + Sync + 'static,
    R::Lease: Send + 'static,
    R::Runtime: Send + Sync + 'static,
{
    /// Acquires a token from the service.
    ///
    /// 1. Calls `resource.acquire_token(runtime, ctx)`.
    /// 2. If [`TokenMode::Cloned`] — returns an owned handle.
    /// 3. If [`TokenMode::Tracked`] — returns a guarded handle whose drop submits `release_token()`
    ///    to the [`ReleaseQueue`].
    ///
    /// # Errors
    ///
    /// Returns an error if token acquisition fails.
    pub async fn acquire(
        &self,
        resource: &R,
        ctx: &dyn Ctx,
        release_queue: &Arc<ReleaseQueue>,
        generation: u64,
        _options: &AcquireOptions,
        metrics: Option<ResourceOpsMetrics>,
    ) -> Result<ResourceHandle<R>, Error> {
        let token = resource
            .acquire_token(&self.runtime, ctx)
            .await
            .map_err(Into::into)?;

        if R::TOKEN_MODE == TokenMode::Cloned {
            return Ok(ResourceHandle::owned(token, R::key(), TopologyTag::Service));
        }

        let runtime = self.runtime.clone();
        let resource_clone = resource.clone();
        let rq = release_queue.clone();

        Ok(ResourceHandle::guarded(
            token,
            R::key(),
            TopologyTag::Service,
            generation,
            move |lease, _tainted| {
                if let Some(m) = &metrics {
                    m.record_release();
                }
                rq.submit(move || Box::pin(release_service_token(resource_clone, runtime, lease)));
            },
        ))
    }
}

/// Async helper for releasing a tracked service token.
async fn release_service_token<R>(resource: R, runtime: Arc<R::Runtime>, lease: R::Lease)
where
    R: Service + Send + Sync + 'static,
    R::Runtime: Send + Sync + 'static,
{
    let _ = resource.release_token(&runtime, lease).await;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use nebula_core::{ExecutionId, ResourceKey, resource_key};

    use super::*;
    use crate::{
        ctx::BasicCtx,
        options::AcquireOptions,
        resource::{ResourceConfig, ResourceMetadata},
    };

    // -- Cloned-mode service --

    #[derive(Clone)]
    struct ClonedService;

    #[derive(Debug, Clone)]
    struct SvcError(String);

    impl std::fmt::Display for SvcError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for SvcError {}

    impl From<SvcError> for Error {
        fn from(e: SvcError) -> Self {
            Error::transient(e.0)
        }
    }

    impl ResourceConfig for String {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
    }

    impl Resource for ClonedService {
        type Config = String;
        type Runtime = String;
        type Lease = String;
        type Error = SvcError;
        type Auth = ();

        fn key() -> ResourceKey {
            resource_key!("cloned-svc")
        }

        async fn create(
            &self,
            _config: &String,
            _auth: &(),
            _ctx: &dyn Ctx,
        ) -> Result<String, SvcError> {
            Ok("runtime".into())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Service for ClonedService {
        const TOKEN_MODE: TokenMode = TokenMode::Cloned;

        fn acquire_token(
            &self,
            runtime: &String,
            _ctx: &dyn Ctx,
        ) -> impl Future<Output = Result<String, SvcError>> + Send {
            let token = format!("{runtime}-token");
            async move { Ok(token) }
        }
    }

    // -- Tracked-mode service --

    #[derive(Clone)]
    struct TrackedService {
        released: Arc<AtomicBool>,
    }

    impl Resource for TrackedService {
        type Config = String;
        type Runtime = String;
        type Lease = String;
        type Error = SvcError;
        type Auth = ();

        fn key() -> ResourceKey {
            resource_key!("tracked-svc")
        }

        async fn create(
            &self,
            _config: &String,
            _auth: &(),
            _ctx: &dyn Ctx,
        ) -> Result<String, SvcError> {
            Ok("tracked-runtime".into())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Service for TrackedService {
        const TOKEN_MODE: TokenMode = TokenMode::Tracked;

        fn acquire_token(
            &self,
            runtime: &String,
            _ctx: &dyn Ctx,
        ) -> impl Future<Output = Result<String, SvcError>> + Send {
            let token = format!("{runtime}-tracked-token");
            async move { Ok(token) }
        }

        fn release_token(
            &self,
            _runtime: &String,
            _token: String,
        ) -> impl Future<Output = Result<(), SvcError>> + Send {
            let released = self.released.clone();
            async move {
                released.store(true, Ordering::Relaxed);
                Ok(())
            }
        }
    }

    fn test_ctx() -> BasicCtx {
        BasicCtx::new(ExecutionId::new())
    }

    #[tokio::test]
    async fn cloned_service_returns_owned_handle() {
        let resource = ClonedService;
        let rt = ServiceRuntime::<ClonedService>::new("runtime".into(), Config::default());
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let ctx = test_ctx();

        let handle = rt
            .acquire(&resource, &ctx, &rq, 0, &AcquireOptions::default(), None)
            .await
            .unwrap();
        assert_eq!(*handle, "runtime-token");
        assert_eq!(handle.topology_tag(), TopologyTag::Service);
        // Owned handle — generation is None.
        assert!(handle.generation().is_none());

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }

    #[tokio::test]
    async fn tracked_service_returns_guarded_handle() {
        let released = Arc::new(AtomicBool::new(false));
        let resource = TrackedService {
            released: released.clone(),
        };
        let rt = ServiceRuntime::<TrackedService>::new("tracked-runtime".into(), Config::default());
        let (rq, rq_handle) = ReleaseQueue::new(1);
        let rq = Arc::new(rq);
        let ctx = test_ctx();

        let handle = rt
            .acquire(&resource, &ctx, &rq, 1, &AcquireOptions::default(), None)
            .await
            .unwrap();
        assert_eq!(*handle, "tracked-runtime-tracked-token");
        assert_eq!(handle.topology_tag(), TopologyTag::Service);
        assert_eq!(handle.generation(), Some(1));

        // Drop triggers release via ReleaseQueue.
        drop(handle);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(released.load(Ordering::Relaxed));

        drop(rq);
        ReleaseQueue::shutdown(rq_handle).await;
    }
}
