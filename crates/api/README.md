---
name: nebula-api
role: API Gateway
status: frontier
last-reviewed: 2026-04-17
canon-invariants: [L2-§4.5, L2-§12.3, L2-§12.4, L2-§13]
related: [nebula-storage, nebula-runtime, nebula-engine, nebula-plugin, nebula-telemetry, nebula-credential]
---

# nebula-api

## Purpose

Provides the HTTP entry point for the Nebula workflow engine. Translates REST
requests into calls against typed port traits (`WorkflowRepo`, `ExecutionRepo`,
`ControlQueueRepo`), then delegates all business logic to the crates below it.
The crate also hosts the `webhook` submodule, which handles inbound trigger
delivery and per-endpoint lifecycle management.

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
  CORS, compression, security headers, auth).
- `ApiConfig` / `ApiConfig::from_env` — runtime configuration; startup fails
  hard on a missing or short JWT secret (no `Default` impl).
- `JwtSecret` — newtype that enforces a 32-byte minimum length at construction;
  any value in hand is valid.
- `ApiError` / `ProblemDetails` — RFC 9457 error envelope; seam for §12.4.
  Every failure path maps to a typed `ApiError` variant with an explicit HTTP
  status — no new ad-hoc 500s for business-logic errors.
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

## Non-goals

- Not a storage driver — no SQL or schema knowledge; see `nebula-storage`.
- Not a workflow engine — no execution logic; see `nebula-engine` / `nebula-runtime`.
- Not an expression evaluator — see `nebula-expression`.
- Not an outbound HTTP client — webhook *delivery* outbound lives in action
  plugins, not here. This module handles inbound receipt only.
- WebSocket / SSE for real-time execution updates — not yet wired end-to-end
  (§4.5: hidden until the engine side exists).
- OpenAPI / Swagger generation — planned but not yet present.

## Maturity

See `docs/MATURITY.md` row for `nebula-api`. Short summary:

- API stability: frontier
- JWT validation middleware, rate limiting, and pagination are stubs; they are
  intentionally hidden under `TODO` rather than advertised as live (§4.5).
- Webhook transport is functional but delivery guarantee is at-least-once;
  exactly-once is not claimed.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.5 (operational honesty), §12.2 (durable
  outbox), §12.3 (local-first), §12.4 (RFC 9457 errors), §13 (knife scenario
  steps 1–3 and 5).
- Satellite docs: `docs/INTEGRATION_MODEL.md`, `docs/OBSERVABILITY.md`.
- Siblings: `nebula-storage` (port impls), `nebula-engine` (execution logic),
  `nebula-runtime` (action registry), `nebula-plugin` (plugin registry),
  `nebula-credential` (secret store), `nebula-telemetry` (metrics export).

## Appendix

### Source layout

```
src/
├── lib.rs              # Crate root, public re-exports
├── app.rs              # Router assembly + middleware stack
├── config.rs           # ApiConfig, JwtSecret (enforces 32-byte minimum)
├── state.rs            # AppState (port traits only — no concrete impls)
├── errors.rs           # RFC 9457 ProblemDetails + ApiError (§12.4 seam)
├── extractors/         # ValidatedJson and other custom extractors
├── handlers/
│   ├── health.rs       # GET /health, GET /ready
│   ├── workflow.rs     # Workflow CRUD + activate (§13 steps 1–2)
│   └── execution.rs    # Start / cancel executions (§13 steps 3, 5)
├── middleware/
│   ├── auth.rs         # JWT + API-key auth
│   ├── rate_limit.rs   # Rate limiting (stub)
│   ├── request_id.rs   # Unique request ID propagation
│   └── security_headers.rs
├── models/             # Request / Response DTOs
│   ├── health.rs
│   ├── workflow.rs
│   └── execution.rs
├── routes/             # Domain-scoped route builders
│   ├── mod.rs          # create_routes()
│   ├── health.rs
│   ├── workflow.rs
│   └── execution.rs
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

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Liveness check (always available) |
| `GET` | `/ready` | Readiness check (verifies dependencies) |
| `GET` | `/api/v1/workflows` | List workflows |
| `POST` | `/api/v1/workflows` | Create workflow (§13 step 1) |
| `GET` | `/api/v1/workflows/:id` | Get workflow by ID |
| `PUT` | `/api/v1/workflows/:id` | Update workflow |
| `DELETE` | `/api/v1/workflows/:id` | Delete workflow |
| `POST` | `/api/v1/workflows/:id/activate` | Activate workflow — runs validation (§13 step 2) |
| `GET` | `/api/v1/workflows/:id/executions` | List executions |
| `POST` | `/api/v1/workflows/:id/executions` | Start execution — 202 Accepted (§13 step 3) |
| `GET` | `/api/v1/executions/:id` | Get execution status |
| `POST` | `/api/v1/executions/:id/cancel` | Cancel execution — durable signal (§13 step 5) |
| `POST` | `/webhooks/:trigger_uuid/:nonce` | Inbound webhook trigger (mounted when `webhook_transport` is set) |
