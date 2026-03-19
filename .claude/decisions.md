# Architecture Decisions

## serde_json::Value as Universal Data Type
**Choice**: `serde_json::Value` everywhere — workflow data, action I/O, config, expressions
**Why**: Eliminates conversion layers; dates use ISO-8601, decimals use base64 convention.

## One-Way Layer Dependencies
**Choice**: Infra → Core → Business → Exec → API, enforced by `cargo deny`
**Why**: Prevents circular deps. Cross-cutting crates (log, config, resilience, eventbus, metrics, telemetry) are exempt.

## EventBus for Cross-Crate Signals
**Choice**: `EventBus<E>` for all cross-crate notifications
**Why**: Prevents circular deps (especially credential↔resource); keeps layers decoupled.

## InProcessSandbox Only (Phase 2)
**Choice**: Actions run in-process — no OS-process or WASM isolation
**Why**: Phase 3 target. Adding isolation now would require major runtime changes.

## AES-256-GCM for Credentials at Rest
**Choice**: AES-256-GCM; `SecretString` zeroizes on drop
**Why**: Credentials always encrypted before storage — no exceptions.

## Actions Use DI via Context
**Choice**: `Context` trait injects credentials, resources, logger
**Why**: Actions may run concurrently; testable without real infrastructure.

## NodeId Separate from ActionKey
**Choice**: `NodeId` = graph position; `ActionKey` = type identity
**Why**: Multiple nodes can run the same action. `NodeDefinition.action_key` is the binding.

## PostgreSQL + MemoryStorage
**Choice**: `MemoryStorage` for tests, `PostgresStorage` for production
**Why**: Tests never hit the database; `Storage` trait abstracts both.

## REST + WebSocket
**Choice**: REST for CRUD, WebSocket for real-time, versioned at `/v1/`
**Why**: Simpler than GraphQL; WebSocket handles live execution updates.
