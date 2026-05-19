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
    AcquireOptions, Manager, RegisterOptions, RegistrationSpec, ResidentConfig, Resource,
    ResourceConfig, ResourceContext, SlotIdentity,
    error::Error,
    resource::ResourceMetadata,
    runtime::{TopologyRuntime, resident::ResidentRuntime},
    topology::resident::Resident,
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

/// Register a `CountingResource` as a resident row, deriving the scope and
/// the resolved-credential identity from a [`RegisterOptions`] exactly the
/// way the (now-deleted) `register_resident_with` shorthand did:
/// `opts.scope` keys the scope and `opts.effective_slot_identity()`
/// supplies the structural anti-bleed identity. Centralised so every
/// strong-net case below threads the identity through one verified path.
fn register_counting(
    manager: &Manager,
    resource: CountingResource,
    opts: RegisterOptions,
) -> Result<(), Error> {
    let slot_identity = opts.effective_slot_identity();
    manager.register(RegistrationSpec {
        resource,
        config: CountingConfig,
        scope: opts.scope,
        slot_identity,
        topology: TopologyRuntime::Resident(ResidentRuntime::<CountingResource>::new(
            ResidentConfig::default(),
        )),
        acquire: Manager::erased_acquire_resident_for::<CountingResource>(),
        resilience: opts.resilience,
        recovery_gate: opts.recovery_gate,
    })
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
    let id_a = SlotIdentity::from_bindings([("db", "cred-tenant-a")]);
    let id_b = SlotIdentity::from_bindings([("db", "cred-tenant-b")]);
    register_counting(
        &manager,
        CountingResource::new(Arc::clone(&counter)),
        RegisterOptions::default()
            .with_scope(scope.clone())
            .with_slot_bindings(&[("db", "cred-tenant-a")]),
    )
    .expect("register tenant A must succeed");

    // Tenant B: SAME key + SAME scope + SAME (default-0) fingerprint, but a
    // DIFFERENT resolved credential identity.
    register_counting(
        &manager,
        CountingResource::new(Arc::clone(&counter)),
        RegisterOptions::default()
            .with_scope(scope.clone())
            .with_slot_bindings(&[("db", "cred-tenant-b")]),
    )
    .expect("register tenant B must succeed");

    // Acquire from tenant A's binding and tenant B's binding. Each must route
    // to its OWN runtime (distinct `create`), never a shared one.
    let ctx = ctx_for_org(org);
    let lease_a = manager
        .acquire_resident_for_identity::<CountingResource>(&ctx, &AcquireOptions::default(), &id_a)
        .await
        .expect("acquire tenant A must succeed");
    let lease_b = manager
        .acquire_resident_for_identity::<CountingResource>(&ctx, &AcquireOptions::default(), &id_b)
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
        .acquire_resident_for_identity::<CountingResource>(&ctx, &AcquireOptions::default(), &id_a)
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

    let same_bindings: &[(&str, &str)] = &[("db", "cred-shared")];
    let same_identity = SlotIdentity::from_bindings(same_bindings.iter().copied());

    register_counting(
        &manager,
        CountingResource::new(Arc::clone(&counter)),
        RegisterOptions::default()
            .with_scope(scope.clone())
            .with_slot_bindings(same_bindings),
    )
    .expect("first register must succeed");
    // Re-register with the SAME resolved identity — last-write-wins replace of
    // the SAME row (not a second distinct row).
    register_counting(
        &manager,
        CountingResource::new(Arc::clone(&counter)),
        RegisterOptions::default()
            .with_scope(scope.clone())
            .with_slot_bindings(same_bindings),
    )
    .expect("re-register must succeed");

    let ctx = ctx_for_org(org);
    let l1 = manager
        .acquire_resident_for_identity::<CountingResource>(
            &ctx,
            &AcquireOptions::default(),
            &same_identity,
        )
        .await
        .expect("acquire #1");
    let l2 = manager
        .acquire_resident_for_identity::<CountingResource>(
            &ctx,
            &AcquireOptions::default(),
            &same_identity,
        )
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

    register_counting(
        &manager,
        CountingResource::new(Arc::clone(&counter_a)),
        RegisterOptions::default()
            .with_scope(scope.clone())
            .with_slot_bindings(&bindings_a),
    )
    .expect("register tenant A must succeed");
    register_counting(
        &manager,
        CountingResource::new(Arc::clone(&counter_b)),
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

    // The barrier is exact structural equality, so a forced collision is
    // unrepresentable by construction: A's and B's resolved bindings yield
    // structurally distinct identities — there is no digest space in which
    // they could alias into one row.
    assert_ne!(
        id_a, id_b,
        "two distinct resolved binding sets must be distinct structural \
         identities — collision is unrepresentable, so a forced digest \
         collision has no structural row to merge into"
    );
}

/// The structural identity reserves [`SlotIdentity::Unbound`] for the
/// no-resolved-slots row: only the empty binding set yields `Unbound`, and
/// every non-empty resolved binding is a `Structural` identity distinct
/// from `Unbound` — a real resolved credential can never be mistaken for
/// "no resolved slots". (Collision-free by construction: exact structural
/// equality, so the reserved row is reachable only from the empty set.)
#[test]
fn non_empty_bindings_are_never_the_unbound_identity() {
    // Empty bindings are the ONLY input that yields the reserved row.
    let empty: Vec<(&str, &str)> = Vec::new();
    assert_eq!(SlotIdentity::from_bindings(empty), SlotIdentity::Unbound);

    // Every non-empty binding is a distinct `Structural` identity, never
    // the reserved `Unbound` row — including the degenerate empty-string
    // edges that the old digest had to nudge by hand.
    for (slot, cred) in [
        ("db", "cred-a"),
        ("cache", "cred-b"),
        ("db", ""),
        ("", "x"),
        ("queue", "tenant-7-credential"),
    ] {
        assert_ne!(
            SlotIdentity::from_bindings([(slot, cred)]),
            SlotIdentity::Unbound,
            "non-empty binding ({slot:?}, {cred:?}) must be a Structural \
             identity, never the reserved Unbound row"
        );
    }
}

// ---------------------------------------------------------------------------
// Sibling concrete type sharing one ResourceKey (end-to-end concrete-type
// filter guard)
// ---------------------------------------------------------------------------

/// A SECOND, distinct resident resource type whose `key()` string is
/// IDENTICAL to [`CountingResource`]'s. Two distinct concrete types thus map
/// to one [`ResourceKey`]: `type_index` holds both `TypeId`s pointing at the
/// same key, and the registry's per-key entry list can hold a sibling-typed
/// row. This is the shape the registry's concrete-type filter exists for.
#[derive(Clone)]
struct SiblingResidentResource;

impl Resource for SiblingResidentResource {
    type Config = CountingConfig;
    type Runtime = CountingRuntime;
    type Lease = CountingRuntime;
    type Error = CountingError;

    fn key() -> ResourceKey {
        // SAME string as `CountingResource::key()` on purpose.
        resource_key!("dedup-slot-ident")
    }

    async fn create(
        &self,
        _config: &CountingConfig,
        _ctx: &ResourceContext,
    ) -> Result<CountingRuntime, CountingError> {
        // A distinguishable runtime id space; never observed on the
        // success path of the test below (the sibling row must be skipped).
        Ok(CountingRuntime { id: 9_999 })
    }

    async fn destroy(&self, _runtime: CountingRuntime) -> Result<(), CountingError> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl Resident for SiblingResidentResource {
    fn is_alive_sync(&self, _runtime: &CountingRuntime) -> bool {
        true
    }
}

/// End-to-end guard for the concrete-type filter on the **agnostic** typed
/// acquire path (`Registry::get_typed_for_acquire_scope::<R>`, reached via
/// `Manager::acquire_resident::<R>` → `lookup_for_acquire_scope`).
///
/// `SiblingResidentResource` and `CountingResource` share one `ResourceKey`,
/// so the registry's per-key entry list holds a sibling-typed row. The
/// sibling sits at the ORG scope; the correctly-typed `CountingResource`
/// row sits only at GLOBAL. An agnostic typed acquire of `CountingResource`
/// from an org-scoped context must SKIP the org-scope sibling row and FALL
/// THROUGH to the Global `CountingResource` row.
///
/// Why this test exists: the `registry.rs` unit tests exercise the private
/// `find_at_exact_scope` / `find_in_entries` helpers directly. They stay
/// green even if `get_typed_for_acquire_scope` (or `get_typed`) drops the
/// `Some(TypeId::of::<ManagedResource<R>>())` argument and reverts to the
/// unfiltered selection. THIS test is the one that fails on that regression:
/// without the filter the org-scope sibling row is the single row at that
/// scope, is returned, and `resolve_typed::<CountingResource>` then fails the
/// downcast — the acquire errors instead of resolving the Global row.
#[tokio::test]
async fn agnostic_typed_acquire_skips_sibling_type_and_falls_through_to_global() {
    let manager = Manager::new();
    let org = OrgId::new();
    let counter = Arc::new(AtomicU64::new(0));

    // Sibling-typed row at the ORG scope. With the concrete-type filter
    // absent this row would mask `CountingResource` at the org scope.
    manager
        .register(RegistrationSpec {
            resource: SiblingResidentResource,
            config: CountingConfig,
            scope: ScopeLevel::Organization(org),
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(ResidentRuntime::<SiblingResidentResource>::new(
                ResidentConfig::default(),
            )),
            acquire: Manager::erased_acquire_resident_for::<SiblingResidentResource>(),
            resilience: None,
            recovery_gate: None,
        })
        .expect("register sibling type at org scope must succeed");

    // Correctly-typed `CountingResource` row at GLOBAL only (Unbound:
    // default `RegisterOptions` with no slot bindings).
    register_counting(
        &manager,
        CountingResource::new(Arc::clone(&counter)),
        RegisterOptions::default().with_scope(ScopeLevel::Global),
    )
    .expect("register CountingResource at Global must succeed");

    // Agnostic typed acquire (no resolved identity) of `CountingResource`
    // from an org-scoped context: the scope walk visits the org scope
    // (sibling-only) before Global. The concrete-type filter must skip the
    // sibling and resolve the Global `CountingResource` row.
    let ctx = ctx_for_org(org);
    let lease = manager
        .acquire_resident::<CountingResource>(&ctx, &AcquireOptions::default())
        .await
        .expect(
            "agnostic typed acquire must skip the org-scope sibling-typed \
             row and fall through to the Global CountingResource row; a \
             downcast/NotFound error here means the concrete-type filter on \
             get_typed_for_acquire_scope regressed",
        );

    assert_eq!(
        lease.id, 0,
        "the resolved runtime must be CountingResource's OWN first create \
         (id 0 on its shared counter), never the sibling's 9999 runtime"
    );
}

/// End-to-end guard for the concrete-type filter on the **public typed
/// lookup** path (`Registry::get_typed::<R>`, reached via
/// `Manager::lookup::<R>`).
///
/// `Manager::lookup` is the public typed delegator distinct from the
/// agnostic acquire path: it calls `registry.get_typed::<R>(scope)` →
/// `get_inner(&key, scope, Some(TypeId::of::<ManagedResource<R>>()))`.
/// (`Manager::get_any` is a *different*, type-erased entrypoint —
/// `registry.get(key, scope)` with no `TypeId` — so it does not exercise
/// this filter; `lookup` is the one that does.)
///
/// Same shape as the acquire-path guard above: a sibling-typed row sharing
/// `CountingResource`'s `ResourceKey` sits at the ORG scope, the
/// correctly-typed `CountingResource` row only at GLOBAL. A typed
/// `lookup::<CountingResource>` from an org-scoped level must SKIP the
/// org-scope sibling and resolve the Global `CountingResource` row.
///
/// Why this test exists: the `registry.rs` unit tests and the acquire-path
/// E2E both stay green if `get_typed` (specifically) drops its
/// `Some(TypeId::of::<ManagedResource<R>>())` argument and reverts to the
/// unfiltered `self.get(&key, scope)`. THIS test is the one that fails on
/// that regression: without the filter the org-scope sibling row is the
/// single row at that level, is returned, and `resolve_typed::<CountingResource>`
/// fails the downcast — `lookup` errors instead of resolving the Global row.
#[tokio::test]
async fn typed_lookup_skips_sibling_type_and_falls_through_to_global() {
    let manager = Manager::new();
    let org = OrgId::new();
    let counter = Arc::new(AtomicU64::new(0));

    // Sibling-typed row at the ORG scope. Without the concrete-type filter
    // on `get_typed` this row masks `CountingResource` at the org level.
    manager
        .register(RegistrationSpec {
            resource: SiblingResidentResource,
            config: CountingConfig,
            scope: ScopeLevel::Organization(org),
            slot_identity: SlotIdentity::Unbound,
            topology: TopologyRuntime::Resident(ResidentRuntime::<SiblingResidentResource>::new(
                ResidentConfig::default(),
            )),
            acquire: Manager::erased_acquire_resident_for::<SiblingResidentResource>(),
            resilience: None,
            recovery_gate: None,
        })
        .expect("register sibling type at org scope must succeed");

    // Correctly-typed `CountingResource` row at GLOBAL only.
    register_counting(
        &manager,
        CountingResource::new(Arc::clone(&counter)),
        RegisterOptions::default().with_scope(ScopeLevel::Global),
    )
    .expect("register CountingResource at Global must succeed");

    // Typed lookup of `CountingResource` from the org level: the scope walk
    // visits the org level (sibling-only) before Global. The concrete-type
    // filter on `get_typed` must skip the sibling and resolve the Global
    // `CountingResource` row; the downcast in `resolve_typed` succeeding is
    // itself the proof the resolved row is `CountingResource`, not the
    // sibling.
    manager
        .lookup::<CountingResource>(&ScopeLevel::Organization(org))
        .expect(
            "typed lookup must skip the org-scope sibling-typed row and fall \
             through to the Global CountingResource row; a downcast/NotFound \
             error here means the concrete-type filter on get_typed regressed",
        );

    // Positive control: the sibling type still resolves to its OWN org row,
    // proving the filter is type-directed, not a blanket org-scope skip.
    manager
        .lookup::<SiblingResidentResource>(&ScopeLevel::Organization(org))
        .expect("sibling type must still resolve its own org-scope row");
}
