---

## name: nebula-api
role: API Gateway
status: frontier
last-reviewed: 2026-04-23
canon-invariants: [L2-§4.5, L2-§12.3, L2-§12.4, L2-§13]
related: [nebula-storage, nebula-runtime, nebula-engine, nebula-plugin, nebula-metrics, nebula-credential, nebula-core]

# nebula-api

## Purpose

Provides the HTTP entry point for the Nebula workflow engine. Translates REST
requests into calls against typed port traits (`WorkflowRepo`, `ExecutionRepo`,
`ControlQueueRepo`, `OrgResolver`, `WorkspaceResolver`, `SessionStore`,
`MembershipStore`), then delegates all business logic to the crates below it.
The crate also hosts the `webhook` submodule, which handles inbound trigger
delivery and per-endpoint lifecycle management.

All routes are tenant-scoped under `/api/v1/orgs/{org}/workspaces/{ws}/…`
(per spec 05-api-routing). Slugs and ULIDs are accepted interchangeably
via `nebula-core::Slug`.

No SQL driver or storage schema knowledge lives here — those details are owned
by `nebula-storage` and injected through ports at startup (§12.3 local-first:
in-memory repos work without Docker or Redis).

## Role

API Gateway (EIP "Message Endpoint" at the system boundary). Thin HTTP shell
with no business logic of its own; all decisions are delegated to the engine
layer via port traits injected into `AppState`.

## Public API

- `AppState` — shared state struct holding all port references; built via
constructor + builder methods (`with_api_keys`, `with_metrics_registry`,
`with_webhook_transport`, …).
- `build_app` — assembles the axum `Router` with all middleware (tracing,
CORS, compression, security headers, auth, tenancy, RBAC, CSRF).
- `ApiConfig` / `ApiConfig::from_env` — runtime configuration with sub-configs:
`TlsConfig`, `CookieConfig`, `CorsConfig`, `VersioningConfig`,
`PaginationConfig`. Startup fails hard on a missing or short JWT secret
(no `Default` impl).
- `JwtSecret` — newtype that enforces a 32-byte minimum length at construction;
any value in hand is valid.
- `ApiError` / `ProblemDetails` — RFC 9457 error envelope; seam for §12.4.
Every failure path maps to a typed `ApiError` variant with an explicit HTTP
status — no new ad-hoc 500s for business-logic errors. Includes variants:
`SessionExpired`, `MfaRequired`, `InsufficientRole`, `OrgNotFound`,
`WorkspaceNotFound`, `SlugConflict`, `CsrfRejected`, `PaginationInvalid`,
`RateLimited`, `TenantMismatch`.
- `CursorParams` / `PaginatedResponse<T>` — cursor-based pagination
infrastructure (opaque base64-encoded cursors).
- Port traits: `OrgResolver`, `WorkspaceResolver`, `SessionStore`,
`MembershipStore` — tenant resolution and session management ports.
- `AuthContext` — authenticated request context extracted by `middleware::auth`.
- `webhook::WebhookTransport` — activate/deactivate/router for inbound webhook
triggers; mounted on `/webhooks/*` when the transport is attached to
`AppState`.
- `webhook::EndpointProviderImpl` — implements `nebula_action::WebhookEndpointProvider`
so action code can read `ctx.webhook.endpoint_url()` without knowing the HTTP
layer.
- `webhook::WebhookRateLimiter` / `RateLimitExceeded` — per-endpoint rate
limiting for inbound webhook requests.

## Contract

- **[L2-§12.4]** All error responses use RFC 9457 `application/problem+json`.
No new ad-hoc 500 for a business-logic failure; map new failure modes into
a typed `ApiError` variant. Seam: `crates/api/src/errors.rs`.
- **[L2-§13 step 1]** Workflow creation (`POST /api/v1/workflows`) delegates
to `WorkflowRepo::create`. Seam: `crates/api/src/handlers/workflow.rs` —
`create_workflow`.
- **[L2-§13 step 2]** Workflow activation (`POST /api/v1/workflows/:id/activate`)
runs `nebula_workflow::validate_workflow` and rejects invalid definitions
with structured RFC 9457 errors — it does not silently flip a flag. Seam:
`crates/api/src/handlers/workflow.rs` — `activate_workflow`.
- **[L2-§13 step 3]** Execution start (`POST /api/v1/workflows/:id/executions`)
returns 202 Accepted and enqueues; it does not block on engine completion.
Seam: `crates/api/src/handlers/execution.rs` — `start_execution`.
- **[L2-§13 step 5]** Cancel (`POST /api/v1/executions/:id/cancel`) writes a
durable signal to `ControlQueueRepo` in the same logical operation as the
state transition — not only a DB-row flip. Seam:
`crates/api/src/handlers/execution.rs` — `cancel_execution`.
- **[L2-§12.3]** Local-first: the server starts with in-memory repos (no
Docker/Redis required). The `AppState::new` constructor accepts any
`WorkflowRepo + ExecutionRepo + ControlQueueRepo` impl.
- **[L2-§4.5]** No capability is advertised that the engine does not honor
end-to-end. Planned features (JWT validation, rate limiting) are hidden
until the engine side is wired.
- **[L2-§12.2]** Cancel signals share the outbox transaction — the
`control_queue_repo` field in `AppState` is the durable outbox (§12.2).
A second in-memory control channel is forbidden (see §12.2 prohibition on
unreconciled second channels).

