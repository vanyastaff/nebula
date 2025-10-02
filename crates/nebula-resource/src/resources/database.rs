//! Database resource implementations
//!
//! This module provides resource implementations for various databases:
//! - **PostgreSQL**: Full-featured PostgreSQL support with connection pooling
//! - **MySQL/MariaDB**: MySQL and MariaDB support with connection pooling
//! - **MongoDB**: MongoDB support with connection pooling
//!
//! # Features
//!
//! - `postgres` - Enable PostgreSQL support via sqlx
//! - `mysql` - Enable MySQL/MariaDB support via sqlx
//! - `mongodb` - Enable MongoDB support (future)
//!
//! # Example
//!
//! ```rust,no_run
//! use nebula_resource::resources::database::{PostgreSqlResource, PostgreSqlConfig};
//!
//! let postgres = PostgreSqlResource;
//! let config = PostgreSqlConfig {
//!     url: "postgresql://user:pass@localhost/db".to_string(),
//!     max_connections: 10,
//!     min_connections: 2,
//!     timeout_seconds: 30,
//!     ..Default::default()
//! };
//! ```

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
    traits::{HealthCheckable, HealthStatus},
};

#[cfg(feature = "postgres")]
use sqlx::{postgres::PgPoolOptions, PgPool, Row};

#[cfg(feature = "mysql")]
use sqlx::{mysql::MySqlPoolOptions, MySqlPool};

/// Database resource configuration
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// Database connection URL
    pub url: String,
    /// Maximum number of connections in the pool
    pub max_connections: u32,
    /// Minimum number of connections to maintain
    pub min_connections: u32,
    /// Connection timeout in seconds
    pub timeout_seconds: u64,
}

impl ResourceConfig for DatabaseConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.url.is_empty() {
            return Err(crate::core::error::ResourceError::configuration("Database URL cannot be empty"));
        }

        if self.max_connections == 0 {
            return Err(crate::core::error::ResourceError::configuration("Max connections must be greater than 0"));
        }

        if self.min_connections > self.max_connections {
            return Err(crate::core::error::ResourceError::configuration("Min connections cannot exceed max connections"));
        }

        if self.timeout_seconds == 0 {
            return Err(crate::core::error::ResourceError::configuration("Timeout must be greater than 0"));
        }

        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.url.is_empty() {
            self.url = other.url;
        }
        if other.max_connections > 0 {
            self.max_connections = other.max_connections;
        }
        if other.min_connections > 0 {
            self.min_connections = other.min_connections;
        }
        if other.timeout_seconds > 0 {
            self.timeout_seconds = other.timeout_seconds;
        }
    }
}

/// Database resource instance
pub struct DatabaseInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: crate::core::context::ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<crate::core::lifecycle::LifecycleState>,

    #[cfg(feature = "postgres")]
    pool: PgPool,

    #[cfg(not(feature = "postgres"))]
    url: String,
    #[cfg(not(feature = "postgres"))]
    max_connections: u32,
}

impl ResourceInstance for DatabaseInstance {
    fn instance_id(&self) -> uuid::Uuid {
        self.instance_id
    }

    fn resource_id(&self) -> &ResourceId {
        &self.resource_id
    }

    fn lifecycle_state(&self) -> crate::core::lifecycle::LifecycleState {
        *self.state.read()
    }

    fn context(&self) -> &crate::core::context::ResourceContext {
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

/// Database resource
pub struct DatabaseResource;

#[async_trait::async_trait]
impl Resource for DatabaseResource {
    type Config = DatabaseConfig;
    type Instance = DatabaseInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("database", "1.0"),
            "Database connection resource".to_string(),
        )
        .poolable()
        .health_checkable()
        .with_default_scope(ResourceScope::Global)
    }

