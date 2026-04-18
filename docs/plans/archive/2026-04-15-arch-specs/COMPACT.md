# Compact Reference — 28 architectural decisions (Q&A session 2026-04-15)

> One-page summary for navigation. Full specs in `./01-*.md` through `./28-*.md`.
> **Authority:** subordinate to `docs/PRODUCT_CANON.md`. Canon wins on conflict.
> Specs 01–20 came from the original Q&A round; 21 (schema), 22 (credential v3),
> 23 (cross-crate foundation), and 24 (nebula-core redesign) were added in
> post-compact expert Q&A rounds.

## Naming (locked)

### Identifiers (spec 06)

| Concept | Name | Prefix | Notes |
|---|---|---|---|
| Organization | `OrgId` | `org_` | Typed newtype from spec 06 |
| Tenant mid-level | `WorkspaceId` | `ws_` | Renamed from `ProjectId` in `nebula-core` |
| Workflow definition | `WorkflowId` | `wf_` | |
| Workflow version | `WorkflowVersionId` | `wfv_` | Spec 13 pinning |
| Execution | `ExecutionId` | `exe_` | |
| Node attempt row | `AttemptId` | `att_` | Renamed from `NodeAttemptId` |
| Workflow graph node ref | `NodeKey` | string | **Renamed from `NodeId`** — `Key<NodeDomain>`, authored string (spec 24) |
| OS process / K8s pod | `InstanceId` | `nbl_` | Renamed from `NebulaInstanceId` |
| Trigger | `TriggerId` | `trg_` | |
| User | `UserId` | `usr_` | |
| Service account | `ServiceAccountId` | `svc_` | |
| Credential instance | `CredentialId` | `cred_` | Spec 22 |
| Resource type | `ResourceKey` | string | Type identifier, not instance ID |

### Context + capability types (spec 23)

| Concept | Name | Location | Notes |
|---|---|---|---|
| Base identity trait | `Context` | `nebula-core::context` | `execution_id()`, `workspace_id()`, `cancellation()`, etc. |
| Identity struct | `BaseContext` | `nebula-core::context` | All shared fields in one struct |
| Scope enum | `ScopeLevel` | `nebula-core::scope` | `Global / Organization / Workspace / Workflow / Execution` |
| Scope identity | `Scope` | `nebula-core::scope` | 9 optional fields (8 `*Id` + 1 `node_key: NodeKey`) |
| Principal enum | `Principal` | `nebula-core::scope` | `User / ServiceAccount / Workflow / System` |
| Capability: resources | `HasResources` | `nebula-core::context` | `fn resources() -> &dyn ResourceAccessor` |
| Capability: credentials | `HasCredentials` | `nebula-core::context` | |
| Capability: logger | `HasLogger` | `nebula-core::context` | |
| Capability: metrics | `HasMetrics` | `nebula-core::context` | |
| Capability: eventbus | `HasEventBus` | `nebula-core::context` | |
| Capability: node identity | `HasNodeIdentity` | `nebula-action::context` | Action-specific |
| Capability: trigger scheduling | `HasTriggerScheduling` | `nebula-action::context` | Trigger-specific |
| Umbrella: action | `ActionContext` | `nebula-action::context` | Blanket impl marker |
| Umbrella: trigger | `TriggerContext` | `nebula-action::context` | Blanket impl marker |
| Internal: credential work | `CredentialContext` | `nebula-credential::context` | Passed to `Credential::resolve/refresh/...` |
| Internal: resource work | `ResourceContext` | `nebula-resource::context` | Passed to `Resource::create/check/...` |
| Concrete: action runtime | `ActionRuntimeContext` | `nebula-engine::context` | Implements `ActionContext` |
| Concrete: trigger runtime | `TriggerRuntimeContext` | `nebula-engine::context` | Implements `TriggerContext` |
| Test context | `TestContext` | `nebula-testing::context` | Implements all capability traits |

### Guard types (spec 23)

| Concept | Name | Location | Notes |
|---|---|---|---|
| Base guard trait | `Guard` | `nebula-core::guard` | `guard_kind()`, `acquired_at()`, `age()` |
| Typed guard subtrait | `TypedGuard` | `nebula-core::guard` | `type Inner`, `as_inner()` |
| Credential RAII wrapper | `CredentialGuard<S: Zeroize>` | `nebula-credential::guard` | `Deref`, zeroize on drop, no Serialize/Display |
| Resource RAII wrapper | `ResourceGuard<R: Resource>` | `nebula-resource::guard` | **Renamed** from `ResourceHandle` |
| Typed access extension | `HasResourcesExt::resource<R>()` | `nebula-resource::ext` | Returns `ResourceGuard<R>` |
| Typed access extension | `HasCredentialsExt::credential<C>()` | `nebula-credential::ext` | Returns `CredentialGuard<C::Scheme>` |

