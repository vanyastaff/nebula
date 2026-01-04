# nebula-derive

Procedural macros for the Nebula workflow engine.

## Overview

This crate provides derive macros that simplify working with various Nebula components. Currently supports:

- **`#[derive(Validator)]`** - Automatic validator implementation for structs
- More derive macros coming soon: `Parameter`, `Action`, `Resource`

## Features

- 🔥 **Type-safe** - Compile-time validation of attributes
- 📦 **Zero runtime cost** - All code generated at compile time
- 🎯 **Ergonomic** - Clean, intuitive syntax
- 🔌 **Composable** - Works seamlessly with nebula-validator

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
nebula-derive = "0.1"
nebula-validator = "0.1"
```

## Usage

### `#[derive(Validator)]`

Automatically implements validation for your structs:

```rust
use nebula_derive::Validator;
use nebula_validator::prelude::*;

#[derive(Validator)]
struct UserRegistration {
    #[validate(min_length = 3, max_length = 20, alphanumeric)]
    username: String,

    #[validate(email)]
    email: String,

    #[validate(range(min = 18, max = 100))]
    age: u8,

    #[validate(min_length = 8)]
    password: String,
}

fn main() -> Result<(), ValidationErrors> {
    let user = UserRegistration {
        username: "alice123".to_string(),
        email: "alice@example.com".to_string(),
        age: 25,
        password: "secret123".to_string(),
    };

    // Validate all fields
    user.validate()?;

    println!("User registered successfully!");
    Ok(())
}
```

### Supported Validators

#### String Validators

- `#[validate(min_length = N)]` - Minimum length
- `#[validate(max_length = N)]` - Maximum length
- `#[validate(exact_length = N)]` - Exact length
- `#[validate(email)]` - Email format
- `#[validate(url)]` - URL format
- `#[validate(regex = "pattern")]` - Regex pattern
- `#[validate(alphanumeric)]` - Alphanumeric only
- `#[validate(contains = "substring")]` - Must contain substring
- `#[validate(starts_with = "prefix")]` - Must start with prefix
- `#[validate(ends_with = "suffix")]` - Must end with suffix

#### Text Validators (Zero Dependencies!)

- `#[validate(uuid)]` - RFC 4122 UUID format
- `#[validate(datetime)]` - ISO 8601 date/time format
- `#[validate(json)]` - Valid JSON string
- `#[validate(slug)]` - URL-friendly slug (lowercase, numbers, hyphens)
- `#[validate(hex)]` - Hexadecimal string
- `#[validate(base64)]` - Base64 encoded string

#### Numeric Validators

- `#[validate(range(min = N, max = M))]` - Range validation
- `#[validate(min = N)]` - Minimum value
- `#[validate(max = N)]` - Maximum value
- `#[validate(positive)]` - Must be positive
- `#[validate(negative)]` - Must be negative
- `#[validate(even)]` - Must be even
- `#[validate(odd)]` - Must be odd

#### Collection Validators

- `#[validate(min_size = N)]` - Minimum collection size
- `#[validate(max_size = N)]` - Maximum collection size
- `#[validate(unique)]` - All elements must be unique
- `#[validate(non_empty)]` - Must not be empty

#### Logical Validators

- `#[validate(required)]` - Field is required (not None)
- `#[validate(nested)]` - Validate nested struct
- `#[validate(custom = "function_name")]` - Custom validation function
- `#[validate(skip)]` - Skip validation for this field

#### 🌟 Universal Validator (Future-Proof!)

- `#[validate(expr = "validator_expression")]` - **Use ANY validator without updating nebula-derive!**

This is the key feature that solves your problem! When you add new validators to `nebula-validator`, you can use them immediately without modifying `nebula-derive`:

