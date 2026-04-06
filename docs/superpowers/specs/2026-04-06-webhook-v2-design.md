# nebula-webhook v2 — Design Spec

## Goal

Enhance webhook crate for production: durable inbound queue, framework-level signature verification, outbound webhook delivery, metrics, rate limiting per path.

## Current State

Solid implementation (49 tests): Axum-based HTTP server, UUID-per-trigger routing, broadcast channels, TriggerHandle with RAII cleanup, WebhookAction trait, test/prod environment isolation, zero-copy payloads via `bytes::Bytes`.

**Missing:** Durable queue (events lost on crash), signature verification framework, outbound webhooks, metrics, rate limiting.

---

## 1. Durable Inbound Queue (W2, Twilio/Telegram feedback)

Events written to storage BEFORE HTTP 200 ack:

```rust
impl WebhookServer {
    async fn handle_webhook(
        &self,
        path: String,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<StatusCode, WebhookError> {
        // 1. Find trigger for this path
        let trigger = self.route_map.get(&path)
            .ok_or(WebhookError::NotFound)?;

        // 2. Verify signature (if trigger has verifier)
        if let Some(verifier) = &trigger.verifier {
            verifier.verify(&headers, &body)?;
        }

        // 3. Write to durable queue BEFORE ack
        let event = WebhookEvent {
            id: Uuid::new_v4().to_string(),
            trigger_id: trigger.id.clone(),
            path: path.clone(),
            headers: serialize_headers(&headers),
            body: body.to_vec(),
            received_at: Utc::now(),
        };
        self.queue.enqueue("webhooks", QueuedTask::from(&event)).await?;

        // 4. Ack — event is durable
        Ok(StatusCode::OK)
    }
}
```

If Postgres is down → RT8 local spill buffer (in-memory ring or local WAL file).

---

## 2. Signature Verification Framework (RT-7, red team)

```rust
/// Framework-level webhook signature verifier.
pub trait WebhookVerifier: Send + Sync {
    /// Verify the request is authentic.
    fn verify(&self, headers: &HeaderMap, body: &[u8]) -> Result<(), WebhookError>;
}

/// HMAC-SHA256 verifier (Stripe, GitHub, Slack pattern).
pub struct HmacSha256Verifier {
    secret: SecretString,
    header_name: String,      // e.g., "X-Hub-Signature-256"
    prefix: Option<String>,   // e.g., "sha256="
}

impl WebhookVerifier for HmacSha256Verifier {
    fn verify(&self, headers: &HeaderMap, body: &[u8]) -> Result<(), WebhookError> {
        let signature = headers.get(&self.header_name)
            .ok_or(WebhookError::MissingSignature)?;

        let expected = hmac_sha256(self.secret.expose(), body);

        // Constant-time comparison (prevents timing attacks)
        if !constant_time_eq(signature.as_bytes(), expected.as_bytes()) {
            return Err(WebhookError::InvalidSignature);
        }
        Ok(())
    }
}

/// Timestamp + signature verifier (Stripe pattern — prevents replay).
pub struct TimestampHmacVerifier {
    secret: SecretString,
    signature_header: String,   // "Stripe-Signature"
    tolerance: Duration,        // max age: 5 minutes
}
```

Trigger declares its verifier at registration:
```rust
webhook_server.register(
    trigger_id,
    path,
    Some(Arc::new(HmacSha256Verifier::new(secret, "X-Hub-Signature-256"))),
);
```

---

## 3. Outbound Webhook Delivery (Notion feedback)

Nebula as webhook SENDER (notify external systems on workflow events):

```rust
/// Outbound webhook endpoint configuration.
pub struct WebhookEndpoint {
    pub id: String,
    pub url: String,
    pub events: Vec<WebhookEventType>,
    pub secret: SecretString,  // for signing outbound requests
    pub enabled: bool,
}

pub enum WebhookEventType {
    WorkflowCompleted,
    WorkflowFailed,
    ExecutionStarted,
    ExecutionCompleted,
    Custom(String),
}

/// Outbound delivery with retry.
pub struct WebhookDeliverer {
    client: reqwest::Client,
    queue: Arc<dyn QueueBackend>,
    max_retries: u32,
    retry_backoff: Duration,
}

impl WebhookDeliverer {
    /// Queue a delivery attempt.
    pub async fn deliver(&self, endpoint: &WebhookEndpoint, event: &WebhookEvent) -> Result<()> {
        let payload = serde_json::to_vec(event)?;
        let signature = hmac_sha256(endpoint.secret.expose(), &payload);

        let task = DeliveryTask {
            endpoint_url: endpoint.url.clone(),
            payload,
            signature,
            attempt: 0,
        };
        self.queue.enqueue("webhook_deliveries", task.into()).await
    }
}
```

Delivery attempts are queryable via API (`GET /webhook-endpoints/{id}/deliveries`).

---

## 4. Metrics

```rust
pub const WEBHOOK_RECEIVED_TOTAL: &str = "nebula_webhook_received_total";
pub const WEBHOOK_VERIFIED_TOTAL: &str = "nebula_webhook_verified_total";
pub const WEBHOOK_VERIFICATION_FAILED: &str = "nebula_webhook_verification_failed_total";
pub const WEBHOOK_QUEUED_TOTAL: &str = "nebula_webhook_queued_total";
pub const WEBHOOK_PROCESSED_TOTAL: &str = "nebula_webhook_processed_total";
pub const WEBHOOK_DELIVERY_TOTAL: &str = "nebula_webhook_delivery_total";     // outbound
pub const WEBHOOK_DELIVERY_FAILED: &str = "nebula_webhook_delivery_failed_total";
pub const WEBHOOK_QUEUE_LAG: &str = "nebula_webhook_queue_lag_seconds";
```

---

## 5. Rate Limiting Per Path

```rust
/// Per-trigger rate limiter prevents webhook flood.
pub struct WebhookRateLimiter {
    limits: DashMap<String, RateLimitBucket>,
    default_rpm: u32,  // requests per minute, default 600
}
```

Returns `429 Too Many Requests` with `Retry-After` header.

---

## 6. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Durability | Events lost on crash | Queue before ack (QueueBackend) |
| Signature verification | Author responsibility | Framework WebhookVerifier trait |
| Outbound delivery | None | WebhookDeliverer with retry |
| Metrics | None | 8 counters |
| Rate limiting | None | Per-path RPM limiter |

---

## 7. Not In Scope

- WebSocket upgrade on webhook paths (v2)
- Custom response bodies (always 200 OK or error)
- Request/response transformation middleware
- Multi-region webhook routing
