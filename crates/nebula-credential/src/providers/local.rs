//! Local filesystem storage provider
//!
//! Implements encrypted credential storage on local filesystem with atomic writes.

// Placeholder - will be implemented in Phase 3 (T014-T020)

use std::path::PathBuf;

/// Placeholder local storage configuration
///
/// This is a stub that will be fully implemented in Phase 3 (T014-T020)
#[derive(Clone, Debug)]
pub struct LocalStorageConfig {
    /// Base directory for credential storage
    pub base_path: PathBuf,
}

/// Placeholder local storage provider
///
/// This is a stub that will be fully implemented in Phase 3 (T014-T020)
#[derive(Clone, Debug)]
pub struct LocalStorageProvider;
