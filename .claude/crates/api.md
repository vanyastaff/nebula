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
- WebSocket message types are defined in `models` — changing them is a breaking API change.

## Relations
- Depends on nebula-engine, nebula-storage, nebula-credential, nebula-execution. Highest layer — nothing depends on it.

<!-- reviewed: 2026-03-30 — derive Classify migration -->
