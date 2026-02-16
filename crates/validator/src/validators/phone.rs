//! Phone number validator for E.164 and common formats.
//!
//! Validates phone numbers with flexible format support.

use crate::foundation::{Validate, ValidationComplexity, ValidationError, ValidatorMetadata};

// ============================================================================
// PHONE NUMBER VALIDATOR
// ============================================================================

/// Validates phone numbers in various formats.
///
/// Supports multiple format modes:
/// - **E.164**: International standard format `+[country][number]` (e.g., `+14155551234`)
/// - **Lenient**: Allows common separators like spaces, dashes, parentheses
/// - **Digits only**: Just validates the digit count after stripping formatting
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::Phone;
/// use nebula_validator::foundation::Validate;
///
/// // E.164 format (strict)
/// let e164 = Phone::e164();
/// assert!(e164.validate("+14155551234").is_ok());
/// assert!(e164.validate("+442071234567").is_ok());
/// assert!(e164.validate("14155551234").is_err()); // missing +
///
/// // Lenient format (allows common formatting)
/// let lenient = Phone::lenient();
/// assert!(lenient.validate("+1 (415) 555-1234").is_ok());
/// assert!(lenient.validate("+44 20 7123 4567").is_ok());
/// assert!(lenient.validate("(415) 555-1234").is_ok());
///
/// // Digits only (just checks count)
/// let digits = Phone::digits_only().min_digits(7).max_digits(15);
/// assert!(digits.validate("4155551234").is_ok());
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Phone {
    mode: PhoneMode,
    min_digits: u8,
    max_digits: u8,
    require_country_code: bool,
}

/// Phone validation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhoneMode {
    /// E.164 international format: `+[country][number]`
    E164,
    /// Lenient: allows spaces, dashes, parentheses
    Lenient,
    /// Digits only: strips all formatting, validates digit count
    DigitsOnly,
}

impl Phone {
    /// Creates a new phone validator with E.164 format.
    ///
    /// E.164 format requires:
    /// - Leading `+` sign
    /// - Only digits after the `+`
    /// - Between 7 and 15 digits (configurable)
    #[must_use]
    pub fn e164() -> Self {
        Self {
            mode: PhoneMode::E164,
            min_digits: 7,
            max_digits: 15,
            require_country_code: true,
        }
    }

    /// Creates a lenient phone validator.
    ///
    /// Allows common formatting characters:
    /// - Spaces, dashes, parentheses, dots
    /// - Optional leading `+` for country code
    #[must_use]
    pub fn lenient() -> Self {
        Self {
            mode: PhoneMode::Lenient,
            min_digits: 7,
            max_digits: 15,
            require_country_code: false,
        }
    }

    /// Creates a digits-only phone validator.
    ///
    /// Strips all non-digit characters and validates the count.
    #[must_use]
    pub fn digits_only() -> Self {
        Self {
            mode: PhoneMode::DigitsOnly,
            min_digits: 7,
            max_digits: 15,
            require_country_code: false,
        }
    }

    /// Sets the minimum number of digits required.
    #[must_use = "builder methods must be chained or built"]
    pub fn min_digits(mut self, min: u8) -> Self {
        self.min_digits = min;
        self
    }

    /// Sets the maximum number of digits allowed.
    #[must_use = "builder methods must be chained or built"]
    pub fn max_digits(mut self, max: u8) -> Self {
        self.max_digits = max;
        self
    }

    /// Requires a country code (leading `+`).
    #[must_use = "builder methods must be chained or built"]
    pub fn require_country_code(mut self) -> Self {
        self.require_country_code = true;
        self
    }

    fn extract_digits(&self, input: &str) -> String {
        input.chars().filter(|c| c.is_ascii_digit()).collect()
    }

    fn validate_e164(&self, input: &str) -> Result<(), ValidationError> {
        if !input.starts_with('+') {
            return Err(ValidationError::new(
                "e164_missing_plus",
                "E.164 phone number must start with '+'",
            ));
        }

        let number_part = &input[1..];

        // Check that all remaining characters are digits
        if !number_part.chars().all(|c| c.is_ascii_digit()) {
            return Err(ValidationError::new(
                "e164_invalid_chars",
                "E.164 phone number must contain only digits after '+'",
            ));
        }

        let digit_count = number_part.len();
        self.validate_digit_count(digit_count)
    }

