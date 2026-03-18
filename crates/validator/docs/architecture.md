# nebula-validator — Architecture

## Problem Statement

Workflow nodes, REST API extractors, plugin parameters, and configuration loaders all need to
validate untrusted input. Without a shared validation layer each boundary would implement its
own error format, its own composition model, and its own notion of a "validated value" — leading
to four diverging error contracts that operator tooling cannot reliably parse.

`nebula-validator` provides one validation contract for the entire platform: a composable trait
hierarchy, a structured error type, and a serializable `Rule` enum. Every layer that validates
input uses the same `code`/`message`/`field` triple in its error responses.

---

## Key Design Decisions

### 1. Type-bound validation as the primary API

Every validator is generically typed to its input: `Validate<str>`, `Validate<i32>`,
`Validate<Option<T>>`. A validator that accepts `str` cannot be applied to an `i32` without
an explicit bridge. This means that invalid validator/input pairings fail at compile time, not
at runtime, and that refactoring a data structure immediately propagates type errors to all
validation sites.

Dynamic dispatch is available through `AnyValidator<T>` for cases where validators of different
concrete types must live in the same collection, but it is opt-in rather than the default path.

### 2. Combinator-first composition

There is no "validator builder" or "validation schema" object. Rules are composed using
first-class combinator types — `And<L, R>`, `Or<L, R>`, `Not<V>`, `When<V, C>` — that the
compiler inlines and monomorphizes away. A chain like `min_length(3).and(max_length(20)).and(email())`
produces a concrete type `And<And<MinLength, MaxLength>, Email>` with zero runtime overhead.

The consequence is that the type system encodes the validation graph. Long chains produce
complex generic types, but those types are erased by the optimizer at the call site.

### 3. Structured errors with nested context

`ValidationError` is an 80-byte struct (fits in a cache line) that carries:
- `code` — machine-readable identifier, stable across versions
- `message` — human-readable description
- `field` — RFC 6901 JSON Pointer path to the failing field
- `params` — key-value diagnostic context (sensitive keys auto-redacted)
- `severity` — `Error / Warning / Info`
- `help` — optional remediation hint
- `nested` — child errors for hierarchical failures (struct fields, collection elements)

The `Cow<'static, str>` fields allow static strings to flow through without allocation.
Dynamic strings (e.g., from `format!`) are heap-allocated only when necessary.

### 4. RFC 6901 pointer canonicalization

All field paths are normalized to RFC 6901 JSON Pointer format at the `ValidationError`
boundary. `with_field("user.email")` and `with_field("user[0].name")` are both converted to
`/user/email` and `/user/0/name`. Consumers receive one deterministic format; per-consumer
path-parsing heuristics are eliminated.

The `field` key in the serialized envelope emits both the raw input and the pointer for
backwards compatibility. `field_pointer()` is the canonical accessor.

### 5. Proof tokens

`Validated<T>` is a zero-cost newtype that can only be constructed through a validated code
path (`Validate::validate_into`, `Validated::new`). Functions that require pre-validated data
declare `fn process(name: Validated<String>)`. The compiler rejects a bare `String` without an
explicit validation step, making trust boundaries visible in the type system.

`Deserialize` is intentionally not derived for `Validated<T>`. Deserialized data is untrusted
and must always be re-validated before the proof token is issued.

### 6. Declarative `Rule` enum

The `Rule` enum is a JSON-serializable representation of the same validation logic exposed by
the programmatic API. It covers:
- **Value rules** — `MinLength`, `MaxLength`, `Pattern`, `Min`, `Max`, …
- **Context predicates** — `Eq`, `Ne`, `In`, `IsNull`, `IsPresent`, … (test sibling fields)
- **Logical combinators** — `All`, `Any`, `Not`

Rules are evaluated by `validate_rules(value, &rules, ExecutionMode)`. `ExecutionMode` controls
which categories run: `StaticOnly` skips deferred/async rules, making evaluation synchronous
and allocation-minimal for hot paths.

### 7. Error code registry governance

A machine-readable registry at `tests/fixtures/compat/error_registry_v1.json` lists every
error code with its stability level (`stable` / `deprecated`). Contract tests in
`tests/contract/` verify that:

- No stable code is removed or semantically changed in a minor release.
- Every new validator registers its code in the registry.
- Behavior-significant changes are documented in [`migration.md`](migration.md) before release.

Minor releases are **additive only** for the error code catalog.

### 8. Operational error separation

`ValidatorError` separates `InvalidConfig` (misconfigured validator, e.g., `min > max` in
`LengthRange`) from `ValidationFailed` (bad input). The lower-level `validate()` method still
returns `Result<(), ValidationError>` for callers that only need pass/fail. `validate_into` and
`Validated::new` return `ValidatorResult<T>` to expose the richer variant.

---

## Module Map

