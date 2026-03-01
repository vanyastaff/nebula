# Architecture

## Problem Statement

- Business problem: multiple Nebula crates need consistent, reusable, machine-readable validation.
- Technical problem: avoid ad-hoc validation logic and fragmented error semantics across API/runtime/plugin boundaries.

## Current Architecture

### Module Map

- `foundation/`
  - `traits.rs` — `Validate<T>`, `ValidateExt<T>`, `Validatable`, inline combinators (`And`, `Or`, `Not`, `When`)
  - `error.rs` — `ValidationError` (80 bytes, `Cow`-based), `ValidationErrors`, `ErrorSeverity`, `codes` constants, sensitive-param redaction
  - `any.rs` — `AnyValidator<T>` (type erasure via internal `ErasedValidator` trait)
  - `validatable.rs` — blanket `Validatable` impl, `AsValidatable<T>` bridge for JSON
  - `category.rs`, `context.rs` — internal category/context helpers
- `validators/`
  - `length.rs` — `NotEmpty`, `MinLength`, `MaxLength`, `ExactLength`, `LengthRange` + `LengthMode` (Chars/Bytes)
  - `pattern.rs` — `Contains`, `StartsWith`, `EndsWith`, `Alphanumeric`, `Alphabetic`, `Numeric`, `Lowercase`, `Uppercase`
  - `content.rs` — `MatchesRegex`, `Email`, `Url` (all via `LazyLock<Regex>`)
  - `range.rs` — `Min<T>`, `Max<T>`, `InRange<T>`, `GreaterThan<T>`, `LessThan<T>`, `ExclusiveRange<T>`
  - `size.rs` — `MinSize<T>`, `MaxSize<T>`, `ExactSize<T>`, `NotEmptyCollection<T>`, `SizeRange<T>` (all `for [T]`)
  - `boolean.rs` — `IsTrue`, `IsFalse` + `IS_TRUE`/`IS_FALSE` const instances
  - `nullable.rs` — `Required<T>` / `NotNull<T>` (phantom-data wrapper, `for Option<T>`)
  - `network.rs` — `Ipv4`, `Ipv6`, `IpAddr` (via `std::net`), `Hostname` (RFC 1123)
  - `temporal.rs` — `Date`, `Time`, `DateTime` (RFC 3339), `Uuid` (RFC 4122) — pure-Rust, no chrono/uuid deps
- `combinators/`
  - `and.rs`, `or.rs`, `not.rs`, `when.rs`, `unless.rs` — logical
  - `optional.rs` — `Optional<V>` (None always passes)
  - `each.rs` — `Each<V>` (slice, collect-all or fail-fast)
  - `factories.rs` — `AllOf<V>`, `AnyOf<V>` (homogeneous `Vec<V>`)
  - `field.rs` — `Field<T,U,V,F>`, `FieldError`, `MultiField<T>`, `FieldValidateExt`
  - `json_field.rs` — `JsonField<V,I>` (RFC 6901 JSON Pointer, `AsValidatable<I>` bridge)
  - `cached.rs`, `lazy.rs` — memoization, deferred construction
  - `message.rs` — `WithMessage<V>`, `WithCode<V>` (override error text/code)
  - `nested.rs` — `NestedValidate`, `OptionalNested`, `CollectionNested` (experimental)
  - `error.rs` — `CombinatorError<E>` (internal combinator error envelope)
- `macros.rs` — `validator!` macro (5 entry-point arms, 5 tail-parser arms, 7 code-generator `@`-helpers), `compose!`, `any_of!`
- `prelude.rs` — single-import convenience

### Data / Control Flow

```
user input ──► typed validator chain ──► Result<(), ValidationError>
                      │
                      ▼
                ValidateExt combinators (.and / .or / .not / .when)
                      │
                      ▼
             ValidationErrors (collect-all)
                      │
                      └──► into_single_error() ──► single nested ValidationError
```

- Short-circuit: `And` returns first failure without evaluating right-hand side.
- Collect-all: `AllOf`, `MultiField`, `Each`, `ValidationErrors::add` gather all failures.
- Fail-fast: `Each::fail_fast` stops at the first failing element.

