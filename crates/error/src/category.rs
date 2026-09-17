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
    /// The service is temporarily unavailable (overloaded, maintenance).
    Unavailable,
    /// The payload or data exceeds size limits.
    DataTooLarge,
}

impl ErrorCategory {
    /// Every variant, in declaration order.
    ///
    /// `Self::named`'s `match` is what keeps this list honest: it has no
    /// wildcard arm, so a variant added to the enum without a matching arm
    /// there fails to **compile**, not merely to pass a round-trip test.
    /// This crate's own tests and `tests/serde.rs`'s `all_categories_roundtrip`
    /// iterate this list rather than each keeping a separate hand copy,
    /// which can silently fall behind the enum as it grows.
    ///
    /// A slice, not an array: the enum is `#[non_exhaustive]` so that adding a
    /// variant is not a breaking change, and an array length in the public
    /// type would make it one.
    pub const ALL: &[ErrorCategory] = &[
        Self::named(Self::NotFound),
        Self::named(Self::Validation),
        Self::named(Self::Authentication),
        Self::named(Self::Authorization),
        Self::named(Self::Conflict),
        Self::named(Self::RateLimit),
        Self::named(Self::Timeout),
        Self::named(Self::Exhausted),
        Self::named(Self::Cancelled),
        Self::named(Self::Internal),
        Self::named(Self::External),
        Self::named(Self::Unsupported),
        Self::named(Self::Unavailable),
        Self::named(Self::DataTooLarge),
    ];

    /// Identity function whose only purpose is its exhaustive `match`: with
    /// no wildcard arm, adding a variant to the enum without an arm here is
    /// a compile error, which is what [`Self::ALL`] leans on to stay
    /// complete.
    const fn named(category: Self) -> Self {
        match category {
            Self::NotFound => category,
            Self::Validation => category,
            Self::Authentication => category,
            Self::Authorization => category,
            Self::Conflict => category,
            Self::RateLimit => category,
            Self::Timeout => category,
            Self::Exhausted => category,
            Self::Cancelled => category,
            Self::Internal => category,
            Self::External => category,
            Self::Unsupported => category,
            Self::Unavailable => category,
            Self::DataTooLarge => category,
        }
    }

