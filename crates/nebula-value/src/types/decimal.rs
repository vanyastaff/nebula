use core::fmt;
use core::ops::{Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, RemAssign, Sub, SubAssign};
use core::str::FromStr;

use rust_decimal::Decimal as RustDecimal;
use rust_decimal::prelude::*;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ══════════════════════════════════════════════════════════════════════════════
// Error Types
// ══════════════════════════════════════════════════════════════════════════════

/// Result type alias for Decimal operations
pub type DecimalResult<T> = Result<T, DecimalError>;

/// Rich, typed errors for Decimal operations
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum DecimalError {
    #[error("Decimal overflow")]
    Overflow,

    #[error("Decimal underflow")]
    Underflow,

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Invalid decimal string: {input}")]
    InvalidString { input: String },

    #[error("Scale exceeds maximum: {scale} > {max}")]
    ScaleExceedsMaximum { scale: u32, max: u32 },

    #[error("Precision error in operation")]
    PrecisionError,

    #[error("Cannot convert to target type: {reason}")]
    ConversionError { reason: String },
}

// ══════════════════════════════════════════════════════════════════════════════
// Core Types
// ══════════════════════════════════════════════════════════════════════════════

/// High-precision decimal number type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Decimal {
    inner: RustDecimal,
}

impl Decimal {
    // ==================== Constants ====================

    /// Zero value
    pub const ZERO: Self = Self {
        inner: RustDecimal::ZERO,
    };

    /// One value
    pub const ONE: Self = Self {
        inner: RustDecimal::ONE,
    };

    /// Minus one value
    pub const NEGATIVE_ONE: Self = Self {
        inner: RustDecimal::NEGATIVE_ONE,
    };

    /// Maximum value
    pub const MAX: Self = Self {
        inner: RustDecimal::MAX,
    };

    /// Minimum value
    pub const MIN: Self = Self {
        inner: RustDecimal::MIN,
    };

    // ==================== Constructors ====================

    /// Create a new decimal from a RustDecimal
    pub const fn new(inner: RustDecimal) -> Self {
        Self { inner }
    }

    /// Create from integer
    pub fn from_i64(value: i64) -> Self {
        Self {
            inner: RustDecimal::from(value),
        }
    }

    /// Create from float (can lose precision)
    pub fn from_f64(value: f64) -> DecimalResult<Self> {
        RustDecimal::try_from(value)
            .map(|inner| Self { inner })
            .map_err(|_| DecimalError::ConversionError {
                reason: format!("Cannot convert f64 {} to decimal", value),
            })
    }

    /// Create from string representation
    pub fn from_str_exact(s: &str) -> DecimalResult<Self> {
        RustDecimal::from_str(s)
            .map(|inner| Self { inner })
            .map_err(|_| DecimalError::InvalidString {
                input: s.to_string(),
            })
    }

    /// Create from parts (sign, coefficient, scale)
    pub fn from_parts(
        lo: u32,
        mid: u32,
        hi: u32,
        negative: bool,
        scale: u32,
    ) -> DecimalResult<Self> {
        if scale > 28 {
            return Err(DecimalError::ScaleExceedsMaximum { scale, max: 28 });
        }

        let inner = RustDecimal::from_parts(lo, mid, hi, negative, scale);
        Ok(Self { inner })
    }

    // ==================== Accessors ====================

    /// Get the underlying RustDecimal
    pub const fn inner(&self) -> RustDecimal {
        self.inner
    }

    /// Get the integer part
    pub fn integer_part(&self) -> Self {
        Self {
            inner: self.inner.trunc(),
        }
    }

    /// Get the fractional part
    pub fn fractional_part(&self) -> Self {
        Self {
            inner: self.inner.fract(),
        }
    }

    /// Get the scale (number of decimal places)
    pub fn scale(&self) -> u32 {
        self.inner.scale()
    }

    /// Check if the decimal is zero
    pub fn is_zero(&self) -> bool {
        self.inner.is_zero()
    }

