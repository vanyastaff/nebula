//! `ResourceStatus.phase` lifecycle + config hot-reload integration tests for
//! nebula-resource v2: registration phase transitions, `Manager::reload_config`
//! across pooled/resident/bounded-exclusive topologies (fingerprint bump,
//! rebuild-on-next-acquire, stale-instance eviction), and the reload error
//! paths (invalid config, not-found, rejected-during-shutdown).
//!
//! Split out of the former monolithic `basic_integration.rs` (pure move, no
//! test-body changes) — shared mocks/helpers live in `tests/common/mod.rs`.

mod common;

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use common::{ResidentTestResource, TestConfig, test_config, test_ctx};
use nebula_core::{ResourceKey, resource_key};
use nebula_resource::{
    AcquireOptions, Bounded, Manager, Pooled, RegistrationSpec, Resident, ResidentConfig,
    ResourceContext, ScopeLevel, ShutdownConfig, SlotIdentity,
    error::{Error, ErrorKind},
    guard::ResourceGuard,
    resource::{Provider, ResourceConfig, ResourceMetadataDraft},
    topology::{
        bounded::BoundedProvider,
        pooled::{BrokenCheck, PoolProvider},
    },
};

// ---------------------------------------------------------------------------
// #387 — ResourceStatus.phase lifecycle
// ---------------------------------------------------------------------------

#[tokio::test]
async fn register_transitions_phase_to_ready() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("register");

    let snap = manager
        .health_check::<ResidentTestResource>(&ScopeLevel::Global)
        .expect("health");
    assert_eq!(snap.phase, nebula_resource::state::ResourcePhase::Ready);
    assert_eq!(snap.generation, 0);
}

#[tokio::test]
async fn reload_config_bumps_status_generation() {
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("register");

    let updated_config = TestConfig {
        name: "test-v2".into(),
    };
    manager
        .reload_config::<ResidentTestResource>(updated_config, &ScopeLevel::Global)
        .expect("reload");

    let snap = manager
        .health_check::<ResidentTestResource>(&ScopeLevel::Global)
        .expect("health");
    assert_eq!(snap.phase, nebula_resource::state::ResourcePhase::Ready);
    assert_eq!(
        snap.generation, 1,
        "reload_config must bake the new generation into ResourceStatus (#387)",
    );
}

#[tokio::test]
async fn reload_config_rebuilds_resident_master_with_new_config() {
    // A resident holds ONE shared master and clones it per acquire. Before the
    // fix, `reload_config` swapped the config but never rebuilt that master, so
    // an operator's new config never took effect on the live runtime. The
    // master must be rebuilt — lazily, on the next acquire — when the config
    // fingerprint changes. `ResidentTestResource::create` returns a fresh
    // instance carrying a monotonic create-id, so a rebuild is observable as a
    // changed id. Red-on-revert: without the rebuild, the same master (same id)
    // is reused after reload.
    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());
    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("register");

    let ctx = test_ctx();
    let first = manager
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire");
    let first_id = first.load(Ordering::Relaxed);
    drop(first);

    // Reload to a config with a DIFFERENT fingerprint (a different name).
    manager
        .reload_config::<ResidentTestResource>(
            TestConfig {
                name: "test-v2".into(),
            },
            &ScopeLevel::Global,
        )
        .expect("reload");

    let second = manager
        .acquire_resident::<ResidentTestResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("second acquire");
    let second_id = second.load(Ordering::Relaxed);

    assert_ne!(
        first_id, second_id,
        "reload_config must rebuild the resident master with the new config \
         (a changed config fingerprint forces a fresh create on the next acquire)"
    );
}

#[tokio::test]
async fn graceful_shutdown_report_marks_registry_cleared() {
    use nebula_resource::manager::ShutdownConfig;

    let manager = Manager::new();
    let resource = ResidentTestResource::new();
    let resident_rt = Resident::<ResidentTestResource>::new(ResidentConfig::default());

    manager
        .register(RegistrationSpec {
            resource,
            config: test_config(),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: resident_rt,
            recovery_gate: None,
        })
        .expect("register");

    let report = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("graceful");
    assert!(report.registry_cleared);
}

#[tokio::test]
async fn remove_nonexistent_returns_not_found() {
    let manager = Manager::new();
    let key = resource_key!("does-not-exist");

    let result = manager.remove(&key);

    assert!(result.is_err());
    let err = result.expect_err("should be an error");
    assert_eq!(*err.kind(), ErrorKind::NotFound);
}