### Dependencies types (spec 23)

| Concept | Name | Location | Notes |
|---|---|---|---|
| Dependency container | `Dependencies` | `nebula-core::dependencies` | Private fields, fluent builder |
| Declaration trait | `DeclaresDependencies` | `nebula-core::dependencies` | Single method, replaces 5-method `ActionDependencies` |
| Credential requirement | `CredentialRequirement` | `nebula-core::dependencies` | `of::<C>()` + `.optional()` + `.purpose()` |
| Resource requirement | `ResourceRequirement` | `nebula-core::dependencies` | Same pattern |
| Derive attribute: single cred | `#[uses_credential(Type)]` | derive macros | Options: `optional`, `purpose = "..."` |
| Derive attribute: bulk creds | `#[uses_credentials([...])]` | derive macros | Array with per-item options |
| Derive attribute: single resource | `#[uses_resource(Type)]` | derive macros | |
| Derive attribute: bulk resources | `#[uses_resources([...])]` | derive macros | |
| Manual helper | `nebula_deps! { ... }` | `nebula-core::dependencies` | Declarative macro alternative |

### Schema types (spec 21)

| Concept | Name | Location | Notes |
|---|---|---|---|
| Schema container | `Schema` | `nebula-schema` | Renamed from `ParameterCollection` |
| Field enum wrapper | `Field` | `nebula-schema` | 18 variants |
| Field key newtype | `FieldKey` | `nebula-schema` | Validated identifier |
| Field value runtime | `FieldValue` | `nebula-schema` | `Literal / Expression / Mode` |
| Field path reference | `FieldPath` | `nebula-schema` | Local + `$root.` absolute |
| 18 per-type structs | `StringField`, `NumberField`, ... , `NoticeField` | `nebula-schema` | Pattern 4 — enum wrapper + structs |
| Required mode enum | `RequiredMode` | `nebula-schema` | `Never / Always / When(Rule)` |
| Visibility mode enum | `VisibilityMode` | `nebula-schema` | `Always / When(Rule)` |
| Widget enums | `StringWidget`, `NumberWidget`, ... | `nebula-schema` | 7 total — only types with real variation |

### Forbidden / deleted patterns

| Deleted | Replaced by | Spec |
|---|---|---|
| `nebula-resource::ResourceHandle<R>` | `nebula-resource::ResourceGuard<R>` | 23 |
| `nebula-resource::Ctx` / `BasicCtx` | `nebula-resource::ResourceContext` | 23 |
| `nebula-resource::ScopeLevel` (local) | `nebula-core::scope::ScopeLevel` | 23 |
| `nebula-action::ActionContext` struct | `nebula-action::ActionContext` trait + `nebula-engine::ActionRuntimeContext` | 23 |
| `nebula-action::ActionDependencies` trait (5 methods) | `nebula-core::DeclaresDependencies` (1 method) | 23 |
| `nebula-credential::retry` facade | `nebula-resilience::retry_with` directly | 22 |
| `nebula-parameter::ParameterCollection` | `nebula-schema::Schema` | 21 |
| `Option<Box<dyn AnyCredential>>` single credential | `Vec<CredentialRequirement>` multi-credential | 23 |
| Credential-to-credential deps | Compile error; use `#[uses_resource(...)]` or external provider | 23 |
| `nebula-core::NodeId` (UUID-backed) | `NodeKey` (`Key<NodeDomain>`, authored string) | 24 |
| `nebula-core::SecretString` + serde helpers | `secrecy` crate + `RedactedSecret` wrapper in `nebula-credential` | 24 |
| `nebula-core::Version` custom struct | `semver::Version` (external crate) | 24 |
| `nebula-core::InterfaceVersion` | Moved to `nebula-action::metadata` | 24 |
| `nebula-core::constants.rs` (entire module) | Deleted — 0 consumers, domain crates own their defaults | 24 |
| `nebula-core::types.rs` (Status, Priority, OperationResult, OperationContext, ProjectType, RoleScope) | Deleted — 0 consumers | 24 |
| `nebula-core::traits.rs` (HasContext, Scoped) | Deleted — replaced by spec 23 Context system | 24 |
| `nebula-core::CoreError` (15 variants) | 5 variants (InvalidId, InvalidKey, ScopeViolation, DependencyCycle, DependencyMissing) | 24 |
| `nebula-core::TenantId` / `RoleId` / `OwnerId` | Deleted — 0 consumers | 24 |

