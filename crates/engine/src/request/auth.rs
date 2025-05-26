use std::collections::HashMap;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::request::{ApplyToRequest, RequestError, RequestOptions};

/// Authentication methods for HTTP requests
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RequestAuth {
    /// Basic authentication with username and password
    BasicAuth {
        /// Username for authentication
        username: String,
        /// Password for authentication
        password: String,
    },

    /// Bearer token authentication
    Bearer {
        /// Authentication token
        token: String,
    },

    /// OAuth2 authentication
    OAuth2 {
        /// Client ID for OAuth2
        client_id: String,
        /// Client secret for OAuth2
        client_secret: String,
        /// Access token (if available)
        #[serde(skip_serializing_if = "Option::is_none")]
        access_token: Option<String>,
        /// Refresh token (if available)
        #[serde(skip_serializing_if = "Option::is_none")]
        refresh_token: Option<String>,
        /// Token type (e.g., "Bearer")
        #[serde(skip_serializing_if = "Option::is_none")]
        token_type: Option<String>,
        /// OAuth2 scope
        #[serde(skip_serializing_if = "Option::is_none")]
        scope: Option<String>,
        /// URL for token endpoint
        #[serde(skip_serializing_if = "Option::is_none")]
        token_url: Option<String>,
        /// URL for authorization endpoint
        #[serde(skip_serializing_if = "Option::is_none")]
        auth_url: Option<String>,
        /// Expiration timestamp for the token
        #[serde(skip_serializing_if = "Option::is_none")]
        expires_at: Option<u64>,
    },

    /// API key in header
    ApiKeyHeader {
        /// Header name
        key: String,
        /// API key value
        value: String,
    },

    /// API key in query parameters
    ApiKeyQuery {
        /// Query parameter name
        key: String,
        /// API key value
        value: String,
    },

    /// Custom authentication with multiple headers
    CustomAuth {
        /// Custom headers for authentication
        headers: HashMap<String, String>,
    },

    /// No authentication
    None,
}

impl RequestAuth {
    /// Creates a basic authentication
    pub fn basic_auth(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self::BasicAuth {
            username: username.into(),
            password: password.into(),
        }
    }

    /// Creates a bearer token authentication
    pub fn bearer(token: impl Into<String>) -> Self {
        Self::Bearer {
            token: token.into(),
        }
    }

    /// Creates an OAuth2 authentication
    pub fn oauth2(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self::OAuth2 {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            access_token: None,
            refresh_token: None,
            token_type: None,
            scope: None,
            token_url: None,
            auth_url: None,
            expires_at: None,
        }
    }

    /// Creates an OAuth2 authentication with token
    pub fn oauth2_with_token(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        access_token: impl Into<String>,
        refresh_token: Option<String>,
        token_url: Option<String>,
    ) -> Self {
        Self::OAuth2 {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            access_token: Some(access_token.into()),
            refresh_token,
            token_type: Some("Bearer".to_string()), // Default token type
            scope: None,
            token_url,
            auth_url: None,
            expires_at: None,
        }
    }

    /// Creates an API key in header authentication
    pub fn api_key_header(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self::ApiKeyHeader {
            key: key.into(),
            value: value.into(),
        }
    }

    /// Creates an API key in query parameter authentication
    pub fn api_key_query(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self::ApiKeyQuery {
            key: key.into(),
            value: value.into(),
        }
    }

    /// Creates a custom authentication with multiple headers
    pub fn custom(headers: HashMap<String, String>) -> Self {
        Self::CustomAuth { headers }
    }

    /// Creates an empty authentication
    pub fn none() -> Self {
        Self::None
    }

    /// Updates the OAuth2 token
    pub fn update_oauth2_token(
        &mut self,
        access_token: impl Into<String>,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
    ) {
        if let Self::OAuth2 {
            access_token: token,
            refresh_token: refresh,
            expires_at: expires,
            ..
        } = self
        {
            // Update access token
            *token = Some(access_token.into());

            // Update refresh token if provided
            if let Some(rt) = refresh_token {
                *refresh = Some(rt);
            }

            // Update expiration time if provided
            if let Some(exp) = expires_in {
                let now: u64 = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                *expires = Some(now + exp);
            }
        }
    }

    /// Checks if the token has expired
    pub fn is_expired(&self) -> bool {
        match self {
            Self::OAuth2 {
                expires_at: Some(expires),
                ..
            } => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                // Consider token expired if less than 5 minutes remaining
                *expires <= now + 300
            }
            _ => false,
        }
    }
}

impl ApplyToRequest for RequestAuth {
    fn apply_to_options(&self, options: &mut RequestOptions) -> Result<(), RequestError> {
        // Save authentication in options
        options.auth = Some(self.clone());

        // If this is an API key in query parameters, add it to the request parameters
        if let Self::ApiKeyQuery { key, value } = self {
            options.query_params.insert(key.clone(), value.clone());
        }

        // For all other types except ApiKeyQuery, add headers
        match self {
            Self::BasicAuth { username, password } => {
                let auth_str = format!("{}:{}", username, password);
                let auth_header = format!("Basic {}", STANDARD.encode(auth_str));
                options
                    .headers
                    .insert("Authorization".to_string(), auth_header);
            }
            Self::Bearer { token } => {
                options
                    .headers
                    .insert("Authorization".to_string(), format!("Bearer {}", token));
            }
            Self::OAuth2 {
                access_token: Some(token),
                token_type: Some(ttype),
                ..
            } => {
                options
                    .headers
                    .insert("Authorization".to_string(), format!("{} {}", ttype, token));
            }
            Self::OAuth2 {
                access_token: Some(token),
                token_type: None,
                ..
            } => {
                options
                    .headers
                    .insert("Authorization".to_string(), format!("Bearer {}", token));
            }
            Self::OAuth2 {
                client_id,
                client_secret,
                ..
            } => {
                let auth_str = format!("{}:{}", client_id, client_secret);
                let auth_header = format!("Basic {}", STANDARD.encode(auth_str));
                options
                    .headers
                    .insert("Authorization".to_string(), auth_header);
            }
            Self::ApiKeyHeader { key, value } => {
                options.headers.insert(key.clone(), value.clone());
            }
            Self::CustomAuth { headers } => {
                for (key, value) in headers {
                    options.headers.insert(key.clone(), value.clone());
                }
            }
            Self::ApiKeyQuery { .. } => {} // Already handled above
            Self::None => {}
        }

        Ok(())
    }
}
