//! CRUD surface of [`CredentialService`] — create / read / list / update /
//! delete of stored credential rows.
//!
//! Split out of `facade.rs` (behaviour-preserving code motion — no logic
//! change). Reads the same `pub(crate)` [`CredentialService`] internals
//! (`ensure_local_source`, `owner_context`, `load_owned`, `head_from`,
//! `owner_matches`, `set_display`, `map_store_err`) as the rest of the
//! service.

use nebula_storage_port::RefreshRetryGate;
use serde_json::Value;

use crate::{
    CredentialCreate, CredentialDisplay, CredentialId, CredentialPersistenceError,
    CredentialReplacement, CredentialTombstone, CredentialVersion, LAST_VALIDATED_AT_METADATA_KEY,
    OWNER_ID_METADATA_KEY as OWNER_ID_KEY,
};

use super::acquire::map_acquisition_finalization_error;
use super::error::CredentialServiceError;
use super::facade::CredentialService;
use super::head::CredentialHead;
use super::ops::AcquisitionCompletionEvidence;
use crate::TenantScope;

impl CredentialService {
    /// Create a credential: validate `props` against the type's schema,
    /// resolve it to encrypted state, and persist it scoped to `scope`.
    ///
    /// The validation pipeline is the canonical credential pipeline
    /// consumes literal JSON, prepares it against `Properties`' schema,
    /// completes validation without an expression engine, and checks typed
    /// decoding before handing the same prepared values to the credential.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::TypeUnknown`] — no type registered under `credential_key`.
    /// - [`CredentialServiceError::ValidationFailed`] — schema or typed-deserialize rejection
    ///   (including `$expr` injection), or a resolve failure.
    /// - [`CredentialServiceError::AcquisitionFinalizationRequired`] — resolution completed but
    ///   the following durable create definitely failed. The erased type does
    ///   not prove that completion was local, so replay is conservatively refused.
    /// - [`CredentialServiceError::OutcomeUnknown`] — commit acknowledgement
    ///   was lost; reconcile before replaying the command.
    pub(crate) async fn create(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        props: Value,
        display: CredentialDisplay,
    ) -> Result<CredentialHead, CredentialServiceError> {
        // Fail loud if an external source was configured but its
        // resolution wiring is not implemented yet — never silently
        // resolve from the local store under a Vault-configured service.
        self.ensure_local_source()?;
        // The type must be registered (TypeUnknown closes the abuse where
        // an unregistered key reaches resolution).
        if !self.registry.contains(credential_key) {
            return Err(CredentialServiceError::TypeUnknown {
                key: credential_key.to_owned(),
            });
        }

        let id = CredentialId::new();
        let ctx = self.owner_context(scope);

        let resolved = self.ops.resolve(credential_key, props, &ctx).await?;
        let completion_evidence = resolved.completion_evidence;

        let head = self
            .persist_resolved(scope, credential_key, id, resolved, display)
            .await
            .map_err(|error| {
                map_acquisition_finalization_error(
                    completion_evidence,
                    Self::map_store_err_for(&id.to_string(), error),
                )
            })?;

        self.observer.on_resolve(&id);
        tracing::info!(
            credential.key = credential_key,
            credential.id = %id,
            "credential created"
        );

        Ok(head)
    }

    /// Persist a freshly-resolved credential under `id` scoped to
    /// `scope`, returning the secret-free [`CredentialHead`] of the
    /// just-persisted row (never the state bytes). Shared by [`create`]
    /// and the synchronous-`Complete` arm of [`resolve`](Self::resolve).
    ///
    /// [`create`]: Self::create
    pub(crate) async fn persist_resolved(
        &self,
        scope: &TenantScope,
        credential_key: &str,
        id: CredentialId,
        resolved: super::ops::ResolvedState,
        display: CredentialDisplay,
    ) -> Result<CredentialHead, CredentialPersistenceError> {
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            OWNER_ID_KEY.to_owned(),
            Value::String(scope.owner_id().to_owned()),
        );
        Self::set_display(&mut metadata, &display);