```rust
#[derive(Validator)]
struct Form {
    // New validator? No problem! Use expr:
    #[validate(expr = "my_new_validator()")]
    field1: String,

    // Complex chain? Works too!
    #[validate(expr = "min_length(5).and(custom_validator()).cached()")]
    field2: String,

    // External validators? Sure!
    #[validate(expr = "external_crate::special_validator()")]
    field3: String,
}
```

### Examples

#### Multiple Validators

Combine multiple validators on a single field:

```rust
#[derive(Validator)]
struct ProductForm {
    #[validate(min_length = 3, max_length = 50, alphanumeric)]
    product_name: String,
}
```

#### Text Validators

Use the new zero-dependency text validators:

```rust
#[derive(Validator)]
struct ApiRequest {
    #[validate(uuid)]
    request_id: String,

    #[validate(datetime)]
    timestamp: String,

    #[validate(json)]
    payload: String,

    #[validate(slug)]
    resource_name: String,

    #[validate(hex)]
    checksum: String,

    #[validate(base64)]
    encoded_data: String,
}

fn main() -> Result<(), ValidationErrors> {
    let request = ApiRequest {
        request_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        timestamp: "2024-01-15T09:30:00Z".to_string(),
        payload: r#"{"user": "alice"}"#.to_string(),
        resource_name: "my-resource".to_string(),
        checksum: "deadbeef".to_string(),
        encoded_data: "SGVsbG8gV29ybGQ=".to_string(),
    };

    request.validate()?;
    println!("Request validated successfully!");
    Ok(())
}
```

#### Custom Validation

Use custom validation functions:

```rust
fn validate_username(username: &str) -> Result<(), ValidationError> {
    if username.starts_with("admin") {
        Err(ValidationError::new("invalid_username", "Cannot start with 'admin'"))
    } else {
        Ok(())
    }
}

#[derive(Validator)]
struct UserForm {
    #[validate(min_length = 3, custom = "validate_username")]
    username: String,
}
```

#### Nested Validation

Validate nested structures:

```rust
#[derive(Validator)]
struct Address {
    #[validate(min_length = 1)]
    street: String,

    #[validate(min_length = 1)]
    city: String,
}

#[derive(Validator)]
struct User {
    #[validate(email)]
    email: String,

    #[validate(nested)]
    address: Address,
}
```

#### Skip Validation

Skip certain fields:

```rust
#[derive(Validator)]
struct FormData {
    #[validate(min_length = 3)]
    validated_field: String,

    #[validate(skip)]
    internal_field: String,  // Not validated
}
```

#### Universal Expression (Future-Proof)

Use ANY validator, even ones not yet supported by derive:

```rust
#[derive(Validator)]
struct FutureProof {
    // Built-in syntax (convenient)
    #[validate(min_length = 5, max_length = 20)]
    username: String,

    // Universal expr (flexible - works with ANY validator!)
    #[validate(expr = "my_brand_new_validator()")]
    custom: String,

    // Complex compositions
    #[validate(expr = "min_length(3).and(alphanumeric()).or(exact_length(0))")]
    flexible: String,
}
```

**Why use `expr`?**
- ✅ Add new validators to `nebula-validator` without updating `nebula-derive`
- ✅ Use third-party validators from external crates
- ✅ Build complex validator chains inline
- ✅ Future-proof your code

## Architecture

The crate is structured to support multiple derive macros:

```
nebula-derive/
├── src/
│   ├── lib.rs           # Public API
│   ├── utils.rs         # Shared utilities
│   └── validator/       # Validator derive implementation
│       ├── mod.rs
│       ├── parse.rs     # Attribute parsing
│       └── generate.rs  # Code generation
└── tests/
    └── validator_derive.rs
```

## Future Derives

Planned derive macros:

- `#[derive(Parameter)]` - Parameter builder generation
- `#[derive(Action)]` - Action trait implementation
- `#[derive(Resource)]` - Resource management

## License

Licensed under the same license as the Nebula project.

## Contributing

Contributions welcome! Please see the main Nebula repository for guidelines.
