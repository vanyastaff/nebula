# nebula-validator

Production-ready validation framework for the Nebula workflow engine with advanced combinators and compositional design.

## Features

- **Comprehensive Validators**: 60+ built-in validators for strings, numbers, collections, and network data
- **Compositional Design**: Chain validators with `and()`, `or()`, `not()` combinators
- **Type-Safe**: Generic validators with strong typing
- **Zero-Cost Abstractions**: Compiled validators with minimal runtime overhead
- **Extensible**: Easy to create custom validators
- **Advanced Combinators**: Field validation, nested structures, conditional logic, caching

## Quick Start

```rust
use nebula_validator::validators::string::{min_length, max_length};
use nebula_validator::combinators::and;
use nebula_validator::core::Validator;

fn main() {
    let validator = and(min_length(5), max_length(20));

    match validator.validate("hello") {
        Ok(_) => println!("Valid!"),
        Err(e) => println!("Error: {}", e),
    }
}
```

## Validator Categories

### String Validators

#### Length

| Validator | Description |
|-----------|-------------|
| `min_length(n)` | Minimum string length |
| `max_length(n)` | Maximum string length |
| `exact_length(n)` | Exact string length |
| `length_range(min, max)` | Length in range |
| `not_empty()` | String not empty |

#### Pattern

| Validator | Description |
|-----------|-------------|
| `contains(s)` | Contains substring |
| `starts_with(s)` | Starts with prefix |
| `ends_with(s)` | Ends with suffix |
| `matches_regex(pattern)` | Matches regex |
| `alphanumeric()` | Alphanumeric characters |
| `alphabetic()` | Alphabetic characters only |
| `ascii()` | ASCII characters only |
| `lowercase()` | Lowercase characters only |
| `uppercase()` | Uppercase characters only |

#### Content

| Validator | Description |
|-----------|-------------|
| `Email` | Valid email address |
| `Url` | Valid URL |

#### Format Validators (Builder Pattern)

| Validator | Description | Key Methods |
|-----------|-------------|-------------|
| `Uuid::new()` | UUID validation | `.version(n)`, `.lowercase_only()`, `.allow_braces()` |
| `DateTime::new()` | DateTime (ISO 8601) | `.require_time()`, `.require_timezone()` |
| `Json::new()` | JSON validation | `.objects_only()`, `.max_depth(n)` |
| `Slug::new()` | URL slug | `.min_length(n)`, `.max_length(n)` |
| `Hex::new()` | Hexadecimal | `.require_prefix()`, `.lowercase_only()` |
| `Base64::new()` | Base64 | `.url_safe()`, `.require_padding()` |
| `Phone::new()` | Phone number | `.country("US")`, `.require_country_code()` |
| `CreditCard::new()` | Credit card | `.only_visa()`, `.only_mastercard()` |
| `Iban::new()` | IBAN | `.allow_spaces()` |
| `SemVer::new()` | Semantic version | `.require_patch()`, `.allow_prerelease()` |
| `Password::new()` | Password strength | `.min_length(n)`, `.require_uppercase()`, `.require_digit()` |

### Numeric Validators

#### Range

| Validator | Description |
|-----------|-------------|
| `min(n)` | Minimum value (>=) |
| `max(n)` | Maximum value (<=) |
| `in_range(min, max)` | Value in range (inclusive) |
| `greater_than(n)` | Strictly greater (>) |
| `less_than(n)` | Strictly less (<) |
| `exclusive_range(min, max)` | Value in range (exclusive) |

#### Properties

| Validator | Description |
|-----------|-------------|
| `positive()` | Must be > 0 |
| `negative()` | Must be < 0 |
| `non_zero()` | Must not be 0 |
| `even()` | Must be even |
| `odd()` | Must be odd |
| `power_of_two()` | Must be power of 2 |

#### Divisibility

| Validator | Description |
|-----------|-------------|
| `divisible_by(n)` | Divisible by n |
| `multiple_of(n)` | Alias for divisible_by |

#### Float

| Validator | Description |
|-----------|-------------|
| `finite()` | Not NaN or infinity |
| `not_nan()` | Not NaN (infinity allowed) |
| `decimal_places(n)` | Max decimal places |

#### Percentage

| Validator | Description |
|-----------|-------------|
| `percentage()` | Value in 0.0..1.0 |
| `percentage_100()` | Value in 0..100 |

### Collection Validators

#### Size

