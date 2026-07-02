//! Capability-operation surface of [`CredentialService`] — the
//! `test` / `refresh` / `revoke` lifecycle operations.
//!
//! Split out of `facade.rs` (behaviour-preserving code motion — no logic
//! change). Reads the same `pub(crate)` [`CredentialService`] internals
//! (`load_owned`, `owner_context`, `map_store_err`, `get`) as the rest of
//! the service; `refresh_inner` (the CAS-persist "inner refresh") and
//! `is_transient_failure` stay private to this module.

use std::time::Duration;

use nebula_resilience::CallError;
use nebula_resilience::retry::{BackoffConfig, RetryConfig, retry_with};
use serde_json::Value;

use crate::CredentialId;
use crate::resolve::TestResult;
use crate::store::{
    LAST_VALIDATED_AT_METADATA_KEY, PutMode, REVOKED_AT_METADATA_KEY, StoredCredential,
};

use super::error::CredentialServiceError;
use super::facade::{CredentialService, RefreshReport, TestReport};
use super::head::CredentialHead;
use super::scope::TenantScope;

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
    ) -> Result<TestReport, CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        if !self.registry.is_testable(&stored.credential_key) {
            return Err(CredentialServiceError::CapabilityUnsupported {
                capability: "test".to_owned(),
                key: stored.credential_key.clone(),
            });
        }
        let ctx = Self::owner_context(scope);
        let result = self
            .ops
            .test(&stored.credential_key, &stored.data, &ctx)
            .await?;
        let report = match result {
            TestResult::Success => TestReport {
                ok: true,
                message: None,
            },
            TestResult::Failed { reason } => TestReport {
                ok: false,
                message: Some(reason),
            },
            // `TestResult` is exhaustively matched here (this crate defines it).
            // Adding a variant is a compile error at this arm, forcing a
            // deliberate decision rather than silently presenting as a pass.
        };
        tracing::info!(credential.id = %id, ok = report.ok, "credential tested");
        Ok(report)
    }

    /// Force-refresh the credential's stored state and re-persist it.
    ///
    /// Owner-checked first. The refresh runs through
    /// [`nebula_resilience::retry_with`] (3 attempts, exponential
    /// backoff). If this caller performed the refresh the resulting state
    /// is written back under compare-and-swap on the version observed at
    /// load; a concurrent refresh/update wins and this attempt fails
    /// explicitly with [`CredentialServiceError::VersionConflict`] —
    /// concurrent-refresh contract: refresh must never silently strand a concurrent
    /// write. If another replica coalesced the refresh
    /// (`RefreshOutcome::CoalescedByOtherReplica`) the write is **skipped
    /// entirely** and the now-fresher state is re-read from the store
    /// instead of clobbering it with the un-mutated local copy. On
    /// success (either path) [`CredentialObserver::on_refresh`](super::observer::CredentialObserver::on_refresh) fires and
    /// the fresh secret-free [`CredentialHead`] is returned.
    ///
    /// ## Fallback-on-interrupt
    ///
    /// If the provider call fails with a **transient** error
    /// ([`CredentialServiceError::TransientProvider`]) AND the currently
    /// stored material is still non-expired, the cached head is
    /// returned instead of propagating the error. This protects in-flight
    /// executions from transient provider 5xx / network blips without
    /// papering over real expiry. Terminal failures (token expired / revoked /
    /// authentication) always propagate regardless of cached state.
    ///
    /// This matches the `aws-credential-types` `fallback_on_interrupt` pattern.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::NotFound`] — absent or cross-tenant id.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Refreshable`.
    /// - [`CredentialServiceError::Provider`] — refresh failed after retries (terminal).
    /// - [`CredentialServiceError::TransientProvider`] — transient failure AND stored material
    ///   is expired (no valid fallback available).
    /// - [`CredentialServiceError::VersionConflict`] — a concurrent write landed first.
    /// - [`CredentialServiceError::Store`] — re-persist failed.
    pub async fn refresh(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<RefreshReport, CredentialServiceError> {
        // Read the current head before attempting refresh. On a transient
        // provider failure we fall back to this if the material is still
        // non-expired — avoids propagating blips to the caller. The
        // report's `refreshed: false` keeps the fallback honest for
        // management callers.
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
                    "credential refresh failed transiently; stored material still non-expired"
                );
                Ok(RefreshReport {
                    head: cached,
                    refreshed: false,
                })
            },
            Err(e) => Err(e),
        }
    }

    /// Inner refresh: actual provider call + CAS-persist. The public
    /// [`refresh`](Self::refresh) wrapper applies the fallback-on-interrupt
    /// logic around this method.
    async fn refresh_inner(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<CredentialHead, CredentialServiceError> {
        let stored = self.load_owned(scope, id).await?;
        if !self.registry.is_refreshable(&stored.credential_key) {
            return Err(CredentialServiceError::CapabilityUnsupported {
                capability: "refresh".to_owned(),
                key: stored.credential_key.clone(),
            });
        }
        let ctx = Self::owner_context(scope);

        let config = RetryConfig::<CredentialServiceError>::new(3)
            .map_err(|e| CredentialServiceError::Internal(format!("retry config invalid: {e}")))?
            .backoff(BackoffConfig::Exponential {
                base: Duration::from_millis(200),
                multiplier: 2.0,
                max: Duration::from_secs(5),
            });

        let outcome = retry_with(config, || async {
            self.ops
                .refresh(&stored.credential_key, &stored.data, &ctx)
                .await
        })
        .await
        .map_err(|call_err| match call_err {
            CallError::Operation(e) | CallError::RetriesExhausted { last: e, .. } => e,
            other => {
                CredentialServiceError::Provider(format!("credential refresh failed: {other}"))
            },
        })
        .map_err(|e| match e {
            // The erased ops closure cannot see the credential id; fill it in
            // here so the typed re-auth signal reaches the API with its id.
            CredentialServiceError::ReauthRequired { reason, .. } => {
                CredentialServiceError::ReauthRequired {
                    credential_id: id.to_owned(),
                    reason,
                }
            },
            other => other,
        })?;

        let (refreshed, refreshed_expires_at) = match outcome {
            super::ops::RefreshOutcomeKind::Rewrote { data, expires_at } => (data, expires_at),
            // Another replica already refreshed and persisted fresher
            // state. Re-writing the un-mutated local copy here would
            // either spuriously `VersionConflict` or clobber that fresher
            // state (concurrent-refresh contract). Skip the write entirely and return the
            // store's current (post-coalesce) head.
            super::ops::RefreshOutcomeKind::CoalescedReRead => {
                let credential_id = CredentialId::parse(&stored.id).map_err(|e| {
                    CredentialServiceError::Internal(format!(
                        "stored credential id unparsable: {e}"
                    ))
                })?;
                self.observer.on_refresh(&credential_id);
                tracing::info!(
                    credential.id = %id,
                    "credential refresh coalesced by another replica; re-reading without re-writing"
                );
                return self.get(scope, id).await;
            },
        };

        let now = chrono::Utc::now();
        let state_kind = stored.state_kind.clone();
        let state_version = stored.state_version;
        // Refresh contacted the provider successfully → advance the
        // re-validation anchor so the mandatory floor measures from this real
        // validation, mirroring the resolver refresh path. `refresh_inner` is a
        // provider-contacting write and was the lone such writer that omitted
        // the stamp; a display-only edit goes through `update` without a
        // re-resolve and must NOT bump it.
        let mut metadata = stored.metadata.clone();
        metadata.insert(
            LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
            Value::String(now.to_rfc3339()),
        );
        let stored_next = StoredCredential {
            id: stored.id.clone(),
            name: stored.name.clone(),
            credential_key: stored.credential_key.clone(),
            data: refreshed.to_vec(),
            state_kind,
            state_version,
            version: stored.version,
            created_at: stored.created_at,
            updated_at: now,
            // The refresh closure read this off the *refreshed* state
            // (`CredentialState::expires_at()`), not the pre-refresh row:
            // a token rotation typically produces a new expiry, so reusing
            // `stored.expires_at` would persist a stale (possibly
            // already-elapsed) expiry against fresh credential bytes.
            expires_at: refreshed_expires_at,
            reauth_required: false,
            metadata,
        };
        // Re-persist under compare-and-swap on the version observed at
        // load. A concurrent refresh/update that landed in between wins
        // and this attempt fails *explicitly* with `VersionConflict`
        // (concurrent-refresh contract: refresh must never silently strand a concurrent
        // write; failure is explicit). Blind `Overwrite` here would
        // last-writer-wins and clobber the racing write.
        self.store
            .put(
                stored_next,
                PutMode::CompareAndSwap {
                    expected_version: stored.version,
                },
            )
            .await
            .map_err(Self::map_store_err)?;

        let credential_id = CredentialId::parse(&stored.id).map_err(|e| {
            CredentialServiceError::Internal(format!("stored credential id unparsable: {e}"))
        })?;
        self.observer.on_refresh(&credential_id);
        tracing::info!(credential.id = %id, "credential refreshed");
        self.get(scope, id).await
    }

    /// True iff this error is a transient refresh/provider failure that the
    /// fallback-on-interrupt path can swallow when cached material is still
    /// non-expired.
    ///
    /// Only [`CredentialServiceError::TransientProvider`] qualifies — this
    /// variant is emitted exclusively by the refresh ops closure for the
    /// transient `CredentialError` kinds (`RefreshFailed(TransientNetwork |
    /// ProviderUnavailable)` / `Provider(Network | RateLimit | ServerError)`).
    /// Terminal failures use [`CredentialServiceError::Provider`] and are
    /// excluded here so the fallback never swallows real expiry or auth errors.
    #[inline]
    fn is_transient_failure(e: &CredentialServiceError) -> bool {
        matches!(e, CredentialServiceError::TransientProvider(_))
    }

    /// Revoke the credential at the provider, release any leases, and write a
    /// revoke **tombstone** over the stored row (it is not deleted).
    ///
    /// Owner-checked first. The provider-side revoke runs the type's
    /// `Revocable::revoke`; lease release is best-effort (a failure is
    /// logged, not propagated — the credential is still revoked); the stored
    /// row is then CAS-overwritten with a tombstone epoch and empty secret
    /// bytes. On success [`CredentialObserver::on_revoke`](super::observer::CredentialObserver::on_revoke) fires.
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
        if !self.registry.is_revocable(&stored.credential_key) {
            return Err(CredentialServiceError::CapabilityUnsupported {
                capability: "revoke".to_owned(),
                key: stored.credential_key.clone(),
            });
        }
        let ctx = Self::owner_context(scope);
        self.ops
            .revoke(&stored.credential_key, &stored.data, &ctx)
            .await?;

        let credential_id = CredentialId::parse(&stored.id).map_err(|e| {
            CredentialServiceError::Internal(format!("stored credential id unparsable: {e}"))
        })?;

        // Best-effort lease release: a credential whose provider-side
        // secret is revoked must not keep dynamic leases alive, but a
        // lease-subsystem hiccup must not block the revoke itself (the
        // secret is already dead at the provider).
        let released = self.lease.revoke_for_credential(credential_id).await;
        if released > 0 {
            tracing::info!(
                credential.id = %id,
                released,
                "released dynamic leases for revoked credential"
            );
        }

        // Write a tombstone instead of deleting the row. A revoked credential
        // must not be resurrectable under the same id, and a workflow slot
        // binding that still points at it must surface a typed
        // `CredentialTombstoned` (via `validate_credential_binding`) rather than
        // a bare `NotFound`. The secret bytes are dropped — a revoked secret has
        // no reason to persist at rest. CAS on the version loaded above so a
        // rotation/update racing this revoke conflicts instead of silently
        // clobbering (or resurrecting) the row.
        let now = chrono::Utc::now();
        let expected_version = stored.version;
        let mut metadata = stored.metadata;
        metadata.insert(
            REVOKED_AT_METADATA_KEY.to_owned(),
            Value::String(now.to_rfc3339()),
        );
        let tombstoned = StoredCredential {
            data: Vec::new(),
            updated_at: now,
            metadata,
            ..stored
        };
        self.store
            .put(tombstoned, PutMode::CompareAndSwap { expected_version })
            .await
            .map_err(Self::map_store_err)?;

        self.observer.on_revoke(&credential_id);
        tracing::info!(credential.id = %id, "credential revoked");
        Ok(())
    }
}
