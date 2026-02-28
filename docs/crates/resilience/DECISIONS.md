# Decisions

## D-001: Pattern-first Abstraction

Status: accepted

Decision:
- implement resilience as composable primitives first, then orchestrate via manager/layer abstractions.

Reason:
- keeps primitives reusable and testable independently.

## D-002: Serializable Policy Model

Status: accepted

Decision:
- expose `ResiliencePolicy` + `RetryPolicyConfig` as serializable configuration contracts.

Reason:
- policies need external configuration and runtime loading.

## D-003: Typed + Compatibility API Duality

Status: accepted

Decision:
- keep typed service/category APIs while preserving untyped compatibility interfaces.

Reason:
- enables progressive adoption without breaking existing integrations.

## D-004: Error-centered Retry Decisions

Status: accepted

Decision:
- retry behavior depends on explicit `is_retryable()` semantics in error model/traits.

Reason:
- deterministic and debuggable retry control.

## D-005: Observability Built-in

Status: accepted

Decision:
- expose hooks/spans/events in crate surface rather than external adapters only.

Reason:
- resilience behavior must be inspectable in production by default.
