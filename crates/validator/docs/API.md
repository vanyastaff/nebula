# API

## Public Surface

Stability tiers:

- **Stable:** `foundation::Validate<T>`, `foundation::ValidateExt<T>`, `foundation::Validatable`,
  `foundation::ValidationError`, `foundation::ValidationErrors`, `foundation::AnyValidator<T>`,
  `foundation::ErrorSeverity`, `proof::Validated<T>`, `error::ValidatorError`,
  all built-in validators from `validators::*`,
  core combinators from `combinators::*`, `validator!` / `compose!` / `any_of!` macros.
- **Experimental:** advanced combinator internals (`MultiField`, `NestedValidate`,
  `CollectionNested`); treat as non-contract.
- **Internal / hidden:** `ErasedValidator` trait, `AsValidatable` conversion bridge,
  macro plumbing `@`-arms.

---

## Core Traits

### `Validate<T>` — `foundation::Validate`

```rust
pub trait Validate<T: ?Sized> {
    fn validate(&self, input: &T) -> Result<(), ValidationError>;

    // Bridge: validate a serde_json::Value against validators that expect T
    fn validate_any<U>(&self, input: &U) -> Result<(), ValidationError>
    where
        U: AsValidatable<T>;

    // Validate and wrap in a proof token
    fn validate_into<V>(&self, value: V) -> ValidatorResult<Validated<V>>
    where
        V: Borrow<T>,
        Self: Sized;
}
```

- Blanket implementation is provided for all types implementing `Validate<T>`.
- `validate_any` lets string validators be called with `&serde_json::Value`
  when `Value: AsValidatable<str>`.
- `validate_into` validates the value and wraps it in a `Validated<V>` proof token,
  returning `Err(ValidatorError::ValidationFailed(..))` on failure.

### `ValidateExt<T>` — combinator builder

Blanket impl on every `Validate<T>`:

| Method | Returns | Behaviour |
|--------|---------|-----------|
| `.and(v)` | `And<Self, V>` | Both must pass; short-circuits on first failure |
| `.or(v)` | `Or<Self, V>` | At least one must pass; error code `or_failed` |
| `.not()` | `Not<Self>` | Inverts result; error code `not_failed` |
| `.when(cond: impl Fn(&T) -> bool)` | `When<Self, C>` | Skips validation when `cond` returns `false` |

### `Validatable` — extension-method style

Blanket impl on all types:

```rust
value.validate_with(&validator) -> Result<&Self, ValidationError>
```

Returns `&self` on success, allowing chaining.

---

## `ValidationError`

80-byte struct. Uses `Cow<'static, str>` for zero-allocation static strings.

### Fields (public)

| Field | Type | Notes |
|-------|------|-------|
| `code` | `Cow<'static, str>` | Machine-readable error code |
| `message` | `Cow<'static, str>` | Human-readable description |
| `field` | `Option<Cow<'static, str>>` | Canonical field path in JSON Pointer (RFC 6901) |

### Constructor

```rust
ValidationError::new(code: impl Into<Cow<'static, str>>, message: impl Into<Cow<'static, str>>)
```

`with_field("a.b[0]")` input is normalized to `"/a/b/0"`.

### Convenience constructors

| Constructor | Error code |
|-------------|-----------|
| `ValidationError::required(field)` | `required` |
| `ValidationError::min_length(field, min, actual)` | `min_length` |
| `ValidationError::max_length(field, max, actual)` | `max_length` |
| `ValidationError::invalid_format(field, expected)` | `invalid_format` |
| `ValidationError::type_mismatch(field, expected, actual)` | `type_mismatch` |
| `ValidationError::out_of_range(field, min, max, actual)` | `out_of_range` |
| `ValidationError::exact_length(field, expected, actual)` | `exact_length` |
| `ValidationError::length_range(field, min, max, actual)` | `length_range` |
| `ValidationError::custom(message)` | `custom` |

### Builder methods

