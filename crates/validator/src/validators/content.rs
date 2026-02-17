//! String content validators
//!
//! This module provides validators for checking string content against
//! common patterns like email addresses and URLs.
//!
//! # Validators
//!
//! - [`MatchesRegex`] - Validates that a string matches a regular expression
//! - [`Email`] - Validates email format
//! - [`Url`] - Validates URL format
//!
//! # Examples
//!
//! ```rust,ignore
//! use nebula_validator::prelude::*;
//!
//! // Email validation
//! let validator = email();
//! assert!(validator.validate("user@example.com").is_ok());
//!
//! // URL validation
//! let validator = url();
//! assert!(validator.validate("https://example.com").is_ok());
//!
//! // Custom regex pattern
//! let validator = matches_regex(r"^\d{3}-\d{4}$").unwrap();
//! assert!(validator.validate("123-4567").is_ok());
//! ```

use std::sync::LazyLock;

use crate::foundation::ValidationError;

static EMAIL_REGEX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
        r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$"
    ).unwrap()
});

static URL_REGEX: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^https?://[^\s/$.?#]+\.[^\s]+$").unwrap());

crate::validator! {
    /// Validates that a string matches a regular expression.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::validators::matches_regex;
    /// use nebula_validator::foundation::Validate;
    ///
    /// let validator = matches_regex(r"^\d{3}-\d{4}$").unwrap();
    /// assert!(validator.validate("123-4567").is_ok());
    /// assert!(validator.validate("invalid").is_err());
    /// ```
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

crate::validator! {
    /// Validates email format.
    ///
    /// Uses a simple but effective regex pattern that checks for basic
    /// email structure (local part @ domain).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::validators::email;
    /// use nebula_validator::foundation::Validate;
    ///
    /// let validator = email();
    /// assert!(validator.validate("user@example.com").is_ok());
    /// assert!(validator.validate("invalid").is_err());
    /// ```
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

crate::validator! {
    /// Validates URL format.
    ///
    /// Validates HTTP and HTTPS URLs using a regex pattern.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use nebula_validator::validators::url;
    /// use nebula_validator::foundation::Validate;
    ///
    /// let validator = url();
    /// assert!(validator.validate("https://example.com").is_ok());
    /// assert!(validator.validate("http://example.com/path").is_ok());
    /// assert!(validator.validate("invalid").is_err());
    /// ```
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
