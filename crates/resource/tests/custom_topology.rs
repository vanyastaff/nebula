//! Round-trip test for a custom `impl Topology`.
//!
//! Verifies that an author-defined topology:
//! - compiles and satisfies the `Topology` trait contract,
//! - correctly routes `try_reserve` → `acquire` → `on_release`,
//! - receives the revoke-epoch fence for free via `InstanceStore`.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Semaphore;

use nebula_resource::error::Error;
use nebula_resource::topology::{
    AdmissionPhase, InstanceStore, Lease, Load, ReturnOutcome, Ticket, Topology, Unavailable,
};

// ─── Custom topology ─────────────────────────────────────────────────────────

/// A minimal permit-only topology: a fixed concurrency cap backed by a
/// `tokio::Semaphore`. Slot = `()` — the permit IS the lease; no stored
/// instance.
struct PermitPool {
    sem: Arc<Semaphore>,
    cap: usize,
}

impl PermitPool {
    fn new(cap: usize) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(cap)),
            cap,
        }
    }
}

#[async_trait]
impl Topology for PermitPool {
    type Slot = ();

    fn try_reserve(&self, _store: &InstanceStore<()>) -> Result<Ticket<()>, Unavailable> {
        self.sem
            .clone()
            .try_acquire_owned()
            .map(Ticket::permit)
            .map_err(|_| Unavailable::Saturated { retry_after: None })
    }

    async fn acquire(
        &self,
        ticket: Ticket<()>,
        _store: &InstanceStore<()>,
    ) -> Result<Lease<()>, Error> {
        let (_, permit) = ticket.take_slot();
        Ok(Lease::new((), 0, permit))
    }

    fn phase(&self, _store: &InstanceStore<()>) -> AdmissionPhase {
        if self.sem.available_permits() == 0 {
            AdmissionPhase::Saturated
        } else {
            AdmissionPhase::Ready
        }
    }

    fn load(&self, _store: &InstanceStore<()>) -> Option<Load> {
        let available = self.sem.available_permits();
        Some(Load::permits(self.cap - available, self.cap))
    }
}

// ─── A slot-storing custom topology ─────────────────────────────────────────

/// A topology that stores `u32` slots in an `InstanceStore` and checks out
/// idle slots on `try_reserve`, demonstrating that the revoke-epoch fence runs
/// automatically on every `return_slot` path.
struct SlotPool {
    sem: Arc<Semaphore>,
    cap: usize,
}

impl SlotPool {
    fn new(cap: usize) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(cap)),
            cap,
        }
    }
}

#[async_trait]
impl Topology for SlotPool {
    type Slot = u32;

    fn try_reserve(&self, store: &InstanceStore<u32>) -> Result<Ticket<u32>, Unavailable> {
        // Try idle first, then semaphore.
        // NOTE: `checkout` is async; this sync method cannot await it, so we
        // fall through to a permit-only ticket and let `acquire` handle
        // checkout.  This is intentional: `try_reserve` is sync; idle checkout
        // is an async operation deferred to `acquire`.
        self.sem
            .clone()
            .try_acquire_owned()
            .map(|p| {
                // We can't await here, but we expose the store's epoch so the
                // caller can stamp fresh slots.
                let _ = store.current_revoke_epoch(); // advisory read
                Ticket::permit(p)
            })
            .map_err(|_| Unavailable::Saturated { retry_after: None })
    }

    async fn acquire(
        &self,
        ticket: Ticket<u32>,
        store: &InstanceStore<u32>,
    ) -> Result<Lease<u32>, Error> {
        // Try to pop a fresh idle slot. The fence discards any stale-epoch
        // slot on checkout (returned in `stale`); a real framework pipeline
        // would destroy those, but this permit-only test topology drops them.
        let checkout = store.checkout().await;
        if let Some(checked_out) = checkout.fresh {
            let (slot, checkout_epoch) = checked_out.into_parts();
            return Ok(Lease::new(slot, checkout_epoch, None));
        }
        // No idle slot — create a new one stamped with the current epoch.
        let epoch = store.stamp_epoch();
        let (_, permit) = ticket.take_slot();
        Ok(Lease::new(42u32, epoch, permit))
    }

    async fn on_release(&self, _slot: &mut u32) -> Result<(), Error> {
        Ok(())
    }

    fn phase(&self, _store: &InstanceStore<u32>) -> AdmissionPhase {
        if self.sem.available_permits() == 0 {
            AdmissionPhase::Saturated
        } else {
            AdmissionPhase::Ready
        }
    }

