# Quickstart: nebula-validator (after restructuring)

## Add dependency

```toml
# crates/parameter/Cargo.toml (or any consumer)
[dependencies]
nebula-validator = { path = "../validator" }
# Features: serde (default), caching (optional), optimizer (optional)
```

## Basic usage (one import)

```rust
use nebula_validator::prelude::*;
use serde_json::json;

// String validation
min_length(5).validate("hello")?;             // typed &str
min_length(5).validate_any(&json!("hello"))?; // serde_json::Value (auto-extract)

// Numeric validation
min(0.0).validate(&42.0)?;
min(0.0).validate_any(&json!(42))?;

// Collection (no turbofish!)
json_min_size(2).validate_any(&json!([1, 2, 3]))?;

// Format validators
email().validate("user@example.com")?;
hostname().validate("api.example.com")?;
time_only().validate("14:30:00")?;
DateTime::date_only().validate("2026-02-16")?;
Uuid::new().validate("550e8400-e29b-41d4-a716-446655440000")?;

// Combinators
let validator = min_length(1).and(max_length(100)).and(email());
validator.validate("user@example.com")?;

// Conditional
let v = min(0.0).when(|n: &f64| *n != 0.0);
v.validate(&0.0)?; // skipped, passes

// Type mismatch handled gracefully
let result = min_length(5).validate_any(&json!(42));
assert!(result.is_err()); // code: "type_mismatch"
```

## Module paths

```rust
// Preferred: prelude
use nebula_validator::prelude::*;

// Direct access (if needed)
use nebula_validator::foundation::{Validate, ValidationError};
use nebula_validator::validators::min_length;
use nebula_validator::combinators::and;

// JSON helpers (requires "serde" feature, default)
use nebula_validator::json::{json_min_size, json_max_size};
```

## Feature flags

```toml
# Default (serde): JSON/Value support
nebula-validator = { path = "../validator" }

# Without serde: pure typed validation only
nebula-validator = { path = "../validator", default-features = false }

# With caching: LRU cache combinator
nebula-validator = { path = "../validator", features = ["caching"] }

# Everything
nebula-validator = { path = "../validator", features = ["full"] }
```

## Build verification

```bash
cargo check -p nebula-validator                    # default features
cargo check -p nebula-validator --no-default-features  # no serde
cargo check -p nebula-validator --all-features     # everything
cargo test -p nebula-validator --all-features       # all tests
cargo clippy -p nebula-validator -- -D warnings    # lint
```
