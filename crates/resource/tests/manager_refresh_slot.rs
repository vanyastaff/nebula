//! `Manager::{refresh_slot, revoke_slot}` — port of the per-slot rotation
//! entry points (ADR-0044).
//!
//! These exercise the real `Manager::register_resident` + `ResourceKey` /
//! `ScopeLevel` API. The test resource carries a real
//! `SlotCell<CredentialGuard<FakeCred>>` field, pre-populated via
//! `SlotCell::store` in the helper (engine-side resolution wiring is out of
//! scope here). The resource's `Runtime` counts hook invocations and records
//! the taint→revoke ordering through shared `Arc` state, so the tests can
//! prove the `&Runtime` reached the `&self` hook and that `revoke_slot`
//! taints before it calls `on_credential_revoke`.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key};
use nebula_credential::CredentialGuard;
use nebula_resource::{
    Manager, ManagerConfig, ResidentConfig, Resource, ResourceConfig, ResourceContext, SlotCell,
    error::Error, resource::ResourceMetadata, topology::resident::Resident,
};
use zeroize::Zeroize;

mod counting {
    use super::*;

    /// Sentinel stamped on the test `Runtime` so the refresh hook can
    /// prove it received *this* live runtime by reference.
    pub const RUNTIME_TAG: usize = 4_242;

    /// A fake credential secret — `Zeroize` so it can sit inside a
    /// `CredentialGuard`.
    #[derive(Default)]
    pub struct FakeCred(pub u32);

    impl Zeroize for FakeCred {
        fn zeroize(&mut self) {
            self.0 = 0;
        }
    }

    /// Shared invocation ledger. Cloned into both the resource descriptor
    /// (which owns the `&self` hooks) and the `Runtime` handle (so a hook
    /// that received a live `&Runtime` can prove it).
    #[derive(Clone, Default)]
    pub struct Ledger {
        /// Bumped by `on_credential_refresh`.
        pub refresh_calls: Arc<AtomicUsize>,
        /// Bumped by `on_credential_revoke`.
        pub revoke_calls: Arc<AtomicUsize>,
        /// Records which `&Runtime` the refresh hook saw (its tag).
        pub refresh_saw_runtime_tag: Arc<AtomicUsize>,
        /// When set, `on_credential_revoke` returns `Err` (drives the
        /// failure-event arm of `revoke_slot`).
        pub revoke_should_fail: Arc<AtomicBool>,
    }

    /// The live runtime handle. Carries the ledger + a tag so the hook can
    /// prove it received *this* runtime by reference.
    #[derive(Clone)]
    pub struct CountingRuntime {
        pub ledger: Ledger,
        pub tag: usize,
    }

