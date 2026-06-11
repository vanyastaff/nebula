//! Type-erased registry for managed resources.
//!
//! [`Registry`] stores managed resources indexed by [`ResourceKey`] and
//! [`TypeId`], supporting scope-aware lookup and typed downcasting.

use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use async_trait::async_trait;
use dashmap::DashMap;
use nebula_core::{ResourceKey, Scope, ScopeLevel};

use crate::{
    context::{ResourceContext, scope_levels_for_acquire},
    dedup::SlotIdentity,
    error::Error,
    options::AcquireOptions,
    resource::Provider,
    runtime::managed::ManagedResource,
    topology_tag::TopologyTag,
};

/// Crate-private seal: only `nebula-resource` can name `sealed::Sealed`,
/// so [`ManagedHandle`] is **not implementable downstream**.
///
/// `ManagedHandle` is engine-internal: the only purpose is to let the
/// [`Registry`] store heterogeneous `ManagedResource<R>` behind one
/// `dyn ManagedHandle`, and the sole implementor is the blanket
/// `impl<R: Provider>` below. Sealing makes that a *structural*
/// guarantee rather than a convention — adding a required method (e.g.
/// the per-resource-drain hook) can never be a downstream compile-break,
/// because no downstream impl can exist. The
/// `LookupOutcome::Found(Arc<dyn ManagedHandle>)` surface stays usable
/// (callers only *consume* the trait object); they just cannot *implement*
/// it.
mod sealed {
    /// Sealed marker. Implemented only by the crate-internal blanket
    /// `impl<R: Provider>` for `ManagedResource<R>`.
    pub trait Sealed {}
}

/// Type-erased trait for managed resources stored in the [`Registry`].
///
/// Every `ManagedResource<R>` implements this trait, allowing the registry
/// to store heterogeneous resource types behind a single `dyn ManagedHandle`.
///
/// **Sealed (engine-internal).** This trait has a private `sealed::Sealed`
/// supertrait, so it can only be implemented inside `nebula-resource` (by
/// the blanket `impl<R: Provider>`). It is an engine-internal erasure
/// boundary, **not** a downstream extension point — new required methods
/// may be added without it being a semver-breaking change for consumers
/// (they only ever hold `Arc<dyn ManagedHandle>` via
/// [`LookupOutcome::Found`], never implement it).
///
/// `#[async_trait]` is applied because lifecycle dispatch goes through
/// `dyn ManagedHandle` at runtime; native async-fn-in-trait (RPITIT)
/// produces associated `Future` types that are not object-safe.
#[async_trait]
pub trait ManagedHandle: sealed::Sealed + Send + Sync + 'static {
    /// Returns the resource key for this managed resource.
    fn resource_key(&self) -> ResourceKey;

    /// Returns a reference to `self` as `&dyn Any` for downcasting.
    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;

    /// Returns the concrete `TypeId` used as the secondary index key.
    ///
    /// For real `ManagedResource<R>` this is `TypeId::of::<ManagedResource<R>>()`.
    /// Used by [`Registry::register`] to scrub stale rows from `type_index`
    /// when an entry is replaced in place (#382).
    fn managed_type_id(&self) -> TypeId;

    /// Lifecycle phase mutation — drives phase transitions on all registered
    /// resources without needing a typed handle, which matters during
    /// graceful shutdown where only the type-erased registry iteration is
    /// available.
    fn set_phase(&self, phase: crate::state::ResourcePhase);

    /// Terminal failure transition.
    ///
    /// Transitions the resource to [`ResourcePhase::Failed`] and records
    /// the supplied human-readable reason in `last_error`. Used by
    /// `Manager::set_phase_all_failed` so `DrainTimeoutPolicy::Abort` can
    /// signal per-resource failure without needing typed access to each
    /// entry.
    ///
    /// [`ResourcePhase::Failed`]: crate::state::ResourcePhase::Failed
    fn set_failed(&self, reason: &str);

    /// Current lifecycle phase — diagnostic-only; typed callers should prefer
    /// `ManagedResource::status().phase` after a successful downcast.
    fn phase(&self) -> crate::state::ResourcePhase;

    /// Topology tag — used by `Manager::{refresh,revoke}_slot` to label
    /// the rotation tracing span without a typed downcast.
    fn topology_tag(&self) -> TopologyTag;

    /// Resource-level taint (credential revoke).
    ///
    /// `Manager::revoke_slot` takes a `ResourceKey`, not a generic `R`, so
    /// it taints through the erased registry view. Forwards to
    /// `ManagedResource::taint`, which sets the same flag the typed
    /// `acquire_*` funnel checks.
    fn taint(&self);

    /// Credential-revoke epoch bump.
    ///
    /// Bumped in the same synchronous pre-`.await` step as [`Self::taint`].
    /// Forwards to `ManagedResource::bump_revoke_epoch`, which advances the
    /// pooled topology's revoke counter so every pool return-to-idle path
    /// fences an instance authenticated with the revoked credential (a no-op
    /// for single-runtime topologies, which have no idle queue).
    fn bump_revoke_epoch(&self);

    /// Per-slot refresh dispatch.
    ///
    /// `#[async_trait]` boxes the future for `dyn`-safety. Forwards to
    /// `ManagedResource::dispatch_slot_hook` with `refresh = true`, which
    /// borrows the live runtime per topology and invokes the resource's
    /// `&self` hook.
    ///
    /// # Cancel Safety
    ///
    /// This method is cancel-safe. The resource taint and revoke-epoch bump
    /// are applied synchronously by the caller before this future is polled.
    /// Dropping the returned future after taint leaves the resource
    /// consistently marked as tainted — no partial-taint state is possible
    /// and new acquires remain rejected.
    async fn dispatch_on_refresh(&self, slot: &str) -> Result<(), Error>;

    /// Per-slot revoke dispatch (symmetric to [`Self::dispatch_on_refresh`];
    /// forwards to `ManagedResource::dispatch_slot_hook` with `refresh = false`).
    ///
    /// # Cancel Safety
    ///
    /// This method is cancel-safe. The resource taint and revoke-epoch bump
    /// are performed synchronously before any `.await` point (in the caller's
    /// `taint_slot_for_identity` phase). Dropping the returned future after
    /// taint leaves the resource consistently marked as tainted — no
    /// partial-taint state is possible, new acquires are still rejected, and
    /// the credential is never silently un-revoked.
    async fn dispatch_on_revoke(&self, slot: &str) -> Result<(), Error>;

    /// Bounded drain of **this resource's own** in-flight acquires.
    ///
    /// Waits on this row's per-resource counter — *not* the manager-wide
    /// `drain_tracker` — so a revoke is isolated from in-flight traffic to
    /// unrelated resources (per-resource revoke deferral).
    /// `Err(outstanding)` carries the counter snapshot at the moment the
    /// timer fired.
    async fn wait_for_in_flight_drain(&self, timeout: std::time::Duration) -> Result<(), u64>;

    /// Type-erased acquire for this row.
    ///
    /// Called by `Manager::acquire_any` after the single registry scope
    /// walk resolves this row; the implementation downcasts `self` to the
    /// concrete `ManagedResource<R>` and dispatches into the topology
    /// pipeline. Receives `Arc<crate::Manager>` because `ManagedResource<R>`
    /// does not own the manager; the manager passes a clone of itself.
    async fn acquire(
        self: Arc<Self>,
        mgr: Arc<crate::manager::Manager>,
        ctx: ResourceContext,
        opts: AcquireOptions,
    ) -> Result<Box<dyn Any + Send + Sync>, Error>;
}

