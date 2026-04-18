# Spec 11 — Triggers (cron, webhook, event, polling)

> **Status:** draft
> **Canon target:** §9.1–§9.5 (new)
> **Depends on:** 02 (tenancy), 04 (RBAC + service accounts), 10 (quotas), 16 (storage)
> **Depended on by:** 15 (delivery semantics)

## Problem

Triggers are the entry point for **every execution that wasn't started by a manual API call**. Getting trigger semantics wrong means:

- Events silently dropped (bad)
- Events duplicated without dedup (worse — causes double charges)
- Webhooks without auth (security hole)
- Cron that runs 1000 missed executions on restart (Airflow foot gun)
- Cron that misses every execution on clock skew (subtle bug)
- Thundering herd on `:00` second every minute

Canon §9 sets the contract: **«at-least-once delivery with documented semantics; duplicates controlled via event identity + idempotency»**. This spec defines the machinery.

## Decision

**One `TriggerAction` base trait with three ergonomic specializations** (`PollingAction`, `WebhookAction`, `EventAction`). Plus native cron via scheduler (not as an action). All trigger events flow through `trigger_events` inbox table with built-in dedup via unique constraint. Engine dispatches by claiming from the inbox. Leaderless coordination for cron via unique slot claiming.

## Trigger type taxonomy

| Type | Initiator | Latency | Example | Spec section |
|---|---|---|---|---|
| **Manual** | User API call | immediate | Alice clicks "Run now" | — (just API endpoint) |
| **Cron** | Internal scheduler | schedule-based | Daily at 9am UTC | 11A |
| **Webhook** | External HTTP POST | on-demand | GitHub push, Stripe event | 11B |
| **Event stream** | Plugin subscribes external queue | on-demand | Kafka, SQS | 11C |
| **Polling** | Plugin polls external API | interval | Check new emails every 5 min | 11D |

## Action trait family

```rust
// nebula-action/src/trigger.rs

/// Base trait for all trigger actions. Not implemented directly by authors usually —
/// they implement one of the specializations below. Blanket impls convert each
/// specialization into TriggerAction.
#[async_trait]
pub trait TriggerAction: Send + Sync + 'static {
    type Config: DeserializeOwned + Serialize + Send + Sync;
    
    /// Long-running task that watches for events and emits them.
    /// Runtime spawns this when trigger is activated, cancels on deactivation.
    async fn run(&self, ctx: TriggerContext, config: Self::Config) -> Result<(), TriggerError>;
}

pub struct TriggerContext {
    pub trigger_id: TriggerId,
    pub workspace_id: WorkspaceId,
    pub org_id: OrgId,
    pub service_account: ServiceAccountId,  // who executes the triggered workflow
    pub cancellation: CancellationSignal,
    pub metrics: MetricsHandle,
    pub logger: LoggerHandle,
    // No direct DB access — emit() is the only sink
}

impl TriggerContext {
    /// Emit an event to the inbox. Engine picks it up and dispatches workflow.
    /// Dedup based on event_id. Idempotent.
    pub async fn emit(&self, event: TriggerEvent) -> Result<EmitOutcome, TriggerError> {
        // Writes to trigger_events with UNIQUE constraint handling
    }
}

pub struct TriggerEvent {
    pub event_id: String,       // dedup key — required
    pub payload: Value,         // passed to workflow as input
    pub received_at: DateTime<Utc>,
    pub metadata: HashMap<String, String>,  // for audit
}

pub enum EmitOutcome {
    Accepted,       // new event, will be dispatched
    Duplicate,      // event_id already seen, ignored
    QuotaExceeded,  // inbox full or workspace quota hit
}
```

### Specializations

