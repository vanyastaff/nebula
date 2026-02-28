# Reliability

## SLO Targets

| Metric | Target | Notes |
|--------|--------|-------|
| **Availability** | 99.9% | With health checks; k8s liveness |
| **Latency** | /health < 1ms; /status < 5ms | P99 |
| **Error budget** | < 0.1% 5xx | Excluding client errors |

## Failure Modes

### Dependency Outage

- **Webhook creation fails:** run() returns ApiError::Webhook; server does not start.
- **Bind fails:** ApiError::Io; address in use, permission denied.
- **axum serve:** Propagates io::Error; server exits.

### Timeout/Backpressure

- **Slow webhook handler:** Blocks worker; no built-in timeout. Webhook crate responsibility.
- **Connection backlog:** tokio TcpListener backlog; OS limit.
- **No rate limiting:** MVP; Phase 2 adds.

### Partial Degradation

- **One route slow:** Others unaffected (axum concurrent).
- **Webhook downstream slow:** Webhook handler blocks; consider timeout in webhook.

### Data Corruption

- **Status response:** Read-only; no corruption.
- **Webhook payload:** Validation in handler; malformed returns 400.

## Resilience Strategies

### Retry Policy

- API does not retry. Clients may retry on 5xx.
- Webhook delivery retry: webhook crate responsibility.

### Circuit Breaking

- N/A at API layer. Could add for downstream (engine, storage) in Phase 2.

### Fallback Behavior

- **Health:** Always 200 when server up; no fallback.
- **Status:** Returns current state; no fallback for missing workers.

### Graceful Degradation

- **Shutdown:** axum respects signal; drain in-flight requests. tokio signal handling in app.

## Operational Runbook

### Alert Conditions

- /health fails (server down)
- /status latency high
- 5xx rate spike
- Webhook error rate

### Dashboards

- Request rate by path
- Latency histogram
- Error rate
- Active connections

### Incident Triage Steps

1. Check /health — is server up?
2. Check /api/v1/status — workers, webhook routes
3. Check webhook logs — delivery failures?
4. Check bind address — port conflict?

## Capacity Planning

### Load Profile Assumptions

- /health: High frequency (k8s probe every 10s)
- /status: Low (dashboard, debugging)
- /webhooks/*: Variable; depends on workflow triggers

### Scaling Constraints

- **Single process:** One listener; one server. Horizontal scaling = more replicas.
- **Connection limit:** tokio default; tune if needed.
- **Worker count:** Passed by app; not API bottleneck.
