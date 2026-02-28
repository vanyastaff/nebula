# Decisions

## D001: Type-bound validation as primary API

Status: Adopt

Context:
- Rust platform needs compile-time safety and low runtime ambiguity.

Decision:
- keep `Validate<T>` + trait bounds as primary contract.

Alternatives considered:
- dynamic schema-only validation as primary model.

Trade-offs:
- stronger safety, but more generic type complexity.

Consequences:
- robust refactoring and fewer runtime type errors.

Migration impact:
- none.

Validation plan:
- compile-time tests + integration tests across consuming crates.

## D002: Combinator-first composition

Status: Adopt

Context:
- workflows require reusable rule pipelines.

Decision:
- compose via `and/or/not/when/unless/optional/field/json_field/cached`.

Alternatives considered:
- custom monolithic validators per use-case.

Trade-offs:
- excellent reuse, but bigger types and more compile-time cost.

Consequences:
- expressive and maintainable validation graphs.

Migration impact:
- none.

Validation plan:
- combinator law tests + behavior regression tests.

## D003: Structured error model with nested context

Status: Adopt

Context:
- downstream crates need machine-readable and human-readable diagnostics.

Decision:
- preserve `ValidationError` rich schema (code/message/field/params/nested/help/severity).

Alternatives considered:
- plain string errors.

Trade-offs:
- richer output with modest allocation overhead.

Consequences:
- easier API/UI mapping and debugging.

Migration impact:
- must keep code semantics stable.

Validation plan:
- serialization and compatibility tests for error payloads.

## D004: Error code registry governance

Status: Defer

Context:
- code stability rules exist conceptually but not yet formalized as registry.

Decision:
- defer full registry rollout until compatibility suite is in place.

Alternatives considered:
- immediate hard enforcement.

Trade-offs:
- faster current iteration vs delayed strictness.

Consequences:
- short-term risk of accidental code drift.

Migration impact:
- future minor-to-major planning required.

Validation plan:
- add registry in roadmap phase with backward mapping tests.

## D005: FieldPath typed model

Status: Defer

Context:
- plain string field paths can drift in formatting.

Decision:
- evaluate typed `FieldPath` model after current docs/contract stabilization.

Alternatives considered:
- keep raw strings forever.

Trade-offs:
- typed path improves safety but may require API changes.

Consequences:
- maintain current compatibility now; prepare for eventual major upgrade.

Migration impact:
- likely breaking if introduced broadly.

Validation plan:
- prototype in proposal + adapter compatibility layer.
