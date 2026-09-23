//! Tenant scoping for credential operations.
//!
//! `TenantScope` carries one mandatory, non-optional owner partition. There is
//! no resolver and no `None == admin` convention: every persistence call is
//! bound to an explicit selector derived inside the credential subsystem.

use std::fmt;

use nebula_core::CredentialId;
use nebula_storage_port::{CredentialOwner, CredentialSelector, Scope};
use thiserror::Error;

const SHA256_BASE64URL_LENGTH: usize = 43;

/// Opaque, domain-separated binding of one exact Plane-A authentication
/// credential to an interactive credential flow.
///
/// First-party authentication produces an unpadded base64url SHA-256 digest.
/// Accepting the exact encoded shape here prevents empty, raw, or ambiguous
/// session identifiers from becoming pending-state partitions.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialAuthenticationBinding(String);

impl CredentialAuthenticationBinding {
    /// Validate an unpadded base64url SHA-256 authentication binding.
    pub fn parse(value: impl Into<String>) -> Result<Self, CredentialAuthenticationBindingError> {
        let value = value.into();
        let valid_alphabet = value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
        if value.len() != SHA256_BASE64URL_LENGTH || !valid_alphabet {
            return Err(CredentialAuthenticationBindingError::InvalidEncoding);
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CredentialAuthenticationBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CredentialAuthenticationBinding([REDACTED])")
    }
}

/// Invalid interactive authentication binding.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialAuthenticationBindingError {
    /// The value is not an unpadded base64url SHA-256 digest.
    #[error("credential authentication binding has invalid encoding")]
    InvalidEncoding,
}

/// Tenant identity for a credential operation. `owner_id` is the canonical
/// [`Scope::credential_owner_id`] key used to build every mandatory
/// [`CredentialSelector`]. It is derived through the **single** shared
/// derivation so the runtime plane and API trust bridge key the same tenant
/// identically.
///
/// An optional `authentication_binding` carries a digest of the exact Plane-A
/// credential used for an interactive flow. The legacy runtime
/// `PendingStateStore` calls this dimension `session_id`, but callers must pass
/// neither a principal id nor raw bearer material. The
/// `PendingStateStore` binds pending acquisitions on
/// `(kind, owner, session, token)`, so an authentication binding is
/// **required** for the
/// interactive paths (`resolve`/`acquire` returning `Pending`,
/// `continue_resolve`). CRUD and the non-interactive capability ops do
/// not consult it; `new` leaves it `None`.
#[derive(Clone, PartialEq, Eq)]
pub struct TenantScope {
    owner: CredentialOwner,
    authentication_binding: Option<CredentialAuthenticationBinding>,
}

impl TenantScope {
    /// Construct from organization + workspace identifiers. The authentication
    /// binding is `None`; attach one with
    /// [`with_authentication_binding`](Self::with_authentication_binding)
    /// before driving an interactive acquisition.
    #[must_use]
    pub fn new(org: impl AsRef<str>, workspace: impl AsRef<str>) -> Self {
        // Route through the one canonical derivation (note `Scope::new` takes
        // workspace first, then org).
        Self {
            owner: CredentialOwner::from_scope(&Scope::new(workspace.as_ref(), org.as_ref())),
            authentication_binding: None,
        }
    }

    /// Construct from an already-resolved storage [`Scope`]. The owner key
    /// is the same canonical [`Scope::credential_owner_id`] derivation as
    /// [`new`](Self::new) — this constructor exists so the API edge, which
    /// holds a resolved `Scope`, cannot drift by re-deriving from raw
    /// org/workspace strings. The authentication binding is `None`; attach one with
    /// [`with_authentication_binding`](Self::with_authentication_binding).
    #[must_use]
    pub fn from_scope(scope: &Scope) -> Self {
        Self {
            owner: CredentialOwner::from_scope(scope),
            authentication_binding: None,
        }
    }

