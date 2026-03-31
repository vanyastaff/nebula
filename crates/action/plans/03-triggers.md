# TriggerAction — Complete Trigger Model

## Core Insight: Triggers USE Resources, Not Manage Connections

Resource-hld already solves connection management, reconnection, health monitoring,
graceful shutdown, and background loops through 7 topology types (Pool, Resident,
Service, Transport, Exclusive, **EventSource**, **Daemon**). Triggers should
**consume** managed resources, not reinvent their lifecycle.

```
Resource layer (nebula-resource):           Trigger layer (nebula-action):
  ✅ Connection create/destroy                ✅ Event filtering/transformation
  ✅ Reconnection (RecoveryGate)              ✅ Emit execution
  ✅ Health monitoring                        ✅ State/checkpoint
  ✅ Graceful shutdown                        ✅ Scheduling (cron, interval)
  ✅ Background loops (Daemon)                ✅ Webhook HTTP handling
  ✅ Event subscriptions (EventSource)
```

---

## TriggerContext

```rust
/// Context for TriggerAction. NOT Clone for ActionContext, but for TriggerContext
/// the run() method receives an **owned** context in a dedicated task.
///
/// Contains resource access (key addition in v5), parameter access, scheduling,
/// emission, checkpoint, and execution status capabilities.
pub struct TriggerContext {
    pub workflow_id: WorkflowId,
    pub trigger_id: NodeId,
    pub cancellation: CancellationToken,
    guard: ExecutionGuard,
    // Capabilities
    resources: Arc<dyn ResourceAccessor>,
    credentials: Arc<dyn CredentialAccessor>,
    scheduler: Arc<dyn TriggerScheduler>,
    emitter: Arc<dyn ExecutionEmitter>,
    checkpoint_sink: Arc<dyn TriggerCheckpointSink>,
    parameters: Arc<dyn ParameterProvider>,
    logger: Arc<dyn ActionLogger>,
}

impl TriggerContext {
    // ── Resource access ──

    /// Acquire typed managed handle/lease by local alias.
    /// Returns a lightweight handle, not the raw resource object.
    /// Resource layer manages lifecycle, reconnection, health.
    pub async fn resource_typed<R: Send + Sync + 'static>(
        &self, key: &str,
    ) -> Result<R, ActionError> {
        self.guard.check()?;
        let boxed = self.resources.acquire(key).await?;
        *boxed.downcast::<R>()
            .map_err(|_| ActionError::fatal(format!("resource '{}' type mismatch", key)))
    }

    // ── Credential access ──

    pub async fn credential_typed<S: 'static>(
        &self, key: &str,
    ) -> Result<S, ActionError> {
        self.guard.check()?;
        let snapshot = self.credentials.get(key).await?;
        snapshot.downcast::<S>()
            .map_err(|_| ActionError::fatal(format!("credential '{}' scheme mismatch", key)))
    }

    // ── Parameter access ──

    pub fn parameter(&self, key: &str) -> Result<serde_json::Value, ActionError> {
        self.parameters.get(key)
            .ok_or_else(|| ActionError::validation(format!("parameter '{}' not found", key)))
    }

    pub fn parameter_typed<T: DeserializeOwned>(&self, key: &str) -> Result<T, ActionError> {
        let val = self.parameter(key)?;
        serde_json::from_value(val)
            .map_err(|e| ActionError::fatal(format!("parameter '{}' deserialize: {}", key, e)))
    }

    // ── Scheduling ──

    pub async fn schedule_after(&self, delay: Duration) -> Result<(), ActionError> {
        self.guard.check()?;
        self.scheduler.schedule_after(delay).await
    }

    pub async fn schedule_at(
        &self, at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), ActionError> {
        self.guard.check()?;
        self.scheduler.schedule_at(at).await
    }

    pub async fn unschedule(&self) -> Result<(), ActionError> {
        self.guard.check()?;
        self.scheduler.unschedule().await
    }

    // ── Execution emission ──

    pub async fn emit_execution(
        &self, input: serde_json::Value,
    ) -> Result<ExecutionId, ActionError> {
        self.guard.check()?;
        self.emitter.emit(input).await
    }

    /// Atomic: emit execution AND persist state in one transaction.
    ///
    /// **Delivery semantics note:** This is atomic only for engine state +
    /// execution creation. Broker offset commit (Kafka, SQS) remains an
    /// external side effect AFTER this call. Default delivery semantics
    /// are at-least-once unless the concrete resource/runtime layer offers
    /// stronger integration (e.g., transactional outbox).
    pub async fn emit_and_checkpoint(
        &self,
        input: serde_json::Value,
        state: serde_json::Value,
    ) -> Result<ExecutionId, ActionError> {
        self.guard.check()?;
        self.emitter.emit_and_checkpoint(input, state).await
    }

    /// Batch emit for high-throughput triggers.
    pub async fn emit_batch(
        &self, inputs: Vec<serde_json::Value>,
    ) -> Result<Vec<ExecutionId>, ActionError> {
        self.guard.check()?;
        self.emitter.emit_batch(inputs).await
    }

    // ── State checkpoint ──

    pub async fn checkpoint(
        &self, state: serde_json::Value,
    ) -> Result<(), ActionError> {
        self.guard.check()?;
        self.checkpoint_sink.save(state).await
    }

    // ── Execution status (backpressure) ──

    pub async fn execution_status(
        &self, id: ExecutionId,
    ) -> Result<ExecutionStatus, ActionError> {
        self.guard.check()?;
        self.emitter.execution_status(id).await
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

pub enum ExecutionStatus {
    Running,
    Completed,
    Failed,
    Unknown,
}

/// Sink for persisting trigger state checkpoints.
#[async_trait]
pub trait TriggerCheckpointSink: Send + Sync {
    async fn save(&self, state: serde_json::Value) -> Result<(), ActionError>;
}

/// Parameter provider for trigger parameter access.
pub trait ParameterProvider: Send + Sync {
    fn get(&self, key: &str) -> Option<serde_json::Value>;
}
```

