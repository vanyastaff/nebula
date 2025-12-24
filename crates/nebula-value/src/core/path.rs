//! Path-based access for Value
//!
//! Supports JSON path-like syntax: $.user.name, $.items[0]

use crate::core::value::Value;
use crate::core::{ValueError, ValueResult};

/// Path segment for navigating values
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    /// Object key access: .key
    Key(String),
    /// Array index access: [index]
    Index(usize),
}

impl Value {
    // ==================== Path-based Access ====================

    /// Get value by path (e.g., "user.name", "items[0]")
    ///
    /// Supported syntax:
    /// - `.key` - object key access
    /// - `[index]` - array index access
    /// - Chained: `user.addresses[0].city`
    pub fn get_path(&self, path: &str) -> ValueResult<&Value> {
        let segments = parse_path(path)?;
        self.get_path_segments(&segments)
    }

    /// Get value by parsed path segments
    ///
    /// Note: Currently returns an error for nested paths in Array/Object.
    /// Path access for collections requires owned Value (use `get_path_owned` when available).
    pub fn get_path_segments(&self, segments: &[PathSegment]) -> ValueResult<&Value> {
        // For now, only support single-level access
        if segments.is_empty() {
            return Ok(self);
        }

        // Path traversal for Array/Object with serde_json::Value storage
        // is complex with borrowed returns. Document limitation.
        match (self, &segments[0]) {
            #[cfg(feature = "serde")]
            (Value::Object(_), PathSegment::Key(_)) | (Value::Array(_), PathSegment::Index(_))
                if segments.len() > 1 =>
            {
                return Err(ValueError::validation(
                    "Multi-level path access not yet supported for Array/Object (requires owned return)",
                ));
            }

            // Single level access for non-collection types would work,
            // but Array/Object need conversion from serde_json::Value
            #[cfg(feature = "serde")]
            (Value::Object(_), PathSegment::Key(_)) | (Value::Array(_), PathSegment::Index(_)) => {
                return Err(ValueError::validation(
                    "Path access for Array/Object requires owned Value (not yet implemented)",
                ));
            }

            #[cfg(not(feature = "serde"))]
            (Value::Object(_), PathSegment::Key(_)) | (Value::Array(_), PathSegment::Index(_)) => {
                return Err(ValueError::validation(
                    "Path access for collections requires 'serde' feature",
                ));
            }

            _ => {}
        }

        // For other types, path access doesn't make sense
        if let Some(segment) = segments.iter().next() {
            return match (self, segment) {
                // Type mismatch errors
                (val, PathSegment::Key(key)) => {
                    Err(
                        ValueError::type_mismatch("Object", val.kind().name()).with_context(
                            format!("Cannot access key '{}' on {}", key, val.kind().name()),
                        ),
                    )
                }

                (val, PathSegment::Index(idx)) => {
                    Err(
                        ValueError::type_mismatch("Array", val.kind().name()).with_context(
                            format!("Cannot access index {} on {}", idx, val.kind().name()),
                        ),
                    )
                }
            };
        }

        Ok(self)
    }

    /// Check if path exists
    pub fn has_path(&self, path: &str) -> bool {
        self.get_path(path).is_ok()
    }

    // ==================== Convenience Methods ====================

    /// Get value from object by key (if this is an object)
    ///
    /// Returns `Some(&Value)` if this is an Object and the key exists.
    /// Returns `None` if this is an Object but the key doesn't exist, or if this is not an Object.
    ///
    /// This is a simplified API - use `as_object()` first if you need to distinguish
    /// between "not an object" and "key not found".
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
    /// Returns `None` if this is an Array but the index is out of bounds, or if this is not an Array.
    ///
    /// This is a simplified API - use `as_array()` first if you need to distinguish
    /// between "not an array" and "index out of bounds".
    #[must_use]
    pub fn get_index(&self, index: usize) -> Option<&Value> {
        match self {
            Value::Array(arr) => arr.get(index),
            _ => None,
        }
    }
}

