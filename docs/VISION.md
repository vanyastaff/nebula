# Nebula — Vision & Architecture Charter

> Status: **Draft for review** · Date: 2026-05-14 · Supersedes ad-hoc decisions in
> ADR-0042/0043/0044/0045/0052; complemented by ADR-0053+ (forthcoming).

This document captures the product positioning and architectural principles
agreed during the May 2026 design sessions. It is the **single source of truth**
for "what Nebula is" and "what shape its public surface takes" until the next
charter revision.

---

## 1. Mission

**Build the first production-grade Rust workflow engine that pairs Python-level
developer ergonomics with Rust-level safety, performance, and ecosystem
access.**

Workflow engines today force a choice:

- **n8n / Zapier / Make** — visual & easy, but JavaScript dynamic typing →
  runtime crashes at scale, no real plugin isolation, single-binary
  monolith.
- **Airflow / Prefect / Dagster** — Python ecosystem, but GIL-bound scheduler,
  no compile-time DAG validation, slow at high throughput.
- **Temporal / Cadence** — durable execution gold standard, but workflow code
  is "Go-shaped Java" — alien programming model, deterministic constraints
  enforced at runtime via panic.
- **Argo Workflows** — Kubernetes-native, but pure YAML hell with no
  programming logic, no type safety.
- **Restate.dev** — closest peer (Rust-based durable execution), but
  TypeScript-first SDK, less rich integration story.

Nebula occupies the gap none of these fill: **type-safe, capability-secured,
fully-Rust, full-ecosystem-access** workflow runtime that scales from a 4-line
Hello World to 24/7 factory automation.

---

## 2. Product Positioning

### Three target profiles, one core engine

| Profile | Sample workload | Critical guarantees |
|---|---|---|
| **Simple API integration** | "fetch URL → transform JSON → POST elsewhere" | <100ns dispatch overhead, 4-line Hello World, zero-config auth |
| **AI agent orchestration** | RAG pipelines, tool use, multi-agent coordination | streaming output, cost tracking, dynamic DAG, deterministic replay |
| **Factory / DevOps automation** | 24/7 monitoring, durable state, sandboxed third-party plugins | OS-level isolation, lease-based recovery, capability-gated security, observability everywhere |

These profiles share one engine. Differences are expressed through
**execution policy** (storage backend, isolation level, observability
configuration), not through separate engines.

### Conscious non-targets (current version)

- **Visual workflow editor** — long-term goal; until v1.0 workflows are
  defined as Rust code or YAML.
- **Polyglot plugin authoring** — Rust-only plugin SDK in the v1.x line.
  WASM-based polyglot plugins revisited in 2028+ when WASI preview3 stable
  and Tokio/reqwest/sqlx/AWS SDK have full WASM targets.
- **Browser/edge runtime** — backend-only.

---

## 3. Architectural Principles

These nine principles guide every API decision. When in doubt, choose the
option that better serves them.

### Product principles (P-series)

> **P1 — "Compiles or doesn't run."** Workflow validation happens at compile
> time wherever possible: type-safe ports, slot bindings, capability
> requirements. Runtime validation is a fallback, not the default.

> **P2 — "One core, many surfaces."** A single engine serves simple API
> chains, AI agents, and factory DevOps. Differences are SDK shapes
> (idiomatic Rust today; YAML-defined DAGs; future visual editor) over
> identical semantics.

> **P3 — "Capability-gated, not permission-gated."** Security flows through
> the type system: an action that doesn't declare `Capability<ReadCredential<X>>`
> in its signature **cannot compile** code that reads credential `X`.
> Runtime checks are last-resort defense.

> **P4 — "Memory-safe end-to-end, including plugins."** `forbid(unsafe)` in
> core, plugin SDK, and any future WASM bridge. Polyglot plugins (when
> they arrive) memory-safe by construction.

> **P5 — "Streaming first-class, not bolted-on."** `OutputEnvelope::Stream`
> is a peer of `OutputEnvelope::Value`. `bytes::Bytes` for zero-copy byte
> streams. AsyncIterator migration after stabilization.

> **P6 — "Ergonomic without sacrifice."** `#[action]` attribute on `async fn`
> = 4-line Hello World. Under the hood: monomorphized, type-safe dispatch.
> Python-level DX, Rust-level performance.

### Foundation principles (F-series — "Foundation Five" crates)

> **F7 — "Schema is the form."** `nebula-schema` serves three purposes
> equally: type safety, runtime validation, UI form generation. Field
> attributes (`#[field(label, placeholder, secret, when, ...)]`) are
> standard vocabulary. Form-rendering crates are optional, opt-in via
> separate crates (`nebula-schema-form-html`, `-react`, etc.).

> **F8 — "Dependencies as a typed graph, validated at registration."**
> Action → {Resource, Credential}, Resource → {Resource, Credential},
> Credential → Credential (derived chains). Cycles forbidden — compile-time
> where possible, registration-time via `tarjan` cycle detection where not.
> `on_failure` policy per-dependency (`fail_fast | degrade | defer`).

> **F9 — "Symmetric API surface across Foundation Five."** Authors interact
> with Resource / Credential through identical patterns:
> `Handle<T>` typed alias resolves via `<T as Acquirable>::Handle`;
> single `#[require("key")]` attribute, kind inferred from type's
> `Acquirable` impl; `ctx.acquire("key")` unified method.
> Modifier semantics encoded through standard wrappers (`Option<Handle<T>>`,
> `Lazy<Handle<T>>`, `Option<Lazy<Handle<T>>>`) — `Resolvable` trait with
> blanket impls auto-composes them. Drop legacy aliases (`CredentialFor`,
> separate `#[resource]`/`#[credential]` as default form). Operators see
> identical observability event shape across kinds.

> **F8/F9 implementation notes:**
>
> *Type discrimination* between Resource and Credential happens through
> `Acquirable` trait family with blanket impls on `R: Resource` and
> `C: Credential`. Sealed pattern (or registration-time runtime check
> until negative impls stable) prevents one type implementing both.
> **Zero-cost dispatch at compile time.**
>
> *Modifier composition*: `Resolvable` trait with blanket impls — direct
> `Handle<T>`, `Option<H>`, `Lazy<H>`. Composition `Option<Lazy<H>>`
> works automatically via blanket-on-blanket. Derive emits one uniform
> call: `<FieldType as Resolvable>::resolve(ctx, "key").await?`.
>
> *Conscious non-support* in v1.x: `Vec<Handle<T>>` (use multiple field
> declarations), `Refresh<H>` (becomes `Refreshable` trait impl on
> Credential), `Pooled<H, N>` (becomes resource topology in `nebula-resource`).
> `#[diagnostic::on_unimplemented]` on `Resolvable` provides helpful
> error messages with suggested alternatives.

> **F10 — "Schema is data, not behavior."** `ValidSchema` carries
> describable shape; `Validate` trait carries check logic; `HasSchema`
> ties type to schema. Three concerns, three traits. The `nebula-schema`
> crate stays free of evaluation logic; runtime evaluation lives in
> `nebula-validator` and `nebula-expression` (sibling Core-layer crates).

> **F11 — "Build on JSON Schema 2020-12, do not parallel it."** Lossless
> export via `x-nebula-*` annotations (already shipped via the optional
> `schemars` feature). Free interop with OpenAPI tooling chain — see
> ADR-0047.

> **F12 — "Extension points: validators only. Field types, widgets,
> input hints, formats — closed-set."** `Validator` trait extensible
> via `nebula-validator`. `Field` enum (13 variants), `Widget` enum
> (per-family typed), `InputHint` enum (~20 variants), and format
> vocabulary all closed-set with `#[non_exhaustive]` semver discipline.
> Closed-set extension surface = security (no XSS injection vector),
> consistency (no UI fragmentation), maintainability (no surface
> bloat), supply-chain hardening (CSP-compatible editor).

