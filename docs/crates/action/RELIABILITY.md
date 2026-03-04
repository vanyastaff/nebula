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

## Error-to-Retry Mapping Conventions (ACT-T027)

Use these conventions to keep retry behavior deterministic across actions:

| Signal | Retry class | Runtime behavior |
|---|---|---|
| `ActionResult::Retry { after, reason }` | explicit retry | reschedule exactly after `after` |
| `ActionError::Retryable { backoff_hint: Some(..) }` | transient retryable | retry with policy, may use hint |
| `ActionError::Retryable { backoff_hint: None }` | transient retryable | retry with default policy |
| `ActionError::Fatal` | non-retryable | fail node/execution path |
| `ActionError::Validation` | non-retryable | fail-fast; indicates author/config issue |
| `ActionError::SandboxViolation` | non-retryable | fail-fast; capability policy breach |
| `ActionError::DataLimitExceeded` | non-retryable by default | fail-fast unless runtime has spill policy |
| `ActionError::Cancelled` | neutral | do not auto-retry unless caller policy explicitly allows |

Authoring rules:

1. Use `ActionResult::Retry` when the action *expects* a later successful re-run (upstream warmup, eventual consistency, rate-window delay).
2. Use `ActionError::Retryable` for transient failures where retry is optional and policy-owned by runtime/resilience.
3. Never encode retry intent inside `Fatal`/`Validation` messages.
4. Include actionable reason text; avoid generic `"failed"` without context.

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
