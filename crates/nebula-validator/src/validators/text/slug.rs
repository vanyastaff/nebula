//! Slug validator for URL-friendly strings.
//!
//! Validates that a string is a valid slug (lowercase letters, numbers, hyphens).

use crate::core::{TypedValidator, ValidationError, ValidatorMetadata, ValidationComplexity};

// ============================================================================
// SLUG VALIDATOR
// ============================================================================

/// Validates URL-friendly slug strings.
///
/// A slug is typically used in URLs and must contain only:
/// - Lowercase letters (a-z)
/// - Numbers (0-9)
/// - Hyphens (-)
///
/// Additional rules:
/// - Cannot start or end with a hyphen
/// - Cannot contain consecutive hyphens
/// - Must have minimum length (default: 1)
/// - Must have maximum length (default: 255)
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::Slug;
/// use nebula_validator::core::TypedValidator;
///
/// let validator = Slug::new();
///
/// // Valid slugs
/// assert!(validator.validate("hello-world").is_ok());
/// assert!(validator.validate("my-post-123").is_ok());
/// assert!(validator.validate("a").is_ok());
/// assert!(validator.validate("123-test").is_ok());
///
/// // Invalid
/// assert!(validator.validate("Hello-World").is_err()); // uppercase
/// assert!(validator.validate("hello_world").is_err()); // underscore
/// assert!(validator.validate("hello--world").is_err()); // consecutive hyphens
/// assert!(validator.validate("-hello").is_err()); // starts with hyphen
/// assert!(validator.validate("hello-").is_err()); // ends with hyphen
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Slug {
    min_length: usize,
    max_length: usize,
    allow_consecutive_hyphens: bool,
}

impl Slug {
    /// Creates a new slug validator with default settings.
    ///
    /// Default settings:
    /// - min_length: 1
    /// - max_length: 255
    /// - allow_consecutive_hyphens: false
    pub fn new() -> Self {
        Self {
            min_length: 1,
            max_length: 255,
            allow_consecutive_hyphens: false,
        }
    }

    /// Sets the minimum length for the slug.
    pub fn min_length(mut self, min: usize) -> Self {
        self.min_length = min;
        self
    }

    /// Sets the maximum length for the slug.
    pub fn max_length(mut self, max: usize) -> Self {
        self.max_length = max;
        self
    }

    /// Allows consecutive hyphens in the slug.
    pub fn allow_consecutive_hyphens(mut self) -> Self {
        self.allow_consecutive_hyphens = true;
        self
    }

    fn is_valid_slug_char(c: char) -> bool {
        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
    }
}

impl Default for Slug {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedValidator for Slug {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<Self::Output, Self::Error> {
        // Check length
        if input.len() < self.min_length {
            return Err(ValidationError::new(
                "slug_too_short",
                format!("Slug must be at least {} characters", self.min_length),
            ));
        }

        if input.len() > self.max_length {
            return Err(ValidationError::new(
                "slug_too_long",
                format!("Slug must not exceed {} characters", self.max_length),
            ));
        }

        // Check if empty (should be caught by min_length, but explicit check)
        if input.is_empty() {
            return Err(ValidationError::new("empty_slug", "Slug cannot be empty"));
        }

        // Check start/end characters
        if input.starts_with('-') {
            return Err(ValidationError::new(
                "invalid_slug_start",
                "Slug cannot start with a hyphen",
            ));
        }

        if input.ends_with('-') {
            return Err(ValidationError::new(
                "invalid_slug_end",
                "Slug cannot end with a hyphen",
            ));
        }

        // Check for invalid characters and consecutive hyphens
        let mut prev_was_hyphen = false;
        for c in input.chars() {
            if !Self::is_valid_slug_char(c) {
                return Err(ValidationError::new(
                    "invalid_slug_char",
                    format!(
                        "Slug contains invalid character '{}'. Only lowercase letters, numbers, and hyphens are allowed",
                        c
                    ),
                ));
            }

            if c == '-' {
                if prev_was_hyphen && !self.allow_consecutive_hyphens {
                    return Err(ValidationError::new(
                        "consecutive_hyphens",
                        "Slug cannot contain consecutive hyphens",
                    ));
                }
                prev_was_hyphen = true;
            } else {
                prev_was_hyphen = false;
            }
        }

        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Slug".to_string(),
            description: Some(format!(
                "Validates URL-friendly slugs (length: {}-{}, consecutive hyphens: {})",
                self.min_length,
                self.max_length,
                self.allow_consecutive_hyphens
            )),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(5)),
            tags: vec!["text".to_string(), "url".to_string(), "slug".to_string()],
            version: Some("1.0.0".to_string()),
            custom: std::collections::HashMap::new(),
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_slugs() {
        let validator = Slug::new();
        assert!(validator.validate("hello-world").is_ok());
        assert!(validator.validate("my-post-123").is_ok());
        assert!(validator.validate("a").is_ok());
        assert!(validator.validate("123").is_ok());
        assert!(validator.validate("test-123-abc").is_ok());
        assert!(validator.validate("single").is_ok());
    }

    #[test]
    fn test_invalid_uppercase() {
        let validator = Slug::new();
        assert!(validator.validate("Hello-World").is_err());
        assert!(validator.validate("HELLO").is_err());
        assert!(validator.validate("hEllo").is_err());
    }

    #[test]
    fn test_invalid_special_chars() {
        let validator = Slug::new();
        assert!(validator.validate("hello_world").is_err());
        assert!(validator.validate("hello world").is_err());
        assert!(validator.validate("hello.world").is_err());
        assert!(validator.validate("hello@world").is_err());
        assert!(validator.validate("hello!").is_err());
    }

    #[test]
    fn test_consecutive_hyphens() {
        let validator = Slug::new();
        assert!(validator.validate("hello--world").is_err());
        assert!(validator.validate("a---b").is_err());

        // Allow consecutive hyphens
        let validator = Slug::new().allow_consecutive_hyphens();
        assert!(validator.validate("hello--world").is_ok());
        assert!(validator.validate("a---b").is_ok());
    }

    #[test]
    fn test_hyphen_at_start_or_end() {
        let validator = Slug::new();
        assert!(validator.validate("-hello").is_err());
        assert!(validator.validate("hello-").is_err());
        assert!(validator.validate("-hello-").is_err());
        assert!(validator.validate("-").is_err());
    }

    #[test]
    fn test_length_constraints() {
        let validator = Slug::new().min_length(3).max_length(10);

        assert!(validator.validate("ab").is_err()); // too short
        assert!(validator.validate("abc").is_ok()); // min length
        assert!(validator.validate("abcdefghij").is_ok()); // max length
        assert!(validator.validate("abcdefghijk").is_err()); // too long
    }

    #[test]
    fn test_empty_slug() {
        let validator = Slug::new();
        assert!(validator.validate("").is_err());
    }

    #[test]
    fn test_numbers_only() {
        let validator = Slug::new();
        assert!(validator.validate("123").is_ok());
        assert!(validator.validate("456789").is_ok());
    }

    #[test]
    fn test_mixed_alphanumeric() {
        let validator = Slug::new();
        assert!(validator.validate("abc123").is_ok());
        assert!(validator.validate("123abc").is_ok());
        assert!(validator.validate("a1b2c3").is_ok());
    }

    #[test]
    fn test_single_character() {
        let validator = Slug::new();
        assert!(validator.validate("a").is_ok());
        assert!(validator.validate("z").is_ok());
        assert!(validator.validate("1").is_ok());
        assert!(validator.validate("-").is_err()); // hyphen alone
    }

    #[test]
    fn test_real_world_slugs() {
        let validator = Slug::new();
        assert!(validator.validate("introduction-to-rust").is_ok());
        assert!(validator.validate("chapter-1").is_ok());
        assert!(validator.validate("2023-annual-report").is_ok());
        assert!(validator.validate("my-first-blog-post").is_ok());
    }
}
