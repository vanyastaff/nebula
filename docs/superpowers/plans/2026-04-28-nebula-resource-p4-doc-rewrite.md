# nebula-resource П4 — Documentation Rewrite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild `crates/resource/docs/*` against the post-cascade trait shape and topology surface. Replace ~50%-fabrication `api-reference.md`, ground-up rewrite `adapters.md`, retire stale `Architecture.md` + `dx-eval-real-world.rs`, fix README intra-doc links, rebuild `events.md` against the actual 12-variant `ResourceEvent`, and verify `Pooling.md` + `recovery.md` accuracy. Closes 🔴 R-030, 🔴 R-031, 🟠 R-032, 🟠 R-033, 🟠 R-034, 🟡 R-035 from `docs/tracking/nebula-resource-concerns-register.md`.

**Architecture:** Pure-docs PR — zero source changes. Each doc file is rewritten or retired against the canonical sources in `crates/resource/src/{lib,resource,manager/*,events,topology_tag,runtime/mod}.rs`. The single integration test added (`crates/resource/tests/adapter_smoke.rs`) is OUT OF SCOPE for П4 — keep `rust,ignore` blocks in `adapters.md` and rely on careful source-grounded review for accuracy.

**Tech Stack:** Markdown + RUSTDOCFLAGS=-D warnings doc gate. No new code paths, no test churn — test counts must stay at 3660/3660 from П3 baseline.

**Source documents:**

- [docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md](../specs/2026-04-24-nebula-resource-tech-spec.md) §6 (docs subsection) — landing site for the doc-rewrite scope
- [docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md](../specs/2026-04-24-nebula-resource-redesign-strategy.md) §4.7 — "rewrite OR delete" policy for the four 🔴/🟠 doc concerns
- [docs/tracking/nebula-resource-concerns-register.md](../../tracking/nebula-resource-concerns-register.md) — closes R-030/R-031/R-032/R-033/R-034/R-035
- [docs/adr/0036-resource-credential-adoption-auth-retirement.md](../../adr/0036-resource-credential-adoption-auth-retirement.md) — `type Credential` shape that api-reference.md must reflect
- [docs/adr/0037-daemon-eventsource-engine-fold.md](../../adr/0037-daemon-eventsource-engine-fold.md) — explains why Daemon/EventSource sections must be REMOVED from resource docs
- [crates/resource/src/lib.rs](../../../crates/resource/src/lib.rs) — public re-export surface (canonical)
- [crates/resource/src/resource.rs](../../../crates/resource/src/resource.rs) — `Resource` trait + `ResourceConfig` + `ResourceMetadata` shape
- [crates/resource/src/manager/mod.rs](../../../crates/resource/src/manager/mod.rs) — Manager API (10× `register_*` + 10× `acquire_*` + `subscribe_events`)
- [crates/resource/src/events.rs](../../../crates/resource/src/events.rs) — actual 12 `ResourceEvent` variants
- [crates/resource/src/topology_tag.rs](../../../crates/resource/src/topology_tag.rs) — actual 5 `TopologyTag` variants
- [crates/resource/src/runtime/mod.rs](../../../crates/resource/src/runtime/mod.rs) — actual 5-variant `TopologyRuntime`

**Closes (concerns register):**

- 🔴 R-030 — `api-reference.md` ~50% fabrication rate (`ResourceContext::with_scope/.with_cancel_token`, `AcquireCircuitBreakerPreset`, 4-field `ResourceMetadata`, 7-variant `TopologyTag`, missing register/acquire methods)
- 🔴 R-031 — `adapters.md` 4/7 compile-fail blocks (post-Auth-retirement: `Credential` const wrong, removed `prepare`, missing `register_*_with` walkthrough)
- 🟠 R-032 — `Architecture.md` describes vanished v1 module map (HookRegistry, QuarantineManager, EventBus, AutoScaler, Poison, DependencyGraph)
- 🟠 R-033 — `README.md` case-drift broken intra-doc links (`pooling.md` vs `Pooling.md`; `events-and-hooks.md` and `health-and-quarantine.md` reference non-existent files)
- 🟠 R-034 — `dx-eval-real-world.rs` references nonexistent types (`Credential::KIND` vs `KEY`, `Credential = ()` removed)
- 🟡 R-035 — `events.md` lists 7 variants vs actual 12 (`RetryAttempt`, `BackpressureDetected`, `RecoveryGateChanged`, `CredentialRefreshed`, `CredentialRevoked` missing)

**Non-goals (explicitly deferred):**

- New integration test scaffolding for adapter examples (deferred to follow-up; keep `rust,ignore` blocks for now)
- Maturity transition (`frontier` → `core`) — Strategy §6.4 keeps it post-soak (П5 candidate)
- Topology trait *behavior* documentation deltas not driven by R-030..R-035 — out of scope; П4 is pure drift correction
- New examples in root `examples/` workspace member — separate follow-up task; this PR keeps in-doc rendered examples
- Rename README.md → readme.md — keep README.md as-is per markdown convention
- Rotation observability documentation in `events.md` — П2 already shipped these; П4 just verifies the catalog accuracy

**Design decisions resolved (from kickoff brief open questions):**

1. **`Architecture.md` rewrite vs delete?** → **Delete.** All 316 LOC describe the v1 module map (HookRegistry, QuarantineManager, EventBus, Poison, DependencyGraph, AutoScaler) that was extracted/replaced cascade-by-cascade. The README.md "Crate Layout" + "Core Concepts" sections are the architectural overview; canonical references live in `docs/PRODUCT_CANON.md` §4.5/§11.4 and `docs/INTEGRATION_MODEL.md`. A slim 100-line replacement would drift the same way. Reduce surface for drift.
2. **`dx-eval-real-world.rs` fate — fix, delete, or gate?** → **Delete.** It's a 1012-LOC non-compiling design evaluation referencing fictional driver types. Most friction commentary references concerns now resolved in П1 (`Credential = ()` retired, `register_pooled_with` exists). Adapters.md + README "Quick Start" + integration tests in `crates/resource/tests/` cover the same ground with working examples.
3. **Doctest harness for adapter examples?** → **Defer.** Keep `rust,ignore` blocks in `adapters.md`; add a clear front-matter note explaining why (the examples reference fictional driver types). Compile-time enforcement via `tests/adapter_smoke.rs` is a follow-up task — not blocking П4. R-031 is closed by accuracy of the *content* (correct types, signatures, lifecycle order) regardless of doctest gating.
4. **Plan grouping — file-by-file or theme-cluster?** → **File-by-file**, with one preliminary task (Task 1) to capture the canonical API surface as a working artefact. Each subagent dispatches against one file with a clear scope; reviews are easier when commits are file-scoped.
5. **Single PR vs split?** → **Single PR.** It's a cohesive doc rewrite — splitting would force artificial dependency ordering between cross-referencing files (e.g., adapters.md cites api-reference.md anchors).

---

## File Structure

### Files deleted

| File | Reason |
|---|---|
| `crates/resource/docs/Architecture.md` | 100% v1 fabrication; README + canon docs cover real architecture; closes R-032 |
| `crates/resource/docs/dx-eval-real-world.rs` | Stale design eval, non-compiling, friction commentary obsolete; closes R-034 |

### Files rewritten

| File | Before LOC | Target LOC | Change kind |
|---|---|---|---|
| `crates/resource/docs/api-reference.md` | 869 | ~600 | Full rewrite from source — closes R-030 |
| `crates/resource/docs/adapters.md` | 414 | ~350 | Ground-up rewrite — closes R-031 |
| `crates/resource/docs/events.md` | 90 | ~150 | Rebuild against 12-variant `ResourceEvent` — closes R-035 |

### Files fixed (smaller edits)

| File | Change |
|---|---|
| `crates/resource/docs/README.md` | Drop Daemon/EventSource topology rows; fix 4 broken intra-doc links (`pooling.md`, `events-and-hooks.md` → `events.md`, `health-and-quarantine.md` → `recovery.md`, `architecture.md` removed); refresh "Crate Layout" with current module tree (incl. manager/ submodule split, removal of v1 names). Closes R-033. |
| `crates/resource/docs/Pooling.md` → renamed `crates/resource/docs/pooling.md` | Lowercase rename for case-consistency with intra-doc references; verify `PoolConfig` field names match `crates/resource/src/topology/pooled/config.rs`; verify backpressure types match current source |
| `crates/resource/docs/recovery.md` | Verify `RecoveryGate`/`WatchdogHandle` API still matches `crates/resource/src/recovery/`; spot-check examples; minor edits if drift found |
| `docs/tracking/nebula-resource-concerns-register.md` | Mark R-030/R-031/R-032/R-033/R-034/R-035 status `landed П4` with PR/commit pointers |

### Verification commands

```bash
# Doc gate (CRITICAL for П4 — catches broken intra-doc links after Daemon/EventSource removal)
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Test counts must stay at 3660/3660 (П3 baseline)
cargo nextest run --workspace --profile ci --no-tests=pass

# Lints/format must remain clean
cargo +nightly fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

# Per-crate sanity
cargo check -p nebula-resource
```

---

## Task 0 — Pre-flight: branch, worktree, baseline

**Files:**
- None modified
- Verify: branch state, baseline `cargo doc`

**Why:** П4 lands in a worktree already pinned to `claude/resource-p4-doc-rewrite` (`.worktrees/nebula/magical-tesla-0f17a4` per `git worktree list`). Confirm baseline gates pass before changes; capture `wc -l` snapshot for end-of-PR delta accounting.

- [ ] **Step 1: Confirm branch and worktree alignment**

```bash
git worktree list | grep "claude/resource-p4-doc-rewrite"
# Expected: .worktrees/nebula/magical-tesla-0f17a4 ... [claude/resource-p4-doc-rewrite]
```

If absent, this plan was misrouted — escalate before continuing.

- [ ] **Step 2: Update branch to current `origin/main`**

From the magical-tesla worktree:

```bash
git pull --ff-only
```

Expected: branch tip moves to `origin/main` HEAD (currently `671a0ffd` П3). If non-FF, escalate (means concurrent work landed).

