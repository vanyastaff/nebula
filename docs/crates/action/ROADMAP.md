# Roadmap

## Phase 1: Contract freeze and cleanup

- remove stale terminology in docs and examples
- lock current stable surface (`Action`, metadata, components, result/output/error/ports)
- publish compatibility policy for metadata versions

Exit criteria:
- no ambiguity between current API and aspirational design
- contract tests for serialization and compatibility pass

## Phase 2: Context and capability model

- replace temporary `NodeContext` bridge with stable context interfaces
- define capability access model (resources, credentials, cancellation, telemetry)
- provide mock/testing contexts for action unit tests

Exit criteria:
- engine/sandbox/runtime can all implement the same context contract
- capability checks map to deterministic action errors

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
- deliver ergonomic authoring layer in optional sibling package

Exit criteria:
- external action authors can build n8n-style nodes with predictable behavior
- runtime and sandbox integrations are documented end-to-end
