# nebula-validator — API Reference

This document covers every public type, trait, and method. For high-level design rationale
see [`architecture.md`](architecture.md). For combinator-specific usage patterns see
[`combinators.md`](combinators.md). For writing custom validators see [`extending.md`](extending.md).

---

## Stability Tiers

| Tier | Items |
|------|-------|
| **Stable** | `Validate<T>`, `ValidateExt<T>`, `Validatable`, `ValidationError`, `ValidationErrors`, `AnyValidator<T>`, `ErrorSeverity`, `Validated<T>`, `ValidatorError`, all built-in validators, core combinators, the `validator!` macro |
| **Experimental** | `MultiField` internals, advanced `NestedValidate` helpers — treat as non-contract |
| **Internal** | `ErasedValidator` trait, `AsValidatable` bridge, macro `@`-arms |

---

## Foundation Traits

### `Validate<T>`

```rust
pub trait Validate<T: ?Sized> {
    fn validate(&self, input: &T) -> Result<(), ValidationError>;

    fn validate_any<U>(&self, input: &U) -> Result<(), ValidationError>
    where
        U: AsValidatable<T>;

    fn validate_into<V>(&self, value: V) -> ValidatorResult<Validated<V>>
    where
        V: Borrow<T>,
        Self: Sized;
}
```

- `validate` — core method every validator must implement.
- `validate_any` — bridge for `serde_json::Value` inputs when `Value: AsValidatable<T>`.
  Allows string validators to accept `&json!("hello")` directly.
- `validate_into` — validates and wraps the value in a `Validated<V>` proof token.
  Returns `Err(ValidatorError::ValidationFailed(..))` on failure.

### `ValidateExt<T>`

Blanket impl on every `Validate<T>`:

| Method | Returns | Behaviour |
|--------|---------|-----------|
| `.and(v)` | `And<Self, V>` | Both must pass; short-circuits on first failure |
| `.or(v)` | `Or<Self, V>` | At least one must pass; code `or_failed` |
| `.not()` | `Not<Self>` | Inverts result; code `not_failed` |
| `.when(fn(&T) -> bool)` | `When<Self, C>` | Skips validation when predicate returns `false` |
| `.for_field(name, accessor)` | `Field<T, U, Self, F>` | Wraps self in a named field extractor |
| `.for_field_unnamed(accessor)` | `Field<T, U, Self, F>` | Same without a name in the error path |

### `Validatable`

Extension-method style: blanket impl on all types.

```rust
value.validate_with(&validator) -> Result<&Self, ValidationError>
```

Returns `&self` on success, enabling chained calls.

---

## `ValidationError`

80-byte struct. `Cow<'static, str>` fields mean static strings flow through without allocation.

### Public fields

| Field | Type | Notes |
|-------|------|-------|
| `code` | `Cow<'static, str>` | Machine-readable error identifier |
| `message` | `Cow<'static, str>` | Human-readable description |
| `field` | `Option<Cow<'static, str>>` | RFC 6901 JSON Pointer path |

### Constructor

```rust
ValidationError::new(
    code:    impl Into<Cow<'static, str>>,
    message: impl Into<Cow<'static, str>>,
)
```

### Convenience constructors

