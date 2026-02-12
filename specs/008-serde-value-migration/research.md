# Research: serde_json Migration

**Phase**: 0 (Research & Prerequisites)
**Date**: 2026-02-11
**Related**: [plan.md](plan.md), [spec.md](spec.md)

## Overview

Research findings for migrating from custom `nebula-value` to `serde_json::Value` and `RawValue`.

## 1. serde_json::Value API

### Core Type Structure

```rust
pub enum Value {
    Null,
    Bool(bool),
    Number(Number),    // Handles i64, u64, f64
    String(String),
    Array(Vec<Value>),
    Object(Map<String, Value>),  // Map is serde_json::Map
}
```

### Key Methods

| Method | nebula_value | serde_json | Notes |
|--------|--------------|------------|-------|
| Type check | `is_integer()` | `is_i64()`, `is_u64()`, `is_f64()` | More granular |
| Safe access | `as_integer()` | `as_i64()` | Returns `Option` |
| Conversion | `to_integer()` | `as_i64().ok_or(...)` | Manual error handling |
| Field access | `.get("key")` | `.get("key")` | Identical |
| Indexing | `value["key"]` | `value["key"]` | Identical, returns `&Value::Null` if missing |

### Differences from nebula_value

- **No Decimal variant**: Use `rust_decimal::Decimal` with custom serde, or Number (loses precision)
- **No Bytes variant**: Serialize as base64 strings or use custom serde with `bytes::Bytes`
- **No Date/Time variants**: Store as ISO 8601 strings, parse with `chrono`
- **No persistent collections**: Uses std `Vec` and `Map` (clone-on-write not built-in)

**Decision**: Accept these differences. Temporal types as strings align with n8n compatibility (spec assumption). Decimal precision via `rust_decimal` serde support. Bytes as base64 for JSON, native `Bytes` type for non-JSON contexts.

---

## 2. RawValue Best Practices

### What is RawValue?

`serde_json::value::RawValue` is a borrowed or owned representation of **unparsed JSON** as a string. It defers deserialization until accessed.

### Usage Patterns

```rust
// Owned - for storing
use serde_json::value::RawValue;
let raw: Box<RawValue> = serde_json::value::to_raw_value(&my_value)?;

// Borrowed - for passing
fn process(input: &RawValue) -> Result<(), Error> {
    // Only parse if needed
    if should_inspect {
        let value: Value = serde_json::from_str(input.get())?;
        // work with value
    }
    Ok(())
}

// Convert back
let value: Value = serde_json::from_str(raw.get())?;
```

### When to Use

- **Pass-through scenarios**: Data flows through node without inspection (Filter, Switch, Merge nodes)
- **Deferred parsing**: Large payloads that might not be accessed
- **API boundaries**: Workflow node inputs/outputs to minimize parsing overhead

### Performance Characteristics

- **Clone**: Cheap (string clone, not full parse)
- **Serialize**: Already JSON string, no re-serialization needed
- **Deserialize**: Deferred until `.get()` + `serde_json::from_str`

**Decision**: Use `Box<RawValue>` for owned workflow data at node boundaries. Convert to `Value` only when nodes actually access fields. Pass-through nodes clone `RawValue` without parsing (optimized path).

---

## 3. Temporal Type Handling

### Format Standards

| Type | Format | Example | `chrono` Type |
|------|--------|---------|---------------|
| Date | ISO 8601 (YYYY-MM-DD) | `"2026-02-11"` | `chrono::NaiveDate` |
| Time | ISO 8601 (HH:MM:SS) | `"14:30:00"` | `chrono::NaiveTime` |
| DateTime | RFC 3339 | `"2026-02-11T14:30:00Z"` | `chrono::DateTime<Utc>` |
| Duration | ISO 8601 duration or milliseconds | `"PT1H30M"` or `5400000` | `chrono::Duration` |

### Parsing Patterns

```rust
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};

// Parse from Value (string)
fn parse_date(value: &Value) -> Result<NaiveDate, Error> {
    let s = value.as_str()
        .ok_or_else(|| Error::TypeMismatch { expected: "string", actual: "..." })?;
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| Error::InvalidDate(e))
}

// Or use chrono's built-in RFC 3339 parsing
fn parse_datetime(value: &Value) -> Result<DateTime<Utc>, Error> {
    let s = value.as_str()...;
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| Error::InvalidDateTime(e))
}
```

### Common Pitfalls

- **Timezone ambiguity**: Always use UTC or explicit timezone. Avoid naive datetime unless truly timezone-independent.
- **Duration format**: ISO 8601 duration (`PT1H`) vs milliseconds (`3600000`). Pick one and document it.
- **Leap seconds/DST**: `chrono` handles these correctly, but test edge cases.

**Decision**: Store temporal values as RFC 3339 strings (DateTime), ISO 8601 date/time strings (Date/Time), and milliseconds for Duration (simpler than ISO 8601 duration parsing). Use `chrono` only when temporal operations are needed (expressions, builtin functions).

---

