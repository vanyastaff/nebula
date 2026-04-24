# 01 — Current State of `nebula-resource`

**Phase:** 0 — Documentation reconciliation (corrected by Phase 1)
**Date:** 2026-04-24
**Commit audited:** `d6cee19f814ff2c955a656afe16c9aeafca16244`
**Worktree:** `.worktrees/nebula/vigilant-mahavira-629d10`
**Orchestrator:** main session (flat coordination)

**Note:** this document was originally generated from `phase-0-code-audit.md` (rust-senior) and `phase-0-manifest-audit.md` (devops), both of which were lost in the filesystem event noted in `CASCADE_LOG.md` 2026-04-24 T+~45min. Content below is reconstructed from orchestrator context. The consolidated Phase 1 deliverable `02-pain-enumeration.md` is unaffected and remains the canonical pain-enumeration reference.

**Status:** Phase 0 gate PASSES — audits were consistent, ground truth established. Phase 1 completed and produced material findings. §3.1 corrected per Phase 1.

---

## Executive summary

The `nebula-resource` crate is **entirely a v2 codebase**. No v1 symbols exist in `src/`. The widely-documented "v1/v2 drift" is **doc drift, not code drift** — `docs/Architecture.md` describes a vanished v1 module map, while `docs/events.md` and `docs/recovery.md` describe the current v2 correctly.

Key code-level findings that shape downstream cascade phases:

1. **🟠/🔴-latent panic surface on credential rotation** — `Manager::on_credential_refreshed` and `on_credential_revoked` are public async methods. Phase 1 corrected Phase 0's characterization — see §3.1 below.

2. **`TopologyTag` is a runtime `#[non_exhaustive] enum`, NOT a phantom type tag per ADR-0035.** The orchestrator prompt mischaracterised this. **Any Phase 1/2/4 work must NOT assume phantom-tag dispatch.**

3. **Five infrastructure gaps** (confirmed Phase 0 devops audit):
   - No `deny.toml` layer-enforcement rule for `nebula-resource` despite 5 consumers spanning business and exec tiers
   - Zero feature flags despite pulling heavy deps
   - No `benches/`, no CodSpeed shard for a runtime-critical pool-acquire hot path
   - No external `nebula-resource-*` adapter crates — `docs/adapters.md` is entirely aspirational
   - Doc drift at multiple layers (Architecture.md v1, README filename case-mismatch, adapters.md out-of-date API signatures, events.md variant count wrong)

---

## 1. Ground truth — code reality

### 1.1 Module structure

Crate body is organized as **five concentric layers** (confirmed via `lib.rs` re-exports):

| Layer | Modules | Role |
|---|---|---|
| **Core trait + metadata** | `resource.rs`, `state.rs`, `topology_tag.rs`, `options.rs`, `reload.rs` | `Resource`, `ResourcePhase`, `TopologyTag`, `AcquireOptions`, `ReloadOutcome` |
| **Topology traits** | `topology/{daemon,event_source,exclusive,pooled,resident,service,transport}.rs` | 7 sub-traits extending `Resource` + config structs |
| **Topology runtimes** | `runtime/{daemon,event_source,exclusive,managed,pool,resident,service,transport}.rs` | 7 `*Runtime<R>` structs + `ManagedResource<R>` bundle + `TopologyRuntime<R>` dispatch enum |
| **Manager orchestration** | `manager.rs` (2101 L), `registry.rs`, `release_queue.rs`, `guard.rs`, `cell.rs`, `context.rs` | `Manager`, `Registry`, `ReleaseQueue`, `ResourceGuard`, etc. |
| **Cross-cutting** | `recovery/{gate,group,watchdog}.rs`, `integration/resilience.rs`, `error.rs`, `events.rs`, `metrics.rs`, `ext.rs` | `RecoveryGate`, `WatchdogHandle`, resilience presets, typed `Error`, `ResourceEvent`, ops counters, `HasResourcesExt` |

**Total source:** ~10.7K L in `src/`. Tests: 5413 L across 4 files. Docs: 2500 L across 7 files (mixed v1/v2).

### 1.2 Public API surface summary

