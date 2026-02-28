# Security

## Threat Model

### Assets

- API server (health, status, webhook endpoints)
- Worker status (queue lengths, ids)
- Webhook payloads (may contain sensitive data)
- Future: workflow definitions, execution data, credentials

### Trust Boundaries

- **Current:** No auth; all routes public. Suitable for internal/local only.
- **Target:** Auth layer; JWT/API key for workflow/execution routes.
- **Webhook:** Caller is external; payload validation critical.

### Attacker Capabilities

- **Unauthenticated access:** Current state; anyone can hit /health, /status.
- **Webhook spoofing:** Forged POST to /webhooks/*; need signature verification.
- **DoS:** Flood /health or /webhooks; rate limiting needed.

## Security Controls

### Authn/Authz

- **Current:** None. All routes unauthenticated.
- **Target:** JWT for user sessions; API key for machine-to-machine. RBAC for workflow/execution.
- **Webhook:** HMAC signature verification (X-Webhook-Signature); configurable per endpoint.

### Isolation/Sandboxing

- API is HTTP layer; no sandboxing. Engine/runtime handle execution isolation.
- Webhook handlers should validate payload; no eval of user input.

### Secret Handling

- **Webhook secrets:** For signature verification; from config/env; never in logs.
- **API keys:** Hashed in storage; plain only at issue.
- **JWT secret:** Strong key; rotation support.

### Input Validation

- **Webhook body:** Size limit (body_limit); JSON schema if applicable.
- **Path params:** Validate format (UUID, etc.).
- **Query params:** Sanitize; limit length.

## Abuse Cases

| Case | Prevention | Detection | Response |
|------|-------------|------------|----------|
| Unauthorized workflow execution | Auth (Phase 2) | Audit log | 401 |
| Webhook spoofing | Signature verification | — | 403 |
| DoS / flood | Rate limiting | Metrics | 429, circuit breaker |
| Path traversal | Validate path params | — | 400 |
| Oversized payload | body_limit | — | 413 |

## Security Requirements

### Must-Have (Phase 2)

- Authentication for workflow/execution routes
- Webhook signature verification
- Rate limiting
- Input validation

### Should-Have

- CORS config (restrict origins)
- Security headers (X-Content-Type-Options, etc.)
- Audit logging for mutations

## Security Test Plan

- **Static analysis:** cargo audit
- **Dynamic tests:** Auth required for protected routes; signature verification
- **Fuzz:** Optional; request body fuzz
