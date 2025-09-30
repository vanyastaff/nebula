//! Unified Value enum (v2) that combines all scalar and collection types
//!
//! This is the central type that represents any value in nebula-value.

use std::fmt;

use crate::core::error::{ValueErrorExt, ValueResult};
use crate::core::kind::ValueKind;
use crate::core::limits::ValueLimits;
use crate::core::NebulaError;
use crate::scalar::{Integer, Float, HashableFloat, Text, Bytes};
use crate::collections::{Array, Object};

// Temporal types (using v1 for now)
use crate::types::{Date, Time, DateTime, Duration};
// File type (using v1)
use crate::types::File;
// Boolean type (using v1)
use crate::types::Boolean;
// Decimal (using v1)
use rust_decimal::Decimal;

/// Unified value type that can represent any data in nebula-value
///
/// This enum combines all scalar types (Integer, Float, Text, Bytes)
/// and collection types (Array, Object) along with temporal and file types.
#[derive(Debug, Clone)]
pub enum ValueV2 {
    /// Null/None value
    Null,

    /// Boolean value
    Boolean(bool),

    /// Integer number (i64)
    Integer(Integer),

    /// Floating point number (f64)
    Float(Float),

    /// Arbitrary precision decimal
    Decimal(Decimal),

    /// UTF-8 text string
    Text(Text),

    /// Binary data
    Bytes(Bytes),

    /// Array of values
    Array(Array),

    /// Object (key-value map)
    Object(Object),

    /// Date (without time)
    Date(Date),

    /// Time (without date)
    Time(Time),

    /// DateTime (date + time + timezone)
    DateTime(DateTime),

    /// Duration (time span)
    Duration(Duration),

    /// File reference
    File(File),
}

impl ValueV2 {
    // ==================== Constructors ====================

    /// Create a null value
    pub const fn null() -> Self {
        Self::Null
    }

    /// Create a boolean value
    pub const fn boolean(v: bool) -> Self {
        Self::Boolean(v)
    }

    /// Create an integer value
    pub const fn integer(v: i64) -> Self {
        Self::Integer(Integer::new(v))
    }

    /// Create a float value
    pub const fn float(v: f64) -> Self {
        Self::Float(Float::new(v))
    }

    /// Create a decimal value
    pub fn decimal(v: Decimal) -> Self {
        Self::Decimal(v)
    }

    /// Create a text value from String
    pub fn text(v: String) -> Self {
        Self::Text(Text::new(v))
    }

    /// Create a text value from &str
    pub fn text_str(v: &str) -> Self {
        Self::Text(Text::from_str(v))
    }

    /// Create a bytes value
    pub fn bytes(v: Vec<u8>) -> Self {
        Self::Bytes(Bytes::new(v))
    }

    /// Create an empty array value
    pub fn array_empty() -> Self {
        Self::Array(Array::new())
    }

    /// Create an empty object value
    pub fn object_empty() -> Self {
        Self::Object(Object::new())
    }

    // ==================== Type queries ====================

    /// Get the kind of this value
    pub fn kind(&self) -> ValueKind {
        match self {
            Self::Null => ValueKind::Null,
            Self::Boolean(_) => ValueKind::Boolean,
            Self::Integer(_) => ValueKind::Integer,
            Self::Float(_) => ValueKind::Float,
            Self::Decimal(_) => ValueKind::Decimal,
            Self::Text(_) => ValueKind::String,
            Self::Bytes(_) => ValueKind::Bytes,
            Self::Array(_) => ValueKind::Array,
            Self::Object(_) => ValueKind::Object,
            Self::Date(_) => ValueKind::Date,
            Self::Time(_) => ValueKind::Time,
            Self::DateTime(_) => ValueKind::DateTime,
            Self::Duration(_) => ValueKind::Duration,
            Self::File(_) => ValueKind::File,
        }
    }

    /// Check if this is null
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Check if this is a boolean
    pub fn is_boolean(&self) -> bool {
        matches!(self, Self::Boolean(_))
    }

    /// Check if this is an integer
    pub fn is_integer(&self) -> bool {
        matches!(self, Self::Integer(_))
    }

    /// Check if this is a float
    pub fn is_float(&self) -> bool {
        matches!(self, Self::Float(_))
    }

    /// Check if this is a decimal
    pub fn is_decimal(&self) -> bool {
        matches!(self, Self::Decimal(_))
    }

    /// Check if this is numeric (integer, float, or decimal)
    pub fn is_numeric(&self) -> bool {
        matches!(self, Self::Integer(_) | Self::Float(_) | Self::Decimal(_))
    }

    /// Check if this is text
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    /// Check if this is bytes
    pub fn is_bytes(&self) -> bool {
        matches!(self, Self::Bytes(_))
    }

