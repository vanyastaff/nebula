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
// NEBULA-VALUE IMPLEMENTATIONS
// ============================================================================

impl AsValidatable<str> for nebula_value::Value {
    type Output<'a>
        = &'a str
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&str, ValidationError> {
        self.as_str().ok_or_else(|| {
            ValidationError::new(
                "type_mismatch",
                format!("Expected string, got {}", self.kind().name()),
            )
        })
    }
}

impl AsValidatable<i64> for nebula_value::Value {
    type Output<'a> = i64;

    #[inline]
    fn as_validatable(&self) -> Result<i64, ValidationError> {
        self.to_integer().map_err(|e| {
            ValidationError::new("type_mismatch", format!("Cannot convert to integer: {e}"))
        })
    }
}

impl AsValidatable<f64> for nebula_value::Value {
    type Output<'a> = f64;

    #[inline]
    fn as_validatable(&self) -> Result<f64, ValidationError> {
        self.to_float().map_err(|e| {
            ValidationError::new("type_mismatch", format!("Cannot convert to float: {e}"))
        })
    }
}

impl AsValidatable<bool> for nebula_value::Value {
    type Output<'a> = bool;

    #[inline]
    fn as_validatable(&self) -> Result<bool, ValidationError> {
        Ok(self.to_boolean())
    }
}

impl AsValidatable<nebula_value::Array> for nebula_value::Value {
    type Output<'a>
        = &'a nebula_value::Array
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&nebula_value::Array, ValidationError> {
        self.as_array().ok_or_else(|| {
            ValidationError::new(
                "type_mismatch",
                format!("Expected array, got {}", self.kind().name()),
            )
        })
    }
}

impl AsValidatable<nebula_value::Object> for nebula_value::Value {
    type Output<'a>
        = &'a nebula_value::Object
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&nebula_value::Object, ValidationError> {
        self.as_object().ok_or_else(|| {
            ValidationError::new(
                "type_mismatch",
                format!("Expected object, got {}", self.kind().name()),
            )
        })
    }
}

/// AsValidatable for Value to slice of Values.
///
/// This enables collection validators to work with Value arrays.
/// Note: Returns a Vec because im::Vector doesn't expose contiguous slice.
impl AsValidatable<[nebula_value::Value]> for nebula_value::Value {
    type Output<'a>
        = Vec<nebula_value::Value>
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<Vec<nebula_value::Value>, ValidationError> {
        let arr = self.as_array().ok_or_else(|| {
            ValidationError::new(
                "type_mismatch",
                format!("Expected array, got {}", self.kind().name()),
            )
        })?;
        Ok(arr.iter().cloned().collect())
    }
}

