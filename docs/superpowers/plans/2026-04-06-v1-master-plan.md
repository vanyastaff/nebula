# Nebula v1.0 Master Implementation Plan

> **For agentic workers:** This is a sequencing document, not an implementation plan. Each phase references a dedicated plan with task-level detail. Use superpowers:subagent-driven-development or superpowers:executing-plans to execute individual phase plans.

**Goal:** Ship a production-ready workflow automation engine that can run a real workflow with credentials, persist state, survive crashes, and expose a REST API.

**Scope:** 22 design specs synthesized into 16 ordered phases. v1.0 critical path (Phases 0-7) delivers "first workflow end-to-end." Phases 8-16 complete v1.0 surface area. v1.1+ deferred.

**Current state:** 159K LOC across 25 crates. ~900 tests. Expression (8.8K LOC, 313 tests) and credential (18K LOC, 329 tests) are production-grade. Storage (1.4K, 0 tests) and runtime (1.2K, 15 tests) are skeletal. Engine (2.1K, 23 tests) works for in-memory but has no persistence or credential DI.

---

## Dependency Graph

```
Phase 0: Critical Fixes ──────────────────────────────────┐
Phase 1: Parameter v4 Internal Quality ───────────────────┤
                                                          ▼
Phase 2: Storage v1 (PgExecutionRepo) ◄── v1 BLOCKER #1 ─┤
                                                          │
Phase 3: Workflow v2 (OwnerId, schema version) ───────────┤
                                                          ▼
Phase 4: Engine v1 (persistence + credential DI) ◄── v1 BLOCKER #2
    ├── needs Phase 2 (PgExecutionRepo)                   │
    ├── needs credential crate stable                     │
    └── needs expression crate stable                     ▼
Phase 5: Expression v1 (missing functions + security) ────┤
                                                          │
Phase 6: Action v2 (derive macro + keyed access) ────────┤
Phase 7: Runtime v2 (sandbox + SpillToBlob) ──────────────┘
    └── needs Phase 6 (ActionMetadata.isolation_level)
                                                          
─── v1.0 CRITICAL PATH COMPLETE (Phases 0-7) ───

Phase 8: Credential v3 (open AuthScheme + security) ─────┐
Phase 9: Resource v2 (rotation + error classification) ──┤ can parallel
Phase 10: Plugin v2 (lifecycle + manifest) ───────────────┤
                                                          ▼
Phase 11: Webhook v2 (durable queue + verification) ──────┤
    └── needs Phase 2 (QueueBackend)                     │
Phase 12: API v1 (endpoints + rate limiting) ─────────────┤
    └── needs Phase 2 (ExecutionRepo.list)               │
Phase 13: Serialization (Arc<RawValue>, rmp-serde) ──────┤
Phase 14: Telemetry (profiles + USDT + Sentry) ──────────┤
Phase 15: Security hardening (red team fixes) ────────────┘

Phase 16: Integration testing + CLI v1 (after all above)
```

---

## Phase Summary