```rust
.with_field(field: impl Into<Cow<'static, str>>)   -> ValidationError
.with_pointer(pointer: impl Into<Cow<'static, str>>) -> ValidationError
.with_param(
    key: impl Into<Cow<'static, str>>,
    value: impl Into<Cow<'static, str>>,
) -> ValidationError                                 // sensitive keys redacted
.with_nested(errors: Vec<ValidationError>)          -> ValidationError
.with_nested_error(e: ValidationError)              -> ValidationError
.with_severity(s: ErrorSeverity)                    -> ValidationError
.with_help(text: impl Into<Cow<'static, str>>)      -> ValidationError
```

**Sensitive param redaction:** keys matching `password`, `secret`, `token`, `api_key`,
`apikey`, or `credential` are stored as `"[REDACTED]"`.

### Accessor methods

```rust
.param(key: &str) -> Option<&str>
.params() -> &[(Cow<str>, Cow<str>)]
.nested() -> &[ValidationError]
.has_nested() -> bool
.severity() -> ErrorSeverity
.help() -> Option<&str>
.field_pointer() -> Option<Cow<str>>
.total_error_count() -> usize        // 1 + recursive count of all nested
.flatten() -> Vec<&ValidationError>  // depth-first, all nested
.to_json_value() -> serde_json::Value
```

`to_json_value()` emits both `field` and `pointer` keys for compatibility.

### Display format

```
[field] code: message (params: [k=v, ...])
  Help: ...
  Nested errors:
    1. ...
```

---

## `ValidationErrors`

Aggregate wrapper: `Vec<ValidationError>`.

```rust
ValidationErrors::new() -> Self
.add(e: ValidationError)
.extend(iter: impl IntoIterator<Item = ValidationError>)
.has_errors() -> bool
.into_single_error(msg: &str) -> ValidationError  // wraps all as nested
.into_result(ok_value: T) -> Result<T, ValidationErrors>  // Ok(ok_value) if empty
```

Implements `FromIterator<ValidationError>` and `IntoIterator`.

---

## `ErrorSeverity`

```rust
pub enum ErrorSeverity { Error, Warning, Info }
```

Default is `Error`. Set via `.with_severity()`.

---

## `AnyValidator<T>` — type erasure

```rust
AnyValidator::<T>::new(v: V) -> AnyValidator<T>
// requires V: Validate<T> + Clone + Send + Sync + 'static
```

Enables storing validators of different concrete types in the same `Vec`.
Uses dynamic dispatch (~2–5 ns overhead per call). Implements `Clone`.

```rust
// Heterogeneous collection:
let validators: Vec<AnyValidator<str>> = vec![
    AnyValidator::new(min_length(3)),
    AnyValidator::new(email()),
];
```

---

## `Validated<T>` — proof token

```rust
pub struct Validated<T> { /* private */ }
```

Zero-cost wrapper certifying the inner value passed validation. Cannot be constructed
without going through a validated code path.

### Construction

| Method | Signature | Notes |
|--------|-----------|-------|
| `Validated::new` | `fn new<V, U: ?Sized>(value: T, validator: &V) -> ValidatorResult<Self>` | `V: Validate<U>`, `T: Borrow<U>` |
| `validate_into` | `fn validate_into<V>(&self, value: V) -> ValidatorResult<Validated<V>>` | on `Validate<T>` trait |
| `new_unchecked` | `fn new_unchecked(value: T) -> Self` | escape hatch, use sparingly |

### Access

| Method | Returns | Notes |
|--------|---------|-------|
| `inner()` | `&T` | reference access |
| `into_inner()` | `T` | consumes wrapper |
| `Deref` / `AsRef` / `Borrow` | `&T` | transparent access |
| `map(f)` | `Validated<U>` | transform inner value |

### Traits

