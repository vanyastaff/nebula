---

title: Pooled Resource
tags: [nebula-resource, how-to, pooling, performance]
status: stable
created: 2025-08-17
---

# Pooled Resource

Guide to creating pooled resources with advanced features like predictive scaling, sharding, and warming strategies.

## When to Use Pooling

Pool resources when:

- Creation is expensive (database connections, SSL handshakes)
- Resources are reusable across requests
- You need to limit concurrent usage
- Performance is critical
- Resources require warming up

## Basic Pooled Resource

### Step 1: Define Pooled Resource

```rust
use nebula_resource::prelude::*;
use nebula_resource::pool::*;

#[derive(Resource)]
#[resource(
    id = "database_pool",
    name = "Database Connection Pool",
    lifecycle = "pooled",
    pool_config = "default"
)]
pub struct DatabasePoolResource;

#[derive(ResourceConfig)]
pub struct DatabasePoolConfig {
    /// Database connection string
    #[credential(id = "database_url")]
    pub connection_url_credential: String,
    
    /// Pool size configuration
    #[validate(range = "1..=100")]
    #[tier(personal = "max:5", enterprise = "max:50", cloud = "max:100")]
    pub min_connections: u32,
    
    #[validate(range = "1..=1000")]
    #[tier(personal = "max:10", enterprise = "max:100", cloud = "max:500")]
    pub max_connections: u32,
    
    /// Connection configuration
    #[validate(range = "1..=300")]
    pub connection_timeout_secs: u64,
    
    #[validate(range = "1..=3600")]
    pub idle_timeout_secs: u64,
    
    #[validate(range = "1..=86400")]
    pub max_lifetime_secs: u64,
    
    /// Advanced pooling
    pub warming_strategy: WarmingStrategy,
    pub scaling_policy: ScalingPolicy,
    pub sharding: Option<ShardingConfig>,
}

#[derive(Deserialize, Clone, Debug)]
pub enum WarmingStrategy {
    /// No pre-warming
    None,
    /// Warm to minimum size on startup
    MinSize,
    /// Warm based on time schedule
    Scheduled(Vec<WarmingSchedule>),
    /// Predictive warming based on history
    Predictive {
        history_window_hours: u32,
        lookahead_minutes: u32,
    },
}

#[derive(Deserialize, Clone, Debug)]
pub struct WarmingSchedule {
    /// Cron expression
    pub schedule: String,
    /// Target pool size
    pub target_size: u32,
    /// How long before to start warming
    pub warmup_minutes: u32,
}
```

### Step 2: Implement Poolable Instance

```rust
pub struct DatabaseConnection {
    id: ConnectionId,
    conn: PgConnection,
    created_at: Instant,
    last_used: RwLock<Instant>,
    use_count: AtomicU64,
    health_status: RwLock<HealthStatus>,
}

#[async_trait]
impl PoolableInstance for DatabaseConnection {
    fn instance_id(&self) -> &str {
        &self.id
    }
    
    async fn is_valid(&self) -> bool {
        // Check if connection is still alive
        match timeout(Duration::from_secs(1), self.conn.ping()).await {
            Ok(Ok(_)) => true,
            _ => false,
        }
    }
    
    async fn reset(&mut self) -> Result<(), PoolError> {
        // Reset connection state for reuse
        self.conn.execute("DISCARD ALL").await
            .map_err(|e| PoolError::ResetFailed(e.to_string()))?;
        
        *self.last_used.write().await = Instant::now();
        self.use_count.fetch_add(1, Ordering::Relaxed);
        
        Ok(())
    }
    
    fn should_retire(&self) -> bool {
        let age = self.created_at.elapsed();
        let use_count = self.use_count.load(Ordering::Relaxed);
        
        // Retire if too old or used too many times
        age > Duration::from_secs(3600) || use_count > 1000
    }
    
    fn metrics(&self) -> InstanceMetrics {
        InstanceMetrics {
            age: self.created_at.elapsed(),
            use_count: self.use_count.load(Ordering::Relaxed),
            last_used: *self.last_used.read().await,
            health: self.health_status.read().await.clone(),
        }
    }
}
```

### Step 3: Implement Advanced Pool

