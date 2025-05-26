use core::fmt;
use core::time::Duration;
use std::collections::HashMap;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// Error types that can occur when handling responses
#[derive(Error, Debug)]
pub enum ResponseError {
    /// HTTP error with status code
    #[error("HTTP error {status}: {message}")]
    HttpError { status: u16, message: String },

    /// Error deserializing response body
    #[error("Failed to deserialize response: {0}")]
    DeserializationError(#[from] serde_json::Error),

    /// No response body available
    #[error("No response body available")]
    NoBodyError,

    /// Other unexpected errors
    #[error("Response error: {0}")]
    Other(String),
}

/// Response status category
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResponseStatusCategory {
    /// 1xx status codes - Informational
    Informational,
    /// 2xx status codes - Success
    Success,
    /// 3xx status codes - Redirection
    Redirection,
    /// 4xx status codes - Client Error
    ClientError,
    /// 5xx status codes - Server Error
    ServerError,
    /// Any other status code
    Unknown,
}

/// HTTP response structure that can be serialized and deserialized
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    /// HTTP status code
    pub status: u16,

    /// HTTP status text (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,

    /// Response headers
    pub headers: HashMap<String, String>,

    /// Response body as Value (can be null if no body)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,

    /// Response body as text (for non-JSON responses)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,

    /// Final URL after redirects
    pub url: String,

    /// Whether the request was redirected
    pub redirected: bool,

    /// Request duration in milliseconds
    pub duration_ms: u64,

    /// Binary body data (base64 encoded when serialized)
    #[serde(skip_serializing_if = "Option::is_none", with = "serde_bytes_base64")]
    pub bytes: Option<Vec<u8>>,
}

// Custom module for base64 encoding byte arrays in serde
mod serde_bytes_base64 {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Option<Vec<u8>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match bytes {
            Some(b) => {
                let base64 = STANDARD.encode(b);
                serializer.serialize_str(&base64)
            }
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<u8>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let base64: Option<String> = Option::deserialize(deserializer)?;
        match base64 {
            Some(s) => STANDARD
                .decode(s.as_bytes())
                .map(Some)
                .map_err(serde::de::Error::custom),
            None => Ok(None),
        }
    }
}

impl Response {
    /// Creates a new response
    pub fn new(
        status: u16,
        headers: HashMap<String, String>,
        url: impl Into<String>,
        duration: Duration,
    ) -> Self {
        Self {
            status,
            status_text: None,
            headers,
            body: None,
            text: None,
            url: url.into(),
            redirected: false,
            duration_ms: duration.as_millis() as u64,
            bytes: None,
        }
    }

    /// Checks if the response status is a success (2xx)
    pub fn is_success(&self) -> bool {
        self.status >= 200 && self.status < 300
    }

    /// Checks if the response status is informational (1xx)
    pub fn is_informational(&self) -> bool {
        self.status >= 100 && self.status < 200
    }

    /// Checks if the response status is a redirection (3xx)
    pub fn is_redirection(&self) -> bool {
        self.status >= 300 && self.status < 400
    }

    /// Checks if the response status is a client error (4xx)
    pub fn is_client_error(&self) -> bool {
        self.status >= 400 && self.status < 500
    }

    /// Checks if the response status is a server error (5xx)
    pub fn is_server_error(&self) -> bool {
        self.status >= 500 && self.status < 600
    }

    /// Gets the status category
    pub fn status_category(&self) -> ResponseStatusCategory {
        match self.status {
            100..=199 => ResponseStatusCategory::Informational,
            200..=299 => ResponseStatusCategory::Success,
            300..=399 => ResponseStatusCategory::Redirection,
            400..=499 => ResponseStatusCategory::ClientError,
            500..=599 => ResponseStatusCategory::ServerError,
            _ => ResponseStatusCategory::Unknown,
        }
    }

    /// Checks if the response has a body
    pub fn has_body(&self) -> bool {
        self.body.is_some() || self.text.is_some() || self.bytes.is_some()
    }

    /// Gets the Content-Type header (if present)
    pub fn content_type(&self) -> Option<&String> {
        self.get_header("content-type")
    }

