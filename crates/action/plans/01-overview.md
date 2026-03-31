# nebula-action v2 — High-Level Design

> **Revision 5 (frozen)** — 5 rounds × 3 models (ChatGPT, DeepSeek, Gemini), triaged by Claude.
> 15 reviews, ~70 accepted changes, 0 architectural blockers remaining.
>
> **Post-freeze additions:**
> - WebhookAction: check_exists() / on_activate() / on_deactivate() lifecycle hooks
>   (n8n webhookMethods parity for external webhook registration/unregistration)
> - RawWebhookAction: same lifecycle hooks
> - PollAction: poll_interval(ctx), validate(), on_poll_error(), emit_mode()
> - ScheduledTrigger: new DX type for cron/interval/one-time
> - Slim context / rich resources: BinaryStorage and StreamOutput moved from
>   ActionContext methods to core resources (via resource_typed)
> - Action versioning: InterfaceVersion per action, VersionedActionKey registry,
>   ActionMigration trait for parameter migration, DeprecationInfo, UI upgrade flow
> - Aligned with existing codebase: BinaryData (inline/stored), WaitCondition naming,
>   ActionResult (alternatives, main_output), OutputEnvelope
> - Port system documented: ActionResult→Port routing contract, multi-input semantics,
>   Support port data flow, Dynamic port resolution, port patterns per node type
> - DataTag system: optional typed wiring (ComfyUI-mode), editor-only enforcement,
>   runtime never blocks, DataTagRegistry for validated tag registration
>
> **Final status:** Approved by all three reviewers.
> - ChatGPT: Approve with nits (wording polish)
> - Gemini: Unconditional approve (10/10)
> - DeepSeek: Approve (all blockers closed)
>
> **Remaining risk:** Implementation fidelity and runtime/resource integration quality.

---

## 1. Overview

nebula-action определяет **что** такое action и **как** action общается с engine.
Это protocol crate — не runtime. Он должен быть маленьким, стабильным, и явным.

### What it solves

Каждый workflow node в Nebula — это action. Action принимает типизированный вход,
выполняет работу (API вызов, трансформация данных, запрос к БД), и возвращает
типизированный результат с flow-control intent. Без nebula-action каждый автор node
сам определяет lifecycle, error handling, parameter binding, и capability model.

| Потребитель | Использует | Получает |
|-------------|-----------|----------|
| Action author | Trait (StatelessAction и др.) | Type-safe execute с flow control |
| Engine | ActionResult + ActionInstance | Deterministic orchestration decisions |
| Runtime | ActionContext + Executor pipeline | Capability injection, middleware |
| Sandbox | CapabilityManifest + ExecutionGuard | Least-privilege + lifetime enforcement |
| UI/Editor | ActionDescriptor + ActionMetadata | Node catalog, parameter forms, port wiring |
| Plugin system | ActionDescriptor + ActionFactory | Discovery + instantiation |

```rust
// Production action — 20 строк с DX helpers
#[derive(Action)]
#[action(key = "http.request", name = "HTTP Request", category = "network")]
#[credential(BearerToken, key = "api_token")]
#[resource(HttpClient, key = "http")]
struct HttpRequest;

#[derive(ActionInput, Deserialize)]
struct HttpRequestInput {
    #[param(label = "URL")]
    url: String,
    #[param(label = "Method", one_of("GET", "POST"), default = "GET")]
    method: String,
}

impl SimpleAction for HttpRequest {
    type Input = HttpRequestInput;
    type Output = serde_json::Value;

    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> Result<Self::Output, ActionError>
    {
        let deps = HttpRequestDeps::resolve(ctx).await?;
        let resp = deps.http.get(&input.url)
            .bearer_auth(deps.api_token.access_token())
            .send().await.retryable()?;
        resp.json().await.fatal()
    }
}
```

### Core guarantees

1. **Type-safe contracts** — каждый action type имеет свой trait с compile-time
   гарантиями Input/Output/State типов.

2. **Explicit flow control** — ActionResult enum с exhaustive matching. Compiler
   заставляет engine обработать каждый вариант.

3. **Separation of concerns** — ActionResult = flow control (что делать дальше),
   ActionOutput = payload form (как передать данные).

4. **Execution-bounded context** — ActionContext **не клонируемый**. Accessors
   защищены `ExecutionGuard` — после completion/cancel любой accessor call
   возвращает ошибку. Action запрашивает ресурсы по **локальным алиасам**, runtime
   маппит на глобальные ID.

5. **Protocol, not runtime** — crate не содержит scheduling, retry engine, state
   storage, parameter binding pipeline. Только traits, types, contracts.
   **Note:** Blanket impls for DX types (PollAction, EventTrigger, ScheduledTrigger)
   contain minimal `tokio` dependency (`tokio::select!`, `tokio::time::sleep`) for
   cancellation and timing. Core traits (`TriggerAction`, `StatelessAction`) have
   zero runtime dependencies. This is intentional — blanket impls are thin adapters.

