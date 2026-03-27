# nebula-validator
Composable, type-safe validation framework — two paradigms: programmatic combinators and JSON-serializable declarative Rules.

## Invariants
- `ValidationError` is capped at 80 bytes (`Cow`-based). Intentional: boxing adds indirection to every validation call. `clippy::result_large_err` is allowed by design — do not "fix" it.
- The two paradigms (programmatic `Validate<T>` + declarative `Rule` enum) are deliberately separate but bridged: `Rule` implements `Validate<Value>` for combinator composition.

## Key Decisions
- `Validated<T>` is a proof token: validate once, carry the proof through the system. Don't re-validate the same value.
- `Rule` enum is JSON-serializable — enables storing validation rules in the database or config files. `#[non_exhaustive]`.
- `Rule` has shorthand constructors (`Rule::min_length(5)`, `Rule::pattern(...)`, `Rule::all/any/not(...)`, etc.) and `.with_message()` fluent builder. Struct literals still work.
- `Rule::min_value_f64`/`max_value_f64` return `Option<Self>` — `None` for NaN/Infinity. No panics.
- `validate_rules()` returns `Result<(), ValidationErrors>` (not `Vec<ValidationError>`).
- `min_i64/max_i64/in_range_i64` + `f64` variants — turbofish-free convenience for JSON numbers.
- `ExecutionMode` controls which rule categories run (`StaticOnly` skips deferred/async rules for fast paths).
- Deep combinator types (`And<Or<Not<...>, ...>, ...>`) are inherent to the design. `clippy::type_complexity` is allowed.

## Traps
- `AnyValidator<T>` is type-erased (dyn-compatible). Combinators are concrete types. They are not interchangeable.
- `validate_rules()` takes a `serde_json::Value` — not a typed struct. Returns `ValidationErrors`, not `Vec`.
- No regex cache — patterns compiled inline each call. Fast enough for schema-validation (not a hot path).
- Predicate comparisons (`Gt`/`Lt`/`Gte`/`Lte`) use precision-safe `json_number_cmp` (i64→u64→f64 fallback).

## Relations
- No nebula deps. Used by nebula-parameter, nebula-macros, nebula-action, nebula-parameter.

<!-- reviewed: 2026-03-26 (phase 2: convenience numerics, Rule Validate bridge, ValidationErrors return type) -->
