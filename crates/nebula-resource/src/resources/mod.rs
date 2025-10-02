//! Built-in resource implementations

/// Database resources
pub mod database;
#[cfg(feature = "mongodb")]
pub mod mongodb;
#[cfg(feature = "mysql")]
pub mod mysql;
#[cfg(feature = "postgres")]
pub mod postgres;

/// Cache resources
pub mod cache;
pub mod memory_cache;
#[cfg(feature = "redis")]
pub mod redis_cache;

/// HTTP client resources
pub mod http_client;

#[cfg(feature = "kafka")]
pub mod kafka;
/// Message queue resources
pub mod message_queue;

/// Observability resources
pub mod logger;
pub mod metrics;
pub mod observability;
pub mod tracer;

/// Storage resources
pub mod storage;
// Re-exports for convenience
pub use cache::CacheResource;
pub use database::DatabaseResource;
pub use http_client::HttpClientResource;
#[cfg(feature = "kafka")]
pub use kafka::{KafkaConsumerResource, KafkaProducerResource};
pub use memory_cache::MemoryCacheResource;
pub use message_queue::MessageQueueResource;
#[cfg(feature = "mongodb")]
pub use mongodb::MongoDbResource;
#[cfg(feature = "mysql")]
pub use mysql::MySqlResource;
pub use observability::{LoggerResource, MetricsResource, TracerResource};
#[cfg(feature = "postgres")]
pub use postgres::PostgresResource;
#[cfg(feature = "redis")]
pub use redis_cache::RedisCacheResource;
pub use storage::StorageResource;
