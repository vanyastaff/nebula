//! UUID validator for RFC 4122 UUIDs.
//!
//! Validates Universally Unique Identifiers (UUIDs) in standard format.

use crate::core::{TypedValidator, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// UUID VALIDATOR
// ============================================================================

/// Validates UUID strings according to RFC 4122.
///
/// Standard UUID format: `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`
/// where each `x` is a hexadecimal digit (0-9, a-f, A-F).
///
/// Supports all UUID versions:
/// - Version 1: Time-based
/// - Version 2: DCE Security
/// - Version 3: MD5 hash
/// - Version 4: Random
/// - Version 5: SHA-1 hash
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::Uuid;
/// use nebula_validator::core::TypedValidator;
///
/// let validator = Uuid::new();
///
/// // Valid UUIDs
/// assert!(validator.validate("123e4567-e89b-12d3-a456-426614174000").is_ok());
/// assert!(validator.validate("550e8400-e29b-41d4-a716-446655440000").is_ok());
/// assert!(validator.validate("f47ac10b-58cc-4372-a567-0e02b2c3d479").is_ok());
///
/// // Invalid
/// assert!(validator.validate("not-a-uuid").is_err());
/// assert!(validator.validate("123e4567-e89b-12d3-a456").is_err()); // too short
/// assert!(validator.validate("123e4567e89b12d3a456426614174000").is_err()); // no hyphens
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Uuid {
    allow_uppercase: bool,
    allow_lowercase: bool,
    allow_braces: bool,
    specific_version: Option<u8>,
}

impl Uuid {
    /// Creates a new UUID validator with default settings.
    ///
    /// Default settings:
    /// - `allow_uppercase`: true
    /// - `allow_lowercase`: true
    /// - `allow_braces`: false
    /// - `specific_version`: None (any version)
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_uppercase: true,
            allow_lowercase: true,
            allow_braces: false,
            specific_version: None,
        }
    }

    /// Only allow uppercase hex digits.
    #[must_use = "builder methods must be chained or built"]
    pub fn uppercase_only(mut self) -> Self {
        self.allow_lowercase = false;
        self
    }

    /// Only allow lowercase hex digits.
    #[must_use = "builder methods must be chained or built"]
    pub fn lowercase_only(mut self) -> Self {
        self.allow_uppercase = false;
        self
    }

    /// Allow UUID wrapped in braces: `{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}`.
    #[must_use = "builder methods must be chained or built"]
    pub fn allow_braces(mut self) -> Self {
        self.allow_braces = true;
        self
    }

    /// Only accept specific UUID version (1-5).
    #[must_use = "builder methods must be chained or built"]
    pub fn version(mut self, version: u8) -> Self {
        self.specific_version = Some(version);
        self
    }

    fn is_valid_hex_char(&self, c: char) -> bool {
        if c.is_ascii_digit() {
            return true;
        }
        if self.allow_lowercase && ('a'..='f').contains(&c) {
            return true;
        }
        if self.allow_uppercase && ('A'..='F').contains(&c) {
            return true;
        }
        false
    }

    fn extract_version(uuid: &str) -> Option<u8> {
        // Version is the first hex digit of the 3rd group (15th character)
        // Format: xxxxxxxx-xxxx-Vxxx-xxxx-xxxxxxxxxxxx
        //                        ^
        uuid.chars()
            .nth(14)
            .and_then(|c| c.to_digit(16))
            .map(|v| v as u8)
    }

    fn extract_variant(uuid: &str) -> Option<u8> {
        // Variant is indicated by the first hex digit of the 4th group (20th character)
        // Format: xxxxxxxx-xxxx-xxxx-Vxxx-xxxxxxxxxxxx
        //                             ^
        uuid.chars()
            .nth(19)
            .and_then(|c| c.to_digit(16))
            .map(|v| v as u8)
    }
}

impl Default for Uuid {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedValidator for Uuid {
    type Input = str;
    type Output = ();
    type Error = ValidationError;

