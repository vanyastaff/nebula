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

    /// OAuth2/SAML callback URL for interactive credential flows.
    callback_url: Option<String>,

    /// Application base URL for redirect targets.
    app_url: Option<String>,

    /// Session ID for `PendingStateStore` token binding.
    session_id: Option<String>,
}

impl CredentialContext {
    /// Create new context with owner
    pub fn new(owner_id: impl Into<String>) -> Self {
        Self {
            owner_id: owner_id.into(),
            caller_scope: None,
            trace_id: Uuid::new_v4(),
            timestamp: Utc::now(),
            callback_url: None,
            app_url: None,
            session_id: None,
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

    /// Set OAuth2/SAML callback URL for interactive flows.
    pub fn with_callback_url(mut self, url: impl Into<String>) -> Self {
        self.callback_url = Some(url.into());
        self
    }

    /// Set application base URL for redirect targets.
    pub fn with_app_url(mut self, url: impl Into<String>) -> Self {
        self.app_url = Some(url.into());
        self
    }

    /// Set session ID for `PendingStateStore` token binding.
    pub fn with_session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }

    /// Returns the callback URL for OAuth2/SAML redirects.
    pub fn callback_url(&self) -> Option<&str> {
        self.callback_url.as_deref()
    }

    /// Returns the application base URL.
    pub fn app_url(&self) -> Option<&str> {
        self.app_url.as_deref()
    }

    /// Returns the session ID for pending state binding.
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
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
        assert!(
            matches!(ctx.caller_scope.as_ref(), Some(ScopeLevel::Project(id)) if *id == project_id)
        );
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
    fn test_context_with_callback_url() {
        let ctx =
            CredentialContext::new("user_123").with_callback_url("https://app.nebula.io/callback");
        assert_eq!(ctx.callback_url(), Some("https://app.nebula.io/callback"));
    }

    #[test]
    fn test_context_with_app_url() {
        let ctx = CredentialContext::new("user_123").with_app_url("https://app.nebula.io");
        assert_eq!(ctx.app_url(), Some("https://app.nebula.io"));
    }

    #[test]
    fn test_context_with_session_id() {
        let ctx = CredentialContext::new("user_123").with_session_id("session-abc-123");
        assert_eq!(ctx.session_id(), Some("session-abc-123"));
    }

    #[test]
    fn test_context_defaults_none() {
        let ctx = CredentialContext::new("user_123");
        assert!(ctx.callback_url().is_none());
        assert!(ctx.app_url().is_none());
        assert!(ctx.session_id().is_none());
    }

    #[test]
    fn test_full_builder_chain() {
        let ctx = CredentialContext::new("user_123")
            .with_scope(ScopeLevel::Project(ProjectId::new()))
            .with_callback_url("https://app/callback")
            .with_app_url("https://app")
            .with_session_id("sess-1");

        assert!(ctx.callback_url().is_some());
        assert!(ctx.app_url().is_some());
        assert!(ctx.session_id().is_some());
        assert!(ctx.caller_scope.is_some());
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
