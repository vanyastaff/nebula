//! Runtime credential resolution.

use std::sync::Arc;

use nebula_credential::{
    Credential, CredentialContext, CredentialEvent, CredentialHandle, CredentialId,
    CredentialState, Refreshable,
    resolve::{ReauthReason, RefreshOutcome},
    store::{CredentialStore, PutMode, StoreError, StoredCredential},
};
#[cfg(feature = "rotation")]
use nebula_credential::{
    credentials::{OAuth2Credential, OAuth2State},
    error::CredentialError,
};
use nebula_eventbus::EventBus;

use crate::credential::refresh::{RefreshCoordinator, RefreshError};
#[cfg(feature = "rotation")]
use crate::credential::rotation::refresh_oauth2_state;

/// Runtime credential resolver with optional coordinated refresh.
pub struct CredentialResolver<S: CredentialStore> {
    store: Arc<S>,
    refresh_coordinator: Arc<RefreshCoordinator>,
    event_bus: Option<Arc<EventBus<CredentialEvent>>>,
}

impl<S: CredentialStore> CredentialResolver<S> {
    /// Create a resolver backed by the given credential store with a
    /// default in-memory `RefreshCoordinator`.
    ///
    /// Production composition (CLI / API entrypoints) should call
    /// [`Self::with_refresh_coordinator`] to wire a Postgres- or SQLite-
    /// backed `RefreshClaimRepo` so cross-replica coordination is
    /// durable. The default is suitable for tests and single-replica
    /// desktop mode.
    #[must_use]
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            refresh_coordinator: Arc::new(RefreshCoordinator::new()),
            event_bus: None,
        }
    }

    #[must_use = "builder methods must be chained or built"]
    /// Attach an event bus to emit credential refresh lifecycle events.
    pub fn with_event_bus(mut self, bus: Arc<EventBus<CredentialEvent>>) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// Replace the default in-memory refresh coordinator with a shared
    /// one.
    ///
    /// Composition root threads `Arc<RefreshCoordinator>` constructed via
    /// `RefreshCoordinator::new_with(repo, replica_id, config)` (where
    /// `repo` is a Postgres / SQLite `RefreshClaimRepo` for production)
    /// here.
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
        let stored = self.load_and_verify::<C>(credential_id).await?;
        let state: C::State = self.deserialize::<C>(credential_id, &stored)?;
        let scheme = C::project(&state);
        Ok(CredentialHandle::new(scheme, credential_id))
    }

    /// Resolve a credential and refresh it when it enters the early-refresh window.
    ///
    /// Per Tech Spec §15.4 — bound on [`Refreshable`] so a non-refreshable
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
        C: Refreshable,
    {
        let stored = self.load_and_verify::<C>(credential_id).await?;
        let state: C::State = self.deserialize::<C>(credential_id, &stored)?;

        let needs_refresh = state.expires_at().is_some_and(|exp| {
            let now = chrono::Utc::now();
            let policy = <C as Refreshable>::REFRESH_POLICY;
            let jitter = if policy.jitter > std::time::Duration::ZERO {
                let bound_ms = policy.jitter.as_millis();
                if bound_ms == 0 {
                    std::time::Duration::ZERO
                } else {
                    let upper = u64::try_from(bound_ms).unwrap_or(u64::MAX);
                    std::time::Duration::from_millis(rand::random_range(0..upper))
                }
            } else {
                std::time::Duration::ZERO
            };
            let early_with_jitter = policy.early_refresh + jitter;
            let early =
                chrono::Duration::from_std(early_with_jitter).unwrap_or(chrono::Duration::zero());
            exp - now <= early
        });

        if !needs_refresh {
            let scheme = C::project(&state);
            return Ok(CredentialHandle::new(scheme, credential_id));
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
            return Ok(CredentialHandle::new(scheme, credential_id));
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
        C: Refreshable,
    {
        // The `refresh_coalesced` user closure must yield `Result<_,
        // RefreshError>`; we wrap the inner `ResolveError` in `Ok(Err(_))`
        // so it propagates without being mistaken for a coordinator
        // failure.
        let coord = Arc::clone(&self.refresh_coordinator);
        let repo = Arc::clone(coord.repo());
        let resolver_state = state;
        let resolver_stored = stored;
        let credential_id_owned = credential_id.to_string();

        // Sub-spec §3.6 post-backoff state recheck. After the L2 backoff
        // sleep the contender's claim may have been released because
        // their refresh succeeded — in that case the credential is now
        // fresh and we should short-circuit rather than running the
        // closure on a freshly-rotated refresh_token (n8n #13088
        // lineage). We re-read the credential from the store and apply
        // the same `needs_refresh` predicate the parent
        // `resolve_with_refresh` used; if the credential is no longer
        // expired, return `false` so the coordinator surfaces
        // `CoalescedByOtherReplica`.
        //
        // Sub-spec §3.6 ProviderRejected gap (review feedback I1): if
        // the contender's refresh returned `ReauthRequired` it persisted
        // `reauth_required = true` on the row before releasing the L2
        // claim. Re-running the IdP closure here would produce another
        // `invalid_grant` rejection — `O(replicas)` rate-limit / IP-ban
        // pressure on the IdP. Short-circuit to `false` so this caller
        // also surfaces `CoalescedByOtherReplica` and the application
        // layer routes the credential to interactive reauth instead.
        let store_for_recheck = Arc::clone(&self.store);
        let recheck_credential_id = credential_id.to_string();
        let needs_refresh_after_backoff = move |_id: &CredentialId| {
            let store = Arc::clone(&store_for_recheck);
            let credential_id = recheck_credential_id.clone();
            async move {
                // On any read/decode failure, conservatively retry — the
                // L2 layer will gate further work via heartbeat / claim
                // ownership, so retrying is safe.
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
                state.expires_at().is_some_and(|exp| {
                    let now = chrono::Utc::now();
                    let policy = <C as Refreshable>::REFRESH_POLICY;
                    // Use `early_refresh` without jitter for the recheck
                    // — jitter belongs on the initial `needs_refresh`
                    // decision (de-correlate replicas at startup), not
                    // on the post-backoff coalesce gate.
                    let early = chrono::Duration::from_std(policy.early_refresh)
                        .unwrap_or(chrono::Duration::zero());
                    exp - now <= early
                })
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
            // CoalescedByOtherReplica is success — another replica
            // refreshed while we were waiting on L2. Re-read state from
            // the store.
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
        use crate::credential::refresh::RefreshAttempt;

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
        async fn try_engine_oauth2_refresh<C: Refreshable>(
            state: &mut C::State,
        ) -> Result<Option<RefreshOutcome>, CredentialError> {
            if C::KEY != OAuth2Credential::KEY {
                return Ok(None);
            }

            let mut oauth_state: OAuth2State =
                serde_json::from_value(serde_json::to_value(&*state).map_err(|e| {
                    CredentialError::Provider(format!(
                        "oauth2 refresh state serialization failed: {e}"
                    ))
                })?)
                .map_err(|e| {
                    CredentialError::Provider(format!("oauth2 refresh state decode failed: {e}"))
                })?;

            refresh_oauth2_state(&mut oauth_state)
                .await
                .map_err(|e| CredentialError::Provider(e.to_string()))?;

            *state = serde_json::from_value(serde_json::to_value(oauth_state).map_err(|e| {
                CredentialError::Provider(format!("oauth2 refresh state serialization failed: {e}"))
            })?)
            .map_err(|e| {
                CredentialError::Provider(format!("oauth2 refresh state encode failed: {e}"))
            })?;

            Ok(Some(RefreshOutcome::Refreshed))
        }

        let outcome = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            #[cfg(feature = "rotation")]
            if let Some(outcome) = try_engine_oauth2_refresh::<C>(&mut state).await? {
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
                let data = serde_json::to_vec(&state).map_err(|e| ResolveError::Refresh {
                    credential_id: credential_id.to_string(),
                    reason: format!("failed to serialize refreshed state: {e}"),
                })?;

                let mut current_version = stored.version;
                for _attempt in 0..3 {
                    let updated = StoredCredential {
                        data: data.clone(),
                        updated_at: chrono::Utc::now(),
                        expires_at: state.expires_at(),
                        // Clear the reauth flag on success — idempotent
                        // when already false, recovers from a stale
                        // `true` left over by a previous ReauthRequired
                        // outcome that the application has since
                        // re-authorized (sub-spec §3.6 / I1).
                        reauth_required: false,
                        ..stored.clone()
                    };
                    match self
                        .store
                        .put(
                            updated,
                            PutMode::CompareAndSwap {
                                expected_version: current_version,
                            },
                        )
                        .await
                    {
                        Ok(_) => {
                            if let Ok(id) = CredentialId::parse(credential_id) {
                                self.emit_refreshed(id);
                            } else {
                                tracing::warn!(
                                    credential_id,
                                    "credential ID is not a valid `CredentialId` (expected `cred_<ULID>`), refresh event not emitted",
                                );
                            }
                            let scheme = C::project(&state);
                            return Ok(CredentialHandle::new(scheme, credential_id));
                        },
                        Err(StoreError::VersionConflict { actual, .. }) => {
                            tracing::warn!(
                                credential_id,
                                expected = current_version,
                                actual,
                                "CAS conflict on refresh write, retrying with same token"
                            );
                            current_version = actual;
                            continue;
                        },
                        Err(e) => return Err(ResolveError::Store(e)),
                    }
                }
                Err(ResolveError::Store(StoreError::VersionConflict {
                    id: credential_id.to_string(),
                    expected: current_version,
                    actual: current_version,
                }))
            },
            RefreshOutcome::ReauthRequired(reason) => {
                // Persist `reauth_required = true` on the credential row
                // BEFORE returning the typed error (sub-spec §3.6 / I1).
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
                // Outcome of the best-effort `reauth_required=true`
                // persist loop. Distinguishing CAS exhaustion from a
                // transient store error matters for observability: the
                // CAS-exhausted metric (sub-spec §6) MUST count only the
                // case where every attempt lost to `VersionConflict`,
                // because that is the failure mode that produces a
                // duplicate IdP load on the next replica's recheck. A
                // transient store error is a different signal already
                // logged inside the loop body, and double-warning on the
                // post-loop branch would smear the two failure modes.
                enum PersistOutcome {
                    /// CAS landed; row now has `reauth_required = true`.
                    Persisted,
                    /// Store returned a non-CAS error; loop body has
                    /// already logged it and the typed `ReauthRequired`
                    /// is surfaced to the caller anyway.
                    OtherStoreError,
                }

                let mut current_version = stored.version;
                let mut outcome: Option<PersistOutcome> = None;
                for _attempt in 0..3 {
                    let updated = StoredCredential {
                        updated_at: chrono::Utc::now(),
                        reauth_required: true,
                        ..stored.clone()
                    };
                    match self
                        .store
                        .put(
                            updated,
                            PutMode::CompareAndSwap {
                                expected_version: current_version,
                            },
                        )
                        .await
                    {
                        Ok(_) => {
                            outcome = Some(PersistOutcome::Persisted);
                            break;
                        },
                        Err(StoreError::VersionConflict { actual, .. }) => {
                            tracing::warn!(
                                credential_id,
                                expected = current_version,
                                actual,
                                "CAS conflict while persisting reauth_required=true; retrying"
                            );
                            current_version = actual;
                            continue;
                        },
                        Err(e) => {
                            // Best-effort persist: log but still surface
                            // the typed error to the caller. The next
                            // refresh attempt will hit the same provider
                            // rejection and retry the persist.
                            tracing::warn!(
                                credential_id,
                                ?e,
                                "failed to persist reauth_required=true; surfacing ReauthRequired anyway"
                            );
                            outcome = Some(PersistOutcome::OtherStoreError);
                            break;
                        },
                    }
                }
                match outcome {
                    Some(PersistOutcome::Persisted) => {},
                    Some(PersistOutcome::OtherStoreError) => {
                        // Already logged inside the loop body; do not
                        // double-warn and do not increment the
                        // CAS-exhausted counter (different signal).
                    },
                    None => {
                        // CAS budget exhausted without committing — every
                        // attempt observed a `VersionConflict`. Without
                        // observability this is invisible: the post-backoff
                        // state-recheck on a different replica will read
                        // `reauth_required = false`, re-run the IdP closure,
                        // and produce another `invalid_grant`. Surface the
                        // failure mode at WARN; metric wiring tracked under
                        // `NEBULA_CREDENTIAL_RESOLVER_REAUTH_PERSIST_CAS_EXHAUSTED_TOTAL`
                        // (sub-spec §6) — increment lands when the resolver
                        // gains a `MetricsRegistry` handle (out of scope for
                        // this PR-583 wave 2 fix; resolver does not currently
                        // hold a metrics handle).
                        //
                        // TODO(metric): wire
                        // `NEBULA_CREDENTIAL_RESOLVER_REAUTH_PERSIST_CAS_EXHAUSTED_TOTAL`
                        // through `CredentialResolver` constructor once the
                        // resolver plumbs a shared `MetricsRegistry`.
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
            // CoalescedByOtherReplica is success — another replica
            // refreshed while we were waiting on L2. Caller re-reads
            // state from the store via the parent dispatch path. This
            // arm reaches us from the inner `RefreshOutcome` only via a
            // future resolver path; today the sub-spec §3.6 coalesce is
            // surfaced by the `RefreshError` layer in
            // `RefreshCoordinator::refresh_coalesced` (handled in the
            // outer match on `outcome` above). Keep the arm explicit so
            // adding consumers does not silently fall through.
            RefreshOutcome::CoalescedByOtherReplica => {
                let scheme = C::project(&state);
                Ok(CredentialHandle::new(scheme, credential_id))
            },
            // RefreshOutcome is `#[non_exhaustive]` — preserve the
            // wildcard so adding future variants (e.g. partial refresh)
            // does not silently break this match. Per Tech Spec §15.4
            // the `NotSupported` variant was removed because membership
            // in `Refreshable` already guarantees the credential
            // supports refresh.
            _ => {
                let scheme = C::project(&state);
                Ok(CredentialHandle::new(scheme, credential_id))
            },
        }
    }
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
    /// escalation per sub-spec §3.4.
    #[error("credential {credential_id}: re-authentication required")]
    ReauthRequired {
        /// Credential identifier.
        credential_id: String,
        /// Why re-authentication is required.
        reason: ReauthReason,
    },
}
