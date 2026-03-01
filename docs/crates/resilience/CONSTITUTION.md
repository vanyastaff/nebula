# nebula-resilience Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Workflow engine, runtime, API, and external calls face failures: transient network errors, overloaded services, and timeouts. Retry, circuit breaker, timeout, bulkhead, and rate limiting prevent cascading failures and give operators predictable behavior. A single resilience crate keeps policies consistent and composable.

**nebula-resilience provides fault-tolerance patterns for Nebula services.**

It answers: *How do callers wrap fallible operations with retry, circuit breaker, timeout, bulkhead, and rate limiting — with composable layers and observable behavior?*

```
Caller has fallible operation F
    ↓
ResilienceManager or LayerBuilder wraps F with Retry, CircuitBreaker, Timeout, Bulkhead, RateLimiter
    ↓
Execute with policy; on failure classify (retryable vs fatal) and apply pattern
    ↓
Observability hooks (events, spans) for metrics and debugging
```

This is the resilience contract: patterns are composable; policy is serializable; observability is optional and non-blocking.

---

## User Stories

### Story 1 — Runtime Retries Action on Transient Failure (P1)

Runtime executes an action; action returns retryable error. Resilience layer retries with backoff (exponential or fixed) up to N times, then fails. Engine sees final success or failure.

**Acceptance**:
- RetryStrategy with max_retries, backoff, and optional retryable-classifier
- ResilienceError or result allows caller to distinguish retry exhaustion vs fatal
- No blocking of execution path on observability

### Story 2 — API Protects Downstream with Circuit Breaker (P1)

API calls external service (e.g. credential store). After N consecutive failures, circuit opens and calls fail fast until half-open window. Resilience provides CircuitBreaker pattern and policy.

**Acceptance**:
- CircuitBreaker with configurable threshold, half-open window
- Fail-open or fail-closed configurable per deployment
- Events or hooks for state transitions (closed → open → half-open)

### Story 3 — Operator Tunes Policy Without Code Change (P2)

Retry and circuit breaker policies are config-driven (e.g. from nebula-config). Operator changes max_retries or timeout via config; no redeploy of business logic.

**Acceptance**:
- ResiliencePolicy (or equivalent) is serializable and loadable from config
- PolicyBuilder or similar for programmatic build
- Document policy schema and precedence

### Story 4 — Observability Without Blocking (P2)

Metrics and tracing need to see retry attempts, circuit state, and timeouts. Hooks or events are fire-and-forget; slow subscriber does not delay the wrapped operation.

**Acceptance**:
- Optional observability hooks (on_retry, on_circuit_open, etc.)
- Hook failure is logged; does not affect operation result
- Spans or events for integration with telemetry crate

---

## Core Principles

### I. Composable Patterns

**Retry, circuit breaker, timeout, bulkhead, rate limiter are separate layers that can be composed in a defined order.**

**Rationale**: Different call sites need different combinations. Composition allows reuse and consistent semantics (e.g. retry inside circuit breaker).

**Rules**:
- LayerBuilder or ResilienceChain composes patterns
- Canonical order documented (e.g. timeout → retry → circuit breaker → bulkhead)
- No single monolithic "resilience wrapper" that cannot be decomposed

### II. Policy Is Serializable and Versioned

**Policies (retry config, circuit breaker config) are data that can be loaded from config and stored. Schema is versioned for compatibility.**

**Rationale**: Operators tune policies without code change. Versioning prevents breakage when policy format evolves.

**Rules**:
- ResiliencePolicy (or per-pattern config) Serialize/Deserialize
- Minor = additive policy fields; major = migration for breaking policy change
- Document policy schema and defaults

### III. Fail-Open vs Fail-Closed Is Explicit

**Circuit breaker and similar patterns have configurable behavior when in "open" or error state: fail-open (allow call through) vs fail-closed (reject).**

**Rationale**: Production safety. Default should be documented; operators choose per use case.

**Rules**:
- Document default and option for each pattern
- Observability event when pattern triggers (e.g. circuit opened)

### IV. No Business Logic in Resilience Crate

**Resilience provides patterns and policy. It does not implement workflow, storage, or credential logic.**

**Rationale**: Single responsibility. Callers (runtime, API, credential) wrap their operations.

**Rules**:
- No dependency on engine, storage, credential for pattern implementation
- Optional integration with telemetry for hooks only

### V. Observability Is Non-Blocking

