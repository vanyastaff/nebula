//! Flat resource context with cancellation support

use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "credentials")]
use std::sync::Arc;

#[cfg(feature = "credentials")]
use crate::credentials::CredentialProvider;
use crate::scope::Scope;

/// Context for resource operations.
///
/// Carries scope, identifiers, cancellation, and arbitrary metadata.
/// Passed to [`Resource::create`](crate::Resource::create) and other lifecycle operations so
/// implementations can make scope-aware, cancellation-aware decisions.
#[derive(Clone)]
pub struct Context {
    /// The visibility scope for this operation (e.g. Global, Tenant, Workflow).
    pub scope: Scope,
    /// Unique identifier of the current workflow execution.
    pub execution_id: String,
    /// Identifier of the workflow definition being executed.
    pub workflow_id: String,
    /// Optional tenant identifier for multi-tenancy isolation.
    pub tenant_id: Option<String>,
    /// Cooperative cancellation token — operations should check this
    /// periodically and abort early when cancelled.
    pub cancellation: CancellationToken,
    /// Arbitrary key-value pairs for passing extra context to resource
    /// implementations (e.g. region hints, priority labels).
    pub metadata: HashMap<String, String>,
    /// Optional credential provider for fetching secrets at resource-creation
    /// time. Gated behind the `credentials` feature.
    #[cfg(feature = "credentials")]
    pub credentials: Option<Arc<dyn CredentialProvider>>,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("Context");
        s.field("scope", &self.scope)
            .field("execution_id", &self.execution_id)
            .field("workflow_id", &self.workflow_id)
            .field("tenant_id", &self.tenant_id)
            .field("cancellation", &self.cancellation)
            .field("metadata", &self.metadata);
        #[cfg(feature = "credentials")]
        s.field("credentials", &self.credentials.is_some());
        s.finish()
    }
}

impl Context {
    /// Create a new context with the given scope, workflow ID, and execution ID.
    pub fn new(
        scope: Scope,
        workflow_id: impl Into<String>,
        execution_id: impl Into<String>,
    ) -> Self {
        Self {
            scope,
            execution_id: execution_id.into(),
            workflow_id: workflow_id.into(),
            tenant_id: None,
            cancellation: CancellationToken::new(),
            metadata: HashMap::new(),
            #[cfg(feature = "credentials")]
            credentials: None,
        }
    }

    /// Set the tenant ID for multi-tenancy isolation.
    pub fn with_tenant(mut self, tenant_id: impl Into<String>) -> Self {
        self.tenant_id = Some(tenant_id.into());
        self
    }

    /// Add a key-value metadata pair to the context.
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Replace the default cancellation token with the provided one.
    pub fn with_cancellation(mut self, token: CancellationToken) -> Self {
        self.cancellation = token;
        self
    }

    /// Attach a credential provider to this context.
    #[cfg(feature = "credentials")]
    pub fn with_credentials(mut self, provider: Arc<dyn CredentialProvider>) -> Self {
        self.credentials = Some(provider);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_creation() {
        let ctx = Context::new(Scope::Global, "wf-1", "ex-1");
        assert_eq!(ctx.workflow_id, "wf-1");
        assert_eq!(ctx.execution_id, "ex-1");
        assert!(ctx.tenant_id.is_none());
        assert!(ctx.metadata.is_empty());
    }

    #[test]
    fn test_context_with_tenant() {
        let ctx = Context::new(Scope::Global, "wf-1", "ex-1").with_tenant("tenant-a");
        assert_eq!(ctx.tenant_id.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn test_context_with_metadata() {
        let ctx = Context::new(Scope::Global, "wf-1", "ex-1")
            .with_metadata("env", "prod")
            .with_metadata("region", "us-east-1");
        assert_eq!(ctx.metadata.get("env").unwrap(), "prod");
        assert_eq!(ctx.metadata.get("region").unwrap(), "us-east-1");
    }

    #[test]
    fn test_context_with_cancellation() {
        let token = CancellationToken::new();
        let child = token.child_token();
        let ctx = Context::new(Scope::Global, "wf-1", "ex-1").with_cancellation(child);
        assert!(!ctx.cancellation.is_cancelled());
        token.cancel();
        assert!(ctx.cancellation.is_cancelled());
    }
}
