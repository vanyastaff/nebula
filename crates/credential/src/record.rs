//! Credential record — runtime operational state.
//!
//! Provides non-sensitive operational state about credential instances
//! for management and tracking (not security-critical).
//!
//! # `CredentialRecord` vs `nebula_storage::rows::CredentialRow`
//!
//! This type holds runtime operational state about a credential instance —
//! created_at, last_accessed, rotation counter, etc. It is the **domain**
//! representation.
//!
//! `nebula_storage::rows::CredentialRow` is the **persisted row** representation.
//! They are intentionally distinct types owned by different crates (domain
//! vs persistence) and may diverge as storage schemas evolve.
//!
//! See spec 2026-04-17-rename-credential-metadata-description.md and
//! follow-up task "Evaluate CredentialRecord placement".

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(feature = "rotation")]
use crate::rotation::policy::RotationPolicy;

/// Credential record — runtime operational state (non-sensitive).
///
/// Tracks creation time, access patterns, and user-defined tags
/// for credential management and organization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRecord {
    /// When credential was created
    pub created_at: DateTime<Utc>,

    /// When credential was last accessed (None if never)
    pub last_accessed: Option<DateTime<Utc>>,

    /// When credential was last modified
    pub last_modified: DateTime<Utc>,

    /// Optional scope for multi-tenant isolation.
    /// Uses `ScopeLevel` from nebula-core for platform consistency.
    pub owner_scope: Option<nebula_core::ScopeLevel>,

    /// Optional rotation policy (for automatic credential rotation)
    #[cfg(feature = "rotation")]
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

impl CredentialRecord {
    /// Create new record with current timestamp
    pub fn new() -> Self {
        let now = Utc::now();
        Self {
            created_at: now,
            last_accessed: None,
            last_modified: now,
            owner_scope: None,
            #[cfg(feature = "rotation")]
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
    /// use nebula_credential::CredentialRecord;
    ///
    /// let mut record = CredentialRecord::new();
    /// assert_eq!(record.version, 1);
    ///
    /// record.increment_version();
    /// assert_eq!(record.version, 2);
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
    /// use std::time::Duration;
    ///
    /// use nebula_credential::CredentialRecord;
    ///
    /// let mut record = CredentialRecord::new();
    /// record.set_expiration(Duration::from_secs(3600)); // 1 hour TTL
    ///
    /// assert!(record.expires_at.is_some());
    /// assert_eq!(record.ttl_seconds, Some(3600));
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
    /// use std::time::Duration;
    ///
    /// use nebula_credential::CredentialRecord;
    ///
    /// let mut record = CredentialRecord::new();
    ///
    /// // No expiration set
    /// assert!(!record.is_expired());
    ///
    /// // Set expiration in future
    /// record.set_expiration(Duration::from_secs(3600));
    /// assert!(!record.is_expired());
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

impl Default for CredentialRecord {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_new() {
        let record = CredentialRecord::new();
        assert!(record.last_accessed.is_none());
        assert_eq!(record.created_at, record.last_modified);
        assert!(record.tags.is_empty());
    }

    #[test]
    fn test_record_mark_accessed() {
        let mut record = CredentialRecord::new();
        assert!(record.last_accessed.is_none());

        record.mark_accessed();
        assert!(record.last_accessed.is_some());
    }

    #[test]
    fn test_record_mark_modified() {
        let mut record = CredentialRecord::new();
        let original_modified = record.last_modified;

        std::thread::sleep(std::time::Duration::from_millis(10));
        record.mark_modified();

        assert!(record.last_modified > original_modified);
    }

    #[test]
    fn test_record_default() {
        let record = CredentialRecord::default();
        assert!(record.last_accessed.is_none());
    }

    #[test]
    fn test_record_tags() {
        let mut record = CredentialRecord::new();
        record
            .tags
            .insert("environment".to_string(), "production".to_string());
        record
            .tags
            .insert("service".to_string(), "api-gateway".to_string());

        assert_eq!(record.tags.len(), 2);
        assert_eq!(
            record.tags.get("environment"),
            Some(&"production".to_string())
        );
    }

    #[test]
    #[cfg(feature = "rotation")]
    fn test_rotation_policy() {
        use std::time::Duration;

        use crate::rotation::policy::{PeriodicConfig, RotationPolicy};

        let policy = RotationPolicy::Periodic(
            PeriodicConfig::new(
                Duration::from_secs(90 * 24 * 3600), // 90 days
                Duration::from_secs(24 * 3600),      // 1 day
                true,
            )
            .unwrap(),
        );
        let mut record = CredentialRecord::new();
        record.rotation_policy = Some(policy);

        assert!(record.rotation_policy.is_some());
        match record.rotation_policy.unwrap() {
            RotationPolicy::Periodic(config) => {
                assert_eq!(config.interval(), Duration::from_secs(90 * 24 * 3600));
            },
            _ => panic!("Expected Periodic policy"),
        }
    }
}
