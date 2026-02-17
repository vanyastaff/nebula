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
