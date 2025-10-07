# Validator Macros

Macros for creating validators with minimal boilerplate.

## Overview

This module provides both **declarative macros** (in this crate) and **derive macros** (in `nebula-validator-derive` crate) for creating validators quickly and idiomatically.

## Declarative Macros

### `validator!` - Create validators

The `validator!` macro generates a complete validator implementation with minimal boilerplate.

#### Syntax

```rust
validator! {
    [pub] struct ValidatorName {
        field1: Type1,
        field2: Type2,
    }
    impl {
        fn check(input: &InputType, field1: Type1, field2: Type2) -> bool {
            // validation logic
        }
        fn error(field1: Type1, field2: Type2) -> String {
            // error message
        }
        const DESCRIPTION: &str = "description";  // optional
    }
}
```

#### Examples

**Simple validator:**

```rust
use nebula_validator::validator;

validator! {
    pub struct MinLength {
        min: usize
    }
    impl {
        fn check(input: &str, min: usize) -> bool {
            input.len() >= min
        }
        fn error(min: usize) -> String {
            format!("Must be at least {} characters", min)
        }
        const DESCRIPTION: &str = "Validates minimum string length";
    }
}

// Use it
let validator = MinLength { min: 5 };
assert!(validator.validate("hello").is_ok());
assert!(validator.validate("hi").is_err());
```

**Complex validator:**

```rust
validator! {
    pub struct InRange {
        min: i32,
        max: i32
    }
    impl {
        fn check(input: &i32, min: i32, max: i32) -> bool {
            *input >= min && *input <= max
        }
        fn error(min: i32, max: i32) -> String {
            format!("Must be between {} and {}", min, max)
        }
        const DESCRIPTION: &str = "Validates numeric range";
    }
}
```

---

### `validate!` - Inline validation

Quick inline validation without creating a struct.

#### Examples

**Basic usage:**

```rust
use nebula_validator::validate;

let input = "hello@example.com";
validate!(input, |s: &str| s.contains('@'), "Must contain @")?;
```

**With custom error code:**

```rust
validate!(
    input,
    |s: &str| s.len() >= 5,
    "min_length",
    "String is too short"
)?;
```

**In a function:**

```rust
fn check_password(password: &str) -> Result<(), ValidationError> {
    validate!(password, |p: &str| p.len() >= 8, "Password too short")?;
    validate!(password, |p: &str| p.chars().any(|c| c.is_uppercase()), "Need uppercase")?;
    validate!(password, |p: &str| p.chars().any(|c| c.is_numeric()), "Need number")?;
    Ok(())
}
```

---

### `validator_fn!` - Function-based validators

Create validators from functions.

#### Examples

```rust
use nebula_validator::validator_fn;

validator_fn!(IsEven, |n: &i32| *n % 2 == 0, "Number must be even");
validator_fn!(IsPositive, |n: &i32| *n > 0, "Number must be positive");
validator_fn!(NotEmpty, |s: &str| !s.is_empty(), "String cannot be empty");

// Use them
let validator = IsEven::new();
assert!(validator.validate(&4).is_ok());
assert!(validator.validate(&3).is_err());
```

---

### `validator_const!` - Zero-size validators

Create zero-size validators for simple checks.

#### Examples

```rust
use nebula_validator::validator_const;

validator_const!(NotEmpty, |s: &str| !s.is_empty(), "Must not be empty");
validator_const!(IsAlpha, |s: &str| s.chars().all(|c| c.is_alphabetic()), "Must be alphabetic");

// These take no memory
assert_eq!(std::mem::size_of_val(&NotEmpty), 0);
```

---

### `compose!` - Compose with AND

Compose multiple validators with AND logic.

#### Examples

```rust
use nebula_validator::compose;

let validator = compose![
    min_length(5),
    max_length(20),
    alphanumeric(),
];

// Equivalent to:
let validator = min_length(5)
    .and(max_length(20))
    .and(alphanumeric());
```

---

### `any_of!` - Compose with OR

Compose multiple validators with OR logic.

#### Examples

```rust
use nebula_validator::any_of;

let validator = any_of![
    exact_length(5),
    exact_length(10),
    exact_length(15),
];

// Equivalent to:
let validator = exact_length(5)
    .or(exact_length(10))
    .or(exact_length(15));
```

---

## Derive Macros

These are in the separate `nebula-validator-derive` crate.

### `#[derive(Validate)]` - Struct validation

Automatically validate structs with field-level attributes.

