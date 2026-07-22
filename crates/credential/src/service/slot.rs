//! Slot / binding resolution surface of [`CredentialService`].
//!
//! Split out of `facade.rs` (behaviour-preserving code motion — no logic
//! change): the execution-time binding validation and typed-guard
//! resolution methods that engine and action code consume —
//! [`validate_credential_binding`](CredentialService::validate_credential_binding),
//! [`resolve_for_slot`](CredentialService::resolve_for_slot),
//! [`scheme_factory`](CredentialService::scheme_factory), and the raw-load
//! helper they share. Kept in the `service` module so it reads the same
//! `pub(crate)` [`CredentialService`] internals as the CRUD facade.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

use crate::{
    Credential, CredentialGuard, CredentialLifecycle, CredentialPersistenceError,
    OWNER_ID_METADATA_KEY as OWNER_ID_KEY, Refreshable, SchemeFactory, StoredCredential,
};

use super::error::CredentialServiceError;
use super::facade::CredentialService;
use super::scope::TenantScope;
use super::state_source::StateSource;

impl CredentialService {
    /// Validate a workflow `slot_bindings` reference against the caller's
    /// tenant scope, returning a typed
    /// [`ValidatedCredentialBinding`](crate::ValidatedCredentialBinding) that
    /// engine execution consumes.
    ///
    /// This is the **only construction path** for
    /// `ValidatedCredentialBinding`. Its `pub(crate)` constructor is
    /// unreachable from outside `nebula-credential`, so engine code
    /// that consumes the handle has a structural proof that the scope-check
    /// already ran.
    ///
    /// # Cross-tenant behaviour
    ///
    /// A cross-tenant probe (the id exists but belongs to a different tenant)
    /// returns [`crate::ValidatedCredentialBindingError::NotFound`] —
    /// existence-hiding, matching every other cross-tenant read in this service.
    /// The real owner is logged internally (at `WARN`) for operator visibility
    /// but is never surfaced to the caller, so an ordinary caller cannot
    /// distinguish "id absent" from "id owned by another tenant". Workflow
    /// authors can diagnose a misconfigured binding by checking service logs
    /// (operator-accessible) rather than by parsing the error message.
    ///
    /// The raw read path (`store_load_raw`) is used so the owner field is
    /// readable for the internal log; it does not bypass any other gate.
    ///
    /// # Errors
    ///
    /// - [`crate::ValidatedCredentialBindingError::NotFound`] — id absent from
    ///   the store **or** id exists but belongs to a different tenant
    ///   (existence-hiding — the two cases are indistinguishable to callers).
    /// - [`crate::ValidatedCredentialBindingError::CredentialTombstoned`] — id
    ///   is owned by the caller but has been revoked.
    /// - [`crate::ValidatedCredentialBindingError::Io`] — underlying store error.
    pub async fn validate_credential_binding(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<
        super::binding::ValidatedCredentialBinding,
        super::binding::ValidatedCredentialBindingError,
    > {
        let stored = self
            .store_load_raw(scope, id)
            .await
            .map_err(super::binding::ValidatedCredentialBindingError::Io)?
            .ok_or_else(
                || super::binding::ValidatedCredentialBindingError::NotFound { id: id.to_owned() },
            )?;

        let owner = stored
            .metadata
            .get(OWNER_ID_KEY)
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if owner != scope.owner_id() {
            // Log the real owner for internal audit/tracing but do NOT expose it
            // to the caller. Returning the actual owning tenant in a
            // `ScopeMismatch` error would be a cross-tenant existence oracle — an
            // ordinary caller could enumerate which ids are owned by other tenants
            // by observing whether the mismatch error names their id. We return
            // `NotFound` instead, matching every other cross-tenant probe in this
            // service (existence-hiding). The structured diagnostic lives only in
            // the trace, where it is visible to operators but not to API consumers.
            tracing::warn!(
                credential.id = id,
                requested_owner = scope.owner_id(),
                actual_owner = %owner,
                "credential binding validation: owner mismatch (cross-tenant probe or misconfigured binding)"
            );
            return Err(super::binding::ValidatedCredentialBindingError::NotFound {
                id: id.to_owned(),
            });
        }

        // Reject a revoked credential here — before any binding (and thus any
        // guard) is produced — with a typed `CredentialTombstoned` rather than
        // a bare `NotFound`, so the caller learns the slot stopped resolving
        // because the credential was revoked. The check is owner-gated above,
        // so it never reveals another tenant's revoke status. No reverse
        // `references()` index is consulted: the tombstone travels with the row.
        if stored.is_tombstoned() {
            return Err(
                super::binding::ValidatedCredentialBindingError::CredentialTombstoned {
                    id: id.to_owned(),
                    revoked_at: stored.revoked_at(),
                },
            );
        }

        Ok(super::binding::ValidatedCredentialBinding::new(
            id.to_owned(),
            super::binding::TenantFingerprint::from_scope(scope),
        ))
    }

    /// Production execution-time resolver. Consumes a tenant-validated
    /// binding (from [`validate_credential_binding`]) and produces a typed
    /// [`CredentialGuard<C::Scheme>`] for an action slot.
    ///
    /// # Hot path
    ///
    /// Called once per action node per execution. The engine resolver
    /// (`CredentialResolver::resolve`) goes through the full layered-store
    /// stack (`Audit(Encryption(raw))`) composed by the deployment application.
    /// `EncryptionLayer` decrypts the selected row and `AuditLayer` records the
    /// access. No storage cache layer is part of the current first-party
    /// composition; performance claims require measurement rather than an
    /// assumed warm-cache path.
    ///
    /// # Cancellation
    ///
    /// `cancel` is observed via [`CancellationToken::run_until_cancelled`]
    /// wrapping the entire resolver delegation. On cancellation, returns
    /// [`CredentialServiceError::Cancelled`] without partial state.
    ///
    /// # Defence in depth
    ///
    /// Re-checks the binding's tenant fingerprint against `scope` even
    /// though [`validate_credential_binding`] already enforced it at
    /// construction — type-safe consumption with a runtime sanity arm that
    /// fires if a binding is presented against the wrong scope.
    ///
    /// # Errors
    ///
    /// - [`CredentialServiceError::ScopeViolation`] — binding's tenant
    ///   fingerprint does not match `scope`.
    /// - [`CredentialServiceError::Cancelled`] — `cancel` fired.
    /// - [`CredentialServiceError::NotFound`] — credential absent from store.
    /// - [`CredentialServiceError::Internal`] — resolver error (kind
    ///   mismatch, deserialisation failure, or store error).
    ///
    /// [`validate_credential_binding`]: Self::validate_credential_binding
    pub async fn resolve_for_slot<C>(
        &self,
        scope: &TenantScope,
        binding: &super::binding::ValidatedCredentialBinding,
        cancel: CancellationToken,
    ) -> Result<CredentialGuard<C::Scheme>, CredentialServiceError>
    where
        C: Credential,
        C::Scheme: Zeroize + Clone,
    {
        // Source gate is enforced at the resolver tail (`resolve_scoped` →
        // `ensure_source_wired`), set from the configured `StateSource` at
        // `from_secure_parts`. A service with an external (unwired) source
        // therefore fails closed by construction here — and on the
        // `scheme_factory` direct path that never reaches this method — instead
        // of relying on a per-call check this moat path could forget. The
        // mapping below turns the resolver's `ExternalSourceNotWired` into the
        // facade error.

        // 1. Defence-in-depth fingerprint check: even though
        //    `validate_credential_binding` enforced the scope at
        //    construction, re-verify here so mismatched bindings fail
        //    loudly at the consume site.
        let expected_fp = super::binding::TenantFingerprint::from_scope(scope);
        if binding.fingerprint() != &expected_fp {
            return Err(CredentialServiceError::ScopeViolation {
                requested: scope.owner_id().to_string(),
            });
        }

        // 2. Delegate to the resolver, wrapped in cancellation. The current
        //    first-party factory supplies Audit(Encryption(raw)), so the
        //    EncryptionLayer is not bypassed. No storage cache layer is part
        //    of this composition.
        let credential_id = binding.credential_id();
        // Resolve through the binding's owner-scoped key: the resolver re-checks
        // the stored row's owner at load, so a cross-tenant id fails closed
        // (`NotFound`) by construction rather than relying on the fingerprint
        // check above alone.
        let selector = binding.selector();
        let scheme = cancel
            .run_until_cancelled(async {
                let handle = self
                    .resolver
                    .resolve_scoped::<C>(&selector)
                    .await
                    .map_err(|e| {
                        // Preserve the documented `NotFound` contract for
                        // resolver lookup misses. The resolver wraps store
                        // errors in `ResolveError::Store(CredentialPersistenceError::NotFound)`
                        // — surface that as `CredentialServiceError::NotFound`
                        // so callers can branch on it. Other resolver errors
                        // collapse to `Internal` with the underlying message.
                        use crate::runtime::ResolveError;
                        match e {
                            ResolveError::Store(CredentialPersistenceError::NotFound {
                                credential_id,
                            }) => CredentialServiceError::NotFound { id: credential_id },
                            // The resolver tail fail-closed on an external, unwired
                            // source. Surface the typed facade error with the
                            // configured provider's name (only `External` reaches
                            // this arm — the gate is derived from the source).
                            ResolveError::ExternalSourceNotWired => {
                                CredentialServiceError::ExternalSourceNotWired {
                                    provider: match &self.source {
                                        StateSource::External(p) => p.provider_name().to_owned(),
                                        StateSource::LocalEncrypted => "unknown".to_owned(),
                                    },
                                }
                            },
                            // Re-auth is a routine OAuth2 outcome (rejected grant /
                            // sentinel / missing material), not an internal fault —
                            // preserve the typed reason so the API can render a
                            // "reconnect" instead of a 500 with a stringified reason.
                            ResolveError::ReauthRequired {
                                credential_id,
                                reason,
                            } => CredentialServiceError::ReauthRequired {
                                credential_id,
                                reason,
                            },
                            other => CredentialServiceError::Internal(other.to_string()),
                        }
                    })?;

                // Extract the owned scheme from the snapshot `Arc`. The
                // resolver caches live handles, so `try_unwrap` succeeds when
                // this is the only outstanding snapshot; otherwise clone.
                let arc = handle.snapshot();
                let owned = Arc::try_unwrap(arc).unwrap_or_else(|arc| (*arc).clone());
                Ok::<_, CredentialServiceError>(owned)
            })
            .await
            .ok_or(CredentialServiceError::Cancelled)??;

        tracing::debug!(
            credential.id = credential_id,
            "credential resolved for slot"
        );
        Ok(CredentialGuard::new(scheme))
    }

    /// Per-request scheme re-acquisition for long-lived resources (§15.7).
    ///
    /// Stash the returned [`SchemeFactory`] on the resource instance at
    /// `create` and call [`SchemeFactory::acquire`] once per outbound
    /// request instead of retaining a [`CredentialGuard`] across spawn
    /// boundaries (which is forbidden — see SEC-05).
    pub fn scheme_factory<C>(&self, scope: &TenantScope, credential_id: &str) -> SchemeFactory<C>
    where
        C: Refreshable + CredentialLifecycle,
        C::Scheme: Zeroize + Clone + Send + Sync + 'static,
    {
        self.resolver
            .scheme_factory(scope.selector(credential_id), Self::owner_context(scope))
    }

    /// Load the raw stored credential row **without** applying the
    /// `owner_id` existence-hiding gate that `load_owned` enforces.
    ///
    /// `pub(crate)` — callers outside this crate cannot bypass the tenant
    /// isolation enforced by the public operations. The only in-crate
    /// caller today is `validate_credential_binding`, which reads the stored
    /// `owner_id` to compare against the requested scope (the result of that
    /// comparison is logged internally but not returned to callers).
    pub(crate) async fn store_load_raw(
        &self,
        scope: &TenantScope,
        id: &str,
    ) -> Result<Option<StoredCredential>, CredentialServiceError> {
        match self.store.get(&scope.selector(id)).await {
            Ok(stored) => Ok(Some(stored)),
            Err(CredentialPersistenceError::NotFound { .. }) => Ok(None),
            Err(e) => Err(Self::map_store_err(e)),
        }
    }
}