### Naming convention (spec 24 — workspace-wide)

| Pattern | Semantics | Backing | Examples |
|---|---|---|---|
| `FooKey` | Author-defined string | `domain_key::Key<D>` | ActionKey, NodeKey, ResourceKey |
| `FooId` | System-generated ULID | `domain_key::Ulid<D>` | ExecutionId, WorkflowId, OrgId |
| `HasFoo` | Capability trait | nebula-core trait | HasResources, HasCredentials |
| `FooAccessor` | Dyn-safe service trait | nebula-core trait | ResourceAccessor, CredentialAccessor |
| `FooGuard<T>` | RAII wrapper + Drop | Domain crate struct | CredentialGuard, ResourceGuard |
| `FooContext` | Internal domain context | Domain crate struct | CredentialContext, ResourceContext |
| `FooRuntimeContext` | Concrete engine impl | nebula-engine struct | ActionRuntimeContext |
| `DeclaresFoo` | Declaration trait | nebula-core trait | DeclaresDependencies |
| `FooRequirement` | Dep declaration struct | nebula-core struct | CredentialRequirement |

## Decisions 1–24

| # | Topic | Core decision |
|---|---|---|
| **01** | Positioning | D: OSS self-host + managed cloud (n8n-like). Primary: solo/small team. Multi-process ready by design, default single-process. |
| **02** | Tenancy | Org → Workspace two-level. Implicit `default` for self-host. Credentials M2 (workspace-local OR org-level with workspace allowlist). Transitive: OrgOwner/Admin → WorkspaceAdmin. No cross-workspace workflow refs. |
| **03** | Identity & auth | Built-in `nebula-auth` crate on Rust ecosystem (`argon2`, `oauth2`, `tower-sessions`, `totp-rs`, `lettre`). Local dev: `NEBULA_AUTH_MODE=none`. Separate `nebula-diagnostics` for opt-out product telemetry. |
| **04** | RBAC | 4×4 fixed roles. Org: `OrgOwner`/`OrgAdmin`/`OrgBilling`/`OrgMember`. Workspace: `WorkspaceAdmin`/`Editor`/`Runner`/`Viewer`. Service accounts first-class, scheduled workflows run as SA. Credential visibility role-based, not workflow-derived. |
| **05** | API routing | Path-based `/api/v1/orgs/{org}/workspaces/{ws}/...`. Slug OR ULID both accepted. Session cookie on domain, tenant from path. Path-based major versioning. |
| **06** | IDs | Prefixed ULID (Stripe-style). 16 bytes binary in storage, `xxx_01J9...` on wire. Extends `nebula-core::domain_key`. Monotonic generator for hot append paths. |
| **07** | Slugs | `[a-z0-9][a-z0-9-]{0,61}[a-z0-9]`. Nickname model (ULID primary, slug mutable alias). ~300 reserved words. Rename grace: org 90d / ws 30d / wf 7d. Auto-gen via `deunicode`. |
| **08** | Cancellation | Two-phase cooperative + hard kill. Hierarchical `CancellationToken` + `TaskTracker` (process → engine → execution → node). Grace waterfall: 60s > 45s > 30s > 30s. `cancel` vs `terminate` as two RBAC'd actions. Author API: `ctx.cancellation.check()` / `.as_future()` / `ctx.tasks.spawn()`. |
| **09** | Retry | Four-layer cascade: R1 `nebula-resilience` in-action; R2 `ActionMetadata::retry_policy` + `Classify::retryable()` + persisted `node_attempts`; R3 DAG `on_error` edges (no cycles); R4a manual restart. `ActionError::{Transient,TransientWithHint,Permanent,Cancelled,CancelledEscalated,Fatal,Timeout}`. Cancel wins over retry. `idempotency_key = {exec}:{node}:{attempt}`. |
| **10** | Timeouts/quotas | Four concepts: timeout (wall-clock per op), budget (retry total), quota (tenant-level, atomic CAS), rate-limit (`governor` in-memory, spike protection). Node attempt 5m, retry total 30m, execution 24h, stateful max 7d. Fair scheduling WRR by workspace. |
| **11** | Triggers | `TriggerAction` base + `PollingAction`/`WebhookAction`/`EventAction` specializations. Cron: `overlap=Skip`, `catch_up=Skip`, `jitter=30s` defaults. Webhook: dedup via `UNIQUE (trigger_id, event_id)`, auth presets (HMAC/Stripe/Bearer/mTLS/IP), `AcknowledgeAndQueue` default. Queue: plugin-based, queue-native offset commit. |
| **12** | Expression | CEL-inspired, non-Turing-complete. Two surfaces: expression + template `"Hello ${x}"`. `EvalContext` whitelist: `input`, `nodes`, `trigger`, `vars`, `workflow`, `execution`, `env`. No FS/network/credentials. Compile once at save time. Non-deterministic results persisted for replay. |
| **13** | Workflow versioning | Two tables: `workflows` (pointer) + `workflow_versions` (immutable history). `Draft → Published → Archived`. `executions.workflow_version_id` pinned at start, never mutated. Auto triggers use latest Published at claim time. Retention: keep referenced + current + last 20 orphaned + 90 days. |
| **14** | Stateful actions | `StatefulAction` trait with typed `State`. `StepOutcome::{Continue, CheckpointAndContinue, Done, WaitUntil}`. `CheckpointPolicy::Hybrid{10 steps, 30s}` default. Write-behind buffer flushed on policy/terminal/suspend/SIGTERM/pressure/lease-loss. State in `execution_nodes.state` column (not separate table). 1 MB inline / 100 MB blob cap. `iteration_idempotency_key` for external dedup. |
| **15** | Delivery semantics | Four guarantees: trigger at-least-once with dedup; node at-least-once with stable key; side effects **effectively-once when contract honored**; cancel eventually-terminated with bounded grace. Marketing forbidden: «exactly-once», «guaranteed no duplicates», «100% reliable». |
| **16** | Storage schema | Full SQL catalog for all tables. Three core: `trigger_events` (inbox + dedup), `executions` (run entity, all states), `execution_nodes` (per-attempt + state column). Plus `execution_journal`, `execution_control_queue`, `workflows`/`workflow_versions`, identity/tenancy/quota/audit. No `workers` table, no `stateful_checkpoints` table. |
| **17** | Multi-process coord | Leaderless through Postgres. Ephemeral `node_id` (`nbl_`) generated at startup. `executions.claimed_by` + `claimed_until` (30s TTL, 10s renewal). Unified claim query: new work OR stale recovery OR wake-up in one `FOR UPDATE SKIP LOCKED`. Takeover resumes from last checkpoint, `Orphaned` after 3 crashes. Three levels: dispatcher loop (intra) + peer coord (inter) + K8s/systemd (infra). |
| **18** | Observability | OpenTelemetry stack. `ObservabilityContext` parallel surface to `ScopeLevel`, reuses IDs. Generated at ingress, persisted `executions.trace_id`, propagated via `TracedEvent<E>`. Structured JSON logs + Prometheus `/metrics` + OTLP export. Four eventbus subscribers: storage writer, metrics collector, websocket broadcaster, audit writer. Websocket: in-process v1, PG LISTEN/NOTIFY v2, Redis v3. |
| **19** | Error taxonomy | Build on existing `nebula-error` (`Classify`, `NebulaError<E>`, 14 categories, detail types). Per-crate domain enums with `thiserror` + `Classify`. Explicit `From` chain at each boundary with `.context()` + `.with_source()`. `ApiError` → RFC 9457 at API boundary. Two-tier projection: internal full-fidelity, public safe subset. PII allowlist: `DebugInfo` never in public. Panics → `Fatal` with sanitized message. |
| **20** | Testing | New `nebula-testing` crate (thin adapters over `mockall`/`wiremock`/`proptest`). Three tiers: Unit (`ActionContextBuilder`, <10ms), Component (`ActionTest`/`StatefulActionTest`, <500ms), Integration (`TestEnvironment::ephemeral`, 1-10s). `TestClock` trait for time control. Dedicated `CronTriggerTest`/`WebhookTriggerTest`/`QueueTriggerTest`/`PollingTriggerTest`. Contract tests: idempotency, cancellation, no-panic, no-credential-leak. Knife fixture `run_knife_scenario(env)` as merge gate. |
| **21** | Schema crate | New `nebula-schema` replaces `nebula-parameter`. Pattern 4: enum `Field` wrapper + 18 per-type structs + `define_field!` macro. Naming: `Schema`/`Field`/`FieldKey`/`FieldValue`. Full compile-time type safety (`StringField.min(5)` = compile error). Unified `Rule` from `nebula-validator` для validation + visibility gating. Dedicated `SecretField`. 7 per-type widget enums. `RequiredMode`/`VisibilityMode`. `ActionMetadata { input: Schema, output: OutputSchema }`. Derive: `Input` + `Output` (CredentialSchema deferred). Migration: 5 PR sequence. |
| **22** | Credential system v3 | Three-crate split: `nebula-credential` (core ~10K LOC), `nebula-storage::credential::*` (repos, layers, rotation), `nebula-engine::credential_executor`. Delete `retry.rs` facade (use `nebula-resilience` directly). **New features**: envelope encryption с pluggable KMS (AWS/GCP/Azure/Vault/local), external provider delegation (Vault/AWS SM/GCP SM/Azure KV/Infisical/Doppler/Keyring), dynamic secrets (`DYNAMIC` const + `release()`), OIDC workload identity federation (AWS STS / GCP STS / Azure AD), tamper-evident audit log (HMAC hash chain), distributed refresh coordinator (Postgres advisory locks), `CredentialId` prefixed ULID, enriched `CredentialContext` (Principal + tenancy + trace_id + clock), migration `ParameterCollection` → `nebula_schema::Schema`. 15 PR sequence. |
| **23** | Cross-crate foundation | `nebula-core` gets `context/` + `accessor/` + `guard` + `dependencies` + `scope` modules. **Context**: `Context` base trait + `BaseContext` struct + capability traits (`HasResources`/`HasCredentials`/`HasLogger`/`HasMetrics`/`HasEventBus`) + action-specific (`HasNodeIdentity`/`HasTriggerScheduling`) + umbrella traits (`ActionContext`/`TriggerContext` via blanket impl) + internal domain contexts (`CredentialContext`/`ResourceContext`) + concrete runtime contexts in engine (`ActionRuntimeContext`/`TriggerRuntimeContext`). **Guard**: `Guard` + `TypedGuard` traits + `CredentialGuard<S: Zeroize>` (existing) + `ResourceGuard<R>` (renamed from `ResourceHandle`) + extension traits `HasResourcesExt::resource<R>()` / `HasCredentialsExt::credential<C>()` — fully typed access, no string keys. **Dependencies**: unified `Dependencies` + `DeclaresDependencies` trait (replaces 5-method `ActionDependencies`) + multi-credential support + resource-to-resource deps + credential-to-resource deps (credential-to-credential **forbidden**) + `#[uses_credential(...)]` / `#[uses_credentials([...])]` / `#[uses_resource(...)]` / `#[uses_resources([...])]` derive attributes (single + bulk forms) + Tarjan SCC cycle detection. **Scope**: 5 `ScopeLevel` variants (Global/Org/Workspace/Workflow/Execution per spec 02 + 06) + 9-field `Scope` struct (adds workflow_version_id/attempt_id/node_key/trigger_id/instance_id) + strict containment access rules. 7 PR sequence. **Prerequisite for per-crate redesign** of resource/credential/action. |
| **28** | `nebula-engine` redesign | **Absorb `nebula-runtime`** → one crate (~20 files). Modules: `orchestrator/` (frontier DAG), `dispatch/` (action execution, sandbox, registry), `context/` (ActionRuntimeContext, TriggerRuntimeContext — spec 23), `durability/` (checkpoint, idempotency, output buffer), `trigger.rs` (generic TriggerManager). **Port-driven routing:** delete `EdgeCondition`, error routing via `OutputPort::Error` + explicit `ErrorRouter` ControlAction node. Edges = simple wires, zero invisible conditions. **Crash recovery:** type-aware (StatelessAction re-execute, StatefulAction resume checkpoint, AwaitAction re-register wait). **Idempotency:** engine-managed iteration counter + `hash(state)` stuck detection + optional `idempotency_key(state)` business key. **Cancel:** in-process=`JoinHandle::abort()`, sandbox=SIGTERM→SIGKILL. Cancel=grace, Terminate=immediate. **Events:** `nebula-eventbus` broadcast (4 subscribers: storage, metrics, websocket, audit). **Expressions:** engine resolves ALL before dispatch, action sees pure Value. **Version:** load from DB per execution, no cache v1. **`nebula-execution`** stays separate (pure types, no async). 4-PR sequence. |
| **27** | `nebula-action` redesign | **Structural:** `nebula_action::Context` deleted → `nebula_core::Context` + `HasNodeIdentity`. `ActionContext`/`TriggerContext` structs → umbrella marker traits (blanket impl). `ActionDependencies` → `DeclaresDependencies`. `anyhow`/`async_trait` removed (Rust 1.94). `#[diagnostic::on_unimplemented]`. **Action types:** `ActionResult<T>` → per-type results (`StatelessResult`: Success/Skip/Drop; `StatefulResult`: Continue/Done/Abort). **New core traits:** `AgentAction` (think→ToolCalls/Answer/Delegate loop), `TransactionalAction` (execute+compensate saga). **New DX wrappers:** `EventTrigger` (event subscriber with `EventOutcome` Emit/Skip/Reject + `AckStrategy`). **Streaming:** `StreamSource`/`StreamStage`/`StreamSink` (composable pipeline, `Vec<Out>` fan-out, `consume_batch`, `BackpressureStrategy`). **Interactive:** `InteractiveAction` (prepare+handle_response, human-in-the-loop). **Agent tools:** `AgentTool` (standalone, JSON protocol) + `ToolProvider` on Resource (resource-bundled tools, typed `&Lease` access, `ToolDef` in `ResourceMetadata`). `SupportPort` (exists in port.rs) for wiring tools/resources to agents. |
| **26** | `nebula-credential` redesign | `CredentialAccessor` unified — delete local trait, use `nebula-core::CredentialAccessor`. `CredentialResolverRef` deleted — replaced by `HasCredentials` on `CredentialContext` (runtime composition via `ctx.credential::<C>()`). `CredentialContext` rewritten: `BaseContext` + `HasResources` (OAuth2→HttpResource) + `HasCredentials` (composition) + domain fields (callback_url, app_url, session_id). `CredentialGuard<S>` gains Guard/TypedGuard. `HasCredentialsExt` extension trait: `ctx.credential::<C>()` → `CredentialGuard<C::Scheme>`. `SecretString` → `secrecy` crate + `RedactedSecret` wrapper + serde helpers moved from core. `ParameterCollection` → `nebula-schema::Schema`. `retry.rs` deleted (366 LOC). Three-crate split boundaries: core traits stay, store/layers → nebula-storage, executor/resolver → nebula-engine. `DeclaresDependencies`: `#[uses_resource]` allowed, `#[uses_credential]` compile error. 5-PR sequence. |
| **25** | `nebula-resource` redesign | `ResourceHandle`→`ResourceGuard` + Guard/TypedGuard impls. `Ctx`/`BasicCtx`/`Extensions`/local `ScopeLevel` → deleted, replaced by `ResourceContext` struct (BaseContext + HasResources + HasCredentials). `auth` parameter on `create()` stays (validated by 4 prototypes). Credential rotation: Manager tracks `credential_id` at registration, subscribes to `EventBus<CredentialEvent>`, dispatches per-topology via `ReloadOutcome` (SwappedImmediately / PendingDrain / Restarting / NoChange) — same path as config hot-reload. `HasResourcesExt` extension trait: `ctx.resource::<R>()` → `ResourceGuard<R>` typed access, blanket impl on any `HasResources`. `#[derive(Resource)]` auto-generates `DeclaresDependencies` from `Auth` type + `#[uses_credential]`/`#[uses_resource]` attributes. 7 topology traits unchanged (only ctx type in signatures). `compat.rs` deleted. 4-PR sequence. Net -75 lines. |
| **24** | `nebula-core` redesign | Full SRP audit + cleanup: **delete** 6 dead files (constants, types, traits, secret_string, serde_secret, option_serde_secret) + 3 orphan IDs (TenantId, RoleId, OwnerId). **Migrate** UUID→prefixed ULID via `domain-key` v0.5. **Rename** `NodeId`→`NodeKey` (`Key<NodeDomain>`, authored string — convention: `*Key`=authored, `*Id`=generated). **Replace** `SecretString`→`secrecy` crate. **Shrink** CoreError 15→5 variants. **Add** spec 08 `LayerLifecycle`+`ShutdownOutcome`. **Move** `InterfaceVersion`→`nebula-action`, `Version`→`semver` crate. **Remove** `zeroize`+`postcard` deps, **add** `tokio-util`. **Establish** workspace-wide naming convention (`FooKey`/`FooId`/`HasFoo`/`FooAccessor`/`FooGuard`/`FooContext`/`FooRuntimeContext`). Target: 15 entries (13 files + 2 dirs), ~40 public types, strict contracts boundary. 5-PR migration (domain-key v0.5 → cleanup → ID migration → spec 23 modules → consumer migration). |

