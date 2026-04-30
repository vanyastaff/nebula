---
id: 0043
title: dependency-declaration-dx
status: accepted
date: 2026-04-29
supersedes: []
superseded_by: []
tags: [action, resource, credential, schema, macro, slot, m11, dependency]
related:
  - .ai-factory/plans/m6-resource-finalization-integration-audit.md
  - docs/adr/0042-node-binding-mechanism.md
  - docs/adr/0044-supersede-0036-resource-credential-singular.md
  - docs/adr/0045-eventtrigger-scope-deferral.md
---

# 0043. Dependency declaration DX (slot binding + Variant A trait + FromWorkflowNode)

## Context

`nebula-action`, `nebula-resource`, `nebula-credential` declare dependencies
through a heterogeneous mix of patterns today:

- `DeclaresDependencies::dependencies()` builder method
  (`crates/core/src/dependencies.rs:44`) — hand-rolled per impl, drift-prone.
- `Resource::Credential` associated type (ADR-0036) — singular, multi-cred
  resources need internal `CredentialStore` lookups.
- `CredentialContextExt::credential::<S>()`
  (`crates/action/src/context.rs:637-643` in external Strategy doc analysis) —
  type-name-derived lookup key, exposes a documented S-C2 cross-plugin shadow
  attack.
- `ctx.resources().acquire_any(&key)` (`crates/core/src/accessor.rs:13`) —
  type-erased `Box<dyn Any>` acquire path, runtime downcast.

Sessions 2026-04-29 ran an extended design dialogue exploring three
alternatives (slot-binding via struct fields; attribute-only `#[uses_credential]`
flags; trait-method declarations). The dialogue informed but did not adopt
external `C:\Users\vanya\RustroverProjects\docs\adr\0038..0041` — those
external ADRs reflect a parallel design path that is informative-not-authoritative
per user direction (ADRs are point-in-time, `feedback_adr_revisable.md`).

The user directed independent reasoning grounded in current Rust 1.95+ idioms
and three concrete pain points: API divergence between crates, underuse of
type-system features for safety, no compile-time enforcement that
`ctx.resource::<R>()` was actually declared.

## Decision

Adopt the **v4 dependency declaration architecture**:

### 1. `#[derive(X)]` + `#[x(...)]` helper attribute (serde-style)

Single canonical entry point for all three crates:

```rust
#[derive(Action)]
#[action(key = "telegram.send", version = "1.0", input = SendTelegramInput, output = MessageId)]
struct SendTelegram { /* slot fields */ }

#[derive(Resource)]
#[resource(key = "postgres", topology = "pool", config = PostgresConfig)]
struct Postgres { /* credential slot fields */ }

#[derive(Credential)]
#[credential(key = "telegram_bot", scheme = "bearer", properties = TelegramProperties)]
struct TelegramCredential;   // typically unit struct
```

`#[action]` *attribute* macro (without `derive`) is reserved for advanced
cases requiring field-type rewriting per ADR-0035 phantom-shim
(`CredentialRef<dyn Bearer>` → `CredentialRef<dyn BearerPhantom>`). Estimated
~5 % of plugin authors will hit dyn-trait positions; the remaining 95 % stay
on derive form.

### 2. Per-field slot attributes

```rust
struct SendTelegram {
    #[resource(key = "bot", purpose = "Bot API client")]
    bot: ResourceGuard<TelegramBot>,

    #[credential(key = "auth", purpose = "Bot API auth")]
    token: CredentialGuard<TelegramCredential>,
}
```

Per-field locality (declaration next to type) is preferred over zone-based
declaration (`#[action(credentials(slot: Type), resources(slot: Type))]`)
because:

- richness of attribute keys (`key`, `purpose`, `scope`) reads naturally on
  individual fields;
- IDE / grep navigation stays local — jump to field, see deps;
- macro complexity is bounded — parse field attrs in one pass.

