//! Standalone shape test for a custom `impl Topology<R>` under the inverted,
//! entry-centric contract.
//!
//! Verifies that an author-defined topology:
//! - compiles and satisfies the `Topology<R>` trait contract with only the thin
//!   entry-centric hooks (`try_reserve` / `create_entry` / `entry_instance` /
//!   `into_instance` / `pools` / `store_capacity`);
//! - drives `try_reserve` admission (Saturated when the semaphore is exhausted);
//! - produces entries via `create_entry` that project + consume cleanly;
//! - gets the revoke-epoch fence **for free** via the framework-owned
//!   `InstanceStore` (the topology writes no fence code).
//!
//! The end-to-end Manager-driven safety proof lives in
//! `custom_topology_manager.rs`; this file pins the standalone hook shape.

use std::sync::Arc;

use async_trait::async_trait;
use nebula_core::{ResourceKey, resource_key};
use nebula_resource::error::Error;
use nebula_resource::resource::{Provider, ResourceConfig, ResourceMetadata};
use nebula_resource::topology::{
    AdmissionPhase, InstanceStore, ReturnOutcome, Ticket, Topology, Unavailable,
};
use nebula_resource::{ResourceContext, TopologyTag};
use tokio::sync::Semaphore;

// ─── A minimal resource to parameterize the custom topology ──────────────────

#[derive(Clone, Default)]
struct PermitCfg;
nebula_resource::impl_empty_has_schema!(PermitCfg);
impl ResourceConfig for PermitCfg {
    fn fingerprint(&self) -> u64 {
        0
    }
}

#[derive(Clone)]
struct PermitRes;

#[async_trait]
impl Provider for PermitRes {
    type Config = PermitCfg;
    type Instance = u32;
    type Topology = EntryPool;

    fn key() -> ResourceKey {
        resource_key!("custom.standalone.permit")
    }

    async fn create(&self, _config: &PermitCfg, _ctx: &ResourceContext) -> Result<u32, Error> {
        Ok(42)
    }

    async fn destroy(&self, _runtime: u32, _cx: nebula_resource::TeardownCx) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

nebula_resource::no_credential_slots!(PermitRes);

// ─── An entry-storing custom topology over the framework store ─────────────────

/// A bespoke pool whose `Entry = u32`. It supplies only the entry-centric hooks;
/// the framework owns the idle store, the checkout, and the revoke fence. The
/// topology holds only a semaphore + capacity — no `InstanceStore`.
struct EntryPool {
    sem: Arc<Semaphore>,
    cap: usize,
}

impl EntryPool {
    fn new(cap: usize) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(cap)),
            cap,
        }
    }
}

#[async_trait]
impl Topology<PermitRes> for EntryPool {
    type Entry = u32;

    fn try_reserve(&self, _store: &InstanceStore<u32>) -> Result<Ticket, Unavailable> {
        self.sem
            .clone()
            .try_acquire_owned()
            .map(Ticket::permit)
            .map_err(|_| Unavailable::Saturated { retry_after: None })
    }

    async fn create_entry(
        &self,
        resource: &PermitRes,
        config: &PermitCfg,
        ctx: &ResourceContext,
    ) -> Result<u32, Error> {
        resource.create(config, ctx).await
    }

    fn entry_instance<'s>(&self, entry: &'s u32) -> &'s u32 {
        entry
    }

    fn into_instance(&self, entry: u32) -> u32 {
        entry
    }

    fn pools(&self) -> bool {
        true
    }

    fn store_capacity(&self) -> Option<usize> {
        Some(self.cap)
    }

    fn phase(&self, _store: &InstanceStore<u32>) -> AdmissionPhase {
        if self.sem.available_permits() == 0 {
            AdmissionPhase::Saturated
        } else {
            AdmissionPhase::Ready
        }
    }

    fn tag(&self) -> TopologyTag {
        TopologyTag::Custom
    }
}

fn test_ctx() -> ResourceContext {
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// `try_reserve` grants tickets up to capacity, then reports Saturated; `phase`
/// tracks it. Dropping a ticket frees capacity again.
#[tokio::test]
async fn try_reserve_admission_and_phase() {
    let store: InstanceStore<u32> = InstanceStore::new(Some(2));
    let topo = EntryPool::new(2);

    assert_eq!(
        Topology::<PermitRes>::phase(&topo, &store),
        AdmissionPhase::Ready
    );
    let t1 = Topology::<PermitRes>::try_reserve(&topo, &store).expect("first ticket");
    let t2 = Topology::<PermitRes>::try_reserve(&topo, &store).expect("second ticket");
    assert!(
        matches!(
            Topology::<PermitRes>::try_reserve(&topo, &store),
            Err(Unavailable::Saturated { .. })
        ),
        "a pool of 2 is saturated after 2 tickets"
    );
    assert_eq!(
        Topology::<PermitRes>::phase(&topo, &store),
        AdmissionPhase::Saturated
    );

    drop(t1);
    drop(t2);
    assert_eq!(
        Topology::<PermitRes>::phase(&topo, &store),
        AdmissionPhase::Ready,
        "phase returns to Ready after the permits are released"
    );
}

/// `create_entry` builds an entry; `entry_instance` / `into_instance` project and
/// consume it cleanly.
#[tokio::test]
async fn create_entry_and_projections() {
    let topo = EntryPool::new(2);
    let resource = PermitRes;
    let entry = topo
        .create_entry(&resource, &PermitCfg, &test_ctx())
        .await
        .expect("create_entry");
    assert_eq!(*topo.entry_instance(&entry), 42);
    assert_eq!(topo.into_instance(entry), 42);
    assert!(Topology::<PermitRes>::pools(&topo));
    assert_eq!(Topology::<PermitRes>::store_capacity(&topo), Some(2));
}

/// The revoke-epoch fence runs on the **framework** `InstanceStore`, not in the
/// topology: an entry returned at the pre-bump epoch is evicted on return after a
/// bump. The custom topology writes no fence code — it gets this for free.
#[tokio::test]
async fn entry_revoke_fence_via_framework_store() {
    let store: InstanceStore<u32> = InstanceStore::new(Some(4));

    // An entry goes idle at epoch 0.
    let epoch = store.stamp_epoch();
    assert_eq!(
        store.return_entry(7u32, epoch).await,
        ReturnOutcome::Recycled
    );

    // Credential revoke: advance the epoch.
    store.bump_revoke_epoch();

    // Return another entry stamped at epoch 0 — the framework fence evicts it.
    let outcome = store.return_entry(9u32, epoch).await;
    assert!(
        outcome.is_evict(),
        "an entry checked out before a revoke must be evicted by the framework \
         store fence — the custom topology writes no fence code"
    );

    // The first (already-idle, pre-revoke) entry is evicted on checkout.
    let checkout = store.checkout().await;
    assert!(
        checkout.fresh.is_none(),
        "an entry idle since before the revoke must never be handed out as fresh"
    );
    assert_eq!(
        checkout.stale,
        vec![7u32],
        "the framework collects the since-revoked idle entry for destruction"
    );
}

/// `Unavailable::Saturated` `PartialEq` sanity (used by the assertions above).
#[test]
fn unavailable_saturated_equality() {
    assert_eq!(
        Unavailable::Saturated { retry_after: None },
        Unavailable::Saturated { retry_after: None }
    );
    assert_ne!(
        Unavailable::Saturated { retry_after: None },
        Unavailable::Warming
    );
}
