# Shared Trigger Resources Architecture - Quick Guide

## 🎯 Core Idea

**One trigger resource → Many workflows**

```
GitHub Repo "octocat/Hello-World"
         ↓
   ONE Resource Instance
   - Webhook registered once
   - Runs HTTP server
   - Has credential
         ↓
   Emits to Kafka/RabbitMQ
         ↓
   3 workflows listening
   - Workflow A (push events)
   - Workflow B (issue events)
   - Workflow C (push to main)
```

---

## 📊 Architecture

```
┌──────────────────────────────────────────────────────────┐
│              WORKFLOW DEFINITIONS                         │
│                                                           │
│  Workflow A                 Workflow B                   │
│   trigger:                   trigger:                    │
│     type: github-webhook      type: github-webhook      │
│     config:                   config:                    │
│       owner: octocat            owner: octocat          │
│       repo: Hello-World         repo: Hello-World       │
│       events: [push]            events: [issues]        │
│     credential: github-pat   credential: github-pat     │
│                                                           │
│  ✅ SAME config hash → Share resource!                   │
└─────────────┬────────────────────────────────────────────┘
              │ activate()
              │
   ┌──────────▼──────────────────────────────────┐
   │    TRIGGER RESOURCE MANAGER                 │
   │                                             │
   │  acquire(config, credential, workflow_id)  │
   │    ↓                                        │
   │  1. Hash config                            │
   │  2. Check if resource exists               │
   │     - YES: Reuse instance                  │
   │     - NO: Create new instance              │
   │  3. Subscribe workflow to trigger_id       │
   │  4. Return trigger_id                      │
   │                                             │
   │  Map: (type, config_hash) → resource       │
   │  Map: trigger_id → [wf_a, wf_b, wf_c]    │
   └─────────────┬───────────────────────────────┘
                 │ create()
                 │
   ┌─────────────▼───────────────────────────────┐
   │  GITHUB WEBHOOK RESOURCE INSTANCE          │
   │                                             │
   │  Fields:                                   │
   │   - trigger_id: "github-webhook-xxx"       │
   │   - config: { owner, repo, events }        │
   │   - credential: GithubToken               │
   │   - webhook_id: "12345" (from GitHub)     │
   │   - webhook_secret: "abc..." (generated)  │
   │   - http_server: AxumServer (running)     │
   │   - emitter: EventEmitter<Event>          │
   │                                             │
   │  Lifecycle:                                │
   │   create()  → Register webhook with GitHub │
   │   cleanup() → Delete webhook from GitHub   │
   │                                             │
   │  On webhook POST:                          │
   │   1. Verify HMAC signature                 │
   │   2. Parse event                           │
   │   3. emitter.emit(event) → Kafka          │
   └─────────────┬───────────────────────────────┘
                 │ emit()
                 │
   ┌─────────────▼───────────────────────────────┐
   │         EVENT EMITTER                       │
   │                                             │
   │  emitter.emit(TriggerEvent) →              │
   │    {                                        │
   │      trigger_id: "github-webhook-xxx",     │
   │      event_type: "push",                   │
   │      data: { ... },                        │
   │      timestamp: "...",                     │
   │      dedup_key: "delivery-123"             │
   │    }                                        │
   └─────────────┬───────────────────────────────┘
                 │ publish()
                 │
   ┌─────────────▼───────────────────────────────┐
   │       EVENT BUS (Kafka/RabbitMQ)           │
   │                                             │
   │  Topic: "triggers.github-webhook-xxx"      │
   │                                             │
   │  Subscribers:                              │
   │   - Engine Consumer A (Workflow A)         │
   │   - Engine Consumer B (Workflow B)         │
   │   - Engine Consumer C (Workflow C)         │
   └─────────────┬───────────────────────────────┘
                 │ consume()
                 │
   ┌─────────────▼───────────────────────────────┐
   │         NEBULA ENGINE                       │
   │                                             │
   │  For each event:                           │
   │    1. Check workflow filters               │
   │       Workflow A: event = "push" ✅        │
   │       Workflow B: event = "issues" ❌      │
   │       Workflow C: event = "push" + branch ✅│
   │                                             │
   │    2. Execute matching workflows           │
   │       - Create execution context           │
   │       - Run first action                   │
   │       - Continue workflow                  │
   └─────────────────────────────────────────────┘
```

