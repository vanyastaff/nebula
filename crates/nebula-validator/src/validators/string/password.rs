//! Password strength validator.
//!
//! Validates passwords against configurable complexity requirements.

use crate::core::{ValidationComplexity, ValidationError, Validator, ValidatorMetadata};

// ============================================================================
// PASSWORD STRENGTH VALIDATOR
// ============================================================================

/// Validates password strength against configurable requirements.
///
/// Requirements can include:
/// - Minimum/maximum length
/// - Uppercase letters
/// - Lowercase letters
/// - Digits
/// - Special characters
/// - No repeated characters
/// - No common patterns
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::string::Password;
/// use nebula_validator::core::Validator;
///
/// // Basic validation
/// let basic = Password::new().min_length(8);
/// assert!(basic.validate("password123").is_ok());
/// assert!(basic.validate("short").is_err());
///
/// // Strong password policy
/// let strong = Password::new()
///     .min_length(12)
///     .require_uppercase()
///     .require_lowercase()
///     .require_digit()
///     .require_special();
/// assert!(strong.validate("MyP@ssw0rd123!").is_ok());
/// assert!(strong.validate("weakpassword").is_err());
///
/// // Preset policies
/// let moderate = Password::moderate();
/// let strong = Password::strong();
/// ```
#[derive(Debug, Clone)]
pub struct Password {
    min_length: usize,
    max_length: usize,
    require_uppercase: bool,
    require_lowercase: bool,
    require_digit: bool,
    require_special: bool,
    min_unique_chars: usize,
    max_repeated: usize,
    disallow_common: bool,
    special_chars: String,
}

impl Password {
    /// Default set of special characters.
    pub const DEFAULT_SPECIAL_CHARS: &'static str = "!@#$%^&*()_+-=[]{}|;':\",./<>?`~";

    /// Creates a new password validator with minimal requirements.
    ///
    /// Default settings:
    /// - Minimum length: 1
    /// - Maximum length: 128
    /// - No character type requirements
    #[must_use]
    pub fn new() -> Self {
        Self {
            min_length: 1,
            max_length: 128,
            require_uppercase: false,
            require_lowercase: false,
            require_digit: false,
            require_special: false,
            min_unique_chars: 0,
            max_repeated: 0,
            disallow_common: false,
            special_chars: Self::DEFAULT_SPECIAL_CHARS.to_string(),
        }
    }

    /// Creates a moderate strength password policy.
    ///
    /// Requirements:
    /// - Minimum 8 characters
    /// - At least one uppercase
    /// - At least one lowercase
    /// - At least one digit
    #[must_use]
    pub fn moderate() -> Self {
        Self::new()
            .min_length(8)
            .require_uppercase()
            .require_lowercase()
            .require_digit()
    }

    /// Creates a strong password policy.
    ///
    /// Requirements:
    /// - Minimum 12 characters
    /// - At least one uppercase
    /// - At least one lowercase
    /// - At least one digit
    /// - At least one special character
    /// - At least 6 unique characters
    /// - Disallow common passwords
    #[must_use]
    pub fn strong() -> Self {
        Self::new()
            .min_length(12)
            .require_uppercase()
            .require_lowercase()
            .require_digit()
            .require_special()
            .min_unique_chars(6)
            .disallow_common()
    }

    /// Sets the minimum password length.
    #[must_use = "builder methods must be chained or built"]
    pub fn min_length(mut self, len: usize) -> Self {
        self.min_length = len;
        self
    }

    /// Sets the maximum password length.
    #[must_use = "builder methods must be chained or built"]
    pub fn max_length(mut self, len: usize) -> Self {
        self.max_length = len;
        self
    }

    /// Requires at least one uppercase letter.
    #[must_use = "builder methods must be chained or built"]
    pub fn require_uppercase(mut self) -> Self {
        self.require_uppercase = true;
        self
    }

    /// Requires at least one lowercase letter.
    #[must_use = "builder methods must be chained or built"]
    pub fn require_lowercase(mut self) -> Self {
        self.require_lowercase = true;
        self
    }

    /// Requires at least one digit.
    #[must_use = "builder methods must be chained or built"]
    pub fn require_digit(mut self) -> Self {
        self.require_digit = true;
        self
    }

    /// Requires at least one special character.
    #[must_use = "builder methods must be chained or built"]
    pub fn require_special(mut self) -> Self {
        self.require_special = true;
        self
    }

    /// Sets the minimum number of unique characters required.
    #[must_use = "builder methods must be chained or built"]
    pub fn min_unique_chars(mut self, count: usize) -> Self {
        self.min_unique_chars = count;
        self
    }

    /// Sets the maximum number of consecutive repeated characters.
    ///
    /// Set to 0 to disable this check.
    #[must_use = "builder methods must be chained or built"]
    pub fn max_repeated(mut self, count: usize) -> Self {
        self.max_repeated = count;
        self
    }

    /// Disallows common weak passwords.
    #[must_use = "builder methods must be chained or built"]
    pub fn disallow_common(mut self) -> Self {
        self.disallow_common = true;
        self
    }

