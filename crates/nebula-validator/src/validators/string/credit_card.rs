//! Credit card number validator with Luhn algorithm.
//!
//! Validates credit card numbers for format and checksum.

use crate::core::{ValidationComplexity, ValidationError, Validator, ValidatorMetadata};

// ============================================================================
// CREDIT CARD VALIDATOR
// ============================================================================

/// Validates credit card numbers using the Luhn algorithm.
///
/// Supports common card types with proper prefix and length validation:
/// - Visa: starts with 4, 13 or 16 digits
/// - Mastercard: starts with 51-55 or 2221-2720, 16 digits
/// - American Express: starts with 34 or 37, 15 digits
/// - Discover: starts with 6011, 622126-622925, 644-649, or 65, 16 digits
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::string::CreditCard;
/// use nebula_validator::core::Validator;
///
/// let validator = CreditCard::new();
///
/// // Valid test card numbers (these pass Luhn check)
/// assert!(validator.validate("4111111111111111").is_ok()); // Visa
/// assert!(validator.validate("5500000000000004").is_ok()); // Mastercard
///
/// // With spaces/dashes (lenient mode)
/// let lenient = CreditCard::new().allow_separators();
/// assert!(lenient.validate("4111 1111 1111 1111").is_ok());
/// assert!(lenient.validate("4111-1111-1111-1111").is_ok());
///
/// // Invalid
/// assert!(validator.validate("1234567890123456").is_err()); // fails Luhn
/// assert!(validator.validate("411111111111111").is_err()); // wrong length
/// ```
#[derive(Debug, Clone, Copy)]
pub struct CreditCard {
    allow_separators: bool,
    validate_card_type: bool,
    allowed_types: CardTypes,
}

/// Bitmask for allowed card types.
#[derive(Debug, Clone, Copy, Default)]
pub struct CardTypes(u8);

impl CardTypes {
    const VISA: u8 = 1 << 0;
    const MASTERCARD: u8 = 1 << 1;
    const AMEX: u8 = 1 << 2;
    const DISCOVER: u8 = 1 << 3;

    /// Allow all card types.
    #[must_use]
    pub fn all() -> Self {
        Self(Self::VISA | Self::MASTERCARD | Self::AMEX | Self::DISCOVER)
    }

    /// Allow no card types (disables type checking).
    #[must_use]
    pub fn none() -> Self {
        Self(0)
    }

    /// Allow Visa cards.
    #[must_use]
    pub fn visa(mut self) -> Self {
        self.0 |= Self::VISA;
        self
    }

    /// Allow Mastercard cards.
    #[must_use]
    pub fn mastercard(mut self) -> Self {
        self.0 |= Self::MASTERCARD;
        self
    }

    /// Allow American Express cards.
    #[must_use]
    pub fn amex(mut self) -> Self {
        self.0 |= Self::AMEX;
        self
    }

    /// Allow Discover cards.
    #[must_use]
    pub fn discover(mut self) -> Self {
        self.0 |= Self::DISCOVER;
        self
    }

    fn allows(&self, card_type: u8) -> bool {
        self.0 == 0 || (self.0 & card_type) != 0
    }
}

/// Detected card type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardType {
    Visa,
    Mastercard,
    Amex,
    Discover,
    Unknown,
}

