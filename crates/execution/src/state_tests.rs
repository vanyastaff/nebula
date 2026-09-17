use nebula_core::node_key;
use nebula_error::{ErrorCategory, ErrorCode};

use super::*;

fn make_state() -> (ExecutionState, NodeKey, NodeKey) {
    let n1 = node_key!("n1");
    let n2 = node_key!("n2");
    let state = ExecutionState::new(
        ExecutionId::new(),
        WorkflowId::new(),
        &[n1.clone(), n2.clone()],
    );
    (state, n1, n2)
}

#[test]
fn new_execution_state() {
    let (state, n1, _n2) = make_state();
    assert_eq!(state.status, ExecutionStatus::Created);
    assert_eq!(state.version, 0);
    assert_eq!(state.node_states.len(), 2);
    assert_eq!(state.node_state(n1).unwrap().state, NodeState::Pending);
}

#[test]
fn node_execution_state_default() {
    let nes = NodeExecutionState::new();
    assert_eq!(nes.state, NodeState::Pending);
    assert_eq!(nes.attempt_count(), 0);
    assert!(nes.latest_attempt().is_none());
    assert!(nes.scheduled_at.is_none());
}

#[test]
fn node_state_transition() {
    let mut nes = NodeExecutionState::new();
    assert!(nes.transition_to(NodeState::Ready).is_ok());
    assert_eq!(nes.state, NodeState::Ready);
    assert!(nes.scheduled_at.is_some());

    assert!(nes.transition_to(NodeState::Running).is_ok());
    assert_eq!(nes.state, NodeState::Running);
    assert!(nes.started_at.is_some());

    assert!(nes.transition_to(NodeState::Completed).is_ok());
    assert!(nes.completed_at.is_some());
}

#[test]
fn node_state_invalid_transition() {
    let mut nes = NodeExecutionState::new();
    let err = nes.transition_to(NodeState::Completed).unwrap_err();
    assert!(err.to_string().contains("invalid transition"));
}

#[test]
fn all_nodes_terminal() {
    let (mut state, n1, n2) = make_state();
    assert!(!state.all_nodes_terminal());

    state.node_states.get_mut(&n1).unwrap().state = NodeState::Completed;
    state.node_states.get_mut(&n2).unwrap().state = NodeState::Failed;
    assert!(state.all_nodes_terminal());
}

#[test]
fn active_node_ids() {
    let (mut state, n1, _n2) = make_state();
    state.node_states.get_mut(&n1).unwrap().state = NodeState::Running;
    let active = state.active_node_ids();
    assert_eq!(active.len(), 1);
    assert!(active.contains(&n1));
}

#[test]
fn completed_and_failed_node_ids() {
    let (mut state, n1, n2) = make_state();
    state.node_states.get_mut(&n1).unwrap().state = NodeState::Completed;
    state.node_states.get_mut(&n2).unwrap().state = NodeState::Failed;

    assert_eq!(state.completed_node_ids(), vec![n1]);
    assert_eq!(state.failed_node_ids(), vec![n2]);
}

#[test]
fn transition_status_valid() {
    let (mut state, _n1, _n2) = make_state();
    assert!(state.transition_status(ExecutionStatus::Running).is_ok());
    assert_eq!(state.status, ExecutionStatus::Running);
    assert_eq!(state.version, 1);
    assert!(state.started_at.is_some());
}

#[test]
fn transition_status_invalid() {
    let (mut state, _n1, _n2) = make_state();
    let err = state
        .transition_status(ExecutionStatus::Completed)
        .unwrap_err();
    assert!(err.to_string().contains("invalid transition"));
    assert_eq!(state.version, 0); // version not bumped
}

#[test]
fn transition_status_terminal_sets_completed_at() {
    let (mut state, _n1, _n2) = make_state();
    state.transition_status(ExecutionStatus::Running).unwrap();
    state.transition_status(ExecutionStatus::Completed).unwrap();
    assert!(state.completed_at.is_some());
}

#[test]
fn cancel_nonterminal_nodes_terminalizes_all_active_and_is_idempotent() {
    let (mut state, n1, n2) = make_state();
    // n1: a parked wait carrying a wake timer; n2: a blocked Pending node.
    {
        let first_node_state = state.node_states.get_mut(&n1).unwrap();
        first_node_state.state = NodeState::Waiting;
        first_node_state.next_attempt_at = Some(Utc::now());
    }
    // n2 stays Pending (its upstream is the parked wait).

    let count = state.cancel_nonterminal_nodes().unwrap();
    assert_eq!(count, 2, "both non-terminal nodes must be cancelled");
    assert!(
        state.all_nodes_terminal(),
        "no non-terminal node may survive cancel-of-no-live-runner"
    );
    assert_eq!(
        state.node_state(n1.clone()).unwrap().state,
        NodeState::Cancelled
    );
    assert_eq!(state.node_state(n2).unwrap().state, NodeState::Cancelled);
    assert!(
        state.node_state(n1).unwrap().next_attempt_at.is_none(),
        "the wake timer must be cleared on cancel"
    );

    // Idempotent: a re-delivered Cancel finds everything terminal.
    assert_eq!(state.cancel_nonterminal_nodes().unwrap(), 0);
}

#[test]
fn set_node_state() {
    let (mut state, _n1, _n2) = make_state();
    let new_node = node_key!("new_node");
    state.set_node_state(new_node.clone(), NodeExecutionState::new());
    assert!(state.node_state(new_node).is_some());
}

/// Regression for issue #255: every node-state transition must
/// bump the parent `ExecutionState::version` so optimistic
/// concurrency readers can detect the change. The old engine
/// pattern `state.node_states.get_mut(&id).unwrap().transition_to(...)`
/// silently skipped the bump — the `transition_node` method closes
/// that hole.
#[test]
fn transition_node_bumps_parent_version_and_touches_updated_at() {
    let (mut state, n1, _n2) = make_state();
    let v0 = state.version;
    let t0 = state.updated_at;

    state
        .transition_node(n1.clone(), NodeState::Ready)
        .expect("valid transition");
    assert_eq!(
        state.node_state(n1.clone()).unwrap().state,
        NodeState::Ready
    );
    assert_eq!(state.version, v0 + 1, "version must be bumped");
    assert!(state.updated_at >= t0, "updated_at must move forward");

    // Chained transitions each bump the version once.
    state
        .transition_node(n1.clone(), NodeState::Running)
        .unwrap();
    assert_eq!(state.version, v0 + 2);
    state
        .transition_node(n1.clone(), NodeState::Completed)
        .unwrap();
    assert_eq!(state.version, v0 + 3);
    assert!(state.node_state(n1).unwrap().state.is_terminal());
}

