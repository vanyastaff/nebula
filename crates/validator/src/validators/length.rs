//! String length validators
//!
//! This module provides validators for checking string length constraints.
//! By default, length is measured in Unicode scalar values (chars).
//! Use the `.bytes()` constructor for byte-length counting when performance
//! is critical and the input is known to be ASCII.

use crate::foundation::{Validate, ValidationError};

// ============================================================================
// LENGTH MODE
// ============================================================================

/// How to count string length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LengthMode {
    /// Count bytes (fastest, ASCII-only correct).
    Bytes,
    /// Count Unicode scalar values (correct for all text).
    #[default]
    Chars,
}

impl LengthMode {
    /// Measures the length of a string according to this mode.
    #[inline]
    fn measure(self, input: &str) -> usize {
        match self {
            LengthMode::Bytes => input.len(),
            LengthMode::Chars => input.chars().count(),
        }
    }
}

// ============================================================================
// NOT EMPTY
// ============================================================================

crate::validator! {
    /// Validates that a string is not empty.
    ///
    /// This is equivalent to `MinLength::new(1)` but more semantic.
    pub NotEmpty for str;
    rule(input) { !input.is_empty() }
    error(input) { ValidationError::new("not_empty", "String must not be empty") }
    fn not_empty();
}

// ============================================================================
// MIN LENGTH
// ============================================================================

crate::validator! {
    /// Validates that a string has at least a minimum length.
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub MinLength { min: usize, mode: LengthMode } for str;
    rule(self, input) { self.mode.measure(input) >= self.min }
    error(self, input) { ValidationError::min_length("", self.min, self.mode.measure(input)) }
    new(min: usize) { Self { min, mode: LengthMode::Chars } }
    fn min_length(min: usize);
}

impl MinLength {
    /// Creates a minimum length validator that counts bytes.
    #[must_use]
    pub fn bytes(min: usize) -> Self {
        Self {
            min,
            mode: LengthMode::Bytes,
        }
    }
}

// ============================================================================
// MAX LENGTH
// ============================================================================

crate::validator! {
    /// Validates that a string does not exceed a maximum length.
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub MaxLength { max: usize, mode: LengthMode } for str;
    rule(self, input) { self.mode.measure(input) <= self.max }
    error(self, input) { ValidationError::max_length("", self.max, self.mode.measure(input)) }
    new(max: usize) { Self { max, mode: LengthMode::Chars } }
    fn max_length(max: usize);
}

impl MaxLength {
    /// Creates a maximum length validator that counts bytes.
    #[must_use]
    pub fn bytes(max: usize) -> Self {
        Self {
            max,
            mode: LengthMode::Bytes,
        }
    }
}

// ============================================================================
// EXACT LENGTH
// ============================================================================

crate::validator! {
    /// Validates that a string has an exact length.
    #[derive(Copy, PartialEq, Eq, Hash)]
    pub ExactLength { length: usize, mode: LengthMode } for str;
    rule(self, input) { self.mode.measure(input) == self.length }
    error(self, input) {
        ValidationError::new(
            "exact_length",
            format!("String must be exactly {} characters", self.length),
        )
        .with_param("expected", self.length.to_string())
        .with_param("actual", self.mode.measure(input).to_string())
    }
    new(length: usize) { Self { length, mode: LengthMode::Chars } }
    fn exact_length(length: usize);
}

impl ExactLength {
    /// Creates an exact length validator that counts bytes.
    #[must_use]
    pub fn bytes(length: usize) -> Self {
        Self {
            length,
            mode: LengthMode::Bytes,
        }
    }
}

// ============================================================================
// LENGTH RANGE
// ============================================================================

/// Validates that a string length is within a range.
///
/// This is more efficient than using `min_length().and(max_length())`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LengthRange {
    /// Minimum length (inclusive).
    pub min: usize,
    /// Maximum length (inclusive).
    pub max: usize,
    /// How to count length.
    pub mode: LengthMode,
}

impl LengthRange {
    /// Creates a new length range validator (counts Unicode chars by default).
    ///
    /// Returns an error if `min > max`.
    pub fn new(min: usize, max: usize) -> Result<Self, ValidationError> {
        if min > max {
            return Err(ValidationError::new("invalid_range", "min must be <= max"));
        }
        Ok(Self {
            min,
            max,
            mode: LengthMode::Chars,
        })
    }

