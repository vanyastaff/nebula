# Architecture

## Problem Statement

- **Business problem:** Workflows must run end-to-end: load definition, schedule nodes, pass data, handle results, persist state, and expose observability. A single orchestrator must own this lifecycle.
- **Technical problem:** Coordinate workflow, execution, runtime, expression, plugin, resource, and telemetry without implementing actions or storage backends; keep scheduling deterministic and state durable.

## Current Architecture

- **Module map:** engine (WorkflowEngine, execute_workflow), resolver (ParamResolver), result (ExecutionResult), error (EngineError). WorkflowEngine holds runtime, event_bus, metrics, action_keys, plugin_registry, param_resolver, expression_engine, optional resource_manager.
- **Data/control flow:** execute_workflow → load workflow → build plan → frontier loop (resolve params → runtime.execute_node → handle ActionResult) → persist state → emit events → return ExecutionResult.
- **Known bottlenecks:** State store integration (gap); trigger lifecycle and backpressure not yet fully integrated.

## Target Architecture

- **Target module map:** Same core; add optional integration points for state store backend, trigger lifecycle, admission control.
- **Public contract boundaries:** execute_workflow, ExecutionResult, ExecutionHandle (if exposed); context and event shapes are contract.
- **Internal invariants:** No action implementation in engine; events never block execution; state transitions align with execution crate.

## Design Reasoning

- **Trade-off:** Engine depends on many crates (workflow, execution, runtime, action, expression, plugin, telemetry, resource); keeps engine as orchestrator only, no domain logic duplication.
- **Rejected:** Runtime calling back into engine for scheduling — would create circular dependency.

## Comparative Analysis

Sources: n8n, Node-RED, Temporal/Prefect.

- **Adopt:** Central orchestrator owning execution lifecycle; frontier/scheduling from DAG; events for observability only (n8n/Temporal style).
- **Reject:** Embedding action implementations in engine; synchronous event delivery that blocks execution.
- **Defer:** Distributed engine (multi-node); trigger registry in separate service.

## Breaking Changes (if any)

- Execution or context contract change: major; see MIGRATION.md.

## Open Questions

- State store abstraction ownership (engine vs execution crate).
- Trigger lifecycle API shape (engine vs runtime).
