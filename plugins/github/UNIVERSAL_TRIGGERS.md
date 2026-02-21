# Universal Trigger Infrastructure for Nebula

## Overview

This document defines a **universal, extensible trigger system** that can be used by any plugin (GitHub, GitLab, Slack, Discord, etc.) with proper lifecycle hooks.

---

## 🎯 Design Goals

1. **Universal**: Works for any external service (GitHub, Slack, Stripe, etc.)
2. **Lifecycle Hooks**: Subscribe, unsubscribe, test, handle, validate
3. **Type Safe**: Compile-time guarantees about trigger capabilities
4. **Composable**: Triggers can share infrastructure (listeners, clients)
5. **Testable**: Easy to mock and test each lifecycle phase

---

## 🏗️ Core Architecture

### **Trait Hierarchy**

```rust
/// Base trait for all triggers
pub trait Trigger: Action {
    type Config: Send + Sync + 'static;
    type Event: Send + Sync + 'static;
    type State: Send + Sync + 'static + Default;  // ← Persistent state
}

/// Trigger that receives webhook notifications
#[async_trait]
pub trait WebhookTrigger: Trigger {
    /// Subscribe to webhook (register with external service)
    /// 
    /// Called when workflow is activated. Should:
    /// - Register webhook with external API (GitHub, Slack, etc.)
    /// - Return subscription info (webhook ID, URL, etc.)
    /// - Store necessary state for later unsubscribe
    async fn subscribe(
        &self,
        config: &Self::Config,
        webhook_url: &str,
        ctx: &ActionContext,
    ) -> Result<SubscriptionInfo, ActionError>;
    
    /// Unsubscribe from webhook (deregister from external service)
    /// 
    /// Called when workflow is deactivated. Should:
    /// - Delete webhook from external API
    /// - Clean up any stored state
    async fn unsubscribe(
        &self,
        config: &Self::Config,
        subscription: &SubscriptionInfo,
        ctx: &ActionContext,
    ) -> Result<(), ActionError>;
    
    /// Test webhook connection (optional)
    /// 
    /// Called to verify webhook is working. Should:
    /// - Trigger test event from external API
    /// - OR validate webhook configuration
    /// - Return validation result
    async fn test(
        &self,
        config: &Self::Config,
        subscription: &SubscriptionInfo,
        ctx: &ActionContext,
    ) -> Result<TestResult, ActionError> {
        Ok(TestResult::Skipped)
    }
    
    /// Handle incoming webhook request
    /// 
    /// Called when webhook is received. Should:
    /// - Verify signature/authenticity
    /// - Parse event payload
    /// - Return trigger event
    async fn handle(
        &self,
        config: &Self::Config,
        request: WebhookRequest,
        ctx: &ActionContext,
    ) -> Result<TriggerEvent<Self::Event>, ActionError>;
    
    /// Verify webhook signature (optional, for additional security)
    /// 
    /// Called before handle(). Default implementation returns true.
    /// Override to implement signature verification.
    async fn verify_signature(
        &self,
        config: &Self::Config,
        request: &WebhookRequest,
    ) -> Result<bool, ActionError> {
        Ok(true)
    }
}

/// Trigger that polls an external API for events
#[async_trait]
pub trait PollTrigger: Trigger {
    /// Get polling interval
    fn interval(&self, config: &Self::Config) -> Duration;
    
    /// Initialize polling (optional)
    /// 
    /// Called when workflow is activated. Should:
    /// - Validate credentials
    /// - Initialize API client
    /// - Return initial state (cursor, timestamp, etc.)
    async fn initialize(
        &self,
        config: &Self::Config,
        ctx: &ActionContext,
    ) -> Result<Self::State, ActionError> {
        Ok(Self::State::default())
    }
    
    /// Clean up polling (optional)
    /// 
    /// Called when workflow is deactivated. Should:
    /// - Close connections
    /// - Clean up resources
    async fn cleanup(
        &self,
        config: &Self::Config,
        state: &Self::State,
        ctx: &ActionContext,
    ) -> Result<(), ActionError> {
        Ok(())
    }
    
    /// Test polling connection (optional)
    /// 
    /// Called to verify polling works. Should:
    /// - Make test API request
    /// - Validate credentials
    /// - Return validation result
    async fn test(
        &self,
        config: &Self::Config,
        ctx: &ActionContext,
    ) -> Result<TestResult, ActionError> {
        Ok(TestResult::Skipped)
    }
    
    /// Poll for new events
    /// 
    /// Called at each interval. Should:
    /// - Query external API with state cursor
    /// - Filter new events since last poll
    /// - Return events + updated state
    async fn poll(
        &self,
        config: &Self::Config,
        state: &Self::State,
        ctx: &ActionContext,
    ) -> Result<PollResult<Self::Event, Self::State>, ActionError>;
}
```

