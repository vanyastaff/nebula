//! Custom assertions for testing

use crate::testing::MockTokenCache;

/// Assert that a credential error is of a specific type
#[macro_export]
macro_rules! assert_credential_error {
    ($result:expr, NotFound) => {
        match $result {
            Err(CredentialError::NotFound { .. }) => (),
            other => panic!("Expected NotFound error, got: {:?}", other),
        }
    };
    ($result:expr, RefreshFailed) => {
        match $result {
            Err(CredentialError::RefreshFailed { .. }) => (),
            other => panic!("Expected RefreshFailed error, got: {:?}", other),
        }
    };
    ($result:expr, CasConflict) => {
        match $result {
            Err(CredentialError::CasConflict) => (),
            other => panic!("Expected CasConflict error, got: {:?}", other),
        }
    };
}

/// Assert token properties
#[macro_export]
macro_rules! assert_token {
    ($token:expr, type: $token_type:expr) => {
        assert_eq!($token.token_type, $token_type, "Token type mismatch");
    };
    ($token:expr, expired: false) => {
        assert!(!$token.is_expired(), "Token should not be expired");
    };
    ($token:expr, expired: true) => {
        assert!($token.is_expired(), "Token should be expired");
    };
    ($token:expr, has_scopes: $scopes:expr) => {
        assert_eq!($token.scopes, Some($scopes), "Scopes mismatch");
    };
}

/// Assert cache statistics
pub fn assert_cache_hit_rate(cache: &MockTokenCache, expected: f32, tolerance: f32) {
    let actual = cache.hit_rate();
    assert!(
        (actual - expected).abs() < tolerance,
        "Cache hit rate {:.2} not within tolerance of expected {:.2}",
        actual,
        expected
    );
}
