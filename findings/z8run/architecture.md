# z8run — Architectural Decomposition

## 0. Project Metadata

- **Repo:** https://github.com/z8run/z8run
- **Stars:** 5 | **Forks:** 2 | **Watchers:** 5
- **Latest tag:** v0.2.0 (2026-04-01); initial release v0.1.0 (2026-03-06)
- **License:** Apache-2.0 OR MIT (dual)
- **Governance:** Solo maintainer (hello@z8run.org); CODEOWNERS file present
- **Crates.io:** z8run-core (86 downloads), z8run-cli (31 downloads) — negligible production adoption
- **MSRV:** 1.91 (Cargo.toml:10 `rust-version = "1.91"`)
- **Edition:** 2021

---

## 1. Concept Positioning [A1, A13, A20]

**Author's description (README.md:36):**
> "z8run is an open-source visual flow engine built from the ground up in Rust for performance, safety, and extensibility. Inspired by tools like Node-RED and n8n, z8run is designed for developers who need real-time automation with a modern stack."

**My description after reading code:**
z8run is a Node-RED-style visual DAG flow engine where flows are directed graphs of `NodeExecutor` implementations connected by typed ports. It has a single-binary deployment model (SQLite-embedded for dev, PostgreSQL for production), a WASM plugin sandbox via wasmtime, 35+ built-in nodes including 10 AI/LLM nodes, and a React-based drag-and-drop editor.

**Comparison with Nebula:**
Both are workflow engines targeting n8n use cases in Rust. The surface-level similarities are significant — tokio runtime, DAG execution, credential storage, WASM plugin target — but the abstraction depth differs. Nebula has 26 purpose-built crates with sealed type hierarchies, 4-level TypeDAG, 5 action kinds, and a formal credential lifecycle (State/Material split, LiveCredential, blue-green refresh). z8run has 7 crates, a runtime-only `PortType` enum for "compatibility", and a simpler credential vault (store/retrieve with no lifecycle beyond CRUD). z8run ships working AI features (LLM, embeddings, vector store, AI agent) today, where Nebula's AI story is still "generic actions + future plugin".

---

## 2. Workspace Structure [A1]

**Crate inventory (Cargo.toml:1-8):**

```
z8run/                          5 library crates + 2 binaries
├── crates/
│   ├── z8run-core              Flow engine, DAG, scheduler, 35+ nodes
│   ├── z8run-protocol          Binary WebSocket protocol (bincode, 11-byte header)
│   ├── z8run-storage           SQLite/PgSQL persistence + AES-256-GCM vault
│   ├── z8run-runtime           WASM sandbox (wasmtime v42), plugin manifest/registry
│   └── z8run-api               Axum REST + WebSocket server, JWT auth
├── bins/
│   ├── z8run-cli               CLI entry point, serves HTTP, plugin commands
│   └── z8run-server            Embedded frontend binary (rust-embed)
└── frontend/                   React + TypeScript visual editor
```

**Layer separation:** Roughly bottom-up: `z8run-core` → `z8run-storage` + `z8run-runtime` + `z8run-protocol` → `z8run-api` → `bins/`. There is no strict domain layering like Nebula's infrastructure/domain/application/presentation. The most egregious cross-cutting is that `z8run-core` directly embeds `reqwest` and `sqlx` for node implementations (database node, HTTP request node), coupling the domain layer to infrastructure.

**Feature flags:** None declared in any crate. The workspace dependency `sqlx` bundles sqlite + postgres + mysql features unconditionally regardless of target deployment (`Cargo.toml:48-49`).

**Umbrella crate:** None. Consumers depend on individual crates.

**Comparison with Nebula:** Nebula has 26 crates vs z8run's 7. Nebula separates nebula-error, nebula-resilience, nebula-credential, nebula-resource, nebula-action, nebula-engine, nebula-tenant, nebula-eventbus as separate concerns. z8run conflates credential vault into storage, resilience into inline timeouts, and all node implementations directly into z8run-core.

---

## 3. Core Abstractions [A3, A17]

### A3.1 — Trait shape

The central abstraction is `NodeExecutor` (`crates/z8run-core/src/engine.rs:75-96`):

```rust
#[async_trait::async_trait]
pub trait NodeExecutor: Send + Sync {
    async fn process(&self, msg: FlowMessage) -> Z8Result<Vec<FlowMessage>>;
    async fn configure(&mut self, config: serde_json::Value) -> Z8Result<()>;
    async fn validate(&self) -> Z8Result<()>;
    async fn shutdown(&self) -> Z8Result<()> { Ok(()) }
    fn set_event_emitter(&mut self, _tx: broadcast::Sender<EngineEvent>) {}
    fn node_type(&self) -> &str;
}
```

The trait is **open** (any external crate can implement it; no sealing mechanism). It is trait-object compatible (`Arc<dyn NodeExecutor>` is used in the engine). There are **no associated types** (Input/Output/Error are not typed — Input is always `FlowMessage` with `serde_json::Value` payload; Output is `Vec<FlowMessage>` with the same). This is a fundamental design decision: complete type erasure at the node boundary.

**Companion trait** `NodeExecutorFactory` (`crates/z8run-core/src/engine.rs:119-125`):
```rust
#[async_trait::async_trait]
pub trait NodeExecutorFactory: Send + Sync {
    async fn create(&self, config: serde_json::Value) -> Z8Result<Box<dyn NodeExecutor>>;
    fn node_type(&self) -> &str;
}
```

Factories are registered by string type name in a `HashMap<String, Arc<dyn NodeExecutorFactory>>` inside `FlowEngine` (`crates/z8run-core/src/engine.rs:113`). There is no compile-time registry — all node dispatch is at runtime via string lookup.

### A3.2 — I/O shape

Input: `FlowMessage` (`crates/z8run-core/src/message.rs:12-28`) carrying `serde_json::Value` as payload. No generic type parameter — complete type erasure. Port type compatibility (`PortType` enum in `node.rs:12-27`) is checked only at flow-build time via `is_compatible_with`, not enforced in the `process` signature.

Output: `Vec<FlowMessage>` — a node can emit zero or more messages on named ports (port name is `source_port` on `FlowMessage`). Streaming is a special case: nodes with streaming LLM output call `event_tx.send(EngineEvent::StreamChunk {...})` via an injected broadcast sender (`crates/z8run-core/src/nodes/llm.rs:409-416`), not via the return value.

### A3.3 — Versioning

No versioning mechanism exists. Nodes are identified only by their `node_type()` string (e.g., `"llm"`, `"http-request"`, `"cron-trigger"`). There is no v1/v2 distinction, no `#[deprecated]`, no migration support. Flow definitions store node type strings and node JSON config; if a node type is removed, the flow simply fails at execution with `Z8Error::Internal("No executor registered for type '...'")` (`crates/z8run-core/src/engine.rs:356-358`).

