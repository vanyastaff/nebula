//! Base64 string validator.
//!
//! Validates that a string is properly encoded in Base64 format.

use crate::core::{ValidationComplexity, ValidationError, Validator, ValidatorMetadata};

// ============================================================================
// BASE64 VALIDATOR
// ============================================================================

/// Validates Base64-encoded strings.
///
/// Base64 encoding uses:
/// - Letters: A-Z, a-z
/// - Digits: 0-9
/// - Special chars: `+` and `/` (standard) or `-` and `_` (URL-safe)
/// - Padding: `=` (optional, depending on configuration)
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::Base64;
/// use nebula_validator::core::Validator;
///
/// let validator = Base64::new();
///
/// // Valid base64 strings
/// assert!(validator.validate("SGVsbG8gV29ybGQ=").is_ok());
/// assert!(validator.validate("YWJjZGVm").is_ok());
/// assert!(validator.validate("MTIzNDU2").is_ok());
///
/// // Invalid
/// assert!(validator.validate("Hello World!").is_err()); // invalid chars
/// assert!(validator.validate("SGVsbG8=World").is_err()); // padding in wrong place
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Base64 {
    alphabet: Base64Alphabet,
    require_padding: bool,
    allow_whitespace: bool,
}

/// Base64 alphabet variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base64Alphabet {
    /// Standard Base64 alphabet with `+` and `/`
    Standard,
    /// URL-safe Base64 alphabet with `-` and `_`
    UrlSafe,
    /// Allow both standard and URL-safe characters
    Mixed,
}

impl Base64 {
    /// Creates a new Base64 validator with default settings.
    ///
    /// Default settings:
    /// - alphabet: Standard
    /// - `require_padding`: false (padding is optional)
    /// - `allow_whitespace`: false
    #[must_use]
    pub fn new() -> Self {
        Self {
            alphabet: Base64Alphabet::Standard,
            require_padding: false,
            allow_whitespace: false,
        }
    }

    /// Use URL-safe alphabet (- and _ instead of + and /).
    #[must_use = "builder methods must be chained or built"]
    pub fn url_safe(mut self) -> Self {
        self.alphabet = Base64Alphabet::UrlSafe;
        self
    }

    /// Allow both standard and URL-safe characters.
    #[must_use = "builder methods must be chained or built"]
    pub fn mixed_alphabet(mut self) -> Self {
        self.alphabet = Base64Alphabet::Mixed;
        self
    }

    /// Require padding with `=` characters.
    #[must_use = "builder methods must be chained or built"]
    pub fn require_padding(mut self) -> Self {
        self.require_padding = true;
        self
    }

    /// Allow whitespace characters (space, tab, newline) in the input.
    /// Whitespace will be ignored during validation.
    #[must_use = "builder methods must be chained or built"]
    pub fn allow_whitespace(mut self) -> Self {
        self.allow_whitespace = true;
        self
    }

    fn is_base64_char(&self, c: char) -> bool {
        if c.is_ascii_alphanumeric() {
            return true;
        }

        match self.alphabet {
            Base64Alphabet::Standard => c == '+' || c == '/',
            Base64Alphabet::UrlSafe => c == '-' || c == '_',
            Base64Alphabet::Mixed => c == '+' || c == '/' || c == '-' || c == '_',
        }
    }

    fn is_whitespace(c: char) -> bool {
        c == ' ' || c == '\t' || c == '\n' || c == '\r'
    }
}

impl Default for Base64 {
    fn default() -> Self {
        Self::new()
    }
}

impl Validator for Base64 {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.is_empty() {
            return Err(ValidationError::new(
                "empty_base64",
                "Base64 string cannot be empty",
            ));
        }

        // Filter out whitespace if allowed
        let cleaned: String = if self.allow_whitespace {
            input.chars().filter(|c| !Self::is_whitespace(*c)).collect()
        } else {
            input.to_string()
        };

        if cleaned.is_empty() {
            return Err(ValidationError::new(
                "empty_base64",
                "Base64 string cannot be empty",
            ));
        }

        // Check for padding
        let padding_count = cleaned.chars().rev().take_while(|&c| c == '=').count();

        if self.require_padding {
            // When padding is required, total length (including padding) must be multiple of 4
            if cleaned.len() % 4 != 0 {
                return Err(ValidationError::new(
                    "invalid_base64_length",
                    "Base64 string length must be a multiple of 4 when padding is required",
                ));
            }

            // Calculate expected padding based on data length (without padding)
            let data_len = cleaned.len() - padding_count;
            let expected_padding = match data_len % 4 {
                0 => 0, // No padding needed
                2 => 2, // 2 data chars => 2 padding
                3 => 1, // 3 data chars => 1 padding
                _ => {
                    return Err(ValidationError::new(
                        "invalid_base64_length",
                        "Invalid Base64 data length",
                    ));
                }
            };

            if padding_count != expected_padding {
                return Err(ValidationError::new(
                    "invalid_base64_padding",
                    format!(
                        "Expected {expected_padding} padding characters, found {padding_count}"
                    ),
                ));
            }
        } else {
            // Padding is optional, but if present, must be valid (1 or 2 '=' at end)
            if padding_count > 2 {
                return Err(ValidationError::new(
                    "invalid_base64_padding",
                    "Too many padding characters (max 2)",
                ));
            }
        }

