//! String content validators
//!
//! Validators for checking string content and patterns.

use crate::core::{TypedValidator, ValidationError, ValidatorMetadata, ValidationComplexity};

// ============================================================================
// REGEX VALIDATOR
// ============================================================================

/// Validates that a string matches a regular expression.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::validators::string::MatchesRegex;
/// use regex::Regex;
///
/// let validator = MatchesRegex {
///     pattern: Regex::new(r"^\d{3}-\d{4}$").unwrap()
/// };
/// assert!(validator.validate("123-4567").is_ok());
/// assert!(validator.validate("invalid").is_err());
/// ```
#[derive(Debug, Clone)]
pub struct MatchesRegex {
    pub pattern: regex::Regex,
}

impl MatchesRegex {
    pub fn new(pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            pattern: regex::Regex::new(pattern)?,
        })
    }
}

impl TypedValidator for MatchesRegex {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if self.pattern.is_match(input) {
            Ok(())
        } else {
            Err(ValidationError::new(
                "regex",
                format!("String must match pattern: {}", self.pattern.as_str()),
            )
            .with_param("pattern", self.pattern.as_str().to_string()))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "MatchesRegex".to_string(),
            description: Some(format!("Must match: {}", self.pattern.as_str())),
            complexity: ValidationComplexity::Expensive,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "regex".to_string(), "pattern".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

pub fn matches_regex(pattern: &str) -> Result<MatchesRegex, regex::Error> {
    MatchesRegex::new(pattern)
}

// ============================================================================
// EMAIL VALIDATOR
// ============================================================================

/// Validates email format.
///
/// Uses a simple but effective regex pattern.
#[derive(Debug, Clone)]
pub struct Email {
    pattern: regex::Regex,
}

impl Email {
    pub fn new() -> Self {
        // Simple email pattern - can be made more strict
        let pattern = regex::Regex::new(
            r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$"
        ).unwrap();
        
        Self { pattern }
    }
}

impl Default for Email {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedValidator for Email {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if self.pattern.is_match(input) {
            Ok(())
        } else {
            Err(ValidationError::invalid_format("", "email"))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Email".to_string(),
            description: Some("Valid email address".to_string()),
            complexity: ValidationComplexity::Expensive,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "email".to_string(), "format".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

pub fn email() -> Email {
    Email::new()
}

// ============================================================================
// URL VALIDATOR
// ============================================================================

/// Validates URL format.
#[derive(Debug, Clone)]
pub struct Url {
    pattern: regex::Regex,
}

impl Url {
    pub fn new() -> Self {
        let pattern = regex::Regex::new(
            r"^https?://[^\s/$.?#].[^\s]*$"
        ).unwrap();
        
        Self { pattern }
    }
}

impl Default for Url {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedValidator for Url {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        if self.pattern.is_match(input) {
            Ok(())
        } else {
            Err(ValidationError::invalid_format("", "url"))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Url".to_string(),
            description: Some("Valid URL".to_string()),
            complexity: ValidationComplexity::Expensive,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".to_string(), "url".to_string(), "format".to_string()],
            version: None,
            custom: std::collections::HashMap::new(),
        }
    }
}

pub fn url() -> Url {
    Url::new()
}

// ============================================================================
// UUID VALIDATOR
// ============================================================================

/// Validates UUID format.
#[derive(Debug, Clone, Copy)]
pub struct Uuid;

impl TypedValidator for Uuid {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &Self::Input) -> Result<Self::Output, Self::Error> {
        // Simple UUID validation
        if input.len() == 36 && input.chars().filter(|&c| c == '-').count() == 4 {
            Ok(())
        } else {
            Err(ValidationError::invalid_format("", "uuid"))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata::simple("Uuid")
            .with_tag("string")
            .with_tag("format")
    }
}

pub const fn uuid() -> Uuid {
    Uuid
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regex() {
        let validator = matches_regex(r"^\d{3}-\d{4}$").unwrap();
        assert!(validator.validate("123-4567").is_ok());
        assert!(validator.validate("invalid").is_err());
    }

    #[test]
    fn test_email() {
        let validator = email();
        assert!(validator.validate("user@example.com").is_ok());
        assert!(validator.validate("invalid").is_err());
        assert!(validator.validate("@example.com").is_err());
        assert!(validator.validate("user@").is_err());
    }

    #[test]
    fn test_url() {
        let validator = url();
        assert!(validator.validate("http://example.com").is_ok());
        assert!(validator.validate("https://example.com/path").is_ok());
        assert!(validator.validate("invalid").is_err());
        assert!(validator.validate("ftp://example.com").is_err());
    }

    #[test]
    fn test_uuid() {
        let validator = uuid();
        assert!(validator.validate("123e4567-e89b-12d3-a456-426614174000").is_ok());
        assert!(validator.validate("invalid").is_err());
    }
}