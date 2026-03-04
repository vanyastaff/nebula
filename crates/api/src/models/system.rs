use serde::Serialize;

/// Single node worker status (for `/api/v1/status`).
#[derive(Debug, Clone, Serialize)]
pub struct WorkerStatus {
    /// Worker id, e.g. `wrk-1`.
    pub id: String,
    /// `"active"` or `"idle"`.
    pub status: String,
    /// Current queue length for this worker.
    pub queue_len: u32,
}

/// Response for `GET /api/v1/status`.
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    /// Node workers (e.g. 4).
    pub workers: Vec<WorkerStatus>,
}
