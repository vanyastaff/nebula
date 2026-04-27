//! Credential lifecycle events for cross-crate signaling.
//!
//! Emitted via `EventBus<CredentialEvent>` by the credential resolver.
//! Consumed by `nebula-resource` for pool invalidation and by monitoring tools.
//!
//! Events carry credential ID only — **never credential data or secrets**.

use std::fmt;

use crate::{CredentialId, resolve::ReauthReason};

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
#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// Credential needs full re-authentication. Per sub-spec §3.4
    /// ([credential-refresh-coordination]) the engine emits this when:
    ///
    /// - The IdP rejects the refresh (`ReauthReason::ProviderRejected`).
    /// - The sentinel threshold (default N=3 within 1h) is exceeded for mid-refresh crashes
    ///   (`ReauthReason::SentinelRepeated`).
    /// - A locally detected lack of refresh material — e.g. an OAuth2 state with no `refresh_token`
    ///   (`ReauthReason::MissingRefreshMaterial`); the IdP was never contacted.
    ///
    /// Consumers (UI, monitoring) surface a re-auth prompt. Pools and
    /// connections using this credential must be invalidated until the
    /// user re-authenticates.
    ///
    /// [credential-refresh-coordination]: https://github.com/nebula-engine/nebula/blob/main/docs/superpowers/specs/2026-04-24-credential-refresh-coordination.md
    ReauthRequired {
        /// The credential instance ID.
        credential_id: CredentialId,
        /// Why re-authentication is required.
        reason: ReauthReason,
    },
}

impl CredentialEvent {
    /// Returns the credential ID for all variants.
    #[must_use]
    pub fn credential_id(&self) -> CredentialId {
        match self {
            Self::Refreshed { credential_id }
            | Self::Revoked { credential_id }
            | Self::ReauthRequired { credential_id, .. } => *credential_id,
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
            Self::ReauthRequired {
                credential_id,
                reason,
            } => {
                write!(
                    f,
                    "credential reauth required: {credential_id} ({reason:?})"
                )
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
    fn clone_and_eq_work() {
        let id = CredentialId::new();
        let event = CredentialEvent::Refreshed { credential_id: id };
        let cloned = event.clone();
        assert_eq!(event, cloned);
    }

    #[test]
    fn reauth_required_carries_reason() {
        use crate::resolve::ReauthReason;

        let id = CredentialId::new();
        let event = CredentialEvent::ReauthRequired {
            credential_id: id,
            reason: ReauthReason::SentinelRepeated {
                event_count: 3,
                window_secs: 3600,
            },
        };
        assert_eq!(event.credential_id(), id);
        let display = event.to_string();
        assert!(
            display.starts_with("credential reauth required: cred_"),
            "display: {display}"
        );
    }
}
