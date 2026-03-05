# Performance Budget

## Scope

Budgets apply to `nebula-resilience` Criterion benchmarks:

- `manager`
- `rate_limiter`
- `circuit_breaker`
- `bulkhead`
- `retry`
- `compose`

## Baseline Policy

- Use named Criterion baselines (for example `main`) created via:
  - `./scripts/bench-resilience.ps1 -Mode baseline -Baseline main`
  - `./scripts/bench-resilience.sh baseline main`
- Compare feature branch against baseline via `Mode compare`.

## Regression Gates

### Hard Gate (block merge)

- `manager/concurrent_access/concurrent_execute/100`: regression > **8%**
- `manager/execute_overhead/no_patterns`: regression > **5%**
- `rate_limiter/contention/concurrent_acquire/100`: regression > **8%**
- `rate_limiter/contention/governor_concurrent_acquire/100`: regression > **8%**
- `bulkhead/contention/concurrent_execute/100`: regression > **8%**
- `retry/successful_operation/fixed_delay/5`: regression > **10%**
- `retry/jitter_calculation/full_jitter`: regression > **12%**
- `timeout/overhead/wrapped_yield_once`: regression > **10%**
- `circuit_breaker/can_execute/closed`: regression > **6%**

### Soft Gate (needs review)

- Any other benchmark regression > **10%**
- Any benchmark with high variance and p-value not significant across 2 reruns
- `timeout/cancellation/*` regressions are reviewed as platform-sensitive signals (informational), not hard-gated across heterogeneous runners
- `retry/with_failures/*` regressions are reviewed with platform timer/scheduler context before accepting as hard failures

## Accepted Trade-offs

Regression may be accepted only if all are true:

1. Change improves correctness/reliability semantics.
2. Regression is documented in PR with measured before/after.
3. No hard-gate benchmark violates thresholds.

## CI/PR Procedure

1. Run compare mode against baseline.
2. Attach summary table in PR:
   - benchmark id
   - baseline mean
   - current mean
   - delta %
   - gate status
3. If soft-gate triggered, rerun benchmark at least once.

## Ownership

- `nebula-resilience` maintainers own threshold updates.
- Threshold updates require rationale in `MIGRATION.md` or `PERFORMANCE.md`.
