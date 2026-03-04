# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `ports`: owns sandbox port abstraction (`SandboxRunner`).
- nebula-runtime: InProcessSandbox (concrete in-process backend).
- `runtime`/`engine`/`execution`: choose sandbox backend and invoke execution.
- `action`: action metadata/result/error contracts consumed by sandbox.
- `resource`/`credential`: capability-sensitive dependencies accessed by actions.
- `resilience`: retry/circuit/backpressure policy around execution failures.
- `log`/`telemetry`: sandbox traces, violations, and operational metrics.

## Planned crates

- (future) sandbox-wasm:
  - why it will exist: stronger isolation for untrusted/community actions.
  - expected owner/boundary: implements `SandboxRunner` while preserving runtime contract.
- (future) sandbox-process (optional):
  - why it will exist: OS-level isolation fallback for non-WASM scenarios.
  - expected owner/boundary: process boundary and resource-limit enforcement.

## Downstream Consumers

- `runtime/engine`:
  - expectations from this crate: deterministic execution boundary and consistent error semantics.
- `security/ops tooling`:
  - expectations from this crate: auditable sandbox decisions and violation events.

## Upstream Dependencies

- `action`:
  - why needed: metadata, input/output types, and action errors.
  - hard contract relied on: stable action execution schema.
  - fallback behavior if unavailable: no execution path.
- `ports`:
  - why needed: decoupled contract for backend implementations.
  - hard contract relied on: stable `SandboxRunner` trait.
  - fallback behavior if unavailable: none.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| sandbox <-> runtime/engine | in/out | `SandboxRunner::execute` | async | runtime maps error to retry/fail-fast policy | primary execution path |
| sandbox <-> action | in | metadata/input/result/error schema | async | propagate `ActionError` with context | execution contract |
| sandbox <-> resource/credential | out (indirect) | capability-gated access intent | async | deny violations | capability boundary |
| sandbox <-> resilience | in/out | retry/circuit behavior integration | async | no hidden retries in backend | policy externalized |
| sandbox <-> telemetry/log | out | trace/audit/violation signals | async | non-blocking observability | ops visibility |

## Runtime Sequence

1. Runtime selects sandbox backend from policy/action metadata.
2. Runtime creates `SandboxedContext` and invokes `SandboxRunner`.
3. Backend checks preconditions (cancellation/policy/capabilities).
4. Backend executes action and returns structured result/error.
5. Runtime applies resilience policy and reports telemetry.

## Cross-Crate Ownership

- who owns domain model: sandbox execution boundary contracts (`ports` + sandbox docs).
- who owns orchestration: runtime/engine.
- who owns persistence: not sandbox.
- who owns retries/backpressure: resilience + runtime.
- who owns security checks: sandbox capability/policy checks; authn/authz in upper layers.

## Failure Propagation

- how failures bubble up:
  - sandbox backend returns `ActionError`/violation errors to runtime.
- where retries are applied:
  - runtime/resilience for retryable transient failures.
- where retries are forbidden:
  - explicit policy violations, unsupported capability, deterministic fatal action errors.

## Versioning and Compatibility

- compatibility promise with each dependent crate:
  - stable sandbox port contract within major versions.
- breaking-change protocol:
  - proposal -> decision -> migration docs -> major release.
- deprecation window:
  - one minor release minimum for non-critical API transitions.

## Contract Tests Needed

- runtime-to-sandbox contract tests for success/failure/cancellation.
- capability violation tests (once capability gating is implemented).
- backend parity tests across inprocess vs wasm/process drivers.
- resilience interoperability tests for error classification.