### A3.4 — Lifecycle hooks

The trait has: `configure`, `validate`, `process`, `shutdown`. No `pre`/`post`/`on-failure` hooks. `configure` is called once at factory creation time; `validate` is called after `configure`; `process` is called per message. Cancellation is not supported — there is no `CancellationToken` or cooperative cancellation. The only timeout mechanism is a `tokio::time::timeout` wrapping the webhook response wait (`crates/z8run-api/src/routes.rs:920`), not per-node.

### A3.5 — Resource and credential deps

Nodes declare no typed resource or credential dependencies. Instead, credentials are resolved at flow-start time by the `canvas_to_flow` function (`crates/z8run-api/src/routes.rs:405-438`): any `serde_json::Value::String` starting with `"vault:"` is replaced with the plaintext secret before the `config` is passed to `NodeExecutorFactory::create`. This means: (a) credentials flow as plaintext strings in node config after resolution; (b) there is no compile-time or type-level declaration of which nodes need which credentials; (c) no live rotation — vault references are resolved once at flow start.

### A3.6 — Retry/resilience attachment

No retry policy exists at the node level. There is no per-action retry config, no backoff strategy, no circuit breaker, no bulkhead. LLM nodes have a hardcoded `timeout_ms` field (default 30,000 ms for `LlmNode`, `crates/z8run-core/src/nodes/llm.rs:717`). The `StructuredOutputNode` has a `retries` field for re-prompting on JSON parse failure (`crates/z8run-core/src/nodes/structured_output.rs:24`), but this is LLM-specific, not a general resilience layer.

### A3.7 — Authoring DX

The `node_factory!` macro (`crates/z8run-core/src/lib.rs` exposes it) auto-generates a `NodeExecutorFactory` implementation from a struct and default field values (example: `crates/z8run-core/src/nodes/llm.rs:707-721`). The `configure_fields!` macro dispatches config JSON fields to struct fields by type tag (`str`, `str_lower`, `f64`, `u64`, `bool`, `value`) — `crates/z8run-core/src/nodes/llm.rs:153-164`. "Hello world" node requires: struct definition + `impl NodeExecutor` + `node_factory!` call — approximately 30 lines.

### A3.8 — Metadata

Node metadata (display name, description, icon, category) lives in the manifest for WASM plugins (`crates/z8run-runtime/src/manifest.rs:9-38`). For native nodes, metadata is absent from Rust structs — it lives exclusively in the React frontend TypeScript node definitions. There is no compile-time metadata, no i18n.

### A3.9 — vs Nebula

Nebula has **5 action kinds** (Process/Supply/Trigger/Event/Schedule), each with sealed trait + `Input`/`Output`/`Error` associated types, providing compile-time type safety for port connections. z8run has **one trait** (`NodeExecutor`) applied uniformly to all node types with no sub-classification. This is a deliberate simplicity trade-off: z8run eschews compile-time guarantees entirely in favor of a uniform JSON-in/JSON-out interface. The `PortType` enum (`node.rs:12-27`) provides only 7 values (`Any/String/Number/Boolean/Object/Array/Binary`) and `Any` is compatible with everything — any strict port checking done at `Flow::connect` time is circumvented by the frontend always using `PortType::Any` for default ports (`crates/z8run-api/src/routes.rs:499, 510`).

---

## 4. DAG / Execution Graph [A2, A9, A10]

### Graph model

A flow is a `Flow` struct (`crates/z8run-core/src/flow.rs:114-143`) containing `Vec<Node>` and `Vec<Edge>`. `Node` carries `Vec<Port>` for inputs and outputs. `Edge` connects `(from_node, from_port)` to `(to_node, to_port)`. There is no petgraph dependency — the graph is stored as plain Vec<> and traversed ad-hoc.

**Cycle detection:** Kahn's algorithm implemented inline in `Flow::validate_acyclic` (`crates/z8run-core/src/flow.rs:255-291`). Called at compile time by `ExecutionPlan::compile`.

**Port type checking:** `PortType::is_compatible_with` (`crates/z8run-core/src/node.rs:46-48`) — L1-only (runtime enum comparison). No L2 TypeId, L3 refinement predicates, or L4 petgraph as in Nebula. In practice, the frontend passes `PortType::Any` for all dynamic nodes, nullifying the check.

### Scheduler

`ExecutionPlan` (`crates/z8run-core/src/scheduler.rs:22-105`) groups nodes into parallel execution steps using Kahn's algorithm variant: all zero-in-degree nodes execute in step 0, their successors in step 1, etc. This is breadth-first topological leveling, not work-stealing.

### Concurrency model

The engine spawns one `tokio::task` per node per step (`crates/z8run-core/src/engine.rs:340`). Nodes at the same step are fully parallel. Communication between nodes uses `tokio::sync::mpsc` channels — one `(tx, rx)` pair per node that has incoming edges (`engine.rs:284-300`). Root nodes (no incoming edges) receive a synthetic trigger message directly. Channel buffer size defaults to `max(flow.config.buffer_size, self.default_buffer_size)` = 256 (`engine.rs:296`).

**!Send handling:** `NodeExecutor: Send + Sync` is required. WASM nodes (which use `wasmtime::Store` that is not `Sync`) are wrapped in `Arc<Mutex<WasmInstance>>` (`crates/z8run-runtime/src/executor.rs:18`). There is no `!Send` isolation or thread-local sandbox — all tasks run on the standard tokio multi-thread pool.

**Comparison with Nebula:** Nebula's frontier-based scheduler supports checkpoint recovery and append-only log replay. z8run has no persistence of execution state — a crash loses all in-flight data. z8run's scheduler is simpler (breadth-first leveling) but lacks work-stealing, and there is no checkpoint mechanism.

---

## 5. Persistence and Recovery [A8, A9]

### Storage model

`FlowRepository` trait (`crates/z8run-storage/src/repository.rs:11-38`) has `save_flow / get_flow / list_flows / delete_flow / search_flows / *_for_user / *_by_user`. Implementations: `PgStorage` (PostgreSQL, `crates/z8run-storage/src/postgres.rs`) and `SqliteStorage` (`crates/z8run-storage/src/sqlite.rs`).

Flows are serialized as `JSONB` in PostgreSQL and `JSON TEXT` in SQLite (`crates/z8run-storage/src/migration.rs:9: data JSONB NOT NULL`). The entire `Flow` struct serializes as one JSON blob. The canvas React Flow state (node positions, viewport) is stored in `flow.metadata.positions` as a nested JSON object.

