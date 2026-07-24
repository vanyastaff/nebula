//! Capability-operation surface of [`CredentialService`] — the
//! `test` / `refresh` / `revoke` lifecycle operations.
//!
//! Split out of `facade.rs` (behaviour-preserving code motion — no logic
//! change). Reads the same `pub(crate)` [`CredentialService`] internals
//! (`load_owned`, `owner_context`, `map_store_err`, `get`) as the rest of
//! the service; `refresh_inner` (the coordinated provider + CAS transition)
//! and `is_transient_failure` stay private to this module.

use serde_json::Value;

use crate::resolve::TestResult;
use crate::runtime::refresh::{
    ReauthWrite, RetryGateWrite, context_from_block, persist_reauth_required, persist_retry_gate,
};
use crate::runtime::{RefreshDisposition, RefreshError, RefreshRecheck, RefreshRecheckError};
use crate::{
    CredentialMaterialTransition, CredentialPersistenceError, CredentialReplacement,
    CredentialTombstone, LAST_VALIDATED_AT_METADATA_KEY, RefreshRetryAdmission, StoredCredential,
};

use super::error::CredentialServiceError;
use super::facade::{CredentialService, ManagementRefreshReport};
use super::head::CredentialHead;
use super::scope::TenantScope;

enum CoordinatedRefreshResult {
    Committed(CredentialHead),
    Reevaluate,
}

impl CredentialService {
    /// Run the credential type's provider health probe.
    ///
    /// Owner-checked first (a cross-tenant id is
    /// [`NotFound`](CredentialServiceError::NotFound), never a capability
    /// leak). If the type is not testable the call fails with
    /// [`CapabilityUnsupported`](CredentialServiceError::CapabilityUnsupported)
    /// **before** any decrypt — a static type cannot be probed.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::NotFound`] — absent or cross-tenant id.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Testable`.
    /// - [`CredentialServiceError::Provider`] — the probe itself failed.
    pub async fn test(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<TestResult, CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        if !self.registry.is_testable(stored.credential_key()) {
            return Err(CredentialServiceError::CapabilityUnsupported {
                capability: "test".to_owned(),
                key: stored.credential_key().to_owned(),
            });
        }
        let ctx = Self::owner_context(scope);
        let result = self
            .ops
            .test(stored.credential_key(), stored.data(), &ctx)
            .await?;
        tracing::info!(
            credential.id = %id,
            ok = result.is_success(),
            failure_code = ?result.failure_code(),
            "credential tested"
        );
        Ok(result)
    }

