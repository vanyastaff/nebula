//! Release-queue isolation between resources of one manager.
//!
//! Every resource of a manager shares one [`ReleaseQueue`], and a release
//! job carries its pool permit until it settles. A teardown that waits on a
//! dead backend must therefore cost its own resource a slot, never another
//! resource its capacity.
//!
//! [`ReleaseQueue`]: nebula_resource::ReleaseQueue

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use nebula_core::ScopeLevel;
use nebula_resource::{
    AcquireOptions, Error, ErrorKind, Manager, ManagerConfig, PoolProvider, Pooled, Provider,
    RegistrationSpec, Resource, ResourceContext, ResourceGuard, ResourceMetadata, SlotIdentity,
    TeardownCx,
    topology::pooled::{BrokenCheck, config::Config as PoolConfig},
};

/// How long a `Stuck` instance takes to tear down: long, but inside the
/// queue's per-job ceiling, so the job is waiting rather than aborted.
const STUCK_TEARDOWN: Duration = Duration::from_secs(30);

/// A pooled resource whose every instance comes back broken and whose
/// `destroy` waits on a backend that does not answer.
#[derive(Resource, Clone)]
struct Stuck {
    destroys_started: Arc<AtomicUsize>,
    destroys_finished: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider for Stuck {
    type Config = ();
    type Instance = ();
    const KEY: &'static str = "isolation.stuck";
    type Topology = Pooled<Self>;

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::new(nebula_resource::metadata_name!("stuck"), "")
    }

    async fn create(&self, _config: &(), _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }

    async fn destroy(&self, _instance: (), _cx: TeardownCx) -> Result<(), Error> {
        self.destroys_started.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(STUCK_TEARDOWN).await;
        self.destroys_finished.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

impl PoolProvider for Stuck {
    fn is_broken(&self, _instance: &()) -> BrokenCheck {
        BrokenCheck::Broken("the backend is gone".into())
    }
}

/// A healthy pooled resource with nothing to wait for on release.
#[derive(Resource, Clone)]
struct Healthy;

#[async_trait]
impl Provider for Healthy {
    type Config = ();
    type Instance = ();
    const KEY: &'static str = "isolation.healthy";
    type Topology = Pooled<Self>;

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::new(nebula_resource::metadata_name!("healthy"), "")
    }

    async fn create(&self, _config: &(), _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

impl PoolProvider for Healthy {}

fn pool<R>(resource: R, max_size: u32) -> RegistrationSpec<R>
where
    R: Provider<Config = (), Topology = Pooled<R>> + PoolProvider,
{
    RegistrationSpec {
        resource,
        config: (),
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: Pooled::new(
            PoolConfig {
                max_size,
                ..Default::default()
            },
            0,
        ),
        recovery_gate: None,
        rate_limit: None,
    }
}

fn ctx() -> ResourceContext {
    ResourceContext::minimal(
        nebula_core::scope::Scope::default(),
        tokio_util::sync::CancellationToken::new(),
    )
}

/// Acquires the one-slot `Healthy` pool, retrying a `Backpressure` refusal
/// the way a caller would, and reports how long the caller waited.
async fn acquire_healthy(manager: &Manager) -> (ResourceGuard<Healthy>, Duration) {
    let started = tokio::time::Instant::now();
    loop {
        match manager
            .acquire::<Healthy>(&ctx(), &AcquireOptions::default())
            .await
        {
            Ok(guard) => return (guard, started.elapsed()),
            Err(error) => {
                assert_eq!(
                    error.kind(),
                    &ErrorKind::Backpressure,
                    "a healthy pool only ever refuses for capacity"
                );
                assert!(
                    started.elapsed() < STUCK_TEARDOWN,
                    "the healthy pool waited out another resource's teardown"
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            },
        }
    }
}

/// The reproduction from the 2026-11 audit: with the release queue's workers
/// awaiting one job at a time, the first hung `destroy` kept the healthy
/// pool's permit queued behind it for the whole teardown.
#[tokio::test(start_paused = true)]
async fn a_hung_teardown_does_not_hold_another_resources_capacity() {
    const STUCK_POOL: u32 = 4;
    // One worker per lane: the tightest configuration an operator can pick.
    let manager = Manager::with_config(ManagerConfig::default().with_release_queue_workers(1));
    let stuck = Stuck {
        destroys_started: Arc::default(),
        destroys_finished: Arc::default(),
    };
    manager.register(pool(stuck.clone(), STUCK_POOL)).unwrap();
    manager.register(pool(Healthy, 1)).unwrap();

    let mut leases = Vec::new();
    for _ in 0..STUCK_POOL {
        leases.push(
            manager
                .acquire::<Stuck>(&ctx(), &AcquireOptions::default())
                .await
                .unwrap(),
        );
    }
    // Every instance is broken, so each release becomes a hung `destroy`.
    drop(leases);

    for round in 0..6 {
        let (guard, waited) = acquire_healthy(&manager).await;
        assert!(
            waited < Duration::from_secs(1),
            "round {round}: the healthy pool's only slot came back after {waited:?}"
        );
        drop(guard);
    }

    assert_eq!(
        stuck.destroys_started.load(Ordering::SeqCst),
        STUCK_POOL as usize,
        "every stuck teardown was running while the healthy pool cycled"
    );
    assert_eq!(
        stuck.destroys_finished.load(Ordering::SeqCst),
        0,
        "none of them had finished: the healthy pool did not wait for one"
    );
}
