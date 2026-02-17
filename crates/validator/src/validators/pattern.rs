//! String pattern validators
//!
//! This module provides validators for checking string patterns and formats.

use crate::foundation::ValidationError;

crate::validator! {
    /// Validates that a string contains a substring.
    #[derive(PartialEq, Eq, Hash)]
    pub Contains { substring: String } for str;
    rule(self, input) { input.contains(&self.substring) }
    error(self, input) {
        ValidationError::new(
            "contains",
            format!("String must contain '{}'", self.substring),
        )
        .with_param("substring", self.substring.clone())
    }
    new(substring: impl Into<String>) { Self { substring: substring.into() } }
    fn contains(substring: impl Into<String>);
}

crate::validator! {
    /// Validates that a string starts with a prefix.
    #[derive(PartialEq, Eq, Hash)]
    pub StartsWith { prefix: String } for str;
    rule(self, input) { input.starts_with(&self.prefix) }
    error(self, input) {
        ValidationError::new(
            "starts_with",
            format!("String must start with '{}'", self.prefix),
        )
        .with_param("prefix", self.prefix.clone())
    }
    new(prefix: impl Into<String>) { Self { prefix: prefix.into() } }
    fn starts_with(prefix: impl Into<String>);
}

crate::validator! {
    /// Validates that a string ends with a suffix.
    #[derive(PartialEq, Eq, Hash)]
    pub EndsWith { suffix: String } for str;
    rule(self, input) { input.ends_with(&self.suffix) }
    error(self, input) {
        ValidationError::new(
            "ends_with",
            format!("String must end with '{}'", self.suffix),
        )
        .with_param("suffix", self.suffix.clone())
    }
    new(suffix: impl Into<String>) { Self { suffix: suffix.into() } }
    fn ends_with(suffix: impl Into<String>);
}

crate::validator! {
    /// Validates that a string contains only alphanumeric characters.
    #[derive(Copy, PartialEq, Eq, Hash, Default)]
    pub Alphanumeric { allow_spaces: bool } for str;
    rule(self, input) {
        input.chars().all(|c| c.is_alphanumeric() || (self.allow_spaces && c.is_whitespace()))
    }
    error(self, input) {
        ValidationError::new("alphanumeric", if self.allow_spaces {
            "String must contain only letters, numbers, and spaces"
        } else {
            "String must contain only letters and numbers"
        })
    }
    new() { Self { allow_spaces: false } }
    fn alphanumeric();
}

crate::validator! {
    /// Validates that a string contains only alphabetic characters.
    #[derive(Copy, PartialEq, Eq, Hash, Default)]
    pub Alphabetic { allow_spaces: bool } for str;
    rule(self, input) {
        input.chars().all(|c| c.is_alphabetic() || (self.allow_spaces && c.is_whitespace()))
    }
    error(self, input) {
        ValidationError::new("alphabetic", if self.allow_spaces {
            "String must contain only letters and spaces"
        } else {
            "String must contain only letters"
        })
    }
    new() { Self { allow_spaces: false } }
    fn alphabetic();
}

// ============================================================================
// NUMERIC
// ============================================================================

crate::validator! {
    /// Validates that a string contains only numeric characters.
    pub Numeric for str;
    rule(input) { input.chars().all(char::is_numeric) }
    error(input) { ValidationError::new("numeric", "String must contain only numbers") }
    fn numeric();
}

crate::validator! {
    /// Validates that a string is lowercase.
    pub Lowercase for str;
    rule(input) { input.chars().all(|c| !c.is_alphabetic() || c.is_lowercase()) }
    error(input) { ValidationError::new("lowercase", "String must be lowercase") }
    fn lowercase();
}

crate::validator! {
    /// Validates that a string is uppercase.
    pub Uppercase for str;
    rule(input) { input.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()) }
    error(input) { ValidationError::new("uppercase", "String must be uppercase") }
    fn uppercase();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::Validate;

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
        let validator = Alphanumeric { allow_spaces: true };
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
