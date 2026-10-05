//! A buffered answer: status, headers and a bounded body.

use std::fmt;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use bytes::Bytes;
use http::{HeaderMap, HeaderName, StatusCode};
use nebula_resource::{ErrorKind, call::OperationError};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de, de::DeserializeOwned};

use super::request::{header_map, header_pairs};

/// The provider's answer to a buffered [`Request`](super::Request): a
/// `2xx`, a `3xx` (redirects are answers, never followed) or a status the
/// request accepted, with a body read within the byte budget.
///
/// `Debug` shows the status, header names and body length only. It
/// serializes as `{status, headers: [[name, value]], body: base64}`, the
/// form an execution journal records and replays; a header value that is
/// not UTF-8 does not serialize, and the answer is then recorded
/// digest-only.
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

/// The wire form of a [`Response`].
#[derive(Serialize, Deserialize)]
struct ResponseWire {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Serialize for Response {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ResponseWire {
            status: self.status.as_u16(),
            headers: header_pairs(&self.headers)?,
            body: STANDARD.encode(&self.body),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let wire = ResponseWire::deserialize(deserializer)?;
        let status = StatusCode::from_u16(wire.status)
            .map_err(|_| de::Error::custom("invalid response status"))?;
        let body = STANDARD
            .decode(wire.body)
            .map_err(|_| de::Error::custom("response body is not base64"))?;
        Ok(Self::new(
            status,
            header_map(wire.headers)?,
            Bytes::from(body),
        ))
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
