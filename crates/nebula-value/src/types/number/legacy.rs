//! Legacy compatibility types for Integer and Float

use core::fmt;
use core::hash::{Hash, Hasher};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Legacy integer type - now just an alias for i64
pub type Integer = IntegerInner;

/// Legacy float type - now just an alias for f64
pub type Float = FloatInner;

/// Wrapper struct for legacy Integer compatibility
#[derive(Clone, Debug, PartialEq, PartialOrd, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct IntegerInner(i64);

impl IntegerInner {
    pub fn new(value: i64) -> Self {
        Self(value)
    }

    pub fn value(&self) -> i64 {
        self.0
    }
}

impl fmt::Display for IntegerInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Wrapper struct for legacy Float compatibility
#[derive(Clone, Debug, PartialEq, PartialOrd)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FloatInner(f64);

impl FloatInner {
    pub fn new(value: f64) -> Self {
        Self(value)
    }

    pub fn value(&self) -> f64 {
        self.0
    }

    pub fn is_nan(&self) -> bool {
        self.0.is_nan()
    }
}

impl fmt::Display for FloatInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Hash for FloatInner {
    fn hash<H: Hasher>(&self, state: &mut H) {
        if self.0.is_nan() {
            // Normalize all NaN values to the same bit pattern
            f64::NAN.to_bits().hash(state);
        } else if self.0 == 0.0 {
            // Normalize -0.0 and +0.0 to the same value
            0.0f64.to_bits().hash(state);
        } else {
            self.0.to_bits().hash(state);
        }
    }
}