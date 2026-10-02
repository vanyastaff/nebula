//! Iteration checkpoints of a journaled stateful node: what the barrier
//! allows, what a resume attests, and the integrity a resume demands.

use std::sync::atomic::AtomicU8;

use nebula_storage_port::FencingToken;

use super::*;

/// A checkpoint store over the harness's in-memory one that fails where
/// scripted.
#[derive(Debug)]
struct ScriptedCheckpoints {
    inner: nebula_storage::InMemoryCheckpointStore,
    /// Every load answers this failure.
    fail_loads: Mutex<Option<IterationCheckpointError>>,
    /// Every save answers this failure (after the inner store saw nothing).
    fail_saves: Mutex<Option<IterationCheckpointError>>,
    saves: AtomicU8,
}

impl ScriptedCheckpoints {
    fn new(harness: &Harness) -> Arc<Self> {
        Arc::new(Self {
            inner: nebula_storage::InMemoryCheckpointStore::new(&harness.executions),
            fail_loads: Mutex::new(None),
            fail_saves: Mutex::new(None),
            saves: AtomicU8::new(0),
        })
    }

    fn fail_loads(&self, error: IterationCheckpointError) {
        *self.fail_loads.lock().expect("script") = Some(error);
    }

    fn fail_saves(&self, error: IterationCheckpointError) {
        *self.fail_saves.lock().expect("script") = Some(error);
    }
}

#[async_trait::async_trait]
impl CheckpointStore for ScriptedCheckpoints {
    async fn load_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
    ) -> Result<Option<IterationCheckpoint>, IterationCheckpointError> {
        if let Some(error) = *self.fail_loads.lock().expect("script") {
            return Err(error);
        }
        self.inner.load_iteration_checkpoint(key).await
    }

    async fn save_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
        checkpoint: &IterationCheckpoint,
        fencing: FencingToken,
    ) -> Result<CheckpointSaved, IterationCheckpointError> {
        self.saves.fetch_add(1, Ordering::SeqCst);
        if let Some(error) = *self.fail_saves.lock().expect("script") {
            return Err(error);
        }
        self.inner
            .save_iteration_checkpoint(key, checkpoint, fencing)
            .await
    }
}

/// The stateful journal of attempt `attempt_generation`, checkpointing into
/// `store`.
fn checkpointed(
    harness: &Harness,
    attempt_generation: u64,
    store: &Arc<ScriptedCheckpoints>,
) -> NodeEffectJournal {
    let mut authority = harness.authority(
        attempt_generation,
        Arc::new(nebula_core::accessor::SystemClock),
        JournalShape::Iterated,
    );
    authority.checkpoints = Some(Arc::clone(store) as Arc<dyn CheckpointStore>);
    NodeEffectJournal::new(authority)
}

/// Runs iteration `iteration` with one charge of `order` and passes its
/// barrier.
async fn charge_iteration(
    harness: &Harness,
    journal: &NodeEffectJournal,
    iteration: u32,
    order: u64,
) {
    journal
        .begin_iteration(iteration)
        .expect("the iteration opens");
    harness
        .handle(journal)
        .submit(Charge::<false> { order })
        .await
        .expect("the charge settles");
    journal
        .end_iteration(DRAIN, true)
        .await
        .expect("the barrier passes");
}

/// Stores a checkpoint naming `iteration` directly, bypassing the journal.
async fn store_checkpoint(
    harness: &Harness,
    store: &ScriptedCheckpoints,
    iteration: u32,
    state: &[u8],
    digest: [u8; 32],
    attested_positions: u32,
) {
    let execution = harness.execution_id.to_string();
    let key = IterationCheckpointKey::new(
        &harness.scope,
        &execution,
        "charge",
        "billing.charge",
        "1.0.0",
    )
    .expect("key");
    let checkpoint = IterationCheckpoint::new(
        iteration,
        state.to_vec(),
        digest,
        None,
        attested_positions,
        1,
    )
    .expect("checkpoint");
    store
        .inner
        .save_iteration_checkpoint(&key, &checkpoint, harness.fencing)
        .await
        .expect("stored");
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

#[tokio::test]
async fn a_resumed_attempt_never_runs_or_demands_its_attested_iterations() {
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    let first = checkpointed(&harness, 1, &store);
    assert_eq!(first.resume_from_checkpoint().await, Ok(None));
    charge_iteration(&harness, &first, 0, 10).await;
    first
        .save_checkpoint(1, &serde_json::json!({"n": 1}), None)
        .await
        .expect("checkpoint after it0");
    charge_iteration(&harness, &first, 1, 11).await;
    first
        .save_checkpoint(
            2,
            &serde_json::json!({"n": 2}),
            Some(Duration::from_secs(9)),
        )
        .await
        .expect("checkpoint after it1");
    // The process dies before iteration 2.
    drop(first);
    assert_eq!(harness.desk.keys().len(), 2);

    let second = checkpointed(&harness, 2, &store);
    let point = second
        .resume_from_checkpoint()
        .await
        .expect("verified")
        .expect("a checkpoint");
    assert_eq!(point.iteration, 2);
    assert_eq!(point.state, serde_json::json!({"n": 2}));
    assert_eq!(
        point.delay,
        Some(Duration::from_secs(9)),
        "nothing shows iteration 2 ran: its delay is honoured"
    );
    charge_iteration(&harness, &second, 2, 12).await;
    assert_eq!(
        second.conclude(DRAIN).await,
        Ok(()),
        "the attested iterations' effects are neither met nor demanded"
    );
    assert_eq!(
        harness.desk.keys().len(),
        3,
        "no attested effect was sent again"
    );
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_RESUMES_TOTAL,
            &[("outcome", "resumed")]
        ),
        1
    );
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_CHECKPOINTS_TOTAL,
            &[("outcome", "recorded")]
        ),
        2
    );
}

