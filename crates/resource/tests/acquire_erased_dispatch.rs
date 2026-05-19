//! `Manager::acquire_erased` exercises the registry-stored `ErasedAcquireFn` hook.

use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use nebula_core::{ExecutionId, OrgId, ResourceKey, scope::Scope};
use nebula_resource::{
    AcquireOptions, Manager, RegistrationSpec, ResourceContext, ScopeLevel, SlotIdentity,
    dedup::SLOT_IDENTITY_UNBOUND,
    error::Error,
    resource::{Resource, ResourceConfig, ResourceMetadata},
    runtime::{
        TopologyRuntime, exclusive::ExclusiveRuntime, pool::PoolRuntime, resident::ResidentRuntime,
        service::ServiceRuntime, transport::TransportRuntime,
    },
    topology::resident::{self, Resident},
};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
struct ProbeError(String);

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProbeError {}

impl From<ProbeError> for Error {
    fn from(e: ProbeError) -> Self {
        Error::permanent(e.0)
    }
}

#[derive(Clone, Debug, Default)]
struct ProbeConfig;

nebula_schema::impl_empty_has_schema!(ProbeConfig);

impl ResourceConfig for ProbeConfig {}

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

impl Resource for ProbeResource {
    type Config = ProbeConfig;
    type Runtime = Arc<AtomicU64>;
    type Lease = Arc<AtomicU64>;
    type Error = ProbeError;

    fn key() -> ResourceKey {
        nebula_core::resource_key!("test.acquire_erased.probe")
    }

    fn create(
        &self,
        _config: &ProbeConfig,
        _ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Arc<AtomicU64>, ProbeError>> + Send {
        let counter = self.create_count.clone();
        async move {
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(counter)
        }
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for ProbeResource {
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
            topology: TopologyRuntime::Resident(ResidentRuntime::<ProbeResource>::new(
                resident::config::Config::default(),
            )),
            acquire: Manager::erased_acquire_resident::<ProbeResource>(SLOT_IDENTITY_UNBOUND),
            resilience: None,
            recovery_gate: None,
        })
        .expect("register");

    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let key = ProbeResource::key();
    let boxed = Manager::acquire_erased(
        Arc::clone(&manager),
        &key,
        &ctx,
        &AcquireOptions::default(),
        SLOT_IDENTITY_UNBOUND,
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
            topology: TopologyRuntime::Resident(ResidentRuntime::<ProbeResource>::new(
                resident::config::Config::default(),
            )),
            acquire: Manager::erased_acquire_resident::<ProbeResource>(SLOT_IDENTITY_UNBOUND),
            resilience: None,
            recovery_gate: None,
        })
        .expect("register at org scope");