```rust
// ------------ PollingAction ------------

#[async_trait]
pub trait PollingAction: Send + Sync + 'static {
    type Config: DeserializeOwned + Serialize + Send + Sync;
    type Item: Serialize + Send + Sync;
    
    /// Minimum interval between polls (runtime enforces).
    fn poll_interval(&self) -> Duration {
        Duration::from_secs(60)
    }
    
    /// Poll the external system once, return new items.
    async fn poll(
        &self,
        ctx: PollingContext,
        config: &Self::Config,
    ) -> Result<Vec<Self::Item>, TriggerError>;
    
    /// Derive stable event id from item (for dedup).
    fn event_id(&self, item: &Self::Item) -> String;
}

pub struct PollingContext {
    pub trigger: TriggerContext,
    pub cursor: Option<Value>,      // last cursor, deserialized per plugin
}

impl<T: PollingAction> TriggerAction for T {
    type Config = T::Config;
    
    async fn run(&self, ctx: TriggerContext, config: Self::Config) -> Result<(), TriggerError> {
        let mut cursor: Option<Value> = load_cursor(&ctx).await?;
        
        loop {
            tokio::select! {
                _ = ctx.cancellation.cancelled() => break,
                _ = tokio::time::sleep(self.poll_interval()) => {}
            }
            
            let polling_ctx = PollingContext {
                trigger: ctx.clone(),
                cursor: cursor.clone(),
            };
            
            match self.poll(polling_ctx, &config).await {
                Ok(items) => {
                    for item in items {
                        let event_id = self.event_id(&item);
                        let event = TriggerEvent {
                            event_id,
                            payload: serde_json::to_value(&item)?,
                            received_at: Utc::now(),
                            metadata: Default::default(),
                        };
                        ctx.emit(event).await?;
                    }
                    // Optionally update cursor based on latest item
                }
                Err(e) => {
                    ctx.logger.warn(format!("poll failed: {}", e));
                    // Continue polling — transient failures don't kill trigger
                }
            }
        }
        Ok(())
    }
}

// ------------ WebhookAction ------------

#[async_trait]
pub trait WebhookAction: Send + Sync + 'static {
    type Config: DeserializeOwned + Serialize + Send + Sync;
    
    /// Authentication scheme. Runtime verifies before calling handle().
    fn auth(&self) -> WebhookAuth {
        WebhookAuth::None
    }
    
    /// Extract stable event id from request. Used for dedup.
    fn event_id(&self, req: &WebhookRequest) -> String;
    
    /// Handle verified, dedup'd request. Return response to sender.
    async fn handle(
        &self,
        ctx: WebhookContext,
        config: &Self::Config,
        request: WebhookRequest,
    ) -> Result<WebhookResponse, TriggerError>;
}

pub struct WebhookRequest {
    pub method: http::Method,
    pub headers: http::HeaderMap,
    pub body: bytes::Bytes,
    pub query_params: HashMap<String, String>,
    pub remote_addr: Option<IpAddr>,
}

pub struct WebhookResponse {
    pub status: http::StatusCode,
    pub headers: http::HeaderMap,
    pub body: bytes::Bytes,
}

pub struct WebhookContext {
    pub trigger: TriggerContext,
}

impl<T: WebhookAction> TriggerAction for T {
    type Config = T::Config;
    
    async fn run(&self, ctx: TriggerContext, _config: Self::Config) -> Result<(), TriggerError> {
        // Webhook triggers don't "run" in the traditional sense.
        // Registration happens at trigger activation; runtime routes incoming
        // HTTP requests to handle() based on the trigger's path.
        // This function just waits for cancellation.
        ctx.cancellation.cancelled().await;
        Ok(())
    }
}

// ------------ EventAction ------------

#[async_trait]
pub trait EventAction: Send + Sync + 'static {
    type Config: DeserializeOwned + Serialize + Send + Sync;
    type Event: Serialize + Send + Sync;
    
    /// Subscribe to external event source and loop.
    /// Plugin manages the external connection (Kafka, SQS, RabbitMQ, etc.).
    /// Must honor ctx.cancellation for graceful shutdown.
    async fn consume(
        &self,
        ctx: EventContext,
        config: &Self::Config,
    ) -> Result<(), TriggerError>;
}

pub struct EventContext {
    pub trigger: TriggerContext,
    // Plugin gets full trigger context, emits via ctx.trigger.emit()
}

impl<T: EventAction> TriggerAction for T {
    type Config = T::Config;
    
    async fn run(&self, ctx: TriggerContext, config: Self::Config) -> Result<(), TriggerError> {
        self.consume(
            EventContext { trigger: ctx.clone() },
            &config
        ).await
    }
}
```

