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
    CoordinatedRefreshResult, ReauthWrite, RefreshCoordinator, RefreshDisposition, RefreshError,
    RefreshRecheck, RefreshRecheckError, RetryGateWrite, context_from_block,
    persist_reauth_required, persist_retry_gate, write_refreshed,
};
use crate::runtime::resolve_error::{
    ResolveError, envelope_error_to_resolve_error, reject_tombstoned,
    resolve_error_to_credential_error,
};
use crate::state_envelope::{decode_state_payload, encode_state_payload};
use crate::{
    Credential, CredentialContext, CredentialEvent, CredentialHandle, CredentialId,
    CredentialLifecycle, CredentialMaterialTransition, CredentialPersistence,
    CredentialPersistenceError, CredentialReplacement, CredentialSelector, CredentialState,
    Decision, LAST_VALIDATED_AT_METADATA_KEY, RefreshAttempt, RefreshNotAppliedContext,
    RefreshRetryAdmission, Refreshable, SchemeFactory, SchemeGuard, StateWireFingerprint,
    StoredCredential, StoredLiveCredential,
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

use crate::runtime::availability::{
    REFRESH_JOIN_FIRST_PAUSE, REFRESH_JOIN_MAX_PAUSE, REFRESH_JOIN_WAIT,
};

/// Whether new use of a loaded credential may proceed.
enum MaterialAdmission {
    /// The loaded material is current and admitted.
    Available,
    /// A joined refresh committed newer material; the caller must re-read.
    Superseded,
}

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
        C::State: StateWireFingerprint,
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
        C::State: StateWireFingerprint,
    {
        self.ensure_source_wired()?;
        let credential_id = selector.credential_id();
        let (stored, status) = self.load_and_verify::<C>(selector).await?;
        Self::admit_status(selector, &stored, status)?;
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
        C::State: StateWireFingerprint,
    {
        self.ensure_source_wired()?;
        // The port applies the complete owner-bound selector before returning a
        // physical record, closing the cross-tenant existence oracle before
        // this code can inspect lifecycle state or kind.
        let (physical, status) = self
            .store
            .get_with_operation_status(selector)
            .await
            .map_err(ResolveError::Store)?;
        reject_tombstoned(&physical)?;
        let StoredCredential::Live(stored) = physical else {
            return Err(ResolveError::Store(CredentialPersistenceError::NotFound));
        };
        let status = status.ok_or(ResolveError::Store(
            CredentialPersistenceError::CorruptRecord,
        ))?;
        Self::admit_status(selector, &stored, status)?;
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
    /// Coalescing requires an authoritative re-read before classifying the result.
    pub async fn resolve_with_refresh<C>(
        &self,
        selector: &CredentialSelector,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        S: 'static,
        C: Refreshable + CredentialLifecycle,
        C::State: StateWireFingerprint,
    {
        let mut coordinated = false;
        let result = async {
            self.ensure_source_wired()?;
            let mut reevaluation = 0;
            loop {
                let credential_id = selector.credential_id();
                let credential_id_text = credential_id.to_string();
                let (stored, status) = self.load_and_verify::<C>(selector).await?;
                if stored.reauth_required() {
                    Self::admit_status(selector, &stored, status)?;
                    reject_persisted_reauth(&stored)?;
                }
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
                    match self.join_in_flight_refresh(selector, &stored, status).await? {
                        MaterialAdmission::Available => {
                            let scheme = C::project(&state);
                            return Ok(self.materialize_handle::<C>(selector, scheme));
                        },
                        MaterialAdmission::Superseded
                            if reevaluation >= MAX_COORDINATED_REEVALUATIONS =>
                        {
                            return Err(ResolveError::Refresh {
                                credential_id: credential_id_text,
                                reason: "credential state kept changing during a joined refresh"
                                    .to_owned(),
                            });
                        },
                        MaterialAdmission::Superseded => {
                            reevaluation += 1;
                            continue;
                        },
                    }
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
                    Self::admit_status(selector, &stored, status)?;
                    let scheme = C::project(&state);
                    return Ok(self.materialize_handle::<C>(selector, scheme));
                }

                coordinated = true;
                match self
                    .refresh_via_coordinator::<C>(selector, &credential_id, stored, ctx)
                    .await?
                {
                    CoordinatedResolve::Resolved(handle) => return Ok(handle),
                    CoordinatedResolve::Reevaluate if reevaluation >= MAX_COORDINATED_REEVALUATIONS => {
                        return Err(ResolveError::Refresh {
                            credential_id: credential_id_text,
                            reason: "credential state kept changing during coordinated refresh"
                                .to_owned(),
                        });
                    },
                    CoordinatedResolve::Reevaluate => {
                        reevaluation += 1;
                        continue;
                    },
                }
            }
        }.await;
        if coordinated {
            let outcome = match &result {
                Ok(_) => CoordinatedRefreshResult::Success,
                Err(ResolveError::ReauthRequired { .. }) => {
                    CoordinatedRefreshResult::ReauthRequired
                },
                Err(ResolveError::RefreshNotApplied { .. }) => CoordinatedRefreshResult::NotApplied,
                Err(
                    ResolveError::ProviderOutcomeUnknown { .. }
                    | ResolveError::RefreshOutcomePending { .. }
                    | ResolveError::Store(CredentialPersistenceError::OutcomeUnknown)
                    | ResolveError::PostProviderPersistence {
                        source: CredentialPersistenceError::OutcomeUnknown,
                        ..
                    },
                ) => CoordinatedRefreshResult::OutcomeUnknown,
                Err(_) => CoordinatedRefreshResult::Failure,
            };
            self.refresh_coordinator.metrics().record_result(outcome);
        }
        result
    }

    /// Two-tier coordinated refresh path for a typed [`CredentialId`].
    async fn refresh_via_coordinator<C>(
        &self,
        selector: &CredentialSelector,
        _typed_id: &CredentialId,
        stored: StoredLiveCredential,
        ctx: &CredentialContext,
    ) -> Result<CoordinatedResolve<C::Scheme>, ResolveError>
    where
        S: 'static,
        C: Refreshable + CredentialLifecycle,
        C::State: StateWireFingerprint,
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
                    let context = Box::new(context_from_block(block.clone()));
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
                .refresh_coalesced(selector, needs_refresh_after_backoff, move || async move {
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
                    let latest = match resolver
                        .load_and_verify::<C>(&selector_owned)
                        .await
                        .map(|(latest, _)| latest)
                    {
                        Ok(latest) => latest,
                        Err(error) => {
                            return RefreshDisposition::no_state_change(Err(error));
                        },
                    };
                    if let Err(error) = reject_persisted_reauth(&latest) {
                        return RefreshDisposition::no_state_change(Err(error));
                    }
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
            Err(RefreshError::OperationBlocked { operation }) => {
                Err(ResolveError::OperationBlocked { operation })
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

    /// Linearize new material use against the operation and aggregate in one
    /// authoritative snapshot. Existing handles retain their acquired material.
    async fn ensure_material_available(
        &self,
        selector: &CredentialSelector,
        stored: &StoredLiveCredential,
    ) -> Result<(), ResolveError> {
        let status = self.store.operation_status(selector).await?;
        Self::admit_status(selector, stored, status)
    }

    /// Admit new use of `stored` under `status`, the operation status read
    /// with it or after it. Existing handles retain their acquired material.
    fn admit_status(
        selector: &CredentialSelector,
        stored: &StoredLiveCredential,
        status: nebula_storage_port::store::CredentialOperationStatus,
    ) -> Result<(), ResolveError> {
        use nebula_storage_port::store::CredentialOperationStatus;

        match status {
            CredentialOperationStatus::InFlight { operation }
            | CredentialOperationStatus::ReconciliationRequired { operation, .. } => {
                tracing::warn!(
                    ?operation,
                    "credential projection blocked by durable operation"
                );
                Err(ResolveError::OperationBlocked { operation })
            },
            CredentialOperationStatus::Open {
                reauth_required: true,
                ..
            } => Err(ResolveError::ReauthRequired {
                credential_id: selector.credential_id().to_string(),
                reason: ReauthReason::ProviderRejected,
            }),
            CredentialOperationStatus::Open {
                version,
                material_epoch,
                ..
            } if material_epoch != stored.material_epoch() => Err(ResolveError::Store(
                CredentialPersistenceError::VersionConflict {
                    expected: stored.version(),
                    actual: version,
                },
            )),
            CredentialOperationStatus::Open { .. } => Ok(()),
        }
    }

    /// Admit new use of `stored`, joining a refresh already in flight.
    ///
    /// A refresh blocks new projections only while its provider call is
    /// outstanding. Failing every use for that window would turn each refresh
    /// into an outage for its credential, so a use that found the material
    /// usable waits for the refresh within [`REFRESH_JOIN_WAIT`]: a committed
    /// refresh supersedes `stored` and the caller re-reads it; a refresh still
    /// outstanding at the bound answers [`ResolveError::OperationBlocked`] as
    /// before. Revoke, reconciliation and reauthorization are never joined.
    async fn join_in_flight_refresh(
        &self,
        selector: &CredentialSelector,
        stored: &StoredLiveCredential,
        status: nebula_storage_port::store::CredentialOperationStatus,
    ) -> Result<MaterialAdmission, ResolveError> {
        use nebula_storage_port::store::CredentialOperationKind;

        let deadline = tokio::time::Instant::now() + REFRESH_JOIN_WAIT;
        let mut pause = REFRESH_JOIN_FIRST_PAUSE;
        let mut joined = false;
        // The first check uses the status read with `stored`; later ones re-read it.
        let mut admission = Self::admit_status(selector, stored, status);
        loop {
            match admission {
                Ok(()) => return Ok(MaterialAdmission::Available),
                Err(ResolveError::OperationBlocked {
                    operation: CredentialOperationKind::Refresh,
                }) if tokio::time::Instant::now() + pause <= deadline => {
                    joined = true;
                    tokio::time::sleep(pause).await;
                    pause = (pause * 2).min(REFRESH_JOIN_MAX_PAUSE);
                    admission = self.ensure_material_available(selector, stored).await;
                },
                Err(ResolveError::Store(CredentialPersistenceError::VersionConflict {
                    ..
                })) if joined => {
                    return Ok(MaterialAdmission::Superseded);
                },
                Err(error) => return Err(error),
            }
        }
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
                Ok(Some(Box::new(context_from_block(block.clone()))))
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
    ) -> Result<
        (
            StoredLiveCredential,
            nebula_storage_port::store::CredentialOperationStatus,
        ),
        ResolveError,
    >
    where
        C: Credential,
    {
        let credential_id = selector.credential_id();
        // One snapshot: the material and the operation status that admits its
        // use are read together, so nothing can change between the two.
        let (physical, status) = self
            .store
            .get_with_operation_status(selector)
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
        let expected_kind = <C::State as CredentialState>::KIND;
        if stored.state_kind() != expected_kind {
            return Err(ResolveError::KindMismatch {
                credential_id: credential_id.to_string(),
                expected: expected_kind.to_string(),
                actual: stored.state_kind().to_owned(),
            });
        }
        let status = status.ok_or(ResolveError::Store(
            CredentialPersistenceError::CorruptRecord,
        ))?;

        Ok((stored, status))
    }

    /// The single resolver-side decode of persisted state. Every resolution
    /// path funnels through here, and every envelope check (unsupported
    /// version, axis disagreement, kind tag, schema fingerprint) runs inside
    /// [`crate::state_envelope::decode_state_payload`] — the one fail-closed choke point
    /// (ADR-0107 Seam 2). Legacy pre-envelope rows fall back to a direct
    /// decode (the ordered migration); corrupt bytes keep their
    /// `Deserialize` classification.
    fn deserialize<C>(
        &self,
        credential_id: CredentialId,
        stored: &StoredLiveCredential,
    ) -> Result<C::State, ResolveError>
    where
        C: Credential,
        C::State: StateWireFingerprint,
    {
        let body = decode_state_payload::<C::State>(
            stored.data(),
            stored.state_kind(),
            stored.state_version(),
        )
        .map_err(|error| envelope_error_to_resolve_error(credential_id.to_string(), error))?;
        body.into_state::<C::State>()
            .map_err(|_| ResolveError::Deserialize {
                credential_id: credential_id.to_string(),
                reason: "stored credential state is invalid".to_owned(),
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
        C::State: StateWireFingerprint,
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
                match persist_retry_gate(
                    self.store.as_ref(),
                    selector,
                    stored,
                    context,
                    C::REFRESH_POLICY.min_retry_backoff,
                )
                .await
                {
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
        C::State: StateWireFingerprint,
    {
        let credential_id = selector.credential_id();
        let credential_id_text = credential_id.to_string();
        let data = match crate::serde_secret::expose_for_serialization(|| {
            encode_state_payload(&state)
        }) {
            Ok(data) => data,
            Err(_) if phase == RefreshCommitPhase::ProviderConfirmed => {
                return RefreshDisposition::retry_unsafe(Err(
                    ResolveError::PostProviderStateEncoding {
                        credential_id: credential_id_text,
                        reason: "credential state serializer rejected refreshed state".to_owned(),
                    },
                ));
            },
            Err(_) => {
                return RefreshDisposition::retry_unsafe(Err(
                    ResolveError::RefreshReconciliationRequired {
                        credential_id: credential_id_text,
                    },
                ));
            },
        };

        let now = chrono::Utc::now();
        let expected_version = stored.version();
        let data = nebula_storage_port::SecretBytes::from(data);
        let expires_at = state.expires_at();
        // Display fields come from the row the write is based on, which is
        // re-read when a rename landed during the provider call.
        let build = |base: &StoredLiveCredential| {
            let mut validated_metadata = base.metadata().clone();
            validated_metadata.insert(
                LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
                serde_json::Value::String(now.to_rfc3339()),
            );
            CredentialReplacement::new(
                base.version(),
                data.clone(),
                base.state_kind().to_owned(),
                // The row's `state_version` axis must advance to the version the
                // writing build stamped in the envelope (`interface_version` =
                // `C::State::VERSION`). Stamping the stored axis instead would
                // leave a legacy row (axis < VERSION) with an envelope whose
                // interface_version disagrees with the row — the next read
                // refuses it as VersionAxesDisagree, poisoning exactly the
                // migration path the envelope exists to serve.
                <C::State as CredentialState>::VERSION,
                base.name().map(str::to_owned),
                expires_at,
                false,
                validated_metadata,
                CredentialMaterialTransition::advance(),
            )
        };

        match write_refreshed(self.store.as_ref(), selector, &stored, build).await {
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
                RefreshDisposition::outcome_unknown(Err(ResolveError::Store(
                    CredentialPersistenceError::OutcomeUnknown,
                )))
            },
            Err(error) => RefreshDisposition::retry_unsafe(Err(ResolveError::Store(error))),
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
mod refresh_revoke_race;