#### Setup

```toml
[dependencies]
nebula-validator = { version = "2.0", features = ["derive"] }
```

#### Field Attributes

| Attribute | Description | Example |
|-----------|-------------|---------|
| `#[validate(skip)]` | Skip this field | `#[validate(skip)]` |
| `#[validate(min_length = N)]` | Min string length | `#[validate(min_length = 3)]` |
| `#[validate(max_length = N)]` | Max string length | `#[validate(max_length = 20)]` |
| `#[validate(email)]` | Email format | `#[validate(email)]` |
| `#[validate(url)]` | URL format | `#[validate(url)]` |
| `#[validate(regex = "...")]` | Regex pattern | `#[validate(regex = "^[0-9]+$")]` |
| `#[validate(range(min = M, max = N))]` | Numeric range | `#[validate(range(min = 18, max = 100))]` |
| `#[validate(nested)]` | Validate nested struct | `#[validate(nested)]` |
| `#[validate(custom = "fn")]` | Custom function | `#[validate(custom = "check_username")]` |

#### Examples

**Basic struct validation:**

```rust
use nebula_validator::Validate;

#[derive(Validate)]
struct User {
    #[validate(min_length = 3, max_length = 20)]
    username: String,

    #[validate(email)]
    email: String,

    #[validate(range(min = 18, max = 100))]
    age: u8,
}

let user = User {
    username: "john".to_string(),
    email: "john@example.com".to_string(),
    age: 25,
};

user.validate()?;  // Validates all fields
```

**Nested validation:**

```rust
#[derive(Validate)]
struct Address {
    #[validate(min_length = 1)]
    street: String,

    #[validate(min_length = 2)]
    city: String,

    #[validate(regex = "^[0-9]{5}$")]
    zipcode: String,
}

#[derive(Validate)]
struct Person {
    #[validate(min_length = 1)]
    name: String,

    #[validate(nested)]  // Recursively validate
    address: Address,
}
```

**Optional fields:**

```rust
#[derive(Validate)]
struct Profile {
    #[validate(min_length = 3)]
    username: String,

    #[validate(email)]
    email: Option<String>,  // None is valid

    #[validate(skip)]
    internal_id: uuid::Uuid,  // Not validated
}
```

**Custom validation:**

```rust
#[derive(Validate)]
struct Account {
    #[validate(min_length = 3)]
    username: String,

    #[validate(custom = "validate_username_available")]
    _username_check: (),
}

impl Account {
    fn validate_username_available(&self) -> Result<(), ValidationError> {
        if database_has_username(&self.username) {
            Err(ValidationError::new("username_taken", "Username already exists"))
        } else {
            Ok(())
        }
    }
}
```

---

### `#[derive(Validator)]` - Create validators

Generate TypedValidator implementation from a struct.

#### Attributes

- `#[validator(input = "Type")]` - Input type (required)
- `#[validator(output = "Type")]` - Output type (optional, default: `()`)
- `#[validator(error = "Type")]` - Error type (optional, default: `ValidationError`)

#### Required Methods

Your struct must implement:
- `validate_impl(&self, input: &Input) -> bool`
- `error_message(&self) -> String`

#### Examples

**Basic validator:**

```rust
use nebula_validator::Validator;

#[derive(Validator)]
#[validator(input = "str")]
struct MinLength {
    min: usize,
}

impl MinLength {
    fn validate_impl(&self, input: &str) -> bool {
        input.len() >= self.min
    }

    fn error_message(&self) -> String {
        format!("Must be at least {} characters", self.min)
    }
}

// Use it
let validator = MinLength { min: 5 };
assert!(validator.validate("hello").is_ok());
```

**With custom types:**

```rust
#[derive(Validator)]
#[validator(input = "i32", output = "()", error = "MyError")]
struct InRange {
    min: i32,
    max: i32,
}

impl InRange {
    fn validate_impl(&self, input: &i32) -> bool {
        *input >= self.min && *input <= self.max
    }

    fn error_message(&self) -> String {
        format!("Must be between {} and {}", self.min, self.max)
    }
}
```

---

## Comparison

| Feature | `validator!` | `validator_fn!` | `Derive(Validate)` | `Derive(Validator)` |
|---------|--------------|-----------------|--------------------|--------------------|
| Struct fields | ✅ | ❌ | ✅ | ✅ |
| Zero-size types | ❌ | ✅ | ❌ | ❌ |
| Field validation | ❌ | ❌ | ✅ | ❌ |
| Custom logic | ✅ | ✅ | ⚠️ Limited | ✅ |
| Nested validation | ❌ | ❌ | ✅ | ❌ |
| Boilerplate | Low | Minimal | Minimal | Low |

