# nebula-storage spec-16 port/adapter/tenancy redesign — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the dual Layer-1/Layer-2 architecture of `nebula-storage` with one object-safe spec-16 port (`nebula-storage-port`, Core), a multi-backend adapter (`nebula-storage`, Exec), and a tenancy security boundary (`nebula-tenancy`, Business), then rewire engine/api/core.

**Architecture:** Hexagonal port/adapter. The port defines ISP-segregated, `#[async_trait]`, object-safe repository traits + plain-data `Scope` + a `TransitionBatch` atomic unit-of-work whose `commit` writes state+outbox+journal in one transaction gated by CAS *and* a lease `FencingToken`. Adapters: InMemory, SQLite, Postgres — verified by one behavioral conformance matrix. Tenancy wraps the port as a scope-enforcing decorator (cross-tenant access ⇒ `NotFound`).

**Tech Stack:** Rust 1.95 (edition 2024), tokio, `async-trait`, `thiserror`, `sqlx` 0.8 (postgres+sqlite), `serde`/`serde_json`, `parking_lot`, `rstest`, `cargo-nextest`, `loom` (probe crate), `cargo-deny`.

**Spec:** `docs/superpowers/specs/2026-05-15-nebula-storage-spec16-redesign-design.md`. Breaking changes allowed.

