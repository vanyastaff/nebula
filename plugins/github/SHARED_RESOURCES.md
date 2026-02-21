# Shared Trigger Resources with Event Bus Architecture

## 🎯 Core Concept

**Problem**: Multiple workflows can subscribe to the same trigger (e.g. GitHub push events from the same repo). Creating N webhooks/pollers is wasteful and hits API rate limits.

**Solution**: 
1. **Shared Trigger Resources** - One resource instance per unique trigger configuration
2. **Event Bus** - Resources emit events to Kafka/RabbitMQ
3. **Event Routing** - Engine subscribes workflows to event streams

---

## 🏗️ Architecture Overview

```
┌──────────────────────────────────────────────────────────────────┐
│                    EXTERNAL SERVICES                              │
│  GitHub │ Slack │ Stripe │ Discord │ Custom                      │
└────┬─────┴───┬───┴────┬───┴────┬────┴────┬───────────────────────┘
     │         │        │        │         │
     │ Webhook │ Events │ Events │ Webhook │ HTTP
     │         │        │        │         │
  ┌──▼─────────▼────────▼────────▼─────────▼────────────────────┐
  │         TRIGGER RESOURCES (Long-Running)                     │
  │                                                              │
  │  GithubWebhookResource {                                    │
  │    config: { owner: "octocat", repo: "Hello-World" }       │
  │    credential: GithubToken                                  │
  │    http_server: AxumServer                                  │
  │    webhook_id: "12345"                                      │
  │  }                                                           │
  │                                                              │
  │  On webhook received:                                       │
  │    1. Verify signature                                      │
  │    2. Parse event                                           │
  │    3. emit_event() → Event Bus                             │
  │                                                              │
  │  GithubIssuePollResource {                                  │
  │    config: { owner: "octocat", repo: "Hello-World" }       │
  │    credential: GithubToken                                  │
  │    poll_task: TokioTask                                     │
  │    state: { since: DateTime, last_id: 123 }                │
  │  }                                                           │
  │                                                              │
  │  On poll interval:                                          │
  │    1. Query GitHub API                                      │
  │    2. Filter new events                                     │
  │    3. emit_event() for each → Event Bus                    │
  └──────────────────────┬───────────────────────────────────────┘
                         │ TriggerEvent
                         │
      ┌──────────────────▼──────────────────┐
      │       EVENT BUS                     │
      │  Kafka / RabbitMQ / NATS / Redis   │
      │                                     │
      │  Topics/Streams:                   │
      │   - triggers.github.push           │
      │   - triggers.github.issues         │
      │   - triggers.slack.message         │
      │   - triggers.stripe.payment        │
      │                                     │
      │  Event Format:                     │
      │  {                                  │
      │    trigger_id: "github-webhook-1", │
      │    event_type: "push",             │
      │    data: {...},                    │
      │    timestamp: "...",               │
      │    dedup_key: "..."                │
      │  }                                  │
      └──────────────────┬──────────────────┘
                         │ Subscribe
                         │
      ┌──────────────────▼──────────────────┐
      │     WORKFLOW SUBSCRIPTIONS          │
      │                                     │
      │  Workflow wf-123:                  │
      │    trigger: "github-webhook-1"     │
      │    filters: { event: "push" }      │
      │                                     │
      │  Workflow wf-456:                  │
      │    trigger: "github-webhook-1"     │
      │    filters: { event: "issues" }    │
      │                                     │
      │  Workflow wf-789:                  │
      │    trigger: "github-webhook-1"     │
      │    filters: { event: "push" }      │
      └──────────────────┬──────────────────┘
                         │ Matching events
                         │
      ┌──────────────────▼──────────────────┐
      │      NEBULA ENGINE                  │
      │                                     │
      │  For each matching workflow:       │
      │    1. Load workflow definition     │
      │    2. Create execution context     │
      │    3. Execute first action         │
      │    4. Continue workflow            │
      └─────────────────────────────────────┘
```

---

## 🔧 Trigger Resource Trait