    let key = ProbeResource::key();
    assert!(
        manager.has_registered_for(&key, &ScopeLevel::Organization(org), SLOT_IDENTITY_UNBOUND),
        "row must exist at org scope before acquire"
    );

    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        org_id: Some(org),
        ..Default::default()
    };
    assert!(
        manager.has_registered_for_scope(&key, &scope, SLOT_IDENTITY_UNBOUND),
        "scope-chain lookup must find org row"
    );

    let ctx = ResourceContext::minimal(scope, CancellationToken::new());

    let boxed = Manager::acquire_erased(
        Arc::clone(&manager),
        &ProbeResource::key(),
        &ctx,
        &AcquireOptions::default(),
        SLOT_IDENTITY_UNBOUND,
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
            topology: TopologyRuntime::Resident(ResidentRuntime::<ProbeResource>::new(
                resident::config::Config::default(),
            )),
            acquire: Manager::erased_acquire_resident::<ProbeResource>(SLOT_IDENTITY_UNBOUND),
            resilience: None,
            recovery_gate: None,
        })
        .expect("register global row");

    manager
        .register(RegistrationSpec {
            resource: org_resource,
            config: ProbeConfig,
            scope: ScopeLevel::Organization(org),
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(ResidentRuntime::<ProbeResource>::new(
                resident::config::Config::default(),
            )),
            acquire: Manager::erased_acquire_resident::<ProbeResource>(SLOT_IDENTITY_UNBOUND),
            resilience: None,
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

    let boxed = Manager::acquire_erased(
        Arc::clone(&manager),
        &key,
        &ctx,
        &AcquireOptions::default(),
        SLOT_IDENTITY_UNBOUND,
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
        .acquire_resident_for::<ProbeResource>(
            &ctx,
            &AcquireOptions::default(),
            SLOT_IDENTITY_UNBOUND,
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
macro_rules! parity_error {
    ($name:ident) => {
        #[derive(Debug)]
        struct $name(String);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl std::error::Error for $name {}

        impl From<$name> for Error {
            fn from(e: $name) -> Self {
                Error::permanent(e.0)
            }
        }
    };
}

mod pool_parity {
    use nebula_resource::topology::pooled::{BrokenCheck, Pooled, RecycleDecision};

    use super::*;

    parity_error!(PoolParityErr);

    #[derive(Clone, Default)]
    struct PoolParityCfg;
    nebula_schema::impl_empty_has_schema!(PoolParityCfg);
    impl ResourceConfig for PoolParityCfg {}

    #[derive(Clone)]
    struct PoolParity {
        create_count: Arc<AtomicU64>,
    }

    impl Resource for PoolParity {
        type Config = PoolParityCfg;
        type Runtime = u64;
        type Lease = u64;
        type Error = PoolParityErr;

        fn key() -> ResourceKey {
            nebula_core::resource_key!("test.ae4.pool")
        }

        async fn create(
            &self,
            _config: &PoolParityCfg,
            _ctx: &ResourceContext,
        ) -> Result<u64, PoolParityErr> {
            Ok(self.create_count.fetch_add(1, Ordering::SeqCst))
        }

        async fn destroy(&self, _runtime: u64) -> Result<(), PoolParityErr> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Pooled for PoolParity {
        fn is_broken(&self, _runtime: &u64) -> BrokenCheck {
            BrokenCheck::Healthy
        }

        async fn recycle(
            &self,
            _runtime: &u64,
            _metrics: &nebula_resource::topology::pooled::InstanceMetrics,
        ) -> Result<RecycleDecision, PoolParityErr> {
            Ok(RecycleDecision::Keep)
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
                topology: TopologyRuntime::Pool(PoolRuntime::<PoolParity>::new(
                    nebula_resource::topology::pooled::config::Config {
                        max_size: 4,
                        ..Default::default()
                    },
                    PoolParityCfg.fingerprint(),
                )),
                acquire: Manager::erased_acquire_pooled::<PoolParity>(SLOT_IDENTITY_UNBOUND),
                resilience: None,
                recovery_gate: None,
            })
            .expect("register pooled Global");

        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let key = PoolParity::key();

        let erased = Manager::acquire_erased(
            Arc::clone(&manager),
            &key,
            &ctx,
            &AcquireOptions::default(),
            SLOT_IDENTITY_UNBOUND,
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

mod service_parity {
    use nebula_resource::topology::service::{self, Service};

    use super::*;

    parity_error!(SvcParityErr);

    #[derive(Clone, Default)]
    struct SvcParityCfg;
    nebula_schema::impl_empty_has_schema!(SvcParityCfg);
    impl ResourceConfig for SvcParityCfg {}

    #[derive(Clone)]
    struct SvcParity {
        create_count: Arc<AtomicU64>,
    }

    impl Resource for SvcParity {
        type Config = SvcParityCfg;
        type Runtime = Arc<AtomicU64>;
        type Lease = u64;
        type Error = SvcParityErr;

        fn key() -> ResourceKey {
            nebula_core::resource_key!("test.ae4.service")
        }

        async fn create(
            &self,
            _config: &SvcParityCfg,
            _ctx: &ResourceContext,
        ) -> Result<Arc<AtomicU64>, SvcParityErr> {
            self.create_count.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::clone(&self.create_count))
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Service for SvcParity {
        async fn acquire_token(
            &self,
            runtime: &Arc<AtomicU64>,
            _ctx: &ResourceContext,
        ) -> Result<u64, SvcParityErr> {
            Ok(runtime.load(Ordering::SeqCst))
        }
    }

    /// Service's runtime is supplied at registration and shared across
    /// acquires (no `Resource::create` on the acquire path). Erased and
    /// typed `acquire_service` must both route through the single
    /// `run_acquire` to the SAME shared runtime — so both tokens read the
    /// same runtime value and `Resource::create` is never called.
    #[tokio::test]
    async fn service_erased_and_typed_share_one_run_acquire() {
        let create_count = Arc::new(AtomicU64::new(0));
        // Shared runtime carries a sentinel the token echoes, so an
        // erased↔typed value match proves both reached the same runtime.
        let shared_runtime = Arc::new(AtomicU64::new(0xA5A5));
        let manager = Arc::new(Manager::new());
        manager
            .register(RegistrationSpec {
                resource: SvcParity {
                    create_count: Arc::clone(&create_count),
                },
                config: SvcParityCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: TopologyRuntime::Service(ServiceRuntime::<SvcParity>::new(
                    Arc::clone(&shared_runtime),
                    service::config::Config::default(),
                )),
                acquire: Manager::erased_acquire_service::<SvcParity>(SLOT_IDENTITY_UNBOUND),
                resilience: None,
                recovery_gate: None,
            })
            .expect("register service Global");

        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let key = SvcParity::key();

        let erased = Manager::acquire_erased(
            Arc::clone(&manager),
            &key,
            &ctx,
            &AcquireOptions::default(),
            SLOT_IDENTITY_UNBOUND,
        )
        .await
        .expect("erased service acquire");
        let erased_token: u64 = **erased
            .downcast::<nebula_resource::ResourceGuard<SvcParity>>()
            .expect("downcast service guard");
        assert_eq!(erased_token, 0xA5A5, "erased token from shared runtime");

        let typed = manager
            .acquire_service::<SvcParity>(&ctx, &AcquireOptions::default())
            .await
            .expect("typed service acquire (single-tenant Global)");
        let typed_token: u64 = *typed;
        assert_eq!(
            typed_token, erased_token,
            "service runtime is shared: erased+typed over one Global row \
             through the single run_acquire must read the SAME runtime"
        );
        assert_eq!(
            create_count.load(Ordering::SeqCst),
            0,
            "service runtime is supplied at register; the acquire pipeline \
             must never call Resource::create"
        );
    }
}

mod transport_parity {
    use nebula_resource::topology::transport::{self, Transport};

    use super::*;

    parity_error!(TportParityErr);

    #[derive(Clone, Default)]
    struct TportParityCfg;
    nebula_schema::impl_empty_has_schema!(TportParityCfg);
    impl ResourceConfig for TportParityCfg {}

    #[derive(Clone)]
    struct TportParity {
        create_count: Arc<AtomicU64>,
    }

    impl Resource for TportParity {
        type Config = TportParityCfg;
        type Runtime = Arc<AtomicU64>;
        type Lease = u64;
        type Error = TportParityErr;

        fn key() -> ResourceKey {
            nebula_core::resource_key!("test.ae4.transport")
        }

        async fn create(
            &self,
            _config: &TportParityCfg,
            _ctx: &ResourceContext,
        ) -> Result<Arc<AtomicU64>, TportParityErr> {
            self.create_count.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::clone(&self.create_count))
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Transport for TportParity {
        async fn open_session(
            &self,
            transport: &Arc<AtomicU64>,
            _ctx: &ResourceContext,
        ) -> Result<u64, TportParityErr> {
            Ok(transport.load(Ordering::SeqCst))
        }
    }

    /// Transport's runtime is supplied at registration and shared across
    /// sessions (no `Resource::create` on the acquire path). Erased and
    /// typed `acquire_transport` must both route through the single
    /// `run_acquire` to the SAME shared runtime.
    #[tokio::test]
    async fn transport_erased_and_typed_share_one_run_acquire() {
        let create_count = Arc::new(AtomicU64::new(0));
        let shared_runtime = Arc::new(AtomicU64::new(0x7E7E));
        let manager = Arc::new(Manager::new());
        manager
            .register(RegistrationSpec {
                resource: TportParity {
                    create_count: Arc::clone(&create_count),
                },
                config: TportParityCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: TopologyRuntime::Transport(TransportRuntime::<TportParity>::new(
                    Arc::clone(&shared_runtime),
                    transport::config::Config::default(),
                )),
                acquire: Manager::erased_acquire_transport::<TportParity>(SLOT_IDENTITY_UNBOUND),
                resilience: None,
                recovery_gate: None,
            })
            .expect("register transport Global");

        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let key = TportParity::key();

        let erased = Manager::acquire_erased(
            Arc::clone(&manager),
            &key,
            &ctx,
            &AcquireOptions::default(),
            SLOT_IDENTITY_UNBOUND,
        )
        .await
        .expect("erased transport acquire");
        let erased_session: u64 = **erased
            .downcast::<nebula_resource::ResourceGuard<TportParity>>()
            .expect("downcast transport guard");
        assert_eq!(erased_session, 0x7E7E, "erased session from shared runtime");

        let typed = manager
            .acquire_transport::<TportParity>(&ctx, &AcquireOptions::default())
            .await
            .expect("typed transport acquire (single-tenant Global)");
        let typed_session: u64 = *typed;
        assert_eq!(
            typed_session, erased_session,
            "transport runtime is shared: erased+typed over one Global row \
             through the single run_acquire must read the SAME runtime"
        );
        assert_eq!(
            create_count.load(Ordering::SeqCst),
            0,
            "transport runtime is supplied at register; the acquire \
             pipeline must never call Resource::create"
        );
    }
}

mod exclusive_parity {
    use nebula_resource::topology::exclusive::{self, Exclusive};

    use super::*;

    parity_error!(ExclParityErr);

    #[derive(Clone, Default)]
    struct ExclParityCfg;
    nebula_schema::impl_empty_has_schema!(ExclParityCfg);
    impl ResourceConfig for ExclParityCfg {}

    #[derive(Clone)]
    struct ExclParity {
        create_count: Arc<AtomicU64>,
    }

    impl Resource for ExclParity {
        type Config = ExclParityCfg;
        type Runtime = u64;
        type Lease = u64;
        type Error = ExclParityErr;

        fn key() -> ResourceKey {
            nebula_core::resource_key!("test.ae4.exclusive")
        }

        async fn create(
            &self,
            _config: &ExclParityCfg,
            _ctx: &ResourceContext,
        ) -> Result<u64, ExclParityErr> {
            Ok(self.create_count.fetch_add(1, Ordering::SeqCst))
        }

        async fn destroy(&self, _runtime: u64) -> Result<(), ExclParityErr> {
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Exclusive for ExclParity {}

    /// Exclusive's runtime is supplied at registration and handed to one
    /// caller at a time (no `Resource::create` on the acquire path). The
    /// erased acquire then (after its guard drops + `reset`) the typed
    /// `acquire_exclusive` must both route through the single `run_acquire`
    /// to the SAME shared runtime; permit-held-until-`reset` preserved.
    #[tokio::test]
    async fn exclusive_erased_and_typed_share_one_run_acquire() {
        let create_count = Arc::new(AtomicU64::new(0));
        let manager = Arc::new(Manager::new());
        manager
            .register(RegistrationSpec {
                resource: ExclParity {
                    create_count: Arc::clone(&create_count),
                },
                config: ExclParityCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: TopologyRuntime::Exclusive(ExclusiveRuntime::<ExclParity>::new(
                    0xE0E0u64,
                    exclusive::config::Config::default(),
                )),
                acquire: Manager::erased_acquire_exclusive::<ExclParity>(SLOT_IDENTITY_UNBOUND),
                resilience: None,
                recovery_gate: None,
            })
            .expect("register exclusive Global");

        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let key = ExclParity::key();

        let erased = Manager::acquire_erased(
            Arc::clone(&manager),
            &key,
            &ctx,
            &AcquireOptions::default(),
            SLOT_IDENTITY_UNBOUND,
        )
        .await
        .expect("erased exclusive acquire");
        let erased_lease: u64 = **erased
            .downcast::<nebula_resource::ResourceGuard<ExclParity>>()
            .expect("downcast exclusive guard");
        assert_eq!(erased_lease, 0xE0E0, "erased lease from shared runtime");
        // Release the exclusive permit before the typed acquire (one caller
        // at a time); reset runs before the next caller is admitted.
        // Re-`downcast` would move the guard; instead drop the typed guard
        // explicitly below — the erased box was already consumed above.

        let typed = manager
            .acquire_exclusive::<ExclParity>(&ctx, &AcquireOptions::default())
            .await
            .expect("typed exclusive acquire (single-tenant Global)");
        let typed_lease: u64 = *typed;
        assert_eq!(
            typed_lease, erased_lease,
            "exclusive runtime is shared: erased+typed over one Global row \
             through the single run_acquire must read the SAME runtime"
        );
        assert_eq!(
            create_count.load(Ordering::SeqCst),
            0,
            "exclusive runtime is supplied at register; the acquire \
             pipeline must never call Resource::create"
        );
        drop(typed);
    }
}
