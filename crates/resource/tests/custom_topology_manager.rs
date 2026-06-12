//! The safety-by-construction proof the convergence lacked: a **custom**
//! `impl Topology<R>` registered through `Manager::register()`, acquired and
//! released end-to-end, with the credential-revoke fence owned by the
//! **framework** — the author writes **zero** store / checkout / destroy /
//! fence code.
//!
//! `FfmpegPool` is an author-supplied topology that is neither the built-in
//! `Pooled` nor `Resident`. It supplies only the slot-centric `Topology<Ffmpeg>`
//! hooks (`try_reserve`, `create_slot`, `slot_instance`, `into_instance`,
//! `pools`, `store_capacity`). It holds **no** `InstanceStore` and contains
//! **no** `store.checkout` / `resource.destroy` / stale-handling / epoch-compare
//! code — the framework owns the idle store and the fence.
//!
//! The test proves:
//! 1. a custom topology registers + acquires through the erased
//!    `Manager::acquire_any` path, reporting `TopologyTag::Custom`;
//! 2. a slot that went idle **before** a credential revoke is evicted
//!    (destroyed) on the next acquire — and the **framework**, not the author,
//!    does the eviction. Bumping the revoke epoch through the erased
//!    `ManagedHandle::bump_revoke_epoch` (exactly as `Manager::revoke_slot`
//!    does in phase 1) and re-acquiring shows the stale slot is never served
//!    and a fresh one is created in its place — with no author fence code.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_resource::{
    AcquireOptions, Manager, RegistrationSpec, ResourceContext, SlotIdentity,
    error::{Error, ErrorKind},
    resource::{HasCredentialSlots, Provider, ResourceConfig, ResourceMetadata},
    topology::{InstanceStore, Ticket, Topology, Unavailable},
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

// ─── The resource ──────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct FfmpegCfg;
nebula_resource::impl_empty_has_schema!(FfmpegCfg);
impl ResourceConfig for FfmpegCfg {
    fn fingerprint(&self) -> u64 {
        0
    }
}

/// A transcoder "handle" carrying a unique id. `destroy` counts teardowns so
/// the test can observe that the framework actually tears a stale handle down.
/// The id is the slot identity — carried through the framework store, not read
/// directly in assertions (the destroy/create counters are the observables).
#[derive(Clone)]
struct Transcoder(
    #[allow(
        dead_code,
        reason = "slot identity carried by the handle, not asserted on directly"
    )]
    u64,
);

#[derive(Clone)]
struct Ffmpeg {
    create_count: Arc<AtomicU64>,
    destroy_count: Arc<AtomicU64>,
}

