# nebula-workflow Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Workflows in Nebula are DAGs of nodes: typed structure (nodes, edges, parameters) that the engine loads, validates, and executes. A dedicated workflow crate keeps the DAG model, validation, and serialization in one place so that engine, API, and UI share the same schema.

**nebula-workflow is the workflow definition and DAG model for the Nebula platform.**

It answers: *What is the structure of a workflow (nodes, edges, metadata) and how is it validated and serialized for engine and API?*

```
Workflow definition (JSON or builder)
    ↓
Workflow type: nodes, edges, metadata; validated (nebula-validator, parameter schema)
    ↓
Engine loads workflow by ID; executes DAG; API/UI read and write definitions
```

This is the workflow contract: DAG is first-class; validation is declarative; schema is stable for storage and API.

---

## User Stories

### Story 1 — Engine Loads Workflow for Execution (P1)

Engine receives workflow_id; it loads workflow definition (from storage or API). Workflow type has nodes and edges; engine uses it to schedule and run nodes in topological order.

**Acceptance**:
- Workflow is a typed struct (nodes, edges, metadata); Serialize/Deserialize
- Load by ID from storage (storage is out of scope; workflow crate defines shape only)
- Validation (required fields, no cycles, valid node refs) before execution

### Story 2 — API and UI Read/Write Workflow Definitions (P1)

API exposes GET/POST workflow; UI edits and saves. Same Workflow type is used for persistence and API response. Schema is stable so that clients do not break.

**Acceptance**:
- Workflow schema is versioned; minor = additive fields
- Validation errors are structured (validator crate); API maps to 400 with field path
- No UI-specific fields in workflow crate; only DAG and metadata

### Story 3 — Validation Before Save or Run (P2)

Before saving or running, workflow is validated: DAG has no cycles, node refs exist, parameter schema is satisfied. Validation is in workflow crate or via validator integration.

**Acceptance**:
- validate() or similar returns ValidationErrors
- Cycles and invalid refs are explicit error variants
- Parameter schema validation (nebula-parameter) integrated where needed

---

## Core Principles

### I. DAG Is First-Class

**Workflow is a directed acyclic graph of nodes and edges. No cycles; topological order is defined.**

**Rationale**: Engine and scheduling depend on DAG semantics. Cycles would make execution undefined.

**Rules**:
- Nodes and edges are explicit; graph structure is queryable (e.g. successors, predecessors)
- Validation rejects cycles
- Optional: use petgraph or similar; keep public API stable

### II. Workflow Crate Does Not Execute

**Workflow crate owns structure and validation. It does not run nodes or schedule; engine does.**

**Rationale**: Execution is engine's responsibility. Workflow is data.

**Rules**:
- No dependency on engine or runtime for core types
- Engine depends on workflow (or shared workflow shape) for loading

### IV. Definition Is Design-Time Only (No Ephemeral Nodes)

**Workflow definitions describe only user-visible nodes and edges drawn on the canvas. Execution-only or engine-generated steps (ephemeral/recovery nodes, timers, gates) live in `nebula-execution` / `nebula-engine`, not in `WorkflowDefinition`.**

**Rationale**: Keeps the design-time DAG simple and stable for API/UI while allowing the execution layer to extend it with retry/wait/gate behavior without mutating stored workflows.

**Rules**:
- `WorkflowDefinition` contains only author-created nodes and connections.
- Engine and execution may extend the plan at run time with ephemeral nodes and patches for retries, waits, gates, and recovery, but those are not written back into workflow definitions.
- Execution views (timeline, waterfall) are free to visualize both user nodes and ephemeral system steps; workflow crate remains unaware of them.

### III. Schema Is Stable and Serializable

**Workflow type is Serialize/Deserialize; schema is versioned for API and storage compatibility.**

**Rationale**: Workflow definitions are long-lived. Breaking schema breaks stored workflows and clients.