```
nebula-validator/src/
│
│  ── Public API ────────────────────────────────────────────────────────────
│
├── foundation/
│   ├── traits.rs      Validate<T>, ValidateExt<T>, Validatable.
│   │                  ValidateExt blanket-impl provides .and(), .or(), .not(), .when().
│   │                  for_field() / for_field_unnamed() extension methods.
│   │
│   ├── any.rs         AnyValidator<T>.
│   │                  Type-erased validator for heterogeneous collections.
│   │                  Requires V: Validate<T> + Clone + Send + Sync + 'static.
│   │                  ~2–5 ns overhead per call from the vtable.
│   │
│   ├── error.rs       ValidationError (80 bytes, Cow-based).
│   │                  ValidationErrors (Vec<ValidationError> aggregate).
│   │                  ErrorSeverity (Error / Warning / Info).
│   │                  Convenience constructors: required, min_length, max_length,
│   │                  invalid_format, type_mismatch, out_of_range, custom, …
│   │
│   ├── field_path.rs  FieldPath — validated RFC 6901 JSON Pointer.
│   │                  Typed path operations: segments(), depth(), parent(),
│   │                  append(), push(), last_segment().
│   │
│   ├── validatable.rs SelfValidating trait — check() method for self-validating types.
│   ├── context.rs     ValidationContext — carries execution metadata into validators.
│   ├── category.rs    ErrorCategory — classifies errors for cross-crate contracts.
│   └── mod.rs         Re-exports the public foundation surface.
│
├── validators/
│   ├── length.rs      MinLength, MaxLength, ExactLength, LengthRange, NotEmpty.
│   │                  Default mode: Unicode char count. Byte-mode variants for ASCII paths.
│   │
│   ├── pattern.rs     Contains, StartsWith, EndsWith, Alphanumeric, Alphabetic,
│   │                  Numeric, Lowercase, Uppercase.
│   │
│   ├── content.rs     MatchesRegex (fallible construction), Email, Url.
│   │                  Email and Url use LazyLock<Regex> (compiled once, shared).
│   │
│   ├── range.rs       Min, Max, InRange (inclusive), GreaterThan, LessThan,
│   │                  ExclusiveRange.  Works for any T: PartialOrd + Display + Copy.
│   │
│   ├── size.rs        MinSize, MaxSize, ExactSize, SizeRange, NotEmptyCollection.
│   │                  Validates &[T] — works for Vec, arrays, slices.
│   │
│   ├── boolean.rs     IsTrue, IsFalse. Const variants IS_TRUE, IS_FALSE for zero-cost
│   │                  use in hot paths.
│   │
│   ├── nullable.rs    Required<T>, NotNull<T>. Validates Option<T> presence.
│   ├── network.rs     Ipv4, Ipv6, IpAddr, Hostname. Uses std::net; no external deps.
│   └── temporal.rs    Date, Time, DateTime (RFC 3339), Uuid. Pure Rust; no chrono.
│
├── combinators/
│   ├── and.rs         And<L, R> — both must pass; short-circuits on first failure.
│   ├── or.rs          Or<L, R>  — either must pass; nests both errors on failure.
│   ├── not.rs         Not<V>    — inverts result; code `not_failed`.
│   ├── when.rs        When<V, C> — skips inner validator when predicate returns false.
│   ├── unless.rs      Unless<V, C> — equivalent to when(!condition).
│   ├── optional.rs    Optional<V> — None always Ok; Some(x) delegates to inner V.
│   ├── each.rs        Each<V> — validates every slice element; collects all errors.
│   │                  Fail-fast mode: stops at first failing element.
│   ├── field.rs       Field<T, U, V, F> — applies V to a field extracted by F.
│   │                  named_field() adds dot-notation path to error.
│   ├── nested.rs      MultiField<T> — validates multiple fields; aggregates errors.
│   │                  CollectionNested<T> — validates collection elements as structs.
│   ├── json_field.rs  JsonField<V, I> — validates a JSON Pointer path in a Value.
│   │                  Required and optional variants.
│   ├── factories.rs   AllOf<V> — all validators in a Vec must pass.
│   │                  AnyOf<V> — at least one must pass.
│   ├── message.rs     WithMessage<V>, WithCode<V> — override error output.
│   ├── cached.rs      Cached<V> — memoizes results by input hash; RwLock-protected.
│   ├── lazy.rs        Lazy<V> — defers construction until first call.
│   └── error.rs       Combinator-level error helpers.
│
├── rule.rs            Rule enum — serializable declarative validation rules.
├── engine.rs          validate_rules(), ExecutionMode (StaticOnly / Deferred / Full).
├── proof.rs           Validated<T> proof token.
├── error.rs           ValidatorError, ValidatorResult<T>.
├── macros.rs          validator!, compose!, any_of! (private; expanded at call site).
└── prelude.rs         Single-import convenience re-export.
```

---

## Data Flow

### Programmatic validation

