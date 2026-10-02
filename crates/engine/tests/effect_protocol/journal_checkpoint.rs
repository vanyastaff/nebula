//! Iteration checkpoints of a journaled stateful action, end to end: where a
//! crash leaves the checkpoint, what the next attempt dispatches again (the
//! gateway's `started` counter) and what it sends (its calls), and how a
//! lost, stale, foreign-version, corrupt or unreachable checkpoint is met.

use nebula_storage_port::{
    CheckpointSaved, FencingToken, IterationCheckpoint, IterationCheckpointError,
    IterationCheckpointKey,
    store::{CheckpointStore, ExecutionStore},
};

use super::{
    faults::{CheckpointFault, FaultCheckpoints},
    journal_fixture::*,
    restart::{Backend, Database},
    *,
};

async fn stateful(ports: Ports) -> JournalFixture {
    JournalFixture::build(ports, Kind::Stateful, None).await
}

/// Starts an execution running one write in each of three iterations.
async fn start_three(fixture: &JournalFixture) -> nebula_core::ExecutionId {
    let units = [write("ck-a:1"), write("ck-b:2"), write("ck-c:3")];
    fixture
        .start_iterations(&[&units[0..1], &units[1..2], &units[2..3]], json!({}))
        .await
}

/// Kills a turn of `execution` as iteration `iteration` starts and readies
/// the store for another owner.
async fn crash_at_iteration(
    fixture: &mut JournalFixture,
    database: &mut Database,
    execution: nebula_core::ExecutionId,
    iteration: u32,
) {
    *fixture.gateway.iterations.hold_at.lock() = Some(iteration);
    fixture
        .crash_at(execution, &fixture.gateway.iterations.held)
        .await;
    fixture.ports = database.reconnect().await;
    database.expire_abandoned_leases().await;
}

/// The fixture action's version, as the engine binds its checkpoints.
pub(super) fn action_version(fixture: &JournalFixture) -> String {
    fixture
        .frozen
        .resolve_action(&action_key!("journal.charge"))
        .expect("the fixture's action")
        .metadata()
        .base()
        .version()
        .to_string()
}

/// The node's checkpoint row stored under `version`, if any.
pub(super) async fn stored(
    fixture: &JournalFixture,
    execution: nebula_core::ExecutionId,
    version: &str,
) -> Option<IterationCheckpoint> {
    let execution = execution.to_string();
    let key = IterationCheckpointKey::new(
        &fixture.scope,
        &execution,
        "charge",
        "journal.charge",
        version,
    )
    .expect("key");
    fixture
        .ports
        .stores
        .checkpoints
        .load_iteration_checkpoint(&key)
        .await
        .expect("loads")
}

/// Writes `checkpoint` under `version` as a short-lived owner of
/// `execution` would.
pub(super) async fn plant(
    fixture: &JournalFixture,
    execution: nebula_core::ExecutionId,
    version: &str,
    checkpoint: &IterationCheckpoint,
) {
    let id = execution.to_string();
    let fencing = fixture
        .ports
        .stores
        .execution
        .acquire_lease(
            &fixture.scope,
            &id,
            "planter",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .expect("the lease is free");
    let key = IterationCheckpointKey::new(&fixture.scope, &id, "charge", "journal.charge", version)
        .expect("key");
    fixture
        .ports
        .stores
        .checkpoints
        .save_iteration_checkpoint(&key, checkpoint, fencing)
        .await
        .expect("planted");
    assert!(
        fixture
            .ports
            .stores
            .execution
            .release_lease(&fixture.scope, &id, fencing)
            .await
            .unwrap()
    );
}

/// A checkpoint of `iteration` whose state is `state` and digest `digest`.
pub(super) fn checkpoint(
    iteration: u32,
    state: &Value,
    digest: Option<[u8; 32]>,
    attested: u32,
) -> IterationCheckpoint {
    use sha2::Digest as _;
    let bytes = serde_json::to_vec(state).unwrap();
    let digest = digest.unwrap_or_else(|| sha2::Sha256::digest(&bytes).into());
    IterationCheckpoint::new(iteration, bytes, digest, None, attested, 1).unwrap()
}

fn assert_node_error(result: &nebula_engine::ExecutionResult, code: &str) {
    assert_eq!(result.status, ExecutionStatus::Failed, "{result:?}");
    assert!(
        node_error(result).starts_with(code),
        "expected {code}: {}",
        node_error(result)
    );
}

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn every_passed_continue_barrier_records_the_next_iteration(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture).await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    let row = stored(&fixture, execution, &action_version(&fixture))
        .await
        .expect("a checkpoint after iteration 1");
    assert_eq!(row.iteration(), 2, "the `Break` iteration records none");
    assert_eq!(row.attested_positions(), 2, "it0 and it1's writes");
    assert!(row.fencing_generation() > 0);
    let state: Value = serde_json::from_slice(row.state()).unwrap();
    assert_eq!(state["next"], json!(2));
    assert_eq!(state["receipts"], json!([1, 2]));
}

