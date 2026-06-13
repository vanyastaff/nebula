//! Validated credential binding — a typed handle proving that a
//! workflow `slot_bindings` entry has been scope-checked against the
//! caller's [`TenantScope`].
//!
//! Constructors are crate-private; engine execution consumes only
//! validated handles, closing the confused-deputy non-goal left open
//! by the ADR-0052 cascade.

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
    credential_id: String,
    tenant_fingerprint: TenantFingerprint,
}

/// Opaque proof of which tenant validated this binding.
///
/// Constructed only from a [`TenantScope`] inside this crate. Equality
/// is intentionally crate-private so downstream consumers cannot forge
/// a fingerprint value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantFingerprint(pub(crate) String);

impl ValidatedCredentialBinding {
    /// Crate-private constructor — the only call site is
    /// [`CredentialService::validate_credential_binding`].
    ///
    /// [`CredentialService::validate_credential_binding`]: crate::CredentialService::validate_credential_binding
    pub(crate) fn new(credential_id: String, tenant_fingerprint: TenantFingerprint) -> Self {
        Self {
            credential_id,
            tenant_fingerprint,
        }
    }

    /// The validated credential's string identifier.
    #[must_use]
    pub fn credential_id(&self) -> &str {
        &self.credential_id
    }

    /// The owner-scoped lookup key for this binding — the credential id paired
    /// with the `owner_id` the scope check proved owns it (the fingerprint is
    /// the `owner_id`).
    ///
    /// The runtime resolver consumes this to re-verify the stored row's owner
    /// at load, so a validated binding is backed by a load-time owner check
    /// rather than authorizing an unscoped load on its provenance alone.
    #[must_use]
    pub fn owner_scoped_key(&self) -> crate::store::OwnerScopedKey {
        crate::store::OwnerScopedKey::new(
            self.tenant_fingerprint.0.clone(),
            self.credential_id.clone(),
        )
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
        Self(scope.owner_id().to_owned())
    }
}

/// Reason a `CredentialService::validate_credential_binding` call failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ValidatedCredentialBindingError {
    /// The credential id does not exist in any tenant.
    ///
    /// Emitted only after the row is confirmed absent — not used to
    /// mask cross-tenant rows (that produces [`ScopeMismatch`] instead).
    ///
    /// [`ScopeMismatch`]: Self::ScopeMismatch
    #[error("credential `{id}` not found")]
    NotFound {
        /// The credential id that was not found.
        id: String,
    },

    /// The credential exists in a different tenant than the caller's.
    ///
    /// Fail-closes: the binding is rejected and the caller is told which
    /// tenant mismatch occurred (but not the credential's secret
    /// material).
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

    /// An underlying store or service error occurred during validation.
    #[error("credential binding validator i/o: {0}")]
    Io(#[from] super::error::CredentialServiceError),
}
