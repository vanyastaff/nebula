---

# nebula-runtime

## Purpose

`nebula-runtime` manages the lifecycle of workflow triggers, coordinates workflow activations, and handles the event-driven aspects of the system.

## Responsibilities

- Trigger lifecycle management
- Event listening and processing
- Workflow activation/deactivation
- Runtime coordination
- Health monitoring
- Resource allocation for triggers

## Architecture

### Core Components

```rust
pub struct Runtime {
    // Unique identifier for this runtime instance
    id: RuntimeId,
    
    // Trigger management
    trigger_manager: Arc<TriggerManager>,
    
    // Event processing
    event_processor: Arc<EventProcessor>,
    
    // Workflow coordination
    coordinator: Arc<WorkflowCoordinator>,
    
    // Health monitoring
    health_monitor: Arc<HealthMonitor>,
    
    // Resource management
    resource_manager: Arc<ResourceManager>,
    
    // Metrics
    metrics: Arc<RuntimeMetrics>,
}

pub struct RuntimeConfig {
    pub id: RuntimeId,
    pub event_bus_config: EventBusConfig,
    pub trigger_config: TriggerConfig,
    pub coordination_config: CoordinationConfig,
    pub resource_limits: ResourceLimits,
}
```

### Trigger Management

```rust
pub struct TriggerManager {
    // Active triggers indexed by workflow ID
    active_triggers: Arc<DashMap<WorkflowId, Vec<ActiveTrigger>>>,
    
    // Trigger registry
    registry: Arc<TriggerRegistry>,
    
    // Lifecycle manager
    lifecycle: Arc<TriggerLifecycle>,
    
    // State persistence
    state_store: Arc<dyn TriggerStateStore>,
}

pub struct ActiveTrigger {
    pub id: TriggerId,
    pub workflow_id: WorkflowId,
    pub trigger_type: TriggerType,
    pub instance: Box<dyn TriggerAction>,
    pub status: TriggerStatus,
    pub handle: TriggerHandle,
    pub created_at: DateTime<Utc>,
    pub last_fired: Option<DateTime<Utc>>,
}

pub enum TriggerStatus {
    Initializing,
    Active,
    Paused,
    Failed { error: String, retry_count: u32 },
    Stopping,
    Stopped,
}

impl TriggerManager {
    pub async fn activate_trigger(
        &self,
        workflow_id: &WorkflowId,
        trigger_def: &TriggerDefinition,
    ) -> Result<TriggerId, Error> {
        // Create trigger instance
        let instance = self.registry
            .create_trigger(&trigger_def.trigger_type, trigger_def.config.clone())?;
            
        // Initialize trigger
        let mut trigger_instance = instance;
        let context = self.create_trigger_context(workflow_id).await?;
        let handle = trigger_instance.initialize(&context).await?;
        
        // Create active trigger
        let trigger_id = TriggerId::new();
        let active_trigger = ActiveTrigger {
            id: trigger_id.clone(),
            workflow_id: workflow_id.clone(),
            trigger_type: trigger_def.trigger_type.clone(),
            instance: trigger_instance,
            status: TriggerStatus::Active,
            handle,
            created_at: Utc::now(),
            last_fired: None,
        };
        
        // Store and start
        self.active_triggers
            .entry(workflow_id.clone())
            .or_insert_with(Vec::new)
            .push(active_trigger);
            
        // Start listening
        self.lifecycle.start_trigger(&trigger_id).await?;
        
        Ok(trigger_id)
    }
    
    pub async fn deactivate_workflow_triggers(
        &self,
        workflow_id: &WorkflowId,
    ) -> Result<(), Error> {
        if let Some((_, triggers)) = self.active_triggers.remove(workflow_id) {
            for trigger in triggers {
                self.lifecycle.stop_trigger(&trigger.id).await?;
            }
        }
        
        Ok(())
    }
}
```

### Trigger Types Implementation

