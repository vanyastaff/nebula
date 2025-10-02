//! Built-in resource implementations
/// Database resources
pub mod database;
#[cfg(feature = "postgres")]
pub mod postgres;
#[cfg(feature = "mysql")]
pub mod mysql;
#[cfg(feature = "mongodb")]
pub mod mongodb;
/// Cache resources
pub mod cache;
/// In-memory cache with LRU eviction
pub mod memory_cache;
/// Redis cache resource
#[cfg(feature = "redis")]
pub mod redis_cache;
/// HTTP client resources
pub mod http_client;
/// Message queue resources
pub mod message_queue;
/// Kafka message queue
#[cfg(feature = "kafka")]
pub mod kafka;
/// Storage resources
pub mod storage;
/// Observability resources
pub mod observability;
// Re-exports for convenience
pub use database::DatabaseResource;
#[cfg(feature = "postgres")]
pub use postgres::PostgresResource;
#[cfg(feature = "mysql")]
pub use mysql::MySqlResource;
#[cfg(feature = "mongodb")]
pub use mongodb::MongoDbResource;
pub use cache::CacheResource;
pub use memory_cache::MemoryCacheResource;
#[cfg(feature = "redis")]
pub use redis_cache::RedisCacheResource;
pub use http_client::HttpClientResource;
pub use message_queue::MessageQueueResource;
#[cfg(feature = "kafka")]
pub use kafka::{KafkaProducerResource, KafkaConsumerResource};
pub use storage::StorageResource;
pub use observability::{LoggerResource, MetricsResource, TracerResource};

