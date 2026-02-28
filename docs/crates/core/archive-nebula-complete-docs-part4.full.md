# Nebula Complete Documentation - Part 4

---
## FILE: docs/crates/nebula-worker.md
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
## FILE: docs/crates/nebula-node-registry.md
---

# nebula-node-registry

## Purpose

`nebula-node-registry` manages the discovery, loading, versioning, and lifecycle of workflow nodes, including support for dynamically loaded plugins and git-based distribution.

## Responsibilities

- Node discovery and registration
- Plugin loading and management
- Version management
- Git-based node distribution
- Node caching and optimization
- Dependency resolution

## Architecture

### Core Components

```rust
pub struct NodeRegistry {
    // Loaded nodes
    nodes: Arc<RwLock<HashMap<String, RegisteredNode>>>,
    
    // Plugin manager
    plugin_manager: Arc<PluginManager>,
    
    // Git integrator
    git_integrator: Arc<GitIntegrator>,
    
    // Node cache
    cache: Arc<NodeCache>,
    
    // Discovery service
    discovery: Arc<DiscoveryService>,
    
    // Dependency resolver
    dependency_resolver: Arc<DependencyResolver>,
    
    // Metrics
    metrics: Arc<RegistryMetrics>,
}

pub struct RegisteredNode {
    pub metadata: NodeMetadata,
    pub factory: Box<dyn NodeFactory>,
    pub source: NodeSource,
    pub loaded_at: DateTime<Utc>,
    pub usage_count: AtomicU64,
}

pub enum NodeSource {
    BuiltIn,
    Plugin { path: PathBuf, manifest: PluginManifest },
    Git { url: String, commit: String },
    Registry { name: String, version: Version },
}
```

### Node Discovery

```rust
pub struct DiscoveryService {
    // Discovery strategies
    strategies: Vec<Box<dyn DiscoveryStrategy>>,
    
    // Discovery cache
    cache: Arc<DiscoveryCache>,
}

#[async_trait]
pub trait DiscoveryStrategy: Send + Sync {
    async fn discover(&self) -> Result<Vec<DiscoveredNode>, Error>;
    fn name(&self) -> &str;
}

pub struct DiscoveredNode {
    pub id: String,
    pub name: String,
    pub version: Version,
    pub location: NodeLocation,
    pub metadata: Option<NodeMetadata>,
}

pub enum NodeLocation {
    Library { path: PathBuf },
    Git { url: String, branch: Option<String> },
    Registry { url: String, package: String },
}

// File system discovery
pub struct FileSystemDiscovery {
    search_paths: Vec<PathBuf>,
    pattern: Regex,
}

#[async_trait]
impl DiscoveryStrategy for FileSystemDiscovery {
    async fn discover(&self) -> Result<Vec<DiscoveredNode>, Error> {
        let mut discovered = Vec::new();
        
        for path in &self.search_paths {
            if !path.exists() {
                continue;
            }
            
            for entry in WalkDir::new(path) {
                let entry = entry?;
                let path = entry.path();
                
                if path.is_file() && self.pattern.is_match(path.to_str().unwrap_or("")) {
                    if let Some(node) = self.analyze_library(path).await? {
                        discovered.push(node);
                    }
                }
            }
        }
        
        Ok(discovered)
    }
}

// Convention-based discovery
pub struct ConventionBasedDiscovery {
    target_dir: PathBuf,
    prefix: String,
}

#[async_trait]
impl DiscoveryStrategy for ConventionBasedDiscovery {
    async fn discover(&self) -> Result<Vec<DiscoveredNode>, Error> {
        let mut discovered = Vec::new();
        let pattern = format!("{}*.{}", self.prefix, LIB_EXTENSION);
        
        for entry in fs::read_dir(&self.target_dir).await? {
            let entry = entry?;
            let path = entry.path();
            let filename = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
                
            if filename.starts_with(&self.prefix) && filename.ends_with(LIB_EXTENSION) {
                discovered.push(DiscoveredNode {
                    id: extract_node_id(filename),
                    name: extract_node_name(filename),
                    version: Version::parse("0.0.0").unwrap(),
                    location: NodeLocation::Library { path },
                    metadata: None,
                });
            }
        }
        
        Ok(discovered)
    }
}
```