### 3. Type-based optional + lazy via composable wrappers

| Field type                                    | Semantics             |
| --------------------------------------------- | --------------------- |
| `ResourceGuard<R>`                            | required + eager      |
| `Option<ResourceGuard<R>>`                    | optional + eager      |
| `Lazy<ResourceGuard<R>>`                      | required + lazy       |
| `Option<Lazy<ResourceGuard<R>>>`              | optional + lazy       |

The macro detects wrapper types by path-tail matching (reusing the
infrastructure in `crates/sdk/macros-support/src/credential_ref.rs`). No
`optional` / `lazy` flag in attributes — the field type carries semantics.
This is the "use Rust type system to its fullest" principle that the user
called out as a pain point.

`Lazy<X>` lives in `nebula-core`, async-aware (built on
`tokio::sync::OnceCell<X>`). `ResourceGuard<R>` and `CredentialGuard<C>` are
existing RAII guards in their respective crates.

### 4. Variant A trait shape — `type Input + Output` on base `Action`

```rust
pub trait Action: Sized + Send + Sync + 'static {
    type Input: HasSchema + DeserializeOwned + Validate + Send + Sync;
    type Output: HasSchema + Serialize + Send + Sync;

    fn metadata() -> &'static ActionMetadata;
    fn input_schema() -> &'static ValidSchema;     // = Self::Input::schema()
    fn output_schema() -> &'static ValidSchema;    // = Self::Output::schema()
    fn dependencies() -> &'static Dependencies;    // slot fields
}

pub trait StatelessAction: Action { /* execute(&self, input, ctx) */ }
pub trait StatefulAction: Action { type State; /* execute(&self, input, &mut state, ctx) */ }
pub trait TriggerAction: Action { /* start(&self, input, scheduler, ctx) + stop(&self) */ }
pub trait ResourceAction: Action<Output = ResourceProduces<<Self as ResourceAction>::Resource>> {
    type Resource: Resource;
    /* configure / cleanup with input arg */
}
```

All four sub-traits have meaningful `Input` and `Output`:

- **Stateless / Stateful** → `Input` per-execution data; `Output` per-execution result payload.
- **Trigger** → `Input` is the trigger configuration (webhook URL, secret,
  filter, allowed event types); `Output` is the event payload type published
  when the trigger fires.
- **Resource** → `Input` is the scoped-resource configuration (tenant id,
  schema name, pool size); `Output` is `ResourceProduces<R>` (a marker type
  for catalog/UI to draw scoped-binding edges).

### 5. Self holds slots only; `<Name>Input` is a separate struct

`Self` type holds only slot fields (resources, credentials). Per-execution /
per-trigger / per-scope user-form data lives in a separate `Self::Input`
struct authored via `#[derive(Schema)]` (decision 6 below).

```rust
#[derive(Action)]
#[action(key = "telegram.send", input = SendTelegramInput, output = MessageId)]
struct SendTelegram {
    #[resource(key = "bot")]    bot: ResourceGuard<TelegramBot>,
    #[credential(key = "auth")] token: CredentialGuard<TelegramCredential>,
    // No #[field] fields on Self
}

#[derive(Schema, Deserialize)]
struct SendTelegramInput {
    #[field(label = "Chat ID")]
    #[validate(required)]
    chat_id: i64,

    #[field(label = "Text")]
    #[validate(required, length(max = 4096))]
    text: String,

    #[field(label = "Reply to")]
    reply_to: Option<i64>,
}

impl StatelessAction for SendTelegram {
    async fn execute(&self, input: SendTelegramInput, ctx: &impl ActionContext)
        -> Result<ActionResult<MessageId>, ActionError>
    {
        let id = self.bot.send(input.chat_id, &input.text, &self.token).await?;
        // self.text → compile error: no field `text` on `SendTelegram`
        // input.text → typed access — single source of truth
        Ok(ActionResult::ok(id))
    }
}
```

