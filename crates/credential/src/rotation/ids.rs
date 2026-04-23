//! Domain identifiers for rotation operations.
//!
//! These types are **contract identifiers** that stay in `nebula-credential`
//! because downstream crates (e.g. `nebula-storage`) reference them without
//! depending on `nebula-engine`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique identifier for a rotation transaction
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct RotationId(Uuid);

impl RotationId {
    /// Generate a new rotation ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Get the inner UUID
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }
}

impl AsRef<Uuid> for RotationId {
    fn as_ref(&self) -> &Uuid {
        &self.0
    }
}

impl Default for RotationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RotationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for RotationId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

/// Unique identifier for a backup
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct BackupId(Uuid);

impl BackupId {
    /// Generate a new backup ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Get the inner UUID
    pub fn as_uuid(&self) -> &Uuid {
        &self.0
    }

    /// Convert to string representation
    pub fn as_str(&self) -> String {
        self.0.to_string()
    }
}

impl Default for BackupId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for BackupId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Uuid> for BackupId {
    fn from(uuid: Uuid) -> Self {
        Self(uuid)
    }
}
