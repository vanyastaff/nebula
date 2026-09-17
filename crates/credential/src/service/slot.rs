//! Slot / binding resolution surface of [`CredentialService`].
//!
//! Contains the execution-time binding validation and typed-guard
//! resolution methods that engine and action code consume —
//! [`validate_credential_binding`](CredentialService::validate_credential_binding),
//! [`resolve_for_slot`](CredentialService::resolve_for_slot),
//! [`scheme_factory`](CredentialService::scheme_factory), and the raw-load
//! helper they share. Kept in the `service` module so it reads the same
//! `pub(crate)` [`CredentialService`] internals as the CRUD facade.

use std::{any::Any, fmt, future::Future, pin::Pin};

use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

use crate::{
    Capabilities, Credential, CredentialGuard, CredentialId, CredentialKey, CredentialLifecycle,
    CredentialPersistenceError, Refreshable, SchemeFactory, StateWireFingerprint, StoredCredential,
};

#[cfg(test)]
use crate::runtime::ResolveError;

use super::error::CredentialServiceError;
use super::facade::CredentialService;
use super::scope::TenantScope;
use super::state_source::StateSource;

pub(super) struct SlotResolutionRequest<'a> {
    pub(super) scope: &'a TenantScope,
    pub(super) credential_id: CredentialId,
    pub(super) expected_key: CredentialKey,
    pub(super) required_capabilities: Capabilities,
    pub(super) cancel: CancellationToken,
}

pub(super) async fn resolve_slot_with(
    store: &dyn crate::CredentialPersistence,
    registry: &crate::CredentialRegistry,
    ops: &super::DispatchOps<crate::ErasedPendingStore>,
    source: &StateSource,
    request: SlotResolutionRequest<'_>,
) -> Result<ErasedCredentialGuard, CredentialSlotResolveError> {
    if !matches!(source, StateSource::LocalEncrypted) {
        return Err(CredentialSlotResolveError::SourceUnavailable);
    }

    request
        .cancel
        .run_until_cancelled(async {
            // Ownership is the first row-dependent decision. Until this
            // complete selector succeeds, key, capability, and lifecycle
            // state remain deliberately unobservable.
            let selector = request.scope.selector(request.credential_id);
            let head = store
                .get_head(&selector)
                .await
                .map_err(|error| match error {
                    CredentialPersistenceError::NotFound => CredentialSlotResolveError::NotFound,
                    CredentialPersistenceError::Unavailable
                    | CredentialPersistenceError::OutcomeUnknown => {
                        CredentialSlotResolveError::Unavailable
                    },
                    _ => CredentialSlotResolveError::InvalidState,
                })?;

            let actual_key = CredentialKey::new(head.credential_key())
                .map_err(|_| CredentialSlotResolveError::InvalidState)?;
            if actual_key != request.expected_key {
                return Err(CredentialSlotResolveError::WrongCredentialKey);
            }

            let capabilities = registry
                .capabilities_of(actual_key.as_str())
                .ok_or(CredentialSlotResolveError::InvalidState)?;
            if !capabilities.contains(request.required_capabilities) {
                return Err(CredentialSlotResolveError::MissingCapabilities);
            }
            if head.reauth_required() {
                return Err(CredentialSlotResolveError::ReauthRequired);
            }

            // Only after the owner-qualified secret-free checks pass may the
            // layered store decrypt material. Re-check the head identity and
            // ordering tuple so a concurrent mutation cannot swap the value
            // validated above for a different projection.
            let stored = store.get(&selector).await.map_err(|error| match error {
                CredentialPersistenceError::NotFound => CredentialSlotResolveError::NotFound,
                CredentialPersistenceError::Unavailable
                | CredentialPersistenceError::OutcomeUnknown => {
                    CredentialSlotResolveError::Unavailable
                },
                _ => CredentialSlotResolveError::InvalidState,
            })?;
            let StoredCredential::Live(stored) = stored else {
                return Err(CredentialSlotResolveError::InvalidState);
            };
            if stored.credential_id() != head.credential_id()
                || stored.credential_key() != head.credential_key()
                || stored.version() != head.version()
                || stored.material_epoch() != head.material_epoch()
                || stored.state_kind() != head.state_kind()
                || stored.state_version() != head.state_version()
                || stored.reauth_required() != head.reauth_required()
            {
                return Err(CredentialSlotResolveError::InvalidState);
            }

            let inner = ops
                .project_guard(
                    actual_key.as_str(),
                    stored.data(),
                    stored.state_kind(),
                    stored.state_version(),
                )
                .map_err(|error| match error {
                    // Envelope refusals keep their distinct, secret-free
                    // classification — never collapsed into `InvalidState`.
                    CredentialServiceError::StateEnvelopeRefused(envelope) => {
                        CredentialSlotResolveError::StoredStateRefused(envelope)
                    },
                    _ => CredentialSlotResolveError::InvalidState,
                })?;
            let metadata = CredentialGuardMetadata {
                credential_id: request.credential_id,
                credential_key: actual_key,
                material_epoch: stored.material_epoch().get() as u64,
                revision: stored.version().get() as u64,
            };

            tracing::debug!(
                credential.key = metadata.credential_key().as_str(),
                credential.material_epoch = metadata.material_epoch(),
                credential.revision = metadata.revision(),
                "credential projected for slot"
            );
            Ok(ErasedCredentialGuard::new(inner, metadata))
        })
        .await
        .ok_or(CredentialSlotResolveError::Cancelled)?
}