The "Self IS Input" mixed-struct alternative was rejected because it surfaces
`self.text` *and* `input.text` ambiguity (or forces complicated serde
`#[serde(skip)]` games on slot fields). Separation makes per-field semantics
explicit at the type system level.

### 6. Unified `#[derive(Schema)]` (single namespace, no role flag)

One derive for all schema-producing structs (Action `<Name>Input`,
ResourceConfig, CredentialProperties, etc.). The macro emits a `HasSchema`
impl; serde `Deserialize` and `nebula_validator::Validator` stay separate
derives so authors can layer `#[serde(...)]` and `#[validator(message = ...)]`
attributes idiomatically.

```rust
#[derive(Schema, Deserialize)]
struct SendTelegramInput {
    #[field(label = "Chat ID")]
    #[validate(required)]
    chat_id: i64,

    #[field(label = "Text")]
    #[validate(required, length(max = 4096))]
    text: String,

    #[field(label = "Reply to")]
    reply_to: Option<i64>,
}

#[derive(Schema, Deserialize)]
struct PostgresConfig {
    #[field(label = "Host")]
    #[validate(required)]
    host: String,
    #[field(label = "Port", default = 5432)]
    port: u16,
}

#[derive(Schema, Deserialize)]
struct TelegramProperties {
    #[field(secret, label = "Bot token")]
    token: SecretString,
    #[field(label = "Refresh URL")]
    refresh_url: Option<String>,
}
```

Field-level attribute namespaces stay aligned across every Schema-derived
type — no role gating, no per-role attribute split:

- `#[field(label, description, placeholder, default, hint, group, secret,
  multiline, no_expression, expression_required, enum_select, skip)]` —
  schema metadata (`secret` flag forces `SecretString` mapping).
- `#[validate(required, length(min, max), range(min..=max), pattern, url,
  email)]` — value rules feeding `ValidSchema::validate(...)`.

`#[derive(serde::Deserialize)]` provides field-level `#[serde(rename, default)]`
ergonomics; `#[derive(nebula_validator::Validator)]` (when needed) provides
the `Validate<Self>` trait via its own `#[validate(...)]` parser. The two
`#[validate(...)]` namespaces are scoped to whichever derive consumes them
(serde-style: each derive parses only its own attribute keys).

### 7. `FromWorkflowNode` async factory pattern

```rust
pub trait FromWorkflowNode: Sized + Send + 'static {
    type Error: Send;

    fn from_workflow_node<'a>(
        node: &'a NodeDefinition,
        ctx: &'a dyn ActionContext,
    ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a;
}
```

`#[derive(Action)]` emits `FromWorkflowNode` for the action struct. The
generated body resolves each slot field per ADR-0042's binding mechanism.

Engine dispatch becomes:

```rust
// Per execution:
let action = SendTelegram::from_workflow_node(node, ctx).await?;          // slots resolved
let input: SendTelegramInput = serde_json::from_value(node.input_json)?;
let values = FieldValues::from_struct(&input)?;                           // FieldValues bridge
SendTelegramInput::schema().validate(&values)?;                           // value-rule check
let result = action.execute(input, ctx).await?;
```

A factory `Box<dyn ErasedAction>` is allocated per execution; alloc cost is
trivial against network / DB calls in the action body.

### 8. `ResourceProduces<R>` marker for ResourceAction Output

```rust
pub struct ResourceProduces<R: Resource> {
    pub resource_key: &'static ResourceKey,
    pub topology: Topology,
    _phantom: PhantomData<R>,
}

impl<R: Resource> HasSchema for ResourceProduces<R> {
    fn schema() -> ValidSchema { ValidSchema::resource_marker::<R>() }
}
```

ResourceAction's Output is constrained at trait level
(`Action<Output = ResourceProduces<<Self as ResourceAction>::Resource>>`) —
catalog / UI code can introspect what scoped resource each ResourceAction
produces and draw the correct workflow-edge.

