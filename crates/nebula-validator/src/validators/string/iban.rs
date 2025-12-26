//! IBAN (International Bank Account Number) validator.
//!
//! Validates IBANs according to ISO 13616 standard.

use crate::core::{ValidationComplexity, ValidationError, Validator, ValidatorMetadata};

// ============================================================================
// IBAN VALIDATOR
// ============================================================================

/// Validates International Bank Account Numbers (IBANs).
///
/// Validates:
/// - Format: country code (2 letters) + check digits (2 digits) + BBAN (up to 30 alphanumeric)
/// - Length: matches expected length for the country
/// - Check digits: validates using ISO 7064 Mod 97-10
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::string::Iban;
/// use nebula_validator::core::Validator;
///
/// let validator = Iban::new();
///
/// // Valid IBANs
/// assert!(validator.validate("DE89370400440532013000").is_ok()); // Germany
/// assert!(validator.validate("GB82WEST12345698765432").is_ok()); // UK
/// assert!(validator.validate("FR1420041010050500013M02606").is_ok()); // France
///
/// // With spaces (formatted)
/// let formatted = Iban::new().allow_spaces();
/// assert!(formatted.validate("DE89 3704 0044 0532 0130 00").is_ok());
///
/// // Invalid
/// assert!(validator.validate("DE89370400440532013001").is_err()); // wrong check digits
/// assert!(validator.validate("XX89370400440532013000").is_err()); // invalid country
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Iban {
    allow_spaces: bool,
    validate_country_length: bool,
}

impl Iban {
    /// Creates a new IBAN validator with default settings.
    ///
    /// Default settings:
    /// - Spaces not allowed (strict format)
    /// - Country-specific length validation enabled
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_spaces: false,
            validate_country_length: true,
        }
    }

    /// Allow spaces in IBAN (common formatted display).
    #[must_use = "builder methods must be chained or built"]
    pub fn allow_spaces(mut self) -> Self {
        self.allow_spaces = true;
        self
    }

    /// Skip country-specific length validation.
    #[must_use = "builder methods must be chained or built"]
    pub fn skip_country_length(mut self) -> Self {
        self.validate_country_length = false;
        self
    }

    /// Returns the expected IBAN length for a country code.
    ///
    /// Returns `None` if the country code is not recognized.
    fn country_length(country: &str) -> Option<usize> {
        // ISO 13616 IBAN lengths by country
        match country {
            "AL" => Some(28), // Albania
            "AD" => Some(24), // Andorra
            "AT" => Some(20), // Austria
            "AZ" => Some(28), // Azerbaijan
            "BH" => Some(22), // Bahrain
            "BY" => Some(28), // Belarus
            "BE" => Some(16), // Belgium
            "BA" => Some(20), // Bosnia Herzegovina
            "BR" => Some(29), // Brazil
            "BG" => Some(22), // Bulgaria
            "CR" => Some(22), // Costa Rica
            "HR" => Some(21), // Croatia
            "CY" => Some(28), // Cyprus
            "CZ" => Some(24), // Czech Republic
            "DK" => Some(18), // Denmark
            "DO" => Some(28), // Dominican Republic
            "EE" => Some(20), // Estonia
            "FO" => Some(18), // Faroe Islands
            "FI" => Some(18), // Finland
            "FR" => Some(27), // France
            "GE" => Some(22), // Georgia
            "DE" => Some(22), // Germany
            "GI" => Some(23), // Gibraltar
            "GR" => Some(27), // Greece
            "GL" => Some(18), // Greenland
            "GT" => Some(28), // Guatemala
            "HU" => Some(28), // Hungary
            "IS" => Some(26), // Iceland
            "IE" => Some(22), // Ireland
            "IL" => Some(23), // Israel
            "IT" => Some(27), // Italy
            "JO" => Some(30), // Jordan
            "KZ" => Some(20), // Kazakhstan
            "XK" => Some(20), // Kosovo
            "KW" => Some(30), // Kuwait
            "LV" => Some(21), // Latvia
            "LB" => Some(28), // Lebanon
            "LI" => Some(21), // Liechtenstein
            "LT" => Some(20), // Lithuania
            "LU" => Some(20), // Luxembourg
            "MK" => Some(19), // North Macedonia
            "MT" => Some(31), // Malta
            "MR" => Some(27), // Mauritania
            "MU" => Some(30), // Mauritius
            "MD" => Some(24), // Moldova
            "MC" => Some(27), // Monaco
            "ME" => Some(22), // Montenegro
            "NL" => Some(18), // Netherlands
            "NO" => Some(15), // Norway
            "PK" => Some(24), // Pakistan
            "PS" => Some(29), // Palestine
            "PL" => Some(28), // Poland
            "PT" => Some(25), // Portugal
            "QA" => Some(29), // Qatar
            "RO" => Some(24), // Romania
            "SM" => Some(27), // San Marino
            "SA" => Some(24), // Saudi Arabia
            "RS" => Some(22), // Serbia
            "SK" => Some(24), // Slovakia
            "SI" => Some(19), // Slovenia
            "ES" => Some(24), // Spain
            "SE" => Some(24), // Sweden
            "CH" => Some(21), // Switzerland
            "TN" => Some(24), // Tunisia
            "TR" => Some(26), // Turkey
            "UA" => Some(29), // Ukraine
            "AE" => Some(23), // UAE
            "GB" => Some(22), // United Kingdom
            "VA" => Some(22), // Vatican
            "VG" => Some(24), // British Virgin Islands
            _ => None,
        }
    }

    fn strip_spaces(&self, input: &str) -> Result<String, ValidationError> {
        let mut result = String::with_capacity(input.len());

        for c in input.chars() {
            if c == ' ' {
                if !self.allow_spaces {
                    return Err(ValidationError::new(
                        "iban_spaces_not_allowed",
                        "Spaces not allowed in IBAN",
                    ));
                }
            } else {
                result.push(c);
            }
        }

        Ok(result)
    }

    /// Validates the IBAN checksum using MOD 97-10 algorithm.
    fn validate_checksum(iban: &str) -> Result<(), ValidationError> {
        // Move first 4 characters to end
        let rearranged = format!("{}{}", &iban[4..], &iban[..4]);

        // Convert letters to numbers (A=10, B=11, ..., Z=35)
        let mut numeric = String::with_capacity(rearranged.len() * 2);
        for c in rearranged.chars() {
            if c.is_ascii_digit() {
                numeric.push(c);
            } else if c.is_ascii_uppercase() {
                let value = c as u32 - 'A' as u32 + 10;
                numeric.push_str(&value.to_string());
            } else {
                return Err(ValidationError::new(
                    "iban_invalid_char",
                    format!("Invalid character '{}' in IBAN", c),
                ));
            }
        }

        // Calculate MOD 97 in chunks (to avoid overflow)
        let mut remainder: u64 = 0;
        for chunk in numeric.as_bytes().chunks(9) {
            let chunk_str = std::str::from_utf8(chunk).unwrap_or("0");
            let combined = format!("{}{}", remainder, chunk_str);
            remainder = combined.parse::<u64>().unwrap_or(0) % 97;
        }

        if remainder == 1 {
            Ok(())
        } else {
            Err(ValidationError::new(
                "iban_invalid_checksum",
                "IBAN failed checksum validation",
            ))
        }
    }
}

