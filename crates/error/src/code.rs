//! Machine-readable error codes.

use std::borrow::Cow;
use std::fmt;

/// A machine-readable error code.
///
/// Error codes are short, unique identifiers for specific failure modes.
/// Prefer the canonical constants in [`codes`] for standard cases,
/// and [`ErrorCode::custom`] for domain-specific codes.
///
/// # Examples
///
/// ```
/// use nebula_error::{ErrorCode, codes};
///
/// let code = codes::NOT_FOUND;
/// assert_eq!(code.as_str(), "NOT_FOUND");
///
/// let custom = ErrorCode::custom("WORKFLOW_CYCLE_DETECTED");
/// assert_eq!(custom.as_str(), "WORKFLOW_CYCLE_DETECTED");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ErrorCode(Cow<'static, str>);

impl ErrorCode {
    /// Creates a new error code from a static string.
    ///
    /// Use this for compile-time constants. For runtime strings,
    /// use [`ErrorCode::custom`].
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorCode;
    ///
    /// const MY_CODE: ErrorCode = ErrorCode::new("MY_CODE");
    /// assert_eq!(MY_CODE.as_str(), "MY_CODE");
    /// ```
    pub const fn new(code: &'static str) -> Self {
        Self(Cow::Borrowed(code))
    }

    /// Creates a new error code from a runtime string.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorCode;
    ///
    /// let code = ErrorCode::custom(format!("PLUGIN_{}", "TIMEOUT"));
    /// assert_eq!(code.as_str(), "PLUGIN_TIMEOUT");
    /// ```
    pub fn custom(code: impl Into<String>) -> Self {
        Self(Cow::Owned(code.into()))
    }

    /// Returns the code as a string slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::codes;
    ///
    /// assert_eq!(codes::INTERNAL.as_str(), "INTERNAL");
    /// ```
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Allows comparing an [`ErrorCode`] directly with a `&str`.
///
/// # Examples
///
/// ```
/// use nebula_error::ErrorCode;
///
/// let code = ErrorCode::new("NOT_FOUND");
/// assert_eq!(code, "NOT_FOUND");
/// ```
impl PartialEq<&str> for ErrorCode {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// Allows comparing a `&str` directly with an [`ErrorCode`].
///
/// # Examples
///
/// ```
/// use nebula_error::ErrorCode;
///
/// let code = ErrorCode::new("NOT_FOUND");
/// assert_eq!("NOT_FOUND", code);
/// ```
impl PartialEq<ErrorCode> for &str {
    fn eq(&self, other: &ErrorCode) -> bool {
        *self == other.as_str()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ErrorCode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::custom(s))
    }
}

/// Canonical error codes matching [`ErrorCategory`](crate::ErrorCategory) variants.
pub mod codes {
    use super::ErrorCode;

    /// Resource not found.
    pub const NOT_FOUND: ErrorCode = ErrorCode::new("NOT_FOUND");
    /// Validation failure.
    pub const VALIDATION: ErrorCode = ErrorCode::new("VALIDATION");
    /// Authentication required or failed.
    pub const AUTHENTICATION: ErrorCode = ErrorCode::new("AUTHENTICATION");
    /// Authorization denied.
    pub const AUTHORIZATION: ErrorCode = ErrorCode::new("AUTHORIZATION");
    /// Conflict detected.
    pub const CONFLICT: ErrorCode = ErrorCode::new("CONFLICT");
    /// Rate limit exceeded.
    pub const RATE_LIMIT: ErrorCode = ErrorCode::new("RATE_LIMIT");
    /// Timeout exceeded.
    pub const TIMEOUT: ErrorCode = ErrorCode::new("TIMEOUT");
    /// Resource exhausted.
    pub const EXHAUSTED: ErrorCode = ErrorCode::new("EXHAUSTED");
    /// Operation cancelled.
    pub const CANCELLED: ErrorCode = ErrorCode::new("CANCELLED");
    /// Internal error.
    pub const INTERNAL: ErrorCode = ErrorCode::new("INTERNAL");
    /// External dependency failure.
    pub const EXTERNAL: ErrorCode = ErrorCode::new("EXTERNAL");
    /// Unsupported operation.
    pub const UNSUPPORTED: ErrorCode = ErrorCode::new("UNSUPPORTED");
    /// Service temporarily unavailable.
    pub const UNAVAILABLE: ErrorCode = ErrorCode::new("UNAVAILABLE");
    /// Payload too large.
    pub const DATA_TOO_LARGE: ErrorCode = ErrorCode::new("DATA_TOO_LARGE");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_code_as_str() {
        let code = ErrorCode::new("MY_CODE");
        assert_eq!(code.as_str(), "MY_CODE");
    }

    #[test]
    fn custom_code_as_str() {
        let code = ErrorCode::custom("DYNAMIC_CODE");
        assert_eq!(code.as_str(), "DYNAMIC_CODE");
    }

    #[test]
    fn static_and_custom_are_equal_when_same_string() {
        let static_code = ErrorCode::new("SAME");
        let custom_code = ErrorCode::custom("SAME");
        assert_eq!(static_code, custom_code);
    }

    #[test]
    fn display_matches_as_str() {
        let code = ErrorCode::new("DISPLAY_TEST");
        assert_eq!(code.to_string(), "DISPLAY_TEST");
    }

    #[test]
    fn canonical_codes_exist() {
        let all = [
            &codes::NOT_FOUND,
            &codes::VALIDATION,
            &codes::AUTHENTICATION,
            &codes::AUTHORIZATION,
            &codes::CONFLICT,
            &codes::RATE_LIMIT,
            &codes::TIMEOUT,
            &codes::EXHAUSTED,
            &codes::CANCELLED,
            &codes::INTERNAL,
            &codes::EXTERNAL,
            &codes::UNSUPPORTED,
        ];
        for code in &all {
            assert!(!code.as_str().is_empty());
        }
    }

    #[test]
    fn clone_preserves_equality() {
        let original = ErrorCode::custom("CLONED");
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn hash_is_consistent() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(ErrorCode::new("TEST"));
        assert!(set.contains(&ErrorCode::custom("TEST")));
    }

    #[test]
    fn error_code_eq_str() {
        let code = ErrorCode::new("MY_CODE");
        assert_eq!(code, "MY_CODE");
        assert_eq!("MY_CODE", code);
        assert_ne!(code, "OTHER");
    }
}
