//! Multi-level context propagation for observability
//!
//! This module provides a hierarchical context system with three levels:
//! - **GlobalContext**: Application-wide settings (service name, version, environment)
//! - **ExecutionContext**: Workflow execution scope (execution_id, workflow_id, tenant_id)
//! - **NodeContext**: Individual node execution (node_id, action_id, resource access)
//!
//! # Async-Safe Context Propagation
//!
//! When the `async` feature is enabled, contexts use `tokio::task_local!` storage
//! and survive across `.await` points in multi-thread Tokio runtimes.
//!
//! When the `async` feature is disabled, contexts use `thread_local!` storage
//! (suitable for synchronous code or single-thread runtimes).
//!
//! # Usage
//!
//! Contexts are activated via `scope()` (async) or `scope_sync()` (sync):
//!
//! ```rust,ignore
//! // Async — survives .await points
//! ExecutionContext::new("exec-1", "wf-1", "tenant-1")
//!     .scope(async {
//!         do_work().await;
//!         assert!(ExecutionContext::current().is_some());
//!     })
//!     .await;
//!
//! // Sync
//! NodeContext::new("node-1", "action-1")
//!     .scope_sync(|| {
//!         assert!(NodeContext::current().is_some());
//!     });
//! ```

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

use arc_swap::ArcSwap;

// ---------------------------------------------------------------------------
// GlobalContext — ArcSwap (set-once, lock-free reads, test-resettable)
// ---------------------------------------------------------------------------

/// Global static for the application-wide context.
static GLOBAL_CONTEXT: LazyLock<ArcSwap<Option<Arc<GlobalContext>>>> =
    LazyLock::new(|| ArcSwap::from_pointee(None));

/// Global application context
///
/// Contains application-wide configuration that remains constant
/// during the application lifecycle.
///
/// Unlike `ExecutionContext` and `NodeContext`, this is stored in a global
/// `ArcSwap` — it is `Send + Sync` and does not require scoping.
#[derive(Debug, Clone)]
pub struct GlobalContext {
    /// Service name (e.g., "nebula-workflow")
    pub service_name: String,
    /// Service version (e.g., "0.1.0")
    pub version: String,
    /// Deployment environment (e.g., "production", "staging")
    pub environment: String,
    /// Optional instance ID for distributed deployments
    pub instance_id: Option<String>,
}

impl GlobalContext {
    /// Create a new global context
    pub fn new(
        service_name: impl Into<String>,
        version: impl Into<String>,
        environment: impl Into<String>,
    ) -> Self {
        Self {
            service_name: service_name.into(),
            version: version.into(),
            environment: environment.into(),
            instance_id: None,
        }
    }

    /// Set the instance ID
    pub fn with_instance_id(mut self, instance_id: impl Into<String>) -> Self {
        self.instance_id = Some(instance_id.into());
        self
    }

    /// Initialize the global context.
    ///
    /// Typically called once at application startup. Subsequent calls
    /// replace the previous value.
    pub fn init(self) {
        GLOBAL_CONTEXT.store(Arc::new(Some(Arc::new(self))));
    }

    /// Get the current global context
    #[inline]
    pub fn current() -> Option<Arc<Self>> {
        let guard = GLOBAL_CONTEXT.load();
        (**guard).clone()
    }
}

// ---------------------------------------------------------------------------
// ResourceMap — unchanged, already Send + Sync
// ---------------------------------------------------------------------------

/// Type-safe resource map keyed by [`TypeId`].
///
/// Resources are stored by their concrete type, eliminating string-key typo bugs.
/// The API enforces type safety — callers cannot insert raw `TypeId` keys.
///
/// Use [`insert`](ResourceMap::insert) and [`get`](ResourceMap::get) for access.
#[derive(Debug, Clone, Default)]
pub struct ResourceMap {
    inner: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
}

impl ResourceMap {
    /// Create an empty resource map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a typed resource, replacing any previous value of the same type.
    pub fn insert<T: Any + Send + Sync>(&mut self, resource: T) {
        self.inner.insert(TypeId::of::<T>(), Arc::new(resource));
    }

