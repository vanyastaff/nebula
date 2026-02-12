# Quickstart: Validator Serde Bridge

**Feature**: 009-validator-serde-bridge
**Date**: 2026-02-11

## Enable the Feature

```toml
# In Cargo.toml
[dependencies]
nebula-validator = { path = "../nebula-validator", features = ["serde-json"] }
```

## Basic Usage: Validate a JSON Value

```rust
use serde_json::json;
use nebula_validator::validators::string::MinLength;
use nebula_validator::core::Validate;

let value = json!("hello world");

// validate_any automatically bridges Value → &str
let validator = MinLength { min: 3 };
assert!(validator.validate_any(&value).is_ok());

let short = json!("hi");
assert!(validator.validate_any(&short).is_err());
```

## Numeric Validation

```rust
use serde_json::json;
use nebula_validator::validators::numeric::InRange;
use nebula_validator::core::Validate;

let port = json!(8080);
let validator = InRange { min: 1i64, max: 65535i64 };
assert!(validator.validate_any(&port).is_ok());

// Type mismatch: string is not a number
let not_a_number = json!("8080");
assert!(validator.validate_any(&not_a_number).is_err());
```

## Field Path Validation

```rust
use serde_json::json;
use nebula_validator::combinators::json_field::{json_field, json_field_optional};
use nebula_validator::validators::string::MinLength;
use nebula_validator::validators::numeric::InRange;
use nebula_validator::core::Validate;

let config = json!({
    "server": {
        "host": "localhost",
        "port": 8080,
        "tags": ["web", "api"]
    }
});

// Validate nested field by path
let host_validator = json_field("server.host", MinLength { min: 1 }).unwrap();
assert!(host_validator.validate(&config).is_ok());

// Validate with array index
let first_tag = json_field("server.tags[0]", MinLength { min: 1 }).unwrap();
assert!(first_tag.validate(&config).is_ok());

// Optional fields pass when missing
let optional = json_field_optional("server.tls", MinLength { min: 1 }).unwrap();
assert!(optional.validate(&config).is_ok());
```

## Composing Validators

```rust
use serde_json::json;
use nebula_validator::combinators::json_field::json_field;
use nebula_validator::validators::string::MinLength;
use nebula_validator::validators::numeric::InRange;
use nebula_validator::core::{Validate, ValidateExt};

let config = json!({
    "server": {
        "host": "localhost",
        "port": 8080
    }
});

// Compose with And combinator
let host = json_field("server.host", MinLength { min: 1 }).unwrap();
let port = json_field("server.port", InRange { min: 1i64, max: 65535i64 }).unwrap();

let combined = host.and(port);
assert!(combined.validate(&config).is_ok());
```

## Error Handling

```rust
use serde_json::json;
use nebula_validator::validators::numeric::InRange;
use nebula_validator::combinators::json_field::json_field;
use nebula_validator::core::Validate;

let config = json!({"port": 99999});

let validator = json_field("port", InRange { min: 1i64, max: 65535i64 }).unwrap();
let err = validator.validate(&config).unwrap_err();

// Error includes field path
assert_eq!(err.field.as_deref(), Some("port"));
assert_eq!(err.code.as_ref(), "field_validation");
```

## Running Tests

```bash
# Run all validator tests including JSON support
cargo test -p nebula-validator --features serde-json

# Run only JSON integration tests
cargo test -p nebula-validator --features serde-json json
```