**Rules**:
- Patch/minor: no breaking change to serialized form
- Major: MIGRATION.md for schema change
- Optional: schema snapshot tests for JSON shape

### V. Validation Is Declarative

**Validation rules (required fields, no cycles, valid refs) are declarative or composable (validator crate). No ad-hoc checks scattered in engine.**

**Rationale**: Single place for validation; consistent errors for API and UI.

**Rules**:
- validate() returns structured errors
- Optional integration with nebula-validator for combinators

---

## Production Vision

### The workflow model in an n8n-class fleet

In production, workflows are stored (by storage or api crate) and loaded by the engine. Workflow type has nodes (with action key, parameters, credential/resource refs) and edges (source → target). Validation runs on load and on save. API and UI consume the same schema. No execution in workflow crate — only structure and validation.

```
definition.rs — WorkflowDefinition (id: WorkflowId, name, description, version: Version, nodes, connections, variables, config, tags, created_at, updated_at)
             — WorkflowConfig (timeout, max_parallel_nodes, checkpointing, retry_policy), CheckpointingConfig, RetryConfig
node.rs      — NodeDefinition (id: NodeId, name, action_id: ActionId, interface_version, parameters: HashMap<String, ParamValue>, retry_policy, timeout), ParamValue (Literal, Reference)
connection.rs — Connection (from_node, to_node, condition: EdgeCondition, branch_key, from_port, to_port), EdgeCondition, ResultMatcher, ErrorMatcher
graph.rs     — DependencyGraph (petgraph DiGraph<NodeId, Connection>); from_definition, has_cycle, topological_sort, compute_levels
builder.rs   — WorkflowBuilder (fluent build with validation)
validate.rs  — validate_workflow(definition) → Vec<WorkflowError>
state.rs     — NodeState (execution progress)
error.rs     — WorkflowError (EmptyName, NoNodes, DuplicateNodeId, UnknownNode, SelfLoop, CycleDetected, NoEntryNodes, InvalidParameterReference, GraphError)
```

Workflow crate depends only on nebula-core and petgraph (no validator); validation is internal (validate_workflow). From the archives: workflow is in Core Layer; stable DAG model, validation before run/save, schema compatibility.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Formal Workflow type and DAG API | High | Ensure engine and API use same type |
| Cycle and ref validation | High | Reject invalid graphs |
| Schema snapshot tests | Medium | Lock JSON shape for compatibility |
| Version field for workflow definitions | Low | Support multiple schema versions |

---

## Key Decisions

### D-001: Workflow as Data, Not Executor

**Decision**: Workflow crate defines structure and validation; engine owns execution.

**Rationale**: Clear separation. Workflow is the contract for engine and API.

**Rejected**: Workflow crate running execution — would mix data and orchestration.

### D-002: DAG Stored as Nodes + Edges

**Decision**: Workflow is stored as list of nodes and list of edges (or equivalent); not as adjacency list only if that loses metadata.

**Rationale**: Serialization and API friendliness. Engine can build adjacency for scheduling.

**Rejected**: Execution-only format (e.g. compiled graph only) — would block API and UI from reading structure.

### D-003: Validation in Workflow Crate

**Decision**: validate() lives in workflow crate or is built from workflow types; validator crate used for composable rules.

**Rationale**: Single place for "is this workflow valid"; engine and API call it.

**Rejected**: Validation only in engine — would duplicate and scatter rules.

---

## Non-Negotiables

1. **DAG is first-class** — nodes and edges; no cycles.
2. **Workflow crate does not execute** — engine runs workflow; workflow is data.
3. **Schema is stable and serializable** — compatible in patch/minor.
4. **Validation before run/save** — structured errors; no invalid workflow execution.
5. **Breaking schema = major + MIGRATION.md** — storage and API depend on it.

---

## Governance

- **PATCH**: Bug fixes, docs. No schema change.
- **MINOR**: Additive (new optional fields). No removal.
- **MAJOR**: Breaking schema or validation. Requires MIGRATION.md.
