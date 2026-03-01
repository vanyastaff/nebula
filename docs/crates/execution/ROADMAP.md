# Roadmap

## Phase 1: Contract and State Machine (Current)

- **Deliverables:** ExecutionStatus, ExecutionState, NodeExecutionState, validated transitions, ExecutionPlan, JournalEntry, NodeOutput/ExecutionOutput, NodeAttempt, IdempotencyKey/IdempotencyManager, ExecutionError. Unit tests for transitions and serde.
- **Risks:** None; implemented and stable.
- **Exit criteria:** All transition tests pass; serde roundtrip for state and output; engine can build plan and apply transitions.

## Phase 2: API and Schema Stability

- **Deliverables:** Formal schema snapshot (e.g. JSON fixtures) for ExecutionState, NodeOutput, JournalEntry for API compatibility tests. Document serialized form in API.md. Optional: resume token type for suspend/resume.
- **Risks:** Schema drift if engine or API adds fields without going through execution crate.
- **Exit criteria:** Fixtures in repo; CI checks that public types roundtrip; API contract tests use execution types.

## Phase 3: Idempotency and Resume

- **Deliverables:** Align with nebula-idempotency for persistent key store; IdempotencyKey format remains stable. Optional: resume token (e.g. for Paused or wait-for-webhook) and Resume variant or field in state.
- **Risks:** Idempotency crate may want to own key type; coordination on key format and DuplicateIdempotencyKey semantics.
- **Exit criteria:** Idempotency key format documented and unchanged; engine or idempotency crate can persist keys; resume path documented if implemented.

## Phase 4: Observability and Operational Hooks

- **Deliverables:** JournalEntry and state transitions sufficient for audit and metrics; no new crate responsibility. Optional: execution duration, node duration aggregates in state or journal for dashboards.
- **Risks:** None if limited to existing types.
- **Exit criteria:** Engine and telemetry can derive metrics from state and journal; no breaking change to execution crate.

## Metrics of Readiness

- **Correctness:** All transition matrix tests pass; no invalid transition accepted; idempotency check_and_mark semantics verified.
- **Stability:** Serialized form of ExecutionStatus, ExecutionState, NodeOutput, JournalEntry unchanged in patch/minor.
- **Operability:** State and journal are sufficient for “get execution status” and “list executions” API; engine persists and loads without loss.
