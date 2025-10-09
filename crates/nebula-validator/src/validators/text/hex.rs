//! Hexadecimal string validator.
//!
//! Validates that a string contains only valid hexadecimal characters.

use crate::core::{TypedValidator, ValidationComplexity, ValidationError, ValidatorMetadata};

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
/// use nebula_validator::core::TypedValidator;
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
    /// - allow_prefix: true (allows "0x" or "0X")
    /// - min_length: None (no minimum)
    /// - max_length: None (no maximum)
    /// - case_sensitive: None (allows any case)
    pub fn new() -> Self {
        Self {
            allow_prefix: true,
            min_length: None,
            max_length: None,
            case_sensitive: None,
        }
    }

    /// Disallow the "0x" prefix.
    pub fn no_prefix(mut self) -> Self {
        self.allow_prefix = false;
        self
    }

    /// Require the "0x" prefix.
    pub fn require_prefix(self) -> RequirePrefixHex {
        RequirePrefixHex {
            min_length: self.min_length,
            max_length: self.max_length,
            case_sensitive: self.case_sensitive,
        }
    }

    /// Sets minimum length for hex string (excluding prefix).
    pub fn min_length(mut self, min: usize) -> Self {
        self.min_length = Some(min);
        self
    }

    /// Sets maximum length for hex string (excluding prefix).
    pub fn max_length(mut self, max: usize) -> Self {
        self.max_length = Some(max);
        self
    }

    /// Only allow lowercase hex digits.
    pub fn lowercase_only(mut self) -> Self {
        self.case_sensitive = Some(HexCase::Lower);
        self
    }

    /// Only allow uppercase hex digits.
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

impl TypedValidator for Hex {
    type Input = str;
    type Output = Vec<u8>;
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<Self::Output, Self::Error> {
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
        if let Some(min) = self.min_length {
            if hex_str.len() < min {
                return Err(ValidationError::new(
                    "hex_too_short",
                    format!("Hex string must be at least {} characters", min),
                ));
            }
        }

        if let Some(max) = self.max_length {
            if hex_str.len() > max {
                return Err(ValidationError::new(
                    "hex_too_long",
                    format!("Hex string must not exceed {} characters", max),
                ));
            }
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
                    format!("Invalid hex character '{}'{}", c, case_hint),
                ));
            }
        }

        // Parse to bytes
        let bytes = (0..hex_str.len())
            .step_by(2)
            .map(|i| {
                let byte_str = if i + 1 < hex_str.len() {
                    &hex_str[i..i + 2]
                } else {
                    // Odd length - pad with 0
                    return u8::from_str_radix(&format!("0{}", &hex_str[i..i + 1]), 16).map_err(
                        |_| ValidationError::new("hex_parse_error", "Failed to parse hex string"),
                    );
                };
                u8::from_str_radix(byte_str, 16).map_err(|_| {
                    ValidationError::new("hex_parse_error", "Failed to parse hex string")
                })
            })
            .collect::<Result<Vec<u8>, ValidationError>>()?;

        Ok(bytes)
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Hex".to_string(),
            description: Some(format!(
                "Validates hexadecimal strings (prefix: {}, case: {:?})",
                if self.allow_prefix {
                    "optional"
                } else {
                    "not allowed"
                },
                self.case_sensitive.unwrap_or(HexCase::Mixed)
            )),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(5)),
            tags: vec![
                "text".to_string(),
                "hex".to_string(),
                "encoding".to_string(),
            ],
            version: Some("1.0.0".to_string()),
            custom: std::collections::HashMap::new(),
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
    pub fn min_length(mut self, min: usize) -> Self {
        self.min_length = Some(min);
        self
    }

    /// Sets maximum length for hex string (excluding prefix).
    pub fn max_length(mut self, max: usize) -> Self {
        self.max_length = Some(max);
        self
    }

    /// Only allow lowercase hex digits.
    pub fn lowercase_only(mut self) -> Self {
        self.case_sensitive = Some(HexCase::Lower);
        self
    }

    /// Only allow uppercase hex digits.
    pub fn uppercase_only(mut self) -> Self {
        self.case_sensitive = Some(HexCase::Upper);
        self
    }
}

impl TypedValidator for RequirePrefixHex {
    type Input = str;
    type Output = Vec<u8>;
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<Self::Output, Self::Error> {
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
            name: "RequirePrefixHex".to_string(),
            description: Some("Validates hexadecimal strings (requires '0x' prefix)".to_string()),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(5)),
            tags: vec![
                "text".to_string(),
                "hex".to_string(),
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
    fn test_output_bytes() {
        let validator = Hex::new();
        let result = validator.validate("deadbeef").unwrap();
        assert_eq!(result, vec![0xde, 0xad, 0xbe, 0xef]);

        let result = validator.validate("0x1234").unwrap();
        assert_eq!(result, vec![0x12, 0x34]);
    }

    #[test]
    fn test_odd_length() {
        let validator = Hex::new();
        let result = validator.validate("abc").unwrap();
        assert_eq!(result, vec![0x0a, 0xbc]); // Padded with 0
    }

    #[test]
    fn test_single_char() {
        let validator = Hex::new();
        let result = validator.validate("f").unwrap();
        assert_eq!(result, vec![0x0f]);
    }
}