### Plugin Management

```rust
pub struct PluginManager {
    // Loaded plugins
    plugins: Arc<RwLock<HashMap<PluginId, LoadedPlugin>>>,
    
    // Plugin loader
    loader: Arc<PluginLoader>,
    
    // Sandbox for plugins
    sandbox: Arc<PluginSandbox>,
}

pub struct LoadedPlugin {
    pub id: PluginId,
    pub manifest: PluginManifest,
    pub library: Library,
    pub nodes: Vec<String>,
    pub resources: PluginResources,
}

#[derive(Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub license: String,
    pub compatibility: CompatibilityInfo,
    pub nodes: Vec<NodeManifest>,
    pub dependencies: Vec<Dependency>,
}

pub struct PluginLoader {
    validator: Arc<PluginValidator>,
    abi_checker: Arc<AbiChecker>,
}

impl PluginLoader {
    pub async fn load_plugin(&self, path: &Path) -> Result<LoadedPlugin, Error> {
        // Read manifest
        let manifest_path = path.join("plugin.toml");
        let manifest: PluginManifest = toml::from_str(
            &fs::read_to_string(manifest_path).await?
        )?;
        
        // Validate plugin
        self.validator.validate(&manifest, path).await?;
        
        // Check ABI compatibility
        let lib_path = path.join(&format!("lib{}.so", manifest.name));
        self.abi_checker.check_compatibility(&lib_path).await?;
        
        // Load library
        let library = unsafe { Library::new(&lib_path)? };
        
        // Get plugin interface
        let plugin_interface: Symbol<fn() -> PluginInterface> =
            unsafe { library.get(b"plugin_interface")? };
            
        let interface = plugin_interface();
        
        // Verify version
        if interface.abi_version != CURRENT_ABI_VERSION {
            return Err(Error::IncompatibleAbiVersion {
                expected: CURRENT_ABI_VERSION,
                found: interface.abi_version,
            });
        }
        
        // Initialize plugin
        let mut context = PluginContext::new();
        if (interface.init)(&mut context) != 0 {
            return Err(Error::PluginInitializationFailed);
        }
        
        // Register nodes
        let mut registry = NodeRegistryHandle::new();
        (interface.register_nodes)(&mut registry);
        
        Ok(LoadedPlugin {
            id: PluginId::from(&manifest.name),
            manifest,
            library,
            nodes: registry.registered_nodes(),
            resources: PluginResources::default(),
        })
    }
}
```

### Git Integration

```rust
pub struct GitIntegrator {
    work_dir: PathBuf,
    builder: Arc<NodeBuilder>,
    cache: Arc<GitCache>,
}

pub struct GitNodeSource {
    pub url: String,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub path: Option<String>,
    pub build_command: Option<String>,
}

impl GitIntegrator {
    pub async fn install_from_git(
        &self,
        source: GitNodeSource,
    ) -> Result<InstalledNode, Error> {
        // Check cache first
        let cache_key = self.calculate_cache_key(&source);
        if let Some(cached) = self.cache.get(&cache_key).await? {
            return Ok(cached);
        }
        
        // Create work directory
        let work_path = self.work_dir.join(&cache_key);
        fs::create_dir_all(&work_path).await?;
        
        // Clone repository
        let repo = self.clone_or_update(&source, &work_path).await?;
        
        // Checkout specific commit/branch
        if let Some(commit) = &source.commit {
            repo.checkout_commit(commit)?;
        } else if let Some(branch) = &source.branch {
            repo.checkout_branch(branch)?;
        }
        
        // Navigate to path if specified
        let build_path = if let Some(path) = &source.path {
            work_path.join(path)
        } else {
            work_path
        };
        
        // Build node
        let build_output = self.builder
            .build(&build_path, source.build_command.as_deref())
            .await?;
            
        // Find built libraries
        let libraries = self.find_built_libraries(&build_output.target_dir).await?;
        
        // Create installed node
        let installed = InstalledNode {
            id: NodeId::new(),
            source: source.clone(),
            libraries,
            built_at: Utc::now(),
        };
        
        // Cache result
        self.cache.put(&cache_key, &installed).await?;
        
        Ok(installed)
    }
    
    async fn clone_or_update(
        &self,
        source: &GitNodeSource,
        path: &Path,
    ) -> Result<Repository, Error> {
        if path.join(".git").exists() {
            // Update existing repository
            let repo = Repository::open(path)?;
            
            let mut remote = repo.find_remote("origin")?;
            remote.fetch(&[], None, None)?;
            
            Ok(repo)
        } else {
            // Clone new repository
            Ok(Repository::clone(&source.url, path)?)
        }
    }
}
```

