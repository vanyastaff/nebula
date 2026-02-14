---
title: "How to: TriggerAction"
tags: [nebula, nebula-action, crate, docs]
status: draft
created: 2025-08-17
---


## Overview

TriggerActions are event sources that initiate workflows. They monitor external sources and emit events that trigger workflow executions. This guide covers creating and managing trigger actions.

## Quick Start

### Basic Implementation

```rust
use nebula_action::prelude::*;
use serde::{Deserialize, Serialize};
use futures::stream::{self, Stream};

pub struct WebhookTrigger {
    metadata: ActionMetadata,
    server: Option<HttpServer>,
}

// Configuration for the trigger
#[derive(Deserialize)]
pub struct WebhookConfig {
    pub port: u16,
    pub path: String,
    pub auth_token: Option<String>,
    pub max_body_size: Option<usize>,
}

// Event emitted by the trigger
#[derive(Serialize, Clone)]
pub struct WebhookEvent {
    pub id: String,
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Value>,
    pub timestamp: DateTime<Utc>,
}

impl WebhookTrigger {
    pub fn new() -> Result<Self, ActionError> {
        let metadata = ActionMetadata::builder()
            .key("webhook.trigger")
            .name("Webhook Trigger")
            .description("Receives HTTP webhook events")
            .version("1.0.0")
            .build()?;
        
        Ok(Self {
            metadata,
            server: None,
        })
    }
}

// Required trait implementations
impl HasMetadata for WebhookTrigger {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl HasType for WebhookTrigger {
    fn r#type(&self) -> ActionType {
        ActionType::Trigger
    }
}

impl Action for WebhookTrigger {}

// TriggerAction implementation
#[async_trait]
impl TriggerAction for WebhookTrigger {
    type Config = WebhookConfig;
    type Event = WebhookEvent;
    
    async fn start(
        &mut self,
        config: Self::Config,
        context: &TriggerContext,
    ) -> Result<TriggerEventStream<Self::Event>, ActionError> {
        // Create HTTP server
        let (tx, rx) = mpsc::channel(100);
        
        let server = HttpServer::new(move || {
            let tx = tx.clone();
            App::new()
                .app_data(web::Data::new(tx))
                .route(&config.path, web::post().to(handle_webhook))
        })
        .bind(("0.0.0.0", config.port))
        .map_err(|e| ActionError::TriggerError(e.to_string()))?
        .run();
        
        // Store server handle
        self.server = Some(server.clone());
        
        // Start server in background
        tokio::spawn(server);
        
        context.log_info(&format!(
            "Webhook trigger started on port {} at path {}",
            config.port, config.path
        ));
        
        // Convert channel to stream
        let stream = ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }
    
    async fn stop(&mut self) -> Result<(), ActionError> {
        if let Some(server) = self.server.take() {
            server.stop(true).await;
        }
        Ok(())
    }
}

async fn handle_webhook(
    req: HttpRequest,
    body: web::Bytes,
    tx: web::Data<mpsc::Sender<WebhookEvent>>,
) -> impl Responder {
    let event = WebhookEvent {
        id: Uuid::new_v4().to_string(),
        method: req.method().to_string(),
        path: req.path().to_string(),
        headers: extract_headers(&req),
        body: parse_body(body),
        timestamp: Utc::now(),
    };
    
    if tx.send(event).await.is_ok() {
        HttpResponse::Ok().json(json!({ "status": "received" }))
    } else {
        HttpResponse::InternalServerError().json(json!({ "error": "Failed to process" }))
    }
}
```

## Common Patterns

### Polling Trigger

