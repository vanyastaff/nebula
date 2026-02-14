//! Credential validation types and logic

use crate::core::{CredentialId, CredentialMetadata};
use chrono::{DateTime, Utc};
use std::time::Duration;

/// Result of credential validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationResult {
    /// Credential identifier
    pub credential_id: CredentialId,

    /// Is credential valid?
    pub valid: bool,

    /// Validation details
    pub details: ValidationDetails,
}

/// Detailed validation information
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationDetails {
    /// Credential is valid
    Valid {
        /// Expiration time if set
        expires_at: Option<DateTime<Utc>>,
    },

    /// Credential expired
    Expired {
        /// When it expired
        expired_at: DateTime<Utc>,
        /// Current time
        now: DateTime<Utc>,
    },

    /// Credential not found
    NotFound,

    /// Credential malformed
    Invalid {
        /// Reason for invalidity
        reason: String,
    },
}

impl ValidationResult {
    /// Check if credential is valid
    pub fn is_valid(&self) -> bool {
        self.valid
    }

    /// Check if credential is expired
    pub fn is_expired(&self) -> bool {
        matches!(self.details, ValidationDetails::Expired { .. })
    }

    /// Check if rotation recommended based on age
    ///
    /// Returns true if credential should be rotated because it's approaching
    /// expiration (less than 25% of lifetime remaining).
    ///
    /// # Arguments
    ///
    /// * `max_age` - Maximum age before rotation recommended
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::manager::validation::{ValidationResult, ValidationDetails};
    /// use nebula_credential::core::CredentialId;
    /// use chrono::{Utc, Duration as ChronoDuration};
    /// use std::time::Duration;
    ///
    /// let expires_in_1_hour = Utc::now() + ChronoDuration::hours(1);
    /// let result = ValidationResult {
    ///     credential_id: CredentialId::from("test"),
    ///     valid: true,
    ///     details: ValidationDetails::Valid {
    ///         expires_at: Some(expires_in_1_hour),
    ///     },
    /// };
    ///
    /// // Rotation recommended if less than 25% lifetime remaining
    /// let max_age = Duration::from_secs(3600 * 5); // 5 hours max age
    /// assert!(result.rotation_recommended(max_age)); // Only 1 hour left of 5 hour max
    /// ```
    pub fn rotation_recommended(&self, max_age: Duration) -> bool {
        match &self.details {
            ValidationDetails::Valid {
                expires_at: Some(exp),
            } => {
                let now = Utc::now();
                if *exp <= now {
                    // Already expired
                    return true;
                }

                let remaining = (*exp - now).to_std().unwrap_or(Duration::ZERO);
                let threshold = max_age / 4; // 25% of max age

                remaining < threshold
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    #[test]
    fn test_validation_result_valid() {
        let result = ValidationResult {
            credential_id: CredentialId::new("test-cred").unwrap(),
            valid: true,
            details: ValidationDetails::Valid { expires_at: None },
        };

        assert!(result.valid);
        assert!(matches!(result.details, ValidationDetails::Valid { .. }));
    }

    #[test]
    fn test_validation_result_expired() {
        let expired_at = Utc::now() - ChronoDuration::hours(1);
        let now = Utc::now();

        let result = ValidationResult {
            credential_id: CredentialId::new("expired-cred").unwrap(),
            valid: false,
            details: ValidationDetails::Expired { expired_at, now },
        };

        assert!(!result.valid);
        assert!(matches!(result.details, ValidationDetails::Expired { .. }));
    }

    #[test]
    fn test_validation_result_not_found() {
        let result = ValidationResult {
            credential_id: CredentialId::new("missing-cred").unwrap(),
            valid: false,
            details: ValidationDetails::NotFound,
        };

        assert!(!result.valid);
        assert!(matches!(result.details, ValidationDetails::NotFound));
    }

    #[test]
    fn test_rotation_recommended_no_expiration() {
        let result = ValidationResult {
            credential_id: CredentialId::new("test-cred").unwrap(),
            valid: true,
            details: ValidationDetails::Valid { expires_at: None },
        };

        assert!(!result.rotation_recommended(Duration::from_secs(3600)));
    }

    #[test]
    fn test_rotation_recommended_expires_soon() {
        // Expires in 1 hour
        let expires_at = Utc::now() + ChronoDuration::hours(1);

        let result = ValidationResult {
            credential_id: CredentialId::new("test-cred").unwrap(),
            valid: true,
            details: ValidationDetails::Valid {
                expires_at: Some(expires_at),
            },
        };

        // Max age is 5 hours, so 25% threshold is 1.25 hours
        // Credential has 1 hour remaining, should recommend rotation
        assert!(result.rotation_recommended(Duration::from_secs(3600 * 5)));
    }

    #[test]
    fn test_rotation_not_recommended_plenty_time() {
        // Expires in 10 hours
        let expires_at = Utc::now() + ChronoDuration::hours(10);

        let result = ValidationResult {
            credential_id: CredentialId::new("test-cred").unwrap(),
            valid: true,
            details: ValidationDetails::Valid {
                expires_at: Some(expires_at),
            },
        };

        // Max age is 5 hours, so 25% threshold is 1.25 hours
        // Credential has 10 hours remaining, should NOT recommend rotation
        assert!(!result.rotation_recommended(Duration::from_secs(3600 * 5)));
    }

    #[test]
    fn test_rotation_recommended_already_expired() {
        // Already expired
        let expires_at = Utc::now() - ChronoDuration::hours(1);

        let result = ValidationResult {
            credential_id: CredentialId::new("test-cred").unwrap(),
            valid: false,
            details: ValidationDetails::Valid {
                expires_at: Some(expires_at),
            },
        };

        assert!(result.rotation_recommended(Duration::from_secs(3600)));
    }
}

/// Create ValidationResult from credential metadata
///
/// Checks expiration based on rotation policy and created_at timestamp.
pub fn validate_credential(
    credential_id: &CredentialId,
    metadata: &CredentialMetadata,
) -> ValidationResult {
    let now = Utc::now();

    // Check if credential has rotation policy with expiration
    if let Some(policy) = &metadata.rotation_policy {
        use crate::rotation::policy::RotationPolicy;

        // Calculate expiration time based on policy type
        let expires_at = match policy {
            RotationPolicy::Periodic(config) => {
                // For periodic rotation, credential expires after interval from creation
                let interval_secs = config.interval().as_secs() as i64;
                metadata.created_at + chrono::Duration::seconds(interval_secs)
            }
            RotationPolicy::BeforeExpiry(_config) => {
                // For before-expiry, we need the actual TTL from the credential
                // For now, use a reasonable default or skip expiration check
                // This will be properly implemented in user stories
                return ValidationResult {
                    credential_id: credential_id.clone(),
                    valid: true,
                    details: ValidationDetails::Valid { expires_at: None },
                };
            }
            RotationPolicy::Scheduled(config) => {
                // For scheduled rotation, use the scheduled time
                config.scheduled_at()
            }
            RotationPolicy::Manual(_) => {
                // Manual rotation has no automatic expiration
                return ValidationResult {
                    credential_id: credential_id.clone(),
                    valid: true,
                    details: ValidationDetails::Valid { expires_at: None },
                };
            }
        };

        if now >= expires_at {
            // Credential expired
            return ValidationResult {
                credential_id: credential_id.clone(),
                valid: false,
                details: ValidationDetails::Expired {
                    expired_at: expires_at,
                    now,
                },
            };
        }

        // Valid with expiration
        return ValidationResult {
            credential_id: credential_id.clone(),
            valid: true,
            details: ValidationDetails::Valid {
                expires_at: Some(expires_at),
            },
        };
    }

    // No expiration policy - always valid
    ValidationResult {
        credential_id: credential_id.clone(),
        valid: true,
        details: ValidationDetails::Valid { expires_at: None },
    }
}
