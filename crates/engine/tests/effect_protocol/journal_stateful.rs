//! A journaled stateful action's effects, per iteration: every iteration's
//! units are recorded under `it{n}/unit/v1/#{k:06}`, every node attempt
//! replays the iterations from the first, and the iteration barrier stops
//! the loop at the first effect the journal cannot vouch for.
//!
//! Each test's controls live on its own fixture's gateway
//! ([`IterationControls`]): nothing is shared between concurrent tests.

use super::{
    faults::{Boundary, Fault, FaultLedger},
    journal_fixture::*,
    restart::{Backend, Database},
    *,
};

/// A fixture whose node runs the journaled stateful action.
async fn stateful(ports: Ports) -> JournalFixture {
    JournalFixture::build(ports, Kind::Stateful, None).await
}

/// The occurrence labels of `slots`, in preparation order.
fn labels(slots: &[nebula_storage_port::dto::EffectOccurrenceRecord]) -> Vec<&str> {
    slots
        .iter()
        .map(nebula_storage_port::dto::EffectOccurrenceRecord::occurrence)
        .collect()
}

/// Three iterations of one write each.
const ITERATIONS: [&str; 3] = ["it-a:1", "it-b:2", "it-c:3"];

/// Starts an execution running one write of each of [`ITERATIONS`], with
/// `extra` script flags.
async fn start_three(fixture: &JournalFixture, extra: Value) -> nebula_core::ExecutionId {
    let units: Vec<Value> = ITERATIONS.iter().map(|request| write(request)).collect();
    fixture
        .start_iterations(&[&units[0..1], &units[1..2], &units[2..3]], extra)
        .await
}

/// Runs a turn of `execution` until iteration `iteration` starts, kills it
/// the way a crashed process would and readies the store for another
/// owner.
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

/// Asserts the failed node's durable error starts with `code`.
fn assert_node_error(result: &nebula_engine::ExecutionResult, code: &str) {
    assert_eq!(result.status, ExecutionStatus::Failed, "{result:?}");
    assert!(
        node_error(result).starts_with(code),
        "expected {code}: {}",
        node_error(result)
    );
}

// 1 ─────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn each_iteration_journals_its_effect_under_its_own_label(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture, json!({})).await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(receipts(&result), json!([1, 2, 3]));
    let slots = fixture.slots(execution).await;
    assert_eq!(
        labels(&slots),
        [
            "it0/unit/v1/#000000",
            "it1/unit/v1/#000000",
            "it2/unit/v1/#000000"
        ],
        "one label per iteration, the ordinal restarting in each"
    );
    assert!(
        slots
            .iter()
            .all(|slot| phase(slot) == EffectPhase::Resolved)
    );
    assert_eq!(fixture.gateway.call_count(), 3);
    assert_eq!(fixture.gateway.started(), [0, 1, 2]);
}

// 2 ─────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_resume_replays_settled_iterations_and_sends_the_next_once(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture, json!({})).await;
    // The process dies once iteration 1 settled, as iteration 2 starts.
    crash_at_iteration(&mut fixture, &mut database, execution, 2).await;
    let settled = fixture.slots(execution).await;
    assert_eq!(
        labels(&settled),
        ["it0/unit/v1/#000000", "it1/unit/v1/#000000"]
    );
    assert_eq!(fixture.gateway.call_count(), 2);

    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(receipts(&result), json!([1, 2, 3]), "the recorded outputs");
    assert_eq!(
        fixture.gateway.call_count(),
        3,
        "iterations 0 and 1 replayed without a call; iteration 2 sent once"
    );
    assert_eq!(fixture.gateway.started(), [0, 1, 2, 0, 1, 2], "from it0");
    let slots = fixture.slots(execution).await;
    assert_eq!(&slots[..2], &settled[..], "replay writes nothing");
    assert_eq!(slots[2].occurrence(), "it2/unit/v1/#000000");
    assert_eq!(phase(&slots[2]), EffectPhase::Resolved);
}

