//! Metrics resource implementation with Prometheus integration

use crate::core::{
    error::{ResourceError, ResourceResult},
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
};

use std::sync::Arc;

/// Metrics resource configuration for Prometheus export
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MetricsConfig {
    /// Metrics HTTP endpoint (e.g., "0.0.0.0:9090")
    pub endpoint: String,
    /// Metrics namespace prefix (e.g., "nebula_resource")
    pub namespace: String,
    /// Enable automatic process metrics (CPU, memory, etc.)
    #[cfg(feature = "metrics")]
    pub enable_process_metrics: bool,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            endpoint: "0.0.0.0:9090".to_string(),
            namespace: "nebula_resource".to_string(),
            #[cfg(feature = "metrics")]
            enable_process_metrics: true,
        }
    }
}

impl ResourceConfig for MetricsConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.endpoint.is_empty() {
            return Err(ResourceError::configuration(
                "Metrics endpoint cannot be empty",
            ));
        }

        if self.namespace.is_empty() {
            return Err(ResourceError::configuration(
                "Metrics namespace cannot be empty",
            ));
        }

        // Validate endpoint format
        if !self.endpoint.contains(':') {
            return Err(ResourceError::configuration(
                "Metrics endpoint must include port (e.g., 0.0.0.0:9090)",
            ));
        }

        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.endpoint.is_empty() {
            self.endpoint = other.endpoint;
        }
        if !other.namespace.is_empty() {
            self.namespace = other.namespace;
        }
        #[cfg(feature = "metrics")]
        {
            self.enable_process_metrics = other.enable_process_metrics;
        }
    }
}

/// Metrics instance with Prometheus exporter
pub struct MetricsInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: crate::core::context::ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<crate::core::lifecycle::LifecycleState>,
    endpoint: String,
    namespace: String,
    #[cfg(feature = "metrics")]
    recorder: Option<Arc<metrics_exporter_prometheus::PrometheusHandle>>,
}

impl ResourceInstance for MetricsInstance {
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

impl MetricsInstance {
    /// Record a counter increment
    #[cfg(feature = "metrics")]
    pub fn increment_counter(&self, name: &str, value: u64) {
        self.touch();
        let full_name = format!("{}_{}", self.namespace, name);
        metrics::counter!(&full_name).increment(value);
    }

    /// Record a gauge value
    #[cfg(feature = "metrics")]
    pub fn record_gauge(&self, name: &str, value: f64) {
        self.touch();
        let full_name = format!("{}_{}", self.namespace, name);
        metrics::gauge!(&full_name).set(value);
    }

    /// Record a histogram value (for latencies, sizes, etc.)
    #[cfg(feature = "metrics")]
    pub fn record_histogram(&self, name: &str, value: f64) {
        self.touch();
        let full_name = format!("{}_{}", self.namespace, name);
        metrics::histogram!(&full_name).record(value);
    }

    /// Get the current metrics snapshot in Prometheus format
    #[cfg(feature = "metrics")]
    pub fn render(&self) -> String {
        if let Some(ref recorder) = self.recorder {
            recorder.render()
        } else {
            String::new()
        }
    }

    /// Get the metrics endpoint address
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

/// Metrics resource with Prometheus integration
pub struct MetricsResource;

#[async_trait::async_trait]
impl Resource for MetricsResource {
    type Config = MetricsConfig;
    type Instance = MetricsInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("metrics", "1.0"),
            "Metrics collection resource".to_string(),
        )
        .with_default_scope(ResourceScope::Global)
    }

    async fn create(
        &self,
        config: &Self::Config,
        context: &crate::core::context::ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        config.validate()?;

        #[cfg(feature = "metrics")]
        let recorder = {
            let builder = metrics_exporter_prometheus::PrometheusBuilder::new();
            let handle = builder.install_recorder().map_err(|e| {
                ResourceError::initialization(
                    "metrics:1.0",
                    format!("Failed to install Prometheus recorder: {}", e),
                )
            })?;
            Some(Arc::new(handle))
        };

        Ok(MetricsInstance {
            instance_id: uuid::Uuid::new_v4(),
            resource_id: self.metadata().id,
            context: context.clone(),
            created_at: chrono::Utc::now(),
            last_accessed: parking_lot::Mutex::new(None),
            state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
            endpoint: config.endpoint.clone(),
            namespace: config.namespace.clone(),
            #[cfg(feature = "metrics")]
            recorder,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::context::ResourceContextBuilder;

    #[tokio::test]
    async fn test_metrics_resource_creation() {
        let resource = MetricsResource;
        let config = MetricsConfig::default();
        let context = ResourceContextBuilder::default().build();

        let instance = resource.create(&config, &context).await.unwrap();
        assert_eq!(instance.endpoint, "0.0.0.0:9090");
        assert_eq!(instance.namespace, "nebula_resource");
    }

    #[tokio::test]
    async fn test_metrics_config_validation() {
        let mut config = MetricsConfig::default();
        config.endpoint = "invalid".to_string(); // Missing port

        assert!(config.validate().is_err());
    }

    #[cfg(feature = "metrics")]
    #[tokio::test]
    async fn test_metrics_recording() {
        let resource = MetricsResource;
        let config = MetricsConfig::default();
        let context = ResourceContextBuilder::default().build();

        let instance = resource.create(&config, &context).await.unwrap();

        // Record some metrics
        instance.increment_counter("test_counter", 1);
        instance.record_gauge("test_gauge", 42.0);
        instance.record_histogram("test_histogram", 100.0);

        // Verify metrics can be rendered
        let metrics_output = instance.render();
        assert!(!metrics_output.is_empty());
    }
}
