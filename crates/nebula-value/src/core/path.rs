//! Path-based access for Value
//!
//! Supports JSON path-like syntax: $.user.name, $.items[0]

use crate::core::NebulaError;
use crate::core::error::{ValueErrorExt, ValueResult};
use crate::core::value::Value;

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
                return Err(NebulaError::validation(
                    "Multi-level path access not yet supported for Array/Object (requires owned return)",
                ));
            }

            // Single level access for non-collection types would work,
            // but Array/Object need conversion from serde_json::Value
            #[cfg(feature = "serde")]
            (Value::Object(_), PathSegment::Key(_)) | (Value::Array(_), PathSegment::Index(_)) => {
                return Err(NebulaError::validation(
                    "Path access for Array/Object requires owned Value (not yet implemented)",
                ));
            }

            #[cfg(not(feature = "serde"))]
            (Value::Object(_), PathSegment::Key(_)) | (Value::Array(_), PathSegment::Index(_)) => {
                return Err(NebulaError::validation(
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
                        NebulaError::value_type_mismatch("Object", val.kind().name()).with_details(
                            format!("Cannot access key '{}' on {}", key, val.kind().name()),
                        ),
                    )
                }

                (val, PathSegment::Index(idx)) => {
                    Err(NebulaError::value_type_mismatch("Array", val.kind().name())
                        .with_details(format!(
                            "Cannot access index {} on {}",
                            idx,
                            val.kind().name()
                        )))
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
    pub fn get_key(&self, key: &str) -> ValueResult<Option<Value>> {
        match self {
            Value::Object(obj) => Ok(obj.get(key).cloned()),
            _ => Err(NebulaError::value_type_mismatch(
                "Object",
                self.kind().name(),
            )),
        }
    }

    /// Get value from array by index (if this is an array)
    pub fn get_index(&self, index: usize) -> ValueResult<Option<Value>> {
        match self {
            Value::Array(arr) => Ok(arr.get(index).cloned()),
            _ => Err(NebulaError::value_type_mismatch(
                "Array",
                self.kind().name(),
            )),
        }
    }
}

/// Parse a path string into segments
///
/// Examples:
/// - "user.name" -> [Key("user"), Key("name")]
/// - "items[0]" -> [Key("items"), Index(0)]
/// - "data[0].value" -> [Key("data"), Index(0), Key("value")]
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
                }
            }
            '[' => {
                if !current.is_empty() {
                    segments.push(PathSegment::Key(current.clone()));
                    current.clear();
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
                    .map_err(|_| NebulaError::value_parse_error("path index", index_str))?;

                segments.push(PathSegment::Index(index));
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if !current.is_empty() {
        segments.push(PathSegment::Key(current));
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
        assert!(result.is_err());
    }

    #[test]
    fn test_get_index_type_mismatch() {
        let val = Value::text("hello");
        let result = val.get_index(0);
        assert!(result.is_err());
    }

    // Note: Full path access tests require proper Value integration with Array/Object
    // These will be added once Array/Object use Value instead of serde_json::Value
}