// 3 ─────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_call_granted_and_never_explained_in_an_iteration(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    for idempotent in [false, true] {
        let mut fixture = stateful(database.ports()).await;
        let units = [
            write("it-a:1"),
            json!({"request": "it-b:2", "idempotent": idempotent, "budget": 2}),
            write("it-c:3"),
        ];
        let execution = fixture
            .start_iterations(&[&units[0..1], &units[1..2], &units[2..3]], json!({}))
            .await;
        // Iteration 1's call reaches the provider; the process dies with it
        // granted and never explained.
        *fixture.gateway.iterations.hold_call_at.lock() = Some(1);
        let gate = Arc::clone(&fixture.gateway.iterations.call_gate);
        fixture.crash_at(execution, &gate.entered).await;
        assert_eq!(
            phase(&fixture.slots(execution).await[1]),
            EffectPhase::InvocationOutstanding
        );
        fixture.ports = database.reconnect().await;
        database.expire_abandoned_leases().await;
        let result = fixture.run(execution).await.unwrap();
        let slots = fixture.slots(execution).await;
        if idempotent {
            // A stable key is granted again within its window, under the
            // same key; the provider deduplicates.
            assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
            assert_eq!(receipts(&result), json!([1, 2, 3]));
            let keys = fixture.gateway.call_keys();
            assert_eq!(keys.len(), 4, "it0, it1 twice, it2");
            assert_eq!(keys[1], keys[2], "it1 again under its recorded key");
            assert_eq!(fixture.gateway.applied(), 3);
            assert!(
                slots
                    .iter()
                    .all(|slot| phase(slot) == EffectPhase::Resolved)
            );
        } else {
            // An opaque write's outcome is unknown: the node halts, and no
            // later iteration sends anything.
            assert_node_error(&result, "ENGINE:EFFECT_OUTCOME_UNKNOWN");
            assert_eq!(fixture.gateway.call_count(), 2, "it2 never sent");
            assert_eq!(slots.len(), 2);
            assert_eq!(phase(&slots[1]), EffectPhase::OutcomeUnknown);
            assert_eq!(fixture.gateway.started(), [0, 1, 0, 1]);
        }
    }
}

// 4 ─────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_lost_outcome_acknowledgement_in_an_iteration_is_recommitted_exactly(
    #[case] backend: Backend,
) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture, json!({})).await;
    // Iteration 1's outcome write is lost before it lands.
    let ledger = Arc::new(
        FaultLedger::new(
            fixture.ports.ledger.clone(),
            Boundary::Outcome,
            Fault::Before,
        )
        .skipping(1),
    );
    fixture.ports.stores.operation_ledger = ledger.clone();
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(receipts(&result), json!([1, 2, 3]));
    assert_eq!(fixture.gateway.call_count(), 3, "no call repeated");
    let attempts = ledger.outcome_attempts.lock().clone();
    assert_eq!(attempts.len(), 4, "it0, it1 twice, it2");
    assert_eq!(attempts[1], attempts[2], "the exact evidence recommitted");
    assert!(
        fixture
            .slots(execution)
            .await
            .iter()
            .all(|slot| phase(slot) == EffectPhase::Resolved)
    );
}

// 5 ─────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_changed_request_in_a_replayed_iteration_is_a_mismatch_with_no_call(
    #[case] backend: Backend,
) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture, json!({})).await;
    crash_at_iteration(&mut fixture, &mut database, execution, 2).await;
    let settled = fixture.slots(execution).await;

    // A non-deterministic iteration 1: its replay asks for another request.
    *fixture.gateway.iterations.override_at.lock() = Some((1, "it-b:changed".to_owned()));
    let result = fixture.run(execution).await.unwrap();
    assert_node_error(&result, "ENGINE:EFFECT_OCCURRENCE_MISMATCH");
    assert_eq!(fixture.gateway.call_count(), 2, "no call on replay");
    assert_eq!(fixture.slots(execution).await, settled);
    assert_eq!(
        fixture.gateway.started(),
        [0, 1, 2, 0, 1],
        "no iteration after the mismatch"
    );
}