#[test]
fn transition_node_invalid_transition_does_not_bump_version() {
    let (mut state, n1, _n2) = make_state();
    let v0 = state.version;
    // Pending -> Completed is invalid (must pass through Ready/Running).
    let err = state
        .transition_node(n1.clone(), NodeState::Completed)
        .expect_err("invalid transition must error");
    assert!(err.to_string().contains("invalid transition"));
    // Version must NOT move on a rejected transition — if it did,
    // optimistic-concurrency readers would see a phantom change.
    assert_eq!(state.version, v0);
    // And the node stayed Pending.
    assert_eq!(state.node_state(n1).unwrap().state, NodeState::Pending);
}

#[test]
fn transition_node_unknown_node_is_error() {
    let (mut state, _n1, _n2) = make_state();
    let ghost = node_key!("ghost");
    let err = state
        .transition_node(ghost, NodeState::Ready)
        .expect_err("unknown node id");
    assert!(matches!(err, ExecutionError::NodeNotFound(_)));
    // Version unchanged.
    assert_eq!(state.version, 0);
}

#[test]
fn start_attempt_pending_path() {
    let mut ns = NodeExecutionState::new();
    ns.start_attempt()
        .expect("pending -> running should be legal");
    assert_eq!(ns.state, NodeState::Running);
    assert!(ns.scheduled_at.is_some());
    assert!(ns.started_at.is_some());
}

/// — engine retries via the `Failed → WaitingRetry` edge,
/// not directly from `Failed`. A `start_attempt` on `Failed` is
/// still rejected (the engine must first promote the node to
/// `WaitingRetry` via the retry-decision path); but
/// `WaitingRetry` itself is now a legal source.
#[test]
fn start_attempt_rejects_failed() {
    let mut ns = NodeExecutionState::new();
    ns.transition_to(NodeState::Ready).unwrap();
    ns.transition_to(NodeState::Running).unwrap();
    ns.transition_to(NodeState::Failed).unwrap();
    let err = ns
        .start_attempt()
        .expect_err("Failed must promote to WaitingRetry before re-dispatch");
    assert!(matches!(err, ExecutionError::InvalidTransition { .. }));
    assert_eq!(ns.state, NodeState::Failed, "state must not move on error");
}

/// — `WaitingRetry → Ready → Running` is the retry
/// re-dispatch path. `start_attempt` honors it.
#[test]
fn start_attempt_promotes_waiting_retry() {
    let mut ns = NodeExecutionState::new();
    ns.transition_to(NodeState::Ready).unwrap();
    ns.transition_to(NodeState::Running).unwrap();
    ns.transition_to(NodeState::Failed).unwrap();
    ns.transition_to(NodeState::WaitingRetry).unwrap();

    ns.start_attempt()
        .expect("WaitingRetry must be a legal start_attempt source for engine retries");
    assert_eq!(ns.state, NodeState::Running);
}

#[test]
fn start_attempt_rejects_completed() {
    let mut ns = NodeExecutionState::new();
    ns.transition_to(NodeState::Ready).unwrap();
    ns.transition_to(NodeState::Running).unwrap();
    ns.transition_to(NodeState::Completed).unwrap();
    let err = ns
        .start_attempt()
        .expect_err("completed nodes cannot start a fresh attempt");
    assert!(matches!(err, ExecutionError::InvalidTransition { .. }));
    assert_eq!(
        ns.state,
        NodeState::Completed,
        "state must not move on error"
    );
}

#[test]
fn execution_state_start_node_attempt_bumps_version() {
    let (mut state, n1, _n2) = make_state();
    let v0 = state.version;
    state.start_node_attempt(n1.clone()).unwrap();
    assert_eq!(state.node_state(n1).unwrap().state, NodeState::Running);
    assert_eq!(state.version, v0 + 1);
}

/// A failure record standing in for one the engine would build.
fn failure(message: &str) -> ErrorEnvelope {
    ErrorEnvelope::new(
        ErrorCode::new("ENGINE:NODE_FAILED"),
        ErrorCategory::Internal,
        false,
    )
    .with_redacted_message(message)
}

#[test]
fn mark_setup_failed_records_error_and_bumps_version() {
    let (mut state, n1, _n2) = make_state();
    let v0 = state.version;
    state
        .mark_setup_failed(n1.clone(), failure("param resolution: missing credential"))
        .unwrap();
    let ns = state.node_state(n1).unwrap();
    assert_eq!(ns.state, NodeState::Failed);
    assert_eq!(
        ns.error_message
            .as_ref()
            .and_then(ErrorEnvelope::redacted_message),
        Some("param resolution: missing credential")
    );
    assert_eq!(state.version, v0 + 1);
}

#[test]
fn workflow_input_roundtrip_via_serde() {
    let (mut state, _n1, _n2) = make_state();
    assert!(state.workflow_input.is_none());
    state.set_workflow_input(serde_json::json!({"trigger": "webhook"}));
    let json = serde_json::to_string(&state).unwrap();
    let back: ExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.workflow_input,
        Some(serde_json::json!({"trigger": "webhook"}))
    );
}

/// Issue #289 — `ExecutionBudget` must round-trip through serde
/// so `resume_execution` can restore the original run's concurrency,
/// timeout, and output-size limits instead of silently falling back
/// to [`ExecutionBudget::default()`].
#[test]
fn budget_roundtrip_via_serde() {
    use std::time::Duration;

    let (mut state, _n1, _n2) = make_state();
    assert!(state.budget.is_none());

    let budget = ExecutionBudget::default()
        .with_max_concurrent_nodes(4)
        .with_max_duration(Duration::from_mins(2))
        .with_max_output_bytes(4 * 1024 * 1024);
    state.set_budget(budget.clone());

    let json = serde_json::to_string(&state).unwrap();
    let back: ExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.budget, Some(budget));
}

