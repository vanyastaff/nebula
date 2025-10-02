//! Storage resource implementation

use crate::core::{
    error::ResourceResult,
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
};

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub endpoint: String,
    pub bucket: String,
    pub region: String,
}

impl ResourceConfig for StorageConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.endpoint.is_empty() {
            return Err(crate::core::error::ResourceError::configuration(
                "Storage endpoint cannot be empty",
            ));
        }

        if self.bucket.is_empty() {
            return Err(crate::core::error::ResourceError::configuration(
                "Storage bucket cannot be empty",
            ));
        }

        if self.region.is_empty() {
            return Err(crate::core::error::ResourceError::configuration(
                "Storage region cannot be empty",
            ));
        }

        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.endpoint.is_empty() {
            self.endpoint = other.endpoint;
        }
        if !other.bucket.is_empty() {
            self.bucket = other.bucket;
        }
        if !other.region.is_empty() {
            self.region = other.region;
        }
    }
}

pub struct StorageInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: crate::core::context::ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<crate::core::lifecycle::LifecycleState>,
    endpoint: String,
    bucket: String,
}

impl ResourceInstance for StorageInstance {
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

pub struct StorageResource;

#[async_trait::async_trait]
impl Resource for StorageResource {
    type Config = StorageConfig;
    type Instance = StorageInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("storage", "1.0"),
            "Object storage resource".to_string(),
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

        Ok(StorageInstance {
            instance_id: uuid::Uuid::new_v4(),
            resource_id: self.metadata().id,
            context: context.clone(),
            created_at: chrono::Utc::now(),
            last_accessed: parking_lot::Mutex::new(None),
            state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
            endpoint: config.endpoint.clone(),
            bucket: config.bucket.clone(),
        })
    }
}
