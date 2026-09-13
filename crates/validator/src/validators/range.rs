//! Numeric range validators

use std::fmt::Display;

use crate::foundation::{Validate, ValidationError};

/// Invalid configuration supplied to a numeric or collection range validator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RangeConfigError {
    /// A bound is not comparable, such as an IEEE-754 NaN value.
    #[error("range bounds must be comparable")]
    Incomparable,
    /// An inclusive range has a minimum greater than its maximum.
    #[error("inclusive range requires min <= max")]
    MinGreaterThanMax,
    /// An exclusive range has a minimum greater than or equal to its maximum.
    #[error("exclusive range requires min < max")]
    MinNotLessThanMax,
}

crate::validator! {
    /// Validates that a value is at least a minimum.
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub Min<T: PartialOrd + Display + Copy> { min: T } for T;
    rule(self, input) { *input >= self.min }
    error(self, input) {
        ValidationError::new("min", format!("Value must be at least {}", self.min))
            .with_param("min", self.min.to_string())
            .with_param("actual", input.to_string())
    }
    fn min(value: T);
}

crate::validator! {
    /// Validates that a value does not exceed a maximum.
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub Max<T: PartialOrd + Display + Copy> { max: T } for T;
    rule(self, input) { *input <= self.max }
    error(self, input) {
        ValidationError::new("max", format!("Value must be at most {}", self.max))
            .with_param("max", self.max.to_string())
            .with_param("actual", input.to_string())
    }
    fn max(value: T);
}

/// Validates that a value is within an inclusive range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InRange<T> {
    min: T,
    max: T,
}

impl<T: PartialOrd + Display + Copy> Validate<T> for InRange<T> {
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
        if *input >= self.min && *input <= self.max {
            Ok(())
        } else {
            Err(ValidationError::out_of_range(
                "", self.min, self.max, *input,
            ))
        }
    }
}

/// Creates an inclusive range validator after checking its bounds.
///
/// Equal bounds are valid and describe a range containing one value.
///
/// # Errors
///
/// Returns [`RangeConfigError::Incomparable`] for unordered bounds and
/// [`RangeConfigError::MinGreaterThanMax`] when `min > max`.
pub fn in_range<T: PartialOrd + Display + Copy>(
    min: T,
    max: T,
) -> Result<InRange<T>, RangeConfigError> {
    match min.partial_cmp(&max) {
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal) => Ok(InRange { min, max }),
        Some(std::cmp::Ordering::Greater) => Err(RangeConfigError::MinGreaterThanMax),
        None => Err(RangeConfigError::Incomparable),
    }
}

crate::validator! {
    /// Validates that a value is strictly greater than a threshold.
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
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub GreaterThan<T: PartialOrd + Display + Copy> { bound: T } for T;
    rule(self, input) { *input > self.bound }
    error(self, input) {
        ValidationError::new(
            "greater_than",
            format!("Value must be greater than {}", self.bound),
        )
        .with_param("bound", self.bound.to_string())
        .with_param("actual", input.to_string())
    }
    fn greater_than(bound: T);
}

crate::validator! {
    /// Validates that a value is strictly less than a threshold.
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
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub LessThan<T: PartialOrd + Display + Copy> { bound: T } for T;
    rule(self, input) { *input < self.bound }
    error(self, input) {
        ValidationError::new(
            "less_than",
            format!("Value must be less than {}", self.bound),
        )
        .with_param("bound", self.bound.to_string())
        .with_param("actual", input.to_string())
    }
    fn less_than(bound: T);
}

/// Validates that a value is within an exclusive range (`min < value < max`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExclusiveRange<T> {
    min: T,
    max: T,
}

