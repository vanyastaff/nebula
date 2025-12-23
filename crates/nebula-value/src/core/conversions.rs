//! Type conversions for Value
//!
//! This module provides TryFrom implementations for extracting native types from Value.

use crate::collections::{Array, Object};
use crate::core::value::Value;
use crate::scalar::{Bytes, Float, Integer, Text};
use rust_decimal::Decimal;
use thiserror::Error;

// ============================================================================
// CONVERSION ERROR TYPES
// ============================================================================

/// Type conversion errors
///
/// Specialized errors for type conversions and coercions.
#[non_exhaustive]
#[derive(Error, Debug, Clone)]
pub enum ConversionError {
    /// Cannot convert between incompatible types
    #[error("Cannot convert from {from} to {to}")]
    Incompatible {
        from: &'static str,
        to: &'static str,
    },

    /// Numeric overflow during conversion
    #[error("Numeric overflow converting to {target}: value {value}")]
    Overflow { target: &'static str, value: String },

    /// Value out of range for target type
    #[error("Value {value} out of range for {target} [{min}, {max}]")]
    OutOfRange {
        value: String,
        target: &'static str,
        min: String,
        max: String,
    },

    /// Precision loss during conversion
    #[error("Precision loss converting {from} to {to}: value {value}")]
    PrecisionLoss {
        from: &'static str,
        to: &'static str,
        value: String,
    },
}

impl ConversionError {
    /// Create an incompatible types error
    #[inline]
    pub fn incompatible(from: &'static str, to: &'static str) -> Self {
        Self::Incompatible { from, to }
    }

    /// Create a numeric overflow error
    #[inline]
    pub fn overflow(target: &'static str, value: impl ToString) -> Self {
        Self::Overflow {
            target,
            value: value.to_string(),
        }
    }

    /// Create an out of range error
    #[inline]
    pub fn out_of_range(
        value: impl ToString,
        target: &'static str,
        min: impl ToString,
        max: impl ToString,
    ) -> Self {
        Self::OutOfRange {
            value: value.to_string(),
            target,
            min: min.to_string(),
            max: max.to_string(),
        }
    }

    /// Create a precision loss error
    #[inline]
    pub fn precision_loss(from: &'static str, to: &'static str, value: impl ToString) -> Self {
        Self::PrecisionLoss {
            from,
            to,
            value: value.to_string(),
        }
    }

    /// Get error code for monitoring
    pub fn code(&self) -> &'static str {
        match self {
            Self::Incompatible { .. } => "CONVERSION_INCOMPATIBLE",
            Self::Overflow { .. } => "CONVERSION_OVERFLOW",
            Self::OutOfRange { .. } => "CONVERSION_OUT_OF_RANGE",
            Self::PrecisionLoss { .. } => "CONVERSION_PRECISION_LOSS",
        }
    }

    /// Check if this is a client error
    #[inline]
    pub fn is_client_error(&self) -> bool {
        true // All conversion errors are client errors
    }
}

/// Result type for conversion operations
pub type ConversionResult<T> = std::result::Result<T, ConversionError>;

// ==================== TryFrom<Value> for primitives ====================

impl TryFrom<Value> for bool {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Boolean(b) => Ok(b),
            _ => Err(ConversionError::incompatible(
                value.kind().name(),
                "Boolean",
            )),
        }
    }
}

impl TryFrom<&Value> for bool {
    type Error = ConversionError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        value
            .as_boolean()
            .ok_or_else(|| ConversionError::incompatible(value.kind().name(), "Boolean"))
    }
}

impl TryFrom<Value> for i64 {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Integer(i) => Ok(i.value()),
            Value::Float(f) => {
                let val = f.value();
                if val.is_finite() && val.fract() == 0.0 {
                    let i = val as i64;
                    // Check if conversion was lossy
                    if (i as f64) == val {
                        Ok(i)
                    } else {
                        Err(ConversionError::overflow("i64", val))
                    }
                } else {
                    Err(ConversionError::precision_loss("Float", "i64", val))
                }
            }
            _ => Err(ConversionError::incompatible(
                value.kind().name(),
                "Integer",
            )),
        }
    }
}

