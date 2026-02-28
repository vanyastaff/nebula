# Reliability

## Reliability Objectives

- no task loss under transient dependency failures
- bounded duplicate execution consistent with at-least-once semantics
- graceful degradation under overload

## Failure Classes

- dependency transient (`queue`, `runtime`, `storage`, `sandbox startup`)
- dependency persistent outage
- local resource exhaustion
- malformed/non-retryable task payload

## Reliability Patterns

- lease heartbeat with redelivery on timeout
- idempotent finalization in runtime
- policy-driven retries with backoff/jitter
- dead-letter routing for exhausted attempts
- graceful drain for rolling restarts

## SLO Candidates

- task loss rate: `0` in validated chaos scenarios
- worker availability: `>= 99.9%`
- lease renewal success: `>= 99.99%`
- recovery time from transient queue outage: `< 5 minutes`

## Runbook Triggers

- queue lag beyond threshold
- repeated heartbeat renewal failures
- elevated sandbox startup failures
- spike in dead-lettered tasks per tenant/action
