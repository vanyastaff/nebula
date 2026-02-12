//! Percentage validators
//!
//! Validators for percentage values in different formats.

use crate::core::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// PERCENTAGE (0.0 - 1.0)
// ============================================================================

/// Validates that a value is a valid percentage in decimal form (0.0 to 1.0).
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::numeric::percentage;
/// use nebula_validator::core::Validate;
///
/// let validator = percentage();
/// assert!(validator.validate(&0.0_f64).is_ok());
/// assert!(validator.validate(&0.5_f64).is_ok());
/// assert!(validator.validate(&1.0_f64).is_ok());
/// assert!(validator.validate(&1.5_f64).is_err());
/// assert!(validator.validate(&-0.1_f64).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Percentage;

impl Validate for Percentage {
    type Input = f64;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if !input.is_finite() {
            return Err(ValidationError::new(
                "percentage",
                "Percentage must be a finite number",
            ));
        }

        if *input >= 0.0 && *input <= 1.0 {
            Ok(())
        } else {
            Err(
                ValidationError::new("percentage", "Percentage must be between 0.0 and 1.0")
                    .with_param("actual", input.to_string()),
            )
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Percentage".into(),
            description: Some("Value must be a percentage (0.0 to 1.0)".into()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".into(), "percentage".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

/// Creates a validator that checks if a value is a valid percentage (0.0 to 1.0).
#[must_use]
pub fn percentage() -> Percentage {
    Percentage
}

// ============================================================================
// PERCENTAGE F32
// ============================================================================

/// Validates that an f32 is a valid percentage (0.0 to 1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PercentageF32;

impl Validate for PercentageF32 {
    type Input = f32;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if !input.is_finite() {
            return Err(ValidationError::new(
                "percentage",
                "Percentage must be a finite number",
            ));
        }

        if *input >= 0.0 && *input <= 1.0 {
            Ok(())
        } else {
            Err(
                ValidationError::new("percentage", "Percentage must be between 0.0 and 1.0")
                    .with_param("actual", input.to_string()),
            )
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "PercentageF32".into(),
            description: Some("Value must be a percentage (0.0 to 1.0)".into()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".into(), "percentage".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

/// Creates a validator that checks if an f32 is a valid percentage (0.0 to 1.0).
#[must_use]
pub fn percentage_f32() -> PercentageF32 {
    PercentageF32
}

// ============================================================================
// PERCENTAGE 100 (0 - 100)
// ============================================================================

/// Validates that a value is a valid percentage (0 to 100).
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::numeric::percentage_100;
/// use nebula_validator::core::Validate;
///
/// let validator = percentage_100();
/// assert!(validator.validate(&0_i32).is_ok());
/// assert!(validator.validate(&50_i32).is_ok());
/// assert!(validator.validate(&100_i32).is_ok());
/// assert!(validator.validate(&101_i32).is_err());
/// assert!(validator.validate(&-1_i32).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Percentage100;

impl Validate for Percentage100 {
    type Input = i32;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input >= 0 && *input <= 100 {
            Ok(())
        } else {
            Err(
                ValidationError::new("percentage_100", "Percentage must be between 0 and 100")
                    .with_param("actual", input.to_string()),
            )
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Percentage100".into(),
            description: Some("Value must be a percentage (0 to 100)".into()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".into(), "percentage".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

/// Creates a validator that checks if an integer is a valid percentage (0 to 100).
#[must_use]
pub fn percentage_100() -> Percentage100 {
    Percentage100
}

/// Validates that an f64 is a valid percentage (0.0 to 100.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Percentage100F64;

impl Validate for Percentage100F64 {
    type Input = f64;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if !input.is_finite() {
            return Err(ValidationError::new(
                "percentage_100",
                "Percentage must be a finite number",
            ));
        }

        if *input >= 0.0 && *input <= 100.0 {
            Ok(())
        } else {
            Err(
                ValidationError::new("percentage_100", "Percentage must be between 0.0 and 100.0")
                    .with_param("actual", input.to_string()),
            )
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Percentage100F64".into(),
            description: Some("Value must be a percentage (0.0 to 100.0)".into()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".into(), "percentage".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

/// Creates a validator that checks if an f64 is a valid percentage (0.0 to 100.0).
#[must_use]
pub fn percentage_100_f64() -> Percentage100F64 {
    Percentage100F64
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentage_valid() {
        let validator = percentage();
        assert!(validator.validate(&0.0).is_ok());
        assert!(validator.validate(&0.5).is_ok());
        assert!(validator.validate(&1.0).is_ok());
        assert!(validator.validate(&0.001).is_ok());
        assert!(validator.validate(&0.999).is_ok());
    }

    #[test]
    fn test_percentage_invalid() {
        let validator = percentage();
        assert!(validator.validate(&-0.1).is_err());
        assert!(validator.validate(&1.1).is_err());
        assert!(validator.validate(&f64::INFINITY).is_err());
        assert!(validator.validate(&f64::NAN).is_err());
    }

    #[test]
    fn test_percentage_f32() {
        let validator = percentage_f32();
        assert!(validator.validate(&0.5_f32).is_ok());
        assert!(validator.validate(&1.5_f32).is_err());
    }

    #[test]
    fn test_percentage_100_valid() {
        let validator = percentage_100();
        assert!(validator.validate(&0).is_ok());
        assert!(validator.validate(&50).is_ok());
        assert!(validator.validate(&100).is_ok());
    }

    #[test]
    fn test_percentage_100_invalid() {
        let validator = percentage_100();
        assert!(validator.validate(&-1).is_err());
        assert!(validator.validate(&101).is_err());
    }

    #[test]
    fn test_percentage_100_f64() {
        let validator = percentage_100_f64();
        assert!(validator.validate(&0.0).is_ok());
        assert!(validator.validate(&50.5).is_ok());
        assert!(validator.validate(&100.0).is_ok());
        assert!(validator.validate(&-0.1).is_err());
        assert!(validator.validate(&100.1).is_err());
    }
}
