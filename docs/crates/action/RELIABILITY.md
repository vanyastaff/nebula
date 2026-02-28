# Reliability

## SLO Targets

- availability:
  - action contract APIs are in-process and expected to be always available with host process.
- latency:
  - contract parsing/serialization overhead must remain negligible vs action execution time.
- error budget:
  - protocol regressions in `ActionResult`/`ActionOutput` semantics have near-zero tolerance.

## Failure Modes

- dependency outage:
  - runtime/resource/credential outages expressed through action error/result signals.
- timeout/backpressure:
  - deferred/streaming flows can stall without robust runtime policies.
- partial degradation:
  - reduced capability environment (sandbox restrictions) may block optional action behavior.
- protocol drift:
  - mismatched assumptions between action and engine/runtime versions.

## Resilience Strategies

- retry policy:
  - action crate signals retryability, resilience layer owns retry execution policy.
- circuit breaking:
  - handled outside this crate (resilience/runtime).
- fallback behavior:
  - action may emit `Skip`/`Route`/`Retry` as controlled degradation intent.
- graceful degradation:
  - prefer explicit flow-control variants over implicit failures.

## Operational Runbook

- alert conditions:
  - spike in `Retryable`, `SandboxViolation`, `DataLimitExceeded`.
- dashboards:
  - result variant distribution, error type rates, deferred resolution timings.
- incident triage steps:
  1. identify failing action type/version
  2. correlate with runtime/sandbox capability changes
  3. inspect output/deferred contract mismatches
  4. apply rollback or migration mapping

## Capacity Planning

- load profile assumptions:
  - high-frequency execution of stateless actions and variable deferred/streaming workloads.
- scaling constraints:
  - protocol crate is lightweight; bottlenecks are runtime/sandbox/IO layers.
