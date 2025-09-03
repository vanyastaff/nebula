---

title: Stateful Resource
tags: [nebula-resource, how-to, stateful, persistence]
status: stable
created: 2025-08-17
---

# Stateful Resource

Guide to creating resources that maintain state across workflow executions with automatic versioning and migration.

## When to Use Stateful Resources

Use stateful resources when you need to:

- Accumulate data across workflow steps
- Maintain session state
- Track progress over time
- Store temporary computation results
- Implement caching with persistence
- Maintain counters or aggregations

## Basic Stateful Resource

### Step 1: Define the Stateful Resource

```rust
use nebula_resource::prelude::*;
use nebula_resource::stateful::*;

#[derive(Resource)]
#[resource(
    id = "workflow_metrics",
    name = "Workflow Metrics Collector",
    lifecycle = "workflow",  // One instance per workflow
    stateful = true,         // Enable state persistence
)]
pub struct WorkflowMetricsResource;
```

### Step 2: Define State Structure with Versioning

```rust
/// Current version (2.0) of the state
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MetricsStateV2 {
    pub workflow_id: String,
    pub started_at: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
    
    // Metrics data
    pub action_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub total_duration_ms: u64,
    
    // New in V2: Detailed metrics
    pub action_metrics: HashMap<String, ActionMetrics>,
    pub error_categories: HashMap<String, u32>,
    pub performance_percentiles: PerformancePercentiles,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ActionMetrics {
    pub execution_count: u64,
    pub total_duration_ms: u64,
    pub min_duration_ms: u64,
    pub max_duration_ms: u64,
    pub error_count: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct PerformancePercentiles {
    pub p50: u64,
    pub p90: u64,
    pub p95: u64,
    pub p99: u64,
}
```

### Step 3: Implement Stateful Instance

```rust
pub struct WorkflowMetricsInstance {
    id: ResourceInstanceId,
    state: MetricsStateV2,
    storage: Arc<dyn StateStorage>,
    auto_save_interval: Duration,
    last_save: RwLock<DateTime<Utc>>,
    dirty: AtomicBool,
}

impl WorkflowMetricsInstance {
    /// Record action execution
    pub async fn record_action(
        &self,
        action_name: &str,
        duration: Duration,
        success: bool,
    ) -> Result<(), MetricsError> {
        // Update counters
        self.state.action_count += 1;
        if success {
            self.state.success_count += 1;
        } else {
            self.state.failure_count += 1;
        }
        
        // Update duration
        let duration_ms = duration.as_millis() as u64;
        self.state.total_duration_ms += duration_ms;
        
        // Update detailed metrics
        let metrics = self.state.action_metrics
            .entry(action_name.to_string())
            .or_insert_with(|| ActionMetrics {
                execution_count: 0,
                total_duration_ms: 0,
                min_duration_ms: u64::MAX,
                max_duration_ms: 0,
                error_count: 0,
            });
        
        metrics.execution_count += 1;
        metrics.total_duration_ms += duration_ms;
        metrics.min_duration_ms = metrics.min_duration_ms.min(duration_ms);
        metrics.max_duration_ms = metrics.max_duration_ms.max(duration_ms);
        
        if !success {
            metrics.error_count += 1;
        }
        
        // Update percentiles
        self.update_percentiles(duration_ms);
        
        // Mark as dirty
        self.dirty.store(true, Ordering::Relaxed);
        self.state.last_updated = Utc::now();
        
        // Auto-save if needed
        if self.should_auto_save().await {
            self.save_state().await?;
        }
        
        Ok(())
    }
    
    /// Get current metrics summary
    pub fn get_summary(&self) -> MetricsSummary {
        MetricsSummary {
            workflow_id: self.state.workflow_id.clone(),
            duration: Utc::now().signed_duration_since(self.state.started_at),
            action_count: self.state.action_count,
            success_rate: if self.state.action_count > 0 {
                (self.state.success_count as f64) / (self.state.action_count as f64)
            } else {
                0.0
            },
            average_duration_ms: if self.state.action_count > 0 {
                self.state.total_duration_ms / self.state.action_count
            } else {
                0
            },
            percentiles: self.state.performance_percentiles.clone(),
            top_errors: self.get_top_errors(5),
        }
    }
    
    async fn should_auto_save(&self) -> bool {
        let last_save = *self.last_save.read().await;
        let since_last_save = Utc::now().signed_duration_since(last_save);
        
        since_last_save > chrono::Duration::from_std(self.auto_save_interval).unwrap()
            && self.dirty.load(Ordering::Relaxed)
    }
}
```