```rust
pub struct DatabasePool {
    config: DatabasePoolConfig,
    pool: Arc<Pool<DatabaseConnection>>,
    predictor: Option<Arc<UsagePredictor>>,
    warmer: Option<Arc<PoolWarmer>>,
    scaler: Arc<AutoScaler>,
    metrics: Arc<PoolMetrics>,
    shards: Option<Vec<Arc<Pool<DatabaseConnection>>>>,
}

impl DatabasePool {
    pub async fn new(config: DatabasePoolConfig, context: &ResourceContext) -> Result<Self, PoolError> {
        // Create connection factory
        let factory = DatabaseConnectionFactory::new(&config, context).await?;
        
        // Create pool with configuration
        let pool_config = PoolConfig {
            min_size: config.min_connections as usize,
            max_size: config.max_connections as usize,
            connection_timeout: Duration::from_secs(config.connection_timeout_secs),
            idle_timeout: Duration::from_secs(config.idle_timeout_secs),
            max_lifetime: Duration::from_secs(config.max_lifetime_secs),
            validation_interval: Duration::from_secs(30),
            retry_policy: RetryPolicy::exponential_backoff(3, Duration::from_millis(100)),
        };
        
        // Create sharded pools if configured
        let (pool, shards) = if let Some(shard_config) = &config.sharding {
            let shards = Self::create_sharded_pools(
                shard_config,
                &pool_config,
                factory
            ).await?;
            
            // Create router pool
            let router = ShardedPool::new(shards.clone(), shard_config.selector.clone());
            (Arc::new(router), Some(shards))
        } else {
            let pool = Pool::new(pool_config, factory);
            (Arc::new(pool), None)
        };
        
        // Initialize warming if configured
        let warmer = match &config.warming_strategy {
            WarmingStrategy::None => None,
            WarmingStrategy::MinSize => {
                let warmer = PoolWarmer::new(pool.clone());
                warmer.warm_to_size(config.min_connections as usize).await?;
                Some(Arc::new(warmer))
            }
            WarmingStrategy::Scheduled(schedules) => {
                let warmer = ScheduledWarmer::new(pool.clone(), schedules.clone());
                warmer.start().await?;
                Some(Arc::new(warmer))
            }
            WarmingStrategy::Predictive { history_window_hours, lookahead_minutes } => {
                let predictor = UsagePredictor::new(
                    Duration::from_hours(*history_window_hours as u64),
                    Duration::from_minutes(*lookahead_minutes as u64),
                );
                let warmer = PredictiveWarmer::new(pool.clone(), predictor.clone());
                warmer.start().await?;
                Some(Arc::new(warmer))
            }
        };
        
        // Initialize auto-scaler
        let scaler = AutoScaler::new(
            pool.clone(),
            config.scaling_policy.clone(),
            Arc::new(PoolMetrics::new()),
        );
        scaler.start().await?;
        
        Ok(Self {
            config,
            pool,
            predictor: warmer.as_ref().and_then(|w| w.predictor()),
            warmer,
            scaler: Arc::new(scaler),
            metrics: Arc::new(PoolMetrics::new()),
            shards,
        })
    }
    
    /// Acquire connection from pool
    pub async fn acquire(&self) -> Result<PooledConnection<DatabaseConnection>, PoolError> {
        let start = Instant::now();
        self.metrics.acquisition_attempts.inc();
        
        // Try fast path first
        if let Some(conn) = self.pool.try_acquire() {
            self.metrics.record_acquisition(start.elapsed(), true);
            return Ok(conn);
        }
        
        // Slow path with waiting
        match timeout(
            self.config.connection_timeout,
            self.pool.acquire()
        ).await {
            Ok(Ok(conn)) => {
                self.metrics.record_acquisition(start.elapsed(), true);
                Ok(conn)
            }
            Ok(Err(e)) => {
                self.metrics.record_acquisition(start.elapsed(), false);
                Err(e)
            }
            Err(_) => {
                self.metrics.timeouts.inc();
                Err(PoolError::AcquisitionTimeout)
            }
        }
    }
    
    /// Get connection with shard key
    pub async fn acquire_sharded(&self, shard_key: &str) -> Result<PooledConnection<DatabaseConnection>, PoolError> {
        if let Some(shards) = &self.shards {
            let shard_id = self.calculate_shard(shard_key);
            shards[shard_id].acquire().await
        } else {
            self.acquire().await
        }
    }
}
```

### Step 4: Predictive Scaling