```rust
pub struct PollingTrigger {
    metadata: ActionMetadata,
    shutdown: Arc<AtomicBool>,
}

#[derive(Deserialize)]
pub struct PollingConfig {
    pub endpoint: String,
    pub interval_seconds: u64,
    pub auth: Option<AuthConfig>,
}

#[derive(Serialize, Clone)]
pub struct PollingEvent {
    pub data: Value,
    pub changed: bool,
    pub timestamp: DateTime<Utc>,
}

#[async_trait]
impl TriggerAction for PollingTrigger {
    type Config = PollingConfig;
    type Event = PollingEvent;
    
    async fn start(
        &mut self,
        config: Self::Config,
        context: &TriggerContext,
    ) -> Result<TriggerEventStream<Self::Event>, ActionError> {
        let shutdown = self.shutdown.clone();
        let interval = Duration::from_secs(config.interval_seconds);
        
        let stream = stream::unfold(
            (config, None, shutdown),
            |(config, last_data, shutdown)| async move {
                // Check shutdown
                if shutdown.load(Ordering::Relaxed) {
                    return None;
                }
                
                // Wait for interval
                tokio::time::sleep(interval).await;
                
                // Fetch data
                let client = reqwest::Client::new();
                let response = client
                    .get(&config.endpoint)
                    .send()
                    .await
                    .ok()?;
                
                let data = response.json::<Value>().await.ok()?;
                
                // Check for changes
                let changed = last_data.as_ref() != Some(&data);
                
                let event = PollingEvent {
                    data: data.clone(),
                    changed,
                    timestamp: Utc::now(),
                };
                
                Some((event, (config, Some(data), shutdown)))
            }
        );
        
        Ok(Box::pin(stream))
    }
    
    async fn stop(&mut self) -> Result<(), ActionError> {
        self.shutdown.store(true, Ordering::Relaxed);
        Ok(())
    }
}
```

### Message Queue Trigger

```rust
pub struct KafkaTrigger {
    metadata: ActionMetadata,
    consumer: Option<StreamConsumer>,
}

#[derive(Deserialize)]
pub struct KafkaConfig {
    pub brokers: Vec<String>,
    pub topic: String,
    pub group_id: String,
    pub auto_offset_reset: String,
}

#[derive(Serialize, Clone)]
pub struct KafkaEvent {
    pub topic: String,
    pub partition: i32,
    pub offset: i64,
    pub key: Option<String>,
    pub value: String,
    pub headers: HashMap<String, String>,
    pub timestamp: DateTime<Utc>,
}

#[async_trait]
impl TriggerAction for KafkaTrigger {
    type Config = KafkaConfig;
    type Event = KafkaEvent;
    
    async fn start(
        &mut self,
        config: Self::Config,
        context: &TriggerContext,
    ) -> Result<TriggerEventStream<Self::Event>, ActionError> {
        // Create Kafka consumer
        let consumer: StreamConsumer = ClientConfig::new()
            .set("bootstrap.servers", config.brokers.join(","))
            .set("group.id", &config.group_id)
            .set("auto.offset.reset", &config.auto_offset_reset)
            .set("enable.auto.commit", "true")
            .create()
            .map_err(|e| ActionError::TriggerError(e.to_string()))?;
        
        // Subscribe to topic
        consumer
            .subscribe(&[&config.topic])
            .map_err(|e| ActionError::TriggerError(e.to_string()))?;
        
        self.consumer = Some(consumer.clone());
        
        // Create event stream
        let stream = stream::unfold(consumer, |consumer| async move {
            match consumer.recv().await {
                Ok(message) => {
                    let event = KafkaEvent {
                        topic: message.topic().to_string(),
                        partition: message.partition(),
                        offset: message.offset(),
                        key: message.key().map(|k| String::from_utf8_lossy(k).to_string()),
                        value: String::from_utf8_lossy(message.payload()?).to_string(),
                        headers: extract_kafka_headers(&message),
                        timestamp: message.timestamp().to_millis()
                            .map(|ms| Utc.timestamp_millis(ms))
                            .unwrap_or_else(Utc::now),
                    };
                    
                    Some((event, consumer))
                }
                Err(e) => {
                    // Log error and continue
                    eprintln!("Kafka error: {}", e);
                    Some((
                        KafkaEvent::error_event(e.to_string()),
                        consumer
                    ))
                }
            }
        });
        
        Ok(Box::pin(stream))
    }
    
    async fn stop(&mut self) -> Result<(), ActionError> {
        if let Some(consumer) = self.consumer.take() {
            consumer.unsubscribe();
        }
        Ok(())
    }
}
```

