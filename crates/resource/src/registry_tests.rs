use nebula_core::WorkspaceId;

use super::*;

struct FakeA;
struct FakeB;

macro_rules! impl_fake_handle {
    ($T:ty) => {
        #[async_trait::async_trait]
        impl ManagedHandle for $T {
            fn resource_key(&self) -> ResourceKey {
                ResourceKey::new("fake").unwrap()
            }
            fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
                self
            }
            fn managed_type_id(&self) -> TypeId {
                TypeId::of::<$T>()
            }
            fn set_phase(&self, _phase: crate::state::ResourcePhase) {}
            fn set_failed(&self, _kind: crate::error::ErrorKind, _reason: &str) {}
            // These registry-index fixtures own no lifecycle entries.
            fn begin_close(&self) {}
            fn abort_maintenance(&self) {}
            async fn close_retained(self: Arc<Self>) -> Result<(), Error> {
                Ok(())
            }

            async fn join_maintenance(&self) -> Result<(), Error> {
                Ok(())
            }
            fn phase(&self) -> crate::state::ResourcePhase {
                crate::state::ResourcePhase::Ready
            }
            fn topology_tag(&self) -> TopologyTag {
                TopologyTag::Resident
            }
            fn taint(&self) {}
            fn bump_revoke_epoch(&self) {}
            fn accepts_credential_slot_name(&self, _slot: &str) -> bool {
                true
            }
            fn pending_projection_hooks(
                &self,
            ) -> &std::sync::Mutex<std::collections::HashMap<String, (u64, u64)>> {
                unreachable!("this fixture does not install projections")
            }

            fn install_credential_slot(
                &self,
                _slot: &str,
                _guard: nebula_credential::ErasedCredentialGuard,
            ) -> Result<crate::SlotUpdate, crate::SlotInstallError> {
                unreachable!("registry lookup fake never installs credential slots")
            }
            fn revoke_credential_slot(
                &self,
                _slot: &str,
            ) -> Result<crate::SlotUpdate, crate::SlotInstallError> {
                unreachable!("registry lookup fake never revokes credential slots")
            }
            fn submit_on_refresh(
                self: Arc<Self>,
                _slot: &str,
                _timeout: std::time::Duration,
                _settlement: SlotHookSettlement,
                _admission: SlotHookAdmission,
            ) -> Result<AcceptedSlotHook, Error> {
                unreachable!("registry lookup fake never dispatches credential hooks")
            }
            fn submit_on_revoke(
                self: Arc<Self>,
                _slot: &str,
                _timeout: std::time::Duration,
                _settlement: SlotHookSettlement,
                _admission: SlotHookAdmission,
            ) -> Result<AcceptedSlotHook, Error> {
                unreachable!("registry lookup fake never dispatches credential hooks")
            }
            async fn wait_for_in_flight_drain(
                &self,
                _timeout: std::time::Duration,
            ) -> Result<(), u64> {
                Ok(())
            }
            fn admission_phase(&self) -> crate::topology::AdmissionPhase {
                crate::topology::AdmissionPhase::Ready
            }
            fn try_reserve_gate(&self) -> Result<(), crate::topology::Unavailable> {
                Ok(())
            }
            fn admission_load(&self) -> Option<crate::topology::Load> {
                None
            }
            async fn acquire(
                self: Arc<Self>,
                _mgr: Arc<crate::manager::Manager>,
                _ctx: ResourceContext,
                _opts: AcquireOptions,
            ) -> Result<Box<dyn Any + Send + Sync>, Error> {
                Err(Error::permanent(
                    "FakeA/FakeB: acquire not implemented for registry unit tests",
                ))
            }
        }
    };
}

impl_fake_handle!(FakeA);
impl_fake_handle!(FakeB);

fn ident(slot: &str, cred: &str) -> SlotIdentity {
    SlotIdentity::from_bindings([(slot, cred)])
}

#[test]
fn public_lookup_returns_read_only_diagnostics() {
    let registry = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    registry.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        ScopeLevel::Global,
        SlotIdentity::Unbound,
        Arc::new(FakeA),
    );

    let LookupOutcome::Found(view) = registry.get(&key, &ScopeLevel::Global) else {
        panic!("registered unambiguous row must return a diagnostic view");
    };
    assert_eq!(view.resource_key(), key);
    assert_eq!(view.phase(), crate::state::ResourcePhase::Ready);
    assert_eq!(view.topology_tag(), TopologyTag::Resident);
    assert_eq!(
        view.admission_phase(),
        crate::topology::AdmissionPhase::Ready
    );
    assert!(view.admission_load().is_none());
}

