# Test Strategy

## Test Pyramid

- unit:
  - locale parsing, negotiation precedence, key lookup/interpolation.
- integration:
  - API/runtime/action/validator localization flows.
- contract:
  - stable key namespace and fallback behavior across crates.
- end-to-end:
  - multilingual user-facing responses in realistic workflow scenarios.

## Critical Invariants

- deterministic locale selection for identical inputs.
- deterministic fallback chain and safe default locale behavior.
- interpolation parameters validated before rendering.
- localization failures never erase canonical machine-readable errors.

## Scenario Matrix

- happy path:
  - supported locale with complete keys.
- retry path:
  - transient catalog backend failures.
- cancellation path:
  - canceled request still returns consistent canonical error context.
- timeout path:
  - slow catalog access handled with fallback policy.
- upgrade/migration path:
  - key namespace and catalog version transitions.
- plugin locale path:
  - plugin with valid `locales/` auto-loads bundles and renders namespaced keys.
- plugin invalid locale path:
  - malformed locale file/tag/key namespace is rejected with diagnostics and safe fallback.

## Tooling

- property testing:
  - negotiation and fallback invariants.
- fuzzing:
  - locale/tag parser and interpolation payloads.
- benchmarks:
  - render throughput and lookup latency per locale set size.
- CI quality gates:
  - key completeness checks and critical locale snapshot tests.
  - plugin bundle validation gate for `locales/` auto-discovery rules.

## Exit Criteria

- coverage goals:
  - full coverage on negotiation/fallback/interpolation critical paths.
- flaky test budget:
  - zero flaky tests for locale selection and fallback invariants.
- performance regression thresholds:
  - no significant regression in render latency under baseline loads.
