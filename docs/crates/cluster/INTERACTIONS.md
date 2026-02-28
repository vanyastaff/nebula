# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `runtime` / `engine` / `execution`: submit work and consume placement/failover decisions.
- `storage`: persistence of execution and control-plane metadata.
- `telemetry` / `log`: cluster health, scheduling, and failover observability.
- `resilience`: retry/backoff/circuit controls around distributed operations.
- `api` / `cli`: operator control path for cluster operations.
- `core`: shared IDs/types/errors for cluster domain integration.

## Planned crates

- `cluster` (this crate):
  - why it will exist: single owner for distributed control-plane contracts.
  - expected owner/boundary: membership, scheduling, failover, and autoscaling governance.

## Downstream Consumers

- `runtime/engine`:
  - expectations from this crate: deterministic placement and failover behavior.
- `api/cli`:
  - expectations from this crate: safe cluster control operations and status views.

## Upstream Dependencies

- `storage`:
  - why needed: durable cluster metadata and execution ownership state.
  - hard contract relied on: consistent reads/writes for control-plane state.
  - fallback behavior if unavailable: fail-safe mode preventing unsafe scheduling.
- `telemetry/log`:
  - why needed: operational visibility and incident triage.
  - hard contract relied on: non-blocking observability pipelines.
  - fallback behavior if unavailable: continue control-plane operations with reduced visibility.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| cluster <-> runtime/engine | in/out | placement + failover APIs | async | retry transient, fail-safe on state uncertainty | critical path |
| cluster <-> execution/worker | out | assignment/ownership updates | async | idempotent reassignment | worker coordination |
| cluster <-> storage | in/out | durable control-plane state | async | degraded safe mode if unavailable | consistency boundary |
| cluster <-> api/cli | out/in | cluster ops and status commands | async | validate and reject unsafe operations | operator path |
| cluster <-> resilience | in/out | retry/backoff semantics for distributed ops | async | centralized policy ownership | reliability layer |
| cluster <-> telemetry/log | out | metrics/events/audit | async | non-blocking | observability |

## Runtime Sequence

1. Node joins cluster and membership state is updated.
2. Runtime requests workflow placement from scheduler.
3. Cluster assigns target node and tracks ownership.
4. On node failure, failover reschedules affected workloads.
5. Autoscale/rebalance adjusts cluster topology over time.

## Cross-Crate Ownership

- who owns domain model: `cluster` owns distributed control-plane model.
- who owns orchestration: runtime/engine own execution lifecycle orchestration.
- who owns persistence: storage crate(s).
- who owns retries/backpressure: resilience + caller policy.
- who owns security checks: cluster operation authorization at API/control layers.

## Failure Propagation

- how failures bubble up:
  - explicit cluster/placement errors returned to runtime/api.
- where retries are applied:
  - transient network or backend state failures.
- where retries are forbidden:
  - invalid operation requests and unsafe state transitions.

## Versioning and Compatibility

- compatibility promise with each dependent crate:
  - stable placement/failover contracts within major version.
- breaking-change protocol:
  - proposal -> decision -> migration guide -> major release.
- deprecation window:
  - one minor release minimum for non-critical transitions.

## Contract Tests Needed

- placement determinism and fairness tests.
- failover idempotency and recovery tests.
- membership consistency tests (join/leave/partition scenarios).
- operator command safety tests.
