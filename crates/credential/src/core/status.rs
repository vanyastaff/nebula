//! Credential status for metadata-based status checks.
//!
//! Used by `get_metadata()` and `list_with_metadata()` to indicate credential
//! readiness without exposing the secret value.

use crate::core::CredentialMetadata;

/// Status of a credential derived from metadata.
///
/// Used when listing or checking credential state without fetching the secret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialStatus {
    /// Credential is ready for use.
    Active,

    /// Flow in progress; user must complete interaction (OAuth2 callback, etc.).
    PendingInteraction,

    /// Credential is invalid (expired, validation failed, etc.).
    Error { reason: String },
}

/// Derive status from metadata.
///
/// - `Error` if expired
/// - `PendingInteraction` if metadata indicates flow in progress (tag `credential_status=pending`)
/// - `Active` otherwise
#[must_use]
pub fn status_from_metadata(metadata: &CredentialMetadata) -> CredentialStatus {
    if metadata.is_expired() {
        return CredentialStatus::Error {
            reason: "credential expired".to_string(),
        };
    }
    if metadata.tags.get("credential_status").map(|s| s.as_str()) == Some("pending") {
        return CredentialStatus::PendingInteraction;
    }
    CredentialStatus::Active
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::CredentialMetadata;

    #[test]
    fn status_active_when_valid() {
        let meta = CredentialMetadata::new();
        assert!(matches!(
            status_from_metadata(&meta),
            CredentialStatus::Active
        ));
    }

    #[test]
    fn status_error_when_expired() {
        use chrono::Utc;
        use std::time::Duration;

        let mut meta = CredentialMetadata::new();
        meta.set_expiration(Duration::from_secs(1));
        meta.expires_at = Some(Utc::now() - chrono::Duration::seconds(10));
        assert!(matches!(
            status_from_metadata(&meta),
            CredentialStatus::Error { .. }
        ));
    }

    #[test]
    fn status_pending_when_tag_set() {
        let mut meta = CredentialMetadata::new();
        meta.tags
            .insert("credential_status".to_string(), "pending".to_string());
        assert!(matches!(
            status_from_metadata(&meta),
            CredentialStatus::PendingInteraction
        ));
    }
}
