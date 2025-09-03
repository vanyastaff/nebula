use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::ops::{
    Add, AddAssign, Div, DivAssign, Mul, MulAssign, Neg, Rem, RemAssign, Sub, SubAssign,
};
use core::str::FromStr;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use thiserror::Error;

#[cfg(feature = "decimal")]
use super::Decimal;

// ══════════════════════════════════════════════════════════════════════════════
// Error Types
// ══════════════════════════════════════════════════════════════════════════════

/// Result type alias for number operations
pub type NumberResult<T> = Result<T, NumberError>;

/// Rich, typed errors for number operations
#[derive(Error, Debug, Clone, PartialEq)]
pub enum NumberError {
    #[error("Integer overflow occurred")]
    Overflow,

    #[error("Integer underflow occurred")]
    Underflow,

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Value is not finite (NaN or ±∞)")]
    NotFinite,

    #[error("Value {value} is out of range [{min}, {max}]")]
    OutOfRange {
        value: String, // Using String to handle both i128 and f64
        min: String,
        max: String,
    },

    #[error("Failed to parse '{input}' as {ty}")]
    ParseError { input: String, ty: &'static str },

    #[error("Loss of precision converting from {from} to {to}")]
    PrecisionLoss { from: &'static str, to: &'static str },

    #[error("Cannot convert NaN to integer")]
    NaNConversion,

    #[error("JSON type mismatch: expected number, got {found}")]
    #[cfg(feature = "serde")]
    JsonTypeMismatch { found: &'static str },
}

// ══════════════════════════════════════════════════════════════════════════════
// Integer Type
// ══════════════════════════════════════════════════════════════════════════════

/// High-performance integer type with safe arithmetic
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct Integer(i64);

impl Integer {
    // ════════════════════════════════════════════════════════════════
    // Constants
    // ════════════════════════════════════════════════════════════════

    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1);
    pub const NEG_ONE: Self = Self(-1);
    pub const MIN: Self = Self(i64::MIN);
    pub const MAX: Self = Self(i64::MAX);

    // Common bit patterns
    pub const BIT_32_MAX: Self = Self(i32::MAX as i64);
    pub const BIT_32_MIN: Self = Self(i32::MIN as i64);

    // ════════════════════════════════════════════════════════════════
    // Constructors
    // ════════════════════════════════════════════════════════════════

    /// Creates a new Integer
    #[inline]
    #[must_use]
    pub const fn new(value: i64) -> Self {
        Self(value)
    }

    /// Creates from i128 with bounds checking
    pub fn from_i128(value: i128) -> NumberResult<Self> {
        if value < i64::MIN as i128 || value > i64::MAX as i128 {
            Err(NumberError::OutOfRange {
                value: "i128 value".to_string(),
                min: "i64::MIN".to_string(),
                max: "i64::MAX".to_string(),
            })
        } else {
            Ok(Self(value as i64))
        }
    }

    /// Creates from u64 with bounds checking
    pub const fn from_u64(value: u64) -> NumberResult<Self> {
        if value > i64::MAX as u64 {
            Err(NumberError::Overflow)
        } else {
            Ok(Self(value as i64))
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Accessors
    // ════════════════════════════════════════════════════════════════

    /// Gets the inner value
    #[inline]
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Gets the inner value (alias)
    #[inline]
    #[must_use]
    pub const fn value(self) -> i64 {
        self.0
    }

    /// Consumes and returns the inner value
    #[inline]
    #[must_use]
    pub const fn into_inner(self) -> i64 {
        self.0
    }

    // ════════════════════════════════════════════════════════════════
    // Properties
    // ════════════════════════════════════════════════════════════════

    #[inline]
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    #[inline]
    #[must_use]
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    #[inline]
    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    #[inline]
    #[must_use]
    pub const fn is_even(self) -> bool {
        self.0 & 1 == 0
    }

    #[inline]
    #[must_use]
    pub const fn is_odd(self) -> bool {
        self.0 & 1 == 1
    }

    #[inline]
    #[must_use]
    pub const fn signum(self) -> i64 {
        self.0.signum()
    }

    // ════════════════════════════════════════════════════════════════
    // Checked Arithmetic (returns Result)
    // ════════════════════════════════════════════════════════════════

    #[inline]
    pub const fn checked_add(self, rhs: Self) -> NumberResult<Self> {
        match self.0.checked_add(rhs.0) {
            Some(v) => Ok(Self(v)),
            None => Err(NumberError::Overflow),
        }
    }

    #[inline]
    pub const fn checked_sub(self, rhs: Self) -> NumberResult<Self> {
        match self.0.checked_sub(rhs.0) {
            Some(v) => Ok(Self(v)),
            None => Err(NumberError::Underflow),
        }
    }

    #[inline]
    pub const fn checked_mul(self, rhs: Self) -> NumberResult<Self> {
        match self.0.checked_mul(rhs.0) {
            Some(v) => Ok(Self(v)),
            None => Err(NumberError::Overflow),
        }
    }

    #[inline]
    pub const fn checked_div(self, rhs: Self) -> NumberResult<Self> {
        if rhs.0 == 0 {
            Err(NumberError::DivisionByZero)
        } else {
            match self.0.checked_div(rhs.0) {
                Some(v) => Ok(Self(v)),
                None => Err(NumberError::Overflow),
            }
        }
    }

    #[inline]
    pub const fn checked_rem(self, rhs: Self) -> NumberResult<Self> {
        if rhs.0 == 0 {
            Err(NumberError::DivisionByZero)
        } else {
            match self.0.checked_rem(rhs.0) {
                Some(v) => Ok(Self(v)),
                None => Err(NumberError::Overflow),
            }
        }
    }

    #[inline]
    pub const fn checked_pow(self, exp: u32) -> NumberResult<Self> {
        match self.0.checked_pow(exp) {
            Some(v) => Ok(Self(v)),
            None => Err(NumberError::Overflow),
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Saturating Arithmetic (clamps to limits)
    // ════════════════════════════════════════════════════════════════

    #[inline]
    #[must_use]
    pub const fn saturating_add(self, rhs: Self) -> Self {
        Self(self.0.saturating_add(rhs.0))
    }

    #[inline]
    #[must_use]
    pub const fn saturating_sub(self, rhs: Self) -> Self {
        Self(self.0.saturating_sub(rhs.0))
    }

    #[inline]
    #[must_use]
    pub const fn saturating_mul(self, rhs: Self) -> Self {
        Self(self.0.saturating_mul(rhs.0))
    }

    #[inline]
    #[must_use]
    pub const fn saturating_div(self, rhs: Self) -> Self {
        Self(self.0.saturating_div(rhs.0))
    }

    #[inline]
    #[must_use]
    pub const fn saturating_pow(self, exp: u32) -> Self {
        Self(self.0.saturating_pow(exp))
    }

    // ════════════════════════════════════════════════════════════════
    // Wrapping Arithmetic (wraps on overflow)
    // ════════════════════════════════════════════════════════════════

    #[inline]
    #[must_use]
    pub const fn wrapping_add(self, rhs: Self) -> Self {
        Self(self.0.wrapping_add(rhs.0))
    }

    #[inline]
    #[must_use]
    pub const fn wrapping_sub(self, rhs: Self) -> Self {
        Self(self.0.wrapping_sub(rhs.0))
    }

    #[inline]
    #[must_use]
    pub const fn wrapping_mul(self, rhs: Self) -> Self {
        Self(self.0.wrapping_mul(rhs.0))
    }

    // ════════════════════════════════════════════════════════════════
    // Mathematical Functions
    // ════════════════════════════════════════════════════════════════

    #[inline]
    #[must_use]
    pub const fn abs(self) -> Self {
        Self(self.0.abs())
    }

    #[inline]
    #[must_use]
    pub const fn abs_diff(self, other: Self) -> u64 {
        self.0.abs_diff(other.0)
    }

    /// Greatest Common Divisor (Euclidean algorithm)
    pub fn gcd(self, other: Self) -> Self {
        let mut a = self.0.abs();
        let mut b = other.0.abs();
        while b != 0 {
            let temp = b;
            b = a % b;
            a = temp;
        }
        Self(a)
    }

    /// Least Common Multiple
    pub fn lcm(self, other: Self) -> NumberResult<Self> {
        if self.is_zero() || other.is_zero() {
            return Ok(Self::ZERO);
        }
        let gcd = self.gcd(other);
        self.checked_div(gcd)?.checked_mul(other)
    }

    /// Integer square root
    #[inline]
    #[must_use]
    pub fn isqrt(self) -> Self {
        if self.0 < 0 {
            Self(0) // Or could return error
        } else {
            Self((self.0 as f64).sqrt() as i64)
        }
    }

    /// Clamp to range
    #[inline]
    #[must_use]
    pub const fn clamp(self, min: Self, max: Self) -> Self {
        if self.0 < min.0 {
            min
        } else if self.0 > max.0 {
            max
        } else {
            self
        }
    }

    /// Check if value is in range
    #[inline]
    #[must_use]
    pub const fn in_range(self, min: Self, max: Self) -> bool {
        self.0 >= min.0 && self.0 <= max.0
    }

    // ════════════════════════════════════════════════════════════════
    // Bit Operations
    // ════════════════════════════════════════════════════════════════

    #[inline]
    #[must_use]
    pub const fn count_ones(self) -> u32 {
        self.0.count_ones()
    }

    #[inline]
    #[must_use]
    pub const fn count_zeros(self) -> u32 {
        self.0.count_zeros()
    }

    #[inline]
    #[must_use]
    pub const fn leading_zeros(self) -> u32 {
        self.0.leading_zeros()
    }

    #[inline]
    #[must_use]
    pub const fn trailing_zeros(self) -> u32 {
        self.0.trailing_zeros()
    }

    #[inline]
    #[must_use]
    pub const fn reverse_bits(self) -> Self {
        Self(self.0.reverse_bits())
    }

    #[inline]
    #[must_use]
    pub const fn rotate_left(self, n: u32) -> Self {
        Self(self.0.rotate_left(n))
    }

    #[inline]
    #[must_use]
    pub const fn rotate_right(self, n: u32) -> Self {
        Self(self.0.rotate_right(n))
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Float Type
// ══════════════════════════════════════════════════════════════════════════════

/// High-performance floating-point type with safety features
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct Float(f64);

impl Float {
    // ════════════════════════════════════════════════════════════════
    // Constants
    // ════════════════════════════════════════════════════════════════

    pub const ZERO: Self = Self(0.0);
    pub const ONE: Self = Self(1.0);
    pub const NEG_ONE: Self = Self(-1.0);
    pub const INFINITY: Self = Self(f64::INFINITY);
    pub const NEG_INFINITY: Self = Self(f64::NEG_INFINITY);
    pub const NAN: Self = Self(f64::NAN);
    pub const EPSILON: Self = Self(f64::EPSILON);

    // Mathematical constants
    pub const PI: Self = Self(core::f64::consts::PI);
    pub const TAU: Self = Self(core::f64::consts::TAU);
    pub const E: Self = Self(core::f64::consts::E);
    pub const SQRT_2: Self = Self(core::f64::consts::SQRT_2);

    // ════════════════════════════════════════════════════════════════
    // Constructors
    // ════════════════════════════════════════════════════════════════

    /// Creates a new Float
    #[inline]
    #[must_use]
    pub const fn new(value: f64) -> Self {
        Self(value)
    }

    /// Creates from finite value only
    pub const fn from_finite(value: f64) -> NumberResult<Self> {
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(NumberError::NotFinite)
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Accessors
    // ════════════════════════════════════════════════════════════════

    #[inline]
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn value(self) -> f64 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn into_inner(self) -> f64 {
        self.0
    }

    // ════════════════════════════════════════════════════════════════
    // Properties
    // ════════════════════════════════════════════════════════════════

    #[inline]
    #[must_use]
    pub fn is_nan(self) -> bool {
        self.0.is_nan()
    }

    #[inline]
    #[must_use]
    pub fn is_infinite(self) -> bool {
        self.0.is_infinite()
    }

    #[inline]
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.0.is_finite()
    }

    #[inline]
    #[must_use]
    pub fn is_normal(self) -> bool {
        self.0.is_normal()
    }

    #[inline]
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0 == 0.0
    }

    #[inline]
    #[must_use]
    pub fn is_positive(self) -> bool {
        self.0 > 0.0
    }

    #[inline]
    #[must_use]
    pub fn is_negative(self) -> bool {
        self.0 < 0.0
    }

    #[inline]
    #[must_use]
    pub fn signum(self) -> Self {
        Self(self.0.signum())
    }

    // ════════════════════════════════════════════════════════════════
    // Mathematical Functions
    // ════════════════════════════════════════════════════════════════

    #[inline]
    #[must_use]
    pub fn abs(self) -> Self {
        Self(self.0.abs())
    }

    #[inline]
    #[must_use]
    pub fn floor(self) -> Self {
        Self(self.0.floor())
    }

    #[inline]
    #[must_use]
    pub fn ceil(self) -> Self {
        Self(self.0.ceil())
    }

    #[inline]
    #[must_use]
    pub fn round(self) -> Self {
        Self(self.0.round())
    }

    #[inline]
    #[must_use]
    pub fn trunc(self) -> Self {
        Self(self.0.trunc())
    }

    #[inline]
    #[must_use]
    pub fn fract(self) -> Self {
        Self(self.0.fract())
    }

    #[inline]
    #[must_use]
    pub fn sqrt(self) -> Self {
        Self(self.0.sqrt())
    }

    #[inline]
    #[must_use]
    pub fn cbrt(self) -> Self {
        Self(self.0.cbrt())
    }

    #[inline]
    #[must_use]
    pub fn powi(self, n: i32) -> Self {
        Self(self.0.powi(n))
    }

    #[inline]
    #[must_use]
    pub fn powf(self, n: Float) -> Self {
        Self(self.0.powf(n.0))
    }

    #[inline]
    #[must_use]
    pub fn exp(self) -> Self {
        Self(self.0.exp())
    }

    #[inline]
    #[must_use]
    pub fn exp2(self) -> Self {
        Self(self.0.exp2())
    }

    #[inline]
    #[must_use]
    pub fn ln(self) -> Self {
        Self(self.0.ln())
    }

    #[inline]
    #[must_use]
    pub fn log2(self) -> Self {
        Self(self.0.log2())
    }

    #[inline]
    #[must_use]
    pub fn log10(self) -> Self {
        Self(self.0.log10())
    }

    #[inline]
    #[must_use]
    pub fn recip(self) -> Self {
        Self(self.0.recip())
    }

    #[inline]
    #[must_use]
    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self(self.0.clamp(min.0, max.0))
    }

    #[inline]
    #[must_use]
    pub fn min(self, other: Self) -> Self {
        Self(self.0.min(other.0))
    }

    #[inline]
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        Self(self.0.max(other.0))
    }

    #[inline]
    #[must_use]
    pub fn copysign(self, sign: Self) -> Self {
        Self(self.0.copysign(sign.0))
    }

    // ════════════════════════════════════════════════════════════════
    // Approximate Equality
    // ════════════════════════════════════════════════════════════════

    /// Absolute tolerance comparison
    #[inline]
    #[must_use]
    pub fn approx_eq_abs(self, other: Self, tolerance: f64) -> bool {
        (self.0 - other.0).abs() <= tolerance
    }

    /// Relative tolerance comparison
    #[inline]
    #[must_use]
    pub fn approx_eq_rel(self, other: Self, tolerance: f64) -> bool {
        let diff = (self.0 - other.0).abs();
        let largest = self.0.abs().max(other.0.abs());
        diff <= largest * tolerance
    }

    /// ULP-based comparison
    #[inline]
    #[must_use]
    pub fn approx_eq_ulps(self, other: Self, max_ulps: u64) -> bool {
        if self.0 == other.0 {
            return true;
        }
        if self.is_nan() || other.is_nan() {
            return false;
        }

        let a_bits = self.0.to_bits() as i64;
        let b_bits = other.0.to_bits() as i64;

        (a_bits - b_bits).abs() as u64 <= max_ulps
    }

    /// Combined absolute and relative tolerance
    #[inline]
    #[must_use]
    pub fn approx_eq(self, other: Self, abs_tol: f64, rel_tol: f64) -> bool {
        self.approx_eq_abs(other, abs_tol) || self.approx_eq_rel(other, rel_tol)
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Number Type (Unified Integer/Float)
// ══════════════════════════════════════════════════════════════════════════════

/// Unified number type supporting both integers and floats
///
/// Operations preserve type when possible:
/// - Int + Int = Int (with overflow checking)
/// - Any operation with Float = Float
/// - Division always returns Float for accuracy
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(untagged))]
pub enum Number {
    Int(Integer),
    Float(Float),
    #[cfg(feature = "decimal")]
    Decimal(Decimal), // Future extension
}

impl Number {
    // ════════════════════════════════════════════════════════════════
    // Constants
    // ════════════════════════════════════════════════════════════════

    pub const ZERO: Self = Self::Int(Integer::ZERO);
    pub const ONE: Self = Self::Int(Integer::ONE);
    pub const NEG_ONE: Self = Self::Int(Integer::NEG_ONE);
    pub const PI: Self = Self::Float(Float::PI);
    pub const E: Self = Self::Float(Float::E);
    pub const INFINITY: Self = Self::Float(Float::INFINITY);
    pub const NEG_INFINITY: Self = Self::Float(Float::NEG_INFINITY);

    // ════════════════════════════════════════════════════════════════
    // Constructors
    // ════════════════════════════════════════════════════════════════

    #[inline]
    #[must_use]
    pub const fn from_i64(value: i64) -> Self {
        Self::Int(Integer::new(value))
    }

    #[inline]
    #[must_use]
    pub const fn from_f64(value: f64) -> Self {
        Self::Float(Float::new(value))
    }

    /// Creates from finite float only
    pub fn from_finite(value: f64) -> NumberResult<Self> {
        match Float::from_finite(value) {
            Ok(f) => Ok(Self::Float(f)),
            Err(e) => Err(e),
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Type Checking
    // ════════════════════════════════════════════════════════════════

    #[inline]
    #[must_use]
    pub const fn is_int(&self) -> bool {
        matches!(self, Self::Int(_))
    }

    #[inline]
    #[must_use]
    pub const fn is_float(&self) -> bool {
        matches!(self, Self::Float(_))
    }

    #[inline]
    #[must_use]
    pub fn is_zero(&self) -> bool {
        match self {
            Self::Int(i) => i.is_zero(),
            Self::Float(f) => f.is_zero(),
            #[cfg(feature = "decimal")]
            Self::Decimal(d) => d.is_zero(),
        }
    }

    #[inline]
    #[must_use]
    pub fn is_positive(&self) -> bool {
        match self {
            Self::Int(i) => i.is_positive(),
            Self::Float(f) => f.is_positive(),
            #[cfg(feature = "decimal")]
            Self::Decimal(d) => d.is_positive(),
        }
    }

    #[inline]
    #[must_use]
    pub fn is_negative(&self) -> bool {
        match self {
            Self::Int(i) => i.is_negative(),
            Self::Float(f) => f.is_negative(),
            #[cfg(feature = "decimal")]
            Self::Decimal(d) => d.is_negative(),
        }
    }

    #[inline]
    #[must_use]
    pub fn is_finite(&self) -> bool {
        match self {
            Self::Int(_) => true,
            Self::Float(f) => f.is_finite(),
            #[cfg(feature = "decimal")]
            Self::Decimal(_) => true,
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Conversions
    // ════════════════════════════════════════════════════════════════

    /// Convert to i64 if possible
    pub fn to_i64(&self) -> NumberResult<i64> {
        match self {
            Self::Int(i) => Ok(i.get()),
            Self::Float(f) => {
                let v = f.get();
                if !v.is_finite() {
                    Err(NumberError::NotFinite)
                } else if v.fract() != 0.0 {
                    Err(NumberError::PrecisionLoss { from: "float", to: "i64" })
                } else if v < i64::MIN as f64 || v > i64::MAX as f64 {
                    Err(NumberError::OutOfRange {
                        value: v.to_string(),
                        min: i64::MIN.to_string(),
                        max: i64::MAX.to_string(),
                    })
                } else {
                    Ok(v as i64)
                }
            },
            #[cfg(feature = "decimal")]
            Self::Decimal(d) => d.to_i64(),
        }
    }

    /// Convert to f64 (always possible, but may lose precision for large integers)
    #[inline]
    #[must_use]
    pub fn to_f64(&self) -> f64 {
        match self {
            Self::Int(i) => i.get() as f64,
            Self::Float(f) => f.get(),
            #[cfg(feature = "decimal")]
            Self::Decimal(d) => d.to_f64(),
        }
    }

    /// Try to convert to Integer
    pub fn to_integer(&self) -> NumberResult<Integer> {
        match self {
            Self::Int(i) => Ok(*i),
            _ => self.to_i64().map(Integer::new),
        }
    }

    /// Convert to Float
    #[inline]
    #[must_use]
    pub fn to_float(&self) -> Float {
        Float::new(self.to_f64())
    }

    // ════════════════════════════════════════════════════════════════
    // Mathematical Operations
    // ════════════════════════════════════════════════════════════════

    #[inline]
    #[must_use]
    pub fn abs(&self) -> Self {
        match self {
            Self::Int(i) => Self::Int(i.abs()),
            Self::Float(f) => Self::Float(f.abs()),
            #[cfg(feature = "decimal")]
            Self::Decimal(d) => Self::Decimal(d.abs()),
        }
    }

    #[inline]
    #[must_use]
    pub fn signum(&self) -> Self {
        match self {
            Self::Int(i) => Self::Int(Integer::new(i.signum())),
            Self::Float(f) => Self::Float(f.signum()),
            #[cfg(feature = "decimal")]
            Self::Decimal(d) => Self::Decimal(d.signum()),
        }
    }

    /// Clamp to range
    pub fn clamp(&self, min: Self, max: Self) -> Self {
        match (self, min, max) {
            (Self::Int(v), Self::Int(mn), Self::Int(mx)) => Self::Int(*v.clamp(&mn, &mx)),
            _ => {
                let v = self.to_f64();
                let mn = min.to_f64();
                let mx = max.to_f64();
                Self::Float(Float::new(v.clamp(mn, mx)))
            },
        }
    }

    /// Power function
    pub fn pow(&self, exp: Self) -> NumberResult<Self> {
        match (self, exp) {
            (Self::Int(base), Self::Int(exp))
                if exp.is_positive() && exp.get() <= u32::MAX as i64 =>
            {
                base.checked_pow(exp.get() as u32).map(Self::Int)
            },
            _ => {
                let result = self.to_f64().powf(exp.to_f64());
                if result.is_finite() {
                    Ok(Self::Float(Float::new(result)))
                } else {
                    Err(NumberError::NotFinite)
                }
            },
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Checked Arithmetic
    // ════════════════════════════════════════════════════════════════

    pub fn checked_add(&self, rhs: Self) -> NumberResult<Self> {
        match (self, rhs) {
            (Self::Int(a), Self::Int(b)) => a.checked_add(b).map(Self::Int),
            _ => Ok(Self::Float(Float::new(self.to_f64() + rhs.to_f64()))),
        }
    }

    pub fn checked_sub(&self, rhs: Self) -> NumberResult<Self> {
        match (self, rhs) {
            (Self::Int(a), Self::Int(b)) => a.checked_sub(b).map(Self::Int),
            _ => Ok(Self::Float(Float::new(self.to_f64() - rhs.to_f64()))),
        }
    }

    pub fn checked_mul(&self, rhs: Self) -> NumberResult<Self> {
        match (self, rhs) {
            (Self::Int(a), Self::Int(b)) => a.checked_mul(b).map(Self::Int),
            _ => Ok(Self::Float(Float::new(self.to_f64() * rhs.to_f64()))),
        }
    }

    pub fn checked_div(&self, rhs: Self) -> NumberResult<Self> {
        if rhs.is_zero() {
            return Err(NumberError::DivisionByZero);
        }
        // Division always returns float for precision
        Ok(Self::Float(Float::new(self.to_f64() / rhs.to_f64())))
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Trait Implementations for Integer
// ══════════════════════════════════════════════════════════════════════════════

impl Default for Integer {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}

impl fmt::Display for Integer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Integer {
    type Err = NumberError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.trim()
            .parse::<i64>()
            .map(Self::new)
            .map_err(|_| NumberError::ParseError { input: s.to_string(), ty: "Integer" })
    }
}

// Conversions
impl From<i8> for Integer {
    #[inline]
    fn from(v: i8) -> Self {
        Self(v as i64)
    }
}
impl From<i16> for Integer {
    #[inline]
    fn from(v: i16) -> Self {
        Self(v as i64)
    }
}
impl From<i32> for Integer {
    #[inline]
    fn from(v: i32) -> Self {
        Self(v as i64)
    }
}
impl From<i64> for Integer {
    #[inline]
    fn from(v: i64) -> Self {
        Self(v)
    }
}
impl From<u8> for Integer {
    #[inline]
    fn from(v: u8) -> Self {
        Self(v as i64)
    }
}
impl From<u16> for Integer {
    #[inline]
    fn from(v: u16) -> Self {
        Self(v as i64)
    }
}
impl From<u32> for Integer {
    #[inline]
    fn from(v: u32) -> Self {
        Self(v as i64)
    }
}

impl TryFrom<u64> for Integer {
    type Error = NumberError;

    #[inline]
    fn try_from(v: u64) -> Result<Self, Self::Error> {
        Self::from_u64(v)
    }
}

impl TryFrom<usize> for Integer {
    type Error = NumberError;

    fn try_from(v: usize) -> Result<Self, Self::Error> {
        if v <= i64::MAX as usize {
            Ok(Self(v as i64))
        } else {
            Err(NumberError::Overflow)
        }
    }
}

impl From<Integer> for i64 {
    #[inline]
    fn from(v: Integer) -> Self {
        v.0
    }
}

// Arithmetic operators with saturating semantics by default
impl Add for Integer {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        self.saturating_add(rhs)
    }
}

impl Sub for Integer {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        self.saturating_sub(rhs)
    }
}

impl Mul for Integer {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        self.saturating_mul(rhs)
    }
}

impl Div for Integer {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self::Output {
        if rhs.0 == 0 {
            panic!("division by zero");
        }
        Self(self.0 / rhs.0)
    }
}

impl Rem for Integer {
    type Output = Self;
    #[inline]
    fn rem(self, rhs: Self) -> Self::Output {
        if rhs.0 == 0 {
            panic!("division by zero");
        }
        Self(self.0 % rhs.0)
    }
}

impl Neg for Integer {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self::Output {
        Self(self.0.saturating_neg())
    }
}

// Assignment operators
impl AddAssign for Integer {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}
impl SubAssign for Integer {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}
impl MulAssign for Integer {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}
impl DivAssign for Integer {
    #[inline]
    fn div_assign(&mut self, rhs: Self) {
        *self = *self / rhs;
    }
}
impl RemAssign for Integer {
    #[inline]
    fn rem_assign(&mut self, rhs: Self) {
        *self = *self % rhs;
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Trait Implementations for Float
// ══════════════════════════════════════════════════════════════════════════════

impl Default for Float {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}

impl fmt::Display for Float {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Smart formatting: avoid .0 for whole numbers
        if self.0.fract() == 0.0 && self.0.abs() < 1e10 {
            write!(f, "{:.0}", self.0)
        } else {
            write!(f, "{}", self.0)
        }
    }
}

impl FromStr for Float {
    type Err = NumberError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.trim()
            .parse::<f64>()
            .map(Self::new)
            .map_err(|_| NumberError::ParseError { input: s.to_string(), ty: "Float" })
    }
}

// Conversions
impl From<f32> for Float {
    #[inline]
    fn from(v: f32) -> Self {
        Self(v as f64)
    }
}
impl From<f64> for Float {
    #[inline]
    fn from(v: f64) -> Self {
        Self(v)
    }
}
impl From<i8> for Float {
    #[inline]
    fn from(v: i8) -> Self {
        Self(v as f64)
    }
}
impl From<i16> for Float {
    #[inline]
    fn from(v: i16) -> Self {
        Self(v as f64)
    }
}
impl From<i32> for Float {
    #[inline]
    fn from(v: i32) -> Self {
        Self(v as f64)
    }
}
impl From<i64> for Float {
    #[inline]
    fn from(v: i64) -> Self {
        Self(v as f64)
    }
}
impl From<u8> for Float {
    #[inline]
    fn from(v: u8) -> Self {
        Self(v as f64)
    }
}
impl From<u16> for Float {
    #[inline]
    fn from(v: u16) -> Self {
        Self(v as f64)
    }
}
impl From<u32> for Float {
    #[inline]
    fn from(v: u32) -> Self {
        Self(v as f64)
    }
}
impl From<u64> for Float {
    #[inline]
    fn from(v: u64) -> Self {
        Self(v as f64)
    }
}

impl From<Integer> for Float {
    #[inline]
    fn from(v: Integer) -> Self {
        Self(v.get() as f64)
    }
}

impl From<Float> for f64 {
    #[inline]
    fn from(v: Float) -> Self {
        v.0
    }
}

impl From<Float> for f32 {
    #[inline]
    fn from(v: Float) -> Self {
        v.0 as f32
    }
}

// Arithmetic operators
impl Add for Float {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Float {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Mul for Float {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self::Output {
        Self(self.0 * rhs.0)
    }
}

impl Div for Float {
    type Output = Self;
    #[inline]
    fn div(self, rhs: Self) -> Self::Output {
        Self(self.0 / rhs.0)
    }
}

impl Rem for Float {
    type Output = Self;
    #[inline]
    fn rem(self, rhs: Self) -> Self::Output {
        Self(self.0 % rhs.0)
    }
}

impl Neg for Float {
    type Output = Self;
    #[inline]
    fn neg(self) -> Self::Output {
        Self(-self.0)
    }
}

// Assignment operators
impl AddAssign for Float {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}
impl SubAssign for Float {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}
impl MulAssign for Float {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        self.0 *= rhs.0;
    }
}
impl DivAssign for Float {
    #[inline]
    fn div_assign(&mut self, rhs: Self) {
        self.0 /= rhs.0;
    }
}
impl RemAssign for Float {
    #[inline]
    fn rem_assign(&mut self, rhs: Self) {
        self.0 %= rhs.0;
    }
}

// Comparison
impl PartialEq for Float {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl PartialOrd for Float {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Trait Implementations for Number
// ══════════════════════════════════════════════════════════════════════════════

impl Default for Number {
    #[inline]
    fn default() -> Self {
        Self::ZERO
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Int(i) => write!(f, "{}", i),
            Self::Float(fl) => write!(f, "{}", fl),
            #[cfg(feature = "decimal")]
            Self::Decimal(d) => write!(f, "{}", d),
        }
    }
}

impl FromStr for Number {
    type Err = NumberError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();

        // Try integer first if no decimal point
        if !trimmed.contains('.') && !trimmed.contains('e') && !trimmed.contains('E') {
            if let Ok(i) = trimmed.parse::<i64>() {
                return Ok(Self::Int(Integer::new(i)));
            }
        }

        // Try float
        if let Ok(f) = trimmed.parse::<f64>() {
            Ok(Self::Float(Float::new(f)))
        } else {
            Err(NumberError::ParseError { input: s.to_string(), ty: "Number" })
        }
    }
}

// Conversions
impl From<i8> for Number {
    #[inline]
    fn from(v: i8) -> Self {
        Self::Int(Integer::from(v))
    }
}
impl From<i16> for Number {
    #[inline]
    fn from(v: i16) -> Self {
        Self::Int(Integer::from(v))
    }
}
impl From<i32> for Number {
    #[inline]
    fn from(v: i32) -> Self {
        Self::Int(Integer::from(v))
    }
}
impl From<i64> for Number {
    #[inline]
    fn from(v: i64) -> Self {
        Self::Int(Integer::from(v))
    }
}
impl From<u8> for Number {
    #[inline]
    fn from(v: u8) -> Self {
        Self::Int(Integer::from(v))
    }
}
impl From<u16> for Number {
    #[inline]
    fn from(v: u16) -> Self {
        Self::Int(Integer::from(v))
    }
}
impl From<u32> for Number {
    #[inline]
    fn from(v: u32) -> Self {
        Self::Int(Integer::from(v))
    }
}

impl From<f32> for Number {
    #[inline]
    fn from(v: f32) -> Self {
        Self::Float(Float::from(v))
    }
}
impl From<f64> for Number {
    #[inline]
    fn from(v: f64) -> Self {
        Self::Float(Float::from(v))
    }
}

impl From<Integer> for Number {
    #[inline]
    fn from(v: Integer) -> Self {
        Self::Int(v)
    }
}
impl From<Float> for Number {
    #[inline]
    fn from(v: Float) -> Self {
        Self::Float(v)
    }
}

// Arithmetic operators
impl Add for Number {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Int(a), Self::Int(b)) => Self::Int(a + b),
            _ => Self::Float(Float::new(self.to_f64() + rhs.to_f64())),
        }
    }
}

impl Sub for Number {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Int(a), Self::Int(b)) => Self::Int(a - b),
            _ => Self::Float(Float::new(self.to_f64() - rhs.to_f64())),
        }
    }
}

impl Mul for Number {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Int(a), Self::Int(b)) => Self::Int(a * b),
            _ => Self::Float(Float::new(self.to_f64() * rhs.to_f64())),
        }
    }
}

