# nebula-api
Thin axum REST + WebSocket server — entry point for external clients.

## Invariants
- Handlers are thin: extract data from request → call service → return response. No business logic in handlers.
- All errors follow RFC 9457 Problem Details format. Never return arbitrary JSON errors — use the `errors` module.
- API is versioned at `/v1/`. WebSocket for real-time execution updates.

## Key Decisions
- `AppState` holds injected ports (storage, engine, credential store) — no direct construction of business objects in handlers.
- `services` module contains orchestration logic (calls multiple ports). `handlers` just parse/validate/delegate.
- `extractors` module for common axum extractors (auth, pagination, typed bodies).
- `middleware` handles auth (JWT), rate limiting, tracing, and CORS.

## Traps
- lib.rs and key modules are in Russian (consistent with early project docs). Don't translate.
- `build_app()` returns the axum Router — compose middlewares here, not in individual route modules.
- WebSocket message types in `models` are a breaking API change.
- Catalog registries on `AppState` are `Option` — absent → 503, not panic.
- `validate_workflow_handler` always returns 200 OK — negative result is `{valid: false, errors}`, not 422.
- Auth folds over ALL keys without short-circuit (timing oracle). `.fold()` is intentional — do not replace with `.any()`.
- `api_keys` from `ApiConfig` must be passed to `.with_api_keys()` after `AppState::new()`. `build_app` does not wire this automatically.
- `get_execution_outputs` and `get_execution_logs` call `get_state` first — return 404 for unknown IDs.
- Execution list uses `list_running()` only; workflow-scoped filter is in-memory (TODO).
- `ApiConfig` has manual `Debug` redacting secrets — never add `#[derive(Debug)]`.
- `ApiConfig::from_env` fails closed when `NEBULA_ENV` is unset (defaults to `production`); dev-only ephemeral JWT secret path activates only for `development`/`dev`/`local`.
- **Webhook transport in `api::webhook`.** HTTP ingress for `nebula-action` `WebhookAction` triggers. `WebhookTransport::activate(handler, ctx_template)` generates `(trigger_uuid, nonce)`, builds `EndpointProviderImpl`, injects it into the `TriggerContext` template via `with_webhook_endpoint`, stores `(handler, ctx)` in a `DashMap`-backed `RoutingMap`, returns `ActivationHandle`. Runtime calls `adapter.start(&handle.ctx)`. Router is `POST /{path_prefix}/{trigger_uuid}/{nonce}` merged into `build_app` when `AppState.webhook_transport` is `Some`. Dispatch: body-size check → rate limit → route lookup → `WebhookRequest::try_new` → oneshot → `handler.handle_event` → await oneshot with timeout → write HTTP response. Error mapping: 404/404/413/400/429/500/504 per spec. `WebhookRateLimiter` salvaged verbatim from deleted `crates/webhook/` orphan, wraps `nebula_resilience::SlidingWindow` per-path with a `max_paths` soft cap. Nonce is 128-bit random per activation — stale external hooks pointing at the same UUID can't route to fresh registrations.
- Webhook router must apply `DefaultBodyLimit::max(body_limit_bytes)` in addition to in-handler length checks. `Bytes` extraction without router-level limit can allocate attacker-sized bodies before application checks run.

## Relations
- Depends on nebula-storage, nebula-workflow, nebula-action, nebula-plugin, nebula-runtime, nebula-resilience. Highest layer.

<!-- reviewed: 2026-04-14 — webhook/mod.rs + webhook/provider.rs docstring cleanup for rustdoc (private `routing` module link, redundant explicit link target on `WebhookEndpointProvider`); no structural changes -->

<!-- reviewed: 2026-04-14 -->
