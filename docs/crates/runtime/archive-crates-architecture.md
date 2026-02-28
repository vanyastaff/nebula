# Archived From "docs/archive/crates-architecture.md"

## 4. nebula-runtime

**Purpose**: Workflow execution engine and scheduling.

```rust
// nebula-runtime/src/lib.rs
pub mod engine;
pub mod executor;
pub mod scheduler;
pub mod context;

// nebula-runtime/src/engine.rs
pub struct WorkflowEngine {
    scheduler: Arc<Scheduler>,
    executor: Arc<Executor>,
    state_manager: Arc<StateManager>,
    resource_pool: Arc<ResourcePool>,
}

impl WorkflowEngine {
    pub async fn new(config: EngineConfig) -> Result<Self, Error> {
        // Initialize components
    }
    
    pub async fn deploy_workflow(&self, workflow: Workflow) -> Result<WorkflowId, Error> {
        // Validate and deploy workflow
    }
    
    pub async fn execute_workflow(
        &self,
        workflow_id: &WorkflowId,
        input: WorkflowDataItem,
    ) -> Result<ExecutionHandle, Error> {
        // Create execution and submit to scheduler
    }
}

// nebula-runtime/src/executor.rs
pub struct Executor {
    registry: Arc<Registry>,
    worker_pool: Arc<WorkerPool>,
}

impl Executor {
    pub async fn execute_node(
        &self,
        node: &Node,
        context: &mut ExecutionContext,
    ) -> Result<ActionResult, Error> {
        let action = self.registry.get_action(&node.action_type)?;
        
        // Prepare input
        let input = self.prepare_input(node, context).await?;
        
        // Execute action
        let result = action.execute(input, context).await?;
        
        // Handle result
        self.handle_result(node, result, context).await
    }
}

// nebula-runtime/src/context.rs
pub struct ExecutionContext {
    pub execution_id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub variables: Variables,
    pub node_outputs: HashMap<NodeId, WorkflowDataItem>,
    pub resources: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    pub supplied_instances: HashMap<String, Arc<dyn Any + Send + Sync>>,
}

impl ExecutionContext {
    pub fn get_resource<T: Resource>(&self) -> Option<Arc<T>> {
        self.resources
            .get(&TypeId::of::<T>())
            .and_then(|r| r.clone().downcast::<T>().ok())
    }
    
    pub fn get_supplied_instance<T: 'static>(&self, key: &str) -> Option<Arc<T>> {
        self.supplied_instances
            .get(key)
            .and_then(|r| r.clone().downcast::<T>().ok())
    }
}
```

