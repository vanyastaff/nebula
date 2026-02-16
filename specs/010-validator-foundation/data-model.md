# Data Model: Validator Foundation

**Branch**: `010-validator-foundation` | **Date**: 2026-02-16

## Core Entities

### Validate Trait (unchanged)

```rust
pub trait Validate {
    type Input: ?Sized;
    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError>;
    fn validate_any<S>(&self, value: &S) -> Result<(), ValidationError>
    where
        Self: Sized,
        S: AsValidatable<Self::Input> + ?Sized,
        for<'a> <S as AsValidatable<Self::Input>>::Output<'a>: Borrow<Self::Input>;
    fn metadata(&self) -> ValidatorMetadata;
    fn name(&self) -> &str;
}
```

**Relationships**: Every validator struct implements `Validate`. Extended by `ValidateExt` (blanket impl).

### AsValidatable Trait (unchanged)

```rust
pub trait AsValidatable<T: ?Sized> {
    type Output<'a>: Borrow<T> where Self: 'a;
    fn as_validatable(&self) -> Result<Self::Output<'_>, ValidationError>;
}
```

**Implementations** (behind `serde` feature for Value variants):
- `Value::String → &str`
- `Value::Number → f64 / i64`
- `Value::Bool → bool`
- `Value::Array → &[Value]`
- Type mismatch → `ValidationError { code: "type_mismatch" }`

### ValidationError (unchanged structure, Cow kept)

```rust
pub struct ValidationError {
    pub code: Cow<'static, str>,
    pub message: Cow<'static, str>,
    pub field: Option<Cow<'static, str>>,
    pub params: Vec<(Cow<'static, str>, Cow<'static, str>)>,
    pub nested: Vec<ValidationError>,
    pub severity: ErrorSeverity,
    pub help: Option<Cow<'static, str>>,
}
```

**Decision**: Cow<'static, str> kept for all fields (see research.md R1).

### ValidateExt Trait (Map removed)

```rust
pub trait ValidateExt: Validate + Sized {
    fn and<V>(self, other: V) -> And<Self, V>;
    fn or<V>(self, other: V) -> Or<Self, V>;
    fn not(self) -> Not<Self>;
    fn when<C>(self, condition: C) -> When<Self, C>;
    fn optional(self) -> Optional<Self>;

    #[cfg(feature = "caching")]
    fn cached(self) -> Cached<Self>;
    #[cfg(feature = "caching")]
    fn cached_with_capacity(self, capacity: usize) -> Cached<Self>;

    // REMOVED: fn map() — deprecated Map combinator deleted
}
```

## New Entities

### Hostname Validator

```rust
pub struct Hostname;

impl Validate for Hostname {
    type Input = str;
    fn validate(&self, input: &str) -> Result<(), ValidationError>;
}

pub fn hostname() -> Hostname;
```

### TimeOnly Validator

```rust
pub struct TimeOnly {
    allow_milliseconds: bool,
    require_timezone: bool,
}

impl TimeOnly {
    pub fn new() -> Self;
    pub fn require_timezone(self) -> Self;
}

impl Validate for TimeOnly {
    type Input = str;
}

pub fn time_only() -> TimeOnly;
```

### DateTime::date_only() (extension of existing)

```rust
impl DateTime {
    pub fn date_only() -> Self;
}
```

## Removed Entities

| Entity | File | Reason |
|--------|------|--------|
| `AsyncValidate` trait | `foundation/traits.rs` | 0 implementations, Phase 8 |
| `Refined<T, V>` | `foundation/refined.rs` | 0 consumers, Phase 7 |
| `Parameter<T, S>` | `foundation/state.rs` | 0 consumers, Phase 7 |
| `Unvalidated` / `Validated<V>` | `foundation/state.rs` | Part of type-state |
| `ParameterBuilder<T, S>` | `foundation/state.rs` | Part of type-state |
| `Map<V, F>` combinator | `combinators/map.rs` | Deprecated no-op |
| Type aliases: `NonEmptyString`, `EmailAddress`, `Url`, `NonEmptyVec`, `PositiveNumber` | `foundation/refined.rs` | Part of Refined |

## Feature-Gated Entities

| Entity | Feature | File |
|--------|---------|------|
| `Cached<V>`, `CacheStats` | `caching` | `combinators/cached.rs` |
| `.cached()`, `.cached_with_capacity()` | `caching` | `foundation/traits.rs` (ValidateExt) |
| `ValidatorChainOptimizer` | `optimizer` | `combinators/optimizer.rs` |
| `ValidatorStats`, `OptimizationReport` | `optimizer` | `combinators/optimizer.rs` |
| `ValidatorStatistics` | `optimizer` | `foundation/metadata.rs` |
| `RegisteredValidatorMetadata` | `optimizer` | `foundation/metadata.rs` |
| `AsValidatable<_> for Value` | `serde` | `foundation/validatable.rs` |
| `JsonField` combinator | `serde` | `combinators/json_field.rs` |
| `json` module | `serde` | `json.rs` |

## Module Structure (Target)

```
crates/validator/src/
├── lib.rs                   # pub mod foundation, validators, combinators, json, prelude
├── prelude.rs               # Re-exports traits + all factory functions + json helpers
├── json.rs                  # #[cfg(feature = "serde")] json_min_size, json_max_size, etc.
│
├── foundation/
│   ├── mod.rs               # Re-exports (no refined, no state, no AsyncValidate)
│   ├── traits.rs            # Validate, ValidateExt (no AsyncValidate)
│   ├── error.rs             # ValidationError, ValidationErrors, ErrorSeverity
│   ├── context.rs           # ValidationContext, ContextualValidator
│   ├── metadata.rs          # ValidatorMetadata, ValidationComplexity
│   ├── category.rs          # Error category taxonomy
│   └── validatable.rs       # AsValidatable + Value impls (serde-gated)
│
├── validators/
│   ├── mod.rs               # Flat re-exports of ALL validators
│   ├── length.rs            # String length validators
│   ├── pattern.rs           # String pattern validators
│   ├── content.rs           # Email, Url
│   ├── uuid.rs
│   ├── datetime.rs          # + date_only()
│   ├── time.rs              # NEW: TimeOnly
│   ├── json_string.rs       # JSON structure validator
│   ├── password.rs
│   ├── phone.rs
│   ├── credit_card.rs
│   ├── iban.rs
│   ├── semver.rs
│   ├── slug.rs
│   ├── hex.rs
│   ├── base64.rs
│   ├── range.rs             # Numeric range validators
│   ├── properties.rs        # Numeric properties
│   ├── divisibility.rs
│   ├── float.rs
│   ├── percentage.rs
│   ├── size.rs              # Collection size validators
│   ├── elements.rs          # Collection element validators
│   ├── structure.rs         # HasKey
│   ├── ip_address.rs
│   ├── hostname.rs          # NEW: RFC 1123
│   ├── port.rs
│   ├── mac_address.rs
│   ├── boolean.rs
│   └── nullable.rs
│
├── combinators/
│   ├── mod.rs
│   ├── and.rs, or.rs, not.rs
│   ├── optional.rs, when.rs, unless.rs
│   ├── each.rs, lazy.rs
│   ├── cached.rs            # #[cfg(feature = "caching")]
│   ├── field.rs, json_field.rs  # json_field: #[cfg(feature = "serde")]
│   ├── nested.rs, message.rs
│   ├── error.rs
│   └── optimizer.rs         # #[cfg(feature = "optimizer")]
│
└── macros/
    └── mod.rs               # validator!, validate!, compose!, any_of!, etc.
```
