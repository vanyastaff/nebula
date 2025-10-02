//! Type conversions for Value
//!
//! This module provides TryFrom implementations for extracting native types from Value.

use crate::core::value::Value;
use crate::core::error::{ValueErrorExt, ValueResult};
use crate::core::NebulaError;
use crate::scalar::{Integer, Float, Text, Bytes};
use crate::collections::{Array, Object};
use rust_decimal::Decimal;

// ==================== TryFrom<Value> for primitives ====================

impl TryFrom<Value> for bool {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Boolean(b) => Ok(b),
            _ => Err(NebulaError::value_type_mismatch("Boolean", value.kind().name())),
        }
    }
}

impl TryFrom<&Value> for bool {
    type Error = NebulaError;

    fn try_from(value: &Value) -> ValueResult<Self> {
        value.as_boolean()
            .ok_or_else(|| NebulaError::value_type_mismatch("Boolean", value.kind().name()))
    }
}

impl TryFrom<Value> for i64 {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Integer(i) => Ok(i.value()),
            Value::Float(f) => {
                if f.is_finite() && f.value().fract() == 0.0 {
                    let val = f.value() as i64;
                    // Check for precision loss
                    if (val as f64) == f.value() {
                        Ok(val)
                    } else {
                        Err(NebulaError::value_conversion_error("Float", "i64"))
                    }
                } else {
                    Err(NebulaError::value_conversion_error("Float", "i64"))
                }
            }
            _ => Err(NebulaError::value_type_mismatch("Integer", value.kind().name())),
        }
    }
}

impl TryFrom<&Value> for i64 {
    type Error = NebulaError;

    fn try_from(value: &Value) -> ValueResult<Self> {
        value.as_integer()
            .ok_or_else(|| NebulaError::value_type_mismatch("Integer", value.kind().name()))
    }
}

impl TryFrom<Value> for i32 {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        let i64_val = i64::try_from(value)?;
        i32::try_from(i64_val)
            .map_err(|_| NebulaError::value_conversion_error("i64", "i32"))
    }
}

impl TryFrom<Value> for u32 {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        let i64_val = i64::try_from(value)?;
        u32::try_from(i64_val)
            .map_err(|_| NebulaError::value_conversion_error("i64", "u32"))
    }
}

impl TryFrom<Value> for u64 {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        let i64_val = i64::try_from(value)?;
        if i64_val >= 0 {
            Ok(i64_val as u64)
        } else {
            Err(NebulaError::value_conversion_error("i64", "u64"))
        }
    }
}

impl TryFrom<Value> for f64 {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Float(f) => Ok(f.value()),
            Value::Integer(i) => Ok(i.value() as f64),
            Value::Decimal(d) => {
                use rust_decimal::prelude::ToPrimitive;
                d.to_f64()
                    .ok_or_else(|| NebulaError::value_conversion_error("Decimal", "f64"))
            }
            _ => Err(NebulaError::value_type_mismatch("Float", value.kind().name())),
        }
    }
}

impl TryFrom<&Value> for f64 {
    type Error = NebulaError;

    fn try_from(value: &Value) -> ValueResult<Self> {
        value.as_float()
            .ok_or_else(|| NebulaError::value_type_mismatch("Float", value.kind().name()))
    }
}

impl TryFrom<Value> for f32 {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        let f64_val = f64::try_from(value)?;
        Ok(f64_val as f32)
    }
}

impl TryFrom<Value> for String {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Text(t) => Ok(t.to_string()),
            _ => Err(NebulaError::value_type_mismatch("Text", value.kind().name())),
        }
    }
}

impl TryFrom<&Value> for String {
    type Error = NebulaError;

    fn try_from(value: &Value) -> ValueResult<Self> {
        value.as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| NebulaError::value_type_mismatch("Text", value.kind().name()))
    }
}

impl TryFrom<Value> for Vec<u8> {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Bytes(b) => Ok(b.to_vec()),
            _ => Err(NebulaError::value_type_mismatch("Bytes", value.kind().name())),
        }
    }
}

impl TryFrom<Value> for Decimal {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Decimal(d) => Ok(d),
            Value::Integer(i) => Ok(Decimal::from(i.value())),
            Value::Float(f) => {
                Decimal::try_from(f.value())
                    .map_err(|_| NebulaError::value_conversion_error("Float", "Decimal"))
            }
            _ => Err(NebulaError::value_type_mismatch("Decimal", value.kind().name())),
        }
    }
}

// ==================== TryFrom<Value> for scalar types ====================

impl TryFrom<Value> for Integer {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Integer(i) => Ok(i),
            _ => Err(NebulaError::value_type_mismatch("Integer", value.kind().name())),
        }
    }
}

impl TryFrom<Value> for Float {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Float(f) => Ok(f),
            Value::Integer(i) => Ok(Float::new(i.value() as f64)),
            _ => Err(NebulaError::value_type_mismatch("Float", value.kind().name())),
        }
    }
}

impl TryFrom<Value> for Text {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Text(t) => Ok(t),
            _ => Err(NebulaError::value_type_mismatch("Text", value.kind().name())),
        }
    }
}