---

## TriggerAction Core Trait (start/run split)

```rust
pub trait TriggerAction: Action {
    type State: Serialize + DeserializeOwned + Send + Sync + 'static;

    /// Initialize trigger state (first start).
    /// **Contract:** Same as StatefulAction::init_state — MUST be pure or idempotent.
    fn init_state(
        &self,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<Self::State, ActionError>> + Send;

    /// Quick setup: connect, register, validate. MUST return quickly.
    /// Runtime persists state after this returns.
    ///
    /// **Contract:** start() MUST be idempotent — runtime may call it
    /// multiple times (after Reconnect, after restart).
    fn start(
        &self,
        state: &mut Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<TriggerStartMode, ActionError>> + Send;

    /// Long-running worker loop. Runtime spawns in dedicated task with owned ctx.
    /// Only called if start() returns TriggerStartMode::Streaming.
    /// Runs until cancellation or TriggerCompletion.
    fn run(
        &self,
        state: &mut Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<TriggerCompletion, ActionError>> + Send {
        async { Ok(TriggerCompletion::Finished) }
    }

    /// Stop the trigger. Receives current state if available.
    /// Runtime loads state from store before calling stop().
    /// If state unavailable (crash recovery, store down), runtime passes None —
    /// action must handle both cases gracefully.
    fn stop(
        &self,
        state: Option<&mut Self::State>,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<(), ActionError>> + Send;

    fn health_check(
        &self,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<TriggerHealth, ActionError>> + Send {
        async { Ok(TriggerHealth::Healthy) }
    }
}

/// What start() tells runtime about trigger mode.
pub enum TriggerStartMode {
    /// Webhook/cron: registered, no background loop needed.
    Registered,
    /// Streaming: runtime should call run() in a dedicated task.
    Streaming,
    /// Transient failure, retry start after delay.
    RetryAfter(Duration),
}

/// How run() terminated. Determines runtime behavior.
///
/// **Semantics:**
/// - `Finished` = trigger permanently done (one-time scheduled trigger fired,
///   source exhausted). Runtime deactivates trigger, no restart.
/// - `Reconnect` = transient failure (connection lost, timeout). Runtime calls
///   start() again after delay. NOT permanent — trigger remains active.
pub enum TriggerCompletion {
    /// Trigger done permanently. Deactivate — do not restart/reschedule.
    Finished,
    /// Connection lost, request restart. Runtime calls start() after delay.
    Reconnect { after: Duration },
}

pub enum TriggerHealth {
    Healthy,
    Degraded { message: String },
    Unhealthy { reason: String },
}
```

### Runtime contract for triggers

```
1. Runtime calls trigger.init_state(ctx) → State (first start only)
2. Runtime calls trigger.start(&mut state, ctx) → TriggerStartMode
3. Runtime persists state
4. Match TriggerStartMode:
   - Registered → done, runtime routes webhooks / fires cron schedule
   - Streaming → runtime spawns dedicated task: trigger.run(&mut state, ctx_owned)
   - RetryAfter → runtime retries start() after delay
5. run() loops until cancellation or TriggerCompletion
6. On stop signal: runtime cancels via CancellationToken, then:
   a. Loads state from store (if available)
   b. Calls trigger.stop(Some(&mut state), ctx) — or stop(None, ctx) if state unavailable
7. On TriggerCompletion::Finished → runtime deactivates trigger permanently
8. On TriggerCompletion::Reconnect → runtime calls start() again after delay
```

---

## EventTrigger (DX — zero-boilerplate streaming triggers)

Bridges `Resource` with `EventSource` topology and `TriggerAction`.
Author writes **only event handling logic**. Connection management, reconnection,
health monitoring, graceful shutdown — all handled by resource layer.

```
Resource layer:                    Trigger layer:
┌──────────────────────┐          ┌──────────────────────┐
│ WebSocketResource    │          │ WebSocketTrigger     │
│   impl EventSource   │──recv──→│   impl EventTrigger  │
│   reconnect: auto    │          │   next_event()       │
│   health: auto       │          │   → emit_execution   │
└──────────────────────┘          └──────────────────────┘
```

