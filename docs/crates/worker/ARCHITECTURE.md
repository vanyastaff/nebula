# Architecture

## Mission

`nebula-worker` executes runtime tasks safely and predictably across a worker fleet, with explicit ownership of execution lifecycle, concurrency control, and result delivery.

## Core Model

- `WorkerId`: stable worker identity.
- `TaskLease`: claim token with TTL/heartbeat contract.
- `ExecutionAttempt`: single run of a task with attempt number.
- `WorkerState`: `starting | ready | busy | draining | stopped`.
- `TaskResultEnvelope`: result/error + execution metadata + observability context.

## High-Level Components

- `WorkerSupervisor`
  - bootstraps runtime, starts loops, handles shutdown/drain.
- `TaskReceiver`
  - claims tasks from `queue`, renews lease heartbeat, handles ack/nack.
- `ExecutionController`
  - enforces concurrency limits, timeout budget, cancellation.
- `SandboxAdapter`
  - provisions sandbox from `sandbox` crate and applies security/resource policy.
- `ResourceGovernor`
  - integrates `resource` quotas and admission checks.
- `ResultReporter`
  - commits final status to `runtime/engine` and emits events/metrics.
- `WorkerTelemetry`
  - structured logs, trace spans, counters/histograms.

## Execution Flow

1. Worker starts and registers liveness/readiness.
2. `TaskReceiver` claims task lease from queue.
3. `ExecutionController` performs admission checks (capacity, quotas, policy).
4. `SandboxAdapter` starts isolated environment.
5. Action/node execution runs with timeout + cancellation token.
6. Result is committed via `ResultReporter` (idempotent finalization).
7. Lease is acked; resources and sandbox are released.

## Concurrency and Backpressure

- Multi-level limits:
  - global worker concurrency
  - per-tenant concurrency
  - per-action type concurrency
- Backpressure behavior:
  - stop claiming when local queue exceeds watermark
  - renew existing leases first
  - expose saturation metric for autoscaler

## Comparative Architecture (Adopt/Reject/Defer)

- n8n worker queue model: `Adopt`
  - Reason: proven pull-based execution and horizontal scaling simplicity.
- Node-RED single-runtime execution style: `Reject`
  - Reason: weak isolation/multi-tenant boundaries for Nebula target.
- Activepieces worker separation: `Adopt`
  - Reason: clear split between control plane and execution plane.
- Temporal task queue + sticky behavior: `Defer`
  - Reason: useful for locality optimization, but complexity can wait for v2.

## Non-Goals (v1)

- speculative execution
- cross-region active-active execution leases
- hard real-time scheduling guarantees
