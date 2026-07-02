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

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use nebula_resource::{
    AcquireOptions, Error, HasCredentialSlots, Manager, PoolConfig, Pooled, Provider,
    RegistrationSpec, Resident, ResidentConfig, ResourceConfig, ResourceContext, ResourceKey,
    ScopeLevel, SlotIdentity,
    resource::ResourceMetadata,
    resource_key,
    topology::pooled::{PoolProvider, RecycleDecision},
    topology::resident::ResidentProvider,
};

#[derive(Clone)]
struct BenchCfg;

nebula_schema::impl_empty_has_schema!(BenchCfg);

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

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl HasCredentialSlots for KeepPool {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

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

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl HasCredentialSlots for DiscardPool {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

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

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl HasCredentialSlots for SharedResident {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

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
        guard.release().await.expect("warm release");
    });

    group.bench_function("pooled_hit", |b| {
        b.to_async(&rt).iter(|| async {
            let guard = manager
                .acquire_pooled::<KeepPool>(&ctx, &options)
                .await
                .expect("idle-hit acquire");
            black_box(*guard);
            guard.release().await.expect("recycling release");
        });
    });

    group.bench_function("pooled_create_destroy", |b| {
        b.to_async(&rt).iter(|| async {
            let guard = manager
                .acquire_pooled::<DiscardPool>(&ctx, &options)
                .await
                .expect("create-path acquire");
            black_box(*guard);
            guard.release().await.expect("discarding release");
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

criterion_group!(benches, bench_acquire);
criterion_main!(benches);
