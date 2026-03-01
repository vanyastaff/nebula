# Interactions

## Ecosystem Map (Current + Planned)

## Existing crates

- `execution` / `runtime` / `engine`: primary expression evaluation consumers.
- `action` / `parameter`: pass dynamic values/templates resolved through expression engine.
- `core`: shared fundamental types and platform contracts.
- `memory`: cache backend (`cache` feature) used by expression engine.
- `log` / `telemetry`: diagnostics and observability for evaluation failures/perf.

## Planned crates

- optional `expression-policy` layer:
  - why it will exist: centralized rules for function allowlist, strict modes, and cost budgets.
  - expected owner/boundary: policy composition on top of core engine.
  - current progress: engine-level MVP allowlist exists via `ExpressionEngine::restrict_to_functions(...)`.

## Downstream Consumers

- `execution/runtime`:
  - expectations from this crate: deterministic expression results and clear errors.
- `action/parameter`:
  - expectations from this crate: stable template and MaybeExpression semantics.

## Upstream Dependencies

- `memory`:
  - why needed: high-throughput cache primitives.
  - hard contract relied on: cache correctness and concurrency behavior.
  - fallback behavior if unavailable: run without cache.
- `core`:
  - why needed: ecosystem consistency for IDs/context semantics.
  - hard contract relied on: stable shared types.
  - fallback behavior if unavailable: none.

## Interaction Matrix

| This crate <-> Other crate | Direction | Contract | Sync/Async | Failure handling | Notes |
|---|---|---|---|---|---|
| expression <-> execution/runtime | out | evaluate/template APIs | sync | deterministic error propagation | critical path |
| expression <-> parameter/action | out | MaybeExpression/Template resolution | sync | fail-fast on invalid expressions | config path |
| expression <-> memory | in | cache backend for parse/template | sync | fallback to no-cache mode | perf optimization |
| expression <-> log/telemetry | out | parse/eval diagnostics | sync/async export | non-blocking telemetry | ops visibility |
| expression <-> core | in | shared ecosystem conventions | sync | none | foundation |

## Runtime Sequence

1. Caller builds `EvaluationContext` from runtime state.
2. Engine parses (or cache-hits) expression/template.
3. Evaluator resolves variables/functions and computes result.
4. Caller consumes `serde_json::Value` or handles `ExpressionError`.

## Cross-Crate Ownership

- who owns domain model: expression grammar, evaluator, and template semantics.
- who owns orchestration: runtime/execution layers.
- who owns persistence: none.
- who owns retries/backpressure: caller/resilience layer.
- who owns security checks: expression engine for parsing/evaluation safety; authn/authz outside.

## Failure Propagation

- how failures bubble up:
  - typed `ExpressionError` returned to caller.
- where retries are applied:
  - transient internal/integration failures only.
- where retries are forbidden:
  - deterministic syntax/semantic/type/function errors.

## Versioning and Compatibility

- compatibility promise with each dependent crate:
  - stable public API and core semantics within major version.
- breaking-change protocol:
  - proposal -> decision -> migration guide -> major release.
- deprecation window:
  - one minor release minimum for non-critical removals.

## Contract Tests Needed

- runtime integration tests for variable resolution contracts.
- parameter/action integration tests for MaybeExpression/Template behavior.
- cache-enabled vs cache-disabled parity tests.
- deterministic error-message shape tests for key failure classes.
