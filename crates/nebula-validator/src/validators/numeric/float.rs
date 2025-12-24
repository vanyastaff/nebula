//! Floating-point validators
//!
//! Validators for special floating-point properties like NaN, infinity, and precision.

use crate::core::{ValidationComplexity, ValidationError, Validator, ValidatorMetadata};

// ============================================================================
// FINITE
// ============================================================================

/// Validates that a floating-point number is finite (not NaN or infinity).
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::numeric::finite;
/// use nebula_validator::core::Validator;
///
/// let validator = finite();
/// assert!(validator.validate(&3.14_f64).is_ok());
/// assert!(validator.validate(&f64::INFINITY).is_err());
/// assert!(validator.validate(&f64::NAN).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Finite;

impl Validator for Finite {
    type Input = f64;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if input.is_finite() {
            Ok(())
        } else if input.is_nan() {
            Err(ValidationError::new("finite", "Value must not be NaN"))
        } else {
            Err(ValidationError::new(
                "finite",
                "Value must be finite (not infinity)",
            ))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Finite".to_string(),
            description: Some("Value must be finite (not NaN or infinity)".to_string()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".to_string(), "float".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a validator that checks if a float is finite.
#[must_use]
pub fn finite() -> Finite {
    Finite
}

// ============================================================================
// FINITE F32
// ============================================================================

/// Validates that an f32 is finite (not NaN or infinity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct FiniteF32;

impl Validator for FiniteF32 {
    type Input = f32;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if input.is_finite() {
            Ok(())
        } else if input.is_nan() {
            Err(ValidationError::new("finite", "Value must not be NaN"))
        } else {
            Err(ValidationError::new(
                "finite",
                "Value must be finite (not infinity)",
            ))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "FiniteF32".to_string(),
            description: Some("Value must be finite (not NaN or infinity)".to_string()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".to_string(), "float".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a validator that checks if an f32 is finite.
#[must_use]
pub fn finite_f32() -> FiniteF32 {
    FiniteF32
}

// ============================================================================
// NOT NAN
// ============================================================================

/// Validates that a floating-point number is not NaN.
///
/// Unlike `Finite`, this allows infinity values.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::numeric::not_nan;
/// use nebula_validator::core::Validator;
///
/// let validator = not_nan();
/// assert!(validator.validate(&3.14_f64).is_ok());
/// assert!(validator.validate(&f64::INFINITY).is_ok());
/// assert!(validator.validate(&f64::NAN).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NotNaN;

impl Validator for NotNaN {
    type Input = f64;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if input.is_nan() {
            Err(ValidationError::new("not_nan", "Value must not be NaN"))
        } else {
            Ok(())
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "NotNaN".to_string(),
            description: Some("Value must not be NaN".to_string()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".to_string(), "float".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a validator that checks if a float is not NaN.
#[must_use]
pub fn not_nan() -> NotNaN {
    NotNaN
}

// ============================================================================
// NOT NAN F32
// ============================================================================

/// Validates that an f32 is not NaN.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct NotNaNF32;

impl Validator for NotNaNF32 {
    type Input = f32;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if input.is_nan() {
            Err(ValidationError::new("not_nan", "Value must not be NaN"))
        } else {
            Ok(())
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "NotNaNF32".to_string(),
            description: Some("Value must not be NaN".to_string()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".to_string(), "float".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a validator that checks if an f32 is not NaN.
#[must_use]
pub fn not_nan_f32() -> NotNaNF32 {
    NotNaNF32
}

// ============================================================================
// DECIMAL PLACES
// ============================================================================

/// Validates that a floating-point number has at most a certain number of decimal places.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::numeric::decimal_places;
/// use nebula_validator::core::Validator;
///
/// let validator = decimal_places(2);
/// assert!(validator.validate(&3.14_f64).is_ok());
/// assert!(validator.validate(&3.1_f64).is_ok());
/// assert!(validator.validate(&3.0_f64).is_ok());
/// assert!(validator.validate(&3.141_f64).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DecimalPlaces {
    /// Maximum number of decimal places allowed.
    pub max_places: u8,
}

impl DecimalPlaces {
    /// Creates a new decimal places validator.
    #[must_use]
    pub fn new(max_places: u8) -> Self {
        Self { max_places }
    }
}

impl Validator for DecimalPlaces {
    type Input = f64;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if !input.is_finite() {
            return Err(ValidationError::new(
                "decimal_places",
                "Cannot check decimal places of non-finite number",
            ));
        }

        // Multiply by 10^max_places and check if it's effectively an integer
        let multiplier = 10_f64.powi(i32::from(self.max_places));
        let scaled = *input * multiplier;
        let rounded = scaled.round();

        // Use epsilon comparison for floating point
        if (scaled - rounded).abs() < 1e-9 {
            Ok(())
        } else {
            Err(ValidationError::new(
                "decimal_places",
                format!(
                    "Value must have at most {} decimal place(s)",
                    self.max_places
                ),
            )
            .with_param("max_places", self.max_places.to_string()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "DecimalPlaces".to_string(),
            description: Some(format!(
                "Value must have at most {} decimal place(s)",
                self.max_places
            )),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec![
                "numeric".to_string(),
                "float".to_string(),
                "precision".to_string(),
            ],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a validator that checks the maximum decimal places.
#[must_use]
pub fn decimal_places(max_places: u8) -> DecimalPlaces {
    DecimalPlaces::new(max_places)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_finite_valid() {
        let validator = finite();
        assert!(validator.validate(&0.0).is_ok());
        assert!(validator.validate(&3.14).is_ok());
        assert!(validator.validate(&-1000.5).is_ok());
        assert!(validator.validate(&f64::MIN).is_ok());
        assert!(validator.validate(&f64::MAX).is_ok());
    }

    #[test]
    fn test_finite_invalid() {
        let validator = finite();
        assert!(validator.validate(&f64::INFINITY).is_err());
        assert!(validator.validate(&f64::NEG_INFINITY).is_err());
        assert!(validator.validate(&f64::NAN).is_err());
    }

    #[test]
    fn test_finite_f32() {
        let validator = finite_f32();
        assert!(validator.validate(&3.14_f32).is_ok());
        assert!(validator.validate(&f32::INFINITY).is_err());
        assert!(validator.validate(&f32::NAN).is_err());
    }

    #[test]
    fn test_not_nan_valid() {
        let validator = not_nan();
        assert!(validator.validate(&0.0).is_ok());
        assert!(validator.validate(&3.14).is_ok());
        assert!(validator.validate(&f64::INFINITY).is_ok());
        assert!(validator.validate(&f64::NEG_INFINITY).is_ok());
    }

    #[test]
    fn test_not_nan_invalid() {
        let validator = not_nan();
        assert!(validator.validate(&f64::NAN).is_err());
    }

    #[test]
    fn test_not_nan_f32() {
        let validator = not_nan_f32();
        assert!(validator.validate(&3.14_f32).is_ok());
        assert!(validator.validate(&f32::NAN).is_err());
    }

    #[test]
    fn test_decimal_places_zero() {
        let validator = decimal_places(0);
        assert!(validator.validate(&3.0).is_ok());
        assert!(validator.validate(&100.0).is_ok());
        assert!(validator.validate(&3.1).is_err());
    }

    #[test]
    fn test_decimal_places_two() {
        let validator = decimal_places(2);
        assert!(validator.validate(&3.0).is_ok());
        assert!(validator.validate(&3.1).is_ok());
        assert!(validator.validate(&3.14).is_ok());
        assert!(validator.validate(&3.141).is_err());
        assert!(validator.validate(&3.1415).is_err());
    }

    #[test]
    fn test_decimal_places_non_finite() {
        let validator = decimal_places(2);
        assert!(validator.validate(&f64::INFINITY).is_err());
        assert!(validator.validate(&f64::NAN).is_err());
    }

    #[test]
    fn test_decimal_places_negative() {
        let validator = decimal_places(2);
        assert!(validator.validate(&-3.14).is_ok());
        assert!(validator.validate(&-3.141).is_err());
    }
}
