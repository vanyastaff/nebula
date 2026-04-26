# emergent-engine — Architectural Decomposition

## 0. Project Metadata

- **Repo**: https://github.com/govcraft/emergent
- **Stars**: 18 | **Forks**: 0 | **Open issues**: 13 | **Closed issues**: ~24
- **Created**: 2026-01-06 | **Last commit**: 2026-04-26
- **License**: MIT OR Apache-2.0
- **Author/Maintainer**: Roland Rodriguez (roland@govcraft.ai, govcraft.ai)
- **Latest releases**: engine v0.10.9, SDKs v0.13.1
- **Governance**: Solo maintainer. No commercial model, no Plugin Fund equivalent. No stated SOC 2 path.
- **Toolchain**: Rust 2024 edition (not pinned to a specific stable version; `cargo install emergent-engine` recommended for end users)

---

## 1. Concept Positioning [A1, A13, A20]

**Author's own description** (README first line):
> "Compose AI-powered automations from CLI tools — no framework required. Any command-line tool — an LLM, a classical ML model, a curl call to an API, a jq transformation — becomes a composable building block."

**My description after reading code**:
Emergent is a single-binary event-bus orchestrator that manages child processes (primitives) communicating over Unix domain sockets using a pub-sub fabric, with TOML-declared topology and dual-backend event storage (JSON log + SQLite). It is explicitly NOT a DAG system — it is a pub-sub graph with cycle support.

**Comparison with Nebula**:
Emergent and Nebula target the same broad space (compose steps into pipelines) but from opposite ends. Nebula starts from the type system — a Rust trait hierarchy with GATs, sealed enums, and compile-time guarantees. Emergent starts from the process model — any executable is a step, the engine is a dumb message bus, and your code (or shell commands) live outside the engine entirely. Nebula is an in-process library; Emergent is a multi-process runtime. Nebula stores workflow state in PostgreSQL with frontier-based recovery; Emergent appends to SQLite + JSON logs with no checkpoint/recovery semantics. Nebula has 5 sealed action kinds; Emergent has 3 open process kinds with no trait constraints at all.

---

## 2. Workspace Structure [A1]

The Rust workspace (`Cargo.toml` root, line 3) declares 7 members:

```
emergent-engine/          ← core engine binary (emergent-engine crate)
sdks/rust/                ← Rust SDK (emergent-client crate)
examples/sources/timer/   ← timer source example
examples/handlers/filter/ ← filter handler example
examples/handlers/exec/   ← exec-handler (stdin/stdout pipe wrapper)
examples/sinks/log/       ← log sink example
examples/sinks/console/   ← console sink example
```

Additionally, three non-Rust SDKs live in the repo (outside the Cargo workspace):
- `sdks/ts/` — TypeScript/Deno SDK (`@govcraft/emergent` on JSR)
- `sdks/py/` — Python 3.11+ SDK (PyPI: `emergent`)
- `sdks/go/` — Go 1.23+ module (`github.com/govcraft/emergent/sdks/go`)

**Feature flags**: None declared. The single release binary includes all features; no conditional compilation for sub-features exists.

**Umbrella crate**: None. `emergent-engine` is the binary; `emergent-client` (sdks/rust) is the library for SDK users.

**Layer separation**: Two layers only — engine (broker/manager) and client (SDK). No separate resilience, credential, or tenant crates.

**Comparison with Nebula**: Nebula has 26 crates across 6+ conceptual layers (error, resilience, credential, resource, action, engine, tenant, eventbus, etc.). Emergent has 2 crates (7 workspace members total counting examples). The difference reflects fundamentally different design philosophies: Nebula encapsulates all concerns in Rust; Emergent externalizes most concerns to the subprocess.

---

## 3. Core Abstractions [A3, A17]

### A3.1 — Trait shape

Emergent has **no Rust trait for the unit-of-work abstraction**. There is no `Action` trait, no `Node` trait, no sealed enum of action kinds. Instead, the three primitive types (`Source`, `Handler`, `Sink`) are:

1. An enum `PrimitiveKind` in `emergent-engine/src/primitives.rs:9-19`:
   ```rust
   pub enum PrimitiveKind {
       Source,    // publish-only
       Handler,   // subscribe + publish
       Sink,      // subscribe-only
   }
   ```
   This is a data tag, not a trait constraint.

2. A `PrimitiveConfig` trait in `emergent-engine/src/config.rs:309-318`:
   ```rust
   pub trait PrimitiveConfig {
       fn name(&self) -> &str;
       fn path(&self) -> &Path;
       fn is_enabled(&self) -> bool;
   }
   ```
   This is a purely internal trait used for generic config validation — not exposed to users.

3. Three plain structs: `SourceConfig`, `HandlerConfig`, `SinkConfig` (all in `config.rs:133-228`). Handlers and sinks have `subscribes` and `publishes` fields; sinks lack `publishes`. There are no associated types, no GATs, no HRTBs.

**In-engine handling**: Each primitive is wrapped in a `PrimitiveActor` (actor pattern from `acton-reactive`), which spawns the child process. The engine does not call any Rust trait on the primitive — it forks a new OS process and communicates via Unix socket.

### A3.2 — I/O shape

The universal message envelope is `EmergentMessage` defined in `sdks/rust/src/message.rs`:

```rust
pub struct EmergentMessage {
    pub id: MessageId,                    // TypeID: msg_<UUIDv7>
    pub message_type: MessageType,        // String-typed, e.g. "timer.tick"
    pub source: PrimitiveName,            // emitting primitive name
    pub correlation_id: Option<CorrelationId>,
    pub causation_id: Option<CausationId>,  // parent→child chain
    pub timestamp_ms: Timestamp,
    pub payload: serde_json::Value,        // FULLY DYNAMIC, any JSON
    pub metadata: Option<serde_json::Value>,
}
```