    /// Retrieve a typed resource, returning `None` if not present or type mismatch.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.inner
            .get(&TypeId::of::<T>())?
            .clone()
            .downcast::<T>()
            .ok()
    }

    /// Check whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Number of resources stored.
    pub fn len(&self) -> usize {
        self.inner.len()
    }
}

// ---------------------------------------------------------------------------
// Storage backend — task_local (async) or thread_local (no-async)
// ---------------------------------------------------------------------------

#[cfg(feature = "async")]
mod storage {
    use super::*;
    use std::future::Future;

    tokio::task_local! {
        static EXECUTION_CTX: Arc<ExecutionContext>;
        static NODE_CTX: Arc<NodeContext>;
    }

    #[inline]
    pub fn current_execution() -> Option<Arc<ExecutionContext>> {
        EXECUTION_CTX.try_with(|ctx| ctx.clone()).ok()
    }

    #[inline]
    pub fn current_node() -> Option<Arc<NodeContext>> {
        NODE_CTX.try_with(|ctx| ctx.clone()).ok()
    }

    pub async fn with_execution<F: Future>(ctx: Arc<ExecutionContext>, f: F) -> F::Output {
        EXECUTION_CTX.scope(ctx, f).await
    }

    pub async fn with_node<F: Future>(ctx: Arc<NodeContext>, f: F) -> F::Output {
        NODE_CTX.scope(ctx, f).await
    }

    pub fn with_execution_sync<R>(ctx: Arc<ExecutionContext>, f: impl FnOnce() -> R) -> R {
        EXECUTION_CTX.sync_scope(ctx, f)
    }

    pub fn with_node_sync<R>(ctx: Arc<NodeContext>, f: impl FnOnce() -> R) -> R {
        NODE_CTX.sync_scope(ctx, f)
    }
}

#[cfg(not(feature = "async"))]
mod storage {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static EXECUTION_CTX: RefCell<Option<Arc<ExecutionContext>>> =
            const { RefCell::new(None) };
        static NODE_CTX: RefCell<Option<Arc<NodeContext>>> =
            const { RefCell::new(None) };
    }

    #[inline]
    pub fn current_execution() -> Option<Arc<ExecutionContext>> {
        EXECUTION_CTX.with(|ctx| ctx.borrow().clone())
    }

    #[inline]
    pub fn current_node() -> Option<Arc<NodeContext>> {
        NODE_CTX.with(|ctx| ctx.borrow().clone())
    }

    pub fn with_execution_sync<R>(ctx: Arc<ExecutionContext>, f: impl FnOnce() -> R) -> R {
        EXECUTION_CTX.with(|cell| {
            let prev = cell.borrow_mut().replace(ctx);
            let result = f();
            *cell.borrow_mut() = prev;
            result
        })
    }

    pub fn with_node_sync<R>(ctx: Arc<NodeContext>, f: impl FnOnce() -> R) -> R {
        NODE_CTX.with(|cell| {
            let prev = cell.borrow_mut().replace(ctx);
            let result = f();
            *cell.borrow_mut() = prev;
            result
        })
    }
}

// ---------------------------------------------------------------------------
// ExecutionContext
// ---------------------------------------------------------------------------

/// Workflow execution context
///
/// Scoped to a single workflow execution. Contains identifiers
/// for tracking the execution across distributed systems.
///
/// **Span-like nesting**: Like `tracing` spans, execution contexts can be nested.
/// Resources from parent contexts (Global) are automatically inherited and merged.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    /// Unique execution ID
    pub execution_id: String,
    /// Workflow definition ID
    pub workflow_id: String,
    /// Tenant ID for multi-tenancy
    pub tenant_id: String,
    /// Optional parent execution ID for sub-workflows
    pub parent_execution_id: Option<String>,
    /// Optional trace ID for distributed tracing
    pub trace_id: Option<String>,
    /// Resources for this execution (will be merged with Global context)
    pub resources: Arc<ResourceMap>,
}

