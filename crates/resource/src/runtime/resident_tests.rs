use std::{
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
    time::Duration,
};

use nebula_core::{ExecutionId, ResourceKey, resource_key};

use super::*;
use crate::{
    context::ResourceContext,
    resource::{ResourceConfig, ResourceMetadataDraft},
    topology::resident::ResidentProvider,
};

#[derive(Clone)]
struct MockResident {
    alive: Arc<AtomicBool>,
    create_count: Arc<AtomicU32>,
}

impl MockResident {
    fn new() -> Self {
        Self {
            alive: Arc::new(AtomicBool::new(true)),
            create_count: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl ResourceConfig for bool {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
}

#[async_trait::async_trait]
impl Provider for MockResident {
    type Config = bool;
    type Instance = u32;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("mock-resident")
    }

    async fn create(&self, _config: &bool, _ctx: &ResourceContext) -> Result<u32, Error> {
        let count = self.create_count.fetch_add(1, Ordering::Relaxed);
        tokio::task::yield_now().await;
        Ok(count + 100)
    }

    async fn destroy(&self, _runtime: u32, _cx: crate::TeardownCx) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("mock-resident"), "")
    }
}

crate::no_credential_slots!(MockResident);

impl ResidentProvider for MockResident {
    fn is_alive_sync(&self, _runtime: &u32) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
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

#[tokio::test]
async fn create_entry_creates_on_first_call() {
    let resource = MockResident::new();
    let rt = Resident::<MockResident>::new(Config::default());
    let ctx = test_ctx();
    let retained = Arc::new(crate::RetainedStore::for_test());

    let inst = rt
        .clone_or_create(&resource, &true, &ctx, &retained)
        .await
        .expect("first create");
    assert_eq!(**inst.entry(), 100);
    assert_eq!(resource.create_count.load(Ordering::Relaxed), 1);
    assert!(rt.is_initialized().await);
}

#[tokio::test]
async fn create_entry_reuses_existing_instance() {
    let resource = MockResident::new();
    let rt = Resident::<MockResident>::new(Config::default());
    let ctx = test_ctx();
    let retained = Arc::new(crate::RetainedStore::for_test());

    let a = rt
        .clone_or_create(&resource, &true, &ctx, &retained)
        .await
        .unwrap();
    let b = rt
        .clone_or_create(&resource, &true, &ctx, &retained)
        .await
        .unwrap();
    assert!(
        Arc::ptr_eq(a.entry(), b.entry()),
        "both entries own the same retained master"
    );
    assert_eq!(
        resource.create_count.load(Ordering::Relaxed),
        1,
        "the second clone reuses the master — only one create"
    );
}

#[tokio::test]
async fn concurrent_create_entry_creates_only_once() {
    let resource = MockResident::new();
    let rt = Arc::new(Resident::<MockResident>::new(Config::default()));
    let ctx = Arc::new(test_ctx());
    let retained = Arc::new(crate::RetainedStore::for_test());

    let mut handles = Vec::new();
    for _ in 0..10 {
        let r = resource.clone();
        let runtime = Arc::clone(&rt);
        let c = Arc::clone(&ctx);
        let retained = Arc::clone(&retained);
        handles.push(tokio::spawn(async move {
            runtime
                .clone_or_create(&r, &true, c.as_ref(), &retained)
                .await
                .unwrap()
        }));
    }
    for h in handles {
        let _ = h.await.unwrap();
    }
    assert_eq!(
        resource.create_count.load(Ordering::Relaxed),
        1,
        "concurrent clone-or-create on an empty cell creates exactly once"
    );
}

#[tokio::test]
async fn recreates_when_not_alive_and_configured() {
    let resource = MockResident::new();
    let config = Config {
        recreate_on_failure: true,
        ..Default::default()
    };
    let rt = Resident::<MockResident>::new(config);
    let ctx = test_ctx();
    let retained = Arc::new(crate::RetainedStore::for_test());

    let a = rt
        .clone_or_create(&resource, &true, &ctx, &retained)
        .await
        .unwrap();
    assert_eq!(**a.entry(), 100);
    resource.alive.store(false, Ordering::Relaxed);
    let b = rt
        .clone_or_create(&resource, &true, &ctx, &retained)
        .await
        .unwrap();
    assert_eq!(
        **b.entry(),
        101,
        "a fresh master was built after liveness failed"
    );
    assert_eq!(resource.create_count.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn fails_when_not_alive_and_no_recreate() {
    let resource = MockResident::new();
    let config = Config {
        recreate_on_failure: false,
        ..Default::default()
    };
    let rt = Resident::<MockResident>::new(config);
    let ctx = test_ctx();
    let retained = Arc::new(crate::RetainedStore::for_test());

    let _a = rt
        .clone_or_create(&resource, &true, &ctx, &retained)
        .await
        .unwrap();
    resource.alive.store(false, Ordering::Relaxed);
    assert!(
        rt.clone_or_create(&resource, &true, &ctx, &retained)
            .await
            .is_err(),
        "a dead master with recreate disabled must fail"
    );
}

#[tokio::test]
async fn topology_does_not_pool() {
    let rt = Resident::<MockResident>::new(Config::default());
    assert!(
        !Topology::<MockResident>::pools(&rt),
        "resident does not pool — released clones are dropped"
    );
    assert_eq!(Topology::<MockResident>::tag(&rt), TopologyTag::Resident);
}

// A resource whose `create()` never returns — for timeout tests.
#[derive(Clone)]
struct HangingResident;

#[async_trait::async_trait]
impl Provider for HangingResident {
    type Config = bool;
    type Instance = u32;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("hanging-resident")
    }

    async fn create(&self, _config: &bool, _ctx: &ResourceContext) -> Result<u32, Error> {
        tokio::time::sleep(Duration::from_hours(1)).await;
        Ok(0)
    }

    async fn destroy(&self, _runtime: u32, _cx: crate::TeardownCx) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("hanging-resident"), "")
    }
}