- **Flat re-export model** — `lib.rs` (111 L) exposes ~60 named items from module paths.
- **RPITIT throughout** — all async trait methods use `impl Future<…> + Send`, no `async_trait` macro, no `Box<dyn Future>`. Idiom compliant.
- **`Resource` trait has 5 associated types** — `Config`, `Runtime`, `Lease`, `Error`, `Auth`. `Auth: nebula_credential::AuthScheme` hardwires credential dependency at trait level.
- **7 topology sub-traits** — Pool, Resident, Service, Transport, Exclusive, EventSource, Daemon.
- **`Manager` has 40+ public methods** — register/acquire in 5 flavors (generic + `register_{pooled,resident,service,exclusive,transport}` + `_with` variants + `_default` variants). **No `register_daemon` / `register_event_source` helpers.**
- **`ResourceGuard<R>`** has Owned / Guarded / Shared release modes. Stores runtime `TopologyTag` (not phantom).

### 1.3 v2 infrastructure present

Confirmed shapes match v2 design in `docs/recovery.md` and `docs/events.md`:

| v2 item | Location | Shape |
|---|---|---|
| `RecoveryGate` (CAS) | `recovery/gate.rs` | `ArcSwap<GateState>` + `compare_and_swap` loop + `Notify` for waiters. `RecoveryTicket` has `#[must_use]` + `Drop` auto-fail. |
| `WatchdogHandle` | `recovery/watchdog.rs` | Cancellable spawn holding `check_fn` + `on_health_change`. `Drop` cancels. |
| `broadcast::Sender<ResourceEvent>` | `manager.rs:252` | 256-event buffer. `Manager::subscribe_events() -> broadcast::Receiver`. |
| `ArcSwap<ResourceStatus>` + `AtomicU64` generation | `runtime/managed.rs` | Lock-free phase reads, generation bump on reload. |
| `CancellationToken` hierarchy | `manager.rs`, `runtime/daemon.rs` | Parent token + per-run child tokens for daemons. |
| Drain tracker | `manager.rs:256`, `guard.rs:39` | `Arc<(AtomicU64, Notify)>` shared with every guard. |
| 3-tier release queue | `release_queue.rs:41-60` | Primary mpsc workers → fallback → `RESCUE_TIMEOUT: 30s` spawn. |
| Credential reverse-index | `manager.rs:262` | `DashMap<CredentialId, Vec<ResourceKey>>` — **declared but never written**, see §3.1. |

---

## 2. Documentation vs code diff (Phase 0 reconciliation)

| Doc claim | Code reality | Status |
|---|---|---|
| `docs/Architecture.md` describes `HookRegistry`, `QuarantineManager`, `HealthChecker`, `HealthPipeline`, `EventBus`, `HealthStage`, `ConnectivityStage`, `HookEvent`, `AutoScaler`, `Poison`, `DependencyGraph`, `Lifecycle`, `AnyPool`, `TypedPool`, etc. | **Zero of these exist in `src/`.** | **Doc is v1, code is v2 — full rewrite needed.** |
| `docs/events.md` table lists 7 `ResourceEvent` variants | Actual `events.rs` has **10 variants** — missing from doc: `RetryAttempt`, `BackpressureDetected`, `RecoveryGateChanged` | Partial drift — doc needs sync pass. |
| `docs/events.md` — "No `HookRegistry`", "No `EventBus` — events come directly from `Manager::subscribe_events()`" | Confirmed. | **Doc is correct.** |
| `docs/recovery.md` — "No `QuarantineManager` — replaced by `RecoveryGate`", "No `HealthChecker`", "No `HealthPipeline`" | All confirmed in code. | **Doc is correct.** |
| `docs/README.md` links `architecture.md` (lowercase), `events-and-hooks.md`, `health-and-quarantine.md`, `pooling.md` | Actual files: `Architecture.md`, `events.md`, `recovery.md`, `Pooling.md` (mixed case) | **Doc drift — filename/case mismatch.** |
| `docs/adapters.md:346` — `let ctx = ResourceContext::new(ExecutionId::new());` | Real: `ResourceContext::new(base: BaseContext, resources: Arc<dyn ResourceAccessor>, credentials: Arc<dyn CredentialAccessor>)` | **Doc shows wrong API shape.** |
| `docs/adapters.md:244-246` — `m.name`, `m.description`, `m.tags` as direct fields on `ResourceMetadata` | Real: `ResourceMetadata` has only `base: BaseMetadata<ResourceKey>` field | **Doc shows wrong struct shape.** |
| `docs/adapters.md` presents `nebula-resource-postgres`, `nebula-resource-redis` as named adapter crates | **No such crates exist anywhere** | **Doc is aspirational.** |
| Orchestrator prompt — "topology_tag (likely phantom-tag pattern per ADR-0035)" | **Incorrect.** Concrete `#[non_exhaustive] enum TopologyTag { Pool, Resident, … }` stored at runtime on `ResourceGuard`. Zero `PhantomData`. | **Brief was wrong. Corrected.** |

