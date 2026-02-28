---

# nebula-worker

## Purpose

`nebula-worker` implements the execution environment for workflow nodes with resource isolation, monitoring, and scalability features.

## Responsibilities

- Node execution in isolated environments
- Resource management and limits
- Task acquisition and scheduling
- Health reporting
- Progress tracking
- Execution metrics

## Architecture

### Core Components

```rust
pub struct Worker {
    // Unique identifier
    id: WorkerId,
    
    // Configuration
    config: WorkerConfig,
    
    // Task execution
    executor: Arc<NodeExecutor>,
    
    // Task acquisition
    task_source: Arc<dyn TaskSource>,
    
    // Resource management
    resource_manager: Arc<WorkerResourceManager>,
    
    // Sandbox environment
    sandbox_factory: Arc<SandboxFactory>,
    
    // Health reporting
    health_reporter: Arc<HealthReporter>,
    
    // Metrics
    metrics: Arc<WorkerMetrics>,
    
    // State
    state: Arc<RwLock<WorkerState>>,
}

pub struct WorkerConfig {
    pub id: WorkerId,
    pub max_concurrent_tasks: usize,
    pub resource_limits: ResourceLimits,
    pub task_timeout: Duration,
    pub heartbeat_interval: Duration,
    pub sandbox_config: SandboxConfig,
}

pub struct WorkerState {
    pub status: WorkerStatus,
    pub current_tasks: HashMap<TaskId, TaskExecution>,
    pub completed_tasks: u64,
    pub failed_tasks: u64,
    pub start_time: DateTime<Utc>,
}

pub enum WorkerStatus {
    Starting,
    Idle,
    Busy { tasks: usize },
    Draining,
    Stopped,
}
```

### Task Execution

```rust
pub struct NodeExecutor {
    // Node registry
    node_registry: Arc<NodeRegistry>,
    
    // Execution context factory
    context_factory: Arc<ContextFactory>,
    
    // Input/output handler
    io_handler: Arc<IoHandler>,
    
    // Progress reporter
    progress_reporter: Arc<ProgressReporter>,
}

pub struct TaskExecution {
    pub task_id: TaskId,
    pub execution_id: ExecutionId,
    pub node_id: NodeId,
    pub started_at: DateTime<Utc>,
    pub sandbox: Box<dyn ExecutionSandbox>,
    pub resources: TaskResources,
    pub cancel_token: CancellationToken,
}

impl NodeExecutor {
    pub async fn execute_task(
        &self,
        task: Task,
        sandbox: Box<dyn ExecutionSandbox>,
    ) -> Result<TaskResult, Error> {
        // Load node implementation
        let node = self.node_registry
            .get_node(&task.node_type)
            .await?
            .ok_or(Error::NodeNotFound)?;
            
        // Prepare execution context
        let mut context = self.context_factory
            .create_context(&task, &sandbox)
            .await?;
            
        // Load input data
        let input = self.io_handler
            .load_input(&task.execution_id, &task.node_id)
            .await?;
            
        // Execute with timeout
        let result = timeout(
            task.timeout.unwrap_or(self.default_timeout),
            self.execute_with_progress(node, input, &mut context)
        ).await??;
        
        // Save output
        self.io_handler
            .save_output(&task.execution_id, &task.node_id, &result)
            .await?;
            
        Ok(TaskResult {
            task_id: task.task_id,
            status: TaskStatus::Completed,
            output: Some(result),
            error: None,
            metrics: context.get_metrics(),
        })
    }
    
    async fn execute_with_progress(
        &self,
        node: Arc<dyn Action>,
        input: WorkflowDataItem,
        context: &mut ExecutionContext,
    ) -> Result<WorkflowDataItem, Error> {
        let progress_handle = self.progress_reporter
            .start_progress(&context.task_id())
            .await?;
            
        // Set up progress callback
        context.set_progress_callback(move |progress| {
            progress_handle.update(progress);
        });
        
        // Execute node
        let result = node.execute(input, context).await?;
        
        // Complete progress
        progress_handle.complete().await?;
        
        Ok(result)
    }
}
```

### Execution Sandbox

