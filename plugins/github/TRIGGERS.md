# GitHub Triggers Architecture

## Overview

GitHub triggers enable workflows to react to GitHub events via webhooks. This document describes the architecture, implementation strategy, and integration with Nebula's trigger system.

---

## 🏗️ Architecture

### **Nebula Trigger System**

Nebula provides a built-in trigger infrastructure in `nebula-action`:

```rust
pub trait TriggerAction: Action {
    type Config: Send + Sync + 'static;
    type Event: Send + Sync + 'static;

    fn kind(&self, config: &Self::Config) -> TriggerKind;
    
    async fn poll(
        &self,
        config: &Self::Config,
        last_state: Option<serde_json::Value>,
        ctx: &ActionContext,
    ) -> Result<Vec<TriggerEvent<Self::Event>>, ActionError>;
    
    async fn handle_webhook(
        &self,
        config: &Self::Config,
        request: WebhookRequest,
        ctx: &ActionContext,
    ) -> Result<TriggerEvent<Self::Event>, ActionError>;
}
```

### **Trigger Types**

```rust
pub enum TriggerKind {
    Poll { interval: Duration },         // Poll GitHub API
    Webhook { path: String },            // Receive webhooks
    Cron { expression: String },         // Scheduled checks
}
```

---

## 📋 n8n GitHub Trigger Analysis

### **Workflow**

1. **Activation** (when workflow is enabled):
   - Generate secure random webhook secret (32 bytes)
   - Register webhook with GitHub API:
     ```http
     POST /repos/{owner}/{repo}/hooks
     {
       "name": "web",
       "config": {
         "url": "{nebula_webhook_url}",
         "content_type": "json",
         "secret": "{generated_secret}",
         "insecure_ssl": "0"
       },
       "events": ["push", "issues", ...],
       "active": true
     }
     ```
   - Store webhook ID and secret in workflow state

2. **Webhook Reception**:
   - GitHub sends POST request with signature
   - Verify HMAC-SHA256 signature:
     ```
     X-Hub-Signature-256: sha256={signature}
     ```
   - Parse event type from `X-GitHub-Event` header
   - Forward to workflow

3. **Deactivation** (when workflow is disabled):
   - Delete webhook via GitHub API:
     ```http
     DELETE /repos/{owner}/{repo}/hooks/{webhook_id}
     ```
   - Clear stored webhook ID and secret

### **Signature Verification**

```typescript
function verifySignature(webhookSecret: string, rawBody: Buffer, signature: string): boolean {
  const hmac = createHmac('sha256', webhookSecret);
  hmac.update(rawBody);
  const computed = hmac.digest('hex');
  
  return timingSafeEqual(
    Buffer.from(computed, 'utf8'),
    Buffer.from(signature.substring(7), 'utf8') // Remove "sha256=" prefix
  );
}
```

---

## 🔧 Implementation for Nebula

### **Directory Structure**

```
plugins/github/
├── src/
│   ├── triggers/
│   │   ├── mod.rs                    # Exports all triggers
│   │   ├── github_webhook.rs         # Main webhook trigger
│   │   ├── issue_events.rs           # Poll-based issue trigger
│   │   └── release_events.rs         # Poll-based release trigger
│   ├── types/
│   │   └── webhook_event.rs          # GitHub webhook event types
│   └── utils/
│       ├── signature.rs              # HMAC-SHA256 verification
│       └── webhook_manager.rs        # GitHub webhook CRUD
```

---

## 📝 Implementation Example

### **1. GitHub Webhook Trigger** (`triggers/github_webhook.rs`)

