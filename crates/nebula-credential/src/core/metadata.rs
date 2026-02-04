//! Credential metadata
//!
//! Provides non-sensitive metadata about credential instances
//! for management and tracking (not security-critical).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Credential metadata (non-sensitive)
///
/// Tracks creation time, access patterns, and user-defined tags
/// for credential management and organization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMetadata {
    /// When credential was created
    pub created_at: DateTime<Utc>,

    /// When credential was last accessed (None if never)
    pub last_accessed: Option<DateTime<Utc>>,

    /// When credential was last modified
    pub last_modified: DateTime<Utc>,

    /// Optional rotation policy (for automatic credential rotation)
    pub rotation_policy: Option<RotationPolicy>,

    /// User-defined tags for organization
    pub tags: HashMap<String, String>,
}

impl CredentialMetadata {
    /// Create new metadata with current timestamp
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            created_at: now,
            last_accessed: None,
            last_modified: now,
            rotation_policy: None,
            tags: HashMap::new(),
        }
    }

    /// Update last accessed timestamp
    pub fn mark_accessed(&mut self) {
        self.last_accessed = Some(Utc::now());
    }

    /// Update last modified timestamp
    pub fn mark_modified(&mut self) {
        self.last_modified = Utc::now();
    }
}

impl Default for CredentialMetadata {
    fn default() -> Self {
        Self::new()
    }
}

/// Rotation policy for automatic credential rotation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RotationPolicy {
    /// Rotation interval in days
    pub interval_days: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_new() {
        let metadata = CredentialMetadata::new();
        assert!(metadata.last_accessed.is_none());
        assert_eq!(metadata.created_at, metadata.last_modified);
        assert!(metadata.tags.is_empty());
    }

    #[test]
    fn test_metadata_mark_accessed() {
        let mut metadata = CredentialMetadata::new();
        assert!(metadata.last_accessed.is_none());

        metadata.mark_accessed();
        assert!(metadata.last_accessed.is_some());
    }

    #[test]
    fn test_metadata_mark_modified() {
        let mut metadata = CredentialMetadata::new();
        let original_modified = metadata.last_modified;

        std::thread::sleep(std::time::Duration::from_millis(10));
        metadata.mark_modified();

        assert!(metadata.last_modified > original_modified);
    }

    #[test]
    fn test_metadata_default() {
        let metadata = CredentialMetadata::default();
        assert!(metadata.last_accessed.is_none());
    }

    #[test]
    fn test_metadata_tags() {
        let mut metadata = CredentialMetadata::new();
        metadata
            .tags
            .insert("environment".to_string(), "production".to_string());
        metadata
            .tags
            .insert("service".to_string(), "api-gateway".to_string());

        assert_eq!(metadata.tags.len(), 2);
        assert_eq!(
            metadata.tags.get("environment"),
            Some(&"production".to_string())
        );
    }

    #[test]
    fn test_rotation_policy() {
        let policy = RotationPolicy { interval_days: 90 };
        let mut metadata = CredentialMetadata::new();
        metadata.rotation_policy = Some(policy);

        assert!(metadata.rotation_policy.is_some());
        assert_eq!(metadata.rotation_policy.unwrap().interval_days, 90);
    }
}
