//! Collection size validators
//!
//! This module provides validators for checking the size of collections.

use crate::core::{ValidationError, Validator, ValidatorMetadata};
use std::marker::PhantomData;

// ============================================================================
// MIN SIZE
// ============================================================================

/// Validates that a collection has at least a minimum size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MinSize<T> {
    min: usize,
    _phantom: PhantomData<T>,
}

impl<T> Validator for MinSize<T>
where
    T: Clone,
{
    type Input = Vec<T>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
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

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("MinSize")
            .with_tag("collection")
            .with_tag("size")
    }
}

/// Creates a validator that checks if a collection has at least a minimum size.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::collection::min_size;
/// use nebula_validator::core::Validator;
///
/// let validator = min_size::<i32>(3);
/// assert!(validator.validate(&vec![1, 2, 3]).is_ok());
/// assert!(validator.validate(&vec![1, 2]).is_err());
/// ```
#[must_use]
pub fn min_size<T>(min: usize) -> MinSize<T>
where
    T: Clone,
{
    MinSize {
        min,
        _phantom: PhantomData,
    }
}

// ============================================================================
// MAX SIZE
// ============================================================================

/// Validates that a collection has at most a maximum size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaxSize<T> {
    max: usize,
    _phantom: PhantomData<T>,
}

impl<T> Validator for MaxSize<T>
where
    T: Clone,
{
    type Input = Vec<T>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
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

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("MaxSize")
            .with_tag("collection")
            .with_tag("size")
    }
}

/// Creates a validator that checks if a collection has at most a maximum size.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::collection::max_size;
/// use nebula_validator::core::Validator;
///
/// let validator = max_size::<i32>(3);
/// assert!(validator.validate(&vec![1, 2, 3]).is_ok());
/// assert!(validator.validate(&vec![1, 2, 3, 4]).is_err());
/// ```
#[must_use]
pub fn max_size<T>(max: usize) -> MaxSize<T>
where
    T: Clone,
{
    MaxSize {
        max,
        _phantom: PhantomData,
    }
}

// ============================================================================
// EXACT SIZE
// ============================================================================

/// Validates that a collection has an exact size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExactSize<T> {
    size: usize,
    _phantom: PhantomData<T>,
}

impl<T> Validator for ExactSize<T>
where
    T: Clone,
{
    type Input = Vec<T>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
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

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("ExactSize")
            .with_tag("collection")
            .with_tag("size")
    }
}

/// Creates a validator that checks if a collection has an exact size.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::collection::exact_size;
/// use nebula_validator::core::Validator;
///
/// let validator = exact_size::<i32>(3);
/// assert!(validator.validate(&vec![1, 2, 3]).is_ok());
/// assert!(validator.validate(&vec![1, 2]).is_err());
/// assert!(validator.validate(&vec![1, 2, 3, 4]).is_err());
/// ```
#[must_use]
pub fn exact_size<T>(size: usize) -> ExactSize<T>
where
    T: Clone,
{
    ExactSize {
        size,
        _phantom: PhantomData,
    }
}

// ============================================================================
// NOT EMPTY
// ============================================================================

/// Validates that a collection is not empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotEmptyCollection<T> {
    _phantom: PhantomData<T>,
}

impl<T> Validator for NotEmptyCollection<T>
where
    T: Clone,
{
    type Input = Vec<T>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if input.is_empty() {
            Err(ValidationError::new(
                "not_empty",
                "Collection must not be empty",
            ))
        } else {
            Ok(())
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("NotEmptyCollection")
            .with_tag("collection")
            .with_tag("size")
    }
}

/// Creates a validator that checks if a collection is not empty.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::collection::not_empty_collection;
/// use nebula_validator::core::Validator;
///
/// let validator = not_empty_collection::<i32>();
/// assert!(validator.validate(&vec![1]).is_ok());
/// assert!(validator.validate(&vec![]).is_err());
/// ```
#[must_use]
pub fn not_empty_collection<T>() -> NotEmptyCollection<T>
where
    T: Clone,
{
    NotEmptyCollection {
        _phantom: PhantomData,
    }
}

// ============================================================================
// SIZE RANGE
// ============================================================================

/// Validates that a collection size is within a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SizeRange<T> {
    min: usize,
    max: usize,
    _phantom: PhantomData<T>,
}

impl<T> Validator for SizeRange<T>
where
    T: Clone,
{
    type Input = Vec<T>;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
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

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("SizeRange")
            .with_tag("collection")
            .with_tag("size")
    }
}

/// Creates a validator that checks if a collection size is within a range.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::collection::size_range;
/// use nebula_validator::core::Validator;
///
/// let validator = size_range::<i32>(2, 4);
/// assert!(validator.validate(&vec![1, 2]).is_ok());
/// assert!(validator.validate(&vec![1, 2, 3, 4]).is_ok());
/// assert!(validator.validate(&vec![1]).is_err());
/// assert!(validator.validate(&vec![1, 2, 3, 4, 5]).is_err());
/// ```
#[must_use]
pub fn size_range<T>(min: usize, max: usize) -> SizeRange<T>
where
    T: Clone,
{
    SizeRange {
        min,
        max,
        _phantom: PhantomData,
    }
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
        assert!(validator.validate(&vec![1, 2, 3]).is_ok());
        assert!(validator.validate(&vec![1, 2, 3, 4]).is_ok());
        assert!(validator.validate(&vec![1, 2]).is_err());
        assert!(validator.validate(&vec![]).is_err());
    }

    #[test]
    fn test_max_size_vec() {
        let validator = max_size::<i32>(3);
        assert!(validator.validate(&vec![1, 2, 3]).is_ok());
        assert!(validator.validate(&vec![1, 2]).is_ok());
        assert!(validator.validate(&vec![]).is_ok());
        assert!(validator.validate(&vec![1, 2, 3, 4]).is_err());
    }

    #[test]
    fn test_exact_size_vec() {
        let validator = exact_size::<i32>(3);
        assert!(validator.validate(&vec![1, 2, 3]).is_ok());
        assert!(validator.validate(&vec![1, 2]).is_err());
        assert!(validator.validate(&vec![1, 2, 3, 4]).is_err());
    }

    #[test]
    fn test_not_empty_vec() {
        let validator = not_empty_collection::<i32>();
        assert!(validator.validate(&vec![1]).is_ok());
        assert!(validator.validate(&vec![1, 2, 3]).is_ok());
        assert!(validator.validate(&vec![]).is_err());
    }

    #[test]
    fn test_min_size_string() {
        let validator = min_size::<String>(2);
        assert!(
            validator
                .validate(&vec!["a".to_string(), "b".to_string()])
                .is_ok()
        );
        assert!(validator.validate(&vec!["a".to_string()]).is_err());
    }

    #[test]
    fn test_size_range() {
        let validator = size_range::<i32>(2, 4);
        assert!(validator.validate(&vec![1, 2]).is_ok());
        assert!(validator.validate(&vec![1, 2, 3]).is_ok());
        assert!(validator.validate(&vec![1, 2, 3, 4]).is_ok());
        assert!(validator.validate(&vec![1]).is_err());
        assert!(validator.validate(&vec![1, 2, 3, 4, 5]).is_err());
    }
}