**Global conventions (apply to every task):**
- No `unwrap`/`expect`/`panic!` in library code (tests/`const`/bins exempt per `clippy.toml`). Production paths return `StorageError`.
- Every new state/error/hot path: typed error variant + `tracing` span + invariant check.
- No plan IDs / "Phase X" / task IDs in committed code or comments.
- Conventional commits, convco-validated, scope = crate without `nebula-` prefix or top-level area. End commit body with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`.
- **Windows worktree:** verify per-crate — `cargo fmt -p <c>`, `cargo clippy -p <c> -- -D warnings`, `cargo nextest run -p <c>`. Do NOT report `task dev:check` green from this worktree (os error 206).
- Postgres-backed tests are `DATABASE_URL`-gated and skip cleanly when unset.
- No `src/*.rs` over ~600 lines; trait→port, InMemory impl→own module, backend impls split by concern, tests→`tests/`.

---

## File Structure (decomposition lock-in)

**New crate `crates/storage-port/` (`nebula-storage-port`, Core tier):**
- `Cargo.toml` — deps: `nebula-core`, `serde`, `serde_json`, `thiserror`, `async-trait`. No sqlx.
- `src/lib.rs` — module wiring + re-exports; `#![warn(missing_docs)]`.
- `src/error.rs` — `StorageError` (`#[non_exhaustive]`).
- `src/scope.rs` — `Scope { workspace_id, org_id }` plain data (reuses `nebula-core` IDs).
- `src/ids.rs` — re-export/newtype seam for `ExecutionId`/`WorkflowId`/`NodeKey` from `nebula-core`; `FencingToken`.
- `src/dto/mod.rs` + `dto/execution.rs`, `dto/workflow.rs`, `dto/control.rs`, `dto/journal.rs`, `dto/node_result.rs`, `dto/idempotency.rs`, `dto/webhook.rs`, `dto/identity.rs` — port-local row/record DTOs (no `ActionResult` dep).
- `src/batch.rs` — `TransitionBatch` + typed builder + `TransitionOutcome`.
- `src/store/mod.rs` + one file per trait: `execution.rs` (`ExecutionStore`), `journal.rs` (`ExecutionJournalReader`), `node_result.rs` (`NodeResultStore`), `checkpoint.rs` (`CheckpointStore`), `idempotency.rs` (`IdempotencyGuard` + `IdempotencyStore`), `workflow.rs` (`WorkflowStore`,`WorkflowVersionStore`), `control_queue.rs` (`ControlQueue`), `webhook.rs` (`WebhookActivationStore`), `refresh_claim.rs` (`RefreshClaimStore`), `identity.rs` (`User/Org/Workspace/Membership/Resource/Trigger/Quota/Audit/Blob` stores).
- `README.md` — port contract.

**Adapter `crates/storage/` (`nebula-storage`, Exec) — restructured:**
- `Cargo.toml` — add `nebula-storage-port` dep; keep sqlx/feature flags.
- `src/lib.rs` — re-export adapters; drop Layer-1/Layer-2 split docs.
- `src/inmem/` — `mod.rs` + per-store files; `parking_lot`-guarded.
- `src/sqlite/` — `mod.rs` + per-store files; `BEGIN IMMEDIATE` commit.
- `src/postgres/` — `mod.rs` + per-store files; tx + `FOR UPDATE SKIP LOCKED`.
- `src/credential/` — kept; `provider_cache.rs` split (cache/config/resolution); `layer/scope.rs` removed (moves to tenancy); `EncryptionLayer`/`CacheLayer`/`AuditLayer` retained; `refresh_claim/{in_memory,sqlite,postgres}.rs` retained, trait import now from port.
- `src/migrations.rs` — per-backend `migrate!()` selection.
- `migrations/postgres/*`, `migrations/sqlite/*` — canonical; flat `migrations/0000…*` deleted.
- `tests/conformance/` — `mod.rs` + `matrix.rs` (rstest over backends × {raw, scoped}).
- Deleted: `src/execution_repo.rs`, `src/workflow_repo.rs`, `src/backend/`, `src/pg/`, `src/repos/`, `src/rows/`, `src/mapping/`, `src/pool.rs` (folded/replaced), `src/error.rs` (moves to port).

**New crate `crates/tenancy/` (`nebula-tenancy`, Business):**
- `Cargo.toml` — deps: `nebula-storage-port`, `nebula-core`, `async-trait`, `tracing`.
- `src/lib.rs`, `src/resolver.rs` (`ScopeResolver: Principal→Scope`), `src/decorator/` (one scoping decorator per store trait), `src/error.rs`.
- `README.md`.

**Probe `crates/storage-loom-probe/`:** `src/lib.rs` doc + `src/cas_fencing.rs` (new boundary) + retained `lease_handoff.rs`/`refresh_claim` probes with invariant-equivalence note.

**Consumers (rewire):** `crates/engine/src/{engine.rs:138-140,control_dispatch.rs,control_consumer.rs,credential/}`, `crates/api/src/{state.rs,handlers/execution.rs,middleware/idempotency.rs,handlers/credential_oauth.rs,services/webhook/}`, `crates/api/tests/knife.rs`, `crates/engine/tests/lease_takeover.rs`.

**Workspace/governance:** `Cargo.toml` members, `deny.toml` `[wrappers]`, `AGENTS.md` (layer map + layout), `docs/adr/0066-*.md`, `docs/MATURITY.md`, `docs/ENGINE_GUARANTEES.md`, `crates/storage/README.md`.

---

## Phase P1 — Port crate (`nebula-storage-port`)

Greenfield, no consumers yet. Each store trait gets a stub `InMemory*` *inside the conformance skeleton only* so the contract compiles and the matrix is red-but-shaped.

### Task 1: Scaffold the port crate

**Files:**
- Create: `crates/storage-port/Cargo.toml`, `crates/storage-port/src/lib.rs`, `crates/storage-port/README.md`
- Modify: `Cargo.toml` (workspace `members`), `deny.toml` (`[wrappers]`)

- [ ] **Step 1: Create `crates/storage-port/Cargo.toml`**

```toml
[package]
name = "nebula-storage-port"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
description = "Storage port (object-safe repository traits + DTOs + Scope) for Nebula"
keywords.workspace = true
authors.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
documentation.workspace = true

[dependencies]
nebula-core = { path = "../core" }
async-trait = { workspace = true }
thiserror = { workspace = true }
serde = { workspace = true, features = ["derive"] }
serde_json = { workspace = true }

[lib]
doctest = false

[lints]
workspace = true
```

- [ ] **Step 2: Add to workspace members**

In root `Cargo.toml` `members` list, add `"crates/storage-port",` next to `"crates/storage",`.

- [ ] **Step 3: Add `nebula-storage-port` to `deny.toml` `[wrappers]`**

Add an allowlist entry (Core-tier contract — all storage consumers may depend on it):

```toml
  { crate = "nebula-storage-port", wrappers = [
    "nebula-storage",
    "nebula-storage-loom-probe",
    "nebula-tenancy",
    "nebula-engine",
    "nebula-api",
    "nebula-credential-vault",
  ], reason = "Storage port is a Core-tier contract; the adapter, tenancy decorator, exec/api consumers, and the credential-vault dev-dep may depend on it" },
```

- [ ] **Step 4: Minimal `src/lib.rs`**

```rust
//! # nebula-storage-port — the storage port
//!
//! Object-safe repository traits, port-local DTOs, the plain-data [`Scope`]
//! value type, and the [`TransitionBatch`] atomic unit-of-work. No backend
//! code lives here. See `docs/superpowers/specs/2026-05-15-nebula-storage-spec16-redesign-design.md`.
#![warn(missing_docs)]
#![warn(clippy::all)]

mod batch;
mod error;
mod ids;
mod scope;
/// Port-local row/record DTOs.
pub mod dto;
/// Repository traits (ISP-segregated, object-safe).
pub mod store;

pub use batch::{TransitionBatch, TransitionBatchBuilder, TransitionOutcome};
pub use error::StorageError;
pub use ids::FencingToken;
pub use scope::Scope;
```

- [ ] **Step 5: README + verify crate resolves**

Create `crates/storage-port/README.md` (purpose, "no sqlx", contract pointer). Run: `cargo check -p nebula-storage-port`
Expected: FAIL — unresolved modules `batch`/`error`/`ids`/`scope`/`dto`/`store` (created next tasks). This confirms the crate is wired into the workspace.

- [ ] **Step 6: Commit**

```bash
git add crates/storage-port Cargo.toml deny.toml
git commit -m "feat(storage-port): scaffold port crate"
```

### Task 2: `StorageError`

**Files:** Create `crates/storage-port/src/error.rs`
**Test:** Create `crates/storage-port/tests/error.rs`

- [ ] **Step 1: Failing test**

```rust
use nebula_storage_port::StorageError;

#[test]
fn not_found_is_constructible_and_display() {
    let e = StorageError::not_found("execution", "01J…");
    assert!(format!("{e}").contains("execution"));
}

#[test]
fn scope_violation_distinct_from_not_found() {
    let a = StorageError::not_found("execution", "x");
    let b = StorageError::ScopeViolation { entity: "execution" };
    assert_ne!(std::mem::discriminant(&a), std::mem::discriminant(&b));
}
```

- [ ] **Step 2: Run, expect FAIL**

Run: `cargo test -p nebula-storage-port --test error`
Expected: FAIL — `StorageError` not found.

- [ ] **Step 3: Implement `src/error.rs`**

```rust
//! Unified storage error.
use std::time::Duration;

/// Error returned by every port operation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StorageError {
    /// Entity absent (also returned on a deliberate cross-scope miss).
    #[error("{entity} not found: {id}")]
    NotFound { /// entity name
        entity: &'static str, /// opaque id
        id: String },
    /// Optimistic-CAS version mismatch.
    #[error("{entity} {id}: version conflict (expected {expected}, actual {actual})")]
    Conflict { /// entity
        entity: &'static str, /// id
        id: String, /// expected version
        expected: u64, /// actual version
        actual: u64 },
    /// Unique-constraint / first-writer collision.
    #[error("{entity} duplicate: {detail}")]
    Duplicate { /// entity
        entity: &'static str, /// detail
        detail: String },
    /// Lease could not be acquired.
    #[error("{entity} {id}: lease unavailable")]
    LeaseUnavailable { /// entity
        entity: &'static str, /// id
        id: String },
    /// Caller's fencing token was superseded.
    #[error("{entity} {id}: fenced out")]
    FencedOut { /// entity
        entity: &'static str, /// id
        id: String },
    /// Operation exceeded its deadline.
    #[error("{operation} timed out after {duration:?}")]
    Timeout { /// operation
        operation: String, /// elapsed
        duration: Duration },
    /// Persisted record carries a schema version this binary cannot decode.
    #[error("unknown schema version {found} (max supported {max})")]
    UnknownSchemaVersion { /// found
        found: u32, /// max
        max: u32 },
    /// Cross-tenant access denial surfaced to audit (never leaks the row).
    #[error("{entity}: scope violation")]
    ScopeViolation { /// entity
        entity: &'static str },
    /// (De)serialization failure.
    #[error("serialization: {0}")]
    Serialization(String),
    /// Backend connectivity failure.
    #[error("connection: {0}")]
    Connection(String),
    /// Misconfiguration (fail-closed).
    #[error("configuration: {0}")]
    Configuration(String),
    /// Unexpected internal invariant break.
    #[error("internal: {0}")]
    Internal(String),
}

impl StorageError {
    /// Construct a [`StorageError::NotFound`].
    pub fn not_found(entity: &'static str, id: impl Into<String>) -> Self {
        Self::NotFound { entity, id: id.into() }
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e.to_string())
    }
}
```

- [ ] **Step 4: Run, expect PASS**

Run: `cargo test -p nebula-storage-port --test error`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/storage-port/src/error.rs crates/storage-port/tests/error.rs
git commit -m "feat(storage-port): StorageError (non_exhaustive, fail-closed variants)"
```

### Task 3: `Scope` + `FencingToken` + id seam

**Files:** Create `crates/storage-port/src/scope.rs`, `crates/storage-port/src/ids.rs`; Test `crates/storage-port/tests/scope.rs`

- [ ] **Step 1: Failing test**

```rust
use nebula_storage_port::{Scope, FencingToken};

#[test]
fn scope_equality_and_serde() {
    let s = Scope::new("ws_1", "org_1");
    let j = serde_json::to_string(&s).unwrap();
    assert_eq!(s, serde_json::from_str(&j).unwrap());
}

#[test]
fn fencing_token_is_monotone_comparable() {
    assert!(FencingToken::from_generation(1) < FencingToken::from_generation(2));
}
```

- [ ] **Step 2: Run, expect FAIL** — `cargo test -p nebula-storage-port --test scope`

- [ ] **Step 3: Implement `src/scope.rs`**

```rust
//! Plain-data tenant scope. Policy/resolution lives in `nebula-tenancy`.
use serde::{Deserialize, Serialize};

/// Workspace + org isolation key. Required by every tenant-scoped operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Scope {
    /// Workspace identifier.
    pub workspace_id: String,
    /// Organization identifier.
    pub org_id: String,
}

impl Scope {
    /// Build a scope from workspace + org ids.
    pub fn new(workspace_id: impl Into<String>, org_id: impl Into<String>) -> Self {
        Self { workspace_id: workspace_id.into(), org_id: org_id.into() }
    }
}
```

- [ ] **Step 4: Implement `src/ids.rs`**

```rust
//! Id seam + lease fencing token.

/// Monotone lease fencing token. A reclaim/takeover bumps the generation;
/// `commit`/`renew_lease` reject a non-current token even on a version match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FencingToken(u64);

impl FencingToken {
    /// Construct from a monotone generation counter.
    pub fn from_generation(g: u64) -> Self { Self(g) }
    /// Underlying generation.
    pub fn generation(self) -> u64 { self.0 }
}
```

- [ ] **Step 5: Run, expect PASS** — `cargo test -p nebula-storage-port --test scope`

- [ ] **Step 6: Commit**

```bash
git add crates/storage-port/src/scope.rs crates/storage-port/src/ids.rs crates/storage-port/tests/scope.rs
git commit -m "feat(storage-port): Scope value type + monotone FencingToken"
```

### Task 4: Port-local DTOs

**Files:** Create `crates/storage-port/src/dto/mod.rs` and `dto/{execution,workflow,control,journal,node_result,idempotency,webhook,identity}.rs`; Test `crates/storage-port/tests/dto.rs`

- [ ] **Step 1: Failing test** — assert `ExecutionRecord`, `NodeResultRecord { kind_tag, json, schema_version }`, `ControlMsg`, `JournalEntry`, `WorkflowRecord`/`WorkflowVersionRecord` round-trip via serde and that `NodeResultRecord` does **not** reference any `ActionResult` type (compile-time: only `serde_json::Value`).

```rust
use nebula_storage_port::dto::{ExecutionRecord, NodeResultRecord, ControlMsg, JournalEntry};
#[test]
fn node_result_record_is_action_result_free_and_roundtrips() {
    let r = NodeResultRecord { kind_tag: "Value".into(), json: serde_json::json!({"k":1}), schema_version: 1 };
    let s = serde_json::to_string(&r).unwrap();
    let back: NodeResultRecord = serde_json::from_str(&s).unwrap();
    assert_eq!(back.schema_version, 1);
}
```

- [ ] **Step 2: Run, expect FAIL** — `cargo test -p nebula-storage-port --test dto`

- [ ] **Step 3: Implement DTOs.** Each file defines `#[derive(Debug,Clone,Serialize,Deserialize,PartialEq)]` structs. Concrete required fields (from spec §4 + canon + exploration of current rows):
  - `execution.rs`: `ExecutionRecord { id:String, workflow_id:String, scope:Scope, version:u64, status:String, state:serde_json::Value, lease_holder:Option<String>, fencing:Option<u64>, created_at:String, updated_at:String }`
  - `node_result.rs`: `NodeResultRecord { kind_tag:String, json:serde_json::Value, schema_version:u32 }`; const `MAX_SUPPORTED_RESULT_SCHEMA_VERSION:u32 = 1;`
  - `control.rs`: `ControlMsg { id:[u8;16], execution_id:String, command:ControlCommand, scope:Scope, w3c_traceparent:Option<String>, reclaim_count:u32 }`, `enum ControlCommand { Start, Cancel, Terminate, Resume, Restart }`
  - `journal.rs`: `JournalEntry { seq:Option<u64>, payload:serde_json::Value }`
  - `workflow.rs`: `WorkflowRecord { id, scope, version:u64, slug:String, deleted:bool }`, `WorkflowVersionRecord { workflow_id, number:u32, published:bool, pinned:bool, definition:serde_json::Value }`
  - `idempotency.rs`: `CachedRecord { status:u16, headers:Vec<u8>, body:Vec<u8>, fingerprint:Vec<u8>, expires_at:String }`
  - `webhook.rs`: `WebhookActivationRecord { trigger_id:String, scope:Scope, slug:String, active:bool }`
  - `identity.rs`: `UserRow/OrgRow/WorkspaceRow/MembershipRow/ResourceRow/TriggerRow/QuotaRow/AuditLogRow/BlobRow` — mirror the column sets in `crates/storage/migrations/postgres/0001-0019` (worker reads those SQL files and maps 1:1).
  - `mod.rs`: `pub mod` each + re-export.

- [ ] **Step 4: Run, expect PASS** — `cargo test -p nebula-storage-port --test dto`

- [ ] **Step 5: Commit**

```bash
git add crates/storage-port/src/dto crates/storage-port/tests/dto.rs
git commit -m "feat(storage-port): port-local DTOs (no ActionResult dependency)"
```

### Task 5: `TransitionBatch` + builder + outcome

**Files:** Create `crates/storage-port/src/batch.rs`; Test `crates/storage-port/tests/batch.rs`

- [ ] **Step 1: Failing test**

```rust
use nebula_storage_port::{TransitionBatch, TransitionOutcome, FencingToken, Scope};
use nebula_storage_port::dto::{ControlMsg, JournalEntry};

#[test]
fn builder_requires_core_fields_and_allows_empty_outbox_journal() {
    let b = TransitionBatch::builder()
        .scope(Scope::new("w","o"))
        .execution_id("01J")
        .expected_version(3)
        .fencing(FencingToken::from_generation(7))
        .new_state(serde_json::json!({"s":"running"}))
        .build()
        .expect("all required fields present");
    assert!(b.outbox().is_empty() && b.journal().is_empty());
}

#[test]
fn outcome_variants_exist() {
    let _ = TransitionOutcome::Applied { new_version: 4 };
    let _ = TransitionOutcome::VersionConflict { actual: 9 };
    let _ = TransitionOutcome::FencedOut;
}
```

- [ ] **Step 2: Run, expect FAIL** — `cargo test -p nebula-storage-port --test batch`

- [ ] **Step 3: Implement `src/batch.rs`** — typed builder; missing required field ⇒ `Err(StorageError::Configuration(...))`; `outbox`/`journal` default empty; getters; `TransitionOutcome { Applied{new_version:u64}, VersionConflict{actual:u64}, FencedOut }`. Builder is the only constructor (fields private) so a caller cannot transition without declaring scope+version+fencing.

- [ ] **Step 4: Run, expect PASS** — `cargo test -p nebula-storage-port --test batch`

- [ ] **Step 5: Commit**

```bash
git add crates/storage-port/src/batch.rs crates/storage-port/tests/batch.rs
git commit -m "feat(storage-port): TransitionBatch unit-of-work + typed builder"
```

### Task 6: Store traits (object-safe, ISP-segregated)

**Files:** Create `crates/storage-port/src/store/mod.rs` + per-trait files (see File Structure); Test `crates/storage-port/tests/object_safe.rs`

- [ ] **Step 1: Failing test (object-safety is the contract)**

```rust
use nebula_storage_port::store::*;
fn _assert_object_safe(
    _a: &dyn ExecutionStore, _b: &dyn ExecutionJournalReader, _c: &dyn NodeResultStore,
    _d: &dyn CheckpointStore, _e: &dyn IdempotencyGuard, _f: &dyn IdempotencyStore,
    _g: &dyn WorkflowStore, _h: &dyn WorkflowVersionStore, _i: &dyn ControlQueue,
    _j: &dyn WebhookActivationStore, _k: &dyn RefreshClaimStore,
) {}
#[test] fn traits_are_object_safe() { /* compiles ⇒ pass */ }
```

- [ ] **Step 2: Run, expect FAIL** — `cargo test -p nebula-storage-port --test object_safe` (traits absent).

- [ ] **Step 3: Implement traits** — each `#[async_trait::async_trait] pub trait X: Send + Sync + std::fmt::Debug`. Signatures per spec §4.1/§4.2. Key ones verbatim:

```rust
#[async_trait::async_trait]
pub trait ExecutionStore: Send + Sync + std::fmt::Debug {
    async fn create(&self, scope: &Scope, id: &str, workflow_id: &str, initial_state: serde_json::Value) -> Result<(), StorageError>;
    async fn get(&self, scope: &Scope, id: &str) -> Result<Option<ExecutionRecord>, StorageError>;
    async fn commit(&self, batch: TransitionBatch) -> Result<TransitionOutcome, StorageError>;
    async fn acquire_lease(&self, scope: &Scope, id: &str, holder: &str, ttl: std::time::Duration) -> Result<Option<FencingToken>, StorageError>;
    async fn renew_lease(&self, scope: &Scope, id: &str, token: FencingToken, ttl: std::time::Duration) -> Result<bool, StorageError>;
    async fn release_lease(&self, scope: &Scope, id: &str, token: FencingToken) -> Result<bool, StorageError>;
    async fn list_running(&self, scope: &Scope) -> Result<Vec<String>, StorageError>;
    async fn list_running_for_workflow(&self, scope: &Scope, workflow_id: &str) -> Result<Vec<String>, StorageError>;
    async fn count(&self, scope: &Scope, workflow_id: Option<&str>) -> Result<u64, StorageError>;
}
```

`ControlQueue` uses typed 16-byte ids (`[u8;16]`), **not** UTF-8-of-ULID. `IdempotencyGuard::check_and_mark` keeps key shape `{execution_id}:{node_id}:{attempt}` and is scope-namespaced. `RefreshClaimStore` mirrors the existing `crates/storage/src/credential/refresh_claim/mod.rs` trait shape exactly (worker copies signatures). `identity.rs` declares the zoo traits (bodies are P5). `store/mod.rs` `pub mod`+re-export all.

