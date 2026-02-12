# Validator Serde Bridge Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add first-class `serde_json::Value` support to `nebula-validator` so all existing validators work with JSON values natively.

**Architecture:** Extend `core/validatable.rs` with `AsValidatable` impls for `Value` (str, i64, f64, bool, Vec<Value>). Add `JsonField<V>` combinator + `JsonPath` in `combinators/json_field.rs`. Everything behind `serde-json` feature flag.

**Tech Stack:** Rust 2024 (MSRV 1.92), serde_json (already a dependency), nebula-validator trait system (`Validate`, `AsValidatable`, `ValidateExt`)

**Spec:** `specs/009-validator-serde-bridge/spec.md`

---

### Task 1: Add `serde-json` Feature Flag

**Files:**
- Modify: `crates/nebula-validator/Cargo.toml`

**Step 1: Add the feature flag**

In `crates/nebula-validator/Cargo.toml`, add `serde-json` to `[features]`:

```toml
[features]
default = ["serde"]
serde = []
serde-json = []
```

Note: `serde_json` is already a non-optional dependency. The feature flag gates only the `AsValidatable` impls and `JsonField` combinator code, not the dependency itself.

**Step 2: Verify it compiles**

Run: `cargo check -p nebula-validator --features serde-json`
Expected: compiles with no errors (feature exists but gates nothing yet)

**Step 3: Verify default build is unaffected**

Run: `cargo check -p nebula-validator`
Expected: compiles with no errors

**Step 4: Commit**

```bash
git add crates/nebula-validator/Cargo.toml
git commit -m "feat(nebula-validator): add serde-json feature flag"
```

---

### Task 2: `AsValidatable<str>` for `Value` — Test

**Files:**
- Modify: `crates/nebula-validator/src/core/validatable.rs`

**Step 1: Write the failing test**

At the bottom of the `#[cfg(test)] mod tests` block in `crates/nebula-validator/src/core/validatable.rs`, add a new test module gated behind the feature:

```rust
#[cfg(feature = "serde-json")]
mod serde_json_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_value_string_as_str() {
        let value = json!("hello");
        let result: Result<_, _> = AsValidatable::<str>::as_validatable(&value);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().borrow() as &str, "hello");
    }

    #[test]
    fn test_value_number_as_str_fails() {
        let value = json!(42);
        let result: Result<_, _> = AsValidatable::<str>::as_validatable(&value);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
    }

    #[test]
    fn test_value_null_as_str_fails() {
        let value = json!(null);
        let result: Result<_, _> = AsValidatable::<str>::as_validatable(&value);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
    }

    #[test]
    fn test_value_string_with_validate_any() {
        use crate::core::Validate;

        struct MinLen { min: usize }
        impl Validate for MinLen {
            type Input = str;
            fn validate(&self, input: &str) -> Result<(), ValidationError> {
                if input.len() >= self.min {
                    Ok(())
                } else {
                    Err(ValidationError::new("min_length", "too short"))
                }
            }
        }

        let validator = MinLen { min: 3 };
        assert!(validator.validate_any(&json!("hello")).is_ok());
        assert!(validator.validate_any(&json!("hi")).is_err());
        assert!(validator.validate_any(&json!(42)).is_err()); // type mismatch
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-validator --features serde-json serde_json_tests -- --nocapture`
Expected: FAIL — `AsValidatable<str>` is not implemented for `serde_json::Value`

---

### Task 3: `AsValidatable<str>` for `Value` — Implementation

**Files:**
- Modify: `crates/nebula-validator/src/core/validatable.rs`

**Step 1: Add the implementation**

After the existing `AsValidatable` impls section (after the numeric widenings block, before `#[cfg(test)]`), add a new gated section:

```rust
// ============================================================================
// SERDE JSON VALUE CONVERSIONS
// ============================================================================

#[cfg(feature = "serde-json")]
impl AsValidatable<str> for serde_json::Value {
    type Output<'a>
        = &'a str
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&str, ValidationError> {
        match self {
            serde_json::Value::String(s) => Ok(s.as_str()),
            other => Err(ValidationError::new(
                "type_mismatch",
                format!("Expected string, got {}", json_type_name(other)),
            )
            .with_param("expected", "string")
            .with_param("actual", json_type_name(other))),
        }
    }
}

/// Returns a human-readable type name for a JSON value.
#[cfg(feature = "serde-json")]
fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}
```

**Step 2: Run tests to verify they pass**

Run: `cargo test -p nebula-validator --features serde-json serde_json_tests -- --nocapture`
Expected: all 4 tests PASS

**Step 3: Verify default build is unaffected**

Run: `cargo test -p nebula-validator`
Expected: all existing tests PASS, no new tests visible (feature not enabled)

**Step 4: Commit**

