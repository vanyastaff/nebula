---
title: nebula-action — Overview
tags: [nebula, nebula-action, crate, docs]
status: published
created: 2025-08-17
last_updated: 2025-11-09
---

# nebula-action — Overview

**nebula-action** is the core crate that defines how units of work (actions) are implemented and executed in Nebula workflows. It provides traits, lifecycle management, and execution primitives for building type-safe, observable, and composable actions.

## What is an Action?

An action is a self-contained, typed unit of work that:

- Has clearly defined inputs and outputs (Rust structs)
- Can be composed into workflows
- Produces logs, metrics, and traces automatically
- Handles errors with rich error types
- Can be tested in isolation
- Can be versioned independently

```rust
use nebula_action::prelude::*;

// Define input and output types
#[derive(Deserialize)]
struct SendEmailInput {
    to: String,
    subject: String,
    body: String,
}

#[derive(Serialize)]
struct SendEmailOutput {
    message_id: String,
    sent_at: DateTime<Utc>,
}

// Implement the action using simple_action! macro
simple_action!(
    SendEmailAction,
    "email.send",
    SendEmailInput,
    SendEmailOutput,
    |_action, input, context| async move {
        // Your implementation here
        let message_id = send_email(&input.to, &input.subject, &input.body).await?;

        Ok(SendEmailOutput {
            message_id,
            sent_at: Utc::now(),
        })
    }
);
```

## When to Use nebula-action

Use this crate when you need to:

- **Create custom actions** — Build domain-specific actions for your workflows
- **Build triggers** — Implement polling, webhook, or event-based triggers that start workflows
- **Provide resources** — Create SupplyAction types that provide shared resources (DB pools, HTTP clients)
- **Handle stateful logic** — Implement StatefulAction for rate limiting, caching, or accumulation
- **Process streams** — Use StreamingAction for long-lived connections or data streams
- **Implement transactions** — Create TransactionalAction with undo/compensation logic
- **Add interactivity** — Build InteractiveAction that waits for user or system input

## When to Choose nebula-action

Choose nebula-action if you need:

✅ **Deterministic execution** — Same inputs always produce same outputs (testable, predictable)
✅ **Type safety** — Compile-time validation of inputs/outputs
✅ **Reusability** — Use actions across multiple workflows
✅ **Observability** — Built-in logging, metrics, and distributed tracing
✅ **Error handling** — Rich error types with automatic retry logic
✅ **Composability** — Build complex workflows from simple actions

## Action Types

nebula-action supports multiple action types, each optimized for different use cases:

| Type | Use Case | Example | Documentation |
|------|----------|---------|---------------|
| **ProcessAction** | Stateless sync/async processing | HTTP requests, data transformation | [[how-to/ProcessAction]] |
| **StatefulAction** | State management across executions | Rate limiting, caching, counters | [[how-to/StatefulAction]] |
| **TriggerAction** | Workflow entry points | Webhooks, polling, scheduled tasks | [[how-to/TriggerAction]] |
| **SupplyAction** | Resource provisioning | DB pools, HTTP clients, loggers | [[02-Crates/nebula-resource/README|nebula-resource]] |
| **StreamingAction** | Long-lived connections | WebSocket streams, database cursors | [[how-to/StreamingAction]] |
| **InteractiveAction** | Human/system input | Approval workflows, user forms | [[how-to/InteractiveAction]] |
| **TransactionalAction** | ACID operations | Multi-step transactions with rollback | [[how-to/TransactionalAction]] |
| **QueueAction** | Queue-based processing | Message queue consumers | [[how-to/QueueAction]] |
| **PollingAction** | Periodic data fetching | API polling, file monitoring | [[how-to/PollingAction]] |
| **ScheduleAction** | Time-based triggers | Cron-like scheduling | [[how-to/ScheduleAction]] |

See [[Action Types]] for detailed comparison and [[Action Catalog]] for built-in actions.

## Quick Start

### 1. Add Dependency

```toml
[dependencies]
nebula-action = "0.1"
serde = { version = "1.0", features = ["derive"] }
```

### 2. Create Your First Action

```rust
use nebula_action::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct GreetInput {
    name: String,
}

#[derive(Serialize)]
struct GreetOutput {
    message: String,
}

simple_action!(
    GreetAction,
    "greet",
    GreetInput,
    GreetOutput,
    |_action, input, context| async move {
        context.log_info(&format!("Greeting {}", input.name));

        Ok(GreetOutput {
            message: format!("Hello, {}!", input.name),
        })
    }
);
```

### 3. Test Your Action

```rust
#[tokio::test]
async fn test_greet_action() {
    let context = TestContext::default();
    let input = GreetInput { name: "Alice".into() };

    let action = GreetAction;
    let result = action.execute(input, &context).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap().message, "Hello, Alice!");
}
```

### 4. Use in Workflow

```rust
workflow! {
    name: "greeting_workflow",
    nodes: [
        node!(id: "greet", action: GreetAction),
    ]
}
```

## Core Concepts

### Action Lifecycle

Every action goes through a standard lifecycle:

1. **Registration** — Action is registered in the action registry
2. **Initialization** — Action instance is created
3. **Configuration** — Parameters are validated and applied
4. **Execution** — Action logic runs with input and context
5. **Result** — Output or error is returned
6. **Cleanup** — Resources are released

See [[Action Lifecycle]] for details.

### Context & Observability

Actions receive a `Context` that provides:

- **Logging** — `context.log_info()`, `context.log_error()`
- **Metrics** — `context.record_metric()`
- **Tracing** — Automatic distributed tracing
- **Credentials** — `context.get_credential()`
- **Memory** — `context.memory().get()`, `context.memory().set()`
- **Events** — `context.publish_event()`

