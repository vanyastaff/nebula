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
//! ```rust
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

use std::sync::OnceLock;

use crate::foundation::{Validate, ValidationError};

/// Email regex pattern (shared with `Rule::Email` in `rule.rs`).
pub(crate) const EMAIL_PATTERN: &str = r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$";

/// URL regex pattern (shared with `Rule::Url` in `rule.rs`).
pub(crate) const URL_PATTERN: &str = r"^https?://[^\s/$.?#]+\.[^\s]+$";

/// Compile a fixed crate pattern once, without panicking on failure.
///
/// A failure here means a build-time regression in one of the constants above;
/// the `built_in_patterns_compile` test catches that in CI. Returning the
/// failure keeps the library panic-free anyway: a validator built on a broken
/// pattern reports `unavailable` (a structural diagnostic) rather than aborting
/// the process or silently rejecting every input.
fn compile_fixed(
    slot: &'static OnceLock<Result<regex::Regex, String>>,
    pattern: &'static str,
) -> Result<&'static regex::Regex, ValidationError> {
    match slot.get_or_init(|| regex::Regex::new(pattern).map_err(|error| error.to_string())) {
        Ok(regex) => Ok(regex),
        Err(_) => Err(ValidationError::unavailable(
            "the built-in validation pattern failed to compile",
        )),
    }
}

/// The shared compiled [`EMAIL_PATTERN`], resolved on first use.
pub(crate) fn email_regex() -> Result<&'static regex::Regex, ValidationError> {
    static REGEX: OnceLock<Result<regex::Regex, String>> = OnceLock::new();
    compile_fixed(&REGEX, EMAIL_PATTERN)
}

/// The shared compiled [`URL_PATTERN`], resolved on first use.
pub(crate) fn url_regex() -> Result<&'static regex::Regex, ValidationError> {
    static REGEX: OnceLock<Result<regex::Regex, String>> = OnceLock::new();
    compile_fixed(&REGEX, URL_PATTERN)
}

crate::validator! {
    /// Validates that a string matches a regular expression.
    ///
    /// # Examples
    ///
    /// ```rust
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

/// Validates email format.
///
/// Checks basic email structure (local part @ domain). The pattern is compiled
/// once per process on first use; a compile failure surfaces as an
/// `unavailable` diagnostic rather than a panic or a silent rejection.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::validators::email;
/// use nebula_validator::foundation::Validate;
///
/// let validator = email();
/// assert!(validator.validate("user@example.com").is_ok());
/// assert!(validator.validate("invalid").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Email;

impl Validate<str> for Email {
    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if email_regex()?.is_match(input) {
            Ok(())
        } else {
            Err(ValidationError::invalid_format("", "email"))
        }
    }
}

/// Creates an email-format validator.
#[must_use]
pub const fn email() -> Email {
    Email
}

/// Validates URL format.
///
/// Accepts HTTP and HTTPS URLs. The pattern is compiled once per process on
/// first use; a compile failure surfaces as an `unavailable` diagnostic rather
/// than a panic or a silent rejection.
///
/// # Examples
///
/// ```rust
/// use nebula_validator::validators::url;
/// use nebula_validator::foundation::Validate;
///
/// let validator = url();
/// assert!(validator.validate("https://example.com").is_ok());
/// assert!(validator.validate("http://example.com/path").is_ok());
/// assert!(validator.validate("invalid").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Url;

impl Validate<str> for Url {
    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if url_regex()?.is_match(input) {
            Ok(())
        } else {
            Err(ValidationError::invalid_format("", "url"))
        }
    }
}

/// Creates a URL-format validator.
#[must_use]
pub const fn url() -> Url {
    Url
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

#[cfg(test)]
mod builtin_pattern_tests {
    use super::*;

    /// A constant that no longer compiles must fail here, not at runtime.
    #[test]
    fn built_in_patterns_compile() {
        assert!(email_regex().is_ok(), "EMAIL_PATTERN must compile");
        assert!(url_regex().is_ok(), "URL_PATTERN must compile");
    }
}
