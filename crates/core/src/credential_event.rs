//! Credential lifecycle events for cross-crate signaling.
//!
//! Emitted via [`EventBus<CredentialEvent>`](nebula_eventbus::EventBus) by the
//! credential resolver. Consumed by `nebula-resource` for pool invalidation
//! and by monitoring tools.
//!
//! Events carry credential ID only — **never credential data or secrets**.

use std::fmt;

/// Cross-crate credential lifecycle event.
///
/// Emitted after credential state changes. All variants carry only
/// identifiers, never secret material.
///
/// # Usage
///
/// ```
/// use nebula_core::CredentialEvent;
///
/// let event = CredentialEvent::Refreshed {
///     credential_id: "oauth2-github-42".to_string(),
/// };
/// assert_eq!(event.credential_id(), "oauth2-github-42");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialEvent {
    /// Auth material was refreshed (e.g., OAuth2 token refresh).
    ///
    /// Existing connections may still work. Pools should re-auth on next
    /// checkout.
    Refreshed {
        /// The credential instance ID.
        credential_id: String,
    },

    /// Credential was explicitly revoked.
    ///
    /// All connections using this credential **must** be terminated
    /// immediately.
    Revoked {
        /// The credential instance ID.
        credential_id: String,
    },
}

impl CredentialEvent {
    /// Returns the credential ID for all variants.
    #[must_use]
    pub fn credential_id(&self) -> &str {
        match self {
            Self::Refreshed { credential_id } | Self::Revoked { credential_id } => credential_id,
        }
    }
}

impl fmt::Display for CredentialEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refreshed { credential_id } => {
                write!(f, "credential refreshed: {credential_id}")
            }
            Self::Revoked { credential_id } => {
                write!(f, "credential revoked: {credential_id}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_id_returns_id_for_all_variants() {
        let refreshed = CredentialEvent::Refreshed {
            credential_id: "cred-1".to_string(),
        };
        assert_eq!(refreshed.credential_id(), "cred-1");

        let revoked = CredentialEvent::Revoked {
            credential_id: "cred-2".to_string(),
        };
        assert_eq!(revoked.credential_id(), "cred-2");
    }

    #[test]
    fn display_formats_correctly() {
        let refreshed = CredentialEvent::Refreshed {
            credential_id: "abc".to_string(),
        };
        assert_eq!(refreshed.to_string(), "credential refreshed: abc");

        let revoked = CredentialEvent::Revoked {
            credential_id: "xyz".to_string(),
        };
        assert_eq!(revoked.to_string(), "credential revoked: xyz");
    }

    #[test]
    fn clone_and_eq_work() {
        let event = CredentialEvent::Refreshed {
            credential_id: "test".to_string(),
        };
        let cloned = event.clone();
        assert_eq!(event, cloned);
    }
}