6. **Two-level type system** — Core types (engine различает: Stateless, Stateful,
   Trigger, Resource) и DX types (удобные обёртки). DX types — convenience over core.

7. **Deterministic error classification** — ActionError варианты определяют retry
   policy на уровне типа. ErrorCode enum для structured classification.

8. **Durable state contract** — StatefulAction state persisted BEFORE any routing
   side effects. Crash = state recoverable from pre-execute snapshot.

9. **Panic safety** — panic in action code caught by runtime, classified as Fatal,
   no routing side effects, state rolled back.

10. **Resource-backed triggers** — triggers consume managed resources from
    nebula-resource (EventSource, Daemon topologies). Connection management,
    reconnection, health — all handled by resource layer, not trigger author.

### System boundaries

| Concern | Owner | Integration |
|---------|-------|-------------|
| Action trait definition | nebula-action | Core + DX traits |
| Action metadata & policies | nebula-action | ActionMetadata, RetryPolicy, TimeoutPolicy |
| Action descriptor (catalog) | nebula-action | ActionDescriptor trait |
| Action factory (instantiation) | nebula-action | ActionFactory trait |
| Parameter schema definition | nebula-parameter | ParameterCollection в ActionMetadata |
| Parameter binding pipeline | nebula-runtime | Expression → Validate → Transform → Bind |
| Flow control semantics | nebula-action | ActionResult enum |
| Output data forms | nebula-action | ActionOutput enum |
| Error classification | nebula-action | ActionError, ErrorCode |
| Port declarations | nebula-action | InputPort, OutputPort, SupportPort, DynamicPort |
| Capability manifest | nebula-action | CapabilityManifest (derived from components) |
| Execution guard | nebula-action (type) + nebula-runtime (lifecycle) | ExecutionGuard |
| Context construction | nebula-runtime / nebula-sandbox | ActionContext, TriggerContext |
| Sandbox enforcement | nebula-sandbox-* | EnforcedAccessor proxy + ExecutionGuard |
| ActionResult → engine decisions | nebula-engine | Engine's own mapping |
| State persistence | nebula-engine | PersistedState, durable commit contract |
| Panic handling | nebula-runtime | tokio::spawn + JoinError::is_panic(), pre-execute snapshot |
| Credential resolution | nebula-credential | Via CredentialAccessor in context |
| Resource management | nebula-resource | Via ResourceAccessor in context |
| Plugin registration | nebula-plugin | Plugin::actions() → ActionDescriptor |
| Trigger supervision | nebula-runtime | Runtime policy, not action contract |

### What nebula-action does NOT own

| Concern | Why not | Owner |
|---------|---------|-------|
| DAG scheduling | Engine responsibility | nebula-engine |
| Retry backoff calculation | Engine policy | nebula-engine |
| State store implementation | Storage concern | nebula-engine |
| Parameter binding pipeline | Runtime concern | nebula-runtime |
| ActionResult → engine mapping | Engine internal | nebula-engine |
| Connection management | Resource concern | nebula-resource |
| Reconnection / health | Resource + runtime | nebula-resource + nebula-runtime |
| Credential storage/encryption | Security concern | nebula-credential |
| WASM sandbox implementation | Isolation concern | nebula-sandbox-wasm |
| Expression evaluation | Language concern | nebula-expression |

---

## 2. Type Hierarchy (Complete)

```
Action (base trait — metadata)
│
├── StatelessAction       — &ActionContext — Input → Output
│   ├── SimpleAction      — DX: Result<O> → auto Success
│   └── TransformAction   — DX: sync pure fn, no async, no ctx
│
├── StatefulAction        — &ActionContext — Input + &mut State → Output
│   ├── InteractiveAction — DX: human-in-the-loop, epoch-based
│   ├── TransactionalAction — DX: saga compensate
│   ├── PaginatedAction   — DX: auto cursor, Continue/Break
│   └── BatchAction       — DX: auto chunking, Continue/Break
│
├── TriggerAction         — &TriggerContext — start/run/stop/health + state
│   ├── EventTrigger      — DX: event-driven from Resource EventSource
│   ├── WebhookAction     — DX: HTTP webhook, signature, state, lifecycle (check/activate/deactivate)
│   ├── RawWebhookAction  — DX: non-JSON webhooks (form, XML, multipart) + same lifecycle
│   ├── PollAction        — DX: periodic polling, cursor, NextPoll, error policy
│   └── ScheduledTrigger  — DX: cron/interval/one-time (next_fire_time → sleep → emit)
│
└── ResourceAction        — &ActionContext — acquire/release scoped lease
```

**Rule:** All DX types blanket-impl to one of 4 core types. Engine sees only core.
Adding a new core ActionKind = major version bump.

### Base trait

```rust
pub trait Action: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;
}
```
