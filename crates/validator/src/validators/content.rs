//! String content validators
//!
//! Validators for checking string content and patterns.

use std::sync::LazyLock;

use crate::foundation::ValidationError;

static EMAIL_REGEX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$"
    ).unwrap()
});

static URL_REGEX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^https?://[^\s/$.?#].[^\s]*$").unwrap()
});

// ============================================================================
// REGEX VALIDATOR
// ============================================================================

crate::validator! {
    /// Validates that a string matches a regular expression.
    pub MatchesRegex { pattern: regex::Regex } for str;
    rule(self, input) { self.pattern.is_match(input) }
    error(self, input) {
        ValidationError::invalid_format("", "regex")
            .with_param("pattern", self.pattern.as_str().to_string())
    }
    new(pattern: &str) -> regex::Error {
        Ok(Self {
            pattern: regex::Regex::new(pattern)?,
        })
    }
    fn matches_regex(pattern: &str) -> regex::Error;
}

// ============================================================================
// EMAIL VALIDATOR
// ============================================================================

crate::validator! {
    /// Validates email format.
    ///
    /// Uses a simple but effective regex pattern.
    pub Email { pattern: regex::Regex } for str;
    rule(self, input) { self.pattern.is_match(input) }
    error(self, input) { ValidationError::invalid_format("", "email") }
    new() {
        Self {
            pattern: EMAIL_REGEX.clone(),
        }
    }
    fn email();
}

// ============================================================================
// URL VALIDATOR
// ============================================================================

crate::validator! {
    /// Validates URL format.
    pub Url { pattern: regex::Regex } for str;
    rule(self, input) { self.pattern.is_match(input) }
    error(self, input) { ValidationError::invalid_format("", "url") }
    new() {
        Self {
            pattern: URL_REGEX.clone(),
        }
    }
    fn url();
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validate;

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
