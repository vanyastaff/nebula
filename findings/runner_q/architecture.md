# RunnerQ — Architectural Decomposition

## 0. Project Metadata

- **Repo:** https://github.com/alob-mtc/runnerq
- **Stars:** 26 (April 2026)
- **Forks:** 4
- **Last activity:** 2026-04-15
- **License:** MIT
- **Governance:** Solo maintainer, Akinlua Bolamigbe (`bolamigbeakinlua@gmail.com`)
- **Published to crates.io:** `runner_q` v0.6.4, `runner_q_redis` v0.1.1
- **Open issues:** 8; closed issues: ~18 (tracked manually below)

---

## 1. Concept Positioning [A1, A13, A20]

**Author's framing (README.md, line 3):** "A robust, scalable activity queue and worker system for Rust applications with pluggable storage backends."

**Mine, after reading code:** RunnerQ is a single-queue background-job processor with at-most-once-then-DLQ semantics, prioritized dequeue, time-delayed execution, idempotency keys, and a pluggable storage abstraction (PostgreSQL built-in, Redis via separate crate). It is a Rust-native alternative to Sidekiq/Bull/BullMQ, not a workflow DAG engine.

**Comparison with Nebula:** RunnerQ occupies the "background job runner" niche rather than the "workflow orchestration engine" niche. It has no DAG, no typed ports between steps, no credential subsystem, no resource lifecycle, no expression engine, and no trigger model — all axes Nebula builds around. The useful comparison is specifically at the queue/execution-loop layer (Nebula's nebula-engine frontier scheduler vs. RunnerQ's worker-pool dequeue loop).

---

## 2. Workspace Structure [A1]

**2 crates in workspace** (`Cargo.toml` line 2):
- `runner_q` (root): main library, v0.6.4, Edition 2021
- `runner_q_redis`: Redis/Valkey backend, v0.1.1, separate optional crate

**Feature flags** (`Cargo.toml` lines 36-41):
```toml
[features]
default = ["postgres", "axum-ui"]
postgres = ["dep:sqlx", "dep:async-stream"]
axum-ui = ["dep:axum", "dep:tower-http"]
```

There are no "layer" separations in the architectural sense Nebula uses (no separate error, resilience, credential, resource crates). The layering is flat: `config.rs`, `activity/`, `queue/`, `runner/`, `storage/`, `observability/`.

**Comparison with Nebula:** Nebula has 26 crates with strict dependency layers (nebula-error → nebula-resilience → nebula-credential → nebula-resource → nebula-action → nebula-engine). RunnerQ is a 2-crate micro-library with feature flags rather than crate boundaries. Much simpler; much less composable.

---

## 3. Core Abstractions [A3, A17] — DEEP

### A3.1 Trait shape

The central trait is `ActivityHandler` (`src/activity/activity.rs:346`):

```rust
#[async_trait]
pub trait ActivityHandler: Send + Sync {
    fn activity_type(&self) -> String;
    async fn handle(&self, payload: serde_json::Value, context: ActivityContext) -> ActivityHandlerResult;
    async fn on_dead_letter(&self, _payload: serde_json::Value, _context: ActivityContext, _error: String) {}
}
```

- **Open, not sealed:** any external crate can implement `ActivityHandler`. No sealing mechanism.
- **`dyn`-compatible:** stored as `Arc<dyn ActivityHandler>` in the registry (`ActivityHandlerRegistry = HashMap<String, Arc<dyn ActivityHandler>>`; `src/activity/activity.rs:402`).
- **Associated types:** zero — the trait has no `Input`, `Output`, or `Error` associated types. All I/O is `serde_json::Value`.
- **No GATs, no HRTBs, no typestate.**
- **Default method:** `on_dead_letter` is a default no-op, making it optional.

The secondary trait is `ActivityExecutor` (`src/runner/runner.rs:1283`):

```rust
#[async_trait]
pub trait ActivityExecutor: Send + Sync {
    fn activity(&self, activity_type: &str) -> ActivityBuilder<'_>;
}
```

This is how handlers enqueue sub-activities for orchestration. It is `dyn`-compatible and injected into `ActivityContext.activity_executor`.

### A3.2 I/O shape

