//! MAC Address validator.
//!
//! Validates MAC addresses in various formats (colon, hyphen, dot notation).

use crate::foundation::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// MAC ADDRESS VALIDATOR
// ============================================================================

/// Validates MAC addresses in various formats.
///
/// Supported formats:
/// - Colon-separated: `AA:BB:CC:DD:EE:FF`
/// - Hyphen-separated: `AA-BB-CC-DD-EE-FF`
/// - Dot-separated (Cisco): `AABB.CCDD.EEFF`
/// - No separator: `AABBCCDDEEFF`
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::MacAddress;
/// use nebula_validator::foundation::Validate;
///
/// let validator = MacAddress::new();
///
/// // Valid formats
/// assert!(validator.validate("AA:BB:CC:DD:EE:FF").is_ok());
/// assert!(validator.validate("aa:bb:cc:dd:ee:ff").is_ok());
/// assert!(validator.validate("AA-BB-CC-DD-EE-FF").is_ok());
/// assert!(validator.validate("AABB.CCDD.EEFF").is_ok());
/// assert!(validator.validate("AABBCCDDEEFF").is_ok());
///
/// // Invalid
/// assert!(validator.validate("GG:HH:II:JJ:KK:LL").is_err());
/// assert!(validator.validate("AA:BB:CC").is_err());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct MacAddress {
    allow_colon: bool,
    allow_hyphen: bool,
    allow_dot: bool,
    allow_no_separator: bool,
}

impl MacAddress {
    /// Creates a new MAC address validator (allows all formats).
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_colon: true,
            allow_hyphen: true,
            allow_dot: true,
            allow_no_separator: true,
        }
    }

    /// Only allow colon-separated format (AA:BB:CC:DD:EE:FF).
    #[must_use = "builder methods must be chained or built"]
    pub fn colon_only(mut self) -> Self {
        self.allow_colon = true;
        self.allow_hyphen = false;
        self.allow_dot = false;
        self.allow_no_separator = false;
        self
    }

    /// Only allow hyphen-separated format (AA-BB-CC-DD-EE-FF).
    #[must_use = "builder methods must be chained or built"]
    pub fn hyphen_only(mut self) -> Self {
        self.allow_colon = false;
        self.allow_hyphen = true;
        self.allow_dot = false;
        self.allow_no_separator = false;
        self
    }

    /// Only allow dot-separated format (AABB.CCDD.EEFF).
    #[must_use = "builder methods must be chained or built"]
    pub fn dot_only(mut self) -> Self {
        self.allow_colon = false;
        self.allow_hyphen = false;
        self.allow_dot = true;
        self.allow_no_separator = false;
        self
    }

    /// Only allow no-separator format (AABBCCDDEEFF).
    #[must_use = "builder methods must be chained or built"]
    pub fn no_separator_only(mut self) -> Self {
        self.allow_colon = false;
        self.allow_hyphen = false;
        self.allow_dot = false;
        self.allow_no_separator = true;
        self
    }

    fn validate_colon_format(&self, input: &str) -> Result<(), ValidationError> {
        let parts: Vec<&str> = input.split(':').collect();
        if parts.len() != 6 {
            return Err(ValidationError::new(
                "invalid_mac_format",
                "MAC address with colons must have 6 parts (AA:BB:CC:DD:EE:FF)",
            ));
        }
        self.parse_hex_parts(&parts)
    }

    fn validate_hyphen_format(&self, input: &str) -> Result<(), ValidationError> {
        let parts: Vec<&str> = input.split('-').collect();
        if parts.len() != 6 {
            return Err(ValidationError::new(
                "invalid_mac_format",
                "MAC address with hyphens must have 6 parts (AA-BB-CC-DD-EE-FF)",
            ));
        }
        self.parse_hex_parts(&parts)
    }

    fn validate_dot_format(&self, input: &str) -> Result<(), ValidationError> {
        let parts: Vec<&str> = input.split('.').collect();
        if parts.len() != 3 {
            return Err(ValidationError::new(
                "invalid_mac_format",
                "MAC address with dots must have 3 parts (AABB.CCDD.EEFF)",
            ));
        }

        for part in parts.iter() {
            if part.len() != 4 {
                return Err(ValidationError::new(
                    "invalid_mac_format",
                    "Each dot-separated part must be 4 hex digits",
                ));
            }

            u8::from_str_radix(&part[0..2], 16).map_err(|_| {
                ValidationError::new("invalid_hex", format!("Invalid hex digits: {part}"))
            })?;

            u8::from_str_radix(&part[2..4], 16).map_err(|_| {
                ValidationError::new("invalid_hex", format!("Invalid hex digits: {part}"))
            })?;
        }

        Ok(())
    }

    fn validate_no_separator_format(&self, input: &str) -> Result<(), ValidationError> {
        if input.len() != 12 {
            return Err(ValidationError::new(
                "invalid_mac_format",
                "MAC address without separators must be exactly 12 hex digits",
            ));
        }

        for i in 0..6 {
            u8::from_str_radix(&input[i * 2..i * 2 + 2], 16).map_err(|_| {
                ValidationError::new(
                    "invalid_hex",
                    format!("Invalid hex digits at position {}", i * 2),
                )
            })?;
        }

        Ok(())
    }

    fn parse_hex_parts(&self, parts: &[&str]) -> Result<(), ValidationError> {
        for part in parts.iter() {
            if part.len() != 2 {
                return Err(ValidationError::new(
                    "invalid_mac_format",
                    format!("Each part must be exactly 2 hex digits, got '{part}'"),
                ));
            }

            u8::from_str_radix(part, 16).map_err(|_| {
                ValidationError::new("invalid_hex", format!("Invalid hex digits: {part}"))
            })?;
        }
        Ok(())
    }
}