```rust
// HTTP Webhook Trigger
pub struct WebhookTrigger {
    config: WebhookConfig,
    endpoint: String,
    auth: Option<WebhookAuth>,
}

#[async_trait]
impl TriggerAction for WebhookTrigger {
    async fn initialize(&mut self, ctx: &TriggerContext) -> Result<TriggerHandle, Error> {
        // Register webhook endpoint
        let endpoint_id = ctx.webhook_registry()
            .register_endpoint(&self.endpoint, ctx.workflow_id())
            .await?;
            
        Ok(TriggerHandle::Webhook(endpoint_id))
    }
    
    async fn listen(&mut self, handle: &TriggerHandle) -> Result<TriggerStream, Error> {
        let (tx, rx) = mpsc::channel(100);
        
        // Subscribe to webhook events
        let mut subscription = ctx.webhook_registry()
            .subscribe(handle.as_webhook_id()?)
            .await?;
            
        tokio::spawn(async move {
            while let Some(event) = subscription.next().await {
                if let Err(_) = tx.send(TriggerEvent::from(event)).await {
                    break;
                }
            }
        });
        
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
    
    async fn shutdown(&mut self, handle: TriggerHandle) -> Result<(), Error> {
        ctx.webhook_registry()
            .unregister_endpoint(handle.as_webhook_id()?)
            .await
    }
}

// Kafka Trigger
pub struct KafkaTrigger {
    config: KafkaConfig,
    consumer: Option<StreamConsumer>,
}

#[async_trait]
impl TriggerAction for KafkaTrigger {
    async fn initialize(&mut self, ctx: &TriggerContext) -> Result<TriggerHandle, Error> {
        let consumer = ClientConfig::new()
            .set("bootstrap.servers", &self.config.brokers)
            .set("group.id", &format!("nebula-{}", ctx.workflow_id()))
            .set("enable.auto.commit", "false")
            .create::<StreamConsumer>()?;
            
        consumer.subscribe(&[&self.config.topic])?;
        
        self.consumer = Some(consumer);
        
        Ok(TriggerHandle::Kafka {
            topic: self.config.topic.clone(),
            group_id: format!("nebula-{}", ctx.workflow_id()),
        })
    }
    
    async fn listen(&mut self, _handle: &TriggerHandle) -> Result<TriggerStream, Error> {
        let consumer = self.consumer.as_ref().ok_or(Error::NotInitialized)?;
        let stream = consumer.stream();
        
        let trigger_stream = stream.map(|result| {
            match result {
                Ok(message) => {
                    let payload = message.payload()
                        .map(|p| String::from_utf8_lossy(p).to_string())
                        .unwrap_or_default();
                        
                    Ok(TriggerEvent {
                        id: Uuid::new_v4(),
                        timestamp: Utc::now(),
                        data: json!({ "message": payload }),
                        metadata: Default::default(),
                    })
                }
                Err(e) => Err(Error::Kafka(e)),
            }
        });
        
        Ok(Box::pin(trigger_stream))
    }
}

// Scheduled Trigger
pub struct ScheduledTrigger {
    config: ScheduleConfig,
    schedule: Schedule,
}

#[async_trait]
impl TriggerAction for ScheduledTrigger {
    async fn initialize(&mut self, _ctx: &TriggerContext) -> Result<TriggerHandle, Error> {
        self.schedule = Schedule::from_str(&self.config.cron_expression)?;
        
        Ok(TriggerHandle::Schedule {
            expression: self.config.cron_expression.clone(),
        })
    }
    
    async fn listen(&mut self, _handle: &TriggerHandle) -> Result<TriggerStream, Error> {
        let schedule = self.schedule.clone();
        let (tx, rx) = mpsc::channel(10);
        
        tokio::spawn(async move {
            let mut next_time = schedule.upcoming(Utc).next().unwrap();
            
            loop {
                let now = Utc::now();
                if now >= next_time {
                    let event = TriggerEvent {
                        id: Uuid::new_v4(),
                        timestamp: now,
                        data: json!({ "scheduled_time": next_time }),
                        metadata: Default::default(),
                    };
                    
                    if tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                    
                    next_time = schedule.upcoming(Utc).next().unwrap();
                } else {
                    tokio::time::sleep_until(next_time.into()).await;
                }
            }
        });
        
        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}
```

### Event Processing

```rust
pub struct EventProcessor {
    event_bus: Arc<dyn EventBus>,
    handlers: Arc<RwLock<HashMap<String, Vec<Box<dyn EventHandler>>>>>,
    processor_threads: Vec<JoinHandle<()>>,
}

#[async_trait]
pub trait EventHandler: Send + Sync {
    async fn handle(&self, event: &RuntimeEvent) -> Result<(), Error>;
    fn event_type(&self) -> &str;
}

pub enum RuntimeEvent {
    TriggerFired {
        trigger_id: TriggerId,
        workflow_id: WorkflowId,
        event: TriggerEvent,
    },
    
    WorkflowDeployed {
        workflow_id: WorkflowId,
        version: Version,
    },
    
    WorkflowActivated {
        workflow_id: WorkflowId,
    },
    
    WorkflowDeactivated {
        workflow_id: WorkflowId,
    },
    
    TriggerFailed {
        trigger_id: TriggerId,
        error: Error,
    },
    
    RuntimeStarted {
        runtime_id: RuntimeId,
    },
    
    RuntimeStopping {
        runtime_id: RuntimeId,
    },
}

impl EventProcessor {
    pub async fn start(&self, num_threads: usize) -> Result<(), Error> {
        for i in 0..num_threads {
            let event_bus = self.event_bus.clone();
            let handlers = self.handlers.clone();
            
            let handle = tokio::spawn(async move {
                let mut subscription = event_bus.subscribe("runtime.*").await.unwrap();
                
                while let Some(event) = subscription.next().await {
                    if let Err(e) = Self::process_event(event, &handlers).await {
                        error!("Event processing error: {}", e);
                    }
                }
            });
            
            self.processor_threads.push(handle);
        }
        
        Ok(())
    }
    
    async fn process_event(
        event: RuntimeEvent,
        handlers: &Arc<RwLock<HashMap<String, Vec<Box<dyn EventHandler>>>>>,
    ) -> Result<(), Error> {
        let event_type = event.event_type();
        let handlers = handlers.read().await;
        
        if let Some(event_handlers) = handlers.get(event_type) {
            for handler in event_handlers {
                handler.handle(&event).await?;
            }
        }
        
        Ok(())
    }
}
```

### Workflow Coordination

```rust
pub struct WorkflowCoordinator {
    // Workflow assignments
    assignments: Arc<DashMap<WorkflowId, RuntimeId>>,
    
    // Coordination strategy
    strategy: Box<dyn CoordinationStrategy>,
    
    // Runtime registry
    runtime_registry: Arc<RuntimeRegistry>,
    
    // Load balancer
    load_balancer: Arc<LoadBalancer>,
}

#[async_trait]
pub trait CoordinationStrategy: Send + Sync {
    async fn assign_workflow(
        &self,
        workflow_id: &WorkflowId,
        runtimes: &[RuntimeInfo],
    ) -> Result<RuntimeId, Error>;
    
    async fn rebalance(
        &self,
        assignments: &HashMap<WorkflowId, RuntimeId>,
        runtimes: &[RuntimeInfo],
    ) -> HashMap<WorkflowId, RuntimeId>;
}

pub struct ConsistentHashStrategy {
    hasher: ConsistentHash<RuntimeId>,
}

impl WorkflowCoordinator {
    pub async fn assign_workflow(&self, workflow_id: &WorkflowId) -> Result<RuntimeId, Error> {
        // Get available runtimes
        let runtimes = self.runtime_registry.get_healthy_runtimes().await?;
        
        if runtimes.is_empty() {
            return Err(Error::NoAvailableRuntime);
        }
        
        // Use strategy to select runtime
        let runtime_id = self.strategy.assign_workflow(workflow_id, &runtimes).await?;
        
        // Store assignment
        self.assignments.insert(workflow_id.clone(), runtime_id.clone());
        
        // Notify runtime
        self.notify_runtime_of_assignment(&runtime_id, workflow_id).await?;
        
        Ok(runtime_id)
    }
    
    pub async fn handle_runtime_failure(&self, failed_runtime: &RuntimeId) -> Result<(), Error> {
        // Find affected workflows
        let affected_workflows: Vec<WorkflowId> = self.assignments
            .iter()
            .filter(|entry| entry.value() == failed_runtime)
            .map(|entry| entry.key().clone())
            .collect();
            
        // Reassign workflows
        for workflow_id in affected_workflows {
            self.reassign_workflow(&workflow_id).await?;
        }
        
        Ok(())
    }
}
```

