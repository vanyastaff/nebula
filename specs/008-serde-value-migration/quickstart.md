# Quick Start: Migrating to serde_json

**Phase**: 1 (Design & Data Model)
**Date**: 2026-02-11
**Related**: [plan.md](plan.md), [data-model.md](data-model.md)

## Overview

This guide helps developers migrate from `nebula-value` to `serde_json::Value` with before/after examples.

---

## Common Patterns

### Import Statements

```rust
// ❌ Before
use nebula_value::Value;
use nebula_value::prelude::*;
use nebula_value::{Array, Object, Integer, Float, Text};

// ✅ After
use serde_json::Value;
use serde_json::Map;  // Only if working with Object internals
```

---

### Creating Values

#### Null

```rust
// ❌ Before
let val = Value::null();

// ✅ After
let val = Value::Null;
```

#### Boolean

```rust
// ❌ Before
let val = Value::boolean(true);

// ✅ After
let val = Value::Bool(true);
```

#### Integer

```rust
// ❌ Before
let val = Value::integer(42);

// ✅ After
let val = Value::Number(42.into());
// or using json! macro
let val = serde_json::json!(42);
```

#### Float

```rust
// ❌ Before
let val = Value::float(3.14);

// ✅ After
let val = serde_json::json!(3.14);
```

#### String

```rust
// ❌ Before
let val = Value::text("hello");

// ✅ After
let val = Value::String("hello".to_string());
// or
let val = serde_json::json!("hello");
```

#### Array

```rust
// ❌ Before
let val = Value::array_empty();
// or
let val = ArrayBuilder::new()
    .push(Value::integer(1))
    .push(Value::integer(2))
    .build()?;

// ✅ After
let val = Value::Array(Vec::new());
// or
let val = Value::Array(vec![
    Value::Number(1.into()),
    Value::Number(2.into()),
]);
// or using json! macro (recommended)
let val = serde_json::json!([1, 2]);
```

#### Object

```rust
// ❌ Before
let val = Value::object_empty();
// or
let val = ObjectBuilder::new()
    .insert("name", Value::text("Alice"))
    .insert("age", Value::integer(30))
    .build()?;

// ✅ After
use serde_json::Map;

let mut map = Map::new();
map.insert("name".to_string(), Value::String("Alice".to_string()));
map.insert("age".to_string(), Value::Number(30.into()));
let val = Value::Object(map);

// or using json! macro (recommended)
let val = serde_json::json!({
    "name": "Alice",
    "age": 30
});
```

---

### Type Checks

```rust
// ❌ Before
if value.is_integer() { /* ... */ }
if value.is_text() { /* ... */ }

// ✅ After
if value.is_i64() { /* ... */ }
// or for unsigned
if value.is_u64() { /* ... */ }
if value.is_string() { /* ... */ }
```

---

### Safe Extraction (Option)

```rust
// ❌ Before
if let Some(num) = value.as_integer() {
    let n: i64 = num.value();
}

if let Some(text) = value.as_text() {
    let s: &str = text.as_str();
}

// ✅ After
if let Some(n) = value.as_i64() {
    // n is already i64
}

if let Some(s) = value.as_str() {
    // s is already &str
}
```

---

### Fallible Conversion (Result)

```rust
// ❌ Before
let num: i64 = value.to_integer()?;
let text: String = value.to_string()?;

// ✅ After
// Manual error handling required
let num = value.as_i64().ok_or_else(|| MyError::TypeMismatch {
    expected: "integer".to_string(),
    actual: value_type_name(&value),
})?;

let text = value.as_str().ok_or_else(|| MyError::TypeMismatch {
    expected: "string".to_string(),
    actual: value_type_name(&value),
})?.to_string();

// Helper function (add to each crate)
fn value_type_name(value: &Value) -> String {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }.to_string()
}
```

---

### Field Access

```rust
// ❌ Before
let name = value.get("name")?;  // ValueError if missing

// ✅ After
// Returns Option
let name = value.get("name");
// or with fallback
let name = value.get("name").unwrap_or(&Value::Null);

// Or error handling
let name = value.get("name").ok_or_else(|| MyError::MissingField("name".to_string()))?;
```

---

### Indexing

```rust
// Both work identically
let name = &value["name"];  // Returns &Value::Null if key doesn't exist
```

---

### Array Iteration

```rust
// ❌ Before
if let Some(arr) = value.as_array() {
    for item in arr.iter() {
        // nebula-value Vector iterator
    }
}

// ✅ After
if let Some(arr) = value.as_array() {
    for item in arr {
        // standard Vec<Value> iterator
    }
}
```

---

### Object Iteration

```rust
// ❌ Before
if let Some(obj) = value.as_object() {
    for (key, val) in obj.iter() {
        // nebula-value HashMap iterator
    }
}

// ✅ After
if let Some(obj) = value.as_object() {
    for (key, val) in obj {
        // serde_json::Map iterator
    }
}
```

---

## Temporal Types

### Date

```rust
// ❌ Before
use nebula_value::Date;
let date = Date::from_ymd(2026, 2, 11);
let value = Value::date(date);

// ✅ After
use chrono::NaiveDate;
let date = NaiveDate::from_ymd_opt(2026, 2, 11).unwrap();
let value = Value::String(date.format("%Y-%m-%d").to_string());
// String format: "2026-02-11"

// Parsing back
let date_str = value.as_str().ok_or(...)?;
let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")?;
```

### DateTime

