# Resource Lifecycle Management Pattern

## Overview

Resource lifecycle management ensures proper initialization, usage, maintenance, and cleanup of resources throughout their lifetime. This pattern prevents resource leaks, ensures consistency, and enables graceful shutdowns.

## Lifecycle States

```
        ┌──────────┐
        │ Created  │
        └────┬─────┘
             │ initialize()
        ┌────▼─────┐
        │Initializing│
        └────┬─────┘
             │ success
        ┌────▼─────┐
        │  Ready   │◄─────────┐
        └────┬─────┘          │
             │ acquire()       │ release()
        ┌────▼─────┐          │
        │  In Use  │──────────┘
        └────┬─────┘
             │ drain()
        ┌────▼─────┐
        │ Draining │
        └────┬─────┘
             │ cleanup()
        ┌────▼─────┐
        │Terminated│
        └──────────┘
```

## Implementation

### Core Lifecycle Manager

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use std::time::{Duration, Instant};
use async_trait::async_trait;

/// Resource lifecycle state
#[derive(Debug, Clone, PartialEq)]
pub enum LifecycleState {
    /// Resource created but not initialized
    Created {
        created_at: Instant,
    },
    
    /// Resource is being initialized
    Initializing {
        started_at: Instant,
    },
    
    /// Resource is ready for use
    Ready {
        initialized_at: Instant,
        last_health_check: Option<Instant>,
    },
    
    /// Resource is currently in use
    InUse {
        acquired_at: Instant,
        acquired_by: String,
        lease_expires: Option<Instant>,
    },
    
    /// Resource is idle
    Idle {
        since: Instant,
        last_used: Option<Instant>,
    },
    
    /// Resource is being maintained
    Maintenance {
        started_at: Instant,
        reason: String,
        estimated_completion: Option<Instant>,
    },
    
    /// Resource is draining (preparing for shutdown)
    Draining {
        started_at: Instant,
        reason: String,
        active_operations: usize,
    },
    
    /// Resource is being cleaned up
    Cleanup {
        started_at: Instant,
    },
    
    /// Resource has been terminated
    Terminated {
        terminated_at: Instant,
        reason: String,
    },
    
    /// Resource is in error state
    Failed {
        failed_at: Instant,
        error: String,
        recoverable: bool,
    },
}

/// Lifecycle manager for resources
pub struct LifecycleManager<R: ManagedResource> {
    /// The resource being managed
    resource: Arc<R>,
    
    /// Current lifecycle state
    state: Arc<RwLock<LifecycleState>>,
    
    /// Lifecycle hooks
    hooks: Arc<RwLock<Vec<Box<dyn LifecycleHook<R>>>>>,
    
    /// Configuration
    config: LifecycleConfig,
    
    /// Metrics
    metrics: Arc<LifecycleMetrics>,
    
    /// State history
    history: Arc<RwLock<StateHistory>>,
}

#[derive(Debug, Clone)]
pub struct LifecycleConfig {
    /// Initialization timeout
    pub init_timeout: Duration,
    
    /// Maximum initialization retries
    pub init_max_retries: u32,
    
    /// Idle timeout before cleanup
    pub idle_timeout: Option<Duration>,
    
    /// Lease duration for acquired resources
    pub lease_duration: Option<Duration>,
    
    /// Enable automatic recovery
    pub auto_recovery: bool,
    
    /// Drain timeout
    pub drain_timeout: Duration,
    
    /// Cleanup timeout
    pub cleanup_timeout: Duration,
}

/// Trait for managed resources
#[async_trait]
pub trait ManagedResource: Send + Sync + 'static {
    /// Initialize the resource
    async fn initialize(&self) -> Result<()>;
    
    /// Validate resource is ready
    async fn validate(&self) -> Result<()>;
    
    /// Perform maintenance
    async fn maintain(&self) -> Result<()>;
    
    /// Drain the resource
    async fn drain(&self) -> Result<()>;
    
    /// Cleanup the resource
    async fn cleanup(&self) -> Result<()>;
    
    /// Get resource metadata
    fn metadata(&self) -> ResourceMetadata;
}

impl<R: ManagedResource> LifecycleManager<R> {
    /// Create new lifecycle manager
    pub fn new(resource: R, config: LifecycleConfig) -> Self {
        Self {
            resource: Arc::new(resource),
            state: Arc::new(RwLock::new(LifecycleState::Created {
                created_at: Instant::now(),
            })),
            hooks: Arc::new(RwLock::new(Vec::new())),
            config,
            metrics: Arc::new(LifecycleMetrics::new()),
            history: Arc::new(RwLock::new(StateHistory::new())),
        }
    }
    
