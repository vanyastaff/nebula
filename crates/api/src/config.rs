//! API Configuration
//!
//! Централизованная конфигурация для Nebula API server.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::Duration;

/// API Server Configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        }
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

        Ok(Self {
            bind_address,
            request_timeout,
            max_body_size,
            cors_allowed_origins,
            enable_compression,
            enable_tracing,
            jwt_secret,
            rate_limit_per_second,
        })
    }
}
