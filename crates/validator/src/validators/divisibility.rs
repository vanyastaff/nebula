//! Divisibility validators

use crate::foundation::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};
use std::fmt::Display;
use std::ops::Rem;

// ============================================================================
// DIVISIBLE BY
// ============================================================================

/// Validates that a number is divisible by a given divisor.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::divisible_by;
/// use nebula_validator::foundation::Validate;
///
/// let validator = divisible_by(3);
/// assert!(validator.validate(&9).is_ok());
/// assert!(validator.validate(&12).is_ok());
/// assert!(validator.validate(&7).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DivisibleBy<T> {
    /// The divisor.
    pub divisor: T,
}

impl<T> DivisibleBy<T> {
    /// Creates a new divisibility validator.
    #[must_use]
    pub fn new(divisor: T) -> Self {
        Self { divisor }
    }
}

impl<T> Validate for DivisibleBy<T>
where
    T: Copy + Rem<Output = T> + PartialEq + Default + Display,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input % self.divisor == T::default() {
            Ok(())
        } else {
            Err(ValidationError::new(
                "divisible_by",
                format!("Value must be divisible by {}", self.divisor),
            )
            .with_param("divisor", self.divisor.to_string())
            .with_param("actual", input.to_string()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "DivisibleBy".into(),
            description: Some(format!("Value must be divisible by {}", self.divisor).into()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".into(), "divisibility".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

/// Creates a validator that checks if a number is divisible by the given divisor.
#[must_use]
pub fn divisible_by<T>(divisor: T) -> DivisibleBy<T>
where
    T: Copy + Rem<Output = T> + PartialEq + Default + Display,
{
    DivisibleBy::new(divisor)
}

/// Alias for `divisible_by` - validates that a number is a multiple of the given value.
#[must_use]
pub fn multiple_of<T>(value: T) -> DivisibleBy<T>
where
    T: Copy + Rem<Output = T> + PartialEq + Default + Display,
{
    DivisibleBy::new(value)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_divisible_by_i32() {
        let validator = divisible_by(3);
        assert!(validator.validate(&0).is_ok());
        assert!(validator.validate(&3).is_ok());
        assert!(validator.validate(&9).is_ok());
        assert!(validator.validate(&-6).is_ok());
        assert!(validator.validate(&1).is_err());
        assert!(validator.validate(&7).is_err());
    }

    #[test]
    fn test_divisible_by_i64() {
        let validator = divisible_by::<i64>(5);
        assert!(validator.validate(&10).is_ok());
        assert!(validator.validate(&25).is_ok());
        assert!(validator.validate(&7).is_err());
    }

    #[test]
    fn test_multiple_of_alias() {
        let validator = multiple_of(4);
        assert!(validator.validate(&8).is_ok());
        assert!(validator.validate(&16).is_ok());
        assert!(validator.validate(&5).is_err());
    }

    #[test]
    fn test_divisible_by_metadata() {
        let validator = divisible_by(7);
        let meta = validator.metadata();
        assert_eq!(meta.name, "DivisibleBy");
        assert!(meta.description.unwrap().contains("7"));
    }

    #[test]
    fn test_error_params() {
        let validator = divisible_by(3);
        let err = validator.validate(&7).unwrap_err();
        assert_eq!(err.param("divisor"), Some("3"));
        assert_eq!(err.param("actual"), Some("7"));
    }
}