| Phase | Name | Spec Source | Crates | Est. LOC | Priority | Status |
|-------|------|-------------|--------|----------|----------|--------|
| **0** | Critical Fixes | cross-spec audit | expression, workflow, execution, core | ~200 | P0 | **Plan exists** |
| **1** | Parameter v4 Internal | parameter-v4 §7 | parameter | ~300 | P0 | **Plan exists** |
| **2** | Storage v1 | storage-v1 | storage | ~2000 | P0 | Needs plan |
| **3** | Workflow v2 | workflow-v2 | workflow | ~400 | P0 | Needs plan |
| **4** | Engine v1 | engine-v1 | engine | ~1500 | P0 | Needs plan |
| **5** | Expression v1 | expression-v1 | expression | ~800 | P0 | Needs plan |
| **6** | Action v2 | action-v2 | action, action/macros | ~1500 | P1 | Needs plan |
| **7** | Runtime v2 | runtime-v2 | runtime | ~600 | P1 | Needs plan |
| **8** | Credential v3 | credential-v3 | credential, credential/macros | ~2000 | P1 | Needs plan |
| **9** | Resource v2 | resource-v2 | resource | ~800 | P1 | Needs plan |
| **10** | Plugin v2 | plugin-v2 | plugin | ~500 | P2 | Needs plan |
| **11** | Webhook v2 | webhook-v2 | webhook | ~800 | P1 | Needs plan |
| **12** | API v1 | api-v1 | api | ~1200 | P1 | Needs plan |
| **13** | Serialization | serialization | engine, expression, storage | ~600 | P2 | Needs plan |
| **14** | Telemetry | telemetry | telemetry, log | ~800 | P2 | Needs plan |
| **15** | Security Hardening | red-team | credential, expression, webhook | ~400 | P1 | Needs plan |
| **16** | CLI + Integration | cli, deployment-modes | apps/cli, apps/server | ~1500 | P2 | Needs plan |

**Total estimated new LOC for v1.0:** ~15,600

---

## Phase Details

### Phase 0: Critical Fixes (5 tasks, ~200 LOC)

**Plan:** `docs/superpowers/plans/2026-04-06-phase0-critical-fixes.md` (EXISTS)

**What:** 5 independent bug fixes in core/cross-cutting crates.

| Task | Crate | Issue | Breaking? |
|------|-------|-------|-----------|
| 1 | expression | `Box::leak` memory leak in lexer string escapes | Yes (TokenKind loses Copy) |
| 2 | expression | Cache error erasure (`get_or_compute` → `get`+`insert`) | No |
| 3 | workflow | `NodeDefinition::new` panics on invalid ActionKey | Yes (returns Result) |
| 4 | execution | Missing `Cancelling → Completed/TimedOut` transitions | No |
| 5 | core | `CredentialEvent` uses `String` instead of `CredentialId` | Yes |

**Blockers:** None. All independent.
**Unblocks:** Phases 4, 5 (clean engine/expression foundation).

---

### Phase 1: Parameter v4 Internal Quality (7 tasks, ~300 LOC)

**Plan:** `docs/superpowers/plans/2026-04-06-parameter-v4-internal-quality.md` (EXISTS)

**What:** Internal refactors without public API changes. Foundation for Phase B (builder API) and Phase C (derive macro rewrite).

| Task | Issue | Impact |
|------|-------|--------|
| 1 | error.rs category/code duplication | Consistency |
| 2 | Regex cache in transformer | Performance |
| 3 | Condition accepts ParameterValues | API cleanliness |
| 4 | Generic Loader\<T\> | -100 LOC dedup |
| 5 | Debug-based variant name → explicit method | Correctness |
| 6 | Reduce validate.rs allocations | Performance |
| 7 | InputHint + deprecate Date/Time/Color/Hidden | API consolidation |

**Blockers:** None.
**Unblocks:** Parameter v4 Phase B (builder API, v1.1), Phase C (derive macro, v1.1).

---

### Phase 2: Storage v1 — PgExecutionRepo (v1 BLOCKER)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-storage-v1.md`

**Spec:** `docs/superpowers/specs/2026-04-06-storage-v1-design.md`

**What:** The single biggest v1 blocker. Engine cannot persist execution state without this.