/// Issue #289 — legacy states that predate `budget` must still
/// deserialize as `None` so the engine can fall back to
/// `ExecutionBudget::default()` with a warning.
#[test]
fn budget_missing_field_deserializes_as_none() {
    let legacy = serde_json::json!({
        "execution_id": ExecutionId::new(),
        "workflow_id": WorkflowId::new(),
        "status": "created",
        "node_states": {},
        "version": 0,
        "created_at": Utc::now(),
        "updated_at": Utc::now(),
        "total_output_bytes": 0,
    });
    let state: ExecutionState = serde_json::from_value(legacy).unwrap();
    assert!(state.budget.is_none());
}

#[test]
fn workflow_input_missing_field_deserializes_as_none() {
    // Legacy stored states that predate `workflow_input` must
    // still deserialize — we rely on `#[serde(default)]`.
    let legacy = serde_json::json!({
        "execution_id": ExecutionId::new(),
        "workflow_id": WorkflowId::new(),
        "status": "created",
        "node_states": {},
        "version": 0,
        "created_at": Utc::now(),
        "updated_at": Utc::now(),
        "total_output_bytes": 0,
    });
    let state: ExecutionState = serde_json::from_value(legacy).unwrap();
    assert!(state.workflow_input.is_none());
}

#[test]
fn serde_roundtrip() {
    let (state, _n1, _n2) = make_state();
    let json = serde_json::to_string(&state).unwrap();
    let back: ExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.execution_id, state.execution_id);
    assert_eq!(back.workflow_id, state.workflow_id);
    assert_eq!(back.status, state.status);
    assert_eq!(back.node_states.len(), state.node_states.len());
}

// Regression for #266: the idempotency key is
// for the **next** dispatch — `attempts.len() + 1`. Push-on-result
// semantics in the engine guarantees that a retried or
// restart-replayed attempt does not collide with a previous
// attempt's persisted output.
#[test]
fn idempotency_key_for_node_uses_attempt_count() {
    use crate::{attempt::NodeAttempt, idempotency::IdempotencyKey};

    let (mut state, n1, _n2) = make_state();
    let eid = state.execution_id;

    let fresh = state.idempotency_key_for_node(n1.clone());
    assert_eq!(
        fresh,
        IdempotencyKey::for_attempt(eid, n1.clone(), 1),
        "first dispatch (no prior attempts) keys on attempt=1"
    );

    let ns = state.node_states.get_mut(&n1).unwrap();
    let seed_key = IdempotencyKey::for_attempt(eid, n1.clone(), 1);
    ns.attempts.push(NodeAttempt::new(1, seed_key));

    let after_one = state.idempotency_key_for_node(n1.clone());
    assert_eq!(
        after_one,
        IdempotencyKey::for_attempt(eid, n1.clone(), 2),
        "after one prior attempt the next dispatch keys on attempt=2"
    );

    let ns = state.node_states.get_mut(&n1).unwrap();
    ns.attempts.push(NodeAttempt::new(
        2,
        IdempotencyKey::for_attempt(eid, n1.clone(), 2),
    ));

    let after_two = state.idempotency_key_for_node(n1.clone());
    assert_eq!(
        after_two,
        IdempotencyKey::for_attempt(eid, n1, 3),
        "after two prior attempts the next dispatch keys on attempt=3"
    );
}

/// `record_node_attempt` pushes a sequential
/// attempt with the right number, captures the outcome, and bumps
/// the parent version (issue #255).
#[test]
fn record_node_attempt_appends_with_sequential_number() {
    use crate::output::ExecutionOutput;

    let (mut state, n1, _n2) = make_state();
    let eid = state.execution_id;
    let v0 = state.version;

    let n = state
        .record_node_attempt(
            n1.clone(),
            AttemptOutcome::Failure {
                error: failure("boom"),
            },
        )
        .unwrap();
    assert_eq!(n, 1, "first attempt is numbered 1");
    assert_eq!(state.version, v0 + 1);

    let ns = state.node_state(n1.clone()).unwrap();
    assert_eq!(ns.attempts.len(), 1);
    assert_eq!(ns.attempts[0].attempt_number, 1);
    assert_eq!(
        ns.attempts[0].idempotency_key,
        IdempotencyKey::for_attempt(eid, n1.clone(), 1),
        "internally minted key must match attempt number"
    );
    assert!(ns.attempts[0].is_failure());

    let n = state
        .record_node_attempt(
            n1.clone(),
            AttemptOutcome::Success {
                output: ExecutionOutput::inline(serde_json::json!({"ok": true})),
                output_bytes: 12,
            },
        )
        .unwrap();
    assert_eq!(n, 2, "second attempt is numbered 2");
    assert_eq!(state.version, v0 + 2);

    let ns = state.node_state(n1.clone()).unwrap();
    assert_eq!(ns.attempts.len(), 2);
    assert_eq!(
        ns.attempts[1].idempotency_key,
        IdempotencyKey::for_attempt(eid, n1, 2),
        "second attempt key carries attempt-2"
    );
    assert!(ns.attempts[1].is_success());
}

/// `record_node_attempt` rejects unknown nodes — the engine must
/// surface the programming error rather than silently lose the
/// attempt record.
#[test]
fn record_node_attempt_unknown_node_is_error() {
    let (mut state, _n1, _n2) = make_state();
    let ghost = node_key!("ghost");
    let err = state
        .record_node_attempt(
            ghost,
            AttemptOutcome::Failure {
                error: failure("boom"),
            },
        )
        .expect_err("unknown node must error");
    assert!(matches!(err, ExecutionError::NodeNotFound(_)));
}

