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
- `validators::try_size_range(min, max)` is the non-panicking constructor for collection size ranges and returns `ValidationError { code: "invalid_range" }` when bounds are inverted. `size_range()` uses `debug_assert!` (not `assert!`).
- `validators::try_in_range(min, max)` / `try_exclusive_range(min, max)` — fallible constructors for numeric ranges. The infallible `in_range()`/`exclusive_range()` silently create degenerate validators on inverted bounds.
- `Rule::try_pattern(regex)` validates the regex at construction time (returns `Option<Self>`). `Rule::pattern()` defers validation to `validate_value()` time.
- `validate_rules()` returns `Result<(), ValidationErrors>` (not `Vec<ValidationError>`).
- `min_i64/max_i64/in_range_i64` + `f64` variants — turbofish-free convenience for JSON numbers.
- `ExecutionMode` controls which rule categories run (`StaticOnly` skips deferred/async rules for fast paths).
- Deep combinator types (`And<Or<Not<...>, ...>, ...>`) are inherent to the design. `clippy::type_complexity` is allowed.

## Traps
- `AnyValidator<T>` is type-erased (dyn-compatible). Combinators are concrete types. They are not interchangeable.
- `validate_rules()` takes a `serde_json::Value` — not a typed struct. Returns `ValidationErrors`, not `Vec`.
- No regex cache — patterns compiled inline each call. Fast enough for schema-validation (not a hot path).
- Email/URL regex patterns are shared constants (`EMAIL_PATTERN`, `URL_PATTERN` in `content.rs`) — used by both programmatic validators and `Rule::Email`/`Rule::Url`.
- `Rule::Matches` with an invalid regex pattern `debug_assert!`s in debug builds and silently returns `false` in release. `Rule::with_message()` is a no-op on predicates and combinators (documented).
- f64 convenience functions (`min_f64`, `max_f64`, `in_range_f64`) `debug_assert!` against NaN bounds.
- Predicate comparisons (`Gt`/`Lt`/`Gte`/`Lte`) use precision-safe `json_number_cmp` (i64→u64→f64 fallback).

## Derive Macro (nebula-validator-macros)
- 3-phase pipeline: `parse.rs` (attrs → IR in `model.rs`) → `emit.rs` (IR → TokenStream). `validator.rs` is a 28-line entry.
- `#[validate(required)]` is parse-time strict: only valid on `Option<T>` fields, otherwise compile error (not silent no-op).
- Derive DSL supports both the `using = ...` form and compositional `all(...)`/`any(...)`, plus call-style aliases inspired by validator/garde: `min(...)`, `max(...)`, `length(...)`, `range(...)`, `inner(...)`. `inner(...)` is the canonical per-element container validator; `each(...)` kept for compatibility.
- Call-style rules are parse-time type-checked: `email()`/`prefix()` require string-like fields, `required()` requires `Option<T>`, `inner(...)`/`each(...)` are constrained to vector-like containers.

## Relations
- No nebula deps. Used by nebula-parameter, nebula-macros, nebula-action, nebula-parameter.