### Health Monitoring

```rust
pub struct HealthMonitor {
    checks: Vec<Box<dyn HealthCheck>>,
    interval: Duration,
    status: Arc<RwLock<HealthStatus>>,
}

#[async_trait]
pub trait HealthCheck: Send + Sync {
    async fn check(&self) -> ComponentHealth;
    fn component_name(&self) -> &str;
}

pub struct ComponentHealth {
    pub status: HealthState,
    pub message: Option<String>,
    pub metrics: HashMap<String, f64>,
}

pub enum HealthState {
    Healthy,
    Degraded,
    Unhealthy,
}

impl HealthMonitor {
    pub async fn start(&self) {
        let checks = self.checks.clone();
        let status = self.status.clone();
        let interval = self.interval;
        
        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            
            loop {
                interval_timer.tick().await;
                
                let mut overall_status = HealthState::Healthy;
                let mut component_results = HashMap::new();
                
                for check in &checks {
                    let result = check.check().await;
                    
                    match &result.status {
                        HealthState::Unhealthy => overall_status = HealthState::Unhealthy,
                        HealthState::Degraded if matches!(overall_status, HealthState::Healthy) => {
                            overall_status = HealthState::Degraded;
                        }
                        _ => {}
                    }
                    
                    component_results.insert(check.component_name().to_string(), result);
                }
                
                let health_status = HealthStatus {
                    status: overall_status,
                    components: component_results,
                    timestamp: Utc::now(),
                };
                
                *status.write().await = health_status;
            }
        });
    }
}
```

### Resource Management

```rust
pub struct ResourceManager {
    // Resource pools
    pools: HashMap<String, Box<dyn ResourcePool>>,
    
    // Resource limits
    limits: ResourceLimits,
    
    // Usage tracking
    usage: Arc<RwLock<ResourceUsage>>,
}

pub struct ResourceLimits {
    pub max_memory: usize,
    pub max_triggers: usize,
    pub max_connections: usize,
    pub max_cpu_percent: f64,
}

pub struct ResourceUsage {
    pub memory_used: usize,
    pub trigger_count: usize,
    pub connection_count: usize,
    pub cpu_percent: f64,
}

impl ResourceManager {
    pub async fn allocate_trigger_resources(
        &self,
        trigger_type: &TriggerType,
    ) -> Result<TriggerResources, Error> {
        // Check limits
        let usage = self.usage.read().await;
        
        if usage.trigger_count >= self.limits.max_triggers {
            return Err(Error::ResourceLimitExceeded("max_triggers"));
        }
        
        // Estimate resource requirements
        let requirements = self.estimate_trigger_requirements(trigger_type)?;
        
        if usage.memory_used + requirements.memory > self.limits.max_memory {
            return Err(Error::ResourceLimitExceeded("memory"));
        }
        
        // Allocate resources
        let resources = TriggerResources {
            memory_limit: requirements.memory,
            connection_pool: self.get_connection_pool(trigger_type)?,
            rate_limiter: self.create_rate_limiter(trigger_type)?,
        };
        
        // Update usage
        self.usage.write().await.trigger_count += 1;
        self.usage.write().await.memory_used += requirements.memory;
        
        Ok(resources)
    }
}
```

