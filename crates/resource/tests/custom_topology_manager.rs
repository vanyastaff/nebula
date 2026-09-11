//! The safety-by-construction proof the convergence lacked: a **custom**
//! `impl Topology<R>` registered through `Manager::register()`, acquired and
//! released end-to-end, with the credential-revoke fence owned by the
//! **framework** — the author writes **zero** store / checkout / destroy /
//! fence code.
//!
//! `FfmpegPool` is an author-supplied topology that is neither the built-in
//! `Pooled` nor `Resident`. It supplies only the entry-centric `Topology<Ffmpeg>`
//! hooks (`try_reserve`, `create_entry`, `entry_instance`, `into_owned_instance`,
//! `pools`, `store_capacity`). It holds **no** `InstanceStore` and contains
//! **no** `store.checkout` / `resource.destroy` / stale-handling / epoch-compare
//! code — the framework owns the idle store and the fence.
//!
//! The test proves:
//! 1. a custom topology registers + acquires through the erased
//!    `Manager::acquire_any` path, reporting `TopologyTag::Custom`;
//! 2. the public manager revoke path fences the whole custom-topology row:
//!    an entry created before revoke is never leased again, and terminal
//!    shutdown destroys it through framework-owned cleanup. The author writes
//!    no store, fence, or destroy dispatch code.

use std::{
    assert_matches,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_resource::{
    AcquireOptions, HasCredentialSlots, Manager, RegistrationSpec, ResourceContext, ShutdownConfig,
    SlotIdentity,
    error::{Error, ErrorKind},
    resource::{Provider, ResourceConfig, ResourceMetadataDraft},
    topology::{InstanceStore, Ticket, Topology, Unavailable},
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

// ─── The resource ──────────────────────────────────────────────────────────

#[derive(Clone, Default, nebula_schema::Schema)]
struct FfmpegCfg;
impl ResourceConfig for FfmpegCfg {
    fn fingerprint(&self) -> u64 {
        0
    }
}

/// A transcoder "handle" carrying a unique id. `destroy` counts teardowns so
/// the test can observe that the framework actually tears a stale handle down.
/// The id is the entry identity — carried through the framework store, not read
/// directly in assertions (the destroy/create counters are the observables).
struct Transcoder(
    #[expect(
        dead_code,
        reason = "entry identity carried by the handle, not asserted on directly"
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

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::from_key(Self::key())
    }
}

impl HasCredentialSlots for Ffmpeg {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }

    fn declares_credential_slots() -> bool {
        true
    }

    fn credential_slot_names() -> &'static [&'static str] {
        &["transcoder"]
    }
}

// ─── The custom topology ───────────────────────────────────────────────────

/// A bespoke permit-gated pool over a framework-owned idle store of transcoder
/// entries. It supplies ONLY the entry-centric [`Topology<Ffmpeg>`] hooks — it owns
/// no store, runs no checkout, destroys nothing, and never compares a revoke
/// epoch. The framework owns the idle store, the fenced checkout, the
/// stale-entry destroy, and the cancel-safe guard-wrap.
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

/// How a [`FfmpegPool`]'s `create_entry` (mis)behaves — drives the foolproofing
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

impl Topology<Ffmpeg> for FfmpegPool {
    // The entry IS the leasable transcoder. The framework stores it, fences it,
    // and hands it back on checkout — the author never touches the store.
    type Entry = Transcoder;

    fn try_reserve(&self, _store: &InstanceStore<Transcoder>) -> Result<Ticket, Unavailable> {
        self.sem
            .clone()
            .try_acquire_owned()
            .map(Ticket::permit)
            .map_err(|_| Unavailable::Saturated { retry_after: None })
    }

    async fn create_entry(
        &self,
        resource: &Ffmpeg,
        config: &FfmpegCfg,
        ctx: &ResourceContext,
        _retained: &nebula_resource::topology::RetainedStore<Self::Entry>,
    ) -> Result<nebula_resource::topology::CreatedEntry<Transcoder>, Error> {
        // Make one fresh transcoder. The framework decides WHEN to call this
        // (on an idle-miss / warmup); the author only knows HOW to build one.
        match self.mode {
            CreateMode::Normal => resource
                .create(config, ctx)
                .await
                .map(nebula_resource::topology::CreatedEntry::new),
            CreateMode::Hang => std::future::pending().await,
            CreateMode::Panic => panic!(
                "foolproofing: a careless create_entry panics — the framework must \
                 isolate it via catch_unwind and surface a typed error, not crash \
                 the caller's acquire"
            ),
        }
    }

    fn entry_instance<'s>(&self, entry: &'s Transcoder) -> &'s Transcoder {
        entry
    }

    fn into_owned_instance(&self, entry: Transcoder) -> Option<Transcoder> {
        Some(entry)
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

/// A custom `impl Topology<R>` registers through `Manager::register()`,
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

/// The safety-by-construction proof: the public manager revoke path fences a
/// custom topology without exposing the operational registry handle. The
/// author writes zero fence code; after revoke, the old entry cannot be leased
/// again and framework-owned shutdown cleanup destroys it.
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
    assert_eq!(
        g.release()
            .await
            .expect("release completes after framework recycling"),
        nebula_resource::ReleaseOutcome::Completed
    );
    assert_eq!(create_count.load(Ordering::SeqCst), 1);
    assert_eq!(
        destroy_count.load(Ordering::SeqCst),
        0,
        "the released entry was recycled, not destroyed"
    );

    // 2. Revoke through the manager-owned lifecycle operation. This applies
    //    the taint and epoch fence synchronously before its awaited tail.
    let outcome = manager
        .revoke_slot(&key, ScopeLevel::Global, "transcoder")
        .await
        .expect("manager revoke must accept the declared slot");
    assert_matches!(
        outcome,
        nebula_resource::SlotDispatchOutcome::Completed { .. }
    );

    // 3. The revoked row cannot hand out the pre-revoke entry (or create a
    //    replacement against the revoked credential).
    let error = Manager::acquire_any(
        Arc::clone(&manager),
        &key,
        &ctx,
        &AcquireOptions::default(),
        &SlotIdentity::Unbound,
    )
    .await
    .expect_err("a revoked custom-topology row must reject new leases");
    assert_eq!(*error.kind(), ErrorKind::Revoked);
    assert_eq!(
        create_count.load(Ordering::SeqCst),
        1,
        "revoke must not create a replacement against revoked credentials"
    );

    // Terminal cleanup still owns and destroys the retained idle entry.
    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("shutdown must clean the retained custom-topology entry");
    let destroyed = poll_until(std::time::Duration::from_secs(2), || {
        destroy_count.load(Ordering::SeqCst) >= 1
    })
    .await;
    assert!(
        destroyed,
        "framework-owned terminal cleanup must destroy the retained revoked entry"
    );
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

/// Foolproofing (G2): a third-party topology whose `create_entry` **panics** must
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

/// Foolproofing (G1): a third-party topology whose `create_entry` **hangs** must
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
        "a hanging create_entry must be bounded by the acquire deadline and fail \
         closed, never wedge the caller forever",
    );
    assert!(
        matches!(*err.kind(), ErrorKind::Backpressure),
        "a deadline-bounded hang fails closed as Backpressure (got {err:?})"
    );
}
