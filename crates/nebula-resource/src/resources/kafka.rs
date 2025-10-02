//! Kafka message queue resource implementation
//!
//! **Note on Windows Support:**
//! The `rdkafka` crate requires the native `librdkafka` C library. On Windows, this can be challenging
//! to build. Options for Windows users:
//! - Use WSL (Windows Subsystem for Linux)
//! - Pre-install librdkafka using vcpkg: `vcpkg install librdkafka`
//! - Use Docker containers for Kafka-dependent services
//! - Build on Linux/macOS for production deployments
//!
//! For CI/CD on Windows, it's recommended to skip kafka feature tests or use WSL-based runners.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use uuid::Uuid;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::core::{
    context::ResourceContext,
    error::{ResourceError, ResourceResult},
    lifecycle::LifecycleState,
    resource::{Resource, ResourceConfig, ResourceId, ResourceInstance, ResourceMetadata},
    scoping::ResourceScope,
    traits::{HealthCheckable, HealthStatus, PoolConfig, Poolable},
};

#[cfg(feature = "kafka")]
use rdkafka::{
    Message,
    config::ClientConfig,
    consumer::{Consumer, StreamConsumer},
    producer::{FutureProducer, FutureRecord},
};

/// Kafka producer configuration
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct KafkaProducerConfig {
    /// Kafka broker addresses (comma-separated)
    pub brokers: String,
    /// Client ID for identification
    pub client_id: String,
    /// Compression codec
    pub compression_type: CompressionType,
    /// Number of acknowledgments the producer requires
    pub acks: AckPolicy,
    /// Maximum time to wait for acknowledgment
    pub request_timeout: Duration,
    /// Batch size for batching messages
    pub batch_size: usize,
    /// Time to wait for batching
    pub linger_ms: Duration,
    /// Enable idempotence (exactly-once semantics)
    pub enable_idempotence: bool,
    /// Maximum in-flight requests per connection
    pub max_in_flight_requests: usize,
    /// Additional configuration
    pub extra_config: HashMap<String, String>,
}

impl Default for KafkaProducerConfig {
    fn default() -> Self {
        Self {
            brokers: "localhost:9092".to_string(),
            client_id: "nebula-producer".to_string(),
            compression_type: CompressionType::Snappy,
            acks: AckPolicy::All,
            request_timeout: Duration::from_secs(30),
            batch_size: 16384,
            linger_ms: Duration::from_millis(10),
            enable_idempotence: true,
            max_in_flight_requests: 5,
            extra_config: HashMap::new(),
        }
    }
}

impl ResourceConfig for KafkaProducerConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.brokers.is_empty() {
            return Err(ResourceError::configuration("Brokers cannot be empty"));
        }
        if self.client_id.is_empty() {
            return Err(ResourceError::configuration("Client ID cannot be empty"));
        }
        if self.batch_size == 0 {
            return Err(ResourceError::configuration("Batch size cannot be zero"));
        }
        if self.max_in_flight_requests == 0 {
            return Err(ResourceError::configuration(
                "Max in-flight requests cannot be zero",
            ));
        }
        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.brokers.is_empty() {
            self.brokers = other.brokers;
        }
        if !other.client_id.is_empty() {
            self.client_id = other.client_id;
        }
        self.compression_type = other.compression_type;
        self.acks = other.acks;
        self.extra_config.extend(other.extra_config);
    }
}

/// Kafka consumer configuration
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct KafkaConsumerConfig {
    /// Kafka broker addresses (comma-separated)
    pub brokers: String,
    /// Consumer group ID
    pub group_id: String,
    /// Client ID for identification
    pub client_id: String,
    /// Topics to subscribe to
    pub topics: Vec<String>,
    /// Auto-offset reset strategy
    pub auto_offset_reset: OffsetResetStrategy,
    /// Enable auto-commit
    pub enable_auto_commit: bool,
    /// Auto-commit interval
    pub auto_commit_interval: Duration,
    /// Session timeout
    pub session_timeout: Duration,
    /// Maximum poll interval
    pub max_poll_interval: Duration,
    /// Additional configuration
    pub extra_config: HashMap<String, String>,
}