---

## 📦 Core Types

### **SubscriptionInfo** (for WebhookTrigger)

```rust
/// Information about an active webhook subscription
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionInfo {
    /// Webhook ID from external service (e.g. GitHub webhook ID)
    pub webhook_id: String,
    
    /// Webhook secret for signature verification
    pub secret: String,
    
    /// Full webhook URL registered with service
    pub webhook_url: String,
    
    /// When subscription was created
    pub created_at: DateTime<Utc>,
    
    /// Additional provider-specific metadata
    #[serde(flatten)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl SubscriptionInfo {
    pub fn new(webhook_id: impl Into<String>, secret: impl Into<String>) -> Self {
        Self {
            webhook_id: webhook_id.into(),
            secret: secret.into(),
            webhook_url: String::new(),
            created_at: Utc::now(),
            metadata: HashMap::new(),
        }
    }
    
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.webhook_url = url.into();
        self
    }
    
    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }
}
```

### **TestResult**

```rust
/// Result of testing a trigger connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TestResult {
    /// Test passed successfully
    Success {
        message: String,
    },
    
    /// Test failed with error
    Failed {
        reason: String,
    },
    
    /// Test was skipped (not implemented)
    Skipped,
    
    /// Test passed with warnings
    Warning {
        message: String,
        warnings: Vec<String>,
    },
}

impl TestResult {
    pub fn success(msg: impl Into<String>) -> Self {
        Self::Success { message: msg.into() }
    }
    
    pub fn failed(reason: impl Into<String>) -> Self {
        Self::Failed { reason: reason.into() }
    }
    
    pub fn warning(msg: impl Into<String>, warnings: Vec<String>) -> Self {
        Self::Warning {
            message: msg.into(),
            warnings,
        }
    }
}
```

### **PollResult**

```rust
/// Result of a poll operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollResult<E, S> {
    /// Events discovered in this poll
    pub events: Vec<TriggerEvent<E>>,
    
    /// Updated state for next poll (cursor, timestamp, etc.)
    pub next_state: S,
    
    /// Optional metadata about the poll
    pub metadata: PollMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PollMetadata {
    /// How many items were examined
    pub items_scanned: usize,
    
    /// How many events were filtered out
    pub items_filtered: usize,
    
    /// Rate limit remaining (if applicable)
    pub rate_limit_remaining: Option<usize>,
    
    /// When rate limit resets
    pub rate_limit_reset: Option<DateTime<Utc>>,
}

impl<E, S> PollResult<E, S> {
    pub fn new(events: Vec<TriggerEvent<E>>, next_state: S) -> Self {
        Self {
            events,
            next_state,
            metadata: PollMetadata::default(),
        }
    }
    
    pub fn with_metadata(mut self, metadata: PollMetadata) -> Self {
        self.metadata = metadata;
        self
    }
}
```

### **WebhookRequest** (enhanced)

```rust
/// Incoming webhook request forwarded to trigger
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookRequest {
    /// HTTP method
    pub method: String,
    
    /// Request path
    pub path: String,
    
    /// HTTP headers
    pub headers: HashMap<String, String>,
    
    /// Parsed request body
    pub body: serde_json::Value,
    
    /// Raw request body (for signature verification)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_body: Option<Vec<u8>>,
    
    /// Query parameters
    #[serde(default)]
    pub query: HashMap<String, String>,
    
    /// Remote IP address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_addr: Option<String>,
}

impl WebhookRequest {
    /// Get header value (case-insensitive)
    pub fn header(&self, name: &str) -> Option<&str> {
        let name_lower = name.to_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| k.to_lowercase() == name_lower)
            .map(|(_, v)| v.as_str())
    }
    
    /// Get query parameter
    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query.get(name).map(String::as_str)
    }
}
```

