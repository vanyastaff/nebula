//! Hexadecimal string validator.
//!
//! Validates that a string contains only valid hexadecimal characters.

use crate::core::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// HEX VALIDATOR
// ============================================================================

/// Validates hexadecimal strings.
///
/// A hexadecimal string can contain:
/// - Digits: 0-9
/// - Letters: a-f (lowercase) or A-F (uppercase)
/// - Optional "0x" or "0X" prefix
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::Hex;
/// use nebula_validator::core::Validate;
///
/// let validator = Hex::new();
///
/// // Valid hex strings
/// assert!(validator.validate("deadbeef").is_ok());
/// assert!(validator.validate("DEADBEEF").is_ok());
/// assert!(validator.validate("0x1234").is_ok());
/// assert!(validator.validate("0X5678").is_ok());
/// assert!(validator.validate("a1b2c3").is_ok());
///
/// // Invalid
/// assert!(validator.validate("xyz").is_err()); // invalid chars
/// assert!(validator.validate("12 34").is_err()); // whitespace
/// assert!(validator.validate("").is_err()); // empty
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Hex {
    allow_prefix: bool,
    min_length: Option<usize>,
    max_length: Option<usize>,
    case_sensitive: Option<HexCase>,
}

/// Case sensitivity options for hex validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HexCase {
    /// Only lowercase hex digits (a-f)
    Lower,
    /// Only uppercase hex digits (A-F)
    Upper,
    /// Allow both lowercase and uppercase
    Mixed,
}

impl Hex {
    /// Creates a new hex validator with default settings.
    ///
    /// Default settings:
    /// - `allow_prefix`: true (allows "0x" or "0X")
    /// - `min_length`: None (no minimum)
    /// - `max_length`: None (no maximum)
    /// - `case_sensitive`: None (allows any case)
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_prefix: true,
            min_length: None,
            max_length: None,
            case_sensitive: None,
        }
    }

    /// Disallow the "0x" prefix.
    #[must_use = "builder methods must be chained or built"]
    pub fn no_prefix(mut self) -> Self {
        self.allow_prefix = false;
        self
    }

    /// Require the "0x" prefix.
    #[must_use]
    pub fn require_prefix(self) -> RequirePrefixHex {
        RequirePrefixHex {
            min_length: self.min_length,
            max_length: self.max_length,
            case_sensitive: self.case_sensitive,
        }
    }

    /// Sets minimum length for hex string (excluding prefix).
    #[must_use = "builder methods must be chained or built"]
    pub fn min_length(mut self, min: usize) -> Self {
        self.min_length = Some(min);
        self
    }

    /// Sets maximum length for hex string (excluding prefix).
    #[must_use = "builder methods must be chained or built"]
    pub fn max_length(mut self, max: usize) -> Self {
        self.max_length = Some(max);
        self
    }

    /// Only allow lowercase hex digits.
    #[must_use = "builder methods must be chained or built"]
    pub fn lowercase_only(mut self) -> Self {
        self.case_sensitive = Some(HexCase::Lower);
        self
    }

    /// Only allow uppercase hex digits.
    #[must_use = "builder methods must be chained or built"]
    pub fn uppercase_only(mut self) -> Self {
        self.case_sensitive = Some(HexCase::Upper);
        self
    }

    fn is_hex_char(c: char, case: Option<HexCase>) -> bool {
        match case {
            Some(HexCase::Lower) => c.is_ascii_digit() || ('a'..='f').contains(&c),
            Some(HexCase::Upper) => c.is_ascii_digit() || ('A'..='F').contains(&c),
            Some(HexCase::Mixed) | None => c.is_ascii_hexdigit(),
        }
    }
}

impl Default for Hex {
    fn default() -> Self {
        Self::new()
    }
}

impl Validate for Hex {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.is_empty() {
            return Err(ValidationError::new(
                "empty_hex",
                "Hex string cannot be empty",
            ));
        }

        // Strip prefix if present
        let hex_str = if input.starts_with("0x") || input.starts_with("0X") {
            if !self.allow_prefix {
                return Err(ValidationError::new(
                    "hex_prefix_not_allowed",
                    "Hex prefix '0x' is not allowed",
                ));
            }
            &input[2..]
        } else {
            input
        };

        // Check if empty after prefix removal
        if hex_str.is_empty() {
            return Err(ValidationError::new(
                "empty_hex",
                "Hex string cannot be just a prefix",
            ));
        }

        // Check length constraints
        if let Some(min) = self.min_length
            && hex_str.len() < min
        {
            return Err(ValidationError::new(
                "hex_too_short",
                format!("Hex string must be at least {min} characters"),
            ));
        }

        if let Some(max) = self.max_length
            && hex_str.len() > max
        {
            return Err(ValidationError::new(
                "hex_too_long",
                format!("Hex string must not exceed {max} characters"),
            ));
        }

        // Validate characters
        for c in hex_str.chars() {
            if !Self::is_hex_char(c, self.case_sensitive) {
                let case_hint = match self.case_sensitive {
                    Some(HexCase::Lower) => " (lowercase only)",
                    Some(HexCase::Upper) => " (uppercase only)",
                    _ => "",
                };
                return Err(ValidationError::new(
                    "invalid_hex_char",
                    format!("Invalid hex character '{c}'{case_hint}"),
                ));
            }
        }

