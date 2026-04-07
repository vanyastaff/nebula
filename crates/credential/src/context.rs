//! Credential operation context
//!
//! Provides request context for observability and audit logging.

use std::any::Any;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use nebula_core::{AuthScheme, ScopeLevel};

use crate::error::CredentialError;

/// Boxed future returned by [`CredentialResolverRef::resolve_scheme`].
type ResolveSchemeResult<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn Any + Send + Sync>, CredentialError>> + Send + 'a>>;

/// Object-safe resolver for credential composition.
///
/// Allows a credential to resolve another credential during its own
/// `resolve()` or `refresh()` call (e.g., AWS Assume Role that depends
/// on a base credential for initial authentication).
///
/// # Implementors
///
/// The framework provides the concrete implementation backed by
/// [`CredentialResolver`](crate::CredentialResolver). Credential authors
/// interact with this trait indirectly through
/// [`CredentialContext::resolve_credential`].
pub trait CredentialResolverRef: Send + Sync {
    /// Resolves a credential by ID and returns the projected `AuthScheme` as `Box<dyn Any>`.
    ///
    /// The `expected_kind` parameter is used for error messages when
    /// the resolved scheme doesn't match expectations.
    fn resolve_scheme(&self, credential_id: &str, expected_kind: &str) -> ResolveSchemeResult<'_>;
}

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
#[derive(Clone)]
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

    /// Optional resolver for credential composition.
    ///
    /// When set, allows this credential's `resolve()` / `refresh()` to
    /// resolve other credentials it depends on.
    resolver: Option<Arc<dyn CredentialResolverRef>>,
}

impl fmt::Debug for CredentialContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialContext")
            .field("owner_id", &self.owner_id)
            .field("caller_scope", &self.caller_scope)
            .field("trace_id", &self.trace_id)
            .field("timestamp", &self.timestamp)
            .field("callback_url", &self.callback_url)
            .field("app_url", &self.app_url)
            .field("session_id", &self.session_id)
            .field("resolver", &self.resolver.as_ref().map(|_| ".."))
            .finish()
    }
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
            resolver: None,
        }
    }

    /// Set scope for this context (builder pattern)
    #[must_use = "builder methods must be chained or built"]
    pub fn with_scope(mut self, scope: ScopeLevel) -> Self {
        self.caller_scope = Some(scope);
        self
    }

    /// Set trace ID for this context (builder pattern)
    #[must_use = "builder methods must be chained or built"]
    pub fn with_trace_id(mut self, trace_id: Uuid) -> Self {
        self.trace_id = trace_id;
        self
    }

    /// Set OAuth2/SAML callback URL for interactive flows.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_callback_url(mut self, url: impl Into<String>) -> Self {
        self.callback_url = Some(url.into());
        self
    }

    /// Set application base URL for redirect targets.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_app_url(mut self, url: impl Into<String>) -> Self {
        self.app_url = Some(url.into());
        self
    }

    /// Set session ID for `PendingStateStore` token binding.
    #[must_use = "builder methods must be chained or built"]
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

    /// Set a credential resolver for composition (builder pattern).
    ///
    /// When a resolver is present, [`resolve_credential`](Self::resolve_credential)
    /// can be used to resolve dependent credentials during this credential's
    /// own `resolve()` or `refresh()` call.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resolver(mut self, resolver: Arc<dyn CredentialResolverRef>) -> Self {
        self.resolver = Some(resolver);
        self
    }

    /// Resolves another credential during this credential's resolution.
    ///
    /// Used for credential composition (e.g., AWS Assume Role that needs
    /// a base credential's auth material).
    ///
    /// # Errors
    ///
    /// - [`CredentialError::CompositionNotAvailable`] if no resolver was injected
    /// - [`CredentialError::CompositionFailed`] if the underlying resolution fails
    /// - [`CredentialError::SchemeMismatch`] if the resolved scheme doesn't match `S`
    pub async fn resolve_credential<S: AuthScheme>(
        &self,
        credential_id: &str,
    ) -> Result<S, CredentialError> {
        let expected_pattern = format!("{:?}", S::pattern());
        let resolver = self
            .resolver
            .as_ref()
            .ok_or(CredentialError::CompositionNotAvailable)?;
        let boxed = resolver
            .resolve_scheme(credential_id, &expected_pattern)
            .await?;
        boxed
            .downcast::<S>()
            .map(|b| *b)
            .map_err(|_| CredentialError::SchemeMismatch {
                expected: expected_pattern,
                actual: "unknown".to_string(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheme::SecretToken;
    use nebula_core::SecretString;
    use nebula_core::{ProjectId, ScopeLevel};

    struct MockResolverForComposition;
    impl CredentialResolverRef for MockResolverForComposition {
        fn resolve_scheme(&self, _id: &str, _kind: &str) -> ResolveSchemeResult<'_> {
            Box::pin(async move {
                let token = SecretToken::new(SecretString::new("composed-token"));
                Ok(Box::new(token) as Box<dyn Any + Send + Sync>)
            })
        }
    }

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

    #[tokio::test]
    async fn resolve_credential_composition() {
        let ctx =
            CredentialContext::new("test-user").with_resolver(Arc::new(MockResolverForComposition));
        let token: SecretToken = ctx.resolve_credential("base-cred").await.unwrap();
        token
            .token()
            .expose_secret(|s| assert_eq!(s, "composed-token"));
    }

    #[tokio::test]
    async fn resolve_credential_no_resolver_returns_error() {
        let ctx = CredentialContext::new("test-user");
        let result = ctx.resolve_credential::<SecretToken>("any").await;
        assert!(matches!(
            result,
            Err(CredentialError::CompositionNotAvailable)
        ));
    }
}
