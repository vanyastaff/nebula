//! Acquire dispatch surface: registration-time erased hooks, scope/identity
//! lookup + taint helpers, the per-topology dispatch closures, the shared
//! `run_acquire` pipeline, and pool diagnostics/warmup.

use std::{any::Any, future::Future, sync::Arc, time::Instant};

use nebula_core::{Context, ResourceKey, ScopeLevel};

use super::{InFlightCounter, Manager};
use crate::{
    context::ResourceContext,
    error::Error,
    hook_guard::{DEFAULT_AUTHOR_HOOK_CEILING, HookFault},
    options::AcquireOptions,
    resource::Provider,
    runtime::managed::ManagedResource,
    topology::{BoundedProvider, PoolProvider, ResidentProvider, Topology},
};

impl Manager {
    /// Typed acquire lookup walking [`scope_levels_for_acquire`](crate::context::scope_levels_for_acquire)
    /// on the context scope bag, then [`taint_gate`](Self::taint_gate).
    pub(crate) fn lookup_for_acquire_scope<R: Provider>(
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
    fn lookup_for_acquire_with_identity<R: Provider>(
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
    pub(crate) fn taint_gate<R: Provider>(
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
    ///   use-after-drain that hands out a [`ResourceGuard`](crate::ResourceGuard) for a drained
    ///   resource. Re-running `shutdown_guard` here — once this acquire is
    ///   reflected in the manager-wide `drain_tracker`
    ///   [`graceful_shutdown`](Self::graceful_shutdown) drains — closes it
    ///   exactly as the taint re-check closes the revoke path.
    ///
    /// See the [`manager`](crate::manager) module docs for the canonical
    /// invariant. Taint maps to `Revoked` → `ErrorCategory::Unavailable`
    /// (unchanged from the gate); shutdown maps to `Cancelled` (unchanged
    /// from `lookup`'s Defense A), so neither caller-facing category moves.
    pub(super) fn reject_if_tainted_or_shutting_down_post_count<R: Provider>(
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
    pub(super) fn tainted_error<R: Provider>() -> Error {
        Error::revoked(format!(
            "{}: resource tainted by credential revoke — new acquires rejected",
            R::key()
        ))
        .with_resource_key(R::key())
    }

    /// The error for an acquire whose row admits nothing in its current
    /// admission generation: `Revoked` when a credential revoke closed it,
    /// `CredentialUnavailable` when a credential suspension closed the
    /// `captured` generation, otherwise `Cancelled` (the row was removed or
    /// the manager is closing). None trips the recovery gate.
    pub(super) fn closed_admission_error<R: Provider>(
        managed: &ManagedResource<R>,
        captured: Option<&crate::runtime::admission::AdmissionGeneration>,
    ) -> Error {
        if managed.is_tainted() {
            return Self::tainted_error::<R>();
        }
        match captured.and_then(crate::runtime::admission::AdmissionGeneration::close_cause) {
            Some(crate::runtime::admission::CloseCause::Credential(reason)) => {
                Self::credential_unavailable_error(&R::key(), reason)
            },
            None => Error::cancelled().with_resource_key(R::key()),
        }
    }

    /// The refusal of work on row `key` while a bound credential suspends it.
    pub(crate) fn credential_unavailable_error(
        key: &ResourceKey,
        reason: crate::error::CredentialUnavailableReason,
    ) -> Error {
        Error::new(
            crate::error::ErrorKind::CredentialUnavailable { reason },
            format!("{key}: bound credential unavailable ({reason}) — new work refused"),
        )
        .with_resource_key(key.clone())
    }

    /// Acquires through the registry row's `ManagedHandle::acquire` method,
    /// keyed by the **collision-free structural** resolved-credential identity
    /// (key + scope + slot identity).
    ///
    /// This is the object-safe engine/action-accessor acquire entry used when
    /// the concrete resource type `R` is not known at compile time. The single
    /// scope walk resolves the exact row; `ManagedHandle::acquire` dispatches
    /// on topology internally with no second registry walk.
    ///
    /// # Errors
    ///
    /// Same as the typed `acquire_*_for_identity` family: not found,
    /// ambiguous, shutdown, taint, topology, and acquire-time failures.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. Dropping the future at any await point
    /// releases the topology permit, settles the drain accounting, and
    /// auto-fails a held recovery-gate probe ticket (its backoff applies).
    /// An instance in flight between checkout/create and the returned guard
    /// is destroyed asynchronously via the release queue, never leaked. The
    /// only effect of cancellation is that no guard is returned.
    pub async fn acquire_any(
        manager: Arc<Self>,
        key: &ResourceKey,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<Box<dyn Any + Send + Sync>, Error> {
        use crate::registry::AcquireLookupOutcome;

        manager.shutdown_guard()?;
        tracing::debug!(
            target: "nebula.resource",
            %key,
            ?slot_identity,
            "acquire_any: resolving registry row"
        );
        match manager
            .registry
            .get_acquire_for(key, ctx.scope(), slot_identity)
        {
            AcquireLookupOutcome::Found { managed } => {
                // Sync capacity gate: rejects before the async acquire when the
                // topology is saturated, warming, recovering, or tainted.
                // Mapped to the typed `Error` kind the engine uses for
                // park/reschedule decisions (`Backpressure`, `Transient`,
                // `Revoked`). The taint-gate and shutdown-guard run first
                // (above); this gate is the topology-level admission check.
                if let Err(unavailable) = managed.try_reserve_gate() {
                    tracing::debug!(
                        target: "nebula.resource",
                        %key,
                        ?slot_identity,
                        reason = ?unavailable,
                        "acquire_any: topology admission rejected"
                    );
                    return Err(unavailable.into_error(key));
                }
                managed
                    .acquire(
                        Arc::clone(&manager),
                        ctx.clone_for_acquire(),
                        options.clone(),
                    )
                    .await
            },
            AcquireLookupOutcome::NotFound => {
                tracing::debug!(target: "nebula.resource", %key, "acquire_any: not found");
                Err(Error::not_found(key))
            },
            AcquireLookupOutcome::Ambiguous { rows } => {
                tracing::warn!(
                    target: "nebula.resource",
                    %key,
                    rows,
                    "acquire_any: ambiguous scope/slot identity"
                );
                Err(Self::ambiguous_row_error(key, rows))
            },
        }
    }

    /// Acquires a lease on resource `R`, whatever its topology.
    ///
    /// `R::Topology` fixes the topology at compile time, so the caller does
    /// not name it: this works the same for [`Pooled`](crate::Pooled),
    /// [`Resident`](crate::Resident), [`Bounded`](crate::Bounded) and custom
    /// [`Topology`] implementations, and switching a resource's topology does
    /// not break its callers. The `acquire_{pooled,resident,bounded}` methods
    /// are equivalent spellings that additionally assert the topology.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered for the context scope.
    /// - [`ErrorKind::Cancelled`](crate::error::ErrorKind::Cancelled) if the manager is shutting
    ///   down.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) if more than one
    ///   resolved-credential registration exists for `(R, scope)`; use
    ///   [`acquire_for_identity`](Self::acquire_for_identity) then.
    /// - Propagates topology-specific acquire errors.
    ///
    /// # Cancel safety
    ///
    /// Cancel safe: dropping the future releases the topology permit, settles
    /// the drain accounting and auto-fails a held recovery-gate probe; an
    /// instance in flight is destroyed asynchronously via the release queue.
    pub async fn acquire<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: Provider,
        R::Topology: Topology<R>,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        self.run_acquire_dispatch(managed, ctx, options).await
    }

    /// [`acquire`](Self::acquire) pinned to the **collision-free structural**
    /// resolved per-slot credential identity, so a caller that resolved
    /// tenant A's credential reaches tenant A's row and never tenant B's.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no row of type `R` matches
    ///   `(scope, slot_identity)`.
    /// - Otherwise as [`acquire`](Self::acquire).
    ///
    /// # Cancel safety
    ///
    /// Same contract as [`acquire`](Self::acquire).
    pub async fn acquire_for_identity<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: Provider,
        R::Topology: Topology<R>,
    {
        let managed = self.lookup_for_acquire_with_identity::<R>(ctx, slot_identity)?;
        self.run_acquire_dispatch(managed, ctx, options).await
    }

    /// The per-unit checkout facade of the row of `R` registered for `ctx`'s
    /// scope: every attempt of a unit submitted on it checks out an
    /// instance of its own only after its quota and row-gate waits (see
    /// [`ManagedRow`](crate::call::ManagedRow)).
    ///
    /// Latches the row's rate-limit profile to
    /// [`RateLimitProfile::PerAttempt`](crate::RateLimitProfile::PerAttempt).
    /// The facade is bound to this registration: once it is removed or
    /// replaced, its units fail `Cancelled`. Its units inherit `ctx`'s
    /// cancellation token: once it fires, a unit whose first attempt was
    /// not granted yet settles `Cancelled` / `NotSent`; a granted one runs
    /// on to its deadline.
    ///
    /// # Errors
    ///
    /// As the acquire lookup: [`NotFound`](crate::ErrorKind::NotFound),
    /// [`Ambiguous`](crate::ErrorKind::Ambiguous) (use
    /// [`managed_row_for_identity`](Self::managed_row_for_identity)),
    /// [`Cancelled`](crate::ErrorKind::Cancelled) while shutting down,
    /// [`Revoked`](crate::ErrorKind::Revoked) for a tainted row.
    pub fn managed_row<R: Provider + crate::call::PinSlots>(
        &self,
        ctx: &ResourceContext,
    ) -> Result<crate::call::ManagedRow<R>, Error> {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        Ok(Self::row_facade(managed, self.acquire.clone(), ctx))
    }

    /// A row facade whose units inherit `ctx`'s cancellation.
    fn row_facade<R: Provider>(
        managed: Arc<ManagedResource<R>>,
        link: super::AcquireLink,
        ctx: &ResourceContext,
    ) -> crate::call::ManagedRow<R> {
        crate::call::ManagedRow::new(managed, link, ctx).with_unit_scope(
            crate::call::UnitScope::from_parts(ctx, &AcquireOptions::default()),
        )
    }

    /// [`managed_row`](Self::managed_row) pinned to the **collision-free
    /// structural** resolved per-slot credential identity.
    ///
    /// # Errors
    ///
    /// [`NotFound`](crate::ErrorKind::NotFound) if no row of type `R`
    /// matches `(scope, slot_identity)`; otherwise as
    /// [`managed_row`](Self::managed_row).
    pub fn managed_row_for_identity<R: Provider + crate::call::PinSlots>(
        &self,
        ctx: &ResourceContext,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<crate::call::ManagedRow<R>, Error> {
        let managed = self.lookup_for_acquire_with_identity::<R>(ctx, slot_identity)?;
        Ok(Self::row_facade(managed, self.acquire.clone(), ctx))
    }

    /// The type-erased [`managed_row_for_identity`](Self::managed_row_for_identity):
    /// a boxed [`ManagedRow<R>`](crate::call::ManagedRow) for the row
    /// registered under `key` for `(ctx`'s scope, `slot_identity)`, for a
    /// caller that knows the row only by its key (the engine's resource
    /// accessor). The caller downcasts it to the `ManagedRow<R>` it expects.
    ///
    /// Synchronous and checks out nothing: the row's capacity is each
    /// attempt's to wait for, so a pool saturated by plain leases still
    /// yields a facade. Latches the row's rate-limit profile to
    /// [`RateLimitProfile::PerAttempt`](crate::RateLimitProfile::PerAttempt).
    /// The facade's units inherit `ctx`'s cancellation token and
    /// `options.deadline` (see [`ManagedRow`](crate::call::ManagedRow)).
    ///
    /// # Errors
    ///
    /// As [`acquire_any`](Self::acquire_any)'s lookup:
    /// [`NotFound`](crate::ErrorKind::NotFound) — also for an unbound
    /// identity at a scope that has only credential-bound rows (fail
    /// closed: never one tenant's row) —,
    /// [`Ambiguous`](crate::ErrorKind::Ambiguous),
    /// [`Cancelled`](crate::ErrorKind::Cancelled) while shutting down,
    /// [`Revoked`](crate::ErrorKind::Revoked) for a tainted row.
    pub fn managed_row_any(
        &self,
        key: &ResourceKey,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<Box<dyn Any + Send + Sync>, Error> {
        use crate::registry::AcquireLookupOutcome;

        self.shutdown_guard()?;
        match self
            .registry
            .get_acquire_for(key, ctx.scope(), slot_identity)
        {
            AcquireLookupOutcome::Found { managed } => {
                tracing::debug!(
                    target: "nebula.resource",
                    %key,
                    ?slot_identity,
                    "managed_row_any: row facade resolved"
                );
                managed.managed_row_any(
                    self.acquire.clone(),
                    ctx,
                    crate::call::UnitScope::from_parts(ctx, options),
                )
            },
            AcquireLookupOutcome::NotFound => {
                tracing::debug!(target: "nebula.resource", %key, "managed_row_any: not found");
                Err(Error::not_found(key))
            },
            AcquireLookupOutcome::Ambiguous { rows } => {
                tracing::warn!(
                    target: "nebula.resource",
                    %key,
                    rows,
                    "managed_row_any: ambiguous scope/slot identity"
                );
                Err(Self::ambiguous_row_error(key, rows))
            },
        }
    }

    /// The refusal of an erased lookup that matched `rows` resolved-credential
    /// rows of `key` without a slot identity to tell them apart.
    fn ambiguous_row_error(key: &ResourceKey, rows: usize) -> Error {
        Error::ambiguous(format!(
            "{key}: {rows} resolved-credential registrations exist at this scope; \
             acquire must target a resolved row via slot identity"
        ))
        .with_resource_key(key.clone())
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
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. Dropping the future at any await point
    /// releases the topology permit, settles the drain accounting, and
    /// auto-fails a held recovery-gate probe ticket (its backoff applies).
    /// An instance in flight between checkout/create and the returned guard
    /// is destroyed asynchronously via the release queue, never leaked. The
    /// only effect of cancellation is that no guard is returned.
    ///
    /// # Examples
    ///
    /// See the doctest on [`register`](Self::register) for the full
    /// derive → `impl Provider` → register → acquire → deref → drop flow
    /// this method sits in the middle of.
    pub async fn acquire_pooled<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: PoolProvider
            + Provider<Topology = crate::topology::Pooled<R>>
            + Clone
            + Send
            + Sync
            + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        self.run_acquire_dispatch(managed, ctx, options).await
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
    ///
    /// # Cancel safety
    ///
    /// Cancel safe — same contract as
    /// [`acquire_pooled`](Self::acquire_pooled): permit, drain accounting,
    /// and gate ticket all settle on drop; an in-flight instance is destroyed
    /// asynchronously via the release queue.
    pub async fn acquire_pooled_for_identity<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: PoolProvider
            + Provider<Topology = crate::topology::Pooled<R>>
            + Clone
            + Send
            + Sync
            + 'static,
    {
        let managed = self.lookup_for_acquire_with_identity::<R>(ctx, slot_identity)?;
        self.run_acquire_dispatch(managed, ctx, options).await
    }

    /// Single generic topology dispatch into the shared
    /// [`run_acquire`](Self::run_acquire) pipeline.
    ///
    /// The dispatch closure runs the **framework acquire loop**
    /// ([`ManagedResource::run_acquire_loop`]): the framework owns the fenced
    /// checkout, stale-slot destroy, cancel-safe guard-wrap, and on-release
    /// return-or-destroy; the resource's
    /// [`Provider::Topology`] supplies only
    /// thin R-aware hooks. There is no runtime variant to mismatch — the
    /// topology is pinned to `R` by the associated type. The loop re-reads
    /// `config`/`generation` itself, so they are fresh on every resilience
    /// retry.
    pub(crate) async fn run_acquire_dispatch<R>(
        &self,
        managed: Arc<ManagedResource<R>>,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: Provider,
        R::Topology: Topology<R>,
    {
        // Foolproofing for open (third-party) topologies: bound the author's
        // acquire hooks (`try_reserve` / `create_entry` / `accept` / `prepare`)
        // so a careless `impl Topology` cannot wedge the caller by hanging, nor
        // crash it by panicking. The caller's deadline wins; absent one, a
        // framework ceiling caps the worst case so a blocking hook can never
        // hang forever. The dropped loop future releases the permit and
        // destroys any in-flight entry via `EntryCreateGuard`.
        //
        // The hook timeout is what is left of the caller's budget when the
        // hooks run, not the budget at entry: the rate-limit wait and the
        // strict credential read spend the same budget first, and the
        // acquire must not end past its deadline.
        let budget = options.remaining();
        let entered = tokio::time::Instant::now();
        self.run_acquire(Arc::clone(&managed), ctx, options, || {
            let hook_timeout = budget.map_or(DEFAULT_AUTHOR_HOOK_CEILING, |budget| {
                budget.saturating_sub(entered.elapsed())
            });
            self.acquire
                .dispatch_checkout(&managed, ctx, options, hook_timeout)
        })
        .await
    }

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
        ctx: &ResourceContext,
        options: &AcquireOptions,
        dispatch: F,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: Provider,
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<crate::guard::ResourceGuard<R>, Error>> + Send,
    {
        let started = Instant::now();
        // Rate limit first, before this acquire is counted as in flight: a
        // caller queued for a slot must not hold up a revoke drain or
        // graceful shutdown. The post-count checks below still reject it if
        // either began while it waited. Local state, so it runs before the
        // recovery gate and a denial never reads as backend ill health.
        tokio::select! {
            ready = managed.rate_limiter.ready_to_acquire(options.deadline) => ready?,
            () = self.cancel.cancelled() => return Err(Error::cancelled()),
        }
        // Strict credential admission, phase 1: read every bound credential's
        // availability now, outside every lock (zero reads for a slot-less or
        // interim row). Phase 2 applies it under `Manager.admission` below.
        // Dropping this acquire drops the read.
        let strict = managed.read_credentials_strict(options.remaining()).await;
        // Lock #1, the recovery gate, the dispatch and the hand-out check:
        // the part a managed row attempt shares (`AcquireLink`).
        self.acquire
            .acquire_admitted(managed, ctx, options, strict.as_ref(), started, dispatch)
            .await
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
    ///
    /// # Cancel safety
    ///
    /// Cancel safe — same contract as
    /// [`acquire_pooled`](Self::acquire_pooled): permit, drain accounting,
    /// and gate ticket all settle on drop; an in-flight instance is destroyed
    /// asynchronously via the release queue.
    pub async fn acquire_resident<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: ResidentProvider
            + Provider<Topology = crate::topology::Resident<R>>
            + Send
            + Sync
            + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        self.run_acquire_dispatch(managed, ctx, options).await
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
    ///
    /// # Cancel safety
    ///
    /// Cancel safe — same contract as
    /// [`acquire_pooled`](Self::acquire_pooled): permit, drain accounting,
    /// and gate ticket all settle on drop; an in-flight instance is destroyed
    /// asynchronously via the release queue.
    pub async fn acquire_resident_for_identity<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: ResidentProvider
            + Provider<Topology = crate::topology::Resident<R>>
            + Send
            + Sync
            + 'static,
    {
        let managed = self.lookup_for_acquire_with_identity::<R>(ctx, slot_identity)?;
        self.run_acquire_dispatch(managed, ctx, options).await
    }

    /// Acquires a handle to a bounded resource.
    ///
    /// Bounded holds no owned `R`-typed instance in the topology itself — it
    /// gates concurrency (unbounded / capped / exclusive) over a resource
    /// that either builds fresh per lease (`Capped`/`Unbounded`) or reuses a
    /// single reset-between-leases instance (`Exclusive`); see
    /// [`Bounded`](crate::topology::Bounded) and [`BoundedMode`](crate::topology::BoundedMode).
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::NotFound`](crate::error::ErrorKind::NotFound) if no resource of type `R` is
    ///   registered.
    /// - [`ErrorKind::Permanent`](crate::error::ErrorKind::Permanent) if the resource is not using
    ///   bounded topology.
    /// - [`ErrorKind::Ambiguous`](crate::error::ErrorKind::Ambiguous) — a
    ///   permanent (non-retryable) caller-conflict deny — if more than one
    ///   resolved-credential registration exists for `(R, scope)`
    ///   (multi-tenant). Acquire through the slot-identity-pinned
    ///   [`acquire_bounded_for_identity`](Self::acquire_bounded_for_identity)
    ///   when the resolved slot identity is known; this identity-agnostic
    ///   path stays fail-closed for the no-identity caller.
    /// - Propagates bounded-specific acquire errors (e.g. `Saturated` when
    ///   the concurrency cap is exhausted).
    ///
    /// # Cancel safety
    ///
    /// Cancel safe — same contract as
    /// [`acquire_pooled`](Self::acquire_pooled): permit, drain accounting,
    /// and gate ticket all settle on drop; an in-flight instance is destroyed
    /// asynchronously via the release queue.
    pub async fn acquire_bounded<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: BoundedProvider
            + Provider<Topology = crate::topology::Bounded<R>>
            + Send
            + Sync
            + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        self.run_acquire_dispatch(managed, ctx, options).await
    }

    /// [`acquire_bounded`](Self::acquire_bounded) pinned to the
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
    ///   bounded topology.
    /// - Propagates bounded-specific acquire errors (e.g. `Saturated` when
    ///   the concurrency cap is exhausted).
    ///
    /// # Cancel safety
    ///
    /// Cancel safe — same contract as
    /// [`acquire_pooled`](Self::acquire_pooled): permit, drain accounting,
    /// and gate ticket all settle on drop; an in-flight instance is destroyed
    /// asynchronously via the release queue.
    pub async fn acquire_bounded_for_identity<R>(
        &self,
        ctx: &ResourceContext,
        options: &AcquireOptions,
        slot_identity: &crate::dedup::SlotIdentity,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: BoundedProvider
            + Provider<Topology = crate::topology::Bounded<R>>
            + Send
            + Sync
            + 'static,
    {
        let managed = self.lookup_for_acquire_with_identity::<R>(ctx, slot_identity)?;
        self.run_acquire_dispatch(managed, ctx, options).await
    }

    /// Returns a snapshot of current pool utilization for a registered Pool resource.
    ///
    /// Returns `None` if the resource is not registered or does not use Pool topology.
    pub async fn pool_stats<R>(&self, scope: &ScopeLevel) -> Option<crate::PoolStats>
    where
        R: PoolProvider
            + Provider<Topology = crate::topology::Pooled<R>>
            + Clone
            + Send
            + Sync
            + 'static,
    {
        let managed = self.lookup::<R>(scope).ok()?;
        Some(managed.topology.stats(&managed.store).await)
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
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. Slots already deposited stay in the pool;
    /// a slot in flight between creation and its fenced deposit is destroyed
    /// asynchronously via the release queue (this also covers the internal
    /// author-hook ceiling timeout). Cancellation only means the caller
    /// never learns how many slots were admitted.
    pub async fn warmup_pool<R>(&self, ctx: &ResourceContext) -> Result<usize, Error>
    where
        R: PoolProvider
            + Provider<Topology = crate::topology::Pooled<R>>
            + Clone
            + Send
            + Sync
            + 'static,
    {
        let managed = self.lookup_for_acquire_scope::<R>(ctx)?;
        let config = managed.config();
        // `warmup` runs `R::create` against the resolved credential to
        // materialize fresh pool instances — it is acquire-like and must
        // observe the SAME post-count re-check the `run_*_acquire` pipelines
        // use (#679 / slot + isolation model). `lookup_for_acquire`'s taint
        // gate *and* `shutdown_guard` both ran *before* this in-flight
        // increment, leaving the two symmetric windows: a concurrent
        // `revoke_slot` could taint, or `graceful_shutdown` could
        // drain-see-`0` + clear the registry, after the gate yet before
        // warmup creates entries. Pre-count this work in both the resource's
        // own in-flight counter (the exact counter `revoke_slot` drains) and
        // the manager-wide `drain_tracker` (`graceful_shutdown`), then
        // re-check both: either we observe taint / `shutting_down` here and
        // reject, or our increment is visible to the respective drain — so no
        // fresh pool entry is ever created on a just-revoked credential or
        // after a completed shutdown drain. The counter is held for the whole
        // `warmup` await (RAII drop on every exit path).
        let _in_flight =
            InFlightCounter::new(self.drain_tracker.clone(), managed.in_flight_tracker());
        self.reject_if_tainted_or_shutting_down_post_count::<R>(&managed)?;
        // Every create is a unit of work of its own: a strict row reads its
        // bound credentials immediately before each create, applies what the
        // read saw to the row's gate, and stops at the first refusal. A
        // refusal before anything was built is the warmup's error.
        let refusal = std::sync::Mutex::new(None);
        let admit = {
            let (managed, refusal) = (&managed, &refusal);
            move || async move {
                match self.strict_credential_admission(managed, None).await {
                    Ok(()) => true,
                    Err(error) => {
                        refusal
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .get_or_insert(error);
                        false
                    },
                }
            }
        };
        // The framework-owned warmup creates `warmup_target` entries via the
        // topology's `create_entry` (which runs the author's `Provider::create`)
        // and deposits them (fenced) into the framework store. `config` is read
        // inside `warmup` itself. Bound + isolate it through the same guard the
        // acquire pipeline uses: a careless `Provider::create` that hangs or
        // panics during warmup must fail closed, not wedge or crash the caller.
        let _ = config;
        // `warmup` bounds and isolates each `create_entry` hook under the
        // author-hook ceiling itself (the stagger interval between creates is
        // not part of that budget) and reports the first fault.
        let count = match managed.warmup_admitted(ctx, admit).await {
            Ok(0) => {
                let refused = refusal
                    .into_inner()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                return refused.map_or(Ok(0), Err);
            },
            Ok(n) => n,
            Err(fault) => {
                fault.observe(&R::key(), "warmup");
                match fault {
                    HookFault::Panicked => {
                        return Err(Error::permanent(format!(
                            "{}: warmup panicked — the topology's `create_entry` hook unwound \
                             (isolated, caller not crashed)",
                            R::key()
                        )));
                    },
                    HookFault::TimedOut => {
                        return Err(Error::backpressure(format!(
                            "{}: a warmup create exceeded {DEFAULT_AUTHOR_HOOK_CEILING:?} — the topology's \
                             `create_entry` hook did not complete in time",
                            R::key()
                        )));
                    },
                }
            },
        };
        Ok(count)
    }
}
