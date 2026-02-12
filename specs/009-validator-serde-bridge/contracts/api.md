# API Contracts: Validator Serde Bridge

**Feature**: 009-validator-serde-bridge
**Date**: 2026-02-11

This feature is a Rust library (not a web API), so contracts are defined as public trait implementations and type signatures.

## Contract 1: AsValidatable Implementations

```rust
// Gate: #[cfg(feature = "serde-json")]

// String extraction
impl AsValidatable<str> for serde_json::Value {
    type Output<'a> = &'a str where Self: 'a;
    fn as_validatable(&self) -> Result<Self::Output<'_>, ValidationError>;
    // Returns Ok(&str) for Value::String, Err(type_mismatch) otherwise
}

// Integer extraction
impl AsValidatable<i64> for serde_json::Value {
    type Output<'a> = i64;
    fn as_validatable(&self) -> Result<i64, ValidationError>;
    // Returns Ok(i64) for Value::Number with valid i64, Err(type_mismatch) otherwise
}

// Float extraction
impl AsValidatable<f64> for serde_json::Value {
    type Output<'a> = f64;
    fn as_validatable(&self) -> Result<f64, ValidationError>;
    // Returns Ok(f64) for Value::Number, Err(type_mismatch) otherwise
}

// Boolean extraction
impl AsValidatable<bool> for serde_json::Value {
    type Output<'a> = bool;
    fn as_validatable(&self) -> Result<bool, ValidationError>;
    // Returns Ok(bool) for Value::Bool, Err(type_mismatch) otherwise
}

// Array extraction (for collection validators)
impl AsValidatable<[serde_json::Value]> for serde_json::Value {
    type Output<'a> = &'a [serde_json::Value] where Self: 'a;
    fn as_validatable(&self) -> Result<Self::Output<'_>, ValidationError>;
    // Returns Ok(&[Value]) for Value::Array, Err(type_mismatch) otherwise
}
```

## Contract 2: JsonPath

```rust
/// Parsed JSON path for field traversal.
pub struct JsonPath { /* ... */ }

impl JsonPath {
    /// Parse a path string like "server.hosts[0].port".
    pub fn parse(path: &str) -> Result<Self, ValidationError>;

    /// Resolve the path against a JSON value, returning a reference to the target.
    pub fn resolve<'a>(&self, root: &'a serde_json::Value) -> Result<&'a serde_json::Value, ValidationError>;
}

impl std::fmt::Display for JsonPath {
    // Formats back to human-readable path string
}
```

## Contract 3: JsonField Combinator

```rust
/// Validates a field within a JSON object by path.
pub struct JsonField<V> { /* ... */ }

impl<V> JsonField<V> {
    /// Create a required field validator.
    pub fn new(path: &str, validator: V) -> Result<Self, ValidationError>;

    /// Create an optional field validator (null/missing = pass).
    pub fn optional(path: &str, validator: V) -> Result<Self, ValidationError>;
}

impl<V> Validate for JsonField<V>
where
    V: Validate,
    serde_json::Value: AsValidatable<V::Input>,
{
    type Input = serde_json::Value;

    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError>;
    // 1. Resolve path to target value
    // 2. If missing: error (required) or Ok (optional)
    // 3. If present: extract via AsValidatable and validate
    // 4. Errors include field path context
}
```

## Contract 4: Helper Functions

```rust
/// Convenience: create a required JSON field validator.
pub fn json_field<V>(path: &str, validator: V) -> Result<JsonField<V>, ValidationError>;

/// Convenience: create an optional JSON field validator.
pub fn json_field_optional<V>(path: &str, validator: V) -> Result<JsonField<V>, ValidationError>;
```

## Error Contract

All bridge errors use existing `ValidationError` type with these codes:

| Code | Params | Context |
|------|--------|---------|
| `type_mismatch` | `expected`, `actual` | `field` set to path if from JsonField |
| `path_not_found` | `path` | `field` set to path |
| `invalid_path` | `path` | No field context |
| `index_out_of_bounds` | `index`, `path` | `field` set to path |