impl<T: PartialOrd + Display + Copy> Validate<T> for ExclusiveRange<T> {
    fn validate(&self, input: &T) -> Result<(), ValidationError> {
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

/// Creates an exclusive range validator after checking its bounds.
///
/// # Errors
///
/// Returns [`RangeConfigError::Incomparable`] for unordered bounds and
/// [`RangeConfigError::MinNotLessThanMax`] when `min >= max`.
pub fn exclusive_range<T: PartialOrd + Display + Copy>(
    min: T,
    max: T,
) -> Result<ExclusiveRange<T>, RangeConfigError> {
    match min.partial_cmp(&max) {
        Some(std::cmp::Ordering::Less) => Ok(ExclusiveRange { min, max }),
        Some(std::cmp::Ordering::Equal | std::cmp::Ordering::Greater) => {
            Err(RangeConfigError::MinNotLessThanMax)
        },
        None => Err(RangeConfigError::Incomparable),
    }
}

// ============================================================================
// CONVENIENCE ALIASES (turbofish-free)
// ============================================================================

/// Creates a [`Min`] validator for `i64` values (no turbofish needed).
///
/// This is a convenience alias for `min::<i64>(value)`, useful when
/// validating JSON numbers which are represented as `i64`.
///
/// # Examples
///
/// ```
/// use nebula_validator::{foundation::Validate, validators::min_i64};
///
/// assert!(min_i64(18).validate(&25_i64).is_ok());
/// assert!(min_i64(18).validate(&10_i64).is_err());
/// ```
#[must_use]
pub fn min_i64(value: i64) -> Min<i64> {
    min(value)
}

/// Creates a [`Max`] validator for `i64` values (no turbofish needed).
///
/// # Examples
///
/// ```
/// use nebula_validator::{foundation::Validate, validators::max_i64};
///
/// assert!(max_i64(100).validate(&50_i64).is_ok());
/// assert!(max_i64(100).validate(&200_i64).is_err());
/// ```
#[must_use]
pub fn max_i64(value: i64) -> Max<i64> {
    max(value)
}

/// Creates an [`InRange`] validator for `i64` values (no turbofish needed).
///
/// # Examples
///
/// ```
/// use nebula_validator::{foundation::Validate, validators::in_range_i64};
///
/// let validator = in_range_i64(1, 100)?;
/// assert!(validator.validate(&50_i64).is_ok());
/// assert!(validator.validate(&0_i64).is_err());
/// # Ok::<(), nebula_validator::validators::RangeConfigError>(())
/// ```
pub fn in_range_i64(min_val: i64, max_val: i64) -> Result<InRange<i64>, RangeConfigError> {
    in_range(min_val, max_val)
}

/// Creates a [`Min`] validator for `f64` values (no turbofish needed).
///
/// # Errors
///
/// Returns [`RangeConfigError::Incomparable`] if `value` is NaN.
///
/// # Examples
///
/// ```
/// use nebula_validator::{foundation::Validate, validators::min_f64};
///
/// let validator = min_f64(0.0)?;
/// assert!(validator.validate(&1.5_f64).is_ok());
/// assert!(validator.validate(&-1.0_f64).is_err());
/// # Ok::<(), nebula_validator::validators::RangeConfigError>(())
/// ```
pub fn min_f64(value: f64) -> Result<Min<f64>, RangeConfigError> {
    if value.is_nan() {
        Err(RangeConfigError::Incomparable)
    } else {
        Ok(min(value))
    }
}

/// Creates a [`Max`] validator for `f64` values (no turbofish needed).
///
/// # Errors
///
/// Returns [`RangeConfigError::Incomparable`] if `value` is NaN.
///
/// # Examples
///
/// ```
/// use nebula_validator::{foundation::Validate, validators::max_f64};
///
/// let validator = max_f64(100.0)?;
/// assert!(validator.validate(&50.5_f64).is_ok());
/// assert!(validator.validate(&200.0_f64).is_err());
/// # Ok::<(), nebula_validator::validators::RangeConfigError>(())
/// ```
pub fn max_f64(value: f64) -> Result<Max<f64>, RangeConfigError> {
    if value.is_nan() {
        Err(RangeConfigError::Incomparable)
    } else {
        Ok(max(value))
    }
}

/// Creates an [`InRange`] validator for `f64` values (no turbofish needed).
///
/// # Errors
///
/// Returns [`RangeConfigError`] if either bound is NaN or the bounds are inverted.
///
/// # Examples
///
/// ```
/// use nebula_validator::{foundation::Validate, validators::in_range_f64};
///
/// let validator = in_range_f64(0.0, 1.0)?;
/// assert!(validator.validate(&0.5_f64).is_ok());
/// assert!(validator.validate(&2.0_f64).is_err());
/// # Ok::<(), nebula_validator::validators::RangeConfigError>(())
/// ```
pub fn in_range_f64(min_val: f64, max_val: f64) -> Result<InRange<f64>, RangeConfigError> {
    in_range(min_val, max_val)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validate;

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
        let validator = in_range(5, 10).expect("ordered bounds");
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
        let validator = exclusive_range(0, 10).expect("strictly ordered bounds");
        assert!(validator.validate(&1).is_ok());
        assert!(validator.validate(&5).is_ok());
        assert!(validator.validate(&9).is_ok());
        assert!(validator.validate(&0).is_err());
        assert!(validator.validate(&10).is_err());
        assert!(validator.validate(&-1).is_err());
        assert!(validator.validate(&11).is_err());
    }

    #[test]
    fn in_range_accepts_equal_bounds() {
        let v = in_range(5, 5).expect("equal inclusive bounds");
        assert!(v.validate(&5).is_ok());
    }

    #[test]
    fn in_range_rejects_inverted_bounds() {
        assert_eq!(
            in_range(10, 1).expect_err("min > max must fail"),
            RangeConfigError::MinGreaterThanMax
        );
    }

    #[test]
    fn exclusive_range_accepts_valid_bounds() {
        let v = exclusive_range(0, 10).expect("strictly ordered bounds");
        assert!(v.validate(&5).is_ok());
    }

    #[test]
    fn exclusive_range_rejects_equal_bounds() {
        assert_eq!(
            exclusive_range(5, 5).expect_err("equal exclusive bounds must fail"),
            RangeConfigError::MinNotLessThanMax
        );
    }

    #[test]
    fn exclusive_range_rejects_inverted_bounds() {
        assert_eq!(
            exclusive_range(10, 1).expect_err("inverted exclusive bounds must fail"),
            RangeConfigError::MinNotLessThanMax
        );
    }

    #[test]
    fn ranges_reject_incomparable_bounds() {
        assert_eq!(
            in_range(f64::NAN, 1.0).expect_err("NaN must fail"),
            RangeConfigError::Incomparable
        );
        assert_eq!(
            exclusive_range(0.0, f64::NAN).expect_err("NaN must fail"),
            RangeConfigError::Incomparable
        );
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
