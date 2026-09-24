//! Acquisition surface of [`CredentialService`] — the `resolve` /
//! `continue_resolve` interactive-acquisition flow.
//!
//! Split out of `facade.rs` (behaviour-preserving code motion — no logic
//! change) so the CRUD facade stays focused. Kept in the `service` module
//! so it reads the same `pub(crate)` [`CredentialService`] internals
//! (`ensure_local_source`, `owner_context`, `persist_resolved`).

use serde_json::Value;

use super::acquisition_intent::AcquisitionIntent;
use super::head::CredentialHead;
use super::ops::ResolvedState;
use crate::resolve::UserInput;
use crate::{
    CredentialDisplay, CredentialId, CredentialMaterialTransition, CredentialReplacement,
    LAST_VALIDATED_AT_METADATA_KEY, PendingToken, StoredCredentialHead,
};
use nebula_storage_port::{CredentialPersistenceError, CredentialReplacementFence};

use super::error::CredentialServiceError;
use super::facade::{Acquisition, CredentialService};
use crate::TenantScope;

impl CredentialService {
    /// Acquire a credential of `credential_key` from `props`, persisting
    /// it on synchronous completion or surfacing an interaction token for
    /// interactive flows.
    ///
    /// Validation is the canonical credential pipeline (the `$expr`
    /// refusal point, credential secrecy). A `Complete` resolution is persisted
    /// through the same path as [`create`](Self::create) and returned as
    /// [`Acquisition::Complete`]; a `Pending` kickoff returns
    /// [`Acquisition::Pending`] with the opaque token + UI instruction.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::TypeUnknown`] — key not registered.
    /// - [`CredentialServiceError::ValidationFailed`] — schema / typed-deserialize / resolve.
    /// - [`CredentialServiceError::SessionRequired`] — the resolution
    ///   went `Pending` (interactive kickoff) but `scope` carries no
    ///   session, so the issued token could never be redeemed.
    /// - [`CredentialServiceError::Store`] — persistence failure on the `Complete` path.
    pub(crate) async fn resolve(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        props: Value,
    ) -> Result<Acquisition, CredentialServiceError> {
        self.ensure_local_source()?;
        if !self.registry.contains(credential_key) {
            return Err(CredentialServiceError::TypeUnknown {
                key: credential_key.to_owned(),
            });
        }
        let ctx = self.owner_context(scope);
        let outcome = self
            .ops
            .acquire(credential_key, props, &ctx, &self.pending)
            .await?;
        self.finish_acquire(
            scope,
            credential_key,
            outcome,
            &AcquisitionIntent::create_for_key(credential_key),
        )
        .await
    }

    /// Begin authorization again for one existing owner-bound aggregate.
    #[tracing::instrument(name = "credential.reauthorize", skip_all, fields(credential.id = %credential_id))]
    pub(crate) async fn reauthorize(
        &self,
        scope: &TenantScope,
        credential_id: CredentialId,
        props: Value,
    ) -> Result<Acquisition, CredentialServiceError> {
        self.ensure_local_source()?;
        let existing = self.load_owned_head(scope, credential_id).await?;
        let key = existing.credential_key();
        let intent = AcquisitionIntent::ReauthorizeExisting {
            credential_id: credential_id.to_string(),
            observed_version: existing.version().get() as u64,
            observed_material_epoch: existing.material_epoch().get() as u64,
            credential_key: key.to_owned(),
        };
        let ctx = self.owner_context(scope);
        let outcome = self
            .ops
            .acquire_with_intent(key, props, &ctx, &self.pending, intent.clone())
            .await?;
        self.finish_acquire(scope, key, outcome, &intent).await
    }