    /// The resource descriptor. Owns the `SlotCell` credential field and the
    /// `&self` rotation hooks.
    #[derive(Clone)]
    pub struct CountingResource {
        pub ledger: Ledger,
        /// `#[credential]`-shaped slot field (declared by the author per
        /// the Alternative (a) slot model). Pre-populated in the helper to
        /// model the engine having resolved the credential before
        /// `create`; the rotation hooks act on the runtime, not this cell,
        /// so it is not read directly in these tests.
        #[allow(
            dead_code,
            reason = "models the author-declared SlotCell field; rotation dispatch borrows the runtime, not this cell"
        )]
        pub db: Arc<SlotCell<CredentialGuard<FakeCred>>>,
    }

    #[derive(Debug)]
    pub struct CountingError(pub String);

    impl std::fmt::Display for CountingError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for CountingError {}

    impl From<CountingError> for Error {
        fn from(e: CountingError) -> Self {
            Error::transient(e.0)
        }
    }

    #[derive(Clone)]
    pub struct CountingConfig;

    nebula_schema::impl_empty_has_schema!(CountingConfig);

    impl ResourceConfig for CountingConfig {
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
    }

    impl Resource for CountingResource {
        type Config = CountingConfig;
        type Runtime = CountingRuntime;
        type Lease = CountingRuntime;
        type Error = CountingError;

        fn key() -> ResourceKey {
            resource_key!("counting-resident")
        }

        async fn create(
            &self,
            _config: &CountingConfig,
            _ctx: &ResourceContext,
        ) -> Result<CountingRuntime, CountingError> {
            Ok(CountingRuntime {
                ledger: self.ledger.clone(),
                tag: RUNTIME_TAG,
            })
        }

        async fn on_credential_refresh(
            &self,
            _slot_name: &str,
            runtime: &CountingRuntime,
        ) -> Result<(), CountingError> {
            // Proves the live `&Runtime` reached the `&self` hook: we read
            // the tag off the runtime we were handed.
            self.ledger
                .refresh_saw_runtime_tag
                .store(runtime.tag, Ordering::SeqCst);
            self.ledger.refresh_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn on_credential_revoke(
            &self,
            _slot_name: &str,
            runtime: &CountingRuntime,
        ) -> Result<(), CountingError> {
            runtime.ledger.revoke_calls.fetch_add(1, Ordering::SeqCst);
            if runtime.ledger.revoke_should_fail.load(Ordering::SeqCst) {
                return Err(CountingError("revoke hook boom".to_owned()));
            }
            Ok(())
        }

        fn metadata() -> ResourceMetadata {
            ResourceMetadata::from_key(&Self::key())
        }
    }

    impl Resident for CountingResource {
        fn is_alive_sync(&self, _runtime: &CountingRuntime) -> bool {
            true
        }
    }

    /// Builds a manager with a `CountingResource` registered as Resident,
    /// its `db` slot pre-populated, and the resident runtime warmed so the
    /// dispatch has a live `&Runtime` to borrow.
    pub async fn registered() -> (Manager, ResourceKey, Ledger) {
        let ledger = Ledger::default();
        let slot: SlotCell<CredentialGuard<FakeCred>> = SlotCell::empty();
        slot.store(Arc::new(CredentialGuard::new(FakeCred(7))));
        let resource = CountingResource {
            ledger: ledger.clone(),
            db: Arc::new(slot),
        };

        let mgr = Manager::new();
        mgr.register_resident(resource, CountingConfig, ResidentConfig::default())
            .expect("register_resident must succeed");

        (mgr, CountingResource::key(), ledger)
    }

    /// Same as [`registered`], but the manager is wired with a real
    /// `MetricsRegistry` so `manager.snapshot()` reflects the rotation
    /// outcome counters.
    pub async fn registered_with_metrics() -> (
        Manager,
        ResourceKey,
        Ledger,
        Arc<nebula_metrics::MetricsRegistry>,
    ) {
        let ledger = Ledger::default();
        let slot: SlotCell<CredentialGuard<FakeCred>> = SlotCell::empty();
        slot.store(Arc::new(CredentialGuard::new(FakeCred(7))));
        let resource = CountingResource {
            ledger: ledger.clone(),
            db: Arc::new(slot),
        };

        let registry = Arc::new(nebula_metrics::MetricsRegistry::new());
        let mgr = Manager::with_config(ManagerConfig {
            metrics_registry: Some(Arc::clone(&registry)),
            ..ManagerConfig::default()
        });
        mgr.register_resident(resource, CountingConfig, ResidentConfig::default())
            .expect("register_resident must succeed");

        (mgr, CountingResource::key(), ledger, registry)
    }

    /// Slot identities for the two-tenant isolation fixture. Two distinct
    /// resolved-credential identities at the same `(key, Global)` occupy two
    /// distinct registry rows — each its own `ManagedResource` with its own
    /// per-resource in-flight counter (ADR-0067 §Deferred).
    pub const SLOT_A: u64 = 0xA;
    pub const SLOT_B: u64 = 0xB;

    /// Registers `CountingResource` twice as Resident at `SLOT_A` / `SLOT_B`
    /// so a revoke on one row can be proven isolated from in-flight traffic
    /// to the other. Returns the manager and per-row ledgers.
    pub async fn registered_two_tenant() -> (Arc<Manager>, ResourceKey, Ledger, Ledger) {
        use nebula_resource::RegisterOptions;

        let ledger_a = Ledger::default();
        let ledger_b = Ledger::default();

        let slot_a: SlotCell<CredentialGuard<FakeCred>> = SlotCell::empty();
        slot_a.store(Arc::new(CredentialGuard::new(FakeCred(1))));
        let slot_b: SlotCell<CredentialGuard<FakeCred>> = SlotCell::empty();
        slot_b.store(Arc::new(CredentialGuard::new(FakeCred(2))));

        let mgr = Manager::new();
        mgr.register_resident_with(
            CountingResource {
                ledger: ledger_a.clone(),
                db: Arc::new(slot_a),
            },
            CountingConfig,
            ResidentConfig::default(),
            RegisterOptions::default().with_slot_identity(SLOT_A),
        )
        .expect("register_resident_with (A) must succeed");
        mgr.register_resident_with(
            CountingResource {
                ledger: ledger_b.clone(),
                db: Arc::new(slot_b),
            },
            CountingConfig,
            ResidentConfig::default(),
            RegisterOptions::default().with_slot_identity(SLOT_B),
        )
        .expect("register_resident_with (B) must succeed");

        (Arc::new(mgr), CountingResource::key(), ledger_a, ledger_b)
    }
}

use counting::{registered, registered_two_tenant, registered_with_metrics};