```bash
git add crates/nebula-validator/src/core/validatable.rs
git commit -m "feat(nebula-validator): AsValidatable<str> for serde_json::Value"
```

---

### Task 4: `AsValidatable<i64>` for `Value` — Test + Implementation

**Files:**
- Modify: `crates/nebula-validator/src/core/validatable.rs`

**Step 1: Add tests to the `serde_json_tests` module**

```rust
#[test]
fn test_value_number_as_i64() {
    let value = json!(42);
    let result: Result<_, _> = AsValidatable::<i64>::as_validatable(&value);
    assert!(result.is_ok());
    assert_eq!(*result.unwrap().borrow(), 42i64);
}

#[test]
fn test_value_float_as_i64_fails() {
    let value = json!(3.14);
    let result: Result<_, _> = AsValidatable::<i64>::as_validatable(&value);
    assert!(result.is_err());
}

#[test]
fn test_value_string_as_i64_fails() {
    let value = json!("42");
    let result: Result<_, _> = AsValidatable::<i64>::as_validatable(&value);
    assert!(result.is_err()); // strict — no coercion
}

#[test]
fn test_value_null_as_i64_fails() {
    let value = json!(null);
    let result: Result<_, _> = AsValidatable::<i64>::as_validatable(&value);
    assert!(result.is_err());
}
```

**Step 2: Run tests — verify they fail**

Run: `cargo test -p nebula-validator --features serde-json test_value_number_as_i64 -- --nocapture`
Expected: FAIL

**Step 3: Add the implementation**

Below the `AsValidatable<str>` impl in the serde-json section:

```rust
#[cfg(feature = "serde-json")]
impl AsValidatable<i64> for serde_json::Value {
    type Output<'a> = i64;

    #[inline]
    fn as_validatable(&self) -> Result<i64, ValidationError> {
        match self {
            serde_json::Value::Number(n) => n.as_i64().ok_or_else(|| {
                ValidationError::new(
                    "type_mismatch",
                    format!("Expected integer, got {n}"),
                )
                .with_param("expected", "integer")
                .with_param("actual", "number")
            }),
            other => Err(ValidationError::new(
                "type_mismatch",
                format!("Expected integer, got {}", json_type_name(other)),
            )
            .with_param("expected", "integer")
            .with_param("actual", json_type_name(other))),
        }
    }
}
```

**Step 4: Run tests — verify they pass**

Run: `cargo test -p nebula-validator --features serde-json serde_json_tests -- --nocapture`
Expected: all tests PASS

**Step 5: Commit**

```bash
git add crates/nebula-validator/src/core/validatable.rs
git commit -m "feat(nebula-validator): AsValidatable<i64> for serde_json::Value"
```

---

### Task 5: `AsValidatable<f64>` for `Value` — Test + Implementation

**Files:**
- Modify: `crates/nebula-validator/src/core/validatable.rs`

**Step 1: Add tests to the `serde_json_tests` module**

```rust
#[test]
fn test_value_float_as_f64() {
    let value = json!(3.14);
    let result: Result<_, _> = AsValidatable::<f64>::as_validatable(&value);
    assert!(result.is_ok());
    let f: f64 = *result.unwrap().borrow();
    assert!((f - 3.14).abs() < f64::EPSILON);
}

#[test]
fn test_value_integer_as_f64() {
    // integers can widen to f64
    let value = json!(42);
    let result: Result<_, _> = AsValidatable::<f64>::as_validatable(&value);
    assert!(result.is_ok());
    assert_eq!(*result.unwrap().borrow(), 42.0);
}

#[test]
fn test_value_string_as_f64_fails() {
    let value = json!("3.14");
    let result: Result<_, _> = AsValidatable::<f64>::as_validatable(&value);
    assert!(result.is_err()); // strict — no coercion
}
```

**Step 2: Run tests — verify they fail**

Run: `cargo test -p nebula-validator --features serde-json test_value_float_as_f64 -- --nocapture`
Expected: FAIL

**Step 3: Add the implementation**

```rust
#[cfg(feature = "serde-json")]
impl AsValidatable<f64> for serde_json::Value {
    type Output<'a> = f64;

    #[inline]
    fn as_validatable(&self) -> Result<f64, ValidationError> {
        match self {
            serde_json::Value::Number(n) => n.as_f64().ok_or_else(|| {
                ValidationError::new(
                    "type_mismatch",
                    format!("Expected number, got {n}"),
                )
                .with_param("expected", "number")
                .with_param("actual", "number")
            }),
            other => Err(ValidationError::new(
                "type_mismatch",
                format!("Expected number, got {}", json_type_name(other)),
            )
            .with_param("expected", "number")
            .with_param("actual", json_type_name(other))),
        }
    }
}
```

**Step 4: Run tests — verify they pass**