    /// Initialize the resource
    pub async fn initialize(&self) -> Result<()> {
        // Check current state
        let current = self.state.read().await.clone();
        if !matches!(current, LifecycleState::Created { .. } | LifecycleState::Failed { recoverable: true, .. }) {
            return Err(Error::InvalidStateTransition {
                from: current,
                to: "Initializing".to_string(),
            });
        }
        
        // Transition to initializing
        self.transition_to(LifecycleState::Initializing {
            started_at: Instant::now(),
        }).await?;
        
        // Execute pre-initialization hooks
        self.execute_hooks(HookPoint::PreInitialize).await?;
        
        // Initialize with retries
        let mut retries = 0;
        loop {
            match timeout(self.config.init_timeout, self.resource.initialize()).await {
                Ok(Ok(())) => {
                    // Validate resource
                    self.resource.validate().await?;
                    
                    // Transition to ready
                    self.transition_to(LifecycleState::Ready {
                        initialized_at: Instant::now(),
                        last_health_check: None,
                    }).await?;
                    
                    // Execute post-initialization hooks
                    self.execute_hooks(HookPoint::PostInitialize).await?;
                    
                    self.metrics.record_initialization(Instant::now());
                    return Ok(());
                }
                Ok(Err(e)) if retries < self.config.init_max_retries => {
                    retries += 1;
                    warn!("Initialization failed (attempt {}/{}): {}", 
                          retries, self.config.init_max_retries, e);
                    tokio::time::sleep(Duration::from_secs(retries as u64)).await;
                }
                Ok(Err(e)) => {
                    self.transition_to(LifecycleState::Failed {
                        failed_at: Instant::now(),
                        error: e.to_string(),
                        recoverable: false,
                    }).await?;
                    return Err(e);
                }
                Err(_) => {
                    let error = Error::InitializationTimeout;
                    self.transition_to(LifecycleState::Failed {
                        failed_at: Instant::now(),
                        error: error.to_string(),
                        recoverable: true,
                    }).await?;
                    return Err(error);
                }
            }
        }
    }
    
    /// Acquire the resource for use
    pub async fn acquire(&self, acquired_by: String) -> Result<ResourceGuard<R>> {
        let mut state = self.state.write().await;
        
        match &*state {
            LifecycleState::Ready { .. } | LifecycleState::Idle { .. } => {
                let lease_expires = self.config.lease_duration
                    .map(|d| Instant::now() + d);
                
                *state = LifecycleState::InUse {
                    acquired_at: Instant::now(),
                    acquired_by: acquired_by.clone(),
                    lease_expires,
                };
                
                self.metrics.record_acquisition();
                
                Ok(ResourceGuard {
                    resource: self.resource.clone(),
                    manager: self.clone(),
                    acquired_by,
                })
            }
            LifecycleState::InUse { lease_expires: Some(expires), .. } if *expires < Instant::now() => {
                // Lease expired, force release and re-acquire
                warn!("Force releasing expired lease");
                *state = LifecycleState::InUse {
                    acquired_at: Instant::now(),
                    acquired_by: acquired_by.clone(),
                    lease_expires: self.config.lease_duration
                        .map(|d| Instant::now() + d),
                };
                
                Ok(ResourceGuard {
                    resource: self.resource.clone(),
                    manager: self.clone(),
                    acquired_by,
                })
            }
            _ => Err(Error::ResourceUnavailable {
                state: format!("{:?}", state),
            }),
        }
    }
    
    /// Release the resource
    pub async fn release(&self, acquired_by: &str) -> Result<()> {
        let mut state = self.state.write().await;
        
        match &*state {
            LifecycleState::InUse { acquired_by: owner, .. } if owner == acquired_by => {
                *state = LifecycleState::Idle {
                    since: Instant::now(),
                    last_used: Some(Instant::now()),
                };
                
                self.metrics.record_release();
                
                // Start idle timer if configured
                if let Some(idle_timeout) = self.config.idle_timeout {
                    self.start_idle_timer(idle_timeout).await;
                }
                
                Ok(())
            }
            _ => Err(Error::InvalidRelease),
        }
    }
    
