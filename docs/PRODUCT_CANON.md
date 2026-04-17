# Nebula — product canon

**Authority:** This file overrides conflicting specs, plans, and chat assumptions until explicitly revised.  
**Audience:** Every implementer (human or agent) before non-trivial changes.

If a task would violate this document, **stop**: either update this canon in a deliberate commit or change the approach.

## Purpose

This canon exists to **keep Nebula honest**: we do not ship a sophisticated framework that promises more than the engine can reliably deliver. When roadmap pressure, architecture taste, or new abstractions compete with product truth, **this document wins**.

Nebula is judged by **what operators and integration authors can trust**, not by how elegant the internal type system looks.

> **Audit alignment:** Binding rules in §11–§13 were re-grounded against the **live workspace** (storage tables, execution control queue, activation validation). If `README.md` or onboarding text contradicts §5 / §11.5 / §12.3, treat that as a **bug** — fix docs in the same PR as code, or update this canon deliberately.

---

## 1. One-line definition

**Nebula is a high-throughput workflow orchestration engine with a first-class integration SDK** — the typed surface in §3.5 (`nebula-parameter`, `nebula-resource`, `nebula-credential`, `nebula-action`, plugin registry) — **Rust-native, self-hosted, owned by you.**

---

## 2. Position (expanded)

**What Nebula is:** A **Rust-native workflow automation engine**: DAG workflows, typed boundaries, durable execution state, explicit runtime orchestration, first-class credentials/resources/actions — not a thin script runner. With room to grow from practical DAG workflows into richer execution models later.

**Peers by problem space (not a single category):** **n8n**, **Zapier**, **Make**, **Temporal**, **Windmill** — each solves a slice of automation/orchestration; Nebula is closest to **self-hosted workflow engines + durable execution**, not to SaaS iPaaS.

**Nebula’s bet against all of them:**

- **Runtime honesty** over feature breadth.
- **Typed authoring contracts** over scriptable glue with opt-in validation.
- **Local-first** (single process / minimal deps) over “managed infrastructure minimum” (e.g. compose-only local path).

**Who it is for (primary):** Developers who **write integrations and nodes** — first-party core, community nodes, or internal nodes for a deployment. They need ergonomics, correct boundaries under failure, and confidence that the runtime handles throughput and resilience so they focus on integration logic.

**Who it is for (secondary):** **Operators** who deploy Nebula and compose workflows from existing nodes — they need clarity on durability, recovery, isolation, and observability.

**Pain we solve:** Many workflow tools treat integrations as **second-class** (opaque SDK, leaky abstractions) and assume the **happy path** (short runs, reliable networks). Nebula bets on **explicit state, clear layering, and operational honesty** (resumability, cancellation, leases, journals) without requiring a zoo of external services for the default local path.

**Competitive dimension (do not dilute):** Reliability and clarity of execution **as a system**, plus **DX for integration authors** — not feature parity with n8n/Make on day one, and **not** a surface-area race in v1.

**Success in one sentence:** *You can explain what happened in a run, recover or cancel safely, and trust the boundaries — not because marketing says so, but because the model matches operational reality.*

### 2.5 Competitive bets

We have studied the leading tools. Each has a real insight. Each has a real ceiling. Nebula makes explicit bets about where those ceilings are.

**n8n**

- **Insight:** Visual graph + self-hosted + large node library is a real product.
- **Ceiling:** JS runtime means no compile-time contracts; node quality is inconsistent; engine-level durability is limited (restart often implies re-run from scratch for many flows); concurrency does not scale to very high throughput without pain.
- **Our bet:** Typed Rust integration contracts + honest durability beat a large but soft ecosystem; a **smaller library of reliable nodes** wins over time.

**Temporal**

- **Insight:** Durable execution as a first-class primitive is the right model; replaying workflows from history is powerful.
- **Ceiling:** Operational complexity is real (worker fleet, persistence cluster, replay constraints bleed into authoring); DX is heavy outside large teams; local path often means **Docker Compose or equivalent**, not “clone and run.”
- **Our bet:** **Checkpoint-based recovery** with explicit persisted state is operationally simpler and equally honest for the use cases we target; **local-first must mean a single binary / minimal deps**, not a compose file as the default dev path.

**Windmill**

- **Insight:** Self-hosted + scriptable + visual composition works for developers; multi-language (Python/TS) lowers the authoring bar.
- **Ceiling:** Scripts-as-workflows is a thin model; deep resilience primitives are not the center; type safety is often **advisory** (e.g. TS types are not runtime contracts).
- **Our bet:** **Rust-native typed boundaries** + engine-owned retry/recovery beat scriptable glue with optional validation.

**Make / Zapier**

- **Insight:** Integration breadth and low-friction onboarding moves non-developers.
- **Ceiling:** Not a developer-first self-hosted product; limited operational insight for authors; pricing/hosting model is SaaS-centric.
- **Our bet:** **Not competing here** — different primary user and deployment model.

**What we borrow (intellectual honesty)**

- From **n8n:** the **visual graph** as the primary artifact; **open plugin ecosystem** shape.
- From **Temporal:** **durable execution as a contract**, not a convention in docs alone.
- From **Windmill:** **local-first, single-deployment simplicity** as a goal worth defending in product.

---

## 3. The problem & core thesis

**Two failures of common engines:**

1. **Integrations are second-class** — node/connector authoring is an afterthought; DX and docs suffer; the long tail is unmaintained community glue.
2. **The happy path is assumed** — real workflows run long, hit flaky APIs, get restarted mid-flight, and need retries and recovery as first principles.

**Thesis (execution):** **Nebula handles concurrent, durable execution reliably so integration developers can focus on integration logic, not on orchestration infrastructure.** Performance and resilience are the runtime’s job; the author describes what the node does and trusts the engine at scale.

**Thesis (product):**

> **Nebula is a serious orchestration core with honest contracts.** Prefer **fewer real guarantees** over many attractive but soft capabilities.

### 3.5 The integration model — one pattern, five concepts