// The one and only `Sealed` impl: every `ManagedResource<R>` (and
// nothing else, anywhere) — this is what makes `ManagedHandle`
// non-implementable downstream.
impl<R: Provider> sealed::Sealed for ManagedResource<R> {}

#[async_trait]
impl<R: Provider + crate::resource::HasCredentialSlots + Send + Sync + 'static> ManagedHandle
    for ManagedResource<R>
{
    fn resource_key(&self) -> ResourceKey {
        R::key()
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn managed_type_id(&self) -> TypeId {
        TypeId::of::<ManagedResource<R>>()
    }

    fn set_phase(&self, phase: crate::state::ResourcePhase) {
        ManagedResource::set_phase(self, phase);
    }

    fn set_failed(&self, reason: &str) {
        ManagedResource::set_failed(self, reason.to_owned());
    }

    fn phase(&self) -> crate::state::ResourcePhase {
        self.status().phase
    }

    fn topology_tag(&self) -> TopologyTag {
        self.topology.tag()
    }

    fn taint(&self) {
        ManagedResource::taint(self);
    }

    fn bump_revoke_epoch(&self) {
        ManagedResource::bump_revoke_epoch(self);
    }

    async fn dispatch_on_refresh(&self, slot: &str) -> Result<(), Error> {
        self.dispatch_slot_hook(slot, true).await
    }

    async fn dispatch_on_revoke(&self, slot: &str) -> Result<(), Error> {
        self.dispatch_slot_hook(slot, false).await
    }

    async fn wait_for_in_flight_drain(&self, timeout: std::time::Duration) -> Result<(), u64> {
        ManagedResource::wait_for_in_flight_drain(self, timeout).await
    }

    async fn acquire(
        self: Arc<Self>,
        mgr: Arc<crate::manager::Manager>,
        ctx: ResourceContext,
        opts: AcquireOptions,
    ) -> Result<Box<dyn Any + Send + Sync>, Error> {
        let this = Arc::clone(&self);
        self.topology.dispatch_acquire(this, mgr, ctx, opts).await
    }
}

/// Outcome of an **identity-agnostic** registry lookup.
///
/// The `Ambiguous` arm is the security-relevant one: when a caller that
/// does not know the resolved slot identity asks for `(key, scope)` and
/// more than one credential row exists there, the registry refuses to pick
/// one (which would alias one tenant's runtime to another). Callers map
/// `Ambiguous` to a typed deny-by-default error — fail closed, never bleed.
///
/// The **slot-identity-pinned** lookups
/// ([`get_for`](Registry::get_for) /
/// `get_typed_for_acquire`)
/// return [`PinnedLookup`] instead — a 2-variant type with **no
/// `Ambiguous`** arm, because a resolved [`SlotIdentity`] addresses exactly
/// one row by construction, so ambiguity is unrepresentable there rather
/// than a runtime branch a caller could mis-handle.
pub enum LookupOutcome {
    /// Exactly one row matched — here it is.
    Found(Arc<dyn ManagedHandle>),
    /// No row matched the key/scope.
    NotFound,
    /// Multiple distinct credential rows exist for the resolved
    /// `(key, scope)` and the caller supplied no slot identity to
    /// disambiguate. Returns the number of competing rows for diagnostics.
    Ambiguous {
        /// How many distinct credential rows competed.
        rows: usize,
    },
}

/// Outcome of a **slot-identity-pinned** registry lookup.
///
/// A resolved [`SlotIdentity`] pins exactly one `(scope, slot_identity)`
/// row, so a pinned lookup is unambiguous *by construction*: a caller that
/// resolved tenant A's credential can only ever reach tenant A's row, never
/// tenant B's, and never "more than one matched". There is therefore **no
/// `Ambiguous` variant** — the cross-tenant-bleed failure mode the
/// identity-agnostic [`LookupOutcome::Ambiguous`] guards against cannot
/// occur on this path, so it is made type-unrepresentable rather than a
/// runtime arm that downstream code must remember to fail closed on. An
/// unknown pin is [`PinnedLookup::NotFound`] (never an accidental alias to
/// a different tenant's row).
pub enum PinnedLookup {
    /// Exactly one row matched the pinned `(scope, slot_identity)`.
    Found(Arc<dyn ManagedHandle>),
    /// No row matched the key/scope/identity. Never an alias to another
    /// resolved credential's row.
    NotFound,
}

/// Outcome of a registry acquire lookup (same semantics as [`LookupOutcome`]).
pub(crate) enum AcquireLookupOutcome {
    /// Exactly one row matched — the already-resolved [`ManagedHandle`].
    ///
    /// The row is read from the same [`RegistryEntry`] in one scope walk so
    /// `acquire_any` downcasts it directly without re-walking the `DashMap`
    /// at the matched scope: one resolution, not two.
    Found {
        /// The managed-resource row resolved by this single scope walk.
        managed: Arc<dyn ManagedHandle>,
    },
    /// No row matched the key/scope/identity.
    NotFound,
    /// More than one row at the resolved scope without a slot identity pin.
    Ambiguous {
        /// How many distinct credential rows competed.
        rows: usize,
    },
}

/// A single entry in the registry, associating a `(scope, slot_identity)`
/// row with a managed resource.
///
/// `slot_identity` is the resolved per-slot credential identity (see
/// [`SlotIdentity`]). Two registrations at the same key + scope but a
/// *different* `slot_identity` are distinct rows with distinct runtimes —
/// the structural barrier against cross-tenant runtime bleed. Equality is
/// exact and structural ([`SlotIdentity`] derives `Eq`), so two distinct
/// resolved binding sets can never collapse onto one row.
struct RegistryEntry {
    scope: ScopeLevel,
    slot_identity: SlotIdentity,
    managed: Arc<dyn ManagedHandle>,
}

enum ScopeFind {
    Hit { managed: Arc<dyn ManagedHandle> },
    NotFound,
    Ambiguous { rows: usize },
}

/// Result of a **pinned** scope lookup.
///
/// Deliberately 2-variant: a resolved [`SlotIdentity`] pins exactly one
/// `(scope, slot_identity)` row, so "more than one matched" is not a
/// reachable state on the pinned path — there is no `Ambiguous` variant to
/// forget to fail closed on. This is the type-level half of the
/// [`PinnedLookup`] guarantee.
enum PinnedFind {
    Hit { managed: Arc<dyn ManagedHandle> },
    NotFound,
}

/// Type-erased storage for all registered resources.
///
/// Provides two lookup paths:
/// - **By key + scope**: `get()` finds the best-matching entry for a given [`ResourceKey`] and
///   [`ScopeLevel`].
/// - **By type**: `get_typed()` uses a secondary [`TypeId`] index for typed lookup with automatic
///   downcasting.
pub struct Registry {
    /// Primary index: ResourceKey -> list of entries (one per scope).
    entries: DashMap<ResourceKey, Vec<RegistryEntry>>,
    /// Secondary index: TypeId -> ResourceKey (for typed lookup).
    type_index: DashMap<TypeId, ResourceKey>,
}

impl Registry {
    /// Creates an empty registry.
    pub fn new() -> Self {
        Self {
            entries: DashMap::new(),
            type_index: DashMap::new(),
        }
    }

    /// Registers a managed resource under the given key, type, scope, and
    /// resolved slot identity.
    ///
    /// An entry is replaced in place **only** when an existing row matches
    /// the same `(scope, slot_identity)`. A registration at the same
    /// `(key, scope)` but a *different* `slot_identity` (i.e. a different
    /// resolved per-slot credential) is appended as a **distinct** row with
    /// its own runtime — it does not overwrite the other tenant's row. This
    /// is the structural barrier against cross-tenant runtime bleed: two
    /// resolved credentials can never collapse onto one shared runtime.
    pub fn register(
        &self,
        key: ResourceKey,
        type_id: TypeId,
        scope: ScopeLevel,
        slot_identity: SlotIdentity,
        managed: Arc<dyn ManagedHandle>,
    ) {
        // Lock order is **strictly one-way**: `entries → (release) → type_index`.
        //
        // `get_typed` takes the `type_index` shard read lock first and only
        // then touches `entries`. If `register` ever held both dashmap
        // shards simultaneously in the opposite order, two concurrent
        // callers (one here, one in `get_typed`) could each be waiting on
        // the shard the other already holds — a classic lock-ordering
        // reversal. We prevent that by doing all `entries` work in a
        // scoped block, dropping the guard, and only *then* touching
        // `type_index`.
        //
        // #382 nuance: it's not enough to compare the replaced entry's
        // prior `TypeId` to the new one. If *another* scope under the
        // same key still holds a `ManagedResource<OldR>` instance, we
        // must NOT remove `OldR -> key` from `type_index` — doing so
        // would break `get_typed::<OldR>` for that other scope. So we
        // scan the rest of the entries while still holding the guard
        // and only mark the stale row for removal if nobody else uses
        // it.
        let stale_type_id = {
            let mut entries = self.entries.entry(key.clone()).or_default();

            // Row identity is `(scope, slot_identity)` — NOT `scope` alone.
            // A registration that resolved a different credential
            // (different `slot_identity`) at the same `(key, scope)` must
            // NOT replace the existing tenant's row; it becomes a separate
            // row with its own runtime.
            if let Some(pos) = entries
                .iter()
                .position(|e| e.scope == scope && e.slot_identity == slot_identity)
            {
                let prev_type_id = entries[pos].managed.managed_type_id();
                entries[pos] = RegistryEntry {
                    scope,
                    slot_identity,
                    managed,
                };

                if prev_type_id != type_id
                    && !entries
                        .iter()
                        .any(|e| e.managed.managed_type_id() == prev_type_id)
                {
                    Some(prev_type_id)
                } else {
                    None
                }
            } else {
                entries.push(RegistryEntry {
                    scope,
                    slot_identity,
                    managed,
                });
                None
            }
            // entries guard drops here.
        };

        if let Some(stale) = stale_type_id {
            self.type_index.remove_if(&stale, |_, k| k == &key);
        }
        self.type_index.insert(type_id, key);
    }

    /// Looks up a managed resource by key and scope (slot-identity
    /// agnostic).
    ///
    /// Returns the entry whose scope matches `scope` exactly, falling back
    /// to [`ScopeLevel::Global`]. This path does **not** know the resolved
    /// slot identity, so it is **fail-closed on ambiguity**: if more than
    /// one credential row exists for the resolved scope it returns
    /// [`LookupOutcome::Ambiguous`] rather than silently picking one (which
    /// would be a cross-tenant bleed). Callers that know the resolved slot
    /// identity must use [`get_for`](Self::get_for).
    ///
    /// Untyped: returns the erased `Arc` without downcasting, so no
    /// concrete-type constraint is implied. [`get_typed`](Self::get_typed)
    /// goes through the same shared private core with the concrete-type
    /// filter so a sibling type sharing the resolved [`ResourceKey`] cannot
    /// mask a correctly-typed row.
    pub fn get(&self, key: &ResourceKey, scope: &ScopeLevel) -> LookupOutcome {
        self.get_inner(key, scope, None)
    }

    /// Shared identity-agnostic lookup core.
    ///
    /// `concrete_type` carries the optional concrete-type constraint:
    /// `Some(TypeId::of::<ManagedResource<R>>())` for the typed entry point
    /// [`get_typed`](Self::get_typed) (so a sibling-typed row sharing the
    /// resolved [`ResourceKey`] is skipped instead of returned and then
    /// `downcast`-failed, which would surface as a spurious `NotFound`
    /// masking a correctly-typed row at an ancestor/Global scope), `None`
    /// for the untyped [`get`](Self::get) callers that return the erased
    /// `Arc` directly. The fail-closed-on-ambiguity contract is unchanged:
    /// two same-type rows the caller cannot disambiguate still report
    /// [`LookupOutcome::Ambiguous`].
    fn get_inner(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        concrete_type: Option<TypeId>,
    ) -> LookupOutcome {
        let Some(entries) = self.entries.get(key) else {
            return LookupOutcome::NotFound;
        };
        match Self::find_in_entries(&entries, scope, None, concrete_type) {
            ScopeFind::Hit { managed, .. } => LookupOutcome::Found(managed),
            ScopeFind::NotFound => LookupOutcome::NotFound,
            ScopeFind::Ambiguous { rows } => LookupOutcome::Ambiguous { rows },
        }
    }

    /// Looks up the erased acquire hook for `(key, scope bag, slot_identity)`.
    ///
    /// Walks [`scope_levels_for_acquire`] from most specific to Global so
    /// org/workspace rows remain visible under execution-scoped contexts.
    ///
    /// Returns the 3-variant [`AcquireLookupOutcome`] (it **keeps**
    /// `Ambiguous`): with [`SlotIdentity::Unbound`] this walk is the
    /// identity-agnostic acquire path, so two competing rows at a level
    /// must still fail closed rather than alias one tenant's runtime. A
    /// non-`Unbound` identity pins exactly one row and cannot be ambiguous,
    /// but the variant stays because the same method serves the
    /// agnostic-`Unbound` callers.
    pub(crate) fn get_acquire_for(
        &self,
        key: &ResourceKey,
        scope: &Scope,
        slot_identity: &SlotIdentity,
    ) -> AcquireLookupOutcome {
        let Some(entries) = self.entries.get(key) else {
            return AcquireLookupOutcome::NotFound;
        };
        for level in scope_levels_for_acquire(scope) {
            // Acquire path: keyed only by `ResourceKey` (no concrete `R`),
            // so no concrete-type constraint — `None`.
            match Self::find_at_exact_scope(&entries, &level, Some(slot_identity), None) {
                ScopeFind::Hit { managed } => {
                    // `managed` came from one scope walk — carry the row out
                    // so `acquire_any` calls `managed.acquire(...)` directly
                    // instead of re-walking the `DashMap` at `level`.
                    return AcquireLookupOutcome::Found { managed };
                },
                ScopeFind::Ambiguous { rows } => {
                    return AcquireLookupOutcome::Ambiguous { rows };
                },
                ScopeFind::NotFound => {
                    if slot_identity.is_unbound()
                        && Self::scope_has_cred_bound_rows_without_unbound(&entries, &level, None)
                    {
                        return AcquireLookupOutcome::NotFound;
                    }
                    continue;
                },
            }
        }
        AcquireLookupOutcome::NotFound
    }

    /// Typed managed-resource lookup for acquire paths, pinned to a resolved
    /// slot identity.
    ///
    /// Walks [`scope_levels_for_acquire`] with the pinned finder at each
    /// level (no within-level Global fallback). Matches
    /// [`get_acquire_for`](Self::get_acquire_for) so the erased hook and
    /// typed row cannot diverge when Global and ancestor-scoped rows
    /// coexist. Returns [`PinnedLookup`] (no `Ambiguous`): a resolved
    /// identity pins exactly one row.
    ///
    /// Constrained to the concrete `ManagedResource<R>`: `type_index` only
    /// proves the resolved [`ResourceKey`], and distinct types can share
    /// one key, so a `(scope, slot_identity)` row of a *sibling* type is
    /// skipped (the scope walk continues) instead of returned and then
    /// failing the caller's `downcast` — which would otherwise short-circuit
    /// to `NotFound` and hide a correctly-typed row at an ancestor scope.
    pub(crate) fn get_typed_for_acquire<R: Provider>(
        &self,
        scope: &Scope,
        slot_identity: &SlotIdentity,
    ) -> PinnedLookup {
        let type_id = TypeId::of::<ManagedResource<R>>();
        let Some(key) = self.type_index.get(&type_id) else {
            return PinnedLookup::NotFound;
        };
        let Some(entries) = self.entries.get(&*key) else {
            return PinnedLookup::NotFound;
        };
        for level in scope_levels_for_acquire(scope) {
            match Self::find_pinned_at_exact_scope(&entries, &level, slot_identity, Some(type_id)) {
                PinnedFind::Hit { managed, .. } => return PinnedLookup::Found(managed),
                PinnedFind::NotFound => {
                    if slot_identity.is_unbound()
                        && Self::scope_has_cred_bound_rows_without_unbound(
                            &entries,
                            &level,
                            Some(type_id),
                        )
                    {
                        return PinnedLookup::NotFound;
                    }
                    continue;
                },
            }
        }
        PinnedLookup::NotFound
    }

    /// [`get_typed_for_acquire`](Self::get_typed_for_acquire) without a
    /// resolved slot identity (fail-closed on ambiguity at each level).
    ///
    /// Keeps the 3-variant [`LookupOutcome`]: this is the identity-agnostic
    /// acquire walk, so it must still report `Ambiguous`.
    ///
    /// Constrained to the concrete `ManagedResource<R>`: `type_index` only
    /// proves the resolved [`ResourceKey`], and distinct types can share one
    /// key, so a sibling-typed row at an exact scope is skipped (the scope
    /// walk continues) instead of returned and then `downcast`-failed —
    /// without the filter that masks a correctly-typed ancestor/Global row.
    pub(crate) fn get_typed_for_acquire_scope<R: Provider>(&self, scope: &Scope) -> LookupOutcome {
        let type_id = TypeId::of::<ManagedResource<R>>();
        let Some(key) = self.type_index.get(&type_id) else {
            return LookupOutcome::NotFound;
        };
        let Some(entries) = self.entries.get(&*key) else {
            return LookupOutcome::NotFound;
        };
        for level in scope_levels_for_acquire(scope) {
            match Self::find_at_exact_scope(&entries, &level, None, Some(type_id)) {
                ScopeFind::Hit { managed, .. } => return LookupOutcome::Found(managed),
                ScopeFind::Ambiguous { rows } => return LookupOutcome::Ambiguous { rows },
                ScopeFind::NotFound => continue,
            }
        }
        LookupOutcome::NotFound
    }

    /// At `level`, cred-bound rows exist but the caller asked for the
    /// [`SlotIdentity::Unbound`] (no-resolved-slots) row.
    ///
    /// When `concrete_type` is `Some`, only rows of that concrete type
    /// count: a cred-bound row of a *sibling* type sharing the resolved
    /// [`ResourceKey`] must not block the `Unbound` lookup of the requested
    /// type from falling through to an ancestor/Global row (it is not a
    /// competing row for *this* type). `None` keeps the
    /// identity-agnostic-key semantics for untyped callers.
    fn scope_has_cred_bound_rows_without_unbound(
        entries: &[RegistryEntry],
        level: &ScopeLevel,
        concrete_type: Option<TypeId>,
    ) -> bool {
        entries.iter().any(|e| {
            e.scope == *level
                && !e.slot_identity.is_unbound()
                && Self::entry_type_matches(e, concrete_type)
        })
    }

    /// Looks up a managed resource by key, scope, and a resolved slot
    /// identity.
    ///
    /// Selects the row whose `(scope, slot_identity)` matches exactly
    /// (scope falls back to [`ScopeLevel::Global`]). Because the row is
    /// pinned by `slot_identity` there is never ambiguity: a caller that
    /// resolved tenant A's credential can only ever reach tenant A's row.
    /// Hence the [`PinnedLookup`] return — `Ambiguous` is unrepresentable.
    pub fn get_for(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &SlotIdentity,
    ) -> PinnedLookup {
        // Untyped: the caller hands back the erased `Arc` without
        // downcasting, so no concrete type is implied (`None`).
        self.get_for_inner(key, scope, slot_identity, None)
    }

    /// Shared pinned-by-`(scope, slot_identity)` lookup core.
    ///
    /// `concrete_type` carries the optional concrete-type constraint:
    /// `Some(TypeId::of::<ManagedResource<R>>())` for the typed entry
    /// points (so a sibling-typed row sharing the resolved [`ResourceKey`]
    /// is skipped instead of returned-then-`downcast`-failed), `None` for
    /// the untyped [`get_for`](Self::get_for) callers that return the
    /// erased `Arc` directly.
    fn get_for_inner(
        &self,
        key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &SlotIdentity,
        concrete_type: Option<TypeId>,
    ) -> PinnedLookup {
        let Some(entries) = self.entries.get(key) else {
            return PinnedLookup::NotFound;
        };
        match Self::find_pinned_in_entries(&entries, scope, slot_identity, concrete_type) {
            PinnedFind::Hit { managed, .. } => PinnedLookup::Found(managed),
            PinnedFind::NotFound => PinnedLookup::NotFound,
        }
    }

    /// Typed lookup: finds the resource for type `R` and downcasts to
    /// `Arc<ManagedResource<R>>` (slot-identity agnostic).
    ///
    /// Inherits [`get`](Self::get)'s fail-closed-on-ambiguity contract.
    /// Constrained to the concrete `ManagedResource<R>`: `type_index` only
    /// proves the resolved [`ResourceKey`], and distinct types can share one
    /// key, so a `(scope, slot_identity)` row of a *sibling* type under that
    /// key is skipped (the scope walk continues) rather than returned and
    /// then failing the caller's `downcast` — without the filter that would
    /// surface as a spurious `NotFound` masking a correctly-typed row at an
    /// ancestor/Global scope.
    pub fn get_typed<R: Provider>(&self, scope: &ScopeLevel) -> LookupOutcome {
        let type_id = TypeId::of::<ManagedResource<R>>();
        let Some(key) = self.type_index.get(&type_id) else {
            return LookupOutcome::NotFound;
        };
        self.get_inner(&key, scope, Some(type_id))
    }

    /// Removes all entries for the given key.
    ///
    /// Returns `true` if the key existed and was removed, `false` otherwise.
    /// Also removes the type index entry if it points to this key.
    pub fn remove(&self, key: &ResourceKey) -> bool {
        let existed = self.entries.remove(key).is_some();
        if existed {
            self.type_index.retain(|_type_id, k| k != key);
        }
        existed
    }

    /// Returns all registered resource keys.
    pub fn keys(&self) -> Vec<ResourceKey> {
        self.entries.iter().map(|r| r.key().clone()).collect()
    }

    /// Returns every registered managed resource across all scopes.
    ///
    /// Used by the manager to drive lifecycle transitions (e.g. shifting
    /// every resource to `Draining` / `ShuttingDown` during graceful
    /// shutdown, #387) without needing typed access to each entry.
    pub(crate) fn all_managed(&self) -> Vec<Arc<dyn ManagedHandle>> {
        let mut out = Vec::new();
        for row in &self.entries {
            for entry in row.value() {
                out.push(Arc::clone(&entry.managed));
            }
        }
        out
    }

    /// Returns `true` if a resource with the given key is registered.
    pub fn contains(&self, key: &ResourceKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Removes all entries from the registry.
    ///
    /// This drops every `Arc<dyn ManagedHandle>`, releasing their
    /// resources (including `Arc<ReleaseQueue>` references).
    pub fn clear(&self) {
        self.entries.clear();
        self.type_index.clear();
    }

    /// Lookup at an exact [`ScopeLevel`] only (no ancestor or Global
    /// fallback).
    ///
    /// With `want_identity = Some(id)`: returns the single row at that
    /// scope whose `slot_identity == *id` (`Hit`/`NotFound` only — a
    /// resolved identity pins one row). With `want_identity = None`
    /// (identity-agnostic): `Hit` iff exactly one row exists at the scope,
    /// `Ambiguous` if two or more — the registry refuses to silently alias
    /// one tenant's runtime to another. (`rows` counts entries at that
    /// scope, not distinct `slot_identity` values.)
    ///
    /// `concrete_type` applies the optional concrete-type constraint (see
    /// [`entry_type_matches`](Self::entry_type_matches)): a sibling-typed row
    /// sharing the resolved [`ResourceKey`] is filtered out *before* the
    /// single-row / ambiguity reasoning, so it neither aliases a typed caller
    /// nor inflates the `Ambiguous` row count. `None` keeps the erased
    /// semantics.
    fn find_at_exact_scope(
        entries: &[RegistryEntry],
        scope: &ScopeLevel,
        want_identity: Option<&SlotIdentity>,
        concrete_type: Option<TypeId>,
    ) -> ScopeFind {
        let mut at_scope = entries
            .iter()
            .filter(|e| e.scope == *scope && Self::entry_type_matches(e, concrete_type));

        if let Some(id) = want_identity {
            return match at_scope.find(|e| &e.slot_identity == id) {
                Some(entry) => ScopeFind::Hit {
                    managed: Arc::clone(&entry.managed),
                },
                None => ScopeFind::NotFound,
            };
        }

        let Some(first) = at_scope.next() else {
            return ScopeFind::NotFound;
        };
        let extra = at_scope.count();
        if extra == 0 {
            ScopeFind::Hit {
                managed: Arc::clone(&first.managed),
            }
        } else {
            ScopeFind::Ambiguous { rows: 1 + extra }
        }
    }

    /// Scope-aware, identity-agnostic lookup within a list of entries.
    ///
    /// Resolves the effective scope first (exact match, else
    /// [`ScopeLevel::Global`] fallback) so the scope-precedence rule is
    /// applied before any reasoning — a Global-scoped row of the wrong
    /// credential must not shadow a correctly-scoped one. Returns
    /// [`ScopeFind::Found`] iff exactly one row exists at the effective
    /// scope and [`ScopeFind::Ambiguous`] if two or more — the registry
    /// refuses to silently alias one tenant's runtime to another.
    ///
    /// `concrete_type` is applied to the effective-scope resolution itself,
    /// not only the final selection: a sibling-typed row at the requested
    /// scope must **not** anchor the effective scope there and shadow a
    /// correctly-typed ancestor/Global row of the requested type (the
    /// cross-type masking failure mode). `None` keeps the erased semantics.
    fn find_in_entries(
        entries: &[RegistryEntry],
        scope: &ScopeLevel,
        want_identity: Option<&SlotIdentity>,
        concrete_type: Option<TypeId>,
    ) -> ScopeFind {
        // Resolve the effective scope: exact match wins; otherwise fall
        // back to Global. Scope precedence is decided BEFORE slot identity,
        // but AFTER the concrete-type filter — a wrong-typed row at the
        // requested scope must not anchor the scope and mask a correctly
        // typed Global row.
        let effective_scope = if entries
            .iter()
            .any(|e| e.scope == *scope && Self::entry_type_matches(e, concrete_type))
        {
            scope.clone()
        } else if *scope != ScopeLevel::Global
            && entries.iter().any(|e| {
                e.scope == ScopeLevel::Global && Self::entry_type_matches(e, concrete_type)
            })
        {
            ScopeLevel::Global
        } else {
            return ScopeFind::NotFound;
        };

        Self::find_at_exact_scope(entries, &effective_scope, want_identity, concrete_type)
    }

    /// `true` if `entry` satisfies the optional concrete-type constraint.
    ///
    /// `type_index` only narrows a typed lookup to a [`ResourceKey`]; it
    /// does **not** prove every row under that key is the requested
    /// `ManagedResource<R>`. Distinct concrete types can share one
    /// [`ResourceKey`], so a `(scope, slot_identity)` row found under the
    /// resolved key may be a sibling type. Every typed lookup — pinned
    /// ([`get_typed_for_acquire`](Self::get_typed_for_acquire)) **and**
    /// identity-agnostic ([`get_typed`](Self::get_typed) /
    /// [`get_typed_for_acquire_scope`](Self::get_typed_for_acquire_scope)) —
    /// passes `Some(TypeId::of::<ManagedResource<R>>())` so a sibling-typed
    /// row is **skipped** (the walk continues) rather than returned and then
    /// failing the caller's `downcast` — the difference between a correct
    /// `NotFound`/ancestor-row and a spurious `NotFound` masking a
    /// correctly-typed row at a later scope. Untyped callers
    /// ([`get`](Self::get) / [`get_for`](Self::get_for) /
    /// [`get_acquire_for`](Self::get_acquire_for)) pass `None`: they hand
    /// back the erased `Arc` without downcasting, so no concrete type is
    /// implied.
    fn entry_type_matches(entry: &RegistryEntry, concrete_type: Option<TypeId>) -> bool {
        match concrete_type {
            Some(tid) => entry.managed.managed_type_id() == tid,
            None => true,
        }
    }

    /// Pinned lookup at an exact [`ScopeLevel`] (no ancestor/Global
    /// fallback).
    ///
    /// Returns the single row at that scope whose `slot_identity` equals
    /// the resolved `want_identity` **and** (when `concrete_type` is
    /// `Some`) whose stored `ManagedResource` is exactly that type. Rows of
    /// a sibling concrete type sharing the resolved [`ResourceKey`] are
    /// skipped (see [`entry_type_matches`](Self::entry_type_matches)), so a
    /// typed caller never gets a wrong-typed row that would only fail later
    /// on `downcast`. The result type ([`PinnedFind`]) has **no
    /// `Ambiguous`** variant: a resolved [`SlotIdentity`] addresses exactly
    /// one `(scope, slot_identity)` row by construction, so ambiguity is
    /// unrepresentable on the pinned path rather than a runtime branch a
    /// caller could mishandle (the cross-tenant-bleed failure mode the
    /// agnostic path guards against cannot occur here).
    fn find_pinned_at_exact_scope(
        entries: &[RegistryEntry],
        scope: &ScopeLevel,
        want_identity: &SlotIdentity,
        concrete_type: Option<TypeId>,
    ) -> PinnedFind {
        match entries.iter().find(|e| {
            e.scope == *scope
                && &e.slot_identity == want_identity
                && Self::entry_type_matches(e, concrete_type)
        }) {
            Some(entry) => PinnedFind::Hit {
                managed: Arc::clone(&entry.managed),
            },
            None => PinnedFind::NotFound,
        }
    }

    /// Scope-aware **pinned** lookup within a list of entries.
    ///
    /// Tries the requested `scope` with `want_identity` (and, when
    /// `concrete_type` is `Some`, the concrete type) first; on `NotFound`
    /// and a non-`Global` scope it retries at [`ScopeLevel::Global`].
    /// Scope selection is therefore **identity-aware**: the Global fallback
    /// is consulted whenever the requested scope holds no row matching
    /// `want_identity`, not skipped merely because *some* (different-tenant)
    /// row exists at that scope.
    ///
    /// This keeps direct pinned lookup
    /// ([`get_for`](Self::get_for)) in agreement with the
    /// acquire-routing walk
    /// ([`get_typed_for_acquire`](Self::get_typed_for_acquire) /
    /// [`get_acquire_for`](Self::get_acquire_for)), which walks
    /// ancestor scopes down to Global with the identity pin at each level:
    /// the prior "pick the effective scope by *any* entry existing there,
    /// before consulting `want_identity`" returned `NotFound` for an
    /// exact-scope row of tenant B even though the caller's tenant-A row
    /// lived at Global and acquire would have found it.
    ///
    /// Still **fail-closed**: the only fallback is a Global row that
    /// matches `want_identity` (and the concrete type) exactly — a
    /// different tenant's row is never aliased. Unambiguous by construction
    /// (see [`PinnedFind`] / [`find_pinned_at_exact_scope`]).
    fn find_pinned_in_entries(
        entries: &[RegistryEntry],
        scope: &ScopeLevel,
        want_identity: &SlotIdentity,
        concrete_type: Option<TypeId>,
    ) -> PinnedFind {
        match Self::find_pinned_at_exact_scope(entries, scope, want_identity, concrete_type) {
            PinnedFind::Hit { managed } => PinnedFind::Hit { managed },
            PinnedFind::NotFound if *scope != ScopeLevel::Global => {
                Self::find_pinned_at_exact_scope(
                    entries,
                    &ScopeLevel::Global,
                    want_identity,
                    concrete_type,
                )
            },
            PinnedFind::NotFound => PinnedFind::NotFound,
        }
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for LookupOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LookupOutcome::Found(_) => f.write_str("Found(..)"),
            LookupOutcome::NotFound => f.write_str("NotFound"),
            LookupOutcome::Ambiguous { rows } => {
                write!(f, "Ambiguous {{ rows: {rows} }}")
            },
        }
    }
}

impl std::fmt::Debug for PinnedLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PinnedLookup::Found(_) => f.write_str("Found(..)"),
            PinnedLookup::NotFound => f.write_str("NotFound"),
        }
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::WorkspaceId;

    use super::*;

    struct FakeA;
    struct FakeB;

    // In-crate test doubles: the seal is crate-private, so the test
    // module can satisfy it directly (an out-of-crate type could not —
    // that is the point of the seal).
    impl sealed::Sealed for FakeA {}
    impl sealed::Sealed for FakeB {}

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
                fn set_failed(&self, _reason: &str) {}
                fn phase(&self) -> crate::state::ResourcePhase {
                    crate::state::ResourcePhase::Ready
                }
                fn topology_tag(&self) -> TopologyTag {
                    TopologyTag::Resident
                }
                fn taint(&self) {}
                fn bump_revoke_epoch(&self) {}
                async fn dispatch_on_refresh(&self, _slot: &str) -> Result<(), Error> {
                    Ok(())
                }
                async fn dispatch_on_revoke(&self, _slot: &str) -> Result<(), Error> {
                    Ok(())
                }
                async fn wait_for_in_flight_drain(
                    &self,
                    _timeout: std::time::Duration,
                ) -> Result<(), u64> {
                    Ok(())
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
            Registry::find_pinned_in_entries(
                &entries,
                &workspace,
                &id,
                Some(TypeId::of::<FakeA>())
            ),
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
}