## Runtime Lifecycle

### Startup Process

```rust
impl Runtime {
    pub async fn start(config: RuntimeConfig) -> Result<Self, Error> {
        info!("Starting runtime {}", config.id);
        
        // Initialize components
        let trigger_manager = Arc::new(TriggerManager::new(&config.trigger_config).await?);
        let event_processor = Arc::new(EventProcessor::new(&config.event_bus_config).await?);
        let coordinator = Arc::new(WorkflowCoordinator::new(&config.coordination_config).await?);
        let health_monitor = Arc::new(HealthMonitor::new());
        let resource_manager = Arc::new(ResourceManager::new(config.resource_limits));
        let metrics = Arc::new(RuntimeMetrics::new());
        
        let runtime = Self {
            id: config.id,
            trigger_manager,
            event_processor,
            coordinator,
            health_monitor,
            resource_manager,
            metrics,
        };
        
        // Start components
        runtime.event_processor.start(4).await?;
        runtime.health_monitor.start().await;
        
        // Register with coordinator
        runtime.coordinator.register_runtime(&runtime.id).await?;
        
        // Load assigned workflows
        runtime.load_assigned_workflows().await?;
        
        // Emit started event
        runtime.event_processor.publish(RuntimeEvent::RuntimeStarted {
            runtime_id: runtime.id.clone(),
        }).await?;
        
        Ok(runtime)
    }
    
    async fn load_assigned_workflows(&self) -> Result<(), Error> {
        let assignments = self.coordinator.get_runtime_assignments(&self.id).await?;
        
        for workflow_id in assignments {
            if let Err(e) = self.activate_workflow(&workflow_id).await {
                error!("Failed to activate workflow {}: {}", workflow_id, e);
            }
        }
        
        Ok(())
    }
}
```

### Shutdown Process

```rust
impl Runtime {
    pub async fn shutdown(&self) -> Result<(), Error> {
        info!("Shutting down runtime {}", self.id);
        
        // Emit stopping event
        self.event_processor.publish(RuntimeEvent::RuntimeStopping {
            runtime_id: self.id.clone(),
        }).await?;
        
        // Stop accepting new workflows
        self.coordinator.mark_runtime_draining(&self.id).await?;
        
        // Deactivate all triggers
        let workflows = self.get_active_workflows().await?;
        for workflow_id in workflows {
            self.deactivate_workflow(&workflow_id).await?;
        }
        
        // Stop components
        self.event_processor.stop().await?;
        self.health_monitor.stop().await?;
        
        // Unregister from coordinator
        self.coordinator.unregister_runtime(&self.id).await?;
        
        info!("Runtime {} shutdown complete", self.id);
        
        Ok(())
    }
}
```

## Metrics

```rust
pub struct RuntimeMetrics {
    // Workflow metrics
    workflows_active: Gauge,
    workflows_activated: Counter,
    workflows_deactivated: Counter,
    
    // Trigger metrics
    triggers_active: Gauge,
    triggers_fired: Counter,
    trigger_errors: Counter,
    trigger_latency: Histogram,
    
    // Event metrics
    events_processed: Counter,
    event_processing_duration: Histogram,
    
    // Resource metrics
    memory_usage: Gauge,
    cpu_usage: Gauge,
    connection_pool_size: Gauge,
}

impl RuntimeMetrics {
    pub fn record_trigger_fired(&self, trigger_type: &str, latency: Duration) {
        self.triggers_fired
            .with_label_values(&[trigger_type])
            .increment();
            
        self.trigger_latency
            .with_label_values(&[trigger_type])
            .record(latency.as_secs_f64());
    }
    
    pub fn record_workflow_activated(&self, workflow_id: &WorkflowId) {
        self.workflows_activated.increment();
        self.workflows_active.increment();
        
        debug!("Workflow {} activated", workflow_id);
    }
}
```