**Key observation**: `payload` is `serde_json::Value` — completely runtime-typed, no compile-time schema enforcement. This is the opposite of Nebula's approach where `Input` and `Output` are associated types on the action trait.

Input is never serializable-required at the trait level because there is no trait. Output is always `serde_json::Value`. Side effects are contained within each subprocess — the engine has no model of what a primitive does internally.

### A3.3 — Versioning

None. There is no versioning system for primitive types, message schemas, or workflow definitions. The `message_type` field is a plain string (e.g., `"timer.tick"`) with no version component. No `#[deprecated]`, no migration support, no v1/v2 distinction. If a message schema changes, all consuming primitives must be updated manually.

### A3.4 — Lifecycle hooks

The engine side (`primitive_actor.rs`) has:
- `after_start` (from acton-reactive actor lifecycle): spawns the child process, broadcasts `system.started.<name>`
- `before_stop`: sends SIGTERM to child, waits, broadcasts `system.stopped.<name>`

The **primitive side** (user code / SDK) has:
- Connection via `EmergentSource::connect()`, `EmergentHandler::connect()`, `EmergentSink::connect()`
- Subscription via `handler.subscribe(types)` returning `MessageStream`
- Message processing loop: `while let Some(msg) = stream.next().await`
- Graceful shutdown: SDK intercepts `system.shutdown` from the engine and closes the stream automatically

No pre/execute/post pattern. No idempotency key. No cancellation points inside the processing loop — cancellation happens only at the stream level (stream closes on `system.shutdown`).

### A3.5 — Resource and credential dependencies

None. A primitive declares no resource dependencies in TOML or in any Rust type. If a handler needs a DB pool, it manages it internally. Credentials are passed via environment variables in the TOML `env` field (e.g., `env = {SLACK_TOKEN = "$SLACK_TOKEN"}`). There is no compile-time check.

### A3.6 — Retry / resilience attachment

No per-primitive retry policy. No circuit breaker. No bulkhead. The `exec-handler` primitive (examples/handlers/exec/src/main.rs) has a per-invocation `--timeout` flag (line 61, milliseconds), but this is a feature of the user's primitive, not the engine. If a handler crashes, the engine logs `system.error.<name>` but does not restart it automatically. Issue #25 (OS-level sandboxing) and #33 (fast-interval sources silently dying) confirm the absence of resilience at the engine layer.

### A3.7 — Authoring DX

Three paths:
1. **Zero code**: Use marketplace exec primitives (`emergent marketplace install exec-handler`). Wire in TOML. The entire pipeline is TOML + shell commands.
2. **Scaffold + SDK**: `emergent scaffold -t handler -n my_filter -l rust` generates a minimal Rust project with correct boilerplate. "Hello world" handler in Rust is ~20 lines (connect, subscribe, loop, publish).
3. **Manual SDK**: Implement the pattern directly. No derive macros required.

The scaffold uses `minijinja` templates (`emergent-engine/templates/*/`) to generate language-specific boilerplate for Rust, TypeScript, Python, and Go.

### A3.8 — Metadata

No built-in mechanism for display name, description, icon, or category on a primitive. Metadata lives in marketplace manifests (`emergent-registry/primitives/<name>/manifest.toml`), not in the primitive binary itself.

### A3.9 — Comparison with Nebula

| Dimension | Nebula | emergent |
|-----------|--------|----------|
| Action kinds | 5 sealed (Process/Supply/Trigger/Event/Schedule) | 3 open (Source/Handler/Sink) |
| Type system | Sealed traits, assoc types Input/Output/Error, GATs | No trait; PrimitiveKind enum + PrimitiveConfig config-only trait |
| I/O schema | Associated generic types, type-safe at compile time | `serde_json::Value`, fully runtime |
| Versioning | Type identity, derive macros | None |
| Lifecycle hooks | pre/execute/post/on-failure | after_start/before_stop (engine-side); stream open/close (SDK-side) |
| Resource deps | Declared, compile-time checked | None; env vars only |
| Authoring | Derive macro + builder | Scaffold + connect/subscribe/loop/publish |

---

## 4. DAG / Execution Graph [A2, A9, A10]

### Graph model

Emergent explicitly rejects the DAG model. From README:
> "Most workflow tools are DAG-based: directed acyclic graphs where data flows one way through a fixed sequence. Emergent uses pub-sub routing instead, which removes structural constraints that DAGs impose."

The topology is a **directed multigraph with cycles** (pub-sub graph): nodes are primitives, edges are subscription relationships (primitive A subscribes to message type X, source B publishes X → implied edge B→A). The graph is not described as a type at compile time; it is reconstructed at startup from TOML declarations and validated only for duplicate names and executable path existence.

There is no petgraph, no topological sort, no compile-time port typing. The routing table is built dynamically in `acton-reactive`'s IPC broker as primitives connect and subscribe.

### Compile-time checks

None on the topology. TOML validation only checks: duplicate names, path existence for enabled primitives (`config.rs:499-527`). The broker does not enforce that a primitive's declared `publishes` list matches what it actually publishes at runtime (this is tracked in issue #23).

### Concurrency model

