# nebula-workflow

Workflow definition, DAG graph, and validation for the Nebula workflow engine. Defines the structure (nodes, edges, metadata) that engine, API, and storage consume.

## Scope

- **In scope:**
  - **definition** — `WorkflowDefinition` (id, name, version, nodes, connections, variables, config, tags, created_at, updated_at); `WorkflowConfig`, `CheckpointingConfig`, `RetryConfig`.
  - **node** — `NodeDefinition` (id, name, action_id, interface_version, parameters, retry_policy, timeout); `ParamValue` (Literal, Reference).
  - **connection** — `Connection` (from_node, to_node, condition, branch_key, from_port, to_port); `EdgeCondition`, `ResultMatcher`, `ErrorMatcher`.
  - **graph** — `DependencyGraph` (petgraph-based); `from_definition`, `has_cycle`, `topological_sort`, `compute_levels`.
  - **builder** — `WorkflowBuilder` (fluent construction with validation).
  - **validate** — `validate_workflow(definition)` → `Vec<WorkflowError>`.
  - **state** — `NodeState` (execution progress).
  - **error** — `WorkflowError` (EmptyName, NoNodes, DuplicateNodeId, UnknownNode, SelfLoop, CycleDetected, NoEntryNodes, InvalidParameterReference, GraphError).
- **Out of scope:** Execution, scheduling, engine logic; storage/API implementation (workflow defines shape only).

## Current State

- **Maturity:** Stable DAG and validation; used by engine, execution, sdk.
- **Dependencies:** nebula-core, petgraph, serde, thiserror, chrono (no nebula-validator).

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [INTERACTIONS.md](./INTERACTIONS.md) — ecosystem, upstream/downstream, contracts
- Further docs (ARCHITECTURE, API, DECISIONS, ROADMAP, PROPOSALS, SECURITY, RELIABILITY, TEST_STRATEGY, MIGRATION) to be added per [docs-deep-pass-checklist.md](../../docs-deep-pass-checklist.md).

## Archive

Legacy material: [\_archive/](./_archive/)
