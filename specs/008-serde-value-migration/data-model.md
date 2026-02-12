# Data Model: Type Mappings for serde_json Migration

**Phase**: 1 (Design & Data Model)
**Date**: 2026-02-11
**Related**: [plan.md](plan.md), [research.md](research.md)

## Overview

This document defines the type mappings from `nebula-value` to `serde_json` and related ecosystem types.

---

## Value Type Mapping

### Core Enum Variants

| nebula_value::Value | serde_json::Value | Notes |
|---------------------|-------------------|-------|
| `Value::Null` | `Value::Null` | Identical |
| `Value::Boolean(bool)` | `Value::Bool(bool)` | Renamed variant |
| `Value::Integer(i64)` | `Value::Number(Number)` | Use `.as_i64()` to extract |
| `Value::Float(f64)` | `Value::Number(Number)` | Use `.as_f64()` to extract |
| `Value::Decimal(Decimal)` | Custom serde | See Special Types section |
| `Value::Text(Arc<str>)` | `Value::String(String)` | Loses Arc optimization |
| `Value::Bytes(Bytes)` | Custom serde or base64 | See Special Types section |
| `Value::Array(Vector<Value>)` | `Value::Array(Vec<Value>)` | Loses persistent structure |
| `Value::Object(HashMap<String, Value>)` | `Value::Object(Map<String, Value>)` | Loses persistent structure, Map is `serde_json::Map` |
| `Value::Date(...)` | `Value::String(...)` | ISO 8601 format "YYYY-MM-DD" |
| `Value::Time(...)` | `Value::String(...)` | ISO 8601 format "HH:MM:SS" |
| `Value::DateTime(...)` | `Value::String(...)` | RFC 3339 format "2026-02-11T14:30:00Z" |
| `Value::Duration(...)` | `Value::Number(...)` | Milliseconds as integer |

### Type Constructors

| nebula_value | serde_json | Code Example |
|--------------|------------|--------------|
| `Value::null()` | `Value::Null` | `Value::Null` |
| `Value::boolean(true)` | `Value::Bool(true)` | `Value::Bool(true)` |
| `Value::integer(42)` | `Value::Number(42.into())` | `Value::Number(42.into())` |
| `Value::float(3.14)` | `serde_json::json!(3.14)` | `json!(3.14)` (uses macro) |
| `Value::text("hello")` | `Value::String("hello".to_string())` | `Value::String("hello".to_string())` or `json!("hello")` |
| `Value::array_empty()` | `Value::Array(Vec::new())` | `Value::Array(vec![])` |
| `Value::object_empty()` | `Value::Object(Map::new())` | `Value::Object(serde_json::Map::new())` |

### Type Checks

| nebula_value | serde_json | Notes |
|--------------|------------|-------|
| `.is_null()` | `.is_null()` | Identical |
| `.is_boolean()` | `.is_boolean()` | Identical |
| `.is_integer()` | `.is_i64()` or `.is_u64()` | More granular |
| `.is_float()` | `.is_f64()` | Identical |
| `.is_numeric()` | `.is_number()` | Identical |
| `.is_text()` | `.is_string()` | Renamed |
| `.is_array()` | `.is_array()` | Identical |
| `.is_object()` | `.is_object()` | Identical |

### Value Extraction (Safe - returns Option)

| nebula_value | serde_json | Return Type |
|--------------|------------|-------------|
| `.as_boolean()` | `.as_bool()` | `Option<bool>` |
| `.as_integer()` | `.as_i64()` | `Option<i64>` |
| `.as_float()` | `.as_f64()` | `Option<f64>` |
| `.as_text()` | `.as_str()` | `Option<&str>` |
| `.as_array()` | `.as_array()` | `Option<&Vec<Value>>` |
| `.as_object()` | `.as_object()` | `Option<&Map<String, Value>>` |

### Value Conversion (Fallible - returns Result)

| nebula_value | serde_json | Code Pattern |
|--------------|------------|--------------|
| `.to_integer()?` | `.as_i64().ok_or(...)?` | Manual error handling |
| `.to_float()?` | `.as_f64().ok_or(...)?` | Manual error handling |
| `.to_string()?` | `.as_str().ok_or(...)?.to_string()` | Manual error handling |