**Execution log:** `ExecutionRepository` trait (`crates/z8run-storage/src/repository.rs:80-99`) records `record_start` / `record_completion` per execution. There is no append-only event log, no event sourcing, no per-node-step logging in the database. The `ExecutionRecord` has a `node_logs: serde_json::Value` field but it is populated as `'{}'` in the schema and never filled in the current code — it is a placeholder.

### Recovery semantics

There is **no crash recovery**. If the server process dies mid-execution, in-flight flow state is lost. Executions stored as "running" in the DB will remain "running" permanently (no reconciliation on startup). There is no frontier-based checkpoint, no replay, no dead-letter queue.

**Comparison with Nebula:** Nebula has frontier-based checkpoint recovery with an append-only execution log. This is a fundamental gap in z8run — it targets Node-RED-style "fire and forget" automation rather than Temporal-style durable execution.

---

## 6. Credentials / Secrets [A4]

### A4.1 — Existence

**Yes, a dedicated credential layer exists.** `CredentialVault` trait (`crates/z8run-storage/src/credential_vault.rs:17-36`) with both `PgCredentialVault` and `SqliteCredentialVault` implementations. The vault is API-accessible via `/api/v1/vault` routes (`crates/z8run-api/src/routes.rs:37-41`). Additionally, "connections" (a second credential type for named integrations) are stored in the `connections` table with encrypted `encrypted_data` (`migration.rs:66-78`).

### A4.2 — Storage

**AES-256-GCM at-rest encryption** via the `aes-gcm` crate (`Cargo.toml:57: aes-gcm = "0.10"`). The `VaultCrypto` struct (`credential_vault.rs:39-88`) derives the key from a string secret using `SHA-256(secret)` to get 32 bytes, then constructs `Aes256Gcm`. Each record stores `(encrypted_value BYTEA, nonce BYTEA)`. Backend is the same DB as flows (SQLite or PostgreSQL — not a separate vault). No external vault (no HashiCorp Vault, no AWS Secrets Manager).

Key rotation: **not implemented**. There is no key rotation endpoint, no dual-key decryption period. Rotating the `Z8_VAULT_SECRET` env var would permanently lose access to existing credentials.

### A4.3 — In-memory protection

**No `secrecy::Secret<T>` or `zeroize`** — searched for both: zero results in `--include="*.rs"`. Decrypted credential strings are plain `String` values that live in heap memory until garbage collected. No memory locking, no zeroing on drop.

### A4.4 — Lifecycle

CRUD + `list_keys`. `issue_temporary_token` method exists (`credential_vault.rs:160-185`) but its format is `hex(nonce):hex(ciphertext)` of `"value|expiry"` — not a signed JWT, not revocable, not a proper token exchange pattern. The flow engine resolves vault references at flow-start via `resolve_vault_refs` (`routes.rs:405-438`); there is no watch/push for rotation events.

### A4.5 — OAuth2/OIDC

**Not implemented.** Searched for `OAuth`, `oauth`, `OIDC`, `pkce`, `client_credentials` — zero results in `.rs` files. The vault stores opaque strings only. OAuth2 flows are left entirely to the user.

### A4.6 — Composition

Each node config field that starts with `"vault:"` is independently resolved. Multiple vault references per node config are supported (recursive resolution in `resolve_vault_refs`). No delegation or SSO pattern.

### A4.7 — Scope

All credentials are **global** (no per-user, per-workspace, or per-tenant scoping in the vault schema). The `credentials` table has `key TEXT PRIMARY KEY` with no `user_id` foreign key (`migration.rs:37-43`). The connections table has `user_id` FK (`migration.rs:68`) but the main vault does not. This is a security gap for multi-user deployments.

### A4.8 — Type safety

No validated/unvalidated state distinction, no phantom types per credential kind. Credentials are plain strings.

### A4.9 — vs Nebula

Nebula has: State/Material split (typed state + opaque material) | LiveCredential with `watch()` | blue-green refresh | `OAuth2Protocol` blanket adapter | `DynAdapter` type erasure.

z8run has: AES-256-GCM store/retrieve | `issue_temporary_token` (non-standard, not revocable) | `vault:key` reference resolution at flow-start | no lifecycle beyond CRUD.

**Delta:** z8run has basic at-rest encryption (good). Nebula has a substantially deeper lifecycle model: typed state, live rotation notification, blue-green refresh, OAuth2 as a first-class protocol. z8run's vault is comparable to n8n's credential storage but without OAuth2 flows.

---

## 7. Resource Management [A5]

### A5.1 — Existence

**No first-class resource abstraction.** Each node creates its own resources:
- `LlmNode` creates `reqwest::Client::new()` on every `process()` call (`crates/z8run-core/src/nodes/llm.rs:68`)
- `DatabaseNode` creates a new connection pool inside `process()` using the connection string from config
- The MQTT node manages its own `rumqttc::AsyncClient` per node instance

There is no `Resource` trait, no global pool registry, no scope-based lifetime management.

### A5.2 — Scoping

Not implemented. All "resources" are ephemeral per-call allocations with no scope concept.

### A5.3 — Lifecycle hooks

The only lifecycle hook is `shutdown()` on `NodeExecutor` (defaulting to `Ok(())`). No `init()`, no `health_check()`.

### A5.4 — Reload

No hot-reload, no blue-green, no ReloadOutcome, no generation counter.

### A5.5 — Sharing

Nodes in the same flow execution do not share resource instances — each node task spawned by the engine creates its own executor via `factory.create(config)` each time it runs (`engine.rs:359, 383`). The `reqwest::Client` is created per-process call, not per-node. Pooling is absent except for the database connection pool managed by `sqlx::PgPool` or `SqlitePool` in `z8run-storage`, but those pools service the storage layer, not node HTTP clients.

### A5.6 — Credential deps

As described in A4.5: vault references resolved once at flow-start, injected as plaintext strings in config. No per-resource notification on rotation.

### A5.7 — Backpressure

`mpsc::channel` with configurable buffer size (default 256) is the only backpressure mechanism (`engine.rs:296`). No acquire timeout, no bounded queue with priority levels.

### A5.8 — vs Nebula

Nebula has 4 scope levels (Global/Workflow/Execution/Action), `ReloadOutcome`, generation tracking, `on_credential_refresh`. z8run has none of these. This is the largest architectural gap: z8run creates new HTTP clients on every node execution, which is expensive and incorrect for production workloads (no connection reuse, no keep-alive pools across calls).

---

## 8. Resilience [A6, A18]

### Error handling

