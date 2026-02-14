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
//!     async fn test(&self) -> RotationResult<TestResult> {
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
//!         Ok(TestResult::success(
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

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::core::{CredentialId, CredentialMetadata};
use crate::traits::TestableCredential;

use super::error::{RotationError, RotationResult};
use tokio::time::timeout;

/// Context for credential testing
#[derive(Debug, Clone)]
pub struct TestContext {
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

impl TestContext {
    /// Create a new test context
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

    /// Test a credential with timeout enforcement
    ///
    /// Wraps the credential's `test()` method with a timeout to prevent
    /// testing from hanging indefinitely.
    ///
    /// # Arguments
    ///
    /// * `credential` - The credential to test
    ///
    /// # Returns
    ///
    /// * `Ok(TestResult)` - Test completed within timeout
    /// * `Err(RotationError::Timeout)` - Test exceeded timeout
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let context = TestContext::new(cred_id, metadata)
    ///     .with_timeout(Duration::from_secs(10));
    ///
    /// let result = context.test(&mysql_credential).await?;
    /// ```
    pub async fn test<T: TestableCredential>(&self, credential: &T) -> RotationResult<TestResult> {
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

/// Result of credential testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// Whether validation passed
    pub passed: bool,

    /// Validation message
    pub message: String,

    /// Validation method used (e.g., "SELECT 1", "userinfo", "TLS handshake")
    pub method: String,

    /// Duration of validation
    pub duration: Duration,
}

impl TestResult {
    /// Create successful test result
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

    /// Create failed test result
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
pub enum FailureKind {
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

impl FailureKind {
    /// Check if failure is transient (worth retrying)
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            FailureKind::NetworkError | FailureKind::Timeout | FailureKind::ServiceUnavailable
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
/// use nebula_credential::rotation::validation::FailureHandler;
///
/// let handler = FailureHandler::new();
/// let failure_type = handler.classify_error("Connection timeout");
///
/// if failure_type.is_transient() {
///     // Retry validation
/// } else {
///     // Trigger rollback
/// }
/// ```
#[derive(Debug, Clone)]
pub struct FailureHandler {
    /// Maximum retry attempts for transient failures
    pub max_retries: u32,

    /// Whether to auto-rollback on permanent failures
    pub auto_rollback: bool,
}

impl Default for FailureHandler {
    fn default() -> Self {
        Self {
            max_retries: 3,
            auto_rollback: true,
        }
    }
}

impl FailureHandler {
    /// Create a new validation failure handler
    pub fn new() -> Self {
        Self::default()
    }

