# Architectural Decisions

Key design choices and the reasoning behind them. These are the decisions that shape every crate and should be understood before making large changes.

---

## ADR-001: Rust as the Implementation Language

**Decision:** Nebula is written in Rust.

**Why:**
- Memory safety without GC — critical for a long-running server-side engine.
- The type system makes architectural invariants (layer separation, typed IDs, state machines) enforceable at compile time, not just convention.
- Async Rust (Tokio) provides high-throughput, low-latency concurrency with cooperative scheduling.
- `serde` makes JSON/binary serialization ergonomic and fast.

**Trade-offs:**
- Steeper learning curve than Go/Python.
- Longer build times (mitigated by incremental compilation and `sccache`).
- Fewer workflow-engine libraries than Node.js ecosystem — we build them ourselves.

---

## ADR-002: serde_json::Value as the Universal Data Type

**Decision:** All workflow runtime data is `serde_json::Value`. There is no `nebula-value` crate.

**Why:**
- Workflow data crosses many boundaries: trigger → engine → action → storage → API. A single JSON-native type eliminates conversion layers.
- `serde_json::Value` is already a dependency everywhere; adding a custom value type would add a dependency without removing others.
- Actions written by third parties already know JSON; we don't want to teach them a new type.

**Consequences:**
- Type information beyond JSON primitives (dates, decimals, binary) is encoded by convention (ISO-8601 strings, base64, etc.) and documented in parameter schemas.
- `nebula-parameter` describes the schema; `nebula-expression` evaluates; the value itself is always `Value`.

**Rejected alternatives:**
- Custom `NebulaValue` enum: added complexity, required conversion at every API/storage boundary, no meaningful gain.

---

## ADR-003: One-Way Layer Dependencies

**Decision:** Dependency direction is strictly Infrastructure → Core → Business → Execution → API. No upward or circular dependencies.

**Why:**
- Makes the system testable in isolation: core crates have no runtime dependencies.
- Prevents tight coupling: changing the storage backend cannot break action authors.
- Enforced by `cargo deny` rules in `deny.toml`.

**Allowed exceptions:**
- Cross-cutting crates (`config`, `log`, `resilience`, `eventbus`, `metrics`) may be imported at any layer. They contain no business logic.
- `nebula-core` is the one crate imported everywhere; it must stay small and stable.

---

## ADR-004: DAG-Based Workflow Model

**Decision:** Workflows are DAGs (directed acyclic graphs) of typed nodes.

**Why:**
- DAGs are well-understood, efficiently schedulable (topological sort), and support parallel fan-out naturally.
- Acyclicity enforces termination — no infinite loops by accident. Explicit loop constructs can be added later.
- `petgraph` provides a proven, performant DAG implementation.

**Consequences:**
- Cyclic workflows must be modeled as repeated executions or explicit loop nodes (future).
- Long-running stateful workflows (sagas) use `StatefulAction` + `TransactionalAction` within a single DAG.

---

## ADR-005: Credential Encryption at Rest (AES-256-GCM)

**Decision:** All stored credentials are encrypted with AES-256-GCM. Keys are never stored alongside data.

**Why:**
- Workflow credentials (API keys, OAuth tokens, DB passwords) are high-value targets.
- AES-256-GCM provides authenticated encryption — tampering is detected.
- Envelope encryption allows key rotation without re-encrypting all credentials.

**Consequences:**
- Credential storage requires a key management service (local file, AWS KMS, HashiCorp Vault).
- Performance overhead is acceptable: credentials are cached and decrypted once per workflow run.

---

## ADR-006: Tauri Desktop App (not Electron, not egui)

**Decision:** The desktop client is a Tauri app (`apps/desktop`) with React + TypeScript frontend and Rust backend. The previous `nebula-app` (egui-based) is abandoned.

**Why:**
- Tauri produces smaller binaries than Electron (no bundled Chromium).
- Tauri's IPC is typed via `tauri-specta` — the same Rust types drive both the backend and frontend TypeScript.
- React + TypeScript is a stronger ecosystem for complex UIs (workflow canvas, data tables) than immediate-mode egui.
- The Rust backend can share code with the server-side engine directly.

**Consequences:**
- Requires Node.js toolchain for frontend development.
- WebView rendering varies by OS (WebKit on macOS/Linux, WebView2 on Windows).

---

## ADR-007: REST + WebSocket API (no GraphQL)

**Decision:** `nebula-api` exposes REST for CRUD operations and WebSocket for real-time events. GraphQL is not planned.

**Why:**
- REST is sufficient for workflow CRUD; OpenAPI tooling generates clients automatically.
- WebSocket is the right transport for streaming execution logs and real-time events.
- GraphQL adds complexity (schema, resolver, N+1 query patterns) without a clear benefit for our use cases.

**Consequences:**
- API versioning is done via URL prefix (`/v1/`, `/v2/`).
- Real-time subscriptions use WebSocket message types; no SSE.

---

## ADR-008: In-Process Sandbox as Phase 2 Default

**Decision:** Phase 2 uses `InProcessSandbox` — actions run in the same process as the engine. WASM or OS-process isolation is deferred to Phase 3+.

**Why:**
- In-process execution is orders of magnitude faster (no IPC overhead) and simpler to implement.
- Phase 2 focus is correctness (state persistence, DAG resolution) not security isolation.
- Capability-checked contexts (`ActionContext` with `ResourceAccessor`, `CredentialAccessor`) provide logical isolation even in-process.

**Consequences:**
- A panicking action can crash the engine in Phase 2. Mitigated by `catch_unwind` in the sandbox.
- WASM sandbox (`wasmtime`) or OS process isolation is the Phase 3 target.

---

## ADR-009: PostgreSQL as the Production State Store

**Decision:** PostgreSQL is the primary persistence backend. Redis is a secondary cache/queue option. No other databases planned.

**Why:**
- PostgreSQL supports transactions, advisory locks, and LISTEN/NOTIFY — all needed for reliable workflow state.
- `sqlx` provides compile-time query checking and async execution without an ORM.
- Avoiding an ORM removes a dependency and keeps the query layer explicit and tunable.

**Consequences:**
- Production deployments require a PostgreSQL instance.
- `MemoryStorage` remains the default for tests and local development.
- The `nebula-storage` trait abstracts the difference; consumers do not care which backend is in use.

---

## ADR-010: Eventbus as a Platform-Wide Abstraction

**Decision:** All inter-crate events (execution lifecycle, credential rotation, resource health) flow through `nebula-eventbus`. Direct channel sharing between crates is phased out.

**Why:**
- Today, credential rotation uses a direct `broadcast::Sender` shared between `nebula-credential` and `nebula-resource`. As more crates emit events (engine, telemetry, webhook), ad-hoc channels become unmanageable.
- A typed eventbus enforces that event producers and consumers are decoupled: `nebula-credential` emits `CredentialRotationEvent`; `nebula-resource` subscribes without holding a reference to the credential manager.

**Consequences:**
- `nebula-eventbus` becomes a cross-cutting dependency (like `nebula-log`).
- Event types must be versioned when their shape changes.