    /// Perform maintenance on the resource
    pub async fn maintain(&self, reason: String) -> Result<()> {
        // Transition to maintenance
        self.transition_to(LifecycleState::Maintenance {
            started_at: Instant::now(),
            reason: reason.clone(),
            estimated_completion: None,
        }).await?;
        
        // Perform maintenance
        self.resource.maintain().await?;
        
        // Validate resource after maintenance
        self.resource.validate().await?;
        
        // Transition back to ready
        self.transition_to(LifecycleState::Ready {
            initialized_at: Instant::now(),
            last_health_check: Some(Instant::now()),
        }).await?;
        
        Ok(())
    }
    
    /// Start draining the resource
    pub async fn drain(&self, reason: String) -> Result<()> {
        self.transition_to(LifecycleState::Draining {
            started_at: Instant::now(),
            reason,
            active_operations: 0,
        }).await?;
        
        // Execute pre-drain hooks
        self.execute_hooks(HookPoint::PreDrain).await?;
        
        // Drain the resource
        match timeout(self.config.drain_timeout, self.resource.drain()).await {
            Ok(Ok(())) => {
                self.execute_hooks(HookPoint::PostDrain).await?;
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(Error::DrainTimeout),
        }
    }
    
    /// Cleanup and terminate the resource
    pub async fn cleanup(&self, reason: String) -> Result<()> {
        // Drain first if not already draining
        let current = self.state.read().await.clone();
        if !matches!(current, LifecycleState::Draining { .. }) {
            self.drain(reason.clone()).await?;
        }
        
        // Transition to cleanup
        self.transition_to(LifecycleState::Cleanup {
            started_at: Instant::now(),
        }).await?;
        
        // Execute pre-cleanup hooks
        self.execute_hooks(HookPoint::PreCleanup).await?;
        
        // Cleanup the resource
        match timeout(self.config.cleanup_timeout, self.resource.cleanup()).await {
            Ok(Ok(())) => {
                // Transition to terminated
                self.transition_to(LifecycleState::Terminated {
                    terminated_at: Instant::now(),
                    reason,
                }).await?;
                
                // Execute post-cleanup hooks
                self.execute_hooks(HookPoint::PostCleanup).await?;
                
                self.metrics.record_termination();
                Ok(())
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(Error::CleanupTimeout),
        }
    }
    
    /// Transition to a new state
    async fn transition_to(&self, new_state: LifecycleState) -> Result<()> {
        let mut state = self.state.write().await;
        let old_state = state.clone();
        
        // Validate transition
        if !self.is_valid_transition(&old_state, &new_state) {
            return Err(Error::InvalidStateTransition {
                from: old_state,
                to: format!("{:?}", new_state),
            });
        }
        
        // Update state
        *state = new_state.clone();
        
        // Record in history
        self.history.write().await.record(StateTransition {
            from: old_state,
            to: new_state,
            timestamp: Instant::now(),
        });
        
        Ok(())
    }
    
    /// Check if state transition is valid
    fn is_valid_transition(&self, from: &LifecycleState, to: &LifecycleState) -> bool {
        use LifecycleState::*;
        
        matches!(
            (from, to),
            (Created { .. }, Initializing { .. }) |
            (Initializing { .. }, Ready { .. }) |
            (Initializing { .. }, Failed { .. }) |
            (Ready { .. }, InUse { .. }) |
            (Ready { .. }, Maintenance { .. }) |
            (Ready { .. }, Draining { .. }) |
            (InUse { .. }, Idle { .. }) |
            (InUse { .. }, Failed { .. }) |
            (Idle { .. }, InUse { .. }) |
            (Idle { .. }, Maintenance { .. }) |
            (Idle { .. }, Draining { .. }) |
            (Maintenance { .. }, Ready { .. }) |
            (Maintenance { .. }, Failed { .. }) |
            (Draining { .. }, Cleanup { .. }) |
            (Cleanup { .. }, Terminated { .. }) |
            (Failed { recoverable: true, .. }, Initializing { .. })
        )
    }
}
```

### Resource Guard

```rust
/// RAII guard for acquired resources
pub struct ResourceGuard<R: ManagedResource> {
    resource: Arc<R>,
    manager: LifecycleManager<R>,
    acquired_by: String,
}

impl<R: ManagedResource> ResourceGuard<R> {
    /// Get the resource
    pub fn get(&self) -> &R {
        &self.resource
    }
    
    /// Extend lease
    pub async fn extend_lease(&self, duration: Duration) -> Result<()> {
        let mut state = self.manager.state.write().await;
        
        if let LifecycleState::InUse { lease_expires, .. } = &mut *state {
            *lease_expires = Some(Instant::now() + duration);
            Ok(())
        } else {
            Err(Error::InvalidState)
        }
    }
}

impl<R: ManagedResource> Drop for ResourceGuard<R> {
    fn drop(&mut self) {
        // Release resource on drop
        let manager = self.manager.clone();
        let acquired_by = self.acquired_by.clone();
        
        tokio::spawn(async move {
            if let Err(e) = manager.release(&acquired_by).await {
                error!("Failed to release resource: {}", e);
            }
        });
    }
}
```

### Lifecycle Hooks

```rust
/// Hook points in resource lifecycle
#[derive(Debug, Clone)]
pub enum HookPoint {
    PreInitialize,
    PostInitialize,
    PreAcquire,
    PostAcquire,
    PreRelease,
    PostRelease,
    PreMaintenance,
    PostMaintenance,
    PreDrain,
    PostDrain,
    PreCleanup,
    PostCleanup,
    OnStateChange,
    OnError,
}

/// Lifecycle hook trait
#[async_trait]
pub trait LifecycleHook<R: ManagedResource>: Send + Sync {
    /// Execute hook
    async fn execute(
        &self,
        resource: &R,
        point: HookPoint,
        context: &HookContext,
    ) -> Result<()>;
    
    /// Hook name
    fn name(&self) -> &str;
    
    /// Hook is enabled
    fn is_enabled(&self) -> bool {
        true
    }
}

/// Logging lifecycle hook
pub struct LoggingHook {
    logger: Arc<Logger>,
}

#[async_trait]
impl<R: ManagedResource> LifecycleHook<R> for LoggingHook {
    async fn execute(
        &self,
        resource: &R,
        point: HookPoint,
        context: &HookContext,
    ) -> Result<()> {
        self.logger.log(LogLevel::Info, &format!(
            "Lifecycle event: {:?} for resource: {:?}",
            point,
            resource.metadata().id
        ));
        Ok(())
    }
    
    fn name(&self) -> &str {
        "logging"
    }
}

/// Metrics lifecycle hook
pub struct MetricsHook {
    metrics: Arc<MetricsCollector>,
}

#[async_trait]
impl<R: ManagedResource> LifecycleHook<R> for MetricsHook {
    async fn execute(
        &self,
        resource: &R,
        point: HookPoint,
        _context: &HookContext,
    ) -> Result<()> {
        self.metrics.record_lifecycle_event(
            &resource.metadata().id,
            &point,
        );
        Ok(())
    }
    
    fn name(&self) -> &str {
        "metrics"
    }
}
```

### Automatic Lifecycle Management

```rust
/// Automatic lifecycle manager
pub struct AutoLifecycleManager {
    /// Resources under management
    resources: Arc<RwLock<HashMap<ResourceId, Arc<dyn ManagedResource>>>>,
    
    /// Lifecycle managers
    managers: Arc<RwLock<HashMap<ResourceId, Arc<LifecycleManager<dyn ManagedResource>>>>>,
    
    /// Configuration
    config: AutoLifecycleConfig,
}

#[derive(Debug, Clone)]
pub struct AutoLifecycleConfig {
    /// Auto-initialize on registration
    pub auto_initialize: bool,
    
    /// Auto-cleanup on idle timeout
    pub auto_cleanup_idle: bool,
    
    /// Idle timeout duration
    pub idle_timeout: Duration,
    
    /// Auto-recover from failures
    pub auto_recover: bool,
    
    /// Recovery backoff strategy
    pub recovery_backoff: BackoffStrategy,
    
    /// Health check interval
    pub health_check_interval: Duration,
}

impl AutoLifecycleManager {
    /// Start automatic management
    pub async fn start(self: Arc<Self>) {
        // Start health check loop
        let health_manager = self.clone();
        tokio::spawn(async move {
            health_manager.health_check_loop().await;
        });
        
        // Start cleanup loop
        let cleanup_manager = self.clone();
        tokio::spawn(async move {
            cleanup_manager.cleanup_loop().await;
        });
        
        // Start recovery loop
        if self.config.auto_recover {
            let recovery_manager = self.clone();
            tokio::spawn(async move {
                recovery_manager.recovery_loop().await;
            });
        }
    }
    
    /// Health check loop
    async fn health_check_loop(&self) {
        let mut interval = tokio::time::interval(self.config.health_check_interval);
        
        loop {
            interval.tick().await;
            
            let managers = self.managers.read().await;
            for (id, manager) in managers.iter() {
                let state = manager.state.read().await.clone();
                
                if
```