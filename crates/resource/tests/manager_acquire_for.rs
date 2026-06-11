//! Slot-identity-pinned acquire (`acquire_*_for_identity`) + slot-rotation
//! (`refresh_slot_for_identity` / `revoke_slot_for_identity`) must resolve
//! the *specific* resolved registry row under a multi-tenant `(key,
//! scope)`.
//!
//! Companion to `dedup_slot_identity.rs` (which proves
//! `acquire_resident_for_identity`) and `manager_refresh_slot.rs`
//! (identity-agnostic rotation). Here two registrations of the same
//! resource type at the same `ScopeLevel` differ only in resolved per-slot
//! credential identity. The identity-pinned `_for_identity` paths must each
//! route to their own row (no cross-tenant runtime bleed); the
//! identity-agnostic path stays fail-closed (`Ambiguous`).
//!
//! Pooled is covered end-to-end (the most common topology); the folded
//! `Bounded` `_for_identity` methods are line-identical refactors of the
//! resident pattern proven in `dedup_slot_identity.rs` (same
//! `lookup_for_acquire_with` → shared `run_acquire`), so they are not
//! re-mocked here. `refresh_slot_for_identity` / `revoke_slot_for_identity`
//! (the ports the engine rotation fan-out drives) are covered directly.

use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

use nebula_core::{OrgId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
use nebula_resource::Pooled;
use nebula_resource::topology::{pooled::PoolProvider, resident::ResidentProvider};
use nebula_resource::{
    AcquireOptions, Manager, Provider, RegisterOptions, RegistrationSpec, ResourceConfig,
    ResourceContext, SlotIdentity,
    error::Error,
    resource::{HasCredentialSlots, ResourceMetadata},
    topology::{Resident, pooled::BrokenCheck},
};
use tokio_util::sync::CancellationToken;

// Custom error boilerplate removed — Resource lifecycle methods now return
// `crate::Error` directly (HasCredentialSlots redesign).

/// `fingerprint()` deliberately left at the `0` default: the row separation
/// must come from the resolved slot identity, never the author overriding
/// `fingerprint()` (a discipline-based defence, explicitly rejected).
#[derive(Clone)]
struct CountingConfig;

nebula_schema::impl_empty_has_schema!(CountingConfig);

impl ResourceConfig for CountingConfig {
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }

    fn fingerprint(&self) -> u64 {
        // Unit struct: all instances identical — constant 0 is correct.
        0
    }
}

/// Each `create` mints a unique runtime id from a shared counter so a
/// distinct `Resource::create` yields a distinguishable runtime — the
/// witness that `acquire_pooled_for_identity` resolved a *distinct* row per
/// tenant.
#[derive(Clone)]
struct PoolRes {
    create_counter: Arc<AtomicU64>,
}

#[async_trait::async_trait]
impl Provider for PoolRes {
    type Config = CountingConfig;
    type Instance = u64;
    type Topology = Pooled<Self>;

    fn key() -> ResourceKey {
        resource_key!("acquire-for-pool")
    }

    async fn create(&self, _config: &CountingConfig, _ctx: &ResourceContext) -> Result<u64, Error> {
        Ok(self.create_counter.fetch_add(1, Ordering::SeqCst))
    }

