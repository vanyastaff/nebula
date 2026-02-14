//! MongoDB database resource implementation

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
    traits::{HealthCheckable, HealthStatus},
};

#[cfg(feature = "mongodb")]
use mongodb::{Client as MongoClient, options::ClientOptions};

/// MongoDB configuration
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MongoDbConfig {
    /// MongoDB connection URL (e.g., "mongodb://localhost:27017")
    pub url: String,
    /// Database name
    pub database: String,
    /// Application name for connection metadata
    pub app_name: Option<String>,
    /// Maximum pool size
    pub max_pool_size: Option<u32>,
    /// Minimum pool size
    pub min_pool_size: Option<u32>,
    /// Connection timeout in seconds
    pub timeout_seconds: u64,
}

impl Default for MongoDbConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            database: String::new(),
            app_name: Some("nebula-resource".to_string()),
            max_pool_size: Some(10),
            min_pool_size: Some(2),
            timeout_seconds: 30,
        }
    }
}

impl ResourceConfig for MongoDbConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.url.is_empty() {
            return Err(ResourceError::configuration("MongoDB URL cannot be empty"));
        }

        if !self.url.starts_with("mongodb://") && !self.url.starts_with("mongodb+srv://") {
            return Err(ResourceError::configuration(
                "MongoDB URL must start with mongodb:// or mongodb+srv://",
            ));
        }

        if self.database.is_empty() {
            return Err(ResourceError::configuration(
                "Database name cannot be empty",
            ));
        }

        if self.timeout_seconds == 0 {
            return Err(ResourceError::configuration(
                "Timeout must be greater than 0",
            ));
        }

        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.url.is_empty() {
            self.url = other.url;
        }
        if !other.database.is_empty() {
            self.database = other.database;
        }
        self.app_name = other.app_name.or_else(|| self.app_name.clone());
        self.max_pool_size = other.max_pool_size.or(self.max_pool_size);
        self.min_pool_size = other.min_pool_size.or(self.min_pool_size);
        if other.timeout_seconds > 0 {
            self.timeout_seconds = other.timeout_seconds;
        }
    }
}

/// MongoDB instance
pub struct MongoDbInstance {
    instance_id: uuid::Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<crate::core::lifecycle::LifecycleState>,
    database_name: String,

    #[cfg(feature = "mongodb")]
    client: MongoClient,

    #[cfg(not(feature = "mongodb"))]
    url: String,
}

impl ResourceInstance for MongoDbInstance {
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

impl MongoDbInstance {
    /// Get the MongoDB database
    #[cfg(feature = "mongodb")]
    pub fn database(&self) -> mongodb::Database {
        self.touch();
        self.client.database(&self.database_name)
    }

    /// Get a collection from the database
    #[cfg(feature = "mongodb")]
    pub fn collection<T>(&self, name: &str) -> mongodb::Collection<T>
    where
        T: Send + Sync,
    {
        self.touch();
        self.database().collection(name)
    }

    /// Get the underlying MongoDB client
    #[cfg(feature = "mongodb")]
    pub fn client(&self) -> &MongoClient {
        &self.client
    }

    /// Get connection information (mock)
    #[cfg(not(feature = "mongodb"))]
    pub fn connection_info(&self) -> (String, String) {
        (self.url.clone(), self.database_name.clone())
    }
}

#[async_trait::async_trait]
impl HealthCheckable for MongoDbInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        #[cfg(feature = "mongodb")]
        {
            use mongodb::bson::doc;

            let start = std::time::Instant::now();

            match self.database().run_command(doc! { "ping": 1 }).await {
                Ok(_) => Ok(HealthStatus::healthy()),
                Err(e) => {
                    let latency = start.elapsed();
                    Ok(
                        HealthStatus::unhealthy(format!("MongoDB ping failed: {}", e))
                            .with_latency(latency),
                    )
                }
            }
        }

        #[cfg(not(feature = "mongodb"))]
        {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            Ok(HealthStatus::healthy())
        }
    }

    async fn detailed_health_check(
        &self,
        _context: &ResourceContext,
    ) -> ResourceResult<HealthStatus> {
        #[cfg(feature = "mongodb")]
        {
            use mongodb::bson::doc;

            let start = std::time::Instant::now();

            match self
                .database()
                .run_command(doc! { "serverStatus": 1 })
                .await
            {
                Ok(status) => {
                    let latency = start.elapsed();

                    let mut health = HealthStatus::healthy().with_latency(latency);

                    if let Ok(version) = status.get_str("version") {
                        health = health.with_metadata("version", version.to_string());
                    }

                    if let Some(connections) = status.get_document("connections").ok() {
                        if let Ok(current) = connections.get_i32("current") {
                            health =
                                health.with_metadata("connections_current", current.to_string());
                        }
                        if let Ok(available) = connections.get_i32("available") {
                            health = health
                                .with_metadata("connections_available", available.to_string());
                        }
                    }

                    health = health.with_metadata("database", self.database_name.clone());

                    Ok(health)
                }
                Err(e) => {
                    let latency = start.elapsed();
                    Ok(
                        HealthStatus::unhealthy(format!("MongoDB serverStatus failed: {}", e))
                            .with_latency(latency)
                            .with_metadata("database", self.database_name.clone()),
                    )
                }
            }
        }

        #[cfg(not(feature = "mongodb"))]
        {
            self.health_check().await
        }
    }
}

