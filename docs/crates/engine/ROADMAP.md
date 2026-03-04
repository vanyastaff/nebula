# Roadmap

Phased path to production-ready workflow execution orchestration. Aligned with [CONSTITUTION.md](./CONSTITUTION.md): engine owns lifecycle, state is durable, events are fire-and-forget.

## Phase 1: Contract and State Integration

- **Deliverables:**
  - Full state store backend integration: execution state persisted and reloadable (e.g. via nebula-storage/execution); engine uses it for restart and query.
  - Contract tests for `execute_workflow`, `ExecutionResult`, and handoff to runtime; documented in API.md.
  - ExecutionContext and execution lifecycle (create, schedule, persist, emit events) stable and documented.
- **Risks:**
  - State store API drift between engine and storage/execution crates.
- **Exit criteria:**
  - Single-node and multi-node runs persist state; engine can resume or query execution by ID.
  - No action implementation in engine; all execution via runtime/sandbox.

## Phase 2: Runtime Hardening

- **Deliverables:**
  - Trigger lifecycle (if adopted): engine or dedicated service owns register/unregister and start/stop; trigger types in nebula-action.
  - Backpressure and admission: integrate with nebula-system pressure events; optional admission control under load.
  - Deterministic scheduling and wait/suspend paths documented and tested.
- **Risks:**
  - Trigger lifecycle coupling engine to runtime/action in complex ways.
  - Admission policy too strict or too loose for production load.
- **Exit criteria:**
  - Scheduling order defined by DAG and explicit wait/trigger; no hidden non-determinism.
  - Under pressure, engine behavior (reject vs queue) is configurable and observable.

## Phase 3: Observability and Operations

- **Deliverables:**
  - EventBus and metrics sufficient for "list executions", "execution detail", and dashboards; no blocking on event delivery.
  - Optional idempotency/deduplication (align with nebula-idempotency) for execution keys.
  - Operational hooks: execution duration, node-level aggregates where needed for telemetry.
- **Risks:**
  - Adding observability that blocks execution path.
- **Exit criteria:**
  - Engine and telemetry derive metrics from state/events; fire-and-forget event contract preserved.
  - Idempotency key format (if used) documented and stable.

## Phase 4: Ecosystem and DX

- **Deliverables:**
  - Clear contract for API/worker: how to start, cancel, and query executions.
  - Migration path for any execution or context contract change (MIGRATION.md).
  - Cookbook or examples for engine + runtime + storage composition.
- **Risks:**
  - Multiple entry points (API, CLI, tests) diverging in how they use engine.
- **Exit criteria:**
  - Single documented composition pattern; breaking execution/context contract = major + MIGRATION.

## Metrics of Readiness

- **Correctness:** All scheduling and state transitions tested; no invalid state; idempotency semantics clear.
- **Latency:** Execution overhead and event emission do not block execution path.
- **Stability:** State and context contract stable in patch/minor; no panics in hot path.
- **Operability:** State queryable; events sufficient for monitoring and debugging.