Implements: `Clone`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Hash`,
`Debug`, `Display`, `Serialize` (transparent). `Deserialize` is intentionally
omitted — deserialized data must be re-validated.

---

## `ValidatorError` — crate-level error

```rust
#[non_exhaustive]
pub enum ValidatorError {
    InvalidConfig { message: Cow<'static, str> },
    ValidationFailed(#[from] ValidationError),
}
pub type ValidatorResult<T> = Result<T, ValidatorError>;
```

Operational error type separating configuration errors from validation failures.
Used by `validate_into` and `Validated::new`.

---

## Built-in Validators

All are in `validators::*` and re-exported by `prelude::*`.

### Length — `validators::length` (`for str`)

Default length mode: **Unicode chars** (`char::count()`).
Byte mode variants available for ASCII-only / performance paths.

| Type | Factory fn | Infallible? | Error code |
|------|-----------|-------------|------------|
| `NotEmpty` | `not_empty()` | yes | `not_empty` |
| `MinLength { min, mode }` | `min_length(min)` | yes | `min_length` |
| `MaxLength { max, mode }` | `max_length(max)` | yes | `max_length` |
| `ExactLength { length, mode }` | `exact_length(length)` | yes | `exact_length` |
| `LengthRange { min, max, mode }` | `length_range(min, max) -> Result<_, ValidationError>` | **fallible** (min > max) | `length_range` |

**Byte-mode factories (ASCII/performance paths):**

```rust
min_length_bytes(min)               -> MinLength
max_length_bytes(max)               -> MaxLength
exact_length_bytes(length)          -> ExactLength
length_range_bytes(min, max)        -> Result<LengthRange, ValidationError>

// Also available as methods:
MinLength::bytes(min)
MaxLength::bytes(max)
ExactLength::bytes(length)
LengthRange::bytes(min, max)
```

`LengthMode` enum: `Chars` (default), `Bytes`.

### Pattern — `validators::pattern` (`for str`)

| Type | Factory fn | Field | Notes |
|------|-----------|-------|-------|
| `Contains { substring }` | `contains(s)` | `substring` | substring check |
| `StartsWith { prefix }` | `starts_with(s)` | `prefix` | prefix check |
| `EndsWith { suffix }` | `ends_with(s)` | `suffix` | suffix check |
| `Alphanumeric { allow_spaces }` | `alphanumeric()` | `allow_spaces` | construct directly for spaces |
| `Alphabetic { allow_spaces }` | `alphabetic()` | `allow_spaces` | construct directly for spaces |
| `Numeric` | `numeric()` | — | all chars numeric |
| `Lowercase` | `lowercase()` | — | no uppercase alpha |
| `Uppercase` | `uppercase()` | — | no lowercase alpha |

Error codes: `contains`, `starts_with`, `ends_with`, `alphanumeric`, `alphabetic`,
`numeric`, `lowercase`, `uppercase`.

### Content — `validators::content` (`for str`)

| Type | Factory fn | Error code | Notes |
|------|-----------|------------|-------|
| `MatchesRegex { pattern }` | `matches_regex(pattern) -> Result<_, regex::Error>` | `invalid_format` | fallible construction |
| `Email { pattern }` | `email()` | `invalid_format` | RFC 5321-ish regex, static lazy init |
| `Url { pattern }` | `url()` | `invalid_format` | HTTP/HTTPS only |

`Email` and `Url` use `LazyLock<Regex>` (compiled once, shared).

### Numeric Range — `validators::range` (`for T: PartialOrd + Display + Copy`)

| Type | Factory fn | Error code | Boundary |
|------|-----------|------------|---------|
| `Min<T> { min }` | `min(value)` | `min` | inclusive `>=` |
| `Max<T> { max }` | `max(value)` | `max` | inclusive `<=` |
| `InRange<T> { min, max }` | `in_range(min, max)` | `out_of_range` | inclusive `[min, max]` |
| `GreaterThan<T> { bound }` | `greater_than(bound)` | `greater_than` | exclusive `>` |
| `LessThan<T> { bound }` | `less_than(bound)` | `less_than` | exclusive `<` |
| `ExclusiveRange<T> { min, max }` | `exclusive_range(min, max)` | `exclusive_range` | exclusive `(min, max)` |

Works with any `T: PartialOrd + Display + Copy` — `i32`, `f64`, `u64`, etc.

### Collection Size — `validators::size` (`for [T]`)

All validators work on slice `&[T]`, `&Vec<T>`, etc.

| Type | Factory fn | Error code |
|------|-----------|------------|
| `MinSize<T> { min }` | `min_size::<T>(min)` | `min_size` |
| `MaxSize<T> { max }` | `max_size::<T>(max)` | `max_size` |
| `ExactSize<T> { size }` | `exact_size::<T>(size)` | `exact_size` |
| `NotEmptyCollection<T>` | `not_empty_collection::<T>()` | `not_empty` |
| `SizeRange<T> { min, max }` | `size_range::<T>(min, max)` | `size_range` |

`SizeRange` does **not** validate that `min <= max` at construction; check at call site.

### Boolean — `validators::boolean` (`for bool`)

| Type | Factory fn | Const | Error code |
|------|-----------|-------|------------|
| `IsTrue` | `is_true()` | `IS_TRUE` | `is_true` |
| `IsFalse` | `is_false()` | `IS_FALSE` | `is_false` |

Const validators (`IS_TRUE`, `IS_FALSE`) are zero-cost — use in hot paths.

### Nullable — `validators::nullable` (`for Option<T>`)

| Type | Factory fn | Error code | Notes |
|------|-----------|------------|-------|
| `Required<T>` | `required::<T>()` | `required` | passes if `Some(_)` |
| `NotNull<T>` | `not_null::<T>()` | `required` | alias for `Required<T>` |

### Network — `validators::network` (`for str`)

Uses `std::net` — no external dependencies.

| Type | Factory fn | Error code | Validates |
|------|-----------|------------|---------|
| `Ipv4` | `ipv4()` | `ipv4` | `std::net::Ipv4Addr::parse` |
| `Ipv6` | `ipv6()` | `ipv6` | `std::net::Ipv6Addr::parse` |
| `IpAddr` | `ip_addr()` | `ip_addr` | IPv4 or IPv6 |
| `Hostname` | `hostname()` | `hostname` | RFC 1123: total 1–253 chars, labels 1–63 chars, `[a-z0-9-]`, no leading/trailing hyphen |

### Temporal — `validators::temporal` (`for str`)

Pure-Rust, no chrono/uuid dependencies. Validates format and value ranges.

| Type | Factory fn | Error code | Format |
|------|-----------|------------|--------|
| `Date` | `date()` | `date` | `YYYY-MM-DD` — validates year, month (1–12), day (leap-year-aware) |
| `Time` | `time()` | `time` | `HH:MM:SS` or `HH:MM:SS.sss` — allows leap second (60) |
| `DateTime` | `date_time()` | `datetime` | RFC 3339 — date`T`time`Z/±HH:MM`; separator may be `T`, `t`, or space |
| `Uuid` | `uuid()` | `uuid` | RFC 4122 `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, case-insensitive |

---

## Combinator Catalog

All combinators are in `combinators::*` and selected ones are re-exported by `prelude::*`.

### Logical (`foundation::traits`)

| Combinator | Created by | Behaviour |
|-----------|-----------|-----------|
| `And<L, R>` | `.and(v)` | Both must pass; returns first failure (short-circuit) |
| `Or<L, R>` | `.or(v)` | Either must pass; code `or_failed`, nests both errors |
| `Not<V>` | `.not()` | Inverts; code `not_failed` |
| `When<V, C>` | `.when(|input| bool)` | Skips inner validator when predicate returns `false` |

### Optional

```rust
optional(v: V) -> Optional<V>    // None always Ok; Some(x) delegates to V
Optional::new(v)
Optional::into_inner(self) -> V
```

### Each (slices)

```rust
each(v: V) -> Each<V>             // validates all elements, collects all errors
each_fail_fast(v: V) -> Each<V>   // stops at first failing element
Each::new(v)
Each::fail_fast(v)
Each::with_fail_fast(self, bool) -> Self
```

Error code: `each_failed`. Params: `failed_count`, `total_count`, `failed_indices` (comma-separated).
In fail-fast mode: param `index` of the first failure.

### Factory combinators (homogeneous `Vec`)

```rust
all_of(validators: impl IntoIterator<Item = V>) -> AllOf<V>
// All must pass; aggregates all failures as nested errors

any_of(validators: impl IntoIterator<Item = V>) -> AnyOf<V>
// At least one must pass; aggregates all failures as nested errors
// Empty AnyOf always passes
```

Both accept same-type iterators. Use `AnyValidator<T>` for heterogeneous types:
```rust
all_of([AnyValidator::new(min_length(3)), AnyValidator::new(email())])
```

### Field combinators (struct fields)

```rust
field(validator: V, accessor: F) -> Field<T, U, V, F>
// unnamed field validator

named_field(name: impl Into<Cow<'static, str>>, validator: V, accessor: F) -> Field<T, U, V, F>
// adds field name to error path (dot notation: "user.email")

// Extension trait (blanket impl on all Validate<U>):
validator.for_field("name", |t: &T| &t.field) -> Field<T, U, V, F>
validator.for_field_unnamed(|t: &T| &t.field) -> Field<T, U, V, F>
```

`Field` error path composition: if inner error has field `"sub"`, final field becomes `"name.sub"`.

**`MultiField<T>` — builder for multiple fields:**

```rust
MultiField::<T>::new()
    .add_field("name", min_length(3), |u: &User| u.name.as_str())
    .add_field("age", in_range(18, 130), |u: &User| &u.age)
// validates all fields, collects all errors
// 1 error → returned directly; 2+ errors → wrapped with code "multiple_field_errors"
```

### JSON field combinators

```rust
json_field(pointer: impl Into<Cow<'static, str>>, validator: V) -> JsonField<V, I>
// Required: missing path → error code "path_not_found"

json_field_optional(pointer, validator) -> JsonField<V, I>
// Missing path or null value → Ok

// Path format: RFC 6901 JSON Pointer
// "/server/host"   — nested object
// "/tags/0"        — array index
// ""               — root value
```

Type mismatch (e.g., number where string expected) → error code `type_mismatch`.
Field path in error matches the pointer string (e.g., `"/name"`).

### Message override combinators

```rust
with_message(validator, message: &'static str) -> WithMessage<V>
with_code(validator, code: &'static str) -> WithCode<V>
```

### Caching

```rust
cached(validator: V) -> Cached<V>
// Memoizes results by input hash; thread-safe via RwLock
// CacheStats: hits, misses, evictions
```

### Lazy construction

```rust
lazy(factory: impl Fn() -> V + Send + Sync + 'static) -> Lazy<V>
// Defers validator construction until first use; useful for expensive regex setup
```

### Unless (negated condition)

```rust
unless(validator: V, condition: C) -> Unless<V, C>
// Equivalent to when(!condition)
```

---

## The `validator!` Macro

Generates: struct definition + `Validate<T>` impl + constructor + optional factory fn.
`#[derive(Debug, Clone)]` is always applied. Add extra derives via `#[derive(...)]`.

### Syntax variants

**1. Unit validator** (zero-sized struct, no fields):
```rust
validator! {
    pub NotEmpty for str;
    rule(input) { !input.is_empty() }
    error(input) { ValidationError::new("not_empty", "must not be empty") }
    fn not_empty();  // optional; becomes const fn for unit structs
}
```

**2. Struct with fields** (auto `new` from all fields):
```rust
validator! {
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub MinLength { min: usize, mode: LengthMode } for str;
    rule(self, input) { self.mode.measure(input) >= self.min }
    error(self, input) { ValidationError::min_length("", self.min, input.len()) }
    fn min_length(min: usize, mode: LengthMode);
}
```

**3. Custom constructor** (overrides auto `new`):
```rust
validator! {
    pub LengthRange { min: usize, max: usize } for str;
    rule(self, input) { let l = input.len(); l >= self.min && l <= self.max }
    error(self, input) { ValidationError::new("length_range", "out of range") }
    new(min: usize, max: usize) {
        Self { min, max }
    }
    fn length_range(min: usize, max: usize);
}
```

**4. Fallible constructor** (returns `Result`):
```rust
validator! {
    pub LengthRange { min: usize, max: usize } for str;
    // ...
    new(min: usize, max: usize) -> ValidationError {
        if min > max { return Err(ValidationError::new("invalid_range", "min > max")); }
        Ok(Self { min, max })
    }
    fn length_range(min: usize, max: usize) -> ValidationError;
}
```

**5. Bounded generic**:
```rust
validator! {
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub Min<T: PartialOrd + Display + Copy> { min: T } for T;
    rule(self, input) { *input >= self.min }
    error(self, input) { ValidationError::new("min", format!("must be >= {}", self.min)) }
    fn min(value: T);
}
```

**6. Phantom generic** (generic, no bounds, no fields):
```rust
validator! {
    pub Required<T> for Option<T>;
    rule(input) { input.is_some() }
    error(input) { ValidationError::new("required", "required") }
    fn required();
}
```

### Helper macros

```rust
// AND-chain (equivalent to a.and(b).and(c)):
let v = compose![min_length(3), max_length(20), alphanumeric()];

// OR-chain (equivalent to a.or(b).or(c)):
let v = any_of![exact_length(5), exact_length(10)];
```

---

## Prelude

`use nebula_validator::prelude::*` imports:

- Foundation: `Validate`, `ValidateExt`, `Validatable`, `ValidationError`, `ValidationErrors`,
  `AnyValidator`, `ErrorSeverity`, `And`, `Or`, `Not`, `When`, `AsValidatable`
- Proof tokens: `Validated`, `ValidatorError`
- All validators: `validators::*` (glob)
- Key combinators: `and`, `or`, `not`, `cached`, `json_field`, `json_field_optional`, `Cached`, `JsonField`

For the full combinator set use `use nebula_validator::combinators::prelude::*`.

---

## Error Code Catalog (Complete)

### Baseline (compatibility-critical, in contract fixtures)

| Code | Produced by | Meaning |
|------|------------|---------|
| `required` | `Required`, `ValidationError::required` | Missing required value |
| `min_length` | `MinLength`, `ValidationError::min_length` | String too short |
| `max_length` | `MaxLength`, `ValidationError::max_length` | String too long |
| `invalid_format` | `MatchesRegex`, `Email`, `Url`, `ValidationError::invalid_format` | Format mismatch |
| `type_mismatch` | JSON bridge, `ValidationError::type_mismatch` | Type conversion failure |
| `out_of_range` | `InRange`, `ValidationError::out_of_range` | Numeric out of inclusive range |
| `exact_length` | `ExactLength`, `ValidationError::exact_length` | Wrong exact length |
| `length_range` | `LengthRange`, `ValidationError::length_range` | Length outside range |
| `or_failed` | `Or` combinator | All OR alternatives failed |
| `not_failed` | `Not` combinator | NOT inner unexpectedly passed |

### Extended (non-contract)

| Code | Produced by |
|------|------------|
| `not_empty` | `NotEmpty`, `NotEmptyCollection` |
| `min` | `Min<T>` |
| `max` | `Max<T>` |
| `greater_than` | `GreaterThan<T>` |
| `less_than` | `LessThan<T>` |
| `exclusive_range` | `ExclusiveRange<T>` |
| `contains` | `Contains` |
| `starts_with` | `StartsWith` |
| `ends_with` | `EndsWith` |
| `alphanumeric` | `Alphanumeric` |
| `alphabetic` | `Alphabetic` |
| `numeric` | `Numeric` |
| `lowercase` | `Lowercase` |
| `uppercase` | `Uppercase` |
| `ipv4` / `ipv6` / `ip_addr` | network validators |
| `hostname` | `Hostname` |
| `date` / `time` / `datetime` / `uuid` | temporal validators |
| `min_size` / `max_size` / `exact_size` / `size_range` | collection size validators |
| `each_failed` | `Each` combinator |
| `is_true` / `is_false` | `IsTrue` / `IsFalse` |
| `path_not_found` | `JsonField` (required mode) |
| `all_of` / `any_of` | `AllOf` / `AnyOf` factory combinators |
| `multiple_field_errors` | `MultiField` (2+ field failures) |
| `custom` | `ValidationError::custom` |
| `invalid_range` | `LengthRange::new` / `LengthRange::bytes` when `min > max` |
| `or_any_failed` | `OrAny` combinator |

---

## Error Semantics

- Retryable: not applicable — validation failures are deterministic contract failures.
- Fatal: invalid input, shape, range, or pattern failures.
- Aggregation: `ValidationErrors` collects multiple `ValidationError`s; use
  `into_single_error()` to wrap as nested errors under one parent.

---

## Field-Path Contract

- **Typed field combinators** (`field`, `named_field`): dot notation
  - `user.email`, `config.timeout`, `items.0.name`
  - Composed by `Field`: `"parent"` + inner field `"sub"` → `"parent.sub"`
- **JSON field combinators** (`json_field`, `json_field_optional`): RFC 6901 JSON Pointer
  - `/user/email`, `/items/0/name`, `""` (root)
- Contract rule: field-path format for an existing API is stable across minor releases.
  Changing format semantics requires a major version bump and migration mapping.

Serialized envelope: runtime JSON uses the `field` key.

---

## Cross-Crate Category Contract (Config Integration)

Canonical category names shared with config compatibility fixtures:

- `source_load_failed`, `merge_failed`, `validation_failed`, `missing_path`,
  `type_mismatch`, `invalid_value`, `watcher_failed`

---

## Compatibility Rules

- A **major bump required** when:
  - Behaviour changes for existing validator semantics
  - Existing baseline error code meanings change
  - Field-path format contract changes
- **Minor versions:** additive validators / combinators / error helpers only
- **Deprecation policy:** mark with migration path; maintain for at least one minor cycle

---

## Contract Test Fixtures

- `crates/validator/tests/fixtures/compat/minor_contract_v1.json`
- `crates/validator/tests/fixtures/compat/error_registry_v1.json`
- `crates/validator/tests/contract/compatibility_fixtures_test.rs`
- `crates/validator/tests/contract/typed_dynamic_equivalence_test.rs`
- `crates/validator/tests/contract/error_envelope_schema_test.rs`
- `crates/validator/tests/contract/governance_policy_test.rs`

---

## Usage Examples

### Minimal

```rust
use nebula_validator::prelude::*;

let username = min_length(3).and(max_length(20)).and(alphanumeric());
username.validate("alice123")?;
// "ab" → Err(min_length)
// "alice_123" → Err(alphanumeric)
```

### validate_any bridge

```rust
use nebula_validator::prelude::*;
use serde_json::json;

let password_rule = min_length(12).and(contains("@"));
password_rule.validate_any(&json!("very@secure_pass"))?;
```

### Struct field validation

```rust
use nebula_validator::combinators::{MultiField, named_field};
use nebula_validator::validators::{min_length, in_range};

struct User { name: String, age: u32 }

let validator = MultiField::<User>::new()
    .add_field("name", min_length(2), |u: &User| u.name.as_str())
    .add_field("age",  in_range(18u32, 130), |u: &User| &u.age);

validator.validate(&User { name: "Al".into(), age: 15 })?;
// Err: multiple_field_errors with nested name/age errors
```

### JSON validation

```rust
use nebula_validator::combinators::{json_field, json_field_optional};
use nebula_validator::validators::{min_length, min};
use serde_json::json;

let v = json_field("/server/host", min_length(1))
    .and(json_field("/server/port", min::<i64>(1)))
    .and(json_field_optional("/server/tls", is_true()));

v.validate(&json!({ "server": { "host": "localhost", "port": 8080 } }))?;
```

### Custom validator via macro

```rust
use nebula_validator::{validator, foundation::{Validate, ValidationError}};

validator! {
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub NonZero<T: PartialEq + Default + std::fmt::Display + Copy> { } for T;
    rule(input) { *input != T::default() }
    error(input) { ValidationError::new("non_zero", format!("{} must not be zero/default", input)) }
    fn non_zero();
}

non_zero::<i32>().validate(&5)?;  // Ok
non_zero::<i32>().validate(&0)?;  // Err(non_zero)
```