/// `schedule_node_retry` promotes Failed →
/// WaitingRetry, stamps `next_attempt_at`, and increments
/// `total_retries`. All three observable effects move atomically
/// (single `checkpoint_node` covers the version bumps).
#[test]
fn schedule_node_retry_promotes_failed_and_increments_total_retries() {
    let (mut state, n1, _n2) = make_state();

    // Drive n1 to Failed via the real path.
    state.start_node_attempt(n1.clone()).unwrap();
    state
        .transition_node(n1.clone(), NodeState::Failed)
        .unwrap();
    let v_before = state.version;
    let total_before = state.total_retries;
    let when = Utc::now() + chrono::Duration::milliseconds(500);

    state.schedule_node_retry(n1.clone(), when).unwrap();

    let ns = state.node_state(n1).unwrap();
    assert_eq!(ns.state, NodeState::WaitingRetry);
    assert_eq!(ns.next_attempt_at, Some(when));
    assert_eq!(state.total_retries, total_before + 1);
    // transition_node + total_retries bump = 2 version moves on
    // the same `checkpoint_node` write.
    assert_eq!(state.version, v_before + 2);
}

/// CodeRabbit review for PR #628 — `schedule_node_retry` must
/// scrub failure-only metadata (`error_message`, `completed_at`)
/// when reactivating a `Failed` node. Otherwise a later
/// successful retry would leave persisted state where
/// `state == Completed` carries the pre-retry error message —
/// post-mortem readers would misattribute the success.
#[test]
fn schedule_node_retry_clears_failure_metadata() {
    let (mut state, n1, _n2) = make_state();
    state.start_node_attempt(n1.clone()).unwrap();
    state
        .transition_node(n1.clone(), NodeState::Failed)
        .unwrap();
    // Stamp failure-only fields the way `mark_node_failed` /
    // `transition_to(Failed)` would.
    if let Some(ns) = state.node_states.get_mut(&n1) {
        ns.error_message = Some(failure("boom"));
        // `completed_at` is set by `transition_to(Failed)` so
        // we expect it to already be `Some` here.
        assert!(ns.completed_at.is_some());
    }
    let when = Utc::now() + chrono::Duration::milliseconds(500);

    state.schedule_node_retry(n1.clone(), when).unwrap();

    let ns = state.node_state(n1).unwrap();
    assert_eq!(ns.state, NodeState::WaitingRetry);
    assert_eq!(ns.next_attempt_at, Some(when));
    assert!(
        ns.error_message.is_none(),
        "stale error_message must be cleared on retry promotion"
    );
    assert!(
        ns.completed_at.is_none(),
        "stale completed_at must be cleared on retry promotion"
    );
}

/// `schedule_node_retry` rejects nodes that are not in `Failed` —
/// e.g. `Running` (race between failure and a stale call).
#[test]
fn schedule_node_retry_rejects_non_failed() {
    let (mut state, n1, _n2) = make_state();
    state.start_node_attempt(n1.clone()).unwrap();
    // n1 is now Running, not Failed.
    let when = Utc::now();
    let err = state
        .schedule_node_retry(n1.clone(), when)
        .expect_err("Running → WaitingRetry must be rejected");
    assert!(matches!(err, ExecutionError::InvalidTransition { .. }));
    // State unchanged.
    assert_eq!(state.node_state(n1).unwrap().state, NodeState::Running);
    assert_eq!(state.total_retries, 0);
}

#[test]
fn idempotency_key_for_node_unknown_node_defaults_to_one() {
    use crate::idempotency::IdempotencyKey;

    let (state, _n1, _n2) = make_state();
    let phantom = node_key!("not_in_state");
    let eid = state.execution_id;

    let key = state.idempotency_key_for_node(phantom.clone());
    assert_eq!(key, IdempotencyKey::for_attempt(eid, phantom, 1));
}

/// ROADMAP §M0.3 — `terminated_by` must round-trip through serde
/// so a resumed execution sees the same authoritative termination
/// signal the original run recorded. Pairs with the runtime
/// guarantee that the engine persists `ExecutionState` (including
/// this field) via `checkpoint_node` immediately after
/// `set_terminated_by`.
#[test]
fn terminated_by_roundtrip_via_serde() {
    let (mut state, n1, _n2) = make_state();
    assert!(state.terminated_by.is_none());

    let was_first = state.set_terminated_by(
        n1.clone(),
        ExecutionTerminationReason::ExplicitStop {
            by_node: n1.clone(),
            note: Some("done".to_owned()),
        },
    );
    assert!(was_first, "first set_terminated_by must return true");

    let json = serde_json::to_string(&state).unwrap();
    let back: ExecutionState = serde_json::from_str(&json).unwrap();
    match back.terminated_by {
        Some((nk, ExecutionTerminationReason::ExplicitStop { by_node, note })) => {
            assert_eq!(nk, n1);
            assert_eq!(by_node, n1);
            assert_eq!(note.as_deref(), Some("done"));
        },
        other => panic!("unexpected terminated_by after roundtrip: {other:?}"),
    }
}

/// ROADMAP §M0.3 — legacy persisted states that predate
/// `terminated_by` must still deserialize so a resumed legacy
/// execution does not crash on missing field. Engine then treats
/// those as never-explicitly-terminated.
#[test]
fn terminated_by_missing_field_deserializes_as_none() {
    let legacy = serde_json::json!({
        "execution_id": ExecutionId::new(),
        "workflow_id": WorkflowId::new(),
        "status": "created",
        "node_states": {},
        "version": 0,
        "created_at": Utc::now(),
        "updated_at": Utc::now(),
        "total_output_bytes": 0,
    });
    let state: ExecutionState = serde_json::from_value(legacy).unwrap();
    assert!(state.terminated_by.is_none());
}

/// ROADMAP §M0.3 — first-write-wins. The engine relies on this
/// return value to decide whether to signal `cancel_token` (only
/// on `true`) so the second signal must NOT replace the first.
#[test]
fn set_terminated_by_is_first_write_wins() {
    let (mut state, n1, n2) = make_state();

    let first = state.set_terminated_by(
        n1.clone(),
        ExecutionTerminationReason::ExplicitStop {
            by_node: n1.clone(),
            note: None,
        },
    );
    assert!(first, "first set must succeed");
    let v_after_first = state.version;

    let second = state.set_terminated_by(
        n2.clone(),
        ExecutionTerminationReason::ExplicitFail {
            by_node: n2,
            code: crate::status::ExecutionTerminationCode::new("E_FAIL"),
            message: "should be ignored".to_owned(),
        },
    );
    assert!(!second, "second set must return false (idempotent)");

    // The original signal must still be the recorded one.
    match state.terminated_by.as_ref() {
        Some((
            nk,
            ExecutionTerminationReason::ExplicitStop {
                by_node,
                note: None,
            },
        )) => {
            assert_eq!(nk, &n1);
            assert_eq!(by_node, &n1);
        },
        other => panic!("first signal must remain in place: {other:?}"),
    }
    // And the version must NOT have moved on the duplicate path.
    assert_eq!(
        state.version, v_after_first,
        "version must not bump on duplicate set_terminated_by"
    );
}