#[tokio::test]
async fn a_failing_resumed_node_keeps_its_own_failure_over_attested_effects() {
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    let first = checkpointed(&harness, 1, &store);
    first.resume_from_checkpoint().await.expect("none");
    charge_iteration(&harness, &first, 0, 10).await;
    first
        .save_checkpoint(1, &serde_json::json!({"n": 1}), None)
        .await
        .expect("checkpoint");
    drop(first);

    let second = checkpointed(&harness, 2, &store);
    second
        .resume_from_checkpoint()
        .await
        .expect("verified")
        .expect("a checkpoint");
    second.begin_iteration(1).expect("it1");
    // The iteration fails before any effect: the barrier keeps its failure,
    // and nothing below is demanded.
    second
        .end_iteration(DRAIN, false)
        .await
        .expect("a failing barrier with nothing to drain");
    assert_eq!(
        second.conclude_node(DRAIN, false).await,
        Ok(Concluded::Clean)
    );
}

#[tokio::test]
async fn the_delay_is_skipped_when_the_ledger_shows_its_iteration_ran() {
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    let first = checkpointed(&harness, 1, &store);
    first.resume_from_checkpoint().await.expect("none");
    charge_iteration(&harness, &first, 0, 10).await;
    first
        .save_checkpoint(
            1,
            &serde_json::json!({"n": 1}),
            Some(Duration::from_hours(1)),
        )
        .await
        .expect("checkpoint");
    // Iteration 1 ran and recorded its effect before the crash.
    charge_iteration(&harness, &first, 1, 11).await;
    drop(first);

    let second = checkpointed(&harness, 2, &store);
    let point = second
        .resume_from_checkpoint()
        .await
        .expect("ok")
        .expect("some");
    assert_eq!(point.iteration, 1);
    assert_eq!(point.delay, None, "iteration 1 already ran after its delay");
}

#[tokio::test]
async fn an_unknown_outcome_in_an_attested_iteration_still_halts_the_node() {
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    harness.desk.script(&[Reply::Lost]);
    let first = harness.stateful_journal(1);
    first.begin_iteration(0).expect("it0");
    let _ = harness
        .handle(&first)
        .submit(Charge::<false> { order: 7 })
        .await;
    drop(first);
    // A checkpoint attesting iteration 0 (written by a faulty peer).
    store_checkpoint(&harness, &store, 1, b"{}", sha256(b"{}"), 1).await;

    let second = checkpointed(&harness, 2, &store);
    second
        .resume_from_checkpoint()
        .await
        .expect("ok")
        .expect("some");
    second.begin_iteration(1).expect("it1");
    second.end_iteration(DRAIN, true).await.expect("it1 passes");
    assert!(
        matches!(
            second.conclude(DRAIN).await,
            Err(EffectExecutionError::JournalOutcomeUnknown { .. })
        ),
        "S8 covers attested slots too"
    );
    assert_eq!(harness.desk.keys().len(), 1, "nothing resent");
}