### Node Caching

```rust
pub struct NodeCache {
    // Memory cache for hot nodes
    memory_cache: Arc<MemoryCache<String, CachedNode>>,
    
    // Disk cache for compiled nodes
    disk_cache: Arc<DiskCache>,
    
    // Cache statistics
    stats: Arc<CacheStats>,
}

pub struct CachedNode {
    pub factory: Arc<dyn NodeFactory>,
    pub metadata: NodeMetadata,
    pub size: usize,
    pub last_used: Instant,
    pub use_count: u64,
}

impl NodeCache {
    pub async fn get_or_load<F>(
        &self,
        node_id: &str,
        loader: F,
    ) -> Result<Arc<dyn NodeFactory>, Error>
    where
        F: FnOnce() -> Future<Output = Result<Box<dyn NodeFactory>, Error>>,
    {
        // Check memory cache
        if let Some(cached) = self.memory_cache.get(node_id).await {
            self.stats.record_hit(CacheLevel::Memory);
            cached.use_count.fetch_add(1, Ordering::Relaxed);
            return Ok(cached.factory.clone());
        }
        
        // Check disk cache
        if let Some(path) = self.disk_cache.get_path(node_id).await? {
            self.stats.record_hit(CacheLevel::Disk);
            
            let factory = self.load_from_disk(&path).await?;
            self.promote_to_memory(node_id, factory.clone()).await?;
            
            return Ok(factory);
        }
        
        // Load and cache
        self.stats.record_miss();
        
        let factory = Arc::from(loader().await?);
        self.cache_node(node_id, factory.clone()).await?;
        
        Ok(factory)
    }
    
    pub async fn evict_cold_nodes(&self) -> Result<EvictionStats, Error> {
        let mut stats = EvictionStats::default();
        
        // Find cold nodes
        let cold_nodes = self.memory_cache
            .entries()
            .filter(|(_, node)| {
                node.last_used.elapsed() > Duration::from_hours(1) &&
                node.use_count < 10
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
            
        // Evict from memory
        for node_id in cold_nodes {
            if let Some(evicted) = self.memory_cache.remove(&node_id).await {
                stats.evicted_count += 1;
                stats.freed_memory += evicted.size;
                
                // Keep in disk cache
                self.disk_cache.ensure_cached(&node_id).await?;
            }
        }
        
        Ok(stats)
    }
}
```

### Dependency Resolution

