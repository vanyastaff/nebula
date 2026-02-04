//! Path-based access for Value
//!
//! Supports JSON path-like syntax for navigating nested structures:
//! - `user.name` - object key access
//! - `items[0]` - array index access
//! - `data[0].value` - chained access
//! - `matrix[0][1]` - multiple index access
//!
//! # Examples
//!
//! ```
//! use nebula_value::Value;
//! use nebula_value::collections::{Array, Object};
//!
//! // Create nested structure
//! let user = Object::from_iter(vec![
//!     ("name".to_string(), Value::text("Alice")),
//!     ("age".to_string(), Value::integer(30)),
//! ]);
//! let data = Object::from_iter(vec![
//!     ("user".to_string(), Value::Object(user)),
//! ]);
//! let root = Value::Object(data);
//!
//! // Access nested values
//! assert_eq!(root.get_path("user.name").unwrap(), Value::text("Alice"));
//! assert_eq!(root.get_path("user.age").unwrap(), Value::integer(30));
//! ```

use std::fmt;

use crate::collections::{Array, Object};
use crate::core::value::Value;
use crate::core::{ValueError, ValueResult};

// ============================================================================
// PATH SEGMENT
// ============================================================================

/// A single segment in a path expression
///
/// Paths are composed of segments that can be either:
/// - Key access: `.key` or just `key`
/// - Index access: `[0]`, `[1]`, etc.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSegment {
    /// Object key access: `.key`
    Key(String),
    /// Array index access: `[index]`
    Index(usize),
}

impl PathSegment {
    /// Create a key segment
    pub fn key(key: impl Into<String>) -> Self {
        Self::Key(key.into())
    }

    /// Create an index segment
    pub const fn index(index: usize) -> Self {
        Self::Index(index)
    }

    /// Check if this is a key segment
    pub fn is_key(&self) -> bool {
        matches!(self, Self::Key(_))
    }

    /// Check if this is an index segment
    pub fn is_index(&self) -> bool {
        matches!(self, Self::Index(_))
    }

    /// Get the key if this is a key segment
    pub fn as_key(&self) -> Option<&str> {
        match self {
            Self::Key(k) => Some(k),
            Self::Index(_) => None,
        }
    }

    /// Get the index if this is an index segment
    pub fn as_index(&self) -> Option<usize> {
        match self {
            Self::Key(_) => None,
            Self::Index(i) => Some(*i),
        }
    }
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(k) => write!(f, ".{}", k),
            Self::Index(i) => write!(f, "[{}]", i),
        }
    }
}

impl From<&str> for PathSegment {
    fn from(s: &str) -> Self {
        Self::Key(s.to_string())
    }
}

impl From<String> for PathSegment {
    fn from(s: String) -> Self {
        Self::Key(s)
    }
}

impl From<usize> for PathSegment {
    fn from(i: usize) -> Self {
        Self::Index(i)
    }
}

// ============================================================================
// PATH TYPE
// ============================================================================

/// A complete path for navigating nested values
///
/// Paths are immutable and can be efficiently cloned.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Path {
    segments: Vec<PathSegment>,
}

impl Path {
    /// Create an empty path (refers to root)
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    /// Create a path from segments
    pub fn from_segments(segments: Vec<PathSegment>) -> Self {
        Self { segments }
    }

    /// Parse a path string
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Path has more than 100 segments (DoS protection)
    /// - Index parsing fails (e.g., `[abc]`)
    /// - Unclosed brackets
    pub fn parse(path: &str) -> ValueResult<Self> {
        let segments = parse_path(path)?;
        Ok(Self { segments })
    }

    /// Check if this is the root path (empty)
    pub fn is_root(&self) -> bool {
        self.segments.is_empty()
    }

    /// Get the number of segments
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Check if path is empty
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Get segments as slice
    pub fn segments(&self) -> &[PathSegment] {
        &self.segments
    }

    /// Get the first segment
    pub fn first(&self) -> Option<&PathSegment> {
        self.segments.first()
    }

    /// Get the last segment
    pub fn last(&self) -> Option<&PathSegment> {
        self.segments.last()
    }

    /// Create a new path with an additional segment
    pub fn push(&self, segment: impl Into<PathSegment>) -> Self {
        let mut segments = self.segments.clone();
        segments.push(segment.into());
        Self { segments }
    }

    /// Create a new path with a key segment appended
    pub fn key(&self, key: impl Into<String>) -> Self {
        self.push(PathSegment::Key(key.into()))
    }

    /// Create a new path with an index segment appended
    pub fn index(&self, index: usize) -> Self {
        self.push(PathSegment::Index(index))
    }