crate::no_credential_slots!(HangingResident);

impl ResidentProvider for HangingResident {}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_timeout_does_not_deadlock() {
    let resource = HangingResident;
    let config = Config {
        create_timeout: Duration::from_millis(50),
        ..Default::default()
    };
    let rt = Arc::new(Resident::<HangingResident>::new(config));
    let ctx = Arc::new(test_ctx());
    let retained = Arc::new(crate::RetainedStore::for_test());

    assert!(
        rt.clone_or_create(&resource, &true, ctx.as_ref(), &retained)
            .await
            .is_err(),
        "first create should time out"
    );
    assert!(
        rt.clone_or_create(&resource, &true, ctx.as_ref(), &retained)
            .await
            .is_err(),
        "second create should time out (lock released)"
    );
}

// ---------------------------------------------------------------------
// `built_epoch` sampled AFTER `create()` reads the slot (not before).
// ---------------------------------------------------------------------

use std::sync::Arc as StdArc;

use tokio::sync::Notify;

use crate::slot::SlotCell;

#[derive(Default)]
struct FakeCred(u32);

impl zeroize::Zeroize for FakeCred {
    fn zeroize(&mut self) {
        self.0 = 0;
    }
}

#[derive(Clone)]
struct SlotReadResident {
    slot: StdArc<SlotCell<FakeCred>>,
    entered_before_read: StdArc<Notify>,
    release_read: StdArc<Notify>,
    park_before_read: StdArc<AtomicBool>,
}

#[async_trait::async_trait]
impl Provider for SlotReadResident {
    type Config = bool;
    type Instance = u32;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("slot-read-resident")
    }

    async fn create(&self, _config: &bool, _ctx: &ResourceContext) -> Result<u32, Error> {
        self.entered_before_read.notify_one();
        if self.park_before_read.load(Ordering::SeqCst) {
            self.release_read.notified().await;
        }
        let cred = self
            .slot
            .load()
            .map(|g| g.0)
            .ok_or_else(|| Error::permanent("slot unbound at create"))?;
        Ok(cred)
    }

    async fn destroy(&self, _runtime: u32, _cx: crate::TeardownCx) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("slot-read-resident"), "")
    }
}