## Webhook submodule

`crates/api/src/webhook/` is the HTTP transport for inbound webhook triggers.
It is **not** a separate crate. Responsibility split:

- `transport` — `WebhookTransport`: activate / deactivate / axum router for
`POST /webhooks/:trigger_uuid/:nonce`. Mounts only when attached to
`AppState::with_webhook_transport`. `activate()` takes a
`nebula_action::WebhookConfig` alongside the handler so the transport
can enforce signature policy before dispatch — the config is read from
the typed `WebhookAction` at activation time and is *not* routed through
the dyn `TriggerHandler` contract.
- `provider` — `EndpointProviderImpl`: implements
`nebula_action::WebhookEndpointProvider` so plugins read the public URL
without knowing the HTTP layer.
- `routing` — private `RoutingMap` (DashMap) keyed by `(trigger_uuid, nonce)`.
- `ratelimit` — `WebhookRateLimiter`: per-endpoint token-bucket guard;
returns `RateLimitExceeded` on breach.

Delivery semantics: at-least-once (§11.3 / §13.4). Duplicate delivery is the
caller's responsibility via stable event identity + idempotency keys.

Signature enforcement (ADR-0022): the transport consults
`entry.config.signature_policy()` between `WebhookRequest::try_new` and
oneshot setup. Outcomes:

- `SignaturePolicy::OptionalAcceptUnsigned` → pass through to `handle_event`.
- `Required` with an empty secret → `500 application/problem+json`
(`https://nebula.dev/problems/webhook-signature-misconfigured`). Handler is NOT invoked.
- `Required` / `Custom` producing `SignatureOutcome::Missing` or `Invalid` → `401 application/problem+json` (`https://nebula.dev/problems/webhook-signature`). Handler is NOT invoked.
- `Valid` → pass through to `handle_event`.

Every rejection increments `nebula_webhook_signature_failures_total` with
`reason ∈ { missing, invalid, missing_secret }` when a metrics registry is
attached via `WebhookTransport::with_metrics`. The counter is low cardinality
by design (three static reason labels, no per-trigger dimension).

## Machine-Readable API

`nebula-api` publishes its full route surface as an **OpenAPI 3.1**
specification (ADR-0047). The document is regenerated on every startup
from the same source as the served `axum::Router`, so the published
contract and the runtime cannot drift apart.

| Endpoint                  | Purpose |
|---------------------------|---------|
| `GET /api/v1/openapi.json` | OpenAPI 3.1 specification document — fetched by client generators (openapi-generator-cli, oapi-codegen, …). Unauthenticated. |
| `GET /api/v1/docs/`        | Swagger UI rendering of the served spec. Unauthenticated. **Self-hosted** via `utoipa_swagger_ui::SwaggerUi` — every static asset (HTML, CSS, JS) ships embedded in the server binary; no third-party CDN is reached at request time. |

### Drift-detection guarantee

The router is built through `utoipa_axum::router::OpenApiRouter::routes(routes!(handler))`,
which is the **only** mounting path that ties the served `axum::Router`
to the generated `OpenApi` value. Handlers without `#[utoipa::path]`
cannot pass through `routes!()` — drift is a compile error rather than
a review-time catch.

`crates/api/tests/openapi_spec.rs` adds runtime guards on top:

- `served_spec_pins_openapi_3_1_0` — `openapi == "3.1.0"`.
- `operation_ids_are_unique` — every handler function name is observed
  exactly once across the spec.
