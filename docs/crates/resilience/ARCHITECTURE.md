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

### Data/Control Flow

1. Policy loaded (config or code) → `ResilienceManager` registers service.
2. Execution request → manager applies patterns (timeout, bulkhead, circuit, retry) in sequence.
3. Operation runs; on failure, retry/circuit/fallback logic applies.
4. Observability hooks fire; result returned to caller.

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
