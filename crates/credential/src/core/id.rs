//! Credential identifier with validation
//!
//! Provides a validated [`CredentialId`] newtype that prevents
//! path traversal and injection attacks through strict validation rules.

use crate::core::ValidationError;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum length for credential IDs (prevents DoS attacks)
const MAX_ID_LENGTH: usize = 255;

/// Maximum length for scope IDs (prevents DoS attacks)
const MAX_SCOPE_LENGTH: usize = 512;

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

/// Hierarchical scope identifier for multi-tenant credential isolation
///
/// Format: `"level:value/level:value/level:value"`
///
/// Examples:
/// - `"org:acme"` - Organization scope
/// - `"org:acme/team:eng"` - Team within organization
/// - `"org:acme/team:eng/service:api"` - Service within team
///
/// Validation rules:
/// - Alphanumeric, hyphens, underscores, colons, and forward slashes allowed
/// - Cannot start or end with forward slash
/// - Maximum length 512 characters
///
/// # Examples
///
/// ```
/// use nebula_credential::core::ScopeId;
///
/// // Valid scopes
/// let org = ScopeId::new("org:acme").unwrap();
/// let team = ScopeId::new("org:acme/team:eng").unwrap();
/// let service = ScopeId::new("org:acme/team:eng/service:api").unwrap();
///
/// // Invalid scopes
/// assert!(ScopeId::new("").is_err()); // Empty
/// assert!(ScopeId::new("/starts-with-slash").is_err());
/// assert!(ScopeId::new("ends-with-slash/").is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ScopeId(String);

impl ScopeId {
    /// Creates a new validated scope ID
    ///
    /// # Arguments
    ///
    /// * `id` - The scope identifier string (format: "level:value/level:value")
    ///
    /// # Returns
    ///
    /// Returns `Ok(ScopeId)` if the ID is valid, or an error describing
    /// why validation failed.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::EmptyCredentialId`] if the scope is empty.
    /// Returns [`ValidationError::InvalidCredentialId`] if the scope has invalid format.
    pub fn new(id: impl Into<String>) -> Result<Self, ValidationError> {
        let id = id.into();

        if id.is_empty() {
            return Err(ValidationError::EmptyCredentialId);
        }

        // Check length limit
        if id.len() > MAX_SCOPE_LENGTH {
            return Err(ValidationError::InvalidCredentialId {
                id: id.clone(),
                reason: format!("exceeds maximum length of {} characters", MAX_SCOPE_LENGTH),
            });
        }

        // Cannot start or end with slash
        if id.starts_with('/') || id.ends_with('/') {
            return Err(ValidationError::InvalidCredentialId {
                id: id.clone(),
                reason: "cannot start or end with forward slash".to_string(),
            });
        }

        // Only allow alphanumeric, hyphens, underscores, colons, and forward slashes
        if !id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ':' || c == '/')
        {
            return Err(ValidationError::InvalidCredentialId {
                id: id.clone(),
                reason: "contains invalid characters (only alphanumeric, hyphens, underscores, colons, slashes allowed)".to_string(),
            });
        }

        Ok(Self(id))
    }

    /// Returns scope ID as string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Converts to owned string
    pub fn into_string(self) -> String {
        self.0
    }

    /// Check if this scope matches another scope exactly
    pub fn matches_exact(&self, other: &ScopeId) -> bool {
        self.0 == other.0
    }

    /// Check if this scope is a parent of (or equal to) another scope
    ///
    /// # Examples
    ///
    /// ```
    /// use nebula_credential::core::ScopeId;
    ///
    /// let parent = ScopeId::new("org:acme").unwrap();
    /// let child = ScopeId::new("org:acme/team:eng").unwrap();
    /// let grandchild = ScopeId::new("org:acme/team:eng/service:api").unwrap();
    ///
    /// assert!(parent.matches_prefix(&child));
    /// assert!(parent.matches_prefix(&grandchild));
    /// assert!(child.matches_prefix(&grandchild));
    /// assert!(!child.matches_prefix(&parent));
    /// ```
    pub fn matches_prefix(&self, other: &ScopeId) -> bool {
        other.0.starts_with(&self.0)
            && (other.0.len() == self.0.len() || other.0.as_bytes()[self.0.len()] == b'/')
    }
}

impl fmt::Display for ScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<ScopeId> for String {
    fn from(id: ScopeId) -> Self {
        id.0
    }
}

impl TryFrom<String> for ScopeId {
    type Error = ValidationError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        ScopeId::new(s)
    }
}

#[cfg(test)]
mod scope_id_tests {
    use super::*;

    #[test]
    fn test_valid_scope_ids() {
        assert!(ScopeId::new("org:acme").is_ok());
        assert!(ScopeId::new("org:acme/team:eng").is_ok());
        assert!(ScopeId::new("org:acme/team:eng/service:api").is_ok());
        assert!(ScopeId::new("a").is_ok()); // Single char
        assert!(ScopeId::new("test-123_abc:xyz").is_ok()); // Mixed
    }

    #[test]
    fn test_invalid_scope_ids() {
        // Empty
        assert!(matches!(
            ScopeId::new(""),
            Err(ValidationError::EmptyCredentialId)
        ));

        // Too long
        let long_id = "a".repeat(513);
        assert!(matches!(
            ScopeId::new(long_id),
            Err(ValidationError::InvalidCredentialId { .. })
        ));

        // Starts with slash
        assert!(matches!(
            ScopeId::new("/org:acme"),
            Err(ValidationError::InvalidCredentialId { .. })
        ));

        // Ends with slash
        assert!(matches!(
            ScopeId::new("org:acme/"),
            Err(ValidationError::InvalidCredentialId { .. })
        ));

        // Invalid characters
        assert!(matches!(
            ScopeId::new("org@acme"),
            Err(ValidationError::InvalidCredentialId { .. })
        ));
    }

    #[test]
    fn test_scope_matches_exact() {
        let scope1 = ScopeId::new("org:acme").unwrap();
        let scope2 = ScopeId::new("org:acme").unwrap();
        let scope3 = ScopeId::new("org:other").unwrap();

        assert!(scope1.matches_exact(&scope2));
        assert!(!scope1.matches_exact(&scope3));
    }

    #[test]
    fn test_scope_matches_prefix() {
        let parent = ScopeId::new("org:acme").unwrap();
        let child = ScopeId::new("org:acme/team:eng").unwrap();
        let grandchild = ScopeId::new("org:acme/team:eng/service:api").unwrap();
        let other = ScopeId::new("org:other").unwrap();

        // Parent matches children
        assert!(parent.matches_prefix(&child));
        assert!(parent.matches_prefix(&grandchild));
        assert!(parent.matches_prefix(&parent)); // Self-match

        // Child matches grandchild
        assert!(child.matches_prefix(&grandchild));
        assert!(child.matches_prefix(&child)); // Self-match

        // Does not match unrelated
        assert!(!parent.matches_prefix(&other));
        assert!(!child.matches_prefix(&parent)); // Child does not match parent
    }

    #[test]
    fn test_scope_id_display() {
        let scope = ScopeId::new("org:acme/team:eng").unwrap();
        assert_eq!(format!("{}", scope), "org:acme/team:eng");
    }

    #[test]
    fn test_scope_id_serde() {
        let scope = ScopeId::new("org:acme/team:eng").unwrap();
        let json = serde_json::to_string(&scope).unwrap();
        assert_eq!(json, "\"org:acme/team:eng\"");

        let deserialized: ScopeId = serde_json::from_str(&json).unwrap();
        assert_eq!(scope, deserialized);
    }
}