Most engines give integration authors one abstraction: a **“node”** that receives credentials and config as loosely typed JSON and returns output. **Authentication, connection management, retry, and validation** are the author’s problem, solved ad hoc per integration.

**Nebula’s bet:** the right model is a **small set of orthogonal concepts**, each with a single clear responsibility — and **all sharing the same structural contract**. This is the complement to §2.5: not “faster n8n,” but a **different authoring and operations model**.

#### Structural contract (uniform across concepts)

Every concept in Nebula’s integration layer is described by two things:


| Piece                     | Role                                                                                                                                                                                                                                                                                              |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`*Metadata`**           | UI-facing description — id, display name, icon, version, and concept-specific fields (e.g. `ActionMetadata`: ports, `ActionCategory`, embedded `ParameterCollection` — execution semantics follow the action **traits** in §3.8).                                                                 |
| **`ParameterCollection`** | The **typed configuration schema** — one parameter system (typed, validated, transformer pipeline, dynamic fields, display modes) used **across** concepts. An author learns it once; it applies to Resource config, Credential setup, and Action inputs **without** a different API per concept. |


The **parameter subsystem** (`ParameterCollection` and friends) is the **fifth concept** — cross-cutting configuration machinery, not an afterthought bolted onto each node. **Concrete shape:** see §3.9 (`nebula-parameter`).

#### How the four integration kinds relate (structural, not “whatever exists at runtime”)

These are **schema-level** links: metadata and parameter types say what an Action **requires** and what a Credential **composes** — the engine **resolves** them from registered types. Nothing is satisfied by implicit global lookup.

**Resource** — `[ ResourceMetadata + ParameterCollection ]` — **base, independent.**

Long-lived managed object: connection pool, SDK client, file handle. Engine owns lifecycle: init, health-check, hot-reload via **ReloadOutcome**, scope-aware teardown. The author declares what the Resource **is**; the engine provides it **healthy** or fails loudly. **Concrete shape:** see §3.6 (`nebula-resource`).

**Credential** — `[ CredentialMetadata + ParameterCollection ]` — **optionally composes a Resource** in metadata/schema (e.g. HTTP client **Resource** for token refresh).

**Who** you are and **how** authentication is maintained. Engine owns rotation, refresh, and the **stored state vs consumer-facing auth material** split — the author binds to a Credential type, never hand-rolls refresh or pending OAuth steps, and never relies on secrets appearing in logs. **Twelve universal auth schemes** (plus extensibility via `AuthScheme`) cover OAuth2, API key, mTLS, and similar; the author picks a type and fills the schema. **Concrete shape:** see §3.7 (`nebula-credential`).

**Action** — `[ ActionMetadata + ParameterCollection ]` — **declares zero or more Resource and/or Credential kinds it needs** (by stable id / type reference in the **integration schema**, not ad hoc runtime lookup).

**What** the step does — with explicit semantics. The engine dispatches by **which action trait** the type implements (`StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`, …) — not by a single metadata “kind” field. **`ActionMetadata`** carries key, ports, parameters, isolation, **`ActionCategory`** (Data / Control / Trigger / …), and checkpoint behavior declaration (e.g. **`CheckpointPolicy`**) for UI/validation/runtime policy; this metadata supplements but does not replace trait-based routing. The trait family determines iteration (Continue / Break), trigger lifecycle, graph-scoped resource nodes, and flow-control **`ActionResult`** variants; the **runtime** applies checkpoint, retry, and cancel rules from those contracts — the author does not re-implement those invariants per action (aligned with `nebula-resilience`). **Concrete shape:** see §3.8 (`nebula-action`).

**Wiring rule:** every Resource and Credential an Action references must be **provided by this plugin’s own `impl Plugin` registry** **or** by a type from **another plugin crate** that is a **declared dependency** in **`Cargo.toml`** (engine loads providers before dependents; see §7.1). Referencing a type that is “in the process” but **not** reachable through that **closed dependency graph** — even if another plugin registered it — is a **misconfiguration**, caught at **activation** (or equivalent validation), not a silent runtime grab.

**Plugin** — `[ registry: Actions + Resources + Credentials ]` → **+ localization + additional features**

**Distribution and registration unit.** A Plugin is not only a bundle — it is the **registry** that wires Actions, Resources, and Credentials together under a **versioned** identity, with localization and metadata for the UI. Types **defined in other plugins** are available only when the dependent crate **depends on the provider plugin crate** in **`Cargo.toml`** and the engine respects that **acyclic** graph at load/activation — same closure idea as **Cargo**, not an open global namespace. **Plugin is the unit of registration, not the unit of size:** a “full” integration crate and a **micro-plugin** (one or two registry entries) are **the same kind of thing** — same `plugin.toml` contract, same registration story; see §7.1. Deployment strategies: **native** in-process (maximum performance), **process-isolated** via IPC (third-party sandboxing), **FFI** via stabby (cross-language, stable ABI). Third-party plugins are **first-class by design**; document any gap vs native until the model is complete.

#### Why the uniform pattern matters

**For authors:** learn `{ *Metadata + ParameterCollection }` once — apply to any concept. Write Stripe logic; do not write credential rotation, connection management, or retry folklore. Each concept has one job.

**For operators:** each concept has a clear **owner**. Credential rotation fails → Credential layer. Connection pool leaks → Resource layer. “Something went wrong in the node” is no longer the only diagnostic category.

**For the ecosystem:** Plugin is the **unit of distribution**. Actions, Resources, and Credentials **version together** under one identity; **cross-plugin composition** is explicit via **Cargo dependencies** between plugin crates plus activation-time checks (§7.1), so versioning and install sets stay honest. The UI consumes metadata uniformly. Localization is a **Plugin** concern, not a per-action afterthought.

**Positioning:** this **pattern + separation** is **rare** in our competitive set (§2.5). Treat it as a **primary architectural differentiator** — and **defend it**: do not collapse metadata, parameters, Resources, Credentials, Actions, or Plugin registration back into a single untyped “node struct” in new public APIs without a canon update.

