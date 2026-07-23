//! Validated credential binding — a typed handle proving that a
//! workflow `slot_bindings` entry has been scope-checked against the
//! caller's [`TenantScope`].
//!
//! Constructors are crate-private; engine execution consumes only
//! validated handles, closing the confused-deputy non-goal left open
//! by the ADR-0052 cascade.

use std::fmt;

use nebula_core::CredentialId;
use nebula_storage_port::{CredentialOwner, CredentialSelector};

use super::scope::TenantScope;

/// Tenant-scope-checked credential binding.
///
/// The only constructor is
/// [`CredentialService::validate_credential_binding`](crate::CredentialService::validate_credential_binding);
/// engine execution consumes this handle directly.
///
/// Fields are private and the constructor is `pub(crate)`, so downstream
/// code outside `nebula-credential` cannot forge a
/// `ValidatedCredentialBinding`.
#[derive(Debug, Clone)]
pub struct ValidatedCredentialBinding {
    credential_id: CredentialId,
    tenant_fingerprint: TenantFingerprint,
}

/// Opaque proof of which tenant validated this binding.
///
/// Constructed only from a [`TenantScope`] inside this crate. Equality
/// is intentionally crate-private so downstream consumers cannot forge
/// a fingerprint value.
#[derive(Clone, PartialEq, Eq)]
pub struct TenantFingerprint(pub(crate) CredentialOwner);

impl fmt::Debug for TenantFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TenantFingerprint([redacted])")
    }
}

impl ValidatedCredentialBinding {
    /// Crate-private constructor — the only call site is
    /// [`CredentialService::validate_credential_binding`].
    ///
    /// [`CredentialService::validate_credential_binding`]: crate::CredentialService::validate_credential_binding
    pub(crate) fn new(credential_id: CredentialId, tenant_fingerprint: TenantFingerprint) -> Self {
        Self {
            credential_id,
            tenant_fingerprint,
        }
    }

    /// The validated credential's typed identifier.
    #[must_use]
    pub fn credential_id(&self) -> CredentialId {
        self.credential_id
    }

    /// The owner-scoped lookup key for this binding — the credential id paired
    /// with the `owner_id` the scope check proved owns it (the fingerprint is
    /// the `owner_id`).
    ///
    /// The runtime resolver consumes this to re-verify the stored row's owner
    /// at load, so a validated binding is backed by a load-time owner check
    /// rather than authorizing an unscoped load on its provenance alone.
    pub(crate) fn selector(&self) -> CredentialSelector {
        CredentialSelector::new(self.tenant_fingerprint.0.clone(), self.credential_id)
    }

    /// Crate-private access to the scope fingerprint. Consumed by the
    /// engine execution path that re-validates the binding before
    /// dispatching secrets (`resolve_for_slot`).
    #[must_use]
    pub(crate) fn fingerprint(&self) -> &TenantFingerprint {
        &self.tenant_fingerprint
    }
}

impl TenantFingerprint {
    /// Derive a fingerprint from a [`TenantScope`]. The fingerprint is
    /// the `owner_id` string — sufficient to detect cross-tenant misuse
    /// without embedding any secret material.
    pub(crate) fn from_scope(scope: &TenantScope) -> Self {
        Self(scope.owner().clone())
    }
}

/// Reason a `CredentialService::validate_credential_binding` call failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ValidatedCredentialBindingError {
    /// The credential id does not exist in any tenant visible to the caller.
    ///
    /// Emitted for both a genuinely absent id AND for an id that exists but
    /// belongs to a different tenant (existence-hiding). The two cases are
    /// deliberately indistinguishable to callers to prevent cross-tenant
    /// enumeration oracles. Internal logs record which case occurred.
    #[error("credential `{id}` not found")]
    NotFound {
        /// The credential id that was not found.
        id: String,
    },

    /// Reserved for future or administrative use.
    ///
    /// `validate_credential_binding` no longer emits this variant — cross-tenant
    /// ids are mapped to [`NotFound`](Self::NotFound) (existence-hiding). This
    /// variant is kept for `#[non_exhaustive]` forward-compatibility and any
    /// future admin/audit surface that is allowed to name the owning tenant.
    #[doc(hidden)]
    #[error(
        "credential `{id}` belongs to tenant `{actual}`; caller requested tenant `{requested}`"
    )]
    ScopeMismatch {
        /// Credential id under dispute.
        id: String,
        /// Tenant the caller claimed (`scope.owner_id()`).
        requested: String,
        /// Tenant actually stored in the credential row.
        actual: String,
    },

    /// The credential exists and is owned by the caller but is in the
    /// structural terminal tombstone state.
    ///
    /// Distinct from [`NotFound`] on purpose: a binding pointing at a revoked
    /// credential is a *clear* error ("this credential was revoked"), not a
    /// generic miss — so a workflow author sees why the slot stopped resolving
    /// rather than chasing a phantom "not found". The check happens here at
    /// bind-validation time, with no reverse `references()` index in the
    /// credential crate (that would invert the service→runtime→contract layering).
    ///
    /// [`NotFound`]: Self::NotFound
    #[error("credential `{id}` was revoked")]
    CredentialTombstoned {
        /// The revoked credential id.
        id: String,
        /// Physical persistence timestamp of the terminal transition.
        revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    },

    /// An underlying store or service error occurred during validation.
    #[error("credential binding validator i/o: {0}")]
    Io(#[from] super::error::CredentialServiceError),
}