    /// Check if this is an array
    pub fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }

    /// Check if this is an object
    pub fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }

    /// Check if this is a collection (array or object)
    pub fn is_collection(&self) -> bool {
        matches!(self, Self::Array(_) | Self::Object(_))
    }

    /// Check if this is temporal (date, time, datetime, duration)
    pub fn is_temporal(&self) -> bool {
        matches!(self, Self::Date(_) | Self::Time(_) | Self::DateTime(_) | Self::Duration(_))
    }

    /// Check if this is a file
    pub fn is_file(&self) -> bool {
        matches!(self, Self::File(_))
    }

    // ==================== Conversions (as_*) ====================

    /// Try to get as boolean
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get as integer
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(i) => Some(i.value()),
            _ => None,
        }
    }

    /// Try to get as float
    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(f.value()),
            _ => None,
        }
    }

    /// Try to get as text reference
    pub fn as_text(&self) -> Option<&Text> {
        match self {
            Self::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Try to get as string slice
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(t) => Some(t.as_str()),
            _ => None,
        }
    }

    /// Try to get as bytes reference
    pub fn as_bytes_ref(&self) -> Option<&Bytes> {
        match self {
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Try to get as array reference
    pub fn as_array(&self) -> Option<&Array> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Try to get as object reference
    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Self::Object(o) => Some(o),
            _ => None,
        }
    }

    // ==================== Conversions (to_*) ====================

    /// Convert to boolean (with type coercion)
    pub fn to_boolean(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Boolean(b) => *b,
            Self::Integer(i) => i.value() != 0,
            Self::Float(f) => {
                let v = f.value();
                v != 0.0 && !v.is_nan()
            }
            Self::Decimal(d) => !d.is_zero(),
            Self::Text(t) => !t.is_empty(),
            Self::Bytes(b) => !b.is_empty(),
            Self::Array(a) => !a.is_empty(),
            Self::Object(o) => !o.is_empty(),
            _ => true, // temporal and file types are truthy
        }
    }

    /// Try to convert to integer
    pub fn to_integer(&self) -> ValueResult<i64> {
        match self {
            Self::Integer(i) => Ok(i.value()),
            Self::Float(f) => {
                let v = f.value();
                if v.is_finite() {
                    Ok(v as i64)
                } else {
                    Err(NebulaError::value_conversion_error("Float", "Integer"))
                }
            }
            Self::Boolean(b) => Ok(if *b { 1 } else { 0 }),
            Self::Text(t) => {
                t.as_str()
                    .parse::<i64>()
                    .map_err(|_| NebulaError::value_conversion_error("Text", "Integer"))
            }
            _ => Err(NebulaError::value_conversion_error(
                self.kind().name(),
                "Integer",
            )),
        }
    }

    /// Try to convert to float
    pub fn to_float(&self) -> ValueResult<f64> {
        match self {
            Self::Float(f) => Ok(f.value()),
            Self::Integer(i) => Ok(i.value() as f64),
            Self::Boolean(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Self::Text(t) => {
                t.as_str()
                    .parse::<f64>()
                    .map_err(|_| NebulaError::value_conversion_error("Text", "Float"))
            }
            _ => Err(NebulaError::value_conversion_error(
                self.kind().name(),
                "Float",
            )),
        }
    }

    /// Convert to string representation
    pub fn to_string_repr(&self) -> String {
        match self {
            Self::Null => "null".to_string(),
            Self::Boolean(b) => b.to_string(),
            Self::Integer(i) => i.to_string(),
            Self::Float(f) => f.to_string(),
            Self::Decimal(d) => d.to_string(),
            Self::Text(t) => t.to_string(),
            Self::Bytes(b) => format!("<bytes: {}>", b.len()),
            Self::Array(a) => format!("<array: {} items>", a.len()),
            Self::Object(o) => format!("<object: {} keys>", o.len()),
            Self::Date(d) => d.to_string(),
            Self::Time(t) => t.to_string(),
            Self::DateTime(dt) => dt.to_string(),
            Self::Duration(dur) => dur.to_string(),
            Self::File(f) => f.to_string(),
        }
    }
}

impl Default for ValueV2 {
    fn default() -> Self {
        Self::Null
    }
}

impl PartialEq for ValueV2 {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Null, Self::Null) => true,
            (Self::Boolean(a), Self::Boolean(b)) => a == b,
            (Self::Integer(a), Self::Integer(b)) => a == b,
            (Self::Float(a), Self::Float(b)) => a == b,
            (Self::Decimal(a), Self::Decimal(b)) => a == b,
            (Self::Text(a), Self::Text(b)) => a == b,
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::Array(a), Self::Array(b)) => a == b,
            (Self::Object(a), Self::Object(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for ValueV2 {}

impl fmt::Display for ValueV2 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string_repr())
    }
}

