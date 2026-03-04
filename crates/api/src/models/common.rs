use serde::{Deserialize, Serialize};

/// Standard API error shape used on non-2xx responses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiErrorResponse {
    /// Stable machine-readable error code.
    pub error: String,
    /// Human-readable message.
    pub message: String,
}

impl ApiErrorResponse {
    /// Build a new error envelope.
    pub fn new(error: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
        }
    }
}

/// Standard pagination query parameters.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginationQuery {
    /// Number of items to skip.
    pub offset: Option<usize>,
    /// Number of items to return.
    pub limit: Option<usize>,
}

/// Standard paginated response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedResponse<T> {
    /// Page items.
    pub items: Vec<T>,
    /// Request offset used for this page.
    pub offset: usize,
    /// Request limit used for this page.
    pub limit: usize,
}