## Storage tables (final list)

| Layer | Tables |
|---|---|
| Identity | `users`, `oauth_links`, `sessions`, `personal_access_tokens`, `verification_tokens` |
| Tenancy | `orgs`, `workspaces`, `org_members`, `workspace_members`, `service_accounts` |
| Workflow | `workflows`, `workflow_versions` |
| Execution | `executions`, `execution_nodes`, `execution_journal`, `execution_control_queue` |
| Triggers | `triggers`, `trigger_events`, `cron_fire_slots`, `pending_signals` |
| Credentials | `credentials`, `resources` |
| Quotas | `org_quotas`, `org_quota_usage`, `workspace_quota_usage`, `workspace_dispatch_state` |
| Audit | `slug_history`, `audit_log` |

**Not needed:** `workers`/`nodes`, `stateful_checkpoints`, `execution_queues`, separate `webhook_event_seen`.

## Canon update map

| Canon section | Source spec |
|---|---|
| §2 extend (D-model positioning) | 01 |
| §3.8.1 new (StatefulAction durability) | 14 |
| §3.11 new (expression language) | 12 |
| §4.5.1 new (marketing anti-patterns) | 15 |
| §4.7 new (observability contract) | 18 |
| §5 update (scope + marketing) | 01, 15 |
| §7.3 new (workflow versioning) | 13 |
| §9.1–§9.6 new (triggers + delivery) | 11, 15 |
| §11.2 rewrite (retry cascade) | 09 |
| §11.3 extend (two-sided idempotency) | 15 |
| §11.5 sharpen (durability matrix final) | 14, 16 |
| §11.7 new (timeouts taxonomy) | 10 |
| §11.8 new (tenancy reference) | 02 |
| §11.9 new (RBAC reference) | 04 |
| §12.2 extend (cancel propagation) | 08 |
| §12.3 update (multi-process) | 17 |
| §12.4 extend (error propagation) | 19 |
| §12.5 extend (auth model) | 03 |
| §12.8 new (multi-process coord) | 17 |
| §12.9 new (product telemetry) | 03 |
| §12.10 new (API routing) | 05 |
| §12.11 new (slug contract) | 07 |
| §12.12 new (testing contract) | 20 |
| §11.10 new (schema vocabulary contract) | 21 |
| §11.11 new (credential system contract) | 22 |
| §11.12 new (cross-crate foundation — Context, Guard, Dependencies, Scope) | 23 |
| §12.13 new (capability trait DI pattern) | 23 |
| §12.14 new (credential / resource dependency rules) | 23 |
| §12.15 new (naming convention: `*Key`/`*Id`/`Has*`/`*Guard`/`*Context`) | 24 |
| §3.10 update (nebula-core scope — strict contracts, no entity models) | 24 |

