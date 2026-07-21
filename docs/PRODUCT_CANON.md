# Nebula — product canon

**Authority:** This file overrides conflicting specs, plans, and chat assumptions until explicitly revised.
**Audience:** Every implementer (human or agent) before non-trivial changes.

If a task would violate this document, **stop**: either update this canon in a deliberate commit or change the approach.

## Purpose

This canon exists to **keep Nebula honest**: we do not ship a sophisticated framework that promises more than the engine can reliably deliver. When roadmap pressure, architecture taste, or new abstractions compete with product truth, **this document wins**.

Nebula is judged by **what operators and integration authors can trust**, not by how elegant the internal type system looks.

> **Audit alignment:** Binding rules in §11–§13 were re-grounded against the **live workspace** (storage tables, execution control queue, activation validation). If `README.md` or onboarding text contradicts §5 / §11.5 / §12.3, treat that as a **bug** — fix docs in the same PR as code, or update this canon deliberately.

---

## 0.1 Layer legend

Canon rules are tagged by **revision cost**:

- **[L1 Principle]** — strategic product intent. Changing means Nebula is a different product. Requires product-level rethink.
- **[L2 Invariant]** — testable contract with a named code seam. Material semantic change requires an ADR (in the maintainers' private design vault) and an updated seam test in the same PR. Wording polish does not.
- **[L3 Convention]** — default style answer. Changing requires a PR with rationale and, if it touches behavior, a test.
- **[L4 Implementation detail]** — not a canon rule. Lives in the owning crate's README. If you find an L4 rule in this file, open a revision per §0.2 and move it.

## 0.2 When canon is wrong (revision triggers)

Canon rules can be stale, premature, or plain wrong. If any of these apply,
**stop, open an ADR, propose revision, then proceed** — do not blind-follow.

- **Dead reference.** Rule mentions a crate, type, or endpoint that no longer exists or has been renamed.
- **Intimacy violation.** Rule can only be changed by editing canon when a single crate refactors. L4 detail leaked into canon. Fix: move to crate README, revise canon to describe the invariant rather than the mechanism.
- **Capability lag.** Rule freezes an implementation that predates a better architectural move, and the improvement is measurable (perf / safety / DX).
- **False capability.** Rule names a type or variant the engine does not honor end-to-end. Per §4.5 the type must be hidden or the rule must drop.
- **Uncovered case.** New failure mode or integration shape the canon is silent on. Write an ADR before blind-applying the nearest rule.

Canon is an articulation, not a prison. Blind-obeying a wrong rule violates
operational honesty (§4.5) more than explicitly revising it.

---

## 1. One-line definition

**[L1]** **Nebula is a high-throughput workflow orchestration engine with `nebula-sdk` as its sole supported and branded Rust surface** — required internal crates are unsupported technical packages that may be published only as exact-version, lockstep dependencies of `nebula-sdk` — **Rust-native, self-hosted, owned by you.**

---

## 2. Position

**[L1]** Nebula is a Rust-native workflow automation engine: DAG workflows, typed boundaries, durable execution state, explicit runtime orchestration, first-class credentials / resources / actions.

**[L1]** Primary audience: developers writing integrations. Secondary: operators deploying and composing workflows.

**[L1]** Competitive dimension: reliability and clarity of execution as a system, plus DX for integration authors.

**[L1]** **Go-to-market shape:** library-first — `nebula-sdk` is the sole supported and branded Rust surface for workflow authors, integration authors, remote clients, embedders, and testing. Every first-party deployment composition root in this workspace lives under `apps/`; reusable library assemblies such as `nebula-worker` are not composition roots. A downstream host becomes a supported composition root only through the curated `nebula_sdk::embedded::RuntimeBuilder`, which cannot replace or bypass aggregate ownership, admission, or tenant authority. Until that façade ships, embedding is not a supported deployment surface. See ADR-0020 (library-first GTM) and private ADR-0117, *Support one Rust SDK surface with lockstep dependency packages*, for the binding decisions. Design records are maintained in the maintainers' private design vault, not in this public repository.

For peer analysis, this canon weighs our explicit bets against n8n / Temporal / Windmill / Make / Zapier, and what we borrow from each. This canon stays normative.

---

## 3. The problem & core thesis

**Two failures of common engines:**

1. **Integrations are second-class** — node/connector authoring is an afterthought; DX and docs suffer; the long tail is unmaintained community glue.
2. **The happy path is assumed** — real workflows run long, hit flaky APIs, get restarted mid-flight, and need retries and recovery as first principles.

**Thesis (execution):** **Nebula handles concurrent, durable execution reliably so integration developers can focus on integration logic, not on orchestration infrastructure.** Performance and resilience are the runtime’s job; the author describes what the node does and trusts the engine at scale.

**Thesis (product):**

> **Nebula is a serious orchestration core with honest contracts.** Prefer **fewer real guarantees** over many attractive but soft capabilities.

### 3.5 Integration model (one pattern, five concepts)

**[L1]** Nebula’s integration surface is a small set of orthogonal concepts, each with a single clear responsibility, all sharing the same structural contract:

- **Resource** — long-lived managed object (connection pool, SDK client). Engine owns lifecycle.
- **Credential** — who you are and how authentication is maintained. The **`nebula-credential` crate owns the resolver / refresh / lease / rotation-state runtime** (consolidated there by ADR-0092, which superseded the historical ADR-0030 "engine owns orchestration" split); the engine keeps only the credential/resource accessor bridges, and the per-slot rotation **fan-out** to live resources lives in `nebula-resource`. The `Credential` trait seals implementation via crate-level supertrait. Actions receive only projected auth material via `Credential::project()`. `ExternalProvider` abstraction supports Vault / AWS Secrets Manager / GCP Secret Manager / Azure Key Vault delegation. `DYNAMIC` credential kind supports ephemeral per-execution secrets.
- **Action** — what a step does. Dispatch via action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`). Adding a trait requires canon revision (§0.2).
- **Plugin** — distribution and startup-registration unit for statically linked, trusted in-process adapters. Plugin is the unit of registration, not the unit of size — full plugins and micro-plugins use the same contract.
- **Schema** — the cross-cutting typed configuration system (`nebula-schema`: `Field`, `Schema`, `ValidValues`, `ResolvedValues` with proof-token pipeline). Shared across Actions, Credentials, Resources.

**[L1]** Structural contract: every integration concept is `*Metadata + Schema` — UI-facing identity plus typed, validated configuration.

For the full model — structural-contract types, wiring rules, plugin packaging (`Cargo.toml` / `plugin.toml` / `impl Plugin`), plugin signing (status: planned), cross-plugin dependency rules — see `docs/INTEGRATION_MODEL.md`. That document is the authoritative source for integration mechanics; this canon states the invariants.

**[L1]** Resource credential dependencies use **typed slot fields** on resources (ADR-0044, consolidated into the M6 contract), not a singular `Resource::Credential` associated type. See `docs/INTEGRATION_MODEL.md` and ADR-0081.

Sections 3.6 through 3.9 (per-crate pointers) are consolidated in `docs/INTEGRATION_MODEL.md`.

### 3.10 Shared vocabulary

Types stabilized in `nebula-core` that cross crate boundaries:

- `AuthScheme`, `AuthPattern` — authentication classification (moved from credential to core).
- `Guard`, `TypedGuard` — RAII lifecycle traits for credential and resource guards.
- `BaseContext`, `Context` trait family — shared capability accessors and lifecycle signals.

Credential-domain vocabulary added in the restructuring:

- `ExternalProvider`, `ProviderKind` — external secret-manager delegation abstraction.
- `CredentialMetrics` — metric name / label constants for credential operations.

---

## 4. Pillars

Directional goals; binding engineering rules live in §12–§14. The **integration model** (§3.5) explains *what* authors ship; full per-crate and cross-cutting details are in `docs/INTEGRATION_MODEL.md`. The pillars below explain *runtime* and *operations* priorities.

### 4.1 Throughput

**[L1]** Async-native execution (Tokio): many concurrent workflow executions should share a small thread pool without one slow I/O blocking others. Memory per execution should stay in the **hundreds-of-KB** order for common paths (not tens of MB per execution by default shape). **Throughput and latency regressions in benchmarked paths are treated as bugs** where benchmarks exist (e.g. CodSpeed in CI).

### 4.2 Safety

**[L1]** **Fail fast and loudly on misuse:** typed errors, validated node contracts where declared, no silent shape mismatches in production. **Credentials** stay behind existing abstractions (no leakage across boundaries; rotation is not the node author's ad-hoc problem). Credential material is guarded by `CredentialGuard` (RAII pattern implementing `nebula-core::Guard`; zeroize-on-drop). Projected material is validated at the credential→action boundary. **Unsafe** stays in engine/runtime layers — integration-facing APIs remain safe Rust. Resilience classifiers (`nebula-resilience` / `ErrorClassifier` pattern) make transient vs permanent failure an explicit decision, not folklore.

### 4.3 Keep-alive

**[L1]** **Duration:** runs that last **minutes through days** (and longer when storage and checkpoints keep up) are a **normal** design target — not only sub-second HTTP hops.

**[L1]** **Process death:** if the **host process** dies mid-run (deploy, OOM, crash), truth is **only what is persisted** (§11). Work **after the last durable checkpoint / journal line** may be **re-executed**, **rolled back**, or **lost** to the extent those paths are best-effort — the operator must see **status, errors, and journal** that say so, not green-washed success. Cancellation, leases, and the control queue (§12.2) exist so “long-lived” does not mean “hope the process lives forever.”

**[L1]** Integration authors assume unreliable networks; the runtime assumes **restartable processes** and makes resume and cancel **inspectable** (§4.6).

### 4.4 DX

**[L1]** **The SDK experience is a product surface:** one coherent `nebula-sdk` entry point serves workflow and integration authoring, testing, remote-client, and embedded personas. Integration contributors get fast scaffolding, test harnesses, actionable errors, and reference integration tests. Trait-driven contracts should make missing implementation pieces a **compile-time** story where possible; activation validation proves dynamic graph references, schemas, compatibility, and capabilities that compilation cannot know.

### 4.5 Operational honesty — no false capabilities

**[L1]** **Public surface exists iff the engine honors it end-to-end.** A type, variant, or endpoint that can be called but the engine rejects at runtime is a **false capability** — per canon, such types must not ship publicly. Options:

1. **Implement end-to-end** — wire the behavior through its owning runtime capability, persistence, resilience policy, and observability. Execution transitions use `nebula_storage_port::ExecutionStore::commit(TransitionBatch)` rather than an invented repository seam.
2. **Make the surface private or feature-gated** — `pub(crate)` or gated under `unstable-*` feature so consumers cannot bind to what the engine does not yet deliver.
3. **Remove the surface entirely.**

**[L1]** Corollaries:

- **Misconfiguration moves left.** Validation / activation-time checks over runtime rejection, wherever feasible for workflow shape.
- **JSON at edges is fine; JSON instead of validated boundaries is not.** Schemas and compatibility rules at workflow / action boundaries win over unstructured blobs.
- **In-process channels decouple components but are not a durable backbone.** Anything requiring reliable delivery — including cancel, dispatch, and business facts — must share the persistence transaction with the owning state transition, or live in an explicit durable outbox/inbox with documented delivery semantics (see §12.2). `nebula-eventbus` is limited to ephemeral observations such as telemetry, cache/UI invalidation, and wake hints; its consumers must tolerate loss, duplication, and reordering and recover from durable truth. A channel whose consumer logs and discards is not a contract.

The Rust patterns that make this invariant easy to uphold: sealed traits, typestate, `#[non_exhaustive]`, `#[unstable]` feature gates.

### 4.6 Observability

**[L1]** Durable is not enough — runs must be explainable. Execution state, append-only journal, structured errors, and metrics let an operator answer what happened and why a run failed without reading Rust source.

**[L1]** Observability is a first-class contract, not polish. SLIs, SLOs, structured event schema for `execution_journal`, and the core analysis loop live in `docs/OBSERVABILITY.md`.

**[L2]** Where a feature is still thin (e.g. lease enforcement at §11.6), say so — do not imply full auditability from partial signals.

---

## 5. Scope table


| Nebula **is**                                                                                                       | Nebula **is not** (until canon changes)                               |
| ------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| Engine + storage + API + runtime as **composable crates** with one-way layers                                       | A single “god binary” that hides all structure                        |
| **Local-first:** core flows runnable with **SQLite** (file or `sqlite::memory:`), no mandatory Docker/Redis for dev | Default path that requires Redis/Kafka “just to start”                |
| **Rust-native** integration model with trusted plugins statically linked into the worker / host                   | A low-code platform where the primary author is non-developer glue    |
| Honest docs: in-process execution = **capability / correctness aid**, not attacker-grade isolation | Claims of untrusted-code isolation the engine does not provide — process isolation, OS-hardened child process, microVM, and WASM are all non-goals (§12.6) |
| **Self-hosted identity** first; cloud-style deployment is “Nebula on infra,” not a different product                | Hosted-service-first product with different core guarantees           |
| Breaking **wrong** internal APIs when the cost of shims exceeds clarity                                             | Compatibility shims that preserve bad shapes “for now”                |


> **Local storage truth:** the supported local deployment path is **SQLite**, usable against a file or `sqlite::memory:`. `nebula-storage` also contains an InMemory implementation, but it is an internal test/reference/conformance adapter and is not a supported deployment backend. Product onboarding must not present it as an operator choice equivalent to SQLite or Postgres.

> **Intended deployment path:** Nebula is not production-ready yet. Once the release and conformance gates pass, **SQLite local/edge** and **Postgres self-hosted** are the only intended supported deployment paths. Additional storage backends or cloud multi-tenant modes are additive and must be explicitly marked **experimental** or **planned** until this canon says otherwise. Alternative plugin execution models are governed by §12.6 and are not additive roadmap items.

---

## 6. Architecture ↔ pillars

Major choices should map to a pillar; if a feature maps to none, it is probably out of scope.


| Decision / artifact                               | Pillar                       |
| ------------------------------------------------- | ---------------------------- |
| Tokio async-native engine                         | Throughput                   |
| `nebula-resilience` composable pipelines          | Keep-alive + Safety          |
| Checkpoint-based recovery vs ad-hoc restart       | Keep-alive                   |
| Credential / resource boundaries                  | Safety                       |
| Trait-based node contracts, typed errors          | Safety + DX                  |
| Testing harnesses for integrations                | DX                           |
| `ExecutionControlQueue` / durable outbox          | Keep-alive + honesty (§12.2) |
| Execution journal, metrics, structured API errors | Observability (§4.6)         |

### 6.1 Platform planes

**[L1]** Private ADR-0116, *Adopt platform planes and profiled execution*, defines the binding architecture direction. The planes have distinct ownership and may depend only toward contracts and capabilities, never back toward a product surface:

1. **Versioned contract plane** — immutable workflow, integration, worker-flavor, execution-profile, and capability revisions. It emits an authority-free `ExecutablePlanRevision`; it does not bind tenant authority or admit execution. “Use whatever is latest on resume” is not a compatibility policy.
2. **Transition kernel** — deterministic, I/O-free state-transition logic. It receives validated state plus an explicit command/event and returns decisions/effects; it does not read storage, call integrations, publish events, or choose deployment policy.
3. **Runtime control plane** — combines an `ExecutablePlanRevision` with opaque authenticated scope and an immutable registry snapshot, then admits and persists the tenant-bound `ExecutionContractBundle`. It alone owns durable execution mutation, the execution journal and queues, execution outbox/inbox, the operation ledger, fencing, effect application, recovery, and bounded runtime turns. Storage adapters implement its ports; API handlers and integrations do not bypass it.
4. **Trusted integration adapters** — statically linked action, resource-provider, credential-provider, and trigger implementations invoked through versioned SDK contracts. Author code owns external-system semantics; framework credential and resource runtimes retain their aggregate write authority and durable lifecycle.
5. **Product surfaces** — `nebula-sdk`, HTTP API, CLI, and any future Studio compose, validate, submit, and observe work through the same contracts. They do not define parallel execution semantics or become sources of durable truth.

**[L1]** Durable authority is aggregate-scoped. Credential runtime is the sole writer of credential/refresh/lease state. Resource lifecycle is the sole writer of resource/binding/fan-out state. Cross-aggregate commands and facts move through persisted state or explicit outbox/inbox ports; `nebula-eventbus` is only an observation or wake hint.

**[L1]** Every first-party deployment composition root in this workspace lives under `apps/`. Library crates may provide reusable assemblies, but a reusable assembly is not a composition root. A downstream embedded host becomes supported only through `nebula_sdk::embedded::RuntimeBuilder` and cannot replace the owning aggregate runtime or manufacture admission/tenant authority. Until that façade ships, downstream embedding is planned rather than supported.

### 6.2 Profiled execution

**[L1]** Universal platform guarantees do not imply one universal execution loop. Identity, tenancy, version pinning, durable acceptance, fencing, persistence, errors, and observability are shared; scheduling and turn semantics belong to bounded execution profiles.

- **Graph** is Nebula's flagship and current generally described profile: an activation-validated dynamic DAG executed against typed integration contracts.
- **Interactive**, **Agent**, and **Stream** are future profiles. Each requires its own bounded runtime semantics, persisted state model, admission policy, recovery tests, and capability negotiation before it may appear in the stable SDK or API.
- A capability-gated profile must be rejected at activation when the selected worker/runtime cannot honor it. Hidden, unstable, or planned profiles are not silently downgraded to Graph behavior.
- MCP or another protocol bridge may be an edge adapter to a supported profile; it is never the durability, identity, or execution authority.

This section records architecture, not a claim that every named profile is implemented. Public status remains governed by §4.5 and §11.6.


---

## 7. Open source contract

- **[L1]** **One supported Rust surface:** `nebula-sdk` is the sole supported and branded Rust API for every supported persona. Stability applies to its documented exports and contracts; breaking changes deserve an RFC-style decision, not drive-by commits.
- **[L1]** **Persona-scoped, not crate-scoped:** the SDK surface is organized around workflow/authoring, integration, schema, testing, client, embedded, and a small prelude. Availability is feature- and maturity-documented; naming a persona here does not claim every module is implemented today.
- **[L1]** **Safe client and embedded façades:** the curated client consumes the versioned transport contract, and the curated embedded surface submits typed runtime commands. Neither exposes raw stores, transition/journal writers, registries, durable mutation or admission capabilities, claim tokens, or tenant-proof constructors.
- **[L2]** **Technical boundaries stay technical:** the API-contract boundary and internal client, embedded, macro, and implementation packages may be published lockstep when Cargo requires it, but they are not separately supported Rust products.
- **[L2]** **Workspace internals:** may break when wrong — but **not** silently: canon + migration note + tests (see §17).
- **[L2]** **Publication mechanics are not API promotion:** internal packages required by `nebula-sdk` may be published to crates.io as doc-hidden technical dependencies, with exact-version pins and lockstep releases. Direct use of those packages is unsupported and receives no independent compatibility promise; consumers must depend on `nebula-sdk`. Internal packages outside the SDK dependency closure remain `publish = false` by default. This supersedes the physical “only one published package” interpretation of ADR-0021; private ADR-0117 is authoritative.
- **[L1]** **Ecosystem quality over node count:** one solid canonical integration per external service beats many half-finished duplicates.
- **[L1]** **Third-party nodes** are first-class in intent: same capabilities as first-party where the plugin model allows; **document** what is shipped vs planned.

### 7.1 Plugin packaging

**[L1]** Plugin is the unit of **registration**, not the unit of size. Full plugins and micro-plugins use the same contract: Rust crate + `plugin.toml` marker + `impl Plugin`.

**[L2]** Three sources of truth, no duplication:


- **`Cargo.toml`** — Rust package identity and dependency graph (including cross-plugin `[dependencies]`).
- **`plugin.toml`** — trust + compatibility boundary (SDK constraint, optional stable plugin id, signing when enabled). Read without compiling.
- **`impl Plugin` + `PluginManifest`** — runtime source of truth for registered actions / resources / credentials / locales (bundle descriptor, not a schematized leaf — ADR-0018).

**[L2]** Cross-plugin types come in via `Cargo.toml` `[dependencies]` on the provider plugin crate. Engine loads providers before dependents (acyclic graph). Referencing a type outside the declared dependency closure is a misconfiguration caught at activation.

**[L4]** Full packaging mechanics — `[nebula]` / `[plugin]` / `[signing]` table shapes, signing rationale, layout examples, and native build / linkage notes — live in `docs/INTEGRATION_MODEL.md`.

### 7.2 Engine upgrade and workflow compatibility

**Operators** need a clear story when the **engine** version changes — not only plugin authors.

- **[L2]** **Persisted workflow definitions** and **plugins** (binaries / SDK linkage) are **two compatibility surfaces**; breaking either belongs in **release notes** and migration guidance.
- **[L2]** **Patch and minor** releases **must** keep **forward-compatible** workflow JSON and documented **plugin SDK** boundaries unless the release **explicitly** announces a break.
- **[L3]** **Plugin build compatibility:** Rust plugin crates are trusted code statically linked into a worker / host. Plugin changes and SDK / engine upgrades require recompiling and redeploying that worker / host; there is no independent binary plugin compatibility surface.
- **[L2]** **Breaking** workflow schema, execution semantics, or public SDK types require **documented migration**, tests, and upgrade notes — not an assumption that existing installs “should work.”
- **[L1]** Do **not** claim “all v1 workflows run unchanged on v2” without a **published compatibility matrix** or equivalent — platform trust requires **honest** upgrade paths.

---

## 8. What Nebula is not

- **[L1]** **Not a low-code tool** — operators may compose graphs; **authors** target Rust through `nebula-sdk` and native statically linked plugins, not replacement of typed integration work.
- **[L1]** **Not optimized for one-shot 50 ms scripts** — value shows up at **scale, duration, and integration depth**.
- **[L1]** **Not “most nodes wins”** — the metric is **SDK quality and reliability**, not inventory size.
- **[L1]** **Not a generic framework playground** or **trait zoo** optimized for elegance over usability and engine truth.
- **[L1]** **Not “JSON everywhere and hope for the best”** — interchange types are deliberate; product-level truth needs validation and boundaries.
- **[L1]** **Not advertising** retry, durability, resource lifecycle, or plugin trust models **before the engine actually owns them** end-to-end.

---

## 9. North star & success

**[L1]** **North star — integration author:** A Rust developer with no prior Nebula experience can open the integration SDK / traits (§3.5 + `docs/INTEGRATION_MODEL.md`), and ship a **working, tested** node for a new service in **a focused day** — without hand-rolling orchestration, credential plumbing, or concurrency bugs.

**[L1]** **North star — operator:** After **any** failed or stuck run, an operator can **explain what happened** — which step, what error, what durable state — using **logs, API, journal, and metrics alone**, without reading integration **source code** (aligned with §2 and §4.6).

**[L1]** **North star — trigger delivery:** Trigger-driven flows have an **explicit, testable delivery contract**: no silent event drop; delivery semantics are documented (typically **at-least-once**), and duplicate events are controlled via event identity + idempotency/dedup rules rather than wishful “exactly once” claims (see §11.3).

**Success sounds like (author):** *“Writing a Nebula node was the easiest integration I’ve ever written; it kept working under load and failure.”*

**Success sounds like (operator):** *“When something broke, I knew **where** it failed and **why** — not only that the run turned red.”*

**Success sounds like (trigger ops):** *“Incoming trigger events were either processed once or safely de-duplicated — never silently lost.”*

**[L1]** **Progress looks like:** engine behavior and **public contracts align**; vertical slices are **boringly reliable**; workflow validity **shifts left** into validation; docs get **shorter and truer**, not larger and wishful.

---

## 10. Golden path (product)

Nebula must **protect one coherent path** before multiplying half-supported options. In intent:

1. **[L1]** Author defines a workflow; definition is persisted and **validatable** (round-trip).
2. **[L2]** **Activate** the workflow where the product supports activation. Activation runs `nebula_workflow::validate_workflow` (or equivalent) and **rejects** invalid definitions with structured **RFC 9457** errors — it does not silently flip a flag. A standalone `/validate` endpoint is a **tool**, not a substitute: activation that enables a workflow **without** validation is a **§10 violation**.
3. **[L1]** Trigger or API starts execution.
4. **[L2]** Engine schedules **executable step semantics** only — triggers, resources, and steps remain **distinct concepts** in validation, not only in dispatch errors.
5. **[L2]** Execution state transitions are **visible and attributable** through runtime control; its current atomic storage seam is `nebula_storage_port::ExecutionStore::commit(TransitionBatch)`, guarded by version CAS and lease fencing. No handler invents an out-of-band lifecycle.
6. **[L2]** Failure, cancellation, retry, and timeout behavior match **documented** contracts — not folklore in traits. **Cancel** requests must be **durable and engine-consumable** (see §12.2), not “only the DB row changed.”
7. **[L1]** **Persistence story is explicit:** what is durable vs best-effort; what resume/replay may assume; what happens on checkpoint failure.
8. **[L1]** Operator can **inspect** what happened and what is trustworthy.

Anything that does not strengthen this path is secondary until the canon says otherwise.

---

## 11. Core product contracts (honesty)

These must stay **explicit in code and operator-facing docs**, not split across half-implemented types.

### 11.1 Execution authority

**[L2]** `nebula-execution` defines execution-state semantics and the persisted execution record is the **single source of truth**. Runtime control is the execution-aggregate writer: it owns execution state, journal, execution queues, execution outbox/inbox, and the operation ledger. Its current physical transition port is `nebula_storage_port::ExecutionStore::commit(TransitionBatch)`, which uses optimistic CAS against persisted `version` plus a lease fencing token. There is no ephemeral “usually DB wins” mode: if persistence is unavailable, the operation **fails** — it does not silently mutate in-memory state. Direct calls from product-surface handlers are a migration gap toward the runtime command boundary, not a pattern to copy or extend.

Current seam: `crates/storage-port/src/store/execution.rs` (`ExecutionStore`) and `crates/storage-port/src/batch.rs` (`TransitionBatch`). Backend implementations live in `crates/storage/src/inmem/execution.rs`, `crates/storage/src/sqlite/execution.rs`, and `crates/storage/src/postgres/execution.rs`; InMemory is test/reference-only. The future `ExecutionPersistence` / `BoundExecution` capability described in private design records is not an implemented replacement yet. Test coverage: see `docs/MATURITY.md`.

### 11.2 Retry

**[L2]** Retry is a **runtime semantic** owned by two disjoint surfaces: the **engine** for operator-declared node retry and **`nebula-resilience`** pipelines around **outbound** calls inside an action. It is not a decorative hint on an `ActionResult` return type. The engine schedules re-execution only from declared `NodeDefinition.retry_policy` / `WorkflowConfig.retry_policy` after a retryable failure, with persisted node state (`WaitingRetry`), `next_attempt_at`, per-attempt idempotency keys, and `ExecutionBudget.max_total_retries`. The engine **does not** schedule re-execution from an `ActionResult::Retry`-style return; no such public `ActionResult` variant or `unstable-retry-scheduler` feature exists in the current action/engine crates.

**Status (per §11.6 vocabulary):**

| Surface | Status | Notes |
| --- | --- | --- |
| `nebula-resilience` pipeline inside an action (in-memory retry around outbound calls) | `implemented` | The **canonical** retry surface today. Author composes retry/timeout/circuit-breaker at the call site. |
| Operator-declared engine-level node retry (`retry_policy`) | `implemented` | After a retryable `Running → Failed` path, the engine computes the effective node/workflow retry policy, parks the node in `WaitingRetry` with `next_attempt_at`, increments execution-level retry accounting, and re-dispatches when the timer fires. `ExecutionBudget.max_total_retries` is the global cap; cancel, explicit terminate, and wall-clock teardown drain parked retries without re-dispatch. |
| Engine-level node re-execution from `ActionResult::Retry` | `not-present` | `ActionResult` intentionally has no `Retry` variant in the current public surface, and there is no `unstable-retry-scheduler` feature in `nebula-action` or `nebula-engine`. Future result-driven retry would be a new capability and must not be documented as implemented unless it is wired through the same persisted state/idempotency guarantees as operator-declared retry. |
| Cross-restart retry of a checkpointed step | `best-effort` | Relies on checkpoint boundaries (§11.5); work since the last checkpoint may be replayed or lost. Not a per-attempt contract. |

**[L2]** Invariant: docs and public APIs must name the retry trigger. **Operator-declared retry** is implemented engine behavior. **Result-driven retry** is not a current `ActionResult` capability. Any future `ActionResult::Retry`-style surface must ship with persisted attempt accounting, idempotency, restart recovery, and engine tests in the same change; otherwise it is a false capability under §4.5.

### 11.3 Idempotency

**[L2]** **One** idempotency story: deterministic per-attempt identity, checked and durably marked through `nebula_storage_port::store::IdempotencyGuard::check_and_mark` before the side effect. **Engine guarantee:** it will not double-dispatch a **marked** attempt. Whether the **external** system de-duplicates is the integration author’s contract with that system — document per node. Contract seam: `crates/storage-port/src/store/idempotency.rs`; exact key semantics are documented there and in `crates/execution/README.md`.

**[L2]** For **non-idempotent or risky side effects** (payments, writes without natural upsert, external one-shot operations), action handlers must guard execution with this idempotency path (or an equivalent documented key contract) before calling the remote system.

**[L2]** For **TriggerAction** sources, each inbound event should carry or derive a stable event identity (provider event id / cursor offset / hash) so at-least-once delivery can be made safe via dedup/idempotent handling; “no duplicates” is not a claim unless the source + runtime can prove it end-to-end.

### 11.4 Resource lifecycle

**[L2]** Resources are first-class because **acquisition** and **scope-bounded release** are **engine-owned**. Teardown has **two contracts** (ADR-0093). **Normal release** (`recycle` / `destroy`) is **awaited, deadline-bounded, and fallible**: the framework composes a per-resource teardown deadline (`Provider::teardown_budget`, capped short on credential revoke), abandons a hook that exceeds it with a typed error, and **discards rather than re-pools** a credentialed instance unless the author’s `recycle` wipes per-lease session state. **Crash release** stays **best-effort** — if the host process dies, orphaned resources rely on the next process to drain (mechanism types: see `crates/resource/README.md`). Operators must be told this; authors must not assume “release ran” without an explicit checkpoint.

Seam: `crates/resource/src/release_queue.rs` — `ReleaseQueue`. Test coverage: see `docs/MATURITY.md`.

**[L1]** For long-lived exclusive/external resources (locks, leased cloud instances), deployments need external TTL / dead-man strategy; Nebula v1 does not provide an external lease arbiter by itself.

### 11.5 Persistence & operators

**[L2]** Checkpointing is **policy-driven**, not “fsync every step.” The engine checkpoints at declared boundaries (workflow/action policy) and on workflow completion. Between checkpoints, progress is in-memory: process death can lose work since the last checkpoint.

**[L2]** Authors should place checkpoint boundaries before irreversible or expensive side effects; the engine does not guess those boundaries for you.


| Artifact                           | Status                                                               | Operator-visible truth                                                                                                                                                           |
| ---------------------------------- | -------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `executions` row + state JSON      | **Durable** (CAS + fencing via `ExecutionStore::commit`)             | Source of truth                                                                                                                                                                  |
| `execution_journal` (append-only)  | **Durable when appended in the same `TransitionBatch`**              | Replayable history coupled to the state it describes                                                                                                                            |
| `execution_control_queue` (outbox) | **Durable when appended in the same `TransitionBatch`**              | At-least-once dispatch + cancel signals (§12.2); a separately enqueued start is a migration gap until reconciled                                                                 |
| stateful-action checkpoint         | **Best-effort optimization; backend support must be stated**         | A missing/failed checkpoint falls back to committed execution state; the current standalone `CheckpointStore` has only an internal InMemory implementation and is not a deployment durability claim |
| execution lease                    | **Port methods exist; enforcement status must be demonstrated**      | Do not imply lease safety unless runtime acquisition, renewal, release, and fencing tests are green                                                                              |
| `nebula-eventbus`, in-process `mpsc` / channels | **Ephemeral**                                             | Observations and wake hints only; never authoritative truth; consumers tolerate loss, duplication, and reordering                                                              |


Seams: execution state, outbox rows, and journal appends enter atomically through `ExecutionStore::commit(TransitionBatch)` in `crates/storage-port/src/store/execution.rs` and `crates/storage-port/src/batch.rs`; journal reads use `crates/storage-port/src/store/journal.rs`. The separate best-effort stateful-action checkpoint contract is `crates/storage-port/src/store/checkpoint.rs`. Backend adapter paths are `crates/storage/src/{inmem,sqlite,postgres}/execution.rs`; do not infer a SQL checkpoint implementation from the execution adapter. Test coverage: see `docs/MATURITY.md`.

**[L2]** If an operator cannot answer durability questions from this section plus code/docstrings, the product is not yet operationally honest.

**[L2]** Checkpoint / side-effect race is a real failure mode: if a side effect commits externally and the checkpoint write fails afterward, replay can re-enter that step. Protection is by design through idempotency keys (§11.3), not by pretending exactly-once.

### 11.6 Documentation truth

**[L2]** Docs must distinguish **implemented**, **best-effort**, **experimental**, and **planned** behavior. A short **guarantees** narrative (in `docs/` or README) should answer durability, validation, retry, resume, and current plugin trust — without collapsing future intent into today’s contract. **README drift** (advertising a removed backend, endpoint, or capability) is a **bug** — fix in the same PR as the code change.


| Status             | Meaning                                                                                |
| ------------------ | -------------------------------------------------------------------------------------- |
| `implemented`      | Works end-to-end, covered by tests, safe to rely on as a current contract.             |
| `best-effort`      | System attempts it, but does not guarantee success under all failures.                 |
| `experimental`     | Shipped but unstable; API/semantics may change; not default production guidance.       |
| `planned`          | Not implemented yet; do not promise to operators as current behavior.                  |
| `demo-only`        | Works in examples/dev flows; explicitly not a product contract.                        |
| `false capability` | Type/endpoint exists but engine does not own behavior end-to-end; remove or implement. |


---

## 12. Non-negotiable invariants

### 12.1 Layering and dependencies

- **[L2]** Follow `AGENTS.md` dependency direction. **No upward dependencies** between layers.
- **[L2]** Direct downward dependencies on domain types and declared ports are normal. Durable commands and business facts use persisted state or explicit outbox/inbox seams. `nebula-eventbus` is observation-only and must never be used to evade the layer map or as a source of truth.
- **[L2]** `crates/api` does not embed SQL drivers or storage schema knowledge beyond declared ports; **storage and orchestration details live in their crates**.
- **[L2]** Every first-party deployment composition root in this workspace lives under `apps/`. Reusable assemblies such as `nebula-worker` do not select deployment policy or own process lifecycle. A downstream embedded host becomes supported only through `nebula_sdk::embedded::RuntimeBuilder` and cannot replace aggregate writers or admission/tenant authority; until that façade ships, embedding is planned.

### 12.2 Execution: single semantic core, durable control plane

- **[L2]** **Authoritative execution semantics** live in the transition kernel and durable state lives behind runtime control's persistence capability. Handlers and API DTOs **do not invent a parallel lifecycle**, bypass runtime mutation authority, or return **synthesized** timestamps or fake defaults for missing fields. Existing direct handler calls to `ExecutionStore::create` or `ExecutionStore::commit` are migration surfaces, not permission to spread persistence ownership into product surfaces.
- **[L2]** **Write authority is aggregate-scoped:** runtime control alone writes the execution aggregate/journal/queues/outbox/inbox/operation ledger; credential runtime alone writes credential/refresh/lease state; resource lifecycle alone writes resource/binding/fan-out state. Cross-aggregate changes use durable commands and persisted outbox/inbox seams. EventBus delivery is never required for correctness.
- **[L2]** **Every “run this” / “cancel this” signal must be durable and engine-consumable.** The contract is:
  1. The signal is written to **`execution_control_queue`** (outbox) **in the same logical operation** as the corresponding state transition. A handler that flips state to `cancelling` **without** enqueueing — or enqueues **without** transitioning — is broken.
  2. A dispatch worker drains the queue and forwards commands to a consumer that **the engine actually listens to**. Removing rows **before** the engine has acted is broken.
  3. There is **one** consumer wiring story per deployment mode, **documented in code**.
- **[L2]** **A demo handler that logs the command and discards it does not satisfy this invariant.** Examples and `simple_server.rs` must either wire a **real** engine consumer or be marked `// DEMO ONLY — does not honor cancel` so nothing mistakes them for the contract.
- **[L2]** **Batching outbox writes for throughput is valid only if per-transition atomicity is preserved.** Never batch as a workaround that breaks “state transition + control signal” integrity.
- **[L2]** Any second **authoritative** control channel is forbidden unless this canon is updated with a reconciliation story. An HTTP ingress or ephemeral EventBus wake hint may notify the owner, but correctness must survive its loss, duplication, and reordering by recovering from persisted state/outbox/inbox truth.

### 12.3 Local path

- **[L3]** The **default developer experience** must allow build, fast unit checks, and core local flows **without** Docker, Redis, or external brokers. **SQLite** is the supported local storage; `sqlite::memory:` is the in-process SQLite path. The distinct InMemory adapter is internal test/reference/conformance infrastructure, not a deployment backend.
- **[L3]** **SQLite is for local and edge deployments.** It has write-lock contention limits under high concurrency. Once production-readiness gates pass, **Postgres** (with `FOR UPDATE SKIP LOCKED`) is the required high-throughput self-hosted path.
- **[L2]** “No external service for hello world” does not make Postgres conformance optional. The required pre-PR/release/CI conformance gate must exercise a real Postgres instance and may not be skipped or substituted with SQLite/InMemory. Fast local checks need not start Postgres; `examples/simple_server.rs` (and similar) must continue to start without external services unless explicitly documented as integration-only.
- **[L3]** Later storage or broker paths are additive and remain experimental/planned until separately gated; they do not weaken SQLite and Postgres conformance.

### 12.4 Errors and contracts

- **[L3]** Library crates: `thiserror`, not `anyhow`, in public library surfaces.
- **[L2]** API boundary: **RFC 9457 `problem+json`** (see `crates/api/src/errors.rs`). **No new ad-hoc `500`** for business-logic mistakes — map new failure modes into typed `ApiError` variants with an explicit status.
- **[L3]** `serde_json::Value` is allowed **where it is the deliberate interchange type**; new **stringly protocols** (magic field names without schema validation) require explicit review.

### 12.5 Secrets and auth

- **[L2]** No secrets in logs, error strings, or metrics labels. **`Zeroize` / `ZeroizeOnDrop`** on key material; redacted `Debug` on credential wrappers (`SecretToken`, etc.). Encryption at rest uses authenticated encryption. Specific algorithm / key-derivation / parameter choices: see `crates/credential/README.md`. Do not bypass encryption "for debugging."
- **[L2]** Every new `tracing::*!` that takes a credential or token argument must use **redacted** forms.
- **[L2]** Credential operations emit metrics through `CredentialMetrics` for dashboarding: resolve, refresh, rotation, dynamic lease, tamper detection.
- **[L2]** `ExternalProvider` boundary: secrets resolved from external managers (Vault / AWS SM / GCP SM / Azure KV) are never persisted locally unless explicitly configured.

### 12.6 Plugin trust model — in-process, no isolation

- **[L1]** **Plugins and actions are statically linked into the host and run in-process as trusted code** (ADR-0091). Startup registration is discovery, not dynamic isolation. There is no process, memory, or capability boundary between a plugin and the engine. `PluginCapabilities` / capability checks are **correctness and least-privilege aids against accidental misuse**, not a security boundary against malicious native code. Keep `lib.rs` / README doc comments and `docs/` threat models aligned with this — never describe in-process dispatch as sandboxed execution of untrusted code.
- **[L1]** **Remote plugin execution, dynamically loaded / FFI plugin ABIs, process isolation, out-of-process plugin execution, and WASM / WASI are abandoned non-goals** (ADR-0091) — not a roadmap, not a deferred phase, not a guarantee any author may assume. The native crates integration authors actually need (`sqlx`, `rdkafka`, `tonic` with native TLS, any `*-sys` crate) do not fit a WASM target, and an out-of-process / child-process "sandbox" narrative would be a §4.5 false capability. Do not reintroduce isolation language the engine does not provide. Reconsidering any of these non-goals requires an explicit revision to this canon followed by an accepted ADR and threat model; an ADR alone is insufficient.

### 12.7 No god files, no orphan modules

- **[L3]** A module that grows past a few hundred lines and mixes unrelated responsibilities is a **refactor**, not a feature — **split before adding**.
- **[L2]** A new file under `crates/*/src/services/`, `crates/storage/src/`, or similar must have an **obvious caller** in the same PR. Code that is **enqueued but never consumed** (or consumed but never produced) is an **integrity bug**, not a TODO.

---

## 13. The knife — demo scenario (must stay green)

This is the **minimum bar** for “we did not break the product direction.” Extend it over time; do not weaken it without a canon update.

**Scenario (current bar):**

1. **[L2]** **Define and persist** a workflow through the API — definition **round-trips**.
   Seam: `crates/api/src/domain/workflow/handler.rs` — `create_workflow`. Test coverage: see `docs/MATURITY.md`.
2. **[L2]** **Activate** the workflow. Activation runs validation and **rejects** invalid definitions with structured RFC 9457 errors — it does **not** silently flip a flag.
   Seam: `crates/api/src/domain/workflow/handler.rs` — `activate_workflow`. Test coverage: see `docs/MATURITY.md`.
3. **[L2]** **Start an execution** (API or equivalent). The persisted row has consistent `status`, monotonic `version`, a real `created_at`, and no `started_at` until runtime control actually starts it. A response that substitutes `created_at` for `started_at` is a wire-contract migration gap, not release-grade semantics. Durable acceptance must also leave recoverable work if the request process dies; the current split `ExecutionStore::create` then `ControlQueue::enqueue` path is a migration gap, not atomic acceptance.
   Current seam: `crates/api/src/domain/execution/handler.rs` — `start_execution`; storage contracts: `crates/storage-port/src/store/execution.rs` and `crates/storage-port/src/store/control_queue.rs`. Test coverage: see `docs/MATURITY.md`.
4. **[L2]** **Observe** via GET — `finished_at` is `None` (not `0`) until terminal; `status` reflects the latest persisted value.
5. **[L2]** **Request cancellation** on a non-terminal execution:
  - the request reaches runtime control; the physical transition uses **`ExecutionStore::commit(TransitionBatch)`** (CAS + fencing),
  - the **same logical operation** enqueues **`Cancel`** in `execution_control_queue`,
  - a dispatch consumer wired to the **real engine** observes the command and the engine’s cancel path runs,
  - the execution reaches a **terminal** `Cancelled` state without hand-waved stubs.
   Current implementation seam: `crates/api/src/domain/execution/handler.rs` — `cancel_execution` / `AppState::cas_transition_with_control_scoped`; atomic contract: `crates/storage-port/src/store/execution.rs` + `crates/storage-port/src/batch.rs`. The direct handler-to-store call is a migration gap toward runtime-control command submission. Test coverage: see `docs/MATURITY.md`.
6. **[L2]** Under test configuration where orchestration is intentionally absent: control endpoints return **503** — never fake success and never an unparsable 500.

**Integration bar (same spirit as execution — must stay green as these paths exist):**

1. **[L2]** **Plugin load → registry:** a plugin loads; **Actions / Resources / Credentials** from `impl Plugin` appear in the catalog (or equivalent) **without** a second manifest that duplicates `fn actions()` / `fn resources()` / `fn credentials()` (§7.1).
2. **[L2]** **Credential refresh / rotation:** where rotation or refresh is implemented, it does **not** silently strand or corrupt **in-flight** executions that hold valid material — failure is **explicit** in status or errors if the system cannot reconcile.
3. **[L2]** **Resource lifecycle visibility:** acquire → use → **release** for Resource-backed steps is **attributable** in **durable journal** or an **operator-visible** trace (aligned with §11.4) — not only in ephemeral logs.
4. **[L2]** **Trigger delivery semantics:** for TriggerAction-backed starts, tests cover the declared delivery contract (**at-least-once** unless explicitly stronger): no silent drop, and duplicate delivery is handled via stable event identity + dedup/idempotency (aligned with §9 and §11.3).
5. **[L2]** **Non-idempotent side effects:** for ordinary Actions that can cause irreversible external effects (e.g. charge/refund/payout), integration tests prove **single-effect safety** under retry/restart/duplicate-dispatch pressure: idempotency key guard is applied before the side effect, and re-entry does **not** execute the external effect twice.

### 13.2 Rotation refresh seam

The canonical bar for credential rotation and refresh discipline (referenced as `§13.2` from credential-system ADRs and sub-specs). Restates the contract behind Integration bar item 2 above and ties it to the credential-owned rotation/refresh seam (relocated into `nebula-credential::runtime` by ADR-0092, superseding ADR-0030 §3) and cross-replica refresh coordination (ADR-0041).

- **[L2]** **No silent strand:** an in-flight execution holding valid auth material survives a concurrent rotation or refresh of that credential — the engine completes the in-flight work against the material it observed at acquire time and does **not** mid-call swap to a fresher value that the action did not consent to.
- **[L2]** **Explicit failure on irreconcilable state:** when reconciliation cannot succeed (e.g. provider rejected the refresh token, sentinel threshold tripped on repeated mid-refresh crashes per ADR-0041), the credential transitions to an explicit `ReauthRequired` state surfaced in `CredentialStatus` — never a silent stuck credential and never a synthetic success.
- **[L2]** **Cross-replica coordination is durable, not folklore:** when running multi-replica, only one replica refreshes a credential per expiry window. The L2 claim repository (ADR-0041) is the ground truth; in-process L1 coalescing is an optimization on top, not a substitute.

Tied seams: ADR-0028 cross-crate invariants (rotation/refresh boundaries between credential / storage / engine), ADR-0030 §3 (historical — engine-owned orchestration; **superseded by ADR-0092**: refresh coordinator + rotation-state now live in `nebula-credential::runtime`), ADR-0033 integration-credentials Plane B, ADR-0041 durable refresh claim repository. The credential Tech Spec §15.7 `SchemeGuard` (handed to resources at refresh time) prevents retention past the call site so a rotated credential does not bleed into the next request through a stale handle.

**What “done” means for a change touching execution / API / storage / plugins:**

- **[L2]** **Integration tests** exercise the path end-to-end, including **step 5** (engine-visible cancel), not only DB metadata.
- **[L2]** Changes to **plugin registration**, **credential refresh**, **resource release**, **trigger ingestion**, or **non-idempotent action execution** that affect §3.5 claims require **coverage** for steps **7–11** where those features are touched — or an explicit canon note that the bar is **narrowed** (not silent regression).
- **[L2]** No new dispatch path, queue, or in-memory channel without an explicit **§12.2** update.
- **[L2]** `simple_server.rs` (and similar) either **honors cancel end-to-end** or carries `// DEMO ONLY` naming exactly which steps are stubbed.

---

## 14. Anti-patterns — do not ship

- **[L1]** **Two truths:** execution state in DB says X, channel/queue says Y, with no formal reconciliation story. (See §12.2.)
- **[L1]** **Phantom types:** enum variants or trait methods the engine **rejects at runtime** — e.g. reintroducing an `ActionResult::Retry`-style variant without persisted accounting. **Implement end-to-end or delete.**
- **[L1]** **Discard-and-log workers:** a dispatch loop that drains an outbox and “handles” commands with `tracing::info!` only — **not** a consumer; it is a leak.
- **[L1]** **Validation-as-a-side-tool:** workflow validation only at `/validate` while **activation skips** it.
- **[L1]** **Green tests, wrong product:** shortcuts that pass tests but violate §12 (e.g. `String` errors in new library crates, new `ExecutionControl` semantics that bypass storage).
- **[L1]** **Framework before product:** abstractions multiplying faster than invariants; types ahead of engine-owned semantics.
- **[L1]** **Trait surface faster than engine truth** — new public trait families or result variants without end-to-end behavior.
- **[L1]** **Runtime rejection instead of validation** for workflow shape where validation is feasible.
- **[L1]** **Best-effort persistence presented as durable truth** — or ambiguous ownership of “what happened.”
- **[L1]** **Docs that describe future intent as current contract** — or internal channels treated as durable infrastructure.
- **[L1]** **README drift:** advertising a backend, capability, or step the code no longer supports.
- **[L1]** **God files:** continuing to add unrelated logic to a file that already exceeds reasonable responsibility instead of splitting (module or crate) when boundaries are clear.
- **[L1]** **Orphan modules:** services, queues, or repos **produced but never consumed** (or vice versa). See §12.7.
- **[L1]** **Spec theater:** long `docs/` plans that contradict this file without a canon revision — **plans follow canon**, not the reverse. The same applies to **this** file: if the change is really about `nebula-resource` APIs, update **`crates/resource/README.md`** or **`docs/INTEGRATION_MODEL.md`**, not a long integration-mechanics section in `docs/PRODUCT_CANON.md`.

---

## 15. How other docs relate

| Document | Role |
|---|---|
| `AGENTS.md` | Commands, formatting, session read-order, decision gate, trap catalog. |
| `docs/PRODUCT_CANON.md` (this file) | Normative core — pillars (§4), golden path (§10), contracts (§11), invariants (§12), knife (§13), anti-patterns (§14), decision filter (§16), DoD (§17). Layer-tagged. |
| `docs/INTEGRATION_MODEL.md` | Integration model mechanics — Resource / Credential / Action / Schema / Plugin contract, wiring rules, plugin packaging, status of aspirational surfaces. |
| `docs/OBSERVABILITY.md` | SLI / SLO / error budgets, structured event schema, operator core-analysis loop. |
| `docs/MATURITY.md` | Per-crate state dashboard (API stability, test coverage, doc completeness, engine integration, SLI-ready). |
| ADRs (private vault) | Architecture Decision Records — `NNNN-kebab-title.md`. Maintained in the maintainers' private design vault, not in this repo; `ADR-NNNN` ids remain stable textual references. |
| `README.md` | Operator-facing summary. Must not contradict §5 / §11.5 / §12.3. |
| `crates/*/README.md` + `lib.rs //!` | Per-crate: Role (named pattern), Contract (invariants + seam tests), Public API, Non-goals, Maturity. |
| `crates/storage/migrations/{sqlite,postgres}/README.md` | Schema parity between dialects. |

Every substantial spec or phase plan must include:

- **Canon:** sections advanced (e.g. §12.2, §12.7, §13).
- **Explicitly out of scope** for that phase.

**Guarantees narrative:** maintain or add a concise place (see §11.6) that operators can read without spelunking crates.

---

## 16. Decision filter (major features / refactors)

Before merging substantial surface-area or behavior changes, ask:

1. **[L1]** Does this **strengthen the golden path** (§10)?
2. **[L1]** Does this **clarify or blur** the product contract (§11)?
3. **[L1]** Does the engine **actually honor** the behavior now? If **not**, the type, handler, or endpoint does **not** ship.
4. **[L1]** Does this **reduce or increase** contributor cognitive load?
5. **[L1]** Does this make local / self-hosted / future hosted stories **more coherent**, or only broader on paper?
6. **[L1]** Does this help operators **understand failures**?
7. **[L1]** Is this **foundational now**, or speculative future-proofing?
8. **[L1]** If we ship this, are we making a **real promise**?
9. **[L1]** Does this **align with the competitive bets** in §2.5 (typed durability vs soft ecosystem, checkpoint/local-first vs replay/compose-heavy, Rust contracts vs script glue) — or does it blur those lines without updating the canon?
10. **[L1]** Does this **preserve the §3.5 integration model** (orthogonal concepts, `*Metadata + Schema`, plugin wiring rules) **and** avoid **spec theater** — duplicating crate-level API detail in this file instead of updating `crates/*/README.md` or `docs/INTEGRATION_MODEL.md`?
11. **[L2]** If this introduces a **queue, channel, or worker:** are **producer**, **consumer**, and **failure mode** all in **this PR** (or explicitly documented as out of scope with no orphan half)?

If the answer implies a **false capability**, the change is not ready — hide, narrow, or implement to completion first.

---

## 17. Definition of done — by canon layer

Incomplete if:

- **[L1]** violated without an explicit canon revision + product-level rationale in the PR description.
- **[L2]** violated without an ADR (in the maintainers' private design vault) + a seam test update in the same PR.
- **[L3]** violated without a PR rationale (one paragraph in the PR body) and, if behavior changed, a test.
- **[L4]** “violation” is not a violation — it is a move of detail into the owning crate's README. If you think an L4 rule is in canon, open an ADR per §0.2 and move it.

Additional DoD items (unchanged from prior canon):

- §13 knife scenario (execution and integration bar where those features exist) not broken or narrowed without replacement by a stronger scenario.
- No new public API surface that contradicts typed-error / layering rules.
- A new public behavioral contract requires §11-level honesty; docs must not mislabel implementation state.
- A new outbox / queue / worker lands with its consumer (or explicit §12.7 exemption) in the same PR.
- A removed backend, endpoint, or capability is not still advertised in `README.md` or `docs/`.
- Local path stays documented in `AGENTS.md` or `README` where applicable.

---

## 18. Change history policy

`docs/PRODUCT_CANON.md` is the source of current product truth, not a long-form changelog.
Track canon evolution in git history (commit log / PR discussion). Keep this file focused on
normative guidance and avoid in-file revision tables that grow unbounded.
