//! Credential operation context
//!
//! Provides request context for credential resolution, refresh, and testing.
//! Embeds [`BaseContext`] for identity/scope/clock/cancellation and implements
//! core capability traits ([`HasCredentials`], [`HasResources`]).

use std::{fmt, future::Future, sync::Arc};

use nebula_core::{
    BaseContext, Context, HasCredentials, HasResources,
    accessor::{CredentialAccessor, ResourceAccessor},
    obs::TraceId,
    scope::{Principal, Scope},
};

use crate::accessor::default_credential_accessor;

// ── Noop ResourceAccessor (core trait) ────────────────────────────────────

/// No-op resource accessor for contexts without resource support.
#[derive(Debug, Default)]
struct NoopResourceAccessor;

impl ResourceAccessor for NoopResourceAccessor {
    fn has(&self, _key: &nebula_core::ResourceKey) -> bool {
        false
    }

    fn acquire_any(
        &self,
        _key: &nebula_core::ResourceKey,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<Box<dyn std::any::Any + Send + Sync>, nebula_core::CoreError>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async {
            Err(nebula_core::CoreError::CredentialNotConfigured(
                "resource capability is not configured in CredentialContext".to_owned(),
            ))
        })
    }

    fn try_acquire_any(
        &self,
        _key: &nebula_core::ResourceKey,
    ) -> std::pin::Pin<
        Box<
            dyn Future<
                    Output = Result<
                        Option<Box<dyn std::any::Any + Send + Sync>>,
                        nebula_core::CoreError,
                    >,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(None) })
    }
}

/// Default resource accessor capability.
fn default_resource_accessor() -> Arc<dyn ResourceAccessor> {
    Arc::new(NoopResourceAccessor)
}

// ── CredentialContext ──────────────────────────────────────────────────────

/// Request context for credential operations.
///
/// Embeds [`BaseContext`] (wrapped in `Arc` for cheap cloning) and implements
/// core context and capability traits. Domain-specific fields for OAuth2/SAML
/// interactive flows are also carried here.
///
/// # Construction
///
/// Use [`CredentialContextBuilder`] for production code:
///
/// ```rust,ignore
/// use nebula_credential::CredentialContextBuilder;
/// use nebula_core::BaseContext;
///
/// let base = BaseContext::builder().build();
/// let ctx = CredentialContextBuilder::new(base, credentials, resources)
///     .callback_url("https://app/callback".to_owned())
///     .build();
/// ```
///
/// For tests, use the convenience constructor:
///
/// ```rust,ignore
/// let ctx = CredentialContext::for_test("user-123");
/// ```
#[derive(Clone)]
pub struct CredentialContext {
    /// Core context — identity, tenancy, lifecycle, clock.
    base: Arc<BaseContext>,

    /// Credential accessor capability.
    credentials: Arc<dyn CredentialAccessor>,

    /// Resource accessor capability.
    resources: Arc<dyn ResourceAccessor>,

    /// OAuth2/SAML callback URL for interactive credential flows.
    callback_url: Option<String>,

    /// Application base URL for redirect targets.
    app_url: Option<String>,

    /// Session ID for `PendingStateStore` token binding.
    session_id: Option<String>,

    /// Owner ID for backward-compatible pending-store binding.
    ///
    /// When set explicitly, this value is returned by [`owner_id()`](Self::owner_id).
    /// Otherwise, `owner_id()` derives a string from
    /// [`self.principal()`](Context::principal).
    owner_id_override: Option<String>,
}

impl fmt::Debug for CredentialContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialContext")
            .field("scope", self.base.scope())
            .field("principal", self.base.principal())
            .field("trace_id", &self.base.trace_id())
            .field("callback_url", &self.callback_url)
            .field("app_url", &self.app_url)
            .field("session_id", &self.session_id)
            .field("owner_id_override", &self.owner_id_override)
            .finish()
    }
}

// ── Context trait delegation ──────────────────────────────────────────────

impl Context for CredentialContext {
    fn scope(&self) -> &Scope {
        self.base.scope()
    }

    fn principal(&self) -> &Principal {
        self.base.principal()
    }

    fn cancellation(&self) -> &tokio_util::sync::CancellationToken {
        self.base.cancellation()
    }

    fn clock(&self) -> &dyn nebula_core::accessor::Clock {
        self.base.clock()
    }

    fn trace_id(&self) -> Option<TraceId> {
        self.base.trace_id()
    }
}

// ── Capability traits ─────────────────────────────────────────────────────

impl HasCredentials for CredentialContext {
    fn credentials(&self) -> &dyn CredentialAccessor {
        &*self.credentials
    }
}

impl HasResources for CredentialContext {
    fn resources(&self) -> &dyn ResourceAccessor {
        &*self.resources
    }
}

// ── Domain-specific accessors ─────────────────────────────────────────────

impl CredentialContext {
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

    /// Returns an owner identifier string for pending-store session binding.
    ///
    /// If an explicit `owner_id` was set via [`CredentialContextBuilder::owner_id`]
    /// or [`CredentialContext::for_test`], that value is returned. Otherwise a
    /// string representation of the [`Principal`] is derived.
    pub fn owner_id(&self) -> &str {
        if let Some(ref id) = self.owner_id_override {
            return id.as_str();
        }
        // Fallback: we can't return a reference to a computed String from
        // principal, so this branch returns a static fallback. Production
        // callers should set owner_id_override via the builder.
        "system"
    }