All instrumentation is automatic — no manual setup required.

### Error Handling

Actions use a rich error type system:

```rust
async fn execute(&self, input: Input, context: &Context) -> Result<Output, ActionError> {
    // Business logic errors
    if input.value < 0 {
        return Err(ActionError::validation("Value must be positive"));
    }

    // External errors (automatically retried)
    let data = fetch_data(&input.url)
        .await
        .map_err(|e| ActionError::transient(e))?;

    // Unrecoverable errors
    if data.is_empty() {
        return Err(ActionError::permanent("No data received"));
    }

    Ok(Output { data })
}
```

See [[Error Model]] for complete error types.

## Documentation Structure

- **[[Action Types]]** — Comparison of all action types
- **[[Action Lifecycle]]** — How actions are initialized and executed
- **[[Action Result System]]** — Output types and error handling
- **[[Action Catalog]]** — Built-in actions reference
- **[[Custom Actions]]** — Building custom action types
- **[[Development Approaches]]** — Patterns and best practices
- **[[Examples]]** — Complete working examples
- **how-to/** — Step-by-step guides for each action type

## Common Patterns

### HTTP API Call (ProcessAction)

```rust
simple_action!(
    FetchUserAction,
    "user.fetch",
    FetchUserInput,
    FetchUserOutput,
    |_action, input, context| async move {
        let client = reqwest::Client::new();
        let response = client
            .get(&format!("https://api.example.com/users/{}", input.user_id))
            .send()
            .await
            .map_err(|e| ActionError::transient(e))?;

        let user: User = response.json().await?;
        Ok(FetchUserOutput { user })
    }
);
```

### Rate Limiting (StatefulAction)

```rust
impl StatefulAction for RateLimitAction {
    async fn execute_stateful(&self, input: Input, context: &Context, state: &mut State) -> Result<Output> {
        if state.requests_in_window >= self.max_requests {
            return Err(ActionError::rate_limited("Too many requests"));
        }

        state.requests_in_window += 1;
        // Process request...
    }
}
```

### Webhook Trigger (TriggerAction)

```rust
impl TriggerAction for WebhookTrigger {
    async fn poll(&self, context: &Context) -> Result<Vec<Event>> {
        // Listen for webhook events
        let events = self.receiver.recv_batch().await?;
        Ok(events)
    }
}
```

See [[Examples]] for 20+ complete action implementations.

## Related Crates

- **[[02-Crates/nebula-parameter/README|nebula-parameter]]** — Parameter validation
- **[[02-Crates/nebula-credential/README|nebula-credential]]** — Credential management
- **[[02-Crates/nebula-resource/README|nebula-resource]]** — Resource pooling
- **[[02-Crates/nebula-derive/README|nebula-derive]]** — Derive macros

## Getting Help

- **Concepts**: Read [[03-Concepts/Actions|Actions concept]] for mental models
- **How-to**: Follow [[04-Development/Creating Actions]] for step-by-step guidance
- **Examples**: Browse [[Examples]] for real-world patterns
- **API**: See [[Action Types]] for trait definitions

---

**Next**: Choose an action type from [[Action Types]] or start with [[how-to/ProcessAction]].

## Deep Dive: Action Traits

### The Action Trait

The core `Action` trait defines the contract for all actions:

```rust
#[async_trait]
pub trait Action: Send + Sync + 'static {
    type Input: DeserializeOwned + Send + 'static;
    type Output: Serialize + Send + 'static;
    type State: Default + Clone + Send + Sync + 'static = ();

    /// Execute the action with the given input and context
    async fn execute(
        &self,
        input: Self::Input,
        context: &Context,
    ) -> Result<Self::Output, ActionError>;

    /// Action metadata (name, version, description)
    fn metadata(&self) -> ActionMetadata {
        ActionMetadata::default()
    }

    /// Validate input before execution
    fn validate_input(&self, input: &Self::Input) -> Result<(), ValidationError> {
        Ok(())
    }

    /// Called once when action is registered
    async fn initialize(&mut self, config: &ActionConfig) -> Result<(), ActionError> {
        Ok(())
    }

    /// Called when action is removed from registry
    async fn shutdown(&mut self) -> Result<(), ActionError> {
        Ok(())
    }

    /// Returns retry configuration for this action
    fn retry_config(&self) -> Option<RetryConfig> {
        Some(RetryConfig::default())
    }

    /// Returns timeout configuration for this action
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(300))
    }
}
```

### ProcessAction Trait

The most common action type for stateless processing:

```rust
#[async_trait]
pub trait ProcessAction: Send + Sync + 'static {
    type Input: DeserializeOwned + Send + 'static;
    type Output: Serialize + Send + 'static;

    async fn process(
        &self,
        input: Self::Input,
        context: &Context,
    ) -> Result<Self::Output, ActionError>;
}

// Auto-implement Action for all ProcessAction
impl<T: ProcessAction> Action for T {
    type Input = T::Input;
    type Output = T::Output;

    async fn execute(&self, input: Self::Input, context: &Context)
        -> Result<Self::Output, ActionError>
    {
        self.process(input, context).await
    }
}
```

**Example: Image Transformation**

```rust
use image::{ImageFormat, DynamicImage};
use nebula_action::prelude::*;

#[derive(Deserialize)]
pub struct ResizeImageInput {
    pub image_url: String,
    pub width: u32,
    pub height: u32,
    pub format: ImageFormat,
}

#[derive(Serialize)]
pub struct ResizeImageOutput {
    pub output_url: String,
    pub size_bytes: u64,
    pub dimensions: (u32, u32),
}

pub struct ResizeImageAction {
    storage: Arc<BlobStorage>,
}

#[async_trait]
impl ProcessAction for ResizeImageAction {
    type Input = ResizeImageInput;
    type Output = ResizeImageOutput;

    async fn process(&self, input: Self::Input, context: &Context)
        -> Result<Self::Output, ActionError>
    {
        context.log_info(&format!("Resizing image from {}", input.image_url));

        // Download image
        let image_bytes = reqwest::get(&input.image_url)
            .await
            .map_err(|e| ActionError::transient(format!("Failed to download: {}", e)))?
            .bytes()
            .await
            .map_err(|e| ActionError::transient(e))?;

        // Decode image
        let img = image::load_from_memory(&image_bytes)
            .map_err(|e| ActionError::validation(format!("Invalid image: {}", e)))?;

        // Resize
        let resized = img.resize_exact(
            input.width,
            input.height,
            image::imageops::FilterType::Lanczos3,
        );

        // Encode to target format
        let mut output_bytes = Vec::new();
        resized.write_to(&mut std::io::Cursor::new(&mut output_bytes), input.format)
            .map_err(|e| ActionError::permanent(format!("Encoding failed: {}", e)))?;

        // Upload to storage
        let output_url = self.storage
            .upload(&output_bytes, &format!("resized-{}.{:?}", Uuid::new_v4(), input.format))
            .await
            .map_err(|e| ActionError::transient(e))?;

        context.record_metric("image.resized", 1.0);
        context.record_metric("image.size_bytes", output_bytes.len() as f64);

        Ok(ResizeImageOutput {
            output_url,
            size_bytes: output_bytes.len() as u64,
            dimensions: (input.width, input.height),
        })
    }
}

impl Action for ResizeImageAction {
    type Input = ResizeImageInput;
    type Output = ResizeImageOutput;

    async fn execute(&self, input: Self::Input, context: &Context)
        -> Result<Self::Output, ActionError>
    {
        self.process(input, context).await
    }

    fn validate_input(&self, input: &Self::Input) -> Result<(), ValidationError> {
        if input.width == 0 || input.height == 0 {
            return Err(ValidationError::new("Width and height must be positive"));
        }

        if input.width > 10000 || input.height > 10000 {
            return Err(ValidationError::new("Dimensions too large (max 10000x10000)"));
        }

        Ok(())
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(60))
    }

    fn retry_config(&self) -> Option<RetryConfig> {
        Some(RetryConfig {
            max_attempts: 3,
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(10),
            backoff_factor: 2.0,
            retry_on: vec![
                ErrorKind::Transient,
                ErrorKind::NetworkError,
            ],
        })
    }
}
```

### StatefulAction Trait

For actions that maintain state across executions:

```rust
#[async_trait]
pub trait StatefulAction: Send + Sync + 'static {
    type Input: DeserializeOwned + Send + 'static;
    type Output: Serialize + Send + 'static;
    type State: Default + Clone + Serialize + DeserializeOwned + Send + Sync + 'static;

    async fn execute_stateful(
        &self,
        input: Self::Input,
        context: &Context,
        state: &mut Self::State,
    ) -> Result<Self::Output, ActionError>;

    /// Called when state needs to be persisted
    async fn persist_state(&self, state: &Self::State, context: &Context)
        -> Result<(), ActionError>
    {
        Ok(())
    }

    /// Called to restore state from storage
    async fn restore_state(&self, context: &Context) -> Result<Option<Self::State>, ActionError> {
        Ok(None)
    }
}
```

**Example: Token Bucket Rate Limiter**

```rust
use std::time::{Duration, Instant};
use nebula_action::prelude::*;

#[derive(Clone, Serialize, Deserialize)]
pub struct TokenBucketState {
    tokens: f64,
    last_refill: Instant,
}

impl Default for TokenBucketState {
    fn default() -> Self {
        Self {
            tokens: 100.0,
            last_refill: Instant::now(),
        }
    }
}

pub struct RateLimitAction {
    capacity: f64,
    refill_rate: f64,  // tokens per second
}

#[async_trait]
impl StatefulAction for RateLimitAction {
    type Input = serde_json::Value;  // Pass-through
    type Output = serde_json::Value;
    type State = TokenBucketState;

    async fn execute_stateful(
        &self,
        input: Self::Input,
        context: &Context,
        state: &mut Self::State,
    ) -> Result<Self::Output, ActionError> {
        // Refill tokens based on elapsed time
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        let tokens_to_add = elapsed * self.refill_rate;

        state.tokens = (state.tokens + tokens_to_add).min(self.capacity);
        state.last_refill = now;

        // Check if we have enough tokens
        if state.tokens < 1.0 {
            let wait_time = (1.0 - state.tokens) / self.refill_rate;
            context.log_warn(&format!("Rate limit exceeded, need to wait {:.2}s", wait_time));

            return Err(ActionError::rate_limited(
                format!("Rate limit exceeded. Retry after {:.2}s", wait_time)
            ));
        }

        // Consume one token
        state.tokens -= 1.0;

        context.log_info(&format!("Tokens remaining: {:.2}/{}", state.tokens, self.capacity));
        context.record_metric("rate_limit.tokens_remaining", state.tokens);

        // Pass through input as output
        Ok(input)
    }

    async fn persist_state(&self, state: &Self::State, context: &Context)
        -> Result<(), ActionError>
    {
        context.memory().set("rate_limit_state", state).await?;
        Ok(())
    }

    async fn restore_state(&self, context: &Context)
        -> Result<Option<Self::State>, ActionError>
    {
        let state = context.memory().get("rate_limit_state").await?;
        Ok(state)
    }
}

impl RateLimitAction {
    pub fn new(capacity: f64, refill_rate: f64) -> Self {
        Self { capacity, refill_rate }
    }

    /// Create a rate limiter: 100 requests per minute
    pub fn per_minute(max_requests: u32) -> Self {
        Self::new(max_requests as f64, max_requests as f64 / 60.0)
    }

    /// Create a rate limiter: 1000 requests per hour
    pub fn per_hour(max_requests: u32) -> Self {
        Self::new(max_requests as f64, max_requests as f64 / 3600.0)
    }
}
```

### TriggerAction Trait

For actions that initiate workflows based on external events:

```rust
#[async_trait]
pub trait TriggerAction: Send + Sync + 'static {
    type Event: Serialize + Send + 'static;

    /// Poll for new events that should trigger workflow execution
    async fn poll(&self, context: &Context) -> Result<Vec<Self::Event>, ActionError>;

    /// Return the polling interval
    fn poll_interval(&self) -> Duration {
        Duration::from_secs(60)
    }

    /// Called when trigger is registered
    async fn start(&mut self, context: &Context) -> Result<(), ActionError> {
        Ok(())
    }

    /// Called when trigger is unregistered
    async fn stop(&mut self) -> Result<(), ActionError> {
        Ok(())
    }
}
```

**Example: GitHub Webhook Trigger**

```rust
use tokio::sync::mpsc;
use warp::Filter;
use nebula_action::prelude::*;

#[derive(Serialize, Deserialize, Clone)]
pub struct GitHubWebhookEvent {
    pub event_type: String,
    pub repository: String,
    pub sender: String,
    pub payload: serde_json::Value,
    pub received_at: DateTime<Utc>,
}

pub struct GitHubWebhookTrigger {
    port: u16,
    secret: String,
    receiver: Arc<tokio::sync::Mutex<mpsc::Receiver<GitHubWebhookEvent>>>,
    sender: mpsc::Sender<GitHubWebhookEvent>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl GitHubWebhookTrigger {
    pub fn new(port: u16, secret: String) -> Self {
        let (tx, rx) = mpsc::channel(100);

        Self {
            port,
            secret,
            sender: tx,
            receiver: Arc::new(tokio::sync::Mutex::new(rx)),
            shutdown_tx: None,
        }
    }

    fn verify_signature(&self, payload: &[u8], signature: &str) -> bool {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let mut mac = Hmac::<Sha256>::new_from_slice(self.secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(payload);

        let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        signature == expected
    }
}

#[async_trait]
impl TriggerAction for GitHubWebhookTrigger {
    type Event = GitHubWebhookEvent;

    async fn start(&mut self, context: &Context) -> Result<(), ActionError> {
        let sender = self.sender.clone();
        let secret = self.secret.clone();
        let port = self.port;

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        self.shutdown_tx = Some(shutdown_tx);

        // Start webhook server
        let webhook_route = warp::post()
            .and(warp::path("webhook"))
            .and(warp::header::<String>("x-hub-signature-256"))
            .and(warp::header::<String>("x-github-event"))
            .and(warp::body::bytes())
            .and_then(move |signature: String, event_type: String, body: bytes::Bytes| {
                let sender = sender.clone();
                let secret = secret.clone();

                async move {
                    // Verify signature
                    if !Self::verify_signature_static(&secret, &body, &signature) {
                        return Err(warp::reject::custom(UnauthorizedError));
                    }

                    // Parse payload
                    let payload: serde_json::Value = serde_json::from_slice(&body)
                        .map_err(|_| warp::reject::custom(InvalidPayloadError))?;

                    let repository = payload["repository"]["full_name"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string();

                    let sender_name = payload["sender"]["login"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string();

                    // Send event to channel
                    let event = GitHubWebhookEvent {
                        event_type,
                        repository,
                        sender: sender_name,
                        payload,
                        received_at: Utc::now(),
                    };

                    sender.send(event).await
                        .map_err(|_| warp::reject::custom(ChannelError))?;

                    Ok::<_, warp::Rejection>(warp::reply::json(&json!({
                        "status": "received"
                    })))
                }
            });

        // Spawn server
        tokio::spawn(async move {
            let (_, server) = warp::serve(webhook_route)
                .bind_with_graceful_shutdown(
                    ([0, 0, 0, 0], port),
                    async {
                        shutdown_rx.await.ok();
                    }
                );
            server.await;
        });

        context.log_info(&format!("GitHub webhook server started on port {}", port));

        Ok(())
    }

    async fn poll(&self, context: &Context) -> Result<Vec<Self::Event>, ActionError> {
        let mut events = Vec::new();
        let mut receiver = self.receiver.lock().await;

        // Collect all pending events (non-blocking)
        while let Ok(event) = receiver.try_recv() {
            context.log_info(&format!(
                "Received {} event from {}",
                event.event_type, event.repository
            ));
            events.push(event);
        }

        Ok(events)
    }

    fn poll_interval(&self) -> Duration {
        Duration::from_millis(100)  // Check frequently for webhooks
    }

    async fn stop(&mut self) -> Result<(), ActionError> {
        if let Some(tx) = self.shutdown_tx.take() {
            tx.send(()).ok();
        }
        Ok(())
    }
}

// Helper for signature verification
impl GitHubWebhookTrigger {
    fn verify_signature_static(secret: &str, payload: &[u8], signature: &str) -> bool {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
            .expect("HMAC can take key of any size");
        mac.update(payload);

        let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        signature == expected
    }
}
```

### StreamingAction Trait

For long-lived connections or continuous data streams:

```rust
#[async_trait]
pub trait StreamingAction: Send + Sync + 'static {
    type Input: DeserializeOwned + Send + 'static;
    type Item: Serialize + Send + 'static;

    async fn stream(
        &self,
        input: Self::Input,
        context: &Context,
    ) -> Result<BoxStream<'static, Result<Self::Item, ActionError>>, ActionError>;
}

// BoxStream is a type alias for pinned boxed stream
pub type BoxStream<'a, T> = Pin<Box<dyn Stream<Item = T> + Send + 'a>>;
```

**Example: Database Query Stream**

```rust
use futures::stream::{Stream, StreamExt};
use sqlx::{PgPool, Row};
use nebula_action::prelude::*;

#[derive(Deserialize)]
pub struct QueryUsersInput {
    pub min_age: i32,
    pub limit: i64,
}

#[derive(Serialize)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub age: i32,
}

pub struct StreamUsersAction {
    pool: PgPool,
}

#[async_trait]
impl StreamingAction for StreamUsersAction {
    type Input = QueryUsersInput;
    type Item = User;

    async fn stream(
        &self,
        input: Self::Input,
        context: &Context,
    ) -> Result<BoxStream<'static, Result<Self::Item, ActionError>>, ActionError> {
        context.log_info(&format!("Streaming users with age >= {}", input.min_age));

        // Create cursor-based query
        let mut rows = sqlx::query(
            "SELECT id, name, email, age FROM users WHERE age >= $1 LIMIT $2"
        )
        .bind(input.min_age)
        .bind(input.limit)
        .fetch(&self.pool);

        // Convert to stream of Users
        let stream = async_stream::stream! {
            let mut count = 0;

            while let Some(row_result) = rows.next().await {
                match row_result {
                    Ok(row) => {
                        let user = User {
                            id: row.get("id"),
                            name: row.get("name"),
                            email: row.get("email"),
                            age: row.get("age"),
                        };

                        count += 1;
                        context.record_metric("users.streamed", 1.0);

                        yield Ok(user);
                    }
                    Err(e) => {
                        context.log_error(&format!("Database error: {}", e));
                        yield Err(ActionError::transient(e));
                        break;
                    }
                }
            }

            context.log_info(&format!("Streamed {} users", count));
        };

        Ok(Box::pin(stream))
    }
}
```

### TransactionalAction Trait

For actions that support rollback/compensation:

```rust
#[async_trait]
pub trait TransactionalAction: Send + Sync + 'static {
    type Input: DeserializeOwned + Send + 'static;
    type Output: Serialize + Send + 'static;
    type CompensationData: Serialize + DeserializeOwned + Send + 'static;

    /// Execute the action and return compensation data
    async fn execute_tx(
        &self,
        input: Self::Input,
        context: &Context,
    ) -> Result<(Self::Output, Self::CompensationData), ActionError>;

    /// Compensate/undo the action using compensation data
    async fn compensate(
        &self,
        compensation_data: Self::CompensationData,
        context: &Context,
    ) -> Result<(), ActionError>;
}
```

**Example: Saga Pattern Order Processing**

```rust
use nebula_action::prelude::*;

#[derive(Serialize, Deserialize)]
pub struct ReserveInventoryCompensation {
    pub order_id: String,
    pub items: Vec<(String, u32)>,  // (product_id, quantity)
}

pub struct ReserveInventoryAction {
    inventory_service: Arc<InventoryService>,
}

#[async_trait]
impl TransactionalAction for ReserveInventoryAction {
    type Input = OrderInput;
    type Output = ReservationOutput;
    type CompensationData = ReserveInventoryCompensation;

    async fn execute_tx(
        &self,
        input: Self::Input,
        context: &Context,
    ) -> Result<(Self::Output, Self::CompensationData), ActionError> {
        context.log_info(&format!("Reserving inventory for order {}", input.order_id));

        let mut reserved_items = Vec::new();

        for item in &input.items {
            self.inventory_service
                .reserve(&item.product_id, item.quantity)
                .await
                .map_err(|e| ActionError::permanent(format!(
                    "Failed to reserve {} units of {}: {}",
                    item.quantity, item.product_id, e
                )))?;

            reserved_items.push((item.product_id.clone(), item.quantity));
        }

        let compensation = ReserveInventoryCompensation {
            order_id: input.order_id.clone(),
            items: reserved_items,
        };

        let output = ReservationOutput {
            reservation_id: format!("rsv_{}", Uuid::new_v4()),
            reserved_at: Utc::now(),
        };

        context.record_metric("inventory.reserved", input.items.len() as f64);

        Ok((output, compensation))
    }

    async fn compensate(
        &self,
        compensation_data: Self::CompensationData,
        context: &Context,
    ) -> Result<(), ActionError> {
        context.log_warn(&format!(
            "Compensating inventory reservation for order {}",
            compensation_data.order_id
        ));

        for (product_id, quantity) in compensation_data.items {
            self.inventory_service
                .release(&product_id, quantity)
                .await
                .map_err(|e| ActionError::permanent(format!(
                    "Compensation failed for {}: {}",
                    product_id, e
                )))?;

            context.log_info(&format!("Released {} units of {}", quantity, product_id));
        }

        context.record_metric("inventory.compensation.success", 1.0);

        Ok(())
    }
}

// Complete Saga workflow
pub async fn build_order_saga() -> Result<Workflow, WorkflowError> {
    WorkflowBuilder::new("order_processing_saga")
        .add_transactional_node("reserve_inventory", ReserveInventoryAction::new())
        .add_transactional_node("charge_payment", ChargePaymentAction::new())
        .add_transactional_node("create_shipment", CreateShipmentAction::new())
        .add_edge("reserve_inventory", "charge_payment", |o| o)
        .add_edge("charge_payment", "create_shipment", |o| o)
        // If any step fails, compensate previous steps in reverse order
        .with_saga_compensation()
        .build()
}
```

## Advanced Patterns

### Batch Processing with Backpressure

Process items in batches with controlled concurrency:

```rust
use futures::stream::{self, StreamExt};
use nebula_action::prelude::*;

pub struct BatchProcessAction {
    batch_size: usize,
    max_concurrency: usize,
}

#[async_trait]
impl ProcessAction for BatchProcessAction {
    type Input = BatchInput;
    type Output = BatchOutput;

    async fn process(&self, input: Self::Input, context: &Context)
        -> Result<Self::Output, ActionError>
    {
        context.log_info(&format!("Processing {} items in batches of {}",
            input.items.len(), self.batch_size));

        let mut results = Vec::new();
        let mut errors = Vec::new();

        // Process in batches with controlled concurrency
        for batch in input.items.chunks(self.batch_size) {
            let batch_results: Vec<_> = stream::iter(batch)
                .map(|item| async move {
                    self.process_single_item(item, context).await
                })
                .buffer_unordered(self.max_concurrency)
                .collect()
                .await;

            for result in batch_results {
                match result {
                    Ok(output) => results.push(output),
                    Err(e) => errors.push(e),
                }
            }

            // Yield to allow other tasks to run
            tokio::task::yield_now().await;
        }

        if !errors.is_empty() {
            context.log_warn(&format!("Batch processing completed with {} errors", errors.len()));
        }

        context.record_metric("batch.processed", results.len() as f64);
        context.record_metric("batch.errors", errors.len() as f64);

        Ok(BatchOutput {
            successful: results,
            failed: errors.len(),
        })
    }
}
```

### Circuit Breaker Pattern

Prevent cascading failures by detecting and stopping repeated failures:

```rust
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct CircuitBreakerAction<A: Action> {
    inner: A,
    failure_threshold: u32,
    success_threshold: u32,
    timeout: Duration,
    state: Arc<CircuitBreakerState>,
}

struct CircuitBreakerState {
    state: AtomicU32,  // 0 = Closed, 1 = Open, 2 = HalfOpen
    failure_count: AtomicU32,
    success_count: AtomicU32,
    last_failure_time: AtomicU64,
}

impl CircuitBreakerState {
    fn is_open(&self) -> bool {
        self.state.load(Ordering::Relaxed) == 1
    }

    fn is_half_open(&self) -> bool {
        self.state.load(Ordering::Relaxed) == 2
    }

    fn open(&self) {
        self.state.store(1, Ordering::Relaxed);
        self.last_failure_time.store(
            Instant::now().elapsed().as_secs(),
            Ordering::Relaxed
        );
    }

    fn half_open(&self) {
        self.state.store(2, Ordering::Relaxed);
        self.success_count.store(0, Ordering::Relaxed);
    }

    fn close(&self) {
        self.state.store(0, Ordering::Relaxed);
        self.failure_count.store(0, Ordering::Relaxed);
    }

    fn record_success(&self) -> u32 {
        self.failure_count.store(0, Ordering::Relaxed);
        self.success_count.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn record_failure(&self) -> u32 {
        self.success_count.store(0, Ordering::Relaxed);
        self.failure_count.fetch_add(1, Ordering::Relaxed) + 1
    }
}

#[async_trait]
impl<A: Action> Action for CircuitBreakerAction<A> {
    type Input = A::Input;
    type Output = A::Output;

    async fn execute(&self, input: Self::Input, context: &Context)
        -> Result<Self::Output, ActionError>
    {
        // Check if circuit is open
        if self.state.is_open() {
            let elapsed = Duration::from_secs(
                Instant::now().elapsed().as_secs() -
                self.state.last_failure_time.load(Ordering::Relaxed)
            );

            if elapsed < self.timeout {
                context.log_warn("Circuit breaker is OPEN, rejecting request");
                return Err(ActionError::circuit_breaker_open(
                    format!("Circuit breaker open, retry after {:?}", self.timeout - elapsed)
                ));
            }

            // Timeout expired, try half-open
            self.state.half_open();
            context.log_info("Circuit breaker transitioning to HALF_OPEN");
        }

        // Execute inner action
        match self.inner.execute(input, context).await {
            Ok(output) => {
                let success_count = self.state.record_success();

                // If half-open and enough successes, close the circuit
                if self.state.is_half_open() && success_count >= self.success_threshold {
                    self.state.close();
                    context.log_info("Circuit breaker CLOSED after successful recovery");
                }

                context.record_metric("circuit_breaker.success", 1.0);
                Ok(output)
            }
            Err(e) => {
                let failure_count = self.state.record_failure();

                // Open circuit if threshold exceeded
                if failure_count >= self.failure_threshold {
                    self.state.open();
                    context.log_error(&format!(
                        "Circuit breaker OPENED after {} failures",
                        failure_count
                    ));
                    context.record_metric("circuit_breaker.opened", 1.0);
                }

                context.record_metric("circuit_breaker.failure", 1.0);
                Err(e)
            }
        }
    }
}

impl<A: Action> CircuitBreakerAction<A> {
    pub fn new(inner: A) -> Self {
        Self {
            inner,
            failure_threshold: 5,
            success_threshold: 2,
            timeout: Duration::from_secs(60),
            state: Arc::new(CircuitBreakerState {
                state: AtomicU32::new(0),
                failure_count: AtomicU32::new(0),
                success_count: AtomicU32::new(0),
                last_failure_time: AtomicU64::new(0),
            }),
        }
    }

    pub fn with_failure_threshold(mut self, threshold: u32) -> Self {
        self.failure_threshold = threshold;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}
```

### Caching Action Results

Memoize expensive action results:

```rust
use lru::LruCache;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;

pub struct CachedAction<A: Action> {
    inner: A,
    cache: Arc<tokio::sync::Mutex<LruCache<u64, A::Output>>>,
    ttl: Duration,
}

impl<A: Action> CachedAction<A>
where
    A::Input: Hash,
    A::Output: Clone,
{
    pub fn new(inner: A, capacity: usize, ttl: Duration) -> Self {
        Self {
            inner,
            cache: Arc::new(tokio::sync::Mutex::new(LruCache::new(capacity))),
            ttl,
        }
    }

    fn hash_input(input: &A::Input) -> u64 {
        let mut hasher = DefaultHasher::new();
        input.hash(&mut hasher);
        hasher.finish()
    }
}

#[async_trait]
impl<A: Action> Action for CachedAction<A>
where
    A::Input: Hash,
    A::Output: Clone,
{
    type Input = A::Input;
    type Output = A::Output;

    async fn execute(&self, input: Self::Input, context: &Context)
        -> Result<Self::Output, ActionError>
    {
        let key = Self::hash_input(&input);

        // Check cache
        {
            let mut cache = self.cache.lock().await;
            if let Some(cached) = cache.get(&key) {
                context.log_info("Cache HIT");
                context.record_metric("cache.hit", 1.0);
                return Ok(cached.clone());
            }
        }

        context.log_info("Cache MISS");
        context.record_metric("cache.miss", 1.0);

        // Execute action
        let result = self.inner.execute(input, context).await?;

        // Store in cache
        {
            let mut cache = self.cache.lock().await;
            cache.put(key, result.clone());
        }

        Ok(result)
    }
}
```

## Testing Actions

### Unit Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use nebula_action::testing::*;

    #[tokio::test]
    async fn test_resize_image_success() {
        let storage = Arc::new(InMemoryBlobStorage::new());
        let action = ResizeImageAction { storage: storage.clone() };

        let context = TestContext::builder()
            .with_log_level(LogLevel::Debug)
            .build();

        let input = ResizeImageInput {
            image_url: "https://example.com/image.jpg".to_string(),
            width: 800,
            height: 600,
            format: ImageFormat::Png,
        };

        let result = action.process(input, &context).await;

        assert!(result.is_ok());
        let output = result.unwrap();
        assert_eq!(output.dimensions, (800, 600));
        assert!(output.size_bytes > 0);

        // Verify metrics were recorded
        assert_eq!(context.get_metric_count("image.resized"), 1);
    }

    #[tokio::test]
    async fn test_resize_image_invalid_dimensions() {
        let storage = Arc::new(InMemoryBlobStorage::new());
        let action = ResizeImageAction { storage };

        let context = TestContext::default();

        let input = ResizeImageInput {
            image_url: "https://example.com/image.jpg".to_string(),
            width: 0,  // Invalid!
            height: 600,
            format: ImageFormat::Png,
        };

        let result = action.validate_input(&input);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens_after_failures() {
        let failing_action = AlwaysFailAction;
        let circuit_breaker = CircuitBreakerAction::new(failing_action)
            .with_failure_threshold(3);

        let context = TestContext::default();
        let input = ();

        // First 3 attempts should fail normally
        for i in 0..3 {
            let result = circuit_breaker.execute(input, &context).await;
            assert!(result.is_err());
            assert!(!matches!(result.unwrap_err(), ActionError::CircuitBreakerOpen(_)));
        }

        // 4th attempt should fail with CircuitBreakerOpen
        let result = circuit_breaker.execute(input, &context).await;
        assert!(matches!(result.unwrap_err(), ActionError::CircuitBreakerOpen(_)));
    }
}
```

### Integration Testing

```rust
#[tokio::test]
async fn test_action_in_workflow() {
    let workflow = WorkflowBuilder::new("test_workflow")
        .add_node("resize", ResizeImageAction::new())
        .add_node("upload", UploadToS3Action::new())
        .add_edge("resize", "upload", |output: ResizeImageOutput| {
            UploadToS3Input {
                file_path: output.output_url,
                bucket: "processed-images".to_string(),
            }
        })
        .build()
        .unwrap();

    let context = ExecutionContext::new();
    let trigger_data = json!({
        "image_url": "https://example.com/test.jpg",
        "width": 800,
        "height": 600,
        "format": "Png"
    });

    let result = workflow.execute(trigger_data, context).await;
    assert!(result.is_ok());
}
```

### Property-Based Testing

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_rate_limiter_never_exceeds_capacity(
        requests in 1..1000usize,
        capacity in 10..100f64,
        refill_rate in 1.0..50.0f64,
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let action = RateLimitAction::new(capacity, refill_rate);
            let context = TestContext::default();

            let mut state = TokenBucketState::default();
            state.tokens = capacity;

            let mut successful = 0;

            for _ in 0..requests {
                let result = action.execute_stateful(
                    json!({}),
                    &context,
                    &mut state
                ).await;

                if result.is_ok() {
                    successful += 1;
                }

                // State invariant: tokens never exceed capacity
                prop_assert!(state.tokens <= capacity);
            }

            // At least some requests should succeed
            prop_assert!(successful > 0);
        });
    }
}
```

## Performance Optimization

### Async Task Pooling

Reuse async tasks to reduce allocation overhead:

```rust
use tokio::task::JoinSet;

pub struct PooledAction<A: Action> {
    inner: A,
    pool_size: usize,
}

impl<A: Action + Clone> PooledAction<A> {
    pub async fn execute_many(
        &self,
        inputs: Vec<A::Input>,
        context: &Context,
    ) -> Vec<Result<A::Output, ActionError>> {
        let mut set = JoinSet::new();

        for input in inputs {
            let action = self.inner.clone();
            let ctx = context.clone();

            set.spawn(async move {
                action.execute(input, &ctx).await
            });
        }

        let mut results = Vec::new();
        while let Some(result) = set.join_next().await {
            results.push(result.unwrap());
        }

        results
    }
}
```

### Zero-Copy Deserialization

Use `serde_json::from_slice` for zero-copy parsing:

```rust
impl Action for OptimizedAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    async fn execute(&self, input: Self::Input, context: &Context)
        -> Result<Self::Output, ActionError>
    {
        // Serialize input to bytes once
        let input_bytes = serde_json::to_vec(&input)?;

        // Use zero-copy deserialization for specific fields
        let user_id: &str = serde_json::from_slice(&input_bytes)?;

        // Process...

        Ok(json!({ "status": "success" }))
    }
}
```

### Memory Pooling

Reuse allocations across action executions:

```rust
use bytes::BytesMut;

pub struct BufferPooledAction {
    buffer_pool: Arc<tokio::sync::Mutex<Vec<BytesMut>>>,
}

impl BufferPooledAction {
    async fn get_buffer(&self) -> BytesMut {
        let mut pool = self.buffer_pool.lock().await;
        pool.pop().unwrap_or_else(|| BytesMut::with_capacity(8192))
    }

    async fn return_buffer(&self, mut buffer: BytesMut) {
        buffer.clear();
        let mut pool = self.buffer_pool.lock().await;
        if pool.len() < 100 {  // Max pool size
            pool.push(buffer);
        }
    }
}
```

## Migration Guide

### Upgrading from 0.1 to 0.2

**Breaking changes:**

1. **Context API**: `context.log()` → `context.log_info()`

```rust
// Before (0.1)
context.log("Processing request");

// After (0.2)
context.log_info("Processing request");
```

2. **Error types**: `ActionError::new()` → `ActionError::permanent()`

```rust
// Before (0.1)
return Err(ActionError::new("Failed"));

// After (0.2)
return Err(ActionError::permanent("Failed"));
```

3. **Retry configuration**: Now part of `Action` trait

```rust
// Before (0.1)
let config = ActionConfig::new().with_retry(3);

// After (0.2)
impl Action for MyAction {
    fn retry_config(&self) -> Option<RetryConfig> {
        Some(RetryConfig::default().with_max_attempts(3))
    }
}
```

### Converting from webhook to TriggerAction

```rust
// Before: Manual webhook handling
async fn handle_webhook(payload: Payload) {
    // Process webhook
}

// After: TriggerAction
impl TriggerAction for MyWebhookTrigger {
    type Event = Payload;

    async fn poll(&self, context: &Context) -> Result<Vec<Self::Event>, ActionError> {
        // Nebula handles polling automatically
        Ok(self.get_pending_webhooks().await?)
    }
}
```

## Configuration Reference

### Action Configuration

```rust
pub struct ActionConfig {
    /// Action identifier
    pub id: String,

    /// Action version
    pub version: Version,

    /// Execution timeout
    pub timeout: Option<Duration>,

    /// Retry configuration
    pub retry: Option<RetryConfig>,

    /// Resource limits
    pub limits: ResourceLimits,

    /// Custom parameters
    pub parameters: HashMap<String, serde_json::Value>,
}

pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub backoff_factor: f64,
    pub retry_on: Vec<ErrorKind>,
}

pub struct ResourceLimits {
    pub max_memory: Option<ByteSize>,
    pub max_cpu_percent: Option<f32>,
    pub max_concurrent_executions: Option<u32>,
}
```

### Environment Variables

```bash
# Default timeout for all actions
NEBULA_ACTION_DEFAULT_TIMEOUT=300

# Enable detailed action logging
NEBULA_ACTION_LOG_LEVEL=debug

# Disable automatic retries
NEBULA_ACTION_DISABLE_RETRY=false

# Circuit breaker configuration
NEBULA_CIRCUIT_BREAKER_THRESHOLD=5
NEBULA_CIRCUIT_BREAKER_TIMEOUT=60
```

## Troubleshooting

### Common Issues

**1. Action timeout**

```
Error: Action exceeded timeout of 30s
```

**Solution**: Increase timeout or optimize action logic

```rust
impl Action for SlowAction {
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(300))  // 5 minutes
    }
}
```

**2. Serialization errors**

```
Error: Failed to serialize output: missing field 'required_field'
```

**Solution**: Ensure all output fields are present

```rust
#[derive(Serialize)]
pub struct Output {
    pub required_field: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optional_field: Option<String>,
}
```

**3. State persistence failures**

```
Error: Failed to persist state: storage unavailable
```

**Solution**: Implement fallback or handle gracefully

```rust
async fn persist_state(&self, state: &Self::State, context: &Context)
    -> Result<(), ActionError>
{
    if let Err(e) = context.memory().set("state", state).await {
        context.log_warn(&format!("State persistence failed: {}", e));
        // Continue anyway - state will be lost but action succeeds
    }
    Ok(())
}
```

## Best Practices Summary

1. **Keep actions small** - Single responsibility principle
2. **Use typed inputs/outputs** - Leverage Rust's type system
3. **Log meaningful events** - Aid debugging and monitoring
4. **Handle errors properly** - Distinguish transient from permanent
5. **Test thoroughly** - Unit tests, integration tests, property tests
6. **Document behavior** - Include examples in doc comments
7. **Version actions** - Use semantic versioning
8. **Monitor performance** - Record metrics for optimization
9. **Respect timeouts** - Don't block indefinitely
10. **Release resources** - Implement Drop when needed

---

**Next Steps**: Explore [[Action Types]] for detailed trait documentation or check [[Examples]] for more real-world patterns.
