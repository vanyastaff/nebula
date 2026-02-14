---
title: Enable Audit Logging
tags: [how-to, audit-logging, security, compliance, observability]
status: published
lang: en
created: 2026-02-03
last_updated: 2026-02-03
audience: [intermediate, advanced, security-engineer]
estimated_reading: 15
priority: P3
---

# Enable Audit Logging

> [!NOTE] Compliance Requirement
> Audit logging is **mandatory** for SOC 2, ISO 27001, HIPAA, and GDPR compliance. All credential access must be logged with user identity, timestamp, and operation result.

## TL;DR

Enable comprehensive audit logging in nebula-credential:
1. Configure structured logging with JSON format
2. Add correlation IDs for distributed tracing
3. Enable event logging for all credential operations
4. Configure log retention and secure storage
5. Set up log monitoring and alerting

All operations (access, rotation, ACL changes) are logged by default with structured metadata.

---

## Prerequisites

- nebula-credential v0.1.0 or higher
- Logging infrastructure (local files, Elasticsearch, CloudWatch, etc.)
- Understanding of [[Getting-Started/Core-Concepts|Core Concepts]]

---

## Step 1: Configure Structured Logging

### Install Logging Dependencies

```toml
[dependencies]
nebula-credential = "0.1.0"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["json", "env-filter"] }
tracing-appender = "0.2"
serde_json = "1.0"
uuid = { version = "1.0", features = ["v4", "serde"] }
```

### Initialize Structured Logger

```rust
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use tracing_appender::rolling;

/// Initialize JSON structured logging
pub fn init_audit_logging() -> Result<(), Box<dyn std::error::Error>> {
    // Create rotating log files (daily rotation)
    let file_appender = rolling::daily("./logs", "audit.log");
    
    // JSON formatter for structured logs
    let json_layer = fmt::layer()
        .json()
        .with_file(true)
        .with_line_number(true)
        .with_thread_ids(true)
        .with_target(true)
        .with_writer(file_appender);
    
    // Console output (human-readable)
    let console_layer = fmt::layer()
        .compact()
        .with_writer(std::io::stdout);
    
    // Environment-based filtering
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,nebula_credential=debug"));
    
    // Combine layers
    tracing_subscriber::registry()
        .with(filter)
        .with(json_layer)
        .with(console_layer)
        .init();
    
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging before any credential operations
    init_audit_logging()?;
    
    tracing::info!("Audit logging initialized");
    
    // Your application code...
    Ok(())
}
```

**Expected Output** (JSON format):
```json
{
  "timestamp": "2026-02-03T10:15:30.123Z",
  "level": "INFO",
  "target": "nebula_credential::audit",
  "fields": {
    "message": "Audit logging initialized"
  },
  "span": {
    "name": "init"
  }
}
```

---

## Step 2: Add Correlation IDs

### Correlation ID Middleware

```rust
use uuid::Uuid;
use std::sync::Arc;
use tokio::sync::RwLock;

thread_local! {
    static CORRELATION_ID: Arc<RwLock<Option<Uuid>>> = Arc::new(RwLock::new(None));
}

/// Set correlation ID for current request
pub async fn set_correlation_id(id: Uuid) {
    CORRELATION_ID.with(|cell| async move {
        *cell.write().await = Some(id);
    }).await;
}

/// Get current correlation ID
pub async fn get_correlation_id() -> Option<Uuid> {
    CORRELATION_ID.with(|cell| async move {
        *cell.read().await
    }).await
}

/// Generate new correlation ID
pub fn generate_correlation_id() -> Uuid {
    Uuid::new_v4()
}

/// Wrapper to execute with correlation ID
pub async fn with_correlation_id<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let correlation_id = generate_correlation_id();
    set_correlation_id(correlation_id).await;
    
    tracing::info!(
        correlation_id = %correlation_id,
        "Request started"
    );
    
    let result = f.await;
    
    tracing::info!(
        correlation_id = %correlation_id,
        "Request completed"
    );
    
    result
}
```

### Use Correlation IDs

```rust
use tracing::instrument;

#[instrument(
    name = "retrieve_credential",
    skip(credential_service),
    fields(
        correlation_id = %get_correlation_id().await.unwrap_or_default(),
        credential_id = %id,
        requester = %requester_id
    )
)]
pub async fn retrieve_credential(
    credential_service: &CredentialService,
    id: &CredentialId,
    requester_id: &OwnerId,
) -> Result<Credential, CredentialError> {
    // Correlation ID automatically included in all logs
    credential_service.retrieve(id, requester_id).await
}
```

