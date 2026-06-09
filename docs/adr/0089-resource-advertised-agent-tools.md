---
# budget-justified: ADR prose document — D4 reconciliation table, schema-shaped ToolDefinition, and a versioned roadmap table; one contiguous decision record, not decomposable code
id: 0089
title: resource-advertised-agent-tools
status: proposed
date: 2026-06-04
supersedes: []
amends:
  - docs/adr/0057-ai-agent-sdk.md
superseded_by: []
tags: [agent, tools, resource, mcp-shape, llm, capability, roadmap, m12, m14]
related:
  - docs/adr/0057-ai-agent-sdk.md
  - docs/adr/0081-m6-resource-credential-integration.md
  - docs/adr/0088-credential-subsystem-rewrite.md
  - docs/adr/0084-pre-expiry-credential-refresh-deferred.md
  - docs/ROADMAP.md  # M12 (resource finalization), M14 (1.0 surface freeze)
  - crates/resource/CLAUDE.md
  - crates/action/CLAUDE.md
---

# 0089. Resource-advertised agent tools (`ResourceTools`, MCP-shaped internal trait)

## Status note

This **amends ADR-0057** (AI agent SDK direction). ADR-0057 modelled agent
tools as standalone actions (`pub trait AgentTool: StatelessAction` registered
into a static `ToolRegistry` — *tools = actions*). This ADR **inverts the
primary tool source**: a `Resource` advertises a schema-described set of
callable operations (`impl ResourceTools for PostgresPool`), and an agent
discovers those tools from the `ResourceGuard`s it has acquired (*tools =
resources*). `AgentTool: StatelessAction` is **narrowed to a secondary
provider** for resource-independent tools; both feed **one** `ToolDefinition`
shape and **one** runtime tool registry. The `Llm` trait, `Memory`, the ReAct
loop, streaming `AgentEvent`s, multi-agent composition, and vector-store-as-
`Resource` from ADR-0057 are **unchanged**.

Three decisions are locked by the requester and bind this ADR:

- **D-lock-1** — "like MCP" means an **internal Rust trait abstraction** whose
  *shape* borrows the Model Context Protocol field inventory; it is **not** an
  MCP wire protocol. No network boundary is crossed by tool discovery or
  invocation in this ADR.
- **D-lock-2** — resource-advertised tools are the **primary** model; ADR-0057's
  action-as-tool model is revised to secondary.
- **D-lock-3** — the deliverable is a **version-ordered roadmap** (this ADR's
  §Roadmap), not a single-shot landing.

## Context

Charter §2 lists AI-agent orchestration as a target profile; ADR-0057 sketched
the `nebula-agent` SDK but left **any resource→tool exposure mechanism
unspecified**. A codebase audit (file:line-verified, 2026-06-04) found the
agent/tool surface is a metadata stub, while the resource-side seams the new
model needs already exist and are the right shape.

### What exists today (grounded)

- **`Resource` trait** (`crates/resource/src/resource.rs:249`): 4 associated
  types (`Config`/`Runtime`/`Lease`/`Error`), `key()`/`create()`, the per-slot
  rotation hooks `on_credential_refresh`/`on_credential_revoke(&self, slot,
  runtime)`, and `check`/`shutdown`/`destroy`. It uses **RPITIT, not
  `async_trait`** (`resource.rs:215` — avoids `Box<dyn Future>` per call). All
  hooks take **`&self`**: a `Resource` is an immutable descriptor, mutable state
  lives on `Runtime` via interior mutability. A `tools(&self, …)` accessor lands
  flush with this pattern.
- **`ResourceGuard<R>`** (`guard.rs:111`): a `#[must_use]` drop-releasing RAII
  lease; `Deref::Target = R::Lease` (`guard.rs:573`). Drop is fire-and-forget to
  the `ReleaseQueue`; `release(self)` is the explicit awaited checkpoint
  (PRODUCT_CANON §11.4). **A tool is valid only while the guard is live** — this
  is the structural safety lever (§Security, §Risks).
- **`SlotCell<S>`** (`slot.rs:57`): generation-stamped, lock-free
  (`ArcSwapOption` + `AtomicU64`); `load_versioned()` yields a torn-read-free
  `(value, generation)`. This is the rotation/TOCTOU primitive used at invoke
  time (§Security).
- **`AnyManagedResource`** (`registry.rs:77`): the **sealed type-erased**
  registry interface — the natural home for a `tools()` discovery hook returning
  `Option<&dyn ResourceTools>`.

### What is a stub

- **No `AgentAction` trait exists.** `ActionCategory::Agent`
  (`crates/action/src/metadata.rs:60`) is a metadata-only enum variant for
  UI/validator grouping; the runtime does not dispatch on it. `SupportPort`
  (`crates/action/src/port.rs:77`) lists `"tools"` only as a documented example
  port key (`port.rs:16,78`); its sole concrete appearance is a unit test
  (`port.rs:340`) — there is **no runtime wiring**, no tool enumeration, no
  catalog handed to any agent. The consumer side of this ADR is built, not
  extended.