| Constructor | Code |
|-------------|------|
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
.with_field(field: impl Into<Cow<'static, str>>)    -> ValidationError
.with_pointer(ptr: impl Into<Cow<'static, str>>)    -> ValidationError
.with_param(key, value)                             -> ValidationError
.with_nested(errors: Vec<ValidationError>)          -> ValidationError
.with_nested_error(e: ValidationError)              -> ValidationError
.with_severity(s: ErrorSeverity)                    -> ValidationError
.with_help(text: impl Into<Cow<'static, str>>)      -> ValidationError
```

`with_field(path)` normalizes dot/bracket notation to RFC 6901 pointer.
`with_param` redacts values for keys matching `password`, `secret`, `token`, `api_key`,
`apikey`, or `credential`.

### Accessor methods

```rust
.param(key: &str)       -> Option<&str>
.params()               -> &[(Cow<str>, Cow<str>)]
.nested()               -> &[ValidationError]
.has_nested()           -> bool
.severity()             -> ErrorSeverity
.help()                 -> Option<&str>
.field_pointer()        -> Option<Cow<str>>   // RFC 6901 canonical form
.total_error_count()    -> usize              // 1 + recursive nested count
.flatten()              -> Vec<&ValidationError> // depth-first, all nested
.to_json_value()        -> serde_json::Value  // emits both `field` and `pointer` keys
```

### `Display` format

```
[/user/email] min_length: must be at least 3 characters (params: [min=3, actual=1])
  Help: Use at least 3 characters.
  Nested errors:
    1. ...
```

---

## `ValidationErrors`

Aggregate wrapper for multiple `ValidationError` values.

```rust
ValidationErrors::new()                                 -> Self
.add(e: ValidationError)
.extend(iter: impl IntoIterator<Item = ValidationError>)
.has_errors()                                           -> bool
.len() / .is_empty() / .iter() / .errors()              -> slice access
.last_mut()                                             -> Option<&mut ValidationError>
.into_single_error(msg: &str)                           -> ValidationError
```

Implements `FromIterator<ValidationError>` and `IntoIterator`.

---

## `ErrorSeverity`

```rust
pub enum ErrorSeverity { Error, Warning, Info }
```

Default is `Error`. Set via `.with_severity()` on `ValidationError`.

---

## `AnyValidator<T>`

Type-erased validator for storing validators of different concrete types in one collection.

```rust
AnyValidator::<T>::new(v: V) -> AnyValidator<T>
// requires V: Validate<T> + Clone + Send + Sync + 'static
```

Implements `Validate<T>` via dynamic dispatch (~2–5 ns overhead). Implements `Clone`.

```rust
// Heterogeneous collection:
let validators: Vec<AnyValidator<str>> = vec![
    AnyValidator::new(min_length(3)),
    AnyValidator::new(email()),
    AnyValidator::new(matches_regex("[a-z]+")?),
];
for v in &validators {
    v.validate("hello")?;
}
```

---

## `Validated<T>`

Zero-cost proof token certifying the inner value passed validation.

### Construction

| Method | Notes |
|--------|-------|
| `validator.validate_into(value)` | Primary path; returns `ValidatorResult<Validated<V>>` |
| `Validated::new(value, &validator)` | Direct construction; same semantics |
| `Validated::new_unchecked(value)` | Internal escape hatch (`pub(crate)`) |

### Access

| Method | Returns | Notes |
|--------|---------|-------|
| `.inner()` | `&T` | Reference access |
| `.into_inner()` | `T` | Consumes wrapper, returns value |
| `Deref` / `AsRef` / `Borrow` | `&T` | Transparent read access |
| `.map(f)` | `Validated<U>` | Transform inner value |

### Trait implementations

`Clone`, `PartialEq`, `Eq`, `PartialOrd`, `Ord`, `Hash`, `Debug`, `Display`, `Serialize`.
`Deserialize` is intentionally omitted — deserialized data must be re-validated.

---

## `ValidatorError`

Crate-level operational error separating configuration failures from validation failures.

```rust
#[non_exhaustive]
pub enum ValidatorError {
    InvalidConfig { message: Cow<'static, str> },
    ValidationFailed(#[from] ValidationError),
}

pub type ValidatorResult<T> = Result<T, ValidatorError>;
```

`validate()` returns `Result<(), ValidationError>`.
`validate_into()` and `Validated::new()` return `ValidatorResult<T>`.

---

## Built-in Validators

All live in `validators::*` and are re-exported by `prelude::*`.

### Length — `validators::length` (for `str`)

Default mode: **Unicode chars** (`char::count()`). Byte-mode variants use `.len()` for
ASCII-only or performance-sensitive paths.

| Type | Factory | Fallible? | Code |
|------|---------|-----------|------|
| `NotEmpty` | `not_empty()` | No | `not_empty` |
| `MinLength { min, mode }` | `min_length(n)` | No | `min_length` |
| `MaxLength { max, mode }` | `max_length(n)` | No | `max_length` |
| `ExactLength { length, mode }` | `exact_length(n)` | No | `exact_length` |
| `LengthRange { min, max, mode }` | `length_range(min, max)` | **Yes** (`min > max`) | `length_range` |

Byte-mode factory functions: `min_length_bytes(n)`, `max_length_bytes(n)`,
`exact_length_bytes(n)`, `length_range_bytes(min, max)`.
Byte-mode constructors: `MinLength::bytes(n)`, `MaxLength::bytes(n)`, etc.

### Pattern — `validators::pattern` (for `str`)

| Type | Factory | Code |
|------|---------|------|
| `Contains { substring }` | `contains(s)` | `contains` |
| `StartsWith { prefix }` | `starts_with(s)` | `starts_with` |
| `EndsWith { suffix }` | `ends_with(s)` | `ends_with` |
| `Alphanumeric { allow_spaces }` | `alphanumeric()` | `alphanumeric` |
| `Alphabetic { allow_spaces }` | `alphabetic()` | `alphabetic` |
| `Numeric` | `numeric()` | `numeric` |
| `Lowercase` | `lowercase()` | `lowercase` |
| `Uppercase` | `uppercase()` | `uppercase` |

For `allow_spaces`, construct directly: `Alphanumeric { allow_spaces: true }`.

### Content — `validators::content` (for `str`)

| Type | Factory | Fallible? | Code |
|------|---------|-----------|------|
| `MatchesRegex { pattern }` | `matches_regex(s)` | **Yes** (`regex::Error`) | `invalid_format` |
| `Email` | `email()` | No | `invalid_format` |
| `Url` | `url()` | No | `invalid_format` |

`Email` and `Url` compile their regex once via `LazyLock<Regex>`.

### Range — `validators::range` (for `T: PartialOrd + Display + Copy`)

| Type | Factory | Boundary | Code |
|------|---------|----------|------|
| `Min<T>` | `min(v)` | Inclusive `>=` | `min` |
| `Max<T>` | `max(v)` | Inclusive `<=` | `max` |
| `InRange<T>` | `in_range(min, max) -> Result<_, RangeConfigError>` | Inclusive `[min, max]` | `out_of_range` |
| `GreaterThan<T>` | `greater_than(v)` | Exclusive `>` | `greater_than` |
| `LessThan<T>` | `less_than(v)` | Exclusive `<` | `less_than` |
| `ExclusiveRange<T>` | `exclusive_range(min, max) -> Result<_, RangeConfigError>` | Exclusive `(min, max)` | `exclusive_range` |

### Size — `validators::size` (for `[T]`)

| Type | Factory | Code |
|------|---------|------|
| `MinSize<T>` | `min_size::<T>(n)` | `min_size` |
| `MaxSize<T>` | `max_size::<T>(n)` | `max_size` |
| `ExactSize<T>` | `exact_size::<T>(n)` | `exact_size` |
| `NotEmptyCollection<T>` | `not_empty_collection::<T>()` | `not_empty` |
| `SizeRange<T>` | `size_range::<T>(min, max) -> Result<_, RangeConfigError>` | `size_range` |

`SizeRange` does not validate `min <= max` at construction; validate at call site if needed.

### Boolean — `validators::boolean` (for `bool`)

| Type | Factory | Const | Code |
|------|---------|-------|------|
| `IsTrue` | `is_true()` | `IS_TRUE` | `is_true` |
| `IsFalse` | `is_false()` | `IS_FALSE` | `is_false` |

Const variants are zero-cost; prefer them in hot paths.

### Nullable — `validators::nullable` (for `Option<T>`)

| Type | Factory | Code |
|------|---------|------|
| `Required<T>` | `required::<T>()` | `required` |
| `NotNull<T>` | `not_null::<T>()` | `required` |

### Network — `validators::network` (for `str`)

Uses `std::net` only; no external dependencies.

| Type | Factory | Validates | Code |
|------|---------|-----------|------|
| `Ipv4` | `ipv4()` | `Ipv4Addr::parse` | `ipv4` |
| `Ipv6` | `ipv6()` | `Ipv6Addr::parse` | `ipv6` |
| `IpAddr` | `ip_addr()` | IPv4 or IPv6 | `ip_addr` |
| `Hostname` | `hostname()` | RFC 1123: 1–253 chars total, labels 1–63 chars, `[a-z0-9-]`, no leading/trailing hyphen | `hostname` |

### Temporal — `validators::temporal` (for `str`)

Pure Rust; no chrono or uuid dependencies. Validates format and value ranges.

| Type | Factory | Format | Code |
|------|---------|--------|------|
| `Date` | `date()` | `YYYY-MM-DD` (leap-year-aware) | `date` |
| `Time` | `time()` | `HH:MM:SS` or `HH:MM:SS.sss` (leap second allowed) | `time` |
| `DateTime` | `date_time()` | RFC 3339, separator `T`/`t`/space | `datetime` |
| `Uuid` | `uuid()` | RFC 4122 `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, case-insensitive | `uuid` |

---

## Declarative Rules

### `Rule`

`Rule` is a typed sum-of-sums over a bounded flat arena. Its children are read through
`RuleView` (returned by `Rule::view()` and `RuleRef::view()`), never through a public
enum of variants. Construction goes through the `Rule::*` constructors.

```rust
pub struct Rule { /* private arena */ }

impl Rule {
    pub fn value(value: ValueRule) -> Result<Self, RuleBuildError>;
    pub fn predicate(predicate: Predicate) -> Result<Self, RuleBuildError>;
    pub fn min_length(n: usize) -> Self;
    pub fn max_length(n: usize) -> Self;
    pub fn pattern(pattern: &str) -> Result<Self, RuleBuildError>;
    pub fn min_value(n: i64) -> Self;
    pub fn max_value(n: i64) -> Self;
    pub fn one_of(values: impl Into<RuleOperands>) -> Result<Self, RuleBuildError>;
    pub fn all(rules: impl IntoIterator<Item = Rule>) -> Result<Self, RuleBuildError>;
    pub fn any(rules: impl IntoIterator<Item = Rule>) -> Result<Self, RuleBuildError>;
    pub fn not(inner: Rule) -> Result<Self, RuleBuildError>;
    pub fn described(rule: Rule, message: impl Into<String>) -> Result<Self, RuleBuildError>;
    pub fn custom(expression: impl Into<String>) -> Result<Self, RuleBuildError>;
    pub fn unique_by(path: impl AsRef<str>) -> Result<Self, RuleBuildError>;
    pub fn check_limits(&self) -> Result<(), RuleBuildError>;
    pub fn kind(&self) -> RuleKind;
    pub fn field_references<'a>(&'a self, out: &mut Vec<&'a str>);
    pub fn matches(&self, ctx: &PredicateContext) -> Result<bool, ValidationError>;
}
```

**Value rules** — `ValueRule`, applied to a single `serde_json::Value`:

| Variant | Code |
|---------|------|
| `ValueRule::MinLength(usize)` | `min_length` |
| `ValueRule::MaxLength(usize)` | `max_length` |
| `ValueRule::Pattern(RulePattern)` | `invalid_format` |
| `ValueRule::Min(Number)` | `min` |
| `ValueRule::Max(Number)` | `max` |
| `ValueRule::GreaterThan(Number)` | `greater_than` |
| `ValueRule::LessThan(Number)` | `less_than` |
| `ValueRule::OneOf(Vec<Value>)` | `one_of` |
| `ValueRule::MinItems(usize)` | `min_size` |
| `ValueRule::MaxItems(usize)` | `max_size` |
| `ValueRule::Email` | `invalid_format` |
| `ValueRule::Url` | `invalid_format` |

**Context predicates** — `Predicate`, evaluated against a `PredicateContext`:

| Variant | Evaluates |
|---------|-----------|
| `Predicate::Eq(path, value)` | `ctx[path] == value` |
| `Predicate::Ne(path, value)` | `ctx[path] != value` |
| `Predicate::Gt/Gte/Lt/Lte(path, n)` | numeric comparison |
| `Predicate::IsTrue(IsFalse)(path)` | boolean equality |
| `Predicate::Set(path)` | non-null, non-empty value |
| `Predicate::Empty(path)` | null, absent, or empty string/array |
| `Predicate::Contains(path, v)` | string or array contains `v` |
| `Predicate::Matches(path, pattern)` | string matches the checked regex |
| `Predicate::In(path, values)` | `values.contains(ctx[path])` |

**Logical nodes** — reached through `RuleView::All` / `RuleView::Any` / `RuleView::Not`.
Each logical node is created with `Rule::all` / `Rule::any` / `Rule::not`.

### `validate_rules` / `validate_rules_with_ctx`

```rust
pub fn validate_rules(
    value: &serde_json::Value,
    rules: &[Rule],
    mode: ExecutionMode,
    disclosure: DiagnosticDisclosure,
) -> Result<EvaluationOutcome, ValidationErrors>;

pub fn validate_rules_with_ctx(
    value: &serde_json::Value,
    rules: &[Rule],
    ctx: Option<&PredicateContext>,
    mode: ExecutionMode,
    disclosure: DiagnosticDisclosure,
) -> Result<EvaluationOutcome, ValidationErrors>;
```

A successful pass is not necessarily proof of satisfaction: `EvaluationOutcome::Deferred`
lists obligations (`DeferredReason`) that a later pass with runtime context must discharge.
`EvaluationOutcome::require_satisfied()` converts a partial result into an `unavailable`
diagnostic.

### `ExecutionMode`

```rust
pub enum ExecutionMode {
    StaticOnly,  // run static rules; report deferred rules as explicit obligations
    Deferred,    // run only deferred rules
    Full,        // run all rules; unavailable context/evaluators are diagnostics
}
```

### `DiagnosticDisclosure`

```rust
pub enum DiagnosticDisclosure {
    IncludeValue, // include evaluated values in diagnostic params
    OmitValue,    // never materialize evaluated values into diagnostics
}
```

Schema owners choose `OmitValue` for protected fields; the policy propagates through the
entire rule tree.

---

## Error Code Catalog

### Baseline (compatibility-critical)

| Code | Source |
|------|--------|
| `required` | `Required`, `ValidationError::required` |
| `min_length` | `MinLength`, `ValidationError::min_length` |
| `max_length` | `MaxLength`, `ValidationError::max_length` |
| `invalid_format` | `MatchesRegex`, `Email`, `Url`, `ValidationError::invalid_format` |
| `type_mismatch` | JSON bridge, `ValidationError::type_mismatch` |
| `out_of_range` | `InRange`, `ValidationError::out_of_range` |
| `exact_length` | `ExactLength`, `ValidationError::exact_length` |
| `length_range` | `LengthRange`, `ValidationError::length_range` |
| `or_failed` | `Or` combinator |
| `not_failed` | `Not` combinator |

### Extended

| Code | Source |
|------|--------|
| `not_empty` | `NotEmpty`, `NotEmptyCollection` |
| `min` / `max` | `Min<T>` / `Max<T>` |
| `greater_than` / `less_than` / `exclusive_range` | range validators |
| `contains` / `starts_with` / `ends_with` | pattern validators |
| `alphanumeric` / `alphabetic` / `numeric` / `lowercase` / `uppercase` | pattern validators |
| `ipv4` / `ipv6` / `ip_addr` / `hostname` | network validators |
| `date` / `time` / `datetime` / `uuid` | temporal validators |
| `min_size` / `max_size` / `exact_size` / `size_range` | size validators |
| `each_failed` | `Each` combinator |
| `is_true` / `is_false` | `IsTrue` / `IsFalse` |
| `path_not_found` | `JsonField` (required mode) |
| `all_of` / `any_of` | `AllOf` / `AnyOf` |
| `multiple_field_errors` | `MultiField` (2+ failures) |
| `custom` | `ValidationError::custom` |
| `invalid_range` | `LengthRange::new` when `min > max` |

---

## Prelude

`use nebula_validator::prelude::*` imports:

- Foundation: `Validate`, `ValidateExt`, `Validatable`, `ValidationError`, `ValidationErrors`,
  `AnyValidator`, `ErrorSeverity`, `AsValidatable`
- Proof: `Validated`, `ValidatorError`
- All validators: `validators::*` (glob)
- Key combinators: `And`, `Or`, `Not`, `When`, `and`, `or`, `not`, `json_field`,
  `json_field_optional`, `JsonField`

`nebula_validator::prelude` is the single import surface. The former
`foundation::prelude` and `combinators::prelude` were removed; import the
individual items from their module paths if you need something the prelude
does not cover.

---

## Field-Path Contract

Typed field combinators (`field`, `named_field`, `MultiField`) produce **dot notation** paths:
`user.email`, `config.timeout`, `items.0.name`.

JSON field combinators (`json_field`, `json_field_optional`) use **RFC 6901 JSON Pointer**
paths: `/user/email`, `/items/0/name`, `""` (root).

Both formats are accepted by `ValidationError::with_field()` and normalized to JSON Pointer
in the stored `field` value. The field-path format for a given validator/combinator combination
is part of the minor-release stability contract.

---

## Category Contract

These canonical category names are recorded in the error registry
(`tests/fixtures/compat/error_registry_v1.json`) and must remain stable:

`source_load_failed`, `merge_failed`, `validation_failed`, `missing_path`,
`type_mismatch`, `invalid_value`, `watcher_failed`

Changes require a major version bump and a migration mapping in [`migration.md`](migration.md).