---

## 🎨 Example: GitHub Webhook Trigger

### **Complete Implementation**

```rust
use nebula_sdk::prelude::*;
use octocrab::Octocrab;

pub struct GithubWebhookTrigger {
    meta: ActionMetadata,
}

impl Action for GithubWebhookTrigger {
    fn metadata(&self) -> &ActionMetadata { &self.meta }
    fn action_type(&self) -> ActionType { ActionType::Trigger }
}

impl Trigger for GithubWebhookTrigger {
    type Config = GithubWebhookConfig;
    type Event = GithubWebhookEvent;
    type State = (); // Webhooks are stateless
}

#[async_trait]
impl WebhookTrigger for GithubWebhookTrigger {
    async fn subscribe(
        &self,
        config: &Self::Config,
        webhook_url: &str,
        ctx: &ActionContext,
    ) -> Result<SubscriptionInfo, ActionError> {
        // 1. Get GitHub client
        let client = get_github_client(&config.credential, ctx).await?;
        
        // 2. Generate secure webhook secret
        let secret = generate_webhook_secret();
        
        // 3. Register webhook with GitHub API
        let webhook = client
            .post(
                format!("/repos/{}/{}/hooks", config.owner, config.repo),
                Some(&serde_json::json!({
                    "name": "web",
                    "config": {
                        "url": webhook_url,
                        "content_type": "json",
                        "secret": secret,
                        "insecure_ssl": "0",
                    },
                    "events": config.events,
                    "active": true,
                }))
            )
            .await
            .map_err(|e| ActionError::external(format!("failed to create webhook: {e}")))?;
        
        // 4. Return subscription info
        Ok(SubscriptionInfo::new(
            webhook["id"].as_i64().unwrap().to_string(),
            secret,
        )
        .with_url(webhook_url)
        .with_metadata("repo", serde_json::json!({
            "owner": config.owner,
            "repo": config.repo,
        })))
    }
    
    async fn unsubscribe(
        &self,
        config: &Self::Config,
        subscription: &SubscriptionInfo,
        ctx: &ActionContext,
    ) -> Result<(), ActionError> {
        let client = get_github_client(&config.credential, ctx).await?;
        
        client
            .delete(
                format!(
                    "/repos/{}/{}/hooks/{}",
                    config.owner,
                    config.repo,
                    subscription.webhook_id
                ),
                None::<&()>
            )
            .await
            .map_err(|e| ActionError::external(format!("failed to delete webhook: {e}")))?;
        
        Ok(())
    }
    
    async fn test(
        &self,
        config: &Self::Config,
        subscription: &SubscriptionInfo,
        ctx: &ActionContext,
    ) -> Result<TestResult, ActionError> {
        let client = get_github_client(&config.credential, ctx).await?;
        
        // Test by sending a ping to the webhook
        match client
            .post(
                format!(
                    "/repos/{}/{}/hooks/{}/tests",
                    config.owner,
                    config.repo,
                    subscription.webhook_id
                ),
                None::<&()>
            )
            .await
        {
            Ok(_) => Ok(TestResult::success("Webhook test ping sent successfully")),
            Err(e) => Ok(TestResult::failed(format!("Test ping failed: {e}"))),
        }
    }
    
    async fn verify_signature(
        &self,
        config: &Self::Config,
        request: &WebhookRequest,
    ) -> Result<bool, ActionError> {
        let signature = request
            .header("x-hub-signature-256")
            .ok_or_else(|| ActionError::validation("missing X-Hub-Signature-256 header"))?;
        
        if !signature.starts_with("sha256=") {
            return Ok(false);
        }
        
        let provided = &signature[7..];
        
        let raw_body = request
            .raw_body
            .as_ref()
            .ok_or_else(|| ActionError::validation("missing raw body for signature verification"))?;
        
        // Get secret from subscription (passed via config or context)
        let secret = &config.webhook_secret;
        
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .map_err(|e| ActionError::fatal(format!("HMAC init failed: {e}")))?;
        mac.update(raw_body);
        let computed = hex::encode(mac.finalize().into_bytes());
        
        Ok(constant_time_compare(&computed, provided))
    }
    
    async fn handle(
        &self,
        config: &Self::Config,
        request: WebhookRequest,
        ctx: &ActionContext,
    ) -> Result<TriggerEvent<Self::Event>, ActionError> {
        // 1. Extract event type
        let event_type = request
            .header("x-github-event")
            .ok_or_else(|| ActionError::validation("missing X-GitHub-Event header"))?
            .to_string();
        
        // 2. Filter by configured events
        if !config.events.contains(&event_type) {
            return Err(ActionError::ignored(format!(
                "event '{event_type}' not in configured events"
            )));
        }
        
        // 3. Parse event
        let action = request.body.get("action")
            .and_then(|v| v.as_str())
            .map(String::from);
        
        let event = GithubWebhookEvent {
            event_type: event_type.clone(),
            action,
            payload: request.body,
        };
        
        // 4. Create deduplication key
        let dedup_key = request
            .header("x-github-delivery")
            .map(String::from)
            .unwrap_or_else(|| format!("{}:{}", event_type, Utc::now().timestamp()));
        
        Ok(TriggerEvent::with_dedup(event, dedup_key))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubWebhookConfig {
    pub owner: String,
    pub repo: String,
    pub events: Vec<String>,
    pub credential: CredentialRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>, // Set after subscribe()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubWebhookEvent {
    pub event_type: String,
    pub action: Option<String>,
    pub payload: serde_json::Value,
}
```