### Trait definition

```rust
/// Zero-boilerplate trigger for event-driven sources.
///
/// Author implements only next_event(). Framework handles:
/// - Resource subscription (EventSource::subscribe)
/// - Event receiving loop (EventSource::recv)
/// - Reconnection on failure (RecoveryGate in resource layer)
/// - Health monitoring (resource health → trigger health)
/// - Graceful shutdown (CancellationToken)
/// - State checkpoint (periodic, via ctx.checkpoint)
///
/// **Naming:** `next_event()` (not `on_event()`) because the author is
/// responsible for pulling the next event from the source handle,
/// not handling an already-received event.
pub trait EventTrigger: Action {
    /// Resource handle type (obtained via ctx.resource_typed).
    type Source: 'static;

    /// Event output type emitted to workflow execution.
    type Event: Serialize + Send + Sync + 'static;

    /// Resource key for the event source (declared in ActionComponents).
    fn source_key(&self) -> &str;

    /// Pull and process the next event from the source.
    /// Return Some(event) to emit workflow execution, None to skip (filter).
    fn next_event(
        &self,
        source: &Self::Source,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<Option<Self::Event>, ActionError>> + Send;

    /// Error handling policy. Default: reconnect via resource layer.
    fn on_error(&self, _error: &ActionError) -> EventTriggerErrorPolicy {
        EventTriggerErrorPolicy::Reconnect
    }

    /// Checkpoint every N events. Default: 100.
    /// High-throughput (Kafka): set higher (1000+).
    /// Low-throughput (Telegram): set lower (1-10).
    fn checkpoint_interval(&self) -> usize { 100 }

    /// Emit strategy. Default: Auto (framework emits + checkpoints).
    /// Manual: author controls emission inside next_event (for atomic emit+checkpoint).
    fn checkpoint_strategy(&self) -> CheckpointStrategy { CheckpointStrategy::Auto }
}

pub enum EventTriggerErrorPolicy {
    /// Reconnect via resource layer (default). Translates to
    /// TriggerCompletion::Reconnect in the blanket impl.
    Reconnect,
    /// Skip this event and continue.
    Skip,
    /// Stop trigger permanently.
    Stop,
}

pub enum CheckpointStrategy {
    /// Framework calls emit_execution after next_event returns Some,
    /// and checkpoints every checkpoint_interval() events.
    Auto,
    /// Author handles emission inside next_event (e.g., Kafka: emit_and_checkpoint + commit).
    /// Framework skips emit_execution for Ok(Some(_)) — author already emitted.
    Manual,
}
```

### Blanket impl: EventTrigger → TriggerAction

```rust
impl<T: EventTrigger> TriggerAction for T {
    type State = EventTriggerState;

    async fn start(&self, state: &mut Self::State, ctx: &TriggerContext)
        -> Result<TriggerStartMode, ActionError>
    {
        // Verify resource is available
        ctx.resource_typed::<Self::Source>(self.source_key()).await?;
        Ok(TriggerStartMode::Streaming)
    }

    async fn run(&self, state: &mut Self::State, ctx: &TriggerContext)
        -> Result<TriggerCompletion, ActionError>
    {
        let source = ctx.resource_typed::<Self::Source>(self.source_key()).await?;
        let interval = self.checkpoint_interval();
        let strategy = self.checkpoint_strategy();

        loop {
            if ctx.is_cancelled() {
                return Ok(TriggerCompletion::Finished);
            }
            match self.next_event(&source, ctx).await {
                Ok(Some(event)) => {
                    match strategy {
                        CheckpointStrategy::Auto => {
                            let payload = serde_json::to_value(&event)
                                .map_err(|e| ActionError::fatal(format!("event serialize: {e}")))?;
                            ctx.emit_execution(payload).await?;
                        }
                        CheckpointStrategy::Manual => {
                            // Author already emitted inside next_event
                        }
                    }
                    state.events_emitted += 1;
                    if interval > 0 && state.events_emitted % interval as u64 == 0 {
                        ctx.checkpoint(serde_json::to_value(state)?).await?;
                    }
                }
                Ok(None) => { /* filtered, skip */ }
                Err(e) => {
                    match self.on_error(&e) {
                        EventTriggerErrorPolicy::Reconnect =>
                            return Ok(TriggerCompletion::Reconnect { after: Duration::from_secs(1) }),
                        EventTriggerErrorPolicy::Skip => continue,
                        EventTriggerErrorPolicy::Stop =>
                            return Ok(TriggerCompletion::Finished),
                    }
                }
            }
        }
    }

    async fn stop(&self, _state: Option<&mut Self::State>, _ctx: &TriggerContext)
        -> Result<(), ActionError> { Ok(()) }

    async fn health_check(&self, ctx: &TriggerContext) -> Result<TriggerHealth, ActionError> {
        match ctx.resource_typed::<Self::Source>(self.source_key()).await {
            Ok(_) => Ok(TriggerHealth::Healthy),
            Err(_) => Ok(TriggerHealth::Unhealthy { reason: "source unavailable".into() }),
        }
    }
}
```

