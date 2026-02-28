# Roadmap

## Phase 1: Contract and Safety Baseline

- deliverables:
  - finalize SPEC-aligned docs and cross-crate memory contracts.
  - define stable API subset and explicit unstable surface.
  - document safety invariants for unsafe-heavy internals.
- risks:
  - hidden assumptions in consumer crates.
- exit criteria:
  - docs map to real code and are review-ready for runtime/action teams.

## Phase 2: Runtime Hardening

- deliverables:
  - expand concurrency/stress tests for shared pools/caches.
  - validate pressure handling and budget enforcement under spikes.
  - tighten error-path behavior consistency.
- risks:
  - contention regressions and flaky concurrency tests.
- exit criteria:
  - deterministic behavior in stress scenarios and no critical flaky tests.

## Phase 3: Scale and Performance

- deliverables:
  - benchmark-guided tuning across allocator/pool/cache modes.
  - published sizing profiles for common workflow load classes.
  - optimize feature-on overhead for monitoring/stats paths.
- risks:
  - over-optimization for synthetic benches.
- exit criteria:
  - measurable p95/p99 improvement on representative scenarios.

## Phase 4: Ecosystem and DX

- deliverables:
  - reference integration patterns for runtime and action crates.
  - optional unified config bootstrap if proposal accepted.
  - improve migration tooling/checklists for feature and API evolution.
- risks:
  - breaking consumer assumptions during API cleanup.
- exit criteria:
  - at least one fully documented end-to-end integration flow.

## Metrics of Readiness

- correctness:
  - all critical invariants covered by tests.
- latency:
  - stable tail-latency behavior in benchmark suites.
- throughput:
  - no regressions in high-churn allocation scenarios.
- stability:
  - low flake rate in concurrency-heavy test sets.
- operability:
  - actionable metrics and pressure diagnostics in production.
