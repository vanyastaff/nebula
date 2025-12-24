//! Semantic version validator (SemVer 2.0.0).
//!
//! Validates version strings according to the Semantic Versioning specification.

use crate::core::{ValidationComplexity, ValidationError, Validator, ValidatorMetadata};

// ============================================================================
// SEMVER VALIDATOR
// ============================================================================

/// Validates semantic version strings according to SemVer 2.0.0.
///
/// Format: `MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]`
///
/// Where:
/// - MAJOR, MINOR, PATCH are non-negative integers without leading zeros
/// - PRERELEASE is optional, dot-separated identifiers (alphanumeric + hyphens)
/// - BUILD is optional, dot-separated identifiers (alphanumeric + hyphens)
///
/// # Examples
///
/// ```
/// use nebula_validator::validators::text::Semver;
/// use nebula_validator::core::Validator;
///
/// let validator = Semver::new();
///
/// // Valid versions
/// assert!(validator.validate("1.0.0").is_ok());
/// assert!(validator.validate("0.1.0").is_ok());
/// assert!(validator.validate("1.2.3-alpha").is_ok());
/// assert!(validator.validate("1.2.3-alpha.1").is_ok());
/// assert!(validator.validate("1.2.3+build.123").is_ok());
/// assert!(validator.validate("1.2.3-beta.1+build.456").is_ok());
///
/// // Invalid versions
/// assert!(validator.validate("1.0").is_err()); // missing patch
/// assert!(validator.validate("v1.0.0").is_err()); // 'v' prefix
/// assert!(validator.validate("01.0.0").is_err()); // leading zero
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Semver {
    allow_v_prefix: bool,
    require_prerelease: bool,
    require_build: bool,
}

impl Semver {
    /// Creates a new semantic version validator with default settings.
    ///
    /// Default settings:
    /// - No 'v' prefix allowed
    /// - Prerelease not required
    /// - Build metadata not required
    #[must_use]
    pub fn new() -> Self {
        Self {
            allow_v_prefix: false,
            require_prerelease: false,
            require_build: false,
        }
    }

    /// Allow optional 'v' or 'V' prefix (e.g., `v1.0.0`).
    #[must_use = "builder methods must be chained or built"]
    pub fn allow_v_prefix(mut self) -> Self {
        self.allow_v_prefix = true;
        self
    }

    /// Require a prerelease identifier.
    #[must_use = "builder methods must be chained or built"]
    pub fn require_prerelease(mut self) -> Self {
        self.require_prerelease = true;
        self
    }

    /// Require build metadata.
    #[must_use = "builder methods must be chained or built"]
    pub fn require_build(mut self) -> Self {
        self.require_build = true;
        self
    }

    fn validate_numeric_identifier(&self, s: &str, name: &str) -> Result<(), ValidationError> {
        if s.is_empty() {
            return Err(ValidationError::new(
                "semver_empty_identifier",
                format!("{} version cannot be empty", name),
            ));
        }

        // Must be all digits
        if !s.chars().all(|c| c.is_ascii_digit()) {
            return Err(ValidationError::new(
                "semver_non_numeric",
                format!("{} version must be numeric", name),
            ));
        }

        // No leading zeros (except for "0" itself)
        if s.len() > 1 && s.starts_with('0') {
            return Err(ValidationError::new(
                "semver_leading_zero",
                format!("{} version cannot have leading zeros", name),
            ));
        }

        Ok(())
    }

    fn validate_prerelease_identifier(&self, s: &str) -> Result<(), ValidationError> {
        if s.is_empty() {
            return Err(ValidationError::new(
                "semver_empty_prerelease",
                "Prerelease identifier cannot be empty",
            ));
        }

        // Check for valid characters (alphanumeric + hyphen)
        if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(ValidationError::new(
                "semver_invalid_prerelease_char",
                "Prerelease identifiers must be alphanumeric or hyphen",
            ));
        }

        // If all digits, check for leading zeros
        if s.chars().all(|c| c.is_ascii_digit()) && s.len() > 1 && s.starts_with('0') {
            return Err(ValidationError::new(
                "semver_prerelease_leading_zero",
                "Numeric prerelease identifiers cannot have leading zeros",
            ));
        }

        Ok(())
    }

    fn validate_build_identifier(&self, s: &str) -> Result<(), ValidationError> {
        if s.is_empty() {
            return Err(ValidationError::new(
                "semver_empty_build",
                "Build metadata identifier cannot be empty",
            ));
        }

        // Check for valid characters (alphanumeric + hyphen)
        if !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(ValidationError::new(
                "semver_invalid_build_char",
                "Build metadata identifiers must be alphanumeric or hyphen",
            ));
        }

        // Note: Build metadata CAN have leading zeros (unlike prerelease)

        Ok(())
    }

    fn validate_prerelease(&self, s: &str) -> Result<(), ValidationError> {
        for part in s.split('.') {
            self.validate_prerelease_identifier(part)?;
        }
        Ok(())
    }

    fn validate_build(&self, s: &str) -> Result<(), ValidationError> {
        for part in s.split('.') {
            self.validate_build_identifier(part)?;
        }
        Ok(())
    }
}

