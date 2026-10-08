//! A controlled scheduling interleaving; every decision still comes from real storage.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use nebula_storage_port::{
    FencingToken, StorageError,
    store::{
        ControlFlavorRefusal, ControlFlavorRefusalOutcome, ControlStartAcceptance,
        ControlStartHandoff, ControlTurnCommit, ControlTurnCommitOutcome, ExecutionStore,
        ExecutionTurnHandoff, TurnAcceptance, TurnHandoff,
    },
};

#[derive(Debug)]
pub(super) struct TakeoverBeforeCommit {
    pub inner: Arc<dyn ExecutionTurnHandoff>,
    pub execution: Arc<dyn ExecutionStore>,
    pub successor: Mutex<Option<FencingToken>>,
    pub triggered: AtomicBool,
}

#[async_trait::async_trait]
impl ExecutionTurnHandoff for TakeoverBeforeCommit {
    fn backend_kind(&self) -> nebula_storage_port::StorageBackendKind {
        self.inner.backend_kind()
    }

    async fn record_control_flavor_refusal(
        &self,
        request: &ControlFlavorRefusal<'_>,
    ) -> Result<ControlFlavorRefusalOutcome, StorageError> {
        self.inner.record_control_flavor_refusal(request).await
    }

    async fn commit_control_turn(
        &self,
        request: &ControlTurnCommit<'_>,
    ) -> Result<ControlTurnCommitOutcome, StorageError> {
        if !self.triggered.swap(true, Ordering::SeqCst) {
            let transition = request.transition();
            assert!(
                self.execution
                    .release_lease(
                        transition.scope(),
                        transition.execution_id(),
                        transition.fence()
                    )
                    .await?,
                "interleaving releases the real old owner token"
            );
            let successor = self
                .execution
                .acquire_lease(
                    transition.scope(),
                    transition.execution_id(),
                    "interleaving-successor",
                    Duration::from_secs(30),
                )
                .await?
                .expect("successor takes real storage ownership");
            assert!(successor.generation() > transition.fence().generation());
            *self.successor.lock().expect("successor witness lock") = Some(successor);
        }
        // No fabricated outcome or forged request: delegate the unchanged old
        // owner's actual transition to the real backend transaction.
        self.inner.commit_control_turn(request).await
    }

    async fn accept_control_start(
        &self,
        request: &ControlStartHandoff<'_>,
    ) -> Result<ControlStartAcceptance, StorageError> {
        self.inner.accept_control_start(request).await
    }

    async fn accept_turn(&self, request: &TurnHandoff<'_>) -> Result<TurnAcceptance, StorageError> {
        self.inner.accept_turn(request).await
    }
}

#[derive(Debug)]
pub(super) struct CheckpointBeforeCommit {
    pub inner: Arc<dyn ExecutionTurnHandoff>,
    pub execution: Arc<dyn ExecutionStore>,
    pub triggered: AtomicBool,
}

#[async_trait::async_trait]
impl ExecutionTurnHandoff for CheckpointBeforeCommit {
    fn backend_kind(&self) -> nebula_storage_port::StorageBackendKind {
        self.inner.backend_kind()
    }

    async fn record_control_flavor_refusal(
        &self,
        request: &ControlFlavorRefusal<'_>,
    ) -> Result<ControlFlavorRefusalOutcome, StorageError> {
        self.inner.record_control_flavor_refusal(request).await
    }

    async fn commit_control_turn(
        &self,
        request: &ControlTurnCommit<'_>,
    ) -> Result<ControlTurnCommitOutcome, StorageError> {
        if !self.triggered.swap(true, Ordering::SeqCst) {
            let transition = request.transition();
            let row = self
                .execution
                .get(transition.scope(), transition.execution_id())
                .await?
                .expect("actual held owner row");
            assert_eq!(row.version, transition.expected_version());
            let mut state: nebula_execution::ExecutionState =
                serde::Deserialize::deserialize(&row.state).unwrap();
            state.version += 1;
            state.updated_at = chrono::Utc::now();
            let batch = nebula_storage_port::TransitionBatch::new(
                transition.scope().clone(),
                transition.execution_id(),
                row.version,
                transition.fence(),
                serde_json::to_value(&state).unwrap(),
                nebula_engine::execution_listing(&state),
            );
            self.execution.commit(batch).await?;
            let updated = self
                .execution
                .get(transition.scope(), transition.execution_id())
                .await?
                .expect("owner checkpoint remains durable");
            assert_eq!(updated.version, row.version + 1);
        }
        self.inner.commit_control_turn(request).await
    }

    async fn accept_control_start(
        &self,
        request: &ControlStartHandoff<'_>,
    ) -> Result<ControlStartAcceptance, StorageError> {
        self.inner.accept_control_start(request).await
    }

    async fn accept_turn(&self, request: &TurnHandoff<'_>) -> Result<TurnAcceptance, StorageError> {
        self.inner.accept_turn(request).await
    }
}