    /// Get the parent path (without the last segment)
    pub fn parent(&self) -> Option<Self> {
        if self.segments.is_empty() {
            None
        } else {
            Some(Self {
                segments: self.segments[..self.segments.len() - 1].to_vec(),
            })
        }
    }

    /// Get tail (all segments except the first)
    pub fn tail(&self) -> Self {
        if self.segments.is_empty() {
            Self::new()
        } else {
            Self {
                segments: self.segments[1..].to_vec(),
            }
        }
    }

    /// Iterate over segments
    pub fn iter(&self) -> impl Iterator<Item = &PathSegment> {
        self.segments.iter()
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, segment) in self.segments.iter().enumerate() {
            match segment {
                PathSegment::Key(k) => {
                    if i > 0 {
                        write!(f, ".{}", k)?;
                    } else {
                        write!(f, "{}", k)?;
                    }
                }
                PathSegment::Index(idx) => write!(f, "[{}]", idx)?,
            }
        }
        Ok(())
    }
}

impl FromIterator<PathSegment> for Path {
    fn from_iter<I: IntoIterator<Item = PathSegment>>(iter: I) -> Self {
        Self {
            segments: iter.into_iter().collect(),
        }
    }
}

// ============================================================================
// PATH PARSING
// ============================================================================

/// Maximum number of path segments allowed (DoS protection)
const MAX_PATH_SEGMENTS: usize = 100;

/// Maximum path string length (DoS protection)
const MAX_PATH_LENGTH: usize = 4096;