    /// Whether this category is retryable by default.
    ///
    /// Returns `true` for transient failures that may succeed on retry:
    /// [`Timeout`](Self::Timeout), [`Exhausted`](Self::Exhausted),
    /// [`External`](Self::External), [`RateLimit`](Self::RateLimit),
    /// and [`Unavailable`](Self::Unavailable).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorCategory;
    ///
    /// assert!(ErrorCategory::Timeout.is_default_retryable());
    /// assert!(ErrorCategory::RateLimit.is_default_retryable());
    /// assert!(ErrorCategory::Unavailable.is_default_retryable());
    /// assert!(!ErrorCategory::NotFound.is_default_retryable());
    /// ```
    pub const fn is_default_retryable(&self) -> bool {
        matches!(
            self,
            Self::Timeout | Self::Exhausted | Self::External | Self::RateLimit | Self::Unavailable
        )
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
                | Self::DataTooLarge
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
            Self::Internal | Self::External | Self::Timeout | Self::Exhausted | Self::Unavailable
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
            Self::Unavailable => "unavailable",
            Self::DataTooLarge => "data_too_large",
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
        /// Matches the wire string against the known category names.
        ///
        /// `visit_str` (not `<&str>::deserialize`) so a caller decoding from an
        /// already-owned buffer — `serde_json::from_value`, `serde_yaml`, any
        /// format whose deserializer cannot hand back a borrow into transient
        /// storage — still succeeds. `Visitor::visit_borrowed_str`'s default
        /// implementation forwards to `visit_str`, so the borrowed-input path
        /// stays zero-alloc: no owned `String` is built either way.
        struct CategoryVisitor;

        impl serde::de::Visitor<'_> for CategoryVisitor {
            type Value = ErrorCategory;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a snake_case error category string")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                match v {
                    "not_found" => Ok(ErrorCategory::NotFound),
                    "validation" => Ok(ErrorCategory::Validation),
                    "authentication" => Ok(ErrorCategory::Authentication),
                    "authorization" => Ok(ErrorCategory::Authorization),
                    "conflict" => Ok(ErrorCategory::Conflict),
                    "rate_limit" => Ok(ErrorCategory::RateLimit),
                    "timeout" => Ok(ErrorCategory::Timeout),
                    "exhausted" => Ok(ErrorCategory::Exhausted),
                    "cancelled" => Ok(ErrorCategory::Cancelled),
                    "internal" => Ok(ErrorCategory::Internal),
                    "external" => Ok(ErrorCategory::External),
                    "unsupported" => Ok(ErrorCategory::Unsupported),
                    "unavailable" => Ok(ErrorCategory::Unavailable),
                    "data_too_large" => Ok(ErrorCategory::DataTooLarge),
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
                            "unavailable",
                            "data_too_large",
                        ],
                    )),
                }
            }
        }

        deserializer.deserialize_str(CategoryVisitor)
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
    fn rate_limit_is_default_retryable() {
        assert!(ErrorCategory::RateLimit.is_default_retryable());
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
    fn unavailable_is_default_retryable() {
        assert!(ErrorCategory::Unavailable.is_default_retryable());
    }

    #[test]
    fn data_too_large_is_not_default_retryable() {
        assert!(!ErrorCategory::DataTooLarge.is_default_retryable());
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
            ErrorCategory::DataTooLarge,
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
            ErrorCategory::Unavailable,
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
            ErrorCategory::Unavailable,
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
            ErrorCategory::DataTooLarge,
        ];
        for cat in &not_server {
            assert!(!cat.is_server_error(), "{cat} should not be server error");
        }
    }

    /// `as_str` must give every variant a distinct, non-empty name: a blank
    /// or colliding name would make two categories indistinguishable on the
    /// wire and in logs.
    #[test]
    fn as_str_names_every_variant_with_a_distinct_string() {
        let mut names = std::collections::HashSet::new();
        for category in ErrorCategory::ALL {
            let name = category.as_str();
            assert!(!name.is_empty(), "{category:?}");
            assert!(names.insert(name), "duplicate as_str() name: {name}");
        }
        assert_eq!(names.len(), ErrorCategory::ALL.len());
    }

    /// Every variant must decode back to itself through both `Deserialize`
    /// entry points: `from_str` (a source-text deserializer) and
    /// `from_value` (an already-owned `Value`, which cannot hand back a
    /// borrow — see `category_decodes_from_an_owned_value`). Iterates
    /// [`ErrorCategory::ALL`] rather than a hand copy, so a variant added to
    /// the enum is covered here without editing this test.
    #[cfg(feature = "serde")]
    #[test]
    fn every_variant_round_trips_through_from_str_and_from_value() {
        for category in ErrorCategory::ALL.iter().copied() {
            let json = serde_json::to_string(&category).expect("encodes");
            let via_str: ErrorCategory =
                serde_json::from_str(&json).expect("from_str decodes a valid category");
            assert_eq!(via_str, category, "from_str round-trip for {category:?}");

            let value = serde_json::to_value(category).expect("encodes to a Value");
            let via_value: ErrorCategory =
                serde_json::from_value(value).expect("from_value decodes a valid category");
            assert_eq!(
                via_value, category,
                "from_value round-trip for {category:?}"
            );
        }
    }

    /// `serde_json::from_value` hands the deserializer an already-owned
    /// `Value`, which cannot yield a borrow into a transient buffer the way
    /// `from_str` can. Red-on-revert: with the `Deserialize` impl back to
    /// `<&str>::deserialize`, this fails with "invalid type: string
    /// \"internal\", expected a borrowed string" — the exact failure that made
    /// `serde_json::from_value::<ErrorEnvelope>` refuse every valid envelope.
    #[cfg(feature = "serde")]
    #[test]
    fn category_decodes_from_an_owned_value() {
        let decoded: ErrorCategory =
            serde_json::from_value(serde_json::json!("internal")).expect("owned value decodes");
        assert_eq!(decoded, ErrorCategory::Internal);
    }
}
