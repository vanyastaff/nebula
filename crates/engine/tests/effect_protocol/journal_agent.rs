//! A journaled agent's turns, end to end (experimental: journaled turns):
//! every turn asks a fake model — a recorded read whose answer changes on
//! every real call — and runs the tools the answer steers, all under
//! `turn{n}/unit/v1/#{k:06}`; every passed `Continue` barrier checkpoints the
//! turn state.
//!
//! The crash matrix kills a turn at every boundary of a two-turn run and
//! checks the recovery against the uncrashed run: each tool reaches the
//! provider once, or again under its recorded key only where a call was
//! granted and never explained; a recorded answer is never asked again; an
//! unrecorded one is asked again exactly once, at the same position; the
//! output is the uncrashed one; and the labels are the turn labels.
//!
//! Each test's controls live on its own fixture's gateway
//! ([`AgentControls`]): nothing is shared between concurrent tests.

use nebula_storage_port::IterationCheckpointError;

use super::{
    faults::{Boundary, CheckpointFault, Fault, FaultCheckpoints, FaultLedger, OutcomeGate},
    journal_checkpoint::{TakeoverBeforeSave, action_version, checkpoint, plant, stored},
    journal_fixture::*,
    journal_stateful::lose_checkpoints,
    restart::{Backend, Database},
    *,
};

/// A fixture whose node runs the journaled agent.
async fn agent(ports: Ports) -> JournalFixture {
    JournalFixture::build(ports, Kind::Agent, None).await
}

/// An idempotent tool of `request`, allowed two attempts: a call granted
/// and never explained may be granted again under its recorded key.
fn tool(request: &str) -> Value {
    json!({"request": request, "idempotent": true, "budget": 2})
}

/// Starts an execution of two turns, one tool each.
async fn start_two(fixture: &JournalFixture) -> nebula_core::ExecutionId {
    fixture
        .start_turns(&[&[tool("t0")], &[tool("t1")]], json!({}))
        .await
}

/// The output of the two-turn run, crashed or not.
fn two_turn_output() -> Value {
    json!({ "turns": 2, "receipts": [1, 2] })
}

/// The labels of the two-turn run: the model, then its tool, per turn.
const TWO_TURN_LABELS: [&str; 4] = [
    "turn0/unit/v1/#000000",
    "turn0/unit/v1/#000001",
    "turn1/unit/v1/#000000",
    "turn1/unit/v1/#000001",
];

/// The occurrence labels of `slots`, in preparation order.
fn labels(slots: &[nebula_storage_port::dto::EffectOccurrenceRecord]) -> Vec<&str> {
    slots
        .iter()
        .map(nebula_storage_port::dto::EffectOccurrenceRecord::occurrence)
        .collect()
}

/// The agent's output.
fn output(result: &nebula_engine::ExecutionResult) -> Value {
    result.node_outputs[&node_key!("charge")].clone()
}

/// The keys of the calls of the tool `request` (`{request}@{answer}`).
fn tool_keys(fixture: &JournalFixture, request: &str) -> Vec<Option<String>> {
    let prefix = format!("{request}@");
    fixture
        .gateway
        .calls
        .lock()
        .iter()
        .filter(|call| call.request.starts_with(&prefix))
        .map(|call| call.key.clone())
        .collect()
}

/// Asserts the failed node's durable error starts with `code`.
fn assert_node_error(result: &nebula_engine::ExecutionResult, code: &str) {
    assert_eq!(result.status, ExecutionStatus::Failed, "{result:?}");
    assert!(
        node_error(result).starts_with(code),
        "expected {code}: {}",
        node_error(result)
    );
}

/// Readies the store for another owner after a crash.
async fn after_crash(fixture: &mut JournalFixture, database: &mut Database) {
    fixture.ports = database.reconnect().await;
    database.expire_abandoned_leases().await;
}

