# nebula-api v1 — Design Spec

## Goal

Complete the REST API for production v1: fill endpoint gaps, production-ready auth, rate limiting, WebSocket for real-time, OpenAPI spec generation.

## Current State

Axum-based API (v1 at `/api/v1/`). JWT auth middleware, RFC 9457 error responses, workflow CRUD, basic execution create/get/cancel. Pagination. Security headers, CORS, compression. Health + Prometheus metrics endpoints.

**Working:** Workflow CRUD, execution create/cancel, JWT validation, error format.
**Gaps:** Execution list incomplete, no rate limit state, no credentials API, no real-time updates, no OpenAPI.

---

## 1. Endpoint Map — Complete v1

### Workflows
| Method | Path | Status | Notes |
|--------|------|--------|-------|
| GET | `/workflows` | ✅ | List with pagination |
| POST | `/workflows` | ✅ | Create |
| GET | `/workflows/{id}` | ✅ | Get by ID |
| PUT | `/workflows/{id}` | ✅ | Update (with version check) |
| DELETE | `/workflows/{id}` | ✅ | Delete |
| POST | `/workflows/{id}/activate` | ✅ | Activate trigger |
| POST | `/workflows/{id}/execute` | ✅ | Execute immediately |
| POST | `/workflows/{id}/validate` | **NEW** | Validate without executing |
| GET | `/workflows/{id}/executions` | **NEW** | List executions for workflow |

### Executions
| Method | Path | Status | Notes |
|--------|------|--------|-------|
| GET | `/executions` | ⚠️ Fix | Needs `ExecutionRepo.list()` |
| POST | `/executions` | ✅ | Create from workflow_id + input |
| GET | `/executions/{id}` | ✅ | Get execution state |
| POST | `/executions/{id}/cancel` | ✅ | Cancel running execution |
| GET | `/executions/{id}/outputs` | **NEW** | Get per-node outputs |
| GET | `/executions/{id}/logs` | **NEW** | Get execution logs |
| WS | `/executions/{id}/stream` | **NEW (v1.1)** | Real-time execution updates |
| POST | `/executions/{id}/nodes/{node_id}/rerun` | **NEW (v1.1)** | Re-run single node (RT15) |

### Credentials
| Method | Path | Status | Notes |
|--------|------|--------|-------|
| GET | `/credentials` | **NEW** | List credential instances (redacted) |
| POST | `/credentials` | **NEW** | Create credential |
| GET | `/credentials/{id}` | **NEW** | Get credential metadata (no secrets) |
| PUT | `/credentials/{id}` | **NEW** | Update credential |
| DELETE | `/credentials/{id}` | **NEW** | Delete credential |
| POST | `/credentials/{id}/test` | **NEW** | Test credential (if self-testable) |
| POST | `/credentials/oauth2/callback` | **NEW** | OAuth2 callback handler |

### Actions (catalog)
| Method | Path | Status | Notes |
|--------|------|--------|-------|
| GET | `/actions` | **NEW** | List registered actions |
| GET | `/actions/{key}` | **NEW** | Get action metadata + parameters |
| GET | `/actions/{key}/versions` | **NEW** | List action versions |

### Plugins
| Method | Path | Status | Notes |
|--------|------|--------|-------|
| GET | `/plugins` | **NEW** | List loaded plugins |
| GET | `/plugins/{key}` | **NEW** | Plugin metadata + components |

### Infrastructure
| Method | Path | Status | Notes |
|--------|------|--------|-------|
| GET | `/health` | ✅ | Health check |
| GET | `/status` | ✅ | Engine status |
| GET | `/metrics` | ✅ | Prometheus metrics |
| POST | `/webhooks/*` | ✅ | Webhook ingest |

---

## 2. Authentication — Production Ready

### JWT (existing, enhance)
```rust
// Current: HS256 with API_JWT_SECRET env var
// Enhance:
pub struct AuthConfig {
    /// JWT signing key (min 32 bytes entropy)
    pub jwt_secret: SecretString,
    /// Token expiry (default 24h)
    pub token_expiry: Duration,
    /// Issuer for JWT claims
    pub issuer: String,
    /// Allow API key auth as alternative
    pub api_key_enabled: bool,
}
```

### API Key auth (NEW — parallel to JWT)
```rust
// Header: X-API-Key: nbl_sk_abc123...
// Prefix "nbl_sk_" for secret keys, "nbl_pk_" for publishable
// Stored hashed (Argon2) in credentials store
```

### Multi-tenant scoping
Every authenticated request carries `OwnerId`:
```rust
async fn auth_middleware(req: Request) -> Result<Request, ApiError> {
    let user = validate_token(req.headers())?;
    // SET LOCAL for RLS
    req.extensions_mut().insert(AuthenticatedUser {
        user_id: user.sub,
        owner_id: user.owner_id,  // tenant scope
        roles: user.roles,
    });
    Ok(req)
}
```

---

