# Universal Trigger System - Quick Reference

## 🎯 Design Philosophy

**Problem**: Different external services (GitHub, Slack, Stripe) need different lifecycle operations (subscribe webhooks, test connections, manage state).

**Solution**: Specialized traits with lifecycle hooks that any plugin can implement.

---

## 📊 Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                    External Services                             │
│  GitHub   │   Slack   │  Stripe  │  Discord  │  Custom          │
└────┬──────┴─────┬─────┴────┬─────┴─────┬─────┴─────┬────────────┘
     │            │          │           │           │
     │ Register   │ Register │ Register  │ Register  │ Register
     │ Webhook    │ Event    │ Webhook   │ Webhook   │ Webhook
     │            │ Sub      │           │           │
     ▼            ▼          ▼           ▼           ▼
┌────────────────────────────────────────────────────────────────┐
│             WebhookTrigger Implementations                      │
│                                                                 │
│  - subscribe(config, url) → SubscriptionInfo                   │
│  - unsubscribe(config, subscription) → ()                      │
│  - test(config, subscription) → TestResult                     │
│  - verify_signature(config, request) → bool                    │
│  - handle(config, request) → TriggerEvent                      │
└─────────────────────┬──────────────────────────────────────────┘
                      │
     ┌────────────────▼────────────────┐
     │  WebhookListenerResource        │
     │  (Universal HTTP Server)        │
     │                                 │
     │  - Axum router                  │
     │  - Dynamic route registration   │
     │  - Signature verification       │
     │  - Error handling               │
     └────────────────┬────────────────┘
                      │ TriggerEvent<T>
                      ▼
     ┌────────────────────────────────┐
     │       Nebula Engine            │
     │  - Route events to workflows   │
     │  - Manage trigger lifecycle    │
     │  - Handle deduplication        │
     └────────────────────────────────┘


┌─────────────────────────────────────────────────────────────────┐
│                    External APIs                                 │
│  GitHub   │  GitLab  │   Jira   │  Linear   │  Custom           │
└────┬──────┴─────┬────┴────┬─────┴─────┬─────┴─────┬─────────────┘
     │ Poll       │ Poll    │ Poll      │ Poll      │ Poll
     │ /issues    │ /events │ /tickets  │ /issues   │ /api
     ▼            ▼         ▼           ▼           ▼
┌────────────────────────────────────────────────────────────────┐
│             PollTrigger Implementations                         │
│                                                                 │
│  - interval(config) → Duration                                 │
│  - initialize(config) → State                                  │
│  - test(config) → TestResult                                   │
│  - poll(config, state) → PollResult<Event, State>             │
│  - cleanup(config, state) → ()                                 │
└─────────────────────┬──────────────────────────────────────────┘
                      │
     ┌────────────────▼────────────────┐
     │  PollSchedulerResource          │
     │  (Tokio Interval Scheduler)     │
     │                                 │
     │  - Interval timers              │
     │  - State persistence            │
     │  - Rate limiting                │
     │  - Error handling               │
     └────────────────┬────────────────┘
                      │ Vec<TriggerEvent<T>>
                      ▼
     ┌────────────────────────────────┐
     │       Nebula Engine            │
     │  - Execute workflows           │
     │  - Track cursor state          │
     │  - Handle errors               │
     └────────────────────────────────┘
```

---

## 🔧 Trait Definitions

### **Base Trait**

```rust
pub trait Trigger: Action {
    type Config: Send + Sync + 'static;
    type Event: Send + Sync + 'static;
    type State: Send + Sync + 'static + Default;
}
```

### **WebhookTrigger** (subscribe/unsubscribe/test/verify/handle)

```rust
#[async_trait]
pub trait WebhookTrigger: Trigger {
    /// Register webhook with external service
    async fn subscribe(
        &self,
        config: &Self::Config,
        webhook_url: &str,
        ctx: &ActionContext,
    ) -> Result<SubscriptionInfo, ActionError>;
    
    /// Deregister webhook from external service
    async fn unsubscribe(
        &self,
        config: &Self::Config,
        subscription: &SubscriptionInfo,
        ctx: &ActionContext,
    ) -> Result<(), ActionError>;
    
    /// Test webhook configuration (optional)
    async fn test(
        &self,
        config: &Self::Config,
        subscription: &SubscriptionInfo,
        ctx: &ActionContext,
    ) -> Result<TestResult, ActionError> {
        Ok(TestResult::Skipped)
    }
    
    /// Verify webhook signature (optional)
    async fn verify_signature(
        &self,
        config: &Self::Config,
        request: &WebhookRequest,
    ) -> Result<bool, ActionError> {
        Ok(true)
    }
    
    /// Handle incoming webhook
    async fn handle(
        &self,
        config: &Self::Config,
        request: WebhookRequest,
        ctx: &ActionContext,
    ) -> Result<TriggerEvent<Self::Event>, ActionError>;
}
```

### **PollTrigger** (initialize/poll/test/cleanup)

```rust
#[async_trait]
pub trait PollTrigger: Trigger {
    /// Polling interval
    fn interval(&self, config: &Self::Config) -> Duration;
    
