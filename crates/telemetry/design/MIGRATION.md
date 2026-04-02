# Migration

## Versioning Policy

- **Compatibility promise:** ExecutionEvent schema additive-only; TelemetryService trait backward compatible; metric types stable.
- **Deprecation window:** Minimum 2 minor releases before removal.

## Breaking Changes

None currently planned. Future changes will be documented here.

### Example (Hypothetical): Histogram Replacement

- **Old behavior:** `Histogram` stores all observations in memory.
- **New behavior:** `BucketedHistogram` with fixed buckets; bounded memory.
- **Migration steps:**
  1. Introduce `BucketedHistogram` and `MetricsRegistry::bucketed_histogram()`.
  2. Deprecate `Histogram` and `MetricsRegistry::histogram()`.
  3. Update consumers to use bucketed variant.
  4. Remove deprecated APIs in next major.

## Rollout Plan

1. **Preparation:** Document new API; add deprecation warnings.
2. **Dual-run / feature-flag stage:** New implementation behind feature flag if needed.
3. **Cutover:** Consumers migrate; remove old code.
4. **Cleanup:** Remove deprecated APIs.

## Rollback Plan

- **Trigger conditions:** Regression in emit/record; panic in hot path.
- **Rollback steps:** Revert to previous crate version; redeploy.
- **Data/state reconciliation:** N/A; telemetry is ephemeral (in-memory).

## Validation Checklist

- [ ] API compatibility: `cargo check` with dependent crates
- [ ] Integration: `cargo test --workspace`
- [ ] Performance: No regression in emit/record latency