## 3. Rate Limiting — Stateful

```rust
pub struct RateLimitConfig {
    /// Requests per minute per tenant
    pub rpm_per_tenant: u32,
    /// Requests per minute per IP (unauthenticated)
    pub rpm_per_ip: u32,
    /// Execution starts per minute per tenant
    pub executions_per_minute: u32,
}
```

Backend: in-memory sliding window counter (v1). Redis backend (v1.1).

```rust
// Middleware extracts tenant_id or IP, checks counter
// Returns 429 with Retry-After header on breach
```

---

## 4. Real-Time Execution Updates (v1.1)

WebSocket endpoint for live execution monitoring:

```rust
// WS /api/v1/executions/{id}/stream
// Client connects, receives events:
{
    "type": "node_started",
    "node_id": "abc",
    "action_key": "http.request",
    "timestamp": "2026-04-06T12:00:00Z"
}
{
    "type": "node_completed",
    "node_id": "abc",
    "output_preview": { "status": 200 },
    "duration_ms": 142
}
{
    "type": "execution_completed",
    "status": "completed",
    "duration_ms": 523
}
```

Engine emits events to a broadcast channel. API streams them to connected WebSocket clients filtered by execution_id.

---

## 5. OpenAPI Specification (v1.1)

Auto-generate from Axum routes via `utoipa`:

```rust
#[derive(utoipa::ToSchema)]
pub struct WorkflowDefinition { ... }

#[utoipa::path(
    get,
    path = "/api/v1/workflows",
    responses(
        (status = 200, body = PaginatedResponse<WorkflowSummary>),
        (status = 401, body = ProblemDetails),
    ),
    security(("bearer" = []))
)]
async fn list_workflows(...) { ... }
```

Served at `GET /api/v1/openapi.json` and `GET /api/v1/docs` (Swagger UI).

---

## 6. Error Response Format (existing, document)

All errors follow RFC 9457 Problem Details:

```json
{
    "type": "https://nebula.dev/errors/validation",
    "title": "Validation Error",
    "status": 422,
    "detail": "Workflow definition has 3 validation errors",
    "instance": "/api/v1/workflows",
    "errors": [
        {
            "field": "/nodes/0/action_key",
            "code": "WORKFLOW:INVALID_ACTION_KEY",
            "message": "invalid action key `BAD KEY`: contains spaces"
        }
    ]
}
```

---

## 7. Webhook Ingest (existing + durability)

Per conference feedback (W2, RT8):

```rust
// POST /webhooks/{trigger_path}
async fn webhook_ingest(path: Path<String>, body: Bytes) -> Result<(), ApiError> {
    // 1. Parse (simd-json if enabled)
    let event = parse_webhook(&body)?;

    // 2. Verify signature (if trigger declares a verifier)
    trigger.verify(&headers, &body)?;

    // 3. Write to durable queue BEFORE ack
    queue_backend.enqueue(WebhookEvent { path, body: event }).await?;

    // 4. Return 200
    Ok(())
}

// Background worker processes queue → dispatches to TriggerAction
```

---

## 8. What Changes vs Current

| Area | Current | New |
|------|---------|-----|
| Execution list | Returns empty | Full list with filters |
| Execution outputs | Not exposed | GET per-node outputs |
| Credentials API | None | Full CRUD + test + OAuth2 callback |
| Action catalog | None | List + metadata + versions |
| Plugin catalog | None | List + components |
| Rate limiting | Skeleton only | Stateful sliding window |
| Auth | JWT only | JWT + API Key |
| Real-time | None | WebSocket stream (v1.1) |
| OpenAPI | None | Auto-generated (v1.1) |
| Webhook durability | None | Queue before ack |
| Workflow validation | Only on create | Dedicated validate endpoint |

---

## 9. Implementation Phases

| Phase | What | Depends on |
|-------|------|------------|
| 1 | Fix execution list + add outputs endpoint | ExecutionRepo.list() |
| 2 | Credentials API (CRUD + test) | Credential v3 resolver |
| 3 | Action + Plugin catalog endpoints | ActionRegistry, PluginRegistry |
| 4 | Rate limiting with in-memory state | None |
| 5 | API Key auth parallel to JWT | Credential store for hashed keys |
| 6 | Workflow validate endpoint | Engine validation |
| 7 | Webhook durable queue | QueueBackend (RT11) |
| 8 | WebSocket execution stream | Engine event broadcast (v1.1) |
| 9 | OpenAPI generation via utoipa | v1.1 |
| 10 | Rerun node endpoint | Engine resume (v1.1) |

**Phase 1-4 = minimum viable API.**

---

## 10. Not In Scope

- GraphQL API (REST is sufficient for v1)
- gRPC API (v2 for SDK embedding)
- Admin API (user management, team management)
- Billing API (cloud service concern)
- SSO / OAuth2 login (enterprise feature)
- API versioning (v2 endpoint) — stable v1 first