// ==================== From implementations ====================

impl From<bool> for ValueV2 {
    fn from(v: bool) -> Self {
        Self::boolean(v)
    }
}

impl From<i64> for ValueV2 {
    fn from(v: i64) -> Self {
        Self::integer(v)
    }
}

impl From<i32> for ValueV2 {
    fn from(v: i32) -> Self {
        Self::integer(v as i64)
    }
}

impl From<f64> for ValueV2 {
    fn from(v: f64) -> Self {
        Self::float(v)
    }
}

impl From<f32> for ValueV2 {
    fn from(v: f32) -> Self {
        Self::float(v as f64)
    }
}

impl From<String> for ValueV2 {
    fn from(v: String) -> Self {
        Self::text(v)
    }
}

impl From<&str> for ValueV2 {
    fn from(v: &str) -> Self {
        Self::text_str(v)
    }
}

impl From<Text> for ValueV2 {
    fn from(v: Text) -> Self {
        Self::Text(v)
    }
}

impl From<Bytes> for ValueV2 {
    fn from(v: Bytes) -> Self {
        Self::Bytes(v)
    }
}

impl From<Integer> for ValueV2 {
    fn from(v: Integer) -> Self {
        Self::Integer(v)
    }
}

impl From<Float> for ValueV2 {
    fn from(v: Float) -> Self {
        Self::Float(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_null() {
        let val = ValueV2::null();
        assert!(val.is_null());
        assert_eq!(val.kind(), ValueKind::Null);
    }

    #[test]
    fn test_value_boolean() {
        let val = ValueV2::boolean(true);
        assert!(val.is_boolean());
        assert_eq!(val.as_boolean(), Some(true));
        assert_eq!(val.kind(), ValueKind::Boolean);
    }

    #[test]
    fn test_value_integer() {
        let val = ValueV2::integer(42);
        assert!(val.is_integer());
        assert!(val.is_numeric());
        assert_eq!(val.as_integer(), Some(42));
        assert_eq!(val.kind(), ValueKind::Integer);
    }

    #[test]
    fn test_value_float() {
        let val = ValueV2::float(3.14);
        assert!(val.is_float());
        assert!(val.is_numeric());
        assert_eq!(val.as_float(), Some(3.14));
        assert_eq!(val.kind(), ValueKind::Float);
    }

    #[test]
    fn test_value_text() {
        let val = ValueV2::text_str("hello");
        assert!(val.is_text());
        assert_eq!(val.as_str(), Some("hello"));
        assert_eq!(val.kind(), ValueKind::String);
    }

    #[test]
    fn test_value_from_conversions() {
        let val: ValueV2 = 42.into();
        assert!(val.is_integer());

        let val: ValueV2 = 3.14.into();
        assert!(val.is_float());

        let val: ValueV2 = "hello".into();
        assert!(val.is_text());

        let val: ValueV2 = true.into();
        assert!(val.is_boolean());
    }

    #[test]
    fn test_value_to_boolean() {
        assert_eq!(ValueV2::null().to_boolean(), false);
        assert_eq!(ValueV2::boolean(true).to_boolean(), true);
        assert_eq!(ValueV2::integer(0).to_boolean(), false);
        assert_eq!(ValueV2::integer(42).to_boolean(), true);
        assert_eq!(ValueV2::text_str("").to_boolean(), false);
        assert_eq!(ValueV2::text_str("hello").to_boolean(), true);
    }

    #[test]
    fn test_value_to_integer() {
        assert_eq!(ValueV2::integer(42).to_integer().unwrap(), 42);
        assert_eq!(ValueV2::float(3.14).to_integer().unwrap(), 3);
        assert_eq!(ValueV2::boolean(true).to_integer().unwrap(), 1);
        assert_eq!(ValueV2::boolean(false).to_integer().unwrap(), 0);

        let val = ValueV2::text_str("42");
        assert_eq!(val.to_integer().unwrap(), 42);

        let val = ValueV2::text_str("invalid");
        assert!(val.to_integer().is_err());
    }

    #[test]
    fn test_value_equality() {
        let v1 = ValueV2::integer(42);
        let v2 = ValueV2::integer(42);
        let v3 = ValueV2::integer(99);

        assert_eq!(v1, v2);
        assert_ne!(v1, v3);
    }

    #[test]
    fn test_value_display() {
        assert_eq!(ValueV2::null().to_string(), "null");
        assert_eq!(ValueV2::boolean(true).to_string(), "true");
        assert_eq!(ValueV2::integer(42).to_string(), "42");
        assert_eq!(ValueV2::text_str("hello").to_string(), "hello");
    }
}