//! Resume-token store port — consume and revoke operations.
//!
//! Minting is intentionally absent from this trait.  A token is always
//! minted inside a [`crate::TransitionBatch`] (the same transaction that
//! writes the `Waiting` state snapshot), so there is no separate
//! `write` / `insert` method here — adding one would create a window
//! where the state and the token row could diverge on a crash.
//!
//! [`ResumeTokenStore::consume`] is the resume primitive: it atomically
//! deletes and returns the row for a given hash, so a second call with
//! the same hash returns `None` (single-use by construction).
//!
//! [`ResumeTokenStore::revoke_on_terminal`] is called by the engine when
//! an execution reaches a terminal state to clean up any un-consumed
//! tokens that will never be used (e.g. the execution was cancelled while
//! a node was parked on a signal).
//!
//! See ADR-0099 W-S3c.
use crate::Scope;
use crate::dto::resume_token::{ResumeTokenRow, TokenHash};
use crate::error::StorageError;

/// Read and lifecycle operations for the resume-token store.
///
/// Minting is not here — tokens are inserted via
/// [`crate::TransitionBatch::resume_tokens`] in the same tx as the
/// `Waiting` state snapshot.  This trait exposes only the operations
/// needed after a token has been minted.
#[async_trait::async_trait]
pub trait ResumeTokenStore: Send + Sync + std::fmt::Debug {
    /// Single-use consume: atomically delete-and-return the row for
    /// `token_hash`.
    ///
    /// A second call with the same hash returns `None` (replay defense —
    /// the row was already deleted on the first consume).  A hash that was
    /// never minted, or has already been consumed, is indistinguishable
    /// from the caller's perspective (both yield `None`).
    ///
    /// **Security note — no `scope` parameter by design:** the scope is
    /// read FROM the returned row (confused-deputy boundary).  Possession
    /// of the 256-bit secret is the only authorisation required; the
    /// caller cannot forge a foreign scope to gain access to another
    /// tenant's token because the hash itself is the only lookup key.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] on a backend failure (connection error,
    /// serialization error, etc.).  `None` is NOT an error — it means
    /// the token does not exist or was already consumed.
    async fn consume(&self, token_hash: &TokenHash)
    -> Result<Option<ResumeTokenRow>, StorageError>;

    /// Revoke all tokens minted for `execution_id` within `scope`.
    ///
    /// Called by the engine when an execution reaches a terminal state
    /// (Completed / Failed / Cancelled) so that any tokens that were never
    /// consumed (e.g. the execution was cancelled while a node was parked)
    /// are cleaned up.  The caller already holds scope authority for the
    /// execution (it owns the lease), so a `scope` parameter is correct
    /// here.
    ///
    /// Returns the count of rows deleted (zero is not an error).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] on a backend failure.
    async fn revoke_on_terminal(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<u64, StorageError>;
}