    /// Check if the decimal is positive
    pub fn is_positive(&self) -> bool {
        self.inner.is_sign_positive()
    }

    /// Check if the decimal is negative
    pub fn is_negative(&self) -> bool {
        self.inner.is_sign_negative()
    }

    /// Check if the decimal is an integer (no fractional part)
    pub fn is_integer(&self) -> bool {
        self.inner.fract().is_zero()
    }

    /// Get the sign of the decimal as 1, 0, or -1
    pub fn signum(&self) -> Self {
        if self.is_positive() {
            Self::ONE
        } else if self.is_negative() {
            Self::NEGATIVE_ONE
        } else {
            Self::ZERO
        }
    }

    // ==================== Mathematical Operations ====================

    /// Absolute value
    pub fn abs(&self) -> Self {
        Self {
            inner: self.inner.abs(),
        }
    }

    /// Round to nearest integer
    pub fn round(&self) -> Self {
        Self {
            inner: self.inner.round(),
        }
    }

    /// Round up (ceiling)
    pub fn ceil(&self) -> Self {
        Self {
            inner: self.inner.ceil(),
        }
    }

    /// Round down (floor)
    pub fn floor(&self) -> Self {
        Self {
            inner: self.inner.floor(),
        }
    }

    /// Truncate to integer (toward zero)
    pub fn trunc(&self) -> Self {
        Self {
            inner: self.inner.trunc(),
        }
    }

    /// Round to specified decimal places
    pub fn round_dp(&self, dp: u32) -> Self {
        Self {
            inner: self.inner.round_dp(dp),
        }
    }

    /// Power operation
    pub fn pow(&self, exp: u64) -> DecimalResult<Self> {
        if exp == 0 {
            return Ok(Self::ONE);
        }
        if exp == 1 {
            return Ok(*self);
        }

        // Use repeated multiplication for integer powers
        let mut result = Self::ONE;
        let mut base = *self;
        let mut exponent = exp;

        while exponent > 0 {
            if exponent % 2 == 1 {
                result = result.checked_mul(base)?;
            }
            base = base.checked_mul(base)?;
            exponent /= 2;
        }

        Ok(result)
    }

    /// Square root using Newton's method
    pub fn sqrt(&self) -> DecimalResult<Self> {
        if self.is_negative() {
            return Err(DecimalError::ConversionError {
                reason: "Cannot take square root of negative number".to_string(),
            });
        }

        if self.is_zero() {
            return Ok(Self::ZERO);
        }

        // Use Newton's method for square root approximation
        let mut x = *self;
        let mut last_x = Self::ZERO;
        let two = Self::from_i64(2);
        let precision = Self::from_str_exact("0.000000000001")?; // High precision

        // Newton iteration: x_new = (x + n/x) / 2
        while (x - last_x).abs() > precision {
            last_x = x;
            let n_over_x = self.checked_div(x)?;
            x = (x + n_over_x).checked_div(two)?;
        }

        Ok(x)
    }

    /// Maximum of two decimals
    pub fn max(self, other: Self) -> Self {
        if self >= other {
            self
        } else {
            other
        }
    }

    /// Minimum of two decimals
    pub fn min(self, other: Self) -> Self {
        if self <= other {
            self
        } else {
            other
        }
    }

    // ==================== Conversions ====================

    /// Convert to i64 (truncating)
    pub fn to_i64(&self) -> Option<i64> {
        self.inner.to_i64()
    }

    /// Convert to f64 (may lose precision)
    pub fn to_f64(&self) -> f64 {
        self.inner.to_f64().unwrap_or(f64::NAN)
    }

    /// Convert to u64 (truncating, returns None if negative)
    pub fn to_u64(&self) -> Option<u64> {
        self.inner.to_u64()
    }

