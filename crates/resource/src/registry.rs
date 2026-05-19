//! Type-erased registry for managed resources.
//!
//! [`Registry`] stores managed resources indexed by [`ResourceKey`] and
//! [`TypeId`], supporting scope-aware lookup and typed downcasting.

use std::{
    any::{Any, TypeId},
    future::Future,
    pin::Pin,
    sync::Arc,
};

use dashmap::DashMap;
use nebula_core::{ResourceKey, Scope, ScopeLevel};

use crate::{
    context::{ResourceContext, scope_levels_for_acquire},
    dedup::SlotIdentity,
    error::Error,
    options::AcquireOptions,
    resource::Resource,
    runtime::managed::ManagedResource,
    topology_tag::TopologyTag,
};

/// Erased acquire hook installed on each registry row at registration.
///
/// The `Arc<dyn AnyManagedResource>` is the row already resolved by the
/// single `Registry::get_acquire_for` scope walk
/// (carried out via `AcquireLookupOutcome::Found`). The hook downcasts
/// it to the concrete `ManagedResource<R>` rather than performing a
/// second `DashMap` walk at the matched scope — one registry resolution
/// per erased acquire, not two.
pub type ErasedAcquireFn = Arc<
    dyn Fn(
            Arc<crate::Manager>,
            ResourceContext,
            AcquireOptions,
            Arc<dyn AnyManagedResource>,
        )
            -> Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send + Sync>, Error>> + Send>>
        + Send
        + Sync,
>;

/// Crate-private seal: only `nebula-resource` can name `sealed::Sealed`,
/// so [`AnyManagedResource`] is **not implementable downstream**.
///
/// `AnyManagedResource` is engine-internal: the only purpose is to let
/// the [`Registry`] store heterogeneous `ManagedResource<R>` behind one
/// `dyn AnyManagedResource`, and the sole implementor is the blanket
/// `impl<R: Resource>` below. Sealing makes that a *structural*
/// guarantee rather than a convention — adding a required method (e.g.
/// the per-resource-drain hook) can never be a downstream
/// compile-break, because no downstream impl can exist. The
/// `LookupOutcome::Found(Arc<dyn AnyManagedResource>)` surface stays
/// usable (callers only *consume* the trait object); they just cannot
/// *implement* it.
mod sealed {
    /// Sealed marker. Implemented only by the crate-internal blanket
    /// `impl<R: Resource>` for `ManagedResource<R>`.
    pub trait Sealed {}
}