impl crate::resource::HasCredentialSlots for SlotReadResident {
    fn credential_slot_epoch(&self) -> u64 {
        self.slot.generation()
    }

    // `slot` is a real `#[credential]`-shaped field — declaring it
    // (final-review item 4) is the honest signal `#[derive(Resource)]`
    // would emit for a struct with a `#[credential]` field, even though
    // this fixture drives it directly via `clone_or_create` rather than
    // through `Manager::refresh_slot`.
    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &["slot"]
    }
}

impl ResidentProvider for SlotReadResident {
    fn is_alive_sync(&self, _runtime: &u32) -> bool {
        true
    }
}

const CRED_OLD: u32 = 7;
const CRED_NEW: u32 = 99;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn built_epoch_records_post_create_slot_epoch_not_presample() {
    let slot: SlotCell<FakeCred> = SlotCell::empty();
    slot.store(StdArc::new(FakeCred(CRED_OLD)));
    let slot = StdArc::new(slot);
    let gen_old = slot.generation();

    let resource = SlotReadResident {
        slot: StdArc::clone(&slot),
        entered_before_read: StdArc::new(Notify::new()),
        release_read: StdArc::new(Notify::new()),
        park_before_read: StdArc::new(AtomicBool::new(true)),
    };
    let rt = Arc::new(Resident::<SlotReadResident>::new(Config::default()));
    let ctx = Arc::new(test_ctx());
    let retained = Arc::new(crate::RetainedStore::for_test());

    let acquire_task = {
        let rt = Arc::clone(&rt);
        let resource = resource.clone();
        let ctx = Arc::clone(&ctx);
        let retained = Arc::clone(&retained);
        tokio::spawn(async move {
            rt.clone_or_create(&resource, &true, ctx.as_ref(), &retained)
                .await
        })
    };

    resource.entered_before_read.notified().await;
    slot.store(StdArc::new(FakeCred(CRED_NEW)));
    let gen_new = slot.generation();
    assert!(
        gen_new > gen_old,
        "store must strictly advance the generation"
    );
    resource.release_read.notify_one();

    let inst = acquire_task
        .await
        .expect("task must not panic")
        .expect("first create must succeed");
    assert_eq!(
        **inst.entry(),
        CRED_NEW,
        "create read the slot after the store"
    );
    assert_eq!(
        rt.built_epoch_for_test().await,
        gen_new,
        "built_epoch must be the epoch the instance actually bound \
         (post-create slot read), not the pre-create sample"
    );
    assert!(
        rt.built_epoch_for_test().await >= slot.generation(),
        "an instance built reading the current slot must not be older \
         than the live slot epoch (no spurious stale reconcile)"
    );
}

#[tokio::test]
async fn built_epoch_matches_slot_epoch_with_no_race() {
    let slot: SlotCell<FakeCred> = SlotCell::empty();
    slot.store(StdArc::new(FakeCred(CRED_OLD)));
    let slot = StdArc::new(slot);

    let resource = SlotReadResident {
        slot: StdArc::clone(&slot),
        entered_before_read: StdArc::new(Notify::new()),
        release_read: StdArc::new(Notify::new()),
        park_before_read: StdArc::new(AtomicBool::new(false)),
    };
    let rt = Resident::<SlotReadResident>::new(Config::default());
    let ctx = test_ctx();
    let retained = Arc::new(crate::RetainedStore::for_test());

    let inst = rt
        .clone_or_create(&resource, &true, &ctx, &retained)
        .await
        .expect("create must succeed");
    assert_eq!(**inst.entry(), CRED_OLD);
    assert!(rt.is_initialized().await);
    assert_eq!(
        rt.built_epoch_for_test().await,
        slot.generation(),
        "with no racing store, built_epoch is exactly the live slot epoch"
    );
}
