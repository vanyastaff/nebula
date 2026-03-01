# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `core`: shared types and platform-wide conventions.
- `config`: provides validated resource/pool settings.
- `credential`: secret material for resource initialization (`credentials` feature).
- `parameter`: runtime inputs that influence resource selection.
- `validator`: validates resource config contracts and policy constraints.
- `action`: primary runtime consumer; acquires resources for node execution.
- `runtime`/`engine`/`worker`: orchestration layers coordinating acquire/use/release lifecycles.
- `resilience`: retry/circuit-breaker/rate-limit policies around acquire/use operations.
- `log`/`metrics`/`telemetry`: observability sinks for resource lifecycle events.
- `plugin`/`registry`/`sdk`: define resource requirements for third-party integrations.

## Planned crates

- `resource-*` adapters (postgres, redis, s3, kafka, http):
  - why it will exist: keep core crate generic and transport-agnostic.
  - expected owner/boundary: adapter crate owns driver-specific config and validation.
- `resource-policy` (optional):
  - why it will exist: shared policy profiles for back-pressure and reload classes.
  - expected owner/boundary: separate policy composition from manager internals.

## Downstream Consumers

- `action`:
  - expects low-latency typed and dynamic acquire, strict scope isolation, clear retryability semantics.
- `runtime/engine/worker`:
  - expects deterministic shutdown and health propagation.
- driver crates:
  - expect stable trait contracts (`Resource`, `Config`).
- `nebula-api` *(Phase 2)*:
  - expects `Manager::list_status()` → `Vec<ResourceStatus>` for `GET /resources`.
  - expects `Manager::get_status(id)` → `Option<ResourceStatus>` for `GET /resources/:id`.
  - expects `Manager::event_bus().subscribe()` for live SSE stream at `GET /resources/events`.
  - expects `Manager::drain(id)` → `Result<()>` for admin `POST /resources/:id/drain`.
  - **read-only contract**: api layer never registers or deregisters resources; it only reads state and subscribes to events.

## Upstream Dependencies

- `config`:
  - why needed: validated pool/resource settings.
  - hard contract relied on: stable field semantics for timeouts and size limits.
  - fallback behavior if unavailable: fail startup for mandatory resources.
- `credential` (feature-gated):
  - why needed: secret acquisition/rotation.
  - hard contract relied on: secure lookup and redaction behavior.
  - fallback behavior if unavailable: resource registration fails for secret-dependent resources.
- `resilience` (integration-level):
  - why needed: circuit-breaker and retry policies external to pooling core.
  - hard contract relied on: error classification via `is_retryable`.
  - fallback behavior if unavailable: fail-fast or bounded-wait behavior only.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| resource <-> action | out | `ResourceProvider`, `acquire(_typed)` | async | action applies retry by policy | main runtime path |
| resource <-> runtime/engine | out | registration, shutdown, health propagation | async | startup fail-fast, runtime degrade | orchestration boundary |
| resource <-> credential | in | credential-backed resource config | async | fail registration on missing secret | feature `credentials` |
| resource <-> config | in | typed pool/resource config | sync/async | validation blocks activation | boot and reload path |
| resource <-> resilience | in/out | retry/circuit-breaker integration | async | avoid hidden retries inside pool | explicit ownership split |
| resource <-> telemetry/log | out | hooks/events/metrics stream | async | observability errors never block acquire | additive layer |
| resource <-> nebula-api | out | `list_status()`, `event_bus().subscribe()` | sync/async | api reads snapshot; errors return 503 | read-only; Phase 2 |

## Runtime Sequence

1. Runtime builds manager and registers resources (scoped where needed).
2. Action execution calls `acquire` through provider contract with `Context`.
3. Manager enforces quarantine, health, and scope constraints.
4. Pool returns idle or creates new instance under back-pressure limits.
5. Guard drop returns instance or triggers cleanup; events and hooks emitted.
6. Shutdown drains and cleans pools according to shutdown config.

## Cross-Crate Ownership

- who owns domain model: `resource` owns lifecycle and pooling model.
- who owns orchestration: `runtime/engine` own execution flow and call sequencing.
- who owns persistence: not `resource`; owned by storage-specific crates.
- who owns retries/backpressure: pool owns concurrency limit; retry policy belongs to `resilience` and caller.
- who owns security checks: scope/isolation in `resource`; authn/authz policy in upper layers.

## Failure Propagation

- how failures bubble up:
  - manager and pool return typed `Error` variants to callers.
- where retries are applied:
  - caller level (`action`/`runtime`) based on `is_retryable` and policy.
- where retries are forbidden:
  - config/validation/scope mismatch and non-retryable unavailable states.

## Versioning and Compatibility

- compatibility promise with each dependent crate:
  - stable traits and error meaning within major version.
- breaking-change protocol:
  - design note in `PROPOSALS.md` -> accepted decision in `DECISIONS.md` -> migration notes in `MIGRATION.md` -> major version bump.
- deprecation window:
  - at least one minor version for non-critical API migrations.

## Contract Tests Needed

- scope isolation contract tests across tenant/workflow/execution/action.
- action-to-resource integration tests for acquire/release under cancellation.
- runtime shutdown contract tests with in-flight operations.
- resilience interoperability tests for retryability mapping.
