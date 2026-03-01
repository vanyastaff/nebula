---

# nebula-engine

## Purpose

`nebula-engine` is the workflow orchestration engine responsible for scheduling, executing, and managing workflow lifecycles. It handles the core execution logic and state management.

## Responsibilities

- Workflow orchestration and scheduling
- Execution state management
- Event processing and routing
- DAG traversal and execution
- Error handling and recovery
- Resource coordination

## Architecture

### Core Components

```rust
pub struct WorkflowEngine {
    // Event processing
    event_bus: Arc<dyn EventBus>,
    
    // State management
    state_manager: Arc<StateManager>,
    
    // Execution scheduling
    scheduler: Arc<Scheduler>,
    
    // DAG processor
    dag_processor: Arc<DagProcessor>,
    
    // Resource coordinator
    resource_coordinator: Arc<ResourceCoordinator>,
    
    // Metrics collector
    metrics: Arc<MetricsCollector>,
}
```

### Event-Driven Architecture

```rust
#[derive(Debug, Clone)]
pub enum EngineEvent {
    // Workflow lifecycle
    WorkflowDeployed { id: WorkflowId, definition: WorkflowDefinition },
    WorkflowActivated { id: WorkflowId },
    WorkflowDeactivated { id: WorkflowId },
    
    // Execution lifecycle
    ExecutionCreated { id: ExecutionId, workflow_id: WorkflowId },
    ExecutionStarted { id: ExecutionId },
    ExecutionCompleted { id: ExecutionId, result: ExecutionResult },
    ExecutionFailed { id: ExecutionId, error: Error },
    ExecutionCancelled { id: ExecutionId },
    
    // Node execution
    NodeReady { execution_id: ExecutionId, node_id: NodeId },
    NodeStarted { execution_id: ExecutionId, node_id: NodeId },
    NodeCompleted { execution_id: ExecutionId, node_id: NodeId, output: Value },
    NodeFailed { execution_id: ExecutionId, node_id: NodeId, error: Error },
    
    // Control flow
    ExecutionSuspended { id: ExecutionId, reason: SuspendReason },
    ExecutionResumed { id: ExecutionId },
}
```

## Execution Flow

### Workflow Deployment

```rust
impl WorkflowEngine {
    pub async fn deploy_workflow(
        &self,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowId, Error> {
        // Validate workflow
        self.validate_workflow(&definition)?;
        
        // Generate workflow ID
        let workflow_id = WorkflowId::new();
        
        // Store workflow definition
        self.state_manager
            .store_workflow(&workflow_id, &definition)
            .await?;
        
        // Process triggers
        for trigger in &definition.triggers {
            self.activate_trigger(&workflow_id, trigger).await?;
        }
        
        // Emit deployment event
        self.event_bus.publish(EngineEvent::WorkflowDeployed {
            id: workflow_id.clone(),
            definition,
        }).await?;
        
        Ok(workflow_id)
    }
}
```

### Execution Creation

```rust
pub struct ExecutionRequest {
    pub workflow_id: WorkflowId,
    pub input: WorkflowDataItem,
    pub trigger: TriggerInfo,
    pub parent_execution: Option<ExecutionId>,
}

impl WorkflowEngine {
    pub async fn create_execution(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionHandle, Error> {
        // Load workflow
        let workflow = self.state_manager
            .load_workflow(&request.workflow_id)
            .await?;
        
        // Create execution state
        let execution = Execution::new(
            request.workflow_id,
            request.input,
            request.trigger,
        );
        
        // Store initial state
        self.state_manager
            .create_execution(&execution)
            .await?;
        
        // Schedule execution
        self.scheduler
            .schedule_execution(&execution.id)
            .await?;
        
        // Emit creation event
        self.event_bus.publish(EngineEvent::ExecutionCreated {
            id: execution.id.clone(),
            workflow_id: request.workflow_id,
        }).await?;
        
        Ok(ExecutionHandle {
            execution_id: execution.id,
            status_receiver: self.create_status_receiver(&execution.id),
        })
    }
}
```

### DAG Processing

```rust
pub struct DagProcessor {
    graph_analyzer: GraphAnalyzer,
    execution_planner: ExecutionPlanner,
}

impl DagProcessor {
    pub async fn process_workflow(
        &self,
        workflow: &Workflow,
        execution: &Execution,
    ) -> Result<ExecutionPlan, Error> {
        // Analyze graph structure
        let analysis = self.graph_analyzer.analyze(&workflow.graph)?;
        
        // Check for cycles
        if analysis.has_cycles {
            return Err(Error::CyclicWorkflow);
        }
        
        // Create execution plan
        let plan = self.execution_planner.create_plan(
            &workflow.graph,
            &analysis,
            execution,
        )?;
        
        Ok(plan)
    }
}

pub struct ExecutionPlan {
    // Topologically sorted nodes
    pub stages: Vec<ExecutionStage>,
    
    // Parallel execution opportunities
    pub parallelism_map: HashMap<StageId, Vec<NodeId>>,
    
    // Dependencies
    pub dependencies: HashMap<NodeId, Vec<NodeId>>,
    
    // Conditional branches
    pub branches: Vec<ConditionalBranch>,
}
```