    /// Sets custom special characters.
    #[must_use = "builder methods must be chained or built"]
    pub fn special_chars(mut self, chars: &str) -> Self {
        self.special_chars = chars.to_string();
        self
    }

    fn is_special(&self, c: char) -> bool {
        self.special_chars.contains(c)
    }

    fn count_unique_chars(password: &str) -> usize {
        let mut chars: Vec<char> = password.chars().collect();
        chars.sort_unstable();
        chars.dedup();
        chars.len()
    }

    fn max_consecutive_repeated(password: &str) -> usize {
        if password.is_empty() {
            return 0;
        }

        let mut max = 1;
        let mut current = 1;
        let mut prev: Option<char> = None;

        for c in password.chars() {
            if Some(c) == prev {
                current += 1;
                max = max.max(current);
            } else {
                current = 1;
            }
            prev = Some(c);
        }

        max
    }

    fn is_common_password(password: &str) -> bool {
        // Common weak passwords (lowercase for comparison)
        const COMMON: &[&str] = &[
            "password",
            "password1",
            "password123",
            "123456",
            "12345678",
            "123456789",
            "1234567890",
            "qwerty",
            "qwerty123",
            "abc123",
            "letmein",
            "welcome",
            "admin",
            "administrator",
            "login",
            "master",
            "hello",
            "monkey",
            "dragon",
            "baseball",
            "football",
            "soccer",
            "hockey",
            "batman",
            "superman",
            "trustno1",
            "iloveyou",
            "sunshine",
            "princess",
            "shadow",
            "ashley",
            "michael",
            "passw0rd",
            "pass123",
            "changeme",
            "secret",
            "access",
            "guest",
            "root",
            "toor",
            "test",
            "test123",
            "temp",
            "temp123",
            "default",
            "zxcvbn",
            "asdfgh",
            "qazwsx",
            "111111",
            "000000",
            "121212",
            "654321",
            "987654321",
            "password!",
            "password1!",
        ];

        let lower = password.to_lowercase();
        COMMON.iter().any(|&common| lower == common)
    }
}

impl Default for Password {
    fn default() -> Self {
        Self::new()
    }
}

impl Validator for Password {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        // Length checks
        if input.len() < self.min_length {
            return Err(ValidationError::new(
                "password_too_short",
                format!(
                    "Password must be at least {} characters (found {})",
                    self.min_length,
                    input.len()
                ),
            ));
        }

        if input.len() > self.max_length {
            return Err(ValidationError::new(
                "password_too_long",
                format!(
                    "Password cannot exceed {} characters (found {})",
                    self.max_length,
                    input.len()
                ),
            ));
        }

        // Character type checks
        if self.require_uppercase && !input.chars().any(|c| c.is_ascii_uppercase()) {
            return Err(ValidationError::new(
                "password_no_uppercase",
                "Password must contain at least one uppercase letter",
            ));
        }

        if self.require_lowercase && !input.chars().any(|c| c.is_ascii_lowercase()) {
            return Err(ValidationError::new(
                "password_no_lowercase",
                "Password must contain at least one lowercase letter",
            ));
        }

        if self.require_digit && !input.chars().any(|c| c.is_ascii_digit()) {
            return Err(ValidationError::new(
                "password_no_digit",
                "Password must contain at least one digit",
            ));
        }

        if self.require_special && !input.chars().any(|c| self.is_special(c)) {
            return Err(ValidationError::new(
                "password_no_special",
                "Password must contain at least one special character",
            ));
        }

        // Unique characters check
        if self.min_unique_chars > 0 {
            let unique = Self::count_unique_chars(input);
            if unique < self.min_unique_chars {
                return Err(ValidationError::new(
                    "password_not_enough_unique",
                    format!(
                        "Password must contain at least {} unique characters (found {})",
                        self.min_unique_chars, unique
                    ),
                ));
            }
        }

        // Repeated characters check
        if self.max_repeated > 0 {
            let max_rep = Self::max_consecutive_repeated(input);
            if max_rep > self.max_repeated {
                return Err(ValidationError::new(
                    "password_too_many_repeated",
                    format!(
                        "Password cannot have more than {} consecutive repeated characters (found {})",
                        self.max_repeated, max_rep
                    ),
                ));
            }
        }

        // Common password check
        if self.disallow_common && Self::is_common_password(input) {
            return Err(ValidationError::new(
                "password_too_common",
                "Password is too common and easily guessable",
            ));
        }

        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        let mut requirements = Vec::new();
        requirements.push(format!("length {}-{}", self.min_length, self.max_length));
        if self.require_uppercase {
            requirements.push("uppercase".to_string());
        }
        if self.require_lowercase {
            requirements.push("lowercase".to_string());
        }
        if self.require_digit {
            requirements.push("digit".to_string());
        }
        if self.require_special {
            requirements.push("special".to_string());
        }