**Hooks and events must not block the wrapped operation or add significant latency.**

**Rationale**: Resilience is on the hot path. Observability failures must not become user-facing failures.

**Rules**:
- Hooks are fire-and-forget or best-effort
- No synchronous "wait for metrics" in execute path

---

## Production Vision

### The resilience layer in an n8n-class fleet

In production, every outbound call (action execution, credential fetch, API proxy) can be wrapped with retry, circuit breaker, timeout, bulkhead, and rate limiting. Policies are loaded from config. Circuit state and retry counts are visible in metrics. Pattern order is canonical so that behavior is predictable across services.

```
ResilienceManager / LayerBuilder
    ├── Timeout: hard limit per call
    ├── Retry: backoff on retryable errors
    ├── CircuitBreaker: fail fast after N failures; half-open probe
    ├── Bulkhead: limit concurrent calls
    └── RateLimiter: limit calls per window
```

Observability: events for circuit open/close, retry attempts, timeout; optional span attributes. Policy serialization is stable for config and API.

### From the archives: pattern order and cross-cutting role

The archive `_archive/archive-business-cross.md` and resilience docs place resilience in cross-cutting concerns. Production vision: canonical pattern order contract, fail-open/fail-closed defaults per pattern, observability schema versioning. Policy serialization stable across minor releases.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|----------|-------|
| Canonical pattern order contract (documented + enforced) | High | Timeout → Retry → CircuitBreaker → Bulkhead (or agreed order) |
| Fail-open/fail-closed defaults per pattern | High | Document and make configurable |
| Observability schema versioning | Medium | Events and hooks have stable schema |
| Typed vs untyped manager duality simplification | Medium | Reduce API surface or document clearly |
| Policy serialization compatibility tests | Medium | Fixture-lock policy format for minor |

---

## Key Decisions

### D-001: Patterns as Composable Layers

**Decision**: Each pattern (Retry, CircuitBreaker, etc.) is a layer; LayerBuilder or chain composes them.

**Rationale**: Reuse and testability. Callers choose which patterns to apply.

**Rejected**: Single "ResilienceWrapper" with all patterns hardcoded — inflexible.

### D-002: Policy Serializable for Config

**Decision**: RetryPolicyConfig, circuit breaker config, etc. implement Serialize/Deserialize for config loading.

**Rationale**: Operators tune without code change. Config crate can validate policy shape.

**Rejected**: Policy only in code — would require redeploy for every tuning.

### D-003: Async-First and Type-Safe

**Decision**: Patterns work with async operations; use const generics or typestate where it improves safety.

**Rationale**: Nebula is async; sync-only would force blocking. Type safety reduces misconfiguration.

**Rejected**: Sync-only or untyped only — would not fit engine/runtime.

### D-004: Observability Hooks Optional

**Decision**: Events/hooks for observability are optional; pattern behavior does not depend on them.

**Rationale**: Minimal builds and tests may not need metrics. Observability must not affect correctness.

**Rejected**: Mandatory hooks — would block use in tests or minimal deployments.

---

## Open Proposals

### P-001: Canonical Order and Documentation

**Problem**: Pattern order affects behavior (e.g. retry inside vs outside circuit breaker).

**Proposal**: Document and enforce single canonical order; provide LayerBuilder preset.

**Impact**: Non-breaking if current usage already follows; document for new users.

### P-002: Policy Compatibility Fixtures

**Problem**: Policy format could regress in minor release.

**Proposal**: Versioned fixture for ResiliencePolicy (or equivalent); CI checks additive-only for minor.

**Impact**: Additive; improves stability.

---

## Non-Negotiables

1. **Patterns are composable** — retry, circuit breaker, timeout, bulkhead, rate limiter as layers.
2. **Policy is serializable** — loadable from config; schema versioned.
3. **Observability is non-blocking** — hooks do not block or fail the operation.
4. **No business logic in resilience** — only patterns and policy; callers wrap their ops.
5. **Fail-open/fail-closed explicit** — documented per pattern; configurable.
6. **Breaking policy or pattern contract = major + MIGRATION.md** — operators depend on behavior.

---

## Governance

- **PATCH**: Bug fixes, docs. No change to pattern semantics or policy format.
- **MINOR**: Additive (new pattern options, new policy fields). No removal.
- **MAJOR**: Breaking changes to pattern or policy. Requires MIGRATION.md.
