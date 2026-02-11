//! Multi-level context propagation for observability
//!
//! This module provides a hierarchical context system with three levels:
//! - **GlobalContext**: Application-wide settings (service name, version, environment)
//! - **ExecutionContext**: Workflow execution scope (execution_id, workflow_id, tenant_id)
//! - **NodeContext**: Individual node execution (node_id, action_id, resource access)
//!
//! Contexts use thread-local storage with RAII guards for automatic cleanup.
//!
//! # Thread-Local Storage Warning
//!
//! All contexts are stored in **thread-local** storage. They are **not** propagated
//! across `.await` points in async runtimes with work-stealing schedulers (e.g.,
//! Tokio multi-thread). If a task is suspended and resumes on a different OS thread,
//! the context will be lost.
//!
//! For async context propagation, use `tracing::Span` fields or `tokio::task_local!`.
//! The thread-local approach works correctly for:
//! - Synchronous code
//! - `tokio::runtime::Builder::new_current_thread()` (single-thread runtime)
//! - Code within a single `.await`-free scope

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

thread_local! {
    static GLOBAL_CONTEXT: RefCell<Option<Arc<GlobalContext>>> = const { RefCell::new(None) };
    static EXECUTION_CONTEXT: RefCell<Option<Arc<ExecutionContext>>> = const { RefCell::new(None) };
    static NODE_CONTEXT: RefCell<Option<Arc<NodeContext>>> = const { RefCell::new(None) };
}

/// Global application context
///
/// Contains application-wide configuration that remains constant
/// during the application lifecycle.
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

    /// Set this as the current global context, returning a guard
    ///
    /// When the guard is dropped, the previous global context is restored.
    pub fn set_current(self) -> GlobalGuard {
        let previous = GLOBAL_CONTEXT.with(|ctx| ctx.borrow_mut().replace(Arc::new(self)));
        GlobalGuard {
            previous,
            _not_send: PhantomData,
        }
    }

    /// Get the current global context
    pub fn current() -> Option<Arc<Self>> {
        GLOBAL_CONTEXT.with(|ctx| ctx.borrow().clone())
    }
}

/// RAII guard for global context
///
/// Restores the previous global context when dropped.
/// This guard is `!Send` — it must not cross thread boundaries.
#[derive(Debug)]
pub struct GlobalGuard {
    previous: Option<Arc<GlobalContext>>,
    _not_send: PhantomData<*const ()>,
}

impl Drop for GlobalGuard {
    fn drop(&mut self) {
        GLOBAL_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = self.previous.take();
        });
    }
}

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
    pub fn current() -> Option<Arc<Self>> {
        EXECUTION_CONTEXT.with(|ctx| ctx.borrow().clone())
    }

    /// Enter this execution context, returning a guard
    ///
    /// When the guard is dropped, the previous context is restored.
    pub fn enter(self) -> ExecutionGuard {
        let previous = EXECUTION_CONTEXT.with(|ctx| ctx.borrow_mut().replace(Arc::new(self)));
        ExecutionGuard {
            previous,
            _not_send: PhantomData,
        }
    }
}

/// RAII guard for execution context
///
/// Restores the previous execution context when dropped.
/// This guard is `!Send` — it must not cross thread boundaries.
#[derive(Debug)]
pub struct ExecutionGuard {
    previous: Option<Arc<ExecutionContext>>,
    _not_send: PhantomData<*const ()>,
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        EXECUTION_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = self.previous.take();
        });
    }
}

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
    pub fn current() -> Option<Arc<Self>> {
        NODE_CONTEXT.with(|ctx| ctx.borrow().clone())
    }

    /// Enter this node context, returning a guard
    ///
    /// When the guard is dropped, the previous context is restored.
    pub fn enter(self) -> NodeGuard {
        let previous = NODE_CONTEXT.with(|ctx| ctx.borrow_mut().replace(Arc::new(self)));
        NodeGuard {
            previous,
            _not_send: PhantomData,
        }
    }
}

/// RAII guard for node context
///
/// Restores the previous node context when dropped.
/// This guard is `!Send` — it must not cross thread boundaries.
#[derive(Debug)]
pub struct NodeGuard {
    previous: Option<Arc<NodeContext>>,
    _not_send: PhantomData<*const ()>,
}

