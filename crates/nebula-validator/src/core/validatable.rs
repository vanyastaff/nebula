//! AsValidatable trait with GAT for universal type conversion
//!
//! This module provides the `AsValidatable` trait that enables validators to accept
//! multiple input types seamlessly through Generic Associated Types (GATs).

use crate::core::ValidationError;
use std::borrow::Borrow;

// ============================================================================
// CORE TRAIT: AsValidatable with GAT
// ============================================================================

/// Trait for types that can be converted for validation.
///
/// Uses GAT to allow returning either borrowed reference or owned value,
/// unified through the `Borrow` trait.
pub trait AsValidatable<T: ?Sized> {
    /// The output type, which must be borrowable as `&T`.
    type Output<'a>: Borrow<T>
    where
        Self: 'a;

    /// Converts self to a validatable form.
    fn as_validatable(&self) -> Result<Self::Output<'_>, ValidationError>;
}

// ============================================================================
// REFLEXIVE IMPLEMENTATIONS
// ============================================================================

impl AsValidatable<str> for str {
    type Output<'a>
        = &'a str
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&str, ValidationError> {
        Ok(self)
    }
}

impl AsValidatable<i64> for i64 {
    type Output<'a> = i64;

    #[inline]
    fn as_validatable(&self) -> Result<i64, ValidationError> {
        Ok(*self)
    }
}

impl AsValidatable<i32> for i32 {
    type Output<'a> = i32;

    #[inline]
    fn as_validatable(&self) -> Result<i32, ValidationError> {
        Ok(*self)
    }
}

impl AsValidatable<f64> for f64 {
    type Output<'a> = f64;

    #[inline]
    fn as_validatable(&self) -> Result<f64, ValidationError> {
        Ok(*self)
    }
}

impl AsValidatable<f32> for f32 {
    type Output<'a> = f32;

    #[inline]
    fn as_validatable(&self) -> Result<f32, ValidationError> {
        Ok(*self)
    }
}

impl AsValidatable<bool> for bool {
    type Output<'a> = bool;

    #[inline]
    fn as_validatable(&self) -> Result<bool, ValidationError> {
        Ok(*self)
    }
}

impl AsValidatable<usize> for usize {
    type Output<'a> = usize;

    #[inline]
    fn as_validatable(&self) -> Result<usize, ValidationError> {
        Ok(*self)
    }
}

// ============================================================================
// COMMON CONVERSIONS
// ============================================================================

impl AsValidatable<str> for String {
    type Output<'a> = &'a str;

    #[inline]
    fn as_validatable(&self) -> Result<&str, ValidationError> {
        Ok(self.as_str())
    }
}

impl AsValidatable<str> for &String {
    type Output<'a>
        = &'a str
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&str, ValidationError> {
        Ok(self.as_str())
    }
}

impl AsValidatable<str> for Box<str> {
    type Output<'a> = &'a str;

    #[inline]
    fn as_validatable(&self) -> Result<&str, ValidationError> {
        Ok(self)
    }
}

impl AsValidatable<str> for std::borrow::Cow<'_, str> {
    type Output<'a>
        = &'a str
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&str, ValidationError> {
        Ok(self.as_ref())
    }
}

impl<T> AsValidatable<[T]> for Vec<T> {
    type Output<'a>
        = &'a [T]
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&[T], ValidationError> {
        Ok(self.as_slice())
    }
}

impl<T> AsValidatable<[T]> for [T] {
    type Output<'a>
        = &'a [T]
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&[T], ValidationError> {
        Ok(self)
    }
}

// Numeric widenings
impl AsValidatable<i64> for i32 {
    type Output<'a> = i64;

    #[inline]
    fn as_validatable(&self) -> Result<i64, ValidationError> {
        Ok(i64::from(*self))
    }
}

impl AsValidatable<i64> for i16 {
    type Output<'a> = i64;

    #[inline]
    fn as_validatable(&self) -> Result<i64, ValidationError> {
        Ok(i64::from(*self))
    }
}

impl AsValidatable<i64> for i8 {
    type Output<'a> = i64;

    #[inline]
    fn as_validatable(&self) -> Result<i64, ValidationError> {
        Ok(i64::from(*self))
    }
}

impl AsValidatable<i64> for u32 {
    type Output<'a> = i64;