**Canon vs crate docs:** §3.6–§3.9 name the **Rust crates** that realize each integration concept. **Authoritative mechanics** — APIs, topology names, crypto parameters, benchmarks — belong in `crates/*/README.md` and source. This file states **what and why** so the product story does not rot when internals refactor (contrast §14 *spec theater*).

### 3.6 `nebula-resource`

**What / why:** typed **Resource** implementations with **engine-owned** lifecycle (acquire, health, release) instead of ad hoc singletons — so connection pools and clients are **scoped and inspectable**.

**Where to read:** `crates/resource/README.md`, `crates/resource/src/lib.rs`.

### 3.7 `nebula-credential`

**What / why:** unified **Credential** contract — stored state vs projected auth material, refresh/resolve/test paths — so secrets and rotation stay **out of Action code** and logs.

**Where to read:** `crates/credential/README.md`, `crates/credential/src/lib.rs`.

### 3.8 `nebula-action`

**What / why:** **Action** traits, declared dependencies, **`ActionResult`** flow, and metadata-declared execution policy (including **`CheckpointPolicy`** in `ActionMetadata`) so the engine can enforce checkpoints, branching, and retries **honestly** — not untyped “JSON in / out.”

**Where to read:** `crates/action/src/lib.rs` (module map; crate `README.md` may lag).

### 3.9 `nebula-parameter`

**What / why:** one **parameter schema** system (`Parameter`, `ParameterCollection`, validation, conditions) shared by Actions, Credentials, Resources — so configuration is **typed and validated once**, not re-invented per integration.

**Where to read:** `crates/parameter/README.md`, `crates/parameter/src/lib.rs`.

### 3.10 Cross-cutting crates (at a glance)

Besides the **integration** reference crates (§3.6–§3.9), the workspace ships **shared infrastructure** — depended on from many layers; they **support** the model above without replacing it.

- **`nebula-core`** — shared identifiers and keys (`ExecutionId`, `ActionKey`, `CredentialKey`, …), **`AuthScheme`** / **`AuthPattern`**, scope levels, **`SecretString`**, credential lifecycle **events**, dependency-graph helpers — the **vocabulary** other crates agree on.
- **`nebula-error`** — **`Classify`**, **`NebulaError`**, categories/codes, structured details — **one** error taxonomy at boundaries instead of ad hoc strings.
- **`nebula-resilience`** — composable **pipelines** (retry, timeout, circuit breaker, bulkhead, …); pairs with **`ActionError`** / retry hints in **`nebula-action`** (§3.8).
- **`nebula-validator`** — programmatic validators + declarative **`Rule`**; **`nebula-parameter`** embeds rules in **`Parameter`** (§3.9).
- **`nebula-config`** — multi-source, merged, optionally hot-reloaded **host** configuration (binaries/services) — **not** the per-node **`ParameterCollection`** story.
- **`nebula-log`** — structured **`tracing`** pipeline (init, sinks, optional OTel/Sentry hooks).
- **`nebula-telemetry`** — in-memory **metric** primitives (registry, histograms, label interning).
- **`nebula-metrics`** — **`nebula_*` naming**, adapters, Prometheus-style **export** and label-safety guards — sits on top of **`nebula-telemetry`**.
- **`nebula-eventbus`** — typed **broadcast** bus with back-pressure policy; **transport only** — domain **`E`** types live in owning crates.
- **`nebula-expression`** — workflow **expression** evaluation (variable access, operators, functions) for dynamic fields — headless, not a UI.
- **`nebula-system`** — cross-platform **host** probes (CPU/memory/network/disk pressure) for ops and telemetry inputs.
- **`nebula-workflow`** + **`nebula-execution`** — the execution semantics core: workflow validation/shape and durable execution lifecycle/state transitions. Read these when the question is “what does the engine guarantee at runtime,” not just “how integrations are authored.”

**Layering:** cross-cutting crates sit **below** API/engine-specific surfaces (see CLAUDE.md boundaries); they must not **depend upward** on integration-only crates. **Canon use:** reuse these crates for their domains instead of duplicating helpers; if something truly belongs in **`nebula-core`** (a new stable key or auth primitive), extend it deliberately rather than inventing a parallel type in a leaf crate.

---

## 4. Pillars

Directional goals; binding engineering rules live in §12–§14. The **integration model** (§3.5) and **crate pointers** (§3.6–§3.9) explain *what* authors ship; **§3.10** names **cross-cutting infrastructure**. The pillars below explain *runtime* and *operations* priorities.

### 4.1 Throughput

Async-native execution (Tokio): many concurrent workflow executions should share a small thread pool without one slow I/O blocking others. Memory per execution should stay in the **hundreds-of-KB** order for common paths (not tens of MB per execution by default shape). **Throughput and latency regressions in benchmarked paths are treated as bugs** where benchmarks exist (e.g. CodSpeed in CI).

### 4.2 Safety

**Fail fast and loudly on misuse:** typed errors, validated node contracts where declared, no silent shape mismatches in production. **Credentials** stay behind existing abstractions (no leakage across boundaries; rotation is not the node author’s ad-hoc problem). **Unsafe** stays in engine/runtime layers — integration-facing APIs remain safe Rust. Resilience classifiers (`nebula-resilience` / `ErrorClassifier` pattern) make transient vs permanent failure an explicit decision, not folklore.

### 4.3 Keep-alive

**Duration:** runs that last **minutes through days** (and longer when storage and checkpoints keep up) are a **normal** design target — not only sub-second HTTP hops.

**Process death:** if the **host process** dies mid-run (deploy, OOM, crash), truth is **only what is persisted** (§11). Work **after the last durable checkpoint / journal line** may be **re-executed**, **rolled back**, or **lost** to the extent those paths are best-effort — the operator must see **status, errors, and journal** that say so, not green-washed success. Cancellation, leases, and the control queue (§12.2) exist so “long-lived” does not mean “hope the process lives forever.”

Integration authors assume unreliable networks; the runtime assumes **restartable processes** and makes resume and cancel **inspectable** (§4.6).

### 4.4 DX

