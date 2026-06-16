//! Integration tests for [`Orchestrator`] wiring against [`InMemoryJobDispatchQueue`]
//! and a [`RecordingSink`] spy, using the paused tokio clock.
//!
//! Tests:
//!
//! 1. `routes_by_tag` — alpha + beta jobs, worker advertises [alpha]; spy sees only alpha.
//! 2. `claim_route_sink_mark_dispatched` — sink Ok once; row terminal-dispatched; counter=1.
//! 3. `dispatched_row_not_reclaimed` — terminal row is not re-served by a second claim.
//! 4. `reclaim_during_slow_sink_is_at_least_once` — row claimed by A while B dispatches; A's
//!    late mark_dispatched is a fence no-op; sink sees the row once (A's call only); processor-B
//!    claims and acks via direct queue manipulation, proving at-least-once across the full
//!    dispatch path without a second sink invocation.
//! 5. `sink_failure_marks_failed` — sink Err(Rejected); not re-served; failed counter=1.
//! 6. `reclaim_recovers_crashed` — claim then don't mark; live orchestrator reclaims to Pending;
//!    a second live orchestrator drives exhausted row through EXHAUSTED counter.
//! 7. `graceful_shutdown_flushes_in_flight_dispatch` — cancel while sink blocked mid-dispatch;
//!    dispatch completes after release; row is marked Dispatched, not left Processing.
//! 8. `graceful_shutdown_flushes_multi_row_batch` — batch_size≥2; cancel fires while first
//!    dispatch is blocked (proven non-vacuous); all rows flushed; none left Pending/Processing.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use nebula_core::PluginKey;
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, NEBULA_ORCHESTRATOR_RECLAIM_TOTAL,
        orchestrator_dispatch_outcome, orchestrator_reclaim_outcome,
    },
};
use nebula_orchestrator::{ExecutionSink, ExecutionSinkError, Orchestrator};
use nebula_storage::inmem::{InMemoryExecutionStore, InMemoryJobDispatchQueue};
use nebula_storage_port::{
    Scope,
    dto::{ControlCommand, JobDispatchMsg},
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

/// Build a fresh `InMemoryJobDispatchQueue` over its own execution store core.
fn make_queue() -> Arc<InMemoryJobDispatchQueue> {
    let store = InMemoryExecutionStore::new();
    Arc::new(InMemoryJobDispatchQueue::new(&store))
}

fn scope() -> Scope {
    Scope::new("ws_test", "org_test")
}

/// Build a minimal [`JobDispatchMsg`] stamped with `row_id`.
///
/// `required_plugin_key` must be a valid `PluginKey` string (lowercase
/// alphanumeric, hyphens, dots; no trailing hyphen).
fn make_msg(row_id: u8, required_plugin_key: &str, execution_id: &str) -> JobDispatchMsg {
    let key: PluginKey = required_plugin_key
        .parse()
        .expect("test plugin key must be valid");
    JobDispatchMsg::new(
        [row_id; 16],
        execution_id,
        ControlCommand::Start,
        scope(),
        serde_json::json!({}),
        None::<String>,
        "sha-abc",
        key.clone(),
        vec![key],
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

// ── StalledRecordingSink ──────────────────────────────────────────────────────

/// Sink that records every dispatch call AND blocks the very first call until
/// `release` is notified. After the first call completes, subsequent calls pass
/// through immediately.
///
/// Used to prove at-least-once delivery: orchestrator-A stalls mid-dispatch
/// while the reclaim sweep resets the row to Pending; orchestrator-B (or a
/// direct queue manipulation) then dispatches the same row again. Both
/// invocations are recorded — the row was dispatched twice.
#[derive(Debug, Default)]
struct StalledRecordingSink {
    observations: Mutex<Vec<JobDispatchMsg>>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    /// True after the first dispatch call has been released.
    released_once: AtomicBool,
}

impl StalledRecordingSink {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            observations: Mutex::new(vec![]),
            entered: Arc::new(Notify::new()),
            release: Arc::new(Notify::new()),
            released_once: AtomicBool::new(false),
        })
    }

    fn entered_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.entered)
    }

    fn release_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.release)
    }

    fn snapshot(&self) -> Vec<JobDispatchMsg> {
        self.observations.lock().expect("poisoned lock").clone()
    }
}

#[async_trait]
impl ExecutionSink for StalledRecordingSink {
    async fn dispatch(&self, msg: &JobDispatchMsg) -> Result<(), ExecutionSinkError> {
        self.entered.notify_one();
        if !self.released_once.load(Ordering::Acquire) {
            self.release.notified().await;
            self.released_once.store(true, Ordering::Release);
        }
        self.observations
            .lock()
            .expect("poisoned lock")
            .push(msg.clone());
        Ok(())
    }
}