**Recommendation**: Create helper functions in each crate for common conversions:

```rust
pub fn get_i64(value: &Value, key: &str) -> Result<i64, Error> {
    value.get(key)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| Error::TypeMismatch {
            expected: "integer".to_string(),
            actual: value_type_name(value),
        })
}
```

---

## Special Types

### Decimal (rust_decimal::Decimal)

**Storage**: Use `rust_decimal::Decimal` type directly, rely on serde support

```rust
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct PriceData {
    amount: Decimal,  // Serializes as string "123.45" in JSON
}

// In workflows:
// - Value::String containing decimal string
// - Parse with Decimal::from_str() when needed
```

**Format**: JSON string `"123.456789"` (preserves precision)

### Bytes (bytes::Bytes)

**Storage**: Base64 string in JSON, native `Bytes` in Rust

```rust
use bytes::Bytes;
use base64::{Engine as _, engine::general_purpose};

// Serialize
let bytes_data = Bytes::from_static(b"binary data");
let base64_str = general_purpose::STANDARD.encode(&bytes_data);
let value = Value::String(base64_str);

// Deserialize
let base64_str = value.as_str().ok_or(...)?;
let bytes_data = general_purpose::STANDARD.decode(base64_str)
    .map(Bytes::from)?;
```

**Format**: JSON string `"YmluYXJ5IGRhdGE="` (base64 encoded)

**Alternative**: For non-JSON contexts, use `Bytes` directly with custom serde

```rust
#[derive(Serialize, Deserialize)]
struct BinaryData {
    #[serde(with = "serde_bytes")]
    data: Bytes,
}
```

### Temporal Types

#### Date (chrono::NaiveDate)

**Storage**: JSON string in ISO 8601 format

```rust
use chrono::NaiveDate;

// Serialize
let date = NaiveDate::from_ymd_opt(2026, 2, 11).unwrap();
let value = Value::String(date.format("%Y-%m-%d").to_string());

// Deserialize
let date_str = value.as_str().ok_or(...)?;
let date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")?;
```

**Format**: `"2026-02-11"`

#### Time (chrono::NaiveTime)

**Storage**: JSON string in ISO 8601 format

```rust
use chrono::NaiveTime;

// Serialize
let time = NaiveTime::from_hms_opt(14, 30, 0).unwrap();
let value = Value::String(time.format("%H:%M:%S").to_string());

// Deserialize
let time_str = value.as_str().ok_or(...)?;
let time = NaiveTime::parse_from_str(time_str, "%H:%M:%S")?;
```

**Format**: `"14:30:00"`

#### DateTime (chrono::DateTime<Utc>)

**Storage**: JSON string in RFC 3339 format

```rust
use chrono::{DateTime, Utc};

// Serialize
let datetime: DateTime<Utc> = Utc::now();
let value = Value::String(datetime.to_rfc3339());

// Deserialize
let datetime_str = value.as_str().ok_or(...)?;
let datetime = DateTime::parse_from_rfc3339(datetime_str)?
    .with_timezone(&Utc);
```

**Format**: `"2026-02-11T14:30:00Z"` or `"2026-02-11T14:30:00+00:00"`

#### Duration (chrono::Duration)

**Storage**: JSON number (milliseconds)

```rust
use chrono::Duration;

// Serialize
let duration = Duration::milliseconds(5400000);  // 1.5 hours
let value = Value::Number(duration.num_milliseconds().into());

// Deserialize
let millis = value.as_i64().ok_or(...)?;
let duration = Duration::milliseconds(millis);
```

**Format**: `5400000` (milliseconds as integer)

**Alternative**: ISO 8601 duration string `"PT1H30M"` (more complex parsing)

---

## Collection Types

### Array (Vec<Value>)

**nebula-value**: `im::Vector<Value>` (persistent, O(log n) operations)
**serde_json**: `Vec<Value>` (standard, O(1) indexed access, O(n) cloning)

```rust
// Creating
let arr = Value::Array(vec![
    Value::Number(1.into()),
    Value::Number(2.into()),
    Value::Number(3.into()),
]);

// Accessing
if let Some(array) = value.as_array() {
    for item in array {
        // process item
    }
}

// Mutating (requires ownership)
if let Value::Array(ref mut arr) = value {
    arr.push(Value::Number(4.into()));
}
```

