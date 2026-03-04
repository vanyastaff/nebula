# Test Strategy

## Test Pyramid

- **Unit:** ParamResolver with mock expression engine; ExecutionResult and EngineError construction; scheduling order for fixed DAG (mock runtime returns known results).
- **Integration:** Engine + real workflow + mock runtime + mock event bus; full execute_workflow with 2–3 nodes; assert ExecutionResult and event order. Optional: engine + execution crate state persistence.
- **Contract:** Engine does not implement Action; context shape matches runtime/action expectations; EventBus send is non-blocking.
- **E2E:** API → engine → runtime → action (optional); out of scope for engine crate alone (API/worker test).

## Critical Invariants

- After execute_workflow returns, ExecutionResult.status and node_outputs are consistent with what runtime reported.
- Events (Started, NodeCompleted, NodeFailed, Completed) are emitted in order and do not block execution.
- Execution budget and timeout are enforced when implemented.

## Scenario Matrix

- **Happy path:** Valid workflow → all nodes succeed → Completed event → ExecutionResult with status and outputs.
- **Retry path:** Node returns Retry → engine applies policy → retry or fail.
- **Cancellation path:** Cancel requested → engine stops scheduling → Cancelled or equivalent status.
- **Timeout path:** Execution budget or timeout exceeded → engine stops → status and event reflect timeout.
- **Migration path:** N/A for engine (no persisted schema in crate); execution/state store may have migration.

## Tooling

- **Property testing:** Optional: random DAG and mock node results; assert no panic and result consistent.
- **Benchmarks:** execute_workflow with N nodes and mock runtime; measure overhead.
- **CI quality gates:** cargo test; integration tests with mock runtime and event bus.

## Exit Criteria

- Unit tests for resolver and error mapping; at least one integration test with multi-node workflow.
- No flaky tests; events and result deterministic for given inputs.
- Performance: engine overhead acceptable for typical workflow sizes (benchmark if added).
