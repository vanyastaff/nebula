//! Value size limits and validation for DoS protection
//!
//! This module provides configurable limits for Value operations to prevent
//! resource exhaustion attacks and accidental memory issues.

use crate::core::NebulaError;

/// Configurable limits for Value operations
///
/// # Example
///
/// ```
/// use nebula_value::ValueLimits;
///
/// let limits = ValueLimits::default();
/// assert_eq!(limits.max_array_length, 1_000_000);
///
/// // Custom limits
/// let strict = ValueLimits::strict();
/// assert_eq!(strict.max_array_length, 10_000);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueLimits {
    /// Maximum number of elements in an array
    pub max_array_length: usize,

    /// Maximum number of keys in an object
    pub max_object_keys: usize,

    /// Maximum string length in bytes
    pub max_string_bytes: usize,

    /// Maximum bytes length
    pub max_bytes_length: usize,

    /// Maximum depth for nested structures
    pub max_nesting_depth: usize,
}

impl Default for ValueLimits {
    fn default() -> Self {
        Self {
            max_array_length: 1_000_000,      // 1M elements
            max_object_keys: 100_000,         // 100k keys
            max_string_bytes: 10_000_000,     // 10MB
            max_bytes_length: 100_000_000,    // 100MB
            max_nesting_depth: 100,           // Same as path depth
        }
    }
}

impl ValueLimits {
    /// Permissive limits for trusted environments
    pub fn permissive() -> Self {
        Self {
            max_array_length: 10_000_000,
            max_object_keys: 1_000_000,
            max_string_bytes: 100_000_000,
            max_bytes_length: 1_000_000_000,
            max_nesting_depth: 200,
        }
    }

    /// Strict limits for untrusted input
    pub fn strict() -> Self {
        Self {
            max_array_length: 10_000,
            max_object_keys: 1_000,
            max_string_bytes: 1_000_000,      // 1MB
            max_bytes_length: 10_000_000,     // 10MB
            max_nesting_depth: 50,
        }
    }

    /// No limits (use with caution!)
    pub const fn unlimited() -> Self {
        Self {
            max_array_length: usize::MAX,
            max_object_keys: usize::MAX,
            max_string_bytes: usize::MAX,
            max_bytes_length: usize::MAX,
            max_nesting_depth: usize::MAX,
        }
    }

    /// Validate array length
    #[inline]
    pub fn check_array_length(&self, len: usize) -> Result<(), NebulaError> {
        if len > self.max_array_length {
            Err(NebulaError::validation(format!(
                "Array length {} exceeds maximum of {}",
                len, self.max_array_length
            )))
        } else {
            Ok(())
        }
    }

    /// Validate object key count
    #[inline]
    pub fn check_object_keys(&self, count: usize) -> Result<(), NebulaError> {
        if count > self.max_object_keys {
            Err(NebulaError::validation(format!(
                "Object key count {} exceeds maximum of {}",
                count, self.max_object_keys
            )))
        } else {
            Ok(())
        }
    }

    /// Validate string byte length
    #[inline]
    pub fn check_string_bytes(&self, bytes: usize) -> Result<(), NebulaError> {
        if bytes > self.max_string_bytes {
            Err(NebulaError::validation(format!(
                "String byte length {} exceeds maximum of {}",
                bytes, self.max_string_bytes
            )))
        } else {
            Ok(())
        }
    }

    /// Validate bytes length
    #[inline]
    pub fn check_bytes_length(&self, len: usize) -> Result<(), NebulaError> {
        if len > self.max_bytes_length {
            Err(NebulaError::validation(format!(
                "Bytes length {} exceeds maximum of {}",
                len, self.max_bytes_length
            )))
        } else {
            Ok(())
        }
    }

    /// Validate nesting depth
    #[inline]
    pub fn check_nesting_depth(&self, depth: usize) -> Result<(), NebulaError> {
        if depth > self.max_nesting_depth {
            Err(NebulaError::validation(format!(
                "Nesting depth {} exceeds maximum of {}",
                depth, self.max_nesting_depth
            )))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_limits() {
        let limits = ValueLimits::default();
        assert!(limits.check_array_length(1000).is_ok());
        assert!(limits.check_array_length(2_000_000).is_err());
    }

    #[test]
    fn test_strict_limits() {
        let limits = ValueLimits::strict();
        assert!(limits.check_array_length(5000).is_ok());
        assert!(limits.check_array_length(20_000).is_err());
    }

    #[test]
    fn test_permissive_limits() {
        let limits = ValueLimits::permissive();
        assert!(limits.check_array_length(5_000_000).is_ok());
    }

    #[test]
    fn test_unlimited() {
        let limits = ValueLimits::unlimited();
        assert!(limits.check_array_length(usize::MAX - 1).is_ok());
    }
}