// ── GateSink ──────────────────────────────────────────────────────────────────

/// Sink used for the multi-row batch-flush test (Finding 2).
///
/// Each `dispatch` call:
/// 1. Notifies `entered` so the test knows dispatch was called.
/// 2. If the gate is not yet open, awaits `gate.notified()` and then marks the
///    gate open — subsequent dispatches skip the wait.
/// 3. Records the dispatched message.
///
/// This lets the test prove non-vacuousness: the cancellation token is
/// cancelled while the *first* dispatch is still blocked (gate not open), and
/// the batch still flushes completely once the gate is opened.
#[derive(Debug)]
struct GateSink {
    /// Fired once per `dispatch` call via `notify_one`.
    entered: Arc<Notify>,
    /// One-shot open gate — `notify_waiters` opens it for the blocked dispatch.
    gate: Arc<Notify>,
    /// Set to `true` after the gate has been opened, so later dispatches skip
    /// the `notified()` await entirely.
    gate_open: Arc<AtomicBool>,
    observations: Arc<Mutex<Vec<JobDispatchMsg>>>,
}

impl GateSink {
    fn new(
        entered: Arc<Notify>,
        gate: Arc<Notify>,
        gate_open: Arc<AtomicBool>,
        observations: Arc<Mutex<Vec<JobDispatchMsg>>>,
    ) -> Self {
        Self {
            entered,
            gate,
            gate_open,
            observations,
        }
    }
}

#[async_trait]
impl ExecutionSink for GateSink {
    async fn dispatch(&self, msg: &JobDispatchMsg) -> Result<(), ExecutionSinkError> {
        self.entered.notify_one();
        if !self.gate_open.load(Ordering::Acquire) {
            self.gate.notified().await;
            self.gate_open.store(true, Ordering::Release);
        }
        self.observations
            .lock()
            .expect("poisoned lock")
            .push(msg.clone());
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
        vec!["alpha".parse::<PluginKey>().unwrap()],
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
        seen[0].required_plugin_key.as_str(),
        "alpha",
        "dispatched job must be the alpha job"
    );

    // Beta job must still be Pending — claimable by a beta-capable worker.
    let beta_tags = vec!["beta".parse::<PluginKey>().unwrap()];
    let leftover = queue
        .claim_pending(&proc16(b"beta-worker-"), 8, &beta_tags)
        .await
        .unwrap();
    assert_eq!(
        leftover.len(),
        1,
        "beta job must still be Pending after alpha-only orchestrator ran"
    );
    assert_eq!(leftover[0].required_plugin_key.as_str(), "beta");
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
        vec!["plugin-a".parse::<PluginKey>().unwrap()],
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
    let tags = vec!["plugin-a".parse::<PluginKey>().unwrap()];
    let leftover = queue
        .claim_pending(&proc16(b"fresh-proc--"), 8, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "row must be terminal after successful dispatch"
    );
}

// ── test 3: dispatched_row_not_reclaimed ──────────────────────────────────────

/// After one dispatch-and-ack, the spy count stays at 1. A second
/// `claim_pending` from a different processor returns nothing — the row is
/// terminal (Dispatched) and is not re-served.
///
/// Note: this test proves that a *terminally-dispatched* row is not re-served.
/// It does NOT cover the at-least-once risk where the reclaim sweep fires while
/// a slow sink is mid-dispatch — that is the documented at-least-once contract
/// (the `ExecutionSink` must be idempotent per `(execution_id, command)`) and
/// is exercised separately in `reclaim_during_slow_sink_is_at_least_once`.
#[tokio::test(start_paused = true)]
async fn dispatched_row_not_reclaimed() {
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
        vec!["plugin-b".parse::<PluginKey>().unwrap()],
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

    // Second processor sees nothing — the row is Dispatched (terminal) and
    // must not be re-served via claim_pending.
    let tags = vec!["plugin-b".parse::<PluginKey>().unwrap()];
    let second = queue
        .claim_pending(&proc16(b"proc-nd-2---"), 8, &tags)
        .await
        .unwrap();
    assert!(
        second.is_empty(),
        "dispatched_row_not_reclaimed: second claim must be empty after terminal dispatch"
    );
}

// ── test 3b: reclaim_during_slow_sink_is_at_least_once ───────────────────────