```
Caller
  │  validator.validate(input)
  ▼
Validate<T>::validate
  │  1. Run rule predicate (self.rule(input) → bool)
  │     └─ true  → Ok(())
  │     └─ false → build ValidationError (self.error(input))
  │
  │  For combinators:
  │  And<L, R>::validate
  │    1. L.validate(input) — if Err, return immediately (short-circuit)
  │    2. R.validate(input)
  │
  │  Or<L, R>::validate
  │    1. L.validate(input) — if Ok, return Ok
  │    2. R.validate(input) — if Ok, return Ok
  │    3. Both failed → ValidationError { code: "or_failed", nested: [l_err, r_err] }
  │
  │  Each<V>::validate (on &[T])
  │    1. Iterate elements, call V.validate(elem) for each
  │    2. Collect all Err results with their indices
  │    3. Return aggregated error if any failures
  ▼
Result<(), ValidationError>
```

### Proof token path

```
Caller
  │  validator.validate_into(value: V)
  │    where V: Borrow<T>, Self: Validate<T>
  ▼
Validate<T>::validate(&value.borrow())
  │  Ok(()) → Validated::new_unchecked(value)  → ValidatorResult::Ok(Validated<V>)
  │  Err(e)  → ValidatorResult::Err(ValidatorError::ValidationFailed(e))
  ▼
Validated<V>   (or error propagated via ?)
```

### Declarative rule evaluation

```
Caller
  │  validate_rules(value, &rules, ExecutionMode::StaticOnly)
  ▼
Engine::evaluate
  │  1. Filter rules by ExecutionMode (skip Deferred rules in StaticOnly)
  │  2. For each rule:
  │     Rule::MinLength { min, .. } → MinLength { min }.validate(value.as_str()?)
  │     Rule::All { rules }         → recursive evaluate on each sub-rule
  │     Rule::Eq { field, value }   → lookup field in ValidationContext, compare
  │  3. Aggregate errors into ValidationErrors
  │  4. .into_result(()) → Ok if empty, Err(ValidationErrors) if any
  ▼
Result<(), ValidationErrors>
```

---

## Test Strategy

The test suite is structured in three layers:

**Unit tests** live in `src/` alongside each module. They verify per-validator correctness:
boundary values, Unicode edge cases, byte-mode vs char-mode agreement, error codes, and
error parameter contents. Every built-in validator has at least a happy-path and a
boundary-failure test.

**Integration tests** in `tests/` exercise combined validator pipelines, field combinators,
struct validation, and JSON path validation against realistic inputs. Property tests in
`tests/property_tests.rs` use `proptest` to verify combinator laws hold over arbitrary inputs.

**Contract tests** in `tests/contract/` are the stability gate for downstream consumers.
They verify:
- `compatibility_fixtures_test.rs` — baseline error codes and field paths match the registry fixture
- `combinator_semantics_contract_test.rs` — `And` short-circuits, `Or` aggregates, `Not` inverts
- `typed_dynamic_equivalence_test.rs` — `AnyValidator` produces identical results to the typed path
- `error_envelope_schema_test.rs` — serialized error JSON matches the documented envelope schema
- `adversarial_inputs_test.rs` — long strings, null bytes, deep nesting, empty inputs
- `safe_diagnostics_test.rs` — sensitive params are redacted, nested counts are correct
- `error_tree_bounds_test.rs` — `total_error_count` and `flatten` are consistent
- `governance_policy_test.rs` — every error code in the codebase is registered in the registry
- `migration_requirements_test.rs` — behavior-significant changes have a migration mapping

Contract tests have **zero flakiness tolerance**. A flaky contract test is treated as a
broken contract, not a test infrastructure issue.

Benchmarks in `benches/` cover the hot paths: string validators, combinator chains,
error construction, and cache hit/miss. Regressions beyond the agreed threshold fail CI.

---

## Invariants

1. **Error codes are stable across minor releases.**
   A code registered as `stable` in `error_registry_v1.json` will not be renamed, removed,
   or have its semantics changed in a minor release. Violations are caught by contract tests.

2. **Field paths are always RFC 6901 JSON Pointers.**
   `ValidationError::with_field(path)` normalizes any input format. Consumers can rely on
   `/user/email` format without per-consumer normalization logic.

3. **Sensitive params are always redacted.**
   Keys matching `password`, `secret`, `token`, `api_key`, `apikey`, or `credential` are
   stored as `"[REDACTED]"` in `ValidationError::params`. This happens at construction time,
   before errors reach any logging or serialization layer.

4. **`Validated<T>` cannot be constructed without validation.**
   There is no `impl Deserialize for Validated<T>`. The only construction paths are
   `validate_into` and `Validated::new`. The internal `new_unchecked` is `pub(crate)`
   and not accessible to downstream consumers.

5. **No unsafe code.**
   The crate is compiled with `#![forbid(unsafe_code)]`.

6. **No upward nebula dependencies.**
   `nebula-validator` has no dependencies on other `nebula-*` crates. It is a foundation
   crate importable at any layer without creating dependency cycles.