    async fn create(
        &self,
        config: &Self::Config,
        context: &crate::core::context::ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        config.validate()?;

        #[cfg(feature = "postgres")]
        {
            // Create real PostgreSQL connection pool
            let pool = PgPoolOptions::new()
                .max_connections(config.max_connections)
                .min_connections(config.min_connections)
                .acquire_timeout(std::time::Duration::from_secs(config.timeout_seconds))
                .connect(&config.url)
                .await
                .map_err(|e| {
                    ResourceError::initialization(
                        "database:1.0",
                        format!("Failed to connect to PostgreSQL: {}", e),
                    )
                })?;

            Ok(DatabaseInstance {
                instance_id: uuid::Uuid::new_v4(),
                resource_id: self.metadata().id,
                context: context.clone(),
                created_at: chrono::Utc::now(),
                last_accessed: parking_lot::Mutex::new(None),
                state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
                pool,
            })
        }

        #[cfg(not(feature = "postgres"))]
        {
            // Mock implementation without sqlx
            Ok(DatabaseInstance {
                instance_id: uuid::Uuid::new_v4(),
                resource_id: self.metadata().id,
                context: context.clone(),
                created_at: chrono::Utc::now(),
                last_accessed: parking_lot::Mutex::new(None),
                state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
                url: config.url.clone(),
                max_connections: config.max_connections,
            })
        }
    }

    async fn cleanup(&self, instance: Self::Instance) -> ResourceResult<()> {
        #[cfg(feature = "postgres")]
        {
            // Close the connection pool
            instance.pool.close().await;
        }

        #[cfg(not(feature = "postgres"))]
        {
            // Simulate connection cleanup
            let _ = instance;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        Ok(())
    }

    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        Ok(matches!(
            instance.lifecycle_state(),
            crate::core::lifecycle::LifecycleState::Ready
                | crate::core::lifecycle::LifecycleState::Idle
                | crate::core::lifecycle::LifecycleState::InUse
        ))
    }
}

impl DatabaseInstance {
    /// Execute a database query
    #[cfg(feature = "postgres")]
    pub async fn execute_query(&self, query: &str) -> ResourceResult<u64> {
        self.touch();

        let result = sqlx::query(query)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                ResourceError::internal("database:1.0", format!("Query execution failed: {}", e))
            })?;

        Ok(result.rows_affected())
    }

    /// Execute a database query (mock implementation)
    #[cfg(not(feature = "postgres"))]
    pub async fn execute_query(&self, _query: &str) -> ResourceResult<u64> {
        self.touch();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        Ok(42) // Mock result
    }

    /// Fetch a single row
    #[cfg(feature = "postgres")]
    pub async fn fetch_one<T>(&self, query: &str) -> ResourceResult<T>
    where
        T: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin,
    {
        self.touch();

        sqlx::query_as::<_, T>(query)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                ResourceError::internal("database:1.0", format!("Fetch failed: {}", e))
            })
    }

    /// Fetch all rows
    #[cfg(feature = "postgres")]
    pub async fn fetch_all<T>(&self, query: &str) -> ResourceResult<Vec<T>>
    where
        T: for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow> + Send + Unpin,
    {
        self.touch();

        sqlx::query_as::<_, T>(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                ResourceError::internal("database:1.0", format!("Fetch failed: {}", e))
            })
    }

    /// Begin a transaction
    #[cfg(feature = "postgres")]
    pub async fn begin_transaction(&self) -> ResourceResult<sqlx::Transaction<'_, sqlx::Postgres>> {
        self.touch();

        self.pool.begin().await.map_err(|e| {
            ResourceError::internal("database:1.0", format!("Transaction start failed: {}", e))
        })
    }

    /// Get the underlying connection pool
    #[cfg(feature = "postgres")]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Perform health check with SELECT 1
    #[cfg(feature = "postgres")]
    pub async fn health_check(&self) -> ResourceResult<bool> {
        match sqlx::query("SELECT 1").execute(&self.pool).await {
            Ok(_) => Ok(true),
            Err(e) => {
                tracing::warn!("Database health check failed: {}", e);
                Ok(false)
            }
        }
    }

    /// Get connection information (mock)
    #[cfg(not(feature = "postgres"))]
    pub fn connection_info(&self) -> (String, u32) {
        (self.url.clone(), self.max_connections)
    }
}