### State Management

```rust
pub struct StateManager {
    // Workflow definitions
    workflows: Arc<RwLock<HashMap<WorkflowId, WorkflowDefinition>>>,
    
    // Execution states
    executions: Arc<RwLock<HashMap<ExecutionId, ExecutionState>>>,
    
    // Persistence layer
    storage: Arc<dyn StorageBackend>,
    
    // State snapshots
    snapshots: Arc<SnapshotManager>,
}

#[derive(Debug, Clone)]
pub struct ExecutionState {
    pub id: ExecutionId,
    pub workflow_id: WorkflowId,
    pub status: ExecutionStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub current_nodes: HashSet<NodeId>,
    pub completed_nodes: HashSet<NodeId>,
    pub node_outputs: HashMap<NodeId, WorkflowDataItem>,
    pub variables: HashMap<String, Value>,
    pub error: Option<ExecutionError>,
}

impl StateManager {
    pub async fn update_execution_state<F>(
        &self,
        execution_id: &ExecutionId,
        updater: F,
    ) -> Result<(), Error>
    where
        F: FnOnce(&mut ExecutionState) -> Result<(), Error>,
    {
        let mut executions = self.executions.write().await;
        
        let state = executions
            .get_mut(execution_id)
            .ok_or(Error::ExecutionNotFound)?;
        
        // Apply update
        updater(state)?;
        
        // Persist changes
        self.storage
            .save_execution_state(execution_id, state)
            .await?;
        
        Ok(())
    }
}
```

### Scheduler

```rust
pub struct Scheduler {
    // Work queue
    work_queue: Arc<WorkQueue>,
    
    // Scheduling strategy
    strategy: Box<dyn SchedulingStrategy>,
    
    // Worker pool
    worker_pool: Arc<WorkerPool>,
    
    // Load balancer
    load_balancer: Arc<LoadBalancer>,
}

#[async_trait]
pub trait SchedulingStrategy: Send + Sync {
    async fn select_worker(
        &self,
        task: &Task,
        workers: &[WorkerInfo],
    ) -> Result<WorkerId, Error>;
    
    async fn prioritize_tasks(
        &self,
        tasks: Vec<Task>,
    ) -> Vec<Task>;
}

pub struct PrioritySchedulingStrategy {
    priority_calculator: Box<dyn PriorityCalculator>,
}

impl Scheduler {
    pub async fn schedule_node(
        &self,
        execution_id: &ExecutionId,
        node_id: &NodeId,
    ) -> Result<(), Error> {
        // Create task
        let task = Task {
            id: TaskId::new(),
            execution_id: execution_id.clone(),
            node_id: node_id.clone(),
            priority: self.calculate_priority(execution_id, node_id).await?,
            created_at: Utc::now(),
        };
        
        // Add to queue
        self.work_queue.push(task).await?;
        
        // Notify workers
        self.worker_pool.notify_available_work().await?;
        
        Ok(())
    }
}
```

## Error Handling

### Error Recovery

```rust
pub struct ErrorHandler {
    retry_policy: RetryPolicy,
    fallback_manager: FallbackManager,
    compensation_engine: CompensationEngine,
}

pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff_strategy: BackoffStrategy,
    pub retryable_errors: HashSet<ErrorType>,
}

pub enum BackoffStrategy {
    Fixed { delay: Duration },
    Exponential { base: Duration, factor: f64, max: Duration },
    Linear { increment: Duration },
}

impl ErrorHandler {
    pub async fn handle_node_error(
        &self,
        execution_id: &ExecutionId,
        node_id: &NodeId,
        error: Error,
    ) -> Result<ErrorRecovery, Error> {
        // Check if retryable
        if self.retry_policy.is_retryable(&error) {
            let attempt = self.get_retry_attempt(execution_id, node_id).await?;
            
            if attempt < self.retry_policy.max_attempts {
                let delay = self.retry_policy.calculate_backoff(attempt);
                
                return Ok(ErrorRecovery::Retry {
                    delay,
                    attempt: attempt + 1,
                });
            }
        }
        
        // Check for fallback
        if let Some(fallback) = self.fallback_manager.get_fallback(node_id).await? {
            return Ok(ErrorRecovery::Fallback { node_id: fallback });
        }
        
        // Check for compensation
        if let Some(compensation) = self.compensation_engine.get_compensation(execution_id).await? {
            return Ok(ErrorRecovery::Compensate { 
                workflow_id: compensation,
            });
        }
        
        // No recovery available
        Ok(ErrorRecovery::Fail)
    }
}
```

### Compensation Logic