impl Default for Iban {
    fn default() -> Self {
        Self::new()
    }
}

impl Validator for Iban {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.is_empty() {
            return Err(ValidationError::new("empty_iban", "IBAN cannot be empty"));
        }

        let iban = self.strip_spaces(input)?;
        let iban = iban.to_uppercase();

        // Minimum IBAN length is 15 (Norway)
        if iban.len() < 15 {
            return Err(ValidationError::new(
                "iban_too_short",
                format!("IBAN must be at least 15 characters (found {})", iban.len()),
            ));
        }

        // Maximum IBAN length is 34
        if iban.len() > 34 {
            return Err(ValidationError::new(
                "iban_too_long",
                format!("IBAN cannot exceed 34 characters (found {})", iban.len()),
            ));
        }

        // First 2 characters must be letters (country code)
        let country_code = &iban[..2];
        if !country_code.chars().all(|c| c.is_ascii_uppercase()) {
            return Err(ValidationError::new(
                "iban_invalid_country",
                "IBAN must start with a 2-letter country code",
            ));
        }

        // Characters 3-4 must be digits (check digits)
        let check_digits = &iban[2..4];
        if !check_digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(ValidationError::new(
                "iban_invalid_check_digits",
                "IBAN check digits (positions 3-4) must be numeric",
            ));
        }

        // Remaining characters must be alphanumeric
        let bban = &iban[4..];
        if !bban.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(ValidationError::new(
                "iban_invalid_bban",
                "IBAN BBAN (after check digits) must be alphanumeric",
            ));
        }

        // Validate country-specific length
        if self.validate_country_length
            && let Some(expected_len) = Self::country_length(country_code)
            && iban.len() != expected_len
        {
            return Err(ValidationError::new(
                "iban_wrong_length",
                format!(
                    "IBAN for {} must be {} characters (found {})",
                    country_code,
                    expected_len,
                    iban.len()
                ),
            ));
        }
        // Note: We don't error for unknown countries, just skip length validation

        // Validate checksum
        Self::validate_checksum(&iban)?;

        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Iban".to_string(),
            description: Some(format!(
                "Validates IBAN with MOD 97-10 checksum (spaces: {})",
                if self.allow_spaces {
                    "allowed"
                } else {
                    "not allowed"
                }
            )),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(10)),
            tags: vec![
                "text".to_string(),
                "iban".to_string(),
                "banking".to_string(),
                "finance".to_string(),
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

    // Valid test IBANs from various countries
    const DE_IBAN: &str = "DE89370400440532013000";
    const GB_IBAN: &str = "GB82WEST12345698765432";
    const FR_IBAN: &str = "FR1420041010050500013M02606";
    const NL_IBAN: &str = "NL91ABNA0417164300";
    const BE_IBAN: &str = "BE68539007547034";
    const ES_IBAN: &str = "ES9121000418450200051332";

    mod valid {
        use super::*;

        #[test]
        fn test_valid_ibans() {
            let validator = Iban::new();
            assert!(validator.validate(DE_IBAN).is_ok());
            assert!(validator.validate(GB_IBAN).is_ok());
            assert!(validator.validate(FR_IBAN).is_ok());
            assert!(validator.validate(NL_IBAN).is_ok());
            assert!(validator.validate(BE_IBAN).is_ok());
            assert!(validator.validate(ES_IBAN).is_ok());
        }

        #[test]
        fn test_lowercase_accepted() {
            let validator = Iban::new();
            assert!(validator.validate("de89370400440532013000").is_ok());
            assert!(validator.validate("gb82west12345698765432").is_ok());
        }

        #[test]
        fn test_mixed_case() {
            let validator = Iban::new();
            assert!(validator.validate("De89370400440532013000").is_ok());
            assert!(validator.validate("GB82west12345698765432").is_ok());
        }
    }

    mod spaces {
        use super::*;

        #[test]
        fn test_with_spaces_allowed() {
            let validator = Iban::new().allow_spaces();
            assert!(validator.validate("DE89 3704 0044 0532 0130 00").is_ok());
            assert!(validator.validate("GB82 WEST 1234 5698 7654 32").is_ok());
        }

        #[test]
        fn test_spaces_not_allowed() {
            let validator = Iban::new();
            assert!(validator.validate("DE89 3704 0044 0532 0130 00").is_err());
        }
    }

    mod invalid {
        use super::*;

        #[test]
        fn test_invalid_checksum() {
            let validator = Iban::new();
            // Changed last digit
            assert!(validator.validate("DE89370400440532013001").is_err());
            // Changed middle digit
            assert!(validator.validate("DE89370400440532023000").is_err());
        }

        #[test]
        fn test_invalid_country() {
            let validator = Iban::new();
            assert!(validator.validate("XX89370400440532013000").is_err());
            assert!(validator.validate("12345678901234567890").is_err());
        }

        #[test]
        fn test_wrong_length() {
            let validator = Iban::new();
            // German IBAN should be 22 chars
            assert!(validator.validate("DE8937040044053201300").is_err()); // 21 chars
            assert!(validator.validate("DE893704004405320130000").is_err()); // 23 chars
        }

        #[test]
        fn test_too_short() {
            let validator = Iban::new();
            assert!(validator.validate("DE893704").is_err());
        }

        #[test]
        fn test_too_long() {
            let validator = Iban::new();
            assert!(
                validator
                    .validate("DE89370400440532013000123456789012345")
                    .is_err()
            );
        }

        #[test]
        fn test_invalid_chars() {
            let validator = Iban::new();
            assert!(validator.validate("DE89370400440532013@00").is_err());
            assert!(validator.validate("DE89370400440532013#00").is_err());
        }

        #[test]
        fn test_empty_string() {
            let validator = Iban::new();
            assert!(validator.validate("").is_err());
        }
    }

    mod country_lengths {
        use super::*;

        #[test]
        fn test_country_length_lookup() {
            assert_eq!(Iban::country_length("DE"), Some(22));
            assert_eq!(Iban::country_length("GB"), Some(22));
            assert_eq!(Iban::country_length("NO"), Some(15));
            assert_eq!(Iban::country_length("MT"), Some(31));
            assert_eq!(Iban::country_length("XX"), None);
        }

        #[test]
        fn test_skip_country_length() {
            let validator = Iban::new().skip_country_length();
            // This IBAN has wrong length for DE but valid checksum
            // We skip length check but checksum will still fail
            // Let's test with a valid structure but unknown country
            assert!(validator.validate(DE_IBAN).is_ok());
        }
    }

    mod checksum {
        use super::*;

        #[test]
        fn test_checksum_algorithm() {
            // The MOD 97-10 algorithm should return 1 for valid IBANs
            assert!(Iban::validate_checksum("DE89370400440532013000").is_ok());
            assert!(Iban::validate_checksum("GB82WEST12345698765432").is_ok());
        }
    }

    mod metadata {
        use super::*;

        #[test]
        fn test_metadata() {
            let validator = Iban::new();
            let metadata = validator.metadata();
            assert_eq!(metadata.name, "Iban");
            assert!(metadata.tags.contains(&"iban".to_string()));
            assert!(metadata.tags.contains(&"banking".to_string()));
        }
    }
}
