//! Shared mock resources and helpers for the `nebula-resource` integration
//! test suite (extracted from the former monolithic `basic_integration.rs`).
//!
//! Every file directly under `tests/` is a separate compilation unit, so a
//! helper that's only exercised by one consumer file looks "dead" to the
//! others — `#![allow(dead_code)]` suppresses those false-positive warnings.

#![allow(dead_code)]

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use nebula_core::{ExecutionId, ResourceKey, resource_key};
use nebula_resource::{
    Manager, ManagerConfig, Pooled, RegistrationSpec, Resident, ResourceContext, ScopeLevel,
    SlotIdentity,
    error::Error,
    resource::{HasCredentialSlots, Provider, ResourceConfig, ResourceMetadata},
    topology::{
        pooled::{BrokenCheck, PoolProvider},
        resident::ResidentProvider,
    },
};

// Custom error boilerplate removed — Resource lifecycle methods now return
// `crate::Error` directly (HasCredentialSlots redesign).

// ---------------------------------------------------------------------------
// Mock config
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub(crate) struct TestConfig {
    pub(crate) name: String,
}

nebula_schema::impl_empty_has_schema!(TestConfig);

impl ResourceConfig for TestConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.name.is_empty() {
            return Err(Error::permanent("name must not be empty"));
        }
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.name.hash(&mut h);
        h.finish()
    }
}

// ---------------------------------------------------------------------------
// Pooled mock resource
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct PoolTestResource {
    pub(crate) create_counter: Arc<AtomicU64>,
    pub(crate) break_flag: Arc<AtomicBool>,
    /// Incremented by `destroy`. The deterministic completion signal for a
    /// release that ends in destruction (tainted / broken / stale) — a
    /// release runs on the [`ReleaseQueue`] worker, so a test that asserts
    /// "the instance was NOT recycled" must wait for this rather than guess
    /// a wall-clock delay (idle stays `0` the whole time, so polling idle is
    /// not a usable settle signal for that case).
    pub(crate) destroy_counter: Arc<AtomicU64>,
}

