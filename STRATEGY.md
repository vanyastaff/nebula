---
name: Nebula
last_updated: 2026-05-17
---

# Nebula Strategy

## How we build (delivery model)

Nebula is built **with** LLM coding agents (Claude Code, Cursor, Copilot, CodeRabbit, and similar)—not on a classic human-month roadmap. Calendar estimates like “this feature takes two months” are not the planning unit; **capability completeness** and **engine truth** are.

**Product ambition:** ship the **strongest market bar from the start**—all dimensions in the 2026 standard table and the enduring bets—not a trimmed MVP that defers visual authoring, agents, or durability “for later.” Sequencing follows **dependencies and proof** (knife, MATURITY levels, canon §4.5), not **scope amputation for speed**.

**Still non-negotiable:** operational honesty. We do not **cut** features from intent; we also do not **claim** capabilities the engine does not own end-to-end yet. Agents accelerate implementation; they do not replace honest maturity labeling.

## Target problem

Solo developers and small teams building workflow-driven products with LLM nodes in the pipeline are stuck between two camps. On the light stack (n8n, Make) they get visual authoring and local runs, but pay for leaky data semantics (e.g. `1+1=11` as runtime-normal) and a plugin model where integrations collapse into monolithic nodes with if/else dispatch by resource/operation, years of cruft, and breaking changes. On the heavy stack (Temporal, Restate) they get durable execution but lose the visual editor, single-binary local runs, and a broad plugin ecosystem—and LLM stays bolt-on in both. They either live with fragile automation they cannot trust with customer state, or run two parallel stacks and lose authors who can assemble workflows without code.

## Our approach

We win on **runtime and contract honesty**, not feature breadth: fewer promises, each honored end-to-end in the engine; typed boundaries between nodes; checkpoint-based recovery instead of Temporal-style replay operations; **library-first** (`nebula-sdk` as the headline surface) and **local-first** (SQLite, single process, no mandatory Docker for dev).

**LLM and agents are not a separate product.** The same engine, journal, credentials, resources, and actions serve classic DAG steps and agent loops (tool use, streaming, dynamic edges where policy allows). Providers sit at the edge; durability, idempotency, cancel, and explainability sit in the middle.

**AI-native in 2026 is mandatory.** A workflow engine that does not interoperate with the AI tool ecosystem (LLM providers, **MCP**, agent loops, visual composable graphs in the ComfyUI / n8n class) is the wrong product. Nebula is built for teams who already live in that stack—not as a pre-AI automation relic.

## Ideality

A solo team can author in Rust or visually, run locally in one binary, and trust that every classical step, tool call, and LLM turn is **durable**, **typed at boundaries**, and **explainable from one run record**—without maintaining a second orchestration stack for AI.

## Standard bar (2026)

What “serious” means in 2026 for workflow + LLM (claim levels in `docs/MATURITY.md`; no L3 in strategy until the engine owns it end-to-end):

| Dimension | 2026 standard (what good looks like) |
|-----------|--------------------------------------|
| **Durable execution** | Persisted execution state, CAS transitions, durable cancel, idempotency, checkpoint policy, at-least-once triggers with dedup (Temporal-class *honesty*, not necessarily replay-as-code) |
| **Typed workflow** | Invalid graphs rejected at activation; validated contracts between steps, not JSON folklore |
| **Agent loop** | Tool registry, streaming, iteration limits, token/cost metadata, sub-workflows as composition (LangGraph/CrewAI-class *patterns* on Nebula primitives) |
| **Run explainability** | One lineage: journal + traces + per-step errors + tool/LLM events answer “what happened” without source spelunking |
| **Integration ecosystem** | Orthogonal Resource / Credential / Action / Plugin / Schema—not monolithic mega-nodes |
| **Visual + durable local** | Visual authoring uses the **same** runtime semantics as self-hosted; local path is not a weak preview runtime |
| **AI ecosystem interop** | First-class paths for LLM providers, composable node graphs, and typed agent tools on native primitives; **MCP wire interop phased after** integrator-ready platform (see **MCP (timing)** below)—not a bolt-on “AI node” on a non-AI core |

## Enduring bets (10+ years)

Architectural choices intended to outlive model and vendor churn:

1. **One semantic core, many surfaces** — API chains, agents, and factory automation share execution, journal, and contracts.
2. **Operational honesty** — public surface only if the engine honors it; trust compounds.
3. **LLM at the edge, engine in the middle** — providers are plugins; durability and tool execution are not prompt glue.
4. **Structural typing between nodes** — schema-validated boundaries survive API churn better than ad hoc JSON in userland.
5. **Full Rust ecosystem in actions** — isolation via process sandbox and capabilities, not shrinking the crate graph for WASM-primary plugins.

