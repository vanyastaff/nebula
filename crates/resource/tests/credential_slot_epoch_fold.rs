//! #690 review (CRITICAL, comment 3255607655 / #680) — the
//! `credential_slot_epoch` fold must be **order-sensitive**, not `max`.
//!
//! The load-bearing contract: the epoch changes whenever *any*
//! `#[credential]` slot's generation changes — including a slot whose
//! generation is **not** the largest. A `max` fold violates this: a
//! runtime built at `(slot_a=5, slot_b=10)` then rotated `slot_a→6` still
//! maxes to `10`, so the resident create-vs-rotate reconcile would never
//! see the runtime as stale and would silently report a rotation success
//! while the runtime keeps serving the pre-rotation credential.
//!
//! Two independent proofs:
//!
//! 1. **Derive fold** — a real `#[derive(Resource)]` two-slot struct:
//!    rotating the NON-max slot must change the *derive-emitted*
//!    `credential_slot_epoch()` (this is the exact macro output #680
//!    depends on).
//! 2. **Resident reconcile** — a two-slot resident whose
//!    `credential_slot_epoch()` uses the same positional fold: rotating
//!    the NON-max slot under a warm runtime must make the reconcile
//!    treat the runtime as stale and deliver `on_credential_refresh`
//!    (the end-to-end #680 invariant).

use std::sync::{
    Arc,
    atomic::{AtomicU32, AtomicUsize, Ordering},
};

use nebula_core::{ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_credential::{
    AuthPattern, Credential, CredentialContext, CredentialError, CredentialGuard,
    CredentialMetadata, ResolveResult, SecretString, SecretToken,
};
use nebula_resource::{
    AcquireOptions, Manager, ResidentConfig, Resource, ResourceConfig, ResourceContext, SlotCell,
    error::Error, resource::ResourceMetadata, topology::resident::Resident,
};
use nebula_schema::FieldValues;
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

// ── Shared fake credential ──────────────────────────────────────────

/// Static credential fixture. `Zeroize` is a no-op (unit-ish payload);
/// the `u32` tag is "which credential" the resident test asserts on.
#[derive(Default)]
struct FakeCred(u32);

impl Zeroize for FakeCred {
    fn zeroize(&mut self) {
        self.0 = 0;
    }
}

impl Credential for FakeCred {
    type Properties = ();
    type Scheme = SecretToken;
    type State = SecretToken;

    const KEY: &'static str = "epochfold.fake";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder()
            .key(nebula_core::credential_key!("epochfold.fake"))
            .name("FakeCred")
            .description("slot-epoch fold fixture")
            .schema(nebula_credential::schema_of::<Self::Properties>())
            .pattern(AuthPattern::SecretToken)
            .build()
            .expect("FakeCred metadata is valid")
    }

    fn project(state: &SecretToken) -> SecretToken {
        state.clone()
    }

    async fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<SecretToken, ()>, CredentialError> {
        Ok(ResolveResult::Complete(SecretToken::new(
            SecretString::new("fake-token"),
        )))
    }
}

// ── Part 1: the DERIVE-emitted fold is order-sensitive ──────────────

#[derive(Clone, Default)]
struct TwoSlotCfg;
impl nebula_schema::HasSchema for TwoSlotCfg {
    fn schema() -> nebula_schema::ValidSchema {
        nebula_schema::ValidSchema::empty()
    }
}
impl ResourceConfig for TwoSlotCfg {}

/// A real derived two-slot resource — `credential_slot_epoch()` here is
/// the exact token stream `#[derive(Resource)]` emits.
#[derive(Resource)]
#[resource(key = "epochfold-derived", topology = "resident", config = TwoSlotCfg)]
struct TwoSlotDerived {
    #[credential(key = "slot_a")]
    slot_a: SlotCell<CredentialGuard<FakeCred>>,
    #[credential(key = "slot_b")]
    slot_b: SlotCell<CredentialGuard<FakeCred>>,
}