```rust
use tokio::sync::mpsc;
use async_trait::async_trait;

/// Resource that emits trigger events to an event bus
#[async_trait]
pub trait TriggerResource: Resource {
    type Config: Config + Clone;
    type Event: Serialize + DeserializeOwned + Send + Sync + 'static;
    type Credential: Send + Sync + 'static;
    
    /// Subscribe to external service (webhook registration, event subscription)
    async fn subscribe(
        &self,
        config: &Self::Config,
        credential: &Self::Credential,
        ctx: &Context,
    ) -> Result<SubscriptionInfo>;
    
    /// Unsubscribe from external service
    async fn unsubscribe(
        &self,
        config: &Self::Config,
        credential: &Self::Credential,
        subscription: &SubscriptionInfo,
        ctx: &Context,
    ) -> Result<()>;
    
    /// Test connection/configuration
    async fn test(
        &self,
        config: &Self::Config,
        credential: &Self::Credential,
        ctx: &Context,
    ) -> Result<TestResult>;
}

/// Active trigger resource instance that emits events
pub struct TriggerResourceInstance<E> {
    /// Unique trigger ID
    pub trigger_id: String,
    
    /// Subscription info (webhook ID, etc.)
    pub subscription: SubscriptionInfo,
    
    /// Event emitter (sends to event bus)
    pub emitter: EventEmitter<E>,
    
    /// Handle to background task (HTTP server, poll loop, etc.)
    pub task_handle: JoinHandle<()>,
}

impl<E> TriggerResourceInstance<E> {
    /// Emit event to event bus
    pub async fn emit(&self, event: TriggerEvent<E>) -> Result<()> {
        self.emitter.emit(event).await
    }
}
```

---

## 📦 Event Emitter

```rust
use serde_json::Value;

/// Emits trigger events to event bus
pub struct EventEmitter<E> {
    trigger_id: String,
    event_tx: mpsc::UnboundedSender<TriggerEventMessage>,
    _phantom: PhantomData<E>,
}

impl<E: Serialize> EventEmitter<E> {
    pub fn new(trigger_id: String, event_tx: mpsc::UnboundedSender<TriggerEventMessage>) -> Self {
        Self {
            trigger_id,
            event_tx,
            _phantom: PhantomData,
        }
    }
    
    /// Emit event to event bus
    pub async fn emit(&self, event: TriggerEvent<E>) -> Result<(), ActionError> {
        let message = TriggerEventMessage {
            trigger_id: self.trigger_id.clone(),
            event_type: std::any::type_name::<E>().to_string(),
            data: serde_json::to_value(&event.data)
                .map_err(|e| ActionError::fatal(format!("failed to serialize event: {e}")))?,
            timestamp: event.timestamp,
            dedup_key: event.dedup_key,
        };
        
        self.event_tx
            .send(message)
            .map_err(|_| ActionError::fatal("event bus disconnected"))?;
        
        Ok(())
    }
}

/// Message sent to event bus
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggerEventMessage {
    /// Trigger ID that emitted this event
    pub trigger_id: String,
    
    /// Event type (for routing)
    pub event_type: String,
    
    /// Event data (serialized)
    pub data: Value,
    
    /// When event occurred
    pub timestamp: DateTime<Utc>,
    
    /// Deduplication key
    pub dedup_key: Option<String>,
}
```

---

## 🎨 Example: GitHub Webhook Resource

