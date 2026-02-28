# Decisions

## D-001: Typed IDs Everywhere

Status: accepted

Decision:
- Keep domain-specific typed IDs (`WorkflowId`, `NodeId`, etc.) instead of plain strings.

Why:
- Compile-time protection from ID mix-ups.
- Clear domain semantics in signatures and logs.

Consequence:
- Slightly more type conversions at API boundaries.
- Better long-term safety and maintainability.

## D-002: Core Is Dependency-Leaf

Status: accepted

Decision:
- `nebula-core` must not depend on feature/domain crates.

Why:
- Prevent cyclic dependencies.
- Keep foundation reusable by all layers.

Consequence:
- Some abstractions in `core` stay generic and intentionally minimal.

## D-003: Explicit Scope Model

Status: accepted

Decision:
- Use `ScopeLevel` as first-class model for lifecycle and ownership.

Why:
- n8n-like automation platforms need clear lifecycle boundaries.
- Enables consistent resource cleanup and access checks.

Consequence:
- Scope relationships must be maintained carefully when new layers are added.

## D-004: Unified Core Error Vocabulary

Status: accepted

Decision:
- Keep `CoreError` broad enough for foundational operations.

Why:
- Shared behavior (serialization, context propagation, lower-level helpers) needs common error categories.

Consequence:
- Higher-level crates still define their own error enums; `CoreError` is not a substitute for domain errors.

## D-005: Serde-First Public Types

Status: accepted

Decision:
- Public cross-crate data types in `core` should remain serializable/deserializable.

Why:
- Data crosses boundaries: API, storage, telemetry, queueing, plugin interfaces.

Consequence:
- Schema compatibility discipline is required (especially for enums and external representations).
