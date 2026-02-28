# Decisions

## D001: Tenant policy must have a single owner crate

Status: Adopt

Context:

Tenant logic currently appears across multiple crates without one authoritative boundary.

Decision:

`nebula-tenant` will own tenant context, policy, and quota contracts.

Alternatives considered:

- keep tenant policy distributed across runtime/storage/resource

Trade-offs:

- pro: consistency and auditable governance
- con: introduces central dependency

Consequences:

Integration contracts become clearer and testable.

Migration impact:

Other crates migrate ad-hoc logic to tenant APIs over time.

Validation plan:

Cross-crate contract tests and migration gates.

## D002: Fail-closed defaults for identity and isolation

Status: Adopt

Context:

Multi-tenant platforms cannot tolerate ambiguous context handling.

Decision:

Unknown/invalid tenant context is denied by default.

Alternatives considered:

- permissive fallback to global/system tenant

Trade-offs:

- pro: safer by design
- con: stricter onboarding requirements

Consequences:

Operational tooling must support clear diagnostics for rejected requests.

Migration impact:

Existing permissive paths need explicit exceptions or system scopes.

Validation plan:

Ingress and cross-tenant denial integration tests.

## D003: Quota checks are explicit runtime control points

Status: Adopt

Context:

Quota decisions affect scheduling, resources, and user experience.

Decision:

Runtime asks tenant policy at explicit checkpoints (admission/scheduling/resource-intensive ops).

Alternatives considered:

- hidden quota checks only inside storage/resource modules

Trade-offs:

- pro: predictable control flow and observability
- con: more integration wiring

Consequences:

Checkpoint APIs and audit logs become mandatory.

Migration impact:

Runtime flows need policy checkpoints added.

Validation plan:

Quota checkpoint coverage in integration suite.

## D004: Partition strategy is policy-driven and pluggable

Status: Adopt

Context:

Different tenants may require different isolation costs/levels.

Decision:

Support strategy abstraction (`schema/table/rls/database-per-tenant`) with explicit policy mapping.

Alternatives considered:

- single hardcoded partition mode

Trade-offs:

- pro: flexibility for cost/security tiers
- con: operational complexity

Consequences:

Storage integration contract must encode strategy compatibility.

Migration impact:

Data migration tooling required for strategy transitions.

Validation plan:

Strategy compatibility tests and migration rehearsals.

## D005: Initial release starts with stable core, advanced controls deferred

Status: Defer

Context:

Need to reduce first-implementation risk.

Decision:

First implementation focuses on context validation + baseline quota + audit hooks; advanced adaptive controls deferred.

Alternatives considered:

- ship full adaptive/autoscaling tenant governance immediately

Trade-offs:

- pro: faster path to reliable baseline
- con: fewer advanced optimization knobs initially

Consequences:

Roadmap phases advanced capabilities after baseline stability.

Migration impact:

Additive follow-up releases for advanced features.

Validation plan:

Definition-of-done per phase with measurable exit criteria.