// ---------------------------------------------------------------------------
// Config hot-reload tests
// ---------------------------------------------------------------------------

/// Config with a controllable fingerprint for reload tests.
#[derive(Clone, Debug, nebula_schema::Schema)]
struct ReloadConfig {
    fingerprint: u64,
    valid: bool,
}

impl ReloadConfig {
    fn new(fingerprint: u64) -> Self {
        Self {
            fingerprint,
            valid: true,
        }
    }

    fn invalid() -> Self {
        Self {
            fingerprint: 0,
            valid: false,
        }
    }
}

impl ResourceConfig for ReloadConfig {
    fn validate(&self) -> Result<(), Error> {
        if self.valid {
            Ok(())
        } else {
            Err(Error::permanent("invalid config"))
        }
    }

    fn fingerprint(&self) -> u64 {
        self.fingerprint
    }
}

/// Minimal pooled resource for reload tests.
#[derive(Clone)]
struct ReloadPoolResource {
    create_counter: Arc<AtomicU64>,
    destroy_counter: Arc<AtomicU64>,
    destroy_started: Arc<tokio::sync::Notify>,
    destroy_finished: Arc<tokio::sync::Notify>,
    destroy_gate: Option<Arc<tokio::sync::Notify>>,
}

impl ReloadPoolResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
            destroy_counter: Arc::new(AtomicU64::new(0)),
            destroy_started: Arc::new(tokio::sync::Notify::new()),
            destroy_finished: Arc::new(tokio::sync::Notify::new()),
            destroy_gate: None,
        }
    }

    fn with_controlled_destroy() -> Self {
        Self {
            destroy_gate: Some(Arc::new(tokio::sync::Notify::new())),
            ..Self::new()
        }
    }

    fn release_one_destroy(&self) {
        if let Some(gate) = &self.destroy_gate {
            gate.notify_one();
        }
    }
}

#[async_trait::async_trait]
impl Provider for ReloadPoolResource {
    type Config = ReloadConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("test-reload-pool")
    }

    async fn create(
        &self,
        _config: &ReloadConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
        let id = self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(AtomicU64::new(id)))
    }

    async fn destroy(
        &self,
        _instance: Arc<AtomicU64>,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        self.destroy_started.notify_one();
        if let Some(gate) = &self.destroy_gate {
            gate.notified().await;
        }
        self.destroy_counter.fetch_add(1, Ordering::Relaxed);
        self.destroy_finished.notify_one();
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::from_key(Self::key())
    }
}

nebula_resource::no_credential_slots!(ReloadPoolResource);

impl PoolProvider for ReloadPoolResource {
    fn is_broken(&self, _runtime: &Arc<AtomicU64>) -> BrokenCheck {
        BrokenCheck::Healthy
    }
}

/// Minimal Bounded-Exclusive resource for reload tests — reuses one instance,
/// reset between leases. `create` carries a monotonic id so a rebuild is
/// observable.
#[derive(Clone)]
struct ReloadExclusiveResource {
    create_counter: Arc<AtomicU64>,
}