impl Default for KafkaConsumerConfig {
    fn default() -> Self {
        Self {
            brokers: "localhost:9092".to_string(),
            group_id: "nebula-consumer-group".to_string(),
            client_id: "nebula-consumer".to_string(),
            topics: Vec::new(),
            auto_offset_reset: OffsetResetStrategy::Earliest,
            enable_auto_commit: false, // Manual commit for reliability
            auto_commit_interval: Duration::from_secs(5),
            session_timeout: Duration::from_secs(10),
            max_poll_interval: Duration::from_secs(300),
            extra_config: HashMap::new(),
        }
    }
}

impl ResourceConfig for KafkaConsumerConfig {
    fn validate(&self) -> ResourceResult<()> {
        if self.brokers.is_empty() {
            return Err(ResourceError::configuration("Brokers cannot be empty"));
        }
        if self.group_id.is_empty() {
            return Err(ResourceError::configuration("Group ID cannot be empty"));
        }
        if self.client_id.is_empty() {
            return Err(ResourceError::configuration("Client ID cannot be empty"));
        }
        if self.topics.is_empty() {
            return Err(ResourceError::configuration(
                "At least one topic must be specified",
            ));
        }
        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if !other.brokers.is_empty() {
            self.brokers = other.brokers;
        }
        if !other.group_id.is_empty() {
            self.group_id = other.group_id;
        }
        if !other.topics.is_empty() {
            self.topics = other.topics;
        }
        self.extra_config.extend(other.extra_config);
    }
}

/// Compression type for Kafka messages
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum CompressionType {
    /// No compression
    None,
    /// Gzip compression
    Gzip,
    /// Snappy compression
    Snappy,
    /// LZ4 compression
    Lz4,
    /// Zstd compression
    Zstd,
}

impl CompressionType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gzip => "gzip",
            Self::Snappy => "snappy",
            Self::Lz4 => "lz4",
            Self::Zstd => "zstd",
        }
    }
}

/// Acknowledgment policy for producers
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum AckPolicy {
    /// No acknowledgment required
    None,
    /// Leader acknowledgment only
    Leader,
    /// All in-sync replicas must acknowledge
    All,
}

impl AckPolicy {
    fn as_str(&self) -> &'static str {
        match self {
            Self::None => "0",
            Self::Leader => "1",
            Self::All => "all",
        }
    }
}

/// Offset reset strategy for consumers
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum OffsetResetStrategy {
    /// Start from earliest available offset
    Earliest,
    /// Start from latest offset
    Latest,
    /// Fail if no offset is available
    None,
}

impl OffsetResetStrategy {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Earliest => "earliest",
            Self::Latest => "latest",
            Self::None => "none",
        }
    }
}

/// Kafka producer resource instance
pub struct KafkaProducerInstance {
    instance_id: Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<LifecycleState>,

    #[cfg(feature = "kafka")]
    producer: Arc<FutureProducer>,

    config: KafkaProducerConfig,
}

impl KafkaProducerInstance {
    /// Create a new Kafka producer instance
    #[cfg(feature = "kafka")]
    pub fn new(
        resource_id: ResourceId,
        context: ResourceContext,
        config: KafkaProducerConfig,
    ) -> ResourceResult<Self> {
        // Build Kafka client configuration
        let mut client_config = ClientConfig::new();
        client_config
            .set("bootstrap.servers", &config.brokers)
            .set("client.id", &config.client_id)
            .set("compression.type", config.compression_type.as_str())
            .set("acks", config.acks.as_str())
            .set(
                "request.timeout.ms",
                config.request_timeout.as_millis().to_string(),
            )
            .set("batch.size", config.batch_size.to_string())
            .set("linger.ms", config.linger_ms.as_millis().to_string())
            .set(
                "max.in.flight.requests.per.connection",
                config.max_in_flight_requests.to_string(),
            )
            .set("enable.idempotence", config.enable_idempotence.to_string());

        // Add extra configuration
        for (key, value) in &config.extra_config {
            client_config.set(key, value);
        }

        // Create producer
        let producer: FutureProducer = client_config.create().map_err(|e| {
            ResourceError::initialization(
                "kafka_producer:1.0",
                format!("Failed to create Kafka producer: {}", e),
            )
        })?;

        Ok(Self {
            instance_id: Uuid::new_v4(),
            resource_id,
            context,
            created_at: chrono::Utc::now(),
            last_accessed: parking_lot::Mutex::new(None),
            state: parking_lot::RwLock::new(LifecycleState::Ready),
            producer: Arc::new(producer),
            config,
        })
    }

