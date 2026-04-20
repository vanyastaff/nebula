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
use nebula_metrics::{
    MetricsRegistry,
    naming::{NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL, control_reclaim_outcome},
};
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

/// Dispatch that pretends to crash mid-handling: the first invocation per
/// `execution_id` blocks forever (simulating a runner that got stuck), the
/// second and subsequent invocations complete Ok. Pairs with a consumer
/// whose future is dropped to simulate the "runner never acked" crash.
#[derive(Default)]
struct FlakyDispatch {
    first_seen: Mutex<std::collections::HashSet<Vec<u8>>>,
    observations: Mutex<Vec<(ControlCommand, ExecutionId)>>,
    notify: Notify,
}

impl FlakyDispatch {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    async fn maybe_stall(&self, id: &ExecutionId) {
        let key = id.to_string().into_bytes();
        let first_time = {
            let mut set = self.first_seen.lock().expect("poisoned");
            set.insert(key)
        };
        if first_time {
            // Simulate a stall long enough that the reclaim test can time
            // out the consumer via drop. We do not call `sleep` forever —
            // the caller drops the future, so we just yield indefinitely.
            std::future::pending::<()>().await;
            unreachable!("stall future dropped by test");
        }
    }

    fn snapshot(&self) -> Vec<(ControlCommand, ExecutionId)> {
        self.observations.lock().expect("poisoned").clone()
    }

    fn record(&self, cmd: ControlCommand, id: ExecutionId) {
        self.observations.lock().expect("poisoned").push((cmd, id));
        self.notify.notify_waiters();
    }
}