Run: `cargo test -p nebula-validator --features serde-json serde_json_tests -- --nocapture`
Expected: all tests PASS

**Step 5: Commit**

```bash
git add crates/nebula-validator/src/core/validatable.rs
git commit -m "feat(nebula-validator): AsValidatable<f64> for serde_json::Value"
```

---

### Task 6: `AsValidatable<bool>` and `AsValidatable<Vec<Value>>` — Test + Implementation

**Files:**
- Modify: `crates/nebula-validator/src/core/validatable.rs`

**Step 1: Add tests**

```rust
#[test]
fn test_value_bool_as_bool() {
    let value = json!(true);
    let result: Result<_, _> = AsValidatable::<bool>::as_validatable(&value);
    assert!(result.is_ok());
    assert!(*result.unwrap().borrow());
}

#[test]
fn test_value_string_as_bool_fails() {
    let value = json!("true");
    let result: Result<_, _> = AsValidatable::<bool>::as_validatable(&value);
    assert!(result.is_err()); // strict — no coercion
}

#[test]
fn test_value_array_as_vec() {
    let value = json!([1, 2, 3]);
    let result = AsValidatable::<Vec<serde_json::Value>>::as_validatable(&value);
    assert!(result.is_ok());
}

#[test]
fn test_value_string_as_vec_fails() {
    let value = json!("not an array");
    let result = AsValidatable::<Vec<serde_json::Value>>::as_validatable(&value);
    assert!(result.is_err());
}
```

**Step 2: Run tests — verify they fail**

Run: `cargo test -p nebula-validator --features serde-json test_value_bool -- --nocapture`
Expected: FAIL

**Step 3: Add both implementations**

```rust
#[cfg(feature = "serde-json")]
impl AsValidatable<bool> for serde_json::Value {
    type Output<'a> = bool;

    #[inline]
    fn as_validatable(&self) -> Result<bool, ValidationError> {
        match self {
            serde_json::Value::Bool(b) => Ok(*b),
            other => Err(ValidationError::new(
                "type_mismatch",
                format!("Expected boolean, got {}", json_type_name(other)),
            )
            .with_param("expected", "boolean")
            .with_param("actual", json_type_name(other))),
        }
    }
}

#[cfg(feature = "serde-json")]
impl AsValidatable<Vec<serde_json::Value>> for serde_json::Value {
    type Output<'a>
        = &'a Vec<serde_json::Value>
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&Vec<serde_json::Value>, ValidationError> {
        match self {
            serde_json::Value::Array(arr) => Ok(arr),
            other => Err(ValidationError::new(
                "type_mismatch",
                format!("Expected array, got {}", json_type_name(other)),
            )
            .with_param("expected", "array")
            .with_param("actual", json_type_name(other))),
        }
    }
}
```

Note: We use `AsValidatable<Vec<Value>>` (not `AsValidatable<[Value]>`) because collection validators like `MinSize<T>` have `type Input = Vec<T>`. The existing `AsValidatable<[T]> for Vec<T>` impl handles the `Each` combinator's `type Input = [T]` case.

**Step 4: Run tests — verify they pass**

Run: `cargo test -p nebula-validator --features serde-json serde_json_tests -- --nocapture`
Expected: all tests PASS

**Step 5: Verify no regressions**

Run: `cargo test -p nebula-validator`
Expected: all existing tests PASS

**Step 6: Commit**

```bash
git add crates/nebula-validator/src/core/validatable.rs
git commit -m "feat(nebula-validator): AsValidatable<bool> and AsValidatable<Vec<Value>> for Value"
```

---

### Task 7: `JsonPath` + `PathSegment` — Tests

**Files:**
- Create: `crates/nebula-validator/src/combinators/json_field.rs`
- Modify: `crates/nebula-validator/src/combinators/mod.rs`

**Step 1: Create the file with types and tests (no implementation yet)**

Create `crates/nebula-validator/src/combinators/json_field.rs`:

```rust
//! JSON field combinator for validating fields within `serde_json::Value`.
//!
//! Provides `JsonField<V>` — a combinator that extracts a value from a JSON
//! structure by path and validates it with an inner validator.

use crate::core::{Validate, ValidationError};
use std::fmt;

// ============================================================================
// PATH TYPES
// ============================================================================

/// A single segment in a JSON path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    /// Object key access, e.g. `"server"`.
    Key(String),
    /// Array index access, e.g. `0`.
    Index(usize),
}

/// A parsed JSON path for field traversal.
///
/// Supports dot notation for object keys and bracket notation for array indices:
/// - `"server.port"` — nested object access
/// - `"items[0].name"` — array index + nested key
/// - `"a.b[2].c"` — mixed traversal
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonPath {
    segments: Vec<PathSegment>,
    original: String,
}

impl JsonPath {
    /// Parses a path string into segments.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` with code `invalid_path` if the path
    /// contains invalid syntax (unclosed brackets, non-numeric index, empty path).
    pub fn parse(path: &str) -> Result<Self, ValidationError> {
        todo!("implement path parsing")
    }

    /// Resolves the path against a JSON value tree.
    ///
    /// # Errors
    ///
    /// - `path_not_found` — a key does not exist in an object
    /// - `index_out_of_bounds` — an array index exceeds the array length
    /// - `type_mismatch` — path expects object/array but value is a scalar
    pub fn resolve<'a>(
        &self,
        root: &'a serde_json::Value,
    ) -> Result<&'a serde_json::Value, ValidationError> {
        todo!("implement path resolution")
    }

    /// Returns the original path string.
    pub fn as_str(&self) -> &str {
        &self.original
    }
}

