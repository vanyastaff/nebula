# Roadmap

`nebula-validator` roadmap focuses on correctness, ergonomics, and high-throughput behavior.

## Phase 1: Contract Clarity

- align all docs/examples with current trait names and extension methods
- add explicit compatibility guidelines for error codes used by API layer
- document canonical validator composition patterns for workflow/plugin configs

## Phase 2: Performance and Allocation Profiling

- benchmark hot paths:
  - common string validators
  - combinator chains
  - nested error construction
- add allocation-focused benchmarks for `ValidationError` heavy paths
- define performance budgets and regression thresholds in CI

## Phase 3: Schema-aware Validation Layer

- improve JSON/object path validation ergonomics (`json_field`, nested paths)
- add stronger typed bridges for `serde_json::Value` validation flows
- standardize field-path formatting across nested errors

## Phase 4: Safety and Reliability

- stress-test deep nested combinators and large error trees
- formalize limits for recursion depth / nested error explosion
- add guidance for fail-fast vs collect-all strategies per use case

## Phase 5: Toolchain and API Stability

- current workspace baseline: Rust `1.93`
- prepare controlled migration to Rust `1.93+`
- stabilize public API for long-lived validator definitions in plugins