// 6 ─────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_new_effect_in_an_iteration_an_earlier_attempt_finished_is_a_gap(
    #[case] backend: Backend,
) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture, json!({})).await;
    // The first attempt reached iteration 1 (and settled it).
    crash_at_iteration(&mut fixture, &mut database, execution, 2).await;
    let settled = fixture.slots(execution).await;

    // The resumed iteration 0 submits one more write after its own: a
    // fresh position below the recorded iteration 1.
    *fixture.gateway.iterations.extra_at.lock() = Some(0);
    let result = fixture.run(execution).await.unwrap();
    assert_node_error(&result, "ENGINE:EFFECT_OCCURRENCE_MISMATCH");
    assert_eq!(fixture.gateway.call_count(), 2, "nothing sent");
    assert_eq!(fixture.slots(execution).await, settled, "nothing prepared");
    assert_eq!(
        fixture.gateway.started(),
        [0, 1, 2, 0],
        "no iteration after the gap"
    );
}

// 7 ─────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_replay_that_ends_before_a_settled_iteration(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    for succeeds in [true, false] {
        let mut fixture = JournalFixture::build_with(
            database.ports(),
            Kind::Stateful,
            None,
            nebula_workflow::ErrorStrategy::IgnoreErrors,
        )
        .await;
        let execution = start_three(&fixture, json!({})).await;
        crash_at_iteration(&mut fixture, &mut database, execution, 2).await;
        let settled = fixture.slots(execution).await;

        // The replay ends at iteration 1, before its settled write:
        // completing, or failing.
        let control = if succeeds {
            &fixture.gateway.iterations.break_at
        } else {
            &fixture.gateway.iterations.fail_at
        };
        *control.lock() = Some(1);
        let result = fixture.run(execution).await.unwrap();
        assert_eq!(
            result.status,
            ExecutionStatus::Failed,
            "succeeds={succeeds}: {result:?}"
        );
        if succeeds {
            // Its success would stand for an attempt that never applied the
            // settled write: a mismatch.
            assert_node_error(&result, "ENGINE:EFFECT_OCCURRENCE_MISMATCH");
        } else {
            // Its own failure stands, and no error strategy routes past it.
            assert!(
                !node_error(&result).starts_with("ENGINE:EFFECT_OCCURRENCE_MISMATCH"),
                "{}",
                node_error(&result)
            );
        }
        assert!(
            !result.node_outputs.contains_key(&node_key!("charge")),
            "succeeds={succeeds}: no output stands for the node"
        );
        assert_eq!(fixture.gateway.call_count(), 2, "succeeds={succeeds}");
        assert_eq!(fixture.slots(execution).await, settled);
    }
}

// 7b ────────────────────────────────────────────────────────────────────────

/// The replay of a poller: an unjournaled read now answers otherwise, so the
/// iteration that recorded a write sends nothing and a later one would send
/// the same write at a fresh position, under another provider key. The
/// barrier of the iteration that passed the recorded write by stops the
/// loop: no second provider call.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_replay_that_passes_a_recorded_iteration_by_never_sends_it_again(
    #[case] backend: Backend,
) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let charge = write("poll-charge:1");
    let execution = fixture
        .start_iterations(
            &[
                &[],
                std::slice::from_ref(&charge),
                std::slice::from_ref(&charge),
            ],
            json!({}),
        )
        .await;
    // Iteration 1 records the charge; the process dies as iteration 2
    // starts.
    crash_at_iteration(&mut fixture, &mut database, execution, 2).await;
    let settled = fixture.slots(execution).await;
    assert_eq!(labels(&settled), ["it1/unit/v1/#000000"]);

    // The replay's iteration 1 sends nothing.
    *fixture.gateway.iterations.skip_at.lock() = Some(1);
    let result = fixture.run(execution).await.unwrap();
    assert_node_error(&result, "ENGINE:EFFECT_OCCURRENCE_MISMATCH");
    assert_eq!(fixture.gateway.call_count(), 1, "no second provider call");
    assert_eq!(
        fixture.gateway.started(),
        [0, 1, 2, 0, 1],
        "iteration 2 never ran again"
    );
    assert_eq!(fixture.slots(execution).await, settled, "nothing prepared");
}

