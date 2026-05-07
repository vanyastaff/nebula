//! Health check models

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Health check response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    /// Service status
    pub status: String,

    /// Version
    pub version: String,

    /// Timestamp
    pub timestamp: i64,
}

/// Readiness check response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReadinessResponse {
    /// Ready status
    pub ready: bool,

    /// Dependencies status
    pub dependencies: DependenciesStatus,
}

/// Dependencies status
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DependenciesStatus {
    /// Database status
    pub database: bool,

    /// Cache status (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache: Option<bool>,
}

/// `GET /api/v1/version` response — application name + version.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VersionInfo {
    /// Application semver version (from `CARGO_PKG_VERSION`).
    pub version: String,
    /// Application name (always `"nebula"`).
    pub name: String,
}