## Product layers (two planes)

Nebula separates **who writes integrations** from **who uses the product**:

| Plane | Who | What they get |
|-------|-----|----------------|
| **Integration surface** | Rust integration authors | Extend the `Action` trait family in `nebula-action`—today `StatelessAction`, `StatefulAction`, `TriggerAction`, `ResourceAction`; planned **`AgentTool`**, **`ResourceTool`** (tools bound to a resource), **`AgentAction`** (agent loop as a workflow node). Same metadata + schema + slot deps; engine owns dispatch, durability, credentials, journal. Authors ship AI-capable integrations without bespoke LLM glue. |
| **Product intelligence** | Workflow authors and operators in the deployed product | The **core helps users of the product**: suggest or generate workflows, fix validation errors, explain failed runs, propose recovery—using the unified journal and typed graph, not a second ad hoc copilot stack. This is not a replacement for integrator code; it is operator/author assistance on top of honest runtime semantics. |

**Design rule:** LLM/provider logic stays at the **edge** (credentials, `Llm` drivers, optional MCP bridges). The **engine** stays provider-agnostic and records every tool call and LLM turn in the same run record.

Planned trait directions (authoring, not marketing):

- **`AgentAction`** — a **composable hub node** on the canvas (n8n “Tools Agent” class UX, Nebula runtime semantics): the agent step **attaches auxiliary nodes** via typed slots, orchestrates them in a loop, and still runs as one durable workflow step. Typical slots:
  - **Chat model** — `Credential` / LLM `Resource` (e.g. OpenAI chat).
  - **Memory** — `Resource` for context store (sheet, DB, vector index).
  - **Tools** — one or more **`AgentTool`** nodes and/or **`ResourceTool`** bundles (e.g. Email, Calendar, Research); may include nested mini-graphs (vector store + embeddings) wired as supply edges, not a monolithic mega-node.
  - **Sub-agents** (optional) — another workflow or `AgentAction` invoked as a tool-shaped attachment.

  At **activation**, the engine validates slot kinds, schemas, and credentials—invalid combinations fail before run. At **runtime**, tool calls, model turns, and memory reads append to the **same execution journal** as the parent run. Visually: hub on the main flow + satellites below/ beside (see `docs/adr/0065-visual-rendering-modes.md` for hidden vs canvas-bound supply edges).

- **`AgentTool`** — one typed tool the hub (or any agent loop) can invoke (`StatelessAction` + tool description projection; see `docs/adr/0057-ai-agent-sdk.md`).
- **`ResourceTool`** — many tools exported from one `Resource` (connection owns lifecycle; tools are typed methods, not if/else inside one node).

External AI tools (including MCP, when phased in) map onto the same tool registry the agent hub uses—one execution model underneath.

## Integration author SDK (`nebula-sdk`)

**One import for integrators.** `nebula-sdk` is the canonical façade (see `docs/adr/0055-nebula-sdk-facade.md`): plugin authors depend on a single crate; internal workspace crates stay implementation details. Authors may always drop to pure traits in underlying crates when they need full control.

| Crate | Author-facing role |
|-------|-------------------|
| **`nebula-schema`** | **`Schema` = configuration form**—the typed description of **input parameters** the product user fills in (labels, validation, defaults). The same mechanism wires **settings** into an **Action** (node input), **Credential** (how to connect / OAuth fields), and **Resource** (pool/bot/client config). Not a separate “UI framework”; it is the shared config surface the engine validates at activation. |
| **`nebula-credential`** | **Smart auth plane**—acquisition, refresh, rotation, projection to actions; uses the same `Schema` to collect what the user must provide (OAuth, API keys, etc.). Engine owns orchestration; integrators implement `Credential` + schemes. |
| **`nebula-action`** | **`Action` trait family** with deliberate execution shapes: `StatelessAction`, `StatefulAction`, `ResourceAction`, `TriggerAction`, `ControlAction`, plus DX layers (`WebhookAction`, `PollAction`, `PaginatedAction`, `BatchAction`, …) and AI shapes (**`AgentAction`** hub, **`AgentTool`**, planned **`StreamAction`**, **`AwaitAction`**, etc.). Authors pick the shape that matches lifecycle; no obligation to use every helper—plain trait impls remain valid. |
| **`nebula-resource`** | **Long-lived managed objects** with topology and lifecycle (pools, clients, bots). A resource can be **shared across workflows** when configuration matches (e.g. one Telegram bot `Resource` serving many graphs). Engine owns acquire/release/refresh; integrators define the type and caps. |
| **`nebula-plugin`** | **Bundle builder**—one `Plugin` registers actions, credentials, resources, and locales in a single manifest. Distribution is a **small native binary** (ordinary Rust crate + `plugin.toml`), executed in **process isolation** (`ProcessSandbox`), **not** WASM/WASI as the primary model—full `crates.io` access for authors. |