/// Within one iteration: the replay submits fewer of the iteration's writes
/// than the earlier attempt recorded. Its barrier stops the loop.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_replay_that_skips_a_recorded_effect_within_an_iteration_stops(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let units = [write("in-it0:1"), write("in-it0:2"), write("in-it1:3")];
    let execution = fixture
        .start_iterations(&[&units[0..2], &units[2..3]], json!({}))
        .await;
    crash_at_iteration(&mut fixture, &mut database, execution, 1).await;
    let settled = fixture.slots(execution).await;
    assert_eq!(
        labels(&settled),
        ["it0/unit/v1/#000000", "it0/unit/v1/#000001"]
    );

    *fixture.gateway.iterations.keep_at.lock() = Some((0, 1));
    let result = fixture.run(execution).await.unwrap();
    assert_node_error(&result, "ENGINE:EFFECT_OCCURRENCE_MISMATCH");
    assert_eq!(fixture.gateway.call_count(), 2, "nothing sent again");
    assert_eq!(fixture.gateway.started(), [0, 1, 0], "it1 never ran again");
    assert_eq!(fixture.slots(execution).await, settled);
}

// 7c ────────────────────────────────────────────────────────────────────────

/// A lower effect's prepare commits but its answer never comes back, and the
/// process dies: the higher effect was never sent, and the recovery runs
/// both in the program's order, each once.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_prepare_whose_answer_was_lost_is_recovered_in_order(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let units = [write("lower:1"), write("higher:2")];
    let execution = fixture.start_iterations(&[&units[..]], json!({})).await;
    let ledger = Arc::new(FaultLedger::new(
        fixture.ports.ledger.clone(),
        Boundary::Prepare,
        Fault::AnswerLost,
    ));
    fixture.ports.stores.operation_ledger = ledger.clone();
    fixture.crash_at(execution, &ledger.answer_lost).await;
    assert_eq!(fixture.slots(execution).await.len(), 1, "the row committed");
    assert_eq!(fixture.gateway.call_count(), 0, "nothing sent");

    fixture.ports = database.reconnect().await;
    database.expire_abandoned_leases().await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(receipts(&result), json!([1, 2]));
    let requests: Vec<String> = fixture
        .gateway
        .calls
        .lock()
        .iter()
        .map(|call| call.request.clone())
        .collect();
    assert_eq!(requests, ["lower:1", "higher:2"], "in order, each once");
}

/// A lower effect changed nothing (throttled, its error swallowed) while a
/// higher one applied; the process dies. A recovery reaching the lower one
/// again would apply it after the higher one: it is refused unsent as
/// superseded — the same failure the program already swallowed — the higher
/// one replays its recorded outcome, and the node goes on to its next
/// iteration and completes.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_recovery_never_applies_a_lower_effect_after_a_higher_applied_one(
    #[case] backend: Backend,
) {
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
    assert_eq!(recorded.len(), 2);
    assert_eq!(phase(&recorded[1]), EffectPhase::Resolved);
    assert_eq!(fixture.gateway.call_count(), 2, "throttled, then applied");

    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    let requests: Vec<String> = fixture
        .gateway
        .calls
        .lock()
        .iter()
        .map(|call| call.request.clone())
        .collect();
    assert_eq!(
        requests,
        ["lower:1", "higher:2", "next:3"],
        "the lower one never resent, the higher one replayed"
    );
    let receipts = receipts(&result);
    assert_eq!(receipts[0]["sent"], json!("not_sent"), "{receipts}");
    assert_eq!(&receipts.as_array().unwrap()[1..], [json!(1), json!(2)]);
    assert_eq!(fixture.slots(execution).await[..2], recorded[..]);
}

// 7d ────────────────────────────────────────────────────────────────────────

