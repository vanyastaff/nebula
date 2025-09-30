//! Unified Number type for nebula-value
//!
//! This module provides a unified Number type that can represent integers,
//! floating-point numbers, and decimals in a single enum, with automatic
//! type promotion and comprehensive arithmetic operations.

use core::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use rust_decimal::Decimal;

// Sub-modules
mod error;
mod conversions;
mod ops;
mod legacy;

// Re-exports
pub use error::{NumberError, NumberResult};
pub use conversions::JsonNumberStrategy;
pub use legacy::{Float, Integer};

/// Unified numeric type that can represent integers, floats, and decimals
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Number {
    /// 64-bit signed integer
    Int(i64),
    /// 64-bit floating point
    Float(f64),
    /// Arbitrary precision decimal
    Decimal(Decimal),
}

impl Number {
    // ==================== Constructors ====================

    /// Create an integer number
    #[inline]
    pub fn int(value: i64) -> Self {
        Self::Int(value)
    }

    /// Create a floating-point number
    #[inline]
    pub fn float(value: f64) -> Self {
        Self::Float(value)
    }

    /// Create a decimal number
    #[inline]
    pub fn decimal(value: Decimal) -> Self {
        Self::Decimal(value)
    }

    // ==================== Type queries ====================

    /// Check if this is an integer
    #[inline]
    pub fn is_int(&self) -> bool {
        matches!(self, Self::Int(_))
    }

    /// Check if this is a float
    #[inline]
    pub fn is_float(&self) -> bool {
        matches!(self, Self::Float(_))
    }

    /// Check if this is a decimal
    #[inline]
    pub fn is_decimal(&self) -> bool {
        matches!(self, Self::Decimal(_))
    }

    /// Check if this number is finite (not NaN or infinite)
    #[inline]
    pub fn is_finite(&self) -> bool {
        match self {
            Self::Int(_) | Self::Decimal(_) => true,
            Self::Float(f) => f.is_finite(),
        }
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(i) => write!(f, "{}", i),
            Self::Float(fl) => {
                if fl.is_finite() {
                    write!(f, "{}", fl)
                } else if fl.is_nan() {
                    write!(f, "NaN")
                } else if fl.is_infinite() && fl.is_sign_positive() {
                    write!(f, "Infinity")
                } else {
                    write!(f, "-Infinity")
                }
            }
            Self::Decimal(d) => write!(f, "{}", d),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_number_operations() {
        let a = Number::int(42);
        let b = Number::float(3.14);
        let c = Number::decimal(Decimal::new(1234, 2)); // 12.34

        // Type queries
        assert!(a.is_int());
        assert!(b.is_float());
        assert!(c.is_decimal());

        // Conversions
        assert_eq!(a.to_i64(), Some(42));
        assert_eq!(b.to_f64(), 3.14);
        assert_eq!(c.to_decimal(), Some(Decimal::new(1234, 2)));

        // Mathematical operations
        assert!(a.is_positive());
        assert!(!a.is_zero());

        // Addition with type promotion
        let result = a.add(&b); // int + float = float
        assert!(result.is_float());
        assert_eq!(result.to_f64(), 45.14);

        let result2 = a.add(&c); // int + decimal = decimal
        assert!(result2.is_decimal());
    }
}