use std::cmp::Ordering;
use std::fmt;

/// IEEE 754 double-precision floating point number
///
/// **IMPORTANT**: This type does NOT implement `Eq` or `Hash` because NaN != NaN.
/// Use `total_cmp()` for ordering that includes NaN, or use `HashableFloat` wrapper for HashMap.
#[derive(Debug, Clone, Copy)]
pub struct Float(f64);

impl Float {
    /// Create a new float
    pub const fn new(value: f64) -> Self {
        Self(value)
    }

    /// Get the inner value
    pub const fn value(&self) -> f64 {
        self.0
    }

    /// Check if this is NaN
    pub fn is_nan(&self) -> bool {
        self.0.is_nan()
    }

    /// Check if this is infinite
    pub fn is_infinite(&self) -> bool {
        self.0.is_infinite()
    }

    /// Check if this is finite (not NaN or infinite)
    pub fn is_finite(&self) -> bool {
        self.0.is_finite()
    }

    /// Check if this is positive infinity
    pub fn is_positive_infinity(&self) -> bool {
        self.0.is_infinite() && self.0.is_sign_positive()
    }

    /// Check if this is negative infinity
    pub fn is_negative_infinity(&self) -> bool {
        self.0.is_infinite() && self.0.is_sign_negative()
    }

    /// Total ordering comparison that includes NaN
    ///
    /// Order: -Infinity < finite < +Infinity < NaN
    ///
    /// This is the IEEE 754-2008 "totalOrder" predicate.
    pub fn total_cmp(&self, other: &Self) -> Ordering {
        self.0.total_cmp(&other.0)
    }

    /// Get the bit representation (for hashing)
    pub fn to_bits(&self) -> u64 {
        self.0.to_bits()
    }

    /// Create from bit representation
    pub fn from_bits(bits: u64) -> Self {
        Self(f64::from_bits(bits))
    }

    /// Absolute value
    pub fn abs(&self) -> Self {
        Self(self.0.abs())
    }

    /// Floor function
    pub fn floor(&self) -> Self {
        Self(self.0.floor())
    }

    /// Ceiling function
    pub fn ceil(&self) -> Self {
        Self(self.0.ceil())
    }

    /// Round to nearest integer
    pub fn round(&self) -> Self {
        Self(self.0.round())
    }

    /// Truncate decimal part
    pub fn trunc(&self) -> Self {
        Self(self.0.trunc())
    }
}

// PartialEq: NaN != NaN (IEEE 754 standard)
impl PartialEq for Float {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

// NO Eq implementation! This is intentional.
// Float cannot implement Eq because NaN != NaN violates Eq requirements.

impl PartialOrd for Float {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl fmt::Display for Float {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_nan() {
            write!(f, "NaN")
        } else if self.is_positive_infinity() {
            write!(f, "+Infinity")
        } else if self.is_negative_infinity() {
            write!(f, "-Infinity")
        } else {
            write!(f, "{}", self.0)
        }
    }
}

// Conversions
impl From<f32> for Float {
    fn from(v: f32) -> Self {
        Self(v as f64)
    }
}

impl From<f64> for Float {
    fn from(v: f64) -> Self {
        Self(v)
    }
}

// Arithmetic operations
impl std::ops::Add for Float {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl std::ops::Sub for Float {
    type Output = Self;

    fn sub(self, other: Self) -> Self {
        Self(self.0 - other.0)
    }
}

impl std::ops::Mul for Float {
    type Output = Self;

    fn mul(self, other: Self) -> Self {
        Self(self.0 * other.0)
    }
}

impl std::ops::Div for Float {
    type Output = Self;

    fn div(self, other: Self) -> Self {
        Self(self.0 / other.0)
    }
}

impl std::ops::Neg for Float {
    type Output = Self;

    fn neg(self) -> Self {
        Self(-self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nan_not_equal() {
        let nan1 = Float::new(f64::NAN);
        let nan2 = Float::new(f64::NAN);

        // NaN != NaN (IEEE 754 standard)
        assert_ne!(nan1, nan2);
        assert!(nan1.is_nan());
        assert!(nan2.is_nan());
    }

    #[test]
    fn test_total_cmp() {
        let neg_inf = Float::new(f64::NEG_INFINITY);
        let zero = Float::new(0.0);
        let pos_inf = Float::new(f64::INFINITY);
        let nan = Float::new(f64::NAN);

        assert_eq!(neg_inf.total_cmp(&zero), Ordering::Less);
        assert_eq!(zero.total_cmp(&pos_inf), Ordering::Less);
        assert_eq!(pos_inf.total_cmp(&nan), Ordering::Less);

        // NaN == NaN in total_cmp
        assert_eq!(nan.total_cmp(&Float::new(f64::NAN)), Ordering::Equal);
    }

    #[test]
    fn test_display() {
        assert_eq!(Float::new(3.14).to_string(), "3.14");
        assert_eq!(Float::new(f64::NAN).to_string(), "NaN");
        assert_eq!(Float::new(f64::INFINITY).to_string(), "+Infinity");
        assert_eq!(Float::new(f64::NEG_INFINITY).to_string(), "-Infinity");
    }

    #[test]
    fn test_special_values() {
        let nan = Float::new(f64::NAN);
        let inf = Float::new(f64::INFINITY);
        let neg_inf = Float::new(f64::NEG_INFINITY);
        let normal = Float::new(3.14);

        assert!(nan.is_nan());
        assert!(!nan.is_finite());

        assert!(inf.is_infinite());
        assert!(inf.is_positive_infinity());
        assert!(!inf.is_finite());

        assert!(neg_inf.is_infinite());
        assert!(neg_inf.is_negative_infinity());

        assert!(normal.is_finite());
        assert!(!normal.is_nan());
        assert!(!normal.is_infinite());
    }

    #[test]
    fn test_arithmetic() {
        let a = Float::new(5.0);
        let b = Float::new(3.0);

        assert_eq!((a + b).value(), 8.0);
        assert_eq!((a - b).value(), 2.0);
        assert_eq!((a * b).value(), 15.0);
        assert_eq!((a / b).value(), 5.0 / 3.0);
        assert_eq!((-a).value(), -5.0);
    }

    #[test]
    fn test_rounding() {
        let f = Float::new(3.7);
        assert_eq!(f.floor().value(), 3.0);
        assert_eq!(f.ceil().value(), 4.0);
        assert_eq!(f.round().value(), 4.0);
        assert_eq!(f.trunc().value(), 3.0);
    }

    // This should NOT compile if uncommented (Float doesn't implement Eq/Hash)
    // #[test]
    // fn test_no_eq() {
    //     use std::collections::HashMap;
    //     let mut map = HashMap::new();
    //     map.insert(Float::new(3.14), "value"); // ERROR: Float doesn't implement Eq + Hash
    // }
}