```rust
use async_trait::async_trait;
use nebula_sdk::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// GitHub webhook trigger - listens for GitHub events via webhooks.
#[derive(Debug)]
pub struct GithubWebhookTrigger {
    meta: ActionMetadata,
}

impl GithubWebhookTrigger {
    pub fn new() -> Self {
        Self {
            meta: ActionMetadata::new(
                "github-webhook",
                "GitHub Webhook",
                "Trigger workflows on GitHub events"
            ),
        }
    }
}

impl Action for GithubWebhookTrigger {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }
    
    fn action_type(&self) -> ActionType {
        ActionType::Trigger
    }
}

#[async_trait]
impl TriggerAction for GithubWebhookTrigger {
    type Config = GithubWebhookConfig;
    type Event = GithubWebhookEvent;

    fn kind(&self, _config: &Self::Config) -> TriggerKind {
        TriggerKind::Webhook {
            path: "/github-webhook".to_string(),
        }
    }

    async fn handle_webhook(
        &self,
        config: &Self::Config,
        request: WebhookRequest,
        ctx: &ActionContext,
    ) -> Result<TriggerEvent<Self::Event>, ActionError> {
        // 1. Verify signature
        verify_github_signature(
            &config.webhook_secret,
            &request.headers,
            &request.body,
        )?;

        // 2. Extract event type
        let event_type = request
            .headers
            .get("x-github-event")
            .ok_or_else(|| ActionError::validation("missing X-GitHub-Event header"))?;

        // 3. Filter by configured events
        if !config.events.contains(event_type) {
            return Err(ActionError::fatal(format!(
                "event {event_type} not in configured events"
            )));
        }

        // 4. Parse webhook payload
        let event = GithubWebhookEvent {
            event_type: event_type.clone(),
            action: request.body.get("action")
                .and_then(|v| v.as_str())
                .map(String::from),
            payload: request.body,
            delivery_id: request.headers.get("x-github-delivery").cloned(),
        };

        // 5. Create deduplication key
        let dedup_key = event.delivery_id.clone()
            .unwrap_or_else(|| format!("{}:{}", event_type, chrono::Utc::now().timestamp()));

        Ok(TriggerEvent::with_dedup(event, dedup_key))
    }
}

/// Configuration for GitHub webhook trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubWebhookConfig {
    /// Repository owner
    pub owner: String,
    /// Repository name
    pub repo: String,
    /// Events to subscribe to (e.g. ["push", "issues", "pull_request"])
    pub events: Vec<String>,
    /// Webhook secret (generated during activation)
    pub webhook_secret: String,
    /// GitHub credentials (for registering webhook)
    pub credential: CredentialRef,
}

/// GitHub webhook event payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubWebhookEvent {
    /// Event type (e.g. "push", "issues", "pull_request")
    pub event_type: String,
    /// Action (e.g. "opened", "closed", "synchronize")
    pub action: Option<String>,
    /// Full webhook payload
    pub payload: serde_json::Value,
    /// Unique delivery ID from GitHub
    pub delivery_id: Option<String>,
}
```

### **2. Signature Verification** (`utils/signature.rs`)

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use nebula_action::error::ActionError;

type HmacSha256 = Hmac<Sha256>;

/// Verify GitHub webhook signature using HMAC-SHA256.
///
/// GitHub sends signature in `X-Hub-Signature-256` header:
/// ```text
/// X-Hub-Signature-256: sha256={hex_digest}
/// ```
pub fn verify_github_signature(
    secret: &str,
    headers: &HashMap<String, String>,
    body: &serde_json::Value,
) -> Result<(), ActionError> {
    // 1. Get signature from header
    let signature_header = headers
        .get("x-hub-signature-256")
        .ok_or_else(|| ActionError::validation("missing X-Hub-Signature-256 header"))?;

    // 2. Validate format
    if !signature_header.starts_with("sha256=") {
        return Err(ActionError::validation("invalid signature format"));
    }

    let provided_signature = &signature_header[7..]; // Remove "sha256=" prefix

    // 3. Serialize body back to bytes (GitHub signs raw JSON)
    let body_bytes = serde_json::to_vec(body)
        .map_err(|e| ActionError::fatal(format!("failed to serialize body: {e}")))?;

    // 4. Compute HMAC
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|e| ActionError::fatal(format!("HMAC initialization failed: {e}")))?;
    mac.update(&body_bytes);
    
    let computed = hex::encode(mac.finalize().into_bytes());

    // 5. Constant-time comparison
    if !constant_time_compare(&computed, provided_signature) {
        return Err(ActionError::validation("signature verification failed"));
    }

    Ok(())
}

