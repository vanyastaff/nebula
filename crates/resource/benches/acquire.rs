//! Acquire hot-path benchmarks over the framework acquire loop.
//!
//! The perf research (2026-06) flagged the acquire cache-hit branch as the
//! path that must stay allocation-lean: reserve → fenced checkout → accept →
//! prepare → guard build, with the boxed release future deferred to guard
//! drop. This bench pins that path so a future "optimization" (or an
//! accidental clone/box on the hit branch) shows up as a regression on
//! CodSpeed rather than in production:
//!
//! - `pooled_hit` — idle-hit acquire → explicit `release` (recycle back to
//!   the idle queue). The steady-state pool cycle.
//! - `pooled_create_destroy` — idle-miss acquire (create) → discarding
//!   release (destroy). The cold path, for the hit/miss ratio.
//! - `resident_hit` — acquire of the shared resident instance (clone) →
//!   drop. The cheapest lease the framework hands out.
//! - `lease_pooled_attempt` / `row_pooled_attempt` — one managed attempt on
//!   a one-connection pool through a lease facade and through a row facade
//!   that checks out per attempt (idle hit, release included).

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_resource::{
    AcquireOptions, Error, Manager, PoolConfig, Pooled, Provider, RegistrationSpec, Resident,
    ResidentConfig, ResourceConfig, ResourceContext, ResourceKey, ScopeLevel, SlotIdentity,
    resource::ResourceMetadataDraft,
    resource_key,
    topology::pooled::{PoolProvider, RecycleDecision},
    topology::resident::ResidentProvider,
};

#[derive(Clone, nebula_schema::Schema)]
struct BenchCfg;

impl ResourceConfig for BenchCfg {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        // Unit struct: all instances identical — constant 0 is correct.
        0
    }
}

/// Pooled resource whose default `recycle` keeps instances (no credential
/// slots), so acquire → release cycles exercise the idle-hit path.
#[derive(Clone)]
struct KeepPool;