#[tokio::test]
async fn a_checkpoint_needs_its_own_passed_barrier() {
    let harness = Harness::new().await;
    let state = serde_json::json!({"n": 1});

    // Before the barrier.
    let store = ScriptedCheckpoints::new(&harness);
    let journal = checkpointed(&harness, 1, &store);
    journal.begin_iteration(0).expect("it0");
    assert_eq!(
        journal.save_checkpoint(1, &state, None).await,
        Err(EffectExecutionError::InvalidContract)
    );

    // After a barrier the iteration failed.
    let journal = checkpointed(&harness, 1, &store);
    journal.begin_iteration(0).expect("it0");
    journal.end_iteration(DRAIN, false).await.expect("drained");
    assert_eq!(
        journal.save_checkpoint(1, &state, None).await,
        Err(EffectExecutionError::InvalidContract)
    );

    // After a barrier that failed on its own.
    harness.desk.script(&[Reply::Lost]);
    let journal = checkpointed(&harness, 1, &store);
    journal.begin_iteration(0).expect("it0");
    let _ = harness
        .handle(&journal)
        .submit(Charge::<false> { order: 3 })
        .await;
    let failed = journal
        .end_iteration(DRAIN, true)
        .await
        .expect_err("unknown");
    assert_eq!(journal.save_checkpoint(1, &state, None).await, Err(failed));

    // After a cancellation.
    let fresh = Harness::new().await;
    let store = ScriptedCheckpoints::new(&fresh);
    let journal = checkpointed(&fresh, 1, &store);
    journal.begin_iteration(0).expect("it0");
    journal.end_iteration(DRAIN, true).await.expect("passes");
    journal.cancel_iteration();
    assert_eq!(
        journal.save_checkpoint(1, &state, None).await,
        Err(EffectExecutionError::Cancelled)
    );

    // Twice, and for another iteration.
    let journal = checkpointed(&fresh, 1, &store);
    journal.begin_iteration(0).expect("it0");
    journal.end_iteration(DRAIN, true).await.expect("passes");
    assert_eq!(
        journal.save_checkpoint(2, &state, None).await,
        Err(EffectExecutionError::InvalidContract),
        "the barrier of iteration 0 allows a checkpoint of iteration 1 only"
    );
    let journal = checkpointed(&fresh, 1, &store);
    journal.begin_iteration(0).expect("it0");
    journal.end_iteration(DRAIN, true).await.expect("passes");
    journal
        .save_checkpoint(1, &state, None)
        .await
        .expect("once");
    assert_eq!(
        journal.save_checkpoint(1, &state, None).await,
        Err(EffectExecutionError::InvalidContract),
        "a passed barrier allows one checkpoint"
    );
    assert_eq!(
        store.saves.load(Ordering::SeqCst),
        1,
        "only one reached the store"
    );
}

#[tokio::test]
async fn resume_is_consulted_once_before_the_first_iteration() {
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    let journal = checkpointed(&harness, 1, &store);
    journal.begin_iteration(0).expect("it0");
    assert_eq!(
        journal.resume_from_checkpoint().await,
        Err(EffectExecutionError::InvalidContract)
    );

    let journal = checkpointed(&harness, 1, &store);
    assert_eq!(journal.resume_from_checkpoint().await, Ok(None));
    assert_eq!(
        journal.resume_from_checkpoint().await,
        Err(EffectExecutionError::InvalidContract)
    );
}

#[tokio::test]
async fn the_first_iteration_after_a_resume_is_the_checkpointed_one() {
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    store_checkpoint(&harness, &store, 3, b"{}", sha256(b"{}"), 0).await;
    let journal = checkpointed(&harness, 2, &store);
    journal
        .resume_from_checkpoint()
        .await
        .expect("ok")
        .expect("some");
    assert_eq!(
        journal.begin_iteration(0),
        Err(EffectExecutionError::InvalidContract),
        "an attested iteration never runs again"
    );
    let journal = checkpointed(&harness, 2, &store);
    journal
        .resume_from_checkpoint()
        .await
        .expect("ok")
        .expect("some");
    assert_eq!(journal.begin_iteration(3), Ok(()));
}

