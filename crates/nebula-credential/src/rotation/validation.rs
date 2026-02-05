//! Credential Validation Framework
//!
//! Validates new credentials before committing rotation.
//!
//! # Design Philosophy
//!
//! Validation is delegated to credential implementations through traits:
//! - `TestableCredential` - credentials that can test themselves
//! - `RotatableCredential` - credentials that support rotation
//!
//! This allows each credential type (MySQL, PostgreSQL, OAuth2, etc.) to implement
//! their own validation logic using their specific client libraries.
//!
//! # Example Implementations
//!
//! ```rust,ignore
//! // MySQL credential with rotation support
//! pub struct MySqlCredential {
//!     pub username: String,
//!     pub password: SecretString,
//!     pub host: String,
//!     pub port: u16,
//!     pub database: String,
//! }
//!
//! #[async_trait]
//! impl TestableCredential for MySqlCredential {
//!     async fn test(&self) -> RotationResult<ValidationOutcome> {
//!         let start = Instant::now();
//!
//!         // Use mysql_async client library to test connection
//!         let opts = OptsBuilder::new()
//!             .user(Some(&self.username))
//!             .pass(Some(self.password.expose_secret()))
//!             .ip_or_hostname(&self.host)
//!             .tcp_port(self.port)
//!             .db_name(Some(&self.database));
//!
//!         let mut conn = Conn::new(opts).await
//!             .map_err(|e| RotationError::ValidationFailed {
//!                 credential_id: self.id.clone(),
//!                 reason: format!("Connection failed: {}", e),
//!             })?;
//!
//!         // Test with SELECT 1 query
//!         conn.query_drop("SELECT 1").await
//!             .map_err(|e| RotationError::ValidationFailed {
//!                 credential_id: self.id.clone(),
//!                 reason: format!("Query failed: {}", e),
//!             })?;
//!
//!         Ok(ValidationOutcome::success(
//!             "MySQL connection successful",
//!             "SELECT 1",
//!             start.elapsed(),
//!         ))
//!     }
//! }
//!
//! #[async_trait]
//! impl RotatableCredential for MySqlCredential {
//!     async fn rotate(&self) -> RotationResult<Self> {
//!         // Generate new password
//!         let new_password = generate_secure_password(32);
//!
//!         // Create new credential with same permissions
//!         let new_cred = MySqlCredential {
//!             username: format!("{}_v{}", self.username, version),
//!             password: SecretString::new(new_password),
//!             host: self.host.clone(),
//!             port: self.port,
//!             database: self.database.clone(),
//!         };
//!
//!         // Use admin connection to create new user with same grants
//!         let admin = self.get_admin_connection().await?;
//!         admin.create_user_with_grants(&new_cred, &self).await?;
//!
//!         Ok(new_cred)
//!     }
//!
//!     async fn cleanup_old(&self) -> RotationResult<()> {
//!         let admin = self.get_admin_connection().await?;
//!         admin.drop_user(&self.username).await?;
//!         Ok(())
//!     }
//! }
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::core::{CredentialId, CredentialMetadata};

use super::error::{RotationError, RotationResult};
use tokio::time::timeout;

/// Context for credential validation
#[derive(Debug, Clone)]
pub struct ValidationContext {
    /// Credential being validated
    pub credential_id: CredentialId,

    /// Credential metadata
    pub metadata: CredentialMetadata,

    /// Timeout for validation
    pub timeout: Duration,

    /// Whether this is a retry attempt
    pub is_retry: bool,

    /// Retry attempt number (if retry)
    pub retry_attempt: u32,
}

impl ValidationContext {
    /// Create a new validation context
    pub fn new(credential_id: CredentialId, metadata: CredentialMetadata) -> Self {
        Self {
            credential_id,
            metadata,
            timeout: Duration::from_secs(30), // Default 30s timeout
            is_retry: false,
            retry_attempt: 0,
        }
    }

    /// Set custom timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Mark as retry attempt
    pub fn with_retry(mut self, attempt: u32) -> Self {
        self.is_retry = true;
        self.retry_attempt = attempt;
        self
    }