```rust
pub struct DependencyResolver {
    registry: Arc<NodeRegistry>,
    version_resolver: Arc<VersionResolver>,
}

pub struct Dependency {
    pub name: String,
    pub version_req: VersionReq,
    pub optional: bool,
    pub features: Vec<String>,
}

impl DependencyResolver {
    pub async fn resolve_dependencies(
        &self,
        node: &NodeManifest,
    ) -> Result<ResolvedDependencies, Error> {
        let mut resolver = DependencyGraph::new();
        
        // Add root node
        resolver.add_node(&node.name, &node.version);
        
        // Resolve recursively
        self.resolve_recursive(&mut resolver, &node.name, &node.dependencies).await?;
        
        // Check for conflicts
        if let Some(conflict) = resolver.find_conflict() {
            return Err(Error::DependencyConflict(conflict));
        }
        
        // Create resolution
        Ok(resolver.create_resolution())
    }
    
    async fn resolve_recursive(
        &self,
        graph: &mut DependencyGraph,
        parent: &str,
        dependencies: &[Dependency],
    ) -> Result<(), Error> {
        for dep in dependencies {
            // Find matching versions
            let versions = self.registry
                .find_node_versions(&dep.name)
                .await?;
                
            let matching = versions
                .into_iter()
                .filter(|v| dep.version_req.matches(v))
                .collect::<Vec<_>>();
                
            if matching.is_empty() && !dep.optional {
                return Err(Error::DependencyNotFound {
                    name: dep.name.clone(),
                    requirement: dep.version_req.clone(),
                });
            }
            
            if let Some(version) = self.version_resolver.select_best(&matching) {
                graph.add_edge(parent, &dep.name, version);
                
                // Load dependency manifest
                let dep_manifest = self.registry
                    .get_node_manifest(&dep.name, version)
                    .await?;
                    
                // Recurse
                self.resolve_recursive(
                    graph,
                    &dep.name,
                    &dep_manifest.dependencies
                ).await?;
            }
        }
        
        Ok(())
    }
}
```

### Registry API

```rust
impl NodeRegistry {
    pub async fn register_node(
        &self,
        factory: Box<dyn NodeFactory>,
        source: NodeSource,
    ) -> Result<(), Error> {
        let metadata = factory.metadata();
        let node_id = metadata.id.clone();
        
        info!("Registering node: {} v{}", metadata.name, metadata.version);
        
        // Check for conflicts
        if let Some(existing) = self.nodes.read().await.get(&node_id) {
            if existing.metadata.version >= metadata.version {
                return Err(Error::NodeAlreadyRegistered {
                    id: node_id,
                    version: existing.metadata.version.clone(),
                });
            }
        }
        
        // Create registered node
        let registered = RegisteredNode {
            metadata: metadata.clone(),
            factory,
            source,
            loaded_at: Utc::now(),
            usage_count: AtomicU64::new(0),
        };
        
        // Register
        self.nodes.write().await.insert(node_id.clone(), registered);
        
        // Update metrics
        self.metrics.nodes_registered.increment();
        
        // Emit event
        self.emit_node_registered_event(&metadata).await?;
        
        Ok(())
    }
    
    pub async fn get_node(&self, node_id: &str) -> Result<Arc<dyn Action>, Error> {
        // Get from registry
        let registered = self.nodes
            .read()
            .await
            .get(node_id)
            .cloned()
            .ok_or(Error::NodeNotFound)?;
            
        // Update usage
        registered.usage_count.fetch_add(1, Ordering::Relaxed);
        
        // Create instance
        let instance = registered.factory.create().await?;
        
        Ok(instance)
    }
    
    pub async fn list_nodes(&self, filter: NodeFilter) -> Vec<NodeMetadata> {
        self.nodes
            .read()
            .await
            .values()
            .filter(|node| filter.matches(&node.metadata))
            .map(|node| node.metadata.clone())
            .collect()
    }
}
```

---
## FILE: docs/crates/nebula-api.md
---

# nebula-api

## Purpose

`nebula-api` provides the external API layer for Nebula: **REST + WebSocket** (GraphQL не планируется в текущей фазе).

## Responsibilities

- REST API endpoints
- WebSocket real-time communication
- Authentication and authorization
- Rate limiting
- API documentation

## Architecture

### Core Components

