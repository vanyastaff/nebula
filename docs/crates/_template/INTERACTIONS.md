# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `core`:
- `action`:
- `workflow`:
- `engine`:
- `runtime`:
- `sandbox`:
- `resource`:
- `credential`:
- `parameter`:
- `validator`:
- `resilience`:
- `storage`:
- `queue`/`eventbus`:
- `metrics`/`telemetry`/`log`:
- `worker`:
- `registry`/`plugin`/`sdk`:
- `api`/`cli`/`ui`:

## Planned crates

- crate:
  - why it will exist:
  - expected owner/boundary:

## Downstream Consumers

- crate/service:
  - expectations from this crate:

## Upstream Dependencies

- crate:
  - why needed:
  - hard contract relied on:
  - fallback behavior if unavailable:

## Interaction Matrix

Use this table for explicit contracts.

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| example | in/out | trait/API/event schema | sync/async | retry/fail-fast/degrade | |

## Runtime Sequence

1. step
2. step
3. step

## Cross-Crate Ownership

- who owns domain model:
- who owns orchestration:
- who owns persistence:
- who owns retries/backpressure:
- who owns security checks:

## Failure Propagation

- how failures bubble up:
- where retries are applied:
- where retries are forbidden:

## Versioning and Compatibility

- compatibility promise with each dependent crate:
- breaking-change protocol:
- deprecation window:

## Contract Tests Needed

- test:
- test:
