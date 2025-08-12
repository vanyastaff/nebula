use core::fmt;
use core::ops::{
    BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Deref, DerefMut, Not,
};
use core::str::FromStr;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use thiserror::Error;

// ══════════════════════════════════════════════════════════════════════════════
// Error Types
// ══════════════════════════════════════════════════════════════════════════════

/// Result type alias for BooleanValue operations
pub type BooleanResult<T> = Result<T, BooleanError>;

/// Rich, typed errors for BooleanValue operations
#[derive(Error, Debug, Clone, PartialEq,)]
pub enum BooleanError {
    #[error("Failed to parse '{input}' as boolean")]
    ParseError { input: String },

    #[error("Invalid numeric value {value} for boolean (expected 0 or 1)")]
    InvalidNumeric { value: i128 },

    #[error("Invalid float value {value} for boolean (expected 0.0 or 1.0)")]
    InvalidFloat { value: f64 },

    #[error("JSON type mismatch: expected bool/string/number, got {found}")]
    #[cfg(feature = "serde")]
    JsonTypeMismatch { found: &'static str },

    #[error("Invalid bit pattern: {bits:08b}")]
    InvalidBitPattern { bits: u8 },
}

// ══════════════════════════════════════════════════════════════════════════════
// BooleanValue
// ══════════════════════════════════════════════════════════════════════════════

/// A high-performance boolean value type with extended functionality
///
/// Features:
/// - Const-friendly operations
/// - Extended logical operations (NAND, NOR, XNOR, implies, etc.)
/// - Comprehensive parsing from various string formats
/// - Bit operations and patterns
/// - Collection operations (all, any, majority)
/// - Zero-cost abstractions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
#[repr(transparent)]
pub struct Boolean(bool);

impl Boolean {
    // ════════════════════════════════════════════════════════════════
    // Constants
    // ════════════════════════════════════════════════════════════════

    /// Constant true value
    pub const TRUE: Self = Self(true);

    /// Constant false value  
    pub const FALSE: Self = Self(false);

    // Bit pattern constants for advanced operations
    pub const BIT_ZERO: u8 = 0b00000000;
    pub const BIT_ONE: u8 = 0b00000001;
    pub const BIT_ALL: u8 = 0b11111111;

    // ════════════════════════════════════════════════════════════════
    // Constructors
    // ════════════════════════════════════════════════════════════════

    /// Creates a new BooleanValue (const-friendly)
    #[inline]
    #[must_use]
    pub const fn new(value: bool) -> Self {
        Self(value)
    }

    /// Creates a true value
    #[inline]
    #[must_use]
    pub const fn true_value() -> Self {
        Self::TRUE
    }

    /// Creates a false value
    #[inline]
    #[must_use]
    pub const fn false_value() -> Self {
        Self::FALSE
    }