    /// Initialize polling state (optional)
    async fn initialize(
        &self,
        config: &Self::Config,
        ctx: &ActionContext,
    ) -> Result<Self::State, ActionError> {
        Ok(Self::State::default())
    }
    
    /// Test connection (optional)
    async fn test(
        &self,
        config: &Self::Config,
        ctx: &ActionContext,
    ) -> Result<TestResult, ActionError> {
        Ok(TestResult::Skipped)
    }
    
    /// Poll for events
    async fn poll(
        &self,
        config: &Self::Config,
        state: &Self::State,
        ctx: &ActionContext,
    ) -> Result<PollResult<Self::Event, Self::State>, ActionError>;
    
    /// Clean up resources (optional)
    async fn cleanup(
        &self,
        config: &Self::Config,
        state: &Self::State,
        ctx: &ActionContext,
    ) -> Result<(), ActionError> {
        Ok(())
    }
}
```

---

## 📦 Core Types

### **SubscriptionInfo**

```rust
pub struct SubscriptionInfo {
    pub webhook_id: String,        // External webhook ID
    pub secret: String,             // For signature verification
    pub webhook_url: String,        // Registered URL
    pub created_at: DateTime<Utc>,
    pub metadata: HashMap<String, Value>,
}
```

### **TestResult**

```rust
pub enum TestResult {
    Success { message: String },
    Failed { reason: String },
    Warning { message: String, warnings: Vec<String> },
    Skipped,
}
```

### **PollResult**

```rust
pub struct PollResult<E, S> {
    pub events: Vec<TriggerEvent<E>>,
    pub next_state: S,
    pub metadata: PollMetadata,
}

pub struct PollMetadata {
    pub items_scanned: usize,
    pub items_filtered: usize,
    pub rate_limit_remaining: Option<usize>,
    pub rate_limit_reset: Option<DateTime<Utc>>,
}
```

### **WebhookRequest** (enhanced)

```rust
pub struct WebhookRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Value,
    pub raw_body: Option<Vec<u8>>,      // For signature verification
    pub query: HashMap<String, String>,
    pub remote_addr: Option<String>,
}
```

---

## 🎨 Usage Example: GitHub Webhook

```rust
impl WebhookTrigger for GithubWebhookTrigger {
    async fn subscribe(&self, config: &Config, url: &str, ctx: &Ctx) 
        -> Result<SubscriptionInfo> 
    {
        let client = get_github_client(&config.credential).await?;
        let secret = generate_webhook_secret();
        
        let webhook = client
            .post(format!("/repos/{}/{}/hooks", config.owner, config.repo), Some(&json!({
                "config": { "url": url, "secret": secret },
                "events": config.events,
            })))
            .await?;
        
        Ok(SubscriptionInfo::new(webhook["id"].to_string(), secret))
    }
    
    async fn unsubscribe(&self, config: &Config, sub: &SubscriptionInfo, ctx: &Ctx) 
        -> Result<()> 
    {
        let client = get_github_client(&config.credential).await?;
        client
            .delete(format!("/repos/{}/{}/hooks/{}", config.owner, config.repo, sub.webhook_id))
            .await?;
        Ok(())
    }
    
    async fn test(&self, config: &Config, sub: &SubscriptionInfo, ctx: &Ctx) 
        -> Result<TestResult> 
    {
        let client = get_github_client(&config.credential).await?;
        match client
            .post(format!("/repos/{}/{}/hooks/{}/tests", config.owner, config.repo, sub.webhook_id))
            .await 
        {
            Ok(_) => Ok(TestResult::success("Ping sent successfully")),
            Err(e) => Ok(TestResult::failed(format!("Ping failed: {e}"))),
        }
    }
    
    async fn verify_signature(&self, config: &Config, req: &WebhookRequest) 
        -> Result<bool> 
    {
        let signature = req.header("x-hub-signature-256")?;
        let secret = &config.webhook_secret;
        let raw_body = req.raw_body.as_ref()?;
        
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
        mac.update(raw_body);
        let computed = hex::encode(mac.finalize().into_bytes());
        
        Ok(constant_time_compare(&computed, &signature[7..])) // Skip "sha256="
    }
    
    async fn handle(&self, config: &Config, req: WebhookRequest, ctx: &Ctx) 
        -> Result<TriggerEvent<Event>> 
    {
        let event_type = req.header("x-github-event")?;
        
        if !config.events.contains(&event_type) {
            return Err(ActionError::ignored("event not subscribed"));
        }
        
        Ok(TriggerEvent::with_dedup(
            GithubEvent { event_type, payload: req.body },
            req.header("x-github-delivery")?,
        ))
    }
}
```

---

## 🎨 Usage Example: GitHub Poll

```rust
#[derive(Default)]
pub struct PollState {
    since: Option<DateTime<Utc>>,
    last_issue_id: Option<u64>,
}