## Open items / technical debt

| Item | Status | Action |
|---|---|---|
| `nebula-core::ProjectId` → `WorkspaceId` rename | pending refactor | Spec 24 PR 2 (ID migration) |
| `NodeAttemptId` → `AttemptId` + `node_` → `att_` prefix | pending | Spec 24 PR 2 (ID migration) |
| `NodeId` (UUID) → `NodeKey` (`Key<NodeDomain>`) | pending | Spec 24 PR 2 (ID migration) |
| `SecretString` → `secrecy` crate | pending | Spec 24 PR 1 (cleanup) |
| `CoreError` 15 → 5 variants | pending | Spec 24 PR 3 (spec 23 modules) |
| `domain-key` v0.5 (ULID support) | pending | Spec 24 PR 0 (separate repo) |
| Dead files cleanup (constants, types, traits) | pending | Spec 24 PR 1 (cleanup) |
| `trace_id` column in `executions` | pending | Schema update from spec 18 |
| `ResourceHandle` → `ResourceGuard` rename | spec 25 PR 1 | Implements new Guard trait |
| `Ctx`/`BasicCtx`/`Extensions` → `ResourceContext` | spec 25 PR 2 | Delete ctx.rs, add context.rs |
| Credential rotation in Manager | spec 25 PR 3 | ReloadOutcome + CredentialEvent subscription |
| `HasResourcesExt` extension trait | spec 25 PR 3 | `ctx.resource::<R>()` typed access |
| `ActionContext` struct → trait + `ActionRuntimeContext` | spec 23 PR 4 | Moves concrete to engine |
| `ActionDependencies` 5-method trait → `DeclaresDependencies` | spec 23 PR 4 | Replaces with multi-credential + deps DAG |
| `nebula-credential::retry` 366-line facade | spec 22 PR 3 | Delete, callers use `nebula-resilience` directly |
| `nebula-parameter` → `nebula-schema` migration | specs 21 + 22 + 23 PRs | `Parameter`/`ParameterCollection` → `Field`/`Schema` |
| `nebula-credential` storage extraction | spec 22 PR 2 | Move `store.rs`, `layer/`, `rotation/` → `nebula-storage::credential` |
| `execution_nodes.state_blob_ref` for >1 MB | planned v1.5 | First user hit |
| Enterprise SSO (SAML/OIDC) | planned v2 | Enterprise ask |
| R4b restart from failed node | planned v2 | Operator ask |
| Custom roles | planned v2 | Enterprise ask |
| Distributed rate limiting | planned v2 | Multi-process load |
| WASM plugin sandbox | **explicit non-goal** | Canon §12.6 final |
| Engine retry persistence (old §11.2 debt) | closes with spec 09 | Implementation PR |
| `CredentialHttp` interface trait (OAuth2 via `HttpResource`) | deferred | Spec 22 defers to future release |
| Cross-workspace resource sharing | deferred | Spec 23 §11.1 — add when operational need arises |
| `Global` scope rename to `Instance` | deferred | Spec 23 §11.2 — current naming retained |