```rust
pub struct PredictiveWarmer {
    pool: Arc<Pool<DatabaseConnection>>,
    predictor: Arc<UsagePredictor>,
    history: Arc<RwLock<UsageHistory>>,
}

impl PredictiveWarmer {
    pub async fn start(&self) -> Result<(), PoolError> {
        let pool = self.pool.clone();
        let predictor = self.predictor.clone();
        let history = self.history.clone();
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_minutes(1));
            
            loop {
                interval.tick().await;
                
                // Record current usage
                let current_usage = pool.active_connections();
                history.write().await.record(current_usage);
                
                // Predict future usage
                if let Ok(prediction) = predictor.predict_usage(&*history.read().await).await {
                    if prediction.confidence > 0.8 {
                        // Warm pool based on prediction
                        let target_size = (prediction.expected_usage * 1.2) as usize;
                        let _ = pool.warm_to_size(target_size).await;
                        
                        log::info!(
                            "Predictive warming: target={}, confidence={:.2}",
                            target_size, prediction.confidence
                        );
                    }
                }
            }
        });
        
        Ok(())
    }
}

pub struct UsagePredictor {
    model: Arc<PredictionModel>,
    features: Arc<FeatureExtractor>,
}

impl UsagePredictor {
    pub async fn predict_usage(&self, history: &UsageHistory) -> Result<UsagePrediction, PredictionError> {
        // Extract features from history
        let features = self.features.extract(history)?;
        
        // Time-based features
        let now = Utc::now();
        let hour_of_day = now.hour() as f32 / 24.0;
        let day_of_week = now.weekday().num_days_from_monday() as f32 / 7.0;
        
        // Historical patterns
        let same_time_yesterday = history.get_usage_at(now - Duration::from_days(1));
        let same_time_last_week = history.get_usage_at(now - Duration::from_weeks(1));
        let recent_trend = history.calculate_trend(Duration::from_hours(1));
        
        // Make prediction
        let input = PredictionInput {
            hour_of_day,
            day_of_week,
            recent_usage: features.recent_average,
            trend: recent_trend,
            historical_same_time: vec![same_time_yesterday, same_time_last_week],
        };
        
        let prediction = self.model.predict(input).await?;
        
        Ok(UsagePrediction {
            expected_usage: prediction.value,
            confidence: prediction.confidence,
            time_horizon: Duration::from_minutes(10),
            factors: prediction.contributing_factors,
        })
    }
}
```

### Step 5: Sharded Pool Implementation

```rust
#[derive(Clone, Debug, Deserialize)]
pub struct ShardingConfig {
    pub shard_count: usize,
    pub selector: ShardSelector,
    pub rebalancing: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub enum ShardSelector {
    /// Hash-based sharding
    Hash,
    /// Round-robin
    RoundRobin,
    /// Least connections
    LeastConnections,
    /// Consistent hashing
    ConsistentHash { replicas: usize },
}

pub struct ShardedPool<T: PoolableInstance> {
    shards: Vec<Arc<Pool<T>>>,
    selector: ShardSelector,
    round_robin_counter: AtomicUsize,
    consistent_hash: Option<ConsistentHashRing>,
}

impl<T: PoolableInstance> ShardedPool<T> {
    pub async fn acquire_with_key(&self, key: &str) -> Result<PooledConnection<T>, PoolError> {
        let shard_id = self.select_shard(key);
        self.shards[shard_id].acquire().await
    }
    
    fn select_shard(&self, key: &str) -> usize {
        match &self.selector {
            ShardSelector::Hash => {
                let mut hasher = DefaultHasher::new();
                key.hash(&mut hasher);
                (hasher.finish() as usize) % self.shards.len()
            }
            ShardSelector::RoundRobin => {
                self.round_robin_counter.fetch_add(1, Ordering::Relaxed) % self.shards.len()
            }
            ShardSelector::LeastConnections => {
                self.shards
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, shard)| shard.active_connections())
                    .map(|(idx, _)| idx)
                    .unwrap_or(0)
            }
            ShardSelector::ConsistentHash { .. } => {
                self.consistent_hash
                    .as_ref()
                    .unwrap()
                    .get_node(key)
            }
        }
    }
    
    pub async fn rebalance(&self) -> Result<(), PoolError> {
        let total_connections: usize = self.shards
            .iter()
            .map(|s| s.active_connections())
            .sum();
        
        let target_per_shard = total_connections / self.shards.len();
        
        for shard in &self.shards {
            let current = shard.active_connections();
            if current > target_per_shard + 2 {
                // Shard is overloaded, mark some connections for migration
                shard.mark_for_migration(current - target_per_shard).await?;
            }
        }
        
        Ok(())
    }
}
```