## 4. Error Conversion Patterns

### Per-Crate Error Types

Each crate defines its own error with `thiserror`:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Invalid configuration: {0}")]
    Invalid(String),
}
```

### Wrapping serde_json::Error with Context

```rust
// Bad - loses context
let value: Value = serde_json::from_str(json_str)?;

// Good - add context
let value: Value = serde_json::from_str(json_str)
    .map_err(|e| ConfigError::Invalid(
        format!("Failed to parse config file {}: {}", filename, e)
    ))?;
```

### Handling Type Mismatches

```rust
// nebula-expression example
#[derive(Debug, Error)]
pub enum ExpressionError {
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },

    #[error("Invalid date format: {0}")]
    InvalidDate(#[from] chrono::ParseError),
}

// Usage
fn get_string(value: &Value) -> Result<&str, ExpressionError> {
    value.as_str().ok_or_else(|| ExpressionError::TypeMismatch {
        expected: "string".to_string(),
        actual: value_type_name(value),
    })
}

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

**Decision**: Each crate includes `#[from] serde_json::Error` for JSON parsing errors. Domain-specific errors (TypeMismatch, InvalidDate) wrap with meaningful context. Follow `thiserror` best practices.

---

## 5. Migration Checklist

### Per-Crate Steps

For each crate (nebula-config, nebula-resilience, nebula-expression):

1. **Update Cargo.toml**
   - [ ] Remove `nebula-value = { path = "../nebula-value", features = [...] }`
   - [ ] Ensure `serde_json = { workspace = true }` present
   - [ ] Add `chrono = { workspace = true }` if temporal types used
   - [ ] Add `rust_decimal = { workspace = true }` if decimals used
   - [ ] Add `bytes = { workspace = true }` if binary data used

2. **Update imports**
   - [ ] Find/replace: `use nebula_value::Value` → `use serde_json::Value`
   - [ ] Find/replace: `use nebula_value::prelude::*` → `use serde_json::Value`
   - [ ] Remove: `use nebula_value::{Array, Object, ...}` (use Vec/Map directly)

3. **Fix type mismatches**
   - [ ] `.is_integer()` → `.is_i64()` or `.is_u64()`
   - [ ] `.as_integer()` → `.as_i64()`
   - [ ] `.to_integer()?` → `.as_i64().ok_or(...)?`
   - [ ] `Value::integer(42)` → `Value::Number(42.into())`
   - [ ] `Value::text("...")` → `Value::String("...".to_string())`
   - [ ] `Value::array_empty()` → `Value::Array(Vec::new())`
   - [ ] `Value::object_empty()` → `Value::Object(serde_json::Map::new())`

4. **Update error types**
   - [ ] Add `#[error("JSON error: {0}")]` variant
   - [ ] Add `Json(#[from] serde_json::Error)` variant
   - [ ] Update error handling to use new variant

5. **Run tests**
   - [ ] `cargo check -p <crate>`
   - [ ] `cargo test -p <crate>`
   - [ ] Fix any test failures (update test expectations if needed)

6. **Quality gates**
   - [ ] `cargo fmt --all`
   - [ ] `cargo clippy -p <crate> -- -D warnings`
   - [ ] `cargo check -p <crate>`
   - [ ] `cargo doc -p <crate> --no-deps`

### Validation

After all crates migrated:

- [ ] `rg "nebula_value" --type rust` → should return only comments/docs (no imports)
- [ ] `cargo test --workspace` → 100% pass rate
- [ ] `cargo check --workspace` → zero errors
- [ ] `cargo clippy --workspace -- -D warnings` → zero warnings

---

## Alternatives Considered

### Alternative 1: Keep nebula-value but wrap serde_json

**Pros**: No migration needed, gradual transition
**Cons**: Maintains conversion overhead (defeats purpose), complexity of dual API

**Rejected**: Violates spec requirement to "eliminate conversion overhead"

### Alternative 2: Create thin alias module (nebula-types)

**Pros**: Single import point
**Cons**: Extra indirection, doesn't simplify as much

**Rejected**: User feedback preferred direct serde_json usage (brainstorming session)

### Alternative 3: Migrate all crates at once

**Pros**: Clean cutover, no intermediate state
**Cons**: High risk, difficult to validate incrementally

**Rejected**: Bottom-up approach safer, allows incremental testing (spec requirement)

---

## Rationale Summary

1. **serde_json::Value**: Industry standard, zero conversion overhead with ecosystem
2. **RawValue**: Optimize pass-through without complexity for node developers
3. **Temporal as strings**: n8n compatibility, `chrono` for operations
4. **Bottom-up migration**: Risk reduction, incremental validation
5. **Test-first**: Existing tests define behavior, migration preserves it

---

## References

- [serde_json documentation](https://docs.rs/serde_json/)
- [chrono documentation](https://docs.rs/chrono/)
- [Rust API Guidelines - Type conversions](https://rust-lang.github.io/api-guidelines/interoperability.html)
- [n8n data types](https://docs.n8n.io/data/)
