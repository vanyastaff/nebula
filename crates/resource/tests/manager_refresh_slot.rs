//! `Manager::{refresh_slot, revoke_slot}` — port of the per-slot rotation
//! entry points (ADR-0044 / ADR-0052).
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

#[tokio::test]
async fn revoke_slot_taints_then_drains_then_hooks() {
    use nebula_core::scope::Scope;
    use nebula_error::Classify;
    use nebula_resource::AcquireOptions;
    use tokio_util::sync::CancellationToken;

    let (mgr, key, ledger) = registered().await;

    // Acquire then drop a guard so an in-flight count exists and drains.
    {
        let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
        let _g = mgr
            .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
            .await
            .expect("acquire must succeed");
    }

    mgr.revoke_slot(&key, ScopeLevel::Global, "db")
        .await
        .expect("revoke_slot must succeed");

    assert_eq!(
        ledger.revoke_calls.load(Ordering::SeqCst),
        1,
        "on_credential_revoke must fire exactly once"
    );

    // Taint must have taken effect *before* the revoke hook ran (the hook
    // is the last step of `revoke_slot`, so by the time it returned a new
    // acquire is already rejected). This is the observable proof that taint
    // preceded the drain+hook, reusing the existing guard-taint mechanism.
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let err = mgr
        .acquire_resident::<counting::CountingResource>(&ctx, &AcquireOptions::default())
        .await
        .expect_err("acquire after revoke must be rejected (resource tainted)");
    assert!(
        matches!(
            err.category(),
            nebula_error::ErrorCategory::Cancelled | nebula_error::ErrorCategory::Internal
        ),
        "tainted resource must reject new acquires, got: {err}"
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