**Integration authoring is the product surface for contributors:** fast scaffolding, test harnesses (`nebula-testing` and friends), actionable errors at API boundaries, integration tests as the reference for how to ship a node. Trait-driven contracts should make missing pieces a **compile-time** story where possible.

### 4.5 Operational honesty — no false capabilities

**If the engine does not own the behavior, the product does not promise it.**

- **Public surface must not outrun the behavioral core** — retry, resource lifecycle, durability, plugin semantics, and similar are **contracts**, not types alone. A type that exists but is **rejected at runtime** (e.g. an unsupported `ActionResult` variant) is a **false capability** — remove it or implement it end-to-end, do not document around it.
- **Misconfiguration should move left** — validation or activation-time checks where possible; **runtime rejection alone** is not an acceptable primary safety boundary for workflow shape.
- **JSON at edges** is fine; **JSON instead of validated boundaries** is not the long-term direction — schemas and explicit compatibility rules at workflow/action boundaries win over unstructured blobs.
- **In-process channels** (`mpsc`, internal buses) may decouple components; they are **not** an implicit durable backbone. Anything that requires reliable delivery — including **cancel and dispatch signals** — must either share the **same transactional story** as persisted state, or live in an **explicit durable outbox** with documented at-least-once semantics. A channel whose consumer **logs and discards** is not a contract.

### 4.6 Observability

**Durable is not enough — runs must be explainable.** Execution state, append-only journal, structured errors, and metrics should let an **operator** answer what happened and why a run failed **without reading Rust source** — this is the operational half of §2’s success sentence. Observability is **not** optional polish layered on top of throughput; it is how we earn trust that “honest execution” is real. Where the engine is still thin (e.g. lease enforcement), **say so** (§11.6) — do not imply full auditability from partial signals.

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

- **Public integration / plugin SDK surface:** stability matters; breaking changes deserve an RFC-style decision, not drive-by commits.
- **Workspace internals:** may break when wrong — but **not** silently: canon + migration note + tests (see §17).
- **Ecosystem quality over node count:** one solid canonical integration per external service beats many half-finished duplicates.
- **Third-party nodes** are first-class in intent: same capabilities as first-party where the plugin model allows; **document** what is shipped vs planned.

### 7.1 Plugin packaging: `Cargo.toml`, `plugin.toml`, and `impl Plugin`

Nebula recognizes **two legitimate packaging patterns** — not “official vs hack.” Both use the same **Rust crate + `plugin.toml` marker + `impl Plugin`** story.

**Full plugin** — e.g. `nebula-plugin-slack/`: many actions, credentials, resources, locales.

**Micro-plugin** — e.g. `nebula-resource-slack/`: one or two registry entries.

**Principle:** **Plugin is the unit of registration, not the unit of size.** Same loader and respect for both shapes.

#### Three sources of truth (no drift)

Avoid **double declaration** — listing every action in TOML **and** in `fn actions()` is **spec theater** (§14): two sources that will diverge.


| Artifact                             | Responsibility                                                                                                                                                                                                                                                                   |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **`Cargo.toml`**                     | Rust **package** identity: `[package].name`, `version`, `authors`, `license`, `homepage`, `description`, and **`[dependencies]`** on other crates — **including other plugin crates**. This is the **dependency graph** the host already knows how to resolve.                   |
| **`plugin.toml`**                    | **Trust + compatibility boundary** — read **without compiling**: **SDK constraint**, optional stable **plugin id**, and (when used) **signing** over a **stable** manifest (see **Signing** below). **Do not** duplicate registry contents (actions/resources/credentials) here. |
| **`impl Plugin` + `PluginMetadata`** | **Runtime source of truth** for **what** gets registered (`actions()`, `resources()`, `credentials()`, locales) and for **display metadata** (`PluginMetadata`: human name, icon, categories, long description, etc.) **after** load.                                            |


**Pre-compile discovery** (registry, CLI list) uses **`Cargo.toml` + minimal `plugin.toml`** only. Full **`PluginMetadata`** is authoritative **once the plugin is loaded**; do not require a second copy of every field in TOML.

**Versioning:** `Cargo.toml` `[package].version` is the **crate** version — do **not** duplicate it in `plugin.toml`.

**`Cargo.toml` stays Rust-standard:** no Nebula-specific tables in `Cargo.toml` — Nebula-specific policy lives in **`plugin.toml`** + Rust code.

**The boundary “this is a Nebula plugin”:** a **`plugin.toml`** file exists at the crate root with at least:

```toml
[nebula]
sdk = "^0.8"   # semver constraint on nebula-api / plugin SDK — read by cargo-nebula / CLI without `cargo build`
```

**Optional `[plugin].id`** — set this **only** when the stable Nebula plugin id must **differ** from the Cargo package name (registry/UI **before** load):

```toml
[plugin]
id = "nebula-plugin-slack"   # if [package].name is e.g. "slack-plugin"
```

**If `id` is omitted**, the **effective plugin id** for discovery and compatibility is **`[package].name`** from `Cargo.toml` — hosts and pre-compile tooling **must** use that string (no other implicit default). Internal mapping to typed keys (e.g. `PluginKey`) must be **deterministic** and **documented** in loader/tooling; if the package name does not map cleanly, authors **must** set `id` explicitly. **Do not** silently derive a different id from Cargo without an explicit `[plugin].id`.

#### Signing: why `plugin.toml`, not `Cargo.toml`

**`Cargo.toml` mutates** whenever dependencies are added, bumped, or re-resolved (`cargo update`, new crates). Signing it would mean **signatures churn constantly** or cover irrelevant churn — a poor trust anchor.

**`plugin.toml` is intentionally stable:** it holds **identity-for-trust**, **SDK compatibility**, and (when enabled) **cryptographic attestation** — not the full registry. That is what you **sign**: the author attests “this **plugin identity** and **policy** are mine.” Same idea as **Android** signing **`AndroidManifest.xml`** (policy/identity), not every `.java` file; or treating a **lockfile** / manifest as the attested surface while sources ship beside it.

