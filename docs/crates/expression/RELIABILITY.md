# Reliability

## SLO Targets

- availability:
  - expression evaluation path meets runtime dependency availability requirements.
- latency:
  - evaluation and template render latency remain within runtime action budget.
- error budget:
  - deterministic user/config errors are expected; internal engine failures are low-tolerance.

## Failure Modes

- dependency outage:
  - memory cache dependency unavailable or disabled.
- timeout/backpressure:
  - large/complex expression workloads causing CPU pressure.
- partial degradation:
  - cache disabled/missed leading to slower parse-heavy flows.
- data corruption:
  - malformed context/value structures causing unexpected type failures.

## Resilience Strategies

- retry policy:
  - caller-owned; retry only transient internal/integration failures.
- circuit breaking:
  - managed in runtime/resilience layer for hot failing expressions.
- fallback behavior:
  - fallback to no-cache evaluation path when cache unavailable.
- graceful degradation:
  - fail-fast on invalid expressions while preserving engine process health.

## Operational Runbook

- alert conditions:
  - spikes in parse/eval errors, regex safety rejections, latency regressions.
- dashboards:
  - evaluation throughput, p95/p99 latency, error-class distribution, cache utilization.
- incident triage steps:
  1. identify failing expression patterns and source workflows.
  2. classify deterministic vs transient errors.
  3. inspect recent function/grammar/config changes.
  4. rollback or patch behavior with migration guidance.

## Capacity Planning

- load profile assumptions:
  - frequent small expressions plus moderate template rendering bursts.
- scaling constraints:
  - CPU for parse/eval, memory for cache, and regex complexity bounds.