        // Validate that hex string can be parsed (check each byte pair)
        for i in (0..hex_str.len()).step_by(2) {
            let byte_str = if i + 1 < hex_str.len() {
                &hex_str[i..i + 2]
            } else {
                // Odd length - pad with 0
                let padded = format!("0{}", &hex_str[i..=i]);
                u8::from_str_radix(&padded, 16).map_err(|_| {
                    ValidationError::new("hex_parse_error", "Failed to parse hex string")
                })?;
                continue;
            };
            u8::from_str_radix(byte_str, 16).map_err(|_| {
                ValidationError::new("hex_parse_error", "Failed to parse hex string")
            })?;
        }

        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Hex".into(),
            description: Some(
                format!(
                    "Validates hexadecimal strings (prefix: {}, case: {:?})",
                    if self.allow_prefix {
                        "optional"
                    } else {
                        "not allowed"
                    },
                    self.case_sensitive.unwrap_or(HexCase::Mixed)
                )
                .into(),
            ),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(5)),
            tags: vec!["text".into(), "hex".into(), "encoding".into()],
            version: Some("1.0.0".into()),
            custom: Vec::new(),
        }
    }
}

// ============================================================================
// REQUIRE PREFIX HEX
// ============================================================================

/// Hex validator that requires "0x" prefix.
#[derive(Debug, Clone, Copy)]
pub struct RequirePrefixHex {
    min_length: Option<usize>,
    max_length: Option<usize>,
    case_sensitive: Option<HexCase>,
}

impl RequirePrefixHex {
    /// Sets minimum length for hex string (excluding prefix).
    #[must_use = "builder methods must be chained or built"]
    pub fn min_length(mut self, min: usize) -> Self {
        self.min_length = Some(min);
        self
    }

    /// Sets maximum length for hex string (excluding prefix).
    #[must_use = "builder methods must be chained or built"]
    pub fn max_length(mut self, max: usize) -> Self {
        self.max_length = Some(max);
        self
    }

    /// Only allow lowercase hex digits.
    #[must_use = "builder methods must be chained or built"]
    pub fn lowercase_only(mut self) -> Self {
        self.case_sensitive = Some(HexCase::Lower);
        self
    }

    /// Only allow uppercase hex digits.
    #[must_use = "builder methods must be chained or built"]
    pub fn uppercase_only(mut self) -> Self {
        self.case_sensitive = Some(HexCase::Upper);
        self
    }
}

impl Validate for RequirePrefixHex {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if !input.starts_with("0x") && !input.starts_with("0X") {
            return Err(ValidationError::new(
                "hex_prefix_required",
                "Hex string must start with '0x' or '0X'",
            ));
        }

        // Use the regular Hex validator logic
        let validator = Hex {
            allow_prefix: true,
            min_length: self.min_length,
            max_length: self.max_length,
            case_sensitive: self.case_sensitive,
        };

        validator.validate(input)
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "RequirePrefixHex".into(),
            description: Some("Validates hexadecimal strings (requires '0x' prefix)".into()),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(5)),
            tags: vec!["text".into(), "hex".into(), "encoding".into()],
            version: Some("1.0.0".into()),
            custom: Vec::new(),
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
    fn test_valid_hex() {
        let validator = Hex::new();
        assert!(validator.validate("deadbeef").is_ok());
        assert!(validator.validate("DEADBEEF").is_ok());
        assert!(validator.validate("123456").is_ok());
        assert!(validator.validate("0x1234").is_ok());
        assert!(validator.validate("0X5678").is_ok());
        assert!(validator.validate("a1b2c3").is_ok());
    }

    #[test]
    fn test_invalid_hex() {
        let validator = Hex::new();
        assert!(validator.validate("xyz").is_err());
        assert!(validator.validate("12 34").is_err());
        assert!(validator.validate("").is_err());
        assert!(validator.validate("0x").is_err());
    }

    #[test]
    fn test_no_prefix() {
        let validator = Hex::new().no_prefix();
        assert!(validator.validate("deadbeef").is_ok());
        assert!(validator.validate("0xdeadbeef").is_err());
    }

    #[test]
    fn test_require_prefix() {
        let validator = Hex::new().require_prefix();
        assert!(validator.validate("0xdeadbeef").is_ok());
        assert!(validator.validate("0Xdeadbeef").is_ok());
        assert!(validator.validate("deadbeef").is_err());
    }

    #[test]
    fn test_length_constraints() {
        let validator = Hex::new().min_length(4).max_length(8);
        assert!(validator.validate("ab").is_err()); // too short
        assert!(validator.validate("abcd").is_ok()); // min
        assert!(validator.validate("abcdef01").is_ok()); // max
        assert!(validator.validate("abcdef0123").is_err()); // too long
    }

    #[test]
    fn test_lowercase_only() {
        let validator = Hex::new().lowercase_only();
        assert!(validator.validate("deadbeef").is_ok());
        assert!(validator.validate("DEADBEEF").is_err());
        assert!(validator.validate("DeAdBeEf").is_err());
    }

    #[test]
    fn test_uppercase_only() {
        let validator = Hex::new().uppercase_only();
        assert!(validator.validate("DEADBEEF").is_ok());
        assert!(validator.validate("deadbeef").is_err());
        assert!(validator.validate("DeAdBeEf").is_err());
    }

    #[test]
    fn test_output_unit() {
        let validator = Hex::new();
        let result = validator.validate("deadbeef").unwrap();
        assert_eq!(result, ());

        let result = validator.validate("0x1234").unwrap();
        assert_eq!(result, ());
    }

    #[test]
    fn test_odd_length() {
        let validator = Hex::new();
        // "abc" -> "ab" (0xab), then "c" padded to "0c" (0x0c) - should validate OK
        assert!(validator.validate("abc").is_ok());
    }

    #[test]
    fn test_single_char() {
        let validator = Hex::new();
        // Single char "f" padded to "0f" - should validate OK
        assert!(validator.validate("f").is_ok());
    }
}