### 9. New runtime types

| Type                        | Crate              | Role                                 |
| --------------------------- | ------------------ | ------------------------------------ |
| `CredentialRef<C: ?Sized>`  | nebula-credential  | id + PhantomData; `.resolve(ctx)` returns `CredentialGuard<C>` |
| `ResourceRef<R: ?Sized>`    | nebula-resource    | id + PhantomData; `.resolve(ctx)` returns `ResourceGuard<R>`  |
| `Lazy<X>`                   | nebula-core        | async-aware lazy wrapper (`tokio::sync::OnceCell<X>`)         |
| `ResourceProduces<R>`       | nebula-action      | ResourceAction Output marker         |

Existing `ResourceGuard<R>` (nebula-resource) and `CredentialGuard<C>`
(nebula-credential) stay — they are the resolved-RAII forms used as field
types. `*Ref<*>` are used internally and as `Lazy<*Ref<*>>` wrappers.

### 10. Macro inventory

| Macro                  | Crate                       | Role                                                |
| ---------------------- | --------------------------- | --------------------------------------------------- |
| `#[derive(Action)]`    | nebula-action/macros        | Self struct + slot fields + meta attribute          |
| `#[derive(Resource)]`  | nebula-resource/macros      | Self struct + credential slots + config attribute   |
| `#[derive(Credential)]`| nebula-credential/macros    | Self struct (often unit) + properties attribute     |
| `#[derive(Schema)]`    | nebula-schema/macros        | Universal — Input / Config / Properties via role flag |
| `#[action]` attribute  | nebula-action/macros        | Advanced — only for `CredentialRef<dyn Cap>` phantom-shim |

Total: 5 macros. `trybuild` + `macrotest` regression coverage mandatory for
each (Phase 11 verification gate).

## Consequences

### Positive

- **Single declaration path** for slot deps eliminates API divergence between
  `nebula-action` / `nebula-resource` / `nebula-credential`.
- **Type-system enforcement** of optional / required / lazy distinctions via
  `Option<X>` / `Lazy<X>` wrappers — no runtime flag interpretation.
- **`ctx.resource::<R>()` shadow attack class (S-C2) closed** by replacing
  type-name lookup with explicit ID-based `ctx.acquire_resource_by_id::<R>(id)`
  helper. Slot fields make IDs explicit.
- **Macro complexity bounded.** Reuse of `crates/sdk/macros-support` path-tail
  matching for wrapper detection avoids reinventing wheel.
- **Composes with credential П1 capability sub-trait pattern** —
  `#[derive(Credential)]` macro detects Interactive / Refreshable / Revocable /
  Testable / Dynamic from impl blocks. ADR-0035 phantom-shim composition
  preserved through `#[action]` attribute escape hatch.
- **Per-execution factory alloc cost trivial** vs network / DB calls; engine
  dispatch refactor is an additive trait (`FromWorkflowNode`) atop existing
  registry plumbing.

### Negative

- **5 macros + 1 attribute macro to ship + maintain.** Each gets `trybuild` +
  `macrotest` coverage; testing matrix grows. Mitigation: macros share the
  `crates/sdk/macros-support` infrastructure for type-rewrite probes.
- **Two structs per action** (Self + Input) — verbose by single-line metric,
  but clear-by-design. Authors get destructuring-let to recover ergonomics
  (`SendTelegramInput { chat_id, text, .. }`).
- **`Resource::Credential` associated type dropped** (ADR-0044 supersession of
  ADR-0036). Hard break: every Resource with credentials migrates to slot
  fields. Codemod ships in cascade.
- **Engine dispatch reshape** — `Arc<dyn ActionHandler>` → `Arc<dyn ActionFactory>` +
  `Box<dyn ErasedAction>` per execution. Validates against the existing
  `crates/engine/tests/resource_integration.rs` smoke path.

