# nebula-resilience

`nebula-resilience` provides fault-tolerance patterns for Nebula services.

Main capabilities:
- retry strategies and retry conditions
- circuit breaker
- timeout and bulkhead isolation
- rate limiting (token/leaky/sliding/adaptive + governor)
- fallback and hedge patterns
- centralized resilience manager and policy model
- observability hooks/spans integration

## Role in Platform

For a Rust n8n-like automation platform, this crate is the execution safety layer around unstable IO and dependency boundaries (HTTP, DB, queues, external APIs).

## Main Surface

- Core: `ResilienceError`, `ResilienceResult`, config/traits/types
- Patterns: `CircuitBreaker`, `RetryStrategy`, `Bulkhead`, `RateLimiter`, `timeout`, `fallback`, `hedge`
- Manager: `ResilienceManager`, `PolicyBuilder`, typed/untyped service execution
- Policy: `ResiliencePolicy`, `RetryPolicyConfig`, `PolicyMetadata`
- Composition: `LayerBuilder`, `ResilienceChain`, `ResilienceLayer`
- Observability: events/hooks/spans utilities

## Document Set

- [ARCHITECTURE.md](ARCHITECTURE.md)
- [API.md](API.md)
- [DECISIONS.md](DECISIONS.md)
- [ROADMAP.md](ROADMAP.md)
- [PROPOSALS.md](PROPOSALS.md)