// 1 ─────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn each_turn_journals_its_model_call_and_tools_under_its_label(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = agent(database.ports()).await;
    let execution = start_two(&fixture).await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(output(&result), two_turn_output());
    let slots = fixture.slots(execution).await;
    assert_eq!(labels(&slots), TWO_TURN_LABELS);
    assert!(
        slots
            .iter()
            .all(|slot| phase(slot) == EffectPhase::Resolved)
    );
    let observations: Vec<bool> = slots
        .iter()
        .map(|slot| slot.record().protocol().unwrap().is_observation())
        .collect();
    assert_eq!(
        observations,
        [true, false, true, false],
        "the model calls are recorded reads, the tools effects"
    );
    assert_eq!(fixture.gateway.model_calls(), 2);
    assert_eq!(fixture.gateway.call_count(), 2);
    assert_eq!(fixture.gateway.turns_started(), [0, 1]);
    let row = stored(&fixture, execution, &action_version(&fixture))
        .await
        .expect("a checkpoint after turn 0");
    assert_eq!(row.iteration(), 1, "the `Break` turn records none");
    assert_eq!(row.attested_positions(), 2, "turn 0's model call and tool");
}

// 2 ─────────────────────────────────────────────────────────────────────────

/// Where the crash matrix kills a turn of the two-turn run.
#[derive(Debug, Clone, Copy)]
enum CrashPoint {
    /// Turn 0 started; nothing prepared.
    BeforeModelPrepare,
    /// Turn 0's model call was granted and never explained.
    ModelGrantedUnexplained,
    /// Turn 0's model answered and its answer was being recorded: the
    /// program never saw it (record before return).
    ModelAnsweredBeforeRecorded,
    /// Turn 0's answer is recorded; no tool ran.
    ModelSettledBeforeTools,
    /// Turn 0's tool call was granted (and applied) and never explained.
    ToolGrantedUnexplained,
    /// Turn 0's tool settled; the turn had not returned.
    ToolSettledBeforeBarrier,
    /// Turn 0's barrier passed; its checkpoint was never saved.
    AfterBarrierBeforeCheckpoint,
    /// Turn 0's checkpoint was saved; turn 1 started.
    AfterCheckpoint,
}

