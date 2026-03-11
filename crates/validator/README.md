# nebula-validator

Composable, type-safe validation framework for the Nebula workflow engine.

> Canonical contract and compatibility rules are documented in
> `crates/validator/docs/API.md`, `crates/validator/docs/DECISIONS.md`,
> and `crates/validator/docs/MIGRATION.md`.

## Features

- **Composable** — chain validators with `.and()`, `.or()`, `.not()` extension methods
- **Type-safe** — generic `Validate<T>` trait with strong typing and zero-cost combinators
- **Proof tokens** — `Validated<T>` wrapper certifies a value passed validation at the type level
- **Structured errors** — 80-byte `ValidationError` with `Cow`-based code/message, RFC 6901 field paths, nested children
- **43 stable error codes** — machine-readable registry (`error_registry_v1.json`) with additive-only minor-release policy
- **Dynamic bridge** — validate `serde_json::Value` through `AsValidatable` trait conversions
- **Extensible** — `validator!` macro for zero-boilerplate custom validators, or implement `Validate<T>` manually

## Quick Start

```rust
use nebula_validator::prelude::*;

// Compose validators left-to-right
let username = min_length(3).and(max_length(20)).and(alphanumeric());
assert!(username.validate("alice123").is_ok());
assert!(username.validate("ab").is_err()); // min_length error

// Proof tokens: validate once, carry proof through the system
let name: Validated<String> = min_length(3).validate_into("alice".to_string()).unwrap();
println!("Validated: {}", name.as_ref());

// Extension method style
"hello".validate_with(&min_length(3)).unwrap();
```

## Validators

### String — Length

| Factory | Error Code | Description |
|---------|-----------|-------------|
| `min_length(n)` | `min_length` | Minimum character count |
| `max_length(n)` | `max_length` | Maximum character count |
| `exact_length(n)` | `exact_length` | Exact character count |
| `length_range(min, max)` | `length_range` | Character count in inclusive range |
| `not_empty()` | `not_empty` | Must not be empty |

Byte-length variants: `min_length_bytes`, `max_length_bytes`, `exact_length_bytes`, `length_range_bytes`.

### String — Pattern

| Factory | Error Code | Description |
|---------|-----------|-------------|
| `contains(s)` | `contains` | Contains substring |
| `starts_with(s)` | `starts_with` | Starts with prefix |
| `ends_with(s)` | `ends_with` | Ends with suffix |
| `alphanumeric()` | `alphanumeric` | Only `[a-zA-Z0-9]` |
| `alphabetic()` | `alphabetic` | Only letters |
| `numeric()` | `numeric` | Only digits |
| `lowercase()` | `lowercase` | All lowercase |
| `uppercase()` | `uppercase` | All uppercase |

### String — Content

| Factory | Error Code | Description |
|---------|-----------|-------------|
| `email()` | `invalid_format` | Valid email address |
| `url()` | `invalid_format` | Valid URL |
| `matches_regex(pattern)` | `invalid_format` | Matches regex pattern |

### Numeric — Range

| Factory | Error Code | Description |
|---------|-----------|-------------|
| `min(n)` | `min` | Value ≥ bound |
| `max(n)` | `max` | Value ≤ bound |
| `in_range(min, max)` | `out_of_range` | Value in inclusive range |
| `greater_than(n)` | `greater_than` | Value > bound |
| `less_than(n)` | `less_than` | Value < bound |
| `exclusive_range(min, max)` | `exclusive_range` | Value in exclusive range |

### Collection — Size

| Factory | Error Code | Description |
|---------|-----------|-------------|
| `min_size::<T>(n)` | `min_size` | Minimum element count |
| `max_size::<T>(n)` | `max_size` | Maximum element count |
| `exact_size::<T>(n)` | `exact_size` | Exact element count |
| `size_range::<T>(min, max)` | `size_range` | Element count in range |
| `not_empty_collection::<T>()` | `not_empty` | Must not be empty |

### Boolean

| Factory | Error Code | Description |
|---------|-----------|-------------|
| `is_true()` | `is_true` | Must be `true` |
| `is_false()` | `is_false` | Must be `false` |

### Nullable

| Factory | Error Code | Description |
|---------|-----------|-------------|
| `required()` | `required` | `Option` must be `Some` |
| `not_null()` | `required` | Alias for `required` |

### Network

| Factory | Error Code | Description |
|---------|-----------|-------------|
| `ipv4()` | `ipv4` | Valid IPv4 address |
| `ipv6()` | `ipv6` | Valid IPv6 address |
| `ip_addr()` | `ip_addr` | Valid IPv4 or IPv6 |
| `hostname()` | `hostname` | Valid hostname (RFC 1123) |

### Temporal

| Factory | Error Code | Description |
|---------|-----------|-------------|
| `date()` | `date` | ISO 8601 date (`YYYY-MM-DD`) |
| `time()` | `time` | Time (`HH:MM:SS[.sss]`) |
| `date_time()` | `datetime` | RFC 3339 datetime |
| `uuid()` | `uuid` | UUID format |

## Combinators

### Extension Methods

Every `Validate<T>` impl gets these via `ValidateExt`:

```rust
use nebula_validator::prelude::*;

let v = min_length(3)
    .and(max_length(20))     // AND — both must pass
    .or(contains("admin"))   // OR — at least one must pass
    .not();                  // NOT — must fail
```

### Factory Functions

