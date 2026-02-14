//! PostgreSQL database resource implementation

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
    traits::{HealthCheckable, HealthStatus},
};

#[cfg(feature = "postgres")]
use sqlx::{PgPool, Row, postgres::PgPoolOptions};

/// PostgreSQL configuration
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct PostgresConfig {
    /// PostgreSQL connection URL (e.g., "postgresql://user:pass@localhost/db")
    /// Can contain placeholders: {credential}, {password}, {token}
    pub url: String,
    /// Maximum number of connections in the pool
    pub max_connections: u32,
    /// Minimum number of connections to maintain
    pub min_connections: u32,
    /// Connection timeout in seconds
    pub timeout_seconds: u64,
    /// Optional credential configuration
    #[cfg(feature = "credentials")]
    pub credential: Option<crate::credentials::CredentialConfig>,
}

impl Default for PostgresConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            max_connections: 10,
            min_connections: 2,
            timeout_seconds: 30,
            #[cfg(feature = "credentials")]
            credential: None,
        }
    }
}

impl ResourceConfig for PostgresConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.url.is_empty() {
            return Err(ResourceError::configuration(
                "PostgreSQL URL cannot be empty",
            ));
        }

        if !self.url.starts_with("postgresql://") && !self.url.starts_with("postgres://") {
            return Err(ResourceError::configuration(
                "PostgreSQL URL must start with postgresql:// or postgres://",
            ));
        }

        if self.max_connections == 0 {
            return Err(ResourceError::configuration(
                "Max connections must be greater than 0",
            ));
        }

        if self.min_connections > self.max_connections {
            return Err(ResourceError::configuration(
                "Min connections cannot exceed max connections",
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

/// PostgreSQL instance
pub struct PostgresInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<crate::core::lifecycle::LifecycleState>,

    #[cfg(feature = "postgres")]
    pool: PgPool,

    #[cfg(not(feature = "postgres"))]
    url: String,
    #[cfg(not(feature = "postgres"))]
    max_connections: u32,

    #[cfg(feature = "credentials")]
    credential_provider: Option<std::sync::Arc<crate::credentials::ResourceCredentialProvider>>,
}

impl ResourceInstance for PostgresInstance {
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

impl PostgresInstance {
    /// Execute a database query
    #[cfg(feature = "postgres")]
    pub async fn execute_query(&self, query: &str) -> ResourceResult<u64> {
        self.touch();

        let result = sqlx::query(query).execute(&self.pool).await.map_err(|e| {
            ResourceError::internal("postgres:1.0", format!("Query execution failed: {}", e))
        })?;

        Ok(result.rows_affected())
    }

    /// Execute a database query (mock)
    #[cfg(not(feature = "postgres"))]
    pub async fn execute_query(&self, _query: &str) -> ResourceResult<u64> {
        self.touch();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        Ok(42)
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
            .map_err(|e| ResourceError::internal("postgres:1.0", format!("Fetch failed: {}", e)))
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
            .map_err(|e| ResourceError::internal("postgres:1.0", format!("Fetch failed: {}", e)))
    }

    /// Begin a transaction
    #[cfg(feature = "postgres")]
    pub async fn begin_transaction(&self) -> ResourceResult<sqlx::Transaction<'_, sqlx::Postgres>> {
        self.touch();

        self.pool.begin().await.map_err(|e| {
            ResourceError::internal("postgres:1.0", format!("Transaction start failed: {}", e))
        })
    }

    /// Get the underlying connection pool
    #[cfg(feature = "postgres")]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Get connection information (mock)
    #[cfg(not(feature = "postgres"))]
    pub fn connection_info(&self) -> (String, u32) {
        (self.url.clone(), self.max_connections)
    }
}

#[async_trait::async_trait]
impl HealthCheckable for PostgresInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        #[cfg(feature = "postgres")]
        {
            let start = std::time::Instant::now();

            match sqlx::query("SELECT 1").execute(&self.pool).await {
                Ok(_) => Ok(HealthStatus::healthy()),
                Err(e) => {
                    let latency = start.elapsed();
                    Ok(
                        HealthStatus::unhealthy(format!("PostgreSQL query failed: {}", e))
                            .with_latency(latency),
                    )
                }
            }
        }

        #[cfg(not(feature = "postgres"))]
        {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            Ok(HealthStatus::healthy())
        }
    }

    async fn detailed_health_check(
        &self,
        _context: &ResourceContext,
    ) -> ResourceResult<HealthStatus> {
        #[cfg(feature = "postgres")]
        {
            let start = std::time::Instant::now();

            let pool_options = self.pool.options();
            let size = self.pool.size() as usize;
            let max_size = pool_options.get_max_connections() as usize;

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
                    Ok(
                        HealthStatus::unhealthy(format!("PostgreSQL query failed: {}", e))
                            .with_latency(latency)
                            .with_metadata("pool_size", size.to_string())
                            .with_metadata("pool_max", max_size.to_string()),
                    )
                }
            }
        }

        #[cfg(not(feature = "postgres"))]
        {
            self.health_check().await
        }
    }
}

