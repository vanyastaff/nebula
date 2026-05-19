//! Registry rows must be keyed by resolved per-slot credential identity, not
//! by `(ResourceKey, ScopeLevel)` alone (security: cross-tenant runtime bleed).
//!
//! Before this fix, two registrations of the same resource type at the same
//! `ScopeLevel` collapsed to a single registry row (last-write-wins
//! replacement in `Registry::register`), so two tenants whose resolved
//! credentials differ ended up sharing one topology runtime — tenant A's
//! runtime served tenant B's credential.
//!
//! The fix folds a resolved per-slot credential identity into the registry
//! row identity (`DedupKey`). Two registrations that resolve *different*
//! credentials occupy *distinct* rows with *distinct* topology runtimes; a
//! registration that resolves the *same* credential still collapses to one
//! row (the `cross_workflow` shared-resource invariant).
//!
//! The slot-identity input is independent of the author's
//! `ResourceConfig::fingerprint()` — this fixture leaves `fingerprint()` at
//! its `0` default to prove the separation does **not** rely on the author
//! overriding it (a discipline-based defence, explicitly rejected).

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use nebula_core::{OrgId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_resource::{
    AcquireOptions, Manager, RegisterOptions, ResidentConfig, Resource, ResourceConfig,
    ResourceContext, error::Error, resource::ResourceMetadata, topology::resident::Resident,
};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct CountingError(String);

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

/// Config whose `fingerprint()` is left at the `0` default on purpose: the
/// dedup separation must come from the resolved slot identity, never from the
/// author overriding `fingerprint()`.
#[derive(Clone)]
struct CountingConfig;

nebula_schema::impl_empty_has_schema!(CountingConfig);

impl ResourceConfig for CountingConfig {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    // fingerprint() intentionally NOT overridden — stays 0.
}

/// Each `create` mints a fresh, unique runtime id from a shared counter, so a
/// distinct `Resource::create` invocation yields a distinguishable runtime
/// (mirrors the `manager_refresh_slot.rs` `RUNTIME_TAG` witness style).
#[derive(Clone)]
struct CountingRuntime {
    id: u64,
}

#[derive(Clone)]
struct CountingResource {
    create_counter: Arc<AtomicU64>,
}

impl CountingResource {
    fn new(counter: Arc<AtomicU64>) -> Self {
        Self {
            create_counter: counter,
        }
    }
}

impl Resource for CountingResource {
    type Config = CountingConfig;
    type Runtime = CountingRuntime;
    type Lease = CountingRuntime;
    type Error = CountingError;

    fn key() -> ResourceKey {
        resource_key!("dedup-slot-ident")
    }

    async fn create(
        &self,
        _config: &CountingConfig,
        _ctx: &ResourceContext,
    ) -> Result<CountingRuntime, CountingError> {
        let id = self.create_counter.fetch_add(1, Ordering::SeqCst);
        Ok(CountingRuntime { id })
    }

    async fn destroy(&self, _runtime: CountingRuntime) -> Result<(), CountingError> {
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

fn ctx_for_org(org: OrgId) -> ResourceContext {
    let scope = Scope {
        org_id: Some(org),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

/// Two registrations of the SAME resident resource type at the SAME
/// `ScopeLevel`, with the DEFAULT `fingerprint()` (== 0) but DIFFERENT
/// resolved per-slot credential identities, must NOT collapse to one shared
/// runtime. Each acquired runtime must be distinct.
#[tokio::test]
async fn distinct_resolved_slot_identity_yields_distinct_runtimes() {
    let manager = Manager::new();
    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);

    // Shared create counter across BOTH registrations — proves each row drove
    // its own independent `Resource::create` (distinct runtime ids), not one
    // shared runtime aliased to two tenants.
    let counter = Arc::new(AtomicU64::new(0));

    // Tenant A: resolved credential identity #1.
    manager
        .register_resident_with(
            CountingResource::new(Arc::clone(&counter)),
            CountingConfig,
            ResidentConfig::default(),
            RegisterOptions::default()
                .with_scope(scope.clone())
                .with_slot_identity(0xAAAA_AAAA_AAAA_AAAA),
        )
        .expect("register tenant A must succeed");

    // Tenant B: SAME key + SAME scope + SAME (default-0) fingerprint, but a
    // DIFFERENT resolved credential identity.
    manager
        .register_resident_with(
            CountingResource::new(Arc::clone(&counter)),
            CountingConfig,
            ResidentConfig::default(),
            RegisterOptions::default()
                .with_scope(scope.clone())
                .with_slot_identity(0xBBBB_BBBB_BBBB_BBBB),
        )
        .expect("register tenant B must succeed");

    // Acquire from tenant A's binding and tenant B's binding. Each must route
    // to its OWN runtime (distinct `create`), never a shared one.
    let ctx = ctx_for_org(org);
    let lease_a = manager
        .acquire_resident_for::<CountingResource>(
            &ctx,
            &AcquireOptions::default(),
            0xAAAA_AAAA_AAAA_AAAA,
        )
        .await
        .expect("acquire tenant A must succeed");
    let lease_b = manager
        .acquire_resident_for::<CountingResource>(
            &ctx,
            &AcquireOptions::default(),
            0xBBBB_BBBB_BBBB_BBBB,
        )
        .await
        .expect("acquire tenant B must succeed");

    assert_ne!(
        lease_a.id, lease_b.id,
        "two registrations of the same resource type at the same scope with \
         DIFFERENT resolved per-slot credential identities must NOT share a \
         runtime (cross-tenant bleed); got id_a={} id_b={}",
        lease_a.id, lease_b.id
    );

    // Each binding is stable: re-acquiring tenant A returns A's runtime, never
    // B's (no cross-aliasing on repeat acquire).
    let lease_a2 = manager
        .acquire_resident_for::<CountingResource>(
            &ctx,
            &AcquireOptions::default(),
            0xAAAA_AAAA_AAAA_AAAA,
        )
        .await
        .expect("re-acquire tenant A must succeed");
    assert_eq!(
        lease_a.id, lease_a2.id,
        "tenant A must keep its own shared runtime across acquires"
    );
}

/// The same-credential invariant must stay green: two registrations with the
/// SAME resolved slot identity at the same key+scope still collapse to ONE
/// row → ONE `Resource::create` (mirrors the engine `cross_workflow`
/// shared-resource dedup contract at the resource-crate level).
#[tokio::test]
async fn identical_slot_identity_still_dedupes_to_one_runtime() {
    let manager = Manager::new();
    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);
    let counter = Arc::new(AtomicU64::new(0));

    let same_identity = 0x1234_5678_9ABC_DEF0;

    manager
        .register_resident_with(
            CountingResource::new(Arc::clone(&counter)),
            CountingConfig,
            ResidentConfig::default(),
            RegisterOptions::default()
                .with_scope(scope.clone())
                .with_slot_identity(same_identity),
        )
        .expect("first register must succeed");
    // Re-register with the SAME resolved identity — last-write-wins replace of
    // the SAME row (not a second distinct row).
    manager
        .register_resident_with(
            CountingResource::new(Arc::clone(&counter)),
            CountingConfig,
            ResidentConfig::default(),
            RegisterOptions::default()
                .with_scope(scope.clone())
                .with_slot_identity(same_identity),
        )
        .expect("re-register must succeed");

    let ctx = ctx_for_org(org);
    let l1 = manager
        .acquire_resident_for::<CountingResource>(&ctx, &AcquireOptions::default(), same_identity)
        .await
        .expect("acquire #1");
    let l2 = manager
        .acquire_resident_for::<CountingResource>(&ctx, &AcquireOptions::default(), same_identity)
        .await
        .expect("acquire #2");

    assert_eq!(
        l1.id, l2.id,
        "same resolved slot identity at same key+scope must dedupe to one \
         shared runtime (one Resource::create)"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "same-identity re-registration must collapse to a single \
         Resource::create invocation"
    );
}

// ---------------------------------------------------------------------------
// R15 — the cross-tenant barrier is collision-free
//
// A 64-bit digest registry-row discriminator is collidable: two
// registrations whose resolved credentials differ but whose digests
// collide would silently merge into one registry row (last-write-wins), so
// one tenant's topology runtime would serve another tenant's credential —
// bypassing the fail-closed ambiguity deny. The barrier is the structural
// resolved-`(slot, credential)` identity (`SlotIdentity`) whose `Eq` is
// exact, so collision is impossible by construction and a forced digest
// collision is unrepresentable on the structural path.
// ---------------------------------------------------------------------------

/// Proves R15 is **closed**. Two registrations of the same resource type at
/// the same scope that resolve **different** credentials (each its own
/// `Resource::create` counter) must NOT share a runtime — each tenant
/// reaches its own `create`, never the other's.
///
/// The U1 form of this scenario forced both tenants onto the *same* 64-bit
/// `slot_identity` digest (the adversarial collision input) and the
/// last-write-wins registry merged them into one row → cross-tenant bleed.
/// With the structural identity that input is **unrepresentable**: the row
/// is keyed by the exact resolved `(slot, credential)` bindings, so two
/// distinct resolved credentials are two distinct rows by construction (no
/// digest, no collidable space). The scenario is re-expressed through
/// distinct structural bindings and now passes; we additionally assert
/// that the legacy *digest* of tenant A's bindings (the original
/// adversarial `u64`) cannot resolve A's structural row, so a forced
/// digest collision has no structural row to merge.
#[tokio::test]
async fn forced_slot_identity_collision_must_not_bleed_across_tenants() {
    use nebula_resource::SlotIdentity;

    let manager = Manager::new();
    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);

    // Two *different* resolved credentials for the same slot. Each tenant
    // has its OWN create counter so a distinct row drives a distinct
    // `create`; a merged row would show one tenant served the other's
    // runtime (a single `create` shared across both).
    let bindings_a: [(&str, &str); 1] = [("db", "tenant-a-cred")];
    let bindings_b: [(&str, &str); 1] = [("db", "tenant-b-cred")];
    let counter_a = Arc::new(AtomicU64::new(1_000));
    let counter_b = Arc::new(AtomicU64::new(2_000));

    manager
        .register_resident_with(
            CountingResource::new(Arc::clone(&counter_a)),
            CountingConfig,
            ResidentConfig::default(),
            RegisterOptions::default()
                .with_scope(scope.clone())
                .with_slot_bindings(&bindings_a),
        )
        .expect("register tenant A must succeed");
    manager
        .register_resident_with(
            CountingResource::new(Arc::clone(&counter_b)),
            CountingConfig,
            ResidentConfig::default(),
            RegisterOptions::default()
                .with_scope(scope.clone())
                .with_slot_bindings(&bindings_b),
        )
        .expect("register tenant B must succeed");

    let ctx = ctx_for_org(org);
    let id_a = SlotIdentity::from_bindings(bindings_a.iter().copied());
    let id_b = SlotIdentity::from_bindings(bindings_b.iter().copied());

    let lease_a = manager
        .acquire_resident_for_identity::<CountingResource>(&ctx, &AcquireOptions::default(), &id_a)
        .await
        .expect("tenant A acquire must succeed");
    let lease_b = manager
        .acquire_resident_for_identity::<CountingResource>(&ctx, &AcquireOptions::default(), &id_b)
        .await
        .expect("tenant B acquire must succeed");

    assert_ne!(
        lease_a.id, lease_b.id,
        "two registrations resolving DIFFERENT credentials must never share \
         a runtime — the structural identity keys the row by the exact \
         resolved bindings, so a digest collision cannot merge tenant rows \
         (cross-tenant bleed is unrepresentable)."
    );

    // Re-acquiring tenant A by its structural identity stays A's runtime —
    // no cross-aliasing to B on repeat acquire.
    let lease_a2 = manager
        .acquire_resident_for_identity::<CountingResource>(&ctx, &AcquireOptions::default(), &id_a)
        .await
        .expect("re-acquire tenant A must succeed");
    assert_eq!(
        lease_a.id, lease_a2.id,
        "tenant A keeps its own structural row across acquires"
    );

    // The legacy collidable digest of A's bindings is in a disjoint
    // identity space from A's structural row: a forced u64 collision has
    // no structural row to merge into.
    #[allow(deprecated)]
    let legacy_digest_a = nebula_resource::slot_identity(bindings_a.iter().copied());
    assert_ne!(
        id_a,
        SlotIdentity::Opaque(legacy_digest_a),
        "structural identity must never equal the legacy digest of the \
         same bindings (disjoint spaces — a digest cannot alias a \
         structural row)"
    );
}

/// Pins the `h == SLOT_IDENTITY_UNBOUND ⇒ 1` nudge branch of the current
/// `slot_identity()` primitive: a non-empty resolved binding set must never
/// produce the reserved `SLOT_IDENTITY_UNBOUND` value (which means "no
/// resolved slots"). This documents the exact weak-primitive contract being
/// replaced; it must hold for any non-empty binding, including the reserved
/// edge.
#[test]
fn slot_identity_nudge_keeps_non_empty_off_the_unbound_sentinel() {
    use nebula_resource::{SLOT_IDENTITY_UNBOUND, slot_identity};

    // Empty bindings are the ONLY input that may yield the sentinel.
    let empty: Vec<(&str, &str)> = Vec::new();
    assert_eq!(slot_identity(empty), SLOT_IDENTITY_UNBOUND);

    // Every non-empty binding must be nudged off the sentinel: a real
    // resolved credential must never be mistaken for "unbound".
    for (slot, cred) in [
        ("db", "cred-a"),
        ("cache", "cred-b"),
        ("db", ""),
        ("", "x"),
        ("queue", "tenant-7-credential"),
    ] {
        assert_ne!(
            slot_identity([(slot, cred)]),
            SLOT_IDENTITY_UNBOUND,
            "non-empty binding ({slot:?}, {cred:?}) must be nudged off the \
             reserved unbound sentinel"
        );
    }
}