### Usage examples

**Telegram (10 строк):**
```rust
#[derive(Action)]
#[action(key = "telegram.messages", name = "Telegram Messages")]
#[resource(TelegramBot, key = "bot")]
struct TelegramMessageTrigger;

impl EventTrigger for TelegramMessageTrigger {
    type Source = TelegramBotHandle;
    type Event = IncomingMessage;
    fn source_key(&self) -> &str { "bot" }

    async fn next_event(&self, bot: &TelegramBotHandle, _ctx: &TriggerContext)
        -> Result<Option<IncomingMessage>, ActionError>
    {
        let update = bot.recv_update().await.retryable()?;
        match update.kind {
            UpdateKind::Message { text: Some(text), .. } =>
                Ok(Some(IncomingMessage { chat_id: update.chat_id, text })),
            _ => Ok(None),
        }
    }
}
```

**Kafka (15 строк):**
```rust
#[derive(Action)]
#[action(key = "kafka.consumer", name = "Kafka Consumer")]
#[resource(KafkaConsumer, key = "kafka")]
struct KafkaConsumerTrigger;

impl EventTrigger for KafkaConsumerTrigger {
    type Source = KafkaConsumerHandle;
    type Event = KafkaMessage;
    fn source_key(&self) -> &str { "kafka" }

    fn checkpoint_strategy(&self) -> CheckpointStrategy {
        CheckpointStrategy::Manual // author controls emit+checkpoint+commit
    }

    async fn next_event(&self, consumer: &KafkaConsumerHandle, ctx: &TriggerContext)
        -> Result<Option<KafkaMessage>, ActionError>
    {
        let msg = consumer.recv().await.retryable()?;
        // Atomic emit + checkpoint, then broker commit
        // NOTE: broker commit is external — at-least-once semantics
        ctx.emit_and_checkpoint(
            serde_json::to_value(&msg)?,
            json!({ "offset": msg.offset }),
        ).await?;
        consumer.commit(&msg).await.retryable()?;
        Ok(None) // already emitted via emit_and_checkpoint (Manual strategy)
    }
}
```

---

## WebhookAction (with State, lifecycle hooks, multiple routes)

n8n parity: `check_exists` → `on_activate` → `on_deactivate` lifecycle.
Runtime calls `check_exists()` before `on_activate()` — if webhook already
registered externally, skip re-registration (idempotent restarts).

Lifecycle hooks receive `&mut State` — critical for storing external IDs
(GitHub hook_id, Stripe webhook endpoint ID) needed for cleanup.

```rust
pub trait WebhookAction: Action {
    type Payload: DeserializeOwned + Send + Sync + 'static;
    type State: Serialize + DeserializeOwned + Default + Send + Sync + 'static;

    /// Internal webhook path (runtime registers in its HTTP server).
    fn webhook_path(&self) -> &str;

    fn routes(&self) -> Vec<WebhookRoute> {
        vec![WebhookRoute {
            path: self.webhook_path().to_string(),
            methods: vec![HttpMethod::POST],
        }]
    }

    // ── External registration lifecycle (n8n webhookMethods parity) ──

    /// Check if webhook already registered in external service.
    /// Called before on_activate(). If returns true → skip on_activate().
    /// Has read access to State (e.g., check stored hook_id is still valid).
    /// Default: false (always activate).
    fn check_exists(
        &self,
        webhook_url: &str,
        state: &Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<bool, ActionError>> + Send {
        async { Ok(false) }
    }

    /// Register webhook in external service (Telegram setWebhook, GitHub create hook).
    /// Skipped if check_exists() returned true.
    /// Has write access to State — store external IDs (hook_id) for later cleanup.
    /// Runtime persists State after on_activate returns.
    /// Default: no-op (for generic webhooks where caller already knows the URL).
    fn on_activate(
        &self,
        webhook_url: &str,
        state: &mut Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        async { Ok(()) }
    }

    /// Unregister webhook from external service (Telegram deleteWebhook, GitHub delete hook).
    /// Has write access to State — can read stored hook_id for targeted deletion.
    /// Called on trigger deactivation / workflow disable.
    /// Default: no-op.
    fn on_deactivate(
        &self,
        state: &mut Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        async { Ok(()) }
    }

    // ── Request handling ──

    fn verify_signature(
        &self,
        headers: &HeaderMap,
        body: &[u8],
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<bool, ActionError>> + Send {
        async { Ok(true) }
    }

    fn handle_request(
        &self,
        payload: Self::Payload,
        state: &mut Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<WebhookResponse, ActionError>> + Send;
}

pub struct WebhookRoute {
    pub path: String,
    pub methods: Vec<HttpMethod>,
}

pub enum HttpMethod { GET, POST, PUT, DELETE, PATCH }
```

### WebhookAction → TriggerAction blanket impl

Runtime orchestrates full lifecycle:

```rust
impl<T: WebhookAction> TriggerAction for T {
    type State = T::State;

    async fn init_state(&self, _ctx: &TriggerContext) -> Result<Self::State, ActionError> {
        Ok(Self::State::default())
    }

    async fn start(&self, state: &mut Self::State, ctx: &TriggerContext)
        -> Result<TriggerStartMode, ActionError>
    {
        // 1. Runtime constructs full public URL: base_url + webhook_path()
        let webhook_url: String = ctx.parameter_typed("__webhook_url")?;

        // 2. Check if already registered externally (idempotent restart)
        let exists = self.check_exists(&webhook_url, state, ctx).await?;

        // 3. Register in external service if not exists
        if !exists {
            self.on_activate(&webhook_url, state, ctx).await?;
            // Runtime persists state after start() returns — hook_id saved
        }

        // 4. Runtime registers path in internal HTTP server (runtime-side, not here)
        Ok(TriggerStartMode::Registered) // no background loop
    }

    async fn stop(&self, state: Option<&mut Self::State>, ctx: &TriggerContext)
        -> Result<(), ActionError>
    {
        // State available → call on_deactivate for cleanup
        if let Some(state) = state {
            self.on_deactivate(state, ctx).await?;
        }
        // State unavailable (crash recovery) → best-effort, log warning
        Ok(())
    }

    async fn health_check(&self, _ctx: &TriggerContext) -> Result<TriggerHealth, ActionError> {
        Ok(TriggerHealth::Healthy)
    }

    // run() — not called (Registered, not Streaming)
}
```

### Runtime HTTP request handling

```
Incoming POST /telegram/webhook
       │
       ▼
┌─ Runtime HTTP Server ──────────────────────────┐
│  1. Match path → find registered WebhookAction │
│  2. Read raw body (bytes)                      │
│  3. action.verify_signature(headers, body, ctx)│
│     → false? Return 401 Unauthorized           │
│  4. Deserialize body → Payload type            │
│     → error? Return 400 Bad Request            │
│  5. Load trigger state from store              │
│  6. action.handle_request(payload, state, ctx) │
│  7. Save updated state to store                │
│  8. Return WebhookResponse as HTTP response    │
└────────────────────────────────────────────────┘
```

### Runtime lifecycle contract

**⚠️ Distributed concurrency:** In multi-worker clusters, runtime MUST acquire
a distributed lock on TriggerId before executing the ACTIVATE pipeline.
Without this, concurrent check_exists→on_activate calls may duplicate webhooks
or hit rate limits (429). This is a runtime/engine responsibility, not action crate.

```
═══ ACTIVATE (workflow enabled) ═══
0. Runtime acquires distributed lock on TriggerId (cluster-safe)
1. Runtime builds webhook_url = base_url + webhook_path()
2. action.check_exists(webhook_url, &state, ctx)?
   → true:  skip on_activate (already registered) ✓
   → false: action.on_activate(webhook_url, &mut state, ctx) — register + store hook_id
3. Runtime persists updated state (hook_id, etc.)
4. Register path in internal HTTP server
5. TriggerStartMode::Registered
6. Release distributed lock

═══ REQUEST (external service sends event) ═══
6. HTTP request → verify_signature → deserialize → handle_request(&mut state)
7. Persist updated state

═══ DEACTIVATE (workflow disabled / trigger stopped) ═══
8. Adapter loads state from store
9. action.on_deactivate(&mut state, ctx) — read hook_id, unregister from external service
10. Persist final state
11. Remove path from HTTP server
12. Revoke ExecutionGuard
```

## RawWebhookAction (non-JSON: form-encoded, XML, multipart)

Same lifecycle as WebhookAction (check_exists/on_activate/on_deactivate),
but receives raw HTTP request instead of deserialized JSON payload.
Lifecycle hooks receive `&mut State` (same fix as WebhookAction).

```rust
pub trait RawWebhookAction: Action {
    type State: Serialize + DeserializeOwned + Default + Send + Sync + 'static;

    fn routes(&self) -> Vec<WebhookRoute>;

    // ── Lifecycle (same as WebhookAction, with State access) ──

    fn check_exists(
        &self,
        webhook_url: &str,
        state: &Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<bool, ActionError>> + Send {
        async { Ok(false) }
    }

    fn on_activate(
        &self,
        webhook_url: &str,
        state: &mut Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        async { Ok(()) }
    }

    fn on_deactivate(
        &self,
        state: &mut Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        async { Ok(()) }
    }

    // ── Request handling (raw bytes, no JSON assumption) ──

    fn handle_raw_request(
        &self,
        request: IncomingHttpRequest,
        state: &mut Self::State,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<WebhookResponse, ActionError>> + Send;
}

/// Content-type agnostic incoming HTTP request.
pub struct IncomingHttpRequest {
    pub method: HttpMethod,
    pub path: String,
    pub headers: HeaderMap,
    pub query: HashMap<String, String>,
    pub body: bytes::Bytes,
    pub content_type: Option<String>,
}
```

## WebhookResponse (full HTTP control)