/// PostgreSQL resource
pub struct PostgresResource;

#[async_trait::async_trait]
impl Resource for PostgresResource {
    type Config = PostgresConfig;
    type Instance = PostgresInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("postgres", "1.0"),
            "PostgreSQL connection resource".to_string(),
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

        // Resolve connection URL with credentials if configured
        #[cfg(feature = "credentials")]
        let connection_url = if let Some(_cred_config) = &config.credential {
            // For now, just use the URL as-is
            // In a real implementation, we would:
            // 1. Get CredentialManager from context
            // 2. Create ResourceCredentialProvider
            // 3. Build connection string with credentials
            config.url.clone()
        } else {
            config.url.clone()
        };

        #[cfg(not(feature = "credentials"))]
        let connection_url = config.url.clone();

        #[cfg(feature = "postgres")]
        {
            let pool = PgPoolOptions::new()
                .max_connections(config.max_connections)
                .min_connections(config.min_connections)
                .acquire_timeout(std::time::Duration::from_secs(config.timeout_seconds))
                .connect(&connection_url)
                .await
                .map_err(|e| {
                    ResourceError::initialization(
                        "postgres:1.0",
                        format!("Failed to connect to PostgreSQL: {}", e),
                    )
                })?;

            Ok(PostgresInstance {
                instance_id: uuid::Uuid::new_v4(),
                resource_id: self.metadata().id,
                context: context.clone(),
                created_at: chrono::Utc::now(),
                last_accessed: parking_lot::Mutex::new(None),
                state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
                pool,
                #[cfg(feature = "credentials")]
                credential_provider: None, // Would be set from context in real implementation
            })
        }

        #[cfg(not(feature = "postgres"))]
        {
            Ok(PostgresInstance {
                instance_id: uuid::Uuid::new_v4(),
                resource_id: self.metadata().id,
                context: context.clone(),
                created_at: chrono::Utc::now(),
                last_accessed: parking_lot::Mutex::new(None),
                state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
                url: connection_url,
                max_connections: config.max_connections,
                #[cfg(feature = "credentials")]
                credential_provider: None,
            })
        }
    }

    async fn cleanup(&self, instance: Self::Instance) -> ResourceResult<()> {
        #[cfg(feature = "postgres")]
        {
            instance.pool.close().await;
        }

        #[cfg(not(feature = "postgres"))]
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
    async fn test_postgres_config_validation() {
        let mut config = PostgresConfig::default();
        config.url = "invalid://url".to_string();
        assert!(config.validate().is_err());

        config.url = "postgresql://localhost/test".to_string();
        assert!(config.validate().is_ok());

        config.url = "postgres://localhost/test".to_string();
        assert!(config.validate().is_ok());
    }

    #[tokio::test]
    async fn test_postgres_resource_creation_mock() {
        let resource = PostgresResource;
        let config = PostgresConfig {
            url: "postgresql://localhost/test".to_string(),
            ..Default::default()
        };
        let context = ResourceContextBuilder::default().build();

        let instance = resource.create(&config, &context).await.unwrap();
        assert_eq!(instance.instance_id().to_string().len(), 36);
    }
}