    /// Creates from a bit (0 or 1)
    #[inline]
    pub const fn from_bit(bit: u8) -> BooleanResult<Self> {
        match bit {
            0 => Ok(Self::FALSE),
            1 => Ok(Self::TRUE),
            _ => Err(BooleanError::InvalidBitPattern { bits: bit }),
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Accessors
    // ════════════════════════════════════════════════════════════════

    /// Returns the underlying bool value
    #[inline]
    #[must_use]
    pub const fn value(&self) -> bool {
        self.0
    }

    /// Returns the underlying bool value (alias)
    #[inline]
    #[must_use]
    pub const fn get(&self) -> bool {
        self.0
    }

    /// Returns the underlying bool value (consuming)
    #[inline]
    #[must_use]
    pub const fn into_inner(self) -> bool {
        self.0
    }

    /// Checks if the value is true
    #[inline]
    #[must_use]
    pub const fn is_true(&self) -> bool {
        self.0
    }

    /// Checks if the value is false
    #[inline]
    #[must_use]
    pub const fn is_false(&self) -> bool {
        !self.0
    }

    // ════════════════════════════════════════════════════════════════
    // Basic Logical Operations
    // ════════════════════════════════════════════════════════════════

    /// Logical NOT
    #[inline]
    #[must_use]
    pub const fn not(&self) -> Self {
        Self(!self.0)
    }

    /// Logical AND
    #[inline]
    #[must_use]
    pub const fn and(&self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Logical OR
    #[inline]
    #[must_use]
    pub const fn or(&self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Logical XOR
    #[inline]
    #[must_use]
    pub const fn xor(&self, other: Self) -> Self {
        Self(self.0 ^ other.0)
    }

    // ════════════════════════════════════════════════════════════════
    // Extended Logical Operations
    // ════════════════════════════════════════════════════════════════

    /// Logical NAND (NOT AND)
    #[inline]
    #[must_use]
    pub const fn nand(&self, other: Self) -> Self {
        Self(!(self.0 & other.0))
    }

    /// Logical NOR (NOT OR)
    #[inline]
    #[must_use]
    pub const fn nor(&self, other: Self) -> Self {
        Self(!(self.0 | other.0))
    }

    /// Logical XNOR (equivalence)
    #[inline]
    #[must_use]
    pub const fn xnor(&self, other: Self) -> Self {
        Self(!(self.0 ^ other.0))
    }

    /// Logical implication (A → B ≡ ¬A ∨ B)
    #[inline]
    #[must_use]
    pub const fn implies(&self, other: Self) -> Self {
        Self(!self.0 | other.0)
    }

    /// Logical biconditional/equivalence (A ↔ B)
    #[inline]
    #[must_use]
    pub const fn iff(&self, other: Self) -> Self {
        Self(self.0 == other.0)
    }

    /// Material nonimplication (A ↛ B ≡ A ∧ ¬B)
    #[inline]
    #[must_use]
    pub const fn nimplies(&self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Converse implication (A ← B ≡ A ∨ ¬B)
    #[inline]
    #[must_use]
    pub const fn converse_implies(&self, other: Self) -> Self {
        Self(self.0 | !other.0)
    }

    // ════════════════════════════════════════════════════════════════
    // Conditional Operations
    // ════════════════════════════════════════════════════════════════

    /// Returns Some(true) if both are true, None otherwise
    #[inline]
    #[must_use]
    pub const fn and_then(&self, other: Self) -> Option<Self> {
        if self.0 & other.0 {
            Some(Self::TRUE)
        } else {
            None
        }
    }

    /// Returns Some(true) if either is true, None if both false
    #[inline]
    #[must_use]
    pub const fn or_else(&self, other: Self) -> Option<Self> {
        if self.0 | other.0 {
            Some(Self::TRUE)
        } else {
            None
        }
    }

    /// Ternary operator: returns first value if true, second if false
    #[inline]
    #[must_use]
    pub const fn select<T: Copy>(&self, if_true: T, if_false: T) -> T {
        if self.0 {
            if_true
        } else {
            if_false
        }
    }

    /// Returns Some(value) if self is true
    #[inline]
    #[must_use]
    pub fn then_some<T>(&self, value: T) -> Option<T> {
        self.0.then_some(value)
    }

    /// Executes closure if true
    #[inline]
    pub fn then<F, T>(&self, f: F) -> Option<T>
    where
        F: FnOnce() -> T,
    {
        self.0.then(f)
    }

    // ════════════════════════════════════════════════════════════════
    // String Representations
    // ════════════════════════════════════════════════════════════════

    /// Standard string representation
    #[inline]
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        if self.0 { "true" } else { "false" }
    }

    /// Uppercase representation
    #[inline]
    #[must_use]
    pub const fn as_upper(&self) -> &'static str {
        if self.0 { "TRUE" } else { "FALSE" }
    }

    /// Title case representation
    #[inline]
    #[must_use]
    pub const fn as_title(&self) -> &'static str {
        if self.0 { "True" } else { "False" }
    }

    /// Short form (T/F)
    #[inline]
    #[must_use]
    pub const fn as_short(&self) -> &'static str {
        if self.0 { "T" } else { "F" }
    }

    /// Numeric string (1/0)
    #[inline]
    #[must_use]
    pub const fn as_numeric_str(&self) -> &'static str {
        if self.0 { "1" } else { "0" }
    }

    /// Yes/No representation
    #[inline]
    #[must_use]
    pub const fn as_yes_no(&self) -> &'static str {
        if self.0 { "yes" } else { "no" }
    }

