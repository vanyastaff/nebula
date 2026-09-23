//! Management-service adapters for credential bindings and projection.
//!
//! The read-only projection contract and implementation live in
//! `runtime::projection`; this module maps them to the service API and owns
//! binding validation.

use std::{future::Future, pin::Pin};

use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

use crate::runtime::projection::slot::{
    CredentialSlotResolveError, CredentialSlotResolver, ErasedCredentialGuard,
    SlotResolutionRequest, resolve_slot_with,
};
use crate::{
    Capabilities, Credential, CredentialGuard, CredentialId, CredentialKey,
    CredentialPersistenceError, StoredCredential, TenantScope,
};

#[cfg(test)]
use crate::runtime::{ResolveError, state_source::StateSource};

use super::error::CredentialServiceError;
use super::facade::CredentialService;

#[cfg(test)]
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
        ResolveError::RefreshNotApplied { context, .. } => {
            CredentialServiceError::RefreshNotApplied(context)
        },
        ResolveError::RefreshOutcomePending { .. }
        | ResolveError::ProviderOutcomeUnknown { .. }
        | ResolveError::PostProviderPersistence {
            source: CredentialPersistenceError::OutcomeUnknown,
            ..
        } => CredentialServiceError::OutcomeUnknown,
        ResolveError::RefreshRetryGateFinalization { .. } => {
            CredentialServiceError::RefreshRetryGateFinalization
        },
        ResolveError::RefreshReconciliationRequired { .. } => {
            CredentialServiceError::RefreshReconciliationRequired
        },
        ResolveError::ReauthDecisionFinalization { .. } => {
            CredentialServiceError::ReauthDecisionFinalization
        },
        ResolveError::PostProviderPersistence { .. }
        | ResolveError::PostProviderStateEncoding { .. } => {
            CredentialServiceError::RefreshPostProviderPersistence
        },
        other => CredentialServiceError::Internal(other.to_string()),
    }
}

impl CredentialService {
    async fn resolve_slot_erased(
        &self,
        scope: &TenantScope,
        credential_id: CredentialId,
        expected_key: CredentialKey,
        required_capabilities: Capabilities,
        cancel: CancellationToken,
    ) -> Result<ErasedCredentialGuard, CredentialSlotResolveError> {
        resolve_slot_with(
            self.store.as_ref(),
            self.registry.as_ref(),
            self.ops.as_ref(),
            &self.source,
            SlotResolutionRequest {
                scope,
                credential_id,
                expected_key,
                required_capabilities,
                cancel,
            },
        )
        .await
    }

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
    /// Called once per action node per execution. Resolution first reads an
    /// owner-qualified secret-free head and validates the expected key and
    /// capabilities. Only then does the full layered-store stack
    /// (`Audit(Encryption(raw))`) decrypt the selected row for projection.
    ///
    /// # Cancellation
    ///
    /// `cancel` is observed via [`CancellationToken::run_until_cancelled`]
    /// around this method's read-only head/load/projection sequence. On
    /// cancellation, returns [`CredentialServiceError::Cancelled`] without
    /// partial state. Provider-refreshing acquisition is deliberately not
    /// hidden inside this wrapper. Management refresh enters through
    /// [`crate::CredentialController`], which authorizes the command before the
    /// coordinator owns the provider/persistence critical section.
    ///
    /// # Defence in depth
    ///
    /// Re-runs an owner-qualified lookup from `scope` instead of trusting the
    /// binding's provenance. It also compares the material-bearing row with
    /// the validated head to reject concurrent replacement before projection.
    ///
    /// # Errors
    ///
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
        C::Scheme: Zeroize,
    {
        let credential_id = binding.credential_id();
        let expected_key =
            CredentialKey::new(C::KEY).map_err(|_| CredentialServiceError::InvalidSlotState)?;
        let erased = self
            .resolve_slot_erased(
                scope,
                credential_id,
                expected_key,
                Capabilities::empty(),
                cancel,
            )
            .await
            .map_err(|error| match error {
                CredentialSlotResolveError::NotFound => CredentialServiceError::NotFound {
                    id: credential_id.to_string(),
                },
                CredentialSlotResolveError::Cancelled => CredentialServiceError::Cancelled,
                CredentialSlotResolveError::Unavailable => {
                    CredentialServiceError::PersistenceUnavailable
                },
                CredentialSlotResolveError::SourceUnavailable => {
                    CredentialServiceError::ExternalSourceNotWired {
                        provider: "configured".to_owned(),
                    }
                },
                CredentialSlotResolveError::ReauthRequired => {
                    CredentialServiceError::ReauthRequired {
                        credential_id: credential_id.to_string(),
                        reason: crate::ReauthReason::ProviderRejected,
                    }
                },
                CredentialSlotResolveError::StoredStateRefused(envelope) => {
                    CredentialServiceError::StateEnvelopeRefused(envelope)
                },
                CredentialSlotResolveError::WrongCredentialKey
                | CredentialSlotResolveError::MissingCapabilities
                | CredentialSlotResolveError::InvalidState => {
                    CredentialServiceError::InvalidSlotState
                },
            })?;
        erased
            .into_typed::<C::Scheme>()
            .map_err(|_| CredentialServiceError::InvalidSlotState)
    }
}

impl CredentialSlotResolver for CredentialService {
    fn resolve_slot<'a>(
        &'a self,
        scope: &'a TenantScope,
        credential_id: CredentialId,
        expected_key: CredentialKey,
        required_capabilities: Capabilities,
        cancel: CancellationToken,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<ErasedCredentialGuard, CredentialSlotResolveError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(self.resolve_slot_erased(
            scope,
            credential_id,
            expected_key,
            required_capabilities,
            cancel,
        ))
    }
}

#[cfg(test)]
#[path = "slot_tests.rs"]
mod tests;