impl fmt::Display for JsonPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.original)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- Path parsing tests ---

    #[test]
    fn test_parse_simple_key() {
        let path = JsonPath::parse("name").unwrap();
        assert_eq!(path.segments, vec![PathSegment::Key("name".into())]);
    }

    #[test]
    fn test_parse_dotted_path() {
        let path = JsonPath::parse("server.port").unwrap();
        assert_eq!(
            path.segments,
            vec![
                PathSegment::Key("server".into()),
                PathSegment::Key("port".into()),
            ]
        );
    }

    #[test]
    fn test_parse_bracket_index() {
        let path = JsonPath::parse("items[0]").unwrap();
        assert_eq!(
            path.segments,
            vec![
                PathSegment::Key("items".into()),
                PathSegment::Index(0),
            ]
        );
    }

    #[test]
    fn test_parse_complex_path() {
        let path = JsonPath::parse("a.b[2].c").unwrap();
        assert_eq!(
            path.segments,
            vec![
                PathSegment::Key("a".into()),
                PathSegment::Key("b".into()),
                PathSegment::Index(2),
                PathSegment::Key("c".into()),
            ]
        );
    }

    #[test]
    fn test_parse_empty_path_fails() {
        assert!(JsonPath::parse("").is_err());
    }

    #[test]
    fn test_parse_unclosed_bracket_fails() {
        assert!(JsonPath::parse("items[0").is_err());
    }

    #[test]
    fn test_parse_non_numeric_index_fails() {
        assert!(JsonPath::parse("items[abc]").is_err());
    }

    // --- Path resolution tests ---

    #[test]
    fn test_resolve_simple_key() {
        let data = json!({"name": "Alice"});
        let path = JsonPath::parse("name").unwrap();
        let result = path.resolve(&data).unwrap();
        assert_eq!(result, &json!("Alice"));
    }

    #[test]
    fn test_resolve_nested_keys() {
        let data = json!({"server": {"port": 8080}});
        let path = JsonPath::parse("server.port").unwrap();
        let result = path.resolve(&data).unwrap();
        assert_eq!(result, &json!(8080));
    }

    #[test]
    fn test_resolve_array_index() {
        let data = json!({"items": ["a", "b", "c"]});
        let path = JsonPath::parse("items[1]").unwrap();
        let result = path.resolve(&data).unwrap();
        assert_eq!(result, &json!("b"));
    }

    #[test]
    fn test_resolve_complex_path() {
        let data = json!({"servers": [{"host": "localhost", "port": 8080}]});
        let path = JsonPath::parse("servers[0].port").unwrap();
        let result = path.resolve(&data).unwrap();
        assert_eq!(result, &json!(8080));
    }

    #[test]
    fn test_resolve_missing_key() {
        let data = json!({"name": "Alice"});
        let path = JsonPath::parse("age").unwrap();
        let err = path.resolve(&data).unwrap_err();
        assert_eq!(err.code.as_ref(), "path_not_found");
    }

    #[test]
    fn test_resolve_index_out_of_bounds() {
        let data = json!({"items": [1, 2]});
        let path = JsonPath::parse("items[5]").unwrap();
        let err = path.resolve(&data).unwrap_err();
        assert_eq!(err.code.as_ref(), "index_out_of_bounds");
    }

    #[test]
    fn test_resolve_key_on_non_object() {
        let data = json!("not an object");
        let path = JsonPath::parse("key").unwrap();
        let err = path.resolve(&data).unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
    }

    #[test]
    fn test_resolve_index_on_non_array() {
        let data = json!({"items": "not an array"});
        let path = JsonPath::parse("items[0]").unwrap();
        let err = path.resolve(&data).unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
    }
}
```

**Step 2: Register the module in `combinators/mod.rs`**

Add at the end of the module declarations (after `pub mod when;`):

```rust
#[cfg(feature = "serde-json")]
pub mod json_field;
```

Add to the re-exports (after the `when` re-export):

```rust
#[cfg(feature = "serde-json")]
pub use json_field::{JsonField, JsonPath, PathSegment, json_field, json_field_optional};
```

Add to the prelude module:

```rust
#[cfg(feature = "serde-json")]
pub use super::{JsonField, JsonPath, PathSegment, json_field, json_field_optional};
```

**Step 3: Run tests — verify they fail**

Run: `cargo test -p nebula-validator --features serde-json json_field -- --nocapture`
Expected: FAIL with `not yet implemented` panics

**Step 4: Commit**

```bash
git add crates/nebula-validator/src/combinators/json_field.rs crates/nebula-validator/src/combinators/mod.rs
git commit -m "test(nebula-validator): add JsonPath parsing and resolution tests (red)"
```

---

### Task 8: `JsonPath` — Implementation

**Files:**
- Modify: `crates/nebula-validator/src/combinators/json_field.rs`

**Step 1: Implement `JsonPath::parse`**

Replace the `todo!()` in `parse`:

```rust
pub fn parse(path: &str) -> Result<Self, ValidationError> {
    if path.is_empty() {
        return Err(
            ValidationError::new("invalid_path", "Path must not be empty")
                .with_param("path", path.to_string()),
        );
    }

    let mut segments = Vec::new();
    let mut current = String::new();

    let mut chars = path.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !current.is_empty() {
                    segments.push(PathSegment::Key(std::mem::take(&mut current)));
                }
            }
            '[' => {
                if !current.is_empty() {
                    segments.push(PathSegment::Key(std::mem::take(&mut current)));
                }
                let mut index_str = String::new();
                loop {
                    match chars.next() {
                        Some(']') => break,
                        Some(c) => index_str.push(c),
                        None => {
                            return Err(ValidationError::new(
                                "invalid_path",
                                format!("Unclosed bracket in path '{path}'"),
                            )
                            .with_param("path", path.to_string()));
                        }
                    }
                }
                let index: usize = index_str.parse().map_err(|_| {
                    ValidationError::new(
                        "invalid_path",
                        format!("Non-numeric array index '[{index_str}]' in path '{path}'"),
                    )
                    .with_param("path", path.to_string())
                })?;
                segments.push(PathSegment::Index(index));
            }
            _ => current.push(ch),
        }
    }

    if !current.is_empty() {
        segments.push(PathSegment::Key(current));
    }

    if segments.is_empty() {
        return Err(
            ValidationError::new("invalid_path", "Path must not be empty")
                .with_param("path", path.to_string()),
        );
    }

    Ok(Self {
        segments,
        original: path.to_string(),
    })
}
```

**Step 2: Implement `JsonPath::resolve`**

Replace the `todo!()` in `resolve`:

```rust
pub fn resolve<'a>(
    &self,
    root: &'a serde_json::Value,
) -> Result<&'a serde_json::Value, ValidationError> {
    let mut current = root;

    for (i, segment) in self.segments.iter().enumerate() {
        let traversed = self
            .segments[..i]
            .iter()
            .map(|s| match s {
                PathSegment::Key(k) => k.clone(),
                PathSegment::Index(idx) => format!("[{idx}]"),
            })
            .collect::<Vec<_>>()
            .join(".");

        match segment {
            PathSegment::Key(key) => match current {
                serde_json::Value::Object(map) => {
                    current = map.get(key.as_str()).ok_or_else(|| {
                        ValidationError::new(
                            "path_not_found",
                            format!("Key '{}' not found at '{}'", key, self.original),
                        )
                        .with_field(self.original.clone())
                        .with_param("path", self.original.clone())
                    })?;
                }
                other => {
                    return Err(ValidationError::new(
                        "type_mismatch",
                        format!(
                            "Expected object at '{}', got {}",
                            if traversed.is_empty() {
                                "<root>".to_string()
                            } else {
                                traversed
                            },
                            json_type_name(other),
                        ),
                    )
                    .with_field(self.original.clone()));
                }
            },
            PathSegment::Index(idx) => match current {
                serde_json::Value::Array(arr) => {
                    current = arr.get(*idx).ok_or_else(|| {
                        ValidationError::new(
                            "index_out_of_bounds",
                            format!(
                                "Index {} out of bounds (length {}) at '{}'",
                                idx,
                                arr.len(),
                                self.original,
                            ),
                        )
                        .with_field(self.original.clone())
                        .with_param("index", idx.to_string())
                        .with_param("path", self.original.clone())
                    })?;
                }
                other => {
                    return Err(ValidationError::new(
                        "type_mismatch",
                        format!(
                            "Expected array at '{}', got {}",
                            if traversed.is_empty() {
                                "<root>".to_string()
                            } else {
                                traversed
                            },
                            json_type_name(other),
                        ),
                    )
                    .with_field(self.original.clone()));
                }
            },
        }
    }

    Ok(current)
}
```

Note: The `json_type_name` helper is in `core/validatable.rs`. We need to make it `pub(crate)` so `json_field.rs` can use it. Go to `core/validatable.rs` and change:

```rust
#[cfg(feature = "serde-json")]
fn json_type_name(value: &serde_json::Value) -> &'static str {
```

to:

```rust
#[cfg(feature = "serde-json")]
pub(crate) fn json_type_name(value: &serde_json::Value) -> &'static str {
```

Then in `json_field.rs`, add the import at the top:

```rust
#[cfg(feature = "serde-json")]
use crate::core::validatable::json_type_name;
```

Wait — the whole file is already `#[cfg(feature = "serde-json")]`-gated via `mod.rs`, so the import doesn't need the attribute. Just add:

