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
use crate::runtime::{RefreshDisposition, RefreshError, RefreshRecheckError};
use crate::{
    CredentialPersistenceError, CredentialReplacement, CredentialTombstone,
    LAST_VALIDATED_AT_METADATA_KEY, StoredCredential,
};

use super::error::CredentialServiceError;
use super::facade::{CredentialService, RefreshReport};
use super::head::CredentialHead;
use super::scope::TenantScope;

enum CoordinatedRefreshResult {
    Committed(CredentialHead),
    ReRead,
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
    /// [`CredentialServiceError::PostProviderPersistence`] and is never blindly
    /// replayed. If another replica coalesced the refresh
    /// (`RefreshOutcome::CoalescedByOtherReplica`) the write is **skipped
    /// entirely** and the now-fresher state is re-read from the store
    /// instead of clobbering it with the un-mutated local copy. On
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
    /// - [`CredentialServiceError::PostProviderPersistence`] — provider work
    ///   completed but local finalization definitely failed.
    /// - [`CredentialServiceError::TransientProvider`] — coordination failed
    ///   before provider dispatch and stored material is expired.
    pub async fn refresh(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<RefreshReport, CredentialServiceError> {
        // Read the current head before attempting refresh. A failure proven to
        // occur before provider dispatch may fall back to still-valid material.
        // The report's `refreshed: false` keeps that fallback honest.
        let cached = self.get(scope, id).await?;

        match self.refresh_inner(scope, id).await {
            Ok(head) => Ok(RefreshReport {
                head,
                refreshed: true,
            }),
            Err(ref e) if Self::is_transient_failure(e) && !cached.is_expired() => {
                tracing::warn!(
                    credential.id = %id,
                    error = %e,
                    "credential refresh coordination failed before provider dispatch; stored material still non-expired"
                );
                Ok(RefreshReport {
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
        let observed_version = stored.version();
        let selector = scope.selector(credential_id);

        // A contender that slept behind another replica may proceed only if
        // the exact row version we loaded is still current. Any version change
        // means another writer established a newer state/reauth decision; the
        // coordinator returns Coalesced and we re-read instead of POSTing the
        // stale grant.
        let store_for_recheck = self.store.clone();
        let selector_for_recheck = selector.clone();
        let needs_refresh_after_backoff = move |_id: &crate::CredentialId| {
            let store = store_for_recheck.clone();
            let selector = selector_for_recheck.clone();
            async move {
                match store.get(&selector).await {
                    Ok(StoredCredential::Live(current)) => {
                        Ok(current.version() == observed_version && !current.reauth_required())
                    },
                    Ok(StoredCredential::Tombstoned(_)) => Ok(false),
                    // A failed recheck cannot establish that the captured
                    // version is still authoritative. Deny provider dispatch;
                    // preserve that distinction instead of pretending another
                    // replica completed the operation.
                    Err(_) => Err(RefreshRecheckError::Unavailable),
                }
            }
        };

        let store = self.store.clone();
        let ops = self.ops.clone();
        let observer = self.observer.clone();
        let id_owned = id.to_owned();
        let selector_for_task = selector.clone();
        let ctx = Self::owner_context(scope);
        let result = self
            .resolver
            .refresh_coordinator()
            .refresh_coalesced(
                &credential_id,
                needs_refresh_after_backoff,
                move || async move {
                    let outcome = ops
                        .refresh(stored.credential_key(), stored.data(), &ctx)
                        .await;

                    let outcome = match outcome {
                        Ok(outcome) => outcome,
                        Err(CredentialServiceError::ReauthRequired { reason, .. }) => {
                            // Make the exact provider/local reauth decision
                            // replica-visible before releasing the claim.
                            let replacement = CredentialReplacement::new(
                                stored.version(),
                                stored.data().clone(),
                                stored.state_kind().to_owned(),
                                stored.state_version(),
                                stored.name().map(str::to_owned),
                                stored.expires_at(),
                                true,
                                stored.metadata().clone(),
                            );
                            return match store.replace(&selector_for_task, replacement).await {
                                Ok(_) => RefreshDisposition::state_advanced(Err(
                                    CredentialServiceError::ReauthRequired {
                                        credential_id: id_owned,
                                        reason,
                                    },
                                )),
                                Err(CredentialPersistenceError::OutcomeUnknown) => {
                                    RefreshDisposition::outcome_unknown(Err(
                                        CredentialServiceError::OutcomeUnknown,
                                    ))
                                },
                                Err(_) => RefreshDisposition::retry_unsafe(Err(
                                    CredentialServiceError::PostProviderPersistence,
                                )),
                            };
                        },
                        Err(CredentialServiceError::OutcomeUnknown) => {
                            return RefreshDisposition::outcome_unknown(Err(
                                CredentialServiceError::OutcomeUnknown,
                            ));
                        },
                        Err(CredentialServiceError::PostProviderPersistence) => {
                            return RefreshDisposition::retry_unsafe(Err(
                                CredentialServiceError::PostProviderPersistence,
                            ));
                        },
                        // Deserialization/type/capability errors occur before
                        // the erased refresh implementation is entered and are
                        // therefore exact and replay-safe.
                        Err(error) => return RefreshDisposition::no_state_change(Err(error)),
                    };

                    let super::ops::RefreshOutcomeKind::Rewrote {
                        data: refreshed,
                        expires_at: refreshed_expires_at,
                    } = outcome
                    else {
                        return RefreshDisposition::state_advanced(Ok(
                            CoordinatedRefreshResult::ReRead,
                        ));
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
                    );
                    let commit = match store.replace(&selector_for_task, replacement).await {
                        Ok(commit) => commit,
                        Err(CredentialPersistenceError::OutcomeUnknown) => {
                            return RefreshDisposition::outcome_unknown(Err(
                                CredentialServiceError::OutcomeUnknown,
                            ));
                        },
                        Err(_) => {
                            return RefreshDisposition::retry_unsafe(Err(
                                CredentialServiceError::PostProviderPersistence,
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
            Ok(Ok(CoordinatedRefreshResult::ReRead))
            | Err(RefreshError::CoalescedByOtherReplica) => {
                let current = self.load_owned(scope, id).await?;
                if current.reauth_required() {
                    return Err(CredentialServiceError::ReauthRequired {
                        credential_id: id.to_owned(),
                        // K2's durable flag does not yet retain the original
                        // reason; use the conservative terminal classification.
                        reason: crate::ReauthReason::ProviderRejected,
                    });
                }
                if current.version() == observed_version {
                    // L1 waiters do not receive the Winner's disposition. If
                    // the row did not advance, the Winner may have returned an
                    // unknown/unsafe post-provider result while retaining L2;
                    // reporting this as a successful coalesced refresh would
                    // invite a blind follow-up.
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
            Err(RefreshError::PriorAttemptNoProgress) => Err(CredentialServiceError::Internal(
                "concurrent credential refresh completed without changing authoritative state"
                    .to_owned(),
            )),
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
            #[expect(
                unreachable_patterns,
                reason = "RefreshError is non-exhaustive; future variants fail closed"
            )]
            Err(_) => Err(CredentialServiceError::Internal(
                "credential refresh coordination failed".to_owned(),
            )),
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
    /// non-replayable [`CredentialServiceError::PostProviderPersistence`].
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
    /// - [`CredentialServiceError::Provider`] — the provider revoke failed.
    /// - [`CredentialServiceError::VersionConflict`] — a concurrent write raced the revoke.
    /// - [`CredentialServiceError::Store`] — persisting the tombstone failed.
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
                        Ok(current.version() == observed_version)
                    },
                    Ok(StoredCredential::Tombstoned(_)) => Ok(false),
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
                    Err(CredentialServiceError::PostProviderPersistence) => {
                        return RefreshDisposition::retry_unsafe(Err(
                            CredentialServiceError::PostProviderPersistence,
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
                        CredentialServiceError::PostProviderPersistence,
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
            #[expect(
                unreachable_patterns,
                reason = "RefreshError is non-exhaustive; future variants fail closed"
            )]
            Err(_) => Err(CredentialServiceError::Internal(
                "credential revoke coordination failed".to_owned(),
            )),
        }
    }
}