```rust
pub struct WebhookResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: WebhookResponseBody,
}

pub enum WebhookResponseBody {
    Json(serde_json::Value),
    Text(String),
    Xml(String),
    Binary(bytes::Bytes),
    Empty,
}

impl WebhookResponse {
    pub fn ok() -> Self { Self { status: 200, headers: HashMap::new(), body: WebhookResponseBody::Empty } }
    pub fn accepted() -> Self { Self { status: 202, headers: HashMap::new(), body: WebhookResponseBody::Empty } }
    pub fn json(status: u16, body: serde_json::Value) -> Self { Self { status, headers: HashMap::new(), body: WebhookResponseBody::Json(body) } }
    pub fn text(status: u16, body: String) -> Self { Self { status, headers: HashMap::new(), body: WebhookResponseBody::Text(body) } }
    pub fn xml(status: u16, body: String) -> Self { Self { status, headers: HashMap::new(), body: WebhookResponseBody::Xml(body) } }
}
```

---

## PollAction (revised — with ctx, error policy, validation, emit mode)

```rust
pub trait PollAction: Action {
    type Cursor: Serialize + DeserializeOwned + Default + Send + Sync + 'static;
    type Item: Serialize + Send + Sync + 'static;

    /// Poll interval. Has ctx access for parameter-driven configurable intervals.
    fn poll_interval(&self, ctx: &TriggerContext) -> Result<Duration, ActionError>;

    /// Validate configuration on activate. Fail fast on bad URL/credentials.
    /// Called once during start(). Default: no-op (first poll discovers issues).
    fn validate(
        &self,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<(), ActionError>> + Send {
        async { Ok(()) }
    }

    /// Poll for new items. Cursor automatically persisted via checkpoint.
    fn poll(
        &self,
        cursor: &mut Self::Cursor,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<PollResult<Self::Item>, ActionError>> + Send;

    /// Error handling policy for poll failures.
    /// Default: retry after normal interval (transient errors).
    fn on_poll_error(&self, _error: &ActionError) -> PollErrorPolicy {
        PollErrorPolicy::RetryWithBackoff
    }

    /// How to emit items: one execution per item, one per batch, or chunked.
    /// Default: one execution per item.
    fn emit_mode(&self) -> PollEmitMode { PollEmitMode::Individual }
}

pub struct PollResult<T> {
    pub items: Vec<T>,
    /// Control when next poll happens.
    pub next_poll: NextPoll,
}

pub enum NextPoll {
    /// Use default poll_interval().
    Default,
    /// Poll again immediately (for long polling / has_more pattern).
    Immediate,
    /// Custom delay before next poll.
    After(Duration),
    /// No more data expected — trigger can deactivate.
    Done,
}

/// Error handling policy for poll failures.
pub enum PollErrorPolicy {
    /// Retry after normal poll_interval (transient error).
    RetryWithBackoff,
    /// Skip this poll cycle, try next.
    Skip,
    /// Stop trigger permanently (unrecoverable).
    Stop,
}

/// How to emit items from a single poll() call.
pub enum PollEmitMode {
    /// One workflow execution per item (default).
    Individual,
    /// One workflow execution with all items as array.
    Batch,
    /// Group items into chunks of N, one execution per chunk.
    BatchChunked(usize),
}
```

### PollAction → TriggerAction blanket impl

**Cancellation safety:** `poll()` may hang on long HTTP requests. The blanket impl
wraps poll() in `tokio::select!` with cancellation to ensure prompt shutdown.

**⚠️ Cancellation safety contract:** The `poll()` future MAY be dropped mid-execution
by `tokio::select!` if cancellation signal arrives. Action authors MUST ensure that
`poll()` is cancellation-safe:

- **Safe:** HTTP GET requests (read-only, idempotent).
- **Unsafe:** Destructive reads from queues (SQS ReceiveMessage + Delete).
  If poll() reads from SQS and the future is dropped before returning,
  messages are lost.

**For destructive sources**, use two-phase pattern inside poll():
1. Read message (non-destructive: SQS visibility timeout, not Delete)
2. Return message in PollResult
3. Blanket impl checkpoints cursor
4. On NEXT poll(), action acknowledges/deletes previously returned messages

Or use `EventTrigger` with `CheckpointStrategy::Manual` for full control.