/// ROADMAP §M0.3 invariant 1 — `set_terminated_by` must reject
/// non-explicit reason variants. `NaturalCompletion`, `Cancelled`,
/// and `SystemError` are engine-attributed via
/// `determine_final_status` priority-ladder branches and must not
/// be recorded directly in `terminated_by`.
#[test]
fn set_terminated_by_rejects_non_explicit_reason() {
    let (mut state, n1, _n2) = make_state();
    let v0 = state.version;

    for reason in [
        ExecutionTerminationReason::NaturalCompletion,
        ExecutionTerminationReason::Cancelled,
        ExecutionTerminationReason::SystemError,
    ] {
        assert!(
            !state.set_terminated_by(n1.clone(), reason),
            "non-explicit variant must be rejected"
        );
        assert!(
            state.terminated_by.is_none(),
            "rejected call must not mutate terminated_by"
        );
        assert_eq!(state.version, v0, "rejected call must not bump version");
    }
}

/// ROADMAP §M0.3 invariant 2 — `set_terminated_by` must reject a
/// reason whose inner `by_node` does not match the `node_key`
/// argument. Engine wiring constructs the reason via
/// `map_termination_reason(node_key.clone(),...)` so a mismatch
/// indicates a programming error (or a refactor regression) and
/// must surface as `false` rather than store inconsistent data.
#[test]
fn set_terminated_by_rejects_mismatched_by_node() {
    let (mut state, n1, n2) = make_state();
    let v0 = state.version;

    // Outer key = n1, inner by_node = n2 — identity mismatch.
    let mismatched = ExecutionTerminationReason::ExplicitStop {
        by_node: n2,
        note: None,
    };
    assert!(
        !state.set_terminated_by(n1, mismatched),
        "mismatched by_node must be rejected"
    );
    assert!(state.terminated_by.is_none());
    assert_eq!(state.version, v0);
}

/// ROADMAP §M0.3 review M1 — recovery escape hatch:
/// `clear_terminated_by` removes an in-memory signal that never
/// made it to disk via `checkpoint_node`. Returns `true` when
/// there was a signal to clear, `false` otherwise. Does NOT bump
/// `version` (the matching set's bump never reached disk either).
#[test]
fn clear_terminated_by_undoes_in_memory_set() {
    let (mut state, n1, _n2) = make_state();

    // No signal to clear initially.
    assert!(
        !state.clear_terminated_by(),
        "clear on empty must return false"
    );

    // Set, then clear.
    let was_first = state.set_terminated_by(
        n1.clone(),
        ExecutionTerminationReason::ExplicitStop {
            by_node: n1,
            note: None,
        },
    );
    assert!(was_first);
    let v_after_set = state.version;

    let cleared = state.clear_terminated_by();
    assert!(cleared, "clear on Some(_) must return true");
    assert!(state.terminated_by.is_none());
    assert_eq!(
        state.version, v_after_set,
        "clear_terminated_by must NOT bump version — readers keying on \
         the set's bump should never have observed the intermediate state"
    );
}

/// `next_attempt_at` must round-trip via
/// serde so a resumed engine picks up scheduled retries at their
/// declared time.
#[test]
fn next_attempt_at_roundtrip_via_serde() {
    let mut ns = NodeExecutionState::new();
    let when = Utc::now();
    ns.next_attempt_at = Some(when);
    let json = serde_json::to_string(&ns).unwrap();
    let back: NodeExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.next_attempt_at,
        Some(when),
        "next_attempt_at must survive serde roundtrip"
    );
}

/// Forward-compat: legacy `NodeExecutionState` JSON that predates
/// `next_attempt_at` deserializes as `None`. Engine then treats
/// the node as not having a pending retry.
#[test]
fn next_attempt_at_missing_field_deserializes_as_none() {
    let legacy = serde_json::json!({
        "state": "pending",
        "attempts": [],
    });
    let ns: NodeExecutionState = serde_json::from_value(legacy).unwrap();
    assert!(ns.next_attempt_at.is_none());
}

/// `ExecutionState` is the blob persisted in the `executions.state` row. A
/// node failure recorded before the envelope existed holds a bare error
/// string, so an old row must fail closed rather than hand that text back as
/// a decode success.
///
/// **Falsifiability**: give `error_message` its pre-envelope `String` type →
/// the decode below succeeds and the assert fails. The non-vacuity assert
/// pins that the downgrade landed on the failed node, so a wrong map key
/// cannot make this pass by refusing a state that was never downgraded.
/// The control decode (the un-downgraded wire, same `from_str` path) is
/// what makes that falsifiability real: `node_states`' key type (`NodeKey`,
/// an external `domain_key` type) only implements `Deserialize` for a
/// borrowed `&str`, so `serde_json::from_value` fails to decode *any*
/// `ExecutionState` with a non-empty `node_states` map, downgraded or not.
/// Without the control, this test would pass for that unrelated reason
/// even if the downgrade detection were deleted entirely — decoding
/// through `from_str` (which reads the borrowed `&str` straight out of the
/// input buffer) is what makes the failure below attributable to the bare
/// string rather than to `from_value`'s map-key limitation.
///
/// Dropping the version check in `EnvelopeVersion`'s `Deserialize` is *not*
/// an alternative downgrade for this fixture, and would not fail it: the
/// bare string is refused one step earlier, when the record is decoded as a
/// map, before any version is read. That gate has its own fixtures —
/// `unknown_version_is_refused_with_the_versions_named` and
/// `legacy_bare_string_record_is_refused` in `error_envelope.rs`.
#[test]
fn legacy_state_row_with_a_bare_error_message_fails_to_decode() {
    let (mut state, n1, _n2) = make_state();
    state
        .mark_setup_failed(n1.clone(), failure("boom"))
        .expect("state must accept a setup failure");

    let mut wire = serde_json::to_value(&state).unwrap();
    let key = serde_json::to_value(&n1)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let persisted = wire["node_states"][&key]["error_message"].clone();
    assert!(
        persisted.is_object(),
        "fixture must persist a typed record, got: {persisted:?}"
    );

    // Control: the un-downgraded wire must decode `Ok` on the exact path
    // (`from_str`) the downgraded wire is decoded on below.
    let control = serde_json::from_str::<ExecutionState>(&wire.to_string());
    assert!(
        control.is_ok(),
        "the un-downgraded wire must decode via from_str: {control:?}"
    );

    wire["node_states"][&key]["error_message"] = serde_json::json!("provider said: token abc123");

    let decoded = serde_json::from_str::<ExecutionState>(&wire.to_string());

    let err = decoded.expect_err("a bare error_message is not an envelope");
    let message = err.to_string();
    assert!(
        message.contains("ErrorEnvelope"),
        "the refusal must name the type it refused to decode as, got: {message}"
    );
}

