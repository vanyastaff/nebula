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

## Relations
- Depends on nebula-storage, nebula-workflow, nebula-action, nebula-plugin. Highest layer.

<!-- reviewed: 2026-04-07 -->