    fn validate_lenient(&self, input: &str) -> Result<(), ValidationError> {
        // Check for valid characters
        let allowed_chars = |c: char| {
            c.is_ascii_digit()
                || c == '+'
                || c == '-'
                || c == ' '
                || c == '('
                || c == ')'
                || c == '.'
        };

        if !input.chars().all(allowed_chars) {
            return Err(ValidationError::new(
                "phone_invalid_chars",
                "Phone number contains invalid characters",
            ));
        }

        // Plus sign can only appear at the start
        if let Some(pos) = input.find('+') {
            if pos != 0 {
                return Err(ValidationError::new(
                    "phone_plus_position",
                    "'+' can only appear at the start of the phone number",
                ));
            }
        } else if self.require_country_code {
            return Err(ValidationError::new(
                "phone_missing_country_code",
                "Phone number must include country code (start with '+')",
            ));
        }

        // Check balanced parentheses
        let open_parens = input.chars().filter(|&c| c == '(').count();
        let close_parens = input.chars().filter(|&c| c == ')').count();
        if open_parens != close_parens {
            return Err(ValidationError::new(
                "phone_unbalanced_parens",
                "Phone number has unbalanced parentheses",
            ));
        }

        let digits = self.extract_digits(input);
        self.validate_digit_count(digits.len())
    }

    fn validate_digits_only(&self, input: &str) -> Result<(), ValidationError> {
        if self.require_country_code && !input.starts_with('+') {
            return Err(ValidationError::new(
                "phone_missing_country_code",
                "Phone number must include country code (start with '+')",
            ));
        }

        let digits = self.extract_digits(input);
        self.validate_digit_count(digits.len())
    }

    fn validate_digit_count(&self, count: usize) -> Result<(), ValidationError> {
        if count < self.min_digits as usize {
            return Err(ValidationError::new(
                "phone_too_few_digits",
                format!(
                    "Phone number must have at least {} digits (found {})",
                    self.min_digits, count
                ),
            ));
        }

        if count > self.max_digits as usize {
            return Err(ValidationError::new(
                "phone_too_many_digits",
                format!(
                    "Phone number must have at most {} digits (found {})",
                    self.max_digits, count
                ),
            ));
        }

        Ok(())
    }
}

impl Default for Phone {
    fn default() -> Self {
        Self::lenient()
    }
}

impl Validate for Phone {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.is_empty() {
            return Err(ValidationError::new(
                "empty_phone",
                "Phone number cannot be empty",
            ));
        }

