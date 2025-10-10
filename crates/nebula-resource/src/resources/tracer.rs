//! Tracer resource implementation with OpenTelemetry integration

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
};

/// Tracer resource configuration for distributed tracing
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TracerConfig {
    /// OpenTelemetry collector endpoint (e.g., "<http://localhost:4317>")
    pub endpoint: String,
    /// Service name for traces
    pub service_name: String,
    /// Sample rate (0.0 to 1.0)
    #[cfg(feature = "tracing")]
    pub sample_rate: f64,
}

impl Default for TracerConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:4317".to_string(),
            service_name: "nebula-resource".to_string(),
            #[cfg(feature = "tracing")]
            sample_rate: 1.0,
        }
    }
}

impl ResourceConfig for TracerConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.endpoint.is_empty() {
            return Err(ResourceError::configuration(
                "Tracer endpoint cannot be empty",
            ));
        }

        if self.service_name.is_empty() {
            return Err(ResourceError::configuration("Service name cannot be empty"));
        }

        #[cfg(feature = "tracing")]
        {
            if self.sample_rate < 0.0 || self.sample_rate > 1.0 {
                return Err(ResourceError::configuration(
                    "Sample rate must be between 0.0 and 1.0",
                ));
            }
        }

        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.endpoint.is_empty() {
            self.endpoint = other.endpoint;
        }
        if !other.service_name.is_empty() {
            self.service_name = other.service_name;
        }
        #[cfg(feature = "tracing")]
        {
            self.sample_rate = other.sample_rate;
        }
    }
}

/// Tracer instance with OpenTelemetry integration
pub struct TracerInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<crate::core::lifecycle::LifecycleState>,
    service_name: String,
    endpoint: String,
}

impl ResourceInstance for TracerInstance {
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

/// Tracer resource with OpenTelemetry integration
pub struct TracerResource;

#[async_trait::async_trait]
impl Resource for TracerResource {
    type Config = TracerConfig;
    type Instance = TracerInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("tracer", "1.0"),
            "Distributed tracing resource".to_string(),
        )
        .with_default_scope(ResourceScope::Global)
    }

    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        config.validate()?;

        // Note: OpenTelemetry tracer initialization would go here when tracing feature is enabled
        // For now, we provide a basic structure that can be enhanced with actual OpenTelemetry setup

        Ok(TracerInstance {
            instance_id: uuid::Uuid::new_v4(),
            resource_id: self.metadata().id,
            context: context.clone(),
            created_at: chrono::Utc::now(),
            last_accessed: parking_lot::Mutex::new(None),
            state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
            service_name: config.service_name.clone(),
            endpoint: config.endpoint.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::ResourceContextBuilder;

    #[tokio::test]
    async fn test_tracer_resource_creation() {
        let resource = TracerResource;
        let config = TracerConfig::default();
        let context = ResourceContextBuilder::default().build();

        let instance = resource.create(&config, &context).await.unwrap();
        assert_eq!(instance.service_name, "nebula-resource");
        assert_eq!(instance.endpoint, "http://localhost:4317");
    }

    #[tokio::test]
    async fn test_tracer_config_validation() {
        let mut config = TracerConfig::default();
        config.service_name = "".to_string();

        assert!(config.validate().is_err());
    }

    #[cfg(feature = "tracing")]
    #[tokio::test]
    async fn test_tracer_sample_rate_validation() {
        let mut config = TracerConfig::default();
        config.sample_rate = 1.5; // Invalid: > 1.0

        assert!(config.validate().is_err());

        config.sample_rate = -0.1; // Invalid: < 0.0
        assert!(config.validate().is_err());

        config.sample_rate = 0.5; // Valid
        assert!(config.validate().is_ok());
    }
}