/// Constant-time string comparison to prevent timing attacks.
fn constant_time_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }

    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_valid_signature() {
        let secret = "test-secret";
        let body = serde_json::json!({"event": "push"});
        
        // Compute expected signature
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(&body_bytes);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        
        let mut headers = HashMap::new();
        headers.insert("x-hub-signature-256".to_string(), signature);
        
        assert!(verify_github_signature(secret, &headers, &body).is_ok());
    }

    #[test]
    fn reject_invalid_signature() {
        let secret = "test-secret";
        let body = serde_json::json!({"event": "push"});
        
        let mut headers = HashMap::new();
        headers.insert("x-hub-signature-256".to_string(), "sha256=invalid".to_string());
        
        assert!(verify_github_signature(secret, &headers, &body).is_err());
    }

    #[test]
    fn constant_time_compare_works() {
        assert!(constant_time_compare("abc", "abc"));
        assert!(!constant_time_compare("abc", "abd"));
        assert!(!constant_time_compare("abc", "ab"));
    }
}
```

### **3. Webhook Lifecycle Management** (`utils/webhook_manager.rs`)

```rust
use octocrab::Octocrab;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("GitHub API error: {0}")]
    Api(String),
    #[error("Webhook already exists")]
    AlreadyExists,
    #[error("Webhook not found")]
    NotFound,
}

/// GitHub webhook configuration.
#[derive(Debug, Serialize, Deserialize)]
pub struct WebhookConfig {
    pub url: String,
    pub secret: String,
    pub events: Vec<String>,
    pub insecure_ssl: bool,
}

/// Webhook manager - handles GitHub webhook CRUD operations.
pub struct WebhookManager {
    client: Octocrab,
}

impl WebhookManager {
    pub fn new(client: Octocrab) -> Self {
        Self { client }
    }

    /// Create a new webhook.
    pub async fn create(
        &self,
        owner: &str,
        repo: &str,
        config: WebhookConfig,
    ) -> Result<i64, WebhookError> {
        let endpoint = format!("/repos/{owner}/{repo}/hooks");
        
        let payload = serde_json::json!({
            "name": "web",
            "config": {
                "url": config.url,
                "content_type": "json",
                "secret": config.secret,
                "insecure_ssl": if config.insecure_ssl { "1" } else { "0" },
            },
            "events": config.events,
            "active": true,
        });

        let response: serde_json::Value = self.client
            ._post(endpoint, Some(&payload))
            .await
            .map_err(|e| WebhookError::Api(e.to_string()))?;

        response["id"]
            .as_i64()
            .ok_or_else(|| WebhookError::Api("missing webhook id in response".into()))
    }

    /// Check if webhook exists.
    pub async fn exists(
        &self,
        owner: &str,
        repo: &str,
        webhook_id: i64,
    ) -> Result<bool, WebhookError> {
        let endpoint = format!("/repos/{owner}/{repo}/hooks/{webhook_id}");
        
        match self.client._get::<serde_json::Value>(endpoint, None::<&()>).await {
            Ok(_) => Ok(true),
            Err(e) if e.to_string().contains("404") => Ok(false),
            Err(e) => Err(WebhookError::Api(e.to_string())),
        }
    }

    /// Delete a webhook.
    pub async fn delete(
        &self,
        owner: &str,
        repo: &str,
        webhook_id: i64,
    ) -> Result<(), WebhookError> {
        let endpoint = format!("/repos/{owner}/{repo}/hooks/{webhook_id}");
        
        self.client
            ._delete(endpoint, None::<&()>)
            .await
            .map_err(|e| {
                if e.to_string().contains("404") {
                    WebhookError::NotFound
                } else {
                    WebhookError::Api(e.to_string())
                }
            })
    }

