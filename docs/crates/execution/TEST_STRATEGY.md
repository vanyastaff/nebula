# Test Strategy

## Test Pyramid

- **Unit:** Crate has extensive unit tests: ExecutionStatus (terminal, active, success, failure, Display, serde); ExecutionState and NodeExecutionState (new, transition_to, transition_status, all_nodes_terminal, active_node_ids, completed_node_ids, failed_node_ids, set_node_state, serde); transition (can_transition_* and validate_* for execution and node); NodeOutput and ExecutionOutput (inline, blob_ref, serde); NodeAttempt (new, complete_success, complete_failure, duration, serde); IdempotencyKey (generate, determinism, different attempts); IdempotencyManager (check_and_mark, is_seen, len, clear); ExecutionError (display, from serde); ExecutionPlan::from_workflow (valid and invalid workflows). All in `crates/execution/src/*.rs` under `#[cfg(test)]`.
- **Integration:** Engine (or e2e) tests that create execution, build plan, apply transitions, persist and load state, append journal, check idempotency. Not in execution crate; in engine or workspace e2e.
- **Contract:** Serialization roundtrip for ExecutionStatus, ExecutionState, NodeOutput, JournalEntry; transition matrix (allowed and disallowed pairs). Optional: JSON fixtures for API compatibility (Phase 2).
- **End-to-end:** Out of scope for execution crate; engine and API own e2e.

## Critical Invariants

- **Invariant 1:** For any ExecutionState, transition_status(to) succeeds only if can_transition_execution(state.status, to). Otherwise ExecutionError::InvalidTransition and state unchanged.
- **Invariant 2:** For any NodeExecutionState, transition_to(to) succeeds only if can_transition_node(state.state, to). Otherwise ExecutionError::InvalidTransition and state unchanged.
- **Invariant 3:** IdempotencyKey::generate(exec_id, node_id, attempt) is deterministic; same inputs ⇒ same key string.
- **Invariant 4:** IdempotencyManager::check_and_mark(key) returns true exactly once per key; subsequent calls for same key return false.
- **Invariant 5:** ExecutionStatus and NodeState terminal states never transition to non-terminal (enforced by transition tables).

## Scenario Matrix

- **Happy path:** Create state → transition to Running → node Ready → Running → Completed → execution Completed. Plan from valid workflow; idempotency check_and_mark true then run; journal entries appended. Covered by unit tests.
- **Retry path:** Node Failed → Retrying → Running → Completed. Idempotency key with attempt 1; check_and_mark true. Covered by transition and attempt tests.
- **Cancellation path:** Running → Cancelling → Cancelled. Node Running → Cancelled. Covered by transition tests.
- **Timeout path:** Running → TimedOut. Covered by transition tests.
- **Invalid transition:** Created → Completed (rejected); Completed → Running (rejected). Covered by transition tests.
- **Upgrade/migration path:** Serde roundtrip ensures old serialized state (if format unchanged) still deserializes. New optional fields in minor with default.

## Tooling

- **Property testing:** Optional: proptest or quickcheck for transition sequences (any sequence of valid transitions from Created leads to valid state).
- **Fuzzing:** Optional: serde fuzz for ExecutionState/NodeOutput/JournalEntry to catch malformed input.
- **Benchmarks:** Optional: plan build and transition hot path if needed.
- **CI quality gates:** `cargo test -p nebula-execution`; `cargo clippy -p nebula-execution`; no unsafe code.

## Exit Criteria

- **Coverage goals:** All transition pairs (allowed and disallowed) tested; all public types have serde roundtrip; idempotency key and manager behavior tested.
- **Flaky test budget:** Zero; tests are deterministic.
- **Performance regression thresholds:** N/A for current scope; plan build and transition are in-process only.