**Expected Output**:
```json
{
  "timestamp": "2026-02-03T10:15:31.456Z",
  "level": "INFO",
  "target": "nebula_credential::api",
  "fields": {
    "message": "Retrieving credential",
    "correlation_id": "550e8400-e29b-41d4-a716-446655440000",
    "credential_id": "cred_abc123",
    "requester": "user_xyz789"
  }
}
```

---

## Step 3: Define Audit Events

### Audit Event Types

```rust
use serde::{Serialize, Deserialize};
use chrono::{DateTime, Utc};

/// All auditable events in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type")]
pub enum AuditEvent {
    /// Credential accessed (read)
    CredentialAccessed {
        credential_id: CredentialId,
        requester: OwnerId,
        timestamp: DateTime<Utc>,
        result: AccessResult,
        duration_ms: u64,
        correlation_id: Uuid,
    },
    
    /// Credential created
    CredentialCreated {
        credential_id: CredentialId,
        creator: OwnerId,
        credential_type: CredentialType,
        scope: CredentialScope,
        timestamp: DateTime<Utc>,
        correlation_id: Uuid,
    },
    
    /// Credential updated
    CredentialUpdated {
        credential_id: CredentialId,
        updater: OwnerId,
        fields_changed: Vec<String>,
        timestamp: DateTime<Utc>,
        correlation_id: Uuid,
    },
    
    /// Credential deleted
    CredentialDeleted {
        credential_id: CredentialId,
        deleter: OwnerId,
        timestamp: DateTime<Utc>,
        correlation_id: Uuid,
    },
    
    /// Credential rotated
    CredentialRotated {
        credential_id: CredentialId,
        initiator: OwnerId,
        old_version: Option<String>,
        new_version: String,
        timestamp: DateTime<Utc>,
        result: RotationResult,
        correlation_id: Uuid,
    },
    
    /// Credential refreshed (OAuth2 token, etc.)
    CredentialRefreshed {
        credential_id: CredentialId,
        timestamp: DateTime<Utc>,
        result: RefreshResult,
        correlation_id: Uuid,
    },
    
    /// Authentication attempt
    AuthenticationAttempt {
        requester: OwnerId,
        credential_id: CredentialId,
        timestamp: DateTime<Utc>,
        result: AuthResult,
        ip_address: Option<String>,
        user_agent: Option<String>,
        correlation_id: Uuid,
    },
    
    /// Decryption failed
    DecryptionFailed {
        credential_id: CredentialId,
        requester: OwnerId,
        error: String,
        key_version: KeyVersion,
        timestamp: DateTime<Utc>,
        correlation_id: Uuid,
    },
    
    /// Scope violation (unauthorized access)
    ScopeViolation {
        credential_id: CredentialId,
        requester: OwnerId,
        required_scope: CredentialScope,
        actual_scope: CredentialScope,
        timestamp: DateTime<Utc>,
        ip_address: Option<String>,
        correlation_id: Uuid,
    },
    
    /// Permission denied
    PermissionDenied {
        credential_id: CredentialId,
        requester: OwnerId,
        required_permission: Permission,
        timestamp: DateTime<Utc>,
        correlation_id: Uuid,
    },
    
    /// ACL modified
    AclModified {
        credential_id: CredentialId,
        modifier: OwnerId,
        grantee: OwnerId,
        permission: Permission,
        action: AclAction,  // Grant or Revoke
        timestamp: DateTime<Utc>,
        correlation_id: Uuid,
    },
    
    /// Key rotated
    KeyRotated {
        old_version: KeyVersion,
        new_version: KeyVersion,
        initiator: OwnerId,
        timestamp: DateTime<Utc>,
        correlation_id: Uuid,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AccessResult {
    Success,
    Denied,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RotationResult {
    Success,
    Failed(String),
    Rollback,
}
```

---

## Step 4: Implement Audit Logger

### AuditLogger Implementation

