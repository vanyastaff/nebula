//! Collection size validators
//!
//! This module provides validators for checking the size of collections.

use crate::foundation::{Validate, ValidationError};
use std::marker::PhantomData;

/// Validates that a collection has at least a minimum size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MinSize<T> {
    min: usize,
    _phantom: PhantomData<T>,
}

impl<T> Validate<[T]> for MinSize<T> {
    fn validate(&self, input: &[T]) -> Result<(), ValidationError> {
        let size = input.len();
        if size >= self.min {
            Ok(())
        } else {
            Err(ValidationError::new(
                "min_size",
                format!(
                    "Collection must have at least {} elements, got {}",
                    self.min, size
                ),
            ))
        }
    }
}

/// Creates a validator that checks if a collection has at least a minimum size.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::min_size;
/// use nebula_validator::foundation::Validate;
///
/// let validator = min_size::<i32>(3);
/// assert!(validator.validate(&vec![1, 2, 3]).is_ok());
/// assert!(validator.validate(&vec![1, 2]).is_err());
/// ```
#[must_use]
pub fn min_size<T>(min: usize) -> MinSize<T> {
    MinSize {
        min,
        _phantom: PhantomData,
    }
}

/// Validates that a collection has at most a maximum size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaxSize<T> {
    max: usize,
    _phantom: PhantomData<T>,
}

impl<T> Validate<[T]> for MaxSize<T> {
    fn validate(&self, input: &[T]) -> Result<(), ValidationError> {
        let size = input.len();
        if size <= self.max {
            Ok(())
        } else {
            Err(ValidationError::new(
                "max_size",
                format!(
                    "Collection must have at most {} elements, got {}",
                    self.max, size
                ),
            ))
        }
    }
}

/// Creates a validator that checks if a collection has at most a maximum size.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::max_size;
/// use nebula_validator::foundation::Validate;
///
/// let validator = max_size::<i32>(3);
/// assert!(validator.validate(&vec![1, 2, 3]).is_ok());
/// assert!(validator.validate(&vec![1, 2, 3, 4]).is_err());
/// ```
#[must_use]
pub fn max_size<T>(max: usize) -> MaxSize<T> {
    MaxSize {
        max,
        _phantom: PhantomData,
    }
}

/// Validates that a collection has an exact size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExactSize<T> {
    size: usize,
    _phantom: PhantomData<T>,
}

impl<T> Validate<[T]> for ExactSize<T> {
    fn validate(&self, input: &[T]) -> Result<(), ValidationError> {
        let actual_size = input.len();
        if actual_size == self.size {
            Ok(())
        } else {
            Err(ValidationError::new(
                "exact_size",
                format!(
                    "Collection must have exactly {} elements, got {}",
                    self.size, actual_size
                ),
            ))
        }
    }
}

/// Creates a validator that checks if a collection has an exact size.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::exact_size;
/// use nebula_validator::foundation::Validate;
///
/// let validator = exact_size::<i32>(3);
/// assert!(validator.validate(&vec![1, 2, 3]).is_ok());
/// assert!(validator.validate(&vec![1, 2]).is_err());
/// assert!(validator.validate(&vec![1, 2, 3, 4]).is_err());
/// ```
#[must_use]
pub fn exact_size<T>(size: usize) -> ExactSize<T> {
    ExactSize {
        size,
        _phantom: PhantomData,
    }
}

/// Validates that a collection is not empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotEmptyCollection<T> {
    _phantom: PhantomData<T>,
}

impl<T> Validate<[T]> for NotEmptyCollection<T> {
    fn validate(&self, input: &[T]) -> Result<(), ValidationError> {
        if input.is_empty() {
            Err(ValidationError::new(
                "not_empty_collection",
                "Collection must not be empty",
            ))
        } else {
            Ok(())
        }
    }
}

/// Creates a validator that checks if a collection is not empty.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::not_empty_collection;
/// use nebula_validator::foundation::Validate;
///
/// let validator = not_empty_collection::<i32>();
/// assert!(validator.validate(&vec![1]).is_ok());
/// assert!(validator.validate(&vec![]).is_err());
/// ```
#[must_use]
pub fn not_empty_collection<T>() -> NotEmptyCollection<T> {
    NotEmptyCollection {
        _phantom: PhantomData,
    }
}

/// Validates that a collection size is within a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SizeRange<T> {
    min: usize,
    max: usize,
    _phantom: PhantomData<T>,
}

