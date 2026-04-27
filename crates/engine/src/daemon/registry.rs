//! `DaemonRegistry` — engine-side dispatcher across registered daemons.
//!
//! Engine bootstrap or application code constructs a `DaemonRegistry`,
//! registers `Daemon` impls via [`DaemonRegistry::register`], and drives the
//! lifecycle via [`DaemonRegistry::start_all`] / [`DaemonRegistry::stop_all`] /
//! [`DaemonRegistry::shutdown`]. Action and resource code does not touch the
//! registry directly.
//!
//! # Cancellation propagation
//!
//! The registry owns a parent [`CancellationToken`]. Each registered
//! `DaemonRuntime` derives its own per-run child token from the parent (see
//! [`crate::daemon::DaemonRuntime`] cancellation model). [`Self::shutdown`]
//! cancels the parent, which cascades to every running daemon loop; live
//! `DaemonRuntime::stop` calls observe the cancellation via biased `select!`
//! and return promptly.
//!
//! # Fail-closed registration
//!
//! Mirrors the `crate::credential::StateProjectionRegistry` policy from
//! ADR-0030 / Tech Spec §15.6 N7 mitigation: duplicate
//! `D::key()` registration returns
//! [`DaemonError::DuplicateKey`] rather than overwriting. Operators resolve
//! the collision by renaming the daemon's `Resource::key`.

use std::{future::Future, pin::Pin, sync::Arc};

use dashmap::DashMap;
use futures::future::join_all;
use nebula_core::ResourceKey;
use nebula_resource::ResourceContext;
use tokio_util::sync::CancellationToken;

use crate::daemon::{Daemon, DaemonConfig, DaemonRuntime};

/// Object-safe handle that erases `D: Daemon` so different daemon types can
/// share a single `DashMap<ResourceKey, Arc<dyn AnyDaemonHandle>>`.
pub trait AnyDaemonHandle: Send + Sync {
    /// Start this daemon's background loop. See [`DaemonRuntime::start`].
    fn start(&self) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + '_>>;
    /// Stop this daemon's background loop. See [`DaemonRuntime::stop`].
    fn stop(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
    /// Whether this daemon's loop is currently running.
    fn is_running(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>>;
    /// The daemon's identifying key.
    fn key(&self) -> &ResourceKey;
}

/// Type-preserving handle wrapping a single registered daemon.
struct TypedDaemonHandle<D>
where
    D: Daemon + Clone + Send + Sync + 'static,
    D::Runtime: Send + Sync + 'static,
{
    daemon: D,
    runtime: Arc<D::Runtime>,
    runtime_state: Arc<DaemonRuntime<D>>,
    ctx: ResourceContext,
    key: ResourceKey,
}

impl<D> AnyDaemonHandle for TypedDaemonHandle<D>
where
    D: Daemon + Clone + Send + Sync + 'static,
    D::Runtime: Send + Sync + 'static,
{
    fn start(&self) -> Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send + '_>> {
        Box::pin(async move {
            self.runtime_state
                .start(self.daemon.clone(), Arc::clone(&self.runtime), &self.ctx)
                .await
                .map_err(|e| DaemonError::StartFailed {
                    key: self.key.clone(),
                    reason: e.to_string(),
                })
        })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move { self.runtime_state.stop().await })
    }

    fn is_running(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { self.runtime_state.is_running().await })
    }

    fn key(&self) -> &ResourceKey {
        &self.key
    }
}

/// Engine-side registry of `Daemon` impls.
///
/// See module docs for the cancellation, fail-closed, and lifecycle model.
pub struct DaemonRegistry {
    daemons: DashMap<ResourceKey, Arc<dyn AnyDaemonHandle>>,
    parent_cancel: CancellationToken,
}

