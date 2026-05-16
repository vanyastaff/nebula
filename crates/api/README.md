---

## name: nebula-api
role: API Gateway
status: frontier
last-reviewed: 2026-05-15
canon-invariants: [L2-§4.5, L2-§12.3, L2-§12.4, L2-§13]
related: [nebula-storage, nebula-runtime, nebula-engine, nebula-plugin, nebula-metrics, nebula-credential, nebula-core]

# nebula-api

## Purpose

Provides the HTTP entry point for the Nebula workflow engine. Translates REST
requests into calls against typed port traits (`WorkflowRepo`, `ExecutionRepo`,
`ControlQueueRepo`, `OrgResolver`, `WorkspaceResolver`, `SessionStore`,
`MembershipStore`), then delegates all business logic to the crates below it.
The crate also hosts the `transport::webhook` subsystem, which handles inbound trigger
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
  W3C Trace Context (`traceparent` / optional `tracestate`) is parsed in
  `middleware::trace_w3c`, linked into the per-request `TraceLayer` span, echoed on
  responses where applicable, and stamped onto durable `execution_control_queue` rows
  for `Start` / `Cancel` (see ADR-0050).
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
- `transport::webhook::WebhookTransport` — activate/activate_slug/deactivate/router
for inbound webhook triggers; mounted on `/webhooks/*` (programmatic) and
`/api/v1/hooks/*` (slug-routed) when the transport is attached to
`AppState`.
- `transport::webhook::EndpointProviderImpl` — implements `nebula_action::WebhookEndpointProvider`
so action code can read `ctx.webhook.endpoint_url()` without knowing the HTTP
layer.
- `transport::webhook::WebhookRateLimiter` / `RateLimitExceeded` — per-key rate
limiting for inbound webhook requests.

## Contract

- **[L2-§12.4]** All error responses use RFC 9457 `application/problem+json`.
No new ad-hoc 500 for a business-logic failure; map new failure modes into
a typed `ApiError` variant. Seam: `crates/api/src/error/mod.rs`.
- **[L2-§13 step 1]** Workflow creation (`POST /api/v1/workflows`) delegates
to `WorkflowRepo::create`. Seam: `crates/api/src/domain/workflow/handler.rs` —
`create_workflow`.
- **[L2-§13 step 2]** Workflow activation (`POST /api/v1/workflows/:id/activate`)
runs `nebula_workflow::validate_workflow` and rejects invalid definitions
with structured RFC 9457 errors — it does not silently flip a flag. Seam:
`crates/api/src/domain/workflow/handler.rs` — `activate_workflow`.
- **[L2-§13 step 3]** Execution start (`POST /api/v1/workflows/:id/executions`)
returns 202 Accepted and enqueues; it does not block on engine completion.
Seam: `crates/api/src/domain/execution/handler.rs` — `start_execution`.
- **[L2-§13 step 5]** Cancel (`POST /api/v1/executions/:id/cancel`) writes a
durable signal to `ControlQueueRepo` in the same logical operation as the
state transition — not only a DB-row flip. Seam:
`crates/api/src/domain/execution/handler.rs` — `cancel_execution`.
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

## Webhook subsystem

`crates/api/src/transport/webhook/` is the **single converged** HTTP transport
for inbound webhook triggers (M3.3 / ADR-0049). Both URL shapes funnel through
one `dispatch_inner` pipeline:

- **Programmatic** — `POST /webhooks/{trigger_uuid}/{nonce}`, minted by
`WebhookTransport::activate(...)` from the typed `nebula_action::WebhookAction`
runtime path.
- **Slug-routed** — `POST|GET /api/v1/hooks/{org}/{ws}/{trigger_slug}`,
loaded from storage at startup via `bootstrap_webhook_activations` and
mutated by `TriggerLifecycleEvent` consumers / the admin reload endpoint.

Responsibility split:

