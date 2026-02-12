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

// ============================================================================
// SERDE JSON VALUE CONVERSIONS
// ============================================================================

/// Returns a human-readable type name for a JSON value.
#[cfg(feature = "serde")]
pub(crate) fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

#[cfg(feature = "serde")]
impl AsValidatable<str> for serde_json::Value {
    type Output<'a>
        = &'a str
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&str, ValidationError> {
        match self {
            serde_json::Value::String(s) => Ok(s.as_str()),
            other => Err(ValidationError::new(
                "type_mismatch",
                format!("Expected string, got {}", json_type_name(other)),
            )
            .with_param("expected", "string")
            .with_param("actual", json_type_name(other))),
        }
    }
}

#[cfg(feature = "serde")]
impl AsValidatable<i64> for serde_json::Value {
    type Output<'a> = i64;

    #[inline]
    fn as_validatable(&self) -> Result<i64, ValidationError> {
        match self {
            serde_json::Value::Number(n) => n.as_i64().ok_or_else(|| {
                ValidationError::new("type_mismatch", format!("Expected integer, got {n}"))
                    .with_param("expected", "integer")
                    .with_param("actual", "number")
            }),
            other => Err(ValidationError::new(
                "type_mismatch",
                format!("Expected integer, got {}", json_type_name(other)),
            )
            .with_param("expected", "integer")
            .with_param("actual", json_type_name(other))),
        }
    }
}

#[cfg(feature = "serde")]
impl AsValidatable<f64> for serde_json::Value {
    type Output<'a> = f64;

    #[inline]
    fn as_validatable(&self) -> Result<f64, ValidationError> {
        match self {
            serde_json::Value::Number(n) => n.as_f64().ok_or_else(|| {
                ValidationError::new("type_mismatch", format!("Expected number, got {n}"))
                    .with_param("expected", "number")
                    .with_param("actual", "number")
            }),
            other => Err(ValidationError::new(
                "type_mismatch",
                format!("Expected number, got {}", json_type_name(other)),
            )
            .with_param("expected", "number")
            .with_param("actual", json_type_name(other))),
        }
    }
}

#[cfg(feature = "serde")]
impl AsValidatable<bool> for serde_json::Value {
    type Output<'a> = bool;

    #[inline]
    fn as_validatable(&self) -> Result<bool, ValidationError> {
        match self {
            serde_json::Value::Bool(b) => Ok(*b),
            other => Err(ValidationError::new(
                "type_mismatch",
                format!("Expected boolean, got {}", json_type_name(other)),
            )
            .with_param("expected", "boolean")
            .with_param("actual", json_type_name(other))),
        }
    }
}

#[cfg(feature = "serde")]
impl AsValidatable<[serde_json::Value]> for serde_json::Value {
    type Output<'a>
        = &'a [serde_json::Value]
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&[serde_json::Value], ValidationError> {
        match self {
            serde_json::Value::Array(arr) => Ok(arr.as_slice()),
            other => Err(ValidationError::new(
                "type_mismatch",
                format!("Expected array, got {}", json_type_name(other)),
            )
            .with_param("expected", "array")
            .with_param("actual", json_type_name(other))),
        }
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

#[cfg(test)]
#[cfg(feature = "serde")]
mod serde_json_tests {
    use super::*;
    use crate::core::Validate;
    use serde_json::json;

    // -- str --

    #[test]
    fn value_string_as_str() {
        let value = json!("hello");
        let result = AsValidatable::<str>::as_validatable(&value).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn value_number_as_str_fails() {
        let err = AsValidatable::<str>::as_validatable(&json!(42)).unwrap_err();
        assert_eq!(err.code.as_ref(), "type_mismatch");
        assert_eq!(err.param("expected"), Some("string"));
        assert_eq!(err.param("actual"), Some("number"));
    }

    #[test]
    fn value_null_as_str_fails() {
        let err = AsValidatable::<str>::as_validatable(&json!(null)).unwrap_err();
        assert_eq!(err.param("actual"), Some("null"));
    }

    // -- i64 --

    #[test]
    fn value_integer_as_i64() {
        let result = AsValidatable::<i64>::as_validatable(&json!(42)).unwrap();
        assert_eq!(*result.borrow(), 42i64);
    }

    #[test]
    fn value_float_as_i64_fails() {
        assert!(AsValidatable::<i64>::as_validatable(&json!(3.14)).is_err());
    }

    #[test]
    fn value_string_as_i64_fails() {
        assert!(AsValidatable::<i64>::as_validatable(&json!("42")).is_err());
    }

    // -- f64 --

    #[test]
    fn value_float_as_f64() {
        let result = AsValidatable::<f64>::as_validatable(&json!(3.14)).unwrap();
        assert!((result.borrow() - 3.14).abs() < f64::EPSILON);
    }

    #[test]
    fn value_integer_widens_to_f64() {
        let result = AsValidatable::<f64>::as_validatable(&json!(42)).unwrap();
        assert_eq!(*result.borrow(), 42.0);
    }

    #[test]
    fn value_string_as_f64_fails() {
        assert!(AsValidatable::<f64>::as_validatable(&json!("3.14")).is_err());
    }

    // -- bool --

    #[test]
    fn value_bool_as_bool() {
        assert!(AsValidatable::<bool>::as_validatable(&json!(true)).unwrap());
        assert!(!AsValidatable::<bool>::as_validatable(&json!(false)).unwrap());
    }

    #[test]
    fn value_string_as_bool_fails() {
        assert!(AsValidatable::<bool>::as_validatable(&json!("true")).is_err());
    }

    // -- [Value] --

    #[test]
    fn value_array_as_slice() {
        let value = json!([1, 2, 3]);
        let result = AsValidatable::<[serde_json::Value]>::as_validatable(&value).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn value_string_as_slice_fails() {
        assert!(AsValidatable::<[serde_json::Value]>::as_validatable(&json!("not array")).is_err());
    }

    // -- validate_any integration --

    #[test]
    fn validate_any_str_validator_with_json_value() {
        use crate::validators::string::min_length;

        let validator = min_length(3);
        assert!(validator.validate_any(&json!("hello")).is_ok());
        assert!(validator.validate_any(&json!("hi")).is_err());
        assert!(validator.validate_any(&json!(42)).is_err()); // type mismatch
    }
}