impl DaemonRegistry {
    /// Build an empty registry with a fresh parent [`CancellationToken`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            daemons: DashMap::new(),
            parent_cancel: CancellationToken::new(),
        }
    }

    /// Build a registry whose parent token is the supplied one — useful when
    /// the engine wants daemon shutdown cascaded through a higher-level
    /// `CancellationToken` (e.g. process-wide shutdown).
    #[must_use]
    pub fn with_parent_cancel(parent_cancel: CancellationToken) -> Self {
        Self {
            daemons: DashMap::new(),
            parent_cancel,
        }
    }

    /// Returns the parent cancellation token.
    pub fn cancel_token(&self) -> &CancellationToken {
        &self.parent_cancel
    }

    /// Number of registered daemons.
    #[must_use]
    pub fn len(&self) -> usize {
        self.daemons.len()
    }

    /// Whether the registry has any registered daemons.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.daemons.is_empty()
    }

    /// Register a `Daemon` impl. Returns `DaemonError::DuplicateKey` if a
    /// daemon with the same `D::key()` is already registered.
    ///
    /// # Errors
    ///
    /// Returns `DaemonError::DuplicateKey` on collision (fail-closed per
    /// ADR-0030 / Tech Spec §15.6 N7).
    pub fn register<D>(
        &self,
        daemon: D,
        runtime: Arc<D::Runtime>,
        config: DaemonConfig,
        ctx: ResourceContext,
    ) -> Result<(), DaemonError>
    where
        D: Daemon + Clone + Send + Sync + 'static,
        D::Runtime: Send + Sync + 'static,
    {
        let key = D::key();
        if self.daemons.contains_key(&key) {
            return Err(DaemonError::DuplicateKey { key });
        }
        let runtime_state = Arc::new(DaemonRuntime::<D>::new(config, self.parent_cancel.clone()));
        let handle: Arc<dyn AnyDaemonHandle> = Arc::new(TypedDaemonHandle {
            daemon,
            runtime,
            runtime_state,
            ctx,
            key: key.clone(),
        });
        tracing::info!(daemon.key = %key, "daemon registered");
        self.daemons.insert(key, handle);
        Ok(())
    }

    /// Start every registered daemon in parallel.
    ///
    /// Failures are aggregated — a single daemon's failure does not abort
    /// sibling startups. Returns the first error if any daemon failed.
    ///
    /// # Errors
    ///
    /// Returns `DaemonError::StartFailed` for the first daemon that failed
    /// (others may have succeeded; check via [`Self::is_running`]).
    pub async fn start_all(&self) -> Result<(), DaemonError> {
        let handles: Vec<Arc<dyn AnyDaemonHandle>> = self
            .daemons
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect();
        let results = join_all(handles.iter().map(|h| h.start())).await;
        for r in results {
            r?;
        }
        Ok(())
    }

    /// Stop every registered daemon in parallel.
    ///
    /// Per-daemon cancellation flows through the per-run child token; parent
    /// stays live so the registry remains usable for subsequent `start_all`.
    pub async fn stop_all(&self) {
        let handles: Vec<Arc<dyn AnyDaemonHandle>> = self
            .daemons
            .iter()
            .map(|entry| Arc::clone(entry.value()))
            .collect();
        join_all(handles.iter().map(|h| h.stop())).await;
    }

    /// Whether the daemon under `key` is currently running.
    pub async fn is_running(&self, key: &ResourceKey) -> bool {
        match self.daemons.get(key) {
            Some(handle) => handle.is_running().await,
            None => false,
        }
    }

    /// Cancel the parent token and stop every daemon.
    ///
    /// After `shutdown`, the registry's parent token is cancelled and cannot
    /// be reused. Construct a new registry to start over.
    pub async fn shutdown(&self) {
        self.parent_cancel.cancel();
        self.stop_all().await;
    }
}

impl Default for DaemonRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for DaemonRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys: Vec<ResourceKey> = self
            .daemons
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        f.debug_struct("DaemonRegistry")
            .field("daemon_keys", &keys)
            .field("parent_cancelled", &self.parent_cancel.is_cancelled())
            .finish_non_exhaustive()
    }
}

