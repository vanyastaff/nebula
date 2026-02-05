//! Testable credential trait
//!
//! Extends base Credential trait with testing capabilities.

use async_trait::async_trait;
use std::time::Duration;

use super::credential::Credential;
use crate::rotation::{RotationResult, ValidationOutcome};

/// Trait for credentials that can test themselves
///
/// This trait extends the base Credential trait with the ability
/// to validate that the credential actually works by performing a real operation.
///
/// Follows the n8n pattern: test actual functionality, not just format validation.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::traits::{Credential, TestableCredential};
///
/// pub struct MySqlCredential {
///     // Credential state after initialization
///     pub username: String,
///     pub password: SecretString,
///     pub host: String,
///     pub port: u16,
/// }
///
/// #[async_trait]
/// impl TestableCredential for MySqlCredential {
///     async fn test(&self) -> RotationResult<ValidationOutcome> {
///         let start = Instant::now();
///
///         // Use mysql_async client library to test connection
///         let opts = OptsBuilder::new()
///             .user(Some(&self.username))
///             .pass(Some(self.password.expose_secret()))
///             .ip_or_hostname(&self.host)
///             .tcp_port(self.port);
///
///         let mut conn = Conn::new(opts).await?;
///         conn.query_drop("SELECT 1").await?;
///
///         Ok(ValidationOutcome::success(
///             "MySQL connection successful",
///             "SELECT 1",
///             start.elapsed(),
///         ))
///     }
/// }
/// ```
#[async_trait]
pub trait TestableCredential: Credential {
    /// Test the credential by performing actual operation
    ///
    /// Each credential type implements this using their client library:
    /// - **MySQL/PostgreSQL**: `SELECT 1` query
    /// - **OAuth2**: Call userinfo endpoint with token
    /// - **API Key**: Call account/status endpoint
    /// - **Certificate**: Perform TLS handshake
    ///
    /// The credential should be in a valid state (initialized) before testing.
    ///
    /// Returns `ValidationOutcome` with success/failure details.
    async fn test(&self) -> RotationResult<ValidationOutcome>;

    /// Get test timeout (default: 30 seconds)
    fn test_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}