`Z8Error` enum (`crates/z8run-core/src/error.rs`) uses `thiserror::Error`. Error variants: `CycleDetected`, `NodeNotFound`, `PortNotFound`, `TypeMismatch`, `InvalidEdge`, `NodeTimeout`, `NodeExecution`, `FlowNotRunnable`, `ChannelClosed`, `InvalidConfig`, `Internal`, `Serialization`. No `ErrorClass` enum (no transient/permanent/cancelled classification). No `StorageError` in z8run-core; `StorageError` lives in z8run-storage.

### Retry/resilience

**No dedicated resilience crate.** Searched for `retry`, `circuit.breaker`, `bulkhead`, `backoff` in `.rs` files — no results. There is:
- Per-node `timeout_ms` config (LLM, HTTP request, database — all hardcoded defaults)
- `StructuredOutputNode.retries` for JSON re-prompting (LLM-specific)
- `tokio::time::timeout` at the webhook response handler (`routes.rs:920`) — global 10-second cap
- Rate limiter in `z8run-api/src/rate_limit.rs` (token bucket for HTTP endpoints)

No circuit breaker, no hedging, no bulkhead, no unified `ErrorClassifier`.

**Comparison with Nebula:** Nebula has `nebula-resilience` as a standalone crate with retry/CB/bulkhead/timeout/hedging and a unified `ErrorClassifier`. This is entirely absent from z8run.

---

## 9. Expression / Data Routing [A7]

### Expression DSL

**No expression DSL.** z8run has no equivalent to Nebula's `$nodes.foo.result.email` syntax. Node configurations accept JSON-typed values. The `json_path` utility (`crates/z8run-core/src/utils/json_path.rs`) provides dot-notation access like `req.body.name` within node logic (e.g., switch/filter nodes), but this is not user-accessible as a DSL — it is internal to node implementations.

The `function` node (`crates/z8run-core/src/nodes/function.rs`) allows users to write JavaScript-style transformations as strings — but these are not evaluated at runtime in the Rust codebase. Instead, the function body is stored as config and evaluated in the frontend (TypeScript sandbox) or may be a future feature. The `crates/z8run-core/src/nodes/function.rs` accepts `code: String` in config but does not execute it natively.

**Searched for:** `eval`, `expression`, `jinja`, `handlebars`, `template_engine`, `rhai`, `lua` — no Rust evaluation engine found. The `PromptTemplateNode` does basic `{{variable}}` replacement in prompt strings (`crates/z8run-core/src/nodes/prompt_template.rs`), but this is not a general expression engine.

**Comparison with Nebula:** Nebula has a custom expression engine with 60+ functions, type inference, and sandboxed eval. z8run has no equivalent — data transformation is delegated to the `function` node (client-side or TBD).

---

## 10. Plugin / Extension System [A11]

### 10.A — Plugin BUILD Process

**A11.1 — Format:** WASM module (`.wasm` file) + `manifest.toml` in a plugin directory. Manifest format is TOML, parsed into `PluginManifest` (`crates/z8run-runtime/src/manifest.rs:9-38`). A single `.wasm` + `manifest.toml` = one plugin. Multiple plugins per package: no.

**A11.2 — Toolchain:** Plugins compile to WASM using any language targeting `wasm32-unknown-unknown` or `wasm32-wasi`. There is no official SDK or cargo template in this repository. The ABI contract is implicit: the WASM module must export `z8_alloc(size: i32) -> i32`, `z8_dealloc(ptr: i32, size: i32)`, `z8_process(ptr: i32, len: i32) -> i32`, `z8_configure(ptr: i32, len: i32) -> i32`, `z8_validate() -> i32`, `z8_node_type() -> i32` (`crates/z8run-runtime/src/sandbox.rs:141, 199, 224, 245, 270, 290`). These are raw pointer/length pairs with a 4-byte length-prefix convention for return values. No WIT interface definition, no wit-bindgen.

**A11.3 — Manifest content:** Required fields: `name`, `version`, `description`, `author`, `category`, `inputs`, `outputs`, `wasm_file`. Optional: `license`, `icon`, `min_runtime_version`. Capabilities: `network: bool`, `filesystem: bool`, `allowed_paths: Vec<String>`, `env_vars: bool`, `allowed_env: Vec<String>`, `memory_limit_mb: u64` (`manifest.rs:54-74`).

**A11.4 — Registry/discovery:** Local directory scan only (`PluginRegistry::scan` — `crates/z8run-runtime/src/registry.rs:43-76`). The plugins directory path is configurable via `Z8_DATA_DIR`. No remote registry, no OCI registry, no signing, no version pinning. Install via CLI: `z8run plugin install ./plugin.wasm` or `z8run plugin install ./plugin-dir/`.

### 10.B — Plugin EXECUTION Sandbox

**A11.5 — Sandbox type:** **wasmtime** (`Cargo.toml:46: wasmtime = "42"`). The sandbox is configured via `SandboxConfig` (`crates/z8run-runtime/src/sandbox.rs:13-33`): memory limit (default 256 MB), fuel limit (default 0 = unlimited), capability flags, debug mode. Wasmtime features enabled: `wasm_simd`, `wasm_bulk_memory`, `wasm_reference_types`, `wasm_multi_value` (`sandbox.rs:50-54`). The WASM interface is **raw linear memory** with pointer/length calling convention, **not** WIT/WASI components.

WASI is referenced (`struct WasiState;` at `sandbox.rs:36`) but not connected — `WasiState` is an empty struct used as the `Store<WasiState>` type parameter. No WASI imports are linked (`Linker::new(&self.engine)` with no `add_to_linker` calls, `sandbox.rs:103`). This means plugins currently have **no WASI access** — no filesystem, no network, no clocks — even if `capabilities.network = true` is declared. The capability declarations in the manifest are stored but **not enforced** (there is no `wasmtime_wasi::WasiCtxBuilder` in the codebase).

**A11.6 — Trust boundary:** Limited. Memory limit is enforced via the 100 MB sanity check in `read_from_memory` (`sandbox.rs:181-185`). Fuel limit is optional and defaults to 0 (unlimited). CPU time limits: none. Network and filesystem: none enforced (despite capability fields in manifest). The sandbox isolates memory (wasmtime guarantees) but does not implement the capability security model declared in the manifest.

**A11.7 — Host↔plugin calls:** Marshaling is a **custom raw ABI**: JSON string → UTF-8 bytes → linear memory via `z8_alloc` → `z8_process(ptr, len)` → result at returned ptr with 4-byte length prefix → `z8_dealloc` (`sandbox.rs:134-235`). No async crossing (all calls are sync blocking). No host-provided functions (no imports linked). Error propagation: non-zero return from `z8_configure` or `z8_validate` is treated as error.

**A11.8 — Lifecycle:** WASM instances are created fresh on every `NodeExecutorFactory::create` call (`executor.rs:149-155`), meaning each flow execution instantiates a new WASM module. No hot-reload, no persistent instance across calls, no crash recovery. Instantiation overhead is paid per execution.

