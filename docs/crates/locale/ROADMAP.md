# Roadmap

## Phase 1: Contract and Safety Baseline

- deliverables:
  - create `crates/locale` MVP contracts for negotiation and translation.
  - define key namespace and fallback chain specification.
  - integrate basic localized error rendering adapters.
- risks:
  - inconsistent legacy key usage across crates.
- exit criteria:
  - contract tests pass for API/runtime/action/validator consumers.

## Phase 2: Runtime Hardening

- deliverables:
  - robust catalog validation and startup checks.
  - missing-key telemetry and alerting hooks.
  - standardized locale context propagation.
- risks:
  - silent fallback masking content gaps.
- exit criteria:
  - deterministic fallback behavior and actionable observability.

## Phase 3: Scale and Performance

- deliverables:
  - translation bundle cache and lookup optimizations.
  - benchmark locale negotiation/render paths.
  - tune memory footprint for multi-locale deployments.
- risks:
  - cache staleness and memory growth.
- exit criteria:
  - stable latency and memory bounds under target load.

## Phase 4: Ecosystem and DX

- deliverables:
  - tooling for key linting and catalog completeness checks.
  - staged support for dynamic catalog reload.
  - contributor guidelines for localization workflows.
- risks:
  - operational complexity for dynamic updates.
- exit criteria:
  - safe and maintainable localization lifecycle in production.

## Metrics of Readiness

- correctness:
  - no unresolved key collisions or fallback ambiguity.
- latency:
  - locale negotiation/render within UX budget.
- throughput:
  - translation lookups scale with request volume.
- stability:
  - no critical regressions in localization flows.
- operability:
  - complete missing-key and fallback diagnostics.
