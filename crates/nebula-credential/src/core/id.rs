//! Credential identifier with validation
//!
//! Provides a validated [`CredentialId`] newtype that prevents
//! path traversal and injection attacks through strict validation rules.

use crate::core::ValidationError;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum length for credential IDs (prevents DoS attacks)
const MAX_ID_LENGTH: usize = 255;

/// Unique credential identifier (validated)
///
/// Only allows alphanumeric characters, hyphens, and underscores to prevent
/// path traversal, filesystem issues, and injection attacks.
///
/// Maximum length is 255 characters to prevent denial-of-service attacks.
///
/// # Examples
///
/// ```
/// use nebula_credential::CredentialId;
///
/// // Valid IDs
/// let id1 = CredentialId::new("github_token").unwrap();
/// let id2 = CredentialId::new("aws-access-key-123").unwrap();
///
/// // Invalid IDs
/// assert!(CredentialId::new("").is_err()); // Empty
/// assert!(CredentialId::new("../etc/passwd").is_err()); // Path traversal
/// assert!(CredentialId::new("token with spaces").is_err()); // Spaces
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CredentialId(String);

impl CredentialId {
    /// Creates a new validated credential ID
    ///
    /// # Arguments
    ///
    /// * `id` - The credential identifier string
    ///
    /// # Returns
    ///
    /// Returns `Ok(CredentialId)` if the ID is valid, or an error describing
    /// why validation failed.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::EmptyCredentialId`] if the ID is empty.
    ///
    /// Returns [`ValidationError::InvalidCredentialId`] if the ID contains
    /// characters other than alphanumeric, hyphens, or underscores.
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::CredentialId;
    ///
    /// let id = CredentialId::new("my_credential_123")?;
    /// assert_eq!(id.as_str(), "my_credential_123");
    /// # Ok::<(), nebula_credential::ValidationError>(())
    /// ```
    pub fn new(id: impl Into<String>) -> Result<Self, ValidationError> {
        let id = id.into();

        if id.is_empty() {
            return Err(ValidationError::EmptyCredentialId);
        }

        // Check length limit
        if id.len() > MAX_ID_LENGTH {
            return Err(ValidationError::InvalidCredentialId {
                id: id.clone(),
                reason: format!("exceeds maximum length of {} characters", MAX_ID_LENGTH),
            });
        }

        // Only allow alphanumeric, hyphens, underscores
        if !id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(ValidationError::InvalidCredentialId {
                id: id.clone(),
                reason:
                    "contains invalid characters (only alphanumeric, hyphens, underscores allowed)"
                        .to_string(),
            });
        }

        Ok(Self(id))
    }

    /// Returns credential ID as string slice
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::CredentialId;
    ///
    /// let id = CredentialId::new("test_id")?;
    /// assert_eq!(id.as_str(), "test_id");
    /// # Ok::<(), nebula_credential::ValidationError>(())
    /// ```
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Converts to owned string
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::CredentialId;
    ///
    /// let id = CredentialId::new("test_id")?;
    /// let string = id.into_string();
    /// assert_eq!(string, "test_id");
    /// # Ok::<(), nebula_credential::ValidationError>(())
    /// ```
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for CredentialId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<CredentialId> for String {
    fn from(id: CredentialId) -> Self {
        id.0
    }
}

impl TryFrom<String> for CredentialId {
    type Error = ValidationError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        CredentialId::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_credential_ids() {
        assert!(CredentialId::new("github_token").is_ok());
        assert!(CredentialId::new("aws-access-key-123").is_ok());
        assert!(CredentialId::new("db_password_prod").is_ok());
        assert!(CredentialId::new("APIKey123").is_ok());
        assert!(CredentialId::new("a").is_ok()); // Single char
        assert!(CredentialId::new("test-123_abc").is_ok()); // Mixed
    }

    #[test]
    fn test_invalid_credential_ids() {
        // Empty
        assert!(matches!(
            CredentialId::new(""),
            Err(ValidationError::EmptyCredentialId)
        ));

        // Too long (exceeds 255 characters)
        let long_id = "a".repeat(256);
        let result = CredentialId::new(long_id);
        assert!(matches!(
            result,
            Err(ValidationError::InvalidCredentialId { .. })
        ));
        if let Err(ValidationError::InvalidCredentialId { reason, .. }) = result {
            assert!(reason.contains("255"));
            assert!(reason.contains("exceeds maximum length"));
        }

        // Exactly 255 characters should be OK
        let max_length_id = "a".repeat(255);
        assert!(CredentialId::new(max_length_id).is_ok());

        // Path traversal
        assert!(matches!(
            CredentialId::new("../etc/passwd"),
            Err(ValidationError::InvalidCredentialId { .. })
        ));

        // Spaces
        assert!(matches!(
            CredentialId::new("token with spaces"),
            Err(ValidationError::InvalidCredentialId { .. })
        ));

        // Special characters
        assert!(matches!(
            CredentialId::new("token@domain.com"),
            Err(ValidationError::InvalidCredentialId { .. })
        ));
        assert!(matches!(
            CredentialId::new("token/path"),
            Err(ValidationError::InvalidCredentialId { .. })
        ));
        assert!(matches!(
            CredentialId::new("token\\path"),
            Err(ValidationError::InvalidCredentialId { .. })
        ));
    }

    #[test]
    fn test_credential_id_as_str() {
        let id = CredentialId::new("test_id").unwrap();
        assert_eq!(id.as_str(), "test_id");
    }

    #[test]
    fn test_credential_id_display() {
        let id = CredentialId::new("display_test").unwrap();
        assert_eq!(format!("{}", id), "display_test");
    }

    #[test]
    fn test_credential_id_clone() {
        let id1 = CredentialId::new("clone_test").unwrap();
        let id2 = id1.clone();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_credential_id_into_string() {
        let id = CredentialId::new("convert_test").unwrap();
        let s: String = id.into();
        assert_eq!(s, "convert_test");
    }

    #[test]
    fn test_credential_id_try_from_string() {
        let s = "test_id".to_string();
        let result: Result<CredentialId, ValidationError> = s.try_into();
        assert!(result.is_ok());

        let invalid = "../invalid".to_string();
        let result: Result<CredentialId, ValidationError> = invalid.try_into();
        assert!(result.is_err());
    }

    #[test]
    fn test_credential_id_serde() {
        let id = CredentialId::new("serde_test").unwrap();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"serde_test\"");

        let deserialized: CredentialId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, deserialized);
    }

    #[test]
    fn test_credential_id_serde_invalid() {
        let json = "\"../invalid\"";
        let result: Result<CredentialId, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }
}
