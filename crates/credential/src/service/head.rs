//! Secret-free credential row view returned by the facade's CRUD surface.
//!
//! [`CredentialHead`] is the management-plane projection of a stored
//! credential: identity, type key, store version (the CAS token), the
//! lifecycle timestamps, and the per-instance display metadata. It carries
//! **no** state bytes and no projected scheme, so reading it never
//! deserializes or decrypts credential material — a row that is not yet
//! resolvable (e.g. an OAuth2 placeholder awaiting authorization, flagged
//! `reauth_required`) still projects a valid head.
//!
//! The scheme-bearing view stays on the execution plane:
//! [`CredentialService::resolve_for_slot`](crate::CredentialService::resolve_for_slot)
//! produces a typed guard, and the engine resolver owns snapshot projection.

use chrono::{DateTime, Utc};
use nebula_credential::{CredentialDisplay, LAST_VALIDATED_AT_METADATA_KEY, StoredCredentialHead};
use serde::Serialize;
use serde_json::Value;

/// Secret-free management view of one stored credential row.
///
/// Returned by the facade CRUD operations (`create` / `get` / `list` /
/// `update` / `refresh`). All fields are non-secret by construction:
/// The persistence projection has no state-byte field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct CredentialHead {
    /// Credential id (`cred_<ULID>` wire form).
    pub id: String,
    /// `Credential::KEY` of the stored type (e.g. `"api_key"`, `"oauth2"`).
    pub credential_key: String,
    /// Store version — the optimistic-concurrency token for
    /// [`update`](crate::CredentialService::update) compare-and-swap.
    pub version: u64,
    /// When the row was created.
    pub created_at: DateTime<Utc>,
    /// When the row was last written.
    pub updated_at: DateTime<Utc>,
    /// When the credential material expires, if it does.
    pub expires_at: Option<DateTime<Utc>>,
    /// When the credential material was last validated or refreshed, if the
    /// runtime has established that anchor.
    pub last_validated_at: Option<DateTime<Utc>>,
    /// True when the credential cannot be used until re-authorized (e.g.
    /// an interactive flow was started but not completed, or a refresh
    /// failed terminally).
    pub reauth_required: bool,
    /// Per-instance display metadata (name / description / tags). Empty
    /// for system-acquired credentials that were never named.
    pub display: CredentialDisplay,
}

impl CredentialHead {
    /// Project a stored row into its secret-free head. `display` is passed
    /// separately because the `metadata["display"]` persistence convention
    /// is owned by the facade, not the row type.
    #[must_use]
    pub(crate) fn from_stored(stored: &StoredCredentialHead, display: CredentialDisplay) -> Self {
        Self {
            id: stored.credential_id().to_string(),
            credential_key: stored.credential_key().to_owned(),
            version: stored.version().get() as u64,
            created_at: stored.created_at(),
            updated_at: stored.updated_at(),
            expires_at: stored.expires_at(),
            last_validated_at: stored
                .metadata()
                .get(LAST_VALIDATED_AT_METADATA_KEY)
                .and_then(Value::as_str)
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|instant| instant.with_timezone(&Utc)),
            reauth_required: stored.reauth_required(),
            display,
        }
    }

    /// True iff this head's `expires_at` is in the past. A credential
    /// without an explicit expiry is treated as non-expiring.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|at| at <= Utc::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_storage_port::{CredentialVersion, StoredCredentialHead};

    fn stored(expires_at: Option<DateTime<Utc>>) -> StoredCredentialHead {
        let now = Utc::now();
        StoredCredentialHead::new(
            crate::CredentialId::new(),
            None,
            "api_key".to_owned(),
            "api_key_state".to_owned(),
            1,
            CredentialVersion::try_from(4_i64).expect("fixture version is valid"),
            now,
            now,
            expires_at,
            false,
            serde_json::Map::new(),
        )
        .expect("fixture is a live head")
    }

    #[test]
    fn from_stored_head_copies_projection_fields() {
        let row = stored(None);
        let head = CredentialHead::from_stored(&row, CredentialDisplay::default());
        assert_eq!(head.id, row.credential_id().to_string());
        assert_eq!(head.credential_key, "api_key");
        assert_eq!(head.version, 4);
        assert_eq!(head.last_validated_at, None);
        assert!(!head.reauth_required);
        assert!(head.display.is_empty());
        // No `data` field exists on either persistence or service projection.
        let json = serde_json::to_value(&head).expect("serialize head");
        assert!(json.get("data").is_none());
    }

    #[test]
    fn is_expired_respects_expiry() {
        let past = Utc::now() - chrono::Duration::seconds(10);
        let future = Utc::now() + chrono::Duration::seconds(600);
        assert!(
            CredentialHead::from_stored(&stored(Some(past)), CredentialDisplay::default())
                .is_expired()
        );
        assert!(
            !CredentialHead::from_stored(&stored(Some(future)), CredentialDisplay::default())
                .is_expired()
        );
        assert!(
            !CredentialHead::from_stored(&stored(None), CredentialDisplay::default()).is_expired()
        );
    }
}