/// Two writes awaited together (`join_all`): one is prepared and its grant
/// never answers, the other applies, and the process dies. The program
/// never ordered them — the applied one was prepared while the other was
/// still open, and records it as concurrent — so the recovery
/// sends the unsent one under its recorded key, once, and the node
/// completes.
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn concurrent_writes_recover_after_a_crash_with_one_applied(#[case] backend: Backend) {
    let Some(mut database) = Database::open(backend).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let units = [write("joined-a:1"), write("joined-b:2")];
    let execution = fixture
        .start_iterations(&[&units[..]], json!({ "concurrent": true }))
        .await;
    // The first grant never answers; the other write applies.
    let ledger = Arc::new(FaultLedger::new(
        fixture.ports.ledger.clone(),
        Boundary::Grant,
        Fault::Hang,
    ));
    fixture.ports.stores.operation_ledger = ledger.clone();
    fixture.crash_at(execution, &ledger.recorded).await;
    let crashed = fixture.slots(execution).await;
    assert_eq!(crashed.len(), 2, "both prepared");
    let phases: Vec<EffectPhase> = crashed.iter().map(phase).collect();
    assert!(
        phases.contains(&EffectPhase::Prepared) && phases.contains(&EffectPhase::Resolved),
        "{phases:?}"
    );
    let applied = crashed
        .iter()
        .find(|slot| phase(slot) == EffectPhase::Resolved)
        .unwrap();
    if applied.occurrence() == "it0/unit/v1/#000001" {
        assert_eq!(
            applied.record().protocol().unwrap().concurrent_with(),
            Some(&[0][..]),
            "the other write was open when it was prepared"
        );
    }
    let unsent = crashed
        .iter()
        .find(|slot| phase(slot) == EffectPhase::Prepared)
        .unwrap();
    let unsent_key = recorded_key(unsent);
    assert_eq!(fixture.gateway.call_count(), 1);

    fixture.ports = database.reconnect().await;
    database.expire_abandoned_leases().await;
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(fixture.gateway.call_count(), 2, "the unsent one, once");
    assert_eq!(fixture.gateway.applied(), 2);
    assert_eq!(
        fixture.gateway.call_keys()[1].as_deref(),
        Some(unsent_key.as_str()),
        "under its recorded key"
    );
}

// 7e ────────────────────────────────────────────────────────────────────────

/// An hourly poller dies as its fourth iteration starts. The recovery
/// replays the three settled iterations without waiting their delays again
/// — those iterations already ran, an hour apart — and waits only the delay
/// at its frontier, before the fresh iteration. (Paused time; no hang guard,
/// since virtual hours pass.)
#[tokio::test(start_paused = true)]
async fn a_recovery_does_not_wait_again_for_delays_it_already_waited() {
    let Some(mut database) = Database::open(Backend::Memory).await else {
        return;
    };
    let mut fixture = stateful(database.ports()).await;
    let units: Vec<Value> = (0..4).map(|n| write(&format!("hourly:{n}"))).collect();
    let execution = fixture
        .start_iterations(
            &[&units[0..1], &units[1..2], &units[2..3], &units[3..4]],
            json!({ "delay_secs": 3600 }),
        )
        .await;
    *fixture.gateway.iterations.hold_at.lock() = Some(3);
    let engine = fixture.engine();
    let scope = fixture.scope.clone();
    let started = tokio::time::Instant::now();
    let turn = tokio::spawn(async move { engine.resume_execution(&scope, execution).await });
    fixture.gateway.iterations.held.notified().await;
    assert!(
        started.elapsed() >= std::time::Duration::from_hours(3),
        "the first run waited between its iterations"
    );
    turn.abort();
    let _ = turn.await;
    assert_eq!(fixture.gateway.call_count(), 3);

    fixture.ports = database.reconnect().await;
    database.expire_abandoned_leases().await;
    let started = tokio::time::Instant::now();
    let result = fixture
        .engine()
        .resume_execution(&fixture.scope, execution)
        .await
        .unwrap();
    let elapsed = started.elapsed();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(fixture.gateway.call_count(), 4, "the replay sent nothing");
    assert!(
        elapsed >= std::time::Duration::from_hours(1)
            && elapsed < std::time::Duration::from_hours(2),
        "only the frontier's delay was waited: {elapsed:?}"
    );
}

