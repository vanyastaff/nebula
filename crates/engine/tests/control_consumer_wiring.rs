//! A1 wiring tests for `ControlConsumer` (canon §12.2, ADR-0008).
//!
//! These tests assert the skeleton exists and functions as a durable-outbox
//! consumer:
//!
//! 1. Construction compiles using only engine-public + storage-port types — no `nebula_api::*`
//!    leaks, no `nebula_storage::rows::*` (row / private) types on the consumer's signature.
//! 2. The consumer observes a queued command via the engine-owned `ControlDispatch` trait.
//! 3. Graceful shutdown via `CancellationToken` completes the spawned task.
//!
//! A2 and A3 replace the test `ControlDispatch` mock with real
//! engine-side dispatch and add assertions about engine state transitions;
//! A1 only asserts that the wiring plumbing is reachable end-to-end.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use nebula_core::id::ExecutionId;
use nebula_engine::{ControlConsumer, ControlDispatch, ControlDispatchError};
use nebula_storage::repos::{
    ControlCommand, ControlQueueEntry, ControlQueueRepo, InMemoryControlQueueRepo,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

/// Records every dispatch invocation so tests can assert the consumer
/// translated storage rows → typed engine calls correctly.
#[derive(Default)]
struct RecordingDispatch {
    observations: Mutex<Vec<(ControlCommand, ExecutionId)>>,
    notify: Notify,
}

impl RecordingDispatch {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn record(&self, cmd: ControlCommand, id: ExecutionId) {
        self.observations.lock().expect("poisoned").push((cmd, id));
        self.notify.notify_waiters();
    }

    fn snapshot(&self) -> Vec<(ControlCommand, ExecutionId)> {
        self.observations.lock().expect("poisoned").clone()
    }
}

#[async_trait]
impl ControlDispatch for RecordingDispatch {
    async fn dispatch_start(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError> {
        self.record(ControlCommand::Start, execution_id);
        Ok(())
    }

    async fn dispatch_cancel(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError> {
        self.record(ControlCommand::Cancel, execution_id);
        Ok(())
    }

    async fn dispatch_terminate(
        &self,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        self.record(ControlCommand::Terminate, execution_id);
        Ok(())
    }

    async fn dispatch_resume(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError> {
        self.record(ControlCommand::Resume, execution_id);
        Ok(())
    }

    async fn dispatch_restart(
        &self,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        self.record(ControlCommand::Restart, execution_id);
        Ok(())
    }
}

fn queue_entry(
    execution_id: &ExecutionId,
    command: ControlCommand,
    row_id: u8,
) -> ControlQueueEntry {
    ControlQueueEntry {
        id: vec![row_id; 16],
        execution_id: execution_id.to_string().into_bytes(),
        command,
        issued_by: None,
        issued_at: chrono::Utc::now(),
        status: "Pending".to_string(),
        processed_by: None,
        processed_at: None,
        error_message: None,
        reclaim_count: 0,
    }
}

/// Load-bearing compile check: the consumer is constructible using only
/// engine-public + nebula-core + nebula-storage-port types.
///
/// This proves ADR-0008 decision 2 (no `nebula-api` / `nebula-storage`-row
/// types leak onto the consumer's public surface) — the `nebula-engine`
/// crate does not depend on `nebula-api`, so any such leak would have
/// failed to compile; this test makes the proof explicit.
#[tokio::test]
async fn control_consumer_public_surface_uses_only_allowed_types() {
    let queue: Arc<dyn ControlQueueRepo> = Arc::new(InMemoryControlQueueRepo::new());
    let dispatch: Arc<dyn ControlDispatch> = RecordingDispatch::new();

    let consumer = ControlConsumer::new(queue, dispatch, b"test-processor".to_vec())
        .with_batch_size(8)
        .with_poll_interval(Duration::from_millis(10));

    let shutdown = CancellationToken::new();
    let handle = consumer.spawn(shutdown.clone());
    shutdown.cancel();
    handle.await.expect("spawned task completed cleanly");
}

#[tokio::test]
async fn consumer_shuts_down_gracefully_on_cancel() {
    let queue: Arc<dyn ControlQueueRepo> = Arc::new(InMemoryControlQueueRepo::new());
    let dispatch: Arc<dyn ControlDispatch> = RecordingDispatch::new();

    let consumer = ControlConsumer::new(queue, dispatch, b"test-processor".to_vec())
        .with_poll_interval(Duration::from_millis(10));
    let shutdown = CancellationToken::new();
    let handle = consumer.spawn(shutdown.clone());

    // Let the loop run a few idle ticks so we exercise the claim-empty path.
    tokio::time::sleep(Duration::from_millis(40)).await;

    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("graceful shutdown within 1s")
        .expect("spawned task panic-free");
}

#[tokio::test]
async fn consumer_observes_each_command_variant_via_dispatch_trait() {
    let repo = Arc::new(InMemoryControlQueueRepo::new());
    let queue: Arc<dyn ControlQueueRepo> = repo.clone();
    let recorder = RecordingDispatch::new();
    let dispatch: Arc<dyn ControlDispatch> = recorder.clone();

    let exec_start = ExecutionId::new();
    let exec_cancel = ExecutionId::new();
    let exec_terminate = ExecutionId::new();
    let exec_resume = ExecutionId::new();
    let exec_restart = ExecutionId::new();

    repo.enqueue(&queue_entry(&exec_start, ControlCommand::Start, 0))
        .await
        .unwrap();
    repo.enqueue(&queue_entry(&exec_cancel, ControlCommand::Cancel, 1))
        .await
        .unwrap();
    repo.enqueue(&queue_entry(&exec_terminate, ControlCommand::Terminate, 2))
        .await
        .unwrap();
    repo.enqueue(&queue_entry(&exec_resume, ControlCommand::Resume, 3))
        .await
        .unwrap();
    repo.enqueue(&queue_entry(&exec_restart, ControlCommand::Restart, 4))
        .await
        .unwrap();

    let consumer = ControlConsumer::new(queue, dispatch, b"test-processor".to_vec())
        .with_batch_size(16)
        .with_poll_interval(Duration::from_millis(10));
    let shutdown = CancellationToken::new();
    let handle = consumer.spawn(shutdown.clone());

    // Wait for the consumer to observe all five rows.
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if recorder.snapshot().len() >= 5 {
                break;
            }
            recorder.notify.notified().await;
        }
    })
    .await
    .expect("all five commands observed within 2s");

    shutdown.cancel();
    handle.await.expect("graceful shutdown");

    let mut seen = recorder.snapshot();
    seen.sort_by_key(|(cmd, _)| cmd.as_str());
    assert_eq!(seen.len(), 5, "all commands observed exactly once");

    let has =
        |cmd: ControlCommand, id: ExecutionId| seen.iter().any(|(c, i)| *c == cmd && *i == id);
    assert!(has(ControlCommand::Start, exec_start), "Start observed");
    assert!(has(ControlCommand::Cancel, exec_cancel), "Cancel observed");
    assert!(
        has(ControlCommand::Terminate, exec_terminate),
        "Terminate observed"
    );
    assert!(has(ControlCommand::Resume, exec_resume), "Resume observed");
    assert!(
        has(ControlCommand::Restart, exec_restart),
        "Restart observed"
    );

    // Every row the consumer observed was acked via `mark_completed`:
    // a second `claim_pending` call from a fresh consumer returns nothing
    // pending. This is the A1 equivalent of "row is drained."
    let leftover = repo.claim_pending(b"fresh-processor", 16).await.unwrap();
    assert!(
        leftover.is_empty(),
        "all rows acked — claim_pending sees nothing pending"
    );
}

#[tokio::test]
async fn consumer_marks_row_failed_on_malformed_execution_id() {
    let repo = Arc::new(InMemoryControlQueueRepo::new());
    let queue: Arc<dyn ControlQueueRepo> = repo.clone();
    let dispatch: Arc<dyn ControlDispatch> = RecordingDispatch::new();

    let mut poison = queue_entry(&ExecutionId::new(), ControlCommand::Cancel, 9);
    poison.execution_id = b"not-a-ulid".to_vec();
    repo.enqueue(&poison).await.unwrap();

    let consumer = ControlConsumer::new(queue, dispatch, b"test-processor".to_vec())
        .with_poll_interval(Duration::from_millis(10));
    let shutdown = CancellationToken::new();
    let handle = consumer.spawn(shutdown.clone());

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let snap = repo.snapshot().await;
            if snap.iter().any(|e| e.status == "Failed") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("poison row marked Failed within 1s");

    shutdown.cancel();
    handle.await.expect("graceful shutdown");

    let snap = repo.snapshot().await;
    let poison_row = snap
        .iter()
        .find(|e| e.id == vec![9; 16])
        .expect("row present");
    assert_eq!(poison_row.status, "Failed");
    assert!(
        poison_row
            .error_message
            .as_deref()
            .is_some_and(|m| m.contains("malformed execution_id")),
        "error message explains why dispatch was rejected, got {:?}",
        poison_row.error_message
    );
}