### Step 4: Implement StatefulResource Trait

```rust
#[async_trait]
impl StatefulResource for WorkflowMetricsResource {
    type State = MetricsStateV2;
    
    fn state_version() -> semver::Version {
        semver::Version::new(2, 0, 0)
    }
    
    async fn save_state(&self, instance: &WorkflowMetricsInstance) -> Result<VersionedState<Self::State>, ResourceError> {
        let state = instance.state.clone();
        let checksum = self.calculate_checksum(&state);
        
        let versioned = VersionedState {
            version: Self::state_version(),
            state,
            checksum,
            created_at: Utc::now(),
            migration_history: vec![],
        };
        
        // Save to storage
        let key = format!("metrics:{}", instance.state.workflow_id);
        instance.storage.save(&key, &versioned).await?;
        
        // Update tracking
        *instance.last_save.write().await = Utc::now();
        instance.dirty.store(false, Ordering::Relaxed);
        
        Ok(versioned)
    }
    
    async fn restore_from_state(
        &mut self,
        instance: &mut WorkflowMetricsInstance,
        versioned_state: VersionedState<Self::State>,
        context: &ResourceContext,
    ) -> Result<(), ResourceError> {
        // Check if migration needed
        let mut state = if versioned_state.version != Self::state_version() {
            context.log_info(&format!(
                "Migrating state from v{} to v{}",
                versioned_state.version,
                Self::state_version()
            ));
            
            self.migrate_state(
                serde_json::to_value(&versioned_state.state)?,
                versioned_state.version.clone(),
                Self::state_version(),
            ).await?
        } else {
            versioned_state.state
        };
        
        // Validate checksum
        if !self.validate_checksum(&versioned_state) {
            return Err(ResourceError::CorruptedState(
                "Checksum validation failed".into()
            ));
        }
        
        instance.state = state;
        context.log_info(&format!(
            "Restored metrics state: {} actions, {:.1}% success rate",
            instance.state.action_count,
            (instance.state.success_count as f64 / instance.state.action_count as f64) * 100.0
        ));
        
        Ok(())
    }
    
    async fn migrate_state(
        &self,
        old_state: serde_json::Value,
        from_version: semver::Version,
        to_version: semver::Version,
    ) -> Result<Self::State, ResourceError> {
        // Handle migration from V1 to V2
        if from_version.major == 1 && to_version.major == 2 {
            #[derive(Deserialize)]
            struct MetricsStateV1 {
                workflow_id: String,
                started_at: DateTime<Utc>,
                last_updated: DateTime<Utc>,
                action_count: u64,
                success_count: u64,
                failure_count: u64,
                total_duration_ms: u64,
                // V1 didn't have detailed metrics
            }
            
            let v1: MetricsStateV1 = serde_json::from_value(old_state)?;
            
            // Create V2 state from V1
            Ok(MetricsStateV2 {
                workflow_id: v1.workflow_id,
                started_at: v1.started_at,
                last_updated: v1.last_updated,
                action_count: v1.action_count,
                success_count: v1.success_count,
                failure_count: v1.failure_count,
                total_duration_ms: v1.total_duration_ms,
                // New fields with defaults
                action_metrics: HashMap::new(),
                error_categories: HashMap::new(),
                performance_percentiles: PerformancePercentiles::default(),
            })
        } else {
            Err(ResourceError::UnsupportedMigration {
                from: from_version,
                to: to_version,
            })
        }
    }
    
    async fn cleanup_state(&self, instance: &WorkflowMetricsInstance, context: &ResourceContext) -> Result<(), ResourceError> {
        // Save final state
        self.save_state(instance).await?;
        
        // Archive if needed
        if context.should_archive() {
            let archive_key = format!("archive:metrics:{}", instance.state.workflow_id);
            instance.storage.archive(&archive_key, &instance.state).await?;
        }
        
        context.log_info(&format!(
            "Cleaned up metrics for workflow {}: {} total actions",
            instance.state.workflow_id,
            instance.state.action_count
        ));
        
        Ok(())
    }
}
```

## Advanced: Multi-Version State Migration