/// Type-erased trait for managed resources stored in the [`Registry`].
///
/// Every `ManagedResource<R>` implements this trait, allowing the registry
/// to store heterogeneous resource types behind a single `dyn AnyManagedResource`.
///
/// **Sealed (engine-internal).** This trait has a private `sealed::Sealed`
/// supertrait, so it can only be implemented inside `nebula-resource` (by
/// the blanket `impl<R: Resource>`). It is an engine-internal erasure
/// boundary, **not** a downstream extension point — new required methods
/// may be added without it being a semver-breaking change for consumers
/// (they only ever hold `Arc<dyn AnyManagedResource>` via
/// [`LookupOutcome::Found`], never implement it).
pub trait AnyManagedResource: sealed::Sealed + Send + Sync + 'static {
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

    /// Type-erased lifecycle phase mutation (#387).
    ///
    /// Lets the manager drive phase transitions on all registered
    /// resources without needing a typed handle, which matters during
    /// graceful shutdown where only the type-erased registry iteration
    /// is available.
    fn set_phase_erased(&self, phase: crate::state::ResourcePhase);

    /// Type-erased terminal failure transition (R-023).
    ///
    /// Transitions the resource to [`ResourcePhase::Failed`] and records
    /// the supplied human-readable reason in `last_error`. Used by
    /// `Manager::set_phase_all_failed` so `DrainTimeoutPolicy::Abort` can
    /// signal per-resource failure without needing typed access to each
    /// entry.
    ///
    /// [`ResourcePhase::Failed`]: crate::state::ResourcePhase::Failed
    fn set_failed_erased(&self, reason: &str);

    /// Type-erased read of the current lifecycle phase.
    ///
    /// Symmetric to [`Self::set_phase_erased`] / [`Self::set_failed_erased`].
    /// Diagnostic-only — typed callers should prefer
    /// `ManagedResource::status().phase` after a successful downcast.
    fn phase_erased(&self) -> crate::state::ResourcePhase;

    /// Type-erased topology tag — used by `Manager::{refresh,revoke}_slot`
    /// to label the rotation tracing span without a typed downcast.
    fn topology_tag_erased(&self) -> TopologyTag;

    /// Type-erased resource-level taint (credential revoke).
    ///
    /// `Manager::revoke_slot` takes a `ResourceKey`, not a generic `R`, so
    /// it must taint through the erased registry view. Symmetric to the
    /// other `*_erased` hooks: it forwards to `ManagedResource::taint`,
    /// which sets the same flag the typed `acquire_*` funnel checks.
    fn taint_erased(&self);

    /// Type-erased credential-revoke epoch bump.
    ///
    /// `Manager::revoke_slot` takes a `ResourceKey`, not a generic `R`, so
    /// it bumps the counter through the erased registry view, symmetric to
    /// [`Self::taint_erased`] and applied in the same synchronous
    /// pre-`.await` step. Forwards to `ManagedResource::bump_revoke_epoch`,
    /// which advances the pooled topology's revoke counter so every pool
    /// return-to-idle path fences an instance authenticated with the
    /// revoked credential (a no-op for single-runtime topologies, which
    /// have no idle queue).
    fn bump_revoke_epoch_erased(&self);

    /// Type-erased per-slot refresh dispatch.
    ///
    /// Boxed future because `dyn AnyManagedResource` cannot carry an
    /// RPITIT method. Forwards to `ManagedResource::dispatch_slot_hook`
    /// with `refresh = true`, which borrows the live runtime per topology
    /// and invokes the resource's `&self` hook.
    fn dispatch_on_refresh_erased<'a>(
        &'a self,
        slot: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;

    /// Type-erased per-slot revoke dispatch (boxed for the same reason as
    /// [`Self::dispatch_on_refresh_erased`]; forwards to
    /// `ManagedResource::dispatch_slot_hook` with `refresh = false`).
    fn dispatch_on_revoke_erased<'a>(
        &'a self,
        slot: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;

    /// Type-erased bounded drain of **this resource's own** in-flight
    /// acquires.
    ///
    /// `Manager::revoke_slot` takes a `ResourceKey`, not a generic `R`, so
    /// it drains through the erased view. Forwards to
    /// `ManagedResource::wait_for_in_flight_drain`, which waits on this
    /// row's per-resource counter — *not* the manager-wide `drain_tracker`
    /// — so a revoke is isolated from in-flight traffic to unrelated
    /// resources (per-resource revoke deferral). Boxed for the same `dyn`-safety
    /// reason as the dispatch hooks. `Err(outstanding)` carries the counter
    /// snapshot at the moment the timer fired.
    fn wait_for_in_flight_drain_erased<'a>(
        &'a self,
        timeout: std::time::Duration,
    ) -> Pin<Box<dyn Future<Output = Result<(), u64>> + Send + 'a>>;
}

// The one and only `Sealed` impl: every `ManagedResource<R>` (and
// nothing else, anywhere) — this is what makes `AnyManagedResource`
// non-implementable downstream.
impl<R: Resource> sealed::Sealed for ManagedResource<R> {}

impl<R: Resource> AnyManagedResource for ManagedResource<R> {
    fn resource_key(&self) -> ResourceKey {
        R::key()
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }

    fn managed_type_id(&self) -> TypeId {
        TypeId::of::<ManagedResource<R>>()
    }

    fn set_phase_erased(&self, phase: crate::state::ResourcePhase) {
        self.set_phase(phase);
    }

    fn set_failed_erased(&self, reason: &str) {
        self.set_failed(reason.to_owned());
    }

    fn phase_erased(&self) -> crate::state::ResourcePhase {
        self.status().phase
    }

    fn topology_tag_erased(&self) -> TopologyTag {
        self.topology.tag()
    }

    fn taint_erased(&self) {
        self.taint();
    }

    fn bump_revoke_epoch_erased(&self) {
        self.bump_revoke_epoch();
    }

