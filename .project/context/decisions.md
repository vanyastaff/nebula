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

## REST API
**Choice**: REST for CRUD, versioned at `/api/v1/`
**Why**: Simpler than GraphQL. WebSocket for real-time execution updates is planned but not yet implemented.

## Credential–Resource Integration via Typed Refs + Events
**Choice**: `CredentialRef<C>` (typed) / `ErasedCredentialRef` in `ResourceComponents`; `Pool<R>` stores credential state + `CredentialHandler<R::Instance>`; rotation via `CredentialRotationEvent` on `EventBus`
**Why**: Avoids circular dep between credential↔resource; `TypeId`-based refs were broken — typed refs enforce protocol at compile time. Resources subscribe via `rotation_subscriber()` and dispatch to affected pools by `CredentialId`.

## Channel Conventions
**Choice**: `mpsc` (bounded) for work queues; `broadcast` for status; `oneshot` for request/response; `RwLock` for shared mutable state
**Why**: Back-pressure via bounded mpsc prevents unbounded queue growth; broadcast decouples status consumers.

## Default Timeouts
**Choice**: HTTP 10 s · Database 5 s · General 30 s
**Why**: Explicit defaults prevent unbounded blocking; overridable via `ApiConfig`.

## No saga / transactional trait today
**Choice**: `nebula-action` has no saga or transactional-action DX trait. Consumers who need compensation model it as two separate actions wired via failure events.
**Why**: A real saga needs a rollback trigger + saga state store + compensation DAG. Until the engine has all three, any DX trait claiming "transactional" would be either dead code or misleading. A previous `TransactionalAction` attempt was deleted for exactly this reason — its `Pending → Executed → Compensated` state machine was unreachable past `Pending` under actual engine dispatch. The public surface stays honest: zero saga support until there's real saga support.
