//! Actual WorkerRuntime restart from queue-independent accepted-turn markers.

#[path = "accepted_turn_recovery/support.rs"]
mod support;

use nebula_storage_port::{
    FencingToken, Scope, StorageError,
    dto::{
        EffectOccurrenceKey, EffectPhase, EffectSlotBinding, EffectSlotId, InvocationDisposition,
        OperationAdvance, OperationCommand, OperationLedgerError, OperationRecord, PrepareOutcome,
    },
    store::{
        ControlStartAcceptance, ControlStartHandoff, ExecutionTurnHandoff, OperationLedger,
        RecoverableTurnPage, RecoveryTurnAcceptance, RecoveryTurnHandoff, TurnAcceptance,
        TurnHandoff, TurnRecovery,
    },
};
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use support::{Admitted, Backend, Calls, EffectObservations};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct LostAcknowledgement {
    owner: Arc<dyn ExecutionTurnHandoff>,
    committed: Arc<tokio::sync::Notify>,
}

#[derive(Debug)]
struct FailingDiscovery {
    owner: Arc<dyn TurnRecovery>,
}

#[derive(Debug)]
struct PauseAfterBeforeBoundary {
    owner: Arc<dyn OperationLedger>,
    disposition_committed: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl OperationLedger for PauseAfterBeforeBoundary {
    async fn read_occurrence(
        &self,
        key: &EffectOccurrenceKey<'_>,
    ) -> Result<Option<OperationRecord>, OperationLedgerError> {
        self.owner.read_occurrence(key).await
    }

    async fn prepare(
        &self,
        binding: &EffectSlotBinding<'_>,
        fencing: FencingToken,
    ) -> Result<PrepareOutcome, OperationLedgerError> {
        self.owner.prepare(binding, fencing).await
    }

    async fn read_exact(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
    ) -> Result<OperationRecord, OperationLedgerError> {
        self.owner.read_exact(scope, slot_id).await
    }

    async fn advance(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        fencing: FencingToken,
        command: &OperationCommand,
    ) -> Result<OperationAdvance, OperationLedgerError> {
        let pause_after_commit = matches!(
            command,
            OperationCommand::RecordDisposition {
                disposition: InvocationDisposition::BeforeBoundary,
                ..
            }
        );
        let result = self.owner.advance(scope, slot_id, fencing, command).await;
        if pause_after_commit && result.is_ok() {
            self.disposition_committed.notify_one();
            std::future::pending::<()>().await;
        }
        result
    }
}

#[async_trait::async_trait]
impl TurnRecovery for FailingDiscovery {
    async fn list_recoverable_turns(
        &self,
        _flavor: nebula_core::WorkerFlavorRevisionId,
        _after: Option<&str>,
        _limit: u32,
    ) -> Result<RecoverableTurnPage, StorageError> {
        Err(StorageError::Timeout {
            operation: "accepted turn discovery".into(),
            duration: Duration::from_secs(1),
        })
    }

    async fn accept_recovery_turn(
        &self,
        request: &RecoveryTurnHandoff<'_>,
    ) -> Result<RecoveryTurnAcceptance, StorageError> {
        self.owner.accept_recovery_turn(request).await
    }
}
#[async_trait::async_trait]
impl ExecutionTurnHandoff for LostAcknowledgement {
    async fn commit_control_turn(
        &self,
        request: &nebula_storage_port::store::ControlTurnCommit<'_>,
    ) -> Result<nebula_storage_port::store::ControlTurnCommitOutcome, StorageError> {
        self.owner.commit_control_turn(request).await
    }

    async fn accept_control_start(
        &self,
        request: &ControlStartHandoff<'_>,
    ) -> Result<ControlStartAcceptance, StorageError> {
        assert!(matches!(
            self.owner.accept_control_start(request).await.unwrap(),
            ControlStartAcceptance::Accepted { .. }
        ));
        self.committed.notify_one();
        Err(StorageError::AcknowledgementUnknown {
            operation: "control_start_handoff",
        })
    }
    async fn accept_turn(&self, request: &TurnHandoff<'_>) -> Result<TurnAcceptance, StorageError> {
        self.owner.accept_turn(request).await
    }
}

async fn wait_for_engine_drop(engine: std::sync::Weak<nebula_engine::WorkflowEngine>) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while engine.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("all prior runtime adapters must be dropped before reconnect");
}