    /// Attach the interactive-flow Plane-A authentication binding. Required
    /// for the pending-store `(kind, owner, session, token)` binding that the
    /// interactive `resolve`/`continue_resolve` paths depend on; CRUD
    /// and the non-interactive ops ignore it.
    #[must_use]
    pub fn with_authentication_binding(mut self, binding: CredentialAuthenticationBinding) -> Self {
        self.authentication_binding = Some(binding);
        self
    }

    /// The scope key carried by every persistence selector.
    /// Unaffected by
    /// [`with_authentication_binding`](Self::with_authentication_binding) — owner
    /// derivation is org/workspace only.
    #[must_use]
    pub fn owner_id(&self) -> &str {
        self.owner.as_str()
    }

    /// The interactive-flow authentication binding, if one was attached via
    /// [`with_authentication_binding`](Self::with_authentication_binding).
    #[must_use]
    pub fn authentication_binding(&self) -> Option<&str> {
        self.authentication_binding
            .as_ref()
            .map(CredentialAuthenticationBinding::as_str)
    }

    /// Derive the owner-bound persistence selector for one credential id.
    pub(crate) fn selector(&self, credential_id: CredentialId) -> CredentialSelector {
        CredentialSelector::new(self.owner.clone(), credential_id)
    }

    /// Borrow the mandatory owner partition for list operations.
    pub(crate) fn owner(&self) -> &CredentialOwner {
        &self.owner
    }
}

impl fmt::Debug for TenantScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TenantScope")
            .field("owner", &self.owner)
            .field(
                "authentication_binding_present",
                &self.authentication_binding.is_some(),
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::CredentialId;
    use nebula_storage_port::Scope;

    use super::{CredentialAuthenticationBinding, TenantScope};

    fn authentication_binding() -> CredentialAuthenticationBinding {
        CredentialAuthenticationBinding::parse("A".repeat(43))
            .expect("test binding has the required digest shape")
    }

    #[test]
    fn owner_id_matches_canonical_derivation() {
        let scope = TenantScope::new("org-1", "ws-2");
        // The owner key is the canonical `Scope::credential_owner_id`, derived
        // identically by the API edge — not a runtime-local `{org}/{ws}` form.
        assert_eq!(
            scope.owner_id(),
            Scope::new("ws-2", "org-1").credential_owner_id()
        );
    }

    #[test]
    fn new_scope_has_no_session() {
        let scope = TenantScope::new("org-1", "ws-2");
        assert_eq!(scope.authentication_binding(), None);
    }

    #[test]
    fn authentication_binding_does_not_change_owner() {
        let scope =
            TenantScope::new("org-1", "ws-2").with_authentication_binding(authentication_binding());
        assert_eq!(
            scope.authentication_binding(),
            Some("A".repeat(43).as_str())
        );
        // Owner derivation is unchanged by the session.
        assert_eq!(
            scope.owner_id(),
            Scope::new("ws-2", "org-1").credential_owner_id()
        );
    }

    #[test]
    fn authentication_binding_rejects_empty_raw_and_padded_values() {
        for invalid in [
            String::new(),
            "raw-bearer".to_owned(),
            format!("{}=", "A".repeat(42)),
            "A".repeat(42),
            "A".repeat(44),
        ] {
            assert!(CredentialAuthenticationBinding::parse(invalid).is_err());
        }
    }

    #[test]
    fn debug_redacts_authentication_binding() {
        const CANARY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let binding = CredentialAuthenticationBinding::parse(CANARY)
            .expect("canary has the required digest shape");
        let scope = TenantScope::new("org-1", "ws-2").with_authentication_binding(binding);
        let rendered = format!("{scope:?}");
        assert!(!rendered.contains(CANARY));
        assert!(rendered.contains("authentication_binding_present"));
    }

    #[test]
    fn selector_is_always_owner_bound() {
        let scope = TenantScope::new("o", "w");
        let credential_id = CredentialId::new();
        let selector = scope.selector(credential_id);
        assert_eq!(selector.credential_id(), credential_id);
        assert_eq!(selector.owner().as_str(), scope.owner_id());
    }
}
