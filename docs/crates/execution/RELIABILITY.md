# Reliability

## SLO Targets

- **Availability:** Execution crate is a library; no direct availability SLO. Engine and API SLOs depend on persistence and runtime; execution state must be persisted and loaded correctly.
- **Latency:** Transition validation and plan build are in-process and fast; no I/O in execution crate. IdempotencyManager::check_and_mark is O(1) in-memory.
- **Error budget:** N/A for library. Engine should treat InvalidTransition, PlanValidation, DuplicateIdempotencyKey as deterministic failures (no retry of same transition/key).

## Failure Modes

- **Invalid transition:** Engine or bug applies invalid transition. Mitigation: always use validate_* before mutating; state machine tests cover all pairs. Consequence: ExecutionError returned; state unchanged.
- **Plan validation failure:** Empty workflow or broken graph. Mitigation: Engine validates workflow before creating execution; PlanValidation returned from from_workflow. Consequence: execution not created or error returned to caller.
- **Duplicate idempotency key:** Same key used twice (retry or bug). Mitigation: check_and_mark before run; return cached result or DuplicateIdempotencyKey. Consequence: no double execution; idempotent behavior.
- **Serialization failure:** State or output fails to serialize/deserialize (e.g. corrupt storage). Mitigation: Execution crate returns Serialization error; engine/storage must handle and optionally retry load. Consequence: execution may be unrecoverable if state is corrupt.

## Resilience Strategies

- **Retry policy:** Not applicable inside execution crate. Engine may retry plan build if workflow definition is updated; should not retry same transition or same idempotency key.
- **Circuit breaking:** N/A.
- **Fallback behavior:** None; invalid transition or plan validation is hard failure.
- **Graceful degradation:** If IdempotencyManager is unavailable (e.g. future persistent store down), engine may fail open (run anyway and risk duplicate) or fail closed (reject run); policy is engine’s.

## Operational Runbook

- **Alert conditions:** N/A for execution crate. Engine should alert on high InvalidTransition or DuplicateIdempotencyKey rate (may indicate bug or abuse).
- **Dashboards:** Execution status distribution, transition counts, idempotency hit rate (from engine/telemetry).
- **Incident triage:** If executions stuck in non-terminal state, check transition rules and engine logic; ensure no invalid transition was applied. If duplicate executions observed, check idempotency key generation and store.

## Capacity Planning

- **Load profile assumptions:** Execution state and journal size grow with node count and output size. Large workflows and long journals may stress persistence layer; execution crate does not limit size (engine/storage may).
- **Scaling constraints:** IdempotencyManager in-memory does not scale across processes; persistent idempotency store required for multi-worker deployment.
