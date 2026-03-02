# Architecture

## Problem Statement

- **Business problem:** workflow automation must tolerate transient failures (network, DB, external APIs) without cascading outages or unbounded retries.
- **Technical problem:** need composable, type-safe resilience primitives (retry, circuit breaker, timeout, bulkhead, rate limit) with policy-driven configuration and observability.

## Positioning

`nebula-resilience` is an infra crate for runtime protection patterns, not business logic.

Dependency direction:
- engine/runtime/api/service adapters -> `nebula-resilience`
- `nebula-resilience` remains independent from domain workflows

## Current Architecture

### Internal Modules

- `core/`
  - foundational errors/results, traits, typed config/types, metrics, categories
- `patterns/`
  - concrete resilience primitives (retry, breaker, timeout, bulkhead, rate limiter, fallback, hedge)
- `manager.rs`
  - centralized service policy registration and protected execution orchestration
- `policy.rs`
  - serializable resilience policy model and validation
- `compose.rs`
  - layer-based middleware composition model
- `observability/`
  - hooks/spans/events for metrics and tracing integrations
- `retryable.rs`
  - lightweight trait bridge for domain error retry semantics

### Advanced Type System Design

The crate uses Rust's advanced type system features for zero-cost safety:

**Const generics** — compile-time configuration validation; invalid configs are caught before the binary is built:
```rust
// FAILURE_THRESHOLD=5, RESET_TIMEOUT_MS=30_000 are type-level constants
CircuitBreakerConfig::<5, 30_000>::new()
ExponentialBackoff::<100, 20, 5000>::default()  // base_ms, multiplier_x10, max_ms
ConservativeCondition::<3>::new()                // max_attempts
```

**Typestate pattern** — circuit breaker states (`Closed`, `HalfOpen`, `Open`) are tracked at the type level via phantom types. Invalid state transitions are compile errors. `TypestatePolicyBuilder` uses the same pattern: `Unconfigured → WithRetry → WithCircuitBreaker → Complete`.

**Phantom types / zero-cost markers** — strategy markers `Aggressive`, `Balanced`, `Conservative` are ZSTs that guide type inference without runtime cost.

**GATs** — `ResiliencePattern` trait uses generic associated types for flexible async operation handling.

**Sealed traits** — `ServiceCategory` and select internal traits are sealed to prevent external implementation while keeping the API open for consumption.

### Circuit Breaker State Machine

```
Closed ──(failure_threshold reached)──▶ Open
  ▲                                        │
  │                                  (reset_timeout)
  │                                        ▼
  └──(half_open_limit successes)──── HalfOpen
                                          │
                                  (any failure)
                                          │
                                          ▼
                                        Open
```

`CircuitState`: `Closed` (normal) | `Open` (fail-fast) | `HalfOpen` (probe).

### Data/Control Flow

1. Policy loaded (config or code) → `ResilienceManager` registers service.
2. Execution request → manager applies patterns (timeout, bulkhead, circuit, retry) in sequence.
3. Operation runs; on failure, retry/circuit/fallback logic applies.
4. Observability hooks fire; result returned to caller.

### Recovery Decisions for Workflow Execution

For workflow runs, callers such as `nebula-engine` and `nebula-runtime` use `nebula-resilience` to classify errors and derive **recovery decisions** rather than hard-coding retry logic in actions or engine:

- Error classifiers and retry policies map operation failures into categories like *transient*, *permanent*, or *throttled*.
- From these, callers can derive decisions such as:
  - "retry now" vs "retry after delay" vs "wait on external resource becoming healthy";
  - "fail this step/workflow" vs "defer to outbox/queue" vs "escalate".
- The resilience crate itself remains workflow-agnostic: it exposes policies, classifiers, and pattern results; engine/execution translate those into execution-time constructs (e.g. ephemeral wait/retry nodes and execution patches) owned by `nebula-execution`.

This keeps fault-handling policy centralized and serializable, while leaving the shape of the execution graph and any phantom/recovery steps to the execution layer.

### Known Bottlenecks

- DashMap contention under high service cardinality.
- Layer composition overhead in deep chains (future optimization).

## Target Architecture

- **Target module map:** same structure; add explicit pattern order contract; optional typed policy profiles.
- **Public contract boundaries:** stable `ResiliencePolicy`, `RetryPolicyConfig`; `ResilienceManager::execute`; pattern traits.
- **Internal invariants:** circuit state machine; retry attempt counting; bulkhead semaphore; rate limiter permits.

## Design Reasoning

- **Pattern-first vs manager-first:** primitives first, then orchestration — keeps primitives reusable and testable (D-001).
- **Serializable policy:** policies need external config and runtime loading (D-002).
- **Typed + untyped duality:** progressive adoption without breaking existing integrations (D-003).

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal/Prefect/Airflow.

- **Adopt:** circuit breaker + retry + timeout as standard layer; policy-as-config; observability hooks.
- **Reject:** implicit retry without explicit policy; hidden circuit state; no cancellation support.
- **Defer:** global retry budget; typed policy profiles; observability schema versioning (see PROPOSALS).

## Breaking Changes (if any)

- P-001 (pattern order): may require chain reordering.
- P-005 (cancellation): may change edge-case behavior.

## Open Questions

- Canonical pattern order (timeout → bulkhead → circuit → retry vs alternatives).
- Fail-open vs fail-closed defaults per pattern.
