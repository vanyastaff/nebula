# Nebula Architecture

## Architecture Pattern
Layered modular monolith implemented as a Rust Cargo workspace with strict one-way crate dependencies.

## Layers
- **Core:** foundational IDs, workflow/execution types, validation, parameters
- **Cross-Cutting:** config, logging, eventbus, resilience, metrics, telemetry
- **Business Logic:** action, credential, resource, plugin
- **Execution:** engine scheduler and runtime orchestration
- **Infrastructure:** storage abstractions and backend implementations
- **Interface/API:** REST + WebSocket endpoints and webhook entrypoints

## Dependency Rules
- No upward or circular dependencies
- `nebula-core` remains minimal and broadly reusable
- Cross-crate communication should prefer event-driven decoupling (eventbus)
- Infrastructure details must not leak into upper layers

## Recommended Folder/Crate Conventions
- Public API in `lib.rs` with docs and explicit module exports
- Per-crate `error` module with `thiserror` enums and `Result` alias
- `prelude` modules for ergonomic re-exports
- Unit tests colocated; integration tests in `tests/`

## Runtime Data Flow (High-Level)
1. Trigger/event enters runtime
2. Runtime resolves workflow DAG via engine
3. Engine schedules executable nodes
4. Actions execute with injected context (credentials/resources/logger)
5. State and outputs are persisted through storage abstraction
6. Events/telemetry emitted for observability and subscribers

## Guardrails
- Prefer typed boundaries over dynamic contracts
- Keep feature additions scoped to the owning crate/layer
- Enforce linting and docs for all public surfaces
- Treat security-sensitive modules (credentials, auth, storage) as high-scrutiny change zones
