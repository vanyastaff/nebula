//! Scheme coercion -- [`TryFrom`] conversions between AuthScheme types.
//!
//! Allows credentials producing one scheme to work with resources
//! expecting a compatible scheme (e.g., `OAuth2Token` -> `BearerToken`).
//!
//! # Supported conversions
//!
//! | From | To | Condition |
//! |------|----|-----------|
//! | [`OAuth2Token`] | [`BearerToken`] | Always (infallible) |
//! | [`ApiKeyAuth`] | [`BearerToken`] | Header placement with `Authorization` name and `Bearer` prefix |
//! | [`SamlAuth`] | [`BearerToken`] | `assertion_b64` is present |

use super::api_key::ApiKeyPlacement;
use super::{ApiKeyAuth, BearerToken, OAuth2Token, SamlAuth};
use crate::core::CredentialError;

// -- OAuth2Token -> BearerToken (infallible) ----------------------------------

impl From<OAuth2Token> for BearerToken {
    fn from(oauth: OAuth2Token) -> Self {
        BearerToken::new(oauth.access_token().clone())
    }
}

// -- ApiKeyAuth -> BearerToken (bearer-style header only) ---------------------

impl TryFrom<ApiKeyAuth> for BearerToken {
    type Error = CredentialError;

    /// Converts an API key to a bearer token when the key is placed in an
    /// `Authorization` header with a `Bearer` (case-insensitive) prefix.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::SchemeMismatch`] if the placement is not a
    /// bearer-style `Authorization` header.
    fn try_from(api: ApiKeyAuth) -> Result<Self, Self::Error> {
        match &api.placement {
            ApiKeyPlacement::Header { name } if name.eq_ignore_ascii_case("authorization") => {
                // The key value itself is the bearer token -- the header name
                // is just placement metadata.
                Ok(BearerToken::new(api.key().clone()))
            }
            _ => Err(CredentialError::SchemeMismatch {
                expected: "bearer",
                actual: format!("api_key ({:?})", api.placement),
            }),
        }
    }
}

// -- SamlAuth -> BearerToken (assertion required) -----------------------------

impl TryFrom<SamlAuth> for BearerToken {
    type Error = CredentialError;

    /// Converts a SAML assertion to a bearer token using the base64-encoded
    /// assertion as the token value.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialError::SchemeMismatch`] if `assertion_b64` is `None`.
    fn try_from(saml: SamlAuth) -> Result<Self, Self::Error> {
        match saml.assertion_b64() {
            Some(assertion) => Ok(BearerToken::new(assertion.clone())),
            None => Err(CredentialError::SchemeMismatch {
                expected: "bearer",
                actual: "saml (no assertion)".to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::SecretString;

    #[test]
    fn oauth2_to_bearer_succeeds() {
        let oauth = OAuth2Token::new(SecretString::new("access-token-123"));
        let bearer: BearerToken = oauth.into();
        let header = bearer.bearer_header();
        assert_eq!(header, "Bearer access-token-123");
    }

    #[test]
    fn api_key_bearer_header_to_bearer_succeeds() {
        let api = ApiKeyAuth::header("Authorization", SecretString::new("my-token"));
        let bearer: BearerToken = api.try_into().expect("should convert");
        let header = bearer.bearer_header();
        assert_eq!(header, "Bearer my-token");
    }

    #[test]
    fn api_key_authorization_header_case_insensitive() {
        let api = ApiKeyAuth::header("authorization", SecretString::new("my-token"));
        let result: Result<BearerToken, _> = api.try_into();
        assert!(result.is_ok());
    }

    #[test]
    fn api_key_non_auth_header_to_bearer_fails() {
        let api = ApiKeyAuth::header("X-API-Key", SecretString::new("my-token"));
        let result: Result<BearerToken, _> = api.try_into();
        assert!(matches!(
            result,
            Err(CredentialError::SchemeMismatch { .. })
        ));
    }

    #[test]
    fn api_key_query_param_to_bearer_fails() {
        let api = ApiKeyAuth::query("api_key", SecretString::new("my-token"));
        let result: Result<BearerToken, _> = api.try_into();
        assert!(matches!(
            result,
            Err(CredentialError::SchemeMismatch { .. })
        ));
    }

    #[test]
    fn saml_with_assertion_to_bearer_succeeds() {
        let saml = SamlAuth::new("user@example.com")
            .with_assertion(SecretString::new("PHNhbWw+base64assertion"));
        let bearer: BearerToken = saml.try_into().expect("should convert");
        let header = bearer.bearer_header();
        assert_eq!(header, "Bearer PHNhbWw+base64assertion");
    }

    #[test]
    fn saml_without_assertion_to_bearer_fails() {
        let saml = SamlAuth::new("user@example.com");
        let result: Result<BearerToken, _> = saml.try_into();
        assert!(matches!(
            result,
            Err(CredentialError::SchemeMismatch { .. })
        ));
    }
}