```rust
#[async_trait]
pub trait ExecutionSandbox: Send + Sync {
    // Resource limits
    async fn set_memory_limit(&mut self, bytes: usize) -> Result<(), Error>;
    async fn set_cpu_limit(&mut self, millicpus: u32) -> Result<(), Error>;
    async fn set_io_limit(&mut self, bytes_per_sec: u64) -> Result<(), Error>;
    
    // Monitoring
    async fn get_resource_usage(&self) -> ResourceUsage;
    async fn get_metrics(&self) -> SandboxMetrics;
    
    // Lifecycle
    async fn start(&mut self) -> Result<(), Error>;
    async fn stop(&mut self) -> Result<(), Error>;
    async fn cleanup(&mut self) -> Result<(), Error>;
}

pub struct ProcessSandbox {
    config: ProcessSandboxConfig,
    process: Option<Child>,
    resource_monitor: ResourceMonitor,
}

pub struct ProcessSandboxConfig {
    pub memory_limit: usize,
    pub cpu_limit: u32,
    pub io_limit: u64,
    pub allowed_syscalls: HashSet<String>,
    pub network_enabled: bool,
    pub temp_dir: PathBuf,
}

#[async_trait]
impl ExecutionSandbox for ProcessSandbox {
    async fn set_memory_limit(&mut self, bytes: usize) -> Result<(), Error> {
        self.config.memory_limit = bytes;
        
        if let Some(process) = &self.process {
            // Apply cgroup limits
            cgroups::memory::set_limit(process.id(), bytes)?;
        }
        
        Ok(())
    }
    
    async fn get_resource_usage(&self) -> ResourceUsage {
        if let Some(process) = &self.process {
            self.resource_monitor.get_usage(process.id()).await
        } else {
            ResourceUsage::default()
        }
    }
    
    async fn start(&mut self) -> Result<(), Error> {
        let mut command = Command::new(&self.config.binary_path);
        
        // Apply security restrictions
        command
            .uid(self.config.uid)
            .gid(self.config.gid)
            .current_dir(&self.config.temp_dir);
            
        // Set environment
        command.env_clear();
        for (key, value) in &self.config.env_vars {
            command.env(key, value);
        }
        
        // Start process
        let process = command.spawn()?;
        self.process = Some(process);
        
        // Start resource monitoring
        self.resource_monitor.start_monitoring(process.id()).await?;
        
        Ok(())
    }
}
```

### Task Acquisition

```rust
#[async_trait]
pub trait TaskSource: Send + Sync {
    async fn acquire_task(&self, worker_id: &WorkerId) -> Result<Option<Task>, Error>;
    async fn complete_task(&self, result: &TaskResult) -> Result<(), Error>;
    async fn heartbeat(&self, worker_id: &WorkerId, task_id: &TaskId) -> Result<(), Error>;
}

pub struct KafkaTaskSource {
    consumer: StreamConsumer,
    producer: FutureProducer,
    config: KafkaTaskConfig,
}

#[async_trait]
impl TaskSource for KafkaTaskSource {
    async fn acquire_task(&self, worker_id: &WorkerId) -> Result<Option<Task>, Error> {
        // Poll for messages
        match self.consumer.recv().await {
            Ok(message) => {
                let task: Task = serde_json::from_slice(message.payload().unwrap())?;
                
                // Claim task
                let claim = TaskClaim {
                    task_id: task.task_id.clone(),
                    worker_id: worker_id.clone(),
                    claimed_at: Utc::now(),
                };
                
                self.producer
                    .send(
                        FutureRecord::to("task-claims")
                            .key(&task.task_id.to_string())
                            .payload(&serde_json::to_string(&claim)?),
                        Duration::from_secs(0),
                    )
                    .await?;
                    
                Ok(Some(task))
            }
            Err(e) => {
                if e.is_timeout() {
                    Ok(None)
                } else {
                    Err(e.into())
                }
            }
        }
    }
}

pub struct QueueTaskSource {
    queue_url: String,
    client: Arc<dyn QueueClient>,
}

#[async_trait]
impl TaskSource for QueueTaskSource {
    async fn acquire_task(&self, worker_id: &WorkerId) -> Result<Option<Task>, Error> {
        let messages = self.client
            .receive_messages(&self.queue_url, 1, Duration::from_secs(20))
            .await?;
            
        if let Some(message) = messages.first() {
            let task: Task = serde_json::from_str(&message.body)?;
            
            // Update visibility timeout for processing
            self.client
                .change_message_visibility(
                    &self.queue_url,
                    &message.receipt_handle,
                    Duration::from_mins(30),
                )
                .await?;
                
            Ok(Some(task))
        } else {
            Ok(None)
        }
    }
}
```