---

## 🔧 Key Components

### **1. TriggerResource Trait**

```rust
#[async_trait]
pub trait TriggerResource: Resource {
    type Config: Config + Clone + Hash;
    type Event: Serialize + Send + Sync;
    type Credential: Send + Sync;
    
    // Subscribe to external service
    async fn subscribe(
        config: &Config,
        credential: &Credential,
        ctx: &Context,
    ) -> Result<SubscriptionInfo>;
    
    // Unsubscribe from external service
    async fn unsubscribe(
        config: &Config,
        credential: &Credential,
        subscription: &SubscriptionInfo,
        ctx: &Context,
    ) -> Result<()>;
}

// Resource instance
pub struct TriggerResourceInstance<E> {
    pub trigger_id: String,
    pub subscription: SubscriptionInfo,
    pub emitter: EventEmitter<E>,        // ← Sends to Kafka
    pub task_handle: JoinHandle<()>,     // ← HTTP server / poll loop
}
```

### **2. EventEmitter**

```rust
pub struct EventEmitter<E> {
    trigger_id: String,
    event_tx: mpsc::UnboundedSender<TriggerEventMessage>,
}

impl<E: Serialize> EventEmitter<E> {
    pub async fn emit(&self, event: TriggerEvent<E>) -> Result<()> {
        let message = TriggerEventMessage {
            trigger_id: self.trigger_id.clone(),
            event_type: type_name::<E>(),
            data: to_value(&event.data)?,
            timestamp: event.timestamp,
            dedup_key: event.dedup_key,
        };
        
        self.event_tx.send(message)?;  // → Kafka/RabbitMQ
        Ok(())
    }
}
```

### **3. TriggerResourceManager**

```rust
pub struct TriggerResourceManager {
    // (resource_type, config_hash) → resource instance
    resources: DashMap<(String, u64), Arc<dyn Any>>,
    
    // trigger_id → [workflow_ids]
    subscriptions: DashMap<String, Vec<WorkflowId>>,
    
    // Channel to event bus
    event_bus_tx: mpsc::UnboundedSender<TriggerEventMessage>,
}

impl TriggerResourceManager {
    // Acquire or reuse resource
    pub async fn acquire<R: TriggerResource>(
        &self,
        config: &R::Config,
        credential: &R::Credential,
        workflow_id: WorkflowId,
    ) -> Result<String> {
        let key = (type_name::<R>(), hash(config));
        
        if !self.resources.contains_key(&key) {
            // Create new resource
            let resource = R::default();
            let instance = resource.create(config, ctx).await?;
            self.resources.insert(key, Arc::new(instance));
        }
        
        let trigger_id = /* get from instance */;
        
        // Subscribe workflow
        self.subscriptions
            .entry(trigger_id.clone())
            .or_default()
            .push(workflow_id);
        
        Ok(trigger_id)
    }
    
    // Release resource
    pub async fn release(
        &self,
        trigger_id: &str,
        workflow_id: WorkflowId,
    ) -> Result<()> {
        // Remove workflow from subscriptions
        if let Some(mut workflows) = self.subscriptions.get_mut(trigger_id) {
            workflows.retain(|&id| id != workflow_id);
            
            // If no more workflows, cleanup resource
            if workflows.is_empty() {
                self.cleanup_resource(trigger_id).await?;
            }
        }
        Ok(())
    }
}
```

---

## 🎨 Complete Example: GitHub Webhook