    /// Validate a credential with timeout enforcement
    ///
    /// Wraps the credential's `test()` method with a timeout to prevent
    /// validation from hanging indefinitely.
    ///
    /// # Arguments
    ///
    /// * `credential` - The credential to validate
    ///
    /// # Returns
    ///
    /// * `Ok(ValidationOutcome)` - Validation completed within timeout
    /// * `Err(RotationError::Timeout)` - Validation exceeded timeout
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let context = ValidationContext::new(cred_id, metadata)
    ///     .with_timeout(Duration::from_secs(10));
    ///
    /// let outcome = context.validate(&mysql_credential).await?;
    /// ```
    pub async fn validate<T: TestableCredential>(
        &self,
        credential: &T,
    ) -> RotationResult<ValidationOutcome> {
        let timeout_duration = self.timeout;

        tracing::debug!(
            credential_id = %self.credential_id,
            timeout_secs = timeout_duration.as_secs(),
            is_retry = self.is_retry,
            retry_attempt = self.retry_attempt,
            "Starting credential validation with timeout"
        );

        match timeout(timeout_duration, credential.test()).await {
            Ok(result) => result,
            Err(_) => {
                tracing::error!(
                    credential_id = %self.credential_id,
                    timeout_secs = timeout_duration.as_secs(),
                    "Credential validation timed out"
                );
                Err(RotationError::Timeout {
                    operation: "credential_validation".to_string(),
                    timeout_secs: timeout_duration.as_secs(),
                })
            }
        }
    }
}

/// Trait for credentials that can test themselves
///
/// Credential implementations use their specific client libraries to validate connectivity.
///
/// This follows the n8n pattern: test actual functionality, not just format validation.
#[async_trait]
pub trait TestableCredential: Send + Sync {
    /// Test the credential by performing actual operation
    ///
    /// Each credential type implements this using their client library:
    /// - **MySQL/PostgreSQL**: `SELECT 1` query
    /// - **OAuth2**: Call userinfo endpoint with token
    /// - **API Key**: Call account/status endpoint
    /// - **Certificate**: Perform TLS handshake
    ///
    /// Returns `ValidationOutcome` with success/failure details.
    async fn test(&self) -> RotationResult<ValidationOutcome>;

    /// Get test timeout (default: 30 seconds)
    fn test_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Trait for credentials that support rotation
///
/// Credentials implementing this trait can generate new versions and cleanup old ones.
///
/// # Rotation Policy
///
/// The rotation policy is stored in `CredentialMetadata.rotation_policy` and can be
/// configured per-credential instance. This allows different instances of the same
/// credential type to have different rotation schedules.
///
/// # Example
///
/// ```rust,ignore
/// // Set rotation policy in metadata
/// let metadata = CredentialMetadata {
///     rotation_policy: Some(RotationPolicy::Periodic(PeriodicConfig {
///         interval: Duration::from_secs(90 * 24 * 3600), // 90 days
///         grace_period: Duration::from_secs(24 * 3600),  // 1 day
///         enable_jitter: true,
///     })),
///     ..Default::default()
/// };
///
/// // Or use BeforeExpiry for OAuth tokens
/// let metadata = CredentialMetadata {
///     rotation_policy: Some(RotationPolicy::BeforeExpiry(BeforeExpiryConfig {
///         threshold_percent: 80.0,
///         min_ttl_seconds: 3600,
///     })),
///     ..Default::default()
/// };
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

/// Trait for credentials with token refresh capabilities (OAuth2, JWT)
///
/// Credentials implementing this trait can refresh their tokens before expiration.
/// This is specifically for OAuth2 access tokens, JWT tokens, and similar short-lived credentials.
#[async_trait]
pub trait TokenRefreshValidator: TestableCredential {
    /// Refresh the token using refresh_token or client credentials
    ///
    /// Returns a new credential instance with refreshed access token.
    /// The refresh mechanism depends on the token type:
    /// - **OAuth2 Authorization Code**: Use refresh_token to get new access_token
    /// - **OAuth2 Client Credentials**: Request new token with client_id/client_secret
    /// - **JWT**: Re-authenticate to get new JWT
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// impl TokenRefreshValidator for OAuth2Credential {
    ///     async fn refresh_token(&self) -> RotationResult<Self> {
    ///         // Use refresh_token to get new access_token
    ///         let token_response = oauth2_client
    ///             .exchange_refresh_token(&self.refresh_token)
    ///             .request_async()
    ///             .await?;
    ///
    ///         Ok(OAuth2Credential {
    ///             access_token: token_response.access_token().secret().clone(),
    ///             refresh_token: token_response.refresh_token()
    ///                 .map(|t| t.secret().clone())
    ///                 .unwrap_or(self.refresh_token.clone()),
    ///             expires_at: Some(Utc::now() + token_response.expires_in().unwrap()),
    ///             ..self.clone()
    ///         })
    ///     }
    /// }
    /// ```
    async fn refresh_token(&self) -> RotationResult<Self>
    where
        Self: Sized;