impl Default for MacAddress {
    fn default() -> Self {
        Self::new()
    }
}

impl Validate for MacAddress {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.is_empty() {
            return Err(ValidationError::new(
                "empty_input",
                "MAC address cannot be empty",
            ));
        }

        // Try different formats
        if input.contains(':') && self.allow_colon {
            return self.validate_colon_format(input);
        }

        if input.contains('-') && self.allow_hyphen {
            return self.validate_hyphen_format(input);
        }

        if input.contains('.') && self.allow_dot {
            return self.validate_dot_format(input);
        }

        if !input.contains(':')
            && !input.contains('-')
            && !input.contains('.')
            && self.allow_no_separator
        {
            return self.validate_no_separator_format(input);
        }

        // Format not allowed
        Err(ValidationError::new(
            "format_not_allowed",
            "MAC address format is not allowed by current configuration",
        ))
    }

    fn metadata(&self) -> ValidatorMetadata {
        let mut formats = Vec::new();
        if self.allow_colon {
            formats.push("colon");
        }
        if self.allow_hyphen {
            formats.push("hyphen");
        }
        if self.allow_dot {
            formats.push("dot");
        }
        if self.allow_no_separator {
            formats.push("no-separator");
        }

        ValidatorMetadata {
            name: "MacAddress".into(),
            description: Some(
                format!("Validates MAC addresses (formats: {})", formats.join(", ")).into(),
            ),
            complexity: ValidationComplexity::Constant,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(10)),
            tags: vec!["network".into(), "mac".into(), "hardware".into()],
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
    fn test_colon_format_valid() {
        let validator = MacAddress::new();
        assert!(validator.validate("AA:BB:CC:DD:EE:FF").is_ok());
        assert!(validator.validate("aa:bb:cc:dd:ee:ff").is_ok());
        assert!(validator.validate("00:11:22:33:44:55").is_ok());
        assert!(validator.validate("FF:FF:FF:FF:FF:FF").is_ok());
    }

    #[test]
    fn test_hyphen_format_valid() {
        let validator = MacAddress::new();
        assert!(validator.validate("AA-BB-CC-DD-EE-FF").is_ok());
        assert!(validator.validate("aa-bb-cc-dd-ee-ff").is_ok());
    }

    #[test]
    fn test_dot_format_valid() {
        let validator = MacAddress::new();
        assert!(validator.validate("AABB.CCDD.EEFF").is_ok());
        assert!(validator.validate("aabb.ccdd.eeff").is_ok());
    }

    #[test]
    fn test_no_separator_format_valid() {
        let validator = MacAddress::new();
        assert!(validator.validate("AABBCCDDEEFF").is_ok());
        assert!(validator.validate("aabbccddeeff").is_ok());
        assert!(validator.validate("001122334455").is_ok());
    }

    #[test]
    fn test_invalid_hex_chars() {
        let validator = MacAddress::new();
        assert!(validator.validate("GG:HH:II:JJ:KK:LL").is_err());
        assert!(validator.validate("ZZ:ZZ:ZZ:ZZ:ZZ:ZZ").is_err());
    }

    #[test]
    fn test_invalid_length() {
        let validator = MacAddress::new();
        assert!(validator.validate("AA:BB:CC").is_err());
        assert!(validator.validate("AA:BB:CC:DD:EE:FF:00").is_err());
        assert!(validator.validate("AABBCC").is_err());
    }

    #[test]
    fn test_empty_input() {
        let validator = MacAddress::new();
        assert!(validator.validate("").is_err());
    }

    #[test]
    fn test_colon_only() {
        let validator = MacAddress::new().colon_only();
        assert!(validator.validate("AA:BB:CC:DD:EE:FF").is_ok());
        assert!(validator.validate("AA-BB-CC-DD-EE-FF").is_err());
        assert!(validator.validate("AABB.CCDD.EEFF").is_err());
        assert!(validator.validate("AABBCCDDEEFF").is_err());
    }

    #[test]
    fn test_hyphen_only() {
        let validator = MacAddress::new().hyphen_only();
        assert!(validator.validate("AA-BB-CC-DD-EE-FF").is_ok());
        assert!(validator.validate("AA:BB:CC:DD:EE:FF").is_err());
    }

    #[test]
    fn test_dot_only() {
        let validator = MacAddress::new().dot_only();
        assert!(validator.validate("AABB.CCDD.EEFF").is_ok());
        assert!(validator.validate("AA:BB:CC:DD:EE:FF").is_err());
    }

    #[test]
    fn test_no_separator_only() {
        let validator = MacAddress::new().no_separator_only();
        assert!(validator.validate("AABBCCDDEEFF").is_ok());
        assert!(validator.validate("AA:BB:CC:DD:EE:FF").is_err());
    }

    #[test]
    fn test_output_unit() {
        let validator = MacAddress::new();
        validator.validate("01:23:45:67:89:AB").unwrap();
        assert_eq!((), ());
    }
}