---

## Best Practices

### 1. Choose the Right Macro

```rust
// Simple validator with config? Use validator!
validator! {
    struct MinLength { min: usize }
    impl { /* ... */ }
}

// One-off check? Use validate!
validate!(input, |s| s.len() >= 5, "Too short")?;

// Struct with many fields? Use derive
#[derive(Validate)]
struct User { /* ... */ }

// Stateless validator? Use validator_const!
validator_const!(NotEmpty, |s: &str| !s.is_empty(), "Empty");
```

### 2. Combine Macros with Combinators

```rust
// Create base validators with macros
validator!(struct MinLen { min: usize } /* ... */);
validator!(struct MaxLen { max: usize } /* ... */);

// Compose with combinators
let length_check = MinLen { min: 5 }.and(MaxLen { max: 20 });
```

### 3. Use Derive for DTOs

```rust
// API request/response types
#[derive(Deserialize, Validate)]
struct CreateUserRequest {
    #[validate(min_length = 3, max_length = 20)]
    username: String,

    #[validate(email)]
    email: String,
}

// Validate on deserialization
let request: CreateUserRequest = serde_json::from_str(json)?;
request.validate()?;
```

### 4. Organize Validators

```rust
// validators/mod.rs
pub mod string {
    validator!(pub struct MinLength { /* ... */ });
    validator!(pub struct MaxLength { /* ... */ });
}

pub mod numeric {
    validator!(pub struct InRange { /* ... */ });
}

// Use
use crate::validators::string::MinLength;
```

---

## Performance Tips

### 1. Prefer `validator_const!` for Simple Checks

```rust
// ✅ Zero-size, no allocation
validator_const!(NotEmpty, |s| !s.is_empty(), "Empty");

// ❌ Has size, less efficient
validator! {
    struct NotEmpty {}
    impl { /* ... */ }
}
```

### 2. Cache Expensive Validators

```rust
validator! {
    struct DatabaseCheck { db: Database }
    impl { /* expensive check */ }
}

let validator = DatabaseCheck { db }.cached();  // Add caching
```

### 3. Validate Early, Fail Fast

```rust
#[derive(Validate)]
struct User {
    // Cheap checks first
    #[validate(min_length = 3)]    // O(1)
    username: String,

    // Expensive checks last
    #[validate(custom = "db_check")]  // I/O
    _db_check: (),
}
```

---

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;

    validator! {
        struct TestValidator { value: i32 }
        impl {
            fn check(input: &i32, value: i32) -> bool {
                *input == value
            }
            fn error(value: i32) -> String {
                format!("Must equal {}", value)
            }
        }
    }

    #[test]
    fn test_macro_generated_validator() {
        let validator = TestValidator { value: 42 };
        assert!(validator.validate(&42).is_ok());
        assert!(validator.validate(&43).is_err());
    }
}
```

---

## Troubleshooting

### Macro hygiene issues

```rust
// ❌ May have issues with imports
validator! { /* ... */ }

// ✅ Use fully qualified paths
validator! {
    struct MyValidator { /* ... */ }
    impl {
        fn check(input: &str, /* ... */) -> bool {
            ::std::ops::Not::not(input.is_empty())
        }
        /* ... */
    }
}
```

### Derive macro not found

```toml
# Make sure derive feature is enabled
[dependencies]
nebula-validator = { version = "2.0", features = ["derive"] }
```

### Complex validation logic

For very complex logic, implement TypedValidator manually:

```rust
// ❌ Hard to express in macros
validator! {
    struct Complex { /* ... */ }
    impl {
        fn check(/* ... */) -> bool {
            // Very complex logic
        }
    }
}

// ✅ Manual implementation
impl TypedValidator for Complex {
    // Full control
}
```

---

## Future Improvements

- [ ] `#[validate(async)]` for async field validation
- [ ] `#[validate(depends_on = "field")]` for cross-field validation
- [ ] `#[validate(if = "condition")]` for conditional validation
- [ ] Better error messages with spans
- [ ] IDE integration for attribute completion

---

## See Also

- [Core Traits](../core/README.md) - Core validator traits
- [Combinators](../combinators/README.md) - Composing validators
- [Validators](../validators/README.md) - Built-in validators