- **Engine**: tokio runtime (full features), acton-reactive actor system. Each primitive has its own `PrimitiveActor` (an acton actor running inside the engine process).
- **Primitives**: each runs as a separate OS process. Concurrency inside a primitive is the primitive author's responsibility.
- **Message routing**: acton-reactive's IPC broker routes messages synchronously within the engine's tokio runtime. Fan-out to multiple subscribers happens via push notifications.
- **Backpressure**: SDK has `publish_ack()` (request-response to broker) for acknowledged delivery, and `publish_stream()`/`stream_offer()` for pull-based consumer-driven backpressure (`sdks/rust/src/connection.rs:916-973`). Fire-and-forget `publish()` has no backpressure. The channel buffer in the push bridge is 256 messages (`connection.rs:284`).
- **!Send**: Not relevant — primitives are separate processes, not in-process tasks.

---

## 5. Persistence and Recovery [A8, A9]

### Storage backends

Two backends, both local-file:

**1. JSON append-only log** (`emergent-engine/src/event_store/json_log.rs`): one newline-delimited JSON file per engine run (`~/.local/share/emergent/<name>/logs/`). Human-readable, append-only.

**2. SQLite database** (`emergent-engine/src/event_store/sqlite.rs:17-55`): single `rusqlite` connection (bundled SQLite) protected by a `Mutex<Connection>`. Schema: one `events` table with columns `id, message_type, source, correlation_id, causation_id, timestamp_ms, payload_json, metadata_json`. Indices on `message_type`, `timestamp_ms`, `source`, `correlation_id`.

**No PostgreSQL, no RLS, no sqlx, no distributed storage.** Nebula uses `sqlx + PgPool + RLS`; Emergent uses embedded SQLite.

### Persistence model

Events are logged for **observability and replay**, not for workflow recovery. There is no checkpoint/recovery mechanism. If the engine crashes mid-pipeline, it restarts from scratch — all in-flight messages are lost. The event store allows post-hoc replay queries by type or time range, but the engine does not use this to resume interrupted pipelines.

**Retention**: configurable in days (`event_store.retention_days`, default 30). `delete_before(timestamp_ms)` is implemented (`sqlite.rs:194`).

**Comparison with Nebula**: Nebula has frontier-based checkpoint recovery (state reconstruction via replay, append-only log). Emergent has append-only log but no frontier/checkpoint — recovery is restart-from-zero. This is a fundamental difference in design goals: Nebula targets long-running workflows that must survive crashes; Emergent targets pipelines where a crash means restart the whole pipeline.

---

## 6. Credentials / Secrets [A4]

**A4.1 — Existence**: No separate credential layer exists.

Searched the entire Rust codebase for: `credential`, `secret`, `token`, `oauth`, `auth`, `vault`, `keychain`, `zeroize`, `secrecy`. Results:
- `credential` — 0 matches
- `secret` — 0 matches  
- `zeroize` — 0 matches
- `secrecy` — 0 matches
- `token` — 1 match in a test fixture (`registry.rs:298`) using the string `"token"` as an example CLI argument name in a Slack primitive manifest test; not a credential system

The mechanism for secrets is plain environment variables passed via the TOML `env` field:
```toml
[[handlers]]
name = "claude-respond"
env = {ANTHROPIC_API_KEY = "$ANTHROPIC_API_KEY"}
```
The engine passes these to child processes via `std::process::Command::env()` (`process_manager.rs:109-111`). No encryption, no at-rest protection, no in-memory protection.

**A4.2 — Storage**: None. No at-rest encryption, no backend (vault/keychain/file).

**A4.3 — In-memory protection**: None. No `Zeroize`, no `secrecy::Secret<T>`.

**A4.4 — Lifecycle**: No CRUD for credentials, no refresh model, no expiry, no revocation.

**A4.5 — OAuth2/OIDC**: None. Not mentioned in any source file or doc.

**A4.6 — Composition**: N/A.

**A4.7 — Scope**: Environment variables are process-scoped (TOML `env` table → `std::process::Command::env()`).

**A4.8 — Type safety**: None. Credentials are plain strings.

**A4.9 — vs Nebula**: Nebula has State/Material split, LiveCredential with watch(), blue-green refresh, OAuth2Protocol blanket adapter. Emergent has none of these. This is explicitly by design: "Emergent does not know or care whether a handler runs a 400B parameter LLM or a hand-tuned regex." The tool is responsible for its own auth.

---

## 7. Resource Management [A5]

**A5.1 — Existence**: No separate resource abstraction exists.

Searched for: `resource`, `pool`, `connection_pool`, `ReloadOutcome`, `ResourceScope`, `on_credential_refresh`. All returned 0 results in Rust source files.

Each primitive manages its own resources internally. A Python handler that needs a DB connection pool creates it in its own process. A Rust handler that needs an HTTP client creates it in `main()` before the message loop. The engine has no visibility into, or control over, any resource a primitive holds.

**A5.2 — Scoping**: No scope levels. Resource lifetime = primitive process lifetime.

**A5.3 — Lifecycle hooks**: No init/shutdown/health-check at the resource level. The primitive process starting/stopping is the only hook.

**A5.4 — Reload**: No hot-reload. No blue-green. No generation counter. Reloading a resource requires restarting the primitive process.

**A5.5 — Sharing**: No sharing across primitives. Each process owns its resources exclusively.

**A5.6 — Credential deps**: No mechanism for a resource to declare or react to credential rotation.

**A5.7 — Backpressure**: No acquire timeout or bounded queue at the resource level. The only backpressure is at the IPC message layer (publish_ack / pull-based streams).

