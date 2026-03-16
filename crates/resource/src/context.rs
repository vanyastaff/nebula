//! Flat resource context with cancellation support

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use nebula_core::{ExecutionId, ResourceKey, WorkflowId};

use crate::error::Result;
use crate::guard::Guard;
use crate::resource::Resource;
use crate::scope::Scope;
use nebula_telemetry::{NoopRecorder, Recorder};

/// Context for resource operations.
///
/// Carries scope, identifiers, cancellation, credentials, and arbitrary metadata.
/// Passed to [`Resource::create`] and other lifecycle operations so
/// implementations can make scope-aware, cancellation-aware decisions.
///
/// ## Cancellation
///
/// `cancellation` is `None` for internal/background contexts (pool maintenance,
/// warm-up, health checks). The pool tests this at the start of `acquire()` to
/// skip the `select!` machinery entirely, saving ~100–130 ns on the hot path.
/// User-facing contexts created via [`Context::new`] carry a live token by default;
/// use [`Context::background`] to create an un-cancellable context.
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
    /// Optional cooperative cancellation token.
    ///
    /// `None` means the operation can never be cancelled externally —
    /// the pool skips `select!` overhead entirely on this path.
    /// `Some(token)` enables the standard cancellable acquire flow.
    pub cancellation: Option<CancellationToken>,
    /// Arbitrary key-value pairs for passing extra context to resource
    /// implementations (e.g. region hints, priority labels).
    pub metadata: HashMap<String, String>,
    /// Recorder for Tier 1/Tier 2 resource usage and call traces. Defaults to [`NoopRecorder`].
    pub recorder: Arc<dyn Recorder>,
    /// Sub-resource pool handles injected by the manager before `Resource::create()`.
    /// Keyed by ResourceKey. Typed via `Any` downcast in [`resource`](Self::resource).
    resolved_resources: HashMap<String, Arc<dyn Any + Send + Sync>>,
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
            .field("recorder", &"Arc<dyn Recorder>");
        s.finish()
    }
}

impl Context {
    /// Create a new context with the given scope, workflow ID, and execution ID.
    ///
    /// The context is cancellable from the start (carries a fresh [`CancellationToken`]).
    /// Use [`Context::background`] when cancellation is never needed (internal/pool ops).
    pub fn new(scope: Scope, workflow_id: WorkflowId, execution_id: ExecutionId) -> Self {
        Self {
            scope,
            execution_id,
            workflow_id,
            tenant_id: None,
            cancellation: Some(CancellationToken::new()),
            metadata: HashMap::new(),
            recorder: Arc::new(NoopRecorder),
            resolved_resources: HashMap::new(),
        }
    }

    /// Create an un-cancellable background context.
    ///
    /// Use this for internal pool operations (warm-up, maintenance, scale-up/down)
    /// where no external cancellation is expected. The pool skips the `select!`
    /// overhead entirely, saving ~100–130 ns per acquire.
    #[must_use]
    pub fn background(scope: Scope, workflow_id: WorkflowId, execution_id: ExecutionId) -> Self {
        Self {
            scope,
            execution_id,
            workflow_id,
            tenant_id: None,
            cancellation: None,
            metadata: HashMap::new(),
            recorder: Arc::new(NoopRecorder),
            resolved_resources: HashMap::new(),
        }
    }

    /// Returns `true` when this context carries a live cancellation token.
    #[inline]
    #[must_use]
    pub fn is_cancellable(&self) -> bool {
        self.cancellation.is_some()
    }