#[async_trait::async_trait]
impl HealthCheckable for DatabaseInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        #[cfg(feature = "postgres")]
        {
            let start = std::time::Instant::now();

            match sqlx::query("SELECT 1").execute(&self.pool).await {
                Ok(_) => Ok(HealthStatus::healthy()),
                Err(e) => {
                    let latency = start.elapsed();
                    Ok(HealthStatus::unhealthy(format!("Database query failed: {}", e))
                        .with_latency(latency))
                }
            }
        }

        #[cfg(not(feature = "postgres"))]
        {
            // Mock health check
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            Ok(HealthStatus::healthy())
        }
    }

    async fn detailed_health_check(&self, _context: &ResourceContext) -> ResourceResult<HealthStatus> {
        #[cfg(feature = "postgres")]
        {
            let start = std::time::Instant::now();

            // Check connection count
            let pool_options = self.pool.options();
            let size = self.pool.size() as usize;
            let max_size = pool_options.get_max_connections() as usize;

            // Try to execute a simple query
            match sqlx::query("SELECT version(), current_database(), current_user")
                .fetch_one(&self.pool)
                .await
            {
                Ok(row) => {
                    let latency = start.elapsed();
                    let version: String = row.get(0);
                    let database: String = row.get(1);
                    let user: String = row.get(2);

                    Ok(HealthStatus::healthy()
                        .with_latency(latency)
                        .with_metadata("version", version)
                        .with_metadata("database", database)
                        .with_metadata("user", user)
                        .with_metadata("pool_size", size.to_string())
                        .with_metadata("pool_max", max_size.to_string()))
                }
                Err(e) => {
                    let latency = start.elapsed();
                    Ok(HealthStatus::unhealthy(format!("Database query failed: {}", e))
                        .with_latency(latency)
                        .with_metadata("pool_size", size.to_string())
                        .with_metadata("pool_max", max_size.to_string()))
                }
            }
        }

        #[cfg(not(feature = "postgres"))]
        {
            self.health_check().await
        }
    }
}

// ============================================================================
// MySQL/MariaDB Resource
// ============================================================================

/// MySQL/MariaDB configuration
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MySqlConfig {
    /// MySQL connection URL (e.g., "mysql://user:pass@localhost/db")
    pub url: String,
    /// Maximum number of connections in the pool
    pub max_connections: u32,
    /// Minimum number of connections to maintain
    pub min_connections: u32,
    /// Connection timeout in seconds
    pub timeout_seconds: u64,
    /// Connection idle timeout in seconds
    pub idle_timeout_seconds: Option<u64>,
    /// Maximum connection lifetime in seconds
    pub max_lifetime_seconds: Option<u64>,
}

impl Default for MySqlConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            max_connections: 10,
            min_connections: 2,
            timeout_seconds: 30,
            idle_timeout_seconds: Some(600), // 10 minutes
            max_lifetime_seconds: Some(1800), // 30 minutes
        }
    }
}

impl ResourceConfig for MySqlConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.url.is_empty() {
            return Err(ResourceError::configuration("MySQL URL cannot be empty"));
        }

        if !self.url.starts_with("mysql://") && !self.url.starts_with("mariadb://") {
            return Err(ResourceError::configuration("MySQL URL must start with mysql:// or mariadb://"));
        }

        if self.max_connections == 0 {
            return Err(ResourceError::configuration("Max connections must be greater than 0"));
        }

        if self.min_connections > self.max_connections {
            return Err(ResourceError::configuration("Min connections cannot exceed max connections"));
        }

        if self.timeout_seconds == 0 {
            return Err(ResourceError::configuration("Timeout must be greater than 0"));
        }

        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.url.is_empty() {
            self.url = other.url;
        }
        if other.max_connections > 0 {
            self.max_connections = other.max_connections;
        }
        if other.min_connections > 0 {
            self.min_connections = other.min_connections;
        }
        if other.timeout_seconds > 0 {
            self.timeout_seconds = other.timeout_seconds;
        }
        self.idle_timeout_seconds = other.idle_timeout_seconds.or(self.idle_timeout_seconds);
        self.max_lifetime_seconds = other.max_lifetime_seconds.or(self.max_lifetime_seconds);
    }
}

/// MySQL/MariaDB instance
pub struct MySqlInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<crate::core::lifecycle::LifecycleState>,

    #[cfg(feature = "mysql")]
    pool: MySqlPool,

    #[cfg(not(feature = "mysql"))]
    url: String,
    #[cfg(not(feature = "mysql"))]
    max_connections: u32,
}

impl ResourceInstance for MySqlInstance {
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

impl MySqlInstance {
    /// Execute a database query
    #[cfg(feature = "mysql")]
    pub async fn execute_query(&self, query: &str) -> ResourceResult<u64> {
        self.touch();

        let result = sqlx::query(query)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                ResourceError::internal("mysql:1.0", format!("Query execution failed: {}", e))
            })?;