impl Div for Number {
    type Output = Self;

    fn div(self, rhs: Self) -> Self::Output {
        // Division always returns float for precision
        Self::Float(Float::new(self.to_f64() / rhs.to_f64()))
    }
}

impl Rem for Number {
    type Output = Self;

    fn rem(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (Self::Int(a), Self::Int(b)) => Self::Int(a % b),
            _ => Self::Float(Float::new(self.to_f64() % rhs.to_f64())),
        }
    }
}

impl Neg for Number {
    type Output = Self;

    fn neg(self) -> Self::Output {
        match self {
            Self::Int(i) => Self::Int(-i),
            Self::Float(f) => Self::Float(-f),
            #[cfg(feature = "decimal")]
            Self::Decimal(d) => Self::Decimal(-d),
        }
    }
}

// Assignment operators
impl AddAssign for Number {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}
impl SubAssign for Number {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}
impl MulAssign for Number {
    #[inline]
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}
impl DivAssign for Number {
    #[inline]
    fn div_assign(&mut self, rhs: Self) {
        *self = *self / rhs;
    }
}
impl RemAssign for Number {
    #[inline]
    fn rem_assign(&mut self, rhs: Self) {
        *self = *self % rhs;
    }
}

// Comparison
impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => a == b,
            _ => {
                // Use approximate equality for float comparisons
                let a = self.to_f64();
                let b = other.to_f64();
                (a - b).abs() < f64::EPSILON * 10.0
            },
        }
    }
}