> **F13 — "Newtype with auto-validation is the flagship pattern; ship
> `nebula-schema::stdlib` module by default."** `Email`, `Url`, `Cron`,
> `DurationStr`, `IpAddr`, `Uuid`, `SemverRange` etc. carry their
> constraints in the type. Author writes `email: Email` — gets
> validation, schema, format hint, widget, docs — all from one type
> declaration. JSON Schema ecosystem cannot do this because dynamic-typed
> languages can't.

> **F14 — "All errors at once, never first-only."** `ValidationReport`
> accumulates everything, returns batched. UX requirement, not
> performance one. Already implemented via existing `ValidationError`
> + `ValidationReport` types.

> **F15 — "Three siblings, one purpose."** `nebula-schema`,
> `nebula-validator`, `nebula-expression` form a coupled-but-separated
> Core-layer subsystem. Each is single-responsibility:
> `nebula-schema` describes shape, `nebula-validator` checks values,
> `nebula-expression` resolves placeholders. Authors interact through
> the `nebula-sdk` facade — they don't see the split.

> **F16 — "Closed-set extension surfaces (overarching rule)."** Where
> extensibility is not strictly required for legitimate author use
> cases — close it. Widgets, renderers, validator built-ins,
> field types — all closed-set with semver-disciplined growth. Only
> `Validator` trait (custom validation rules, scoped per-author) and
> JSON Schema interop (vendor extensions namespace) are extension
> points.

> **F17 — "Schema describes input. Slot bindings describe selection.
> Two channels."** User-facing form rendering has two distinct sources:
> **schema** (generated from `#[derive(Schema)]` on `Self::Input`) for
> action input fields, and **slot bindings** (generated from
> `#[require(...)]` declarations on the Action struct) for resource and
> credential pickers. UI editor renders these as separate panels — no
> overlap, no boilerplate copy-paste, no runtime mismatches between
> declared type and picked instance. (n8n collapses both into one
> schema and pays for it; we deliberately separate.)

> **F18 — "Slot binding has three layers."**
> 1. **Author declares need** (`#[require("key")] field: Handle<T>`)
>    — compile-time, in plugin code.
> 2. **Workflow author binds instance** (`slot_bindings: { key:
>    "instance_id" }`) — config time, in workflow YAML or via UI
>    picker.
> 3. **Deployment registers instances** (`resources.register("instance_id",
>    ...)`) — startup time, in deployment binary.
>
> Three audiences, three layers, single source of truth per layer. UI
> editor's picker dropdown options come from layer-3 registered
> instances filtered by layer-1 type. Forward-compatible with future
> TypedDAG (compile-time generic bounds replace runtime picker without
> changing author code).

> **F19 — "Author API decoupled from visual presentation."**
> `#[require(...)]` declarations are stable across all visual
> rendering choices. Visual editor renders bindings via **pluggable
> mode**: hidden + inspector (default) / canvas node (opt-in) /
> layered canvas (future). Per-workflow preference persisted;
> per-user default. Engine doesn't depend on visual choice — same
> `slot_bindings` produced regardless. Author writes one declaration;
> editor chooses how to draw it.

