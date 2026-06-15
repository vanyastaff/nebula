//! Runtime credential resolution (ADR-0092).
//!
//! Relocated from `nebula-engine::credential::resolver` so the whole
//! credential subsystem lives in one crate. No `nebula-engine` or
//! `nebula-storage` edge — transport is injected via [`RefreshTransport`].

use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

#[cfg(feature = "rotation")]
use crate::credentials::{OAuth2Credential, OAuth2State};
use crate::error::{CredentialError, ProviderErrorContext, ProviderErrorKind};
use crate::runtime::refresh::transport::RefreshTransport;
use crate::runtime::refresh::{RefreshCoordinator, RefreshError};
use crate::{
    Credential, CredentialContext, CredentialEvent, CredentialHandle, CredentialId,
    CredentialLifecycle, CredentialState, Decision, Refreshable, SchemeFactory, SchemeGuard,
    SecretFreeMessage,
    resolve::{ReauthReason, RefreshOutcome},
    store::{
        CredentialStore, LAST_VALIDATED_AT_METADATA_KEY, OWNER_ID_METADATA_KEY, OwnerScopedKey,
        PutMode, StoreError, StoredCredential,
    },
};

/// Framework-imposed mandatory re-validation floor for a refreshable credential
/// that carries neither an inline expiry nor a lease — the backstop that keeps
/// even a signal-less refreshable credential from being served indefinitely
/// without re-contacting its provider. Owner ruling: there is no "valid forever".
/// (Per-credential override is a later configuration concern; this is the default.)
const DEFAULT_REVALIDATION_FLOOR: std::time::Duration = std::time::Duration::from_hours(24);
use nebula_core::auth::{AuthScheme, SchemeFamily};
use nebula_eventbus::EventBus;
use parking_lot::Mutex;

#[cfg(feature = "rotation")]
use crate::runtime::refresh::token_refresh::refresh_oauth2_state;

type HandleCache = Mutex<HashMap<(String, TypeId), Arc<dyn Any + Send + Sync>>>;

/// Runtime credential resolver with optional coordinated refresh.
pub struct CredentialResolver<S: CredentialStore> {
    store: Arc<S>,
    refresh_coordinator: Arc<RefreshCoordinator>,
    transport: Arc<dyn RefreshTransport>,
    event_bus: Option<Arc<EventBus<CredentialEvent>>>,
    /// Live [`CredentialHandle`]s keyed by `(credential_id, scheme TypeId)` so
    /// refresh can [`CredentialHandle::replace`] in place instead of minting
    /// disconnected handles on every resolve/refresh cycle.
    handle_cache: Arc<HandleCache>,
    /// When `true`, the service is configured with an external
    /// [`StateSource`](crate::service) whose resolution bridge is not yet wired,
    /// so **every** resolution path refuses to read local bytes (fail-closed at
    /// the resolver tail — see [`gate_external_source`](Self::gate_external_source)).
    /// This closes the source gate on the direct-resolver paths
    /// (`scheme_factory` → `resolve_with_refresh`) that bypass the facade's
    /// per-call check, by construction rather than by discipline.
    external_source_unwired: bool,
}

