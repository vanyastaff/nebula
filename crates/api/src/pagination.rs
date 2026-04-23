//! Cursor-based pagination infrastructure.
//!
//! All list endpoints use opaque cursor pagination per spec §05.
//! Cursors are base64-encoded JSON payloads — never parsed by clients.

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

/// Query parameters for cursor-based pagination.
#[derive(Debug, Clone, Deserialize)]
pub struct CursorParams {
    /// Opaque cursor from a previous response's `next_cursor`.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Maximum number of items to return. Capped at `PaginationConfig::max_limit`.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// A paginated response envelope.
#[derive(Debug, Clone, Serialize)]
pub struct PaginatedResponse<T> {
    /// The items on this page.
    pub items: Vec<T>,
    /// Opaque cursor for fetching the next page; absent on the last page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    /// Whether more items exist beyond this page.
    pub has_more: bool,
}

impl<T: Serialize> PaginatedResponse<T> {
    /// Create a response page.
    pub fn new(items: Vec<T>, next_cursor: Option<String>, has_more: bool) -> Self {
        Self {
            items,
            next_cursor,
            has_more,
        }
    }

    /// Convenience: create a final page with no more results.
    pub fn last_page(items: Vec<T>) -> Self {
        Self {
            items,
            next_cursor: None,
            has_more: false,
        }
    }
}

/// Internal cursor payload. Encoded/decoded as base64 JSON.
/// Not exposed to API clients — they see an opaque string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorPayload {
    /// The ID of the last item on the current page.
    pub last_id: String,
    /// Optional secondary sort key for deterministic ordering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sort_key: Option<String>,
}

impl CursorPayload {
    /// Encode this payload into an opaque cursor string.
    pub fn encode(&self) -> Result<String, CursorError> {
        let json = serde_json::to_vec(self).map_err(|e| CursorError::Encode(e.to_string()))?;
        Ok(URL_SAFE_NO_PAD.encode(&json))
    }

    /// Decode an opaque cursor string back into a payload.
    pub fn decode(cursor: &str) -> Result<Self, CursorError> {
        let bytes = URL_SAFE_NO_PAD
            .decode(cursor)
            .map_err(|e| CursorError::Decode(e.to_string()))?;
        serde_json::from_slice(&bytes).map_err(|e| CursorError::Decode(e.to_string()))
    }
}

/// Errors from cursor encoding/decoding.
#[derive(Debug, Clone, thiserror::Error)]
pub enum CursorError {
    /// Failed to serialize cursor payload.
    #[error("failed to encode cursor: {0}")]
    Encode(String),
    /// Failed to deserialize or base64-decode cursor string.
    #[error("invalid cursor: {0}")]
    Decode(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_roundtrip() {
        let payload = CursorPayload {
            last_id: "exe_01J9ABCDEF".to_string(),
            last_sort_key: Some("2026-01-01".to_string()),
        };
        let encoded = payload.encode().expect("encode must succeed");
        let decoded = CursorPayload::decode(&encoded).expect("decode must succeed");
        assert_eq!(decoded.last_id, "exe_01J9ABCDEF");
        assert_eq!(decoded.last_sort_key.as_deref(), Some("2026-01-01"));
    }

    #[test]
    fn cursor_decode_invalid_base64() {
        let err = CursorPayload::decode("not-valid-base64!!!").unwrap_err();
        assert!(matches!(err, CursorError::Decode(_)));
    }

    #[test]
    fn paginated_response_last_page() {
        let resp = PaginatedResponse::last_page(vec![1, 2, 3]);
        assert!(!resp.has_more);
        assert!(resp.next_cursor.is_none());
        assert_eq!(resp.items.len(), 3);
    }
}