        Ok(result.rows_affected())
    }

    /// Execute a database query (mock)
    #[cfg(not(feature = "mysql"))]
    pub async fn execute_query(&self, _query: &str) -> ResourceResult<u64> {
        self.touch();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        Ok(42)
    }

    /// Fetch a single row
    #[cfg(feature = "mysql")]
    pub async fn fetch_one<T>(&self, query: &str) -> ResourceResult<T>
    where
        T: for<'r> sqlx::FromRow<'r, sqlx::mysql::MySqlRow> + Send + Unpin,
    {
        self.touch();

        sqlx::query_as::<_, T>(query)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                ResourceError::internal("mysql:1.0", format!("Fetch failed: {}", e))
            })
    }

    /// Fetch all rows
    #[cfg(feature = "mysql")]
    pub async fn fetch_all<T>(&self, query: &str) -> ResourceResult<Vec<T>>
    where
        T: for<'r> sqlx::FromRow<'r, sqlx::mysql::MySqlRow> + Send + Unpin,
    {
        self.touch();

        sqlx::query_as::<_, T>(query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                ResourceError::internal("mysql:1.0", format!("Fetch failed: {}", e))
            })
    }

    /// Begin a transaction
    #[cfg(feature = "mysql")]
    pub async fn begin_transaction(&self) -> ResourceResult<sqlx::Transaction<'_, sqlx::MySql>> {
        self.touch();

        self.pool.begin().await.map_err(|e| {
            ResourceError::internal("mysql:1.0", format!("Transaction start failed: {}", e))
        })
    }

    /// Get the underlying connection pool
    #[cfg(feature = "mysql")]
    pub fn pool(&self) -> &MySqlPool {
        &self.pool
    }

    /// Get connection information (mock)
    #[cfg(not(feature = "mysql"))]
    pub fn connection_info(&self) -> (String, u32) {
        (self.url.clone(), self.max_connections)
    }
}

#[async_trait::async_trait]
impl HealthCheckable for MySqlInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        #[cfg(feature = "mysql")]
        {
            let start = std::time::Instant::now();

            match sqlx::query("SELECT 1").execute(&self.pool).await {
                Ok(_) => Ok(HealthStatus::healthy()),
                Err(e) => {
                    let latency = start.elapsed();
                    Ok(HealthStatus::unhealthy(format!("MySQL query failed: {}", e))
                        .with_latency(latency))
                }
            }
        }

        #[cfg(not(feature = "mysql"))]
        {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            Ok(HealthStatus::healthy())
        }
    }

    async fn detailed_health_check(&self, _context: &ResourceContext) -> ResourceResult<HealthStatus> {
        #[cfg(feature = "mysql")]
        {
            let start = std::time::Instant::now();

            let size = self.pool.size() as usize;
            let pool_options = self.pool.options();
            let max_size = pool_options.get_max_connections() as usize;

            match sqlx::query("SELECT VERSION(), DATABASE(), USER()")
                .fetch_one(&self.pool)
                .await
            {
                Ok(row) => {
                    use sqlx::Row;
                    let latency = start.elapsed();
                    let version: String = row.get(0);
                    let database: String = row.get(1);
                    let user: String = row.get(2);

                    Ok(HealthStatus::healthy()
                        .with_latency(latency)
                        .with_metadata("version", version)
                        .with_metadata("database", database)
                        .with_metadata("user", user)
                        .with_metadata("pool_size", size.to_string())
                        .with_metadata("pool_max", max_size.to_string()))
                }
                Err(e) => {
                    let latency = start.elapsed();
                    Ok(HealthStatus::unhealthy(format!("MySQL query failed: {}", e))
                        .with_latency(latency)
                        .with_metadata("pool_size", size.to_string())
                        .with_metadata("pool_max", max_size.to_string()))
                }
            }
        }

        #[cfg(not(feature = "mysql"))]
        {
            self.health_check().await
        }
    }
}

/// MySQL/MariaDB resource
pub struct MySqlResource;

