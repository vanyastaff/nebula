use serde::Serialize;

/// Response for `GET /api/v1/status`.
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    /// Service health summary.
    pub status: String,
}
