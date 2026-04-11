//! Webhook payload types

use std::collections::HashMap;

use bytes::Bytes;
use chrono::{DateTime, Utc};

/// Raw HTTP request data from an incoming webhook
///
/// This struct contains the unprocessed request data. It's the
/// responsibility of the `WebhookAction` implementation to parse
/// and validate the payload.
#[derive(Debug, Clone)]
pub struct WebhookPayload {
    /// Full request path
    pub path: String,

    /// HTTP method (usually POST)
    pub method: String,

    /// Request headers as key-value pairs
    ///
    /// Header names are lowercase for case-insensitive matching.
    pub headers: HashMap<String, String>,

    /// Request body as raw bytes
    pub body: Bytes,

    /// Query parameters from the URL
    pub query: HashMap<String, String>,

    /// Timestamp when the request was received
    pub received_at: DateTime<Utc>,
}

impl WebhookPayload {
    /// Create a new webhook payload
    pub fn new(
        path: String,
        method: String,
        headers: HashMap<String, String>,
        body: Bytes,
    ) -> Self {
        Self {
            path,
            method,
            headers,
            body,
            query: HashMap::new(),
            received_at: Utc::now(),
        }
    }

    /// Add query parameters
    pub fn with_query(mut self, query: HashMap<String, String>) -> Self {
        self.query = query;
        self
    }

    /// Get a header value (case-insensitive)
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(String::as_str)
    }

    /// Get a query parameter
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(String::as_str)
    }

    /// Get the body as a UTF-8 string
    ///
    /// Returns `None` if the body is not valid UTF-8.
    pub fn body_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.body).ok()
    }

    /// Parse the body as JSON
    pub fn body_json<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_slice(&self.body)
    }

    /// Get the content type header
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    /// Get the user agent header
    pub fn user_agent(&self) -> Option<&str> {
        self.header("user-agent")
    }

    /// Get the body size in bytes
    pub fn body_size(&self) -> usize {
        self.body.len()
    }

    /// Check if the body is empty
    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payload_creation() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        headers.insert("X-Custom-Header".to_string(), "value".to_string());

        let body = Bytes::from(r#"{"key":"value"}"#);
        let payload = WebhookPayload::new(
            "/webhooks/test/123".to_string(),
            "POST".to_string(),
            headers,
            body,
        );

        assert_eq!(payload.path, "/webhooks/test/123");
        assert_eq!(payload.method, "POST");
        assert_eq!(payload.body_size(), 15);
        assert!(!payload.is_empty());
    }

    #[test]
    fn test_header_case_insensitive() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let payload = WebhookPayload::new(
            "/test".to_string(),
            "POST".to_string(),
            headers,
            Bytes::new(),
        );

        assert_eq!(payload.header("Content-Type"), Some("application/json"));
        assert_eq!(payload.header("CONTENT-TYPE"), Some("application/json"));
        assert_eq!(payload.header("content-type"), Some("application/json"));
    }

    #[test]
    fn test_body_str() {
        let payload = WebhookPayload::new(
            "/test".to_string(),
            "POST".to_string(),
            HashMap::new(),
            Bytes::from("Hello, World!"),
        );

        assert_eq!(payload.body_str(), Some("Hello, World!"));
    }

    #[test]
    fn test_body_json() {
        use serde_json::Value;

        let payload = WebhookPayload::new(
            "/test".to_string(),
            "POST".to_string(),
            HashMap::new(),
            Bytes::from(r#"{"key":"value"}"#),
        );

        let json: Value = payload.body_json().unwrap();
        assert_eq!(json["key"], "value");
    }

    #[test]
    fn test_query_params() {
        let mut query = HashMap::new();
        query.insert("page".to_string(), "1".to_string());
        query.insert("limit".to_string(), "10".to_string());

        let payload = WebhookPayload::new(
            "/test".to_string(),
            "POST".to_string(),
            HashMap::new(),
            Bytes::new(),
        )
        .with_query(query);

        assert_eq!(payload.query_param("page"), Some("1"));
        assert_eq!(payload.query_param("limit"), Some("10"));
        assert_eq!(payload.query_param("offset"), None);
    }

    #[test]
    fn test_content_type() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());

        let payload = WebhookPayload::new(
            "/test".to_string(),
            "POST".to_string(),
            headers,
            Bytes::new(),
        );

        assert_eq!(payload.content_type(), Some("application/json"));
    }
}