/// Secret-free ordering metadata attached to a resolved credential guard.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialGuardMetadata {
    credential_id: CredentialId,
    credential_key: CredentialKey,
    material_epoch: u64,
    revision: u64,
}

impl CredentialGuardMetadata {
    /// Credential instance that produced the guard.
    #[must_use]
    pub fn credential_id(&self) -> CredentialId {
        self.credential_id
    }

    /// Registered credential contract used for projection.
    #[must_use]
    pub const fn credential_key(&self) -> &CredentialKey {
        &self.credential_key
    }

    /// Backend-authored material epoch used to reject stale refreshes.
    #[must_use]
    pub const fn material_epoch(&self) -> u64 {
        self.material_epoch
    }

    /// Persisted aggregate revision observed with the material epoch.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

impl fmt::Debug for CredentialGuardMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialGuardMetadata")
            .field("credential_id", &"[redacted]")
            .field("credential_key", &self.credential_key)
            .field("material_epoch", &self.material_epoch)
            .field("revision", &self.revision)
            .finish()
    }
}

/// Opaque type-erased projected credential guard.
///
/// The erased storage is private and can only be consumed through
/// [`into_typed`](Self::into_typed), which verifies the concrete scheme type.
#[must_use = "credential guards must be held for the duration of use"]
pub struct ErasedCredentialGuard {
    inner: Box<dyn Any + Send + Sync>,
    metadata: CredentialGuardMetadata,
}

impl ErasedCredentialGuard {
    fn new(inner: Box<dyn Any + Send + Sync>, metadata: CredentialGuardMetadata) -> Self {
        Self { inner, metadata }
    }

    /// Borrow the secret-free ordering metadata.
    #[must_use]
    pub const fn metadata(&self) -> &CredentialGuardMetadata {
        &self.metadata
    }

    /// Recover the projected guard when `S` is the registered scheme type.
    ///
    /// # Errors
    ///
    /// Returns [`ErasedCredentialGuardTypeError`] when `S` differs from the
    /// scheme selected by the validated credential key. The erased guard is
    /// dropped and zeroized on this path.
    pub fn into_typed<S>(self) -> Result<CredentialGuard<S>, ErasedCredentialGuardTypeError>
    where
        S: Zeroize + Send + Sync + 'static,
    {
        self.inner
            .downcast::<CredentialGuard<S>>()
            .map(|guard| *guard)
            .map_err(|_| ErasedCredentialGuardTypeError)
    }
}

impl fmt::Debug for ErasedCredentialGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ErasedCredentialGuard")
            .field("metadata", &self.metadata)
            .field("guard", &"[REDACTED]")
            .finish()
    }
}

/// The requested concrete scheme does not match an erased projected guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("resolved credential guard has a different scheme type")]
pub struct ErasedCredentialGuardTypeError;

/// Closed, secret-free failures from [`CredentialSlotResolver`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialSlotResolveError {
    /// The id is absent, malformed, or belongs to another tenant.
    #[error("credential not found")]
    NotFound,
    /// The stored credential contract differs from the slot declaration.
    #[error("credential key does not match slot contract")]
    WrongCredentialKey,
    /// The credential does not provide every capability required by the slot.
    #[error("credential does not satisfy slot capabilities")]
    MissingCapabilities,
    /// The credential requires re-authentication before it can be projected.
    #[error("credential requires re-authentication")]
    ReauthRequired,
    /// Resolution was cancelled before a guard was published.
    #[error("credential slot resolution cancelled")]
    Cancelled,
    /// Credential persistence is temporarily unavailable.
    #[error("credential persistence is temporarily unavailable")]
    Unavailable,
    /// Stored state or runtime registration is inconsistent.
    #[error("credential slot state is invalid")]
    InvalidState,
    /// The configured external credential source is not available.
    #[error("credential source is unavailable")]
    SourceUnavailable,
    /// The persisted state failed a fail-closed state-envelope check
    /// (schema fingerprint, kind tag, or version) and is reported as
    /// stored rather than as invalid local state.
    #[error("stored credential state was refused: {0}")]
    StoredStateRefused(crate::StateEnvelopeError),
}

/// Object-safe tenant-scoped credential slot projection boundary.
pub trait CredentialSlotResolver: Send + Sync {
    /// Resolve one owner-qualified credential into an opaque projected guard.
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
    >;
}

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
    /// hidden inside this wrapper: long-lived refreshable consumers enter via
    /// [`scheme_factory`](Self::scheme_factory), whose resolver path transfers
    /// provider+persistence work to the owned coordinator before exposing an
    /// outer wait/cancellation boundary.
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
        C::State: StateWireFingerprint,
    {
        self.resolver
            .scheme_factory(scope.selector(credential_id), self.owner_context(scope))
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
