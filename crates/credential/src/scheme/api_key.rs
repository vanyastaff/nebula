//! API key authentication with configurable placement.

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use crate::utils::SecretString;

/// Where the API key is placed in the request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ApiKeyPlacement {
    /// In a custom HTTP header (e.g., `X-API-Key`).
    Header {
        /// Header name.
        name: String,
    },
    /// As a query parameter (e.g., `?api_key=...`).
    QueryParam {
        /// Parameter name.
        name: String,
    },
}

/// API key with placement information.
///
/// Produced by: API key credential.
/// Consumed by: HTTP APIs that use custom header or query auth.
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiKeyAuth {
    key: SecretString,
    /// Where to place the key in the request.
    pub placement: ApiKeyPlacement,
}

impl ApiKeyAuth {
    /// Creates an API key placed in a custom HTTP header.
    pub fn header(name: impl Into<String>, key: SecretString) -> Self {
        Self {
            key,
            placement: ApiKeyPlacement::Header { name: name.into() },
        }
    }

    /// Creates an API key placed as a query parameter.
    pub fn query(name: impl Into<String>, key: SecretString) -> Self {
        Self {
            key,
            placement: ApiKeyPlacement::QueryParam { name: name.into() },
        }
    }

    /// Returns the key secret.
    pub fn key(&self) -> &SecretString {
        &self.key
    }
}

impl AuthScheme for ApiKeyAuth {}

impl std::fmt::Debug for ApiKeyAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeyAuth")
            .field("key", &"[REDACTED]")
            .field("placement", &self.placement)
            .finish()
    }
}