    /// On/Off representation
    #[inline]
    #[must_use]
    pub const fn as_on_off(&self) -> &'static str {
        if self.0 { "on" } else { "off" }
    }

    /// Enabled/Disabled representation
    #[inline]
    #[must_use]
    pub const fn as_enabled(&self) -> &'static str {
        if self.0 { "enabled" } else { "disabled" }
    }

    /// Active/Inactive representation
    #[inline]
    #[must_use]
    pub const fn as_active(&self) -> &'static str {
        if self.0 { "active" } else { "inactive" }
    }

    /// Pass/Fail representation
    #[inline]
    #[must_use]
    pub const fn as_pass_fail(&self) -> &'static str {
        if self.0 { "pass" } else { "fail" }
    }

    /// Success/Failure representation
    #[inline]
    #[must_use]
    pub const fn as_success(&self) -> &'static str {
        if self.0 { "success" } else { "failure" }
    }

    // ════════════════════════════════════════════════════════════════
    // Parsing
    // ════════════════════════════════════════════════════════════════

    /// Parse from string with comprehensive format support
    pub fn parse(s: &str) -> BooleanResult<Self> {
        // Fast path for common cases
        match s {
            "true" | "1" => return Ok(Self::TRUE),
            "false" | "0" => return Ok(Self::FALSE),
            _ => {}
        }

        // Case-insensitive parsing
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(BooleanError::ParseError {
                input: s.to_string(),
            });
        }

        // Use a static lookup table for efficiency
        static TRUE_VALUES: &[&str] = &[
            "true", "t", "1",
            "yes", "y",
            "on", "enable", "enabled",
            "active", "activated",
            "positive", "pos", "+",
            "ok", "okay", "accept", "accepted",
            "pass", "passed", "success", "successful",
            "high", "hi", "up",
            "set", "valid", "correct",
        ];

        static FALSE_VALUES: &[&str] = &[
            "false", "f", "0",
            "no", "n",
            "off", "disable", "disabled",
            "inactive", "deactivated",
            "negative", "neg", "-",
            "cancel", "cancelled", "reject", "rejected",
            "fail", "failed", "failure",
            "low", "lo", "down",
            "unset", "invalid", "incorrect",
        ];

        let lower = trimmed.to_lowercase();

        if TRUE_VALUES.iter().any(|&v| lower == v) {
            Ok(Self::TRUE)
        } else if FALSE_VALUES.iter().any(|&v| lower == v) {
            Ok(Self::FALSE)
        } else {
            Err(BooleanError::ParseError {
                input: s.to_string(),
            })
        }
    }

    /// Strict parsing (only "true" or "false")
    pub fn parse_strict(s: &str) -> BooleanResult<Self> {
        match s {
            "true" => Ok(Self::TRUE),
            "false" => Ok(Self::FALSE),
            _ => Err(BooleanError::ParseError {
                input: s.to_string(),
            }),
        }
    }

    /// Lenient parsing (alias for parse)
    #[inline]
    pub fn parse_lenient(s: &str) -> BooleanResult<Self> {
        Self::parse(s)
    }

    // ════════════════════════════════════════════════════════════════
    // Numeric Conversions
    // ════════════════════════════════════════════════════════════════

    /// Convert to i8
    #[inline]
    #[must_use]
    pub const fn as_i8(&self) -> i8 {
        if self.0 { 1 } else { 0 }
    }

    /// Convert to u8
    #[inline]
    #[must_use]
    pub const fn as_u8(&self) -> u8 {
        if self.0 { 1 } else { 0 }
    }

    /// Convert to i32
    #[inline]
    #[must_use]
    pub const fn as_i32(&self) -> i32 {
        if self.0 { 1 } else { 0 }
    }

    /// Convert to u32
    #[inline]
    #[must_use]
    pub const fn as_u32(&self) -> u32 {
        if self.0 { 1 } else { 0 }
    }

    /// Convert to i64
    #[inline]
    #[must_use]
    pub const fn as_i64(&self) -> i64 {
        if self.0 { 1 } else { 0 }
    }

    /// Convert to u64
    #[inline]
    #[must_use]
    pub const fn as_u64(&self) -> u64 {
        if self.0 { 1 } else { 0 }
    }

    /// Convert to isize
    #[inline]
    #[must_use]
    pub const fn as_isize(&self) -> isize {
        if self.0 { 1 } else { 0 }
    }

    /// Convert to usize
    #[inline]
    #[must_use]
    pub const fn as_usize(&self) -> usize {
        if self.0 { 1 } else { 0 }
    }

    /// Convert to f32
    #[inline]
    #[must_use]
    pub const fn as_f32(&self) -> f32 {
        if self.0 { 1.0 } else { 0.0 }
    }

    /// Convert to f64
    #[inline]
    #[must_use]
    pub const fn as_f64(&self) -> f64 {
        if self.0 { 1.0 } else { 0.0 }
    }

    /// Create from integer (loose: non-zero = true)
    #[inline]
    #[must_use]
    pub const fn from_int_loose(value: i128) -> Self {
        Self(value != 0)
    }

    /// Create from integer (strict: only 0 or 1)
    pub const fn from_int_strict(value: i128) -> BooleanResult<Self> {
        match value {
            0 => Ok(Self::FALSE),
            1 => Ok(Self::TRUE),
            _ => Err(BooleanError::InvalidNumeric { value }),
        }
    }

    /// Create from float (loose: non-zero and not NaN = true)
    #[inline]
    #[must_use]
    pub fn from_float_loose(value: f64) -> Self {
        Self(value != 0.0 && !value.is_nan())
    }

    /// Create from float (strict: only 0.0 or 1.0)
    pub fn from_float_strict(value: f64) -> BooleanResult<Self> {
        if value.is_nan() || value.is_infinite() {
            return Err(BooleanError::InvalidFloat { value });
        }

        // Use epsilon for float comparison
        const EPSILON: f64 = f64::EPSILON;

        if (value - 0.0).abs() < EPSILON {
            Ok(Self::FALSE)
        } else if (value - 1.0).abs() < EPSILON {
            Ok(Self::TRUE)
        } else {
            Err(BooleanError::InvalidFloat { value })
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Bit Operations
    // ════════════════════════════════════════════════════════════════

    /// Convert to bit pattern
    #[inline]
    #[must_use]
    pub const fn as_bit(&self) -> u8 {
        if self.0 { Self::BIT_ONE } else { Self::BIT_ZERO }
    }

    /// Convert to bit mask (all 1s or all 0s)
    #[inline]
    #[must_use]
    pub const fn as_mask(&self) -> u8 {
        if self.0 { Self::BIT_ALL } else { Self::BIT_ZERO }
    }

    /// Extract bit at position from byte
    #[inline]
    #[must_use]
    pub const fn from_bit_at(byte: u8, position: u8) -> Self {
        if position >= 8 {
            Self::FALSE
        } else {
            Self((byte >> position) & 1 != 0)
        }
    }

    // ════════════════════════════════════════════════════════════════
    // Mutation Operations
    // ════════════════════════════════════════════════════════════════

    /// Toggle the value in place
    #[inline]
    pub fn toggle(&mut self) {
        self.0 = !self.0;
    }

    /// Set to true
    #[inline]
    pub fn set_true(&mut self) {
        self.0 = true;
    }

    /// Set to false
    #[inline]
    pub fn set_false(&mut self) {
        self.0 = false;
    }

    /// Set to a specific value
    #[inline]
    pub fn set(&mut self, value: bool) {
        self.0 = value;
    }

    /// Swap values with another BooleanValue
    #[inline]
    pub fn swap(&mut self, other: &mut Self) {
        core::mem::swap(&mut self.0, &mut other.0);
    }

    /// Returns a toggled copy
    #[inline]
    #[must_use]
    pub const fn toggled(&self) -> Self {
        Self(!self.0)
    }

    // ════════════════════════════════════════════════════════════════
    // Collection Operations
    // ════════════════════════════════════════════════════════════════

    /// Returns true if all values are true
    #[inline]
    #[must_use]
    pub fn all(values: &[Self]) -> Self {
        Self(values.iter().all(|b| b.0))
    }

    /// Returns true if any value is true
    #[inline]
    #[must_use]
    pub fn any(values: &[Self]) -> Self {
        Self(values.iter().any(|b| b.0))
    }

    /// Returns true if no values are true
    #[inline]
    #[must_use]
    pub fn none(values: &[Self]) -> Self {
        Self(!values.iter().any(|b| b.0))
    }

    /// Count true values
    #[inline]
    #[must_use]
    pub fn count_true(values: &[Self]) -> usize {
        values.iter().filter(|b| b.0).count()
    }

    /// Count false values
    #[inline]
    #[must_use]
    pub fn count_false(values: &[Self]) -> usize {
        values.len() - Self::count_true(values)
    }

    /// Get majority value (None if tie)
    pub fn majority(values: &[Self]) -> Option<Self> {
        if values.is_empty() {
            return None;
        }

        let true_count = Self::count_true(values);
        let half = values.len() / 2;

        if true_count > half {
            Some(Self::TRUE)
        } else if true_count < values.len() - half {
            Some(Self::FALSE)
        } else {
            None // Tie
        }
    }

    /// Check if exactly n values are true
    #[inline]
    #[must_use]
    pub fn exactly_n(values: &[Self], n: usize) -> Self {
        Self(Self::count_true(values) == n)
    }

    /// Check if at least n values are true
    #[inline]
    #[must_use]
    pub fn at_least_n(values: &[Self], n: usize) -> Self {
        Self(Self::count_true(values) >= n)
    }

    /// Check if at most n values are true
    #[inline]
    #[must_use]
    pub fn at_most_n(values: &[Self], n: usize) -> Self {
        Self(Self::count_true(values) <= n)
    }

    /// Performs parity check (true if odd number of trues)
    #[inline]
    #[must_use]
    pub fn parity(values: &[Self]) -> Self {
        Self(Self::count_true(values) % 2 == 1)
    }

    // ════════════════════════════════════════════════════════════════
    // Advanced Operations
    // ════════════════════════════════════════════════════════════════

    /// Three-valued logic: returns None for indeterminate
    pub fn three_valued_and(&self, other: Option<Self>) -> Option<Self> {
        match (self.0, other) {
            (false, _) => Some(Self::FALSE),
            (true, Some(b)) => Some(b),
            (true, None) => None,
        }
    }

    /// Three-valued logic OR
    pub fn three_valued_or(&self, other: Option<Self>) -> Option<Self> {
        match (self.0, other) {
            (true, _) => Some(Self::TRUE),
            (false, Some(b)) => Some(b),
            (false, None) => None,
        }
    }

    /// Fuzzy logic operations (returns probability)
    #[inline]
    #[must_use]
    pub fn as_probability(&self) -> f64 {
        if self.0 { 1.0 } else { 0.0 }
    }

    /// Create from probability (>= 0.5 is true)
    #[inline]
    #[must_use]
    pub fn from_probability(p: f64) -> Self {
        Self(p >= 0.5)
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Core Trait Implementations
// ══════════════════════════════════════════════════════════════════════════════

impl Default for Boolean {
    #[inline]
    fn default() -> Self {
        Self::FALSE
    }
}

impl fmt::Display for Boolean {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Binary for Boolean {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Binary::fmt(&self.as_u8(), f)
    }
}

impl fmt::Octal for Boolean {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Octal::fmt(&self.as_u8(), f)
    }
}

impl fmt::LowerHex for Boolean {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.as_u8(), f)
    }
}