- `all_refs_resolve_to_declared_components` — recursive `$ref` walk.
- `security_schemes_match_adr_0047` — `bearer` / `api_key` / `csrf`.
- `drift_smoke_known_paths_are_present` — load-bearing paths inventory.
- `served_spec_round_trips_through_oas3_parser` — strict 3.1 parse.

### Stub Endpoint Policy

Endpoints whose handler currently returns `ApiError::Internal("not
implemented")` (e.g. `me/*`, `org/*`, `resource/list`,
`execution/{terminate,restart}`) are documented honestly per
ADR-0047 §4:

- `#[deprecated]` on the handler so utoipa flags the operation in spec.
- `responses((status = 501, …))` carries the **planned** payload shape.
- Tag suffix ` (planned)` groups the stubs visibly in Swagger UI.

`crates/api/tests/openapi_canon_compliance.rs` enforces the policy in
both directions (every deprecated operation has a 501 response; every
stub module reaches the handler at runtime returning 500/501) so a
silently-shipped endpoint cannot pass review.

### Regeneration

The spec is materialised inside `build_app` via
`OpenApiRouter::split_for_parts()` and handed straight to
`utoipa_swagger_ui::SwaggerUi`, which serves both `/api/v1/openapi.json`
and `/api/v1/docs/` as a Tower service merged into the application
router. `build_app` writes a stable
`tracing::info!(spec.version=%, paths=N, "openapi: spec compiled")`
line at startup so production logs can pin against it. To regenerate
locally, run any test that calls `nebula_api::build_app` — for example
`cargo nextest run -p nebula-api --test openapi_spec`.

### Idempotency-Key (M3.4 / ADR-0048)

Every state-changing endpoint reachable from `build_app` is replay-protected
through the `IdempotencyLayer` middleware. Clients opt in by sending an
`Idempotency-Key` header on a `POST` request — the middleware caches the
first response (status + body + filtered headers) keyed by
`(method, path, key, identity-fingerprint, body-fingerprint)` and replays
it byte-for-byte on subsequent requests within the configured TTL.

