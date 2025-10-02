//! Generic database resource implementation
//!
//! This module provides a generic database resource for testing and development.
//! For production use, consider using specific database resources:
//! - `PostgresResource` - PostgreSQL (feature: postgres)
//! - `MySqlResource` - MySQL/MariaDB (feature: mysql)
//! - `MongoDbResource` - MongoDB (feature: mongodb)

use crate::core::{
    error::ResourceResult,
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
};

/// Generic database resource configuration
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            max_connections: 10,
            min_connections: 2,
            timeout_seconds: 30,
        }
    }
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

/// Generic database resource instance (mock implementation)
pub struct DatabaseInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: crate::core::context::ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<crate::core::lifecycle::LifecycleState>,
    url: String,
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

impl DatabaseInstance {
    /// Execute a mock database query
    pub async fn execute_query(&self, _query: &str) -> ResourceResult<u64> {
        self.touch();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        Ok(42) // Mock result
    }

    /// Get connection information
    pub fn connection_info(&self) -> (&str, u32) {
        (&self.url, self.max_connections)
    }
}

/// Generic database resource (mock implementation)
///
/// For production use, consider using specific database resources:
/// - `PostgresResource` - PostgreSQL
/// - `MySqlResource` - MySQL/MariaDB
/// - `MongoDbResource` - MongoDB
pub struct DatabaseResource;

#[async_trait::async_trait]
impl Resource for DatabaseResource {
    type Config = DatabaseConfig;
    type Instance = DatabaseInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("database", "1.0"),
            "Generic database resource (mock)".to_string(),
        )
        .poolable()
        .with_default_scope(ResourceScope::Global)
    }

    async fn create(
        &self,
        config: &Self::Config,
        context: &crate::core::context::ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        config.validate()?;

        // Simulate connection delay
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

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

    async fn cleanup(&self, _instance: Self::Instance) -> ResourceResult<()> {
        // Simulate cleanup
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
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
    async fn test_database_config_validation() {
        let mut config = DatabaseConfig::default();
        config.url = "".to_string();
        assert!(config.validate().is_err());

        config.url = "mock://localhost/test".to_string();
        assert!(config.validate().is_ok());
    }

    #[tokio::test]
    async fn test_database_resource_creation() {
        let resource = DatabaseResource;
        let config = DatabaseConfig {
            url: "mock://localhost/test".to_string(),
            ..Default::default()
        };
        let context = ResourceContextBuilder::default().build();

        let instance = resource.create(&config, &context).await.unwrap();
        assert_eq!(instance.instance_id().to_string().len(), 36);

        let (url, max_conn) = instance.connection_info();
        assert_eq!(url, "mock://localhost/test");
        assert_eq!(max_conn, 10);
    }

    #[tokio::test]
    async fn test_database_execute_query() {
        let resource = DatabaseResource;
        let config = DatabaseConfig {
            url: "mock://localhost/test".to_string(),
            ..Default::default()
        };
        let context = ResourceContextBuilder::default().build();

        let instance = resource.create(&config, &context).await.unwrap();
        let result = instance.execute_query("SELECT 1").await.unwrap();
        assert_eq!(result, 42);
    }
}
