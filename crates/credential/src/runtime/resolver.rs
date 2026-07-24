//! Runtime credential resolution (ADR-0092).
//!
//! Relocated from `nebula-engine::credential::resolver` so the whole
//! credential subsystem lives in one crate. No `nebula-engine` or
//! `nebula-storage` edge — transport is injected via [`RefreshTransport`].

use std::{
    any::{Any, TypeId},
    sync::Arc,
};

#[cfg(test)]
use crate::error::CredentialError;
use crate::runtime::refresh::transport::RefreshTransport;
use crate::runtime::refresh::{
    ReauthWrite, RefreshCoordinator, RefreshDisposition, RefreshError, RefreshRecheck,
    RefreshRecheckError, RetryGateWrite, context_from_block, persist_reauth_required,
    persist_retry_gate,
};
use crate::runtime::resolve_error::{
    ResolveError, reject_tombstoned, resolve_error_to_credential_error,
};
use crate::{
    Credential, CredentialContext, CredentialEvent, CredentialHandle, CredentialId,
    CredentialLifecycle, CredentialMaterialTransition, CredentialPersistence,
    CredentialPersistenceError, CredentialReplacement, CredentialSelector, CredentialState,
    Decision, LAST_VALIDATED_AT_METADATA_KEY, RefreshAttempt, RefreshNotAppliedContext,
    RefreshRetryAdmission, Refreshable, SchemeFactory, SchemeGuard, StoredCredential,
    StoredLiveCredential,
    contract::{RefreshReauthPhase, RefreshReportKind},
    resolve::ReauthReason,
};

/// Framework-imposed mandatory re-validation floor for a refreshable credential
/// that carries neither an inline expiry nor a lease — the backstop that keeps
/// even a signal-less refreshable credential from being served indefinitely
/// without re-contacting its provider. Owner ruling: there is no "valid forever".
/// (Per-credential override is a later configuration concern; this is the default.)
const DEFAULT_REVALIDATION_FLOOR: std::time::Duration = std::time::Duration::from_hours(24);

/// Bound transparent re-evaluation when authoritative state changes while a
/// caller waits behind L1/L2. Continuous management churn must not create an
/// unbounded async recursion chain or retain stale snapshots indefinitely.
const MAX_COORDINATED_REEVALUATIONS: usize = 3;

fn reject_persisted_reauth(stored: &StoredLiveCredential) -> Result<(), ResolveError> {
    if stored.reauth_required() {
        return Err(ResolveError::ReauthRequired {
            credential_id: stored.credential_id().to_string(),
            // K2 persists only the security decision, not its reason. Both a
            // provider rejection and locally missing refresh material can set
            // this bit; subsequent reads therefore use the conservative
            // ProviderRejected classification. A reason-bearing durable
            // transition is explicit K3 model debt.
            reason: ReauthReason::ProviderRejected,
        });
    }
    Ok(())
}

fn last_validated_at(stored: &StoredLiveCredential) -> Option<chrono::DateTime<chrono::Utc>> {
    stored
        .metadata()
        .get(LAST_VALIDATED_AT_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|instant| instant.with_timezone(&chrono::Utc))
}

fn last_validated_or_created(stored: &StoredLiveCredential) -> chrono::DateTime<chrono::Utc> {
    last_validated_at(stored).unwrap_or_else(|| stored.created_at())
}

use dashmap::DashMap;
use nebula_core::auth::{AuthScheme, SchemeFamily};
use nebula_eventbus::EventBus;

/// Live [`CredentialHandle`] cache keyed by `(credential_id, scheme TypeId)`.
///
/// Read-heavy / write-light: `resolve` and `resolve_with_refresh` read on
/// every call; writes occur only on first resolve (insert) or after a
/// successful refresh (`replace`). `DashMap`'s per-shard sharding eliminates
/// the write-lock contention that `Mutex<HashMap>` imposed under concurrent
/// resolution — a write to one shard does not block reads on others.
type HandleCache = DashMap<(CredentialSelector, TypeId), Arc<dyn Any + Send + Sync>>;

enum CoordinatedResolve<S: AuthScheme> {
    Resolved(CredentialHandle<S>),
    Reevaluate,
}

