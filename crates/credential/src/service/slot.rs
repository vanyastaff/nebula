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
    Credential, CredentialGuard, CredentialId, CredentialLifecycle, CredentialPersistenceError,
    Refreshable, SchemeFactory, StoredCredential, runtime::ResolveError,
};

use super::error::CredentialServiceError;
use super::facade::CredentialService;
use super::scope::TenantScope;
use super::state_source::StateSource;

fn map_slot_resolve_error(
    source: &StateSource,
    requested_id: &str,
    error: ResolveError,
) -> CredentialServiceError {
    match error {
        ResolveError::Store(error) => CredentialService::map_store_err_for(requested_id, error),
        ResolveError::ExternalSourceNotWired => CredentialServiceError::ExternalSourceNotWired {
            provider: match source {
                StateSource::External(provider) => provider.provider_name().to_owned(),
                StateSource::LocalEncrypted => "unknown".to_owned(),
            },
        },
        ResolveError::ReauthRequired {
            credential_id,
            reason,
        } => CredentialServiceError::ReauthRequired {
            credential_id,
            reason,
        },
        ResolveError::RefreshOutcomePending { .. }
        | ResolveError::ProviderOutcomeUnknown { .. }
        | ResolveError::PostProviderPersistence {
            source: CredentialPersistenceError::OutcomeUnknown,
            ..
        } => CredentialServiceError::OutcomeUnknown,
        ResolveError::PostProviderPersistence { .. }
        | ResolveError::PostProviderStateEncoding { .. } => {
            CredentialServiceError::PostProviderPersistence
        },
        other => CredentialServiceError::Internal(other.to_string()),
    }
}

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
    /// The owner-qualified storage predicate deliberately makes both cases the
    /// same `NotFound`; metadata is never consulted as authority.
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
        let credential_id = CredentialId::parse(id).map_err(|_| {
            super::binding::ValidatedCredentialBindingError::NotFound { id: id.to_owned() }
        })?;
        let stored = match self.store.get(&scope.selector(credential_id)).await {
            Ok(stored) => stored,
            Err(CredentialPersistenceError::NotFound) => {
                return Err(super::binding::ValidatedCredentialBindingError::NotFound {
                    id: id.to_owned(),
                });
            },
            Err(error) => {
                return Err(super::binding::ValidatedCredentialBindingError::Io(
                    Self::map_store_err_for(id, error),
                ));
            },
        };

        if let StoredCredential::Tombstoned(tombstone) = stored {
            return Err(
                super::binding::ValidatedCredentialBindingError::CredentialTombstoned {
                    id: id.to_owned(),
                    revoked_at: Some(tombstone.tombstoned_at()),
                },
            );
        }

        Ok(super::binding::ValidatedCredentialBinding::new(
            credential_id,
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
    /// around this method's read-only `resolve_scoped` delegation. On
    /// cancellation, returns [`CredentialServiceError::Cancelled`] without
    /// partial state. Provider-refreshing acquisition is deliberately not
    /// hidden inside this wrapper: long-lived refreshable consumers enter via
    /// [`scheme_factory`](Self::scheme_factory), whose resolver path transfers
    /// provider+persistence work to the owned coordinator before exposing an
    /// outer wait/cancellation boundary.
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
                let handle =
                    self.resolver
                        .resolve_scoped::<C>(&selector)
                        .await
                        .map_err(|error| {
                            // Preserve phase-aware no-replay taxonomy across the
                            // resolver/service boundary.
                            map_slot_resolve_error(&self.source, &credential_id.to_string(), error)
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
            credential.id = %credential_id,
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
    pub fn scheme_factory<C>(
        &self,
        scope: &TenantScope,
        credential_id: CredentialId,
    ) -> SchemeFactory<C>
    where
        C: Refreshable + CredentialLifecycle,
        C::Scheme: Zeroize + Clone + Send + Sync + 'static,
    {
        self.resolver
            .scheme_factory(scope.selector(credential_id), Self::owner_context(scope))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_mapping_preserves_unknown_and_definite_post_provider_failures() {
        let source = StateSource::LocalEncrypted;
        let unknown = map_slot_resolve_error(
            &source,
            "cred-test",
            ResolveError::ProviderOutcomeUnknown {
                credential_id: "cred-test".to_owned(),
            },
        );
        assert!(matches!(unknown, CredentialServiceError::OutcomeUnknown));

        let lost_commit = map_slot_resolve_error(
            &source,
            "cred-test",
            ResolveError::PostProviderPersistence {
                credential_id: "cred-test".to_owned(),
                source: CredentialPersistenceError::OutcomeUnknown,
            },
        );
        assert!(matches!(
            lost_commit,
            CredentialServiceError::OutcomeUnknown
        ));

        let definite = map_slot_resolve_error(
            &source,
            "cred-test",
            ResolveError::PostProviderStateEncoding {
                credential_id: "cred-test".to_owned(),
                reason: "closed test failure".to_owned(),
            },
        );
        assert!(matches!(
            definite,
            CredentialServiceError::PostProviderPersistence
        ));
    }
}