        // Validate characters (excluding padding)
        let base64_part = &cleaned[..cleaned.len() - padding_count];
        for (i, c) in base64_part.chars().enumerate() {
            if !self.is_base64_char(c) {
                let alphabet_hint = match self.alphabet {
                    Base64Alphabet::Standard => " (standard alphabet: A-Z, a-z, 0-9, +, /)",
                    Base64Alphabet::UrlSafe => " (URL-safe alphabet: A-Z, a-z, 0-9, -, _)",
                    Base64Alphabet::Mixed => " (mixed alphabet: A-Z, a-z, 0-9, +, /, -, _)",
                };
                return Err(ValidationError::new(
                    "invalid_base64_char",
                    format!("Invalid Base64 character '{c}' at position {i}{alphabet_hint}"),
                ));
            }
        }

        // Check if padding appears in the middle
        if padding_count > 0 && cleaned[..cleaned.len() - padding_count].contains('=') {
            return Err(ValidationError::new(
                "invalid_base64_padding",
                "Padding characters '=' must only appear at the end",
            ));
        }

        // Validate length (with padding, must be multiple of 4)
        if padding_count > 0 && !cleaned.len().is_multiple_of(4) {
            return Err(ValidationError::new(
                "invalid_base64_length",
                "Base64 string with padding must have length that is a multiple of 4",
            ));
        }

        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Base64".to_string(),
            description: Some(format!(
                "Validates Base64 strings (alphabet: {:?}, padding: {}, whitespace: {})",
                self.alphabet,
                if self.require_padding {
                    "required"
                } else {
                    "optional"
                },
                if self.allow_whitespace {
                    "allowed"
                } else {
                    "not allowed"
                }
            )),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(5)),
            tags: vec![
                "text".to_string(),
                "base64".to_string(),
                "encoding".to_string(),
            ],
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
    fn test_valid_base64() {
        let validator = Base64::new();
        assert!(validator.validate("SGVsbG8gV29ybGQ=").is_ok()); // "Hello World"
        assert!(validator.validate("YWJjZGVm").is_ok()); // "abcdef"
        assert!(validator.validate("MTIzNDU2").is_ok()); // "123456"
        assert!(validator.validate("YQ==").is_ok()); // "a"
        assert!(validator.validate("YWI=").is_ok()); // "ab"
    }

    #[test]
    fn test_invalid_base64() {
        let validator = Base64::new();
        assert!(validator.validate("").is_err());
        assert!(validator.validate("Hello World!").is_err());
        assert!(validator.validate("SGVsbG8=World").is_err()); // padding in middle
        assert!(validator.validate("ABC===").is_err()); // too much padding
    }

    #[test]
    fn test_url_safe() {
        let validator = Base64::new().url_safe();
        assert!(validator.validate("SGVsbG8tV29ybGQ").is_ok()); // with -
        assert!(validator.validate("SGVsbG8_V29ybGQ").is_ok()); // with _
        assert!(validator.validate("SGVsbG8+V29ybGQ").is_err()); // standard +
        assert!(validator.validate("SGVsbG8/V29ybGQ").is_err()); // standard /
    }

    #[test]
    fn test_mixed_alphabet() {
        let validator = Base64::new().mixed_alphabet();
        assert!(validator.validate("SGVsbG8+V29ybGQ").is_ok()); // standard +
        assert!(validator.validate("SGVsbG8/V29ybGQ").is_ok()); // standard /
        assert!(validator.validate("SGVsbG8-V29ybGQ").is_ok()); // URL-safe -
        assert!(validator.validate("SGVsbG8_V29ybGQ").is_ok()); // URL-safe _
    }

    #[test]
    fn test_require_padding() {
        let validator = Base64::new().require_padding();
        assert!(validator.validate("YWJj").is_ok()); // no padding needed
        assert!(validator.validate("YQ==").is_ok()); // 2 padding chars
        assert!(validator.validate("YWI=").is_ok()); // 1 padding char
        assert!(validator.validate("YQ").is_err()); // missing padding
        assert!(validator.validate("YWI").is_err()); // missing padding
    }

    #[test]
    fn test_optional_padding() {
        let validator = Base64::new();
        assert!(validator.validate("YWJj").is_ok()); // no padding
        assert!(validator.validate("YQ==").is_ok()); // with padding
        assert!(validator.validate("YQ").is_ok()); // without padding (optional)
        assert!(validator.validate("YWI").is_ok()); // without padding (optional)
    }

    #[test]
    fn test_allow_whitespace() {
        let validator = Base64::new().allow_whitespace();
        assert!(validator.validate("SGVs bG8g V29y bGQ=").is_ok());
        assert!(validator.validate("SGVs\nbG8g\tV29y\rbGQ=").is_ok());
        assert!(validator.validate("  YWJj  ").is_ok());

        let validator_no_ws = Base64::new();
        assert!(validator_no_ws.validate("SGVs bG8g V29y bGQ=").is_err());
    }

    #[test]
    fn test_empty_string() {
        let validator = Base64::new();
        assert!(validator.validate("").is_err());
    }

    #[test]
    fn test_only_whitespace() {
        let validator = Base64::new().allow_whitespace();
        assert!(validator.validate("   ").is_err());
    }

    #[test]
    fn test_padding_in_middle() {
        let validator = Base64::new();
        assert!(validator.validate("SGVs=bG8=").is_err());
        assert!(validator.validate("YQ=Z").is_err());
    }

    #[test]
    fn test_real_world_examples() {
        let validator = Base64::new();

        // JWT-like structure (without signature verification)
        assert!(
            validator
                .validate("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9")
                .is_ok()
        );

        // Image data
        assert!(validator.validate("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==").is_ok());

        // Binary data
        assert!(validator.validate("AQIDBAU=").is_ok());
    }
}
