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
