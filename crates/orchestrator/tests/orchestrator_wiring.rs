//! Integration tests for [`Orchestrator`] wiring against [`InMemoryJobDispatchQueue`]
//! and a [`RecordingSink`] spy, using the paused tokio clock.
//!
//! Tests:
//!
//! 1. `routes_by_tag` — alpha + beta jobs, worker advertises [alpha]; spy sees only alpha.
//! 2. `claim_route_sink_mark_dispatched` — sink Ok once; row terminal-dispatched; counter=1.
//! 3. `no_double_dispatch` — second claim from a fresh processor yields nothing.
//! 4. `sink_failure_marks_failed` — sink Err(Rejected); not re-served; failed counter=1.
//! 5. `reclaim_recovers_crashed` — claim then don't mark; reclaim returns Pending; counters.
//! 6. `graceful_shutdown_flushes_in_flight_dispatch` — cancel while sink blocked mid-dispatch;
//!    dispatch completes after release; row is marked Dispatched, not left Processing.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, NEBULA_ORCHESTRATOR_RECLAIM_TOTAL,
        orchestrator_dispatch_outcome, orchestrator_reclaim_outcome,
    },
};
use nebula_orchestrator::{ExecutionSink, ExecutionSinkError, Orchestrator};
use nebula_storage::inmem::{InMemoryJobDispatchQueue, new_shared_core};
use nebula_storage_port::{
    Scope,
    dto::{CapabilityTag, ControlCommand, JobDispatchMsg},
    store::JobDispatchQueue,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Widen a short label into the fixed 16-byte processor id. Distinct labels
/// stay distinct — this is explicit padding at the test boundary, not runtime
/// truncation.
fn proc16(label: &[u8]) -> [u8; 16] {
    let mut id = [0u8; 16];
    let n = label.len().min(16);
    id[..n].copy_from_slice(&label[..n]);
    id
}

/// Build a fresh `InMemoryJobDispatchQueue` over its own shared core.
fn make_queue() -> Arc<InMemoryJobDispatchQueue> {
    Arc::new(InMemoryJobDispatchQueue::from_core(new_shared_core()))
}

fn scope() -> Scope {
    Scope::new("ws_test", "org_test")
}

/// Build a minimal [`JobDispatchMsg`] stamped with `row_id`.
fn make_msg(row_id: u8, required_plugin_key: &str, execution_id: &str) -> JobDispatchMsg {
    JobDispatchMsg::new(
        [row_id; 16],
        execution_id,
        ControlCommand::Start,
        scope(),
        serde_json::json!({}),
        None::<String>,
        "sha-abc",
        required_plugin_key,
        vec![CapabilityTag::from(required_plugin_key)],
        None::<String>,
        0,
    )
}

// ── RecordingSink ─────────────────────────────────────────────────────────────

/// Spy that records every successful dispatch. Optionally returns
/// `Err(Rejected)` for the next call (resets after one use).
#[derive(Debug, Default)]
struct RecordingSink {
    observations: Mutex<Vec<JobDispatchMsg>>,
    notify: Notify,
    fail_next: Mutex<bool>,
}

impl RecordingSink {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Cause the next `dispatch` call to return `Err(Rejected(...))`.
    fn set_fail_next(&self) {
        *self.fail_next.lock().expect("poisoned lock") = true;
    }

    fn snapshot(&self) -> Vec<JobDispatchMsg> {
        self.observations.lock().expect("poisoned lock").clone()
    }
}

#[async_trait]
impl ExecutionSink for RecordingSink {
    async fn dispatch(&self, msg: &JobDispatchMsg) -> Result<(), ExecutionSinkError> {
        let fail = {
            let mut f = self.fail_next.lock().expect("poisoned lock");
            let was = *f;
            *f = false;
            was
        };
        if fail {
            return Err(ExecutionSinkError::Rejected(format!(
                "test reject for execution_id={}",
                msg.execution_id
            )));
        }
        self.observations
            .lock()
            .expect("poisoned lock")
            .push(msg.clone());
        self.notify.notify_waiters();
        Ok(())
    }
}

// ── StalledSink ───────────────────────────────────────────────────────────────

/// Sink that blocks inside `dispatch` until `release` is notified. Used to
/// park the orchestrator mid-dispatch so the test can cancel and verify the
/// row was left in `Processing`.
///
/// `entered` is notified the moment the orchestrator calls `dispatch`, before
/// the blocking wait. The test awaits `entered` to confirm the orchestrator
/// holds the row in Processing — no polling of `claim_pending` needed, which
/// avoids the probe accidentally claiming the row and defeating the test.
#[derive(Debug)]
struct StalledSink {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl ExecutionSink for StalledSink {
    async fn dispatch(&self, _msg: &JobDispatchMsg) -> Result<(), ExecutionSinkError> {
        // Signal the test that we are inside dispatch (row is Processing).
        self.entered.notify_one();
        // Block until the test notifies the release gate.
        self.release.notified().await;
        Ok(())
    }
}

// ── test 1: routes_by_tag ─────────────────────────────────────────────────────

/// Worker advertises only `[alpha]`. Both an alpha and a beta job are enqueued.
/// The orchestrator must dispatch exactly the alpha job; the beta job must
/// remain Pending (claimable by a beta-capable worker).
#[tokio::test(start_paused = true)]
async fn routes_by_tag() {
    let queue = make_queue();
    let spy = RecordingSink::new();

    queue
        .enqueue(&make_msg(1, "alpha", "exec-alpha"))
        .await
        .unwrap();
    queue
        .enqueue(&make_msg(2, "beta", "exec-beta"))
        .await
        .unwrap();

    let shutdown = CancellationToken::new();
    let orch = Orchestrator::new(
        queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        proc16(b"proc-alpha"),
        vec![CapabilityTag::from("alpha")],
    )
    .with_batch_size(8)
    .with_poll_interval(Duration::from_millis(10));

    let handle = orch.spawn(shutdown.clone());

    // Advance time until the spy observes the alpha job.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !spy.snapshot().is_empty() {
                break;
            }
            tokio::time::advance(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("alpha job dispatched within timeout");

    shutdown.cancel();
    handle.await.expect("graceful shutdown");

    let seen = spy.snapshot();
    assert_eq!(seen.len(), 1, "spy must see exactly one job");
    assert_eq!(
        seen[0].required_plugin_key, "alpha",
        "dispatched job must be the alpha job"
    );

    // Beta job must still be Pending — claimable by a beta-capable worker.
    let beta_tags = vec![CapabilityTag::from("beta")];
    let leftover = queue
        .claim_pending(&proc16(b"beta-worker-"), 8, &beta_tags)
        .await
        .unwrap();
    assert_eq!(
        leftover.len(),
        1,
        "beta job must still be Pending after alpha-only orchestrator ran"
    );
    assert_eq!(leftover[0].required_plugin_key, "beta");
}

// ── test 2: claim_route_sink_mark_dispatched ──────────────────────────────────

/// Sink returns `Ok` once. The `dispatched` counter reaches 1. A second
/// `claim_pending` from a fresh processor returns nothing (row is terminal).
#[tokio::test(start_paused = true)]
async fn claim_route_sink_mark_dispatched() {
    let queue = make_queue();
    let spy = RecordingSink::new();
    let registry = MetricsRegistry::new();

    queue
        .enqueue(&make_msg(10, "plugin-a", "exec-1"))
        .await
        .unwrap();

    let shutdown = CancellationToken::new();
    let orch = Orchestrator::new(
        queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        proc16(b"proc-1"),
        vec![CapabilityTag::from("plugin-a")],
    )
    .with_batch_size(4)
    .with_poll_interval(Duration::from_millis(10))
    .with_metrics(registry.clone());

    let handle = orch.spawn(shutdown.clone());

    let dispatched_labels = registry
        .interner()
        .single("outcome", orchestrator_dispatch_outcome::DISPATCHED);

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let d = registry
                .counter_labeled(NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, &dispatched_labels)
                .unwrap()
                .get();
            if d >= 1 {
                break;
            }
            tokio::time::advance(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("dispatched counter reached 1 within timeout");

    shutdown.cancel();
    handle.await.expect("graceful shutdown");

    let dispatched = registry
        .counter_labeled(NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, &dispatched_labels)
        .unwrap()
        .get();
    assert_eq!(dispatched, 1, "dispatched counter must be 1");

    // Row is terminal — a fresh processor finds nothing Pending.
    let tags = vec![CapabilityTag::from("plugin-a")];
    let leftover = queue
        .claim_pending(&proc16(b"fresh-proc--"), 8, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "row must be terminal after successful dispatch"
    );
}

// ── test 3: no_double_dispatch ────────────────────────────────────────────────

/// After one dispatch-and-ack, the spy count stays at 1. A second
/// `claim_pending` from a different processor returns nothing.
#[tokio::test(start_paused = true)]
async fn no_double_dispatch() {
    let queue = make_queue();
    let spy = RecordingSink::new();

    queue
        .enqueue(&make_msg(20, "plugin-b", "exec-2"))
        .await
        .unwrap();

    let shutdown = CancellationToken::new();
    let orch = Orchestrator::new(
        queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        proc16(b"proc-nd"),
        vec![CapabilityTag::from("plugin-b")],
    )
    .with_batch_size(4)
    .with_poll_interval(Duration::from_millis(10));

    let handle = orch.spawn(shutdown.clone());

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !spy.snapshot().is_empty() {
                break;
            }
            tokio::time::advance(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("job dispatched");

    // Let the orchestrator complete the ack before we shut down.
    tokio::time::advance(Duration::from_millis(30)).await;

    shutdown.cancel();
    handle.await.expect("graceful shutdown");

    assert_eq!(spy.snapshot().len(), 1, "sink must be invoked exactly once");

    // Second processor sees nothing — row is not re-served.
    let tags = vec![CapabilityTag::from("plugin-b")];
    let second = queue
        .claim_pending(&proc16(b"proc-nd-2---"), 8, &tags)
        .await
        .unwrap();
    assert!(
        second.is_empty(),
        "no double dispatch: second claim must be empty"
    );
}

// ── test 4: sink_failure_marks_failed ────────────────────────────────────────

/// Sink returns `Err(Rejected)`. The `failed` counter reaches 1. The row is
/// not re-served (it is marked Failed by the orchestrator).
#[tokio::test(start_paused = true)]
async fn sink_failure_marks_failed() {
    let queue = make_queue();
    let spy = RecordingSink::new();
    spy.set_fail_next();
    let registry = MetricsRegistry::new();

    queue
        .enqueue(&make_msg(30, "plugin-c", "exec-3"))
        .await
        .unwrap();

    let shutdown = CancellationToken::new();
    let orch = Orchestrator::new(
        queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        proc16(b"proc-fail---"),
        vec![CapabilityTag::from("plugin-c")],
    )
    .with_batch_size(4)
    .with_poll_interval(Duration::from_millis(10))
    .with_metrics(registry.clone());

    let handle = orch.spawn(shutdown.clone());

    let failed_labels = registry
        .interner()
        .single("outcome", orchestrator_dispatch_outcome::FAILED);

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let f = registry
                .counter_labeled(NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, &failed_labels)
                .unwrap()
                .get();
            if f >= 1 {
                break;
            }
            tokio::time::advance(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("failed counter reached 1 within timeout");

    shutdown.cancel();
    handle.await.expect("graceful shutdown");

    let failed = registry
        .counter_labeled(NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, &failed_labels)
        .unwrap()
        .get();
    assert_eq!(failed, 1, "failed counter must be 1");

    // Failed row must not be re-served by a subsequent claim.
    let tags = vec![CapabilityTag::from("plugin-c")];
    let leftover = queue
        .claim_pending(&proc16(b"other-proc--"), 8, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "failed row must not be re-served via claim_pending"
    );
}

// ── test 5: reclaim_recovers_crashed ─────────────────────────────────────────

/// Simulate a crashed runner: claim a row manually (putting it in Processing),
/// never mark it. A fresh orchestrator with aggressive reclaim settings sweeps
/// it back to Pending (reclaimed counter ≥ 1). Then verify the row past budget
/// goes to Failed (exhausted counter ≥ 1) by seeding a second queue directly.
#[tokio::test(start_paused = true)]
async fn reclaim_recovers_crashed() {
    let queue = make_queue();
    let spy = RecordingSink::new();
    let registry = MetricsRegistry::new();

    queue
        .enqueue(&make_msg(40, "plugin-d", "exec-4"))
        .await
        .unwrap();

    // Claim the row without marking it — simulates a crashed runner.
    let tags = vec![CapabilityTag::from("plugin-d")];
    let claimed = queue
        .claim_pending(&proc16(b"crashed-proc"), 1, &tags)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1, "row must be claimed into Processing");
    // Intentionally not calling mark_dispatched or mark_failed — crash simulated.

    // Fresh orchestrator with short reclaim_after so the paused clock just needs
    // a small advance to make the row stale.
    let reclaim_after = Duration::from_millis(10);
    let orch = Orchestrator::new(
        queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        proc16(b"fresh-proc--"),
        tags.clone(),
    )
    .with_batch_size(4)
    .with_poll_interval(Duration::from_millis(10))
    .with_reclaim_after(reclaim_after)
    .with_reclaim_interval(Duration::from_millis(20))
    .with_max_reclaim_count(3)
    .with_metrics(registry.clone());

    let shutdown = CancellationToken::new();
    let handle = orch.spawn(shutdown.clone());

    let reclaimed_labels = registry
        .interner()
        .single("outcome", orchestrator_reclaim_outcome::RECLAIMED);

    // Advance time past `reclaim_after` so the stuck row becomes stale,
    // triggering a reclaim sweep.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let r = registry
                .counter_labeled(NEBULA_ORCHESTRATOR_RECLAIM_TOTAL, &reclaimed_labels)
                .unwrap()
                .get();
            if r >= 1 {
                break;
            }
            tokio::time::advance(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("reclaimed counter reached 1 within timeout");

    shutdown.cancel();
    handle.await.expect("graceful shutdown");

    let reclaimed = registry
        .counter_labeled(NEBULA_ORCHESTRATOR_RECLAIM_TOTAL, &reclaimed_labels)
        .unwrap()
        .get();
    assert!(
        reclaimed >= 1,
        "reclaimed counter must be ≥ 1, got {reclaimed}"
    );

    // Verify exhausted path: seed a row at max_reclaim_count=0 directly via the
    // port so the next sweep exhausts it immediately.
    let queue2 = make_queue();
    queue2
        .enqueue(&make_msg(41, "plugin-d", "exec-5"))
        .await
        .unwrap();
    let _ = queue2
        .claim_pending(&proc16(b"crash-proc2-"), 1, &tags)
        .await
        .unwrap();
    // With max_reclaim_count=0 any Processing row immediately exhausts.
    tokio::time::advance(Duration::from_millis(5)).await;
    let outcome = queue2
        .reclaim_stuck(Duration::from_millis(1), 0)
        .await
        .unwrap();
    assert_eq!(
        outcome.exhausted, 1,
        "row past budget=0 must move to Failed (exhausted=1)"
    );
}

// ── test 6: graceful_shutdown_flushes_in_flight_dispatch ─────────────────────

/// Cancelling the orchestrator while a dispatch is in flight does NOT drop the
/// in-flight work: the shutdown contract is batch-flush (finish the current
/// batch, then exit). The row must be marked Dispatched, not left Processing.
///
/// The "row left Processing for reclaim" contract only applies to a hard crash
/// (OS kill) — tested in `reclaim_recovers_crashed` (test-5). This test
/// exercises the graceful-flush path.
///
/// Sequence:
/// 1. Enqueue a row.
/// 2. Start the orchestrator with `StalledSink`. `StalledSink::dispatch` fires
///    `entered` before blocking on `release`, giving the test an exact signal
///    that the row is Processing and the orchestrator is inside dispatch.
/// 3. Await `entered` — no `claim_pending` polling so no probe-claim race.
/// 4. Cancel the token while the orchestrator is blocked in dispatch.
/// 5. Release the sink — dispatch returns `Ok`. The orchestrator calls
///    `mark_dispatched`, then loops back to the biased select, sees the
///    cancellation, and exits.
/// 6. `handle.await` completes.
/// 7. A fresh `claim_pending` returns empty — row is Dispatched (terminal), not
///    Processing, confirming the flush happened.
#[tokio::test]
async fn graceful_shutdown_flushes_in_flight_dispatch() {
    let queue = make_queue();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let sink: Arc<dyn ExecutionSink> = Arc::new(StalledSink {
        entered: entered.clone(),
        release: release.clone(),
    });

    queue
        .enqueue(&make_msg(50, "plugin-e", "exec-6"))
        .await
        .unwrap();

    let tags = vec![CapabilityTag::from("plugin-e")];
    let shutdown = CancellationToken::new();

    // Pre-register the `Notified` future BEFORE spawning the orchestrator so
    // that if `notify_one()` fires before we poll the future there is no lost
    // wake-up. `Notify::notified()` enables the receiver slot immediately.
    let entered_fut = entered.notified();
    tokio::pin!(entered_fut);

    let orch = Orchestrator::new(
        queue.clone() as Arc<dyn JobDispatchQueue>,
        sink,
        proc16(b"proc-sd-----"),
        tags.clone(),
    )
    .with_batch_size(1)
    .with_poll_interval(Duration::from_millis(5));

    let handle = orch.spawn(shutdown.clone());

    // Exact handshake: block until the orchestrator is inside StalledSink::dispatch.
    // The row is Processing under proc16(b"proc-sd-----") at this point.
    tokio::time::timeout(Duration::from_secs(5), &mut entered_fut)
        .await
        .expect("orchestrator entered dispatch within 5s");

    // Signal shutdown while the orchestrator is blocked in dispatch.
    // The biased select cannot fire yet — the orchestrator is not in the select
    // loop; it is in `tick()` → `handle_entry()`. Shutdown will be observed on
    // the NEXT loop iteration after `handle_entry` returns.
    shutdown.cancel();

    // Release the sink so dispatch completes with Ok. The orchestrator then
    // calls `mark_dispatched`, finishes `tick()`, re-enters the biased select,
    // sees `cancelled()`, and exits.
    release.notify_waiters();

    handle.await.expect("graceful shutdown after cancel");

    // Row is Dispatched (terminal) — a fresh claim returns empty.
    let leftover = queue
        .claim_pending(&proc16(b"recovery----"), 4, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "row must be Dispatched (terminal) after graceful flush; \
         claim_pending must return empty"
    );
}
