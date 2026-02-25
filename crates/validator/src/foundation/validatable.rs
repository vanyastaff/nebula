//! AsValidatable trait with GAT for universal type conversion
//!
//! This module provides the `AsValidatable` trait that enables validators to accept
//! multiple input types seamlessly through Generic Associated Types (GATs).
//!
//! # Implementing for Custom Types
//!
//! Similar to how serde works, external crates can implement `AsValidatable` for their
//! own types without any feature flags:
//!
//! ```ignore
//! use nebula_validator::foundation::{AsValidatable, ValidationError};
//!
//! // For types that implement Display (uuid, chrono, url, etc.)
//! impl AsValidatable<str> for uuid::Uuid {
//!     type Output<'a> = String;
//!
//!     fn as_validatable(&self) -> Result<String, ValidationError> {
//!         Ok(self.to_string())
//!     }
//! }
//!
//! // For chrono DateTime with RFC3339 format
//! impl<Tz: chrono::TimeZone> AsValidatable<str> for chrono::DateTime<Tz>
//! where
//!     Tz::Offset: std::fmt::Display,
//! {
//!     type Output<'a> = String where Self: 'a;
//!
//!     fn as_validatable(&self) -> Result<String, ValidationError> {
//!         Ok(self.to_rfc3339())
//!     }
//! }
//! ```
//!
//! This design ensures nebula-validator has zero knowledge of third-party crates,
//! avoiding version conflicts entirely.

use crate::foundation::ValidationError;
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
    type Output<'a>
        = &'a str
    where
        Self: 'a;

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

// std::net types — all implement Display
macro_rules! impl_display_as_str {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl AsValidatable<str> for $ty {
                type Output<'a> = String;

                #[inline]
                fn as_validatable(&self) -> Result<String, ValidationError> {
                    Ok(self.to_string())
                }
            }
        )+
    };
}