#[rstest::rstest]
#[tokio::test]
async fn the_crash_matrix_recovers_to_the_uncrashed_run(
    #[values(Backend::Memory, Backend::Sqlite, Backend::Postgres)] backend: Backend,
    #[values(
        CrashPoint::BeforeModelPrepare,
        CrashPoint::ModelGrantedUnexplained,
        CrashPoint::ModelAnsweredBeforeRecorded,
        CrashPoint::ModelSettledBeforeTools,
        CrashPoint::ToolGrantedUnexplained,
        CrashPoint::ToolSettledBeforeBarrier,
        CrashPoint::AfterBarrierBeforeCheckpoint,
        CrashPoint::AfterCheckpoint
    )]
    point: CrashPoint,
) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = agent(database.ports()).await;
    let execution = start_two(&fixture).await;
    let gateway = Arc::clone(&fixture.gateway);
    let controls = &gateway.agent;
    match point {
        CrashPoint::BeforeModelPrepare => {
            *controls.hold_at.lock() = Some(0);
            fixture.crash_at(execution, &controls.held).await;
        },
        CrashPoint::ModelGrantedUnexplained => {
            *controls.hold_model_at.lock() = Some(0);
            fixture
                .crash_at(execution, &controls.model_gate.entered)
                .await;
        },
        CrashPoint::ModelAnsweredBeforeRecorded => {
            // The first outcome written is the model's answer: it waits at
            // the gate, never committed.
            let gate = Arc::new(OutcomeGate::default());
            let mut ledger =
                FaultLedger::new(fixture.ports.ledger.clone(), Boundary::Outcome, Fault::Hang);
            ledger.outcome_gate = Some(Arc::clone(&gate));
            fixture.ports.stores.operation_ledger = Arc::new(ledger);
            // The program is given time to go on with an answer it should
            // not have: none reaches it while the record is pending.
            let engine = fixture.engine();
            let scope = fixture.scope.clone();
            let turn =
                tokio::spawn(async move { engine.resume_execution(&scope, execution).await });
            tokio::time::timeout(HANG_GUARD, gate.entered.notified())
                .await
                .expect("the answer is being recorded");
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            turn.abort();
            let _ = turn.await;
        },
        CrashPoint::ModelSettledBeforeTools => {
            *controls.hold_after_model_at.lock() = Some(0);
            fixture.crash_at(execution, &controls.held).await;
        },
        CrashPoint::ToolGrantedUnexplained => {
            *controls.hold_tool_at.lock() = Some(0);
            fixture
                .crash_at(execution, &controls.tool_gate.entered)
                .await;
        },
        CrashPoint::ToolSettledBeforeBarrier => {
            *controls.hold_after_tools_at.lock() = Some(0);
            fixture.crash_at(execution, &controls.held).await;
        },
        CrashPoint::AfterBarrierBeforeCheckpoint => {
            let saves = Arc::new(FaultCheckpoints::new(
                Arc::clone(&fixture.ports.stores.checkpoints),
                CheckpointFault::SavesHang,
            ));
            fixture.ports.stores.checkpoints = saves.clone();
            fixture.crash_at(execution, &saves.save_entered).await;
        },
        CrashPoint::AfterCheckpoint => {
            *controls.hold_at.lock() = Some(1);
            fixture.crash_at(execution, &controls.held).await;
        },
    }
    let crashed = fixture.slots(execution).await;
    let model_calls = gateway.model_calls();
    let tool_calls = gateway.call_count();
    // What the crash left behind.
    let (left_labels, left_model, left_tool): (&[&str], u32, usize) = match point {
        CrashPoint::BeforeModelPrepare => (&[], 0, 0),
        CrashPoint::ModelGrantedUnexplained
        | CrashPoint::ModelAnsweredBeforeRecorded
        | CrashPoint::ModelSettledBeforeTools => (&TWO_TURN_LABELS[..1], 1, 0),
        CrashPoint::ToolGrantedUnexplained
        | CrashPoint::ToolSettledBeforeBarrier
        | CrashPoint::AfterBarrierBeforeCheckpoint
        | CrashPoint::AfterCheckpoint => (&TWO_TURN_LABELS[..2], 1, 1),
    };
    assert_eq!(labels(&crashed), left_labels, "{point:?}");
    assert_eq!(model_calls, left_model, "{point:?}");
    assert_eq!(tool_calls, left_tool, "{point:?}");
    if matches!(
        point,
        CrashPoint::ModelGrantedUnexplained | CrashPoint::ModelAnsweredBeforeRecorded
    ) {
        assert_eq!(
            phase(&crashed[0]),
            EffectPhase::InvocationOutstanding,
            "{point:?}: no answer recorded, and the program never saw one"
        );
    }
    if matches!(point, CrashPoint::ToolGrantedUnexplained) {
        assert_eq!(phase(&crashed[1]), EffectPhase::InvocationOutstanding);
    }
    let checkpointed = stored(&fixture, execution, &action_version(&fixture))
        .await
        .is_some();
    assert_eq!(
        checkpointed,
        matches!(point, CrashPoint::AfterCheckpoint),
        "{point:?}: only a passed barrier's checkpoint is saved"
    );

    after_crash(&mut fixture, &mut database).await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(
        result.status,
        ExecutionStatus::Completed,
        "{point:?}: {result:?}"
    );
    // (d) the uncrashed output.
    assert_eq!(output(&result), two_turn_output(), "{point:?}");
    // (e) the turn labels, at the positions the crash left.
    let slots = fixture.slots(execution).await;
    assert_eq!(labels(&slots), TWO_TURN_LABELS, "{point:?}");
    for (recovered, left) in slots.iter().zip(&crashed) {
        assert_eq!(
            recovered.record().operation().slot_id(),
            left.record().operation().slot_id(),
            "{point:?}: the recovery met the recorded slot, never a new one"
        );
    }
    assert!(
        slots
            .iter()
            .all(|slot| phase(slot) == EffectPhase::Resolved),
        "{point:?}"
    );
    // (b), (c) the model: asked again exactly once only where its answer was
    // never recorded, at the same position; never for a recorded answer.
    let asked_again = gateway.model_calls() - model_calls;
    let expected_asks = match point {
        CrashPoint::BeforeModelPrepare
        | CrashPoint::ModelGrantedUnexplained
        | CrashPoint::ModelAnsweredBeforeRecorded => 2,
        _ => 1,
    };
    assert_eq!(
        asked_again, expected_asks,
        "{point:?}: model calls after the crash"
    );
    // (a) each tool: once, or again under its recorded key only where its
    // call was granted and never explained; applied once either way.
    let t0 = tool_keys(&fixture, "t0");
    let t1 = tool_keys(&fixture, "t1");
    let expected_t0 = if matches!(point, CrashPoint::ToolGrantedUnexplained) {
        2
    } else {
        1
    };
    assert_eq!(t0.len(), expected_t0, "{point:?}: {t0:?}");
    assert!(
        t0.windows(2).all(|pair| pair[0] == pair[1]),
        "{point:?}: under one recorded key"
    );
    assert_eq!(t0[0].as_deref(), Some(recorded_key(&slots[1]).as_str()));
    assert_eq!(t1.len(), 1, "{point:?}");
    assert_eq!(gateway.applied(), 2, "{point:?}: each tool applied once");
    // Turn 0 runs again unless its checkpoint attests it.
    let expected_turns: &[u32] = match point {
        CrashPoint::AfterCheckpoint => &[0, 1, 1],
        _ => &[0, 0, 1],
    };
    assert_eq!(gateway.turns_started(), expected_turns, "{point:?}");
}

