//! JSON convenience validators for turbofish-free collection validation.
//!
//! This module provides type aliases and factory functions that specialize
//! generic collection validators for `serde_json::Value`, eliminating
//! the need for turbofish syntax like `min_size::<Value>(2)`.
//!
//! # Examples
//!
//! ```
//! use nebula_validator::json::{json_min_size, json_max_size, json_size_range};
//! use nebula_validator::foundation::Validate;
//! use serde_json::json;
//!
//! // Without this module: min_size::<serde_json::Value>(2)
//! // With this module:
//! let v = json_min_size(2);
//! assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
//! assert!(v.validate_any(&json!([1])).is_err());
//! ```

use crate::validators::size::{ExactSize, MaxSize, MinSize, SizeRange};

/// Type alias for JSON array minimum size validator.
pub type JsonMinSize = MinSize<serde_json::Value>;

/// Type alias for JSON array maximum size validator.
pub type JsonMaxSize = MaxSize<serde_json::Value>;

/// Type alias for JSON array exact size validator.
pub type JsonExactSize = ExactSize<serde_json::Value>;

/// Type alias for JSON array size range validator.
pub type JsonSizeRange = SizeRange<serde_json::Value>;

/// Creates a validator that checks a JSON array has at least `min` elements.
///
/// # Examples
///
/// ```
/// use nebula_validator::json::json_min_size;
/// use nebula_validator::foundation::Validate;
/// use serde_json::json;
///
/// let v = json_min_size(2);
/// assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
/// assert!(v.validate_any(&json!([1])).is_err());
/// ```
#[must_use]
pub fn json_min_size(min: usize) -> JsonMinSize {
    crate::validators::min_size(min)
}

/// Creates a validator that checks a JSON array has at most `max` elements.
///
/// # Examples
///
/// ```
/// use nebula_validator::json::json_max_size;
/// use nebula_validator::foundation::Validate;
/// use serde_json::json;
///
/// let v = json_max_size(3);
/// assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
/// assert!(v.validate_any(&json!([1, 2, 3, 4])).is_err());
/// ```
#[must_use]
pub fn json_max_size(max: usize) -> JsonMaxSize {
    crate::validators::max_size(max)
}

/// Creates a validator that checks a JSON array has exactly `size` elements.
///
/// # Examples
///
/// ```
/// use nebula_validator::json::json_exact_size;
/// use nebula_validator::foundation::Validate;
/// use serde_json::json;
///
/// let v = json_exact_size(3);
/// assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
/// assert!(v.validate_any(&json!([1, 2])).is_err());
/// ```
#[must_use]
pub fn json_exact_size(size: usize) -> JsonExactSize {
    crate::validators::exact_size(size)
}

/// Creates a validator that checks a JSON array size is within the given range.
///
/// # Examples
///
/// ```
/// use nebula_validator::json::json_size_range;
/// use nebula_validator::foundation::Validate;
/// use serde_json::json;
///
/// let v = json_size_range(1, 5);
/// assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
/// assert!(v.validate_any(&json!([])).is_err());
/// assert!(v.validate_any(&json!([1, 2, 3, 4, 5, 6])).is_err());
/// ```
#[must_use]
pub fn json_size_range(min: usize, max: usize) -> JsonSizeRange {
    crate::validators::size_range(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validate;
    use serde_json::json;

    #[test]
    fn json_min_size_valid() {
        let v = json_min_size(2);
        assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
    }

    #[test]
    fn json_min_size_invalid() {
        let v = json_min_size(3);
        assert!(v.validate_any(&json!([1])).is_err());
    }

    #[test]
    fn json_max_size_valid() {
        let v = json_max_size(3);
        assert!(v.validate_any(&json!([1, 2])).is_ok());
    }

    #[test]
    fn json_max_size_invalid() {
        let v = json_max_size(2);
        assert!(v.validate_any(&json!([1, 2, 3])).is_err());
    }

    #[test]
    fn json_exact_size_valid() {
        let v = json_exact_size(3);
        assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
    }

    #[test]
    fn json_exact_size_invalid() {
        let v = json_exact_size(2);
        assert!(v.validate_any(&json!([1, 2, 3])).is_err());
    }

    #[test]
    fn json_size_range_valid() {
        let v = json_size_range(2, 4);
        assert!(v.validate_any(&json!([1, 2, 3])).is_ok());
    }

    #[test]
    fn json_size_range_too_small() {
        let v = json_size_range(2, 4);
        assert!(v.validate_any(&json!([1])).is_err());
    }

    #[test]
    fn json_size_range_too_large() {
        let v = json_size_range(2, 4);
        assert!(v.validate_any(&json!([1, 2, 3, 4, 5])).is_err());
    }

    #[test]
    fn type_mismatch_on_non_array() {
        let v = json_min_size(1);
        let err = v.validate_any(&json!("not an array")).unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
    }

    #[test]
    fn type_mismatch_on_null() {
        let v = json_min_size(1);
        let err = v.validate_any(&json!(null)).unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
    }
}