| Validator | Description |
|-----------|-------------|
| `min_size(n)` | Minimum collection size |
| `max_size(n)` | Maximum collection size |
| `exact_size(n)` | Exact collection size |
| `size_range(min, max)` | Size in range |
| `not_empty_collection()` | Collection not empty |

#### Elements

| Validator | Description |
|-----------|-------------|
| `all(validator)` | All elements must pass |
| `any(validator)` | At least one must pass |
| `none(validator)` | No element must pass |
| `count(validator, n)` | Exactly n elements pass |
| `at_least_count(validator, n)` | At least n elements pass |
| `at_most_count(validator, n)` | At most n elements pass |
| `first(validator)` | First element must pass |
| `last(validator)` | Last element must pass |
| `nth(validator, n)` | Nth element must pass |
| `unique()` | All elements unique |
| `contains_element(v)` | Contains specific element |
| `contains_all(vec)` | Contains all elements |
| `contains_any(vec)` | Contains at least one |
| `sorted()` | Sorted ascending |
| `sorted_descending()` | Sorted descending |

#### Structure

| Validator | Description |
|-----------|-------------|
| `has_key(k)` | Map has key |

### Network Validators

| Validator | Description | Key Methods |
|-----------|-------------|-------------|
| `IpAddress::new()` | IP address | `.v4_only()`, `.v6_only()`, `.private_only()` |
| `Port::new()` | Port number | `.well_known_only()`, `.registered_only()` |
| `MacAddress::new()` | MAC address | `.colon_only()`, `.hyphen_only()` |

### Logical Validators

| Validator | Description |
|-----------|-------------|
| `required()` | Value must not be None |
| `not_null()` | Value must not be None |
| `is_true()` | Boolean must be true |
| `is_false()` | Boolean must be false |

## Combinators

### Basic Combinators

```rust
use nebula_validator::combinators::*;

// AND - all must pass
let v = and(min_length(3), max_length(20));

// OR - at least one must pass
let v = or(contains("@"), contains("."));

// NOT - must fail
let v = not(contains("admin"));

// Optional - validate Option<T>
let v = optional(min_length(5));
```

### Advanced Combinators

| Combinator | Description |
|-----------|-------------|
| `and_all(vec)` | All validators must pass |
| `or_any(vec)` | At least one must pass |
| `when(condition, validator)` | Conditional validation |
| `unless(validator, condition)` | Skip when condition true |
| `each(validator)` | Validate each element |
| `each_fail_fast(validator)` | Stop on first error |
| `with_message(validator, msg)` | Custom error message |
| `with_code(validator, code)` | Custom error code |
| `lazy(|| validator)` | Deferred initialization |
| `cached(validator)` | Cache validation results |
| `field(name, extractor, validator)` | Field-specific validation |

### Combinator Examples

```rust
use nebula_validator::combinators::*;

// Validate each element
let v = each(positive());
v.validate(&[1, 2, 3]); // Ok

// Conditional validation
let v = when(|s: &str| s.len() > 5, contains("@"));

// Skip validation for admins
let v = unless(min_length(10), |s: &str| s.starts_with("admin:"));

// Custom error message
let v = with_message(min_length(8), "Password too short");
```

## Usage Examples

### Username Validation

```rust
use nebula_validator::validators::string::*;
use nebula_validator::combinators::and;
use nebula_validator::core::Validator;

let username = and(
    min_length(3),
    and(max_length(20), alphanumeric())
);

assert!(username.validate("alice123").is_ok());
assert!(username.validate("ab").is_err());
```

### Password Validation

```rust
use nebula_validator::validators::string::Password;
use nebula_validator::core::Validator;

let password = Password::new()
    .min_length(8)
    .require_uppercase()
    .require_lowercase()
    .require_digit()
    .require_special();

assert!(password.validate("SecureP@ss1").is_ok());
```

### Numeric Validation

```rust
use nebula_validator::validators::numeric::*;
use nebula_validator::combinators::and;

// Age: positive integer 0-120
let age = and(positive(), in_range(0, 120));

// Price: positive with max 2 decimal places
let price = and(positive(), decimal_places(2));

// Port: well-known ports only
let port = and(min(1), max(1023));
```

### Collection Validation