// 8 ─────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn an_unknown_outcome_the_action_swallowed_stops_the_iterations(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture, json!({ "swallow": true })).await;
    // Iteration 1's answer is lost; the action swallows the unit's error and
    // would go on to iteration 2.
    *fixture.gateway.iterations.lose_at.lock() = Some(1);
    let result = fixture.run(execution).await.unwrap();
    assert_node_error(&result, "ENGINE:EFFECT_OUTCOME_UNKNOWN");
    assert_eq!(fixture.gateway.call_count(), 2, "it2 never sent");
    assert_eq!(fixture.gateway.started(), [0, 1], "it2 never started");
    let slots = fixture.slots(execution).await;
    assert_eq!(slots.len(), 2);
    assert_eq!(phase(&slots[1]), EffectPhase::OutcomeUnknown);
}

// 9 ─────────────────────────────────────────────────────────────────────────

/// A unit an iteration detached holds the next iteration back until it
/// settles: nothing of one iteration crosses into the next. (A unit still in
/// flight past the drain limit fails the barrier
/// `ENGINE:EFFECT_ITERATION_BARRIER`, or the node unknown when the unit had
/// been granted a call; the journal's unit tests cover it, as the limit is
/// minutes long here.)
#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_detached_unit_holds_the_next_iteration_at_the_barrier(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture, json!({})).await;
    *fixture.gateway.iterations.leak_at.lock() = Some(1);
    *fixture.gateway.iterations.hold_call_at.lock() = Some(1);
    let gate = Arc::clone(&fixture.gateway.iterations.call_gate);
    let engine = fixture.engine();
    let scope = fixture.scope.clone();
    let turn = tokio::spawn(async move { engine.resume_execution(&scope, execution).await });
    tokio::time::timeout(HANG_GUARD, gate.entered.notified())
        .await
        .expect("the detached unit's call reached the provider");
    // Iteration 1 returned without its unit; iteration 2 waits for it.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(fixture.gateway.started(), [0, 1]);
    assert!(!turn.is_finished());
    gate.release.notify_one();
    let result = tokio::time::timeout(HANG_GUARD, turn)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(fixture.gateway.started(), [0, 1, 2]);
    assert_eq!(
        receipts(&result),
        json!([1, 3]),
        "it1 did not await its unit"
    );
    let slots = fixture.slots(execution).await;
    assert_eq!(slots.len(), 3);
    assert!(
        slots
            .iter()
            .all(|slot| phase(slot) == EffectPhase::Resolved),
        "the detached unit was recorded in its iteration"
    );
    assert_eq!(slots[1].occurrence(), "it1/unit/v1/#000000");
}

// 10 ────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_node_retry_replays_earlier_iterations_and_grants_the_failed_one_again(
    #[case] backend: Backend,
) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = JournalFixture::build(
        database.ports(),
        Kind::Stateful,
        Some(nebula_workflow::RetryConfig::fixed(3, 1)),
    )
    .await;
    let execution = start_three(&fixture, json!({})).await;
    // Iteration 2's call is throttled: nothing applied, a retryable failure.
    *fixture.gateway.iterations.throttle_at.lock() = Some(2);
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(receipts(&result), json!([1, 2, 3]));
    assert_eq!(fixture.gateway.started(), [0, 1, 2, 0, 1, 2]);
    let keys = fixture.gateway.call_keys();
    assert_eq!(keys.len(), 4, "it0, it1, it2 throttled, it2 again");
    assert_eq!(keys[2], keys[3], "the same slot granted again");
    assert_eq!(fixture.gateway.applied(), 3);
    let slots = fixture.slots(execution).await;
    assert_eq!(slots.len(), 3, "the retry reused the occurrences");
    assert!(
        slots
            .iter()
            .all(|slot| phase(slot) == EffectPhase::Resolved)
    );
}