impl TryFrom<&Value> for i64 {
    type Error = ConversionError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        match value {
            Value::Integer(i) => Ok(i.value()),
            Value::Float(f) => {
                let val = f.value();
                if val.is_finite() && val.fract() == 0.0 {
                    let i = val as i64;
                    // Check if conversion was lossy
                    if (i as f64) == val {
                        Ok(i)
                    } else {
                        Err(ConversionError::overflow("i64", val))
                    }
                } else {
                    Err(ConversionError::precision_loss("Float", "i64", val))
                }
            }
            _ => Err(ConversionError::incompatible(
                value.kind().name(),
                "Integer",
            )),
        }
    }
}

impl TryFrom<Value> for i32 {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let i64_val = i64::try_from(value)?;
        i32::try_from(i64_val).map_err(|_| ConversionError::overflow("i32", i64_val))
    }
}

impl TryFrom<Value> for u32 {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let i64_val = i64::try_from(value)?;
        u32::try_from(i64_val)
            .map_err(|_| ConversionError::out_of_range(i64_val, "u32", 0, u32::MAX))
    }
}

impl TryFrom<Value> for u64 {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        let i64_val = i64::try_from(value)?;
        if i64_val >= 0 {
            Ok(i64_val as u64)
        } else {
            Err(ConversionError::out_of_range(i64_val, "u64", 0, u64::MAX))
        }
    }
}

impl TryFrom<Value> for f64 {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Float(f) => Ok(f.value()),
            Value::Integer(i) => Ok(i.value() as f64),
            Value::Decimal(d) => {
                use rust_decimal::prelude::ToPrimitive;
                d.to_f64()
                    .ok_or_else(|| ConversionError::incompatible("Decimal", "f64"))
            }
            _ => Err(ConversionError::incompatible(value.kind().name(), "Float")),
        }
    }
}

impl TryFrom<&Value> for f64 {
    type Error = ConversionError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        match value {
            Value::Float(f) => Ok(f.value()),
            Value::Integer(i) => Ok(i.value() as f64),
            Value::Decimal(d) => {
                use rust_decimal::prelude::ToPrimitive;
                d.to_f64()
                    .ok_or_else(|| ConversionError::incompatible("Decimal", "f64"))
            }
            _ => Err(ConversionError::incompatible(value.kind().name(), "Float")),
        }
    }
}

impl TryFrom<Value> for f32 {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        f64::try_from(value).and_then(|f64_val| {
            if f64_val.is_finite() && (f64_val < f32::MIN as f64 || f64_val > f32::MAX as f64) {
                Err(ConversionError::overflow("f32", f64_val))
            } else {
                Ok(f64_val as f32)
            }
        })
    }
}

impl TryFrom<&Value> for f32 {
    type Error = ConversionError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        f64::try_from(value).and_then(|f64_val| {
            if f64_val.is_finite() && (f64_val < f32::MIN as f64 || f64_val > f32::MAX as f64) {
                Err(ConversionError::overflow("f32", f64_val))
            } else {
                Ok(f64_val as f32)
            }
        })
    }
}

impl TryFrom<&Value> for i32 {
    type Error = ConversionError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        let i64_val = i64::try_from(value)?;
        i32::try_from(i64_val).map_err(|_| ConversionError::overflow("i32", i64_val))
    }
}

impl TryFrom<&Value> for u32 {
    type Error = ConversionError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        let i64_val = i64::try_from(value)?;
        u32::try_from(i64_val)
            .map_err(|_| ConversionError::out_of_range(i64_val, "u32", 0, u32::MAX))
    }
}