### Follow-up work

- ADR-0044 supersedes ADR-0036 (Resource::Credential singular → slot fields).
- ADR-0045 defers EventTrigger DX wrapper (candidate ROADMAP §M6.4).
- Phase 1-5 of `m6-resource-finalization-integration-audit.md` implement the
  runtime types, macros, and engine dispatch refactor.
- Cross-link from external reference docs at
  `C:\Users\vanya\RustroverProjects\docs\adr\0035..0041` — informational only;
  not load-bearing on this ADR.

## Alternatives considered

### Alternative A — Keep `#[derive(Action)]` with adjacent emission (current pattern)

Rejected: derive cannot perform field-type rewriting, so dyn-trait
phantom-shim composition (ADR-0035) is structurally absent. Plugin authors
hit `dyn Bearer` cases and have no path forward without parallel-vocabulary
debt.

### Alternative B — Attribute macro `#[action]` always (replace derive)

Rejected: 95 % of plugin authors do not need field rewriting. Forcing
attribute-macro form on every action raises macro complexity for the
common case. `#[derive(Action)]` is the established Rust pattern (serde,
diesel, clap) and authors are familiar with it. Reserve `#[action]` for
the dyn-trait minority.

### Alternative C — Mixed struct (Self IS Input, `#[field]` on Self)

Rejected: surfaces `self.text` *and* `input.text` ambiguity, forces
custom serde skip logic on slot fields, mixes "action infra" and "user
data" concerns at the type level. Separation eliminates a class of bugs
(synchronization between two access paths).

### Alternative D — Function-based actions (axum extractor pattern)

Rejected: too radical for current scope. Would require multi-function modules
for trigger lifecycle (start / stop / fire). Plugin discovery currently uses
concrete types; extractor-functions need wrappers. Reconsider if a future
cascade has bandwidth.

### Alternative E — `optional` / `lazy` flags in attribute (not type-based)

Rejected: type IS the declaration. `#[resource(key = "bot", optional)]` on a
required `ResourceGuard<TelegramBot>` field is a paradox the macro must
either reject (more code) or honor (silent correctness loss). `Option<X>` /
`Lazy<X>` wrappers express the same intent natively.

### Alternative F — External ADR-0038/0039 zone-based attribute syntax

Reviewed: external `C:\Users\vanya\RustroverProjects\docs\adr\0038..0039`
(FROZEN CP3 2026-04-25, not imported into nebula repo) propose
`#[action(credentials(slot: Type), resources(slot: Type))]` zone-based
declaration. Per-field declaration was preferred during this design dialogue
because:

- richness of attribute keys reads naturally on individual fields;
- locality of declaration matches Rust idiom (serde, ts-rs, derive_more);
- macro complexity for zone-form scales worse with N slots.

The external ADRs' insights informed but did not bind this ADR per
`feedback_adr_revisable.md` and explicit user direction.

## Seam / verification

- **Trait shape lock.** `nebula-action::Action` trait moves to Variant A in
  Phase 3.1. Old base `Action` trait deleted (no shims per
  `feedback_no_shims.md`).
- **Macro emission point.** `crates/{action,resource,credential,schema}/macros/`
  ships `#[derive(X)]` + helper attribute parsing. Each macro gets
  `trybuild` + `macrotest` regression coverage with ≥6 probes per
  external `2026-04-24-nebula-action-tech-spec.md` §16.1 lineage.
- **Runtime types.** `CredentialRef<C>`, `ResourceRef<R>`, `Lazy<X>`,
  `ResourceProduces<R>` land in Phase 1.
- **Engine dispatch.** `ActionFactory` + `ErasedAction` registry shape lands
  in Phase 3.5.
- **Test gate.** `crates/engine/tests/resource_integration.rs` extended with
  end-to-end factory-dispatched action execution; existing
  `ResourceProbeHandler` test continues to pass through the new dispatch path.