impl CreditCard {
    /// Creates a new credit card validator with default settings.
    ///
    /// Default settings:
    /// - Separators not allowed (strict digits only)
    /// - All card types accepted
    /// - Luhn checksum validation enabled
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_separators: false,
            validate_card_type: true,
            allowed_types: CardTypes::all(),
        }
    }

    /// Allow spaces and dashes as separators.
    #[must_use = "builder methods must be chained or built"]
    pub fn allow_separators(mut self) -> Self {
        self.allow_separators = true;
        self
    }

    /// Skip card type validation (only validate Luhn checksum).
    #[must_use = "builder methods must be chained or built"]
    pub fn skip_type_validation(mut self) -> Self {
        self.validate_card_type = false;
        self
    }

    /// Only allow specific card types.
    #[must_use = "builder methods must be chained or built"]
    pub fn only_types(mut self, types: CardTypes) -> Self {
        self.allowed_types = types;
        self
    }

    fn extract_digits(&self, input: &str) -> Result<String, ValidationError> {
        let mut digits = String::new();

        for c in input.chars() {
            if c.is_ascii_digit() {
                digits.push(c);
            } else if c == ' ' || c == '-' {
                if !self.allow_separators {
                    return Err(ValidationError::new(
                        "cc_separators_not_allowed",
                        "Separators not allowed in credit card number",
                    ));
                }
            } else {
                return Err(ValidationError::new(
                    "cc_invalid_char",
                    format!("Invalid character '{}' in credit card number", c),
                ));
            }
        }

        Ok(digits)
    }

    fn detect_card_type(digits: &str) -> CardType {
        if digits.is_empty() {
            return CardType::Unknown;
        }

        // Visa: starts with 4
        if digits.starts_with('4') {
            return CardType::Visa;
        }

        // Mastercard: 51-55 or 2221-2720
        if digits.len() >= 2 {
            let prefix2: u32 = digits[..2].parse().unwrap_or(0);
            if (51..=55).contains(&prefix2) {
                return CardType::Mastercard;
            }
        }
        if digits.len() >= 4 {
            let prefix4: u32 = digits[..4].parse().unwrap_or(0);
            if (2221..=2720).contains(&prefix4) {
                return CardType::Mastercard;
            }
        }

        // American Express: 34 or 37
        if digits.starts_with("34") || digits.starts_with("37") {
            return CardType::Amex;
        }

        // Discover: 6011, 622126-622925, 644-649, or 65
        if digits.starts_with("6011") || digits.starts_with("65") {
            return CardType::Discover;
        }
        if digits.len() >= 3 {
            let prefix3: u32 = digits[..3].parse().unwrap_or(0);
            if (644..=649).contains(&prefix3) {
                return CardType::Discover;
            }
        }
        if digits.len() >= 6 {
            let prefix6: u32 = digits[..6].parse().unwrap_or(0);
            if (622126..=622925).contains(&prefix6) {
                return CardType::Discover;
            }
        }

        CardType::Unknown
    }

    fn validate_length(card_type: CardType, len: usize) -> Result<(), ValidationError> {
        let valid = match card_type {
            CardType::Visa => len == 13 || len == 16,
            CardType::Mastercard => len == 16,
            CardType::Amex => len == 15,
            CardType::Discover => len == 16,
            CardType::Unknown => (13..=19).contains(&len), // Generic range
        };

        if !valid {
            return Err(ValidationError::new(
                "cc_invalid_length",
                format!(
                    "Invalid credit card length for {:?}: {} digits",
                    card_type, len
                ),
            ));
        }

        Ok(())
    }

    /// Validates using the Luhn algorithm (mod 10 check).
    fn validate_luhn(digits: &str) -> Result<(), ValidationError> {
        let mut sum = 0;
        let mut double = false;

        // Process digits from right to left
        for c in digits.chars().rev() {
            let mut digit = c.to_digit(10).unwrap_or(0);

            if double {
                digit *= 2;
                if digit > 9 {
                    digit -= 9;
                }
            }

            sum += digit;
            double = !double;
        }

        if sum % 10 == 0 {
            Ok(())
        } else {
            Err(ValidationError::new(
                "cc_invalid_luhn",
                "Credit card number failed Luhn checksum",
            ))
        }
    }
}

impl Default for CreditCard {
    fn default() -> Self {
        Self::new()
    }
}

impl Validator for CreditCard {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.is_empty() {
            return Err(ValidationError::new(
                "empty_cc",
                "Credit card number cannot be empty",
            ));
        }

        let digits = self.extract_digits(input)?;

        if digits.is_empty() {
            return Err(ValidationError::new(
                "cc_no_digits",
                "Credit card number must contain digits",
            ));
        }

        // Detect card type
        let card_type = Self::detect_card_type(&digits);

        // Validate card type is allowed
        if self.validate_card_type {
            let type_bit = match card_type {
                CardType::Visa => CardTypes::VISA,
                CardType::Mastercard => CardTypes::MASTERCARD,
                CardType::Amex => CardTypes::AMEX,
                CardType::Discover => CardTypes::DISCOVER,
                CardType::Unknown => 0,
            };

            if card_type != CardType::Unknown && !self.allowed_types.allows(type_bit) {
                return Err(ValidationError::new(
                    "cc_type_not_allowed",
                    format!("{:?} cards are not accepted", card_type),
                ));
            }
        }

        // Validate length
        Self::validate_length(card_type, digits.len())?;

