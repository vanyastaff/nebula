//! Core error types for nebula-core operations.

use thiserror::Error;

/// Core error -- only errors that core vocabulary operations produce.
#[derive(Error, Debug, Clone)]
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
    #[error("missing dependency: `{required_by}` requires `{name}`")]
    DependencyMissing {
        /// The name of the missing dependency.
        name: &'static str,
        /// The component requiring it.
        required_by: &'static str,
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
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new(match self {
            Self::InvalidId { .. } => "CORE:INVALID_ID",
            Self::InvalidKey { .. } => "CORE:INVALID_KEY",
            Self::ScopeViolation { .. } => "CORE:SCOPE_VIOLATION",
            Self::DependencyCycle { .. } => "CORE:DEPENDENCY_CYCLE",
            Self::DependencyMissing { .. } => "CORE:DEPENDENCY_MISSING",
        })
    }

    fn is_retryable(&self) -> bool {
        false
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
            CoreError::ScopeViolation {
                actor: "a".into(),
                target: "b".into(),
            },
            CoreError::DependencyCycle {
                path: vec!["a", "b"],
            },
            CoreError::DependencyMissing {
                name: "x",
                required_by: "y",
            },
        ];
        for e in &errors {
            assert!(!e.is_retryable());
        }
    }
}