```rust
impl<T: PollAction> TriggerAction for T {
    type State = PollState<T::Cursor>;

    async fn start(&self, state: &mut Self::State, ctx: &TriggerContext)
        -> Result<TriggerStartMode, ActionError>
    {
        self.validate(ctx).await?;
        let _ = self.poll_interval(ctx)?;
        Ok(TriggerStartMode::Streaming)
    }

    async fn run(&self, state: &mut Self::State, ctx: &TriggerContext)
        -> Result<TriggerCompletion, ActionError>
    {
        loop {
            if ctx.is_cancelled() {
                return Ok(TriggerCompletion::Finished);
            }

            // Cancellation-safe poll: if shutdown signal arrives during a long
            // poll() call (e.g., 5-min HTTP long-poll), we abort immediately.
            let poll_result = tokio::select! {
                result = self.poll(&mut state.cursor, ctx) => result,
                _ = ctx.cancellation.cancelled() => {
                    return Ok(TriggerCompletion::Finished);
                }
            };

            match poll_result {
                Ok(result) => {
                    // Emit items
                    if !result.items.is_empty() {
                        let items_json: Vec<serde_json::Value> = result.items.iter()
                            .map(|item| serde_json::to_value(item))
                            .collect::<Result<_, _>>()
                            .map_err(|e| ActionError::fatal(format!("item serialize: {e}")))?;

                        match self.emit_mode() {
                            PollEmitMode::Individual => {
                                for item in items_json {
                                    ctx.emit_execution(item).await?;
                                }
                            }
                            PollEmitMode::Batch => {
                                ctx.emit_execution(serde_json::Value::Array(items_json)).await?;
                            }
                            PollEmitMode::BatchChunked(n) => {
                                for chunk in items_json.chunks(n) {
                                    ctx.emit_execution(serde_json::Value::Array(chunk.to_vec())).await?;
                                }
                            }
                        }
                    }

                    // Checkpoint cursor
                    ctx.checkpoint(serde_json::to_value(&state)?).await?;

                    // Determine next action
                    match result.next_poll {
                        NextPoll::Done => return Ok(TriggerCompletion::Finished),
                        NextPoll::Immediate => continue,
                        delay_variant => {
                            let delay = match delay_variant {
                                NextPoll::After(d) => d,
                                NextPoll::Default => match self.poll_interval(ctx) {
                                    Ok(d) => d,
                                    Err(e) => match self.on_poll_error(&e) {
                                        PollErrorPolicy::Stop => return Ok(TriggerCompletion::Finished),
                                        _ => Duration::from_secs(60),
                                    }
                                },
                                _ => unreachable!(),
                            };
                            tokio::select! {
                                _ = tokio::time::sleep(delay) => {}
                                _ = ctx.cancellation.cancelled() => {
                                    return Ok(TriggerCompletion::Finished);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    match self.on_poll_error(&e) {
                        PollErrorPolicy::RetryWithBackoff => {
                            let delay = self.poll_interval(ctx).unwrap_or(Duration::from_secs(60));
                            tokio::select! {
                                _ = tokio::time::sleep(delay) => {}
                                _ = ctx.cancellation.cancelled() => {
                                    return Ok(TriggerCompletion::Finished);
                                }
                            }
                        }
                        PollErrorPolicy::Skip => continue,
                        PollErrorPolicy::Stop => {
                            return Ok(TriggerCompletion::Finished);
                        }
                    }
                }
            }
        }
    }

    async fn stop(&self, _state: Option<&mut Self::State>, _ctx: &TriggerContext)
        -> Result<(), ActionError> { Ok(()) }
}
```

---

## ScheduledTrigger (new DX type — Cron, Interval, One-time)

Reduces 50-line cron boilerplate to 6-10 lines. Author implements only
`next_fire_time()` — framework handles sleep loop, cancellation, emit, checkpoint.

```rust
/// DX type for schedule-based triggers (cron, interval, one-time).
/// Blanket impl → TriggerAction (Streaming mode with sleep loop).
pub trait ScheduledTrigger: Action {
    /// Compute next fire time. Return None to deactivate (one-time triggers).
    fn next_fire_time(
        &self,
        last_fired: Option<chrono::DateTime<chrono::Utc>>,
        ctx: &TriggerContext,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, ActionError>;

    /// Build execution payload for this tick. Default: timestamp JSON.
    fn on_tick(
        &self,
        scheduled_at: chrono::DateTime<chrono::Utc>,
        ctx: &TriggerContext,
    ) -> impl Future<Output = Result<serde_json::Value, ActionError>> + Send {
        async move {
            Ok(serde_json::json!({
                "scheduled_at": scheduled_at.to_rfc3339(),
                "fired_at": chrono::Utc::now().to_rfc3339(),
            }))
        }
    }
}

// Blanket: ScheduledTrigger → TriggerAction
impl<T: ScheduledTrigger> TriggerAction for T {
    type State = ScheduleState; // { last_fired, fire_count }

    async fn start(&self, state: &mut ScheduleState, ctx: &TriggerContext)
        -> Result<TriggerStartMode, ActionError>
    {
        // Validate: can we compute at least one fire time?
        let next = self.next_fire_time(state.last_fired, ctx)?;
        if next.is_none() {
            return Ok(TriggerStartMode::Registered); // already finished (one-time)
        }
        Ok(TriggerStartMode::Streaming)
    }

    async fn run(&self, state: &mut ScheduleState, ctx: &TriggerContext)
        -> Result<TriggerCompletion, ActionError>
    {
        loop {
            let next = self.next_fire_time(state.last_fired, ctx)?;
            let Some(fire_at) = next else {
                // No more fire times → deactivate (one-time trigger done)
                return Ok(TriggerCompletion::Finished);
            };

            let now = chrono::Utc::now();
            let delay = (fire_at - now).to_std().unwrap_or(Duration::ZERO);

            tokio::select! {
                _ = tokio::time::sleep(delay) => {
                    let payload = self.on_tick(fire_at, ctx).await?;
                    ctx.emit_execution(payload).await?;

                    state.last_fired = Some(chrono::Utc::now());
                    state.fire_count += 1;
                    ctx.checkpoint(serde_json::to_value(state)?).await?;
                }
                _ = ctx.cancellation.cancelled() => {
                    return Ok(TriggerCompletion::Finished);
                }
            }
        }
    }

    async fn stop(&self, _state: Option<&mut ScheduleState>, _ctx: &TriggerContext)
        -> Result<(), ActionError> { Ok(()) }
}
```