/// A crash after iteration 1's barrier passed but before its checkpoint
/// was saved: the store answered nothing. The next attempt replays from the
/// last saved checkpoint (none here: iteration 0) and sends nothing twice.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_checkpoint_that_never_landed_falls_back_without_a_second_call(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture).await;
    let inner = Arc::clone(&fixture.ports.stores.checkpoints);
    let faulty = Arc::new(FaultCheckpoints::new(
        inner,
        CheckpointFault::SavesUnavailable,
    ));
    fixture.ports.stores.checkpoints = faulty.clone();
    crash_at_iteration(&mut fixture, &mut database, execution, 2).await;
    assert_eq!(
        faulty.saves.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "both saves were tried and the loop went on"
    );
    assert!(
        stored(&fixture, execution, &action_version(&fixture))
            .await
            .is_none()
    );

    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(fixture.gateway.started(), [0, 1, 2, 0, 1, 2]);
    assert_eq!(fixture.gateway.call_count(), 3, "no second provider call");
}

/// The checkpoint committed and its acknowledgement was lost: the loop went
/// on, and the next attempt resumes from it.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_lost_save_acknowledgement_still_resumes_from_the_row(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture).await;
    let inner = Arc::clone(&fixture.ports.stores.checkpoints);
    fixture.ports.stores.checkpoints =
        Arc::new(FaultCheckpoints::new(inner, CheckpointFault::SaveAckLost));
    crash_at_iteration(&mut fixture, &mut database, execution, 2).await;

    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(receipts(&result), json!([1, 2, 3]));
    assert_eq!(
        fixture.gateway.started(),
        [0, 1, 2, 2],
        "attested iterations never re-dispatched"
    );
    assert_eq!(fixture.gateway.call_count(), 3);
}

/// An unsettled slot of an attested iteration — throttled, nothing applied,
/// the program moved on — is never granted again: the resume starts past
/// it.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn an_attested_unsettled_slot_is_never_granted_again(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let units = [write("lower:1"), write("higher:2"), write("next:3")];
    let execution = fixture
        .start_iterations(&[&units[0..2], &units[2..3]], json!({ "swallow": true }))
        .await;
    *fixture.gateway.iterations.throttle_at.lock() = Some(0);
    crash_at_iteration(&mut fixture, &mut database, execution, 1).await;
    let recorded = fixture.slots(execution).await;
    assert_ne!(
        phase(&recorded[0]),
        EffectPhase::Resolved,
        "it0's throttled write"
    );
    let calls_before = fixture.gateway.call_count();

    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(fixture.gateway.started(), [0, 1, 1]);
    assert_eq!(
        fixture.gateway.call_count(),
        calls_before + 1,
        "only iteration 1's write"
    );
    let slots = fixture.slots(execution).await;
    assert_eq!(slots[0], recorded[0], "the attested slot is untouched");
}

/// A stale owner's checkpoint is refused by the fence: the lease moved to
/// another owner before the save, the node defers, nothing is written and no
/// later iteration runs.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_stale_owner_cannot_record_a_checkpoint(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture).await;
    fixture.ports.stores.checkpoints = Arc::new(TakeoverBeforeSave {
        inner: Arc::clone(&fixture.ports.stores.checkpoints),
        execution: Arc::clone(&fixture.ports.stores.execution),
        scope: fixture.scope.clone(),
        successor: parking_lot::Mutex::new(None),
    });
    let result = fixture.run(execution).await;
    assert!(
        matches!(
            result,
            Err(nebula_engine::EngineError::Effect(
                nebula_engine::EffectExecutionError::IterationCheckpoint(
                    IterationCheckpointError::ExecutionLeaseRejected
                )
            ))
        ),
        "a stale owner defers: {result:?}"
    );
    assert_eq!(fixture.gateway.started(), [0], "no iteration after it");
    assert!(
        stored(&fixture, execution, &action_version(&fixture))
            .await
            .is_none()
    );
}

/// Hands the lease to another owner just before the save it wraps.
#[derive(Debug)]
pub(super) struct TakeoverBeforeSave {
    pub inner: Arc<dyn CheckpointStore>,
    pub execution: Arc<dyn ExecutionStore>,
    pub scope: Scope,
    pub successor: parking_lot::Mutex<Option<FencingToken>>,
}

