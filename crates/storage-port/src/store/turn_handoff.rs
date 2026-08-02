//! Durable handoff from a dispatch claim to an execution turn.
//!
//! A dispatch claim protects *delivery*, not work. Holding one for the duration
//! of an action makes the queue's reclaim timeout an implicit limit on how long
//! an action may run: a slow action outlives its claim, a sweep redelivers the
//! row, and a second worker starts the same turn while the first is still
//! running it. Extending the claim while the action runs only moves the
//! problem — it turns the queue row into a second, weaker lease competing with
//! the execution aggregate's own.
//!
//! The handoff ends the claim at a definite point instead. Runtime control
//! durably accepts ownership of the execution turn under the aggregate's
//! lease/fence **and** acknowledges the queue row in one transaction, so the
//! two can never disagree:
//!
//! - Crash before the commit: the row is still `Processing`, the reclaim sweep
//!   redelivers it, and no turn was ever accepted.
//! - Crash after the commit: the row is acknowledged and the execution holds a
//!   durable lease, so recovery drives from aggregate truth rather than from
//!   the queue.
//!
//! After the handoff the action's duration is governed by the execution lease
//! and persisted recovery state. The dispatch claim is already finished, so it
//! cannot be extended by how long the action takes.

use core::fmt;
use std::time::Duration;

use crate::error::StorageError;
use crate::ids::FencingToken;
use crate::scope::Scope;
use crate::store::job_dispatch::JobClaimToken;

/// Everything one durable handoff needs, in one transaction.
#[derive(Debug, Clone)]
pub struct TurnHandoff<'a> {
    /// Tenant that owns the execution and the queue row.
    pub scope: &'a Scope,
    /// Execution whose turn is being accepted.
    pub execution_id: &'a str,
    /// Proof the caller currently owns the queue row it is acknowledging.
    ///
    /// A processor identity cannot serve here: a stable processor may claim a
    /// row, lose it to a reclaim sweep, and claim it again, so an
    /// acknowledgement issued against the first claim would terminalise a row
    /// the second claim is still working.
    pub claim: JobClaimToken,
    /// Identity recorded as the execution's lease holder.
    pub holder: &'a str,
    /// How long the accepted turn's lease is valid for.
    ///
    /// This bounds recovery latency after a crash, not how long the action may
    /// run: a live owner renews it, and the queue claim is already finished.
    pub lease_ttl: Duration,
}

/// What a durable handoff did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnAcceptance {
    /// The turn is durably owned and the queue row is acknowledged.
    ///
    /// The fence is the execution aggregate's, so every subsequent write this
    /// owner makes is rejected once a reclaim supersedes it.
    Accepted {
        /// Fence proving ownership of the accepted turn.
        fence: FencingToken,
    },
    /// The claim was superseded before the handoff committed.
    ///
    /// Nothing was written: neither the lease nor the queue row moved. The
    /// caller must not begin the turn — another worker already holds the row.
    ClaimSuperseded,
    /// Another holder owns a live lease on this execution.
    ///
    /// Nothing was written, and the queue row is deliberately left
    /// unacknowledged so the sweep can redeliver it once that lease expires.
    /// Acknowledging here would drop the turn on the floor: the row would be
    /// terminal while no owner ever ran it.
    TurnHeldByAnotherOwner,
}

/// Durable owner of the dispatch-claim → execution-turn handoff.
///
/// Implementations **own** both writes and must not delegate to
/// [`crate::store::ExecutionStore::acquire_lease`] plus
/// [`crate::store::JobDispatchQueue::mark_dispatched`] — those run as separate
/// operations, and a crash between them is exactly the state this capability
/// exists to make unreachable.
#[async_trait::async_trait]
pub trait ExecutionTurnHandoff: Send + Sync + fmt::Debug {
    /// Accept the execution turn and acknowledge the queue row, committing once.
    ///
    /// **Ordering (all backends), inside one transaction:**
    ///
    /// 1. Re-check the queue row is still `Processing` at the claim's
    ///    generation. A superseded generation returns
    ///    [`TurnAcceptance::ClaimSuperseded`] with nothing written.
    /// 2. Acquire the execution lease for `holder`. A live lease held by
    ///    someone else returns [`TurnAcceptance::TurnHeldByAnotherOwner`], again
    ///    with nothing written — including no queue acknowledgement.
    /// 3. Acknowledge the queue row.
    /// 4. Commit, returning [`TurnAcceptance::Accepted`] with the fence.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::NotFound`] when the queue row or the execution
    /// does not exist, and a connection error when the backend is unreachable.
    /// A failure at any step rolls back the whole transaction, so a turn is
    /// never accepted without its acknowledgement and a row is never
    /// acknowledged without an owner.
    async fn accept_turn(&self, handoff: &TurnHandoff<'_>) -> Result<TurnAcceptance, StorageError>;
}
