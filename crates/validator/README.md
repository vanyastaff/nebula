---
name: nebula-validator
role: Validation Rules Engine + Declarative Rule
status: frontier
last-reviewed: 2026-04-17
canon-invariants: [L1-3.5, L1-4.5]
related: [nebula-schema, nebula-expression, nebula-error, nebula-core]
---

# nebula-validator

## Purpose

Integration code and schema fields need validation at two points: structural checks at
schema-lint time, and runtime value checks when an execution resolves field values. Without a
shared validation layer each integration author re-implements their own length checks, pattern
tests, and combinator logic, which fragments error message quality and makes cross-field
conditional rules impossible to express declaratively. `nebula-validator` provides the single
shared rules engine: composable programmatic validators for use in Rust code, and a
JSON-serializable `Rule` enum that schema fields carry for engine-evaluated validation at
activation and runtime.

## Role

**Validation Rules Engine + Declarative Rule.** The crate that `nebula-schema` delegates
rule evaluation to when schema fields declare constraints. Pattern inspiration: *Make illegal
states unrepresentable* (Domain Modeling Made Functional) — `Validated<T>` is a proof-token
that a value passed validation; the type cannot be constructed without calling `validate`.

Related DMMF / typestate discussion: `docs/GLOSSARY.md` §9, `docs/STYLE.md`.

## Public API

- `Validate<T>` (`foundation::Validate`) — core trait every validator implements.
- `ValidateExt<T>` (`foundation::ValidateExt`) — combinator methods: `.and()`, `.or()`, `.not()`.
- `Validated<T>` (`proof::Validated`) — proof-token certifying a value passed validation.
- `ValidationError` (`foundation::ValidationError`) — structured error (80 bytes, `Cow`-based, RFC 6901 field paths).
- `AnyValidator<T>` (`foundation::AnyValidator`) — type-erased validator for dynamic dispatch.
- `Rule` — typed sum-of-sums: `Value(ValueRule)` / `Predicate(Predicate)` / `Logic(Box<Logic>)` / `Deferred(DeferredRule)` / `Described(Box<Rule>, String)`. Each inner kind owns exactly one method that makes sense for it; cross-kind silent-pass is a compile error.
- `FieldPath` — RFC 6901 JSON-pointer with construction-time validation (replaces raw `String` paths in predicates).
- `Described` — decorator with `{placeholder}` message templates (replaces per-variant `message: Option<String>` fields).
- `RuleContext` — context map for predicate evaluation (sibling field lookups).
- `ExecutionMode` — controls which rule categories run (`StaticOnly`, `Deferred`, `Full`).
- `validate_rules` — batch-evaluate a slice of `Rule` against a `serde_json::Value`.
- `ValidatorError` — crate-level operational error type.
- `validator!` macro — zero-boilerplate custom validator.
- `#[derive(Validator)]` — derive macro (feature `derive`) for struct-level validation.
- `prelude` — single-import convenience module.

Built-in validators: string (`MinLength`, `MaxLength`, `NotEmpty`, `Contains`,
`Alphanumeric`), numeric (`Min`, `Max`, `InRange`), collection (`MinSize`, `MaxSize`),
boolean (`IsTrue`, `IsFalse`), nullable (`Required`), network (`Ipv4`, `Hostname`),
temporal (`DateTime`, `Uuid`).

## Contract

- **[L1-§3.5]** Schema is the typed-configuration surface for all integration concepts;
  `nebula-validator` is the rules engine that `nebula-schema` delegates Rule evaluation to.
  See `docs/INTEGRATION_MODEL.md`.
- **[L1-§4.5]** `Validated<T>` is a proof-token: a caller cannot obtain one without calling
  `validate`. `Validated<T>` deliberately does not implement `Deserialize` — deserialized
  data must be re-validated.
- **Rule cross-kind safety** — each inner kind (`ValueRule`, `Predicate`, `Logic`, `DeferredRule`) exposes only the method that makes sense for it. Calling a value-only method on a predicate-carrying `Rule` is a compile error (typed narrowing). This replaces the old flat enum's documented silent-pass ergonomics. Seam: `crates/validator/src/rule/mod.rs`. Tests: `crates/validator/tests/`.
- **Wire format compactness** — externally-tagged tuple-compact encoding keeps compound-rule JSON ~60% smaller than the old flat variants.

## Non-goals

- Not a schema system — see `nebula-schema` for `Field`, `Schema`, and the proof-token
  pipeline (`ValidValues` → `ResolvedValues`).
- Not an expression evaluator — see `nebula-expression` for dynamic field resolution.
- Not a resilience pipeline — see `nebula-resilience` for retry / circuit-breaker.
- Not an API error formatter — see `nebula-api` for RFC 9457 `problem+json` mapping.

## Maturity

See `docs/MATURITY.md` row for `nebula-validator`.

- API stability: `frontier` — the `Rule` type just moved from a flat 30-variant enum to the
  typed sum-of-sums above (commit landed; `docs/superpowers/specs/2026-04-17-nebula-validator-rule-refactor-design.md`).
  The programmatic validator API (`Validate<T>`, `ValidateExt`, `Validated<T>`,
  `ValidationError`) is stable and unchanged. Wire format for `Rule` JSON has changed
  (externally-tagged tuple-compact encoding); consumers must re-serialize any stored
  rule data. Alpha-stage breakage acknowledged.
- The `#[derive(Validator)]` macro public attribute syntax is stable across the refactor.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.5, §4.5.
- Glossary: `docs/GLOSSARY.md` §5 (`Rule`, `ValidValues`, `ResolvedValues`).
- Siblings: `nebula-schema` (consumes `Rule`), `nebula-error` (error taxonomy).
- Spec: `docs/superpowers/specs/2026-04-17-nebula-validator-rule-refactor-design.md` — Rule type split (Refactor 1).

## Appendix

### Existing extended documentation

The crate carries a rich secondary docs set in `crates/validator/docs/`:

| Document | Contents |
|----------|----------|
| `docs/README.md` | Core concepts, feature matrix, crate layout |
| `docs/architecture.md` | Design decisions, module map, data flow, invariants |
| `docs/api-reference.md` | Every public type, trait, and method |
| `docs/combinators.md` | Full combinator catalog and composition patterns |
| `docs/extending.md` | Writing custom validators, the `validator!` macro |
| `docs/migration.md` | Versioning policy, breaking changes, migration paths |

### Error code stability

Error codes are stable across minor releases; the registry lives in
`tests/fixtures/compat/error_registry_v1.json`.

### Quick Start

```rust
use nebula_validator::prelude::*;

// Compose validators with .and() / .or() / .not()
let username = min_length(3).and(max_length(20)).and(alphanumeric());
assert!(username.validate("alice").is_ok());
assert!(username.validate("ab").is_err()); // min_length fails

// Proof token: validate once, carry the guarantee in the type system
let name: Validated<String> = min_length(3).validate_into("alice".to_string())?;
// fn process(name: Validated<String>) — the compiler enforces the check happened
```
