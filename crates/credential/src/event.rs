//! Credential lifecycle events for cross-crate signaling.
//!
//! Emitted via `EventBus<CredentialEvent>` by the credential resolver.
//! Consumed by `nebula-resource` for pool invalidation and by monitoring tools.
//!
//! Events carry credential ID only — **never credential data or secrets**.

use std::fmt;

use crate::CredentialId;

/// Cross-crate credential lifecycle event.
///
/// Emitted after credential state changes. All variants carry only
/// identifiers, never secret material.
///
/// # Usage
///
/// ```
/// use nebula_credential::{CredentialEvent, CredentialId};
///
/// let id = CredentialId::new();
/// let event = CredentialEvent::Refreshed { credential_id: id };
/// assert_eq!(event.credential_id(), id);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CredentialEvent {
    /// Auth material was refreshed (e.g., OAuth2 token refresh).
    ///
    /// Existing connections may still work. Pools should re-auth on next
    /// checkout.
    Refreshed {
        /// The credential instance ID.
        credential_id: CredentialId,
    },

    /// Credential was explicitly revoked.
    ///
    /// All connections using this credential **must** be terminated
    /// immediately.
    Revoked {
        /// The credential instance ID.
        credential_id: CredentialId,
    },
}

impl CredentialEvent {
    /// Returns the credential ID for all variants.
    #[must_use]
    pub fn credential_id(&self) -> CredentialId {
        match self {
            Self::Refreshed { credential_id } | Self::Revoked { credential_id } => *credential_id,
        }
    }
}

impl fmt::Display for CredentialEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refreshed { credential_id } => {
                write!(f, "credential refreshed: {credential_id}")
            },
            Self::Revoked { credential_id } => {
                write!(f, "credential revoked: {credential_id}")
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_id_returns_typed_id_for_all_variants() {
        let id1 = CredentialId::new();
        let refreshed = CredentialEvent::Refreshed { credential_id: id1 };
        assert_eq!(refreshed.credential_id(), id1);

        let id2 = CredentialId::new();
        let revoked = CredentialEvent::Revoked { credential_id: id2 };
        assert_eq!(revoked.credential_id(), id2);
    }

    #[test]
    fn display_formats_with_prefix_and_valid_id() {
        let id = CredentialId::new();
        let refreshed = CredentialEvent::Refreshed { credential_id: id };
        let display = refreshed.to_string();
        assert!(display.starts_with("credential refreshed: cred_"));
        // Verify the suffix is the full ID string (parseable back)
        let id_suffix = display.strip_prefix("credential refreshed: ").unwrap();
        let parsed: CredentialId = id_suffix
            .parse()
            .expect("display suffix must be a valid CredentialId");
        assert_eq!(parsed, id);

        let revoked = CredentialEvent::Revoked { credential_id: id };
        let display = revoked.to_string();
        assert!(display.starts_with("credential revoked: cred_"));
        let id_suffix = display.strip_prefix("credential revoked: ").unwrap();
        let parsed: CredentialId = id_suffix
            .parse()
            .expect("display suffix must be a valid CredentialId");
        assert_eq!(parsed, id);
    }

    #[test]
    fn copy_and_eq_work() {
        let id = CredentialId::new();
        let event = CredentialEvent::Refreshed { credential_id: id };
        let copied = event;
        assert_eq!(event, copied);
    }
}