```rust
use crate::core::validatable::json_type_name;
```

**Step 3: Run tests**

Run: `cargo test -p nebula-validator --features serde-json json_field::tests -- --nocapture`
Expected: all parsing + resolution tests PASS

**Step 4: Commit**

```bash
git add crates/nebula-validator/src/combinators/json_field.rs crates/nebula-validator/src/core/validatable.rs
git commit -m "feat(nebula-validator): implement JsonPath parsing and resolution"
```

---

### Task 9: `JsonField<V>` Combinator — Tests

**Files:**
- Modify: `crates/nebula-validator/src/combinators/json_field.rs`

**Step 1: Add `JsonField` struct stub and tests**

Add after the `JsonPath` impl block (before `#[cfg(test)]`):

```rust
// ============================================================================
// JSON FIELD COMBINATOR
// ============================================================================

/// Validates a field within a JSON value by path.
///
/// Extracts a value at the given path and applies the inner validator.
/// Supports required (default) and optional modes.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::combinators::json_field::json_field;
/// use nebula_validator::validators::string::min_length;
/// use nebula_validator::core::Validate;
///
/// let validator = json_field("server.host", min_length(1)).unwrap();
/// assert!(validator.validate(&json!({"server": {"host": "localhost"}})).is_ok());
/// ```
pub struct JsonField<V> {
    path: JsonPath,
    validator: V,
    required: bool,
}