impl<S: CredentialStore> Clone for CredentialResolver<S> {
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

impl<S: CredentialStore> CredentialResolver<S> {
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
            handle_cache: Arc::new(Mutex::new(HashMap::new())),
            external_source_unwired: false,
        }
    }

    /// Fail-closed the resolver against an external, not-yet-wired state source.
    ///
    /// Set by the composition root when the service is built with
    /// [`StateSource::External`](crate::service). Once gated, **every** resolution
    /// entry point ([`resolve`](Self::resolve) / [`resolve_scoped`](Self::resolve_scoped)
    /// / [`resolve_with_refresh`](Self::resolve_with_refresh), and therefore
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

    fn handle_cache_key<C: Credential>(credential_id: &str) -> (String, TypeId) {
        (credential_id.to_string(), TypeId::of::<C::Scheme>())
    }

    fn cached_handle<C: Credential>(
        &self,
        credential_id: &str,
    ) -> Option<CredentialHandle<C::Scheme>> {
        let key = Self::handle_cache_key::<C>(credential_id);
        self.handle_cache.lock().get(&key).and_then(|entry| {
            entry
                .clone()
                .downcast::<CredentialHandle<C::Scheme>>()
                .ok()
                .map(|arc| (*arc).clone())
        })
    }

    fn store_handle<C: Credential>(
        &self,
        credential_id: &str,
        handle: CredentialHandle<C::Scheme>,
    ) {
        let key = Self::handle_cache_key::<C>(credential_id);
        self.handle_cache.lock().insert(key, Arc::new(handle));
    }

    fn materialize_handle<C: Credential>(
        &self,
        credential_id: &str,
        scheme: C::Scheme,
    ) -> CredentialHandle<C::Scheme> {
        if let Some(existing) = self.cached_handle::<C>(credential_id) {
            existing.replace(scheme);
            return existing;
        }
        let handle = CredentialHandle::new(scheme, credential_id);
        self.store_handle::<C>(credential_id, handle.clone());
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
        credential_id: impl Into<String>,
        ctx: CredentialContext,
    ) -> SchemeFactory<C>
    where
        S: CredentialStore + 'static,
        C: Refreshable + CredentialLifecycle,
        C::Scheme: zeroize::Zeroize + Clone + Send + Sync + 'static,
    {
        let resolver = self.clone();
        let credential_id = credential_id.into();
        SchemeFactory::new(move || {
            let resolver = resolver.clone();
            let credential_id = credential_id.clone();
            let ctx = ctx.clone();
            Box::pin(async move {
                let handle = resolver
                    .resolve_with_refresh::<C>(&credential_id, &ctx)
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

    /// Resolve a credential state into a typed handle.
    pub async fn resolve<C>(
        &self,
        credential_id: &str,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Credential,
    {
        self.ensure_source_wired()?;
        let stored = self.load_and_verify::<C>(credential_id).await?;
        let state: C::State = self.deserialize::<C>(credential_id, &stored)?;
        let scheme = C::project(&state);
        Ok(self.materialize_handle::<C>(credential_id, scheme))
    }

    /// Resolve a credential for an action slot through its owner-scoped key.
    ///
    /// The [`OwnerScopedKey`] is obtainable only from a
    /// `ValidatedCredentialBinding` (whose constructor is gated by
    /// `CredentialService::validate_credential_binding`). This method
    /// **re-verifies** the loaded row's stamped `owner_id` against the key before
    /// projecting the scheme, so a credential id belonging to another tenant
    /// resolves to [`StoreError::NotFound`] (existence-hiding). The confused
    /// deputy is closed at the load — binding provenance is backed by a load-time
    /// owner check, not trusted on its own.
    ///
    /// # Errors
    ///
    /// Returns [`ResolveError::Store`] with [`StoreError::NotFound`] when the id
    /// is absent **or** the stored row's owner does not match the key; other
    /// [`ResolveError`] variants on kind-mismatch or deserialization failure.
    pub async fn resolve_scoped<C>(
        &self,
        key: &OwnerScopedKey,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Credential,
    {
        self.ensure_source_wired()?;
        // Load the raw row, then verify owner BEFORE any type/kind signal: a
        // kind-mismatch error on a foreign id would be an existence/type oracle
        // (a cross-tenant probe could distinguish "absent" from "exists but wrong
        // type"). `verify_owner` maps a foreign owner to `NotFound`
        // (existence-hiding), so the kind check below only ever runs on a row the
        // caller is entitled to — never use `load_and_verify` here (it kind-checks
        // first).
        let stored = self
            .store
            .get(key.credential_id())
            .await
            .map_err(ResolveError::Store)?;
        verify_owner(key, &stored)?;
        reject_tombstoned(key.credential_id(), &stored)?;

        let expected_kind = <C::State as CredentialState>::KIND;
        if stored.state_kind != expected_kind {
            return Err(ResolveError::KindMismatch {
                credential_id: key.credential_id().to_owned(),
                expected: expected_kind.to_string(),
                actual: stored.state_kind,
            });
        }

        let state: C::State = self.deserialize::<C>(key.credential_id(), &stored)?;
        let scheme = C::project(&state);
        Ok(self.materialize_handle::<C>(key.credential_id(), scheme))
    }

    /// Resolve a credential and refresh it when it enters the early-refresh window.
    ///
    /// Per Tech Spec — bound on [`Refreshable`] so a non-refreshable
    /// credential cannot reach this dispatch path. Probe 4
    /// (`compile_fail_engine_dispatch_capability`) cements the structural
    /// barrier with `E0277` at the dispatch site.
    ///
    /// Refresh path goes through the two-tier
    /// [`RefreshCoordinator::refresh_coalesced`] when `credential_id`
    /// parses as a typed [`CredentialId`]. Non-parseable legacy ids fall
    /// back to the L1-only coalescing path. `CoalescedByOtherReplica` is
    /// success — caller re-reads state.
    #[allow(deprecated)] // Calls deprecated `is_circuit_open` until П3 typed-id migration completes.
    pub async fn resolve_with_refresh<C>(
        &self,
        credential_id: &str,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Refreshable + CredentialLifecycle,
    {
        self.ensure_source_wired()?;
        let stored = self.load_and_verify::<C>(credential_id).await?;
        let state: C::State = self.deserialize::<C>(credential_id, &stored)?;

        // Route on the credential's own state-derived policy, not an ad-hoc
        // inline expiry test: `decide_refresh` is the single, pure, tested
        // decision. It distinguishes "expiring but nothing to renew" (serve and
        // let it ride) from "expiring and renewable" (refresh), and applies the
        // mandatory re-validation floor for a signal-less credential. Jitter is
        // deliberately not applied on this hot path — proactive jittered refresh
        // is a scheduler-seam concern, not a per-resolve one.
        let policy = C::policy(&state);

        // F3 containment law, state-level (complete): the live policy's refresh
        // kind must be one the scheme family sanctions. Registration enforces the
        // capability-level half at boot (a `Refreshable` credential on a
        // `Static`-only family is rejected); this catches a hand-written or plugin
        // policy that returns a refresh outside its family's declared classes.
        // `debug_assert` — proven for built-ins by tests + at registration, so this
        // is a dev/test net with zero release hot-path cost. `Lease` and `Watched`
        // are exempt (orthogonal lifecycle wrappers — see
        // `SchemeFamily::refresh_classes`).
        debug_assert!(
            <C::Scheme as AuthScheme>::Family::permits_refresh(policy.refresh.kind()),
            "credential '{credential_id}': policy refresh {:?} is not permitted by its \
             scheme family {:?} (refresh_classes = {:?}) — F3 containment law; the \
             credential's policy() drifted from its AuthScheme::Family declaration",
            policy.refresh.kind(),
            <C::Scheme as AuthScheme>::Family::pattern(),
            <C::Scheme as AuthScheme>::Family::refresh_classes(),
        );

        let decision = policy.decide_refresh(
            // Measure the re-validation floor from the last real provider
            // validation, NOT `updated_at` (a display-only rename/tag bumps
            // `updated_at` without revalidating — it must not postpone the floor).
            stored.last_validated_or_created(),
            chrono::Utc::now(),
            <C as Refreshable>::REFRESH_POLICY.early_refresh,
            DEFAULT_REVALIDATION_FLOOR,
        );

        if decision == Decision::Usable {
            let scheme = C::project(&state);
            return Ok(self.materialize_handle::<C>(credential_id, scheme));
        }

        if self.refresh_coordinator.is_circuit_open(credential_id) {
            let now = chrono::Utc::now();
            let truly_expired = state.expires_at().is_some_and(|exp| exp <= now);
            if truly_expired {
                tracing::warn!(
                    credential_id,
                    "circuit breaker open and token has passed its expiry; failing fast"
                );
                return Err(ResolveError::Refresh {
                    credential_id: credential_id.to_string(),
                    reason: "refresh circuit breaker open and token is expired".to_string(),
                });
            }
            tracing::warn!(
                credential_id,
                "circuit breaker open: too many refresh failures, serving stale-but-valid credential within early-refresh window"
            );
            let scheme = C::project(&state);
            return Ok(self.materialize_handle::<C>(credential_id, scheme));
        }

        // Parse the string id; the typed `CredentialId` is required by
        // the L2 claim repo. Legacy non-parseable ids (test fixtures
        // such as `"herd-cred"`) skip the L2 layer and use only the L1
        // coalescer — Stage 4 chaos test exercises the typed-id cross-
        // process L2 path.
        match CredentialId::parse(credential_id) {
            Ok(typed_id) => {
                self.refresh_via_coordinator::<C>(credential_id, &typed_id, state, stored, ctx)
                    .await
            },
            Err(_) => {
                self.refresh_via_l1_only::<C>(credential_id, state, stored, ctx)
                    .await
            },
        }
    }

    /// Two-tier coordinated refresh path (parseable [`CredentialId`]).
    #[allow(deprecated)] // Calls deprecated `record_success` / `record_failure` for L1 circuit breaker until П3.
    async fn refresh_via_coordinator<C>(
        &self,
        credential_id: &str,
        typed_id: &CredentialId,
        state: C::State,
        stored: StoredCredential,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Refreshable + CredentialLifecycle,
    {
        // The `refresh_coalesced` user closure must yield `Result<_,
        // RefreshError>`; we wrap the inner `ResolveError` in `Ok(Err(_))`
        // so it propagates without being mistaken for a coordinator failure.
        let coord = Arc::clone(&self.refresh_coordinator);
        let repo = Arc::clone(coord.repo());
        let resolver_state = state;
        let resolver_stored = stored;
        let credential_id_owned = credential_id.to_string();

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
        let recheck_credential_id = credential_id.to_string();
        let needs_refresh_after_backoff = move |_id: &CredentialId| {
            let store = Arc::clone(&store_for_recheck);
            let credential_id = recheck_credential_id.clone();
            async move {
                // On any read/decode failure, conservatively retry — the L2
                // layer will gate further work via heartbeat / claim ownership,
                // so retrying is safe.
                let stored = match store.get(&credential_id).await {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::debug!(
                            credential_id,
                            ?e,
                            "post-backoff state recheck: store read failed; retrying claim"
                        );
                        return true;
                    },
                };
                if stored.reauth_required {
                    tracing::debug!(
                        credential_id,
                        "post-backoff state recheck: reauth_required=true on stored \
                         credential — short-circuiting to CoalescedByOtherReplica \
                         (sub-spec §3.6 / I1)"
                    );
                    return false;
                }
                let state: C::State = match serde_json::from_slice(&stored.data) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::debug!(
                            credential_id,
                            ?e,
                            "post-backoff state recheck: state decode failed; retrying claim"
                        );
                        return true;
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
                C::policy(&state).decide_refresh(
                    stored.last_validated_or_created(),
                    chrono::Utc::now(),
                    <C as Refreshable>::REFRESH_POLICY.early_refresh,
                    DEFAULT_REVALIDATION_FLOOR,
                ) != Decision::Usable
            }
        };

        let outcome: Result<Result<CredentialHandle<C::Scheme>, ResolveError>, RefreshError> =
            coord
                .refresh_coalesced(typed_id, needs_refresh_after_backoff, |claim| async move {
                    // Stage 2.4 — mark sentinel = RefreshInFlight
                    // immediately before the IdP POST (perform_refresh
                    // dispatches into the OAuth2 token endpoint via
                    // refresh_oauth2_state). On the success path
                    // RefreshCoordinator::refresh_coalesced calls
                    // repo.release(token) which deletes the row,
                    // clearing the sentinel by removal — no separate
                    // clear call needed.
                    repo.mark_sentinel(&claim.token).await?;

                    Ok::<_, RefreshError>(
                        self.perform_refresh::<C>(
                            &credential_id_owned,
                            resolver_state,
                            resolver_stored,
                            ctx,
                        )
                        .await,
                    )
                })
                .await;

        match outcome {
            Ok(Ok(handle)) => {
                self.refresh_coordinator.record_success(credential_id);
                Ok(handle)
            },
            Ok(Err(e)) => {
                self.refresh_coordinator.record_failure(credential_id);
                Err(e)
            },
            // CoalescedByOtherReplica is success — another replica refreshed
            // while we were waiting on L2. Re-read state from the store.
            Err(RefreshError::CoalescedByOtherReplica) => {
                tracing::debug!(
                    credential_id,
                    "refresh coalesced by another replica; re-reading state from store"
                );
                self.refresh_coordinator.record_success(credential_id);
                self.resolve::<C>(credential_id).await
            },
            Err(e) => {
                self.refresh_coordinator.record_failure(credential_id);
                Err(ResolveError::Refresh {
                    credential_id: credential_id.to_string(),
                    reason: e.to_string(),
                })
            },
        }
    }

    /// Legacy L1-only refresh path for non-parseable string ids.
    ///
    /// Mirrors the pre-Stage-2 single-process coalescer behavior: first
    /// caller wins, others wait on a `oneshot::Receiver`, completion
    /// drains all waiters. No L2 claim is acquired.
    #[allow(deprecated)] // Whole function is the legacy-id fallback; uses deprecated L1 surface until П3.
    async fn refresh_via_l1_only<C>(
        &self,
        credential_id: &str,
        state: C::State,
        stored: StoredCredential,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Refreshable,
    {
        use crate::runtime::refresh::RefreshAttempt;

        match self.refresh_coordinator.try_refresh(credential_id) {
            RefreshAttempt::Winner => {
                let _permit = self.refresh_coordinator.acquire_permit().await;

                let credential_id_for_guard = credential_id.to_string();
                let coordinator = Arc::clone(&self.refresh_coordinator);
                let _guard = scopeguard::guard((), |()| {
                    coordinator.complete(&credential_id_for_guard);
                });
                let result = self
                    .perform_refresh::<C>(credential_id, state, stored, ctx)
                    .await;
                if result.is_ok() {
                    self.refresh_coordinator.record_success(credential_id);
                } else {
                    self.refresh_coordinator.record_failure(credential_id);
                }
                result
            },
            RefreshAttempt::Waiter(rx) => {
                if tokio::time::timeout(std::time::Duration::from_secs(5), rx)
                    .await
                    .is_err()
                {
                    tracing::debug!(
                        credential_id,
                        "refresh waiter timed out after 5s, re-reading from store"
                    );
                }
                self.resolve::<C>(credential_id).await
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
        credential_id: &str,
    ) -> Result<StoredCredential, ResolveError>
    where
        C: Credential,
    {
        let stored = self
            .store
            .get(credential_id)
            .await
            .map_err(ResolveError::Store)?;

        // Fail-closed on EVERY load path, not just the scoped one: a revoked
        // (tombstoned) row must never project to a handle. Checked before the
        // kind comparison so a tombstoned row of the wrong type maps to
        // existence-hiding `NotFound` instead of leaking a `KindMismatch`
        // oracle. This closes the tombstone half of the resurrection class for
        // `resolve` / `resolve_with_refresh` / `scheme_factory`; the owner half
        // stays on `resolve_scoped` (it requires an `OwnerScopedKey`).
        reject_tombstoned(credential_id, &stored)?;

        let expected_kind = <C::State as CredentialState>::KIND;
        if stored.state_kind != expected_kind {
            return Err(ResolveError::KindMismatch {
                credential_id: credential_id.to_string(),
                expected: expected_kind.to_string(),
                actual: stored.state_kind,
            });
        }

        Ok(stored)
    }

    fn deserialize<C>(
        &self,
        credential_id: &str,
        stored: &StoredCredential,
    ) -> Result<C::State, ResolveError>
    where
        C: Credential,
    {
        serde_json::from_slice(&stored.data).map_err(|e| ResolveError::Deserialize {
            credential_id: credential_id.to_string(),
            reason: e.to_string(),
        })
    }

    async fn perform_refresh<C>(
        &self,
        credential_id: &str,
        mut state: C::State,
        stored: StoredCredential,
        ctx: &CredentialContext,
    ) -> Result<CredentialHandle<C::Scheme>, ResolveError>
    where
        C: Refreshable,
    {
        #[cfg(feature = "rotation")]
        async fn try_oauth2_refresh<C: Refreshable>(
            state: &mut C::State,
            transport: &dyn RefreshTransport,
        ) -> Result<Option<RefreshOutcome>, CredentialError> {
            if C::KEY != OAuth2Credential::KEY {
                return Ok(None);
            }

            // Cleartext serialization for an internal full-fidelity round-trip
            // (`state` → `OAuth2State`); the intermediate `Value` is transient.
            // Outside this scope the same serialize redacts to `[REDACTED]`,
            // which `OAuth2State`'s secret fields would then ingest verbatim —
            // hence the explicit scope.
            let state_value =
                crate::serde_secret::expose_for_serialization(|| serde_json::to_value(&*state))
                    .map_err(|e| {
                        CredentialError::Provider(Box::new(ProviderErrorContext::new(
                            ProviderErrorKind::Schema,
                            SecretFreeMessage::new(format!(
                                "oauth2 refresh state serialization failed: {e}"
                            )),
                        )))
                    })?;
            let mut oauth_state: OAuth2State =
                serde_json::from_value(state_value).map_err(|e| {
                    CredentialError::Provider(Box::new(ProviderErrorContext::new(
                        ProviderErrorKind::Schema,
                        SecretFreeMessage::new(format!("oauth2 refresh state decode failed: {e}")),
                    )))
                })?;

            refresh_oauth2_state(&mut oauth_state, transport)
                .await
                .map_err(|e| {
                    CredentialError::Provider(Box::new(ProviderErrorContext::new(
                        ProviderErrorKind::ServerError,
                        SecretFreeMessage::new(e.to_string()),
                    )))
                })?;

            // Cleartext serialization for the same internal round-trip on the
            // way back (`OAuth2State` → `state`).
            let refreshed_value =
                crate::serde_secret::expose_for_serialization(|| serde_json::to_value(oauth_state))
                    .map_err(|e| {
                        CredentialError::Provider(Box::new(ProviderErrorContext::new(
                            ProviderErrorKind::Schema,
                            SecretFreeMessage::new(format!(
                                "oauth2 refresh state serialization failed: {e}"
                            )),
                        )))
                    })?;
            *state = serde_json::from_value(refreshed_value).map_err(|e| {
                CredentialError::Provider(Box::new(ProviderErrorContext::new(
                    ProviderErrorKind::Schema,
                    SecretFreeMessage::new(format!("oauth2 refresh state encode failed: {e}")),
                )))
            })?;

            Ok(Some(RefreshOutcome::Refreshed))
        }

        #[cfg_attr(not(feature = "rotation"), allow(unused_variables))]
        let transport = Arc::clone(&self.transport);
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            #[cfg(feature = "rotation")]
            if let Some(outcome) = try_oauth2_refresh::<C>(&mut state, &*transport).await? {
                return Ok(outcome);
            }
            <C as Refreshable>::refresh(&mut state, ctx).await
        })
        .await
        .map_err(|_| ResolveError::Refresh {
            credential_id: credential_id.to_string(),
            reason: "framework timeout: refresh took longer than 30s".to_string(),
        })?
        .map_err(|e| ResolveError::Refresh {
            credential_id: credential_id.to_string(),
            reason: e.to_string(),
        })?;

        match outcome {
            RefreshOutcome::Refreshed => {
                // Cleartext serialization for the encrypted-at-rest store.
                let data =
                    crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&state))
                        .map_err(|e| ResolveError::Refresh {
                            credential_id: credential_id.to_string(),
                            reason: format!("failed to serialize refreshed state: {e}"),
                        })?;

                // Refresh contacted the provider successfully → stamp the
                // validation time so the mandatory re-validation floor measures
                // from this real validation, not from a later display edit. The
                // metadata map is rebuilt from the CURRENT row on each attempt
                // (never the stale pre-refresh snapshot): a `revoke` racing this
                // write bumps the version and stamps `revoked_at`, and rebuilding
                // from `..stored.clone()` would overwrite the whole metadata
                // column on the next CAS — erasing `revoked_at` and RESURRECTING
                // a revoked credential. On conflict we re-read and abort
                // fail-closed when the row is now tombstoned.
                //
                // Bounded at 3 attempts; perform_refresh runs inside the L2
                // refresh claim, so a concurrent version bump is a revoke /
                // display-edit / reauth-flag write, never another refresh.
                let now = chrono::Utc::now();
                let mut current = stored;
                for _attempt in 0..3 {
                    let mut validated_metadata = current.metadata.clone();
                    validated_metadata.insert(
                        LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
                        serde_json::Value::String(now.to_rfc3339()),
                    );
                    let expected_version = current.version;
                    let updated = StoredCredential {
                        data: data.clone(),
                        updated_at: now,
                        expires_at: state.expires_at(),
                        // Clear the reauth flag on success — idempotent when
                        // already false, recovers from a stale `true` left
                        // over by a previous ReauthRequired outcome that the
                        // application has since re-authorized (sub-spec / I1).
                        reauth_required: false,
                        metadata: validated_metadata,
                        ..current.clone()
                    };
                    match self
                        .store
                        .put(updated, PutMode::CompareAndSwap { expected_version })
                        .await
                    {
                        Ok(_) => {
                            if let Ok(id) = CredentialId::parse(credential_id) {
                                self.emit_refreshed(id);
                            } else {
                                tracing::warn!(
                                    credential_id,
                                    "credential ID is not a valid `CredentialId` (expected \
                                     `cred_<ULID>`), refresh event not emitted",
                                );
                            }
                            let scheme = C::project(&state);
                            return Ok(self.materialize_handle::<C>(credential_id, scheme));
                        },
                        Err(StoreError::VersionConflict { actual, .. }) => {
                            tracing::warn!(
                                credential_id,
                                expected = expected_version,
                                actual,
                                "CAS conflict on refresh write; re-reading row before retry"
                            );
                            let refetched = self
                                .store
                                .get(credential_id)
                                .await
                                .map_err(ResolveError::Store)?;
                            // Fail-closed: never resurrect a revoked credential.
                            reject_tombstoned(credential_id, &refetched)?;
                            current = refetched;
                            continue;
                        },
                        Err(e) => return Err(ResolveError::Store(e)),
                    }
                }
                Err(ResolveError::Store(StoreError::VersionConflict {
                    id: credential_id.to_string(),
                    expected: current.version,
                    actual: current.version,
                }))
            },
            RefreshOutcome::ReauthRequired(reason) => {
                // Persist `reauth_required = true` on the credential row
                // BEFORE returning the typed error (sub-spec / I1).
                // Cross-replica readers consult this flag in their
                // post-backoff state-recheck predicate; without the
                // persisted bit, every replica would re-run the IdP
                // closure and produce another invalid_grant rejection.
                //
                // CAS retry loop mirrors the Refreshed-path write so we
                // do not clobber a concurrent successful refresh that
                // bumped the version while we were waiting for the IdP
                // POST. CAS conflict here means another replica already
                // committed something — retry with the new version so
                // our reauth flag is layered on the latest row.
                enum PersistOutcome {
                    /// CAS landed; row now has `reauth_required = true`.
                    Persisted,
                    /// Store returned a non-CAS error; loop body has
                    /// already logged it and the typed `ReauthRequired`
                    /// is surfaced to the caller anyway.
                    OtherStoreError,
                }

                let mut current = stored;
                let mut persist_outcome: Option<PersistOutcome> = None;
                for _attempt in 0..3 {
                    let expected_version = current.version;
                    let updated = StoredCredential {
                        updated_at: chrono::Utc::now(),
                        reauth_required: true,
                        ..current.clone()
                    };
                    match self
                        .store
                        .put(updated, PutMode::CompareAndSwap { expected_version })
                        .await
                    {
                        Ok(_) => {
                            persist_outcome = Some(PersistOutcome::Persisted);
                            break;
                        },
                        Err(StoreError::VersionConflict { actual, .. }) => {
                            tracing::warn!(
                                credential_id,
                                expected = expected_version,
                                actual,
                                "CAS conflict while persisting reauth_required=true; \
                                 re-reading row before retry"
                            );
                            let refetched = self
                                .store
                                .get(credential_id)
                                .await
                                .map_err(ResolveError::Store)?;
                            // Fail-closed: if a revoke landed while we waited on
                            // the IdP, surface `NotFound` rather than writing the
                            // reauth flag onto a tombstoned row (the CAS would
                            // clobber `revoked_at` and resurrect it).
                            reject_tombstoned(credential_id, &refetched)?;
                            current = refetched;
                            continue;
                        },
                        Err(e) => {
                            // Best-effort persist: log but still surface the
                            // typed error to the caller. The next refresh
                            // attempt will hit the same provider rejection and
                            // retry the persist.
                            tracing::warn!(
                                credential_id,
                                ?e,
                                "failed to persist reauth_required=true; surfacing ReauthRequired anyway"
                            );
                            persist_outcome = Some(PersistOutcome::OtherStoreError);
                            break;
                        },
                    }
                }
                match persist_outcome {
                    Some(PersistOutcome::Persisted | PersistOutcome::OtherStoreError) => {},
                    None => {
                        // CAS budget exhausted without committing — every attempt
                        // observed a `VersionConflict`. Without observability this
                        // is invisible: the post-backoff state-recheck on a
                        // different replica will read `reauth_required = false`,
                        // re-run the IdP closure, and produce another
                        // `invalid_grant`. Surface the failure mode at WARN.
                        tracing::warn!(
                            credential_id,
                            "reauth_required CAS exhausted after 3 attempts; next refresh will retry"
                        );
                    },
                }
                Err(ResolveError::ReauthRequired {
                    credential_id: credential_id.to_string(),
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
                Ok(self.materialize_handle::<C>(credential_id, scheme))
            },
            // `RefreshOutcome` is `#[non_exhaustive]`; this arm is required for
            // forward-compatibility with future variants. Clippy flags it
            // unreachable against current variants — that is the intent.
            #[allow(unreachable_patterns)]
            _ => {
                let scheme = C::project(&state);
                Ok(self.materialize_handle::<C>(credential_id, scheme))
            },
        }
    }
}

fn resolve_error_to_credential_error(err: ResolveError) -> CredentialError {
    CredentialError::Provider(Box::new(ProviderErrorContext::new(
        ProviderErrorKind::ServerError,
        SecretFreeMessage::new(err.to_string()),
    )))
}

/// Errors produced by [`CredentialResolver`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResolveError {
    /// Backing credential store operation failed.
    #[error("store error: {0}")]
    Store(#[from] StoreError),
    /// Stored state kind does not match the credential state type.
    #[error("credential {credential_id}: expected kind {expected}, found {actual}")]
    KindMismatch {
        /// Credential identifier.
        credential_id: String,
        /// Expected state kind.
        expected: String,
        /// Actual state kind from storage.
        actual: String,
    },
    /// Stored state bytes failed deserialization.
    #[error("credential {credential_id}: deserialize failed: {reason}")]
    Deserialize {
        /// Credential identifier.
        credential_id: String,
        /// Deserialization error message.
        reason: String,
    },
    /// Refresh path failed.
    #[error("credential {credential_id}: refresh failed: {reason}")]
    Refresh {
        /// Credential identifier.
        credential_id: String,
        /// Refresh error message.
        reason: String,
    },
    /// Credential requires full re-authentication.
    ///
    /// Carries a typed [`ReauthReason`] so callers (UI, metrics, audit)
    /// can distinguish provider-rejected refresh from sentinel-threshold
    /// escalation per sub-spec.
    #[error("credential {credential_id}: re-authentication required")]
    ReauthRequired {
        /// Credential identifier.
        credential_id: String,
        /// Why re-authentication is required.
        reason: ReauthReason,
    },
    /// The service is configured with an external [`StateSource`](crate::service)
    /// whose resolution bridge (ADR-0051) is not yet wired, so the resolver
    /// refuses to read local bytes. Fail-closed: never a silent local-store
    /// fallback. The facade maps this to
    /// `CredentialServiceError::ExternalSourceNotWired`.
    #[error("external state source is not wired; cannot resolve credential material")]
    ExternalSourceNotWired,
}

/// Fail-closed owner gate for the scoped resolution path: the loaded row's
/// stamped `owner_id` must equal the key's owner. A mismatch maps to
/// [`StoreError::NotFound`] (existence-hiding, matching the management facade) so
/// a cross-tenant probe cannot tell "absent" from "owned by another tenant". An
/// unstamped row (no `owner_id` metadata) is treated as foreign and rejected.
///
/// Complexity: O(1).
fn verify_owner(key: &OwnerScopedKey, stored: &StoredCredential) -> Result<(), ResolveError> {
    let stored_owner = stored
        .metadata
        .get(OWNER_ID_METADATA_KEY)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if stored_owner != key.owner_id() {
        return Err(ResolveError::Store(StoreError::NotFound {
            id: key.credential_id().to_owned(),
        }));
    }
    Ok(())
}

/// Fail-closed tombstone gate for the scoped resolution path.
///
/// Defence in depth for the resolve-during-revoke race:
/// `CredentialService::validate_credential_binding` already rejects a tombstoned
/// id when the binding is minted, but a binding validated immediately before a
/// concurrent `revoke` could still reach `resolve_scoped`. A revoked row is
/// mapped to [`StoreError::NotFound`] (same existence-hiding shape as
/// [`verify_owner`]) so a revoked secret is never projected to a guard.
///
/// Complexity: O(1).
fn reject_tombstoned(credential_id: &str, stored: &StoredCredential) -> Result<(), ResolveError> {
    if stored.is_tombstoned() {
        return Err(ResolveError::Store(StoreError::NotFound {
            id: credential_id.to_owned(),
        }));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored_with_owner(owner: Option<&str>) -> StoredCredential {
        let mut metadata = serde_json::Map::new();
        if let Some(o) = owner {
            metadata.insert(
                OWNER_ID_METADATA_KEY.to_owned(),
                serde_json::Value::String(o.to_owned()),
            );
        }
        StoredCredential {
            id: "cred_x".to_owned(),
            name: None,
            credential_key: "github_oauth".to_owned(),
            data: Vec::new(),
            state_kind: "oauth2_state".to_owned(),
            state_version: 1,
            version: 1,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            reauth_required: false,
            metadata,
        }
    }

    #[test]
    fn verify_owner_accepts_matching_owner() {
        let key = OwnerScopedKey::new("alice".to_owned(), "cred_x".to_owned());
        assert!(verify_owner(&key, &stored_with_owner(Some("alice"))).is_ok());
    }

    #[test]
    fn cross_tenant_load_is_not_found() {
        // Confused-deputy regression: a key for owner "bob" must not read a row
        // stamped "alice"; the load fails closed as NotFound (existence-hiding),
        // so a foreign tenant cannot even distinguish existence.
        let key = OwnerScopedKey::new("bob".to_owned(), "cred_x".to_owned());
        let err = verify_owner(&key, &stored_with_owner(Some("alice"))).unwrap_err();
        assert!(matches!(
            err,
            ResolveError::Store(StoreError::NotFound { .. })
        ));
    }

    #[test]
    fn unstamped_row_is_treated_as_foreign() {
        let key = OwnerScopedKey::new("alice".to_owned(), "cred_x".to_owned());
        let err = verify_owner(&key, &stored_with_owner(None)).unwrap_err();
        assert!(matches!(
            err,
            ResolveError::Store(StoreError::NotFound { .. })
        ));
    }

    fn tombstoned(owner: Option<&str>) -> StoredCredential {
        let mut stored = stored_with_owner(owner);
        stored.metadata.insert(
            crate::store::REVOKED_AT_METADATA_KEY.to_owned(),
            serde_json::Value::String("2026-06-13T10:00:00Z".to_owned()),
        );
        stored
    }

    #[test]
    fn tombstoned_row_is_rejected_as_not_found() {
        // Resolve-during-revoke race: a row revoked after its binding was
        // validated must not project a guard — it fails closed as NotFound,
        // never exposing the revoked secret.
        let err = reject_tombstoned("cred_x", &tombstoned(Some("alice"))).unwrap_err();
        assert!(matches!(
            err,
            ResolveError::Store(StoreError::NotFound { .. })
        ));
    }

    #[test]
    fn live_row_passes_tombstone_check() {
        assert!(reject_tombstoned("cred_x", &stored_with_owner(Some("alice"))).is_ok());
    }
}

/// FIX-1 regressions: a resolve/refresh must never project or resurrect a
/// revoked credential.
///
/// Hand-rolled test doubles of the crate's *own* ports (`CredentialStore`,
/// `RefreshClaimStore`, `RefreshTransport`) — no `nebula-storage` edge, so no
/// dependency cycle. The `ScriptedStore` simulates a `revoke` landing between
/// the resolver's load and the refresh write-back by tombstoning + version-
/// bumping the row on the first CAS `put`, which is exactly the race the
/// resurrection bug exploited.
#[cfg(test)]
mod refresh_revoke_race {
    use std::future::Future;
    use std::pin::Pin;
    use std::time::Duration;

    use chrono::{DateTime, Utc};
    use nebula_core::auth::{AuthPattern, EgressShape, RefreshStrategyKind};
    use nebula_schema::FieldValues;
    use nebula_storage_port::store::{
        ClaimAttempt, ClaimToken, HeartbeatError, ReclaimedClaim, RefreshClaimError,
        RefreshClaimStore, ReplicaId,
    };
    use serde::{Deserialize, Serialize};
    use zeroize::{Zeroize, ZeroizeOnDrop};

    use super::*;
    use crate::resolve::{RefreshPolicy, ResolveResult};
    use crate::runtime::refresh::RefreshCoordConfig;
    use crate::runtime::refresh::transport::{
        RefreshTransport, RefreshTransportError, TokenPostRequest, TokenPostResponse,
    };
    use crate::store::REVOKED_AT_METADATA_KEY;
    use crate::{CredentialMetadata, CredentialPolicy, RefreshStrategy, RevokeStrategy};

    // ── Test doubles of the crate's own ports ──────────────────────────

    /// No-op L2 claim repo: the legacy l1-only refresh path (non-parseable id)
    /// never acquires an L2 claim, so every method is unreachable. It exists
    /// solely to satisfy `RefreshCoordinator::new_with`.
    struct StubClaimRepo;

    #[async_trait::async_trait]
    impl RefreshClaimStore for StubClaimRepo {
        async fn try_claim(
            &self,
            _credential_id: &CredentialId,
            _holder: &ReplicaId,
            _ttl: Duration,
        ) -> Result<ClaimAttempt, RefreshClaimError> {
            unreachable!("l1-only refresh path never acquires an L2 claim")
        }
        async fn heartbeat(
            &self,
            _token: &ClaimToken,
            _ttl: Duration,
        ) -> Result<(), HeartbeatError> {
            unreachable!("l1-only refresh path never heartbeats an L2 claim")
        }
        async fn release(&self, _token: ClaimToken) -> Result<(), RefreshClaimError> {
            unreachable!("l1-only refresh path never releases an L2 claim")
        }
        async fn mark_sentinel(&self, _token: &ClaimToken) -> Result<(), RefreshClaimError> {
            unreachable!("l1-only refresh path never marks a sentinel")
        }
        async fn reclaim_stuck(&self) -> Result<Vec<ReclaimedClaim>, RefreshClaimError> {
            unreachable!("l1-only refresh path never sweeps claims")
        }
        async fn record_sentinel_event(
            &self,
            _credential_id: &CredentialId,
            _crashed_holder: &ReplicaId,
            _generation: u64,
        ) -> Result<(), RefreshClaimError> {
            unreachable!("l1-only refresh path never records a sentinel event")
        }
        async fn count_sentinel_events_in_window(
            &self,
            _credential_id: &CredentialId,
            _window_start: DateTime<Utc>,
        ) -> Result<u32, RefreshClaimError> {
            unreachable!("l1-only refresh path never counts sentinel events")
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

    /// In-memory single-row store. When `revoke_on_first_put` is set, the first
    /// CAS `put` tombstones + version-bumps the row and rejects the write —
    /// modelling a `revoke` that raced the refresh between load and write-back.
    struct ScriptedStore {
        row: Mutex<StoredCredential>,
        revoke_on_first_put: bool,
        puts: Mutex<u32>,
    }

    impl ScriptedStore {
        fn new(row: StoredCredential, revoke_on_first_put: bool) -> Self {
            Self {
                row: Mutex::new(row),
                revoke_on_first_put,
                puts: Mutex::new(0),
            }
        }

        fn snapshot(&self) -> StoredCredential {
            self.row.lock().clone()
        }

        fn put_count(&self) -> u32 {
            *self.puts.lock()
        }

        // Synchronous core so the trait futures hold no lock across an `.await`.
        fn put_sync(
            &self,
            credential: StoredCredential,
            mode: PutMode,
        ) -> Result<StoredCredential, StoreError> {
            let mut row = self.row.lock();
            let id = row.id.clone();
            let first = {
                let mut puts = self.puts.lock();
                *puts += 1;
                *puts == 1
            };

            let expected = match mode {
                PutMode::CompareAndSwap { expected_version } => Some(expected_version),
                PutMode::CreateOnly | PutMode::Overwrite => None,
            };

            if self.revoke_on_first_put && first {
                // A `revoke` lands between the resolver's load and this CAS:
                // bump the version and tombstone the row, then reject the write.
                row.version += 1;
                row.metadata.insert(
                    REVOKED_AT_METADATA_KEY.to_owned(),
                    serde_json::Value::String("2026-06-13T00:00:00Z".to_owned()),
                );
                return Err(StoreError::VersionConflict {
                    id,
                    expected: expected.unwrap_or(0),
                    actual: row.version,
                });
            }

            if let Some(expected_version) = expected
                && expected_version != row.version
            {
                return Err(StoreError::VersionConflict {
                    id,
                    expected: expected_version,
                    actual: row.version,
                });
            }

            let mut committed = credential;
            committed.version = row.version + 1;
            *row = committed.clone();
            Ok(committed)
        }
    }

    impl CredentialStore for ScriptedStore {
        // No `.await` in any method body, so the (`!Send`) `parking_lot` guard
        // never crosses an await point — the returned futures stay `Send`.
        async fn get(&self, _id: &str) -> Result<StoredCredential, StoreError> {
            Ok(self.row.lock().clone())
        }

        async fn put(
            &self,
            credential: StoredCredential,
            mode: PutMode,
        ) -> Result<StoredCredential, StoreError> {
            self.put_sync(credential, mode)
        }

        async fn delete(&self, _id: &str) -> Result<(), StoreError> {
            unreachable!("delete is not exercised by the resolver refresh regressions")
        }

        async fn list(&self, _state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
            Ok(Vec::new())
        }

        async fn exists(&self, _id: &str) -> Result<bool, StoreError> {
            Ok(true)
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

    // ── Fixtures ───────────────────────────────────────────────────────

    // Non-parseable id (not `cred_<ULID>`) → the resolver takes the l1-only
    // refresh path, which needs no L2 claim repo.
    const TEST_ID: &str = "test-cred";

    fn test_state_bytes() -> Vec<u8> {
        serde_json::to_vec(&TestState {
            token: "live".to_owned(),
        })
        .expect("serialize test state")
    }

    fn live_row() -> StoredCredential {
        StoredCredential {
            id: TEST_ID.to_owned(),
            name: None,
            credential_key: TestCred::KEY.to_owned(),
            data: test_state_bytes(),
            state_kind: TestState::KIND.to_owned(),
            state_version: TestState::VERSION,
            version: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            expires_at: None,
            reauth_required: false,
            metadata: serde_json::Map::new(),
        }
    }

    fn tombstoned_row() -> StoredCredential {
        let mut row = live_row();
        row.metadata.insert(
            REVOKED_AT_METADATA_KEY.to_owned(),
            serde_json::Value::String("2026-06-13T00:00:00Z".to_owned()),
        );
        row
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

    // ── Regressions ────────────────────────────────────────────────────

    #[tokio::test]
    async fn refresh_racing_revoke_does_not_resurrect() {
        let store = Arc::new(ScriptedStore::new(
            live_row(),
            /* revoke_on_first_put */ true,
        ));
        let resolver = resolver_with(Arc::clone(&store));
        let ctx = CredentialContext::for_test("test-owner");

        let err = resolver
            .resolve_with_refresh::<TestCred>(TEST_ID, &ctx)
            .await
            .expect_err("a refresh racing a revoke must fail closed, never resurrect");

        assert!(
            matches!(err, ResolveError::Store(StoreError::NotFound { .. })),
            "expected existence-hiding NotFound, got {err:?}"
        );

        let final_row = store.snapshot();
        assert!(
            final_row.is_tombstoned(),
            "the revoke tombstone must survive — a resurrection bug would erase revoked_at"
        );
        assert_eq!(
            store.put_count(),
            1,
            "must not issue a second CAS write after observing the revoke"
        );
    }

    #[tokio::test]
    async fn refresh_without_race_succeeds_and_stamps_validation() {
        let store = Arc::new(ScriptedStore::new(
            live_row(),
            /* revoke_on_first_put */ false,
        ));
        let resolver = resolver_with(Arc::clone(&store));
        let ctx = CredentialContext::for_test("test-owner");

        resolver
            .resolve_with_refresh::<TestCred>(TEST_ID, &ctx)
            .await
            .expect("an uncontended refresh must succeed");

        let final_row = store.snapshot();
        assert!(
            !final_row.is_tombstoned(),
            "an uncontended refresh must not tombstone"
        );
        assert!(
            final_row.last_validated_at().is_some(),
            "a provider-contacting refresh must stamp the re-validation anchor"
        );
        assert_eq!(
            store.put_count(),
            1,
            "exactly one CAS write on the happy path"
        );
    }

    #[tokio::test]
    async fn resolve_with_refresh_rejects_pretombstoned_row() {
        let store = Arc::new(ScriptedStore::new(tombstoned_row(), false));
        let resolver = resolver_with(Arc::clone(&store));
        let ctx = CredentialContext::for_test("test-owner");

        let err = resolver
            .resolve_with_refresh::<TestCred>(TEST_ID, &ctx)
            .await
            .expect_err("a row revoked before resolve must not refresh");

        assert!(
            matches!(err, ResolveError::Store(StoreError::NotFound { .. })),
            "got {err:?}"
        );
        assert_eq!(
            store.put_count(),
            0,
            "a tombstoned row must be rejected at load, before any write"
        );
    }

    #[tokio::test]
    async fn resolve_rejects_tombstoned_row() {
        let store = Arc::new(ScriptedStore::new(tombstoned_row(), false));
        let resolver = resolver_with(Arc::clone(&store));

        let err = resolver
            .resolve::<TestCred>(TEST_ID)
            .await
            .expect_err("a tombstoned row must not resolve to a handle");

        assert!(
            matches!(err, ResolveError::Store(StoreError::NotFound { .. })),
            "got {err:?}"
        );
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
            .resolve::<TestCred>(TEST_ID)
            .await
            .expect_err("a gated resolver must refuse to resolve from the local store");
        assert!(
            matches!(err, ResolveError::ExternalSourceNotWired),
            "got {err:?}"
        );
    }
}