impl<T> Validate<[T]> for SizeRange<T> {
    fn validate(&self, input: &[T]) -> Result<(), ValidationError> {
        let size = input.len();
        if size >= self.min && size <= self.max {
            Ok(())
        } else {
            Err(ValidationError::new(
                "size_range",
                format!(
                    "Collection must have between {} and {} elements, got {}",
                    self.min, self.max, size
                ),
            )
            .with_param("min", self.min.to_string())
            .with_param("max", self.max.to_string())
            .with_param("actual", size.to_string()))
        }
    }
}

/// Creates a validator that checks if a collection size is within a range.
///
/// # Panics
///
/// Panics if `min > max`.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::size_range;
/// use nebula_validator::foundation::Validate;
///
/// let validator = size_range::<i32>(2, 4);
/// assert!(validator.validate(&vec![1, 2]).is_ok());
/// assert!(validator.validate(&vec![1, 2, 3, 4]).is_ok());
/// assert!(validator.validate(&vec![1]).is_err());
/// assert!(validator.validate(&vec![1, 2, 3, 4, 5]).is_err());
/// ```
#[must_use]
pub fn size_range<T>(min: usize, max: usize) -> SizeRange<T> {
    assert!(min <= max, "size_range: min ({min}) must be <= max ({max})");
    SizeRange {
        min,
        max,
        _phantom: PhantomData,
    }
}

/// Creates a validator that checks if a collection size is within a range.
///
/// Returns an error when `min > max`.
pub fn try_size_range<T>(min: usize, max: usize) -> Result<SizeRange<T>, ValidationError> {
    if min > max {
        return Err(ValidationError::new(
            "invalid_range",
            format!("size_range requires min <= max (got {min} > {max})"),
        )
        .with_param("min", min.to_string())
        .with_param("max", max.to_string()));
    }

    Ok(SizeRange {
        min,
        max,
        _phantom: PhantomData,
    })
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_min_size_vec() {
        let validator = min_size::<i32>(3);
        assert!(validator.validate(&[1, 2, 3]).is_ok());
        assert!(validator.validate(&[1, 2, 3, 4]).is_ok());
        assert!(validator.validate(&[1, 2]).is_err());
        assert!(validator.validate(&[]).is_err());
    }

    #[test]
    fn test_max_size_vec() {
        let validator = max_size::<i32>(3);
        assert!(validator.validate(&[1, 2, 3]).is_ok());
        assert!(validator.validate(&[1, 2]).is_ok());
        assert!(validator.validate(&[]).is_ok());
        assert!(validator.validate(&[1, 2, 3, 4]).is_err());
    }

    #[test]
    fn test_exact_size_vec() {
        let validator = exact_size::<i32>(3);
        assert!(validator.validate(&[1, 2, 3]).is_ok());
        assert!(validator.validate(&[1, 2]).is_err());
        assert!(validator.validate(&[1, 2, 3, 4]).is_err());
    }

    #[test]
    fn test_not_empty_vec() {
        let validator = not_empty_collection::<i32>();
        assert!(validator.validate(&[1]).is_ok());
        assert!(validator.validate(&[1, 2, 3]).is_ok());
        assert!(validator.validate(&[]).is_err());
    }

    #[test]
    fn test_min_size_string() {
        let validator = min_size::<String>(2);
        assert!(
            validator
                .validate(&["a".to_string(), "b".to_string()])
                .is_ok()
        );
        assert!(validator.validate(&["a".to_string()]).is_err());
    }

    #[test]
    fn test_size_range() {
        let validator = size_range::<i32>(2, 4);
        assert!(validator.validate(&[1, 2]).is_ok());
        assert!(validator.validate(&[1, 2, 3]).is_ok());
        assert!(validator.validate(&[1, 2, 3, 4]).is_ok());
        assert!(validator.validate(&[1]).is_err());
        assert!(validator.validate(&[1, 2, 3, 4, 5]).is_err());
    }

    #[test]
    #[should_panic(expected = "min (10) must be <= max (2)")]
    fn test_size_range_inverted_bounds_panics() {
        let _ = size_range::<i32>(10, 2);
    }

    #[test]
    fn test_not_empty_collection_error_code() {
        let validator = not_empty_collection::<i32>();
        let err = validator.validate(&[]).unwrap_err();
        assert_eq!(err.code.as_ref(), "not_empty_collection");
    }

    #[test]
    fn test_try_size_range_accepts_valid_bounds() {
        let validator = try_size_range::<i32>(1, 3).expect("valid bounds");
        assert!(validator.validate(&[1, 2]).is_ok());
    }

    #[test]
    fn test_try_size_range_rejects_inverted_bounds() {
        let err = try_size_range::<i32>(10, 2).expect_err("min > max must fail");
        assert_eq!(err.code.as_ref(), "invalid_range");
        assert_eq!(err.param("min"), Some("10"));
        assert_eq!(err.param("max"), Some("2"));
    }
}