- [ ] **Step 4: Run, expect PASS** — `cargo test -p nebula-storage-port --test object_safe`; then `cargo clippy -p nebula-storage-port -- -D warnings`.

- [ ] **Step 5: Commit**

```bash
git add crates/storage-port/src/store crates/storage-port/tests/object_safe.rs
git commit -m "feat(storage-port): object-safe ISP-segregated store traits"
```

### Task 7: Conformance skeleton (the green target)

**Files:** Create `crates/storage-port/tests/conformance_contract.rs` (trait-level contract doc-tests as compile-time guards). Real backend matrix lands in P2 under `crates/storage/tests/`.

- [ ] **Step 1:** Write a `#[cfg(test)]` reference `StubExecutionStore` (always `FencedOut`) and assert: `commit` with a stale `FencingToken` ⇒ `TransitionOutcome::FencedOut`; `get` with mismatched scope ⇒ `Ok(None)`. This pins the *contract* before any real backend.
- [ ] **Step 2: Run, expect FAIL** then implement stub, **expect PASS** — `cargo test -p nebula-storage-port --test conformance_contract`.
- [ ] **Step 3: Commit** — `git commit -m "test(storage-port): contract guards for fencing + cross-scope miss"`.
- [ ] **Step 4: Phase gate** — `cargo fmt -p nebula-storage-port`; `cargo clippy -p nebula-storage-port -- -D warnings`; `cargo nextest run -p nebula-storage-port`. All green ⇒ P1 done.