    /// Creates a length range validator that counts bytes.
    ///
    /// Returns an error if `min > max`.
    pub fn bytes(min: usize, max: usize) -> Result<Self, ValidationError> {
        if min > max {
            return Err(ValidationError::new("invalid_range", "min must be <= max"));
        }
        Ok(Self {
            min,
            max,
            mode: LengthMode::Bytes,
        })
    }
}

impl Validate for LengthRange {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let len = self.mode.measure(input);
        if len >= self.min && len <= self.max {
            Ok(())
        } else {
            Err(ValidationError::new(
                "length_range",
                format!(
                    "String length must be between {} and {}",
                    self.min, self.max
                ),
            )
            .with_param("min", self.min.to_string())
            .with_param("max", self.max.to_string())
            .with_param("actual", len.to_string()))
        }
    }
}

/// Creates a length range validator.
pub fn length_range(min: usize, max: usize) -> Result<LengthRange, ValidationError> {
    LengthRange::new(min, max)
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_min_length_valid() {
        let validator = MinLength::new(5);
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hello world").is_ok());
    }

    #[test]
    fn test_min_length_invalid() {
        let validator = MinLength::new(5);
        assert!(validator.validate("hi").is_err());
        assert!(validator.validate("").is_err());
    }

    #[test]
    fn test_min_length_exact() {
        let validator = MinLength::new(5);
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_max_length_valid() {
        let validator = MaxLength::new(10);
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("helloworld").is_ok());
    }

    #[test]
    fn test_max_length_invalid() {
        let validator = MaxLength::new(10);
        assert!(validator.validate("verylongstring").is_err());
    }

    #[test]
    fn test_max_length_exact() {
        let validator = MaxLength::new(5);
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_exact_length_valid() {
        let validator = ExactLength::new(5);
        assert!(validator.validate("hello").is_ok());
    }

    #[test]
    fn test_exact_length_too_short() {
        let validator = ExactLength::new(5);
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_exact_length_too_long() {
        let validator = ExactLength::new(5);
        assert!(validator.validate("toolong").is_err());
    }

    #[test]
    fn test_length_range_valid() {
        let validator = LengthRange::new(5, 10).unwrap();
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("helloworld").is_ok());
    }

    #[test]
    fn test_length_range_too_short() {
        let validator = LengthRange::new(5, 10).unwrap();
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_length_range_too_long() {
        let validator = LengthRange::new(5, 10).unwrap();
        assert!(validator.validate("verylongstring").is_err());
    }

    #[test]
    fn test_length_range_boundaries() {
        let validator = LengthRange::new(5, 10).unwrap();
        assert!(validator.validate("hello").is_ok()); // min
        assert!(validator.validate("helloworld").is_ok()); // max
    }

    #[test]
    fn test_length_range_invalid() {
        assert!(LengthRange::new(10, 5).is_err());
    }

    #[test]
    fn test_not_empty_valid() {
        let validator = NotEmpty;
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate(" ").is_ok()); // whitespace is not empty
    }

    #[test]
    fn test_not_empty_invalid() {
        let validator = NotEmpty;
        assert!(validator.validate("").is_err());
    }

    #[test]
    fn test_helper_functions() {
        assert!(min_length(5).validate("hello").is_ok());
        assert!(max_length(10).validate("hello").is_ok());
        assert!(exact_length(5).validate("hello").is_ok());
        assert!(length_range(5, 10).unwrap().validate("hello").is_ok());
        assert!(not_empty().validate("hello").is_ok());
    }

    #[test]
    fn test_unicode_handling() {
        // Default mode counts Unicode chars, not bytes
        let validator = MinLength::new(5);
        assert!(validator.validate("hello").is_ok()); // 5 chars
        assert!(validator.validate("\u{1f44b}\u{1f30d}").is_err()); // 2 chars < 5

        // Bytes mode counts raw bytes
        let byte_validator = MinLength::bytes(5);
        assert!(byte_validator.validate("\u{1f44b}\u{1f30d}").is_ok()); // 8 bytes >= 5

        // Demonstrate the difference
        assert_eq!("h\u{e9}llo".chars().count(), 5); // 5 chars
        assert_eq!("h\u{e9}llo".len(), 6); // 6 bytes (e with accent = 2 bytes)
        assert!(MinLength::new(5).validate("h\u{e9}llo").is_ok()); // char count
        assert!(MinLength::bytes(6).validate("h\u{e9}llo").is_ok()); // byte count
    }

    #[test]
    fn test_composition() {
        use crate::foundation::ValidateExt;

        // Compose length validators
        let validator = min_length(5).and(max_length(10));
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
        assert!(validator.validate("verylongstring").is_err());
    }
}
