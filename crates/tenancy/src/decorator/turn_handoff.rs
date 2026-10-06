//! Scope-enforcing [`ExecutionTurnHandoff`] decorator.

use std::sync::Arc;

use nebula_storage_port::store::{
    ControlStartAcceptance, ControlStartHandoff, ControlTurnCommit, ControlTurnCommitOutcome,
    ControlTurnTransition, ExecutionTurnHandoff, TurnAcceptance, TurnHandoff,
};
use nebula_storage_port::{Scope, StorageError};

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
}

#[async_trait::async_trait]
impl ExecutionTurnHandoff for ScopedExecutionTurnHandoff {
    async fn record_control_flavor_refusal(
        &self,
        request: &nebula_storage_port::store::ControlFlavorRefusal<'_>,
    ) -> Result<nebula_storage_port::store::ControlFlavorRefusalOutcome, StorageError> {
        let claim = nebula_storage_port::store::ControlClaimToken::new(
            *request.claim().row_id(),
            request.claim().generation(),
            self.bound.clone(),
        );
        let scoped = nebula_storage_port::store::ControlFlavorRefusal::new(
            &claim,
            request.execution_id(),
            request.actual_worker_flavor_revision_id(),
        );
        self.inner.record_control_flavor_refusal(&scoped).await
    }

    fn backend_kind(&self) -> nebula_storage_port::StorageBackendKind {
        self.inner.backend_kind()
    }

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
                    commit.claim().clone(),
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
                let scoped_batch = batch.rebound_to(&self.bound);
                let scoped_commit = ControlTurnCommit::new(
                    commit.claim().clone(),
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
            handoff.claim().clone(),
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
            handoff.claim().clone(),
            handoff.worker_flavor_revision_id(),
        )
        .lease_to(handoff.holder(), handoff.lease_ttl());
        self.inner.accept_turn(&scoped_handoff).await
    }
}