---

## Phase P2 — Adapter parity (InMemory → SQLite → Postgres)

Implement the port for three backends + the behavioral conformance matrix. **Parallelizable:** after Task 8 (matrix harness) lands, InMemory/SQLite/Postgres execution-core impls (Tasks 9–11) and the non-execution stores (control/idempotency/webhook/journal — Tasks 12–13) are independent worker streams; refresh-claim re-home (Task 14) is independent.

### Task 8: Conformance matrix harness

**Files:** Create `crates/storage/tests/conformance/mod.rs`, `crates/storage/tests/conformance/matrix.rs`; Modify `crates/storage/Cargo.toml` (add `nebula-storage-port`, `rstest` already dev-dep).

- [ ] **Step 1:** Define a `trait Backend { async fn execution_store(&self) -> Arc<dyn ExecutionStore>; … }` and an `rstest` `#[case]` per backend (`InMemory`; `Sqlite(":memory:")`; `Postgres` `#[cfg_attr(not(env DATABASE_URL), ignore)]`). Behavioral assertions (the contract): create→get round-trip; CAS conflict returns `VersionConflict{actual}`; stale fencing returns `FencedOut`; atomic triple (commit with outbox+journal then read both) is all-or-nothing on injected failure; idempotency key shape + first-writer-wins; cross-scope `get`/`commit` ⇒ `NotFound`/`None`.
- [ ] **Step 2: Run, expect FAIL** — `cargo test -p nebula-storage --test conformance` (no impls yet; all cases fail/panic-to-fail, not compile error).
- [ ] **Step 3: Commit** — `test(storage): backend conformance matrix harness (red)`.