```rust
use octocrab::Octocrab;
use axum::Router;

/// Configuration for GitHub webhook trigger resource
#[derive(Clone, Debug, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub struct GithubWebhookResourceConfig {
    pub owner: String,
    pub repo: String,
    pub events: Vec<String>,
}

impl Config for GithubWebhookResourceConfig {
    fn validate(&self) -> Result<()> {
        if self.owner.is_empty() || self.repo.is_empty() {
            return Err(Error::validation("owner and repo required"));
        }
        Ok(())
    }
}

/// GitHub webhook trigger resource
pub struct GithubWebhookResource;

impl Resource for GithubWebhookResource {
    type Config = GithubWebhookResourceConfig;
    type Instance = TriggerResourceInstance<GithubWebhookEvent>;

    fn id(&self) -> &str {
        "github-webhook"
    }

    async fn create(&self, config: &Self::Config, ctx: &Context) -> Result<Self::Instance> {
        // 1. Get credential from context
        let credential = ctx.get_credential::<GithubCredential>()?;
        
        // 2. Create GitHub client
        let client = Octocrab::builder()
            .personal_token(credential.token.clone())
            .build()?;
        
        // 3. Generate webhook secret
        let webhook_secret = generate_webhook_secret();
        
        // 4. Generate webhook URL
        let trigger_id = format!("github-webhook-{}-{}", config.owner, config.repo);
        let webhook_url = format!("{}/webhooks/{}", ctx.base_url, trigger_id);
        
        // 5. Register webhook with GitHub
        let webhook_response = client
            ._post(
                format!("/repos/{}/{}/hooks", config.owner, config.repo),
                Some(&serde_json::json!({
                    "name": "web",
                    "config": {
                        "url": webhook_url,
                        "content_type": "json",
                        "secret": webhook_secret,
                        "insecure_ssl": "0",
                    },
                    "events": config.events,
                    "active": true,
                }))
            )
            .await?;
        
        let webhook_id = webhook_response["id"].as_i64().unwrap().to_string();
        
        // 6. Create event emitter
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let emitter = EventEmitter::new(trigger_id.clone(), event_tx.clone());
        
        // 7. Start HTTP server to receive webhooks
        let server_secret = webhook_secret.clone();
        let server_config = config.clone();
        let server_emitter = emitter.clone();
        
        let task_handle = tokio::spawn(async move {
            let app = Router::new()
                .route(&format!("/{}", trigger_id), post(move |req| {
                    handle_github_webhook(
                        req,
                        server_config.clone(),
                        server_secret.clone(),
                        server_emitter.clone(),
                    )
                }));
            
            let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
            axum::serve(listener, app).await.unwrap();
        });
        
        // 8. Create subscription info
        let subscription = SubscriptionInfo::new(webhook_id, webhook_secret)
            .with_url(webhook_url)
            .with_metadata("repo", serde_json::json!({
                "owner": config.owner,
                "repo": config.repo,
            }));
        
        Ok(TriggerResourceInstance {
            trigger_id,
            subscription,
            emitter,
            task_handle,
        })
    }

    async fn cleanup(&self, instance: Self::Instance) -> Result<()> {
        // 1. Abort HTTP server
        instance.task_handle.abort();
        
        // 2. Unsubscribe from GitHub (if we stored credential)
        // This would need to be enhanced to access credential during cleanup
        
        Ok(())
    }
}

#[async_trait]
impl TriggerResource for GithubWebhookResource {
    type Config = GithubWebhookResourceConfig;
    type Event = GithubWebhookEvent;
    type Credential = GithubCredential;
    
    async fn subscribe(
        &self,
        config: &Self::Config,
        credential: &Self::Credential,
        ctx: &Context,
    ) -> Result<SubscriptionInfo> {
        // Already handled in create(), but can be called separately
        let client = create_github_client(credential)?;
        let webhook_secret = generate_webhook_secret();
        let webhook_url = format!("{}/webhooks/github-{}-{}", ctx.base_url, config.owner, config.repo);
        
        let response = client
            ._post(
                format!("/repos/{}/{}/hooks", config.owner, config.repo),
                Some(&serde_json::json!({
                    "config": {
                        "url": webhook_url,
                        "secret": webhook_secret,
                    },
                    "events": config.events,
                }))
            )
            .await?;
        
        Ok(SubscriptionInfo::new(
            response["id"].as_i64().unwrap().to_string(),
            webhook_secret,
        ))
    }
    
    async fn unsubscribe(
        &self,
        config: &Self::Config,
        credential: &Self::Credential,
        subscription: &SubscriptionInfo,
        ctx: &Context,
    ) -> Result<()> {
        let client = create_github_client(credential)?;
        
        client
            ._delete(
                format!("/repos/{}/{}/hooks/{}", config.owner, config.repo, subscription.webhook_id),
                None::<&()>
            )
            .await?;
        
        Ok(())
    }
    
    async fn test(
        &self,
        config: &Self::Config,
        credential: &Self::Credential,
        ctx: &Context,
    ) -> Result<TestResult> {
        let client = create_github_client(credential)?;
        
        match client.repos(&config.owner, &config.repo).get().await {
            Ok(repo) => Ok(TestResult::success(format!(
                "Connected to repository: {}",
                repo.full_name.unwrap_or_default()
            ))),
            Err(e) => Ok(TestResult::failed(format!("Connection failed: {e}"))),
        }
    }
}

/// Handle incoming GitHub webhook
async fn handle_github_webhook(
    request: axum::extract::Request,
    config: GithubWebhookResourceConfig,
    webhook_secret: String,
    emitter: EventEmitter<GithubWebhookEvent>,
) -> axum::http::StatusCode {
    // 1. Extract headers
    let headers = request.headers();
    
    // 2. Verify signature
    let signature = headers.get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    
    let raw_body = axum::body::to_bytes(request.into_body(), usize::MAX).await.unwrap();
    
    if !verify_github_signature(&webhook_secret, &raw_body, signature) {
        return axum::http::StatusCode::UNAUTHORIZED;
    }
    
    // 3. Parse body
    let body: serde_json::Value = match serde_json::from_slice(&raw_body) {
        Ok(v) => v,
        Err(_) => return axum::http::StatusCode::BAD_REQUEST,
    };
    
    // 4. Extract event type
    let event_type = headers.get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("unknown")
        .to_string();
    
    // 5. Filter by configured events
    if !config.events.contains(&event_type) {
        return axum::http::StatusCode::OK; // Ignore silently
    }
    
    // 6. Create event
    let event = GithubWebhookEvent {
        event_type: event_type.clone(),
        action: body.get("action").and_then(|v| v.as_str()).map(String::from),
        payload: body,
    };
    
    // 7. Emit to event bus
    let dedup_key = headers.get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    
    if let Err(e) = emitter.emit(TriggerEvent::with_dedup(event, dedup_key)).await {
        tracing::error!("failed to emit event: {e}");
        return axum::http::StatusCode::INTERNAL_SERVER_ERROR;
    }
    
    axum::http::StatusCode::OK
}
```