    /// Classify an error message into failure type
    ///
    /// Uses heuristics based on error message content.
    pub fn classify_error(&self, error_message: &str) -> FailureKind {
        let error_lower = error_message.to_lowercase();

        if error_lower.contains("timeout")
            || error_lower.contains("timed out")
            || error_lower.contains("deadline exceeded")
        {
            return FailureKind::Timeout;
        }

        if error_lower.contains("network")
            || error_lower.contains("connection refused")
            || error_lower.contains("connection reset")
            || error_lower.contains("dns")
        {
            return FailureKind::NetworkError;
        }

        if error_lower.contains("authentication")
            || error_lower.contains("auth failed")
            || error_lower.contains("invalid credentials")
            || error_lower.contains("unauthorized")
            || error_lower.contains("401")
        {
            return FailureKind::AuthenticationError;
        }

        if error_lower.contains("authorization")
            || error_lower.contains("permission denied")
            || error_lower.contains("access denied")
            || error_lower.contains("forbidden")
            || error_lower.contains("403")
        {
            return FailureKind::AuthorizationError;
        }

        if error_lower.contains("service unavailable")
            || error_lower.contains("503")
            || error_lower.contains("temporarily unavailable")
        {
            return FailureKind::ServiceUnavailable;
        }

        if error_lower.contains("invalid format")
            || error_lower.contains("malformed")
            || error_lower.contains("parse error")
        {
            return FailureKind::InvalidFormat;
        }

        FailureKind::Unknown
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
    pub fn should_trigger_rollback(&self, failure_type: &FailureKind, retry_count: u32) -> bool {
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
    pub fn should_retry(&self, failure_type: &FailureKind, retry_count: u32) -> bool {
        failure_type.is_transient() && retry_count < self.max_retries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    use crate::core::result::InitializeResult;
    use crate::core::{CredentialContext, CredentialDescription, CredentialError, CredentialState};
    use crate::traits::{Credential, RotatableCredential};

    #[derive(Clone, Serialize, Deserialize)]
    struct MockState;

    impl CredentialState for MockState {
        const VERSION: u16 = 1;
        const KIND: &'static str = "mock";
    }

    #[derive(Serialize, Deserialize)]
    struct MockInput;

    // Mock credential for testing
    struct MockCredential {
        should_pass: bool,
    }

    #[async_trait]
    impl Credential for MockCredential {
        type Input = MockInput;
        type State = MockState;

        fn description(&self) -> CredentialDescription {
            CredentialDescription::builder()
                .key("mock")
                .name("Mock Credential")
                .description("Mock credential for testing")
                .build()
                .unwrap()
        }

        async fn initialize(
            &self,
            _input: &Self::Input,
            _ctx: &mut CredentialContext,
        ) -> Result<InitializeResult<Self::State>, CredentialError> {
            Ok(InitializeResult::Complete(MockState))
        }

        async fn refresh(
            &self,
            _state: &mut Self::State,
            _ctx: &mut CredentialContext,
        ) -> Result<(), CredentialError> {
            Ok(())
        }

        async fn revoke(
            &self,
            _state: &mut Self::State,
            _ctx: &mut CredentialContext,
        ) -> Result<(), CredentialError> {
            Ok(())
        }
    }

    #[async_trait]
    impl TestableCredential for MockCredential {
        async fn test(&self) -> RotationResult<TestResult> {
            let start = std::time::Instant::now();
            let duration = start.elapsed();

            if self.should_pass {
                Ok(TestResult::success(
                    "Mock test passed",
                    "mock_test",
                    duration,
                ))
            } else {
                Ok(TestResult::failure(
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

        let context = TestContext::new(cred_id.clone(), metadata)
            .with_timeout(Duration::from_secs(10))
            .with_retry(2);

        assert_eq!(context.credential_id, cred_id);
        assert_eq!(context.timeout, Duration::from_secs(10));
        assert!(context.is_retry);
        assert_eq!(context.retry_attempt, 2);
    }

    #[test]
    fn test_validation_failure_type_classification() {
        assert!(FailureKind::NetworkError.is_transient());
        assert!(FailureKind::Timeout.is_transient());
        assert!(FailureKind::ServiceUnavailable.is_transient());

        assert!(FailureKind::AuthenticationError.is_permanent());
        assert!(FailureKind::AuthorizationError.is_permanent());
        assert!(FailureKind::InvalidFormat.is_permanent());
        assert!(FailureKind::Unknown.is_permanent());
    }

    #[test]
    fn test_validation_failure_handler_classify_timeout() {
        let handler = FailureHandler::new();

        let result = handler.classify_error("Connection timeout");
        assert_eq!(result, FailureKind::Timeout);

        let result = handler.classify_error("Operation timed out");
        assert_eq!(result, FailureKind::Timeout);
    }

    #[test]
    fn test_validation_failure_handler_classify_network() {
        let handler = FailureHandler::new();

        let result = handler.classify_error("Network error occurred");
        assert_eq!(result, FailureKind::NetworkError);

        let result = handler.classify_error("Connection refused");
        assert_eq!(result, FailureKind::NetworkError);
    }

    #[test]
    fn test_validation_failure_handler_classify_auth() {
        let handler = FailureHandler::new();

        let result = handler.classify_error("Authentication failed");
        assert_eq!(result, FailureKind::AuthenticationError);

        let result = handler.classify_error("Invalid credentials");
        assert_eq!(result, FailureKind::AuthenticationError);
    }

    #[test]
    fn test_validation_failure_handler_should_trigger_rollback() {
        let handler = FailureHandler::new();

        // Permanent failures trigger immediate rollback
        assert!(handler.should_trigger_rollback(&FailureKind::AuthenticationError, 0));
        assert!(handler.should_trigger_rollback(&FailureKind::InvalidFormat, 0));

        // Transient failures don't trigger rollback until max retries
        assert!(!handler.should_trigger_rollback(&FailureKind::Timeout, 0));
        assert!(!handler.should_trigger_rollback(&FailureKind::Timeout, 2));
        assert!(handler.should_trigger_rollback(&FailureKind::Timeout, 3));
    }

    #[test]
    fn test_validation_failure_handler_should_retry() {
        let handler = FailureHandler::new();

        // Transient failures should retry
        assert!(handler.should_retry(&FailureKind::Timeout, 0));
        assert!(handler.should_retry(&FailureKind::NetworkError, 2));
        assert!(!handler.should_retry(&FailureKind::Timeout, 3));

        // Permanent failures should not retry
        assert!(!handler.should_retry(&FailureKind::AuthenticationError, 0));
        assert!(!handler.should_retry(&FailureKind::InvalidFormat, 0));
    }
}
