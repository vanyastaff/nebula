//! Credential metadata
//!
//! Provides non-sensitive metadata about credential instances
//! for management and tracking (not security-critical).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::rotation::policy::RotationPolicy;

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

    /// Optional scope for multi-tenant isolation
    pub scope: Option<crate::core::ScopeId>,

    /// Optional rotation policy (for automatic credential rotation)
    pub rotation_policy: Option<RotationPolicy>,

    /// Version number for rotation tracking (incremented on each rotation)
    ///
    /// Used to distinguish between old and new credentials during grace periods.
    /// Starts at 1 for initial credential, incremented with each rotation.
    pub version: u32,

    /// When the credential expires (None if no expiration)
    ///
    /// Used for time-limited credentials like OAuth2 tokens, JWT tokens, temporary passwords.
    /// The ExpiryMonitor uses this field to determine when to trigger rotation based on
    /// BeforeExpiry policy.
    pub expires_at: Option<DateTime<Utc>>,

    /// Time-to-live in seconds (None if unlimited)
    ///
    /// Original TTL of the credential when created. Used in combination with created_at
    /// to calculate expiration time and rotation trigger points.
    ///
    /// For renewable credentials (OAuth2 access tokens), this represents the TTL of
    /// a single token instance, not the overall credential lifetime.
    pub ttl_seconds: Option<u64>,

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
            scope: None,
            rotation_policy: None,
            version: 1, // Initial version
            expires_at: None,
            ttl_seconds: None,
            tags: HashMap::new(),
        }
    }

    /// Increment version number for rotation
    ///
    /// Called when credential is rotated to track the new version.
    ///
    /// # Example
    ///
    /// ```
    /// use nebula_credential::core::CredentialMetadata;
    ///
    /// let mut metadata = CredentialMetadata::new();
    /// assert_eq!(metadata.version, 1);
    ///
    /// metadata.increment_version();
    /// assert_eq!(metadata.version, 2);
    /// ```
    pub fn increment_version(&mut self) {
        self.version = self.version.saturating_add(1);
        self.mark_modified();
    }

    /// Set expiration time and TTL
    ///
    /// Helper to set both expires_at and ttl_seconds based on a TTL duration.
    /// Uses created_at as the base time.
    ///
    /// # Arguments
    ///
    /// * `ttl` - Time-to-live duration
    ///
    /// # Example
    ///
    /// ```
    /// use nebula_credential::core::CredentialMetadata;
    /// use std::time::Duration;
    ///
    /// let mut metadata = CredentialMetadata::new();
    /// metadata.set_expiration(Duration::from_secs(3600)); // 1 hour TTL
    ///
    /// assert!(metadata.expires_at.is_some());
    /// assert_eq!(metadata.ttl_seconds, Some(3600));
    /// ```
    pub fn set_expiration(&mut self, ttl: std::time::Duration) {
        self.ttl_seconds = Some(ttl.as_secs());
        self.expires_at = Some(
            self.created_at + chrono::Duration::from_std(ttl).unwrap_or(chrono::Duration::zero()),
        );
        self.mark_modified();
    }

    /// Check if credential has expired
    ///
    /// Returns `true` if expires_at is set and has passed.
    ///
    /// # Example
    ///
    /// ```
    /// use nebula_credential::core::CredentialMetadata;
    /// use std::time::Duration;
    ///
    /// let mut metadata = CredentialMetadata::new();
    ///
    /// // No expiration set
    /// assert!(!metadata.is_expired());
    ///
    /// // Set expiration in future
    /// metadata.set_expiration(Duration::from_secs(3600));
    /// assert!(!metadata.is_expired());
    /// ```
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| exp <= Utc::now())
            .unwrap_or(false)
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
        use crate::rotation::policy::{PeriodicConfig, RotationPolicy};
        use std::time::Duration;

        let policy = RotationPolicy::Periodic(
            PeriodicConfig::new(
                Duration::from_secs(90 * 24 * 3600), // 90 days
                Duration::from_secs(24 * 3600),      // 1 day
                true,
            )
            .unwrap(),
        );
        let mut metadata = CredentialMetadata::new();
        metadata.rotation_policy = Some(policy);

        assert!(metadata.rotation_policy.is_some());
        match metadata.rotation_policy.unwrap() {
            RotationPolicy::Periodic(config) => {
                assert_eq!(config.interval(), Duration::from_secs(90 * 24 * 3600));
            }
            _ => panic!("Expected Periodic policy"),
        }
    }
}