// 3 ─────────────────────────────────────────────────────────────────────────

/// With every checkpoint lost, a recovery replays the agent from turn 0
/// with no model call and no tool call for the recorded turns.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn a_lost_checkpoint_replays_from_turn_zero_without_a_call(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = agent(database.ports()).await;
    let execution = fixture
        .start_turns(&[&[tool("t0")], &[tool("t1")], &[tool("t2")]], json!({}))
        .await;
    *fixture.gateway.agent.hold_at.lock() = Some(2);
    fixture
        .crash_at(execution, &fixture.gateway.agent.held)
        .await;
    after_crash(&mut fixture, &mut database).await;
    lose_checkpoints(&mut fixture);
    assert_eq!(fixture.gateway.model_calls(), 2);
    assert_eq!(fixture.gateway.call_count(), 2);

    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(
        output(&result),
        json!({ "turns": 3, "receipts": [1, 2, 3] })
    );
    assert_eq!(fixture.gateway.turns_started(), [0, 1, 2, 0, 1, 2]);
    assert_eq!(
        fixture.gateway.model_calls(),
        3,
        "turns 0 and 1 replayed their recorded answers; turn 2 asked once"
    );
    assert_eq!(
        fixture.gateway.call_count(),
        3,
        "turns 0 and 1 replayed their tools; turn 2's sent once"
    );
}

// 4 ─────────────────────────────────────────────────────────────────────────