    /// Generate a secure random webhook secret.
    pub fn generate_secret() -> String {
        use rand::Rng;
        let bytes: [u8; 32] = rand::thread_rng().gen();
        hex::encode(bytes)
    }
}
```

---

## 🔄 Lifecycle Integration

### **Workflow Activation** (when trigger is enabled)

```rust
// 1. Generate webhook secret
let secret = WebhookManager::generate_secret();

// 2. Register webhook with GitHub
let webhook_id = webhook_manager.create(
    &config.owner,
    &config.repo,
    WebhookConfig {
        url: format!("{}/webhooks/github-webhook", nebula_base_url),
        secret: secret.clone(),
        events: config.events.clone(),
        insecure_ssl: false,
    }
).await?;

// 3. Store webhook ID and secret in workflow state
workflow_state.set("webhook_id", webhook_id);
workflow_state.set("webhook_secret", secret);
```

### **Webhook Reception** (when GitHub sends event)

```rust
// Engine receives HTTP POST at /webhooks/github-webhook

// 1. Forward to trigger handler
let result = trigger_adapter.execute(
    serde_json::json!({
        "op": "webhook",
        "config": {
            "webhook_secret": workflow_state.get("webhook_secret"),
            "events": ["push", "issues"],
            ...
        },
        "request": {
            "method": "POST",
            "path": "/webhooks/github-webhook",
            "headers": {
                "x-github-event": "push",
                "x-hub-signature-256": "sha256=...",
                "x-github-delivery": "uuid-...",
            },
            "body": { /* GitHub webhook payload */ }
        }
    }),
    action_ctx
).await?;

// 2. Start workflow execution with event data
executor.start_workflow(workflow_id, result.output).await?;
```

### **Workflow Deactivation** (when trigger is disabled)

```rust
// 1. Get webhook ID from workflow state
let webhook_id = workflow_state.get("webhook_id")?;

// 2. Delete webhook from GitHub
webhook_manager.delete(&config.owner, &config.repo, webhook_id).await?;

// 3. Clear workflow state
workflow_state.remove("webhook_id");
workflow_state.remove("webhook_secret");
```

---

## 🎯 Supported GitHub Events

Based on n8n, we should support **all 44 webhook events**:

| Category | Events |
|----------|--------|
| **Repository** | push, fork, create, delete, repository, repository_import |
| **Issues** | issues, issue_comment, label, milestone |
| **Pull Requests** | pull_request, pull_request_review, pull_request_review_comment |
| **Releases** | release |
| **Deployments** | deployment, deployment_status, deploy_key |
| **Checks** | check_run, check_suite, status |
| **Projects** | project, project_card, project_column |
| **Team** | team, team_add, member, membership, organization, org_block |
| **Wiki** | gollum (wiki pages) |
| **Stars** | star, watch |
| **Security** | repository_vulnerability_alert, security_advisory |
| **Pages** | page_build |
| **Meta** | meta (webhook deleted) |
| **Marketplace** | marketplace_purchase |
| **GitHub Apps** | installation, installation_repositories, github_app_authorization |
| **Commit** | commit_comment |

### **Event Filtering**

Users configure which events to listen for:

```json
{
  "events": [
    "push",
    "pull_request",
    "issues"
  ]
}
```

Or use wildcard `"*"` for all events (⚠️ high volume).

---

## 🧪 Testing Strategy

### **Unit Tests**

```rust
#[tokio::test]
async fn valid_signature_passes() {
    let secret = "test-secret";
    let body = serde_json::json!({"action": "opened"});
    let signature = compute_signature(secret, &body);
    
    let mut headers = HashMap::new();
    headers.insert("x-hub-signature-256".into(), signature);
    
    assert!(verify_github_signature(secret, &headers, &body).is_ok());
}

