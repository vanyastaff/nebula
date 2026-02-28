# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `core`: shared IDs, execution metadata primitives.
- `runtime` / `engine`: task orchestration and execution state ownership.
- `queue` / `eventbus`: task delivery and redelivery contracts.
- `action`: executable node/action interface.
- `sandbox`: process/container isolation for untrusted execution.
- `resource`: quota policy and resource admission limits.
- `resilience`: retry/circuit/bulkhead policy definitions.
- `tenant`: tenant isolation boundaries and per-tenant limits.
- `config`: worker, queue, lease, timeout, and policy config.
- `log` / `metrics` / `telemetry`: observability export and diagnostics.
- `storage`: result payload persistence and artifact references.
- `credential`: secure secret retrieval during execution.

## Planned crates

- `worker` (this crate):
  - why it will exist: dedicated execution plane for reliable horizontal scale.
  - expected owner/boundary: task lease lifecycle, execution concurrency, result delivery.

## Downstream Consumers

- `runtime/engine`:
  - expectations from this crate: deterministic completion/failure signaling.
- `api/cli`:
  - expectations from this crate: accurate status/health and stable progress semantics.
- operations/SRE:
  - expectations from this crate: autoscaling signals, clear drain/failure behavior.

## Upstream Dependencies

- `queue`:
  - why needed: task claim and acknowledgment.
  - hard contract relied on: lease TTL + redelivery semantics.
  - fallback behavior if unavailable: pause claims, keep heartbeating active leases.
- `sandbox`:
  - why needed: execution isolation.
  - hard contract relied on: enforced resource and syscall/network policy.
  - fallback behavior if unavailable: fail-closed for protected workloads.
- `resource`:
  - why needed: quota/limit checks.
  - hard contract relied on: admission decision and quota accounting.
  - fallback behavior if unavailable: conservative deny for risky classes, allow safe defaults only.
- `runtime/engine`:
  - why needed: execution state ownership.
  - hard contract relied on: idempotent finalization endpoint.
  - fallback behavior if unavailable: local retry queue with bounded retention.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| worker <-> queue | in/out | claim/heartbeat/ack/nack lease API | async | retry + redelivery aware | core control loop |
| worker <-> runtime/engine | out | result finalization contract | async | idempotent retry, dead-letter on exhaustion | source of truth update |
| worker <-> sandbox | out | sandbox lifecycle + policy contract | async | fail-closed on policy mismatch | isolation |
| worker <-> resource | out | admission/quota contract | sync/async | reject/park task when over quota | capacity control |
| worker <-> action | out | action execution trait | async | classify retryable/non-retryable | business execution |
| worker <-> resilience | in | retry/backoff/circuit policies | sync | policy-driven retries only | reliability governance |
| worker <-> log/telemetry | out | logs/metrics/traces | async | non-blocking emit | observability |
| worker <-> credential | out | scoped credential fetch contract | async | fail with redacted diagnostics | secret access |

## Runtime Sequence

1. Worker claims lease from queue.
2. Admission checks with resource/tenant policy.
3. Sandbox starts and action executes.
4. Heartbeat loop renews lease while running.
5. Result is finalized in runtime/engine.
6. Queue ack/nack and cleanup follow.

## Cross-Crate Ownership

- who owns domain model: `core` + `runtime` own execution domain entities.
- who owns orchestration: `runtime/engine`.
- who owns persistence: `runtime/storage`.
- who owns retries/backpressure: `worker` executes; `resilience` defines policy.
- who owns security checks: `sandbox`, `credential`, and policy layers.

## Failure Propagation

- how failures bubble up:
  - execution failures become structured task failure outcomes with reason class.
- where retries are applied:
  - transient queue/runtime/storage/sandbox startup failures (policy-bound).
- where retries are forbidden:
  - invalid config, deterministic validation failures, policy violations.

## Versioning and Compatibility

- compatibility promise with each dependent crate:
  - stable lease and result semantics inside major version.
- breaking-change protocol:
  - proposal -> decision -> contract tests -> migration doc -> major release.
- deprecation window:
  - one minor release minimum for deprecated worker APIs/contracts.

## Contract Tests Needed

- queue lease TTL and heartbeat renewal tests.
- ack/nack idempotency tests under duplicates.
- runtime finalization idempotency tests.
- resource admission over-limit behavior tests.
- drain/shutdown contract tests with in-flight tasks.
