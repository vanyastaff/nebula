//! Cloud/infrastructure instance identity (IMDS, managed identity).

use nebula_core::{AuthPattern, AuthScheme};
use serde::{Deserialize, Serialize};

/// Instance-level identity binding for cloud/infrastructure authentication.
///
/// Represents credentials that are derived from the execution environment
/// rather than explicitly configured — e.g., AWS EC2 instance profiles,
/// Azure managed identities, GCP service account bindings, or Kubernetes
/// workload identity.
///
/// This type contains **no secrets** — authentication happens via the
/// cloud provider's instance metadata service (IMDS) at runtime, using
/// the provider, role/account, and region as lookup parameters.
///
/// # Examples
///
/// ```
/// use nebula_credential::scheme::InstanceBinding;
///
/// let binding = InstanceBinding::new("aws", "arn:aws:iam::123456789012:role/MyRole")
///     .with_region("us-east-1");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceBinding {
    provider: String,
    role_or_account: String,
    region: Option<String>,
}

impl InstanceBinding {
    /// Creates a new instance binding for the given provider and role/account.
    #[must_use]
    pub fn new(provider: impl Into<String>, role_or_account: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            role_or_account: role_or_account.into(),
            region: None,
        }
    }

    /// Sets the region or zone for the instance binding.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_region(mut self, region: impl Into<String>) -> Self {
        self.region = Some(region.into());
        self
    }

    /// Returns the cloud provider identifier (e.g., `"aws"`, `"azure"`, `"gcp"`).
    pub fn provider(&self) -> &str {
        &self.provider
    }

    /// Returns the role ARN, managed identity client ID, or service account email.
    pub fn role_or_account(&self) -> &str {
        &self.role_or_account
    }

    /// Returns the optional region or zone.
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }
}

impl AuthScheme for InstanceBinding {
    fn pattern() -> AuthPattern {
        AuthPattern::InstanceIdentity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_instance_identity() {
        assert_eq!(InstanceBinding::pattern(), AuthPattern::InstanceIdentity);
    }

    #[test]
    fn debug_exposes_no_secrets_because_there_are_none() {
        let binding = InstanceBinding::new("aws", "arn:aws:iam::123456789012:role/MyRole")
            .with_region("us-east-1");
        let debug = format!("{binding:?}");
        // No secrets to redact — all fields are safe to display.
        assert!(debug.contains("aws"));
        assert!(debug.contains("us-east-1"));
    }
}
