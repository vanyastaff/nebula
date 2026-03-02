# Roadmap

## Phase 1: Contract freeze and cleanup

**Status:** In progress

- Lock current stable surface (`Action`, metadata, components, result/output/error/ports). ✓
- Contract tests in `crates/action/tests/contracts.rs` for `ActionOutput`, `FlowKind`. ✓
- Compatibility policy in `COMPATIBILITY.md`. ✓
- Remove stale terminology in docs and examples (StatelessAction vs ProcessAction).

Exit criteria:
- No ambiguity between current API and aspirational design.
- Contract tests for serialization and compatibility pass. ✓

## Phase 2: Context and capability model

**Status:** In progress

- Replace temporary `NodeContext` bridge: `ActionContext` and `TriggerContext` added; `NodeContext` deprecated. ✓
- `Context` trait now has `execution_id()`, `node_id()`, `workflow_id()`, `cancellation()`.
- `StatelessAction` trait added (execute with `&impl Context`).
- Capability modules (resources, credentials, logger) to be added by runtime/sandbox as fields on context structs.

Exit criteria:
- Engine/sandbox/runtime can all implement the same context contract. ✓ (trait + concrete types)
- Capability checks map to deterministic action errors (future: SandboxViolation on undeclared access).

## Phase 3: Deferred and streaming hardening

- lock deferred/streaming resolution behavior expected from engine
- define compatibility matrix for downstream nodes consuming each output form
- document persistence/checkpoint requirements for long-running outputs

Exit criteria:
- resume/recovery scenarios for deferred outputs are fully specified
- streaming backpressure semantics are testable and documented

## Phase 4: Port and metadata governance

- freeze dynamic/support port schema semantics
- add compatibility checks for metadata version changes
- provide validation tools for action package authors

Exit criteria:
- CI-level contract validation for action packages
- clear migration guide for version bumps

## Phase 5: Ecosystem and DX rollout

- publish end-to-end examples with runtime + action implementations
- define recommended error-to-retry mapping patterns
- deliver ergonomic authoring layer in same crate (e.g. dx/authoring module)

Exit criteria:
- external action authors can build n8n-style nodes with predictable behavior
- runtime and sandbox integrations are documented end-to-end
