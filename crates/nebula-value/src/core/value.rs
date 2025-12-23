//! Unified Value enum that combines all scalar and collection types
//!
//! This is the central type that represents any value in nebula-value.

use crate::collections::{Array, Object};
use crate::core::kind::ValueKind;
use crate::core::{ValueError, ValueResult};
use crate::scalar::{Boolean, Bytes, Float, Integer, Text};
#[cfg(feature = "temporal")]
use crate::temporal::{Date, DateTime, Duration, Time};

// Decimal (rust_decimal)
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

/// Unified value type that can represent any data in nebula-value
///
/// This enum combines all scalar types (Integer, Float, Text, Bytes)
/// and collection types (Array, Object) along with temporal and file types.
#[derive(Debug, Clone, Default)]
pub enum Value {
    /// Null/None value
    #[default]
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

    /// Date (year, month, day)
    #[cfg(feature = "temporal")]
    Date(Date),

    /// Time (hour, minute, second, nanosecond)
    #[cfg(feature = "temporal")]
    Time(Time),

    /// DateTime (date + time + timezone)
    #[cfg(feature = "temporal")]
    DateTime(DateTime),

    /// Duration (time span)
    #[cfg(feature = "temporal")]
    Duration(Duration),
}

impl Value {
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

    /// Create a text value from String or &str
    pub fn text(v: impl Into<String>) -> Self {
        Self::Text(Text::new(v.into()))
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

    /// Create a date value
    #[cfg(feature = "temporal")]
    pub fn date(v: Date) -> Self {
        Self::Date(v)
    }

    /// Create a time value
    #[cfg(feature = "temporal")]
    pub fn time(v: Time) -> Self {
        Self::Time(v)
    }

    /// Create a datetime value
    #[cfg(feature = "temporal")]
    pub fn datetime(v: DateTime) -> Self {
        Self::DateTime(v)
    }

    /// Create a duration value
    #[cfg(feature = "temporal")]
    pub fn duration(v: Duration) -> Self {
        Self::Duration(v)
    }

    // ==================== Type queries ====================

    /// Get the kind of this value
    #[inline]
    #[must_use]
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
            #[cfg(feature = "temporal")]
            Self::Date(_) => ValueKind::Date,
            #[cfg(feature = "temporal")]
            Self::Time(_) => ValueKind::Time,
            #[cfg(feature = "temporal")]
            Self::DateTime(_) => ValueKind::DateTime,
            #[cfg(feature = "temporal")]
            Self::Duration(_) => ValueKind::Duration,
        }
    }

    /// Check if this is null
    #[inline]
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Check if this is a boolean
    #[inline]
    #[must_use]
    pub fn is_boolean(&self) -> bool {
        matches!(self, Self::Boolean(_))
    }

    /// Check if this is an integer
    #[inline]
    #[must_use]
    pub fn is_integer(&self) -> bool {
        matches!(self, Self::Integer(_))
    }

    /// Check if this is a float
    #[inline]
    #[must_use]
    pub fn is_float(&self) -> bool {
        matches!(self, Self::Float(_))
    }

    /// Check if this is a decimal
    #[inline]
    #[must_use]
    pub fn is_decimal(&self) -> bool {
        matches!(self, Self::Decimal(_))
    }

    /// Check if this is numeric (integer, float, or decimal)
    #[inline]
    #[must_use]
    pub fn is_numeric(&self) -> bool {
        matches!(self, Self::Integer(_) | Self::Float(_) | Self::Decimal(_))
    }

    /// Check if this is text
    #[inline]
    #[must_use]
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    /// Check if this is bytes
    #[inline]
    #[must_use]
    pub fn is_bytes(&self) -> bool {
        matches!(self, Self::Bytes(_))
    }

    /// Check if this is an array
    #[inline]
    #[must_use]
    pub fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }

    /// Check if this is an object
    #[inline]
    #[must_use]
    pub fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }

    /// Check if this is a collection (array or object)
    #[inline]
    #[must_use]
    pub fn is_collection(&self) -> bool {
        matches!(self, Self::Array(_) | Self::Object(_))
    }

    // ==================== Conversions (as_*) ====================

    /// Try to get as boolean
    #[inline]
    #[must_use]
    pub fn as_boolean(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    /// Try to get as integer
    ///
    /// Returns `Some(Integer)` if the value is an Integer, or can be losslessly
    /// converted from Decimal or Float (when the value is finite and has no fractional part).
    ///
    /// Returns `None` for Text, Null, and other types. Use `to_integer()` for parsing text.
    #[inline]
    #[must_use]
    pub fn as_integer(&self) -> Option<Integer> {
        match self {
            Self::Decimal(d) => {
                if d.fract().is_zero() {
                    d.to_i64().map(Integer::new)
                } else {
                    None
                }
            }
            Self::Integer(i) => Some(*i),
            Self::Float(f) => {
                let v = f.value();
                if v.is_finite() && v.fract() == 0.0
                    && v >= i64::MIN as f64 && v <= i64::MAX as f64 {
                    Some(Integer::new(v as i64))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Try to get as float reference
    ///
    /// Returns `Some(&Float)` if the value is a Float.
    ///
    /// Returns `None` for all other types. Use `to_float()` for conversion from other numeric types.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    ///
    /// let val = Value::float(3.14);
    /// assert!(val.as_float().is_some());
    ///
    /// let val = Value::integer(42);
    /// assert!(val.as_float().is_none()); // No automatic conversion
    /// ```
    #[inline]
    #[must_use]
    pub fn as_float(&self) -> Option<&Float> {
        match self {
            Self::Float(f) => Some(f),
            // Cannot return owned Float as reference - caller should use to_float() for conversion
            _ => None,
        }
    }

    /// Try to get as float with automatic conversion (may lose precision)
    ///
    /// Returns `Some(Float)` if the value is a Float, or can be converted from Integer or Decimal.
    ///
    /// Returns `None` for Text, Null, and other types. Use `to_float()` for parsing text.
    ///
    /// Note: Converting large integers to float may lose precision.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    ///
    /// let val = Value::float(3.14);
    /// assert!(val.as_float_lossy().is_some());
    ///
    /// let val = Value::integer(42);
    /// assert_eq!(val.as_float_lossy().map(|f| f.value()), Some(42.0));
    /// ```
    #[inline]
    #[must_use]
    pub fn as_float_lossy(&self) -> Option<Float> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Integer(i) => Some(Float::new(i.value() as f64)),
            Self::Decimal(d) => d.to_f64().map(Float::new),
            _ => None,
        }
    }

    /// Try to get as decimal value
    ///
    /// Returns `Some(Decimal)` if the value is a Decimal, or can be converted from Integer or Float.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::Value;
    /// use rust_decimal::Decimal;
    ///
    /// let val = Value::decimal(Decimal::new(314, 2));
    /// assert!(val.as_decimal().is_some());
    ///
    /// let val = Value::integer(42);
    /// assert_eq!(val.as_decimal(), Some(Decimal::from(42)));
    /// ```
    #[inline]
    #[must_use]
    pub fn as_decimal(&self) -> Option<Decimal> {
        match self {
            Self::Decimal(d) => Some(*d),
            Self::Integer(i) => Some(Decimal::from(i.value())),
            Self::Float(f) => Decimal::from_f64_retain(f.value()),
            _ => None,
        }
    }

    /// Try to get as text reference
    #[inline]
    #[must_use]
    pub fn as_text(&self) -> Option<&Text> {
        match self {
            Self::Text(t) => Some(t),
            _ => None,
        }
    }

    /// Try to get as string slice
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(t) => Some(t.as_str()),
            _ => None,
        }
    }

    /// Try to get as bytes reference
    #[inline]
    #[must_use]
    pub fn as_bytes_ref(&self) -> Option<&Bytes> {
        match self {
            Self::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// Try to get as array reference
    #[inline]
    #[must_use]
    pub fn as_array(&self) -> Option<&Array> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }

    /// Try to get as object reference
    #[inline]
    #[must_use]
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
            #[cfg(feature = "temporal")]
            Self::Date(_) => true,
            #[cfg(feature = "temporal")]
            Self::Time(_) => true,
            #[cfg(feature = "temporal")]
            Self::DateTime(_) => true,
            #[cfg(feature = "temporal")]
            Self::Duration(_) => true,
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
                    Err(ValueError::conversion_error("Float", "Integer"))
                }
            }
            Self::Boolean(b) => Ok(if *b { 1 } else { 0 }),
            Self::Text(t) => t
                .as_str()
                .parse::<i64>()
                .map_err(|_| ValueError::conversion_error("Text", "Integer")),
            _ => Err(ValueError::conversion_error(self.kind().name(), "Integer")),
        }
    }

    /// Try to convert to float
    pub fn to_float(&self) -> ValueResult<f64> {
        match self {
            Self::Float(f) => Ok(f.value()),
            Self::Integer(i) => Ok(i.value() as f64),
            Self::Boolean(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Self::Text(t) => t
                .as_str()
                .parse::<f64>()
                .map_err(|_| ValueError::conversion_error("Text", "Float")),
            _ => Err(ValueError::conversion_error(self.kind().name(), "Float")),
        }
    }
}

