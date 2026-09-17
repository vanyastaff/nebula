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
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use chrono::Utc;
    use nebula_storage_port::{
        CredentialCommit, CredentialCreate, CredentialMaterialEpoch, CredentialOwner,
        CredentialReplacement, CredentialSelector, CredentialTombstone, CredentialVersion,
        RefreshRetrySnapshot, SecretBytes, StoredCredentialHead, StoredLiveCredential,
    };

    use super::*;
    use crate::error::{
        RefreshErrorKind, RefreshFailureSpec, RefreshNotAppliedContext, RefreshNotAppliedPhase,
        RetryAdvice,
    };
    use crate::{
        BearerTokenCredential, CredentialState, SecretString, SharedKey, scheme::SecretToken,
    };

    const SECRET_CANARY: &str = "slot-resolver-secret-NEVER-DEBUG-994";

    #[derive(Debug)]
    struct SlotStore {
        owner: CredentialOwner,
        row: StoredCredential,
        material_loads: AtomicUsize,
    }

    #[async_trait]
    impl crate::CredentialPersistence for SlotStore {
        async fn get(
            &self,
            selector: &CredentialSelector,
        ) -> Result<StoredCredential, CredentialPersistenceError> {
            self.material_loads.fetch_add(1, Ordering::Relaxed);
            if selector.owner() == &self.owner
                && selector.credential_id() == self.row.credential_id()
            {
                Ok(self.row.clone())
            } else {
                Err(CredentialPersistenceError::NotFound)
            }
        }

        async fn get_head(
            &self,
            selector: &CredentialSelector,
        ) -> Result<StoredCredentialHead, CredentialPersistenceError> {
            if selector.owner() != &self.owner
                || selector.credential_id() != self.row.credential_id()
            {
                return Err(CredentialPersistenceError::NotFound);
            }
            match &self.row {
                StoredCredential::Live(stored) => Ok(StoredCredentialHead::from(stored)),
                StoredCredential::Tombstoned(_) => Err(CredentialPersistenceError::NotFound),
            }
        }

        async fn refresh_retry_snapshot(
            &self,
            _selector: &CredentialSelector,
        ) -> Result<RefreshRetrySnapshot, CredentialPersistenceError> {
            Err(CredentialPersistenceError::Unavailable)
        }

        async fn create(
            &self,
            _selector: &CredentialSelector,
            _create: CredentialCreate,
        ) -> Result<CredentialCommit, CredentialPersistenceError> {
            Err(CredentialPersistenceError::Unavailable)
        }

        async fn replace(
            &self,
            _selector: &CredentialSelector,
            _replacement: CredentialReplacement,
        ) -> Result<CredentialCommit, CredentialPersistenceError> {
            Err(CredentialPersistenceError::Unavailable)
        }

        async fn tombstone(
            &self,
            _selector: &CredentialSelector,
            _tombstone: CredentialTombstone,
        ) -> Result<CredentialCommit, CredentialPersistenceError> {
            Err(CredentialPersistenceError::Unavailable)
        }

        async fn list(
            &self,
            _owner: &CredentialOwner,
            _state_kind: Option<&str>,
        ) -> Result<Vec<CredentialId>, CredentialPersistenceError> {
            Err(CredentialPersistenceError::Unavailable)
        }

        async fn list_heads(
            &self,
            _owner: &CredentialOwner,
            _state_kind: Option<&str>,
        ) -> Result<Vec<StoredCredentialHead>, CredentialPersistenceError> {
            Err(CredentialPersistenceError::Unavailable)
        }

        async fn exists(
            &self,
            _selector: &CredentialSelector,
        ) -> Result<bool, CredentialPersistenceError> {
            Err(CredentialPersistenceError::Unavailable)
        }
    }

    fn fixture() -> (
        SlotStore,
        crate::CredentialRegistry,
        super::super::DispatchOps<crate::ErasedPendingStore>,
        TenantScope,
        CredentialId,
        CredentialKey,
    ) {
        // Plain (pre-envelope) legacy JSON: the legacy-decode path keeps this
        // exact fixture as its slot-surface coverage.
        let token = SecretToken::new(SecretString::new(SECRET_CANARY));
        let data = crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&token))
            .expect("test token serializes");
        fixture_with_payload(
            SecretBytes::new(data),
            <SecretToken as CredentialState>::KIND,
            <SecretToken as CredentialState>::VERSION,
        )
    }

    fn fixture_with_payload(
        data: SecretBytes,
        kind: &str,
        version: u32,
    ) -> (
        SlotStore,
        crate::CredentialRegistry,
        super::super::DispatchOps<crate::ErasedPendingStore>,
        TenantScope,
        CredentialId,
        CredentialKey,
    ) {
        let scope = TenantScope::new("org-slot", "workspace-slot");
        let id = CredentialId::new();
        let now = Utc::now();
        let row = StoredLiveCredential::new(
            id,
            Some("default".to_owned()),
            BearerTokenCredential::KEY.to_owned(),
            data,
            kind.to_owned(),
            version,
            CredentialVersion::MIN,
            CredentialMaterialEpoch::MIN,
            now,
            now,
            None,
            false,
            serde_json::Map::new(),
            None,
        )
        .expect("fixture row is valid");
        let store = SlotStore {
            owner: scope.owner().clone(),
            row: row.into(),
            material_loads: AtomicUsize::new(0),
        };
        let mut registry = crate::CredentialRegistry::new();
        registry
            .register(BearerTokenCredential, "slot-test")
            .expect("fixture registration is unique");
        let mut ops = super::super::DispatchOps::new();
        crate::register_runtime_ops::<BearerTokenCredential, crate::ErasedPendingStore>(&mut ops)
            .expect("fixture ops registration is unique");
        let key = CredentialKey::new(BearerTokenCredential::KEY).expect("fixture key is valid");
        (store, registry, ops, scope, id, key)
    }

    /// Envelope bytes for the slot surface with caller-supplied overrides for
    /// the three checked facets (`None` = the honest value).
    fn envelope_payload(
        fingerprint: Option<u64>,
        kind_tag: Option<&str>,
        interface_version: Option<u32>,
    ) -> Vec<u8> {
        let token = SecretToken::new(SecretString::new(SECRET_CANARY));
        crate::serde_secret::expose_for_serialization(|| {
            let body = serde_json::to_value(&token).expect("fixture state encodes");
            let envelope = serde_json::json!({
                "interface_version": interface_version
                    .unwrap_or(<SecretToken as CredentialState>::VERSION),
                "schema_fingerprint": fingerprint
                    .unwrap_or(<SecretToken as StateWireFingerprint>::SCHEMA_FINGERPRINT)
                    .to_le_bytes(),
                "kind_tag": kind_tag.unwrap_or(<SecretToken as CredentialState>::KIND),
                "body": body,
            });
            serde_json::to_vec(&envelope).expect("fixture envelope encodes")
        })
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the test helper keeps each security-relevant resolver input explicit"
    )]
    async fn resolve_fixture(
        store: &SlotStore,
        registry: &crate::CredentialRegistry,
        ops: &super::super::DispatchOps<crate::ErasedPendingStore>,
        scope: &TenantScope,
        id: CredentialId,
        key: CredentialKey,
        required: Capabilities,
        cancel: CancellationToken,
    ) -> Result<ErasedCredentialGuard, CredentialSlotResolveError> {
        resolve_slot_with(
            store,
            registry,
            ops,
            &StateSource::LocalEncrypted,
            SlotResolutionRequest {
                scope,
                credential_id: id,
                expected_key: key,
                required_capabilities: required,
                cancel,
            },
        )
        .await
    }

    #[tokio::test]
    async fn resolves_opaque_guard_with_authoritative_ordering_metadata() {
        let (store, registry, ops, scope, id, key) = fixture();
        let erased = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            id,
            key.clone(),
            Capabilities::empty(),
            CancellationToken::new(),
        )
        .await
        .expect("matching owner and contract resolve");

        assert_eq!(erased.metadata().credential_id(), id);
        assert_eq!(erased.metadata().credential_key(), &key);
        assert_eq!(erased.metadata().material_epoch(), 1);
        assert_eq!(erased.metadata().revision(), 1);
        let typed = erased
            .into_typed::<SecretToken>()
            .expect("registered scheme type extracts");
        assert_eq!(typed.token().expose_secret(), SECRET_CANARY);
    }

    #[tokio::test]
    async fn rejects_wrong_key_before_projection() {
        let (store, registry, ops, scope, id, _) = fixture();
        let wrong = CredentialKey::new("shared_key").expect("test key is valid");
        let error = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            id,
            wrong,
            Capabilities::empty(),
            CancellationToken::new(),
        )
        .await
        .expect_err("wrong contract must fail");
        assert_eq!(error, CredentialSlotResolveError::WrongCredentialKey);
        assert_eq!(store.material_loads.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn rejects_missing_capability_before_projection() {
        let (store, registry, ops, scope, id, key) = fixture();
        let error = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            id,
            key,
            Capabilities::REFRESHABLE,
            CancellationToken::new(),
        )
        .await
        .expect_err("unsupported capability must fail");
        assert_eq!(error, CredentialSlotResolveError::MissingCapabilities);
        assert_eq!(store.material_loads.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn missing_and_cross_tenant_are_indistinguishable() {
        let (store, registry, ops, scope, id, key) = fixture();
        let other_scope = TenantScope::new("other-org", "other-workspace");
        let cross_tenant = resolve_fixture(
            &store,
            &registry,
            &ops,
            &other_scope,
            id,
            key.clone(),
            Capabilities::empty(),
            CancellationToken::new(),
        )
        .await
        .expect_err("cross-tenant lookup must be hidden");
        let missing = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            CredentialId::new(),
            key,
            Capabilities::empty(),
            CancellationToken::new(),
        )
        .await
        .expect_err("missing lookup must fail");

        assert_eq!(cross_tenant, CredentialSlotResolveError::NotFound);
        assert_eq!(missing, cross_tenant);
        assert_eq!(cross_tenant.to_string(), missing.to_string());
    }

    #[tokio::test]
    async fn cancellation_prevents_guard_publication() {
        let (store, registry, ops, scope, id, key) = fixture();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let error = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            id,
            key,
            Capabilities::empty(),
            cancel,
        )
        .await
        .expect_err("pre-cancelled resolution must fail");
        assert_eq!(error, CredentialSlotResolveError::Cancelled);
    }

    #[tokio::test]
    async fn typed_extraction_rejects_a_different_scheme() {
        let (store, registry, ops, scope, id, key) = fixture();
        let erased = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            id,
            key,
            Capabilities::empty(),
            CancellationToken::new(),
        )
        .await
        .expect("fixture resolves");
        let error = erased
            .into_typed::<SharedKey>()
            .expect_err("different scheme must not downcast");
        assert_eq!(error, ErasedCredentialGuardTypeError);
    }

    #[tokio::test]
    async fn debug_and_errors_do_not_expose_projected_secret() {
        let (store, registry, ops, scope, id, key) = fixture();
        let erased = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            id,
            key,
            Capabilities::empty(),
            CancellationToken::new(),
        )
        .await
        .expect("fixture resolves");
        let debug = format!("{erased:?}");
        let extraction_error = erased
            .into_typed::<SharedKey>()
            .expect_err("different scheme must not downcast")
            .to_string();

        assert!(!debug.contains(SECRET_CANARY));
        assert!(!extraction_error.contains(SECRET_CANARY));
        assert!(debug.contains("REDACTED"));
    }

    #[test]
    fn resolver_trait_is_object_safe() {
        fn accepts_object(_: Option<&dyn CredentialSlotResolver>) {}
        fn assert_implemented<T: CredentialSlotResolver>() {}
        accepts_object(None);
        assert_implemented::<CredentialService>();
    }

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
            CredentialServiceError::RefreshPostProviderPersistence
        ));

        let retry_gate = map_slot_resolve_error(
            &source,
            "cred-test",
            ResolveError::RefreshRetryGateFinalization {
                credential_id: "cred-test".to_owned(),
            },
        );
        assert!(matches!(
            retry_gate,
            CredentialServiceError::RefreshRetryGateFinalization
        ));

        let reconciliation = map_slot_resolve_error(
            &source,
            "cred-test",
            ResolveError::RefreshReconciliationRequired {
                credential_id: "cred-test".to_owned(),
            },
        );
        assert!(matches!(
            reconciliation,
            CredentialServiceError::RefreshReconciliationRequired
        ));

        let reauth = map_slot_resolve_error(
            &source,
            "cred-test",
            ResolveError::ReauthDecisionFinalization {
                credential_id: "cred-test".to_owned(),
            },
        );
        assert!(matches!(
            reauth,
            CredentialServiceError::ReauthDecisionFinalization
        ));
    }

    #[test]
    fn slot_mapping_preserves_proof_bearing_refresh_failure() {
        let source = StateSource::LocalEncrypted;
        let context = RefreshNotAppliedContext::from_spec(
            RefreshNotAppliedPhase::ProviderConfirmedNotApplied,
            RefreshFailureSpec::new(RefreshErrorKind::ProtocolError, RetryAdvice::Never),
        );

        let mapped = map_slot_resolve_error(
            &source,
            "cred-test",
            ResolveError::RefreshNotApplied {
                credential_id: "cred-test".to_owned(),
                context: Box::new(context),
            },
        );

        let CredentialServiceError::RefreshNotApplied(context) = mapped else {
            panic!("slot resolution must preserve the typed exact failure");
        };
        assert_eq!(
            context.phase(),
            RefreshNotAppliedPhase::ProviderConfirmedNotApplied
        );
        assert_eq!(context.retry(), RetryAdvice::Never);
    }

    // ── Envelope fail-closed decode (ADR-0107 Seam 2) ─────────────────

    #[tokio::test]
    async fn envelope_wrapped_row_resolves_on_the_slot_surface() {
        let data = crate::serde_secret::expose_for_serialization(|| {
            let token = SecretToken::new(SecretString::new(SECRET_CANARY));
            crate::state_envelope::encode_state_payload(&token)
        })
        .expect("fixture envelope encodes");
        let (store, registry, ops, scope, id, key) = fixture_with_payload(
            SecretBytes::from(data),
            <SecretToken as CredentialState>::KIND,
            <SecretToken as CredentialState>::VERSION,
        );

        let erased = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            id,
            key.clone(),
            Capabilities::empty(),
            CancellationToken::new(),
        )
        .await
        .expect("an envelope-wrapped row resolves exactly like a legacy row");
        let typed = erased
            .into_typed::<SecretToken>()
            .expect("registered scheme type extracts");
        assert_eq!(typed.token().expose_secret(), SECRET_CANARY);
    }

    #[tokio::test]
    async fn envelope_schema_fingerprint_mismatch_refuses_as_stored_state_refused() {
        let honest = <SecretToken as StateWireFingerprint>::SCHEMA_FINGERPRINT;
        let (store, registry, ops, scope, id, key) = fixture_with_payload(
            SecretBytes::new(envelope_payload(
                Some(honest ^ 0x0000_0000_0000_0001),
                None,
                None,
            )),
            <SecretToken as CredentialState>::KIND,
            <SecretToken as CredentialState>::VERSION,
        );

        let error = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            id,
            key,
            Capabilities::empty(),
            CancellationToken::new(),
        )
        .await
        .expect_err("a stored fingerprint mismatch must refuse slot resolution");
        assert_eq!(
            error,
            CredentialSlotResolveError::StoredStateRefused(
                crate::StateEnvelopeError::SchemaFingerprintMismatch {
                    expected: honest,
                    stored: honest ^ 0x0000_0000_0000_0001,
                },
            ),
            "the slot surface must keep the refusal distinct from InvalidState"
        );
    }

    #[tokio::test]
    async fn envelope_kind_tag_mismatch_refuses_as_stored_state_refused() {
        let (store, registry, ops, scope, id, key) = fixture_with_payload(
            SecretBytes::new(envelope_payload(None, Some("intruder_kind"), None)),
            <SecretToken as CredentialState>::KIND,
            <SecretToken as CredentialState>::VERSION,
        );

        let error = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            id,
            key,
            Capabilities::empty(),
            CancellationToken::new(),
        )
        .await
        .expect_err("a stored kind-tag mismatch must refuse slot resolution");
        assert_eq!(
            error,
            CredentialSlotResolveError::StoredStateRefused(
                crate::StateEnvelopeError::KindMismatch {
                    expected: <SecretToken as CredentialState>::KIND,
                },
            ),
            "the slot surface must keep the refusal distinct from InvalidState"
        );
    }

    #[tokio::test]
    async fn envelope_unsupported_version_refuses_as_stored_state_refused() {
        let future = <SecretToken as CredentialState>::VERSION + 1;
        let (store, registry, ops, scope, id, key) = fixture_with_payload(
            SecretBytes::new(envelope_payload(None, None, Some(future))),
            <SecretToken as CredentialState>::KIND,
            future,
        );

        let error = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            id,
            key,
            Capabilities::empty(),
            CancellationToken::new(),
        )
        .await
        .expect_err("a stored shape newer than this build must refuse slot resolution");
        assert_eq!(
            error,
            CredentialSlotResolveError::StoredStateRefused(
                crate::StateEnvelopeError::UnknownSchemaVersion {
                    stored_version: future,
                    supported_version: <SecretToken as CredentialState>::VERSION,
                },
            ),
            "the slot surface must keep the refusal distinct from InvalidState"
        );
    }

    #[tokio::test]
    async fn legacy_payload_with_future_row_version_refuses_as_stored_state_refused() {
        // A NON-envelope payload (plain legacy JSON) whose row axis exceeds
        // this build's VERSION must refuse on the legacy path's own version
        // check. The envelope-unsupported-version test above feeds an envelope
        // payload and hits the envelope's `interface_version` check; this pins
        // the sibling branch (state_envelope.rs legacy fallback) on the slot
        // surface.
        let future = <SecretToken as CredentialState>::VERSION + 1;
        let token = SecretToken::new(SecretString::new(SECRET_CANARY));
        let data = crate::serde_secret::expose_for_serialization(|| serde_json::to_vec(&token))
            .expect("test token serializes");
        let (store, registry, ops, scope, id, key) = fixture_with_payload(
            SecretBytes::new(data),
            <SecretToken as CredentialState>::KIND,
            future,
        );

        let error = resolve_fixture(
            &store,
            &registry,
            &ops,
            &scope,
            id,
            key,
            Capabilities::empty(),
            CancellationToken::new(),
        )
        .await
        .expect_err("a legacy row claiming a future version must refuse slot resolution");
        assert_eq!(
            error,
            CredentialSlotResolveError::StoredStateRefused(
                crate::StateEnvelopeError::UnknownSchemaVersion {
                    stored_version: future,
                    supported_version: <SecretToken as CredentialState>::VERSION,
                },
            ),
            "the slot surface must keep the refusal distinct from InvalidState"
        );
    }
}
