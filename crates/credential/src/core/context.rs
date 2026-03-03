//! Credential operation context
//!
//! Provides request context for observability and audit logging.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use nebula_core::ScopeLevel;

/// Request context for credential operations
///
/// Carries owner, scope, and tracing metadata for observability
/// and audit logging.
///
/// # Examples
///
/// ```
/// use nebula_credential::CredentialContext;
/// use nebula_core::{ProjectId, ScopeLevel};
///
/// // Basic context
/// let ctx = CredentialContext::new("user_123");
///
/// // With scope for multi-tenancy
/// let project_id = ProjectId::new();
/// let ctx = CredentialContext::new("user_123")
///     .with_scope(ScopeLevel::Project(project_id));
///
/// // With custom trace ID
/// use uuid::Uuid;
/// let trace_id = Uuid::new_v4();
/// let ctx = CredentialContext::new("user_123")
///     .with_trace_id(trace_id);
/// ```
#[derive(Debug, Clone)]
pub struct CredentialContext {
    /// Owner of the credential
    pub owner_id: String,

    /// Optional scope for isolation (multi-tenancy support).
    /// Uses `ScopeLevel` from nebula-core for platform consistency.
    pub caller_scope: Option<ScopeLevel>,

    /// Trace ID for distributed tracing
    pub trace_id: Uuid,

    /// Timestamp of the request
    pub timestamp: DateTime<Utc>,
}

impl CredentialContext {
    /// Create new context with owner
    pub fn new(owner_id: impl Into<String>) -> Self {
        Self {
            owner_id: owner_id.into(),
            caller_scope: None,
            trace_id: Uuid::new_v4(),
            timestamp: Utc::now(),
        }
    }

    /// Set scope for this context (builder pattern)
    pub fn with_scope(mut self, scope: ScopeLevel) -> Self {
        self.caller_scope = Some(scope);
        self
    }

    /// Set trace ID for this context (builder pattern)
    pub fn with_trace_id(mut self, trace_id: Uuid) -> Self {
        self.trace_id = trace_id;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_core::{ProjectId, ScopeLevel};

    #[test]
    fn test_context_new() {
        let ctx = CredentialContext::new("user_123");
        assert_eq!(ctx.owner_id, "user_123");
        assert!(ctx.caller_scope.is_none());
    }

    #[test]
    fn test_context_with_scope() {
        let project_id = ProjectId::new();
        let ctx = CredentialContext::new("user_123").with_scope(ScopeLevel::Project(project_id));

        assert_eq!(ctx.owner_id, "user_123");
        assert!(matches!(ctx.caller_scope.as_ref(), Some(ScopeLevel::Project(id)) if *id == project_id));
    }

    #[test]
    fn test_context_with_trace_id() {
        let custom_trace = Uuid::new_v4();
        let ctx = CredentialContext::new("user_123").with_trace_id(custom_trace);

        assert_eq!(ctx.trace_id, custom_trace);
    }

    #[test]
    fn test_context_builder_pattern() {
        let trace = Uuid::new_v4();
        let project_id = ProjectId::new();
        let ctx = CredentialContext::new("user_123")
            .with_scope(ScopeLevel::Project(project_id))
            .with_trace_id(trace);

        assert_eq!(ctx.owner_id, "user_123");
        assert!(ctx.caller_scope.is_some());
        assert_eq!(ctx.trace_id, trace);
    }

    #[test]
    fn test_context_clone() {
        let project_id = ProjectId::new();
        let ctx1 = CredentialContext::new("user_123").with_scope(ScopeLevel::Project(project_id));
        let ctx2 = ctx1.clone();

        assert_eq!(ctx1.owner_id, ctx2.owner_id);
        assert_eq!(ctx1.caller_scope, ctx2.caller_scope);
        assert_eq!(ctx1.trace_id, ctx2.trace_id);
    }

    #[test]
    fn test_context_timestamp() {
        let before = Utc::now();
        let ctx = CredentialContext::new("user_123");
        let after = Utc::now();

        assert!(ctx.timestamp >= before);
        assert!(ctx.timestamp <= after);
    }
}
