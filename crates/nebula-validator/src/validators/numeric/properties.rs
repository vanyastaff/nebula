//! Numeric property validators

use crate::core::{TypedValidator, ValidationError, ValidatorMetadata};
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

impl<T> TypedValidator for Positive<T>
where
    T: PartialOrd + Default + Display,
{
    type Input = T;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
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
/// use nebula_validator::core::TypedValidator;
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

impl<T> TypedValidator for Negative<T>
where
    T: PartialOrd + Default + Display,
{
    type Input = T;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
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
/// use nebula_validator::core::TypedValidator;
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

impl<T> TypedValidator for Even<T>
where
    T: Copy + std::ops::Rem<Output = T> + PartialEq + From<u8>,
{
    type Input = T;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
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
/// use nebula_validator::core::TypedValidator;
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

impl<T> TypedValidator for Odd<T>
where
    T: Copy + std::ops::Rem<Output = T> + PartialEq + From<u8>,
{
    type Input = T;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
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
/// use nebula_validator::core::TypedValidator;
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
}