- **Canonical signed payload:** the **bytes of `plugin.toml`** (or a defined canonical serialization of it — tooling decides). **`impl Plugin` / `PluginMetadata` are not the signed blob** — they describe **content** and can change without invalidating publisher identity, as long as the **attested manifest** is unchanged or re-signed.

Illustrative **`[signing]`** shape (field names and algorithms are **tooling-defined** until frozen):

```toml
[nebula]
sdk = "^0.8"

[plugin]
id = "nebula-plugin-slack"

[signing]
publisher   = "vanya@example.com"
fingerprint = "sha256:abc123..."
signature   = "base64:..."
```

**Three layers (summary):**


| Layer             | Role                                                                                            |
| ----------------- | ----------------------------------------------------------------------------------------------- |
| **`plugin.toml`** | **Trust + compatibility** — SDK bound, optional id, **signature** (what the publisher attests). |
| **`Cargo.toml`**  | **Build graph** — what compiles; **not** the Nebula trust document.                             |
| **`impl Plugin`** | **Content** — what registers at runtime.                                                        |


#### Why not list `[[actions]]` in `plugin.toml`?

Flutter-style **pubspec** asset lists work because there is no second source. Here, **`impl Plugin`** already returns the registry — a parallel TOML table would **duplicate** it. **SDK constraint + Cargo metadata + Rust registry** keeps a **single** responsibility per layer.

#### Plugin dependency rule (cross-plugin types)

For **Rust** plugins, **another plugin’s** types are brought in only via **`Cargo.toml` `[dependencies]`** on the **provider plugin crate**. The engine loads / activates providers **before** dependents according to that **acyclic** graph (topological order).

- **Versioning & discoverability:** `cargo tree`, lockfiles, and `Cargo.toml` already say “A depends on B.”
- **Isolation:** an Action in crate A that references a Resource type from crate B **without** a Cargo dependency on B is **invalid** — fail at **activation** / compile time, not a silent global lookup.

If a future **non-Rust** host needs a manifest-only dependency list, that can be a **separate** extension — the canon for **Rust-native** plugins is **Cargo-first**.

#### Layout

Directories such as `actions/`, `credentials/`, `resources/`, `locales/` are **recommended**; only **`plugin.toml`** (minimal) + **`Cargo.toml`** are **required** at the canon level for the marker story above.

Illustrative (non-normative) layouts:

```text
nebula-plugin-slack/          # full plugin
  plugin.toml                 # required manifest
  actions/
    send_message.rs
    create_channel.rs
  credentials/
    oauth.rs
    bot_token.rs
  resources/
    http_client.rs
  locales/
    en.json
    ru.json

nebula-resource-slack/        # micro-plugin
  plugin.toml                 # same manifest contract
  resource.rs
  credential.rs
```

### 7.2 Engine upgrade and workflow compatibility

**Operators** need a clear story when the **engine** version changes — not only plugin authors.

- **Persisted workflow definitions** and **plugins** (binaries / SDK linkage) are **two compatibility surfaces**; breaking either belongs in **release notes** and migration guidance.
- **Patch and minor** releases **must** keep **forward-compatible** workflow JSON and documented **plugin SDK** boundaries unless the release **explicitly** announces a break.
- **Plugin binary compatibility:** Rust plugin crates are compiled artifacts tied to SDK/engine versions; upgrades may require recompilation against the target `nebula-api` / SDK version. Binary-stable ABI is an **FFI path** concern (e.g. stabby), not an implicit guarantee for native Rust plugin binaries.
- **Breaking** workflow schema, execution semantics, or public SDK types require **documented migration**, tests, and upgrade notes — not an assumption that existing installs “should work.”
- Do **not** claim “all v1 workflows run unchanged on v2” without a **published compatibility matrix** or equivalent — platform trust requires **honest** upgrade paths.

---

## 8. What Nebula is not

- **Not a low-code tool** — operators may compose graphs; **authors** target Rust (and future FFI), not replacement of typed integration work.
- **Not optimized for one-shot 50 ms scripts** — value shows up at **scale, duration, and integration depth**.
- **Not “most nodes wins”** — the metric is **SDK quality and reliability**, not inventory size.
- **Not a generic framework playground** or **trait zoo** optimized for elegance over usability and engine truth.
- **Not “JSON everywhere and hope for the best”** — interchange types are deliberate; product-level truth needs validation and boundaries.
- **Not advertising** retry, durability, resource lifecycle, or plugin trust models **before the engine actually owns them** end-to-end.

---

## 9. North star & success

**North star — integration author:** A Rust developer with no prior Nebula experience can open the integration SDK / traits (§3.5–§3.9), and ship a **working, tested** node for a new service in **a focused day** — without hand-rolling orchestration, credential plumbing, or concurrency bugs.

**North star — operator:** After **any** failed or stuck run, an operator can **explain what happened** — which step, what error, what durable state — using **logs, API, journal, and metrics alone**, without reading integration **source code** (aligned with §2 and §4.6).

**North star — trigger delivery:** Trigger-driven flows have an **explicit, testable delivery contract**: no silent event drop; delivery semantics are documented (typically **at-least-once**), and duplicate events are controlled via event identity + idempotency/dedup rules rather than wishful “exactly once” claims (see §11.3).

**Success sounds like (author):** *“Writing a Nebula node was the easiest integration I’ve ever written; it kept working under load and failure.”*

**Success sounds like (operator):** *“When something broke, I knew **where** it failed and **why** — not only that the run turned red.”*

**Success sounds like (trigger ops):** *“Incoming trigger events were either processed once or safely de-duplicated — never silently lost.”*

**Progress looks like:** engine behavior and **public contracts align**; vertical slices are **boringly reliable**; workflow validity **shifts left** into validation; docs get **shorter and truer**, not larger and wishful.

---

## 10. Golden path (product)

Nebula must **protect one coherent path** before multiplying half-supported options. In intent:

1. Author defines a workflow; definition is persisted and **validatable** (round-trip).
2. **Activate** the workflow where the product supports activation. Activation runs `nebula_workflow::validate_workflow` (or equivalent) and **rejects** invalid definitions with structured **RFC 9457** errors — it does not silently flip a flag. A standalone `/validate` endpoint is a **tool**, not a substitute: activation that enables a workflow **without** validation is a **§10 violation**.
3. Trigger or API starts execution.
4. Engine schedules **executable step semantics** only — triggers, resources, and steps remain **distinct concepts** in validation, not only in dispatch errors.
5. Execution state transitions are **visible and attributable** through `ExecutionRepo` with **version-checked CAS**; no handler invents an out-of-band lifecycle.
6. Failure, cancellation, retry, and timeout behavior match **documented** contracts — not folklore in traits. **Cancel** requests must be **durable and engine-consumable** (see §12.2), not “only the DB row changed.”
7. **Persistence story is explicit:** what is durable vs best-effort; what resume/replay may assume; what happens on checkpoint failure.
8. Operator can **inspect** what happened and what is trustworthy.

Anything that does not strengthen this path is secondary until the canon says otherwise.

---

## 11. Core product contracts (honesty)

These must stay **explicit in code and operator-facing docs**, not split across half-implemented types.

### 11.1 Execution authority

`nebula-execution` + `ExecutionRepo` are the **single source of truth** for execution state. Transitions use **optimistic CAS** against persisted `version`. There is no ephemeral “usually DB wins” mode: if persistence is unavailable, the operation **fails** — it does not silently mutate in-memory state.

### 11.2 Retry

Retry is a **runtime semantic** owned by the **engine** and **`nebula-resilience`** pipelines around **outbound** calls inside an action — not a decorative hint on a return type. The engine **does not** schedule re-execution of a failed node from an `ActionResult::Retry`-style return unless that path is wired with **persisted attempt accounting**. If such a variant exists but is not honored end-to-end, it is a **false capability** (remove it or implement it). Until durable per-attempt retry accounting exists, the canonical retry surface is the **resilience pipeline** an action uses internally.

**Status (per §11.6 vocabulary):**

| Surface | Status | Notes |
| --- | --- | --- |
| `nebula-resilience` pipeline inside an action (in-memory retry around outbound calls) | `implemented` | The **canonical** retry surface today. Author composes retry/timeout/circuit-breaker at the call site. |
| Engine-level node re-execution from `ActionResult::Retry` with persisted attempt accounting | `planned` | No persisted `attempts` row, no CAS-protected bump, no consumer wired through `ExecutionRepo`. Any return variant that implies it is a **false capability** under §4.5 — hide or delete until end-to-end. |
| Cross-restart retry of a checkpointed step | `best-effort` | Relies on checkpoint boundaries (§11.5); work since the last checkpoint may be replayed or lost. Not a per-attempt contract. |

Canon debt: until the `planned` row above moves to `implemented`, no public API, trait variant, or docs comment may describe engine-level retry as a current capability. Track this row as an **open invariant debt** — revisit whenever `ActionResult`, `ExecutionRepo`, or attempt accounting is touched.

### 11.3 Idempotency

**One** idempotency story: deterministic key shape **`{execution_id}:{node_id}:{attempt}`**, persisted in `idempotency_keys`, checked and marked through `ExecutionRepo` before the side effect. **Engine guarantee:** it will not double-dispatch a **marked** attempt. Whether the **external** system de-duplicates is the integration author’s contract with that system — document per node.

For **non-idempotent or risky side effects** (payments, writes without natural upsert, external one-shot operations), action handlers must guard execution with this idempotency path (or an equivalent documented key contract) before calling the remote system.

For **TriggerAction** sources, each inbound event should carry or derive a stable event identity (provider event id / cursor offset / hash) so at-least-once delivery can be made safe via dedup/idempotent handling; “no duplicates” is not a claim unless the source + runtime can prove it end-to-end.

### 11.4 Resource lifecycle

Resources are first-class because **acquisition** and **scope-bounded release** are **engine-owned**. The async release path is **best-effort on crash** — orphaned resources rely on the next process to drain via `DrainTimeoutPolicy` / `ReleaseQueue`. Operators must be told this; authors must not assume “release ran” without an explicit checkpoint.

For long-lived exclusive/external resources (locks, leased cloud instances), deployments need external TTL / dead-man strategy; Nebula v1 does not provide an external lease arbiter by itself.

### 11.5 Persistence & operators

Checkpointing is **policy-driven**, not “fsync every step.” The engine checkpoints at declared boundaries (workflow/action policy) and on workflow completion. Between checkpoints, progress is in-memory: process death can lose work since the last checkpoint.

Authors should place checkpoint boundaries before irreversible or expensive side effects; the engine does not guess those boundaries for you.


| Artifact                           | Status                                                                | Operator-visible truth                                                                                                                                                           |
| ---------------------------------- | --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `executions` row + state JSON      | **Durable** (CAS via `ExecutionRepo`)                                 | Source of truth                                                                                                                                                                  |
| `execution_journal` (append-only)  | **Durable**                                                           | Replayable history                                                                                                                                                               |
| `execution_control_queue` (outbox) | **Durable**                                                           | At-least-once dispatch + cancel signals (§12.2)                                                                                                                                  |
| `stateful_checkpoints`             | **Durable at checkpoint boundaries**; failure mode is **best-effort** | Checkpoint write failure may **log** and **not** abort execution; resume falls back to last successful checkpoint or journal; work since last checkpoint may be replayed or lost |
| `execution_leases` (schema)        | **Schema may exist before full enforcement**                          | If the engine does not consume leases yet, **say so** — do not imply lease safety                                                                                                |
| In-process `mpsc` / channels       | **Ephemeral**                                                         | Never authoritative truth                                                                                                                                                        |


If an operator cannot answer durability questions from this section plus code/docstrings, the product is not yet operationally honest.

Checkpoint / side-effect race is a real failure mode: if a side effect commits externally and the checkpoint write fails afterward, replay can re-enter that step. Protection is by design through idempotency keys (§11.3), not by pretending exactly-once.

### 11.6 Documentation truth

