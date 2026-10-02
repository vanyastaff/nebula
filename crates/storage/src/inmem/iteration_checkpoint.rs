//! In-memory iteration-checkpoint store — the reference/conformance model.
//!
//! Rows live in the execution owner's shared state and every save runs in
//! one critical section of its mutex — the in-memory equivalent of the SQL
//! backends' single transaction under the execution fence: the lease check,
//! the decision and the write cannot be separated by a lease takeover.

use nebula_storage_port::store::CheckpointStore;
use nebula_storage_port::{
    CheckpointSaved, FencingToken, IterationCheckpoint, IterationCheckpointError,
    IterationCheckpointKey,
};

use super::execution_fence::require_live_execution;
use crate::iteration_checkpoint::{SaveDecision, decide_save, load_label, save_label};

/// The address of one stored checkpoint: `(workspace_id, org_id,
/// execution_id, node_key, action_key, action_version)`. The scope is part of
/// it, so another tenant's probe can never reach the row.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct CheckpointRowKey([String; 6]);

impl CheckpointRowKey {
    fn of(key: &IterationCheckpointKey<'_>) -> Self {
        Self([
            key.scope().workspace_id.clone(),
            key.scope().org_id.clone(),
            key.execution_id().to_owned(),
            key.node_key().to_owned(),
            key.action_key().to_owned(),
            key.action_version().to_owned(),
        ])
    }
}

/// In-memory reference implementation of the fenced iteration-checkpoint
/// store, sharing the execution owner's atomic state and clock.
#[derive(Clone, Debug)]
pub struct InMemoryCheckpointStore {
    execution: super::InMemoryExecutionStore,
}

impl InMemoryCheckpointStore {
    /// Share `execution`'s atomic state and clock: a save is fenced by the
    /// lease that store holds.
    #[must_use]
    pub fn new(execution: &super::InMemoryExecutionStore) -> Self {
        Self {
            execution: execution.clone(),
        }
    }
}

#[async_trait::async_trait]
impl CheckpointStore for InMemoryCheckpointStore {
    #[tracing::instrument(
        level = "debug",
        name = "iteration_checkpoint.load",
        skip_all,
        fields(backend = "in_memory", outcome = tracing::field::Empty)
    )]
    async fn load_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
    ) -> Result<Option<IterationCheckpoint>, IterationCheckpointError> {
        let result = Ok(self
            .execution
            .inner
            .lock()
            .iteration_checkpoints
            .get(&CheckpointRowKey::of(key))
            .cloned());
        tracing::Span::current().record("outcome", load_label(&result));
        result
    }

    #[tracing::instrument(
        level = "debug",
        name = "iteration_checkpoint.save",
        skip_all,
        fields(backend = "in_memory", iteration = checkpoint.iteration(), outcome = tracing::field::Empty)
    )]
    async fn save_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
        checkpoint: &IterationCheckpoint,
        fencing: FencingToken,
    ) -> Result<CheckpointSaved, IterationCheckpointError> {
        let result = (|| {
            let now = self.execution.clock.now();
            let mut state = self.execution.inner.lock();
            require_live_execution(&state, key.scope(), key.execution_id(), Some(fencing), now)?;
            let row_key = CheckpointRowKey::of(key);
            let stored = state.iteration_checkpoints.get(&row_key);
            let decision = decide_save(
                stored.map(|stored| (stored.iteration(), stored.state_digest())),
                checkpoint,
            )?;
            if decision != SaveDecision::AlreadyRecorded {
                state.iteration_checkpoints.insert(
                    row_key,
                    checkpoint
                        .clone()
                        .with_write_provenance(fencing.generation(), now.timestamp_millis()),
                );
            }
            Ok(decision.saved())
        })();
        tracing::Span::current().record("outcome", save_label(result));
        result
    }
}