    async fn destroy(&self, _runtime: u64) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl HasCredentialSlots for PoolRes {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

impl PoolProvider for PoolRes {
    fn is_broken(&self, _runtime: &u64) -> BrokenCheck {
        BrokenCheck::Healthy
    }
}

fn pool_cfg() -> nebula_resource::topology::pooled::config::Config {
    nebula_resource::topology::pooled::config::Config {
        max_size: 4,
        ..Default::default()
    }
}

/// Register a `PoolRes` row, deriving scope + resolved-credential identity
/// from a [`RegisterOptions`] exactly the way the (now-deleted)
/// `register_pooled[_with]` shorthands did. Centralised so every
/// slot-identity strong-net below threads the identity through one path.
fn register_pool_res(
    manager: &Manager,
    resource: PoolRes,
    opts: RegisterOptions,
) -> Result<(), Error> {
    let fingerprint = CountingConfig.fingerprint();
    let slot_identity = opts.effective_slot_identity();
    manager.register(RegistrationSpec {
        resource,
        config: CountingConfig,
        scope: opts.scope,
        slot_identity,
        topology: Pooled::<PoolRes>::new(pool_cfg(), fingerprint),
        recovery_gate: opts.recovery_gate,
    })
}

/// Resident counterpart of [`register_pool_res`].
fn register_res_res(
    manager: &Manager,
    resource: ResRes,
    opts: RegisterOptions,
) -> Result<(), Error> {
    let slot_identity = opts.effective_slot_identity();
    manager.register(RegistrationSpec {
        resource,
        config: CountingConfig,
        scope: opts.scope,
        slot_identity,
        topology: Resident::<ResRes>::new(nebula_resource::ResidentConfig::default()),
        recovery_gate: opts.recovery_gate,
    })
}

fn ctx_for_org(org: OrgId) -> ResourceContext {
    let scope = Scope {
        org_id: Some(org),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

/// Resolved `(slot, credential)` bindings for the two pooled tenants. Two
/// distinct resolved-credential identities at the same `(key, scope)`
/// occupy two distinct registry rows — each its own `ManagedResource` with
/// its own pooled runtime (the structural cross-tenant barrier).
const POOL_A: &[(&str, &str)] = &[("db", "cred-pool-tenant-a")];
const POOL_B: &[(&str, &str)] = &[("db", "cred-pool-tenant-b")];

/// Structural [`SlotIdentity`] for [`POOL_A`] / [`POOL_B`] — the
/// collision-free row key the acquire / taint ports pin on.
fn pool_a_id() -> SlotIdentity {
    SlotIdentity::from_bindings(POOL_A.iter().copied())
}
fn pool_b_id() -> SlotIdentity {
    SlotIdentity::from_bindings(POOL_B.iter().copied())
}

/// Register two pooled tenants (distinct resolved slot identity) under ONE
/// `(key, scope)`. The shared `create_counter` proves each row drove its
/// own independent `Resource::create` (distinct runtime ids), not one
/// shared runtime aliased to two tenants.
fn two_tenant(org: OrgId, a: &[(&str, &str)], b: &[(&str, &str)]) -> (Manager, Arc<AtomicU64>) {
    let scope = ScopeLevel::Organization(org);
    let create_counter = Arc::new(AtomicU64::new(0));
    let manager = Manager::new();

    for bindings in [a, b] {
        register_pool_res(
            &manager,
            PoolRes {
                create_counter: Arc::clone(&create_counter),
            },
            RegisterOptions::default()
                .with_scope(scope.clone())
                .with_slot_bindings(bindings),
        )
        .expect("register tenant must succeed");
    }

    (manager, create_counter)
}

// ───────────────────────────────────────────────────────────────────────
// Resident resource for the slot-rotation routing proofs.
//
// `refresh_slot_for_identity` / `revoke_slot_for_identity` only add row
// *resolution* (`registry.get_for`) over the verified `refresh_slot` /
// `revoke_slot` dispatch (`manager_refresh_slot.rs`). A resident resource
// keeps its runtime in `rt.current()` after one acquire (no idle-queue
// release race, unlike Pool), so the routing assertion is deterministic —
// the same shape `manager_refresh_slot.rs` uses.
// ───────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct ResRes {
    create_counter: Arc<AtomicU64>,
    refresh_saw: Arc<AtomicU64>,
    revoke_saw: Arc<AtomicU64>,
    refresh_total: Arc<AtomicUsize>,
    id_tag: u64,
}

#[async_trait::async_trait]
impl Provider for ResRes {
    type Config = CountingConfig;
    type Instance = u64;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("acquire-for-resident")
    }

    async fn create(&self, _config: &CountingConfig, _ctx: &ResourceContext) -> Result<u64, Error> {
        Ok(self.create_counter.fetch_add(1, Ordering::SeqCst))
    }

    async fn destroy(&self, _runtime: u64) -> Result<(), Error> {
        Ok(())
    }

    async fn on_credential_refresh(&self, _slot: &str, _runtime: &u64) -> Result<(), Error> {
        self.refresh_total.fetch_add(1, Ordering::SeqCst);
        self.refresh_saw.store(self.id_tag, Ordering::SeqCst);
        Ok(())
    }

    async fn on_credential_revoke(&self, _slot: &str, _runtime: &u64) -> Result<(), Error> {
        self.revoke_saw.store(self.id_tag, Ordering::SeqCst);
        Ok(())
    }

    fn metadata() -> ResourceMetadata {
        ResourceMetadata::from_key(&Self::key())
    }
}

impl HasCredentialSlots for ResRes {
    fn credential_slot_epoch(&self) -> u64 {
        0
    }
}

#[async_trait::async_trait]
impl ResidentProvider for ResRes {
    fn is_alive_sync(&self, _runtime: &u64) -> bool {
        true
    }
}

/// Resolved `(slot, credential)` bindings for the two resident tenants —
/// the resident analogue of [`POOL_A`] / [`POOL_B`].
const RES_A: &[(&str, &str)] = &[("db", "cred-res-tenant-a")];
const RES_B: &[(&str, &str)] = &[("db", "cred-res-tenant-b")];

fn res_a_id() -> SlotIdentity {
    SlotIdentity::from_bindings(RES_A.iter().copied())
}
fn res_b_id() -> SlotIdentity {
    SlotIdentity::from_bindings(RES_B.iter().copied())
}

/// Register two resident tenants (distinct resolved slot identity) under
/// ONE `(key, scope)` and warm each runtime (resident persists it in
/// `rt.current()`), so the rotation hook has a live `&Runtime`. `id_tag`
/// (a stable per-tenant witness number) lets a hook prove which resolved
/// row it ran on. Returns `(manager, refresh_saw, revoke_saw,
/// refresh_total)`.
async fn two_tenant_resident(
    org: OrgId,
    a: (&[(&str, &str)], u64),
    b: (&[(&str, &str)], u64),
) -> (Manager, Arc<AtomicU64>, Arc<AtomicU64>, Arc<AtomicUsize>) {
    let scope = ScopeLevel::Organization(org);
    let create_counter = Arc::new(AtomicU64::new(0));
    let refresh_saw = Arc::new(AtomicU64::new(0));
    let revoke_saw = Arc::new(AtomicU64::new(0));
    let refresh_total = Arc::new(AtomicUsize::new(0));
    let manager = Manager::new();

    for (bindings, id_tag) in [a, b] {
        register_res_res(
            &manager,
            ResRes {
                create_counter: Arc::clone(&create_counter),
                refresh_saw: Arc::clone(&refresh_saw),
                revoke_saw: Arc::clone(&revoke_saw),
                refresh_total: Arc::clone(&refresh_total),
                id_tag,
            },
            RegisterOptions::default()
                .with_scope(scope.clone())
                .with_slot_bindings(bindings),
        )
        .expect("register resident tenant");

        // Resident materializes its shared runtime lazily on first acquire
        // and keeps it in `rt.current()` — touch each tenant's pinned row
        // so the rotation hook has a live `&Runtime`.
        let ctx = ctx_for_org(org);
        let id = SlotIdentity::from_bindings(bindings.iter().copied());
        let _g = manager
            .acquire_resident_for_identity::<ResRes>(&ctx, &AcquireOptions::default(), &id)
            .await
            .expect("warm resident tenant runtime");
    }

    (manager, refresh_saw, revoke_saw, refresh_total)
}

/// `acquire_pooled_for_identity` must resolve the row pinned by the
/// resolved slot identity — tenant A's binding never aliases tenant B's
/// runtime.
#[tokio::test]
async fn acquire_pooled_for_resolves_the_pinned_row() {
    let org = OrgId::new();
    let (manager, _create_counter) = two_tenant(org, POOL_A, POOL_B);
    let ctx = ctx_for_org(org);
    let a = pool_a_id();
    let b = pool_b_id();

    let la = manager
        .acquire_pooled_for_identity::<PoolRes>(&ctx, &AcquireOptions::default(), &a)
        .await
        .expect("acquire tenant A");
    let lb = manager
        .acquire_pooled_for_identity::<PoolRes>(&ctx, &AcquireOptions::default(), &b)
        .await
        .expect("acquire tenant B");

    assert_ne!(
        *la, *lb,
        "two registrations of the same type at the same scope with distinct \
         resolved slot identities must NOT share a pooled runtime instance"
    );

    // Re-acquiring A returns A's pool, never B's (binding is stable).
    drop(la);
    let la2 = manager
        .acquire_pooled_for_identity::<PoolRes>(&ctx, &AcquireOptions::default(), &a)
        .await
        .expect("re-acquire tenant A");
    let lb_id = *lb;
    assert_ne!(
        *la2, lb_id,
        "tenant A's pinned pool must never resolve to tenant B's instance"
    );
}

/// The identity-agnostic `acquire_pooled` stays fail-closed under a
/// multi-tenant `(key, scope)` (the no-identity caller must not pick a row).
#[tokio::test]
async fn acquire_pooled_identity_agnostic_fails_closed_when_multi_tenant() {
    use nebula_error::{Classify, ErrorCategory};

    let org = OrgId::new();
    let (manager, _create_counter) = two_tenant(org, POOL_A, POOL_B);
    let ctx = ctx_for_org(org);

    let err = manager
        .acquire_pooled::<PoolRes>(&ctx, &AcquireOptions::default())
        .await
        .expect_err("identity-agnostic acquire under multi-tenant must fail closed");
    assert_eq!(
        err.category(),
        ErrorCategory::Conflict,
        "ambiguous multi-tenant acquire must be a non-retryable client conflict, got: {err}"
    );
    assert!(
        !err.is_retryable(),
        "Ambiguous is a permanent caller error, not retryable"
    );
}

/// A single-tenant `(key, scope)` is unaffected: the identity-agnostic path
/// still resolves the one row.
#[tokio::test]
async fn acquire_pooled_identity_agnostic_single_tenant_ok() {
    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);
    let manager = Manager::new();
    register_pool_res(
        &manager,
        PoolRes {
            create_counter: Arc::new(AtomicU64::new(0)),
        },
        RegisterOptions::default()
            .with_scope(scope)
            .with_slot_bindings(&[("db", "cred-only-tenant")]),
    )
    .expect("register single tenant");

    let ctx = ctx_for_org(org);
    let _guard = manager
        .acquire_pooled::<PoolRes>(&ctx, &AcquireOptions::default())
        .await
        .expect("single-tenant identity-agnostic acquire must still succeed");
}

/// `refresh_slot_for_identity` must drive the rotation hook of the
/// *specific* resolved row, not an arbitrary tenant's — and the
/// identity-agnostic `refresh_slot` must fail closed under multi-tenant.
#[tokio::test]
async fn refresh_slot_for_routes_to_the_resolved_row() {
    use nebula_error::{Classify, ErrorCategory};

    let org = OrgId::new();
    let (a_tag, b_tag) = (0xA1, 0xB2);
    let (manager, refresh_saw, _vs, refresh_total) =
        two_tenant_resident(org, (RES_A, a_tag), (RES_B, b_tag)).await;
    let key = ResRes::key();
    let scope = ScopeLevel::Organization(org);

    // Identity-agnostic refresh_slot must fail closed (two rows share
    // (key, scope), caller gave no identity).
    let amb = manager
        .refresh_slot(&key, scope.clone(), "db")
        .await
        .expect_err("identity-agnostic refresh under multi-tenant must fail closed");
    assert_eq!(
        amb.category(),
        ErrorCategory::Conflict,
        "ambiguous refresh must be a client conflict, got: {amb}"
    );

    // Slot-identity-pinned refresh routes to tenant B's row only.
    manager
        .refresh_slot_for_identity(&key, scope.clone(), "db", &res_b_id())
        .await
        .expect("pinned refresh of tenant B must succeed");
    assert_eq!(
        refresh_saw.load(Ordering::SeqCst),
        b_tag,
        "refresh_slot_for_identity must have driven tenant B's resolved row"
    );
    assert_eq!(
        refresh_total.load(Ordering::SeqCst),
        1,
        "exactly one row's hook ran — the sibling was not touched"
    );

    // And tenant A's row, pinned by A's identity.
    manager
        .refresh_slot_for_identity(&key, scope, "db", &res_a_id())
        .await
        .expect("pinned refresh of tenant A must succeed");
    assert_eq!(
        refresh_saw.load(Ordering::SeqCst),
        a_tag,
        "refresh_slot_for_identity must have driven tenant A's resolved row"
    );
    assert_eq!(
        refresh_total.load(Ordering::SeqCst),
        2,
        "second pinned refresh ran exactly one more hook"
    );
}

/// `refresh_slot_for_identity` with an identity that was never registered
/// is a typed `NotFound` — never an accidental alias to another tenant's
/// row.
#[tokio::test]
async fn refresh_slot_for_unknown_identity_is_not_found() {
    use nebula_error::{Classify, ErrorCategory};

    let org = OrgId::new();
    let (manager, refresh_saw, _vs, refresh_total) =
        two_tenant_resident(org, (RES_A, 0xA1), (RES_B, 0xB2)).await;
    let key = ResRes::key();
    let scope = ScopeLevel::Organization(org);

    let unknown = SlotIdentity::from_bindings([("db", "cred-never-registered")]);
    let err = manager
        .refresh_slot_for_identity(&key, scope, "db", &unknown)
        .await
        .expect_err("unknown slot identity must error, never alias a tenant");
    assert_eq!(
        err.category(),
        ErrorCategory::NotFound,
        "unregistered slot identity must classify NotFound, got: {err}"
    );
    assert_eq!(
        refresh_total.load(Ordering::SeqCst),
        0,
        "no row's hook must run for an unknown identity"
    );
    assert_eq!(
        refresh_saw.load(Ordering::SeqCst),
        0,
        "no tenant row was touched"
    );
}

/// `revoke_slot_for_identity` must taint + run the revoke hook of the
/// pinned row only, leaving the multi-tenant sibling acquirable.
#[tokio::test]
async fn revoke_slot_for_revokes_only_the_pinned_row() {
    let org = OrgId::new();
    let (a_tag, b_tag) = (0xCAFE, 0xF00D);
    let (manager, _rs, revoke_saw, _rt) =
        two_tenant_resident(org, (RES_A, a_tag), (RES_B, b_tag)).await;
    let key = ResRes::key();
    let scope = ScopeLevel::Organization(org);
    let ctx = ctx_for_org(org);

    // Revoke only tenant A's resolved row.
    manager
        .revoke_slot_for_identity(&key, scope.clone(), "db", &res_a_id())
        .await
        .expect("pinned revoke of tenant A must succeed");
    assert_eq!(
        revoke_saw.load(Ordering::SeqCst),
        a_tag,
        "revoke_slot_for_identity must have driven tenant A's resolved row"
    );

    // Tenant A's pinned row is now tainted by the revoke: a subsequent
    // identity-pinned acquire for A must be rejected (not silently still
    // serviceable). This proves the revoke actually tainted A's row —
    // without it, "B is acquirable" alone would also pass if the revoke
    // were a no-op. `Unavailable` is the post-revoke/tainted category
    // (mirrors `manager_refresh_slot.rs`'s post-revoke assertion).
    use nebula_error::{Classify, ErrorCategory};
    let a_after = manager
        .acquire_resident_for_identity::<ResRes>(&ctx, &AcquireOptions::default(), &res_a_id())
        .await
        .expect_err("tenant A must NOT be acquirable after its pinned row is revoked");
    assert_eq!(
        a_after.category(),
        ErrorCategory::Unavailable,
        "a revoked/tainted pinned row must reject acquires with Unavailable, got: {a_after}"
    );

    // Tenant B's row is a distinct registry row (distinct slot_identity):
    // A's revoke taints A's row only, so B remains acquirable.
    let _guard = manager
        .acquire_resident_for_identity::<ResRes>(&ctx, &AcquireOptions::default(), &res_b_id())
        .await
        .expect("tenant B must remain acquirable after tenant A's revoke");
}

/// #690 review (CodeRabbit, CRITICAL) — `warmup_pool` must NOT create
/// fresh pool entries on a credential a concurrent revoke tainted after
/// the pre-lookup gate. `warmup` runs `R::create` against the resolved
/// credential, so it is acquire-like and must enforce the same
/// revoke-vs-acquire post-taint re-check the `run_acquire` pipeline got in
/// #679.
///
/// Deterministic model of the race: `taint_slot` applies the taint
/// **synchronously** (phase 1) — exactly the state a concurrent
/// `revoke_slot` establishes by the time warmup would proceed. With the
/// resource tainted, `warmup_pool` must reject (Revoked/Unavailable) and
/// `R::create` must run **zero** times — no fresh pool instance is ever
/// minted on the revoked credential.
#[tokio::test]
async fn warmup_pool_rejected_after_revoke_taint_creates_no_entries() {
    use nebula_error::{Classify, ErrorCategory};

    let create_counter = Arc::new(AtomicU64::new(0));
    let manager = Manager::new();
    register_pool_res(
        &manager,
        PoolRes {
            create_counter: Arc::clone(&create_counter),
        },
        RegisterOptions::default(),
    )
    .expect("register pooled (Global) must succeed");

    // Phase-1 synchronous taint — the exact state a racing `revoke_slot`
    // leaves the row in before `warmup_pool` could call `rt.warmup`.
    let _tainted = manager
        .taint_slot(&PoolRes::key(), ScopeLevel::Global, "db")
        .expect("taint_slot must resolve the registered pooled row");

    let ctx = ResourceContext::minimal(Scope::default(), CancellationToken::new());
    let err = manager
        .warmup_pool::<PoolRes>(&ctx)
        .await
        .expect_err("warmup_pool on a revoke-tainted resource must be rejected");
    assert_eq!(
        err.category(),
        ErrorCategory::Unavailable,
        "tainted warmup must reject with Revoked/Unavailable, got: {err}"
    );
    assert_eq!(
        create_counter.load(Ordering::SeqCst),
        0,
        "warmup_pool must create ZERO fresh pool instances on a revoked \
         credential (post-taint re-check, #679 pattern applied to warmup)"
    );
}

/// Collapsing the five `run_*_acquire` into one generic `run_acquire` must
/// not perturb the `InFlightCounter::new` → `reject_if_tainted_post_count`
/// ordering that closes the revoke-vs-acquire TOCTOU. A synchronous
/// `taint_slot_for_identity` models the state a concurrent `revoke_slot`
/// establishes by the time the acquire would proceed; the
/// slot-identity-pinned acquire must reject (`Revoked` → `Unavailable`)
/// and never hand out a guard on the tainted row. Behavior-preservation
/// pin for the collapse — the post-count re-check now lives once in
/// `run_acquire`, not five times.
#[tokio::test]
async fn tainted_pinned_acquire_rejected_after_run_acquire_collapse() {
    use nebula_error::{Classify, ErrorCategory};

    let org = OrgId::new();
    let bindings: &[(&str, &str)] = &[("db", "cred-pinned-tenant")];
    let slot_id = SlotIdentity::from_bindings(bindings.iter().copied());
    let scope = ScopeLevel::Organization(org);
    let create_counter = Arc::new(AtomicU64::new(0));
    let manager = Manager::new();
    register_pool_res(
        &manager,
        PoolRes {
            create_counter: Arc::clone(&create_counter),
        },
        RegisterOptions::default()
            .with_scope(scope.clone())
            .with_slot_bindings(bindings),
    )
    .expect("register single pinned tenant");

    // Synchronous taint == the state a racing `revoke_slot` leaves the row
    // in before the acquire's in-flight increment + post-count re-check.
    let _tainted = manager
        .taint_slot_for_identity(&PoolRes::key(), scope, "db", &slot_id)
        .expect("taint_slot_for_identity must resolve the pinned pooled row");

    let ctx = ctx_for_org(org);
    let err = manager
        .acquire_pooled_for_identity::<PoolRes>(&ctx, &AcquireOptions::default(), &slot_id)
        .await
        .expect_err("acquire on a revoke-tainted pinned row must be rejected");
    assert_eq!(
        err.category(),
        ErrorCategory::Unavailable,
        "post-count taint re-check (preserved verbatim in the single \
         run_acquire) must reject with Revoked/Unavailable, got: {err}"
    );
    assert_eq!(
        create_counter.load(Ordering::SeqCst),
        0,
        "a tainted acquire must never reach Resource::create through the \
         collapsed pipeline"
    );
}

// ───────────────────────────────────────────────────────────────────────
// Pool two-tenant routing consistency check.
//
// Verifies `acquire_pooled_for_identity` routes to the correct row under
// multi-tenant registration, and that `revoke_slot_for_identity` taints
// only the pinned row without affecting the sibling tenant.  Mirrors the
// structural shape of the resident two-tenant net but exercises the Pool
// acquire path.
// ───────────────────────────────────────────────────────────────────────

/// `acquire_pooled_for_identity` routes each pinned acquire to its own row
/// under multi-tenant registration: distinct slot identities at the same
/// `(key, scope)` must never share a pooled runtime.  The lease is the
/// u64 minted by `create_counter.fetch_add`, so distinct values prove
/// distinct `Resource::create` calls.  `revoke_slot_for_identity` must
/// taint only the pinned row, leaving the sibling acquirable.
#[tokio::test]
async fn pool_two_tenant_pinned_acquires_route_independently() {
    use nebula_error::{Classify, ErrorCategory};

    const EXTRA_A: &[(&str, &str)] = &[("db", "cred-pool-extra-a")];
    const EXTRA_B: &[(&str, &str)] = &[("db", "cred-pool-extra-b")];

    let org = OrgId::new();
    let scope = ScopeLevel::Organization(org);
    let create_counter = Arc::new(AtomicU64::new(0));
    let manager = Manager::new();

    // Two tenants under the same (key, scope) with distinct slot identities.
    let id_a = SlotIdentity::from_bindings(EXTRA_A.iter().copied());
    let id_b = SlotIdentity::from_bindings(EXTRA_B.iter().copied());

    for bindings in [EXTRA_A, EXTRA_B] {
        register_pool_res(
            &manager,
            PoolRes {
                create_counter: Arc::clone(&create_counter),
            },
            RegisterOptions::default()
                .with_scope(scope.clone())
                .with_slot_bindings(bindings),
        )
        .expect("both pool tenants must register under distinct slot identities");
    }

    let ctx = ctx_for_org(org);

    // Each identity-pinned acquire resolves its own row → distinct runtime ids.
    let la = manager
        .acquire_pooled_for_identity::<PoolRes>(&ctx, &AcquireOptions::default(), &id_a)
        .await
        .expect("acquire tenant A must succeed");
    let lb = manager
        .acquire_pooled_for_identity::<PoolRes>(&ctx, &AcquireOptions::default(), &id_b)
        .await
        .expect("acquire tenant B must succeed");

    // The counter minted 2 distinct ids: each row called Resource::create once.
    assert_eq!(
        create_counter.load(Ordering::SeqCst),
        2,
        "two distinct rows must each call Resource::create exactly once"
    );
    // Lease values must differ — a shared runtime would produce the same id.
    assert_ne!(
        *la, *lb,
        "tenant A and tenant B must resolve distinct pool runtimes, never a shared one"
    );
    drop(la);
    drop(lb);

    // Identity-agnostic acquire under multi-tenant stays fail-closed.
    let amb = manager
        .acquire_pooled::<PoolRes>(&ctx, &AcquireOptions::default())
        .await
        .expect_err("identity-agnostic pooled acquire under multi-tenant must fail closed");
    assert_eq!(
        amb.category(),
        ErrorCategory::Conflict,
        "ambiguous multi-tenant acquire must be a non-retryable Conflict, got: {amb}"
    );

    // Revoke only tenant A's pinned row.
    let key = PoolRes::key();
    manager
        .revoke_slot_for_identity(&key, scope.clone(), "db", &id_a)
        .await
        .expect("pinned revoke of tenant A must succeed");

    // Tenant A is now tainted: its row must reject subsequent acquires.
    let a_after = manager
        .acquire_pooled_for_identity::<PoolRes>(&ctx, &AcquireOptions::default(), &id_a)
        .await
        .expect_err("tenant A must NOT be acquirable after its pinned row is revoked");
    assert_eq!(
        a_after.category(),
        ErrorCategory::Unavailable,
        "revoked pinned row must reject with Unavailable, not silently serve, got: {a_after}"
    );

    // Tenant B is a distinct row (distinct slot_identity): A's revoke must
    // not taint it.
    let lb2 = manager
        .acquire_pooled_for_identity::<PoolRes>(&ctx, &AcquireOptions::default(), &id_b)
        .await
        .expect("tenant B must remain acquirable after tenant A's revoke");
    assert_ne!(
        *lb2, 0,
        "tenant B's row must still serve its own runtime after A's revoke"
    );
}
