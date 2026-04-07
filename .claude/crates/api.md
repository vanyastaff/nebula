# nebula-api
Thin axum REST + WebSocket server — entry point for external clients.

## Invariants
- Handlers are thin: extract → delegate → respond. No business logic in handlers.
- All errors follow RFC 9457 Problem Details format.
- API versioned at `/v1/`.

## Key Decisions
- `AppState` holds injected ports (storage, engine, resource manager). `with_resource_manager()` adds optional `ResourceManager`.
- `GET /api/v1/resources` lists resources (topology, phase, generation, metrics). Returns 503 if resource manager not configured.
- `services` = orchestration, `handlers` = parse/validate/delegate, `extractors` = common axum extractors.

## Traps
- lib.rs and key modules are in Russian. Don't translate.
- `build_app()` returns the Router — compose middlewares here, not in route modules.
- WebSocket message types in `models` — changing them is a breaking API change.

## Relations
- Depends on nebula-engine, nebula-storage, nebula-credential, nebula-execution, nebula-resource. Highest layer — nothing depends on it.

<!-- reviewed: 2026-04-07 — resource manager + GET /resources endpoint added -->