---

## 🎨 Example: GitHub Poll Resource

```rust
/// Configuration for GitHub issue poll resource
#[derive(Clone, Debug, Serialize, Deserialize, Hash, Eq, PartialEq)]
pub struct GithubIssuePollResourceConfig {
    pub owner: String,
    pub repo: String,
    pub interval_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GithubIssuePollState {
    pub since: Option<DateTime<Utc>>,
    pub last_issue_id: Option<u64>,
}

pub struct GithubIssuePollResource;

impl Resource for GithubIssuePollResource {
    type Config = GithubIssuePollResourceConfig;
    type Instance = TriggerResourceInstance<IssueEvent>;

    fn id(&self) -> &str {
        "github-issue-poll"
    }

    async fn create(&self, config: &Self::Config, ctx: &Context) -> Result<Self::Instance> {
        // 1. Get credential
        let credential = ctx.get_credential::<GithubCredential>()?;
        
        // 2. Create GitHub client
        let client = Octocrab::builder()
            .personal_token(credential.token.clone())
            .build()?;
        
        // 3. Validate access
        client.repos(&config.owner, &config.repo).get().await?;
        
        // 4. Create event emitter
        let trigger_id = format!("github-issue-poll-{}-{}", config.owner, config.repo);
        let (event_tx, _event_rx) = mpsc::unbounded_channel();
        let emitter = EventEmitter::new(trigger_id.clone(), event_tx);
        
        // 5. Start polling task
        let poll_config = config.clone();
        let poll_client = client.clone();
        let poll_emitter = emitter.clone();
        let mut poll_state = GithubIssuePollState {
            since: Some(Utc::now()),
            last_issue_id: None,
        };
        
        let task_handle = tokio::spawn(async move {
            let interval = Duration::from_secs(poll_config.interval_seconds);
            
            loop {
                tokio::time::sleep(interval).await;
                
                match poll_github_issues(
                    &poll_client,
                    &poll_config,
                    &poll_state,
                    &poll_emitter,
                ).await {
                    Ok(new_state) => {
                        poll_state = new_state;
                    }
                    Err(e) => {
                        tracing::error!("poll error: {e}");
                    }
                }
            }
        });
        
        Ok(TriggerResourceInstance {
            trigger_id,
            subscription: SubscriptionInfo::new("poll", ""), // No webhook ID for polling
            emitter,
            task_handle,
        })
    }

    async fn cleanup(&self, instance: Self::Instance) -> Result<()> {
        instance.task_handle.abort();
        Ok(())
    }
}

async fn poll_github_issues(
    client: &Octocrab,
    config: &GithubIssuePollResourceConfig,
    state: &GithubIssuePollState,
    emitter: &EventEmitter<IssueEvent>,
) -> Result<GithubIssuePollState> {
    // 1. Query issues
    let mut query = client
        .issues(&config.owner, &config.repo)
        .list()
        .state(octocrab::params::State::All)
        .per_page(100);
    
    if let Some(since) = state.since {
        query = query.since(since);
    }
    
    let page = query.send().await?;
    
    // 2. Filter and emit events
    let mut new_state = state.clone();
    
    for issue in page.items {
        // Skip if already seen
        if let Some(last_id) = state.last_issue_id {
            if issue.number <= last_id {
                continue;
            }
        }
        
        // Update state
        if new_state.since.is_none() || Some(issue.updated_at) > new_state.since {
            new_state.since = Some(issue.updated_at);
        }
        new_state.last_issue_id = Some(issue.number);
        
        // Emit event
        let event = IssueEvent::from(issue.clone());
        emitter.emit(TriggerEvent::with_dedup(
            event,
            format!("issue-{}", issue.number),
        )).await?;
    }
    
    Ok(new_state)
}
```