**A5.8 — vs Nebula**: Nebula has 4 scope levels (Global/Workflow/Execution/Action), `ReloadOutcome` enum, generation tracking, `on_credential_refresh` per-resource hook. Emergent has none of these. The design philosophy is different: Nebula manages resources; Emergent delegates resource management entirely to the subprocess.

---

## 8. Resilience [A6, A18]

### Retry / Circuit Breaker / Bulkhead

No retry, circuit breaker, bulkhead, or hedging at the engine level. Searched for `retry`, `circuit_breaker`, `backoff`, `bulkhead`, `fallback` — found only:
- A timeout in the exec-handler example (`examples/handlers/exec/src/main.rs:61,129`): per-invocation `--timeout` flag for the child command, in milliseconds. This is user-land code, not an engine feature.
- Tokio timeouts in SDK subscription handshake (`sdks/rust/src/connection.rs:336,387`): 30-second timeouts on topology/subscription queries.

If a primitive crashes, the engine logs `system.error.<name>` but takes no automatic remediation action. Issue #33 (exec-sources silently dying after 2 minutes) confirms this gap — the fix was in heartbeat handling, not in a retry policy.

### Error model

`thiserror = "2"` is used throughout. Key error types:
- `ConfigError` (`config.rs:13-26`): config parse/validation errors
- `ProcessManagerError` (`process_manager.rs:29-40`): lifecycle errors
- `EventStoreError` (`event_store/mod.rs:17-32`): storage errors
- `ClientError` (`sdks/rust/src/error.rs`): connection/publish/subscribe errors
- `MarketplaceError` (`marketplace/error.rs`): install/registry errors