    /// Try to convert to exact i64
    pub fn try_to_i64(&self) -> DecimalResult<i64> {
        if !self.is_integer() {
            return Err(DecimalError::ConversionError {
                reason: "Decimal has fractional part".to_string(),
            });
        }

        self.to_i64().ok_or(DecimalError::ConversionError {
            reason: "Value out of range for i64".to_string(),
        })
    }

    // ==================== Checked Operations ====================

    /// Checked addition
    pub fn checked_add(self, other: Self) -> DecimalResult<Self> {
        self.inner
            .checked_add(other.inner)
            .map(|inner| Self { inner })
            .ok_or(DecimalError::Overflow)
    }

    /// Checked subtraction
    pub fn checked_sub(self, other: Self) -> DecimalResult<Self> {
        self.inner
            .checked_sub(other.inner)
            .map(|inner| Self { inner })
            .ok_or(DecimalError::Underflow)
    }

    /// Checked multiplication
    pub fn checked_mul(self, other: Self) -> DecimalResult<Self> {
        self.inner
            .checked_mul(other.inner)
            .map(|inner| Self { inner })
            .ok_or(DecimalError::Overflow)
    }

    /// Checked division
    pub fn checked_div(self, other: Self) -> DecimalResult<Self> {
        if other.is_zero() {
            return Err(DecimalError::DivisionByZero);
        }

        self.inner
            .checked_div(other.inner)
            .map(|inner| Self { inner })
            .ok_or(DecimalError::Overflow)
    }

    /// Checked remainder
    pub fn checked_rem(self, other: Self) -> DecimalResult<Self> {
        if other.is_zero() {
            return Err(DecimalError::DivisionByZero);
        }

        self.inner
            .checked_rem(other.inner)
            .map(|inner| Self { inner })
            .ok_or(DecimalError::Overflow)
    }

    // ==================== Rescaling ====================

    /// Rescale to specified decimal places
    pub fn rescale(&self, scale: u32) -> DecimalResult<Self> {
        if scale > 28 {
            return Err(DecimalError::ScaleExceedsMaximum { scale, max: 28 });
        }

        let mut inner = self.inner;
        inner.rescale(scale);
        Ok(Self { inner })
    }

    /// Normalize (remove trailing zeros)
    pub fn normalize(&self) -> Self {
        Self {
            inner: self.inner.normalize(),
        }
    }
}

// ==================== Trait Implementations ====================

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl FromStr for Decimal {
    type Err = DecimalError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_str_exact(s)
    }
}

impl Default for Decimal {
    fn default() -> Self {
        Self::ZERO
    }
}

// ==================== Arithmetic Operations ====================

impl Add for Decimal {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            inner: self.inner + rhs.inner,
        }
    }
}

impl AddAssign for Decimal {
    fn add_assign(&mut self, rhs: Self) {
        self.inner += rhs.inner;
    }
}

impl Sub for Decimal {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            inner: self.inner - rhs.inner,
        }
    }
}

impl SubAssign for Decimal {
    fn sub_assign(&mut self, rhs: Self) {
        self.inner -= rhs.inner;
    }
}

impl Mul for Decimal {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self {
            inner: self.inner * rhs.inner,
        }
    }
}

impl MulAssign for Decimal {
    fn mul_assign(&mut self, rhs: Self) {
        self.inner *= rhs.inner;
    }
}

impl Div for Decimal {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output {
        Self {
            inner: self.inner / rhs.inner,
        }
    }
}

impl DivAssign for Decimal {
    fn div_assign(&mut self, rhs: Self) {
        self.inner /= rhs.inner;
    }
}

impl Rem for Decimal {
    type Output = Self;

    fn rem(self, rhs: Self) -> Self::Output {
        Self {
            inner: self.inner % rhs.inner,
        }
    }
}

impl RemAssign for Decimal {
    fn rem_assign(&mut self, rhs: Self) {
        self.inner %= rhs.inner;
    }
}

impl Neg for Decimal {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self {
            inner: -self.inner,
        }
    }
}

// ==================== From Implementations ====================