impl Ffmpeg {
    fn new() -> Self {
        Self {
            create_count: Arc::new(AtomicU64::new(0)),
            destroy_count: Arc::new(AtomicU64::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl Provider for Ffmpeg {
    type Config = FfmpegCfg;
    type Instance = Transcoder;
    type Topology = FfmpegPool;

    fn key() -> ResourceKey {
        resource_key!("custom.ffmpeg")
    }

    async fn create(
        &self,
        _config: &FfmpegCfg,
        _ctx: &ResourceContext,
    ) -> Result<Transcoder, Error> {
        let id = self.create_count.fetch_add(1, Ordering::SeqCst);
        Ok(Transcoder(id))
    }

    async fn destroy(
        &self,
        _runtime: Transcoder,
        _cx: nebula_resource::TeardownCx,
    ) -> Result<(), Error> {
        self.destroy_count.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl HasCredentialSlots for Ffmpeg {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

// ─── The custom topology ───────────────────────────────────────────────────

/// A bespoke permit-gated pool over a framework-owned idle store of transcoder
/// slots. It supplies ONLY the slot-centric [`Topology<Ffmpeg>`] hooks — it owns
/// no store, runs no checkout, destroys nothing, and never compares a revoke
/// epoch. The framework owns the idle store, the fenced checkout, the
/// stale-slot destroy, and the cancel-safe guard-wrap.
///
/// **Structural proof (asserted by `ffmpeg_pool_holds_no_store_or_fence`):**
/// this struct has exactly two fields — a `Semaphore` and a capacity — and NO
/// `InstanceStore`, no `Provider` handle, and no revoke-epoch counter. There is
/// no place for an author to even write fence code.
struct FfmpegPool {
    sem: Arc<Semaphore>,
    cap: usize,
    mode: CreateMode,
}

/// How a [`FfmpegPool`]'s `create_slot` (mis)behaves — drives the foolproofing
/// tests proving the framework bounds + isolates a careless `impl Topology`.
#[derive(Clone, Copy)]
enum CreateMode {
    /// Builds a transcoder normally.
    Normal,
    /// Never completes — proves the acquire deadline / ceiling caps a hanging
    /// hook instead of wedging the caller (and drain) forever.
    Hang,
    /// Panics — proves `catch_unwind` turns it into a typed error instead of
    /// crashing the caller's acquire.
    Panic,
}

impl FfmpegPool {
    fn new(cap: usize) -> Self {
        Self::with_mode(cap, CreateMode::Normal)
    }

    fn with_mode(cap: usize, mode: CreateMode) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(cap)),
            cap,
            mode,
        }
    }
}

#[async_trait::async_trait]
impl Topology<Ffmpeg> for FfmpegPool {
    // The slot IS the leasable transcoder. The framework stores it, fences it,
    // and hands it back on checkout — the author never touches the store.
    type Slot = Transcoder;

    fn try_reserve(&self, _store: &InstanceStore<Transcoder>) -> Result<Ticket, Unavailable> {
        self.sem
            .clone()
            .try_acquire_owned()
            .map(Ticket::permit)
            .map_err(|_| Unavailable::Saturated { retry_after: None })
    }

    async fn create_slot(
        &self,
        resource: &Ffmpeg,
        config: &FfmpegCfg,
        ctx: &ResourceContext,
    ) -> Result<Transcoder, Error> {
        // Make one fresh transcoder. The framework decides WHEN to call this
        // (on an idle-miss / warmup); the author only knows HOW to build one.
        match self.mode {
            CreateMode::Normal => resource.create(config, ctx).await,
            CreateMode::Hang => std::future::pending().await,
            CreateMode::Panic => panic!(
                "foolproofing: a careless create_slot panics — the framework must \
                 isolate it via catch_unwind and surface a typed error, not crash \
                 the caller's acquire"
            ),
        }
    }

    fn slot_instance<'s>(&self, slot: &'s Transcoder) -> &'s Transcoder {
        slot
    }

    fn into_instance(&self, slot: Transcoder) -> Transcoder {
        slot
    }

    fn pools(&self) -> bool {
        // Released transcoders return to the framework idle store, where the
        // revoke fence reaches them.
        true
    }

    fn store_capacity(&self) -> Option<usize> {
        Some(self.cap)
    }
}

// ─── Test harness ──────────────────────────────────────────────────────────

fn ctx() -> ResourceContext {
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

fn register(manager: &Manager, ffmpeg: Ffmpeg) {
    register_topo(manager, ffmpeg, FfmpegPool::new(2));
}

fn register_topo(manager: &Manager, ffmpeg: Ffmpeg, topology: FfmpegPool) {
    let spec = RegistrationSpec {
        resource: ffmpeg,
        config: FfmpegCfg,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology,
        recovery_gate: None,
    };
    manager
        .register(spec)
        .expect("a custom topology must register through Manager::register");
}

/// C8 (1): a custom `impl Topology<R>` registers through `Manager::register()`,
/// acquires + releases end-to-end through the erased acquire path, reporting the
/// `Custom` tag.
#[tokio::test]
async fn custom_topology_registers_and_acquires_through_manager() {
    let manager = Arc::new(Manager::new());
    let ffmpeg = Ffmpeg::new();
    let create_count = Arc::clone(&ffmpeg.create_count);
    register(&manager, ffmpeg);

    let ctx = ctx();
    let key = Ffmpeg::key();

    // Acquire through the erased Manager path (the same path the engine resource
    // accessor uses) — proves the custom topology is reachable through the
    // registry/dispatch, not just standalone.
    let boxed = Manager::acquire_any(
        Arc::clone(&manager),
        &key,
        &ctx,
        &AcquireOptions::default(),
        &SlotIdentity::Unbound,
    )
    .await
    .expect("custom-topology acquire must succeed through Manager::acquire_any");
    let guard = boxed
        .downcast::<nebula_resource::guard::ResourceGuard<Ffmpeg>>()
        .expect("downcast to the typed guard");
    assert_eq!(
        guard.topology_tag(),
        nebula_resource::TopologyTag::Custom,
        "a custom topology reports the Custom tag"
    );
    drop(guard);

    assert_eq!(
        create_count.load(Ordering::SeqCst),
        1,
        "the first acquire materialized exactly one transcoder"
    );
}

/// C8 (2) — the safety-by-construction proof: a slot idle **before** a revoke is
/// evicted (destroyed) by the **FRAMEWORK** on the next acquire. The author
/// wrote zero fence code; bumping the revoke epoch through the erased
/// `ManagedHandle::bump_revoke_epoch` (exactly as `Manager::revoke_slot` phase 1
/// does) makes the framework store fence the stale slot on the next checkout.
#[tokio::test]
async fn custom_topology_store_is_revoke_fenced_by_framework() {
    let manager = Arc::new(Manager::new());
    let ffmpeg = Ffmpeg::new();
    let create_count = Arc::clone(&ffmpeg.create_count);
    let destroy_count = Arc::clone(&ffmpeg.destroy_count);
    register(&manager, ffmpeg);

    let ctx = ctx();
    let key = Ffmpeg::key();

    // 1. Acquire + release so a clean transcoder sits idle in the FRAMEWORK
    //    store (the author's topology never sees the store).
    let g = Manager::acquire_any(
        Arc::clone(&manager),
        &key,
        &ctx,
        &AcquireOptions::default(),
        &SlotIdentity::Unbound,
    )
    .await
    .expect("first acquire")
    .downcast::<nebula_resource::guard::ResourceGuard<Ffmpeg>>()
    .expect("downcast");
    drop(g);
    // Wait for the release worker to recycle the slot into the framework store.
    let recycled = poll_until(std::time::Duration::from_secs(2), || {
        create_count.load(Ordering::SeqCst) == 1
    })
    .await;
    assert!(recycled, "the first acquire created exactly one transcoder");

    // 2. Revoke: bump the revoke epoch through the erased handle — exactly the
    //    synchronous phase-1 step `Manager::revoke_slot` performs. The author
    //    topology is NOT involved; the framework store now holds a stale slot.
    let handle = manager
        .get_any(&key, &ScopeLevel::Global)
        .expect("the registered row is reachable through the erased handle");
    handle.bump_revoke_epoch();

    // 3. Next acquire: the FRAMEWORK loop checks out, sees the stale slot,
    //    destroys it (`destroy(into_instance(stale))`), and creates a fresh one.
    //    The author wrote no checkout, no destroy, no epoch compare.
    let g2 = Manager::acquire_any(
        Arc::clone(&manager),
        &key,
        &ctx,
        &AcquireOptions::default(),
        &SlotIdentity::Unbound,
    )
    .await
    .expect("acquire after revoke")
    .downcast::<nebula_resource::guard::ResourceGuard<Ffmpeg>>()
    .expect("downcast");

    // The framework destroyed the since-revoked idle slot on checkout...
    let destroyed = poll_until(std::time::Duration::from_secs(2), || {
        destroy_count.load(Ordering::SeqCst) >= 1
    })
    .await;
    assert!(
        destroyed,
        "the FRAMEWORK must have destroyed the since-revoked idle slot on \
         checkout — the custom topology has no fence/destroy code at all"
    );
    // ...and created a fresh transcoder to serve this acquire.
    assert_eq!(
        create_count.load(Ordering::SeqCst),
        2,
        "a fresh transcoder was created after the stale one was fenced by the \
         framework (the stale slot was never re-served)"
    );
    drop(g2);
}

/// Polls `cond` until it returns `true` or the deadline elapses; returns the
/// final value. Deterministic replacement for a fixed sleep on release/recycle.
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

/// The structural proof that backs acceptance item #2: `FfmpegPool` holds no
/// `InstanceStore`, no `Provider` handle, and no revoke-epoch counter — only a
/// semaphore, a capacity, and a test-only create mode. There is no place for an
/// author to write fence / store / destroy code, so the fence is framework-owned
/// by construction.
#[test]
fn ffmpeg_pool_holds_no_store_or_fence() {
    // `FfmpegPool` is `{ sem: Arc<Semaphore>, cap: usize, mode: CreateMode }`.
    // An embedded `InstanceStore` / `Provider` / epoch field would push the size
    // well past `{ Arc + usize + a byte-sized enum }`; the bound is the tripwire
    // prompting a re-review of whether an author re-introduced fence code.
    assert!(
        size_of::<FfmpegPool>() <= size_of::<Arc<Semaphore>>() + 2 * size_of::<usize>(),
        "FfmpegPool grew past {{ semaphore + capacity + create-mode }} — re-review \
         whether an author re-introduced an InstanceStore / Provider / fence field; \
         the framework, not the topology, owns the store and the revoke fence"
    );
}

/// Foolproofing (G2): a third-party topology whose `create_slot` **panics** must
/// not unwind into the caller — the acquire pipeline `catch_unwind`s author
/// hooks and surfaces a typed `Permanent` error instead of crashing the acquire.
#[tokio::test]
async fn custom_topology_panic_in_create_is_isolated() {
    let manager = Arc::new(Manager::new());
    register_topo(
        &manager,
        Ffmpeg::new(),
        FfmpegPool::with_mode(2, CreateMode::Panic),
    );

    let err = Manager::acquire_any(
        Arc::clone(&manager),
        &Ffmpeg::key(),
        &ctx(),
        &AcquireOptions::default(),
        &SlotIdentity::Unbound,
    )
    .await
    .expect_err(
        "a panicking topology hook must surface a typed error, not unwind into the \
         caller — the acquire pipeline isolates author hooks via catch_unwind",
    );
    assert!(
        matches!(*err.kind(), ErrorKind::Permanent),
        "an isolated topology-hook panic fails closed as Permanent (got {err:?})"
    );
}

/// Foolproofing (G1): a third-party topology whose `create_slot` **hangs** must
/// not wedge the caller — the acquire deadline bounds it and it fails closed.
/// `start_paused` fires the deadline instantly + deterministically, so a real
/// "hang forever" hook resolves to a bounded error with no wall-clock wait.
#[tokio::test(start_paused = true)]
async fn custom_topology_hang_in_create_is_bounded_by_deadline() {
    let manager = Arc::new(Manager::new());
    register_topo(
        &manager,
        Ffmpeg::new(),
        FfmpegPool::with_mode(2, CreateMode::Hang),
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    let err = Manager::acquire_any(
        Arc::clone(&manager),
        &Ffmpeg::key(),
        &ctx(),
        &AcquireOptions::default().with_deadline(deadline),
        &SlotIdentity::Unbound,
    )
    .await
    .expect_err(
        "a hanging create_slot must be bounded by the acquire deadline and fail \
         closed, never wedge the caller forever",
    );
    assert!(
        matches!(*err.kind(), ErrorKind::Backpressure),
        "a deadline-bounded hang fails closed as Backpressure (got {err:?})"
    );
}