- The action **authoring surface** is a declarative slot field —
  `#[resource(key="…")] field: ResourceGuard<R>`. The resolution primitive —
  today `ctx.acquire_resource_by_id::<R>(id)` / `resolve_credential_by_id::<C>`
  (`context.rs:741`/`:788`, `ActionContextExt`), **slated to be renamed
  `ctx.resource_by_key::<R>(key)` / `credential_by_key::<C>(key)`** to match the
  `*Key` metadata vocabulary (`ActionKey`/`ResourceKey`/`CredentialKey`,
  `metadata/base.rs:207`; the `id` param already becomes a `ResourceKey`
  internally, so `_by_id` is a holdover) — is **derive-emitted plumbing the
  author never writes** (`from_workflow_node.rs:71` →
  `macros/.../field_slots.rs:284`; its own doc: "Plugin authors normally do not
  call these methods directly"). Its `Pin<Box<dyn Future>>` + turbofish shape is
  the object-safety price of dispatching through `&dyn ActionContext`, justified
  *because* it stays behind the macro. A separate type-only sugar
  `ctx.resource::<R>()` already exists (`resource/ext.rs:122`) for the
  canonical/global single-instance case — **not** a substitute for `_by_key`
  (it cannot disambiguate two instances of one type; ADR-0081). The agent-tool
  surface (D3) inherits the declarative-field property — never a hand-called
  acquire. Input JSON Schema comes from `Input: HasSchema` (`#[derive(Schema)]`),
  rendered as `nebula_schema::ValidSchema`.

### ADR-0057 model being amended

ADR-0057 commits `trait AgentTool: StatelessAction` + `description() ->
ToolDescription`; an `Agent<L: Llm>` ReAct loop holding a **static,
pre-registered** `tools: ToolRegistry`; streaming; multi-agent via workflow;
vector-store-as-`Resource`. The tool source (actions) and the registry keying
are what this ADR revises; everything else stands.

## Decision

Resources advertise their callable operations through an internal
`ResourceTools` trait; agents discover and invoke those tools off live
`ResourceGuard`s; one `ToolDefinition` shape and one registry unify
resource-tools (primary) with action-tools (secondary).

### D1 — `ResourceTools` internal trait (sealed, `&self`, `#[async_trait]`)

The trait sits flush with the existing rotation-hook pattern — another `&self`
capability read off the immutable descriptor. Unlike `Resource` (RPITIT), the async
method here is erased with `#[async_trait]` (rationale below): `invoke` is a cold,
LLM-gated path **and** the trait must be `dyn`-safe for the heterogeneous tool
registry, so the hot-path rationale that drove `Resource`'s RPITIT does not apply.
It is **sealed** so methods can be added later without a semver major.

```rust
// crates/resource/src/tools.rs

/// A resource that advertises callable, schema-described operations to an agent.
///
/// Internal Rust trait (D-lock-1) — NOT an MCP wire protocol. The shape borrows
/// MCP's field inventory [1] but never crosses a network boundary.
///
/// `#[async_trait]` makes the trait `dyn`-safe directly (the boxed-future shape is
/// generated for us), so the heterogeneous tool registry holds `dyn ResourceTools`
/// with **no separate erased twin trait**.
#[async_trait]
pub trait ResourceTools: sealed::Sealed + Send + Sync {
    /// Catalog of operations this resource exposes, tenant-scoped: tools the
    /// caller's scope cannot use are not returned, so the LLM never plans with
    /// them (discovery-time gating — §Security). Sync — no I/O at enumeration.
    fn tools(&self, scope: &TenantScope) -> Vec<ToolDefinition>;

    /// Invoke one operation by name. `&self` borrow ties the call to the guard's
    /// lifetime (the tool cannot outlive the live resource). One `Box<dyn Future>`
    /// per call — negligible on this LLM-gated cold path (see D1 rationale).
    async fn invoke(
        &self,
        name: &str,
        args: serde_json::Value,
        cx: ToolInvokeCx<'_>,
    ) -> Result<ToolOutput, ToolError>;
}

mod sealed { pub trait Sealed {} }
```

**Why `#[async_trait]`, not RPITIT or `dynosaur`.** The registry holds tools from
differently-typed resources and dispatches by name at runtime, so `ResourceTools`
*must* be `dyn`-safe. Native `async fn` in traits is **not** `dyn`-compatible in
2026 — the `.box` operator that will eventually standardise call-site boxing is an
unfinished Rust project goal [R4] — so the async method needs erasing into
`Pin<Box<dyn Future>>` one way or another. The candidates collapse to the **same
runtime shape**: a hand-written erased twin, the `dynosaur` macro, or
`#[async_trait]`. **Decision: `#[async_trait]`** — the boring, ubiquitous, trusted
choice (thousands of dependents vs `dynosaur`'s few dozen); it makes the trait
`dyn`-safe *directly*, so the second `ErasedResourceTools` trait is **dropped
entirely**; and the one heap alloc per call is irrelevant on a path already gated
by an LLM round-trip. This is a deliberate, **local** exception to the crate's
RPITIT habit — that habit exists to avoid per-acquire allocation on
`Resource::create`'s **hot** path, a rationale absent on this cold path. Discovery
returns `&dyn ResourceTools` straight off `AnyManagedResource` (alongside the
existing sealed registry erasure at `registry.rs:77`); no parallel erased trait,
and the only new dependency is the well-trodden `async-trait` macro itself.

### D2 — `ToolDefinition` shape (MCP field lessons, schema reuse)

```rust
#[derive(Clone, Debug)]
#[non_exhaustive]                       // additive-evolution safety (§Roadmap, R5)
pub struct ToolDefinition {
    /// Stable machine routing key — valid in code, stable across versions.
    /// NOT the display label [1].
    pub name: Cow<'static, str>,
    /// Optional LLM-visible label; verbose, changeable, no trust weight.
    pub title: Option<Cow<'static, str>>,
    pub description: Cow<'static, str>,
    /// Input contract reusing the validated-schema type `nebula-schema` already
    /// exposes for action inputs — one schema vocabulary, no drift, no new
    /// schema dependency.
    pub input_schema: nebula_schema::ValidSchema,
    pub output_schema: Option<nebula_schema::ValidSchema>,  // MCP 2025-06-18 [4]
    pub annotations: ToolAnnotations,
}

/// MCP annotation vocabulary [1][8] — but in-process, with a trusted
/// implementor, these are ENFORCEABLE contracts (§Security), not the untrusted
/// hints MCP must tolerate across the wire.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct ToolAnnotations {
    // ── MCP-derived safety hints (enforced at the invoke gate, §Security) ──
    pub read_only: bool,
    pub destructive: bool,     // default false ≠ safe; gated at invoke (§Security)
    pub idempotent: bool,
    pub open_world: bool,      // trust-boundary marker on tool output
    // ── Nebula orchestration/planning hints (agent-loop scheduling, [R6]) ──
    pub parallel_safe: bool,       // may run concurrently with sibling tool calls
    pub cacheable: bool,           // result is a pure fn of args (consumer cache hint)
    pub cost_class: CostClass,     // planner budget hint
    pub latency_class: LatencyClass, // planner scheduling hint
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CostClass { #[default] Free, Cheap, Metered, Expensive }

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LatencyClass { #[default] Fast, Instant, Slow, Streaming }
```

The `name`/`title` split is the non-obvious MCP lesson worth keeping: `name` is
the stable routing key, `title` is the free-to-change LLM label. `input_schema`
reuses `nebula_schema::ValidSchema` — the validated-schema type
`HasSchema::schema()` already produces for action inputs — so tool schemas and
action schemas share one source of truth.

**`ToolOutput` — MCP-shaped dual channel with a *typed* `ResourceLink`.** MCP
2025-06-18 split a tool result into a human/LLM-readable `content[]` array
(text/image/audio/`resource_link`/embedded resource) **plus** a `structuredContent`
object validated against `output_schema` [1][R7]. Mirror that shape so wire-MCP
export (1.1+) is pure serialisation, never a re-model:

```rust
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ToolOutput {
    /// LLM-facing blocks (untrusted DATA, §Security). Empty ⇒ structured-only.
    pub content: Vec<ContentBlock>,
    /// Machine-readable payload, validated against `ToolDefinition::output_schema`.
    pub structured: Option<serde_json::Value>,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ContentBlock {
    Text(Cow<'static, str>),
    Json(serde_json::Value),
    /// **Lazy payload as a first-class resource handle — the D-lock-1 dividend.**
    /// MCP returns `resource_link` as an opaque URI string [R7]; in-process we
    /// already have typed resources, so we hand back the *typed key* + a size
    /// hint. The agent re-acquires it through the normal `ResourceGuard` path
    /// (scope + rotation enforced) instead of inlining a 10 MB blob into context.
    /// Wire-MCP export simply renders `key` → URI.
    ResourceLink { key: ResourceKey, mime: Option<Cow<'static, str>>, bytes_hint: Option<u64> },
    /// `data_ref` is a lease-able transient resource (scope/rotation governed),
    /// NOT inline bytes — the producer registers the blob as a resource. For
    /// small (<~8 KB) inline payloads prefer `Json`/`Text`; use these for
    /// governed binary artifacts only.
    Image { mime: Cow<'static, str>, data_ref: ResourceKey },
    Audio { mime: Cow<'static, str>, data_ref: ResourceKey },
}
```

`ResourceLink` is the structural reason **not** to speak wire-MCP internally
(D-lock-1, §Alternatives): a tool that produces a large or sensitive artifact
returns a lease-able key the engine already governs — lazy, scope-checked,
rotation-aware — whereas wire-MCP can only offer an unauthenticated URI the client
must re-fetch. Lazy-payload-as-typed-handle is unrepresentable across the wire and
free in-process.

**`ToolError` — a dedicated taxonomy (locked, §Open decisions).** A new error in
`nebula-resource`, not a reuse of `ActionError`, so the agent loop's retry /
self-correct logic stays clean; `From<ToolError> for ActionError` bridges at the
agent seam:

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ToolError {
    #[error("unknown tool: {0}")]           UnknownTool(String),
    #[error("invalid arguments: {0}")]      InvalidArgs(String),   // schema-validation failure
    #[error("not authorised for scope")]    Unauthorized,
    #[error("credential rotated mid-call")] CredentialRotated,     // TOCTOU close (§D5)
    #[error("resource unavailable")]        Unavailable,           // guard dropped / generation gone
    /// Opaque resource-specific failure. Contract: new *typed* variants may be
    /// added (sealed `#[non_exhaustive]`) before a case falls through to here.
    #[error(transparent)]                   Other(Box<dyn std::error::Error + Send + Sync>),
}

impl ToolError {
    /// The agent loop self-corrects on retryable errors instead of aborting.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::CredentialRotated | Self::Unavailable)
    }
}
```

### D3 — discovery + dispatch off the live `ResourceGuard`

**Authoring surface (declarative — no `resource_by_key`, no manual
registry).** An `AgentAction` declares its tool-providing resources exactly the
way any action declares a resource: slot fields. The agent derive folds every
`#[resource]` field whose type `impl ResourceTools` into one scope-filtered
`ToolRegistry`. The author wires nothing — same declarative contract as
`#[resource]`/`#[credential]` resolution (ADR-0081/0088 §D5):

```rust
#[derive(Action)]                                  // agent variant
#[action(key = "support.agent")]
struct SupportAgent {
    #[resource(key = "pg")]   db:   ResourceGuard<PgPool>,      // advertises pg.query / pg.execute
    #[resource(key = "http")] http: ResourceGuard<HttpClient>, // advertises http.get / http.post
}
// The derive emits the tool catalog (union of every ResourceTools field,
// scope-filtered) and the dispatch back to the owning guard. No
// `resource_by_key`, no hand-built ToolRegistry, no per-tool glue.
```

**Engine-internal mechanism.** Beneath that derive, the drop-releasing guard is
the workbench: the `ToolDefinition`s (cloneable, `Send + Sync`) escape to the LLM
layer; the **live handle never does**. Dispatch routes back through the guard,
keeping the borrow in scope for the whole call. Authors never write the loop
below — it is the generated / engine path:

```rust
// in nebula-agent (NOT in nebula-resource — see D6); engine-internal, not authored
async fn run_tool_loop<R: Resource + ResourceTools>(
    guard: &ResourceGuard<R>,            // borrow keeps the resource live for the call
    scope: &TenantScope,
    llm: &impl Llm,
) -> Result<AgentOutput, AgentError> {
    let catalog = guard.resource_tools().tools(scope);    // scope-filtered at source
    loop {
        match llm.step(&catalog).await? {
            LlmStep::Final(answer) => return Ok(answer.into()),
            LlmStep::Call { name, args } => {
                // re-check liveness + scope at invoke time (TOCTOU close, §Security)
                let cx = ToolInvokeCx::attenuate(guard, scope)?;  // Err if generation advanced
                let out = guard.resource_tools().invoke(&name, args, cx).await?;
                llm.observe_tool_result(&name, out);              // output is DATA, not instruction
            }
        }
    }
}
```

The catalog is re-fetchable after each call (`&self`, cheap), leaving room for
state-dependent tool sets later without committing to push notifications now.

A project-local `#[resource_tools]` proc-macro (mirroring the slot-accessor
emission in `#[derive(Resource)]`, `crates/resource/macros/src/field_slots.rs`)
generates both the `tools()` slice and the `invoke` dispatch table from
per-method `#[tool(description, read_only, …)]` attributes — **deferred** so the
hand-written trait stabilizes the contract before the macro freezes around it
(§Roadmap).

**Author-side DX (the producer half).** What a resource author actually writes —
the 1.0 macro form is annotated methods; the v0 hand-written form is the same
logic the macro will emit:

```rust
// 1.0 — macro form: the whole authoring surface is annotated methods.
#[resource_tools]
impl PgPool {
    /// Run a read-only SQL query and return rows as JSON.
    #[tool(read_only, idempotent, parallel_safe, cacheable)]
    async fn query(&self, sql: String, params: Vec<Value>) -> Result<Rows, ToolError> { /* … */ }

    /// Execute a writing statement.
    #[tool(destructive)]                    // → invoke-time HITL / allowlist gate (§D5)
    async fn execute(&self, sql: String) -> Result<u64, ToolError> { /* … */ }
}
// Generated: `impl ResourceTools for PgPool` — `tools(scope)` (input schema from
// arg types via `#[derive(Schema)]`, annotations from the attrs) + the `invoke`
// name→method table. The macro adapts each method's return type into `ToolOutput`
// (e.g. `Rows → ToolOutput::structured(json)`); the author returns domain types
// and never writes a `ToolDefinition` or a match arm.
```

This puts Nebula's authoring bar at or below the Rust agent-framework field, with
a property none of them give: the tool is **lifecycle/scope/rotation-governed by
construction** because it is a method on a *leased resource*, not a free function.

| Framework | Author writes | Schema source | Resource-lifetime safety |
|---|---|---|---|
| rmcp `#[tool_router]` [R2] | annotated methods on a server | schemars | none (server owns state) |
| rig `Tool` / `rig_tool` [R5] | trait impl or fn + derive | schemars | manual |
| swiftide `Tool` [R5] | trait impl or attr macro | derived `ToolSpec` | manual |
| **Nebula `#[resource_tools]`** | annotated methods on a `Resource` | `nebula-schema` (shared w/ actions) | **structural — guard borrow + `SlotCell` generation** |

### D4 — reconciliation with ADR-0057 (what is revised)

ADR-0057 and resource-tools are the **inverted discovery model**: 0057 = tools
authored as standalone actions pre-registered in a static registry;
resource-tools = tools as methods on an *acquired resource instance*, discovered
per guard. D-lock-2 makes resource-tools primary.

| ADR-0057 claim | Revision |
|---|---|
| `trait AgentTool: StatelessAction` is **the** tool source | **Narrowed to secondary** — the "standalone action tool" path for tools with no long-lived resource dependency (e.g. `search_web`). No longer the primary discovery path. |
| `tools: ToolRegistry` is a closed set fixed at `Agent` construction | **Multi-source + late-bound.** The registry is assembled per agent activation as the **union of two providers**: (1) primary — `ResourceTools` discovered from acquired `ResourceGuard`s; (2) secondary — static `AgentTool` actions. Keyed by `ToolId { source, name }`. |
| `ToolDescription` shape unspecified | **Unify on one `ToolDefinition`** (D2). `AgentTool::description()` becomes a projection into `ToolDefinition`. Non-breaking — 0057 never pinned the shape. |
| vector-store-as-`Resource` **plus** a separate `rag.retrieve` action | **Clarified:** the resource *is* the tool source. `PineconeStore: ResourceTools` exposes `retrieve`/`upsert`/`delete`; the standalone `rag.retrieve` action is optional scaffolding for non-agent workflows, not a second path the LLM sees — removes the "two ways to call the same thing" ambiguity. |

**Decision line:** resource-advertised tools are the **primary** agent-tool
source; `AgentTool: StatelessAction` is retained as a **secondary** provider; one
`ToolDefinition` and one scope-filtered `ToolRegistry` (union of both providers)
underlie both. ADR-0057's `Llm`/`Memory`/loop primitives stand unchanged. This is
one unified registry with two providers — not two parallel systems.

### D5 — security & multi-tenant scoping

D-lock-1 (internal trait, not wire) is itself the headline security win: tool
descriptions **never cross a network trust boundary before reaching the LLM**,
neutralising the tool-poisoning class (CVE-2025-54136/54135) by construction
[9][12]. The residual surface migrates inward (dynamically-loaded plugin
resources) and is handled as Definition-of-Done, not follow-up:

- **Tool acquired under a tenant scope.** `tools(scope)` filters at discovery
  (Decision-Gateway pattern [6]); tools the scope cannot use are never returned,
  so the LLM cannot plan with them. The `ResourceTools` impl consults the
  `nebula-tenancy` scope (it does not re-implement scoping).
- **Confused-deputy avoidance via attenuation.** `invoke` receives
  `ToolInvokeCx` — an attenuated view scoped to the one operation — **never the
  raw `ResourceGuard`**. Capability-security "pass attenuated capabilities, never
  raw ones" [3], expressed in Rust ownership: a scoped borrow dropped after the
  call.
- **Credential `SlotCell` + rotation TOCTOU.** `ToolInvokeCx::attenuate` records
  the `SlotCell` generation observed at discovery; at invoke time it re-reads
  `load_versioned()`. If the generation advanced (rotation/revoke), invoke fails
  `ToolError::CredentialRotated` rather than acting on a stale credential view —
  closing the plan-vs-execute window [6].
- **Destructive-tool gating, not hints.** `ToolAnnotations` are advisory to LLM
  planning but **enforced** at the invoke gate: `destructive`/`open_world` tools
  require a workflow human-in-the-loop step or an explicit `allow_destructive`
  execution flag (OWASP LLM08 excessive agency [9]). Default-deny destructive.
- **AuthZ at both ends.** Discovery filter + per-invocation re-check — scope can
  narrow between plan and execute (guard drop/reacquire).
- **Eventbus audit + observability triple.** Every invoke emits a `ToolInvoked`
  event via `nebula-eventbus` (never a direct sibling import): tool name,
  `ResourceKey`, `TenantScope`, input-schema **hash** (not raw args — they may
  carry secrets), outcome, credential generation. Each `ResourceTools` impl ships
  the Nebula DoD triple: a typed `ToolError` variant + a `tracing` span around
  invoke + an invariant check ("generation matches discovery").
- **Tool output is data, not instructions.** The loop treats every `ToolOutput`
  as untrusted before it re-enters LLM context (tool-chaining attack [4]);
  `open_world: true` marks the boundary.

### D6 — crate / layer allocation

| Layer | Crate | Tool responsibility |
|---|---|---|
| Business | **`nebula-resource`** | `ResourceTools` (`#[async_trait]`, `dyn`-safe) + `ToolDefinition`/`ToolAnnotations` + `ToolOutput`/`ContentBlock` + `ToolError` + `ToolInvokeCx` + a `tools()` accessor on `AnyManagedResource` returning `&dyn ResourceTools`. Lifecycle-adjacent capability only. |
| Business | `nebula-action` | `AgentTool` secondary projection into `ToolDefinition` (no new tool model). |
| Business-or-above | **`nebula-agent`** (per ADR-0057) | `ToolRegistry` (the two-provider union), `Agent<L: Llm>` loop, `AgentAction`, `Llm`/`Memory`. **Must depend downward** on `nebula-resource`/`nebula-action`; it cannot live below them. Exact tier (Business vs API/Public) is an open fork (§Open decisions), deferred to the `nebula-agent` crate's own ADR. |

The registry and the agent loop **must not** live in `nebula-resource` — that
crate is the lifecycle wrapper only (its CLAUDE.md non-goals), and hosting agent
orchestration would invert the dependency and erode the boundary. Cross-crate
communication uses `nebula-eventbus`, not sibling imports.

### D7 — lifecycle hooks + runtime tool toggling (1.0, after the trait freezes)

Two capabilities proven by the Rust agent-framework field [R5] but **deferred to
1.0** so the v0 contract stabilises first. Both land as **defaulted** methods on
the sealed trait, so they are semver-additive — no break for v0 implementors:

- **Invoke lifecycle hooks** — default-`{}` `before_invoke(&self, name, &mut args,
  cx)` and `after_invoke(&self, name, &mut ToolOutput, cx)`. They are the engine
  seam for output redaction, PII scrubbing, argument coercion, and the
  observability span — the swiftide "hooks may modify tool output" pattern [R5],
  here as trusted in-process code, not untrusted plugin callbacks.
- **Runtime enable/disable** — `set_enabled(&self, name, bool)` so a scope or a
  circuit-breaker can drop a tool from discovery without rebuilding the resource
  (rmcp shipped exactly this, PR #809 [R8]). Pairs with the 1.1 `listChanged`
  line: toggling re-emits the catalog.

### Consumer-side controls (owned by the `nebula-agent` ADR, not this one)

The 2026 tool-use literature [R6] converges on five agent-loop controls that are
emphatically **not** `ResourceTools` contract surface — they belong to the
`nebula-agent` loop. Listed only so the boundary is explicit:

- **Output budget + truncation** — cap tool-output tokens before re-entry; large
  payloads are already `ContentBlock::ResourceLink`, not inlined (D2).
- **Max tool turns** — a hard ReAct step ceiling to stop runaway fan-out
  (8–25 typical [R6]).
- **HITL gate** — `destructive`/`open_world` tools run a propose → approve →
  execute flow with a scoped, short-lived credential, not a standing grant [R6];
  the credential `SlotCell` TOCTOU check (§D5) is the execute-time half.
- **Untrusted-data lane** — every `ContentBlock` is data, never instruction
  (tool-chaining injection [4][R6]); `open_world` marks the boundary.
- **Result caching** — semantic/prefix cache keyed on `(tool, args)`, invalidated
  when the credential generation advances or `cacheable=false` [R6].

This ADR fixes only the *contract* that makes these implementable (annotations,
typed output, generation stamps); the *policy* is the agent crate's.

## Roadmap (versioned — the deliverable, D-lock-3)

Slotted onto the M0–M14 scaffold. `nebula-resource` is `frontier`; **M12.4
bind-population gates `stable`** and is the hard prerequisite — resource-tools
**must not jump that queue**. The trait is **not on the 1.0 critical path**; it
lands in the M12.4→M14 interval or defers cleanly to 1.1. Versioning discipline
(R5): seal the trait early, `&self` from day one, `#[non_exhaustive]` on every
tool/annotation struct, `#[async_trait]` for the `dyn`-safe tool trait (RPITIT
stays on `Resource`'s hot path).

| Version | What lands | Why now | Depends on | Risk | ADR action |
|---|---|---|---|---|---|
| **v0.x — experimental** | `ResourceTools` (sealed, `&self`, `#[async_trait]`, `dyn`-safe); `ToolDefinition`/`ToolAnnotations` (`#[non_exhaustive]`); typed `ToolError`; a `tools()` accessor on `AnyManagedResource` returning `&dyn ResourceTools`; **discovery-only** proven on one read-only resource (Postgres `pg.query`); the `ToolOutput`/`ContentBlock` envelope with the typed `ResourceLink` and the `parallel_safe` annotation (D2). `nebula-agent` crate created; minimal loop fetches a catalog off a guard. | Stabilise the contract before the macro and before any consumer; read-only first removes the destructive-gating surface from v0. | M12.4 bind-population; `nebula-schema`; `async-trait` | Trait churn — bounded by sealing + `#[non_exhaustive]`. AFIT-in-`dyn` gap — solved by `#[async_trait]`. | This ADR (amends 0057). |
| **v0.x+1 — security hardening** | `ToolInvokeCx` attenuation; credential-generation TOCTOU check; tenant-scope discovery filter; `ToolInvoked` audit + observability triple; destructive annotation + invoke-time allowlist gate; secondary `AgentTool` provider folded into one `ToolRegistry`; `cacheable`/`cost_class`/`latency_class` planning annotations (D2). | Security is DoD — a writable tool surface cannot reach an LLM without attenuation + audit + gating. | v0.x; `nebula-tenancy`; `nebula-eventbus`. | Confused-deputy / TOCTOU if attenuation is incomplete. | Amend (gating + audit invariants). |
| **1.0 — hardened freeze** | Freeze `ResourceTools` + `ToolDefinition` in the SDK / plugin-sdk contract; `#[resource_tools]` derive macro; exercise `output_schema`; add `fn resource_capabilities() -> ResourceCapabilities` (default `NONE`, additive on the sealed trait); D7 lifecycle hooks (`before_invoke`/`after_invoke`) + `set_enabled`. | Freeze the surface in the 1.0 contract; the macro lowers author cost once the contract is proven. | M12 + M13; `MATURITY.md` stable surface (M14.1). | Macro freezing an unproven contract — mitigated by shipping hand-written v0 first. | Macro design note; capabilities flag additive. |
| **1.1+ — deferred (NOT in 1.0)** | Dynamic tool sets (`listChanged` via eventbus); streaming tool output; wire-MCP **export** to external clients (separate `nebula-mcp-bridge` crate, `tools()` → MCP JSON); elicitation. Wire-export is now structurally trivial — `ToolOutput`/`ContentBlock` already mirror the MCP result shape (D2), so the bridge is serialisation, not a re-model. | MCP client `listChanged` support is sparse (2026 target = static lists); streaming waits on AFIT-in-`dyn` ecosystem stabilisation; wire export must stay out of the internal crate. | 1.0 contract stable; a consumer for each. | Premature dynamism / boundary erosion. | Wire-bridge = its own ADR + its own crate. |

**Other `nebula-resource` improvements surfaced alongside (not blocked by, and
not blocking, tools):** the M12.4 bind-population resolver (the real `stable`
gate); M12.4.2 engine frontier-loop per-branch resource cleanup; and the 1.1
resource features already on the books — `InfraProvider` (resource-on-resource),
`ConnectionAware` disconnect detection, `ResourceGroup` multi-resource
transactions, `Authenticate<C>`. Tools must not absorb these.

**1.0 must NOT include:** dynamic/`listChanged` tools, streaming tool output,
wire-MCP export, elicitation — all deferred to 1.1+ for the reasons above.

## Open decisions / forks

- **`tools()` dispatch:** hand-written `match` (v0, contract-stabilising) vs the
  `#[resource_tools]` macro dispatch table (1.0). Recommendation: hand-written
  first, macro after freeze — timing is the requester's.
- **Static `tools()` vs async discovery:** **Locked — sync** `fn tools(&self,
  scope) -> Vec<…>` (cheap, re-callable); `async fn discover()` is deferred to the
  1.1 `listChanged` line and only if enumeration ever does I/O.
- **`ToolError` — new type vs reuse `ActionError`:** a dedicated `ToolError`
  (`is_retryable()`, `CredentialRotated`, `UnknownTool`, `Unauthorized`) keeps
  the agent loop's retry/self-correct logic clean and avoids coupling
  `nebula-resource` to the action error taxonomy. **Locked — new `ToolError` in
  `nebula-resource`** with `From<ToolError> for ActionError` at the agent seam.
- **`nebula-agent` tier:** Business vs API/Public. Constraint: it depends
  downward on `nebula-resource`/`nebula-action`, so it cannot sit below Business.
  Deferred to the `nebula-agent` crate's own ADR.
- **Agent declaration surface (DX):** a new `#[derive(AgentAction)]` vs an agent
  *variant* of the existing `#[derive(Action)]` that auto-folds every
  `#[resource]` field implementing `ResourceTools` into the catalog. Either way
  the bar is fixed: the author declares slot fields and gets a tool catalog — they
  never call `resource_by_key` (the renamed `acquire_resource_by_id`) or assemble
  a `ToolRegistry` by hand (that primitive stays derive-only plumbing, per
  Context). **Locked — extend `#[derive(Action)]`** (agent variant) so resource /
  credential / tool resolution share one emitter; a separate
  `#[derive(AgentAction)]` is rejected to avoid a second resolution path.
- **`title` field — keep or drop?** R1 argued a minimal `{name, description,
  input_schema}`; R2/R5 argued to carry `title`/`output_schema`/`annotations`
  from the start behind `#[non_exhaustive]`. Resolved for R2/R5 — the name/title
  routing distinction and the destructive-annotation gate are hard to retrofit;
  `#[non_exhaustive]` makes optional fields free.
- **`ResourceCapabilities` granularity:** bitflags vs `#[non_exhaustive]`
  presence-marker struct. Defer the *shape* to 1.0; the *presence* of the method
  is additive on a sealed trait.

## Alternatives considered

- **Keep ADR-0057's action-as-tool as primary** (tools authored as standalone
  actions): rejected by D-lock-2. It cannot express a tool bound to an *acquired
  resource instance* (a live DB pool, an authenticated browser session) without
  smuggling the resource handle through global state — the exact confused-deputy
  / lifetime hazard D3/D5 close structurally.
- **Speak the MCP wire protocol internally** (resources as in-process MCP
  servers): rejected by D-lock-1. Serialising every tool call to JSON-RPC across
  an in-process boundary buys nothing, re-opens the tool-poisoning trust surface,
  and pins a wire schema version. Wire-MCP **export** is kept as a 1.1 bridge
  crate for *external* clients only.
- **One trait method per tool** (`trait PgTools { async fn query(...); }`):
  rejected — not enumerable or dynamically dispatchable by an LLM that picks a
  tool by name at runtime; `tools()` + `invoke(name, …)` is the rmcp/`ToolRouter`
  shape that supports discovery.
- **Host the `ToolRegistry`/agent loop in `nebula-resource` "for convenience":**
  rejected (D6) — inverts the dependency and erodes the lifecycle-only boundary.
- **Blanket `#[async_trait]` across `nebula-resource`:** rejected. Every RPITIT
  method in the crate (`Resource::create`, rotation hooks, `Bounded`
  acquire/release, `Pooled` recycle, the `ext.rs` accessors) is on the **hot**
  pool/acquire path, where the per-call `Box<dyn Future>` is exactly what RPITIT
  exists to avoid (`resource.rs:215`). Scoping rule: **RPITIT on the hot lifecycle
  path; `#[async_trait]` only when a method is *both* cold *and* must be
  `dyn`-safe** — which is one method, `ResourceTools::invoke` (D1). That choice is
  the exception that proves the rule, not a precedent to generalise crate-wide.

## Scope / non-goals

- **Out (1.1+):** dynamic/`listChanged` tools, streaming tool output, wire-MCP
  export, elicitation.
- **Out:** the LLM provider implementations and the `Agent` loop internals — they
  remain ADR-0057's scope (`nebula-agent` + per-provider contrib crates).
- **Preserved (not redesigned):** resource `SlotCell`/hooks/fan-out
  (ADR-0067/0081), the credential subsystem (ADR-0088), storage spec-16 ports
  (ADR-0072). Resource-tools is a *consumer* of already-resolved credential
  guards; it never resolves or persists secrets.
- **Sequencing constraint:** resource-tools lands **after** M12.4
  bind-population; it does not gate, and is not gated by, the 1.0 critical path
  beyond that.

## Consequences

- Tools become a first-class property of resources: a Postgres pool, an
  authenticated HTTP client, or a vector store advertises its operations once and
  any agent can use them — with the resource's lifecycle, scope, and credential
  rotation enforced by construction.
- ADR-0057 gains a primary tool source and a unified `ToolDefinition`; its loop
  and provider model are untouched, so the amendment is additive for consumers
  already targeting 0057.
- Tool descriptions never cross a network trust boundary before the LLM — the
  tool-poisoning class is closed by construction; the residual surface is
  attenuation/TOCTOU/destructive-gating, all DoD in v0.x+1.
- The lifetime-leak failure mode that plagues Python agent frameworks
  (tool invoked after its session closed) is unrepresentable: `invoke(&self, …)`
  borrows the live guard and only the schema (`ToolDefinition`) escapes.
- New surface area is bounded and sealed; `#[non_exhaustive]` + sealing + defaulted
  methods on the sealed trait keep the v0→1.0→1.1 evolution semver-additive.

## References

> Citation convention: bracketed `[Rn]` denotes an **external source** (see
> Research evidence below); a bare `Rn` in prose (e.g. "R2/R5 argued") denotes an
> internal **reviewer round**, not a citation.

- ADR-0057 — AI agent SDK direction (amended: tool source inverted to
  resource-primary; loop/Llm/streaming preserved).
- ADR-0081 — M6 resource & credential integration (resource contract,
  `ResourceGuard`, slot fields — consumed unchanged).
- ADR-0088 — credential subsystem rewrite (`CredentialGuard`/`SlotCell` semantics
  this ADR reads at invoke time).
- ADR-0084 — pre-expiry refresh deferred to 1.1 (reactive-only boundary the audit
  trail relies on).
- `docs/ROADMAP.md` — M12 (resource finalization, M12.4 bind-population gate),
  M14 (1.0 surface freeze).
- `crates/resource/CLAUDE.md` / `crates/action/CLAUDE.md` — crate non-goals and
  the `&self`/RPITIT/`SlotCell` conventions this design extends.

### External sources (research, 2026-06-04)

- [1] MCP server tools spec — https://modelcontextprotocol.io/specification/2025-06-18/server/tools
- [3] CSA — AI-agent confused-deputy / prompt-injection — https://labs.cloudsecurityalliance.org/research/csa-research-note-ai-agent-confused-deputy-prompt-injection/
- [4] MCP ambient authority / tool-chaining — https://tianpan.co/blog/2026-05-07-mcp-ambient-authority-tool-chaining
- [6] Decision Gateway authorization pattern — https://medium.com/advisor360-com/designing-authorization-for-production-ai-agents-the-decision-gateway-pattern-59582093ccb8
- [8] MCP tool annotations — https://blog.modelcontextprotocol.io/posts/2026-03-16-tool-annotations/
- [9] OWASP Top-10 for LLM Applications 2025 — https://dev.to/foxgem/overview-owasp-top-10-for-llm-applications-2025-a-comprehensive-guide-8pk
- [12] MCP tool poisoning / gateway defense — https://www.truefoundry.com/blog/blog-mcp-tool-poisoning-gateway-defense
- [R2] rmcp (official Rust MCP SDK) tool-router idiom — https://docs.rs/rmcp/latest/rmcp/ ; `#[tool_router]`/`#[tool]` macros — https://docs.rs/rmcp-macros/latest/rmcp_macros/attr.tool_router.html ; sealed traits — https://predr.ag/blog/definitive-guide-to-sealed-traits-in-rust/
- [R3] LangChain MCP adapter resource-lifetime bug — https://github.com/langchain-ai/langchain-mcp-adapters/issues/189

### Research evidence (verified 2026-06-08)

- [R4] async-fn-in-`dyn` still unstable in 2026 — `.box`-notation Rust project goal — https://rust-lang.github.io/rust-project-goals/2026/afidt-box.html ; `async-trait` (chosen erasure) — https://docs.rs/async-trait/ ; `dynosaur` (rejected RPITIT-preserving alternative) — https://docs.rs/dynosaur/latest/dynosaur/
- [R5] Rust agent-framework tool shapes: rig `Tool` trait (`name`/args/output/error, `definition()`) — https://docs.rs/rig-core/latest/rig/tool/trait.Tool.html ; swiftide tool hooks (before/after, output-modifying) — https://swiftide.rs/agents/creating-tools/
- [R6] Agent tool-use patterns 2026 (caching, parallel/batch + orchestration mode, output budgets, HITL propose→approve→execute, untrusted-data lane) — https://www.agentpatterns.tech/en/security/tool-permissions ; https://fieldjournal.ai/blog/shipping-safe-tooling-for-tool-calling-agents/
- [R7] MCP 2025-06-18 tools (structuredContent + `output_schema`, `resource_link` content block as lazy URI pointer) — https://modelcontextprotocol.io/specification/2025-06-18/server/tools ; changelog — https://modelcontextprotocol.io/specification/2025-06-18/changelog
- [R8] rmcp runtime tool enable/disable (PR #809), auto default router (PR #785) — https://github.com/modelcontextprotocol/rust-sdk/pull/809