// 11 ────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn a_wait_after_an_iterations_effects_parks_and_resumes_without_a_call(
    #[case] backend: Backend,
) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture, json!({})).await;
    *fixture.gateway.iterations.wait_at.lock() = Some(0);
    let result = fixture.run(execution).await.unwrap();
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    assert_eq!(fixture.gateway.call_count(), 1, "it0's write, once");
    assert_eq!(
        fixture.gateway.started(),
        [0],
        "the timer completes the node"
    );
    let slots = fixture.slots(execution).await;
    assert_eq!(labels(&slots), ["it0/unit/v1/#000000"]);
    assert_eq!(phase(&slots[0]), EffectPhase::Resolved);

    // A later turn of the finished execution sends nothing.
    let _ = fixture.run(execution).await;
    assert_eq!(fixture.gateway.call_count(), 1);
    assert_eq!(fixture.slots(execution).await, slots);
}

// 12 ────────────────────────────────────────────────────────────────────────

#[rstest::rstest]
#[case::memory(Backend::Memory)]
#[case::sqlite(Backend::Sqlite)]
#[tokio::test]
async fn cancellation_mid_iteration_sends_nothing_further(#[case] backend: Backend) {
    let Some(database) = Database::open(backend).await else {
        return;
    };
    let fixture = stateful(database.ports()).await;
    let execution = start_three(&fixture, json!({})).await;
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
    assert!(engine.cancel_execution(execution), "the loop is live here");
    let result = tokio::time::timeout(HANG_GUARD, turn)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(result.status, ExecutionStatus::Cancelled, "{result:?}");
    assert_eq!(fixture.gateway.call_count(), 1, "only it0's write");
    assert_eq!(fixture.gateway.started(), [0, 1]);
    let slots = fixture.slots(execution).await;
    assert_eq!(labels(&slots), ["it0/unit/v1/#000000"]);
    assert_eq!(phase(&slots[0]), EffectPhase::Resolved);
}

// 13 ────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn a_storeless_stateful_action_keeps_read_only_handles_saying_why() {
    let fixture = stateful(Ports::memory()).await;
    let result = fixture
        .run_storeless(json!({
            "iterations": [[write("it-a:1")]],
            "swallow": true,
        }))
        .await;
    assert_eq!(result.status, ExecutionStatus::Completed, "{result:?}");
    let refused = &receipts(&result)[0];
    assert_eq!(refused["kind"], "permanent", "{refused}");
    assert_eq!(refused["sent"], "not_sent", "{refused}");
    assert_eq!(
        refused["detail"], "journaled effects need execution stores",
        "{refused}"
    );
    assert_eq!(fixture.gateway.call_count(), 0, "no provider call");
    assert_eq!(fixture.gateway.started(), [0], "the read ran");
}

// 14 ────────────────────────────────────────────────────────────────────────

/// One node attempt prepares at most 10 000 journaled effects. (In memory:
/// ten thousand durable slots per backend run would only slow the suite.)
#[tokio::test]
async fn a_node_over_its_journaled_slot_cap_is_refused_and_sends_nothing() {
    const CAP: usize = 10_000;
    let fixture = stateful(Ports::memory()).await;
    // Iteration 0 fills the cap; its own write is one too many.
    let execution = fixture
        .start_iterations(&[&[write("over-the-cap")]], json!({ "fill": CAP }))
        .await;
    let result = fixture.run(execution).await.unwrap();
    assert_node_error(&result, "ENGINE:EFFECT_JOURNAL_SLOT_CAP");
    assert_eq!(fixture.gateway.call_count(), CAP, "nothing past the cap");
    let slots = fixture.slots(execution).await;
    assert_eq!(slots.len(), CAP, "nothing prepared past the cap");
    assert!(
        fixture
            .gateway
            .calls
            .lock()
            .iter()
            .all(|call| call.request != "over-the-cap")
    );
}