**A11.9 — vs Nebula:** Nebula targets wasmtime + WIT/wit-bindgen + capability-based security + Plugin Fund commercial monetization. z8run uses wasmtime + raw pointer ABI + partial capability declarations (declared but not enforced) + no commercial model. z8run's WASM execution is working but the capability security layer and WASI integration are stubs. Nebula's plugin model (per the plugin-v2 spec) is more thoroughly designed but not yet implemented. **Both are incomplete at different levels.**

---

## 11. Trigger / Event Model [A12]

### A12.1 — Trigger types

Built-in trigger types:
- **Webhook** (`crates/z8run-core/src/nodes/webhook.rs`, `webhook_trigger.rs`): HMAC-SHA256 verification, event type filtering, Bearer/Basic/HMAC auth modes
- **HTTP In** (`nodes/http_in.rs`): generic HTTP endpoint trigger
- **Cron Trigger** (`nodes/cron_trigger.rs`): schedule-based trigger with timezone field
- **Timer** (`nodes/timer.rs`): interval-based trigger
- **MQTT** (`nodes/mqtt.rs`): subscribe to MQTT topic
- No: Kafka, RabbitMQ, NATS, pubsub, Redis streams, FS watch, DB change/CDC, internal event (beyond the broadcast channel for UI events)

### A12.2 — Webhook

URL format: `/hook/{flow_id}` or `/hook/{flow_id}/{*path}` — per-flow namespace prevents collisions (`routes.rs:59-63`). URL is stable (deterministic based on flow UUID). Registration: routes are registered in-memory when `POST /api/v1/flows/{id}/start` is called (`routes.rs:316-329`). HMAC-SHA256 signature verification with constant-time comparison (`webhook.rs:45-68`). Bearer and Basic auth modes also supported (`routes.rs:748-792`). Rate limiting: partial implementation in `rate_limit.rs` but **not wired** to the hook handler. Idempotency key: not implemented.

### A12.3 — Schedule

`CronTriggerNode` (`nodes/cron_trigger.rs`) accepts a cron expression string and timezone string. However, the cron firing mechanism is **not implemented in the Rust code** — `process()` just returns a trigger payload when called. There is no cron scheduler that actually fires the flow at scheduled times. The `validate()` method checks that the cron expression has 5-6 fields but does not validate the syntax beyond field count. DST handling: none. Missed schedule recovery: none. Distributed double-fire prevention: none.

### A12.4 — External events

MQTT subscribe is the only message broker integration (`nodes/mqtt.rs`). No Kafka, no Redis streams, no NATS. The MQTT implementation uses `rumqttc::AsyncClient` (`Cargo.toml:70: rumqttc = "0.25"`).

### A12.5 — Reactive vs polling

HTTP In and Webhook are reactive (server-pushed). MQTT subscribe is reactive. Timer/Cron are polling-style (timer interval). The cron is currently a "node that fires when triggered" not a "node that fires itself on schedule."

### A12.6 — Trigger→workflow dispatch

1:1 dispatch: one webhook call = one flow execution. The `execute_with_trigger` path passes a `FlowMessage` with the HTTP request data to the flow's root nodes (`routes.rs:896-907`). No fan-out, no conditional triggers, no replay.

The response to the webhook caller is handled via a `oneshot::channel` — the http-out node sends its response back through `state.webhook_responders` (`routes.rs:889-911`). This is a synchronous request-response pattern with a 10-second timeout.

### A12.7 — Trigger as Action

In z8run, trigger nodes (webhook, http-in, cron, timer, MQTT) implement `NodeExecutor` like all other nodes — they are not a distinct trait or kind. They are "root nodes" (no incoming edges) that generate initial messages when executed. This is single-stage: no separate Source/Event/TriggerAction decomposition as in Nebula.

### A12.8 — vs Nebula

Nebula: `Source` trait normalizes raw inbound (HTTP req / Kafka msg / cron tick) → typed `Event` → `TriggerAction` with `Input = Config` (registration) and `Output = Event` (payload). This is a 2-stage pipeline with a formal contract.

z8run: A trigger node is just a regular `NodeExecutor` that returns a `FlowMessage` when `process()` is called. There is no Source/Event separation. The trigger registration mechanism (webhook route memory map) is in the API layer, not in the core trigger model.

**The cron trigger is broken** — it has no scheduler to actually fire it at the configured time. This is a significant gap for production scheduling use cases.

---

## 12. Multi-tenancy [A14]

**Partial implementation.** User accounts exist with JWT auth and `user_id` FK on flows (`migration.rs:80`). The `list_flows_by_user` / `get_flow_for_user` / `delete_flow_for_user` methods enforce per-user flow ownership at the API layer.

However:
- The credential vault has **no user_id scoping** (`credentials` table: `key TEXT PRIMARY KEY` only — shared globally across all users)
- No RBAC beyond `roles: Vec<String>` in JWT claims (no role-based endpoint guards beyond `has_role()` method that is not called in any route handler)
- No SSO, no SCIM
- No schema/database isolation or RLS
- No tenant concept (workspace = user account, no org/team layer)

A comment in the hook handler notes "ready for multi-tenant SaaS" (`routes.rs:643`) but the implementation is basic user-per-flow ownership, not full multi-tenancy.

**Comparison with Nebula:** Nebula has `nebula-tenant` with three isolation modes (schema/RLS/database), RBAC, planned SSO/SCIM. z8run has user accounts + per-flow ownership only.

---

## 13. Observability [A15]

### Tracing

`tracing` crate (`Cargo.toml:73: tracing = "0.1"`) with `tracing-subscriber` using `env-filter` and `json` features. Logging configured via `Z8_LOG_LEVEL` env var. `#[instrument]` macro used on `FlowEngine::execute` and `execute_with_trigger` (`engine.rs:192, 199`).

**No OpenTelemetry.** Searched for `opentelemetry`, `tracing_opentelemetry`, `jaeger`, `zipkin`, `prometheus`, `metrics` — zero results. Spans are local only; no distributed trace export.

### Metrics

No metrics framework. No `prometheus`, no `opentelemetry-metrics`, no counters/histograms per node.

### Execution events

The `EngineEvent` enum (`engine.rs:19-68`) provides: `FlowStarted`, `NodeStarted`, `NodeCompleted` (with `duration_us`), `NodeSkipped`, `NodeError`, `MessageSent` (with payload preview), `StreamChunk` (LLM token), `FlowCompleted` (with `duration_ms`), `FlowError`. These are broadcast over a `tokio::sync::broadcast` channel and forwarded to WebSocket clients for real-time UI display (`z8run-api/src/ws.rs`).

