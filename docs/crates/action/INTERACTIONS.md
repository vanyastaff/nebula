# Interactions

## Primary integrations

## `nebula-action` <-> `nebula-core`

- uses core IDs and interface versioning primitives
- must follow core compatibility rules for versioned contracts

Contract:
- metadata version increments only on schema/port contract changes

## `nebula-action` <-> `nebula-parameter`

- action metadata contains parameter definitions (`ParameterCollection`)
- parameter validation should happen before action execution

Contract:
- invalid parameter payload maps to `ActionError::Validation`

## `nebula-action` <-> `nebula-credential`

- action declares credential needs via `ActionComponents::credential(...)`
- runtime resolves and injects credentials according to sandbox policy

Contract:
- missing or forbidden credential access must fail predictably (fatal or sandbox violation)

## `nebula-action` <-> `nebula-resource`

- action declares resource needs via `ActionComponents::resource(...)`
- runtime provides scoped resource access through context adapters

Contract:
- resource unavailability should surface as retryable or fatal according to runtime mapping policy

## `nebula-action` <-> `nebula-sandbox`

- sandbox wraps context calls and enforces declared capabilities
- undeclared access becomes `ActionError::SandboxViolation`

Contract:
- same action code can run in-process or sandboxed with identical semantic outcomes

## `nebula-action` <-> `nebula-runtime` / `nebula-engine`

- engine interprets `ActionResult` and advances workflow graph
- runtime resolves deferred/streaming outputs per resolution contract

Contract:
- action crate never directly orchestrates retries, scheduling, or DAG transitions

## `nebula-action` <-> `nebula-resilience`

- resilience policy consumes `ActionError` and `ActionResult::Retry` hints
- retries/backoff/budgets stay outside action crate

Contract:
- retryability signal from action remains advisory but explicit

## `nebula-action` <-> `nebula-log`

- action code and runtime adapters emit structured logs/traces
- log crate is optional integration, not part of action trait contract

## Interaction sequence (target)

1. Runtime validates parameters and capability envelope.
2. Runtime prepares sandbox-aware context.
3. Action executes and returns `ActionResult<ActionOutput<T>>`.
4. Engine interprets control flow.
5. Runtime resolves deferred/streaming outputs if required.
6. Resilience layer applies retry strategy on retryable failures/signals.
7. Observability layer emits structured events/metrics.