impl ReloadExclusiveResource {
    fn new() -> Self {
        Self {
            create_counter: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for ReloadExclusiveResource {
    type Config = ReloadConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Bounded<Self>;

    fn key() -> ResourceKey {
        resource_key!("test-reload-exclusive")
    }

    async fn create(
        &self,
        _config: &ReloadConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
        let id = self.create_counter.fetch_add(1, Ordering::Relaxed);
        Ok(Arc::new(AtomicU64::new(id)))
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::from_key(Self::key())
    }
}

nebula_resource::no_credential_slots!(ReloadExclusiveResource);

impl BoundedProvider for ReloadExclusiveResource {}

#[tokio::test]
async fn reload_config_rebuilds_bounded_exclusive_instance_with_new_config() {
    // Bounded-Exclusive reuses ONE instance (reset between leases). Before the
    // fix, `reload_config` swapped the config but never rebuilt that reused
    // store-held instance, so an operator's new config never took effect. A
    // changed config fingerprint must evict-and-rebuild it on the next acquire
    // — symmetric to Pooled eviction and the Resident master rebuild. The
    // create-id makes a rebuild observable. Red-on-revert: without the
    // fingerprint-aware `accept`, the reset instance (same id) is reused.
    let manager = Arc::new(Manager::new());
    let resource = ReloadExclusiveResource::new();
    manager
        .register(RegistrationSpec {
            resource,
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Bounded::<ReloadExclusiveResource>::exclusive(),
            recovery_gate: None,
        })
        .expect("register");

    let ctx = test_ctx();
    let key = ReloadExclusiveResource::key();
    let acquire = || async {
        let boxed = Manager::acquire_any(
            Arc::clone(&manager),
            &key,
            &ctx,
            &AcquireOptions::default(),
            &SlotIdentity::Unbound,
        )
        .await
        .expect("acquire");
        *boxed
            .downcast::<ResourceGuard<ReloadExclusiveResource>>()
            .expect("downcast")
    };

    let first = acquire().await;
    let first_id = first.load(Ordering::Relaxed);
    // Await release so the reset instance is back in the store BEFORE the
    // reload — otherwise the next acquire would create fresh from an empty
    // store and not exercise the fingerprint-aware eviction.
    assert_eq!(
        first.release().await.expect("release returns the instance"),
        nebula_resource::ReleaseOutcome::Completed
    );

    manager
        .reload_config::<ReloadExclusiveResource>(ReloadConfig::new(42), &ScopeLevel::Global)
        .expect("reload");

    let second = acquire().await;
    let second_id = second.load(Ordering::Relaxed);

    assert_ne!(
        first_id, second_id,
        "reload_config must evict-and-rebuild the Bounded-Exclusive instance \
         with the new config (a changed fingerprint forces a fresh create)"
    );
}

#[tokio::test]
async fn reload_config_swaps_config_and_bumps_generation() {
    let manager = Manager::new();
    let resource = ReloadPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = Pooled::<ReloadPoolResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource,
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("register should succeed");

    // Check initial generation.
    let managed = manager
        .lookup::<ReloadPoolResource>(&ScopeLevel::Global)
        .expect("lookup should succeed");
    assert_eq!(managed.generation(), 0);
    assert_eq!(managed.config().fingerprint, 1);

    // Reload with new config.
    manager
        .reload_config::<ReloadPoolResource>(ReloadConfig::new(42), &ScopeLevel::Global)
        .expect("reload should succeed");

    assert_eq!(managed.generation(), 1);
    assert_eq!(managed.config().fingerprint, 42);
}

#[tokio::test]
async fn reload_config_rejects_invalid_config() {
    let manager = Manager::new();
    let resource = ReloadPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = Pooled::<ReloadPoolResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource,
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("register should succeed");

    // Reload with invalid config — should fail.
    let result =
        manager.reload_config::<ReloadPoolResource>(ReloadConfig::invalid(), &ScopeLevel::Global);
    assert!(result.is_err());
    assert_eq!(*result.unwrap_err().kind(), ErrorKind::Permanent);

    // Original config still intact.
    let managed = manager
        .lookup::<ReloadPoolResource>(&ScopeLevel::Global)
        .expect("lookup should succeed");
    assert_eq!(
        managed.generation(),
        0,
        "generation should not change on failure"
    );
    assert_eq!(
        managed.config().fingerprint,
        1,
        "config should not change on failure"
    );
}

#[tokio::test]
async fn reload_config_emits_event() {
    let manager = Manager::new();
    let mut rx = manager.subscribe_events();
    let resource = ReloadPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = Pooled::<ReloadPoolResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource,
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("register should succeed");

    // Drain the Registered event.
    let _ = rx.recv().await.expect("should receive Registered event");

    manager
        .reload_config::<ReloadPoolResource>(ReloadConfig::new(99), &ScopeLevel::Global)
        .expect("reload should succeed");

    let event = rx.recv().await.expect("should receive event");
    assert!(
        matches!(event, nebula_resource::ResourceEvent::ConfigReloaded { ref key } if key == &resource_key!("test-reload-pool")),
        "expected ConfigReloaded event, got {event:?}"
    );
}

#[tokio::test]
async fn reload_config_evicts_stale_pool_instances() {
    let manager = Manager::new();
    let resource = ReloadPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    };
    let pool_rt = Pooled::<ReloadPoolResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("register should succeed");

    let ctx = test_ctx();

    // Acquire and release to populate idle queue with fingerprint=1.
    let handle: ResourceGuard<ReloadPoolResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("first acquire should succeed");
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 1);
    drop(handle);
    // Wait for the release worker to recycle the instance back into idle so
    // there is a stale entry for the reload to evict (deterministic settle
    // via the observable idle count, not a wall-clock guess).
    {
        let deadline = std::time::Duration::from_secs(2);
        let start = std::time::Instant::now();
        loop {
            let idle = manager
                .pool_stats::<ReloadPoolResource>(&ScopeLevel::Global)
                .await
                .map_or(0, |s| s.idle);
            if idle >= 1 {
                break;
            }
            assert!(
                start.elapsed() < deadline,
                "released instance never recycled back into idle (idle={idle})"
            );
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    // Reload with new fingerprint — stale instances should be evicted.
    manager
        .reload_config::<ReloadPoolResource>(ReloadConfig::new(2), &ScopeLevel::Global)
        .expect("reload should succeed");

    // Next acquire should create a fresh instance (stale one evicted).
    let handle2: ResourceGuard<ReloadPoolResource> = manager
        .acquire_pooled(&ctx, &AcquireOptions::default())
        .await
        .expect("second acquire should succeed");
    assert_eq!(
        resource.create_counter.load(Ordering::Relaxed),
        2,
        "stale instance should have been evicted, forcing new creation"
    );

    drop(handle2);

    manager
        .graceful_shutdown(
            ShutdownConfig::default().with_drain_timeout(std::time::Duration::from_millis(50)),
        )
        .await
        .expect("graceful_shutdown must succeed");
}

#[tokio::test]
async fn stale_pool_destroy_does_not_block_fresh_acquire_under_topology_permit() {
    let manager = Arc::new(Manager::new());
    let resource = ReloadPoolResource::with_controlled_destroy();
    let pool_config = nebula_resource::topology::pooled::config::Config {
        max_size: 1,
        ..Default::default()
    };
    manager
        .register(RegistrationSpec {
            resource: resource.clone(),
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Pooled::<ReloadPoolResource>::new(pool_config, 1),
            recovery_gate: None,
        })
        .expect("register should succeed");

    let first = manager
        .acquire_pooled::<ReloadPoolResource>(&test_ctx(), &AcquireOptions::default())
        .await
        .expect("seed acquire should succeed");
    assert_eq!(
        first.release().await.expect("seed release should recycle"),
        nebula_resource::ReleaseOutcome::Completed
    );
    manager
        .reload_config::<ReloadPoolResource>(ReloadConfig::new(2), &ScopeLevel::Global)
        .expect("reload should stale the idle generation");

    let acquire_manager = Arc::clone(&manager);
    let mut acquire_task = tokio::spawn(async move {
        acquire_manager
            .acquire_pooled::<ReloadPoolResource>(&test_ctx(), &AcquireOptions::default())
            .await
    });
    resource.destroy_started.notified().await;
    let Ok(joined) =
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut acquire_task).await
    else {
        resource.release_one_destroy();
        let _ = acquire_task.await;
        panic!("fresh acquire waited for physical teardown while holding the pool permit");
    };
    let replacement = joined
        .expect("acquire task must not panic")
        .expect("fresh acquire must succeed");

    assert_eq!(replacement.load(Ordering::Relaxed), 1);
    assert_eq!(resource.create_counter.load(Ordering::Relaxed), 2);
    assert_eq!(
        resource.destroy_counter.load(Ordering::Relaxed),
        0,
        "the replacement is usable before stale physical teardown completes"
    );

    resource.release_one_destroy();
    resource.destroy_finished.notified().await;
    assert_eq!(resource.destroy_counter.load(Ordering::Relaxed), 1);
    assert_eq!(
        replacement.release().await.expect("replacement recycles"),
        nebula_resource::ReleaseOutcome::Completed
    );
    resource.release_one_destroy();
    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("shutdown succeeds after stale teardown settles");
    assert_eq!(resource.destroy_counter.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn reload_config_not_found_returns_error() {
    let manager = Manager::new();

    let result =
        manager.reload_config::<ReloadPoolResource>(ReloadConfig::new(1), &ScopeLevel::Global);
    assert!(result.is_err());
    assert_eq!(*result.unwrap_err().kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn reload_config_rejected_when_shutdown() {
    let manager = Manager::new();
    let resource = ReloadPoolResource::new();
    let pool_config = nebula_resource::topology::pooled::config::Config::default();
    let pool_rt = Pooled::<ReloadPoolResource>::new(pool_config, 1);

    manager
        .register(RegistrationSpec {
            resource,
            config: ReloadConfig::new(1),
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: pool_rt,
            recovery_gate: None,
        })
        .expect("register should succeed");

    manager.shutdown();

    let result =
        manager.reload_config::<ReloadPoolResource>(ReloadConfig::new(2), &ScopeLevel::Global);
    assert!(result.is_err());
    assert_eq!(*result.unwrap_err().kind(), ErrorKind::Cancelled);
}