| Task | What | Schema/Code |
|------|------|-------------|
| 1 | SQL migrations: `executions`, `node_outputs`, `execution_journal`, `idempotency_keys` | 4 migration files |
| 2 | `NodeOutputRecord` type + serialization | New type in execution or storage |
| 3 | `ExecutionFilter` + `ExecutionSummary` types | New types in storage |
| 4 | `JournalEntry` type | New type in storage |
| 5 | `PgExecutionRepo` — `create`, `save_state`, `load_state`, `transition` | CAS via version column |
| 6 | `PgExecutionRepo` — `save_node_output`, `load_node_output`, `load_all_outputs` | BYTEA column (rmp-serde) |
| 7 | `PgExecutionRepo` — `list`, `list_running`, `count` | ExecutionFilter queries |
| 8 | `PgExecutionRepo` — `check_idempotency`, `mark_idempotent` | idempotency_keys table |
| 9 | `PgExecutionRepo` — `append_journal`, `load_journal` | execution_journal table |
| 10 | `QueueBackend` trait + `PgQueue` impl | `task_queue` table, `SELECT ... FOR UPDATE SKIP LOCKED` |
| 11 | Row-Level Security setup | `SET LOCAL app.current_owner` per transaction (RT-10 fix) |
| 12 | `ExecutionRepo` trait — add missing methods to trait def | Trait in storage/execution |
| 13 | Integration tests with real Postgres (`task db:up`) | Docker Compose test env |
| 14 | sqlx offline data regeneration (`task db:prepare`) | CI compatibility |

**Key decisions:**
- `node_outputs.output` is `BYTEA` (MessagePack via rmp-serde), not JSONB — 30-50% smaller, 2x faster
- `StorageFormat` enum handles legacy JSON reads + new MessagePack writes
- CAS (Compare-And-Swap) via `version` column on `executions` table — same pattern as `PgWorkflowRepo`
- `SET LOCAL app.current_owner` per transaction, NOT per connection (RT-10 red team fix)

**Blockers:** None (Postgres infra exists via `task db:up`).
**Unblocks:** Phase 4 (engine persistence), Phase 11 (webhook queue), Phase 12 (API execution list).

**New dependencies:** `rmp-serde` (workspace, feature-gated `msgpack-storage`).

---

### Phase 3: Workflow v2 (4 tasks, ~400 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-workflow-v2.md`

**Spec:** `docs/superpowers/specs/2026-04-06-workflow-v2-design.md`

**What:** Add `owner_id`, `ui_metadata`, `schema_version` to WorkflowDefinition. Improve validation.

| Task | What |
|------|------|
| 1 | `OwnerId` newtype in nebula-core (W1 — required, not optional) |
| 2 | `UiMetadata`, `NodePosition`, `Viewport`, `Annotation` types |
| 3 | `WorkflowDefinition` gains `owner_id: OwnerId`, `ui_metadata`, `schema_version` |
| 4 | Trigger validation (cron syntax, webhook path), new error variants |
| 5 | `PartialEq` derive on WorkflowDefinition + config types |
| 6 | Schema snapshot test (serialize/deserialize fixture) |
| 7 | WorkflowBuilder additions: `.owner()`, `.trigger()`, `.ui_metadata()` |

**Key decisions:**
- `owner_id` is `OwnerId` (required newtype), NOT `Option<String>` — per W1 conference feedback
- `UiMetadata` is opaque to engine — only desktop/web reads it
- `schema_version: u32` with `is_schema_supported()` check

**Blockers:** Phase 0 Task 3 (NodeDefinition::new returns Result — touches workflow crate).
**Unblocks:** Phase 4 (engine needs OwnerId for RLS), Phase 12 (API filtering by owner).

---

### Phase 4: Engine v1 — Persistence + Credential DI (v1 BLOCKER)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-engine-v1.md`

**Spec:** `docs/superpowers/specs/2026-04-06-engine-v1-design.md`

**What:** Wire storage persistence, credential resolution, budget enforcement, and crash recovery into the existing frontier-based engine.