    #[inline]
    fn as_validatable(&self) -> Result<i64, ValidationError> {
        Ok(i64::from(*self))
    }
}

impl AsValidatable<i64> for u16 {
    type Output<'a> = i64;

    #[inline]
    fn as_validatable(&self) -> Result<i64, ValidationError> {
        Ok(i64::from(*self))
    }
}

impl AsValidatable<i64> for u8 {
    type Output<'a> = i64;

    #[inline]
    fn as_validatable(&self) -> Result<i64, ValidationError> {
        Ok(i64::from(*self))
    }
}

impl AsValidatable<f64> for f32 {
    type Output<'a> = f64;

    #[inline]
    fn as_validatable(&self) -> Result<f64, ValidationError> {
        Ok(f64::from(*self))
    }
}

impl AsValidatable<f64> for i64 {
    type Output<'a> = f64;

    #[inline]
    fn as_validatable(&self) -> Result<f64, ValidationError> {
        Ok(*self as f64)
    }
}

impl AsValidatable<f64> for i32 {
    type Output<'a> = f64;

    #[inline]
    fn as_validatable(&self) -> Result<f64, ValidationError> {
        Ok(f64::from(*self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_str_identity() {
        let s: &str = "hello";
        let output = s.as_validatable().unwrap();
        assert_eq!(output, "hello");
    }

    #[test]
    fn test_string_to_str() {
        let s = String::from("hello");
        let output = s.as_validatable().unwrap();
        assert_eq!(output, "hello");
    }

    #[test]
    fn test_i64_identity() {
        let n: i64 = 42;
        let output: i64 = AsValidatable::<i64>::as_validatable(&n).unwrap();
        assert_eq!(output, 42);
    }

    #[test]
    fn test_i32_to_i64() {
        let n: i32 = 42;
        let result: i64 = AsValidatable::<i64>::as_validatable(&n).unwrap();
        assert_eq!(result, 42i64);
    }

    #[test]
    fn test_vec_to_slice() {
        let v = vec![1, 2, 3];
        let output = v.as_validatable().unwrap();
        assert_eq!(output, &[1, 2, 3]);
    }

    #[test]
    fn test_cow_str() {
        use std::borrow::Cow;

        let borrowed: Cow<str> = Cow::Borrowed("hello");
        let output = borrowed.as_validatable().unwrap();
        assert_eq!(output, "hello");

        let owned: Cow<str> = Cow::Owned(String::from("world"));
        let output = owned.as_validatable().unwrap();
        assert_eq!(output, "world");
    }

    #[test]
    fn test_validate_any_with_string_validator() {
        use crate::core::{Validate, ValidationError};

        struct MinLength {
            min: usize,
        }

        impl Validate for MinLength {
            type Input = str;

            fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
                if input.len() >= self.min {
                    Ok(())
                } else {
                    Err(ValidationError::new("min_length", "too short"))
                }
            }
        }

        let validator = MinLength { min: 3 };

        // Works with &str
        assert!(validator.validate_any("hello").is_ok());
        assert!(validator.validate_any("hi").is_err());

        // Works with String
        let s = String::from("hello");
        assert!(validator.validate_any(&s).is_ok());

        // Works with Cow<str>
        use std::borrow::Cow;
        let cow: Cow<str> = Cow::Borrowed("hello");
        assert!(validator.validate_any(&cow).is_ok());

        let cow_owned: Cow<str> = Cow::Owned(String::from("world"));
        assert!(validator.validate_any(&cow_owned).is_ok());
    }

    #[test]
    fn test_validate_any_with_numeric_validator() {
        use crate::core::{Validate, ValidationError};

        struct MinValue {
            min: i64,
        }

        impl Validate for MinValue {
            type Input = i64;

            fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
                if *input >= self.min {
                    Ok(())
                } else {
                    Err(ValidationError::new("min_value", "too small"))
                }
            }
        }

        let validator = MinValue { min: 10 };

        // Works with i64
        assert!(validator.validate_any(&42i64).is_ok());
        assert!(validator.validate_any(&5i64).is_err());

        // Works with i32 (widened to i64)
        assert!(validator.validate_any(&42i32).is_ok());
        assert!(validator.validate_any(&5i32).is_err());

        // Works with i16
        assert!(validator.validate_any(&42i16).is_ok());

        // Works with u8
        assert!(validator.validate_any(&42u8).is_ok());
    }
}