**Why specializations:** ergonomic clarity for authors. A Kafka consumer plugin author should think about `consume()`, not reimplement a loop around `run()`. Runtime can also apply specialization-specific policies (e.g., enforce min poll interval for `PollingAction`).

## 11A. Cron triggers

Cron is **not an action trait** — it's native to the scheduler. Workflow definition includes cron trigger config; scheduler fires based on schedule.

### Configuration

```rust
// nebula-workflow/src/triggers.rs
pub struct CronTriggerConfig {
    pub schedule: String,                  // e.g., "0 9 * * *"
    pub timezone: String,                  // IANA TZ, e.g., "Europe/Moscow"
    pub overlap_policy: OverlapPolicy,
    pub catch_up: CatchUpPolicy,
    pub jitter_seconds: u32,               // default 30
    pub run_as: Option<ServiceAccountId>,  // default: workspace's auto-created sa_cron_default
}

pub enum OverlapPolicy {
    /// Run concurrently with previous (default for idempotent workflows).
    Allow,
    /// Skip this firing if previous still running. (SAFER default)
    Skip,
    /// Queue up to N pending firings, drop further.
    Buffer { max_pending: u32 },
    /// Cancel previous, start new.
    CancelPrevious,
}

pub enum CatchUpPolicy {
    /// Default — only real-time schedule matches, never catch up.
    /// If process was down during a scheduled time, it's missed forever.
    Skip,
    /// Fire the most recent missed slot (if any), discard older.
    LatestOnly,
    /// Fire all missed slots in order (up to max, Airflow's dangerous default).
    All { max: u32 },
}

impl Default for CronTriggerConfig {
    fn default() -> Self {
        Self {
            schedule: String::new(),
            timezone: "UTC".to_string(),
            overlap_policy: OverlapPolicy::Skip,
            catch_up: CatchUpPolicy::Skip,
            jitter_seconds: 30,
            run_as: None,
        }
    }
}
```

**Defaults chosen to prevent foot guns:**

- `overlap_policy: Skip` — if previous hasn't finished, skip. Conservative.
- `catch_up: Skip` — never catch up on missed runs. No Airflow disasters.
- `jitter: 30s` — deterministic but spread; prevents thundering herd on `:00`.

### Deterministic jitter

```rust
pub fn compute_fire_time(
    workflow_id: WorkflowId,
    scheduled_time: DateTime<Utc>,
    jitter_seconds: u32,
) -> DateTime<Utc> {
    // Hash workflow ID to pick stable offset within [0, jitter_seconds)
    let mut hasher = Sha256::new();
    hasher.update(workflow_id.as_bytes());
    let hash = hasher.finalize();
    let offset_secs = u32::from_le_bytes([hash[0], hash[1], hash[2], hash[3]]) % jitter_seconds;
    scheduled_time + Duration::from_secs(offset_secs as u64)
}
```

Each workflow gets a **stable** offset, reproducible for debugging. 100 workflows scheduled for `0 9 * * *` spread across `9:00:00` to `9:00:30`.

### Leaderless claiming

No cron leader. Each dispatcher periodically scans upcoming fire slots and claims via unique constraint:

```sql
CREATE TABLE cron_fire_slots (
    trigger_id     BYTEA NOT NULL,
    scheduled_for  TIMESTAMPTZ NOT NULL,
    claimed_by     BYTEA NOT NULL,         -- node_id that claimed
    claimed_at     TIMESTAMPTZ NOT NULL,
    execution_id   BYTEA,                  -- populated after execution created
    PRIMARY KEY (trigger_id, scheduled_for)
);

-- Index for cleanup
CREATE INDEX idx_cron_fire_slots_cleanup
    ON cron_fire_slots (claimed_at)
    WHERE claimed_at < NOW() - INTERVAL '7 days';
```

**Claim query:**

```sql
INSERT INTO cron_fire_slots (trigger_id, scheduled_for, claimed_by, claimed_at)
VALUES ($1, $2, $3, NOW())
ON CONFLICT (trigger_id, scheduled_for) DO NOTHING
RETURNING *;
```

- If INSERT returns row: this dispatcher claimed this slot, proceed to create execution
- If INSERT returns empty: another dispatcher claimed it first, skip

This is **unique constraint as coordination primitive** — no leader election needed. Same pattern Kubernetes CronJob controller uses.

### Scheduler loop

Each dispatcher periodically (every 10s) runs:

```rust
async fn cron_scheduler_tick(&self) -> Result<()> {
    // 1. Load all active cron triggers
    let triggers = self.storage.list_active_cron_triggers().await?;
    
    // 2. For each, compute upcoming fire slot (with jitter)
    let now = Utc::now();
    for trigger in triggers {
        let config = &trigger.config;
        let schedule = cron::Schedule::from_str(&config.schedule)?;
        let tz: chrono_tz::Tz = config.timezone.parse()?;
        
        // Find slots to consider: last 10 minutes + next 1 minute
        let lookback = now - Duration::from_secs(600);
        let lookahead = now + Duration::from_secs(60);
        
        for scheduled in schedule.after(&lookback.with_timezone(&tz)) {
            if scheduled > lookahead.with_timezone(&tz) { break; }
            
            let fire_time = compute_fire_time(trigger.id, scheduled.with_timezone(&Utc), config.jitter_seconds);
            if fire_time > now { continue; }  // future, handled later
            
            // 3. Try to claim this slot
            let claimed = self.storage.claim_cron_slot(trigger.id, fire_time, self.node_id).await?;
            if !claimed { continue; }  // another dispatcher got it
            
            // 4. Check overlap policy
            match config.overlap_policy {
                OverlapPolicy::Skip => {
                    if self.storage.has_running_execution_for_trigger(trigger.id).await? {
                        continue;  // skip this firing
                    }
                }
                // ... other policies
            }
            
            // 5. Create execution
            self.create_cron_execution(&trigger, fire_time).await?;
        }
    }
    
    Ok(())
}
```

**Catch-up handling:** lookback window (10 minutes) with `CatchUpPolicy::LatestOnly` processes only most-recent missed slot. `Skip` only processes slots within the last few seconds. `All` processes each slot in the lookback window.

### Timezone and DST

`chrono-tz` handles DST correctly:

- **Spring forward:** times `02:30` on DST day don't exist. Cron expression `30 2 * * *` → schedule skips this day (or uses `LatestOnly` to fire at `03:30` if configured).
- **Fall back:** times `02:30` happen twice. `chrono-tz` picks the first occurrence (standard) by default. Cron fires once per logical day.

## 11B. Webhook triggers

### Routing

```
POST /api/v1/hooks/{org}/{workspace}/{trigger_slug}
```

Path-based, consistent with rest of API. `{org}` and `{workspace}` can be slugs or IDs.

### Flow

