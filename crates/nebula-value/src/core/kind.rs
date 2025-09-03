use crate::Value;
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
    #[cfg(feature = "decimal")]
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
            #[cfg(feature = "decimal")]
            Self::Decimal,
        ]
    }

    /// Check if this kind is numeric
    pub const fn is_numeric(&self) -> bool {
        matches!(self, Self::Integer | Self::Float)
    }

    /// Check if this kind is a collection
    pub const fn is_collection(&self) -> bool {
        matches!(self, Self::Array | Self::Object)
    }

    /// Check if this kind is primitive (not a collection)
    pub const fn is_primitive(&self) -> bool {
        !self.is_collection()
    }

    /// Get the kind from a Value
    pub fn from_value(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(_) => Self::Boolean,
            Value::Int(_) => Self::Integer,
            Value::Float(_) => Self::Float,
            Value::String(_) => Self::String,
            Value::Array(_) => Self::Array,
            Value::Object(_) => Self::Object,
            Value::Bytes(_) => Self::Bytes,
            #[cfg(feature = "decimal")]
            Value::Decimal(_) => Self::Decimal,
            Value::Time(_) => Self::Time,
            Value::Date(_) => Self::Date,
            Value::DateTime(_) => Self::DateTime,
            Value::Duration(_) => Self::Duration,
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
            #[cfg(feature = "decimal")]
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
            #[cfg(feature = "decimal")]
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
            #[cfg(feature = "decimal")]
            'm' => Some(Self::Decimal),
            _ => None,
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
    /// Check if source type can be converted to target type
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
        assert_eq!(ValueKind::from_code('x'), None);
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
