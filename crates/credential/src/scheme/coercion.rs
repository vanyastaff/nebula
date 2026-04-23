//! Scheme coercion -- [`From`]/[`TryFrom`] conversions between scheme types.
//!
//! # Supported conversions
//!
//! | From | To | Condition |
//! |------|----|-----------|
//! | [`OAuth2Token`] | [`SecretToken`] | Always (extracts access_token) |

use super::{OAuth2Token, SecretToken};

impl From<OAuth2Token> for SecretToken {
    fn from(oauth: OAuth2Token) -> Self {
        SecretToken::new(oauth.access_token().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretString;

    #[test]
    fn oauth2_to_secret_token() {
        let oauth = OAuth2Token::new(SecretString::new("access-token-123"));
        let token: SecretToken = oauth.into();
        assert_eq!(token.token().expose_secret(), "access-token-123");
    }
}
