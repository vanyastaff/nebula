# Test Strategy

## Test Pyramid

- **Unit:** Builder build success/failure; TestContext and MockExecution construction; prelude compiles and exports expected types.
- **Integration:** Build node with sdk (derive or builder), run with TestContext or ExecutionHarness; assert output. Optional: run same node via engine test harness (contract test).
- **Contract:** Prelude and builder output compatible with action contract; TestContext shape matches runtime context expectations.
- **E2E:** Out of scope for sdk (engine/API own E2E).

## Critical Invariants

- Prelude re-exports compile and are documented. Any removal or signature change is breaking (major).
- TestContext and MockExecution allow running action-compatible nodes without full engine; result matches expected contract.
- Builder output implements Action (or required traits) and works with engine/runtime when integrated.

## Scenario Matrix

- **Happy path:** Author uses prelude + derive or builder → compiles → runs in test with TestContext or in engine.
- **Compatibility path:** After action or runtime change, sdk compatibility test (if added) passes or fails explicitly; no silent break.
- **Migration path:** When prelude or builder breaks (major), MIGRATION.md and upgrade guide.

## Tooling

- **CI:** cargo test with default and full features; optional contract test with action/engine.
- **Benchmarks:** N/A unless builder performance is critical.

## Exit Criteria

- All builder and TestContext/MockExecution paths covered by tests; prelude documented. No flaky tests. Compatibility test (when added) in CI.
