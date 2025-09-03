---
title:  QuarantineRecovery
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---

# Quarantine and Recovery System

## Overview

The quarantine and recovery system in nebula-resource provides automatic isolation of problematic resources, preventing cascading failures while maintaining system stability. Resources can be quarantined based on health checks, error rates, or manual intervention.

## Core Concepts

### Quarantine States

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum QuarantineState {
    /// Resource is operating normally
    Healthy,
    
    /// Resource is experiencing issues but still operational
    Degraded {
        since: Instant,
        reason: String,
        error_rate: f64,
    },
    
    /// Resource is quarantined and not available
    Quarantined {
        since: Instant,
        reason: QuarantineReason,
        recovery_attempts: u32,
        next_retry: Option<Instant>,
    },
    
    /// Resource is being recovered
    Recovering {
        since: Instant,
        progress: f32,
        estimated_completion: Option<Instant>,
    },
}

#[derive(Debug, Clone)]
pub enum QuarantineReason {
    HealthCheckFailed { consecutive_failures: u32 },
    ErrorRateExceeded { rate: f64, threshold: f64 },
    ManualIntervention { reason: String },
    DependencyFailure { dependency: String },
    ResourceExhaustion,
    SecurityViolation,
}
```

## Implementation

### Basic Quarantine Manager

```rust
use nebula_resource::prelude::*;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct QuarantineManager {
    resources: Arc<RwLock<HashMap<ResourceId, QuarantineEntry>>>,
    config: QuarantineConfig,
    recovery_strategies: HashMap<String, Box<dyn RecoveryStrategy>>,
}

struct QuarantineEntry {
    resource: Arc<dyn Resource>,
    state: QuarantineState,
    metrics: QuarantineMetrics,
    history: Vec<QuarantineEvent>,
}

#[derive(Debug, Clone)]
pub struct QuarantineConfig {
    /// Maximum error rate before quarantine
    pub error_threshold: f64,
    
    /// Time window for error rate calculation
    pub error_window: Duration,
    
    /// Consecutive health check failures before quarantine
    pub health_check_failures: u32,
    
    /// Initial recovery delay
    pub initial_recovery_delay: Duration,
    
    /// Maximum recovery attempts
    pub max_recovery_attempts: u32,
    
    /// Backoff multiplier for recovery attempts
    pub backoff_multiplier: f64,
    
    /// Maximum backoff duration
    pub max_backoff: Duration,
    
    /// Enable automatic recovery
    pub auto_recovery: bool,
}

impl Default for QuarantineConfig {
    fn default() -> Self {
        Self {
            error_threshold: 0.5,
            error_window: Duration::from_secs(60),
            health_check_failures: 3,
            initial_recovery_delay: Duration::from_secs(30),
            max_recovery_attempts: 5,
            backoff_multiplier: 2.0,
            max_backoff: Duration::from_secs(300),
            auto_recovery: true,
        }
    }
}
```

### Quarantine Detection

```rust
impl QuarantineManager {
    /// Monitor resource health and trigger quarantine if needed
    pub async fn monitor_resource(&self, resource_id: ResourceId) -> Result<()> {
        let mut resources = self.resources.write().await;
        let entry = resources.get_mut(&resource_id)
            .ok_or_else(|| Error::ResourceNotFound(resource_id))?;
        
        // Check error rate
        if let Some(rate) = entry.metrics.error_rate() {
            if rate > self.config.error_threshold {
                self.quarantine_resource(
                    &mut entry,
                    QuarantineReason::ErrorRateExceeded {
                        rate,
                        threshold: self.config.error_threshold,
                    }
                ).await?;
                return Ok(());
            }
        }
        
        // Check health status
        if let Err(e) = entry.resource.health_check().await {
            entry.metrics.record_health_failure();
            
            if entry.metrics.consecutive_health_failures >= self.config.health_check_failures {
                self.quarantine_resource(
                    &mut entry,
                    QuarantineReason::HealthCheckFailed {
                        consecutive_failures: entry.metrics.consecutive_health_failures,
                    }
                ).await?;
            }
        } else {
            entry.metrics.reset_health_failures();
        }
        
        Ok(())
    }
    