impl From<i32> for Decimal {
    fn from(value: i32) -> Self {
        Self {
            inner: RustDecimal::from(value),
        }
    }
}

impl From<i64> for Decimal {
    fn from(value: i64) -> Self {
        Self {
            inner: RustDecimal::from(value),
        }
    }
}

impl From<u32> for Decimal {
    fn from(value: u32) -> Self {
        Self {
            inner: RustDecimal::from(value),
        }
    }
}

impl From<u64> for Decimal {
    fn from(value: u64) -> Self {
        Self {
            inner: RustDecimal::from(value),
        }
    }
}

impl TryFrom<f32> for Decimal {
    type Error = DecimalError;

    fn try_from(value: f32) -> Result<Self, Self::Error> {
        RustDecimal::try_from(value)
            .map(|inner| Self { inner })
            .map_err(|_| DecimalError::ConversionError {
                reason: format!("Cannot convert f32 {} to decimal", value),
            })
    }
}

impl TryFrom<f64> for Decimal {
    type Error = DecimalError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Self::from_f64(value)
    }
}

impl From<RustDecimal> for Decimal {
    fn from(value: RustDecimal) -> Self {
        Self { inner: value }
    }
}

impl From<Decimal> for RustDecimal {
    fn from(value: Decimal) -> Self {
        value.inner
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let a = Decimal::from(10);
        let b = Decimal::from(3);

        assert_eq!(a + b, Decimal::from(13));
        assert_eq!(a - b, Decimal::from(7));
        assert_eq!(a * b, Decimal::from(30));
        assert!(a / b > Decimal::from(3));
        assert_eq!(a % b, Decimal::from(1));
    }

    #[test]
    fn test_precision() {
        let a = Decimal::from_str_exact("0.1").unwrap();
        let b = Decimal::from_str_exact("0.2").unwrap();
        let expected = Decimal::from_str_exact("0.3").unwrap();

        assert_eq!(a + b, expected);
    }

    #[test]
    fn test_constants() {
        assert!(Decimal::ZERO.is_zero());
        assert_eq!(Decimal::ONE, Decimal::from(1));
        assert_eq!(Decimal::NEGATIVE_ONE, Decimal::from(-1));
    }

    #[test]
    fn test_rounding() {
        let d = Decimal::from_str_exact("3.14159").unwrap();

        assert_eq!(d.round(), Decimal::from(3));
        assert_eq!(d.ceil(), Decimal::from(4));
        assert_eq!(d.floor(), Decimal::from(3));
        assert_eq!(d.round_dp(2), Decimal::from_str_exact("3.14").unwrap());
    }

    #[test]
    fn test_checked_operations() {
        let a = Decimal::from(10);
        let b = Decimal::from(3);
        let zero = Decimal::ZERO;

        assert!(a.checked_add(b).is_ok());
        assert!(a.checked_sub(b).is_ok());
        assert!(a.checked_mul(b).is_ok());
        assert!(a.checked_div(b).is_ok());
        assert!(a.checked_div(zero).is_err());
        assert!(a.checked_rem(zero).is_err());
    }

    #[test]
    fn test_conversions() {
        let d = Decimal::from(42);

        assert_eq!(d.to_i64(), Some(42));
        assert_eq!(d.to_f64(), 42.0);
        assert_eq!(d.try_to_i64().unwrap(), 42);
    }

    #[test]
    fn test_properties() {
        let positive = Decimal::from(5);
        let negative = Decimal::from(-5);
        let integer = Decimal::from(10);
        let fractional = Decimal::from_str_exact("10.5").unwrap();

        assert!(positive.is_positive());
        assert!(negative.is_negative());
        assert!(integer.is_integer());
        assert!(!fractional.is_integer());
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_serialization() {
        let d = Decimal::from_str_exact("123.456").unwrap();
        let json = serde_json::to_string(&d).unwrap();
        let deserialized: Decimal = serde_json::from_str(&json).unwrap();
        assert_eq!(d, deserialized);
    }
}