impl Default for Semver {
    fn default() -> Self {
        Self::new()
    }
}

impl Validator for Semver {
    type Input = str;

    fn validate(&self, input: &str) -> Result<(), ValidationError> {
        if input.is_empty() {
            return Err(ValidationError::new(
                "empty_semver",
                "Version string cannot be empty",
            ));
        }

        let mut version = input;

        // Handle optional 'v' prefix
        if version.starts_with('v') || version.starts_with('V') {
            if !self.allow_v_prefix {
                return Err(ValidationError::new(
                    "semver_v_prefix",
                    "Version string cannot start with 'v' prefix",
                ));
            }
            version = &version[1..];
        }

        // Split build metadata first (+ takes precedence after -)
        let (version_with_prerelease, build) = match version.find('+') {
            Some(pos) => (&version[..pos], Some(&version[pos + 1..])),
            None => (version, None),
        };

        // Split prerelease
        let (core_version, prerelease) = match version_with_prerelease.find('-') {
            Some(pos) => (
                &version_with_prerelease[..pos],
                Some(&version_with_prerelease[pos + 1..]),
            ),
            None => (version_with_prerelease, None),
        };

        // Parse core version (MAJOR.MINOR.PATCH)
        let parts: Vec<&str> = core_version.split('.').collect();
        if parts.len() != 3 {
            return Err(ValidationError::new(
                "semver_invalid_format",
                format!(
                    "Version must have exactly 3 parts (MAJOR.MINOR.PATCH), found {}",
                    parts.len()
                ),
            ));
        }

        self.validate_numeric_identifier(parts[0], "Major")?;
        self.validate_numeric_identifier(parts[1], "Minor")?;
        self.validate_numeric_identifier(parts[2], "Patch")?;

        // Validate prerelease if present
        if let Some(pre) = prerelease {
            if pre.is_empty() {
                return Err(ValidationError::new(
                    "semver_empty_prerelease",
                    "Prerelease version cannot be empty after '-'",
                ));
            }
            self.validate_prerelease(pre)?;
        } else if self.require_prerelease {
            return Err(ValidationError::new(
                "semver_missing_prerelease",
                "Prerelease identifier is required",
            ));
        }

        // Validate build metadata if present
        if let Some(bld) = build {
            if bld.is_empty() {
                return Err(ValidationError::new(
                    "semver_empty_build",
                    "Build metadata cannot be empty after '+'",
                ));
            }
            self.validate_build(bld)?;
        } else if self.require_build {
            return Err(ValidationError::new(
                "semver_missing_build",
                "Build metadata is required",
            ));
        }

        Ok(())
    }