    /// Get the token expiration time
    ///
    /// Returns `None` if token doesn't expire or expiration is unknown.
    fn get_expiration(&self) -> Option<chrono::DateTime<chrono::Utc>>;

    /// Get remaining token lifetime
    ///
    /// Returns `None` if token doesn't expire.
    fn time_until_expiry(&self) -> Option<chrono::Duration> {
        self.get_expiration().map(|exp| exp - chrono::Utc::now())
    }

    /// Check if token should be refreshed based on remaining TTL
    ///
    /// Default implementation: refresh when < 20% of original TTL remains
    fn should_refresh(&self, threshold_percentage: f32) -> bool {
        if let Some(expires_at) = self.get_expiration() {
            let now = chrono::Utc::now();

            // Already expired or about to expire
            if expires_at <= now {
                return true;
            }

            // Calculate how much time is left as percentage of total TTL
            // For simplicity, assume tokens have standard lifetimes:
            // - OAuth2 access tokens: typically 1 hour
            // - Assume we want to refresh when < threshold remains
            let time_remaining = expires_at - now;
            let threshold = chrono::Duration::seconds(
                (time_remaining.num_seconds() as f32 / threshold_percentage) as i64,
            );

            time_remaining <= threshold
        } else {
            // No expiration time, cannot determine if refresh needed
            false
        }
    }
}

/// Outcome of credential validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationOutcome {
    /// Whether validation passed
    pub passed: bool,

    /// Validation message
    pub message: String,

    /// Validation method used (e.g., "SELECT 1", "userinfo", "TLS handshake")
    pub method: String,

    /// Duration of validation
    pub duration: Duration,
}

impl ValidationOutcome {
    /// Create successful validation outcome
    pub fn success(
        message: impl Into<String>,
        method: impl Into<String>,
        duration: Duration,
    ) -> Self {
        Self {
            passed: true,
            message: message.into(),
            method: method.into(),
            duration,
        }
    }

    /// Create failed validation outcome
    pub fn failure(
        message: impl Into<String>,
        method: impl Into<String>,
        duration: Duration,
    ) -> Self {
        Self {
            passed: false,
            message: message.into(),
            method: method.into(),
            duration,
        }
    }
}

/// Validation test definition (for future use)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationTest {
    /// Test method
    pub test_method: TestMethod,

    /// Test endpoint or query
    pub endpoint: String,

    /// Expected success criteria
    pub expected_criteria: SuccessCriteria,

    /// Validation timeout
    pub timeout: Duration,
}

/// Test method for validation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TestMethod {
    /// HTTP request test
    HttpRequest {
        method: String,
        headers: Vec<(String, String)>,
    },

    /// Database query test
    DatabaseQuery { query: String },

    /// TLS handshake test
    TlsHandshake { hostname: String, port: u16 },

    /// Custom test
    Custom { command: String },
}

/// Success criteria for validation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SuccessCriteria {
    /// HTTP 2xx response
    HttpSuccess,

    /// Query returns result
    QuerySuccess,

    /// Valid TLS handshake
    HandshakeSuccess,

    /// Custom criteria
    Custom { description: String },
}

/// Validation failure classification
///
/// Categorizes validation failures to determine appropriate response.
///
/// # T074: Validation Failure Handler
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationFailureType {
    /// Network connectivity issue (transient - may succeed on retry)
    NetworkError,

    /// Authentication failure (likely permanent - credential invalid)
    AuthenticationError,

    /// Authorization failure (permission issue - likely permanent)
    AuthorizationError,

    /// Timeout during validation (transient - may succeed on retry)
    Timeout,

    /// Invalid credential format (permanent - need new credential)
    InvalidFormat,

    /// Service unavailable (transient - may succeed on retry)
    ServiceUnavailable,

    /// Unknown error (default - treat as permanent)
    Unknown,
}

impl ValidationFailureType {
    /// Check if failure is transient (worth retrying)
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            ValidationFailureType::NetworkError
                | ValidationFailureType::Timeout
                | ValidationFailureType::ServiceUnavailable
        )
    }

    /// Check if failure is permanent (should trigger rollback)
    pub fn is_permanent(&self) -> bool {
        !self.is_transient()
    }
}