        ValidatorMetadata {
            name: "Password".to_string(),
            description: Some(format!(
                "Validates password strength ({})",
                requirements.join(", ")
            )),
            complexity: ValidationComplexity::Linear,
            cacheable: false, // Passwords shouldn't be cached
            estimated_time: Some(std::time::Duration::from_micros(5)),
            tags: vec![
                "text".to_string(),
                "password".to_string(),
                "security".to_string(),
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

    mod length {
        use super::*;

        #[test]
        fn test_min_length() {
            let validator = Password::new().min_length(8);
            assert!(validator.validate("12345678").is_ok());
            assert!(validator.validate("1234567").is_err());
        }

        #[test]
        fn test_max_length() {
            let validator = Password::new().max_length(16);
            assert!(validator.validate("1234567890123456").is_ok());
            assert!(validator.validate("12345678901234567").is_err());
        }
    }

    mod character_types {
        use super::*;

        #[test]
        fn test_require_uppercase() {
            let validator = Password::new().require_uppercase();
            assert!(validator.validate("Password").is_ok());
            assert!(validator.validate("password").is_err());
        }

        #[test]
        fn test_require_lowercase() {
            let validator = Password::new().require_lowercase();
            assert!(validator.validate("Password").is_ok());
            assert!(validator.validate("PASSWORD").is_err());
        }

        #[test]
        fn test_require_digit() {
            let validator = Password::new().require_digit();
            assert!(validator.validate("Password1").is_ok());
            assert!(validator.validate("Password").is_err());
        }

        #[test]
        fn test_require_special() {
            let validator = Password::new().require_special();
            assert!(validator.validate("Password!").is_ok());
            assert!(validator.validate("Password1").is_err());
        }

        #[test]
        fn test_custom_special_chars() {
            let validator = Password::new().require_special().special_chars("@#");
            assert!(validator.validate("Password@").is_ok());
            assert!(validator.validate("Password#").is_ok());
            assert!(validator.validate("Password!").is_err()); // ! not in custom set
        }
    }

    mod complexity {
        use super::*;

        #[test]
        fn test_unique_chars() {
            let validator = Password::new().min_unique_chars(5);
            assert!(validator.validate("abcde").is_ok());
            assert!(validator.validate("aaabbb").is_err()); // only 2 unique
            assert!(validator.validate("abcd").is_err()); // only 4 unique
        }

        #[test]
        fn test_max_repeated() {
            let validator = Password::new().max_repeated(2);
            assert!(validator.validate("aabbcc").is_ok());
            assert!(validator.validate("aaabbb").is_err()); // 3 consecutive
        }

        #[test]
        fn test_count_unique_chars() {
            assert_eq!(Password::count_unique_chars("abc"), 3);
            assert_eq!(Password::count_unique_chars("aabbcc"), 3);
            assert_eq!(Password::count_unique_chars("aaaa"), 1);
            assert_eq!(Password::count_unique_chars("aAbB"), 4);
        }

        #[test]
        fn test_max_consecutive_repeated() {
            assert_eq!(Password::max_consecutive_repeated("abc"), 1);
            assert_eq!(Password::max_consecutive_repeated("aabbcc"), 2);
            assert_eq!(Password::max_consecutive_repeated("aaabbb"), 3);
            assert_eq!(Password::max_consecutive_repeated("abcaaaa"), 4);
        }
    }

    mod common_passwords {
        use super::*;

        #[test]
        fn test_disallow_common() {
            let validator = Password::new().disallow_common();
            assert!(validator.validate("MyUniqueP@ss!").is_ok());
            assert!(validator.validate("password").is_err());
            assert!(validator.validate("Password123").is_err());
            assert!(validator.validate("qwerty").is_err());
            assert!(validator.validate("123456").is_err());
        }

        #[test]
        fn test_common_case_insensitive() {
            let validator = Password::new().disallow_common();
            assert!(validator.validate("PASSWORD").is_err());
            assert!(validator.validate("Password").is_err());
            assert!(validator.validate("QWERTY").is_err());
        }
    }

    mod presets {
        use super::*;

        #[test]
        fn test_moderate_preset() {
            let validator = Password::moderate();
            assert!(validator.validate("Password1").is_ok());
            assert!(validator.validate("password").is_err()); // no uppercase/digit
            assert!(validator.validate("PASSWORD1").is_err()); // no lowercase
            assert!(validator.validate("Pass1").is_err()); // too short
        }

        #[test]
        fn test_strong_preset() {
            let validator = Password::strong();
            assert!(validator.validate("MyStr0ng!Pass").is_ok());
            assert!(validator.validate("Password1!").is_err()); // too short
            assert!(validator.validate("mystrongpass!").is_err()); // no uppercase/digit
        }
    }

    mod metadata {
        use super::*;

        #[test]
        fn test_metadata() {
            let validator = Password::new();
            let metadata = validator.metadata();
            assert_eq!(metadata.name, "Password");
            assert!(!metadata.cacheable); // Passwords should not be cached
            assert!(metadata.tags.contains(&"password".to_string()));
            assert!(metadata.tags.contains(&"security".to_string()));
        }
    }
}
