# nebula-resilience

`nebula-resilience` provides fault-tolerance patterns for Nebula services.

## Scope

- **In scope:** retry, circuit breaker, timeout, bulkhead, rate limiting, fallback, hedge; cooperative shutdown barrier (`Gate`/`GateGuard`); composition; policy model; observability hooks.
- **Out of scope:** business logic, workflow orchestration, persistence, credential storage.

## Main Surface

- **Core:** `ResilienceError`, `ResilienceResult`, config/traits/types
- **Patterns:** `CircuitBreaker`, `RetryStrategy`, `Bulkhead`, `RateLimiter`, `timeout`, `fallback`, `hedge`
- **Sync utilities:** `Gate`, `GateGuard`, `GateClosed` — cooperative shutdown barrier for in-flight task drain
- **Manager:** `ResilienceManager`, `PolicyBuilder`, typed/untyped service execution
- **Policy:** `ResiliencePolicy`, `RetryPolicyConfig`, `PolicyMetadata`
- **Composition:** `LayerBuilder`, `ResilienceChain`, `ResilienceLayer`
- **Observability:** events/hooks/spans utilities

## Document Map

- [PATTERNS.md](PATTERNS.md)
- [API.md](API.md)
- [RELIABILITY.md](RELIABILITY.md)
- [MIGRATION.md](MIGRATION.md)

## Notes

- This docs set is intentionally compact: only active operational/API/migration guides are kept here.
- Architecture, security, test strategy, and performance gate summaries are consolidated into `PATTERNS.md` and `RELIABILITY.md`.