**Performance Impact**: Cloning is O(n) instead of O(1). Acceptable per spec clarifications (cloning infrequent in workflow use cases).

### Object (Map<String, Value>)

**nebula-value**: `im::HashMap<String, Value>` (persistent, O(log n) operations)
**serde_json**: `serde_json::Map<String, Value>` (standard HashMap wrapper, O(1) access)

```rust
use serde_json::Map;

// Creating
let mut obj = Map::new();
obj.insert("name".to_string(), Value::String("Alice".to_string()));
obj.insert("age".to_string(), Value::Number(30.into()));
let value = Value::Object(obj);

// Accessing
if let Some(name) = value.get("name").and_then(|v| v.as_str()) {
    println!("Name: {}", name);
}

// Mutating (requires ownership)
if let Value::Object(ref mut map) = value {
    map.insert("email".to_string(), Value::String("alice@example.com".to_string()));
}
```

**Performance Impact**: Same as Array - cloning is O(n). Acceptable per spec.

---

## Error Type Mappings

### nebula-config

```rust
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid configuration: {0}")]
    Invalid(String),

    #[error("Missing required field: {0}")]
    MissingField(String),
}
```

### nebula-resilience

```rust
#[derive(Debug, Error)]
pub enum ResilienceError {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid policy configuration: {0}")]
    InvalidPolicy(String),
}
```

### nebula-expression

```rust
#[derive(Debug, Error)]
pub enum ExpressionError {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },

    #[error("Invalid date format: {0}")]
    InvalidDate(#[from] chrono::format::ParseError),

    #[error("Invalid field access: {0}")]
    InvalidField(String),

    #[error("Evaluation error: {0}")]
    Evaluation(String),
}
```

---

## Migration Patterns

### Pattern 1: Simple Import Replacement

```rust
// Before
use nebula_value::Value;

// After
use serde_json::Value;
```

### Pattern 2: Type Check + Extraction

```rust
// Before
if let Some(num) = value.as_integer() {
    let n = num.value();  // nebula-value wraps primitives
}

// After
if let Some(n) = value.as_i64() {
    // direct i64
}
```

### Pattern 3: Builder Pattern (Arrays)

```rust
// Before
let arr = ArrayBuilder::new()
    .push(Value::integer(1))
    .push(Value::integer(2))
    .build()?;

// After
let arr = Value::Array(vec![
    Value::Number(1.into()),
    Value::Number(2.into()),
]);
```

### Pattern 4: Builder Pattern (Objects)

```rust
// Before
let obj = ObjectBuilder::new()
    .insert("name", Value::text("Alice"))
    .insert("age", Value::integer(30))
    .build()?;

// After
use serde_json::{Map, json};

// Option 1: Manual construction
let mut map = Map::new();
map.insert("name".to_string(), Value::String("Alice".to_string()));
map.insert("age".to_string(), Value::Number(30.into()));
let obj = Value::Object(map);

// Option 2: Use json! macro (recommended)
let obj = json!({
    "name": "Alice",
    "age": 30
});
```

### Pattern 5: Error Handling

```rust
// Before
let num = value.to_integer()?;  // ValueError auto-converted

// After
let num = value.as_i64().ok_or_else(|| ExpressionError::TypeMismatch {
    expected: "integer".to_string(),
    actual: value_type_name(&value),
})?;

// Helper function
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

## Summary

| Aspect | Impact | Mitigation |
|--------|--------|------------|
| Persistent collections removed | O(n) cloning instead of O(1) | Acceptable - cloning infrequent per spec |
| Arc<str> removed | More allocations for strings | Acceptable - not a bottleneck |
| Custom Decimal/Bytes variants removed | Manual serde or custom types | Use rust_decimal/bytes with serde |
| Temporal variants removed | Parse from strings | Use chrono when operations needed |
| Builder patterns removed | More verbose construction | Use json! macro |

**Net Result**: Simpler types, ecosystem compatibility, zero conversion overhead. Performance impact acceptable per spec (SC-007: validated by existing tests).