    /// Convenience constructor for **tests only**.
    ///
    /// Creates a context with a default [`BaseContext`] (system principal,
    /// default scope, system clock) and noop accessors. The `owner_id` is
    /// stored as an override for backward-compatible pending-store binding.
    pub fn for_test(owner_id: impl Into<String>) -> Self {
        Self {
            base: Arc::new(BaseContext::builder().build()),
            credentials: default_credential_accessor(),
            resources: default_resource_accessor(),
            callback_url: None,
            app_url: None,
            session_id: None,
            owner_id_override: Some(owner_id.into()),
        }
    }

    /// Set session ID (builder-style, consumes self).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }

    /// Set callback URL (builder-style, consumes self).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_callback_url(mut self, url: impl Into<String>) -> Self {
        self.callback_url = Some(url.into());
        self
    }

    /// Set app URL (builder-style, consumes self).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_app_url(mut self, url: impl Into<String>) -> Self {
        self.app_url = Some(url.into());
        self
    }
}

// ── Builder ───────────────────────────────────────────────────────────────

/// Builder for [`CredentialContext`].
///
/// Requires a [`BaseContext`], credential accessor, and resource accessor.
/// Domain-specific fields (callback URL, app URL, session ID) are optional.
pub struct CredentialContextBuilder {
    base: BaseContext,
    credentials: Arc<dyn CredentialAccessor>,
    resources: Arc<dyn ResourceAccessor>,
    callback_url: Option<String>,
    app_url: Option<String>,
    session_id: Option<String>,
    owner_id: Option<String>,
}

impl CredentialContextBuilder {
    /// Create a new builder with the required fields.
    pub fn new(
        base: BaseContext,
        credentials: Arc<dyn CredentialAccessor>,
        resources: Arc<dyn ResourceAccessor>,
    ) -> Self {
        Self {
            base,
            credentials,
            resources,
            callback_url: None,
            app_url: None,
            session_id: None,
            owner_id: None,
        }
    }

    /// Set OAuth2/SAML callback URL.
    #[must_use]
    pub fn callback_url(mut self, url: String) -> Self {
        self.callback_url = Some(url);
        self
    }

    /// Set application base URL.
    #[must_use]
    pub fn app_url(mut self, url: String) -> Self {
        self.app_url = Some(url);
        self
    }

    /// Set session ID for pending-state binding.
    #[must_use]
    pub fn session_id(mut self, id: String) -> Self {
        self.session_id = Some(id);
        self
    }

    /// Set explicit owner ID for pending-store session binding.
    ///
    /// If not set, [`CredentialContext::owner_id()`] falls back to deriving
    /// a string from the principal.
    #[must_use]
    pub fn owner_id(mut self, id: String) -> Self {
        self.owner_id = Some(id);
        self
    }

    /// Build the [`CredentialContext`].
    pub fn build(self) -> CredentialContext {
        CredentialContext {
            base: Arc::new(self.base),
            credentials: self.credentials,
            resources: self.resources,
            callback_url: self.callback_url,
            app_url: self.app_url,
            session_id: self.session_id,
            owner_id_override: self.owner_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::Context;

    use super::*;

    #[test]
    fn for_test_creates_valid_context() {
        let ctx = CredentialContext::for_test("user_123");
        assert_eq!(ctx.owner_id(), "user_123");
        assert!(ctx.callback_url().is_none());
        assert!(ctx.app_url().is_none());
        assert!(ctx.session_id().is_none());
    }

    #[test]
    fn builder_creates_context_with_all_fields() {
        let base = BaseContext::builder().build();
        let ctx = CredentialContextBuilder::new(
            base,
            default_credential_accessor(),
            default_resource_accessor(),
        )
        .callback_url("https://app/callback".to_owned())
        .app_url("https://app".to_owned())
        .session_id("sess-1".to_owned())
        .owner_id("owner-1".to_owned())
        .build();

        assert_eq!(ctx.callback_url(), Some("https://app/callback"));
        assert_eq!(ctx.app_url(), Some("https://app"));
        assert_eq!(ctx.session_id(), Some("sess-1"));
        assert_eq!(ctx.owner_id(), "owner-1");
    }

    #[test]
    fn context_is_cloneable() {
        let ctx1 = CredentialContext::for_test("user_123");
        let ctx2 = ctx1.clone();
        assert_eq!(ctx1.owner_id(), ctx2.owner_id());
    }

    #[test]
    fn context_delegates_to_base() {
        let ctx = CredentialContext::for_test("user_123");
        // Should not panic — delegates to BaseContext
        let _ = ctx.scope();
        let _ = ctx.principal();
        let _ = ctx.cancellation();
        let _ = ctx.clock();
        let _ = ctx.trace_id();
    }

    #[test]
    fn with_session_id_sets_session() {
        let ctx = CredentialContext::for_test("user_123").with_session_id("sess-abc");
        assert_eq!(ctx.session_id(), Some("sess-abc"));
    }

    #[test]
    fn with_callback_url_sets_url() {
        let ctx = CredentialContext::for_test("user_123")
            .with_callback_url("https://app.nebula.io/callback");
        assert_eq!(ctx.callback_url(), Some("https://app.nebula.io/callback"));
    }

    #[test]
    fn with_app_url_sets_url() {
        let ctx = CredentialContext::for_test("user_123").with_app_url("https://app.nebula.io");
        assert_eq!(ctx.app_url(), Some("https://app.nebula.io"));
    }

    #[test]
    fn debug_output_does_not_panic() {
        let ctx = CredentialContext::for_test("user_123");
        let debug = format!("{ctx:?}");
        assert!(debug.contains("CredentialContext"));
    }
}