/// Validation failure handler
///
/// Analyzes validation failures and determines appropriate response.
///
/// # T074: Validation Failure Handler
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::validation::ValidationFailureHandler;
///
/// let handler = ValidationFailureHandler::new();
/// let failure_type = handler.classify_error("Connection timeout");
///
/// if failure_type.is_transient() {
///     // Retry validation
/// } else {
///     // Trigger rollback
/// }
/// ```
#[derive(Debug, Clone)]
pub struct ValidationFailureHandler {
    /// Maximum retry attempts for transient failures
    pub max_retries: u32,

    /// Whether to auto-rollback on permanent failures
    pub auto_rollback: bool,
}

impl Default for ValidationFailureHandler {
    fn default() -> Self {
        Self {
            max_retries: 3,
            auto_rollback: true,
        }
    }
}

impl ValidationFailureHandler {
    /// Create a new validation failure handler
    pub fn new() -> Self {
        Self::default()
    }

    /// Classify an error message into failure type
    ///
    /// Uses heuristics based on error message content.
    pub fn classify_error(&self, error_message: &str) -> ValidationFailureType {
        let error_lower = error_message.to_lowercase();

        if error_lower.contains("timeout")
            || error_lower.contains("timed out")
            || error_lower.contains("deadline exceeded")
        {
            return ValidationFailureType::Timeout;
        }

        if error_lower.contains("network")
            || error_lower.contains("connection refused")
            || error_lower.contains("connection reset")
            || error_lower.contains("dns")
        {
            return ValidationFailureType::NetworkError;
        }

        if error_lower.contains("authentication")
            || error_lower.contains("auth failed")
            || error_lower.contains("invalid credentials")
            || error_lower.contains("unauthorized")
            || error_lower.contains("401")
        {
            return ValidationFailureType::AuthenticationError;
        }

        if error_lower.contains("authorization")
            || error_lower.contains("permission denied")
            || error_lower.contains("access denied")
            || error_lower.contains("forbidden")
            || error_lower.contains("403")
        {
            return ValidationFailureType::AuthorizationError;
        }

        if error_lower.contains("service unavailable")
            || error_lower.contains("503")
            || error_lower.contains("temporarily unavailable")
        {
            return ValidationFailureType::ServiceUnavailable;
        }

        if error_lower.contains("invalid format")
            || error_lower.contains("malformed")
            || error_lower.contains("parse error")
        {
            return ValidationFailureType::InvalidFormat;
        }

        ValidationFailureType::Unknown
    }

    /// Determine if rollback should be triggered
    ///
    /// # T075: Should Trigger Rollback
    ///
    /// # Arguments
    ///
    /// * `failure_type` - Type of validation failure
    /// * `retry_count` - Number of retries already attempted
    ///
    /// # Returns
    ///
    /// * `bool` - True if rollback should be triggered
    pub fn should_trigger_rollback(
        &self,
        failure_type: &ValidationFailureType,
        retry_count: u32,
    ) -> bool {
        // Always rollback if auto-rollback is disabled
        if !self.auto_rollback {
            return false;
        }

        // Permanent failures trigger immediate rollback
        if failure_type.is_permanent() {
            return true;
        }

        // Transient failures trigger rollback after max retries
        if failure_type.is_transient() && retry_count >= self.max_retries {
            return true;
        }

        false
    }

