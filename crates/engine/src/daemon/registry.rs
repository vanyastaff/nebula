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
//! [`crate::daemon::DaemonRuntime`] cancellation model). [`DaemonRegistry::shutdown`]
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
                    source: Box::new(e),
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
    #[must_use]
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

    /// Register a `Daemon` impl.
    ///
    /// Atomic via DashMap's [`Entry`](dashmap::mapref::entry::Entry) API:
    /// duplicate detection and insertion happen under a single shard lock.
    ///
    /// # Errors
    ///
    /// - [`DaemonError::DuplicateKey`] when a daemon with the same `D::key()` is already registered
    ///   (fail-closed per ADR-0030 / Tech Spec §15.6 N7).
    /// - [`DaemonError::RegistryCancelled`] when [`Self::shutdown`] has already cancelled the
    ///   parent token; subsequent daemons would inherit a pre-cancelled child and never run, so the
    ///   registry refuses the registration upfront.
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
        if self.parent_cancel.is_cancelled() {
            return Err(DaemonError::RegistryCancelled);
        }
        let key = D::key();
        let runtime_state = Arc::new(DaemonRuntime::<D>::new(config, self.parent_cancel.clone()));
        let handle: Arc<dyn AnyDaemonHandle> = Arc::new(TypedDaemonHandle {
            daemon,
            runtime,
            runtime_state,
            ctx,
            key: key.clone(),
        });
        match self.daemons.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(_) => Err(DaemonError::DuplicateKey { key }),
            dashmap::mapref::entry::Entry::Vacant(slot) => {
                tracing::info!(daemon.key = %key, "daemon registered");
                slot.insert(handle);
                Ok(())
            },
        }
    }

    /// Start every registered daemon in parallel.
    ///
    /// Failures are aggregated — a single daemon's failure does not abort
    /// sibling startups. Returns the first error if any daemon failed.
    ///
    /// # Errors
    ///
    /// - [`DaemonError::RegistryCancelled`] if [`Self::shutdown`] already cancelled the parent
    ///   token.
    /// - [`DaemonError::StartFailed`] for the first daemon that failed (others may have succeeded;
    ///   check via [`Self::is_running`]).
    pub async fn start_all(&self) -> Result<(), DaemonError> {
        if self.parent_cancel.is_cancelled() {
            return Err(DaemonError::RegistryCancelled);
        }
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
        // Clone the Arc<dyn AnyDaemonHandle> out of the DashMap before
        // awaiting — holding the entry guard across `.await` would block
        // shard access for other operations and is the same pattern used
        // by `start_all`/`stop_all` for their handle-collection step.
        let handle = self.daemons.get(key).map(|entry| Arc::clone(entry.value()));
        match handle {
            Some(handle) => handle.is_running().await,
            None => false,
        }
    }

    /// Cancel the parent token and stop every daemon.
    ///
    /// After `shutdown`, the registry's parent token is cancelled. Subsequent
    /// [`Self::register`] / [`Self::start_all`] calls return
    /// [`DaemonError::RegistryCancelled`] — the registry refuses to register
    /// new daemons that would inherit a pre-cancelled child token, and
    /// refuses to attempt restarts. Construct a new registry to start over.
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
    ///
    /// The original error is preserved as `#[source]` so callers can walk
    /// the error chain via [`std::error::Error::source`] for full diagnostic
    /// context.
    #[error("daemon start failed for {key}")]
    StartFailed {
        /// The daemon whose start failed.
        key: ResourceKey,
        /// Original error from `DaemonRuntime::start` (typically a
        /// `nebula_resource::Error`).
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The registry's parent token has been cancelled by [`DaemonRegistry::shutdown`].
    /// Subsequent registrations or start attempts are refused — construct a
    /// new registry instead.
    #[error(
        "daemon registry has been shut down; construct a new registry to register or start daemons"
    )]
    RegistryCancelled,
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

    /// Second test daemon used by the parallel-start + aggregation tests.
    /// Distinct `Resource::key` so it can register alongside `CountedDaemon`.
    #[derive(Clone)]
    struct CountedDaemonB {
        attempts: Arc<AtomicU32>,
    }
    impl Resource for CountedDaemonB {
        type Config = EmptyCfg;
        type Runtime = ();
        type Lease = ();
        type Error = TestError;
        type Credential = nebula_credential::NoCredential;
        fn key() -> ResourceKey {
            ResourceKey::new("registry-counted-b").unwrap()
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
    impl Daemon for CountedDaemonB {
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

    /// `start_all` invokes every registered daemon's start in parallel —
    /// neither daemon blocks the other from making progress.
    #[tokio::test]
    async fn start_all_starts_daemons_in_parallel() {
        let reg = DaemonRegistry::new();
        let attempts_a = Arc::new(AtomicU32::new(0));
        let attempts_b = Arc::new(AtomicU32::new(0));

        reg.register(
            CountedDaemon {
                attempts: Arc::clone(&attempts_a),
            },
            Arc::new(()),
            DaemonConfig::default(),
            make_ctx(),
        )
        .expect("register A");
        reg.register(
            CountedDaemonB {
                attempts: Arc::clone(&attempts_b),
            },
            Arc::new(()),
            DaemonConfig::default(),
            make_ctx(),
        )
        .expect("register B");

        reg.start_all().await.expect("start_all ok");
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Both daemons reached their loop body — parallel start, neither
        // blocked the other.
        assert_eq!(attempts_a.load(Ordering::SeqCst), 1);
        assert_eq!(attempts_b.load(Ordering::SeqCst), 1);
        assert!(reg.is_running(&CountedDaemon::key()).await);
        assert!(reg.is_running(&CountedDaemonB::key()).await);

        reg.shutdown().await;
    }

    /// Failure-aggregation contract: when `start_all` calls `start()` on a
    /// daemon that's already running (returns `Err("daemon is already
    /// running")`), the failure does NOT tear down the live siblings.
    /// `start_all` returns the first error; both daemons remain running
    /// from the prior `start_all` call.
    #[tokio::test]
    async fn start_all_does_not_tear_down_siblings_on_repeat_failure() {
        let reg = DaemonRegistry::new();
        let attempts_a = Arc::new(AtomicU32::new(0));
        let attempts_b = Arc::new(AtomicU32::new(0));

        reg.register(
            CountedDaemon {
                attempts: Arc::clone(&attempts_a),
            },
            Arc::new(()),
            DaemonConfig::default(),
            make_ctx(),
        )
        .expect("register A");
        reg.register(
            CountedDaemonB {
                attempts: Arc::clone(&attempts_b),
            },
            Arc::new(()),
            DaemonConfig::default(),
            make_ctx(),
        )
        .expect("register B");

        reg.start_all().await.expect("first start_all ok");
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(reg.is_running(&CountedDaemon::key()).await);
        assert!(reg.is_running(&CountedDaemonB::key()).await);

        // Second start_all — both daemons still have running tasks, so each
        // `DaemonRuntime::start` returns Err("daemon is already running").
        // The aggregation contract: `start_all` collects both errors and
        // returns the first one — it does NOT short-circuit, and it does
        // NOT tear down siblings.
        let result = reg.start_all().await;
        assert!(
            matches!(result, Err(DaemonError::StartFailed { .. })),
            "second start_all expected to surface 'already running' error: {result:?}",
        );

        // Both daemons remain running — the failed start_all did not poison
        // their state.
        assert!(reg.is_running(&CountedDaemon::key()).await);
        assert!(reg.is_running(&CountedDaemonB::key()).await);

        reg.shutdown().await;
    }

    /// `register` and `start_all` reject calls after `shutdown` has cancelled
    /// the parent token. Without this guard, daemons would inherit a
    /// pre-cancelled child and silently never run.
    #[tokio::test]
    async fn register_and_start_all_rejected_after_shutdown() {
        let reg = DaemonRegistry::new();
        reg.shutdown().await;

        let err = reg
            .register(
                CountedDaemon {
                    attempts: Arc::new(AtomicU32::new(0)),
                },
                Arc::new(()),
                DaemonConfig::default(),
                make_ctx(),
            )
            .expect_err("register after shutdown must error");
        assert!(matches!(err, DaemonError::RegistryCancelled));

        let start_err = reg
            .start_all()
            .await
            .expect_err("start_all after shutdown must error");
        assert!(matches!(start_err, DaemonError::RegistryCancelled));
    }
}