#[tokio::test]
async fn refresh_slot_invokes_hook_with_runtime() {
    use nebula_core::scope::Scope;
    use nebula_resource::AcquireOptions;
    use tokio_util::sync::CancellationToken;

    let (mgr, key, ledger) = registered().await;

    // Resident creates its shared runtime lazily on first acquire — touch
    // it so `refresh_slot` has a live `&Runtime` to hand the hook.
    {
        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let _g = mgr
            .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
            .await
            .expect("acquire must succeed");
    }

    mgr.refresh_slot(&key, ScopeLevel::Global, "db")
        .await
        .expect("refresh_slot must succeed");

    assert_eq!(
        ledger.refresh_calls.load(Ordering::SeqCst),
        1,
        "on_credential_refresh must fire exactly once"
    );
    assert_eq!(
        ledger.refresh_saw_runtime_tag.load(Ordering::SeqCst),
        counting::RUNTIME_TAG,
        "the hook must have received the live &Runtime (its tag)"
    );
}

/// Proves the safety-critical happens-before of `revoke_slot`:
/// `taint` → (`wait_for_drain` blocks on a still-held guard) → (guard
/// dropped) → `on_credential_revoke` fires *last*.
///
/// The earlier version dropped the in-flight guard *before* calling
/// `revoke_slot`, so `wait_for_drain` early-returned (`active == 0`) and
/// the drain never actually waited — ordering was only inferred. Here a
/// real `ResourceGuard` is held for the whole window so the drain is
/// genuinely parked on it, and we observe the revoke future as *pending*
/// while a new acquire is *already rejected* and the hook has *not* run.
#[tokio::test]
async fn revoke_slot_taints_then_drains_then_hooks() {
    use std::sync::Arc;

    use nebula_core::scope::Scope;
    use nebula_error::{Classify, ErrorCategory};
    use nebula_resource::AcquireOptions;
    use tokio_util::sync::CancellationToken;

    let (mgr, key, ledger) = registered().await;
    // `Manager` is shared via `Arc<Manager>`; wrap so the revoke can run
    // on a task while we keep an in-flight guard held on this task.
    let mgr = Arc::new(mgr);

    // 1. Acquire a *real* in-flight guard and KEEP it held. This keeps the
    //    shared drain counter at 1, so `wait_for_drain` inside `revoke_slot`
    //    must actually block (it does not early-return on `active == 0`).
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let in_flight_guard = mgr
        .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("initial acquire must succeed");

    // 2. Spawn `revoke_slot` so it runs *while the guard is still held*.
    //    It taints first, then parks in `wait_for_drain` on our guard.
    let revoke_handle = {
        let mgr = Arc::clone(&mgr);
        let key = key.clone();
        tokio::spawn(async move { mgr.revoke_slot(&key, ScopeLevel::Global, "db").await })
    };

    // 3. While the guard is held and revoke is in-flight, prove the
    //    happens-before precondition:
    //
    //    (a) the taint is ALREADY active — a fresh acquire is rejected
    //        with the exact `Unavailable` category, even though the revoke
    //        future has not resolved; and
    //    (b) the revoke task is still pending (parked in `wait_for_drain`
    //        on our held guard) and the revoke hook has NOT fired.
    //
    // Give the spawned task a few scheduler turns to reach the taint +
    // `wait_for_drain` park point without an arbitrary sleep.
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }

    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let rejected = mgr
        .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
        .await
        .expect_err("acquire while revoke in-flight must be rejected (resource tainted)");
    assert_eq!(
        rejected.category(),
        ErrorCategory::Unavailable,
        "tainted resource must reject new acquires with Unavailable, got: {rejected}"
    );

    // The revoke future must still be pending: it is blocked in
    // `wait_for_drain` because we still hold `in_flight_guard`. A short
    // bounded timeout that *expires* is the proof of "drain is waiting".
    let mut revoke_handle = revoke_handle;
    let still_pending =
        tokio::time::timeout(std::time::Duration::from_millis(150), &mut revoke_handle).await;
    assert!(
        still_pending.is_err(),
        "revoke_slot must still be pending while the in-flight guard is held \
         (blocked in wait_for_drain)"
    );
    assert_eq!(
        ledger.revoke_calls.load(Ordering::SeqCst),
        0,
        "on_credential_revoke must NOT fire while drain is still waiting on the held guard"
    );

    // 4. Drop the in-flight guard → drain counter hits 0 → `wait_for_drain`
    //    wakes → `revoke_slot` proceeds to the hook and returns.
    drop(in_flight_guard);

    revoke_handle
        .await
        .expect("revoke task must not panic")
        .expect("revoke_slot must succeed once the guard is dropped");

    // The hook fired exactly once, *after* the taint and *after* the drain
    // unblocked — a genuine taint → drain-blocks → guard-dropped → hook-last
    // happens-before proof.
    assert_eq!(
        ledger.revoke_calls.load(Ordering::SeqCst),
        1,
        "on_credential_revoke must fire exactly once, as the last step"
    );

    // Resource stays tainted after revoke: a post-revoke acquire is still
    // rejected with the exact `Unavailable` category.
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let post = mgr
        .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
        .await
        .expect_err("acquire after revoke must still be rejected (resource tainted)");
    assert_eq!(
        post.category(),
        ErrorCategory::Unavailable,
        "post-revoke acquire must still be rejected with Unavailable, got: {post}"
    );
}