#[test]
fn derived_epoch_changes_when_non_max_slot_rotates() {
    let r = TwoSlotDerived {
        slot_a: SlotCell::empty(),
        slot_b: SlotCell::empty(),
    };

    // Empty ⇒ 0 ("never bound"), preserved by the fold.
    assert_eq!(
        r.credential_slot_epoch(),
        0,
        "no slot bound yet — epoch must be 0"
    );

    // Bind both. slot_b is `store`d more times so its generation is
    // strictly larger than slot_a's — i.e. slot_a is the NON-max slot.
    r.slot_a.store(Arc::new(CredentialGuard::new(FakeCred(1))));
    r.slot_b.store(Arc::new(CredentialGuard::new(FakeCred(2))));
    r.slot_b.store(Arc::new(CredentialGuard::new(FakeCred(3))));
    r.slot_b.store(Arc::new(CredentialGuard::new(FakeCred(4))));
    assert!(
        r.slot_b.generation() > r.slot_a.generation(),
        "fixture sanity: slot_b must out-generation slot_a so slot_a is the \
         NON-max slot (a `max` fold would ignore a slot_a rotation)"
    );

    let before = r.credential_slot_epoch();

    // Rotate ONLY the non-max slot (slot_a). A `max` fold would NOT
    // change (slot_b still dominates); the order-sensitive fold MUST.
    r.slot_a.store(Arc::new(CredentialGuard::new(FakeCred(9))));
    let after = r.credential_slot_epoch();

    assert_ne!(
        before, after,
        "rotating the NON-max slot must change the derive-emitted epoch \
         (the #680 invariant — `max` would have missed this)"
    );

    // And clearing the non-max slot (a revoke is also a generation
    // transition) changes it again.
    let after2_input = after;
    r.slot_a.take();
    assert_ne!(
        after2_input,
        r.credential_slot_epoch(),
        "clearing the non-max slot (revoke) must also change the epoch"
    );
}

// ── Part 2: the resident reconcile keys off the order-sensitive epoch ─

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
struct RaceCfg;
nebula_schema::impl_empty_has_schema!(RaceCfg);
impl ResourceConfig for RaceCfg {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
}

#[derive(Clone)]
struct TwoSlotRuntime {
    bound_b: Arc<AtomicU32>,
    refresh_calls: Arc<AtomicUsize>,
}

/// Hand-written two-slot resident. `credential_slot_epoch()` mirrors the
/// exact positional fold `#[derive(Resource)]` emits (this fixture is not
/// derived because the derive's `create` body is `todo!()`); the point is
/// that the *reconcile* keys off an order-sensitive epoch, so rotating
/// the non-max slot makes a warm runtime stale.
#[derive(Clone)]
struct TwoSlotResident {
    slot_a: Arc<SlotCell<CredentialGuard<FakeCred>>>,
    slot_b: Arc<SlotCell<CredentialGuard<FakeCred>>>,
    refresh_calls: Arc<AtomicUsize>,
}

impl Resource for TwoSlotResident {
    type Config = RaceCfg;
    type Runtime = TwoSlotRuntime;
    type Lease = TwoSlotRuntime;
    type Error = RaceError;

    fn key() -> ResourceKey {
        resource_key!("epochfold-resident")
    }

    async fn create(
        &self,
        _config: &RaceCfg,
        _ctx: &ResourceContext,
    ) -> Result<TwoSlotRuntime, RaceError> {
        // Bind the runtime to slot_b's current credential tag (the
        // realistic shape — a connection bound to a resolved credential).
        let b = self
            .slot_b
            .load()
            .map(|g| g.0)
            .ok_or_else(|| RaceError("slot_b unbound at create".to_owned()))?;
        Ok(TwoSlotRuntime {
            bound_b: Arc::new(AtomicU32::new(b)),
            refresh_calls: self.refresh_calls.clone(),
        })
    }

