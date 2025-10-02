//! Built-in resource implementations
/// HTTP client resources
pub mod redis_cache;
pub mod http_client;

/// Database resources
pub mod database;

/// Cache resources
pub mod cache;

/// Message queue resources
pub mod message_queue;

/// Storage resources
pub mod storage;

/// Observability resources
pub mod observability;

// Re-exports for convenience
pub use http_client::HttpClientResource;
pub use database::DatabaseResource;
pub use cache::CacheResource;
pub use message_queue::MessageQueueResource;
pub use storage::StorageResource;
pub use observability::{LoggerResource, MetricsResource, TracerResource};
pub use redis_cache::RedisCacheResource;