impl PartialEq for Value {
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
            #[cfg(feature = "temporal")]
            (Self::Date(a), Self::Date(b)) => a == b,
            #[cfg(feature = "temporal")]
            (Self::Time(a), Self::Time(b)) => a == b,
            #[cfg(feature = "temporal")]
            (Self::DateTime(a), Self::DateTime(b)) => a == b,
            #[cfg(feature = "temporal")]
            (Self::Duration(a), Self::Duration(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for Value {}

// ==================== From implementations ====================

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Self::boolean(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Self::integer(v)
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Self::integer(v as i64)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Self::float(v)
    }
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Self::float(v as f64)
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Self::text(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::text(v)
    }
}

impl From<Boolean> for Value {
    fn from(v: Boolean) -> Self {
        Self::Boolean(v.value())
    }
}

impl From<Text> for Value {
    fn from(v: Text) -> Self {
        Self::Text(v)
    }
}

impl From<Bytes> for Value {
    fn from(v: Bytes) -> Self {
        Self::Bytes(v)
    }
}

impl From<Integer> for Value {
    fn from(v: Integer) -> Self {
        Self::Integer(v)
    }
}

impl From<Float> for Value {
    fn from(v: Float) -> Self {
        Self::Float(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_null() {
        let val = Value::null();
        assert!(val.is_null());
        assert_eq!(val.kind(), ValueKind::Null);
    }

    #[test]
    fn test_value_boolean() {
        let val = Value::boolean(true);
        assert!(val.is_boolean());
        assert_eq!(val.as_boolean(), Some(true));
        assert_eq!(val.kind(), ValueKind::Boolean);
    }

    #[test]
    fn test_value_integer() {
        let val = Value::integer(42);
        assert!(val.is_integer());
        assert!(val.is_numeric());
        assert_eq!(val.as_integer(), Some(Integer::new(42)));
        assert_eq!(val.kind(), ValueKind::Integer);
    }

    #[test]
    fn test_value_float() {
        let val = Value::float(3.14);
        assert!(val.is_float());
        assert!(val.is_numeric());
        assert_eq!(val.as_float().map(|f| f.value()), Some(3.14));
        assert_eq!(val.as_float_lossy().map(|f| f.value()), Some(3.14));
        assert_eq!(val.kind(), ValueKind::Float);
    }

    #[test]
    fn test_value_text() {
        let val = Value::text("hello");
        assert!(val.is_text());
        assert_eq!(val.as_str(), Some("hello"));
        assert_eq!(val.kind(), ValueKind::String);
    }

    #[test]
    fn test_value_from_conversions() {
        let val: Value = 42.into();
        assert!(val.is_integer());

        let val: Value = 3.14.into();
        assert!(val.is_float());

        let val: Value = "hello".into();
        assert!(val.is_text());

        let val: Value = true.into();
        assert!(val.is_boolean());
    }

    #[test]
    fn test_value_to_boolean() {
        assert_eq!(Value::null().to_boolean(), false);
        assert_eq!(Value::boolean(true).to_boolean(), true);
        assert_eq!(Value::integer(0).to_boolean(), false);
        assert_eq!(Value::integer(42).to_boolean(), true);
        assert_eq!(Value::text("").to_boolean(), false);
        assert_eq!(Value::text("hello").to_boolean(), true);
    }

    #[test]
    fn test_value_to_integer() {
        assert_eq!(Value::integer(42).to_integer().unwrap(), 42);
        assert_eq!(Value::float(3.14).to_integer().unwrap(), 3);
        assert_eq!(Value::boolean(true).to_integer().unwrap(), 1);
        assert_eq!(Value::boolean(false).to_integer().unwrap(), 0);

        let val = Value::text("42");
        assert_eq!(val.to_integer().unwrap(), 42);

        let val = Value::text("invalid");
        assert!(val.to_integer().is_err());
    }

    #[test]
    fn test_value_equality() {
        let v1 = Value::integer(42);
        let v2 = Value::integer(42);
        let v3 = Value::integer(99);

        assert_eq!(v1, v2);
        assert_ne!(v1, v3);
    }

    #[test]
    fn test_value_display() {
        assert_eq!(Value::null().to_string(), "null");
        assert_eq!(Value::boolean(true).to_string(), "true");
        assert_eq!(Value::integer(42).to_string(), "42");
        assert_eq!(Value::text("hello").to_string(), "hello");
    }

    #[test]
    #[cfg(feature = "serde")]
    fn test_value_from_str() {
        use std::str::FromStr;

        // Parse primitives
        assert_eq!(Value::from_str("null").unwrap(), Value::Null);
        assert_eq!(Value::from_str("true").unwrap(), Value::boolean(true));
        assert_eq!(Value::from_str("false").unwrap(), Value::boolean(false));
        assert_eq!(Value::from_str("42").unwrap().as_integer(), Some(Integer::new(42)));
        assert!(Value::from_str("3.14").unwrap().is_float());
        assert_eq!(Value::from_str("\"hello\"").unwrap().as_str(), Some("hello"));

        // Parse arrays
        let arr: Value = "[1, 2, 3]".parse().unwrap();
        assert!(matches!(arr, Value::Array(_)));

        // Parse objects
        let obj: Value = r#"{"key": "value"}"#.parse().unwrap();
        assert!(matches!(obj, Value::Object(_)));

        // Invalid JSON should error
        assert!(Value::from_str("invalid").is_err());
    }
}

// ==================== FromStr Implementation ====================

#[cfg(feature = "serde")]
impl std::str::FromStr for Value {
    type Err = ValueError;

    /// Parse a Value from a JSON string
    ///
    /// This uses `serde_json` to parse the string and then converts it to a Value.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_value::{Value, Integer};
    /// use std::str::FromStr;
    ///
    /// let value = Value::from_str("42").unwrap();
    /// assert_eq!(value.as_integer(), Some(Integer::new(42)));
    ///
    /// let value: Value = r#"{"name": "Alice"}"#.parse().unwrap();
    /// assert!(matches!(value, Value::Object(_)));
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`ValueError::ParseError`] if the string is not valid JSON.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str::<serde_json::Value>(s)
            .map(Value::from)
            .map_err(|e| ValueError::parse_error("JSON", format!("{}", e)))
    }
}