| Task | What | Depends on |
|------|------|------------|
| 1 | Add `workflow_repo` + `execution_repo` to `WorkflowEngine` | Phase 2 |
| 2 | Checkpoint contract: save node output + state after each node | Phase 2 |
| 3 | `EngineCredentialAccessor` — wires CredentialResolver into ActionContext | credential crate |
| 4 | `EngineResourceAccessor` — wires ResourceManager into ActionContext | resource crate |
| 5 | `ExecutionTracker` — budget enforcement (duration, bytes, retries) | — |
| 6 | Error strategy enforcement (FailFast/ContinueOnError/IgnoreErrors) | — |
| 7 | `NodeDefinition.enabled` check — skip disabled nodes | — |
| 8 | Durable idempotency via `ExecutionRepo.check/mark_idempotent` | Phase 2 |
| 9 | `resume_execution()` — crash recovery from checkpoint | Phase 2 |
| 10 | Proactive credential refresh before node dispatch (C5) | Task 3 |
| 11 | Action version pinning from `NodeDefinition.action_version` | — |
| 12 | Integration tests: persist + crash + resume cycle | Phase 2 |

**Key decisions:**
- Checkpoint after EACH node (mandatory) — not batched, not optional
- `EngineCredentialAccessor` validates key is declared in action dependencies before resolving
- Budget checks happen BEFORE dispatching next node, not during execution
- Resume builds "ready queue" from non-terminal nodes whose predecessors are all terminal

**Blockers:** Phase 2 (PgExecutionRepo), Phase 0 (clean foundation).
**Unblocks:** Everything downstream — this is the engine MVP.

---

### Phase 5: Expression v1 Completion (3 tasks, ~800 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-expression-v1.md`

**Spec:** `docs/superpowers/specs/2026-04-06-expression-v1-design.md`

**What:** Add missing functions, memory budget, value redaction in errors.

| Task | What |
|------|------|
| 1 | Add 12 missing array/object functions: `some`, `every`, `find`, `find_index`, `unique`, `group_by`, `flat_map`, `merge`, `pick`, `omit`, `entries`, `from_entries` |
| 2 | Add 5 missing string/utility functions: `pad_start`, `pad_end`, `repeat`, `coalesce`, `type_of` |
| 3 | RT-1: Memory budget per evaluation (`max_eval_memory_bytes` on `EvaluationPolicy`) |
| 4 | RT-2: Value redaction in error messages (`redact_value()` shows shape not content) |

**Key decisions:**
- Memory budget via periodic allocation tracking, not per-op allocator hooks (too expensive)
- `redact_value()` returns `"Object{token: String, expires: Number}"` — structure only
- Functions follow existing pattern in `builtins.rs`

**Blockers:** Phase 0 Tasks 1-2 (expression crate fixes).
**Unblocks:** Phase 4 (engine uses expression evaluation).

---

### Phase 6: Action v2 — Derive Macro + Keyed Access (6 tasks, ~1500 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-action-v2.md`

**Spec:** `docs/superpowers/specs/2026-04-06-action-v2-design.md`

**What:** `#[derive(Action)]` generates metadata + dependencies. Keyed credential access on ActionContext.

| Task | What |
|------|------|
| 1 | `ActionDependencies` trait + `CredentialKey`/`ResourceKey` declarations |
| 2 | `#[derive(Action)]` macro rewrite — generates Action + ActionDependencies |
| 3 | `ActionContext::credential::<S>(key)` + `credential_opt::<S>(key)` |
| 4 | `ActionContext::resource::<R>(key)` + `resource_opt::<R>(key)` |
| 5 | `ActionRegistry` version-aware (`VersionedActionKey`, `get_versioned`, `get_latest`) |
| 6 | `TestContextBuilder` + `SpyLogger` + assertion macros |
| 7 | `ActionMetadata` gains `dependencies: ActionDependencies` field |
| 8 | `IsolationLevel` enum on `ActionMetadata` (for Phase 7) |

**Blockers:** Phase 1 (parameter internals — Action uses HasParameters).
**Unblocks:** Phase 4 Task 3 (EngineCredentialAccessor), Phase 7 (sandbox routing).

---

### Phase 7: Runtime v2 — Sandbox + SpillToBlob (4 tasks, ~600 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-runtime-v2.md`

