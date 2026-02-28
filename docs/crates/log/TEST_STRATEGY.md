# Test Strategy

## Test Pyramid

- **Unit:** Config presets, error types, timing helpers, hook registry (with serialization)
- **Integration:** Init with each preset; file writer; observability hooks; context propagation
- **Contract:** Config schema snapshot tests; init API contract
- **End-to-end:** Examples as smoke tests; multi-crate observability example

## Critical Invariants

- Hook panic does not abort event emission
- Context propagates across `.await` in async mode
- Init with valid config always succeeds
- Config round-trip (serialize/deserialize) preserves semantics

## Scenario Matrix

- **Happy path:** `auto_init` → log → guard drop
- **Retry path:** N/A (init is one-shot)
- **Cancellation path:** `shutdown_hooks` during active emission
- **Timeout path:** N/A
- **Upgrade/migration path:** Config schema changes; see MIGRATION.md

## Tooling

- **Property testing:** proptest for config (optional)
- **Fuzzing:** Config deserialization (optional)
- **Benchmarks:** criterion for event emission, context propagation, timing macros
- **CI quality gates:** `cargo test --workspace`; `cargo clippy`; format check

## Exit Criteria

- **Coverage goals:** Critical paths covered; init, hooks, context
- **Flaky test budget:** Zero; use `OnceLock` and test init guard
- **Performance regression thresholds:** Criterion benchmarks with CI baseline