    /// Check if retry should be attempted
    ///
    /// # Arguments
    ///
    /// * `failure_type` - Type of validation failure
    /// * `retry_count` - Number of retries already attempted
    ///
    /// # Returns
    ///
    /// * `bool` - True if retry should be attempted
    pub fn should_retry(&self, failure_type: &ValidationFailureType, retry_count: u32) -> bool {
        failure_type.is_transient() && retry_count < self.max_retries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock credential for testing
    struct MockCredential {
        should_pass: bool,
    }

    #[async_trait]
    impl TestableCredential for MockCredential {
        async fn test(&self) -> RotationResult<ValidationOutcome> {
            let start = std::time::Instant::now();
            let duration = start.elapsed();

            if self.should_pass {
                Ok(ValidationOutcome::success(
                    "Mock test passed",
                    "mock_test",
                    duration,
                ))
            } else {
                Ok(ValidationOutcome::failure(
                    "Mock test failed",
                    "mock_test",
                    duration,
                ))
            }
        }
    }

    #[async_trait]
    impl RotatableCredential for MockCredential {
        async fn rotate(&self) -> RotationResult<Self> {
            Ok(MockCredential {
                should_pass: self.should_pass,
            })
        }
    }

    #[tokio::test]
    async fn test_testable_credential_success() {
        let cred = MockCredential { should_pass: true };
        let outcome = cred.test().await.unwrap();
        assert!(outcome.passed);
        assert_eq!(outcome.method, "mock_test");
    }

    #[tokio::test]
    async fn test_testable_credential_failure() {
        let cred = MockCredential { should_pass: false };
        let outcome = cred.test().await.unwrap();
        assert!(!outcome.passed);
    }

    #[tokio::test]
    async fn test_rotatable_credential() {
        let cred = MockCredential { should_pass: true };
        let new_cred = cred.rotate().await.unwrap();
        assert!(new_cred.should_pass);

        // Cleanup should succeed
        assert!(cred.cleanup_old().await.is_ok());
    }

    #[tokio::test]
    async fn test_validation_context() {
        let cred_id = CredentialId::new("test-cred").unwrap();
        let metadata = CredentialMetadata {
            created_at: chrono::Utc::now(),
            last_accessed: None,
            last_modified: chrono::Utc::now(),
            scope: None,
            rotation_policy: None,
            version: 1,
            expires_at: None,
            ttl_seconds: None,
            tags: std::collections::HashMap::new(),
        };

        let context = ValidationContext::new(cred_id.clone(), metadata)
            .with_timeout(Duration::from_secs(10))
            .with_retry(2);

        assert_eq!(context.credential_id, cred_id);
        assert_eq!(context.timeout, Duration::from_secs(10));
        assert!(context.is_retry);
        assert_eq!(context.retry_attempt, 2);
    }

    #[test]
    fn test_validation_failure_type_classification() {
        assert!(ValidationFailureType::NetworkError.is_transient());
        assert!(ValidationFailureType::Timeout.is_transient());
        assert!(ValidationFailureType::ServiceUnavailable.is_transient());

        assert!(ValidationFailureType::AuthenticationError.is_permanent());
        assert!(ValidationFailureType::AuthorizationError.is_permanent());
        assert!(ValidationFailureType::InvalidFormat.is_permanent());
        assert!(ValidationFailureType::Unknown.is_permanent());
    }

    #[test]
    fn test_validation_failure_handler_classify_timeout() {
        let handler = ValidationFailureHandler::new();

        let result = handler.classify_error("Connection timeout");
        assert_eq!(result, ValidationFailureType::Timeout);

        let result = handler.classify_error("Operation timed out");
        assert_eq!(result, ValidationFailureType::Timeout);
    }

    #[test]
    fn test_validation_failure_handler_classify_network() {
        let handler = ValidationFailureHandler::new();

        let result = handler.classify_error("Network error occurred");
        assert_eq!(result, ValidationFailureType::NetworkError);

        let result = handler.classify_error("Connection refused");
        assert_eq!(result, ValidationFailureType::NetworkError);
    }

    #[test]
    fn test_validation_failure_handler_classify_auth() {
        let handler = ValidationFailureHandler::new();

        let result = handler.classify_error("Authentication failed");
        assert_eq!(result, ValidationFailureType::AuthenticationError);

        let result = handler.classify_error("Invalid credentials");
        assert_eq!(result, ValidationFailureType::AuthenticationError);
    }

    #[test]
    fn test_validation_failure_handler_should_trigger_rollback() {
        let handler = ValidationFailureHandler::new();

        // Permanent failures trigger immediate rollback
        assert!(handler.should_trigger_rollback(&ValidationFailureType::AuthenticationError, 0));
        assert!(handler.should_trigger_rollback(&ValidationFailureType::InvalidFormat, 0));

        // Transient failures don't trigger rollback until max retries
        assert!(!handler.should_trigger_rollback(&ValidationFailureType::Timeout, 0));
        assert!(!handler.should_trigger_rollback(&ValidationFailureType::Timeout, 2));
        assert!(handler.should_trigger_rollback(&ValidationFailureType::Timeout, 3));
    }

    #[test]
    fn test_validation_failure_handler_should_retry() {
        let handler = ValidationFailureHandler::new();

        // Transient failures should retry
        assert!(handler.should_retry(&ValidationFailureType::Timeout, 0));
        assert!(handler.should_retry(&ValidationFailureType::NetworkError, 2));
        assert!(!handler.should_retry(&ValidationFailureType::Timeout, 3));

        // Permanent failures should not retry
        assert!(!handler.should_retry(&ValidationFailureType::AuthenticationError, 0));
        assert!(!handler.should_retry(&ValidationFailureType::InvalidFormat, 0));
    }
}
