//! Value kinds and type-compatibility utilities.
//!
//! This module defines `ValueKind` (a lightweight classification for `Value`)
//! and `TypeCompatibility` helpers for conversions, comparisons, arithmetic,
//! concatenation and result type inference.
//!
//! Quick example:
//! ```rust
//! use nebula_value::{Value, ValueKind};
//!
//! let v = Value::from(3.14);
//! assert_eq!(ValueKind::from_value(&v), ValueKind::Float);
//! assert!(ValueKind::Float.is_numeric());
//! assert_eq!(ValueKind::Float.code(), 'f');
//! assert_eq!(ValueKind::from_code('i'), Some(ValueKind::Integer));
//! ```
//!
//! Feature notes:
//! - When the `decimal` feature is enabled, `Decimal` participates in numeric
//!   operations and has its own type code `'m'`.
//!
use crate::{Value, Number};
use core::fmt::{Display, Formatter};

/// Represents the kind/type of a Value
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum ValueKind {
    Null,
    Boolean,
    Integer,
    Float,
    String,
    Array,
    Object,
    Time,
    Date,
    DateTime,
    Duration,
    Bytes,
    File,
    Decimal,
}

impl ValueKind {
    /// Get all available kinds
    pub fn all() -> Vec<Self> {
        vec![
            Self::Null,
            Self::Boolean,
            Self::Integer,
            Self::Float,
            Self::String,
            Self::Array,
            Self::Object,
            Self::DateTime,
            Self::Duration,
            Self::Bytes,
            Self::File,
            Self::Decimal,
        ]
    }

    /// Check if this kind is numeric
    pub const fn is_numeric(&self) -> bool {
        matches!(self, Self::Integer | Self::Float | Self::Decimal)
    }

    /// Check if this kind is a collection
    pub const fn is_collection(&self) -> bool {
        matches!(self, Self::Array | Self::Object)
    }

    /// Check if this kind is primitive (not a collection)
    pub const fn is_primitive(&self) -> bool {
        !self.is_collection()
    }

    /// Check if this kind is temporal (date/time-related)
    pub const fn is_temporal(&self) -> bool {
        matches!(
            self,
            Self::Date | Self::Time | Self::DateTime | Self::Duration
        )
    }

    /// Get the kind from a Value
    pub fn from_value(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(_) => Self::Boolean,
            Value::Number(Number::Int(_)) => Self::Integer,
            Value::Number(Number::Float(_)) => Self::Float,
            Value::Number(Number::Decimal(_)) => Self::Decimal,
            Value::String(_) => Self::String,
            Value::Array(_) => Self::Array,
            Value::Object(_) => Self::Object,
            Value::Bytes(_) => Self::Bytes,
            Value::Time(_) => Self::Time,
            Value::Date(_) => Self::Date,
            Value::DateTime(_) => Self::DateTime,
            Value::Duration(_) => Self::Duration,
            Value::File(_) => Self::File,
        }
    }

    /// Parse from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "null" | "nil" | "none" => Some(Self::Null),
            "bool" | "boolean" => Some(Self::Boolean),
            "int" | "integer" | "i64" => Some(Self::Integer),
            "float" | "f64" | "double" => Some(Self::Float),
            "string" | "str" | "text" => Some(Self::String),
            "array" | "list" | "vec" => Some(Self::Array),
            "object" | "map" | "dict" => Some(Self::Object),
            _ => None,
        }
    }

    /// Get a descriptive name
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Float => "float",
            Self::String => "string",
            Self::Array => "array",
            Self::Object => "object",
            Self::Time => "time",
            Self::Date => "date",
            Self::DateTime => "datetime",
            Self::Duration => "duration",
            Self::Bytes => "bytes",
            Self::File => "file",
            Self::Decimal => "decimal",
        }
    }

    /// Get a short type code (useful for serialization)
    pub const fn code(&self) -> char {
        match self {
            Self::Null => 'n',
            Self::Boolean => 'b',
            Self::Integer => 'i',
            Self::Float => 'f',
            Self::String => 's',
            Self::Array => 'a',
            Self::Object => 'o',
            Self::Time => 't',
            Self::Date => 'd',
            Self::DateTime => 'x',
            Self::Duration => 'z',
            Self::Bytes => 'y',
            Self::File => 'F',
            Self::Decimal => 'm',
        }
    }

    /// Parse from type code
    pub fn from_code(c: char) -> Option<Self> {
        match c {
            'n' => Some(Self::Null),
            'b' => Some(Self::Boolean),
            'i' => Some(Self::Integer),
            'f' => Some(Self::Float),
            's' => Some(Self::String),
            'a' => Some(Self::Array),
            'o' => Some(Self::Object),
            't' => Some(Self::Time),
            'd' => Some(Self::Date),
            'x' => Some(Self::DateTime),
            'z' => Some(Self::Duration),
            'y' => Some(Self::Bytes),
            'F' => Some(Self::File),
            'm' => Some(Self::Decimal),
            _ => None,
        }
    }

    /// Get an estimated size hint in bytes (if fixed size)
    pub const fn size_hint(&self) -> Option<usize> {
        match self {
            Self::Null => Some(0),
            Self::Boolean => Some(1),
            Self::Integer => Some(8),
            Self::Float => Some(8),
            Self::Decimal => Some(16),
            Self::Date => Some(4),
            Self::Time => Some(8),
            Self::DateTime => Some(12),
            Self::Duration => Some(8),
            Self::String | Self::Array | Self::Object | Self::Bytes | Self::File => None,
        }
    }

    /// Get coercion priority (higher means more general)
    pub const fn coercion_priority(&self) -> u8 {
        match self {
            Self::Null => 0,
            Self::Boolean => 1,
            Self::Integer => 2,
            Self::Float => 3,
            Self::Decimal => 4,
            Self::String => 5,
            Self::Date => 6,
            Self::Time => 7,
            Self::DateTime => 8,
            Self::Duration => 9,
            Self::Bytes => 10,
            Self::Array => 11,
            Self::Object => 12,
            Self::File => 13,
        }
    }
}