/// W-S2b — `park_node` stamps the (`next_attempt_at`, `wait_wake`) pair
/// together. A timer wake parked with `WaitWake::Timeout` records both
/// the deadline and the timeout discriminator so a post-crash recovery
/// reads the wake as a failure, not a completion.
///
/// **Falsifiability**: drop the `wait_wake` field write from `park_node`
/// → `back.wait_wake` is `None` not `Some(Timeout)` → the assert fails.
#[test]
fn park_node_sets_wait_wake() {
    let (mut state, n1, _n2) = make_state();
    // Drive n1 to Running so the `Running → Waiting` park edge is legal.
    state.start_node_attempt(n1.clone()).unwrap();

    let wake_at = Utc::now() + chrono::Duration::seconds(30);
    // A signal+timeout park: the timer is the Timeout deadline and the wait
    // carries its resume-identity (a webhook callback_id).
    state
        .park_node(
            n1.clone(),
            Some(wake_at),
            Some(WaitWake::Timeout),
            Some(WaitSignal::Webhook {
                callback_id: "cb-timeout".to_owned(),
            }),
        )
        .expect("Running → Waiting park must succeed");

    let ns = state.node_state(n1).unwrap();
    assert_eq!(ns.state, NodeState::Waiting);
    assert_eq!(ns.next_attempt_at, Some(wake_at));
    assert_eq!(
        ns.wait_wake,
        Some(WaitWake::Timeout),
        "park_node must record the Timeout wake discriminator alongside the timer"
    );
}

/// W-S2b — a signal-only park (no timer) carries neither a wake instant
/// nor a `wait_wake` discriminator. This is the case-a path: the node is
/// satisfied by an explicit Resume, never by a timer.
#[test]
fn park_node_signal_only_has_no_wait_wake() {
    let (mut state, n1, _n2) = make_state();
    state.start_node_attempt(n1.clone()).unwrap();

    state
        .park_node(
            n1.clone(),
            None,
            None,
            Some(WaitSignal::Webhook {
                callback_id: "cb-signal-only".to_owned(),
            }),
        )
        .expect("Running → Waiting signal-only park must succeed");

    let ns = state.node_state(n1).unwrap();
    assert_eq!(ns.state, NodeState::Waiting);
    assert!(ns.next_attempt_at.is_none());
    assert!(
        ns.wait_wake.is_none(),
        "a signal-only park must not carry a wake discriminator"
    );
    assert_eq!(
        ns.wait_signal,
        Some(WaitSignal::Webhook {
            callback_id: "cb-signal-only".to_owned(),
        }),
        "a signal-only park must persist its resume-identity"
    );
}

/// W-S2b — `park_node` REJECTS a desynced (`wake_at`, `wait_wake`) pair
/// with a typed error and leaves the node untouched. A `Some(wake_at),
/// None` (or the inverse) would persist a timer wake with no declared
/// meaning — silently turning a timeout into a completion — so the pairing
/// is a load-bearing runtime guard, not a debug-only assert.
///
/// **Falsifiability**: replace the runtime guard with the old
/// `debug_assert_eq!` → in a release/`cargo test --release` build the
/// desync slips through, `park_node` returns `Ok`, the node becomes
/// `Waiting` with a `Some(wake_at), None` desync → both asserts flip.
#[test]
fn park_node_rejects_wake_pairing_desync() {
    let (mut state, n1, _n2) = make_state();
    state.start_node_attempt(n1.clone()).unwrap();
    let wake_at = Utc::now() + chrono::Duration::seconds(30);

    // wake_at without a wait_wake meaning is a desync — must be rejected.
    let err = state
        .park_node(n1.clone(), Some(wake_at), None, None)
        .expect_err("a Some(wake_at), None park must be rejected");
    assert!(
        matches!(err, ExecutionError::InvalidTransition { .. }),
        "the desync must surface as a typed InvalidTransition, got {err:?}"
    );

    // The inverse desync (wait_wake without a timer) is equally rejected.
    let err = state
        .park_node(n1.clone(), None, Some(WaitWake::Timeout), None)
        .expect_err("a None, Some(wait_wake) park must be rejected");
    assert!(matches!(err, ExecutionError::InvalidTransition { .. }));

    // The guard ran before any mutation: the node is still Running, never
    // parked, and carries no stale wake metadata.
    let ns = state.node_state(n1).unwrap();
    assert_eq!(
        ns.state,
        NodeState::Running,
        "a rejected park must leave the node untouched (still Running)"
    );
    assert!(ns.next_attempt_at.is_none());
    assert!(ns.wait_wake.is_none());
}