impl PartialOrd for Number {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match (self, other) {
            (Self::Int(a), Self::Int(b)) => Some(a.cmp(b)),
            _ => self.to_f64().partial_cmp(&other.to_f64()),
        }
    }
}

// Hash implementation
impl Hash for Number {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::Int(i) => {
                0u8.hash(state); // discriminant
                i.hash(state);
            },
            Self::Float(f) => {
                1u8.hash(state); // discriminant
                f.get().to_bits().hash(state);
            },
            #[cfg(feature = "decimal")]
            Self::Decimal(d) => {
                2u8.hash(state); // discriminant
                d.hash(state);
            },
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// JSON Support
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "serde")]
impl TryFrom<serde_json::Value> for Number {
    type Error = NumberError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(Self::Int(Integer::new(i)))
                } else if let Some(f) = n.as_f64() {
                    Ok(Self::Float(Float::new(f)))
                } else {
                    Err(NumberError::ParseError { input: n.to_string(), ty: "Number" })
                }
            },
            serde_json::Value::String(s) => s.parse(),
            serde_json::Value::Bool(b) => Ok(Self::Int(Integer::new(if b { 1 } else { 0 }))),
            serde_json::Value::Null => Ok(Self::Int(Integer::ZERO)),
            _ => Err(NumberError::JsonTypeMismatch {
                found: match value {
                    serde_json::Value::Array(_) => "array",
                    serde_json::Value::Object(_) => "object",
                    _ => "unknown",
                },
            }),
        }
    }
}

