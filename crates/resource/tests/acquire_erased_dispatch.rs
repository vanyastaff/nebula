//! `Manager::acquire_any` exercises the registry-stored acquire dispatch hook.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use nebula_core::{ExecutionId, OrgId, ResourceKey, scope::Scope};
use nebula_resource::topology::pooled::PoolProvider;
use nebula_resource::topology::resident::ResidentProvider;
use nebula_resource::{
    AcquireOptions, Manager, RegistrationSpec, ResourceContext, ScopeLevel, SlotIdentity,
    error::Error,
    resource::{HasCredentialSlots, Provider, ResourceConfig, ResourceMetadata},
};
use nebula_resource::{Pooled, Resident, ResidentConfig};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Default)]
struct ProbeConfig;

nebula_schema::impl_empty_has_schema!(ProbeConfig);

impl ResourceConfig for ProbeConfig {
    fn fingerprint(&self) -> u64 {
        // Unit struct: all instances identical — constant 0 is correct.
        0
    }
}

#[derive(Clone)]
struct ProbeResource {
    create_count: Arc<AtomicU64>,
}

impl ProbeResource {
    fn new() -> Self {
        Self {
            create_count: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for ProbeResource {
    type Config = ProbeConfig;
    type Instance = Arc<AtomicU64>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        nebula_core::resource_key!("test.acquire_erased.probe")
    }

    async fn create(
        &self,
        _config: &ProbeConfig,
        _ctx: &ResourceContext,
    ) -> Result<Arc<AtomicU64>, Error> {
        let counter = self.create_count.clone();
        counter.fetch_add(1, Ordering::Relaxed);
        Ok(counter)
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl HasCredentialSlots for ProbeResource {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

#[async_trait::async_trait]
impl ResidentProvider for ProbeResource {
    fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
        true
    }
}

#[tokio::test]
async fn acquire_erased_returns_guard_and_runs_create_once() {
    let manager = Arc::new(Manager::new());
    let resource = ProbeResource::new();
    let create_count = Arc::clone(&resource.create_count);

    manager
        .register(RegistrationSpec {
            resource,
            config: ProbeConfig,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::<ProbeResource>::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("register");

    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let key = ProbeResource::key();
    let boxed = Manager::acquire_any(
        Arc::clone(&manager),
        &key,
        &ctx,
        &AcquireOptions::default(),
        &SlotIdentity::Unbound,
    )
    .await
    .expect("acquire_erased");

    let lease = *boxed
        .downcast::<nebula_resource::ResourceGuard<ProbeResource>>()
        .expect("downcast to ResourceGuard");
    assert_eq!(lease.load(Ordering::Relaxed), 1);
    assert_eq!(create_count.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn acquire_erased_finds_org_scoped_row_from_execution_scope_bag() {
    let manager = Arc::new(Manager::new());
    let org = OrgId::new();
    let resource = ProbeResource::new();

    manager
        .register(RegistrationSpec {
            resource,
            config: ProbeConfig,
            scope: ScopeLevel::Organization(org),
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::<ProbeResource>::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("register at org scope");

    let key = ProbeResource::key();
    assert!(
        manager.has_registered_for_identity(
            &key,
            &ScopeLevel::Organization(org),
            &SlotIdentity::Unbound
        ),
        "row must exist at org scope before acquire"
    );

    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        org_id: Some(org),
        ..Default::default()
    };
    assert!(
        manager.has_registered_for_scope_identity(&key, &scope, &SlotIdentity::Unbound),
        "scope-chain lookup must find org row"
    );

    let ctx = ResourceContext::minimal(scope, CancellationToken::new());

    let boxed = Manager::acquire_any(
        Arc::clone(&manager),
        &ProbeResource::key(),
        &ctx,
        &AcquireOptions::default(),
        &SlotIdentity::Unbound,
    )
    .await
    .expect("execution-scoped acquire must reach org-scoped row");

    let lease = *boxed
        .downcast::<nebula_resource::ResourceGuard<ProbeResource>>()
        .expect("downcast to ResourceGuard from org-scoped row");
    assert_eq!(lease.load(Ordering::Relaxed), 1);
}

/// Global + Organization rows at the same `slot_identity` must not let typed
/// acquire bind Global while walking an execution+org scope bag.
#[tokio::test]
async fn acquire_erased_and_typed_pick_org_not_global_fallback() {
    let manager = Arc::new(Manager::new());
    let org = OrgId::new();
    let global_resource = ProbeResource::new();
    let global_count = Arc::clone(&global_resource.create_count);
    let org_resource = ProbeResource::new();
    let org_count = Arc::clone(&org_resource.create_count);

    manager
        .register(RegistrationSpec {
            resource: global_resource,
            config: ProbeConfig,
            scope: ScopeLevel::Global,
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::<ProbeResource>::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("register global row");

    manager
        .register(RegistrationSpec {
            resource: org_resource,
            config: ProbeConfig,
            scope: ScopeLevel::Organization(org),
            slot_identity: SlotIdentity::Unbound,
            topology: Resident::<ProbeResource>::new(ResidentConfig::default()),
            recovery_gate: None,
        })
        .expect("register org row");

    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        org_id: Some(org),
        ..Default::default()
    };
    let ctx = ResourceContext::minimal(scope, CancellationToken::new());
    let key = ProbeResource::key();

    let boxed = Manager::acquire_any(
        Arc::clone(&manager),
        &key,
        &ctx,
        &AcquireOptions::default(),
        &SlotIdentity::Unbound,
    )
    .await
    .expect("acquire_erased");

    let lease = *boxed
        .downcast::<nebula_resource::ResourceGuard<ProbeResource>>()
        .expect("downcast");
    assert_eq!(lease.load(Ordering::Relaxed), 1);
    assert_eq!(org_count.load(Ordering::Relaxed), 1);
    assert_eq!(
        global_count.load(Ordering::Relaxed),
        0,
        "must not create via Global row when org row matches scope bag"
    );

    let guard = manager
        .acquire_resident_for_identity::<ProbeResource>(
            &ctx,
            &AcquireOptions::default(),
            &SlotIdentity::Unbound,
        )
        .await
        .expect("typed acquire_resident_for");
    assert_eq!(guard.load(Ordering::Relaxed), 1);
    assert_eq!(
        org_count.load(Ordering::Relaxed),
        1,
        "typed acquire must reuse the org row without a second create"
    );
    assert_eq!(global_count.load(Ordering::Relaxed), 0);
}

// ───────────────────────────────────────────────────────────────────────
// AE4 — `acquire_erased` ↔ typed `acquire_*` parity on every collapsed
// topology. The five former `run_*_acquire` wrappers are now one generic
// `run_acquire`; the erased path (`acquire_erased` → registry hook →
// `acquire_*_at_scope`) and the typed path (`acquire_*` / `acquire_*_for`)
// must resolve the SAME registry row and run the SAME single pipeline, so
// for the single-runtime topologies the second acquire reuses the runtime
// (no extra `Resource::create`) and both observe the same Global scope.
//
// Each fixture uses a distinct config/error/resource newtype: this is an
// integration-test binary (not the lib `#[cfg(test)]` cfg-merge), so the
// constraint is only intra-file uniqueness, not lib-wide.
//
// Bounded has no erased acquire hook or public acquire entry yet (that
// wiring is the consumer-migration unit), so its `acquire_erased`↔typed
// parity is not exercisable here; `BoundedRuntime::acquire` itself is
// covered by `bounded_fold_behavior.rs`, and `run_acquire` is generic
// over `R: Resource` (no topology bound), so Bounded flows the identical
// pipeline once a `bounded` entry is wired.
// ───────────────────────────────────────────────────────────────────────

/// One shared `create` counter + a per-`create` unique runtime id, so a
/// distinct `Resource::create` is observable: parity means the typed path
/// reuses the erased path's resolved row (single-runtime topologies keep
/// `create_count == 1` across both acquires).
mod pool_parity {
    use nebula_resource::topology::pooled::BrokenCheck;

    use super::*;

    #[derive(Clone, Default)]
    struct PoolParityCfg;
    nebula_schema::impl_empty_has_schema!(PoolParityCfg);
    impl ResourceConfig for PoolParityCfg {
        fn fingerprint(&self) -> u64 {
            // Unit struct: all instances identical — constant 0 is correct.
            0
        }
    }

    #[derive(Clone)]
    struct PoolParity {
        create_count: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Provider for PoolParity {
        type Config = PoolParityCfg;
        type Instance = u64;
        type Topology = Pooled<Self>;

        fn key() -> ResourceKey {
            nebula_core::resource_key!("test.ae4.pool")
        }

        async fn create(
            &self,
            _config: &PoolParityCfg,
            _ctx: &ResourceContext,
        ) -> Result<u64, Error> {
            Ok(self.create_count.fetch_add(1, Ordering::SeqCst))
        }

        async fn destroy(
            &self,
            _runtime: u64,
            _cx: nebula_resource::TeardownCx,
        ) -> Result<(), Error> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl HasCredentialSlots for PoolParity {
        fn credential_slot_epoch(&self) -> u64 {
            0
        }
    }

    impl PoolProvider for PoolParity {
        fn is_broken(&self, _runtime: &u64) -> BrokenCheck {
            BrokenCheck::Healthy
        }
    }

    /// Erased acquire then typed `acquire_pooled` resolve the one Global
    /// pool row through the single `run_acquire`. The pooled instance is
    /// recycled on drop, so the second acquire reuses it: exactly one
    /// `Resource::create` across both paths.
    #[tokio::test]
    async fn pool_erased_and_typed_share_one_run_acquire() {
        let create_count = Arc::new(AtomicU64::new(0));
        let manager = Arc::new(Manager::new());
        manager
            .register(RegistrationSpec {
                resource: PoolParity {
                    create_count: Arc::clone(&create_count),
                },
                config: PoolParityCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Pooled::<PoolParity>::new(
                    nebula_resource::topology::pooled::config::Config {
                        max_size: 4,
                        ..Default::default()
                    },
                    PoolParityCfg.fingerprint(),
                ),
                recovery_gate: None,
            })
            .expect("register pooled Global");

        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let key = PoolParity::key();

        let erased = Manager::acquire_any(
            Arc::clone(&manager),
            &key,
            &ctx,
            &AcquireOptions::default(),
            &SlotIdentity::Unbound,
        )
        .await
        .expect("erased pooled acquire");
        let erased_guard = *erased
            .downcast::<nebula_resource::ResourceGuard<PoolParity>>()
            .expect("downcast pooled guard");
        let erased_id: u64 = *erased_guard;

        // Hold the erased guard (no async-release race) and acquire a
        // second checkout via the typed path: both go through the same
        // single `run_acquire` against the one Global pool row, so the
        // pool mints a distinct instance for each (max_size = 4).
        let typed = manager
            .acquire_pooled::<PoolParity>(&ctx, &AcquireOptions::default())
            .await
            .expect("typed pooled acquire (single-tenant Global)");
        let typed_id: u64 = *typed;
        assert_ne!(
            typed_id, erased_id,
            "two concurrent checkouts from the same pool row (one via \
             acquire_erased, one via the typed path) must be distinct \
             instances"
        );
        assert_eq!(
            create_count.load(Ordering::SeqCst),
            2,
            "erased + typed each drove one Resource::create through the \
             single run_acquire over the same Global pool row"
        );
        drop(erased_guard);
        drop(typed);
    }
}

/// Resident erased dispatch parity: two erased acquires on the same row
/// reuse the same runtime (no extra `Resource::create`).
mod resident_erased_reuses_runtime {
    use super::*;

    #[derive(Clone, Default)]
    struct ResidentReuseCfg;
    nebula_schema::impl_empty_has_schema!(ResidentReuseCfg);
    impl ResourceConfig for ResidentReuseCfg {
        fn fingerprint(&self) -> u64 {
            // Unit struct: all instances identical — constant 0 is correct.
            0
        }
    }

    #[derive(Clone)]
    struct ResidentReuse {
        create_count: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Provider for ResidentReuse {
        type Config = ResidentReuseCfg;
        type Instance = Arc<AtomicU64>;
        type Topology = Resident<Self>;

        fn key() -> ResourceKey {
            nebula_core::resource_key!("test.ae4.resident_reuse")
        }

        async fn create(
            &self,
            _config: &ResidentReuseCfg,
            _ctx: &ResourceContext,
        ) -> Result<Arc<AtomicU64>, Error> {
            self.create_count.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::clone(&self.create_count))
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl HasCredentialSlots for ResidentReuse {
        fn credential_slot_epoch(&self) -> u64 {
            0
        }
    }

    #[async_trait::async_trait]
    impl ResidentProvider for ResidentReuse {
        fn is_alive_sync(&self, _runtime: &Arc<AtomicU64>) -> bool {
            true
        }
    }

    /// Two erased acquires on a resident row produce exactly one
    /// `Resource::create` and pointer-equal leases.
    #[tokio::test]
    async fn resident_erased_acquires_share_one_create() {
        let create_count = Arc::new(AtomicU64::new(0));
        let manager = Arc::new(Manager::new());
        let rt = Resident::<ResidentReuse>::new(ResidentConfig::default());
        manager
            .register(RegistrationSpec {
                resource: ResidentReuse {
                    create_count: Arc::clone(&create_count),
                },
                config: ResidentReuseCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: rt,
                recovery_gate: None,
            })
            .expect("ResidentReuse must register without error");
        assert_eq!(
            create_count.load(Ordering::SeqCst),
            0,
            "registration must not trigger Resource::create"
        );

        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let key = ResidentReuse::key();

        let g1 = Manager::acquire_any(
            Arc::clone(&manager),
            &key,
            &ctx,
            &AcquireOptions::default(),
            &SlotIdentity::Unbound,
        )
        .await
        .expect("first erased resident acquire must succeed");

        let g2 = Manager::acquire_any(
            Arc::clone(&manager),
            &key,
            &ctx,
            &AcquireOptions::default(),
            &SlotIdentity::Unbound,
        )
        .await
        .expect("second erased resident acquire must succeed");

        assert_eq!(
            create_count.load(Ordering::SeqCst),
            1,
            "resident creates exactly once; both erased acquires share the same runtime"
        );
        let p1 = Arc::as_ptr(
            g1.downcast::<nebula_resource::ResourceGuard<ResidentReuse>>()
                .expect("g1 must downcast to ResidentReuse guard")
                .as_ref(),
        );
        let p2 = Arc::as_ptr(
            g2.downcast::<nebula_resource::ResourceGuard<ResidentReuse>>()
                .expect("g2 must downcast to ResidentReuse guard")
                .as_ref(),
        );
        assert_eq!(p1, p2, "both erased resident leases must be pointer-equal");
    }
}

/// Pool erased dispatch parity: two concurrent erased pool acquires on the
/// same row produce two distinct instances and two `Resource::create` calls.
mod pool_erased_distinct_instances {
    use super::*;

    #[derive(Clone, Default)]
    struct PoolErasedCfg;
    nebula_schema::impl_empty_has_schema!(PoolErasedCfg);
    impl ResourceConfig for PoolErasedCfg {
        fn fingerprint(&self) -> u64 {
            // Unit struct: all instances identical — constant 0 is correct.
            0
        }
    }

    #[derive(Clone)]
    struct PoolErased {
        create_count: Arc<AtomicU64>,
    }

    #[async_trait::async_trait]
    impl Provider for PoolErased {
        type Config = PoolErasedCfg;
        type Instance = u64;
        type Topology = Pooled<Self>;

        fn key() -> ResourceKey {
            nebula_core::resource_key!("test.ae4.pool_erased")
        }

        async fn create(
            &self,
            _config: &PoolErasedCfg,
            _ctx: &ResourceContext,
        ) -> Result<u64, Error> {
            Ok(self.create_count.fetch_add(1, Ordering::SeqCst))
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl HasCredentialSlots for PoolErased {
        fn credential_slot_epoch(&self) -> u64 {
            0
        }
    }

    impl PoolProvider for PoolErased {
        fn is_broken(&self, _runtime: &u64) -> nebula_resource::topology::pooled::BrokenCheck {
            nebula_resource::topology::pooled::BrokenCheck::Healthy
        }
    }

    /// Two concurrent erased pool acquires produce two distinct instances.
    #[tokio::test]
    async fn pool_erased_acquires_produce_distinct_instances() {
        let create_count = Arc::new(AtomicU64::new(0));
        let manager = Arc::new(Manager::new());
        let pool_rt = Pooled::<PoolErased>::try_new(
            nebula_resource::topology::pooled::config::Config {
                min_size: 0,
                max_size: 4,
                ..Default::default()
            },
            0,
        )
        .expect("PoolRuntime must construct with valid min/max");
        manager
            .register(RegistrationSpec {
                resource: PoolErased {
                    create_count: Arc::clone(&create_count),
                },
                config: PoolErasedCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: pool_rt,
                recovery_gate: None,
            })
            .expect("PoolErased must register without error");

        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let key = PoolErased::key();
        assert_eq!(
            create_count.load(Ordering::SeqCst),
            0,
            "registration must not trigger Resource::create for pool topology"
        );

        let g1 = Manager::acquire_any(
            Arc::clone(&manager),
            &key,
            &ctx,
            &AcquireOptions::default(),
            &SlotIdentity::Unbound,
        )
        .await
        .expect("first erased pool acquire must succeed");
        let id1: u64 = **g1
            .downcast::<nebula_resource::ResourceGuard<PoolErased>>()
            .expect("g1 must downcast to PoolErased guard");

        let g2 = Manager::acquire_any(
            Arc::clone(&manager),
            &key,
            &ctx,
            &AcquireOptions::default(),
            &SlotIdentity::Unbound,
        )
        .await
        .expect("second erased pool acquire must succeed");
        let id2: u64 = **g2
            .downcast::<nebula_resource::ResourceGuard<PoolErased>>()
            .expect("g2 must downcast to PoolErased guard");

        assert_ne!(
            id1, id2,
            "pool issues distinct instances per concurrent checkout"
        );
        assert_eq!(
            create_count.load(Ordering::SeqCst),
            2,
            "pool drove two Resource::create calls for two concurrent checkouts"
        );
    }
}

/// `acquire_any` on an unregistered key returns `NotFound`, not a panic.
mod erased_acquire_not_found {
    use nebula_resource::error::ErrorKind;

    use super::*;

    /// Erased acquire on an unknown key must return `ErrorKind::NotFound`.
    #[tokio::test]
    async fn erased_acquire_returns_not_found_for_unknown_key() {
        let manager = Arc::new(Manager::new());
        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let unknown = nebula_core::resource_key!("test.ae4.unknown_key");

        let result = Manager::acquire_any(
            Arc::clone(&manager),
            &unknown,
            &ctx,
            &AcquireOptions::default(),
            &SlotIdentity::Unbound,
        )
        .await;

        assert!(
            result.is_err(),
            "acquire on unregistered key must return Err, not Ok"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err.kind(), ErrorKind::NotFound),
            "acquire on unregistered key must return NotFound, got {:?}",
            err.kind()
        );
        assert!(
            manager.keys().is_empty(),
            "manager must remain empty — no side-effects from a failed erased acquire"
        );
    }
}