```rust
pub struct GithubWebhookResource;

#[derive(Clone, Hash, Eq, PartialEq)]
pub struct GithubWebhookConfig {
    pub owner: String,
    pub repo: String,
    pub events: Vec<String>,
}

impl Resource for GithubWebhookResource {
    type Config = GithubWebhookConfig;
    type Instance = TriggerResourceInstance<GithubEvent>;

    async fn create(&self, config: &Config, ctx: &Context) -> Result<Instance> {
        // 1. Get credential from context
        let credential = ctx.get_credential::<GithubToken>()?;
        
        // 2. Create GitHub client
        let client = Octocrab::builder()
            .personal_token(credential.token.clone())
            .build()?;
        
        // 3. Generate webhook secret + URL
        let secret = generate_webhook_secret();
        let trigger_id = format!("github-webhook-{}-{}", config.owner, config.repo);
        let webhook_url = format!("{}/webhooks/{}", ctx.base_url, trigger_id);
        
        // 4. Register webhook with GitHub API
        let response = client
            ._post(format!("/repos/{}/{}/hooks", config.owner, config.repo), Some(&json!({
                "config": {
                    "url": webhook_url,
                    "secret": secret,
                    "content_type": "json",
                },
                "events": config.events,
                "active": true,
            })))
            .await?;
        
        let webhook_id = response["id"].as_i64().unwrap().to_string();
        
        // 5. Create event emitter
        let (event_tx, _rx) = mpsc::unbounded_channel();
        let emitter = EventEmitter::new(trigger_id.clone(), event_tx);
        
        // 6. Start HTTP server
        let server_emitter = emitter.clone();
        let server_config = config.clone();
        let server_secret = secret.clone();
        
        let task_handle = tokio::spawn(async move {
            let app = Router::new()
                .route("/*path", post(move |req| {
                    handle_webhook(req, server_config, server_secret, server_emitter)
                }));
            
            let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
            axum::serve(listener, app).await.unwrap();
        });
        
        // 7. Create subscription info
        let subscription = SubscriptionInfo::new(webhook_id, secret)
            .with_url(webhook_url);
        
        Ok(TriggerResourceInstance {
            trigger_id,
            subscription,
            emitter,
            task_handle,
        })
    }

    async fn cleanup(&self, instance: Instance) -> Result<()> {
        // Stop HTTP server
        instance.task_handle.abort();
        
        // Delete webhook from GitHub
        // (Would need credential access here)
        
        Ok(())
    }
}

async fn handle_webhook(
    req: Request,
    config: GithubWebhookConfig,
    secret: String,
    emitter: EventEmitter<GithubEvent>,
) -> StatusCode {
    // 1. Verify signature
    let signature = req.headers().get("x-hub-signature-256")?;
    let raw_body = to_bytes(req.into_body()).await?;
    
    if !verify_signature(&secret, &raw_body, signature) {
        return StatusCode::UNAUTHORIZED;
    }
    
    // 2. Parse event
    let body: Value = serde_json::from_slice(&raw_body)?;
    let event_type = req.headers().get("x-github-event")?.to_str()?.to_string();
    
    // 3. Filter by configured events
    if !config.events.contains(&event_type) {
        return StatusCode::OK; // Ignore
    }
    
    // 4. Emit to event bus
    let event = GithubEvent { event_type, payload: body };
    let dedup_key = req.headers().get("x-github-delivery")?.to_str()?.to_string();
    
    emitter.emit(TriggerEvent::with_dedup(event, dedup_key)).await?;
    
    StatusCode::OK
}
```

---

## 🔄 Lifecycle Flows

### **Activate 3 Workflows with Same Trigger**