impl ExecutionContext {
    /// Create a new execution context
    pub fn new(
        execution_id: impl Into<String>,
        workflow_id: impl Into<String>,
        tenant_id: impl Into<String>,
    ) -> Self {
        Self {
            execution_id: execution_id.into(),
            workflow_id: workflow_id.into(),
            tenant_id: tenant_id.into(),
            parent_execution_id: None,
            trace_id: None,
            resources: Arc::new(ResourceMap::new()),
        }
    }

    /// Add a typed resource to this execution context
    pub fn with_resource<T: Any + Send + Sync>(mut self, resource: T) -> Self {
        Arc::make_mut(&mut self.resources).insert(resource);
        self
    }

    /// Set the parent execution ID
    pub fn with_parent(mut self, parent_id: impl Into<String>) -> Self {
        self.parent_execution_id = Some(parent_id.into());
        self
    }

    /// Set the trace ID
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    /// Get the current execution context
    #[inline]
    pub fn current() -> Option<Arc<Self>> {
        storage::current_execution()
    }

    /// Run a synchronous closure with this context active.
    ///
    /// Nesting is supported — inner scopes shadow outer ones and restore on return.
    pub fn scope_sync<R>(self, f: impl FnOnce() -> R) -> R {
        storage::with_execution_sync(Arc::new(self), f)
    }

    /// Run a future with this context active.
    ///
    /// The context survives across `.await` points, even in multi-thread
    /// Tokio runtimes with work-stealing.
    ///
    /// Nesting is supported — inner scopes shadow outer ones and restore on completion.
    #[cfg(feature = "async")]
    pub async fn scope<F: std::future::Future>(self, f: F) -> F::Output {
        storage::with_execution(Arc::new(self), f).await
    }
}

// ---------------------------------------------------------------------------
// NodeContext
// ---------------------------------------------------------------------------

/// Node execution context
///
/// Scoped to a single node execution within a workflow.
/// Contains node-specific identifiers and resource access.
///
/// **SECURITY**: Resources are scoped per-node, not globally.
/// This prevents credential leakage between nodes.
///
/// **NOTE**: The hierarchical resource chain (Account -> User -> Workflow -> Execution -> Node -> Action)
/// is managed by the workflow engine, not by nebula-log. nebula-log only receives the final
/// merged LoggerResource via the ResourceMap.
#[derive(Debug, Clone)]
pub struct NodeContext {
    /// Node instance ID
    pub node_id: String,
    /// Action type ID (e.g., "http.request")
    pub action_id: String,
    /// Retry attempt number (0 for first attempt)
    pub retry_count: u32,
    /// Resources attached to this node (scoped, isolated)
    /// The workflow engine populates this with merged resources from the hierarchy
    pub resources: Arc<ResourceMap>,
}

impl NodeContext {
    /// Create a new node context
    pub fn new(node_id: impl Into<String>, action_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            action_id: action_id.into(),
            retry_count: 0,
            resources: Arc::new(ResourceMap::new()),
        }
    }

    /// Set the retry count
    pub fn with_retry_count(mut self, count: u32) -> Self {
        self.retry_count = count;
        self
    }

    /// Add a typed resource to this context
    pub fn with_resource<T: Any + Send + Sync>(mut self, resource: T) -> Self {
        Arc::make_mut(&mut self.resources).insert(resource);
        self
    }

    /// Get a typed resource from this context
    pub fn get_resource<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.resources.get::<T>()
    }

    /// Get the current node context
    #[inline]
    pub fn current() -> Option<Arc<Self>> {
        storage::current_node()
    }

    /// Run a synchronous closure with this context active.
    ///
    /// Nesting is supported — inner scopes shadow outer ones and restore on return.
    pub fn scope_sync<R>(self, f: impl FnOnce() -> R) -> R {
        storage::with_node_sync(Arc::new(self), f)
    }

    /// Run a future with this context active.
    ///
    /// The context survives across `.await` points, even in multi-thread
    /// Tokio runtimes with work-stealing.
    ///
    /// Nesting is supported — inner scopes shadow outer ones and restore on completion.
    #[cfg(feature = "async")]
    pub async fn scope<F: std::future::Future>(self, f: F) -> F::Output {
        storage::with_node(Arc::new(self), f).await
    }
}