#[test]
fn register_replace_preserves_type_id_still_used_by_another_scope() {
    // Regression for a correctness hole: if scope A and scope B both
    // hold `TypeA`, replacing scope A with `TypeB` must NOT scrub
    // `TypeA -> key` from `type_index`, otherwise `get_typed::<TypeA>(B)`
    // would break.
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();

    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        ScopeLevel::Global,
        SlotIdentity::Unbound,
        Arc::new(FakeA),
    );
    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        ScopeLevel::Workspace(WorkspaceId::new()),
        SlotIdentity::Unbound,
        Arc::new(FakeA),
    );

    // Replace only the Global entry with FakeB. Workflow still
    // holds FakeA, so the TypeA row in type_index must survive.
    reg.register(
        key,
        TypeId::of::<FakeB>(),
        ScopeLevel::Global,
        SlotIdentity::Unbound,
        Arc::new(FakeB),
    );

    assert!(
        reg.type_index.contains_key(&TypeId::of::<FakeA>()),
        "TypeA row must survive because the Workspace scope still uses it",
    );
    assert!(reg.type_index.contains_key(&TypeId::of::<FakeB>()));
}

#[test]
fn register_replace_drops_stale_type_id_row() {
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let scope = ScopeLevel::Global;

    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        SlotIdentity::Unbound,
        Arc::new(FakeA),
    );
    assert!(reg.type_index.contains_key(&TypeId::of::<FakeA>()));

    // Replace at the same key+scope+slot_identity with a different
    // concrete type — same row, last-write-wins.
    reg.register(
        key,
        TypeId::of::<FakeB>(),
        scope,
        SlotIdentity::Unbound,
        Arc::new(FakeB),
    );

    // The stale TypeId row for FakeA must be gone (#382).
    assert!(
        !reg.type_index.contains_key(&TypeId::of::<FakeA>()),
        "stale TypeId for FakeA still in type_index after replace"
    );
    assert!(reg.type_index.contains_key(&TypeId::of::<FakeB>()));
}

#[test]
fn remove_for_removes_one_tenant_row_and_keeps_sibling() {
    // `remove_for` is the narrow, additive counterpart to `remove`
    // (which nukes every row under a key). Two tenants share
    // `(key, scope)` but resolve different credentials — removing one
    // resolved row must not disturb the other, and the shared
    // `type_index` entry must survive as long as ANY row still uses
    // that concrete type.
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let scope = ScopeLevel::Global;
    let id_a = ident("db", "cred-a");
    let id_b = ident("db", "cred-b");

    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        id_a.clone(),
        Arc::new(FakeA),
    );
    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        id_b.clone(),
        Arc::new(FakeA),
    );

    assert!(
        reg.remove_for(&key, &scope, &id_a).is_some(),
        "remove_for must report success for a row that exists"
    );

    // Tenant A's row is gone; tenant B's sibling row survives.
    assert!(matches!(
        reg.get_for(&key, &scope, &id_a),
        PinnedLookup::NotFound
    ));
    assert!(matches!(
        reg.get_for(&key, &scope, &id_b),
        PinnedLookup::Found(_)
    ));
    // `type_index` must still resolve FakeA — tenant B's row still
    // uses it (the #382 "don't scrub while a sibling still needs it"
    // discipline, mirrored from `register`).
    assert!(
        reg.type_index.contains_key(&TypeId::of::<FakeA>()),
        "type_index must survive remove_for while a sibling row of the \
         same concrete type remains"
    );

    // A second remove_for on the same (now-absent) row is a clean
    // no-op `false`, not a panic.
    assert!(reg.remove_for(&key, &scope, &id_a).is_none());

    // Removing the LAST row for this key must scrub the (now genuinely
    // stale) `type_index` entry too.
    assert!(reg.remove_for(&key, &scope, &id_b).is_some());
    assert!(
        !reg.type_index.contains_key(&TypeId::of::<FakeA>()),
        "type_index must be scrubbed once no row under the key uses \
         that concrete type anymore"
    );
    assert!(
        matches!(reg.get(&key, &scope), LookupOutcome::NotFound),
        "the key's entries row must be empty (and behave as NotFound) \
         once every row under it is removed via remove_for"
    );
}