/// MongoDB resource
pub struct MongoDbResource;

#[async_trait::async_trait]
impl Resource for MongoDbResource {
    type Config = MongoDbConfig;
    type Instance = MongoDbInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("mongodb", "1.0"),
            "MongoDB connection resource".to_string(),
        )
        .poolable()
        .health_checkable()
        .with_default_scope(ResourceScope::Global)
    }

    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        config.validate()?;

        #[cfg(feature = "mongodb")]
        {
            use std::time::Duration;

            let mut client_options = ClientOptions::parse(&config.url).await.map_err(|e| {
                ResourceError::initialization(
                    "mongodb:1.0",
                    format!("Failed to parse MongoDB URL: {}", e),
                )
            })?;

            if let Some(ref app_name) = config.app_name {
                client_options.app_name = Some(app_name.clone());
            }

            if let Some(max_pool_size) = config.max_pool_size {
                client_options.max_pool_size = Some(max_pool_size);
            }

            if let Some(min_pool_size) = config.min_pool_size {
                client_options.min_pool_size = Some(min_pool_size);
            }

            client_options.connect_timeout = Some(Duration::from_secs(config.timeout_seconds));

            let client = MongoClient::with_options(client_options).map_err(|e| {
                ResourceError::initialization(
                    "mongodb:1.0",
                    format!("Failed to create MongoDB client: {}", e),
                )
            })?;

            // Test connection with ping
            use mongodb::bson::doc;
            let db = client.database(&config.database);
            db.run_command(doc! { "ping": 1 }).await.map_err(|e| {
                ResourceError::initialization(
                    "mongodb:1.0",
                    format!("Failed to connect to MongoDB: {}", e),
                )
            })?;

            Ok(MongoDbInstance {
                instance_id: uuid::Uuid::new_v4(),
                resource_id: self.metadata().id,
                context: context.clone(),
                created_at: chrono::Utc::now(),
                last_accessed: parking_lot::Mutex::new(None),
                state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
                database_name: config.database.clone(),
                client,
            })
        }

        #[cfg(not(feature = "mongodb"))]
        {
            Ok(MongoDbInstance {
                instance_id: uuid::Uuid::new_v4(),
                resource_id: self.metadata().id,
                context: context.clone(),
                created_at: chrono::Utc::now(),
                last_accessed: parking_lot::Mutex::new(None),
                state: parking_lot::RwLock::new(crate::core::lifecycle::LifecycleState::Ready),
                database_name: config.database.clone(),
                url: config.url.clone(),
            })
        }
    }

    async fn cleanup(&self, _instance: Self::Instance) -> ResourceResult<()> {
        // MongoDB client automatically handles cleanup
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
    async fn test_mongodb_config_validation() {
        let mut config = MongoDbConfig::default();
        config.url = "invalid://url".to_string();
        config.database = "testdb".to_string();
        assert!(config.validate().is_err());

        config.url = "mongodb://localhost:27017".to_string();
        assert!(config.validate().is_ok());

        config.url = "mongodb+srv://cluster.example.com".to_string();
        assert!(config.validate().is_ok());
    }

    #[tokio::test]
    async fn test_mongodb_config_merge() {
        let mut config1 = MongoDbConfig {
            url: "mongodb://localhost:27017".to_string(),
            database: "db1".to_string(),
            max_pool_size: Some(5),
            ..Default::default()
        };

        let config2 = MongoDbConfig {
            url: "mongodb://localhost:27018".to_string(),
            database: "db2".to_string(),
            max_pool_size: Some(20),
            min_pool_size: Some(5),
            ..Default::default()
        };

        config1.merge(config2);
        assert_eq!(config1.url, "mongodb://localhost:27018");
        assert_eq!(config1.database, "db2");
        assert_eq!(config1.max_pool_size, Some(20));
        assert_eq!(config1.min_pool_size, Some(5));
    }

    #[tokio::test]
    async fn test_mongodb_resource_creation_mock() {
        let resource = MongoDbResource;
        let config = MongoDbConfig {
            url: "mongodb://localhost:27017".to_string(),
            database: "testdb".to_string(),
            ..Default::default()
        };
        let context = ResourceContextBuilder::default().build();

        let instance = resource.create(&config, &context).await.unwrap();
        assert_eq!(instance.instance_id().to_string().len(), 36);
        assert_eq!(instance.database_name, "testdb");
    }

    #[tokio::test]
    async fn test_mongodb_config_default() {
        let config = MongoDbConfig::default();
        assert_eq!(config.max_pool_size, Some(10));
        assert_eq!(config.min_pool_size, Some(2));
        assert_eq!(config.timeout_seconds, 30);
        assert_eq!(config.app_name, Some("nebula-resource".to_string()));
    }
}