```rust
use tracing::{info, warn, error};

pub struct AuditLogger {
    config: AuditConfig,
}

pub struct AuditConfig {
    pub enabled: bool,
    pub log_successful_access: bool,
    pub log_failed_access: bool,
    pub include_ip_address: bool,
    pub include_user_agent: bool,
    pub retention_days: u32,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            log_successful_access: true,
            log_failed_access: true,
            include_ip_address: true,
            include_user_agent: true,
            retention_days: 365,  // 1 year (compliance requirement)
        }
    }
}

impl AuditLogger {
    pub fn new(config: AuditConfig) -> Self {
        Self { config }
    }
    
    /// Log audit event
    pub async fn log(&self, event: AuditEvent) {
        if !self.config.enabled {
            return;
        }
        
        // Filter based on configuration
        match &event {
            AuditEvent::CredentialAccessed { result, .. } => {
                match result {
                    AccessResult::Success if !self.config.log_successful_access => return,
                    AccessResult::Denied | AccessResult::Error(_) if !self.config.log_failed_access => return,
                    _ => {}
                }
            }
            _ => {}
        }
        
        // Serialize event to JSON
        let json = serde_json::to_string(&event)
            .unwrap_or_else(|e| format!(r#"{{"error":"Serialization failed: {}"}}"#, e));
        
        // Log with appropriate level
        match &event {
            AuditEvent::PermissionDenied { .. }
            | AuditEvent::ScopeViolation { .. }
            | AuditEvent::DecryptionFailed { .. } => {
                warn!(
                    target: "nebula_credential::audit",
                    audit_event = %json,
                    "Security event"
                );
            }
            
            AuditEvent::CredentialDeleted { .. }
            | AuditEvent::KeyRotated { .. }
            | AuditEvent::AclModified { .. } => {
                info!(
                    target: "nebula_credential::audit",
                    audit_event = %json,
                    "Administrative event"
                );
            }
            
            _ => {
                info!(
                    target: "nebula_credential::audit",
                    audit_event = %json,
                    "Audit event"
                );
            }
        }
        
        // Optionally send to external audit system
        self.send_to_external_audit_system(&event).await;
    }
    
    /// Send to external audit system (SIEM, Elasticsearch, etc.)
    async fn send_to_external_audit_system(&self, event: &AuditEvent) {
        // Example: Send to Elasticsearch, Splunk, CloudWatch, etc.
        // Implementation depends on your infrastructure
    }
}
```

---

## Step 5: Integrate Audit Logging

### Credential Service Integration

```rust
pub struct CredentialService {
    storage: Arc<dyn StorageProvider>,
    audit_logger: Arc<AuditLogger>,
    // ... other fields
}

impl CredentialService {
    /// Retrieve credential with audit logging
    #[instrument(skip(self), fields(correlation_id))]
    pub async fn retrieve(
        &self,
        id: &CredentialId,
        requester: &OwnerId,
    ) -> Result<Credential, CredentialError> {
        let start = std::time::Instant::now();
        let correlation_id = get_correlation_id().await.unwrap_or_else(generate_correlation_id);
        
        // Attempt to retrieve credential
        let result = self.retrieve_internal(id, requester).await;
        
        // Log audit event
        let event = AuditEvent::CredentialAccessed {
            credential_id: id.clone(),
            requester: requester.clone(),
            timestamp: Utc::now(),
            result: match &result {
                Ok(_) => AccessResult::Success,
                Err(CredentialError::PermissionDenied) => AccessResult::Denied,
                Err(e) => AccessResult::Error(e.to_string()),
            },
            duration_ms: start.elapsed().as_millis() as u64,
            correlation_id,
        };
        
        self.audit_logger.log(event).await;
        
        result
    }
    
    /// Rotate credential with audit logging
    #[instrument(skip(self))]
    pub async fn rotate(
        &self,
        id: &CredentialId,
        initiator: &OwnerId,
    ) -> Result<Credential, CredentialError> {
        let correlation_id = get_correlation_id().await.unwrap_or_else(generate_correlation_id);
        
        // Get current version
        let old_version = self.storage
            .get(id)
            .await?
            .version
            .map(|v| v.to_string());
        
        // Attempt rotation
        let result = self.rotate_internal(id, initiator).await;
        
        // Log audit event
        let event = AuditEvent::CredentialRotated {
            credential_id: id.clone(),
            initiator: initiator.clone(),
            old_version,
            new_version: match &result {
                Ok(cred) => cred.version.clone().unwrap_or_default().to_string(),
                Err(_) => "N/A".to_string(),
            },
            timestamp: Utc::now(),
            result: match &result {
                Ok(_) => RotationResult::Success,
                Err(e) => RotationResult::Failed(e.to_string()),
            },
            correlation_id,
        };
        
        self.audit_logger.log(event).await;
        
        result
    }
}
```

---

## Step 6: Configure Log Storage

### Local File Storage

```rust
// Already configured in Step 1 with tracing-appender
let file_appender = rolling::daily("./logs", "audit.log");

// Files created:
// ./logs/audit.log.2026-02-03
// ./logs/audit.log.2026-02-04
// ...
```

