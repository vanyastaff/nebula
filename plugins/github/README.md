# GitHub Plugin: Quick Architecture Reference

## 🎯 Core Concept

**Specialized Traits + Resource Listeners** instead of unified trigger interface.

---

## 📊 Component Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                        GitHub API                            │
└───────────────┬─────────────────────────┬───────────────────┘
                │ Webhook POST            │ Poll GET
                │                         │
    ┌───────────▼──────────┐  ┌──────────▼────────────┐
    │ WebhookListener      │  │   PollListener        │
    │ (Resource)           │  │   (Resource)          │
    │                      │  │                       │
    │ - Axum HTTP Server   │  │ - Tokio loop          │
    │ - Signature verify   │  │ - State cursor        │
    │ - Event channel      │  │ - Event channel       │
    └───────────┬──────────┘  └──────────┬────────────┘
                │ WebhookEvent            │ PollEvent
                │                         │
    ┌───────────▼──────────┐  ┌──────────▼────────────┐
    │ GithubWebhookTrigger │  │ GithubIssuePollTrigger│
    │ (WebhookTrigger)     │  │ (PollTrigger)         │
    │                      │  │                       │
    │ - handle_webhook()   │  │ - poll()              │
    │ - path()             │  │ - interval()          │
    └───────────┬──────────┘  └──────────┬────────────┘
                │                         │
                └────────┬────────────────┘
                         │ TriggerEvent<T>
                ┌────────▼─────────┐
                │  Nebula Engine   │
                │                  │
                │ - Route events   │
                │ - Start workflow │
                │ - Execute        │
                └──────────────────┘
```

---

## 🔧 Trait Hierarchy

```rust
// Base trait (all triggers)
trait Trigger: Action {
    type Config;
    type Event;
}

// Specialized traits
trait PollTrigger: Trigger {
    fn interval(&self, config: &Config) -> Duration;
    async fn poll(&self, ...) -> Result<Vec<TriggerEvent<Event>>>;
}

trait WebhookTrigger: Trigger {
    fn path(&self, config: &Config) -> String;
    async fn handle_webhook(&self, ...) -> Result<TriggerEvent<Event>>;
}
```

---

## 🎨 Key Types

```rust
// Trigger event with deduplication
pub struct TriggerEvent<T> {
    pub data: T,
    pub timestamp: DateTime<Utc>,
    pub dedup_key: Option<String>,
}

// Incoming webhook request
pub struct WebhookRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: serde_json::Value,
}

// Listener resources produce events via channels
pub struct WebhookListener {
    server_handle: JoinHandle<()>,
    event_rx: UnboundedReceiver<WebhookEvent>,
}

pub struct PollListener {
    poll_handle: JoinHandle<()>,
    event_rx: UnboundedReceiver<PollEvent>,
}
```

---

## 🔄 Lifecycle Flow

### **Webhook Trigger Activation**

```
1. User activates workflow with GithubWebhookTrigger
2. Engine calls resource_manager.acquire::<WebhookListener>(...)
3. WebhookListenerResource.create():
   - Start axum HTTP server on port
   - Create event channel (tx, rx)
   - Return WebhookListener instance
4. Engine spawns task: loop { listener.recv().await }
5. When webhook arrives:
   - Axum handler verifies signature
   - Sends WebhookEvent to channel
   - Engine receives event
   - Calls trigger.handle_webhook()
   - Starts workflow execution
```

### **Webhook Trigger Deactivation**

```
1. User deactivates workflow
2. Engine calls resource_manager.release(listener)
3. WebhookListenerResource.cleanup():
   - Abort server_handle
   - Close event channel
4. Server shuts down
```

### **Poll Trigger Activation**

```
1. User activates workflow with GithubIssuePollTrigger
2. Engine calls resource_manager.acquire::<PollListener>(...)
3. PollListenerResource.create():
   - Spawn tokio task with interval timer
   - Create event channel
   - Return PollListener instance
4. Poll loop:
   - Every N seconds, call GitHub API
   - Filter new events since last_state
   - Send PollEvents to channel
5. Engine receives events and starts workflows
```

---

## 📁 File Structure

```
plugins/github/
├── src/
│   ├── triggers/
│   │   ├── mod.rs
│   │   ├── webhook.rs              ← impl WebhookTrigger
│   │   ├── issue_poll.rs           ← impl PollTrigger
│   │   └── release_poll.rs         ← impl PollTrigger
│   │
│   ├── resources/
│   │   ├── mod.rs
│   │   ├── webhook_listener.rs     ← impl Resource (HTTP server)
│   │   ├── poll_listener.rs        ← impl Resource (polling loop)
│   │   └── github_client.rs        ← impl Resource (Octocrab)
│   │
│   ├── utils/
│   │   ├── signature.rs            ← HMAC-SHA256 verification
│   │   └── lifecycle.rs            ← GitHub webhook CRUD
│   │
│   └── types/
│       ├── webhook_event.rs        ← GitHub event payloads
│       └── common.rs
│
├── ARCHITECTURE.md (detailed)
├── TRIGGERS.md (implementation guide)
└── README.md (this file)
```

---

## 🚀 Quick Start Examples

### **Example 1: GitHub Webhook Trigger**

```rust
use nebula_sdk::prelude::*;