**Comparison with Nebula:** Nebula uses OpenTelemetry with per-execution trace spans and per-action metrics. z8run uses local structured logging + WebSocket broadcast events. No distributed tracing, no metrics export.

---

## 14. API Surface [A16]

### REST API

Axum 0.8. Routes mounted at `/api/v1/`:
- `GET/POST /flows` — list, create
- `GET/PUT/DELETE /flows/{id}` — get, update, delete
- `POST /flows/{id}/start` — start execution
- `POST /flows/{id}/stop` — stop execution
- `GET /flows/{id}/export` — export as JSON
- `POST /flows/import` — import from JSON
- `GET/POST /vault` — list keys, store credential
- `GET/DELETE /vault/{key}` — get, delete credential
- `GET/POST /auth/register`, `POST /auth/login`, `GET /auth/me`, `POST /auth/refresh`
- `GET /health`, `GET /info` (public)
- `ANY /hook/{flow_id}`, `ANY /hook/{flow_id}/{*path}` (webhook triggers)

No OpenAPI spec generated. No GraphQL, no gRPC. No versioning beyond the `v1` path prefix.

### WebSocket

Binary protocol at `/ws/engine`. 11-byte frame header: `[version:1 | msg_type:2 | correlation_id:4 | payload_len:4]` with bincode-serialized payload (`crates/z8run-protocol/src/frame.rs`). Protocol version is v1 only, no negotiation.

**Comparison with Nebula:** Nebula has REST now with GraphQL + gRPC planned and OpenAPI spec. z8run has REST + binary WebSocket but no spec, no GraphQL/gRPC, no versioning mechanism.

---

## 15. Testing Infrastructure [A19]

**No dedicated testing crate.** All tests are `#[cfg(test)]` modules inline in source files. Test count: 270 `#[test]` / `#[tokio::test]` annotations across 79 `.rs` files — a reasonable unit test density for 22K LOC.

Test coverage areas: `PortType::is_compatible_with`, `Flow::validate_acyclic`, `Flow::topological_order`, `ExecutionPlan::compile`, `VaultCrypto` roundtrip, `FrameHeader` roundtrip, HMAC-SHA256 vectors, `WebhookNode` accept/filter scenarios.

**No integration tests** (explicitly listed as roadmap item `[ ] Integration tests`). No contract tests, no wiremock, no mockall. No public testing utilities for plugin/node authors.

**Comparison with Nebula:** Nebula has `nebula-testing` as a public crate with resource-author contract tests. z8run has only inline unit tests.

---

## 16. AI / LLM Integration [A21]

### A21.1 — Existence

**First-class, built-in.** 10 AI nodes ship in `z8run-core`:
`llm`, `embeddings`, `classifier`, `prompt_template`, `text_splitter`, `vector_store`, `structured_output`, `summarizer`, `ai_agent`, `image_gen`. This is the most mature AI integration of any Rust workflow engine surveyed.

### A21.2 — Provider abstraction

**Multi-provider, but thin.** The `LlmNode` supports `provider` field: `"openai"` (default), `"anthropic"`, `"ollama"` (local). Selection is via `match self.provider.as_str()` branching in `process()` (`crates/z8run-core/src/nodes/llm.rs:74-125`). No provider trait, no `dyn Provider` abstraction — each branch has separate call/stream implementations. BYOL endpoint supported via `base_url` config field (for Ollama: `http://localhost:11434`, for OpenAI-compat proxies). Local model support via Ollama only (no candle, no mistral.rs, no llama.cpp bindings).

The shared `call_llm` utility (`crates/z8run-core/src/utils/llm_client.rs`) provides a centralized non-streaming call path used by `StructuredOutputNode`, `ClassifierNode`, and `SummarizerNode`.

### A21.3 — Prompt management

`PromptTemplateNode` (`nodes/prompt_template.rs`): `{{variable}}` substitution from message payload. No versioning, no few-shot management, no system/user/assistant template structure (system prompt is a separate `system_prompt` config field on `LlmNode`).

### A21.4 — Structured output

`StructuredOutputNode` (`nodes/structured_output.rs`): sends JSON schema as part of the system prompt instruction ("respond ONLY with a valid JSON object that matches this schema"). Retries on parse failure (configurable `retries` field). Uses `serde_json::from_str` for validation. No JSON Schema library validation — only JSON parse success. No function/tool calling for structured output.

### A21.5 — Tool calling

**`AiAgentNode` implements multi-turn tool calling** (`nodes/ai_agent.rs`). The agent node supports `tools: Vec<ToolDefinition>` (name, description, JSON Schema parameters). Flow:
1. First call → LLM may return text or `tool_call`
2. If `tool_call`: emits on `"tool_call"` port with `conversation_history` in payload
3. A subsequent node processes the tool call and returns result
4. The result feeds back to `AiAgentNode` via `conversation_history` + `tool_result` in payload
5. Loop continues until text response or `max_iterations` reached

This is implemented via flow re-entry: the `AiAgentNode` detects continuation by checking `msg.payload.get("conversation_history")` presence (`ai_agent.rs:72`). This is a clever but fragile pattern — it relies on the flow graph having a cycle-like feedback path, which conflicts with the DAG constraint. In practice, it works because the continuation message is injected as a new execution, not a graph cycle.

Providers supported for tool calling: OpenAI (function calling API), Anthropic (tool use API), Ollama (tool use). Parallel tool execution: not implemented (single tool call per turn). Multi-agent coordination via `HumanHandoffNode` for escalation patterns.

### A21.6 — Streaming

**Implemented** for OpenAI, Anthropic, and Ollama. Streaming mode is activated when `event_tx` is set (i.e., when the engine has a WebSocket subscriber). Tokens emitted via `EngineEvent::StreamChunk { flow_id, node_id, chunk, done }` broadcast channel (`nodes/llm.rs:409-416`). The final `done: true` chunk signals completion. No backpressure on the stream — tokens are fire-and-forget on the broadcast channel.

### A21.7 — Multi-agent

`AiAgentNode` supports tool calling loops with `max_iterations` (`ai_agent.rs:42`). `HumanHandoffNode` (`nodes/human_handoff.rs`) provides escalation from AI to human with ticket tracking (in-memory, not persisted). No agent-to-agent delegation, no shared memory across agent nodes (beyond `ConversationMemoryNode`).

### A21.8 — RAG/vector

`EmbeddingsNode` (OpenAI `text-embedding-3-small`, Ollama `nomic-embed-text`) generates vectors. `VectorStoreNode` implements **in-memory** cosine similarity search with a `OnceLock<Arc<RwLock<HashMap<String, Vec<VectorEntry>>>>>` global store (`nodes/vector_store.rs:38-43`). Actions: store, search (cosine similarity, top-k, min-score threshold), delete, clear. **No external vector store integration** (no Qdrant, no Pinecone, no pgvector, no Weaviate). The in-memory store is lost on restart.