```
Engine: activate_workflow(wf_a)
  ↓
ResourceManager.acquire(config, cred, wf_a)
  ↓
  Hash config → "abc123"
  Check resources[(GithubWebhook, abc123)] → NOT FOUND
  ↓
  Create GithubWebhookResource instance
    - Call GitHub API: POST /repos/octocat/Hello-World/hooks
    - Start HTTP server on port 8080
    - Return trigger_id: "github-webhook-octocat-Hello-World"
  ↓
  Store: resources[(GithubWebhook, abc123)] = instance
  Store: subscriptions[trigger_id] = [wf_a]
  ↓
  Return trigger_id

---

Engine: activate_workflow(wf_b)
  ↓
ResourceManager.acquire(config, cred, wf_b)
  ↓
  Hash config → "abc123" (SAME!)
  Check resources[(GithubWebhook, abc123)] → FOUND ✅
  ↓
  Reuse existing instance (no API call, no new server)
  ↓
  Store: subscriptions[trigger_id] = [wf_a, wf_b]
  ↓
  Return trigger_id

---

Engine: activate_workflow(wf_c)
  ↓
ResourceManager.acquire(config, cred, wf_c)
  ↓
  Hash config → "abc123" (SAME!)
  Reuse existing instance ✅
  ↓
  Store: subscriptions[trigger_id] = [wf_a, wf_b, wf_c]
  ↓
  Return trigger_id

---

RESULT: One resource, three workflows subscribed
```

### **Deactivate Workflows**

```
Engine: deactivate_workflow(wf_a)
  ↓
ResourceManager.release(trigger_id, wf_a)
  ↓
  subscriptions[trigger_id] = [wf_b, wf_c]
  Still 2 workflows → Keep resource alive ✅

---

Engine: deactivate_workflow(wf_b)
  ↓
ResourceManager.release(trigger_id, wf_b)
  ↓
  subscriptions[trigger_id] = [wf_c]
  Still 1 workflow → Keep resource alive ✅

---

Engine: deactivate_workflow(wf_c)
  ↓
ResourceManager.release(trigger_id, wf_c)
  ↓
  subscriptions[trigger_id] = []
  No more workflows → Cleanup resource ❌
    - Abort HTTP server
    - DELETE /repos/octocat/Hello-World/hooks/12345
    - Remove from resources map

RESULT: Resource cleaned up
```

---

## 📡 Event Flow

```
1. GitHub sends webhook:
   POST /webhooks/github-webhook-octocat-Hello-World
   
2. HTTP server receives, verifies, parses:
   event_type: "push"
   
3. Emitter sends to event bus:
   emitter.emit(event) →
     Kafka topic: "triggers.github-webhook-octocat-Hello-World"
     Message: {
       trigger_id: "github-webhook-octocat-Hello-World",
       event_type: "push",
       data: {...}
     }

4. Engine consumes from Kafka:
   3 consumers subscribed to topic
   
5. Each consumer applies workflow filters:
   Workflow A: filter { event: "push" } → ✅ Execute
   Workflow B: filter { event: "issues" } → ❌ Skip
   Workflow C: filter { event: "push", branch: "main" } → ✅ Execute

6. Execute workflows A and C
```

---

## ✅ Key Benefits

| Benefit | Description |
|---------|-------------|
| **Resource Sharing** | N workflows → 1 webhook registration |
| **API Rate Limits** | Single resource respects limits |
| **Credential Reuse** | Stored once with resource |
| **Scalability** | Event bus handles distribution |
| **Decoupling** | Trigger resources ↔ workflow execution |
| **Persistence** | Events in Kafka survive restarts |

---

## 🚀 Implementation Plan

1. ✅ Define `TriggerResource` trait
2. ✅ Implement `EventEmitter` with Kafka adapter
3. ✅ Build `TriggerResourceManager` with ref counting
4. ✅ Create `GithubWebhookResource`
5. ✅ Create `GithubIssuePollResource`
6. ✅ Integrate with Engine activation/deactivation
7. ✅ Add workflow filtering
8. ✅ Test with multiple workflows

---

## 📚 Files

- `SHARED_RESOURCES.md` - Full architecture (this file)
- `UNIVERSAL_TRIGGERS.md` - Original lifecycle hooks design
- `ARCHITECTURE.md` - Resource-based listener pattern
- `TRIGGERS.md` - n8n comparison