```rust
pub struct ApiServer {
    // HTTP server
    server: Server,
    
    // API implementations (REST + WebSocket only)
    rest_api: Arc<RestApi>,
    websocket_handler: Arc<WebSocketHandler>,
    
    // Shared services
    auth_service: Arc<AuthService>,
    rate_limiter: Arc<RateLimiter>,
    
    // Backend services
    engine: Arc<WorkflowEngine>,
    storage: Arc<dyn StorageBackend>,
    
    // Metrics
    metrics: Arc<ApiMetrics>,
}

pub struct ApiConfig {
    pub host: String,
    pub port: u16,
    pub tls_config: Option<TlsConfig>,
    pub cors_config: CorsConfig,
    pub auth_config: AuthConfig,
    pub rate_limit_config: RateLimitConfig,
}
```

### REST API

```rust
pub struct RestApi {
    engine: Arc<WorkflowEngine>,
    storage: Arc<dyn StorageBackend>,
    validator: Arc<RequestValidator>,
}

impl RestApi {
    pub fn routes(&self) -> Router {
        Router::new()
            // Workflow endpoints
            .route("/api/v1/workflows", post(create_workflow))
            .route("/api/v1/workflows", get(list_workflows))
            .route("/api/v1/workflows/:id", get(get_workflow))
            .route("/api/v1/workflows/:id", put(update_workflow))
            .route("/api/v1/workflows/:id", delete(delete_workflow))
            .route("/api/v1/workflows/:id/versions", get(list_versions))
            .route("/api/v1/workflows/:id/activate", post(activate_workflow))
            .route("/api/v1/workflows/:id/deactivate", post(deactivate_workflow))
            
            // Execution endpoints
            .route("/api/v1/workflows/:id/execute", post(execute_workflow))
            .route("/api/v1/executions", get(list_executions))
            .route("/api/v1/executions/:id", get(get_execution))
            .route("/api/v1/executions/:id/cancel", post(cancel_execution))
            .route("/api/v1/executions/:id/logs", get(get_execution_logs))
            .route("/api/v1/executions/:id/nodes/:node_id/output", get(get_node_output))
            
            // Node endpoints
            .route("/api/v1/nodes", get(list_nodes))
            .route("/api/v1/nodes/:id", get(get_node))
            .route("/api/v1/nodes/:id/documentation", get(get_node_docs))
            
            // Resource endpoints
            .route("/api/v1/resources", get(list_resources))
            .route("/api/v1/resources/:id/health", get(check_resource_health))
            
            // System endpoints
            .route("/api/v1/health", get(health_check))
            .route("/api/v1/metrics", get(get_metrics))
            
            // Apply middleware
            .layer(AuthLayer::new(self.auth_service.clone()))
            .layer(RateLimitLayer::new(self.rate_limiter.clone()))
            .layer(TraceLayer::new_for_http())
            .layer(CompressionLayer::new())
            .layer(CorsLayer::new(self.cors_config.clone()))
    }
}

// Workflow handlers
async fn create_workflow(
    State(api): State<Arc<RestApi>>,
    Json(request): Json<CreateWorkflowRequest>,
) -> Result<Json<CreateWorkflowResponse>, ApiError> {
    // Validate request
    api.validator.validate(&request)?;
    
    // Create workflow
    let workflow = Workflow::from_request(request)?;
    api.storage.save_workflow(&workflow).await?;
    
    // Deploy if requested
    if request.deploy {
        api.engine.deploy_workflow(workflow.clone()).await?;
    }
    
    Ok(Json(CreateWorkflowResponse {
        id: workflow.id,
        version: workflow.version,
        status: workflow.status,
    }))
}

async fn execute_workflow(
    State(api): State<Arc<RestApi>>,
    Path(workflow_id): Path<String>,
    Json(request): Json<ExecuteWorkflowRequest>,
) -> Result<Json<ExecuteWorkflowResponse>, ApiError> {
    let workflow_id = WorkflowId::from_str(&workflow_id)?;
    
    // Create execution request
    let execution_request = ExecutionRequest {
        workflow_id,
        input: request.input,
        trigger: TriggerInfo::Manual {
            user: request.user,
        },
        parent_execution: request.parent_execution,
    };
    
    // Execute
    let handle = api.engine.create_execution(execution_request).await?;
    
    Ok(Json(ExecuteWorkflowResponse {
        execution_id: handle.execution_id,
        status: ExecutionStatus::Created,
    }))
}
```