### File Watcher Trigger

```rust
pub struct FileWatcherTrigger {
    metadata: ActionMetadata,
    watcher: Option<RecommendedWatcher>,
}

#[derive(Deserialize)]
pub struct WatcherConfig {
    pub paths: Vec<PathBuf>,
    pub recursive: bool,
    pub filters: Vec<String>,
    pub debounce_ms: u64,
}

#[derive(Serialize, Clone)]
pub struct FileEvent {
    pub path: PathBuf,
    pub event_type: FileEventType,
    pub timestamp: DateTime<Utc>,
    pub metadata: Option<FileMetadata>,
}

#[derive(Serialize, Clone)]
pub enum FileEventType {
    Created,
    Modified,
    Deleted,
    Renamed { from: PathBuf, to: PathBuf },
}

#[async_trait]
impl TriggerAction for FileWatcherTrigger {
    type Config = WatcherConfig;
    type Event = FileEvent;
    
    async fn start(
        &mut self,
        config: Self::Config,
        context: &TriggerContext,
    ) -> Result<TriggerEventStream<Self::Event>, ActionError> {
        let (tx, rx) = mpsc::channel(100);
        
        // Create file watcher
        let tx_clone = tx.clone();
        let mut watcher = notify::recommended_watcher(
            move |event: Result<notify::Event, notify::Error>| {
                if let Ok(event) = event {
                    let file_event = convert_notify_event(event);
                    let _ = tx_clone.blocking_send(file_event);
                }
            }
        ).map_err(|e| ActionError::TriggerError(e.to_string()))?;
        
        // Watch paths
        for path in &config.paths {
            let mode = if config.recursive {
                RecursiveMode::Recursive
            } else {
                RecursiveMode::NonRecursive
            };
            
            watcher.watch(path, mode)
                .map_err(|e| ActionError::TriggerError(e.to_string()))?;
            
            context.log_info(&format!("Watching path: {:?}", path));
        }
        
        self.watcher = Some(watcher);
        
        // Apply filters and debouncing
        let stream = ReceiverStream::new(rx)
            .filter(move |event| {
                future::ready(should_process_file(event, &config.filters))
            })
            .debounce(Duration::from_millis(config.debounce_ms));
        
        Ok(Box::pin(stream))
    }
    
    async fn stop(&mut self) -> Result<(), ActionError> {
        self.watcher = None; // Drop watcher to stop watching
        Ok(())
    }
}
```

### Schedule Trigger

