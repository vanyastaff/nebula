# Decisions

## D001: String resource ids as primary registry key

Status: Accepted

`Manager` indexes pools by `Resource::id()` string. This allows multiple resource flavors and explicit config-level naming.  
Tradeoff: runtime lookup errors are possible if id mismatch; mitigated by typed helpers and tests.

## D002: Scope containment is explicit and deny-by-default

Status: Accepted

Scope compatibility uses parent chain consistency; missing parent metadata in child scope does not imply access.  
Reason: hard multi-tenant isolation and transitive safety.

## D003: Hooks and events are additive observability layers

Status: Accepted

Core acquire/release must work without hooks/metrics/tracing.  
Observability failures should not break functional flow (except explicit hook cancellation before acquire/create).

## D004: Optional features for heavyweight integrations

Status: Accepted

`metrics`, `tracing`, `credentials` are feature-gated to keep base crate lean.

## D005: Pool is generic over `Resource` and drives lifecycle

Status: Accepted

`Pool<R>` calls `create`, `is_valid`, `recycle`, `cleanup`.  
Reason: clear ownership of instance lifecycle and better compile-time correctness.

## D006: Health is split into validation and monitoring

Status: Accepted

Fast-path validity check happens during acquire/reuse.  
Background `HealthChecker` handles longer-running liveness trends and threshold actions (quarantine, escalation).
