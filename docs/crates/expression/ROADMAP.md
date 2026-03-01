# Roadmap

## Phase 1: Contract and Safety Baseline

- deliverables:
  - formalize stable public API and hidden-internal boundaries.
  - lock evaluator safety guard behavior and docs.
  - align function naming/behavior documentation with implementation.
- risks:
  - mismatch between docs and real function semantics.
- exit criteria:
  - stable contract docs validated by integration tests.

## Phase 2: Runtime Hardening

- deliverables:
  - improve error context consistency and diagnostic quality.
  - strengthen integration tests with runtime/action/parameter crates.
  - harden context resolution edge cases.
- risks:
  - behavior drift in implicit coercion paths.
- exit criteria:
  - deterministic failures and no unresolved contract ambiguities.

## Phase 3: Scale and Performance

- deliverables:
  - benchmark-driven cache tuning guidance.
  - optimize hot evaluator paths and template rendering overhead.
  - expose lightweight cache observability metrics.
- risks:
  - optimization changes accidentally altering semantics.
- exit criteria:
  - measurable throughput/latency gains with semantic parity.

Current progress:
- lightweight cache observability now available via `ExpressionEngine::cache_overview()`.

## Phase 4: Ecosystem and DX

- deliverables:
  - strict/compatibility evaluation modes (if accepted).
  - richer migration tooling for function/grammar evolution.
  - additional production examples for common workflow patterns.
- risks:
  - fragmentation between compatibility modes.
- exit criteria:
  - clear operator guidance and low-friction adoption path.

Current progress:
- strict mode foundation exists via `EvaluationPolicy::with_strict_mode(true)`.
- first strict checks are enforced for boolean-only control/logical expressions.

## Metrics of Readiness

- correctness:
  - critical evaluator and template invariants fully tested.
- latency:
  - stable p95/p99 evaluation latency under representative load.
- throughput:
  - scalable expression throughput with cache-enabled profiles.
- stability:
  - low flake rate for parser/evaluator integration tests.
- operability:
  - actionable error and cache diagnostics.