- **Input:** `serde_json::Value` — fully type-erased. No generic associated type, no schema enforcement.
- **Output:** `ActivityHandlerResult = Result<Option<serde_json::Value>, ActivityError>` — also type-erased.
- **No streaming output.** Results are stored via `store_result()` to the backend and polled by `ActivityFuture::get_result()` which spins every 100 ms (`src/activity/activity.rs:461`).
- **Side-effects model:** open — the handler can call any I/O.

### A3.3 Versioning

No versioning. Activities are identified by a `String` activity type (e.g., `"send_email"`). There is no `v1`/`v2` distinction, no migration support, no `#[deprecated]`. Referenced by name only.

### A3.4 Lifecycle hooks

Two lifecycle points: `handle` (execute) and `on_dead_letter` (failure terminal callback). No pre/execute/post/cleanup pattern. No `cancel` hook — cancellation is opt-in via `ActivityContext.cancel_token: CancellationToken` (`src/activity/activity.rs:291`).

### A3.5 Resource and credential deps

No abstraction exists. The handler is a plain struct; dependencies (DB pools, HTTP clients) are injected via normal Rust constructor injection — fields on the implementing struct. There is no framework-level declaration of "I need DB pool X + credential Y."

### A3.6 Retry/resilience attachment

Retry is per-activity-enqueue, not per-handler. `ActivityOption.max_retries` (`src/activity/activity.rs:153`) is set at dispatch time. The exponential backoff delay is computed in the backend (`ack_failure` in `src/storage/postgres/mod.rs:812`): `base_delay * 2^(retry_count+1)`. No circuit breaker, no bulkhead, no global policy.

### A3.7 Authoring DX

Minimal: implement two methods (`activity_type()` and `handle()`), register with `engine.register_activity("type", Arc::new(MyHandler))`. No derive macro, no builder for the trait itself. The builder pattern is for dispatch (`ActivityBuilder`), not handler authoring. Hello-world handler: ~10 lines of trait impl.

### A3.8 Metadata

Activity types are bare strings at runtime. No display name, description, icon, or category. Custom metadata is a `HashMap<String, String>` stored in the `Activity` struct (`src/activity/activity.rs:191`) — it is caller-supplied data, not a handler-level declaration. No i18n.

### A3.9 vs Nebula

Nebula has 5 sealed action kinds (Process/Supply/Trigger/Event/Schedule) with associated `Input`/`Output`/`Error` types, versioning, and derive macros. RunnerQ has exactly 1 kind ("activity handler") with no associated types, all I/O is `serde_json::Value`. RunnerQ is simpler and more approachable but provides zero compile-time guarantees on I/O shape.

---

## 4. DAG / Execution Graph [A2, A9, A10]

**No DAG.** RunnerQ has no graph model, no port typing, no topological ordering, no dependency resolution between activities. Activities can enqueue other activities via `ActivityContext.activity_executor`, creating implicit "fan-out" chains, but there is no tracked DAG structure — each enqueued activity is independent. This is closer to Sidekiq's fire-and-forget chaining than to Temporal's workflow graph or Nebula's TypeDAG.

**Scheduler model:** `N` independent worker loops (one `tokio::spawn` per `max_concurrent_activities`; `src/runner/runner.rs:303`). Each loop continuously dequeues one activity, executes it, acks, then loops. No semaphore; the bound is the loop count. Exponential backoff (100ms base, 5s max) when the queue is empty (`src/runner/runner.rs:417`).

**Comparison with Nebula:** Nebula's TypeDAG tracks port connections and enforces types at L1-L4. Nebula's frontier scheduler tracks execution state per-node. RunnerQ has neither; it is a flat queue with no DAG semantics.

---

## 5. Persistence and Recovery [A8, A9]

### A8 — Storage layer

**PostgreSQL backend** (`src/storage/postgres/mod.rs`):
- `sqlx` + `PgPool`, feature-gated (`postgres` feature)
- 4 tables created inline at startup via `SCHEMA_SQL` (`src/storage/postgres/mod.rs:1589`):
  - `runnerq_activities` — main table, all statuses in one table
  - `runnerq_events` — append-only event log per activity
  - `runnerq_results` — activity result storage
  - `runnerq_idempotency` — idempotency key → activity_id mapping
- No separate migration tool (no sqlx `migrate!` macro, no Flyway); schema is idempotent `CREATE TABLE IF NOT EXISTS`

**Redis backend** (`runner_q_redis` crate):
- `bb8-redis` connection pool
- ZSETs for main queue (priority-scored), scheduled set, processing set
- Hashes for activity data; Redis Streams for events
- `SET NX EX 24h` for idempotency keys
- TTL-based retention (7d snapshots, 24h results, 1h events)