/// The model's answer is written and its acknowledgement lost: the exact
/// evidence is recommitted, and the answer the agent saw is the one
/// recorded.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn a_lost_answer_acknowledgement_is_recommitted_exactly(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = agent(database.ports()).await;
    let execution = start_two(&fixture).await;
    let ledger = Arc::new(FaultLedger::new(
        fixture.ports.ledger.clone(),
        Boundary::Outcome,
        Fault::Before,
    ));
    fixture.ports.stores.operation_ledger = ledger.clone();
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(output(&result), two_turn_output());
    assert_eq!(fixture.gateway.model_calls(), 2, "no answer asked twice");
    let attempts = ledger.outcome_attempts.lock().clone();
    assert_eq!(attempts.len(), 5, "the model's answer twice, then three");
    assert_eq!(attempts[0], attempts[1], "the exact answer recommitted");
    let answer: Value = serde_json::from_slice(attempts[0].payload()).unwrap();
    assert_eq!(answer["output"], json!(1), "the answer the agent saw");
}

// 5 ─────────────────────────────────────────────────────────────────────────

/// A changed prompt at a recorded model call is a mismatch: nothing is
/// asked or sent, and the execution halts.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn a_changed_prompt_is_a_mismatch_and_asks_nothing(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = agent(database.ports()).await;
    let execution = start_two(&fixture).await;
    *fixture.gateway.agent.hold_after_model_at.lock() = Some(0);
    fixture
        .crash_at(execution, &fixture.gateway.agent.held)
        .await;
    after_crash(&mut fixture, &mut database).await;
    *fixture.gateway.agent.prompt_override.lock() = Some("another prompt".to_owned());

    let result = fixture.run(execution).await.unwrap();
    assert_node_error(&result, "ENGINE:EFFECT_OCCURRENCE_MISMATCH");
    assert_eq!(fixture.gateway.model_calls(), 1, "not asked again");
    assert_eq!(fixture.gateway.call_count(), 0, "no tool sent");
    assert_eq!(fixture.slots(execution).await.len(), 1);
}

// 6 ─────────────────────────────────────────────────────────────────────────

/// Tools steered by a plain `Read` (a clock) instead of a recorded read
/// diverge on replay: the changed tool request at its recorded position is
/// a mismatch, nothing sent.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn a_plain_read_that_steers_the_tools_diverges_and_halts(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = agent(database.ports()).await;
    let execution = fixture
        .start_turns(
            &[&[tool("t0")], &[tool("t1")]],
            json!({ "steer_by_read": true }),
        )
        .await;
    *fixture.gateway.agent.hold_after_tools_at.lock() = Some(0);
    fixture
        .crash_at(execution, &fixture.gateway.agent.held)
        .await;
    after_crash(&mut fixture, &mut database).await;
    assert_eq!(fixture.gateway.call_count(), 1);

    let result = fixture.run(execution).await.unwrap();
    assert_node_error(&result, "ENGINE:EFFECT_OCCURRENCE_MISMATCH");
    assert_eq!(fixture.gateway.call_count(), 1, "nothing sent again");
    assert_eq!(fixture.gateway.model_calls(), 1, "the answer replayed");
}

// 7 ─────────────────────────────────────────────────────────────────────────

