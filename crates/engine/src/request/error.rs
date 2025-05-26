use thiserror::Error;

use crate::request::ResponseError;

/// Errors that can occur when working with requests
#[derive(Error, Debug)]
pub enum RequestError {
    /// Invalid URL
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),

    /// Error when working with a proxy
    #[error("Proxy error: {0}")]
    ProxyError(String),

    /// Error with the request body
    #[error("Body error: {0}")]
    BodyError(String),

    /// Error building the request
    #[error("Build error: {0}")]
    BuildError(String),

    /// Error executing the request
    #[error("Request failed: {0}")]
    RequestFailed(String),

    /// Error in the response
    #[error("Response error: {0}")]
    ResponseError(#[from] ResponseError),

    /// Serialization/deserialization error
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    /// Authentication error
    #[error("Authentication error: {0}")]
    AuthError(String),

    /// Timeout error
    #[error("Timeout error: {0}")]
    TimeoutError(String),

    /// Incompatible request components
    #[error("Incompatible request components: {0}")]
    IncompatibleComponents(String),

    /// Other errors
    #[error("Request error: {0}")]
    Other(String),
}
