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
//! [`ResumeTokenStore::revoke_on_terminal`] is the cleanup primitive for
//! the un-consumed tokens a terminal execution leaves behind (e.g. it was
//! cancelled while a node was parked on a signal). The engine calls it,
//! best-effort, at its terminal sinks (W-S3e) so dead tokens are purged
//! proactively; the `ON DELETE CASCADE` from `port_executions` remains the
//! backstop for the crash window between the terminal commit and the revoke.
//!
//! See ADR-0099 W-S3c (this trait) and W-S3e (the engine wiring).
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
    /// **Cleanup primitive (wired in the engine — W-S3e).** The engine calls
    /// this, best-effort and post-commit, at its terminal sinks (the
    /// consolidated final-state persist and the no-live-runner
    /// cancel-of-parked cleanup) to clean up tokens that were never consumed
    /// (e.g. the execution was cancelled while a node was parked). The revoke
    /// is intentionally NOT atomic with the terminal transition; the
    /// `ON DELETE CASCADE` constraint on `port_resume_tokens` (firing when the
    /// `port_executions` row is deleted) backstops the crash window between
    /// the terminal commit and the revoke.
    ///
    /// The caller must already hold scope authority for the execution (owns
    /// the lease), so a `scope` parameter is correct here.
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