### GraphQL — отложен

API только REST + WebSocket. GraphQL при необходимости можно добавить позже.

<details>
<summary>Возможная будущая структура GraphQL (не в текущем плане)</summary>

```rust
pub struct GraphqlApi {
    schema: Schema<Query, Mutation, Subscription>,
}

#[derive(Default)]
pub struct Query;

#[Object]
impl Query {
    async fn workflow(&self, ctx: &Context<'_>, id: ID) -> Result<Option<Workflow>> {
        let storage = ctx.data::<Arc<dyn StorageBackend>>()?;
        let workflow_id = WorkflowId::from_str(&id)?;
        
        match storage.load_workflow(&workflow_id).await {
            Ok(workflow) => Ok(Some(workflow)),
            Err(Error::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    
    async fn workflows(
        &self,
        ctx: &Context<'_>,
        filter: Option<WorkflowFilterInput>,
        first: Option<i32>,
        after: Option<String>,
    ) -> Result<Connection<Workflow>> {
        let storage = ctx.data::<Arc<dyn StorageBackend>>()?;
        
        let filter = filter.map(Into::into).unwrap_or_default();
        let workflows = storage.list_workflows(filter).await?;
        
        // Create connection
        let connection = Connection::new(
            workflows,
            first.unwrap_or(20) as usize,
            after,
        );
        
        Ok(connection)
    }
    
    async fn execution(&self, ctx: &Context<'_>, id: ID) -> Result<Option<Execution>> {
        let storage = ctx.data::<Arc<dyn StorageBackend>>()?;
        let execution_id = ExecutionId::from_str(&id)?;
        
        match storage.load_execution(&execution_id).await {
            Ok(execution) => Ok(Some(execution)),
            Err(Error::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    
    async fn node(&self, ctx: &Context<'_>, id: String) -> Result<Option<NodeInfo>> {
        let registry = ctx.data::<Arc<NodeRegistry>>()?;
        
        match registry.get_node_metadata(&id).await {
            Some(metadata) => Ok(Some(NodeInfo::from(metadata))),
            None => Ok(None),
        }
    }
}

#[derive(Default)]
pub struct Mutation;

#[Object]
impl Mutation {
    async fn create_workflow(
        &self,
        ctx: &Context<'_>,
        input: CreateWorkflowInput,
    ) -> Result<CreateWorkflowPayload> {
        let storage = ctx.data::<Arc<dyn StorageBackend>>()?;
        let engine = ctx.data::<Arc<WorkflowEngine>>()?;
        
        let workflow = Workflow::from_input(input)?;
        storage.save_workflow(&workflow).await?;
        
        if input.deploy {
            engine.deploy_workflow(workflow.clone()).await?;
        }
        
        Ok(CreateWorkflowPayload {
            workflow,
            success: true,
        })
    }
    
    async fn execute_workflow(
        &self,
        ctx: &Context<'_>,
        workflow_id: ID,
        input: Option<Json>,
    ) -> Result<ExecuteWorkflowPayload> {
        let engine = ctx.data::<Arc<WorkflowEngine>>()?;
        
        let execution_request = ExecutionRequest {
            workflow_id: WorkflowId::from_str(&workflow_id)?,
            input: input.unwrap_or(json!({})),
            trigger: TriggerInfo::Manual {
                user: ctx.data::<User>()?.clone(),
            },
            parent_execution: None,
        };
        
        let handle = engine.create_execution(execution_request).await?;
        
        Ok(ExecuteWorkflowPayload {
            execution_id: handle.execution_id,
            status: ExecutionStatus::Created,
        })
    }
}

#[derive(Default)]
pub struct Subscription;

#[Subscription]
impl Subscription {
    async fn execution_updates(
        &self,
        ctx: &Context<'_>,
        execution_id: ID,
    ) -> impl Stream<Item = ExecutionUpdate> {
        let event_bus = ctx.data::<Arc<dyn EventBus>>()?;
        let execution_id = ExecutionId::from_str(&execution_id)?;
        
        let stream = event_bus
            .subscribe(&format!("execution.{}", execution_id))
            .await?
            .filter_map(move |event| {
                match event {
                    Event::ExecutionUpdate(update) if update.execution_id == execution_id => {
                        Some(update)
                    }
                    _ => None,
                }
            });
            
        Ok(stream)
    }
    
    async fn workflow_logs(
        &self,
        ctx: &Context<'_>,
        workflow_id: ID,
    ) -> impl Stream<Item = LogEntry> {
        let event_bus = ctx.data::<Arc<dyn EventBus>>()?;
        let workflow_id = WorkflowId::from_str(&workflow_id)?;
        
        let stream = event_bus
            .subscribe(&format!("logs.workflow.{}", workflow_id))
            .await?
            .filter_map(move |event| {
                match event {
                    Event::LogEntry(entry) if entry.workflow_id == Some(workflow_id) => {
                        Some(entry)
                    }
                    _ => None,
                }
            });
            
        Ok(stream)
    }
}
```

