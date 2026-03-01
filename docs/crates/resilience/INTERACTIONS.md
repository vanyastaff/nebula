# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `core`: ids, interface versioning; resilience uses for typed identifiers where applicable.
- `action`: retryability hints via `ActionError::Retryable`; resilience applies policy, not action.
- `workflow`: graph compilation; resilience not directly involved.
- `engine` / `runtime`: orchestration; wraps action execution with resilience patterns.
- `sandbox`: capability enforcement; resilience runs outside sandbox boundary.
- `resource`: health checks; callers may use resilience for timeout/retry around resource ops.
- `credential`: credential resolution; callers may wrap with resilience.
- `config`: hot-reload; resilience policy can be loaded from config sources.
- `log`: logging; resilience observability hooks integrate with nebula-log.
- `resilience`: this crate — fault-tolerance primitives and orchestration.
- `storage`: key-value backends; callers may wrap with resilience.
- `eventbus` / `queue`: messaging; callers may rate-limit or circuit-break.
- `telemetry` / `metrics`: observability; resilience hooks emit events/metrics.
- `plugin` / `sdk` / `registry`: plugin authoring; may consume resilience for external calls.
- `api` / `cli` / `ui`: control plane; may use resilience for backend calls.

## Planned crates

- `worker`: execution workers; will consume resilience for HTTP/DB/queue calls.
- `metrics` (if split from telemetry): resilience metrics schema integration.

## Downstream Consumers

- `engine` / `runtime`: wrap action execution with retry/circuit-breaker/timeout.
- `resource`: callers use resilience for health checks and acquire timeouts.
- `config`: loads `ResiliencePolicy` from config; resilience validates and applies.
- `api` / service adapters: wrap external HTTP/DB/queue calls with resilience.

## Upstream Dependencies

- `nebula-core`: optional; typed identifiers if used.
- `nebula-config`: `ConfigSource` for policy loading; fallback to defaults if unavailable.
- `nebula-log`: `debug`, `info`, `warn`, `error`; resilience degrades if unavailable (no hard contract).
- `tokio`: async runtime; hard contract for timeouts and cancellation.
- `serde` / `serde_json`: policy serialization; hard contract for `ResiliencePolicy`.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| resilience <-> engine/runtime | out | `ResilienceManager`, `execute`, policy override | async | retry/circuit/fail-fast per policy | core integration |
| resilience <-> action | in | `Retryable` trait (`is_retryable()`, `retry_delay()`, `max_retries()`), `ActionError::Retryable` | sync (trait), async (execution) | resilience decides retries from signals | action provides hints; resilience owns policy |
| resilience <-> config | in | `ConfigSource`, policy deserialization | sync load, async reload | invalid config -> `ConfigError` | optional integration |
| resilience <-> log | out | `nebula_log::{debug,info,warn,error}` | sync | best-effort; no hard dependency | observability |
| resilience <-> resource | out | caller wraps resource ops | async | timeout/retry at caller | no direct dependency |
| resilience <-> credential | out | caller wraps credential ops | async | same as resource | no direct dependency |

## Runtime Sequence

1. Engine/runtime registers services with `ResilienceManager` and policies.
2. Action execution is wrapped: timeout → bulkhead → circuit breaker → retry (canonical order TBD).
3. Action returns `ActionResult` or `ActionError`; resilience checks `is_retryable()`.
4. Retry/circuit/rate-limit decisions are applied; observability hooks fire.
5. Final result propagates to engine for flow control.

## Cross-Crate Ownership

- **resilience:** owns retry/backoff/circuit/rate-limit policy and execution.
- **engine/runtime:** owns orchestration, lifecycle, persistence, scheduling.
- **action:** owns contract semantics; provides retryability signals.
- **config:** owns policy loading; resilience validates and applies.

## Failure Propagation

- **Retryable errors:** `ResilienceError::Timeout`, `RateLimitExceeded`, `CircuitBreakerOpen` (when retry_after set); `Custom { retryable: true }`.
- **Terminal errors:** `RetryLimitExceeded`, `FallbackFailed`, `Cancelled`, `InvalidConfig`; `Custom { retryable: false }`.
- **Retries applied:** inside resilience layer; action does not retry itself.
- **Retries forbidden:** after `RetryLimitExceeded`; when circuit is open (fail-fast).

## Versioning and Compatibility

- **Policy serialization:** `ResiliencePolicy`, `RetryPolicyConfig` — additive changes only; breaking changes require major version.
- **Breaking-change protocol:** major version bump; migration doc; compatibility tests with engine/config.
- **Deprecation window:** 2 minor versions before removal.

## Contract Tests Needed

- Policy serialization round-trip with engine/config.
- Retryability signal mapping from `ActionError` to resilience decisions.
- Pattern composition order and cancellation propagation.