    /// Quarantine a resource
    async fn quarantine_resource(
        &self,
        entry: &mut QuarantineEntry,
        reason: QuarantineReason,
    ) -> Result<()> {
        // Record event
        entry.history.push(QuarantineEvent {
            timestamp: Instant::now(),
            event_type: QuarantineEventType::Quarantined,
            reason: reason.clone(),
        });
        
        // Update state
        entry.state = QuarantineState::Quarantined {
            since: Instant::now(),
            reason: reason.clone(),
            recovery_attempts: 0,
            next_retry: if self.config.auto_recovery {
                Some(Instant::now() + self.config.initial_recovery_delay)
            } else {
                None
            },
        };
        
        // Notify listeners
        self.notify_quarantine(&entry.resource.id(), &reason).await;
        
        // Cleanup resource
        entry.resource.cleanup().await?;
        
        Ok(())
    }
}
```

### Recovery Strategies

```rust
#[async_trait]
pub trait RecoveryStrategy: Send + Sync {
    /// Attempt to recover a quarantined resource
    async fn recover(&self, resource: &dyn Resource) -> Result<RecoveryResult>;
    
    /// Check if recovery should be attempted
    fn should_attempt(&self, attempts: u32, last_failure: Instant) -> bool;
    
    /// Calculate next retry delay
    fn next_retry_delay(&self, attempts: u32) -> Duration;
}

pub enum RecoveryResult {
    /// Recovery successful
    Success,
    
    /// Recovery failed but can retry
    Retry { reason: String },
    
    /// Recovery failed, don't retry
    Failed { reason: String },
    
    /// Need manual intervention
    ManualRequired { reason: String },
}

/// Exponential backoff recovery strategy
pub struct ExponentialBackoffRecovery {
    initial_delay: Duration,
    max_delay: Duration,
    multiplier: f64,
    max_attempts: u32,
}

#[async_trait]
impl RecoveryStrategy for ExponentialBackoffRecovery {
    async fn recover(&self, resource: &dyn Resource) -> Result<RecoveryResult> {
        // Try to initialize the resource
        match resource.initialize().await {
            Ok(_) => {
                // Verify with health check
                match resource.health_check().await {
                    Ok(_) => Ok(RecoveryResult::Success),
                    Err(e) => Ok(RecoveryResult::Retry {
                        reason: format!("Health check failed: {}", e),
                    }),
                }
            }
            Err(e) if e.is_retryable() => {
                Ok(RecoveryResult::Retry {
                    reason: e.to_string(),
                })
            }
            Err(e) => {
                Ok(RecoveryResult::Failed {
                    reason: e.to_string(),
                })
            }
        }
    }
    
    fn should_attempt(&self, attempts: u32, _last_failure: Instant) -> bool {
        attempts < self.max_attempts
    }
    
    fn next_retry_delay(&self, attempts: u32) -> Duration {
        let delay = self.initial_delay.as_secs_f64() * self.multiplier.powi(attempts as i32);
        Duration::from_secs_f64(delay.min(self.max_delay.as_secs_f64()))
    }
}

/// Circuit breaker recovery strategy
pub struct CircuitBreakerRecovery {
    failure_threshold: u32,
    success_threshold: u32,
    timeout: Duration,
    half_open_requests: u32,
}

#[async_trait]
impl RecoveryStrategy for CircuitBreakerRecovery {
    async fn recover(&self, resource: &dyn Resource) -> Result<RecoveryResult> {
        // Implement circuit breaker logic
        // Open -> Half-Open -> Closed state transitions
        todo!()
    }
    
    fn should_attempt(&self, attempts: u32, last_failure: Instant) -> bool {
        // Check if enough time has passed
        last_failure.elapsed() > self.timeout
    }
    
    fn next_retry_delay(&self, _attempts: u32) -> Duration {
        self.timeout
    }
}
```

### Automatic Recovery

```rust
impl QuarantineManager {
    /// Start automatic recovery loop
    pub async fn start_recovery_loop(self: Arc<Self>) {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        
        loop {
            interval.tick().await;
            
            if !self.config.auto_recovery {
                continue;
            }
            
            let resources = self.resources.read().await;
            let quarantined: Vec<_> = resources
                .iter()
                .filter_map(|(id, entry)| {
                    if let QuarantineState::Quarantined { next_retry: Some(retry), .. } = entry.state {
                        if Instant::now() >= retry {
                            return Some(id.clone());
                        }
                    }
                    None
                })
                .collect();
            drop(resources);
            
            for resource_id in quarantined {
                if let Err(e) = self.attempt_recovery(resource_id).await {
                    error!("Recovery failed for resource {}: {}", resource_id, e);
                }
            }
        }
    }
    