---

## 🎨 Example: GitHub Poll Trigger

```rust
pub struct GithubIssuePollTrigger {
    meta: ActionMetadata,
}

impl Trigger for GithubIssuePollTrigger {
    type Config = IssueConfig;
    type Event = IssueEvent;
    type State = PollState;
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PollState {
    /// Last updated timestamp
    pub since: Option<DateTime<Utc>>,
    
    /// Last seen issue ID (for deduplication)
    pub last_issue_id: Option<u64>,
}

#[async_trait]
impl PollTrigger for GithubIssuePollTrigger {
    fn interval(&self, config: &Self::Config) -> Duration {
        Duration::from_secs(config.poll_interval_seconds)
    }
    
    async fn initialize(
        &self,
        config: &Self::Config,
        ctx: &ActionContext,
    ) -> Result<Self::State, ActionError> {
        // Validate credentials by making test request
        let client = get_github_client(&config.credential, ctx).await?;
        
        client
            .repos(&config.owner, &config.repo)
            .get()
            .await
            .map_err(|e| ActionError::validation(format!("repository access failed: {e}")))?;
        
        Ok(PollState {
            since: Some(Utc::now()),
            last_issue_id: None,
        })
    }
    
    async fn test(
        &self,
        config: &Self::Config,
        ctx: &ActionContext,
    ) -> Result<TestResult, ActionError> {
        let client = get_github_client(&config.credential, ctx).await?;
        
        match client.repos(&config.owner, &config.repo).get().await {
            Ok(repo) => Ok(TestResult::success(format!(
                "Connected to repository: {}",
                repo.full_name.unwrap_or_default()
            ))),
            Err(e) => Ok(TestResult::failed(format!("Connection failed: {e}"))),
        }
    }
    
    async fn poll(
        &self,
        config: &Self::Config,
        state: &Self::State,
        ctx: &ActionContext,
    ) -> Result<PollResult<Self::Event, Self::State>, ActionError> {
        let client = get_github_client(&config.credential, ctx).await?;
        
        // Query issues since last poll
        let mut query = client
            .issues(&config.owner, &config.repo)
            .list()
            .state(octocrab::params::State::All)
            .per_page(100);
        
        if let Some(since) = state.since {
            query = query.since(since);
        }
        
        let page = query
            .send()
            .await
            .map_err(|e| ActionError::external(format!("failed to fetch issues: {e}")))?;
        
        // Filter and convert to events
        let mut events = Vec::new();
        let mut max_updated = state.since;
        
        for issue in page.items {
            // Skip if we've seen this issue ID
            if let Some(last_id) = state.last_issue_id {
                if issue.number <= last_id {
                    continue;
                }
            }
            
            // Track latest timestamp
            if max_updated.is_none() || Some(issue.updated_at) > max_updated {
                max_updated = Some(issue.updated_at);
            }
            
            events.push(TriggerEvent::with_dedup(
                IssueEvent::from(issue.clone()),
                format!("issue-{}", issue.number),
            ));
        }
        
        let next_state = PollState {
            since: max_updated,
            last_issue_id: page.items.last().map(|i| i.number),
        };
        
        Ok(PollResult::new(events, next_state).with_metadata(PollMetadata {
            items_scanned: page.items.len(),
            items_filtered: page.items.len() - events.len(),
            rate_limit_remaining: None, // Could extract from GitHub API headers
            rate_limit_reset: None,
        }))
    }
    
    async fn cleanup(
        &self,
        config: &Self::Config,
        state: &Self::State,
        ctx: &ActionContext,
    ) -> Result<(), ActionError> {
        // No cleanup needed for polling
        Ok(())
    }
}
```