### Key Internal Invariants

- **`ValidationError` memory layout:** 80 bytes on the stack. Rarely-used extras
  (`params`, `nested`, `severity`, `help`) are heap-allocated inside `Option<Box<ErrorExtras>>`
  (lazy boxing). `Cow<'static, str>` enables zero-allocation static string codes/messages.
  `ErrorExtras::params` uses `SmallVec<[(Cow,Cow); 2]>` to inline up to 2 params without heap.
- **`AnyValidator<T>`:** wraps `Box<dyn ErasedValidator<T>>` where `ErasedValidator` is an
  internal sealed trait. Requires `V: Clone + Send + Sync + 'static`.
- **Sensitive param redaction:** enforced in `with_param()`; keys matching
  `password`, `secret`, `token`, `api_key`, `apikey`, `credential` are stored as `"[REDACTED]"`.
- **`LengthMode`:** two measurement paths — `Chars` (default, `str.chars().count()`) and
  `Bytes` (`str.len()`). Unicode correctness vs ASCII-performance trade-off is caller's choice.
- **`JsonField` type bridge:** uses `AsValidatable<I>` to convert `serde_json::Value` into
  `&I` (e.g., `&str`, `&i64`). Type mismatch → error code `type_mismatch` with the pointer
  path set in `error.field`.
- **`Field` path composition:** when `Field::named("parent")` wraps a validator that produces
  `error.field = Some("sub")`, the final error field becomes `"parent.sub"` (dot notation).
- **`validator!` macro architecture:** three-layer design — (1) 5 entry-point arms parse
  user syntax and normalize into a canonical KV form; (2) 5 tail-parser arms detect
  constructor variant (auto/custom/fallible) and presence of factory fn; (3) 7 `@`-helper
  arms each generate exactly one piece of code with zero duplication.

### Known Bottlenecks

- Deeply nested generic combinator types: compile time and error diagnostics.
  Use `AnyValidator<T>` to break inference chains in large validators.
- `Email`/`Url` validators clone `Regex` on construction (shared via `LazyLock`); the
  clone itself is cheap (Arc bump) but allocation still happens.
- `JsonField` with deeply nested JSON objects or large payloads can dominate runtime CPU;
  prefer validated structs at deserialization time for hot paths.

## Target Architecture

- target module map:
  - preserve current split; do not over-fragment into many micro-modules
  - add stricter docs and compatibility contracts rather than structural churn
- public contract boundaries:
  - `Validate<T>`/`ValidateExt` as primary stable API
  - `ValidationError` code/field-path schema as cross-crate contract
- internal invariants:
  - validators are side-effect free
  - combinators preserve deterministic evaluation semantics
  - error codes remain stable unless major migration is declared

## Design Reasoning

- trade-off 1: static typing vs dynamic flexibility
  - chosen: static first (`Validate<T>`), dynamic bridge via `AnyValidator` and `validate_any`
- trade-off 2: rich errors vs allocation cost
  - chosen: rich `ValidationError` with boxed extras and smallvec optimization
- rejected alternatives:
  - stringly-typed validator registry as primary model (reject: unsafe and hard to refactor)

## Comparative Analysis

Sources considered: n8n, Node-RED, Activepieces/Activeflow, Temporal/Airflow style systems.

- Adopt:
  - Node-based platforms’ need for human-readable validation errors and field-path mapping.
  - Workflow platforms’ need for deterministic, replay-safe validation behavior.
- Reject:
  - JS-style runtime schema-only validation as sole source of truth (too weak for Rust compile-time guarantees).
  - heavily implicit coercion behavior (causes hidden bugs in automation flows).
- Defer:
  - declarative schema DSL on top of current typed API, if demand from plugin SDK grows.

## Breaking Changes (if any)

- currently none required.
- potential future break candidates:
  - formalized `FieldPath` type in place of plain string paths
  - stricter error code registry enforcement

## Open Questions

- should `collect-all` vs `fail-fast` be first-class in combinator API or policy wrapper?
- should validator catalogs be exported as machine-readable metadata for UI generation?