**Mental model:** `Schema` defines **what the user configures** (forms / parameters) → those values bind into **Action**, **Credential**, and **Resource** instances → actions execute with resolved config → **`Plugin`** registers the bundle. Product users edit parameters in the UI; integrators define the `Schema` once in Rust.

### Dependency graph (Action, Credential, Resource)

Integration types are not flat—each declares **what it needs** before it can run. The engine resolves dependencies in order and validates the graph at **activation** (missing or wrong-type binding = structured error, not runtime surprise).

| Dependent | Can depend on | Example |
|-----------|----------------|---------|
| **Action** | `Credential`, `Resource` (slots) | Send-message action → `TelegramBot` resource + bot token credential |
| **Resource** | **`Credential`** | `SlackResource` → `SlackCredential` (auth before client is created) |
| **Resource** | **`Resource`** (another resource) | Domain resource → `HttpResource` transport; app resource → `LoggerResource` / `MetricResource` for cross-cutting infra |
| **Credential** | *(typically leaf)* | Holds secrets/settings via its own `Schema`; may use external providers |

**Patterns:**

- **Auth before client:** `SlackResource` / `PostgresResource` declare `#[credential]` slots; the framework resolves credentials **before** `Resource::create`.
- **Compose infrastructure:** `HttpResource`, `LoggerResource`, `MetricResource` are reusable resources other resources or actions attach to—shared topology when config matches (same HTTP pool, same logger sink).
- **Actions consume the stack:** an action binds only the **leaf** resources/credentials it needs; supply edges on the canvas show the chain (see `docs/adr/0065-visual-rendering-modes.md`).

Integrators declare deps via slot fields (`#[credential]`, `#[resource]`) and `Dependencies` / `DeclaresDependencies`—the graph is typed, not stringly wiring in workflow JSON.

**Bindings vary by case.** The same integration types support different link shapes depending on the scenario—the engine validates each graph, but authors and workflow designers choose the wiring:

| Case | Typical binding |
|------|-----------------|
| Required auth | `Credential` slot required before `Resource::create` (e.g. Slack) |
| Optional audit | `Option<CredentialGuard<…>>` or optional resource slot |
| Lazy / expensive | `Lazy<…>` — resolve only when the code path uses it |
| Shared infra | One `HttpResource` / `LoggerResource` / `MetricResource` referenced from many actions or resources |
| Per-workflow override | Same action type, different `slot_bindings` on each node (bind another bot, another DB) |
| Resource-on-resource | Only when that resource’s impl declares a `#[resource]` slot (not every resource needs every infra type) |
| Multi-credential resource | Several `#[credential]` slots with per-slot `on_credential_refresh` |
| Agent hub | `AgentAction` slots: model, memory, tools—different sets per assistant node |

There is no single universal wiring diagram; there is a **small set of slot kinds** (credential, resource, tool, model, …) and **consistent resolve + validation rules** so any allowed graph is safe and explainable in the journal.

## Who it's for

**Primary:** A Rust developer writing integrations (actions, credentials, resources, plugins, **AgentTool / ResourceTool / AgentAction**). They hire Nebula to describe node and tool logic and trust the engine for concurrency, retry, credential rotation, durable state, and agent loops—without hand-rolling orchestration infrastructure.

**Secondary:** An operator or workflow author using the **product** (visual or configured graphs). They hire Nebula to compose flows, get AI-assisted authoring and debugging, and after any failure explain what happened and recover safely—from journal, API, and metrics, without reading integration source.

## Key metrics

**Operator (runtime health):**

- **execution_terminal_rate** — share of executions reaching terminal status over 28d
- **cancel_honor_latency_p95** — p95 cancel request → terminal `Cancelled`; target ≤5s
- **checkpoint_write_success_rate** — successful checkpoint writes / attempts; target ≥99.9%
- **dispatch_lag_p95** — p95 control-queue drain lag; target ≤1s

**Author / agent (product proof):**

- **time_to_first_typed_agent_tool** — median time from clone to a working, tested agent tool on the golden path (leading indicator for SDK + agent idiom)

## Tracks

All tracks are strategic commitments; sequencing is by proof points and dependencies, not by dropping pillars.

### Unified run record

One durable lineage for classical steps, tool calls, LLM turns, checkpoints, cancel, and token/cost metadata—operator and debugger see a single story.