    async fn on_credential_refresh(
        &self,
        _slot_name: &str,
        runtime: &TwoSlotRuntime,
    ) -> Result<(), RaceError> {
        if let Some(g) = self.slot_b.load() {
            runtime.bound_b.store(g.0, Ordering::SeqCst);
        }
        runtime.refresh_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    // The exact positional fold the derive emits (FNV-1a 64-bit prime,
    // wrapping). NOT author discipline for production resources —
    // `#[derive(Resource)]` generates this; hand-mirrored only because
    // this fixture cannot be derived (derive `create` is `todo!()`).
    fn credential_slot_epoch(&self) -> u64 {
        const K: u64 = 0x0000_0100_0000_01b3;
        [self.slot_a.generation(), self.slot_b.generation()]
            .into_iter()
            .fold(0u64, |acc, slot_gen| {
                acc.wrapping_mul(K).wrapping_add(slot_gen)
            })
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for TwoSlotResident {
    fn is_alive_sync(&self, _runtime: &TwoSlotRuntime) -> bool {
        true
    }
}

/// Rotating the NON-max slot under a warm resident runtime must make the
/// create-vs-rotate reconcile treat the runtime as **stale** and deliver
/// `on_credential_refresh` — even though the rotated slot is not the
/// largest-generation one. A `max` epoch would leave the runtime serving
/// the pre-rotation credential with a silent false success (#680).
#[tokio::test]
async fn resident_reconcile_fires_when_non_max_slot_rotates() {
    let slot_a: SlotCell<CredentialGuard<FakeCred>> = SlotCell::empty();
    let slot_b: SlotCell<CredentialGuard<FakeCred>> = SlotCell::empty();
    // Make slot_b the MAX-generation slot (stored 3×) and slot_a the
    // non-max slot (stored 1×).
    slot_a.store(Arc::new(CredentialGuard::new(FakeCred(11))));
    slot_b.store(Arc::new(CredentialGuard::new(FakeCred(21))));
    slot_b.store(Arc::new(CredentialGuard::new(FakeCred(22))));
    slot_b.store(Arc::new(CredentialGuard::new(FakeCred(23))));

    let refresh_calls = Arc::new(AtomicUsize::new(0));
    let resource = TwoSlotResident {
        slot_a: Arc::new(slot_a),
        slot_b: Arc::new(slot_b),
        refresh_calls: Arc::clone(&refresh_calls),
    };
    assert!(
        resource.slot_b.generation() > resource.slot_a.generation(),
        "fixture sanity: slot_b must be the max-generation slot"
    );

    let mgr = Manager::new();
    mgr.register_resident(resource.clone(), RaceCfg, ResidentConfig::default())
        .expect("register_resident must succeed");

    // Warm the runtime (records the build epoch against the current
    // slot generations).
    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let guard = mgr
        .acquire_resident::<TwoSlotResident>(&ctx, &AcquireOptions::default())
        .await
        .expect("warm acquire must succeed");
    assert_eq!(guard.bound_b.load(Ordering::SeqCst), 23);

    // Rotate ONLY the NON-max slot (slot_a). With a `max` epoch the
    // built epoch would still equal the live epoch (slot_b dominates) and
    // the reconcile would skip the hook (false success). With the
    // order-sensitive fold the epoch changes, so the reconcile delivers
    // the hook.
    resource
        .slot_a
        .store(Arc::new(CredentialGuard::new(FakeCred(99))));
    mgr.refresh_slot(&TwoSlotResident::key(), ScopeLevel::Global, "slot_a")
        .await
        .expect("refresh_slot must succeed (the hook ran — a real success)");

    assert_eq!(
        refresh_calls.load(Ordering::SeqCst),
        1,
        "rotating the NON-max slot must deliver on_credential_refresh \
         exactly once (order-sensitive epoch → reconcile saw the runtime \
         as stale; a `max` epoch would have skipped it with a false success)"
    );
}