```rust
pub struct ScheduleTrigger {
    metadata: ActionMetadata,
    scheduler: Option<JobScheduler>,
}

#[derive(Deserialize)]
pub struct ScheduleConfig {
    pub cron: String,
    pub timezone: String,
    pub metadata: HashMap<String, Value>,
}

#[derive(Serialize, Clone)]
pub struct ScheduleEvent {
    pub scheduled_time: DateTime<Utc>,
    pub actual_time: DateTime<Utc>,
    pub metadata: HashMap<String, Value>,
}

#[async_trait]
impl TriggerAction for ScheduleTrigger {
    type Config = ScheduleConfig;
    type Event = ScheduleEvent;
    
    async fn start(
        &mut self,
        config: Self::Config,
        context: &TriggerContext,
    ) -> Result<TriggerEventStream<Self::Event>, ActionError> {
        let (tx, rx) = mpsc::channel(100);
        
        // Parse cron expression
        let schedule = Schedule::from_str(&config.cron)
            .map_err(|e| ActionError::InvalidInput {
                field: "cron".to_string(),
                reason: e.to_string(),
            })?;
        
        // Parse timezone
        let tz: Tz = config.timezone.parse()
            .map_err(|e| ActionError::InvalidInput {
                field: "timezone".to_string(),
                reason: e.to_string(),
            })?;
        
        // Create scheduler
        let scheduler = JobScheduler::new().await
            .map_err(|e| ActionError::TriggerError(e.to_string()))?;
        
        // Add job
        let job = Job::new_async(config.cron.as_str(), move |uuid, mut scheduler| {
            let tx = tx.clone();
            let metadata = config.metadata.clone();
            
            Box::pin(async move {
                let event = ScheduleEvent {
                    scheduled_time: scheduler.next_tick_for_job(uuid).await.unwrap(),
                    actual_time: Utc::now(),
                    metadata,
                };
                
                let _ = tx.send(event).await;
            })
        }).map_err(|e| ActionError::TriggerError(e.to_string()))?;
        
        scheduler.add(job).await
            .map_err(|e| ActionError::TriggerError(e.to_string()))?;
        
        // Start scheduler
        scheduler.start().await
            .map_err(|e| ActionError::TriggerError(e.to_string()))?;
        
        self.scheduler = Some(scheduler);
        
        context.log_info(&format!(
            "Schedule trigger started with cron: {}",
            config.cron
        ));
        
        let stream = ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }
    
    async fn stop(&mut self) -> Result<(), ActionError> {
        if let Some(mut scheduler) = self.scheduler.take() {
            scheduler.shutdown().await
                .map_err(|e| ActionError::TriggerError(e.to_string()))?;
        }
        Ok(())
    }
}
```

## Event Stream Management

### Backpressure Handling

```rust
impl TriggerAction for BackpressureTrigger {
    async fn start(
        &mut self,
        config: Self::Config,
        context: &TriggerContext,
    ) -> Result<TriggerEventStream<Self::Event>, ActionError> {
        let (tx, rx) = mpsc::channel(config.buffer_size);
        
        // Handle backpressure
        tokio::spawn(async move {
            loop {
                let event = generate_event().await;
                
                // Try send with timeout
                match timeout(Duration::from_secs(5), tx.send(event)).await {
                    Ok(Ok(_)) => {
                        // Event sent successfully
                    }
                    Ok(Err(_)) => {
                        // Channel full, apply backpressure
                        log::warn!("Channel full, dropping event");
                        // Could also: slow down, buffer to disk, etc.
                    }
                    Err(_) => {
                        // Timeout, receiver might be stuck
                        log::error!("Send timeout, receiver might be stuck");
                        break;
                    }
                }
            }
        });
        
        let stream = ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }
}
```

### Error Recovery

```rust
impl TriggerAction for ResilientTrigger {
    async fn start(
        &mut self,
        config: Self::Config,
        context: &TriggerContext,
    ) -> Result<TriggerEventStream<Self::Event>, ActionError> {
        let stream = stream::unfold(
            (config, 0usize),
            |(config, retry_count)| async move {
                loop {
                    match fetch_event(&config).await {
                        Ok(event) => {
                            return Some((event, (config, 0)));
                        }
                        Err(e) if retry_count < config.max_retries => {
                            // Exponential backoff
                            let delay = Duration::from_secs(2_u64.pow(retry_count as u32));
                            tokio::time::sleep(delay).await;
                            return Some((
                                Event::retry_event(e),
                                (config, retry_count + 1)
                            ));
                        }
                        Err(e) => {
                            // Max retries exceeded
                            return Some((
                                Event::error_event(e),
                                (config, 0)
                            ));
                        }
                    }
                }
            }
        );
        
        Ok(Box::pin(stream))
    }
}
```

## Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::testing::*;
    
    #[tokio::test]
    async fn test_webhook_trigger_start() {
        let mut trigger = WebhookTrigger::new().unwrap();
        let context = TestTriggerContext::new();
        
        let config = WebhookConfig {
            port: 8080,
            path: "/webhook".to_string(),
            auth_token: None,
            max_body_size: None,
        };
        
        let stream = trigger.start(config, &context).await.unwrap();
        
        // Send test request
        let client = reqwest::Client::new();
        let response = client
            .post("http://localhost:8080/webhook")
            .json(&json!({ "test": "data" }))
            .send()
            .await
            .unwrap();
        
        assert_eq!(response.status(), 200);
        
        // Receive event
        let event = stream.next().await.unwrap();
        assert_eq!(event.method, "POST");
        assert!(event.body.is_some());
        
        trigger.stop().await.unwrap();
    }
    
    #[tokio::test]
    async fn test_polling_trigger() {
        let mut trigger = PollingTrigger::new().unwrap();
        let context = TestTriggerContext::new();
        
        // Start mock server
        let mock_server = MockServer::start().await;
        
        Mock::given(method("GET"))
            .and(path("/data"))
            .respond_with(ResponseTemplate::new(200)
                .set_body_json(json!({ "value": 42 })))
            .mount(&mock_server)
            .await;
        
        let config = PollingConfig {
            endpoint: format!("{}/data", mock_server.uri()),
            interval_seconds: 1,
            auth: None,
        };
        
        let mut stream = trigger.start(config, &context).await.unwrap();
        
        // Get first event
        let event = timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap();
        
        assert_eq!(event.data["value"], 42);
        
        trigger.stop().await.unwrap();
    }
}
```

## Best Practices

### ✅ DO's

1. **Handle backpressure** - Don't overwhelm downstream
2. **Implement graceful shutdown** - Clean up resources
3. **Add retry logic** - Handle transient failures
4. **Log important events** - Aid debugging
5. **Validate configuration** - Fail fast on bad config
6. **Use appropriate buffering** - Balance memory vs throughput

### ❌ DON'Ts

1. **Don't block the stream** - Keep events flowing
2. **Don't ignore errors** - At least log them
3. **Don't leak resources** - Clean up on stop
4. **Don't assume ordering** - Events might arrive out of order
5. **Don't forget monitoring** - Track trigger health

## Templates

### TriggerAction Template

```rust
// <% tp.file.cursor() %>
use nebula_action::prelude::*;
use serde::{Deserialize, Serialize};
use futures::stream::Stream;

#[derive(Deserialize)]
pub struct <%= tp.file.title %>Config {
    // TODO: Define configuration
}

#[derive(Serialize, Clone)]
pub struct <%= tp.file.title %>Event {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    // TODO: Define event fields
}

pub struct <%= tp.file.title %>Trigger {
    metadata: ActionMetadata,
    // TODO: Add state fields
}

impl <%= tp.file.title %>Trigger {
    pub fn new() -> Result<Self, ActionError> {
        let metadata = ActionMetadata::builder()
            .key("<%= tp.file.title.toLowerCase() %>.trigger")
            .name("<%= tp.file.title %> Trigger")
            .description("TODO: Add description")
            .version("1.0.0")
            .build()?;
        
        Ok(Self {
            metadata,
        })
    }
}

impl HasMetadata for <%= tp.file.title %>Trigger {
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl HasType for <%= tp.file.title %>Trigger {
    fn r#type(&self) -> ActionType {
        ActionType::Trigger
    }
}

impl Action for <%= tp.file.title %>Trigger {}

#[async_trait]
impl TriggerAction for <%= tp.file.title %>Trigger {
    type Config = <%= tp.file.title %>Config;
    type Event = <%= tp.file.title %>Event;
    
    async fn start(
        &mut self,
        config: Self::Config,
        context: &TriggerContext,
    ) -> Result<TriggerEventStream<Self::Event>, ActionError> {
        // TODO: Implement trigger logic
        
        let stream = stream::empty();
        Ok(Box::pin(stream))
    }
    
    async fn stop(&mut self) -> Result<(), ActionError> {
        // TODO: Cleanup resources
        Ok(())
    }
}
```

## Related Documentation

- [[Action Types#TriggerAction]] - TriggerAction overview
- [[how-to/PollingAction]] - Polling triggers
- [[how-to/WebhookAction]] - Webhook handling
- [[Examples#TriggerAction Examples]] - More examples