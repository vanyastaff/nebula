# Roadmap

`nebula-resilience` roadmap is focused on correctness under load and operational clarity.

## Phase 1: API Contract Consolidation

- reconcile typed/untyped manager APIs and document preferred adoption path
- align examples with current production guidance
- clarify stability boundaries for advanced type-system APIs

## Phase 2: Performance and Scalability

- benchmark manager hot paths with high service cardinality
- optimize circuit/rate limiter contention scenarios
- profile layer composition overhead in deep chains

## Phase 3: Policy and Config Hardening

- strengthen policy validation for conflicting combinations
- add policy migration/versioning strategy
- tighten dynamic config behavior and reload semantics

## Phase 4: Reliability and Safety

- expand fault-injection tests for retry+breaker+timeout interplay
- validate observability behavior in failure storms
- formalize fail-open/fail-closed defaults per pattern

## Phase 5: Toolchain and Compatibility

- workspace baseline today: Rust `1.93`
- prepare controlled migration to Rust `1.93+`
- define compatibility guarantees for policy serialization and metrics schema