impl Drop for NodeGuard {
    fn drop(&mut self) {
        NODE_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = self.previous.take();
        });
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_context() {
        let ctx =
            GlobalContext::new("test-service", "1.0.0", "test").with_instance_id("instance-1");

        let _guard = ctx.clone().set_current();

        let current = GlobalContext::current().unwrap();
        assert_eq!(current.service_name, "test-service");
        assert_eq!(current.version, "1.0.0");
        assert_eq!(current.environment, "test");
        assert_eq!(current.instance_id.as_deref(), Some("instance-1"));
    }

    #[test]
    fn test_global_context_guard_restores() {
        // Set initial context
        let _guard1 = GlobalContext::new("service-1", "1.0", "prod").set_current();
        assert_eq!(GlobalContext::current().unwrap().service_name, "service-1");

        {
            let _guard2 = GlobalContext::new("service-2", "2.0", "staging").set_current();
            assert_eq!(GlobalContext::current().unwrap().service_name, "service-2");
        }

        // After guard2 drops, service-1 should be restored
        assert_eq!(GlobalContext::current().unwrap().service_name, "service-1");
    }

    #[test]
    fn test_execution_context_guard() {
        let ctx1 = ExecutionContext::new("exec-1", "wf-1", "tenant-1");
        let ctx2 = ExecutionContext::new("exec-2", "wf-2", "tenant-2");

        {
            let _guard1 = ctx1.enter();
            let current = ExecutionContext::current().unwrap();
            assert_eq!(current.execution_id, "exec-1");

            {
                let _guard2 = ctx2.enter();
                let current = ExecutionContext::current().unwrap();
                assert_eq!(current.execution_id, "exec-2");
            }

            // After guard2 drops, ctx1 should be restored
            let current = ExecutionContext::current().unwrap();
            assert_eq!(current.execution_id, "exec-1");
        }

        // After guard1 drops, no context should be set
        assert!(ExecutionContext::current().is_none());
    }

    #[test]
    fn test_node_context_resources() {
        #[derive(Debug, Clone, PartialEq)]
        struct TestResource {
            value: String,
        }

        let resource = TestResource {
            value: "test-value".to_string(),
        };

        let ctx = NodeContext::new("node-1", "test.action").with_resource(resource.clone());

        let retrieved = ctx.get_resource::<TestResource>().unwrap();
        assert_eq!(*retrieved, resource);

        // Non-existent resource should return None
        assert!(ctx.get_resource::<String>().is_none());
    }

    #[test]
    fn test_node_context_guard() {
        let ctx1 = NodeContext::new("node-1", "action-1");
        let ctx2 = NodeContext::new("node-2", "action-2");

        {
            let _guard1 = ctx1.enter();
            let current = NodeContext::current().unwrap();
            assert_eq!(current.node_id, "node-1");

            {
                let _guard2 = ctx2.enter();
                let current = NodeContext::current().unwrap();
                assert_eq!(current.node_id, "node-2");
            }

            let current = NodeContext::current().unwrap();
            assert_eq!(current.node_id, "node-1");
        }

        assert!(NodeContext::current().is_none());
    }

    #[test]
    fn test_context_snapshot() {
        let _global_guard = GlobalContext::new("test", "1.0", "dev").set_current();
        let exec_ctx = ExecutionContext::new("exec-1", "wf-1", "tenant-1");
        let node_ctx = NodeContext::new("node-1", "action-1");

        let _exec_guard = exec_ctx.enter();
        let _node_guard = node_ctx.enter();

        let snapshot = current_contexts();
        assert!(snapshot.global.is_some());
        assert!(snapshot.execution.is_some());
        assert!(snapshot.node.is_some());

        assert_eq!(snapshot.global.unwrap().service_name, "test");
        assert_eq!(snapshot.execution.unwrap().execution_id, "exec-1");
        assert_eq!(snapshot.node.unwrap().node_id, "node-1");
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

        // Resources should be isolated
        let res1 = ctx1.get_resource::<SensitiveData>().unwrap();
        let res2 = ctx2.get_resource::<SensitiveData>().unwrap();

        assert_eq!(res1.secret, "secret-1");
        assert_eq!(res2.secret, "secret-2");

        // Different type should return None
        assert!(ctx1.get_resource::<String>().is_none());
    }
}
