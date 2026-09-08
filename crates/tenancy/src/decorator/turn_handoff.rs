//! Scope-enforcing [`ExecutionTurnHandoff`] decorator.

use std::sync::Arc;

use nebula_storage_port::store::{
    ControlStartAcceptance, ControlStartHandoff, ControlTurnCommit, ControlTurnCommitOutcome,
    ControlTurnTransition, ExecutionTurnHandoff, TurnAcceptance, TurnHandoff,
};
use nebula_storage_port::{Scope, StorageError, TransitionBatch};

/// Forces tenant-facing durable handoff operations into one bound tenant.
///
/// Global abandoned-turn discovery is exposed through the separate
/// `TurnRecovery` capability and is intentionally absent here.
#[derive(Clone)]
pub struct ScopedExecutionTurnHandoff {
    inner: Arc<dyn ExecutionTurnHandoff>,
    bound: Scope,
}

impl std::fmt::Debug for ScopedExecutionTurnHandoff {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ScopedExecutionTurnHandoff")
            .field("bound", &self.bound)
            .finish_non_exhaustive()
    }
}

impl ScopedExecutionTurnHandoff {
    /// Bind `inner` to `scope`.
    #[must_use]
    pub fn new(inner: Arc<dyn ExecutionTurnHandoff>, scope: Scope) -> Self {
        Self {
            inner,
            bound: scope,
        }
    }

    fn rebind_batch(&self, batch: &TransitionBatch) -> Result<TransitionBatch, StorageError> {
        let outbox = batch
            .outbox()
            .iter()
            .cloned()
            .map(|mut message| {
                message.scope = self.bound.clone();
                message
            })
            .collect();
        let resume_tokens = batch
            .resume_tokens()
            .iter()
            .cloned()
            .map(|mut token| {
                token.scope = self.bound.clone();
                token
            })
            .collect();
        let mut builder = TransitionBatch::builder()
            .scope(self.bound.clone())
            .execution_id(batch.execution_id())
            .expected_version(batch.expected_version())
            .fencing(batch.fencing())
            .new_state(batch.new_state().clone())
            .outbox(outbox)
            .journal(batch.journal().to_vec())
            .resume_tokens(resume_tokens);
        if let Some(reference_transition) = batch.reference_transition() {
            builder = builder.reference_transition(reference_transition);
        }
        builder.build()
    }
}

#[async_trait::async_trait]
impl ExecutionTurnHandoff for ScopedExecutionTurnHandoff {
    async fn commit_control_turn(
        &self,
        commit: &ControlTurnCommit<'_>,
    ) -> Result<ControlTurnCommitOutcome, StorageError> {
        match commit.transition() {
            ControlTurnTransition::Unchanged {
                execution_id,
                expected_version,
                fence,
                ..
            } => {
                let scoped_commit = ControlTurnCommit::new(
                    commit.claim(),
                    commit.worker_flavor_revision_id(),
                    commit.command().clone(),
                    ControlTurnTransition::Unchanged {
                        scope: &self.bound,
                        execution_id,
                        expected_version: *expected_version,
                        fence: *fence,
                    },
                );
                self.inner.commit_control_turn(&scoped_commit).await
            },
            ControlTurnTransition::Checkpoint(batch) => {
                let scoped_batch = self.rebind_batch(batch)?;
                let scoped_commit = ControlTurnCommit::new(
                    commit.claim(),
                    commit.worker_flavor_revision_id(),
                    commit.command().clone(),
                    ControlTurnTransition::Checkpoint(&scoped_batch),
                );
                self.inner.commit_control_turn(&scoped_commit).await
            },
            _ => Err(StorageError::Configuration(
                "unsupported control turn transition".into(),
            )),
        }
    }

    async fn accept_control_start(
        &self,
        handoff: &ControlStartHandoff<'_>,
    ) -> Result<ControlStartAcceptance, StorageError> {
        let scoped_handoff = ControlStartHandoff::for_claim(
            &self.bound,
            handoff.execution_id(),
            handoff.claim(),
            handoff.worker_flavor_revision_id(),
        )
        .at_version(handoff.expected_execution_version())
        .lease_to(handoff.holder(), handoff.lease_ttl());
        self.inner.accept_control_start(&scoped_handoff).await
    }

    async fn accept_turn(&self, handoff: &TurnHandoff<'_>) -> Result<TurnAcceptance, StorageError> {
        let scoped_handoff = TurnHandoff::for_claim(
            &self.bound,
            handoff.execution_id(),
            handoff.claim(),
            handoff.worker_flavor_revision_id(),
        )
        .lease_to(handoff.holder(), handoff.lease_ttl());
        self.inner.accept_turn(&scoped_handoff).await
    }
}