impl<V> JsonField<V> {
    /// Creates a required field validator.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` if the path syntax is invalid.
    pub fn new(path: &str, validator: V) -> Result<Self, ValidationError> {
        todo!()
    }

    /// Creates an optional field validator.
    ///
    /// Missing fields and `null` values pass validation.
    ///
    /// # Errors
    ///
    /// Returns `ValidationError` if the path syntax is invalid.
    pub fn optional(path: &str, validator: V) -> Result<Self, ValidationError> {
        todo!()
    }
}

/// Creates a required JSON field validator.
pub fn json_field<V>(path: &str, validator: V) -> Result<JsonField<V>, ValidationError> {
    JsonField::new(path, validator)
}

/// Creates an optional JSON field validator.
pub fn json_field_optional<V>(path: &str, validator: V) -> Result<JsonField<V>, ValidationError> {
    JsonField::optional(path, validator)
}
```

Add these tests to the `tests` module:

```rust
// --- JsonField tests ---

#[test]
fn test_json_field_required_valid() {
    use crate::validators::string::min_length;

    let data = json!({"name": "Alice"});
    let validator = json_field("name", min_length(3)).unwrap();
    assert!(validator.validate(&data).is_ok());
}

#[test]
fn test_json_field_required_invalid() {
    use crate::validators::string::min_length;

    let data = json!({"name": "Al"});
    let validator = json_field("name", min_length(3)).unwrap();
    let err = validator.validate(&data).unwrap_err();
    assert!(err.field.is_some());
    assert_eq!(err.field.as_deref().unwrap(), "name");
}

#[test]
fn test_json_field_required_missing() {
    use crate::validators::string::min_length;

    let data = json!({"age": 25});
    let validator = json_field("name", min_length(3)).unwrap();
    assert!(validator.validate(&data).is_err());
}

#[test]
fn test_json_field_optional_missing_passes() {
    use crate::validators::string::min_length;

    let data = json!({"age": 25});
    let validator = json_field_optional("name", min_length(3)).unwrap();
    assert!(validator.validate(&data).is_ok());
}

#[test]
fn test_json_field_optional_null_passes() {
    use crate::validators::string::min_length;

    let data = json!({"name": null});
    let validator = json_field_optional("name", min_length(3)).unwrap();
    assert!(validator.validate(&data).is_ok());
}

#[test]
fn test_json_field_optional_present_invalid() {
    use crate::validators::string::min_length;

    let data = json!({"name": "Al"});
    let validator = json_field_optional("name", min_length(3)).unwrap();
    assert!(validator.validate(&data).is_err());
}

