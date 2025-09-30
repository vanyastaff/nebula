//! Type conversions and JSON serialization strategies for Number

use super::Number;
use rust_decimal::Decimal;

/// Strategy for JSON serialization of numbers
#[derive(Clone, Debug, PartialEq)]
pub enum JsonNumberStrategy {
    /// Use JSON number
    Number,
    /// Use JSON string
    String,
}

impl Number {
    /// Convert to i64, with potential loss of precision
    pub fn to_i64(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Float(f) => {
                if f.is_finite() && *f >= i64::MIN as f64 && *f <= i64::MAX as f64 {
                    Some(*f as i64)
                } else {
                    None
                }
            }
            Self::Decimal(d) => i64::try_from(*d).ok(),
        }
    }

    /// Convert to f64, always succeeds but may lose precision
    pub fn to_f64(&self) -> f64 {
        match self {
            Self::Int(i) => *i as f64,
            Self::Float(f) => *f,
            Self::Decimal(d) => f64::try_from(*d).unwrap_or(f64::NAN),
        }
    }

    /// Convert to Decimal, may lose precision for very large floats
    pub fn to_decimal(&self) -> Option<Decimal> {
        match self {
            Self::Int(i) => rust_decimal::Decimal::try_from(*i).ok(),
            Self::Float(f) => rust_decimal::Decimal::try_from(*f).ok(),
            Self::Decimal(d) => Some(*d),
        }
    }

    /// How this number should be serialized to JSON
    pub fn json_strategy(&self) -> JsonNumberStrategy {
        match self {
            Self::Int(_) => JsonNumberStrategy::Number,
            Self::Float(f) => {
                if f.is_finite() {
                    JsonNumberStrategy::Number
                } else {
                    JsonNumberStrategy::String  // NaN, Infinity
                }
            }
            Self::Decimal(_) => JsonNumberStrategy::String, // Preserve precision
        }
    }
}