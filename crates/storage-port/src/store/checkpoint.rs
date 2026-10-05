//! Fenced iteration-checkpoint store of journaled stateful actions.
use crate::dto::{
    CheckpointSaved, IterationCheckpoint, IterationCheckpointError, IterationCheckpointKey,
};
use crate::ids::FencingToken;

/// Durable, fenced, version-bound iteration checkpoints.
///
/// A journaled stateful action's node attempt records, after an iteration's
/// effect barrier passed, the next iteration to run and the state to run it
/// with; a later attempt of the node resumes there. One row per
/// [`IterationCheckpointKey`] — tenant, execution, node, action key and
/// action version — so a redeployed action reads nothing an older version
/// wrote.
///
/// **Save** runs as one transaction under the execution owner's
/// serialization point (the same fence as the operation ledger):
///
/// 1. the execution row must exist in the key's scope and hold the live lease
///    `fencing` names, by the backend's clock — otherwise
///    [`ExecutionLeaseRejected`](IterationCheckpointError::ExecutionLeaseRejected);
/// 2. no row: insert, [`CheckpointSaved::Recorded`];
/// 3. a row of a lower iteration: replace it, [`CheckpointSaved::Recorded`];
/// 4. a row of the same iteration and the same state digest: an exact
///    recommit, [`CheckpointSaved::AlreadyRecorded`], nothing changes;
/// 5. the same iteration with another digest:
///    [`Conflict`](IterationCheckpointError::Conflict);
/// 6. a row of a higher iteration:
///    [`Regressed`](IterationCheckpointError::Regressed).
///
/// The adapter writes the fencing generation and its own clock's timestamp;
/// the caller's values for both are ignored. A failure before the commit is
/// [`Unavailable`](IterationCheckpointError::Unavailable) (nothing written);
/// a lost commit acknowledgement is
/// [`AcknowledgementUnknown`](IterationCheckpointError::AcknowledgementUnknown).
///
/// **Load** reads without a fence: a checkpoint grants no authority — the
/// node's effects stay governed by its operation ledger, which the engine
/// cross-checks on resume. A row of another tenant, action key or version is
/// indistinguishable from none.
///
/// Rows are never cleared at terminal: they go with their execution
/// (`ON DELETE CASCADE`). State is never logged and never appears in an
/// error; credentials never belong in it.
#[async_trait::async_trait]
pub trait CheckpointStore: Send + Sync + std::fmt::Debug {
    /// Load the checkpoint stored under `key`, if any.
    ///
    /// # Errors
    ///
    /// [`Unavailable`](IterationCheckpointError::Unavailable) when the
    /// backend cannot answer; [`InvalidRecord`](IterationCheckpointError::InvalidRecord)
    /// when the stored row cannot be interpreted.
    async fn load_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
    ) -> Result<Option<IterationCheckpoint>, IterationCheckpointError>;

    /// Save `checkpoint` under `key`, fenced by `fencing` (see the trait
    /// documentation for the decision table).
    ///
    /// # Errors
    ///
    /// Every [`IterationCheckpointError`] the decision table names.
    async fn save_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
        checkpoint: &IterationCheckpoint,
        fencing: FencingToken,
    ) -> Result<CheckpointSaved, IterationCheckpointError>;
}
