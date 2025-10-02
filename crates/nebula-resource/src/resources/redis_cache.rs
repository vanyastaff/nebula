//! Redis cache resource implementation

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
    traits::{HealthCheckable, HealthStatus},
};

#[cfg(feature = "redis")]
use redis::aio::ConnectionManager;

/// Redis cache configuration
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct RedisCacheConfig {
    /// Redis connection URL (redis://host:port/db)
    pub url: String,
    /// Connection timeout in seconds
    pub timeout_seconds: u64,
    /// Maximum number of retries
    pub max_retries: u32,
    /// Key prefix for all operations
    pub key_prefix: Option<String>,
}

impl ResourceConfig for RedisCacheConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.url.is_empty() {
            return Err(ResourceError::configuration("Redis URL cannot be empty"));
        }

        if !self.url.starts_with("redis://") && !self.url.starts_with("rediss://") {
            return Err(ResourceError::configuration(
                "Redis URL must start with redis:// or rediss://",
            ));
        }

        if self.timeout_seconds == 0 {
            return Err(ResourceError::configuration(
                "Timeout must be greater than 0",
            ));
        }

        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.url.is_empty() {
            self.url = other.url;
        }
        if other.timeout_seconds > 0 {
            self.timeout_seconds = other.timeout_seconds;
        }
        if other.max_retries > 0 {
            self.max_retries = other.max_retries;
        }
        if other.key_prefix.is_some() {
            self.key_prefix = other.key_prefix;
        }
    }
}

/// Redis cache resource
pub struct RedisCacheResource;

#[async_trait::async_trait]
impl Resource for RedisCacheResource {
    type Config = RedisCacheConfig;
    type Instance = RedisCacheInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("redis-cache", "1.0"),
            "Redis cache for key-value storage and caching".to_string(),
        )
        .poolable()
        .health_checkable()
        .with_tag("type", "cache")
        .with_tag("backend", "redis")
    }

    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        config.validate()?;

        #[cfg(feature = "redis")]
        {
            // Create Redis client
            let client = redis::Client::open(config.url.as_str()).map_err(|e| {
                ResourceError::initialization(
                    "redis-cache:1.0",
                    format!("Failed to create Redis client: {}", e),
                )
            })?;

            // Create connection manager (handles reconnection automatically)
            let manager = ConnectionManager::new(client).await.map_err(|e| {
                ResourceError::initialization(
                    "redis-cache:1.0",
                    format!("Failed to connect to Redis: {}", e),
                )
            })?;

            Ok(RedisCacheInstance {
                instance_id: uuid::Uuid::new_v4(),
                resource_id: self.metadata().id,
                context: context.clone(),
                created_at: chrono::Utc::now(),
                last_accessed: parking_lot::Mutex::new(None),
                state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
                manager,
                key_prefix: config.key_prefix.clone(),
            })
        }

        #[cfg(not(feature = "redis"))]
        {
            Err(ResourceError::configuration(
                "Redis feature not enabled. Enable 'redis' feature to use Redis cache",
            ))
        }
    }

    async fn cleanup(&self, instance: Self::Instance) -> ResourceResult<()> {
        #[cfg(feature = "redis")]
        {
            // ConnectionManager handles cleanup automatically on drop
            drop(instance);
        }

        #[cfg(not(feature = "redis"))]
        {
            drop(instance);
        }

        Ok(())
    }

    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        #[cfg(feature = "redis")]
        {
            // Try PING command
            use redis::AsyncCommands;
            let mut conn = instance.manager.clone();
            match redis::cmd("PING").query_async::<_, String>(&mut conn).await {
                Ok(_) => Ok(true),
                Err(_) => Ok(false),
            }
        }

        #[cfg(not(feature = "redis"))]
        {
            let _ = instance;
            Ok(false)
        }
    }
}

/// Redis cache resource instance
pub struct RedisCacheInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<crate::core::lifecycle::LifecycleState>,

    #[cfg(feature = "redis")]
    manager: ConnectionManager,

    #[cfg(feature = "redis")]
    key_prefix: Option<String>,
}

impl ResourceInstance for RedisCacheInstance {
    fn instance_id(&self) -> uuid::Uuid {
        self.instance_id
    }

    fn resource_id(&self) -> &ResourceId {
        &self.resource_id
    }

    fn lifecycle_state(&self) -> crate::core::lifecycle::LifecycleState {
        *self.state.read()
    }

    fn context(&self) -> &ResourceContext {
        &self.context
    }

    fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.created_at
    }

    fn last_accessed_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.last_accessed.lock()
    }

    fn touch(&self) {
        *self.last_accessed.lock() = Some(chrono::Utc::now());
    }
}

#[cfg(feature = "redis")]
impl RedisCacheInstance {
    /// Add prefix to key if configured
    fn prefix_key(&self, key: &str) -> String {
        if let Some(ref prefix) = self.key_prefix {
            format!("{}:{}", prefix, key)
        } else {
            key.to_string()
        }
    }

    /// Get a value from Redis
    pub async fn get(&self, key: &str) -> ResourceResult<Option<String>> {
        self.touch();

        use redis::AsyncCommands;
        let key = self.prefix_key(key);
        let mut conn = self.manager.clone();

        conn.get(&key).await.map_err(|e| {
            ResourceError::internal("redis-cache:1.0", format!("Get failed: {}", e))
        })
    }