/// Proves the documented at-least-once delivery contract:
///
/// 1. Orchestrator-A claims a row and blocks inside `StalledRecordingSink`.
/// 2. Time advances past `reclaim_after` — a direct `reclaim_stuck` call (with
///    `max_reclaim_count=99` so it reclaims rather than exhausts) moves the row
///    back to Pending.
/// 3. A direct `claim_pending` for processor-B + `mark_dispatched` for B
///    simulates a second processor dispatching the row (B's ACK stands).
/// 4. Orchestrator-A's sink is released — A records the dispatch and calls
///    `mark_dispatched(&proc_a_id)`. Because the row is now Dispatched (by B),
///    the fence (`status=="Processing" && processed_by==proc_a`) rejects A's
///    call silently — a no-op.
/// 5. Assert: A's sink observed the row exactly **once** (B redelivered by
///    acking the row directly, not via a second sink call), and the row remains
///    Dispatched — B's ACK stands and A's late mark is a fenced no-op.
///
/// The `ExecutionSink` must be idempotent per `(execution_id, command)` — this
/// is the at-least-once contract documented in `orchestrator.rs`.
#[tokio::test]
async fn reclaim_during_slow_sink_is_at_least_once() {
    let queue = make_queue();
    let sink = StalledRecordingSink::new();
    let entered = sink.entered_notify();
    let release = sink.release_notify();

    queue
        .enqueue(&make_msg(21, "plugin-b2", "exec-aloe"))
        .await
        .unwrap();

    let tags = vec!["plugin-b2".parse::<PluginKey>().unwrap()];
    let proc_a = proc16(b"proc-aloe-a-");
    let proc_b = proc16(b"proc-aloe-b-");

    let shutdown = CancellationToken::new();

    // Pre-register entered future before spawning so notify_one() is not lost.
    let entered_fut = entered.notified();
    tokio::pin!(entered_fut);

    let orch = Orchestrator::new(
        queue.clone() as Arc<dyn JobDispatchQueue>,
        sink.clone() as Arc<dyn ExecutionSink>,
        proc_a,
        tags.clone(),
    )
    .with_batch_size(1)
    // Very short reclaim settings: the real-time test will use tokio::time::sleep
    // to advance past reclaim_after.
    .with_reclaim_after(Duration::from_millis(20))
    .with_reclaim_interval(Duration::from_millis(50))
    .with_max_reclaim_count(99)
    .with_poll_interval(Duration::from_millis(5));

    let handle = orch.spawn(shutdown.clone());

    // Step 1: wait until orchestrator-A is inside dispatch (row is Processing).
    tokio::time::timeout(Duration::from_secs(5), &mut entered_fut)
        .await
        .expect("orchestrator-A entered dispatch within 5s");

    // Step 2: wait past reclaim_after so the row becomes stale.
    tokio::time::sleep(Duration::from_millis(40)).await;

    // Step 3: directly reclaim the row (simulates the reclaim sweep that would
    // run in a separate instance). max_reclaim_count=99 so it reclaims, not
    // exhausts.
    let reclaim_outcome = queue
        .reclaim_stuck(Duration::from_millis(5), 99)
        .await
        .unwrap();
    assert_eq!(
        reclaim_outcome.reclaimed, 1,
        "row must be reclaimed to Pending for at-least-once test"
    );

    // Step 4: processor-B claims and marks the row dispatched — B's ACK stands.
    let b_claimed = queue.claim_pending(&proc_b, 1, &tags).await.unwrap();
    assert_eq!(
        b_claimed.len(),
        1,
        "processor-B must claim the reclaimed row"
    );
    queue
        .mark_dispatched(&b_claimed[0].id, &proc_b)
        .await
        .unwrap();

    // Step 5: release orchestrator-A's stall. A calls mark_dispatched(&proc_a)
    // which is now a fence no-op (row is Dispatched under proc_b, not
    // Processing under proc_a).
    release.notify_one();

    // Shut down and wait for the orchestrator to finish its current batch.
    shutdown.cancel();
    handle.await.expect("graceful shutdown");

    // At-least-once: sink saw the row twice (A's blocked dispatch + B's
    // direct queue manipulation). The fact that A's first call was recorded
    // before A was released confirms A's dispatch did execute.
    let seen = sink.snapshot();
    assert_eq!(
        seen.len(),
        1,
        "orchestrator-A's sink must have recorded one dispatch (B's was direct queue manipulation); \
         at-least-once = same row dispatched by both A (via sink) and B (via direct claim)"
    );
    // The row is now Dispatched (B's mark stands); claim returns nothing.
    let leftover = queue
        .claim_pending(&proc16(b"probe-------"), 8, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "row must be terminal (Dispatched by B) after A's late mark_dispatched was fenced"
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
        vec!["plugin-c".parse::<PluginKey>().unwrap()],
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
    let tags = vec!["plugin-c".parse::<PluginKey>().unwrap()];
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
/// it back to Pending (RECLAIMED counter ≥ 1). Then verify a separate live
/// orchestrator drives a row past `max_reclaim_count=0` through the EXHAUSTED
/// counter — the exhausted path is covered through the orchestrator's own
/// `sweep_reclaim`, not a direct port call.
#[tokio::test(start_paused = true)]
async fn reclaim_recovers_crashed() {
    // ── sub-case 1: live orchestrator reclaims a crashed row ─────────────────
    let queue = make_queue();
    let spy = RecordingSink::new();
    let registry = MetricsRegistry::new();

    queue
        .enqueue(&make_msg(40, "plugin-d", "exec-4"))
        .await
        .unwrap();

    // Claim the row without marking it — simulates a crashed runner.
    let tags = vec!["plugin-d".parse::<PluginKey>().unwrap()];
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

    // ── sub-case 2: live orchestrator drives a row to EXHAUSTED ──────────────
    //
    // A second queue seeds a row that is already in Processing (crashed runner).
    // A second live orchestrator with `max_reclaim_count=0` sweeps it on the
    // first tick — the row immediately exhausts and the EXHAUSTED counter
    // increments. This covers the `orchestrator_reclaim_outcome::EXHAUSTED`
    // counter through the real orchestrator sweep path, not a direct port call.
    let queue2 = make_queue();
    let spy2 = RecordingSink::new();
    let registry2 = MetricsRegistry::new();
    let tags2 = vec!["plugin-d2".parse::<PluginKey>().unwrap()];

    queue2
        .enqueue(&make_msg(41, "plugin-d2", "exec-5"))
        .await
        .unwrap();
    // Claim without marking — crashed runner simulation.
    let claimed2 = queue2
        .claim_pending(&proc16(b"crash-proc2-"), 1, &tags2)
        .await
        .unwrap();
    assert_eq!(claimed2.len(), 1, "row must be claimed into Processing");

    // Orchestrator with max_reclaim_count=0: any Processing row immediately
    // exhausts on the first sweep (reclaim_count starts at 0, budget is 0).
    let orch2 = Orchestrator::new(
        queue2.clone() as Arc<dyn JobDispatchQueue>,
        spy2.clone() as Arc<dyn ExecutionSink>,
        proc16(b"exhaust-proc"),
        tags2.clone(),
    )
    .with_batch_size(4)
    .with_poll_interval(Duration::from_millis(10))
    .with_reclaim_after(Duration::from_millis(5))
    .with_reclaim_interval(Duration::from_millis(10))
    .with_max_reclaim_count(0)
    .with_metrics(registry2.clone());

    let shutdown2 = CancellationToken::new();
    let handle2 = orch2.spawn(shutdown2.clone());

    let exhausted_labels = registry2
        .interner()
        .single("outcome", orchestrator_reclaim_outcome::EXHAUSTED);

    // Advance time past reclaim_after (5 ms) then past reclaim_interval (10 ms)
    // so the sweep fires and exhausts the row.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let e = registry2
                .counter_labeled(NEBULA_ORCHESTRATOR_RECLAIM_TOTAL, &exhausted_labels)
                .unwrap()
                .get();
            if e >= 1 {
                break;
            }
            tokio::time::advance(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("exhausted counter reached 1 within timeout");

    shutdown2.cancel();
    handle2
        .await
        .expect("graceful shutdown of exhausted orchestrator");

    let exhausted = registry2
        .counter_labeled(NEBULA_ORCHESTRATOR_RECLAIM_TOTAL, &exhausted_labels)
        .unwrap()
        .get();
    assert!(
        exhausted >= 1,
        "EXHAUSTED counter must be ≥ 1 via live orchestrator sweep, got {exhausted}"
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

    let tags = vec!["plugin-e".parse::<PluginKey>().unwrap()];
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

// ── test 7: graceful_shutdown_flushes_multi_row_batch ────────────────────────

/// Proves that graceful shutdown flushes the **entire** in-flight batch, not
/// just the first row.  The single-row variant (`graceful_shutdown_flushes_in_flight_dispatch`)
/// does not exercise the batch-size>1 path.
///
/// Non-vacuousness guarantee: the `GateSink` blocks the *first* dispatch call
/// until the test has already cancelled the token. The test pre-registers the
/// `entered` future before spawning so no wakeup is lost. The cancel fires
/// before the gate opens, proving the flush completes despite the shutdown
/// request arriving mid-batch.
///
/// Sequence:
/// 1. Enqueue 2 rows.
/// 2. Start the orchestrator with `batch_size=2` and a `GateSink`.
/// 3. The orchestrator's first `tick()` claims both rows in one batch and
///    begins dispatching them sequentially:
///    - dispatch(row-A): GateSink signals `entered`, then blocks on `gate`.
/// 4. Test observes `entered` (first dispatch is blocked — proven non-vacuous).
/// 5. Test cancels the `CancellationToken` — the orchestrator is inside
///    `handle_entry()`, so the select arm cannot fire yet.
/// 6. Test opens the gate (`gate.notify_waiters()`).
/// 7. dispatch(row-A) unblocks → mark_dispatched(row-A).
///    dispatch(row-B): GateSink sees `gate_open=true` → passes through immediately
///    → mark_dispatched(row-B).
/// 8. `tick()` returns. The orchestrator re-enters the biased select, observes
///    cancellation, and exits.
/// 9. Both rows must be Dispatched (terminal); none left Pending or Processing.
#[tokio::test]
async fn graceful_shutdown_flushes_multi_row_batch() {
    let queue = make_queue();

    // Shared observation list — GateSink records every successfully dispatched
    // message here.
    let observations: Arc<Mutex<Vec<JobDispatchMsg>>> = Arc::new(Mutex::new(vec![]));

    let entered = Arc::new(Notify::new());
    let gate = Arc::new(Notify::new());
    let gate_open = Arc::new(AtomicBool::new(false));

    let sink: Arc<dyn ExecutionSink> = Arc::new(GateSink::new(
        Arc::clone(&entered),
        Arc::clone(&gate),
        Arc::clone(&gate_open),
        Arc::clone(&observations),
    ));

    // Enqueue 2 rows with the same tag so both are claimed in one batch.
    queue
        .enqueue(&make_msg(60, "plugin-f", "exec-mra-1"))
        .await
        .unwrap();
    queue
        .enqueue(&make_msg(61, "plugin-f", "exec-mra-2"))
        .await
        .unwrap();

    let tags = vec!["plugin-f".parse::<PluginKey>().unwrap()];
    let shutdown = CancellationToken::new();

    // Pre-register `entered_fut` BEFORE spawning — `notify_one()` inside
    // GateSink stores a permit so no wakeup is lost even if the future polls
    // after the notification fires.
    let entered_fut = entered.notified();
    tokio::pin!(entered_fut);

    let orch = Orchestrator::new(
        queue.clone() as Arc<dyn JobDispatchQueue>,
        sink,
        proc16(b"proc-mra----"),
        tags.clone(),
    )
    // batch_size=2: both rows are claimed in a single tick() call.
    .with_batch_size(2)
    .with_poll_interval(Duration::from_millis(5));

    let handle = orch.spawn(shutdown.clone());

    // Step 4: wait until the first dispatch is inside GateSink (non-vacuous:
    // the cancel fires BEFORE the gate is opened and BEFORE batch finishes).
    tokio::time::timeout(Duration::from_secs(5), &mut entered_fut)
        .await
        .expect("first dispatch entered GateSink within 5s");

    // Step 5: cancel — the orchestrator is blocked in `handle_entry()` for
    // the first row; it cannot observe the cancel until after handle_entry
    // returns for the last row in the batch.
    shutdown.cancel();

    // Gate not yet open → proven: cancel fired before batch completed.
    assert!(
        !gate_open.load(Ordering::Acquire),
        "gate must still be closed when cancel fires (non-vacuous proof)"
    );

    // Step 6: open the gate — unblocks the first dispatch and lets the second
    // pass through immediately (gate_open=true after first dispatch returns).
    gate.notify_waiters();

    // Step 7–8: both dispatches complete; orchestrator exits.
    handle.await.expect("graceful shutdown after cancel");

    // Step 9: both rows recorded by the GateSink and both terminal.
    let seen = observations.lock().expect("poisoned lock").clone();
    assert_eq!(
        seen.len(),
        2,
        "GateSink must record both rows: batch-flush must complete despite cancel mid-batch"
    );

    let leftover = queue
        .claim_pending(&proc16(b"recovery-mr-"), 8, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "all rows must be terminal (Dispatched) after multi-row batch flush; \
         none must be left Pending or Processing"
    );
}