    fn load(&self, _store: &InstanceStore<u32>) -> Option<Load> {
        let available = self.sem.available_permits();
        Some(Load::permits(self.cap - available, self.cap))
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// A permit-only custom topology compiles, reserves, acquires, and releases.
#[tokio::test]
async fn permit_only_topology_round_trip() {
    let store: InstanceStore<()> = InstanceStore::new(None);
    let topo = PermitPool::new(2);

    // Phase is Ready when permits available.
    assert_eq!(topo.phase(&store), AdmissionPhase::Ready);

    // Reserve + acquire two leases.
    let ticket1 = topo.try_reserve(&store).expect("first ticket");
    let ticket2 = topo.try_reserve(&store).expect("second ticket");

    // Saturated after two acquires.
    let third = topo.try_reserve(&store);
    assert!(
        matches!(third, Err(Unavailable::Saturated { retry_after: None })),
        "pool of 2 is saturated after 2 reservations"
    );
    assert_eq!(topo.phase(&store), AdmissionPhase::Saturated);

    let lease1 = topo.acquire(ticket1, &store).await.expect("acquire 1");
    let lease2 = topo.acquire(ticket2, &store).await.expect("acquire 2");

    // Load reflects 2/2 used.
    let load = topo.load(&store).expect("load present");
    assert!(
        (load.saturation - 1.0_f32).abs() < f32::EPSILON,
        "saturation must be 1.0 when fully occupied"
    );

    // Drop permits → capacity freed.
    drop(lease1.permit);
    drop(lease2.permit);

    assert_eq!(
        topo.phase(&store),
        AdmissionPhase::Ready,
        "phase returns to Ready after permits released"
    );
}

/// Slot-storing custom topology: the revoke-epoch fence runs on `return_slot`
/// and evicts a slot whose checkout epoch is behind the live counter.
#[tokio::test]
async fn slot_topology_revoke_fence_via_instance_store() {
    let store: InstanceStore<u32> = InstanceStore::new(Some(4));
    let topo = SlotPool::new(4);

    // Acquire a slot — stamped with epoch 0.
    let ticket = topo.try_reserve(&store).expect("ticket");
    let lease = topo.acquire(ticket, &store).await.expect("lease");
    assert_eq!(lease.slot, 42u32);
    let checkout_epoch = lease.checkout_epoch;

    // Simulate credential revoke: advance epoch.
    store.bump_revoke_epoch();

    // Return the slot — checkout_epoch (0) < live epoch (1) → must evict.
    let outcome = store.return_slot(lease.slot, checkout_epoch).await;
    assert_eq!(
        outcome,
        ReturnOutcome::Evict,
        "slot checked out before revoke must be evicted by the uniform fence"
    );
    assert!(
        store.is_empty().await,
        "evicted slot must not appear in idle queue"
    );
}

/// Slot-storing custom topology: a slot returned at the current epoch is recycled.
#[tokio::test]
async fn slot_topology_clean_return_is_recycled() {
    let store: InstanceStore<u32> = InstanceStore::new(Some(4));
    let topo = SlotPool::new(4);

    let ticket = topo.try_reserve(&store).expect("ticket");
    let lease = topo.acquire(ticket, &store).await.expect("lease");

    // Return without a revoke — same epoch.
    let outcome = store.return_slot(lease.slot, lease.checkout_epoch).await;
    assert_eq!(
        outcome,
        ReturnOutcome::Recycled,
        "slot returned at current epoch must be recycled"
    );
    assert_eq!(store.len().await, 1, "idle queue holds the recycled slot");
}

/// `on_release` default (no-op) is inherited and doesn't error.
#[tokio::test]
async fn custom_topology_on_release_default_ok() {
    let store: InstanceStore<()> = InstanceStore::new(None);
    let topo = PermitPool::new(1);

    let ticket = topo.try_reserve(&store).expect("ticket");
    let mut lease = topo.acquire(ticket, &store).await.expect("lease");
    // on_release default should succeed without any slot cleanup.
    topo.on_release(&mut lease.slot)
        .await
        .expect("on_release ok");
}

/// `Unavailable` variants implement `PartialEq` via `#[derive]`; verify
/// `Saturated` equality used in tests above works as expected.
#[test]
fn unavailable_saturated_equality() {
    let a = Unavailable::Saturated { retry_after: None };
    let b = Unavailable::Saturated { retry_after: None };
    assert_eq!(a, b);
    assert_ne!(
        Unavailable::Saturated { retry_after: None },
        Unavailable::Warming
    );
}