> **F20 — "Default hidden, opt-in visible."**
> Default visual rendering: bindings invisible from main canvas,
> fully exposed in side **Inspector** panel (search by key, "show
> where used", audit, promote actions). Power users **promote**
> any binding to canvas node — Pattern B style with supply edges
> visible. Multi-agent workflows **auto-promote** shared tools (3+
> consumers heuristic) so that integration topology becomes visible
> exactly where it matters. Default protects 80% of workflow authors
> (business analysts / operators) from infrastructure cognitive
> load; opt-in serves 20% (integration architects / multi-agent
> designers).

---

## 4. SDK & Plugin Model

### Single facade — `nebula-sdk`

Plugin authors and engine integrators depend on **exactly one** Nebula
crate: `nebula-sdk`. All other internal crates (`nebula-action`,
`nebula-engine`, `nebula-credential`, `nebula-resource`, `nebula-workflow`,
`nebula-storage`, `nebula-execution`, `nebula-schema`) are **implementation
details** re-exported through the facade.

Pattern: `tokio` umbrella crate, `serde` re-exporting `serde_derive`,
`axum` re-exporting `axum-core` + `axum-macros`.

```toml
# A plugin author's Cargo.toml
[dependencies]
nebula-sdk = "1.0"

# Any third-party crate from crates.io / git — no restrictions
reqwest    = "0.12"
sqlx       = { version = "0.8", features = ["postgres"] }
aws-sdk-s3 = "1.50"
```

```toml
# An integrator's Cargo.toml
[dependencies]
nebula-sdk = { version = "1.0", features = [
    "stateful", "webhook", "credentials", "resources",
    "storage-postgres", "sandbox-process", "metrics", "tracing",
] }

# Plugins are ordinary cargo crates
nebula-plugin-stripe   = "0.1"
nebula-plugin-slack    = "0.5"
my-internal-actions    = { path = "../my-internal-actions" }
```

### Facade contents

```rust
// nebula-sdk/src/lib.rs

// === FOR PLUGIN AUTHORS ===
pub use nebula_action::{
    Action, StatelessAction, StatefulAction, TriggerAction, ResourceAction,
    ControlAction, WebhookAction, PollAction, PaginatedAction, BatchAction,
    StatelessOutcome, StatefulOutcome, ControlOutcome, TriggerEventOutcome,
    OutputEnvelope, OutputMeta, ActionMetadata, ActionError,
    ActionContext, TriggerContext,
    action,                                 // attribute macro
};
pub use nebula_action::derive::Action;      // derive macro

pub use nebula_credential::{Credential, CredentialGuard, CredentialFor};
pub use nebula_resource::{Resource, ResourceGuard};
pub use nebula_schema::{
    Schema, HasSchema, ValidSchema, ValidValues, ResolvedValues,
    Field, FieldKey, FieldValue, FieldValues,
    field_key,
};
pub use nebula_schema::stdlib::{Email, Url, IpAddr, Cron, DurationStr, Uuid};   // F13 newtype zoo
pub use nebula_validator::{Validator, ValidatorRegistry, Predicate, Rule};
pub use nebula_expression::{Expression, ExpressionAst, ExpressionEngine};

// === FOR ENGINE INTEGRATORS ===
pub use nebula_engine::{WorkflowEngine, EngineBuilder, EngineConfig, ActionRegistry};
pub use nebula_storage::Storage;            // backends behind features

// === COMMON ===
pub use nebula_error::{NebulaError, ErrorKind};

// === PRELUDE ===
pub mod prelude {
    pub use crate::{
        Action, StatelessAction, StatefulAction, ActionContext, ActionError,
        ActionMetadata, StatelessOutcome, OutputEnvelope, action,
    };
    pub use serde::{Deserialize, Serialize};
}
```

### Feature flags

```toml
# nebula-sdk/Cargo.toml [features]
default = ["webhook", "poll", "stateful", "stateless", "control"]

# Action shapes — opt-in to keep compile-time minimal
stateful  = ["nebula-action/stateful"]
webhook   = ["nebula-action/webhook"]
poll      = ["nebula-action/poll"]
control   = ["nebula-action/control"]

# Subsystems
credentials = ["dep:nebula-credential"]
resources   = ["dep:nebula-resource"]

# Storage backends
storage-postgres = ["nebula-storage/postgres"]
storage-sqlite   = ["nebula-storage/sqlite"]
storage-memory   = ["nebula-storage/memory"]

# Sandbox tier
sandbox-inproc  = []                          # default, always available
sandbox-process = ["nebula-engine/sandbox-process"]

# Observability
metrics = ["nebula-engine/metrics"]
tracing = ["nebula-engine/tracing"]
```

### Versioning policy

- **`nebula-sdk = "1.x"`** — single major version per multi-year release
  cycle. Hyrum's law minimization.
- Internal crates may bump major freely; `nebula-sdk = "1.x"` pins them to
  a tested, compatible set.
- Public re-exports treated as `#[stable(since = "1.0")]`. Removal requires
  major bump (`2.0`).
- `prelude` modules — extra-stable. Changes require major bump.
- Internal crates may be referenced through `nebula_sdk::__internal::*`
  with `#[doc(hidden)]` for the rare escape-hatch case; using them is
  explicit acknowledgement of API instability.

### Plugin distribution

Plugin = ordinary cargo crate. Convention:

```rust
// my-stripe-plugin/src/lib.rs
use nebula_sdk::prelude::*;

#[action("stripe.create_customer", name = "Create Stripe Customer")]
async fn create_customer(/* … */) -> Result<StatelessOutcome<String>, ActionError> {
    /* … */
}

/// Public registration — integrator calls this once.
pub fn register_into(registry: &mut nebula_sdk::ActionRegistry) {
    registry.register(create_customer);
    /* additional actions … */
}
```

```rust
// integrator's main.rs
let mut registry = ActionRegistry::new();
nebula_plugin_stripe::register_into(&mut registry);
nebula_plugin_slack::register_into(&mut registry);
```

No special plugin packaging. No private registry. No dynamic loading. No
ABI compatibility concerns. `cargo build --release` produces a single
deployable binary with all plugins statically linked.

---

## 5. Action Surface — "Concept A-modified"

### Trait family

```rust
// Marker trait — author never implements directly.
pub trait Action: DeclaresDependencies + Send + Sync + 'static {}

// Blanket impls per shape:
impl<A: StatelessAction> Action for A {}
impl<A: StatefulAction>  Action for A {}
impl<A: TriggerAction>   Action for A {}
impl<A: ResourceAction>  Action for A {}
impl<A: ControlAction>   Action for A {}
```

`Action` carries no `metadata()` method. `ActionMetadata` lives in the
registry, supplied at registration time:

```rust
registry.register(my_action);                    // metadata via #[action] / derive
registry.register_with(metadata, my_action);     // explicit
```

### Per-shape sub-traits — typed Input/Output

```rust
pub trait StatelessAction: Send + Sync + 'static {
    type Input:  HasSchema + DeserializeOwned + Send + Sync;
    type Output: Serialize + Send + Sync;

    async fn execute(
        &self,
        input: Self::Input,
        ctx: &(impl ActionContext + ?Sized),
    ) -> Result<StatelessOutcome<Self::Output>, ActionError>;
}

pub trait StatefulAction: Send + Sync + 'static {
    type Input:  HasSchema + DeserializeOwned + Send + Sync;
    type Output: Serialize + Send + Sync;
    type State:  Serialize + DeserializeOwned + Clone + Send + Sync;

    fn init_state(&self) -> Self::State;
    async fn execute(
        &self, input: Self::Input, state: &mut Self::State,
        ctx: &(impl ActionContext + ?Sized),
    ) -> Result<StatefulOutcome<Self::Output>, ActionError>;
}

pub trait ControlAction: Send + Sync + 'static {
    async fn evaluate(
        &self, input: ControlInput,
        ctx: &(impl ActionContext + ?Sized),
    ) -> Result<ControlOutcome, ActionError>;
}

// TriggerAction, ResourceAction, WebhookAction, PollAction,
// PaginatedAction, BatchAction — analogous shape-specific signatures.
```

### Authoring patterns

**Function-style** (90% case — recommended for slot-less actions):

```rust
use nebula_sdk::prelude::*;

#[action("greet.hello", name = "Hello", description = "Greet a user by name")]
async fn hello(name: String) -> Result<String, ActionError> {
    Ok(format!("Hello, {name}!"))
}
```

The `#[action]` macro emits the struct, `StatelessAction` impl, registration
helper, and metadata. Author writes nothing else.

**Struct-style** (when state or multi-method DX preferred):

```rust
struct HttpAction;

impl StatelessAction for HttpAction {
    type Input = HttpRequest;
    type Output = HttpResponse;

    async fn execute(
        &self, input: HttpRequest,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> Result<StatelessOutcome<HttpResponse>, ActionError> {
        /* … */
    }
}

// Registration: caller supplies metadata explicitly.
registry.register_with(
    ActionMetadata::new(action_key!("http.request"), "HTTP Request", "…"),
    HttpAction,
);
```

**Derive-style** (required for slot-binding actions; uses symmetric API
per F9):

```rust
#[derive(Action)]
#[action(key = "telegram.send_message", name = "Send Telegram Message")]
struct SendTelegram {
    #[require("bot")]
    bot: Handle<TelegramBot>,                         // resource — kind inferred from type
    #[require("auth")]
    token: Handle<TelegramCredential>,                // credential — kind inferred from type

    #[require("metrics")]
    metrics: Option<Handle<MetricsCollector>>,        // optional + eager modifier
    #[require("expensive_cache")]
    cache:   Lazy<Handle<RedisCache>>,                // required + lazy modifier
}

impl StatelessAction for SendTelegram {
    type Input = SendMessageInput;
    type Output = MessageId;
    async fn execute(/* … */) { /* … */ }
}

registry.register_factory::<SendTelegram>();
```

### Removed by Concept A-modified

- `StaticMetadata` companion trait — eliminated. Metadata lives in
  registry only.
- `Action::metadata()` method — gone. Action is a marker.
- `OnceLock`-ritual in manual implementations — gone.
- `<Self as Action>::Input/Output` — replaced with
  `<Self as StatelessAction>::Input/Output`.

### Symmetric API migration table (per F9 + ADR-0060)

This is a **hard breaking change** for plugin authors. Pre-1.0 acceptable
per `feedback_hard_breaking_changes.md`.

| Aspect | Today (asymmetric) | After F9 (symmetric) |
|---|---|---|
| Resource handle type | `ResourceGuard<R>` | `Handle<R>` (alias for `ResourceHandle<R>`) |
| Credential handle type | `CredentialGuard<C::Scheme>` / `CredentialFor<C>` | `Handle<C>` (alias for `CredentialHandle<C>`) |
| Resource slot attribute | `#[resource(key = "...")]` | `#[require("...")]` (default) or `#[resource(key = "...")]` (explicit, opt-in) |
| Credential slot attribute | `#[credential(key = "...")]` | `#[require("...")]` (default) or `#[credential(key = "...")]` (explicit) |
| Optional resource | `Option<ResourceGuard<R>>` | `Option<Handle<R>>` |
| Lazy resource | `Lazy<ResourceGuard<R>>` | `Lazy<Handle<R>>` |
| Acquisition method | `ctx.acquire_resource_by_id(...)` / `ctx.resolve_credential_by_id(...)` | `ctx.acquire("key")` (single dispatch via `Acquirable`) |
| Error trait | distinct `ResourceError` / `CredentialError` | both impl shared `AcquireFailure` for cross-kind retry |
| Observability event target | `nebula::resource` / `nebula::credential` | uniform `nebula::acquire` with `kind` field |

Kind discrimination happens at compile time through `Acquirable` trait:
`<R as Acquirable>::Handle = ResourceHandle<R>` (via `impl<R: Resource>
Acquirable`); `<C as Acquirable>::Handle = CredentialHandle<C>` (via
`impl<C: Credential> Acquirable`). Sealed pattern prevents one type
implementing both. **Zero runtime cost.**

---

## 6. Result & Output Model

### Per-shape outcomes (instead of one fat enum)

```rust
pub enum StatelessOutcome<T> {
    Success(T),
    Skip { reason: String },
}

pub enum StatefulOutcome<T> {
    Continue { output: T, progress: Option<f64>, delay: Option<Duration> },
    Break    { output: T, reason: BreakReason },
}

pub enum ControlOutcome {
    Branch { port: BranchKey, output: Value },
    Multi  { outputs: HashMap<PortKey, Value> },
    Drop   { reason: Option<String> },
    Pass,
}

pub enum TriggerEventOutcome {
    Emit(Value),
    Drop,
    Defer { until: Instant },
}
```

The compiler enforces shape-correct outcomes: a stateless action **cannot**
return `Continue`. The previous fat `ActionResult<T>` is removed.

`Wait { condition }` and `Stop { code, reason }` move to **engine layer**:
they describe orchestration intent that the engine produces, not what an
action returns.

### Output envelope — observability-first

```rust
pub struct OutputEnvelope<T> {
    pub output: ActionOutput<T>,
    pub meta:   OutputMeta,        // origin, cost, resolution, overflow,
}                                  // tokens, timing, cache_info — defaults
                                   // available via builder

pub enum ActionOutput<T> {
    Value(T),
    Binary(BinaryData),            // bytes::Bytes-backed, refcount-shared
    Stream(StreamOutput<T>),       // Pin<Box<dyn Stream + Send>>; bytes::Bytes
                                   // specialization for byte streams
    Reference(DataReference),      // pointer to external storage
    Deferred(DeferredOutput<T>),   // result fills in later (callback URL +
                                   // retry config); Deferred and Stream
                                   // mutually exclusive (PhantomData enforced)
    Empty,
}
```

`OutputEnvelope<T>` is **always** the carrier. Builder chain for
observability-conscious authors:

```rust
OutputEnvelope::value(my_result)
    .with_cost(Cost::tokens(150))
    .with_progress(0.85)
    .into_port("main")
```

When author returns the bare `T`, defaults fill in `OutputMeta` (origin =
`Computed`, cost zero, no truncation).

### Streaming

`bytes::Bytes` specialization for raw byte streams — refcount-shared, zero
copy. Generic `Stream<Item = T>` for typed payloads (LLM tokens, sensor
events). `AsyncIterator` migration considered after Rust 1.95+ stabilization
checkpoint.

---

## 7. Ports & Connections

Ports describe DAG topology — they are **metadata**, not part of the action
trait API.

```rust
pub struct InputPort  { key, flow, schema, … }
pub struct OutputPort { key, flow, schema, kind, … }   // kind: Primary | Diagnostic
pub struct DynamicPort { … }                           // runtime-determined ports (Switch, Router)

pub enum FlowKind { Data, Control, Meta }
pub struct ConnectionFilter { … }                       // typed compatibility constraint
```

Defaults:

- `StatelessAction` → one output port `"main"`, type derived from
  `Self::Output`.
- `ControlAction` → multiple output ports declared via metadata.
- Author may add custom ports through `ActionMetadata::with_outputs(…)`.

`ConnectionFilter` is enforced at three layers: **UI editor** (greys out
incompatible wires), **workflow validator** (rejects load), **engine
runtime** (paranoid invariant check). One filter, three enforcement points.

`SupportPort` (legacy v4 name) is being renamed to `OutputPort { kind:
Diagnostic }` — semantic clarity, fewer type names.

---

## 8. Security Model

Three-tier isolation. Author / integrator chooses per action via
`IsolationLevel` in `ActionMetadata`.

| Tier | Trust assumption | Enforcement | Overhead | When |
|---|---|---|---|---|
| **In-process trusted** | Plugin = your team or vetted vendor; `forbid(unsafe)` audited | Compile-time **capability through types** — action that doesn't declare `Capability<ReadCredential<X>>` cannot compile credential read | None | Internal plugins, audited dependencies |
| **In-process capability-checked** | Community plugin, declared but unverified | Type-system declaration + runtime atomic check at acquisition | <100ns | Marketplace plugins post-review |
| **ProcessSandbox** | Untrusted source | OS-level: Linux namespaces+seccomp; Windows AppContainer; macOS sandbox-exec; IPC via UDS/Named Pipe | ~100µs | Third-party untrusted code |

Capability through types is the **primary** security gate, not a fallback.
Forthcoming **ADR-0054** specifies the capability trait family.

WASM sandbox (Tier 4) is forward-compatible architecture: when WASI
preview3 is GA and ecosystem (Tokio, sqlx, reqwest, AWS SDKs) supports it,
adding Tier 4 does not break Tiers 1-3.

---

## 9. Roadmap (next 18 months)

**Iteration discipline:** land one milestone per quarter. No promises for
quarter N+1 until quarter N is green. Per Bryan Cantrill's standing rule:
"Workflow engines die from over-promise / under-deliver."

### Q3 2026 — Action surface & Plugin SDK foundation
- Land Concept A-modified (this charter's Section 5).
- Per-shape `*Outcome` enums (Section 6).
- `OutputEnvelope` always returned, defaults via builder.
- Plugin distribution: cargo crate convention, `register_into` API.
- Documentation: "Writing your first Nebula action plugin" — 15-min tutorial.
- **Exit criteria:** `cargo install nebula-sdk` works, Hello World in 4 lines verified.

### Q4 2026 — `nebula-sdk` facade & Capability foundation
- Refactor `nebula-sdk` to full facade (this charter's Section 4).
- Feature flags audit and standardization.
- ADR-0054 — typed capabilities (Sam Scott design collaboration).
- Compile-time capability enforcement for credential & resource access.

### Q1 2027 — Type-safe DAG validation (experimental)
- `nebula-workflow-typed` crate — workflow defined as Rust types.
- Compile error when output type A connects to incompatible input type B.
- Optional path; YAML-defined workflows continue to work.

### Q2 2027 — Production AI agent SDK
- `nebula-agent` crate: typed tool calls, dynamic DAGs, streaming
  outputs, deterministic replay.
- Reference implementation: ReAct agent in ≤200 lines of Rust.

### Q3-Q4 2027 — Plugin ecosystem & Visual MVP
- Curated plugin registry: official integrations for top 30 SaaS.
- Cargo conventions for Nebula plugins published.
- Visual workflow builder MVP — generates workflow YAML; always exports
  to typed Rust code.

### 2028+ — Selective revisits
- WASM Tier 4 sandbox if WASI preview3 GA + ecosystem mature.
- Multi-region durable execution (Stephan Ewen patterns).
- Workflow-as-code (Temporal pattern) **plus** workflow-as-DAG-typed
  (Nebula unique).

### Backlog — Conference Day 4 contributions (industry veterans)

These items emerged from the May 2026 "Predecessors speak" session
([CONFERENCE-NOTES.md](./CONFERENCE-NOTES.md) Day 4). Scheduling deferred
to post-Concept-A-modified landing; each will be promoted to a quarterly
milestone or its own ADR when prioritized.

| ID | Item | Source | Likely target |
|---|---|---|---|
| **B-01** | Workflow planner stage between definition and execution (architectural slot for future optimizer passes — predicate pushdown, parallel branch detection, join elimination) | Matei Zaharia (Spark) | Q2 2027+ |
| **B-02** | Apache Arrow as data transport: `ActionOutput::Binary(BinaryData::Arrow(RecordBatch))` for zero-copy GB-scale tensor passing; interop with pandas/Polars/DuckDB/Spark | Wes McKinney (Arrow), Matei Zaharia | Q1-Q2 2027 |
| **B-03** | Typed agent tools: `AgentTool` trait extending `StatelessAction`, `ToolDescription` auto-derived from `Self::Input: HasSchema` for LLM consumption | Harrison Chase (LangChain) | Q2 2027 (within `nebula-agent`) |
| **B-04** | Token budget tracking: extend `OutputMeta::TokenUsage` with `context_tokens`, `max_context`, `overflow_strategy: Truncate \| Summarize \| Reject` | Jerry Liu (LlamaIndex) | Q2 2027 |
| **B-05** | Built-in OSS dashboard: `nebula serve --dashboard`, per-action latency/success/queue stats, retry visualization, circuit breaker state | Mike Perham (Sidekiq) | Q4 2026 - Q1 2027 |
| **B-06** | Asset materialization events: optional `AssetMaterialization` emit for data-engineering profile (lineage tracking without manual instrumentation) | Nick Schrock (Dagster) | Q3 2027+ |
| **B-07** | Recipe mode YAML: simplified single-trigger-single-action format that compiles to full workflow, for non-developer surface | Linden Tibbets (IFTTT) | Q4 2027 (with visual editor MVP) |
| **B-08** | Workflow versioning via `MigratesFrom<V>` trait — compile-time guarantees that migration paths exist for long-running workflow state changes | Maxim Fateev (Temporal) | Q1 2027+ |
| **B-09** | `run_workflow!` macro for workflow composition — multi-agent orchestration via existing primitives, no new `Agent` abstraction | Joao Moura (CrewAI) | Q2 2027 (within `nebula-agent`) |

### Backlog — Conference Day 5 (Foundation Five sessions)

These items emerged from the May 2026 "Foundation Five" sessions
([CONFERENCE-NOTES.md](./CONFERENCE-NOTES.md) Day 5 morning, evening,
late, and breakfast follow-ups). They land alongside or shortly after
the Concept A-modified iteration, with most folded into ADR-0058
through ADR-0060.

#### `nebula-schema` track

| ID | Item | Source |
|---|---|---|
| **S-1** | Three-tier error split: `SchemaError` / `ValidatorError` / `ValidationError` | Esteban Küber |
| **S-2** | Const `FieldKey` support: `const NAME: FieldKey = field_key!(...);` | dtolnay |
| **S-3** | `JsonSchema` adapter as separate `nebula-schema-jsonschema` contrib crate | Ari Seyhun |
| **S-4** | Newtype-with-auto-validation pattern documented (Email, URL, IpAddr) | Ari Seyhun |
| **S-5** | `nebula-expression-macros` for compile-time path validation (deferred) | Niko Matsakis |
| **S-6** | Extended `#[field(...)]` vocabulary: `label`, `description`, `placeholder`, `secret`, `multiline`, `options`, `section`, `order`, `advanced`, `when` | dtolnay |
| **S-7** | Crate split: `nebula-schema-form` (form metadata) + optional `-html` / `-react` renderers | matklad |
| **S-8** | Custom widget registration via `#[field(widget = "cron")]` | Cart |
| **S-9** | `nebula-cli form-preview --action my.action` — local form preview | Cart |
| **S-10** | Diagnostic for `secret = true` fields suggesting `SecretString` | Esteban Küber |

#### `nebula-credential` track

| ID | Item | Source |
|---|---|---|
| **C-1** | Audit `Drop` impl on every credential type — verify zeroize | Tony Arcieri |
| **C-2** | AAD must include `(workflow_id, node_key, version)` — document + verify | Tony Arcieri |
| **C-3** | `rotate()` API as first-class trait method | Tony Arcieri |
| **C-4** | Single-flight refresh semantics in `RefreshScheduler` | withoutboats |
| **C-5** | Borrowed-while-refreshing pattern (no-blocking) | Carl Lerche |
| **C-6** | `nebula-credential-http` adapter generating `reqwest::Middleware` | Sean McArthur |
| **C-7** | Schedule formal security review with Trail of Bits / NCC Group for v1.0 | Tony Arcieri |
| **C-8** | Derived credential chains via `#[credential]` field on Credential struct — `Refreshable` derives from parent | Tony Arcieri |
| **C-9** | Engine enforces: derived credential cannot have wider scope than parent (security invariant) | Tony Arcieri |

#### `nebula-action` track (post Concept A-modified)

| ID | Item | Source |
|---|---|---|
| **A-1** | `cargo build -p nebula-action` incremental budget <8s, monitor in CI | matklad |
| **A-2** | `lib.rs` public re-exports counted; budget set | matklad |

#### `nebula-resource` track

| ID | Item | Source |
|---|---|---|
| **R-1** | `CleanupFailureBehavior { LogContinue \| AbortScope \| RetryWith(Backoff) }` per `ResourceMetadata` | Cart |
| **R-2** | Pool topology — use `deadpool` or `bb8` underneath, not reinvent | Carl Lerche |
| **R-3** | Resident topology — use `arc-swap` for atomic rotate | Carl Lerche |
| **R-4** | Runtime-agnostic crate — no tokio dep, only traits | Stjepan Glavina |
| **R-5** | Mostly-data crate, engine logic kept out | matklad |
| **R-6** | Resource → Resource dependencies via `#[resource]` field on Resource struct | Carl Lerche |
| **R-7** | `on_failure` policy per-dependency: `fail_fast \| degrade \| defer` | withoutboats |
| **R-8** | Both type-keyed and string-keyed dependency forms | Cart |

#### `nebula-plugin` track

| ID | Item | Source |
|---|---|---|
| **P-1** | `Plugin` trait with `id() / version() / dependencies() / register()` methods | Niko Matsakis |
| **P-2** | `PluginGroup` for bundled plugins (Bevy `DefaultPlugins` precedent) | Cart |
| **P-3** | Topological sort for plugin dependencies | Niko Matsakis |
| **P-4** | Explicit registration only — no `linkme`-style automatic discovery | matklad |
| **P-5** | Numbered error codes in messages (`NEBULA_PLUGIN_001` etc.) | Esteban Küber |

#### Cross-cutting consistency (X-series)

| ID | Item | Source |
|---|---|---|
| **X-1** | `Has*` trait naming convention documented | dtolnay |
| **X-2** | `*Guard<T>` / `*Handle<T>` newtype pattern for acquired resources | Niko Matsakis |
| **X-3** | Per-crate error code ranges allocated (Schema 100-299, Credential 300-399, Resource 400-499, Action 500-599, Plugin 600-699) | Esteban Küber |
| **X-4** | `*Metadata` data-only structs, no behavior | matklad |
| **X-5** | `*Provider` trait pattern for pluggable backends | Carl Lerche |
| **X-6** | Cycle detection algorithm at registration time (`tarjan` SCC) | matklad |
| **X-7** | Numbered cycle error (`NEBULA_DEP_001`) with graph visualization | Esteban Küber + matklad |
| **X-8** | Topological sort of init order: resources → credentials → actions; per-layer sort by depth | (consensus) |
| **X-9** | Common derive infrastructure — extract shared slot-handling logic to `nebula-foundation-derive` helper crate | (consensus) |

#### Symmetric Foundation API (Y-series)

| ID | Item | Source |
|---|---|---|
| **Y-1** | `Acquirable` trait + `Handle<T>` type alias as `<T as Acquirable>::Handle` | Yoshua Wuyts, Carl Lerche |
| **Y-2** | Single `#[require("key")]` attribute, kind inferred from type | withoutboats, Alice Ryhl |
| **Y-3** | Drop `CredentialFor` alias; drop `<C as Credential>::Scheme` from author-facing API | Carl Lerche |
| **Y-4** | `AcquireFailure` trait shared between `ResourceError` and `CredentialError` (cross-type retry semantics) | Eliza Weisman |
| **Y-5** | Standardized observability event structure: `target = "nebula::acquire"`, fields `{kind, key, duration_ms, outcome}` | Eliza Weisman |
| **Y-6** | Negative-impl or registration-time check that no type implements both `Resource` and `Credential` | Niko Matsakis |
| **Y-7** | Documented default: `#[require]` + `Handle<T>`. Explicit form `#[resource]`/`#[credential]` + `ResourceHandle<R>`/`CredentialHandle<C>` remain as opt-in for emphasis cases | dtolnay compromise |
| **Y-8** | `Resolvable` trait + blanket impls on `T: Acquirable`, `Option<H>`, `Lazy<H>` — composability automatic | Niko Matsakis |
| **Y-9** | Derive `#[require("key")]` emits `<FieldType as Resolvable>::resolve(ctx, "key").await?` — uniform for all 4 modifier combinations | dtolnay |
| **Y-10** | Conscious non-support: `Vec<Handle<X>>`, `Refresh<H>`, `Pooled<H, N>`, `Cached<H>`, `Failover<[H; N]>` — typed compile errors with helpful suggestions | Esteban + matklad + withoutboats |
| **Y-11** | Documentation page "Modifier wrappers" — single page explaining 4 supported combinations + when to wrap each | Alice Ryhl |

### Backlog — Conference Day 6 (`nebula-schema` deep design + honest reckoning)

After auditing actual `crates/schema/src/` (24 modules, 11K LOC, already
production-grade), most utром proposals turned out to be already
implemented. Refined backlog reflects **only true gaps** plus
documentation/polish work.

#### `nebula-schema` refinements (NS-series)

| ID | Item | Source |
|---|---|---|
| **NS-1** | Add `nebula-schema::stdlib` module: `Email`, `Url`, `IpAddr`, `Cron`, `DurationStr`, `Uuid`, `SemverRange`. Each newtype impls `Deserialize` + `Serialize` + `HasSchema` (auto-emits `InputHint`) + `Validate`. Default feature `stdlib`. | Aaron Turon |
| **NS-2** | Document three-tier proof token pattern (`Schema → ValidSchema → ValidValues → ResolvedValues`) as flagship feature. README section + blog material. | Niko Matsakis |
| **NS-3** | Document `Mode` field + `Computed` + `Dynamic` as Nebula's clean extensions over JSON Schema. Bring `Mode` proposal to JSON Schema spec community (Henry Andrews offer). | Henry Andrews |
| **NS-4** | Diagnostic improvements in `lint.rs` — concrete PR collaboration | Esteban Küber |
| **NS-5** | File split for `validated.rs` (1943 LOC) → `validated.rs` (proof token types) + `validation.rs` (validation logic). Tier-2, opportunistic. | matklad |
| **NS-6** | Audit `nebula-schema-display` separation — only if heavy formatting deps present. | Carl Lerche |
| **NS-7** | `HasSchemaObject` companion trait — add when first legitimate use case emerges. YAGNI until then. | Niko Matsakis |
| **NS-8** | Add `MetadataSlot` description to `ActionMetadata` — derived from `#[require]` declarations: slot key, expected `TypeId`, kind (Resource \| Credential), `on_failure` policy, optional human-readable label. UI editor reads `ActionMetadata.slots` to render binding panel. | Carl Lerche |
| **NS-9** | Document `#[require]` declarations as **forward-compatible with future TypedDAG generics** — no breaking change planned to author code. | Niko Matsakis |

**Removed** from earlier proposals (already implemented in existing crate):

- ~~SC-7~~ (three-tier error split) — already exists.
- ~~SC-13/14/15~~ (newtype zoo as separate crate) — folded into `nebula-schema::stdlib` module (NS-1).
- ~~SC-17~~ (`FormatRegistry` as extension point) — closed-set `InputHint` enum already covers it (F12).
- ~~SC-19~~ (form renderer crates) — moved to `nebula-editor` separate product.
- ~~Custom widget registration~~ — closed `Widget` enum per F12 (F12 refined).

#### Editor / UI composition (UI-series)

| ID | Item | Source |
|---|---|---|
| **UI-1** | Document **two-panel rendering convention** for workflow editor: "Action Input" (from schema) + "Bindings" (from `#[require]`). Visual mockup in editor design spec. Lives in `nebula-editor` separate product, but contract documented in `nebula-schema` README. | Cart |
| **UI-2** | Helpful disabled state when no resources of required type registered ("ⓘ Add X resource to enable this binding"). | Maxim Fateev (n8n-pain-avoidance) |
| **UI-3** | Document Mode field + slot binding interaction: scheme selection (Mode field) filters instance picker options (slot binding). Two-tier credential rendering. | dtolnay |
| **UI-4** | **Inspector panel** design — list all bindings of selected node and workflow-wide; search by binding key; "show where used" navigation; promote-to-canvas action; audit log. Bevy ECS Inspector precedent. | Cart |
| **UI-5** | **Promote-to-canvas mechanism** — per-workflow persisted state; resource/credential rendered as canvas node with supply edges (dotted, distinguishable from solid data-flow edges). NiFi controllers-on-canvas precedent. | Mark Payne |
| **UI-6** | **Multi-agent shared-tool auto-promotion heuristic** — when 3+ agents/actions reference same resource/credential, automatically promote to canvas node. Tunable threshold per workflow. CrewAI multi-agent insight. | Joao Moura |
| **UI-7** | **Layered canvas** — toggle between "logic layer" (actions + data flow) and "infrastructure layer" (resources + credentials + supply edges). Backlog feature; prototype after default mode (UI-4 + UI-5) ships and adoption is evaluated. | Rich Hickey |

### Process commitments

These are not features — they are operational disciplines committed at the
conference:

- **OSS-first** *(Jeremiah Lowin / Prefect lesson)* — UI, dashboard,
  observability, and core engine functionality stay in **OSS forever**.
  Commercial offerings (when they exist) are managed cloud, hosted SLA,
  enterprise support — never gated functionality.
- **Roadmap honesty** *(Bryan Cantrill, Mike Perham)* — quarterly check-ins.
  When a milestone slips, communicate openly and reset; do not silently
  re-promise.
- **One milestone at a time** *(Bryan Cantrill)* — do not announce Q+1 work
  until Q is green. The temptation to announce ahead of execution kills
  workflow-engine projects.
- **Predictable releases** *(Mike Perham)* — minor every quarter, major
  every 18 months once v1.0 ships. Customers plan around predictability.
- **Cross-promotion ready** — when v1.0 ships, several conference attendees
  pledged blog posts / integration examples (Jan Oberhauser, Harrison Chase,
  Jerry Liu, Joao Moura, Mike Perham, Matei Zaharia at Stanford). Maintain
  these relationships through periodic technical updates.

---

## 10. Conscious non-goals (current charter)

- **Polyglot plugin authoring.** Rust-only in v1.x. Plugin authors are
  expected to be Rust developers. This loses ~70% of potential authors
  vs Python; we accept the trade-off in exchange for type safety,
  performance, and crates.io ecosystem access.

- **WASM runtime / sandbox.** Deferred to 2028+ per Section 8.
  Architecturally compatible.

- **Visual editor as primary surface.** Editor is exporter, not authoring
  primitive. Code-first.

- **Workflow language DSL (Temporal-style).** Not in v1.x. Workflows are
  data (YAML or typed Rust struct).

- **Cross-process distributed actions.** Engine is single-process today
  (with multi-runner lease takeover for HA). Cross-process distributed
  workflows are a 2028+ topic.

- **Two-struct DX (Self + Self::Input).** Acknowledged pain from ADR-0043;
  separate track via **ADR-0053** (forthcoming). Not blocking v1.x.

---

## 11. Decision log

### Existing ADRs preserved or amended

| ADR | Title | Status under this charter |
|---|---|---|
| 0042 | Node binding mechanism (slot binding) | **Amended** by 0052 — slot-binding mechanism preserved |
| 0043 | Dependency declaration DX | **Amended** by 0052 + future 0053 |
| 0044 | Resource/credential singular | Unchanged |
| 0045 | EventTrigger scope deferral | Unchanged |
| 0046 | Metrics/telemetry boundary | Unchanged |
| 0047 | OpenAPI 3.1 generator | Unchanged |
| 0048 | Idempotency store backend | Unchanged |
| 0049 | Webhook handler convergence | Unchanged |
| 0050 | W3C trace context propagation | Unchanged |
| 0051 | External Provider redesign | Unchanged |
| 0052 | Action surface hybrid | **Superseded in part** by this charter — Concept A-modified replaces Concept-Hybrid; `StaticMetadata` removal mandated |

### Forthcoming ADRs

| ADR | Title | Owner | Target |
|---|---|---|---|
| **0053** | Two-struct DX consolidation (Self vs Self::Input) | TBD | Q1 2027 |
| **0054** | Typed capability system | Sam Scott + core team | Q4 2026 |
| **0055** | `nebula-sdk` facade specification | core team | Q4 2026 |
| **0056** | Type-safe DAG validation | core team | Q1 2027 |
| **0057** | AI agent SDK (`nebula-agent`) | core team | Q2 2027 |
| **0058** | Schema field UI vocabulary (`#[field(...)]` standard, closed `Widget` enum, closed format catalog) | dtolnay style + Ari Seyhun | Q3-Q4 2026 |
| **0059** | Cross-foundation dependency graph (Action → {R, C}, R → {R, C}, C → C) + cycle detection | Niko Matsakis + matklad | Q4 2026 |
| **0060** | Symmetric Foundation API (`Acquirable` + `Resolvable` trait families, `Handle<T>` alias, `#[require(...)]` attribute, modifier wrappers) | Carl Lerche + Yoshua Wuyts + dtolnay | Q3-Q4 2026 (alongside `nebula-sdk` facade) |
| **0061** | `nebula-schema` core trait shape ratified — `HasSchema`, `Validate`, `Validator`, three-tier proof tokens (`Schema → ValidSchema → ValidValues → ResolvedValues`); explicit boundaries with `nebula-validator` and `nebula-expression` siblings | core team (ratification of existing design) | Q3 2026 |
| **0062** | `nebula-schema::stdlib` newtype-with-auto-validation pattern: `Email`, `Url`, `Cron`, etc. — NOT separate crate, ship as default feature | Aaron Turon | Q4 2026 |
| **0063** | JSON Schema 2020-12 lossless interop — extends existing `schemars`-feature export with import direction; `x-nebula-*` annotation namespace documented | Henry Andrews + Ari Seyhun | Q1 2027 |
| **0064** | UI form composition — schema vs slot bindings as two distinct channels; two-panel editor rendering; three-layer slot binding architecture (author / workflow / deployment) | Cart + Jan Oberhauser collaboration | Q4 2026 (with `nebula-editor` MVP) |
| **0065** | Visual rendering modes for slot bindings — hidden + Inspector (default) / canvas nodes with supply edges (opt-in) / layered canvas (future). Multi-agent auto-promotion heuristic. Pluggable per-workflow mode persisted in workflow file. | Mark Payne (NiFi) + Cart (Bevy) + Mitchell Hashimoto (Terraform) cross-collaboration | Q1 2027 (with `nebula-editor` MVP) |
| **0066** | Concept A-modified ratified — `Action` as pure marker, `StaticMetadata` deleted, `ActionFactory::metadata` registry-side (Day 8 reckoning) | core team | Q3 2026 |
| **0067** | `HasSchema` blanket-impl policy (`Vec`/`Option`/`Box`); `ValidSchema`/`LintWarnings` split; `#[diagnostic::on_unimplemented]` (Day 9 R1) | core team | Q3 2026 |
| **0068** | `nebula-credential-http` — `HttpAuthScheme` capability + engine-mediated `CredentialMiddleware` (Day 9 Interlude II); reframed as one `HttpClient` Resource, not optional middleware | core team (Sean McArthur advisory) | Q4 2026 |
| **0069** | `Field::Map` 14th variant — string keys, value-schema, duplicate-key/min/max, JSON `additionalProperties` (Day 9 R1) | core team | Q3 2026 |
| **0070** | Credential audience binding + Resource-boundary secret model — `allowed_destinations` mandatory; secret reachable only in `Resource::create`, never Action; deny-by-default (Day 9 Interlude II) | core team (Tony Arcieri / Joe Beda advisory) | Q4 2026 — **v1.0 gate** |
| **0071** | Credential binding scope/RBAC — `bindable_scope`, deny-by-default namespace-local, binding-time rejection (Day 9 Interlude II) | core team (Sam Scott advisory) | Q4 2026 — **v1.0 gate** |
| **0072** | Derived credential chains — C→C slot dependency, scope/audience/TTL narrowing invariant, cascade revoke (Day 9 R2) | core team (Joe Beda / Colm MacCárthaigh advisory) | Q4 2026 |
| **0073** | OAuth interactive flow security — mandatory CSRF `state` + PKCE S256, pending-store TTL/single-use (Day 9 R2) | core team (Filippo Valsorda advisory) | Q4 2026 |
| **0074** | `nebula-credential` audience split — thin contract / `-builtin` / engine-internal; remove `credentials/` dup; ADR-0035 phantom-shim revisit vs Rust 1.95 (Day 9 R2-B) | core team (matklad / Niko advisory) | Q3-Q4 2026 |
| **0075** | Bevy-path many-crate discipline — two-layer umbrella (`nebula-sdk`/`nebula-internal`), single-version lockstep, one-crate-one-responsibility, `nebula-dylib` dev-iteration, `nebula-macro-utils`; kill `nebula-metadata`/`storage-loom-probe`-as-crate; supersedes the consolidation proposal (Day 9 audit + Bevy correction) | core team (Cart advisory) | Q3 2026 |
| **0076** | `nebula-value` v2 — Foundation-zero lean value substrate (Decimal+precision/scale, Bytes, IndexMap, `NotNan` Float, `Arc<[Value]>`, no Secret/Redacted, durable=bytes+codec_id); ТЗ archived — see `docs/ARCHIVE.md` (nebula-value v2 design, Day 9 Round V) | core team | Q3-Q4 2026 |

---

## 11A. Day 9 Charter Amendments (2026-05-15)

> Extends §11 Decision log. Synthesis of the Day 9 re-revision
> session ([CONFERENCE-DAY9.md](./CONFERENCE-DAY9.md)). The user
> invoked a from-zero Charter revision; the panel initially
> echo-validated, then echo-cut — both corrected after user
> challenge. These amendments are **binding decisions**; §3 keeps
> its F-numbering (footnote discipline, RC-13), new principles
> appended as F21+.
> Companion ТЗ:
> Nebula-value v2 design (archived — see [`docs/ARCHIVE.md`](./ARCHIVE.md)).

### 14.1 New principles (appended to §3)

> **F21 — Single Validation Language.** `nebula-validator`'s `Rule` /
> `Predicate` / `Logic` AST is the only language for validation,
> conditional visibility, and dependent-field logic across the whole
> stack. `#[field(when_*)]` attributes are sugar that lowers to it;
> no parallel mini-language.

> **F22 — Schema Immutability per KEY+Version.** Schema attached to
> a `(KEY, version)` is immutable for that version's lifetime. An
> upstream API change is a new Action / major-KEY bump, never a
> silent schema mutation. Already enforced by
> `nebula-metadata::validate_base_compat`; home moves to `schema`
> when `metadata` is dissolved (F26).

> **F23 — Schema is a transport contract.** JSON-serializable schema
> export exists for the client to render forms, toggle conditional
> visibility, and run synchronous validation. Async/custom
> validators run server-side at submit; server always re-validates.
> JSON-Schema-2020-12 compatibility is a side benefit, not a goal;
> `$schema` is `https://nebula.dev/schema/v1`; `x-nebula-*`
> annotations are mandatory. (Rewores the old F11 "build on JSON
> Schema" framing.)

> **F24 — Closed-set extension surfaces are absolute.** No
> `Widget::Custom`, no `Field::Custom`, no escape hatches. Missing
> variants enter the closed enum via ADR amendment + minor bump,
> gated by a universal-applicability review. (Hardens F12/F16; the
> `Field::Map` 14th variant, ADR-0069, is the canonical example of
> a disciplined addition.)

> **F25 — No stdlib newtype zoo.** `nebula-schema` ships no
> predefined `Email`/`Url`/`Cron`/etc. Universality over
> convenience (RFC-variant flame wars). The closed `InputHint` enum
> carries standardized rendering + sync-validation; authors write
> their own newtype when a domain warrants. (Deletes the old F13;
> ADR-0062 Superseded.)

> **F26 — Credential security model.** (a) Credential is a
> dependency (`#[require]`), never a `Field`. (b) The secret is
> reachable only inside `Resource::create`, never Action code, never
> the configurator; not hidden from library code that legitimately
> needs it. (c) `allowed_destinations` is a mandatory part of the
> credential type (audience binding, deny-by-default). (d)
> `bindable_scope` gates who may bind it (RBAC, deny-by-default
> namespace-local). (e) Derived credentials narrow: scope/audience/
> TTL ⊆ parent; revoke cascades parent→derived. Untrusted code is
> contained by Charter §8 tiers, not by hiding the secret.

> **F27 — Foundation-zero value substrate.** `nebula-value` is the
> lowest layer, zero `nebula-*` deps except `nebula-error`; all
> crates depend on it, it on none. Lean (~2-4k LOC), serde-interop
> first, durable persistence = `(opaque bytes, codec_id)` decoupled
> from in-execution `Value` evolution. (Reverses the Feb-2026
> rollback to `serde_json::Value`; ADR-0076.)

> **F28 — Bevy-path many-crate discipline.** Many focused crates are
> endorsed iff: (a) a two-layer umbrella (`nebula-sdk` thin stable
> face / `nebula-internal` feature-wiring) hides the split; (b)
> single-version lockstep release; (c) one crate = one
> responsibility (bloated crates split, junk-common crates killed);
> (d) `dynamic_linking` (`nebula-dylib`) keeps dev iteration fast.
> Crate count is not the metric; discipline is.

### 14.2 Reworded / deleted F-principles

- **F11** — reworded by **F23** (transport contract, not "build on
  JSON Schema; import best-effort").
- **F12 / F16** — hardened by **F24** (absolute closed-set, no
  escape hatches; `Widget::Custom` rejected).
- **F13** — **deleted**, replaced by **F25** (no stdlib zoo).
  F-numbering gap kept with footnote (RC-13), not renumbered.

### 14.3 nebula-value v2 (Round V)

Option D ratified on primary-source evidence (Polars/DataFusion/
bson/VRL/CEL/Restate — every richer-than-JSON Rust project ships a
custom value enum). Decisions: `rust_decimal` + explicit
`(precision, scale)`; `Arc<[Value]>`; `IndexMap` (insertion order,
T13); `Float = NotNan<f64>` + `Ord`; no `Value::Secret` and no
`Value::Redacted` (redaction is a serializer+schema concern;
Interlude II keeps secret out of `Value`); `#[non_exhaustive]`
(future Arrow B-02). Full ТЗ in the companion spec.

### 14.4 Credential crate architecture (Round 2-B)

`nebula-credential` (70 files, ~7 responsibilities) is anti-Bevy
bloat → split to a thin contract (~15 author-facing types);
`provider/` integrator-opt-in; `rotation/`/`store/`/`pending` →
engine-internal; `credentials/` dup removed (built-ins only in
`-builtin`); ADR-0035 phantom-shim revisited vs Rust 1.95 RPITIT
(idiom currency). `SchemeFactory` (not `SchemeGuard`) for
long-running actions; journal carries `CredentialRef`, never
material.

### 14.5 Decomposition: Bevy-path adopted

The "26 → 13 consolidation" audit is **withdrawn**. Most merges
(plugin/sandbox→engine, workflow+execution, observability-trio,
credential-vault, core+error, schema-triad) retracted — these are
valid focused crates on the Bevy/Tokio umbrella model. Surviving
structural work: kill `nebula-metadata` (junk-common; `CatalogLeaf`
trait → `core`, compat → `schema`), `storage-loom-probe` →
`storage/tests/` cfg(loom), split bloated `nebula-credential`
(§14.4). Add service crates `nebula-internal`, `nebula-dylib`;
`sdk/macros-support` → `nebula-macro-utils`. Single-version
lockstep policy adopted. ALT-A/B/E (phase decomposition, core+
opt-in, drop-traits) recorded as roads not taken.

### 14.6 ADR ledger (Day 9)

Accepted/seeded: 0066 (Action marker), 0067 (schema blanket impls/
LintWarnings), 0068 (`-credential-http`), 0069 (`Field::Map`),
0070 (audience binding — **v1.0 gate**), 0071 (binding scope —
**v1.0 gate**), 0072 (derived chains), 0073 (OAuth interactive
security), 0074 (credential crate split), 0075 (Bevy-path
discipline), 0076 (`nebula-value` v2).
Status changes: **ADR-0062 Superseded** (stdlib zoo rejected, F25).
**ADR-0063 narrowed** to export-only "Schema Transport Contract"
(F23; import direction dropped). **ADR-0054 (typed capabilities)
deferred → v2.0 confirmed** (sub-traits + audience + binding scope
+ delegation narrowing = sufficient multi-axis typed authz for
v1.0).

### 14.7 Process / open

- **v1.0 release gates**: ADR-0070 + ADR-0071 (credential security
  model) + a Trail of Bits / NCC review of derived chains + audience
  + binding scope + secret model — **before** v1.0, not follow-up
  (Bryan Cantrill).
- **Day 8 reckoning debt still open**: F1-F6 not yet prepended to
  §3; F-drift footnotes (RC-5/RC-6) unapplied. Day 9 appends F21-F28
  without resolving the F1-F6 gap — a single §3 consolidation pass
  (Day 8 RC + Day 9) is owed.
- **Process commitment (reaffirmed)**: simulated panels must seat
  radical critics that attack the architecture and propose
  alternative decompositions; never echo-validate. (Recorded after
  the user caught the panel doing exactly that.)

---

## 12. Glossary

- **Action** — a typed unit of work that can be invoked by the engine.
  Marker trait; specific shape via sub-trait.
- **Sub-trait shape** — `StatelessAction`, `StatefulAction`, `TriggerAction`,
  `ResourceAction`, `ControlAction`, plus DX specializations
  (`PaginatedAction`, `BatchAction`, `WebhookAction`, `PollAction`).
- **Outcome** — per-shape return enum. `StatelessOutcome`,
  `StatefulOutcome`, `ControlOutcome`, `TriggerEventOutcome`.
- **OutputEnvelope** — universal carrier for action output + metadata
  (cost, tokens, timing, origin, etc.).
- **Capability** — type-encoded permission. Action declares required
  capabilities in its signature; compiler enforces.
- **IsolationLevel** — `None | CapabilityGated | Isolated`. Routes to one
  of the three security tiers (Section 8).
- **Plugin** — ordinary cargo crate exposing `register_into(&mut
  ActionRegistry)`.
- **Integrator** — engineer who builds a deployment binary by combining
  `nebula-sdk` + plugin crates + `EngineBuilder`.
- **Plugin author** — engineer who writes one or more actions, distributes
  via crates.io / git / private registry.

---

## 13. Acknowledgements

This charter synthesizes design input from the May 2026 simulated design
sessions documented in [CONFERENCE-NOTES.md](./CONFERENCE-NOTES.md).
Reactions attributed to named individuals are *literary approximations* of
their publicly known stances and writing — not direct quotations. The
synthesis represents one team's interpretation of how those public positions
might apply to Nebula, not personal endorsements.

**Design input contributors** *(alphabetical, four sessions combined)*:

Aaron Turon · Adrián Barreau · Alice Ryhl · Andy Pavlo · Bryan Cantrill ·
Carl Lerche · David Tolnay (dtolnay) · Esteban Küber · Fabrice Bellard ·
Greg Brockman · Harrison Chase · Jan Oberhauser · Jeremiah Lowin · Jerry
Liu · Joao Moura · Linden Tibbets · Matei Zaharia · Maxim Fateev · Maxime
Beauchemin · Mike Perham · Mitchell Hashimoto · Nick Schrock · Niko
Matsakis · Pat Hickey · Sam Scott · Stephan Ewen · Wes McKinney ·
withoutboats · Yoshua Wuyts.

Charter ratification: pending team approval. Review cycle: every 6 months
or upon major roadmap milestone slip.

---

*End of charter.*
