//! A buffered answer: status, headers and a bounded body.

use std::fmt;

use bytes::Bytes;
use http::{HeaderMap, HeaderName, StatusCode};
use nebula_resource::{ErrorKind, call::OperationError};
use serde::de::DeserializeOwned;

/// The provider's answer to a buffered [`Request`](super::Request): a
/// `2xx`, a `3xx` (redirects are answers, never followed) or a status the
/// request accepted, with a body read within the byte budget.
///
/// `Debug` shows the status, header names and body length only.
pub struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
}

impl Response {
    pub(super) fn new(status: StatusCode, headers: HeaderMap, body: Bytes) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }

    /// The status.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.status
    }

    /// The headers.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The body.
    #[must_use]
    pub fn body(&self) -> &Bytes {
        &self.body
    }

    /// Takes the body.
    #[must_use]
    pub fn into_body(self) -> Bytes {
        self.body
    }

    /// Decodes the body as JSON.
    ///
    /// # Errors
    ///
    /// A permanent error when the body is not the expected JSON; the
    /// decoder's message, which may quote the body, is not kept.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, OperationError> {
        serde_json::from_slice(&self.body).map_err(|_| {
            OperationError::new(
                ErrorKind::Permanent,
                "response body is not the expected JSON",
            )
        })
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<&str> = self.headers.keys().map(HeaderName::as_str).collect();
        formatter
            .debug_struct("Response")
            .field("status", &self.status)
            .field("headers", &headers)
            .field("body_len", &self.body.len())
            .finish()
    }
}