---

## 🚀 Resource Manager Integration

```rust
use std::collections::HashMap;
use std::sync::Arc;
use dashmap::DashMap;

/// Manages shared trigger resources
pub struct TriggerResourceManager {
    /// Map: (resource_type, config_hash) -> resource instance
    resources: Arc<DashMap<(String, u64), Arc<dyn Any + Send + Sync>>>,
    
    /// Map: trigger_id -> list of workflow IDs subscribed
    subscriptions: Arc<DashMap<String, Vec<WorkflowId>>>,
    
    /// Event bus sender
    event_bus_tx: mpsc::UnboundedSender<TriggerEventMessage>,
}

impl TriggerResourceManager {
    pub fn new(event_bus_tx: mpsc::UnboundedSender<TriggerEventMessage>) -> Self {
        Self {
            resources: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
            event_bus_tx,
        }
    }
    
    /// Acquire or create shared trigger resource
    pub async fn acquire<R: TriggerResource>(
        &self,
        config: &R::Config,
        credential: &R::Credential,
        workflow_id: WorkflowId,
        ctx: &Context,
    ) -> Result<String> 
    where
        R::Config: Hash,
    {
        let resource_type = std::any::type_name::<R>();
        let config_hash = calculate_hash(config);
        let key = (resource_type.to_string(), config_hash);
        
        // Check if resource already exists
        if !self.resources.contains_key(&key) {
            // Create new resource
            let resource = R::default();
            let instance = resource.create(config, ctx).await?;
            let trigger_id = instance.trigger_id.clone();
            
            self.resources.insert(key.clone(), Arc::new(instance));
            
            tracing::info!(
                resource_type = %resource_type,
                trigger_id = %trigger_id,
                "created shared trigger resource"
            );
        }
        
        // Get trigger ID
        let instance = self.resources.get(&key).unwrap();
        let instance = instance
            .downcast_ref::<TriggerResourceInstance<R::Event>>()
            .unwrap();
        let trigger_id = instance.trigger_id.clone();
        
        // Subscribe workflow to this trigger
        self.subscriptions
            .entry(trigger_id.clone())
            .or_insert_with(Vec::new)
            .push(workflow_id);
        
        tracing::info!(
            workflow_id = %workflow_id,
            trigger_id = %trigger_id,
            "workflow subscribed to trigger"
        );
        
        Ok(trigger_id)
    }
    
    /// Release trigger resource (unsubscribe workflow)
    pub async fn release(
        &self,
        trigger_id: &str,
        workflow_id: WorkflowId,
    ) -> Result<()> {
        // Remove workflow subscription
        if let Some(mut workflows) = self.subscriptions.get_mut(trigger_id) {
            workflows.retain(|&wf_id| wf_id != workflow_id);
            
            // If no more workflows subscribed, cleanup resource
            if workflows.is_empty() {
                drop(workflows); // Release lock
                self.cleanup_resource(trigger_id).await?;
            }
        }
        
        tracing::info!(
            workflow_id = %workflow_id,
            trigger_id = %trigger_id,
            "workflow unsubscribed from trigger"
        );
        
        Ok(())
    }
    
    async fn cleanup_resource(&self, trigger_id: &str) -> Result<()> {
        // Find and remove resource by trigger_id
        // This would need reverse lookup or additional indexing
        
        tracing::info!(
            trigger_id = %trigger_id,
            "cleaned up unused trigger resource"
        );
        
        Ok(())
    }
}

fn calculate_hash<T: Hash>(obj: &T) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    
    let mut hasher = DefaultHasher::new();
    obj.hash(&mut hasher);
    hasher.finish()
}
```