impl TryFrom<&Value> for u64 {
    type Error = ConversionError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        let i64_val = i64::try_from(value)?;
        if i64_val >= 0 {
            Ok(i64_val as u64)
        } else {
            Err(ConversionError::out_of_range(i64_val, "u64", 0, u64::MAX))
        }
    }
}

impl TryFrom<Value> for String {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Text(t) => Ok(t.to_string()),
            _ => Err(ConversionError::incompatible(value.kind().name(), "Text")),
        }
    }
}

impl TryFrom<&Value> for String {
    type Error = ConversionError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        value
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| ConversionError::incompatible(value.kind().name(), "Text"))
    }
}

impl TryFrom<Value> for Vec<u8> {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Bytes(b) => Ok(b.to_vec()),
            _ => Err(ConversionError::incompatible(value.kind().name(), "Bytes")),
        }
    }
}

impl TryFrom<Value> for Decimal {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Decimal(d) => Ok(d),
            Value::Integer(i) => Ok(Decimal::from(i.value())),
            Value::Float(f) => Decimal::try_from(f.value())
                .map_err(|_| ConversionError::incompatible("Float", "Decimal")),
            _ => Err(ConversionError::incompatible(
                value.kind().name(),
                "Decimal",
            )),
        }
    }
}

// ==================== TryFrom<Value> for scalar types ====================

impl TryFrom<Value> for Integer {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Integer(i) => Ok(i),
            _ => Err(ConversionError::incompatible(
                value.kind().name(),
                "Integer",
            )),
        }
    }
}

impl TryFrom<Value> for Float {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Float(f) => Ok(f),
            Value::Integer(i) => Ok(Float::new(i.value() as f64)),
            _ => Err(ConversionError::incompatible(value.kind().name(), "Float")),
        }
    }
}

impl TryFrom<Value> for Text {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Text(t) => Ok(t),
            _ => Err(ConversionError::incompatible(value.kind().name(), "Text")),
        }
    }
}

impl TryFrom<Value> for Bytes {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Bytes(b) => Ok(b),
            _ => Err(ConversionError::incompatible(value.kind().name(), "Bytes")),
        }
    }
}

// ==================== TryFrom<Value> for collections ====================

impl TryFrom<Value> for Array {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Array(a) => Ok(a),
            _ => Err(ConversionError::incompatible(value.kind().name(), "Array")),
        }
    }
}

impl TryFrom<Value> for Object {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Object(o) => Ok(o),
            _ => Err(ConversionError::incompatible(value.kind().name(), "Object")),
        }
    }
}

impl TryFrom<Value> for Vec<Value> {
    type Error = ConversionError;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Array(a) => Ok(a.to_vec()),
            _ => Err(ConversionError::incompatible(value.kind().name(), "Vec<Value>")),
        }
    }
}

impl TryFrom<&Value> for Vec<Value> {
    type Error = ConversionError;

    fn try_from(value: &Value) -> Result<Self, Self::Error> {
        match value {
            Value::Array(a) => Ok(a.to_vec()),
            _ => Err(ConversionError::incompatible(value.kind().name(), "Vec<Value>")),
        }
    }
}

// ==================== Helper trait for convenient conversions ====================

/// Extension trait for Value conversions
pub trait ValueConversion: Sized {
    /// Try to convert this value to the target type
    fn to_value_type<T: TryFrom<Self, Error = ConversionError>>(self) -> ConversionResult<T>;

    /// Try to convert this value to the target type, with a fallback
    fn to_value_type_or<T: TryFrom<Self, Error = ConversionError>>(self, default: T) -> T;
}

impl ValueConversion for Value {
    fn to_value_type<T: TryFrom<Value, Error = ConversionError>>(self) -> ConversionResult<T> {
        T::try_from(self)
    }

    fn to_value_type_or<T: TryFrom<Value, Error = ConversionError>>(self, default: T) -> T {
        T::try_from(self).unwrap_or(default)
    }
}