impl TryFrom<Value> for Bytes {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Bytes(b) => Ok(b),
            _ => Err(NebulaError::value_type_mismatch("Bytes", value.kind().name())),
        }
    }
}

// ==================== TryFrom<Value> for collections ====================

impl TryFrom<Value> for Array {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Array(a) => Ok(a),
            _ => Err(NebulaError::value_type_mismatch("Array", value.kind().name())),
        }
    }
}

impl TryFrom<Value> for Object {
    type Error = NebulaError;

    fn try_from(value: Value) -> ValueResult<Self> {
        match value {
            Value::Object(o) => Ok(o),
            _ => Err(NebulaError::value_type_mismatch("Object", value.kind().name())),
        }
    }
}

// ==================== Helper trait for convenient conversions ====================

/// Extension trait for Value conversions
pub trait ValueConversion: Sized {
    /// Try to convert this value to the target type
    fn to_value_type<T: TryFrom<Value, Error = NebulaError>>(self) -> ValueResult<T>;

    /// Try to convert this value to the target type, with a fallback
    fn to_value_type_or<T: TryFrom<Value, Error = NebulaError>>(self, default: T) -> T;
}

impl ValueConversion for Value {
    fn to_value_type<T: TryFrom<Value, Error = NebulaError>>(self) -> ValueResult<T> {
        T::try_from(self)
    }

    fn to_value_type_or<T: TryFrom<Value, Error = NebulaError>>(self, default: T) -> T {
        T::try_from(self).unwrap_or(default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_from_bool() {
        let val = Value::Boolean(true);
        assert_eq!(bool::try_from(val).unwrap(), true);

        let val = Value::integer(42);
        assert!(bool::try_from(val).is_err());
    }

    #[test]
    fn test_try_from_i64() {
        let val = Value::integer(42);
        assert_eq!(i64::try_from(val).unwrap(), 42);

        let val = Value::float(42.0);
        assert_eq!(i64::try_from(val).unwrap(), 42);

        let val = Value::float(42.5);
        assert!(i64::try_from(val).is_err());
    }

    #[test]
    fn test_try_from_i32() {
        let val = Value::integer(42);
        assert_eq!(i32::try_from(val).unwrap(), 42);

        let val = Value::integer(i64::MAX);
        assert!(i32::try_from(val).is_err());
    }

    #[test]
    fn test_try_from_u64() {
        let val = Value::integer(42);
        assert_eq!(u64::try_from(val).unwrap(), 42);

        let val = Value::integer(-1);
        assert!(u64::try_from(val).is_err());
    }

    #[test]
    fn test_try_from_f64() {
        let val = Value::float(3.14);
        assert_eq!(f64::try_from(val).unwrap(), 3.14);

        let val = Value::integer(42);
        assert_eq!(f64::try_from(val).unwrap(), 42.0);
    }

    #[test]
    fn test_try_from_string() {
        let val = Value::text("hello");
        assert_eq!(String::try_from(val).unwrap(), "hello");

        let val = Value::integer(42);
        assert!(String::try_from(val).is_err());
    }

    #[test]
    fn test_try_from_vec_u8() {
        let val = Value::bytes(vec![1, 2, 3]);
        assert_eq!(Vec::<u8>::try_from(val).unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn test_try_from_decimal() {
        let val = Value::integer(42);
        let decimal = Decimal::try_from(val).unwrap();
        assert_eq!(decimal, Decimal::from(42));
    }

    #[test]
    fn test_try_from_integer_type() {
        let val = Value::integer(42);
        let int = Integer::try_from(val).unwrap();
        assert_eq!(int.value(), 42);
    }

    #[test]
    fn test_try_from_float_type() {
        let val = Value::float(3.14);
        let float = Float::try_from(val).unwrap();
        assert_eq!(float.value(), 3.14);

        // Integer can be converted to Float
        let val = Value::integer(42);
        let float = Float::try_from(val).unwrap();
        assert_eq!(float.value(), 42.0);
    }

    #[test]
    fn test_try_from_text_type() {
        let val = Value::text("hello");
        let text = Text::try_from(val).unwrap();
        assert_eq!(text.as_str(), "hello");
    }

    #[test]
    fn test_try_from_array_type() {
        let val = Value::array_empty();
        let array = Array::try_from(val).unwrap();
        assert_eq!(array.len(), 0);
    }

    #[test]
    fn test_try_from_object_type() {
        let val = Value::object_empty();
        let object = Object::try_from(val).unwrap();
        assert_eq!(object.len(), 0);
    }

    #[test]
    fn test_value_conversion_trait() {
        let val = Value::integer(42);
        let result: i64 = val.to_value_type().unwrap();
        assert_eq!(result, 42);
    }

    #[test]
    fn test_value_conversion_with_default() {
        let val = Value::text("not a number");
        let result: i64 = val.to_value_type_or(0);
        assert_eq!(result, 0);
    }

    #[test]
    fn test_ref_conversion() {
        let val = Value::integer(42);
        let result = i64::try_from(&val).unwrap();
        assert_eq!(result, 42);
    }
}