### Task 9: InMemory adapter

**Files:** Create `crates/storage/src/inmem/{mod,execution,journal,node_result,checkpoint,control_queue,idempotency,webhook}.rs`; Modify `crates/storage/src/lib.rs`.

- [ ] **Step 1–4 (TDD against the matrix):** implement `InMemoryExecutionStore` etc. with one `parking_lot::Mutex<State>` per store; `commit` performs CAS+fencing+state+outbox+journal under one lock; scope predicate enforced (mismatch ⇒ `None`/`NotFound`). Run `cargo nextest run -p nebula-storage --test conformance` filtering the `InMemory` case until green.
- [ ] **Step 5: Commit** — `feat(storage): InMemory adapter (conformance green)`.

### Task 10: SQLite adapter

**Files:** Create `crates/storage/src/sqlite/*`; Modify `lib.rs`, `Cargo.toml`.

- [ ] Implement against the matrix `Sqlite(:memory:)` case. `commit` opens `BEGIN IMMEDIATE`; control-queue claim is single-consumer status flip (documented: no `SKIP LOCKED`). Dialect: `BLOB`/`INTEGER`/`TEXT`, `ON CONFLICT`, no `gen_random_uuid()`. Green ⇒ commit `feat(storage): SQLite adapter (single-writer correctness)`.

### Task 11: Postgres adapter

**Files:** Create `crates/storage/src/postgres/*` (port `backend/pg_execution.rs` + `pg/*` logic, split by concern: `cas.rs`/`lease.rs`/`journal.rs`/`node_output.rs`/`idempotency.rs`/`control_queue.rs`).

- [ ] Implement against `DATABASE_URL`-gated matrix case. `commit` = real tx; control-queue claim = `FOR UPDATE SKIP LOCKED`. Reuse proven SQL from current `pg/control_queue.rs`/`pg/idempotency.rs`. Green (when `DATABASE_URL` set) ⇒ commit `feat(storage): Postgres adapter (tx + SKIP LOCKED)`.

### Task 12–13: Control-queue / idempotency / webhook / journal stores (all backends)

- [ ] Port `repos/control_queue.rs`, `pg/idempotency.rs`, `pg/webhook_activation.rs` semantics to the new traits with **typed 16-byte ids** (delete the UTF-8-ULID hack); scope-namespaced idempotency `cache_key`; matrix green for the relevant cases per backend. Commit per store: `feat(storage): <store> across backends`.

### Task 14: Re-home refresh-claim

- [ ] Change `crates/storage/src/credential/refresh_claim/{mod,in_memory,sqlite,postgres}.rs` to `impl nebula_storage_port::store::RefreshClaimStore` (shape unchanged); update `crates/storage/src/lib.rs` re-exports. Run existing `crates/storage/tests/refresh_claim_*` — expect PASS unchanged. Commit `refactor(storage): refresh-claim implements port trait (shape unchanged)`.

- [ ] **Phase gate P2:** `cargo nextest run -p nebula-storage` (InMemory+SQLite green; Postgres green iff `DATABASE_URL`); `cargo clippy -p nebula-storage -- -D warnings`; `cargo fmt -p nebula-storage`.

---

## Phase P3 — Tenancy (`nebula-tenancy`)

