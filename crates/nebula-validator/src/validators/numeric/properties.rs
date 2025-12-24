//! Numeric property validators

use crate::core::{ValidationComplexity, ValidationError, Validator, ValidatorMetadata};
use std::fmt::Display;
use std::marker::PhantomData;

// ============================================================================
// POSITIVE
// ============================================================================

/// Validates that a number is positive (> 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Positive<T> {
    _phantom: PhantomData<T>,
}

impl<T> Validator for Positive<T>
where
    T: PartialOrd + Default + Display,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input > T::default() {
            Ok(())
        } else {
            Err(ValidationError::new(
                "positive",
                "Value must be positive (greater than zero)",
            ))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Positive")
            .with_tag("numeric")
            .with_tag("property")
    }
}

/// Creates a validator that checks if a number is positive.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::numeric::positive;
/// use nebula_validator::core::Validator;
///
/// let validator = positive();
/// assert!(validator.validate(&5).is_ok());
/// assert!(validator.validate(&0).is_err());
/// assert!(validator.validate(&-5).is_err());
/// ```
#[must_use]
pub fn positive<T>() -> Positive<T>
where
    T: PartialOrd + Default + Display,
{
    Positive {
        _phantom: PhantomData,
    }
}

// ============================================================================
// NEGATIVE
// ============================================================================

/// Validates that a number is negative (< 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Negative<T> {
    _phantom: PhantomData<T>,
}

impl<T> Validator for Negative<T>
where
    T: PartialOrd + Default + Display,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input < T::default() {
            Ok(())
        } else {
            Err(ValidationError::new(
                "negative",
                "Value must be negative (less than zero)",
            ))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Negative")
            .with_tag("numeric")
            .with_tag("property")
    }
}

/// Creates a validator that checks if a number is negative.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::numeric::negative;
/// use nebula_validator::core::Validator;
///
/// let validator = negative();
/// assert!(validator.validate(&-5).is_ok());
/// assert!(validator.validate(&0).is_err());
/// assert!(validator.validate(&5).is_err());
/// ```
#[must_use]
pub fn negative<T>() -> Negative<T>
where
    T: PartialOrd + Default + Display,
{
    Negative {
        _phantom: PhantomData,
    }
}

// ============================================================================
// EVEN
// ============================================================================

/// Validates that an integer is even.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Even<T> {
    _phantom: PhantomData<T>,
}

impl<T> Validator for Even<T>
where
    T: Copy + std::ops::Rem<Output = T> + PartialEq + From<u8>,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input % T::from(2) == T::from(0) {
            Ok(())
        } else {
            Err(ValidationError::new("even", "Number must be even"))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Even")
            .with_tag("numeric")
            .with_tag("property")
    }
}

/// Creates a validator that checks if a number is even.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::numeric::even;
/// use nebula_validator::core::Validator;
///
/// let validator = even();
/// assert!(validator.validate(&4).is_ok());
/// assert!(validator.validate(&0).is_ok());
/// assert!(validator.validate(&3).is_err());
/// ```
#[must_use]
pub fn even<T>() -> Even<T>
where
    T: Copy + std::ops::Rem<Output = T> + PartialEq + From<u8>,
{
    Even {
        _phantom: PhantomData,
    }
}

// ============================================================================
// ODD
// ============================================================================

/// Validates that an integer is odd.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Odd<T> {
    _phantom: PhantomData<T>,
}

impl<T> Validator for Odd<T>
where
    T: Copy + std::ops::Rem<Output = T> + PartialEq + From<u8>,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input % T::from(2) == T::from(0) {
            Err(ValidationError::new("odd", "Number must be odd"))
        } else {
            Ok(())
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Odd")
            .with_tag("numeric")
            .with_tag("property")
    }
}

/// Creates a validator that checks if a number is odd.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::numeric::odd;
/// use nebula_validator::core::Validator;
///
/// let validator = odd();
/// assert!(validator.validate(&3).is_ok());
/// assert!(validator.validate(&1).is_ok());
/// assert!(validator.validate(&0).is_err());
/// assert!(validator.validate(&4).is_err());
/// ```
#[must_use]
pub fn odd<T>() -> Odd<T>
where
    T: Copy + std::ops::Rem<Output = T> + PartialEq + From<u8>,
{
    Odd {
        _phantom: PhantomData,
    }
}

// ============================================================================
// NON ZERO
// ============================================================================

/// Validates that a number is not zero.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::numeric::non_zero;
/// use nebula_validator::core::Validator;
///
/// let validator = non_zero();
/// assert!(validator.validate(&5).is_ok());
/// assert!(validator.validate(&-3).is_ok());
/// assert!(validator.validate(&0).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NonZero<T> {
    _phantom: PhantomData<T>,
}