---

## 🔄 Engine Integration

### **Workflow Activation Flow**

```rust
// When user activates workflow with trigger

match trigger_type {
    TriggerType::Webhook(webhook_trigger) => {
        // 1. Generate webhook URL
        let webhook_url = format!(
            "{}/webhooks/{}/{}",
            base_url,
            workflow_id,
            trigger_id
        );
        
        // 2. Call subscribe()
        let subscription = webhook_trigger
            .subscribe(&config, &webhook_url, &ctx)
            .await?;
        
        // 3. Store subscription info in workflow state
        workflow_state.set("subscription", subscription);
        
        // 4. Register webhook route in listener
        webhook_listener.register_route(
            &webhook_url,
            webhook_trigger.clone(),
            config.clone(),
        );
        
        tracing::info!(
            workflow_id = %workflow_id,
            webhook_id = %subscription.webhook_id,
            "webhook subscribed"
        );
    }
    
    TriggerType::Poll(poll_trigger) => {
        // 1. Initialize polling
        let initial_state = poll_trigger
            .initialize(&config, &ctx)
            .await?;
        
        // 2. Store state
        workflow_state.set("poll_state", initial_state);
        
        // 3. Register with poll scheduler
        poll_scheduler.register(
            workflow_id,
            poll_trigger.interval(&config),
            poll_trigger.clone(),
            config.clone(),
        );
        
        tracing::info!(
            workflow_id = %workflow_id,
            interval = ?poll_trigger.interval(&config),
            "poll trigger initialized"
        );
    }
}
```

### **Workflow Deactivation Flow**

```rust
match trigger_type {
    TriggerType::Webhook(webhook_trigger) => {
        // 1. Get subscription info
        let subscription: SubscriptionInfo = workflow_state.get("subscription")?;
        
        // 2. Unregister route
        webhook_listener.unregister_route(&subscription.webhook_url);
        
        // 3. Call unsubscribe()
        webhook_trigger
            .unsubscribe(&config, &subscription, &ctx)
            .await?;
        
        // 4. Clear state
        workflow_state.remove("subscription");
        
        tracing::info!(
            workflow_id = %workflow_id,
            webhook_id = %subscription.webhook_id,
            "webhook unsubscribed"
        );
    }
    
    TriggerType::Poll(poll_trigger) => {
        // 1. Unregister from scheduler
        poll_scheduler.unregister(workflow_id);
        
        // 2. Get state
        let state: PollState = workflow_state.get("poll_state")?;
        
        // 3. Call cleanup()
        poll_trigger
            .cleanup(&config, &state, &ctx)
            .await?;
        
        // 4. Clear state
        workflow_state.remove("poll_state");
        
        tracing::info!(
            workflow_id = %workflow_id,
            "poll trigger cleaned up"
        );
    }
}
```