**Log Rotation**:
- Daily rotation by default
- Configure retention: Keep logs for 365 days (compliance requirement)
- Compress old logs: `gzip audit.log.2026-02-03`

---

### Elasticsearch Integration

```rust
use elasticsearch::{Elasticsearch, http::transport::Transport};
use serde_json::json;

pub struct ElasticsearchAuditLogger {
    client: Elasticsearch,
    index_prefix: String,
}

impl ElasticsearchAuditLogger {
    pub async fn new(url: &str, index_prefix: String) -> Result<Self, Box<dyn std::error::Error>> {
        let transport = Transport::single_node(url)?;
        let client = Elasticsearch::new(transport);
        
        Ok(Self {
            client,
            index_prefix,
        })
    }
    
    pub async fn log_event(&self, event: &AuditEvent) -> Result<(), Box<dyn std::error::Error>> {
        // Use date-based indices for better performance
        let index = format!("{}-{}", self.index_prefix, Utc::now().format("%Y.%m.%d"));
        
        let body = json!({
            "@timestamp": Utc::now(),
            "event": event,
        });
        
        self.client
            .index(elasticsearch::IndexParts::Index(&index))
            .body(body)
            .send()
            .await?;
        
        Ok(())
    }
}
```

**Index Template**:
```json
{
  "index_patterns": ["nebula-audit-*"],
  "settings": {
    "number_of_shards": 3,
    "number_of_replicas": 2,
    "index.lifecycle.name": "audit-retention-policy"
  },
  "mappings": {
    "properties": {
      "@timestamp": { "type": "date" },
      "event.event_type": { "type": "keyword" },
      "event.credential_id": { "type": "keyword" },
      "event.requester": { "type": "keyword" },
      "event.correlation_id": { "type": "keyword" },
      "event.result": { "type": "keyword" }
    }
  }
}
```

---

### AWS CloudWatch Integration

```rust
use aws_sdk_cloudwatch_logs::{Client, types::InputLogEvent};

pub struct CloudWatchAuditLogger {
    client: Client,
    log_group: String,
    log_stream: String,
}

impl CloudWatchAuditLogger {
    pub async fn new(log_group: String, log_stream: String) -> Self {
        let config = aws_config::load_from_env().await;
        let client = Client::new(&config);
        
        Self {
            client,
            log_group,
            log_stream,
        }
    }
    
    pub async fn log_event(&self, event: &AuditEvent) -> Result<(), Box<dyn std::error::Error>> {
        let message = serde_json::to_string(event)?;
        
        let log_event = InputLogEvent::builder()
            .timestamp(Utc::now().timestamp_millis())
            .message(message)
            .build();
        
        self.client
            .put_log_events()
            .log_group_name(&self.log_group)
            .log_stream_name(&self.log_stream)
            .log_events(log_event)
            .send()
            .await?;
        
        Ok(())
    }
}
```

---

## Step 7: Query and Monitor Logs

### Query Examples (Elasticsearch)

**Find all credential access by user**:
```json
GET /nebula-audit-*/_search
{
  "query": {
    "bool": {
      "must": [
        { "term": { "event.event_type": "CredentialAccessed" }},
        { "term": { "event.requester": "user_xyz789" }}
      ]
    }
  },
  "sort": [{ "@timestamp": "desc" }]
}
```

**Find all failed access attempts**:
```json
GET /nebula-audit-*/_search
{
  "query": {
    "bool": {
      "must": [
        { "term": { "event.event_type": "CredentialAccessed" }},
        { "term": { "event.result": "Denied" }}
      ],
      "filter": [
        { "range": { "@timestamp": { "gte": "now-24h" }}}
      ]
    }
  }
}
```

**Trace request by correlation ID**:
```json
GET /nebula-audit-*/_search
{
  "query": {
    "term": { "event.correlation_id": "550e8400-e29b-41d4-a716-446655440000" }
  },
  "sort": [{ "@timestamp": "asc" }]
}
```

---

### Alerting Rules

**Security Alerts**:

1. **Multiple Failed Access Attempts**:
```json
{
  "trigger": {
    "schedule": { "interval": "5m" }
  },
  "input": {
    "search": {
      "request": {
        "indices": ["nebula-audit-*"],
        "body": {
          "query": {
            "bool": {
              "must": [
                { "term": { "event.result": "Denied" }},
                { "range": { "@timestamp": { "gte": "now-5m" }}}
              ]
            }
          },
          "aggs": {
            "by_user": {
              "terms": { "field": "event.requester" }
            }
          }
        }
      }
    }
  },
  "condition": {
    "script": {
      "source": "ctx.payload.aggregations.by_user.buckets.any(bucket -> bucket.doc_count > 10)"
    }
  },
  "actions": {
    "notify_security_team": {
      "email": {
        "to": "security@example.com",
        "subject": "Multiple failed access attempts detected"
      }
    }
  }
}
```

