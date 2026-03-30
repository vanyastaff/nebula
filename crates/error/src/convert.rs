//! Error conversion utilities.
//!
//! Provides bidirectional mapping between [`ErrorCategory`] and HTTP status codes.
//! Additional protocol bridges (gRPC, etc.) will be added behind feature flags.

use crate::ErrorCategory;

impl ErrorCategory {
    /// Maps this category to an HTTP status code.
    ///
    /// The mapping follows standard HTTP semantics:
    ///
    /// | Category | HTTP Status |
    /// |----------|-------------|
    /// | NotFound | 404 |
    /// | Validation | 400 |
    /// | Authentication | 401 |
    /// | Authorization | 403 |
    /// | Conflict | 409 |
    /// | RateLimit | 429 |
    /// | Timeout | 504 |
    /// | Exhausted | 429 |
    /// | Cancelled | 499 |
    /// | Internal | 500 |
    /// | External | 502 |
    /// | Unsupported | 501 |
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorCategory;
    ///
    /// assert_eq!(ErrorCategory::NotFound.http_status_code(), 404);
    /// assert_eq!(ErrorCategory::Internal.http_status_code(), 500);
    /// ```
    #[must_use]
    pub const fn http_status_code(&self) -> u16 {
        match self {
            Self::NotFound => 404,
            Self::Validation => 400,
            Self::Authentication => 401,
            Self::Authorization => 403,
            Self::Conflict => 409,
            Self::RateLimit => 429,
            Self::Timeout => 504,
            Self::Exhausted => 429,
            Self::Cancelled => 499,
            Self::Internal => 500,
            Self::External => 502,
            Self::Unsupported => 501,
        }
    }

    /// Attempts to recover an error category from an HTTP status code.
    ///
    /// Returns `None` for status codes that don't map to a known category.
    /// When multiple categories share a status code (e.g. RateLimit and Exhausted
    /// both map to 429), the more common interpretation is returned (RateLimit).
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_error::ErrorCategory;
    ///
    /// assert_eq!(ErrorCategory::from_http_status(404), Some(ErrorCategory::NotFound));
    /// assert_eq!(ErrorCategory::from_http_status(418), None);
    /// ```
    #[must_use]
    pub const fn from_http_status(status: u16) -> Option<Self> {
        match status {
            400 => Some(Self::Validation),
            401 => Some(Self::Authentication),
            403 => Some(Self::Authorization),
            404 => Some(Self::NotFound),
            409 => Some(Self::Conflict),
            429 => Some(Self::RateLimit),
            499 => Some(Self::Cancelled),
            500 => Some(Self::Internal),
            501 => Some(Self::Unsupported),
            502 => Some(Self::External),
            504 => Some(Self::Timeout),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_maps_to_404() {
        assert_eq!(ErrorCategory::NotFound.http_status_code(), 404);
    }

    #[test]
    fn validation_maps_to_400() {
        assert_eq!(ErrorCategory::Validation.http_status_code(), 400);
    }

    #[test]
    fn authentication_maps_to_401() {
        assert_eq!(ErrorCategory::Authentication.http_status_code(), 401);
    }

    #[test]
    fn authorization_maps_to_403() {
        assert_eq!(ErrorCategory::Authorization.http_status_code(), 403);
    }

    #[test]
    fn conflict_maps_to_409() {
        assert_eq!(ErrorCategory::Conflict.http_status_code(), 409);
    }

    #[test]
    fn rate_limit_maps_to_429() {
        assert_eq!(ErrorCategory::RateLimit.http_status_code(), 429);
    }

    #[test]
    fn timeout_maps_to_504() {
        assert_eq!(ErrorCategory::Timeout.http_status_code(), 504);
    }

    #[test]
    fn exhausted_maps_to_429() {
        assert_eq!(ErrorCategory::Exhausted.http_status_code(), 429);
    }

    #[test]
    fn cancelled_maps_to_499() {
        assert_eq!(ErrorCategory::Cancelled.http_status_code(), 499);
    }

    #[test]
    fn internal_maps_to_500() {
        assert_eq!(ErrorCategory::Internal.http_status_code(), 500);
    }

    #[test]
    fn external_maps_to_502() {
        assert_eq!(ErrorCategory::External.http_status_code(), 502);
    }

    #[test]
    fn unsupported_maps_to_501() {
        assert_eq!(ErrorCategory::Unsupported.http_status_code(), 501);
    }

    #[test]
    fn round_trip_unique_statuses() {
        // All categories with unique HTTP status codes should round-trip.
        for cat in [
            ErrorCategory::NotFound,
            ErrorCategory::Validation,
            ErrorCategory::Authentication,
            ErrorCategory::Authorization,
            ErrorCategory::Conflict,
            ErrorCategory::RateLimit,
            ErrorCategory::Timeout,
            ErrorCategory::Cancelled,
            ErrorCategory::Internal,
            ErrorCategory::External,
            ErrorCategory::Unsupported,
        ] {
            let code = cat.http_status_code();
            let recovered = ErrorCategory::from_http_status(code);
            assert_eq!(recovered, Some(cat), "{cat} should round-trip via {code}");
        }
    }

    #[test]
    fn exhausted_maps_to_rate_limit_on_reverse() {
        // Exhausted and RateLimit both map to 429; reverse picks RateLimit.
        let code = ErrorCategory::Exhausted.http_status_code();
        assert_eq!(code, 429);
        assert_eq!(
            ErrorCategory::from_http_status(code),
            Some(ErrorCategory::RateLimit)
        );
    }

    #[test]
    fn unknown_status_returns_none() {
        assert_eq!(ErrorCategory::from_http_status(418), None);
    }
}
