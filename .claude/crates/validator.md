# nebula-validator
Composable, type-safe validation framework — two paradigms: programmatic combinators and JSON-serializable declarative Rules.

## Invariants
- `ValidationError` is capped at 80 bytes (`Cow`-based). Intentional: boxing adds indirection to every validation call. `clippy::result_large_err` is allowed by design — do not "fix" it.
- The two paradigms (programmatic `Validate<T>` + declarative `Rule` enum) are deliberately separate but bridged: `Rule` implements `Validate<Value>` for combinator composition.

## Key Decisions
- `Validated<T>` is a proof token: validate once, carry the proof through the system. Don't re-validate the same value.
- `Rule` enum is JSON-serializable — enables storing validation rules in the database or config files. `#[non_exhaustive]`.
- `Rule` has shorthand constructors (`Rule::min_length(5)`, `Rule::pattern(...)`, `Rule::email()`, `Rule::url()`, `Rule::all/any/not(...)`, etc.) and `.with_message()` fluent builder. Struct literals still work.
- `collect_json_fields` combinator — validates multiple JSON fields, collects ALL errors (non-short-circuiting).
- `Rule::min_value_f64`/`max_value_f64` return `Option<Self>` — `None` for NaN/Infinity. No panics.
- `validators::try_size_range(min, max)` is the non-panicking constructor for collection size ranges and returns `ValidationError { code: "invalid_range" }` when bounds are inverted.
- `validate_rules()` returns `Result<(), ValidationErrors>` (not `Vec<ValidationError>`).
- `min_i64/max_i64/in_range_i64` + `f64` variants — turbofish-free convenience for JSON numbers.
- `ExecutionMode` controls which rule categories run (`StaticOnly` skips deferred/async rules for fast paths).
- Deep combinator types (`And<Or<Not<...>, ...>, ...>`) are inherent to the design. `clippy::type_complexity` is allowed.

## Traps
- `AnyValidator<T>` is type-erased (dyn-compatible). Combinators are concrete types. They are not interchangeable.
- `validate_rules()` takes a `serde_json::Value` — not a typed struct. Returns `ValidationErrors`, not `Vec`.
- No regex cache — patterns compiled inline each call. Fast enough for schema-validation (not a hot path).
- Predicate comparisons (`Gt`/`Lt`/`Gte`/`Lte`) use precision-safe `json_number_cmp` (i64→u64→f64 fallback).

## Derive Macro Architecture (nebula-validator-macros)
- 3-phase pipeline: `parse.rs` (attrs → IR) → `emit.rs` (IR → TokenStream). `validator.rs` is a 28-line entry point.
- IR types in `model.rs`: `ValidatorInput`, `FieldDef`, `Rule` enum (19 variants), `EachRules`, `StringFormat`, `StringFactoryKind`.
- Option-wrapping centralized in `emit::wrap_option()` — binds `value` for both Option and non-Option fields.
- Message override centralized in `emit::wrap_message()` — the before/after/last_mut pattern in one place.
- `nested` and `custom` validators use inline message override (mut e pattern) — they need to modify the error before adding.
- `each()` rules reuse the same `Rule` enum as field-level rules.
- `#[validate(required)]` is strict: only valid on `Option<T>` fields. Using it on non-optional fields is a parse-time compile error, not a silent no-op.
- Derive DSL now supports compositional sugar: `all(...)` and `any(...)` in addition to `using = ...`.
- Inside `each(...)`, nested composition supports standard call form: `each(any(v1, v2))`, `each(all(v1, v2))`.
- Derive DSL now also supports canonical call-style aliases inspired by validator/garde: `min(...)`, `max(...)`, `length(...)`, `range(...)`, and `inner(...)`.
- `inner(...)` is the canonical public alias for per-element container validation; `each(...)` remains supported for compatibility.
- Canonical call-style derive rules are parse-time type-checked too: e.g. `email()`/`prefix()` require string-like fields, `required()` requires `Option<T>`, and `inner(...)`/`each(...)` are constrained to vector-like containers.
- `validation_codegen.rs` helpers in macro-support are still used by `config-macros`; validator-macros uses its own IR.

## Relations
- No nebula deps. Used by nebula-parameter, nebula-macros, nebula-action, nebula-parameter.

<!-- reviewed: 2026-04-01 — Added strict canonical min/max/length/range/inner derive DSL, using/all/any composition paths, standard each(any/all(...)) syntax, try_size_range + strict required enforcement -->