#[async_trait::async_trait]
impl CheckpointStore for TakeoverBeforeSave {
    async fn load_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
    ) -> Result<Option<IterationCheckpoint>, IterationCheckpointError> {
        self.inner.load_iteration_checkpoint(key).await
    }

    async fn save_iteration_checkpoint(
        &self,
        key: &IterationCheckpointKey<'_>,
        checkpoint: &IterationCheckpoint,
        fencing: FencingToken,
    ) -> Result<CheckpointSaved, IterationCheckpointError> {
        if self.successor.lock().is_none() {
            assert!(
                self.execution
                    .release_lease(&self.scope, key.execution_id(), fencing)
                    .await
                    .unwrap()
            );
            let successor = self
                .execution
                .acquire_lease(
                    &self.scope,
                    key.execution_id(),
                    "replacement-owner",
                    std::time::Duration::from_secs(30),
                )
                .await
                .unwrap()
                .expect("the successor takes the lease");
            assert_ne!(successor, fencing);
            *self.successor.lock() = Some(successor);
        }
        self.inner
            .save_iteration_checkpoint(key, checkpoint, fencing)
            .await
    }
}

/// A checkpoint another action version wrote is invisible: with no ledger
/// slot the execution starts afresh; with the earlier slots it replays them
/// from iteration 0 (a changed effect there would be a mismatch).
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_checkpoint_of_another_action_version_is_never_read(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    // Without slots.
    let fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture).await;
    let foreign = checkpoint(2, &json!({"next": 2, "receipts": [7, 8]}), None, 0);
    plant(&fixture, execution, "0.9.0", &foreign).await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(receipts(&result), json!([1, 2, 3]), "not the foreign state");
    assert_eq!(fixture.gateway.started(), [0, 1, 2]);

    // With the slots of an earlier attempt that saved no checkpoint of its
    // own; only an older version's row exists.
    let mut fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture).await;
    let inner = Arc::clone(&fixture.ports.stores.checkpoints);
    fixture.ports.stores.checkpoints = Arc::new(FaultCheckpoints::new(
        inner,
        CheckpointFault::SavesUnavailable,
    ));
    crash_at_iteration(&mut fixture, &mut database, execution, 2).await;
    assert_ne!(action_version(&fixture), "0.9.0");
    let foreign = checkpoint(2, &json!({"next": 2, "receipts": [7, 8]}), None, 2);
    plant(&fixture, execution, "0.9.0", &foreign).await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(fixture.gateway.started(), [0, 1, 2, 0, 1, 2], "from it0");
    assert_eq!(fixture.gateway.call_count(), 3, "replayed, not resent");
}

/// A row whose digest does not match its state halts the execution before
/// any iteration runs.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_corrupt_checkpoint_halts_with_nothing_run(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture).await;
    let corrupt = checkpoint(1, &json!({"next": 1, "receipts": [1]}), Some([0; 32]), 0);
    plant(&fixture, execution, &action_version(&fixture), &corrupt).await;
    let result = fixture.run(execution).await.unwrap();
    assert_node_error(&result, "ENGINE:EFFECT_ITERATION_CHECKPOINT");
    assert!(fixture.gateway.started().is_empty(), "no iteration ran");
    assert_eq!(fixture.gateway.call_count(), 0);
}

/// A checkpoint store that does not answer defers the node: nothing runs,
/// and it never starts over at iteration 0.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn an_unreachable_checkpoint_store_defers_the_node(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture).await;
    let inner = Arc::clone(&fixture.ports.stores.checkpoints);
    fixture.ports.stores.checkpoints = Arc::new(FaultCheckpoints::new(
        inner,
        CheckpointFault::LoadsUnavailable,
    ));
    let result = fixture.run(execution).await;
    assert!(
        matches!(
            result,
            Err(nebula_engine::EngineError::Effect(
                nebula_engine::EffectExecutionError::IterationCheckpoint(
                    IterationCheckpointError::Unavailable
                )
            ))
        ),
        "{result:?}"
    );
    assert!(fixture.gateway.started().is_empty());
    assert_eq!(fixture.gateway.call_count(), 0);
}

/// A node cancelled after a checkpoint keeps the row (rows go with their
/// execution, never cleared at terminal) and sends nothing further.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_cancellation_after_a_checkpoint_keeps_the_row(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture).await;
    *fixture.gateway.iterations.hold_at.lock() = Some(1);
    let engine = Arc::new(fixture.engine());
    let scope = fixture.scope.clone();
    let turn = tokio::spawn({
        let engine = Arc::clone(&engine);
        async move { engine.resume_execution(&scope, execution).await }
    });
    tokio::time::timeout(HANG_GUARD, fixture.gateway.iterations.held.notified())
        .await
        .expect("iteration 1 started");
    assert!(engine.cancel_execution(execution));
    let result = tokio::time::timeout(HANG_GUARD, turn)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Cancelled, "{result:?}");
    assert_eq!(fixture.gateway.call_count(), 1);
    let row = stored(&fixture, execution, &action_version(&fixture))
        .await
        .expect("the checkpoint after iteration 0 stays");
    assert_eq!(row.iteration(), 1);
}
