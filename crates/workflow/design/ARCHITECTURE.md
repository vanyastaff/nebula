# Architecture

## Problem Statement

- **Business problem:** Engine, API, and UI must share a single workflow schema; validation and DAG semantics must live in one place so that invalid workflows never reach execution.
- **Technical problem:** Workflow definition (nodes, edges, metadata) must be serializable, versioned, and queryable as a DAG without coupling to execution or storage implementation.

## Current Architecture

- **Module map:** definition (WorkflowDefinition, config), node (NodeDefinition, ParamValue), connection (Connection, EdgeCondition), graph (DependencyGraph from petgraph), builder (WorkflowBuilder), validate (validate_workflow), state (NodeState), error (WorkflowError).
- **Data/control flow:** Definition → validate_workflow → DependencyGraph::from_definition → topological_sort / compute_levels; engine consumes graph and definition.
- **Known bottlenecks:** Validation is synchronous; large graphs may need incremental validation or size limits (out of scope for crate; engine/storage may enforce).

## Target Architecture

- **Target module map:** Same; no execution or storage in crate. Optional: schema snapshot module for JSON fixtures.
- **Public contract boundaries:** WorkflowDefinition, NodeDefinition, Connection, DependencyGraph, validate_workflow, WorkflowError are the stable surface; builder and state are supporting.
- **Internal invariants:** DAG has no cycles; all node refs in edges exist in nodes; validation is the single gate before engine/API accept a definition.

## Design Reasoning

- **Trade-off:** Workflow crate does not depend on nebula-validator; validation is internal. Allows minimal deps; optional future integration with validator combinators.
- **Rejected:** Putting validation only in engine — would duplicate rules and scatter errors; API and UI need the same validation.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces/Activeflow.

- **Adopt:** DAG as first-class (nodes + edges), validation before save/run, stable serialized schema for API and storage (n8n/Node-RED style).
- **Reject:** Execution logic in workflow definition crate; ephemeral nodes in stored definition.
- **Defer:** Optional nebula-validator integration for composable rules; multi-version schema migration tooling.

## Breaking Changes (if any)

- Schema or validation contract change: major version; see MIGRATION.md.

## Open Questions

- Formal schema snapshot location (this crate vs shared fixtures repo).
- Version field semantics for workflow definitions (single version vs multi-version support).
