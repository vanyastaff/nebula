//! String pattern validators
//!
//! This module provides validators for checking string patterns and formats.

use crate::core::{TypedValidator, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// CONTAINS
// ============================================================================

/// Validates that a string contains a substring.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::validators::string::Contains;
///
/// let validator = Contains { substring: "test".to_string() };
/// assert!(validator.validate("test string").is_ok());
/// assert!(validator.validate("hello world").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Contains {
    /// The substring to search for.
    pub substring: String,
}

impl Contains {
    /// Creates a new contains validator.
    pub fn new(substring: impl Into<String>) -> Self {
        Self {
            substring: substring.into(),
        }
    }
}

impl TypedValidator for Contains {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input.contains(&self.substring) {
            Ok(())
        } else {
            Err(ValidationError::new(
                "contains",
                format!("String must contain '{}'", self.substring),
            )
            .with_param("substring", self.substring.clone()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Contains".to_string(),
            description: Some(format!("String must contain '{}'", self.substring)),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "pattern".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

/// Creates a contains validator.
pub fn contains(substring: impl Into<String>) -> Contains {
    Contains::new(substring)
}

// ============================================================================
// STARTS WITH
// ============================================================================

/// Validates that a string starts with a prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StartsWith {
    /// The required prefix.
    pub prefix: String,
}

impl StartsWith {
    pub fn new(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
        }
    }
}

impl TypedValidator for StartsWith {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input.starts_with(&self.prefix) {
            Ok(())
        } else {
            Err(ValidationError::new(
                "starts_with",
                format!("String must start with '{}'", self.prefix),
            )
            .with_param("prefix", self.prefix.clone()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "StartsWith".to_string(),
            description: Some(format!("String must start with '{}'", self.prefix)),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "pattern".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

pub fn starts_with(prefix: impl Into<String>) -> StartsWith {
    StartsWith::new(prefix)
}

// ============================================================================
// ENDS WITH
// ============================================================================

/// Validates that a string ends with a suffix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EndsWith {
    /// The required suffix.
    pub suffix: String,
}

impl EndsWith {
    pub fn new(suffix: impl Into<String>) -> Self {
        Self {
            suffix: suffix.into(),
        }
    }
}

impl TypedValidator for EndsWith {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input.ends_with(&self.suffix) {
            Ok(())
        } else {
            Err(ValidationError::new(
                "ends_with",
                format!("String must end with '{}'", self.suffix),
            )
            .with_param("suffix", self.suffix.clone()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "EndsWith".to_string(),
            description: Some(format!("String must end with '{}'", self.suffix)),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "pattern".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

pub fn ends_with(suffix: impl Into<String>) -> EndsWith {
    EndsWith::new(suffix)
}

// ============================================================================
// ALPHANUMERIC
// ============================================================================

/// Validates that a string contains only alphanumeric characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Alphanumeric {
    /// Whether to allow spaces.
    pub allow_spaces: bool,
}

impl Alphanumeric {
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_spaces: false,
        }
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn with_spaces(mut self) -> Self {
        self.allow_spaces = true;
        self
    }
}

impl Default for Alphanumeric {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedValidator for Alphanumeric {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let is_valid = input
            .chars()
            .all(|c| c.is_alphanumeric() || (self.allow_spaces && c.is_whitespace()));

        if is_valid {
            Ok(())
        } else {
            let msg = if self.allow_spaces {
                "String must contain only letters, numbers, and spaces"
            } else {
                "String must contain only letters and numbers"
            };
            Err(ValidationError::new("alphanumeric", msg))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Alphanumeric".to_string(),
            description: Some("String must be alphanumeric".to_string()),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "pattern".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

#[must_use]
pub fn alphanumeric() -> Alphanumeric {
    Alphanumeric::new()
}

// ============================================================================
// ALPHABETIC
// ============================================================================

/// Validates that a string contains only alphabetic characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Alphabetic {
    /// Whether to allow spaces in the alphabetic string.
    pub allow_spaces: bool,
}

impl Alphabetic {
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_spaces: false,
        }
    }

    #[must_use = "builder methods must be chained or built"]
    pub fn with_spaces(mut self) -> Self {
        self.allow_spaces = true;
        self
    }
}

impl Default for Alphabetic {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedValidator for Alphabetic {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        let is_valid = input
            .chars()
            .all(|c| c.is_alphabetic() || (self.allow_spaces && c.is_whitespace()));

        if is_valid {
            Ok(())
        } else {
            Err(ValidationError::new(
                "alphabetic",
                "String must contain only letters",
            ))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Alphabetic".to_string(),
            description: Some("String must contain only letters".to_string()),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "pattern".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

#[must_use]
pub fn alphabetic() -> Alphabetic {
    Alphabetic::new()
}

// ============================================================================
// NUMERIC
// ============================================================================

/// Validates that a string contains only numeric characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Numeric;

impl TypedValidator for Numeric {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input.chars().all(char::is_numeric) {
            Ok(())
        } else {
            Err(ValidationError::new(
                "numeric",
                "String must contain only numbers",
            ))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Numeric".to_string(),
            description: Some("String must be numeric".to_string()),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "pattern".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

#[must_use]
pub const fn numeric() -> Numeric {
    Numeric
}

// ============================================================================
// LOWERCASE / UPPERCASE
// ============================================================================

/// Validates that a string is lowercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Lowercase;

impl TypedValidator for Lowercase {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input
            .chars()
            .all(|c| !c.is_alphabetic() || c.is_lowercase())
        {
            Ok(())
        } else {
            Err(ValidationError::new(
                "lowercase",
                "String must be lowercase",
            ))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Lowercase")
            .with_tag("string")
            .with_tag("case")
    }
}

#[must_use]
pub const fn lowercase() -> Lowercase {
    Lowercase
}

/// Validates that a string is uppercase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Uppercase;

impl TypedValidator for Uppercase {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if input
            .chars()
            .all(|c| !c.is_alphabetic() || c.is_uppercase())
        {
            Ok(())
        } else {
            Err(ValidationError::new(
                "uppercase",
                "String must be uppercase",
            ))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Uppercase")
            .with_tag("string")
            .with_tag("case")
    }
}

#[must_use]
pub const fn uppercase() -> Uppercase {
    Uppercase
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contains() {
        let validator = contains("test");
        assert!(validator.validate("this is a test").is_ok());
        assert!(validator.validate("hello world").is_err());
    }

    #[test]
    fn test_starts_with() {
        let validator = starts_with("http://");
        assert!(validator.validate("http://example.com").is_ok());
        assert!(validator.validate("https://example.com").is_err());
    }

    #[test]
    fn test_ends_with() {
        let validator = ends_with(".com");
        assert!(validator.validate("example.com").is_ok());
        assert!(validator.validate("example.org").is_err());
    }

    #[test]
    fn test_alphanumeric() {
        let validator = alphanumeric();
        assert!(validator.validate("hello123").is_ok());
        assert!(validator.validate("hello_123").is_err());
        assert!(validator.validate("hello 123").is_err());
    }

    #[test]
    fn test_alphanumeric_with_spaces() {
        let validator = alphanumeric().with_spaces();
        assert!(validator.validate("hello 123").is_ok());
        assert!(validator.validate("hello_123").is_err());
    }

    #[test]
    fn test_alphabetic() {
        let validator = alphabetic();
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hello123").is_err());
    }

    #[test]
    fn test_numeric() {
        let validator = numeric();
        assert!(validator.validate("12345").is_ok());
        assert!(validator.validate("123.45").is_err());
    }

    #[test]
    fn test_lowercase() {
        let validator = lowercase();
        assert!(validator.validate("hello").is_ok());
        assert!(validator.validate("hello123").is_ok());
        assert!(validator.validate("Hello").is_err());
    }

    #[test]
    fn test_uppercase() {
        let validator = uppercase();
        assert!(validator.validate("HELLO").is_ok());
        assert!(validator.validate("HELLO123").is_ok());
        assert!(validator.validate("Hello").is_err());
    }
}