    /// Gets a header by name (case-insensitive)
    pub fn get_header(&self, name: &str) -> Option<&String> {
        let name_lower = name.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == name_lower)
            .map(|(_, v)| v)
    }

    /// Tries to parse the response body as JSON into the given type
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, ResponseError> {
        match &self.body {
            Some(value) => {
                serde_json::from_value(value.clone()).map_err(ResponseError::DeserializationError)
            }
            None => {
                // Try to parse from text if available
                if let Some(text) = &self.text {
                    serde_json::from_str(text).map_err(ResponseError::DeserializationError)
                } else {
                    Err(ResponseError::NoBodyError)
                }
            }
        }
    }

    /// Gets the response body as text (reference)
    pub fn text(&self) -> Result<&str, ResponseError> {
        if let Some(text) = &self.text {
            Ok(text)
        } else if let Some(Value::String(s)) = &self.body {
            Ok(s)
        } else {
            Err(ResponseError::NoBodyError)
        }
    }

    /// Gets the response body as text (owned string)
    pub fn text_owned(&self) -> Result<String, ResponseError> {
        if let Some(text) = &self.text {
            Ok(text.clone())
        } else if let Some(value) = &self.body {
            match value {
                Value::String(s) => Ok(s.clone()),
                _ => Ok(value.to_string()),
            }
        } else {
            Err(ResponseError::NoBodyError)
        }
    }

    /// Gets the response body as bytes
    pub fn bytes(&self) -> Result<&[u8], ResponseError> {
        if let Some(bytes) = &self.bytes {
            Ok(bytes)
        } else if let Some(text) = &self.text {
            Ok(text.as_bytes())
        } else {
            Err(ResponseError::NoBodyError)
        }
    }

    /// Gets the response body as owned bytes
    pub fn bytes_owned(&self) -> Result<Vec<u8>, ResponseError> {
        if let Some(bytes) = &self.bytes {
            Ok(bytes.clone())
        } else if let Some(text) = &self.text {
            Ok(text.as_bytes().to_vec())
        } else {
            Err(ResponseError::NoBodyError)
        }
    }

    /// Checks if the content type is JSON
    pub fn is_json(&self) -> bool {
        self.content_type()
            .map(|ct| ct.contains("application/json"))
            .unwrap_or(false)
    }

    /// Sets the response body as a JSON value
    pub fn with_json(mut self, json: Value) -> Self {
        self.body = Some(json);
        self
    }

    /// Sets the response body as text
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Sets the response body as binary data
    pub fn with_bytes(mut self, bytes: impl Into<Vec<u8>>) -> Self {
        self.bytes = Some(bytes.into());
        self
    }

    /// Adds a header to the response
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    /// Sets the status text
    pub fn with_status_text(mut self, text: impl Into<String>) -> Self {
        self.status_text = Some(text.into());
        self
    }

    /// Marks the response as redirected
    pub fn redirected(mut self, is_redirected: bool) -> Self {
        self.redirected = is_redirected;
        self
    }

    /// Creates an error from this response if it's not successful
    pub fn error_for_status(self) -> Result<Self, ResponseError> {
        if !self.is_success() {
            let message =
                self.status_text
                    .clone()
                    .unwrap_or_else(|| match self.status_category() {
                        ResponseStatusCategory::ClientError => "Client Error".to_string(),
                        ResponseStatusCategory::ServerError => "Server Error".to_string(),
                        _ => format!("Unexpected Status Code: {}", self.status),
                    });

            Err(ResponseError::HttpError {
                status: self.status,
                message,
            })
        } else {
            Ok(self)
        }
    }

    /// Converts response to a Result based on whether it was successful
    pub fn json_or_error<T: DeserializeOwned>(self) -> Result<T, ResponseError> {
        self.error_for_status()?.json()
    }
}

impl fmt::Display for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Response [{}] {} ({} ms) - {} bytes",
            self.status,
            self.url,
            self.duration_ms,
            self.bytes.as_ref().map_or(0, |b| b.len())
        )
    }
}

/// Trait for asynchronous conversion from a type to Response
#[async_trait]
pub trait AsyncTryFrom<T>: Sized {
    /// The error type returned by the conversion
    type Error;

    /// Tries to convert a value into a Response asynchronously
    async fn async_try_from(value: T) -> Result<Self, Self::Error>;
}