### Task 15: Scaffold + `ScopeResolver`

**Files:** Create `crates/tenancy/{Cargo.toml,src/lib.rs,src/resolver.rs,src/error.rs,README.md}`; Modify root `Cargo.toml` members, `deny.toml` (`nebula-tenancy` wrappers = `nebula-api`, plus dev-deps), `AGENTS.md`.

- [ ] TDD: `ScopeResolver` trait (`fn resolve(&self, principal:&Principal) -> Result<Scope, TenancyError>`) + a default impl; unit test resolve happy/deny. Commit `feat(tenancy): scaffold + ScopeResolver`.

### Task 16: Scoping decorators + threat-model conformance

**Files:** Create `crates/tenancy/src/decorator/{mod,execution,control_queue,idempotency,…}.rs`; Create `crates/tenancy/tests/cross_tenant_denial.rs`.

- [ ] **Step 1: Failing tests = the abuse cases (spec §6.1):** id↔scope mismatch ⇒ `NotFound` (never the row); cross-tenant idempotency key isolation; cross-tenant control enqueue rejected; pending-store cross-tenant replay denied.
- [ ] **Step 2–4:** implement decorators wrapping `Arc<dyn …>`; adapter constructors made `pub(crate)`-to-wiring so engine can only get the scoped handle. Run `cargo nextest run -p nebula-tenancy` + the scoped variant of the storage matrix (wire `nebula-tenancy` as a dev-dep in `crates/storage/tests/conformance`).
- [ ] **Step 5: Commit** — `feat(tenancy): scope-enforcing decorators + cross-tenant denial conformance`.

### Task 17: Credential scope-layer re-home

- [ ] Move `crates/storage/src/credential/layer/scope.rs` logic into `nebula-tenancy`; re-compose `EncryptionLayer`/`CacheLayer`/`AuditLayer` on top with **fail-closed audit + zeroize regression tests** (port the existing `crates/storage/tests/credential_*` assertions, add a cross-tenant pending-replay test). Commit `refactor(tenancy,storage): re-home credential scope layer, preserve fail-closed+zeroize`.
- [ ] **Phase gate P3:** clippy/fmt/nextest per touched crate; scoped conformance variant green.

---

## Phase P4 — Engine/api/core rewire (canon-critical depth)

### Task 18: Engine on the scoped port

**Files:** Modify `crates/engine/src/engine.rs:138-140`, `control_dispatch.rs`, `control_consumer.rs`, `credential/*`, `crates/engine/Cargo.toml` (dep `nebula-storage-port`, drop direct concrete-only usage where possible).

- [ ] Replace `Arc<dyn nebula_storage::ExecutionRepo>` with `Arc<dyn nebula_storage_port::store::ExecutionStore>` (already-scoped handle). Map old call sites to new signatures (`get_state`→`get`, `transition`→`commit(TransitionBatch)`, lease ops→fencing-token ops, `append_journal`→batch journal). Engine acquires `FencingToken` on lease and threads it into every `commit` (closes zombie-runner). Compile-driven; trust compiler at milestone end (bold pass, not per-edit gating).
- [ ] **Tests:** port `crates/engine/tests/lease_takeover.rs` to the new seam with a written invariant-equivalence note at the top of the test file. Run `cargo nextest run -p nebula-engine`.
- [ ] Commit `refactor(engine): consume scoped storage port; fencing gates every commit`.

### Task 19: API on the scoped port

**Files:** Modify `crates/api/src/state.rs`, `handlers/execution.rs`, `middleware/idempotency.rs`, `handlers/credential_oauth.rs`, `services/webhook/*`, `crates/api/Cargo.toml` (+`nebula-storage-port`, +`nebula-tenancy`).

- [ ] `AppState` builds the adapter, wraps it in the `nebula-tenancy` decorator from the request `Principal`, stores `Arc<dyn …>` port handles. Update imports/usages. Idempotency middleware uses scope-namespaced key.
- [ ] Commit `refactor(api): scoped port wiring in AppState + handlers`.

### Task 20: Knife end-to-end + loom

**Files:** Modify `crates/api/tests/knife.rs`; `crates/storage-loom-probe/src/{lib.rs,cas_fencing.rs}`.

- [ ] Port knife to the new wiring; assert §13 guarantee unchanged (equivalence note in file header). Add a loom probe for the CAS+fencing+outbox boundary; keep `lease_handoff`/`refresh_claim` probes with an invariant-equivalence note in `lib.rs`.
- [ ] Run knife: `cargo nextest run -p nebula-api --test knife`. Run loom: `RUSTFLAGS="--cfg loom" cargo nextest run -p nebula-storage-loom-probe --features loom-test --profile ci --no-tests=pass`.
- [ ] Commit `test(api,storage-loom-probe): port knife + CAS/fencing loom probe (equivalence noted)`.
- [ ] **Phase gate P4:** per-crate clippy/fmt/nextest for storage-port, storage, tenancy, engine, api; knife + loom green.

---

## Phase P5 — Identity zoo (breadth; parallelizable, does not block P4)

### Task 21: Identity stores × 3 backends