#[tokio::test]
async fn refresh_slot_unknown_key_is_typed_not_found() {
    use nebula_error::Classify;

    let (mgr, _key, _ledger) = registered().await;
    let unknown = ResourceKey::new("no-such-resource").expect("valid key");

    let err = mgr
        .refresh_slot(&unknown, ScopeLevel::Global, "db")
        .await
        .expect_err("unknown key must error");

    assert_eq!(
        err.category(),
        nebula_error::ErrorCategory::NotFound,
        "unknown key must classify as not_found, got: {err}"
    );
}

/// A drain timeout must be **terminal** for the dispatch's outcome metric:
/// exactly one outcome moves (`timed_out == 1`), and the subsequent
/// successful hook does NOT also record `Success`. The invariant
/// `attempts == success + failed + timed_out` therefore holds with a single
/// recorded outcome per dispatch.
///
/// Time is paused so the 30 s `wait_for_drain` budget elapses in virtual
/// time while a real in-flight guard is still held (drain genuinely times
/// out); the held guard is dropped only after the revoke completes.
#[tokio::test(start_paused = true)]
async fn revoke_drain_timeout_records_exactly_one_outcome() {
    use std::sync::Arc;

    use nebula_core::scope::Scope;
    use nebula_resource::AcquireOptions;
    use tokio_util::sync::CancellationToken;

    let (mgr, key, ledger, _registry) = registered_with_metrics().await;
    let mgr = Arc::new(mgr);

    // Hold a real in-flight guard so `wait_for_drain` cannot early-return
    // (drain counter stays at 1) and the 30 s bounded wait actually expires.
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let in_flight_guard = mgr
        .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("initial acquire must succeed");

    let revoke_handle = {
        let mgr = Arc::clone(&mgr);
        let key = key.clone();
        tokio::spawn(async move { mgr.revoke_slot(&key, ScopeLevel::Global, "db").await })
    };

    // Let the spawned revoke reach the `wait_for_drain` park point, then let
    // paused time auto-advance past the 30 s drain budget. The revoke then
    // proceeds to the (successful) hook while the guard is still held.
    revoke_handle
        .await
        .expect("revoke task must not panic")
        .expect("revoke_slot must still succeed after a drain timeout");

    // Hook ran exactly once and succeeded, but because the drain timed out
    // first, only the terminal `timed_out` outcome was recorded.
    assert_eq!(
        ledger.revoke_calls.load(Ordering::SeqCst),
        1,
        "revoke hook must still run after the drain timeout"
    );

    let snap = mgr
        .metrics()
        .expect("manager wired with a registry must expose metrics")
        .snapshot()
        .slot_revoke_outcomes;
    assert_eq!(
        snap.timed_out, 1,
        "drain timeout must record exactly one timed_out"
    );
    assert_eq!(
        snap.success, 0,
        "timed-out dispatch must NOT also record success"
    );
    assert_eq!(
        snap.failed, 0,
        "timed-out dispatch must NOT also record failed"
    );
    assert_eq!(
        snap.success + snap.failed + snap.timed_out,
        1,
        "exactly one outcome per dispatch — attempts == success + failed + timed_out"
    );

    drop(in_flight_guard);
}

/// A revoke-hook failure must emit `SlotRevokeFailed` — never the refresh
/// event. (Regression: the error arm previously sent
/// `ResourceEvent::SlotRefreshFailed` for a *revoke* failure.)
#[tokio::test]
async fn revoke_failure_emits_slot_revoke_failed_not_refresh() {
    use nebula_core::scope::Scope;
    use nebula_resource::{AcquireOptions, events::ResourceEvent};
    use tokio_util::sync::CancellationToken;

    let (mgr, key, ledger) = registered().await;

    // Warm the resident runtime so the revoke hook has a live `&Runtime`.
    {
        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let _g = mgr
            .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
            .await
            .expect("acquire must succeed");
    }

    ledger.revoke_should_fail.store(true, Ordering::SeqCst);
    let mut events = mgr.subscribe_events();

    let err = mgr
        .revoke_slot(&key, ScopeLevel::Global, "db")
        .await
        .expect_err("revoke must fail when the hook returns Err");
    assert!(
        err.to_string().contains("revoke hook boom"),
        "revoke_slot must surface the hook error, got: {err}"
    );

    // Drain the broadcast channel and assert the failure event is the
    // revoke variant, never the refresh one.
    let mut saw_revoke_failed = false;
    while let Ok(evt) = events.try_recv() {
        match evt {
            ResourceEvent::SlotRevokeFailed {
                key: k,
                slot,
                error,
            } => {
                saw_revoke_failed = true;
                assert_eq!(k.as_str(), key.as_str());
                assert_eq!(slot, "db");
                assert!(
                    error.contains("revoke hook boom"),
                    "event error must carry the (redacted) hook message"
                );
            },
            ResourceEvent::SlotRefreshFailed { .. } => {
                panic!("revoke failure must NOT emit SlotRefreshFailed");
            },
            _ => {},
        }
    }
    assert!(
        saw_revoke_failed,
        "a failed revoke must emit ResourceEvent::SlotRevokeFailed"
    );
}

