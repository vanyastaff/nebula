//! Core error types for nebula-core operations.

use std::time::Duration;

use thiserror::Error;

/// Core error -- only errors that core vocabulary operations produce.
#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum CoreError {
    /// Prefixed ULID failed to parse (wrong prefix, malformed).
    #[error("invalid ID: expected prefix `{expected_prefix}_`, got `{raw}`")]
    InvalidId {
        /// The raw string that failed to parse.
        raw: String,
        /// The expected prefix.
        expected_prefix: &'static str,
    },

    /// Domain key failed validation.
    #[error("invalid key in domain `{domain}`: `{raw}`")]
    InvalidKey {
        /// The raw string that failed validation.
        raw: String,
        /// The domain name.
        domain: &'static str,
    },

    /// Scope containment violation.
    #[error("scope violation: {actor} cannot access {target}")]
    ScopeViolation {
        /// The actor scope.
        actor: String,
        /// The target scope.
        target: String,
    },

    /// Dependency cycle detected (Tarjan SCC).
    #[error("dependency cycle: {}", path.join(" -> "))]
    DependencyCycle {
        /// The path forming the cycle.
        path: Vec<&'static str>,
    },

    /// Required dependency not registered.
    ///
    /// Both `name` and `required_by` are `&'static str` intentionally:
    /// dependency names are compile-time constants defined in action/plugin
    /// metadata, never user-supplied strings. This keeps `CoreError: Clone`
    /// without allocating and makes the error zero-cost to construct.
    #[error("missing dependency: `{required_by}` requires `{name}`")]
    DependencyMissing {
        /// The name of the missing dependency.
        name: &'static str,
        /// The component requiring it.
        required_by: &'static str,
    },

    /// Credential capability is not configured in context.
    #[error("credential not configured: {0}")]
    CredentialNotConfigured(String),

    /// Credential not found by key.
    #[error("credential not found: {key}")]
    CredentialNotFound {
        /// The key that was not found.
        key: String,
    },

    /// Credential access denied (action not authorized for this key).
    #[error("credential access denied: `{capability}` for action `{action_id}`")]
    CredentialAccessDenied {
        /// Description of the denied capability.
        capability: String,
        /// The action that attempted access.
        action_id: String,
    },

    /// Resource lease/acquire failed (surfaced through the shared accessor seam).
    ///
    /// Actions reach this via [`ResourceAccessor`](crate::accessor::ResourceAccessor);
    /// retryable variants are converted to retryable action errors at the action boundary.
    #[error("resource unavailable: {key}: {detail}")]
    ResourceUnavailable {
        /// Resource key label for observability.
        key: String,
        /// Human-readable failure detail (no secret bytes).
        detail: String,
        /// When true, the action runtime may retry with backoff.
        retryable: bool,
        /// Optional retry delay hint from the resource layer.
        retry_after: Option<Duration>,
    },
}

impl CoreError {
    /// Create an invalid ID error.
    pub fn invalid_id(raw: impl Into<String>, expected_prefix: &'static str) -> Self {
        Self::InvalidId {
            raw: raw.into(),
            expected_prefix,
        }
    }

    /// Create an invalid key error.
    pub fn invalid_key(raw: impl Into<String>, domain: &'static str) -> Self {
        Self::InvalidKey {
            raw: raw.into(),
            domain,
        }
    }

    /// Create a scope violation error.
    pub fn scope_violation(actor: impl Into<String>, target: impl Into<String>) -> Self {
        Self::ScopeViolation {
            actor: actor.into(),
            target: target.into(),
        }
    }

    /// Create a dependency cycle error.
    pub fn dependency_cycle(path: Vec<&'static str>) -> Self {
        Self::DependencyCycle { path }
    }

    /// Create a missing dependency error.
    pub fn dependency_missing(name: &'static str, required_by: &'static str) -> Self {
        Self::DependencyMissing { name, required_by }
    }

    /// Create a resource-acquire failure surfaced through the accessor seam.
    pub fn resource_unavailable(
        key: impl Into<String>,
        detail: impl Into<String>,
        retryable: bool,
        retry_after: Option<Duration>,
    ) -> Self {
        Self::ResourceUnavailable {
            key: key.into(),
            detail: detail.into(),
            retryable,
            retry_after,
        }
    }
}

/// Result type for core operations.
pub type CoreResult<T> = Result<T, CoreError>;

impl nebula_error::Classify for CoreError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::InvalidId { .. } | Self::InvalidKey { .. } => {
                nebula_error::ErrorCategory::Validation
            },
            Self::ScopeViolation { .. } => nebula_error::ErrorCategory::Authorization,
            Self::DependencyCycle { .. } | Self::DependencyMissing { .. } => {
                nebula_error::ErrorCategory::Validation
            },
            Self::CredentialNotConfigured(_) | Self::CredentialNotFound { .. } => {
                nebula_error::ErrorCategory::NotFound
            },
            Self::CredentialAccessDenied { .. } => nebula_error::ErrorCategory::Authorization,
            Self::ResourceUnavailable {
                retryable: true, ..
            } => nebula_error::ErrorCategory::Unavailable,
            Self::ResourceUnavailable {
                retryable: false, ..
            } => nebula_error::ErrorCategory::Cancelled,
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new(match self {
            Self::InvalidId { .. } => "CORE:INVALID_ID",
            Self::InvalidKey { .. } => "CORE:INVALID_KEY",
            Self::ScopeViolation { .. } => "CORE:SCOPE_VIOLATION",
            Self::DependencyCycle { .. } => "CORE:DEPENDENCY_CYCLE",
            Self::DependencyMissing { .. } => "CORE:DEPENDENCY_MISSING",
            Self::CredentialNotConfigured(_) => "CORE:CREDENTIAL_NOT_CONFIGURED",
            Self::CredentialNotFound { .. } => "CORE:CREDENTIAL_NOT_FOUND",
            Self::CredentialAccessDenied { .. } => "CORE:CREDENTIAL_ACCESS_DENIED",
            Self::ResourceUnavailable { .. } => "CORE:RESOURCE_UNAVAILABLE",
        })
    }

    fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::ResourceUnavailable {
                retryable: true,
                ..
            }
        )
    }

    fn retry_hint(&self) -> Option<nebula_error::RetryHint> {
        match self {
            Self::ResourceUnavailable {
                retry_after: Some(d),
                retryable: true,
                ..
            } => Some(nebula_error::RetryHint::after(*d)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let e = CoreError::invalid_id("bad", "exe");
        assert!(e.to_string().contains("exe"));
    }

    #[test]
    fn all_variants_not_retryable() {
        use nebula_error::Classify;
        let errors = [
            CoreError::invalid_id("x", "exe"),
            CoreError::invalid_key("x", "action"),
            CoreError::scope_violation("a", "b"),
            CoreError::dependency_cycle(vec!["a", "b"]),
            CoreError::dependency_missing("x", "y"),
        ];
        for e in &errors {
            assert!(!e.is_retryable());
        }
    }

    #[test]
    fn helper_constructors_match_variants() {
        assert!(matches!(
            CoreError::scope_violation("actor", "target"),
            CoreError::ScopeViolation { .. }
        ));
        assert!(matches!(
            CoreError::dependency_cycle(vec!["a", "b"]),
            CoreError::DependencyCycle { .. }
        ));
        assert!(matches!(
            CoreError::dependency_missing("dep", "owner"),
            CoreError::DependencyMissing { .. }
        ));
    }
}
