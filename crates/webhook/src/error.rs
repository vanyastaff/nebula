//! Error types for webhook operations

use thiserror::Error;

/// Result type for webhook operations
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur during webhook operations
#[derive(Debug, Error)]
pub enum Error {
    /// Server configuration error
    #[error("Configuration error: {0}")]
    Config(String),

    /// Server failed to start
    #[error("Failed to start webhook server: {0}")]
    ServerStart(#[from] std::io::Error),

    /// Server bind error
    #[error("Failed to bind to address {address}: {source}")]
    BindFailed {
        /// Address that failed to bind
        address: String,
        /// Underlying error
        source: std::io::Error,
    },

    /// Route already registered
    #[error("Route '{path}' is already registered")]
    RouteConflict {
        /// Conflicting path
        path: String,
    },

    /// Route not found
    #[error("Route '{path}' not found")]
    RouteNotFound {
        /// Missing path
        path: String,
    },

    /// Invalid webhook path
    #[error("Invalid webhook path: {0}")]
    InvalidPath(String),

    /// Trigger operation failed
    #[error("Trigger operation failed: {0}")]
    TriggerFailed(String),

    /// Webhook payload parsing error
    #[error("Failed to parse webhook payload: {0}")]
    PayloadParse(String),

    /// Signature verification failed
    #[error("Webhook signature verification failed")]
    SignatureInvalid,

    /// Operation was cancelled
    #[error("Operation was cancelled")]
    Cancelled,

    /// Resource error
    #[error("Resource error: {0}")]
    Resource(#[from] nebula_resource::Error),

    /// Timeout error
    #[error("Operation timed out after {seconds}s")]
    Timeout {
        /// Seconds elapsed before timeout
        seconds: u64,
    },

    /// Generic error with context
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Create a configuration error
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }

    /// Create a route conflict error
    pub fn route_conflict(path: impl Into<String>) -> Self {
        Self::RouteConflict { path: path.into() }
    }

    /// Create a route not found error
    pub fn route_not_found(path: impl Into<String>) -> Self {
        Self::RouteNotFound { path: path.into() }
    }

    /// Create an invalid path error
    pub fn invalid_path(msg: impl Into<String>) -> Self {
        Self::InvalidPath(msg.into())
    }

    /// Create a trigger failed error
    pub fn trigger_failed(msg: impl Into<String>) -> Self {
        Self::TriggerFailed(msg.into())
    }

    /// Create a payload parse error
    pub fn payload_parse(msg: impl Into<String>) -> Self {
        Self::PayloadParse(msg.into())
    }

    /// Create a bind failed error
    pub fn bind_failed(address: impl Into<String>, source: std::io::Error) -> Self {
        Self::BindFailed {
            address: address.into(),
            source,
        }
    }

    /// Create a timeout error
    pub fn timeout(seconds: u64) -> Self {
        Self::Timeout { seconds }
    }

    /// Create a generic error
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}
