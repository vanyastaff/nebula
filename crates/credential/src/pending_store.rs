//! Trait for managing ephemeral state in interactive credential flows.

use std::future::Future;

use crate::pending::{PendingState, PendingToken};

/// Manages ephemeral pending state for interactive credential flows.
///
/// Separate subsystem from [`CredentialStore`](crate::store::CredentialStore):
/// different lifecycle (minutes, not years), TTL-enforced, single-use consume.
///
/// # Security: 4-dimensional token binding
///
/// Store key = (credential_kind, owner_id, session_id, token_id).
/// All four validated on [`consume()`](Self::consume):
///
/// | Dimension | Prevents |
/// |-----------|----------|
/// | credential_kind | Type confusion (credential A reading B's state) |
/// | owner_id | Cross-user token replay |
/// | session_id | Session fixation / confused deputy |
/// | token_id | Token guessing (32-byte CSPRNG) |
pub trait PendingStateStore: Send + Sync {
    /// Stores pending state, returns an opaque token.
    ///
    /// The token is bound to (credential_kind, owner_id, session_id).
    fn put<P: PendingState>(
        &self,
        credential_kind: &str,
        owner_id: &str,
        session_id: &str,
        pending: P,
    ) -> impl Future<Output = Result<PendingToken, PendingStoreError>> + Send;

    /// Reads pending state without consuming (for polling flows like device code).
    fn get<P: PendingState>(
        &self,
        token: &PendingToken,
    ) -> impl Future<Output = Result<P, PendingStoreError>> + Send;

    /// Reads and deletes atomically (single-use consume).
    ///
    /// Validates all 4 dimensions: credential_kind, owner_id, session_id must
    /// match the values provided at `put()` time.
    fn consume<P: PendingState>(
        &self,
        credential_kind: &str,
        token: &PendingToken,
        owner_id: &str,
        session_id: &str,
    ) -> impl Future<Output = Result<P, PendingStoreError>> + Send;

    /// Explicit delete (cleanup on error paths).
    fn delete(
        &self,
        token: &PendingToken,
    ) -> impl Future<Output = Result<(), PendingStoreError>> + Send;
}

/// Error from pending state operations.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PendingStoreError {
    /// No pending state found for this token.
    #[error("pending state not found")]
    NotFound,

    /// Pending state has expired (TTL exceeded).
    #[error("pending state expired")]
    Expired,

    /// Already consumed (single-use violation).
    #[error("pending state already consumed")]
    AlreadyConsumed,

    /// 4-dimensional validation failed.
    #[error("validation failed: {reason}")]
    ValidationFailed {
        /// Which dimension failed and why.
        reason: String,
    },

    /// Backend storage error.
    #[error("pending store backend error: {0}")]
    Backend(Box<dyn std::error::Error + Send + Sync>),
}
