//! Resume-token DTO for the W-S3c mint-on-park token store.
//!
//! [`TokenHash`] is the 32-byte SHA-256 of the plaintext bearer token
//! (hashed-at-rest; the hash is the primary key in `port_resume_tokens`).
//! [`ResumeTokenRow`] is the full persisted record; it carries the mint
//! scope, the execution and node that parked, and the wait kind — but
//! NEVER the plaintext token.  The plaintext lives only in the engine's
//! `SecretString` for the duration of the request; it is dropped
//! (zeroized) once the commit batch returns.
//!
//! See ADR-0099 W-S3c.
use serde::{Deserialize, Serialize};

use crate::Scope;

/// 32-byte SHA-256 digest of a plaintext resume token.
///
/// Stored as the primary key in `port_resume_tokens` (BYTEA / BLOB).
/// The bytes are the raw hash output — not hex, not base64 — so
/// case-folding collations cannot break exact-match lookups.
///
/// Constructed via [`TokenHash::try_from_bytes`]; the constructor
/// validates the 32-byte length.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TokenHash(Vec<u8>);

impl TokenHash {
    /// Wrap a 32-byte SHA-256 digest.
    ///
    /// # Errors
    ///
    /// Returns `TokenHashLengthError` when `bytes.len() != 32`.
    pub fn try_from_bytes(bytes: Vec<u8>) -> Result<Self, TokenHashLengthError> {
        if bytes.len() != 32 {
            return Err(TokenHashLengthError {
                actual: bytes.len(),
            });
        }
        Ok(Self(bytes))
    }

    /// The raw 32-byte hash slice (no copy).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// Error returned when a byte slice is not exactly 32 bytes.
#[derive(Debug, thiserror::Error)]
#[error("token hash must be exactly 32 bytes; got {actual}")]
pub struct TokenHashLengthError {
    /// Actual length supplied.
    pub actual: usize,
}

/// The wait kind that triggered a resume-token mint.
///
/// Only signal waits that a bearer can resolve externally get a token.
/// Timer waits and execution-completion waits are NOT included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ResumeTokenWaitKind {
    /// Node awaiting an inbound HTTP webhook (`WaitCondition::Webhook`).
    Webhook,
    /// Node awaiting human approval (`WaitCondition::Approval`).
    Approval,
}

/// One row in `port_resume_tokens`.
///
/// Produced by the engine at signal-park time and inserted in the
/// same [`crate::TransitionBatch`] transaction as the `Waiting` state
/// snapshot.  Consumed (atomically deleted and returned) by
/// [`crate::store::ResumeTokenStore::consume`].
///
/// The `callback_label` is the author-declared `callback_id` or
/// `approver` identifier — an internal routing label, never a secret.
/// The plaintext bearer token that was hashed into `token_hash` is
/// NEVER stored here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeTokenRow {
    /// SHA-256 of the plaintext token (primary key, 32 bytes).
    pub token_hash: TokenHash,
    /// Tenant scope the parked execution belongs to.
    pub scope: Scope,
    /// Execution that parked.
    pub execution_id: String,
    /// Node within that execution that is now `Waiting`.
    pub node_key: String,
    /// Which kind of signal wait minted this token.
    pub wait_kind: ResumeTokenWaitKind,
    /// Author-declared label (`callback_id` / `approver` — never a secret).
    pub callback_label: String,
    /// Row creation timestamp (RFC 3339).
    pub created_at: String,
    /// Optional expiry timestamp (RFC 3339); matches the wait's `wake_at`
    /// when both a signal and a timeout are active simultaneously.
    pub expires_at: Option<String>,
}