/// ADR-0044/0036 "no authenticated traffic on a revoked credential
/// post-revoke" — the revoke-vs-acquire TOCTOU close.
///
/// The acquire-side taint *gate* runs before the in-flight counter is
/// incremented, so a concurrent revoke that taints in that window would, in
/// the old code, still let the acquire complete and hand out a live guard on
/// the just-revoked credential. The fix re-checks the taint *after* the
/// per-resource in-flight increment (the same counter `revoke_slot` drains).
///
/// Deterministic proof of the re-check: a real in-flight guard is held so
/// `revoke_slot` taints and then parks in the per-resource drain. While it is
/// parked (taint already set, revoke NOT yet returned) a burst of fresh
/// acquires is fired — every one passes the gate's predecessor work but MUST
/// be rejected by the post-count re-check, never returning a guard.
#[tokio::test]
async fn revoke_vs_acquire_post_taint_recheck_rejects_late_acquire() {
    use std::sync::Arc;

    use nebula_core::scope::Scope;
    use nebula_error::{Classify, ErrorCategory};
    use nebula_resource::AcquireOptions;
    use tokio_util::sync::CancellationToken;

    let (mgr, key, _ledger) = registered().await;
    let mgr = Arc::new(mgr);

    // Hold a real in-flight guard so the per-resource drain inside
    // `revoke_slot` genuinely blocks (counter stays at 1) — this parks the
    // revoke *after* it has tainted but *before* it returns.
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let held = mgr
        .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("initial acquire must succeed");

    let revoke_handle = {
        let mgr = Arc::clone(&mgr);
        let key = key.clone();
        tokio::spawn(async move { mgr.revoke_slot(&key, ScopeLevel::Global, "db").await })
    };

    // Let the revoke task reach the taint + per-resource-drain park point.
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
    let mut revoke_handle = revoke_handle;
    // Sanity: the revoke is genuinely still pending (parked on our guard).
    let pending =
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut revoke_handle).await;
    assert!(
        pending.is_err(),
        "revoke must still be parked in the per-resource drain while the guard is held"
    );

    // Burst of acquires that all start *after* the taint but *while the
    // revoke has not returned*. Every one must be rejected by the post-count
    // re-check — none may return a live guard on the revoked credential.
    for i in 0..64 {
        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let outcome = mgr
            .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
            .await;
        let err = outcome.expect_err(&format!(
            "acquire #{i} during in-flight revoke must be rejected, not return a guard"
        ));
        assert_eq!(
            err.category(),
            ErrorCategory::Unavailable,
            "post-taint re-check must reject with Unavailable, got: {err}"
        );
    }

    // Release the held guard → per-resource drain completes → revoke returns.
    drop(held);
    revoke_handle
        .await
        .expect("revoke task must not panic")
        .expect("revoke_slot must succeed once the held guard drops");

    // And it stays revoked.
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let post = mgr
        .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
        .await
        .expect_err("acquire after revoke must still be rejected");
    assert_eq!(post.category(), ErrorCategory::Unavailable);
}