---

## 📡 Event Bus Integration

```rust
/// Event bus abstraction (supports Kafka, RabbitMQ, Redis, etc.)
#[async_trait]
pub trait EventBus: Send + Sync {
    /// Publish event to topic/stream
    async fn publish(&self, topic: &str, event: TriggerEventMessage) -> Result<()>;
    
    /// Subscribe to topic/stream
    async fn subscribe(&self, topic: &str) -> Result<mpsc::UnboundedReceiver<TriggerEventMessage>>;
}

/// Kafka implementation
pub struct KafkaEventBus {
    producer: rdkafka::producer::FutureProducer,
    consumer_config: rdkafka::ClientConfig,
}

#[async_trait]
impl EventBus for KafkaEventBus {
    async fn publish(&self, topic: &str, event: TriggerEventMessage) -> Result<()> {
        let payload = serde_json::to_vec(&event)?;
        
        let record = rdkafka::producer::FutureRecord::to(topic)
            .payload(&payload)
            .key(&event.trigger_id);
        
        self.producer.send(record, Duration::from_secs(5)).await
            .map_err(|(e, _)| Error::external(format!("kafka publish failed: {e}")))?;
        
        Ok(())
    }
    
    async fn subscribe(&self, topic: &str) -> Result<mpsc::UnboundedReceiver<TriggerEventMessage>> {
        let consumer: rdkafka::consumer::StreamConsumer = self.consumer_config
            .create()
            .map_err(|e| Error::external(format!("kafka consumer creation failed: {e}")))?;
        
        consumer.subscribe(&[topic])
            .map_err(|e| Error::external(format!("kafka subscribe failed: {e}")))?;
        
        let (tx, rx) = mpsc::unbounded_channel();
        
        tokio::spawn(async move {
            use rdkafka::message::Message;
            
            loop {
                match consumer.recv().await {
                    Ok(message) => {
                        if let Some(payload) = message.payload() {
                            if let Ok(event) = serde_json::from_slice::<TriggerEventMessage>(payload) {
                                if tx.send(event).is_err() {
                                    break; // Channel closed
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("kafka receive error: {e}");
                    }
                }
            }
        });
        
        Ok(rx)
    }
}

/// RabbitMQ implementation
pub struct RabbitMQEventBus {
    connection: lapin::Connection,
    exchange: String,
}

#[async_trait]
impl EventBus for RabbitMQEventBus {
    async fn publish(&self, routing_key: &str, event: TriggerEventMessage) -> Result<()> {
        let channel = self.connection.create_channel().await?;
        
        let payload = serde_json::to_vec(&event)?;
        
        channel.basic_publish(
            &self.exchange,
            routing_key,
            lapin::options::BasicPublishOptions::default(),
            &payload,
            lapin::BasicProperties::default(),
        ).await?;
        
        Ok(())
    }
    
    async fn subscribe(&self, routing_key: &str) -> Result<mpsc::UnboundedReceiver<TriggerEventMessage>> {
        let channel = self.connection.create_channel().await?;
        
        let queue = channel.queue_declare(
            "",
            lapin::options::QueueDeclareOptions {
                exclusive: true,
                auto_delete: true,
                ..Default::default()
            },
            lapin::types::FieldTable::default(),
        ).await?;
        
        channel.queue_bind(
            queue.name().as_str(),
            &self.exchange,
            routing_key,
            lapin::options::QueueBindOptions::default(),
            lapin::types::FieldTable::default(),
        ).await?;
        
        let mut consumer = channel.basic_consume(
            queue.name().as_str(),
            "nebula-engine",
            lapin::options::BasicConsumeOptions::default(),
            lapin::types::FieldTable::default(),
        ).await?;
        
        let (tx, rx) = mpsc::unbounded_channel();
        
        tokio::spawn(async move {
            while let Some(delivery) = consumer.next().await {
                match delivery {
                    Ok(delivery) => {
                        if let Ok(event) = serde_json::from_slice::<TriggerEventMessage>(&delivery.data) {
                            if tx.send(event).is_err() {
                                break;
                            }
                        }
                        delivery.ack(lapin::options::BasicAckOptions::default()).await.ok();
                    }
                    Err(e) => {
                        tracing::error!("rabbitmq receive error: {e}");
                    }
                }
            }
        });
        
        Ok(rx)
    }
}
```