```rust
// ❌ Before
use nebula_value::DateTime;
let dt = DateTime::now();
let value = Value::datetime(dt);

// ✅ After
use chrono::{DateTime, Utc};
let dt: DateTime<Utc> = Utc::now();
let value = Value::String(dt.to_rfc3339());
// String format: "2026-02-11T14:30:00Z"

// Parsing back
let dt_str = value.as_str().ok_or(...)?;
let dt = DateTime::parse_from_rfc3339(dt_str)?.with_timezone(&Utc);
```

### Duration

```rust
// ❌ Before
use nebula_value::Duration;
let dur = Duration::milliseconds(5000);
let value = Value::duration(dur);

// ✅ After
use chrono::Duration;
let dur = Duration::milliseconds(5000);
let value = Value::Number(dur.num_milliseconds().into());
// Number format: 5000

// Parsing back
let millis = value.as_i64().ok_or(...)?;
let dur = Duration::milliseconds(millis);
```

---

## Special Types

### Decimal

```rust
// ❌ Before
use nebula_value::Decimal;
let dec = Decimal::from_str("123.45")?;
let value = Value::decimal(dec);

// ✅ After
use rust_decimal::Decimal;
// Store as string in Value
let dec = Decimal::from_str("123.45")?;
let value = Value::String(dec.to_string());
// String format: "123.45"

// Or for struct fields, use serde directly
#[derive(Serialize, Deserialize)]
struct Price {
    amount: Decimal,  // Serializes as string automatically
}
```

### Bytes

```rust
// ❌ Before
use nebula_value::Bytes;
let bytes = Bytes::from_static(b"data");
let value = Value::bytes(bytes);

// ✅ After
use bytes::Bytes;
use base64::{Engine as _, engine::general_purpose};

let bytes = Bytes::from_static(b"data");
let base64_str = general_purpose::STANDARD.encode(&bytes);
let value = Value::String(base64_str);
// String format: "ZGF0YQ==" (base64)

// Parsing back
let base64_str = value.as_str().ok_or(...)?;
let bytes = general_purpose::STANDARD.decode(base64_str)
    .map(Bytes::from)?;
```

---

## Error Handling Updates

### Update Error Types

Each crate needs to add `serde_json::Error` support:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum MyError {
    // ✅ Add this variant
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },

    // ... other variants
}
```

### Wrapping Errors with Context

```rust
// ❌ Before
let value: Value = serde_json::from_str(json_str)?;

// ✅ After
let value: Value = serde_json::from_str(json_str)
    .map_err(|e| MyError::Json(e))?;

// or with context
let value: Value = serde_json::from_str(json_str)
    .map_err(|e| MyError::Invalid(
        format!("Failed to parse JSON from {}: {}", source_name, e)
    ))?;
```

---

## Testing

### Update Test Expectations

```rust
// ❌ Before
assert_eq!(value.to_integer()?, 42);

// ✅ After
assert_eq!(value.as_i64().unwrap(), 42);

// or with error handling
let num = value.as_i64().ok_or_else(|| ...)?;
assert_eq!(num, 42);
```

### JSON Literals in Tests

```rust
// ❌ Before
let value = nebula_value::json!({ "name": "Alice", "age": 30 });

// ✅ After
let value = serde_json::json!({ "name": "Alice", "age": 30 });
```

---

## Cargo.toml Updates

### Remove nebula-value

```toml
# ❌ Remove this
nebula-value = { path = "../nebula-value", features = ["temporal", "serde"] }
```

### Ensure serde_json present

```toml
# ✅ Ensure this exists (usually already in workspace dependencies)
serde_json = { workspace = true }
```

### Add optional dependencies

```toml
# ✅ Add if using temporal types
chrono = { workspace = true }

# ✅ Add if using decimals
rust_decimal = { workspace = true }

# ✅ Add if using binary data
bytes = { workspace = true }
base64 = { workspace = true }
```

---

## Validation Checklist

After migration, verify:

- [ ] `cargo check -p <crate>` - no compilation errors
- [ ] `cargo test -p <crate>` - all tests pass
- [ ] `cargo clippy -p <crate> -- -D warnings` - no warnings
- [ ] `cargo fmt --all` - code formatted
- [ ] `rg "nebula_value" crates/<crate>/` - no remaining imports
- [ ] Review diffs - confirm no logic changes, only type updates

---

## Common Pitfalls

### 1. Forgetting to convert primitives

```rust
// ❌ Wrong
let value = Value::Number(42);  // Error: expected Number, got i32

// ✅ Correct
let value = Value::Number(42.into());
// or
let value = serde_json::json!(42);
```

### 2. Assuming NaN handling

```rust
// nebula-value has special NaN handling
// serde_json treats NaN as null in some contexts

// ✅ Check explicitly
if value.is_number() {
    if let Some(f) = value.as_f64() {
        if f.is_nan() {
            // handle NaN
        }
    }
}
```

### 3. Clone performance

```rust
// nebula-value: O(1) clone (persistent structure)
// serde_json: O(n) clone (deep copy)

// ✅ Avoid unnecessary clones
// Instead of:
let value_copy = value.clone();

// Consider:
let value_ref = &value;
```

### 4. Mutable operations

```rust
// ❌ This won't work on &Value
value["key"] = new_value;

// ✅ Need mut access
if let Value::Object(ref mut map) = value {
    map.insert("key".to_string(), new_value);
}
```

---

## Next Steps

1. Update Cargo.toml dependencies
2. Replace imports
3. Fix compilation errors (type method changes)
4. Update error handling
5. Run tests and fix failures
6. Run quality gates (fmt, clippy, check, doc)
7. Review diffs for correctness

See [plan.md](plan.md) for detailed migration order (nebula-config → nebula-resilience → nebula-expression).
