---
title: System Events
tags: [nebula, nebula-resource, docs]
status: draft
created: 2025-08-17
---

# Events Reference

## Overview

The event system in nebula-resource provides reactive, asynchronous event handling for resource lifecycle, state changes, and custom events. It supports multiple event buses, filtering, replay, and distributed event streaming.

## Core Event Types

### `ResourceEvent`

Base event type for all resource-related events.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceEvent {
    /// Unique event ID
    pub id: EventId,
    
    /// Event type
    pub event_type: ResourceEventType,
    
    /// Resource that triggered the event
    pub resource_id: ResourceId,
    
    /// Event timestamp
    pub timestamp: DateTime<Utc>,
    
    /// Event source
    pub source: EventSource,
    
    /// Event metadata
    pub metadata: EventMetadata,
    
    /// Event payload
    pub payload: Option<Value>,
    
    /// Correlation ID for tracing
    pub correlation_id: Option<String>,
    
    /// Causation ID (parent event)
    pub causation_id: Option<EventId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResourceEventType {
    // Lifecycle events
    Created,
    Initialized,
    Ready,
    Acquired { by: String },
    Released,
    Refreshed,
    Drained,
    Terminated,
    Failed { error: String },
    
    // State change events
    StateChanged {
        from: String,
        to: String,
    },
    
    // Health events
    HealthCheckPassed,
    HealthCheckFailed { reason: String },
    HealthStatusChanged {
        from: HealthStatus,
        to: HealthStatus,
    },
    
    // Pool events
    AddedToPool,
    RemovedFromPool,
    PoolScaled { from: usize, to: usize },
    
    // Configuration events
    ConfigurationUpdated {
        fields: Vec<String>,
    },
    
    // Custom events
    Custom {
        name: String,
        data: Value,
    },
}
```

### `EventMetadata`

Metadata associated with events.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMetadata {
    /// Event version
    pub version: String,
    
    /// Event schema
    pub schema: Option<String>,
    
    /// Event tags
    pub tags: HashSet<String>,
    
    /// Event labels
    pub labels: HashMap<String, String>,
    
    /// Event priority
    pub priority: EventPriority,
    
    /// Time-to-live for event
    pub ttl: Option<Duration>,
    
    /// Whether event should be persisted
    pub persistent: bool,
    
    /// Retry policy for handlers
    pub retry_policy: Option<RetryPolicy>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EventPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}
```

## Event Bus

### `EventBus`

Central event distribution system.

```rust
pub struct EventBus {
    /// Event handlers
    handlers: Arc<RwLock<HashMap<String, Vec<EventHandler>>>>,
    
    /// Event filters
    filters: Arc<RwLock<Vec<Box<dyn EventFilter>>>>,
    
    /// Event store for persistence
    store: Option<Arc<dyn EventStore>>,
    
    /// Event processors
    processors: Arc<RwLock<Vec<Box<dyn EventProcessor>>>>,
    
    /// Metrics collector
    metrics: Arc<EventMetrics>,
    
    /// Configuration
    config: EventBusConfig,
}

impl EventBus {
    /// Publish an event
    pub async fn publish(&self, event: ResourceEvent) -> Result<()> {
        // Apply filters
        if !self.should_process(&event).await? {
            return Ok(());
        }
        
        // Pre-process event
        let event = self.preprocess(event).await?;
        
        // Store if persistent
        if event.metadata.persistent {
            if let Some(store) = &self.store {
                store.append(event.clone()).await?;
            }
        }
        
        // Distribute to handlers
        self.distribute(event).await?;
        
        // Update metrics
        self.metrics.record_event_published();
        
        Ok(())
    }
    
    /// Subscribe to events
    pub async fn subscribe<F>(&self, pattern: EventPattern, handler: F) -> Subscription
    where
        F: Fn(ResourceEvent) -> BoxFuture<'static, Result<()>> + Send + Sync + 'static,
    {
        let handler_id = Uuid::new_v4().to_string();
        let handler = Arc::new(handler);
        
        self.handlers.write().await
            .entry(pattern.to_string())
            .or_insert_with(Vec::new)
            .push(EventHandler {
                id: handler_id.clone(),
                handler,
                pattern: pattern.clone(),
            });
        
        Subscription {
            id: handler_id,
            pattern,
            bus: Arc::downgrade(&Arc::new(self.clone())),
        }
    }
    
    /// Query historical events
    pub async fn query(&self, query: EventQuery) -> Result<Vec<ResourceEvent>> {
        if let Some(store) = &self.store {
            store.query(query).await
        } else {
            Ok(Vec::new())
        }
    }
}
```

### `EventPattern`

Pattern matching for event subscriptions.

```rust
#[derive(Debug, Clone)]
pub enum EventPattern {
    /// Match exact event type
    Exact(ResourceEventType),
    
    /// Match by resource ID
    Resource(ResourceId),
    
    /// Match by resource type
    ResourceType(String),
    
    /// Match by event tags
    Tags(HashSet<String>),
    
    /// Match by custom predicate
    Custom(Arc<dyn Fn(&ResourceEvent) -> bool + Send + Sync>),
    
    /// Combine patterns with AND
    And(Vec<EventPattern>),
    
    /// Combine patterns with OR
    Or(Vec<EventPattern>),
    
    /// Negate pattern
    Not(Box<EventPattern>),
    
    /// Match all events
    All,
}

impl EventPattern {
    /// Check if event matches pattern
    pub fn matches(&self, event: &ResourceEvent) -> bool {
        match self {
            EventPattern::Exact(event_type) => &event.event_type == event_type,
            EventPattern::Resource(id) => &event.resource_id == id,
            EventPattern::ResourceType(rtype) => {
                event.metadata.labels.get("resource_type")
                    .map(|t| t == rtype)
                    .unwrap_or(false)
            }
            EventPattern::Tags(tags) => {
                tags.is_subset(&event.metadata.tags)
            }
            EventPattern::Custom(predicate) => predicate(event),
            EventPattern::And(patterns) => {
                patterns.iter().all(|p| p.matches(event))
            }
            EventPattern::Or(patterns) => {
                patterns.iter().any(|p| p.matches(event))
            }
            EventPattern::Not(pattern) => !pattern.matches(event),
            EventPattern::All => true,
        }
    }
}
```

## Event Handlers

### `EventHandler`

Event handler implementation.

```rust
pub struct EventHandler {
    pub id: String,
    pub pattern: EventPattern,
    pub handler: Arc<dyn Fn(ResourceEvent) -> BoxFuture<'static, Result<()>> + Send + Sync>,
}

/// Async event handler trait
#[async_trait]
pub trait AsyncEventHandler: Send + Sync {
    /// Handle event
    async fn handle(&self, event: ResourceEvent) -> Result<()>;
    
    /// Get handler name
    fn name(&self) -> &str;
    
    /// Check if handler can process event
    async fn can_handle(&self, event: &ResourceEvent) -> bool {
        true
    }
    
    /// Handle error
    async fn on_error(&self, event: &ResourceEvent, error: Error) {
        error!("Handler {} failed for event {}: {}", self.name(), event.id, error);
    }
}
```

### Built-in Event Handlers

```rust
/// Logger handler - logs all events
pub struct LoggerHandler {
    level: log::Level,
    format: LogFormat,
}

#[async_trait]
impl AsyncEventHandler for LoggerHandler {
    async fn handle(&self, event: ResourceEvent) -> Result<()> {
        match self.format {
            LogFormat::Json => {
                log::log!(self.level, "{}", serde_json::to_string(&event)?);
            }
            LogFormat::Pretty => {
                log::log!(self.level, "Event: {} - Type: {:?} - Resource: {}",
                    event.id, event.event_type, event.resource_id);
            }
        }
        Ok(())
    }
    
    fn name(&self) -> &str {
        "logger"
    }
}

/// Metrics handler - records event metrics
pub struct MetricsHandler {
    collector: Arc<MetricsCollector>,
}

#[async_trait]
impl AsyncEventHandler for MetricsHandler {
    async fn handle(&self, event: ResourceEvent) -> Result<()> {
        self.collector.record_event(
            &event.event_type.to_string(),
            &event.resource_id.to_string(),
            event.metadata.labels.clone(),
        );
        Ok(())
    }
    
    fn name(&self) -> &str {
        "metrics"
    }
}

/// Webhook handler - sends events to external endpoints
pub struct WebhookHandler {
    endpoint: String,
    client: Client,
    headers: HashMap<String, String>,
    timeout: Duration,
}

#[async_trait]
impl AsyncEventHandler for WebhookHandler {
    async fn handle(&self, event: ResourceEvent) -> Result<()> {
        let response = self.client
            .post(&self.endpoint)
            .headers(self.headers.clone())
            .json(&event)
            .timeout(self.timeout)
            .send()
            .await?;
        
        if !response.status().is_success() {
            return Err(Error::WebhookFailed {
                endpoint: self.endpoint.clone(),
                status: response.status().as_u16(),
            });
        }
        
        Ok(())
    }
    
    fn name(&self) -> &str {
        "webhook"
    }
}
```

## Event Processors

### `EventProcessor`

Process events before distribution.

```rust
#[async_trait]
pub trait EventProcessor: Send + Sync {
    /// Process event
    async fn process(&self, event: ResourceEvent) -> Result<ResourceEvent>;
    
    /// Processor name
    fn name(&self) -> &str;
    
    /// Processing order (lower = earlier)
    fn order(&self) -> i32 {
        0
    }
}

/// Enrichment processor - adds metadata
pub struct EnrichmentProcessor {
    enrichers: Vec<Box<dyn EventEnricher>>,
}

#[async_trait]
impl EventProcessor for EnrichmentProcessor {
    async fn process(&self, mut event: ResourceEvent) -> Result<ResourceEvent> {
        for enricher in &self.enrichers {
            event = enricher.enrich(event).await?;
        }
        Ok(event)
    }
    
    fn name(&self) -> &str {
        "enrichment"
    }
}

/// Validation processor - validates events
pub struct ValidationProcessor {
    validators: Vec<Box<dyn EventValidator>>,
}

#[async_trait]
impl EventProcessor for ValidationProcessor {
    async fn process(&self, event: ResourceEvent) -> Result<ResourceEvent> {
        for validator in &self.validators {
            validator.validate(&event)?;
        }
        Ok(event)
    }
    
    fn name(&self) -> &str {
        "validation"
    }
}

/// Transformation processor - transforms events
pub struct TransformationProcessor {
    transformers: Vec<Box<dyn EventTransformer>>,
}

#[async_trait]
impl EventProcessor for TransformationProcessor {
    async fn process(&self, mut event: ResourceEvent) -> Result<ResourceEvent> {
        for transformer in &self.transformers {
            if transformer.should_transform(&event) {
                event = transformer.transform(event).await?;
            }
        }
        Ok(event)
    }
    
    fn name(&self) -> &str {
        "transformation"
    }
}
```

## Event Store

### `EventStore`

Persistent event storage.

```rust
#[async_trait]
pub trait EventStore: Send + Sync {
    /// Append event to store
    async fn append(&self, event: ResourceEvent) -> Result<()>;
    
    /// Append multiple events
    async fn append_batch(&self, events: Vec<ResourceEvent>) -> Result<()>;
    
    /// Query events
    async fn query(&self, query: EventQuery) -> Result<Vec<ResourceEvent>>;
    
    /// Get event by ID
    async fn get(&self, id: &EventId) -> Result<Option<ResourceEvent>>;
    
    /// Delete old events
    async fn prune(&self, before: DateTime<Utc>) -> Result<usize>;
    
    /// Create snapshot
    async fn snapshot(&self) -> Result<EventSnapshot>;
    
    /// Restore from snapshot
    async fn restore(&self, snapshot: EventSnapshot) -> Result<()>;
}

/// In-memory event store
pub struct InMemoryEventStore {
    events: Arc<RwLock<Vec<ResourceEvent>>>,
    max_size: usize,
}

#[async_trait]
impl EventStore for InMemoryEventStore {
    async fn append(&self, event: ResourceEvent) -> Result<()> {
        let mut events = self.events.write().await;
        
        // Enforce size limit
        if events.len() >= self.max_size {
            events.remove(0);
        }
        
        events.push(event);
        Ok(())
    }
    
    async fn query(&self, query: EventQuery) -> Result<Vec<ResourceEvent>> {
        let events = self.events.read().await;
        
        Ok(events.iter()
            .filter(|e| query.matches(e))
            .cloned()
            .collect())
    }
    
    // Other implementations...
}

/// Database event store
pub struct DatabaseEventStore {
    pool: PgPool,
    table_name: String,
}

#[async_trait]
impl EventStore for DatabaseEventStore {
    async fn append(&self, event: ResourceEvent) -> Result<()> {
        sqlx::query(&format!(
            "INSERT INTO {} (id, event_type, resource_id, timestamp, metadata, payload)
             VALUES ($1, $2, $3, $4, $5, $6)",
            self.table_name
        ))
        .bind(&event.id)
        .bind(&event.event_type.to_string())
        .bind(&event.resource_id.to_string())
        .bind(&event.timestamp)
        .bind(&serde_json::to_value(&event.metadata)?)
        .bind(&event.payload)
        .execute(&self.pool)
        .await?;
        
        Ok(())
    }
    
    // Other implementations...
}
```

## Event Queries

### `EventQuery`

Query builder for events.

```rust
#[derive(Debug, Clone, Default)]
pub struct EventQuery {
    /// Filter by event types
    pub event_types: Option<Vec<ResourceEventType>>,
    
    /// Filter by resource IDs
    pub resource_ids: Option<Vec<ResourceId>>,
    
    /// Filter by time range
    pub time_range: Option<TimeRange>,
    
    /// Filter by tags
    pub tags: Option<HashSet<String>>,
    
    /// Filter by labels
    pub labels: Option<HashMap<String, String>>,
    
    /// Custom filter predicate
    pub filter: Option<Arc<dyn Fn(&ResourceEvent) -> bool + Send + Sync>>,
    
    /// Sort order
    pub order_by: OrderBy,
    
    /// Limit results
    pub limit: Option<usize>,
    
    /// Offset for pagination
    pub offset: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct TimeRange {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum OrderBy {
    Timestamp(SortOrder),
    EventType(SortOrder),
    ResourceId(SortOrder),
}

#[derive(Debug, Clone)]
pub enum SortOrder {
    Ascending,
    Descending,
}

impl EventQuery {
    /// Create new query builder
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Filter by event type
    pub fn with_type(mut self, event_type: ResourceEventType) -> Self {
        self.event_types.get_or_insert_with(Vec::new).push(event_type);
        self
    }
    
    /// Filter by resource
    pub fn with_resource(mut self, resource_id: ResourceId) -> Self {
        self.resource_ids.get_or_insert_with(Vec::new).push(resource_id);
        self
    }
    
    /// Filter by time range
    pub fn between(mut self, start: DateTime<Utc>, end: DateTime<Utc>) -> Self {
        self.time_range = Some(TimeRange { start, end });
        self
    }
    
    /// Add tag filter
    pub fn with_tag(mut self, tag: String) -> Self {
        self.tags.get_or_insert_with(HashSet::new).insert(tag);
        self
    }
    
    /// Set result limit
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
    
    /// Check if event matches query
    pub fn matches(&self, event: &ResourceEvent) -> bool {
        // Check event type
        if let Some(types) = &self.event_types {
            if !types.contains(&event.event_type) {
                return false;
            }
        }
        
        // Check resource ID
        if let Some(ids) = &self.resource_ids {
            if !ids.contains(&event.resource_id) {
                return false;
            }
        }
        
        // Check time range
        if let Some(range) = &self.time_range {
            if event.timestamp < range.start || event.timestamp > range.end {
                return false;
            }
        }
        
        // Check tags
        if let Some(tags) = &self.tags {
            if !tags.is_subset(&event.metadata.tags) {
                return false;
            }
        }
        
        // Check custom filter
        if let Some(filter) = &self.filter {
            if !filter(event) {
                return false;
            }
        }
        
        true
    }
}
```

## Event Streaming

### `EventStream`

Real-time event streaming.

```rust
pub struct EventStream {
    receiver: broadcast::Receiver<ResourceEvent>,
    filter: Option<EventPattern>,
    buffer_size: usize,
}

impl Stream for EventStream {
    type Item = ResourceEvent;
    
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match self.receiver.poll_recv(cx) {
                Poll::Ready(Ok(event)) => {
                    if let Some(filter) = &self.filter {
                        if !filter.matches(&event) {
                            continue;
                        }
                    }
                    return Poll::Ready(Some(event));
                }
                Poll::Ready(Err(_)) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl EventBus {
    /// Create event stream
    pub fn stream(&self, filter: Option<EventPattern>) -> EventStream {
        EventStream {
            receiver: self.broadcaster.subscribe(),
            filter,
            buffer_size: 1024,
        }
    }
    
    /// Stream with backpressure
    pub fn stream_with_backpressure(
        &self,
        filter: Option<EventPattern>,
        buffer_size: usize,
    ) -> impl Stream<Item = ResourceEvent> {
        self.stream(filter)
            .throttle(Duration::from_millis(10))
            .buffer_unordered(buffer_size)
    }
}
```

## Event Replay

### `EventReplayer`

Replay historical events.

```rust
pub struct EventReplayer {
    store: Arc<dyn EventStore>,
    bus: Arc<EventBus>,
    config: ReplayConfig,
}

#[derive(Debug, Clone)]
pub struct ReplayConfig {
    /// Speed multiplier (1.0 = real-time)
    pub speed: f64,
    
    /// Skip certain event types
    pub skip_types: HashSet<ResourceEventType>,
    
    /// Transform events during replay
    pub transformer: Option<Arc<dyn EventTransformer>>,
    
    /// Maximum events to replay
    pub max_events: Option<usize>,
}

impl EventReplayer {
    /// Replay events from time range
    pub async fn replay_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<ReplayResult> {
        let query = EventQuery::new().between(start, end);
        let events = self.store.query(query).await?;
        
        self.replay_events(events).await
    }
    
    /// Replay specific events
    pub async fn replay_events(&self, events: Vec<ResourceEvent>) -> Result<ReplayResult> {
        let mut replayed = 0;
        let mut skipped = 0;
        let mut errors = Vec::new();
        
        let start_time = Instant::now();
        let mut last_timestamp = events.first().map(|e| e.timestamp);
        
        for event in events {
            // Skip if configured
            if self.config.skip_types.contains(&event.event_type) {
                skipped += 1;
                continue;
            }
            
            // Apply timing
            if let Some(last) = last_timestamp {
                let time_diff = event.timestamp - last;
                let delay = Duration::from_secs_f64(
                    time_diff.num_milliseconds() as f64 / 1000.0 / self.config.speed
                );
                tokio::time::sleep(delay).await;
            }
            
            // Transform if configured
            let event = if let Some(transformer) = &self.config.transformer {
                transformer.transform(event).await?
            } else {
                event
            };
            
            // Replay event
            match self.bus.publish(event.clone()).await {
                Ok(_) => replayed += 1,
                Err(e) => errors.push((event.id, e)),
            }
            
            last_timestamp = Some(event.timestamp);
            
            // Check limit
            if let Some(max) = self.config.max_events {
                if replayed >= max {
                    break;
                }
            }
        }
        
        Ok(ReplayResult {
            replayed,
            skipped,
            errors,
            duration: start_time.elapsed(),
        })
    }
}
```

## Event Aggregation

### `EventAggregator`

Aggregate events into summaries.

```rust
pub struct EventAggregator {
    window: Duration,
    aggregators: Vec<Box<dyn Aggregator>>,
}

#[async_trait]
pub trait Aggregator: Send + Sync {
    /// Add event to aggregation
    async fn add(&mut self, event: &ResourceEvent);
    
    /// Get aggregated result
    async fn result(&self) -> AggregationResult;
    
    /// Reset aggregation
    async fn reset(&mut self);
}

/// Count aggregator
pub struct CountAggregator {
    counts: HashMap<String, usize>,
}

#[async_trait]
impl Aggregator for CountAggregator {
    async fn add(&mut self, event: &ResourceEvent) {
        *self.counts.entry(event.event_type.to_string()).or_insert(0) += 1;
    }
    
    async fn result(&self) -> AggregationResult {
        AggregationResult::Counts(self.counts.clone())
    }
    
    async fn reset(&mut self) {
        self.counts.clear();
    }
}

/// Rate aggregator
pub struct RateAggregator {
    window: Duration,
    events: VecDeque<(Instant, ResourceEventType)>,
}

impl RateAggregator {
    pub fn rate(&self) -> HashMap<String, f64> {
        let mut rates = HashMap::new();
        let now = Instant::now();
        
        // Remove old events
        let cutoff = now - self.window;
        
        for (timestamp, event_type) in &self.events {
            if *timestamp > cutoff {
                *rates.entry(event_type.to_string()).or_insert(0.0) += 1.0;
            }
        }
        
        // Convert to rate per second
        let window_secs = self.window.as_secs_f64();
        for rate in rates.values_mut() {
            *rate /= window_secs;
        }
        
        rates
    }
}
```

## Testing Events

### Event Testing Utilities

```rust
#[cfg(test)]
pub mod testing {
    use super::*;
    
    /// Mock event builder
    pub struct EventBuilder {
        event: ResourceEvent,
    }
    
    impl EventBuilder {
        pub fn new() -> Self {
            Self {
                event: ResourceEvent {
                    id: EventId::new(),
                    event_type: ResourceEventType::Created,
                    resource_id: ResourceId::new(),
                    timestamp: Utc::now(),
                    source: EventSource::System,
                    metadata: EventMetadata::default(),
                    payload: None,
                    correlation_id: None,
                    causation_id: None,
                },
            }
        }
        
        pub fn with_type(mut self, event_type: ResourceEventType) -> Self {
            self.event.event_type = event_type;
            self
        }
        
        pub fn with_resource(mut self, resource_id: ResourceId) -> Self {
            self.event.resource_id = resource_id;
            self
        }
        
        pub fn with_payload<T: Serialize>(mut self, payload: T) -> Self {
            self.event.payload = Some(serde_json::to_value(payload).unwrap());
            self
        }
        
        pub fn build(self) -> ResourceEvent {
            self.event
        }
    }
    
    /// Test event handler
    pub struct TestHandler {
        pub events: Arc<RwLock<Vec<ResourceEvent>>>,
    }
    
    #[async_trait]
    impl AsyncEventHandler for TestHandler {
        async fn handle(&self, event: ResourceEvent) -> Result<()> {
            self.events.write().await.push(event);
            Ok(())
        }
        
        fn name(&self) -> &str {
            "test_handler"
        }
    }
    
    /// Event assertion helpers
    pub struct EventAssert;
    
    impl EventAssert {
        pub fn has_type(event: &ResourceEvent, expected: ResourceEventType) {
            assert_eq!(event.event_type, expected);
        }
        
        pub fn has_resource(event: &ResourceEvent, expected: &ResourceId) {
            assert_eq!(&event.resource_id, expected);
        }
        
        pub fn has_tag(event: &ResourceEvent, tag: &str) {
            assert!(event.metadata.tags.contains(tag));
        }
        
        pub fn matches_pattern(event: &ResourceEvent, pattern: &EventPattern) {
            assert!(pattern.matches(event));
        }
    }
}
```

## Configuration

### Event System Configuration

```yaml
events:
  # Event bus configuration
  bus:
    buffer_size: 10000
    processing_threads: 4
    max_handlers_per_event: 100
    
  # Event store configuration
  store:
    type: database  # memory | database | file
    
    # Database store config
    database:
      connection_string: ${DATABASE_URL}
      table_name: resource_events
      max_events: 1000000
      retention_days: 30
      
    # File store config
    file:
      path: /var/log/nebula/events
      rotation: daily
      compression: gzip
      
  # Event processing
  processing:
    # Processors
    processors:
      - type: enrichment
        enabled: true
      - type: validation
        enabled: true
      - type: transformation
        enabled: false
        
    # Async processing
    async:
      enabled: true
      queue_size: 5000
      worker_threads: 2
      
  # Event replay
  replay:
    enabled: true
    max_speed: 10.0
    buffer_size: 1000
    
  # Event streaming
  streaming:
    enabled: true
    buffer_size: 1024
    backpressure_threshold: 0.8
```

## Best Practices

1. **Use structured events** - Include all relevant context
2. **Keep events immutable** - Never modify published events
3. **Use correlation IDs** - For tracing related events
4. **Handle events idempotently** - Events may be delivered multiple times
5. **Set appropriate TTLs** - Don't store events forever
6. **Use event sourcing carefully** - It's not always the right pattern
7. **Monitor event rates** - Watch for event storms
8. **Test event handlers** - Including error scenarios
9. **Document event schemas** - Make events discoverable
10. **Version event types** - Support schema evolution