---

## 🔄 Complete Workflow Lifecycle

```rust
// ═══════════════════════════════════════════════════════════════
// WORKFLOW ACTIVATION
// ═══════════════════════════════════════════════════════════════

async fn activate_workflow(
    workflow_id: WorkflowId,
    workflow: &Workflow,
    resource_manager: &TriggerResourceManager,
    event_bus: &Arc<dyn EventBus>,
) -> Result<()> {
    // 1. Get trigger configuration
    let trigger_config = &workflow.trigger.config;
    let credential = load_credential(&workflow.trigger.credential)?;
    
    // 2. Acquire shared trigger resource (or reuse existing)
    let trigger_id = resource_manager
        .acquire::<GithubWebhookResource>(
            trigger_config,
            &credential,
            workflow_id,
            &context,
        )
        .await?;
    
    // 3. Subscribe to event bus topic
    let topic = format!("triggers.{}", trigger_id);
    let mut event_rx = event_bus.subscribe(&topic).await?;
    
    // 4. Store subscription info
    workflow_state.set("trigger_id", trigger_id);
    workflow_state.set("event_topic", topic.clone());
    
    // 5. Start event consumer task
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            // Check if event matches workflow filters
            if matches_workflow_filters(&event, &workflow.trigger.filters) {
                // Execute workflow
                executor.execute_workflow(workflow_id, event.data).await.ok();
            }
        }
    });
    
    tracing::info!(
        workflow_id = %workflow_id,
        trigger_id = %trigger_id,
        "workflow activated"
    );
    
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// EVENT EMISSION (from trigger resource)
// ═══════════════════════════════════════════════════════════════

// Inside GithubWebhookResource when webhook is received:
async fn on_webhook_received(
    event: GithubWebhookEvent,
    emitter: &EventEmitter<GithubWebhookEvent>,
) -> Result<()> {
    // Emit event to event bus
    emitter.emit(TriggerEvent::with_dedup(
        event,
        delivery_id,
    )).await?;
    
    // Event goes to Kafka/RabbitMQ topic: "triggers.github-webhook-octocat-Hello-World"
    // All workflows subscribed to this trigger receive the event
    
    Ok(())
}

// ═══════════════════════════════════════════════════════════════
// WORKFLOW DEACTIVATION
// ═══════════════════════════════════════════════════════════════

async fn deactivate_workflow(
    workflow_id: WorkflowId,
    resource_manager: &TriggerResourceManager,
) -> Result<()> {
    // 1. Load trigger ID
    let trigger_id: String = workflow_state.get("trigger_id")?;
    
    // 2. Release trigger resource (unsubscribe workflow)
    resource_manager.release(&trigger_id, workflow_id).await?;
    
    // If this was the last workflow using this trigger, resource is cleaned up automatically
    
    // 3. Clear workflow state
    workflow_state.remove("trigger_id");
    workflow_state.remove("event_topic");
    
    tracing::info!(
        workflow_id = %workflow_id,
        trigger_id = %trigger_id,
        "workflow deactivated"
    );
    
    Ok(())
}
```