#[cfg(feature = "serde")]
impl From<Number> for serde_json::Value {
    fn from(num: Number) -> Self {
        match num {
            Number::Int(i) => serde_json::Value::Number(i.get().into()),
            Number::Float(f) => serde_json::Number::from_f64(f.get())
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            #[cfg(feature = "decimal")]
            Number::Decimal(d) => serde_json::Value::String(d.to_string()),
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Send + Sync
// ══════════════════════════════════════════════════════════════════════════════

// All number types are automatically Send + Sync

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_integer_basic() {
        let a = Integer::new(10);
        let b = Integer::new(20);

        assert_eq!(a + b, Integer::new(30));
        assert_eq!(b - a, Integer::new(10));
        assert_eq!(a * Integer::new(3), Integer::new(30));
        assert_eq!(b / a, Integer::new(2));
        assert_eq!(b % Integer::new(3), Integer::new(2));
    }

    #[test]
    fn test_integer_overflow() {
        let max = Integer::MAX;
        let one = Integer::ONE;

        // Saturating arithmetic
        assert_eq!(max + one, Integer::MAX);
        assert_eq!(Integer::MIN - one, Integer::MIN);

        // Checked arithmetic
        assert!(max.checked_add(one).is_err());
        assert!(Integer::MIN.checked_sub(one).is_err());
    }

    #[test]
    fn test_float_basic() {
        let a = Float::new(10.5);
        let b = Float::new(20.5);

        assert_eq!(a + b, Float::new(31.0));
        assert_eq!(b - a, Float::new(10.0));
        assert_eq!(a * Float::new(2.0), Float::new(21.0));
        assert_eq!(b / Float::new(2.0), Float::new(10.25));
    }

    #[test]
    fn test_float_approx_eq() {
        let a = Float::new(1.0);
        let b = Float::new(1.0 + f64::EPSILON);

        assert!(a.approx_eq_abs(b, f64::EPSILON * 2.0));
        assert!(a.approx_eq_rel(b, 0.0001));
        assert!(a.approx_eq_ulps(b, 2));
    }

    #[test]
    fn test_number_mixed() {
        let int = Number::from_i64(10);
        let float = Number::from_f64(5.5);

        // Mixed operations promote to float
        assert_eq!(int + float, Number::Float(Float::new(15.5)));
        assert_eq!(int - float, Number::Float(Float::new(4.5)));
        assert_eq!(int * float, Number::Float(Float::new(55.0)));

        // Division always returns float
        let int2 = Number::from_i64(4);
        assert_eq!(int / int2, Number::Float(Float::new(2.5)));
    }

    #[test]
    fn test_number_parsing() {
        assert_eq!("42".parse::<Number>().unwrap(), Number::from_i64(42));
        assert_eq!("3.14".parse::<Number>().unwrap(), Number::Float(Float::new(3.14)));
        assert_eq!("-100".parse::<Number>().unwrap(), Number::from_i64(-100));
        assert_eq!("1.5e2".parse::<Number>().unwrap(), Number::Float(Float::new(150.0)));
    }

    #[test]
    fn test_gcd_lcm() {
        let a = Integer::new(48);
        let b = Integer::new(18);

        assert_eq!(a.gcd(b), Integer::new(6));
        assert_eq!(a.lcm(b).unwrap(), Integer::new(144));
    }

    #[test]
    fn test_bit_operations() {
        let n = Integer::new(0b1010_1100);

        assert_eq!(n.count_ones(), 4);
        assert_eq!(n.count_zeros(), 60);
        assert_eq!(n.trailing_zeros(), 2);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_json_conversion() {
        use serde_json::json;

        let int = Number::from_i64(42);
        let float = Number::from_f64(3.14);

        assert_eq!(serde_json::Value::from(int), json!(42));
        assert_eq!(serde_json::Value::from(float), json!(3.14));

        assert_eq!(Number::try_from(json!(42)).unwrap(), int);
        assert_eq!(Number::try_from(json!(3.14)).unwrap(), float);
        assert_eq!(Number::try_from(json!(true)).unwrap(), Number::from_i64(1));
        assert_eq!(Number::try_from(json!(null)).unwrap(), Number::from_i64(0));
    }
}
