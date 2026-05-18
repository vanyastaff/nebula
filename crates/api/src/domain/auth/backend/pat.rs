//! Personal Access Token primitives.
//!
//! A PAT has the on-the-wire shape `pat_<32 bytes URL-safe base64>`. The
//! backend stores only the SHA-256 of the *full* token; lookups hash the
//! incoming bearer value and compare in constant time.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use nebula_core::UserId;
use rand::Rng;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::error::AuthError;

/// Canonical prefix that identifies a Nebula personal access token.
pub const PAT_PREFIX: &str = "pat_";

/// PAT secret length in random bytes (256 bits of entropy).
pub const PAT_BYTES: usize = 32;

/// PAT-secret length in characters of the URL-safe base64 form (no padding).
const PAT_BODY_CHARS: usize = 43;

/// A freshly-minted PAT: returned to the caller **once**, never logged.
#[derive(Debug, Clone)]
pub struct MintedPat {
    /// Plaintext token to display to the user — `pat_<...>`.
    pub plaintext: String,
    /// Server-stored record (no plaintext).
    pub record: PatRecord,
}

/// Server-side PAT record (everything *except* the plaintext).
#[derive(Debug, Clone)]
pub struct PatRecord {
    /// Opaque PAT identifier — `pat_<ULID>` (NOT the secret).
    pub id: String,
    /// Owning user.
    pub user_id: UserId,
    /// Caller-chosen label.
    pub name: String,
    /// First 12 chars of the plaintext token, for display in lists.
    pub prefix: String,
    /// SHA-256 of the *full* plaintext token.
    pub hash: [u8; 32],
    /// Allowed scopes. Use `full_access` for complete access; empty scopes
    /// are invalid at the API auth boundary.
    pub scopes: Vec<String>,
    /// When the PAT was created.
    pub created_at: DateTime<Utc>,
    /// Optional wall-clock expiry; `None` means no expiry.
    pub expires_at: Option<DateTime<Utc>>,
    /// Last successful authentication using this PAT, if any.
    pub last_used_at: Option<DateTime<Utc>>,
    /// When the PAT was revoked, if applicable.
    pub revoked_at: Option<DateTime<Utc>>,
}

impl PatRecord {
    /// Whether this PAT can still authenticate at `now`.
    #[must_use]
    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        if self.revoked_at.is_some() {
            return false;
        }
        if let Some(exp) = self.expires_at
            && exp <= now
        {
            return false;
        }
        true
    }
}

/// Mint a new PAT for `user_id` with the given `name` and `scopes`.
pub fn mint_pat(
    user_id: UserId,
    name: String,
    scopes: Vec<String>,
    expires_at: Option<DateTime<Utc>>,
) -> Result<MintedPat, AuthError> {
    let mut secret = [0u8; PAT_BYTES];
    rand::rng().fill_bytes(&mut secret);
    let body = URL_SAFE_NO_PAD.encode(secret);
    let plaintext = format!("{PAT_PREFIX}{body}");

    let mut id_bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut id_bytes);
    let id = format!("pat_{}", URL_SAFE_NO_PAD.encode(id_bytes));

    let prefix = plaintext.chars().take(12).collect();
    let hash = sha256(&plaintext);

    Ok(MintedPat {
        plaintext,
        record: PatRecord {
            id,
            user_id,
            name,
            prefix,
            hash,
            scopes,
            created_at: Utc::now(),
            expires_at,
            last_used_at: None,
            revoked_at: None,
        },
    })
}

/// Hash a plaintext PAT for lookup. Returns `Err` if the prefix or shape is
/// wrong — backends never log the plaintext on failure.
pub fn hash_for_lookup(presented: &str) -> Result<[u8; 32], AuthError> {
    let body = presented
        .strip_prefix(PAT_PREFIX)
        .ok_or(AuthError::InvalidCredentials)?;
    if body.len() != PAT_BODY_CHARS {
        return Err(AuthError::InvalidCredentials);
    }
    Ok(sha256(presented))
}

/// Constant-time comparison of two SHA-256 digests.
#[must_use]
pub fn hashes_equal(a: &[u8; 32], b: &[u8; 32]) -> bool {
    a.ct_eq(b).into()
}

fn sha256(s: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> UserId {
        UserId::new()
    }

    #[test]
    fn mint_pat_returns_pat_prefixed_token() {
        let m = mint_pat(user(), "ci".to_owned(), vec![], None).expect("mint");
        assert!(m.plaintext.starts_with(PAT_PREFIX));
        assert!(m.record.prefix.starts_with(PAT_PREFIX));
        assert_eq!(m.record.prefix.len(), 12);
        assert_eq!(m.record.scopes, Vec::<String>::new());
        assert!(m.record.is_active(Utc::now()));
    }

    #[test]
    fn mint_pat_two_calls_produce_distinct_secrets() {
        let a = mint_pat(user(), "a".to_owned(), vec![], None).unwrap();
        let b = mint_pat(user(), "b".to_owned(), vec![], None).unwrap();
        assert_ne!(a.plaintext, b.plaintext);
        assert_ne!(a.record.hash, b.record.hash);
    }

    #[test]
    fn lookup_hash_matches_minted_record() {
        let m = mint_pat(user(), "ci".to_owned(), vec![], None).unwrap();
        let h = hash_for_lookup(&m.plaintext).unwrap();
        assert!(hashes_equal(&h, &m.record.hash));
    }

    #[test]
    fn lookup_hash_rejects_wrong_prefix() {
        let err =
            hash_for_lookup("nbl_sk_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap_err();
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[test]
    fn lookup_hash_rejects_wrong_length() {
        let err = hash_for_lookup("pat_short").unwrap_err();
        assert!(matches!(err, AuthError::InvalidCredentials));
    }

    #[test]
    fn revoked_pat_is_inactive() {
        let mut m = mint_pat(user(), "ci".to_owned(), vec![], None).unwrap();
        m.record.revoked_at = Some(Utc::now());
        assert!(!m.record.is_active(Utc::now()));
    }

    #[test]
    fn expired_pat_is_inactive() {
        let past = Utc::now() - chrono::Duration::seconds(60);
        let mut m = mint_pat(user(), "ci".to_owned(), vec![], None).unwrap();
        m.record.expires_at = Some(past);
        assert!(!m.record.is_active(Utc::now()));
    }
}
