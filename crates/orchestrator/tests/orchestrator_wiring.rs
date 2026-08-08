//! Integration tests for [`Orchestrator`] wiring against [`InMemoryJobDispatchQueue`],
//! [`InMemoryTurnHandoff`], and a [`RecordingSink`] spy, using the paused tokio clock.
//!
//! Every row a test enqueues gets a seeded execution row under the same
//! in-memory core the queue and the handoff share: since the handoff (#976)
//! ends each claim at `accept_turn`, a row without an execution is an orphan
//! and is terminalised, not dispatched.
//!
//! Tests:
//!
//! 1. `routes_by_tag` — alpha + beta jobs, worker advertises [alpha]; spy sees only alpha.
//! 2. `claim_route_sink_mark_dispatched` — sink Ok once; row terminal-dispatched; counter=1.
//! 3. `dispatched_row_not_reclaimed` — terminal row is not re-served by a second claim.
//! 4. `blocked_action_outlives_claim_without_renewal_or_duplicates` — the NS05
//!    acceptance: a deliberately blocked action outlives the dispatch claim with
//!    no claim renewal and no second live owner.
//! 5. `invalid_execution_id_marks_failed` — validation terminalises rows that can
//!    never be dispatched before the handoff writes anything.
//! 6. `orphaned_execution_marks_failed` — a valid id with no execution row is an
//!    orphaned dispatch and is terminalised at the handoff.
//! 7. `post_handoff_sink_failure_keeps_row_terminal` — a sink error after the
//!    handoff records `failed` without touching the acknowledged row; the lease
//!    governs recovery.
//! 8. `reclaim_recovers_crashed` — a claim abandoned before the handoff is
//!    reclaimed to Pending; a second live orchestrator drives a row through the
//!    EXHAUSTED counter.
//! 9. `graceful_shutdown_flushes_in_flight_dispatch` — cancel while sink blocked
//!    mid-dispatch; dispatch completes after release; the row stays terminal.
//! 10. `graceful_shutdown_flushes_multi_row_batch` — batch_size≥2; cancel fires
//!     while first dispatch is blocked (proven non-vacuous); all rows flushed.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use nebula_core::{PluginKey, id::ExecutionId};
use nebula_metrics::{
    MetricsRegistry,
    naming::{
        NEBULA_ORCHESTRATOR_DISPATCH_TOTAL, NEBULA_ORCHESTRATOR_HANDOFF_TOTAL,
        NEBULA_ORCHESTRATOR_RECLAIM_TOTAL, orchestrator_dispatch_outcome,
        orchestrator_handoff_outcome, orchestrator_reclaim_outcome,
    },
};
use nebula_orchestrator::{DispatchedTurn, ExecutionSink, ExecutionSinkError, Orchestrator};
use nebula_storage::inmem::{
    InMemoryExecutionStore, InMemoryJobDispatchQueue, InMemoryTurnHandoff,
};
use nebula_storage_port::{
    Scope,
    dto::{ControlCommand, JobDispatchMsg},
    store::{ExecutionStore, ExecutionTurnHandoff, JobDispatchQueue},
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

fn scope() -> Scope {
    Scope::new("ws_test", "org_test")
}

/// One shared in-memory core: execution store, job-dispatch queue, and turn
/// handoff over the SAME state. Sharing the core is the wiring invariant the
/// production composition roots honour (#976): the handoff commits the lease
/// write and the queue acknowledgement under one boundary.
struct TestCore {
    store: Arc<InMemoryExecutionStore>,
    queue: Arc<InMemoryJobDispatchQueue>,
    handoff: Arc<InMemoryTurnHandoff>,
}

impl TestCore {
    fn new() -> Self {
        let store = Arc::new(InMemoryExecutionStore::new());
        let queue = Arc::new(InMemoryJobDispatchQueue::new(&store));
        let handoff = Arc::new(InMemoryTurnHandoff::new(&store));
        Self {
            store,
            queue,
            handoff,
        }
    }

    /// Seed a `Created` execution row and return the parseable execution id
    /// string the queue row carries. A queue row without one is an orphan:
    /// the handoff terminalises it instead of dispatching.
    async fn seed_execution(&self) -> String {
        let id = ExecutionId::new().to_string();
        self.store
            .create(
                &scope(),
                &id,
                "wf_test",
                serde_json::json!({"status": "Created"}),
            )
            .await
            .expect("seed execution row");
        id
    }
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

/// Spy that records every accepted turn. Optionally returns
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
    async fn dispatch(&self, turn: &DispatchedTurn<'_>) -> Result<(), ExecutionSinkError> {
        let fail = {
            let mut f = self.fail_next.lock().expect("poisoned lock");
            let was = *f;
            *f = false;
            was
        };
        if fail {
            return Err(ExecutionSinkError::Rejected(format!(
                "test reject for execution_id={}",
                turn.msg.execution_id
            )));
        }
        self.observations
            .lock()
            .expect("poisoned lock")
            .push(turn.msg.clone());
        self.notify.notify_waiters();
        Ok(())
    }
}

// ── StalledSink ───────────────────────────────────────────────────────────────

/// Sink that blocks inside `dispatch` until `release` is notified. Used to
/// park the orchestrator mid-dispatch so the test can cancel and verify the
/// row was left terminal (the handoff already acknowledged it).
///
/// `entered` is notified the moment the orchestrator calls `dispatch`, before
/// the blocking wait. The test awaits `entered` to confirm the orchestrator
/// holds the turn — no polling of `claim_pending` needed, which avoids the
/// probe accidentally claiming the row and defeating the test.
#[derive(Debug)]
struct StalledSink {
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

#[async_trait]
impl ExecutionSink for StalledSink {
    async fn dispatch(&self, _turn: &DispatchedTurn<'_>) -> Result<(), ExecutionSinkError> {
        // Signal the test that we are inside dispatch (turn accepted).
        self.entered.notify_one();
        // Block until the test notifies the release gate.
        self.release.notified().await;
        Ok(())
    }
}

// ── StalledRecordingSink ──────────────────────────────────────────────────────

/// Sink that counts every dispatch entry, records every completed dispatch,
/// and blocks until `release` is notified. Used by the NS05 acceptance test to
/// hold the orchestrator inside dispatch long past `reclaim_after` and prove
/// the claim neither renewed nor redelivered the row.
///
/// `entries` increments BEFORE the block, so the test can assert exactly one
/// turn was handed to the sink while it is still blocked; `observations`
/// fills after release.
#[derive(Debug, Default)]
struct StalledRecordingSink {
    entries: std::sync::atomic::AtomicUsize,
    observations: Mutex<Vec<JobDispatchMsg>>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl StalledRecordingSink {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn entered_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.entered)
    }

    fn release_notify(&self) -> Arc<Notify> {
        Arc::clone(&self.release)
    }

    fn entry_count(&self) -> usize {
        self.entries.load(Ordering::Acquire)
    }

    fn snapshot(&self) -> Vec<JobDispatchMsg> {
        self.observations.lock().expect("poisoned lock").clone()
    }
}

#[async_trait]
impl ExecutionSink for StalledRecordingSink {
    async fn dispatch(&self, turn: &DispatchedTurn<'_>) -> Result<(), ExecutionSinkError> {
        self.entries.fetch_add(1, Ordering::AcqRel);
        self.entered.notify_one();
        self.release.notified().await;
        self.observations
            .lock()
            .expect("poisoned lock")
            .push(turn.msg.clone());
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
    async fn dispatch(&self, turn: &DispatchedTurn<'_>) -> Result<(), ExecutionSinkError> {
        self.entered.notify_one();
        if !self.gate_open.load(Ordering::Acquire) {
            self.gate.notified().await;
            self.gate_open.store(true, Ordering::Release);
        }
        self.observations
            .lock()
            .expect("poisoned lock")
            .push(turn.msg.clone());
        Ok(())
    }
}

/// Read the handoff `accepted` counter from `registry`.
fn accepted_count(registry: &MetricsRegistry) -> u64 {
    let labels = registry
        .interner()
        .single("outcome", orchestrator_handoff_outcome::ACCEPTED);
    registry
        .counter_labeled(NEBULA_ORCHESTRATOR_HANDOFF_TOTAL, &labels)
        .unwrap()
        .get()
}

// ── test 1: routes_by_tag ─────────────────────────────────────────────────────

/// Worker advertises only `[alpha]`. Both an alpha and a beta job are enqueued.
/// The orchestrator must dispatch exactly the alpha job; the beta job must
/// remain Pending (claimable by a beta-capable worker).
#[tokio::test(start_paused = true)]
async fn routes_by_tag() {
    let core = TestCore::new();
    let spy = RecordingSink::new();

    let alpha_id = core.seed_execution().await;
    let beta_id = core.seed_execution().await;
    core.queue
        .enqueue(&make_msg(1, "alpha", &alpha_id))
        .await
        .unwrap();
    core.queue
        .enqueue(&make_msg(2, "beta", &beta_id))
        .await
        .unwrap();

    let shutdown = CancellationToken::new();
    let orch = Orchestrator::new(
        core.queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        core.handoff.clone() as Arc<dyn ExecutionTurnHandoff>,
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
    let leftover = core
        .queue
        .claim_pending(&proc16(b"beta-worker-"), 8, &beta_tags)
        .await
        .unwrap();
    assert_eq!(
        leftover.len(),
        1,
        "beta job must still be Pending after alpha-only orchestrator ran"
    );
    assert_eq!(leftover[0].msg.required_plugin_key.as_str(), "beta");
}

// ── test 2: claim_route_sink_mark_dispatched ──────────────────────────────────

/// Sink returns `Ok` once. The `dispatched` counter reaches 1 and the handoff
/// `accepted` counter reaches 1. A second `claim_pending` from a fresh
/// processor returns nothing (row is terminal).
#[tokio::test(start_paused = true)]
async fn claim_route_sink_mark_dispatched() {
    let core = TestCore::new();
    let spy = RecordingSink::new();
    let registry = MetricsRegistry::new();

    let exec_id = core.seed_execution().await;
    core.queue
        .enqueue(&make_msg(10, "plugin-a", &exec_id))
        .await
        .unwrap();

    let shutdown = CancellationToken::new();
    let orch = Orchestrator::new(
        core.queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        core.handoff.clone() as Arc<dyn ExecutionTurnHandoff>,
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
    assert_eq!(
        accepted_count(&registry),
        1,
        "handoff accepted counter must be 1"
    );

    // Row is terminal — a fresh processor finds nothing Pending.
    let tags = vec!["plugin-a".parse::<PluginKey>().unwrap()];
    let leftover = core
        .queue
        .claim_pending(&proc16(b"fresh-proc--"), 8, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "row must be terminal after successful dispatch"
    );
}

// ── test 3: dispatched_row_not_reclaimed ──────────────────────────────────────

/// After one handoff-and-dispatch, the spy count stays at 1. A second
/// `claim_pending` from a different processor returns nothing — the row is
/// terminal (acknowledged by the handoff) and is not re-served.
#[tokio::test(start_paused = true)]
async fn dispatched_row_not_reclaimed() {
    let core = TestCore::new();
    let spy = RecordingSink::new();

    let exec_id = core.seed_execution().await;
    core.queue
        .enqueue(&make_msg(20, "plugin-b", &exec_id))
        .await
        .unwrap();

    let shutdown = CancellationToken::new();
    let orch = Orchestrator::new(
        core.queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        core.handoff.clone() as Arc<dyn ExecutionTurnHandoff>,
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

    // Let the orchestrator finish the entry before we shut down.
    tokio::time::advance(Duration::from_millis(30)).await;

    shutdown.cancel();
    handle.await.expect("graceful shutdown");

    assert_eq!(spy.snapshot().len(), 1, "sink must be invoked exactly once");

    // Second processor sees nothing — the row was acknowledged by the
    // handoff and must not be re-served via claim_pending.
    let tags = vec!["plugin-b".parse::<PluginKey>().unwrap()];
    let second = core
        .queue
        .claim_pending(&proc16(b"proc-nd-2---"), 8, &tags)
        .await
        .unwrap();
    assert!(
        second.is_empty(),
        "dispatched_row_not_reclaimed: second claim must be empty after terminal dispatch"
    );
}

// ── test 4: blocked_action_outlives_claim_without_renewal_or_duplicates ───────

/// The NS05 acceptance criterion (#976): **action duration must not extend
/// dispatch-queue claim ownership.**
///
/// 1. The orchestrator claims the row and blocks inside `StalledRecordingSink`
///    — but only AFTER the handoff committed: the row is already acknowledged
///    and the execution holds a durable lease.
/// 2. Real time advances far past `reclaim_after`.
/// 3. A reclaim sweep finds nothing to reclaim (the row is not `Processing`).
/// 4. A second processor claims nothing — no redelivery.
/// 5. A second runner cannot take the execution lease — no duplicate live
///    owner while the first sink is still blocked.
/// 6. The sink observed the row exactly once.
///
/// Red-ability: reverting the orchestrator to the pre-#976 flow (dispatch
/// first, ack after) makes the row `Processing` during the stall, so step 3
/// reclaims it (`reclaimed == 1`) and step 4 hands it to processor-B — both
/// assertions fail.
#[tokio::test]
async fn blocked_action_outlives_claim_without_renewal_or_duplicates() {
    let core = TestCore::new();
    let sink = StalledRecordingSink::new();
    let entered = sink.entered_notify();
    let release = sink.release_notify();

    let exec_id = core.seed_execution().await;
    core.queue
        .enqueue(&make_msg(21, "plugin-ns5", &exec_id))
        .await
        .unwrap();

    let tags = vec!["plugin-ns5".parse::<PluginKey>().unwrap()];
    let proc_a = proc16(b"proc-ns5-a--");

    let shutdown = CancellationToken::new();

    // Pre-register entered future before spawning so notify_one() is not lost.
    let entered_fut = entered.notified();
    tokio::pin!(entered_fut);

    let orch = Orchestrator::new(
        core.queue.clone() as Arc<dyn JobDispatchQueue>,
        sink.clone() as Arc<dyn ExecutionSink>,
        core.handoff.clone() as Arc<dyn ExecutionTurnHandoff>,
        proc_a,
        tags.clone(),
    )
    .with_batch_size(1)
    // Very short reclaim window: the blocked action will outlive it many
    // times over without any claim renewal.
    .with_reclaim_after(Duration::from_millis(20))
    .with_reclaim_interval(Duration::from_millis(50))
    .with_max_reclaim_count(99)
    .with_poll_interval(Duration::from_millis(5));

    let handle = orch.spawn(shutdown.clone());

    // Step 1: wait until the orchestrator is inside dispatch. The handoff has
    // already committed at this point — prove it: the execution row carries
    // the orchestrator's lease.
    tokio::time::timeout(Duration::from_secs(5), &mut entered_fut)
        .await
        .expect("orchestrator entered dispatch within 5s");

    let record = core
        .store
        .get(&scope(), &exec_id)
        .await
        .unwrap()
        .expect("execution row exists");
    assert!(
        record
            .lease_holder
            .as_deref()
            .is_some_and(|h| h.starts_with("orchestrator:")),
        "the handoff must hold the execution lease before dispatch runs; got {:?}",
        record.lease_holder
    );

    // Step 2: outlive the claim window several times over.
    tokio::time::sleep(Duration::from_millis(120)).await;

    // Step 3: the sweep finds nothing — the row was acknowledged at handoff
    // time and never sat `Processing` while the action ran.
    let swept = core
        .queue
        .reclaim_stuck(Duration::from_millis(5), 99)
        .await
        .unwrap();
    assert_eq!(
        swept.reclaimed, 0,
        "a blocked action must not leave its row reclaimable (claim ended at handoff)"
    );
    assert_eq!(swept.exhausted, 0, "nothing may be exhausted either");

    // Step 4: a second processor claims nothing — no redelivery of a turn
    // that is already running.
    let second = core
        .queue
        .claim_pending(&proc16(b"proc-ns5-b--"), 8, &tags)
        .await
        .unwrap();
    assert!(
        second.is_empty(),
        "no redelivery: the row is terminal, a second worker must claim nothing"
    );

    // Step 5: no duplicate live owner — the execution lease blocks a second
    // acquisition outright while the first sink is still blocked.
    let stolen = core
        .store
        .acquire_lease(
            &scope(),
            &exec_id,
            "probe-second-runner",
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert!(
        stolen.is_none(),
        "a second runner must not acquire the lease while the blocked action owns it"
    );

    // Step 6: exactly one sink entry — no duplicate turn while blocked.
    assert_eq!(
        sink.entry_count(),
        1,
        "the blocked action must be dispatched exactly once"
    );

    // Release the sink and shut down cleanly.
    release.notify_one();
    shutdown.cancel();
    handle.await.expect("graceful shutdown");

    assert_eq!(
        sink.entry_count(),
        1,
        "releasing the stall must not re-dispatch the turn"
    );
    assert_eq!(
        sink.snapshot().len(),
        1,
        "the released dispatch must complete exactly once"
    );
}

// ── test 5: invalid_execution_id_marks_failed ─────────────────────────────────

/// A row whose execution id does not parse can never be dispatched. The
/// orchestrator terminalises it before the handoff writes anything: `failed`
/// counter 1, no handoff accepted, row not re-served.
#[tokio::test(start_paused = true)]
async fn invalid_execution_id_marks_failed() {
    let core = TestCore::new();
    let spy = RecordingSink::new();
    let registry = MetricsRegistry::new();

    // No execution row seeded — the id never even reaches the handoff.
    core.queue
        .enqueue(&make_msg(30, "plugin-c", "not-an-execution-id"))
        .await
        .unwrap();

    let shutdown = CancellationToken::new();
    let orch = Orchestrator::new(
        core.queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        core.handoff.clone() as Arc<dyn ExecutionTurnHandoff>,
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
    assert_eq!(
        accepted_count(&registry),
        0,
        "validation failure must never reach the handoff"
    );
    assert!(
        spy.snapshot().is_empty(),
        "an invalid execution id must never reach the sink"
    );

    // The terminalised row must not be re-served.
    let tags = vec!["plugin-c".parse::<PluginKey>().unwrap()];
    let leftover = core
        .queue
        .claim_pending(&proc16(b"other-proc--"), 8, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "invalid-id row must be terminal, not re-served"
    );
}

// ── test 6: orphaned_execution_marks_failed ───────────────────────────────────

/// A valid execution id with no execution row is an orphaned dispatch: the
/// emitter materialises both rows atomically, so a missing execution will not
/// appear later. The handoff reports NotFound and the orchestrator
/// terminalises the row instead of redelivering forever.
#[tokio::test(start_paused = true)]
async fn orphaned_execution_marks_failed() {
    let core = TestCore::new();
    let spy = RecordingSink::new();
    let registry = MetricsRegistry::new();

    // Valid, parseable id — but deliberately NOT seeded in the store.
    let orphan_id = ExecutionId::new().to_string();
    core.queue
        .enqueue(&make_msg(31, "plugin-orp", &orphan_id))
        .await
        .unwrap();

    let shutdown = CancellationToken::new();
    let orch = Orchestrator::new(
        core.queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        core.handoff.clone() as Arc<dyn ExecutionTurnHandoff>,
        proc16(b"proc-orphan-"),
        vec!["plugin-orp".parse::<PluginKey>().unwrap()],
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

    let handoff_error_labels = registry
        .interner()
        .single("outcome", orchestrator_handoff_outcome::ERROR);
    let handoff_errors = registry
        .counter_labeled(NEBULA_ORCHESTRATOR_HANDOFF_TOTAL, &handoff_error_labels)
        .unwrap()
        .get();
    assert_eq!(
        handoff_errors, 1,
        "the handoff must report the missing execution once"
    );
    assert_eq!(
        accepted_count(&registry),
        0,
        "an orphaned execution must never be accepted"
    );
    assert!(
        spy.snapshot().is_empty(),
        "an orphaned execution must never reach the sink"
    );

    let tags = vec!["plugin-orp".parse::<PluginKey>().unwrap()];
    let leftover = core
        .queue
        .claim_pending(&proc16(b"probe-orphan"), 8, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "orphaned row must be terminal after mark_failed"
    );
}

// ── test 7: post_handoff_sink_failure_keeps_row_terminal ──────────────────────

/// A sink error AFTER the handoff records `failed` for operators but touches
/// no queue state: the row was already acknowledged, and recovery is governed
/// by the execution lease the handoff minted. The row is not re-served, and
/// the lease stays held for lease-governed recovery.
#[tokio::test(start_paused = true)]
async fn post_handoff_sink_failure_keeps_row_terminal() {
    let core = TestCore::new();
    let spy = RecordingSink::new();
    spy.set_fail_next();
    let registry = MetricsRegistry::new();

    let exec_id = core.seed_execution().await;
    core.queue
        .enqueue(&make_msg(32, "plugin-pf", &exec_id))
        .await
        .unwrap();

    let shutdown = CancellationToken::new();
    let orch = Orchestrator::new(
        core.queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        core.handoff.clone() as Arc<dyn ExecutionTurnHandoff>,
        proc16(b"proc-pf-----"),
        vec!["plugin-pf".parse::<PluginKey>().unwrap()],
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

    assert_eq!(
        accepted_count(&registry),
        1,
        "the handoff must have accepted the turn before the sink failed"
    );

    // The acknowledged row is not re-served.
    let tags = vec!["plugin-pf".parse::<PluginKey>().unwrap()];
    let leftover = core
        .queue
        .claim_pending(&proc16(b"probe-pf----"), 8, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "a post-handoff sink failure must not make the row claimable again"
    );

    // Recovery is lease-governed: the handoff's lease is still on the row.
    let record = core
        .store
        .get(&scope(), &exec_id)
        .await
        .unwrap()
        .expect("execution row exists");
    assert!(
        record.lease_holder.is_some(),
        "the execution lease must remain held for lease-governed recovery"
    );
}

// ── test 8: reclaim_recovers_crashed ─────────────────────────────────────────

/// Simulate a runner that crashed BEFORE its handoff committed: claim a row
/// manually (putting it in Processing), never hand off, never mark. A fresh
/// orchestrator with aggressive reclaim settings sweeps it back to Pending
/// (RECLAIMED counter ≥ 1). Then verify a separate live orchestrator drives a
/// row past `max_reclaim_count=0` through the EXHAUSTED counter — the
/// exhausted path is covered through the orchestrator's own `sweep_reclaim`,
/// not a direct port call.
#[tokio::test(start_paused = true)]
async fn reclaim_recovers_crashed() {
    // ── sub-case 1: live orchestrator reclaims a crashed row ─────────────────
    let core = TestCore::new();
    let spy = RecordingSink::new();
    let registry = MetricsRegistry::new();

    let exec_id = core.seed_execution().await;
    core.queue
        .enqueue(&make_msg(40, "plugin-d", &exec_id))
        .await
        .unwrap();

    // Claim the row without handing off or marking — crash simulated.
    let tags = vec!["plugin-d".parse::<PluginKey>().unwrap()];
    let claimed = core
        .queue
        .claim_pending(&proc16(b"crashed-proc"), 1, &tags)
        .await
        .unwrap();
    assert_eq!(claimed.len(), 1, "row must be claimed into Processing");
    // Intentionally no handoff, no mark — crash simulated.

    // Fresh orchestrator with short reclaim_after so the paused clock just needs
    // a small advance to make the row stale.
    let reclaim_after = Duration::from_millis(10);
    let orch = Orchestrator::new(
        core.queue.clone() as Arc<dyn JobDispatchQueue>,
        spy.clone() as Arc<dyn ExecutionSink>,
        core.handoff.clone() as Arc<dyn ExecutionTurnHandoff>,
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
    // A second core seeds a row that is already in Processing (crashed runner).
    // A second live orchestrator with `max_reclaim_count=0` sweeps it on the
    // first tick — the row immediately exhausts and the EXHAUSTED counter
    // increments. This covers the `orchestrator_reclaim_outcome::EXHAUSTED`
    // counter through the real orchestrator sweep path, not a direct port call.
    let core2 = TestCore::new();
    let spy2 = RecordingSink::new();
    let registry2 = MetricsRegistry::new();
    let tags2 = vec!["plugin-d2".parse::<PluginKey>().unwrap()];

    let exec_id2 = core2.seed_execution().await;
    core2
        .queue
        .enqueue(&make_msg(41, "plugin-d2", &exec_id2))
        .await
        .unwrap();
    // Claim without marking — crashed runner simulation.
    let claimed2 = core2
        .queue
        .claim_pending(&proc16(b"crash-proc2-"), 1, &tags2)
        .await
        .unwrap();
    assert_eq!(claimed2.len(), 1, "row must be claimed into Processing");

    // Orchestrator with max_reclaim_count=0: any Processing row immediately
    // exhausts on the first sweep (reclaim_count starts at 0, budget is 0).
    let orch2 = Orchestrator::new(
        core2.queue.clone() as Arc<dyn JobDispatchQueue>,
        spy2.clone() as Arc<dyn ExecutionSink>,
        core2.handoff.clone() as Arc<dyn ExecutionTurnHandoff>,
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

// ── test 9: graceful_shutdown_flushes_in_flight_dispatch ─────────────────────

/// Cancelling the orchestrator while a dispatch is in flight does NOT drop the
/// in-flight work: the shutdown contract is batch-flush (finish the current
/// batch, then exit). The row was acknowledged by the handoff before dispatch
/// began; the flush completes the dispatch and the row stays terminal.
///
/// The "row left Processing for reclaim" contract applies only to rows whose
/// runner crashed BEFORE the handoff committed — tested in
/// `reclaim_recovers_crashed` (test 8). This test exercises the
/// graceful-flush path.
///
/// Sequence:
/// 1. Enqueue a row (execution seeded).
/// 2. Start the orchestrator with `StalledSink`. `StalledSink::dispatch` fires
///    `entered` before blocking on `release`, giving the test an exact signal
///    that the turn was accepted and the orchestrator is inside dispatch.
/// 3. Await `entered` — no `claim_pending` polling so no probe-claim race.
/// 4. Cancel the token while the orchestrator is blocked in dispatch.
/// 5. Release the sink — dispatch returns `Ok`. The orchestrator finishes the
///    entry, loops back to the biased select, sees the cancellation, and exits.
/// 6. `handle.await` completes.
/// 7. A fresh `claim_pending` returns empty — the row is terminal.
#[tokio::test]
async fn graceful_shutdown_flushes_in_flight_dispatch() {
    let core = TestCore::new();
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let sink: Arc<dyn ExecutionSink> = Arc::new(StalledSink {
        entered: entered.clone(),
        release: release.clone(),
    });

    let exec_id = core.seed_execution().await;
    core.queue
        .enqueue(&make_msg(50, "plugin-e", &exec_id))
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
        core.queue.clone() as Arc<dyn JobDispatchQueue>,
        sink,
        core.handoff.clone() as Arc<dyn ExecutionTurnHandoff>,
        proc16(b"proc-sd-----"),
        tags.clone(),
    )
    .with_batch_size(1)
    .with_poll_interval(Duration::from_millis(5));

    let handle = orch.spawn(shutdown.clone());

    // Exact handshake: block until the orchestrator is inside StalledSink::dispatch.
    tokio::time::timeout(Duration::from_secs(5), &mut entered_fut)
        .await
        .expect("orchestrator entered dispatch within 5s");

    // Signal shutdown while the orchestrator is blocked in dispatch.
    // The biased select cannot fire yet — the orchestrator is not in the select
    // loop; it is in `tick()` → `handle_entry()`. Shutdown will be observed on
    // the NEXT loop iteration after `handle_entry` returns.
    shutdown.cancel();

    // Release the sink so dispatch completes with Ok. The orchestrator then
    // finishes `tick()`, re-enters the biased select, sees `cancelled()`, and
    // exits.
    release.notify_waiters();

    handle.await.expect("graceful shutdown after cancel");

    // Row is terminal — a fresh claim returns empty.
    let leftover = core
        .queue
        .claim_pending(&proc16(b"recovery----"), 4, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "row must be terminal after graceful flush; claim_pending must return empty"
    );
}

// ── test 10: graceful_shutdown_flushes_multi_row_batch ────────────────────────

/// Proves that graceful shutdown flushes the **entire** in-flight batch, not
/// just the first row. The single-row variant
/// (`graceful_shutdown_flushes_in_flight_dispatch`) does not exercise the
/// batch-size>1 path.
///
/// Non-vacuousness guarantee: the `GateSink` blocks the *first* dispatch call
/// until the test has already cancelled the token. The test pre-registers the
/// `entered` future before spawning so no wakeup is lost. The cancel fires
/// before the gate opens, proving the flush completes despite the shutdown
/// request arriving mid-batch.
///
/// Sequence:
/// 1. Enqueue 2 rows (both executions seeded).
/// 2. Start the orchestrator with `batch_size=2` and a `GateSink`.
/// 3. The orchestrator's first `tick()` claims both rows in one batch and
///    begins handing off + dispatching them sequentially:
///    - handoff(row-A), dispatch(row-A): GateSink signals `entered`, then
///      blocks on `gate`.
/// 4. Test observes `entered` (first dispatch is blocked — proven non-vacuous).
/// 5. Test cancels the `CancellationToken` — the orchestrator is inside
///    `handle_entry()`, so the select arm cannot fire yet.
/// 6. Test opens the gate (`gate.notify_waiters()`).
/// 7. dispatch(row-A) unblocks; handoff(row-B) + dispatch(row-B) pass through
///    immediately (gate_open=true after first dispatch returns).
/// 8. `tick()` returns. The orchestrator re-enters the biased select, observes
///    cancellation, and exits.
/// 9. Both rows must be terminal; none left Pending or Processing.
#[tokio::test]
async fn graceful_shutdown_flushes_multi_row_batch() {
    let core = TestCore::new();

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
    let exec_id1 = core.seed_execution().await;
    let exec_id2 = core.seed_execution().await;
    core.queue
        .enqueue(&make_msg(60, "plugin-f", &exec_id1))
        .await
        .unwrap();
    core.queue
        .enqueue(&make_msg(61, "plugin-f", &exec_id2))
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
        core.queue.clone() as Arc<dyn JobDispatchQueue>,
        sink,
        core.handoff.clone() as Arc<dyn ExecutionTurnHandoff>,
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

    let leftover = core
        .queue
        .claim_pending(&proc16(b"recovery-mr-"), 8, &tags)
        .await
        .unwrap();
    assert!(
        leftover.is_empty(),
        "all rows must be terminal after multi-row batch flush; \
         none must be left Pending or Processing"
    );
}