fn map_refresh_disposition<T, U>(
    disposition: RefreshDisposition<T>,
    map: impl FnOnce(T) -> U,
) -> RefreshDisposition<U> {
    match disposition {
        RefreshDisposition::StateAdvanced(value) => RefreshDisposition::state_advanced(map(value)),
        RefreshDisposition::NoStateChange(value) => RefreshDisposition::no_state_change(map(value)),
        RefreshDisposition::ReplaySafe(value) => RefreshDisposition::replay_safe(map(value)),
        RefreshDisposition::RetryUnsafe(value) => RefreshDisposition::retry_unsafe(map(value)),
        RefreshDisposition::OutcomeUnknown(value) => {
            RefreshDisposition::outcome_unknown(map(value))
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RefreshCommitPhase {
    ProviderConfirmed,
    LocalOnly,
}

/// Runtime credential resolver with optional coordinated refresh.
pub struct CredentialResolver<S: CredentialPersistence + ?Sized> {
    store: Arc<S>,
    refresh_coordinator: Arc<RefreshCoordinator>,
    transport: Arc<dyn RefreshTransport>,
    event_bus: Option<Arc<EventBus<CredentialEvent>>>,
    /// Live [`CredentialHandle`]s keyed by `(credential_id, scheme TypeId)` so
    /// refresh can [`CredentialHandle::replace`] in place instead of minting
    /// disconnected handles on every resolve/refresh cycle.
    handle_cache: Arc<HandleCache>,
    /// When `true`, the service is configured with an external
    /// [`StateSource`](crate::StateSource) whose resolution bridge is not yet wired,
    /// so **every** resolution path refuses to read local bytes (fail-closed at
    /// the resolver tail — see [`gate_external_source`](Self::gate_external_source)).
    /// This closes the source gate on the direct-resolver paths
    /// (`scheme_factory` → `resolve_with_refresh`) that bypass the facade's
    /// per-call check, by construction rather than by discipline.
    external_source_unwired: bool,
}

impl<S: CredentialPersistence + ?Sized> Clone for CredentialResolver<S> {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            refresh_coordinator: Arc::clone(&self.refresh_coordinator),
            transport: Arc::clone(&self.transport),
            event_bus: self.event_bus.clone(),
            handle_cache: Arc::clone(&self.handle_cache),
            external_source_unwired: self.external_source_unwired,
        }
    }
}

impl<S: CredentialPersistence + ?Sized> CredentialResolver<S> {
    /// Construct a resolver from all required collaborators.
    ///
    /// Production composition roots call this directly, supplying a durable
    /// `RefreshCoordinator` (Postgres / SQLite `RefreshClaimRepo`) and a
    /// `ReqwestRefreshTransport`. Tests may inject an in-memory coordinator
    /// and a stub transport.
    #[must_use]
    pub fn with_dependencies(
        store: Arc<S>,
        refresh_coordinator: Arc<RefreshCoordinator>,
        transport: Arc<dyn RefreshTransport>,
    ) -> Self {
        Self {
            store,
            refresh_coordinator,
            transport,
            event_bus: None,
            handle_cache: Arc::new(DashMap::new()),
            external_source_unwired: false,
        }
    }

    /// Fail-closed the resolver against an external, not-yet-wired state source.
    ///
    /// Set by the composition root when the service is built with
    /// [`StateSource::External`](crate::StateSource). Once gated, **every** resolution
    /// entry point ([`resolve_scoped`](Self::resolve_scoped) /
    /// [`resolve_with_refresh`](Self::resolve_with_refresh), and therefore
    /// [`scheme_factory`](Self::scheme_factory)) returns
    /// [`ResolveError::ExternalSourceNotWired`] instead of reading local bytes —
    /// so the direct-resolver paths that bypass the facade's per-call source
    /// check cannot silently resolve from the wrong place. The external provider
    /// resolution bridge (ADR-0051) is not yet built; until it lands, gated is a
    /// hard error, never a local-store fallback.
    #[must_use = "builder methods must be chained or built"]
    pub fn gate_external_source(mut self, unwired: bool) -> Self {
        self.external_source_unwired = unwired;
        self
    }

    fn handle_cache_key<C: Credential>(
        selector: &CredentialSelector,
    ) -> (CredentialSelector, TypeId) {
        (selector.clone(), TypeId::of::<C::Scheme>())
    }

    fn cached_handle<C: Credential>(
        &self,
        selector: &CredentialSelector,
    ) -> Option<CredentialHandle<C::Scheme>> {
        let key = Self::handle_cache_key::<C>(selector);
        self.handle_cache.get(&key).and_then(|entry| {
            entry
                .value()
                .clone()
                .downcast::<CredentialHandle<C::Scheme>>()
                .ok()
                .map(|arc| (*arc).clone())
        })
    }

    fn store_handle<C: Credential>(
        &self,
        selector: &CredentialSelector,
        handle: CredentialHandle<C::Scheme>,
    ) {
        let key = Self::handle_cache_key::<C>(selector);
        self.handle_cache.insert(key, Arc::new(handle));
    }

    fn materialize_handle<C: Credential>(
        &self,
        selector: &CredentialSelector,
        scheme: C::Scheme,
    ) -> CredentialHandle<C::Scheme> {
        if let Some(existing) = self.cached_handle::<C>(selector) {
            existing.replace(scheme);
            return existing;
        }
        let handle = CredentialHandle::new(scheme, selector.credential_id().to_string());
        self.store_handle::<C>(selector, handle.clone());
        handle
    }

    /// Per-request scheme re-acquisition for long-lived resources (Tech Spec §15.7).
    ///
    /// The returned [`SchemeFactory`] delegates to
    /// [`resolve_with_refresh`](Self::resolve_with_refresh) on each
    /// [`SchemeFactory::acquire`] call, yielding a lifetime-pinned
    /// [`SchemeGuard`] suitable for scoped use inside a single task.
    pub fn scheme_factory<C>(
        &self,
        selector: CredentialSelector,
        ctx: CredentialContext,
    ) -> SchemeFactory<C>
    where
        S: CredentialPersistence + 'static,
        C: Refreshable + CredentialLifecycle,
        C::Scheme: zeroize::Zeroize + Clone + Send + Sync + 'static,
    {
        let resolver = self.clone();
        SchemeFactory::new(move || {
            let resolver = resolver.clone();
            let selector = selector.clone();
            let ctx = ctx.clone();
            Box::pin(async move {
                let handle = resolver
                    .resolve_with_refresh::<C>(&selector, &ctx)
                    .await
                    .map_err(resolve_error_to_credential_error)?;
                let scheme =
                    Arc::try_unwrap(handle.snapshot()).unwrap_or_else(|arc| (*arc).clone());
                Ok(SchemeGuard::new(scheme))
            })
        })
    }

    /// Attach an event bus to emit credential refresh lifecycle events.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_event_bus(mut self, bus: Arc<EventBus<CredentialEvent>>) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// Replace the refresh coordinator.
    ///
    /// Composition root threads `Arc<RefreshCoordinator>` constructed via
    /// `RefreshCoordinator::new_with(repo, replica_id, config)` (where
    /// `repo` is a Postgres / SQLite `RefreshClaimRepo` for production) here.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_refresh_coordinator(mut self, coord: Arc<RefreshCoordinator>) -> Self {
        self.refresh_coordinator = coord;
        self
    }

    /// Test-only direct resolution path used to exercise common load gates
    /// independently from the action-slot façade.
    #[cfg(test)]
    async fn resolve<C>(
        &self,
        selector: &CredentialSelector,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Credential,
    {
        self.ensure_source_wired()?;
        let credential_id = selector.credential_id();
        let stored = self.load_and_verify::<C>(selector).await?;
        let state: C::State = self.deserialize::<C>(credential_id, &stored)?;
        let scheme = C::project(&state);
        Ok(self.materialize_handle::<C>(selector, scheme))
    }

    /// Resolve a credential for an action slot through its owner-scoped key.
    ///
    /// The [`CredentialSelector`] is obtainable only from a
    /// `ValidatedCredentialBinding` (whose constructor is gated by
    /// `CredentialService::validate_credential_binding`). This method
    /// Relies on the persistence port's complete `(owner, credential_id)`
    /// predicate before projecting the scheme, so a credential id belonging to
    /// another tenant resolves to [`CredentialPersistenceError::NotFound`]
    /// (existence-hiding). Owner metadata is ordinary application data and is
    /// never treated as authority.
    ///
    /// # Errors
    ///
    /// Returns [`ResolveError::Store`] with [`CredentialPersistenceError::NotFound`] when the id
    /// is absent **or** the stored row's owner does not match the key; other
    /// [`ResolveError`] variants on kind-mismatch or deserialization failure.
    pub async fn resolve_scoped<C>(
        &self,
        selector: &CredentialSelector,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Credential,
    {
        self.ensure_source_wired()?;
        // The port applies the complete owner-bound selector before returning a
        // physical record, closing the cross-tenant existence oracle before
        // this code can inspect lifecycle state or kind.
        let physical = self
            .store
            .get(selector)
            .await
            .map_err(ResolveError::Store)?;
        reject_tombstoned(&physical)?;
        let StoredCredential::Live(stored) = physical else {
            return Err(ResolveError::Store(CredentialPersistenceError::NotFound));
        };
        reject_persisted_reauth(&stored)?;

        let expected_kind = <C::State as CredentialState>::KIND;
        if stored.state_kind() != expected_kind {
            return Err(ResolveError::KindMismatch {
                credential_id: selector.credential_id().to_string(),
                expected: expected_kind.to_string(),
                actual: stored.state_kind().to_owned(),
            });
        }

        let state: C::State = self.deserialize::<C>(selector.credential_id(), &stored)?;
        let scheme = C::project(&state);
        Ok(self.materialize_handle::<C>(selector, scheme))
    }

    /// Resolve a credential and refresh it when it enters the early-refresh window.
    ///
    /// Per Tech Spec — bound on [`Refreshable`] so a non-refreshable
    /// credential cannot reach this dispatch path. Probe 4
    /// (`compile_fail_engine_dispatch_capability`) cements the structural
    /// barrier with `E0277` at the dispatch site.
    ///
    /// Refresh always goes through the two-tier
    /// [`RefreshCoordinator::refresh_coalesced`]: the persistence selector
    /// carries a typed [`CredentialId`], so there is no legacy string-id bypass.
    /// `CoalescedByOtherReplica` is success — caller re-reads state.
    pub async fn resolve_with_refresh<C>(
        &self,
        selector: &CredentialSelector,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        S: 'static,
        C: Refreshable + CredentialLifecycle,
    {
        self.ensure_source_wired()?;
        for reevaluation in 0..=MAX_COORDINATED_REEVALUATIONS {
            let credential_id = selector.credential_id();
            let credential_id_text = credential_id.to_string();
            let stored = self.load_and_verify::<C>(selector).await?;
            let state: C::State = self.deserialize::<C>(credential_id, &stored)?;

            // Route on the credential's own state-derived policy, not an ad-hoc
            // inline expiry test: `decide_refresh` is the single, pure, tested
            // decision. It distinguishes "expiring but nothing to renew" (serve and
            // let it ride) from "expiring and renewable" (refresh), and applies the
            // mandatory re-validation floor for a signal-less credential. Jitter is
            // deliberately not applied on this hot path — proactive jittered refresh
            // is a scheduler-seam concern, not a per-resolve one.
            let policy = C::policy(&state);

            // F3 containment law, state-level: the live policy's refresh kind must be
            // one the scheme family sanctions. Registration enforces the
            // capability-level half at boot (a `Refreshable` credential on a
            // `Static`-only family is rejected); this runtime guard catches a
            // hand-written or plugin policy that returns a refresh kind outside its
            // family's declared classes. `Lease` and `Watched` are exempt (orthogonal
            // lifecycle wrappers — see `SchemeFamily::refresh_classes`).
            //
            // Hard `Err` in all build profiles — a policy drift is a security
            // containment violation that must not silently proceed even in release.
            // The structured `RefreshContainmentViolation` error carries all the
            // diagnostic information (credential id, disallowed kind, family pattern)
            // that a developer needs to diagnose the drift without a backtrace.
            if !<C::Scheme as AuthScheme>::Family::permits_refresh(policy.refresh.kind()) {
                return Err(ResolveError::RefreshContainmentViolation {
                    credential_id: credential_id_text.clone(),
                    refresh_kind: format!("{:?}", policy.refresh.kind()),
                    family_pattern: format!("{:?}", <C::Scheme as AuthScheme>::Family::pattern()),
                });
            }

            let decision = policy.decide_refresh(
                // Measure the re-validation floor from the last real provider
                // validation, NOT `updated_at` (a display-only rename/tag bumps
                // `updated_at` without revalidating — it must not postpone the floor).
                last_validated_or_created(&stored),
                chrono::Utc::now(),
                <C as Refreshable>::REFRESH_POLICY.early_refresh,
                DEFAULT_REVALIDATION_FLOOR,
            );

            if decision == Decision::Usable {
                let scheme = C::project(&state);
                return Ok(self.materialize_handle::<C>(selector, scheme));
            }

            self.ensure_refresh_admitted(selector).await?;

            if self
                .refresh_coordinator
                .is_circuit_open(&credential_id_text)
            {
                let now = chrono::Utc::now();
                let truly_expired = state.expires_at().is_some_and(|exp| exp <= now);
                if truly_expired {
                    tracing::warn!(
                        credential_id = %credential_id,
                        "circuit breaker open and token has passed its expiry; failing fast"
                    );
                    return Err(ResolveError::Refresh {
                        credential_id: credential_id_text,
                        reason: "refresh circuit breaker open and token is expired".to_string(),
                    });
                }
                tracing::warn!(
                    credential_id = %credential_id,
                    "circuit breaker open: too many refresh failures, serving stale-but-valid credential within early-refresh window"
                );
                let scheme = C::project(&state);
                return Ok(self.materialize_handle::<C>(selector, scheme));
            }

            match self
                .refresh_via_coordinator::<C>(selector, &credential_id, stored, ctx)
                .await?
            {
                CoordinatedResolve::Resolved(handle) => return Ok(handle),
                CoordinatedResolve::Reevaluate if reevaluation < MAX_COORDINATED_REEVALUATIONS => {
                    continue;
                },
                CoordinatedResolve::Reevaluate => {
                    return Err(ResolveError::Refresh {
                        credential_id: credential_id_text,
                        reason: "credential state kept changing during coordinated refresh"
                            .to_owned(),
                    });
                },
            }
        }
        unreachable!("the bounded coordinated re-evaluation loop always returns")
    }

    /// Two-tier coordinated refresh path for a typed [`CredentialId`].
    async fn refresh_via_coordinator<C>(
        &self,
        selector: &CredentialSelector,
        typed_id: &CredentialId,
        stored: StoredLiveCredential,
        ctx: &CredentialContext,
    ) -> Result<CoordinatedResolve<C::Scheme>, ResolveError>
    where
        S: 'static,
        C: Refreshable + CredentialLifecycle,
    {
        let credential_id = selector.credential_id();
        let credential_id_text = credential_id.to_string();
        // The coordinator owns transport/claim failures. The critical closure
        // returns the resolver result together with its exact commit
        // disposition so `OutcomeUnknown` retains L2 to TTL.
        let coord = Arc::clone(&self.refresh_coordinator);
        let resolver = self.clone();
        let resolver_stored = stored;
        let observed_material_epoch = resolver_stored.material_epoch();
        let selector_owned = selector.clone();
        let ctx_owned = ctx.clone();

        // Every admission point (after L1 wake, L2 contention, and immediate
        // L2 acquisition) consumes one backend-atomic snapshot. A blocked gate
        // wins even when installing it advanced the row version. Display-only
        // writes preserve the material epoch and therefore remain `Needed`;
        // only a newer material epoch or durable reauth decision is
        // `Satisfied` and forces parent-path re-evaluation.
        // Combining a separate admission read with `get` would permit a gate
        // write between them to masquerade as successful coalescing.
        let store_for_recheck = Arc::clone(&self.store);
        let recheck_selector = selector.clone();
        let needs_refresh_after_backoff = move |_id: &CredentialId| {
            let store = Arc::clone(&store_for_recheck);
            let selector = recheck_selector.clone();
            async move {
                let credential_id = selector.credential_id();
                let snapshot = match store.refresh_retry_snapshot(&selector).await {
                    Ok(snapshot) => snapshot,
                    Err(CredentialPersistenceError::NotFound) => {
                        return Ok(RefreshRecheck::Satisfied);
                    },
                    Err(CredentialPersistenceError::CorruptRecord) => {
                        return Err(RefreshRecheckError::InvalidState);
                    },
                    Err(_) => return Err(RefreshRecheckError::Unavailable),
                };
                if let RefreshRetryAdmission::Blocked(block) = snapshot.admission() {
                    let context = context_from_block(block.clone())
                        .map_err(|_| RefreshRecheckError::InvalidState)?;
                    return Ok(RefreshRecheck::Suppressed(context));
                }
                if snapshot.material_epoch() != observed_material_epoch {
                    tracing::debug!(
                        credential_id = %credential_id,
                        observed_material_epoch = %observed_material_epoch,
                        current_material_epoch = %snapshot.material_epoch(),
                        "post-backoff state recheck: captured material authority is stale; re-reading through the parent path"
                    );
                    return Ok(RefreshRecheck::Satisfied);
                }
                if snapshot.reauth_required() {
                    tracing::debug!(
                        credential_id = %credential_id,
                        "post-backoff state recheck: reauth_required=true on stored \
                         credential — short-circuiting to CoalescedByOtherReplica \
                         (sub-spec §3.6 / I1)"
                    );
                    return Ok(RefreshRecheck::Satisfied);
                }
                Ok(RefreshRecheck::Needed)
            }
        };

        let outcome: Result<Result<CoordinatedResolve<C::Scheme>, ResolveError>, RefreshError> =
            coord
                .refresh_coalesced(typed_id, needs_refresh_after_backoff, move || async move {
                    // The coordinator has durably marked RefreshInFlight and
                    // transferred both claim and heartbeat into this owned task
                    // before invoking us. From this point provider contact and
                    // its persistence transition cannot be cancelled by caller
                    // Drop, timeout, or heartbeat loss.
                    //
                    // Re-read once more after acquisition. A display-only write
                    // may have advanced the CAS version while preserving the
                    // refresh authority; dispatch must use that latest row so a
                    // harmless rename cannot turn provider success into an
                    // unsafe post-provider conflict. Conversely, a material
                    // epoch advance after the atomic recheck supersedes this
                    // attempt before provider contact.
                    let latest = match resolver.load_and_verify::<C>(&selector_owned).await {
                        Ok(latest) => latest,
                        Err(error) => {
                            return RefreshDisposition::no_state_change(Err(error));
                        },
                    };
                    if latest.material_epoch() != observed_material_epoch {
                        return RefreshDisposition::state_advanced(Ok(
                            CoordinatedResolve::Reevaluate,
                        ));
                    }
                    let latest_state =
                        match resolver.deserialize::<C>(selector_owned.credential_id(), &latest) {
                            Ok(state) => state,
                            Err(error) => {
                                return RefreshDisposition::no_state_change(Err(error));
                            },
                        };
                    map_refresh_disposition(
                        resolver
                            .perform_refresh::<C>(&selector_owned, latest_state, latest, &ctx_owned)
                            .await,
                        |result| result.map(CoordinatedResolve::Resolved),
                    )
                })
                .await;

        match outcome {
            Ok(Ok(result)) => {
                self.refresh_coordinator.record_success(&credential_id_text);
                Ok(result)
            },
            Ok(Err(e)) => {
                self.refresh_coordinator.record_failure(&credential_id_text);
                Err(e)
            },
            // CoalescedByOtherReplica is success — another replica refreshed
            // while we were waiting on L2. Re-read state from the store.
            Err(RefreshError::CoalescedByOtherReplica) => {
                tracing::debug!(
                    credential_id = %credential_id,
                    "refresh coalesced by another replica; re-reading state from store"
                );
                self.refresh_coordinator.record_success(&credential_id_text);
                Ok(CoordinatedResolve::Reevaluate)
            },
            Err(RefreshError::CriticalOutcomePending) => {
                self.refresh_coordinator.record_failure(&credential_id_text);
                Err(ResolveError::RefreshOutcomePending {
                    credential_id: credential_id_text,
                })
            },
            Err(RefreshError::ReconciliationRequired) => {
                self.refresh_coordinator.record_failure(&credential_id_text);
                Err(ResolveError::RefreshReconciliationRequired {
                    credential_id: credential_id_text,
                })
            },
            Err(RefreshError::StateRecheck(RefreshRecheckError::Unavailable)) => {
                self.refresh_coordinator.record_failure(&credential_id_text);
                Err(ResolveError::Store(CredentialPersistenceError::Unavailable))
            },
            Err(RefreshError::StateRecheck(RefreshRecheckError::InvalidState)) => {
                self.refresh_coordinator.record_failure(&credential_id_text);
                Err(ResolveError::Store(
                    CredentialPersistenceError::CorruptRecord,
                ))
            },
            Err(RefreshError::RetrySuppressed(context)) => {
                self.refresh_coordinator.record_failure(&credential_id_text);
                Err(ResolveError::RefreshNotApplied {
                    credential_id: credential_id_text,
                    context,
                })
            },
            Err(e) => {
                self.refresh_coordinator.record_failure(&credential_id_text);
                Err(ResolveError::Refresh {
                    credential_id: credential_id_text,
                    reason: e.to_string(),
                })
            },
        }
    }

    /// Access the refresh coordinator used by this resolver.
    pub fn refresh_coordinator(&self) -> &RefreshCoordinator {
        &self.refresh_coordinator
    }

    /// Derive the resolver-owned, non-request-cancellable refresh context.
    ///
    /// This crate-private seam is shared by execution-time refresh and the
    /// management forced-refresh path. The resolver remains the sole owner of
    /// the provider transport; callers cannot inject a parallel authority.
    pub(crate) fn refresh_context(&self, request: &CredentialContext) -> CredentialContext {
        request.for_refresh_critical_section(Arc::clone(&self.transport))
    }

    fn emit_refreshed(&self, credential_id: CredentialId) {
        if let Some(bus) = &self.event_bus {
            let _ = bus.emit(CredentialEvent::Refreshed { credential_id });
        }
    }

    /// Fail-closed at the resolution tail when the configured state source is
    /// external and its resolution bridge is unwired. Called by every resolution
    /// entry point so the gate is structural, not per-call discipline (see
    /// [`gate_external_source`](Self::gate_external_source)).
    fn ensure_source_wired(&self) -> Result<(), ResolveError> {
        if self.external_source_unwired {
            return Err(ResolveError::ExternalSourceNotWired);
        }
        Ok(())
    }

    async fn active_refresh_retry_gate(
        &self,
        selector: &CredentialSelector,
    ) -> Result<Option<Box<RefreshNotAppliedContext>>, ResolveError> {
        match self.store.refresh_retry_snapshot(selector).await {
            Ok(snapshot) if matches!(snapshot.admission(), RefreshRetryAdmission::Open) => Ok(None),
            Ok(snapshot) => {
                let RefreshRetryAdmission::Blocked(block) = snapshot.admission() else {
                    return Ok(None);
                };
                context_from_block(block.clone())
                    .map(Some)
                    .map_err(|_| ResolveError::Store(CredentialPersistenceError::CorruptRecord))
            },
            Err(error) => Err(ResolveError::Store(error)),
        }
    }

    async fn ensure_refresh_admitted(
        &self,
        selector: &CredentialSelector,
    ) -> Result<(), ResolveError> {
        if let Some(context) = self.active_refresh_retry_gate(selector).await? {
            return Err(ResolveError::RefreshNotApplied {
                credential_id: selector.credential_id().to_string(),
                context,
            });
        }
        Ok(())
    }

    async fn load_and_verify<C>(
        &self,
        selector: &CredentialSelector,
    ) -> Result<StoredLiveCredential, ResolveError>
    where
        C: Credential,
    {
        let credential_id = selector.credential_id();
        let physical = self
            .store
            .get(selector)
            .await
            .map_err(ResolveError::Store)?;

        // Fail-closed on EVERY load path, not just the scoped one: a revoked
        // (tombstoned) row must never project to a handle. Checked before the
        // kind comparison so a tombstoned row of the wrong type maps to
        // existence-hiding `NotFound` instead of leaking a `KindMismatch`
        // oracle. This closes the tombstone half of the resurrection class for
        // `resolve` / `resolve_with_refresh` / `scheme_factory`; the owner half
        // is enforced structurally by the mandatory `CredentialSelector`.
        reject_tombstoned(&physical)?;
        let StoredCredential::Live(stored) = physical else {
            return Err(ResolveError::Store(CredentialPersistenceError::NotFound));
        };
        reject_persisted_reauth(&stored)?;

        let expected_kind = <C::State as CredentialState>::KIND;
        if stored.state_kind() != expected_kind {
            return Err(ResolveError::KindMismatch {
                credential_id: credential_id.to_string(),
                expected: expected_kind.to_string(),
                actual: stored.state_kind().to_owned(),
            });
        }

        Ok(stored)
    }

    fn deserialize<C>(
        &self,
        credential_id: CredentialId,
        stored: &StoredLiveCredential,
    ) -> Result<C::State, ResolveError>
    where
        C: Credential,
    {
        serde_json::from_slice(stored.data()).map_err(|e| ResolveError::Deserialize {
            credential_id: credential_id.to_string(),
            reason: e.to_string(),
        })
    }

    async fn perform_refresh<C>(
        &self,
        selector: &CredentialSelector,
        mut state: C::State,
        stored: StoredLiveCredential,
        ctx: &CredentialContext,
    ) -> RefreshDisposition<Result<CredentialHandle<C::Scheme>, ResolveError>>
    where
        C: Refreshable,
    {
        let credential_id = selector.credential_id();
        let credential_id_text = credential_id.to_string();
        let refresh_ctx = self.refresh_context(ctx);
        // This future already runs inside the coordinator's owned
        // provider/persistence task. Do not wrap it in a cancelling timeout:
        // dropping an HTTP future cannot prove the provider did not consume or
        // rotate the grant. The coordinator's caller-wait timeout instead
        // returns a non-retryable `RefreshOutcomePending` while this owned
        // future continues under heartbeat + L2 to an exact disposition.
        let outcome = <C as Refreshable>::refresh(
            &mut state,
            RefreshAttempt::new(&refresh_ctx, C::REFRESH_EXECUTION_MODE),
        )
        .await
        .into_kind();

        match outcome {
            RefreshReportKind::NotApplied(context) => {
                match persist_retry_gate(self.store.as_ref(), selector, stored, context).await {
                    RetryGateWrite::Applied(context) => {
                        RefreshDisposition::state_advanced(Err(ResolveError::RefreshNotApplied {
                            credential_id: credential_id_text,
                            context,
                        }))
                    },
                    RetryGateWrite::Superseded(error) => {
                        RefreshDisposition::no_state_change(Err(ResolveError::Store(error)))
                    },
                    RetryGateWrite::DefiniteFailure(error) => {
                        tracing::warn!(
                            credential_id = %credential_id,
                            ?error,
                            "durable refresh retry gate finalization failed; retaining claim"
                        );
                        RefreshDisposition::retry_unsafe(Err(
                            ResolveError::RefreshRetryGateFinalization {
                                credential_id: credential_id_text,
                            },
                        ))
                    },
                    RetryGateWrite::OutcomeUnknown => RefreshDisposition::outcome_unknown(Err(
                        ResolveError::Store(CredentialPersistenceError::OutcomeUnknown),
                    )),
                }
            },
            RefreshReportKind::OutcomeUnknown => {
                RefreshDisposition::outcome_unknown(Err(ResolveError::ProviderOutcomeUnknown {
                    credential_id: credential_id_text,
                }))
            },
            RefreshReportKind::ProviderRefreshed => {
                self.persist_refreshed_state::<C>(
                    selector,
                    state,
                    stored,
                    RefreshCommitPhase::ProviderConfirmed,
                )
                .await
            },
            RefreshReportKind::LocallyRefreshed => {
                self.persist_refreshed_state::<C>(
                    selector,
                    state,
                    stored,
                    RefreshCommitPhase::LocalOnly,
                )
                .await
            },
            RefreshReportKind::ReauthRequired { reason, phase } => {
                let phase_name = match phase {
                    RefreshReauthPhase::BeforeDispatch => "before_dispatch",
                    RefreshReauthPhase::ProviderConfirmed => "provider_confirmed",
                };
                match persist_reauth_required(self.store.as_ref(), selector, stored).await {
                    ReauthWrite::Applied => {
                        RefreshDisposition::state_advanced(Err(ResolveError::ReauthRequired {
                            credential_id: credential_id_text,
                            reason,
                        }))
                    },
                    ReauthWrite::Superseded(error) => {
                        tracing::warn!(
                            credential_id = %credential_id,
                            refresh.reauth_phase = phase_name,
                            ?error,
                            "failed to persist an exact reauthentication decision"
                        );
                        RefreshDisposition::no_state_change(Err(ResolveError::Store(error)))
                    },
                    ReauthWrite::DefiniteFailure(error)
                        if phase == RefreshReauthPhase::ProviderConfirmed =>
                    {
                        tracing::warn!(
                            credential_id = %credential_id,
                            ?error,
                            "provider-confirmed reauthentication decision was not durable; retaining claim"
                        );
                        RefreshDisposition::retry_unsafe(Err(
                            ResolveError::ReauthDecisionFinalization {
                                credential_id: credential_id_text,
                            },
                        ))
                    },
                    ReauthWrite::DefiniteFailure(error) => {
                        RefreshDisposition::no_state_change(Err(ResolveError::Store(error)))
                    },
                    ReauthWrite::OutcomeUnknown => RefreshDisposition::outcome_unknown(Err(
                        ResolveError::Store(CredentialPersistenceError::OutcomeUnknown),
                    )),
                }
            },
        }
    }

    async fn persist_refreshed_state<C>(
        &self,
        selector: &CredentialSelector,
        state: C::State,
        stored: StoredLiveCredential,
        phase: RefreshCommitPhase,
    ) -> RefreshDisposition<Result<CredentialHandle<C::Scheme>, ResolveError>>
    where
        C: Refreshable,
    {
        let credential_id = selector.credential_id();
        let credential_id_text = credential_id.to_string();
        let data =
            match crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&state)) {
                Ok(data) => data,
                Err(error) if phase == RefreshCommitPhase::ProviderConfirmed => {
                    return RefreshDisposition::retry_unsafe(Err(
                        ResolveError::PostProviderStateEncoding {
                            credential_id: credential_id_text,
                            reason: error.to_string(),
                        },
                    ));
                },
                Err(error) => {
                    return RefreshDisposition::no_state_change(Err(ResolveError::Refresh {
                        credential_id: credential_id_text,
                        reason: format!("local refresh state encoding failed: {error}"),
                    }));
                },
            };

        let now = chrono::Utc::now();
        let mut validated_metadata = stored.metadata().clone();
        validated_metadata.insert(
            LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
            serde_json::Value::String(now.to_rfc3339()),
        );
        let expected_version = stored.version();
        let replacement = CredentialReplacement::new(
            expected_version,
            data.into(),
            stored.state_kind().to_owned(),
            stored.state_version(),
            stored.name().map(str::to_owned),
            state.expires_at(),
            false,
            validated_metadata,
            CredentialMaterialTransition::advance(),
        );

        match self.store.replace(selector, replacement).await {
            Ok(_) => {
                self.emit_refreshed(credential_id);
                let scheme = C::project(&state);
                RefreshDisposition::state_advanced(Ok(
                    self.materialize_handle::<C>(selector, scheme)
                ))
            },
            Err(error @ CredentialPersistenceError::OutcomeUnknown)
                if phase == RefreshCommitPhase::ProviderConfirmed =>
            {
                RefreshDisposition::outcome_unknown(Err(ResolveError::PostProviderPersistence {
                    credential_id: credential_id_text,
                    source: error,
                }))
            },
            Err(error) if phase == RefreshCommitPhase::ProviderConfirmed => {
                tracing::warn!(
                    credential_id = %credential_id,
                    expected = %expected_version,
                    ?error,
                    "provider refresh succeeded but its CAS finalization failed"
                );
                RefreshDisposition::retry_unsafe(Err(ResolveError::PostProviderPersistence {
                    credential_id: credential_id_text,
                    source: error,
                }))
            },
            Err(CredentialPersistenceError::OutcomeUnknown) => {
                RefreshDisposition::replay_safe(Err(ResolveError::Store(
                    CredentialPersistenceError::OutcomeUnknown,
                )))
            },
            Err(error) => RefreshDisposition::no_state_change(Err(ResolveError::Store(error))),
        }
    }
}

/// FIX-1 regressions: a resolve/refresh must never project or resurrect a
/// revoked credential.
///
/// Hand-rolled test doubles of the crate's *own* ports (`CredentialPersistence`,
/// `RefreshClaimStore`, `RefreshTransport`) — no `nebula-storage` edge, so no
/// dependency cycle. The `ScriptedStore` simulates a `revoke` landing between
/// the resolver's load and the refresh write-back by tombstoning + version-
/// bumping the row on the first version-fenced replacement, which is exactly the race the
/// resurrection bug exploited.
#[cfg(test)]
mod refresh_revoke_race {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use chrono::Utc;
    use nebula_core::auth::{AuthPattern, EgressShape, RefreshStrategyKind};
    use nebula_schema::FieldValues;
    use nebula_storage_port::SecretBytes;
    use nebula_storage_port::store::{
        ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, RefreshClaimError,
        RefreshClaimStore, ReplicaId,
    };
    use nebula_storage_port::{
        CredentialAlreadyExistsKey, CredentialCommit, CredentialCreate, CredentialMaterialEpoch,
        CredentialOwner, CredentialTombstone, CredentialVersion, RefreshRetryAdmission,
        RefreshRetryBlock, RefreshRetryDelay, RefreshRetryGate, RefreshRetrySnapshot,
        RefreshRetryTransition, StoredCredentialHead, StoredTombstonedCredential,
    };
    use serde::{Deserialize, Serialize};
    use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

    use parking_lot::Mutex;
    use std::sync::atomic::AtomicBool;
    use tokio::sync::Notify;

    use super::*;
    use crate::credentials::{OAuth2Credential, OAuth2State};
    use crate::resolve::{RefreshPolicy, ResolveResult};
    use crate::runtime::refresh::RefreshCoordConfig;
    use crate::runtime::refresh::transport::{
        RefreshTransport, RefreshTransportError, TokenPostRequest, TokenPostResponse,
    };
    use crate::{CredentialMetadata, CredentialPolicy, RefreshStrategy, RevokeStrategy};

    // ── Test doubles of the crate's own ports ──────────────────────────

    /// Single-caller L2 claim repo. Typed K2 selectors always exercise the L2
    /// path, so this fixture grants every claim and accepts its lifecycle.
    struct StubClaimRepo;

    #[async_trait::async_trait]
    impl RefreshClaimStore for StubClaimRepo {
        async fn try_claim(
            &self,
            credential_id: &CredentialId,
            _holder: &ReplicaId,
            ttl: Duration,
        ) -> Result<ClaimAttempt, RefreshClaimError> {
            let acquired_at = Utc::now();
            let ttl =
                chrono::Duration::from_std(ttl).expect("test refresh-claim TTL is representable");
            Ok(ClaimAttempt::Acquired(
                nebula_storage_port::store::RefreshClaim {
                    credential_id: credential_id.to_owned(),
                    token: ClaimToken {
                        claim_id: "00000000-0000-0000-0000-000000000001"
                            .parse()
                            .expect("test claim id is a UUID"),
                        generation: 1,
                    },
                    acquired_at,
                    expires_at: acquired_at + ttl,
                },
            ))
        }
        async fn heartbeat(
            &self,
            _token: &ClaimToken,
            _ttl: Duration,
        ) -> Result<(), HeartbeatError> {
            Ok(())
        }
        async fn release(&self, _token: ClaimToken) -> Result<(), RefreshClaimError> {
            Ok(())
        }
        async fn mark_sentinel(&self, _token: &ClaimToken) -> Result<(), RefreshClaimError> {
            Ok(())
        }
        async fn reclaim_stuck(&self) -> Result<Vec<ExpiredClaim>, RefreshClaimError> {
            Ok(Vec::new())
        }
        async fn count_sentinel_events_in_window(
            &self,
            _credential_id: &CredentialId,
            _window: Duration,
        ) -> Result<u32, RefreshClaimError> {
            Ok(0)
        }
    }

    /// Stateful L2 fixture used by replay-safety regressions. Unlike
    /// [`StubClaimRepo`], it keeps an acquired claim active until an explicit
    /// release, so a retained sentinel claim can prove that a second request
    /// reaches L2 but cannot repeat provider work.
    #[derive(Default)]
    struct StatefulClaimRepo {
        active: AtomicBool,
        try_claim_count: AtomicUsize,
        release_count: AtomicUsize,
        try_claim_seen: Notify,
        release_seen: Notify,
    }

    impl StatefulClaimRepo {
        async fn wait_for_try_claim_count(&self, target: usize) {
            while self.try_claim_count.load(Ordering::SeqCst) < target {
                self.try_claim_seen.notified().await;
            }
        }

        async fn wait_for_release_count(&self, target: usize) {
            while self.release_count.load(Ordering::SeqCst) < target {
                self.release_seen.notified().await;
            }
        }
    }

    #[async_trait::async_trait]
    impl RefreshClaimStore for StatefulClaimRepo {
        async fn try_claim(
            &self,
            credential_id: &CredentialId,
            _holder: &ReplicaId,
            ttl: Duration,
        ) -> Result<ClaimAttempt, RefreshClaimError> {
            self.try_claim_count.fetch_add(1, Ordering::SeqCst);
            self.try_claim_seen.notify_one();
            let acquired_at = Utc::now();
            let ttl =
                chrono::Duration::from_std(ttl).expect("test refresh-claim TTL is representable");
            if self
                .active
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return Ok(ClaimAttempt::Contended {
                    existing_expires_at: acquired_at + ttl,
                });
            }

            Ok(ClaimAttempt::Acquired(
                nebula_storage_port::store::RefreshClaim {
                    credential_id: credential_id.to_owned(),
                    token: ClaimToken {
                        claim_id: "00000000-0000-0000-0000-000000000002"
                            .parse()
                            .expect("test claim id is a UUID"),
                        generation: 1,
                    },
                    acquired_at,
                    expires_at: acquired_at + ttl,
                },
            ))
        }

        async fn heartbeat(
            &self,
            _token: &ClaimToken,
            _ttl: Duration,
        ) -> Result<(), HeartbeatError> {
            if self.active.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(HeartbeatError::ClaimLost)
            }
        }

        async fn release(&self, _token: ClaimToken) -> Result<(), RefreshClaimError> {
            self.active.store(false, Ordering::SeqCst);
            self.release_count.fetch_add(1, Ordering::SeqCst);
            self.release_seen.notify_one();
            Ok(())
        }

        async fn mark_sentinel(&self, _token: &ClaimToken) -> Result<(), RefreshClaimError> {
            if self.active.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(RefreshClaimError::InvalidState)
            }
        }

        async fn reclaim_stuck(&self) -> Result<Vec<ExpiredClaim>, RefreshClaimError> {
            Ok(Vec::new())
        }

        async fn count_sentinel_events_in_window(
            &self,
            _credential_id: &CredentialId,
            _window: Duration,
        ) -> Result<u32, RefreshClaimError> {
            Ok(0)
        }
    }

    /// No-op transport for typed credentials whose refresh performs no
    /// provider I/O.
    struct StubTransport;

    impl RefreshTransport for StubTransport {
        fn post_token<'a>(
            &'a self,
            _request: TokenPostRequest,
        ) -> Pin<
            Box<dyn Future<Output = Result<TokenPostResponse, RefreshTransportError>> + Send + 'a>,
        > {
            Box::pin(async {
                unreachable!("default-feature refresh performs no OAuth2 token POST")
            })
        }
    }

    struct UnusedPendingStore;

    impl crate::DynPendingStateStore for UnusedPendingStore {
        fn put_serialized<'a>(
            &'a self,
            _credential_kind: &'a str,
            _owner_id: &'a str,
            _session_id: &'a str,
            _data: Zeroizing<Vec<u8>>,
            _expires_in: Duration,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<crate::PendingToken, crate::PendingStoreError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Err(crate::PendingStoreError::NotFound) })
        }

        fn get_serialized<'a>(
            &'a self,
            _token: &'a crate::PendingToken,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<Zeroizing<Vec<u8>>, crate::PendingStoreError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Err(crate::PendingStoreError::NotFound) })
        }

        fn get_bound_serialized<'a>(
            &'a self,
            _credential_kind: &'a str,
            _token: &'a crate::PendingToken,
            _owner_id: &'a str,
            _session_id: &'a str,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<Zeroizing<Vec<u8>>, crate::PendingStoreError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Err(crate::PendingStoreError::NotFound) })
        }

        fn consume_serialized<'a>(
            &'a self,
            _credential_kind: &'a str,
            _token: &'a crate::PendingToken,
            _owner_id: &'a str,
            _session_id: &'a str,
        ) -> Pin<
            Box<
                dyn Future<Output = Result<Zeroizing<Vec<u8>>, crate::PendingStoreError>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Err(crate::PendingStoreError::NotFound) })
        }

        fn delete<'a>(
            &'a self,
            _token: &'a crate::PendingToken,
        ) -> Pin<Box<dyn Future<Output = Result<(), crate::PendingStoreError>> + Send + 'a>>
        {
            Box::pin(async { Ok(()) })
        }
    }

    #[derive(Clone, Copy)]
    enum OAuthTransportResult {
        AckLost,
        InvalidGrant,
        InvalidClient401,
        Other429,
        ServerError503,
        MalformedSuccess,
        Success,
    }

    /// Deterministic OAuth2 transport used to distinguish an ambiguous
    /// post-dispatch failure from an exact RFC 6749 provider rejection.
    struct ScriptedOAuthTransport {
        result: OAuthTransportResult,
        calls: AtomicUsize,
    }

    impl ScriptedOAuthTransport {
        fn new(result: OAuthTransportResult) -> Self {
            Self {
                result,
                calls: AtomicUsize::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl RefreshTransport for ScriptedOAuthTransport {
        fn post_token<'a>(
            &'a self,
            _request: TokenPostRequest,
        ) -> Pin<
            Box<dyn Future<Output = Result<TokenPostResponse, RefreshTransportError>> + Send + 'a>,
        > {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let result = self.result;
            Box::pin(async move {
                match result {
                    OAuthTransportResult::AckLost => Err(RefreshTransportError::ReadBody),
                    OAuthTransportResult::InvalidGrant => Ok(TokenPostResponse::try_new(
                        400,
                        SecretBytes::new(
                            br#"{"error":"invalid_grant","error_description":"grant revoked"}"#
                                .to_vec(),
                        ),
                    )
                    .expect("scripted response is bounded")),
                    OAuthTransportResult::InvalidClient401 => Ok(
                        TokenPostResponse::try_new(
                            401,
                            SecretBytes::new(br#"{"error":"invalid_client"}"#.to_vec()),
                        )
                        .expect("scripted response is bounded"),
                    ),
                    OAuthTransportResult::Other429 => Ok(
                        TokenPostResponse::try_new(
                            429,
                            SecretBytes::new(br#"{"error":"provider_extension"}"#.to_vec()),
                        )
                        .expect("scripted response is bounded"),
                    ),
                    OAuthTransportResult::ServerError503 => Ok(
                        TokenPostResponse::try_new(
                            503,
                            SecretBytes::new(br#"{"error":"server_error"}"#.to_vec()),
                        )
                        .expect("scripted response is bounded"),
                    ),
                    OAuthTransportResult::MalformedSuccess => Ok(
                        TokenPostResponse::try_new(
                            200,
                            SecretBytes::new(br#"{"token_type":"Bearer"}"#.to_vec()),
                        )
                        .expect("scripted response is bounded"),
                    ),
                    OAuthTransportResult::Success => Ok(
                        TokenPostResponse::try_new(
                            200,
                            SecretBytes::new(
                                br#"{"access_token":"new-access-token","token_type":"Bearer","refresh_token":"new-refresh-token","expires_in":3600,"scope":"read"}"#
                                    .to_vec(),
                            ),
                        )
                        .expect("scripted response is bounded"),
                    ),
                }
            })
        }
    }

    /// In-memory single-row store. When `revoke_on_first_replace` is set, the
    /// first replacement structurally tombstones + version-bumps the row —
    /// modelling a `revoke` that raced the refresh between load and write-back.
    #[derive(Debug)]
    struct ScriptedStore {
        owner: CredentialOwner,
        row: Mutex<StoredCredential>,
        revoke_on_first_replace: bool,
        display_on_first_replace: bool,
        display_on_every_replace: bool,
        display_on_second_get: bool,
        material_on_second_get: bool,
        replace_error: Option<CredentialPersistenceError>,
        replacements: Mutex<u32>,
        gets: Mutex<u32>,
    }

    impl ScriptedStore {
        fn new(row: StoredCredential, revoke_on_first_replace: bool) -> Self {
            Self::with_owner(row, revoke_on_first_replace, test_owner())
        }

        fn with_owner(
            row: StoredCredential,
            revoke_on_first_replace: bool,
            owner: CredentialOwner,
        ) -> Self {
            Self {
                owner,
                row: Mutex::new(row),
                revoke_on_first_replace,
                display_on_first_replace: false,
                display_on_every_replace: false,
                display_on_second_get: false,
                material_on_second_get: false,
                replace_error: None,
                replacements: Mutex::new(0),
                gets: Mutex::new(0),
            }
        }

        fn failing_replace(
            row: StoredCredential,
            replace_error: CredentialPersistenceError,
        ) -> Self {
            Self {
                owner: test_owner(),
                row: Mutex::new(row),
                revoke_on_first_replace: false,
                display_on_first_replace: false,
                display_on_every_replace: false,
                display_on_second_get: false,
                material_on_second_get: false,
                replace_error: Some(replace_error),
                replacements: Mutex::new(0),
                gets: Mutex::new(0),
            }
        }

        fn racing_display_replace(row: StoredCredential) -> Self {
            Self {
                owner: test_owner(),
                row: Mutex::new(row),
                revoke_on_first_replace: false,
                display_on_first_replace: true,
                display_on_every_replace: false,
                display_on_second_get: false,
                material_on_second_get: false,
                replace_error: None,
                replacements: Mutex::new(0),
                gets: Mutex::new(0),
            }
        }

        fn racing_display_read(row: StoredCredential) -> Self {
            Self {
                owner: test_owner(),
                row: Mutex::new(row),
                revoke_on_first_replace: false,
                display_on_first_replace: false,
                display_on_every_replace: false,
                display_on_second_get: true,
                material_on_second_get: false,
                replace_error: None,
                replacements: Mutex::new(0),
                gets: Mutex::new(0),
            }
        }

        fn racing_identical_material_read(row: StoredCredential) -> Self {
            Self {
                owner: test_owner(),
                row: Mutex::new(row),
                revoke_on_first_replace: false,
                display_on_first_replace: false,
                display_on_every_replace: false,
                display_on_second_get: false,
                material_on_second_get: true,
                replace_error: None,
                replacements: Mutex::new(0),
                gets: Mutex::new(0),
            }
        }

        fn continuous_display_churn(row: StoredCredential) -> Self {
            Self {
                owner: test_owner(),
                row: Mutex::new(row),
                revoke_on_first_replace: false,
                display_on_first_replace: false,
                display_on_every_replace: true,
                display_on_second_get: false,
                material_on_second_get: false,
                replace_error: None,
                replacements: Mutex::new(0),
                gets: Mutex::new(0),
            }
        }

        fn snapshot(&self) -> StoredCredential {
            self.row.lock().clone()
        }

        fn replacement_count(&self) -> u32 {
            *self.replacements.lock()
        }

        fn get_count(&self) -> u32 {
            *self.gets.lock()
        }

        fn evaluate_retry_admission(
            live: &StoredLiveCredential,
            now: chrono::DateTime<Utc>,
        ) -> Result<RefreshRetryAdmission, CredentialPersistenceError> {
            let Some(gate) = live.refresh_retry_gate() else {
                return Ok(RefreshRetryAdmission::Open);
            };
            match gate {
                RefreshRetryGate::Never { evidence } => {
                    Ok(RefreshRetryAdmission::Blocked(RefreshRetryBlock::Never {
                        evidence: evidence.clone(),
                    }))
                },
                RefreshRetryGate::NotBefore {
                    not_before,
                    evidence,
                } => {
                    let remaining = *not_before - now;
                    if remaining <= chrono::Duration::zero() {
                        return Ok(RefreshRetryAdmission::Open);
                    }
                    let remaining = remaining
                        .to_std()
                        .map_err(|_| CredentialPersistenceError::CorruptRecord)?;
                    let remaining = RefreshRetryDelay::new(remaining)
                        .map_err(|_| CredentialPersistenceError::CorruptRecord)?;
                    Ok(RefreshRetryAdmission::Blocked(RefreshRetryBlock::After {
                        remaining,
                        evidence: evidence.clone(),
                    }))
                },
            }
        }

        // Synchronous core so the trait futures hold no lock across an `.await`.
        fn replace_sync(
            &self,
            selector: &CredentialSelector,
            replacement: CredentialReplacement,
        ) -> Result<CredentialCommit, CredentialPersistenceError> {
            let mut row = self.row.lock();
            if selector.owner() != &self.owner || row.credential_id() != selector.credential_id() {
                return Err(CredentialPersistenceError::NotFound);
            }
            let StoredCredential::Live(current) = row.clone() else {
                return Err(CredentialPersistenceError::NotFound);
            };
            let first = {
                let mut replacements = self.replacements.lock();
                *replacements += 1;
                *replacements == 1
            };
            let expected = replacement.expected_version();

            if let Some(error) = self.replace_error {
                return Err(error);
            }

            if self.revoke_on_first_replace && first {
                // A `revoke` lands between the resolver's load and this CAS:
                // consume the next version, clear every live-only field by
                // construction, then reject the stale replacement.
                let actual = current.version().next_tombstone()?;
                let now = Utc::now();
                *row = StoredTombstonedCredential::new(
                    current.credential_id(),
                    current.credential_key().to_owned(),
                    current.state_kind().to_owned(),
                    current.state_version(),
                    actual,
                    current.created_at(),
                    now,
                    now,
                )
                .into();
                return Err(CredentialPersistenceError::VersionConflict { expected, actual });
            }

            if (self.display_on_first_replace && first) || self.display_on_every_replace {
                let actual = current.version().next_live()?;
                let now = Utc::now();
                let mut metadata = current.metadata().clone();
                metadata.insert(
                    "display".to_owned(),
                    serde_json::json!({"description": "concurrent display edit"}),
                );
                let concurrent = StoredLiveCredential::new(
                    current.credential_id(),
                    Some("renamed-concurrently".to_owned()),
                    current.credential_key().to_owned(),
                    current.data().clone(),
                    current.state_kind().to_owned(),
                    current.state_version(),
                    actual,
                    current.material_epoch(),
                    current.created_at(),
                    now,
                    current.expires_at(),
                    current.reauth_required(),
                    metadata,
                    current.refresh_retry_gate().cloned(),
                )?;
                *row = concurrent.into();
                return Err(CredentialPersistenceError::VersionConflict { expected, actual });
            }

            if expected != current.version() {
                return Err(CredentialPersistenceError::VersionConflict {
                    expected,
                    actual: current.version(),
                });
            }

            let version = current.version().next_live()?;
            let updated_at = Utc::now();
            let (material_epoch, refresh_retry_gate) = match replacement.material_transition() {
                CredentialMaterialTransition::Advance => (current.material_epoch().next()?, None),
                CredentialMaterialTransition::Preserve { refresh_retry } => {
                    let gate = match refresh_retry {
                        RefreshRetryTransition::Preserve => current.refresh_retry_gate().cloned(),
                        RefreshRetryTransition::Clear => None,
                        RefreshRetryTransition::SetNever { evidence } => {
                            Some(RefreshRetryGate::Never {
                                evidence: evidence.clone(),
                            })
                        },
                        RefreshRetryTransition::SetAfter { delay, evidence } => {
                            let delay = chrono::Duration::from_std(delay.get())
                                .map_err(|_| CredentialPersistenceError::CorruptRecord)?;
                            Some(RefreshRetryGate::NotBefore {
                                not_before: updated_at + delay,
                                evidence: evidence.clone(),
                            })
                        },
                    };
                    (current.material_epoch(), gate)
                },
            };
            let committed = StoredLiveCredential::new(
                current.credential_id(),
                replacement.name().map(str::to_owned),
                current.credential_key().to_owned(),
                replacement.data().clone(),
                replacement.state_kind().to_owned(),
                replacement.state_version(),
                version,
                material_epoch,
                current.created_at(),
                updated_at,
                replacement.expires_at(),
                replacement.reauth_required(),
                replacement.metadata().clone(),
                refresh_retry_gate,
            )?;
            *row = committed.into();
            CredentialCommit::live(
                current.credential_id(),
                version,
                current.created_at(),
                updated_at,
            )
        }

        fn tombstone_sync(
            &self,
            selector: &CredentialSelector,
            tombstone: CredentialTombstone,
        ) -> Result<CredentialCommit, CredentialPersistenceError> {
            let mut row = self.row.lock();
            if selector.owner() != &self.owner || row.credential_id() != selector.credential_id() {
                return Err(CredentialPersistenceError::NotFound);
            }
            let StoredCredential::Live(current) = row.clone() else {
                return Err(CredentialPersistenceError::NotFound);
            };
            if tombstone.expected_version() != current.version() {
                return Err(CredentialPersistenceError::VersionConflict {
                    expected: tombstone.expected_version(),
                    actual: current.version(),
                });
            }
            let version = current.version().next_tombstone()?;
            let now = Utc::now();
            *row = StoredTombstonedCredential::new(
                current.credential_id(),
                current.credential_key().to_owned(),
                current.state_kind().to_owned(),
                current.state_version(),
                version,
                current.created_at(),
                now,
                now,
            )
            .into();
            Ok(CredentialCommit::tombstoned(
                current.credential_id(),
                version,
                current.created_at(),
                now,
                now,
            ))
        }
    }

    #[async_trait::async_trait]
    impl CredentialPersistence for ScriptedStore {
        // No `.await` in any method body, so the (`!Send`) `parking_lot` guard
        // never crosses an await point — the returned futures stay `Send`.
        async fn get(
            &self,
            selector: &CredentialSelector,
        ) -> Result<StoredCredential, CredentialPersistenceError> {
            let get_number = {
                let mut gets = self.gets.lock();
                *gets += 1;
                *gets
            };
            let mut row = self.row.lock();
            if get_number == 2 && (self.display_on_second_get || self.material_on_second_get) {
                let StoredCredential::Live(current) = row.clone() else {
                    return Err(CredentialPersistenceError::NotFound);
                };
                let version = current.version().next_live()?;
                let material_epoch = if self.material_on_second_get {
                    current.material_epoch().next()?
                } else {
                    current.material_epoch()
                };
                let mut metadata = current.metadata().clone();
                let name = if self.display_on_second_get {
                    metadata.insert(
                        "display".to_owned(),
                        serde_json::json!({"description": "pre-dispatch display edit"}),
                    );
                    Some("renamed-before-dispatch".to_owned())
                } else {
                    current.name().map(str::to_owned)
                };
                let raced = StoredLiveCredential::new(
                    current.credential_id(),
                    name,
                    current.credential_key().to_owned(),
                    // Intentionally byte-identical in the material-race case:
                    // epoch, not byte comparison, establishes new authority.
                    current.data().clone(),
                    current.state_kind().to_owned(),
                    current.state_version(),
                    version,
                    material_epoch,
                    current.created_at(),
                    Utc::now(),
                    current.expires_at(),
                    current.reauth_required(),
                    metadata,
                    if self.material_on_second_get {
                        None
                    } else {
                        current.refresh_retry_gate().cloned()
                    },
                )?;
                *row = raced.into();
            }
            let row = row.clone();
            if selector.owner() == &self.owner && row.credential_id() == selector.credential_id() {
                Ok(row)
            } else {
                Err(CredentialPersistenceError::NotFound)
            }
        }

        async fn get_head(
            &self,
            selector: &CredentialSelector,
        ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
            match self.get(selector).await? {
                StoredCredential::Live(stored) => Ok(StoredCredentialHead::from(&stored)),
                StoredCredential::Tombstoned(_) => Err(CredentialPersistenceError::NotFound),
            }
        }

        async fn refresh_retry_snapshot(
            &self,
            selector: &CredentialSelector,
        ) -> Result<RefreshRetrySnapshot, CredentialPersistenceError> {
            let row = self.row.lock();
            if selector.owner() != &self.owner || row.credential_id() != selector.credential_id() {
                return Err(CredentialPersistenceError::NotFound);
            }
            let StoredCredential::Live(live) = &*row else {
                return Err(CredentialPersistenceError::NotFound);
            };
            let admission = Self::evaluate_retry_admission(live, Utc::now())?;
            Ok(RefreshRetrySnapshot::new(
                live.version(),
                live.material_epoch(),
                live.reauth_required(),
                admission,
            ))
        }

        async fn create(
            &self,
            selector: &CredentialSelector,
            _create: CredentialCreate,
        ) -> Result<CredentialCommit, CredentialPersistenceError> {
            if selector.owner() != &self.owner {
                return Err(CredentialPersistenceError::NotFound);
            }
            Err(CredentialPersistenceError::AlreadyExists {
                key: CredentialAlreadyExistsKey::Id,
            })
        }

        async fn replace(
            &self,
            selector: &CredentialSelector,
            replacement: CredentialReplacement,
        ) -> Result<CredentialCommit, CredentialPersistenceError> {
            self.replace_sync(selector, replacement)
        }

        async fn tombstone(
            &self,
            selector: &CredentialSelector,
            tombstone: CredentialTombstone,
        ) -> Result<CredentialCommit, CredentialPersistenceError> {
            self.tombstone_sync(selector, tombstone)
        }

        async fn list(
            &self,
            owner: &CredentialOwner,
            state_kind: Option<&str>,
        ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
            let row = self.row.lock();
            let listed = owner == &self.owner
                && row
                    .as_live()
                    .is_some_and(|live| state_kind.is_none_or(|kind| live.state_kind() == kind));
            Ok(if listed {
                vec![row.credential_id()]
            } else {
                Vec::new()
            })
        }

        async fn list_heads(
            &self,
            owner: &CredentialOwner,
            state_kind: Option<&str>,
        ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
            let row = self.row.lock();
            let Some(live) = row.as_live() else {
                return Ok(Vec::new());
            };
            if owner != &self.owner || state_kind.is_some_and(|kind| live.state_kind() != kind) {
                return Ok(Vec::new());
            }
            Ok(vec![StoredCredentialHead::from(live)])
        }

        async fn exists(
            &self,
            selector: &CredentialSelector,
        ) -> Result<bool, CredentialPersistenceError> {
            let row = self.row.lock();
            Ok(selector.owner() == &self.owner
                && row.credential_id() == selector.credential_id()
                && matches!(&*row, StoredCredential::Live(_)))
        }
    }

    // ── A faithful refreshable credential ──────────────────────────────

    /// Active family declaring an engine-drivable `RefreshToken` class, so the
    /// F3 containment `debug_assert` in `resolve_with_refresh` passes honestly
    /// (rather than via the `Lease` exemption back door).
    struct TestActiveFamily;

    impl SchemeFamily for TestActiveFamily {
        const EGRESS: &'static [EgressShape] = &[EgressShape::InlineSecret];
        fn refresh_classes() -> &'static [RefreshStrategyKind] {
            &[RefreshStrategyKind::RefreshToken]
        }
        fn pattern() -> AuthPattern {
            AuthPattern::OAuth2
        }
    }

    #[derive(Debug)]
    struct TestScheme;

    impl AuthScheme for TestScheme {
        type Family = TestActiveFamily;
        fn pattern() -> AuthPattern {
            AuthPattern::OAuth2
        }
    }

    #[derive(Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
    struct TestState {
        token: String,
    }

    impl CredentialState for TestState {
        const KIND: &'static str = "test_refreshable_state";
        const VERSION: u32 = 1;
    }

    struct TestCred;

    static CAS_PROVIDER_CALLS: AtomicUsize = AtomicUsize::new(0);
    static NAME_PROVIDER_CALLS: AtomicUsize = AtomicUsize::new(0);
    static UNAVAILABLE_PROVIDER_CALLS: AtomicUsize = AtomicUsize::new(0);
    static UNKNOWN_PROVIDER_CALLS: AtomicUsize = AtomicUsize::new(0);
    static REAUTH_PROVIDER_CALLS: AtomicUsize = AtomicUsize::new(0);
    static AFTER_PROVIDER_CALLS: AtomicUsize = AtomicUsize::new(0);
    static MODE_MISMATCH_REFRESH_CALLS: AtomicUsize = AtomicUsize::new(0);
    static SAME_KEY_TYPED_REFRESH_CALLS: AtomicUsize = AtomicUsize::new(0);
    static CANCELLATION_REFRESH_STARTED: AtomicBool = AtomicBool::new(false);
    static CANCELLATION_REFRESH_STARTED_NOTIFY: Notify = Notify::const_new();
    static CANCELLATION_REFRESH_RELEASE: Notify = Notify::const_new();

    impl Credential for TestCred {
        type Properties = ();
        type Scheme = TestScheme;
        type State = TestState;

        const KEY: &'static str = "test.refreshable";

        fn metadata() -> CredentialMetadata {
            CredentialMetadata::builder()
                .key(nebula_core::credential_key!("test.refreshable"))
                .name("TestCred")
                .description("refreshable test credential for resolver regressions")
                .schema(crate::schema_of::<Self::Properties>())
                .pattern(AuthPattern::OAuth2)
                .build()
                .expect("TestCred metadata is valid")
        }

        fn project(_state: &TestState) -> TestScheme {
            TestScheme
        }

        async fn resolve(
            _values: &FieldValues,
            _ctx: &CredentialContext,
        ) -> Result<ResolveResult<TestState, ()>, CredentialError> {
            Ok(ResolveResult::Complete(TestState {
                token: "live".to_owned(),
            }))
        }
    }

    impl Refreshable for TestCred {
        const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

        async fn refresh(
            state: &mut TestState,
            attempt: RefreshAttempt<'_>,
        ) -> crate::RefreshReport {
            let original_token = state.token.clone();
            let completed = attempt
                .dispatch(|| async move {
                    match original_token.as_str() {
                        "counted-cas" => {
                            CAS_PROVIDER_CALLS.fetch_add(1, Ordering::SeqCst);
                        },
                        "counted-name" => {
                            NAME_PROVIDER_CALLS.fetch_add(1, Ordering::SeqCst);
                        },
                        "counted-unavailable" => {
                            UNAVAILABLE_PROVIDER_CALLS.fetch_add(1, Ordering::SeqCst);
                        },
                        "counted-unknown" => {
                            UNKNOWN_PROVIDER_CALLS.fetch_add(1, Ordering::SeqCst);
                        },
                        "reject" => {
                            REAUTH_PROVIDER_CALLS.fetch_add(1, Ordering::SeqCst);
                        },
                        "after" => {
                            AFTER_PROVIDER_CALLS.fetch_add(1, Ordering::SeqCst);
                        },
                        _ => {},
                    }
                    Ok::<u8, std::convert::Infallible>(match original_token.as_str() {
                        "reject" => 1,
                        "after" => 2,
                        _ => 0,
                    })
                })
                .await;
            let Ok(completed) = completed else {
                panic!("infallible test dispatch must complete");
            };
            let (response, proof) = completed.into_parts();
            match response {
                1 => return proof.provider_rejected(),
                2 => {
                    let delay = crate::RetryDelay::new(Duration::from_secs(1))
                        .expect("one second is a valid retry delay");
                    return proof.confirmed_not_applied(crate::RefreshFailureSpec::new(
                        crate::RefreshErrorKind::ProviderUnavailable,
                        crate::RetryAdvice::After(delay),
                    ));
                },
                _ => {},
            }
            state.token = "refreshed".to_owned();
            proof.refreshed()
        }
    }

    impl CredentialLifecycle for TestCred {
        fn policy(_state: &TestState) -> CredentialPolicy {
            CredentialPolicy {
                // Already expired → `decide_refresh` routes to `Refresh` (the
                // family declares `RefreshToken`, so it is auto-renewable).
                expires_at: Some(Utc::now() - chrono::Duration::minutes(5)),
                lease: None,
                refresh: RefreshStrategy::RefreshToken,
                revoke: RevokeStrategy::HandleBased,
            }
        }
    }

    /// Deliberately invalid implementation: it declares the default provider
    /// execution mode but attempts to complete through the providerless path.
    /// The linear attempt must reject this before any transport call.
    struct ProviderModeLocalCompletionCred;

    impl Credential for ProviderModeLocalCompletionCred {
        type Properties = ();
        type Scheme = TestScheme;
        type State = TestState;

        const KEY: &'static str = TestCred::KEY;

        fn metadata() -> CredentialMetadata {
            TestCred::metadata()
        }

        fn project(_state: &TestState) -> TestScheme {
            TestScheme
        }

        async fn resolve(
            _values: &FieldValues,
            _ctx: &CredentialContext,
        ) -> Result<ResolveResult<TestState, ()>, CredentialError> {
            Ok(ResolveResult::Complete(TestState {
                token: "live".to_owned(),
            }))
        }
    }

    impl Refreshable for ProviderModeLocalCompletionCred {
        const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

        async fn refresh(
            state: &mut TestState,
            attempt: RefreshAttempt<'_>,
        ) -> crate::RefreshReport {
            MODE_MISMATCH_REFRESH_CALLS.fetch_add(1, Ordering::SeqCst);
            state.token = "must-not-commit".to_owned();
            attempt.local_refresh_completed()
        }
    }

    impl CredentialLifecycle for ProviderModeLocalCompletionCred {
        fn policy(state: &TestState) -> CredentialPolicy {
            TestCred::policy(state)
        }
    }

    /// Regression credential deliberately sharing OAuth2's registry key while
    /// carrying a different state type. A generic resolver must dispatch its
    /// own `Refreshable` implementation, never reinterpret state by key.
    struct SameKeyNonOAuthCred;

    impl Credential for SameKeyNonOAuthCred {
        type Properties = ();
        type Scheme = TestScheme;
        type State = TestState;

        const KEY: &'static str = "oauth2";

        fn metadata() -> CredentialMetadata {
            CredentialMetadata::new(
                nebula_core::credential_key!("oauth2"),
                "Same-key typed test credential",
                "proves refresh dispatch follows the Rust type rather than its registry key",
                crate::schema_of::<Self::Properties>(),
                AuthPattern::OAuth2,
            )
        }

        fn project(_state: &TestState) -> TestScheme {
            TestScheme
        }

        async fn resolve(
            _values: &FieldValues,
            _ctx: &CredentialContext,
        ) -> Result<ResolveResult<TestState, ()>, CredentialError> {
            Ok(ResolveResult::Complete(TestState {
                token: "same-key-live".to_owned(),
            }))
        }
    }

    impl Refreshable for SameKeyNonOAuthCred {
        async fn refresh(
            state: &mut TestState,
            attempt: RefreshAttempt<'_>,
        ) -> crate::RefreshReport {
            if attempt.context().refresh_transport().is_none() {
                return attempt.outcome_unknown();
            }
            let completed = attempt
                .dispatch(|| async {
                    SAME_KEY_TYPED_REFRESH_CALLS.fetch_add(1, Ordering::SeqCst);
                    Ok::<(), std::convert::Infallible>(())
                })
                .await;
            let Ok(completed) = completed else {
                panic!("infallible test dispatch must complete");
            };
            let ((), proof) = completed.into_parts();
            state.token = "same-key-typed-refresh".to_owned();
            proof.refreshed()
        }
    }

    impl CredentialLifecycle for SameKeyNonOAuthCred {
        fn policy(_state: &TestState) -> CredentialPolicy {
            CredentialPolicy {
                expires_at: Some(Utc::now() - chrono::Duration::minutes(5)),
                lease: None,
                refresh: RefreshStrategy::RefreshToken,
                revoke: RevokeStrategy::HandleBased,
            }
        }
    }

    /// Integration whose provider future explicitly observes the context
    /// cancellation token. It catches accidental reuse of request
    /// cancellation inside the owned K2 critical section.
    struct CancellationAwareCred;

    impl Credential for CancellationAwareCred {
        type Properties = ();
        type Scheme = TestScheme;
        type State = TestState;

        const KEY: &'static str = "test.cancellation_aware";

        fn metadata() -> CredentialMetadata {
            CredentialMetadata::new(
                nebula_core::credential_key!("test.cancellation_aware"),
                "Cancellation-aware test credential",
                "proves K2 refresh is detached from request cancellation",
                crate::schema_of::<Self::Properties>(),
                AuthPattern::OAuth2,
            )
        }

        fn project(_state: &TestState) -> TestScheme {
            TestScheme
        }

        async fn resolve(
            _values: &FieldValues,
            _ctx: &CredentialContext,
        ) -> Result<ResolveResult<TestState, ()>, CredentialError> {
            Ok(ResolveResult::Complete(TestState {
                token: "cancellation-aware-live".to_owned(),
            }))
        }
    }

    impl Refreshable for CancellationAwareCred {
        async fn refresh(
            state: &mut TestState,
            attempt: RefreshAttempt<'_>,
        ) -> crate::RefreshReport {
            if attempt.context().refresh_transport().is_none() {
                return attempt.outcome_unknown();
            }
            CANCELLATION_REFRESH_STARTED.store(true, Ordering::SeqCst);
            CANCELLATION_REFRESH_STARTED_NOTIFY.notify_waiters();

            let cancel = attempt.context().cancel_token().clone();
            let completed = match attempt
                .dispatch(|| async move {
                    tokio::select! {
                        biased;
                        () = cancel.cancelled() => Err(()),
                        () = CANCELLATION_REFRESH_RELEASE.notified() => Ok(()),
                    }
                })
                .await
            {
                Ok(completed) => completed,
                Err(unknown) => return unknown.into_report(),
            };
            let ((), proof) = completed.into_parts();
            state.token = "cancellation-aware-refreshed".to_owned();
            proof.refreshed()
        }
    }

    impl CredentialLifecycle for CancellationAwareCred {
        fn policy(_state: &TestState) -> CredentialPolicy {
            CredentialPolicy {
                expires_at: Some(Utc::now() - chrono::Duration::minutes(5)),
                lease: None,
                refresh: RefreshStrategy::RefreshToken,
                revoke: RevokeStrategy::HandleBased,
            }
        }
    }

    #[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
    struct PostProviderEncodingState {
        fail_serialization: bool,
    }

    impl Serialize for PostProviderEncodingState {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            if self.fail_serialization {
                return Err(serde::ser::Error::custom(
                    "intentional post-provider encoding failure",
                ));
            }

            use serde::ser::SerializeStruct as _;

            let mut state = serializer.serialize_struct("PostProviderEncodingState", 1)?;
            state.serialize_field("fail_serialization", &false)?;
            state.end()
        }
    }

    impl CredentialState for PostProviderEncodingState {
        const KIND: &'static str = "post_provider_encoding_test";
        const VERSION: u32 = 1;
    }

    struct PostProviderEncodingCred;

    static ENCODING_PROVIDER_CALLS: AtomicUsize = AtomicUsize::new(0);

    impl Credential for PostProviderEncodingCred {
        type Properties = ();
        type Scheme = TestScheme;
        type State = PostProviderEncodingState;

        const KEY: &'static str = "test.post_provider_encoding";

        fn metadata() -> CredentialMetadata {
            CredentialMetadata::new(
                nebula_core::credential_key!("test.post_provider_encoding"),
                "Post-provider encoding test",
                "proves state encoding failures retain replay-unsafe disposition",
                crate::schema_of::<Self::Properties>(),
                AuthPattern::OAuth2,
            )
        }

        fn project(_state: &PostProviderEncodingState) -> TestScheme {
            TestScheme
        }

        async fn resolve(
            _values: &FieldValues,
            _ctx: &CredentialContext,
        ) -> Result<ResolveResult<PostProviderEncodingState, ()>, CredentialError> {
            Ok(ResolveResult::Complete(PostProviderEncodingState {
                fail_serialization: false,
            }))
        }
    }

    impl Refreshable for PostProviderEncodingCred {
        async fn refresh(
            state: &mut PostProviderEncodingState,
            attempt: RefreshAttempt<'_>,
        ) -> crate::RefreshReport {
            ENCODING_PROVIDER_CALLS.fetch_add(1, Ordering::SeqCst);
            let completed = attempt
                .dispatch(|| async { Ok::<(), std::convert::Infallible>(()) })
                .await;
            let Ok(completed) = completed else {
                panic!("infallible test dispatch must complete");
            };
            let ((), proof) = completed.into_parts();
            state.fail_serialization = true;
            proof.refreshed()
        }
    }

    impl CredentialLifecycle for PostProviderEncodingCred {
        fn policy(_state: &PostProviderEncodingState) -> CredentialPolicy {
            CredentialPolicy {
                expires_at: Some(Utc::now() - chrono::Duration::minutes(5)),
                lease: None,
                refresh: RefreshStrategy::RefreshToken,
                revoke: RevokeStrategy::HandleBased,
            }
        }
    }

    // ── Fixtures ───────────────────────────────────────────────────────

    static TEST_ID: OnceLock<CredentialId> = OnceLock::new();

    fn test_id() -> CredentialId {
        TEST_ID.get_or_init(CredentialId::new).to_owned()
    }

    fn test_owner() -> CredentialOwner {
        CredentialOwner::from_canonical("test-owner")
    }

    fn test_selector() -> CredentialSelector {
        CredentialSelector::new(test_owner(), test_id())
    }

    fn test_state_bytes() -> Vec<u8> {
        serde_json::to_vec(&TestState {
            token: "live".to_owned(),
        })
        .expect("serialize test state")
    }

    fn live_row() -> StoredCredential {
        live_row_with_token("live")
    }

    fn live_row_with_token(token: &str) -> StoredCredential {
        let now = Utc::now();
        let data = serde_json::to_vec(&TestState {
            token: token.to_owned(),
        })
        .expect("serialize test state");
        StoredLiveCredential::new(
            test_id(),
            None,
            TestCred::KEY.to_owned(),
            data.into(),
            TestState::KIND.to_owned(),
            TestState::VERSION,
            CredentialVersion::MIN,
            CredentialMaterialEpoch::MIN,
            now,
            now,
            None,
            false,
            serde_json::Map::new(),
            None,
        )
        .expect("fixture is a valid live credential")
        .into()
    }

    fn same_key_non_oauth_row() -> StoredCredential {
        let now = Utc::now();
        let data = serde_json::to_vec(&TestState {
            token: "same-key-live".to_owned(),
        })
        .expect("serialize same-key test state");
        StoredLiveCredential::new(
            test_id(),
            None,
            SameKeyNonOAuthCred::KEY.to_owned(),
            data.into(),
            TestState::KIND.to_owned(),
            TestState::VERSION,
            CredentialVersion::MIN,
            CredentialMaterialEpoch::MIN,
            now,
            now,
            Some(now - chrono::Duration::minutes(5)),
            false,
            serde_json::Map::new(),
            None,
        )
        .expect("fixture is a valid same-key credential")
        .into()
    }

    fn cancellation_aware_row() -> StoredCredential {
        let now = Utc::now();
        let data = serde_json::to_vec(&TestState {
            token: "cancellation-aware-live".to_owned(),
        })
        .expect("serialize cancellation-aware state");
        StoredLiveCredential::new(
            test_id(),
            None,
            CancellationAwareCred::KEY.to_owned(),
            data.into(),
            TestState::KIND.to_owned(),
            TestState::VERSION,
            CredentialVersion::MIN,
            CredentialMaterialEpoch::MIN,
            now,
            now,
            Some(now - chrono::Duration::minutes(5)),
            false,
            serde_json::Map::new(),
            None,
        )
        .expect("fixture is a valid cancellation-aware credential")
        .into()
    }

    fn post_provider_encoding_row() -> StoredCredential {
        let now = Utc::now();
        let data = serde_json::to_vec(&PostProviderEncodingState {
            fail_serialization: false,
        })
        .expect("initial state encoding must succeed");
        StoredLiveCredential::new(
            test_id(),
            None,
            PostProviderEncodingCred::KEY.to_owned(),
            data.into(),
            PostProviderEncodingState::KIND.to_owned(),
            PostProviderEncodingState::VERSION,
            CredentialVersion::MIN,
            CredentialMaterialEpoch::MIN,
            now,
            now,
            Some(now - chrono::Duration::minutes(5)),
            false,
            serde_json::Map::new(),
            None,
        )
        .expect("fixture is a valid encoding-failure credential")
        .into()
    }

    fn oauth2_row(refresh_token: Option<&str>) -> StoredCredential {
        oauth2_row_with_token_url(refresh_token, "https://provider.example/token")
    }

    fn corrupt_oauth2_row() -> StoredCredential {
        let now = Utc::now();
        StoredLiveCredential::new(
            test_id(),
            None,
            OAuth2Credential::KEY.to_owned(),
            b"not-json".to_vec().into(),
            OAuth2State::KIND.to_owned(),
            OAuth2State::VERSION,
            CredentialVersion::MIN,
            CredentialMaterialEpoch::MIN,
            now,
            now,
            Some(now - chrono::Duration::minutes(5)),
            false,
            serde_json::Map::new(),
            None,
        )
        .expect("fixture is a structurally valid row with corrupt state bytes")
        .into()
    }

    fn oauth2_row_with_token_url(refresh_token: Option<&str>, token_url: &str) -> StoredCredential {
        let now = Utc::now();
        let expires_at = now - chrono::Duration::minutes(5);
        let state = OAuth2State {
            access_token: crate::SecretString::new("expired-access-token"),
            token_type: "Bearer".to_owned(),
            refresh_token: refresh_token.map(crate::SecretString::new),
            expires_at: Some(expires_at),
            scopes: vec!["read".to_owned()],
            client_id: crate::SecretString::new("client-id"),
            client_secret: crate::SecretString::new("client-secret"),
            token_url: token_url.to_owned(),
            auth_style: crate::AuthStyle::Header,
        };
        let data = crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&state))
            .expect("serialize OAuth2 test state with explicit secret scope");
        StoredLiveCredential::new(
            test_id(),
            None,
            OAuth2Credential::KEY.to_owned(),
            data.into(),
            OAuth2State::KIND.to_owned(),
            OAuth2State::VERSION,
            CredentialVersion::MIN,
            CredentialMaterialEpoch::MIN,
            now,
            now,
            Some(expires_at),
            false,
            serde_json::Map::new(),
            None,
        )
        .expect("fixture is a valid OAuth2 credential")
        .into()
    }

    fn reauth_required_row() -> StoredCredential {
        let now = Utc::now();
        StoredLiveCredential::new(
            test_id(),
            None,
            TestCred::KEY.to_owned(),
            test_state_bytes().into(),
            TestState::KIND.to_owned(),
            TestState::VERSION,
            CredentialVersion::MIN,
            CredentialMaterialEpoch::MIN,
            now,
            now,
            None,
            true,
            serde_json::Map::new(),
            None,
        )
        .expect("fixture is a valid reauth-required credential")
        .into()
    }

    fn tombstoned_row() -> StoredCredential {
        let StoredCredential::Live(live) = live_row() else {
            unreachable!("live fixture must be structurally live");
        };
        let now = Utc::now();
        StoredTombstonedCredential::new(
            live.credential_id(),
            live.credential_key().to_owned(),
            live.state_kind().to_owned(),
            live.state_version(),
            live.version()
                .next_tombstone()
                .expect("fixture has tombstone headroom"),
            live.created_at(),
            now,
            now,
        )
        .into()
    }

    fn resolver_with(store: Arc<ScriptedStore>) -> CredentialResolver<ScriptedStore> {
        let coord = RefreshCoordinator::new_with(
            Arc::new(StubClaimRepo),
            ReplicaId::new("test-replica"),
            RefreshCoordConfig::default(),
        )
        .expect("default coordinator config is valid");
        CredentialResolver::with_dependencies(store, Arc::new(coord), Arc::new(StubTransport))
    }

    fn resolver_with_runtime(
        store: Arc<ScriptedStore>,
        claims: Arc<dyn RefreshClaimStore>,
        transport: Arc<dyn RefreshTransport>,
    ) -> CredentialResolver<ScriptedStore> {
        let coord = RefreshCoordinator::new_with(
            claims,
            ReplicaId::new("stateful-test-replica"),
            RefreshCoordConfig::default(),
        )
        .expect("default coordinator config is valid");
        CredentialResolver::with_dependencies(store, Arc::new(coord), transport)
    }

    fn oauth_service_with_runtime(
        store: Arc<ScriptedStore>,
        claims: Arc<dyn RefreshClaimStore>,
        transport: Arc<dyn RefreshTransport>,
    ) -> (
        crate::CredentialService,
        tokio_util::sync::CancellationToken,
    ) {
        let store_port: Arc<dyn CredentialPersistence> = store;
        let coord = RefreshCoordinator::new_with(
            claims,
            ReplicaId::new("service-test-replica"),
            RefreshCoordConfig::default(),
        )
        .expect("default coordinator config is valid");
        let observer: Arc<dyn crate::CredentialObserver> = Arc::new(crate::NoopObserver::new());
        let resolver = CredentialResolver::with_dependencies(
            Arc::clone(&store_port),
            Arc::new(coord),
            transport,
        )
        .with_event_bus(observer.event_bus());

        let pending = crate::ErasedPendingStore::new(Arc::new(UnusedPendingStore));
        let mut ops = crate::DispatchOps::new();
        crate::register_runtime_ops::<OAuth2Credential, crate::ErasedPendingStore>(&mut ops)
            .expect("OAuth2 base ops register");
        crate::register_refreshable_ops::<OAuth2Credential, crate::ErasedPendingStore>(&mut ops)
            .expect("OAuth2 refresh ops register");

        let mut registry = crate::CredentialRegistry::new();
        registry
            .register(OAuth2Credential, "nebula-credential-test")
            .expect("OAuth2 registry entry is unique");

        let shutdown = tokio_util::sync::CancellationToken::new();
        let lease = crate::runtime::LeaseLifecycle::spawn(
            crate::runtime::LeaseLifecycleConfig::default(),
            None,
            None,
            shutdown.clone(),
        );
        (
            crate::CredentialService::from_secure_parts(
                store_port,
                resolver,
                lease,
                pending,
                Arc::new(registry),
                Arc::new(ops),
                observer,
                crate::StateSource::LocalEncrypted,
            ),
            shutdown,
        )
    }

    // ── Regressions ────────────────────────────────────────────────────

    #[tokio::test]
    async fn linear_refresh_evidence_separates_predispatch_from_unknown_dispatch() {
        let ctx = CredentialContext::for_owner("test-owner");
        let exact = RefreshAttempt::new(&ctx, crate::RefreshExecutionMode::Provider)
            .not_dispatched(crate::RefreshFailureSpec::new(
                crate::RefreshErrorKind::ProtocolError,
                crate::RetryAdvice::Never,
            ))
            .into_kind();
        let RefreshReportKind::NotApplied(context) = exact else {
            panic!("pre-dispatch proof must remain exact");
        };
        assert_eq!(context.retry(), crate::error::RetryAdvice::Never);

        let unknown = RefreshAttempt::new(&ctx, crate::RefreshExecutionMode::Provider)
            .dispatch(|| async { Err::<(), ()>(()) })
            .await;
        let Err(unknown) = unknown else {
            panic!("failed dispatch must not produce completed-response proof");
        };
        let unknown = unknown.into_report().into_kind();
        assert!(matches!(unknown, RefreshReportKind::OutcomeUnknown));
    }

    #[tokio::test]
    async fn refresh_dispatch_is_type_directed_even_when_key_matches_oauth2() {
        SAME_KEY_TYPED_REFRESH_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::new(same_key_non_oauth_row(), false));
        let resolver = resolver_with(Arc::clone(&store));

        resolver
            .resolve_with_refresh::<SameKeyNonOAuthCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect("same-key credential must use its own typed refresh");

        assert_eq!(
            SAME_KEY_TYPED_REFRESH_CALLS.load(Ordering::SeqCst),
            1,
            "typed Refreshable implementation must be called exactly once"
        );
        let StoredCredential::Live(row) = store.snapshot() else {
            panic!("typed refresh must leave a live credential row");
        };
        let persisted: TestState =
            serde_json::from_slice(row.data()).expect("persisted typed state must decode");
        assert_eq!(persisted.token, "same-key-typed-refresh");
    }

    #[tokio::test]
    async fn request_cancellation_after_provider_start_does_not_abort_owned_refresh() {
        CANCELLATION_REFRESH_STARTED.store(false, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::new(cancellation_aware_row(), false));
        let resolver = Arc::new(resolver_with(Arc::clone(&store)));
        let request_context = CredentialContext::for_owner("test-owner");
        let request_cancel = request_context.cancel_token().clone();

        let task = tokio::spawn({
            let resolver = Arc::clone(&resolver);
            async move {
                resolver
                    .resolve_with_refresh::<CancellationAwareCred>(
                        &test_selector(),
                        &request_context,
                    )
                    .await
            }
        });

        loop {
            let started = CANCELLATION_REFRESH_STARTED_NOTIFY.notified();
            if CANCELLATION_REFRESH_STARTED.load(Ordering::SeqCst) {
                break;
            }
            started.await;
        }
        request_cancel.cancel();
        CANCELLATION_REFRESH_RELEASE.notify_one();

        task.await
            .expect("refresh task must not panic")
            .expect("request cancellation must not abort the owned K2 section");
        assert_eq!(store.replacement_count(), 1);

        let StoredCredential::Live(stored) = store.snapshot() else {
            panic!("successful refresh must leave a live credential");
        };
        let state: TestState =
            serde_json::from_slice(stored.data()).expect("stored refreshed state is valid");
        assert_eq!(state.token, "cancellation-aware-refreshed");
    }

    #[tokio::test]
    async fn post_provider_state_encoding_failure_is_replay_unsafe() {
        use nebula_error::Classify;

        ENCODING_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::new(post_provider_encoding_row(), false));
        let claims = Arc::new(StatefulClaimRepo::default());
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = Arc::new(StubTransport);
        let resolver = resolver_with_runtime(Arc::clone(&store), claim_port, transport_port);

        let error = resolver
            .resolve_with_refresh::<PostProviderEncodingCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("encoding after provider success must fail closed");

        assert!(matches!(
            &error,
            ResolveError::PostProviderStateEncoding { .. }
        ));
        assert_eq!(ENCODING_PROVIDER_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(
            store.replacement_count(),
            0,
            "encoding failure must stop before persistence"
        );
        assert!(
            claims.active.load(Ordering::SeqCst),
            "post-provider encoding failure must retain fail-closed L2 poison"
        );
        assert_eq!(claims.release_count.load(Ordering::SeqCst), 0);
        let mapped = resolve_error_to_credential_error(error);
        assert!(matches!(&mapped, CredentialError::PostProviderPersistence));
        assert!(!mapped.is_retryable());
    }

    #[tokio::test]
    async fn pre_dispatch_display_write_is_merged_without_false_coalescing() {
        CAS_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::racing_display_read(live_row_with_token(
            "counted-cas",
        )));
        let resolver = resolver_with(Arc::clone(&store));

        resolver
            .resolve_with_refresh::<TestCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect("a display-only write must not suppress the required refresh");

        assert_eq!(
            CAS_PROVIDER_CALLS.load(Ordering::SeqCst),
            1,
            "a row-version bump with the same epoch is not refresh success"
        );
        assert_eq!(store.replacement_count(), 1);
        let StoredCredential::Live(row) = store.snapshot() else {
            panic!("successful refresh keeps the row live");
        };
        assert_eq!(row.name(), Some("renamed-before-dispatch"));
        assert_eq!(
            row.material_epoch(),
            CredentialMaterialEpoch::MIN
                .next()
                .expect("fixture has epoch headroom")
        );
        let state: TestState =
            serde_json::from_slice(row.data()).expect("refreshed state remains valid");
        assert_eq!(state.token, "refreshed");
    }

    #[tokio::test]
    async fn byte_identical_material_replacement_forces_re_evaluation_by_epoch() {
        CAS_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::racing_identical_material_read(
            live_row_with_token("counted-cas"),
        ));
        let resolver = resolver_with(Arc::clone(&store));

        resolver
            .resolve_with_refresh::<TestCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect("new material authority must be re-evaluated before refresh");

        assert_eq!(
            CAS_PROVIDER_CALLS.load(Ordering::SeqCst),
            1,
            "the superseded attempt must not dispatch; only the re-evaluated epoch may refresh"
        );
        assert_eq!(store.replacement_count(), 1);
        let StoredCredential::Live(row) = store.snapshot() else {
            panic!("successful re-evaluation keeps the row live");
        };
        let second_epoch = CredentialMaterialEpoch::MIN
            .next()
            .and_then(CredentialMaterialEpoch::next)
            .expect("fixture has epoch headroom");
        assert_eq!(
            row.material_epoch(),
            second_epoch,
            "byte-identical replacement and later refresh each advance authority"
        );
    }

    #[tokio::test]
    async fn refresh_racing_revoke_does_not_resurrect() {
        CAS_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::new(
            live_row_with_token("counted-cas"),
            /* revoke_on_first_replace */ true,
        ));
        let resolver = resolver_with(Arc::clone(&store));
        let ctx = CredentialContext::for_owner("test-owner");

        let err = resolver
            .resolve_with_refresh::<TestCred>(&test_selector(), &ctx)
            .await
            .expect_err("a refresh racing a revoke must fail closed, never resurrect");

        assert!(
            matches!(
                &err,
                ResolveError::PostProviderPersistence {
                    source: CredentialPersistenceError::VersionConflict { .. },
                    ..
                }
            ),
            "the stale provider result must stop at its original CAS boundary, got {err:?}"
        );

        let final_row = store.snapshot();
        assert!(
            matches!(final_row, StoredCredential::Tombstoned(_)),
            "the structural tombstone must survive"
        );
        assert_eq!(
            store.replacement_count(),
            1,
            "must not issue a second replacement after observing the tombstone"
        );
        assert_eq!(
            store.get_count(),
            2,
            "initial load plus the required pre-dispatch authority reload are exact; \
             the post-provider conflict must not trigger a third reconciliation read"
        );
        assert_eq!(
            CAS_PROVIDER_CALLS.load(Ordering::SeqCst),
            1,
            "a CAS conflict after provider success must not repeat provider work"
        );
        use nebula_error::Classify;
        let mapped = resolve_error_to_credential_error(err);
        assert!(
            !mapped.is_retryable(),
            "a post-provider CAS conflict must stop generic retry loops"
        );
    }

    #[tokio::test]
    async fn refresh_without_race_succeeds_and_stamps_validation() {
        let store = Arc::new(ScriptedStore::new(
            live_row(),
            /* revoke_on_first_replace */ false,
        ));
        let resolver = resolver_with(Arc::clone(&store));
        let ctx = CredentialContext::for_owner("test-owner");

        resolver
            .resolve_with_refresh::<TestCred>(&test_selector(), &ctx)
            .await
            .expect("an uncontended refresh must succeed");

        let final_row = store.snapshot();
        let StoredCredential::Live(final_row) = final_row else {
            panic!("an uncontended refresh must remain live");
        };
        assert!(
            last_validated_at(&final_row).is_some(),
            "a provider-contacting refresh must stamp the re-validation anchor"
        );
        assert_eq!(
            store.replacement_count(),
            1,
            "exactly one replacement on the happy path"
        );
        assert_eq!(
            store.get_count(),
            2,
            "initial load plus the pre-dispatch authority reload are required; \
             a confirmed replacement must not trigger a post-write read"
        );
    }

    #[tokio::test]
    async fn resolve_with_refresh_rejects_pretombstoned_row() {
        let store = Arc::new(ScriptedStore::new(tombstoned_row(), false));
        let resolver = resolver_with(Arc::clone(&store));
        let ctx = CredentialContext::for_owner("test-owner");

        let err = resolver
            .resolve_with_refresh::<TestCred>(&test_selector(), &ctx)
            .await
            .expect_err("a row revoked before resolve must not refresh");

        assert!(
            matches!(
                err,
                ResolveError::Store(CredentialPersistenceError::NotFound)
            ),
            "got {err:?}"
        );
        assert_eq!(
            store.replacement_count(),
            0,
            "a tombstoned row must be rejected at load, before any write"
        );
    }

    #[tokio::test]
    async fn resolve_rejects_tombstoned_row() {
        let store = Arc::new(ScriptedStore::new(tombstoned_row(), false));
        let resolver = resolver_with(Arc::clone(&store));

        let err = resolver
            .resolve::<TestCred>(&test_selector())
            .await
            .expect_err("a tombstoned row must not resolve to a handle");

        assert!(
            matches!(
                err,
                ResolveError::Store(CredentialPersistenceError::NotFound)
            ),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn persisted_reauth_blocks_plain_and_refresh_resolution_without_provider_work() {
        for path in ["plain", "scoped", "refresh"] {
            let store = Arc::new(ScriptedStore::new(reauth_required_row(), false));
            let resolver = resolver_with(Arc::clone(&store));
            let error = match path {
                "plain" => resolver
                    .resolve::<TestCred>(&test_selector())
                    .await
                    .expect_err("plain resolve must honor persisted reauth"),
                "scoped" => resolver
                    .resolve_scoped::<TestCred>(&test_selector())
                    .await
                    .expect_err("scoped resolve must honor persisted reauth"),
                "refresh" => resolver
                    .resolve_with_refresh::<TestCred>(
                        &test_selector(),
                        &CredentialContext::for_owner("test-owner"),
                    )
                    .await
                    .expect_err("refresh resolve must honor persisted reauth"),
                _ => unreachable!("closed test path set"),
            };

            assert!(
                matches!(
                    error,
                    ResolveError::ReauthRequired {
                        reason: ReauthReason::ProviderRejected,
                        ..
                    }
                ),
                "{path} returned {error:?}"
            );
            assert_eq!(
                store.replacement_count(),
                0,
                "{path} must reject before provider or persistence work"
            );
        }
    }

    #[tokio::test]
    async fn provider_mode_local_completion_is_exact_before_dispatch_and_releases_claim() {
        MODE_MISMATCH_REFRESH_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::new(live_row(), false));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(OAuthTransportResult::Success));
        let resolver = resolver_with_runtime(
            Arc::clone(&store),
            Arc::clone(&claims) as Arc<dyn RefreshClaimStore>,
            Arc::clone(&transport) as Arc<dyn RefreshTransport>,
        );

        let error = resolver
            .resolve_with_refresh::<ProviderModeLocalCompletionCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("provider mode must reject a providerless completion");

        let ResolveError::RefreshNotApplied { context, .. } = error else {
            panic!("mode mismatch must retain exact no-dispatch evidence");
        };
        assert_eq!(
            context.phase(),
            crate::RefreshNotAppliedPhase::BeforeDispatch
        );
        assert_eq!(context.kind(), crate::RefreshErrorKind::ProtocolError);
        assert_eq!(context.retry(), crate::RetryAdvice::Never);
        assert_eq!(
            context
                .diagnostic_code()
                .map(crate::RefreshDiagnosticCode::as_str),
            Some("refresh.execution_mode_mismatch")
        );
        assert_eq!(MODE_MISMATCH_REFRESH_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(
            transport.call_count(),
            0,
            "a declaration mismatch must be refused before provider transport"
        );
        assert_eq!(
            claims.release_count.load(Ordering::SeqCst),
            1,
            "an exact before-dispatch result must release the refresh claim"
        );
        assert!(!claims.active.load(Ordering::SeqCst));

        let StoredCredential::Live(row) = store.snapshot() else {
            panic!("retry-gate finalization must keep the credential live");
        };
        assert_eq!(row.material_epoch(), CredentialMaterialEpoch::MIN);
        assert!(matches!(
            row.refresh_retry_gate(),
            Some(RefreshRetryGate::Never { .. })
        ));
        let state: TestState =
            serde_json::from_slice(row.data()).expect("stored state remains valid");
        assert_eq!(
            state.token, "live",
            "the invalid locally-mutated state must never be committed"
        );

        let restarted_claims = Arc::new(StatefulClaimRepo::default());
        let restarted = resolver_with_runtime(
            Arc::clone(&store),
            Arc::clone(&restarted_claims) as Arc<dyn RefreshClaimStore>,
            Arc::clone(&transport) as Arc<dyn RefreshTransport>,
        );
        let restarted_error = restarted
            .resolve_with_refresh::<ProviderModeLocalCompletionCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("a second replica must honor the durable mismatch gate");
        assert!(matches!(
            restarted_error,
            ResolveError::RefreshNotApplied { .. }
        ));
        assert_eq!(MODE_MISMATCH_REFRESH_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(transport.call_count(), 0);
        assert_eq!(
            restarted_claims.try_claim_count.load(Ordering::SeqCst),
            0,
            "durable admission must suppress the second replica before L2"
        );
    }

    #[tokio::test]
    async fn provider_rejection_is_not_reported_without_durable_reauth_acknowledgement() {
        REAUTH_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::failing_replace(
            live_row_with_token("reject"),
            CredentialPersistenceError::Unavailable,
        ));
        let claims = Arc::new(StatefulClaimRepo::default());
        let resolver = resolver_with_runtime(
            Arc::clone(&store),
            Arc::clone(&claims) as Arc<dyn RefreshClaimStore>,
            Arc::new(StubTransport),
        );

        let error = resolver
            .resolve_with_refresh::<TestCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("an unpersisted reauth decision must fail closed");

        assert!(matches!(
            &error,
            ResolveError::ReauthDecisionFinalization { .. }
        ));
        assert_eq!(store.replacement_count(), 1);
        let StoredCredential::Live(row) = store.snapshot() else {
            panic!("failed replacement must preserve the live row");
        };
        assert!(
            !row.reauth_required(),
            "failed persistence cannot pretend the replica-visible bit was committed"
        );
        assert_eq!(REAUTH_PROVIDER_CALLS.load(Ordering::SeqCst), 1);
        assert!(claims.active.load(Ordering::SeqCst));
        assert_eq!(
            claims.release_count.load(Ordering::SeqCst),
            0,
            "a provider-confirmed decision without durable acknowledgement retains L2"
        );
        use nebula_error::Classify;
        let mapped = resolve_error_to_credential_error(error);
        assert!(matches!(&mapped, CredentialError::RefreshFinalization));
        assert!(!matches!(&mapped, CredentialError::OutcomeUnknown));
        assert!(
            !mapped.is_retryable(),
            "an unpersisted provider rejection must not POST the dead grant again"
        );
    }

    #[tokio::test]
    async fn definite_retry_gate_finalization_failure_is_exact_and_retry_unsafe() {
        AFTER_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::failing_replace(
            live_row_with_token("after"),
            CredentialPersistenceError::Unavailable,
        ));
        let claims = Arc::new(StatefulClaimRepo::default());
        let resolver = resolver_with_runtime(
            Arc::clone(&store),
            Arc::clone(&claims) as Arc<dyn RefreshClaimStore>,
            Arc::new(StubTransport),
        );

        let error = resolver
            .resolve_with_refresh::<TestCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("an exact provider refusal without a durable gate must fail closed");

        assert!(matches!(
            &error,
            ResolveError::RefreshRetryGateFinalization { .. }
        ));
        assert_eq!(AFTER_PROVIDER_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(store.replacement_count(), 1);
        assert!(
            claims.active.load(Ordering::SeqCst),
            "retry-unsafe exact finalization failure retains L2"
        );
        assert_eq!(claims.release_count.load(Ordering::SeqCst), 0);

        let mapped = resolve_error_to_credential_error(error);
        assert!(matches!(&mapped, CredentialError::RefreshFinalization));
        assert!(
            !matches!(&mapped, CredentialError::OutcomeUnknown),
            "definite finalization failure must not collapse into ambiguous outcome"
        );
        use nebula_error::Classify;
        assert!(!mapped.is_retryable());
    }

    #[tokio::test]
    async fn predispatch_reauth_persistence_failure_releases_claim() {
        let store = Arc::new(ScriptedStore::failing_replace(
            oauth2_row(None),
            CredentialPersistenceError::Unavailable,
        ));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(
            OAuthTransportResult::InvalidGrant,
        ));
        let resolver = resolver_with_runtime(
            Arc::clone(&store),
            Arc::clone(&claims) as Arc<dyn RefreshClaimStore>,
            Arc::clone(&transport) as Arc<dyn RefreshTransport>,
        );

        let error = resolver
            .resolve_with_refresh::<OAuth2Credential>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("missing material cannot become durable during the outage");
        assert!(matches!(
            error,
            ResolveError::Store(CredentialPersistenceError::Unavailable)
        ));
        assert_eq!(transport.call_count(), 0);
        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_release_count(1))
            .await
            .expect("definite predispatch persistence failure releases the claim");
        assert!(!claims.active.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn reauth_cas_merges_display_race_without_redispatch() {
        REAUTH_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::racing_display_replace(live_row_with_token(
            "reject",
        )));
        let resolver = resolver_with(Arc::clone(&store));

        let error = resolver
            .resolve_with_refresh::<TestCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("provider rejection requires reauthentication");
        assert!(matches!(error, ResolveError::ReauthRequired { .. }));
        assert_eq!(REAUTH_PROVIDER_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(store.replacement_count(), 2);
        let StoredCredential::Live(row) = store.snapshot() else {
            panic!("reauth keeps a live management row");
        };
        assert_eq!(row.name(), Some("renamed-concurrently"));
        assert!(row.reauth_required());
        assert_eq!(
            row.material_epoch(),
            CredentialMaterialEpoch::MIN
                .next()
                .expect("fixture has epoch headroom"),
            "a durable reauth decision invalidates stale refresh authority"
        );

        let second = resolver_with(store)
            .resolve_with_refresh::<TestCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("durable reauth blocks a second provider request");
        assert!(matches!(second, ResolveError::ReauthRequired { .. }));
        assert_eq!(REAUTH_PROVIDER_CALLS.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn stale_same_epoch_retry_gate_cannot_reattach_after_durable_reauth() {
        let store = Arc::new(ScriptedStore::new(live_row(), false));
        let selector = test_selector();
        let StoredCredential::Live(observed) = store.snapshot() else {
            panic!("fixture must start live");
        };

        assert!(matches!(
            persist_reauth_required(store.as_ref(), &selector, observed.clone()).await,
            ReauthWrite::Applied
        ));
        let context = Box::new(RefreshNotAppliedContext::from_spec(
            crate::RefreshNotAppliedPhase::BeforeDispatch,
            crate::RefreshFailureSpec::new(
                crate::RefreshErrorKind::TransientNetwork,
                crate::RetryAdvice::Never,
            ),
        ));
        assert!(matches!(
            persist_retry_gate(store.as_ref(), &selector, observed, context).await,
            RetryGateWrite::Superseded(CredentialPersistenceError::VersionConflict { .. })
        ));

        let StoredCredential::Live(current) = store.snapshot() else {
            panic!("reauth decision keeps a live management row");
        };
        assert!(current.reauth_required());
        assert_eq!(
            current.material_epoch(),
            CredentialMaterialEpoch::MIN
                .next()
                .expect("fixture has epoch headroom")
        );
        assert!(
            current.refresh_retry_gate().is_none(),
            "old-epoch retry evidence must not attach to new reauth authority"
        );
    }

    #[tokio::test]
    async fn bounded_display_churn_is_definite_conflict_not_unknown_outcome() {
        let selector = test_selector();
        let gate_store = Arc::new(ScriptedStore::continuous_display_churn(live_row()));
        let StoredCredential::Live(gate_observed) = gate_store.snapshot() else {
            panic!("fixture must start live");
        };
        let context = Box::new(RefreshNotAppliedContext::from_spec(
            crate::RefreshNotAppliedPhase::BeforeDispatch,
            crate::RefreshFailureSpec::new(
                crate::RefreshErrorKind::TransientNetwork,
                crate::RetryAdvice::Never,
            ),
        ));
        assert!(matches!(
            persist_retry_gate(gate_store.as_ref(), &selector, gate_observed, context).await,
            RetryGateWrite::DefiniteFailure(CredentialPersistenceError::VersionConflict { .. })
        ));
        assert!(
            gate_store.replacement_count() > 1,
            "fixture must exhaust the bounded display-merge loop"
        );

        let reauth_store = Arc::new(ScriptedStore::continuous_display_churn(live_row()));
        let StoredCredential::Live(reauth_observed) = reauth_store.snapshot() else {
            panic!("fixture must start live");
        };
        assert!(matches!(
            persist_reauth_required(reauth_store.as_ref(), &selector, reauth_observed).await,
            ReauthWrite::DefiniteFailure(CredentialPersistenceError::VersionConflict { .. })
        ));
        let StoredCredential::Live(current) = reauth_store.snapshot() else {
            panic!("display churn keeps the row live");
        };
        assert!(!current.reauth_required());
        assert_eq!(current.material_epoch(), CredentialMaterialEpoch::MIN);
    }

    #[tokio::test]
    async fn post_provider_name_conflict_and_unavailable_are_typed_non_retryable() {
        use nebula_error::Classify;

        NAME_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let name_store = Arc::new(ScriptedStore::failing_replace(
            live_row_with_token("counted-name"),
            CredentialPersistenceError::AlreadyExists {
                key: CredentialAlreadyExistsKey::Name,
            },
        ));
        let name_error = resolver_with(Arc::clone(&name_store))
            .resolve_with_refresh::<TestCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("a post-provider name collision must fail closed");
        assert!(matches!(
            &name_error,
            ResolveError::PostProviderPersistence {
                source: CredentialPersistenceError::AlreadyExists {
                    key: CredentialAlreadyExistsKey::Name,
                },
                ..
            }
        ));
        assert_eq!(NAME_PROVIDER_CALLS.load(Ordering::SeqCst), 1);
        assert!(
            !resolve_error_to_credential_error(name_error).is_retryable(),
            "a name-only race must not replay provider work"
        );

        UNAVAILABLE_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let unavailable_store = Arc::new(ScriptedStore::failing_replace(
            live_row_with_token("counted-unavailable"),
            CredentialPersistenceError::Unavailable,
        ));
        let unavailable_error = resolver_with(Arc::clone(&unavailable_store))
            .resolve_with_refresh::<TestCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("a post-provider definite outage must fail closed");
        assert!(matches!(
            &unavailable_error,
            ResolveError::PostProviderPersistence {
                source: CredentialPersistenceError::Unavailable,
                ..
            }
        ));
        assert_eq!(UNAVAILABLE_PROVIDER_CALLS.load(Ordering::SeqCst), 1);
        assert!(
            !resolve_error_to_credential_error(unavailable_error).is_retryable(),
            "a definite persistence outage after provider success is not a safe full retry"
        );
    }

    #[tokio::test]
    async fn post_provider_unknown_commit_outcome_remains_exact_and_non_retryable() {
        use nebula_error::Classify;

        UNKNOWN_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::failing_replace(
            live_row_with_token("counted-unknown"),
            CredentialPersistenceError::OutcomeUnknown,
        ));
        let error = resolver_with(Arc::clone(&store))
            .resolve_with_refresh::<TestCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("unknown commit acknowledgement must survive the resolver");
        assert!(matches!(
            &error,
            ResolveError::PostProviderPersistence {
                source: CredentialPersistenceError::OutcomeUnknown,
                ..
            }
        ));
        assert_eq!(UNKNOWN_PROVIDER_CALLS.load(Ordering::SeqCst), 1);
        let mapped = resolve_error_to_credential_error(error);
        assert!(matches!(mapped, CredentialError::OutcomeUnknown));
        assert!(!mapped.is_retryable());
    }

    #[tokio::test]
    async fn oauth_ack_loss_is_outcome_unknown_and_retains_l2_against_immediate_replay() {
        use nebula_error::Classify;

        let store = Arc::new(ScriptedStore::new(
            oauth2_row(Some("rotating-grant")),
            false,
        ));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(OAuthTransportResult::AckLost));
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = transport.clone();
        let resolver = resolver_with_runtime(Arc::clone(&store), claim_port, transport_port);
        let selector = test_selector();
        let ctx = CredentialContext::for_owner("test-owner");

        let error = resolver
            .resolve_with_refresh::<OAuth2Credential>(&selector, &ctx)
            .await
            .expect_err("a response acknowledgement loss cannot prove provider outcome");

        assert!(matches!(
            &error,
            ResolveError::ProviderOutcomeUnknown { .. }
        ));
        assert_eq!(transport.call_count(), 1, "exactly one provider dispatch");
        assert_eq!(
            store.replacement_count(),
            0,
            "unknown provider outcome must not synthesize a persistence transition"
        );
        assert!(
            claims.active.load(Ordering::SeqCst),
            "the sentinel L2 claim must remain active and become poison until reconciliation"
        );
        assert_eq!(
            claims.release_count.load(Ordering::SeqCst),
            0,
            "unknown outcome must not release L2"
        );
        let mapped = resolve_error_to_credential_error(error);
        assert!(matches!(mapped, CredentialError::OutcomeUnknown));
        assert!(!mapped.is_retryable());

        let second = tokio::spawn({
            let resolver = resolver.clone();
            let selector = selector.clone();
            let ctx = ctx.clone();
            async move {
                resolver
                    .resolve_with_refresh::<OAuth2Credential>(&selector, &ctx)
                    .await
            }
        });
        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_try_claim_count(2))
            .await
            .expect("the second local request must reach the retained L2 claim");
        assert_eq!(
            transport.call_count(),
            1,
            "a retained ambiguous claim must prevent an immediate second POST"
        );
        second.abort();
        let _ = second.await;
    }

    #[tokio::test]
    async fn oauth_invalid_grant_persists_reauth_and_releases_confirmed_claim() {
        use nebula_error::Classify;

        let store = Arc::new(ScriptedStore::new(oauth2_row(Some("revoked-grant")), false));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(
            OAuthTransportResult::InvalidGrant,
        ));
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = transport.clone();
        let resolver = resolver_with_runtime(Arc::clone(&store), claim_port, transport_port);

        let error = resolver
            .resolve_with_refresh::<OAuth2Credential>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("invalid_grant must require user re-authorization");

        assert!(matches!(
            &error,
            ResolveError::ReauthRequired {
                reason: ReauthReason::ProviderRejected,
                ..
            }
        ));
        assert_eq!(transport.call_count(), 1);
        assert_eq!(
            store.replacement_count(),
            1,
            "the replica-visible reauth decision must be persisted exactly once"
        );
        let StoredCredential::Live(row) = store.snapshot() else {
            panic!("invalid_grant must retain a live row marked for reauth");
        };
        assert!(row.reauth_required());

        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_release_count(1))
            .await
            .expect("a confirmed provider rejection should release L2 best-effort");
        assert!(!claims.active.load(Ordering::SeqCst));

        let mapped = resolve_error_to_credential_error(error);
        let CredentialError::Provider(context) = mapped else {
            panic!("reauth must map to the public provider error taxonomy");
        };
        assert_eq!(
            context.kind(),
            crate::error::ProviderErrorKind::InvalidGrant
        );
        assert!(!CredentialError::Provider(context).is_retryable());
    }

    #[tokio::test]
    async fn oauth_missing_refresh_material_is_exact_without_transport_dispatch() {
        let store = Arc::new(ScriptedStore::new(oauth2_row(None), false));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(
            OAuthTransportResult::InvalidGrant,
        ));
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = transport.clone();
        let resolver = resolver_with_runtime(Arc::clone(&store), claim_port, transport_port);

        let error = resolver
            .resolve_with_refresh::<OAuth2Credential>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("a credential without refresh material must require reacquisition");

        assert!(matches!(
            error,
            ResolveError::ReauthRequired {
                reason: ReauthReason::MissingRefreshMaterial,
                ..
            }
        ));
        assert_eq!(
            transport.call_count(),
            0,
            "missing local material is known before provider dispatch"
        );
        assert_eq!(store.replacement_count(), 1);
        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_release_count(1))
            .await
            .expect("a locally exact reauth outcome should release L2");
    }

    #[tokio::test]
    async fn oauth_pre_dispatch_rejection_persists_never_gate_before_releasing_l2() {
        let store = Arc::new(ScriptedStore::new(
            oauth2_row_with_token_url(Some("refresh-grant"), "http://provider.example/token"),
            false,
        ));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(OAuthTransportResult::Success));
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = transport.clone();
        let resolver = resolver_with_runtime(Arc::clone(&store), claim_port, transport_port);

        let error = resolver
            .resolve_with_refresh::<OAuth2Credential>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("invalid local endpoint must fail before dispatch");

        assert!(matches!(&error, ResolveError::RefreshNotApplied { .. }));
        assert_eq!(transport.call_count(), 0);
        assert_eq!(
            store.replacement_count(),
            1,
            "the exact failure must be made durable before L2 release"
        );
        let StoredCredential::Live(gated) = store.snapshot() else {
            panic!("the credential remains live behind a retry gate");
        };
        assert!(matches!(
            gated.refresh_retry_gate(),
            Some(RefreshRetryGate::Never { .. })
        ));
        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_release_count(1))
            .await
            .expect("a durably gated pre-dispatch failure should release L2");

        let CredentialError::RefreshNotApplied(context) = resolve_error_to_credential_error(error)
        else {
            panic!("pre-dispatch failure must retain typed refresh context");
        };
        assert_eq!(
            context.kind(),
            crate::error::RefreshErrorKind::ProtocolError
        );
        assert_eq!(context.retry(), crate::error::RetryAdvice::Never);

        let restarted_claims = Arc::new(StatefulClaimRepo::default());
        let restarted_claim_port: Arc<dyn RefreshClaimStore> = restarted_claims.clone();
        let restarted_transport: Arc<dyn RefreshTransport> = Arc::clone(&transport) as _;
        let restarted = resolver_with_runtime(
            Arc::clone(&store),
            restarted_claim_port,
            restarted_transport,
        );
        let restarted_error = restarted
            .resolve_with_refresh::<OAuth2Credential>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("restart must honor the backend retry gate before L2");
        assert!(matches!(
            restarted_error,
            ResolveError::RefreshNotApplied { .. }
        ));
        assert_eq!(restarted_claims.try_claim_count.load(Ordering::SeqCst), 0);
        assert_eq!(transport.call_count(), 0);
    }

    #[tokio::test]
    async fn oauth_transient_or_unknown_endpoint_response_retains_l2() {
        for result in [
            OAuthTransportResult::ServerError503,
            OAuthTransportResult::Other429,
        ] {
            let store = Arc::new(ScriptedStore::new(oauth2_row(Some("refresh-grant")), false));
            let claims = Arc::new(StatefulClaimRepo::default());
            let transport = Arc::new(ScriptedOAuthTransport::new(result));
            let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
            let transport_port: Arc<dyn RefreshTransport> = transport.clone();
            let resolver = resolver_with_runtime(Arc::clone(&store), claim_port, transport_port);

            let error = resolver
                .resolve_with_refresh::<OAuth2Credential>(
                    &test_selector(),
                    &CredentialContext::for_owner("test-owner"),
                )
                .await
                .expect_err("transient or unknown endpoint response is replay-ambiguous");

            assert!(matches!(
                &error,
                ResolveError::ProviderOutcomeUnknown { .. }
            ));
            assert_eq!(transport.call_count(), 1);
            assert_eq!(store.replacement_count(), 0);
            assert!(claims.active.load(Ordering::SeqCst));
            assert_eq!(claims.release_count.load(Ordering::SeqCst), 0);
        }
    }

    #[tokio::test]
    async fn oauth_definitive_endpoint_denial_is_exact_and_releases_l2() {
        let store = Arc::new(ScriptedStore::new(oauth2_row(Some("refresh-grant")), false));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(
            OAuthTransportResult::InvalidClient401,
        ));
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = transport.clone();
        let resolver = resolver_with_runtime(Arc::clone(&store), claim_port, transport_port);

        let error = resolver
            .resolve_with_refresh::<OAuth2Credential>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("definitive OAuth denial must be exact");

        assert!(matches!(&error, ResolveError::RefreshNotApplied { .. }));
        assert_eq!(transport.call_count(), 1);
        assert_eq!(store.replacement_count(), 1);
        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_release_count(1))
            .await
            .expect("a definitive provider denial should release L2");

        let CredentialError::RefreshNotApplied(context) = resolve_error_to_credential_error(error)
        else {
            panic!("endpoint denial must retain typed refresh context");
        };
        assert_eq!(
            context.kind(),
            crate::error::RefreshErrorKind::ProtocolError
        );
        assert_eq!(context.retry(), crate::error::RetryAdvice::Never);
        assert_eq!(
            context
                .diagnostic_code()
                .map(crate::RefreshDiagnosticCode::as_str),
            Some("invalid_client")
        );

        let second = resolver_with_runtime(
            Arc::clone(&store),
            Arc::clone(&claims) as Arc<dyn RefreshClaimStore>,
            Arc::clone(&transport) as Arc<dyn RefreshTransport>,
        )
        .resolve_with_refresh::<OAuth2Credential>(
            &test_selector(),
            &CredentialContext::for_owner("test-owner"),
        )
        .await
        .expect_err("a second coordinator must observe the durable Never gate");
        assert!(matches!(second, ResolveError::RefreshNotApplied { .. }));
        assert_eq!(
            transport.call_count(),
            1,
            "two independent coordinators sharing one store dispatch exactly once"
        );
    }

    #[tokio::test]
    async fn retry_gate_cas_merges_display_race_without_redispatch() {
        let store = Arc::new(ScriptedStore::racing_display_replace(oauth2_row(Some(
            "refresh-grant",
        ))));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(
            OAuthTransportResult::InvalidClient401,
        ));
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = transport.clone();
        let selector = test_selector();
        let ctx = CredentialContext::for_owner("test-owner");

        let first = resolver_with_runtime(
            Arc::clone(&store),
            Arc::clone(&claim_port),
            Arc::clone(&transport_port),
        )
        .resolve_with_refresh::<OAuth2Credential>(&selector, &ctx)
        .await
        .expect_err("the exact provider denial remains an error");
        assert!(matches!(first, ResolveError::RefreshNotApplied { .. }));
        assert_eq!(transport.call_count(), 1);
        assert_eq!(
            store.replacement_count(),
            2,
            "the gate writer must re-read and CAS once after the display-only race"
        );
        let StoredCredential::Live(row) = store.snapshot() else {
            panic!("display race keeps the credential live");
        };
        assert_eq!(row.name(), Some("renamed-concurrently"));
        assert!(matches!(
            row.refresh_retry_gate(),
            Some(RefreshRetryGate::Never { .. })
        ));
        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_release_count(1))
            .await
            .expect("the claim releases only after the merged gate is durable");

        let second = resolver_with_runtime(store, claim_port, transport_port)
            .resolve_with_refresh::<OAuth2Credential>(&selector, &ctx)
            .await
            .expect_err("the merged gate must block a new coordinator");
        assert!(matches!(second, ResolveError::RefreshNotApplied { .. }));
        assert_eq!(transport.call_count(), 1, "CAS retry must not redispatch");
    }

    #[tokio::test]
    async fn timed_retry_gate_blocks_all_replicas_until_backend_expiry() {
        AFTER_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::new(live_row_with_token("after"), false));
        let claims = Arc::new(StatefulClaimRepo::default());
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport: Arc<dyn RefreshTransport> = Arc::new(StubTransport);
        let selector = test_selector();
        let ctx = CredentialContext::for_owner("test-owner");

        let first = resolver_with_runtime(
            Arc::clone(&store),
            Arc::clone(&claim_port),
            Arc::clone(&transport),
        )
        .resolve_with_refresh::<TestCred>(&selector, &ctx)
        .await
        .expect_err("the first exact transient response installs a timed gate");
        let ResolveError::RefreshNotApplied { context, .. } = first else {
            panic!("timed exact failure must preserve its typed context");
        };
        assert!(matches!(context.retry(), crate::RetryAdvice::After(_)));
        assert_eq!(AFTER_PROVIDER_CALLS.load(Ordering::SeqCst), 1);
        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_release_count(1))
            .await
            .expect("acknowledged timed gate permits claim release");

        let immediate = resolver_with_runtime(
            Arc::clone(&store),
            Arc::clone(&claim_port),
            Arc::clone(&transport),
        )
        .resolve_with_refresh::<TestCred>(&selector, &ctx)
        .await
        .expect_err("another replica must honor the unexpired backend gate");
        assert!(matches!(immediate, ResolveError::RefreshNotApplied { .. }));
        assert_eq!(AFTER_PROVIDER_CALLS.load(Ordering::SeqCst), 1);

        tokio::time::sleep(Duration::from_millis(1_100)).await;

        let left = resolver_with_runtime(
            Arc::clone(&store),
            Arc::clone(&claim_port),
            Arc::clone(&transport),
        );
        let right = resolver_with_runtime(
            Arc::clone(&store),
            Arc::clone(&claim_port),
            Arc::clone(&transport),
        );
        let left_selector = selector.clone();
        let left_ctx = ctx.clone();
        let left_task = tokio::spawn(async move {
            left.resolve_with_refresh::<TestCred>(&left_selector, &left_ctx)
                .await
        });
        let right_task = tokio::spawn(async move {
            right
                .resolve_with_refresh::<TestCred>(&selector, &ctx)
                .await
        });
        let (left_result, right_result) = tokio::time::timeout(Duration::from_secs(3), async {
            tokio::join!(left_task, right_task)
        })
        .await
        .expect("both replica attempts finish under the renewed gate");
        assert!(matches!(
            left_result.expect("left task joins"),
            Err(ResolveError::RefreshNotApplied { .. })
        ));
        assert!(matches!(
            right_result.expect("right task joins"),
            Err(ResolveError::RefreshNotApplied { .. })
        ));
        assert_eq!(
            AFTER_PROVIDER_CALLS.load(Ordering::SeqCst),
            2,
            "gate expiry admits exactly one new provider dispatch across replicas"
        );
    }

    #[tokio::test]
    async fn service_forced_oauth_refresh_uses_resolver_transport_and_exact_k2_disposition() {
        let scope = crate::TenantScope::new("test-org", "test-workspace");
        let store = Arc::new(ScriptedStore::with_owner(
            oauth2_row(Some("refresh-grant")),
            false,
            CredentialOwner::from_canonical(scope.owner_id()),
        ));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(
            OAuthTransportResult::InvalidClient401,
        ));
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = transport.clone();
        let (service, shutdown) =
            oauth_service_with_runtime(Arc::clone(&store), claim_port, transport_port);

        let error = service
            .refresh(&scope, &test_id().to_string())
            .await
            .expect_err("definitive OAuth denial must remain exact through the service");
        shutdown.cancel();

        let crate::CredentialServiceError::RefreshNotApplied(context) = error else {
            panic!("service must preserve the proof-bearing refresh context");
        };
        assert_eq!(transport.call_count(), 1);
        assert_eq!(context.retry(), crate::error::RetryAdvice::Never);
        assert_eq!(
            context
                .diagnostic_code()
                .map(crate::RefreshDiagnosticCode::as_str),
            Some("invalid_client")
        );
        assert_eq!(store.replacement_count(), 1);
        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_release_count(1))
            .await
            .expect("exact management refresh failure should release L2");
    }

    #[tokio::test]
    async fn service_forced_oauth_transient_response_is_unknown_and_retains_l2() {
        let scope = crate::TenantScope::new("test-org", "test-workspace");
        let store = Arc::new(ScriptedStore::with_owner(
            oauth2_row(Some("refresh-grant")),
            false,
            CredentialOwner::from_canonical(scope.owner_id()),
        ));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(
            OAuthTransportResult::ServerError503,
        ));
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = transport.clone();
        let (service, shutdown) =
            oauth_service_with_runtime(Arc::clone(&store), claim_port, transport_port);

        let error = service
            .refresh(&scope, &test_id().to_string())
            .await
            .expect_err("transient provider response must remain replay-ambiguous");
        shutdown.cancel();

        assert!(matches!(
            error,
            crate::CredentialServiceError::OutcomeUnknown
        ));
        assert_eq!(transport.call_count(), 1);
        assert_eq!(store.replacement_count(), 0);
        assert!(claims.active.load(Ordering::SeqCst));
        assert_eq!(claims.release_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn service_pre_dispatch_deserialize_failure_releases_l2_without_transport() {
        let scope = crate::TenantScope::new("test-org", "test-workspace");
        let store = Arc::new(ScriptedStore::with_owner(
            corrupt_oauth2_row(),
            false,
            CredentialOwner::from_canonical(scope.owner_id()),
        ));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(OAuthTransportResult::Success));
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = transport.clone();
        let (service, shutdown) =
            oauth_service_with_runtime(Arc::clone(&store), claim_port, transport_port);

        let error = service
            .refresh(&scope, &test_id().to_string())
            .await
            .expect_err("corrupt state must fail before provider dispatch");
        shutdown.cancel();

        assert!(matches!(error, crate::CredentialServiceError::Internal(_)));
        assert_eq!(transport.call_count(), 0);
        assert_eq!(store.replacement_count(), 0);
        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_release_count(1))
            .await
            .expect("pre-dispatch deserialize failure should release L2");
    }

    #[tokio::test]
    async fn oauth_success_parse_failure_is_outcome_unknown_and_retains_l2() {
        let store = Arc::new(ScriptedStore::new(
            oauth2_row(Some("rotating-grant")),
            false,
        ));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(
            OAuthTransportResult::MalformedSuccess,
        ));
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = transport.clone();
        let resolver = resolver_with_runtime(Arc::clone(&store), claim_port, transport_port);

        let error = resolver
            .resolve_with_refresh::<OAuth2Credential>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("an unusable 2xx response cannot prove provider state");

        assert!(matches!(
            &error,
            ResolveError::ProviderOutcomeUnknown { .. }
        ));
        assert_eq!(transport.call_count(), 1);
        assert_eq!(store.replacement_count(), 0);
        assert!(claims.active.load(Ordering::SeqCst));
        assert_eq!(claims.release_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn oauth_post_provider_persistence_failure_is_replay_unsafe() {
        use nebula_error::Classify;

        let store = Arc::new(ScriptedStore::failing_replace(
            oauth2_row(Some("rotating-grant")),
            CredentialPersistenceError::Unavailable,
        ));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(OAuthTransportResult::Success));
        let claim_port: Arc<dyn RefreshClaimStore> = claims.clone();
        let transport_port: Arc<dyn RefreshTransport> = transport.clone();
        let resolver = resolver_with_runtime(Arc::clone(&store), claim_port, transport_port);

        let error = resolver
            .resolve_with_refresh::<OAuth2Credential>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("persistence after provider success must not become replay-safe");

        assert!(matches!(
            &error,
            ResolveError::PostProviderPersistence {
                source: CredentialPersistenceError::Unavailable,
                ..
            }
        ));
        assert_eq!(transport.call_count(), 1);
        assert_eq!(store.replacement_count(), 1);
        assert!(
            claims.active.load(Ordering::SeqCst),
            "post-provider persistence failure must retain fail-closed L2 poison"
        );
        assert_eq!(claims.release_count.load(Ordering::SeqCst), 0);
        assert!(!resolve_error_to_credential_error(error).is_retryable());
    }

    #[tokio::test]
    async fn gated_resolver_refuses_resolution_at_the_tail() {
        // Q10 structural: a resolver gated for an external (unwired) source
        // refuses to read local bytes on EVERY resolution path — even though
        // the store holds a live row — so the direct-resolver paths
        // (`scheme_factory` → `resolve_with_refresh`) that bypass the facade's
        // per-call check are fail-closed by construction.
        let store = Arc::new(ScriptedStore::new(live_row(), false));
        let resolver = resolver_with(Arc::clone(&store)).gate_external_source(true);

        let err = resolver
            .resolve::<TestCred>(&test_selector())
            .await
            .expect_err("a gated resolver must refuse to resolve from the local store");
        assert!(
            matches!(err, ResolveError::ExternalSourceNotWired),
            "got {err:?}"
        );
    }

    // ── FIX-1: F3 containment law enforced in release ─────────────────────

    /// A credential whose `policy()` returns a refresh kind outside its
    /// scheme family's declared `refresh_classes()`. This simulates a
    /// hand-written `CredentialLifecycle::policy` that drifted from the
    /// `AuthScheme::Family` declaration.
    struct MismatchedPolicyCred;

    /// Static-only family: declares no active refresh classes. Uses
    /// `SecretToken` as its pattern (the closest built-in to "API key").
    struct StaticOnlyFamily;

    impl SchemeFamily for StaticOnlyFamily {
        const EGRESS: &'static [EgressShape] = &[EgressShape::InlineSecret];
        fn refresh_classes() -> &'static [RefreshStrategyKind] {
            &[] // static-only: no refresh permitted
        }
        fn pattern() -> AuthPattern {
            AuthPattern::SecretToken
        }
    }

    #[derive(Debug)]
    struct StaticScheme;

    impl AuthScheme for StaticScheme {
        type Family = StaticOnlyFamily;
        fn pattern() -> AuthPattern {
            AuthPattern::SecretToken
        }
    }

    impl Credential for MismatchedPolicyCred {
        type Properties = ();
        type Scheme = StaticScheme;
        type State = TestState;

        const KEY: &'static str = "test.mismatched_policy";

        fn metadata() -> CredentialMetadata {
            CredentialMetadata::builder()
                .key(nebula_core::credential_key!("test.mismatched_policy"))
                .name("MismatchedPolicyCred")
                .description("credential whose policy drifts from its family")
                .schema(crate::schema_of::<Self::Properties>())
                .pattern(AuthPattern::SecretToken)
                .build()
                .expect("MismatchedPolicyCred metadata is valid")
        }

        fn project(_state: &TestState) -> StaticScheme {
            StaticScheme
        }

        async fn resolve(
            _values: &FieldValues,
            _ctx: &CredentialContext,
        ) -> Result<ResolveResult<TestState, ()>, CredentialError> {
            Ok(ResolveResult::Complete(TestState {
                token: "live".to_owned(),
            }))
        }
    }

    impl Refreshable for MismatchedPolicyCred {
        const REFRESH_EXECUTION_MODE: crate::RefreshExecutionMode =
            crate::RefreshExecutionMode::Local;
        const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

        async fn refresh(
            state: &mut TestState,
            attempt: RefreshAttempt<'_>,
        ) -> crate::RefreshReport {
            state.token = "refreshed".to_owned();
            attempt.local_refresh_completed()
        }
    }

    impl CredentialLifecycle for MismatchedPolicyCred {
        fn policy(_state: &TestState) -> CredentialPolicy {
            CredentialPolicy {
                // Already expired to trigger the refresh path.
                expires_at: Some(Utc::now() - chrono::Duration::minutes(5)),
                lease: None,
                // RefreshToken is NOT in StaticOnlyFamily::refresh_classes() —
                // this is the out-of-family drift the F3 guard must catch.
                refresh: RefreshStrategy::RefreshToken,
                revoke: RevokeStrategy::None,
            }
        }
    }

    fn mismatched_row() -> StoredCredential {
        let now = Utc::now();
        StoredLiveCredential::new(
            test_id(),
            None,
            MismatchedPolicyCred::KEY.to_owned(),
            serde_json::to_vec(&TestState {
                token: "live".to_owned(),
            })
            .expect("serialize test state")
            .into(),
            TestState::KIND.to_owned(),
            TestState::VERSION,
            CredentialVersion::MIN,
            CredentialMaterialEpoch::MIN,
            now,
            now,
            None,
            false,
            serde_json::Map::new(),
            None,
        )
        .expect("fixture is a valid live credential")
        .into()
    }

    /// FIX-1 regression: an out-of-family refresh kind from `policy()` must
    /// return `Err(RefreshContainmentViolation)` in ALL build profiles, not only
    /// in debug. This test runs in optimised (non-debug) semantics under the test
    /// harness and verifies the `if !permits_refresh` guard fires — if the guard
    /// were still a bare `debug_assert!`, the test would see `Ok` in release.
    #[tokio::test]
    async fn out_of_family_refresh_kind_returns_containment_violation_err() {
        let store = Arc::new(ScriptedStore::new(mismatched_row(), false));
        let resolver = resolver_with(Arc::clone(&store));
        let ctx = CredentialContext::for_owner("test-owner");

        let err = resolver
            .resolve_with_refresh::<MismatchedPolicyCred>(&test_selector(), &ctx)
            .await
            .expect_err(
                "an out-of-family refresh kind must be rejected in all build profiles \
                 (F3 containment law)",
            );

        assert!(
            matches!(err, ResolveError::RefreshContainmentViolation { .. }),
            "expected RefreshContainmentViolation, got {err:?}"
        );
        // The row must not have been written — the guard fires before any refresh.
        assert_eq!(
            store.replacement_count(),
            0,
            "no CAS write must occur when the F3 guard rejects the refresh"
        );
    }
}