    /// Continue an acquisition according to its authenticated durable intent.
    /// Reauthorization validates the original aggregate fence before provider work.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::TypeUnknown`] — key not registered.
    /// - [`CredentialServiceError::SessionRequired`] — `scope` carries no
    ///   session; the pending-store binding makes a continuation
    ///   structurally impossible without one.
    /// - [`CredentialServiceError::CapabilityUnsupported`] — type is not `Interactive`.
    /// - [`CredentialServiceError::ValidationFailed`] — continuation failed.
    /// - [`CredentialServiceError::Store`] — persistence failure on the `Complete` path.
    pub(crate) async fn continue_resolve(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        pending_token: &str,
        user_input: UserInput,
    ) -> Result<Acquisition, CredentialServiceError> {
        self.ensure_local_source()?;
        if !self.registry.contains(credential_key) {
            return Err(CredentialServiceError::TypeUnknown {
                key: credential_key.to_owned(),
            });
        }
        // A continuation is structurally dead without a Plane-A
        // authentication binding: the
        // engine's `execute_continue` requires `ctx.session_id()` and the
        // `PendingStateStore` binds the pending on
        // `(kind, owner, session, token)`. Surface that explicitly here
        // rather than letting it collapse into a misleading
        // `ValidationFailed` deep inside the executor.
        if scope.authentication_binding().is_none() {
            return Err(CredentialServiceError::SessionRequired {
                capability: "continue",
            });
        }
        let token = PendingToken::parse(pending_token).ok_or_else(|| {
            CredentialServiceError::validation("/pending_token", "credential.pending_token_invalid")
        })?;
        let ctx = self.owner_context(scope);
        let intent = self
            .ops
            .pending_intent(credential_key, &token, &ctx, &self.pending)
            .await?;
        self.validate_acquisition_intent(scope, credential_key, &intent)
            .await?;
        let outcome = self
            .ops
            .continue_with_intent(
                &token,
                &user_input,
                &ctx,
                &self.pending,
                intent.expectation(),
            )
            .await?;
        self.finish_acquire(scope, credential_key, outcome, &intent)
            .await
    }

    /// Map an [`AcquireOutcome`](super::ops::AcquireOutcome) into the public [`Acquisition`]:
    /// `Complete` is persisted (shared create path); `Pending`/`Retry`
    /// surface the token + interaction without persisting.
    async fn finish_acquire(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        outcome: super::ops::AcquireOutcome,
        intent: &AcquisitionIntent,
    ) -> Result<Acquisition, CredentialServiceError> {
        match outcome {
            super::ops::AcquireOutcome::Complete(resolved) => {
                if matches!(intent, AcquisitionIntent::ReauthorizeExisting { .. }) {
                    let head = self
                        .replace_reauthorized(scope, credential_key, intent, resolved)
                        .await?;
                    return Ok(Acquisition::Complete { head });
                }
                let id = CredentialId::new();
                // Acquisition carries no caller-supplied display metadata
                // (the interactive/resolve flow names nothing); a later
                // `update` can attach it.
                let head = self
                    .persist_resolved(
                        scope,
                        credential_key,
                        id,
                        resolved,
                        CredentialDisplay::default(),
                    )
                    .await?;
                self.observer.on_resolve(&id);
                tracing::info!(
                    credential.key = credential_key,
                    credential.id = %id,
                    "credential acquired"
                );
                Ok(Acquisition::Complete { head })
            },
            super::ops::AcquireOutcome::Pending { token, interaction } => {
                // The interaction can only be completed through
                // `continue_resolve`, which the engine binds on
                // `(kind, owner, session, token)`. Without a session on
                // the scope the issued token is unusable, so refuse the
                // kickoff explicitly instead of handing back a token that
                // can never be redeemed.
                if scope.authentication_binding().is_none() {
                    return Err(CredentialServiceError::SessionRequired {
                        capability: "resolve",
                    });
                }
                Ok(Acquisition::Pending {
                    token: token.as_str().to_owned(),
                    interaction,
                })
            },
            super::ops::AcquireOutcome::Retry { after } => Ok(Acquisition::Retry { after }),
        }
    }
    async fn validate_acquisition_intent(
        &self,
        scope: &TenantScope,
        key: &str,
        intent: &AcquisitionIntent,
    ) -> Result<Option<StoredCredentialHead>, CredentialServiceError> {
        match intent {
            AcquisitionIntent::Create { credential_key } if credential_key == key => Ok(None),
            AcquisitionIntent::ReauthorizeExisting { credential_id, .. } => {
                let credential_id = CredentialId::parse(credential_id).map_err(|_| {
                    CredentialServiceError::NotFound {
                        id: credential_id.clone(),
                    }
                })?;
                let existing = self.load_owned_head(scope, credential_id).await?;
                validate_reauthorization_fence(key, intent, &existing)?;
                Ok(Some(existing))
            },
            _ => Err(CredentialServiceError::validation(
                "/pending_token",
                "credential.acquisition_intent_mismatch",
            )),
        }
    }