impl PollTrigger for GithubIssuePollTrigger {
    fn interval(&self, config: &Config) -> Duration {
        Duration::from_secs(config.interval_seconds)
    }
    
    async fn initialize(&self, config: &Config, ctx: &Ctx) -> Result<PollState> {
        let client = get_github_client(&config.credential).await?;
        
        // Validate access
        client.repos(&config.owner, &config.repo).get().await?;
        
        Ok(PollState {
            since: Some(Utc::now()),
            last_issue_id: None,
        })
    }
    
    async fn test(&self, config: &Config, ctx: &Ctx) -> Result<TestResult> {
        let client = get_github_client(&config.credential).await?;
        
        match client.repos(&config.owner, &config.repo).get().await {
            Ok(repo) => Ok(TestResult::success(format!("Connected: {}", repo.full_name))),
            Err(e) => Ok(TestResult::failed(format!("Failed: {e}"))),
        }
    }
    
    async fn poll(&self, config: &Config, state: &PollState, ctx: &Ctx) 
        -> Result<PollResult<Event, PollState>> 
    {
        let client = get_github_client(&config.credential).await?;
        
        let issues = client
            .issues(&config.owner, &config.repo)
            .list()
            .since(state.since)
            .send()
            .await?;
        
        let events: Vec<_> = issues
            .items
            .into_iter()
            .filter(|i| state.last_issue_id.map_or(true, |last| i.number > last))
            .map(|issue| TriggerEvent::with_dedup(
                IssueEvent::from(issue.clone()),
                format!("issue-{}", issue.number),
            ))
            .collect();
        
        let next_state = PollState {
            since: Some(Utc::now()),
            last_issue_id: events.last().map(|e| e.data.number),
        };
        
        Ok(PollResult::new(events, next_state))
    }
}
```

---

## 🔄 Lifecycle Flows

### **Webhook Activation**

```
User activates workflow
       ↓
Engine generates webhook_url
       ↓
trigger.subscribe(config, webhook_url, ctx)
       ↓
Store SubscriptionInfo in workflow state
       ↓
Register route in WebhookListener
       ↓
✅ Webhook active
```

### **Webhook Deactivation**

```
User deactivates workflow
       ↓
Load SubscriptionInfo from state
       ↓
Unregister route from WebhookListener
       ↓
trigger.unsubscribe(config, subscription, ctx)
       ↓
Clear workflow state
       ↓
✅ Webhook removed
```

### **Poll Activation**

```
User activates workflow
       ↓
trigger.initialize(config, ctx) → initial_state
       ↓
Store state in workflow state
       ↓
Register with PollScheduler
       ↓
✅ Polling active
       ↓
Every interval:
  trigger.poll(config, state, ctx) → PollResult
       ↓
  Update state in workflow state
       ↓
  Execute workflows for each event
```

### **Poll Deactivation**

```
User deactivates workflow
       ↓
Load state from workflow state
       ↓
Unregister from PollScheduler
       ↓
trigger.cleanup(config, state, ctx)
       ↓
Clear workflow state
       ↓
✅ Polling stopped
```

---

## ✅ Benefits

| Feature | Description |
|---------|-------------|
| **Universal** | Works for any service (GitHub, Slack, Stripe, custom) |
| **Lifecycle** | subscribe/unsubscribe automatically managed |
| **Testable** | `test()` validates config before activation |
| **Secure** | Built-in signature verification |
| **Stateful** | Poll triggers maintain cursor between polls |
| **Metadata** | Rich context (rate limits, scan counts) |
| **Type Safe** | Can't mix webhook and poll methods |

---

## 🚀 Plugin Developer Guide

### **Step 1: Define Config & Event Types**

```rust
#[derive(Serialize, Deserialize)]
pub struct MyWebhookConfig {
    pub api_key: String,
    pub events: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct MyEvent {
    pub event_type: String,
    pub data: Value,
}
```

### **Step 2: Implement Trigger Trait**

```rust
pub struct MyWebhookTrigger;

impl Trigger for MyWebhookTrigger {
    type Config = MyWebhookConfig;
    type Event = MyEvent;
    type State = ();
}
```

### **Step 3: Implement WebhookTrigger**

```rust
#[async_trait]
impl WebhookTrigger for MyWebhookTrigger {
    async fn subscribe(...) -> Result<SubscriptionInfo> {
        // Call your API to register webhook
    }
    
    async fn unsubscribe(...) -> Result<()> {
        // Call your API to delete webhook
    }
    
    async fn handle(...) -> Result<TriggerEvent<Event>> {
        // Parse webhook payload
    }
}
```

### **Step 4: Register with Engine**

```rust
engine.register_trigger(MyWebhookTrigger);
```

---

## 📚 See Also

- `UNIVERSAL_TRIGGERS.md` - Full documentation with examples
- `ARCHITECTURE.md` - Resource-based listener design
- `TRIGGERS.md` - Original n8n analysis
- `crates/action/src/types/trigger.rs` - Trait definitions