### Resource Management

```rust
pub struct WorkerResourceManager {
    // Resource pools
    memory_pool: Arc<MemoryPool>,
    cpu_scheduler: Arc<CpuScheduler>,
    io_controller: Arc<IoController>,
    
    // Current allocations
    allocations: Arc<RwLock<HashMap<TaskId, ResourceAllocation>>>,
    
    // Limits
    limits: ResourceLimits,
}

pub struct ResourceAllocation {
    pub memory: MemoryAllocation,
    pub cpu: CpuAllocation,
    pub io: IoAllocation,
    pub allocated_at: DateTime<Utc>,
}

impl WorkerResourceManager {
    pub async fn allocate_resources(
        &self,
        task: &Task,
    ) -> Result<ResourceAllocation, Error> {
        // Check available resources
        let available = self.get_available_resources().await?;
        
        let requirements = task.resource_requirements
            .as_ref()
            .cloned()
            .unwrap_or_default();
            
        if !available.can_satisfy(&requirements) {
            return Err(Error::InsufficientResources);
        }
        
        // Allocate memory
        let memory = self.memory_pool
            .allocate(requirements.memory_mb * 1024 * 1024)
            .await?;
            
        // Reserve CPU
        let cpu = self.cpu_scheduler
            .reserve(requirements.cpu_millicores)
            .await?;
            
        // Configure I/O
        let io = self.io_controller
            .create_cgroup(requirements.io_ops_per_sec)
            .await?;
            
        let allocation = ResourceAllocation {
            memory,
            cpu,
            io,
            allocated_at: Utc::now(),
        };
        
        self.allocations.write().await.insert(task.task_id.clone(), allocation.clone());
        
        Ok(allocation)
    }
    
    pub async fn release_resources(&self, task_id: &TaskId) -> Result<(), Error> {
        if let Some(allocation) = self.allocations.write().await.remove(task_id) {
            self.memory_pool.release(allocation.memory).await?;
            self.cpu_scheduler.release(allocation.cpu).await?;
            self.io_controller.destroy_cgroup(allocation.io).await?;
        }
        
        Ok(())
    }
}
```

### Worker Pool

```rust
pub struct WorkerPool {
    workers: Arc<RwLock<HashMap<WorkerId, WorkerHandle>>>,
    config: WorkerPoolConfig,
    scaler: Arc<AutoScaler>,
    load_balancer: Arc<LoadBalancer>,
}

pub struct WorkerPoolConfig {
    pub min_workers: usize,
    pub max_workers: usize,
    pub scale_up_threshold: f64,
    pub scale_down_threshold: f64,
    pub worker_config_template: WorkerConfig,
}

pub struct WorkerHandle {
    pub worker: Arc<Worker>,
    pub thread: JoinHandle<()>,
    pub metrics: WorkerMetrics,
}

impl WorkerPool {
    pub async fn start(&self) -> Result<(), Error> {
        // Start minimum workers
        for i in 0..self.config.min_workers {
            self.spawn_worker().await?;
        }
        
        // Start auto-scaling loop
        self.scaler.start(self.clone()).await?;
        
        Ok(())
    }
    
    pub async fn spawn_worker(&self) -> Result<WorkerId, Error> {
        let worker_id = WorkerId::new();
        let mut config = self.config.worker_config_template.clone();
        config.id = worker_id.clone();
        
        let worker = Arc::new(Worker::new(config).await?);
        let worker_clone = worker.clone();
        
        let thread = tokio::spawn(async move {
            if let Err(e) = worker_clone.run().await {
                error!("Worker {} failed: {}", worker_clone.id, e);
            }
        });
        
        let handle = WorkerHandle {
            worker: worker.clone(),
            thread,
            metrics: worker.metrics.clone(),
        };
        
        self.workers.write().await.insert(worker_id.clone(), handle);
        
        info!("Spawned worker {}", worker_id);
        
        Ok(worker_id)
    }
    
    pub async fn scale_workers(&self, target_count: usize) -> Result<(), Error> {
        let current_count = self.workers.read().await.len();
        
        match target_count.cmp(&current_count) {
            Ordering::Greater => {
                // Scale up
                let to_spawn = (target_count - current_count)
                    .min(self.config.max_workers - current_count);
                    
                for _ in 0..to_spawn {
                    self.spawn_worker().await?;
                }
            }
            Ordering::Less => {
                // Scale down
                let to_remove = current_count - target_count;
                self.remove_workers(to_remove).await?;
            }
            Ordering::Equal => {
                // No change needed
            }
        }
        
        Ok(())
    }
}
```

