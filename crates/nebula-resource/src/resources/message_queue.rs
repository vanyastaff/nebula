//! Message queue resource implementation

use crate::core::{
    error::ResourceResult,
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
};

#[derive(Debug, Clone)]
pub struct MessageQueueConfig {
    pub broker_url: String,
    pub topic_prefix: String,
}

impl ResourceConfig for MessageQueueConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.broker_url.is_empty() {
            return Err(crate::core::error::ResourceError::configuration("Broker URL cannot be empty"));
        }

        if self.topic_prefix.is_empty() {
            return Err(crate::core::error::ResourceError::configuration("Topic prefix cannot be empty"));
        }

        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.broker_url.is_empty() {
            self.broker_url = other.broker_url;
        }
        if !other.topic_prefix.is_empty() {
            self.topic_prefix = other.topic_prefix;
        }
    }
}

pub struct MessageQueueInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: crate::core::context::ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: std::sync::RwLock<crate::core::lifecycle::LifecycleState>,
    broker_url: String,
}

impl ResourceInstance for MessageQueueInstance {
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

pub struct MessageQueueResource;

#[async_trait::async_trait]
impl Resource for MessageQueueResource {
    type Config = MessageQueueConfig;
    type Instance = MessageQueueInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("message_queue", "1.0"),
            "Message queue for async communication".to_string(),
        )
        .poolable()
        .health_checkable()
        .with_default_scope(ResourceScope::Global)
    }

    async fn create(&self, config: &Self::Config, context: &crate::core::context::ResourceContext) -> ResourceResult<Self::Instance> {
        config.validate()?;

        Ok(MessageQueueInstance {
            instance_id: uuid::Uuid::new_v4(),
            resource_id: self.metadata().id,
            context: context.clone(),
            created_at: chrono::Utc::now(),
            last_accessed: std::sync::Mutex::new(None),
            state: std::sync::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
            broker_url: config.broker_url.clone(),
        })
    }
}