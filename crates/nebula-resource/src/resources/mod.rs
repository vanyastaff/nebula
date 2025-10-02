//! Built-in resource implementations
/// Database resources
pub mod http_client;
pub mod database;
/// Cache resources
pub mod cache;
/// In-memory cache with LRU eviction
pub mod memory_cache;
/// Redis cache resource
#[cfg(feature = "redis")]
pub mod redis_cache;
/// HTTP client resources
/// Message queue resources
pub mod message_queue;
/// Storage resources
pub mod storage;
/// Observability resources
pub mod observability;
// Re-exports for convenience
pub use database::DatabaseResource;
pub use cache::CacheResource;
pub use memory_cache::MemoryCacheResource;
#[cfg(feature = "redis")]
pub use redis_cache::RedisCacheResource;
pub use http_client::HttpClientResource;
pub use message_queue::MessageQueueResource;
pub use storage::StorageResource;
pub use observability::{LoggerResource, MetricsResource, TracerResource};