### Health Reporting

```rust
pub struct HealthReporter {
    worker_id: WorkerId,
    health_endpoint: String,
    client: reqwest::Client,
    interval: Duration,
}

impl HealthReporter {
    pub async fn start(&self, state: Arc<RwLock<WorkerState>>) {
        let mut interval = tokio::time::interval(self.interval);
        
        loop {
            interval.tick().await;
            
            let health = self.collect_health(state.clone()).await;
            
            if let Err(e) = self.report_health(health).await {
                warn!("Failed to report health: {}", e);
            }
        }
    }
    
    async fn collect_health(&self, state: Arc<RwLock<WorkerState>>) -> WorkerHealth {
        let state = state.read().await;
        let resource_usage = self.get_resource_usage().await;
        
        WorkerHealth {
            worker_id: self.worker_id.clone(),
            status: state.status.clone(),
            current_tasks: state.current_tasks.len(),
            completed_tasks: state.completed_tasks,
            failed_tasks: state.failed_tasks,
            uptime: Utc::now() - state.start_time,
            resource_usage,
            timestamp: Utc::now(),
        }
    }
    
    async fn report_health(&self, health: WorkerHealth) -> Result<(), Error> {
        self.client
            .post(&self.health_endpoint)
            .json(&health)
            .send()
            .await?;
            
        Ok(())
    }
}
```

## Worker Lifecycle

### Main Execution Loop

```rust
impl Worker {
    pub async fn run(&self) -> Result<(), Error> {
        info!("Worker {} starting", self.id);
        
        // Update state
        self.state.write().await.status = WorkerStatus::Idle;
        
        // Start health reporting
        let health_handle = self.start_health_reporting();
        
        // Main loop
        while !self.should_stop().await {
            match self.task_source.acquire_task(&self.id).await? {
                Some(task) => {
                    self.handle_task(task).await?;
                }
                None => {
                    // No task available, wait a bit
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
        
        // Graceful shutdown
        self.drain_tasks().await?;
        
        info!("Worker {} stopped", self.id);
        
        Ok(())
    }
    
    async fn handle_task(&self, task: Task) -> Result<(), Error> {
        let task_id = task.task_id.clone();
        
        info!("Worker {} executing task {}", self.id, task_id);
        
        // Update state
        {
            let mut state = self.state.write().await;
            state.status = WorkerStatus::Busy {
                tasks: state.current_tasks.len() + 1,
            };
            
            state.current_tasks.insert(
                task_id.clone(),
                TaskExecution {
                    task_id: task_id.clone(),
                    execution_id: task.execution_id.clone(),
                    node_id: task.node_id.clone(),
                    started_at: Utc::now(),
                    sandbox: self.sandbox_factory.create_sandbox(&task).await?,
                    resources: self.resource_manager.allocate_resources(&task).await?,
                    cancel_token: CancellationToken::new(),
                },
            );
        }
        
        // Execute task
        let result = self.executor.execute_task(task, sandbox).await;
        
        // Update state
        {
            let mut state = self.state.write().await;
            state.current_tasks.remove(&task_id);
            
            match &result {
                Ok(_) => state.completed_tasks += 1,
                Err(_) => state.failed_tasks += 1,
            }
            
            if state.current_tasks.is_empty() {
                state.status = WorkerStatus::Idle;
            }
        }
        
        // Report result
        self.task_source.complete_task(&result).await?;
        
        // Release resources
        self.resource_manager.release_resources(&task_id).await?;
        
        // Update metrics
        self.metrics.record_task_completion(&result);
        
        Ok(())
    }
}
```

---