No unified `ErrorClass` enum (Nebula's transient/permanent/cancelled taxonomy). Each module has its own error type with no cross-module classification.

**Comparison with Nebula**: Nebula has `nebula-error` crate with `ErrorClass` used by `ErrorClassifier` in resilience to distinguish transient vs permanent. Emergent uses standard `thiserror` per module with no classification layer.

---

## 9. Expression / Data Routing [A7]

**No expression language or DSL exists.** Searched for `expression`, `eval`, `formula`, `jsonpath`, `jmespath`, `$nodes` — 0 results.

Data transformation is handled entirely within primitives. The engine is a dumb routing bus: it matches `message_type` strings (exact or wildcard `system.started.*`) and forwards messages to all matching subscribers. There is no server-side filtering, projection, or computation on message content.

The closest analog to an expression language is `jq` run as the exec-handler command:
```toml
args = ["--", "jq", ".data | map(select(.score > 0.8))"]
```
But this runs in a child process, not in a DSL embedded in the engine.

**Comparison with Nebula**: Nebula has a 60+ function expression engine with type inference and sandbox. Emergent explicitly delegates this to the subprocess tool.

---

## 10. Plugin / Extension System [A11]

### 10.A — Plugin BUILD process

**A11.1 — Format**: Primitives are **native binaries** (any executable). The "plugin" concept in Emergent is a `marketplace primitive` — a pre-built binary registered in the `emergent-registry` git repository. Manifest format is TOML (`primitives/<name>/manifest.toml`), version is a plain semver string. Schema is ad-hoc (not versioned). Multiple primitives per registry repo (it is a monorepo of manifests).

Example manifest structure (`marketplace/registry.rs:283-311`):
```toml
[primitive]
name = "slack-source"
version = "0.1.0"
kind = "source"

[messages]
publishes = ["slack.message", "slack.reaction"]

[[args]]
name = "token"
long = "token"
env = "SLACK_TOKEN"
required = true

[binaries]
release_url = "https://github.com/govcraft/emergent-primitives/releases"
[binaries.targets]
x86_64-unknown-linux-gnu = "slack-source-0.1.0-x86_64-unknown-linux-gnu.tar.gz"
```

**A11.2 — Toolchain**: Primitives compile in a **separate git repository** (`govcraft/emergent-primitives`). The `emergent scaffold` command generates the scaffolding for a new primitive in the appropriate language. Rust primitives use `emergent-client` from crates.io. No in-tree compilation. Cross-compilation targets: x86_64-linux, aarch64-linux, x86_64-macos, aarch64-macos. Reproducibility: standard GitHub Actions CI.

**A11.3 — Manifest content**: Required fields: `name`, `version`, `kind`, `release_url`, platform targets. No capability declaration, no permission grants, no network/fs policy. The manifest is purely discovery/download metadata.

**A11.4 — Registry/discovery**: `emergent-registry` is a **plain git repository** cloned/pulled by `emergent marketplace` commands. Registry URL is configurable in `~/.local/share/emergent/marketplace/config.toml`. No HTTP API, no signing, no version pinning beyond manifest version field. `emergent marketplace list` fetches `index.toml` from the registry repo.

### 10.B — Plugin EXECUTION sandbox

**A11.5 — Sandbox type**: **OS subprocess** — no sandbox whatsoever. Each primitive is spawned as a child process via `tokio::process::Command` (`primitive_actor.rs`). The process has full OS permissions of the user running the engine.

**A11.6 — Trust boundary**: Primitives are fully trusted. Issue #25 explicitly tracks "OS-level sandboxing for primitive processes" as a future feature — confirming that no sandbox exists today. Issue #24 tracks "authenticate primitive connections by spawned PID" — confirming that any process that knows the socket path can connect and pretend to be a primitive.

**A11.7 — Host↔plugin calls**: IPC over Unix domain sockets. Wire format: MessagePack (default) or JSON. The message envelope is `EmergentMessage` (see §3). No capability-gated API, no prost, no WIT. Async is handled in the SDK's tokio runtime on the client side.

**A11.8 — Lifecycle**: Engine sends SIGTERM on shutdown; monitors child process exit via `tokio::spawn` background task that awaits `child.wait_with_output()`. No crash recovery/restart — if a primitive crashes, it stays crashed (`PrimitiveState::Failed`). No hot reload.

**A11.9 — vs Nebula**: Nebula targets WASM sandbox (wasmtime), capability-based security, Plugin Fund commercial model with royalties. Emergent uses plain OS processes with zero isolation or trust model. This is a deliberate simplicity trade-off: "If it has a CLI, it already works." vs Nebula's "plugins are sandboxed WASM modules with declared capabilities."

---

## 11. Trigger / Event Model [A12]

### A12.1 — Trigger types

**Supported:**
- **Interval/polling source** (`exec-source --interval <ms>`): runs a shell command on a fixed interval. Equivalent to a schedule trigger.
- **One-shot source** (`exec-source --command ... --interval 0`): runs once and exits.
- **HTTP webhook source** (`http-source`, marketplace primitive): receives HTTP requests and emits them as events. The `http-source` is in the separate `emergent-primitives` repo, not in the engine core.
- **WebSocket source** (marketplace `websocket-handler`): bidirectional WebSocket bridge.
- **System event triggers**: any primitive can subscribe to `system.started.<name>`, `system.stopped.<name>`, `system.error.<name>`. The ouroboros-loop example seeds itself from `system.started.webhook`.
- **Custom source** (SDK): any protocol can be implemented in Rust/Python/TypeScript/Go.

**Not supported out of box:**
- Kafka / RabbitMQ / NATS (require custom SDK source)
- Database CDC / LISTEN-NOTIFY (require custom SDK source)
- Filesystem watch (require custom SDK source or `find`/`inotifywait` in exec-source)
- Cron scheduling (exec-source uses interval, not cron expressions)

### A12.2 — Webhook

Webhook support is in the marketplace `http-source` primitive (external repo), not in the engine itself. No stable URL allocation system, no HMAC verification, no idempotency key — these would be implemented in the user's handler. Registration happens at TOML load time (path points to the binary, which starts an HTTP server).

### A12.3 — Schedule

Interval-based only (milliseconds). No cron expression support, no timezone-awareness, no DST handling, no missed-schedule recovery. `exec-source --interval <ms>` is a `tokio::time::interval` loop.

### A12.4 — External events

No direct broker integration. External message queues (Kafka, NATS, Redis Streams) require writing a custom SDK source. The engine is agnostic to message origin.

### A12.5 — Reactive vs polling

Both supported. Reactive: SDK sources that hold open connections (WebSocket, HTTP SSE). Polling: exec-source with interval.

### A12.6 — Trigger→workflow dispatch

Fan-out is the default — one source event reaches all subscribing handlers/sinks. 1:1 (direct dispatch) happens when only one primitive subscribes to a message type. Fan-out is the architectural default. No conditional triggers at the engine level (conditional filtering belongs in a handler). Replay: available via `SqliteEventStore::query_by_type()` but not wired into a user-facing replay command.

### A12.7 — Trigger as Action

No — triggers are **Sources** (a primitive kind), not a special type of Handler. They are separate processes. A Source can run forever (persistent WebSocket), run on an interval (timer), or run once (seed event). Lifecycle: forever or one-shot, determined by the source process itself (exits when done).

### A12.8 — vs Nebula

Nebula has a 2-stage trigger model: `Source` trait normalizes inbound (HTTP request / Kafka message / cron tick) → typed `Event`, then `TriggerAction` with `Input = Config` (registration) and `Output = Event` (typed payload). Emergent has no such 2-stage model — the Source process IS both the ingress normalization and the event emission, indistinguishably. No backpressure model at the trigger level; only at the IPC message level (publish_ack, pull-based streams).

---

## 12. Multi-tenancy [A14]

No multi-tenancy. Searched for `tenant`, `rbac`, `sso`, `scim`, `organization`, `workspace`, `multi.tenant` — 0 results in Rust source. The engine is single-user, single-instance, single-process. No RLS, no schema isolation, no permission model.

**Comparison with Nebula**: Nebula has a `nebula-tenant` crate with three isolation modes (schema/RLS/database), RBAC, and planned SSO/SCIM.

---

## 13. Observability [A15]

**Tracing**: `tracing = "0.1"` with `tracing-subscriber` configured per process. Each engine process and each primitive (via SDK) initializes its own subscriber. Engine logs to stderr by default; primitives log to `~/.local/share/emergent/<name>/primitive.log` by default (configurable via `EMERGENT_LOG=stderr`). No structured trace IDs shared across process boundaries.

**OpenTelemetry**: None. Searched `opentelemetry` — 0 results. No OTel spans, no distributed tracing.

**Metrics**: None built-in. No Prometheus metrics, no counters per message type. The topology HTTP API (`GET /api/topology`, axum, port 8891) provides runtime state but not time-series metrics.

**Event sourcing for debugging**: The `EmergentMessage` envelope includes `causation_id` and `correlation_id` (set by SDK users), enabling causation chain reconstruction via `SqliteEventStore::query_by_correlation()`. This is opt-in.

**Comparison with Nebula**: Nebula uses OpenTelemetry with one trace per workflow execution, per-action latency metrics. Emergent uses basic structured logging (tracing) per process with no cross-process correlation out of the box.

---

## 14. API Surface [A16]

**Programmatic API**: The `emergent-client` crate (Rust SDK) exposes `EmergentSource`, `EmergentHandler`, `EmergentSink` with async methods: `connect()`, `publish()`, `publish_ack()`, `subscribe()`, `discover()`. Also available as Python, TypeScript, and Go SDKs with identical patterns.

**HTTP API** (engine, axum): `GET /api/topology` on port 8891. Returns JSON array of all primitives with name, kind, state, publishes, subscribes, pid. No authentication, no API versioning, no OpenAPI spec.

**Pub-sub meta-protocol**: `system.request.topology` / `system.response.topology` and `system.request.subscriptions` / `system.response.subscriptions` are first-class message-type pairs for runtime introspection from within the fabric (`connection.rs:307-413`).

**CLI**: `emergent --config <file>`, `emergent init`, `emergent scaffold`, `emergent marketplace <subcommand>`, `emergent update`.

**Comparison with Nebula**: Nebula has REST + planned GraphQL/gRPC with OpenAPI spec generation and OwnerId-aware per-tenant routing. Emergent has a minimal read-only topology HTTP endpoint with no versioning or auth.

---

## 15. Testing Infrastructure [A19]

**Unit tests**: Present in source files (`#[cfg(test)]` modules). `config.rs` has ~25 tests covering path expansion, TOML parsing, and validation. `sqlite.rs` has ~5 tests covering storage/query operations. `process_manager.rs` has 2 basic tests. Engine tests do not spin up real child processes — they test config logic and data transformations only.

**Integration tests**: The Rust SDK (`sdks/rust/tests/publish_stream.rs`) has integration tests for the pull-based streaming protocol. Python SDK (`sdks/py/tests/`) has ~50 tests covering protocol, message types, streaming.

**No public testing utilities** equivalent to Nebula's `nebula-testing` crate. No contract tests for primitive implementors.

**Workspace lints** (`Cargo.toml:21-23`): `unwrap_used = "deny"`, `expect_used = "deny"` — enforces proper error handling across the codebase.

---

## 16. AI / LLM Integration [A21]

### A21.1 — Existence

**No built-in or first-class LLM integration exists in the engine or SDK.**

Searched all Rust, Python, TypeScript, and Go source files for: `openai`, `anthropic`, `llm`, `embedding`, `completion`, `gpt`, `claude` (as code), `llama`, `rag`, `vector`, `inference`. Results:
- In Python SDK `types.py`: `from pydantic import BaseModel` and `model_config = ConfigDict(...)` — this is Pydantic's `model_config`, not an AI model.
- In `main.rs:475`: `actor.model.message_count += 1` — acton-reactive's actor model reference, not an AI model.
- Zero results for OpenAI/Anthropic API calls, LLM client libraries, embedding functions, vector store integrations.

LLM integration in Emergent is achieved **by design through the exec-handler pattern**: any CLI tool (including `claude`, `ollama`, curl to an OpenAI API) can be wrapped as an exec-handler. This is explicitly showcased in the README:
```toml
args = ["--", "sh", "-c",
  "response=$(echo \"$input\" | claude -p 2>/dev/null) && ..."]
```
The engine is entirely agnostic to whether the child process calls an LLM or a regex.

### A21.2 — Provider abstraction

None. The "abstraction" is that you change one `args` line in TOML to switch from Claude to Ollama to GPT-4. This is not a provider trait — it is subprocess substitution.

### A21.3 — Prompt management

None built in. Prompts live in shell snippets within TOML `args` or in primitive code. No templating, no few-shot storage, no versioning. Issue #28 proposes a "standardized tool request/response message protocol" which could evolve toward structured prompt/response patterns, but this is not implemented.

### A21.4 — Structured output

None. The engine passes raw JSON payloads. Schema enforcement for LLM outputs must be done in the handler (e.g., piping through `jq` to extract fields from Claude's response). No JSON Schema enforcement, no re-prompting on validation fail.

### A21.5 — Tool calling

None in the engine. Issue #28 ("feat(engine): standardized tool request/response message protocol") explicitly proposes building this — confirming it does not exist yet. The pub-sub fabric could theoretically implement tool calling as a request/response message pattern (source→handler→sink with correlation IDs), but this is not scaffolded.

### A21.6 — Streaming

Pull-based streaming was added to the SDK (`publish_stream`, `stream_offer`, `stream_consume` in `connection.rs:616-1055`). This could be used to stream LLM tokens if the primitive implements it. The engine does not natively handle SSE/chunked streaming from LLM APIs. Issue #27 tracks "streaming/chunked message delivery" as a future feature.

### A21.7 — Multi-agent

Not built in. The pub-sub architecture theoretically supports agent-to-agent message passing (agent A publishes a topic that agent B subscribes to), but there is no concept of an "agent", no handoff protocol, no shared memory, and no termination conditions tracked by the engine. Feedback loops are supported (cycles in pub-sub graph, as shown in the ouroboros example), which is the structural prerequisite for agent loops.

### A21.8 — RAG / Vector

None. No embedding APIs, no vector store integrations (Qdrant, pgvector, Pinecone, Weaviate). A handler could call these via exec, but the engine has no knowledge of them.

### A21.9 — Memory / Context

None. `EmergentMessage` has `correlation_id` for request grouping and `causation_id` for chain tracking, but there is no conversation memory, session management, or context window management.

### A21.10 — Cost / Tokens

None. No token counting, no per-provider cost calculation, no budget circuit breakers.

### A21.11 — Observability

None specific to LLM calls. LLM-call traces would appear as regular handler processing time in the `tracing` logs if the handler logs them.

### A21.12 — Safety

None. No content filtering, no prompt injection mitigation, no output validation. These concerns are fully delegated to the primitive's internal implementation.

### A21.13 — vs Nebula + Surge

Nebula's position: "No first-class LLM abstraction yet. Strategic bet: AI workflows realized through generic actions + plugin LLM client. Surge (separate project) handles agent orchestration on ACP."

Emergent's position: similar structural bet ("AI = subprocess call"), but taken further. Emergent's exec-handler makes this pattern completely zero-code (just a TOML line), whereas Nebula's "generic action" still requires writing a Rust struct. Emergent also acknowledges in its README that it differentiates from LangChain/CrewAI by being language-agnostic and process-isolated. However, Emergent has zero LLM-specific affordances (no token counting, no structured tool calling, no streaming, no prompt management), while Nebula's roadmap at least acknowledges an LLM plugin model. The conclusion: **same strategic bet, Emergent is further from first-class LLM support**, treating "any CLI tool" as the universal abstraction including LLMs.

---

## 17. Notable Design Decisions

### 17.1 — Everything is a process; the engine is a message bus

The single most important architectural decision: the engine does not execute user code. It only routes messages between OS processes. This eliminates the need for sandboxing, resource management, credential injection, or language-specific runtimes in the engine. The trade-off: no compile-time type safety across steps, no resource pooling, no execution tracing inside a step, no per-step retry logic.

**Applicability to Nebula**: Nebula could offer a "subprocess mode" for steps that cannot or should not run in-process (e.g., untrusted plugins). The emergent model shows how minimal an engine can be when it delegates execution entirely.

### 17.2 — TOML-as-topology over code-as-topology

Pipeline topology is declared in a single TOML file, not in Rust code. This enables `git diff` review of topology changes, simple deployment (one file), and language-agnostic composition. The trade-off: no compile-time topology validation, no type-checking across step boundaries.

**Applicability to Nebula**: Nebula's TypeDAG enforces topology at compile time (L1-L2) and runtime (L3-L4). A TOML-based mode for Nebula would weaken those guarantees but could enable non-Rust step authors to build pipelines.

### 17.3 — Pub-sub over DAG enables cycles

The ouroboros-loop and feedback-loop examples exploit this deliberately. In LangChain, Step Functions, n8n — you cannot have cycles. In Emergent you get them for free. This is architecturally significant for agent loops (LLM calls itself), gaming simulations (Game of Life, reaction-diffusion), and self-monitoring pipelines.

**Applicability to Nebula**: Nebula's TypeDAG L4 (petgraph) does detect cycles. A cycle-aware extension could enable Emergent-style feedback loops within Nebula's typed framework.

### 17.4 — Three-phase ordered shutdown for zero message loss

Sources stop → handlers drain → sinks drain. This is carefully implemented in `ProcessManager::graceful_shutdown()` with phase-appropriate signaling (`SIGTERM` for sources, `system.shutdown` broadcast for handlers/sinks). This is more sophisticated than a naive SIGKILL approach.

**Applicability to Nebula**: Nebula has execution lifecycle (cancel/drain). The emergent three-phase pattern with `system.shutdown` broadcast to the pub-sub fabric is elegant — primitives opt into graceful shutdown by subscribing to `system.shutdown`. Nebula could adopt a similar pattern.

### 17.5 — Marketplace as git-repo registry

The marketplace uses a plain git repository (`emergent-registry`) as the source of truth. `git clone/pull` provides versioning, no server infrastructure, offline-capable, auditable. Binary distribution via GitHub Releases. The SHA256 checksum verification code exists (`installer.rs:375-390`) but is currently marked `#[allow(dead_code)]` — checksums are not yet verified at install time.

**Potential borrow for Nebula**: Nebula's Plugin Fund model could use a similar git-based registry for plugin discovery, while keeping the WASM sandbox approach for execution.

---

## 18. Known Limitations / Pain Points

All cited from GitHub issues:

1. **No process sandbox** (issue [#25](https://github.com/govcraft/emergent/issues/25)): "feat(engine): OS-level sandboxing for primitive processes" — currently any primitive has full OS permissions. No isolation.

2. **No IPC authentication** (issue [#24](https://github.com/govcraft/emergent/issues/24)): "feat(engine): authenticate primitive connections by spawned PID" — any process that knows the socket path can connect. Open enhancement.

3. **Pub/sub declarations not enforced** (issue [#23](https://github.com/govcraft/emergent/issues/23)): TOML `publishes`/`subscribes` are informational only. The broker does not enforce them. A primitive can publish to any topic regardless of its declared list.

4. **Fast-interval sources silently stop** (closed issue [#33](https://github.com/govcraft/emergent/issues/33)): "bug(ipc): fast-interval exec-sources silently stop publishing after ~2 minutes." Fixed but indicates IPC heartbeat fragility.

5. **No crash recovery / restart** (no dedicated issue but confirmed from source): `PrimitiveState::Failed` is terminal. No automatic restart policy.

6. **Stale socket on unclean shutdown** (closed issue [#8](https://github.com/govcraft/emergent/issues/8)): engine startup blocked by leftover socket file if previous run crashed without cleanup.

7. **No credential management**: By design, but limits enterprise adoption. Secrets in TOML `env` blocks appear in config files and process environments.

8. **SHA256 not verified at install** (`installer.rs:375`, `#[allow(dead_code)]`): checksum infrastructure exists but is not called from the install flow. Supply-chain risk.

---

## 19. Bus Factor / Sustainability

- **Maintainer**: 1 (Roland Rodriguez, govcraft.ai)
- **Stars**: 18 | **Forks**: 0
- **Created**: 2026-01-06 (3.5 months old at analysis time)
- **Commit cadence**: Active — 20 commits visible in depth-50 history, spanning from initial setup to streaming additions and SDK refinements
- **Issues ratio**: 13 open / 24 closed — healthy iteration cycle for a young project
- **Release frequency**: SDK versions progressed from 0.10.x (engine) and 0.11.x → 0.13.x (SDK) in 3.5 months — rapid iteration
- **External dependencies**: Heavy reliance on `acton-reactive` (govcraft's own actor framework). If acton-reactive loses maintenance, emergent is deeply affected.
- **Sustainability risk**: Solo maintainer, no commercial model, no funding signal. Low stars suggest limited community adoption so far.

---

## 20. Final Scorecard vs Nebula

| Axis | emergent approach | Nebula approach | Verdict | Borrow? |
|------|-------------------|-----------------|---------|---------|
| **A1 Workspace** | 2 core crates (engine + client), 5 example members; no layer separation within engine | 26 crates, layered: nebula-error/resilience/credential/resource/action/engine/tenant/eventbus | Nebula deeper; emergent intentionally flat | no — different goals |
| **A2 DAG** | Pub-sub multigraph with cycles; no compile-time topology check; acton-reactive routing at runtime | TypeDAG L1-L4: static generics → TypeId → predicates → petgraph | Nebula deeper; emergent explicitly rejects DAG by design | maybe — cycle support idea worth ADR |
| **A3 Action** | 3 OS process kinds (Source/Handler/Sink); no Rust trait; `serde_json::Value` I/O; scaffold DX | 5 action kinds, sealed traits, assoc Input/Output/Error, derive macros, versioning | Nebula much deeper; emergent trades type safety for subprocess freedom | no — Nebula already richer |
| **A4 Credential** | None — env vars in TOML config, passed to child processes; no encryption, no lifecycle | State/Material split, LiveCredential watch(), blue-green refresh, OAuth2Protocol | Nebula much deeper | no — Nebula already richer |
| **A5 Resource** | None — each subprocess manages own resources | 4 scope levels, ReloadOutcome, generation tracking, on_credential_refresh | Nebula much deeper | no — Nebula already richer |
| **A6 Resilience** | Timeout on exec invocation (user-land only); no retry/CB/bulkhead at engine | retry/CB/bulkhead/timeout/hedging in nebula-resilience | Nebula much deeper | no — Nebula already richer |
| **A7 Expression** | None — jq/awk in exec-handler subprocess | 60+ funcs, type inference, sandboxed eval | Nebula much deeper | no — Nebula already richer |
| **A8 Storage** | SQLite (rusqlite bundled) + JSON log; local only; no RLS | sqlx + PgPool + RLS; SQL migrations | Nebula deeper (enterprise); emergent simpler (single-node) | no — different goals |
| **A9 Persistence** | Append-only log + SQLite for observability; no checkpoint/frontier recovery | Frontier + checkpoint + append-only; state reconstruction via replay | Nebula deeper; emergent restart-from-zero | no — Nebula already richer |
| **A10 Concurrency** | tokio + acton-reactive actor system + OS process isolation per primitive | tokio + frontier scheduler + !Send isolation | Different decomposition; emergent's OS-process isolation is structurally stronger per-step | refine — subprocess mode idea |
| **A11 Plugin BUILD** | git-repo registry + pre-built binaries + manifest.toml; SHA256 present but disabled | WASM, plugin-v2 spec, Plugin Fund commercial model | Different approach: emergent simpler/available now; Nebula more secure/not yet shipped | refine — git-repo registry for Nebula plugin discovery |
| **A11 Plugin EXEC** | OS subprocess — no sandbox, no trust model, no capability system | WASM sandbox + capability security (planned) | Nebula planned approach is more correct for security; emergent's approach is simpler and works now | no — Nebula's WASM bet is right |
| **A12 Trigger** | Sources = primitives (processes); interval, webhook (marketplace), WebSocket, system events; no cron | TriggerAction with Input=Config, Output=Event; Source trait 2-stage normalization | Different decomposition: Nebula richer type model; emergent simpler dispatch | no — Nebula already richer |
| **A21 AI/LLM** | None native — exec-handler subprocess is the "abstraction" (change one TOML line to change model) | None yet — generic actions + plugin LLM client bet | Same strategic bet; emergent makes it zero-code via exec-handler; Nebula needs a Rust action | refine — exec-handler-style "run any command as action" could inform Nebula's LLM plugin DX |

**14 rows filled (A11 split into BUILD + EXEC).**

---

## Summary of Borrowable Ideas

1. **Three-phase ordered shutdown via pub-sub**: `system.shutdown` broadcast to fabric instead of engine-managed teardown. Primitives opt into graceful shutdown by subscription. Clean and language-agnostic.

2. **Git-repo registry for plugin discovery**: The `emergent-registry` pattern (index.toml + per-primitive manifest.toml in a plain git repo) is operational today. Nebula's Plugin Fund could use this for discovery while keeping WASM for execution.

3. **Feedback loop / cycle-aware execution**: The pub-sub architecture naturally supports cycles. Nebula's TypeDAG L4 detects cycles but forbids them. An opt-in "loop" mode (explicitly declared in workflow) could enable LLM-style agent loops.

4. **"Run any command as action" DX**: emergent's exec-handler makes integrating any CLI tool (including LLMs) zero-code. A Nebula `ExecAction` wrapper for shell-command steps could lower the barrier for non-Rust tool integration without compromising Nebula's type model for Rust-native steps.
