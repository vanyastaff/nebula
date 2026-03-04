# Test Strategy

## Test Pyramid

- **Unit:** validate_workflow for each WorkflowError variant (cycle, duplicate node, unknown node, self-loop, no entry nodes, invalid ref, empty name, no nodes, graph error). WorkflowBuilder build success/failure. DependencyGraph::from_definition, topological_sort, compute_levels for known graphs.
- **Integration:** Engine (or test harness) loads workflow, validates, builds graph, runs; workflow crate only up to graph build. Optional: API roundtrip (serialize definition, validate, deserialize).
- **Contract:** Engine and API consume workflow types only; no execution logic in workflow crate. Schema: when fixtures exist, roundtrip and optional schema diff.
- **E2E:** Out of scope for workflow crate (engine/API own E2E).

## Critical Invariants

- After validate_workflow(definition) returns empty, DependencyGraph::from_definition(definition) succeeds and topological_sort returns a valid order.
- Any workflow with a cycle or invalid ref fails validate_workflow.
- Serialized WorkflowDefinition roundtrips (serde) without loss in patch/minor.

## Scenario Matrix

- **Happy path:** Valid definition → validate empty → graph build → topological order.
- **Validation path:** Invalid definition → validate returns errors; graph build not attempted or fails.
- **Upgrade/migration path:** When schema version changes (major), MIGRATION.md and optional migration tests.

## Tooling

- **Property testing:** Optional: proptest for random DAGs, assert no cycle implies valid order.
- **Benchmarks:** Optional: validate + graph build for large node/edge counts.
- **CI quality gates:** cargo test; optional schema snapshot check when added.

## Exit Criteria

- All validation variants covered by unit tests; at least one integration test with engine or harness.
- No flaky tests; validation is deterministic.
- Performance: no regression target specified; keep validation and graph build fast for typical definition sizes.
