# 02 — Pain Enumeration

**Phase:** 1 — Pain enumeration
**Date:** 2026-04-24
**Commit:** `d6cee19f814ff2c955a656afe16c9aeafca16244`
**Gate verdict:** **PASSES (easily).** Six 🔴 findings (deduplicated), 20+ 🟠 findings. Redesign is **clearly justified**. The "nothing-to-redesign" escalation gate does not trigger.

**Inputs (4 parallel agents):**
- `phase-1-dx-tester-findings.md` (680s, 85 tools, ~20 KB) — adopter-authoring surface
- `phase-1-security-lead-findings.md` (575s, 72 tools, 22 findings) — credential boundary + threat model
- `phase-1-rust-senior-findings.md` (698s, 94 tools, 24-row table) — idiomatic Rust review
- `phase-1-tech-lead-findings.md` (429s, 65 tools, 7 sections + priority preview) — architectural coherence

---

## 0. Headline

Three independent themes converge on a common driver, and every agent reached a variant of the same conclusion:

**The credential×resource seam is the primary redesign driver.**

- Security-lead: reverse-index never written → revocations silently dropped today → latent panic when write path lands without dispatcher.
- Tech-lead: `Resource::Auth: AuthScheme` has **zero non-`()` usage in the workspace**; Tech Spec §3.6 designs an incompatible `Resource::Credential: Credential` hook.
- Rust-senior: `Auth` associated type has **zero convenience-path reach** — every `register_*`/`acquire_*` helper bounds `Auth = ()`.
- DX-tester: adopter docs ship **three contradictory stories** for the `Credential`/`Auth` naming.

**Secondary themes** (independently supported by multiple agents):

1. **Doc surface is severely stale/fabricated** — `api-reference.md` fabrication rate ~50%, `adapters.md` compile-fails on 4/7 code blocks, `dx-eval-real-world.rs` references types that don't exist.
2. **Daemon + EventSource topologies are structurally incomplete and canon-out-of-band** — no public start path (pub(crate) barrier), zero Manager-level tests, fall outside canon §3.5 "resource = long-lived connection/SDK client" definition.
3. **Manager surface has `_with` builder anti-pattern + 5-topology × 4-variant cross-product** — 40+ methods, combinatorial. Tech-lead confirms: the type is NOT a god-object (internal state is genuinely shared); the *surface* is the god-object.
4. **Reserved-but-unused public API** (`AcquireOptions::intent`/`.tags`, `ErrorScope::Target`, `AcquireIntent::Critical`) — canon §4.5 false-capability violations.

**Corollary for cascade scope:** the brief's migration-plan machinery ("deprecation window, feature flag strategy, in-repo adapter crate coordination") is **over-engineered** — there are no external adapter crates, 5 in-tree consumers only, MATURITY is `frontier`. Migration is 2-3 `breaking-change` PRs, not a phased rollout.

---

## 1. Convergent findings (cited by 2+ agents)

Confidence on these is highest because independent review angles surface the same underlying issue.

### 1.1 🔴 Credential×Resource seam is structurally wrong
- **Security-lead §1.1 + §1.2:** Silent revocation drop today (reverse-index never written) + spec↔code mismatch (§3.6 places hook on `Resource`, not `Manager`).
- **Tech-lead §4:** `Resource::Auth: AuthScheme` is dead weight (0% non-`()` usage); §3.6 designs `Resource::Credential` with a per-resource blue-green swap hook.
- **Rust-senior §1.2:** `Auth` assoc type has zero convenience-path reach; every `register_*`/`acquire_*_default` bounds `R: Resource<Auth = ()>`.
- **DX-tester §A2:** `crates/resource/README.md:22` says `Credential`; `resource.rs:233` says `Auth`; `dx-eval-real-world.rs:42-65` imports nonexistent `nebula_resource::Credential`. Commit `f37cf609 feat(resource)!: rename Resource::Credential to Resource::Auth` renamed the trait but doc surfaces weren't all updated.

