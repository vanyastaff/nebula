## D-006: Ephemeral Execution Nodes via Patches, Not Workflow Mutation

**Status:** Planned

**Context:** Engine needs to express retry delays, waits on external resources, and other recovery steps that do not exist in the design-time workflow DAG. We want rich Execution Views (timeline with "phantom" nodes like WaitResourceHealthy or RetryDelay) and durable replay, without mutating or resaving the original `WorkflowDefinition`.

**Decision:** Execution-time extensions to the plan (ephemeral/phantom nodes) will be modeled as data in `nebula-execution` (e.g. future `ExecutionPatch` and/or dedicated journal variants), derived deterministically from history and resilience policy. The workflow crate remains design-time only; engine never writes ephemeral nodes back into workflow definitions.

**Alternatives considered:** (1) Mutating the workflow graph at run time by inserting system nodes — rejected to keep stored workflows and design-time DAGs simple and stable; (2) Hiding all retry/wait behavior inside engine-only logic and logs — rejected because it makes Execution View opaque and replay semantics unclear.

**Trade-offs:** Adding an explicit patch/ephemeral-node model increases surface area in execution crate, but makes Execution View, debugging, and replay consistent across engine implementations. UI can show "phantom" nodes based on execution data, even though they are not part of the author-edited workflow.

**Consequences:** For a given workflow, input, journal, and resilience policy, the extended execution graph (including ephemeral steps) must be reproducible. Engine and UI will treat the execution graph as `WorkflowDefinition + ExecutionPlan + patches/journal`, never as a mutated workflow. Any future `ExecutionPatch` or ephemeral-node schema changes live in this crate and are versioned like other execution types.

**Migration impact:** Initial introduction is additive (new types/variants). Future breaking changes to patch/ephemeral-node representation require MIGRATION.md and careful handling in any persistence or Execution View consumers.

**Validation plan:** (1) Unit tests that reconstruct an extended execution graph from workflow + plan + journal/patches; (2) Golden tests for Execution View serialization; (3) Integration tests with engine that assert ephemeral wait/retry behavior is replayable from stored execution data.

# Decisions

## D-001: State Machine and Transitions in This Crate

**Status:** Adopt

**Context:** Engine and API need a single definition of execution and node states and allowed transitions. Duplicating rules in engine would risk drift and inconsistent persistence.

**Decision:** Execution crate owns `ExecutionStatus` and (with `nebula_workflow::NodeState`) node state; provides `validate_execution_transition` and `validate_node_transition`. Engine calls these before mutating state. Allowed transitions are explicit in code and tests.

**Alternatives considered:** Engine-owned transition table — rejected to avoid engine depending on execution only for types while owning rules. Shared “state machine” crate — rejected to keep execution as the single place for execution-state vocabulary.

**Trade-offs:** Any new transition (e.g. Suspended→Running for resume) requires a change in execution crate and possibly workflow crate for node state.

**Consequences:** Engine must use execution crate validators; no ad-hoc transitions in engine. API and storage see consistent state values.

**Migration impact:** None for current use. Adding new status or transition is additive (minor).

**Validation plan:** Unit tests in crate for every allowed and disallowed transition pair; engine integration tests that apply transitions through crate API.

---

## D-002: Idempotency Key Format and In-Memory Manager Here

**Status:** Adopt

**Context:** Worker retry and at-least-once delivery require duplicate detection. Key format must be deterministic and stable so that persistent idempotency (future) can reuse it.

**Decision:** `IdempotencyKey::generate(execution_id, node_id, attempt)` produces string `{execution_id}:{node_id}:{attempt}`. `IdempotencyManager` in this crate is in-memory (HashSet); persistent store is out of scope (nebula-idempotency or engine).

**Alternatives considered:** Key in engine only — rejected so API and idempotency crate share one format. Key in nebula-idempotency only — rejected because execution and attempt tracking already need the key type here; idempotency crate can depend on execution for key type or re-export.