#[async_trait::async_trait]
impl Provider for KeepPool {
    type Config = BenchCfg;
    type Instance = u64;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("bench-pool-keep")
    }

    async fn create(&self, _config: &BenchCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
        Ok(0xBEEF)
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("bench-pool-keep"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(KeepPool);

impl PoolProvider for KeepPool {}

/// Pooled resource that discards on release, so every acquire runs the
/// create path and every release runs destroy.
#[derive(Clone)]
struct DiscardPool;

#[async_trait::async_trait]
impl Provider for DiscardPool {
    type Config = BenchCfg;
    type Instance = u64;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("bench-pool-discard")
    }

    async fn create(&self, _config: &BenchCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
        Ok(0xDEAD)
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("bench-pool-discard"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(DiscardPool);

impl PoolProvider for DiscardPool {
    async fn recycle(
        &self,
        _instance: &u64,
        _metrics: &nebula_resource::topology::pooled::InstanceMetrics,
    ) -> Result<RecycleDecision, Error> {
        Ok(RecycleDecision::Drop)
    }
}

/// Resident resource: one shared instance, cloned per acquire.
#[derive(Clone)]
struct SharedResident;

#[async_trait::async_trait]
impl Provider for SharedResident {
    type Config = BenchCfg;
    type Instance = u64;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("bench-resident")
    }

    async fn create(&self, _config: &BenchCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
        Ok(0xF00D)
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("bench-resident"),
            "",
        )
    }
}

nebula_resource::no_credential_slots!(SharedResident);

#[async_trait::async_trait]
impl ResidentProvider for SharedResident {
    fn is_alive_sync(&self, _instance: &u64) -> bool {
        true
    }
}

fn bench_ctx() -> ResourceContext {
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

fn bench_acquire(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("bench runtime");
    let mut group = c.benchmark_group("resource/acquire");

    // Manager construction spawns the ReleaseQueue workers — build inside
    // the runtime so the background tasks have an executor.
    let manager = rt.block_on(async {
        let manager = Manager::new();
        manager
            .register(RegistrationSpec {
                resource: KeepPool,
                config: BenchCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Pooled::<KeepPool>::new(PoolConfig::default(), 0),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register keep pool");
        manager
            .register(RegistrationSpec {
                resource: DiscardPool,
                config: BenchCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Pooled::<DiscardPool>::new(PoolConfig::default(), 0),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register discard pool");
        manager
            .register(RegistrationSpec {
                resource: SharedResident,
                config: BenchCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::<SharedResident>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register resident");
        manager
    });
    let ctx = bench_ctx();
    let options = AcquireOptions::default();

    // Warm one pooled instance so the loop below is a pure idle-hit cycle.
    rt.block_on(async {
        let guard = manager
            .acquire_pooled::<KeepPool>(&ctx, &options)
            .await
            .expect("warm the pool");
        let _release_outcome = guard.release().await.expect("warm release");
    });

    group.bench_function("pooled_hit", |b| {
        b.to_async(&rt).iter(|| async {
            let guard = manager
                .acquire_pooled::<KeepPool>(&ctx, &options)
                .await
                .expect("idle-hit acquire");
            black_box(*guard);
            let _release_outcome = guard.release().await.expect("recycling release");
        });
    });

    group.bench_function("pooled_create_destroy", |b| {
        b.to_async(&rt).iter(|| async {
            let guard = manager
                .acquire_pooled::<DiscardPool>(&ctx, &options)
                .await
                .expect("create-path acquire");
            black_box(*guard);
            let _release_outcome = guard.release().await.expect("discarding release");
        });
    });

    group.bench_function("resident_hit", |b| {
        b.to_async(&rt).iter(|| async {
            let guard = manager
                .acquire_resident::<SharedResident>(&ctx, &options)
                .await
                .expect("resident acquire");
            black_box(*guard);
            drop(guard);
        });
    });

    group.finish();
}

/// A resident resource with one credential slot implementing the projection
/// port, bound to material 1 at use revision 1.
#[derive(Clone)]
struct BoundResident {
    db: std::sync::Arc<nebula_resource::SlotCell<nebula_credential::CredentialGuard<u64>>>,
}

impl BoundResident {
    fn bound() -> Self {
        let db = std::sync::Arc::new(nebula_resource::SlotCell::empty());
        let metadata = nebula_credential::CredentialGuardMetadata::new(
            nebula_core::CredentialId::new(),
            "oauth".parse().expect("credential key"),
            1,
            1,
        )
        .with_admission_epoch(1)
        .with_scope(nebula_credential::TenantScope::new("org", "workspace"));
        let _installed = db
            .install_projected(
                metadata,
                std::sync::Arc::new(nebula_credential::CredentialGuard::new(1_u64)),
            )
            .expect("bind");
        Self { db }
    }
}

#[async_trait::async_trait]
impl Provider for BoundResident {
    type Config = BenchCfg;
    type Instance = u64;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("bench-bound-resident")
    }

    async fn create(&self, _config: &BenchCfg, _ctx: &ResourceContext) -> Result<u64, Error> {
        Ok(0xCAFE)
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("bench-bound-resident"),
            "",
        )
    }
}

impl nebula_resource::HasCredentialSlots for BoundResident {
    fn credential_slot_epoch(&self) -> u64 {
        self.db.generation()
    }

    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &["db"]
    }

    fn supports_credential_slot_projection(&self, slot: &str) -> bool {
        slot == "db"
    }

    fn credential_slot_projection(
        &self,
        slot: &str,
    ) -> Option<(u64, Option<nebula_credential::CredentialGuardMetadata>)> {
        (slot == "db").then(|| self.db.projection_snapshot())
    }
}

impl nebula_resource::PinSlots for BoundResident {
    type Pinned = Option<std::sync::Arc<nebula_credential::CredentialGuard<u64>>>;

    fn pin_slots(&self) -> Self::Pinned {
        self.db.load()
    }
}

#[async_trait::async_trait]
impl ResidentProvider for BoundResident {
    fn is_alive_sync(&self, _instance: &u64) -> bool {
        true
    }
}

/// An in-memory observer answering "available at the installed material"
/// at once: what remains is the framework's cost around the read.
struct AvailableObserver;

impl nebula_credential::CredentialAvailabilityObserver for AvailableObserver {
    fn observe_availability<'a>(
        &'a self,
        _scope: &'a nebula_credential::TenantScope,
        _credential_id: nebula_core::CredentialId,
        _expected_key: nebula_credential::CredentialKey,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        nebula_credential::CredentialAvailabilityObservation,
                        nebula_credential::CredentialObserveError,
                    >,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async {
            Ok(nebula_credential::CredentialAvailabilityObservation::new(
                1,
                1,
                nebula_credential::CredentialAvailability::Available,
            )
            .with_admission_epoch(1))
        })
    }
}

/// `resident_bound_interim` vs `resident_bound_strict`: the same bound
/// resident acquire on an interim manager (no read) and on a strict one (a
/// read through an in-memory observer that answers at once). The difference
/// is the strict framework overhead beyond the read itself.
fn bench_strict_credential_admission(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("bench runtime");
    let mut group = c.benchmark_group("resource/acquire");
    let register = |manager: &Manager| {
        manager
            .register(RegistrationSpec {
                resource: BoundResident::bound(),
                config: BenchCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::<BoundResident>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register bound resident");
    };
    let (interim, strict) = rt.block_on(async {
        let interim = Manager::new();
        register(&interim);
        let strict = Manager::with_config(
            nebula_resource::ManagerConfig::default()
                .with_credential_observer(std::sync::Arc::new(AvailableObserver)),
        );
        register(&strict);
        (interim, strict)
    });
    let ctx = bench_ctx();
    let options = AcquireOptions::default();

    for (name, manager) in [
        ("resident_bound_interim", &interim),
        ("resident_bound_strict", &strict),
    ] {
        group.bench_function(name, |b| {
            b.to_async(&rt).iter(|| async {
                let guard = manager
                    .acquire_resident::<BoundResident>(&ctx, &options)
                    .await
                    .expect("bound resident acquire");
                black_box(*guard);
                drop(guard);
            });
        });
    }

    group.finish();
}

/// One free answered call on the bound resident.
#[derive(serde::Serialize, serde::Deserialize)]
struct OneAttempt;

impl nebula_resource::call::Operation<BoundResident> for OneAttempt {
    type Output = u64;
    const KEY: &'static str = "bench.one_attempt";

    async fn run(
        self,
        cx: &mut nebula_resource::call::OperationCx<'_, BoundResident>,
    ) -> Result<u64, nebula_resource::call::OperationError> {
        cx.call(nebula_resource::call::Cost::FREE, async |instance, _| {
            Ok(*instance)
        })
        .await
    }
}

/// `attempt_bound_interim` vs `attempt_bound_strict`: one managed unit of
/// one free attempt on a row handle of a warm resident, on an interim
/// manager (lock-free registration, no read) and on a strict one (a read
/// through an in-memory observer that answers at once, then registration
/// under `Manager.admission`). The difference is the per-attempt strict
/// overhead beyond the read itself; the unit's own task spawn and its
/// per-attempt checkout are common to both.
fn bench_strict_attempt_admission(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("bench runtime");
    let mut group = c.benchmark_group("resource/attempt");
    let facade = |manager: Manager| async move {
        manager
            .register(RegistrationSpec {
                resource: BoundResident::bound(),
                config: BenchCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::<BoundResident>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register bound resident");
        let managed = manager
            .handle::<BoundResident>(&bench_ctx())
            .expect("bound resident handle");
        // Warm the resident so the measured attempts never create it.
        managed.submit(OneAttempt).await.expect("warm the resident");
        (manager, managed)
    };
    let (interim, strict) = rt.block_on(async {
        let interim = facade(Manager::new()).await;
        let strict = facade(Manager::with_config(
            nebula_resource::ManagerConfig::default()
                .with_credential_observer(std::sync::Arc::new(AvailableObserver)),
        ))
        .await;
        (interim, strict)
    });

    for (name, (_manager, managed)) in [
        ("attempt_bound_interim", &interim),
        ("attempt_bound_strict", &strict),
    ] {
        group.bench_function(name, |b| {
            b.to_async(&rt).iter(|| async {
                let output = managed.submit(OneAttempt).await.expect("granted");
                black_box(output);
            });
        });
    }

    group.finish();
}

/// One free answered call on the keep-pool.
#[derive(serde::Serialize, serde::Deserialize)]
struct PooledAttempt;

impl nebula_resource::call::Operation<KeepPool> for PooledAttempt {
    type Output = u64;
    const KEY: &'static str = "bench.pooled_attempt";

    async fn run(
        self,
        cx: &mut nebula_resource::call::OperationCx<'_, KeepPool>,
    ) -> Result<u64, nebula_resource::call::OperationError> {
        cx.call(nebula_resource::call::Cost::FREE, async |instance, ()| {
            Ok(*instance)
        })
        .await
    }
}

/// `row_pooled_attempt`: one managed unit of one free attempt on a
/// one-connection pool through a row facade: each attempt passes the row
/// gate, runs the acquire pipeline's admission — `Manager.admission` held
/// for lock #1 only — checks out the idle connection and releases it when
/// the attempt ends. Compare with `resource/acquire/pooled_hit` for the
/// facade's overhead over a bare host checkout.
fn bench_row_attempt(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("bench runtime");
    let mut group = c.benchmark_group("resource/attempt");
    let (row_manager, row) = rt.block_on(async {
        let row_manager = Manager::new();
        row_manager
            .register(RegistrationSpec {
                resource: KeepPool,
                config: BenchCfg,
                scope: ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Pooled::<KeepPool>::new(
                    PoolConfig {
                        min_size: 0,
                        max_size: 1,
                        ..PoolConfig::default()
                    },
                    0,
                ),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register keep pool");
        let row = row_manager
            .handle::<KeepPool>(&bench_ctx())
            .expect("row facade");
        // Warm the row's one connection into the idle queue.
        row.submit(PooledAttempt).await.expect("warm the row");
        (row_manager, row)
    });

    group.bench_function("row_pooled_attempt", |b| {
        b.to_async(&rt).iter(|| async {
            black_box(row.submit(PooledAttempt).await.expect("granted"));
        });
    });
    group.finish();
    drop((row, row_manager));
}

criterion_group!(
    benches,
    bench_acquire,
    bench_strict_credential_admission,
    bench_strict_attempt_admission,
    bench_row_attempt
);
criterion_main!(benches);