**Severity:** 🔴 CRITICAL (structural). Must be the scope pillar for Phase 2.

**Blocking for standalone fix?** Yes — atomic redesign required (reverse-index + dispatcher + per-resource hook + observability).

### 1.2 🔴 Daemon topology has no public end-to-end path
- **DX-tester §B1:** `Manager::register(..., TopologyRuntime::Daemon(DaemonRuntime::new(...)), ...)` succeeds, but `DaemonRuntime::start()` is only reachable via `ManagedResource.topology` which is `pub(crate)`. Daemon cannot be started through the `Manager` public API.
- **Tech-lead §1 row 7:** 0 Manager-level tests for Daemon; all 3 DaemonRuntime unit tests bypass Manager.
- **Rust-senior §1.6:** EventSource and Daemon are orphan surface — no `Manager::register_daemon*` / `register_event_source*` helpers.

**Severity:** 🔴 CRITICAL (public API incomplete — topology is unusable via intended Manager dispatch).

### 1.3 🔴 `docs/api-reference.md` fabrication + `adapters.md` compile-fail surface
- **DX-tester §A1, §A3, §A4, §C:** 
  - `ResourceConfig: HasSchema` super-trait hidden (`src/resource.rs:37`, never mentioned in adapters.md)
  - `ResourceMetadata { name, description, tags }` 4-field shape fabricated (`adapters.md:242`, `api-reference.md:836`, `dx-eval-real-world.rs:183`) — real struct has ONE field `base: BaseMetadata<ResourceKey>`
  - `ResourceContext::new(ExecutionId)` 1-arg lie vs real 3-arg `new(BaseContext, Arc<dyn ResourceAccessor>, Arc<dyn CredentialAccessor>)`
  - `ResourceContext::with_scope`, `.with_cancel_token` — neither exists in source
  - `AcquireResilience.circuit_breaker` field + `AcquireCircuitBreakerPreset` type — neither exists

