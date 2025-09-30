use std::fmt;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};

/// Signed 64-bit integer
///
/// This is a newtype wrapper around i64 that provides:
/// - Checked arithmetic operations (no panics)
/// - Safe conversions
/// - Proper error handling
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Integer(i64);

impl Integer {
    /// Create a new integer
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// Get the inner value
    pub const fn value(&self) -> i64 {
        self.0
    }

    /// Checked addition (returns None on overflow)
    pub fn checked_add(self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    /// Checked subtraction (returns None on overflow)
    pub fn checked_sub(self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }

    /// Checked multiplication (returns None on overflow)
    pub fn checked_mul(self, other: Self) -> Option<Self> {
        self.0.checked_mul(other.0).map(Self)
    }

    /// Checked division (returns None if divisor is zero or on overflow)
    pub fn checked_div(self, other: Self) -> Option<Self> {
        self.0.checked_div(other.0).map(Self)
    }

    /// Checked remainder (returns None if divisor is zero)
    pub fn checked_rem(self, other: Self) -> Option<Self> {
        self.0.checked_rem(other.0).map(Self)
    }

    /// Checked negation (returns None on overflow for i64::MIN)
    pub fn checked_neg(self) -> Option<Self> {
        self.0.checked_neg().map(Self)
    }

    /// Absolute value with overflow check
    pub fn checked_abs(self) -> Option<Self> {
        self.0.checked_abs().map(Self)
    }
}

impl fmt::Display for Integer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// Conversions from standard types
impl From<i8> for Integer {
    fn from(v: i8) -> Self {
        Self(v as i64)
    }
}

impl From<i16> for Integer {
    fn from(v: i16) -> Self {
        Self(v as i64)
    }
}

impl From<i32> for Integer {
    fn from(v: i32) -> Self {
        Self(v as i64)
    }
}

impl From<i64> for Integer {
    fn from(v: i64) -> Self {
        Self(v)
    }
}

impl From<u8> for Integer {
    fn from(v: u8) -> Self {
        Self(v as i64)
    }
}

impl From<u16> for Integer {
    fn from(v: u16) -> Self {
        Self(v as i64)
    }
}

impl From<u32> for Integer {
    fn from(v: u32) -> Self {
        Self(v as i64)
    }
}

// Try to convert to Integer from larger types
impl TryFrom<u64> for Integer {
    type Error = std::num::TryFromIntError;

    fn try_from(v: u64) -> Result<Self, Self::Error> {
        i64::try_from(v).map(Self)
    }
}

impl TryFrom<i128> for Integer {
    type Error = std::num::TryFromIntError;

    fn try_from(v: i128) -> Result<Self, Self::Error> {
        i64::try_from(v).map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checked_add() {
        let a = Integer::new(5);
        let b = Integer::new(3);
        assert_eq!(a.checked_add(b), Some(Integer::new(8)));

        // Overflow
        let max = Integer::new(i64::MAX);
        assert_eq!(max.checked_add(Integer::new(1)), None);
    }

    #[test]
    fn test_checked_sub() {
        let a = Integer::new(10);
        let b = Integer::new(3);
        assert_eq!(a.checked_sub(b), Some(Integer::new(7)));

        // Overflow
        let min = Integer::new(i64::MIN);
        assert_eq!(min.checked_sub(Integer::new(1)), None);
    }

    #[test]
    fn test_checked_mul() {
        let a = Integer::new(5);
        let b = Integer::new(3);
        assert_eq!(a.checked_mul(b), Some(Integer::new(15)));

        // Overflow
        let max = Integer::new(i64::MAX);
        assert_eq!(max.checked_mul(Integer::new(2)), None);
    }

    #[test]
    fn test_checked_div() {
        let a = Integer::new(10);
        let b = Integer::new(2);
        assert_eq!(a.checked_div(b), Some(Integer::new(5)));

        // Division by zero
        assert_eq!(a.checked_div(Integer::new(0)), None);

        // Overflow (i64::MIN / -1)
        let min = Integer::new(i64::MIN);
        assert_eq!(min.checked_div(Integer::new(-1)), None);
    }

    #[test]
    fn test_checked_neg() {
        let a = Integer::new(5);
        assert_eq!(a.checked_neg(), Some(Integer::new(-5)));

        // Overflow
        let min = Integer::new(i64::MIN);
        assert_eq!(min.checked_neg(), None);
    }

    #[test]
    fn test_conversions() {
        assert_eq!(Integer::from(42i8).value(), 42);
        assert_eq!(Integer::from(42i32).value(), 42);
        assert_eq!(Integer::from(42u16).value(), 42);
    }

    #[test]
    fn test_try_from_u64() {
        assert!(Integer::try_from(100u64).is_ok());
        assert!(Integer::try_from(u64::MAX).is_err());
    }

    #[test]
    fn test_ordering() {
        let a = Integer::new(5);
        let b = Integer::new(10);
        assert!(a < b);
        assert!(b > a);
        assert_eq!(a, Integer::new(5));
    }
}