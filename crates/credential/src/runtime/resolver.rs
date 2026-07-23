//! Runtime credential resolution (ADR-0092).
//!
//! Relocated from `nebula-engine::credential::resolver` so the whole
//! credential subsystem lives in one crate. No `nebula-engine` or
//! `nebula-storage` edge — transport is injected via [`RefreshTransport`].

use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use crate::error::CredentialError;
use crate::runtime::refresh::transport::RefreshTransport;
use crate::runtime::refresh::{
    RefreshCoordinator, RefreshDisposition, RefreshError, RefreshRecheckError,
};
use crate::runtime::resolve_error::{
    ResolveError, reject_tombstoned, resolve_error_to_credential_error,
};
use crate::{
    Credential, CredentialContext, CredentialEvent, CredentialHandle, CredentialId,
    CredentialLifecycle, CredentialPersistence, CredentialPersistenceError, CredentialReplacement,
    CredentialSelector, CredentialState, Decision, LAST_VALIDATED_AT_METADATA_KEY, Refreshable,
    SchemeFactory, SchemeGuard, StoredCredential, StoredLiveCredential,
    resolve::{ReauthReason, RefreshOutcome},
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

/// Translate the only proof-bearing credential refresh failure into the
/// coordinator's replay-safe class. All other implementation-defined errors
/// fail closed because the generic resolver cannot infer whether provider
/// dispatch crossed an irreversible boundary.
fn classify_refresh_error(error: CredentialError, credential_id: &str) -> ResolveError {
    match error {
        CredentialError::RefreshFailed(context) => ResolveError::ExactRefreshFailure {
            credential_id: credential_id.to_owned(),
            context,
        },
        CredentialError::OutcomeUnknown => ResolveError::ProviderOutcomeUnknown {
            credential_id: credential_id.to_owned(),
        },
        _ => ResolveError::ProviderOutcomeUnknown {
            credential_id: credential_id.to_owned(),
        },
    }
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
                .refresh_via_coordinator::<C>(selector, &credential_id, state, stored, ctx)
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
        state: C::State,
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
        let resolver_state = state;
        let resolver_stored = stored;
        let observed_version = resolver_stored.version();
        let selector_owned = selector.clone();
        let ctx_owned = ctx.clone();

        // Sub-spec post-backoff state recheck. After the L2 backoff sleep the
        // contender's claim may have been released because their refresh
        // succeeded — in that case the credential is now fresh and we should
        // short-circuit rather than running the closure on a freshly-rotated
        // refresh_token (n8n #13088 lineage). We re-read the credential from
        // the store and apply the same `needs_refresh` predicate the parent
        // `resolve_with_refresh` used; if the credential is no longer expired,
        // return `false` so the coordinator surfaces `CoalescedByOtherReplica`.
        //
        // Sub-spec ProviderRejected gap (review feedback I1): if the
        // contender's refresh returned `ReauthRequired` it persisted
        // `reauth_required = true` on the row before releasing the L2 claim.
        // Re-running the IdP closure here would produce another
        // `invalid_grant` rejection — `O(replicas)` rate-limit / IP-ban
        // pressure on the IdP. Short-circuit to `false` so this caller also
        // surfaces `CoalescedByOtherReplica` and the application layer routes
        // the credential to interactive reauth instead.
        let store_for_recheck = Arc::clone(&self.store);
        let recheck_selector = selector.clone();
        let needs_refresh_after_backoff = move |_id: &CredentialId| {
            let store = Arc::clone(&store_for_recheck);
            let selector = recheck_selector.clone();
            async move {
                let credential_id = selector.credential_id();
                let physical = match store.get(&selector).await {
                    Ok(stored) => stored,
                    Err(e) => {
                        tracing::warn!(
                            credential_id = %credential_id,
                            ?e,
                            "post-backoff state recheck: store read failed; provider dispatch denied"
                        );
                        return Err(RefreshRecheckError::Unavailable);
                    },
                };
                let StoredCredential::Live(stored) = physical else {
                    tracing::debug!(
                        credential_id = %credential_id,
                        "post-backoff state recheck: credential is tombstoned; \
                         short-circuiting to CoalescedByOtherReplica"
                    );
                    return Ok(false);
                };
                if stored.version() != observed_version {
                    tracing::debug!(
                        credential_id = %credential_id,
                        observed_version = %observed_version,
                        current_version = %stored.version(),
                        "post-backoff state recheck: captured row is stale; re-reading through the parent path"
                    );
                    return Ok(false);
                }
                if stored.reauth_required() {
                    tracing::debug!(
                        credential_id = %credential_id,
                        "post-backoff state recheck: reauth_required=true on stored \
                         credential — short-circuiting to CoalescedByOtherReplica \
                         (sub-spec §3.6 / I1)"
                    );
                    return Ok(false);
                }
                let state: C::State = match serde_json::from_slice(stored.data()) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::warn!(
                            credential_id = %credential_id,
                            ?e,
                            "post-backoff state recheck: state decode failed; provider dispatch denied"
                        );
                        return Err(RefreshRecheckError::InvalidState);
                    },
                };
                // Mirror the parent `resolve_with_refresh` routing: re-run the
                // SAME `decide_refresh` (not an ad-hoc inline-expiry test) so a
                // leased credential with no inline `expires_at`, or a static one
                // past its re-validation floor, is still seen as needing work
                // after the backoff — otherwise the contender would surface
                // `CoalescedByOtherReplica` and the parent serves it stale. Jitter
                // is deliberately omitted here (it belongs on the initial decision
                // to de-correlate replicas at startup, not on the coalesce gate).
                Ok(C::policy(&state).decide_refresh(
                    last_validated_or_created(&stored),
                    chrono::Utc::now(),
                    <C as Refreshable>::REFRESH_POLICY.early_refresh,
                    DEFAULT_REVALIDATION_FLOOR,
                ) != Decision::Usable)
            }
        };

        let outcome: Result<Result<CredentialHandle<C::Scheme>, ResolveError>, RefreshError> =
            coord
                .refresh_coalesced(typed_id, needs_refresh_after_backoff, move || async move {
                    // The coordinator has durably marked RefreshInFlight and
                    // transferred both claim and heartbeat into this owned task
                    // before invoking us. From this point provider contact and
                    // its persistence transition cannot be cancelled by caller
                    // Drop, timeout, or heartbeat loss.
                    let result = resolver
                        .perform_refresh::<C>(
                            &selector_owned,
                            resolver_state,
                            resolver_stored,
                            &ctx_owned,
                        )
                        .await;
                    match &result {
                        Err(
                            ResolveError::PostProviderPersistence {
                                source: CredentialPersistenceError::OutcomeUnknown,
                                ..
                            }
                            | ResolveError::ProviderOutcomeUnknown { .. },
                        ) => RefreshDisposition::outcome_unknown(result),
                        Err(
                            ResolveError::PostProviderPersistence { .. }
                            | ResolveError::PostProviderStateEncoding { .. },
                        ) => RefreshDisposition::retry_unsafe(result),
                        Ok(_) | Err(ResolveError::ReauthRequired { .. }) => {
                            RefreshDisposition::state_advanced(result)
                        },
                        // Includes proof-bearing ExactRefreshFailure: provider
                        // state is known not to have advanced, so L2 can be
                        // released without poisoning subsequent attempts.
                        Err(_) => RefreshDisposition::no_state_change(result),
                    }
                })
                .await;

        match outcome {
            Ok(Ok(handle)) => {
                self.refresh_coordinator.record_success(&credential_id_text);
                Ok(CoordinatedResolve::Resolved(handle))
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
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Refreshable,
    {
        let credential_id = selector.credential_id();
        let credential_id_text = credential_id.to_string();
        let refresh_ctx = ctx
            .clone()
            .with_refresh_transport(Arc::clone(&self.transport));
        // This future already runs inside the coordinator's owned
        // provider/persistence task. Do not wrap it in a cancelling timeout:
        // dropping an HTTP future cannot prove the provider did not consume or
        // rotate the grant. The coordinator's caller-wait timeout instead
        // returns a non-retryable `RefreshOutcomePending` while this owned
        // future continues under heartbeat + L2 to an exact disposition.
        let outcome = <C as Refreshable>::refresh(&mut state, &refresh_ctx)
            .await
            .map_err(|error| classify_refresh_error(error, &credential_id_text))?;

        match outcome {
            RefreshOutcome::Refreshed => {
                // Cleartext serialization for the encrypted-at-rest store.
                let data =
                    crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&state))
                        .map_err(|e| ResolveError::PostProviderStateEncoding {
                            credential_id: credential_id_text.clone(),
                            reason: e.to_string(),
                        })?;

                // Refresh contacted the provider successfully → stamp the
                // validation time so the mandatory re-validation floor measures
                // from this real validation, not from a later display edit.
                //
                // The provider response was derived from exactly `stored`.
                // Retrying it against a newer persistence version could overwrite
                // a concurrent management replacement with stale provider state,
                // so a CAS conflict is a reconciliation boundary, not a retry.
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
                    // Clear the reauth flag on success — idempotent when
                    // already false, recovers from a stale `true` left
                    // over by a previous ReauthRequired outcome that the
                    // application has since re-authorized (sub-spec / I1).
                    false,
                    validated_metadata,
                );
                match self.store.replace(selector, replacement).await {
                    Ok(_) => {
                        self.emit_refreshed(credential_id);
                        let scheme = C::project(&state);
                        Ok(self.materialize_handle::<C>(selector, scheme))
                    },
                    Err(conflict @ CredentialPersistenceError::VersionConflict { actual, .. }) => {
                        tracing::warn!(
                            credential_id = %credential_id,
                            expected = %expected_version,
                            actual = %actual,
                            "CAS conflict on refresh write; refusing to replay stale provider state"
                        );
                        Err(ResolveError::PostProviderPersistence {
                            credential_id: credential_id_text,
                            source: conflict,
                        })
                    },
                    Err(error) => Err(ResolveError::PostProviderPersistence {
                        credential_id: credential_id_text,
                        source: error,
                    }),
                }
            },
            RefreshOutcome::ReauthRequired(reason) => {
                // Persist `reauth_required = true` on the credential row
                // BEFORE returning the typed error (sub-spec / I1).
                // Cross-replica readers consult this flag in their
                // post-backoff state-recheck predicate; without the
                // persisted bit, every replica would re-run the IdP
                // closure and produce another invalid_grant rejection.
                //
                // The provider rejection was derived from exactly `stored`.
                // A conflict means a newer row may already contain a successful
                // refresh or operator replacement. Replaying the stale rejection
                // would incorrectly mark that newer state as requiring reauth.
                let expected_version = stored.version();
                let replacement = CredentialReplacement::new(
                    expected_version,
                    stored.data().clone(),
                    stored.state_kind().to_owned(),
                    stored.state_version(),
                    stored.name().map(str::to_owned),
                    stored.expires_at(),
                    true,
                    stored.metadata().clone(),
                );
                match self.store.replace(selector, replacement).await {
                    Ok(_) => {},
                    Err(conflict @ CredentialPersistenceError::VersionConflict { actual, .. }) => {
                        tracing::warn!(
                            credential_id = %credential_id,
                            expected = %expected_version,
                            actual = %actual,
                            "CAS conflict while persisting reauth_required=true; refusing to replay stale provider state"
                        );
                        return Err(ResolveError::PostProviderPersistence {
                            credential_id: credential_id_text,
                            source: conflict,
                        });
                    },
                    Err(error) => {
                        // The reauth decision is replica-visible only after an
                        // acknowledged durable write. Surfacing the provider
                        // result on any persistence failure would release the
                        // refresh claim while leaving another replica free to
                        // re-POST the same known-dead grant.
                        tracing::warn!(
                            credential_id = %credential_id,
                            ?error,
                            "failed to persist reauth_required=true; failing closed"
                        );
                        return Err(ResolveError::PostProviderPersistence {
                            credential_id: credential_id_text,
                            source: error,
                        });
                    },
                }
                Err(ResolveError::ReauthRequired {
                    credential_id: credential_id_text,
                    reason,
                })
            },
            // CoalescedByOtherReplica is success — another replica refreshed
            // while we were waiting on L2. Caller re-reads state from the
            // store via the parent dispatch path. This arm reaches us from
            // the inner `RefreshOutcome` only via a future resolver path;
            // today the sub-spec coalesce is surfaced by the `RefreshError`
            // layer in `RefreshCoordinator::refresh_coalesced` (handled in
            // the outer match on `outcome` above). Keep the arm explicit so
            // adding consumers does not silently fall through.
            RefreshOutcome::CoalescedByOtherReplica => {
                let scheme = C::project(&state);
                Ok(self.materialize_handle::<C>(selector, scheme))
            },
            // `RefreshOutcome` is `#[non_exhaustive]`; this arm is required for
            // forward-compatibility with future variants. Clippy flags it
            // unreachable against current variants — that is the intent.
            #[expect(unreachable_patterns)]
            _ => {
                let scheme = C::project(&state);
                Ok(self.materialize_handle::<C>(selector, scheme))
            },
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
    #[cfg(feature = "rotation")]
    use nebula_storage_port::SecretBytes;
    use nebula_storage_port::store::{
        ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, RefreshClaimError,
        RefreshClaimStore, ReplicaId,
    };
    use nebula_storage_port::{
        CredentialAlreadyExistsKey, CredentialCommit, CredentialCreate, CredentialOwner,
        CredentialTombstone, CredentialVersion, StoredCredentialHead, StoredTombstonedCredential,
    };
    use serde::{Deserialize, Serialize};
    use zeroize::{Zeroize, ZeroizeOnDrop};

    use parking_lot::Mutex;
    #[cfg(feature = "rotation")]
    use std::sync::atomic::AtomicBool;
    #[cfg(feature = "rotation")]
    use tokio::sync::Notify;

    use super::*;
    #[cfg(feature = "rotation")]
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
    #[cfg(feature = "rotation")]
    #[derive(Default)]
    struct StatefulClaimRepo {
        active: AtomicBool,
        try_claim_count: AtomicUsize,
        release_count: AtomicUsize,
        try_claim_seen: Notify,
        release_seen: Notify,
    }

    #[cfg(feature = "rotation")]
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

    #[cfg(feature = "rotation")]
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

    /// No-op transport: with the `rotation` feature off, `perform_refresh`
    /// drives `Refreshable::refresh` directly and never performs a token POST.
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

    #[cfg(feature = "rotation")]
    #[derive(Clone, Copy)]
    enum OAuthTransportResult {
        AckLost,
        InvalidGrant,
        EndpointUnavailable,
        MalformedSuccess,
        Success,
    }

    /// Deterministic OAuth2 transport used to distinguish an ambiguous
    /// post-dispatch failure from an exact RFC 6749 provider rejection.
    #[cfg(feature = "rotation")]
    struct ScriptedOAuthTransport {
        result: OAuthTransportResult,
        calls: AtomicUsize,
    }

    #[cfg(feature = "rotation")]
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

    #[cfg(feature = "rotation")]
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
                    OAuthTransportResult::EndpointUnavailable => Ok(
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
        replace_error: Option<CredentialPersistenceError>,
        replacements: Mutex<u32>,
        gets: Mutex<u32>,
    }

    impl ScriptedStore {
        fn new(row: StoredCredential, revoke_on_first_replace: bool) -> Self {
            Self {
                owner: test_owner(),
                row: Mutex::new(row),
                revoke_on_first_replace,
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
                replace_error: Some(replace_error),
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

            if expected != current.version() {
                return Err(CredentialPersistenceError::VersionConflict {
                    expected,
                    actual: current.version(),
                });
            }

            let version = current.version().next_live()?;
            let updated_at = Utc::now();
            let committed = StoredLiveCredential::new(
                current.credential_id(),
                replacement.name().map(str::to_owned),
                current.credential_key().to_owned(),
                replacement.data().clone(),
                replacement.state_kind().to_owned(),
                replacement.state_version(),
                version,
                current.created_at(),
                updated_at,
                replacement.expires_at(),
                replacement.reauth_required(),
                replacement.metadata().clone(),
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
            *self.gets.lock() += 1;
            let row = self.row.lock().clone();
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
    static SAME_KEY_TYPED_REFRESH_CALLS: AtomicUsize = AtomicUsize::new(0);

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
            _ctx: &CredentialContext,
        ) -> Result<RefreshOutcome, CredentialError> {
            match state.token.as_str() {
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
                _ => {},
            }
            if state.token == "reject" {
                return Ok(RefreshOutcome::ReauthRequired(
                    ReauthReason::ProviderRejected,
                ));
            }
            state.token = "refreshed".to_owned();
            Ok(RefreshOutcome::Refreshed)
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
            ctx: &CredentialContext,
        ) -> Result<RefreshOutcome, CredentialError> {
            if ctx.refresh_transport().is_none() {
                return Err(CredentialError::InvalidInput(
                    "resolver did not stamp its internal refresh transport".to_owned(),
                ));
            }
            SAME_KEY_TYPED_REFRESH_CALLS.fetch_add(1, Ordering::SeqCst);
            state.token = "same-key-typed-refresh".to_owned();
            Ok(RefreshOutcome::Refreshed)
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

    #[cfg(feature = "rotation")]
    #[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
    struct PostProviderEncodingState {
        fail_serialization: bool,
    }

    #[cfg(feature = "rotation")]
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

    #[cfg(feature = "rotation")]
    impl CredentialState for PostProviderEncodingState {
        const KIND: &'static str = "post_provider_encoding_test";
        const VERSION: u32 = 1;
    }

    #[cfg(feature = "rotation")]
    struct PostProviderEncodingCred;

    #[cfg(feature = "rotation")]
    static ENCODING_PROVIDER_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[cfg(feature = "rotation")]
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

    #[cfg(feature = "rotation")]
    impl Refreshable for PostProviderEncodingCred {
        async fn refresh(
            state: &mut PostProviderEncodingState,
            _ctx: &CredentialContext,
        ) -> Result<RefreshOutcome, CredentialError> {
            ENCODING_PROVIDER_CALLS.fetch_add(1, Ordering::SeqCst);
            state.fail_serialization = true;
            Ok(RefreshOutcome::Refreshed)
        }
    }

    #[cfg(feature = "rotation")]
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
            now,
            now,
            None,
            false,
            serde_json::Map::new(),
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
            now,
            now,
            Some(now - chrono::Duration::minutes(5)),
            false,
            serde_json::Map::new(),
        )
        .expect("fixture is a valid same-key credential")
        .into()
    }

    #[cfg(feature = "rotation")]
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
            now,
            now,
            Some(now - chrono::Duration::minutes(5)),
            false,
            serde_json::Map::new(),
        )
        .expect("fixture is a valid encoding-failure credential")
        .into()
    }

    #[cfg(feature = "rotation")]
    fn oauth2_row(refresh_token: Option<&str>) -> StoredCredential {
        oauth2_row_with_token_url(refresh_token, "https://provider.example/token")
    }

    #[cfg(feature = "rotation")]
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
            now,
            now,
            Some(expires_at),
            false,
            serde_json::Map::new(),
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
            now,
            now,
            None,
            true,
            serde_json::Map::new(),
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

    #[cfg(feature = "rotation")]
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

    // ── Regressions ────────────────────────────────────────────────────

    #[test]
    fn generic_refresh_errors_fail_closed_unless_they_carry_exact_proof() {
        let opaque = classify_refresh_error(
            CredentialError::InvalidInput("custom credential failure".to_owned()),
            "cred_x",
        );
        assert!(matches!(
            opaque,
            ResolveError::ProviderOutcomeUnknown { .. }
        ));

        let exact = classify_refresh_error(
            CredentialError::RefreshFailed(Box::new(crate::error::RefreshFailedContext::new(
                crate::error::RefreshErrorKind::ProtocolError,
                crate::error::RetryAdvice::Never,
                crate::error::SecretFreeMessage::new(
                    "custom credential proved no provider state transition",
                ),
            ))),
            "cred_x",
        );
        let ResolveError::ExactRefreshFailure { context, .. } = exact else {
            panic!("proof-bearing refresh error must remain exact");
        };
        assert_eq!(context.retry(), crate::error::RetryAdvice::Never);
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

    #[cfg(feature = "rotation")]
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
            1,
            "a version conflict must not trigger a hidden retry or reconciliation read"
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
            1,
            "a confirmed replacement must not trigger a post-write read"
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
    async fn provider_rejection_is_not_reported_without_durable_reauth_acknowledgement() {
        REAUTH_PROVIDER_CALLS.store(0, Ordering::SeqCst);
        let store = Arc::new(ScriptedStore::failing_replace(
            live_row_with_token("reject"),
            CredentialPersistenceError::Unavailable,
        ));
        let resolver = resolver_with(Arc::clone(&store));

        let error = resolver
            .resolve_with_refresh::<TestCred>(
                &test_selector(),
                &CredentialContext::for_owner("test-owner"),
            )
            .await
            .expect_err("an unpersisted reauth decision must fail closed");

        assert!(matches!(
            &error,
            ResolveError::PostProviderPersistence {
                source: CredentialPersistenceError::Unavailable,
                ..
            }
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
        use nebula_error::Classify;
        let mapped = resolve_error_to_credential_error(error);
        assert!(
            !mapped.is_retryable(),
            "an unpersisted provider rejection must not POST the dead grant again"
        );
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

    #[cfg(feature = "rotation")]
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

    #[cfg(feature = "rotation")]
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

    #[cfg(feature = "rotation")]
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

    #[cfg(feature = "rotation")]
    #[tokio::test]
    async fn oauth_pre_dispatch_rejection_is_exact_never_retry_and_releases_l2() {
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

        assert!(matches!(&error, ResolveError::ExactRefreshFailure { .. }));
        assert_eq!(transport.call_count(), 0);
        assert_eq!(store.replacement_count(), 0);
        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_release_count(1))
            .await
            .expect("a proven pre-dispatch failure should release L2");

        let CredentialError::RefreshFailed(context) = resolve_error_to_credential_error(error)
        else {
            panic!("pre-dispatch failure must retain typed refresh context");
        };
        assert_eq!(
            context.kind(),
            crate::error::RefreshErrorKind::ProtocolError
        );
        assert_eq!(context.retry(), crate::error::RetryAdvice::Never);
    }

    #[cfg(feature = "rotation")]
    #[tokio::test]
    async fn oauth_complete_endpoint_rejection_preserves_bounded_retry_advice() {
        let store = Arc::new(ScriptedStore::new(oauth2_row(Some("refresh-grant")), false));
        let claims = Arc::new(StatefulClaimRepo::default());
        let transport = Arc::new(ScriptedOAuthTransport::new(
            OAuthTransportResult::EndpointUnavailable,
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
            .expect_err("complete 503 response must be an exact no-state-change failure");

        assert!(matches!(&error, ResolveError::ExactRefreshFailure { .. }));
        assert_eq!(transport.call_count(), 1);
        assert_eq!(store.replacement_count(), 0);
        tokio::time::timeout(Duration::from_secs(1), claims.wait_for_release_count(1))
            .await
            .expect("a complete provider rejection should release L2");

        let CredentialError::RefreshFailed(context) = resolve_error_to_credential_error(error)
        else {
            panic!("endpoint rejection must retain typed refresh context");
        };
        assert_eq!(
            context.kind(),
            crate::error::RefreshErrorKind::ProviderUnavailable
        );
        assert_eq!(
            context.retry(),
            crate::error::RetryAdvice::After(RefreshPolicy::DEFAULT.min_retry_backoff)
        );
    }

    #[cfg(feature = "rotation")]
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

    #[cfg(feature = "rotation")]
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
        const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

        async fn refresh(
            state: &mut TestState,
            _ctx: &CredentialContext,
        ) -> Result<RefreshOutcome, CredentialError> {
            state.token = "refreshed".to_owned();
            Ok(RefreshOutcome::Refreshed)
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
            now,
            now,
            None,
            false,
            serde_json::Map::new(),
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