**Spec:** `docs/superpowers/specs/2026-04-06-runtime-v2-design.md` + `sandbox-design.md`

**What:** Activate sandbox routing by IsolationLevel, implement SpillToBlob.

| Task | What |
|------|------|
| 1 | `SandboxedContext` — wraps ActionContext with capability checks |
| 2 | `InProcessSandbox` — routes by `IsolationLevel` from `ActionMetadata` |
| 3 | `BlobStorage` trait + `BlobRef` type |
| 4 | `SpillToBlob` — enforce data limit with actual blob write (replace warning-only) |
| 5 | `ActionRegistry` version-aware integration (from action v2) |

**Blockers:** Phase 6 (ActionMetadata.isolation_level).
**Unblocks:** Production safety — actions can't accidentally access undeclared credentials.

---

### Phase 8: Credential v3 — Open AuthScheme + Security (8 tasks, ~2000 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-credential-v3.md`

**Spec:** `docs/superpowers/specs/2026-04-06-credential-v3-design.md`

**What:** Open AuthScheme trait (12 universal patterns), encryption key rotation, AAD hardening, Zeroizing buffers.

| Task | What | Breaking? |
|------|------|-----------|
| 1 | `AuthScheme` trait (open, replaces closed enum) | Yes |
| 2 | `AuthPattern` classification enum (13 variants) | No |
| 3 | 12 built-in scheme types (SecretToken, IdentityPassword, OAuth2Token, etc.) | Yes |
| 4 | `EncryptedData` gains `key_id` + multi-key `EncryptionLayer` | Migration |
| 5 | Remove AAD legacy fallback — hard cutover + migration tool | Breaking |
| 6 | `Zeroizing<Vec<u8>>` for all plaintext buffers | No |
| 7 | `#[derive(Credential)]` rewrite with `#[credential(into = "field")]` | Yes |
| 8 | `CredentialKey` (type) + `CredentialId` (instance) dual identity | Clarification |

**Blockers:** None (credential crate is self-contained).
**Unblocks:** Phase 4 Task 3 (EngineCredentialAccessor uses resolved AuthScheme).

**Note:** Can run in parallel with Phases 2-5 since credential is isolated.

---

### Phase 9: Resource v2 — Rotation + Error Classification (5 tasks, ~800 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-resource-v2.md`

**Spec:** `docs/superpowers/specs/2026-04-06-resource-v2-design.md`

**What:** Restore credential rotation, typed callbacks, error classification, test-connection.

| Task | What |
|------|------|
| 1 | `ResourceError` gains `AuthFailed`, `ConnectionFailed`, `CredentialNotConfigured`, `MissingCredential` |
| 2 | `RotationStrategy` enum (HotSwap, DrainAndRecreate, Reconnect) |
| 3 | Typed `AuthorizeCallback<R>` (receives `&R::Auth`, not `&Value`) |
| 4 | `test_resource()` with `TestOptions` + error sanitization |
| 5 | `ResourceDependencies` trait (`credential_keys()`, `rotation_strategy()`) |
| 6 | `Manager::register_with_credential()` + `spawn_rotation_listener()` |
| 7 | Rotation event validation (known CredentialId, size limits, fail-closed) |

**Blockers:** Phase 8 (AuthScheme types for `R::Auth`).
**Unblocks:** Phase 4 Task 4 (EngineResourceAccessor).

---

### Phase 10: Plugin v2 — Lifecycle + Manifest (4 tasks, ~500 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-plugin-v2.md`

**Spec:** `docs/superpowers/specs/2026-04-06-plugin-v2-design.md`

**What:** Plugin trait gains lifecycle hooks, component descriptors, manifest format.

| Task | What |
|------|------|
| 1 | `Plugin` trait enhancement: `actions()`, `credentials()`, `resources()`, `data_tags()`, `on_load()`, `on_unload()` |
| 2 | `ActionDescriptor`, `CredentialDescriptor`, `ResourceDescriptor` types |
| 3 | `PluginMetadata` extended (author, license, homepage, nebula_version) |
| 4 | `nebula-plugin.toml` manifest parsing |
| 5 | `#[derive(Plugin)]` macro |

