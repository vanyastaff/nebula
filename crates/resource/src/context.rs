//! Flat resource context with cancellation support

use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use nebula_core::{ExecutionId, WorkflowId};

use nebula_telemetry::{NoopRecorder, Recorder};

use crate::credentials::CredentialProvider;
use crate::scope::Scope;

/// Context for resource operations.
///
/// Carries scope, identifiers, cancellation, credentials, and arbitrary metadata.
/// Passed to [`Resource::create`](crate::Resource::create) and other lifecycle operations so
/// implementations can make scope-aware, cancellation-aware decisions.
#[derive(Clone)]
pub struct Context {
    /// The visibility scope for this operation (e.g. Global, Tenant, Workflow).
    pub scope: Scope,
    /// Unique identifier of the current workflow execution.
    pub execution_id: ExecutionId,
    /// Identifier of the workflow definition being executed.
    pub workflow_id: WorkflowId,
    /// Optional tenant identifier for multi-tenancy isolation.
    pub tenant_id: Option<String>,
    /// Cooperative cancellation token — operations should check this
    /// periodically and abort early when cancelled.
    pub cancellation: CancellationToken,
    /// Arbitrary key-value pairs for passing extra context to resource
    /// implementations (e.g. region hints, priority labels).
    pub metadata: HashMap<String, String>,
    /// Optional credential provider for fetching secrets at resource-creation time.
    pub credentials: Option<Arc<dyn CredentialProvider>>,
    /// Recorder for Tier 1/Tier 2 resource usage and call traces. Defaults to [`NoopRecorder`].
    pub recorder: Arc<dyn Recorder>,
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = f.debug_struct("Context");
        s.field("scope", &self.scope)
            .field("execution_id", &self.execution_id)
            .field("workflow_id", &self.workflow_id)
            .field("tenant_id", &self.tenant_id)
            .field("cancellation", &self.cancellation)
            .field("metadata", &self.metadata)
            .field("credentials", &self.credentials.is_some())
            .field("recorder", &"Arc<dyn Recorder>");
        s.finish()
    }
}

impl Context {
    /// Create a new context with the given scope, workflow ID, and execution ID.
    pub fn new(scope: Scope, workflow_id: WorkflowId, execution_id: ExecutionId) -> Self {
        Self {
            scope,
            execution_id,
            workflow_id,
            tenant_id: None,
            cancellation: CancellationToken::new(),
            metadata: HashMap::new(),
            credentials: None,
            recorder: Arc::new(NoopRecorder),
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
    pub fn with_credentials(mut self, provider: Arc<dyn CredentialProvider>) -> Self {
        self.credentials = Some(provider);
        self
    }

    /// Set the recorder for resource usage and optional call enrichment.
    pub fn with_recorder(mut self, recorder: Arc<dyn Recorder>) -> Self {
        self.recorder = recorder;
        self
    }

    /// Get a reference to the credential provider, if attached.
    ///
    /// Resource implementations can use this to fetch secrets during
    /// [`Resource::create`](crate::Resource::create):
    ///
    /// ```ignore
    /// if let Some(creds) = ctx.credentials() {
    ///     let password = creds.get("db_password").await?;
    ///     // use `password.expose()` to access the underlying value
    /// }
    /// ```
    pub fn credentials(&self) -> Option<&dyn CredentialProvider> {
        self.credentials.as_deref()
    }

    /// Get the recorder for resource usage and call traces.
    #[must_use]
    pub fn recorder(&self) -> Arc<dyn Recorder> {
        Arc::clone(&self.recorder)
    }
}

#[cfg(test)]
mod tests {
    use nebula_core::{ExecutionId, WorkflowId};

    use super::*;

    #[test]
    fn test_context_creation() {
        let wf = WorkflowId::new();
        let ex = ExecutionId::new();
        let ctx = Context::new(Scope::Global, wf, ex);
        assert_eq!(ctx.workflow_id, wf);
        assert_eq!(ctx.execution_id, ex);
        assert!(ctx.tenant_id.is_none());
        assert!(ctx.metadata.is_empty());
    }

    #[test]
    fn test_context_with_tenant() {
        let ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
            .with_tenant("tenant-a");
        assert_eq!(ctx.tenant_id.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn test_context_with_metadata() {
        let ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
            .with_metadata("env", "prod")
            .with_metadata("region", "us-east-1");
        assert_eq!(ctx.metadata.get("env").unwrap(), "prod");
        assert_eq!(ctx.metadata.get("region").unwrap(), "us-east-1");
    }

    #[test]
    fn test_context_with_cancellation() {
        let token = CancellationToken::new();
        let child = token.child_token();
        let ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
            .with_cancellation(child);
        assert!(!ctx.cancellation.is_cancelled());
        token.cancel();
        assert!(ctx.cancellation.is_cancelled());
    }

    #[test]
    fn test_context_with_credentials() {
        use crate::credentials::{CredentialProvider, SecureString};
        use crate::error::Error;

        struct DummyProvider;

        impl CredentialProvider for DummyProvider {
            fn get(
                &self,
                _key: &str,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<SecureString, Error>> + Send + '_>,
            > {
                Box::pin(async { Ok(SecureString::new("secret")) })
            }
        }

        let provider = Arc::new(DummyProvider);
        let ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
            .with_credentials(provider);

        assert!(ctx.credentials().is_some());
    }

    #[test]
    fn test_context_with_recorder() {
        use nebula_telemetry::NoopRecorder;

        let recorder: Arc<dyn nebula_telemetry::Recorder> = Arc::new(NoopRecorder);
        let ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
            .with_recorder(Arc::clone(&recorder));
        assert!(!ctx.recorder().is_enrichment_enabled());
    }
}
