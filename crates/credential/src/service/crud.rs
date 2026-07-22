//! CRUD surface of [`CredentialService`] — create / read / list / update /
//! delete of stored credential rows.
//!
//! Split out of `facade.rs` (behaviour-preserving code motion — no logic
//! change). Reads the same `pub(crate)` [`CredentialService`] internals
//! (`ensure_local_source`, `owner_context`, `load_owned`, `head_from`,
//! `owner_matches`, `set_display`, `map_store_err`) as the rest of the
//! service.

use serde_json::Value;

use crate::{
    CredentialDisplay, CredentialId, CredentialPersistenceError, CredentialWriteMode,
    LAST_VALIDATED_AT_METADATA_KEY, OWNER_ID_METADATA_KEY as OWNER_ID_KEY, StoredCredential,
};

use super::error::CredentialServiceError;
use super::facade::CredentialService;
use super::head::CredentialHead;
use super::scope::TenantScope;

impl CredentialService {
    /// Create a credential: validate `props` against the type's schema,
    /// resolve it to encrypted state, and persist it scoped to `scope`.
    ///
    /// The validation pipeline is the canonical credential pipeline
    /// (credential secrecy): `schema_of::<Properties>().validate(FieldValues)`
    /// then a typed `serde_json::from_value` round-trip — a `{"$expr": ..}`
    /// envelope survives schema validation but is refused by the typed
    /// deserialize, so secrets never depend on workflow state.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::TypeUnknown`] — no type registered under `credential_key`.
    /// - [`CredentialServiceError::ValidationFailed`] — schema or typed-deserialize rejection
    ///   (including `$expr` injection), or a resolve failure.
    /// - [`CredentialServiceError::Store`] — persistence failure, including a propagated audit
    ///   sink error. A sink error does not imply that an inner mutation was rolled back.
    pub async fn create(
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

        // Canonical validation pipeline: schema validate + typed
        // deserialize (the `$expr` refusal point) without ever resolving
        // expressions. Monomorphised per type in the ops table.
        self.ops.validate(credential_key, &props)?;

        // Union-aware ingress: a record `Properties` folds via `from_json`; a union
        // folds serde's tagged wire into the `{mode, value}` envelope `resolve`
        // consumes (per-type, keyed by the registered schema's `serde_tagging`).
        let values = self.ops.ingest(credential_key, &props)?;

        let id = CredentialId::new();
        let ctx = Self::owner_context(scope);

        let resolved = self
            .ops
            .resolve(credential_key, &values, &ctx, &self.pending)
            .await?;

        let head = self
            .persist_resolved(scope, credential_key, id, resolved, display)
            .await?;

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
    ) -> Result<CredentialHead, CredentialServiceError> {
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
        let stored = StoredCredential {
            id: id.to_string(),
            name: None,
            credential_key: credential_key.to_owned(),
            data: resolved.data.clone().into(),
            state_kind: resolved.state_kind,
            state_version: resolved.state_version,
            version: 0,
            created_at: now,
            updated_at: now,
            expires_at: resolved.expires_at,
            reauth_required: false,
            metadata,
        };

        // The store returns the persisted row (with its post-put version),
        // which is the authoritative source for the returned head — the
        // CAS token must reflect what a subsequent `update` has to match.
        let persisted = self
            .store
            .put(
                &scope.selector(stored.id.clone()),
                stored,
                CredentialWriteMode::CreateOnly,
            )
            .await
            .map_err(Self::map_store_err)?;

        Ok(Self::head_from(&persisted))
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
        let stored = match self.store.get_head(&scope.selector(id)).await {
            Ok(stored) => stored,
            Err(CredentialPersistenceError::NotFound { .. }) => {
                return Err(CredentialServiceError::NotFound { id: id.to_owned() });
            },
            Err(error) => return Err(Self::map_store_err(error)),
        };
        if !Self::projected_owner_matches(&stored, scope) || stored.is_tombstoned() {
            return Err(CredentialServiceError::NotFound { id: id.to_owned() });
        }
        Ok(Self::head_from_projection(&stored))
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
            .list_heads(scope.owner(), None)
            .await
            .map_err(Self::map_store_err)?;
        let mut visible = Vec::new();
        for stored in rows {
            // The adapter already owner-filters; keep the metadata check as
            // defence in depth against corrupt legacy rows.
            if Self::projected_owner_matches(&stored, scope) && !stored.is_tombstoned() {
                visible.push(Self::head_from_projection(&stored));
            }
        }
        Ok(visible)
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
    /// - [`CredentialServiceError::VersionConflict`] — stale `expected_version`.
    /// - [`CredentialServiceError::Store`] — persistence failure.
    pub async fn update(
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

        // Re-resolve only when new properties were supplied; a
        // display-only update carries the existing state through.
        let resolved = match props {
            Some(props) => {
                self.ops.validate(&existing.credential_key, &props)?;
                let values = self.ops.ingest(&existing.credential_key, &props)?;
                let ctx = Self::owner_context(scope);
                Some(
                    self.ops
                        .resolve(&existing.credential_key, &values, &ctx, &self.pending)
                        .await?,
                )
            },
            None => None,
        };

        let mut metadata = existing.metadata.clone();
        metadata.insert(
            OWNER_ID_KEY.to_owned(),
            Value::String(scope.owner_id().to_owned()),
        );
        Self::set_display(&mut metadata, &display);

        let now = chrono::Utc::now();
        let stored = match resolved {
            // Props supplied ⇒ re-resolved against the provider ⇒ stamp the
            // validation time. A display-only edit (the `None` arm) preserves the
            // existing stamp and bumps only `updated_at`, so it cannot postpone
            // the re-validation floor.
            Some(resolved) => {
                metadata.insert(
                    LAST_VALIDATED_AT_METADATA_KEY.to_owned(),
                    Value::String(now.to_rfc3339()),
                );
                StoredCredential {
                    id: existing.id.clone(),
                    name: existing.name.clone(),
                    credential_key: existing.credential_key.clone(),
                    data: resolved.data.clone().into(),
                    state_kind: resolved.state_kind,
                    state_version: resolved.state_version,
                    version: existing.version,
                    created_at: existing.created_at,
                    updated_at: now,
                    expires_at: resolved.expires_at,
                    reauth_required: false,
                    metadata,
                }
            },
            None => StoredCredential {
                updated_at: now,
                metadata,
                ..existing.clone()
            },
        };

        // No blind-overwrite path: when the caller supplied no version,
        // CAS on the version loaded above. A display-only rename racing a
        // token refresh must conflict, never silently restore the stale
        // secret bytes captured at load time.
        let mode = CredentialWriteMode::CompareAndSwap {
            expected_version: expected_version.unwrap_or(existing.version),
        };

        let persisted = self
            .store
            .put(&scope.selector(id), stored, mode)
            .await
            .map_err(Self::map_store_err)?;

        tracing::info!(credential.id = %id, "credential updated");
        Ok(Self::head_from(&persisted))
    }

    /// Delete a credential scoped to `scope`.
    ///
    /// # Errors
    ///
    /// [`CredentialServiceError::NotFound`] if absent or cross-tenant;
    /// [`CredentialServiceError::Store`] on a backend failure.
    pub async fn delete(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<(), CredentialServiceError> {
        // Owner check: cross-tenant delete is indistinguishable from a
        // missing credential.
        let _existing = self.get(scope, id).await?;
        self.store
            .delete(&scope.selector(id))
            .await
            .map_err(Self::map_store_err)?;
        tracing::info!(credential.id = %id, "credential deleted");
        Ok(())
    }
}