**Trade-offs:** IdempotencyManager is process-local; no cross-process dedupe until persistent backend exists.

**Consequences:** Engine or runtime calls `check_and_mark` before running node; on duplicate, returns cached result or `ExecutionError::DuplicateIdempotencyKey`. Key format is stable for storage backends.

**Migration impact:** If key format ever changes (major), all stored keys must be migrated or versioned.

**Validation plan:** Unit tests for key determinism and manager check_and_mark semantics; integration with engine when worker/idempotency is implemented.

---

## D-003: No Persistence in Execution Crate

**Status:** Adopt

**Context:** Where should execution state and journal be stored? Execution crate could own a storage trait, or engine/storage could own persistence.

**Decision:** Execution crate defines types only. Engine (or storage crate) is responsible for persisting `ExecutionState`, `JournalEntry`, and any idempotency store. No `Storage` or `ExecutionStore` trait in execution crate.

**Alternatives considered:** Execution crate with optional storage trait — rejected to keep crate free of I/O and storage backends. Engine-only persistence — adopted; engine uses nebula-storage or own adapter to persist execution-shaped data.

**Trade-offs:** Engine must know how to serialize and store execution state; execution crate stays dependency-light (core, workflow only).

**Consequences:** Clear boundary: execution = vocabulary and validation; engine = lifecycle and persistence. API reads execution state from engine/storage, not from execution crate.

**Migration impact:** None. Persistence format is engine’s choice as long as it uses execution types for in-memory representation.

**Validation plan:** Engine and storage tests cover persist/load of ExecutionState and journal.

---

## D-004: ExecutionPlan from Workflow in This Crate

**Status:** Adopt

**Context:** Execution plan (parallel groups, entry/exit nodes) is derived from workflow definition. Plan could live in engine or in execution crate.

**Decision:** `ExecutionPlan::from_workflow(execution_id, workflow, budget)` lives in execution crate. Uses `nebula_workflow::DependencyGraph` and `compute_levels()`; returns `ExecutionError::PlanValidation` on empty workflow or graph errors.

**Alternatives considered:** Plan build in engine — rejected so plan type and build logic stay next to execution state. Plan in workflow crate — rejected because plan is execution-scoped (execution_id, budget) and ties to execution lifecycle.

**Trade-offs:** Execution crate depends on nebula-workflow; workflow crate must expose DependencyGraph and level computation.

**Consequences:** Engine calls `ExecutionPlan::from_workflow` once per execution; plan is immutable after creation. Changes to workflow definition require new execution and new plan.

**Migration impact:** None. Plan shape is additive (e.g. new budget fields) in minor.

**Validation plan:** Unit tests in plan.rs for from_workflow with valid/invalid workflows; engine uses plan for scheduling.

---

## D-005: JournalEntry as Enum in This Crate

**Status:** Adopt

**Context:** Audit and replay need a serializable log of execution events. Who owns the event schema?

**Decision:** `JournalEntry` is an enum in execution crate (ExecutionStarted, NodeScheduled, NodeStarted, NodeCompleted, NodeFailed, NodeSkipped, etc.). All variants carry timestamp and relevant ids. Engine appends entries; storage persists (engine responsibility). Serialized with `#[serde(tag = "event", rename_all = "snake_case")]`.

**Alternatives considered:** Journal in engine — rejected so API and storage can depend on a stable event schema without pulling in engine. Separate “journal” crate — rejected for now; execution and journal are tightly coupled (node_id, execution_id, status).

**Trade-offs:** New event types require execution crate change and possibly migration for existing journal consumers.

**Consequences:** Single schema for execution audit; engine must not invent event types outside this enum.

**Migration impact:** Additive variants in minor; removing or renaming variants is major with MIGRATION.md.

**Validation plan:** Serde roundtrip tests for all JournalEntry variants; engine tests append and persist journal.
