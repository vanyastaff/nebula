use std::time::Duration;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionStatus {
    NotStarted,
    Running {
        attempt: u32,
        elapsed: Duration,
        last_result: Option<String>,
    },
    Paused {
        attempt: u32,
        elapsed: Duration,
        reason: String,
    },
    Completed {
        attempts: u32,
        total_time: Duration,
        result: String,
    },
    Failed {
        attempts: u32,
        total_time: Duration,
        error: String,
    },
    Cancelled {
        attempts: u32,
        total_time: Duration,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionProgress {
    pub task_id: String,
    pub attempt: u32,
    pub elapsed: Duration,
    pub status: ExecutionStatus,
    pub estimated_remaining: Option<Duration>,
}