#[test]
fn test_json_field_nested_path() {
    use crate::validators::string::min_length;

    let data = json!({"server": {"host": "localhost"}});
    let validator = json_field("server.host", min_length(1)).unwrap();
    assert!(validator.validate(&data).is_ok());
}

#[test]
fn test_json_field_with_array_index() {
    use crate::validators::string::min_length;

    let data = json!({"tags": ["web", "api"]});
    let validator = json_field("tags[0]", min_length(1)).unwrap();
    assert!(validator.validate(&data).is_ok());
}

#[test]
fn test_json_field_composition_and() {
    use crate::core::ValidateExt;
    use crate::validators::string::min_length;

    let data = json!({"first": "Alice", "last": "Smith"});
    let v = json_field("first", min_length(1))
        .unwrap()
        .and(json_field("last", min_length(1)).unwrap());
    assert!(v.validate(&data).is_ok());
}

#[test]
fn test_json_field_type_mismatch_error() {
    use crate::validators::string::min_length;

    let data = json!({"name": 42});
    let validator = json_field("name", min_length(1)).unwrap();
    let err = validator.validate(&data).unwrap_err();
    assert!(err.field.is_some());
}
```

**Step 2: Run tests — verify they fail**

Run: `cargo test -p nebula-validator --features serde-json test_json_field -- --nocapture`
Expected: FAIL with `not yet implemented` panics

**Step 3: Commit**

```bash
git add crates/nebula-validator/src/combinators/json_field.rs
git commit -m "test(nebula-validator): add JsonField combinator tests (red)"
```

---

### Task 10: `JsonField<V>` Combinator — Implementation

**Files:**
- Modify: `crates/nebula-validator/src/combinators/json_field.rs`

**Step 1: Implement constructors**

Replace the `todo!()` in `JsonField::new` and `JsonField::optional`:

```rust
pub fn new(path: &str, validator: V) -> Result<Self, ValidationError> {
    Ok(Self {
        path: JsonPath::parse(path)?,
        validator,
        required: true,
    })
}

pub fn optional(path: &str, validator: V) -> Result<Self, ValidationError> {
    Ok(Self {
        path: JsonPath::parse(path)?,
        validator,
        required: false,
    })
}
```

**Step 2: Implement `Validate` for `JsonField<V>`**

Add after the `json_field_optional` function:

```rust
use crate::core::validatable::AsValidatable;
use std::borrow::Borrow;

impl<V> Validate for JsonField<V>
where
    V: Validate,
    serde_json::Value: AsValidatable<V::Input>,
{
    type Input = serde_json::Value;

    fn validate(&self, input: &serde_json::Value) -> Result<(), ValidationError> {
        match self.path.resolve(input) {
            Ok(value) => {
                if !self.required && value.is_null() {
                    return Ok(());
                }
                let converted = AsValidatable::<V::Input>::as_validatable(value)
                    .map_err(|e| e.with_field(self.path.original.clone()))?;
                self.validator
                    .validate(converted.borrow())
                    .map_err(|e| e.with_field(self.path.original.clone()))
            }
            Err(_) if !self.required => Ok(()),
            Err(e) => Err(e),
        }
    }
}
```

Also add `Debug` and `Clone` impls:

```rust
impl<V: fmt::Debug> fmt::Debug for JsonField<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JsonField")
            .field("path", &self.path)
            .field("validator", &self.validator)
            .field("required", &self.required)
            .finish()
    }
}

impl<V: Clone> Clone for JsonField<V> {
    fn clone(&self) -> Self {
        Self {
            path: self.path.clone(),
            validator: self.validator.clone(),
            required: self.required,
        }
    }
}
```

**Step 3: Run tests**

Run: `cargo test -p nebula-validator --features serde-json json_field -- --nocapture`
Expected: all JsonField + JsonPath tests PASS

**Step 4: Run all validator tests**

Run: `cargo test -p nebula-validator --features serde-json`
Expected: ALL tests PASS

**Step 5: Run default build (no feature)**

Run: `cargo test -p nebula-validator`
Expected: ALL existing tests PASS, no JSON tests visible

**Step 6: Commit**

```bash
git add crates/nebula-validator/src/combinators/json_field.rs
git commit -m "feat(nebula-validator): implement JsonField combinator"
```

---

### Task 11: Integration Tests

**Files:**
- Create: `crates/nebula-validator/tests/json_integration.rs`

**Step 1: Write integration tests**

Create `crates/nebula-validator/tests/json_integration.rs`:

```rust
//! Integration tests for serde_json::Value validation.
//!
//! Tests end-to-end usage with real validators from the library.

#![cfg(feature = "serde-json")]

use nebula_validator::combinators::{json_field, json_field_optional, JsonField};
use nebula_validator::core::{Validate, ValidateExt};
use nebula_validator::validators::string::min_length;
use serde_json::json;

