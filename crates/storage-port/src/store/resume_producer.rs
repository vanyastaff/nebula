//! Resume-producer port — atomic single-use consume + Resume-enqueue.
//!
//! [`ResumeTokenStore::consume`](crate::store::ResumeTokenStore::consume) burns
//! a resume token (atomic `DELETE … RETURNING`) but cannot, in the same
//! transaction, enqueue the `Resume` it authorises. A `POST /resume` handler
//! that consumes-then-enqueues across two statements has a durability gap: if
//! the enqueue fails after the token is burned, the token is gone and no
//! `Resume` exists — the caller retries, the now-absent token yields a `404`,
//! and a `timeout: None` webhook wait parks `Paused` forever.
//!
//! [`ResumeProducer`] closes that gap. `consume_and_enqueue_resume` deletes the
//! token row and inserts the `Resume` control message in ONE transaction: the
//! `DELETE … RETURNING` (rows-affected == 1) is the single-use replay gate, and
//! the enqueue rides the same commit, so either both land or neither does. A
//! transient backend fault rolls the transaction back, leaving the token live
//! for the caller's retry.
//!
//! [`ResumeProducer::peek`] is the read-only lookup the handler uses BEFORE the
//! burn — to reject wrong-kind / expired tokens (and surface storage faults as
//! `503`) without consuming the token.
//!
//! See ADR-0099 W-S3d.
use crate::dto::ControlMsg;
use crate::dto::resume_token::{ResumeTokenRow, TokenHash};
use crate::error::StorageError;

/// Atomic consume + Resume-enqueue for the `POST /resume` producer.
///
/// Object-safe — consumed as `Arc<dyn ResumeProducer>`. Backed by the same
/// pool / shared state as [`crate::store::ExecutionStore`] and
/// [`crate::store::ControlQueue`] so the token DELETE and the control-queue
/// INSERT commit in one transaction.
#[async_trait::async_trait]
pub trait ResumeProducer: Send + Sync + std::fmt::Debug {
    /// Read-only lookup by hash — does NOT delete the row.
    ///
    /// Returns the row if a token with `token_hash` is present, else `None`.
    /// The caller inspects `wait_kind` / `expires_at` and surfaces a storage
    /// `Err` as `503` (token NOT burned) before deciding to consume.
    ///
    /// **Security note — no `scope` parameter by design:** scope is read FROM
    /// the returned row (the same confused-deputy boundary as
    /// [`crate::store::ResumeTokenStore::consume`]). Possession of the 256-bit
    /// secret is the only authority; the caller cannot forge a foreign scope
    /// because the hash itself is the only lookup key.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] on a backend failure. `None` is NOT an error —
    /// it means the token does not exist or was already consumed.
    async fn peek(&self, token_hash: &TokenHash) -> Result<Option<ResumeTokenRow>, StorageError>;

    /// Atomic single-use consume + Resume-enqueue in ONE transaction.
    ///
    /// Deletes the token row for `token_hash`; IFF exactly one row is deleted,
    /// inserts `resume_msg` into the control queue and commits. The
    /// `DELETE … RETURNING` (rows-affected == 1) IS the single-use replay gate:
    /// a second call with the same hash deletes zero rows and returns
    /// `Ok(false)` without enqueuing.
    ///
    /// Returns:
    /// - `Ok(true)` — row deleted AND `Resume` enqueued (caller → `202`).
    /// - `Ok(false)` — zero rows deleted; raced, replayed, or absent
    ///   (caller → uniform `404`).
    ///
    /// On any backend fault the transaction is rolled back: the token row is
    /// left intact (live for retry) and no `Resume` is enqueued. This is the
    /// durability invariant the two-statement consume-then-enqueue lacked.
    ///
    /// `resume_msg` is built by the caller and passed in pre-formed; its
    /// `scope` MUST be the row's scope (never request-derived) and its
    /// `command` MUST be [`crate::dto::ControlCommand::Resume`].
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] on a backend failure (the transaction is rolled
    /// back; the token survives).
    async fn consume_and_enqueue_resume(
        &self,
        token_hash: &TokenHash,
        resume_msg: &ControlMsg,
    ) -> Result<bool, StorageError>;
}
