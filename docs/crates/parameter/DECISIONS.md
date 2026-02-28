# Decisions

Use this format for every decision:

## D001: Schema-first, Value-later Model

Status: Adopt

Context: Workflow nodes need flexible transport/storage compatibility and dynamic execution; compile-time guarantees cannot extend to runtime JSON values.

Decision: Define parameter structure as typed Rust schema; runtime values remain `serde_json::Value`.

Alternatives considered: Typed value enum at runtime; compile-time generics for value types.

Trade-offs: Type mismatch detection pushed to validation boundary; JSON enables persistence and cross-layer portability.

Consequences: Validation layer must enforce type compatibility; typed extraction helpers (`get_string`, `get_f64`) provide convenience.

Migration impact: None; established pattern.

Validation plan: Serde round-trip tests; validation tests for type mismatches.

---

## D002: Declarative Validation Rules

Status: Adopt

Context: Validation constraints must be serializable for persistence, tooling, and cross-layer portability.

Decision: Represent validation constraints as serializable `ValidationRule` data; evaluate in validation layer via `nebula-validator`.

Alternatives considered: Procedural validation in each parameter type; external DSL.

Trade-offs: Rule semantics delegated to validator; Custom rules require expression engine (evaluated elsewhere).

Consequences: `ParameterCollection::validate` aggregates errors; rule evolution needs versioning (P-005).

Migration impact: None.

Validation plan: Rule round-trip; validator integration tests.

---

## D003: Capability-based Kind Semantics

Status: Adopt

Context: Downstream logic (UI, engine, render) needs clear behavior flags per parameter kind.

Decision: Model behavior via `ParameterCapability` flags instead of hardcoded ad-hoc checks.

Alternatives considered: Per-kind match arms; boolean fields on metadata.

Trade-offs: Centralized capability definition; new kinds require capability update.

Consequences: `ParameterKind` exposes `has_value`, `is_editable`, `is_validatable`, etc.

Migration impact: None.

Validation plan: Capability consistency tests per kind.

---

## D004: Recursive Containers

Status: Adopt

Context: Real-world node configs require hierarchical inputs (connection objects, list of items).

Decision: Support nested schema structures through Object, List, Mode, Group, Expirable.

Alternatives considered: Flat-only; fixed-depth nesting.

Trade-offs: Validation recursion; path building for error reporting.

Consequences: `validate_param` recurses for Object/List; dotted paths in errors.

Migration impact: None.

Validation plan: Deep nesting tests; path correctness.

---

## D005: Error Aggregation over Fail-fast

Status: Adopt

Context: Complex parameter forms benefit from seeing all validation failures at once.

Decision: `ParameterCollection::validate` accumulates errors instead of returning first failure.

Alternatives considered: Fail-fast; configurable mode.

Trade-offs: More allocations for large error sets; better UX and faster debugging.

Consequences: Caller receives `Vec<ParameterError>`; API/UI can present all issues.

Migration impact: None.

Validation plan: Multi-error validation tests.