#[test]
fn test_validate_string_value_directly() {
    let validator = min_length(3);
    assert!(validator.validate_any(&json!("hello")).is_ok());
    assert!(validator.validate_any(&json!("hi")).is_err());
}

#[test]
fn test_validate_config_structure() {
    let data = json!({
        "server": {
            "host": "localhost",
            "port": 8080,
            "tags": ["web", "api"]
        },
        "database": {
            "url": "postgres://localhost/db"
        }
    });

    let host = json_field("server.host", min_length(1)).unwrap();
    let db_url = json_field("database.url", min_length(5)).unwrap();

    let combined = host.and(db_url);
    assert!(combined.validate(&data).is_ok());
}

#[test]
fn test_validate_array_element() {
    let data = json!({
        "servers": [
            {"host": "web1", "port": 80},
            {"host": "web2", "port": 443}
        ]
    });

    let first_host = json_field("servers[0].host", min_length(1)).unwrap();
    assert!(first_host.validate(&data).is_ok());

    let second_host = json_field("servers[1].host", min_length(1)).unwrap();
    assert!(second_host.validate(&data).is_ok());
}

#[test]
fn test_optional_field_missing() {
    let data = json!({"name": "Alice"});
    let optional = json_field_optional("email", min_length(5)).unwrap();
    assert!(optional.validate(&data).is_ok());
}

#[test]
fn test_type_mismatch_gives_clear_error() {
    let data = json!({"port": "not a number"});
    let validator = min_length(1); // string validator
    // This works because "not a number" IS a string
    assert!(validator.validate_any(&json!("not a number")).is_ok());

    // But a number passed to string validator fails
    let err = validator.validate_any(&json!(42)).unwrap_err();
    assert_eq!(err.code.as_ref(), "type_mismatch");
    assert!(err.message.contains("string"));
}

#[test]
fn test_null_handling() {
    let validator = min_length(1);
    let err = validator.validate_any(&json!(null)).unwrap_err();
    assert_eq!(err.code.as_ref(), "type_mismatch");
}
```

**Step 2: Run integration tests**

Run: `cargo test -p nebula-validator --features serde-json --test json_integration -- --nocapture`
Expected: all tests PASS

**Step 3: Commit**

```bash
git add crates/nebula-validator/tests/json_integration.rs
git commit -m "test(nebula-validator): add JSON integration tests"
```

---

### Task 12: Quality Gates and Documentation

**Files:**
- Modify: `crates/nebula-validator/src/combinators/json_field.rs` (doc comments)
- Modify: `crates/nebula-validator/src/core/validatable.rs` (doc comments)

**Step 1: Run clippy**

Run: `cargo clippy -p nebula-validator --features serde-json -- -D warnings`
Expected: no warnings. If there are, fix them.

**Step 2: Run fmt**

Run: `cargo fmt --all -- --check`
Expected: no formatting issues. If there are, run `cargo fmt --all`.

**Step 3: Run doc build**

Run: `cargo doc -p nebula-validator --features serde-json --no-deps`
Expected: builds with no warnings

**Step 4: Run full workspace test suite**

Run: `cargo test --workspace`
Expected: all tests PASS (no regressions)

**Step 5: Run feature-enabled tests one final time**

Run: `cargo test -p nebula-validator --features serde-json`
Expected: all tests PASS

**Step 6: Commit any doc/lint fixes**

```bash
git add -A
git commit -m "docs(nebula-validator): add rustdoc for JSON validation support"
```

---

## Summary

| Task | What | Files | Commit |
|------|------|-------|--------|
| 1 | Feature flag | `Cargo.toml` | `feat: add serde-json feature flag` |
| 2 | `AsValidatable<str>` tests | `validatable.rs` | (merged with 3) |
| 3 | `AsValidatable<str>` impl | `validatable.rs` | `feat: AsValidatable<str> for Value` |
| 4 | `AsValidatable<i64>` | `validatable.rs` | `feat: AsValidatable<i64> for Value` |
| 5 | `AsValidatable<f64>` | `validatable.rs` | `feat: AsValidatable<f64> for Value` |
| 6 | `AsValidatable<bool>` + `Vec<Value>` | `validatable.rs` | `feat: AsValidatable<bool,Vec> for Value` |
| 7 | `JsonPath` tests (red) | `json_field.rs`, `mod.rs` | `test: JsonPath tests (red)` |
| 8 | `JsonPath` impl | `json_field.rs`, `validatable.rs` | `feat: implement JsonPath` |
| 9 | `JsonField` tests (red) | `json_field.rs` | `test: JsonField tests (red)` |
| 10 | `JsonField` impl | `json_field.rs` | `feat: implement JsonField` |
| 11 | Integration tests | `json_integration.rs` | `test: JSON integration tests` |
| 12 | Quality gates + docs | various | `docs: rustdoc for JSON support` |