### **Test Trigger Flow**

```rust
// When user tests trigger configuration

match trigger_type {
    TriggerType::Webhook(webhook_trigger) => {
        // Option 1: Test before subscription (validate config)
        let test_result = webhook_trigger
            .test(&config, &dummy_subscription, &ctx)
            .await?;
        
        match test_result {
            TestResult::Success { message } => {
                return Ok(TestResponse::ok(message));
            }
            TestResult::Failed { reason } => {
                return Err(TestResponse::error(reason));
            }
            TestResult::Warning { message, warnings } => {
                return Ok(TestResponse::warning(message, warnings));
            }
            TestResult::Skipped => {
                // Fallback: temporarily subscribe and unsubscribe
                let subscription = webhook_trigger.subscribe(&config, &temp_url, &ctx).await?;
                let test_result = webhook_trigger.test(&config, &subscription, &ctx).await?;
                webhook_trigger.unsubscribe(&config, &subscription, &ctx).await?;
                
                return Ok(test_result.into());
            }
        }
    }
    
    TriggerType::Poll(poll_trigger) => {
        let test_result = poll_trigger.test(&config, &ctx).await?;
        return Ok(test_result.into());
    }
}
```

---

## 📋 Universal Webhook Listener Resource

```rust
use axum::{Router, routing::post, extract::{State, Path}, Json};
use std::sync::Arc;
use dashmap::DashMap;

/// Configuration for universal webhook listener
#[derive(Clone)]
pub struct WebhookListenerConfig {
    pub port: u16,
    pub base_path: String,
}

impl Config for WebhookListenerConfig {
    fn validate(&self) -> Result<()> {
        if self.port == 0 {
            return Err(Error::validation("port must be non-zero"));
        }
        Ok(())
    }
}

/// Universal webhook listener that routes to registered triggers
pub struct WebhookListenerResource;

impl Resource for WebhookListenerResource {
    type Config = WebhookListenerConfig;
    type Instance = WebhookListener;

    fn id(&self) -> &str {
        "webhook-listener"
    }

    async fn create(&self, config: &Self::Config, ctx: &Context) -> Result<Self::Instance> {
        let routes = Arc::new(DashMap::new());
        
        let app = Router::new()
            .route("/*path", post(handle_webhook))
            .route("/*path", axum::routing::get(handle_webhook))
            .with_state(WebhookState {
                routes: routes.clone(),
            });
        
        let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
        
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        
        Ok(WebhookListener {
            server_handle: server,
            routes,
        })
    }

    async fn cleanup(&self, instance: Self::Instance) -> Result<()> {
        instance.server_handle.abort();
        Ok(())
    }
}

pub struct WebhookListener {
    server_handle: tokio::task::JoinHandle<()>,
    routes: Arc<DashMap<String, WebhookRoute>>,
}

impl WebhookListener {
    /// Register a webhook route
    pub fn register<T: WebhookTrigger + 'static>(
        &self,
        path: String,
        trigger: Arc<T>,
        config: T::Config,
    ) {
        self.routes.insert(
            path,
            WebhookRoute {
                handler: Box::new(move |req, ctx| {
                    let trigger = trigger.clone();
                    let config = config.clone();
                    Box::pin(async move {
                        trigger.handle(&config, req, &ctx).await
                    })
                }),
            },
        );
    }
    
    /// Unregister a webhook route
    pub fn unregister(&self, path: &str) {
        self.routes.remove(path);
    }
}

#[derive(Clone)]
struct WebhookState {
    routes: Arc<DashMap<String, WebhookRoute>>,
}

struct WebhookRoute {
    handler: Box<dyn Fn(WebhookRequest, ActionContext) -> BoxFuture<'static, Result<TriggerEvent<Value>, ActionError>> + Send + Sync>,
}

async fn handle_webhook(
    State(state): State<WebhookState>,
    Path(path): Path<String>,
    headers: axum::http::HeaderMap,
    body: axum::extract::Bytes,
) -> axum::http::StatusCode {
    // 1. Find route
    let route = match state.routes.get(&format!("/{}", path)) {
        Some(r) => r,
        None => return axum::http::StatusCode::NOT_FOUND,
    };
    
    // 2. Parse body
    let json_body: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return axum::http::StatusCode::BAD_REQUEST,
    };
    
    // 3. Build request
    let request = WebhookRequest {
        method: "POST".to_string(),
        path: format!("/{}", path),
        headers: headers.iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect(),
        body: json_body,
        raw_body: Some(body.to_vec()),
        query: HashMap::new(),
        remote_addr: None,
    };
    
    // 4. Call handler
    let ctx = ActionContext::default(); // Get from request metadata
    
    match route.handler(request, ctx).await {
        Ok(_event) => axum::http::StatusCode::OK,
        Err(e) if matches!(e, ActionError::Validation(_)) => axum::http::StatusCode::BAD_REQUEST,
        Err(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
    }
}
```