#[async_trait::async_trait]
impl Resource for MySqlResource {
    type Config = MySqlConfig;
    type Instance = MySqlInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("mysql", "1.0"),
            "MySQL/MariaDB connection resource".to_string(),
        )
        .poolable()
        .health_checkable()
        .with_default_scope(ResourceScope::Global)
    }

    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        config.validate()?;

        #[cfg(feature = "mysql")]
        {
            let mut pool_options = MySqlPoolOptions::new()
                .max_connections(config.max_connections)
                .min_connections(config.min_connections)
                .acquire_timeout(std::time::Duration::from_secs(config.timeout_seconds));

            if let Some(idle_timeout) = config.idle_timeout_seconds {
                pool_options = pool_options.idle_timeout(std::time::Duration::from_secs(idle_timeout));
            }

            if let Some(max_lifetime) = config.max_lifetime_seconds {
                pool_options = pool_options.max_lifetime(std::time::Duration::from_secs(max_lifetime));
            }

            let pool = pool_options
                .connect(&config.url)
                .await
                .map_err(|e| {
                    ResourceError::initialization(
                        "mysql:1.0",
                        format!("Failed to connect to MySQL: {}", e),
                    )
                })?;

            Ok(MySqlInstance {
                instance_id: uuid::Uuid::new_v4(),
                resource_id: self.metadata().id,
                context: context.clone(),
                created_at: chrono::Utc::now(),
                last_accessed: parking_lot::Mutex::new(None),
                state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
                pool,
            })
        }

        #[cfg(not(feature = "mysql"))]
        {
            Ok(MySqlInstance {
                instance_id: uuid::Uuid::new_v4(),
                resource_id: self.metadata().id,
                context: context.clone(),
                created_at: chrono::Utc::now(),
                last_accessed: parking_lot::Mutex::new(None),
                state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
                url: config.url.clone(),
                max_connections: config.max_connections,
            })
        }
    }

    async fn cleanup(&self, instance: Self::Instance) -> ResourceResult<()> {
        #[cfg(feature = "mysql")]
        {
            instance.pool.close().await;
        }

        #[cfg(not(feature = "mysql"))]
        {
            let _ = instance;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        Ok(())
    }

    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        Ok(matches!(
            instance.lifecycle_state(),
            crate::core::lifecycle::LifecycleState::Ready
                | crate::core::lifecycle::LifecycleState::Idle
                | crate::core::lifecycle::LifecycleState::InUse
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::ResourceContextBuilder;

    #[tokio::test]
    async fn test_mysql_config_validation() {
        let mut config = MySqlConfig::default();
        config.url = "invalid://url".to_string();
        assert!(config.validate().is_err());

        config.url = "mysql://localhost/test".to_string();
        assert!(config.validate().is_ok());

        config.url = "mariadb://localhost/test".to_string();
        assert!(config.validate().is_ok());
    }

    #[tokio::test]
    async fn test_mysql_config_merge() {
        let mut config1 = MySqlConfig {
            url: "mysql://localhost/db1".to_string(),
            max_connections: 5,
            ..Default::default()
        };

        let config2 = MySqlConfig {
            url: "mysql://localhost/db2".to_string(),
            max_connections: 20,
            min_connections: 5,
            idle_timeout_seconds: Some(300),
            ..Default::default()
        };

        config1.merge(config2);
        assert_eq!(config1.url, "mysql://localhost/db2");
        assert_eq!(config1.max_connections, 20);
        assert_eq!(config1.min_connections, 5);
        assert_eq!(config1.idle_timeout_seconds, Some(300));
    }

    #[tokio::test]
    async fn test_mysql_resource_creation_mock() {
        let resource = MySqlResource;
        let config = MySqlConfig {
            url: "mysql://localhost/test".to_string(),
            ..Default::default()
        };
        let context = ResourceContextBuilder::default().build();

        let instance = resource.create(&config, &context).await.unwrap();
        assert_eq!(instance.instance_id().to_string().len(), 36);
    }

    #[tokio::test]
    async fn test_mysql_config_default() {
        let config = MySqlConfig::default();
        assert_eq!(config.max_connections, 10);
        assert_eq!(config.min_connections, 2);
        assert_eq!(config.timeout_seconds, 30);
        assert_eq!(config.idle_timeout_seconds, Some(600));
        assert_eq!(config.max_lifetime_seconds, Some(1800));
    }
}