/// W-S2b — `wait_wake` round-trips through serde, and a legacy
/// `NodeExecutionState` JSON that predates the field deserializes as
/// `None` (read by the engine as `Completion` for an armed timer wait,
/// preserving W-S1 timer-wake semantics).
///
/// **Falsifiability**: drop `#[serde(default)]` from `wait_wake` → the
/// legacy-JSON `from_value` errors (missing field) → the test panics.
#[test]
fn wait_wake_serde_roundtrip_and_legacy_default() {
    // Round-trip a `Some(Timeout)`.
    let mut ns = NodeExecutionState::new();
    ns.wait_wake = Some(WaitWake::Timeout);
    let json = serde_json::to_string(&ns).unwrap();
    let back: NodeExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(
        back.wait_wake,
        Some(WaitWake::Timeout),
        "wait_wake must survive a serde roundtrip"
    );

    // Round-trip a `Some(Completion)`.
    ns.wait_wake = Some(WaitWake::Completion);
    let json = serde_json::to_string(&ns).unwrap();
    let back: NodeExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.wait_wake, Some(WaitWake::Completion));

    // Legacy JSON without the field deserializes as `None`.
    let legacy = serde_json::json!({
        "state": "waiting",
        "attempts": [],
        "next_attempt_at": Utc::now(),
    });
    let ns: NodeExecutionState = serde_json::from_value(legacy).unwrap();
    assert!(
        ns.wait_wake.is_none(),
        "legacy rows without wait_wake must deserialize as None"
    );
}

/// W-S2b — `arm_wait_completion` and `clear_wait_timer` write the
/// (`next_attempt_at`, `wait_wake`) pair together, keeping the
/// `next_attempt_at.is_some() == wait_wake.is_some()` invariant the engine's
/// signal-wait routing relies on. These methods are the single paired-write
/// entry points the engine calls instead of touching the two fields raw, so
/// a future edit cannot desync them.
///
/// **Falsifiability**: drop the `wait_wake` write from `arm_wait_completion`
/// → the `wait_wake == Some(Completion)` assert fails; drop the
/// `next_attempt_at` clear from `clear_wait_timer` → the `is_none` assert
/// fails.
#[test]
fn arm_and_clear_wait_timer_write_the_pair_together() {
    let when = Utc::now();
    let mut ns = NodeExecutionState::new();

    ns.arm_wait_completion(when);
    assert_eq!(
        ns.next_attempt_at,
        Some(when),
        "arm must stamp the wake instant"
    );
    assert_eq!(
        ns.wait_wake,
        Some(WaitWake::Completion),
        "arm must record the Completion discriminator"
    );
    assert_eq!(
        ns.next_attempt_at.is_some(),
        ns.wait_wake.is_some(),
        "armed pair must satisfy the next_attempt_at/wait_wake invariant"
    );

    ns.clear_wait_timer();
    assert!(
        ns.next_attempt_at.is_none(),
        "clear must drop the wake instant"
    );
    assert!(
        ns.wait_wake.is_none(),
        "clear must drop the wake discriminator"
    );
    assert_eq!(
        ns.next_attempt_at.is_some(),
        ns.wait_wake.is_some(),
        "cleared pair must satisfy the next_attempt_at/wait_wake invariant"
    );
}

/// W-S3a — `wait_signal` round-trips through serde across all three
/// variants, and a legacy `NodeExecutionState` JSON that predates the field
/// deserializes as `None` (read by the engine as untargetable-by-identity,
/// only an untargeted Resume arms it — preserving W-S2b behavior).
///
/// **Falsifiability**: drop `#[serde(default)]` from `wait_signal` → the
/// legacy-JSON `from_value` errors (missing field) → the test panics.
#[test]
fn wait_signal_serde_roundtrip_and_legacy_default() {
    let roundtrip = |signal: WaitSignal| {
        let mut ns = NodeExecutionState::new();
        ns.wait_signal = Some(signal.clone());
        let json = serde_json::to_string(&ns).unwrap();
        let back: NodeExecutionState = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.wait_signal,
            Some(signal),
            "wait_signal must survive a serde roundtrip"
        );
    };
    roundtrip(WaitSignal::Webhook {
        callback_id: "cb-1".to_owned(),
    });
    roundtrip(WaitSignal::Approval {
        approver: "boss".to_owned(),
    });
    roundtrip(WaitSignal::Execution {
        execution_id: ExecutionId::new(),
    });

    // Legacy JSON without the field deserializes as `None`.
    let legacy = serde_json::json!({
        "state": "waiting",
        "attempts": [],
    });
    let ns: NodeExecutionState = serde_json::from_value(legacy).unwrap();
    assert!(
        ns.wait_signal.is_none(),
        "legacy rows without wait_signal must deserialize as None"
    );
}

/// W-S3a — `park_node` persists the resume-identity for a signal wait so a
/// later targeted Resume can match it. Covers the signal-only shape
/// (`Approval`) here; the signal+timeout shape is covered by
/// `park_node_sets_wait_wake`.
///
/// **Falsifiability**: drop the `wait_signal` field write from `park_node`
/// → `back.wait_signal` is `None` not `Some(Approval{..})` → the assert
/// fails.
#[test]
fn park_node_sets_wait_signal_for_signal_conditions() {
    let (mut state, n1, _n2) = make_state();
    state.start_node_attempt(n1.clone()).unwrap();

    state
        .park_node(
            n1.clone(),
            None,
            None,
            Some(WaitSignal::Approval {
                approver: "boss".to_owned(),
            }),
        )
        .expect("Running → Waiting signal park must succeed");

    let ns = state.node_state(n1).unwrap();
    assert_eq!(ns.state, NodeState::Waiting);
    assert_eq!(
        ns.wait_signal,
        Some(WaitSignal::Approval {
            approver: "boss".to_owned(),
        }),
        "park_node must persist the Approval resume-identity"
    );
}

/// W-S3a — a timer-driven wait (`Until` / `Duration`) carries NO
/// `wait_signal` (it is satisfied by a timer, never a Resume identity).
/// `park_node` records the timer + `Completion` discriminator and leaves
/// `wait_signal` `None`.
///
/// **Falsifiability**: have `park_node` stamp a `wait_signal` on the timer
/// path → the `is_none()` assert fails; or relax the classification guard
/// so a `Completion` wake with a `Some(wait_signal)` is accepted → the
/// rejection test below stops catching it.
#[test]
fn park_node_no_wait_signal_for_timer() {
    let (mut state, n1, _n2) = make_state();
    state.start_node_attempt(n1.clone()).unwrap();
    let wake_at = Utc::now() + chrono::Duration::seconds(30);

    state
        .park_node(n1.clone(), Some(wake_at), Some(WaitWake::Completion), None)
        .expect("Running → Waiting timer park must succeed");

    let ns = state.node_state(n1).unwrap();
    assert_eq!(ns.state, NodeState::Waiting);
    assert_eq!(ns.next_attempt_at, Some(wake_at));
    assert_eq!(ns.wait_wake, Some(WaitWake::Completion));
    assert!(
        ns.wait_signal.is_none(),
        "a timer wait must not carry a resume-identity"
    );
}