    /// Attempt to recover a quarantined resource
    pub async fn attempt_recovery(&self, resource_id: ResourceId) -> Result<()> {
        let mut resources = self.resources.write().await;
        let entry = resources.get_mut(&resource_id)
            .ok_or_else(|| Error::ResourceNotFound(resource_id))?;
        
        // Check if resource is quarantined
        let (reason, attempts) = match &entry.state {
            QuarantineState::Quarantined { reason, recovery_attempts, .. } => {
                (reason.clone(), *recovery_attempts)
            }
            _ => return Ok(()),
        };
        
        // Update state to recovering
        entry.state = QuarantineState::Recovering {
            since: Instant::now(),
            progress: 0.0,
            estimated_completion: None,
        };
        
        // Get recovery strategy
        let strategy = self.get_recovery_strategy(&reason)?;
        
        // Attempt recovery
        match strategy.recover(entry.resource.as_ref()).await? {
            RecoveryResult::Success => {
                entry.state = QuarantineState::Healthy;
                entry.metrics.reset();
                entry.history.push(QuarantineEvent {
                    timestamp: Instant::now(),
                    event_type: QuarantineEventType::Recovered,
                    reason: reason.clone(),
                });
                
                info!("Resource {} recovered successfully", resource_id);
                self.notify_recovery(&resource_id).await;
            }
            RecoveryResult::Retry { reason: retry_reason } => {
                let next_attempts = attempts + 1;
                let next_delay = strategy.next_retry_delay(next_attempts);
                
                entry.state = QuarantineState::Quarantined {
                    since: Instant::now(),
                    reason,
                    recovery_attempts: next_attempts,
                    next_retry: Some(Instant::now() + next_delay),
                };
                
                warn!("Recovery retry scheduled for {}: {}", resource_id, retry_reason);
            }
            RecoveryResult::Failed { reason: fail_reason } => {
                entry.state = QuarantineState::Quarantined {
                    since: Instant::now(),
                    reason,
                    recovery_attempts: attempts + 1,
                    next_retry: None,
                };
                
                error!("Recovery failed permanently for {}: {}", resource_id, fail_reason);
            }
            RecoveryResult::ManualRequired { reason: manual_reason } => {
                entry.state = QuarantineState::Quarantined {
                    since: Instant::now(),
                    reason: QuarantineReason::ManualIntervention {
                        reason: manual_reason.clone(),
                    },
                    recovery_attempts: attempts + 1,
                    next_retry: None,
                };
                
                warn!("Manual intervention required for {}: {}", resource_id, manual_reason);
                self.notify_manual_required(&resource_id, &manual_reason).await;
            }
        }
        
        Ok(())
    }
}
```

### Manual Recovery

```rust
impl QuarantineManager {
    /// Manually recover a resource
    pub async fn manual_recover(&self, resource_id: ResourceId) -> Result<()> {
        let mut resources = self.resources.write().await;
        let entry = resources.get_mut(&resource_id)
            .ok_or_else(|| Error::ResourceNotFound(resource_id))?;
        
        // Force recovery regardless of state
        entry.state = QuarantineState::Recovering {
            since: Instant::now(),
            progress: 0.0,
            estimated_completion: None,
        };
        
        // Initialize resource
        entry.resource.initialize().await?;
        
        // Verify health
        entry.resource.health_check().await?;
        
        // Update state
        entry.state = QuarantineState::Healthy;
        entry.metrics.reset();
        
        entry.history.push(QuarantineEvent {
            timestamp: Instant::now(),
            event_type: QuarantineEventType::ManualRecovery,
            reason: QuarantineReason::ManualIntervention {
                reason: "Manual recovery initiated".to_string(),
            },
        });
        
        Ok(())
    }
    
    /// Force quarantine a resource
    pub async fn manual_quarantine(
        &self,
        resource_id: ResourceId,
        reason: String,
    ) -> Result<()> {
        let mut resources = self.resources.write().await;
        let entry = resources.get_mut(&resource_id)
            .ok_or_else(|| Error::ResourceNotFound(resource_id))?;
        
        self.quarantine_resource(
            entry,
            QuarantineReason::ManualIntervention { reason },
        ).await
    }
}
```

## Advanced Features

### Quarantine Policies

```rust
pub struct QuarantinePolicy {
    /// Conditions that trigger quarantine
    pub triggers: Vec<QuarantineTrigger>,
    
    /// Recovery strategy to use
    pub recovery_strategy: String,
    
    /// Resources to notify on quarantine
    pub notify_on_quarantine: Vec<String>,
    