impl<T> Validator for NonZero<T>
where
    T: PartialEq + Default + Display,
{
    type Input = T;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input != T::default() {
            Ok(())
        } else {
            Err(ValidationError::new("non_zero", "Value must not be zero"))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "NonZero".to_string(),
            description: Some("Value must not be zero".to_string()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".to_string(), "property".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a validator that checks if a number is not zero.
#[must_use]
pub fn non_zero<T>() -> NonZero<T>
where
    T: PartialEq + Default + Display,
{
    NonZero {
        _phantom: PhantomData,
    }
}

// ============================================================================
// POWER OF TWO
// ============================================================================

/// Validates that a positive integer is a power of two.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::numeric::power_of_two;
/// use nebula_validator::core::Validator;
///
/// let validator = power_of_two();
/// assert!(validator.validate(&1_u32).is_ok());
/// assert!(validator.validate(&2_u32).is_ok());
/// assert!(validator.validate(&4_u32).is_ok());
/// assert!(validator.validate(&8_u32).is_ok());
/// assert!(validator.validate(&3_u32).is_err());
/// assert!(validator.validate(&0_u32).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PowerOfTwo;

impl Validator for PowerOfTwo {
    type Input = u32;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input > 0 && (*input & (*input - 1)) == 0 {
            Ok(())
        } else {
            Err(
                ValidationError::new("power_of_two", "Value must be a power of two")
                    .with_param("actual", input.to_string()),
            )
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "PowerOfTwo".to_string(),
            description: Some("Value must be a power of two".to_string()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".to_string(), "property".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a validator that checks if a u32 is a power of two.
#[must_use]
pub fn power_of_two() -> PowerOfTwo {
    PowerOfTwo
}

/// Validates that a u64 is a power of two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PowerOfTwoU64;

impl Validator for PowerOfTwoU64 {
    type Input = u64;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if *input > 0 && (*input & (*input - 1)) == 0 {
            Ok(())
        } else {
            Err(
                ValidationError::new("power_of_two", "Value must be a power of two")
                    .with_param("actual", input.to_string()),
            )
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "PowerOfTwoU64".to_string(),
            description: Some("Value must be a power of two".to_string()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["numeric".to_string(), "property".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a validator that checks if a u64 is a power of two.
#[must_use]
pub fn power_of_two_u64() -> PowerOfTwoU64 {
    PowerOfTwoU64
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_positive_i32() {
        let validator = positive::<i32>();
        assert!(validator.validate(&5).is_ok());
        assert!(validator.validate(&1).is_ok());
        assert!(validator.validate(&0).is_err());
        assert!(validator.validate(&-5).is_err());
    }

    #[test]
    fn test_positive_f64() {
        let validator = positive::<f64>();
        assert!(validator.validate(&5.5).is_ok());
        assert!(validator.validate(&0.1).is_ok());
        assert!(validator.validate(&0.0).is_err());
        assert!(validator.validate(&-5.5).is_err());
    }

    #[test]
    fn test_negative_i32() {
        let validator = negative::<i32>();
        assert!(validator.validate(&-5).is_ok());
        assert!(validator.validate(&-1).is_ok());
        assert!(validator.validate(&0).is_err());
        assert!(validator.validate(&5).is_err());
    }

    #[test]
    fn test_negative_f64() {
        let validator = negative::<f64>();
        assert!(validator.validate(&-5.5).is_ok());
        assert!(validator.validate(&-0.1).is_ok());
        assert!(validator.validate(&0.0).is_err());
        assert!(validator.validate(&5.5).is_err());
    }

    #[test]
    fn test_even_i32() {
        let validator = even::<i32>();
        assert!(validator.validate(&4).is_ok());
        assert!(validator.validate(&0).is_ok());
        assert!(validator.validate(&-2).is_ok());
        assert!(validator.validate(&3).is_err());
        assert!(validator.validate(&-1).is_err());
    }

    #[test]
    fn test_odd_i32() {
        let validator = odd::<i32>();
        assert!(validator.validate(&3).is_ok());
        assert!(validator.validate(&1).is_ok());
        assert!(validator.validate(&-1).is_ok());
        assert!(validator.validate(&0).is_err());
        assert!(validator.validate(&4).is_err());
    }

    #[test]
    fn test_even_i64() {
        let validator = even::<i64>();
        assert!(validator.validate(&1000).is_ok());
        assert!(validator.validate(&1001).is_err());
    }

    #[test]
    fn test_odd_u8() {
        let validator = odd::<u8>();
        assert!(validator.validate(&1).is_ok());
        assert!(validator.validate(&255).is_ok());
        assert!(validator.validate(&0).is_err());
        assert!(validator.validate(&2).is_err());
    }

    #[test]
    fn test_non_zero_i32() {
        let validator = non_zero::<i32>();
        assert!(validator.validate(&1).is_ok());
        assert!(validator.validate(&-5).is_ok());
        assert!(validator.validate(&100).is_ok());
        assert!(validator.validate(&0).is_err());
    }

    #[test]
    fn test_non_zero_f64() {
        let validator = non_zero::<f64>();
        assert!(validator.validate(&0.1).is_ok());
        assert!(validator.validate(&-0.5).is_ok());
        assert!(validator.validate(&0.0).is_err());
    }

    #[test]
    fn test_power_of_two() {
        let validator = power_of_two();
        assert!(validator.validate(&1).is_ok());
        assert!(validator.validate(&2).is_ok());
        assert!(validator.validate(&4).is_ok());
        assert!(validator.validate(&8).is_ok());
        assert!(validator.validate(&16).is_ok());
        assert!(validator.validate(&1024).is_ok());
        assert!(validator.validate(&0).is_err());
        assert!(validator.validate(&3).is_err());
        assert!(validator.validate(&5).is_err());
        assert!(validator.validate(&6).is_err());
    }

    #[test]
    fn test_power_of_two_u64() {
        let validator = power_of_two_u64();
        assert!(validator.validate(&1).is_ok());
        assert!(validator.validate(&(1u64 << 32)).is_ok());
        assert!(validator.validate(&(1u64 << 63)).is_ok());
        assert!(validator.validate(&0).is_err());
        assert!(validator.validate(&3).is_err());
    }
}
