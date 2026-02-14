//! String length validators
//!
//! This module provides validators for checking string length constraints.
//! By default, length is measured in Unicode scalar values (chars).
//! Use the `.bytes()` constructor for byte-length counting when performance
//! is critical and the input is known to be ASCII.

use crate::core::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};

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
// MIN LENGTH
// ============================================================================

/// Validates that a string has at least a minimum length.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validators::string::MinLength;
///
/// let validator = MinLength { min: 5 };
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("hi").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MinLength {
    /// Minimum required length (inclusive).
    pub min: usize,
    /// How to count length.
    pub mode: LengthMode,
}

impl MinLength {
    /// Creates a new minimum length validator (counts Unicode chars by default).
    #[must_use]
    pub fn new(min: usize) -> Self {
        Self {
            min,
            mode: LengthMode::Chars,
        }
    }

    /// Creates a minimum length validator that counts bytes.
    #[must_use]
    pub fn bytes(min: usize) -> Self {
        Self {
            min,
            mode: LengthMode::Bytes,
        }
    }
}

impl Validate for MinLength {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let len = self.mode.measure(input);
        if len >= self.min {
            Ok(())
        } else {
            Err(ValidationError::min_length("", self.min, len))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "MinLength".into(),
            description: Some(format!("String must be at least {} characters", self.min).into()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".into(), "length".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

/// Creates a minimum length validator.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validators::string::min_length;
///
/// let validator = min_length(5);
/// assert!(validator.validate("hello").is_ok());
/// ```
#[must_use]
pub fn min_length(min: usize) -> MinLength {
    MinLength::new(min)
}

// ============================================================================
// MAX LENGTH
// ============================================================================

/// Validates that a string does not exceed a maximum length.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validators::string::MaxLength;
///
/// let validator = MaxLength { max: 10 };
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("verylongstring").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaxLength {
    /// Maximum allowed length (inclusive).
    pub max: usize,
    /// How to count length.
    pub mode: LengthMode,
}

impl MaxLength {
    /// Creates a new maximum length validator (counts Unicode chars by default).
    #[must_use]
    pub fn new(max: usize) -> Self {
        Self {
            max,
            mode: LengthMode::Chars,
        }
    }

    /// Creates a maximum length validator that counts bytes.
    #[must_use]
    pub fn bytes(max: usize) -> Self {
        Self {
            max,
            mode: LengthMode::Bytes,
        }
    }
}

impl Validate for MaxLength {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let len = self.mode.measure(input);
        if len <= self.max {
            Ok(())
        } else {
            Err(ValidationError::max_length("", self.max, len))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "MaxLength".into(),
            description: Some(format!("String must be at most {} characters", self.max).into()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".into(), "length".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

/// Creates a maximum length validator.
#[must_use]
pub fn max_length(max: usize) -> MaxLength {
    MaxLength::new(max)
}

// ============================================================================
// EXACT LENGTH
// ============================================================================

/// Validates that a string has an exact length.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validators::string::ExactLength;
///
/// let validator = ExactLength { length: 5 };
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("hi").is_err());
/// assert!(validator.validate("toolong").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExactLength {
    /// Required exact length.
    pub length: usize,
    /// How to count length.
    pub mode: LengthMode,
}

impl ExactLength {
    /// Creates a new exact length validator (counts Unicode chars by default).
    #[must_use]
    pub fn new(length: usize) -> Self {
        Self {
            length,
            mode: LengthMode::Chars,
        }
    }

    /// Creates an exact length validator that counts bytes.
    #[must_use]
    pub fn bytes(length: usize) -> Self {
        Self {
            length,
            mode: LengthMode::Bytes,
        }
    }
}

impl Validate for ExactLength {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        let len = self.mode.measure(input);
        if len == self.length {
            Ok(())
        } else {
            Err(ValidationError::new(
                "exact_length",
                format!("String must be exactly {} characters", self.length),
            )
            .with_param("expected", self.length.to_string())
            .with_param("actual", len.to_string()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "ExactLength".into(),
            description: Some(format!("String must be exactly {} characters", self.length).into()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".into(), "length".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

/// Creates an exact length validator.
#[must_use]
pub fn exact_length(length: usize) -> ExactLength {
    ExactLength::new(length)
}

// ============================================================================
// LENGTH RANGE
// ============================================================================

/// Validates that a string length is within a range.
///
/// This is more efficient than using `min_length().and(max_length())`.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validators::string::LengthRange;
///
/// let validator = LengthRange { min: 5, max: 10 };
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("hi").is_err());
/// assert!(validator.validate("verylongstring").is_err());
/// ```
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

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "LengthRange".into(),
            description: Some(
                format!(
                    "String must be between {} and {} characters",
                    self.min, self.max
                )
                .into(),
            ),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".into(), "length".into(), "range".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

/// Creates a length range validator.
pub fn length_range(min: usize, max: usize) -> Result<LengthRange, ValidationError> {
    LengthRange::new(min, max)
}

// ============================================================================
// NOT EMPTY
// ============================================================================

/// Validates that a string is not empty.
///
/// This is equivalent to `MinLength { min: 1 }` but more semantic.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validators::string::NotEmpty;
///
/// let validator = NotEmpty;
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotEmpty;

impl Validate for NotEmpty {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if input.is_empty() {
            Err(ValidationError::new(
                "not_empty",
                "String must not be empty",
            ))
        } else {
            Ok(())
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "NotEmpty".into(),
            description: Some("String must not be empty".into()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".into(), "length".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

/// Creates a not-empty validator.
#[must_use]
pub const fn not_empty() -> NotEmpty {
    NotEmpty
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
    fn test_metadata() {
        let validator = MinLength::new(5);
        let meta = validator.metadata();

        assert_eq!(meta.name, "MinLength");
        assert!(meta.description.is_some());
        assert_eq!(meta.complexity, ValidationComplexity::Constant);
        assert!(meta.tags.contains(&"string".into()));
    }

    #[test]
    fn test_unicode_handling() {
        // Default mode counts Unicode chars, not bytes
        let validator = MinLength::new(5);
        assert!(validator.validate("hello").is_ok()); // 5 chars
        assert!(validator.validate("👋🌍").is_err()); // 2 chars < 5

        // Bytes mode counts raw bytes
        let byte_validator = MinLength::bytes(5);
        assert!(byte_validator.validate("👋🌍").is_ok()); // 8 bytes >= 5

        // Demonstrate the difference
        assert_eq!("héllo".chars().count(), 5); // 5 chars
        assert_eq!("héllo".len(), 6); // 6 bytes (é = 2 bytes)
        assert!(MinLength::new(5).validate("héllo").is_ok()); // char count
        assert!(MinLength::bytes(6).validate("héllo").is_ok()); // byte count
    }

    #[test]
    fn test_composition() {
        use crate::core::ValidateExt;

        // Compose length validators
        let validator = min_length(5).and(max_length(10));
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
        assert!(validator.validate("verylongstring").is_err());
    }
}