/// Errors produced by [`DaemonRegistry`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DaemonError {
    /// A daemon with the same `Resource::key` is already registered.
    #[error("daemon already registered: {key}")]
    DuplicateKey {
        /// The colliding `Resource::key`.
        key: ResourceKey,
    },
    /// `DaemonRuntime::start` returned an error.
    #[error("daemon start failed for {key}: {reason}")]
    StartFailed {
        /// The daemon whose start failed.
        key: ResourceKey,
        /// The error message from `Resource::Error` propagation.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
        time::Duration,
    };

    use nebula_core::{ExecutionId, ResourceKey};
    use nebula_resource::{
        context::ResourceContext,
        error::Error as ResourceError,
        resource::{Resource, ResourceConfig, ResourceMetadata},
    };
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::daemon::{Daemon, DaemonConfig, RestartPolicy};

    #[derive(Clone, Debug, Default)]
    struct EmptyCfg;

    nebula_schema::impl_empty_has_schema!(EmptyCfg);

    impl ResourceConfig for EmptyCfg {
        fn fingerprint(&self) -> u64 {
            0
        }
    }

    #[derive(Debug, thiserror::Error)]
    #[error("registry-test: {0}")]
    struct TestError(&'static str);

    impl From<TestError> for ResourceError {
        fn from(e: TestError) -> Self {
            ResourceError::transient(e.to_string())
        }
    }

    #[derive(Clone)]
    struct CountedDaemon {
        attempts: Arc<AtomicU32>,
    }

    impl Resource for CountedDaemon {
        type Config = EmptyCfg;
        type Runtime = ();
        type Lease = ();
        type Error = TestError;
        type Credential = nebula_credential::NoCredential;

        fn key() -> ResourceKey {
            ResourceKey::new("registry-counted").unwrap()
        }

        async fn create(
            &self,
            _config: &Self::Config,
            _scheme: &<Self::Credential as nebula_credential::Credential>::Scheme,
            _ctx: &ResourceContext,
        ) -> Result<(), TestError> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Daemon for CountedDaemon {
        async fn run(
            &self,
            _runtime: &Self::Runtime,
            _ctx: &ResourceContext,
            cancel: CancellationToken,
        ) -> Result<(), TestError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            cancel.cancelled().await;
            Ok(())
        }
    }

    fn make_ctx() -> ResourceContext {
        ResourceContext::minimal(
            nebula_core::scope::Scope {
                execution_id: Some(ExecutionId::new()),
                ..Default::default()
            },
            CancellationToken::new(),
        )
    }

    #[tokio::test]
    async fn empty_registry_starts_and_stops() {
        let reg = DaemonRegistry::new();
        assert!(reg.is_empty());
        reg.start_all().await.expect("empty start_all is ok");
        reg.stop_all().await;
    }

    #[tokio::test]
    async fn register_starts_daemon_and_shutdown_cancels() {
        let reg = DaemonRegistry::new();
        let attempts = Arc::new(AtomicU32::new(0));
        let daemon = CountedDaemon {
            attempts: Arc::clone(&attempts),
        };

        reg.register(
            daemon,
            Arc::new(()),
            DaemonConfig {
                restart_policy: RestartPolicy::Never,
                max_restarts: 0,
                restart_backoff: Duration::from_millis(10),
            },
            make_ctx(),
        )
        .expect("register ok");
        assert_eq!(reg.len(), 1);

        reg.start_all().await.expect("start_all ok");
        // Give the daemon time to enter the cancel-await.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        assert!(reg.is_running(&CountedDaemon::key()).await);

        reg.shutdown().await;
        assert!(!reg.is_running(&CountedDaemon::key()).await);
    }

    #[tokio::test]
    async fn duplicate_register_fails_closed() {
        let reg = DaemonRegistry::new();
        let attempts = Arc::new(AtomicU32::new(0));
        let daemon_a = CountedDaemon {
            attempts: Arc::clone(&attempts),
        };
        let daemon_b = CountedDaemon {
            attempts: Arc::clone(&attempts),
        };

        reg.register(daemon_a, Arc::new(()), DaemonConfig::default(), make_ctx())
            .expect("first register ok");

        let err = reg
            .register(daemon_b, Arc::new(()), DaemonConfig::default(), make_ctx())
            .expect_err("second register must fail-closed");
        assert!(matches!(err, DaemonError::DuplicateKey { .. }));
    }
}
