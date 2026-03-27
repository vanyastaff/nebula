//! Numeric range validators

use std::fmt::Display;

use crate::foundation::ValidationError;

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

crate::validator! {
    /// Validates that a value is within an inclusive range.
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub InRange<T: PartialOrd + Display + Copy> { min: T, max: T } for T;
    rule(self, input) { *input >= self.min && *input <= self.max }
    error(self, input) {
        ValidationError::out_of_range("", self.min, self.max, *input)
    }
    fn in_range(min: T, max: T);
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

crate::validator! {
    /// Validates that a value is within an exclusive range (min < value < max).
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
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub ExclusiveRange<T: PartialOrd + Display + Copy> { min: T, max: T } for T;
    rule(self, input) { *input > self.min && *input < self.max }
    error(self, input) {
        ValidationError::new(
            "exclusive_range",
            format!(
                "Value must be between {} and {} (exclusive)",
                self.min, self.max
            ),
        )
        .with_param("min", self.min.to_string())
        .with_param("max", self.max.to_string())
        .with_param("actual", input.to_string())
    }
    fn exclusive_range(min: T, max: T);
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
/// use nebula_validator::validators::min_i64;
/// use nebula_validator::foundation::Validate;
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
/// use nebula_validator::validators::max_i64;
/// use nebula_validator::foundation::Validate;
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
/// use nebula_validator::validators::in_range_i64;
/// use nebula_validator::foundation::Validate;
///
/// assert!(in_range_i64(1, 100).validate(&50_i64).is_ok());
/// assert!(in_range_i64(1, 100).validate(&0_i64).is_err());
/// ```
#[must_use]
pub fn in_range_i64(min_val: i64, max_val: i64) -> InRange<i64> {
    in_range(min_val, max_val)
}

/// Creates a [`Min`] validator for `f64` values (no turbofish needed).
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::min_f64;
/// use nebula_validator::foundation::Validate;
///
/// assert!(min_f64(0.0).validate(&1.5_f64).is_ok());
/// assert!(min_f64(0.0).validate(&-1.0_f64).is_err());
/// ```
#[must_use]
pub fn min_f64(value: f64) -> Min<f64> {
    min(value)
}

/// Creates a [`Max`] validator for `f64` values (no turbofish needed).
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::max_f64;
/// use nebula_validator::foundation::Validate;
///
/// assert!(max_f64(100.0).validate(&50.5_f64).is_ok());
/// assert!(max_f64(100.0).validate(&200.0_f64).is_err());
/// ```
#[must_use]
pub fn max_f64(value: f64) -> Max<f64> {
    max(value)
}

/// Creates an [`InRange`] validator for `f64` values (no turbofish needed).
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::in_range_f64;
/// use nebula_validator::foundation::Validate;
///
/// assert!(in_range_f64(0.0, 1.0).validate(&0.5_f64).is_ok());
/// assert!(in_range_f64(0.0, 1.0).validate(&2.0_f64).is_err());
/// ```
#[must_use]
pub fn in_range_f64(min_val: f64, max_val: f64) -> InRange<f64> {
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
