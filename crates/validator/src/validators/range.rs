//! Numeric range validators

use crate::foundation::{Validate, ValidationError};
use std::fmt::Display;

// ============================================================================
// MIN VALUE
// ============================================================================

/// Validates that a number is at least a minimum value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Min<T> {
    /// The minimum allowed value.
    pub min: T,
}

impl<T> Min<T> {
    pub fn new(min: T) -> Self {
        Self { min }
    }
}

impl<T> Validate for Min<T>
where
    T: PartialOrd + Display + Copy,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input >= self.min {
            Ok(())
        } else {
            Err(
                ValidationError::new("min", format!("Value must be at least {}", self.min))
                    .with_param("min", self.min.to_string())
                    .with_param("actual", input.to_string()),
            )
        }
    }
}

pub fn min<T>(value: T) -> Min<T> {
    Min::new(value)
}

// ============================================================================
// MAX VALUE
// ============================================================================

/// Validates that a number does not exceed a maximum value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Max<T> {
    /// The maximum allowed value.
    pub max: T,
}

impl<T> Max<T> {
    pub fn new(max: T) -> Self {
        Self { max }
    }
}

impl<T> Validate for Max<T>
where
    T: PartialOrd + Display + Copy,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input <= self.max {
            Ok(())
        } else {
            Err(
                ValidationError::new("max", format!("Value must be at most {}", self.max))
                    .with_param("max", self.max.to_string())
                    .with_param("actual", input.to_string()),
            )
        }
    }
}

pub fn max<T>(value: T) -> Max<T> {
    Max::new(value)
}

// ============================================================================
// RANGE
// ============================================================================

/// Validates that a number is within a range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InRange<T> {
    /// The minimum bound of the range (inclusive).
    pub min: T,
    /// The maximum bound of the range (inclusive).
    pub max: T,
}

impl<T> InRange<T> {
    pub fn new(min: T, max: T) -> Self {
        Self { min, max }
    }
}

impl<T> Validate for InRange<T>
where
    T: PartialOrd + Display + Copy,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input >= self.min && *input <= self.max {
            Ok(())
        } else {
            Err(ValidationError::out_of_range(
                "", self.min, self.max, *input,
            ))
        }
    }
}

pub fn in_range<T>(min: T, max: T) -> InRange<T> {
    InRange::new(min, max)
}

// ============================================================================
// GREATER THAN (exclusive)
// ============================================================================

/// Validates that a number is strictly greater than a given value.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::greater_than;
/// use nebula_validator::foundation::Validate;
///
/// let validator = greater_than(5);
/// assert!(validator.validate(&6).is_ok());
/// assert!(validator.validate(&5).is_err()); // Not strictly greater
/// assert!(validator.validate(&4).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GreaterThan<T> {
    /// The exclusive lower bound.
    pub bound: T,
}

impl<T> GreaterThan<T> {
    /// Creates a new greater-than validator.
    #[must_use]
    pub fn new(bound: T) -> Self {
        Self { bound }
    }
}

impl<T> Validate for GreaterThan<T>
where
    T: PartialOrd + Display + Copy,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input > self.bound {
            Ok(())
        } else {
            Err(ValidationError::new(
                "greater_than",
                format!("Value must be greater than {}", self.bound),
            )
            .with_param("bound", self.bound.to_string())
            .with_param("actual", input.to_string()))
        }
    }
}

/// Creates a validator that checks if a number is strictly greater than the given value.
#[must_use]
pub fn greater_than<T>(bound: T) -> GreaterThan<T>
where
    T: PartialOrd + Display + Copy,
{
    GreaterThan::new(bound)
}

// ============================================================================
// LESS THAN (exclusive)
// ============================================================================

/// Validates that a number is strictly less than a given value.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::less_than;
/// use nebula_validator::foundation::Validate;
///
/// let validator = less_than(10);
/// assert!(validator.validate(&9).is_ok());
/// assert!(validator.validate(&10).is_err()); // Not strictly less
/// assert!(validator.validate(&11).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LessThan<T> {
    /// The exclusive upper bound.
    pub bound: T,
}

impl<T> LessThan<T> {
    /// Creates a new less-than validator.
    #[must_use]
    pub fn new(bound: T) -> Self {
        Self { bound }
    }
}

