---
name: nebula-resource redesign — strategy (frozen)
status: FROZEN (CP3) — approved by architect + spec-auditor + tech-lead
date: 2026-04-24
authors: [architect (subagent dispatch)]
scope: nebula-resource (single-crate redesign; 5 in-tree consumers migrated in one PR wave)
supersedes: []
related:
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/01-current-state.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-3-cp2-tech-lead-ratification.md
  - docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/phase-3-cp2-spec-auditor-review.md
  - docs/superpowers/specs/2026-04-24-credential-redesign-strategy.md
  - docs/superpowers/specs/2026-04-24-credential-tech-spec.md
  - docs/adr/0035-phantom-shim-capability-pattern.md
---

# nebula-resource redesign — strategy (frozen)

**Status:** FROZEN (CP3) — approved by architect + spec-auditor + tech-lead
**Phase:** 3 complete
**Next phase:** Phase 4 (spike) or Phase 5 (ADR) per Phase 2 scope decision
**Date:** 2026-04-24
**Author:** architect (subagent dispatch)
**Scope (LOCKED per [`03-scope-decision.md`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)):**
Option B — Targeted. Credential reshape per credential Tech Spec §3.6 (drop `type Auth`, add `type Credential: Credential`), Daemon/EventSource extraction from the crate, `manager.rs` file-split (keep `Manager` type), atomic migration of 5 in-tree consumers (action, sdk, engine, plugin, sandbox).

---

## §0 — Freeze policy

**Purpose of this document.** Strategy-level decisions that block Phase 4 spike dispatch and Phase 5 ADR authoring. Narrow by design — no Tech Spec content (trait signatures, module layouts, per-consumer migration steps live in Phase 6).

**Not in scope here:**

