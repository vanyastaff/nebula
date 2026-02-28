# Reliability

## SLO Targets

- availability:
  - validator functions are in-process and expected to be always available with the host process.
- latency:
  - strict budget for synchronous request-path validation in API/runtime boundaries.
- error budget:
  - semantic regressions in validation behavior have near-zero tolerance.

## Failure Modes

- dependency outage:
  - not network-dependent; failure mainly from internal bugs or panic conditions.
- timeout/backpressure:
  - heavy regex or deep nested validation can increase latency.
- partial degradation:
  - optional memoization/cache may degrade performance if disabled.
- data corruption:
  - incorrect validation acceptance/rejection is the primary reliability risk.
- oversized diagnostics:
  - deeply nested invalid payloads can create large nested error trees.

## Resilience Strategies

- retry policy:
  - validation failures are deterministic; no internal retry.
- circuit breaking:
  - not applicable inside crate; caller may short-circuit repeated invalid sources.
- fallback behavior:
  - callers may fall back to simplified validation profile under pressure.
- graceful degradation:
  - switch to fail-fast mode to cap resource usage in overload conditions.
- bounded diagnostics:
  - cap nested error-tree size at caller policy boundary for large payloads.

## Operational Runbook

- alert conditions:
  - sudden increase in validation latency or validation failure rates by endpoint.
- dashboards:
  - per-validator timing, failure code distribution, payload size vs latency.
- incident triage steps:
  1. identify impacted validators and input patterns
  2. isolate regression vs attack traffic pattern
3. apply temporary policy caps/fail-fast
4. ship targeted fix and regression tests

Error-tree handling notes:

- use collect-all only where downstream consumers need full diagnostics.
- use fail-fast profile on high-traffic boundaries under stress.

## Capacity Planning

- load profile assumptions:
  - high-frequency boundary validations on API and workflow compilation paths.
- scaling constraints:
  - CPU-bound operations; scale via process parallelism and bounded payload policy.
