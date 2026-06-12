//! End-to-end proof that the built-in [`Bounded`] topology works through the
//! real `Manager::register()` → `Manager::acquire_any` → guard-drop release
//! pipeline (not just the standalone topology-surface unit tests).
//!
//! Three modes, one runtime-valued cap (no const generic):
//! - `Capped(n)` gates concurrency at `n` and builds a fresh instance per lease
//!   (destroyed on release — no idle reuse);
//! - `Exclusive` serialises to one reused instance, reset between leases;
//! - `Unbounded` never rejects.
//!
//! [`Bounded`]: nebula_resource::Bounded

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_resource::{
    AcquireOptions, Bounded, Manager, RegistrationSpec, ResourceContext, SlotIdentity, TopologyTag,
    error::Error,
    resource::{HasCredentialSlots, Provider, ResourceConfig, ResourceMetadata},
    topology::bounded::BoundedProvider,
};
use tokio_util::sync::CancellationToken;

// ─── The resource ──────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct SeatCfg;
nebula_resource::impl_empty_has_schema!(SeatCfg);
impl ResourceConfig for SeatCfg {
    fn fingerprint(&self) -> u64 {
        0
    }
}

/// A leased "seat" carrying a unique id.
#[derive(Clone)]
struct Seat(
    #[allow(
        dead_code,
        reason = "id carried by the handle; counters are the observable"
    )]
    u64,
);

#[derive(Clone)]
struct Seats {
    create_count: Arc<AtomicU64>,
    destroy_count: Arc<AtomicU64>,
    reset_count: Arc<AtomicU64>,
    reset_ok: Arc<AtomicBool>,
}

