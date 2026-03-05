# Migration

## Versioning Policy

- **Compatibility promise:** semantic versioning; minor versions additive; patch for bug fixes.
- **Deprecation window:** 2 minor versions before removal; deprecation notice in changelog and doc.

## Breaking Changes

### 2026-03 Contract Updates (Rust 1.93 wave)

- **Bulkhead queue is now enforced at runtime**
  - **Old behavior:** `queue_size` was declarative only; waiting was effectively unbounded.
  - **New behavior:** `acquire()` rejects when wait queue reaches `queue_size` (`BulkheadFull`).
  - **Migration steps:** increase `queue_size` for high fan-out workloads, or handle `BulkheadFull` explicitly.

- **Bulkhead acquire timeout now enforced from config**
  - **Old behavior:** `BulkheadConfig.timeout` did not affect `acquire()`.
  - **New behavior:** waiting for permit respects `timeout`; expiration returns `Timeout`.
  - **Migration steps:** tune timeout per service SLO and treat `Timeout` as expected backpressure signal.

- **Manager now applies `circuit_breaker` config during registration**
  - **Old behavior:** manager always built default circuit breaker when config existed.
  - **New behavior:** manager uses provided config; invalid config skips breaker registration for that service.
  - **Migration steps:** validate policy configs during startup; monitor logs for skipped registrations.

- **Retry jitter flag now has effect**
  - **Old behavior:** `use_jitter` field was ignored in delay calculation.
  - **New behavior:** when enabled, delay is randomized in range `0..=computed_backoff`.
  - **Migration steps:** update deterministic tests to disable jitter (`use_jitter = false`) where exact delays are asserted.

- **Policy schema change:** if `ResiliencePolicy` or `RetryPolicyConfig` fields change incompatibly:
  - **Old behavior:** deserialization fails or ignores unknown fields.
  - **New behavior:** new schema with migration path.
  - **Migration steps:** provide migration script or config transformer; document in CHANGELOG.

- **Pattern order contract (P-001):** if canonical order is enforced:
  - **Old behavior:** caller-defined order; possible inconsistency.
  - **New behavior:** fixed order (e.g., timeout → bulkhead → circuit → retry).
  - **Migration steps:** audit existing compositions; adjust if order differs; document in PROPOSALS.

- **Cancellation semantics (P-005):** if unified:
  - **Old behavior:** subtle differences between patterns.
  - **New behavior:** consistent propagation.
  - **Migration steps:** test cancellation paths; fix edge cases; document guarantees.

## Rollout Plan

1. **Preparation:** update policy schema/order/cancellation; add compatibility layer if needed.
2. **Dual-run / feature-flag stage:** optional; run new behavior behind flag for validation.
3. **Cutover:** release with migration doc; consumers update config/code.
4. **Cleanup:** remove deprecated APIs after deprecation window.

## Rollback Plan

- **Trigger conditions:** critical regression; policy load failures; performance degradation.
- **Rollback steps:** revert to previous crate version; restore previous config if schema changed.
- **Data/state reconciliation:** resilience is stateless per execution; no persistent state to reconcile. Policy config in external store may need manual rollback.

## Validation Checklist

- **API compatibility checks:** `cargo check --workspace`; dependent crates build.
- **Integration checks:** `cargo test --workspace`; engine/runtime integration tests pass.
- **Performance checks:** `cargo bench -p nebula-resilience`; no regression beyond threshold.