impl Display for ValueKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Type compatibility rules
pub struct TypeCompatibility;

impl TypeCompatibility {
    /// Check if a source type can be converted to a target type
    pub fn can_convert(from: ValueKind, to: ValueKind) -> bool {
        if from == to {
            return true;
        }

        match (from, to) {
            // Null can be converted to anything
            (ValueKind::Null, _) => true,

            // Boolean conversions
            (_, ValueKind::Boolean) => true, // Everything can be coerced to bool

            // Numeric conversions
            (ValueKind::Integer, ValueKind::Float) => true,
            (ValueKind::Integer, ValueKind::String) => true,
            (ValueKind::Float, ValueKind::String) => true,

            // String can try to convert to numbers
            (ValueKind::String, ValueKind::Integer | ValueKind::Float) => true,

            // Everything can be converted to string
            (_, ValueKind::String) => true,

            _ => false,
        }
    }

    /// Check if types are compatible for comparison
    pub fn can_compare(a: ValueKind, b: ValueKind) -> bool {
        if a == b {
            return true;
        }

        // Numeric types can be compared
        if a.is_numeric() && b.is_numeric() {
            return true;
        }

        // Null can be compared with anything
        if a == ValueKind::Null || b == ValueKind::Null {
            return true;
        }

        false
    }

    /// Check if arithmetic operations are valid between two kinds
    pub fn can_arithmetic(left: ValueKind, right: ValueKind) -> bool {
        match (left, right) {
            (ValueKind::Integer, ValueKind::Integer) => true,
            (ValueKind::Float, ValueKind::Float) => true,
            (ValueKind::Integer, ValueKind::Float) | (ValueKind::Float, ValueKind::Integer) => true,
            (ValueKind::Decimal, ValueKind::Decimal) => true,
            (ValueKind::Decimal, ValueKind::Integer) | (ValueKind::Integer, ValueKind::Decimal) => {
                true
            }
            (ValueKind::Decimal, ValueKind::Float) | (ValueKind::Float, ValueKind::Decimal) => true,
            _ => false,
        }
    }

    /// Determines if two `ValueKind` variants can be concatenated.
    ///
    /// # Arguments
    ///
    /// * `left` - A `ValueKind` representing the left operand.
    /// * `right` - A `ValueKind` representing the right operand.
    ///
    /// # Returns
    ///
    /// * `true` if the two `ValueKind` variants can be concatenated, otherwise `false`.
    ///
    /// # Supported Concatenation Rules
    ///
    /// - Both `ValueKind::String` values can be concatenated.
    /// - Both `ValueKind::Array` values can be concatenated.
    /// - A `ValueKind::String` value can be concatenated with any other `ValueKind`
    ///   (either as the left or right operand).
    ///
    pub fn can_concatenate(left: ValueKind, right: ValueKind) -> bool {
        match (left, right) {
            (ValueKind::String, ValueKind::String) => true,
            (ValueKind::Array, ValueKind::Array) => true,
            (ValueKind::String, _) | (_, ValueKind::String) => true,
            _ => false,
        }
    }