#[test]
fn distinct_slot_identity_at_same_key_scope_is_a_distinct_row() {
    // Two registrations at the same key + scope but different resolved
    // slot identities must NOT collapse — the second does not replace
    // the first; both rows coexist. Identities are *structural*, so
    // "different resolved credential" is exact inequality, not a
    // (collidable) digest.
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let scope = ScopeLevel::Global;

    let id_a = ident("db", "cred-tenant-a");
    let id_b = ident("db", "cred-tenant-b");
    let id_unregistered = ident("db", "cred-tenant-c");

    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        id_a.clone(),
        Arc::new(FakeA),
    );
    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        id_b.clone(),
        Arc::new(FakeA),
    );

    // Each resolved identity pins its own row — `PinnedLookup`, no
    // `Ambiguous` variant exists on this path at all.
    assert!(matches!(
        reg.get_for(&key, &scope, &id_a),
        PinnedLookup::Found(_)
    ));
    assert!(matches!(
        reg.get_for(&key, &scope, &id_b),
        PinnedLookup::Found(_)
    ));
    // An identity that was never registered is NotFound, never an
    // accidental alias to a different tenant's row.
    assert!(matches!(
        reg.get_for(&key, &scope, &id_unregistered),
        PinnedLookup::NotFound
    ));
}

#[test]
fn pinned_lookup_resolves_exactly_one_row_or_not_found() {
    // The pinned lookup is 2-variant: exactly the resolved row, or
    // NotFound — never an alias to a sibling tenant's row, and no
    // `Ambiguous` variant exists to mishandle. (The typed acquire
    // walk `get_typed_for_acquire::<R>` shares this pinned resolution;
    // the `dedup_slot_identity` integration test covers it on a real
    // `Resource` end to end.)
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let scope = ScopeLevel::Global;

    let id_a = ident("db", "cred-a");
    let id_b = ident("db", "cred-b");

    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        id_a.clone(),
        Arc::new(FakeA),
    );
    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        id_b,
        Arc::new(FakeA),
    );

    assert!(matches!(
        reg.get_for(&key, &scope, &id_a),
        PinnedLookup::Found(_)
    ));
    assert!(matches!(
        reg.get_for(&key, &scope, &ident("db", "never-registered")),
        PinnedLookup::NotFound
    ));
}

#[test]
fn identity_agnostic_get_fails_closed_on_ambiguity() {
    // When two credential rows exist for the same (key, scope) and the
    // caller cannot disambiguate, the registry must refuse to pick one
    // (deny-by-default — never bleed one tenant's runtime to another).
    // The identity-agnostic path KEEPS the 3-variant `Ambiguous`
    // (AE6 fail-closed preserved).
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let scope = ScopeLevel::Global;

    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        ident("db", "cred-a"),
        Arc::new(FakeA),
    );
    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        ident("db", "cred-b"),
        Arc::new(FakeA),
    );

    match reg.get(&key, &scope) {
        LookupOutcome::Ambiguous { rows } => assert_eq!(rows, 2),
        other => panic!("expected Ambiguous, got a non-ambiguous outcome: {other:?}"),
    }
}

#[test]
fn identity_agnostic_get_returns_single_row() {
    // The historical single-row-per-(key,scope) path is unaffected:
    // exactly one row → Found, no ambiguity.
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let scope = ScopeLevel::Global;

    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        SlotIdentity::Unbound,
        Arc::new(FakeA),
    );

    assert!(matches!(reg.get(&key, &scope), LookupOutcome::Found(_)));
}

#[test]
fn pinned_finder_skips_sibling_typed_row_under_shared_key() {
    // Cross-type correctness gap: distinct concrete types can share one
    // `ResourceKey`. `type_index` only narrows a typed lookup to the
    // key — it does NOT prove a `(scope, slot_identity)` row under that
    // key is the requested type. A typed pinned lookup must SKIP a
    // sibling-typed row (continue), not return it and let the caller's
    // `downcast` fail (which would surface as a spurious `NotFound`).
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let scope = ScopeLevel::Global;
    let id = ident("db", "cred-shared");

    // One row under `key` at `(Global, id)` holding a `FakeB`.
    reg.register(
        key.clone(),
        TypeId::of::<FakeB>(),
        scope.clone(),
        id.clone(),
        Arc::new(FakeB),
    );

    let entries = reg.entries.get(&key).unwrap();

    // Untyped (`None`): the row is returned — the untyped caller hands
    // back the erased `Arc` and implies no concrete type.
    assert!(matches!(
        Registry::find_pinned_in_entries(&entries, &scope, &id, None),
        PinnedFind::Hit { .. }
    ));

    // Typed as `FakeB`: matches.
    assert!(matches!(
        Registry::find_pinned_in_entries(&entries, &scope, &id, Some(TypeId::of::<FakeB>())),
        PinnedFind::Hit { .. }
    ));

    // Typed as `FakeA`: the only row at `(Global, id)` is a `FakeB`, so
    // the sibling-typed row is skipped → `NotFound` (NOT a `FakeB`
    // handed to a `FakeA` caller that would fail to `downcast`).
    assert!(matches!(
        Registry::find_pinned_in_entries(&entries, &scope, &id, Some(TypeId::of::<FakeA>())),
        PinnedFind::NotFound
    ));
}

