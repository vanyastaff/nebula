# Roadmap

Phased path to production-ready workflow definition and DAG model. Platform role: engine/API/UI share the same schema.

## Phase 1: Contract and Schema Baseline

- **Deliverables:**
  - Formal `WorkflowDefinition` and DAG API used by engine and API; no divergent types.
  - Cycle and ref validation: `validate_workflow()` rejects invalid graphs; structured `WorkflowError`.
  - Docs (ARCHITECTURE, API) aligned with current types (definition, node, connection, graph, builder).
- **Risks:**
  - Engine or API introducing workflow-shaped types outside this crate, causing drift.
- **Exit criteria:**
  - Engine and API depend on workflow crate for definition and graph only.
  - All validation paths covered by tests; no invalid workflow accepted for execution.

## Phase 2: Schema Stability and Compatibility

- **Deliverables:**
  - Schema snapshot tests (e.g. JSON fixtures) for `WorkflowDefinition`, nodes, connections; CI enforces roundtrip.
  - Version field and compatibility policy: patch/minor = additive only; major = MIGRATION.md.
  - Document serialized form in API.md; compatibility rules in MIGRATION.md or CONSTITUTION.
- **Risks:**
  - New fields added without snapshot update; breaking clients or storage.
- **Exit criteria:**
  - Fixtures in repo; CI checks public types roundtrip; no breaking change without major + MIGRATION.

## Phase 3: Validation and Integrations

- **Deliverables:**
  - Optional integration with nebula-validator for composable rules (if adopted).
  - Validation errors sufficient for API 400 responses with field path.
  - No UI-only or execution-only fields in workflow definition; design-time DAG only.
- **Risks:**
  - Scope creep: ephemeral nodes or execution state leaking into definition.
- **Exit criteria:**
  - Validation contract documented; API and engine use same validation entry point.
  - Definition remains design-time only; execution extensions live in execution/engine.

## Phase 4: Ecosystem and DX

- **Deliverables:**
  - Builder and validation ergonomics for API and CLI (workflow create/edit).
  - Migration tooling or guidance for schema version bumps.
  - Operator guidance: when to validate, where errors surface.
- **Risks:**
  - Fragmentation between builder API and raw struct usage.
- **Exit criteria:**
  - Clear path for workflow authoring and validation; low-friction adoption for API/UI.

## Metrics of Readiness

- **Correctness:** All validation cases (cycle, refs, empty, duplicates) covered; no invalid DAG executed.
- **Stability:** Serialized form stable in patch/minor; snapshot tests green.
- **Operability:** Schema and errors sufficient for "save workflow" and "run workflow" API and engine load.