## Not yet specced (future Q&A)

- **#25** Plugin distribution — `cargo-nebula`, signing, registry, install flow
- **#26** Storage migration tooling — running cluster upgrades
- **#27** Deployment modes — systemd, Docker, K8s, desktop app
- **#28** Backup / disaster recovery — pg_dump strategy, point-in-time
- **#29** Upgrade path details — engine binary, schema, plugins

## Files produced in this session

```
docs/plans/
├── 2026-04-15-architecture-review-qa.md      (parent summary, ~600 lines)
└── 2026-04-15-arch-specs/
    ├── README.md                              (index + template)
    ├── COMPACT.md                             (this file)
    ├── 01-product-positioning.md              (~150 lines)
    ├── 02-tenancy-model.md                    (~400 lines)
    ├── 03-identity-auth.md                    (~600 lines)
    ├── 04-rbac-sharing.md                     (~570 lines)
    ├── 05-api-routing.md                      (~590 lines)
    ├── 06-id-format.md                        (~450 lines)
    ├── 07-slug-contract.md                    (~690 lines)
    ├── 08-cancellation-cascade.md             (~770 lines)
    ├── 09-retry-cascade.md                    (~770 lines)
    ├── 10-timeouts-quotas.md                  (~700 lines)
    ├── 11-triggers.md                         (~850 lines)
    ├── 12-expression-language.md              (~675 lines)
    ├── 13-workflow-versioning.md              (~570 lines)
    ├── 14-stateful-actions.md                 (~830 lines)
    ├── 15-delivery-semantics.md               (~405 lines)
    ├── 16-storage-schema.md                   (~850 lines)
    ├── 17-multi-process-coordination.md       (~635 lines)
    ├── 18-observability-stack.md              (~800 lines)
    ├── 19-error-taxonomy.md                   (~900 lines)
    ├── 20-testing-story.md                    (~1500 lines)
    ├── 21-schema-crate.md                     (~1900 lines)
    ├── 22-credential-system.md                (~1900 lines)
    ├── 23-cross-crate-foundation.md           (~2000 lines)
    ├── 24-nebula-core-redesign.md             (~500 lines)
    ├── 25-nebula-resource-redesign.md         (~350 lines)
    ├── 26-nebula-credential-redesign.md       (~400 lines)
    ├── 27-nebula-action-redesign.md           (~800 lines)
    └── 28-nebula-engine-redesign.md           (~500 lines)
```

**Total: ~21 700 строк технической документации** покрывающей 28 decisions от positioning до engine redesign. Full stack specced: core → resource → credential → action → engine.