Docs must distinguish **implemented**, **best-effort**, **experimental**, and **planned** behavior. A short **guarantees** narrative (in `docs/` or README) should answer durability, validation, retry, resume, and current plugin trust — without collapsing future intent into today’s contract. **README drift** (advertising a removed backend, endpoint, or capability) is a **bug** — fix in the same PR as the code change.


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

- Follow `CLAUDE.md` dependency direction. **No upward dependencies** between layers.
- `crates/api` does not embed SQL drivers or storage schema knowledge beyond declared ports; **storage and orchestration details live in their crates**.

### 12.2 Execution: single semantic core, durable control plane

- **Authoritative execution state** lives in `nebula-execution` + `ExecutionRepo`. Handlers and API DTOs **do not invent a parallel lifecycle**, do not mutate state without going through **`ExecutionRepo::transition`** (CAS on `version`), and do not return **synthesized** timestamps or fake defaults for missing fields.
- **Every “run this” / “cancel this” signal must be durable and engine-consumable.** The contract is:
  1. The signal is written to **`execution_control_queue`** (outbox) **in the same logical operation** as the corresponding state transition. A handler that flips state to `cancelling` **without** enqueueing — or enqueues **without** transitioning — is broken.
  2. A dispatch worker drains the queue and forwards commands to a consumer that **the engine actually listens to**. Removing rows **before** the engine has acted is broken.
  3. There is **one** consumer wiring story per deployment mode, **documented in code**.
- **A demo handler that logs the command and discards it does not satisfy this invariant.** Examples and `simple_server.rs` must either wire a **real** engine consumer or be marked `// DEMO ONLY — does not honor cancel` so nothing mistakes them for the contract.
- **Batching outbox writes for throughput is valid only if per-transition atomicity is preserved.** Never batch as a workaround that breaks “state transition + control signal” integrity.
- Any second control channel (HTTP webhook, in-memory event) is **forbidden** unless this canon is updated with a **reconciliation** story.

### 12.3 Local path

- The **default developer experience** must allow: build, run tests, run core flows **without** Docker, Redis, or external brokers. **SQLite** is the default local storage; `sqlite::memory:` via `nebula_storage::test_support` is the reference in-process path.
- Optional production paths (**Postgres**, later Redis, etc.) are **additive**, not prerequisites for “hello world” or CI sanity. `examples/simple_server.rs` (and similar) must continue to start **without** external services unless explicitly documented as integration-only.

### 12.4 Errors and contracts

- Library crates: `thiserror`, not `anyhow`, in public library surfaces.
- API boundary: **RFC 9457 `problem+json`** (see `crates/api/src/errors.rs`). **No new ad-hoc `500`** for business-logic mistakes — map new failure modes into typed `ApiError` variants with an explicit status.
- `serde_json::Value` is allowed **where it is the deliberate interchange type**; new **stringly protocols** (magic field names without schema validation) require explicit review.

### 12.5 Secrets and auth

- No secrets in logs, error strings, or metrics labels. **`Zeroize` / `ZeroizeOnDrop`** on key material; redacted `Debug` on credential wrappers (`SecretToken`, etc.). Encryption at rest uses authenticated encryption with a KDF — do not bypass “for debugging.” Details: `crates/credential/README.md`.
- Every new `tracing::*!` that takes a credential or token argument must use **redacted** forms.

### 12.6 Isolation honesty

- In-process sandbox / capability checks: **correctness and least privilege for accidental misuse**, not a security boundary against malicious native code. Keep `crates/sandbox` doc comments aligned with this canon and `docs/` threat models.
- **Plugin IPC today:** sequential dispatch over a **JSON envelope** to a child process — that **is** the trust model; do not describe it as **sandboxed execution of untrusted native code**.
- **WASM / WASI is an explicit non-goal for plugin isolation.** The Rust plugin ecosystem integration authors actually need — `redis`, `sqlx` with native drivers, `rdkafka`, `tonic` with native TLS, any `*-sys` crate — does **not** compile to `wasm32-wasip2`, and where parts compile, the feature surface forces authors into host-polyfill folklore that violates the §3.5 promise ("Write Stripe logic; do not write credential rotation, connection management, or retry folklore"). Offering WASM as "the future sandbox" would be a §4.5 false capability and a §4.4 DX regression at the same time. **The real isolation roadmap is:** `ProcessSandbox` (already shipping) → full `PluginCapabilities` enforcement wired from `plugin.toml` through discovery (closes `nebula-sandbox/src/discovery.rs:117`) → `plugin.toml` signing verification in tooling (canon §7.1) → per-platform OS hardening in `os_sandbox` (seccomp-bpf / landlock on Linux, `sandbox_init` on macOS, `AppContainer` / job objects on Windows) → parallelism within `ProcessSandbox` for throughput (§4.1). Revisit WASM only if the Rust WASM ecosystem crosses a specific, documented capability threshold — not as aspiration, and never as docs drift in crate-level `lib.rs` or README.

### 12.7 No god files, no orphan modules

- A module that grows past a few hundred lines and mixes unrelated responsibilities is a **refactor**, not a feature — **split before adding**.
- A new file under `crates/*/src/services/`, `crates/storage/src/`, or similar must have an **obvious caller** in the same PR. Code that is **enqueued but never consumed** (or consumed but never produced) is an **integrity bug**, not a TODO.

---

## 13. The knife — demo scenario (must stay green)

This is the **minimum bar** for “we did not break the product direction.” Extend it over time; do not weaken it without a canon update.

**Scenario (current bar):**

1. **Define and persist** a workflow through the API — definition **round-trips**.
2. **Activate** the workflow. Activation runs validation and **rejects** invalid definitions with structured RFC 9457 errors — it does **not** silently flip a flag.
3. **Start an execution** (API or equivalent). The execution row exists with consistent `status`, monotonic `version`, and a real `started_at` (no synthetic zero, no placeholder `now()` where the field should be `None`).
4. **Observe** via GET — `finished_at` is `None` (not `0`) until terminal; `status` reflects the latest persisted value.
5. **Request cancellation** on a non-terminal execution:
  - the handler transitions through **`ExecutionRepo`** (CAS),
  - the **same logical operation** enqueues **`Cancel`** in `execution_control_queue`,
  - a dispatch consumer wired to the **real engine** observes the command and the engine’s cancel path runs,
  - the execution reaches a **terminal** `Cancelled` state without hand-waved stubs.