#[tokio::test]
async fn a_checkpoint_that_contradicts_the_ledger_halts_with_nothing_sent() {
    let invalid =
        EffectExecutionError::IterationCheckpoint(IterationCheckpointError::InvalidRecord);

    // Another count of positions below it.
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    let first = harness.stateful_journal(1);
    charge_iteration(&harness, &first, 0, 1).await;
    drop(first);
    store_checkpoint(&harness, &store, 1, b"{}", sha256(b"{}"), 2).await;
    let journal = checkpointed(&harness, 2, &store);
    assert_eq!(journal.resume_from_checkpoint().await, Err(invalid));
    assert!(invalid.halts_execution());
    assert_eq!(journal.begin_iteration(1), Err(invalid), "nothing runs");

    // A digest that does not match its state.
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    store_checkpoint(&harness, &store, 1, b"{}", sha256(b"[]"), 0).await;
    let journal = checkpointed(&harness, 2, &store);
    assert_eq!(journal.resume_from_checkpoint().await, Err(invalid));

    // A flat occurrence: the node's action changed kind under the row.
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    let flat = harness.journal(1);
    harness
        .handle(&flat)
        .submit(Charge::<false> { order: 1 })
        .await
        .expect("a flat charge");
    drop(flat);
    store_checkpoint(&harness, &store, 1, b"{}", sha256(b"{}"), 0).await;
    let journal = checkpointed(&harness, 2, &store);
    assert_eq!(journal.resume_from_checkpoint().await, Err(invalid));
    assert_eq!(harness.desk.keys().len(), 1, "nothing sent by the resume");
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_RESUMES_TOTAL,
            &[("outcome", "invalid")]
        ),
        1
    );
}

#[tokio::test]
async fn an_unavailable_checkpoint_store_defers_and_never_starts_over() {
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    store.fail_loads(IterationCheckpointError::Unavailable);
    let journal = checkpointed(&harness, 1, &store);
    let deferred = journal
        .resume_from_checkpoint()
        .await
        .expect_err("deferred");
    assert_eq!(
        deferred,
        EffectExecutionError::IterationCheckpoint(IterationCheckpointError::Unavailable)
    );
    assert!(deferred.is_deferred());
    assert_eq!(
        journal.begin_iteration(0),
        Err(deferred),
        "no fallback to iteration 0"
    );
}

#[tokio::test]
async fn save_failures_continue_defer_or_halt_by_kind() {
    let state = serde_json::json!({"n": 1});
    let passed = |harness: &Harness, store: &Arc<ScriptedCheckpoints>| {
        let journal = checkpointed(harness, 1, store);
        async move {
            journal.begin_iteration(0).expect("it0");
            journal.end_iteration(DRAIN, true).await.expect("passes");
            journal
        }
    };

    // An unavailable store or a lost acknowledgement only costs the
    // optimisation.
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    for transient in [
        IterationCheckpointError::Unavailable,
        IterationCheckpointError::AcknowledgementUnknown,
    ] {
        store.fail_saves(transient);
        let journal = passed(&harness, &store).await;
        assert_eq!(journal.save_checkpoint(1, &state, None).await, Ok(()));
        assert_eq!(journal.begin_iteration(1), Ok(()), "the loop goes on");
    }
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_CHECKPOINTS_TOTAL,
            &[("outcome", "unavailable")]
        ),
        2
    );

    // A conflict halts.
    store.fail_saves(IterationCheckpointError::Conflict);
    let journal = passed(&harness, &store).await;
    let halted = journal
        .save_checkpoint(1, &state, None)
        .await
        .expect_err("halts");
    assert!(halted.halts_execution(), "{halted:?}");

    // A lost lease defers: the real fence refuses a released lease.
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    let journal = passed(&harness, &store).await;
    harness
        .executions
        .release_lease(
            &harness.scope,
            &harness.execution_id.to_string(),
            harness.fencing,
        )
        .await
        .expect("released");
    let deferred = journal
        .save_checkpoint(1, &state, None)
        .await
        .expect_err("defers");
    assert_eq!(
        deferred,
        EffectExecutionError::IterationCheckpoint(IterationCheckpointError::ExecutionLeaseRejected)
    );
    assert!(deferred.is_deferred());

    // A state past the bound is not saved.
    let harness = Harness::new().await;
    let store = ScriptedCheckpoints::new(&harness);
    let journal = passed(&harness, &store).await;
    let huge = Value::String("x".repeat(MAX_ITERATION_CHECKPOINT_STATE_BYTES));
    assert_eq!(journal.save_checkpoint(1, &huge, None).await, Ok(()));
    assert_eq!(store.saves.load(Ordering::SeqCst), 0, "no save was tried");
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_CHECKPOINTS_TOTAL,
            &[("outcome", "oversize")]
        ),
        1
    );
}

#[tokio::test]
async fn an_attempt_without_a_store_neither_resumes_nor_saves() {
    let harness = Harness::new().await;
    let journal = harness.stateful_journal(1);
    assert_eq!(journal.resume_from_checkpoint().await, Ok(None));
    journal.begin_iteration(0).expect("it0");
    journal.end_iteration(DRAIN, true).await.expect("passes");
    assert_eq!(
        journal
            .save_checkpoint(1, &serde_json::json!({}), None)
            .await,
        Ok(())
    );
}
