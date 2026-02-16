//! Hostname validator (RFC 1123).
//!
//! Validates hostnames according to RFC 1123 rules:
//! - Total length: 1..=253 characters (excluding optional trailing dot)
//! - Split by `.` into labels
//! - Each label: 1..=63 characters, `[a-zA-Z0-9-]` only
//! - Labels must not start or end with a hyphen
//! - At least one label required
//! - Trailing dot optional (FQDN)

use crate::foundation::{Validate, ValidationError};

// ============================================================================
// HOSTNAME VALIDATOR
// ============================================================================

/// Validates hostnames per RFC 1123.
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::Hostname;
/// use nebula_validator::foundation::Validate;
///
/// let v = Hostname;
/// assert!(v.validate("example.com").is_ok());
/// assert!(v.validate("localhost").is_ok());
/// assert!(v.validate("example.com.").is_ok()); // trailing dot FQDN
/// assert!(v.validate("").is_err());
/// assert!(v.validate("-bad.com").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hostname;

impl Validate for Hostname {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.is_empty() {
            return Err(ValidationError::new(
                "empty_hostname",
                "Hostname cannot be empty",
            ));
        }

        // Strip optional trailing dot (FQDN notation)
        let hostname = input.strip_suffix('.').unwrap_or(input);

        // After stripping trailing dot, the hostname part must not be empty
        if hostname.is_empty() {
            return Err(ValidationError::new(
                "invalid_hostname",
                "Hostname must contain at least one label",
            ));
        }

        // Total length check (excluding trailing dot)
        if hostname.len() > 253 {
            return Err(ValidationError::new(
                "hostname_too_long",
                format!(
                    "Hostname length {} exceeds maximum of 253 characters",
                    hostname.len()
                ),
            ));
        }

        // Split into labels and validate each
        for label in hostname.split('.') {
            if label.is_empty() {
                return Err(ValidationError::new(
                    "empty_label",
                    "Hostname labels must not be empty",
                ));
            }

            if label.len() > 63 {
                return Err(ValidationError::new(
                    "label_too_long",
                    format!(
                        "Label '{}' length {} exceeds maximum of 63 characters",
                        label,
                        label.len()
                    ),
                ));
            }

            if label.starts_with('-') {
                return Err(ValidationError::new(
                    "label_starts_with_hyphen",
                    format!("Label '{label}' must not start with a hyphen"),
                ));
            }

            if label.ends_with('-') {
                return Err(ValidationError::new(
                    "label_ends_with_hyphen",
                    format!("Label '{label}' must not end with a hyphen"),
                ));
            }

            // Check all characters are valid: [a-zA-Z0-9-]
            if let Some(ch) = label
                .chars()
                .find(|c| !c.is_ascii_alphanumeric() && *c != '-')
            {
                return Err(ValidationError::new(
                    "invalid_hostname_character",
                    format!("Label '{label}' contains invalid character '{ch}'"),
                ));
            }
        }

        Ok(())
    }

    crate::validator_metadata!(
        "Hostname",
        "Validates hostnames per RFC 1123",
        complexity = Linear,
        tags = ["network", "hostname"]
    );
}

/// Creates a new [`Hostname`] validator.
#[must_use]
pub const fn hostname() -> Hostname {
    Hostname
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Valid hostnames ---

    #[test]
    fn valid_simple_domain() {
        let v = hostname();
        assert!(v.validate("example.com").is_ok());
    }

    #[test]
    fn valid_subdomain() {
        let v = hostname();
        assert!(v.validate("api.example.com").is_ok());
    }

    #[test]
    fn valid_localhost() {
        let v = hostname();
        assert!(v.validate("localhost").is_ok());
    }

    #[test]
    fn valid_single_char() {
        let v = hostname();
        assert!(v.validate("a").is_ok());
    }

    #[test]
    fn valid_multi_label_short() {
        let v = hostname();
        assert!(v.validate("a.b.c").is_ok());
    }

    #[test]
    fn valid_uppercase() {
        let v = hostname();
        assert!(v.validate("EXAMPLE.COM").is_ok());
    }

    #[test]
    fn valid_hyphenated() {
        let v = hostname();
        assert!(v.validate("my-host.example.com").is_ok());
    }

    #[test]
    fn valid_trailing_dot_fqdn() {
        let v = hostname();
        assert!(v.validate("example.com.").is_ok());
    }

    #[test]
    fn valid_numeric_label() {
        let v = hostname();
        assert!(v.validate("123.456.789").is_ok());
    }

    #[test]
    fn valid_max_label_length() {
        let v = hostname();
        let label = "a".repeat(63);
        assert!(v.validate(&label).is_ok());
    }

    // --- Invalid hostnames ---

    #[test]
    fn invalid_empty() {
        let v = hostname();
        assert!(v.validate("").is_err());
    }

    #[test]
    fn invalid_single_dot() {
        let v = hostname();
        assert!(v.validate(".").is_err());
    }

    #[test]
    fn invalid_double_dot() {
        let v = hostname();
        assert!(v.validate("..").is_err());
    }

    #[test]
    fn invalid_leading_hyphen() {
        let v = hostname();
        assert!(v.validate("-example.com").is_err());
    }

    #[test]
    fn invalid_trailing_hyphen() {
        let v = hostname();
        assert!(v.validate("example-.com").is_err());
    }

    #[test]
    fn invalid_label_too_long() {
        let v = hostname();
        let label = "a".repeat(64);
        let host = format!("{label}.com");
        assert!(v.validate(&host).is_err());
    }

    #[test]
    fn invalid_total_too_long() {
        let v = hostname();
        // Build a hostname > 253 chars from valid labels
        // 63-char labels separated by dots: 63 + 1 + 63 + 1 + 63 + 1 + 63 = 255
        let label = "a".repeat(63);
        let host = format!("{label}.{label}.{label}.{label}");
        assert!(host.len() > 253);
        assert!(v.validate(&host).is_err());
    }

    #[test]
    fn invalid_space_in_hostname() {
        let v = hostname();
        assert!(v.validate("exam ple.com").is_err());
    }

    #[test]
    fn invalid_empty_label_double_dot() {
        let v = hostname();
        assert!(v.validate("example..com").is_err());
    }

    #[test]
    fn invalid_underscore() {
        let v = hostname();
        assert!(v.validate("my_host.com").is_err());
    }

    #[test]
    fn invalid_leading_dot() {
        let v = hostname();
        assert!(v.validate(".example.com").is_err());
    }
}