        match self.mode {
            PhoneMode::E164 => self.validate_e164(input),
            PhoneMode::Lenient => self.validate_lenient(input),
            PhoneMode::DigitsOnly => self.validate_digits_only(input),
        }
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Phone".into(),
            description: Some(
                format!(
                    "Validates phone numbers ({:?} mode, {}-{} digits{})",
                    self.mode,
                    self.min_digits,
                    self.max_digits,
                    if self.require_country_code {
                        ", country code required"
                    } else {
                        ""
                    }
                )
                .into(),
            ),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(3)),
            tags: vec!["text".into(), "phone".into(), "contact".into()],
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

    // E.164 Format Tests
    mod e164 {
        use super::*;

        #[test]
        fn test_valid_e164() {
            let validator = Phone::e164();
            assert!(validator.validate("+14155551234").is_ok());
            assert!(validator.validate("+442071234567").is_ok());
            assert!(validator.validate("+33123456789").is_ok());
            assert!(validator.validate("+81312345678").is_ok());
        }

        #[test]
        fn test_missing_plus() {
            let validator = Phone::e164();
            assert!(validator.validate("14155551234").is_err());
        }

        #[test]
        fn test_invalid_chars_in_e164() {
            let validator = Phone::e164();
            assert!(validator.validate("+1 415 555 1234").is_err()); // spaces
            assert!(validator.validate("+1-415-555-1234").is_err()); // dashes
            assert!(validator.validate("+1(415)5551234").is_err()); // parens
        }

        #[test]
        fn test_e164_digit_limits() {
            let validator = Phone::e164();
            assert!(validator.validate("+123456").is_err()); // too short (6 digits)
            assert!(validator.validate("+1234567").is_ok()); // min (7 digits)
            assert!(validator.validate("+123456789012345").is_ok()); // max (15 digits)
            assert!(validator.validate("+1234567890123456").is_err()); // too long (16 digits)
        }
    }

    // Lenient Format Tests
    mod lenient {
        use super::*;

        #[test]
        fn test_valid_lenient() {
            let validator = Phone::lenient();
            assert!(validator.validate("+1 (415) 555-1234").is_ok());
            assert!(validator.validate("+44 20 7123 4567").is_ok());
            assert!(validator.validate("(415) 555-1234").is_ok());
            assert!(validator.validate("415-555-1234").is_ok());
            assert!(validator.validate("415.555.1234").is_ok());
            assert!(validator.validate("4155551234").is_ok());
        }

        #[test]
        fn test_plus_position() {
            let validator = Phone::lenient();
            assert!(validator.validate("+14155551234").is_ok());
            assert!(validator.validate("1+4155551234").is_err()); // plus in wrong position
        }

        #[test]
        fn test_unbalanced_parens() {
            let validator = Phone::lenient();
            assert!(validator.validate("(415 555-1234").is_err());
            assert!(validator.validate("415) 555-1234").is_err());
        }

        #[test]
        fn test_invalid_chars_lenient() {
            let validator = Phone::lenient();
            assert!(validator.validate("+1 415 555 1234 ext").is_err()); // letters
            assert!(validator.validate("+1#415#555#1234").is_err()); // hash
        }

        #[test]
        fn test_require_country_code() {
            let validator = Phone::lenient().require_country_code();
            assert!(validator.validate("+1 415 555-1234").is_ok());
            assert!(validator.validate("415 555-1234").is_err());
        }
    }

    // Digits Only Tests
    mod digits_only {
        use super::*;

        #[test]
        fn test_valid_digits_only() {
            let validator = Phone::digits_only();
            assert!(validator.validate("4155551234").is_ok());
            assert!(validator.validate("+14155551234").is_ok());
            assert!(validator.validate("1-415-555-1234").is_ok()); // strips non-digits
            assert!(validator.validate("anything (with) 1234567 digits").is_ok());
        }

        #[test]
        fn test_custom_digit_limits() {
            let validator = Phone::digits_only().min_digits(10).max_digits(10);
            assert!(validator.validate("1234567890").is_ok());
            assert!(validator.validate("123456789").is_err()); // too short
            assert!(validator.validate("12345678901").is_err()); // too long
        }
    }

    // General Tests
    mod general {
        use super::*;

        #[test]
        fn test_empty_string() {
            let validator = Phone::e164();
            assert!(validator.validate("").is_err());

            let validator = Phone::lenient();
            assert!(validator.validate("").is_err());
        }

        #[test]
        fn test_real_world_numbers() {
            let e164 = Phone::e164();
            let lenient = Phone::lenient();

            // US numbers
            assert!(e164.validate("+12025551234").is_ok());
            assert!(lenient.validate("+1 (202) 555-1234").is_ok());

            // UK numbers
            assert!(e164.validate("+442071234567").is_ok());
            assert!(lenient.validate("+44 20 7123 4567").is_ok());

            // German numbers
            assert!(e164.validate("+4930123456").is_ok());
            assert!(lenient.validate("+49 30 123456").is_ok());

            // Japanese numbers
            assert!(e164.validate("+81312345678").is_ok());
            assert!(lenient.validate("+81 3 1234 5678").is_ok());
        }

        #[test]
        fn test_metadata() {
            let validator = Phone::e164();
            let metadata = validator.metadata();
            assert_eq!(metadata.name, "Phone");
            assert!(metadata.tags.contains(&"phone".into()));
        }
    }
}
