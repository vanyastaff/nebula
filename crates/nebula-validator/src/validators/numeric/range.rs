//! Numeric range validators

use crate::core::{TypedValidator, ValidationComplexity, ValidationError, ValidatorMetadata};
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

impl<T> TypedValidator for Min<T>
where
    T: PartialOrd + Display + Copy,
{
    type Input = T;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
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

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Min".to_string(),
            description: Some(format!("Value must be >= {}", self.min)),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".to_string(), "range".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
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

impl<T> TypedValidator for Max<T>
where
    T: PartialOrd + Display + Copy,
{
    type Input = T;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
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

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Max".to_string(),
            description: Some(format!("Value must be <= {}", self.max)),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".to_string(), "range".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
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

impl<T> TypedValidator for InRange<T>
where
    T: PartialOrd + Display + Copy,
{
    type Input = T;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if *input >= self.min && *input <= self.max {
            Ok(())
        } else {
            Err(ValidationError::out_of_range(
                "", self.min, self.max, *input,
            ))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "InRange".to_string(),
            description: Some(format!(
                "Value must be between {} and {}",
                self.min, self.max
            )),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".to_string(), "range".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

pub fn in_range<T>(min: T, max: T) -> InRange<T> {
    InRange::new(min, max)
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
}