</details>

### WebSocket Handler

```rust
pub struct WebSocketHandler {
    sessions: Arc<DashMap<SessionId, WebSocketSession>>,
    event_bus: Arc<dyn EventBus>,
    auth_service: Arc<AuthService>,
}

pub struct WebSocketSession {
    id: SessionId,
    user: User,
    subscriptions: Vec<Subscription>,
    sender: mpsc::UnboundedSender<Message>,
}

impl WebSocketHandler {
    pub async fn handle_connection(
        &self,
        ws: WebSocket,
        user: User,
    ) -> Result<(), Error> {
        let session_id = SessionId::new();
        let (tx, rx) = mpsc::unbounded_channel();
        let (ws_sender, mut ws_receiver) = ws.split();
        
        // Create session
        let session = WebSocketSession {
            id: session_id.clone(),
            user,
            subscriptions: Vec::new(),
            sender: tx,
        };
        
        self.sessions.insert(session_id.clone(), session);
        
        // Spawn sender task
        let sender_task = tokio::spawn(
            rx.forward(ws_sender).map(|_| ())
        );
        
        // Handle incoming messages
        while let Some(msg) = ws_receiver.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    self.handle_message(&session_id, text).await?;
                }
                Ok(Message::Binary(bin)) => {
                    self.handle_binary(&session_id, bin).await?;
                }
                Ok(Message::Close(_)) => break,
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }
        
        // Cleanup
        self.cleanup_session(&session_id).await?;
        sender_task.abort();
        
        Ok(())
    }
    
    async fn handle_message(
        &self,
        session_id: &SessionId,
        text: String,
    ) -> Result<(), Error> {
        let message: WsMessage = serde_json::from_str(&text)?;
        
        match message {
            WsMessage::Subscribe { channel } => {
                self.handle_subscribe(session_id, channel).await?;
            }
            WsMessage::Unsubscribe { channel } => {
                self.handle_unsubscribe(session_id, channel).await?;
            }
            WsMessage::Execute { workflow_id, input } => {
                self.handle_execute(session_id, workflow_id, input).await?;
            }
            WsMessage::Ping => {
                self.send_to_session(session_id, WsMessage::Pong).await?;
            }
        }
        
        Ok(())
    }
}
```

### Authentication

