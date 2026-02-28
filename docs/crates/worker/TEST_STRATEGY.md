# Test Strategy

## Test Layers

- unit tests:
  - worker state transitions
  - retry policy application and classification
  - timeout/cancellation behavior
- contract tests:
  - queue lease lifecycle (claim/heartbeat/ack/nack)
  - runtime finalization idempotency
  - sandbox policy application contract
- integration tests:
  - full worker loop with mocked queue/runtime/sandbox
  - graceful drain with in-flight tasks
  - resource admission and over-limit handling
- chaos/failure tests:
  - queue outage and recovery
  - sandbox start failures and retry strategy
  - runtime finalization timeout/retry scenarios
- performance tests:
  - throughput and latency under mixed workload
  - backpressure behavior at saturation

## Required Fixtures

- deterministic fake queue with lease TTL control
- fake runtime finalization endpoint with idempotency modes
- sandbox simulator for policy success/failure matrix
- per-tenant load generator

## CI Gates

- contract tests are mandatory for merges touching worker contracts.
- failure-injection suite runs on nightly pipeline.
- performance regression gate for p95 latency and throughput baseline.
