//! OAuth2 State — access token, refresh token, expiry.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::core::CredentialState;

/// Persisted state after a successful OAuth2 flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuth2State {
    pub access_token: String,
    /// Typically "Bearer"
    pub token_type: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<String>,
}

impl OAuth2State {
    /// Returns `true` if the access token is expired or expires within `margin`.
    #[must_use]
    pub fn is_expired(&self, margin: Duration) -> bool {
        match self.expires_at {
            None => false,
            Some(exp) => {
                let margin = chrono::Duration::from_std(margin).unwrap_or_default();
                Utc::now() + margin >= exp
            }
        }
    }

    /// `Authorization: Bearer <access_token>` header value.
    #[must_use]
    pub fn bearer_header(&self) -> String {
        format!("Bearer {}", self.access_token)
    }
}

impl CredentialState for OAuth2State {
    const VERSION: u16 = 1;
    const KIND: &'static str = "oauth2";

    fn scrub_ephemeral(&mut self) {
        // access_token and refresh_token must be stored for later use
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_state(expires_at: Option<DateTime<Utc>>) -> OAuth2State {
        OAuth2State {
            access_token: "tok_abc".into(),
            token_type: "Bearer".into(),
            refresh_token: None,
            expires_at,
            scopes: vec![],
        }
    }

    #[test]
    fn bearer_header_format() {
        let state = make_state(None);
        assert_eq!(state.bearer_header(), "Bearer tok_abc");
    }

    #[test]
    fn expired_token_detected() {
        let state = make_state(Some(Utc::now() - chrono::Duration::seconds(60)));
        assert!(state.is_expired(Duration::from_secs(0)));
    }

    #[test]
    fn valid_token_not_expired() {
        let state = make_state(Some(Utc::now() + chrono::Duration::seconds(300)));
        assert!(!state.is_expired(Duration::from_secs(0)));
    }

    #[test]
    fn no_expiry_never_expired() {
        let state = make_state(None);
        assert!(!state.is_expired(Duration::from_secs(9999)));
    }

    #[test]
    fn margin_detected() {
        // Expires in 30s but margin is 60s — should be considered expired
        let state = make_state(Some(Utc::now() + chrono::Duration::seconds(30)));
        assert!(state.is_expired(Duration::from_secs(60)));
    }
}
