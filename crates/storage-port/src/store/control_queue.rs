//! Durable control-signal outbox trait (cancel/terminate/pause + lease reclaim).
use std::time::Duration;

use crate::dto::ControlMsg;
use crate::error::StorageError;
use crate::store::ClaimGeneration;

/// Summary of a single `reclaim_stuck` sweep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReclaimOutcome {
    /// Rows moved `Processing → Pending` for redelivery.
    pub reclaimed: u64,
    /// Rows moved `Processing → Failed` because the reclaim budget ran out.
    pub exhausted: u64,
}

/// Proof that its holder owns the *current* claim on one control-queue row.
///
/// Same contract, and same reason for existing, as
/// [`crate::store::JobClaimToken`]: a stable processor identity cannot fence an
/// ABA reclaim. One runner may claim a command, lose it to the reclaim sweep,
/// and claim it again — at which point an acknowledgement issued against the
/// *first* claim still satisfies `processed_by = <self>` and terminalises a
/// command the second attempt is still dispatching. The generation makes the
/// two attempts distinguishable.
///
/// Only a storage backend mints these, and only from a row it just transitioned
/// to `Processing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ControlClaimToken {
    row_id: [u8; 16],
    generation: ClaimGeneration,
}

impl ControlClaimToken {
    /// Mint a token for a row a backend just claimed.
    #[must_use]
    pub const fn new(row_id: [u8; 16], generation: ClaimGeneration) -> Self {
        Self { row_id, generation }
    }

    /// The row this claim is for.
    #[must_use]
    pub const fn row_id(&self) -> &[u8; 16] {
        &self.row_id
    }

    /// The claim attempt this token proves ownership of.
    #[must_use]
    pub const fn generation(&self) -> ClaimGeneration {
        self.generation
    }
}

/// One claimed control command: the message plus the token that acknowledges it.
///
/// The two travel together so a caller cannot acknowledge one row with another
/// row's authority.
#[derive(Clone, Debug)]
pub struct ControlClaim {
    /// The claimed control message.
    pub msg: ControlMsg,
    /// Authority to acknowledge exactly this claim attempt.
    pub token: ControlClaimToken,
}

/// Durable control-command outbox.
///
/// `enqueue` is scoped so a low-privilege tenant cannot enqueue a
/// Cancel/Terminate for another tenant's execution (§6.1 confused-deputy
/// mitigation). Ids are typed 16-byte ULIDs (raw bytes on [`ControlMsg`]) —
/// the legacy "UTF-8 of the ULID string" encoding is gone. Acknowledgement is
/// fenced on a storage-minted claim generation, not on the processor identity.
#[async_trait::async_trait]
pub trait ControlQueue: Send + Sync + std::fmt::Debug {
    /// Enqueue a control command (scoped).
    async fn enqueue(&self, msg: &ControlMsg) -> Result<(), StorageError>;

    /// Atomically claim up to `batch_size` pending commands for
    /// `processor`. Postgres uses `FOR UPDATE SKIP LOCKED`; SQLite uses a
    /// single-consumer status flip.
    ///
    /// `processor` is recorded for observability only; it carries no
    /// authority. The returned [`ControlClaimToken`] is what acknowledges the
    /// row.
    async fn claim_pending(
        &self,
        processor: &[u8; 16],
        batch_size: u32,
    ) -> Result<Vec<ControlClaim>, StorageError>;

    /// Mark a claimed command completed.
    ///
    /// Fenced on `(row id, status = Processing, claim generation)`. A token
    /// from a superseded claim changes nothing and returns
    /// [`StorageError::FencedOut`]; an unknown row returns
    /// [`StorageError::NotFound`].
    async fn mark_completed(&self, claim: &ControlClaimToken) -> Result<(), StorageError>;

    /// Mark a claimed command failed (records `error`). Same generation fence
    /// as [`Self::mark_completed`].
    async fn mark_failed(&self, claim: &ControlClaimToken, error: &str)
    -> Result<(), StorageError>;

    /// Return a claimed command to `Pending` so the next poll redelivers it.
    ///
    /// For a dispatch that could not be completed *now* but is expected to
    /// succeed shortly — momentary lease contention, say. Without this the only
    /// way back to `Pending` is the reclaim sweep, which by design waits
    /// minutes: correct for detecting a runner that died mid-dispatch, far too
    /// slow for a command that merely arrived a few milliseconds early. A user
    /// waiting on a cancel would feel every second of it.
    ///
    /// Same generation fence as [`Self::mark_completed`], so a superseded claim
    /// cannot release a row another processor now owns. The reclaim budget is
    /// deliberately **not** consumed: this is an ordinary retry, not evidence of
    /// a stuck row, and burning the budget here would eventually fail a command
    /// that was never in trouble.
    async fn release_claim(&self, claim: &ControlClaimToken) -> Result<(), StorageError>;

    /// Reclaim rows stuck in `Processing` past `reclaim_after`. Rows under
    /// the `max_reclaim_count` budget go back to `Pending`; rows past it go
    /// to `Failed`.
    async fn reclaim_stuck(
        &self,
        reclaim_after: Duration,
        max_reclaim_count: u32,
    ) -> Result<ReclaimOutcome, StorageError>;

    /// Delete rows older than `retention`; returns the count deleted.
    async fn cleanup(&self, retention: Duration) -> Result<u64, StorageError>;
}