/// Maximum number of path segments allowed (DoS protection)
const MAX_PATH_SEGMENTS: usize = 100;

/// Parse a path string into segments
///
/// Examples:
/// - "user.name" -> [Key("user"), Key("name")]
/// - "items[0]" -> [Key("items"), Index(0)]
/// - "data[0].value" -> [Key("data"), Index(0), Key("value")]
///
/// # Errors
///
/// Returns `ValueError::LimitExceeded` if the path has more than 100 segments.
fn parse_path(path: &str) -> ValueResult<Vec<PathSegment>> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut chars = path.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '.' => {
                if !current.is_empty() {
                    segments.push(PathSegment::Key(current.clone()));
                    current.clear();

                    // Check segment limit
                    if segments.len() > MAX_PATH_SEGMENTS {
                        return Err(ValueError::limit_exceeded(
                            "path segments",
                            MAX_PATH_SEGMENTS,
                            segments.len(),
                        ));
                    }
                }
            }
            '[' => {
                if !current.is_empty() {
                    segments.push(PathSegment::Key(current.clone()));
                    current.clear();

                    // Check segment limit
                    if segments.len() > MAX_PATH_SEGMENTS {
                        return Err(ValueError::limit_exceeded(
                            "path segments",
                            MAX_PATH_SEGMENTS,
                            segments.len(),
                        ));
                    }
                }

                // Parse index
                let mut index_str = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch == ']' {
                        chars.next(); // consume ']'
                        break;
                    }
                    index_str.push(
                        chars
                            .next()
                            .expect("peek() returned Some, so next() must succeed"),
                    );
                }

                let index = index_str
                    .parse::<usize>()
                    .map_err(|_| ValueError::parse_error("path index", index_str))?;

                segments.push(PathSegment::Index(index));

                // Check segment limit
                if segments.len() > MAX_PATH_SEGMENTS {
                    return Err(ValueError::limit_exceeded(
                        "path segments",
                        MAX_PATH_SEGMENTS,
                        segments.len(),
                    ));
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        segments.push(PathSegment::Key(current));

        // Final check
        if segments.len() > MAX_PATH_SEGMENTS {
            return Err(ValueError::limit_exceeded(
                "path segments",
                MAX_PATH_SEGMENTS,
                segments.len(),
            ));
        }
    }

    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_path_simple() {
        let segments = parse_path("user").unwrap();
        assert_eq!(segments, vec![PathSegment::Key("user".to_string())]);
    }

    #[test]
    fn test_parse_path_nested() {
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
    fn test_parse_path_index() {
        let segments = parse_path("items[0]").unwrap();
        assert_eq!(
            segments,
            vec![PathSegment::Key("items".to_string()), PathSegment::Index(0)]
        );
    }

    #[test]
    fn test_parse_path_complex() {
        let segments = parse_path("data[0].value").unwrap();
        assert_eq!(
            segments,
            vec![
                PathSegment::Key("data".to_string()),
                PathSegment::Index(0),
                PathSegment::Key("value".to_string())
            ]
        );
    }

    #[test]
    fn test_parse_path_multiple_indices() {
        let segments = parse_path("matrix[0][1]").unwrap();
        assert_eq!(
            segments,
            vec![
                PathSegment::Key("matrix".to_string()),
                PathSegment::Index(0),
                PathSegment::Index(1)
            ]
        );
    }

    #[test]
    fn test_get_key_type_mismatch() {
        let val = Value::integer(42);
        let result = val.get_key("foo");
        assert!(result.is_none());
    }

    #[test]
    fn test_get_index_type_mismatch() {
        let val = Value::text("hello");
        let result = val.get_index(0);
        assert!(result.is_none());
    }

    // Note: Full path access tests require proper Value integration with Array/Object
    // These will be added once Array/Object use Value instead of serde_json::Value
}