    /// Create a new Kafka producer instance (non-kafka fallback)
    #[cfg(not(feature = "kafka"))]
    pub fn new(
        resource_id: ResourceId,
        context: ResourceContext,
        config: KafkaProducerConfig,
    ) -> ResourceResult<Self> {
        Err(ResourceError::configuration(
            "Kafka feature not enabled. Enable 'kafka' feature to use Kafka producer",
        ))
    }

    /// Send a message to a topic
    #[cfg(feature = "kafka")]
    pub async fn send(
        &self,
        topic: &str,
        key: Option<&[u8]>,
        payload: &[u8],
    ) -> ResourceResult<(i32, i64)> {
        self.touch();

        let mut record = FutureRecord::to(topic).payload(payload);

        if let Some(key) = key {
            record = record.key(key);
        }

        let result = self
            .producer
            .send(record, Duration::from_secs(0))
            .await
            .map_err(|(e, _)| {
                ResourceError::internal(
                    "kafka_producer:1.0",
                    format!("Failed to send message: {}", e),
                )
            })?;

        Ok(result)
    }

    /// Send a message to a topic (non-kafka fallback)
    #[cfg(not(feature = "kafka"))]
    pub async fn send(
        &self,
        _topic: &str,
        _key: Option<&[u8]>,
        _payload: &[u8],
    ) -> ResourceResult<(i32, i64)> {
        Err(ResourceError::configuration("Kafka feature not enabled"))
    }

    /// Send a JSON message
    #[cfg(all(feature = "kafka", feature = "serde"))]
    pub async fn send_json<T: serde::Serialize>(
        &self,
        topic: &str,
        key: Option<&str>,
        value: &T,
    ) -> ResourceResult<(i32, i64)> {
        let payload = serde_json::to_vec(value).map_err(|e| {
            ResourceError::internal(
                "kafka_producer:1.0",
                format!("Failed to serialize JSON: {}", e),
            )
        })?;

        let key_bytes = key.map(|k| k.as_bytes());
        self.send(topic, key_bytes, &payload).await
    }

    /// Flush pending messages
    #[cfg(feature = "kafka")]
    pub async fn flush(&self, timeout: Duration) -> ResourceResult<()> {
        self.producer.flush(timeout).map_err(|e| {
            ResourceError::internal("kafka_producer:1.0", format!("Failed to flush: {}", e))
        })
    }
}

impl ResourceInstance for KafkaProducerInstance {
    fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    fn resource_id(&self) -> &ResourceId {
        &self.resource_id
    }

    fn lifecycle_state(&self) -> LifecycleState {
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

#[async_trait]
impl HealthCheckable for KafkaProducerInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        #[cfg(feature = "kafka")]
        {
            // Check if producer is available (rdkafka doesn't have direct health check)
            // We can try to get metadata as a health check
            let start = std::time::Instant::now();

            // Producers are stateless in rdkafka, assume healthy if created successfully
            let latency = start.elapsed();
            Ok(HealthStatus::healthy().with_latency(latency))
        }

        #[cfg(not(feature = "kafka"))]
        {
            Ok(HealthStatus::unhealthy("Kafka feature not enabled"))
        }
    }

    fn health_check_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

impl Poolable for KafkaProducerInstance {
    fn pool_config(&self) -> PoolConfig {
        PoolConfig {
            min_size: 1,
            max_size: 5,
            acquire_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(3600),
            validation_interval: Duration::from_secs(60),
        }
    }

    fn is_valid_for_pool(&self) -> bool {
        matches!(
            self.lifecycle_state(),
            LifecycleState::Ready | LifecycleState::Idle
        )
    }

    fn prepare_for_pool(&mut self) -> ResourceResult<()> {
        *self.state.write() = LifecycleState::Idle;
        Ok(())
    }