2. **Unusual Access Pattern** (access from new IP):
```json
{
  "trigger": { "schedule": { "interval": "1h" }},
  "input": {
    "search": {
      "request": {
        "body": {
          "query": {
            "bool": {
              "must": [
                { "term": { "event.event_type": "CredentialAccessed" }},
                { "range": { "@timestamp": { "gte": "now-1h" }}}
              ],
              "must_not": [
                { "terms": { "event.ip_address": ["known.ip.list"] }}
              ]
            }
          }
        }
      }
    }
  }
}
```

---

## Compliance Requirements

### SOC 2 Type II

**Requirements**:
- **CC-04**: Log all access to credentials
- **CC-05**: Retain logs for minimum 1 year
- **CC-06**: Protect log integrity (immutable, encrypted)

**Implementation**:
```rust
pub struct Soc2AuditLogger {
    logger: AuditLogger,
    retention_days: u32,
}

impl Soc2AuditLogger {
    pub fn new() -> Self {
        Self {
            logger: AuditLogger::new(AuditConfig {
                enabled: true,
                log_successful_access: true,
                log_failed_access: true,
                include_ip_address: true,
                include_user_agent: true,
                retention_days: 365,  // SOC 2 requirement
            }),
            retention_days: 365,
        }
    }
}
```

---

### HIPAA Compliance

**Requirements**:
- **164.312(b)**: Audit controls to record and examine access
- **164.312(d)**: Person or entity authentication

**Required Events**:
- All credential access
- All administrative actions
- All authentication attempts
- All access denials

---

### GDPR Compliance

**Requirements**:
- **Article 30**: Maintain records of processing activities
- **Article 33**: Breach notification within 72 hours

**Implementation**:
```rust
/// GDPR-compliant breach detection
pub async fn detect_breach(&self) -> Option<BreachEvent> {
    // Check for unusual patterns
    let suspicious_events = self.query_logs(
        r#"event.event_type IN ["PermissionDenied", "ScopeViolation"]
           AND @timestamp > now-1h
           GROUP BY event.requester
           HAVING count > 100"#
    ).await;
    
    if !suspicious_events.is_empty() {
        Some(BreachEvent {
            detected_at: Utc::now(),
            affected_users: suspicious_events.len(),
            // Must notify within 72 hours
            notification_deadline: Utc::now() + Duration::hours(72),
        })
    } else {
        None
    }
}
```

---

## Troubleshooting

### Issue: Logs Not Appearing

**Symptoms**:
```
No audit events in log files
```

**Cause**: Audit logging disabled or incorrect configuration

**Solution**:
```rust
// Verify audit config
let config = AuditConfig::default();
assert!(config.enabled, "Audit logging must be enabled");

// Check log file permissions
// Linux/Mac:
$ ls -la ./logs/audit.log
-rw-r--r-- 1 user group 1234 Feb  3 10:00 audit.log

// Verify log level
$ export RUST_LOG=nebula_credential=debug
```

---

### Issue: Performance Degradation

**Symptoms**:
```
Credential operations slow after enabling audit logging
```

**Cause**: Synchronous logging blocking operations

**Solution**:
```rust
// Use async logging with buffering
use tokio::sync::mpsc;

pub struct AsyncAuditLogger {
    tx: mpsc::UnboundedSender<AuditEvent>,
}

impl AsyncAuditLogger {
    pub fn new() -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel();
        
        // Background task to process events
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                // Log asynchronously
                log_event_to_storage(event).await;
            }
        });
        
        Self { tx }
    }
    
    pub async fn log(&self, event: AuditEvent) {
        // Non-blocking send
        let _ = self.tx.send(event);
    }
}
```

---

## See Also

- [[Advanced/Security-Architecture|Security Architecture]]
- [[Advanced/Observability-Guide|Observability Guide]]
- [[Advanced/Compliance-SOC2|SOC 2 Compliance]]
- [[Advanced/Compliance-ISO27001|ISO 27001 Compliance]]
- [[Advanced/Compliance-HIPAA|HIPAA Compliance]]
- [[Advanced/Compliance-GDPR|GDPR Compliance]]
- [[Reference/API-Reference|API Reference]]