6. Under test configuration where orchestration is intentionally absent: control endpoints return **503** — never fake success and never an unparsable 500.

**Integration bar (same spirit as execution — must stay green as these paths exist):**

1. **Plugin load → registry:** a plugin loads; **Actions / Resources / Credentials** from `impl Plugin` appear in the catalog (or equivalent) **without** a second manifest that duplicates `fn actions()` / `fn resources()` / `fn credentials()` (§7.1).
2. **Credential refresh / rotation:** where rotation or refresh is implemented, it does **not** silently strand or corrupt **in-flight** executions that hold valid material — failure is **explicit** in status or errors if the system cannot reconcile.
3. **Resource lifecycle visibility:** acquire → use → **release** for Resource-backed steps is **attributable** in **durable journal** or an **operator-visible** trace (aligned with §11.4) — not only in ephemeral logs.
4. **Trigger delivery semantics:** for TriggerAction-backed starts, tests cover the declared delivery contract (**at-least-once** unless explicitly stronger): no silent drop, and duplicate delivery is handled via stable event identity + dedup/idempotency (aligned with §9 and §11.3).
5. **Non-idempotent side effects:** for ordinary Actions that can cause irreversible external effects (e.g. charge/refund/payout), integration tests prove **single-effect safety** under retry/restart/duplicate-dispatch pressure: idempotency key guard is applied before the side effect, and re-entry does **not** execute the external effect twice.

**What “done” means for a change touching execution / API / storage / plugins:**

- **Integration tests** exercise the path end-to-end, including **step 5** (engine-visible cancel), not only DB metadata.
- Changes to **plugin registration**, **credential refresh**, **resource release**, **trigger ingestion**, or **non-idempotent action execution** that affect §3.5 claims require **coverage** for steps **7–11** where those features are touched — or an explicit canon note that the bar is **narrowed** (not silent regression).
- No new dispatch path, queue, or in-memory channel without an explicit **§12.2** update.
- `simple_server.rs` (and similar) either **honors cancel end-to-end** or carries `// DEMO ONLY` naming exactly which steps are stubbed.

---

## 14. Anti-patterns — do not ship

- **Two truths:** execution state in DB says X, channel/queue says Y, with no formal reconciliation story. (See §12.2.)
- **Phantom types:** enum variants or trait methods the engine **rejects at runtime** — e.g. `ActionResult::Retry` with no persisted accounting. **Implement end-to-end or delete.**
- **Discard-and-log workers:** a dispatch loop that drains an outbox and “handles” commands with `tracing::info!` only — **not** a consumer; it is a leak.
- **Validation-as-a-side-tool:** workflow validation only at `/validate` while **activation skips** it.
- **Green tests, wrong product:** shortcuts that pass tests but violate §12 (e.g. `String` errors in new library crates, new `ExecutionControl` semantics that bypass storage).
- **Framework before product:** abstractions multiplying faster than invariants; types ahead of engine-owned semantics.
- **Trait surface faster than engine truth** — new public trait families or result variants without end-to-end behavior.
- **Runtime rejection instead of validation** for workflow shape where validation is feasible.
- **Best-effort persistence presented as durable truth** — or ambiguous ownership of “what happened.”
- **Docs that describe future intent as current contract** — or internal channels treated as durable infrastructure.
- **README drift:** advertising a backend, capability, or step the code no longer supports.
- **God files:** continuing to add unrelated logic to a file that already exceeds reasonable responsibility instead of splitting (module or crate) when boundaries are clear.
- **Orphan modules:** services, queues, or repos **produced but never consumed** (or vice versa). See §12.7.
- **Spec theater:** long `docs/` plans that contradict this file without a canon revision — **plans follow canon**, not the reverse. The same applies to **this** file: if the change is really about `nebula-resource` APIs, update **`crates/resource/README.md`**, not a three-page **§3.6** essay in `docs/PRODUCT_CANON.md`.

---

## 15. How other docs relate


| Document                                                                        | Role                                                                                                                                                                                                          |
| ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CLAUDE.md`                                                                     | Commands, formatting, layer diagram, agent workflow                                                                                                                                                           |
| `docs/PRODUCT_CANON.md` (this file)                                             | Product direction, competitive bets (§2.5), integration model (§3.5) + crate pointers (§3.6–§3.9) + cross-cutting (§3.10), OSS / plugin (§7.1–§7.2), pillars (incl. §4.6), contracts, invariants, knife (§13) |
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

1. Does this **strengthen the golden path** (§10)?
2. Does this **clarify or blur** the product contract (§11)?
3. Does the engine **actually honor** the behavior now? If **not**, the type, handler, or endpoint does **not** ship.
4. Does this **reduce or increase** contributor cognitive load?
5. Does this make local / self-hosted / future hosted stories **more coherent**, or only broader on paper?
6. Does this help operators **understand failures**?
7. Is this **foundational now**, or speculative future-proofing?
8. If we ship this, are we making a **real promise**?
9. Does this **align with the competitive bets** in §2.5 (typed durability vs soft ecosystem, checkpoint/local-first vs replay/compose-heavy, Rust contracts vs script glue) — or does it blur those lines without updating the canon?
10. Does this **preserve the §3.5 integration model** (orthogonal concepts, `*Metadata` + `ParameterCollection`, plugin wiring rules) **and** avoid **spec theater** — duplicating crate-level API detail in this file instead of updating `crates/*/README.md` — or **duplicate** cross-cutting concerns §3.10 already owns, without a deliberate canon update?
11. If this introduces a **queue, channel, or worker:** are **producer**, **consumer**, and **failure mode** all in **this PR** (or explicitly documented as out of scope with no orphan half)?

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