/// AsValidatable for Array to slice of Values.
impl AsValidatable<[nebula_value::Value]> for nebula_value::Array {
    type Output<'a>
        = Vec<nebula_value::Value>
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<Vec<nebula_value::Value>, ValidationError> {
        Ok(self.iter().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Validator;

    #[test]
    fn test_str_identity() {
        let s: &str = "hello";
        let output = s.as_validatable().unwrap();
        let result: &str = output.borrow();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_string_to_str() {
        let s = String::from("hello");
        let output = s.as_validatable().unwrap();
        let result: &str = output.borrow();
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_i64_identity() {
        let n: i64 = 42;
        let output: i64 = AsValidatable::<i64>::as_validatable(&n).unwrap();
        let result: &i64 = output.borrow();
        assert_eq!(*result, 42);
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
        let result: &[i32] = output.borrow();
        assert_eq!(result, &[1, 2, 3]);
    }

    #[test]
    fn test_cow_str() {
        use std::borrow::Cow;

        let borrowed: Cow<str> = Cow::Borrowed("hello");
        let output = borrowed.as_validatable().unwrap();
        let result: &str = output.borrow();
        assert_eq!(result, "hello");

        let owned: Cow<str> = Cow::Owned(String::from("world"));
        let output = owned.as_validatable().unwrap();
        let result: &str = output.borrow();
        assert_eq!(result, "world");
    }

    #[test]
    fn test_validate_any_with_string_validator() {
        use crate::core::{ValidationError, Validator};

        struct MinLength {
            min: usize,
        }

        impl Validator for MinLength {
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
        use crate::core::{ValidationError, Validator};

        struct MinValue {
            min: i64,
        }

        impl Validator for MinValue {
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

    #[test]
    fn test_value_to_str() {
        let value = nebula_value::Value::text("hello");
        let result: Result<&str, _> = AsValidatable::<str>::as_validatable(&value);
        assert_eq!(result.unwrap(), "hello");
    }

    #[test]
    fn test_value_to_str_wrong_type() {
        let value = nebula_value::Value::integer(42);
        let result: Result<&str, _> = AsValidatable::<str>::as_validatable(&value);
        assert!(result.is_err());
    }

    #[test]
    fn test_value_to_i64() {
        let value = nebula_value::Value::integer(42);
        let result: Result<i64, _> = AsValidatable::<i64>::as_validatable(&value);
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn test_value_to_f64() {
        let value = nebula_value::Value::float(3.14);
        let result: Result<f64, _> = AsValidatable::<f64>::as_validatable(&value);
        assert!((result.unwrap() - 3.14).abs() < 0.001);
    }

    #[test]
    fn test_value_to_bool() {
        let value = nebula_value::Value::Boolean(true);
        let result: Result<bool, _> = AsValidatable::<bool>::as_validatable(&value);
        assert!(result.unwrap());
    }

    #[test]
    fn test_value_to_array() {
        let arr = nebula_value::Array::from_iter([
            nebula_value::Value::integer(1),
            nebula_value::Value::integer(2),
        ]);
        let value = nebula_value::Value::Array(arr);
        let result: Result<&nebula_value::Array, _> =
            AsValidatable::<nebula_value::Array>::as_validatable(&value);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }

    #[test]
    fn test_value_to_array_wrong_type() {
        let value = nebula_value::Value::text("not an array");
        let result: Result<&nebula_value::Array, _> =
            AsValidatable::<nebula_value::Array>::as_validatable(&value);
        assert!(result.is_err());
    }

    #[test]
    fn test_value_to_object() {
        let obj = nebula_value::Object::from_iter([(
            "name".to_string(),
            nebula_value::Value::text("Alice"),
        )]);
        let value = nebula_value::Value::Object(obj);
        let result: Result<&nebula_value::Object, _> =
            AsValidatable::<nebula_value::Object>::as_validatable(&value);
        assert!(result.is_ok());
    }

    #[test]
    fn test_value_to_slice() {
        let arr = nebula_value::Array::from_iter([
            nebula_value::Value::integer(1),
            nebula_value::Value::integer(2),
            nebula_value::Value::integer(3),
        ]);
        let value = nebula_value::Value::Array(arr);
        let result: Result<Vec<nebula_value::Value>, _> =
            AsValidatable::<[nebula_value::Value]>::as_validatable(&value);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 3);
    }

    #[test]
    fn test_validate_value_with_string_validator() {
        use crate::validators::string::min_length;

        let validator = min_length(3);
        let value = nebula_value::Value::text("hello");

        // Value implements AsValidatable<str>, so validate_any works
        assert!(validator.validate_any(&value).is_ok());

        let short_value = nebula_value::Value::text("hi");
        assert!(validator.validate_any(&short_value).is_err());
    }

    #[test]
    fn test_validate_value_with_numeric_validator() {
        use crate::validators::numeric::min;

        let validator = min(10.0f64);
        let value = nebula_value::Value::float(25.0);

        assert!(validator.validate_any(&value).is_ok());

        let small_value = nebula_value::Value::float(5.0);
        assert!(validator.validate_any(&small_value).is_err());
    }
}