| Factory | Error Code | Description |
|---------|-----------|-------------|
| `and(a, b)` | *(delegates)* | Both must pass |
| `and_all(vec)` / `all_of(vec)` | *(delegates)* | All must pass |
| `or(a, b)` | `or_failed` | At least one must pass |
| `or_any(vec)` / `any_of(vec)` | `or_any_failed` | At least one must pass |
| `not(v)` | `not_failed` | Inner must fail |
| `each(v)` | `each_failed` | Validate each element |
| `each_fail_fast(v)` | `each_failed` | Stop on first element error |
| `optional(v)` | *(delegates)* | Skip `None`, validate `Some` |
| `when(cond, v)` | *(delegates)* | Conditional validation |
| `unless(v, cond)` | *(delegates)* | Skip when condition true |
| `with_message(v, msg)` | *(delegates)* | Override error message |
| `with_code(v, code)` | *(delegates)* | Override error code |
| `lazy(\|\| v)` | *(delegates)* | Deferred initialization |
| `cached(v)` | *(delegates)* | Cache validation results |

### JSON Field Validation

Validate fields inside `serde_json::Value` with RFC 6901 JSON Pointer paths:

```rust
use nebula_validator::prelude::*;
use serde_json::json;

let v = json_field::<_, str>("/user/name", min_length(3));
assert!(v.validate(&json!({"user": {"name": "alice"}})).is_ok());

// Missing required path → "path_not_found" error with field "/user/name"
assert!(v.validate(&json!({"user": {}})).is_err());

// Optional field — missing/null is OK
let v = json_field_optional::<_, str>("/email", email());
assert!(v.validate(&json!({"name": "alice"})).is_ok());
```

## Custom Validators

### Using the `validator!` Macro

```rust
use nebula_validator::prelude::*;

validator!(EvenNumber, i32, "even", "must be even", |input| input % 2 == 0);

assert!(EvenNumber.validate(&4).is_ok());
assert!(EvenNumber.validate(&3).is_err());
```

### Manual Implementation

```rust
use nebula_validator::foundation::{Validate, ValidationError};

struct DivisibleBy(i32);

impl Validate<i32> for DivisibleBy {
    fn validate(&self, input: &i32) -> Result<(), ValidationError> {
        if input % self.0 == 0 {
            Ok(())
        } else {
            Err(ValidationError::new(
                "divisible_by",
                format!("must be divisible by {}", self.0),
            ))
        }
    }
}
```

## Error Model

`ValidationError` is an 80-byte struct with `Cow<'static, str>` fields for zero-alloc
on static strings:

```rust
use nebula_validator::foundation::ValidationError;

let err = ValidationError::new("min_length", "must be at least 3 characters")
    .with_field("user.name")       // auto-converts to RFC 6901: /user/name
    .with_param("min", "3")
    .with_severity(ErrorSeverity::Error)
    .with_help("usernames must be 3-20 alphanumeric characters");

// Nested errors for combinator trees
let parent = ValidationError::new("or_failed", "all alternatives failed")
    .with_nested_error(err);

// Serialize to JSON envelope
let json = parent.to_json_value();
```

## Proof Tokens

`Validated<T>` is a newtype wrapper that proves a value has been validated:

```rust
use nebula_validator::prelude::*;

fn process_username(name: Validated<String>) {
    // Type guarantees validation was performed
    println!("Processing: {}", name.as_ref());
}

let v = min_length(3).and(max_length(20)).and(alphanumeric());
let name: Validated<String> = v.validate_into("alice".to_string()).unwrap();
process_username(name);
```

## Architecture

```
src/
├── lib.rs              # Crate root, lints, re-exports
├── prelude.rs          # Single-import convenience module
├── error.rs            # ValidatorError (crate-level thiserror type)
├── proof.rs            # Validated<T> proof token
├── macros.rs           # validator! macro
├── foundation/
│   ├── traits.rs       # Validate<T>, ValidateExt<T>
│   ├── error.rs        # ValidationError, ValidationErrors, codes
│   ├── combinators.rs  # And, Or, Not, When trait-level types
│   ├── validatable.rs  # AsValidatable dynamic bridge
│   └── any.rs          # AnyValidator (type-erased)
├── validators/
│   ├── boolean.rs      # IsTrue, IsFalse
│   ├── content.rs      # Email, Url, MatchesRegex
│   ├── length.rs       # MinLength, MaxLength, ExactLength, etc.
│   ├── network.rs      # Ipv4, Ipv6, IpAddr, Hostname
│   ├── nullable.rs     # Required, NotNull
│   ├── pattern.rs      # Contains, StartsWith, Alphanumeric, etc.
│   ├── range.rs        # Min, Max, InRange, GreaterThan, etc.
│   ├── size.rs         # MinSize, MaxSize, ExactSize, etc.
│   └── temporal.rs     # Date, Time, DateTime, Uuid
└── combinators/
    ├── and.rs          # And, AndAll
    ├── or.rs           # Or, OrAny
    ├── not.rs          # Not
    ├── each.rs         # Each (collection iteration)
    ├── optional.rs     # Optional
    ├── when.rs / unless.rs  # Conditional
    ├── json_field.rs   # JSON Pointer field extraction
    ├── field.rs        # Struct field validation
    ├── nested.rs       # Nested/collection validation
    ├── message.rs      # WithCode, WithMessage
    ├── lazy.rs         # Lazy initialization
    ├── cached.rs       # Memoized validation
    └── factories.rs    # all_of, any_of convenience
```

## Contract & Compatibility

Error codes and serialization envelopes follow a **minor-additive** contract:

- **43 stable error codes** tracked in `tests/fixtures/compat/error_registry_v1.json`
- Field paths use **RFC 6901 JSON Pointer** format
- Contract tests in `tests/contract/` enforce code stability, envelope shape, and governance policy
- Deprecation and migration rules in `crates/validator/docs/MIGRATION.md`

## Testing

```bash
cargo test -p nebula-validator              # all tests (438+)
cargo test -p nebula-validator contract     # contract & governance tests
cargo clippy -p nebula-validator -- -D warnings
```

## License

MIT OR Apache-2.0