    fn prepare_for_acquisition(&mut self) -> ResourceResult<()> {
        *self.state.write() = LifecycleState::InUse;
        self.touch();
        Ok(())
    }
}

/// Kafka producer resource
pub struct KafkaProducerResource;

#[async_trait]
impl Resource for KafkaProducerResource {
    type Config = KafkaProducerConfig;
    type Instance = KafkaProducerInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("kafka_producer", "1.0"),
            "Kafka producer for async message publishing".to_string(),
        )
        .with_tag("type", "message_queue")
        .with_tag("backend", "kafka")
        .with_tag("role", "producer")
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
        KafkaProducerInstance::new(self.metadata().id, context.clone(), config.clone())
    }

    async fn cleanup(&self, instance: Self::Instance) -> ResourceResult<()> {
        #[cfg(feature = "kafka")]
        {
            // Flush any pending messages before cleanup
            let _ = instance.flush(Duration::from_secs(5)).await;
        }

        #[cfg(not(feature = "kafka"))]
        {
            let _ = instance;
        }

        Ok(())
    }

    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        Ok(matches!(
            instance.lifecycle_state(),
            LifecycleState::Ready | LifecycleState::Idle | LifecycleState::InUse
        ))
    }
}

/// Kafka consumer resource instance
pub struct KafkaConsumerInstance {
    instance_id: Uuid,
    resource_id: ResourceId,
    context: ResourceContext,
    created_at: chrono::DateTime<chrono::Utc>,
    last_accessed: parking_lot::Mutex<Option<chrono::DateTime<chrono::Utc>>>,
    state: parking_lot::RwLock<LifecycleState>,

    #[cfg(feature = "kafka")]
    consumer: Arc<StreamConsumer>,

    config: KafkaConsumerConfig,
}

impl KafkaConsumerInstance {
    /// Create a new Kafka consumer instance
    #[cfg(feature = "kafka")]
    pub fn new(
        resource_id: ResourceId,
        context: ResourceContext,
        config: KafkaConsumerConfig,
    ) -> ResourceResult<Self> {
        use rdkafka::consumer::Consumer;

        // Build Kafka client configuration
        let mut client_config = ClientConfig::new();
        client_config
            .set("bootstrap.servers", &config.brokers)
            .set("group.id", &config.group_id)
            .set("client.id", &config.client_id)
            .set("auto.offset.reset", config.auto_offset_reset.as_str())
            .set("enable.auto.commit", config.enable_auto_commit.to_string())
            .set(
                "auto.commit.interval.ms",
                config.auto_commit_interval.as_millis().to_string(),
            )
            .set(
                "session.timeout.ms",
                config.session_timeout.as_millis().to_string(),
            )
            .set(
                "max.poll.interval.ms",
                config.max_poll_interval.as_millis().to_string(),
            );

        // Add extra configuration
        for (key, value) in &config.extra_config {
            client_config.set(key, value);
        }

        // Create consumer
        let consumer: StreamConsumer = client_config.create().map_err(|e| {
            ResourceError::initialization(
                "kafka_consumer:1.0",
                format!("Failed to create Kafka consumer: {}", e),
            )
        })?;

        // Subscribe to topics
        let topic_refs: Vec<&str> = config.topics.iter().map(|s| s.as_str()).collect();
        consumer.subscribe(&topic_refs).map_err(|e| {
            ResourceError::initialization(
                "kafka_consumer:1.0",
                format!("Failed to subscribe to topics: {}", e),
            )
        })?;

        Ok(Self {
            instance_id: Uuid::new_v4(),
            resource_id,
            context,
            created_at: chrono::Utc::now(),
            last_accessed: parking_lot::Mutex::new(None),
            state: parking_lot::RwLock::new(LifecycleState::Ready),
            consumer: Arc::new(consumer),
            config,
        })
    }

    /// Create a new Kafka consumer instance (non-kafka fallback)
    #[cfg(not(feature = "kafka"))]
    pub fn new(
        _resource_id: ResourceId,
        _context: ResourceContext,
        _config: KafkaConsumerConfig,
    ) -> ResourceResult<Self> {
        Err(ResourceError::configuration(
            "Kafka feature not enabled. Enable 'kafka' feature to use Kafka consumer",
        ))
    }

    /// Get the underlying consumer for advanced operations
    #[cfg(feature = "kafka")]
    pub fn consumer(&self) -> &StreamConsumer {
        &self.consumer
    }

