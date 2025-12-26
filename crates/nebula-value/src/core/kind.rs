//! Value kinds and type classification

use core::fmt::{Display, Formatter};

/// Represents the kind/type of a Value
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum ValueKind {
    /// Null/nil value
    Null,
    /// Boolean true/false
    Boolean,
    /// Signed 64-bit integer
    Integer,
    /// 64-bit floating point number
    Float,
    /// Arbitrary precision decimal
    Decimal,
    /// UTF-8 string
    String,
    /// Binary data
    Bytes,
    /// Ordered collection of values
    Array,
    /// Key-value map
    Object,
    /// Calendar date
    #[cfg(feature = "temporal")]
    Date,
    /// Time of day
    #[cfg(feature = "temporal")]
    Time,
    /// Date and time with timezone
    #[cfg(feature = "temporal")]
    DateTime,
    /// Time duration
    #[cfg(feature = "temporal")]
    Duration,
}

impl ValueKind {
    /// Get all available kinds
    pub fn all() -> Vec<Self> {
        vec![
            Self::Null,
            Self::Boolean,
            Self::Integer,
            Self::Float,
            Self::Decimal,
            Self::String,
            Self::Bytes,
            Self::Array,
            Self::Object,
            #[cfg(feature = "temporal")]
            Self::Date,
            #[cfg(feature = "temporal")]
            Self::Time,
            #[cfg(feature = "temporal")]
            Self::DateTime,
            #[cfg(feature = "temporal")]
            Self::Duration,
        ]
    }

    /// Get single-character type code for serialization/debugging
    pub fn code(&self) -> char {
        match self {
            Self::Null => 'n',
            Self::Boolean => 'b',
            Self::Integer => 'i',
            Self::Float => 'f',
            Self::Decimal => 'm', // m for "money" / arbitrary precision
            Self::String => 's',
            Self::Bytes => 'y', // y for "bytes"
            Self::Array => 'a',
            Self::Object => 'o',
            #[cfg(feature = "temporal")]
            Self::Date => 'd',
            #[cfg(feature = "temporal")]
            Self::Time => 't',
            #[cfg(feature = "temporal")]
            Self::DateTime => 'D',
            #[cfg(feature = "temporal")]
            Self::Duration => 'r', // r for "duration"
        }
    }

    /// Get ValueKind from a type code character
    pub fn from_code(code: char) -> Option<Self> {
        match code {
            'n' => Some(Self::Null),
            'b' => Some(Self::Boolean),
            'i' => Some(Self::Integer),
            'f' => Some(Self::Float),
            'm' => Some(Self::Decimal),
            's' => Some(Self::String),
            'y' => Some(Self::Bytes),
            'a' => Some(Self::Array),
            'o' => Some(Self::Object),
            #[cfg(feature = "temporal")]
            'd' => Some(Self::Date),
            #[cfg(feature = "temporal")]
            't' => Some(Self::Time),
            #[cfg(feature = "temporal")]
            'D' => Some(Self::DateTime),
            #[cfg(feature = "temporal")]
            'r' => Some(Self::Duration),
            _ => None,
        }
    }

    /// Human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            Self::Null => "Null",
            Self::Boolean => "Boolean",
            Self::Integer => "Integer",
            Self::Float => "Float",
            Self::Decimal => "Decimal",
            Self::String => "String",
            Self::Bytes => "Bytes",
            Self::Array => "Array",
            Self::Object => "Object",
            #[cfg(feature = "temporal")]
            Self::Date => "Date",
            #[cfg(feature = "temporal")]
            Self::Time => "Time",
            #[cfg(feature = "temporal")]
            Self::DateTime => "DateTime",
            #[cfg(feature = "temporal")]
            Self::Duration => "Duration",
        }
    }

    /// Check if this kind is numeric
    pub fn is_numeric(&self) -> bool {
        matches!(self, Self::Integer | Self::Float | Self::Decimal)
    }

    /// Check if this kind is a collection
    pub fn is_collection(&self) -> bool {
        matches!(self, Self::Array | Self::Object)
    }

    /// Check if this kind is temporal (date/time related)
    #[cfg(feature = "temporal")]
    pub fn is_temporal(&self) -> bool {
        matches!(
            self,
            Self::Date | Self::Time | Self::DateTime | Self::Duration
        )
    }
}

impl Display for ValueKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_code() {
        assert_eq!(ValueKind::Integer.code(), 'i');
        assert_eq!(ValueKind::Float.code(), 'f');
        assert_eq!(ValueKind::String.code(), 's');
    }

    #[test]
    fn test_from_code() {
        assert_eq!(ValueKind::from_code('i'), Some(ValueKind::Integer));
        assert_eq!(ValueKind::from_code('f'), Some(ValueKind::Float));
        assert_eq!(ValueKind::from_code('x'), None);
    }

    #[test]
    fn test_is_numeric() {
        assert!(ValueKind::Integer.is_numeric());
        assert!(ValueKind::Float.is_numeric());
        assert!(ValueKind::Decimal.is_numeric());
        assert!(!ValueKind::String.is_numeric());
    }

    #[test]
    fn test_is_collection() {
        assert!(ValueKind::Array.is_collection());
        assert!(ValueKind::Object.is_collection());
        assert!(!ValueKind::Integer.is_collection());
    }
}
