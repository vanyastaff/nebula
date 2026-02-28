# Proposals (Senior Review)

## P-001: Deterministic Pattern Ordering Contract (Potential Breaking)

Problem:
- composed chains can be sensitive to layer order assumptions.

Proposal:
- define and enforce canonical execution order contract for manager and `LayerBuilder`.

Impact:
- some existing chains may behave differently after order normalization.

## P-002: Unified Retry Budget Model

Problem:
- retries are configured per policy, but system-level retry budgets are not explicit.

Proposal:
- add global/per-service retry budgets (attempts + time window) with backpressure hooks.

Impact:
- behavior changes under heavy failure; improves downstream protection.

## P-003: Typed Policy Profiles

Problem:
- policy misconfiguration risk remains high with purely dynamic values.

Proposal:
- add compile-time-safe profile constructors for common service classes (db/http/queue/cache).

Impact:
- non-breaking additive feature, stronger defaults and safer onboarding.

## P-004: Observability Schema Versioning

Problem:
- event/metric shape drift can break dashboards and alerts.

Proposal:
- define versioned observability schema for pattern/manager events.

Impact:
- non-breaking if additive; prevents accidental telemetry breakage.

## P-005: Explicit Cancellation Semantics Across Patterns

Problem:
- cancellation handling differs subtly between timeout/retry/hedge/bulkhead flows.

Proposal:
- unify cancellation contract and document propagation guarantees.

Impact:
- may require breaking behavioral adjustments in edge cases, but improves correctness.