    fn metadata(&self) -> ValidatorMetadata {
        ValidatorMetadata {
            name: "Semver".to_string(),
            description: Some(format!(
                "Validates Semantic Versioning 2.0.0 (v prefix: {}, prerelease: {}, build: {})",
                if self.allow_v_prefix {
                    "allowed"
                } else {
                    "not allowed"
                },
                if self.require_prerelease {
                    "required"
                } else {
                    "optional"
                },
                if self.require_build {
                    "required"
                } else {
                    "optional"
                },
            )),
            complexity: ValidationComplexity::Linear,
            cacheable: true,
            estimated_time: Some(std::time::Duration::from_micros(5)),
            tags: vec![
                "text".to_string(),
                "semver".to_string(),
                "version".to_string(),
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

    mod valid {
        use super::*;

        #[test]
        fn test_simple_versions() {
            let validator = Semver::new();
            assert!(validator.validate("0.0.0").is_ok());
            assert!(validator.validate("0.0.1").is_ok());
            assert!(validator.validate("0.1.0").is_ok());
            assert!(validator.validate("1.0.0").is_ok());
            assert!(validator.validate("1.2.3").is_ok());
            assert!(validator.validate("10.20.30").is_ok());
            assert!(validator.validate("999.999.999").is_ok());
        }

        #[test]
        fn test_with_prerelease() {
            let validator = Semver::new();
            assert!(validator.validate("1.0.0-alpha").is_ok());
            assert!(validator.validate("1.0.0-alpha.1").is_ok());
            assert!(validator.validate("1.0.0-0.3.7").is_ok());
            assert!(validator.validate("1.0.0-x.7.z.92").is_ok());
            assert!(validator.validate("1.0.0-alpha-beta").is_ok());
            assert!(validator.validate("1.0.0-rc.1").is_ok());
            assert!(validator.validate("1.0.0-beta.11").is_ok());
        }

        #[test]
        fn test_with_build() {
            let validator = Semver::new();
            assert!(validator.validate("1.0.0+build").is_ok());
            assert!(validator.validate("1.0.0+build.123").is_ok());
            assert!(validator.validate("1.0.0+20130313144700").is_ok());
            assert!(validator.validate("1.0.0+exp.sha.5114f85").is_ok());
            // Build metadata CAN have leading zeros
            assert!(validator.validate("1.0.0+001").is_ok());
        }

        #[test]
        fn test_with_prerelease_and_build() {
            let validator = Semver::new();
            assert!(validator.validate("1.0.0-alpha+build").is_ok());
            assert!(validator.validate("1.0.0-alpha.1+build.123").is_ok());
            assert!(validator.validate("1.0.0-beta+exp.sha.5114f85").is_ok());
        }
    }

    mod invalid {
        use super::*;

        #[test]
        fn test_missing_parts() {
            let validator = Semver::new();
            assert!(validator.validate("1").is_err());
            assert!(validator.validate("1.0").is_err());
            assert!(validator.validate("1.0.0.0").is_err());
        }

        #[test]
        fn test_leading_zeros() {
            let validator = Semver::new();
            assert!(validator.validate("01.0.0").is_err());
            assert!(validator.validate("0.01.0").is_err());
            assert!(validator.validate("0.0.01").is_err());
        }

        #[test]
        fn test_v_prefix_not_allowed() {
            let validator = Semver::new();
            assert!(validator.validate("v1.0.0").is_err());
            assert!(validator.validate("V1.0.0").is_err());
        }

        #[test]
        fn test_invalid_prerelease() {
            let validator = Semver::new();
            assert!(validator.validate("1.0.0-").is_err()); // empty prerelease
            assert!(validator.validate("1.0.0-alpha..1").is_err()); // empty segment
            assert!(validator.validate("1.0.0-01").is_err()); // numeric with leading zero
            assert!(validator.validate("1.0.0-alpha_1").is_err()); // underscore not allowed
        }

        #[test]
        fn test_invalid_build() {
            let validator = Semver::new();
            assert!(validator.validate("1.0.0+").is_err()); // empty build
            assert!(validator.validate("1.0.0+build..1").is_err()); // empty segment
            assert!(validator.validate("1.0.0+build_1").is_err()); // underscore not allowed
        }

        #[test]
        fn test_non_numeric_core() {
            let validator = Semver::new();
            assert!(validator.validate("a.0.0").is_err());
            assert!(validator.validate("1.b.0").is_err());
            assert!(validator.validate("1.0.c").is_err());
        }

        #[test]
        fn test_empty_string() {
            let validator = Semver::new();
            assert!(validator.validate("").is_err());
        }
    }

    mod options {
        use super::*;

        #[test]
        fn test_allow_v_prefix() {
            let validator = Semver::new().allow_v_prefix();
            assert!(validator.validate("v1.0.0").is_ok());
            assert!(validator.validate("V1.0.0").is_ok());
            assert!(validator.validate("1.0.0").is_ok()); // still works without
        }

        #[test]
        fn test_require_prerelease() {
            let validator = Semver::new().require_prerelease();
            assert!(validator.validate("1.0.0-alpha").is_ok());
            assert!(validator.validate("1.0.0").is_err());
        }

        #[test]
        fn test_require_build() {
            let validator = Semver::new().require_build();
            assert!(validator.validate("1.0.0+build").is_ok());
            assert!(validator.validate("1.0.0").is_err());
            assert!(validator.validate("1.0.0-alpha+build").is_ok());
        }
    }

    mod real_world {
        use super::*;

        #[test]
        fn test_real_world_versions() {
            let validator = Semver::new();

            // Rust versions
            assert!(validator.validate("1.75.0").is_ok());
            assert!(validator.validate("1.76.0-beta.1").is_ok());
            assert!(validator.validate("1.77.0-nightly").is_ok());

            // Node.js versions
            assert!(validator.validate("20.10.0").is_ok());
            assert!(validator.validate("21.0.0-rc.1").is_ok());

            // Common patterns
            assert!(validator.validate("2.0.0-rc.1+build.123").is_ok());
            assert!(validator.validate("3.0.0-alpha.1.2.3").is_ok());
        }

        #[test]
        fn test_metadata() {
            let validator = Semver::new();
            let metadata = validator.metadata();
            assert_eq!(metadata.name, "Semver");
            assert!(metadata.tags.contains(&"semver".to_string()));
            assert!(metadata.tags.contains(&"version".to_string()));
        }
    }
}