impl Seats {
    fn new() -> Self {
        Self {
            create_count: Arc::new(AtomicU64::new(0)),
            destroy_count: Arc::new(AtomicU64::new(0)),
            reset_count: Arc::new(AtomicU64::new(0)),
            reset_ok: Arc::new(AtomicBool::new(true)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Seats {
    type Config = SeatCfg;
    type Instance = Seat;
    type Topology = Bounded<Seats>;

    fn key() -> ResourceKey {
        resource_key!("bounded.seats")
    }

    async fn create(&self, _config: &SeatCfg, _ctx: &ResourceContext) -> Result<Seat, Error> {
        let id = self.create_count.fetch_add(1, Ordering::SeqCst);
        Ok(Seat(id))
    }

    async fn destroy(
        &self,
        _instance: Seat,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        self.destroy_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl HasCredentialSlots for Seats {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

#[async_trait::async_trait]
impl BoundedProvider for Seats {
    async fn reset(&self, _instance: &mut Seat) -> Result<(), Error> {
        self.reset_count.fetch_add(1, Ordering::SeqCst);
        if self.reset_ok.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(Error::transient("seat reset failed"))
        }
    }
}

// ─── Harness ────────────────────────────────────────────────────────────────

fn ctx() -> ResourceContext {
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

fn register(manager: &Manager, seats: Seats, topology: Bounded<Seats>) {
    let spec = RegistrationSpec {
        resource: seats,
        config: SeatCfg,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology,
        recovery_gate: None,
    };
    manager
        .register(spec)
        .expect("a bounded resource must register through Manager::register");
}

async fn acquire(
    manager: &Arc<Manager>,
    key: &ResourceKey,
    ctx: &ResourceContext,
) -> Result<nebula_resource::guard::ResourceGuard<Seats>, Error> {
    let boxed = Manager::acquire_any(
        Arc::clone(manager),
        key,
        ctx,
        &AcquireOptions::default(),
        &SlotIdentity::Unbound,
    )
    .await?;
    Ok(*boxed
        .downcast::<nebula_resource::guard::ResourceGuard<Seats>>()
        .expect("downcast to the typed guard"))
}

async fn poll_until(deadline: std::time::Duration, mut cond: impl FnMut() -> bool) -> bool {
    let start = std::time::Instant::now();
    loop {
        if cond() {
            return true;
        }
        if start.elapsed() >= deadline {
            return cond();
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────

/// `Capped(2)` admits two concurrent leases and rejects the third with
/// backpressure; releasing one re-admits. The cap is read from a runtime value.
#[tokio::test]
async fn capped_gates_concurrency_through_manager() {
    let manager = Arc::new(Manager::new());
    let seats = Seats::new();
    register(&manager, seats, Bounded::capped(2).expect("cap >= 1"));
    let key = Seats::key();
    let ctx = ctx();

    let g1 = acquire(&manager, &key, &ctx).await.expect("first lease");
    let g2 = acquire(&manager, &key, &ctx).await.expect("second lease");
    assert_eq!(g1.topology_tag(), TopologyTag::Bounded);

    // The third lease exceeds the cap of 2 — rejected, not blocked forever.
    assert!(
        acquire(&manager, &key, &ctx).await.is_err(),
        "a third concurrent lease exceeds Capped(2) and must be rejected"
    );

    // Release one; the permit frees synchronously on guard drop and the next
    // lease admits.
    drop(g1);
    let g3 = acquire(&manager, &key, &ctx)
        .await
        .expect("a freed permit re-admits");
    drop((g2, g3));
}

/// `Exclusive` serialises to one lease and reuses its single instance, resetting
/// it between leases — sequential acquires create the instance once and reset on
/// each release.
#[tokio::test]
async fn exclusive_reuses_one_instance_and_resets() {
    let manager = Arc::new(Manager::new());
    let seats = Seats::new();
    let create_count = Arc::clone(&seats.create_count);
    let reset_count = Arc::clone(&seats.reset_count);
    register(&manager, seats, Bounded::exclusive());
    let key = Seats::key();
    let ctx = ctx();

    // Lease, release (resets + returns the one instance to the store), lease
    // again (reuses it). Each release must run reset and return the slot before
    // the next lease can reuse it, so wait for the recycle between acquires.
    for round in 0..3u64 {
        let g = acquire(&manager, &key, &ctx)
            .await
            .expect("exclusive lease");
        assert_eq!(g.topology_tag(), TopologyTag::Bounded);
        drop(g);
        let reset_landed = poll_until(std::time::Duration::from_secs(2), || {
            reset_count.load(Ordering::SeqCst) > round
        })
        .await;
        assert!(reset_landed, "release {round} must run reset before reuse");
    }

    assert_eq!(
        create_count.load(Ordering::SeqCst),
        1,
        "exclusive reuses its one instance across leases — created exactly once"
    );
    assert_eq!(
        reset_count.load(Ordering::SeqCst),
        3,
        "each of the three releases reset the reused instance"
    );
}

/// A failed `reset` on `Exclusive` destroys the instance instead of reusing it
/// (the S4 invariant); the next acquire builds a fresh one.
#[tokio::test]
async fn exclusive_failed_reset_destroys_then_recreates() {
    let manager = Arc::new(Manager::new());
    let seats = Seats::new();
    let create_count = Arc::clone(&seats.create_count);
    let destroy_count = Arc::clone(&seats.destroy_count);
    seats.reset_ok.store(false, Ordering::SeqCst);
    register(&manager, seats, Bounded::exclusive());
    let key = Seats::key();
    let ctx = ctx();

    let g = acquire(&manager, &key, &ctx).await.expect("first lease");
    drop(g);
    // The failed reset must destroy the one instance (never reissue a half-reset
    // one). The next acquire then builds a fresh instance.
    let destroyed = poll_until(std::time::Duration::from_secs(2), || {
        destroy_count.load(Ordering::SeqCst) >= 1
    })
    .await;
    assert!(destroyed, "a failed reset destroys the instance (S4)");

    let g2 = acquire(&manager, &key, &ctx)
        .await
        .expect("a fresh instance is built after the poisoned one was destroyed");
    drop(g2);
    assert_eq!(
        create_count.load(Ordering::SeqCst),
        2,
        "the poisoned instance was not reused — a second was created"
    );
}

/// `Unbounded` never rejects an acquire.
#[tokio::test]
async fn unbounded_never_rejects_through_manager() {
    let manager = Arc::new(Manager::new());
    let seats = Seats::new();
    register(&manager, seats, Bounded::unbounded());
    let key = Seats::key();
    let ctx = ctx();

    let mut guards = Vec::new();
    for _ in 0..8 {
        guards.push(
            acquire(&manager, &key, &ctx)
                .await
                .expect("unbounded admits"),
        );
    }
    assert_eq!(guards.len(), 8);
    assert_eq!(guards[0].topology_tag(), TopologyTag::Bounded);
}
