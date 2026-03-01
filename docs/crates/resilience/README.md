# nebula-resilience

`nebula-resilience` provides fault-tolerance patterns for Nebula services.

## Scope

- **In scope:** retry, circuit breaker, timeout, bulkhead, rate limiting, fallback, hedge; composition; policy model; observability hooks.
- **Out of scope:** business logic, workflow orchestration, persistence, credential storage.

## Current State

- **Maturity:** production-ready patterns; manager API evolving; policy serialization stable.
- **Key strengths:** type-safe patterns (const generics, typestate), composable layers, async-first.
- **Key risks:** large API surface; typed/untyped manager duality; pattern ordering semantics.

## Target State

- **Production criteria:** canonical pattern order contract; fail-open/fail-closed defaults per pattern; observability schema versioning.
- **Compatibility guarantees:** stable policy serialization; semantic versioning for public APIs.

## Main Surface

- **Core:** `ResilienceError`, `ResilienceResult`, config/traits/types
- **Patterns:** `CircuitBreaker`, `RetryStrategy`, `Bulkhead`, `RateLimiter`, `timeout`, `fallback`, `hedge`
- **Manager:** `ResilienceManager`, `PolicyBuilder`, typed/untyped service execution
- **Policy:** `ResiliencePolicy`, `RetryPolicyConfig`, `PolicyMetadata`
- **Composition:** `LayerBuilder`, `ResilienceChain`, `ResilienceLayer`
- **Observability:** events/hooks/spans utilities

## Document Map

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
- [ARCHITECTURE.md](ARCHITECTURE.md)
- [API.md](API.md)
- [INTERACTIONS.md](INTERACTIONS.md)
- [DECISIONS.md](DECISIONS.md)
- [ROADMAP.md](ROADMAP.md)
- [PROPOSALS.md](PROPOSALS.md)
- [SECURITY.md](SECURITY.md)
- [RELIABILITY.md](RELIABILITY.md)
- [TEST_STRATEGY.md](TEST_STRATEGY.md)
- [MIGRATION.md](MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