- `transport::webhook::WebhookTransport` — activate / deactivate / activate_slug /
replace_slug_map / axum router. Owns the routing map, rate limiter, signature
enforcement, replay-window check, and `pre_handle` short-circuit.
- `bootstrap` — `bootstrap_webhook_activations` / `collect_webhook_activations`,
`WebhookSecretResolver`, `WebhookContextFactory`. The composition root
invokes the bootstrap before `build_app`; admin reload uses `collect_*` and
`replace_slug_map` for atomic swaps.
- `events` — `TriggerLifecycleEvent` { Created / Updated / Deleted } +
`TriggerLifecycleSubscriber`. M3.3 ships the consumer; producer-side
wiring is deferred (ADR-0049 § "Out of scope").
- `transport::webhook::provider::EndpointProviderImpl` — implements
`nebula_action::WebhookEndpointProvider` so plugins read the public URL
without knowing the HTTP layer.
- `transport::webhook::key::WebhookKey` — `Programmatic { uuid, nonce }` | `Slug(TriggerCoordinates)`.
- `transport::webhook::routing` — private `RoutingMap` (DashMap) keyed by `WebhookKey`.
- `transport::webhook::ratelimit::WebhookRateLimiter` — per-key sliding-window guard with LRU-capped
path table (#271 mitigation).

Provider catalog (Slack `url_verification`, Stripe `pending_webhook` ping,
Generic `?challenge=…`) lives in `crates/action/src/webhook/providers/`;
each implements `WebhookAction::pre_handle`.

Operator-configured webhook URLs are documented here (and ADR-0049),
not in the OpenAPI spec — promoting the slug surface back into
`/api/v1/openapi.json` is a 1.0 follow-up that requires typed schemas
for every provider's request envelope.

Internal admin reload: `POST /internal/v1/webhooks/reload` (gated by
`X-Internal-Token`) atomically swaps the slug map after consulting
`WebhookActivationRepo::list_active`.

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
├── app.rs              # build_app: OpenApiRouter merge + split_for_parts + middleware stack + serve()
├── state.rs            # AppState (builder) + API-tier port traits (OrgResolver/WorkspaceResolver/
│                       # MembershipStore/SessionStore/AuthBackend etc.)
├── openapi/
│   └── mod.rs          # OpenApiDoc + spec assembly
├── telemetry_init.rs   # init_api_telemetry()
├── trace_capture.rs
├── config/             # Was 1123-line config.rs — split into:
│   ├── mod.rs          # ApiConfig re-exports
│   ├── jwt.rs          # JwtSecret (32-byte min enforcement)
│   ├── errors.rs       # ConfigError
│   ├── sub.rs          # TlsConfig, CookieConfig, CorsConfig, VersioningConfig, PaginationConfig
│   └── env.rs          # ApiConfig::from_env loader
├── error/              # Was 759-line errors.rs — split into:
│   ├── mod.rs          # ApiError (§12.4 seam, #[non_exhaustive])
│   ├── problem.rs      # ProblemDetails (RFC 9457 envelope)
│   └── classify.rs     # HTTP-status mapping helpers
├── extractors/
│   ├── mod.rs
│   ├── json_extractor.rs  # ValidatedJson
│   └── credential.rs
├── middleware/
│   ├── mod.rs
│   ├── auth.rs            # JWT + API-key auth → AuthContext
│   ├── tenancy.rs         # Tenant resolution from path (org/workspace)
│   ├── rbac.rs            # Role-based access control checks
│   ├── csrf.rs            # CSRF token validation
│   ├── rate_limit.rs      # Rate limiting
│   ├── request_id.rs      # Unique request ID propagation
│   ├── security_headers.rs
│   ├── trace_w3c.rs       # W3C Trace Context (traceparent/tracestate) — ADR-0050
│   ├── internal_auth.rs   # X-Internal-Token gate
│   └── idempotency/       # Was 1224-line idempotency.rs — split into:
│       ├── mod.rs
│       ├── layer.rs       # IdempotencyLayer Tower middleware
│       ├── store.rs       # IdempotencyStore trait
│       ├── memory.rs      # InMemoryIdempotencyStore
│       └── key.rs         # IdempotencyKey construction
├── domain/             # Per-domain handlers + DTOs + routes (§12.7 knife seam)
│   ├── mod.rs          # create_routes + build_openapi_router assembly
│   ├── shared.rs       # CursorParams, PaginatedResponse, PaginationParams,
│   │                   # AckResponse, OrgRoleDto, WorkspaceRoleDto
│   ├── workspace.rs    # Tenant-prefix nesting: merges workflow/execution/resource/credential routers
│   ├── internal.rs     # /internal/v1/* ops (plain axum Router; X-Internal-Token)
│   ├── metrics.rs      # Prometheus scrape endpoint
│   ├── workflow/
│   │   ├── mod.rs
│   │   ├── handler.rs  # §13 seam: create_workflow, activate_workflow, start_execution
│   │   └── dto.rs
│   ├── execution/
│   │   ├── mod.rs
│   │   ├── handler.rs  # §13 seam: cancel_execution
│   │   └── dto.rs
│   ├── credential/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   ├── dto.rs
│   │   └── oauth.rs
│   ├── catalog/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   └── dto.rs
│   ├── auth/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   └── backend/    # AuthBackend impls (in-memory, session, password, MFA, OAuth, PAT)
│   ├── org/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   └── dto.rs
│   ├── me/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   └── dto.rs
│   ├── health/
│   │   ├── mod.rs
│   │   ├── routes.rs
│   │   ├── handler.rs
│   │   └── dto.rs
│   └── resource/
│       ├── mod.rs
│       ├── handler.rs
│       └── dto.rs
└── transport/          # Protocol transports (was services/ — NOT business services)
    ├── mod.rs
    ├── credential.rs   # Plane-B credential CRUD stubs (Phase 4 will implement)
    ├── oauth/          # OAuth2 flow transport
    │   ├── mod.rs
    │   ├── flow.rs
    │   ├── http.rs
    │   └── state.rs
    └── webhook/        # Inbound trigger transport (§11.3 / §13.4)
        ├── mod.rs
        ├── transport.rs  # WebhookTransport — activate/deactivate/router
        ├── bootstrap.rs  # bootstrap_webhook_activations, WebhookSecretResolver
        ├── dispatch.rs   # dispatch_inner pipeline
        ├── events.rs     # TriggerLifecycleEvent + subscriber
        ├── provider.rs   # EndpointProviderImpl
        ├── routing.rs    # RoutingMap (private, DashMap)
        ├── signature.rs  # Signature enforcement (ADR-0022)
        ├── replay.rs     # Replay-window check
        ├── ratelimit.rs  # WebhookRateLimiter — per-key sliding-window + LRU cap
        └── key.rs        # WebhookKey: Programmatic { uuid, nonce } | Slug(TriggerCoordinates)
```

### Startup example

The canonical minimal startup is `examples/examples/api_simple_server.rs`
(run: `cargo run -p nebula-examples --example api_simple_server`). The shape below
is faithful to that file:

```rust
use std::sync::Arc;

use nebula_api::{ApiConfig, AppState, app, middleware::InMemoryIdempotencyStore};
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt::init();

    // `from_env` reads `API_JWT_SECRET` (must be 32+ bytes).
    // Set `NEBULA_ENV=development` to get an ephemeral per-process secret.
    let api_config = ApiConfig::from_env()?;

    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let idempotency_store = Arc::new(InMemoryIdempotencyStore::default());

    let state = AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret.clone(),
    )
    .with_api_keys(api_config.api_keys.clone())
    .with_idempotency_store(idempotency_store);

    let bind_address = api_config.bind_address;
    let app = app::build_app(state, &api_config);

    // `app::serve` installs a built-in Ctrl-C / SIGTERM graceful-shutdown handler.
    app::serve(app, bind_address).await?;
    Ok(())
}
```

### Transport binaries

`nebula-api` is a **pure library** — it ships no binaries. The composition root
and the single `nebula-server` binary live in the `apps/server` workspace member.

`nebula-server` selects the active transport(s) via `--transport` (or `NEBULA_TRANSPORT`
env var). Valid values: `api`, `webhook`, `realtime`, `all` (default).

Run locally:

```bash
# All transports (default)
cargo run -p nebula-server

# REST API transport only
cargo run -p nebula-server -- --transport=api

# Webhook ingress only
cargo run -p nebula-server -- --transport=webhook

# Realtime scaffold only (/ws currently returns 501 — Phase 5)
cargo run -p nebula-server -- --transport=realtime
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