**Blockers:** Phase 6 (ActionDescriptor references InternalHandler from action v2).
**Unblocks:** Phase 12 (API plugin catalog endpoints).

---

### Phase 11: Webhook v2 — Durable Queue + Verification (5 tasks, ~800 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-webhook-v2.md`

**Spec:** `docs/superpowers/specs/2026-04-06-webhook-v2-design.md`

**What:** Write events to queue before ack, signature verification framework, outbound delivery.

| Task | What |
|------|------|
| 1 | `WebhookVerifier` trait + `HmacSha256Verifier` + `TimestampHmacVerifier` |
| 2 | Durable inbound queue: write to `QueueBackend` before HTTP 200 ack |
| 3 | `WebhookDeliverer` — outbound webhook with retry |
| 4 | `WebhookRateLimiter` — per-path RPM limiting |
| 5 | Webhook metrics (8 counters) |

**Blockers:** Phase 2 Task 10 (QueueBackend).
**Unblocks:** Phase 12 (API webhook endpoints).

---

### Phase 12: API v1 — Endpoints + Rate Limiting (6 tasks, ~1200 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-api-v1.md`

**Spec:** `docs/superpowers/specs/2026-04-06-api-v1-design.md`

**What:** Fill endpoint gaps, credentials API, rate limiting, API key auth.

| Task | What |
|------|------|
| 1 | Fix execution list (`GET /executions` with ExecutionFilter) |
| 2 | Execution outputs endpoint (`GET /executions/{id}/outputs`) |
| 3 | Credentials CRUD (`GET/POST/PUT/DELETE /credentials`, `/test`, OAuth2 callback) |
| 4 | Action + Plugin catalog (`GET /actions`, `/actions/{key}`, `/plugins`) |
| 5 | Rate limiting middleware (in-memory sliding window, per-tenant) |
| 6 | API Key auth parallel to JWT (`X-API-Key: nbl_sk_...`) |
| 7 | Workflow validate endpoint (`POST /workflows/{id}/validate`) |

**Blockers:** Phase 2 (ExecutionRepo.list), Phase 8 (Credentials API), Phase 10 (Plugin catalog).
**Unblocks:** Phase 16 (CLI uses API as client).

---

### Phase 13: Serialization Optimization (5 tasks, ~600 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-serialization.md`

**Spec:** `docs/superpowers/specs/2026-04-06-serialization-strategy-design.md`

**What:** `Arc<RawValue>` for node-to-node passing, `rmp-serde` for storage, `simd-json` for ingest.

| Task | What | Feature gate |
|------|------|-------------|
| 1 | `NodeOutput` with `Arc<RawValue>` + lazy `OnceLock<Value>` in engine | None (internal) |
| 2 | Expression `EvaluationContext` uses `Arc<RawValue>` | None (internal) |
| 3 | `Bytes` for binary payloads in action/runtime | None (internal) |
| 4 | `rmp-serde` for execution state in PgExecutionRepo | `msgpack-storage` |
| 5 | `simd-json` for webhook ingest (x86_64 only) | `simd-json` |

**Blockers:** Phase 2 (storage), Phase 4 (engine node output flow).
**Unblocks:** Performance targets (RT6: <1ms for 3-node workflow).

---

### Phase 14: Telemetry — Profiles + Observability (5 tasks, ~800 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-telemetry-v2.md`

**Spec:** `docs/superpowers/specs/2026-04-06-telemetry-design.md`

**What:** Three export profiles, OTEL traces opt-in, Sentry integration, USDT probes.