        let now = chrono::Utc::now();
        // Creation resolved the credential against its provider → stamp the
        // validation time so the mandatory re-validation floor measures from a
        // real validation, not from a later display edit.
        metadata.insert(
            LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
            Value::String(now.to_rfc3339()),
        );
        let create = CredentialCreate::new(
            credential_key.to_owned(),
            resolved.data.clone().into(),
            resolved.state_kind,
            resolved.state_version,
            display.display_name.clone(),
            resolved.expires_at,
            false,
            metadata,
        );

        let commit = self.store.create(&scope.selector(id), create).await?;

        Ok(CredentialHead {
            id: commit.credential_id().to_string(),
            credential_key: credential_key.to_owned(),
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

    /// Fetch a credential's secret-free [`CredentialHead`], scoped to
    /// `scope`. Never deserializes the state bytes, so a row that is not
    /// yet resolvable (e.g. an interactive flow awaiting authorization,
    /// `reauth_required = true`) still reads back as a valid head.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::NotFound`] if the id is absent **or**
    /// belongs to another tenant (no cross-tenant existence leak).
    pub async fn get(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<CredentialHead, CredentialServiceError> {
        let credential_id = CredentialId::parse(id)
            .map_err(|_| CredentialServiceError::NotFound { id: id.to_owned() })?;
        let stored = match self
            .store
            .get_operational_head(&scope.selector(credential_id))
            .await
        {
            Ok(stored) => stored,
            Err(CredentialPersistenceError::NotFound) => {
                return Err(CredentialServiceError::NotFound { id: id.to_owned() });
            },
            Err(error) => return Err(Self::map_store_err_for(id, error)),
        };
        Ok(Self::head_from_projection(stored.head()).with_operation_status(stored.status()))
    }

    /// List the secret-free heads of every credential visible to `scope`
    /// (rows whose stored `owner_id` matches).
    ///
    /// Listing is owner-bound in the persistence port and therefore scales
    /// with the caller's partition rather than the global credential count.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::Store`] on a backend failure.
    pub async fn list(
        &self,
        scope: &TenantScope,
    ) -> Result<Vec<CredentialHead>, CredentialServiceError> {
        let rows = self
            .store
            .list_operational_heads(scope.owner(), None)
            .await
            .map_err(|error| Self::map_store_err_for("credential", error))?;
        Ok(rows
            .into_iter()
            .map(|stored| {
                Self::head_from_projection(stored.head()).with_operation_status(stored.status())
            })
            .collect())
    }

    /// Update a credential's stored state and/or display metadata.
    ///
    /// `props = Some(..)` re-runs the canonical validate→resolve pipeline
    /// for the row's (unchanged) credential type and replaces the stored
    /// state; `props = None` preserves the existing semantic state and rewrites
    /// only display metadata at the service boundary — a rename/re-tag never
    /// re-resolves provider material. The storage encryption decorator may
    /// re-encrypt the same plaintext into a fresh envelope/current key during
    /// that write.
    ///
    /// `display` is the **full replacement** value; callers that want
    /// field-wise merge semantics read the current head first and merge
    /// before calling.
    ///
    /// `expected_version = Some(v)` engages compare-and-swap on the
    /// caller's version (a mismatch surfaces as
    /// [`CredentialServiceError::VersionConflict`]); `None` CASes on the
    /// version this call just loaded, so a concurrent write landing
    /// between the load and the put surfaces as `VersionConflict` instead
    /// of silently rolling the row — including its secret state and any
    /// concurrently-rotated tokens — back to the loaded copy. There is no
    /// blind-overwrite path.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::NotFound`] — absent or cross-tenant id.
    /// - [`CredentialServiceError::ValidationFailed`] — schema / typed-deserialize / resolve.
    /// - [`CredentialServiceError::VersionConflict`] — stale `expected_version`, detected before
    ///   replacement resolution, or a display-only update lost its CAS race.
    /// - [`CredentialServiceError::AcquisitionFinalizationRequired`] — replacement material was
    ///   resolved but the following durable replace definitely failed.
    /// - [`CredentialServiceError::Store`] — display-only persistence failure.
    pub(crate) async fn update(
        &self,
        scope: &TenantScope,
        id: &str,
        props: Option<Value>,
        expected_version: Option<u64>,
        display: CredentialDisplay,
    ) -> Result<CredentialHead, CredentialServiceError> {
        // Owner check first: a cross-tenant id is reported as missing,
        // never as a version conflict (no existence leak).
        let existing = self.load_owned(scope, id).await?;
        let operation_status = self
            .store
            .operation_status(&scope.selector(existing.credential_id()))
            .await
            .map_err(|error| Self::map_store_err_for(id, error))?;
        if props.is_some()
            && let Some(operation) = operation_status.blocking_operation()
        {
            return Err(CredentialServiceError::OperationBlocked { operation });
        }
        let actual = existing.version();
        let requested = expected_version.unwrap_or_else(|| actual.get() as u64);
        let expected = CredentialVersion::try_from(requested).map_err(|_| {
            CredentialServiceError::VersionConflict {
                id: id.to_owned(),
                expected: requested,
                actual: actual.get() as u64,
            }
        })?;
        if expected != actual {
            return Err(CredentialServiceError::VersionConflict {
                id: id.to_owned(),
                expected: requested,
                actual: actual.get() as u64,
            });
        }

        // Re-resolve only when new properties were supplied; a
        // display-only update carries the existing state through.
        let resolved = match props {
            Some(props) => {
                // Match create/acquisition: an external source must never
                // fall back to resolving replacement material locally.
                self.ensure_local_source()?;
                let ctx = self.owner_context(scope);
                Some(
                    self.ops
                        .resolve(existing.credential_key(), props, &ctx)
                        .await?,
                )
            },
            None => None,
        };
        let completion_evidence = resolved
            .as_ref()
            .map(|resolved| resolved.completion_evidence);

        let material_replaced = resolved.is_some();
        let mut metadata = existing.metadata().clone();
        metadata.insert(
            OWNER_ID_KEY.to_owned(),
            Value::String(scope.owner_id().to_owned()),
        );
        Self::set_display(&mut metadata, &display);

        let now = chrono::Utc::now();
        let (data, state_kind, state_version, expires_at, reauth_required, last_validated_at) =
            match resolved {
                // Props supplied ⇒ re-resolved against the provider ⇒ stamp the
                // validation time. A display-only edit (the `None` arm) preserves the
                // existing stamp and bumps only `updated_at`, so it cannot postpone
                // the re-validation floor.
                Some(resolved) => {
                    metadata.insert(
                        LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
                        Value::String(now.to_rfc3339()),
                    );
                    (
                        resolved.data.clone().into(),
                        resolved.state_kind,
                        resolved.state_version,
                        resolved.expires_at,
                        false,
                        Some(now),
                    )
                },
                None => (
                    existing.data().clone(),
                    existing.state_kind().to_owned(),
                    existing.state_version(),
                    existing.expires_at(),
                    existing.reauth_required(),
                    metadata
                        .get(LAST_VALIDATED_AT_METADATA_KEY)
                        .and_then(Value::as_str)
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                        .map(|instant| instant.with_timezone(&chrono::Utc)),
                ),
            };

        // No blind-overwrite path: when the caller supplied no version,
        // CAS on the version loaded above. A display-only rename racing a
        // token refresh must conflict, never silently restore the stale
        // secret bytes captured at load time.
        let replacement = CredentialReplacement::new(
            expected,
            data,
            state_kind,
            state_version,
            display.display_name.clone(),
            expires_at,
            reauth_required,
            metadata,
            if material_replaced {
                crate::CredentialMaterialTransition::advance()
            } else {
                crate::CredentialMaterialTransition::preserve(
                    crate::RefreshRetryTransition::Preserve,
                )
            },
        );

        let commit = self
            .store
            .replace(&scope.selector(existing.credential_id()), replacement)
            .await
            .map_err(|error| {
                map_update_finalization_error(
                    completion_evidence,
                    Self::map_store_err_for(id, error),
                )
            })?;

        tracing::info!(credential.id = %id, "credential updated");
        let lifecycle = if reauth_required {
            crate::CredentialLifecycleState::ReauthRequired
        } else if material_replaced {
            crate::CredentialLifecycleState::Ready
        } else {
            match existing.refresh_retry_gate() {
                None => crate::CredentialLifecycleState::Ready,
                Some(RefreshRetryGate::Never { .. }) => {
                    crate::CredentialLifecycleState::RefreshBlocked
                },
                Some(RefreshRetryGate::NotBefore { not_before, .. })
                    if *not_before > commit.updated_at() =>
                {
                    crate::CredentialLifecycleState::RefreshDeferred {
                        retry_at: *not_before,
                    }
                },
                Some(RefreshRetryGate::NotBefore { .. }) => crate::CredentialLifecycleState::Ready,
            }
        };
        Ok(CredentialHead {
            id: commit.credential_id().to_string(),
            credential_key: existing.credential_key().to_owned(),
            version: commit.version().get() as u64,
            created_at: commit.created_at(),
            updated_at: commit.updated_at(),
            expires_at,
            last_validated_at,
            lifecycle,
            reauth_required,
            display,
        }
        .with_operation_status(operation_status))
    }

    /// Replace a live credential with a secret-free tombstone scoped to
    /// `scope`.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::NotFound`] if absent or cross-tenant;
    /// [`CredentialServiceError::Store`] on a backend failure.
    pub(crate) async fn delete(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<(), CredentialServiceError> {
        let credential_id = CredentialId::parse(id)
            .map_err(|_| CredentialServiceError::NotFound { id: id.to_owned() })?;
        let selector = scope.selector(credential_id);
        // Tombstoning needs only the live structural version. Reading the
        // secret-bearing row would unnecessarily make revocation depend on
        // successful ciphertext decryption.
        let existing = self
            .store
            .get_head(&selector)
            .await
            .map_err(|error| Self::map_store_err_for(id, error))?;
        self.store
            .tombstone(&selector, CredentialTombstone::new(existing.version()))
            .await
            .map_err(|error| Self::map_store_err_for(id, error))?;
        tracing::info!(credential.id = %id, "credential tombstoned");
        Ok(())
    }
}

fn map_update_finalization_error(
    completion_evidence: Option<AcquisitionCompletionEvidence>,
    error: CredentialServiceError,
) -> CredentialServiceError {
    match completion_evidence {
        Some(evidence) => map_acquisition_finalization_error(evidence, error),
        None => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version_conflict() -> CredentialServiceError {
        CredentialServiceError::VersionConflict {
            id: "cred-1".to_owned(),
            expected: 1,
            actual: 2,
        }
    }

    #[test]
    fn material_update_finalization_failure_is_retry_unsafe() {
        let mapped = map_update_finalization_error(
            Some(AcquisitionCompletionEvidence::ProviderBoundaryUnproven),
            version_conflict(),
        );
        assert!(matches!(
            mapped,
            CredentialServiceError::AcquisitionFinalizationRequired
        ));
    }

    #[test]
    fn display_only_update_preserves_pre_provider_store_classification() {
        let mapped = map_update_finalization_error(None, version_conflict());
        assert!(matches!(
            mapped,
            CredentialServiceError::VersionConflict { .. }
        ));
    }

    #[test]
    fn material_update_lost_acknowledgement_remains_outcome_unknown() {
        let mapped = map_update_finalization_error(
            Some(AcquisitionCompletionEvidence::ProviderBoundaryUnproven),
            CredentialServiceError::OutcomeUnknown,
        );
        assert!(matches!(mapped, CredentialServiceError::OutcomeUnknown));
    }
}