#[derive(Debug)]
pub struct GithubWebhookTrigger {
    meta: ActionMetadata,
}

impl Trigger for GithubWebhookTrigger {
    type Config = GithubWebhookConfig;
    type Event = GithubEvent;
}

#[async_trait]
impl WebhookTrigger for GithubWebhookTrigger {
    fn path(&self, config: &Self::Config) -> String {
        format!("/github/{}/{}", config.owner, config.repo)
    }
    
    async fn handle_webhook(
        &self,
        config: &Self::Config,
        request: WebhookRequest,
        _ctx: &ActionContext,
    ) -> Result<TriggerEvent<Self::Event>, ActionError> {
        // 1. Verify signature
        verify_github_signature(&config.secret, &request)?;
        
        // 2. Extract event
        let event_type = request.headers
            .get("x-github-event")
            .ok_or(ActionError::validation("missing event header"))?;
        
        // 3. Filter
        if !config.events.contains(event_type) {
            return Err(ActionError::ignored("event not subscribed"));
        }
        
        // 4. Return event
        Ok(TriggerEvent::with_dedup(
            GithubEvent {
                event_type: event_type.clone(),
                payload: request.body,
            },
            request.headers.get("x-github-delivery").cloned(),
        ))
    }
}
```

### **Example 2: GitHub Poll Trigger**

```rust
#[derive(Debug)]
pub struct GithubIssuePollTrigger {
    meta: ActionMetadata,
}

impl Trigger for GithubIssuePollTrigger {
    type Config = IssueConfig;
    type Event = IssueEvent;
}

#[async_trait]
impl PollTrigger for GithubIssuePollTrigger {
    fn interval(&self, config: &Self::Config) -> Duration {
        Duration::from_secs(config.poll_interval_seconds)
    }
    
    async fn poll(
        &self,
        config: &Self::Config,
        last_state: Option<Value>,
        _ctx: &ActionContext,
    ) -> Result<Vec<TriggerEvent<Self::Event>>, ActionError> {
        let client = get_github_client(&config.credential)?;
        
        // Get cursor from last poll
        let since = last_state
            .and_then(|s| s.get("since"))
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok());
        
        // Fetch new issues
        let issues = client
            .issues(&config.owner, &config.repo)
            .list()
            .since(since)
            .send()
            .await?;
        
        // Convert to trigger events
        Ok(issues
            .items
            .into_iter()
            .map(|issue| TriggerEvent::with_dedup(
                IssueEvent::from(issue.clone()),
                format!("issue-{}", issue.number),
            ))
            .collect())
    }
}
```

### **Example 3: Webhook Listener Resource**

```rust
use nebula_resource::prelude::*;
use axum::{Router, routing::post};

pub struct WebhookListenerResource;

impl Resource for WebhookListenerResource {
    type Config = WebhookListenerConfig;
    type Instance = WebhookListener;

    fn id(&self) -> &str {
        "github-webhook-listener"
    }

    async fn create(&self, config: &Self::Config, _ctx: &Context) -> Result<Self::Instance> {
        // 1. Create channel
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        // 2. Build router
        let app = Router::new()
            .route("/*path", post(handle_webhook))
            .with_state(WebhookState {
                secret: config.secret.clone(),
                event_tx: tx,
            });

        // 3. Start server
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Ok(WebhookListener {
            server_handle: server,
            event_rx: rx,
        })
    }

    async fn cleanup(&self, instance: Self::Instance) -> Result<()> {
        instance.server_handle.abort();
        Ok(())
    }
}

pub struct WebhookListener {
    server_handle: tokio::task::JoinHandle<()>,
    event_rx: tokio::sync::mpsc::UnboundedReceiver<WebhookEvent>,
}

impl WebhookListener {
    pub async fn recv(&mut self) -> Option<WebhookEvent> {
        self.event_rx.recv().await
    }
}
```

---

## ✅ Benefits

| Feature | Benefit |
|---------|---------|
| **Specialized Traits** | Type safety: can't call `poll()` on webhook trigger |
| **Resource Listeners** | Automatic lifecycle management via `nebula-resource` |
| **Channel-based** | Decoupling + buffering + backpressure |
| **Reusability** | Multiple triggers share one listener resource |
| **Testability** | Easy to mock listeners and inject events |

---

## 🎯 Implementation Order

1. ✅ **Define base traits** (`Trigger`, `PollTrigger`, `WebhookTrigger`)
2. ✅ **Build WebhookListenerResource** (axum HTTP server)
3. ✅ **Implement GithubWebhookTrigger**
4. ✅ **Add signature verification** (HMAC-SHA256)
5. ✅ **Build PollListenerResource** (tokio polling loop)
6. ✅ **Implement GithubIssuePollTrigger**
7. ✅ **Add tests** (unit, integration, e2e)
8. ✅ **Document usage**

---

## 📚 See Also

- `ARCHITECTURE.md` - Detailed design document
- `TRIGGERS.md` - Original trigger analysis (n8n comparison)
- `crates/resource/` - Resource trait documentation
- `crates/action/` - Action and trigger base traits