```rust
use nebula_validator::validators::collection::*;
use nebula_validator::validators::numeric::positive;
use nebula_validator::combinators::and;

// Tags: 1-10 unique strings
let tags = and(size_range(1, 10), unique());

// Scores: all positive, sorted
let scores = and(all(positive()), sorted());

// Require at least 2 passing grades
let grades = at_least_count(|&g| g >= 60, 2);
```

### UUID Validation

```rust
use nebula_validator::validators::string::Uuid;
use nebula_validator::core::Validator;

let uuid = Uuid::new()
    .version(4)
    .lowercase_only();

assert!(uuid.validate("550e8400-e29b-41d4-a716-446655440000").is_ok());
```

### Credit Card Validation

```rust
use nebula_validator::validators::string::CreditCard;
use nebula_validator::core::Validator;

let card = CreditCard::new()
    .only_visa()
    .allow_spaces();

assert!(card.validate("4111 1111 1111 1111").is_ok());
```

## Custom Validators

Implement the `Validator` trait:

```rust
use nebula_validator::core::{Validator, ValidationError, ValidatorMetadata};

struct DivisibleBy {
    divisor: i32,
}

impl Validator for DivisibleBy {
    type Input = i32;

    fn validate(&self, input: &i32) -> Result<(), ValidationError> {
        if input % self.divisor == 0 {
            Ok(())
        } else {
            Err(ValidationError::new(
                "divisible",
                format!("Must be divisible by {}", self.divisor)
            ))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("DivisibleBy")
    }
}
```

## Architecture

```
nebula-validator/
├── core/                  # Core traits and types
│   ├── traits.rs          # Validator trait
│   ├── error.rs           # ValidationError
│   ├── context.rs         # ValidationContext
│   ├── metadata.rs        # ValidatorMetadata
│   └── refined.rs         # Refined types
├── validators/
│   ├── string/            # String validators
│   │   ├── length.rs      # Length validators
│   │   ├── pattern.rs     # Pattern validators
│   │   ├── content.rs     # Email, URL
│   │   ├── uuid.rs        # UUID
│   │   ├── datetime.rs    # DateTime
│   │   ├── json.rs        # JSON
│   │   ├── phone.rs       # Phone
│   │   ├── credit_card.rs # Credit card
│   │   ├── iban.rs        # IBAN
│   │   ├── password.rs    # Password
│   │   └── ...
│   ├── numeric/           # Numeric validators
│   │   ├── range.rs       # Range validators
│   │   ├── properties.rs  # Properties (positive, even, etc.)
│   │   ├── divisibility.rs# Divisibility
│   │   ├── float.rs       # Float-specific
│   │   └── percentage.rs  # Percentage
│   ├── collection/        # Collection validators
│   │   ├── size.rs        # Size validators
│   │   ├── elements.rs    # Element validators
│   │   └── structure.rs   # Structure validators
│   ├── network/           # Network validators
│   │   ├── ip_address.rs  # IP address
│   │   ├── port.rs        # Port
│   │   └── mac_address.rs # MAC address
│   ├── logical/           # Logical validators
│   └── value/             # Value type validators
└── combinators/           # Combinators
    ├── and.rs             # AND combinator
    ├── or.rs              # OR combinator
    ├── not.rs             # NOT combinator
    ├── optional.rs        # Optional validation
    ├── when.rs            # Conditional (when)
    ├── unless.rs          # Conditional (unless)
    ├── each.rs            # Collection iteration
    ├── message.rs         # Custom messages
    ├── lazy.rs            # Lazy initialization
    ├── cached.rs          # Caching
    ├── field.rs           # Field validation
    ├── nested.rs          # Nested validation
    ├── map.rs             # Value transformation
    └── optimizer.rs       # Validator optimization
```

## Best Practices

### 1. Order Validators by Cost

```rust
// Cheap checks first
let v = and(
    min_length(5),              // O(1)
    matches_regex(r"...").unwrap()  // O(n)
);
```

### 2. Use Builders for Complex Validation

```rust
let password = Password::new()
    .min_length(12)
    .require_uppercase()
    .require_digit()
    .require_special()
    .no_common_passwords();
```

### 3. Reuse Validators

```rust
let email = email();
let strong_password = Password::new().min_length(12).require_special();

// Use across application
validate_user(&email, &strong_password);
```

### 4. Use Custom Messages for UX

```rust
let v = with_message(
    min_length(8),
    "Password must be at least 8 characters"
);
```

## Testing

```bash
cargo test -p nebula-validator
```

## License

MIT OR Apache-2.0