#[test]
fn agnostic_typed_finder_skips_sibling_typed_row_under_shared_key() {
    // Same cross-type gap on the *unpinned* (identity-agnostic) typed
    // path: `get_typed::<R>` / `get_typed_for_acquire_scope::<R>` resolve
    // `type_index` to a `ResourceKey`, but a sibling concrete type can
    // share that key. The concrete-type filter must apply to
    // `find_at_exact_scope` / `find_in_entries` too, or a sibling row
    // would (a) be handed to a typed caller whose `downcast` then fails,
    // or (b) anchor the effective scope and mask a correctly-typed
    // Global row, or (c) inflate the `Ambiguous` row count.
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let workspace = ScopeLevel::Workspace(WorkspaceId::new());

    // Sibling `FakeB` at the workspace scope; correctly-typed `FakeA`
    // only at Global. Both identity-agnostic (`Unbound`).
    reg.register(
        key.clone(),
        TypeId::of::<FakeB>(),
        workspace.clone(),
        SlotIdentity::Unbound,
        Arc::new(FakeB),
    );
    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        ScopeLevel::Global,
        SlotIdentity::Unbound,
        Arc::new(FakeA),
    );

    let entries = reg.entries.get(&key).unwrap();

    // Erased (`None`) at the workspace scope: the `FakeB` row is the
    // single row there and is returned — the untyped caller hands back
    // the erased `Arc` and implies no concrete type.
    assert!(matches!(
        Registry::find_at_exact_scope(&entries, &workspace, None, None),
        ScopeFind::Hit { .. }
    ));

    // Typed as `FakeA` at the workspace scope: the only row there is a
    // `FakeB`, so it is skipped → `NotFound` at this exact scope (NOT a
    // `FakeB` handed to a `FakeA` caller).
    assert!(matches!(
        Registry::find_at_exact_scope(&entries, &workspace, None, Some(TypeId::of::<FakeA>())),
        ScopeFind::NotFound
    ));

    // Masking regression: a typed-`FakeA` agnostic lookup *at the
    // workspace scope* must fall through to the correctly-typed Global
    // row, NOT stop at the sibling-`FakeB` workspace row.
    assert!(matches!(
        Registry::find_in_entries(&entries, &workspace, None, Some(TypeId::of::<FakeA>())),
        ScopeFind::Hit { .. }
    ));

    // Erased fall-through still resolves the nearer (workspace) row —
    // the type filter is the only behavior change.
    assert!(matches!(
        Registry::find_in_entries(&entries, &workspace, None, None),
        ScopeFind::Hit { .. }
    ));
}

#[test]
fn agnostic_typed_finder_does_not_inflate_ambiguity_with_sibling_rows() {
    // A sibling-typed row sharing the resolved `ResourceKey` must not
    // count toward the identity-agnostic `Ambiguous` fail-closed tally:
    // `Ambiguous` guards against same-type cross-tenant bleed, not a
    // different concrete type that this typed caller can never reach.
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let scope = ScopeLevel::Global;

    // One `FakeA` and one `FakeB` coexisting at the same (key, Global).
    // The row key is (key, scope, slot_identity) — registering both
    // under one identity would collapse last-write-wins regardless of
    // type, so distinct identities are required to get two real rows.
    let id_a = ident("db", "cred-a");
    let id_b = ident("db", "cred-b");
    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        id_a,
        Arc::new(FakeA),
    );
    reg.register(
        key.clone(),
        TypeId::of::<FakeB>(),
        scope.clone(),
        id_b,
        Arc::new(FakeB),
    );

    let entries = reg.entries.get(&key).unwrap();

    // Typed as `FakeA`: exactly one `FakeA` row → `Hit`, not
    // `Ambiguous` (the `FakeB` sibling is filtered out first).
    assert!(matches!(
        Registry::find_at_exact_scope(&entries, &scope, None, Some(TypeId::of::<FakeA>())),
        ScopeFind::Hit { .. }
    ));

    // Erased (`None`): both rows are visible → fail-closed `Ambiguous`
    // is preserved exactly as before (AE6).
    assert!(
        matches!(
            Registry::find_at_exact_scope(&entries, &scope, None, None),
            ScopeFind::Ambiguous { rows } if rows == 2
        ),
        "expected erased Ambiguous across both sibling rows"
    );
}