---

## 🧪 Testing Utilities

### **Mock Webhook Trigger**

```rust
#[cfg(test)]
pub struct MockWebhookTrigger {
    pub subscribe_called: Arc<AtomicBool>,
    pub unsubscribe_called: Arc<AtomicBool>,
    pub handle_called: Arc<AtomicUsize>,
}

#[async_trait]
impl WebhookTrigger for MockWebhookTrigger {
    async fn subscribe(...) -> Result<SubscriptionInfo> {
        self.subscribe_called.store(true, Ordering::SeqCst);
        Ok(SubscriptionInfo::new("test-webhook-123", "test-secret"))
    }
    
    async fn unsubscribe(...) -> Result<()> {
        self.unsubscribe_called.store(true, Ordering::SeqCst);
        Ok(())
    }
    
    async fn handle(...) -> Result<TriggerEvent<Event>> {
        self.handle_called.fetch_add(1, Ordering::SeqCst);
        Ok(TriggerEvent::new(/* mock event */))
    }
}
```

### **Integration Test Example**

```rust
#[tokio::test]
async fn test_webhook_lifecycle() {
    // 1. Create trigger
    let trigger = GithubWebhookTrigger::new();
    let config = test_config();
    let ctx = test_context();
    
    // 2. Subscribe
    let subscription = trigger
        .subscribe(&config, "https://test.com/webhook", &ctx)
        .await
        .unwrap();
    
    assert!(!subscription.webhook_id.is_empty());
    assert!(!subscription.secret.is_empty());
    
    // 3. Test
    let test_result = trigger
        .test(&config, &subscription, &ctx)
        .await
        .unwrap();
    
    assert!(matches!(test_result, TestResult::Success { .. }));
    
    // 4. Handle webhook
    let request = mock_webhook_request();
    let event = trigger
        .handle(&config, request, &ctx)
        .await
        .unwrap();
    
    assert_eq!(event.data.event_type, "push");
    
    // 5. Unsubscribe
    trigger
        .unsubscribe(&config, &subscription, &ctx)
        .await
        .unwrap();
}
```

---

## 📚 Summary

### **Key Features**

| Feature | Benefit |
|---------|---------|
| **subscribe/unsubscribe** | Proper lifecycle management with external services |
| **test** | Validate configuration before activation |
| **verify_signature** | Security-first webhook verification |
| **State management** | Poll triggers maintain cursor between polls |
| **Metadata** | Rich context (rate limits, scan counts, etc.) |
| **Type safe** | Can't call webhook methods on poll trigger |

### **Universal Design**

Works for **any** service:
- ✅ GitHub webhooks + polling
- ✅ GitLab webhooks + polling
- ✅ Slack Events API
- ✅ Discord webhooks
- ✅ Stripe webhooks
- ✅ Twilio webhooks
- ✅ Custom HTTP webhooks

### **Implementation Phases**

1. ✅ Define universal traits in `nebula-action`
2. ✅ Implement `WebhookListenerResource`
3. ✅ Create GitHub webhook trigger
4. ✅ Create GitHub poll trigger
5. ✅ Add tests
6. ✅ Document for other plugins