```
1. POST arrives
  ↓
2. Path resolution (nebula-api middleware)
   Resolve org_slug → org_id
   Resolve workspace_slug → workspace_id
   Look up trigger by (workspace_id, slug)
   If not found → 404
  ↓
3. Load trigger config (includes WebhookAction implementation reference + auth config)
  ↓
4. Rate limit check (per-trigger limit, not user-based)
   429 if exceeded
  ↓
5. Authenticate request per trigger.config.auth
   - None: skip
   - Bearer: check header matches secret
   - HmacSignature: compute HMAC, verify matches header
   - StripeSignature: parse Stripe-Signature header, verify with tolerance
   - MutualTls: check client cert
   - IpAllowlist: check remote_addr in CIDR list
   Failure → 401 Unauthorized
  ↓
6. Extract event_id per trigger.config.event_id_strategy
  ↓
7. Atomic insert into trigger_events:
   INSERT INTO trigger_events (trigger_id, event_id, payload, received_at, ...)
   ON CONFLICT (trigger_id, event_id) DO NOTHING
   RETURNING id
   - If 1 row: accepted, proceed
   - If 0 rows: duplicate, return 200 with "deduplicated" marker
  ↓
8. Return response to sender
   - AcknowledgeAndQueue (default): 202 Accepted { "execution_id": null, "queued": true }
   - SynchronousShort: wait for execution to complete (with timeout)
   - CustomResponse: wait for workflow's first node to provide response
  ↓
9. Worker picks up trigger_events row (via dispatcher's unified claim query §17)
  ↓
10. Create execution, run workflow
```

### Auth variants

```rust
pub enum WebhookAuth {
    None,                               // anyone can POST (trusted network)
    BearerToken {
        header: String,                 // default "Authorization"
        secret_ref: CredentialId,       // points to stored secret
    },
    HmacSignature {
        header: String,                 // e.g., "X-Hub-Signature-256"
        algorithm: HmacAlgo,            // Sha256 / Sha1 / Sha512
        secret_ref: CredentialId,
        prefix: Option<String>,         // e.g., "sha256="
        signed_body: SignedBody,
    },
    StripeSignature {
        header: String,                 // "Stripe-Signature"
        secret_ref: CredentialId,
        tolerance: Duration,             // e.g., 5 minutes
    },
    MutualTls {
        client_ca_ref: CredentialId,
    },
    IpAllowlist {
        cidrs: Vec<IpCidr>,
    },
}

pub enum HmacAlgo { Sha256, Sha1, Sha512 }
pub enum SignedBody { Raw, CanonicalizedJson }
```

**Integration presets:** plugin authors provide pre-configured `WebhookAuth` for popular services. User picks «Stripe» and only enters webhook secret — preset handles the rest.

```rust
pub fn stripe_preset() -> WebhookAuth {
    WebhookAuth::StripeSignature {
        header: "Stripe-Signature".into(),
        secret_ref: CredentialId::placeholder(),  // user fills in
        tolerance: Duration::from_secs(300),      // 5 min
    }
}
```

### Event ID extraction

```rust
pub enum EventIdStrategy {
    /// Extract from HTTP header (common for providers that include delivery id).
    Header { name: String },                  // e.g., "X-GitHub-Delivery"
    /// Extract via path expression on JSON body.
    BodyPath { expression: String },          // e.g., "$.event.id"
    /// Hash of canonicalized body.
    BodyHash,
    /// Combination — paranoid fallback.
    HeaderAndBodyHash { header: String },
}

pub fn extract_event_id(strategy: &EventIdStrategy, req: &WebhookRequest) -> Result<String, TriggerError> {
    match strategy {
        EventIdStrategy::Header { name } => {
            req.headers.get(name)
                .and_then(|v| v.to_str().ok())
                .map(String::from)
                .ok_or(TriggerError::EventIdMissing)
        }
        EventIdStrategy::BodyPath { expression } => {
            let json: Value = serde_json::from_slice(&req.body)?;
            let result = jsonpath_lib::select(&json, expression)?;
            result.first()
                .and_then(|v| v.as_str())
                .map(String::from)
                .ok_or(TriggerError::EventIdMissing)
        }
        EventIdStrategy::BodyHash => {
            let hash = sha256(&req.body);
            Ok(format!("hash:{}", hex::encode(hash)))
        }
        EventIdStrategy::HeaderAndBodyHash { header } => {
            let h = req.headers.get(header).and_then(|v| v.to_str().ok()).unwrap_or("");
            let body_hash = hex::encode(sha256(&req.body));
            Ok(format!("{}:{}", h, body_hash))
        }
    }
}
```

