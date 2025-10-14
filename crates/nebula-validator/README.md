# nebula-validator

[![Crates.io](https://img.shields.io/crates/v/nebula-validator.svg)](https://crates.io/crates/nebula-validator)
[![Documentation](https://docs.rs/nebula-validator/badge.svg)](https://docs.rs/nebula-validator)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)

Production-ready validation framework for the Nebula workflow engine with advanced combinators and compositional design.

## Features

- 🔍 **Comprehensive Validators**: Built-in validators for strings, numbers, collections, text formats, and network data
- 🧩 **Compositional Design**: Chain validators with `and()`, `or()`, `not()` combinators
- 🏗️ **Type-Safe**: Generic validators with strong typing
- ⚡ **Zero-Cost Abstractions**: Compiled validators with minimal runtime overhead
- 🔧 **Extensible**: Easy to create custom validators
- 🎨 **Advanced Combinators**: Field validation, nested structures, conditional logic, caching

## Quick Start

```rust
use nebula_validator::validators::string::*;
use nebula_validator::combinators::and;
use nebula_validator::core::TypedValidator;

fn main() {
    // Simple validation
    let validator = and(min_length(5), max_length(20));

    match validator.validate("hello") {
        Ok(_) => println!("Valid!"),
        Err(e) => println!("Error: {}", e),
    }
}
```

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
nebula-validator = "0.1"
```

## Core Concepts

### TypedValidator

A `TypedValidator<T>` validates values of type `T`:

```rust
pub trait TypedValidator<T> {
    fn validate(&self, value: T) -> Result<(), ValidationError>;
}
```

### Composition

Validators can be composed using logical combinators:

```rust
use nebula_validator::combinators::{and, or, not};
use nebula_validator::validators::string::*;

// AND: all must pass
let validator = and(min_length(3), max_length(20));

// OR: at least one must pass
let validator = or(contains("@"), contains("."));

// NOT: must NOT pass
let validator = not(contains("forbidden"));
```

## Validator Categories

### String Validators

#### Length

| Validator | Description | Example |
|-----------|-------------|---------|
| `min_length(n)` | Minimum string length | `min_length(5)` |
| `max_length(n)` | Maximum string length | `max_length(100)` |
| `exact_length(n)` | Exact string length | `exact_length(10)` |
| `length_range(min, max)` | Length in range | `length_range(5, 20)` |

#### Pattern

| Validator | Description | Example |
|-----------|-------------|---------|
| `contains(s)` | Contains substring | `contains("@")` |
| `starts_with(s)` | Starts with prefix | `starts_with("http")` |
| `ends_with(s)` | Ends with suffix | `ends_with(".com")` |
| `matches_regex(pattern)` | Matches regex | `matches_regex(r"^\d{3}-\d{4}$")` |
| `alphanumeric()` | Alphanumeric characters | `alphanumeric()` |
| `alphabetic()` | Alphabetic characters only | `alphabetic()` |

#### Content

| Validator | Description | Example |
|-----------|-------------|---------|
| `email()` | Valid email address | `email()` |
| `url()` | Valid URL | `url()` |

### Numeric Validators

#### Range

| Validator | Description | Example |
|-----------|-------------|---------|
| `min(n)` | Minimum value | `min(0.0)` |
| `max(n)` | Maximum value | `max(100.0)` |
| `in_range(min, max)` | Value in range | `in_range(0.0, 100.0)` |

#### Properties

| Validator | Description | Example |
|-----------|-------------|---------|
| `positive()` | Must be positive | `positive()` |
| `negative()` | Must be negative | `negative()` |
| `even()` | Must be even | `even()` |
| `odd()` | Must be odd | `odd()` |

### Collection Validators

#### Size

| Validator | Description | Example |
|-----------|-------------|---------|
| `min_size(n)` | Minimum size | `min_size(1)` |
| `max_size(n)` | Maximum size | `max_size(100)` |
| `exact_size(n)` | Exact size | `exact_size(5)` |
| `not_empty_collection()` | Collection not empty | `not_empty_collection()` |

#### Structure

| Validator | Description | Example |
|-----------|-------------|---------|
| `has_key(k)` | Object has key | `has_key("name")` |

#### Elements

| Validator | Description | Example |
|-----------|-------------|---------|
| `unique()` | All elements unique | `unique()` |
| `all(validator)` | All elements match | `all(min_length(2))` |
| `any(validator)` | At least one matches | `any(contains("test"))` |
| `contains_element(v)` | Contains specific element | `contains_element("item")` |

### Text Format Validators

| Validator | Description | Builder Methods |
|-----------|-------------|-----------------|
| `Uuid::new()` | UUID validation | `.uppercase_only()`, `.lowercase_only()`, `.allow_braces()`, `.version(n)` |
| `DateTime::new()` | DateTime validation | `.require_time()`, `.require_timezone()`, `.no_milliseconds()` |
| `Json::new()` | JSON validation | `.objects_only()`, `.max_depth(n)` |
| `Slug::new()` | URL slug validation | `.min_length(n)`, `.max_length(n)`, `.allow_consecutive_hyphens()` |
| `Hex::new()` | Hexadecimal validation | `.no_prefix()`, `.require_prefix()`, `.min_length(n)`, `.lowercase_only()` |
| `Base64::new()` | Base64 validation | `.url_safe()`, `.require_padding()`, `.allow_whitespace()` |

### Network Validators

| Validator | Description | Builder Methods |
|-----------|-------------|-----------------|
| `IpAddress::new()` | IP address validation | `.v4_only()`, `.v6_only()` |
| `Port::new()` | Port validation | `.well_known_only()`, `.registered_only()`, `.dynamic_only()` |
| `MacAddress::new()` | MAC address validation | `.colon_only()`, `.hyphen_only()`, `.dot_only()`, `.no_separator_only()` |

### Logical Validators

| Validator | Description | Example |
|-----------|-------------|---------|
| `required()` | Value must not be None | `required()` |
| `not_null()` | Value must not be None | `not_null()` |

## Combinators

### Basic Combinators

```rust
use nebula_validator::combinators::*;

// AND combinator
let validator = and(min_length(3), max_length(20));

// OR combinator
let validator = or(contains("@"), contains("."));

// NOT combinator
let validator = not(contains("admin"));

// Optional combinator - wraps validator for Option<T>
let validator = optional(min_length(5));
```

### Advanced Combinators

```rust
// Field validation - validate specific fields in structs
let validator = field("email", and(contains("@"), contains(".")));

// Nested validation - validate nested structures
let validator = nested("address", has_key("street"));

// Conditional validation - apply validators conditionally
let validator = when(|ctx| ctx.is_premium(), email());

// Cached validation - cache expensive validation results
let validator = cached(expensive_validator, Duration::from_secs(60));

// Error mapping - customize error messages
let validator = map_error(min_length(8), |_| "Password too short");
```

## Usage Examples

### Username Validation

```rust
use nebula_validator::validators::string::*;
use nebula_validator::combinators::and;
use nebula_validator::core::TypedValidator;

let username_validator = and(
    min_length(3),
    and(max_length(20), alphanumeric())
);

assert!(username_validator.validate("alice123").is_ok());
assert!(username_validator.validate("ab").is_err()); // Too short
```

### Email Validation

```rust
use nebula_validator::validators::string::email;

let email_validator = email();

assert!(email_validator.validate("user@example.com").is_ok());
assert!(email_validator.validate("invalid").is_err());
```

### Password Validation

```rust
use nebula_validator::validators::string::*;
use nebula_validator::combinators::and;

let password_validator = and(
    min_length(8),
    and(max_length(128), matches_regex(r"[A-Z]").unwrap())
);

assert!(password_validator.validate("SecurePass123").is_ok());
```

### Numeric Range Validation

```rust
use nebula_validator::validators::numeric::*;
use nebula_validator::combinators::and;

let age_validator = and(positive(), in_range(0.0, 120.0));

assert!(age_validator.validate(25).is_ok());
assert!(age_validator.validate(-5).is_err());
assert!(age_validator.validate(150).is_err());
```

### UUID Validation

```rust
use nebula_validator::validators::text::Uuid;
use nebula_validator::core::TypedValidator;

let uuid_validator = Uuid::new().lowercase_only();

assert!(uuid_validator.validate("550e8400-e29b-41d4-a716-446655440000").is_ok());
```

### IP Address Validation

```rust
use nebula_validator::validators::network::IpAddress;
use nebula_validator::core::TypedValidator;

let ip_validator = IpAddress::new();

assert!(ip_validator.validate("192.168.1.1").is_ok());
assert!(ip_validator.validate("2001:0db8::1").is_ok());
```

### Port Validation

```rust
use nebula_validator::validators::network::Port;
use nebula_validator::core::TypedValidator;

let port_validator = Port::new().well_known_only(); // Ports 0-1023

assert!(port_validator.validate("80").is_ok());
assert!(port_validator.validate("8080").is_err()); // Not well-known
```

### Collection Validation

```rust
use nebula_validator::validators::collection::*;
use nebula_validator::combinators::and;

let tags_validator = and(
    min_size(1),
    and(max_size(10), unique())
);

assert!(tags_validator.validate(&vec!["rust", "async", "validator"]).is_ok());
assert!(tags_validator.validate(&vec!["rust", "rust"]).is_err()); // Not unique
```

### Builder Pattern

```rust
use nebula_validator::validators::text::*;
use nebula_validator::core::TypedValidator;

// UUID with specific format
let uuid_validator = Uuid::new()
    .lowercase_only()
    .allow_braces()
    .version(4);

// Hexadecimal with constraints
let hex_validator = Hex::new()
    .require_prefix()
    .lowercase_only()
    .min_length(8);

// DateTime with requirements
let datetime_validator = DateTime::new()
    .require_time()
    .require_timezone();
```

## Examples

The crate includes comprehensive examples:

```bash
# Basic validation examples
cargo run --example basic_usage

# Advanced combinators
cargo run --example combinators
```

## Testing

Run the test suite:

```bash
# Run all tests
cargo test -p nebula-validator

# Run specific module tests
cargo test -p nebula-validator string::tests
cargo test -p nebula-validator numeric::tests
cargo test -p nebula-validator collection::tests
```

## Architecture

```
nebula-validator/
├── core/                  # Core traits and types
│   ├── traits.rs          # TypedValidator trait
│   ├── error.rs           # ValidationError
│   ├── context.rs         # ValidationContext
│   ├── state.rs           # ValidationState
│   ├── metadata.rs        # Metadata support
│   └── refined.rs         # Refined types
├── validators/            # Validator implementations
│   ├── logical/           # required, not_null
│   │   ├── nullable.rs    # Nullable validators
│   │   └── boolean.rs     # Boolean validators
│   ├── string/            # String validators
│   │   ├── length.rs      # min_length, max_length, etc.
│   │   ├── content.rs     # email, url, regex
│   │   └── pattern.rs     # contains, alphanumeric, etc.
│   ├── numeric/           # Numeric validators
│   │   ├── range.rs       # min, max, in_range
│   │   └── properties.rs  # positive, even, odd, etc.
│   ├── collection/        # Collection validators
│   │   ├── size.rs        # min_size, max_size, exact_size
│   │   ├── structure.rs   # has_key
│   │   └── elements.rs    # unique, all, any, contains_element
│   ├── text/              # Text format validators (builders)
│   │   ├── uuid.rs        # Uuid::new()
│   │   ├── datetime.rs    # DateTime::new()
│   │   ├── json.rs        # Json::new()
│   │   ├── slug.rs        # Slug::new()
│   │   ├── hex.rs         # Hex::new()
│   │   └── base64.rs      # Base64::new()
│   └── network/           # Network validators (builders)
│       ├── ip_address.rs  # IpAddress::new()
│       ├── port.rs        # Port::new()
│       └── mac_address.rs # MacAddress::new()
└── combinators/           # Combinators
    ├── and.rs             # Logical AND
    ├── or.rs              # Logical OR
    ├── not.rs             # Logical NOT
    ├── optional.rs        # Optional validation
    ├── field.rs           # Field-specific validation
    ├── nested.rs          # Nested validation
    ├── when.rs            # Conditional validation
    ├── map.rs             # Value transformation
    ├── cached.rs          # Result caching
    ├── error.rs           # Error mapping
    └── optimizer.rs       # Validator optimization
```

## Design Patterns

### Function-Style Validators

Simple validators are provided as functions:

```rust
use nebula_validator::validators::string::*;

min_length(5)      // Function returning validator
max_length(100)    // Function returning validator
contains("@")      // Function returning validator
```

### Builder-Style Validators

Complex validators use the builder pattern:

```rust
use nebula_validator::validators::text::*;

Uuid::new()
    .lowercase_only()
    .allow_braces()
    .version(4)
```

### Composition

Combine validators using combinators:

```rust
use nebula_validator::combinators::and;

and(
    min_length(8),
    and(max_length(128), contains("@"))
)
```

## Best Practices

### 1. Compose Simple Validators

```rust
let strong_password = and(
    min_length(8),
    and(
        max_length(128),
        and(
            matches_regex(r"[A-Z]").unwrap(),
            matches_regex(r"\d").unwrap()
        )
    )
);
```

### 2. Order by Cost

Put cheap validators first:

```rust
// Cheap check first
let validator = and(
    min_length(5),           // Fast: just length
    matches_regex(r"...").unwrap()  // Expensive: regex
);
```

### 3. Use Builders for Complex Cases

```rust
let hex_validator = Hex::new()
    .require_prefix()
    .lowercase_only()
    .min_length(8)
    .max_length(64);
```

### 4. Reuse Validators

```rust
let email_validator = email();
let username_validator = and(min_length(3), max_length(20));

// Reuse across your application
```

## Custom Validators

Implement `TypedValidator<T>` for custom validation:

```rust
use nebula_validator::core::{TypedValidator, ValidationError};

struct DomainValidator {
    allowed_domains: Vec<String>,
}

impl TypedValidator<&str> for DomainValidator {
    fn validate(&self, value: &str) -> Result<(), ValidationError> {
        if self.allowed_domains.iter().any(|d| value.ends_with(d)) {
            Ok(())
        } else {
            Err(ValidationError::new("Invalid domain"))
        }
    }
}
```

## Integration with nebula-derive

Use derive macros for struct validation (if available):

```rust
use nebula_derive::Validate;

#[derive(Validate)]
struct User {
    #[validate(min_length = 3, max_length = 20)]
    username: String,

    #[validate(email)]
    email: String,

    #[validate(positive, max = 120)]
    age: i32,
}
```

## Contributing

Contributions are welcome! Please read our [Contributing Guide](../../CONTRIBUTING.md).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE))
- MIT License ([LICENSE-MIT](../../LICENSE-MIT))

at your option.

## Related Crates

- [nebula-derive](../nebula-derive) - Derive macros for validation
- [nebula-parameter](../nebula-parameter) - Parameter handling with validation
- [nebula-log](../nebula-log) - Logging infrastructure

## Status

✅ **Production Ready** - Fully tested and documented, ready for use in production environments.

**Version**: 0.1.0
**API Style**: Function-style validators + Builder pattern for complex types
**Examples**: 2 comprehensive examples
**Documentation**: Complete with real usage examples