/// W-S3a — `park_node` REJECTS an inconsistent signal/timer classification
/// with a typed error and leaves the node untouched. A signal wait
/// (`wait_wake != Some(Completion)`) with no persisted identity would be
/// untargetable; a timer wait (`wait_wake == Some(Completion)`) with a
/// persisted identity would let a webhook/approval Resume mis-satisfy a
/// pure timer. Both are load-bearing guards, not debug-only asserts.
///
/// **Falsifiability**: drop the classification guard from `park_node` →
/// both `park_node` calls return `Ok`, the node becomes `Waiting` with an
/// inconsistent (`wait_wake`, `wait_signal`) shape → both `expect_err`
/// flip → RED.
#[test]
fn park_node_rejects_signal_classification_desync() {
    let (mut state, n1, _n2) = make_state();
    state.start_node_attempt(n1.clone()).unwrap();
    let wake_at = Utc::now() + chrono::Duration::seconds(30);

    // Signal-only park (wait_wake None) with NO identity — must be rejected.
    let err = state
        .park_node(n1.clone(), None, None, None)
        .expect_err("a signal park with no wait_signal must be rejected");
    assert!(
        matches!(err, ExecutionError::InvalidTransition { .. }),
        "the classification desync must surface as a typed InvalidTransition, got {err:?}"
    );

    // Timer park (wait_wake Completion) WITH an identity — must be rejected.
    let err = state
        .park_node(
            n1.clone(),
            Some(wake_at),
            Some(WaitWake::Completion),
            Some(WaitSignal::Webhook {
                callback_id: "cb".to_owned(),
            }),
        )
        .expect_err("a timer park with a wait_signal must be rejected");
    assert!(matches!(err, ExecutionError::InvalidTransition { .. }));

    // The guard ran before any mutation: the node is still Running, never
    // parked, and carries no stale wait metadata.
    let ns = state.node_state(n1).unwrap();
    assert_eq!(
        ns.state,
        NodeState::Running,
        "a rejected park must leave the node untouched (still Running)"
    );
    assert!(ns.next_attempt_at.is_none());
    assert!(ns.wait_wake.is_none());
    assert!(ns.wait_signal.is_none());
}

/// `total_retries` round-trips and starts
/// at zero.
#[test]
fn total_retries_roundtrip_and_default() {
    let (state, _n1, _n2) = make_state();
    assert_eq!(state.total_retries, 0);

    let json = serde_json::to_string(&state).unwrap();
    let back: ExecutionState = serde_json::from_str(&json).unwrap();
    assert_eq!(back.total_retries, 0);
}

/// Forward-compat: legacy `ExecutionState` JSON that predates
/// `total_retries` deserializes as `0`.
#[test]
fn total_retries_missing_field_deserializes_as_zero() {
    let legacy = serde_json::json!({
        "execution_id": ExecutionId::new(),
        "workflow_id": WorkflowId::new(),
        "status": "created",
        "node_states": {},
        "version": 0,
        "created_at": Utc::now(),
        "updated_at": Utc::now(),
        "total_output_bytes": 0,
    });
    let state: ExecutionState = serde_json::from_value(legacy).unwrap();
    assert_eq!(state.total_retries, 0);
}

/// `increment_total_retries` bumps both the
/// counter and the parent execution version (issue #255).
#[test]
fn increment_total_retries_bumps_version() {
    let (mut state, _n1, _n2) = make_state();
    let v0 = state.version;
    state.increment_total_retries();
    assert_eq!(state.total_retries, 1);
    assert_eq!(state.version, v0 + 1);
    state.increment_total_retries();
    assert_eq!(state.total_retries, 2);
    assert_eq!(state.version, v0 + 2);
}

/// `has_exhausted_retry_budget` reflects the cap when set, and
/// returns `false` when no cap is configured.
#[test]
fn has_exhausted_retry_budget_respects_cap() {
    let (mut state, _n1, _n2) = make_state();

    // No budget set — never exhausted.
    assert!(!state.has_exhausted_retry_budget());

    // Budget without cap — still not exhausted.
    state.set_budget(ExecutionBudget::default());
    assert!(!state.has_exhausted_retry_budget());

    // Cap = 2: counter 0 and 1 are under cap; 2 is exhausted.
    state.set_budget(ExecutionBudget::default().with_max_total_retries(2));
    assert!(!state.has_exhausted_retry_budget());
    state.increment_total_retries();
    assert!(!state.has_exhausted_retry_budget());
    state.increment_total_retries();
    assert!(state.has_exhausted_retry_budget());

    // Cap = 0 disables retry entirely from the start.
    let mut zero_cap =
        ExecutionState::new(ExecutionId::new(), WorkflowId::new(), &[node_key!("only")]);
    zero_cap.set_budget(ExecutionBudget::default().with_max_total_retries(0));
    assert!(zero_cap.has_exhausted_retry_budget());
}

/// ROADMAP §M0.3 — successful set bumps `version` and
/// `updated_at` so optimistic-concurrency readers observe the
/// change (issue #255).
#[test]
fn set_terminated_by_bumps_version_and_updated_at() {
    let (mut state, n1, _n2) = make_state();
    let v0 = state.version;
    let t0 = state.updated_at;

    let was_first = state.set_terminated_by(
        n1.clone(),
        ExecutionTerminationReason::ExplicitStop {
            by_node: n1,
            note: None,
        },
    );
    assert!(was_first);
    assert_eq!(state.version, v0 + 1, "version must be bumped on first set");
    assert!(state.updated_at >= t0, "updated_at must move forward");
}
