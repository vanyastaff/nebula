//! AWS authentication (IAM credentials).

use nebula_core::AuthScheme;
use serde::{Deserialize, Serialize};

use crate::utils::SecretString;

/// AWS IAM credentials for AWS API authentication.
///
/// Produced by: AWS credential configurations, STS assume-role.
/// Consumed by: AWS SDK clients (S3, Lambda, DynamoDB, etc.)
#[derive(Clone, Serialize, Deserialize)]
pub struct AwsAuth {
    /// AWS access key ID.
    access_key_id: SecretString,
    /// AWS secret access key.
    secret_access_key: SecretString,
    /// Temporary session token from STS.
    session_token: Option<SecretString>,
    /// AWS region (e.g., `"us-east-1"`).
    pub region: String,
}

impl AwsAuth {
    /// Creates new AWS credentials with an access key pair and region.
    pub fn new(
        access_key_id: SecretString,
        secret_access_key: SecretString,
        region: impl Into<String>,
    ) -> Self {
        Self {
            access_key_id,
            secret_access_key,
            session_token: None,
            region: region.into(),
        }
    }

    /// Sets a temporary session token (from STS).
    pub fn with_session_token(mut self, token: SecretString) -> Self {
        self.session_token = Some(token);
        self
    }

    /// Returns the access key ID.
    pub fn access_key_id(&self) -> &SecretString {
        &self.access_key_id
    }

    /// Returns the secret access key.
    pub fn secret_access_key(&self) -> &SecretString {
        &self.secret_access_key
    }

    /// Returns the session token, if set.
    pub fn session_token(&self) -> Option<&SecretString> {
        self.session_token.as_ref()
    }
}

impl AuthScheme for AwsAuth {
    const KIND: &'static str = "aws";
}

impl std::fmt::Debug for AwsAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsAuth")
            .field("access_key_id", &"[REDACTED]")
            .field("secret_access_key", &"[REDACTED]")
            .field(
                "session_token",
                if self.session_token.is_some() {
                    &"Some([REDACTED])"
                } else {
                    &"None"
                },
            )
            .field("region", &self.region)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_is_correct() {
        assert_eq!(AwsAuth::KIND, "aws");
    }

    #[test]
    fn debug_redacts_secrets() {
        let auth = AwsAuth::new(
            SecretString::new("AKIAIOSFODNN7EXAMPLE"),
            SecretString::new("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
            "us-east-1",
        )
        .with_session_token(SecretString::new("session123"));
        let debug = format!("{auth:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!debug.contains("wJalrXUtnFEMI"));
        assert!(!debug.contains("session123"));
        assert!(debug.contains("us-east-1"));
    }

    #[test]
    fn accessors_return_secrets() {
        let auth = AwsAuth::new(
            SecretString::new("key-id"),
            SecretString::new("secret-key"),
            "eu-west-1",
        );
        auth.access_key_id()
            .expose_secret(|v| assert_eq!(v, "key-id"));
        auth.secret_access_key()
            .expose_secret(|v| assert_eq!(v, "secret-key"));
        assert!(auth.session_token().is_none());
    }
}