- [ ] **Step 3: Run baseline gates and record outcomes**

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps 2>&1 | tail -20
cargo nextest run --workspace --profile ci --no-tests=pass 2>&1 | tail -5
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
cargo +nightly fmt --all -- --check 2>&1 | tail -5
```

Expected:
- `cargo doc`: clean (warning-free)
- `nextest`: **3660/3660 passed** (П3 baseline)
- `clippy`: 0 warnings
- `fmt`: no diff

If any baseline fails, fix-or-escalate before changes — that means main is broken, not your concern.

- [ ] **Step 4: Capture baseline doc LOC for delta accounting**

```bash
wc -l crates/resource/docs/*.md crates/resource/docs/*.rs
```

Expected (per kickoff brief audit):

```
   316 crates/resource/docs/Architecture.md
   370 crates/resource/docs/Pooling.md
   292 crates/resource/docs/README.md
   414 crates/resource/docs/adapters.md
   869 crates/resource/docs/api-reference.md
    90 crates/resource/docs/events.md
   165 crates/resource/docs/recovery.md
  1012 crates/resource/docs/dx-eval-real-world.rs
  3528 total
```

Save numbers somewhere (in your scratchpad or PR description draft) — final PR description cites delta.

- [ ] **Step 5: No commit**

Pre-flight is observation only. Proceed to Task 1.

---

## Task 1 — Capture API ground truth (working artefact, not committed)

**Files:**
- Create (transient): `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/p4-api-surface.md`

**Why:** Tasks 4–6 each rewrite a doc file. Without a single source of truth captured up front, each subagent re-walks the same source files and risks divergent claims (e.g., one says Manager has 10 register methods, another says 11). This artefact is the canonical handoff: every subagent reads it once and grounds their writes in it.

The file is committed to `drafts/` (not `plans/`) so it lives alongside the cascade artefacts; it is NOT part of the public docs surface.

- [ ] **Step 1: Create the surface file**

Write `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/p4-api-surface.md` with the following content:

````markdown
# П4 — Canonical API surface snapshot (post-П3, pre-П4)

**Source-of-truth ledger for the doc-rewrite subagents.** Every claim made in
`docs/api-reference.md` / `docs/adapters.md` / `docs/events.md` MUST trace
back to a line range in this file (or to the source files this file cites).

If subagent finds drift between this file and `crates/resource/src/`, the
**source wins** — update this ledger and re-issue the contradiction back via
the review.

Captured at: branch `claude/resource-p4-doc-rewrite` HEAD = `<commit-sha>`
(record from `git rev-parse HEAD` after Task 0 step 2).

---

## Public re-export surface (`crates/resource/src/lib.rs`)

```text
// Top-level types
Cell                                    (cell.rs)
ResourceContext                         (context.rs)
Error, ErrorKind, ErrorScope            (error.rs)
RefreshOutcome, RevokeOutcome, RotationOutcome  (error.rs)
ResourceEvent                           (events.rs — 12 variants, see below)
HasResourcesExt                         (ext.rs)
ResourceGuard                           (guard.rs)
AcquireResilience, AcquireRetryConfig   (integration.rs)

// Manager surface
DrainTimeoutPolicy, Manager, ManagerConfig, RegisterOptions,
ResourceHealthSnapshot, ShutdownConfig, ShutdownError, ShutdownReport
                                        (manager/mod.rs + manager/options.rs + manager/shutdown.rs)

// Metrics
OutcomeCountersSnapshot, ResourceOpsMetrics, ResourceOpsSnapshot
                                        (metrics.rs)

// Re-exports from nebula-core
ExecutionId, ResourceKey, ScopeLevel, WorkflowId, resource_key!
                                        (nebula_core)

// Re-exports from nebula-credential (ADR-0036)
Credential, CredentialContext, CredentialId, NoCredential,
NoCredentialState, SchemeGuard          (nebula_credential)

// Macros
ClassifyError, Resource                 (nebula_resource_macros — derives)

// Acquire options
AcquireIntent, AcquireOptions           (options.rs)

// Recovery surface
GateState, RecoveryGate, RecoveryGateConfig, RecoveryGroupKey,
RecoveryGroupRegistry, RecoveryTicket, RecoveryWaiter,
WatchdogConfig, WatchdogHandle          (recovery/)

// Registry
AnyManagedResource, Registry            (registry.rs)
ReleaseQueue                            (release_queue.rs)
ReloadOutcome                           (reload.rs)

// Resource trait surface
AnyResource, MetadataCompatibilityError, Resource,
ResourceConfig, ResourceMetadata        (resource.rs)

// Runtime types (for direct register())
TopologyRuntime                         (runtime/mod.rs — 5 variants)
ExclusiveRuntime, ManagedResource, PoolRuntime, PoolStats,
ResidentRuntime, ServiceRuntime, TransportRuntime
                                        (runtime/{exclusive,managed,pool,resident,service,transport}.rs)

// State
ResourcePhase, ResourceStatus           (state.rs)

// Topology configs (for register_*)
Exclusive, ExclusiveConfig              (topology/exclusive/)
BrokenCheck, InstanceMetrics, Pooled, RecycleDecision, PoolConfig
                                        (topology/pooled/)
Resident, ResidentConfig                (topology/resident/)
Service, TokenMode, ServiceConfig       (topology/service/)
Transport, TransportConfig              (topology/transport/)

// Topology tag (5 variants — POST-П3, no Daemon/EventSource)
TopologyTag                             (topology_tag.rs)
```

## `Resource` trait — actual signature (`crates/resource/src/resource.rs`)

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;
    type Lease: Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static;
    type Credential: Credential;        // ADR-0036 — NOT `type Auth`

    fn key() -> ResourceKey;

    fn create(
        &self,
        config: &Self::Config,
        scheme: &<Self::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        ctx: &'a CredentialContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
        let _ = (new_scheme, ctx);
        async { Ok(()) }
    }

    fn on_credential_revoke(
        &self,
        credential_id: &CredentialId,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = credential_id;
        async { Ok(()) }
    }

    fn check(...) -> ... { /* default: Ok(()) */ }
    fn shutdown(...) -> ... { /* default: no-op */ }
    fn destroy(...) -> ... { /* default: drop */ }

    fn schema() -> nebula_schema::ValidSchema where Self: Sized;
    fn metadata() -> ResourceMetadata where Self: Sized;
}
```

**Lifecycle:** `create → check (periodic) → on_credential_refresh / on_credential_revoke (rotation) → shutdown → destroy`

**ResourceConfig bound:** `: nebula_schema::HasSchema + Send + Sync + Clone + 'static`
(api-reference.md currently MISSING the `HasSchema` super-bound — drift to fix)

**ResourceMetadata shape:** `pub struct ResourceMetadata { pub base: nebula_metadata::BaseMetadata<ResourceKey> }`
(api-reference.md currently shows 4-field {key, name, description, tags} — fabrication to fix)

## `Manager` API — actual surface (`crates/resource/src/manager/mod.rs`)

| Section | Methods |
|---|---|
| Construction | `new()`, `with_config(ManagerConfig)` |
| Subscribe | `subscribe_events() -> broadcast::Receiver<ResourceEvent>` |
| Register (full) | `register<R: Resource>(resource, config, scope, topology, options: RegisterOptions)` |
| Register (no-credential convenience, default options) | `register_pooled<R>`, `register_resident<R>`, `register_service<R>`, `register_exclusive<R>`, `register_transport<R>` (5 methods, all bound `Credential = NoCredential`) |
| Register (full options, supports credentials) | `register_pooled_with<R>`, `register_resident_with<R>`, `register_service_with<R>`, `register_transport_with<R>`, `register_exclusive_with<R>` (5 methods, take `RegisterOptions { credential_id, resilience, recovery_gate, ... }`) |
| Lookup | `lookup<R>(scope) -> Arc<ManagedResource<R>>`, `contains(key)`, `keys()`, `get_any(key, scope)` |
| Acquire (full, requires `scheme` arg) | `acquire_pooled<R>`, `acquire_resident<R>`, `acquire_service<R>`, `acquire_transport<R>`, `acquire_exclusive<R>` (5 methods) |
| Acquire (NoCredential default — no `scheme` arg) | `acquire_pooled_default<R>`, `acquire_resident_default<R>`, `acquire_service_default<R>`, `acquire_transport_default<R>`, `acquire_exclusive_default<R>` (5 methods, bound `Credential = NoCredential`) |
| Reload / remove | `reload_config<R>(new_config, scope)`, `remove(key)` |
| Shutdown | `shutdown()` (immediate cancel), `graceful_shutdown(ShutdownConfig) -> ShutdownReport` |
| Observability accessors | `metrics() -> Option<&ResourceOpsMetrics>`, `recovery_groups() -> &RecoveryGroupRegistry`, `cancel_token() -> &CancellationToken`, `is_shutdown() -> bool` |
| Rotation (called by engine, not directly by users) | `on_credential_refreshed<C>(...)`, `on_credential_revoked(...)` (in `manager/rotation.rs`) |

**Total:** 11 register methods (1 full + 5 + 5), 10 acquire methods (5 + 5), plus the rest.

## `ResourceEvent` enum — actual 12 variants (`crates/resource/src/events.rs`)

```rust
#[non_exhaustive]
pub enum ResourceEvent {
    Registered { key: ResourceKey },
    Removed { key: ResourceKey },
    AcquireSuccess { key: ResourceKey, duration: Duration },
    AcquireFailed { key: ResourceKey, error: String },
    Released { key: ResourceKey, held: Duration, tainted: bool },
    HealthChanged { key: ResourceKey, healthy: bool },
    ConfigReloaded { key: ResourceKey },
    RetryAttempt { key: ResourceKey, attempt: u32, backoff: Duration, error: String },
    BackpressureDetected { key: ResourceKey },
    RecoveryGateChanged { key: ResourceKey, state: String },
    CredentialRefreshed { credential_id: CredentialId, resources_affected: usize, outcome: RotationOutcome },
    CredentialRevoked   { credential_id: CredentialId, resources_affected: usize, outcome: RotationOutcome },
}

impl ResourceEvent {
    pub fn key(&self) -> Option<&ResourceKey>;  // returns None for Credential* variants
}
```

## `TopologyTag` enum — actual 5 variants (`crates/resource/src/topology_tag.rs`)

```rust
#[non_exhaustive]
pub enum TopologyTag { Pool, Resident, Service, Transport, Exclusive }

impl TopologyTag {
    pub fn as_str(self) -> &'static str;  // "pool", "resident", "service", "transport", "exclusive"
}
```

(Daemon and EventSource were extracted to `nebula_engine::daemon` per ADR-0037 / П3 — they are NOT in `nebula-resource` anymore.)

## `TopologyRuntime` enum — actual 5 variants (`crates/resource/src/runtime/mod.rs`)

```rust
pub enum TopologyRuntime<R: Resource> {
    Pool(PoolRuntime<R>),
    Resident(ResidentRuntime<R>),
    Service(ServiceRuntime<R>),
    Transport(TransportRuntime<R>),
    Exclusive(ExclusiveRuntime<R>),
}
```

## Drift to FIX in api-reference.md (R-030)

- `TopologyTag` listed with 7 variants → FIX to 5
- `TopologyRuntime` listed with 7 variants → FIX to 5
- Whole "EventSource" + "Daemon" trait sections → REMOVE (link to `nebula_engine::daemon` instead)
- `EventSourceConfig` + `DaemonConfig` rows in topology configs table → REMOVE
- `ResourceEvent` enum listed with 7 variants → REPLACE with full 12
- `ResourceEvent::key()` returns `&ResourceKey` → FIX to `Option<&ResourceKey>`
- `ResourceMetadata` shown as 4-field `{key, name, description, tags}` → REPLACE with `{ base: BaseMetadata<ResourceKey> }`
- `ResourceMetadata::from_key` example → keep
- `ResourceConfig` super-bound `Send + Sync + Clone + 'static` → ADD `nebula_schema::HasSchema +` prefix
- `Resource` trait section missing `on_credential_refresh` + `on_credential_revoke` → ADD per ADR-0036 §Decision
- `Resource::create` signature uses `auth: &R::Auth` (or `()`) → REPLACE with `scheme: &<Self::Credential as Credential>::Scheme`
- `Manager::register_*` listed at 4 methods → EXPAND to 11 (1 full + 5 + 5)
- `Manager::acquire_*` listed at 5 methods → EXPAND to 10 (5 + 5)
- `ResourceContext::with_scope` and `::with_cancel_token` → these DO NOT EXIST. The actual API is `ResourceContext::new(execution_id) -> Self` and capability traits (`HasResources`, `HasCredentials`); REMOVE the with_* builders.
- `AcquireCircuitBreakerPreset` → does not exist. `AcquireResilience` does NOT include a `circuit_breaker` field. REMOVE the preset enum and the `circuit_breaker` field; document only `timeout` + `retry`.
- `register` full signature lists `resilience: Option<AcquireResilience>` and `recovery_gate: Option<Arc<RecoveryGate>>` as positional args → ACTUAL signature passes both via `RegisterOptions { resilience, recovery_gate, credential_id, .. }`. REWRITE.

## Drift to FIX in adapters.md (R-031)

- Step 5 example uses `acquire_pooled::<R>(&(), &ctx, &opts)` — verify against current signature; `()` is the `<NoCredential as Credential>::Scheme` for opt-out. Either keep with explanation OR switch to `acquire_pooled_default::<R>(&ctx, &opts)` (cleaner; matches ADR-0036 idiom).
- `Pooled::prepare` is documented in current adapters.md but **does not exist** on the `Pooled` trait today (verify against `crates/resource/src/topology/pooled/mod.rs`). REMOVE if absent.
- `Resource::Credential = NoCredential` line in checklist — verify still applicable wording.
- `ClassifyError` syntax block uses `#[classify(transient)]`, `#[classify(permanent)]`, `#[classify(exhausted(retry_after_secs))]` — verify against `crates/resource-macros/src/`. The current attr `exhausted(retry_after_secs)` references a *field name* of the enum variant; this is correct usage, but verify error message clarity.
- Step 1 `validate` rule "Reject every bad field, not just the first" — actual `Error` doesn't have a multi-field aggregator built-in. Soften to "Validate format and bounds before connectivity is attempted; connectivity belongs in `create`."

## Drift to FIX in events.md (R-035)