        // Validate Luhn checksum
        Self::validate_luhn(&digits)?;

        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "CreditCard".to_string(),
            description: Some(format!(
                "Validates credit card numbers with Luhn checksum (separators: {})",
                if self.allow_separators {
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
                "credit_card".to_string(),
                "payment".to_string(),
                "luhn".to_string(),
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

    // Test card numbers (well-known test numbers that pass Luhn)
    const VISA_TEST: &str = "4111111111111111";
    const MASTERCARD_TEST: &str = "5500000000000004";
    const AMEX_TEST: &str = "340000000000009";
    const DISCOVER_TEST: &str = "6011000000000004";

    mod basic {
        use super::*;

        #[test]
        fn test_valid_cards() {
            let validator = CreditCard::new();
            assert!(validator.validate(VISA_TEST).is_ok());
            assert!(validator.validate(MASTERCARD_TEST).is_ok());
            assert!(validator.validate(AMEX_TEST).is_ok());
            assert!(validator.validate(DISCOVER_TEST).is_ok());
        }

        #[test]
        fn test_empty_string() {
            let validator = CreditCard::new();
            assert!(validator.validate("").is_err());
        }

        #[test]
        fn test_invalid_chars() {
            let validator = CreditCard::new();
            assert!(validator.validate("4111-1111-1111-1111").is_err()); // no separators
            assert!(validator.validate("4111a111111111111").is_err()); // letters
        }
    }

    mod separators {
        use super::*;

        #[test]
        fn test_with_spaces() {
            let validator = CreditCard::new().allow_separators();
            assert!(validator.validate("4111 1111 1111 1111").is_ok());
        }

        #[test]
        fn test_with_dashes() {
            let validator = CreditCard::new().allow_separators();
            assert!(validator.validate("4111-1111-1111-1111").is_ok());
        }

        #[test]
        fn test_separators_not_allowed() {
            let validator = CreditCard::new();
            assert!(validator.validate("4111 1111 1111 1111").is_err());
        }
    }

    mod luhn {
        use super::*;

        #[test]
        fn test_valid_luhn() {
            let validator = CreditCard::new().skip_type_validation();
            // These pass Luhn (16 digits with valid checksum)
            assert!(validator.validate("4532015112830366").is_ok());
            assert!(validator.validate("6304000000000000").is_ok()); // Maestro test
        }

        #[test]
        fn test_invalid_luhn() {
            let validator = CreditCard::new().skip_type_validation();
            // Modify a valid number to break Luhn
            assert!(validator.validate("4111111111111112").is_err());
            assert!(validator.validate("1234567890123456").is_err());
        }
    }

    mod card_types {
        use super::*;

        #[test]
        fn test_detect_visa() {
            assert_eq!(
                CreditCard::detect_card_type("4111111111111111"),
                CardType::Visa
            );
            assert_eq!(CreditCard::detect_card_type("4"), CardType::Visa);
        }

        #[test]
        fn test_detect_mastercard() {
            assert_eq!(
                CreditCard::detect_card_type("5500000000000004"),
                CardType::Mastercard
            );
            assert_eq!(
                CreditCard::detect_card_type("5100000000000000"),
                CardType::Mastercard
            );
            assert_eq!(
                CreditCard::detect_card_type("2221000000000000"),
                CardType::Mastercard
            );
        }

        #[test]
        fn test_detect_amex() {
            assert_eq!(
                CreditCard::detect_card_type("340000000000009"),
                CardType::Amex
            );
            assert_eq!(
                CreditCard::detect_card_type("370000000000000"),
                CardType::Amex
            );
        }

        #[test]
        fn test_detect_discover() {
            assert_eq!(
                CreditCard::detect_card_type("6011000000000004"),
                CardType::Discover
            );
            assert_eq!(
                CreditCard::detect_card_type("6500000000000000"),
                CardType::Discover
            );
        }

        #[test]
        fn test_only_visa() {
            let validator = CreditCard::new().only_types(CardTypes::none().visa());
            assert!(validator.validate(VISA_TEST).is_ok());
            assert!(validator.validate(MASTERCARD_TEST).is_err());
        }

        #[test]
        fn test_only_amex() {
            let validator = CreditCard::new().only_types(CardTypes::none().amex());
            assert!(validator.validate(AMEX_TEST).is_ok());
            assert!(validator.validate(VISA_TEST).is_err());
        }
    }

    mod length {
        use super::*;

        #[test]
        fn test_visa_lengths() {
            let validator = CreditCard::new();
            // 16 digits
            assert!(validator.validate("4111111111111111").is_ok());
            // 13 digits (old Visa format)
            assert!(validator.validate("4222222222222").is_ok());
            // 14 digits - invalid
            assert!(validator.validate("42222222222222").is_err());
        }

        #[test]
        fn test_amex_length() {
            let validator = CreditCard::new();
            // Must be 15 digits
            assert!(validator.validate("340000000000009").is_ok());
            assert!(validator.validate("3400000000000").is_err()); // 13 digits
        }
    }

    mod metadata {
        use super::*;

        #[test]
        fn test_metadata() {
            let validator = CreditCard::new();
            let metadata = validator.metadata();
            assert_eq!(metadata.name, "CreditCard");
            assert!(metadata.tags.contains(&"credit_card".to_string()));
            assert!(metadata.tags.contains(&"luhn".to_string()));
        }
    }
}
