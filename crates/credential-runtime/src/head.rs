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
use nebula_credential::{CredentialDisplay, StoredCredential};
use serde::Serialize;

/// Secret-free management view of one stored credential row.
///
/// Returned by the facade CRUD operations (`create` / `get` / `list` /
/// `update` / `refresh`). All fields are non-secret by construction:
/// `StoredCredential::data` is never read to build a head.
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
    pub(crate) fn from_stored(stored: &StoredCredential, display: CredentialDisplay) -> Self {
        Self {
            id: stored.id.clone(),
            credential_key: stored.credential_key.clone(),
            version: stored.version,
            created_at: stored.created_at,
            updated_at: stored.updated_at,
            expires_at: stored.expires_at,
            reauth_required: stored.reauth_required,
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

    fn stored(expires_at: Option<DateTime<Utc>>) -> StoredCredential {
        let now = Utc::now();
        StoredCredential {
            id: "cred_01ABCDEFGHJKMNPQRSTVWXYZ0".to_owned(),
            credential_key: "api_key".to_owned(),
            data: vec![1, 2, 3],
            state_kind: "api_key_state".to_owned(),
            state_version: 1,
            version: 4,
            created_at: now,
            updated_at: now,
            expires_at,
            reauth_required: false,
            metadata: serde_json::Map::new(),
        }
    }

    #[test]
    fn from_stored_copies_row_fields_without_data() {
        let row = stored(None);
        let head = CredentialHead::from_stored(&row, CredentialDisplay::default());
        assert_eq!(head.id, row.id);
        assert_eq!(head.credential_key, "api_key");
        assert_eq!(head.version, 4);
        assert!(!head.reauth_required);
        assert!(head.display.is_empty());
        // No `data` field exists on the head — the projection is
        // structurally secret-free; serialize and prove the bytes absent.
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