/// Two tools the model listed, joined: the process dies with one applied
/// and the other granted and never explained. The recovery sends the
/// unexplained one again under its recorded key — at least once — and
/// replays the applied one; neither is ever applied in reverse.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn joined_tools_recover_in_model_order_under_their_recorded_keys(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = agent(database.ports()).await;
    let execution = fixture
        .start_turns(&[&[tool("ta"), tool("tb")]], json!({ "concurrent": true }))
        .await;
    *fixture.gateway.agent.hold_tool_at.lock() = Some(0);
    let engine = fixture.engine();
    let scope = fixture.scope.clone();
    let turn = tokio::spawn(async move { engine.resume_execution(&scope, execution).await });
    tokio::time::timeout(
        HANG_GUARD,
        fixture.gateway.agent.tool_gate.entered.notified(),
    )
    .await
    .expect("a tool call reached the provider and waits");
    // The other tool settles.
    tokio::time::timeout(HANG_GUARD, async {
        loop {
            let settled = fixture
                .slots(execution)
                .await
                .iter()
                .filter(|slot| {
                    phase(slot) == EffectPhase::Resolved
                        && !slot.record().protocol().unwrap().is_observation()
                })
                .count();
            if settled == 1 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the other tool settled");
    turn.abort();
    let _ = turn.await;
    let crashed = fixture.slots(execution).await;
    assert_eq!(labels(&crashed).len(), 3, "the model and both tools");
    let held = crashed
        .iter()
        .find(|slot| phase(slot) == EffectPhase::InvocationOutstanding)
        .expect("the held tool's call is outstanding");
    let held_key = recorded_key(held);
    // The later-positioned tool lists the earlier one as concurrent: their
    // order was the model's, not the program's.
    assert_eq!(
        crashed[2].record().protocol().unwrap().concurrent_with(),
        Some(&[nebula_storage_port::dto::PositionRange::new(1, 1).unwrap()][..])
    );

    after_crash(&mut fixture, &mut database).await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(fixture.gateway.applied(), 2, "each tool applied once");
    let held_calls = fixture
        .gateway
        .call_keys()
        .into_iter()
        .filter(|key| key.as_deref() == Some(held_key.as_str()))
        .count();
    assert_eq!(held_calls, 2, "sent again under its recorded key");
    assert_eq!(fixture.gateway.model_calls(), 1, "the answer replayed");
    assert!(
        fixture
            .slots(execution)
            .await
            .iter()
            .all(|slot| phase(slot) == EffectPhase::Resolved)
    );
}

// 8 ─────────────────────────────────────────────────────────────────────────

/// A turn past its timeout with its model call in flight: the turn is
/// abandoned, the call settles ambiguous (no answer, never unknown), the
/// node fails retryably, and its retry replays the turn — asking the model
/// again once, at the same position — and completes.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn a_turn_past_its_timeout_is_retried_and_asks_again_at_its_position(
    #[case] backend: Backend,
) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = JournalFixture::build(
        database.ports(),
        Kind::TimedAgent,
        Some(nebula_workflow::RetryConfig::fixed(3, 1)),
    )
    .await;
    let model_deadline = AGENT_TURN_TIMEOUT + std::time::Duration::from_secs(3);
    let execution = fixture
        .start_turns(
            &[&[tool("t0")]],
            json!({ "model_deadline_ms": model_deadline.as_millis() }),
        )
        .await;
    *fixture.gateway.agent.hang_model_at.lock() = Some(0);
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(output(&result), json!({ "turns": 1, "receipts": [1] }));
    assert_eq!(fixture.gateway.turns_started(), [0, 0], "the turn replayed");
    assert_eq!(fixture.gateway.model_calls(), 2, "asked again once");
    let prompts = fixture.gateway.agent.prompts.lock().clone();
    assert_eq!(prompts[0], prompts[1], "the same request");
    let slots = fixture.slots(execution).await;
    assert_eq!(
        labels(&slots),
        ["turn0/unit/v1/#000000", "turn0/unit/v1/#000001"],
        "the same position, never a new one"
    );
    assert_eq!(fixture.gateway.call_count(), 1, "the tool once");
}

// 9 ─────────────────────────────────────────────────────────────────────────

/// A cancellation mid-turn stays cancelled: nothing further is asked or
/// sent.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn cancellation_mid_turn_sends_nothing_further(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = agent(database.ports()).await;
    let execution = start_two(&fixture).await;
    *fixture.gateway.agent.hold_at.lock() = Some(1);
    let engine = Arc::new(fixture.engine());
    let scope = fixture.scope.clone();
    let turn = tokio::spawn({
        let engine = Arc::clone(&engine);
        async move { engine.resume_execution(&scope, execution).await }
    });
    tokio::time::timeout(HANG_GUARD, fixture.gateway.agent.held.notified())
        .await
        .expect("turn 1 started");
    assert!(engine.cancel_execution(execution), "the loop is live here");
    let result = tokio::time::timeout(HANG_GUARD, turn)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Cancelled, "{result:?}");
    assert_eq!(fixture.gateway.model_calls(), 1);
    assert_eq!(fixture.gateway.call_count(), 1);
    assert_eq!(
        labels(&fixture.slots(execution).await),
        TWO_TURN_LABELS[..2]
    );
}