    fn dispatch_on_refresh_erased<'a>(
        &'a self,
        slot: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(self.dispatch_slot_hook(slot, true))
    }

    fn dispatch_on_revoke_erased<'a>(
        &'a self,
        slot: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(self.dispatch_slot_hook(slot, false))
    }

    fn wait_for_in_flight_drain_erased<'a>(
        &'a self,
        timeout: std::time::Duration,
    ) -> Pin<Box<dyn Future<Output = Result<(), u64>> + Send + 'a>> {
        Box::pin(self.wait_for_in_flight_drain(timeout))
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
/// [`get_typed_for`](Registry::get_typed_for) /
/// `get_typed_for_acquire`)
/// return [`PinnedLookup`] instead — a 2-variant type with **no
/// `Ambiguous`** arm, because a resolved [`SlotIdentity`] addresses exactly
/// one row by construction, so ambiguity is unrepresentable there rather
/// than a runtime branch a caller could mis-handle.
pub enum LookupOutcome {
    /// Exactly one row matched — here it is.
    Found(Arc<dyn AnyManagedResource>),
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
    Found(Arc<dyn AnyManagedResource>),
    /// No row matched the key/scope/identity. Never an alias to another
    /// resolved credential's row.
    NotFound,
}

/// Outcome of a registry acquire-hook lookup (same semantics as [`LookupOutcome`]).
pub(crate) enum AcquireLookupOutcome {
    /// Exactly one row matched — hook plus the already-resolved row.
    Found {
        /// Erased topology dispatch for this row.
        acquire: ErasedAcquireFn,
        /// The managed-resource row resolved by this single scope walk.
        ///
        /// `acquire` and `managed` are read from the **same**
        /// [`RegistryEntry`] in one walk, so the erased-acquire path
        /// downcasts this `Arc` directly instead of re-walking the
        /// `DashMap` at the matched scope: one resolution, not two. The
        /// downcast (in `Manager`) resolves the identical row the second
        /// walk would have — same `(scope, slot_identity)` row from the
        /// same `entries` Vec.
        managed: Arc<dyn AnyManagedResource>,
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
    managed: Arc<dyn AnyManagedResource>,
    acquire: ErasedAcquireFn,
}

enum ScopeFind {
    Hit {
        managed: Arc<dyn AnyManagedResource>,
        acquire: ErasedAcquireFn,
    },
    NotFound,
    Ambiguous {
        rows: usize,
    },
}

/// Result of a **pinned** scope lookup.
///
/// Deliberately 2-variant: a resolved [`SlotIdentity`] pins exactly one
/// `(scope, slot_identity)` row, so "more than one matched" is not a
/// reachable state on the pinned path — there is no `Ambiguous` variant to
/// forget to fail closed on. This is the type-level half of the
/// [`PinnedLookup`] guarantee.
enum PinnedFind {
    Hit {
        managed: Arc<dyn AnyManagedResource>,
    },
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
        managed: Arc<dyn AnyManagedResource>,
        acquire: ErasedAcquireFn,
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
                    acquire,
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
                    acquire,
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
    pub fn get(&self, key: &ResourceKey, scope: &ScopeLevel) -> LookupOutcome {
        let Some(entries) = self.entries.get(key) else {
            return LookupOutcome::NotFound;
        };
        match Self::find_in_entries(&entries, scope, None) {
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
            match Self::find_at_exact_scope(&entries, &level, Some(slot_identity)) {
                ScopeFind::Hit { managed, acquire } => {
                    // `managed` and `acquire` came from the same
                    // `RegistryEntry` in this one walk. Carry the row out
                    // so the erased-acquire path downcasts it directly
                    // instead of re-walking the `DashMap` at `level`.
                    return AcquireLookupOutcome::Found { acquire, managed };
                },
                ScopeFind::Ambiguous { rows } => {
                    return AcquireLookupOutcome::Ambiguous { rows };
                },
                ScopeFind::NotFound => {
                    if slot_identity.is_unbound()
                        && Self::scope_has_cred_bound_rows_without_unbound(&entries, &level)
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
    pub(crate) fn get_typed_for_acquire<R: Resource>(
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
            match Self::find_pinned_at_exact_scope(&entries, &level, slot_identity) {
                PinnedFind::Hit { managed, .. } => return PinnedLookup::Found(managed),
                PinnedFind::NotFound => {
                    if slot_identity.is_unbound()
                        && Self::scope_has_cred_bound_rows_without_unbound(&entries, &level)
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
    pub(crate) fn get_typed_for_acquire_scope<R: Resource>(&self, scope: &Scope) -> LookupOutcome {
        let type_id = TypeId::of::<ManagedResource<R>>();
        let Some(key) = self.type_index.get(&type_id) else {
            return LookupOutcome::NotFound;
        };
        let Some(entries) = self.entries.get(&*key) else {
            return LookupOutcome::NotFound;
        };
        for level in scope_levels_for_acquire(scope) {
            match Self::find_at_exact_scope(&entries, &level, None) {
                ScopeFind::Hit { managed, .. } => return LookupOutcome::Found(managed),
                ScopeFind::Ambiguous { rows } => return LookupOutcome::Ambiguous { rows },
                ScopeFind::NotFound => continue,
            }
        }
        LookupOutcome::NotFound
    }

    /// At `level`, cred-bound rows exist but the caller asked for the
    /// [`SlotIdentity::Unbound`] (no-resolved-slots) row.
    fn scope_has_cred_bound_rows_without_unbound(
        entries: &[RegistryEntry],
        level: &ScopeLevel,
    ) -> bool {
        entries
            .iter()
            .any(|e| e.scope == *level && !e.slot_identity.is_unbound())
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
        let Some(entries) = self.entries.get(key) else {
            return PinnedLookup::NotFound;
        };
        match Self::find_pinned_in_entries(&entries, scope, slot_identity) {
            PinnedFind::Hit { managed, .. } => PinnedLookup::Found(managed),
            PinnedFind::NotFound => PinnedLookup::NotFound,
        }
    }

    /// Typed lookup: finds the resource for type `R` and downcasts to
    /// `Arc<ManagedResource<R>>` (slot-identity agnostic).
    ///
    /// Inherits [`get`](Self::get)'s fail-closed-on-ambiguity contract.
    pub fn get_typed<R: Resource>(&self, scope: &ScopeLevel) -> LookupOutcome {
        let type_id = TypeId::of::<ManagedResource<R>>();
        let Some(key) = self.type_index.get(&type_id) else {
            return LookupOutcome::NotFound;
        };
        self.get(&key, scope)
    }

    /// Typed lookup pinned to a resolved slot identity.
    ///
    /// Returns [`PinnedLookup`] — a pinned identity addresses exactly one
    /// row, so ambiguity is unrepresentable.
    pub fn get_typed_for<R: Resource>(
        &self,
        scope: &ScopeLevel,
        slot_identity: &SlotIdentity,
    ) -> PinnedLookup {
        let type_id = TypeId::of::<ManagedResource<R>>();
        let Some(key) = self.type_index.get(&type_id) else {
            return PinnedLookup::NotFound;
        };
        self.get_for(&key, scope, slot_identity)
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
    pub(crate) fn all_managed(&self) -> Vec<Arc<dyn AnyManagedResource>> {
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
    /// This drops every `Arc<dyn AnyManagedResource>`, releasing their
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
    fn find_at_exact_scope(
        entries: &[RegistryEntry],
        scope: &ScopeLevel,
        want_identity: Option<&SlotIdentity>,
    ) -> ScopeFind {
        let mut at_scope = entries.iter().filter(|e| e.scope == *scope);

        if let Some(id) = want_identity {
            return match at_scope.find(|e| &e.slot_identity == id) {
                Some(entry) => ScopeFind::Hit {
                    managed: Arc::clone(&entry.managed),
                    acquire: Arc::clone(&entry.acquire),
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
                acquire: Arc::clone(&first.acquire),
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
    fn find_in_entries(
        entries: &[RegistryEntry],
        scope: &ScopeLevel,
        want_identity: Option<&SlotIdentity>,
    ) -> ScopeFind {
        // Resolve the effective scope: exact match wins; otherwise fall
        // back to Global. Scope precedence is decided BEFORE slot identity.
        let effective_scope = if entries.iter().any(|e| e.scope == *scope) {
            scope.clone()
        } else if *scope != ScopeLevel::Global
            && entries.iter().any(|e| e.scope == ScopeLevel::Global)
        {
            ScopeLevel::Global
        } else {
            return ScopeFind::NotFound;
        };

        Self::find_at_exact_scope(entries, &effective_scope, want_identity)
    }

    /// Pinned lookup at an exact [`ScopeLevel`] (no ancestor/Global
    /// fallback).
    ///
    /// Returns the single row at that scope whose `slot_identity` equals
    /// the resolved `want_identity`. The result type ([`PinnedFind`]) has
    /// **no `Ambiguous`** variant: a resolved [`SlotIdentity`] addresses
    /// exactly one `(scope, slot_identity)` row by construction, so
    /// ambiguity is unrepresentable on the pinned path rather than a
    /// runtime branch a caller could mishandle (the cross-tenant-bleed
    /// failure mode the agnostic path guards against cannot occur here).
    fn find_pinned_at_exact_scope(
        entries: &[RegistryEntry],
        scope: &ScopeLevel,
        want_identity: &SlotIdentity,
    ) -> PinnedFind {
        match entries
            .iter()
            .find(|e| e.scope == *scope && &e.slot_identity == want_identity)
        {
            Some(entry) => PinnedFind::Hit {
                managed: Arc::clone(&entry.managed),
            },
            None => PinnedFind::NotFound,
        }
    }

    /// Scope-aware **pinned** lookup within a list of entries.
    ///
    /// Resolves the effective scope first (exact match, else
    /// [`ScopeLevel::Global`] fallback) so scope precedence is decided
    /// before slot identity, then returns the row at that scope whose
    /// `slot_identity` equals `want_identity`. Unambiguous by construction
    /// (see [`PinnedFind`] / [`find_pinned_at_exact_scope`]).
    fn find_pinned_in_entries(
        entries: &[RegistryEntry],
        scope: &ScopeLevel,
        want_identity: &SlotIdentity,
    ) -> PinnedFind {
        let effective_scope = if entries.iter().any(|e| e.scope == *scope) {
            scope.clone()
        } else if *scope != ScopeLevel::Global
            && entries.iter().any(|e| e.scope == ScopeLevel::Global)
        {
            ScopeLevel::Global
        } else {
            return PinnedFind::NotFound;
        };

        Self::find_pinned_at_exact_scope(entries, &effective_scope, want_identity)
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

    fn test_acquire() -> ErasedAcquireFn {
        crate::manager::acquire_dispatch::noop_erased_acquire()
    }

    struct FakeA;
    struct FakeB;

    // In-crate test doubles: the seal is crate-private, so the test
    // module can satisfy it directly (an out-of-crate type could not —
    // that is the point of the seal).
    impl sealed::Sealed for FakeA {}
    impl sealed::Sealed for FakeB {}

    fn unit_dispatch<'a>() -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    impl AnyManagedResource for FakeA {
        fn resource_key(&self) -> ResourceKey {
            ResourceKey::new("fake").unwrap()
        }
        fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
            self
        }
        fn managed_type_id(&self) -> TypeId {
            TypeId::of::<FakeA>()
        }
        fn set_phase_erased(&self, _phase: crate::state::ResourcePhase) {}
        fn set_failed_erased(&self, _reason: &str) {}
        fn phase_erased(&self) -> crate::state::ResourcePhase {
            crate::state::ResourcePhase::Ready
        }
        fn topology_tag_erased(&self) -> TopologyTag {
            TopologyTag::Resident
        }
        fn taint_erased(&self) {}
        fn bump_revoke_epoch_erased(&self) {}
        fn dispatch_on_refresh_erased<'a>(
            &'a self,
            _slot: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
            unit_dispatch()
        }
        fn dispatch_on_revoke_erased<'a>(
            &'a self,
            _slot: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
            unit_dispatch()
        }
        fn wait_for_in_flight_drain_erased<'a>(
            &'a self,
            _timeout: std::time::Duration,
        ) -> Pin<Box<dyn Future<Output = Result<(), u64>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    impl AnyManagedResource for FakeB {
        fn resource_key(&self) -> ResourceKey {
            ResourceKey::new("fake").unwrap()
        }
        fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
            self
        }
        fn managed_type_id(&self) -> TypeId {
            TypeId::of::<FakeB>()
        }
        fn set_phase_erased(&self, _phase: crate::state::ResourcePhase) {}
        fn set_failed_erased(&self, _reason: &str) {}
        fn phase_erased(&self) -> crate::state::ResourcePhase {
            crate::state::ResourcePhase::Ready
        }
        fn topology_tag_erased(&self) -> TopologyTag {
            TopologyTag::Resident
        }
        fn taint_erased(&self) {}
        fn bump_revoke_epoch_erased(&self) {}
        fn dispatch_on_refresh_erased<'a>(
            &'a self,
            _slot: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
            unit_dispatch()
        }
        fn dispatch_on_revoke_erased<'a>(
            &'a self,
            _slot: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>> {
            unit_dispatch()
        }
        fn wait_for_in_flight_drain_erased<'a>(
            &'a self,
            _timeout: std::time::Duration,
        ) -> Pin<Box<dyn Future<Output = Result<(), u64>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    fn ident(slot: &str, cred: &str) -> SlotIdentity {
        SlotIdentity::from_bindings([(slot, cred)])
    }

    #[test]
    fn register_replace_preserves_type_id_still_used_by_another_scope() {
        // Regression for a correctness hole raised in PR #399 review:
        // if scope A and scope B both hold `TypeA`, replacing scope A
        // with `TypeB` must NOT scrub `TypeA -> key` from `type_index`,
        // otherwise `get_typed::<TypeA>(B)` would break.
        let reg = Registry::new();
        let key = ResourceKey::new("fake").unwrap();

        reg.register(
            key.clone(),
            TypeId::of::<FakeA>(),
            ScopeLevel::Global,
            SlotIdentity::Unbound,
            Arc::new(FakeA),
            test_acquire(),
        );
        reg.register(
            key.clone(),
            TypeId::of::<FakeA>(),
            ScopeLevel::Workspace(WorkspaceId::new()),
            SlotIdentity::Unbound,
            Arc::new(FakeA),
            test_acquire(),
        );

        // Replace only the Global entry with FakeB. Workflow still
        // holds FakeA, so the TypeA row in type_index must survive.
        reg.register(
            key,
            TypeId::of::<FakeB>(),
            ScopeLevel::Global,
            SlotIdentity::Unbound,
            Arc::new(FakeB),
            test_acquire(),
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
            test_acquire(),
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
            test_acquire(),
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
            test_acquire(),
        );
        reg.register(
            key.clone(),
            TypeId::of::<FakeA>(),
            scope.clone(),
            id_b.clone(),
            Arc::new(FakeA),
            test_acquire(),
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
        // `Ambiguous` variant exists to mishandle. (The generic
        // `get_typed_for::<R>` shares this pinned resolution; the
        // `dedup_slot_identity` integration test covers it on a real
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
            test_acquire(),
        );
        reg.register(
            key.clone(),
            TypeId::of::<FakeA>(),
            scope.clone(),
            id_b,
            Arc::new(FakeA),
            test_acquire(),
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
            test_acquire(),
        );
        reg.register(
            key.clone(),
            TypeId::of::<FakeA>(),
            scope.clone(),
            ident("db", "cred-b"),
            Arc::new(FakeA),
            test_acquire(),
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
            test_acquire(),
        );

        assert!(matches!(reg.get(&key, &scope), LookupOutcome::Found(_)));
    }

    #[test]
    fn structurally_distinct_bindings_never_collide_even_if_digests_would() {
        // The R15 guarantee at the registry level: two registrations with
        // structurally distinct bindings occupy distinct rows regardless
        // of what any hash of those bindings is. We additionally pin that
        // the structural identity is in a disjoint space from the legacy
        // `Opaque(u64)` bridge, so a forced u64 collision (the U1
        // adversarial input) cannot merge two structural rows.
        let reg = Registry::new();
        let key = ResourceKey::new("fake").unwrap();
        let scope = ScopeLevel::Global;

        let id_a = ident("db", "tenant-a-cred");
        let id_b = ident("db", "tenant-b-cred");
        assert_ne!(id_a, id_b, "distinct bindings are exact-unequal");

        // Even an Opaque value built from the (collidable) digest of a's
        // bindings is a different identity space than a's structural row.
        #[allow(deprecated)]
        let digest_a = crate::dedup::slot_identity([("db", "tenant-a-cred")]);
        assert_ne!(id_a, SlotIdentity::Opaque(digest_a));

        reg.register(
            key.clone(),
            TypeId::of::<FakeA>(),
            scope.clone(),
            id_a.clone(),
            Arc::new(FakeA),
            test_acquire(),
        );
        reg.register(
            key.clone(),
            TypeId::of::<FakeB>(),
            scope.clone(),
            id_b.clone(),
            Arc::new(FakeB),
            test_acquire(),
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
        // The Opaque digest of a's bindings must NOT resolve a's
        // structural row (disjoint spaces — no cross-bleed via a digest).
        assert!(matches!(
            reg.get_for(&key, &scope, &SlotIdentity::Opaque(digest_a)),
            PinnedLookup::NotFound
        ));
    }
}
