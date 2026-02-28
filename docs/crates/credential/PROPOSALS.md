# Proposals (Senior Review)

## P-001: Mandatory Capability Negotiation for Providers (Potential Breaking)

Problem:
- provider behavior can differ subtly by backend.

Proposal:
- introduce explicit provider capability negotiation at manager startup.

Impact:
- stricter startup validation, potential configuration breakage for ambiguous setups.

## P-002: Strict Scope Enforcement Mode (Potential Breaking)

Problem:
- scope handling mistakes are catastrophic in multi-tenant systems.

Proposal:
- add strict mode that rejects any operation with incomplete/ambiguous scope context.

Impact:
- some existing calls may fail until context propagation is fixed.

## P-003: Rotation Policy Versioning

Problem:
- policy evolution risks schema drift for persisted metadata.

Proposal:
- add explicit versioned policy envelope and migration helpers.

Impact:
- non-breaking initially with compatibility parser; long-term safer migrations.

## P-004: Secret Access Budget and Rate Controls

Problem:
- high-load systems can over-pull secrets and stress provider backends.

Proposal:
- introduce configurable fetch budgets/rate limits with backpressure semantics.

Impact:
- behavior changes under load; improves resilience and cost control.

## P-005: Unified Error Taxonomy Across Modules

Problem:
- manager/provider/rotation errors are rich but can fragment observability pipelines.

Proposal:
- define shared machine-readable taxonomy and mapping guide for all error families.

Impact:
- additive documentation + helper APIs; large observability payoff.
