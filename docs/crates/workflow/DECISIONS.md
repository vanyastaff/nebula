# Decisions

## D-001: Workflow as Data, Not Executor

**Status:** Adopt

**Context:** Need clear ownership of workflow structure vs execution lifecycle.

**Decision:** Workflow crate defines structure and validation only; engine owns execution, scheduling, and state.

**Alternatives considered:** Workflow crate owning a "compiled" execution plan — rejected to keep API/UI and storage schema simple and avoid execution details in definition.

**Trade-offs:** Engine and execution crates depend on workflow; workflow has no dependency on engine/runtime.

**Consequences:** Any execution-time extension (ephemeral nodes, retry gates) lives in execution/engine; workflow definition remains design-time only.

**Migration impact:** None; this is the current design.

**Validation plan:** Contract tests: engine loads workflow types only; no execution code in workflow crate.

---

## D-002: DAG Stored as Nodes + Edges

**Status:** Adopt

**Context:** Serialization and API/UI need to read and write workflow structure.

**Decision:** Workflow is stored as list of nodes and list of edges (connections); engine builds adjacency/graph for scheduling.

**Alternatives considered:** Storing only compiled adjacency — rejected because API and UI need to display and edit nodes/edges with metadata.

**Trade-offs:** Slightly more work for engine to build graph from definition; benefit is single schema for storage, API, and UI.

**Consequences:** WorkflowDefinition schema is the single source of truth; DependencyGraph is derived.

**Migration impact:** Any future change to node/connection shape is a schema change (major + MIGRATION).

**Validation plan:** Schema snapshot tests (when added) lock JSON shape.

---

## D-003: Validation in Workflow Crate

**Status:** Adopt

**Context:** Engine and API both need "is this workflow valid?" with consistent errors.

**Decision:** `validate_workflow()` lives in workflow crate; returns structured WorkflowError list. Optional future use of nebula-validator for composable rules.

**Alternatives considered:** Validation only in engine — rejected to avoid duplication and to allow API to validate before save.

**Trade-offs:** Workflow crate owns validation rules; no dependency on nebula-validator today.

**Consequences:** Single entry point for validation; API maps WorkflowError to 400 with field path.

**Migration impact:** None.

**Validation plan:** Unit tests for all error variants; integration tests with engine (validate then run).