    /// Set a value in Redis
    pub async fn set(&self, key: &str, value: &str) -> ResourceResult<()> {
        self.touch();

        use redis::AsyncCommands;
        let key = self.prefix_key(key);
        let mut conn = self.manager.clone();

        conn.set(&key, value).await.map_err(|e| {
            ResourceError::internal("redis-cache:1.0", format!("Set failed: {}", e))
        })
    }

    /// Set a value with expiration (in seconds)
    pub async fn setex(&self, key: &str, value: &str, seconds: u64) -> ResourceResult<()> {
        self.touch();

        use redis::AsyncCommands;
        let key = self.prefix_key(key);
        let mut conn = self.manager.clone();

        conn.set_ex(&key, value, seconds).await.map_err(|e| {
            ResourceError::internal("redis-cache:1.0", format!("SetEx failed: {}", e))
        })
    }

    /// Delete a key
    pub async fn del(&self, key: &str) -> ResourceResult<bool> {
        self.touch();

        use redis::AsyncCommands;
        let key = self.prefix_key(key);
        let mut conn = self.manager.clone();

        let deleted: i64 = conn.del(&key).await.map_err(|e| {
            ResourceError::internal("redis-cache:1.0", format!("Del failed: {}", e))
        })?;

        Ok(deleted > 0)
    }

    /// Check if key exists
    pub async fn exists(&self, key: &str) -> ResourceResult<bool> {
        self.touch();

        use redis::AsyncCommands;
        let key = self.prefix_key(key);
        let mut conn = self.manager.clone();

        conn.exists(&key).await.map_err(|e| {
            ResourceError::internal("redis-cache:1.0", format!("Exists failed: {}", e))
        })
    }

    /// Set expiration on a key
    pub async fn expire(&self, key: &str, seconds: u64) -> ResourceResult<bool> {
        self.touch();

        use redis::AsyncCommands;
        let key = self.prefix_key(key);
        let mut conn = self.manager.clone();

        conn.expire(&key, seconds as i64).await.map_err(|e| {
            ResourceError::internal("redis-cache:1.0", format!("Expire failed: {}", e))
        })
    }

    /// Increment a counter
    pub async fn incr(&self, key: &str) -> ResourceResult<i64> {
        self.touch();

        use redis::AsyncCommands;
        let key = self.prefix_key(key);
        let mut conn = self.manager.clone();

        conn.incr(&key, 1).await.map_err(|e| {
            ResourceError::internal("redis-cache:1.0", format!("Incr failed: {}", e))
        })
    }

    /// Decrement a counter
    pub async fn decr(&self, key: &str) -> ResourceResult<i64> {
        self.touch();

        use redis::AsyncCommands;
        let key = self.prefix_key(key);
        let mut conn = self.manager.clone();

        conn.decr(&key, 1).await.map_err(|e| {
            ResourceError::internal("redis-cache:1.0", format!("Decr failed: {}", e))
        })
    }

    /// Get the underlying connection manager
    pub fn manager(&self) -> &ConnectionManager {
        &self.manager
    }
}

#[cfg(feature = "redis")]
#[async_trait::async_trait]
impl HealthCheckable for RedisCacheInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        let start = std::time::Instant::now();

        use redis::AsyncCommands;
        let mut conn = self.manager.clone();

        match redis::cmd("PING")
            .query_async::<_, String>(&mut conn)
            .await
        {
            Ok(_) => Ok(HealthStatus::healthy().with_latency(start.elapsed())),
            Err(e) => {
                let latency = start.elapsed();
                Ok(HealthStatus::unhealthy(format!("PING failed: {}", e)).with_latency(latency))
            }
        }
    }

    async fn detailed_health_check(&self, _context: &ResourceContext) -> ResourceResult<HealthStatus> {
        let start = std::time::Instant::now();

        use redis::AsyncCommands;
        let mut conn = self.manager.clone();

        // Get Redis INFO
        match redis::cmd("INFO")
            .arg("SERVER")
            .query_async::<_, String>(&mut conn)
            .await
        {
            Ok(info) => {
                let latency = start.elapsed();

                // Parse key metrics from INFO output
                let mut status = HealthStatus::healthy().with_latency(latency);

                // Extract version
                for line in info.lines() {
                    if line.starts_with("redis_version:") {
                        let version = line.trim_start_matches("redis_version:").trim();
                        status = status.with_metadata("version", version);
                    } else if line.starts_with("uptime_in_seconds:") {
                        let uptime = line.trim_start_matches("uptime_in_seconds:").trim();
                        status = status.with_metadata("uptime_seconds", uptime);
                    } else if line.starts_with("used_memory_human:") {
                        let memory = line.trim_start_matches("used_memory_human:").trim();
                        status = status.with_metadata("memory_used", memory);
                    }
                }

                Ok(status)
            }
            Err(e) => {
                let latency = start.elapsed();
                Ok(HealthStatus::unhealthy(format!("INFO failed: {}", e)).with_latency(latency))
            }
        }
    }
}

#[cfg(not(feature = "redis"))]
#[async_trait::async_trait]
impl HealthCheckable for RedisCacheInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        Ok(HealthStatus::unhealthy(
            "Redis feature not enabled",
        ))
    }

    async fn detailed_health_check(&self, _context: &ResourceContext) -> ResourceResult<HealthStatus> {
        self.health_check().await
    }
}