/// Multi-threaded revoke-vs-acquire stress: many acquire loops race a single
/// `revoke_slot` on a `multi_thread` runtime. The binding invariant
/// (ADR-0044/0036): **no acquire may return a live guard once `revoke_slot`
/// has returned** — a guard handed out on the revoked credential after the
/// revoke completed is exactly the bug. A per-acquire flag, set the instant
/// the revoke future resolves, makes "this guard was issued after revoke
/// completed" observable and is asserted to never happen.
///
/// Robustness: the precondition that acquire works *before* the revoke is
/// proven deterministically by a guard taken on the main task (independent of
/// worker scheduling). The race window is then established by a bounded
/// *barrier* — the revoke is issued only once enough workers have each landed
/// a real successful acquire — rather than a fixed yield budget that starves
/// under heavy parallel test load.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_vs_acquire_multithread_no_guard_after_revoke() {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        time::Duration,
    };

    use nebula_core::scope::Scope;
    use nebula_resource::AcquireOptions;
    use tokio_util::sync::CancellationToken;

    let (mgr, key, _ledger) = registered().await;
    let mgr = Arc::new(mgr);

    // Deterministic precondition: acquire works on the *unrevoked* resource.
    // Proven here on the main task so the test never depends on a spawned
    // worker getting scheduler time before the revoke (the prior flake).
    {
        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let g = mgr
            .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
            .await
            .expect("acquire on the unrevoked resource must succeed (precondition)");
        drop(g);
    }

    let revoke_done = Arc::new(AtomicBool::new(false));
    let guards_after_revoke = Arc::new(AtomicU64::new(0));
    let attempts_after_revoke = Arc::new(AtomicU64::new(0));
    // Per-worker "I have landed ≥1 successful acquire" tally — drives the
    // pre-revoke barrier without a timing guess.
    let ok_before_revoke = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));

    const WORKERS: u64 = 8;
    let mut workers = Vec::new();
    for _ in 0..WORKERS {
        let mgr = Arc::clone(&mgr);
        let revoke_done = Arc::clone(&revoke_done);
        let guards_after_revoke = Arc::clone(&guards_after_revoke);
        let attempts_after_revoke = Arc::clone(&attempts_after_revoke);
        let ok_before_revoke = Arc::clone(&ok_before_revoke);
        let stop = Arc::clone(&stop);
        workers.push(tokio::spawn(async move {
            let mut counted_ok = false;
            while !stop.load(Ordering::Acquire) {
                let after = revoke_done.load(Ordering::Acquire);
                let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
                let outcome = mgr
                    .acquire_resident::<counting::CountingResource>(
                        &ctx,
                        &AcquireOptions::default(),
                    )
                    .await;
                if after {
                    attempts_after_revoke.fetch_add(1, Ordering::AcqRel);
                }
                if let Ok(guard) = outcome {
                    if !counted_ok {
                        counted_ok = true;
                        ok_before_revoke.fetch_add(1, Ordering::AcqRel);
                    }
                    // A live guard observed after the revoke future resolved
                    // is the exact ADR-0044/0036 violation.
                    if revoke_done.load(Ordering::Acquire) {
                        guards_after_revoke.fetch_add(1, Ordering::AcqRel);
                    }
                    drop(guard);
                }
                tokio::task::yield_now().await;
            }
        }));
    }

    // Barrier: issue the revoke only once a majority of workers have each
    // landed a real successful acquire — a genuine, scheduler-independent
    // race window. Bounded so a hang fails loudly instead of looping.
    let barrier = tokio::time::timeout(Duration::from_secs(10), async {
        while ok_before_revoke.load(Ordering::Acquire) < WORKERS / 2 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        barrier.is_ok(),
        "workers failed to establish the pre-revoke acquire window in time"
    );

    mgr.revoke_slot(&key, ScopeLevel::Global, "db")
        .await
        .expect("revoke_slot must succeed");
    // Mark the boundary the instant the revoke future resolved.
    revoke_done.store(true, Ordering::Release);

    // Establish a real *post*-revoke window: wait until workers have made a
    // healthy number of acquire attempts strictly after the boundary (every
    // one of which must be rejected by the fix), bounded so it fails loudly.
    let post = tokio::time::timeout(Duration::from_secs(10), async {
        while attempts_after_revoke.load(Ordering::Acquire) < WORKERS * 4 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(
        post.is_ok(),
        "workers failed to exercise a post-revoke acquire window in time"
    );

    stop.store(true, Ordering::Release);
    for w in workers {
        w.await.expect("worker must not panic");
    }

    assert!(
        ok_before_revoke.load(Ordering::Acquire) >= WORKERS / 2,
        "sanity: the pre-revoke acquire window must have been real"
    );
    assert_eq!(
        guards_after_revoke.load(Ordering::Acquire),
        0,
        "a guard on the revoked credential was handed out AFTER revoke_slot \
         returned — revoke-vs-acquire TOCTOU is not closed"
    );
}

/// ADR-0067 §Deferred: a revoke on one resource must NOT block on in-flight
/// traffic to an *unrelated* resource. Two tenants (`SLOT_A` / `SLOT_B`) of
/// the same resource type occupy two distinct registry rows, each with its
/// own per-resource in-flight counter. With a long-lived lease held on B,
/// `revoke_slot_for(A)` must drain only A's (empty) counter and return
/// promptly — *well* under the manager-wide 30 s budget — never parking on
/// B's held guard. B stays acquirable throughout.
#[tokio::test]
async fn revoke_on_one_resource_does_not_block_on_unrelated_resource() {
    use std::time::{Duration, Instant};

    use nebula_core::scope::Scope;
    use nebula_resource::AcquireOptions;
    use tokio_util::sync::CancellationToken;

    let (mgr, key, _ledger_a, _ledger_b) = registered_two_tenant().await;

    // Hold a long-lived lease on tenant B. If the revoke on A wrongly drained
    // the manager-wide tracker, this would wedge A's revoke for the full 30 s.
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let b_guard = mgr
        .acquire_resident_for::<counting::CountingResource>(
            &ctx,
            &AcquireOptions::default(),
            counting::SLOT_B,
        )
        .await
        .expect("acquire on tenant B must succeed");

    // Revoke tenant A while B's lease is still held. A's per-resource counter
    // is empty, so this must return promptly.
    let started = Instant::now();
    tokio::time::timeout(
        Duration::from_secs(5),
        mgr.revoke_slot_for(&key, ScopeLevel::Global, "db", counting::SLOT_A),
    )
    .await
    .expect("revoke_slot_for(A) must NOT block on tenant B's held lease (would hit 30s)")
    .expect("revoke_slot_for(A) must succeed");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "revoke on A must complete promptly with B's lease held (per-resource \
         drain isolation); took {elapsed:?}"
    );

    // B's lease was never disturbed and B stays acquirable (only A tainted).
    drop(b_guard);
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let _b_again = mgr
        .acquire_resident_for::<counting::CountingResource>(
            &ctx,
            &AcquireOptions::default(),
            counting::SLOT_B,
        )
        .await
        .expect("tenant B must stay acquirable after an unrelated revoke on A");
}

/// #681 — phase 1 (`taint_slot`) is **synchronous**: the taint is fully
/// applied the instant `taint_slot` returns, *before* any `.await`. Proven
/// by acquiring on the row immediately after `taint_slot` returns and before
/// `drain_and_revoke` is ever constructed — it must already be rejected.
/// This is the property a dropped `tokio::time::timeout` future relied on:
/// because the taint is not inside an async body it cannot be skipped by a
/// timeout that fires before the first poll.
#[tokio::test]
async fn taint_slot_applies_taint_synchronously_before_any_await() {
    use nebula_core::scope::Scope;
    use nebula_error::{Classify, ErrorCategory};
    use nebula_resource::AcquireOptions;
    use tokio_util::sync::CancellationToken;

    let (mgr, key, ledger) = registered().await;

    // Warm the resident runtime (acquire+drop) so the phase-2 revoke hook
    // has a live `&Runtime` to borrow — `dispatch_slot_hook` is a no-op
    // `Ok(())` on a Resident whose runtime was never materialized.
    {
        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let g = mgr
            .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
            .await
            .expect("warm the resident runtime");
        drop(g);
    }

    // Phase 1 only — a plain (non-`async`) call. No `.await` has run.
    let tainted = mgr
        .taint_slot(&key, ScopeLevel::Global, "db")
        .expect("taint_slot must resolve the registered row");

    // The row is already tainted: an acquire issued now — before
    // `drain_and_revoke` is even built, let alone awaited — must be rejected.
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let err = mgr
        .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
        .await
        .expect_err("acquire after synchronous taint_slot must be rejected");
    assert_eq!(
        err.category(),
        ErrorCategory::Unavailable,
        "synchronous taint must reject with Revoked/Unavailable, got: {err}"
    );
    assert_eq!(
        ledger.revoke_calls.load(Ordering::SeqCst),
        0,
        "phase 1 must NOT have run the revoke hook (that is phase 2's job)"
    );

    // Completing phase 2 still works and runs the hook exactly once.
    assert!(
        matches!(
            mgr.drain_and_revoke(tainted, std::time::Duration::from_secs(5))
                .await,
            nebula_resource::RevokeTail::Done
        ),
        "drain_and_revoke (phase 2) must complete the revoke hook"
    );
    assert_eq!(
        ledger.revoke_calls.load(Ordering::SeqCst),
        1,
        "phase 2 runs the revoke hook exactly once"
    );
}

/// #681 — cancellation-safety: dropping the `drain_and_revoke` future
/// (phase 2) mid-flight — e.g. a task abort or runtime shutdown
/// cancelling the awaiting task — must leave the row tainted, because the
/// taint already ran in the synchronous phase 1. The credential is never
/// silently un-revoked by a dropped tail. (Post-#690 the engine fan-out
/// no longer wraps this in an outer `tokio::time::timeout`; the
/// drop-safety invariant still holds for any other cancellation and is
/// still load-bearing.)
///
/// The drop is made *genuine*: a real in-flight guard is held so
/// `drain_and_revoke` actually parks in the per-resource drain (it cannot
/// complete on the first poll), and the future is then explicitly dropped
/// while still pending.
#[tokio::test]
async fn dropping_drain_and_revoke_future_keeps_row_tainted() {
    use std::sync::Arc;

    use nebula_core::scope::Scope;
    use nebula_error::{Classify, ErrorCategory};
    use nebula_resource::AcquireOptions;
    use tokio_util::sync::CancellationToken;

    let (mgr, key, ledger) = registered().await;
    let mgr = Arc::new(mgr);

    // Hold a real in-flight guard so phase 2's per-resource drain genuinely
    // blocks (counter stays at 1) — `drain_and_revoke` parks rather than
    // completing on its first poll, making the subsequent drop a true
    // mid-flight cancellation.
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let in_flight = mgr
        .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("initial acquire must succeed");

    // Phase 1: synchronous taint.
    let tainted = mgr
        .taint_slot(&key, ScopeLevel::Global, "db")
        .expect("taint_slot must resolve the registered row");

    // Phase 2 constructed and polled enough to park in the drain, then
    // explicitly DROPPED while still pending — models a task abort /
    // runtime shutdown cancelling the awaiting task mid-tail (a generic
    // cancellation; the engine fan-out no longer wraps this in an outer
    // timeout post-#690, but the drop-safety invariant is unchanged).
    {
        let mut fut = Box::pin(mgr.drain_and_revoke(tainted, std::time::Duration::from_secs(30)));
        // A bounded select that loses to a short timer: the drain future is
        // polled (and parks on the held guard) but never completes, then is
        // dropped at the end of this block.
        let parked = tokio::time::timeout(std::time::Duration::from_millis(150), &mut fut).await;
        assert!(
            parked.is_err(),
            "drain_and_revoke must still be parked in the per-resource drain \
             while the in-flight guard is held"
        );
        drop(fut);
    }
    assert_eq!(
        ledger.revoke_calls.load(Ordering::SeqCst),
        0,
        "the dropped (never-completed) drain future must not have run the \
         revoke hook"
    );

    // The decisive #681 assertion: the taint survived the dropped tail —
    // every subsequent acquire is still rejected. The taint was applied
    // synchronously in phase 1, so dropping the phase-2 future cannot roll
    // it back; the credential is never silently un-revoked.
    for i in 0..16 {
        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let err = mgr
            .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
            .await
            .expect_err(&format!(
                "acquire #{i} after a dropped drain future must still be rejected (#681)"
            ));
        assert_eq!(
            err.category(),
            ErrorCategory::Unavailable,
            "dropped-tail acquire must still hit the Revoked/Unavailable taint, got: {err}"
        );
    }

    drop(in_flight);
}

/// #690 review — single-owner budget: a **timed-out drain still runs the
/// revoke hook**. `drain_and_revoke` is the sole owner of the
/// per-resource budget — it bounds the drain *best-effort* (a drain
/// timeout is non-fatal and proceeds to the hook) and there is no outer
/// `tokio::time::timeout` wrapper that could elapse on the slow drain and
/// drop the whole future before the hook ran. A real in-flight guard is
/// held so the per-resource drain genuinely times out against a short
/// budget; the hook must still fire exactly once and the row stay
/// tainted. Pre-fix the engine fan-out wrapped the whole call in a 30 s
/// timeout that, on a slow drain, dropped the future and silently skipped
/// the hook.
#[tokio::test]
async fn drain_timeout_still_runs_revoke_hook_single_budget_owner() {
    use std::sync::Arc;
    use std::time::Duration;

    use nebula_core::scope::Scope;
    use nebula_resource::AcquireOptions;
    use tokio_util::sync::CancellationToken;

    let (mgr, key, ledger) = registered().await;
    let mgr = Arc::new(mgr);

    // Hold a real in-flight guard so the per-resource drain cannot
    // early-return (counter stays at 1) — with a short budget the drain
    // genuinely times out.
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let in_flight = mgr
        .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("initial acquire must succeed");

    // Phase 1: synchronous taint.
    let tainted = mgr
        .taint_slot(&key, ScopeLevel::Global, "db")
        .expect("taint_slot must resolve the registered row");

    // Phase 2 with a SHORT per-resource budget: the held guard makes the
    // drain exceed it. The hook (`CountingResource::on_credential_revoke`
    // returns immediately) must STILL run — proof the timed-out drain did
    // not drop the tail before the hook.
    let tail = mgr
        .drain_and_revoke(tainted, Duration::from_millis(50))
        .await;
    assert!(
        matches!(tail, nebula_resource::RevokeTail::Done),
        "a timed-out DRAIN must still complete the revoke hook (single \
         budget owner; no outer wrapper drops the post-drain hook), got: {tail:?}"
    );
    assert_eq!(
        ledger.revoke_calls.load(Ordering::SeqCst),
        1,
        "the revoke hook must run exactly once even though the drain timed out"
    );

    drop(in_flight);
}