    fn validate(&self, input: &str) -> Result<Self::Output, Self::Error> {
        if input.is_empty() {
            return Err(ValidationError::new(
                "empty_uuid",
                "UUID string cannot be empty",
            ));
        }

        // Strip braces if present
        let uuid = if input.starts_with('{') && input.ends_with('}') {
            if !self.allow_braces {
                return Err(ValidationError::new(
                    "braces_not_allowed",
                    "UUID with braces is not allowed",
                ));
            }
            &input[1..input.len() - 1]
        } else {
            input
        };

        // Check length
        if uuid.len() != 36 {
            return Err(ValidationError::new(
                "invalid_uuid_length",
                format!("UUID must be 36 characters long (found {})", uuid.len()),
            ));
        }

        // Check hyphen positions
        let expected_hyphens = [8, 13, 18, 23];
        for &pos in &expected_hyphens {
            if uuid.chars().nth(pos) != Some('-') {
                return Err(ValidationError::new(
                    "invalid_uuid_format",
                    format!("Expected hyphen at position {pos}"),
                ));
            }
        }

        // Validate hex characters
        for (i, c) in uuid.chars().enumerate() {
            if expected_hyphens.contains(&i) {
                continue; // Skip hyphens
            }

            if !self.is_valid_hex_char(c) {
                let case_hint = if !self.allow_uppercase {
                    " (lowercase only)"
                } else if !self.allow_lowercase {
                    " (uppercase only)"
                } else {
                    ""
                };
                return Err(ValidationError::new(
                    "invalid_uuid_char",
                    format!("Invalid UUID character '{c}' at position {i}{case_hint}"),
                ));
            }
        }

        // Check version if specified
        if let Some(expected_version) = self.specific_version {
            if let Some(version) = Self::extract_version(uuid) {
                if version != expected_version {
                    return Err(ValidationError::new(
                        "invalid_uuid_version",
                        format!("Expected UUID version {expected_version}, found {version}"),
                    ));
                }
            } else {
                return Err(ValidationError::new(
                    "invalid_uuid_version",
                    "Could not extract UUID version",
                ));
            }
        }

        // Validate RFC 4122 variant (bits 10xx in the variant field)
        // Special case: nil UUID (all zeros) is valid per RFC 4122 Section 4.1.7
        let is_nil = uuid.chars().all(|c| c == '0' || c == '-');
        if !is_nil {
            if let Some(variant) = Self::extract_variant(uuid) {
                // RFC 4122 variant has bits 10xx (decimal 8-11)
                if !(8..=11).contains(&variant) {
                    return Err(ValidationError::new(
                        "invalid_uuid_variant",
                        format!(
                            "Invalid UUID variant (expected RFC 4122, found variant bits: {variant:x})"
                        ),
                    ));
                }
            }
        }

        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Uuid".to_string(),
            description: Some(format!(
                "Validates RFC 4122 UUIDs (version: {:?}, braces: {})",
                self.specific_version
                    .map_or("any".to_string(), |v| v.to_string()),
                if self.allow_braces {
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
                "uuid".to_string(),
                "identifier".to_string(),
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
    fn test_valid_uuids() {
        let validator = Uuid::new();
        assert!(
            validator
                .validate("123e4567-e89b-12d3-a456-426614174000")
                .is_ok()
        );
        assert!(
            validator
                .validate("550e8400-e29b-41d4-a716-446655440000")
                .is_ok()
        );
        assert!(
            validator
                .validate("f47ac10b-58cc-4372-a567-0e02b2c3d479")
                .is_ok()
        );
        assert!(
            validator
                .validate("00000000-0000-0000-0000-000000000000")
                .is_ok()
        ); // nil UUID
    }

    #[test]
    fn test_uppercase_uuid() {
        let validator = Uuid::new();
        assert!(
            validator
                .validate("123E4567-E89B-12D3-A456-426614174000")
                .is_ok()
        );
        assert!(
            validator
                .validate("F47AC10B-58CC-4372-A567-0E02B2C3D479")
                .is_ok()
        );
    }

    #[test]
    fn test_mixed_case() {
        let validator = Uuid::new();
        assert!(
            validator
                .validate("123e4567-E89B-12d3-A456-426614174000")
                .is_ok()
        );
    }

    #[test]
    fn test_lowercase_only() {
        let validator = Uuid::new().lowercase_only();
        assert!(
            validator
                .validate("123e4567-e89b-12d3-a456-426614174000")
                .is_ok()
        );
        assert!(
            validator
                .validate("123E4567-E89B-12D3-A456-426614174000")
                .is_err()
        );
    }

    #[test]
    fn test_uppercase_only() {
        let validator = Uuid::new().uppercase_only();
        assert!(
            validator
                .validate("123E4567-E89B-12D3-A456-426614174000")
                .is_ok()
        );
        assert!(
            validator
                .validate("123e4567-e89b-12d3-a456-426614174000")
                .is_err()
        );
    }

    #[test]
    fn test_with_braces() {
        let validator = Uuid::new().allow_braces();
        assert!(
            validator
                .validate("{123e4567-e89b-12d3-a456-426614174000}")
                .is_ok()
        );
        assert!(
            validator
                .validate("123e4567-e89b-12d3-a456-426614174000")
                .is_ok()
        );
    }

    #[test]
    fn test_braces_not_allowed() {
        let validator = Uuid::new();
        assert!(
            validator
                .validate("{123e4567-e89b-12d3-a456-426614174000}")
                .is_err()
        );
    }

    #[test]
    fn test_invalid_format() {
        let validator = Uuid::new();
        assert!(
            validator
                .validate("123e4567e89b12d3a456426614174000")
                .is_err()
        ); // no hyphens
        assert!(validator.validate("123e4567-e89b-12d3-a456").is_err()); // too short
        assert!(
            validator
                .validate("123e4567-e89b-12d3-a456-426614174000-extra")
                .is_err()
        ); // too long
    }

    #[test]
    fn test_invalid_characters() {
        let validator = Uuid::new();
        assert!(
            validator
                .validate("123g4567-e89b-12d3-a456-426614174000")
                .is_err()
        ); // 'g' is not hex
        assert!(
            validator
                .validate("not-a-uuid-at-all-really-not-at-all")
                .is_err()
        );
    }

    #[test]
    fn test_version_specific() {
        let validator = Uuid::new().version(4);
        assert!(
            validator
                .validate("123e4567-e89b-42d3-a456-426614174000")
                .is_ok()
        ); // version 4
        assert!(
            validator
                .validate("123e4567-e89b-12d3-a456-426614174000")
                .is_err()
        ); // version 1
    }

    #[test]
    fn test_empty_string() {
        let validator = Uuid::new();
        assert!(validator.validate("").is_err());
    }

    #[test]
    fn test_real_world_uuids() {
        let validator = Uuid::new();
        // UUID v4 (random)
        assert!(
            validator
                .validate("550e8400-e29b-41d4-a716-446655440000")
                .is_ok()
        );
        // UUID v1 (time-based)
        assert!(
            validator
                .validate("6ba7b810-9dad-11d1-80b4-00c04fd430c8")
                .is_ok()
        );
        // UUID v5 (SHA-1 hash)
        assert!(
            validator
                .validate("886313e1-3b8a-5372-9b90-0c9aee199e5d")
                .is_ok()
        );
    }
}