- Variant table currently lists 9 (Registered, Removed, AcquireSuccess, AcquireFailed, Released, HealthChanged, ConfigReloaded, CredentialRefreshed, CredentialRevoked) → ADD 3 (RetryAttempt, BackpressureDetected, RecoveryGateChanged)
- "Per-resource variants carry a key: ResourceKey accessible via event.key() (returns Some)" — drift: ALL variants except CredentialRefreshed/Revoked carry a key. Adjust phrasing if ambiguous.
- Aggregate variant doc claims `outcome: RotationOutcome` — true, but link to `RotationOutcome` from `crate::error::RotationOutcome` for navigation (it's not on `ResourceEvent` but on `Error`).

## Drift to FIX in README.md (R-033)

- Topology Decision Guide (lines 30-43) lists Daemon and EventSource as "secondary topology" — REMOVE both rows; they live in `nebula_engine::daemon` now.
- Crate Layout block (lines 248-279) lists v1 module names: `manager.rs` (now `manager/`), `registry.rs` (still exists), `integration.rs` (still exists), but no longer matches actual tree (e.g., manager is now a directory). UPDATE.
- Documentation table (lines 285-292) lists 6 doc files; 4 of them have wrong references:
  - `architecture.md` — file is `Architecture.md`, but per Task 2 the file is being DELETED → REMOVE the row entirely
  - `pooling.md` — file is `Pooling.md` (case mismatch, currently broken on case-sensitive filesystems); fix in Task 7 by renaming `Pooling.md` → `pooling.md`
  - `events-and-hooks.md` — file is `events.md`; FIX reference
  - `health-and-quarantine.md` — file is `recovery.md`; FIX reference + retitle
- Line 89 `type Credential = NoCredential;` — already correct from П1.
- Line 113 `recycle()` example uses `impl Future<...>` syntax — verify still matches actual `Pooled::recycle` signature (it does).

## Drift to FIX in Pooling.md (verify pass)

- Module-level intro (lines 1-7) names `Pool<R>` — actual public type is `PoolRuntime<R>`. Either rename or clarify "the pool runtime, exposed via `nebula_resource::PoolRuntime`".
- "How the Pool Works" diagram (lines 26-35): `idle_queue: VecDeque<IdleEntry<R::Instance>>` — `R::Instance` is not a `Resource` associated type; should be `R::Runtime`. Verify and fix.
- `PoolConfig` field list (lines 60-74) — verify against `crates/resource/src/topology/pooled/config.rs`. Likely close but check field-by-field.
- "Atomic operation counters" / "AutoScaler" mentions — AutoScaler is a v1 concept; verify removed.
````

- [ ] **Step 2: Commit Task 1**

```bash
git add docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/p4-api-surface.md
git commit -m "docs(superpowers): capture canonical API surface for П4 doc rewrite

Working artefact for the П4 doc-rewrite subagents — pins the public
surface against post-П3 source (Resource trait per ADR-0036, 5-variant
TopologyTag/TopologyRuntime, 12-variant ResourceEvent, 11 register +
10 acquire Manager methods). Lists per-doc drift to correct.

Refs: cascade Phase 8 deferred to П4; closes nothing yet (R-030..R-035
land per-task)"
```

---

## Task 2 — Delete `Architecture.md` and `dx-eval-real-world.rs`

**Files:**
- Delete: `crates/resource/docs/Architecture.md`
- Delete: `crates/resource/docs/dx-eval-real-world.rs`
- Modify: `crates/resource/docs/README.md` (remove the `architecture.md` documentation-table row)

**Why:** Both files are 100% v1 fabrications and net negatives:
- `Architecture.md` describes a module map that no longer exists (HookRegistry, QuarantineManager, EventBus, AutoScaler, Poison, DependencyGraph, lifecycle states like `Maintenance`/`Cleanup` that don't exist in `crate::state::ResourcePhase`). README.md "Crate Layout" + canon docs cover real architecture.
- `dx-eval-real-world.rs` is a 1012-LOC non-compiling design eval whose friction commentary references concerns now resolved in П1 (`Credential = ()` retired in favor of `NoCredential`; `register_pooled_with` exists). Its stated purpose ("design-only evaluation, does NOT compile") is no longer needed.

This task is the cheapest LOC-delta wins (-1328 LOC) and unblocks the README fix in Task 7 (one less broken-link source).

- [ ] **Step 1: Delete the two files**

```bash
git rm crates/resource/docs/Architecture.md
git rm crates/resource/docs/dx-eval-real-world.rs
```

Expected: both files removed; `git status` shows two deletions staged.

- [ ] **Step 2: Remove the `architecture.md` row from README.md documentation table**

The table at `crates/resource/docs/README.md:285-292` currently has the row:

```markdown
| [`architecture.md`](architecture.md) | Module dependency map, data flow, layer invariants |
```

Remove that single row. Leave the other rows in place (Task 7 fixes them properly).

```bash
# Use Edit tool — old_string includes only the single row + the line break before it
```

Edit the file: replace
```text
| [`architecture.md`](architecture.md) | Module dependency map, data flow, layer invariants |
| [`api-reference.md`](api-reference.md) | Every public type, trait, and method with signatures |
```
with
```text
| [`api-reference.md`](api-reference.md) | Every public type, trait, and method with signatures |
```

- [ ] **Step 3: Run baseline gates**

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps 2>&1 | tail -10
```

Expected: clean. (If a `//!` doc in source code linked to the deleted Architecture.md, that link would warn — but per the П3 plan grep evidence, no source files cite `Architecture.md`.)

```bash
cargo nextest run -p nebula-resource --profile ci --no-tests=pass 2>&1 | tail -5
```

Expected: same test count as before. (Deletions affect no test build.)

- [ ] **Step 4: Commit Task 2**

```bash
git add crates/resource/docs/README.md  # the documentation-table edit
git commit -m "docs(resource)!: retire Architecture.md and dx-eval-real-world.rs (R-032, R-034)

Both files are v1 fabrications:

- Architecture.md (316 LOC) describes a module map that no longer
  exists (HookRegistry, QuarantineManager, EventBus, AutoScaler,
  Poison, DependencyGraph). The real architectural overview lives in
  README.md (Crate Layout + Core Concepts) and PRODUCT_CANON.md §4.5
  / §11.4. A slim replacement would drift the same way.
- dx-eval-real-world.rs (1012 LOC) is a non-compiling design eval
  whose friction commentary is obsolete after П1
  (Credential = () retired in favor of NoCredential; register_*_with
  exists). README Quick Start + adapters.md cover the same use cases
  with working examples.

README documentation table loses the architecture.md row; other rows
fixed in a separate commit per file-by-file scope.

Refs: cascade Strategy §4.7 (rewrite OR delete); register R-032, R-034"
```

The `!` BREAKING marker in the type is conservative — these are docs but readers may have linked to them. Add a `BREAKING CHANGE:` footer if convco is strict; otherwise it's a chore-level retirement.

---

## Task 3 — Rebuild `events.md` against the actual 12-variant `ResourceEvent`

**Files:**
- Rewrite: `crates/resource/docs/events.md` (90 → ~150 LOC)

**Why:** Smallest doc; confidence-builder before the bigger rewrites. Current content lists 9 variants (drift R-035 underestimated — the kickoff brief said "7 vs 10"; actual is 12). The 3 missing entries are observability hot-paths that П2 already wired up (RetryAttempt, BackpressureDetected, RecoveryGateChanged).

The document also has subtle drift in the `event.key()` description (current text says it "returns Some" for per-resource events, but `key() -> Option<&ResourceKey>` returns None for the Credential* aggregate variants — must clarify).

Plus: post-П2 the rotation-cycle aggregate semantics need a paragraph explaining `RotationOutcome` (the `outcome.total()` invariant, per-resource health-event interplay).

- [ ] **Step 1: Read current `events.md` end-to-end**

```bash
cat crates/resource/docs/events.md
```

Familiarise with current section layout (Overview / Event Catalog table / Usage Patterns / Differences from v1).

- [ ] **Step 2: Read actual source**

Read `crates/resource/src/events.rs` lines 1-153 to confirm:
1. 12 enum variants (Registered, Removed, AcquireSuccess, AcquireFailed, Released, HealthChanged, ConfigReloaded, RetryAttempt, BackpressureDetected, RecoveryGateChanged, CredentialRefreshed, CredentialRevoked)
2. `key() -> Option<&ResourceKey>` returns `None` for `CredentialRefreshed` / `CredentialRevoked` (they have `credential_id` instead)
3. `RotationOutcome` is exported from `crate::error` (also re-exported as `nebula_resource::RotationOutcome`)
4. `#[non_exhaustive]` on the enum

Also verify `subscribe_events` returns `tokio::sync::broadcast::Receiver<ResourceEvent>` and the buffer size (broadcast capacity at construction in `manager/mod.rs`).

- [ ] **Step 3: Replace `events.md` with the rebuilt content**

Write `crates/resource/docs/events.md` (~150 LOC) with this structure:

```markdown
# Events

Lifecycle event system for observability and diagnostics.

---

## Overview

The [`Manager`] emits [`ResourceEvent`]s on every significant lifecycle
transition. Events are broadcast via a `tokio::sync::broadcast` channel —
see `Manager::subscribe_events()` for the receiver type.

Subscribe with [`Manager::subscribe_events()`]:

```rust,ignore
let mut rx = manager.subscribe_events();
tokio::spawn(async move {
    while let Ok(event) = rx.recv().await {
        tracing::info!(?event, "resource lifecycle event");
    }
});
```

The channel buffer is fixed at construction; slow consumers receive
[`tokio::sync::broadcast::error::RecvError::Lagged`] when they fall
behind — see [Slow consumers](#slow-consumers) below for handling.

---

## Event Catalog

Twelve `#[non_exhaustive]` variants; new variants may be added in minor
releases without bumping the major version.

### Per-resource variants (10)

| Variant | Emitted when | Key fields |
|---------|--------------|------------|
| `Registered` | A resource is registered with the manager | `key` |
| `Removed` | A resource is removed from the registry | `key` |
| `AcquireSuccess` | A handle is acquired | `key`, `duration` |
| `AcquireFailed` | An acquire returns an error | `key`, `error: String` |
| `Released` | A handle is dropped (returned/destroyed) | `key`, `held: Duration`, `tainted: bool` |
| `HealthChanged` | The resource's health status flips | `key`, `healthy: bool` |
| `ConfigReloaded` | `Manager::reload_config` succeeded for this key | `key` |
| `RetryAttempt` | A transient acquire failure is about to be retried | `key`, `attempt: u32`, `backoff: Duration`, `error: String` |
| `BackpressureDetected` | Pool semaphore signalled saturation | `key` |
| `RecoveryGateChanged` | A recovery gate transitioned (Idle ↔ InProgress ↔ Failed ↔ PermanentlyFailed) | `key`, `state: String` |

### Aggregate (rotation cycle) variants (2)

Emitted by `Manager::on_credential_refreshed` / `_revoked` after every
per-resource dispatch future has completed (see Tech Spec §6.2).

| Variant | Emitted when | Key fields |
|---------|--------------|------------|
| `CredentialRefreshed` | One refresh-cycle fan-out is complete | `credential_id`, `resources_affected: usize`, `outcome: RotationOutcome` |
| `CredentialRevoked` | One revoke-cycle fan-out is complete | `credential_id`, `resources_affected: usize`, `outcome: RotationOutcome` |

`outcome.total()` always equals `resources_affected`. Per-resource
revocation failures are also signalled inline as `HealthChanged { healthy:
false }` (security amendment B-2 from cascade Phase 6 CP2 review), so
subscribers that miss the aggregate event still see per-resource failure
events.

`RotationOutcome` is `nebula_resource::RotationOutcome` — see
[`RotationOutcome`] for the `ok` / `failed` / `timed_out` count breakdown.

---

## Reading the resource key

Per-resource variants carry a `key: ResourceKey`; the aggregate rotation
variants do not (they span multiple resources). Use the convenience
accessor:

```rust,ignore
fn key(&self) -> Option<&ResourceKey>
```

Returns `Some(&key)` for the 10 per-resource variants; returns `None` for
`CredentialRefreshed` and `CredentialRevoked` — for those, read the
`credential_id` field directly to identify the rotation.

---

## Usage Patterns

### Metrics collection

```rust,ignore
while let Ok(event) = rx.recv().await {
    match &event {
        ResourceEvent::AcquireSuccess { duration, .. } => {
            histogram.record(duration.as_millis() as f64);
        }
        ResourceEvent::AcquireFailed { error, .. } => {
            counter.increment(1);
            tracing::warn!(%error, "acquire failed");
        }
        ResourceEvent::RetryAttempt { attempt, backoff, .. } => {
            tracing::info!(attempt, ?backoff, "retrying transient acquire");
        }
        _ => {}
    }
}
```

### Slow consumers

Slow consumers receive
[`tokio::sync::broadcast::error::RecvError::Lagged(n)`] when the channel
overruns — *n* events were dropped. Handle it:

```rust,ignore
match rx.recv().await {
    Ok(event) => handle(event),
    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
        tracing::warn!(dropped = n, "event consumer lagged");
    }
    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
}
```

The broadcast channel drops oldest events on overflow — there is no
back-pressure or retry mechanism inside the Manager.

### Auditing rotation cycles

```rust,ignore
while let Ok(event) = rx.recv().await {
    match event {
        ResourceEvent::CredentialRefreshed { credential_id, outcome, .. } => {
            metrics::credential_rotation_attempts(&credential_id)
                .increment(outcome.total() as u64);
            if outcome.failed > 0 || outcome.timed_out > 0 {
                tracing::warn!(?credential_id, ?outcome, "rotation had partial failures");
            }
        }
        ResourceEvent::CredentialRevoked { credential_id, outcome, .. } => {
            audit::record_revocation(&credential_id, outcome.total());
        }
        _ => {}
    }
}
```

---

## Differences from v1

- **No `EventBus`** — events come directly from `Manager::subscribe_events()`;
  no separate bus crate, no subscriber registration, no priority/ordering layer.
- **No `HookRegistry`** — pre/post hooks were removed. Use events for observation.
- **No filtered subscriptions** — filter in your consumer logic.
- **No `BackPressurePolicy`** — the broadcast channel drops oldest on overflow.

---

[`Manager`]: crate::manager::Manager
[`Manager::subscribe_events()`]: crate::manager::Manager::subscribe_events
[`ResourceEvent`]: crate::events::ResourceEvent
[`RotationOutcome`]: crate::error::RotationOutcome
```

The `[`Manager`]: crate::manager::Manager` style at the bottom is intra-doc-link reference syntax; verify rendered output below.

- [ ] **Step 4: Verify rendered docs**

```bash
RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-resource --no-deps 2>&1 | tail -20
```

Expected: clean. Markdown intra-doc links must resolve (the references at the bottom of `events.md` are inside the crate, so rustdoc can resolve them when the file is included via `#![doc = include_str!("../docs/events.md")]` if it is — verify whether `events.md` is currently included via `include_str!` in `lib.rs` or `events.rs`).

If `events.md` is NOT included via `include_str!`, the bracketed-but-not-resolved links don't fail the build (since rustdoc never sees the file). In that case the links are still valid for human readers via GitHub.

```bash
grep -rn 'include_str!("../docs/events.md")' crates/resource/src/
grep -rn 'include_str!("../docs/events.md")' crates/resource/
```

If included → the rustdoc gate proves the links work. If not → leave them; they render correctly on GitHub.

- [ ] **Step 5: Commit Task 3**

```bash
git add crates/resource/docs/events.md
git commit -m "docs(resource): rebuild events.md against 12-variant ResourceEvent (R-035)

Current events.md catalogued 9 variants; actual ResourceEvent has 12.
Adds RetryAttempt, BackpressureDetected, RecoveryGateChanged
(observability hot-paths landed in П2). Splits the catalog into
'Per-resource' (10 variants) and 'Aggregate' (2 rotation variants)
sections so callers know which carry a key vs which carry a
credential_id.

Clarifies event.key() semantics:
- Returns Some(key) for per-resource variants
- Returns None for CredentialRefreshed / CredentialRevoked
  (use credential_id field instead)

Adds rotation-audit usage pattern citing RotationOutcome.total()
invariant + the per-resource HealthChanged interplay (security
amendment B-2 from cascade Phase 6 CP2 review).

Refs: register R-035; ADR-0036; cascade Tech Spec §6.2"
```

---

## Task 4 — Rewrite `api-reference.md` from source

**Files:**
- Rewrite: `crates/resource/docs/api-reference.md` (869 → ~600 LOC)

**Why:** This is the largest single rewrite — the ~50% fabrication concern (R-030). Current document was authored against the v1 + early-cascade trait shape and never re-grounded. Drifts to fix were enumerated in Task 1 (`p4-api-surface.md` "Drift to FIX in api-reference.md" section).

The rewrite must be **source-grounded for every type and signature** — no claim survives without a corresponding line in `crates/resource/src/`. Subagents working this task should keep the Task 1 surface file open as a checklist.

- [ ] **Step 1: Read the Task 1 surface file end-to-end**

```bash
cat docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/p4-api-surface.md
```

Internalize the canonical surface before writing. Your job is to render the surface as `api-reference.md` user docs.

- [ ] **Step 2: Read all referenced source files in one pass**

Open and skim:

- `crates/resource/src/lib.rs` — top-level re-exports
- `crates/resource/src/resource.rs` — `Resource`, `ResourceConfig`, `ResourceMetadata`, `ResourceMetadataBuilder`, `AnyResource`, `MetadataCompatibilityError`
- `crates/resource/src/manager/mod.rs` — Manager constructors + 11 register + 10 acquire methods
- `crates/resource/src/manager/options.rs` — `ManagerConfig`, `RegisterOptions`, `ShutdownConfig`, `DrainTimeoutPolicy`
- `crates/resource/src/manager/shutdown.rs` — `ShutdownReport`, `ShutdownError`
- `crates/resource/src/error.rs` — `Error`, `ErrorKind`, `ErrorScope`, `RefreshOutcome`, `RevokeOutcome`, `RotationOutcome`
- `crates/resource/src/events.rs` — `ResourceEvent` (link to events.md, don't duplicate the table)
- `crates/resource/src/options.rs` — `AcquireOptions`, `AcquireIntent`
- `crates/resource/src/integration.rs` — `AcquireResilience`, `AcquireRetryConfig` (NO `AcquireCircuitBreakerPreset` — it doesn't exist; verify and remove)
- `crates/resource/src/recovery/{gate,group,watchdog,waiter}.rs` (paths approx — verify) — `RecoveryGate`, `RecoveryTicket`, `GateState`, `RecoveryGateConfig`, `RecoveryGroupRegistry`, `RecoveryGroupKey`, `RecoveryWaiter`, `WatchdogHandle`, `WatchdogConfig`
- `crates/resource/src/state.rs` — `ResourcePhase`, `ResourceStatus`
- `crates/resource/src/runtime/mod.rs` + each topology runtime — `TopologyRuntime`, `PoolRuntime`, `PoolStats`, etc.
- `crates/resource/src/topology/{pooled,resident,service,transport,exclusive}/{mod,config}.rs` — topology trait signatures + config structs
- `crates/resource/src/topology_tag.rs` — `TopologyTag` (5 variants)
- `crates/resource/src/guard.rs` — `ResourceGuard` (`Owned` / `Guarded` / `Shared`)
- `crates/resource/src/cell.rs` — `Cell`
- `crates/resource/src/release_queue.rs` — `ReleaseQueue`, `ReleaseQueueHandle`
- `crates/resource/src/registry.rs` — `Registry`, `AnyManagedResource`
- `crates/resource/src/metrics.rs` — `ResourceOpsMetrics`, `ResourceOpsSnapshot`, `OutcomeCountersSnapshot`
- `crates/resource/src/context.rs` — `ResourceContext`

For each item, capture: pub signature, super-bounds, default body presence, panic-safety guarantees if any.

- [ ] **Step 3: Replace `api-reference.md`**

Write the new file with this top-level structure (target ~600 LOC; do not exceed without justification):

```markdown
# nebula-resource — API Reference (post-cascade)

Complete public API reference. All types are in `nebula_resource` unless
noted. Re-exported from `nebula_core`: `ExecutionId`, `ResourceKey`,
`ScopeLevel`, `WorkflowId`, `resource_key!`. Re-exported from
`nebula_credential` (per ADR-0036): `Credential`, `CredentialContext`,
`CredentialId`, `NoCredential`, `NoCredentialState`, `SchemeGuard`.

---

## Table of Contents

- [Core Traits](#core-traits)
- [Topology Traits](#topology-traits)
- [Topology Configs](#topology-configs)
- [Handle](#handle)
- [Manager](#manager)
- [Manager Options](#manager-options)
- [Error Model](#error-model)
- [Context](#context)
- [Acquire Options](#acquire-options)
- [Resilience](#resilience)
- [Recovery](#recovery)
- [Events](#events)
- [Metrics](#metrics)
- [State](#state)
- [Runtime Types](#runtime-types)
- [Utilities](#utilities)

---

## Core Traits

### `Resource`

The central abstraction. Five associated types and six lifecycle methods.
Uses RPITIT (`impl Future`) — no `Box<dyn Future>` overhead.

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;
    type Lease: Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static;
    type Credential: Credential;        // ADR-0036; use NoCredential to opt out

    fn key() -> ResourceKey;

    fn create(
        &self,
        config: &Self::Config,
        scheme: &<Self::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    // Default no-op rotation hooks (override for credential-bound resources):
    fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        ctx: &'a CredentialContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a;

    fn on_credential_revoke(
        &self,
        credential_id: &CredentialId,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    // Default no-op lifecycle hooks:
    fn check(&self, runtime: &Self::Runtime)    -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn shutdown(&self, runtime: &Self::Runtime) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn destroy(&self, runtime: Self::Runtime)   -> impl Future<Output = Result<(), Self::Error>> + Send;

    // Schema/metadata defaults derive from Config:
    fn schema()   -> nebula_schema::ValidSchema where Self: Sized;
    fn metadata() -> ResourceMetadata          where Self: Sized;
}
```

**Lifecycle:** `create → check (periodic) → on_credential_{refresh,revoke}
(rotation) → shutdown → destroy`

If `Runtime == Lease`, the blanket `impl From<T> for T` satisfies the
conversion bounds used by `Pooled`/`Resident`/`Exclusive` topologies.

#### `Credential` opt-out

Resources that don't bind to an authenticated identity write
`type Credential = NoCredential;`. The `<NoCredential as Credential>::Scheme`
is `()`, and the runtime threads `&()` into `create`. Per ADR-0036, this
is the canonical "no-auth" idiom — there is no separate `Auth` associated
type anymore.

---

### `ResourceConfig`

Operational configuration. **No secrets** — credential material flows
through `Credential` instead.

```rust
pub trait ResourceConfig:
    nebula_schema::HasSchema + Send + Sync + Clone + 'static
{
    fn validate(&self) -> Result<(), Error> { Ok(()) }
    fn fingerprint(&self) -> u64 { 0 }
}
```

The `nebula_schema::HasSchema` super-bound is required so
`ResourceMetadata::for_resource` can auto-derive a schema from the config
type. Use `()` / `bool` / `String` for schema-less stubs (baseline impls
in `nebula-schema` cover primitives with empty schemas).

`fingerprint` enables config hot-reload via `Manager::reload_config`. Two
configs with the same non-zero fingerprint are treated as identical.

---

### `ResourceMetadata`

UI/diagnostic descriptor. The shared catalog prefix (`key`, `name`,
`description`, `schema`, `version`, `tags`, etc.) lives on the composed
[`BaseMetadata`](nebula_metadata::BaseMetadata).

```rust
#[non_exhaustive]
pub struct ResourceMetadata {
    pub base: nebula_metadata::BaseMetadata<ResourceKey>,
}

impl ResourceMetadata {
    pub fn new(key, name, description, schema) -> Self;
    pub fn from_key(key: &ResourceKey) -> Self;
    pub fn for_resource<R: Resource>(key, name, description) -> Self;  // schema from R::Config
    pub fn builder(key, name, description) -> ResourceMetadataBuilder;
    pub fn validate_compatibility(&self, previous: &Self) -> Result<(), MetadataCompatibilityError>;
}
```

`validate_compatibility` enforces the catalog-citizen rules (`key
immutable / version monotonic / schema-break-requires-major`); see
`MetadataCompatibilityError`.

---

### `AnyResource`

Trait-object-safe marker for type-erased resource registration.

```rust
pub trait AnyResource: Send + Sync + 'static {
    fn key(&self) -> ResourceKey;
    fn metadata(&self) -> ResourceMetadata;
}
```

Implementors of `Resource` typically implement `AnyResource` via the
`#[derive(Resource)]` macro (which auto-implements both).

---

## Topology Traits

Topology traits extend `Resource` with lifecycle hooks specific to how
instances are managed. Register the matching runtime via
`TopologyRuntime`.

### `Pooled` — N interchangeable instances

```rust
pub trait Pooled: Resource {
    fn is_broken(&self, runtime: &Self::Runtime) -> BrokenCheck { Healthy }
    fn recycle(&self, runtime: &Self::Runtime, metrics: &InstanceMetrics)
        -> impl Future<Output = Result<RecycleDecision, Self::Error>> + Send { /* default Keep */ }
}
```

| Type | Purpose |
|------|---------|
| `BrokenCheck` | Sync O(1) result from `is_broken`: `Healthy` or `Broken(String)` |
| `RecycleDecision` | Async recycle outcome: `Keep` (return to pool) or `Drop` (destroy) |
| `InstanceMetrics` | `error_count`, `checkout_count`, `created_at` — available to `recycle` |

`is_broken` runs in the `Drop` path — no async, no I/O. Acquire bounds:
`R: Clone`, `R::Runtime: Clone + Into<R::Lease>`, `R::Lease: Into<R::Runtime>`.

(Verify whether `prepare` exists today in `crates/resource/src/topology/pooled/mod.rs`. If absent, do NOT document it. If present, document with the exact signature.)

### `Resident` — one shared instance, clone on acquire
### `Service` — long-lived runtime, short-lived tokens
### `Transport` — shared connection, multiplexed sessions
### `Exclusive` — one caller at a time

(Each section: signature with default bodies, acquire bounds, link to
matching `*Config` and `*Runtime`. ~30-40 LOC each.)

> **Note: secondary topologies.** `Daemon` and `EventSource` were
> extracted to `nebula_engine::daemon` per ADR-0037. See the
> `nebula-engine` documentation for `DaemonRegistry`, `EventSourceAdapter<E>`,
> and lifecycle. They are NOT part of the `nebula-resource` public surface
> anymore.

---

## Topology Configs

Each config is the `Config` type alias exported from `nebula_resource`.

| Type | Key fields | Defaults |
|------|-----------|----------|
| `PoolConfig` | … | … |
| `ResidentConfig` | … | … |
| `ServiceConfig` | … | … |
| `TransportConfig` | … | … |
| `ExclusiveConfig` | … | … |

(**Important:** rows for `EventSourceConfig` and `DaemonConfig` MUST NOT
appear — those types live in `nebula-engine` now.)

---

## Handle

### `ResourceGuard<R>`

The value callers hold while using a resource. Annotated `#[must_use]` —
dropping immediately releases the resource back to the topology.

Three ownership modes (`Owned` / `Guarded` / `Shared`); same as the
current doc, with verified constructor signatures and panic-safety prose.

---

## Manager

### `Manager`

Central registry and lifecycle manager. Share via `Arc<Manager>`.

#### Construction

```rust
impl Manager {
    pub fn new() -> Self;
    pub fn with_config(config: ManagerConfig) -> Self;
}
```

#### Subscribe to events

```rust
pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<ResourceEvent>;
```

See [`docs/events.md`](events.md) for the catalog and usage patterns.

#### Register

The full-control method:

```rust
pub fn register<R: Resource>(
    &self,
    resource: R,
    config: R::Config,
    scope: ScopeLevel,
    topology: TopologyRuntime<R>,
    options: RegisterOptions,
) -> Result<(), Error>;
```

`RegisterOptions` carries the credential binding, resilience, recovery
gate, and per-resource rotation timeout — see [Manager Options](#manager-options).

Five no-credential convenience methods (`Credential = NoCredential`,
default `RegisterOptions`, scope `Global`):

```rust
pub fn register_pooled<R: Pooled<Credential = NoCredential>>(
    &self, resource: R, config: R::Config, pool_config: PoolConfig,
) -> Result<(), Error>;

pub fn register_resident<R: Resident<Credential = NoCredential>>(
    &self, resource: R, config: R::Config, resident_config: ResidentConfig,
) -> Result<(), Error>;

pub fn register_service<R: Service<Credential = NoCredential>>(
    &self, resource: R, config: R::Config, runtime: R::Runtime, service_config: ServiceConfig,
) -> Result<(), Error>;

pub fn register_transport<R: Transport<Credential = NoCredential>>(
    &self, resource: R, config: R::Config, runtime: R::Runtime, transport_config: TransportConfig,
) -> Result<(), Error>;

pub fn register_exclusive<R: Exclusive<Credential = NoCredential>>(
    &self, resource: R, config: R::Config, runtime: R::Runtime, exclusive_config: ExclusiveConfig,
) -> Result<(), Error>;
```

Five `_with` variants taking explicit `RegisterOptions` — these are the
path for credential-bound resources, custom scope, attached recovery
gate, or non-default resilience policy:

```rust
pub fn register_pooled_with<R>(
    &self, resource: R, config: R::Config, pool_config: PoolConfig, options: RegisterOptions,
) -> Result<(), Error>
where
    R: Pooled,                          // any Credential, including authenticated
    R::Runtime: Clone + Into<R::Lease>,
    R::Lease: Into<R::Runtime>;
```

(Same shape for `register_resident_with`, `register_service_with`,
`register_transport_with`, `register_exclusive_with` — verify exact
where-bounds against `crates/resource/src/manager/mod.rs:470-647`.)

#### Acquire

For each topology, two methods — the full one taking the credential
scheme, and the `_default` variant for `Credential = NoCredential`
resources:

```rust
pub async fn acquire_pooled<R: Pooled + Clone + ...>(
    &self,
    scheme: &<R::Credential as Credential>::Scheme,
    ctx: &ResourceContext,
    options: &AcquireOptions,
) -> Result<ResourceGuard<R>, Error>;

pub async fn acquire_pooled_default<R: Pooled<Credential = NoCredential> + ...>(
    &self,
    ctx: &ResourceContext,
    options: &AcquireOptions,
) -> Result<ResourceGuard<R>, Error>;
```

(Same shape for `_resident`, `_service`, `_transport`, `_exclusive`.)

#### Reload, remove, shutdown

```rust
pub fn reload_config<R: Resource>(&self, new_config: R::Config, scope: &ScopeLevel) -> Result<(), Error>;
pub fn remove(&self, key: &ResourceKey) -> Result<(), Error>;

pub fn shutdown(&self);                                                   // immediate cancel
pub async fn graceful_shutdown(&self, config: ShutdownConfig) -> Result<ShutdownReport, ShutdownError>;
```

`graceful_shutdown` phases: (1) cancel token → new acquires rejected;
(2) drain in-flight handles up to `drain_timeout`; (3) clear registry;
(4) await release queue workers. The result `ShutdownReport` records
counts per phase; `ShutdownError::AlreadyShuttingDown` if a second
caller races in.

#### Lookup and observability

```rust
pub fn lookup<R: Resource>(&self, scope: &ScopeLevel) -> Result<Arc<ManagedResource<R>>, Error>;
pub fn contains(&self, key: &ResourceKey) -> bool;
pub fn keys(&self) -> Vec<ResourceKey>;
pub fn get_any(&self, key: &ResourceKey, scope: &ScopeLevel) -> Option<Arc<dyn AnyManagedResource>>;

pub fn metrics(&self) -> Option<&ResourceOpsMetrics>;
pub fn recovery_groups(&self) -> &RecoveryGroupRegistry;
pub fn cancel_token(&self) -> &CancellationToken;
pub fn is_shutdown(&self) -> bool;
```

---

## Manager Options

### `ManagerConfig`

```rust
pub struct ManagerConfig { … }
```

(Document fields against `crates/resource/src/manager/options.rs`.)

### `RegisterOptions`

```rust
#[non_exhaustive]
pub struct RegisterOptions {
    pub credential_id: Option<CredentialId>,
    pub resilience: Option<AcquireResilience>,
    pub recovery_gate: Option<Arc<RecoveryGate>>,
    pub credential_rotation_timeout: Option<Duration>,
    pub scope: ScopeLevel,
    /* ... */
}
```

(Verify field list against current source.) Docs the credential reverse-
index population semantics: when `credential_id` is `Some(...)` AND
`R::Credential != NoCredential`, the manager populates the
`credential_resources` reverse-index for rotation fan-out (П2 dispatcher
hooks).

### `ShutdownConfig`, `DrainTimeoutPolicy`, `ShutdownReport`, `ShutdownError`

(Document against `crates/resource/src/manager/{options,shutdown}.rs`.)

---

## Error Model

### `Error`

```rust
pub struct Error { … }

impl Error {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self;
    pub fn transient(message)   -> Self;
    pub fn permanent(message)   -> Self;
    pub fn exhausted(message, retry_after) -> Self;
    pub fn not_found(key)       -> Self;
    pub fn cancelled()          -> Self;
    pub fn backpressure(message) -> Self;
    /* ... */
    pub fn with_resource_key(self, key) -> Self;
    pub fn with_source(self, source)    -> Self;
    pub fn with_scope(self, scope)      -> Self;

    pub fn kind(&self) -> &ErrorKind;
    pub fn scope(&self) -> &ErrorScope;
    pub fn resource_key(&self) -> Option<&ResourceKey>;
    pub fn is_retryable(&self) -> bool;
    pub fn retry_after(&self) -> Option<Duration>;
}
```

### `ErrorKind`, `ErrorScope`, `RefreshOutcome`, `RevokeOutcome`, `RotationOutcome`

(Each gets a small section with the variants and their semantics. The
rotation outcome types are П2's contribution to error.rs — document
them as the rotation cycle's per-resource result type.)

### `ClassifyError` derive macro

Brief — refer to `nebula-resource-macros` rustdoc for full attribute
syntax.

---

## Context

### `ResourceContext`

Concrete execution context passed to all resource lifecycle methods.

```rust
impl ResourceContext {
    pub fn new(execution_id: ExecutionId) -> Self;          // scope = Global
    /* document the actual constructor + accessor surface */
}
```

(**Critical:** verify against `crates/resource/src/context.rs`. The current
api-reference.md cites `with_scope` and `with_cancel_token` builders;
those do NOT exist on the type. Document the actual capability-trait
surface — `HasResources`, `HasCredentials`, etc. — and the actual
constructors.)

---

## Acquire Options

### `AcquireOptions`, `AcquireIntent`

(Document against `crates/resource/src/options.rs`. Note that the
`intent` and `tags` fields are reserved-but-unused in the current
implementation — Strategy §5.2 / Tech Spec §15.2 deprecated them. Add
a sidebar explaining the deprecation and pointing at Tech Spec.)

---

## Resilience

### `AcquireResilience`, `AcquireRetryConfig`

```rust
pub struct AcquireResilience {
    pub timeout: Option<Duration>,
    pub retry: Option<AcquireRetryConfig>,
    /* DO NOT document a circuit_breaker field — it does not exist */
}

impl AcquireResilience {
    pub fn standard() -> Self;
    pub fn fast() -> Self;
    pub fn slow() -> Self;
    pub fn none() -> Self;
}

pub struct AcquireRetryConfig {
    pub max_attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}
```

(**No `AcquireCircuitBreakerPreset`** — that type does not exist in the
crate. Verify by `grep -rn AcquireCircuitBreakerPreset crates/resource/src/`
and remove from docs.)

---

## Recovery

### `RecoveryGate`, `RecoveryTicket`, `RecoveryGateConfig`, `GateState`, `RecoveryWaiter`, `RecoveryGroupRegistry`, `RecoveryGroupKey`, `WatchdogHandle`, `WatchdogConfig`

(Mostly accurate today — verify each signature and prose against
`crates/resource/src/recovery/`. Cross-link to `recovery.md` for
operator-level usage; this section keeps API signatures only.)

---

## Events

See [`docs/events.md`](events.md) for the catalog and usage patterns.

```rust
pub fn subscribe_events(&self) -> tokio::sync::broadcast::Receiver<ResourceEvent>;
```

---

## Metrics

### `ResourceOpsMetrics`, `ResourceOpsSnapshot`, `OutcomeCountersSnapshot`

(`OutcomeCountersSnapshot` was added in П2 alongside `RotationOutcome`;
document it.)

---

## State

### `ResourcePhase`, `ResourceStatus`

(Document the actual `ResourcePhase` variants per
`crates/resource/src/state.rs` — there is no `Maintenance` / `Cleanup`
state, those are v1 fabrications.)

---

## Runtime Types

### `TopologyRuntime<R>`

5 variants (Pool, Resident, Service, Transport, Exclusive). Drop the
`EventSource(EventSourceRuntime<R>)` and `Daemon(DaemonRuntime<R>)`
arms — those types live in `nebula-engine` now.

### `ManagedResource<R>`, `Registry`, `AnyManagedResource`

(Document accurately; verify against `crates/resource/src/runtime/managed.rs`
and `crates/resource/src/registry.rs`.)

---

## Utilities

### `Cell<T>`, `ReleaseQueue`, `ReleaseQueueHandle`, `ReloadOutcome`, `TopologyTag`

`TopologyTag` has 5 variants:

```rust
#[non_exhaustive]
pub enum TopologyTag { Pool, Resident, Service, Transport, Exclusive }
```

(Drop `EventSource` and `Daemon` variants — they were removed in П3.)
```

The above is a STRUCTURAL skeleton — every section's sub-content (field
lists, defaults, panic-safety prose) MUST be filled in against current
source. Target ~600 LOC total.

- [ ] **Step 4: Run rustdoc gate**

```bash
RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-resource --no-deps 2>&1 | tail -30
```

Expected: clean. If `api-reference.md` is `include_str!`'d into the
crate's `lib.rs` or another `src/` file, intra-doc links must resolve;
otherwise broken bracketed refs will silently render OK on GitHub.

```bash
grep -rn 'include_str!("../docs/api-reference.md")' crates/resource/
```

If included, all `[`Foo`]` paths must resolve. If not included, leave as
GitHub-friendly Markdown.

- [ ] **Step 5: Sanity-check that no fabricated symbols remain**

```bash
# Symbols that MUST be absent from api-reference.md (fabrications):
for sym in "AcquireCircuitBreakerPreset" \
           "ResourceContext::with_scope" \
           "ResourceContext::with_cancel_token" \
           "EventSourceConfig" \
           "DaemonConfig" \
           "EventSource\$" \
           "Daemon\$" \
           "type Auth"; do
  if grep -F "$sym" crates/resource/docs/api-reference.md > /dev/null; then
    echo "DRIFT: $sym still appears"
  fi
done
```

Expected: zero "DRIFT" lines.

- [ ] **Step 6: Commit Task 4**

```bash
git add crates/resource/docs/api-reference.md
git commit -m "docs(resource): rewrite api-reference.md from source (R-030)

Pre-rewrite api-reference.md had ~50% fabrication rate — referencing
types that don't exist (AcquireCircuitBreakerPreset,
ResourceContext::with_scope/.with_cancel_token), wrong
ResourceMetadata shape (4-field vs actual { base }), wrong variant
counts (TopologyTag 7 vs actual 5; ResourceEvent 7 vs 12 — see
events.md commit), missing register_*_with / acquire_*_default
methods (П1+П3 surface).

Rewritten section-by-section against current source:

- Resource trait: type Credential per ADR-0036, on_credential_refresh
  + on_credential_revoke default-no-op hooks, RPITIT signatures
- ResourceConfig: HasSchema super-bound (was missing)
- ResourceMetadata: { base: BaseMetadata } shape; for_resource /
  builder constructors; validate_compatibility prose
- Manager: full surface (1 register + 5 register_<topology> + 5
  register_<topology>_with + 5 acquire_<topology> + 5
  acquire_<topology>_default + lookup/remove/reload/shutdown/graceful
  + observability accessors)
- RegisterOptions field list including credential_id, recovery_gate,
  credential_rotation_timeout
- Daemon / EventSource sections REMOVED — they live in
  nebula_engine::daemon now per ADR-0037
- TopologyTag / TopologyRuntime: 5 variants (post-П3)
- AcquireResilience: timeout + retry only (no circuit_breaker field;
  no AcquireCircuitBreakerPreset enum)
- ResourceEvent reference points at events.md (canonical catalog)
- ResourceContext: actual ::new(ExecutionId) constructor + capability
  traits (no fabricated builders)

Verifies clean with RUSTDOCFLAGS=-D warnings cargo doc.

Refs: register R-030; ADR-0036; ADR-0037; cascade Tech Spec §6"
```

---

## Task 5 — Ground-up rewrite of `adapters.md`

**Files:**
- Rewrite: `crates/resource/docs/adapters.md` (414 → ~350 LOC)

**Why:** Closes R-031. Current document had compile-fail blocks at the time of the cascade Phase 1 audit (4/7); П1's `Credential` reshape resolved some, but case-drift accumulated. This rewrite rebuilds the adapter walkthrough end-to-end against the current trait shape.

The PostgreSQL pseudo-example stays — it's the right teaching shape; just re-ground the code blocks against actual types.

- [ ] **Step 1: Read the current `adapters.md` end-to-end**

Familiarise with the 6-step walkthrough (Cargo.toml, Config, Error,
Resource, Topology, Register, Tests, Checklist).

- [ ] **Step 2: Read the actual `Pooled` trait + topology configs**

```bash
cat crates/resource/src/topology/pooled/mod.rs
cat crates/resource/src/topology/pooled/config.rs
```

Note exactly which methods exist on `Pooled` today (especially: does
`prepare` exist? if not, REMOVE the section. The current adapters.md
documents `prepare` — it may or may not be real today. Source wins.).

- [ ] **Step 3: Read the actual `register_pooled_with` signature**

`crates/resource/src/manager/mod.rs:470-507`. Copy the exact signature
into a scratchpad — Step 6's bootstrap example must use it.

- [ ] **Step 4: Read the `ClassifyError` macro**

```bash
cat crates/resource-macros/src/lib.rs  # path approx
```

Verify the attribute syntax (`#[classify(transient)]`,
`#[classify(permanent)]`, `#[classify(exhausted(retry_after_secs))]`).

- [ ] **Step 5: Replace `adapters.md`**

Write the new file (~350 LOC) with this structure:

```markdown
# nebula-resource — Writing Adapter Crates

An adapter crate (e.g. `nebula-resource-postgres`, `nebula-resource-redis`)
wraps a specific driver library and implements `Resource` so that
`nebula-resource` can pool, health-check, and lifecycle-manage it.

This guide walks through a complete pseudo-Postgres example. Real
Postgres drivers (e.g., `tokio-postgres`) are not workspace dependencies,
so all code blocks use ` ```rust,ignore ` to keep them out of `cargo test
--doc`. Replace `ignore` with `no_run` once you add the driver crate to
your project.

> **Note:** the examples below intentionally use fictional types
> (`PgConnection`, `tokio_postgres::connect`) so you can read the
> walkthrough without depending on a real driver. Substitute your driver's
> types as you go. The signatures shown for `nebula_resource::*` types are
> the real signatures — verify against
> [`api-reference.md`](api-reference.md) at the version you're targeting.

---

## Overview

An adapter crate owns three things:

1. A **config struct** — operational parameters (host, port, timeouts).
   No secrets.
2. A **resource struct** — implements `Resource` with five associated
   types and the lifecycle methods. The factory that creates and tears
   down connections.
3. A **topology impl** — `Pooled`, `Resident`, `Service`, `Transport`, or
   `Exclusive`. Most database adapters use `Pooled`.

The manager calls `Resource::create` when it needs a new instance,
`Pooled::recycle` when an instance is returned, `Resource::check` periodically
when configured, and `Resource::destroy` when an instance is discarded.

If your resource binds to credentials (database password, API key, OAuth
token), declare a `Credential` type per ADR-0036; `register_pooled_with`
takes a `RegisterOptions { credential_id: Some(...), .. }` to wire the
reverse-index. Resources without authentication declare
`type Credential = NoCredential;`.

---

## Cargo.toml

```toml
[package]
name = "nebula-resource-postgres"
version = "0.1.0"
edition = "2024"

[dependencies]
nebula-resource = { path = "../../crates/resource" }
nebula-core     = { path = "../../crates/core" }
thiserror       = { workspace = true }
tokio           = { workspace = true, features = ["rt-multi-thread"] }
tracing         = { workspace = true }
# your driver:
# tokio-postgres = "0.7"
```

---

## Step 1: Define Config

`ResourceConfig` holds operational parameters. Implement `validate()` to
reject bad inputs before the manager tries to create connections, and
`fingerprint()` to enable config hot-reload without recreating healthy
instances.

[Code block — same shape as current; verify the `nebula_schema::HasSchema`
super-bound is reflected. The current example doesn't `derive(HasSchema)`;
add a note that `HasSchema` is auto-implemented for many types via
`nebula-schema` baselines, or show the manual impl.]

Two rules for `validate`:

- Reject every bad field, not just the first. Build up errors and return
  a combined message.
- Validate format (non-empty host, valid port range), not connectivity.
  Connectivity belongs in `Resource::create`.

`fingerprint` must be deterministic: same config fields → same
fingerprint. When the manager detects a fingerprint change after
`reload_config`, it evicts idle pool instances created with the old
config.

---

## Step 2: Define Error

Your resource's `Error` type must implement `Into<nebula_resource::Error>`.
The framework reads `ErrorKind` on the converted error to decide whether
to retry (`Transient`), give up (`Permanent`), or backoff (`Exhausted`).

### Option A: `ClassifyError` derive (recommended)

[Code block — verify attribute syntax against
`crates/resource-macros/src/`. The current example uses
`#[classify(exhausted(retry_after_secs))]` referencing a field name —
if that's still the syntax, keep it; otherwise update.]

### Option B: Manual `From` impl

[Same shape as current — verify `Error::transient`/`Error::permanent`
constructor signatures.]

---

## Step 3: Implement Resource

[Code block — full Resource impl. Critical fixes:
- type Credential = NoCredential (or the credential type if authed)
- create signature: `scheme: &<Self::Credential as Credential>::Scheme`
  (or `_scheme: &()` for NoCredential)
- Add a sidebar showing the credential-bound version with on_credential_refresh
- Remove any v1 method references]

If your resource binds to a credential, override `on_credential_refresh`
to perform a blue-green pool swap per credential Tech Spec §15.7. Default
no-op is correct for unauthenticated resources or resources whose
identity is checked at connection time only.

```rust,ignore
// Authenticated variant — overrides on_credential_refresh
impl Resource for MyAuthedResource {
    type Credential = MyApiCredential;     // user-defined, implements Credential

    async fn create(
        &self,
        config: &MyConfig,
        scheme: &<MyApiCredential as Credential>::Scheme,
        _ctx: &ResourceContext,
    ) -> Result<MyClient, MyError> {
        MyClient::connect(&config.url, scheme.bearer_token()).await
    }

    async fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, MyApiCredential>,
        _ctx: &'a CredentialContext,
    ) -> Result<(), MyError> {
        // Build a fresh client from new_scheme.bearer_token(), atomically
        // swap into self.client_arc — let RAII drain the old one.
        Ok(())
    }
    /* ... */
}
```

---

## Step 4: Pick and Implement Topology

[Same Pooled walkthrough — VERIFY method list:
- is_broken — confirm signature
- recycle — confirm async signature returning RecycleDecision
- prepare — VERIFY EXISTS; if not, remove the bullet and the example
- session-setup mention — only if prepare exists]

---

## Step 5: Register and Acquire

`Manager::register_pooled` is the zero-boilerplate path for unauthenticated
pooled resources. It sets `scope = Global` and uses default
`RegisterOptions` (no resilience, no recovery gate, no credential).

```rust,ignore
let manager = Manager::new();

manager.register_pooled(
    PostgresResource,
    PostgresConfig { /* ... */ },
    PoolConfig { max_size: 10, ..PoolConfig::default() },
)?;
```

To acquire a connection — for `Credential = NoCredential` resources,
use the `_default` variant:

```rust,ignore
use nebula_resource::{AcquireOptions, ResourceContext};
use nebula_core::ExecutionId;

let ctx = ResourceContext::new(ExecutionId::new());
let handle = manager
    .acquire_pooled_default::<PostgresResource>(&ctx, &AcquireOptions::default())
    .await?;

let conn: &PgConnection = &*handle;
// handle is held until dropped; instance returns to the pool automatically
```

For credential-bound resources, use `acquire_pooled` directly with the
projected scheme:

```rust,ignore
let scheme = credential_store.project::<MyApiCredential>(&credential_id).await?;
let handle = manager
    .acquire_pooled::<MyAuthedResource>(&scheme, &ctx, &AcquireOptions::default())
    .await?;
```

If you need credentials, a recovery gate, or non-default resilience, use
the `_with` variant:

```rust,ignore
use nebula_resource::{AcquireResilience, RecoveryGate, RecoveryGateConfig, RegisterOptions};
use std::sync::Arc;

let gate = Arc::new(RecoveryGate::new(RecoveryGateConfig::default()));

manager.register_pooled_with(
    PostgresResource,
    PostgresConfig { /* ... */ },
    PoolConfig::default(),
    RegisterOptions {
        credential_id: Some(my_credential_id),     // populates rotation reverse-index
        resilience:    Some(AcquireResilience::standard()),
        recovery_gate: Some(gate.clone()),
        ..RegisterOptions::default()
    },
)?;
```

---

## Step 6: Integration Tests

Tests should not require a real database. Use a mock `PgConnection` with
atomic flags to exercise the lifecycle without network I/O.

[Code block — same shape as current; verify:
- ResourceContext::new(ExecutionId::new()) constructor
- AcquireOptions::default() construction
- handle.topology_tag() returns TopologyTag::Pool (no EventSource/Daemon)
- Drop semantics]

```rust,ignore
#[tokio::test]
async fn register_and_acquire() {
    let manager = Manager::new();
    manager
        .register_pooled(
            PostgresResource,
            PostgresConfig::default(),
            PoolConfig::default(),
        )
        .expect("valid config must register");

    let ctx = ResourceContext::new(ExecutionId::new());
    let handle = manager
        .acquire_pooled_default::<PostgresResource>(&ctx, &AcquireOptions::default())
        .await
        .expect("acquire after registration");

    assert_eq!(handle.topology_tag(), nebula_resource::TopologyTag::Pool);

    drop(handle);  // returns instance to the pool automatically
}
```

---

## Checklist

Before publishing an adapter crate:

- [ ] `ResourceConfig::validate` rejects all invalid inputs without
      panicking.
- [ ] `ResourceConfig::fingerprint` is deterministic: equal configs
      produce equal fingerprints.
- [ ] `Resource::create` maps driver errors to `Transient` vs `Permanent`
      `ErrorKind` so the framework knows whether to retry.
- [ ] `Resource::key()` returns a stable string literal, not a derived
      type name.
- [ ] `Pooled::is_broken` is O(1) and performs no I/O — it runs in `Drop`.
- [ ] `Pooled::recycle` rolls back any open transaction before returning
      `Keep`.
- [ ] `Resource::Credential = NoCredential` unless the adapter genuinely
      binds to a credential.
- [ ] If credential-bound: `Resource::on_credential_refresh` performs the
      blue-green pool swap per credential Tech Spec §15.7;
      `Resource::on_credential_revoke` ensures no further authenticated
      traffic.
- [ ] `Runtime` does not implement `Debug`, or implements a redacted
      version that omits connection strings, secrets, and internal buffer
      state.
- [ ] Integration tests use a mock/in-memory runtime, not a live network
      service.
- [ ] `#![forbid(unsafe_code)]` is set in `lib.rs`.
```

- [ ] **Step 6: Sanity check — no fabricated symbols**

```bash
# Symbols that MUST be absent from adapters.md:
for sym in "type Auth" \
           "Credential::KIND" \
           "AcquireCircuitBreakerPreset" \
           "EventSourceConfig" \
           "DaemonConfig"; do
  if grep -F "$sym" crates/resource/docs/adapters.md > /dev/null; then
    echo "DRIFT: $sym still appears"
  fi
done
```

Expected: zero "DRIFT" lines.

- [ ] **Step 7: Run rustdoc gate**

```bash
RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-resource --no-deps 2>&1 | tail -20
```

Expected: clean.

- [ ] **Step 8: Commit Task 5**

```bash
git add crates/resource/docs/adapters.md
git commit -m "docs(resource): ground-up rewrite of adapters.md against current trait shape (R-031)

Pre-rewrite adapters.md had compile-fails on multiple example blocks
(per Phase 1 audit), and post-П1 + post-П3 the trait surface drifted
further. This rewrite rebuilds the walkthrough against current code:

- Resource impl uses type Credential (ADR-0036) with NoCredential
  opt-out and a credential-bound sidebar
- Step 5 'Register and Acquire' shows the three real paths:
  register_pooled (no-credential, default options) →
  acquire_pooled_default; acquire_pooled with explicit scheme;
  register_pooled_with for authenticated / gate / resilience cases
- on_credential_refresh / on_credential_revoke override pattern
  documented; references credential Tech Spec §15.7 (blue-green pool
  swap)
- Checklist gains credential-rotation hygiene items
- Removed v1 fabrications (type Auth, Credential::KIND, prepare if
  absent in source)
- Front-matter explains the rust,ignore convention so readers don't
  expect the examples to compile against the workspace

Refs: register R-031; ADR-0036; cascade Tech Spec §6 + §11"
```

---

## Task 6 — Fix `README.md` (broken links + Daemon/EventSource removal + crate layout refresh)

**Files:**
- Modify: `crates/resource/docs/README.md` (target ~280 LOC after edits)

**Why:** Closes R-033. Three classes of fix:

1. Topology Decision Guide includes Daemon and EventSource as "secondary topology" — those live in `nebula-engine` now (per ADR-0037 / П3).
2. Documentation table (lines ~285-292) has 4 broken intra-doc links — `architecture.md` (file deleted in Task 2), `pooling.md` (file is `Pooling.md` — case mismatch on case-sensitive filesystems), `events-and-hooks.md` (file is `events.md`), `health-and-quarantine.md` (file is `recovery.md`).
3. Crate Layout block (lines ~248-279) lists v1 module names: `manager.rs` is now `manager/` (a directory with submodules), the directory map shows old paths, AutoScaler / HookRegistry / etc. references should be gone (verify line by line).

- [ ] **Step 1: Read the current README.md fully**

```bash
cat crates/resource/docs/README.md
```

- [ ] **Step 2: Edit Topology Decision Guide — remove Daemon/EventSource rows**

The Topology Decision Guide block at `crates/resource/docs/README.md`
lines ~30-43 currently includes:

```markdown
| `Daemon` | Background task with no direct callers (secondary topology) | Metrics flush loop |
| `EventSource` | Pull-based subscription stream (secondary topology) | Webhook ingestion tail |
```

Remove both rows. Add a one-line note below the table:

```markdown
> **Note:** background workers and event-source adapters live in
> [`nebula-engine`](https://docs.rs/nebula-engine) (`nebula_engine::daemon::*`)
> per [ADR-0037](../../docs/adr/0037-daemon-eventsource-engine-fold.md).
> They are not part of the `nebula-resource` topology surface.
```

(Adjust the link target paths to match the actual directory structure
relative to README.md.)

- [ ] **Step 3: Refresh "Crate Layout" block**

The Crate Layout block at `crates/resource/docs/README.md` lines ~248-279
shows a directory tree. Verify each line against actual `crates/resource/src/`:

```bash
ls -la crates/resource/src/
ls -la crates/resource/src/manager/
ls -la crates/resource/src/runtime/
ls -la crates/resource/src/topology/
ls -la crates/resource/src/recovery/
```

The current README block lists `manager.rs` and `registry.rs` etc. as
flat files. Replace with the real tree:

```text
crates/resource/
├── src/
│   ├── lib.rs              Re-exports and crate-level docs
│   ├── resource.rs         Resource trait, ResourceConfig, ResourceMetadata
│   ├── manager/            Manager directory (split per Tech Spec §5.4):
│   │   ├── mod.rs          Manager type + register/acquire entry points
│   │   ├── options.rs      ManagerConfig, RegisterOptions, ShutdownConfig, DrainTimeoutPolicy
│   │   ├── registration.rs Internal register_inner + reverse-index write
│   │   ├── gate.rs         Recovery-gate admission helpers
│   │   ├── execute.rs      Resilience pipeline
│   │   ├── rotation.rs     ResourceDispatcher trampoline + rotation fan-out
│   │   └── shutdown.rs     graceful_shutdown + drain helpers
│   ├── registry.rs         Registry, AnyManagedResource — type-erased storage
│   ├── guard.rs            ResourceGuard — RAII acquire lease (Owned/Guarded/Shared)
│   ├── context.rs          ResourceContext — execution context with capabilities
│   ├── error.rs            Error, ErrorKind, ErrorScope, RotationOutcome
│   ├── events.rs           ResourceEvent — 12 lifecycle event variants
│   ├── options.rs          AcquireOptions, AcquireIntent
│   ├── metrics.rs          ResourceOpsMetrics, ResourceOpsSnapshot
│   ├── state.rs            ResourcePhase, ResourceStatus
│   ├── cell.rs             Cell — ArcSwap-based lock-free cell for Resident
│   ├── release_queue.rs    ReleaseQueue — background async cleanup workers
│   ├── reload.rs           ReloadOutcome
│   ├── topology_tag.rs     TopologyTag — 5-variant discriminant enum
│   ├── integration.rs      AcquireResilience, AcquireRetryConfig
│   ├── ext.rs              HasResourcesExt trait
│   ├── recovery/           RecoveryGate, RecoveryGroupRegistry, WatchdogHandle
│   ├── runtime/            Per-topology runtime wrappers (5 topologies)
│   └── topology/           Per-topology trait definitions (5 topologies)
└── docs/
    ├── README.md           ← this file
    ├── api-reference.md    Full public API with signatures
    ├── adapters.md         Implementing Resource for a driver crate
    ├── pooling.md          PoolConfig, recycle policy, broken-check, max-lifetime
    ├── events.md           ResourceEvent catalog, subscribe_events usage
    └── recovery.md         RecoveryGate, WatchdogHandle, gate state transitions
```

(The exact list of paths must match `ls -la` output. Add/remove lines as
needed. Do NOT include `Architecture.md` or `dx-eval-real-world.rs` —
they were deleted in Task 2.)

- [ ] **Step 4: Fix the Documentation table (lines ~285-292)**

The table currently has 6 rows. Update to 5 rows (architecture row was
removed in Task 2; pooling/events/recovery references must be fixed):

```markdown
## Documentation

| Document | Contents |
|----------|----------|
| [`api-reference.md`](api-reference.md) | Every public type, trait, and method with signatures |
| [`adapters.md`](adapters.md) | Writing a `Resource` adapter crate (`nebula-resource-postgres`, etc.) |
| [`pooling.md`](pooling.md) | `PoolConfig`, recycle decisions, broken checks, max-lifetime eviction |
| [`events.md`](events.md) | `ResourceEvent` catalog, `subscribe_events` patterns |
| [`recovery.md`](recovery.md) | `RecoveryGate`, `WatchdogHandle`, gate state transitions |
```

(Note: `pooling.md` lowercase. The actual file is `Pooling.md` — Task 7
renames it. If you're sequencing strictly, rename FIRST then update the
link, or update the link AFTER rename. Either works as long as a single
commit lands both changes; a clean approach: do the rename in this same
commit so the file and the link change atomically.)

- [ ] **Step 5: Rename `Pooling.md` → `pooling.md`**

```bash
git mv crates/resource/docs/Pooling.md crates/resource/docs/pooling.md
```

(On case-insensitive filesystems on Windows you may need a two-step
rename: `git mv Pooling.md pooling.tmp.md && git mv pooling.tmp.md pooling.md`.)

- [ ] **Step 6: Run rustdoc gate + verify links**

```bash
RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-resource --no-deps 2>&1 | tail -20
```

Expected: clean.

If README.md is `include_str!`'d into `lib.rs` or any source file,
rustdoc will validate the markdown links against actual files. Either
way, manually verify each link target file exists:

```bash
for f in api-reference.md adapters.md pooling.md events.md recovery.md; do
  test -f "crates/resource/docs/$f" || echo "MISSING: $f"
done
```

Expected: zero MISSING lines.

- [ ] **Step 7: Sanity-check absent symbols**

```bash
# Symbols that MUST be absent from README.md after this edit:
for sym in "EventSource" "Daemon" "AutoScaler" "HookRegistry" "QuarantineManager" "EventBus" "DependencyGraph" "Architecture.md"; do
  if grep -E "\b$sym\b" crates/resource/docs/README.md > /dev/null; then
    echo "DRIFT: $sym still appears"
  fi
done
```

Expected: zero DRIFT lines. (If a sentence uses "Daemon" in a non-topology
sense — e.g., "background daemon thread" — that's fine; the grep above
is conservative. Read context for false positives.)

- [ ] **Step 8: Commit Task 6**

```bash
git add crates/resource/docs/README.md
git add crates/resource/docs/Pooling.md      # the deleted side of the rename
git add crates/resource/docs/pooling.md      # the new lowercase file
git commit -m "docs(resource): fix README intra-doc links + crate layout refresh (R-033)

Three classes of fix:

1. Topology Decision Guide drops Daemon/EventSource rows — they live
   in nebula_engine::daemon now per ADR-0037 / П3. Adds a sidebar
   note pointing users at nebula-engine.
2. Documentation table fixes 4 broken links: removes architecture.md
   (file deleted in prior commit), fixes events-and-hooks.md →
   events.md, fixes health-and-quarantine.md → recovery.md, and
   renames Pooling.md → pooling.md so the case-sensitive references
   resolve on Linux/CI.
3. Crate Layout refreshed against current src tree — manager/ is now
   a directory with 7 submodules (Tech Spec §5.4 split), removed v1
   fabrications (AutoScaler, HookRegistry, QuarantineManager,
   EventBus, DependencyGraph), 12-variant ResourceEvent and
   5-variant TopologyTag noted explicitly.

Refs: register R-033; ADR-0037; cascade Tech Spec §5.4"
```

---

## Task 7 — Verify and lightly update `pooling.md` and `recovery.md`

**Files:**
- Modify (if drift found): `crates/resource/docs/pooling.md`
- Modify (if drift found): `crates/resource/docs/recovery.md`

**Why:** The Phase 1 audit didn't flag these as 🔴 / 🟠 — they were "OK at the time". П1+П2+П3 may have introduced minor drift; this task is a verification pass with edit-only-if-needed. If both files come up clean, this task collapses to a no-op (delete the planned commit).

The file rename `Pooling.md → pooling.md` happened in Task 6.

- [ ] **Step 1: Verify `pooling.md` against current source**

```bash
cat crates/resource/docs/pooling.md
cat crates/resource/src/topology/pooled/config.rs
cat crates/resource/src/topology/pooled/mod.rs
cat crates/resource/src/runtime/pool.rs       # PoolRuntime + PoolStats
```

Check the following invariants:

- "How the Pool Works" diagram (lines 26-35): does `idle_queue:
  VecDeque<IdleEntry<R::Instance>>` reference an actual field/type? `R::Instance`
  is NOT a `Resource` associated type — the actual type is `R::Runtime`.
  If drift: fix to `R::Runtime`.
- `PoolConfig` field list (lines ~60-74) must match `PoolConfig` struct
  field-by-field. If a field was added/renamed in П1/П2/П3, update.
- "AutoScaler" / "auto-scaling" references — AutoScaler was a v1 concept;
  if mentioned, remove unless `pooling.md` documents an actual current
  feature.
- Backpressure types (`PoolBackpressurePolicy`, `AdaptiveBackpressurePolicy`)
  — verify still public and the variants/fields match.
- Circuit-breaker references — `create_breaker` / `recycle_breaker` —
  verify against `nebula-resilience` and `crates/resource/src/topology/pooled/config.rs`.

If drift found, edit minimally — preserve voice and structure. If
no drift found, skip the commit at Step 3.

- [ ] **Step 2: Verify `recovery.md` against current source**

```bash
cat crates/resource/docs/recovery.md
cat crates/resource/src/recovery/gate.rs
cat crates/resource/src/recovery/group.rs
cat crates/resource/src/recovery/watchdog.rs
```

Check invariants:

- `RecoveryGate::try_begin` return type — verify against current source.
- `RecoveryTicket::resolve` / `fail_transient` / `fail_permanent` /
  `attempt` — verify signatures.
- `RecoveryGateConfig` fields (`max_attempts`, `base_backoff`) — verify.
- `WatchdogHandle::start` signature — current doc shows full closure
  arguments; if the API changed, update.
- "Manager registers a RecoveryGate" example — verify against
  `register_*_with(... RegisterOptions { recovery_gate, .. })` shape.
- "Differences from v1" — remove if not relevant; otherwise verify items.

Edit minimally if drift found.

- [ ] **Step 3: Commit only if changes**

```bash
git diff --stat crates/resource/docs/pooling.md crates/resource/docs/recovery.md
```

If changes:

```bash
git add crates/resource/docs/pooling.md crates/resource/docs/recovery.md
git commit -m "docs(resource): refresh pooling.md / recovery.md against current source

Verification pass post-cascade — minor drift fixes:
[describe each fix in 1 line; if both files needed only renames, this
commit is just the renames bundled with Task 6]

Refs: cascade Tech Spec §6"
```

If no changes: skip the commit. Move to Task 8.

---

## Task 8 — Update concerns register

**Files:**
- Modify: `docs/tracking/nebula-resource-concerns-register.md`

**Why:** Lifecycle Rule 4 of the register requires items to migrate from
`tech-spec-material` `decided` → `landed` once the implementing PR ships.
П4 closes 6 rows: R-030 (api-reference), R-031 (adapters), R-032
(Architecture), R-033 (README), R-034 (dx-eval-real-world), R-035 (events).

This task does NOT yet have a PR URL — that lands in the П4 PR description
after the PR opens. Use a `landed П4 (PR #TBD; commit @ HEAD)` placeholder;
the user updates the PR number after `gh pr create`.

- [ ] **Step 1: Read the current register**

```bash
cat docs/tracking/nebula-resource-concerns-register.md | head -120
```

Locate the "Documentation" section (rows R-030 through R-035, around
lines 60-69 of the register). Each row currently has Status pointing to
"Strategy §4.7" or similar — none has `landed` yet.

- [ ] **Step 2: Update the 6 rows**

Edit each Status cell:

| ID | New Status (paste verbatim into the table cell) |
|----|--------------------------------------------------|
| R-030 | `**landed П4** (PR #TBD); api-reference.md rewritten from source — Resource trait per ADR-0036, 5-variant TopologyTag/TopologyRuntime, 12-variant ResourceEvent, 11 register + 10 acquire methods documented` |
| R-031 | `**landed П4** (PR #TBD); adapters.md rewritten — Resource impl with type Credential (NoCredential opt-out), credential-bound sidebar, register_pooled / register_pooled_with paths separated` |
| R-032 | `**landed П4** (PR #TBD); Architecture.md deleted — README + canon docs cover real architecture` |
| R-033 | `**landed П4** (PR #TBD); README intra-doc links fixed (architecture.md row removed; events-and-hooks.md → events.md; health-and-quarantine.md → recovery.md), Pooling.md → pooling.md case-rename, Daemon/EventSource topology rows dropped per ADR-0037` |
| R-034 | `**landed П4** (PR #TBD); dx-eval-real-world.rs deleted — friction commentary obsolete after П1` |
| R-035 | `**landed П4** (PR #TBD); events.md rebuilt — full 12 ResourceEvent variants split into per-resource (10) + aggregate rotation (2), event.key() Option semantics clarified` |

(The exact wording above can be tightened; preserve the four-element
shape: status + PR placeholder + 1-line rationale + reference.)

- [ ] **Step 3: Update "Register updates" log**

At the bottom of the register, add a single line:

```markdown
- 2026-04-28 — П4 doc rewrite landed (R-030/R-031/R-032/R-033/R-034/R-035 → `landed`)
```

(Date matches the plan filename. Update the actual landing date if the
PR merges on a different day.)

- [ ] **Step 4: Commit Task 8**

```bash
git add docs/tracking/nebula-resource-concerns-register.md
git commit -m "docs(register): mark R-030..R-035 landed for П4 doc rewrite

All 6 documentation concerns from the cascade Phase 1 audit close in
П4:

- R-030 🔴 api-reference.md fabrication → rewritten from source
- R-031 🔴 adapters.md compile-fail blocks → ground-up rewrite
- R-032 🟠 Architecture.md v1 module map → deleted (README + canon docs)
- R-033 🟠 README broken intra-doc links → fixed + Daemon/EventSource removed
- R-034 🟠 dx-eval-real-world.rs stale → deleted
- R-035 🟡 events.md variant undercounts → rebuilt against 12 variants

PR # placeholder TBD — update on PR open.

Refs: cascade Phase 1 §1.3 (dx-tester); cascade Phase 0 finding 8;
register lifecycle rule 4"
```

---

## Task 9 — Final verification (no commits)

**Files:**
- None modified

**Why:** Pre-PR gate. Confirms all four toolchain checks remain clean and
the test count is stable. Per `feedback_lefthook_mirrors_ci` memory, the
local gate and CI must agree.

- [ ] **Step 1: Format check**

```bash
cargo +nightly fmt --all -- --check 2>&1 | tail -5
```

Expected: no diff.

- [ ] **Step 2: Clippy**

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5
```

Expected: 0 warnings.

- [ ] **Step 3: Tests**

```bash
cargo nextest run --workspace --profile ci --no-tests=pass 2>&1 | tail -5
```

Expected: **3660/3660 passed** (П3 baseline preserved — П4 does not
change test surface).

- [ ] **Step 4: Doc gate (CRITICAL for П4)**

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps 2>&1 | tail -20
```

Expected: clean. This is the gate that catches broken intra-doc links.
If this fails, the most likely cause is a `[`Foo`]` reference in
`api-reference.md` / `events.md` / `adapters.md` that points at a path
rustdoc can't resolve. Fix the offending link or remove the bracket
syntax (use plain backticks).

- [ ] **Step 5: Cross-check: no fabricated symbols remain anywhere**

```bash
for sym in "AcquireCircuitBreakerPreset" \
           "ResourceContext::with_scope" \
           "ResourceContext::with_cancel_token" \
           "EventSourceConfig" \
           "DaemonConfig" \
           "type Auth" \
           "Credential::KIND" \
           "AutoScaler" \
           "HookRegistry" \
           "QuarantineManager" \
           "DependencyGraph"; do
  matches=$(grep -rln "$sym" crates/resource/docs/ 2>/dev/null || true)
  if [ -n "$matches" ]; then
    echo "DRIFT: $sym appears in: $matches"
  fi
done
```

Expected: zero DRIFT lines. If any appear, fix the offending file before
opening the PR.

- [ ] **Step 6: Compare LOC delta to plan target**

```bash
wc -l crates/resource/docs/*.md
```

Expected (rough):

```
~280 crates/resource/docs/README.md
~600 crates/resource/docs/api-reference.md
~350 crates/resource/docs/adapters.md
~370 crates/resource/docs/pooling.md     (renamed from Pooling.md)
~150 crates/resource/docs/events.md
~165 crates/resource/docs/recovery.md
≈ 1915 LOC total (vs 3528 baseline; delta ≈ -1613 LOC)
```

The delta target was ~+200 / -1500 ≈ -1300 net. We over-deleted by
removing `Architecture.md` (316) + `dx-eval-real-world.rs` (1012) =
-1328, plus rewrote api-reference smaller (-269), and grew events.md
(+60). Net check: matches the kickoff-brief estimate.

- [ ] **Step 7: Inventory the commit history**

```bash
git log --oneline origin/main..HEAD
```

Expected: 5-7 commits, ordered:

```
docs(register): mark R-030..R-035 landed for П4 doc rewrite
docs(resource): refresh pooling.md / recovery.md against current source  (skipped if no diff)
docs(resource): fix README intra-doc links + crate layout refresh (R-033)
docs(resource): ground-up rewrite of adapters.md against current trait shape (R-031)
docs(resource): rewrite api-reference.md from source (R-030)
docs(resource): rebuild events.md against 12-variant ResourceEvent (R-035)
docs(resource)!: retire Architecture.md and dx-eval-real-world.rs (R-032, R-034)
docs(superpowers): capture canonical API surface for П4 doc rewrite
docs(superpowers): П4 doc-rewrite implementation plan                     (the plan itself; landed before Task 0)
```

(The two `docs(superpowers)` commits are the plan and the surface
artefact — both landed before the doc-rewrite work began.)

- [ ] **Step 8: No commit; ready for PR**

Open the PR via `gh pr create` once Task 9 passes. PR description
template:

```markdown
## Summary

П4 of the nebula-resource redesign cascade — pure documentation rewrite.

- Deletes `Architecture.md` (316 LOC v1 fabrication) and
  `dx-eval-real-world.rs` (1012 LOC stale design eval). README +
  PRODUCT_CANON cover real architecture; П1 retired the friction
  commentary.
- Rewrites `api-reference.md` from source — closes R-030 (~50%
  fabrication rate). Documents the actual 11 register + 10 acquire
  methods, Resource trait per ADR-0036, 5-variant TopologyTag /
  TopologyRuntime, AcquireResilience without fabricated
  `circuit_breaker` field, ResourceContext without fabricated
  `with_scope` / `with_cancel_token` builders.
- Rewrites `adapters.md` ground-up — closes R-031. Resource impl uses
  `type Credential` (`NoCredential` opt-out), credential-bound sidebar
  documents `on_credential_refresh` blue-green pool swap pattern.
- Rebuilds `events.md` against the actual 12-variant `ResourceEvent` —
  closes R-035. Splits per-resource (10) + aggregate rotation (2).
- Fixes README intra-doc links (R-033): drops Daemon/EventSource
  topology rows, fixes 4 broken doc references, refreshes Crate Layout
  for the manager/ submodule split (Tech Spec §5.4).
- Renames `Pooling.md → pooling.md` for case consistency on
  case-sensitive filesystems.
- Concerns register: marks R-030/R-031/R-032/R-033/R-034/R-035 as
  `landed П4`.

Test count stable at 3660/3660. `RUSTDOCFLAGS=-D warnings cargo doc`
clean.

## Test plan

- [ ] `cargo +nightly fmt --all -- --check` clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` 0 warnings
- [ ] `cargo nextest run --workspace --profile ci --no-tests=pass` 3660/3660
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` clean
- [ ] No fabricated symbols remain in `crates/resource/docs/` (grep audit)

## Cascade context

This is wave 4 of 5 in the nebula-resource redesign cascade. Prior:
- П1: ec36365f — `Resource::Credential` adoption (ADR-0036)
- П2: 84d57414 — rotation L2 dispatch (Tech Spec CP2)
- П3: 671a0ffd — Daemon/EventSource engine fold (ADR-0037)
- П4: this PR — documentation rewrite
- П5: post-soak maturity bump (frontier → core, Strategy §6.4)

Refs: docs/tracking/nebula-resource-concerns-register.md;
      docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md §4.7;
      docs/superpowers/plans/2026-04-28-nebula-resource-p4-doc-rewrite.md
```

---

## Self-review checklist (run before declaring plan complete)

- [x] **Spec coverage** — every R-030..R-035 row in the kickoff brief has a Task that closes it. R-030 → Task 4. R-031 → Task 5. R-032 → Task 2. R-033 → Task 6. R-034 → Task 2. R-035 → Task 3.
- [x] **No placeholders in tasks** — every step has a concrete command or block content. Where a doc structure is shown as a skeleton (Task 4 / Task 5), the skeleton names every section header and notes which source file fills the contents.
- [x] **Type consistency** — `Manager` API is referenced consistently (10 acquire / 11 register methods); `ResourceEvent` is 12 variants throughout; `TopologyTag` / `TopologyRuntime` are 5 variants throughout; `type Credential` (not `type Auth`) is used everywhere.
- [x] **Verification commands** — every doc task ends with a rustdoc gate; final task has the four-tool gate.
- [x] **Honest deferrals** — `prepare()` on `Pooled` is flagged "verify; remove if absent"; `RegisterOptions` field list is "verify against current source"; `pooling.md` / `recovery.md` are "edit only if drift found". These are real uncertainties left to the executing subagent rather than forced into the plan.
- [x] **No new code paths** — Task 7 (the `tests/adapter_smoke.rs` scaffold) was DEFERRED per design decision 3; the plan honors "PURE DOCS — no source-code changes" except for the `docs/tracking/` register update.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-28-nebula-resource-p4-doc-rewrite.md`.

Two execution options per the established cascade pattern:

1. **Subagent-Driven (recommended; matches П1–П3)** — main session dispatches one fresh implementer subagent per task with the Task 1 surface artefact as their canonical context, two-stage review (spec-auditor for cross-reference + content accuracy; rust-senior or dx-tester for newcomer-readability) after each, fix-or-merge cadence.

2. **Inline Execution** — main session executes tasks in this same conversation using `superpowers:executing-plans`, batch with checkpoints between Tasks 4/5 (the two largest rewrites) for review.

Per-wave tradition (П1–П3 all used Subagent-Driven), default to option 1 unless the user requests otherwise.
