//! Validation utilities for nebula-value

#![allow(dead_code)]

// Placeholder validation module. Implement detailed validators as needed.

/// A simple result type alias for validation operations
pub type ValidationResult<T> = core::result::Result<T, crate::core::error::ValidationError>;

/// Validates that a string matches a minimal non-empty constraint
pub fn validate_non_empty(s: &str) -> ValidationResult<()> {
    if s.is_empty() {
        Err(crate::core::error::ValidationError::Failed { reason: "empty string".to_string() })
    } else {
        Ok(())
    }
}

/// Validates that a string matches the given regex pattern (feature-gated)
#[cfg(feature = "pattern")]
pub fn validate_pattern(s: &str, pattern: &regex::Regex) -> ValidationResult<()> {
    if pattern.is_match(s) { Ok(()) } else { Err(crate::core::error::ValidationError::PatternMismatch { value: s.to_string(), pattern: pattern.as_str().to_string() }) }
}