```rust
pub struct AuthService {
    jwt_validator: Arc<JwtValidator>,
    api_key_store: Arc<ApiKeyStore>,
    oauth_provider: Arc<OAuthProvider>,
}

#[async_trait]
pub trait Authenticator: Send + Sync {
    async fn authenticate(&self, request: &Request) -> Result<AuthContext, Error>;
}

pub struct AuthContext {
    pub user: User,
    pub method: AuthMethod,
    pub permissions: Vec<Permission>,
}

pub enum AuthMethod {
    Jwt,
    ApiKey,
    OAuth,
    Basic,
}

impl AuthService {
    pub async fn authenticate(
        &self,
        request: &Request<Body>,
    ) -> Result<AuthContext, Error> {
        // Try JWT first
        if let Some(token) = extract_bearer_token(request) {
            if let Ok(claims) = self.jwt_validator.validate(token).await {
                return Ok(AuthContext {
                    user: User::from_claims(claims)?,
                    method: AuthMethod::Jwt,
                    permissions: claims.permissions,
                });
            }
        }
        
        // Try API key
        if let Some(api_key) = extract_api_key(request) {
            if let Some(key_info) = self.api_key_store.get_key(api_key).await? {
                return Ok(AuthContext {
                    user: key_info.user,
                    method: AuthMethod::ApiKey,
                    permissions: key_info.permissions,
                });
            }
        }
        
        // Try OAuth
        if let Some(oauth_token) = extract_oauth_token(request) {
            if let Ok(user_info) = self.oauth_provider.get_user_info(oauth_token).await {
                return Ok(AuthContext {
                    user: User::from_oauth(user_info)?,
                    method: AuthMethod::OAuth,
                    permissions: vec![Permission::Read],
                });
            }
        }
        
        Err(Error::Unauthorized)
    }
}
```

### Rate Limiting

```rust
pub struct RateLimiter {
    store: Arc<RateLimitStore>,
    config: RateLimitConfig,
}

pub struct RateLimitConfig {
    pub default_limit: u32,
    pub window: Duration,
    pub burst_size: u32,
    pub custom_limits: HashMap<String, RateLimit>,
}

pub struct RateLimit {
    pub requests: u32,
    pub window: Duration,
    pub burst: u32,
}

impl RateLimiter {
    pub async fn check_rate_limit(
        &self,
        key: &str,
        cost: u32,
    ) -> Result<RateLimitStatus, Error> {
        let limit = self.get_limit_for_key(key);
        let window_start = Utc::now() - limit.window;
        
        // Get current usage
        let usage = self.store
            .get_usage(key, window_start)
            .await?;
            
        if usage + cost > limit.requests {
            return Ok(RateLimitStatus::Exceeded {
                limit: limit.requests,
                remaining: 0,
                reset_at: window_start + limit.window,
            });
        }
        
        // Record usage
        self.store.record_usage(key, cost).await?;
        
        Ok(RateLimitStatus::Allowed {
            limit: limit.requests,
            remaining: limit.requests - usage - cost,
            reset_at: window_start + limit.window,
        })
    }
}
```

### API Documentation

```rust
pub struct ApiDocumentation {
    openapi: OpenApi,
    examples: HashMap<String, Example>,
}

impl ApiDocumentation {
    pub fn generate() -> Self {
        let mut openapi = OpenApi {
            openapi: "3.0.0".to_string(),
            info: Info {
                title: "Nebula Workflow API".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                description: Some("Workflow automation engine API".to_string()),
                ..Default::default()
            },
            servers: vec![Server {
                url: "/api/v1".to_string(),
                description: Some("API v1".to_string()),
                ..Default::default()
            }],
            ..Default::default()
        };
        
        // Add paths
        openapi.paths.insert(
            "/workflows".to_string(),
            PathItem {
                get: Some(Operation {
                    summary: Some("List workflows".to_string()),
                    operation_id: Some("listWorkflows".to_string()),
                    parameters: vec![
                        Parameter::Query {
                            name: "limit".to_string(),
                            required: false,
                            schema: Schema::Integer {
                                default: Some(20),
                                minimum: Some(1),
                                maximum: Some(100),
                            },
                        },
                    ],
                    responses: Responses {
                        responses: btreemap! {
                            "200".to_string() => Response {
                                description: "List of workflows".to_string(),