    /// Determines the resulting `ValueKind` of an operation applied to two operands.
    ///
    /// This function evaluates the type of the result of a given operation between two `ValueKind` operands,
    /// considering type coercion, arithmetic compatibility, and operation-specific rules.
    ///
    /// # Parameters
    ///
    /// * `operation` - A string slice representing the operation to be performed (e.g., "+", "-", "&&").
    /// * `left` - The `ValueKind` of the left operand.
    /// * `right` - The `ValueKind` of the right operand.
    ///
    /// # Returns
    ///
    /// * `Some(ValueKind)` - The resulting `ValueKind` of the operation if it can be determined.
    /// * `None` - If the operation cannot be applied to the given operand types.
    ///
    /// # Rules
    ///
    /// 1. Arithmetic Operations ("+", "-", "*", "/"):
    ///    - The operation is valid if the operand types are supported for arithmetic (`can_arithmetic`).
    ///    - The resulting type is the operand with the higher coercion priority.
    ///    - If the operation is addition ("+") and operands can be concatenated (`can_concatenate`), the behavior is:
    ///        - `Array + Array` produces `ValueKind::Array`.
    ///        - All other concatenations produce `ValueKind::String`.
    ///    - Otherwise, returns `None` for unsupported arithmetic or concatenation.
    ///
    /// 2. Comparison Operations ("==", "!=", "<", ">", "<=", ">="):
    ///    - The operation always returns `ValueKind::Boolean`.
    ///
    /// 3. Logical Operations ("&&", "||"):
    ///    - The operation always returns `ValueKind::Boolean`.
    ///
    /// 4. Unsupported Operations:
    ///    - For unknown operations, return `None`.
    ///
    pub fn result_type(operation: &str, left: ValueKind, right: ValueKind) -> Option<ValueKind> {
        match operation {
            "+" | "-" | "*" | "/" => {
                if Self::can_arithmetic(left, right) {
                    if left.coercion_priority() >= right.coercion_priority() {
                        Some(left)
                    } else {
                        Some(right)
                    }
                } else if operation == "+" && Self::can_concatenate(left, right) {
                    match (left, right) {
                        (ValueKind::Array, ValueKind::Array) => Some(ValueKind::Array),
                        _ => Some(ValueKind::String),
                    }
                } else {
                    None
                }
            }
            "==" | "!=" | "<" | ">" | "<=" | ">=" => Some(ValueKind::Boolean),
            "&&" | "||" => Some(ValueKind::Boolean),
            _ => None,
        }
    }

    /// Get the common type for two kinds (for operations like addition)
    pub fn common_type(a: ValueKind, b: ValueKind) -> Option<ValueKind> {
        if a == b {
            return Some(a);
        }

        match (a, b) {
            // Numeric promotions
            (ValueKind::Integer, ValueKind::Float) | (ValueKind::Float, ValueKind::Integer) => {
                Some(ValueKind::Float)
            }

            // String concatenation
            (ValueKind::String, _) | (_, ValueKind::String) => Some(ValueKind::String),

            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_from_str() {
        assert_eq!(ValueKind::from_str("int"), Some(ValueKind::Integer));
        assert_eq!(ValueKind::from_str("INTEGER"), Some(ValueKind::Integer));
        assert_eq!(ValueKind::from_str("bool"), Some(ValueKind::Boolean));
        assert_eq!(ValueKind::from_str("array"), Some(ValueKind::Array));
        assert_eq!(ValueKind::from_str("invalid"), None);
    }

    #[test]
    fn test_kind_code() {
        assert_eq!(ValueKind::Integer.code(), 'i');
        assert_eq!(ValueKind::from_code('i'), Some(ValueKind::Integer));
        assert_eq!(ValueKind::from_code('x'), Some(ValueKind::DateTime));
    }

    #[test]
    fn test_type_compatibility() {
        assert!(TypeCompatibility::can_convert(
            ValueKind::Integer,
            ValueKind::Float
        ));
        assert!(TypeCompatibility::can_convert(
            ValueKind::Null,
            ValueKind::String
        ));
        assert!(!TypeCompatibility::can_convert(
            ValueKind::Array,
            ValueKind::Integer
        ));

        assert!(TypeCompatibility::can_compare(
            ValueKind::Integer,
            ValueKind::Float
        ));
        assert!(TypeCompatibility::can_compare(
            ValueKind::String,
            ValueKind::String
        ));
        assert!(!TypeCompatibility::can_compare(
            ValueKind::Array,
            ValueKind::Integer
        ));
    }
}
