use serde::{Deserialize, Serialize};

/// Run summary DTO for list endpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    /// Run identifier.
    pub id: String,
    /// Workflow identifier.
    pub workflow_id: String,
    /// Run status (queued/running/completed/failed/canceled).
    pub status: String,
    /// Start timestamp (RFC3339).
    pub started_at: Option<String>,
    /// End timestamp (RFC3339).
    pub finished_at: Option<String>,
}