---

## 🎯 Example Scenario

```
┌─────────────────────────────────────────────────────────────┐
│ 3 Workflows subscribe to same GitHub repo:                  │
│                                                              │
│ Workflow A: triggers.github-webhook-octocat-Hello-World    │
│   Filter: event_type = "push"                              │
│   Action: Send Slack notification                          │
│                                                              │
│ Workflow B: triggers.github-webhook-octocat-Hello-World    │
│   Filter: event_type = "issues"                            │
│   Action: Create Jira ticket                               │
│                                                              │
│ Workflow C: triggers.github-webhook-octocat-Hello-World    │
│   Filter: event_type = "push" AND branch = "main"         │
│   Action: Deploy to production                             │
└─────────────────────────────────────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────────┐
│ Resource Manager:                                            │
│   - Only ONE GithubWebhookResource instance created         │
│   - Registered once with GitHub API (webhook ID: 12345)    │
│   - Single HTTP server listening for webhooks              │
│   - Config: { owner: "octocat", repo: "Hello-World" }     │
└─────────────────────────────────────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────────┐
│ GitHub sends push event:                                     │
│   POST /webhooks/github-webhook-octocat-Hello-World         │
│   X-GitHub-Event: push                                      │
│   Body: { ref: "refs/heads/main", commits: [...] }         │
└─────────────────────────────────────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────────┐
│ GithubWebhookResource:                                      │
│   1. Verify signature                                       │
│   2. Parse event                                            │
│   3. emitter.emit(event) → Kafka                           │
└─────────────────────────────────────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────────┐
│ Kafka Topic: triggers.github-webhook-octocat-Hello-World   │
│   Message: {                                                │
│     trigger_id: "github-webhook-octocat-Hello-World",      │
│     event_type: "push",                                     │
│     data: { ref: "refs/heads/main", ... }                  │
│   }                                                          │
└─────────────────────────────────────────────────────────────┘
                          ↓
┌─────────────────────────────────────────────────────────────┐
│ Engine receives event, applies filters:                     │
│                                                              │
│ ✅ Workflow A: event_type = "push" → Execute                │
│ ❌ Workflow B: event_type = "issues" → Skip                 │
│ ✅ Workflow C: event_type = "push" AND branch = "main" → Execute│
└─────────────────────────────────────────────────────────────┘
```

---

## ✅ Benefits

| Feature | Benefit |
|---------|---------|
| **Resource Sharing** | One webhook/poller serves multiple workflows |
| **Scalability** | Event bus handles high throughput |
| **Decoupling** | Trigger resources independent from workflow execution |
| **Persistence** | Events in Kafka/RabbitMQ survive restarts |
| **Filtering** | Workflows receive only relevant events |
| **Credentials** | Stored once with resource, not per workflow |
| **Rate Limits** | Shared resource respects API limits |

---

## 📚 Summary

**Architecture:**
1. **Trigger Resources** - Long-running, shared instances with credentials
2. **Event Emitters** - Resources emit to Kafka/RabbitMQ
3. **Event Bus** - Distributes events to all subscribed workflows
4. **Workflow Filters** - Engine applies filters before execution
5. **Lifecycle Management** - Resources cleaned up when last workflow unsubscribes

**Next Steps:**
1. Implement `TriggerResource` trait
2. Create `TriggerResourceManager` with ref counting
3. Build `EventEmitter` with Kafka/RabbitMQ adapters
4. Implement first resource: `GithubWebhookResource`
5. Add workflow filtering and routing
