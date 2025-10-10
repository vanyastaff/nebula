//! String length validators
//!
//! This module provides validators for checking string length constraints.

use crate::core::{TypedValidator, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// MIN LENGTH
// ============================================================================

/// Validates that a string has at least a minimum length.
///
/// # Examples
///
/// ```rust
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
}

impl MinLength {
    /// Creates a new minimum length validator.
    #[must_use] 
    pub fn new(min: usize) -> Self {
        Self { min }
    }
}

impl TypedValidator for MinLength {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input.len() >= self.min {
            Ok(())
        } else {
            Err(ValidationError::min_length("", self.min, input.len()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "MinLength".to_string(),
            description: Some(format!("String must be at least {} characters", self.min)),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "length".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a minimum length validator.
///
/// # Examples
///
/// ```rust
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
/// ```rust
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
}

impl MaxLength {
    /// Creates a new maximum length validator.
    #[must_use] 
    pub fn new(max: usize) -> Self {
        Self { max }
    }
}

impl TypedValidator for MaxLength {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input.len() <= self.max {
            Ok(())
        } else {
            Err(ValidationError::max_length("", self.max, input.len()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "MaxLength".to_string(),
            description: Some(format!("String must be at most {} characters", self.max)),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "length".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
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
/// ```rust
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
}

impl ExactLength {
    /// Creates a new exact length validator.
    #[must_use] 
    pub fn new(length: usize) -> Self {
        Self { length }
    }
}

impl TypedValidator for ExactLength {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input.len() == self.length {
            Ok(())
        } else {
            Err(ValidationError::new(
                "exact_length",
                format!("String must be exactly {} characters", self.length),
            )
            .with_param("expected", self.length.to_string())
            .with_param("actual", input.len().to_string()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "ExactLength".to_string(),
            description: Some(format!("String must be exactly {} characters", self.length)),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "length".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
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
/// ```rust
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
}

impl LengthRange {
    /// Creates a new length range validator.
    ///
    /// # Panics
    ///
    /// Panics if `min > max`.
    #[must_use] 
    pub fn new(min: usize, max: usize) -> Self {
        assert!(min <= max, "min must be <= max");
        Self { min, max }
    }
}

impl TypedValidator for LengthRange {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let len = input.len();
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
            name: "LengthRange".to_string(),
            description: Some(format!(
                "String must be between {} and {} characters",
                self.min, self.max
            )),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec![
                "string".to_string(),
                "length".to_string(),
                "range".to_string(),
            ],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a length range validator.
#[must_use] 
pub fn length_range(min: usize, max: usize) -> LengthRange {
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
/// ```rust
/// use nebula_validator::validators::string::NotEmpty;
///
/// let validator = NotEmpty;
/// assert!(validator.validate("hello").is_ok());
/// assert!(validator.validate("").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NotEmpty;

impl TypedValidator for NotEmpty {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
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
            name: "NotEmpty".to_string(),
            description: Some("String must not be empty".to_string()),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "length".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
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
        let validator = LengthRange::new(5, 10);
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("helloworld").is_ok());
    }

    #[test]
    fn test_length_range_too_short() {
        let validator = LengthRange::new(5, 10);
        assert!(validator.validate("hi").is_err());
    }

    #[test]
    fn test_length_range_too_long() {
        let validator = LengthRange::new(5, 10);
        assert!(validator.validate("verylongstring").is_err());
    }

    #[test]
    fn test_length_range_boundaries() {
        let validator = LengthRange::new(5, 10);
        assert!(validator.validate("hello").is_ok()); // min
        assert!(validator.validate("helloworld").is_ok()); // max
    }

    #[test]
    #[should_panic(expected = "min must be <= max")]
    fn test_length_range_invalid() {
        LengthRange::new(10, 5);
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
        assert!(length_range(5, 10).validate("hello").is_ok());
        assert!(not_empty().validate("hello").is_ok());
    }

    #[test]
    fn test_metadata() {
        let validator = MinLength::new(5);
        let meta = validator.metadata();

        assert_eq!(meta.name, "MinLength");
        assert!(meta.description.is_some());
        assert_eq!(meta.complexity, ValidationComplexity::Constant);
        assert!(meta.tags.contains(&"string".to_string()));
    }

    #[test]
    fn test_unicode_handling() {
        // Emoji and multi-byte characters
        let validator = MinLength::new(5);
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("👋🌍").is_err()); // 2 chars but 8 bytes

        // String length counts Unicode scalar values, not bytes
        assert_eq!("👋🌍".len(), 8); // bytes
        assert_eq!("👋🌍".chars().count(), 2); // characters
    }

    #[test]
    fn test_composition() {
        use crate::core::ValidatorExt;

        // Compose length validators
        let validator = min_length(5).and(max_length(10));
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hi").is_err());
        assert!(validator.validate("verylongstring").is_err());
    }
}