```rust
/// Support multiple migration paths
impl WorkflowMetricsResource {
    async fn migrate_state_advanced(
        &self,
        old_state: serde_json::Value,
        from_version: semver::Version,
        to_version: semver::Version,
    ) -> Result<MetricsStateV2, ResourceError> {
        // Build migration path
        let path = self.find_migration_path(from_version.clone(), to_version.clone())?;
        
        let mut current_state = old_state;
        let mut current_version = from_version;
        
        for target_version in path {
            current_state = self.migrate_step(
                current_state,
                current_version.clone(),
                target_version.clone()
            ).await?;
            current_version = target_version;
        }
        
        serde_json::from_value(current_state)
            .map_err(|e| ResourceError::StateMigrationFailed(e.to_string()))
    }
    
    fn find_migration_path(
        &self,
        from: semver::Version,
        to: semver::Version,
    ) -> Result<Vec<semver::Version>, ResourceError> {
        // Define migration graph
        let migrations = vec![
            (Version::new(1, 0, 0), Version::new(1, 1, 0)),
            (Version::new(1, 1, 0), Version::new(2, 0, 0)),
            (Version::new(2, 0, 0), Version::new(2, 1, 0)),
        ];
        
        // Find shortest path using BFS
        // ... implementation
        
        Ok(path)
    }
    
    async fn migrate_step(
        &self,
        state: serde_json::Value,
        from: semver::Version,
        to: semver::Version,
    ) -> Result<serde_json::Value, ResourceError> {
        match (from.major, from.minor, to.major, to.minor) {
            (1, 0, 1, 1) => self.migrate_1_0_to_1_1(state).await,
            (1, 1, 2, 0) => self.migrate_1_1_to_2_0(state).await,
            (2, 0, 2, 1) => self.migrate_2_0_to_2_1(state).await,
            _ => Err(ResourceError::UnsupportedMigration { from, to })
        }
    }
}
```

## State Storage Backends

```rust
/// In-memory storage (for testing)
pub struct InMemoryStorage {
    data: Arc<DashMap<String, Vec<u8>>>,
}

/// Redis storage
pub struct RedisStorage {
    client: redis::Client,
    ttl: Option<Duration>,
}

/// PostgreSQL storage
pub struct PostgresStorage {
    pool: PgPool,
    table_name: String,
}

/// S3 storage (for large states)
pub struct S3Storage {
    client: S3Client,
    bucket: String,
    prefix: String,
}

#[async_trait]
impl StateStorage for RedisStorage {
    async fn save(&self, key: &str, value: &impl Serialize) -> Result<(), StorageError> {
        let data = bincode::serialize(value)?;
        let mut conn = self.client.get_async_connection().await?;
        
        if let Some(ttl) = self.ttl {
            redis::cmd("SETEX")
                .arg(key)
                .arg(ttl.as_secs())
                .arg(data)
                .query_async(&mut conn)
                .await?;
        } else {
            redis::cmd("SET")
                .arg(key)
                .arg(data)
                .query_async(&mut conn)
                .await?;
        }
        
        Ok(())
    }
    
    async fn load<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, StorageError> {
        let mut conn = self.client.get_async_connection().await?;
        let data: Option<Vec<u8>> = redis::cmd("GET")
            .arg(key)
            .query_async(&mut conn)
            .await?;
        
        match data {
            Some(bytes) => {
                let value = bincode::deserialize(&bytes)?;
                Ok(Some(value))
            }
            None => Ok(None)
        }
    }
    
    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let mut conn = self.client.get_async_connection().await?;
        redis::cmd("DEL")
            .arg(key)
            .query_async(&mut conn)
            .await?;
        Ok(())
    }
}
```

## Using Stateful Resources

```rust
#[derive(Action)]
#[action(id = "report.generate")]
#[resources([WorkflowMetricsResource, DatabaseResource])]
pub struct GenerateReportAction;

impl ProcessAction for GenerateReportAction {
    async fn execute(
        &self,
        input: Input,
        context: &ExecutionContext,
    ) -> Result<ActionResult<Output>, ActionError> {
        let metrics = context.get_resource::<WorkflowMetricsResource>().await?;
        let db = context.get_resource::<DatabaseResource>().await?;
        
        let start = Instant::now();
        
        // Do work
        let report = generate_report(&input, &db).await?;
        
        // Record in stateful resource
        metrics.record_action(
            "generate_report",
            start.elapsed(),
            true
        ).await?;
        
        // Get summary for response
        let summary = metrics.get_summary();
        
        Ok(ActionResult::Success(Output {
            report,
            metrics_summary: summary,
        }))
    }
}
```

## Best Practices

1. **Version from the start** - Always include version in state
2. **Plan for migration** - Design state changes carefully
3. **Use checksums** - Detect corruption early
4. **Auto-save periodically** - Don't wait until cleanup
5. **Keep state minimal** - Only essential data
6. **Test migrations** - Ensure backward compatibility
7. **Archive old states** - For debugging and audit
8. **Handle concurrent access** - Use appropriate locking