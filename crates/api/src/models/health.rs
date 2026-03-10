//! Health check models

use serde::{Deserialize, Serialize};

/// Health check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Service status
    pub status: String,

    /// Version
    pub version: String,

    /// Timestamp
    pub timestamp: i64,
}

/// Readiness check response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessResponse {
    /// Ready status
    pub ready: bool,

    /// Dependencies status
    pub dependencies: DependenciesStatus,
}

/// Dependencies status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependenciesStatus {
    /// Database status
    pub database: bool,

    /// Cache status (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<bool>,
}
