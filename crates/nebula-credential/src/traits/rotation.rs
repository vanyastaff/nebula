//! Rotatable credential trait
//!
//! Extends TestableCredential with rotation capabilities.

use async_trait::async_trait;

use super::testable::TestableCredential;
use crate::rotation::RotationResult;

/// Trait for credentials that support rotation
///
/// This trait extends TestableCredential, which in turn should extend Credential.
/// The hierarchy is: Credential → TestableCredential → RotatableCredential
///
/// Credentials implementing this trait can generate new versions and cleanup old ones.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::traits::RotatableCredential;
///
/// #[async_trait]
/// impl RotatableCredential for MySqlCredential {
///     async fn rotate(&self) -> RotationResult<Self> {
///         // Generate new password
///         let new_password = generate_secure_password(32);
///
///         // Create new credential with same permissions
///         let new_cred = MySqlCredential {
///             username: format!("{}_v{}", self.username, version),
///             password: SecretString::new(new_password),
///             host: self.host.clone(),
///             port: self.port,
///         };
///
///         // Use admin connection to create new user with same grants
///         let admin = self.get_admin_connection().await?;
///         admin.create_user_with_grants(&new_cred, &self).await?;
///
///         Ok(new_cred)
///     }
///
///     async fn cleanup_old(&self) -> RotationResult<()> {
///         let admin = self.get_admin_connection().await?;
///         admin.drop_user(&self.username).await?;
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait RotatableCredential: TestableCredential {
    /// Generate a new version of this credential
    ///
    /// The new credential should have:
    /// - Different secrets (password, token, key)
    /// - Same permissions and access levels
    /// - Same connection details (host, port, database)
    ///
    /// For databases, this typically means creating a new user with identical grants.
    async fn rotate(&self) -> RotationResult<Self>
    where
        Self: Sized;

    /// Clean up old credential after rotation completes
    ///
    /// Optional: implement if cleanup is needed (e.g., delete old database user).
    /// Called after grace period expires.
    async fn cleanup_old(&self) -> RotationResult<()> {
        Ok(())
    }
}