### A21.9 — Memory/context

`ConversationMemoryNode` (`nodes/conversation_memory.rs`) provides in-memory per-conversation history (save/load/clear/list). TTL-based expiry. Max message count per conversation. **Not persisted to DB** — lost on restart.

### A21.10 — Cost/tokens

**Not implemented.** Searched for `cost`, `token_count`, `tokens_used`, `prompt_tokens`, `usage`, `billing` — zero results. The LLM API responses that include `usage` in the JSON body are not parsed.

### A21.11 — Observability

Per-LLM-call tracing via `tracing::info!` macros (`nodes/llm.rs:63-66`). No per-call span, no prompt/response logging (PII concern addressed by design omission, not explicit filtering), no eval hooks.

### A21.12 — Safety

`SanitizeNode` (`nodes/sanitize.rs`) exists for input sanitization. No content filtering pre/post LLM calls, no prompt injection detection, no output validation beyond `StructuredOutputNode`'s JSON parse check.

### A21.13 — vs Nebula+Surge

Nebula: no first-class LLM (bet: AI = generic actions + plugin LLM client). Surge = agent orchestrator on ACP.

z8run: **10 built-in AI nodes, working multi-turn tool calling, streaming, multi-provider (OpenAI/Anthropic/Ollama), embeddings, in-memory vector store, conversation memory**. This is substantially more AI capability than any comparable Rust workflow engine.

The trade-off: z8run's AI nodes are deeply coupled to provider-specific APIs (no provider trait abstraction), in-memory only for vector/conversation store, no cost tracking, no PII-safe logging, no evaluation framework. The implementation is solid for rapid prototyping but not production-grade AI pipeline infrastructure.

---

## 17. Notable Design Decisions

### 1. Type erasure at node boundary (deliberate simplicity)
All nodes receive `FlowMessage` with `serde_json::Value` payload. No associated Input/Output types. This makes authoring easy (30-line hello-world node) but eliminates compile-time safety. The `PortType` enum exists but its `Any` escape hatch and frontend-default usage make it a documentation hint rather than a constraint. **Trade-off:** Faster iteration, lower barrier to third-party nodes. Nebula's sealed traits prevent certain bugs at compile time that z8run will encounter at runtime.

### 2. Credential vault in storage layer (architectural coupling)
Credential encryption (`AES-256-GCM`) lives inside `z8run-storage`, the same crate as flow persistence. This creates a coupling: changing the vault backend requires changing the storage layer. The vault also lacks user-scoping, creating a security risk in multi-user deployments (all users share a global credential namespace). Nebula separates `nebula-credential` as an independent crate.

### 3. Canvas-as-truth: frontend state stored in flow metadata
The React Flow canvas state (node positions, types, config) is stored as `metadata.positions.canvas_nodes` — a JSON blob — in the database. The core `Flow.nodes` and `Flow.edges` are populated only at execution time via `canvas_to_flow()` (`routes.rs:442-543`). This makes the canonical representation the visual canvas, not the core domain model. Benefits: visual state is always preserved. Risks: the core model is a derived transformation from the canvas, not the source of truth.

### 4. WASM ABI is raw pointer/length (not WIT)
Rather than using WIT (WebAssembly Interface Types) and wit-bindgen, z8run defines a custom ABI: `z8_alloc`, `z8_process`, `z8_configure`, `z8_validate`, `z8_node_type` with raw linear memory access. This is simpler to implement for the host but harder for plugin authors, requires manual memory management, and does not compose with the WASI component model.

### 5. AI as first-class built-in vs plugin
z8run ships 10 AI nodes in `z8run-core` — the core library — not as optional plugins. This makes AI-heavy workflows simple to deploy (single binary, no plugin install) but bloats the core with LLM API dependencies (`reqwest`, OpenAI/Anthropic/Ollama HTTP contracts) that are unavoidable even for non-AI deployments. Nebula's bet (AI via generic actions + plugin) keeps the core lean.

---

## 18. Known Limitations / Pain Points

Based on code analysis and CHANGELOG:

1. **No checkpoint/recovery** — All execution state is in-memory; a crash loses running flows. Execution records left as "running" in DB permanently.
2. **Cron trigger fires on call, not on schedule** — `CronTriggerNode.process()` returns a payload when invoked but there is no scheduler that fires it periodically. Cron-scheduled flows require external triggering.
3. **reqwest::Client created per process call** — LLM, HTTP request, embedding, and other network nodes create a new HTTP client on every `process()` call. This means no connection pooling, no TLS session reuse.
4. **Global credential vault (no user scoping)** — `credentials` table has no `user_id` column. Any user can read/write any credential in multi-user deployments.
5. **WASI capabilities not enforced** — The manifest declares `network`, `filesystem`, `env_vars` capabilities but the wasmtime linker has no WASI imports. A WASM plugin requesting `network: false` has the same access as one requesting `network: true` — zero.
6. **In-memory vector store lost on restart** — `VectorStoreNode` and `ConversationMemoryNode` use `OnceLock`-backed in-memory storage with no persistence.
7. **No integration tests** — roadmap item `[ ] Integration tests`.
8. **MySQL storage adapter stub** — `sqlx` includes `mysql` feature but no `mysql.rs` source file exists.

---

## 19. Bus Factor / Sustainability

- **Maintainers:** Apparent solo maintainer (hello@z8run.org, `z8run` GitHub org with no other public members)
- **Commit cadence:** Burst pattern — 14 commits in initial month (March 2026), cleanup/hardening in April 2026
- **Stars:** 5 | **Forks:** 2 — pre-traction
- **Issues:** 0 real issues (only Dependabot PRs); no community engagement
- **Crates.io downloads:** z8run-core 86, z8run-cli 31 — negligible
- **Bus factor: 1** — single maintainer, no contributors
- **Last release:** v0.2.0 (2026-04-01) — 25 days before research date, active

The project is very early stage. CI/CD is configured (GitHub Actions with build, test, deploy, release workflows), Docker images on GHCR, domain + live demo at app.z8run.org. Technical foundation is solid. Community adoption has not begun.

---

## 20. Final Scorecard vs Nebula