    /// Returns `true` if the cancellation token has been cancelled.
    ///
    /// Returns `false` for background (non-cancellable) contexts.
    #[inline]
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }

    /// Inject a resolved sub-resource pool handle (called by manager, not by resource impls).
    ///
    /// The handle must be `Arc<TypedPool<R>>` for the resource type `R` at the given key.
    /// Used when the manager prepares the context before `Resource::create()` for resources
    /// that declare sub-resources via `ResourceDependencies`.
    #[allow(dead_code)] // used in tests; full manager→create() injection wiring in progress
    pub(crate) fn inject_resource(&mut self, key: ResourceKey, handle: Arc<dyn Any + Send + Sync>) {
        self.resolved_resources
            .insert(key.to_string(), handle);
    }

    /// Retrieve a resolved sub-resource pool handle for typed acquisition.
    ///
    /// Returns `None` if not injected (resource not declared in `ResourceDependencies`, or not yet init).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let handle = ctx.resource::<HttpPool>("http-global")
    ///     .expect("http-global declared in components");
    /// let (guard, _) = handle.acquire(ctx).await?;
    /// ```
    #[must_use]
    pub fn resource<R: Resource>(&self, key: &str) -> Option<ResourcePoolHandle<R>>
    where
        R::Instance: Any,
    {
        use crate::manager_pool::TypedPool;

        let handle = self.resolved_resources.get(key)?;
        let typed = handle.clone().downcast::<TypedPool<R>>().ok()?;
        Some(ResourcePoolHandle { inner: typed })
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
        self.cancellation = Some(token);
        self
    }

    /// Set the recorder for resource usage and optional call enrichment.
    pub fn with_recorder(mut self, recorder: Arc<dyn Recorder>) -> Self {
        self.recorder = recorder;
        self
    }

    /// Get the recorder for resource usage and call traces.
    #[must_use]
    pub fn recorder(&self) -> Arc<dyn Recorder> {
        Arc::clone(&self.recorder)
    }
}

/// Handle to a sub-resource pool, injected into [`Context`] before `Resource::create()`.
///
/// Use [`acquire`](Self::acquire) to acquire an instance from the pool.
///
/// # Example
///
/// ```rust,ignore
/// let handle = ctx.resource::<HttpPool>("http-global")?;
/// let (guard, _) = handle.acquire(ctx).await?;
/// let client = guard.get();
/// ```
#[derive(Clone)]
pub struct ResourcePoolHandle<R: Resource> {
    pub(crate) inner: Arc<crate::manager_pool::TypedPool<R>>,
}

impl<R: Resource> ResourcePoolHandle<R>
where
    R::Instance: Any,
{
    /// Acquire an instance from the sub-resource pool.
    pub async fn acquire(
        &self,
        ctx: &Context,
    ) -> Result<(
        Guard<R::Instance, impl FnOnce(R::Instance, bool) + Send + 'static + use<R>>,
        std::time::Duration,
    )> {
        self.inner.pool.acquire(ctx).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nebula_core::{resource_key, ExecutionId, ResourceKey, WorkflowId};

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
        assert!(!ctx.is_cancelled());
        token.cancel();
        assert!(ctx.is_cancelled());
    }

    #[test]
    fn test_context_with_recorder() {
        use nebula_telemetry::NoopRecorder;

        let recorder: Arc<dyn nebula_telemetry::Recorder> = Arc::new(NoopRecorder);
        let ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
            .with_recorder(Arc::clone(&recorder));
        assert!(!ctx.recorder().is_enrichment_enabled());
    }

    #[tokio::test]
    async fn context_resolves_sub_resource_by_type() {
        use std::any::Any;

        use crate::error::Result;
        use crate::manager_pool::TypedPool;
        use crate::pool::{Pool, PoolConfig};
        use crate::resource::{Config, Resource};

        #[derive(Clone)]
        struct TestConfig;

        impl Config for TestConfig {}

        struct TestResource;

        impl Resource for TestResource {
            type Config = TestConfig;
            type Instance = String;

            fn key(&self) -> ResourceKey {
                resource_key!("test-http")
            }

            async fn create(
                &self,
                _config: &Self::Config,
                _ctx: &Context,
            ) -> Result<Self::Instance> {
                Ok("ok".to_string())
            }
        }

        let key = resource_key!("http-global");
        let pool =
            Pool::new(TestResource, TestConfig, PoolConfig::default()).expect("pool creation");
        let typed = Arc::new(TypedPool { pool });
        let handle: Arc<dyn Any + Send + Sync> = typed.clone();

        let mut ctx = Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new());
        ctx.inject_resource(key, handle);

        let retrieved = ctx.resource::<TestResource>("http-global");
        assert!(retrieved.is_some(), "resource handle should be resolved");
    }
}