#[async_trait]
impl ControlDispatch for FlakyDispatch {
    async fn dispatch_start(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError> {
        self.maybe_stall(&execution_id).await;
        self.record(ControlCommand::Start, execution_id);
        Ok(())
    }

    async fn dispatch_cancel(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError> {
        self.maybe_stall(&execution_id).await;
        self.record(ControlCommand::Cancel, execution_id);
        Ok(())
    }

    async fn dispatch_terminate(
        &self,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        self.maybe_stall(&execution_id).await;
        self.record(ControlCommand::Terminate, execution_id);
        Ok(())
    }

    async fn dispatch_resume(&self, execution_id: ExecutionId) -> Result<(), ControlDispatchError> {
        self.maybe_stall(&execution_id).await;
        self.record(ControlCommand::Resume, execution_id);
        Ok(())
    }

    async fn dispatch_restart(
        &self,
        execution_id: ExecutionId,
    ) -> Result<(), ControlDispatchError> {
        self.maybe_stall(&execution_id).await;
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

/// End-to-end: simulate a consumer whose dispatch stalls forever on the
/// first attempt, drop it (leaving the row in `Processing`), advance
/// through a reclaim sweep, then spin up a fresh consumer and verify it
/// picks up the redelivered row and drives it to `Completed`.
///
/// This is the B1 acceptance test — ADR-0008 §5 liveness guarantee.
#[tokio::test]
async fn reclaim_sweep_recovers_orphaned_processing_row_end_to_end() {
    let repo = Arc::new(InMemoryControlQueueRepo::new());
    let queue: Arc<dyn ControlQueueRepo> = repo.clone();
    let dispatch_flaky = FlakyDispatch::new();
    let dispatch1: Arc<dyn ControlDispatch> = dispatch_flaky.clone();

    let exec = ExecutionId::new();
    repo.enqueue(&queue_entry(&exec, ControlCommand::Cancel, 42))
        .await
        .unwrap();

    // Consumer #1 — claims the row, stalls in dispatch, never acks. Use an
    // aggressive reclaim_after (50ms) + reclaim_interval (30ms) so the test
    // runs in well under a second. Chrono is wall-clock; tokio time-pause
    // would not advance it — honest short sleeps are the answer here.
    let consumer1 = ControlConsumer::new(queue.clone(), dispatch1, b"runner-one".to_vec())
        .with_batch_size(4)
        .with_poll_interval(Duration::from_millis(5))
        .with_reclaim_after(Duration::from_millis(50))
        .with_reclaim_interval(Duration::from_millis(30))
        .with_max_reclaim_count(3);
    let shutdown1 = CancellationToken::new();
    let handle1 = consumer1.spawn(shutdown1.clone());

    // Wait for the row to be claimed by consumer #1 (Pending → Processing).
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let snap = repo.snapshot().await;
            if snap
                .iter()
                .any(|e| e.id == vec![42u8; 16] && e.status == "Processing")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("row claimed within 1s");

    // Simulate the crash: aborting the join handle drops the dispatch
    // future — exactly the "runner process died" shape. The dispatch
    // future is stalled inside `std::future::pending` so the ack never
    // happens.
    handle1.abort();
    let _ = handle1.await;

    // Confirm the row is stuck in Processing right after the crash.
    let snap_after_crash = repo.snapshot().await;
    let stuck = snap_after_crash
        .iter()
        .find(|e| e.id == vec![42u8; 16])
        .expect("row present");
    assert_eq!(
        stuck.status, "Processing",
        "orphaned in Processing post-crash"
    );
    assert_eq!(stuck.reclaim_count, 0, "no reclaim yet");

    // Sleep past the reclaim_after window so the next sweep finds it stale.
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Consumer #2 — clean runner. Its reclaim tick will sweep the stuck row
    // back to Pending on startup; then its claim loop picks it up and the
    // non-flaky second-dispatch path returns Ok, which acks the row Completed.
    let dispatch_fresh: Arc<dyn ControlDispatch> = dispatch_flaky.clone();
    let consumer2 = ControlConsumer::new(queue.clone(), dispatch_fresh, b"runner-two".to_vec())
        .with_batch_size(4)
        .with_poll_interval(Duration::from_millis(5))
        .with_reclaim_after(Duration::from_millis(50))
        .with_reclaim_interval(Duration::from_millis(30))
        .with_max_reclaim_count(3);
    let shutdown2 = CancellationToken::new();
    let handle2 = consumer2.spawn(shutdown2.clone());

    // Wait for the row to finish its second life — reclaim → reclaim bump →
    // claim → dispatch (non-flaky second call) → mark_completed.
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let snap = repo.snapshot().await;
            if snap
                .iter()
                .any(|e| e.id == vec![42u8; 16] && e.status == "Completed")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("row reclaimed and completed within 3s");

    shutdown2.cancel();
    handle2.await.expect("graceful shutdown");

    // Post-conditions:
    let final_snap = repo.snapshot().await;
    let row = final_snap
        .iter()
        .find(|e| e.id == vec![42u8; 16])
        .expect("row still present");
    assert_eq!(row.status, "Completed", "drove to Completed after reclaim");
    assert_eq!(row.reclaim_count, 1, "reclaimed exactly once");

    // The dispatch observed exactly one successful Cancel (second call).
    let observed = dispatch_flaky.snapshot();
    let cancels_for_exec: Vec<_> = observed
        .iter()
        .filter(|(cmd, id)| *cmd == ControlCommand::Cancel && *id == exec)
        .collect();
    assert_eq!(
        cancels_for_exec.len(),
        1,
        "exactly one successful dispatch recorded (first stalled), got {observed:?}"
    );
}

/// ADR-0017 Seam: every reclaim sweep must increment
/// `nebula_engine_control_reclaim_total{outcome}` by the per-row count, so a
/// non-zero `exhausted` is alertable and a steady `reclaimed` flags
/// crashed-runner load. Pre-seeds two stale-but-budgeted rows + one stale
/// past-budget row, runs one sweep, and asserts both label values bump
/// exactly once per row (not once per sweep).
#[tokio::test]
async fn reclaim_sweep_emits_counter_metric_per_outcome() {
    let repo = Arc::new(InMemoryControlQueueRepo::new());
    let queue: Arc<dyn ControlQueueRepo> = repo.clone();

    // 600s in the past is far past the 50ms reclaim window we'll configure
    // below — every row qualifies as stuck on the first sweep.
    let stale = chrono::Utc::now() - chrono::Duration::seconds(600);
    let mk_row = |row_id: u8, exec: &ExecutionId, reclaim_count: u32| ControlQueueEntry {
        id: vec![row_id; 16],
        execution_id: exec.to_string().into_bytes(),
        command: ControlCommand::Cancel,
        issued_by: None,
        issued_at: chrono::Utc::now(),
        status: "Processing".to_string(),
        processed_by: Some(b"dead-runner".to_vec()),
        processed_at: Some(stale),
        error_message: None,
        reclaim_count,
    };
    let exec_a = ExecutionId::new();
    let exec_b = ExecutionId::new();
    let exec_c = ExecutionId::new();
    repo.enqueue(&mk_row(1, &exec_a, 0)).await.unwrap();
    repo.enqueue(&mk_row(2, &exec_b, 0)).await.unwrap();
    // reclaim_count == max_reclaim_count → moves to Failed, not requeued.
    repo.enqueue(&mk_row(3, &exec_c, 3)).await.unwrap();

    let registry = MetricsRegistry::new();
    let dispatch: Arc<dyn ControlDispatch> = RecordingDispatch::new();
    let consumer = ControlConsumer::new(queue.clone(), dispatch, b"runner-test".to_vec())
        .with_batch_size(8)
        .with_poll_interval(Duration::from_millis(20))
        .with_reclaim_after(Duration::from_millis(50))
        .with_reclaim_interval(Duration::from_millis(30))
        .with_max_reclaim_count(3)
        .with_metrics(registry.clone());

    let shutdown = CancellationToken::new();
    let handle = consumer.spawn(shutdown.clone());

    let reclaimed_labels = registry
        .interner()
        .single("outcome", control_reclaim_outcome::RECLAIMED);
    let exhausted_labels = registry
        .interner()
        .single("outcome", control_reclaim_outcome::EXHAUSTED);

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let r = registry
                .counter_labeled(NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL, &reclaimed_labels)
                .get();
            let e = registry
                .counter_labeled(NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL, &exhausted_labels)
                .get();
            if r >= 2 && e >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("counters reached expected values within 2s");

    shutdown.cancel();
    handle.await.expect("graceful shutdown");

    // Once the two reclaimed rows are picked up by the claim arm and
    // acked Completed, and the exhausted row sits in Failed, no further
    // sweep finds work — counters stay at 2 and 1 (per-row, not per-sweep).
    let reclaimed = registry
        .counter_labeled(NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL, &reclaimed_labels)
        .get();
    let exhausted = registry
        .counter_labeled(NEBULA_ENGINE_CONTROL_RECLAIM_TOTAL, &exhausted_labels)
        .get();
    assert_eq!(reclaimed, 2, "reclaimed counter bumps by row count");
    assert_eq!(exhausted, 1, "exhausted counter bumps by row count");
}
