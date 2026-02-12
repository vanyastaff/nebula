# API Contract: nebula-expression

**Crate**: nebula-expression
**Migration Phase**: Phase 2.3
**Date**: 2026-02-11

## Public API Stability

### Value Type Change (Non-Breaking for Users)

The `nebula-expression` crate changes its internal Value type from `nebula_value::Value` to `serde_json::Value`. **Expression evaluation behavior remains identical**, but the underlying Value type changes.

### Impact on Users

| Aspect | Before | After | Breaking? |
|--------|--------|-------|-----------|
| Expression syntax | `$nodes.filter.output.data` | Identical | No |
| Builtin functions | `$now()`, `$json()`, etc. | Identical behavior | No |
| Template rendering | `{{ variable }}` | Identical | No |
| Error messages | Type mismatch errors | May differ slightly | No (compatible) |
| Return type | `nebula_value::Value` | `serde_json::Value` | Yes (but transparent) |

### Breaking Change Mitigation

While the return type changes from `nebula_value::Value` to `serde_json::Value`, this is **transparent to workflow users** because:
1. Both types represent the same JSON-compatible data
2. Workflow definitions don't reference the Rust type explicitly
3. Serialization format (JSON) remains identical

For **Rust API users** (if any depend on nebula-expression directly), this is a breaking change that requires updating imports.

---

## Migration Details

### Public API Surface Changes

```rust
// ❌ Before
pub fn evaluate(expr: &str, context: &Context) -> Result<nebula_value::Value, ExpressionError>;

// ✅ After
pub fn evaluate(expr: &str, context: &Context) -> Result<serde_json::Value, ExpressionError>;
```

### Dependencies Updated

```toml
# Removed
nebula-value = { path = "../nebula-value", features = ["temporal", "serde"] }

# Ensured present
serde_json = { workspace = true }
chrono = { workspace = true }  # For temporal operations
```

### Error Handling

```rust
#[derive(Debug, Error)]
pub enum ExpressionError {
    // ✅ NEW: Handle serde_json errors
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    // ✅ NEW: Handle chrono errors
    #[error("Invalid date format: {0}")]
    InvalidDate(#[from] chrono::format::ParseError),

    // Existing variants updated for serde_json types
    #[error("Type mismatch: expected {expected}, got {actual}")]
    TypeMismatch { expected: String, actual: String },

    #[error("Invalid field access: {0}")]
    InvalidField(String),

    #[error("Evaluation error: {0}")]
    Evaluation(String),
}
```

---

## Affected Components

### Builtin Functions

All builtin functions updated to work with `serde_json::Value`:

| Module | Changes |
|--------|---------|
| `builtins/math.rs` | Use `.as_i64()` / `.as_f64()` instead of `.as_integer()` / `.as_float()` |
| `builtins/string.rs` | Use `.as_str()` instead of `.as_text()` |
| `builtins/datetime.rs` | Parse ISO 8601 strings with `chrono` instead of using Date/DateTime variants |
| `builtins/array.rs` | Use `.as_array()` (returns `Vec<Value>`) |
| `builtins/object.rs` | Use `.as_object()` (returns `Map<String, Value>`) |

### Template Engine

- Context values use `serde_json::Value`
- Variable substitution unchanged (same syntax)
- Type coercion updated for serde_json types

### Expression Context

```rust
// ❌ Before
pub struct Context {
    variables: HashMap<String, nebula_value::Value>,
}

// ✅ After
pub struct Context {
    variables: HashMap<String, serde_json::Value>,
}
```

---

## Behavioral Guarantees

### Expression Evaluation

**Identical Results**: All expressions produce the same output (semantically equivalent JSON).

**Example**:

```javascript
// Expression
$nodes.filter.output.items[0].name

// Before: nebula_value::Value::Text("Alice")
// After: serde_json::Value::String("Alice")
// JSON output: "Alice" (identical)
```

### Temporal Operations

**Before** (nebula-value):
```rust
$now()  // Returns Value::DateTime(DateTime { ... })
```

**After** (serde_json):
```rust
$now()  // Returns Value::String("2026-02-11T14:30:00Z")
```

**Impact**: String representation instead of custom variant, but JSON serialization identical.

### Type Checking

**Before**:
```javascript
$isNumber($value)  // Checks value.is_integer() || value.is_float()
```

**After**:
```javascript
$isNumber($value)  // Checks value.is_number()
```

**Impact**: Behavior identical, implementation simplified.

---

## Testing Strategy

### Existing Tests

All existing expression evaluation tests MUST pass with identical outputs (JSON comparison).

### Temporal Type Tests

```rust
#[test]
fn test_datetime_now() {
    let result = evaluate("$now()", &context)?;
    // Before: assert!(result.is_datetime());
    // After:
    assert!(result.is_string());
    let dt_str = result.as_str().unwrap();
    DateTime::parse_from_rfc3339(dt_str).unwrap();  // Validate format
}
```

### Type Coercion Tests

```rust
#[test]
fn test_type_coercion() {
    let result = evaluate("$number('42')", &context)?;
    // Before: assert_eq!(result.to_integer()?, 42);
    // After:
    assert_eq!(result.as_i64().unwrap(), 42);
}
```

---

## Migration Complexity

**High**: This is the most complex migration due to:
1. Extensive Value usage across many modules (builtins, template, context)
2. Temporal type handling changes (variants → strings)
3. Many type checks and conversions to update
4. Comprehensive test suite to validate

**Estimated effort**: 8-12 hours

---

## Validation

- [ ] `cargo check -p nebula-expression` - compiles
- [ ] `cargo test -p nebula-expression` - 100% pass rate
- [ ] `cargo clippy -p nebula-expression -- -D warnings` - no warnings
- [ ] Expression evaluation tests produce identical JSON outputs
- [ ] Builtin functions behave identically (tested with sample expressions)
- [ ] Temporal operations produce valid RFC 3339 strings
- [ ] Type coercion works correctly with serde_json types
- [ ] Template rendering produces identical output