impl_display_as_str! {
    std::net::IpAddr,
    std::net::Ipv4Addr,
    std::net::Ipv6Addr,
    std::net::SocketAddr,
    std::net::SocketAddrV4,
    std::net::SocketAddrV6,
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

// Numeric widenings — lossless conversions via `From`
macro_rules! impl_numeric_widening {
    ($($from:ty => $to:ty),+ $(,)?) => {
        $(
            impl AsValidatable<$to> for $from {
                type Output<'a> = $to;

                #[inline]
                fn as_validatable(&self) -> Result<$to, ValidationError> {
                    Ok(<$to>::from(*self))
                }
            }
        )+
    };
}

impl_numeric_widening! {
    i8 => i64, i16 => i64, i32 => i64,
    u8 => i64, u16 => i64, u32 => i64,
    f32 => f64,
    i32 => f64,
}

// i64 -> f64 is lossy for |n| > 2^53 (no From impl), handled with precision check.
// The cast-back `f64 as i64` saturates for out-of-range values, so we guard
// against that before the roundtrip comparison.
impl AsValidatable<f64> for i64 {
    type Output<'a> = f64;

    #[inline]
    fn as_validatable(&self) -> Result<f64, ValidationError> {
        let converted = *self as f64;
        // Guard: f64 values >= 2^63 would saturate to i64::MAX on cast-back,
        // producing a false roundtrip match.
        if converted >= 9_223_372_036_854_775_808.0_f64 || converted as i64 != *self {
            return Err(ValidationError::new(
                "precision_loss",
                format!("Integer {self} cannot be represented exactly as f64"),
            )
            .with_param("value", self.to_string()));
        }
        Ok(converted)
    }
}

// ============================================================================
// PATH CONVERSIONS (Path/PathBuf don't implement Display)
// ============================================================================

impl AsValidatable<str> for std::path::Path {
    type Output<'a>
        = &'a str
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&str, ValidationError> {
        self.to_str().ok_or_else(|| {
            ValidationError::new(
                "invalid_path",
                "Path contains non-UTF-8 characters and cannot be validated as a string",
            )
        })
    }
}

impl AsValidatable<str> for std::path::PathBuf {
    type Output<'a>
        = &'a str
    where
        Self: 'a;

    #[inline]
    fn as_validatable(&self) -> Result<&str, ValidationError> {
        self.as_path().to_str().ok_or_else(|| {
            ValidationError::new(
                "invalid_path",
                "Path contains non-UTF-8 characters and cannot be validated as a string",
            )
        })
    }
}

// ============================================================================
// SERDE JSON VALUE CONVERSIONS
// ============================================================================

/// Returns a human-readable type name for a JSON value.
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
    fn test_i64_to_f64_precision_ok() {
        // 2^53 is exactly representable
        let n: i64 = 1 << 53;
        let result: f64 = AsValidatable::<f64>::as_validatable(&n).unwrap();
        assert_eq!(result, 9007199254740992.0);
    }

    #[test]
    fn test_i64_to_f64_precision_loss() {
        // 2^53 + 1 loses precision
        let n: i64 = (1 << 53) + 1;
        let err = AsValidatable::<f64>::as_validatable(&n).unwrap_err();
        assert_eq!(err.code.as_ref(), "precision_loss");
    }

    #[test]
    fn test_i64_max_precision_loss() {
        let err = AsValidatable::<f64>::as_validatable(&i64::MAX).unwrap_err();
        assert_eq!(err.code.as_ref(), "precision_loss");
    }

    #[test]
    fn test_i64_to_f64_small_values_ok() {
        for n in [-100i64, -1, 0, 1, 42, 100] {
            assert!(AsValidatable::<f64>::as_validatable(&n).is_ok());
        }
    }

    #[test]
    fn std_net_types_as_str() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

        let ip4: Ipv4Addr = "192.168.0.1".parse().unwrap();
        assert_eq!(ip4.as_validatable().unwrap(), "192.168.0.1");

        let ip6: Ipv6Addr = "::1".parse().unwrap();
        assert_eq!(ip6.as_validatable().unwrap(), "::1");

        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert_eq!(ip.as_validatable().unwrap(), "10.0.0.1");

        let addr: SocketAddr = "0.0.0.0:8080".parse().unwrap();
        assert_eq!(addr.as_validatable().unwrap(), "0.0.0.0:8080");
    }

    #[test]
    fn path_types_as_str() {
        use std::path::{Path, PathBuf};

        let path = Path::new("/var/data");
        assert_eq!(path.as_validatable().unwrap(), "/var/data");

        let buf = PathBuf::from("/tmp/file.txt");
        assert_eq!(buf.as_validatable().unwrap(), "/tmp/file.txt");
    }

    #[test]
    fn validate_with_extension_method() {
        use crate::foundation::{Validatable, Validate};
        use crate::validators::{hostname, ipv4};

        // Direct: validator.validate(input)
        assert!(ipv4().validate("192.168.0.1").is_ok());
        assert!(hostname().validate("example.com").is_ok());

        // Extension: input.validate_with(&validator)
        assert!("192.168.0.1".validate_with(&ipv4()).is_ok());
        assert!("example.com".validate_with(&hostname()).is_ok());
    }

    #[test]
    fn test_validate_with_string_validator() {
        use crate::foundation::{Validatable, Validate, ValidationError};

        struct MinLength {
            min: usize,
        }

        impl Validate<str> for MinLength {
            fn validate(&self, input: &str) -> Result<(), ValidationError> {
                if input.len() >= self.min {
                    Ok(())
                } else {
                    Err(ValidationError::new("min_length", "too short"))
                }
            }
        }

        let validator = MinLength { min: 3 };

        // Direct validator style
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());

        // Extension method style
        assert!("hello".validate_with(&validator).is_ok());
        assert!("hi".validate_with(&validator).is_err());

        // Works with String via Validatable
        let s = String::from("hello");
        assert!(validator.validate(&s).is_ok());
    }

    #[test]
    fn test_validate_with_numeric_validator() {
        use crate::foundation::{Validatable, Validate, ValidationError};

        struct MinValue {
            min: i64,
        }

        impl Validate<i64> for MinValue {
            fn validate(&self, input: &i64) -> Result<(), ValidationError> {
                if *input >= self.min {
                    Ok(())
                } else {
                    Err(ValidationError::new("min_value", "too small"))
                }
            }
        }

        let validator = MinValue { min: 10 };

        // Direct validator style
        assert!(validator.validate(&42i64).is_ok());
        assert!(validator.validate(&5i64).is_err());

        // Extension method style
        assert!(42i64.validate_with(&validator).is_ok());
        assert!(5i64.validate_with(&validator).is_err());
    }
}

#[cfg(test)]

mod serde_json_tests {
    use super::*;
    use crate::foundation::Validate;
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

    // -- json_field integration --

    #[test]
    fn json_field_validator_with_json_value() {
        use crate::combinators::json_field;
        use crate::validators::min_length;

        let validator = json_field("", min_length(3));
        assert!(validator.validate(&json!("hello")).is_ok());
        assert!(validator.validate(&json!("hi")).is_err());
    }
}
