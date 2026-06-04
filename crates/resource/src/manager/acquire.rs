//! Acquire dispatch surface: registration-time erased hooks, scope/identity
//! lookup + taint helpers, the per-topology dispatch closures, the shared
//! `run_acquire` pipeline, and pool diagnostics/warmup.

use std::{future::Future, sync::Arc, time::Instant};

use nebula_core::{Context, ResourceKey, ScopeLevel};

use super::{InFlightCounter, Manager, gate::admit_through_gate, gate::settle_gate_admission};
use crate::{
    context::ResourceContext,
    error::Error,
    events::ResourceEvent,
    manager::ErasedAcquireFn,
    options::AcquireOptions,
    resource::Resource,
    runtime::{TopologyRuntime, managed::ManagedResource},
};

impl Manager {
    /// Erased acquire hook for a resident row.
    ///
    /// Takes **no** slot-identity argument: the single-walk acquire
    /// resolution pins the row by the *caller's* runtime slot identity, so
    /// the registration-time identity never parameterised the hook. The
    /// structural register path ([`register_resolved`](Self::register_resolved))
    /// hands this hook in by value with no identity threading.
    #[must_use]
    pub fn erased_acquire_resident_for<R>() -> ErasedAcquireFn
    where
        R: crate::topology::resident::Resident + Resource + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        super::acquire_dispatch::erased_acquire_resident::<R>()
    }

    /// Erased acquire hook for a pooled row, structural-identity form.
    ///
    /// See [`erased_acquire_resident_for`](Self::erased_acquire_resident_for)
    /// — no slot-identity argument; the single-walk resolution pins the row.
    #[must_use]
    pub fn erased_acquire_pooled_for<R>() -> ErasedAcquireFn
    where
        R: crate::topology::pooled::Pooled + Clone + Resource + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        super::acquire_dispatch::erased_acquire_pooled::<R>()
    }

    /// Erased acquire hook for a [`Bounded`](crate::topology::bounded::Bounded)
    /// row.
    ///
    /// The registration-time hook for a `TopologyRuntime::Bounded` row. No
    /// slot-identity argument — the single-walk acquire resolution pins
    /// the row by the caller's runtime slot identity, and the release
    /// shape is the resource's [`Cap`](crate::topology::bounded::Bounded::Cap)
    /// typestate (resolved inside the pipeline), not a registration
    /// parameter.
    #[must_use]
    pub fn erased_acquire_bounded_for<R>() -> ErasedAcquireFn
    where
        R: crate::topology::bounded::BoundedRelease + Clone + Resource + Send + Sync + 'static,
        R::Runtime: Clone + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        super::acquire_dispatch::erased_acquire_bounded::<R>()
    }

    /// Typed acquire lookup walking [`scope_levels_for_acquire`](crate::context::scope_levels_for_acquire)
    /// on the context scope bag, then [`taint_gate`](Self::taint_gate).
    pub(crate) fn lookup_for_acquire_scope<R: Resource>(
        &self,
        ctx: &ResourceContext,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        self.shutdown_guard()?;
        let managed =
            Self::resolve_typed::<R>(self.registry.get_typed_for_acquire_scope::<R>(ctx.scope()))?;
        Self::taint_gate::<R>(managed)
    } // visible cross-module after impl split

    /// [`lookup_for_acquire_scope`](Self::lookup_for_acquire_scope) pinned to
    /// the **collision-free structural** resolved per-slot credential
    /// identity. The pinned lookup is 2-variant (no `Ambiguous`).
    fn lookup_for_acquire_with_identity<R: Resource>(
        &self,
        ctx: &ResourceContext,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        self.shutdown_guard()?;
        let managed = Self::resolve_typed_pinned::<R>(
            self.registry
                .get_typed_for_acquire::<R>(ctx.scope(), slot_identity),
        )?;
        Self::taint_gate::<R>(managed)
    }

    /// Downcasts the row already resolved by
    /// [`Registry::get_acquire_for`](crate::registry::Registry::get_acquire_for)'s
    /// single scope walk, then applies the shared shutdown + taint tail.
    ///
    /// The erased-acquire path threads the resolved
    /// `Arc<dyn AnyManagedResource>` out of that one walk (via
    /// [`AcquireLookupOutcome::Found`](crate::registry::AcquireLookupOutcome::Found)),
    /// so the typed handle is recovered by a **downcast of that exact
    /// row** — not a second `DashMap` walk at the matched scope. The
    /// resolved row is, by construction, the `ManagedResource<R>` the
    /// `erased_acquire_*::<R>` hook was registered alongside, so the
    /// downcast yields the identical handle the prior pinned re-walk
    /// would have. Failure mapping (`NotFound` on a type mismatch) and
    /// the [`taint_gate`](Self::taint_gate) tail are byte-identical to
    /// the replaced pinned-lookup path.
    fn downcast_resolved_row<R: Resource>(
        &self,
        managed: Arc<dyn crate::registry::AnyManagedResource>,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        use crate::registry::PinnedLookup;
        self.shutdown_guard()?;
        let managed = Self::resolve_typed_pinned::<R>(PinnedLookup::Found(managed))?;
        Self::taint_gate::<R>(managed)
    }

    /// Shared taint check tail for the acquire-side lookups.
    ///
    /// Every `acquire_*` path funnels through here so a single check
    /// rejects new leases once `revoke_slot` has tainted the resource.
    /// Diagnostic paths (`health_check`, `pool_stats`, `reload_config`) use
    /// the plain `lookup` so they keep working on a tainted resource.
    ///
    /// `warmup_pool` is routed through the acquire funnel (taint-gated) because
    /// it materializes instances via `R::create`.
    ///
    /// Taint rejects with [`ErrorKind::Revoked`](crate::error::ErrorKind::Revoked),
    /// distinct from [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled)
    /// raised by [`Self::shutdown_guard`].
    fn taint_gate<R: Resource>(
        managed: Arc<ManagedResource<R>>,
    ) -> Result<Arc<ManagedResource<R>>, Error> {
        if managed.is_tainted() {
            return Err(Self::tainted_error::<R>());
        }
        Ok(managed)
    }

    /// Post-`InFlightCounter::new` re-check shared by every
    /// `run_*_acquire` / `try_acquire_*` pipeline. Re-observes **both**
    /// revoke taint *and* `graceful_shutdown` once this acquire is reflected
    /// in the in-flight counters the respective drains read.
    ///
    /// Two structurally identical pre-check/post-count-recheck closes funnel
    /// through here:
    ///
    /// - **Revoke (`revoke_slot`).** The acquire-side
    ///   [`taint_gate`](Self::taint_gate) ran before the in-flight counter
    ///   was constructed, leaving a window where a concurrent `revoke_slot`
    ///   could taint *after* the gate but *before* the increment.
    ///   Re-checking taint here — once this acquire is reflected in the
    ///   resource's own in-flight counter (the exact counter `revoke_slot`
    ///   drains) — closes the revoke-vs-acquire TOCTOU.
    /// - **Graceful shutdown (`graceful_shutdown`).** `lookup`'s
    ///   [`shutdown_guard`](Self::shutdown_guard) ran before the in-flight
    ///   counter too, leaving the *symmetric* window: an acquire that
    ///   passed `lookup` while `shutting_down == false` could have its
    ///   `InFlightCounter::new()` increment land *after* `wait_for_drain`
    ///   already observed `0` and `registry.clear()` ran — a logical
    ///   use-after-drain that hands out a [`ResourceGuard`] for a drained
    ///   resource. Re-running `shutdown_guard` here — once this acquire is
    ///   reflected in the manager-wide `drain_tracker`
    ///   [`graceful_shutdown`](Self::graceful_shutdown) drains — closes it
    ///   exactly as the taint re-check closes the revoke path.
    ///
    /// See the [`manager`](crate::manager) module docs for the canonical
    /// invariant. Taint maps to `Revoked` → `ErrorCategory::Unavailable`
    /// (unchanged from the gate); shutdown maps to `Cancelled` (unchanged
    /// from `lookup`'s Defense A), so neither caller-facing category moves.
    fn reject_if_tainted_or_shutting_down_post_count<R: Resource>(
        &self,
        managed: &Arc<ManagedResource<R>>,
    ) -> Result<(), Error> {
        if managed.is_tainted() {
            return Err(Self::tainted_error::<R>());
        }
        // Symmetric with the taint re-check above: the increment is now
        // visible to `wait_for_drain`, so observing `shutting_down`/`cancel`
        // here means either this acquire is rejected, or its increment was
        // seen by the drain and the drain waited for the resulting guard.
        self.shutdown_guard()?;
        Ok(())
    }

    /// The single typed error both taint checks return — keeps the message
    /// and `Revoked` (→ `Unavailable`) classification identical at the
    /// pre-count gate and the post-count re-check.
    fn tainted_error<R: Resource>() -> Error {
        Error::revoked(format!(
            "{}: resource tainted by credential revoke — new acquires rejected",
            R::key()
        ))
        .with_resource_key(R::key())
    }

    /// Acquires a [`crate::guard::ResourceGuard`] through the registry row's
    /// erased dispatch hook, keyed by the **collision-free structural**
    /// resolved-credential identity (key + scope + slot identity).
    ///
    /// This is the object-safe engine/action-accessor acquire entry used
    /// when the concrete resource type `R` is not known at compile time: the
    /// accessor holds the structural
    /// [`SlotIdentity`](crate::dedup::SlotIdentity) recorded for the key at
    /// activation and passes it here, so the single scope walk resolves the
    /// *exact* resolved row (no digest aliasing). The resolved row is
    /// downcast by the hook with no second registry walk.
    ///
    /// # Errors
    ///
    /// Same as the typed `acquire_*_for_identity` family: not found,
    /// ambiguous (when `slot_identity` does not match a row), shutdown,
    /// taint, topology, and acquire-time failures.
    pub async fn acquire_erased_for(
        manager: Arc<Self>,
        key: &ResourceKey,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<Box<dyn std::any::Any + Send + Sync>, Error> {
        use crate::registry::AcquireLookupOutcome;

        manager.shutdown_guard()?;
        tracing::debug!(
            target: "nebula.resource",
            %key,
            ?slot_identity,
            "acquire_erased: resolving registry hook"
        );
        match manager
            .registry
            .get_acquire_for(key, ctx.scope(), slot_identity)
        {
            AcquireLookupOutcome::Found { acquire, managed } => {
                // `managed` is the row this single scope walk already
                // resolved; the hook downcasts it to the concrete
                // `ManagedResource<R>` instead of re-walking the registry
                // at the matched scope.
                acquire(manager, ctx.clone_for_acquire(), options.clone(), managed).await
            },
            AcquireLookupOutcome::NotFound => {
                tracing::debug!(target: "nebula.resource", %key, "acquire_erased: not found");
                Err(Error::not_found(key))
            },
            AcquireLookupOutcome::Ambiguous { rows } => {
                tracing::warn!(
                    target: "nebula.resource",
                    %key,
                    rows,
                    "acquire_erased: ambiguous scope/slot identity"
                );
                Err(Error::ambiguous(format!(
                    "{key}: {rows} resolved-credential registrations exist at this scope; \
                     acquire must target a resolved row via slot identity"
                ))
                .with_resource_key(key.clone()))
            },
        }
    }

    /// Acquires a handle to a pooled resource.
    ///
    /// Performs typed lookup, then dispatches to the pool runtime's acquire.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   pool topology.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) — a
    ///   permanent (non-retryable) caller-conflict deny — if more than one
    ///   resolved-credential registration exists for `(R, scope)`
    ///   (multi-tenant). Acquire through the slot-identity-pinned
    ///   [`acquire_pooled_for_identity`](Self::acquire_pooled_for_identity)
    ///   when the resolved slot identity is known; this identity-agnostic
    ///   path stays fail-closed for the no-identity caller.
    /// - Propagates pool-specific acquire errors.
    pub async fn acquire_pooled<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        self.pooled_pipeline(managed, ctx, options).await
    }

    /// [`acquire_pooled`](Self::acquire_pooled) pinned to the
    /// **collision-free structural** resolved per-slot credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches, so a caller
    /// that resolved tenant A's credential reaches tenant A's runtime and
    /// never tenant B's. This is the unambiguous acquire path the engine
    /// resolution layer uses once it has resolved a node's slot bindings;
    /// it is also how callers reach a resource registered with a non-default
    /// [`RegisterOptions::with_slot_bindings`](crate::RegisterOptions::with_slot_bindings). Equality is exact (no
    /// digest), so a forced digest collision cannot merge two tenants here.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   pool topology.
    /// - Propagates pool-specific acquire errors.
    pub async fn acquire_pooled_for_identity<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup_for_acquire_with_identity::<R>(ctx, slot_identity)?;
        self.pooled_pipeline(managed, ctx, options).await
    }

    /// [`acquire_pooled_for_identity`](Self::acquire_pooled_for_identity) for
    /// a row already resolved by the erased-acquire scope walk (downcast, no
    /// re-walk).
    pub(crate) async fn acquire_pooled_at_scope<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        resolved: Arc<dyn crate::registry::AnyManagedResource>,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.downcast_resolved_row::<R>(resolved)?;
        self.pooled_pipeline(managed, ctx, options).await
    }

    /// Pool topology dispatch into the shared [`run_acquire`](Self::run_acquire)
    /// pipeline. Holds only the one-arm `TopologyRuntime::Pool` match (the
    /// irreducible per-topology surface: the topology traits are siblings,
    /// not a hierarchy, so the shared generic pipeline cannot prove the
    /// variant statically). `config`/`generation` are recomputed inside the
    /// dispatch closure so they are re-read on every resilience retry.
    async fn pooled_pipeline<R>(
        &self,
        managed: Arc<ManagedResource<R>>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        self.run_acquire(Arc::clone(&managed), || {
            let generation = managed.generation();
            let config = managed.config();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Pool(rt) => {
                        rt.acquire(
                            &managed.resource,
                            &config,
                            ctx,
                            &managed.release_queue,
                            generation,
                            options,
                            self.metrics.clone(),
                        )
                        .await
                    },
                    other => Err(Self::unexpected_topology::<R>(other)),
                }
            }
        })
        .await
    }

    /// The single typed error every topology dispatch returns when the
    /// resolved row's [`TopologyRuntime`] variant does not match the
    /// statically-bound acquire path.
    ///
    /// Registration binds the row's topology to its trait (`R: Pooled`
    /// registers `TopologyRuntime::Pool`, etc.), so a mismatch here is a
    /// registration/lookup invariant breach, not a caller error — but the
    /// per-topology dispatch closures are bound to *one* sibling topology
    /// trait each (the traits are siblings, not a hierarchy), so a single
    /// generic pipeline cannot statically prove the variant. This collapses
    /// the five byte-identical `"{key}: expected X topology, registered as
    /// {tag}"` arms into one shared classifier instead of duplicating the
    /// `format!` once per topology dispatcher.
    pub(crate) fn unexpected_topology<R: Resource>(topology: &TopologyRuntime<R>) -> Error {
        Error::permanent(format!(
            "{}: resolved row topology {} does not match the acquired topology",
            R::key(),
            topology.tag()
        ))
    } // visible cross-module after impl split

    /// Single generic acquire pipeline (resilience + gate + drain
    /// bookkeeping) over an already-resolved [`ManagedResource`], replacing
    /// the five byte-identical per-topology acquire wrappers. The only thing
    /// that differed between them was the one-arm topology dispatch, which
    /// each caller now supplies as `dispatch` (recomputed per resilience
    /// retry, exactly as the inline closures did). Every public `acquire_*` /
    /// `acquire_*_for` / `acquire_*_at_scope` entry point differs only in
    /// how it resolves the row (identity-agnostic vs. slot-identity-pinned
    /// vs. scope-pinned) and which topology runtime its closure calls; the
    /// pipeline — including the `InFlightCounter` → post-taint re-check
    /// ordering this method owns — is identical.
    pub(crate) async fn run_acquire<R, F, Fut>(
        &self,
        managed: Arc<ManagedResource<R>>,
        mut dispatch: F,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: Resource,
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<crate::guard::ResourceGuard<R>, Error>> + Send,
    {
        let started = Instant::now();
        // Pre-count this acquire on both the manager-wide and per-resource
        // in-flight trackers, from the moment `lookup()` succeeds. RAII
        // decrements + notifies on every failure / cancel / panic path; on
        // success the slot is handed off to the resulting `ResourceGuard` and
        // held continuously until the guard drops. The `AcqRel` increment here
        // is strictly before the post-taint re-check below. Two-phase-revoke
        // invariant: see the `manager` module documentation.
        let in_flight =
            InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
        // Post-count re-check — now that this acquire is reflected in the
        // per-resource counter `revoke_slot` drains *and* the manager-wide
        // `drain_tracker` `graceful_shutdown` drains, re-observe both revoke
        // taint (closes the revoke-vs-acquire TOCTOU) and `shutting_down`
        // (closes the symmetric shutdown-vs-acquire use-after-drain: an
        // acquire that passed `lookup`'s Defense A before shutdown, whose
        // increment landed after the drain saw `0` + the registry cleared,
        // is rejected here instead of handing out a guard for a drained
        // resource). Same `Revoked`/`Cancelled` classifications as the
        // pre-checks. Rationale: see the `manager` module documentation.
        self.reject_if_tainted_or_shutting_down_post_count::<R>(&managed)?;
        let gate_admission = admit_through_gate(&managed.recovery_gate)?;

        // Publish a `RetryAttempt` event when this acquire is the recovery
        // probe (the CAS-claimed single-probe slot that follows a transient
        // backend failure). `backoff_on_fail` carries the delay the gate
        // would impose *if this probe fails again* — the next caller's wait,
        // not a wait this acquire incurs. Emitted **before** `dispatch()` so
        // observers see the attempt go out rather than only the result. The
        // error field carries the prior failure message snapshotted in
        // `admit_through_gate` before the CAS rotated the gate.
        if let super::gate::GateAdmission::Probe {
            attempt,
            backoff_on_fail,
            last_failure,
            ..
        } = &gate_admission
        {
            self.emit(ResourceEvent::RetryAttempt {
                key: R::key(),
                attempt: *attempt,
                backoff: *backoff_on_fail,
                error: last_failure.clone().unwrap_or_default(),
            });
        }

        let result = dispatch().await;

        // Settle the gate ticket based on the acquire result. #322: this
        // makes the ticket ownership end-to-end — on success we `resolve`,
        // on retryable error we `fail_transient`, on permanent error we
        // `fail_permanent`. The `Drop` impl of `RecoveryTicket` covers
        // cancellation/panic paths.
        settle_gate_admission(gate_admission, &result);
        self.record_acquire_result(&result, started);
        match result {
            // Attach the manager's event bus so the guard's `Drop` emits
            // `ResourceEvent::Released`. Done here, on the success path only,
            // because failed acquires never minted a guard to begin with —
            // there is nothing to release.
            Ok(h) => Ok(h
                .with_drain_tracker(in_flight.release_to_guard())
                .with_event_bus(Arc::clone(&self.event_bus))),
            Err(e) => Err(e),
        }
    } // visible cross-module after impl split

    /// Acquires a handle to a resident resource.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   resident topology.
    /// - Propagates resident-specific acquire errors.
    pub async fn acquire_resident<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        self.resident_pipeline(managed, ctx, options).await
    }

    /// [`acquire_resident`](Self::acquire_resident) pinned to the
    /// **collision-free structural** resolved per-slot credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches, so a caller
    /// that resolved tenant A's credential reaches tenant A's runtime and
    /// never tenant B's. This is the unambiguous acquire path the engine
    /// resolution layer uses once it has resolved a node's slot bindings;
    /// it is also how callers reach a resource registered with a non-default
    /// [`RegisterOptions::with_slot_bindings`](crate::RegisterOptions::with_slot_bindings). Two registrations whose
    /// resolved `(slot, credential)` bindings differ are distinct rows with
    /// distinct runtimes; equality is exact (no digest), so a forced digest
    /// collision cannot merge two tenants here.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   resident topology.
    /// - Propagates resident-specific acquire errors.
    pub async fn acquire_resident_for_identity<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        let managed = self.lookup_for_acquire_with_identity::<R>(ctx, slot_identity)?;
        self.resident_pipeline(managed, ctx, options).await
    }

    /// [`acquire_resident_for_identity`](Self::acquire_resident_for_identity)
    /// for a row already resolved by the erased-acquire scope walk
    /// (downcast, no re-walk).
    pub(crate) async fn acquire_resident_at_scope<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        resolved: Arc<dyn crate::registry::AnyManagedResource>,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        let managed = self.downcast_resolved_row::<R>(resolved)?;
        self.resident_pipeline(managed, ctx, options).await
    }

    /// Resident topology dispatch into the shared
    /// [`run_acquire`](Self::run_acquire) pipeline. Holds only the one-arm
    /// `TopologyRuntime::Resident` match (resident `acquire` takes neither
    /// `release_queue`/`generation` nor `metrics`). `config` is recomputed
    /// inside the dispatch closure so it is re-read on every resilience
    /// retry.
    async fn resident_pipeline<R>(
        &self,
        managed: Arc<ManagedResource<R>>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::resident::Resident + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Clone + Send + 'static,
    {
        self.run_acquire(Arc::clone(&managed), || {
            let config = managed.config();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Resident(rt) => {
                        rt.acquire(&managed.resource, &config, ctx, options).await
                    },
                    other => Err(Self::unexpected_topology::<R>(other)),
                }
            }
        })
        .await
    }

    /// Acquires a handle to a [`Bounded`](crate::topology::bounded::Bounded)
    /// resource.
    ///
    /// The release shape is the resource's [`Cap`](crate::topology::bounded::Bounded::Cap)
    /// typestate — `Unbounded` → owned handle (no release), `Capped<N>` /
    /// `Exclusive` → guarded handle whose drop runs the observed
    /// `release_one` (R17). Identity-agnostic: a multi-tenant `(R, scope)`
    /// fails closed with
    /// [`Ambiguous`](crate::error::ErrorKind::Ambiguous); use
    /// [`acquire_bounded_for_identity`](Self::acquire_bounded_for_identity)
    /// with the resolved structural identity.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   bounded topology.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) if more
    ///   than one resolved-credential registration exists for `(R, scope)`.
    /// - Propagates the cap's acquire errors (permit timeout / closed).
    pub async fn acquire_bounded<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::bounded::BoundedRelease + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        self.bounded_pipeline(managed, ctx, options).await
    }

    /// [`acquire_bounded`](Self::acquire_bounded) keyed by the
    /// **collision-free structural** resolved-credential identity.
    ///
    /// Resolves the registry row whose `slot_identity` matches exactly (no
    /// digest aliasing), so a caller that resolved tenant A's credential
    /// reaches tenant A's runtime and never tenant B's.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   bounded topology.
    /// - Propagates the cap's acquire errors.
    pub async fn acquire_bounded_for_identity<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::bounded::BoundedRelease + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.lookup_for_acquire_with_identity::<R>(ctx, slot_identity)?;
        self.bounded_pipeline(managed, ctx, options).await
    }

    /// [`acquire_bounded`](Self::acquire_bounded) for a row already resolved
    /// by the erased-acquire scope walk (downcast, no re-walk).
    pub(crate) async fn acquire_bounded_at_scope<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        resolved: Arc<dyn crate::registry::AnyManagedResource>,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::bounded::BoundedRelease + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        let managed = self.downcast_resolved_row::<R>(resolved)?;
        self.bounded_pipeline(managed, ctx, options).await
    }

    /// Bounded topology dispatch into the shared
    /// [`run_acquire`](Self::run_acquire) pipeline. One-arm
    /// `TopologyRuntime::Bounded` match (same shape as transport:
    /// `release_queue`/`generation`/`metrics`, no `config`). `generation`
    /// is recomputed inside the dispatch closure so it is re-read on every
    /// resilience retry.
    async fn bounded_pipeline<R>(
        &self,
        managed: Arc<ManagedResource<R>>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::bounded::BoundedRelease + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Send + Sync + 'static,
        R::Lease: Send + 'static,
    {
        self.run_acquire(Arc::clone(&managed), || {
            let generation = managed.generation();
            let managed = Arc::clone(&managed);
            async move {
                match &managed.topology {
                    TopologyRuntime::Bounded(rt) => {
                        rt.acquire(
                            &managed.resource,
                            ctx,
                            &managed.release_queue,
                            generation,
                            options,
                            self.metrics.clone(),
                        )
                        .await
                    },
                    other => Err(Self::unexpected_topology::<R>(other)),
                }
            }
        })
        .await
    }

    /// Returns a snapshot of current pool utilization for a registered Pool resource.
    ///
    /// Returns `None` if the resource is not registered or does not use Pool topology.
    pub async fn pool_stats<R>(&self, scope: &ScopeLevel) -> Option<crate::runtime::pool::PoolStats>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup::<R>(scope).ok()?;
        match &managed.topology {
            TopologyRuntime::Pool(rt) => Some(rt.stats().await),
            _ => None,
        }
    }

    /// Pre-warms a registered Pool resource.
    ///
    /// Per slot model, the resource's `#[credential]` slot fields are
    /// already populated on the resource value — `Pool::warmup` calls
    /// `R::create(config, ctx)` directly, no scheme parameter required.
    ///
    /// This fills the idle queue before production traffic hits, eliminating
    /// cold-start latency on the first batch of requests. Warmup follows the
    /// [`WarmupStrategy`](crate::topology::pooled::config::WarmupStrategy) set
    /// in the pool's configuration.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   pool topology.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) — a
    ///   permanent (non-retryable) caller-conflict deny — if more than one
    ///   resolved-credential registration exists for `(R, scope)`
    ///   (multi-tenant). Warmup is identity-agnostic and stays fail-closed;
    ///   a multi-tenant pool is warmed per resolved row through the
    ///   slot-identity-pinned acquire path
    ///   ([`acquire_pooled_for_identity`](Self::acquire_pooled_for_identity)).
    pub async fn warmup_pool<R>(&self, ctx: &ResourceContext) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        let config = managed.config();
        match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                // `warmup` runs `R::create` against the resolved credential
                // to materialize fresh pool instances — it is acquire-like
                // and must observe the SAME post-count re-check the
                // `run_*_acquire` pipelines use (#679 / slot + isolation model).
                // `lookup_for_acquire`'s taint gate *and* `shutdown_guard`
                // both ran *before* this in-flight increment, leaving the
                // two symmetric windows: a concurrent `revoke_slot` could
                // taint, or `graceful_shutdown` could drain-see-`0` +
                // clear the registry, after the gate yet before warmup
                // creates entries. Pre-count this work in both the
                // resource's own in-flight counter (the exact counter
                // `revoke_slot` drains) and the manager-wide `drain_tracker`
                // (`graceful_shutdown`), then re-check both: either we
                // observe taint / `shutting_down` here and reject, or our
                // increment is visible to the respective drain — so no
                // fresh pool entry is ever created on a just-revoked
                // credential or after a completed shutdown drain. The
                // counter is held for the whole `warmup` await (RAII drop
                // on every exit path).
                let _in_flight =
                    InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
                self.reject_if_tainted_or_shutting_down_post_count::<R>(&managed)?;
                let count = rt.warmup(&managed.resource, &config, ctx).await;
                Ok(count)
            },
            _ => Err(Error::permanent(format!(
                "{}: warmup_pool requires Pool topology, registered as {}",
                R::key(),
                managed.topology.tag()
            ))),
        }
    }
}