#[tokio::test]
async fn invalid_signature_rejected() {
    let secret = "test-secret";
    let body = serde_json::json!({"action": "opened"});
    
    let mut headers = HashMap::new();
    headers.insert("x-hub-signature-256".into(), "sha256=invalid".into());
    
    assert!(verify_github_signature(secret, &headers, &body).is_err());
}
```

### **Integration Tests**

```rust
#[tokio::test]
async fn webhook_lifecycle() {
    let client = test_github_client().await;
    let manager = WebhookManager::new(client);
    
    // Create webhook
    let webhook_id = manager.create(
        "test-org",
        "test-repo",
        WebhookConfig {
            url: "https://nebula.example.com/webhooks/test".into(),
            secret: "secret".into(),
            events: vec!["push".into()],
            insecure_ssl: false,
        }
    ).await.unwrap();
    
    // Verify exists
    assert!(manager.exists("test-org", "test-repo", webhook_id).await.unwrap());
    
    // Delete
    manager.delete("test-org", "test-repo", webhook_id).await.unwrap();
    
    // Verify deleted
    assert!(!manager.exists("test-org", "test-repo", webhook_id).await.unwrap());
}
```

---

## 🔐 Security Considerations

### **1. Signature Verification**

- ✅ **MUST** verify `X-Hub-Signature-256` on every webhook
- ✅ Use constant-time comparison to prevent timing attacks
- ✅ Reject webhooks with missing or invalid signatures

### **2. Secret Management**

- ✅ Generate cryptographically secure random secrets (32 bytes)
- ✅ Store secrets encrypted in workflow state
- ✅ Never log webhook secrets

### **3. Rate Limiting**

- ⚠️ GitHub can send high volumes for `*` (wildcard) events
- ✅ Implement rate limiting per repository
- ✅ Use `dedup_key` to avoid duplicate processing

### **4. Permissions**

- ⚠️ Webhook creation requires **admin** access to repository
- ✅ Validate credentials have sufficient permissions
- ✅ Graceful error messages for permission errors

---

## 📚 Dependencies

Add to `plugins/github/Cargo.toml`:

```toml
[dependencies]
# Existing
octocrab = { version = "0.49", default-features = false, features = ["default-client", "rustls", "retry", "jwt-rust-crypto"] }

# New for triggers
hmac = { workspace = true }
sha2 = { workspace = true }
hex = "0.4"
rand = { workspace = true }
```

---

## 🚀 Rollout Plan

### **Phase 1: Core Webhook Trigger**
- ✅ Implement `GithubWebhookTrigger`
- ✅ Signature verification (`utils/signature.rs`)
- ✅ Webhook manager (`utils/webhook_manager.rs`)
- ✅ Unit tests

### **Phase 2: Event Types**
- ✅ Define typed event structs (`types/webhook_event.rs`)
- ✅ Support top 10 events (push, PR, issues, release, etc.)
- ✅ Event filtering

### **Phase 3: Poll-Based Triggers** (alternative to webhooks)
- ✅ `IssueEventsTrigger` (poll for new issues)
- ✅ `ReleaseEventsTrigger` (poll for new releases)
- ✅ Cursor-based pagination state

### **Phase 4: Advanced Features**
- ✅ Webhook validation (test endpoint)
- ✅ Webhook health monitoring
- ✅ Replay protection (deduplication)

---

## 📖 Resources

- [GitHub Webhooks Documentation](https://docs.github.com/en/webhooks)
- [GitHub Webhook Events](https://docs.github.com/en/webhooks/webhook-events-and-payloads)
- [Securing Webhooks](https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries)
- [n8n GitHub Trigger Source](https://github.com/n8n-io/n8n/blob/master/packages/nodes-base/nodes/Github/GithubTrigger.node.ts)
