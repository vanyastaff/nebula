//! String content validators
//!
//! Validators for checking string content and patterns.

use crate::foundation::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// REGEX VALIDATOR
// ============================================================================

/// Validates that a string matches a regular expression.
///
/// # Examples
///
/// ```rust,ignore
/// use nebula_validator::validators::MatchesRegex;
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
    /// The compiled regex pattern to match against.
    pub pattern: regex::Regex,
}

impl MatchesRegex {
    #[must_use = "constructor result must be handled"]
    pub fn new(pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            pattern: regex::Regex::new(pattern)?,
        })
    }
}

impl Validate for MatchesRegex {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
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
            name: "MatchesRegex".into(),
            description: Some(format!("Must match: {}", self.pattern.as_str()).into()),
            complexity: ValidationComplexity::Expensive,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".into(), "regex".into(), "pattern".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

#[must_use = "validator must be used"]
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
    #[must_use]
    pub fn new() -> Self {
        // Simple email pattern - can be made more strict
        let pattern = regex::Regex::new(
            r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$"
        ).expect("hardcoded email regex pattern is valid");

        Self { pattern }
    }
}

impl Default for Email {
    fn default() -> Self {
        Self::new()
    }
}

impl Validate for Email {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if self.pattern.is_match(input) {
            Ok(())
        } else {
            Err(ValidationError::invalid_format("", "email"))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Email".into(),
            description: Some("Valid email address".into()),
            complexity: ValidationComplexity::Expensive,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".into(), "email".into(), "format".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

#[must_use]
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
    #[must_use]
    pub fn new() -> Self {
        let pattern = regex::Regex::new(r"^https?://[^\s/$.?#].[^\s]*$")
            .expect("hardcoded URL regex pattern is valid");

        Self { pattern }
    }
}

impl Default for Url {
    fn default() -> Self {
        Self::new()
    }
}

impl Validate for Url {
    type Input = str;

    fn validate(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if self.pattern.is_match(input) {
            Ok(())
        } else {
            Err(ValidationError::invalid_format("", "url"))
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Url".into(),
            description: Some("Valid URL".into()),
            complexity: ValidationComplexity::Expensive,
            cacheable: true,
            estimated_time: None,
            tags: vec!["string".into(), "url".into(), "format".into()],
            version: None,
            custom: Vec::new(),
        }
    }
}

#[must_use]
pub fn url() -> Url {
    Url::new()
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
}
