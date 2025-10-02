//! Database resource implementation

use crate::core::{
    error::ResourceResult,
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
};

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
    last_accessed: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: std::sync::RwLock<crate::core::lifecycle::LifecycleState>,
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
        *self.state.read().unwrap()
    }

    fn context(&self) -> &crate::core::context::ResourceContext {
        &self.context
    }

    fn created_at(&self) -> chrono::DateTime<chrono::Utc> {
        self.created_at
    }

    fn last_accessed_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        *self.last_accessed.lock().unwrap()
    }

    fn touch(&self) {
        *self.last_accessed.lock().unwrap() = Some(chrono::Utc::now());
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

        Ok(DatabaseInstance {
            instance_id: uuid::Uuid::new_v4(),
            resource_id: self.metadata().id,
            context: context.clone(),
            created_at: chrono::Utc::now(),
            last_accessed: std::sync::Mutex::new(None),
            state: std::sync::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
            url: config.url.clone(),
            max_connections: config.max_connections,
        })
    }

    async fn cleanup(&self, _instance: Self::Instance) -> ResourceResult<()> {
        // Simulate connection cleanup
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

impl DatabaseInstance {
    /// Simulate a database query
    pub async fn execute_query(&self, _query: &str) -> ResourceResult<u64> {
        // Update last accessed time
        *self.last_accessed.lock().unwrap() = Some(chrono::Utc::now());

        // Simulate query execution
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        Ok(42) // Mock result
    }

    /// Get connection information
    pub fn connection_info(&self) -> (String, u32) {
        (self.url.clone(), self.max_connections)
    }
}