```rust
pub struct CompensationEngine {
    saga_definitions: HashMap<WorkflowId, SagaDefinition>,
}

pub struct SagaDefinition {
    pub steps: Vec<SagaStep>,
    pub compensation_strategy: CompensationStrategy,
}

pub struct SagaStep {
    pub forward_action: NodeId,
    pub compensating_action: Option<NodeId>,
}

impl CompensationEngine {
    pub async fn compensate(
        &self,
        execution_id: &ExecutionId,
        failed_at: &NodeId,
    ) -> Result<(), Error> {
        let execution = self.load_execution(execution_id).await?;
        let saga = self.saga_definitions.get(&execution.workflow_id)
            .ok_or(Error::NoSagaDefined)?;
        
        // Find completed steps that need compensation
        let steps_to_compensate = self.find_steps_to_compensate(
            &execution,
            failed_at,
            saga,
        )?;
        
        // Execute compensation in reverse order
        for step in steps_to_compensate.iter().rev() {
            if let Some(compensating_action) = &step.compensating_action {
                self.execute_compensation(
                    execution_id,
                    compensating_action,
                ).await?;
            }
        }
        
        Ok(())
    }
}
```

## Performance Optimization

### Execution Cache

```rust
pub struct ExecutionCache {
    // Hot executions in memory
    hot_cache: Arc<RwLock<LruCache<ExecutionId, ExecutionState>>>,
    
    // Warm executions in Redis
    warm_cache: Arc<RedisCache>,
    
    // Metrics
    metrics: Arc<CacheMetrics>,
}

impl ExecutionCache {
    pub async fn get(&self, id: &ExecutionId) -> Result<Option<ExecutionState>, Error> {
        // Check hot cache
        if let Some(state) = self.hot_cache.read().await.get(id) {
            self.metrics.record_hit(CacheLevel::Hot);
            return Ok(Some(state.clone()));
        }
        
        // Check warm cache
        if let Some(state) = self.warm_cache.get(id).await? {
            self.metrics.record_hit(CacheLevel::Warm);
            
            // Promote to hot cache
            self.hot_cache.write().await.put(id.clone(), state.clone());
            
            return Ok(Some(state));
        }
        
        self.metrics.record_miss();
        Ok(None)
    }
}
```

### Parallel Execution

```rust
pub struct ParallelExecutor {
    concurrency_limit: usize,
    semaphore: Arc<Semaphore>,
}

impl ParallelExecutor {
    pub async fn execute_parallel_nodes(
        &self,
        nodes: Vec<NodeId>,
        execution_context: &ExecutionContext,
    ) -> Result<Vec<(NodeId, Result<WorkflowDataItem, Error>)>, Error> {
        let mut handles = Vec::new();
        
        for node_id in nodes {
            let permit = self.semaphore.acquire().await?;
            let context = execution_context.clone();
            
            let handle = tokio::spawn(async move {
                let result = execute_node(&node_id, &context).await;
                drop(permit); // Release semaphore
                (node_id, result)
            });
            
            handles.push(handle);
        }
        
        // Wait for all to complete
        let results = futures::future::join_all(handles).await;
        
        results.into_iter()
            .map(|r| r.map_err(Error::from))
            .collect()
    }
}
```

## Monitoring

### Metrics Collection

```rust
pub struct EngineMetrics {
    // Workflow metrics
    workflows_deployed: Counter,
    workflows_active: Gauge,
    
    // Execution metrics
    executions_created: Counter,
    executions_completed: Counter,
    executions_failed: Counter,
    execution_duration: Histogram,
    
    // Node metrics
    nodes_executed: Counter,
    node_duration: Histogram,
    node_errors: Counter,
    
    // Queue metrics
    queue_depth: Gauge,
    queue_wait_time: Histogram,
}

impl EngineMetrics {
    pub fn record_execution_completed(&self, duration: Duration) {
        self.executions_completed.increment();
        self.execution_duration.record(duration.as_secs_f64());
    }
    
    pub fn record_node_executed(&self, node_type: &str, duration: Duration) {
        self.nodes_executed
            .with_label_values(&[node_type])
            .increment();
        
        self.node_duration
            .with_label_values(&[node_type])
            .record(duration.as_secs_f64());
    }
}
```

### Health Checks

```rust
pub struct EngineHealth {
    components: Vec<Box<dyn HealthCheck>>,
}

#[async_trait]
pub trait HealthCheck: Send + Sync {
    async fn check(&self) -> HealthStatus;
    fn component_name(&self) -> &str;
}

pub struct HealthStatus {
    pub status: Status,
    pub message: Option<String>,
    pub details: HashMap<String, Value>,
}

pub enum Status {
    Healthy,
    Degraded,
    Unhealthy,
}

impl EngineHealth {
    pub async fn check_health(&self) -> OverallHealth {
        let mut results = HashMap::new();
        let mut overall_status = Status::Healthy;
        
        for component in &self.components {
            let status = component.check().await;
            
            match status.status {
                Status::Unhealthy => overall_status = Status::Unhealthy,
                Status::Degraded if overall_status == Status::Healthy => {
                    overall_status = Status::Degraded;
                }
                _ => {}
            }
            
            results.insert(component.component_name().to_string(), status);
        }
        
        OverallHealth {
            status: overall_status,
            components: results,
            timestamp: Utc::now(),
        }
    }
}
```