_Why it serves the approach:_ the 10-year jump is explainability and replay policy on one model, not two stacks.

### Durable execution & control plane

`ExecutionRepo` + CAS, durable `execution_control_queue`, checkpoints, cancel end-to-end, idempotency—customer state survives restart.

_Why it serves the approach:_ without this, “durable” is imitation, like the light engines.

### Typed integration & agent surface

Resource / Credential / Action / Schema / Plugin plus agent loop idioms on the **same** primitives (`nebula-agent` direction); typed tools, streaming, workflow-composed multi-agent; **MCP and external tool protocols** as interoperability surfaces (not a parallel ad hoc integration style).

_Why it serves the approach:_ LLM is first-class without a second platform; closes weak contracts, bolt-on agents, and “automation without AI” irrelevance in 2026.

### Visual authoring with production semantics

Visual editor track tied to the same activation, validation, and runtime as Rust/YAML—local single-binary dev is not a toy runtime.

_Why it serves the approach:_ closes the gap in the target problem (visual **and** durable); built in parallel with engine maturity, not deferred as a “phase 2 product.”

### Operational honesty & observability

Activation validation, no false capabilities, journal + SLI/SLO, credential metrics—real status, not green-washed success.

_Why it serves the approach:_ trust in customer state beats types on paper.

### Local-first path to production

SQLite-by-default dev, embeddable crates, knife scenario green, Postgres for throughput—one path local → self-hosted → cloud-shaped deploy.

_Why it serves the approach:_ removes Temporal-class “compose to hello world” friction for the target audience.

## Not working on

- **Scope-trimmed MVPs** — cutting visual, agent, durable, or **AI interop** pillars “to ship faster”; plans optimize for full bar, sequenced by dependencies only
- **Pre-AI workflow product** — shipping a “classic automation only” core and treating LLM/MCP/visual-AI graphs as optional extras
- **Calendar-first roadmaps** — human-month gating as the primary planning language (agent-assisted delivery is the default)
- Competing with Make/Zapier for no-code / SaaS iPaaS primary audience
- A “most nodes wins” race instead of SDK quality and reliable canonical integrations
- WASM/WASI as the primary plugin isolation path (explicit non-goal in canon)
- Low-code as the primary author surface (operators compose; authors write Rust)
- Marketing retry, durability, agent, or isolation capabilities before the engine owns them end-to-end
- A parallel LLM orchestration stack while classical workflow runs on Nebula

## Marketing

**One-liner:** High-throughput workflow orchestration with a first-class Rust integration SDK — typed, self-hosted, owned by you.

**Key message:** The 2026 standard for serious workflow + LLM: durable, typed, explainable runs—including agent loops—on one engine. Not the largest node catalog; the runtime that still makes sense in ten years.

## Flagship (what “done” proves first)

The flagship is **not** one polished end-user assistant flow. It is a **ready integration platform** developers can extend on day one:

- **Action trait family** shipped and documented (`StatelessAction`, `StatefulAction`, `ResourceAction`, `TriggerAction`, `ControlAction`, AI shapes, DX helpers)—authors can start new integrations immediately.
- **Reference nodes and flows**—canonical workflow steps that demonstrate patterns (not a minimal toy engine).
- **Credential catalog** covering popular auth shapes (**API key**, **OAuth2**, and the other common schemes)—built-in types integrators reuse or subclass instead of reimplementing auth plumbing.
- **`nebula-sdk` + `Schema`**—one path from `schema` → credential/resource/action → `plugin` bundle.

Product-user copilot flows and flashy demos come **after** this substrate is trustworthy. Success sounds like: *“I cloned Nebula, found Slack/OAuth/API-key credentials and example actions, and shipped my plugin the same day.”*

## MCP (timing)

**MCP wire protocol and external MCP server interop are deferred** until the integration platform above is in place and third-party authors are actively building on it. Strategy still treats AI ecosystem interop as mandatory long-term; first delivery is **typed tools + agent hub on native primitives**, then MCP bridge once the project attracts integrators and extension demand is real.

## Related artifacts

- Requirements: `docs/brainstorms/2026-05-17-strategy-llm-standard-bar-requirements.md`
- Normative product rules: `docs/PRODUCT_CANON.md`
- Integration mechanics: `docs/INTEGRATION_MODEL.md`
- Doc consolidation gate (must pass before flagship implementation): `docs/plans/2026-05-17-002-refactor-doc-consolidation-plan.md`
- Flagship integrator platform (blocked on doc gate): `docs/plans/2026-05-17-001-feat-integrator-flagship-platform-plan.md`
- Agent direction (proposed): `docs/adr/0057-ai-agent-sdk.md`