## Usage Example

```rust
#[derive(Action)]
#[action(id = "database.query")]
#[resources([DatabasePoolResource])]
pub struct DatabaseQueryAction;

impl ProcessAction for DatabaseQueryAction {
    async fn execute(
        &self,
        input: QueryInput,
        context: &ExecutionContext,
    ) -> Result<ActionResult<QueryOutput>, ActionError> {
        let pool = context.get_resource::<DatabasePoolResource>().await?;
        
        // Acquire connection from pool
        let mut conn = pool.acquire().await
            .map_err(|e| ActionError::ResourceError(e.to_string()))?;
        
        // Use connection
        let rows = sqlx::query(&input.query)
            .fetch_all(&mut *conn)  // Auto-deref to connection
            .await
            .map_err(|e| ActionError::DatabaseError(e.to_string()))?;
        
        // Connection automatically returned to pool when dropped
        
        Ok(ActionResult::Success(QueryOutput { rows }))
    }
}
```

## Configuration

```toml
[database_pool]
min_connections = 5
max_connections = 50
connection_timeout_secs = 30
idle_timeout_secs = 300
max_lifetime_secs = 3600

[database_pool.warming_strategy]
type = "predictive"
history_window_hours = 24
lookahead_minutes = 10

[database_pool.scaling_policy]
type = "utilization"
target_utilization = 0.75
scale_up_threshold = 0.85
scale_down_threshold = 0.5
cooldown_seconds = 60

[database_pool.sharding]
shard_count = 4
selector = "consistent_hash"
rebalancing = true

# Tier-specific
[tier.personal]
min_connections = 1
max_connections = 5
sharding = null  # No sharding for personal tier

[tier.enterprise]
min_connections = 10
max_connections = 100
```

## Monitoring Pool Health

```rust
impl DatabasePool {
    pub fn get_pool_stats(&self) -> PoolStats {
        PoolStats {
            total_connections: self.pool.total_connections(),
            active_connections: self.pool.active_connections(),
            idle_connections: self.pool.idle_connections(),
            waiting_requests: self.pool.waiting_count(),
            
            total_created: self.metrics.total_created.get(),
            total_destroyed: self.metrics.total_destroyed.get(),
            
            acquisition_time_p50: self.metrics.acquisition_time_p50(),
            acquisition_time_p99: self.metrics.acquisition_time_p99(),
            
            health_check_failures: self.metrics.health_check_failures.get(),
            timeout_errors: self.metrics.timeouts.get(),
            
            prediction_accuracy: self.predictor
                .as_ref()
                .map(|p| p.accuracy())
                .unwrap_or(0.0),
        }
    }
    
    pub async fn health_report(&self) -> HealthReport {
        let mut report = HealthReport::new();
        
        // Check pool utilization
        let utilization = self.pool.utilization();
        if utilization > 0.9 {
            report.add_warning("Pool utilization above 90%");
        }
        
        // Check wait times
        if self.metrics.acquisition_time_p99() > Duration::from_secs(5) {
            report.add_warning("P99 acquisition time exceeds 5 seconds");
        }
        
        // Check connection health
        let unhealthy = self.pool.unhealthy_connections();
        if unhealthy > 0 {
            report.add_issue(format!("{} unhealthy connections", unhealthy));
        }
        
        // Check prediction accuracy
        if let Some(predictor) = &self.predictor {
            if predictor.accuracy() < 0.7 {
                report.add_info("Prediction accuracy below 70%, consider retraining");
            }
        }
        
        report
    }
}
```

## Best Practices

1. **Size appropriately** - Start small, monitor, adjust
2. **Set reasonable timeouts** - Prevent indefinite waiting
3. **Implement health checks** - Detect bad connections early
4. **Use predictive scaling** - For predictable workloads
5. **Consider sharding** - For very high throughput
6. **Monitor continuously** - Track metrics and adjust
7. **Test under load** - Ensure pool behaves correctly
8. **Handle pool exhaustion** - Graceful degradation