    /// Auto-escalation rules
    pub escalation: Option<EscalationPolicy>,
}

pub enum QuarantineTrigger {
    ErrorRate { threshold: f64, window: Duration },
    LatencyP99 { threshold: Duration },
    MemoryUsage { threshold_mb: u64 },
    Custom { evaluator: Box<dyn Fn(&ResourceMetrics) -> bool> },
}

pub struct EscalationPolicy {
    /// Time before escalation
    pub after_duration: Duration,
    
    /// Number of recovery failures before escalation
    pub after_failures: u32,
    
    /// Action to take
    pub action: EscalationAction,
}

pub enum EscalationAction {
    NotifyOncall,
    CreateIncident { severity: IncidentSeverity },
    ShutdownResource,
    FailoverToBackup,
}
```

### Quarantine Groups

```rust
/// Group related resources for coordinated quarantine
pub struct QuarantineGroup {
    pub id: String,
    pub resources: Vec<ResourceId>,
    pub policy: GroupQuarantinePolicy,
}

pub enum GroupQuarantinePolicy {
    /// Quarantine all if any fails
    AllOrNone,
    
    /// Quarantine individually
    Independent,
    
    /// Quarantine if percentage fails
    Threshold { percentage: f32 },
    
    /// Custom logic
    Custom { evaluator: Box<dyn Fn(&[ResourceState]) -> Vec<ResourceId>> },
}

impl QuarantineManager {
    pub async fn evaluate_group(&self, group: &QuarantineGroup) -> Result<()> {
        let resources = self.resources.read().await;
        
        let states: Vec<_> = group.resources
            .iter()
            .filter_map(|id| resources.get(id))
            .map(|entry| (&entry.resource.id(), &entry.state))
            .collect();
        
        match group.policy {
            GroupQuarantinePolicy::AllOrNone => {
                if states.iter().any(|(_, state)| matches!(state, QuarantineState::Quarantined { .. })) {
                    // Quarantine all resources in group
                    for resource_id in &group.resources {
                        self.manual_quarantine(
                            resource_id.clone(),
                            format!("Group {} policy: all-or-none", group.id),
                        ).await?;
                    }
                }
            }
            GroupQuarantinePolicy::Threshold { percentage } => {
                let quarantined_count = states
                    .iter()
                    .filter(|(_, state)| matches!(state, QuarantineState::Quarantined { .. }))
                    .count();
                
                let quarantine_percentage = (quarantined_count as f32) / (states.len() as f32);
                
                if quarantine_percentage >= percentage {
                    // Quarantine remaining healthy resources
                    for (id, state) in states {
                        if matches!(state, QuarantineState::Healthy) {
                            self.manual_quarantine(
                                id.clone(),
                                format!("Group {} threshold exceeded: {:.1}%", group.id, quarantine_percentage * 100.0),
                            ).await?;
                        }
                    }
                }
            }
            _ => {}
        }
        
        Ok(())
    }
}
```

## Monitoring and Observability

```rust
#[derive(Debug, Clone)]
pub struct QuarantineMetrics {
    pub total_quarantines: u64,
    pub successful_recoveries: u64,
    pub failed_recoveries: u64,
    pub manual_interventions: u64,
    pub current_quarantined: u32,
    pub avg_recovery_time: Duration,
    pub error_count: u64,
    pub error_window: VecDeque<Instant>,
    pub consecutive_health_failures: u32,
}

impl QuarantineMetrics {
    pub fn error_rate(&self) -> Option<f64> {
        if self.error_window.is_empty() {
            return None;
        }
        
        let window_start = Instant::now() - Duration::from_secs(60);
        let recent_errors = self.error_window
            .iter()
            .filter(|&&t| t > window_start)
            .count();
        
        Some(recent_errors as f64 / 60.0)
    }
    