impl fmt::UpperHex for Boolean {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::UpperHex::fmt(&self.as_u8(), f)
    }
}

impl FromStr for Boolean {
    type Err = BooleanError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Deref Implementations
// ══════════════════════════════════════════════════════════════════════════════

impl Deref for Boolean {
    type Target = bool;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Boolean {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl AsRef<bool> for Boolean {
    #[inline]
    fn as_ref(&self) -> &bool {
        &self.0
    }
}

impl AsMut<bool> for Boolean {
    #[inline]
    fn as_mut(&mut self) -> &mut bool {
        &mut self.0
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Logical Operator Implementations
// ══════════════════════════════════════════════════════════════════════════════

impl Not for Boolean {
    type Output = Self;

    #[inline]
    fn not(self) -> Self::Output {
        Self::not(&self)
    }
}

impl BitAnd for Boolean {
    type Output = Self;

    #[inline]
    fn bitand(self, rhs: Self) -> Self::Output {
        self.and(rhs)
    }
}

impl BitOr for Boolean {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self::Output {
        self.or(rhs)
    }
}

impl BitXor for Boolean {
    type Output = Self;

    #[inline]
    fn bitxor(self, rhs: Self) -> Self::Output {
        self.xor(rhs)
    }
}

impl BitAndAssign for Boolean {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}

impl BitOrAssign for Boolean {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitXorAssign for Boolean {
    #[inline]
    fn bitxor_assign(&mut self, rhs: Self) {
        self.0 ^= rhs.0;
    }
}

// Mixed operations with bool
impl BitAnd<bool> for Boolean {
    type Output = Self;

    #[inline]
    fn bitand(self, rhs: bool) -> Self::Output {
        self.and(Self(rhs))
    }
}

impl BitOr<bool> for Boolean {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: bool) -> Self::Output {
        self.or(Self(rhs))
    }
}

impl BitXor<bool> for Boolean {
    type Output = Self;

    #[inline]
    fn bitxor(self, rhs: bool) -> Self::Output {
        self.xor(Self(rhs))
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Conversion Traits
// ══════════════════════════════════════════════════════════════════════════════

impl From<bool> for Boolean {
    #[inline]
    fn from(value: bool) -> Self {
        Self(value)
    }
}

impl From<Boolean> for bool {
    #[inline]
    fn from(value: Boolean) -> Self {
        value.0
    }
}

// Numeric conversions
macro_rules! impl_from_int {
    ($($t:ty),*) => {
        $(
            impl From<$t> for Boolean {
                #[inline]
                fn from(value: $t) -> Self {
                    Self::from_int_loose(value as i128)
                }
            }
        )*
    };
}

impl_from_int!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

impl From<f32> for Boolean {
    #[inline]
    fn from(value: f32) -> Self {
        Self::from_float_loose(value as f64)
    }
}

impl From<f64> for Boolean {
    #[inline]
    fn from(value: f64) -> Self {
        Self::from_float_loose(value)
    }
}


// Convert to numeric types
impl From<Boolean> for i32 {
    #[inline]
    fn from(value: Boolean) -> Self {
        value.as_i32()
    }
}

impl From<Boolean> for u32 {
    #[inline]
    fn from(value: Boolean) -> Self {
        value.as_u32()
    }
}

impl From<Boolean> for f64 {
    #[inline]
    fn from(value: Boolean) -> Self {
        value.as_f64()
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Comparison Traits
// ══════════════════════════════════════════════════════════════════════════════

impl PartialEq<bool> for Boolean {
    #[inline]
    fn eq(&self, other: &bool) -> bool {
        self.0 == *other
    }
}

impl PartialEq<Boolean> for bool {
    #[inline]
    fn eq(&self, other: &Boolean) -> bool {
        *self == other.0
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Iterator Support
// ══════════════════════════════════════════════════════════════════════════════

impl FromIterator<Boolean> for Boolean {
    /// Performs AND operation on all values
    fn from_iter<T: IntoIterator<Item =Boolean>>(iter: T) -> Self {
        Self(iter.into_iter().all(|b| b.0))
    }
}

impl FromIterator<bool> for Boolean {
    /// Performs AND operation on all values
    fn from_iter<T: IntoIterator<Item = bool>>(iter: T) -> Self {
        Self(iter.into_iter().all(|b| b))
    }
}


// ══════════════════════════════════════════════════════════════════════════════
// JSON Support
// ══════════════════════════════════════════════════════════════════════════════

#[cfg(feature = "serde")]
impl From<Boolean> for serde_json::Value {
    #[inline]
    fn from(value: Boolean) -> Self {
        serde_json::Value::Bool(value.0)
    }
}

#[cfg(feature = "serde")]
impl TryFrom<serde_json::Value> for Boolean {
    type Error = BooleanError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::Bool(b) => Ok(Self(b)),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Self::from_int_strict(i as i128)
                } else if let Some(f) = n.as_f64() {
                    Self::from_float_strict(f)
                } else {
                    Err(BooleanError::JsonTypeMismatch { found: "number" })
                }
            }
            serde_json::Value::String(s) => Self::parse(&s),
            serde_json::Value::Null => Ok(Self::FALSE),
            serde_json::Value::Array(_) => Err(BooleanError::JsonTypeMismatch { found: "array" }),
            serde_json::Value::Object(_) => Err(BooleanError::JsonTypeMismatch { found: "object" }),
        }
    }
}

// ══════════════════════════════════════════════════════════════════════════════
// Send + Sync
// ══════════════════════════════════════════════════════════════════════════════

// BooleanValue is automatically Send + Sync as bool is Send + Sync

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert!(Boolean::TRUE.is_true());
        assert!(Boolean::FALSE.is_false());

        const T: Boolean = Boolean::new(true);
        const F: Boolean = Boolean::new(false);
        assert!(T.is_true());
        assert!(F.is_false());
    }

    #[test]
    fn test_logical_operations() {
        let t = Boolean::TRUE;
        let f = Boolean::FALSE;

        // Basic operations
        assert_eq!(t.not(), f);
        assert_eq!(t.and(f), f);
        assert_eq!(t.or(f), t);
        assert_eq!(t.xor(f), t);

        // Extended operations
        assert_eq!(t.nand(f), t);
        assert_eq!(t.nor(f), f);
        assert_eq!(t.xnor(f), f);
        assert_eq!(t.implies(f), f);
        assert_eq!(f.implies(t), t);
        assert_eq!(t.iff(t), t);
        assert_eq!(t.iff(f), f);
    }

    #[test]
    fn test_parsing() {
        // Standard cases
        assert_eq!(Boolean::parse("true").unwrap(), Boolean::TRUE);
        assert_eq!(Boolean::parse("false").unwrap(), Boolean::FALSE);

        // Case insensitive
        assert_eq!(Boolean::parse("TRUE").unwrap(), Boolean::TRUE);
        assert_eq!(Boolean::parse("False").unwrap(), Boolean::FALSE);

        // Numeric
        assert_eq!(Boolean::parse("1").unwrap(), Boolean::TRUE);
        assert_eq!(Boolean::parse("0").unwrap(), Boolean::FALSE);

        // Extended formats
        assert_eq!(Boolean::parse("yes").unwrap(), Boolean::TRUE);
        assert_eq!(Boolean::parse("no").unwrap(), Boolean::FALSE);
        assert_eq!(Boolean::parse("on").unwrap(), Boolean::TRUE);
        assert_eq!(Boolean::parse("off").unwrap(), Boolean::FALSE);

        // Invalid
        assert!(Boolean::parse("maybe").is_err());
        assert!(Boolean::parse("").is_err());
    }

    #[test]
    fn test_string_representations() {
        let t = Boolean::TRUE;
        let f = Boolean::FALSE;

        assert_eq!(t.as_str(), "true");
        assert_eq!(f.as_str(), "false");
        assert_eq!(t.as_upper(), "TRUE");
        assert_eq!(t.as_title(), "True");
        assert_eq!(t.as_short(), "T");
        assert_eq!(t.as_numeric_str(), "1");
        assert_eq!(t.as_yes_no(), "yes");
        assert_eq!(t.as_on_off(), "on");
        assert_eq!(t.as_enabled(), "enabled");
        assert_eq!(t.as_active(), "active");
        assert_eq!(t.as_pass_fail(), "pass");
        assert_eq!(t.as_success(), "success");
    }

    #[test]
    fn test_numeric_conversions() {
        let t = Boolean::TRUE;
        let f = Boolean::FALSE;

        assert_eq!(t.as_i32(), 1);
        assert_eq!(f.as_i32(), 0);
        assert_eq!(t.as_f64(), 1.0);
        assert_eq!(f.as_f64(), 0.0);

        assert_eq!(Boolean::from_int_loose(42), t);
        assert_eq!(Boolean::from_int_loose(0), f);
        assert_eq!(Boolean::from_float_loose(3.14), t);
        assert_eq!(Boolean::from_float_loose(0.0), f);

        assert!(Boolean::from_int_strict(1).is_ok());
        assert!(Boolean::from_int_strict(2).is_err());
        assert!(Boolean::from_float_strict(1.0).is_ok());
        assert!(Boolean::from_float_strict(0.5).is_err());
    }

    #[test]
    fn test_bit_operations() {
        let t = Boolean::TRUE;
        let f = Boolean::FALSE;

        assert_eq!(t.as_bit(), 0b00000001);
        assert_eq!(f.as_bit(), 0b00000000);
        assert_eq!(t.as_mask(), 0b11111111);
        assert_eq!(f.as_mask(), 0b00000000);

        assert_eq!(Boolean::from_bit_at(0b00001000, 3), t);
        assert_eq!(Boolean::from_bit_at(0b00001000, 2), f);
    }

    #[test]
    fn test_collection_operations() {
        let values = vec![
            Boolean::TRUE,
            Boolean::FALSE,
            Boolean::TRUE,
            Boolean::TRUE,
        ];

        assert_eq!(Boolean::all(&values), Boolean::FALSE);
        assert_eq!(Boolean::any(&values), Boolean::TRUE);
        assert_eq!(Boolean::none(&values), Boolean::FALSE);
        assert_eq!(Boolean::count_true(&values), 3);
        assert_eq!(Boolean::count_false(&values), 1);
        assert_eq!(Boolean::majority(&values), Some(Boolean::TRUE));
        assert_eq!(Boolean::exactly_n(&values, 3), Boolean::TRUE);
        assert_eq!(Boolean::at_least_n(&values, 2), Boolean::TRUE);
        assert_eq!(Boolean::at_most_n(&values, 2), Boolean::FALSE);
        assert_eq!(Boolean::parity(&values), Boolean::TRUE);
    }

    #[test]
    fn test_three_valued_logic() {
        let t = Boolean::TRUE;
        let f = Boolean::FALSE;

        assert_eq!(t.three_valued_and(Some(t)), Some(t));
        assert_eq!(t.three_valued_and(Some(f)), Some(f));
        assert_eq!(t.three_valued_and(None), None);
        assert_eq!(f.three_valued_and(None), Some(f));

        assert_eq!(t.three_valued_or(Some(f)), Some(t));
        assert_eq!(f.three_valued_or(Some(f)), Some(f));
        assert_eq!(t.three_valued_or(None), Some(t));
        assert_eq!(f.three_valued_or(None), None);
    }

    #[test]
    fn test_operators() {
        let mut a = Boolean::TRUE;
        let b = Boolean::FALSE;

        assert_eq!(a & b, Boolean::FALSE);
        assert_eq!(a | b, Boolean::TRUE);
        assert_eq!(a ^ b, Boolean::TRUE);
        assert_eq!(!a, Boolean::FALSE);

        a &= b;
        assert_eq!(a, Boolean::FALSE);

        a |= Boolean::TRUE;
        assert_eq!(a, Boolean::TRUE);

        a ^= Boolean::TRUE;
        assert_eq!(a, Boolean::FALSE);
    }

    #[test]
    fn test_display_formats() {
        let t = Boolean::TRUE;

        assert_eq!(format!("{}", t), "true");
        assert_eq!(format!("{:b}", t), "1");
        assert_eq!(format!("{:o}", t), "1");
        assert_eq!(format!("{:x}", t), "1");
        assert_eq!(format!("{:X}", t), "1");
    }

    #[cfg(feature = "serde")]
    #[test]
    fn test_json_conversion() {
        use serde_json::json;

        let t = Boolean::TRUE;
        let f = Boolean::FALSE;

        // To JSON
        assert_eq!(serde_json::Value::from(t), json!(true));
        assert_eq!(serde_json::Value::from(f), json!(false));

        // From JSON
        assert_eq!(Boolean::try_from(json!(true)).unwrap(), t);
        assert_eq!(Boolean::try_from(json!(false)).unwrap(), f);
        assert_eq!(Boolean::try_from(json!(1)).unwrap(), t);
        assert_eq!(Boolean::try_from(json!(0)).unwrap(), f);
        assert_eq!(Boolean::try_from(json!("yes")).unwrap(), t);
        assert_eq!(Boolean::try_from(json!(null)).unwrap(), f);
    }
}