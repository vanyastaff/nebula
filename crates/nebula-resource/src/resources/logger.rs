//! Logger resource implementation with nebula-log integration

use crate::core::{
    error::{ResourceError, ResourceResult},
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
};

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
            "trace" | "debug" | "info" | "warn" | "error" => {}
            _ => {
                return Err(ResourceError::configuration(
                    "Invalid log level. Must be one of: trace, debug, info, warn, error",
                ));
            }
        }

        match self.format.to_lowercase().as_str() {
            "json" | "pretty" | "compact" => {}
            _ => {
                return Err(ResourceError::configuration(
                    "Invalid format. Must be one of: json, pretty, compact",
                ));
            }
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
#[derive(Debug)]
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

/// Logger resource with nebula-log integration
#[derive(Debug)]
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

    async fn create(
        &self,
        config: &Self::Config,
        context: &crate::core::context::ResourceContext,
    ) -> ResourceResult<Self::Instance> {
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
            level: config.level.clone(), // level is String in nebula-log Config
            format,
            ..Default::default()
        };

        // Initialize logger with nebula-log
        // Note: The LoggerGuard is dropped immediately after initialization, which is fine
        // because nebula-log sets up a global logger that persists
        let _guard = nebula_log::init_with(logger_config).map_err(|e| {
            ResourceError::initialization("logger:1.0", format!("Failed to initialize logger: {e}"))
        })?;

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
}
