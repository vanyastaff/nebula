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
- **[L2 Invariant]** — testable contract with a named code seam. Material semantic change requires an ADR (`docs/adr/`) and an updated seam test in the same PR. Wording polish does not.
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

**[L1]** **Nebula is a high-throughput workflow orchestration engine with a first-class integration SDK** — the typed integration surface (`nebula-schema`, `nebula-resource`, `nebula-credential`, `nebula-action`, plus the plugin registry — see `docs/INTEGRATION_MODEL.md`) — **Rust-native, self-hosted, owned by you.**

---

## 2. Position

**[L1]** Nebula is a Rust-native workflow automation engine: DAG workflows, typed boundaries, durable execution state, explicit runtime orchestration, first-class credentials / resources / actions.

**[L1]** Primary audience: developers writing integrations. Secondary: operators deploying and composing workflows.

**[L1]** Competitive dimension: reliability and clarity of execution as a system, plus DX for integration authors.

For peer analysis, our explicit bets against n8n / Temporal / Windmill / Make / Zapier, and what we borrow from each, see `docs/COMPETITIVE.md`. That document is persuasive; this canon stays normative.

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
- **Credential** — who you are and how authentication is maintained. Engine owns rotation and the stored-state vs consumer-facing auth-material split.
- **Action** — what a step does. Dispatch via action trait family (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`). Adding a trait requires canon revision (§0.2).
- **Plugin** — distribution and registration unit. Plugin is the unit of registration, not the unit of size — full plugins and micro-plugins use the same contract.
- **Schema** — the cross-cutting typed configuration system (`nebula-schema`: `Field`, `Schema`, `ValidValues`, `ResolvedValues` with proof-token pipeline). Shared across Actions, Credentials, Resources.

**[L1]** Structural contract: every integration concept is `*Metadata + Schema` — UI-facing identity plus typed, validated configuration.

For the full model — structural-contract types, wiring rules, plugin packaging (`Cargo.toml` / `plugin.toml` / `impl Plugin`), plugin signing (status: planned), cross-plugin dependency rules — see `docs/INTEGRATION_MODEL.md`. That document is the authoritative source for integration mechanics; this canon states the invariants.

Sections 3.6 through 3.10 (per-crate pointers) are consolidated in `docs/INTEGRATION_MODEL.md`.

---

## 4. Pillars

Directional goals; binding engineering rules live in §12–§14. The **integration model** (§3.5) explains *what* authors ship; full per-crate and cross-cutting details are in `docs/INTEGRATION_MODEL.md`. The pillars below explain *runtime* and *operations* priorities.

### 4.1 Throughput

**[L1]** Async-native execution (Tokio): many concurrent workflow executions should share a small thread pool without one slow I/O blocking others. Memory per execution should stay in the **hundreds-of-KB** order for common paths (not tens of MB per execution by default shape). **Throughput and latency regressions in benchmarked paths are treated as bugs** where benchmarks exist (e.g. CodSpeed in CI).

### 4.2 Safety

**[L1]** **Fail fast and loudly on misuse:** typed errors, validated node contracts where declared, no silent shape mismatches in production. **Credentials** stay behind existing abstractions (no leakage across boundaries; rotation is not the node author’s ad-hoc problem). **Unsafe** stays in engine/runtime layers — integration-facing APIs remain safe Rust. Resilience classifiers (`nebula-resilience` / `ErrorClassifier` pattern) make transient vs permanent failure an explicit decision, not folklore.

### 4.3 Keep-alive

**[L1]** **Duration:** runs that last **minutes through days** (and longer when storage and checkpoints keep up) are a **normal** design target — not only sub-second HTTP hops.

**[L1]** **Process death:** if the **host process** dies mid-run (deploy, OOM, crash), truth is **only what is persisted** (§11). Work **after the last durable checkpoint / journal line** may be **re-executed**, **rolled back**, or **lost** to the extent those paths are best-effort — the operator must see **status, errors, and journal** that say so, not green-washed success. Cancellation, leases, and the control queue (§12.2) exist so “long-lived” does not mean “hope the process lives forever.”

**[L1]** Integration authors assume unreliable networks; the runtime assumes **restartable processes** and makes resume and cancel **inspectable** (§4.6).

### 4.4 DX

**[L1]** **Integration authoring is the product surface for contributors:** fast scaffolding, test harnesses (`nebula-testing` and friends), actionable errors at API boundaries, integration tests as the reference for how to ship a node. Trait-driven contracts should make missing pieces a **compile-time** story where possible.

### 4.5 Operational honesty — no false capabilities

**[L1]** **Public surface exists iff the engine honors it end-to-end.** A type, variant, or endpoint that can be called but the engine rejects at runtime is a **false capability** — per canon, such types must not ship publicly. Options:

1. **Implement end-to-end** — wire the behavior through `ExecutionRepo`, resilience pipeline, persistence, observability.
2. **Make the surface private or feature-gated** — `pub(crate)` or gated under `unstable-*` feature so consumers cannot bind to what the engine does not yet deliver.
3. **Remove the surface entirely.**

**[L1]** Corollaries:

- **Misconfiguration moves left.** Validation / activation-time checks over runtime rejection, wherever feasible for workflow shape.
- **JSON at edges is fine; JSON instead of validated boundaries is not.** Schemas and compatibility rules at workflow / action boundaries win over unstructured blobs.
- **In-process channels decouple components but are not a durable backbone.** Anything requiring reliable delivery — including cancel and dispatch signals — must share the persistence transaction with the owning state transition, or live in an explicit durable outbox with documented at-least-once semantics (see §12.2). A channel whose consumer logs and discards is not a contract.

See also `docs/STYLE.md` §5 (type design bets) for the Rust patterns that make this invariant easy to uphold: sealed traits, typestate, `#[non_exhaustive]`, `#[unstable]` feature gates.

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
| **Rust-first** integration model (FFI/plugin evolution is additive, not a second-class hack)                        | A low-code platform where the primary author is non-developer glue    |
| Honest docs: in-process sandbox = **capability / correctness**, not attacker-grade isolation                        | Claims of full untrusted-code isolation without an OS-hardened child-process boundary or a microVM-grade backend (see §12.6 — WASM is **not** on the isolation roadmap) |
| **Self-hosted identity** first; cloud-style deployment is “Nebula on infra,” not a different product                | Hosted-service-first product with different core guarantees           |
| Breaking **wrong** internal APIs when the cost of shims exceeds clarity                                             | Compatibility shims that preserve bad shapes “for now”                |


> **Local storage truth:** v1-era wording suggested two backends (“SQLite or in-memory”). There is **one** local storage path — **SQLite** — usable against a file or `sqlite::memory:`. In-process tests use **`nebula_storage::test_support`** (`sqlite_memory_*` helpers), not a separate HashMap “memory backend.” README / onboarding that still advertise a distinct in-memory backend must be updated to match.

> **Supported production path:** the deployment configuration Nebula claims operators can rely on **today** is **SQLite local** and **Postgres self-hosted**. Anything outside this set (e.g. Redis dependencies, FFI isolation, cloud multi-tenant modes) is additive and must be explicitly marked **experimental** or **planned** until this canon says otherwise.

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


---

## 7. Open source contract

- **[L1]** **Public integration / plugin SDK surface:** stability matters; breaking changes deserve an RFC-style decision, not drive-by commits.
- **[L2]** **Workspace internals:** may break when wrong — but **not** silently: canon + migration note + tests (see §17).
- **[L1]** **Ecosystem quality over node count:** one solid canonical integration per external service beats many half-finished duplicates.
- **[L1]** **Third-party nodes** are first-class in intent: same capabilities as first-party where the plugin model allows; **document** what is shipped vs planned.

### 7.1 Plugin packaging

**[L1]** Plugin is the unit of **registration**, not the unit of size. Full plugins and micro-plugins use the same contract: Rust crate + `plugin.toml` marker + `impl Plugin`.

**[L2]** Three sources of truth, no duplication:


- **`Cargo.toml`** — Rust package identity and dependency graph (including cross-plugin `[dependencies]`).
- **`plugin.toml`** — trust + compatibility boundary (SDK constraint, optional stable plugin id, signing when enabled). Read without compiling.
- **`impl Plugin` + `PluginMetadata`** — runtime source of truth for registered actions / resources / credentials / locales.

**[L2]** Cross-plugin types come in via `Cargo.toml` `[dependencies]` on the provider plugin crate. Engine loads providers before dependents (acyclic graph). Referencing a type outside the declared dependency closure is a misconfiguration caught at activation.

**[L4]** Full packaging mechanics — `[nebula]` / `[plugin]` / `[signing]` table shapes, signing rationale, layout examples, FFI-path notes — live in `docs/INTEGRATION_MODEL.md`.

### 7.2 Engine upgrade and workflow compatibility

**Operators** need a clear story when the **engine** version changes — not only plugin authors.

- **[L2]** **Persisted workflow definitions** and **plugins** (binaries / SDK linkage) are **two compatibility surfaces**; breaking either belongs in **release notes** and migration guidance.
- **[L2]** **Patch and minor** releases **must** keep **forward-compatible** workflow JSON and documented **plugin SDK** boundaries unless the release **explicitly** announces a break.
- **[L3]** **Plugin binary compatibility:** Rust plugin crates are compiled artifacts tied to SDK/engine versions; upgrades may require recompilation against the target `nebula-api` / SDK version. Binary-stable ABI is an **FFI path** concern (e.g. stabby), not an implicit guarantee for native Rust plugin binaries.
- **[L2]** **Breaking** workflow schema, execution semantics, or public SDK types require **documented migration**, tests, and upgrade notes — not an assumption that existing installs “should work.”
- **[L1]** Do **not** claim “all v1 workflows run unchanged on v2” without a **published compatibility matrix** or equivalent — platform trust requires **honest** upgrade paths.

---

## 8. What Nebula is not

- **[L1]** **Not a low-code tool** — operators may compose graphs; **authors** target Rust (and future FFI), not replacement of typed integration work.
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
5. **[L2]** Execution state transitions are **visible and attributable** through `ExecutionRepo` with **version-checked CAS**; no handler invents an out-of-band lifecycle.
6. **[L2]** Failure, cancellation, retry, and timeout behavior match **documented** contracts — not folklore in traits. **Cancel** requests must be **durable and engine-consumable** (see §12.2), not “only the DB row changed.”
7. **[L1]** **Persistence story is explicit:** what is durable vs best-effort; what resume/replay may assume; what happens on checkpoint failure.
8. **[L1]** Operator can **inspect** what happened and what is trustworthy.

Anything that does not strengthen this path is secondary until the canon says otherwise.

---

## 11. Core product contracts (honesty)

These must stay **explicit in code and operator-facing docs**, not split across half-implemented types.

### 11.1 Execution authority

**[L2]** `nebula-execution` + `ExecutionRepo` are the **single source of truth** for execution state. Transitions use **optimistic CAS** against persisted `version`. There is no ephemeral “usually DB wins” mode: if persistence is unavailable, the operation **fails** — it does not silently mutate in-memory state.

### 11.2 Retry

**[L2]** Retry is a **runtime semantic** owned by the **engine** and **`nebula-resilience`** pipelines around **outbound** calls inside an action — not a decorative hint on a return type. The engine **does not** schedule re-execution of a failed node from an `ActionResult::Retry`-style return unless that path is wired with **persisted attempt accounting**. If such a variant exists but is not honored end-to-end, it is a **false capability** (remove it or implement it). Until durable per-attempt retry accounting exists, the canonical retry surface is the **resilience pipeline** an action uses internally.

**Status (per §11.6 vocabulary):**

| Surface | Status | Notes |
| --- | --- | --- |
| `nebula-resilience` pipeline inside an action (in-memory retry around outbound calls) | `implemented` | The **canonical** retry surface today. Author composes retry/timeout/circuit-breaker at the call site. |
| Engine-level node re-execution from `ActionResult::Retry` with persisted attempt accounting | `planned` | No persisted `attempts` row, no CAS-protected bump, no consumer wired through `ExecutionRepo`. Any return variant that implies it is a **false capability** under §4.5 — hide or delete until end-to-end. |
| Cross-restart retry of a checkpointed step | `best-effort` | Relies on checkpoint boundaries (§11.5); work since the last checkpoint may be replayed or lost. Not a per-attempt contract. |

**[L2]** Canon debt: until the `planned` row above moves to `implemented`, no public API, trait variant, or docs comment may describe engine-level retry as a current capability. Track this row as an **open invariant debt** — revisit whenever `ActionResult`, `ExecutionRepo`, or attempt accounting is touched.

### 11.3 Idempotency

**[L2]** **One** idempotency story: deterministic key shape **`{execution_id}:{node_id}:{attempt}`**, persisted in `idempotency_keys`, checked and marked through `ExecutionRepo` before the side effect. **Engine guarantee:** it will not double-dispatch a **marked** attempt. Whether the **external** system de-duplicates is the integration author’s contract with that system — document per node.

**[L2]** For **non-idempotent or risky side effects** (payments, writes without natural upsert, external one-shot operations), action handlers must guard execution with this idempotency path (or an equivalent documented key contract) before calling the remote system.

**[L2]** For **TriggerAction** sources, each inbound event should carry or derive a stable event identity (provider event id / cursor offset / hash) so at-least-once delivery can be made safe via dedup/idempotent handling; “no duplicates” is not a claim unless the source + runtime can prove it end-to-end.

### 11.4 Resource lifecycle

**[L2]** Resources are first-class because **acquisition** and **scope-bounded release** are **engine-owned**. The async release path is **best-effort on crash** — orphaned resources rely on the next process to drain via `DrainTimeoutPolicy` / `ReleaseQueue`. Operators must be told this; authors must not assume “release ran” without an explicit checkpoint.

**[L1]** For long-lived exclusive/external resources (locks, leased cloud instances), deployments need external TTL / dead-man strategy; Nebula v1 does not provide an external lease arbiter by itself.

### 11.5 Persistence & operators

**[L2]** Checkpointing is **policy-driven**, not “fsync every step.” The engine checkpoints at declared boundaries (workflow/action policy) and on workflow completion. Between checkpoints, progress is in-memory: process death can lose work since the last checkpoint.

**[L2]** Authors should place checkpoint boundaries before irreversible or expensive side effects; the engine does not guess those boundaries for you.


| Artifact                           | Status                                                                | Operator-visible truth                                                                                                                                                           |
| ---------------------------------- | --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `executions` row + state JSON      | **Durable** (CAS via `ExecutionRepo`)                                 | Source of truth                                                                                                                                                                  |
| `execution_journal` (append-only)  | **Durable**                                                           | Replayable history                                                                                                                                                               |
| `execution_control_queue` (outbox) | **Durable**                                                           | At-least-once dispatch + cancel signals (§12.2)                                                                                                                                  |
| `stateful_checkpoints`             | **Durable at checkpoint boundaries**; failure mode is **best-effort** | Checkpoint write failure may **log** and **not** abort execution; resume falls back to last successful checkpoint or journal; work since last checkpoint may be replayed or lost |
| `execution_leases` (schema)        | **Schema may exist before full enforcement**                          | If the engine does not consume leases yet, **say so** — do not imply lease safety                                                                                                |
| In-process `mpsc` / channels       | **Ephemeral**                                                         | Never authoritative truth                                                                                                                                                        |


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

- **[L2]** Follow `CLAUDE.md` dependency direction. **No upward dependencies** between layers.
- **[L2]** `crates/api` does not embed SQL drivers or storage schema knowledge beyond declared ports; **storage and orchestration details live in their crates**.

### 12.2 Execution: single semantic core, durable control plane

- **[L2]** **Authoritative execution state** lives in `nebula-execution` + `ExecutionRepo`. Handlers and API DTOs **do not invent a parallel lifecycle**, do not mutate state without going through **`ExecutionRepo::transition`** (CAS on `version`), and do not return **synthesized** timestamps or fake defaults for missing fields.
- **[L2]** **Every “run this” / “cancel this” signal must be durable and engine-consumable.** The contract is:
  1. The signal is written to **`execution_control_queue`** (outbox) **in the same logical operation** as the corresponding state transition. A handler that flips state to `cancelling` **without** enqueueing — or enqueues **without** transitioning — is broken.
  2. A dispatch worker drains the queue and forwards commands to a consumer that **the engine actually listens to**. Removing rows **before** the engine has acted is broken.
  3. There is **one** consumer wiring story per deployment mode, **documented in code**.
- **[L2]** **A demo handler that logs the command and discards it does not satisfy this invariant.** Examples and `simple_server.rs` must either wire a **real** engine consumer or be marked `// DEMO ONLY — does not honor cancel` so nothing mistakes them for the contract.
- **[L2]** **Batching outbox writes for throughput is valid only if per-transition atomicity is preserved.** Never batch as a workaround that breaks “state transition + control signal” integrity.
- **[L2]** Any second control channel (HTTP webhook, in-memory event) is **forbidden** unless this canon is updated with a **reconciliation** story.

### 12.3 Local path

- **[L3]** The **default developer experience** must allow: build, run tests, run core flows **without** Docker, Redis, or external brokers. **SQLite** is the default local storage; `sqlite::memory:` via `nebula_storage::test_support` is the reference in-process path.
- **[L3]** Optional production paths (**Postgres**, later Redis, etc.) are **additive**, not prerequisites for “hello world” or CI sanity. `examples/simple_server.rs` (and similar) must continue to start **without** external services unless explicitly documented as integration-only.

### 12.4 Errors and contracts

- **[L3]** Library crates: `thiserror`, not `anyhow`, in public library surfaces.
- **[L2]** API boundary: **RFC 9457 `problem+json`** (see `crates/api/src/errors.rs`). **No new ad-hoc `500`** for business-logic mistakes — map new failure modes into typed `ApiError` variants with an explicit status.
- **[L3]** `serde_json::Value` is allowed **where it is the deliberate interchange type**; new **stringly protocols** (magic field names without schema validation) require explicit review.

### 12.5 Secrets and auth

- **[L2]** No secrets in logs, error strings, or metrics labels. **`Zeroize` / `ZeroizeOnDrop`** on key material; redacted `Debug` on credential wrappers (`SecretToken`, etc.). Encryption at rest uses authenticated encryption with a KDF — do not bypass “for debugging.” Details: `crates/credential/README.md`.
- **[L2]** Every new `tracing::*!` that takes a credential or token argument must use **redacted** forms.

### 12.6 Isolation honesty

- **[L1]** In-process sandbox / capability checks: **correctness and least privilege for accidental misuse**, not a security boundary against malicious native code. Keep `crates/sandbox` doc comments aligned with this canon and `docs/` threat models.
- **[L1]** **Plugin IPC today:** sequential dispatch over a **JSON envelope** to a child process — that **is** the trust model; do not describe it as **sandboxed execution of untrusted native code**.
- **[L1]** **WASM / WASI is an explicit non-goal for plugin isolation.** The Rust plugin ecosystem integration authors actually need — `redis`, `sqlx` with native drivers, `rdkafka`, `tonic` with native TLS, any `*-sys` crate — does **not** compile to `wasm32-wasip2`, and where parts compile, the feature surface forces authors into host-polyfill folklore that violates the §3.5 promise ("Write Stripe logic; do not write credential rotation, connection management, or retry folklore"). Offering WASM as "the future sandbox" would be a §4.5 false capability and a §4.4 DX regression at the same time. **The real isolation roadmap is:** `ProcessSandbox` (already shipping) → full `PluginCapabilities` enforcement wired from `plugin.toml` through discovery (closes `nebula-sandbox/src/discovery.rs:117`) → `plugin.toml` signing verification in tooling (canon §7.1) → per-platform OS hardening in `os_sandbox` (seccomp-bpf / landlock on Linux, `sandbox_init` on macOS, `AppContainer` / job objects on Windows) → parallelism within `ProcessSandbox` for throughput (§4.1). Revisit WASM only if the Rust WASM ecosystem crosses a specific, documented capability threshold — not as aspiration, and never as docs drift in crate-level `lib.rs` or README.

### 12.7 No god files, no orphan modules

- **[L3]** A module that grows past a few hundred lines and mixes unrelated responsibilities is a **refactor**, not a feature — **split before adding**.
- **[L2]** A new file under `crates/*/src/services/`, `crates/storage/src/`, or similar must have an **obvious caller** in the same PR. Code that is **enqueued but never consumed** (or consumed but never produced) is an **integrity bug**, not a TODO.

---

## 13. The knife — demo scenario (must stay green)

This is the **minimum bar** for “we did not break the product direction.” Extend it over time; do not weaken it without a canon update.

**Scenario (current bar):**

1. **[L2]** **Define and persist** a workflow through the API — definition **round-trips**.
2. **[L2]** **Activate** the workflow. Activation runs validation and **rejects** invalid definitions with structured RFC 9457 errors — it does **not** silently flip a flag.
3. **[L2]** **Start an execution** (API or equivalent). The execution row exists with consistent `status`, monotonic `version`, and a real `started_at` (no synthetic zero, no placeholder `now()` where the field should be `None`).
4. **[L2]** **Observe** via GET — `finished_at` is `None` (not `0`) until terminal; `status` reflects the latest persisted value.
5. **[L2]** **Request cancellation** on a non-terminal execution:
  - the handler transitions through **`ExecutionRepo`** (CAS),
  - the **same logical operation** enqueues **`Cancel`** in `execution_control_queue`,
  - a dispatch consumer wired to the **real engine** observes the command and the engine’s cancel path runs,
  - the execution reaches a **terminal** `Cancelled` state without hand-waved stubs.
6. **[L2]** Under test configuration where orchestration is intentionally absent: control endpoints return **503** — never fake success and never an unparsable 500.

**Integration bar (same spirit as execution — must stay green as these paths exist):**

1. **[L2]** **Plugin load → registry:** a plugin loads; **Actions / Resources / Credentials** from `impl Plugin` appear in the catalog (or equivalent) **without** a second manifest that duplicates `fn actions()` / `fn resources()` / `fn credentials()` (§7.1).
2. **[L2]** **Credential refresh / rotation:** where rotation or refresh is implemented, it does **not** silently strand or corrupt **in-flight** executions that hold valid material — failure is **explicit** in status or errors if the system cannot reconcile.
3. **[L2]** **Resource lifecycle visibility:** acquire → use → **release** for Resource-backed steps is **attributable** in **durable journal** or an **operator-visible** trace (aligned with §11.4) — not only in ephemeral logs.
4. **[L2]** **Trigger delivery semantics:** for TriggerAction-backed starts, tests cover the declared delivery contract (**at-least-once** unless explicitly stronger): no silent drop, and duplicate delivery is handled via stable event identity + dedup/idempotency (aligned with §9 and §11.3).
5. **[L2]** **Non-idempotent side effects:** for ordinary Actions that can cause irreversible external effects (e.g. charge/refund/payout), integration tests prove **single-effect safety** under retry/restart/duplicate-dispatch pressure: idempotency key guard is applied before the side effect, and re-entry does **not** execute the external effect twice.

**What “done” means for a change touching execution / API / storage / plugins:**

- **[L2]** **Integration tests** exercise the path end-to-end, including **step 5** (engine-visible cancel), not only DB metadata.
- **[L2]** Changes to **plugin registration**, **credential refresh**, **resource release**, **trigger ingestion**, or **non-idempotent action execution** that affect §3.5 claims require **coverage** for steps **7–11** where those features are touched — or an explicit canon note that the bar is **narrowed** (not silent regression).
- **[L2]** No new dispatch path, queue, or in-memory channel without an explicit **§12.2** update.
- **[L2]** `simple_server.rs` (and similar) either **honors cancel end-to-end** or carries `// DEMO ONLY` naming exactly which steps are stubbed.

---

## 14. Anti-patterns — do not ship

- **[L1]** **Two truths:** execution state in DB says X, channel/queue says Y, with no formal reconciliation story. (See §12.2.)
- **[L1]** **Phantom types:** enum variants or trait methods the engine **rejects at runtime** — e.g. `ActionResult::Retry` with no persisted accounting. **Implement end-to-end or delete.**
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


| Document                                                                        | Role                                                                                                                                                                                                          |
| ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CLAUDE.md`                                                                     | Commands, formatting, layer diagram, agent workflow                                                                                                                                                           |
| `docs/PRODUCT_CANON.md` (this file)                                             | Product direction, competitive bets (§2.5), integration model summary (§3.5), OSS / plugin (§7.1–§7.2), pillars (incl. §4.6), contracts, invariants, knife (§13) |
| `README.md`                                                                     | Operator-facing summary; **must not contradict** §5 / §11.5 / §12.3 — fix drift **on the same PR**                                                                                                            |
| `docs/` specs                                                                   | Detailed design — **subordinate**; conflict ⇒ fix spec or update canon deliberately                                                                                                                           |
| `docs/PLUGIN_MODEL.md` / `docs/ENGINE_GUARANTEES.md` / `docs/UPGRADE_COMPAT.md` | Satellite detail docs for §7.1, §11, §7.2 respectively; keep mechanics there, keep canon normative.                                                                                                           |
| `docs/GLOSSARY.md`                                                              | Navigation aid: canonical identifiers (types, traits, tables) referenced by this file, grouped by layer, with canon status. Not normative — canon wins on conflict.                                          |
| `crates/storage/migrations/{sqlite,postgres}/README.md`                         | Source of truth for **schema parity** between dialects (where present)                                                                                                                                        |


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

## 17. Definition of done (beyond green tests)

Incomplete if:

- §12 violated without updating this file and a short migration note in the PR.
- §13 knife scenario (execution **and** integration bar where those features exist) broken or narrowed without replacing it with a **stronger** scenario.
- New public API surface that contradicts typed-error / layering rules “temporarily.”
- New **public behavioral** contract (retry, idempotency, resource scope, etc.) without §11-level honesty — or docs that mislabel implementation state.
- A new **outbox / queue / worker** landed **without** its consumer (or vice versa) in the same PR — unless explicitly exempted with **no orphan wiring** (§12.7).
- A removed backend, endpoint, or capability is still **advertised** in `README.md` or `docs/`.
- “Works on my machine” only via undocumented env vars that become mandatory for basic dev — **local path must stay documented** in `CLAUDE.md` or `README` (where applicable).

---

## 18. Change history policy

`docs/PRODUCT_CANON.md` is the source of current product truth, not a long-form changelog.
Track canon evolution in git history (commit log / PR discussion). Keep this file focused on
normative guidance and avoid in-file revision tables that grow unbounded.