// 10 ────────────────────────────────────────────────────────────────────────

/// A tool whose outcome is unknown (an opaque write whose answer was lost)
/// halts the agent: no later turn runs, nothing more is asked or sent.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[case::postgres(Backend::Postgres)]
#[tokio::test]
async fn an_unknown_tool_outcome_halts_the_agent(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = agent(database.ports()).await;
    let execution = fixture
        .start_turns(&[&[write("t0")], &[write("t1")]], json!({}))
        .await;
    *fixture.gateway.agent.lose_tool_at.lock() = Some(0);
    let result = fixture.run(execution).await.unwrap();
    assert_node_error(&result, "ENGINE:EFFECT_OUTCOME_UNKNOWN");
    assert_eq!(fixture.gateway.turns_started(), [0], "turn 1 never started");
    assert_eq!(fixture.gateway.model_calls(), 1);
    assert_eq!(fixture.gateway.call_count(), 1);
}

// 11 ────────────────────────────────────────────────────────────────────────

/// A stale owner cannot record a turn checkpoint: the node defers and no
/// later turn runs.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_stale_owner_cannot_record_a_turn_checkpoint(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = agent(database.ports()).await;
    let execution = start_two(&fixture).await;
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
    assert_eq!(fixture.gateway.turns_started(), [0], "no turn after it");
    assert!(
        stored(&fixture, execution, &action_version(&fixture))
            .await
            .is_none()
    );
}

/// A turn checkpoint another action version wrote is never read: the agent
/// starts at turn 0.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_turn_checkpoint_of_another_action_version_is_never_read(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = agent(database.ports()).await;
    let execution = start_two(&fixture).await;
    assert_ne!(action_version(&fixture), "0.9.0");
    let foreign = checkpoint(
        1,
        &json!({ "script": {"turns": []}, "turn": 1, "receipts": [7] }),
        None,
        0,
    );
    plant(&fixture, execution, "0.9.0", &foreign).await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(output(&result), two_turn_output(), "not the foreign state");
    assert_eq!(fixture.gateway.turns_started(), [0, 1]);
}

// 12 ────────────────────────────────────────────────────────────────────────

/// A stateful iteration's recorded read replays its answer too: a recovery
/// with its checkpoints lost replays iteration 0 without asking the model
/// again, and its write stays the one the answer steered.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_stateful_iteration_replays_its_recorded_answer(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = JournalFixture::build(database.ports(), Kind::Stateful, None).await;
    let units = [write("it-a"), write("it-b")];
    let execution = fixture
        .start_iterations(&[&units[0..1], &units[1..2]], json!({ "ask": true }))
        .await;
    *fixture.gateway.iterations.hold_at.lock() = Some(1);
    fixture
        .crash_at(execution, &fixture.gateway.iterations.held)
        .await;
    after_crash(&mut fixture, &mut database).await;
    lose_checkpoints(&mut fixture);
    assert_eq!(fixture.gateway.model_calls(), 1);

    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(receipts(&result), json!([1, 2]));
    assert_eq!(fixture.gateway.started(), [0, 1, 0, 1]);
    assert_eq!(
        fixture.gateway.model_calls(),
        2,
        "iteration 0's answer replayed; iteration 1 asked once"
    );
    assert_eq!(fixture.gateway.call_count(), 2, "it-a sent once");
    assert_eq!(
        labels(&fixture.slots(execution).await),
        [
            "it0/unit/v1/#000000",
            "it0/unit/v1/#000001",
            "it1/unit/v1/#000000",
            "it1/unit/v1/#000001"
        ]
    );
}
