# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `core`: tenant identifiers and scope primitives.
- `api`/`webhook`: tenant context entry points from external requests.
- `runtime`/`engine`/`execution`: execution scheduling and quota-aware flow.
- `resource`: tenant-aware resource scope enforcement.
- `storage`: tenant partition boundaries for data persistence.
- `credential`: tenant-scoped secret access.
- `config`: tenant policy and quota configuration sources.
- `telemetry`/`log`: audit, metrics, and forensic traces.

## Planned crates

- `tenant` (this crate):
  - why it will exist: own authoritative tenant context + policy + quota contracts.
  - expected owner/boundary: governance layer between ingress and runtime/data/resource subsystems.

## Downstream Consumers

- `runtime/engine`:
  - expectations from this crate: deterministic quota decisions and context integrity.
- `storage/resource/credential`:
  - expectations from this crate: consistent tenant policy mapping and isolation signals.

## Upstream Dependencies

- `core`:
  - why needed: canonical tenant/scope identifiers.
  - hard contract relied on: stable identity and scope types.
  - fallback behavior if unavailable: none.
- `config`:
  - why needed: tenant policy/quota definitions.
  - hard contract relied on: validated config model.
  - fallback behavior if unavailable: safe deny/default policy.
- `storage` (policy state backend):
  - why needed: tenant metadata and usage accounting persistence.
  - hard contract relied on: consistent read/write semantics.
  - fallback behavior if unavailable: fail closed for critical operations.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| tenant <-> api/webhook | in | tenant identity extraction + validation | async | reject unknown/invalid tenant | ingress boundary |
| tenant <-> runtime/engine | out | quota and policy decisions | async | fail-closed on hard policy errors | orchestration control |
| tenant <-> resource | out | scope/limit mapping for resources | async | deny on mismatch | isolation boundary |
| tenant <-> storage | out/in | partition strategy and usage accounting | async | degraded mode with strict limits | persistence coupling |
| tenant <-> credential | out | tenant scope contract | async | deny cross-tenant access | security-critical |
| tenant <-> telemetry/log | out | audit events and usage metrics | async | non-blocking telemetry | audit trail |

## Runtime Sequence

1. Ingress resolves tenant identity and asks `tenant` for validated context.
2. Runtime requests quota/policy decision before scheduling execution.
3. Resource/storage/credential consumers enforce tenant-specific boundaries.
4. Usage is accounted and emitted to telemetry/audit streams.

## Cross-Crate Ownership

- who owns domain model:
  - `tenant` owns tenant policy/context model once implemented.
- who owns orchestration:
  - `runtime/engine`.
- who owns persistence:
  - `storage` (tenant data + usage state).
- who owns retries/backpressure:
  - `resilience` + runtime layer.
- who owns security checks:
  - tenant context validation in `tenant`; request authn in ingress/auth layers.

## Failure Propagation

- how failures bubble up:
  - policy/quota/identity failures propagate as explicit tenant errors.
- where retries are applied:
  - transient backend/lock contention paths.
- where retries are forbidden:
  - unknown tenant, disabled tenant, cross-tenant violation.

## Versioning and Compatibility

- compatibility promise with each dependent crate:
  - stable tenant context and decision contracts within major versions.
- breaking-change protocol:
  - proposal -> decision -> migration -> major release.
- deprecation window:
  - minimum one minor for non-critical API removals.

## Contract Tests Needed

- tenant identity extraction and canonicalization tests.
- cross-tenant denial tests for resource/storage/credential integrations.
- quota race/atomicity tests under concurrent execution load.
- policy fallback tests when config/storage backends are degraded.
