//! Arithmetic operations and trait implementations for Number

use super::Number;
use core::cmp::Ordering;
use core::hash::{Hash, Hasher};
// Arithmetic operations will be added later if needed

impl Number {
    /// Check if this number is positive
    pub fn is_positive(&self) -> bool {
        match self {
            Self::Int(i) => *i > 0,
            Self::Float(f) => f.is_finite() && *f > 0.0,
            Self::Decimal(d) => d.is_sign_positive() && !d.is_zero(),
        }
    }

    /// Check if this number is negative
    pub fn is_negative(&self) -> bool {
        match self {
            Self::Int(i) => *i < 0,
            Self::Float(f) => f.is_finite() && *f < 0.0,
            Self::Decimal(d) => d.is_sign_negative() && !d.is_zero(),
        }
    }

    /// Check if this number is zero
    pub fn is_zero(&self) -> bool {
        match self {
            Self::Int(i) => *i == 0,
            Self::Float(f) => *f == 0.0,
            Self::Decimal(d) => d.is_zero(),
        }
    }

    /// Add two numbers, promoting to appropriate precision
    pub fn add(&self, other: &Self) -> Self {
        use Number::*;
        match (self, other) {
            // Same types - preserve type
            (Int(a), Int(b)) => {
                if let Some(result) = a.checked_add(*b) {
                    Int(result)
                } else {
                    // Overflow - promote to decimal
                    let a_dec = rust_decimal::Decimal::try_from(*a).unwrap();
                    let b_dec = rust_decimal::Decimal::try_from(*b).unwrap();
                    Decimal(a_dec + b_dec)
                }
            }
            (Float(a), Float(b)) => Float(a + b),
            (Decimal(a), Decimal(b)) => Decimal(a + b),

            // Mixed types - promote to higher precision
            (Int(a), Float(b)) | (Float(b), Int(a)) => Float(*a as f64 + b),
            (Int(a), Decimal(b)) | (Decimal(b), Int(a)) => {
                let a_dec = rust_decimal::Decimal::try_from(*a).unwrap();
                Decimal(a_dec + b)
            }
            (Float(a), Decimal(b)) | (Decimal(b), Float(a)) => {
                if let Ok(a_dec) = rust_decimal::Decimal::try_from(*a) {
                    Decimal(a_dec + b)
                } else {
                    // Can't convert float to decimal, use float
                    Float(a + f64::try_from(*b).unwrap_or(f64::NAN))
                }
            }
        }
    }
}

impl PartialOrd for Number {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            // Same types
            (Number::Int(a), Number::Int(b)) => a.partial_cmp(b),
            (Number::Float(a), Number::Float(b)) => a.partial_cmp(b),
            (Number::Decimal(a), Number::Decimal(b)) => a.partial_cmp(b),

            // Mixed types - convert to common type for comparison
            (Number::Int(a), Number::Float(b)) => (*a as f64).partial_cmp(b),
            (Number::Float(a), Number::Int(b)) => a.partial_cmp(&(*b as f64)),

            (Number::Int(a), Number::Decimal(b)) => {
                rust_decimal::Decimal::try_from(*a).ok().and_then(|a_dec| a_dec.partial_cmp(b))
            }
            (Number::Decimal(a), Number::Int(b)) => {
                rust_decimal::Decimal::try_from(*b).ok().and_then(|b_dec| a.partial_cmp(&b_dec))
            }

            (Number::Float(a), Number::Decimal(b)) => {
                if let Ok(a_dec) = rust_decimal::Decimal::try_from(*a) {
                    a_dec.partial_cmp(b)
                } else {
                    None // NaN or infinite float
                }
            }
            (Number::Decimal(a), Number::Float(b)) => {
                if let Ok(b_dec) = rust_decimal::Decimal::try_from(*b) {
                    a.partial_cmp(&b_dec)
                } else {
                    None // NaN or infinite float
                }
            }
        }
    }
}

impl Hash for Number {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Number::Int(i) => {
                0u8.hash(state);
                i.hash(state);
            }
            Number::Float(f) => {
                1u8.hash(state);
                if f.is_nan() {
                    // All NaN values hash to the same value
                    f64::NAN.to_bits().hash(state);
                } else {
                    f.to_bits().hash(state);
                }
            }
            Number::Decimal(d) => {
                2u8.hash(state);
                d.hash(state);
            }
        }
    }
}