### Response modes

```rust
pub enum WebhookResponseMode {
    /// Default: 202 Accepted immediately, async processing.
    AcknowledgeAndQueue,
    
    /// Wait for execution to complete, return its output.
    /// Use for short workflows (< 30s).
    SynchronousShort { timeout: Duration },
    
    /// Workflow's first node writes response via RespondToWebhook action.
    /// If workflow doesn't respond within timeout, default 202.
    CustomResponse { timeout: Duration },
}
```

Most webhook integrations should use `AcknowledgeAndQueue`. `SynchronousShort` is for API-like interactions where the sender expects an immediate response. `CustomResponse` is for building APIs on top of Nebula (advanced).

### Replay protection

- **Timestamp check** (Stripe-style): `Stripe-Signature` includes timestamp. Reject if `|now - timestamp| > tolerance`.
- **Dedup** handles replay of same event ID

Together these block replay attacks: old event ID already in dedup table, or timestamp outside tolerance.

## 11C. Event stream triggers (queues)

Plugin-based. Plugin owns connection to Kafka / SQS / RabbitMQ / Redis Streams / etc.

### Pattern

```rust
struct KafkaTrigger;

#[async_trait]
impl EventAction for KafkaTrigger {
    type Config = KafkaConfig;
    type Event = KafkaMessage;
    
    async fn consume(&self, ctx: EventContext, config: &Self::Config) -> Result<(), TriggerError> {
        let consumer = kafka_client::Consumer::connect(&config.brokers).await?;
        consumer.subscribe(&config.topics).await?;
        
        loop {
            tokio::select! {
                _ = ctx.trigger.cancellation.cancelled() => {
                    consumer.close().await?;
                    break;
                }
                msg = consumer.recv() => {
                    match msg {
                        Ok(msg) => {
                            let event_id = format!("{}:{}:{}", msg.topic, msg.partition, msg.offset);
                            let event = TriggerEvent {
                                event_id,
                                payload: serde_json::to_value(&msg)?,
                                received_at: Utc::now(),
                                metadata: HashMap::new(),
                            };
                            
                            match ctx.trigger.emit(event).await? {
                                EmitOutcome::Accepted | EmitOutcome::Duplicate => {
                                    consumer.commit(&msg).await?;  // mark as consumed in queue
                                }
                                EmitOutcome::QuotaExceeded => {
                                    // Backpressure — don't commit, pause briefly, retry
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                }
                            }
                        }
                        Err(e) => {
                            ctx.trigger.logger.warn(format!("kafka recv error: {}", e));
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
```

### Offset commit strategy

**Queue-native commit** — after successful emit, plugin commits offset to the external queue. Kafka: `commit(offset)`. SQS: `delete_message`. RabbitMQ: `ack`.