- Compile-able Rust signatures for the reshaped `Resource` trait or `on_credential_refresh` hook — Phase 4 spike produces those; Phase 6 Tech Spec ratifies.
- Full lifecycle / observability / testing / migration plan — Phase 6 Tech Spec (16 sections per cadence in [`03-scope-decision.md`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) §6).
- Sub-spec material (Daemon/EventSource landing site design in engine/scheduler, `AcquireOptions::intent/.tags` engine integration per #391, bench harness design) — tracked in §6 post-validation roadmap when CP3 lands; CP2 §5 records open items that frame what §6 must address.

**Checkpoint path.**

1. **Checkpoint 1** (committed; ratification edits applied in CP2): §0 freeze policy + §1 problem statement + §2 constraints + §3 options analysis. Blocks Phase 4 spike dispatch.
2. **Checkpoint 2** (this draft): §4 decision record (what CP2 ratifies as in-scope) + §5 open items that Phase 6 Tech Spec must resolve. Blocks Phase 5 ADR authoring.
3. **Checkpoint 3** (planned): §6 post-validation roadmap — sub-spec queue, deferred-findings pointers, hand-off to Phase 6 Tech Spec. Strategy freezes on CP3 signoff.

**Freeze policy.** CP3 signoff = Strategy freeze. After freeze:

- **Amendments that change scope or locked decisions** (§1 problem framing, §2 constraints, §3 options record, §4 decisions when ratified) require a new co-decision cycle — architect drafts amendment rationale, tech-lead priority-calls, security-lead security-gates. Same 3-reviewer protocol as Phase 2.
- **Bug fixes** (typo, broken intra-doc link, clarification that does not change semantics) land directly via a normal PR with "docs(strategy)" prefix. Reviewer: spec-auditor.
- **Down-stream clarifications** that surface during Phase 6 Tech Spec drafting and reveal Strategy §-level ambiguity (e.g., "§4.2 said 'parallel dispatch' but is silent on back-pressure during a concurrent revocation") are recorded in the Tech Spec with explicit "extends Strategy §X.Y" annotation. If the extension changes Strategy semantics, escalate to amendment cycle per above.

**Amendments via ADR.** Strategy-level decisions in §4 (when CP2 ratifies them) are candidates for ADR extraction — each ratified decision should be paired with an ADR citation in CP3. When a future ADR supersedes a §4 decision, the amending ADR is linked inline at the amended paragraph (pattern per credential Strategy §0: `*Superseded by ADR-NNNN*` prefix + replacement text below).

**Relationship to draft materials.** Artefacts in [`docs/superpowers/drafts/2026-04-24-nebula-resource-redesign/`](../drafts/2026-04-24-nebula-resource-redesign/) (phases 0-2) are **archival** after Strategy freeze. This Strategy is the canonical record; drafts remain for cascade audit trail. Phase 4 spike artefacts will live on an isolated worktree branch (not in `docs/superpowers/drafts/`) per cascade worktree convention (see credential Strategy §5.12 for the pattern).

**Reading order.** §0 → §1 (why redesign) → §2 (what constrains) → §3 (what was considered at Phase 2, why Option B). §4 (decisions) + §5 (open items) land in CP2; §6 (roadmap) lands in CP3.

**Pointer to deferred work.** Nothing is silently dropped. Phase 1 surfaced 28 findings in [`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) §4; verified split per [`03-scope-decision.md`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) §1-§3:

- **12 in-scope** (Option B addresses) — `03-scope-decision.md` §1 table.
- **6 deferred-with-pointer** (future cascade / cross-crate dependency) — 🟠-8 (`AcquireOptions::intent/.tags` → engine #391; CP2 §5.2 records as open item; Phase 6 §5 picks interim treatment), 🟠-11 (`Runtime`/`Lease` collapse → future cascade; cross-ref §5.3), 🟠-13 (Transport-test debt → follow-up task post-cascade, issue filed by orchestrator), 🟡-16 (`AuthScheme: Clone` → credential Tech Spec §3.6+ extension), 🟡-18 (`CredentialId` split import → drive-by future PR), 🟡-20 (`Resource::destroy` default no-op → Phase 4 spike may surface).
- **3 absorbed-as-cleanup** (effectively in-scope under Option B) — 🟡-19 (`_with` convention → resolved by file-split), 🟡-21 (`integration/` module rename → absorbed into file-split), 🟡-23 (`dx-eval-real-world.rs` → absorbed into Phase 6 doc rewrite).
- **5 accepted-as-is** — 🟡-22 (Service/Transport differentiation; future cascade if evidence mounts), 🟡-24 (`ResourceMetadata` `#[non_exhaustive]` cosmetic), 🟢-25/-26/-27 (minor inconsistencies; no action).
- **1 standalone-fix** (ships independently of cascade) — 🟠-10 (SF-1: `deny.toml` wrappers rule → devops standalone PR, [`03-scope-decision.md §3`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)). SF-2 (drain-abort) **absorbed** into Option B (was originally a candidate, now in-scope).
- **1 positive finding** preserved — ✅-28 (`#[forbid(unsafe_code)]`, zero CVEs, no Debug/Display leakage; Phase 6 Tech Spec §6 preserves invariants).

Total: 12 + 6 + 3 + 5 + 1 + 1 = 28 ✓. CP3 §6 re-tables deferred-with-pointer if any shift during Phase 4-6.

## §1 — Problem statement

Six convergent pain drivers surfaced in Phase 1 review. Each subsection states the symptom, cites the primary evidence, and states the impact if unaddressed. Not all 28 Phase 1 findings are re-listed here — the canonical enumeration lives in [`02-pain-enumeration.md`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) §4. This section selects the drivers Option B addresses.

### §1.1 Credential rotation surface is structurally wrong

**Symptom.** The `Resource` trait advertises `type Auth: AuthScheme` as a credential binding mechanism; `Manager` advertises `on_credential_refreshed` and `on_credential_revoked` as rotation dispatch entry points. Neither works. `type Auth` is dead weight in practice; the rotation dispatch silently drops all events today and will panic the moment anyone tries to populate the reverse-index.

**Evidence.**

- [`crates/resource/src/resource.rs:220-244`](../../../crates/resource/src/resource.rs) — `Resource` trait declares `type Auth: AuthScheme` on line 233; `create` signature takes `auth: &Self::Auth` on line 242. Phase 1 finding 🔴-5 ([`02-pain-enumeration.md:166`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)) confirms 100% `()` usage across workspace: every in-tree `register_*`/`acquire_*` convenience helper bounds `R: Resource<Auth = ()>`.
- [`crates/resource/src/manager.rs:262`](../../../crates/resource/src/manager.rs) — `credential_resources: DashMap<CredentialId, Vec<ResourceKey>>` declared (reverse-index). [`crates/resource/src/manager.rs:370`](../../../crates/resource/src/manager.rs) — `Manager::register` hardcodes `credential_id: None` on every registration. **The write site does not exist anywhere in the codebase** (Phase 1 correction, [`01-current-state.md:96-117`](../drafts/2026-04-24-nebula-resource-redesign/01-current-state.md)).
- [`crates/resource/src/manager.rs:1360-1378`](../../../crates/resource/src/manager.rs) — `on_credential_refreshed` reads the reverse-index, always finds empty, returns `Ok(vec![])`. The `todo!()` on line 1378 is unreachable today but becomes a reachable panic if any PR adds a reverse-index write without also implementing dispatch. [`crates/resource/src/manager.rs:1386-1400`](../../../crates/resource/src/manager.rs) — `on_credential_revoked` has the same shape (latent panic on line 1400).
- Credential Tech Spec §3.6 ([`docs/superpowers/specs/2026-04-24-credential-tech-spec.md:928-996`](2026-04-24-credential-tech-spec.md)) designs rotation as a per-resource `on_credential_refresh` method on the `Resource` trait (blue-green pool swap), receiving `&<Self::Credential as Credential>::Scheme`. The current code puts dispatch on `Manager` against a reverse-index — structurally the wrong surface.
- Phase 1 convergence: security-lead (🔴-1 silent revocation drop, `02-pain-enumeration.md:42-50`), tech-lead (🔴-5 `Auth` dead weight, same reference), rust-senior (zero convenience-path reach), dx-tester (🟠-15 three-way doc contradiction `Credential` vs `Auth`).

**Impact.** Credential revocations silently drop today. Outstanding guards holding revoked credentials continue to serve traffic until natural refresh (minutes to hours in pooled configs). Effective TTL of a revoked credential extends to pool idle-timeout, not the revocation API call — the tenant believes they revoked but the resource continues issuing authenticated queries. A PR adding a reverse-index write without the dispatcher transitions this from silent-drop to a reachable `todo!()` panic (liveness regression). `Auth` as a trait-level associated type is operationally honest-only if every resource actually uses it; 100% `()` usage is a canon §4.5 false-capability signal at the trait level.

### §1.2 Daemon and EventSource topologies are orphan surface

**Symptom.** Two of seven topologies (Daemon, EventSource) ship without a public end-to-end path. `Manager::register` accepts them, but callers cannot reach their runtime-specific start methods without the field visibility barrier blocking access.

**Evidence.**

- [`crates/resource/src/runtime/managed.rs:35`](../../../crates/resource/src/runtime/managed.rs) — `ManagedResource::topology` is `pub(crate)`. External callers cannot reach `DaemonRuntime::start()` through this field.
- No `Manager::register_daemon*` or `Manager::register_event_source*` convenience helpers exist (verified against `manager.rs` method enumeration in [`01-current-state.md:135-139`](../drafts/2026-04-24-nebula-resource-redesign/01-current-state.md) §3.4).
- Zero Manager-level integration tests for Daemon or EventSource (Phase 1 finding 🔴-2 / 🔴-6, [`02-pain-enumeration.md:161-162`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)). The 3 existing `DaemonRuntime` tests bypass `Manager` entirely.
- `PRODUCT_CANON.md §3.5` defines Resource as "long-lived managed object (connection pool, SDK client). Engine owns lifecycle." Canon does not mention daemons or event subscriptions as Resource variants. [`docs/INTEGRATION_MODEL.md:89-91`](../../INTEGRATION_MODEL.md) reinforces: "Long-lived managed object: connection pool, SDK client, file handle." Long-running workers and event subscriptions belong to the engine/scheduler layer (`TriggerAction` already covers event-driven ingress per canon §3.5).
- Phase 1 convergence: dx-tester (🔴-2 no public start path), tech-lead (🟠-9 out-of-canon §3.5), rust-senior (🔴-6 orphan surface — no `register_*` helper for 2 of 7 topologies).

**Impact.** Two public-API topologies are unusable via intended dispatch. A newcomer following [`docs/api-reference.md`](../../../crates/resource/docs/api-reference.md) can register a daemon but cannot start it. The canon §3.5 drift creates a structural coupling: anything useful about Daemon/EventSource must be rebuilt either inside `nebula-resource` (going further out-of-canon) or outside it (forcing extraction eventually). Every cascade of features added to Daemon/EventSource inside this crate compounds the debt.

### §1.3 Documentation surface rot

**Symptom.** The crate's user-facing documentation misrepresents the current API at approximately 50% rate. A newcomer following the adapter walkthrough produces code that does not compile and cannot explain the failure from documentation alone.

**Evidence.**

- [`crates/resource/docs/api-reference.md`](../../../crates/resource/docs/api-reference.md) — Phase 1 dx-tester measured ~50% fabrication rate ([`02-pain-enumeration.md:63-66`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) §1.3). Specific fabrications: `ResourceConfig: HasSchema` super-trait hidden (real: [`crates/resource/src/resource.rs:37`](../../../crates/resource/src/resource.rs)); `ResourceMetadata { name, description, tags }` 4-field shape (real: one field `base: BaseMetadata<ResourceKey>`); `ResourceContext::new(ExecutionId)` 1-arg (real: 3-arg `new(BaseContext, Arc<dyn ResourceAccessor>, Arc<dyn CredentialAccessor>)`); `ResourceContext::with_scope` and `.with_cancel_token` (neither exists); `AcquireResilience.circuit_breaker` field + `AcquireCircuitBreakerPreset` type (neither exists).
- [`crates/resource/docs/adapters.md`](../../../crates/resource/docs/adapters.md) — Phase 1 dx-tester: 4 of 7 code blocks compile-fail against current source. Adapters presented as named crates (`nebula-resource-postgres`, `nebula-resource-redis`) do not exist anywhere in the workspace ([`01-current-state.md:87`](../drafts/2026-04-24-nebula-resource-redesign/01-current-state.md)).
- [`crates/resource/docs/dx-eval-real-world.rs:42-65`](../../../crates/resource/docs/dx-eval-real-world.rs) imports `nebula_resource::Credential` — a symbol that does not exist (the trait is [`Resource::Auth`](../../../crates/resource/src/resource.rs) per commit `f37cf609 feat(resource)!: rename Resource::Credential to Resource::Auth`, which renamed but did not propagate to doc surfaces). 
- [`crates/resource/README.md:22`](../../../crates/resource/README.md) says `Credential`; [`crates/resource/src/resource.rs:233`](../../../crates/resource/src/resource.rs) says `Auth`; [`crates/resource/docs/adapters.md:204`](../../../crates/resource/docs/adapters.md) says `Auth`; `dx-eval-real-world.rs` says `Credential`. Three-way contradiction (🟠-15).
- [`crates/resource/docs/events.md`](../../../crates/resource/docs/events.md) lists 7 `ResourceEvent` variants; [`crates/resource/src/events.rs`](../../../crates/resource/src/events.rs) has 10 variants ([`01-current-state.md:81`](../drafts/2026-04-24-nebula-resource-redesign/01-current-state.md)).

**Impact.** Newcomer adoption signal is broken. Phase 1 dx-tester flagged that no single doc file can be trusted end-to-end (only one copyable assertion survived review — `TopologyTag::Pool`, [`02-pain-enumeration.md:93-94`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)). Fixing this *before* trait shape locks means writing the docs twice (per dx-tester's Phase 2 input, [`02-pain-enumeration.md:239-242`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)) — `feedback_incomplete_work.md` applies. Docs must be rewritten atomically with the trait reshape, not standalone.

### §1.4 Manager orchestration surface is a grab-bag

**Symptom.** `crates/resource/src/manager.rs` is a 2101-line file bundling `Manager`, 7+ config/error types, 40+ public methods (including a combinatorial `register_*_with` surface), internal gate logic, and shutdown helpers. The `Manager` type itself is a legitimate coordinator (internal state is shared across all methods), but the *surface* is unmanageable.

**Evidence.**

- [`crates/resource/src/manager.rs`](../../../crates/resource/src/manager.rs) — 2101 lines. Phase 1 rust-senior measured ([`02-pain-enumeration.md:125-131`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md) §2.3). Tech-lead confirmed the type is shared-state concentration, not god-object ([`02-pain-enumeration.md:76`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)): "every field is referenced by multiple public methods."
- Asymmetric topology ergonomics: 5 of 7 topologies (Pool, Resident, Service, Transport, Exclusive) have `register_*` / `register_*_with` / `acquire_*` / `try_acquire_*` / `_default` conveniences; Daemon and EventSource have none ([`01-current-state.md:135-139`](../drafts/2026-04-24-nebula-resource-redesign/01-current-state.md) §3.4).
- `_with` builder anti-pattern: every convenience method has a paired `_with` variant (5 variants at [`crates/resource/src/manager.rs:561, 597, 627, 659, 691`](../../../crates/resource/src/manager.rs) per [`03-scope-options.md:415`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-options.md)). Phase 1 rust-senior: "named-args workaround the language now has better answers to" ([`02-pain-enumeration.md:77`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)).
- [`crates/resource/src/manager.rs:411,446,476,507,538`](../../../crates/resource/src/manager.rs) — every `register_*` convenience bounds `R: Resource<Auth = ()>` ([`02-pain-enumeration.md:107-108`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)). Authenticated adapters fall back to the 6-arg generic `register()` with no documented example.

**Impact.** The 2101-line file is a bandwidth tax on every review and every new contributor's orientation. The asymmetric ergonomics (5 of 7 topologies have conveniences) sends a message that the two without are either second-class or meant to be extracted. The `_with` surface with `Auth = ()` bound silently forces the ergonomic path to credential-less resources only, contradicting the stated `Auth` abstraction. Splitting the file (but keeping the `Manager` type) aligns the surface with the internal reality: one coordinator, many submodules of helpers.

### §1.5 Drain-abort phase corruption

**Symptom.** `Manager::graceful_shutdown` with `DrainTimeoutPolicy::Abort` flips every resource phase back to `Ready` without recording the failure. The fix helper exists but is dead-coded. Operators cannot detect the failed-drain state from the resource phase alone.

**Evidence.**

- [`crates/resource/src/manager.rs:1493-1510`](../../../crates/resource/src/manager.rs) — `DrainTimeoutPolicy::Abort` branch calls `self.set_phase_all(ResourcePhase::Ready)` on line 1507, then returns `Err(ShutdownError::DrainTimeout { outstanding })`. The phase is `Ready`; the shutdown has failed; the two signals disagree.
- [`crates/resource/src/runtime/managed.rs:93-102`](../../../crates/resource/src/runtime/managed.rs) — `set_failed(error)` helper exists, sets `phase: ResourcePhase::Failed` + `last_error: Some(error)`. Decorated with `#[expect(dead_code, reason = "callers will land with the recovery-error work")]`. The recovery-error work never landed for this path.
- Phase 1 rust-senior 🔴-4 ([`02-pain-enumeration.md:69-74`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)).

**Impact.** Operators reading `ResourceStatus` after a failed drain see `phase: Ready` — indistinguishable from a successful graceful shutdown that reached steady state. The audit trail for a drain timeout is split across a tracing warning (ephemeral) and an error return (caller-observable but lost to the audit log). This is a §12.6 observability-honesty violation: the phase is the advertised capability for "is the resource healthy?" and it lies on the Abort path.

### §1.6 Observability gaps on credential rotation path

**Symptom.** The credential rotation dispatch path has no trace span, no counter, and no event variant. Any rotation — successful or failed — is invisible to operators.

**Evidence.**

- [`crates/resource/src/manager.rs:1360-1401`](../../../crates/resource/src/manager.rs) — the two `on_credential_*` methods have no `tracing::span!` wrapping, no counter increment, no event emission. They are `todo!()` bodies, but even the outer scaffolding (reverse-index read, `keys.is_empty()` guard) emits nothing.
- [`crates/resource/src/events.rs`](../../../crates/resource/src/events.rs) has 10 `ResourceEvent` variants; none is `CredentialRefreshed` or equivalent. A rotation that *did* land (via a future PR) would have no event variant to broadcast.
- Phase 1 security-lead 🟠-14 ([`02-pain-enumeration.md:175`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)): "Missing observability on credential rotation path (no trace span, no counter, no event)."

**Impact.** Incident response on a credential compromise requires knowing whether the revocation dispatch fired and whether the resources acted on it. Without instrumentation, operators get silent-success indistinguishable from silent-drop. `feedback_observability_as_completion.md` applies: observability is Definition of Done for a hot path, not a follow-up. Landing the rotation redesign without trace + counter + event would reproduce the same "advertised-but-unverifiable" gap the redesign is supposed to close.

## §2 — Constraints

What bounds the redesign. Dense citation-heavy — no narrative. Each bullet identifies the source and what it forbids or requires.

### §2.1 ADR references

- [**ADR-0035 — Phantom-shim capability pattern**](../../adr/0035-phantom-shim-capability-pattern.md). *For reference, not binding on this redesign.* The resource crate's `TopologyTag` is a concrete runtime `#[non_exhaustive] enum` ([`crates/resource/src/topology_tag.rs`](../../../crates/resource/src/topology_tag.rs); [`01-current-state.md §3.1, line 88`](../drafts/2026-04-24-nebula-resource-redesign/01-current-state.md): "**Brief was wrong. Corrected.**") — **not a phantom-tag** pattern. ADR-0035 applies to credential-side phantom-shim capability traits (`BitbucketBearerPhantom` etc.); it neither constrains nor permits resource-side design. Cited here so Phase 4 spike and Phase 6 Tech Spec do not accidentally invoke phantom-shim framing for topology dispatch.
- **ADR candidate (Phase 5 deliverable).** Per [`03-scope-decision.md §6, line 168`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md), Phase 5 authors at minimum 1 ADR: "`Resource::Credential` adoption and `Auth` retirement." Possibly a second for Daemon/EventSource extraction. These ADRs are not yet written; this Strategy frames the problem they will record.

### §2.2 Canon references

- **`PRODUCT_CANON.md §3.5` ([line 80](../../PRODUCT_CANON.md)):** *"Resource — long-lived managed object (connection pool, SDK client). Engine owns lifecycle."* Canon does not mention daemons, workers, or event subscriptions. Resource is pool/SDK client only. Daemon and EventSource are out-of-band per §1.2 above.
- **`PRODUCT_CANON.md §3.5` ([line 81](../../PRODUCT_CANON.md)):** *"Credential — who you are and how authentication is maintained. Engine owns resolver/rotation orchestration (not the credential crate)."* Locates rotation orchestration at the engine layer; the resource-side hook (`on_credential_refresh`) is how engine-initiated rotation reaches per-resource handlers.
- **`PRODUCT_CANON.md §4.5` ([lines 131-138](../../PRODUCT_CANON.md)):** *"Public surface exists iff the engine honors it end-to-end."* Shapes treatment of:
  - `type Auth` (100% `()` usage = false capability at the trait level — must remove or deliver).
  - `AcquireOptions::intent/.tags` ([`crates/resource/src/options.rs:17-64`](../../../crates/resource/src/options.rs) — reserved for #391, zero readers = false capability). Per [`03-scope-decision.md §2, line 54`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md), deferred to future cascade; interim treatment (mark `#[doc(hidden)]` or `#[deprecated]`) to be decided in CP2 §4.
  - `AcquireIntent::Critical`, `ErrorScope::Target { id: String }`, `ManagedResource::credential_id` dead-coded — same class.
- **`PRODUCT_CANON.md §12.5` ([lines 386-392](../../PRODUCT_CANON.md)):** Secrets invariants — no secrets in logs/errors/metrics labels; `Zeroize`/`ZeroizeOnDrop` on key material; `redacted Debug` on credential wrappers; every `tracing::*!` taking a credential must use redacted forms; credential operations emit metrics through `CredentialMetrics`. Applies to §1.6 observability: trace spans on the rotation path must use redacted forms; counters must emit via `CredentialMetrics` (or a resource-side equivalent that composes with `CredentialMetrics`, CP2 to decide).
- **`PRODUCT_CANON.md §12.7` ([lines 399-402](../../PRODUCT_CANON.md)):** *"A module that grows past a few hundred lines and mixes unrelated responsibilities is a refactor, not a feature — split before adding."* Applies to §1.4: 2101-line `manager.rs` must be file-split in this cascade. Type stays; file splits into submodules.

### §2.3 Cross-crate contracts

- **Credential Tech Spec §3.6** ([`docs/superpowers/specs/2026-04-24-credential-tech-spec.md:928-996`](2026-04-24-credential-tech-spec.md)). *Normative for this redesign.* Defines:
  - Trait shape: `type Credential: Credential` on `Resource` directly (line 936).
  - `create` signature: `async fn create(ctx: &ResourceContext<'_>, scheme: &<Self::Credential as Credential>::Scheme) -> Result<Self, Self::Error>` (lines 939-944).
  - Rotation hook: `async fn on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme) -> Result<(), Self::Error>` with default no-op body (lines 949-955).
  - Blue-green pool swap pattern (lines 961-993) — `Arc<RwLock<Pool>>` + write-lock swap inside the resource impl; Manager never holds the new scheme across an `await` longer than the dispatch call.
- **Credential Tech Spec §3.6 silence on revocation.** Spec defines `on_credential_refresh` only. Does not define `on_credential_revoke` or equivalent. Per security-lead amendment B-2 ([`phase-2-security-lead-review.md:67-74`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)), **CP2 §4 must extend §3.6 with revoke semantics**. Candidate approaches (CP2 to pick): (a) `on_credential_refresh` carries both semantics (resource decides how to tear down when the scheme is revoked); (b) separate `on_credential_revoke` method. Either way, the dispatcher must emit `HealthChanged { healthy: false }` per-resource on revocation.
- **Engine integration ticket #391** (referenced [`crates/resource/src/options.rs:17-64`](../../../crates/resource/src/options.rs)). Out-of-scope for this cascade per [`03-scope-decision.md §2](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) — `AcquireOptions::intent/.tags` wiring awaits engine-side design.
- **Workspace consumer set (5 in-tree):** `nebula-action`, `nebula-sdk`, `nebula-engine`, `nebula-plugin`, `nebula-sandbox` per [`03-scope-decision.md §4.7`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md). Phase 6 Tech Spec §13 enumerates per-consumer migration changes.
- **No external adapters.** Workspace grep confirms no `nebula-resource-*` adapter crates exist ([`01-current-state.md:87`](../drafts/2026-04-24-nebula-resource-redesign/01-current-state.md)). Brief's deprecation-window / feature-flag machinery is unnecessary — migration is a bundled breaking-change PR per [`03-scope-decision.md §4.7`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md).
- **Engine does not consume Daemon/EventSource.** Tech-lead verified ([`phase-2-tech-lead-review.md:72-74`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md)): grep of `crates/engine/` for `DaemonRuntime|EventSourceRuntime|TopologyTag::Daemon|TopologyTag::EventSource` returns zero hits. Extraction does not force a replacement primitive in engine. CP3 Strategy must re-verify with a broader workspace grep before Phase 6 migration lands.

### §2.4 Toolchain and quality invariants

- **Rust 1.95.0 pinned** (`rust-toolchain.toml`). MSRV for all crate changes.
- **RPITIT throughout** ([`01-current-state.md:52-53`](../drafts/2026-04-24-nebula-resource-redesign/01-current-state.md)): all async trait methods use `impl Future<…> + Send`, no `async_trait`, no `Box<dyn Future>`. Reshaped `Resource` trait must preserve this.
- **`#[forbid(unsafe_code)]`** on the crate — Phase 1 confirmed ([`02-pain-enumeration.md:119`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md), positive finding ✅-28). Reshape preserves.
- **`default-features = []`** on `Cargo.toml` — zero feature flags today ([`01-current-state.md:28-29`](../drafts/2026-04-24-nebula-resource-redesign/01-current-state.md)). Redesign does not introduce features; if a future cascade splits credential-bearing from credential-less paths behind features, that is a separate decision.
- **`feedback_no_shims.md`** — no adapter / bridge / shim layers between old and new shape. `Auth` is removed, not forwarded. Per [`03-scope-decision.md §4.7`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md).
- **`feedback_hard_breaking_changes.md`** — expert-level breaking changes acceptable given `frontier` maturity (§2.5 below) and 5 in-tree consumers.
- **`feedback_observability_as_completion.md`** — trace span + counter + event = DoD for the rotation path. Phase 6 Tech Spec CP review has an explicit observability gate ([`03-scope-decision.md §4.4`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)).
- **`feedback_boundary_erosion.md`** — Daemon/EventSource extraction must land a defined landing site (engine/scheduler fold OR sibling crate, CP2 §4 picks). "Extract from resource, figure out where later" is boundary erosion.
- **`feedback_active_dev_mode.md`** — `frontier` maturity + active-dev posture means breaking changes ship now, not deferred to a hypothetical "v0.2 stability boundary". Justifies the bundled-PR-wave / no-deprecation-window posture in §2.5 below. Also justifies the "remove `Auth`, don't dual-shape" position in §2.4 above (a hypothetical compat surface would be expedient but not ideal).
- **Spike exit criteria (Phase 4):** do NOT include sub-trait fallback. The `Resource::Credential` shape per credential Tech Spec §3.6 is the locked target. If §3.6 ergonomics or perf fail the spike, escalate to Phase 2 round 2 — not a mid-flight shape change. (Encodes Phase 2 tech-lead amendment 1, [`phase-2-tech-lead-review.md:80-94`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md).)
- **No `Scheme::default()` at warmup** — security-lead amendment B-3 ([`phase-2-security-lead-review.md:76-82`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)). Current code at [`crates/resource/src/manager.rs:1268`](../../../crates/resource/src/manager.rs) calls `R::Auth::default()`; new shape must not reproduce with `R::Credential::Scheme::default()`. Strategy CP2 §4 specifies warmup semantics (likely: warmup requires a real scheme, or skips for credential-bearing pools).
- **No `clone()` on secret schemes in the dispatcher hot path** — security-lead constraint 7 ([`phase-2-security-lead-review.md:118`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)). Dispatcher passes `&Scheme`, not owned `Scheme`. Each clone is another zeroize obligation.

### §2.5 Maturity and release posture

- **`nebula-resource` maturity: `frontier`** ([`docs/MATURITY.md:36`](../../MATURITY.md)). `frontier` = design-stable, interfaces-stable, behavior-stable, observability-partial. Breaking changes are expected in the `frontier` stability class; no deprecation window obligation to external adopters.
- **Zero external adopters to protect.** No downstream crates outside the workspace consume `nebula-resource` ([`01-current-state.md:87`](../drafts/2026-04-24-nebula-resource-redesign/01-current-state.md) — `docs/adapters.md` is aspirational).
- **Breaking changes land in one bundled PR wave** per [`03-scope-decision.md §4.7`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md). Trait reshape + 5 consumer migrations + doc rewrite + rotation dispatcher + observability = one atomic migration.

## §3 — Options analysis

Historical record. Phase 2 considered three scope options; co-decision body (architect + tech-lead + security-lead) unanimously picked Option B in round 1 of the max-3 protocol (with 2 tech-lead amendments and 3 security-lead amendments tightening the in-scope envelope; all endorsed in the single round, per the per-review pointers below). Full comparison matrix in [`03-scope-options.md`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-options.md) §"Comparison matrix" (line 350); this subsection captures only the narrative record of what was considered and why Option B won.

### §3.1 Option A — Minimal (BLOCKED by security-lead)

**What it was.** Doc rewrite + two standalone fixes (SF-1 `deny.toml` containment, SF-2 drain-abort phase corruption). No `Resource` trait reshape. No topology extraction. Credential×Resource seam deferred to a "follow-up project" outside the cascade. Option A's doc rewrite would have been written against the current `Auth`-shaped trait. Full specification: [`03-scope-options.md §"Option A"`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-options.md) (lines 31-108).

**Why it was rejected.** Security-lead BLOCKED on the grounds that 🔴-1 (silent revocation drop + latent `todo!()` panic on any reverse-index write) cannot be resolved by deferral. Phase 1 security position ([`02-pain-enumeration.md:229-231`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)) was "atomic landing, standalone PR not viable" — Option A's "atomically, eventually, in a follow-up project" framing does not satisfy the atomicity invariant of *today's* trunk. Security-lead verdict in full: [`phase-2-security-lead-review.md §"Option A — BLOCK"`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md) (lines 25-46). Tech-lead independently dismissed on `feedback_incomplete_work.md` grounds ([`phase-2-tech-lead-review.md §"Q1"`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md)): writing the doc rewrite against an `Auth`-shaped trait known to be superseded by credential Tech Spec §3.6 is "don't write the docs twice" at the architecture level.

### §3.2 Option B — Targeted (chosen)

**What it is.** Reshape the `Resource` trait per credential Tech Spec §3.6 verbatim (`type Credential: Credential`, `on_credential_refresh` hook). Extract Daemon and EventSource from the crate (landing site chosen in CP2 §4). Split `manager.rs` file (keep `Manager` type monolithic). Rewrite adapter documentation against the new shape. Absorb SF-2 (drain-abort fix) into the Manager file-split PR. Migrate 5 in-tree consumers atomically. Ship rotation observability (trace span + counter + event) in the same PR as dispatcher.

**Why it was picked.** Convergent endorsement from both co-deciders. Tech-lead priority-called Option B with two bounded amendments (lock §3.6 shape, no sub-trait fallback during spike; make rotation observability explicit DoD) — full rationale in [`phase-2-tech-lead-review.md §"Priority call"`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md) (lines 10-16): B is the smallest scope that atomically closes the credential seam, aligns 1:1 with the Phase 1 preview at the 4-point level, and fits the 5-day envelope with margin. Security-lead ENDORSED B with three amendments (isolation invariant on concurrent dispatch, revocation extension of §3.6, warmup footgun resolution) — full rationale in [`phase-2-security-lead-review.md §"Option B"`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md) (lines 47-82). The atomic landing, §3.6 blue-green pattern, and observability-as-DoD were already baked into B.2 🔴-1 treatment — security-lead's amendments tightened rather than redirected. Locked in round 1 of the max-3-rounds co-decision protocol.

### §3.3 Option C — Comprehensive (rejected)

**What it was.** Everything in Option B, plus: collapse `Runtime`/`Lease` distinction (9/9 test resources set `Lease = Runtime`) via associated-type default; resolve `AcquireOptions::intent/.tags` (C.8a remove entirely, or C.8b wire documented semantics); optional Service/Transport topology merge if spike evidence supported. Full specification: [`03-scope-options.md §"Option C"`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-options.md) (lines 255-346).

**Why it was rejected.** Added surface the Phase 1 evidence did not force:

- **`Runtime`/`Lease` collapse** — friction is real but orthogonal to the credential driver. Can land in a future cascade with a standalone ADR without coupling to this cascade's critical path.
- **`AcquireOptions::intent/.tags` resolution** — blocked on engine-side design (ticket #391) that does not yet exist. Resolving inside this cascade would be guessing; per security-lead ([`phase-2-security-lead-review.md:96`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)) C.8a (remove) is the least-harm interim but not required for the credential seam fix.
- **Service/Transport merge** — tech-lead flagged differentiation as "defensible but thin" ([`02-pain-enumeration.md:139`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)); not forced by evidence. `feedback_boundary_erosion.md` applies: merging without strong evidence trades one unclear boundary for another.

Schedule risk was secondary: Option C's ~4-5 day estimate bumps up against the 5-day envelope with no margin ([`03-scope-options.md:346`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-options.md)). Tech-lead deferred with a "not now, not no" framing ([`phase-2-tech-lead-review.md §"Q4"/"Q5"`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md)): each C-only sub-decision can land standalone in a later cascade without coupling to the credential seam fix. Security-lead: security-neutral relative to B ([`phase-2-security-lead-review.md §"Option C"`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)) — no security argument for C over B, no blocking objection either.

## §4 — Decision record

What CP2 ratifies as the in-scope shape of Option B. Decisions here become the binding contract for Phase 4 spike, Phase 5 ADR, and Phase 6 Tech Spec. Each subsection is dense by design: question / decision / rationale / cross-refs. Implementation lives in Phase 6, not here.

### §4.1 Resource trait reshape

**Decision.** Replace `type Auth: AuthScheme` on the `Resource` trait with `type Credential: Credential` per credential Tech Spec §3.6 ([`docs/superpowers/specs/2026-04-24-credential-tech-spec.md:935-956`](2026-04-24-credential-tech-spec.md)). `type Credential = NoCredential;` is the idiomatic opt-out for resources without an authenticated binding (replaces today's `Auth = ()` pattern). The reshape includes a new trait method `on_credential_refresh(&self, new_scheme: &<Self::Credential as Credential>::Scheme) -> impl Future<Output = Result<(), Self::Error>> + Send` with default no-op body. The blue-green pool swap is internalised by the resource (per `nebula-credential` Tech Spec §3.6 lines 961-993, the pattern is `Arc<RwLock<Pool>>` + write-lock swap inside the impl); `Manager` does not orchestrate the swap.

**Rationale.** Adopts the ratified downstream contract verbatim. 100% `()` usage of `type Auth` ([`02-pain-enumeration.md:166`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md), 🔴-5) makes a sub-trait double the API learning surface for no benefit ([`03-scope-decision.md §4.1, line 88`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)). NO `AuthenticatedResource: Resource` sub-trait — explicitly rejected per [`03-scope-decision.md:88-92`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) and §2.4 spike-exit-criteria above.

**Cross-refs.** §1.1 (problem), §2.3 (constraint = §3.6 verbatim), §3.2 (Option B). Phase 6 Tech Spec §3 produces the compile-able Rust shape. Phase 4 spike validates ergonomics + perf per `03-scope-decision.md §5`.

### §4.2 Revocation semantics — extends credential Tech Spec §3.6

**Decision.** Extend §3.6 with a separate `on_credential_revoke(&self, credential_id: &CredentialId) -> impl Future<Output = Result<(), Self::Error>> + Send` method on the `Resource` trait (option (b) from §2.3 candidates, not option (a) dual-semantics-on-refresh). **Default body invariant: post-invocation, the resource emits no further authenticated traffic on the revoked credential.** The mechanism for honoring that invariant (destroy pool instances, mark instances tainted, wait-for-drain, reject new acquires) is a Phase 6 Tech Spec §5 decision; Strategy commits to the *invariant*, not the *implementation*. Revocation **≠ refresh**: there is no new scheme to swap in, so the blue-green swap pattern from §4.1 does not apply. Dispatcher must emit `ResourceEvent::CredentialRevoked { credential_id, resources_affected, outcome }` (new variant; companion to §4.9's `CredentialRefreshed`) and per-resource `HealthChanged { healthy: false }` per security amendment B-2 ([`phase-2-security-lead-review.md:67-74`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)).

**Rationale.** Two semantically distinct events deserve two methods (`feedback_observability_as_completion.md` symmetry: refresh and revoke each ship typed event + invariant). Dual-semantics-on-refresh would force every implementer to branch on "is this a refresh or a revoke?" via the scheme reference, which is awkward and error-prone. Two methods make the operational outcomes explicit.

**Credential-side coordination — closed (no spec dependency).** The credential Tech Spec already provides `Credential::revoke` ([`docs/superpowers/specs/2026-04-24-credential-tech-spec.md:228`](2026-04-24-credential-tech-spec.md): `async fn revoke(ctx, state) -> Result<(), RevokeError>`) and revocation lifecycle modes ([§4.3 lines 1062-1068](2026-04-24-credential-tech-spec.md): soft/hard/cascade revocation, `state_kind = 'revoked'` semantics). The resource-side `on_credential_revoke` hook is a **consumer of those existing primitives, not an extension of credential §3.6**. No credential-side spec extension required; no cross-cascade coordination round needed before Phase 6 dispatch. (Earlier framing as open item §5.5 is downgraded by tech-lead ratification per [`phase-3-cp2-tech-lead-ratification.md` E4](../drafts/2026-04-24-nebula-resource-redesign/phase-3-cp2-tech-lead-ratification.md).)

**Cross-refs.** §2.3 (constraint surface), §4.9 (observability symmetry).

### §4.3 Rotation dispatch mechanics

**Decision.** Parallel dispatch across N resources sharing one credential, via `futures::future::join_all` initially, with `FuturesUnordered` fan-out cap deferred as future optimization (per [`03-scope-decision.md §4.2, line 94-100`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)). Per-resource isolation invariant: one resource's failing `on_credential_refresh` must NOT block sibling dispatches (security amendment B-1, [`phase-2-security-lead-review.md:60-66`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)) **AND each per-resource `on_credential_refresh` future is bounded by its own timeout — NOT a single global dispatch timeout. A global timeout defeats the isolation invariant (one slow resource would cascade-fail siblings).** Phase 6 Tech Spec §5 specifies the timeout configurable surface (per-resource budget; default value; surfacing through `RegisterOptions`). Each per-resource future has its own error path; failures emit `ResourceEvent::CredentialRefreshed { outcome: Failed(...) }` per-resource and tracing span continues for sibling dispatches.

**Rationale.** Parallel because rotation latency = max(per-resource latency), not sum. Per-resource isolation because a single misbehaving resource cannot hold up sibling resources from observing the new scheme — the alternative (serial or all-or-nothing) means one slow PG pool could starve a Kafka producer of a refresh. `FuturesUnordered` cap deferred because the current upper bound on N (resources sharing one credential) is small in practice (5 in-tree consumers × maybe 2-3 resources each = ~10-15); fan-out optimization can land standalone if a future operational signal demands it.

**Hot-path invariant.** Dispatcher holds `&Scheme` (borrowed from credential store), not owned `Scheme`. Each clone is another zeroize obligation per security constraint 7 ([`phase-2-security-lead-review.md:118`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)) and `PRODUCT_CANON.md §12.5`. Per-resource futures borrow the same `&Scheme` for the duration of the dispatch.

**Cross-refs.** §1.1, §2.3 (no clone in dispatcher), §4.9 (observability gate).

### §4.4 Daemon + EventSource extraction target — engine fold

**Decision.** Fold Daemon and EventSource into the engine layer (option (a) from [`03-scope-decision.md §4.6`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)), not a sibling crate. The engine already owns `TriggerAction` substrate for event-driven ingress ([`docs/INTEGRATION_MODEL.md:99`](../../INTEGRATION_MODEL.md), [`docs/PRODUCT_CANON.md §3.5, line 82`](../../PRODUCT_CANON.md): "`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`"); EventSource maps onto that substrate with an EventSource→Trigger adapter on the engine side. Daemon (long-running worker) lands as a new engine primitive — name TBD by Phase 6 Tech Spec / engine team coordination, but conceptually a `DaemonRegistry` parallel to the existing action dispatch. The `nebula-resource` crate retains zero references to `DaemonRuntime` / `EventSourceRuntime` post-extraction; canon §3.5 ("Resource = pool/SDK client") is honored.

**Rationale.** Three reasons engine-fold beats sibling crate. (1) **No precedent for the sibling**: workspace `Cargo.toml` lists 23 top-level non-macro crates today (verified against `[workspace.members]`); none is scheduler-shaped, worker-shaped, or background-shaped. Creating `nebula-scheduler` / `nebula-worker` / `nebula-background` introduces a new crate-level boundary with zero existing adopters — `feedback_boundary_erosion.md` cuts both ways and "extract to a new crate without precedent" is a worse boundary than "fold into the layer that already orchestrates the action lifecycle." (2) **TriggerAction precedent**: engine already dispatches event-driven trigger lifecycles per canon §3.5; EventSource is conceptually a thin extension. (3) **`feedback_active_dev_mode.md`**: bundling extraction with the credential reshape (one PR wave) means migrating 5 in-tree consumers' Daemon/EventSource references in the same atomic change — splitting it across two crates' migrations doubles consumer churn for no upside.

**Trade-off accepted.** Engine surface grows. The alternative — sibling crate — keeps engine narrow but creates a new crate to maintain. Given workspace evidence (no existing scheduler-shaped crate, TriggerAction already covers half the use cases, 5 in-tree consumers as the entire user base), engine-fold is the smaller-surface answer for now. If post-redesign evidence (e.g., Daemon-specific lifecycle proves heavyweight in engine, or non-trigger long-running workers proliferate) supports it, a future cascade can spin out `nebula-scheduler` from the engine side without re-routing through `nebula-resource` again. **Open item §5.1** records the conditions under which this revisits.

**Cross-refs.** §1.2 (problem), §2.2 (canon §3.5), §3.2 (Option B). Phase 6 Tech Spec §13 enumerates the engine-side migration; Phase 5 ADR records the choice.

### §4.5 Manager file-split (keep type, split file)

**Decision.** Keep `Manager` as a single coordinator type per [`03-scope-decision.md §4 row 🟠-7`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) and tech-lead Phase 1+2 position ([`02-pain-enumeration.md:76`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md): "every field is referenced by multiple public methods"). Split the 2101-line `manager.rs` file into submodules. Proposed cuts (Phase 6 Tech Spec §5 finalizes):

- `manager/mod.rs` — `Manager` struct + core `register` / `acquire` / `shutdown` public surface (~500-800 lines target).
- `manager/options.rs` — `ManagerConfig`, `RegisterOptions`, `ShutdownConfig`, `DrainTimeoutPolicy`, `ShutdownError`, `ShutdownReport`.
- `manager/gate.rs` — `GateAdmission` enum + `admit_through_gate` / `settle_gate_admission` helpers.
- `manager/execute.rs` — `execute_with_resilience` + `validate_pool_config` + `wait_for_drain` helpers.
- `manager/rotation.rs` — `on_credential_refresh` / `on_credential_revoke` dispatchers + observability scaffolding (trace span, counter emission, event broadcast per §4.9). Phase 6 Tech Spec §5 finalizes naming and scope; the dispatcher is a real internal seam (currently inline at [`crates/resource/src/manager.rs:1360-1401`](../../../crates/resource/src/manager.rs)) distinct from `register` / `acquire` (in `mod.rs`) and `execute_with_resilience` (in `execute.rs`).

**Rationale.** No public API change — this is purely structural. The 2101-line file is a bandwidth tax on review and orientation per §1.4; submodule cuts trace the existing internal seams (config, gate, execute) without changing what callers see. Type stays single because the internal state is genuinely shared (`feedback_boundary_erosion.md`: split-by-state-shape, not split-by-line-count).

**Cross-refs.** §1.4 (problem), §2.2 (canon §12.7 split-before-grow). Phase 6 Tech Spec §5 finalizes the cut points.

### §4.6 Drain-abort fix — absorbed into Manager file-split PR

**Decision.** Wire `ManagedResource::set_failed()` ([`crates/resource/src/runtime/managed.rs:93-102`](../../../crates/resource/src/runtime/managed.rs)) into the `graceful_shutdown::Abort` path ([`crates/resource/src/manager.rs:1493-1510`](../../../crates/resource/src/manager.rs)); remove `#[expect(dead_code)]`. Phase becomes `ResourcePhase::Failed` with `last_error: Some(ShutdownError::DrainTimeout { … })` rather than `ResourcePhase::Ready` (current corruption). Lands atomically with the Manager file-split PR per [`03-scope-decision.md §3, line 80`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) (SF-2 absorbed, not standalone).

**Rationale.** The fix touches `manager.rs:1507` (the `set_phase_all(Ready)` corruption) and `runtime/managed.rs:93` (the dead-coded helper) — both are file-split-PR territory. Bundling avoids a context-thrash review ("split the file, then later wire the helper") and closes the §12.6 observability-honesty gap from §1.5 in the same atomic change.

**Cross-refs.** §1.5 (problem), §2.2 (canon §12.6 honesty). Phase 6 Tech Spec §6 records the assertion test.

### §4.7 Documentation rewrite (Phase 6 deliverable)

**Decision.** All `crates/resource/docs/` files rewritten atomically against the new trait shape; lands in the same PR wave as the trait reshape and consumer migration. Specific deliverables:

- `crates/resource/docs/api-reference.md` — reconstructed from source ([`crates/resource/src/lib.rs`](../../../crates/resource/src/lib.rs) public re-export tree + `manager/mod.rs` post-split). Removes ~50% fabrication rate per §1.3.
- `crates/resource/docs/adapters.md` — full rewrite. Replaces the 4-of-7 compile-fail walkthroughs with `type Credential = NoCredential;` + `type Credential = SomeCredential;` examples that compile against trunk. Removes references to `nebula-resource-postgres` / `nebula-resource-redis` (adapters that don't exist).
- `crates/resource/docs/dx-eval-real-world.rs` — gated behind `cargo check` in CI per `feedback_no_shims.md` (a doc that doesn't compile is a shim for documentation). Phase 6 Tech Spec §13 specifies the harness.
- `crates/resource/docs/Architecture.md` — rewrite to describe v2 shape OR delete; the redundancy with `README.md` per §1.3 may be resolved by collapsing into one canonical file.
- `crates/resource/docs/README.md` — fix case-drift links surfaced in §1.3.
- `crates/resource/docs/events.md` — rebuild against actual `events.rs` variant list (currently 7 documented vs 10 in source).

**Rationale.** Fixing docs before trait shape locks would require writing them twice per `feedback_incomplete_work.md` and dx-tester Phase 2 input ([`02-pain-enumeration.md:239-242`](../drafts/2026-04-24-nebula-resource-redesign/02-pain-enumeration.md)). Atomic landing means a newcomer hitting trunk after the redesign sees consistent docs, not a half-migrated state.

**Cross-refs.** §1.3 (problem), §2.4 (`feedback_incomplete_work.md` cited indirectly). Phase 6 Tech Spec §13 enumerates per-file change list.

### §4.8 Migration wave — atomic 5-consumer PR

**Decision.** Single PR wave migrates all 5 in-tree consumers atomically per [`03-scope-decision.md §4.7`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) and `feedback_no_shims.md` + `feedback_hard_breaking_changes.md`. Consumers: `nebula-action`, `nebula-sdk`, `nebula-engine`, `nebula-plugin`, `nebula-sandbox`. No deprecation window, no parallel-shape trait, no feature flag (workspace has zero external adopters per §2.5 and `01-current-state.md:87`). `MATURITY.md` row for `nebula-resource` may transition from `frontier` to `core` post-redesign if observability-partial gap closes (Phase 6 ratifies the maturity move; not assumed here).

**Per-consumer change list.** Phase 6 Tech Spec §13 enumerates. Strategy-level invariant: every `Auth = ()` bound becomes `Credential = NoCredential`; every `R::Auth::default()` call site at warmup becomes a real-scheme-or-skip path per §2.4 / §4.1.

**Rationale.** Bundled landing per `feedback_no_shims.md` (no adapter / bridge / shim — replace the wrong thing directly) and `feedback_hard_breaking_changes.md` (expert-level breaking changes acceptable given `frontier` maturity). Atomic close of the credential seam is required by security-lead BLOCK on Option A ([`phase-2-security-lead-review.md:25-46`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)).

**Cross-refs.** §2.5 (release posture), §3.1 (Option A blocked), §4.1 (trait reshape), §4.7 (doc rewrite atomicity).

### §4.9 Observability discipline — DoD, not follow-up

**Decision.** Every rotation-path operation ships with three observability artefacts per tech-lead amendment 2 ([`phase-2-tech-lead-review.md:90-94`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-tech-lead-review.md)) + security amendment B-3 ([`phase-2-security-lead-review.md:76-82`](../drafts/2026-04-24-nebula-resource-redesign/phase-2-security-lead-review.md)) + `feedback_observability_as_completion.md`:

- **Trace span**: `tracing::span!(Level::INFO, "resource.credential_refresh", credential_id = %credential_id, …)` wrapping each per-resource future. Span uses redacted forms per `PRODUCT_CANON.md §12.5` (no scheme content in span fields).
- **Counter metric**: `nebula_resource.credential_rotation_attempts` (or similar; Phase 6 Tech Spec §6 finalizes name) incremented per dispatch with `outcome` label (`success` / `failed`). Companion metric `nebula_resource.credential_rotation_dispatch_latency_seconds` histogram for SLO observability.
- **Event variant**: `ResourceEvent::CredentialRefreshed { credential_id, resources_affected: usize, outcome: RotationOutcome }` (NEW) + `ResourceEvent::CredentialRevoked { credential_id, resources_affected: usize, outcome: RotationOutcome }` (NEW). Both broadcast through the existing `ResourceEvent` channel ([`crates/resource/src/events.rs`](../../../crates/resource/src/events.rs)).

**Phase 6 CP gate.** Tech Spec §6 has an explicit observability gate: no CP advances without verifying trace + counter + event are wired in the same PR as the dispatcher. Per `feedback_observability_as_completion.md`, observability is DoD for a hot path, not a follow-up. **`warmup_pool` must NOT call `Scheme::default()` under the new shape** per §2.4 / security B-3 — Phase 6 §5 specifies the credential-bearing warmup signature.

**Rationale.** §1.6 framing — "advertised but unverifiable" is the same false-capability anti-pattern the redesign is supposed to close. Without instrumentation, operators cannot tell silent-success from silent-drop on a credential compromise; the redesign must not reproduce that gap.

**Cross-refs.** §1.6 (problem), §2.4 (`feedback_observability_as_completion.md`), §4.2 (revoke event symmetry), §4.3 (per-resource isolation surfaced through events).

---

## §5 — Open items

What CP2 cannot fully resolve and Phase 6 Tech Spec or future cascade must answer. Each item names: question, who answers, when, what depends on the answer. Five items below; CP3 §6 turns these into a tracked roadmap.

- **§5.1 — Daemon/EventSource re-evaluation.** §4.4 picked engine-fold. Owner: engine team via Phase 6 §13. *Trigger to revisit sibling-crate*: Daemon-specific engine code grows beyond ~500 LOC OR non-trigger long-running workers proliferate beyond 2. Re-opens via §0 amendment cycle.

- **§5.2 — `AcquireOptions::intent/.tags` interim treatment (#391).** Out-of-scope per §3.3. Phase 6 §5 picks: (a) `#[doc(hidden)]`, (b) `#[deprecated(note = "#391 not wired")]`. Per `PRODUCT_CANON.md §4.5` + `feedback_incomplete_work.md`, (b) is more honest. Owner: tech-lead in Phase 6 §5.

- **§5.3 — `Runtime`/`Lease` collapse.** Phase 1 evidence (9/9 tests `Runtime == Lease`) supports collapse; deferred per §3.3. *Trigger*: any consumer sets `Runtime != Lease` during spike or Tech Spec drafting. Owner: future cascade orchestrator; ADR candidate.

- **§5.4 — Convenience method symmetry under `NoCredential`.** Today's `register_*` + `_with` variants bound `Auth = ()`. Phase 6 §5 question: keep `Credential = NoCredential` shortcut, or require explicit `register_pooled::<R>(…)` with credential bound? Owner: rust-senior + dx-tester via Tech Spec CP2a.

- **§5.5 — Phase 4 spike trigger confirmed.** CP2 locks: spike runs (trait reshape needs §3.6-shape ergonomic + perf validation per §2.4 exit criteria). Scope + exit already locked in [`03-scope-decision.md:145-156`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md). Status confirmation rather than open question; retained in §5 for visibility of the go-no-go gate. (Credential §3.6 revoke extension — formerly §5.5 — closed in §4.2 footnote per CP2 tech-lead E4: credential Tech Spec already provides `Credential::revoke` and revocation lifecycle.)

---

## §6 — Post-validation roadmap

What happens after the cascade — milestones, implementation wave, soak, MATURITY transition, future cascades flagged, register ownership. Each subsection is dense and linkable, not narrative. Roadmap, not specification — Phase 6 Tech Spec is the implementation contract; this section sequences the journey.

### §6.1 Cascade completion milestones

Phase 4 onward turns the Strategy ratifications into compile-able artefacts and a landed redesign. Sequenced phases:

- **Phase 4 — spike** (conditional). Trigger from §5.5: trait reshape needs §3.6-shape ergonomic + perf validation. Scope + exit per [`03-scope-decision.md:145-156`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md). Fallback per §2.4 spike-exit-criteria: do NOT add sub-trait fallback; if §3.6 ergonomics or perf fail, escalate to Phase 2 round 2 (not mid-flight shape change). Owner: rust-senior on isolated worktree.
- **Phase 5 — ADR authoring**. Primary ADR: `Resource::Credential` adoption + `Auth` retirement (Phase 5 deliverable per [`03-scope-decision.md §6, line 168`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)). Candidate second ADR: Daemon/EventSource extraction target = engine fold (records §4.4 decision + revisit triggers from §5.1). Owner: architect drafts; tech-lead ratifies.
- **Phase 6 — Tech Spec**. 5-checkpoint cadence per [`03-scope-decision.md §6, line 165`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md) (16 sections; CP1 outline → CP5 final). Strategy §4 decisions are the binding contract; Tech Spec produces compile-able trait shapes, module layouts, per-consumer migration steps, observability wiring, test plan. Owner: architect drafts; tech-lead + security-lead + dx-tester + rust-senior + spec-auditor review per CP.
- **Phase 7 — Register**. `docs/tracking/nebula-resource-concerns-register.md` created and seeded with all 28 Phase 1 findings (status: which §4 decision resolves which finding; which §5 open item carries which deferred concern; which `03-scope-decision.md` deferred-with-pointer rows trace forward). Pattern from credential register (`docs/tracking/credential-concerns-register.md`). Owner: architect.
- **Phase 8 — Summary deliverable**. Cascade complete: Strategy + ADRs + Tech Spec + register + landed implementation (PR wave merged). Summary doc records the cascade arc for future reference. Owner: architect with orchestrator review.

### §6.2 Implementation wave

How the redesign actually lands as code. Sequencing:

- **PR plan**. Two-phase landing:
  - **Phase A (precursor)**: SF-1 (`deny.toml` wrappers rule) lands as a standalone devops PR, decoupled from the cascade per [`03-scope-decision.md §3, line 80`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md). Owner: devops.
  - **Phase B (cascade)**: Strategy + ADRs + Tech Spec land as documentation PRs (review-only, no implementation). Phase B-1 PR closes Strategy + ADR set; Phase B-2 PR closes Tech Spec.
  - **Phase C (implementation wave)**: single atomic PR or 2-3 sequenced PRs implementing the redesign per `feedback_no_shims.md` + `feedback_hard_breaking_changes.md`. Decision (one PR vs sequenced) at Tech Spec CP5 review; defaults to one PR unless review surfaces a separable concern. Includes: trait reshape, file-split + drain-abort fix bundled, Daemon/EventSource extraction, observability scaffolding, doc rewrite, all 5 consumer migrations, Phase 7 register seeded.
- **Consumer migration list** (5 in-tree per [`03-scope-decision.md §4.7`](../drafts/2026-04-24-nebula-resource-redesign/03-scope-decision.md)): `nebula-action`, `nebula-sdk`, `nebula-engine`, `nebula-plugin`, `nebula-sandbox`. Per-consumer change list in Phase 6 Tech Spec §13.
- **Validation gate**. Each consumer's tests must pass against the new shape before the implementation PR wave merges. CI: `cargo nextest run -p nebula-{action,sdk,engine,plugin,sandbox} --profile ci` (per [`.github/workflows/test-matrix.yml:160-164`](../../../.github/workflows/test-matrix.yml)) green for all 5 + the resource crate itself. `cargo clippy --workspace -- -D warnings` clean. `cargo +nightly fmt --all -- --check` clean.

### §6.3 Post-merge validation

Soak period: redesign cannot be declared "complete" the moment PR merges. Observability-driven validation in `main`:

- **Soak window**. 1-2 weeks in `main` before declaring cascade complete. Window length is observability-driven, not calendar-driven — extend if signal is missing.
- **Observability checks**. Verify §4.9 instrumentation actually fires in a production-representative environment:
  - Counter `nebula_resource.credential_rotation_attempts` (final name from Phase 6 Tech Spec §6) shows non-zero increments across `outcome` labels (`success` / `failed`).
  - `ResourceEvent::CredentialRefreshed` and `ResourceEvent::CredentialRevoked` event stream broadcasts events with realistic `resources_affected` counts (not just zero — confirms the dispatcher is wired end-to-end).
  - Tracing spans (`resource.credential_refresh`, `resource.credential_revoke`) appear in collected traces with redacted fields per `PRODUCT_CANON.md §12.5` (no scheme content).
  - Companion histogram `nebula_resource.credential_rotation_dispatch_latency_seconds` populates (Phase 6 Tech Spec §6 finalizes name).
- **Follow-up issues**. Open GitHub issues for any Phase 1 🟠 / 🟡 finding that surfaces operationally during soak (e.g., transport-test debt 🟠-13 if a regression appears; warmup default 🟡-17 cleanup if `Scheme::default()` re-emerges in any consumer). Issues filed by orchestrator on cascade completion.

### §6.4 MATURITY.md transition

When the redesign earns the maturity bump:

- **Pre-redesign state**: `nebula-resource = frontier` ([`docs/MATURITY.md:36`](../../MATURITY.md)) — design-stable, interfaces-stable, behavior-stable, observability-partial.
- **Post-redesign target**: `core` (or `stable` per `docs/MATURITY.md` legend — design-stable + interfaces-stable + behavior-stable + observability-stable + tests-stable) if redesign closes the observability-partial gap atomically per §4.9.
- **Transition criteria**:
  - Zero 🔴 findings in new `nebula_resource.credential_rotation_attempts` counter `errors` label over the §6.3 soak window.
  - Phase 7 register shows zero unresolved `concerns: open` rows (every concern either carries a § resolution pointer or has explicit deferred-with-pointer status to a future cascade).
  - Per-consumer tests pass against new shape (validation gate in §6.2 closed).
  - Documentation surface rebuilt per §4.7; dx-tester re-evaluation reports zero compile-fail walkthroughs (current baseline: ~50% per §1.3).
- **Owner**. Architect proposes maturity bump in cascade completion summary (§6.1 Phase 8); tech-lead ratifies in dedicated PR per `docs/MATURITY.md` review cadence.

### §6.5 Future cascades flagged

Concerns deferred to future cascades, with explicit triggers. Six bullets:

- **`Runtime`/`Lease` collapse** (Phase 1 🟠-11; §5.3) — *Trigger*: any consumer sets `Lease != Runtime` in spike, Tech Spec, or post-cascade implementation. ADR candidate at trigger fire.
- **`AcquireOptions::intent/.tags` wiring** (Phase 1 🟠-8; §5.2) — *Trigger*: engine integration ticket #391 closes the loop. Phase 6 §5 sets interim `#[deprecated]` posture; future cascade resolves to either remove (#391 not pursued) or wire (#391 lands).
- **Service/Transport merge** (Phase 1 🟡-22) — *Trigger*: evidence accumulates that "Transport = Service + max_sessions + keepalive" simplification is forced by a real consumer (currently absent). `feedback_boundary_erosion.md` applies — do not merge without strong evidence.
- **Bench harness + CodSpeed shard** — *Trigger*: post-cascade. Add `criterion` benches against the new dispatcher path (rotation latency p50/p95/p99) and wire into existing CodSpeed CI shard. Owner: devops + rust-senior.
- **`AuthScheme: Clone` revisit** (Phase 1 🟡-16) — *Trigger*: credential cascade follow-up surfaces zeroize concern in mTLS / signing-key schemes. Coordinated with credential-side spec author per credential register row `arch-authscheme-clone-zeroize`.
- **Daemon/EventSource sibling extraction** (§5.1) — *Trigger*: Daemon-specific engine code grows past ~500 LOC OR ≥2 non-trigger long-running workers materialize. Spin out `nebula-scheduler` from engine side; does not re-route through `nebula-resource`.

### §6.6 Register ownership

`docs/tracking/nebula-resource-concerns-register.md` follows the credential register pattern ([`docs/tracking/credential-concerns-register.md`](../../tracking/credential-concerns-register.md)). Lifecycle:

- **Created**. Phase 7 (§6.1), seeded with all 28 Phase 1 findings + Phase 2 amendments + CP2/CP3 open items as initial rows.
- **Schema**. Same 6-label classification (`strategy-blocking` / `tech-spec-material` / `sub-spec` / `implementation-phase` / `product-policy` / `process`) and 8-value status enum (`decided` / `locked-post-spike` / `pending-sub-spec` / `in-implementation` / `proposed` / `policy-frozen` / `open` / `out-of-scope`) as credential register. Each row has an ID, category, concern, label, status, resolution pointer.
- **Maintained**. Living document through cascade completion + soak window. Updates on: new concern surfaces in any phase; resolution lands (status transitions to `decided` / `in-implementation` / etc.); sub-spec or follow-up PR lands.
- **Closed**. Transitions to "completion-frozen" status after MATURITY.md transition (§6.4) with a one-time cleanup pass — every row's resolution pointer verified, every `open` row resolved or explicitly accepted-as-deferred. After freeze: read-only archive; new concerns become new register entries in subsequent cascade registers, not retroactive amendments.
- **Owner**. Architect maintains; tech-lead reviews quarterly; spec-auditor verifies on each Tech Spec / ADR / Strategy amendment that the register is updated.

---

### Open items raised this checkpoint (CP1 + CP2 consolidated)

**CP1-raised, status:**

- **§2.3 revocation extension** — RESOLVED in §4.2 (separate `on_credential_revoke` method); credential-side coordination CLOSED via §4.2 footnote (credential Tech Spec already provides `Credential::revoke` line 228 + revocation lifecycle §4.3 lines 1062-1068; no spec extension required, no coordination round needed).
- **§2.4 warmup semantics** — partially resolved in §4.9 (`warmup_pool` must not call `Scheme::default()`); exact signature deferred to Phase 6 Tech Spec §5.
- **§2.1 ADR-0035 amendment** — RESOLVED: per tech-lead Ratification answer to ambiguity #2, no new ADR needed; close as no-action. CP3 §6 records closure.
- **§3.3 AcquireOptions resurrection trigger** — carried to §5.2 with explicit interim treatment options.

**CP2-raised, status:** all five §5 items above (§5.1-§5.5; original §5.5 credential-side coordination closed via §4.2 footnote per CP2 tech-lead E4).

## Changelog

- 2026-04-24 CP1 draft — §0-§3 (architect)
- 2026-04-24 CP1 review — spec-auditor PASS_WITH_MINOR / tech-lead RATIFY_WITH_EDITS
- 2026-04-24 CP2 draft — §4-§5 + CP1 edits applied (architect)
- 2026-04-24 CP2 review — spec-auditor PASS_WITH_MINOR / tech-lead RATIFY_WITH_EDITS
- 2026-04-24 CP3 draft — §6 + CP2 edits applied (architect) — FROZEN

---

**Strategy Document complete.** Subsequent Strategy-level evolution via ADRs only (per §0 freeze policy). Phase 4 (spike) or Phase 5 (ADR) per Phase 2 scope decision is the next step.