### Usage examples

**Cron (10 строк):**
```rust
#[derive(Action, Clone)]
#[action(key = "trigger.cron", name = "Cron", category = "schedule")]
struct CronTrigger;

impl ScheduledTrigger for CronTrigger {
    fn next_fire_time(
        &self, _last: Option<DateTime<Utc>>, ctx: &TriggerContext,
    ) -> Result<Option<DateTime<Utc>>, ActionError> {
        let expr: String = ctx.parameter_typed("cron")?;
        let schedule = cron::Schedule::from_str(&expr).validation()?;
        Ok(schedule.upcoming(chrono::Utc).next())
    }
}
```

**Interval (8 строк):**
```rust
#[derive(Action, Clone)]
#[action(key = "trigger.interval", name = "Interval", category = "schedule")]
struct IntervalTrigger;

impl ScheduledTrigger for IntervalTrigger {
    fn next_fire_time(
        &self, last: Option<DateTime<Utc>>, ctx: &TriggerContext,
    ) -> Result<Option<DateTime<Utc>>, ActionError> {
        let minutes: u64 = ctx.parameter_typed("minutes")?;
        let base = last.unwrap_or_else(chrono::Utc::now);
        Ok(Some(base + chrono::Duration::minutes(minutes as i64)))
    }
}
```

**One-time (6 строк):**
```rust
#[derive(Action, Clone)]
#[action(key = "trigger.once", name = "One-Time", category = "schedule")]
struct OneTimeTrigger;

impl ScheduledTrigger for OneTimeTrigger {
    fn next_fire_time(
        &self, last: Option<DateTime<Utc>>, ctx: &TriggerContext,
    ) -> Result<Option<DateTime<Utc>>, ActionError> {
        if last.is_some() { return Ok(None); } // already fired → deactivate
        let at: String = ctx.parameter_typed("scheduled_at")?;
        Ok(Some(at.parse().validation()?))
    }
}
```

---

## n8n Parity Reference

| n8n Concept | Nebula Equivalent | Notes |
|-------------|-------------------|-------|
| `INodeType.execute()` | `StatelessAction` / `SimpleAction` | One-shot input → output |
| `INodeType.poll()` | `PollAction` | cursor + interval + return new items |
| `INodeType.webhook()` | `WebhookAction.handle_request()` | path + verify + handle + response |
| `INodeType.trigger()` | `EventTrigger` / raw `TriggerAction` | long-running event source |
| Cron Trigger node | `ScheduledTrigger` | 10 строк vs n8n's manual cron node |
| `INodeTypeDescription` | `ActionDescriptor` + `ActionInput` | UI schema, parameters, credentials |
| `credentials` array | `#[credential(BearerToken, key = "bot")]` | declarative dependency |
| `webhookMethods.checkExists` | `WebhookAction::check_exists()` | skip re-registration on restart |
| `webhookMethods.create` | `WebhookAction::on_activate()` | register in external service |
| `webhookMethods.delete` | `WebhookAction::on_deactivate()` | unregister from external service |

### Where Nebula goes beyond n8n

| Feature | n8n | Nebula |
|---------|-----|--------|
| Type safety | ❌ runtime JSON | ✅ compile-time Input/Output/State |
| Webhook deduplication | ❌ manual per node | ✅ `type State` with persistent ring buffer |
| Signature verification | ❌ manual in webhook() | ✅ `verify_signature()` called by framework |
| Streaming triggers | ❌ (poll/webhook only) | ✅ `EventTrigger` + Resource EventSource |
| Schedule DX | ❌ manual cron/interval logic | ✅ `ScheduledTrigger` — 6-10 lines per schedule type |
| Poll error policy | ❌ crash or silent fail | ✅ `on_poll_error()` → Retry/Skip/Stop |
| Poll configurable interval | ❌ hardcoded | ✅ `poll_interval(ctx)` — parameter-driven |
| Connection management | ❌ each node manages itself | ✅ Resource layer (reconnect, health, shutdown) |
| Atomic emit+checkpoint | ❌ none | ✅ `emit_and_checkpoint()` |
| Sandbox enforcement | ❌ none | ✅ CapabilityManifest + ExecutionGuard |
| Binary streaming | ❌ all in memory | ✅ BinaryData (inline/stored) + BinaryStorage resource |
| LLM token streaming | ❌ none | ✅ StreamSender |
| State migration | ❌ none | ✅ state_version + migrate_state |
| Panic safety | ❌ process crash | ✅ tokio::spawn + JoinError::is_panic() + pre-execute snapshot |
