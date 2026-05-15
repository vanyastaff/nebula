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
    atomic::{AtomicUsize, Ordering},
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key};
use nebula_credential::CredentialGuard;
use nebula_resource::{
    Manager, ResidentConfig, Resource, ResourceConfig, ResourceContext, SlotCell, error::Error,
    resource::ResourceMetadata, topology::resident::Resident,
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
}

use counting::registered;

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
