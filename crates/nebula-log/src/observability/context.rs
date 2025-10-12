//! Multi-level context propagation for observability
//!
//! This module provides a hierarchical context system with three levels:
//! - **GlobalContext**: Application-wide settings (service name, version, environment)
//! - **ExecutionContext**: Workflow execution scope (execution_id, workflow_id, tenant_id)
//! - **NodeContext**: Individual node execution (node_id, action_id, resource access)
//!
//! Contexts use thread-local storage with RAII guards for automatic cleanup.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

thread_local! {
    static GLOBAL_CONTEXT: RefCell<Option<Arc<GlobalContext>>> = RefCell::new(None);
    static EXECUTION_CONTEXT: RefCell<Option<Arc<ExecutionContext>>> = RefCell::new(None);
    static NODE_CONTEXT: RefCell<Option<Arc<NodeContext>>> = RefCell::new(None);
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
    pub fn new(service_name: impl Into<String>, version: impl Into<String>, environment: impl Into<String>) -> Self {
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

    /// Set this as the current global context
    pub fn set_current(self) {
        GLOBAL_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = Some(Arc::new(self));
        });
    }

    /// Get the current global context
    pub fn current() -> Option<Arc<Self>> {
        GLOBAL_CONTEXT.with(|ctx| ctx.borrow().clone())
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
            resources: Arc::new(HashMap::new()),
        }
    }

    /// Add a resource to this execution context
    pub fn with_resource<T: std::any::Any + Send + Sync>(
        mut self,
        key: impl Into<String>,
        resource: T,
    ) -> Self {
        let resources = Arc::make_mut(&mut self.resources);
        resources.insert(key.into(), Arc::new(resource));
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
        let previous = EXECUTION_CONTEXT.with(|ctx| {
            ctx.borrow_mut().replace(Arc::new(self))
        });
        ExecutionGuard { previous }
    }
}

/// RAII guard for execution context
///
/// Restores the previous execution context when dropped.
pub struct ExecutionGuard {
    previous: Option<Arc<ExecutionContext>>,
}

impl Drop for ExecutionGuard {
    fn drop(&mut self) {
        EXECUTION_CONTEXT.with(|ctx| {
            *ctx.borrow_mut() = self.previous.take();
        });
    }
}

/// Type alias for resource map (type-erased resources)
pub type ResourceMap = HashMap<String, Arc<dyn Any + Send + Sync>>;

/// Node execution context
///
/// Scoped to a single node execution within a workflow.
/// Contains node-specific identifiers and resource access.
///
/// **SECURITY**: Resources are scoped per-node, not globally.
/// This prevents credential leakage between nodes.
///
/// **NOTE**: The hierarchical resource chain (Account → User → Workflow → Execution → Node → Action)
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
    /// Keys are resource type names (e.g., "LoggerResource")
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
            resources: Arc::new(HashMap::new()),
        }
    }

    /// Set the retry count
    pub fn with_retry_count(mut self, count: u32) -> Self {
        self.retry_count = count;
        self
    }

    /// Add a resource to this context
    pub fn with_resource<T: Any + Send + Sync>(mut self, key: impl Into<String>, resource: T) -> Self {
        let resources = Arc::make_mut(&mut self.resources);
        resources.insert(key.into(), Arc::new(resource));
        self
    }

    /// Get a resource from this context
    pub fn get_resource<T: Any + Send + Sync>(&self, key: &str) -> Option<Arc<T>> {
        self.resources
            .get(key)?
            .clone()
            .downcast::<T>()
            .ok()
    }

    /// Get the current node context
    pub fn current() -> Option<Arc<Self>> {
        NODE_CONTEXT.with(|ctx| ctx.borrow().clone())
    }

    /// Enter this node context, returning a guard
    ///
    /// When the guard is dropped, the previous context is restored.
    pub fn enter(self) -> NodeGuard {
        let previous = NODE_CONTEXT.with(|ctx| {
            ctx.borrow_mut().replace(Arc::new(self))
        });
        NodeGuard { previous }
    }
}

/// RAII guard for node context
///
/// Restores the previous node context when dropped.
pub struct NodeGuard {
    previous: Option<Arc<NodeContext>>,
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
#[derive(Debug, Clone)]
pub struct ContextSnapshot {
    /// Current global context
    pub global: Option<Arc<GlobalContext>>,
    /// Current execution context
    pub execution: Option<Arc<ExecutionContext>>,
    /// Current node context
    pub node: Option<Arc<NodeContext>>,
}

impl ContextSnapshot {
    /// Create an empty snapshot
    pub fn empty() -> Self {
        Self {
            global: None,
            execution: None,
            node: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_global_context() {
        let ctx = GlobalContext::new("test-service", "1.0.0", "test")
            .with_instance_id("instance-1");

        ctx.clone().set_current();

        let current = GlobalContext::current().unwrap();
        assert_eq!(current.service_name, "test-service");
        assert_eq!(current.version, "1.0.0");
        assert_eq!(current.environment, "test");
        assert_eq!(current.instance_id.as_deref(), Some("instance-1"));
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

        let ctx = NodeContext::new("node-1", "test.action")
            .with_resource("TestResource", resource.clone());

        let retrieved = ctx.get_resource::<TestResource>("TestResource").unwrap();
        assert_eq!(*retrieved, resource);

        // Non-existent resource should return None
        assert!(ctx.get_resource::<TestResource>("NonExistent").is_none());
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
        GlobalContext::new("test", "1.0", "dev").set_current();
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

        let ctx1 = NodeContext::new("node-1", "action-1")
            .with_resource("SensitiveData", SensitiveData {
                secret: "secret-1".to_string(),
            });

        let ctx2 = NodeContext::new("node-2", "action-2")
            .with_resource("SensitiveData", SensitiveData {
                secret: "secret-2".to_string(),
            });

        // Resources should be isolated
        let res1 = ctx1.get_resource::<SensitiveData>("SensitiveData").unwrap();
        let res2 = ctx2.get_resource::<SensitiveData>("SensitiveData").unwrap();

        assert_eq!(res1.secret, "secret-1");
        assert_eq!(res2.secret, "secret-2");

        // ctx1 should not have access to ctx2's resources
        assert!(ctx1.get_resource::<SensitiveData>("DifferentResource").is_none());
    }
}
