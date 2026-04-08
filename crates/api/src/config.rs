//! API Configuration
//!
//! Централизованная конфигурация для Nebula API server.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;

/// API Server Configuration
#[derive(Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// Host and port to bind (e.g. "0.0.0.0:8080")
    pub bind_address: SocketAddr,

    /// Request timeout
    pub request_timeout: Duration,

    /// Maximum request body size (bytes)
    pub max_body_size: usize,

    /// CORS allowed origins
    pub cors_allowed_origins: Vec<String>,

    /// Enable compression (gzip, brotli, zstd)
    pub enable_compression: bool,

    /// Enable request tracing
    pub enable_tracing: bool,

    /// JWT secret for authentication
    pub jwt_secret: String,

    /// Rate limiting: requests per second per IP
    pub rate_limit_per_second: u32,

    /// Static API keys accepted via `X-API-Key` header.
    ///
    /// Each key must have the `nbl_sk_` prefix. Keys are compared in constant
    /// time to prevent timing attacks. An empty list disables API key auth.
    #[serde(default)]
    pub api_keys: Vec<String>,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0:8080".parse().unwrap(),
            request_timeout: Duration::from_secs(30),
            max_body_size: 2 * 1024 * 1024, // 2MB
            cors_allowed_origins: vec!["*".to_string()],
            enable_compression: true,
            enable_tracing: true,
            jwt_secret: "dev-secret-change-in-production".to_string(),
            rate_limit_per_second: 100,
            api_keys: Vec::new(),
        }
    }
}

impl std::fmt::Debug for ApiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiConfig")
            .field("bind_address", &self.bind_address)
            .field("request_timeout", &self.request_timeout)
            .field("max_body_size", &self.max_body_size)
            .field("cors_allowed_origins", &self.cors_allowed_origins)
            .field("enable_compression", &self.enable_compression)
            .field("enable_tracing", &self.enable_tracing)
            .field("jwt_secret", &"[REDACTED]")
            .field("rate_limit_per_second", &self.rate_limit_per_second)
            .field("api_keys", &"[REDACTED]")
            .finish()
    }
}

impl ApiConfig {
    /// Load configuration from environment variables
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let bind_address = std::env::var("API_BIND_ADDRESS")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
            .parse()?;

        let request_timeout = std::env::var("API_REQUEST_TIMEOUT")
            .unwrap_or_else(|_| "30".to_string())
            .parse::<u64>()
            .map(Duration::from_secs)?;

        let max_body_size = std::env::var("API_MAX_BODY_SIZE")
            .unwrap_or_else(|_| "2097152".to_string())
            .parse()?;

        let cors_allowed_origins = std::env::var("API_CORS_ORIGINS")
            .unwrap_or_else(|_| "*".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .collect();

        let enable_compression = std::env::var("API_ENABLE_COMPRESSION")
            .unwrap_or_else(|_| "true".to_string())
            .parse()?;

        let enable_tracing = std::env::var("API_ENABLE_TRACING")
            .unwrap_or_else(|_| "true".to_string())
            .parse()?;

        let jwt_secret = std::env::var("API_JWT_SECRET")
            .unwrap_or_else(|_| "dev-secret-change-in-production".to_string());

        let rate_limit_per_second = std::env::var("API_RATE_LIMIT")
            .unwrap_or_else(|_| "100".to_string())
            .parse()?;

        // API keys: comma-separated list in `API_KEYS` env var.
        let api_keys = std::env::var("API_KEYS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();

        Ok(Self {
            bind_address,
            request_timeout,
            max_body_size,
            cors_allowed_origins,
            enable_compression,
            enable_tracing,
            jwt_secret,
            rate_limit_per_second,
            api_keys,
        })
    }
}