Phase 1 dx-tester independently confirmed additional doc-drift findings (`api-reference.md` ~50% fabrication rate, `adapters.md` hidden `HasSchema` super-trait, `ResourceContext::with_scope/.with_cancel_token` don't exist). See `02-pain-enumeration.md` §1.3.

---

## 3. Critical findings (foundation for Phase 1)

### 3.1 🟠 Silent revocation drop + latent 🔴 panic on credential rotation path (manager.rs:1360-1400)

> **Phase 1 correction to Phase 0:** initial Phase 0 wording said "reverse-index populated on register but never read by a real dispatcher". **That was factually wrong.** Phase 1 security-lead verified (orchestrator re-confirmed via grep): `credential_resources: DashMap<CredentialId, Vec<ResourceKey>>` at `manager.rs:262` is the **field declaration**; initialized empty at `manager.rs:293`; `Manager::register` hardcodes `credential_id: None` at `manager.rs:370` on every registration. **The write site does not exist anywhere in the codebase.** The only reads are at lines 1365, 1388 — both inside the two `todo!()` methods.

**Today's behaviour (not a panic — a silent no-op):**
```rust
// manager.rs:1360-1372 — on_credential_refreshed
pub async fn on_credential_refreshed(&self, credential_id: &CredentialId)
    -> Result<Vec<(ResourceKey, ReloadOutcome)>, Error> {
    let keys = self.credential_resources.get(credential_id)
                   .map(|v| v.clone()).unwrap_or_default();
    if keys.is_empty() { return Ok(vec![]); }  // always hits — always empty
    // unreachable:
    todo!("Implementation deferred to runtime integration")  // manager.rs:1378
}
// manager.rs:1386-1400 — on_credential_revoked — same pattern
```

**Threat characterization (updated per Phase 1):**
- **Today:** 🟠 HIGH — credential revocations silently dropped. Outstanding guards holding revoked credentials continue to serve traffic until natural refresh (minutes to hours in pooled configs). Effective TTL of a revoked credential extends to pool idle-timeout, not the revocation API call.
- **Latent (becomes 🔴):** any PR adding a write path to `credential_resources` without also implementing the rotation dispatcher will transition today's silent no-op to a reachable `todo!()` panic. Redesign must land reverse-index write + dispatcher **atomically**.

**Cross-reference:** credential Tech Spec `docs/superpowers/specs/2026-04-24-credential-tech-spec.md` **§3.6** (not §15.7 as originally cited) designs rotation as a per-resource **`on_credential_refresh` method on `Resource` trait** (blue-green pool swap). Current code puts it on `Manager` as reverse-index dispatch. Spec↔code structural mismatch — Phase 3 Strategy must reconcile.

**NOT a standalone-fix PR candidate.** Requires atomic reverse-index write + dispatcher + observability — full redesign scope.

### 3.2 🟠 No `deny.toml` containment for `nebula-resource`

`deny.toml` `[bans].deny` explicitly guards `nebula-api`, `nebula-engine`, `nebula-sandbox`, `nebula-storage`, `nebula-sdk`, `nebula-plugin-sdk` with `wrappers = [...]` allowlists. **`nebula-resource` is absent.** 5 direct consumers span business (`action`, `sdk`, `plugin`) and exec (`engine`, `sandbox`) tiers. Layer policy for resource is documentation-only.

**Phase 1 security-lead flagged this as standalone-fix PR candidate SF-1** — mechanical, CI-enforceable, locks consumer set today.

### 3.3 🟠 2101-line `manager.rs` bundles many concerns

Contains: `Manager` + `ManagerConfig` + `RegisterOptions` + `ShutdownConfig` + `DrainTimeoutPolicy` + `ShutdownError` + `ShutdownReport` + `ResourceHealthSnapshot` + internal `GateAdmission` enum + `admit_through_gate` / `settle_gate_admission` / `execute_with_resilience` / `validate_pool_config` / `wait_for_drain` helpers.

**Phase 1 tech-lead verdict:** split the *file*, keep the *type* — Manager is a legitimate coordinator, not a god-object. Internal state is genuinely shared.

### 3.4 🟠 Asymmetric topology ergonomics

- **5 of 7 topologies** have `register_*` / `register_*_with` / `acquire_*` / `try_acquire_*` / `_default` conveniences (Pool, Resident, Service, Transport, Exclusive).
- **2 of 7 topologies** (Daemon, EventSource) require callers to use generic `Manager::register()` with hand-built `TopologyRuntime::Daemon(...)` / `TopologyRuntime::EventSource(...)`.

**Phase 1 dx-tester flagged 🔴:** Daemon has NO public start path — `ManagedResource.topology` is `pub(crate)`, user cannot reach `DaemonRuntime::start()` through `Manager`.

### 3.5 🟠 `AcquireOptions::intent` / `.tags` are reserved fields

`options.rs:18-22, 57-64` docs explicitly state: reserved for future engine integration (#391), no topology reads them. Two public fields with no runtime behavior today. Also `AcquireIntent::Critical`, `ErrorScope::Target { id: String }` (zero producers), `ManagedResource::credential_id` (dead-coded) — all canon §4.5 false-capability violations.

### 3.6 🟡 `macros/` emits `DeclaresDependencies` impls for attribute-declared dependencies, but trait definition is not in the runtime crate

`macros/src/dependencies.rs` parses `#[uses_credential(...)]`, `#[uses_credentials(...)]`, `#[uses_resource(...)]`, `#[uses_resources(...)]` attributes and emits a `DeclaresDependencies` impl. The trait itself does not exist in `crates/resource/src/`. Consumption path is untraceable from the runtime crate alone.

### 3.7 🟡 Runtime-critical crate with no perf gate and no slim mode

- No `benches/` directory.
- Not sharded in `codspeed.yml`.
- No feature flags at all (`default = []`, no optionals).

---

## 4. Known unknowns (Phase 1 probed and confirmed most; see `02-pain-enumeration.md` for results)

1. **`DeclaresDependencies` wiring** — still untraced from runtime crate alone. Phase 3 draft may need to resolve.
2. **Credential×Resource rotation design** — Phase 1 confirmed Tech Spec §3.6 is the reference; current code is a different shape.
3. **Topology runtime `new()` visibility** — Phase 1 tech-lead confirms intentional surface (integration tests use direct construction).
4. **Are `AcquireOptions::intent` / `.tags` blocking?** — Phase 1 agents confirmed reserved/unused; Phase 2 open question.
5. **`ResourceMetadata` `#[non_exhaustive]` with one field** — flagged 🟡 in Phase 1 rust-senior.
6. **`PoolRuntime` 1465 L internal structure** — Phase 1 did not deep-dive; deferred.
7. **`integration/` name collision** — Phase 1 tech-lead flagged 🟡, rename candidate or fold into `options.rs`.

---

## 5. Artefact index (after filesystem event)

| Artefact | Path | Status |
|---|---|---|
| Phase 0 code audit (rust-senior) | `.../phase-0-code-audit.md` | **LOST** (see CASCADE_LOG soft escalation) |
| Phase 0 manifest audit (devops) | `.../phase-0-manifest-audit.md` | **LOST** |
| Phase 1 DX findings | `.../phase-1-dx-tester-findings.md` | **LOST** |
| Phase 1 security findings | `.../phase-1-security-lead-findings.md` | **LOST** |
| Phase 1 rust-senior findings | `.../phase-1-rust-senior-findings.md` | **LOST** |
| Phase 1 tech-lead findings | `.../phase-1-tech-lead-findings.md` | **LOST** |
| This doc (reconstructed) | `.../01-current-state.md` | present |
| Phase 1 consolidation | `.../02-pain-enumeration.md` | **present (canonical)** |
| Cascade log | `.../CASCADE_LOG.md` | present (reconstructed) |

Per-agent findings content is preserved in `02-pain-enumeration.md` §1-§4 severity matrix and convergent-theme narrative. No cascade progress affected.

---

## 6. Phase 0 gate verdict

**PASSES.** Audits were consistent. Phase 1 completed successfully. Phase 2 may proceed.