    /// Receive a message with timeout
    #[cfg(feature = "kafka")]
    pub async fn receive(&self, timeout: Duration) -> ResourceResult<KafkaMessage> {
        use futures::StreamExt;

        self.touch();

        let stream = self.consumer.stream();
        tokio::pin!(stream);

        match tokio::time::timeout(timeout, stream.next()).await {
            Ok(Some(Ok(msg))) => {
                let partition = msg.partition();
                let offset = msg.offset();
                let key = msg.key().map(|k| k.to_vec());
                let payload = msg.payload().map(|p| p.to_vec());
                let topic = msg.topic().to_string();

                Ok(KafkaMessage {
                    topic,
                    partition,
                    offset,
                    key,
                    payload,
                })
            }
            Ok(Some(Err(e))) => Err(ResourceError::internal(
                "kafka_consumer:1.0",
                format!("Failed to receive message: {}", e),
            )),
            Ok(None) => Err(ResourceError::internal(
                "kafka_consumer:1.0",
                "Stream ended unexpectedly",
            )),
            Err(_) => Err(ResourceError::Timeout {
                resource_id: "kafka_consumer:1.0".to_string(),
                timeout_ms: timeout.as_millis() as u64,
                operation: "receive_message".to_string(),
            }),
        }
    }

    /// Commit offsets manually
    #[cfg(feature = "kafka")]
    pub async fn commit(&self) -> ResourceResult<()> {
        self.consumer
            .commit_consumer_state(rdkafka::consumer::CommitMode::Async)
            .map_err(|e| {
                ResourceError::internal(
                    "kafka_consumer:1.0",
                    format!("Failed to commit offsets: {}", e),
                )
            })
    }
}

impl ResourceInstance for KafkaConsumerInstance {
    fn instance_id(&self) -> Uuid {
        self.instance_id
    }

    fn resource_id(&self) -> &ResourceId {
        &self.resource_id
    }

