//! Owner-qualified, read-only slot projection and its consumer contract.
//!
//! Both management and worker runtimes delegate to this sequence. The head is
//! checked before decryption, then compared with the loaded material before a
//! guard is published. Refresh and management writes are outside this boundary.

use std::{any::Any, fmt, future::Future, pin::Pin};

use tokio_util::sync::CancellationToken;
use zeroize::Zeroize;

use crate::runtime::state_source::StateSource;
use crate::service::error::CredentialServiceError;
use crate::{
    Capabilities, CredentialGuard, CredentialId, CredentialKey, CredentialPersistenceError,
    ErasedPendingStore, StoredCredential, TenantScope,
};

pub(crate) struct SlotResolutionRequest<'a> {
    pub(crate) scope: &'a TenantScope,
    pub(crate) credential_id: CredentialId,
    pub(crate) expected_key: CredentialKey,
    pub(crate) required_capabilities: Capabilities,
    pub(crate) cancel: CancellationToken,
}

pub(crate) async fn resolve_slot_with(
    store: &dyn crate::CredentialPersistence,
    registry: &crate::CredentialRegistry,
    ops: &crate::DispatchOps<ErasedPendingStore>,
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
            let head = match store.get_head(&selector).await {
                Ok(head) => head,
                Err(CredentialPersistenceError::NotFound) => {
                    return match store.get(&selector).await {
                        Ok(StoredCredential::Tombstoned(_)) => {
                            Err(CredentialSlotResolveError::Revoked)
                        },
                        Ok(StoredCredential::Live(_)) => {
                            Err(CredentialSlotResolveError::InvalidState)
                        },
                        Err(CredentialPersistenceError::NotFound) => {
                            Err(CredentialSlotResolveError::NotFound)
                        },
                        Err(
                            CredentialPersistenceError::Unavailable
                            | CredentialPersistenceError::OutcomeUnknown,
                        ) => Err(CredentialSlotResolveError::Unavailable),
                        Err(_) => Err(CredentialSlotResolveError::InvalidState),
                    };
                },
                Err(
                    CredentialPersistenceError::Unavailable
                    | CredentialPersistenceError::OutcomeUnknown,
                ) => {
                    return Err(CredentialSlotResolveError::Unavailable);
                },
                Err(_) => return Err(CredentialSlotResolveError::InvalidState),
            };

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
                scope: Some(request.scope.durable_owner_scope()),
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
    scope: Option<TenantScope>,
}

impl CredentialGuardMetadata {
    /// Construct ordering metadata for a trusted projection adapter.
    #[must_use]
    pub fn new(
        credential_id: CredentialId,
        credential_key: CredentialKey,
        material_epoch: u64,
        revision: u64,
    ) -> Self {
        Self {
            credential_id,
            credential_key,
            material_epoch,
            revision,
            scope: None,
        }
    }

    /// Retain the owner-qualified read scope for durable reprojection.
    /// This is routing context, not an authorization grant.
    #[must_use]
    pub fn with_scope(mut self, scope: TenantScope) -> Self {
        self.scope = Some(scope.durable_owner_scope());
        self
    }

    /// Owner-qualified read scope, when supplied by the projection adapter.
    #[must_use]
    pub fn scope(&self) -> Option<&TenantScope> {
        self.scope.as_ref()
    }

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

    /// Erase a guard produced by a trusted projection adapter.
    pub fn from_typed<S>(guard: CredentialGuard<S>, metadata: CredentialGuardMetadata) -> Self
    where
        S: Zeroize + Send + Sync + 'static,
    {
        Self::new(Box::new(guard), metadata)
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
    /// The owner-qualified credential has a durable terminal tombstone.
    #[error("credential is revoked")]
    Revoked,
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
#[path = "slot_tests.rs"]
mod tests;