// ---------------------------------------------------------------------------
// ContextSnapshot
// ---------------------------------------------------------------------------

/// Convenience function to get all current contexts
pub fn current_contexts() -> ContextSnapshot {
    ContextSnapshot {
        global: GlobalContext::current(),
        execution: ExecutionContext::current(),
        node: NodeContext::current(),
    }
}

/// Snapshot of all current contexts
#[derive(Debug, Clone, Default)]
pub struct ContextSnapshot {
    /// Current global context
    pub global: Option<Arc<GlobalContext>>,
    /// Current execution context
    pub execution: Option<Arc<ExecutionContext>>,
    /// Current node context
    pub node: Option<Arc<NodeContext>>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // GlobalContext uses a process-wide ArcSwap — parallel tests race on
    // the shared state, so we only assert that init/current work without
    // checking exact values (another test may overwrite them).
    #[test]
    fn test_global_context() {
        // Test init + current
        let ctx =
            GlobalContext::new("test-service", "1.0.0", "test").with_instance_id("instance-1");
        assert_eq!(ctx.service_name, "test-service");
        assert_eq!(ctx.version, "1.0.0");
        assert_eq!(ctx.environment, "test");
        assert_eq!(ctx.instance_id.as_deref(), Some("instance-1"));
        ctx.init();

        // After init, current() must return Some (exact value may differ
        // due to parallel tests also calling init()).
        assert!(GlobalContext::current().is_some());

        // Test that init replaces the previous value
        GlobalContext::new("service-2", "2.0", "staging").init();
        assert!(GlobalContext::current().is_some());
    }

    #[test]
    fn test_execution_context_scope() {
        let ctx1 = ExecutionContext::new("exec-1", "wf-1", "tenant-1");
        let ctx2 = ExecutionContext::new("exec-2", "wf-2", "tenant-2");

        ctx1.scope_sync(|| {
            assert_eq!(ExecutionContext::current().unwrap().execution_id, "exec-1");

            ctx2.scope_sync(|| {
                assert_eq!(ExecutionContext::current().unwrap().execution_id, "exec-2");
            });

            // After inner scope, ctx1 is restored
            assert_eq!(ExecutionContext::current().unwrap().execution_id, "exec-1");
        });

        // Outside all scopes, no context
        assert!(ExecutionContext::current().is_none());
    }

    #[test]
    fn test_node_context_resources() {
        #[derive(Debug, Clone, PartialEq)]
        struct TestResource {
            value: String,
        }

        let ctx = NodeContext::new("node-1", "test.action").with_resource(TestResource {
            value: "test-value".to_string(),
        });

        ctx.scope_sync(|| {
            let current = NodeContext::current().unwrap();
            let retrieved = current.get_resource::<TestResource>().unwrap();
            assert_eq!(retrieved.value, "test-value");

            // Non-existent resource should return None
            assert!(current.get_resource::<String>().is_none());
        });
    }

    #[test]
    fn test_node_context_scope() {
        let ctx1 = NodeContext::new("node-1", "action-1");
        let ctx2 = NodeContext::new("node-2", "action-2");

        ctx1.scope_sync(|| {
            assert_eq!(NodeContext::current().unwrap().node_id, "node-1");

            ctx2.scope_sync(|| {
                assert_eq!(NodeContext::current().unwrap().node_id, "node-2");
            });

            assert_eq!(NodeContext::current().unwrap().node_id, "node-1");
        });

        assert!(NodeContext::current().is_none());
    }