**Protocol contract** — the IETF draft
[`draft-ietf-httpapi-idempotency-key`](https://datatracker.ietf.org/doc/draft-ietf-httpapi-idempotency-key/)
governs header semantics. Highlights:

- Same `Idempotency-Key` + same body within the TTL → cached replay
  (response carries `Idempotent-Replay: true`).
- Same key + different body → **422 Unprocessable Entity** with
  `application/problem+json`.
- `5xx` responses are passed through uncached so transient backend failures
  do not pin a permanent error for the TTL window.

**Environment variables** (defaults applied by `ApiConfig::from_env`):

| Var | Default | Notes |
|---|---|---|
| `API_IDEMPOTENCY_BACKEND` | `memory` | `memory` \| `postgres`. See backend tradeoffs below. |
| `API_IDEMPOTENCY_TTL_SECS` | `86400` | Cached-entry lifetime (24h matches the IETF draft). |
| `API_IDEMPOTENCY_MAX_ENTRIES` | `10000` | Cap for the in-memory backend; PG honours `expires_at` instead. |
| `API_IDEMPOTENCY_MAX_REQUEST_BODY_BYTES` | `1048576` | Requests beyond this skip caching (forwarded as-is). |
| `API_IDEMPOTENCY_MAX_RESPONSE_BODY_BYTES` | `1048576` | Responses beyond this are returned uncached. |
| `API_IDEMPOTENCY_SWEEP_INTERVAL_SECS` | `300` | PG-only: cadence for the `evict_expired` background sweep. `0` disables. `< 60` triggers a startup `WARN`. |

**Store-backend tradeoffs** (see `docs/adr/0048-idempotency-store-backend.md`):

| Backend | When | Restart-survival | Multi-replica share |
|---|---|---|---|
| `memory` | Dev / single-process tests / one-replica deployments | No — state lost on restart | No — process-local |
| `postgres` | Production deployments (≥ 2 replicas, or restart-tolerance required) | Yes — table survives restart | Yes — same `DATABASE_URL` shared |

> **Operator warning:** selecting `memory` outside `NEBULA_ENV=development`
> emits a startup `tracing::warn!` — dedup state is lost on restart and
> across runners. The §M3 1.0 closure criterion requires `postgres` for
> production.

The `postgres` backend is gated behind the `nebula-api/postgres` cargo
feature so default builds remain lightweight. Selecting
`API_IDEMPOTENCY_BACKEND=postgres` without that feature compiled in fails
closed at startup (per ADR-0048; no silent fallback).

### Cross-layer schema strategy

Per ADR-0047 §3, API DTOs MUST NOT embed types from `nebula-core`,
`nebula-storage`, `nebula-engine`, or `nebula-credential`. Cross-layer
types (`OrgRole`, `WorkspaceRole`) are wrapped at the API boundary as
`OrgRoleDto` / `WorkspaceRoleDto` so the spec is decoupled from internal
type evolution.

## Non-goals

- Not a storage driver — no SQL or schema knowledge; see `nebula-storage`.
- Not a workflow engine — no execution logic; see `nebula-engine` / `nebula-runtime`.
- Not an expression evaluator — see `nebula-expression`.
- Not an outbound HTTP client — webhook *delivery* outbound lives in action
plugins, not here. This module handles inbound receipt only.
- WebSocket / SSE for real-time execution updates — not yet wired end-to-end
(§4.5: hidden until the engine side exists).

## Maturity

See `docs/MATURITY.md` row for `nebula-api`. Short summary:

- API stability: frontier
- JWT validation middleware, rate limiting, and pagination are implemented;
some route handlers contain TODO stubs pending full business logic (§4.5).
- Webhook transport is functional but delivery guarantee is at-least-once;
exactly-once is not claimed.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.5 (operational honesty), §12.2 (durable
outbox), §12.3 (local-first), §12.4 (RFC 9457 errors), §13 (knife scenario
steps 1–3 and 5).
- Satellite docs: `docs/INTEGRATION_MODEL.md`, `docs/OBSERVABILITY.md`.
- Siblings: `nebula-storage` (port impls), `nebula-engine` (execution logic),
`nebula-runtime` (action registry), `nebula-plugin` (plugin registry),
`nebula-credential` (secret store), `nebula-metrics` (metric primitives,
naming policy, and Prometheus export).

## Appendix

### Source layout

```
src/
├── lib.rs              # Crate root, public re-exports
├── app.rs              # Router assembly + middleware stack
├── config.rs           # ApiConfig, JwtSecret, TlsConfig, CookieConfig, CorsConfig,
│                       # VersioningConfig, PaginationConfig
├── state.rs            # AppState (port traits only — no concrete impls)
├── errors.rs           # RFC 9457 ProblemDetails + ApiError (§12.4 seam)
├── pagination.rs       # CursorParams, PaginatedResponse<T>
├── extractors/         # ValidatedJson and other custom extractors
├── handlers/
│   ├── auth.rs         # Login, logout, refresh, MFA
│   ├── health.rs       # GET /health, GET /ready
│   ├── me.rs           # Current user profile
│   ├── org.rs          # Organization CRUD
│   ├── workflow.rs     # Workflow CRUD + activate (§13 steps 1–2)
│   ├── execution.rs    # Start / cancel executions (§13 steps 3, 5)
│   ├── credential.rs   # Credential management + OAuth2 flows
│   ├── catalog.rs      # Action/resource/credential catalog listing
│   ├── openapi.rs      # OpenAPI schema endpoint
│   └── webhook.rs      # Webhook management
├── middleware/
│   ├── auth.rs         # JWT + API-key auth → AuthContext
│   ├── tenancy.rs      # Tenant resolution from path (org/workspace)
│   ├── rbac.rs         # Role-based access control checks
│   ├── csrf.rs         # CSRF token validation
│   ├── rate_limit.rs   # Rate limiting
│   ├── request_id.rs   # Unique request ID propagation
│   └── security_headers.rs
├── models/             # Request / Response DTOs
│   ├── health.rs
│   ├── workflow.rs
│   └── execution.rs
├── routes/             # Domain-scoped route builders
│   ├── mod.rs          # create_routes()
│   ├── auth.rs         # /api/v1/auth/*
│   ├── me.rs           # /api/v1/me
│   ├── org.rs          # /api/v1/orgs/*
│   ├── workspace.rs    # /api/v1/orgs/{org}/workspaces/{ws}/*
│   ├── health.rs
│   ├── workflow.rs     # Tenant-scoped workflow routes
│   ├── execution.rs    # Tenant-scoped execution routes
│   ├── credential.rs   # Tenant-scoped credential routes
│   ├── catalog.rs
│   ├── webhook.rs
│   ├── metrics.rs
│   └── openapi.rs
├── services/           # Orchestration layer (currently empty)
└── webhook/            # Inbound trigger transport (§11.3 / §13.4)
    ├── mod.rs
    ├── transport.rs    # WebhookTransport — activate/deactivate/router
    ├── provider.rs     # EndpointProviderImpl
    ├── routing.rs      # RoutingMap (private)
    └── ratelimit.rs    # WebhookRateLimiter
```

### Startup example

```rust
use nebula_api::{build_app, ApiConfig, AppState};
use nebula_storage::{InMemoryWorkflowRepo, InMemoryExecutionRepo};
use nebula_storage::repos::InMemoryControlQueueRepo;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();

    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());

    // `from_env` reads `API_JWT_SECRET` (must be 32+ bytes).
    // In development set `NEBULA_ENV=development` to get an ephemeral secret.
    let api_config = ApiConfig::from_env()?;
    let state = AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    );
    let app = build_app(state, &api_config);

    let addr = api_config.bind_address;
    tracing::info!("Starting server on {}", addr);
    nebula_api::app::serve(app, addr).await?;
    Ok(())
}
```

### Transport binaries

This crate now ships with three binary targets that share a common startup
foundation (`src/server/mod.rs`) but run different ingress transports:

- `nebula-server` — full REST API (`SERVER_BIND_ADDRESS` override, fallback to `API_BIND_ADDRESS`)
- `nebula-webhook` — webhook ingress-only (`WEBHOOK_BIND_ADDRESS` override)
- `nebula-realtime` — realtime scaffold (`REALTIME_BIND_ADDRESS` override, `/ws` currently returns 501)

Run locally:

```bash
cargo run -p nebula-api --bin nebula-server
cargo run -p nebula-api --bin nebula-webhook
cargo run -p nebula-api --bin nebula-realtime
```

### Error format (RFC 9457)

```json
{
  "type": "https://nebula.dev/problems/not-found",
  "title": "Not Found",
  "status": 404,
  "detail": "Workflow abc123 not found"
}
```

### Endpoint reference


| Method   | Path                                                        | Description                                                       |
| -------- | ----------------------------------------------------------- | ----------------------------------------------------------------- |
| `GET`    | `/health`                                                   | Liveness check (always available)                                 |
| `GET`    | `/ready`                                                    | Readiness check (verifies dependencies)                           |
| `POST`   | `/api/v1/auth/login`                                        | Session login                                                     |
| `POST`   | `/api/v1/auth/logout`                                       | Session logout                                                    |
| `POST`   | `/api/v1/auth/refresh`                                      | Token refresh                                                     |
| `GET`    | `/api/v1/me`                                                | Current user profile                                              |
| `GET`    | `/api/v1/orgs`                                              | List organizations                                                |
| `POST`   | `/api/v1/orgs`                                              | Create organization                                               |
| `GET`    | `/api/v1/orgs/:org`                                         | Get organization by slug or ID                                    |
| `GET`    | `/api/v1/orgs/:org/workspaces`                              | List workspaces                                                   |
| `POST`   | `/api/v1/orgs/:org/workspaces`                              | Create workspace                                                  |
| `GET`    | `/api/v1/orgs/:org/workspaces/:ws/workflows`                | List workflows (tenant-scoped)                                    |
| `POST`   | `/api/v1/orgs/:org/workspaces/:ws/workflows`                | Create workflow (tenant-scoped, §13 step 1)                       |
| `GET`    | `/api/v1/orgs/:org/workspaces/:ws/workflows/:id`            | Get workflow by ID                                                |
| `PUT`    | `/api/v1/orgs/:org/workspaces/:ws/workflows/:id`            | Update workflow                                                   |
| `DELETE` | `/api/v1/orgs/:org/workspaces/:ws/workflows/:id`            | Delete workflow                                                   |
| `POST`   | `/api/v1/orgs/:org/workspaces/:ws/workflows/:id/activate`   | Activate workflow — runs validation (§13 step 2)                  |
| `GET`    | `/api/v1/orgs/:org/workspaces/:ws/workflows/:id/executions` | List executions                                                   |
| `POST`   | `/api/v1/orgs/:org/workspaces/:ws/workflows/:id/executions` | Start execution — 202 Accepted (§13 step 3)                       |
| `GET`    | `/api/v1/orgs/:org/workspaces/:ws/executions/:id`           | Get execution status                                              |
| `POST`   | `/api/v1/orgs/:org/workspaces/:ws/executions/:id/cancel`    | Cancel execution — durable signal (§13 step 5)                    |
| `POST`   | `/webhooks/:trigger_uuid/:nonce`                            | Inbound webhook trigger (mounted when `webhook_transport` is set) |
| `GET`    | `/api/v1/openapi.json`                                      | OpenAPI schema (planned)                                          |