    async fn replace_reauthorized(
        &self,
        scope: &TenantScope,
        key: &str,
        intent: &AcquisitionIntent,
        resolved: ResolvedState,
    ) -> Result<CredentialHead, CredentialServiceError> {
        let existing = self
            .validate_acquisition_intent(scope, key, intent)
            .await?
            .ok_or_else(|| {
                CredentialServiceError::validation(
                    "/pending_token",
                    "credential.acquisition_intent_mismatch",
                )
            })?;
        let display = Self::display_from_metadata(existing.metadata());
        let mut metadata = existing.metadata().clone();
        let now = chrono::Utc::now();
        metadata.insert(
            LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
            Value::String(now.to_rfc3339()),
        );
        let replacement = CredentialReplacement::new(
            existing.version(),
            resolved.data.to_vec().into(),
            resolved.state_kind,
            resolved.state_version,
            display.display_name.clone(),
            resolved.expires_at,
            false,
            metadata,
            CredentialMaterialTransition::advance(),
        )
        .with_fence(CredentialReplacementFence::new(
            existing.material_epoch(),
            key.to_owned(),
        ));
        let id = existing.credential_id();
        let commit = self
            .store
            .replace(&scope.selector(id), replacement)
            .await
            .map_err(|error| Self::map_store_err_for(&id.to_string(), error))?;
        self.observer.on_refresh(&id);
        tracing::info!(credential.id = %id, "credential reauthorized");
        Ok(CredentialHead {
            id: id.to_string(),
            credential_key: key.to_owned(),
            version: commit.version().get() as u64,
            created_at: commit.created_at(),
            updated_at: commit.updated_at(),
            expires_at: resolved.expires_at,
            last_validated_at: Some(now),
            lifecycle: crate::CredentialLifecycleState::Ready,
            reauth_required: false,
            display,
        })
    }

    async fn load_owned_head(
        &self,
        scope: &TenantScope,
        credential_id: CredentialId,
    ) -> Result<StoredCredentialHead, CredentialServiceError> {
        self.store
            .get_head(&scope.selector(credential_id))
            .await
            .map_err(|error| match error {
                CredentialPersistenceError::NotFound => CredentialServiceError::NotFound {
                    id: credential_id.to_string(),
                },
                error => Self::map_store_err_for(&credential_id.to_string(), error),
            })
    }
}

fn validate_reauthorization_fence(
    key: &str,
    intent: &AcquisitionIntent,
    existing: &StoredCredentialHead,
) -> Result<(), CredentialServiceError> {
    let AcquisitionIntent::ReauthorizeExisting {
        credential_id,
        observed_version,
        observed_material_epoch,
        credential_key,
    } = intent
    else {
        return Err(CredentialServiceError::validation(
            "/pending_token",
            "credential.acquisition_intent_mismatch",
        ));
    };
    if credential_key != key
        || credential_id != &existing.credential_id().to_string()
        || existing.credential_key() != key
        || *observed_version != existing.version().get() as u64
        || *observed_material_epoch != existing.material_epoch().get() as u64
    {
        tracing::warn!("credential reauthorization rejected stale aggregate fence");
        return Err(CredentialServiceError::VersionConflict {
            id: existing.credential_id().to_string(),
            expected: *observed_version,
            actual: existing.version().get() as u64,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_storage_port::{CredentialMaterialEpoch, CredentialVersion};

    fn row(id: CredentialId, key: &str, version: u64, epoch: u64) -> StoredCredentialHead {
        let now = chrono::Utc::now();
        StoredCredentialHead::new(
            id,
            Some("Original display".to_owned()),
            key.to_owned(),
            "token".to_owned(),
            1,
            CredentialVersion::try_from(version).expect("version"),
            CredentialMaterialEpoch::try_from(epoch).expect("epoch"),
            now,
            now,
            None,
            true,
            serde_json::Map::new(),
        )
        .expect("head fixture")
    }

    #[test]
    fn reauthorization_fence_rejects_each_stale_or_substituted_axis() {
        let id = CredentialId::new();
        let intent = AcquisitionIntent::ReauthorizeExisting {
            credential_id: id.to_string(),
            observed_version: 4,
            observed_material_epoch: 2,
            credential_key: "oauth2".to_owned(),
        };
        assert!(
            validate_reauthorization_fence("oauth2", &intent, &row(id, "oauth2", 4, 2)).is_ok()
        );
        for stored in [
            row(id, "oauth2", 5, 2),
            row(id, "oauth2", 4, 3),
            row(id, "different", 4, 2),
            row(CredentialId::new(), "oauth2", 4, 2),
        ] {
            assert!(matches!(
                validate_reauthorization_fence("oauth2", &intent, &stored),
                Err(CredentialServiceError::VersionConflict { .. })
            ));
        }
        assert!(matches!(
            validate_reauthorization_fence("different", &intent, &row(id, "oauth2", 4, 2)),
            Err(CredentialServiceError::VersionConflict { .. })
        ));
        assert!(matches!(
            validate_reauthorization_fence(
                "oauth2",
                &AcquisitionIntent::create_for_key("oauth2"),
                &row(id, "oauth2", 4, 2)
            ),
            Err(CredentialServiceError::ValidationFailed { .. })
        ));
    }
}