### A9 — Persistence model

- **Not append-only.** The `runnerq_activities` row is mutated in place as status transitions (`pending` → `processing` → `completed`/`retrying`/`dead_letter`).
- **Event log exists but is secondary.** `runnerq_events` is an append-only timeline of events per activity.
- **No checkpoint/replay recovery.** Recovery is lease-based: a background `Reaper` scans for activities in `processing` whose `lease_deadline_ms` has expired and resets them to `pending` (`src/storage/postgres/mod.rs:916`). This is at-least-once-on-crash, not durable replay.
- **No frontier-based scheduling.** Contrast with Nebula's frontier-based checkpoint where execution state is reconstructed from an append-only log.

**Comparison with Nebula:** Nebula uses a frontier-based scheduler with checkpoint recovery and append-only execution log. RunnerQ uses lease-based at-least-once recovery. Both support recovery from worker crash, but Nebula's model supports deterministic replay while RunnerQ's does not.

---

## 6. Credentials / Secrets [A4] — DEEP

### A4.1 Existence

**No credential layer exists in RunnerQ.** Searched for "credential", "secret", "token", "auth", "oauth", "jwt", "api_key" across all `.rs` files.

Grep evidence:
```
grep -rn "credential\|secret\|oauth\|jwt\|api_key" --include="*.rs" -i
```
Results:
- `runner_q_redis/src/backend/pool.rs:143` — comment only: "Redact credentials in logs" (but no actual implementation of credential redaction).
- `src/activity/activity.rs:45` — in doc comment: "for permanent failures like invalid input data, authentication errors" (context only, not a credential system).
- No `SecretString`, no `Zeroize`, no OAuth2 code, no vault integration, no lifecycle management.

**Design decision or omission?** Omission — RunnerQ's scope is the queue/worker layer. Secrets are passed to handler constructors by the application. This is consistent with its role as a library, not a platform.

### A4.2–A4.9

All absent. No at-rest encryption, no in-memory protection (`secrecy::Secret`), no key rotation, no OAuth2/OIDC, no scope concept, no type-state for validated vs unvalidated credentials.

**A4.9 vs Nebula:** Nebula's credential subsystem (State/Material split, `LiveCredential`, blue-green refresh, `OAuth2Protocol`, `DynAdapter`) has no equivalent in RunnerQ. RunnerQ delegates credential concerns entirely to the application layer.

---

## 7. Resource Management [A5] — DEEP

### A5.1 Existence

**No resource abstraction exists.** Searched for "resource", "pool", "lifecycle", "ReloadOutcome", "scope" in the context of a resource framework.

Grep evidence:
```
grep -rn "resource\|ReloadOutcome\|on_credential_refresh" --include="*.rs" -i
```
Nothing found that indicates a framework-level resource concept. Connection pools (`PgPool`, `bb8-redis` pool) are created by the backend implementations themselves and held inside the backend structs.

### A5.2–A5.8

All absent:
- **A5.2 Scoping:** No scope levels. Resources are scoped implicitly to the backend lifetime (= application lifetime).
- **A5.3 Lifecycle hooks:** `PostgresBackend::new()` runs `init_schema()` synchronously at construction (`src/storage/postgres/mod.rs:193`). No shutdown hook, no health-check protocol.
- **A5.4 Reload:** No hot-reload, no blue-green swap, no `ReloadOutcome`.
- **A5.5 Sharing:** `PostgresBackend` is `Clone` (wraps a `PgPool` which is itself an `Arc<T>`; `src/storage/postgres/mod.rs:83`). Multiple engines can share a backend via `Arc<dyn Storage>`.
- **A5.6 Credential deps:** No notification of credential rotation.
- **A5.7 Backpressure:** Pool limits are configured at the DB layer (`PgPoolOptions::max_connections`). RunnerQ does not expose a queue-side backpressure mechanism.
- **A5.8 vs Nebula:** Nebula has 4 scope levels (Global/Workflow/Execution/Action), `ReloadOutcome`, generation tracking, and `on_credential_refresh`. RunnerQ has none of these.

---

## 8. Resilience [A6, A18]

### A6 — Resilience patterns