impl<T> Validate for LessThan<T>
where
    T: PartialOrd + Display + Copy,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input < self.bound {
            Ok(())
        } else {
            Err(ValidationError::new(
                "less_than",
                format!("Value must be less than {}", self.bound),
            )
            .with_param("bound", self.bound.to_string())
            .with_param("actual", input.to_string()))
        }
    }
}

/// Creates a validator that checks if a number is strictly less than the given value.
#[must_use]
pub fn less_than<T>(bound: T) -> LessThan<T>
where
    T: PartialOrd + Display + Copy,
{
    LessThan::new(bound)
}

// ============================================================================
// EXCLUSIVE RANGE
// ============================================================================

/// Validates that a number is within an exclusive range (min < value < max).
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::exclusive_range;
/// use nebula_validator::foundation::Validate;
///
/// let validator = exclusive_range(0, 10);
/// assert!(validator.validate(&5).is_ok());
/// assert!(validator.validate(&0).is_err()); // Boundary not included
/// assert!(validator.validate(&10).is_err()); // Boundary not included
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExclusiveRange<T> {
    /// The exclusive minimum bound.
    pub min: T,
    /// The exclusive maximum bound.
    pub max: T,
}

impl<T> ExclusiveRange<T> {
    /// Creates a new exclusive range validator.
    #[must_use]
    pub fn new(min: T, max: T) -> Self {
        Self { min, max }
    }
}

impl<T> Validate for ExclusiveRange<T>
where
    T: PartialOrd + Display + Copy,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input > self.min && *input < self.max {
            Ok(())
        } else {
            Err(ValidationError::new(
                "exclusive_range",
                format!(
                    "Value must be between {} and {} (exclusive)",
                    self.min, self.max
                ),
            )
            .with_param("min", self.min.to_string())
            .with_param("max", self.max.to_string())
            .with_param("actual", input.to_string()))
        }
    }
}

/// Creates a validator that checks if a number is within an exclusive range.
#[must_use]
pub fn exclusive_range<T>(min: T, max: T) -> ExclusiveRange<T>
where
    T: PartialOrd + Display + Copy,
{
    ExclusiveRange::new(min, max)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_min() {
        let validator = min(5);
        assert!(validator.validate(&5).is_ok());
        assert!(validator.validate(&10).is_ok());
        assert!(validator.validate(&3).is_err());
    }

    #[test]
    fn test_max() {
        let validator = max(10);
        assert!(validator.validate(&5).is_ok());
        assert!(validator.validate(&10).is_ok());
        assert!(validator.validate(&15).is_err());
    }

    #[test]
    fn test_in_range() {
        let validator = in_range(5, 10);
        assert!(validator.validate(&5).is_ok());
        assert!(validator.validate(&7).is_ok());
        assert!(validator.validate(&10).is_ok());
        assert!(validator.validate(&3).is_err());
        assert!(validator.validate(&12).is_err());
    }

    #[test]
    fn test_greater_than() {
        let validator = greater_than(5);
        assert!(validator.validate(&6).is_ok());
        assert!(validator.validate(&100).is_ok());
        assert!(validator.validate(&5).is_err());
        assert!(validator.validate(&4).is_err());
    }

    #[test]
    fn test_less_than() {
        let validator = less_than(10);
        assert!(validator.validate(&9).is_ok());
        assert!(validator.validate(&0).is_ok());
        assert!(validator.validate(&10).is_err());
        assert!(validator.validate(&11).is_err());
    }

    #[test]
    fn test_exclusive_range() {
        let validator = exclusive_range(0, 10);
        assert!(validator.validate(&1).is_ok());
        assert!(validator.validate(&5).is_ok());
        assert!(validator.validate(&9).is_ok());
        assert!(validator.validate(&0).is_err());
        assert!(validator.validate(&10).is_err());
        assert!(validator.validate(&-1).is_err());
        assert!(validator.validate(&11).is_err());
    }

    #[test]
    fn test_greater_than_float() {
        let validator = greater_than(0.0_f64);
        assert!(validator.validate(&0.001).is_ok());
        assert!(validator.validate(&0.0).is_err());
        assert!(validator.validate(&-0.001).is_err());
    }

    #[test]
    fn test_less_than_float() {
        let validator = less_than(1.0_f64);
        assert!(validator.validate(&0.999).is_ok());
        assert!(validator.validate(&1.0).is_err());
        assert!(validator.validate(&1.001).is_err());
    }
}