**Files:** `crates/storage/src/{inmem,sqlite,postgres}/identity/*`; `crates/storage/tests/conformance/identity_matrix.rs`.

- [ ] One independent sub-task per store (`User`,`Org`,`Workspace`,`Membership`,`Resource`,`Trigger`,`Quota`,`Audit`,`Blob`) — **dispatch as parallel implement-workers**. Each: implement the port trait for InMemory+SQLite+Postgres against an `identity_matrix` rstest case (CRUD + scope isolation + soft-delete where applicable), reading the column set from `migrations/postgres/000X_*.sql`. Commit per store: `feat(storage): <entity> store across backends`.
- [ ] **Phase gate P5:** identity matrix green (InMemory+SQLite; Postgres iff `DATABASE_URL`).

---

## Phase P6 — Migration consolidation

### Task 22: Single migration source + runner

**Files:** Create `crates/storage/src/migrations.rs`; Delete `crates/storage/migrations/00000000000002…0009*.sql`; Modify `crates/storage/src/lib.rs`.

- [ ] Port any drift from the flat tree into `migrations/{postgres,sqlite}/*` (diff the flat DDL vs structured; add missing columns/indexes as new numbered structured migrations — never edit applied ones). Wire `sqlx::migrate!()` per backend dir. Delete the flat tree. Update `crates/storage/migrations/{postgres,sqlite}/README.md`.
- [ ] **Verify:** fresh `task db:reset && task db:migrate` (Postgres) succeeds; SQLite `:memory:` conformance still green. Add to `crates/storage/README.md`: "rebuild destroys local dev DB — run `task db:reset`".
- [ ] Commit `refactor(storage): single per-backend migration tree; delete legacy flat tree`.

---

## Phase P7 — Docs / ADR / governance

### Task 23: ADR-0072 + doc updates

**Files:** Create `docs/adr/0072-nebula-storage-spec16-port-adapter-tenancy.md`; Modify `docs/adr/README.md`, `crates/storage/README.md`, `crates/storage-port/README.md`, `crates/tenancy/README.md`, `docs/MATURITY.md`, `docs/ENGINE_GUARANTEES.md`, `AGENTS.md`, `deny.toml` (comment accuracy), remove `crates/storage/src/pool.rs` if still unused.

- [ ] ADR-0072 records every spec §2 decision (object-safe shape + rationale, crate split + layer-map/deny deltas, data-vs-policy tenancy, `TransitionBatch`+fencing, migration cutover assumption, SQLite parity boundary, delegated-review note) and **supersedes the Sprint E deferral**; cross-link from `README.md` ADR index. MATURITY: storage row → `stable`; add `nebula-storage-port`, `nebula-tenancy` rows. ENGINE_GUARANTEES durability matrix: SQLite now first-class for the core. AGENTS.md layer map + workspace layout updated. Verify no plan IDs leaked into code (`rg -n "P[1-7]\b|Phase [A-Z]|spec-16 task" crates/`).
- [ ] Commit `docs(storage): ADR-0072 + MATURITY/ENGINE_GUARANTEES/AGENTS/README updates`.
- [ ] **Final gate:** per-crate `cargo fmt`/`cargo clippy -- -D warnings`/`cargo nextest run` for storage-port, storage, tenancy, engine, api, storage-loom-probe; `cargo deny check wrappers`; knife + loom green; conformance matrix green (InMemory+SQLite always; Postgres iff `DATABASE_URL`). Report per-crate results faithfully (no workspace `dev:check` claim from this worktree).

---

## Self-Review

**Spec coverage:** §2.1 object-safe shape→T6; §2.2 topology→T1/T15; §2.3 `TransitionBatch`→T5; §2.4 fencing→T5/T6/T9-11/T18; §2.5 tenancy security→T15-17; §2.6 port deps→T1/T4; §2.7 SQLite parity→T10; §2.8 conformance/loom/knife→T7/T8/T20. §3 crates→T1/T15. §4 traits/DTOs→T4-6. §5 concurrency→T9-11. §6 abuse cases→T16. §7 migrations→T22. §8 credential→T14/T17. §9 conformance→T8/T20. §10 quality→global conventions + phase gates. §11 phasing→P1-P7. §13 ADR/docs→T23. All spec sections mapped.

**Placeholder scan:** identity DTO/column sets defer to "read `migrations/postgres/000X`" — that is a concrete, located instruction (the SQL is the source of truth), not a TBD. Backend SQL bodies (P2/P5) are specified by contract + matrix + proven-source pointer rather than full final SQL because they are edits against existing reading-required code; the conformance matrix is the executable acceptance. No "TODO/later/handle edge cases" placeholders.

**Type consistency:** `Scope`, `FencingToken`, `TransitionBatch`/`TransitionOutcome`, `StorageError`, `NodeResultRecord{kind_tag,json,schema_version}`, `ExecutionStore::{create,get,commit,acquire_lease,renew_lease,release_lease}` are named identically across T2-T6, T8-T11, T18-T19. `ControlMsg`/`ControlCommand` consistent T4/T6/T12.

Gaps fixed inline: none outstanding.