**Why queue-native:**
- Queue tools (Kafka admin UI, SQS console) show truth
- Nebula doesn't duplicate state
- At-least-once semantics preserved (Nebula's dedup handles duplicates on recommit)

**Crash recovery:** plugin process dies between `emit` and `commit`. On restart, same message re-consumed, `event_id` matches in `trigger_events` inbox, `EmitOutcome::Duplicate` returned, plugin commits offset. No double processing.

### Backpressure

If workspace quota (concurrent executions) is hit, `emit` returns `QuotaExceeded`. Plugin **does not commit** the message, pauses, retries. Queue backs up until quota frees.

**Result:** external queue visibly grows. Operator alerts on queue depth monitoring. Natural flow control.

## 11D. Polling triggers

Use `PollingAction` trait. Examples: «check new emails every 5 minutes», «poll Jira for new issues», «check folder for new files».

### Pattern

```rust
struct NewEmailsTrigger;

#[async_trait]
impl PollingAction for NewEmailsTrigger {
    type Config = ImapConfig;
    type Item = Email;
    
    fn poll_interval(&self) -> Duration {
        Duration::from_secs(300)  // every 5 minutes
    }
    
    async fn poll(&self, ctx: PollingContext, config: &Self::Config) -> Result<Vec<Self::Item>, TriggerError> {
        let client = imap_client::connect(&config.server, &config.username, &config.password).await?;
        let last_seen_uid = ctx.cursor
            .as_ref()
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        
        let new_emails = client.fetch_since(last_seen_uid).await?;
        Ok(new_emails)
    }
    
    fn event_id(&self, item: &Self::Item) -> String {
        format!("imap:{}", item.message_id)
    }
}
```

Runtime loops, calls `poll()` at intervals, emits each item as separate event. Dedup via `event_id`.

**Cursor management:** runtime can optionally persist cursor between polls. Plugin updates cursor (e.g., «last seen UID») as it processes. On restart, polling resumes from cursor.

### Adaptive polling (optional, v2)

If plugin returns 0 items for several cycles, runtime can back off polling interval (exponential up to max). When items start flowing again, snap back to configured interval. Saves resources for rarely-updated sources.

## Data model

### `triggers` table

```sql
CREATE TABLE triggers (
    id                 BYTEA PRIMARY KEY,
    workspace_id       BYTEA NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    workflow_id        BYTEA NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    slug               TEXT NOT NULL,           -- unique per workspace for URL routing
    display_name       TEXT NOT NULL,
    kind               TEXT NOT NULL,           -- 'manual' / 'cron' / 'webhook' / 'event' / 'polling'
    config             JSONB NOT NULL,          -- kind-specific config
    state              TEXT NOT NULL,           -- 'active' / 'paused' / 'archived'
    run_as             BYTEA,                   -- ServiceAccountId, NULL → use workspace default
    created_at         TIMESTAMPTZ NOT NULL,
    created_by         BYTEA NOT NULL,
    version            BIGINT NOT NULL DEFAULT 0,
    deleted_at         TIMESTAMPTZ
);

CREATE UNIQUE INDEX idx_triggers_workspace_slug
    ON triggers (workspace_id, LOWER(slug))
    WHERE deleted_at IS NULL;

CREATE INDEX idx_triggers_active
    ON triggers (workspace_id, state)
    WHERE state = 'active' AND deleted_at IS NULL;
```

### `trigger_events` inbox (shared with spec 16)

```sql
CREATE TABLE trigger_events (
    id                 BYTEA PRIMARY KEY,
    trigger_id         BYTEA NOT NULL REFERENCES triggers(id) ON DELETE CASCADE,
    event_id           TEXT NOT NULL,              -- dedup key
    received_at        TIMESTAMPTZ NOT NULL,
    claim_state        TEXT NOT NULL,              -- 'pending' / 'claimed' / 'dispatched' / 'failed'
    claimed_by         BYTEA,                       -- dispatcher node that claimed
    claimed_at         TIMESTAMPTZ,
    payload            JSONB NOT NULL,
    execution_id       BYTEA,                       -- set after execution created
    metadata           JSONB,
    UNIQUE (trigger_id, event_id)                  -- dedup enforcement
);

CREATE INDEX idx_trigger_events_pending
    ON trigger_events (received_at)
    WHERE claim_state = 'pending';

CREATE INDEX idx_trigger_events_cleanup
    ON trigger_events (received_at)
    WHERE claim_state = 'dispatched';
```

**Retention:** dispatched rows kept for audit (e.g., 30 days), then GC'd. Keeps dedup history for that window — duplicate events arriving after 30 days of original will re-fire (rare, acceptable).

## Configuration surface

```toml
[triggers.cron]
scheduler_interval = "10s"     # how often dispatcher scans cron triggers
lookback_window = "10m"        # how far back to look for missed slots
default_jitter_seconds = 30

[triggers.webhook]
max_body_size = "1MB"
default_response_mode = "acknowledge_and_queue"

[triggers.polling]
min_poll_interval = "10s"      # prevent authors from overwhelming external APIs
default_poll_interval = "1m"

[triggers.inbox]
retention_days = 30            # how long to keep dispatched events
quota_per_workspace = 10_000   # max inbox depth before rejecting
```

## Testing criteria

**Unit tests:**
- Cron schedule parser matches `cron` crate expectations
- `compute_fire_time` produces stable jitter per workflow ID
- Event ID extraction for each strategy
- Webhook auth verification for each variant
- DST edge cases (spring forward, fall back)

**Integration tests:**
- Cron fires at scheduled time + jitter
- Cron skip on overlap works
- Cron catch_up=Skip doesn't run missed slots
- Cron catch_up=LatestOnly runs most recent missed
- Cron leaderless claiming: 2 dispatchers, 1 trigger, only one execution per slot
- Webhook 202 Accepted on valid request
- Webhook 401 on bad signature (HMAC, Stripe)
- Webhook 404 on unknown trigger slug
- Webhook dedup: same event_id twice → second returns «deduplicated»
- Queue consumer: emit + commit on success
- Queue consumer: emit + retry on QuotaExceeded
- Polling: cursor persistence across polls
- Polling: new items produce events with stable IDs

**Chaos tests:**
- Dispatcher crashes mid-cron-claim — slot either fully claimed or not, not half-claimed
- Webhook flood: 1000 requests/sec, dedup still works
- Duplicate cron fires across restarts: unique constraint prevents double execution

**Security tests:**
- Webhook without auth on production endpoint — 401
- HMAC signature forgery attempts — rejected
- Timestamp tolerance replay attack — rejected
- IP allowlist bypass via X-Forwarded-For without trusted proxies — rejected
- Path traversal in trigger slug — rejected by slug validator

## Performance targets

- Cron scheduler tick: **< 100 ms** for up to 1000 active cron triggers
- Webhook request handling: **< 50 ms p99** (auth + dedup + insert + 202)
- Event queue consumer throughput: **> 1000 events/sec** per consumer
- Polling trigger overhead: **< 10 MB memory** per active trigger

## Module boundaries

| Component | Crate |
|---|---|
| `TriggerAction`, `PollingAction`, `WebhookAction`, `EventAction` traits | `nebula-action` |
| `TriggerContext`, `TriggerEvent`, `EmitOutcome` | `nebula-action` |
| `CronTriggerConfig`, `OverlapPolicy`, `CatchUpPolicy` | `nebula-workflow` |
| `WebhookAuth`, `EventIdStrategy`, `WebhookResponseMode` | `nebula-action` |
| Cron scheduler loop | `nebula-engine` |
| Webhook routing + auth verification | `nebula-api::webhook` |
| `trigger_events` repo | `nebula-storage` |
| `triggers` repo | `nebula-storage` |
| Plugin-side trigger runners | per-plugin |

## Open questions

- **Scheduled future events from workflows** — can an executing workflow emit a trigger event for later («resume in 1 hour»)? Partially covered by `StepOutcome::WaitUntil` in stateful actions (spec 14). Dedicated workflow-emit-event concept deferred.
- **Event replay from operator panel** — «replay this webhook event that caused the bug» — useful for debugging, needs UI. Deferred to v1.5.
- **Trigger health checks** — auto-pause trigger if it fails N times in a row? Opinionated policy, deferred until user ask.
- **Fan-out triggers** — one trigger event creates N executions (e.g., one webhook → N workflows). Not in v1, use workflow-level branching instead.
- **Transactional triggers** — emit event as part of another DB transaction (e.g., «when row inserted, fire workflow»). Deferred, would require DB-specific integration.
