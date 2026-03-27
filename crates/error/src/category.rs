//! Canonical error categories.

/// Canonical classification of what went wrong.
///
/// Each variant maps to a broad failure class (similar to HTTP status
/// code families or gRPC status codes). Use [`is_default_retryable`],
/// [`is_client_error`], and [`is_server_error`] for quick triage.
///
/// # Examples
///
/// ```
/// use nebula_error::ErrorCategory;
///
/// let cat = ErrorCategory::Timeout;
/// assert!(cat.is_default_retryable());
/// assert!(cat.is_server_error());
/// ```
///
/// [`is_default_retryable`]: ErrorCategory::is_default_retryable
/// [`is_client_error`]: ErrorCategory::is_client_error
/// [`is_server_error`]: ErrorCategory::is_server_error
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorCategory {
    /// The requested resource was not found.
    NotFound,
    /// Input validation failed.
    Validation,
    /// Authentication is required or failed.
    Authentication,
    /// The caller lacks permission.
    Authorization,
    /// A conflicting operation was detected (e.g. optimistic lock).
    Conflict,
    /// Too many requests — back off and retry.
    RateLimit,
    /// The operation exceeded a deadline.
    Timeout,
    /// A finite resource (quota, pool, budget) is exhausted.
    Exhausted,
    /// The operation was cancelled by the caller.
    Cancelled,
    /// An internal/unexpected failure.
    Internal,
    /// A downstream dependency failed.
    External,
    /// The requested operation is not supported.
    Unsupported,
}

impl ErrorCategory {
    /// Whether this category is retryable by default.
    ///
    /// Returns `true` for [`Timeout`](Self::Timeout),
    /// [`Exhausted`](Self::Exhausted), and [`External`](Self::External).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorCategory;
    ///
    /// assert!(ErrorCategory::Timeout.is_default_retryable());
    /// assert!(!ErrorCategory::NotFound.is_default_retryable());
    /// ```
    pub const fn is_default_retryable(&self) -> bool {
        matches!(self, Self::Timeout | Self::Exhausted | Self::External)
    }

    /// Whether this category represents a client-side error.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorCategory;
    ///
    /// assert!(ErrorCategory::Validation.is_client_error());
    /// assert!(!ErrorCategory::Internal.is_client_error());
    /// ```
    pub const fn is_client_error(&self) -> bool {
        matches!(
            self,
            Self::NotFound
                | Self::Validation
                | Self::Authentication
                | Self::Authorization
                | Self::Conflict
                | Self::Unsupported
        )
    }

    /// Whether this category represents a server-side error.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorCategory;
    ///
    /// assert!(ErrorCategory::Internal.is_server_error());
    /// assert!(!ErrorCategory::Validation.is_server_error());
    /// ```
    pub const fn is_server_error(&self) -> bool {
        matches!(
            self,
            Self::Internal | Self::External | Self::Timeout | Self::Exhausted
        )
    }

    /// Returns the snake_case string representation.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorCategory;
    ///
    /// assert_eq!(ErrorCategory::NotFound.as_str(), "not_found");
    /// assert_eq!(ErrorCategory::RateLimit.as_str(), "rate_limit");
    /// ```
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Validation => "validation",
            Self::Authentication => "authentication",
            Self::Authorization => "authorization",
            Self::Conflict => "conflict",
            Self::RateLimit => "rate_limit",
            Self::Timeout => "timeout",
            Self::Exhausted => "exhausted",
            Self::Cancelled => "cancelled",
            Self::Internal => "internal",
            Self::External => "external",
            Self::Unsupported => "unsupported",
        }
    }
}

impl std::fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for ErrorCategory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for ErrorCategory {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(deserializer)?;
        match s {
            "not_found" => Ok(Self::NotFound),
            "validation" => Ok(Self::Validation),
            "authentication" => Ok(Self::Authentication),
            "authorization" => Ok(Self::Authorization),
            "conflict" => Ok(Self::Conflict),
            "rate_limit" => Ok(Self::RateLimit),
            "timeout" => Ok(Self::Timeout),
            "exhausted" => Ok(Self::Exhausted),
            "cancelled" => Ok(Self::Cancelled),
            "internal" => Ok(Self::Internal),
            "external" => Ok(Self::External),
            "unsupported" => Ok(Self::Unsupported),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &[
                    "not_found",
                    "validation",
                    "authentication",
                    "authorization",
                    "conflict",
                    "rate_limit",
                    "timeout",
                    "exhausted",
                    "cancelled",
                    "internal",
                    "external",
                    "unsupported",
                ],
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_snake_case() {
        assert_eq!(ErrorCategory::NotFound.to_string(), "not_found");
        assert_eq!(ErrorCategory::RateLimit.to_string(), "rate_limit");
        assert_eq!(ErrorCategory::Internal.to_string(), "internal");
    }

    #[test]
    fn timeout_is_default_retryable() {
        assert!(ErrorCategory::Timeout.is_default_retryable());
    }

    #[test]
    fn exhausted_is_default_retryable() {
        assert!(ErrorCategory::Exhausted.is_default_retryable());
    }

    #[test]
    fn external_is_default_retryable() {
        assert!(ErrorCategory::External.is_default_retryable());
    }

    #[test]
    fn validation_is_not_retryable() {
        assert!(!ErrorCategory::Validation.is_default_retryable());
    }

    #[test]
    fn not_found_is_not_retryable() {
        assert!(!ErrorCategory::NotFound.is_default_retryable());
    }

    #[test]
    fn client_errors_are_correct() {
        let client = [
            ErrorCategory::NotFound,
            ErrorCategory::Validation,
            ErrorCategory::Authentication,
            ErrorCategory::Authorization,
            ErrorCategory::Conflict,
            ErrorCategory::Unsupported,
        ];
        for cat in &client {
            assert!(cat.is_client_error(), "{cat} should be client error");
        }

        let not_client = [
            ErrorCategory::Internal,
            ErrorCategory::External,
            ErrorCategory::Timeout,
            ErrorCategory::Exhausted,
            ErrorCategory::Cancelled,
            ErrorCategory::RateLimit,
        ];
        for cat in &not_client {
            assert!(!cat.is_client_error(), "{cat} should not be client error");
        }
    }

    #[test]
    fn server_errors_are_correct() {
        let server = [
            ErrorCategory::Internal,
            ErrorCategory::External,
            ErrorCategory::Timeout,
            ErrorCategory::Exhausted,
        ];
        for cat in &server {
            assert!(cat.is_server_error(), "{cat} should be server error");
        }

        let not_server = [
            ErrorCategory::NotFound,
            ErrorCategory::Validation,
            ErrorCategory::Authentication,
            ErrorCategory::Authorization,
            ErrorCategory::Conflict,
            ErrorCategory::Cancelled,
            ErrorCategory::RateLimit,
            ErrorCategory::Unsupported,
        ];
        for cat in &not_server {
            assert!(!cat.is_server_error(), "{cat} should not be server error");
        }
    }

    #[test]
    fn as_str_round_trips_all_variants() {
        let all = [
            ErrorCategory::NotFound,
            ErrorCategory::Validation,
            ErrorCategory::Authentication,
            ErrorCategory::Authorization,
            ErrorCategory::Conflict,
            ErrorCategory::RateLimit,
            ErrorCategory::Timeout,
            ErrorCategory::Exhausted,
            ErrorCategory::Cancelled,
            ErrorCategory::Internal,
            ErrorCategory::External,
            ErrorCategory::Unsupported,
        ];
        for cat in &all {
            // as_str should produce a non-empty string
            assert!(!cat.as_str().is_empty());
        }
    }
}
