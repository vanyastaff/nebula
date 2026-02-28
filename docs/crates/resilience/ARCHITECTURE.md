# Architecture

## Positioning

`nebula-resilience` is an infra crate for runtime protection patterns, not business logic.

Dependency direction:
- engine/runtime/api/service adapters -> `nebula-resilience`
- `nebula-resilience` remains independent from domain workflows

## Internal Modules

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

## Design Characteristics

- async-first pattern APIs
- heavy use of type-safety patterns (const generics, typestate, marker traits)
- composition-focused architecture (patterns + manager + chain layers)
- explicit policy model for per-service customization

## Operational Concerns

- large API surface with both typed and compatibility abstractions
- potential complexity in mixed typed/untyped manager usage
- policy + runtime behavior must stay synchronized with observability semantics
