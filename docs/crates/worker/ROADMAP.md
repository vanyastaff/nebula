# Roadmap

## Phase 1: Contract and Safety Baseline

- deliverables:
  - worker config/state model
  - queue lease contract (claim/heartbeat/ack/nack)
  - basic sandbox integration and timeout/cancel flow
  - core metrics/logging/tracing skeleton
- risks:
  - contract mismatch with runtime/queue
  - missing idempotency in finalization flow
- exit criteria:
  - contract tests green for lease lifecycle and finalization idempotency
  - drain behavior validated in integration tests

## Phase 2: Runtime Hardening

- deliverables:
  - robust retry/backoff with resilience policies
  - structured failure taxonomy and dead-letter strategy
  - health/readiness + graceful rolling restart behavior
- risks:
  - retry storms during partial outage
  - false-positive unhealthy signals
- exit criteria:
  - chaos tests pass for queue/runtime transient failures
  - no task loss in restart simulations

## Phase 3: Scale and Performance

- deliverables:
  - adaptive concurrency and queue backpressure
  - autoscaling signals (saturation, lease lag, completion latency)
  - hot-path optimization for execution overhead
- risks:
  - resource starvation in multi-tenant load spikes
  - noisy autoscaling causing thrashing
- exit criteria:
  - target throughput and p95 latency met under stress profile
  - stable scaling behavior in load tests

## Phase 4: Ecosystem and DX

- deliverables:
  - clear worker operator handbook and runbooks
  - plugin/action compatibility matrix
  - richer telemetry dashboards + SLO alerts
- risks:
  - docs drift from implementation
  - ecosystem integrations introducing weak contracts
- exit criteria:
  - on-call runbook drill completed
  - contract versioning and migration policy exercised once

## Metrics of Readiness

- correctness:
  - zero lost tasks under failure-injection suite
- latency:
  - p95 task start latency and completion latency within SLO
- throughput:
  - sustained target tasks/sec per cluster profile
- stability:
  - no crash-loop under malformed task payloads and transient dependency failures
- operability:
  - full dashboard + alert coverage for queue lag, saturation, failure classes