    fn lifecycle_state(&self) -> LifecycleState {
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

#[async_trait]
impl HealthCheckable for KafkaConsumerInstance {
    async fn health_check(&self) -> ResourceResult<HealthStatus> {
        #[cfg(feature = "kafka")]
        {
            // Check consumer subscription
            let start = std::time::Instant::now();
            let subscription = self.consumer.subscription();

            if subscription.is_ok() && !subscription.unwrap().elements().is_empty() {
                let latency = start.elapsed();
                Ok(HealthStatus::healthy().with_latency(latency))
            } else {
                let latency = start.elapsed();
                Ok(HealthStatus::unhealthy("No active subscriptions").with_latency(latency))
            }
        }

        #[cfg(not(feature = "kafka"))]
        {
            Ok(HealthStatus::unhealthy("Kafka feature not enabled"))
        }
    }

    fn health_check_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Kafka consumer resource
pub struct KafkaConsumerResource;

#[async_trait]
impl Resource for KafkaConsumerResource {
    type Config = KafkaConsumerConfig;
    type Instance = KafkaConsumerInstance;

    fn metadata(&self) -> ResourceMetadata {
        ResourceMetadata::new(
            ResourceId::new("kafka_consumer", "1.0"),
            "Kafka consumer for async message consumption".to_string(),
        )
        .with_tag("type", "message_queue")
        .with_tag("backend", "kafka")
        .with_tag("role", "consumer")
        .health_checkable()
        .with_default_scope(ResourceScope::Workflow)
    }

    async fn create(
        &self,
        config: &Self::Config,
        context: &ResourceContext,
    ) -> ResourceResult<Self::Instance> {
        config.validate()?;
        KafkaConsumerInstance::new(self.metadata().id, context.clone(), config.clone())
    }

    async fn cleanup(&self, _instance: Self::Instance) -> ResourceResult<()> {
        // Consumer cleanup happens automatically on drop
        Ok(())
    }

    async fn validate_instance(&self, instance: &Self::Instance) -> ResourceResult<bool> {
        Ok(matches!(
            instance.lifecycle_state(),
            LifecycleState::Ready | LifecycleState::Idle | LifecycleState::InUse
        ))
    }
}

/// Kafka message representation
#[derive(Debug, Clone)]
pub struct KafkaMessage {
    /// Topic name
    pub topic: String,
    /// Partition number
    pub partition: i32,
    /// Message offset
    pub offset: i64,
    /// Optional message key
    pub key: Option<Vec<u8>>,
    /// Optional message payload
    pub payload: Option<Vec<u8>>,
}

impl KafkaMessage {
    /// Get payload as UTF-8 string
    pub fn payload_as_str(&self) -> ResourceResult<Option<&str>> {
        self.payload
            .as_ref()
            .map(|p| {
                std::str::from_utf8(p).map_err(|e| {
                    ResourceError::internal(
                        "kafka_message",
                        format!("Failed to parse payload as UTF-8: {}", e),
                    )
                })
            })
            .transpose()
    }

    /// Get payload as JSON
    #[cfg(feature = "serde")]
    pub fn payload_as_json<T>(&self) -> ResourceResult<Option<T>>
    where
        T: serde::de::DeserializeOwned,
    {
        self.payload
            .as_ref()
            .map(|p| {
                serde_json::from_slice(p).map_err(|e| {
                    ResourceError::internal(
                        "kafka_message",
                        format!("Failed to parse payload as JSON: {}", e),
                    )
                })
            })
            .transpose()
    }

    /// Get key as UTF-8 string
    pub fn key_as_str(&self) -> ResourceResult<Option<&str>> {
        self.key
            .as_ref()
            .map(|k| {
                std::str::from_utf8(k).map_err(|e| {
                    ResourceError::internal(
                        "kafka_message",
                        format!("Failed to parse key as UTF-8: {}", e),
                    )
                })
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kafka_producer_config_validation() {
        let mut config = KafkaProducerConfig::default();
        assert!(config.validate().is_ok());

        config.brokers = String::new();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_kafka_producer_config_merge() {
        let mut config1 = KafkaProducerConfig::default();
        let mut config2 = KafkaProducerConfig::default();

        config2.brokers = "kafka1:9092,kafka2:9092".to_string();
        config2.client_id = "custom-producer".to_string();
        config2.compression_type = CompressionType::Gzip;

        config1.merge(config2);

        assert_eq!(config1.brokers, "kafka1:9092,kafka2:9092");
        assert_eq!(config1.client_id, "custom-producer");
    }

    #[test]
    fn test_kafka_consumer_config_validation() {
        let mut config = KafkaConsumerConfig::default();
        config.topics.push("test-topic".to_string());
        assert!(config.validate().is_ok());

        config.brokers = String::new();
        assert!(config.validate().is_err());

        let mut config = KafkaConsumerConfig::default();
        config.topics.clear();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_compression_types() {
        assert_eq!(CompressionType::None.as_str(), "none");
        assert_eq!(CompressionType::Gzip.as_str(), "gzip");
        assert_eq!(CompressionType::Snappy.as_str(), "snappy");
        assert_eq!(CompressionType::Lz4.as_str(), "lz4");
        assert_eq!(CompressionType::Zstd.as_str(), "zstd");
    }

    #[test]
    fn test_ack_policies() {
        assert_eq!(AckPolicy::None.as_str(), "0");
        assert_eq!(AckPolicy::Leader.as_str(), "1");
        assert_eq!(AckPolicy::All.as_str(), "all");
    }

    #[test]
    fn test_offset_reset_strategies() {
        assert_eq!(OffsetResetStrategy::Earliest.as_str(), "earliest");
        assert_eq!(OffsetResetStrategy::Latest.as_str(), "latest");
        assert_eq!(OffsetResetStrategy::None.as_str(), "none");
    }

    #[test]
    fn test_kafka_message_parsing() {
        let msg = KafkaMessage {
            topic: "test".to_string(),
            partition: 0,
            offset: 42,
            key: Some(b"key".to_vec()),
            payload: Some(b"payload".to_vec()),
        };

        assert_eq!(msg.payload_as_str().unwrap(), Some("payload"));
        assert_eq!(msg.key_as_str().unwrap(), Some("key"));
    }

    #[tokio::test]
    async fn test_kafka_producer_resource() {
        let resource = KafkaProducerResource;
        let metadata = resource.metadata();

        assert_eq!(metadata.id.name, "kafka_producer");
        assert!(metadata.poolable);
        assert!(metadata.health_checkable);
    }

    #[tokio::test]
    async fn test_kafka_consumer_resource() {
        let resource = KafkaConsumerResource;
        let metadata = resource.metadata();

        assert_eq!(metadata.id.name, "kafka_consumer");
        assert!(metadata.health_checkable);
    }
}
