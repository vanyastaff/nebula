//! Observability resource implementations
//!
//! This module provides resource implementations for observability:
//! - **LoggerResource**: Structured logging with nebula-log integration
//! - **MetricsResource**: Prometheus metrics collection and export
//! - **TracerResource**: OpenTelemetry distributed tracing
//!
//! # Features
//!
//! - `metrics` - Enable Prometheus metrics export
//! - `tracing` - Enable OpenTelemetry tracing
//!
//! # Example
//!
//! ```rust,no_run
//! use nebula_resource::resources::observability::{MetricsResource, MetricsConfig};
//!
//! let metrics_resource = MetricsResource;
//! let config = MetricsConfig {
//!     endpoint: "0.0.0.0:9090".to_string(),
//!     namespace: "nebula".to_string(),
//!     enable_process_metrics: true,
//! };
//! ```

use crate::core::{
    error::{ResourceError, ResourceResult},
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
};

use std::sync::Arc;

// ============================================================================
// Logger Resource - Structured Logging with nebula-log
// ============================================================================

/// Logger resource configuration
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LoggerConfig {
    /// Log level: trace, debug, info, warn, error
    pub level: String,
    /// Log format: json, pretty, compact
    pub format: String,
    /// Output destination: stdout, stderr, or file path
    pub output: String,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            format: "json".to_string(),
            output: "stdout".to_string(),
        }
    }
}

impl ResourceConfig for LoggerConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.level.is_empty() {
            return Err(ResourceError::configuration("Log level cannot be empty"));
        }

        match self.level.to_lowercase().as_str() {
            "trace" | "debug" | "info" | "warn" | "error" => {},
            _ => return Err(ResourceError::configuration("Invalid log level. Must be one of: trace, debug, info, warn, error")),
        }

        match self.format.to_lowercase().as_str() {
            "json" | "pretty" | "compact" => {},
            _ => return Err(ResourceError::configuration("Invalid format. Must be one of: json, pretty, compact")),
        }

        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.level.is_empty() {
            self.level = other.level;
        }
        if !other.format.is_empty() {
            self.format = other.format;
        }
        if !other.output.is_empty() {
            self.output = other.output;
        }
    }
}

/// Logger instance with nebula-log integration
pub struct LoggerInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: crate::core::context::ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<crate::core::lifecycle::LifecycleState>,
    level: String,
    format: String,
}

impl ResourceInstance for LoggerInstance {
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

pub struct LoggerResource;

#[async_trait::async_trait]
impl Resource for LoggerResource {
    type Config = LoggerConfig;
    type Instance = LoggerInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("logger", "1.0"),
            "Structured logging resource".to_string(),
        )
        .with_default_scope(ResourceScope::Global)
    }

    async fn create(&self, config: &Self::Config, context: &crate::core::context::ResourceContext) -> ResourceResult<Self::Instance> {
        config.validate()?;

        // Parse format
        let format = match config.format.to_lowercase().as_str() {
            "json" => nebula_log::Format::Json,
            "pretty" => nebula_log::Format::Pretty,
            "compact" => nebula_log::Format::Compact,
            _ => nebula_log::Format::Json,
        };

        // Initialize logger with nebula-log
        let logger_config = nebula_log::Config {
            level: config.level.clone(),  // level is String in nebula-log Config
            format,
            ..Default::default()
        };

        // Initialize logger with nebula-log
        // Note: The LoggerGuard is dropped immediately after initialization, which is fine
        // because nebula-log sets up a global logger that persists
        let _guard = nebula_log::init_with(logger_config)
            .map_err(|e| ResourceError::initialization(
                "logger:1.0",
                format!("Failed to initialize logger: {}", e)
            ))?;

        Ok(LoggerInstance {
            instance_id: uuid::Uuid::new_v4(),
            resource_id: self.metadata().id,
            context: context.clone(),
            created_at: chrono::Utc::now(),
            last_accessed: parking_lot::Mutex::new(None),
            state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
            level: config.level.clone(),
            format: config.format.clone(),
        })
    }
}

// ============================================================================
// Metrics Resource - Prometheus Integration
// ============================================================================

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
            return Err(ResourceError::configuration("Metrics endpoint cannot be empty"));
        }

        if self.namespace.is_empty() {
            return Err(ResourceError::configuration("Metrics namespace cannot be empty"));
        }

        // Validate endpoint format
        if !self.endpoint.contains(':') {
            return Err(ResourceError::configuration("Metrics endpoint must include port (e.g., 0.0.0.0:9090)"));
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

    async fn create(&self, config: &Self::Config, context: &crate::core::context::ResourceContext) -> ResourceResult<Self::Instance> {
        config.validate()?;

        #[cfg(feature = "metrics")]
        let recorder = {
            let builder = metrics_exporter_prometheus::PrometheusBuilder::new();
            let handle = builder
                .install_recorder()
                .map_err(|e| ResourceError::initialization(
                    "metrics:1.0",
                    format!("Failed to install Prometheus recorder: {}", e)
                ))?;
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

// ============================================================================
// Tracer Resource - OpenTelemetry Integration
// ============================================================================

/// Tracer resource configuration for distributed tracing
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TracerConfig {
    /// OpenTelemetry collector endpoint (e.g., "http://localhost:4317")
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
            return Err(ResourceError::configuration("Tracer endpoint cannot be empty"));
        }

        if self.service_name.is_empty() {
            return Err(ResourceError::configuration("Service name cannot be empty"));
        }

        #[cfg(feature = "tracing")]
        {
            if self.sample_rate < 0.0 || self.sample_rate > 1.0 {
                return Err(ResourceError::configuration("Sample rate must be between 0.0 and 1.0"));
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
    context: crate::core::context::ResourceContext,
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

    async fn create(&self, config: &Self::Config, context: &crate::core::context::ResourceContext) -> ResourceResult<Self::Instance> {
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
    async fn test_logger_resource_creation() {
        let resource = LoggerResource;
        let config = LoggerConfig::default();
        let context = ResourceContextBuilder::default().build();

        let instance = resource.create(&config, &context).await.unwrap();
        assert_eq!(instance.level, "info");
        assert_eq!(instance.format, "json");
    }

    #[tokio::test]
    async fn test_logger_config_validation() {
        let mut config = LoggerConfig::default();
        config.level = "invalid".to_string();

        assert!(config.validate().is_err());
    }

    #[tokio::test]
    async fn test_logger_config_merge() {
        let mut config1 = LoggerConfig::default();
        let config2 = LoggerConfig {
            level: "debug".to_string(),
            format: "pretty".to_string(),
            output: "stderr".to_string(),
        };

        config1.merge(config2);
        assert_eq!(config1.level, "debug");
        assert_eq!(config1.format, "pretty");
        assert_eq!(config1.output, "stderr");
    }

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