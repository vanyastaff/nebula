//! Observability resource implementations

use crate::core::{
    error::ResourceResult,
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
};

// Logger Resource
#[derive(Debug, Clone)]
pub struct LoggerConfig {
    pub level: String,
    pub format: String,
    pub output: String,
}

impl ResourceConfig for LoggerConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.level.is_empty() {
            return Err(crate::core::error::ResourceError::configuration("Log level cannot be empty"));
        }

        match self.level.to_lowercase().as_str() {
            "trace" | "debug" | "info" | "warn" | "error" => {},
            _ => return Err(crate::core::error::ResourceError::configuration("Invalid log level")),
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

pub struct LoggerInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: crate::core::context::ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: std::sync::RwLock<crate::core::lifecycle::LifecycleState>,
    level: String,
}

impl ResourceInstance for LoggerInstance {
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

        Ok(LoggerInstance {
            instance_id: uuid::Uuid::new_v4(),
            resource_id: self.metadata().id,
            context: context.clone(),
            created_at: chrono::Utc::now(),
            last_accessed: std::sync::Mutex::new(None),
            state: std::sync::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
            level: config.level.clone(),
        })
    }
}

// Metrics Resource
#[derive(Debug, Clone)]
pub struct MetricsConfig {
    pub endpoint: String,
    pub namespace: String,
}

impl ResourceConfig for MetricsConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.endpoint.is_empty() {
            return Err(crate::core::error::ResourceError::configuration("Metrics endpoint cannot be empty"));
        }

        if self.namespace.is_empty() {
            return Err(crate::core::error::ResourceError::configuration("Metrics namespace cannot be empty"));
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
    }
}

pub struct MetricsInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: crate::core::context::ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: std::sync::RwLock<crate::core::lifecycle::LifecycleState>,
    endpoint: String,
}

impl ResourceInstance for MetricsInstance {
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

        Ok(MetricsInstance {
            instance_id: uuid::Uuid::new_v4(),
            resource_id: self.metadata().id,
            context: context.clone(),
            created_at: chrono::Utc::now(),
            last_accessed: std::sync::Mutex::new(None),
            state: std::sync::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
            endpoint: config.endpoint.clone(),
        })
    }
}

// Tracer Resource
#[derive(Debug, Clone)]
pub struct TracerConfig {
    pub endpoint: String,
    pub service_name: String,
}

impl ResourceConfig for TracerConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.endpoint.is_empty() {
            return Err(crate::core::error::ResourceError::configuration("Tracer endpoint cannot be empty"));
        }

        if self.service_name.is_empty() {
            return Err(crate::core::error::ResourceError::configuration("Service name cannot be empty"));
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
    }
}

pub struct TracerInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: crate::core::context::ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: std::sync::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: std::sync::RwLock<crate::core::lifecycle::LifecycleState>,
    service_name: String,
}

impl ResourceInstance for TracerInstance {
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

        Ok(TracerInstance {
            instance_id: uuid::Uuid::new_v4(),
            resource_id: self.metadata().id,
            context: context.clone(),
            created_at: chrono::Utc::now(),
            last_accessed: std::sync::Mutex::new(None),
            state: std::sync::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
            service_name: config.service_name.clone(),
        })
    }
}