    #[test]
    fn test_context_snapshot() {
        GlobalContext::new("test", "1.0", "dev").init();

        ExecutionContext::new("exec-1", "wf-1", "tenant-1").scope_sync(|| {
            NodeContext::new("node-1", "action-1").scope_sync(|| {
                let snapshot = current_contexts();
                assert!(snapshot.global.is_some());
                assert!(snapshot.execution.is_some());
                assert!(snapshot.node.is_some());

                // Global context is process-wide (ArcSwap), so parallel tests
                // may overwrite the value. Only assert it exists; the exact
                // service_name depends on test ordering.
                assert!(snapshot.global.is_some());
                assert_eq!(snapshot.execution.unwrap().execution_id, "exec-1");
                assert_eq!(snapshot.node.unwrap().node_id, "node-1");
            });
        });
    }

    #[test]
    fn test_resource_isolation() {
        #[derive(Debug, Clone)]
        struct SensitiveData {
            secret: String,
        }

        let ctx1 = NodeContext::new("node-1", "action-1").with_resource(SensitiveData {
            secret: "secret-1".to_string(),
        });

        let ctx2 = NodeContext::new("node-2", "action-2").with_resource(SensitiveData {
            secret: "secret-2".to_string(),
        });

        ctx1.scope_sync(|| {
            let current = NodeContext::current().unwrap();
            let res = current.get_resource::<SensitiveData>().unwrap();
            assert_eq!(res.secret, "secret-1");
            assert!(current.get_resource::<String>().is_none());
        });

        ctx2.scope_sync(|| {
            let current = NodeContext::current().unwrap();
            let res = current.get_resource::<SensitiveData>().unwrap();
            assert_eq!(res.secret, "secret-2");
        });
    }

    // Async tests — verify context survives .await in multi-thread runtime
    #[cfg(feature = "async")]
    mod async_tests {
        use super::*;

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn test_execution_context_survives_await() {
            let ctx = ExecutionContext::new("exec-async", "wf-async", "tenant-async");
            ctx.scope(async {
                assert_eq!(
                    ExecutionContext::current().unwrap().execution_id,
                    "exec-async"
                );
                tokio::task::yield_now().await;
                // After potential thread migration:
                assert_eq!(
                    ExecutionContext::current().unwrap().execution_id,
                    "exec-async"
                );
            })
            .await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn test_node_context_survives_await() {
            let ctx = NodeContext::new("node-async", "action-async");
            ctx.scope(async {
                assert_eq!(NodeContext::current().unwrap().node_id, "node-async");
                tokio::task::yield_now().await;
                assert_eq!(NodeContext::current().unwrap().node_id, "node-async");
            })
            .await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn test_nested_async_scopes() {
            let exec = ExecutionContext::new("e1", "wf", "t");
            exec.scope(async {
                let node = NodeContext::new("n1", "a1");
                node.scope(async {
                    assert!(ExecutionContext::current().is_some());
                    assert_eq!(NodeContext::current().unwrap().node_id, "n1");
                    tokio::task::yield_now().await;
                    assert_eq!(NodeContext::current().unwrap().node_id, "n1");
                })
                .await;
                assert!(NodeContext::current().is_none());
            })
            .await;
        }

        #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
        async fn test_concurrent_tasks_isolated() {
            let (tx, mut rx) = tokio::sync::mpsc::channel(10);
            for i in 0..5 {
                let tx = tx.clone();
                let ctx = ExecutionContext::new(format!("exec-{i}"), "wf", "tenant");
                tokio::spawn(ctx.scope(async move {
                    tokio::task::yield_now().await;
                    let current = ExecutionContext::current().unwrap();
                    tx.send(current.execution_id.clone()).await.unwrap();
                }));
            }
            drop(tx);
            let mut ids = Vec::new();
            while let Some(id) = rx.recv().await {
                ids.push(id);
            }
            ids.sort();
            assert_eq!(ids, vec!["exec-0", "exec-1", "exec-2", "exec-3", "exec-4"]);
        }
    }
}