async fn assert_completed(backend: &Backend, admitted: &Admitted) {
    let ports = backend.ports();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let record = ports
                .stores
                .execution
                .get(&admitted.scope, &admitted.id.to_string())
                .await
                .unwrap()
                .unwrap();
            let state: nebula_execution::ExecutionState =
                serde_json::from_slice(&serde_json::to_vec(&record.state).unwrap()).unwrap();
            if state.status.is_terminal() {
                assert_eq!(state.status, nebula_execution::ExecutionStatus::Completed);
                for node in [
                    nebula_core::node_key!("predecessor"),
                    nebula_core::node_key!("successor"),
                ] {
                    let nebula_execution::NodeCheckpoint::ActionResult { value, .. } = state
                        .checkpoint
                        .as_ref()
                        .unwrap()
                        .nodes()
                        .get(&node)
                        .unwrap()
                    else {
                        panic!("actual completed action evidence required");
                    };
                    let result: nebula_action::ActionResult<serde_json::Value> =
                        serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap();
                    let nebula_action::ActionResult::Success { output } = result else {
                        panic!("echo must succeed");
                    };
                    assert_eq!(output.as_value(), Some(&admitted.input));
                }
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("startup discovery must recover accepted execution without a queue message");
}

async fn wait_for_effect_completion(backend: &Backend, admitted: &Admitted) {
    let execution = admitted.id.to_string();
    let ports = backend.ports();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let record = ports
                .stores
                .execution
                .get(&admitted.scope, &execution)
                .await
                .unwrap()
                .unwrap();
            let state: nebula_execution::ExecutionState =
                serde_json::from_slice(&serde_json::to_vec(&record.state).unwrap()).unwrap();
            if state.status.is_terminal() {
                assert_eq!(state.status, nebula_execution::ExecutionStatus::Completed);
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("recovered remote effect must complete through the accepted-turn marker");
}

async fn worker_restart(kind: &str) {
    let Some(mut backend) = Backend::new(kind).await else {
        return;
    };
    for warm in [false, true] {
        let calls = Arc::new(Calls::default());
        let admitted = support::admit(backend.ports(), &calls).await;
        let mut ports = backend.ports();
        let acknowledged = Arc::new(tokio::sync::Notify::new());
        if !warm {
            ports.handoff = Arc::new(LostAcknowledgement {
                owner: ports.handoff,
                committed: acknowledged.clone(),
            });
        }
        let (worker, weak) = support::worker(ports, &calls, warm);
        let shutdown = CancellationToken::new();
        let task = worker.spawn(shutdown.clone());
        if warm {
            tokio::time::timeout(Duration::from_secs(10), calls.entered.notified())
                .await
                .unwrap();
            assert_eq!(calls.predecessor.load(Ordering::SeqCst), 1);
            assert_eq!(calls.successor.load(Ordering::SeqCst), 1);
        } else {
            tokio::time::timeout(Duration::from_secs(10), acknowledged.notified())
                .await
                .unwrap();
            assert_eq!(calls.predecessor.load(Ordering::SeqCst), 0);
            assert_eq!(calls.successor.load(Ordering::SeqCst), 0);
        }
        if warm {
            // The successor is observably in flight, so aborting here models
            // a process loss during an accepted turn.
            task.abort();
            shutdown.cancel();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            // Drain the acknowledgement-unknown result before dropping the
            // runtime. The durable commit notification happens inside the
            // storage call, while successful shutdown proves the consumer
            // received and handled the returned error.
            shutdown.cancel();
            tokio::time::timeout(Duration::from_secs(10), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
        wait_for_engine_drop(weak).await;
        backend.delete_completed_delivery().await;
        backend.reconnect().await;
        backend.release_abandoned_lease(&admitted).await;
        let (worker, weak) = support::worker(backend.ports(), &calls, false);
        let shutdown = CancellationToken::new();
        let task = worker.spawn(shutdown.clone());
        assert_completed(&backend, &admitted).await;
        assert_eq!(
            calls.predecessor.load(Ordering::SeqCst),
            1,
            "committed predecessor cannot run again"
        );
        assert_eq!(
            calls.successor.load(Ordering::SeqCst),
            if warm { 2 } else { 1 }
        );
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        wait_for_engine_drop(weak).await;
        backend.reconnect().await;
    }
}

async fn remote_effect_restart(kind: &str) {
    let Some(mut backend) = Backend::new(kind).await else {
        return;
    };
    let observations = Arc::new(EffectObservations::default());
    let admitted = support::admit_remote_effect(backend.ports(), &observations).await;
    let execution = admitted.id.to_string();
    let mut ports = backend.ports();
    let durable_ledger = ports.stores.operation_ledger.clone();
    let disposition_committed = Arc::new(tokio::sync::Notify::new());
    ports.stores.operation_ledger = Arc::new(PauseAfterBeforeBoundary {
        owner: durable_ledger.clone(),
        disposition_committed: disposition_committed.clone(),
    });
    let (worker, weak) = support::effect_worker(ports, &observations);
    let shutdown = CancellationToken::new();
    let task = worker.spawn(shutdown.clone());

    tokio::time::timeout(Duration::from_secs(10), disposition_committed.notified())
        .await
        .expect("the first worker must durably record the provider boundary");
    let original = durable_ledger
        .read_occurrence(&EffectOccurrenceKey::new(
            &admitted.scope,
            &execution,
            "effect",
            "node-effect/v1",
        ))
        .await
        .unwrap()
        .expect("the durable effect occurrence must exist before process loss");
    assert_eq!(
        original.protocol().unwrap().phase(),
        EffectPhase::BeforeBoundary
    );
    assert_eq!(
        observations.operation_ids(),
        vec![original.operation().operation_id()]
    );
    assert_eq!(observations.business_effects(), 0);

    task.abort();
    shutdown.cancel();
    assert!(task.await.unwrap_err().is_cancelled());
    wait_for_engine_drop(weak).await;
    drop(durable_ledger);
    backend.reconnect().await;
    backend.release_abandoned_lease(&admitted).await;

    let ports = backend.ports();
    let recovered_ledger = ports.stores.operation_ledger.clone();
    let (worker, weak) = support::effect_worker(ports, &observations);
    let shutdown = CancellationToken::new();
    let task = worker.spawn(shutdown.clone());
    wait_for_effect_completion(&backend, &admitted).await;

    let recovered = recovered_ledger
        .read_occurrence(&EffectOccurrenceKey::new(
            &admitted.scope,
            &execution,
            "effect",
            "node-effect/v1",
        ))
        .await
        .unwrap()
        .expect("the recovered effect occurrence must remain durable");
    let operation_id = original.operation().operation_id();
    assert_eq!(recovered.operation().operation_id(), operation_id);
    assert_eq!(recovered.protocol().unwrap().phase(), EffectPhase::Resolved);
    assert_eq!(
        observations.operation_ids(),
        vec![operation_id, operation_id]
    );
    assert_eq!(observations.business_effects(), 1);

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    wait_for_engine_drop(weak).await;
}

#[tokio::test]
async fn memory_worker_recovers_accepted_cold_and_warm_turns() {
    worker_restart("memory").await;
}

#[tokio::test]
async fn aborted_worker_drops_engine_without_out_of_band_shutdown() {
    let backend = Backend::new("memory").await.unwrap();
    let calls = Arc::new(Calls::default());
    let (worker, weak) = support::worker(backend.ports(), &calls, false);
    let shutdown = CancellationToken::new();
    let task = worker.spawn(shutdown);
    // Let the actual startup sweep create its periodic timer child. No caller
    // retains or cancels the shutdown token after killing the worker task.
    tokio::time::sleep(Duration::from_millis(20)).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    wait_for_engine_drop(weak).await;
}

#[tokio::test(start_paused = true)]
async fn persistent_discovery_failure_stops_the_supervised_worker() {
    let backend = Backend::new("memory").await.unwrap();
    let calls = Arc::new(Calls::default());
    let mut ports = backend.ports();
    ports.recovery = Arc::new(FailingDiscovery {
        owner: ports.recovery,
    });
    let (worker, weak) = support::worker(ports, &calls, false);
    let task = worker.spawn(CancellationToken::new());

    tokio::task::yield_now().await;
    for backoff in [2, 4, 8, 16] {
        tokio::time::advance(Duration::from_secs(backoff)).await;
        tokio::task::yield_now().await;
    }

    let error = task.await.unwrap().unwrap_err();
    std::assert_matches!(
        error,
        nebula_worker::WorkerRuntimeError::AcceptedTurnRecovery { attempts: 5, .. }
    );
    wait_for_engine_drop(weak).await;
}
#[tokio::test]
async fn sqlite_worker_recovers_accepted_cold_and_warm_turns_after_queue_retention() {
    worker_restart("sqlite").await;
}
#[tokio::test]
async fn postgres_worker_recovers_accepted_cold_and_warm_turns_after_queue_retention() {
    worker_restart("postgres").await;
}

#[tokio::test]
async fn memory_worker_preserves_remote_effect_identity_across_recovery() {
    remote_effect_restart("memory").await;
}

#[tokio::test]
async fn sqlite_worker_preserves_remote_effect_identity_across_recovery() {
    remote_effect_restart("sqlite").await;
}

#[tokio::test]
async fn postgres_worker_preserves_remote_effect_identity_across_recovery() {
    remote_effect_restart("postgres").await;
}