| Task | What |
|------|------|
| 1 | `TelemetryProfile` enum (Local, SelfHosted, Cloud) |
| 2 | `LocalMetricsStore` + `LocalEventStore` (SQLite) for desktop |
| 3 | OTEL trace spans (opt-in via `otel` feature flag) |
| 4 | Sentry integration (opt-in local, off server, always cloud) |
| 5 | 6 USDT probes (action entry/return, checkpoint, credential, blob, resource) |
| 6 | Cardinality protection (10K limit per metric) |

**Blockers:** None (telemetry crate is cross-cutting).
**Unblocks:** Desktop app observability, production debugging.

---

### Phase 15: Security Hardening — Red Team Fixes (5 tasks, ~400 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-security-hardening.md`

**Spec:** `docs/superpowers/specs/2026-04-06-red-team-findings.md`

**What:** Address all High/Medium red team findings targeted at v1.

| ID | Severity | Fix | Phase dep |
|----|----------|-----|-----------|
| RT-3 | High | `Zeroizing<T>` on rkyv cache buffers | Phase 8 |
| RT-7 | High | `WebhookVerifier` trait | Phase 11 (done there) |
| RT-8 | High | EventBus `Origin` tags on rotation events | Phase 9 |
| RT-10 | High | `SET LOCAL` per-transaction RLS | Phase 2 (done there) |
| RT-1 | Medium | Expression memory budget | Phase 5 (done there) |
| RT-2 | Medium | Value redaction in expression errors | Phase 5 (done there) |
| RT-4 | Medium | Atomic `ScopeLayer::get()` (check + read) | Phase 8 |

**Note:** Most red team fixes are embedded in their respective phase plans. This phase covers the remaining ones and serves as a verification pass.

---

### Phase 16: CLI + Integration + Deployment (7 tasks, ~1500 LOC)

**Plan:** Needs writing → `docs/superpowers/plans/2026-04-06-cli-and-integration.md`

**Spec:** `docs/superpowers/specs/2026-04-06-cli-design.md` + `deployment-modes-design.md`

**What:** CLI binary, server binary, integration tests, deployment wiring.

| Task | What |
|------|------|
| 1 | `apps/cli/` — `nebula run` (local embedded engine + SQLite) |
| 2 | `apps/cli/` — `nebula validate` (workflow validation) |
| 3 | `apps/server/` — server binary with Postgres + API + Auth |
| 4 | CI: both desktop + server binaries must compile |
| 5 | End-to-end integration test: webhook → engine → action → checkpoint → resume |
| 6 | Performance benchmark: 3-node workflow <1ms (RT6) |
| 7 | Config files: `nebula-desktop.toml`, `nebula-server.toml` |

**Blockers:** All previous phases (this is the integration layer).

---

## Cross-Spec Consistency Fixes (from audit)

These are embedded in their respective phases:

| Fix | Where addressed |
|-----|----------------|
| R1. SandboxRunner not object-safe | Phase 7 (use `Pin<Box<dyn Future>>`) |
| R2. rkyv incompatible with `serde_json::Value` | Phase 13 (scope rkyv to metadata-only) |
| R3. nebula-plugin needs nebula-action dep | Phase 10 (add dependency) |
| Y1. OwnerId newtype everywhere | Phase 3 |
| Y5. Engine uses NodeOutput from serialization | Phase 13 |
| Y6. expression-v1 spec must exist | EXISTS |
| Y7. Engine refreshes by CredentialId | Phase 4 Task 10 |
| Y8. ActionMetadata gains dependencies | Phase 6 Task 7 |
| Y9. Plugin manifest gains isolation | Phase 10 |
| Y10. simd-json claim corrected | Phase 13 (1.3-1.5x, not 2-4x) |

---

## Parallelism Opportunities

These phase groups can execute concurrently:

**Track A (Critical Path):** Phase 0 → Phase 2 → Phase 4 → Phase 16

**Track B (Business Logic):** Phase 1 → Phase 6 → Phase 7

**Track C (Credential/Resource):** Phase 8 → Phase 9