    pub fn record_error(&mut self) {
        self.error_count += 1;
        self.error_window.push_back(Instant::now());
        
        // Keep only last hour of errors
        let cutoff = Instant::now() - Duration::from_secs(3600);
        while let Some(&front) = self.error_window.front() {
            if front < cutoff {
                self.error_window.pop_front();
            } else {
                break;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuarantineEvent {
    pub timestamp: Instant,
    pub event_type: QuarantineEventType,
    pub reason: QuarantineReason,
}

#[derive(Debug, Clone)]
pub enum QuarantineEventType {
    Quarantined,
    RecoveryAttempted,
    Recovered,
    RecoveryFailed,
    ManualRecovery,
    Escalated,
}
```

## Configuration Example

```yaml
quarantine:
  # Error rate configuration
  error_threshold: 0.5
  error_window: 60s
  
  # Health check configuration
  health_check_failures: 3
  health_check_interval: 10s
  
  # Recovery configuration
  auto_recovery: true
  initial_recovery_delay: 30s
  max_recovery_attempts: 5
  backoff_multiplier: 2.0
  max_backoff: 5m
  
  # Recovery strategies
  strategies:
    default:
      type: exponential_backoff
      initial_delay: 30s
      max_delay: 5m
      multiplier: 2.0
      max_attempts: 5
    
    circuit_breaker:
      type: circuit_breaker
      failure_threshold: 5
      success_threshold: 2
      timeout: 60s
      half_open_requests: 3
    
    database:
      type: custom
      class: DatabaseRecoveryStrategy
      config:
        connection_timeout: 10s
        warmup_queries: 5
  
  # Notification configuration
  notifications:
    on_quarantine:
      - slack: "#ops-alerts"
      - email: "oncall@example.com"
    
    on_recovery:
      - slack: "#ops-info"
    
    on_manual_required:
      - pagerduty: "service-id"
      - slack: "#ops-critical"
  
  # Escalation policies
  escalation:
    default:
      after_duration: 30m
      after_failures: 3
      action: notify_oncall
    
    critical:
      after_duration: 5m
      after_failures: 1
      action: create_incident
      incident_severity: P1
```

## Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_quarantine_on_health_failure() {
        let manager = QuarantineManager::new(QuarantineConfig {
            health_check_failures: 2,
            ..Default::default()
        });
        
        let resource = MockResource::new()
            .with_health_check_result(Err(Error::HealthCheckFailed));
        
        manager.register_resource(resource).await.unwrap();
        
        // First failure
        manager.monitor_resource(resource.id()).await.unwrap();
        assert!(!manager.is_quarantined(resource.id()).await);
        
        // Second failure - should quarantine
        manager.monitor_resource(resource.id()).await.unwrap();
        assert!(manager.is_quarantined(resource.id()).await);
    }
    
    #[tokio::test]
    async fn test_automatic_recovery() {
        let manager = Arc::new(QuarantineManager::new(QuarantineConfig {
            auto_recovery: true,
            initial_recovery_delay: Duration::from_millis(100),
            ..Default::default()
        }));
        
        let resource = MockResource::new()
            .with_health_check_sequence(vec![
                Err(Error::HealthCheckFailed),
                Err(Error::HealthCheckFailed),
                Err(Error::HealthCheckFailed),
                Ok(HealthStatus::Healthy),
            ]);
        
        manager.register_resource(resource).await.unwrap();
        
        // Trigger quarantine
        for _ in 0..3 {
            manager.monitor_resource(resource.id()).await.unwrap();
        }
        assert!(manager.is_quarantined(resource.id()).await);
        
        // Start recovery loop
        let manager_clone = manager.clone();
        tokio::spawn(async move {
            manager_clone.start_recovery_loop().await;
        });
        
        // Wait for recovery
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!manager.is_quarantined(resource.id()).await);
    }
    
    #[tokio::test]
    async fn test_exponential_backoff() {
        let strategy = ExponentialBackoffRecovery {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(100),
            multiplier: 2.0,
            max_attempts: 5,
        };
        
        assert_eq!(strategy.next_retry_delay(0), Duration::from_secs(1));
        assert_eq!(strategy.next_retry_delay(1), Duration::from_secs(2));
        assert_eq!(strategy.next_retry_delay(2), Duration::from_secs(4));
        assert_eq!(strategy.next_retry_delay(3), Duration::from_secs(8));
        assert_eq!(strategy.next_retry_delay(10), Duration::from_secs(100)); // Max
    }
}
```

## Best Practices

1. **Set appropriate thresholds** - Balance between stability and availability
2. **Use gradual recovery** - Don't overwhelm recovering resources
3. **Monitor recovery attempts** - Track success rates and adjust strategies
4. **Implement circuit breakers** - Prevent thundering herd on recovery
5. **Log all state transitions** - For debugging and auditing
6. **Test recovery strategies** - Ensure they work under load
7. **Set up alerts** - Know when manual intervention is needed
8. **Document quarantine reasons** - Help with root cause analysis
9. **Implement health checks properly** - They're the foundation of quarantine
10. **Consider dependencies** - Quarantine dependent resources appropriately