#[test]
fn pinned_finder_falls_back_to_global_when_exact_scope_has_only_other_tenant() {
    // Global-fallback gap: direct pinned lookup must agree with acquire
    // routing. If tenant B has an exact-scope row and tenant A only has
    // a Global row, the prior "pick effective scope by *any* entry at
    // that scope" decided the scope BEFORE consulting `want_identity`
    // and returned `NotFound` for tenant A — even though acquire (which
    // walks ancestor scopes down to Global with the identity pin) would
    // still find tenant A's Global row.
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let workspace = ScopeLevel::Workspace(WorkspaceId::new());

    let id_a = ident("db", "cred-tenant-a");
    let id_b = ident("db", "cred-tenant-b");

    // Tenant B: exact-scope row at `workspace`.
    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        workspace.clone(),
        id_b.clone(),
        Arc::new(FakeA),
    );
    // Tenant A: only a Global row.
    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        ScopeLevel::Global,
        id_a.clone(),
        Arc::new(FakeA),
    );

    // Tenant A asks at `workspace`: an entry exists at that scope (B's)
    // but none matches A → must fall back to A's Global row, NOT
    // `NotFound`. This is what acquire routing already does.
    assert!(matches!(
        reg.get_for(&key, &workspace, &id_a),
        PinnedLookup::Found(_)
    ));

    // Tenant B still resolves to its exact-scope row.
    assert!(matches!(
        reg.get_for(&key, &workspace, &id_b),
        PinnedLookup::Found(_)
    ));

    // Fail-closed preserved: an identity bound to neither row never
    // aliases a different tenant — Global fallback only matches the
    // requested identity exactly.
    assert!(matches!(
        reg.get_for(&key, &workspace, &ident("db", "cred-tenant-c")),
        PinnedLookup::NotFound
    ));
}

#[test]
fn pinned_global_fallback_is_identity_and_type_aware() {
    // Global fallback must match BOTH `want_identity` and (for typed
    // callers) the concrete type — a Global row of the right identity
    // but a sibling type must not be aliased back to a typed caller.
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let workspace = ScopeLevel::Workspace(WorkspaceId::new());
    let id = ident("db", "cred-shared");

    // Only a Global row, holding a `FakeB`.
    reg.register(
        key.clone(),
        TypeId::of::<FakeB>(),
        ScopeLevel::Global,
        id.clone(),
        Arc::new(FakeB),
    );

    let entries = reg.entries.get(&key).unwrap();

    // Untyped: workspace has no row → falls back to the Global row.
    assert!(matches!(
        Registry::find_pinned_in_entries(&entries, &workspace, &id, None),
        PinnedFind::Hit { .. }
    ));

    // Typed as `FakeA`: the Global row is a `FakeB` → fallback skips it
    // → `NotFound` (no wrong-typed alias via the Global fallback).
    assert!(matches!(
        Registry::find_pinned_in_entries(&entries, &workspace, &id, Some(TypeId::of::<FakeA>())),
        PinnedFind::NotFound
    ));
}

// The legacy `Opaque(u64)` / `slot_identity` digest assertions were
// removed with the deleted primitives (R15); structural identity is the
// sole row key.
#[test]
fn structurally_distinct_bindings_never_collide() {
    // The R15 guarantee at the registry level: two registrations with
    // structurally distinct bindings occupy distinct rows regardless
    // of what any hash of those bindings is — collision is impossible
    // by construction (exact string equality, no digest space).
    let reg = Registry::new();
    let key = ResourceKey::new("fake").unwrap();
    let scope = ScopeLevel::Global;

    let id_a = ident("db", "tenant-a-cred");
    let id_b = ident("db", "tenant-b-cred");
    assert_ne!(id_a, id_b, "distinct bindings are exact-unequal");

    reg.register(
        key.clone(),
        TypeId::of::<FakeA>(),
        scope.clone(),
        id_a.clone(),
        Arc::new(FakeA),
    );
    reg.register(
        key.clone(),
        TypeId::of::<FakeB>(),
        scope.clone(),
        id_b.clone(),
        Arc::new(FakeB),
    );

    // Two distinct rows, each pinned, neither aliasing the other.
    assert!(matches!(
        reg.get_for(&key, &scope, &id_a),
        PinnedLookup::Found(_)
    ));
    assert!(matches!(
        reg.get_for(&key, &scope, &id_b),
        PinnedLookup::Found(_)
    ));
    // A structurally-distinct identity never resolves another row.
    assert!(matches!(
        reg.get_for(&key, &scope, &ident("db", "tenant-c-cred")),
        PinnedLookup::NotFound
    ));
}