**Track D (API Surface):** Phase 3 → Phase 12

**Track E (Expression):** Phase 5 (independent after Phase 0)

**Track F (Ecosystem):** Phase 10 → Phase 11

**Track G (Cross-cutting):** Phase 13, Phase 14 (after Phase 4)

Maximum parallelism: 4 tracks simultaneously after Phase 0 completes.

---

## v1.1 Deferred Items (NOT in this plan)

These are documented in specs but explicitly out of scope for v1.0:

| Item | Spec | Reason deferred |
|------|------|----------------|
| `#[derive(Parameters)]` rewrite | parameter-v4 §1 | Needs builder API first |
| Typed closure builders | parameter-v4 §2 | Public API change, needs design stabilization |
| Inline expression caching (10-50x) | breakthrough #1 | v1.1 optimization |
| Vectorized batch expression eval | breakthrough #6 | v1.1 optimization |
| `Arc<RawValue>` for node passing | serialization §2.1 | Phase 13 (late v1.0 or v1.1) |
| SQLite/libSQL storage backend | storage-v1 §3 | Desktop-specific, after Postgres |
| WebSocket execution stream | api-v1 §4 | v1.1 |
| OpenAPI generation (utoipa) | api-v1 §5 | v1.1 |
| `Engine::rerun_node()` | runtime-v2 RT15 | v1.1 |
| `--dry-run` execution mode | workflow-v2 round 9 | v1.1 |
| n8n migration tool | governance §7 | v1.1 |
| Per-tenant rate limiting | runtime-v2 RT9 | Needs rate limiter refactor |
| CachePolicy (input-hash caching) | workflow-v2 W6 | v1.1 |
| ClickHouse ExecutionHistoryWriter | telemetry §10 | v1.1 |
| OTEL traces (full) | telemetry §4 | v1.1 (opt-in) |
| Pyroscope profiling | telemetry §12 | v1.1 |
| AIMD adaptive rate limiting | breakthrough #4 | v1.1 |
| Deterministic simulation testing | breakthrough #3 | v1.1 |

---

## v2.0+ Deferred Items (far future)

| Item | Spec |
|------|------|
| WASM plugin loading | sandbox §4 |
| Firecracker microVM | sandbox §5, breakthrough #9 |
| Arena-based allocation | breakthrough #2 |
| Consistent hashing | breakthrough #5 |
| ForEach / dynamic fan-out | workflow-v2 W3 |
| Sub-workflows | workflow-v2 W3 |
| WaitForEvent / suspended execution | workflow-v2 W5 |
| Distributed scheduling | runtime-v2 RT11 |
| AgentAction + AgentContext | action-v2 §1 |
| Multi-language plugin SDKs | governance §6 |
| Redis/NATS queue backends | storage-v1 §2 |
| Plugin Hub page | ecosystem strategy |
| Auth subsystem (JWT, RBAC, SSO) | governance |
| Execution state encryption (PCI) | governance §5 |

---

## Success Criteria for v1.0

The engine can:

1. **Execute a real workflow** with 3+ nodes including HTTP request + data transform + Slack notification
2. **Resolve credentials** — OAuth2 token injection into HTTP action, API key into Slack
3. **Persist execution state** — Postgres backend, survives process restart
4. **Resume after crash** — incomplete execution recovers from checkpoint
5. **Enforce budgets** — max duration, max output bytes, max retries
6. **Handle errors** — FailFast/ContinueOnError/IgnoreErrors per workflow
7. **Expose REST API** — workflow CRUD, execution management, credential management
8. **Verify webhooks** — HMAC-SHA256 signature verification at framework level
9. **Rate limit** — per-tenant, per-provider request limiting
10. **Multi-tenant** — OwnerId + RLS enforced at database level

**Benchmark target:** 3-node workflow with 1KB payloads, no external I/O: **< 1ms end-to-end** (RT6).

**Test target:** 1500+ tests across all crates (current: ~900).
