//! Browser-session token lookup digests.
//!
//! A session cookie is a 256-bit random bearer issued once to the browser.
//! Persistence never needs the bearer itself: repositories derive this
//! domain-separated SHA-256 digest at their create/read/touch/revoke boundary
//! and use only the digest as the database key.

use sha2::{Digest, Sha256};

const SESSION_TOKEN_DIGEST_DOMAIN: &[u8] = b"nebula:plane-a:session-cookie:v1\0";

/// Fixed-size lookup digest for one opaque browser-session token.
///
/// The digest is not accepted by the HTTP surface as a bearer. Its `Debug`
/// representation is nevertheless redacted so identifiers derived from live
/// authority do not drift into diagnostics.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionTokenDigest([u8; 32]);

impl SessionTokenDigest {
    /// Reconstruct a digest read from the fixed-width persistence column.
    #[cfg(feature = "postgres")]
    pub(crate) const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the 32 bytes stored in the session lookup column.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl std::fmt::Debug for SessionTokenDigest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SessionTokenDigest([redacted])")
    }
}

/// Derive the storage lookup key for a presented session-cookie token.
///
/// The fixed domain prefix prevents the digest from being confused with any
/// other SHA-256 lookup surface (PATs, verification tokens, OAuth state, or
/// future capability tokens). Session tokens carry 256 bits of CSPRNG entropy,
/// so an unsalted one-way lookup digest does not enable an offline dictionary
/// attack.
#[must_use]
pub fn session_token_digest(presented_token: &[u8]) -> SessionTokenDigest {
    let mut hasher = Sha256::new();
    hasher.update(SESSION_TOKEN_DIGEST_DOMAIN);
    hasher.update(presented_token);
    SessionTokenDigest(hasher.finalize().into())
}