| Axis | z8run approach | Nebula approach | Verdict | Borrow? |
|------|---------------|-----------------|---------|---------|
| A1 Workspace | 7 crates (5 lib + 2 bin); no strict layering; z8run-core includes infra (reqwest, sqlx) | 26 crates, layered: nebula-error / nebula-resilience / nebula-credential / ... Edition 2024 | Nebula deeper (domain isolation) | no |
| A2 DAG | `Vec<Node>` + `Vec<Edge>`; PortType enum (7 values + Any escape); Kahn's topological leveling; no petgraph | TypeDAG: L1 static generics; L2 TypeId; L3 refinement predicates; L4 petgraph | Nebula deeper (compile-time safety) | no |
| A3 Action | Single `NodeExecutor` trait (open, no assoc types); `serde_json::Value` I/O; one factory per type; `node_factory!` + `configure_fields!` macros | 5 action kinds, sealed, assoc Input/Output/Error, versioning, derive macros | Nebula deeper (type safety); z8run simpler (lower barrier) | no — different goals |
| A4 Credential | AES-256-GCM vault (store/retrieve/delete/list); `vault:key` resolution at flow-start; no lifecycle, no OAuth2, no user scoping | State/Material split, LiveCredential watch(), blue-green refresh, OAuth2Protocol adapter | Nebula deeper (lifecycle, type safety) | refine — borrow z8run's WASM token isolation concept |
| A5 Resource | No resource abstraction; `reqwest::Client::new()` per call; no pooling, no scope | 4 scope levels, ReloadOutcome, generation tracking, on_credential_refresh | Nebula deeper; z8run has a notable perf bug | no |
| A6 Resilience | None (no crate, no retry, no CB); per-node `timeout_ms`; webhook 10s global timeout | nebula-resilience: retry/CB/bulkhead/timeout/hedging; ErrorClassifier | Nebula deeper | no |
| A7 Expression | No DSL; dot-notation in switch/filter (internal only); PromptTemplate `{{var}}`; no eval engine | 60+ funcs, type inference, sandboxed eval, `$nodes.foo.result.email` | Nebula deeper | no |
| A8 Storage | sqlx 0.8 + sqlite + postgres; flow as JSON blob in single table; 2-migration schema | sqlx + PgPool, Pg*Repo per aggregate, SQL migrations, PostgreSQL RLS | Different decomposition; z8run simpler schema | no |
| A9 Persistence | No checkpoint; execution log stub (`node_logs = '{}'`); in-memory state lost on crash | Frontier + checkpoint + append-only log; replay recovery | Nebula deeper | no |
| A10 Concurrency | tokio; breadth-first topological leveling; one task per node; mpsc channels (buf=256); no !Send support | tokio, frontier scheduler work-stealing, !Send isolation via thread-local | Nebula deeper (!Send, work-stealing) | maybe — breadth-first leveling is simpler to reason about |
| A11 Plugin BUILD | `.wasm` + `manifest.toml`; no SDK; capability declarations (unenforced); local dir only | WASM, plugin-v2 spec, Plugin Fund | Different decomposition; z8run working, Nebula designed | refine — borrow z8run's TOML manifest format |
| A11 Plugin EXEC | wasmtime v42; raw ptr ABI (z8_alloc/process/configure); no WIT; WASI stubs (declared, not enforced); fresh instance per call | WASM sandbox + WIT/capability security (planned) | z8run working; Nebula more principled design | refine — z8run's working ABI is a concrete data point |
| A12 Trigger | webhook (HMAC + Bearer + Basic), http-in, cron (process-only, no scheduler), timer, MQTT; no Kafka/NATS/Redis; 1:1 dispatch; 10s response timeout | TriggerAction Source→Event 2-stage; Kafka/NATS planned | Different decomposition; z8run simpler; cron broken | no |
| A13 Deployment | Single binary (`z8run serve`); SQLite embedded (dev) or PostgreSQL (prod); no multi-mode | 3 modes from one binary: desktop/self-hosted/cloud | Convergent (single binary ✓); z8run lacks desktop mode | no |
| A14 Multi-tenancy | User accounts + JWT; per-flow ownership; no tenant layer; no RLS; global vault (no user scoping) | nebula-tenant: schema/RLS/database isolation; RBAC; planned SSO/SCIM | Nebula deeper | no |
| A15 Observability | `tracing` + structured logs + WebSocket broadcast events (per node/flow); no OTel; no metrics | OpenTelemetry per execution; metrics per action | Nebula deeper (OTel export) | no |
| A16 API | REST (Axum 0.8) + binary WebSocket; no OpenAPI spec; no GraphQL/gRPC; `/api/v1` prefix only | REST + planned GraphQL/gRPC; OpenAPI spec generated | Different decomposition; z8run's binary WS protocol is notable | refine — binary WS protocol for editor sync is good DX |
| A17 Type safety | Open trait, no GATs, no HRTBs, no typestate; PortType::Any escape | Sealed traits, GATs, HRTBs, typestate, Validated<T> | Nebula deeper | no |
| A18 Errors | `thiserror` Z8Error enum; no ErrorClass; StorageError in storage crate | nebula-error + ErrorClass enum; contextual errors | Nebula richer; z8run simpler | no |
| A19 Testing | 270 inline unit tests; no integration tests; no testing crate; no contracts | nebula-testing crate; contract tests; insta + wiremock + mockall | Nebula deeper | no |
| A20 Governance | Apache-2.0/MIT dual; solo maintainer; GitHub Sponsors; no commercial model | Open core; Plugin Fund (commercial model for plugin authors); planned SOC 2 | Different goals; z8run simpler licensing | no |
| A21 AI/LLM | 10 built-in AI nodes: LLM (OpenAI/Anthropic/Ollama/streaming), embeddings, vector store (in-memory), AI agent (multi-turn tool calling), structured output, conversation memory, image gen; no cost tracking; no PII-safe logging | No first-class LLM; generic actions + plugin LLM (future Surge) | **Competitor deeper** (working AI suite today) | **yes — study z8run's AI node structure as reference for Nebula's LLM plugin** |

---

## Summary

z8run is a credible early-stage competitor in the Rust visual workflow engine space. Its core strengths are the **working WASM plugin sandbox** (wasmtime v42, functional ABI), the **10-node AI suite** (first-class LLM/embeddings/agent/vector-store), the **AES-256-GCM credential vault**, and the **visual editor + binary WebSocket protocol**. It ships as a single binary with SQLite embedded — a compelling DX for self-hosted deployment.

Its significant gaps relative to Nebula are: no checkpoint/recovery (Node-RED-style stateless execution), no resilience layer (no retry/CB/bulkhead), no expression engine, no first-class resource management (reqwest client per call is a production bug), incomplete WASM capability enforcement (declarations stored but not enforced), and broken cron trigger (no scheduler fires it).

The most valuable Nebula insight from z8run is **axis A21**: z8run proves that AI nodes can be first-class built-ins in a Rust workflow engine and that multi-turn LLM tool-calling can be implemented via flow re-entry. For Nebula's AI strategy (currently "generic actions + Surge"), z8run's working implementation is a concrete data point worth studying.