    /// Force-refresh the credential's stored state and re-persist it.
    ///
    /// Owner-checked first. Provider work, state encoding, and the versioned
    /// persistence transition run once inside the same cancel-safe
    /// [`crate::runtime::RefreshCoordinator`] boundary used by execution-time
    /// refresh. There is no generic retry around provider dispatch: an erased
    /// integration error cannot prove that a rotating grant was not consumed.
    /// If this caller performed the refresh, the resulting state is written
    /// back under compare-and-swap on the version observed at load. A
    /// concurrent refresh/update after provider success is surfaced as
    /// [`CredentialServiceError::RefreshPostProviderPersistence`] and is never
    /// blindly replayed. If the framework coordinator observes that another replica
    /// completed the refresh, this caller skips the write entirely and
    /// re-reads the now-fresher state instead of clobbering it with the
    /// unmutated local copy. On
    /// success (either path) [`CredentialObserver::on_refresh`](super::observer::CredentialObserver::on_refresh) fires and
    /// the fresh secret-free [`CredentialHead`] is returned.
    ///
    /// ## Fallback-on-interrupt
    ///
    /// A fallback is allowed only when coordination fails **before** the
    /// sentinel/provider boundary and the currently stored material remains
    /// non-expired. Errors from an erased integration after dispatch are
    /// `OutcomeUnknown` and always propagate; serving a cached head there would
    /// hide a possibly consumed rotating grant.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::NotFound`] — absent or cross-tenant id.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Refreshable`.
    /// - [`CredentialServiceError::OutcomeUnknown`] — provider/persistence
    ///   acknowledgement is ambiguous; reconcile before any retry.
    /// - [`CredentialServiceError::RefreshReconciliationRequired`] — a
    ///   concurrent winner produced an exact retry-unsafe result; its durable
    ///   state must be reconciled before another refresh.
    /// - [`CredentialServiceError::RefreshPostProviderPersistence`] — provider
    ///   refresh completed but local finalization definitely failed.
    /// - [`CredentialServiceError::RefreshNotApplied`] — linear evidence
    ///   proved that provider state did not advance; typed retry advice is
    ///   retained for an explicit caller policy.
    /// - [`CredentialServiceError::TransientProvider`] — coordination failed
    ///   before provider dispatch and stored material is expired.
    pub async fn refresh(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<ManagementRefreshReport, CredentialServiceError> {
        // Read the current head before attempting refresh. A failure proven to
        // occur before provider dispatch may fall back to still-valid material.
        // The report's `refreshed: false` keeps that fallback honest.
        let cached = self.get(scope, id).await?;

        match self.refresh_inner(scope, id).await {
            Ok(head) => Ok(ManagementRefreshReport {
                head,
                refreshed: true,
            }),
            Err(ref e) if Self::is_transient_failure(e) && !cached.is_expired() => {
                tracing::warn!(
                    credential.id = %id,
                    error = %e,
                    "credential refresh coordination failed before provider dispatch; stored material still non-expired"
                );
                Ok(ManagementRefreshReport {
                    head: cached,
                    refreshed: false,
                })
            },
            Err(e) => Err(e),
        }
    }

    /// Inner refresh: one coordinated provider call + state encoding +
    /// CAS-persist. The public [`refresh`](Self::refresh) wrapper applies
    /// fallback-on-interrupt only to failures proven to occur before provider
    /// dispatch.
    async fn refresh_inner(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<CredentialHead, CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        if !self.registry.is_refreshable(stored.credential_key()) {
            return Err(CredentialServiceError::CapabilityUnsupported {
                capability: "refresh".to_owned(),
                key: stored.credential_key().to_owned(),
            });
        }
        let credential_id = stored.credential_id();
        let observed_material_epoch = stored.material_epoch();
        let selector = scope.selector(credential_id);
        if let Some(context) = self.active_refresh_retry_context(&selector, id).await? {
            return Err(CredentialServiceError::RefreshNotApplied(context));
        }

        // A contender that slept behind another replica may proceed while the
        // durable material epoch is still current. Display-only writes advance
        // the row version but preserve that epoch, so they must not masquerade
        // as a completed refresh. A newer epoch or durable reauth decision
        // instead forces an authoritative re-read before provider dispatch.
        let store_for_recheck = self.store.clone();
        let selector_for_recheck = selector.clone();
        let needs_refresh_after_backoff = move |_id: &crate::CredentialId| {
            let store = store_for_recheck.clone();
            let selector = selector_for_recheck.clone();
            async move {
                match store.refresh_retry_snapshot(&selector).await {
                    Ok(snapshot) => {
                        if let RefreshRetryAdmission::Blocked(block) = snapshot.admission() {
                            let context = Box::new(context_from_block(block.clone()));
                            return Ok(RefreshRecheck::Suppressed(context));
                        }
                        Ok(
                            if snapshot.material_epoch() == observed_material_epoch
                                && !snapshot.reauth_required()
                            {
                                RefreshRecheck::Needed
                            } else {
                                RefreshRecheck::Satisfied
                            },
                        )
                    },
                    Err(CredentialPersistenceError::NotFound) => Ok(RefreshRecheck::Satisfied),
                    Err(CredentialPersistenceError::CorruptRecord) => {
                        Err(RefreshRecheckError::InvalidState)
                    },
                    Err(_) => Err(RefreshRecheckError::Unavailable),
                }
            }
        };

        let store = self.store.clone();
        let ops = self.ops.clone();
        let observer = self.observer.clone();
        let id_owned = id.to_owned();
        let selector_for_task = selector.clone();
        let ctx = self.resolver.refresh_context(&Self::owner_context(scope));
        let result = self
            .resolver
            .refresh_coordinator()
            .refresh_coalesced(
                &credential_id,
                needs_refresh_after_backoff,
                move || async move {
                    // Merge any display-only mutation that landed between the
                    // caller's initial read and L2 acquisition. Provider
                    // dispatch must use the latest CAS version for the same
                    // material authority; a newer epoch supersedes this
                    // attempt without contacting the provider.
                    let stored = match store.get(&selector_for_task).await {
                        Ok(StoredCredential::Live(stored)) => stored,
                        Ok(StoredCredential::Tombstoned(_))
                        | Err(CredentialPersistenceError::NotFound) => {
                            return RefreshDisposition::no_state_change(Err(
                                CredentialServiceError::NotFound {
                                    id: id_owned.clone(),
                                },
                            ));
                        },
                        Err(error) => {
                            return RefreshDisposition::no_state_change(Err(
                                Self::map_store_err_for(&id_owned, error),
                            ));
                        },
                    };
                    if stored.material_epoch() != observed_material_epoch {
                        return RefreshDisposition::state_advanced(Ok(
                            CoordinatedRefreshResult::Reevaluate,
                        ));
                    }
                    if stored.reauth_required() {
                        return RefreshDisposition::state_advanced(Err(
                            CredentialServiceError::ReauthRequired {
                                credential_id: id_owned.clone(),
                                reason: crate::ReauthReason::ProviderRejected,
                            },
                        ));
                    }

                    let outcome = ops
                        .refresh(stored.credential_key(), stored.data(), &ctx)
                        .await;

                    let (refreshed, refreshed_expires_at, commit_phase) = match outcome {
                        Ok(super::ops::RefreshExecutionResult::ReauthRequired {
                            reason,
                            phase,
                        }) => {
                            let phase_name = match phase {
                                crate::contract::RefreshReauthPhase::BeforeDispatch => {
                                    "before_dispatch"
                                },
                                crate::contract::RefreshReauthPhase::ProviderConfirmed => {
                                    "provider_confirmed"
                                },
                            };
                            return match persist_reauth_required(
                                store.as_ref(),
                                &selector_for_task,
                                stored,
                            )
                            .await
                            {
                                ReauthWrite::Applied => RefreshDisposition::state_advanced(Err(
                                    CredentialServiceError::ReauthRequired {
                                        credential_id: id_owned,
                                        reason,
                                    },
                                )),
                                ReauthWrite::OutcomeUnknown => {
                                    RefreshDisposition::outcome_unknown(Err(
                                        CredentialServiceError::OutcomeUnknown,
                                    ))
                                },
                                ReauthWrite::Superseded(error) => {
                                    tracing::warn!(
                                        credential.id = %id_owned,
                                        refresh.reauth_phase = phase_name,
                                        ?error,
                                        "failed to persist an exact reauthentication decision"
                                    );
                                    RefreshDisposition::no_state_change(Err(
                                        Self::map_store_err_for(&id_owned, error),
                                    ))
                                },
                                ReauthWrite::DefiniteFailure(error)
                                    if phase
                                        == crate::contract::RefreshReauthPhase::ProviderConfirmed =>
                                {
                                    tracing::warn!(
                                        credential.id = %id_owned,
                                        ?error,
                                        "provider-confirmed reauthentication decision was not durable; retaining claim"
                                    );
                                    RefreshDisposition::retry_unsafe(Err(
                                        CredentialServiceError::ReauthDecisionFinalization,
                                    ))
                                },
                                ReauthWrite::DefiniteFailure(error) => {
                                    RefreshDisposition::no_state_change(Err(
                                        Self::map_store_err_for(&id_owned, error),
                                    ))
                                },
                            };
                        },
                        Ok(super::ops::RefreshExecutionResult::OutcomeUnknown) => {
                            return RefreshDisposition::outcome_unknown(Err(
                                CredentialServiceError::OutcomeUnknown,
                            ));
                        },
                        Ok(super::ops::RefreshExecutionResult::PostProviderPersistence) => {
                            return RefreshDisposition::retry_unsafe(Err(
                                CredentialServiceError::RefreshPostProviderPersistence,
                            ));
                        },
                        Ok(super::ops::RefreshExecutionResult::LocalFinalizationFailed) => {
                            return RefreshDisposition::retry_unsafe(Err(
                                CredentialServiceError::RefreshReconciliationRequired,
                            ));
                        },
                        Ok(super::ops::RefreshExecutionResult::NotApplied(context)) => {
                            return match persist_retry_gate(
                                store.as_ref(),
                                &selector_for_task,
                                stored,
                                context,
                            )
                            .await
                            {
                                RetryGateWrite::Applied(context) => {
                                    RefreshDisposition::state_advanced(Err(
                                        CredentialServiceError::RefreshNotApplied(context),
                                    ))
                                },
                                RetryGateWrite::OutcomeUnknown => {
                                    RefreshDisposition::outcome_unknown(Err(
                                        CredentialServiceError::OutcomeUnknown,
                                    ))
                                },
                                RetryGateWrite::Superseded(error) => {
                                    RefreshDisposition::no_state_change(Err(
                                        Self::map_store_err_for(&id_owned, error),
                                    ))
                                },
                                RetryGateWrite::DefiniteFailure(error) => {
                                    tracing::warn!(
                                        credential.id = %id_owned,
                                        ?error,
                                        "durable refresh retry gate finalization failed; retaining claim"
                                    );
                                    RefreshDisposition::retry_unsafe(Err(
                                        CredentialServiceError::RefreshRetryGateFinalization,
                                    ))
                                },
                            };
                        },
                        Ok(super::ops::RefreshExecutionResult::PreparationFailed(error)) => {
                            return RefreshDisposition::no_state_change(Err(error));
                        },
                        Ok(super::ops::RefreshExecutionResult::Rewrote {
                            data,
                            expires_at,
                            phase,
                        }) => (data, expires_at, phase),
                        // Dispatch lookup fails before the implementation
                        // receives its linear attempt.
                        Err(
                            error @ (CredentialServiceError::TypeUnknown { .. }
                            | CredentialServiceError::CapabilityUnsupported { .. }),
                        ) => return RefreshDisposition::no_state_change(Err(error)),
                        // A future erased-op error is phase-ambiguous. It must
                        // retain L2 and surface only the conservative outcome.
                        Err(error) => {
                            tracing::warn!(
                                ?error,
                                "unexpected credential refresh error after entering coordinated task"
                            );
                            return RefreshDisposition::outcome_unknown(Err(
                                CredentialServiceError::OutcomeUnknown,
                            ));
                        },
                    };

                    let now = chrono::Utc::now();
                    let mut metadata = stored.metadata().clone();
                    metadata.insert(
                        LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
                        Value::String(now.to_rfc3339()),
                    );
                    let display = Self::display_from_metadata(&metadata);
                    let replacement = CredentialReplacement::new(
                        stored.version(),
                        refreshed.clone().into(),
                        stored.state_kind().to_owned(),
                        stored.state_version(),
                        stored.name().map(str::to_owned),
                        refreshed_expires_at,
                        false,
                        metadata,
                        CredentialMaterialTransition::advance(),
                    )
                    ;
                    let commit = match store.replace(&selector_for_task, replacement).await {
                        Ok(commit) => commit,
                        Err(CredentialPersistenceError::OutcomeUnknown)
                            if commit_phase
                                == super::ops::RefreshCommitPhase::ProviderConfirmed =>
                        {
                            return RefreshDisposition::outcome_unknown(Err(
                                CredentialServiceError::OutcomeUnknown,
                            ));
                        },
                        Err(_)
                            if commit_phase
                                == super::ops::RefreshCommitPhase::ProviderConfirmed =>
                        {
                            return RefreshDisposition::retry_unsafe(Err(
                                CredentialServiceError::RefreshPostProviderPersistence,
                            ));
                        },
                        Err(CredentialPersistenceError::OutcomeUnknown) => {
                            return RefreshDisposition::outcome_unknown(Err(
                                CredentialServiceError::OutcomeUnknown,
                            ));
                        },
                        Err(error) => {
                            return RefreshDisposition::retry_unsafe(Err(
                                Self::map_store_err_for(&id_owned, error),
                            ));
                        },
                    };

                    observer.on_refresh(&credential_id);
                    tracing::info!(credential.id = %id_owned, "credential refreshed");
                    RefreshDisposition::state_advanced(Ok(CoordinatedRefreshResult::Committed(
                        CredentialHead {
                            id: commit.credential_id().to_string(),
                            credential_key: stored.credential_key().to_owned(),
                            version: commit.version().get() as u64,
                            created_at: commit.created_at(),
                            updated_at: commit.updated_at(),
                            expires_at: refreshed_expires_at,
                            last_validated_at: Some(now),
                            reauth_required: false,
                            display,
                        },
                    )))
                },
            )
            .await;

        match result {
            Ok(Ok(CoordinatedRefreshResult::Committed(head))) => Ok(head),
            Ok(Ok(CoordinatedRefreshResult::Reevaluate)) => self.get(scope, id).await,
            Err(RefreshError::CoalescedByOtherReplica) => {
                if let Some(context) = self.active_refresh_retry_context(&selector, id).await? {
                    return Err(CredentialServiceError::RefreshNotApplied(context));
                }
                let current = self.load_owned(scope, id).await?;
                if current.reauth_required() {
                    return Err(CredentialServiceError::ReauthRequired {
                        credential_id: id.to_owned(),
                        // K2's durable flag does not yet retain the original
                        // reason; use the conservative terminal classification.
                        reason: crate::ReauthReason::ProviderRejected,
                    });
                }
                if current.material_epoch() == observed_material_epoch {
                    // L1 waiters do not receive the Winner's disposition. If
                    // material authority did not advance, the Winner may have
                    // returned an unknown/unsafe post-provider result while
                    // retaining L2. A display-only version bump is not proof
                    // of refresh and must not be reported as success.
                    return Err(CredentialServiceError::OutcomeUnknown);
                }
                self.observer.on_refresh(&credential_id);
                tracing::info!(
                    credential.id = %id,
                    "credential refresh coalesced; returning the current stored head"
                );
                self.get(scope, id).await
            },
            Ok(Err(error)) => Err(error),
            Err(RefreshError::CriticalOutcomePending) => {
                Err(CredentialServiceError::OutcomeUnknown)
            },
            Err(RefreshError::ReconciliationRequired) => {
                Err(CredentialServiceError::RefreshReconciliationRequired)
            },
            Err(RefreshError::PriorAttemptNoProgress) => Err(CredentialServiceError::Internal(
                "concurrent credential refresh completed without changing authoritative state"
                    .to_owned(),
            )),
            Err(RefreshError::RetrySuppressed(context)) => {
                Err(CredentialServiceError::RefreshNotApplied(context))
            },
            Err(
                RefreshError::ContentionExhausted
                | RefreshError::Repo(_)
                | RefreshError::ClaimLostBeforeProvider
                | RefreshError::StateRecheck(RefreshRecheckError::Unavailable),
            ) => Err(CredentialServiceError::TransientProvider(
                "credential refresh coordination failed before provider dispatch".to_owned(),
            )),
            Err(RefreshError::StateRecheck(RefreshRecheckError::InvalidState)) => {
                Err(CredentialServiceError::Internal(
                    "credential refresh state recheck failed".to_owned(),
                ))
            },
        }
    }

    async fn active_refresh_retry_context(
        &self,
        selector: &crate::CredentialSelector,
        id: &str,
    ) -> Result<Option<Box<crate::RefreshNotAppliedContext>>, CredentialServiceError> {
        match self.store.refresh_retry_snapshot(selector).await {
            Ok(snapshot) => match snapshot.admission() {
                RefreshRetryAdmission::Open => Ok(None),
                RefreshRetryAdmission::Blocked(block) => {
                    Ok(Some(Box::new(context_from_block(block.clone()))))
                },
            },
            Err(error) => Err(Self::map_store_err_for(id, error)),
        }
    }

    /// True iff this error is a transient refresh/provider failure that the
    /// fallback-on-interrupt path can swallow when cached material is still
    /// non-expired.
    ///
    /// Only [`CredentialServiceError::TransientProvider`] qualifies. The
    /// coordinated management path emits it solely for failures proven to
    /// occur before the sentinel/provider boundary. Errors returned by an
    /// erased integration after dispatch map to `OutcomeUnknown`, so this
    /// fallback can never hide an ambiguous rotating-grant result.
    #[inline]
    fn is_transient_failure(e: &CredentialServiceError) -> bool {
        matches!(e, CredentialServiceError::TransientProvider(_))
    }

    /// Revoke the credential at the provider, release any leases, and write a
    /// revoke **tombstone** over the stored row (it is not deleted).
    ///
    /// Owner-checked first. Provider-side revoke, lease release, and the
    /// version-fenced tombstone transition run once inside the cancel-safe
    /// credential mutation coordinator. An erased integration error is
    /// outcome-unknown because the trait cannot prove provider-side revocation
    /// did not happen. A local finalization failure after provider success is
    /// non-replayable
    /// [`CredentialServiceError::RevokePostProviderPersistence`].
    /// Lease release remains best-effort; the stored row then transitions into
    /// the structural tombstone state, which contains no live-only secret or
    /// metadata fields.
    /// On success [`CredentialObserver::on_revoke`](super::observer::CredentialObserver::on_revoke) fires.
    ///
    /// The row is tombstoned rather than deleted so the id cannot be
    /// resurrected and a slot binding still pointing at it resolves to a typed
    /// [`CredentialTombstoned`](super::binding::ValidatedCredentialBindingError::CredentialTombstoned)
    /// rather than a bare `NotFound`. Every management read
    /// ([`get`](Self::get)/[`list`](Self::list)/[`update`](Self::update)/
    /// [`refresh`](Self::refresh)) then treats the row as gone, so a second
    /// revoke of the same id returns
    /// [`NotFound`](CredentialServiceError::NotFound) (idempotent from the
    /// caller's view).
    ///
    /// `Revocable::revoke` receives `&mut state` and may mutate it. That
    /// mutation is intentionally **not** re-persisted: the tombstone drops the
    /// secret bytes, so there is no live state to write back — unlike
    /// [`refresh`](Self::refresh), which keeps the row and CAS-persists its
    /// mutated state.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::NotFound`] — absent, cross-tenant, or already-revoked id.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Revocable`.
    /// - [`CredentialServiceError::OutcomeUnknown`] — provider/persistence
    ///   acknowledgement is ambiguous.
    /// - [`CredentialServiceError::RevokePostProviderPersistence`] — provider
    ///   revoke completed exactly, but durable tombstone finalization failed or
    ///   a concurrent exact winner requires reconciliation.
    pub async fn revoke(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<(), CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        if !self.registry.is_revocable(stored.credential_key()) {
            return Err(CredentialServiceError::CapabilityUnsupported {
                capability: "revoke".to_owned(),
                key: stored.credential_key().to_owned(),
            });
        }
        let credential_id = stored.credential_id();
        let observed_version = stored.version();
        let selector = scope.selector(credential_id);
        let store_for_recheck = self.store.clone();
        let selector_for_recheck = selector.clone();
        let still_same_live_row = move |_id: &crate::CredentialId| {
            let store = store_for_recheck.clone();
            let selector = selector_for_recheck.clone();
            async move {
                match store.get(&selector).await {
                    Ok(StoredCredential::Live(current)) => {
                        Ok(if current.version() == observed_version {
                            RefreshRecheck::Needed
                        } else {
                            RefreshRecheck::Satisfied
                        })
                    },
                    Ok(StoredCredential::Tombstoned(_)) => Ok(RefreshRecheck::Satisfied),
                    // Never authorize provider revocation over an unverified
                    // captured row or flatten unavailability into success.
                    Err(_) => Err(RefreshRecheckError::Unavailable),
                }
            }
        };

        let store = self.store.clone();
        let ops = self.ops.clone();
        let lease = self.lease.clone();
        let observer = self.observer.clone();
        let selector_for_task = selector.clone();
        let id_owned = id.to_owned();
        let ctx = Self::owner_context(scope);
        let result = self
            .resolver
            .refresh_coordinator()
            .refresh_coalesced(&credential_id, still_same_live_row, move || async move {
                match ops
                    .revoke(stored.credential_key(), stored.data(), &ctx)
                    .await
                {
                    Ok(()) => {},
                    Err(CredentialServiceError::OutcomeUnknown) => {
                        return RefreshDisposition::outcome_unknown(Err(
                            CredentialServiceError::OutcomeUnknown,
                        ));
                    },
                    Err(CredentialServiceError::RevokePostProviderPersistence) => {
                        return RefreshDisposition::retry_unsafe(Err(
                            CredentialServiceError::RevokePostProviderPersistence,
                        ));
                    },
                    // Deserialization/type/capability failures occur
                    // before the erased provider implementation.
                    Err(error) => return RefreshDisposition::no_state_change(Err(error)),
                }

                // Best effort inside the owned section: caller Drop cannot
                // interrupt the path between provider revoke and tombstone.
                let released = lease.revoke_for_credential(credential_id).await;
                if released > 0 {
                    tracing::info!(
                        credential.id = %id_owned,
                        released,
                        "released dynamic leases for revoked credential"
                    );
                }

                match store
                    .tombstone(
                        &selector_for_task,
                        CredentialTombstone::new(stored.version()),
                    )
                    .await
                {
                    Ok(_) => {
                        observer.on_revoke(&credential_id);
                        tracing::info!(credential.id = %id_owned, "credential revoked");
                        RefreshDisposition::state_advanced(Ok(()))
                    },
                    Err(CredentialPersistenceError::OutcomeUnknown) => {
                        RefreshDisposition::outcome_unknown(Err(
                            CredentialServiceError::OutcomeUnknown,
                        ))
                    },
                    Err(_) => RefreshDisposition::retry_unsafe(Err(
                        CredentialServiceError::RevokePostProviderPersistence,
                    )),
                }
            })
            .await;

        match result {
            Ok(outcome) => outcome,
            Err(RefreshError::CoalescedByOtherReplica) => {
                // A completed winner makes revoke idempotent for concurrent
                // callers. If the row is still live, however, the winner had
                // an unsafe/unknown post-provider outcome; do not replay.
                match self.store.get(&selector).await {
                    Ok(StoredCredential::Tombstoned(_))
                    | Err(CredentialPersistenceError::NotFound) => Ok(()),
                    Ok(StoredCredential::Live(_))
                    | Err(CredentialPersistenceError::OutcomeUnknown) => {
                        Err(CredentialServiceError::OutcomeUnknown)
                    },
                    Err(error) => Err(Self::map_store_err_for(id, error)),
                }
            },
            Err(RefreshError::CriticalOutcomePending) => {
                Err(CredentialServiceError::OutcomeUnknown)
            },
            Err(RefreshError::ReconciliationRequired) => {
                Err(CredentialServiceError::RevokePostProviderPersistence)
            },
            Err(RefreshError::PriorAttemptNoProgress) => Err(CredentialServiceError::Internal(
                "concurrent credential revoke completed without changing authoritative state"
                    .to_owned(),
            )),
            Err(
                RefreshError::ContentionExhausted
                | RefreshError::Repo(_)
                | RefreshError::ClaimLostBeforeProvider
                | RefreshError::StateRecheck(RefreshRecheckError::Unavailable),
            ) => Err(CredentialServiceError::TransientProvider(
                "credential revoke coordination failed before provider dispatch".to_owned(),
            )),
            Err(RefreshError::StateRecheck(RefreshRecheckError::InvalidState)) => {
                Err(CredentialServiceError::Internal(
                    "credential revoke state recheck failed".to_owned(),
                ))
            },
            Err(RefreshError::RetrySuppressed(_)) => Err(CredentialServiceError::Internal(
                "credential revoke received an invalid refresh-only retry gate signal".to_owned(),
            )),
        }
    }
}