/// Parse a path string into segments
///
/// Supported syntax:
/// - `user` -> `[Key("user")]`
/// - `user.name` -> `[Key("user"), Key("name")]`
/// - `items[0]` -> `[Key("items"), Index(0)]`
/// - `data[0].value` -> `[Key("data"), Index(0), Key("value")]`
/// - `[0]` -> `[Index(0)]` (root array access)
/// - `[0][1]` -> `[Index(0), Index(1)]` (multi-index)
///
/// # Errors
///
/// Returns error if:
/// - Path exceeds MAX_PATH_LENGTH characters
/// - Path has more than MAX_PATH_SEGMENTS segments
/// - Index parsing fails
/// - Unclosed brackets
fn parse_path(path: &str) -> ValueResult<Vec<PathSegment>> {
    // DoS protection: limit path length
    if path.len() > MAX_PATH_LENGTH {
        return Err(ValueError::limit_exceeded(
            "path length",
            MAX_PATH_LENGTH,
            path.len(),
        ));
    }

    // Empty path = root
    if path.is_empty() {
        return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    let mut current = String::new();
    let chars = path.chars().peekable();
    let mut in_bracket = false;
    let mut bracket_content = String::new();

    for ch in chars {
        match ch {
            '.' if !in_bracket => {
                // Flush current key
                if !current.is_empty() {
                    segments.push(PathSegment::Key(current.clone()));
                    current.clear();
                    check_segment_limit(&segments)?;
                }
                // Skip leading dots
            }
            '[' if !in_bracket => {
                // Flush current key before bracket
                if !current.is_empty() {
                    segments.push(PathSegment::Key(current.clone()));
                    current.clear();
                    check_segment_limit(&segments)?;
                }
                in_bracket = true;
                bracket_content.clear();
            }
            ']' if in_bracket => {
                // Parse index from bracket content
                let content = bracket_content.trim();

                // Check for quoted string keys: ["key"] or ['key']
                if (content.starts_with('"') && content.ends_with('"'))
                    || (content.starts_with('\'') && content.ends_with('\''))
                {
                    // Remove quotes
                    let key = &content[1..content.len() - 1];
                    segments.push(PathSegment::Key(key.to_string()));
                } else {
                    // Parse as index
                    let index = content
                        .parse::<usize>()
                        .map_err(|_| ValueError::parse_error("path index", content.to_string()))?;
                    segments.push(PathSegment::Index(index));
                }

                check_segment_limit(&segments)?;
                in_bracket = false;
                bracket_content.clear();
            }
            _ if in_bracket => {
                bracket_content.push(ch);
            }
            _ => {
                current.push(ch);
            }
        }
    }

    // Check for unclosed bracket
    if in_bracket {
        return Err(ValueError::parse_error("path", "unclosed bracket '['"));
    }

    // Flush remaining key
    if !current.is_empty() {
        segments.push(PathSegment::Key(current));
        check_segment_limit(&segments)?;
    }

    Ok(segments)
}

/// Check segment count limit
fn check_segment_limit(segments: &[PathSegment]) -> ValueResult<()> {
    if segments.len() > MAX_PATH_SEGMENTS {
        return Err(ValueError::limit_exceeded(
            "path segments",
            MAX_PATH_SEGMENTS,
            segments.len(),
        ));
    }
    Ok(())
}

// ============================================================================
// VALUE PATH ACCESS IMPLEMENTATION
// ============================================================================

impl Value {
    // ==================== Read Operations ====================

    /// Get a value by path string
    ///
    /// Returns a cloned value at the specified path.
    ///
    /// # Syntax
    ///
    /// - `user.name` - nested object access
    /// - `items[0]` - array index
    /// - `data[0].value` - combined
    /// - `["special-key"]` - quoted key with special characters
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("name".to_string(), Value::text("Alice")),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// assert_eq!(root.get_path("name").unwrap(), Value::text("Alice"));
    /// ```
    ///
    /// # Errors
    ///
    /// - `PathNotFound` if any segment doesn't exist
    /// - `TypeMismatch` if accessing key on non-object or index on non-array
    /// - `ParseError` if path syntax is invalid
    pub fn get_path(&self, path: &str) -> ValueResult<Value> {
        let segments = parse_path(path)?;
        self.get_by_segments(&segments)
    }

    /// Get a reference to a value by path string
    ///
    /// More efficient than `get_path` when you don't need ownership.
    ///
    /// # Errors
    ///
    /// Same as `get_path`
    pub fn get_path_ref(&self, path: &str) -> ValueResult<&Value> {
        let segments = parse_path(path)?;
        self.get_ref_by_segments(&segments)
    }

    /// Get a value by Path object
    pub fn get_by_path(&self, path: &Path) -> ValueResult<Value> {
        self.get_by_segments(path.segments())
    }

    /// Get a reference by Path object
    pub fn get_ref_by_path(&self, path: &Path) -> ValueResult<&Value> {
        self.get_ref_by_segments(path.segments())
    }

    /// Get a value by path segments
    pub fn get_by_segments(&self, segments: &[PathSegment]) -> ValueResult<Value> {
        self.get_ref_by_segments(segments).cloned()
    }

    /// Get a reference by path segments
    pub fn get_ref_by_segments(&self, segments: &[PathSegment]) -> ValueResult<&Value> {
        if segments.is_empty() {
            return Ok(self);
        }

        let mut current = self;

        for (i, segment) in segments.iter().enumerate() {
            current = match (current, segment) {
                (Value::Object(obj), PathSegment::Key(key)) => obj.get(key).ok_or_else(|| {
                    let traversed = format_path(&segments[..=i]);
                    ValueError::path_not_found(traversed)
                })?,
                (Value::Array(arr), PathSegment::Index(idx)) => arr.get(*idx).ok_or_else(|| {
                    ValueError::index_out_of_bounds(*idx, arr.len())
                        .at_path(format_path(&segments[..i]))
                })?,
                (Value::Object(_), PathSegment::Index(idx)) => {
                    return Err(ValueError::type_mismatch("Array", "Object")
                        .with_context(format!("cannot access index [{}] on Object", idx)));
                }
                (Value::Array(_), PathSegment::Key(key)) => {
                    return Err(ValueError::type_mismatch("Object", "Array")
                        .with_context(format!("cannot access key '{}' on Array", key)));
                }
                (val, PathSegment::Key(key)) => {
                    return Err(ValueError::type_mismatch("Object", val.kind().name())
                        .with_context(format!(
                            "cannot access key '{}' on {}",
                            key,
                            val.kind().name()
                        )));
                }
                (val, PathSegment::Index(idx)) => {
                    return Err(ValueError::type_mismatch("Array", val.kind().name())
                        .with_context(format!(
                            "cannot access index [{}] on {}",
                            idx,
                            val.kind().name()
                        )));
                }
            };
        }

        Ok(current)
    }

    /// Check if a path exists
    ///
    /// Returns `true` if the path can be traversed successfully.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("name".to_string(), Value::text("Alice")),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// assert!(root.has_path("name"));
    /// assert!(!root.has_path("age"));
    /// assert!(!root.has_path("name.first")); // "Alice" is not an object
    /// ```
    pub fn has_path(&self, path: &str) -> bool {
        self.get_path(path).is_ok()
    }

    // ==================== Write Operations ====================

    /// Set a value at a path, returning a new Value
    ///
    /// Creates intermediate objects/arrays as needed.
    /// This is an immutable operation - returns a new Value.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let root = Value::Object(Object::new());
    /// let updated = root.set_path("user.name", Value::text("Alice")).unwrap();
    ///
    /// assert_eq!(updated.get_path("user.name").unwrap(), Value::text("Alice"));
    /// ```
    ///
    /// # Errors
    ///
    /// - `TypeMismatch` if path traversal encounters incompatible types
    /// - `ParseError` if path syntax is invalid
    pub fn set_path(&self, path: &str, value: Value) -> ValueResult<Value> {
        let segments = parse_path(path)?;
        self.set_by_segments(&segments, value)
    }

    /// Set a value by Path object
    pub fn set_by_path(&self, path: &Path, value: Value) -> ValueResult<Value> {
        self.set_by_segments(path.segments(), value)
    }

    /// Set a value by path segments
    pub fn set_by_segments(&self, segments: &[PathSegment], value: Value) -> ValueResult<Value> {
        if segments.is_empty() {
            // Setting root = replace entirely
            return Ok(value);
        }

        // Recursive implementation for immutable update
        self.set_recursive(segments, 0, value)
    }

    /// Recursive helper for immutable path setting
    fn set_recursive(
        &self,
        segments: &[PathSegment],
        index: usize,
        value: Value,
    ) -> ValueResult<Value> {
        if index >= segments.len() {
            return Ok(value);
        }

        let segment = &segments[index];
        let is_last = index == segments.len() - 1;

        match (self, segment) {
            (Value::Object(obj), PathSegment::Key(key)) => {
                let new_value = if is_last {
                    value
                } else {
                    // Get existing or create empty container for next segment
                    let existing = obj.get(key).cloned().unwrap_or_else(|| {
                        // Determine type based on next segment
                        match segments.get(index + 1) {
                            Some(PathSegment::Key(_)) => Value::Object(Object::new()),
                            Some(PathSegment::Index(_)) => Value::Array(Array::new()),
                            None => Value::Null,
                        }
                    });
                    existing.set_recursive(segments, index + 1, value)?
                };
                Ok(Value::Object(obj.insert(key.clone(), new_value)))
            }
            (Value::Array(arr), PathSegment::Index(idx)) => {
                if *idx > arr.len() {
                    return Err(ValueError::index_out_of_bounds(*idx, arr.len()));
                }

                let new_value = if is_last {
                    value
                } else {
                    let existing =
                        arr.get(*idx)
                            .cloned()
                            .unwrap_or_else(|| match segments.get(index + 1) {
                                Some(PathSegment::Key(_)) => Value::Object(Object::new()),
                                Some(PathSegment::Index(_)) => Value::Array(Array::new()),
                                None => Value::Null,
                            });
                    existing.set_recursive(segments, index + 1, value)?
                };

                if *idx == arr.len() {
                    // Append
                    Ok(Value::Array(arr.push(new_value)))
                } else {
                    // Update existing
                    Ok(Value::Array(arr.set(*idx, new_value)?))
                }
            }
            (Value::Null, PathSegment::Key(key)) => {
                // Auto-create object
                let obj = Object::new();
                let new_value = if is_last {
                    value
                } else {
                    let next = match segments.get(index + 1) {
                        Some(PathSegment::Key(_)) => Value::Object(Object::new()),
                        Some(PathSegment::Index(_)) => Value::Array(Array::new()),
                        None => Value::Null,
                    };
                    next.set_recursive(segments, index + 1, value)?
                };
                Ok(Value::Object(obj.insert(key.clone(), new_value)))
            }
            (Value::Null, PathSegment::Index(idx)) => {
                // Auto-create array
                if *idx != 0 {
                    return Err(ValueError::index_out_of_bounds(*idx, 0));
                }
                let arr = Array::new();
                let new_value = if is_last {
                    value
                } else {
                    let next = match segments.get(index + 1) {
                        Some(PathSegment::Key(_)) => Value::Object(Object::new()),
                        Some(PathSegment::Index(_)) => Value::Array(Array::new()),
                        None => Value::Null,
                    };
                    next.set_recursive(segments, index + 1, value)?
                };
                Ok(Value::Array(arr.push(new_value)))
            }
            (val, PathSegment::Key(key)) => Err(ValueError::type_mismatch(
                "Object",
                val.kind().name(),
            )
            .with_context(format!("cannot set key '{}' on {}", key, val.kind().name()))),
            (val, PathSegment::Index(idx)) => {
                Err(
                    ValueError::type_mismatch("Array", val.kind().name()).with_context(format!(
                        "cannot set index [{}] on {}",
                        idx,
                        val.kind().name()
                    )),
                )
            }
        }
    }

    /// Remove a value at a path, returning updated Value and removed value
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let obj = Object::from_iter(vec![
    ///     ("name".to_string(), Value::text("Alice")),
    ///     ("age".to_string(), Value::integer(30)),
    /// ]);
    /// let root = Value::Object(obj);
    ///
    /// let (updated, removed) = root.remove_path("age").unwrap();
    /// assert_eq!(removed, Value::integer(30));
    /// assert!(!updated.has_path("age"));
    /// ```
    ///
    /// # Errors
    ///
    /// - `PathNotFound` if path doesn't exist
    /// - `TypeMismatch` if path traversal encounters incompatible types
    pub fn remove_path(&self, path: &str) -> ValueResult<(Value, Value)> {
        let segments = parse_path(path)?;
        self.remove_by_segments(&segments)
    }

    /// Remove by Path object
    pub fn remove_by_path(&self, path: &Path) -> ValueResult<(Value, Value)> {
        self.remove_by_segments(path.segments())
    }

    /// Remove by path segments
    pub fn remove_by_segments(&self, segments: &[PathSegment]) -> ValueResult<(Value, Value)> {
        if segments.is_empty() {
            // Remove root = return (Null, self)
            return Ok((Value::Null, self.clone()));
        }

        self.remove_recursive(segments, 0)
    }

    /// Recursive helper for immutable removal
    fn remove_recursive(
        &self,
        segments: &[PathSegment],
        index: usize,
    ) -> ValueResult<(Value, Value)> {
        let segment = &segments[index];
        let is_last = index == segments.len() - 1;

        match (self, segment) {
            (Value::Object(obj), PathSegment::Key(key)) => {
                if is_last {
                    // Remove this key
                    match obj.remove(key) {
                        Some((new_obj, removed)) => Ok((Value::Object(new_obj), removed)),
                        None => Err(ValueError::key_not_found(key)),
                    }
                } else {
                    // Recurse
                    let child = obj.get(key).ok_or_else(|| ValueError::key_not_found(key))?;
                    let (new_child, removed) = child.remove_recursive(segments, index + 1)?;
                    Ok((Value::Object(obj.insert(key.clone(), new_child)), removed))
                }
            }
            (Value::Array(arr), PathSegment::Index(idx)) => {
                if *idx >= arr.len() {
                    return Err(ValueError::index_out_of_bounds(*idx, arr.len()));
                }

                if is_last {
                    // Remove this index
                    let (new_arr, removed) = arr.remove(*idx)?;
                    Ok((Value::Array(new_arr), removed))
                } else {
                    // Recurse
                    let child = arr
                        .get(*idx)
                        .ok_or_else(|| ValueError::index_out_of_bounds(*idx, arr.len()))?;
                    let (new_child, removed) = child.remove_recursive(segments, index + 1)?;
                    Ok((Value::Array(arr.set(*idx, new_child)?), removed))
                }
            }
            (val, PathSegment::Key(key)) => {
                Err(ValueError::type_mismatch("Object", val.kind().name())
                    .with_context(format!("at key '{}'", key)))
            }
            (val, PathSegment::Index(idx)) => {
                Err(ValueError::type_mismatch("Array", val.kind().name())
                    .with_context(format!("at index [{}]", idx)))
            }
        }
    }

    // ==================== Convenience Methods ====================

    /// Get value from object by key (if this is an object)
    ///
    /// Returns `Some(&Value)` if this is an Object and the key exists.
    /// Returns `None` if this is not an Object or key doesn't exist.
    #[must_use]
    pub fn get_key(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(obj) => obj.get(key),
            _ => None,
        }
    }

    /// Get value from array by index (if this is an array)
    ///
    /// Returns `Some(&Value)` if this is an Array and the index is valid.
    /// Returns `None` if this is not an Array or index is out of bounds.
    #[must_use]
    pub fn get_index(&self, index: usize) -> Option<&Value> {
        match self {
            Value::Array(arr) => arr.get(index),
            _ => None,
        }
    }

    /// Pluck multiple paths from a value, returning a new object
    ///
    /// Useful for extracting specific fields from a complex structure.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let user = Object::from_iter(vec![
    ///     ("name".to_string(), Value::text("Alice")),
    ///     ("email".to_string(), Value::text("alice@example.com")),
    ///     ("password".to_string(), Value::text("secret")),
    /// ]);
    /// let root = Value::Object(user);
    ///
    /// let plucked = root.pluck(&["name", "email"]).unwrap();
    /// assert!(plucked.has_path("name"));
    /// assert!(plucked.has_path("email"));
    /// assert!(!plucked.has_path("password"));
    /// ```
    pub fn pluck(&self, paths: &[&str]) -> ValueResult<Value> {
        let mut result = Value::Object(Object::new());

        for path in paths {
            if let Ok(value) = self.get_path(path) {
                result = result.set_path(path, value)?;
            }
        }

        Ok(result)
    }

    /// Omit paths from a value, returning a new value without those paths
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use nebula_value::collections::Object;
    ///
    /// let user = Object::from_iter(vec![
    ///     ("name".to_string(), Value::text("Alice")),
    ///     ("password".to_string(), Value::text("secret")),
    /// ]);
    /// let root = Value::Object(user);
    ///
    /// let safe = root.omit(&["password"]).unwrap();
    /// assert!(safe.has_path("name"));
    /// assert!(!safe.has_path("password"));
    /// ```
    pub fn omit(&self, paths: &[&str]) -> ValueResult<Value> {
        let mut result = self.clone();

        for path in paths {
            if result.has_path(path) {
                result = result.remove_path(path)?.0;
            }
        }

        Ok(result)
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Format path segments as a string
fn format_path(segments: &[PathSegment]) -> String {
    let mut result = String::new();
    for (i, segment) in segments.iter().enumerate() {
        match segment {
            PathSegment::Key(k) => {
                if i > 0 {
                    result.push('.');
                }
                result.push_str(k);
            }
            PathSegment::Index(idx) => {
                result.push('[');
                result.push_str(&idx.to_string());
                result.push(']');
            }
        }
    }
    result
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== Path Parsing Tests ====================

    #[test]
    fn test_parse_empty() {
        let segments = parse_path("").unwrap();
        assert!(segments.is_empty());
    }

    #[test]
    fn test_parse_simple_key() {
        let segments = parse_path("user").unwrap();
        assert_eq!(segments, vec![PathSegment::Key("user".to_string())]);
    }

    #[test]
    fn test_parse_nested_keys() {
        let segments = parse_path("user.name").unwrap();
        assert_eq!(
            segments,
            vec![
                PathSegment::Key("user".to_string()),
                PathSegment::Key("name".to_string())
            ]
        );
    }

    #[test]
    fn test_parse_array_index() {
        let segments = parse_path("items[0]").unwrap();
        assert_eq!(
            segments,
            vec![PathSegment::Key("items".to_string()), PathSegment::Index(0)]
        );
    }

    #[test]
    fn test_parse_complex_path() {
        let segments = parse_path("data[0].user.addresses[1].city").unwrap();
        assert_eq!(
            segments,
            vec![
                PathSegment::Key("data".to_string()),
                PathSegment::Index(0),
                PathSegment::Key("user".to_string()),
                PathSegment::Key("addresses".to_string()),
                PathSegment::Index(1),
                PathSegment::Key("city".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_multiple_indices() {
        let segments = parse_path("matrix[0][1][2]").unwrap();
        assert_eq!(
            segments,
            vec![
                PathSegment::Key("matrix".to_string()),
                PathSegment::Index(0),
                PathSegment::Index(1),
                PathSegment::Index(2),
            ]
        );
    }

    #[test]
    fn test_parse_root_index() {
        let segments = parse_path("[0]").unwrap();
        assert_eq!(segments, vec![PathSegment::Index(0)]);
    }

    #[test]
    fn test_parse_quoted_key() {
        let segments = parse_path(r#"data["special-key"]"#).unwrap();
        assert_eq!(
            segments,
            vec![
                PathSegment::Key("data".to_string()),
                PathSegment::Key("special-key".to_string()),
            ]
        );
    }

    #[test]
    fn test_parse_invalid_index() {
        let result = parse_path("items[abc]");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_unclosed_bracket() {
        let result = parse_path("items[0");
        assert!(result.is_err());
    }

    // ==================== Path Object Tests ====================

    #[test]
    fn test_path_display() {
        let path = Path::parse("user.addresses[0].city").unwrap();
        assert_eq!(path.to_string(), "user.addresses[0].city");
    }

    #[test]
    fn test_path_push() {
        let path = Path::new().key("user").key("name");
        assert_eq!(path.to_string(), "user.name");

        let path = path.index(0);
        assert_eq!(path.to_string(), "user.name[0]");
    }

    #[test]
    fn test_path_parent() {
        let path = Path::parse("user.addresses[0].city").unwrap();
        let parent = path.parent().unwrap();
        assert_eq!(parent.to_string(), "user.addresses[0]");
    }

    // ==================== Get Path Tests ====================

    #[test]
    fn test_get_path_simple() {
        let obj = Object::from_iter(vec![("name".to_string(), Value::text("Alice"))]);
        let root = Value::Object(obj);

        assert_eq!(root.get_path("name").unwrap(), Value::text("Alice"));
    }

    #[test]
    fn test_get_path_nested() {
        let inner = Object::from_iter(vec![("city".to_string(), Value::text("NYC"))]);
        let outer = Object::from_iter(vec![("address".to_string(), Value::Object(inner))]);
        let root = Value::Object(outer);

        assert_eq!(root.get_path("address.city").unwrap(), Value::text("NYC"));
    }

    #[test]
    fn test_get_path_array() {
        let arr = Array::from_vec(vec![
            Value::text("first"),
            Value::text("second"),
            Value::text("third"),
        ]);
        let obj = Object::from_iter(vec![("items".to_string(), Value::Array(arr))]);
        let root = Value::Object(obj);

        assert_eq!(root.get_path("items[0]").unwrap(), Value::text("first"));
        assert_eq!(root.get_path("items[2]").unwrap(), Value::text("third"));
    }

    #[test]
    fn test_get_path_complex() {
        // Build: { users: [{ name: "Alice", tags: ["admin", "user"] }] }
        let tags = Array::from_vec(vec![Value::text("admin"), Value::text("user")]);
        let user = Object::from_iter(vec![
            ("name".to_string(), Value::text("Alice")),
            ("tags".to_string(), Value::Array(tags)),
        ]);
        let users = Array::from_vec(vec![Value::Object(user)]);
        let root = Value::Object(Object::from_iter(vec![(
            "users".to_string(),
            Value::Array(users),
        )]));

        assert_eq!(
            root.get_path("users[0].name").unwrap(),
            Value::text("Alice")
        );
        assert_eq!(
            root.get_path("users[0].tags[0]").unwrap(),
            Value::text("admin")
        );
        assert_eq!(
            root.get_path("users[0].tags[1]").unwrap(),
            Value::text("user")
        );
    }

    #[test]
    fn test_get_path_not_found() {
        let obj = Object::from_iter(vec![("name".to_string(), Value::text("Alice"))]);
        let root = Value::Object(obj);

        let result = root.get_path("age");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ValueError::PathNotFound { .. }
        ));
    }

    #[test]
    fn test_get_path_index_out_of_bounds() {
        let arr = Array::from_vec(vec![Value::integer(1), Value::integer(2)]);
        let root = Value::Array(arr);

        let result = root.get_path("[5]");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_path_type_mismatch() {
        let root = Value::integer(42);

        let result = root.get_path("name");
        assert!(result.is_err());

        // Error is wrapped in WithContext, check the error message contains type info
        let err = result.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("Object") && err_msg.contains("Integer"),
            "Expected type mismatch error mentioning Object and Integer, got: {}",
            err_msg
        );
    }

    // ==================== Set Path Tests ====================

    #[test]
    fn test_set_path_simple() {
        let root = Value::Object(Object::new());
        let updated = root.set_path("name", Value::text("Alice")).unwrap();

        assert_eq!(updated.get_path("name").unwrap(), Value::text("Alice"));
    }

    #[test]
    fn test_set_path_nested_auto_create() {
        let root = Value::Object(Object::new());
        let updated = root
            .set_path("user.profile.name", Value::text("Alice"))
            .unwrap();

        assert_eq!(
            updated.get_path("user.profile.name").unwrap(),
            Value::text("Alice")
        );
    }

    #[test]
    fn test_set_path_update_existing() {
        let obj = Object::from_iter(vec![("name".to_string(), Value::text("Alice"))]);
        let root = Value::Object(obj);

        let updated = root.set_path("name", Value::text("Bob")).unwrap();

        assert_eq!(updated.get_path("name").unwrap(), Value::text("Bob"));
        // Original unchanged (immutable)
        assert_eq!(root.get_path("name").unwrap(), Value::text("Alice"));
    }

    #[test]
    fn test_set_path_array() {
        let arr = Array::from_vec(vec![Value::integer(1), Value::integer(2)]);
        let root = Value::Array(arr);

        let updated = root.set_path("[0]", Value::integer(99)).unwrap();

        assert_eq!(updated.get_path("[0]").unwrap(), Value::integer(99));
        assert_eq!(updated.get_path("[1]").unwrap(), Value::integer(2));
    }

    #[test]
    fn test_set_path_array_append() {
        let arr = Array::from_vec(vec![Value::integer(1)]);
        let root = Value::Array(arr);

        let updated = root.set_path("[1]", Value::integer(2)).unwrap();

        assert_eq!(updated.get_path("[0]").unwrap(), Value::integer(1));
        assert_eq!(updated.get_path("[1]").unwrap(), Value::integer(2));
    }

    #[test]
    fn test_set_path_from_null() {
        let root = Value::Null;
        let updated = root.set_path("user.name", Value::text("Alice")).unwrap();

        assert_eq!(updated.get_path("user.name").unwrap(), Value::text("Alice"));
    }

    // ==================== Remove Path Tests ====================

    #[test]
    fn test_remove_path_simple() {
        let obj = Object::from_iter(vec![
            ("name".to_string(), Value::text("Alice")),
            ("age".to_string(), Value::integer(30)),
        ]);
        let root = Value::Object(obj);

        let (updated, removed) = root.remove_path("age").unwrap();

        assert_eq!(removed, Value::integer(30));
        assert!(!updated.has_path("age"));
        assert!(updated.has_path("name"));
    }

    #[test]
    fn test_remove_path_nested() {
        let inner = Object::from_iter(vec![
            ("city".to_string(), Value::text("NYC")),
            ("zip".to_string(), Value::text("10001")),
        ]);
        let outer = Object::from_iter(vec![("address".to_string(), Value::Object(inner))]);
        let root = Value::Object(outer);

        let (updated, removed) = root.remove_path("address.city").unwrap();

        assert_eq!(removed, Value::text("NYC"));
        assert!(!updated.has_path("address.city"));
        assert!(updated.has_path("address.zip"));
    }

    #[test]
    fn test_remove_path_array() {
        let arr = Array::from_vec(vec![Value::text("a"), Value::text("b"), Value::text("c")]);
        let root = Value::Array(arr);

        let (updated, removed) = root.remove_path("[1]").unwrap();

        assert_eq!(removed, Value::text("b"));
        assert_eq!(updated.get_path("[0]").unwrap(), Value::text("a"));
        assert_eq!(updated.get_path("[1]").unwrap(), Value::text("c"));
    }

    // ==================== has_path Tests ====================

    #[test]
    fn test_has_path() {
        let obj = Object::from_iter(vec![("name".to_string(), Value::text("Alice"))]);
        let root = Value::Object(obj);

        assert!(root.has_path("name"));
        assert!(!root.has_path("age"));
        assert!(!root.has_path("name.first")); // "Alice" is not an object
    }

    // ==================== Pluck/Omit Tests ====================

    #[test]
    fn test_pluck() {
        let obj = Object::from_iter(vec![
            ("name".to_string(), Value::text("Alice")),
            ("email".to_string(), Value::text("alice@example.com")),
            ("password".to_string(), Value::text("secret")),
        ]);
        let root = Value::Object(obj);

        let plucked = root.pluck(&["name", "email"]).unwrap();

        assert!(plucked.has_path("name"));
        assert!(plucked.has_path("email"));
        assert!(!plucked.has_path("password"));
    }

    #[test]
    fn test_omit() {
        let obj = Object::from_iter(vec![
            ("name".to_string(), Value::text("Alice")),
            ("password".to_string(), Value::text("secret")),
        ]);
        let root = Value::Object(obj);

        let safe = root.omit(&["password"]).unwrap();

        assert!(safe.has_path("name"));
        assert!(!safe.has_path("password"));
    }

    // ==================== Convenience Method Tests ====================

    #[test]
    fn test_get_key() {
        let obj = Object::from_iter(vec![("name".to_string(), Value::text("Alice"))]);
        let root = Value::Object(obj);

        assert_eq!(root.get_key("name"), Some(&Value::text("Alice")));
        assert_eq!(root.get_key("age"), None);

        // Non-object returns None
        let num = Value::integer(42);
        assert_eq!(num.get_key("anything"), None);
    }

    #[test]
    fn test_get_index() {
        let arr = Array::from_vec(vec![Value::integer(1), Value::integer(2)]);
        let root = Value::Array(arr);

        assert_eq!(root.get_index(0), Some(&Value::integer(1)));
        assert_eq!(root.get_index(5), None);

        // Non-array returns None
        let num = Value::integer(42);
        assert_eq!(num.get_index(0), None);
    }

    // ==================== Edge Cases ====================

    #[test]
    fn test_empty_path_returns_self() {
        let root = Value::integer(42);
        assert_eq!(root.get_path("").unwrap(), Value::integer(42));
    }

    #[test]
    fn test_set_root_replaces() {
        let root = Value::integer(42);
        let updated = root.set_path("", Value::text("replaced")).unwrap();
        assert_eq!(updated, Value::text("replaced"));
    }

    #[test]
    fn test_deeply_nested() {
        let root = Value::Object(Object::new());
        let updated = root.set_path("a.b.c.d.e.f.g", Value::integer(42)).unwrap();

        assert_eq!(
            updated.get_path("a.b.c.d.e.f.g").unwrap(),
            Value::integer(42)
        );
    }
}