impl PoolTestResource {
    pub(crate) fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            break_flag: Arc::new(AtomicBool::new(false)),
            destroy_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for PoolTestResource {
    type Config = TestConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("test-pool")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
        let id = self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(AtomicU64::new(id)))
    }

    async fn destroy(
        &self,
        _runtime: Arc<AtomicU64>,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        self.destroy_counter.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

nebula_resource::no_credential_slots!(PoolTestResource);

impl PoolProvider for PoolTestResource {
    fn is_broken(&self, _runtime: &Arc<AtomicU64>) -> BrokenCheck {
        if self.break_flag.load(Ordering::Relaxed) {
            BrokenCheck::Broken("forced".into())
        } else {
            BrokenCheck::Healthy
        }
    }
}

// ---------------------------------------------------------------------------
// Resident mock resource
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(crate) struct ResidentTestResource {
    pub(crate) create_counter: Arc<AtomicU64>,
    pub(crate) alive: Arc<AtomicBool>,
}

impl ResidentTestResource {
    pub(crate) fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            alive: Arc::new(AtomicBool::new(true)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for ResidentTestResource {
    type Config = TestConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test-resident")
    }

    async fn create(
        &self,
        _config: &TestConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
        let id = self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(AtomicU64::new(id)))
    }

    async fn destroy(
        &self,
        _runtime: Arc<AtomicU64>,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

nebula_resource::no_credential_slots!(ResidentTestResource);

#[async_trait::async_trait]
impl ResidentProvider for ResidentTestResource {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn test_ctx() -> ResourceContext {
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

pub(crate) fn test_config() -> TestConfig {
    TestConfig {
        name: "test".into(),
    }
}

/// Polls `cond` until it returns `true` or the deadline elapses, then
/// returns the final value of `cond`.
///
/// Replaces fixed `sleep(50ms)` "settle" points: release/recycle work runs
/// on the [`ReleaseQueue`] background worker, so the test must wait for the
/// *observable effect* (an idle count, a counter) rather than guess a
/// wall-clock delay. A short poll interval keeps fast cases fast; the
/// bounded deadline turns a real regression into a prompt failure instead of
/// a hang.
pub(crate) async fn poll_until(
    deadline: std::time::Duration,
    mut cond: impl FnMut() -> bool,
) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < deadline {
        if cond() {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    cond()
}

/// Reads the current idle count of a registered pool through the Manager's
/// public `pool_stats`. The framework owns the idle store now, so idle
/// observation goes through the Manager, not a (removed) inherent pool method.
pub(crate) async fn idle_count<R>(mgr: &Manager) -> usize
where
    R: PoolProvider
        + Provider<Topology = Pooled<R>>
        + HasCredentialSlots
        + Clone
        + Send
        + Sync
        + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    mgr.pool_stats::<R>(&ScopeLevel::Global)
        .await
        .map_or(0, |s| s.idle)
}

/// Waits until a registered pool's idle count equals `expected` (bounded),
/// failing the test with the observed count if it never does. The deterministic
/// replacement for `drop(handle); sleep(50ms); assert_eq!(idle, n)`.
pub(crate) async fn wait_idle_count<R>(mgr: &Manager, expected: usize)
where
    R: PoolProvider
        + Provider<Topology = Pooled<R>>
        + HasCredentialSlots
        + Clone
        + Send
        + Sync
        + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    let deadline = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        let idle = idle_count::<R>(mgr).await;
        if idle == expected {
            return;
        }
        assert!(
            start.elapsed() < deadline,
            "pool idle count never reached {expected}; last observed {idle}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Registers a pool resource at `Global` through the Manager funnel for the
/// acquire-path integration tests (the framework owns the acquire loop, so the
/// tests drive `Manager::acquire_pooled` rather than a removed inherent method).
pub(crate) fn register_pool<R>(mgr: &Manager, resource: R, config: R::Config, pool: Pooled<R>)
where
    R: PoolProvider
        + Provider<Topology = Pooled<R>>
        + HasCredentialSlots
        + Clone
        + Send
        + Sync
        + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    mgr.register(RegistrationSpec {
        resource,
        config,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: pool,
        recovery_gate: None,
    })
    .expect("pool registration must succeed");
}

/// Builds a metrics-wired `Manager` and registers a pool against it,
/// returning both so a test can read `manager.metrics().snapshot()` to
/// assert the recycle-vs-discard outcome split (ADR-0093 Tier-4).
pub(crate) fn pool_manager_with_metrics<R>(
    resource: R,
    config: R::Config,
    pool: Pooled<R>,
) -> Manager
where
    R: PoolProvider
        + Provider<Topology = Pooled<R>>
        + HasCredentialSlots
        + Clone
        + Send
        + Sync
        + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    let registry = Arc::new(nebula_metrics::MetricsRegistry::new());
    let mgr = Manager::with_config(ManagerConfig::default().with_metrics_registry(registry));
    register_pool(&mgr, resource, config, pool);
    mgr
}

/// Registers a resident resource at `Global` through the Manager funnel.
pub(crate) fn register_resident<R>(mgr: &Manager, resource: R, config: R::Config, rt: Resident<R>)
where
    R: ResidentProvider
        + Provider<Topology = Resident<R>>
        + HasCredentialSlots
        + Send
        + Sync
        + 'static,
    R::Instance: Clone + Send + Sync + 'static,
{
    mgr.register(RegistrationSpec {
        resource,
        config,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: rt,
        recovery_gate: None,
    })
    .expect("resident registration must succeed");
}

/// Waits until `counter` reaches at least `expected` (bounded). Used as the
/// release-completion signal for the destroyed-not-recycled case, where the
/// idle count stays `0` throughout and is therefore not a usable settle
/// signal.
pub(crate) async fn wait_count_at_least(counter: &Arc<AtomicU64>, expected: u64) {
    let deadline = std::time::Duration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        let observed = counter.load(Ordering::Relaxed);
        if observed >= expected {
            return;
        }
        assert!(
            start.elapsed() < deadline,
            "counter never reached {expected}; last observed {observed}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}
