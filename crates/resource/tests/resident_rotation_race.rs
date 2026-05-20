//! #680 — resident create-vs-rotate lost-update.
//!
//! per-resource revoke deferral. The Resident arm of `dispatch_slot_hook` used to map
//! `current() == None` to `Ok(())` (a silent no-op recorded as a rotation
//! *success*). When a refresh raced the **first** acquire of a Resident
//! resource, the runtime could be built from the pre-rotation credential
//! while the fan-out reported success and the `on_credential_refresh` hook
//! was never delivered — the runtime then served the stale credential
//! indefinitely.
//!
//! These tests drive that exact interleaving deterministically (a gate
//! inside `create()` parks the first build *after* it has read the slot;
//! the credential is then rotated and `refresh_slot` dispatched while the
//! build is parked) and assert the runtime ends up bound to the **new**
//! credential with the hook delivered exactly once — no sleeps, no
//! yield-budget guessing; the ordering is enforced by a barrier and the
//! resident `create_lock`.

use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicUsize, Ordering},
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::CredentialGuard;
use nebula_resource::{
    AcquireOptions, Manager, RegistrationSpec, ResidentConfig, Resource, ResourceConfig,
    ResourceContext, SlotCell, SlotIdentity,
    error::Error,
    resource::ResourceMetadata,
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident::Resident,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

/// A fake credential secret (`Zeroize` so it can sit inside a
/// `CredentialGuard`). The `u32` is the "which credential" tag the test
/// asserts the runtime is bound to.
#[derive(Default)]
struct FakeCred(u32);

impl Zeroize for FakeCred {
    fn zeroize(&mut self) {
        self.0 = 0;
    }
}

#[derive(Debug)]
struct RaceError(String);

impl std::fmt::Display for RaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RaceError {}

impl From<RaceError> for Error {
    fn from(e: RaceError) -> Self {
        Error::transient(e.0)
    }
}

#[derive(Clone)]
struct RaceConfig;

nebula_schema::impl_empty_has_schema!(RaceConfig);

impl ResourceConfig for RaceConfig {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
}

/// Coordinates the deterministic interleaving. Shared between the resource
/// descriptor and the test body.
#[derive(Clone, Default)]
struct RaceGate {
    /// Fired by `create()` the instant it has read the slot — the test
    /// waits on this before rotating, so the rotation strictly follows the
    /// pre-rotation slot read.
    slot_read: Arc<Notify>,
    /// `create()` parks on this *after* the slot read until the test
    /// releases it (the test rotates + dispatches refresh while parked).
    release_create: Arc<Notify>,
    /// When `true`, `create()` performs the park; when `false` it returns
    /// immediately (used for the warm / never-activated fixtures).
    park_in_create: Arc<std::sync::atomic::AtomicBool>,
}

/// The live runtime. `bound_cred` is the credential tag the runtime is
/// currently bound to; it is an `Arc<AtomicU32>` so the lease handed to the
/// caller and the cell-stored runtime share it — a reconcile that mutates
/// the stored runtime's binding is observable through the caller's guard.
#[derive(Clone)]
struct RaceRuntime {
    bound_cred: Arc<AtomicU32>,
    refresh_calls: Arc<AtomicUsize>,
    revoke_calls: Arc<AtomicUsize>,
}

/// Resident resource whose `create()` *reads its credential slot* (the
/// realistic shape — a connection bound to the resolved credential) and
/// whose `on_credential_refresh` re-reads the slot and rebinds the live
/// runtime via interior mutability (the blue-green `&self` reaction).
#[derive(Clone)]
struct RaceResource {
    db: Arc<SlotCell<CredentialGuard<FakeCred>>>,
    gate: RaceGate,
    refresh_calls: Arc<AtomicUsize>,
    revoke_calls: Arc<AtomicUsize>,
}

impl Resource for RaceResource {
    type Config = RaceConfig;
    type Runtime = RaceRuntime;
    type Lease = RaceRuntime;
    type Error = RaceError;

    fn key() -> ResourceKey {
        resource_key!("race-resident")
    }

    async fn create(
        &self,
        _config: &RaceConfig,
        _ctx: &ResourceContext,
    ) -> Result<RaceRuntime, RaceError> {
        // Read the resolved credential exactly as a real resource would
        // (bind the runtime to whatever the slot holds *now*).
        let cred = self
            .db
            .load()
            .map(|g| g.0)
            .ok_or_else(|| RaceError("slot unbound at create".to_owned()))?;
        let runtime = RaceRuntime {
            bound_cred: Arc::new(AtomicU32::new(cred)),
            refresh_calls: self.refresh_calls.clone(),
            revoke_calls: self.revoke_calls.clone(),
        };

        // Signal "I have read the slot", then park (if armed) so the test
        // can rotate + dispatch refresh while this build is in flight and
        // still holding the resident `create_lock`.
        self.gate.slot_read.notify_one();
        if self.gate.park_in_create.load(Ordering::SeqCst) {
            self.gate.release_create.notified().await;
        }
        Ok(runtime)
    }

    async fn on_credential_refresh(
        &self,
        _slot_name: &str,
        runtime: &RaceRuntime,
    ) -> Result<(), RaceError> {
        // The blue-green `&self` reaction: re-read the (now rotated) slot
        // and rebind the live runtime to it via interior mutability.
        if let Some(g) = self.db.load() {
            runtime.bound_cred.store(g.0, Ordering::SeqCst);
        }
        runtime.refresh_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn on_credential_revoke(
        &self,
        _slot_name: &str,
        runtime: &RaceRuntime,
    ) -> Result<(), RaceError> {
        // Model "stop serving the revoked credential": clear the binding.
        runtime.bound_cred.store(0, Ordering::SeqCst);
        runtime.revoke_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    // Activate the create-vs-rotate epoch reconcile: a hand-written impl
    // would otherwise inherit the `0` default and never detect staleness.
    // The derive emits exactly this (the `max` slot generation); we mirror
    // it by hand because this fixture is not derived. NOT author discipline
    // for production resources — `#[derive(Resource)]` generates it.
    fn credential_slot_epoch(&self) -> u64 {
        self.db.generation()
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for RaceResource {
    fn is_alive_sync(&self, _runtime: &RaceRuntime) -> bool {
        true
    }
}

/// Pre-rotation / post-rotation credential tags.
const CRED_OLD: u32 = 7;
const CRED_NEW: u32 = 99;

fn ctx() -> ResourceContext {
    ResourceContext::minimal(Scope::default(), CancellationToken::new())
}

fn build(park: bool) -> (Arc<Manager>, ResourceKey, RaceResource) {
    let slot: SlotCell<CredentialGuard<FakeCred>> = SlotCell::empty();
    slot.store(Arc::new(CredentialGuard::new(FakeCred(CRED_OLD))));
    let resource = RaceResource {
        db: Arc::new(slot),
        gate: RaceGate {
            park_in_create: Arc::new(std::sync::atomic::AtomicBool::new(park)),
            ..RaceGate::default()
        },
        refresh_calls: Arc::new(AtomicUsize::new(0)),
        revoke_calls: Arc::new(AtomicUsize::new(0)),
    };
    let mgr = Manager::new();
    mgr.register(RegistrationSpec {
        resource: resource.clone(),
        config: RaceConfig,
        scope: ScopeLevel::Global,
        slot_identity: SlotIdentity::Unbound,
        topology: TopologyRuntime::Resident(ResidentRuntime::<RaceResource>::new(
            ResidentConfig::default(),
        )),
        acquire: Manager::erased_acquire_resident_for::<RaceResource>(),
        recovery_gate: None,
    })
    .expect("resident registration must succeed");
    (Arc::new(mgr), RaceResource::key(), resource)
}

/// THE #680 regression. The first acquire's `create()` reads the OLD
/// credential and parks (still holding `create_lock`). While parked, the
/// credential is rotated (`slot.store(NEW)`) and `refresh_slot` is
/// dispatched — the dispatch must block on `create_lock`. When the build is
/// released, the dispatch observes the freshly-stored runtime as *stale*
/// (built epoch < slot epoch) and delivers `on_credential_refresh`, so the
/// runtime the caller holds ends up bound to the NEW credential and the
/// hook fired exactly once. Pre-fix the dispatch saw `current() == None`,
/// returned `Ok(())` (false success), and the runtime served `CRED_OLD`
/// forever.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resident_create_during_rotation_delivers_hook_not_false_success() {
    let (mgr, key, resource) = build(true);

    // Task A: first acquire. Its `create()` will read the slot then park.
    let acquire_task = {
        let mgr = Arc::clone(&mgr);
        tokio::spawn(async move {
            mgr.acquire_resident::<RaceResource>(&ctx(), &AcquireOptions::default())
                .await
        })
    };

    // Wait until `create()` has read the OLD credential (deterministic —
    // the rotation strictly follows the pre-rotation slot read).
    resource.gate.slot_read.notified().await;

    // Rotate the credential underneath the in-flight build (the engine
    // fan-out contract: store the new guard into the slot cell, *then*
    // dispatch the refresh). `store` bumps the slot generation.
    resource
        .db
        .store(Arc::new(CredentialGuard::new(FakeCred(CRED_NEW))));

    // Dispatch the refresh concurrently. It must park on `create_lock`
    // (task A holds it for the whole parked build), so give it a real
    // chance to reach that park point, then release the build.
    let refresh_task = {
        let mgr = Arc::clone(&mgr);
        let key = key.clone();
        tokio::spawn(async move { mgr.refresh_slot(&key, ScopeLevel::Global, "db").await })
    };

    // The refresh future must still be pending while the build is parked
    // (it is blocked on `create_lock`). A short bounded timeout that
    // *expires* is the proof the dispatch is genuinely serialised behind
    // the in-flight create rather than racing it.
    let mut refresh_task = refresh_task;
    let still_pending =
        tokio::time::timeout(std::time::Duration::from_millis(150), &mut refresh_task).await;
    assert!(
        still_pending.is_err(),
        "refresh_slot must be parked on create_lock while the first build \
         is in flight (serialised create-vs-rotate)"
    );

    // Release the parked build → it stores the (OLD-bound) runtime and its
    // build epoch, releases `create_lock`; the refresh dispatch then runs
    // and must reconcile.
    resource.gate.release_create.notify_one();

    let guard = acquire_task
        .await
        .expect("acquire task must not panic")
        .expect("first acquire must succeed");
    refresh_task
        .await
        .expect("refresh task must not panic")
        .expect("refresh_slot must succeed (the hook ran — a real success)");

    // The decisive assertions: the runtime the caller holds was reconciled
    // to the NEW credential and the hook fired exactly once. Pre-fix this
    // would be `CRED_OLD` with `refresh_calls == 0`.
    assert_eq!(
        guard.bound_cred.load(Ordering::SeqCst),
        CRED_NEW,
        "the resident runtime must be reconciled to the rotated credential \
         (create-vs-rotate lost-update closed)"
    );
    assert_eq!(
        resource.refresh_calls.load(Ordering::SeqCst),
        1,
        "on_credential_refresh must be delivered exactly once (not skipped \
         with a false success, not double-delivered)"
    );

    // And a subsequent rotation still reconciles (epoch keeps advancing).
    resource
        .db
        .store(Arc::new(CredentialGuard::new(FakeCred(123))));
    mgr.refresh_slot(&key, ScopeLevel::Global, "db")
        .await
        .expect("second refresh must succeed");
    assert_eq!(guard.bound_cred.load(Ordering::SeqCst), 123);
    assert_eq!(resource.refresh_calls.load(Ordering::SeqCst), 2);
}

/// Revoke inverse: a revoke racing the first resident acquire must not
/// leave the runtime serving the revoked credential. Same interleaving as
/// the refresh test but `revoke_slot` (taint + drain + revoke hook) — the
/// runtime built mid-revoke must still receive `on_credential_revoke`
/// (here: binding cleared to `0`) rather than continuing to serve
/// `CRED_OLD`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resident_revoke_during_first_acquire_does_not_serve_revoked_credential() {
    let (mgr, key, resource) = build(true);

    let acquire_task = {
        let mgr = Arc::clone(&mgr);
        tokio::spawn(async move {
            mgr.acquire_resident::<RaceResource>(&ctx(), &AcquireOptions::default())
                .await
        })
    };

    resource.gate.slot_read.notified().await;

    // Engine revoke contract: the fan-out runs the *synchronous* taint
    // first (outside any timeout), then the awaited drain+hook tail. Drive
    // the same two phases. `taint_slot` is synchronous and must not block
    // on the in-flight create.
    let tainted = mgr
        .taint_slot(&key, ScopeLevel::Global, "db")
        .expect("taint_slot (phase 1) must resolve the row synchronously");

    // Clear the slot (a revoke is a credential-state transition — bumps the
    // generation just like `store`).
    let _ = resource.db.take();

    let revoke_task = {
        let mgr = Arc::clone(&mgr);
        tokio::spawn(async move {
            mgr.drain_and_revoke(tainted, std::time::Duration::from_secs(30))
                .await
        })
    };

    // The drain+revoke tail must be parked while the first build is in
    // flight: it is blocked both on the per-resource drain (task A is an
    // in-flight acquire) and then on `create_lock`.
    let mut revoke_task = revoke_task;
    let pending =
        tokio::time::timeout(std::time::Duration::from_millis(150), &mut revoke_task).await;
    assert!(
        pending.is_err(),
        "drain_and_revoke must be parked while the first build is in flight \
         (per-resource drain + create_lock serialisation)"
    );

    resource.gate.release_create.notify_one();

    let guard = acquire_task
        .await
        .expect("acquire task must not panic")
        .expect("first acquire must succeed");
    // `bound_cred` is shared (Arc) between the caller's lease and the
    // cell-stored runtime the revoke hook acts on. Capture it, then drop
    // the guard so the per-resource drain completes promptly (otherwise the
    // held in-flight guard wedges the 30 s drain) — the revoke hook then
    // runs on the still-cell-resident runtime and clears the shared
    // binding.
    let bound_cred = Arc::clone(&guard.bound_cred);
    drop(guard);

    let tail = revoke_task.await.expect("revoke task must not panic");
    assert!(
        matches!(tail, nebula_resource::RevokeTail::Done),
        "drain_and_revoke must complete the revoke hook, got: {tail:?}"
    );

    assert_eq!(
        bound_cred.load(Ordering::SeqCst),
        0,
        "the runtime built mid-revoke must NOT keep serving the revoked \
         credential — the revoke hook must have cleared the binding"
    );
    assert_eq!(
        resource.revoke_calls.load(Ordering::SeqCst),
        1,
        "on_credential_revoke must be delivered exactly once to the \
         mid-revoke-built runtime"
    );
}

/// A genuinely never-activated resident (registered, credential bound, but
/// **no acquire ever** so no runtime was created): a refresh dispatch is a
/// *legitimate* no-op success, NOT a false failure and NOT a stale-skip.
/// This proves the "never created" vs "created against a stale epoch"
/// distinction — the former is the runtime-presence check returning `None`.
#[tokio::test]
async fn never_activated_resident_refresh_is_legitimate_noop() {
    let (mgr, key, resource) = build(false);

    // No acquire has ever run → no runtime exists. Rotate then refresh.
    resource
        .db
        .store(Arc::new(CredentialGuard::new(FakeCred(CRED_NEW))));
    mgr.refresh_slot(&key, ScopeLevel::Global, "db")
        .await
        .expect("refresh on a never-activated resident must be a legitimate Ok no-op");

    // The hook must NOT have fired (there is no live runtime to refresh) —
    // this is the legitimate-no-op case, distinct from a stale-skip.
    assert_eq!(
        resource.refresh_calls.load(Ordering::SeqCst),
        0,
        "no runtime exists — the hook must not fire (legitimate no-op)"
    );

    // And the first acquire *after* the rotation builds against the NEW
    // credential (it reads the current slot), needing no hook delivery.
    let guard = mgr
        .acquire_resident::<RaceResource>(&ctx(), &AcquireOptions::default())
        .await
        .expect("first acquire after rotation must succeed");
    assert_eq!(
        guard.bound_cred.load(Ordering::SeqCst),
        CRED_NEW,
        "a create that runs strictly after the rotation binds the new \
         credential directly (no reconcile needed)"
    );
    assert_eq!(
        resource.refresh_calls.load(Ordering::SeqCst),
        0,
        "still no hook — the build read the fresh credential itself"
    );
}

/// Sanity: with a runtime already warm and *no* intervening rotation, a
/// refresh still delivers the hook exactly once (idempotent per resource runtime status
/// D1) and never spuriously reports the runtime stale. Guards against the
/// epoch compare mis-classifying an up-to-date runtime.
#[tokio::test]
async fn warm_resident_no_rotation_refresh_delivers_once_not_stale() {
    let (mgr, key, resource) = build(false);

    let guard = mgr
        .acquire_resident::<RaceResource>(&ctx(), &AcquireOptions::default())
        .await
        .expect("warm acquire must succeed");
    assert_eq!(guard.bound_cred.load(Ordering::SeqCst), CRED_OLD);

    // No rotation between build and refresh — built epoch == slot epoch.
    mgr.refresh_slot(&key, ScopeLevel::Global, "db")
        .await
        .expect("refresh on a warm, un-rotated resident must succeed");
    assert_eq!(
        resource.refresh_calls.load(Ordering::SeqCst),
        1,
        "the hook is still delivered once (idempotent), not skipped"
    );
    assert_eq!(
        guard.bound_cred.load(Ordering::SeqCst),
        CRED_OLD,
        "no rotation occurred — the binding is unchanged"
    );
}