**Severity:** 🔴 CRITICAL (newcomer onboarding is broken — following docs verbatim produces compile errors they can't explain from docs alone).

### 1.4 🔴 Drain-abort phase corruption (rust-senior §2)
- **Rust-senior:** `Manager::graceful_shutdown` with `DrainTimeoutPolicy::Abort` (`manager.rs:1493-1510`) flips every resource phase back to `Ready` without recording the failure. The fix helper exists — `ManagedResource::set_failed(error)` at `runtime/managed.rs:93-102` — but is dead-coded behind `#[expect(dead_code, reason = "callers will land with the recovery-error work")]`.
- **Standalone-fix PR candidate** — self-contained single-function fix, no design dependency on credential rotation.

**Severity:** 🔴 CRITICAL (liveness/observability corruption on shutdown path).

### 1.5 🟠 2101-line `manager.rs` — split file, NOT type
- **Tech-lead §2:** "This is a legitimate coordinator. Every field is referenced by multiple public methods. Splitting into multi-managers would require all three to hold `Arc<Registry>` + `Arc<ReleaseQueue>` + drain_tracker — internal state is genuinely shared."
- **Rust-senior §2:** "Dominant style finding: `_with` methods are a named-args workaround that the language now has better answers to."
- **DX-tester §A5:** Convenience helpers require `Auth = ()` — real authenticated adapters drop to 6-arg generic `register()` with no example.

**Severity:** 🟠 HIGH (refactor the file + API surface; keep the type).

**Tech-lead priority-call preview:** "Do NOT split Manager. 2101 L is concentration of coordination, not distribution. Split the *file* (extract options/shutdown into submodules), keep the *type* monolithic."

### 1.6 🟠 Daemon + EventSource are out-of-canon (tech-lead §1 + canon §3.5)
- **Tech-lead §1 + §6:** Canon §3.5 defines Resource as "long-lived managed object (connection pool, SDK client). Engine owns lifecycle." `INTEGRATION_MODEL.md:121` same framing. Neither canon doc mentions daemons or event subscriptions. `TriggerAction` already covers event-driven ingress; long-running workers belong to engine/scheduler layer.
- **Tech-lead priority-call preview:** "Extract Daemon + EventSource. Canon-out-of-band, untested via Manager. Keep Pool / Resident / Service / Transport / Exclusive."

**Severity:** 🟠 HIGH — canon alignment decision for Phase 2.

### 1.7 🟠 Reserved-but-unused public API (canon §4.5 false-capability)
- **Rust-senior §6:** `AcquireOptions::intent` + `.tags` — zero readers (`options.rs:17-64`); `ErrorScope::Target { id: String }` — zero producers; `ManagedResource::credential_id` dead-coded; `AcquireIntent::Critical` — reserved.
- **Tech-lead §7 bullet:** `AcquireIntent::Critical`, `AcquireOptions.{intent,tags}` — public fields with no runtime behavior.
- **DX-tester §A6:** Only one assertion in `adapters.md` was actually copyable (`TopologyTag::Pool`).

**Severity:** 🟠 HIGH — redesign must either remove these or wire them up this cycle. Advertising capability without implementation violates `feedback_incomplete_work.md`.

---

## 2. Unique findings per agent

Items that only one agent surfaced — lower-confidence in isolation but important to capture.

### 2.1 DX-tester — unique

| Sev | Finding | Evidence |
|---|---|---|
| 🟠 | `Resource::Credential` vs `Resource::Auth` three contradictory stories (README=Credential, trait=Auth, adapters.md=Auth, dx-eval-real-world.rs=Credential) | `crates/resource/README.md:22`, `src/resource.rs:233`, `docs/dx-eval-real-world.rs:42-65`, `docs/adapters.md:204` |
| 🟠 | `register_pooled` silently requires `Auth = ()` — no documented escape for real auth | `src/manager.rs:411,446,476,507,538`; adapters.md:354-355 says "use Manager::register directly" with zero example |
| 🟡 | `docs/dx-eval-real-world.rs` — is this a compile target? Unclear purpose | N/A (file exists, no test harness verifying it compiles) |

### 2.2 Security-lead — unique

| Sev | Finding | Evidence |
|---|---|---|
| 🟠 | No `deny.toml` wrappers rule for `nebula-resource` despite 5 consumers across 2 tiers | `deny.toml:41-81` (explicit wrappers for api/engine/sandbox/storage/sdk/plugin-sdk; resource absent) |
| 🟡 | `AuthScheme: Clone` bound forces every secret type cloneable → each clone is another zeroize obligation | `crates/core/src/auth.rs:63`, `crates/credential/src/scheme/secret_token.rs:20` |
| 🟡 | `warmup_pool` calls `R::Auth::default()` — plugin footgun: if a plugin impls `Default` with a zero secret, warmup uses empty credential | `manager.rs:1268` |
| 🟡 | `CredentialId` import split — `nebula_core` vs `nebula-credential` — harder to reason about coupling | `src/manager.rs:26`, `src/runtime/managed.rs:13` |
| ✅ | `#[forbid(unsafe_code)]`, zero CVEs, secrets never hit Debug/Display/log output | (positive findings — noted for completeness) |

**Security-lead standalone-fix PR candidate:** the `deny.toml` wrapper rule — mechanical, CI-enforceable, locks the consumer set today.

### 2.3 Rust-senior — unique

| Sev | Finding | Evidence |
|---|---|---|
| 🟡 | 5 associated types × 5 `acquire_*` methods × 4 variants = combinatorial `where` bounds. 9/9 test resources set `Runtime == Lease` — the separation buys no observed value | `src/resource.rs:220-234`, `src/manager.rs:752-1262` (every acquire method restates `R::Runtime: Into<R::Lease>` style bounds); `tests/basic_integration.rs:106,180,616,1694` |
| 🟡 | `_with` methods + inconsistent `with_*` conventions (constructor vs builder-setter on same type) | `Manager::new + with_config` vs `ShutdownConfig::with_drain_timeout` (different semantics) |
| 🟡 | `Resource::destroy` default `async { Ok(()) }` encourages leaks (compare with `ReleaseQueueHandle::#[must_use]`) | `src/resource.rs:269-275` vs `src/release_queue.rs:65` |
| 🟢 | `fn key()` vs `fn metadata() where Self: Sized` inconsistency | `src/resource.rs:236` vs `:288-298` |
| 🟢 | RPITIT `+ Send` bound is undocumented contract (excludes LocalSet runtimes) | `src/resource.rs:244,252,262,272` + all topology traits |
| 🟢 | `Exclusive` trait is thin — entire behavior is in `ExclusiveRuntime` | `topology/exclusive.rs:19-33` (1 method, default no-op) |

### 2.4 Tech-lead — unique

| Sev | Finding | Evidence |
|---|---|---|
| 🟠 | Transport has zero Manager-level integration tests despite shipping `register_transport` as public API | `tests/basic_integration.rs` (no `register_transport` call sites) |
| 🟠 | Service vs Transport differentiation is defensible but thin — Transport ≈ Service + max_sessions semaphore + keepalive | `runtime/service.rs:80-101` vs `runtime/transport.rs:29-31,85-95` |
| 🟡 | `integration/` module name collides with adapter-integration sense in docs — rename candidate `resilience_policy/` or fold into `options.rs` | `src/integration/mod.rs:1-9` |

---

## 3. Standalone-fix PR candidates (outside cascade scope)

Items multiple agents flagged as fixable independently of the redesign. These should ship as separate PRs to reduce cascade risk.

| # | Finding | Complexity | Agent | Proposed handling |
|---|---------|-----------|-------|-------------------|
| SF-1 | `deny.toml` wrappers rule for `nebula-resource` | Trivial (5-10 lines TOML) | security-lead | Ship as standalone PR; locks consumer set today. Recommend via `devops` agent. |
| SF-2 | Drain-abort phase corruption — wire `ManagedResource::set_failed()` in `graceful_shutdown::Abort` path | Single-function fix | rust-senior | Ship as standalone PR; no design dependency on credential rotation. Removes one of the 🔴 findings. |
| SF-3 | Doc surface corrections — update `api-reference.md`, `adapters.md`, `dx-eval-real-world.rs`, README to match current trait names (`Credential` → `Auth`) and signatures | Medium (doc-only, no code) | dx-tester | Can ship as a docs-only PR, but **only if redesign keeps `Auth`**. If redesign moves back to `Credential` per §3.6 spec, defer to post-redesign. |

**Recommendation:** escalate SF-1 and SF-2 to a small follow-up session after cascade completion. Do NOT touch doc files (SF-3) until scope is locked in Phase 2.

---

## 4. Consolidated severity matrix (deduplicated)

| Sev | # | Finding | Convergent agents |
|-----|---|---------|-------------------|
| 🔴 | 1 | Credential×Resource seam: silent revocation drop + latent panic + spec↔code mismatch | security, tech, rust, dx |
| 🔴 | 2 | Daemon topology has no public start path (`ManagedResource.topology` pub(crate) barrier) | dx, tech, rust |
| 🔴 | 3 | `docs/api-reference.md` ~50% fabrication rate + `adapters.md` compile-fail 4/7 blocks | dx |
| 🔴 | 4 | Drain-abort phase corruption (`DrainTimeoutPolicy::Abort` path) — **standalone-fix candidate SF-2** | rust |
| 🔴 | 5 | `Resource::Auth` bound is dead weight (100% `()`-usage) vs Tech Spec §3.6 `Resource::Credential: Credential` | tech, rust |
| 🔴 | 6 | EventSource same orphan-surface pattern as Daemon — 0 Manager-level tests, no `register_*` helper | dx, tech, rust |
| 🟠 | 7 | `Manager::register_*_with` builder anti-pattern + 2101-line file | rust, tech |
| 🟠 | 8 | Reserved-but-unused public API (`AcquireOptions::intent/.tags`, `ErrorScope::Target`, `AcquireIntent::Critical`) | rust, tech |
| 🟠 | 9 | Daemon + EventSource out-of-canon §3.5 — should extract from crate | tech |
| 🟠 | 10 | No `deny.toml` wrappers rule for resource — **standalone-fix candidate SF-1** | security |
| 🟠 | 11 | 5-assoc-type friction: 9/9 tests set `Runtime == Lease` — unused degree of freedom | rust |
| 🟠 | 12 | `register_pooled` silently requires `Auth = ()` — no documented escape for auth'd adapters | dx, rust |
| 🟠 | 13 | Transport topology — 0 Manager-level integration tests | tech |
| 🟠 | 14 | Missing observability on credential rotation path (no trace span, no counter, no event) | security |
| 🟠 | 15 | `Resource::Credential` vs `Resource::Auth` 3-way doc contradiction | dx |
| 🟡 | 16 | `AuthScheme: Clone` bound forces secret cloneability | security |
| 🟡 | 17 | `warmup_pool` uses `R::Auth::default()` — plugin footgun | security |
| 🟡 | 18 | `CredentialId` split import (nebula_core vs nebula-credential) | security |
| 🟡 | 19 | `_with` convention inconsistent with `Manager::new + with_config` | rust |
| 🟡 | 20 | `Resource::destroy` default no-op encourages leaks | rust |
| 🟡 | 21 | `integration/` module name collision | tech |
| 🟡 | 22 | Service vs Transport differentiation is thin | tech |
| 🟡 | 23 | `docs/dx-eval-real-world.rs` unclear purpose | dx |
| 🟡 | 24 | `ResourceMetadata` is `#[non_exhaustive]` with one field | rust |
| 🟢 | 25 | `fn key()` vs `fn metadata() where Self: Sized` — minor inconsistency | rust |
| 🟢 | 26 | RPITIT `+ Send` undocumented contract | rust |
| 🟢 | 27 | `Exclusive` trait is thin (1 default method) | rust |
| ✅ | 28 | `#[forbid(unsafe_code)]`, zero CVEs, no secret leakage in Debug/Display/log | security (positive) |

**Totals:** 6 🔴 / 9 🟠 / 9 🟡 / 3 🟢 / 1 ✅

---

## 5. Phase 0 corrections

1. **`credential_resources` reverse-index:** Phase 0 said "populated on register but never read by a real dispatcher." **Wrong.** The write site does not exist. Field is declared + init'd empty; only reads are in the two `todo!()` methods. `Manager::register` hardcodes `credential_id: None` on every call (`manager.rs:370`). Corrected inline in `01-current-state.md §3.1`.

2. **Tech Spec §-reference:** Phase 0 cited `§15.7 SchemeFactory integration`. Phase 1 security-lead cites `§3.6` for the rotation hook design. The §15.7 citation may refer to a different aspect of the credential/resource boundary. Phase 2/3 should confirm which is canonical. (Both sections exist; §3.6 is the per-resource rotation hook, §15.7 likely describes factory composition.)

3. **Migration scope:** Phase 0 (via the prompt) assumed the cascade needed deprecation-window / feature-flag / external-adapter-migration machinery. Phase 1 tech-lead confirms: **5 in-tree consumers only, MATURITY = `frontier`, skip the phased rollout.** Redesign ships as 2-3 `breaking-change` PRs per `feedback_hard_breaking_changes.md` + `feedback_no_shims.md`.

---

## 6. Findings explicitly out of scope for Phase 1

Captured for Phase 2/3 consumption.

- **`DeclaresDependencies` trait consumption path** — macros emit an impl but the trait is not defined in `crates/resource/src/`. Likely lives in `nebula-sdk` or `nebula-macro-support`. Grep workspace in Phase 3 draft if it affects trait shape.
- **`docs/plans/`** — 15 pre-implementation planning docs archived in the crate. Whether any still-relevant content needs carrying forward is a Phase 3 archival question per `feedback_incomplete_work.md`.
- **Bench + CodSpeed gap** — confirmed in Phase 0 devops audit. Not evaluated in Phase 1; Phase 3 Strategy should consider whether a bench harness is an explicit redesign deliverable or a follow-up.

---

## 7. Phase 2 handoff

**Phase 2 scope-narrowing co-decision** protocol per prompt:
- architect proposes 2-3 scope options
- tech-lead priority-call
- security-lead blocks if security-critical dropped
- max 3 rounds before escalation

**Tech-lead's priority-call preview** (from findings §7):
> 1. **Primary driver = credential×resource seam.** Reshape `Resource` trait: drop `Auth`, add `AuthenticatedResource: Resource` sub-trait with `type Credential` per Tech Spec §3.6.
> 2. **Prune 7 → 5 topologies.** Extract Daemon + EventSource. Keep Pool / Resident / Service / Transport / Exclusive.
> 3. **Do NOT split Manager.** Split the *file*, keep the *type* monolithic.
> 4. **Migration = internal-only, one PR.** No external adapters, 5 consumers in-tree, MATURITY = frontier.

**Security-lead's Phase 2 input:**
> - Rotation dispatcher + reverse-index write must land atomically. Standalone PR not viable.
> - Spec §3.6 blue-green per-resource swap is materially safer than manager-orchestrated recreation — support extracting `on_credential_refresh` to `Resource` trait.
> - Independent of cascade: push SF-1 (`deny.toml`) to devops as separate PR.

**Rust-senior's Phase 2 input:**
> - Collapse `Runtime`/`Lease` distinction if redesign can (9/9 tests prove it's unused). Or make `Lease = Runtime` the default.
> - Strategy must address `_with` surface — either remove via default-args/Option-builder, or formalize with a single `Register<R>` type.
> - SF-2 (drain-abort) can ship separately to remove one 🔴.

**DX-tester's Phase 2 input:**
> - Doc rewrite is the surface fix but only AFTER the trait shape is locked (don't write the docs twice).
> - Daemon register→start gap must be fixed OR Daemon extracted — either removes the 🔴.
> - New public type names must be docs-first this time — draft adapter walkthrough in Strategy/Tech Spec, not after.

**Open questions for Phase 2 to answer:**
1. `Auth` → `Credential` reshape: drop Auth entirely, make it optional (`AuthenticatedResource` sub-trait), or keep current shape?
2. Topology count: 5 (extract Daemon/EventSource), 6 (merge Service/Transport), or keep 7?
3. `Runtime` vs `Lease` associated types: collapse, default, or keep separate?
4. `AcquireOptions::intent/.tags`: remove, defer to future spec, or wire up this cascade?
5. `manager.rs` split: file-split only, no type change (tech-lead's default)?

These five questions frame Phase 2's co-decision surface.

---

## 8. Budget usage

| Phase | Wall time | Agent effort equivalent |
|---|---|---|
| Phase 0 (2 parallel agents) | ~9 min | ~13 min |
| Phase 1 (4 parallel agents) | ~12 min | ~40 min (680+575+698+429 s) |
| Orchestrator consolidation (0+1) | ~15 min | ~15 min |
| **Cumulative** | **~36 min** | **~68 min** |

**Budget remaining:** well inside the 5-day envelope. Phase 2-8 have ample headroom.

---

## 9. Artefact index

| Artefact | Path |
|---|---|
| This doc | `docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md` |
| DX findings | `.../phase-1-dx-tester-findings.md` + `scratch/probe-{a,b,c}-*.md` |
| Security findings | `.../phase-1-security-lead-findings.md` |
| Rust findings | `.../phase-1-rust-senior-findings.md` |
| Tech-lead findings | `.../phase-1-tech-lead-findings.md` |
| Ground-truth consolidation | `.../01-current-state.md` (corrected §3.1 per Phase 1) |
| Cascade log | `.../CASCADE_LOG.md` |