RunnerQ embeds some resilience at the execution layer:
- **Retry with exponential backoff:** Built into the backend ack_failure path. Formula: `base_delay * 2^(retry_count+1)` (`src/storage/postgres/mod.rs:812`). Redis backend has the same logic. Cap: not explicitly in code; open issue #43 "Retry exponential backoff should be capped at 1hr" (closed as fixed).
- **Timeout:** `tokio::time::timeout(activity.timeout_seconds, handle_fut)` per activity (`src/runner/runner.rs:503`). On timeout, activity is retried.
- **Panic safety:** `AssertUnwindSafe(handle_fut).catch_unwind()` converts panics to `ActivityError::Retry` (`src/runner/runner.rs:505`).
- **Dead letter queue:** After `max_retries` exhausted, activity moves to `dead_letter` status.
- **Lease reaper:** Background task reclaims crashed-worker activities.

**No circuit breaker, no bulkhead, no hedging.** These patterns are absent and would need to be implemented by the application if needed.

### A18 — Error types

**Two error crates worth of types:**
- `ActivityError` (`src/activity/error.rs:36`): 2 variants: `Retry(String)` and `NonRetry(String)`. Deliberately simple — the Retry/NonRetry distinction drives the entire retry logic.
- `WorkerError` (`src/runner/error.rs:5`): 13 variants covering queue, serialization, timeout, configuration, shutdown, idempotency, etc. Uses `thiserror`.
- `StorageError` (`src/storage/error.rs`): backend-specific, mapped to `WorkerError` via `From<StorageError>`.
- **No unified error classification enum** (no equivalent to Nebula's `ErrorClass` with transient/permanent/cancelled categories beyond Retry/NonRetry).

**Comparison with Nebula:** Nebula has `nebula-error` with `ErrorClass` used by `ErrorClassifier` in `nebula-resilience` to route between retry policies. RunnerQ's error model is simpler: `Retry` vs `NonRetry` is the only classification, and the "policy" is just `max_retries` with exponential backoff.

---

## 9. Expression / Data Routing [A7]

**No expression engine.** RunnerQ has no DSL, no template system, no `$nodes.foo.result` syntax, no data routing between steps. Payload is passed as-is (`serde_json::Value`) from enqueue to handler.

Grep evidence:
```
grep -rn "expression\|eval\|template\|jinja\|handlebars\|\$nodes" --include="*.rs" -i
```
Found nothing.

**Comparison with Nebula:** Nebula's expression engine with 60+ functions, type inference, and sandboxed evaluation has no equivalent.

---

## 10. Plugin / Extension System [A11] — DEEP

### 10.A — Plugin BUILD process

**A11.1–A11.4:** No plugin system exists. Searched for "plugin", "wasm", "sandbox", "extension", "hook" across all `.rs` files.

Grep evidence:
```
grep -rn "plugin\|wasm\|sandbox\|extension" --include="*.rs" -i
```
Result (from `src/runner/runner.rs:437`):
```
warn!(activity_id = %activity.id, "Lease extension skipped; member not found in processing set")
```
The word "extension" appears only in the context of lease extension, not plugin extension.

**Design approach:** The extension point in RunnerQ is the `Storage` trait — any user can implement `QueueStorage + InspectionStorage` to provide a custom backend. This is the only formalized extension mechanism. There is no plugin runtime, no manifest format, no registry.

**No BUILD toolchain for plugins — concept does not exist.**

### 10.B — Plugin EXECUTION sandbox

**A11.5–A11.9:** Not applicable. There is no plugin execution sandbox, no WASM runtime, no subprocess isolation, no capability-based security. Activity handlers are plain Rust structs compiled into the same binary.

**A11.9 vs Nebula:** Nebula targets WASM sandbox + capability security + Plugin Fund commercial model. RunnerQ has no such ambition — it is a library embedded in the application binary.

---

## 11. Trigger / Event Model [A12] — DEEP

### A12.1 Trigger types

RunnerQ does not have a trigger model. It is a pull-based queue: callers explicitly call `engine.get_activity_executor().activity("type").payload(...).execute().await` to enqueue. There are no:
- Webhooks
- Cron/schedule triggers (only time-delayed execution of already-enqueued activities)
- External event consumers (no Kafka, RabbitMQ, NATS integration)
- FS watch or DB CDC triggers
- Internal event triggers

Grep evidence:
```
grep -rn "webhook\|cron\|trigger\|kafka\|rabbitmq\|nats\|pubsub" --include="*.rs" -i
```
Results: only `scheduled_at` timestamp-based delay logic. No trigger-as-concept.

### A12.2 Webhook

Not implemented.

### A12.3 Schedule

RunnerQ implements **delayed execution**, not a cron/recurring schedule. An activity can be given a `delay_seconds` that sets `scheduled_at = Utc::now() + delay`. This is a one-shot future execution, not a repeating schedule. No cron syntax, no timezone, no DST handling, no distributed double-fire prevention beyond the normal dequeue mutex.

**Implementation detail:** For Redis, a separate `Scheduled Processor` polls a sorted set every N seconds via a Lua script. For PostgreSQL, `dequeue()` natively picks up activities where `scheduled_at <= NOW()` using the `idx_runnerq_dequeue_effective` partial index (`src/storage/postgres/mod.rs:1616`), so no separate processor is needed.

### A12.4 External event

Not implemented. No broker integration.

### A12.5 Reactive vs polling

**Pull-based / polling.** Each worker loop calls `dequeue()` in a tight loop with exponential backoff (`src/runner/runner.rs:419`). No push model, no event subscription, no callback from the storage when a new item arrives.

### A12.6 Trigger-to-workflow dispatch

Not applicable — no trigger model.

### A12.7 Trigger as Action

Not applicable. RunnerQ has no concept of a "trigger kind."

### A12.8 vs Nebula

Nebula has a 2-stage Source → Event → TriggerAction model where a TriggerAction normalizes raw inbound (HTTP request, Kafka message, cron tick) into a typed Event, and then a workflow is dispatched from that Event. RunnerQ has none of this — activities must be enqueued imperatively by application code. Nebula is architecturally richer here; RunnerQ trades trigger sophistication for simplicity.

---

## 12. Multi-Tenancy [A14]

RunnerQ provides **queue-name namespacing** as its only isolation mechanism. All SQL queries include `WHERE queue_name = $N` (`src/storage/postgres/mod.rs:479`). Multiple applications or environments can share the same PostgreSQL instance/database by using different queue names.

No RBAC, no SSO, no schema-per-tenant, no RLS, no SCIM, no tenant isolation guarantees. The `queue_name` is purely an application-level prefix — there is no access control preventing one application from reading another's queue.

**Comparison with Nebula:** Nebula has `nebula-tenant` with three isolation modes (schema/RLS/database), RBAC, and planned SSO/SCIM. RunnerQ's "tenancy" is just a naming convention.

---

## 13. Observability [A15]

RunnerQ uses the `tracing` crate for structured logging (`src/runner/runner.rs:21`: `use tracing::{debug, error, info, warn}`). No OpenTelemetry integration, no distributed tracing spans, no trace context propagation.

**Metrics:** The `MetricsSink` trait (`src/runner/runner.rs:64`) is a simple counter + histogram interface. Metric names are constants in `src/runner/metrics.rs` (24 constants covering activity lifecycle, worker lifecycle, scheduler/reaper, duration histograms). The default is `NoopMetrics`. Prometheus integration requires the user to implement `MetricsSink`.

**Built-in UI:** A web-based observability console is embedded (HTML/JS served from `src/observability/ui/html.rs`). It provides:
- Real-time statistics via REST API
- Activity browser by status
- Event timeline per activity
- SSE event stream

SSE for the PostgreSQL backend uses `LISTEN/NOTIFY` (`src/storage/postgres/mod.rs:1373`); for Redis, Redis Streams (`XREAD`).

**Comparison with Nebula:** Nebula uses OpenTelemetry with per-execution trace context and structured spans per action. RunnerQ uses `tracing` macros for structured logs but no distributed trace context. RunnerQ's built-in UI is more polished than Nebula's (which has no bundled UI as of now).

---

## 14. API Surface [A16]

RunnerQ is a **library**, not a server. It exposes:
- A Rust API: `WorkerEngine`, `ActivityHandler` trait, `ActivityExecutor` trait, `Storage` trait
- Optional Axum HTTP endpoints (feature-gated `axum-ui`):
  - `GET /stats` — queue statistics
  - `GET /activities/:status` — list activities by status
  - `GET /activities/:id/events` — event timeline
  - `GET /activities/:id/result` — activity result
  - `GET /dead-letter` — dead letter queue
  - `GET /stream` — SSE event stream (`src/observability/ui/axum.rs`)

No REST API for enqueueing (only programmatic via Rust API), no gRPC, no GraphQL, no OpenAPI spec. The observability API is purely for monitoring, not for control.

**Comparison with Nebula:** Nebula exposes a REST API for workflow management + plans for GraphQL/gRPC with OpenAPI spec generation. RunnerQ's HTTP surface is observability-only.

---

## 15. Testing Infrastructure [A19]

No dedicated testing crate. No public testing utilities or contract tests. Integration tests use standard `#[tokio::test]` in `tests/` (not present in the shallow clone but referenced in CI workflows). Examples in `examples/` serve as integration demonstration.

The project does not expose a `mock_backend` or `in-memory backend` for testing — users must provide their own PostgreSQL or Redis instance.

**Comparison with Nebula:** Nebula has `nebula-testing` crate with contract tests for resource implementors, `insta` snapshot tests, and `wiremock`/`mockall` integrations.

---

## 16. AI / LLM Integration [A21] — DEEP

### A21.1–A21.13

**No AI/LLM integration exists.** Searched across all Rust source files:

```
grep -rn "openai\|anthropic\|llm\|embedding\|completion\|gpt\|claude\|ai\|machine.learning" --include="*.rs" -i
```

Results: no matches related to LLM/AI. The word "ai" appears only in a Redis ZADD command name in the Redis backend.

RunnerQ is a pure background-job library with no AI-specific abstractions, no provider integration, no prompt management, no structured output validation, no tool calling, no streaming LLM support, no RAG, no memory, no cost tracking, and no safety layer.

**A21.13 vs Nebula:** Nebula has no first-class LLM either (strategic bet: AI = generic actions + plugin LLM client). Both projects are in the same position on this axis. RunnerQ's generic `ActivityHandler` with `serde_json::Value` I/O could trivially host an LLM call, just as Nebula's `ProcessAction` could.

---

## 17. Notable Design Decisions

**Decision 1: Pluggable backend via trait instead of feature flags per backend**
The `Storage` trait (`src/storage/traits.rs:375`) allows any type implementing `QueueStorage + InspectionStorage` to be a backend. Redis was originally built-in, then extracted to `runner_q_redis` (PR #73) to avoid mandatory Redis dependency. The PostgreSQL backend is now the default. This clean separation allows future backends (MongoDB, Kafka — open issues #64, #58) without touching the core. **Trade-off:** Adds an adapter layer (`BackendQueueAdapter`); complicates the internal type map between `Activity` and `QueuedActivity`.

**Decision 2: `schedules_natively()` flag to eliminate polling for capable backends**
The `QueueStorage::schedules_natively()` method (`src/storage/traits.rs:295`) lets a backend skip the scheduled-activities polling loop. PostgreSQL returns `true` because its `dequeue()` query already picks up due scheduled activities via `COALESCE(scheduled_at, created_at) <= NOW()`. Redis returns `false` because it needs the separate Lua-based polling loop. **Trade-off:** Complicates the startup code but eliminates unnecessary polling overhead for PostgreSQL.

**Decision 3: Age-weighted fair scheduling in PostgreSQL dequeue**
The dequeue query uses a three-tier `ORDER BY`: priority DESC, retry_count DESC (starvation avoidance for retrying activities), COALESCE(scheduled_at, created_at) ASC (FIFO within tier). This ensures high-retry activities are not starved when new work constantly arrives. **Trade-off:** Requires a composite index covering all three columns (`idx_runnerq_dequeue_effective`); index maintenance adds write overhead.

**Decision 4: `ActivityFuture` polling at 100ms**
`ActivityFuture::get_result()` spins with a `tokio::time::sleep(100ms)` poll loop (`src/activity/activity.rs:461`). This is simple but wasteful for long-running activities. No push notification from storage to the waiting caller. **Trade-off:** Easy to implement; creates O(activity_duration / 100ms) query load per waiting caller. The PostgreSQL `LISTEN/NOTIFY` infrastructure exists for the observability SSE stream but is not used for `ActivityFuture` polling — a missed optimization.

**Decision 5: Idempotency behaviors as enum**
`OnDuplicate` has four values (AllowReuse, ReturnExisting, AllowReuseOnFailure, NoReuse) (`src/activity/activity.rs:83`). This is a clean, explicit API for deduplication semantics. **Applicability to Nebula:** Nebula's trigger-level idempotency could benefit from a similar enum rather than a boolean flag.

---

## 18. Known Limitations / Pain Points

**Issue #70 (closed): "Document scale limitations: scheduler–executor coupling, single queue, storage bottleneck"**
The single `runnerq_activities` table is a known bottleneck. There is no partitioning, no shard-per-queue-name option. At very high throughput, all workers compete for the same table.

**Issue #67 (closed): "Queue starvation: retrying activities never complete when queue is busy"**
Activities in the `retrying` state were not picked up when the queue was saturated with new `pending` work. Fixed by the age-weighted scheduling in commit `7b2224d` (feat: implement age-weighted fair scheduling). The `retry_count DESC` tiebreaker ensures retrying activities eventually get processed.

**Issue #36 (closed): "Event stream uses in-memory channel, breaking cross-process event visibility"**
The original SSE event stream used an in-memory Tokio channel, so multiple app instances each had their own isolated event streams. Fixed by switching to PostgreSQL LISTEN/NOTIFY and Redis Streams for cross-process propagation (commit `9e34fd9`).

**Issue #33 (closed): "Fix duplicate reaper processing in multi-node deployments"**
Multiple worker instances running simultaneously could each invoke the reaper, causing double-requeue of the same expired activity. Fixed via `FOR UPDATE SKIP LOCKED` in the reaper query.

**Issue #72 (open): "Redis backend: apply Postgres scale improvements (scheduling, activity type filtering)"**
The Redis backend does not yet support activity type filtering (it accepts the `activity_types` parameter in `dequeue()` but ignores it). This means workload isolation only works with the PostgreSQL backend.

**Issue #64 (open): "feat: Implement MongoDBBackend"**
Community request; not planned by maintainer.

**Issue #19 (open): "Retry forever by default and not 3 times"**
Default `max_retries=3` is considered too aggressive by some users. No resolution yet.

---

## 19. Bus Factor / Sustainability

- **Maintainer count:** 1 (Akinlua Bolamigbe)
- **Repository age:** Created 2025-08-24 (approximately 8 months old as of April 2026)
- **Commit velocity:** 77 commits in ~8 months; active (latest commit 2026-04-15 is ~11 days ago)
- **Stars:** 26; **Forks:** 4 — small community
- **crates.io:** Published and versioned; v0.6.4 indicates active iteration
- **Bus factor:** 1 — sole maintainer; no governance document, no CODEOWNERS
- **Sustainability risk:** High single-point-of-failure. The library is MIT-licensed so forks are possible but fragmentation is likely if the maintainer steps back.
- **Open/Closed ratio:** 8 open / ~18 closed — healthy (31% open)
- **Issues quality:** Well-labeled with `enhancement`, `backend`, `metrics`, `observability` labels; issues are specific and actionable

---

## 20. Final Scorecard vs Nebula

| Axis | RunnerQ approach | Nebula approach | Assessment | Borrow? |
|------|-----------------|-----------------|------------|---------|
| **A1 Workspace** | 2 crates, feature-gated postgres + axum-ui; Edition 2021 | 26 crates layered; Edition 2024 | Nebula deeper; RunnerQ simpler (appropriate for library scope) | no — different goals |
| **A2 DAG** | No DAG. Flat queue; activities can enqueue sub-activities but no tracked graph | TypeDAG L1-L4 (generics → TypeId → predicates → petgraph) | Nebula deeper — RunnerQ has no graph concept | no — different goals |
| **A3 Action** | 1 kind: `ActivityHandler` with `serde_json::Value` I/O; open trait; no associated types; no versioning | 5 action kinds (Process/Supply/Trigger/Event/Schedule); sealed trait; assoc Input/Output/Error; derive macros | Nebula richer — RunnerQ simpler, no compile-time type safety | refine — the `on_dead_letter` callback and idempotency enum are worth adopting |
| **A4 Credential** | None. Application injects secrets via constructor | State/Material split, LiveCredential, blue-green refresh, OAuth2Protocol | Nebula much deeper; RunnerQ has no credential layer | no — different scope |
| **A5 Resource** | None. PgPool embedded in backend struct; no lifecycle abstraction | 4 scope levels, ReloadOutcome, generation tracking, on_credential_refresh | Nebula much deeper; RunnerQ has no resource layer | no — different scope |
| **A6 Resilience** | Retry + exponential backoff + timeout + DLQ + panic recovery; no CB/bulkhead/hedging | retry/CB/bulkhead/timeout/hedging; unified ErrorClassifier | Competitor simpler, Nebula richer; RunnerQ's per-activity retry config at dispatch time is ergonomic | refine — the explicit Retry/NonRetry binary on handler return type is clean DX to consider |
| **A7 Expression** | None | 60+ funcs, type inference, sandboxed eval, `$nodes.foo.result.email` | Nebula deeper; RunnerQ does not need expressions (no DAG routing) | no — different goals |
| **A8 Storage** | sqlx + PgPool (PostgreSQL); bb8-redis (Redis); `FOR UPDATE SKIP LOCKED`; permanent records | sqlx + PgPool, Pg*Repo per aggregate, SQL migrations, RLS | Convergent at the sqlx/PgPool level; RunnerQ lacks RLS/migrations framework; Nebula has richer per-aggregate repo pattern | maybe — RunnerQ's `schedules_natively()` flag is a clean abstraction |
| **A9 Persistence** | Lease-based at-least-once; row mutation in place; `runnerq_events` append-only log | Frontier + checkpoint + append-only log; replay-based recovery | Different decomposition; Nebula supports deterministic replay; RunnerQ is simpler crash-recovery | no — Nebula's model is more correct for workflow semantics |
| **A10 Concurrency** | tokio; N dedicated worker loops (no semaphore); exponential backoff on idle; `catch_unwind` panic safety | tokio; frontier scheduler; `!Send` action support via thread-local | Different decomposition; RunnerQ simpler (flat pool); Nebula richer (frontier per-DAG-node) | refine — the `catch_unwind` panic-to-retry conversion is worth adopting in Nebula |
| **A11 Plugin BUILD** | None — no plugin system; `Storage` trait as the only extension point | WASM, plugin-v2 spec, Plugin Fund | Nebula deeper; RunnerQ has no plugin concept | no — different goals |
| **A11 Plugin EXEC** | None | WASM sandbox + capability security | Nebula deeper; RunnerQ has no sandbox | no — different goals |
| **A12 Trigger** | No trigger model. Pull-based imperative enqueue only; time-delay for one-shot future execution | TriggerAction with Input=Config, Output=Event; Source normalizes raw inbound; 2-stage | Nebula deeper; RunnerQ has no trigger/event abstraction | no — different goals |
| **A13 Deployment** | Library embedded in user binary; no standalone server mode | 3 modes from one binary: desktop/serve/cloud | Different decomposition — RunnerQ is a library, Nebula is a platform | no — different goals |
| **A14 Multi-tenancy** | Queue name namespacing only (no RBAC, no isolation guarantee) | nebula-tenant: schema/RLS/database isolation, RBAC, SSO | Nebula much deeper | no — different scope |
| **A15 Observability** | `tracing` macros + `MetricsSink` trait + built-in SSE dashboard; no OTel | OpenTelemetry per-execution trace; structured spans | Different approach; RunnerQ built-in UI is polished; Nebula has richer distributed tracing | refine — RunnerQ's built-in dashboard concept is worth considering for Nebula |
| **A16 API** | Library API + optional Axum observability-only HTTP; no control plane API | REST + planned GraphQL/gRPC; OpenAPI spec; OwnerId-aware | Nebula richer | no — different goals |
| **A17 Type safety** | Open trait, `serde_json::Value` I/O, no GATs/HRTBs/typestate | Sealed traits, GATs, HRTBs, typestate, Validated<T> | Nebula much deeper; RunnerQ prioritizes simplicity over type safety | no — different design philosophy |
| **A18 Errors** | `ActivityError` (Retry/NonRetry), `WorkerError` (13 variants), `StorageError`; `thiserror`; no class enum | nebula-error + ErrorClass enum + ErrorClassifier | Competitor simpler; Nebula richer; RunnerQ's Retry/NonRetry binary is simpler DX | refine — explicit Retry/NonRetry handler return type is cleaner than Nebula's anyhow-style approach for the "retryable?" question |
| **A19 Testing** | No testing crate; no public mock backend; examples as integration tests | nebula-testing crate, contract tests, insta/wiremock/mockall | Nebula deeper | yes — RunnerQ should add in-memory mock backend; Nebula has this right |
| **A20 Governance** | MIT, solo maintainer, no commercial model | Open core, Plugin Fund, planned SOC 2, solo maintainer | Both solo; Nebula has commercial story | no |
| **A21 AI/LLM** | None — no LLM integration | None yet — generic actions + plugin LLM client | Convergent — neither has first-class LLM | no |
