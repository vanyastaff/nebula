use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use nebula_core::{ExecutionId, ResourceKey, resource_key};

use super::*;
use crate::topology::store::{InstanceStore, StoreView};
use crate::{
    resource::{ResourceConfig, ResourceMetadataDraft},
    topology::bounded::BoundedProvider,
};

#[derive(Debug, Clone, Copy, nebula_schema::Schema)]
struct BoundedCfg;

impl ResourceConfig for BoundedCfg {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        // Unit struct: all instances identical — constant 0 is correct.
        0
    }
}

#[derive(Clone)]
struct MockBounded {
    reset_ok: Arc<AtomicBool>,
    reset_calls: Arc<AtomicUsize>,
}

impl MockBounded {
    fn new() -> Self {
        Self {
            reset_ok: Arc::new(AtomicBool::new(true)),
            reset_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl Provider for MockBounded {
    type Config = BoundedCfg;
    type Instance = u32;
    type Topology = Bounded<Self>;

    fn key() -> ResourceKey {
        resource_key!("mock-bounded")
    }

    async fn create(&self, _config: &BoundedCfg, _ctx: &ResourceContext) -> Result<u32, Error> {
        Ok(7)
    }

    async fn destroy(&self, _instance: u32, _cx: crate::TeardownCx) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("mock-bounded"), "")
    }
}

crate::no_credential_slots!(MockBounded);

impl BoundedProvider for MockBounded {
    async fn reset(&self, _instance: &mut u32) -> Result<(), Error> {
        self.reset_calls.fetch_add(1, Ordering::Relaxed);
        if self.reset_ok.load(Ordering::Relaxed) {
            Ok(())
        } else {
            Err(Error::transient("reset failed"))
        }
    }
}

fn store() -> InstanceStore<u32> {
    InstanceStore::new(None)
}

fn test_ctx() -> ResourceContext {
    use nebula_core::scope::Scope;
    use tokio_util::sync::CancellationToken;
    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

#[test]
fn capped_rejects_zero_cap() {
    assert!(
        Bounded::<MockBounded>::capped(0).is_err(),
        "a zero cap can never admit and must be rejected at construction"
    );
    assert!(Bounded::<MockBounded>::capped(1).is_ok());
}

#[tokio::test]
async fn capped_gate_admits_up_to_n() {
    let topo = Bounded::<MockBounded>::capped(2).expect("cap >= 1");
    let st = store();

    let t1 = topo
        .try_reserve(StoreView::new(&st))
        .expect("first lease admitted");
    let t2 = topo
        .try_reserve(StoreView::new(&st))
        .expect("second lease admitted");
    assert!(
        topo.try_reserve(StoreView::new(&st)).is_err(),
        "third lease exceeds the cap of 2 — must be rejected"
    );
    assert_eq!(topo.phase(StoreView::new(&st)), AdmissionPhase::Saturated);
    assert_eq!(
        topo.load(StoreView::new(&st))
            .expect("capped reports load")
            .saturation,
        1.0
    );

    // Releasing a ticket returns its permit; capacity frees up.
    drop(t1);
    let t3 = topo
        .try_reserve(StoreView::new(&st))
        .expect("a freed permit re-admits");
    drop((t2, t3));
}

#[tokio::test]
async fn exclusive_serialises_to_one() {
    let topo = Bounded::<MockBounded>::exclusive();
    let st = store();

    let held = topo
        .try_reserve(StoreView::new(&st))
        .expect("first exclusive lease");
    assert!(
        topo.try_reserve(StoreView::new(&st)).is_err(),
        "exclusive admits exactly one at a time"
    );
    drop(held);
    let _next = topo
        .try_reserve(StoreView::new(&st))
        .expect("the next lease admits once the first releases");
}

#[tokio::test]
async fn exclusive_resets_and_keeps_on_release() {
    let resource = MockBounded::new();
    let topo = Bounded::<MockBounded>::exclusive();
    let mut entry = 7u32;

    let keep = topo
        .on_release(&mut entry, &resource)
        .await
        .expect("a clean reset keeps the instance");
    assert!(
        keep,
        "exclusive reuses its one instance after a clean reset"
    );
    assert_eq!(resource.reset_calls.load(Ordering::Relaxed), 1);
    assert!(
        topo.pools(),
        "exclusive pools its single reused instance (store cap 1)"
    );
    assert_eq!(topo.store_capacity(), Some(1));
}

#[tokio::test]
async fn exclusive_reset_error_discards_and_surfaces() {
    let resource = MockBounded::new();
    resource.reset_ok.store(false, Ordering::Relaxed);
    let topo = Bounded::<MockBounded>::exclusive();
    let mut entry = 7u32;

    let outcome = topo.on_release(&mut entry, &resource).await;
    assert!(
        outcome.is_err(),
        "a failed reset surfaces the error so the framework destroys the \
         instance (never reissues a half-reset one — S4)"
    );
}

#[tokio::test]
async fn capped_destroys_on_release() {
    let resource = MockBounded::new();
    let topo = Bounded::<MockBounded>::capped(4).expect("cap >= 1");
    let mut entry = 7u32;

    let keep = topo.on_release(&mut entry, &resource).await.expect("ok");
    assert!(
        !keep,
        "capped does not pool — released instances are destroyed"
    );
    assert_eq!(
        resource.reset_calls.load(Ordering::Relaxed),
        0,
        "only exclusive resets; capped never calls reset"
    );
    assert!(!topo.pools());
    assert!(
        topo.handles_own_revoke(),
        "non-pooling bounded keeps no idle credentialed state to leak"
    );
}

#[tokio::test]
async fn unbounded_always_admits() {
    let topo = Bounded::<MockBounded>::unbounded();
    let st = store();

    let held: Vec<_> = (0..64)
        .map(|_| topo.try_reserve(StoreView::new(&st)).ok())
        .collect();
    assert!(
        held.iter().all(Option::is_some),
        "unbounded never rejects a lease"
    );
    assert_eq!(topo.phase(StoreView::new(&st)), AdmissionPhase::Ready);
    assert!(
        topo.load(StoreView::new(&st)).is_none(),
        "unbounded reports no load"
    );
}

#[tokio::test]
async fn set_cap_grows_and_shrinks() {
    let topo = Bounded::<MockBounded>::capped(2).expect("cap >= 1");
    let st = store();

    // Grow 2 → 4: two more leases now fit.
    topo.set_cap(4).expect("grow");
    let leases: Vec<_> = (0..4)
        .map(|_| {
            topo.try_reserve(StoreView::new(&st))
                .expect("4 leases fit after grow")
        })
        .collect();
    assert!(
        topo.try_reserve(StoreView::new(&st)).is_err(),
        "the 5th exceeds the grown cap"
    );
    drop(leases);

    // Shrink 4 → 1 while idle: only one lease fits.
    topo.set_cap(1).expect("shrink while idle");
    let _one = topo
        .try_reserve(StoreView::new(&st))
        .expect("one lease fits");
    assert!(
        topo.try_reserve(StoreView::new(&st)).is_err(),
        "the cap shrank to 1 — a second lease is rejected"
    );
}

#[test]
fn set_cap_rejects_non_capped_and_zero() {
    let exclusive = Bounded::<MockBounded>::exclusive();
    assert!(
        exclusive.set_cap(4).is_err(),
        "exclusive cap is fixed at one"
    );
    let unbounded = Bounded::<MockBounded>::unbounded();
    assert!(unbounded.set_cap(4).is_err(), "unbounded has no cap to set");
    let capped = Bounded::<MockBounded>::capped(2).expect("cap >= 1");
    assert!(capped.set_cap(0).is_err(), "a zero cap is rejected");
}

#[tokio::test]
async fn create_entry_builds_a_fresh_instance() {
    let resource = MockBounded::new();
    let topo = Bounded::<MockBounded>::capped(2).expect("cap >= 1");
    let retained = crate::RetainedStore::for_test();
    let inst = topo
        .create_entry(&resource, &BoundedCfg, &test_ctx(), &retained)
        .await
        .expect("create");
    assert_eq!(*inst.entry(), 7);
    assert!(retained.drain_retired().is_empty());
    assert_eq!(topo.tag(), TopologyTag::Bounded);
}

#[derive(Clone, nebula_schema::Schema)]
struct PanickingFingerprint;

impl ResourceConfig for PanickingFingerprint {
    fn fingerprint(&self) -> u64 {
        panic!("intentional fingerprint panic")
    }
}

#[derive(Clone)]
struct FingerprintResource(Arc<AtomicUsize>);

#[async_trait]
impl Provider for FingerprintResource {
    type Config = PanickingFingerprint;
    type Instance = u32;
    type Topology = Bounded<Self>;

    fn key() -> ResourceKey {
        resource_key!("fingerprint-ownership-regression")
    }
    async fn create(&self, _: &Self::Config, _: &ResourceContext) -> Result<u32, Error> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(1)
    }
    async fn destroy(&self, _: u32, _: crate::TeardownCx) -> Result<(), Error> {
        Ok(())
    }
    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            crate::metadata_name!("fingerprint-ownership-regression"),
            "",
        )
    }
}

crate::no_credential_slots!(FingerprintResource);
impl BoundedProvider for FingerprintResource {}

#[tokio::test]
async fn fingerprint_panic_cannot_leave_a_created_instance_unowned() {
    use futures::FutureExt as _;
    let created = Arc::new(AtomicUsize::new(0));
    let resource = FingerprintResource(Arc::clone(&created));
    let topology = Bounded::<FingerprintResource>::unbounded();
    let retained = crate::RetainedStore::for_test();
    let context = test_ctx();
    let outcome = std::panic::AssertUnwindSafe(topology.create_entry(
        &resource,
        &PanickingFingerprint,
        &context,
        &retained,
    ))
    .catch_unwind()
    .await;
    assert!(outcome.is_err());
    assert_eq!(
        created.load(Ordering::SeqCst),
        0,
        "author metadata must be evaluated before creating an unarmed instance"
    );
}
