//! Cache resource implementation

use crate::core::{
    error::ResourceResult,
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
};

/// Cache resource configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Cache URL/connection string
    pub url: String,
    /// Maximum connections
    pub max_connections: u32,
}

impl ResourceConfig for CacheConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.url.is_empty() {
            return Err(crate::core::error::ResourceError::configuration("Cache URL cannot be empty"));
        }

        if self.max_connections == 0 {
            return Err(crate::core::error::ResourceError::configuration("Max connections must be greater than 0"));
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
    }
}

/// Cache resource instance
pub struct CacheInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: crate::core::context::ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: std::sync::RwLock<crate::core::lifecycle::LifecycleState>,
    url: String,
    max_connections: u32,
}

impl ResourceInstance for CacheInstance {
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

    fn touch(&mut self) {
        *self.last_accessed.lock().unwrap() = Some(chrono::Utc::now());
    }
}

/// Cache resource
pub struct CacheResource;

#[async_trait::async_trait]
impl Resource for CacheResource {
    type Config = CacheConfig;
    type Instance = CacheInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("cache", "1.0"),
            "Cache resource for data caching".to_string(),
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

        Ok(CacheInstance {
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
}