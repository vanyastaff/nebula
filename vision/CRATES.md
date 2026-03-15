# Crate Descriptions

Purpose, responsibility contract, and key public types for every crate in the workspace.

For inter-crate dependencies (who depends on what) see **[DEPENDENCIES.md](./DEPENDENCIES.md)**.

---

## Quick Reference

### Implemented

| Crate | Layer | One-line purpose |
|-------|-------|-----------------|
| [`nebula-core`](#nebula-core) | Foundation | Universal vocabulary: IDs, keys, scope, base traits |
| [`nebula-log`](#nebula-log) | Foundation | Structured logging pipeline built on `tracing` |
| [`nebula-system`](#nebula-system) | Foundation | Cross-platform OS/hardware information |
| [`nebula-eventbus`](#nebula-eventbus) | Foundation | Typed broadcast channel — transport only, zero domain knowledge |
| [`nebula-validator`](#nebula-validator) | Foundation | Composable, type-safe validation combinators |
| [`nebula-storage`](#nebula-storage) | Infrastructure | Storage port: in-memory / PostgreSQL / Redis / S3 backends |
| [`nebula-workflow`](#nebula-workflow) | Infrastructure | Workflow definition, DAG graph, topological sort |
| [`nebula-memory`](#nebula-memory) | Infrastructure | Memory pools, arenas, multi-level cache, budget tracking |
| [`nebula-telemetry`](#nebula-telemetry) | Infrastructure | Execution event bus, metrics primitives, telemetry backend trait |
| [`nebula-config`](#nebula-config) | Infrastructure | Multi-source configuration with hot-reload and validation |
| [`nebula-parameter`](#nebula-parameter) | Data | Rich parameter type system (UI + runtime contract) |
| [`nebula-macros`](#nebula-macros) | Tooling | Proc-macros: `#[derive(Action)]`, `#[derive(Plugin)]`, etc. |
| [`nebula-metrics`](#nebula-metrics) | Cross-cutting | Standard metric names + Prometheus export adapter |
| [`nebula-resilience`](#nebula-resilience) | Cross-cutting | Retry, circuit-breaker, bulkhead, rate-limiter, timeout |
| [`nebula-expression`](#nebula-expression) | Cross-cutting | Expression language for dynamic values (n8n-compatible syntax) |
| [`nebula-credential`](#nebula-credential) | Domain | Credential lifecycle: OAuth2, API keys, JWT, SAML, mTLS, rotation |
| [`nebula-resource`](#nebula-resource) | Domain | Resource lifecycle management: pooling, health, scoping, metrics |
| [`nebula-action`](#nebula-action) | Domain | Action contract: traits, types, I/O model — not the executor |
| [`nebula-execution`](#nebula-execution) | Domain | Execution state machine, journals, idempotency, plan |
| [`nebula-plugin`](#nebula-plugin) | Domain | Plugin registry and packaging unit (e.g. "Slack", "HTTP Request") |
| [`nebula-runtime`](#nebula-runtime) | Execution | Action runner: registry, sandbox interface, data-passing policy |
| [`nebula-engine`](#nebula-engine) | Execution | Workflow orchestrator: level-by-level scheduling, node resolution |
| [`nebula-api`](#nebula-api) | Entry point | Thin REST/WebSocket server with no business logic |
| [`nebula-webhook`](#nebula-webhook) | Entry point | Inbound webhook server with UUID-isolated trigger endpoints |
| [`nebula-sdk`](#nebula-sdk) | Entry point | All-in-one developer library for building plugins and workflows |

### Planned

> These crates are designed and documented in `docs/crates/` but **not yet implemented**. They represent the next phases of development.

| Crate | Layer | One-line purpose | Priority |
|-------|-------|-----------------|----------|
| [`nebula-resource-postgres`](#nebula-resource-postgres-planned) | Adapter | Reference PostgreSQL resource adapter for `nebula-resource` | P1 |
| [`nebula-sandbox`](#nebula-sandbox-planned) | Execution | Action isolation contract: capability enforcement, WASM/process backends | P1 |
| [`nebula-worker`](#nebula-worker-planned) | Execution | Distributed worker: task state machine, heartbeats, graceful drain | P1 |
| [`nebula-idempotency`](#nebula-idempotency-planned) | Cross-cutting | Deduplication, retry-safety, exactly-once execution | P1 |
| [`nebula-testing`](#nebula-testing-planned) | Tooling | Test harnesses for workflows, actions, and integrations | P1 |
| [`nebula-tenant`](#nebula-tenant-planned) | Domain | Multi-tenancy: isolation strategies, quota enforcement | P2 |
| [`nebula-cluster`](#nebula-cluster-planned) | Infrastructure | Distributed coordination: membership, consensus, failover | P2 |
| [`nebula-locale`](#nebula-locale-planned) | Cross-cutting | Internationalisation: locale negotiation, translation bundles | P2 |
| [`nebula-cli`](#nebula-cli-planned) | Entry point | Command-line interface for workflows, executions, and cluster ops | P2 |
| [`nebula-desktop`](#nebula-desktop-planned) | Entry point | Tauri 2 visual workflow editor and execution monitor | P2 |

---

## Foundation Crates

These crates have **zero nebula-\* dependencies** and are safe to import anywhere.

---

### `nebula-core`

**Layer:** Foundation  
**Crate name:** `nebula-core`

#### Purpose

The universal vocabulary of the entire system. Every other crate uses types from `nebula-core`. It provides stable, serializable identifiers and the scope contract that all runtime components must honour.

#### Responsibilities

- **Identifiers** — strongly-typed UUID wrappers: `UserId`, `TenantId`, `WorkflowId`, `ExecutionId`, `NodeId`, `ResourceId`, `CredentialId`, `ProjectId`, `RoleId`, `OrganizationId`
- **Keys** — string-based discriminators: `PluginKey` (e.g. `"telegram_bot"`), `ActionKey` (e.g. `"send_message"`), `ParameterKey`, `CredentialKey`
- **Scope system** — `Scope` enum (`Global → Organization → Project → Workflow → Execution → Action`) with lifecycle rules
- **Base traits** — `Identifiable`, `Scoped`, `HasContext` implemented by domain types
- **Common types & constants** — small utilities and domain-wide invariants
- **Multi-tenancy types** — `ProjectType`, `RoleScope` for identity and access management

#### What does NOT belong here

Business logic, I/O, external dependencies, telemetry, or anything that could pull in heavyweight crates. If a type requires `serde` or `uuid`, that is acceptable via re-export; anything requiring async runtimes, HTTP, or database clients must live elsewhere.

---

### `nebula-log`

**Layer:** Foundation  
**Crate name:** `nebula-log`

#### Purpose

The single logging pipeline for the entire Nebula ecosystem. All crates that emit logs should use only this crate — never raw `tracing` macros or `println!` in library code.

#### Responsibilities

- **Structured log emission** — JSON and pretty-print formats, configurable at init time
- **Writer backends** — stderr, stdout, file, and fanout (multiple sinks)
- **Observability hooks** — `ObservabilityRegistry` for attaching custom event handlers
- **Integrations** — optional OpenTelemetry and Sentry exporters (behind feature flags)
- **`LogConfig`** — env-driven preset builder (`NEBULA_LOG`, `RUST_LOG`)

#### What does NOT belong here

Domain events, metrics counters, or execution tracing. Those are the responsibility of `nebula-telemetry`. `nebula-log` is about textual/structured log lines, not about observability pipelines.

---

### `nebula-system`

**Layer:** Foundation  
**Crate name:** `nebula-system`

#### Purpose

Cross-platform host introspection. Used by `nebula-memory` for pressure-based eviction and by monitoring components that need to report host-level capacity.

#### Responsibilities

- **CPU** — core count, usage, frequency
- **Memory** — total / available / used, pressure signals
- **OS info** — platform, hostname, kernel version
- **Process info** — PID, open file handles, RSS (feature-gated)
- **Network / Disk / Component** — optional hardware metrics

#### What does NOT belong here

Application-level metrics, workflow-domain data, or anything that requires knowing about Nebula's own concepts.

---

### `nebula-eventbus`

**Layer:** Foundation  
**Crate name:** `nebula-eventbus`

#### Purpose

A generic broadcast channel with back-pressure policies. It is **transport-only** — no domain event types are defined here. Domain crates create their own `EventBus<MyEvent>` instances.

#### Responsibilities

- **`EventBus<E>`** — typed broadcast channel backed by `tokio::sync::broadcast`
- **`BackPressurePolicy`** — `DropOldest`, `DropNewest`, `Block`, `Error`
- **`Subscriber<E>`** — consumer handle with `recv()` / `try_recv()` / async stream
- **`EventBusStats`** — `sent_count`, `dropped_count`, `subscriber_count` for observability

#### Contract

- **Non-blocking send by default** — producers never block on subscriber speed
- **Best-effort delivery** — no global ordering guarantee, no persistence
- Domain-specific event types live in their respective crates (e.g. `ExecutionEvent` in `nebula-telemetry`)

#### What does NOT belong here

Any Nebula-specific event type, retry logic, or persistence.

---

### `nebula-validator`

**Layer:** Foundation  
**Crate name:** `nebula-validator`

#### Purpose

A composable, type-safe validation framework. Used by `nebula-parameter` for input validation, by `nebula-config` for configuration validation, and by `nebula-macros` for derive-based validation.

#### Responsibilities

- **`Validate<T>` trait** — the core combinator interface
- **Combinators** — `.and()`, `.or()`, `.not()`, `.when()`, `.unless()`, `.each()`, `.optional()`, `.field()`, `.nested()`
- **Built-in validators** — length, range, pattern (regex), network (URL, IP, email), boolean, content, temporal, size
- **`validator!` macro** — zero-boilerplate anonymous validators
- **Error model** — `ValidationError` with path, message, and category

#### What does NOT belong here

Parameter display logic, UI rendering hints, or anything that knows about Nebula's parameter system specifically.

---

## Infrastructure Crates

Provide reusable plumbing; may depend on Foundation crates.

---

### `nebula-storage`

**Layer:** Infrastructure  
**Crate name:** `nebula-storage`

#### Purpose

The storage port (hexagonal architecture): a backend-agnostic abstraction over persistence. All domain crates that need to persist state use this crate's traits — they never depend on a specific database directly.

#### Responsibilities

- **`WorkflowRepository`** — CRUD for `WorkflowDefinition`
- **`ExecutionRepository`** — CRUD and state-update for executions
- **Backends** (feature-gated):
  - `memory` — in-process, zero-config, used in tests and development
  - `postgres` — feature `postgres`, production backend
  - `redis` — feature `redis`, planned
  - `s3` / local filesystem — planned
- **`StorageError`** — unified error type for all backends

#### What does NOT belong here

Business rules about executions or workflows. The repository layer is pure I/O — it stores and retrieves, it does not validate, orchestrate, or transform domain objects.

---

### `nebula-workflow`

**Layer:** Infrastructure  
**Crate name:** `nebula-workflow`

#### Purpose

The canonical data model for a workflow. Defines what a workflow *is*, how its nodes and edges are structured, and how to validate and traverse the graph.

#### Responsibilities

- **`WorkflowDefinition`** — top-level definition (id, name, nodes, connections, config)
- **`NodeDefinition`** — individual step (`node_id`, `action_key`, input `ParamValue`s)
- **`Connection` / `EdgeCondition`** — edges between nodes with optional conditional routing
- **`DependencyGraph`** — `petgraph`-backed DAG: topological sort, level computation, cycle detection
- **`WorkflowBuilder`** — fluent, validated construction of workflows
- **`validate_workflow`** — comprehensive multi-error validation (cycles, dangling refs, etc.)
- **`NodeState`** — execution progress tracking per node

#### What does NOT belong here

Execution state (that is `nebula-execution`), action logic, plugin resolution, or anything that changes at runtime.

---

### `nebula-memory`

**Layer:** Infrastructure  
**Crate name:** `nebula-memory`

#### Purpose

High-performance, workflow-optimized memory management. Reduces allocator pressure during high-throughput execution by providing arenas, pools, and caches tuned for the short-lived allocation patterns of workflow nodes.

#### Responsibilities

- **Allocators** — bump allocator, pool allocator, stack allocator, tracked/monitored wrappers
- **Arenas** — scoped arenas, thread-local and cross-thread variants
- **`ObjectPool<T>`** — bounded reuse pool with RAII return-on-drop
- **Multi-level cache** — LRU, LFU, FIFO, TTL, random eviction policies; partitioned and scheduled variants
- **Memory budget** — per-execution memory budget with reservations and policy enforcement
- **Stats & monitoring** — per-allocator stats, predictive usage, real-time snapshots

#### What does NOT belong here

Application-level caches (e.g. credential caches — those belong in their respective crates), OS interaction beyond what `nebula-system` already provides.

---

### `nebula-telemetry`

**Layer:** Infrastructure  
**Crate name:** `nebula-telemetry`

#### Purpose

Observability plumbing for execution-level events and metrics. Acts as the bridge between execution internals and external monitoring systems (Prometheus, OTLP, custom backends).

#### Responsibilities

- **`ExecutionEvent`** — execution lifecycle events (started, node_completed, failed, etc.)
- **`EventBus<ExecutionEvent>`** — wraps `nebula-eventbus` with domain-specific type
- **`MetricsRegistry`** — in-memory `Counter`, `Gauge`, `Histogram` primitives
- **`TelemetryService` trait** — pluggable backend: implement to integrate Prometheus, Datadog, etc.
- **`NoopTelemetry`** — zero-cost no-op for tests and MVP deployments
- **`TraceContext`** — distributed trace context propagation

#### What does NOT belong here

Metric naming conventions (those are in `nebula-metrics`), log formatting (that is `nebula-log`), or execution business logic.

---

### `nebula-config`

**Layer:** Infrastructure  
**Crate name:** `nebula-config`

#### Purpose

Flexible, validated, hot-reloadable configuration management. Provides a unified interface for loading configuration from multiple sources in priority order.

#### Responsibilities

- **`ConfigBuilder`** — fluent builder for composing sources and validators
- **`ConfigSource`** — `File`, `Env`, `EnvPrefix`, `Default`, `Bytes`, `Custom`
- **Loaders** — composite, env, file (TOML / YAML / JSON / RON), hot-reload polling watcher
- **Validators** — no-op, function-based, JSON Schema
- **`ConfigResult<T>`** — typed extraction from the config tree
- **Hot-reload** — file watcher with debounced re-parse and subscriber notification

#### What does NOT belong here

Application-specific configuration structs (those live in the crate that owns the config), runtime state, or any I/O beyond reading config sources.

---

## Data & Tooling Crates

---

### `nebula-parameter`

**Layer:** Data  
**Crate name:** `nebula-parameter`

#### Purpose

The rich parameter type system that bridges UI configuration and runtime execution. Defines exactly what kinds of inputs an action can declare and how those inputs are displayed, validated, and passed at runtime.

#### Responsibilities

- **`ParameterDef`** — full definition of a single parameter (key, kind, metadata, validation rules, display conditions)
- **`ParameterKind`** — discriminated union of all parameter types:
  - Text, Textarea, Number, Boolean/Checkbox, Select, MultiSelect
  - DateTime, Date, Time, Color, Code (with language selector)
  - Secret, Hidden, Group, List, Object, Notice
- **`ParameterCollection`** — ordered list of `ParameterDef`s with lookup by key
- **`ParameterValues`** — runtime key→value map for passing inputs to actions
- **`DisplayCondition` / `DisplayRuleSet`** — conditional UI visibility (show field X only when field Y equals Z)
- **`ValidationRule`** — per-parameter validation (delegates to `nebula-validator`)
- **`SelectOption` / `OptionsSource`** — static or dynamic select options

#### What does NOT belong here

Action execution logic, plugin metadata, or UI rendering code. `nebula-parameter` defines the *contract* for parameters; how they are rendered is the responsibility of the frontend.

---

### `nebula-macros`

**Layer:** Tooling  
**Crate name:** `nebula-macros`

#### Purpose

Proc-macro crate providing `#[derive(...)]` macros that eliminate boilerplate for the most common Nebula patterns. This is a **proc-macro crate** — it cannot be `use`d directly in library code, only in `[dev-dependencies]` or alongside `nebula-sdk`.

#### Responsibilities

| Macro | What it generates |
|-------|-------------------|
| `#[derive(Action)]` | Implements the `Action` trait from metadata attributes |
| `#[derive(Resource)]` | Implements the `Resource` trait |
| `#[derive(Plugin)]` | Implements the `Plugin` trait with component registration |
| `#[derive(Credential)]` | Implements the `Credential` trait |
| `#[derive(Parameters)]` | Generates `ParameterCollection` from struct fields |
| `#[derive(Validator)]` | Implements field-based validation |
| `#[derive(Config)]` | Loads from env variables with validation |

#### What does NOT belong here

Runtime logic, business rules, or anything that cannot be determined at compile time.

---

## Cross-Cutting Crates

---

### `nebula-metrics`

**Layer:** Cross-cutting  
**Crate name:** `nebula-metrics`

#### Purpose

Enforces consistent metric naming across the system and provides a ready-to-use Prometheus text exporter. Sits on top of `nebula-telemetry`'s in-memory primitives.

#### Responsibilities

- **`naming` module** — `const` metric name strings: `nebula_executions_total`, `nebula_action_duration_seconds`, etc.
- **`TelemetryAdapter`** — thin adapter over `nebula-telemetry::MetricsRegistry` that records using the standard names
- **`PrometheusExporter`** — renders current metric state as Prometheus text format

#### What does NOT belong here

Metric storage primitives (those are in `nebula-telemetry`), log lines, or domain-specific event types.

---

### `nebula-resilience`

**Layer:** Cross-cutting  
**Crate name:** `nebula-resilience`

#### Purpose

Production-grade resilience patterns for any async operation. Any crate that makes external calls (HTTP, database, message queues) should use `nebula-resilience` rather than rolling its own retry/circuit-breaker logic.

#### Responsibilities

- **Retry** — configurable backoff (fixed, exponential, jitter), max attempts, retryable error classification
- **Circuit Breaker** — closed/open/half-open state machine with failure threshold and recovery window
- **Bulkhead** — concurrency limiting with semaphore-based isolation
- **Rate Limiter** — token bucket, leaky bucket, sliding window, adaptive; backed by `governor`
- **Timeout** — per-call deadline with cancellation propagation
- **Hedge** — speculative parallel requests, take fastest response
- **Fallback** — value/function/async fallback on failure
- **Compose** — chain multiple patterns into a single policy
- **`ResilienceManager`** — named policy registry for reuse

#### What does NOT belong here

Domain logic, credential handling, or any knowledge of Nebula-specific types.

---

### `nebula-expression`

**Layer:** Cross-cutting  
**Crate name:** `nebula-expression`

#### Purpose

A safe, sandboxed expression language for evaluating dynamic values inside workflow definitions. Compatible with n8n syntax so that users migrating from n8n can reuse their expressions.

#### Responsibilities

- **Lexer + Parser + AST** — tokenises and parses expression strings
- **Evaluator** — walks the AST against an `EvalContext` holding `$node`, `$execution`, `$workflow`, `$input`, `$env`
- **Built-in functions** — string, array, math, datetime, object, conversion, utility
- **Template strings** — `{{ expression }}` interpolation inside parameter values
- **`ExpressionPolicy`** — controls allowed features (disable network access, etc.)
- **`ExpressionEngine`** — thread-safe, re-usable evaluation engine with interned strings
- **`MaybeExpression<T>`** — value that is either a literal `T` or an expression string resolved at runtime

#### What does NOT belong here

Workflow scheduling, action execution, or any I/O. The expression engine is a pure interpreter.

---

## Domain Crates

---

### `nebula-credential`

**Layer:** Domain  
**Crate name:** `nebula-credential`

#### Purpose

Universal credential lifecycle management. Handles acquiring, validating, rotating, and revoking credentials across many authentication protocols. Actions and resources obtain credentials through this crate rather than managing secrets themselves.

#### Responsibilities

- **Protocols** — OAuth2 (PKCE, client credentials, device flow), API Key, Basic Auth, JWT, SAML, Kerberos, LDAP, mTLS, Header Auth, Database connection strings
- **`CredentialManager`** — central registry: load, cache, refresh, and revoke credentials
- **`CredentialProvider` trait** — decoupled acquisition (local file, Vault, AWS Secrets Manager, Kubernetes Secret, PostgreSQL)
- **Rotation** — background rotation scheduler with blue/green swap, grace period, retry, rollback
- **`SecretString`** — zero-copy wrapper with automatic zeroization on drop
- **`CredentialState` / `CredentialStatus`** — lifecycle state machine (active, expiring, expired, revoked)
- **`CredentialReference`** — how actions refer to credentials without holding the secret directly
- **Storage** — optional persistence via `nebula-storage` (feature `storage-postgres`)

#### What does NOT belong here

HTTP client logic, action execution, or resource pooling. Credentials are a *security primitive*, not an execution mechanism.

---

### `nebula-resource`

**Layer:** Domain  
**Crate name:** `nebula-resource`

#### Purpose

Lifecycle management for shared, reusable I/O resources: database connection pools, HTTP clients, file handles, message queue connections, and any other resource that needs controlled acquisition, health monitoring, and scoped cleanup.

#### Responsibilities

- **`Resource` trait** — interface for any managed resource type
- **`Manager`** — central registry: create, pool, scope, health-check, and tear down resources
- **`Pool<R>`** — FIFO/LIFO pool with configurable min/max size, idle timeout, and health validation
- **`Poison<T>`** — poisoned-state guard for pool internals; prevents reusing state after interrupted critical sections
- **`Scope`** — ties resource lifetime to workflow / execution / action scope; auto-cleanup on scope exit
- **`ResourceProvider` trait** — decoupled acquisition (for testing and DI)
- **`ResourceRef`** — `TypeId`-based handle used by actions to request resources
- **Health checks** — periodic background health validation with quarantine on failure
- **Resilience breakers** — integrated create/recycle circuit breakers via `nebula-resilience` to stop retry storms
- **Autoscaling** — pool resizing based on demand metrics
- **Instrumentation** — per-resource metrics via `nebula-metrics`
- **Dependency graph** — ordered initialization / shutdown respecting resource dependencies

#### What does NOT belong here

Credential acquisition (that is `nebula-credential`), action execution logic, or domain-specific resource implementations (e.g. a future `nebula-resource-postgres`).

---

### `nebula-action`

**Layer:** Domain  
**Crate name:** `nebula-action`

#### Purpose

The execution contract for workflow nodes. Defines *what* actions are and *how they communicate* with the engine, but **not** how they are executed or scheduled. Follows Ports & Drivers: the traits live here, the executor lives in `nebula-runtime`.

#### Responsibilities

- **`Action` trait** — base trait: `metadata()` + `components()`
- **Specialised action traits:**
  - `SimpleAction` — returns `Result<Output, Error>`, zero boilerplate
  - `StatelessAction` — single execution with flow-control (`Continue`, `Break`, `Skip`)
  - `StatefulAction` — iterative with persistent state across calls
  - `TriggerAction` — event source that starts workflows (start/stop lifecycle)
  - `ResourceAction` — graph-level DI (configure/cleanup), scoped to downstream branch
  - `StreamingAction` — continuous stream producer
  - `TransactionalAction` — distributed transaction participant (saga pattern)
  - `InteractiveAction` — human-in-the-loop interaction
- **`ActionResult<O>`** — execution result with output and flow-control intent
- **`ActionOutput`** — first-class output: value, binary blob, reference, stream
- **`ActionError`** — distinguishes retryable from fatal failures
- **`Context` trait** — base execution context (credentials, resources, logger)
- **`ActionMetadata`** — static descriptor (key, version, capabilities)
- **`ActionComponents`** — declared parameters, credentials, and resources

#### What does NOT belong here

The actual action executor (that is `nebula-runtime`), plugin registration (that is `nebula-plugin`), or engine scheduling (that is `nebula-engine`).

---

### `nebula-execution`

**Layer:** Domain  
**Crate name:** `nebula-execution`

#### Purpose

Everything the engine needs to track and persist during a live execution — state machines, journals, idempotency keys, and the pre-computed execution plan. This is the *model* of an execution, not the *executor*.

#### Responsibilities

- **`ExecutionStatus`** — 8-state machine (Pending → Running → Succeeded / Failed / Cancelled / TimedOut / Paused / Waiting)
- **`ExecutionState`** — full execution snapshot (status, started_at, completed_at, error)
- **`NodeExecutionState`** — per-node status, attempts, and output reference
- **`ExecutionPlan`** — pre-computed parallel execution schedule (topologically sorted levels)
- **`ExecutionContext`** — lightweight runtime context (execution_id, timeout budget, tenant)
- **`JournalEntry`** — append-only audit log of all execution events
- **`NodeOutput`** — node output data with metadata
- **`NodeAttempt`** — individual attempt tracking (attempt number, started_at, error)
- **`IdempotencyKey` / `IdempotencyManager`** — exactly-once guarantees for action side-effects
- **`transition` module** — validated state machine transitions with invariant checks

#### What does NOT belong here

The engine that drives transitions (that is `nebula-engine`), persistence (that is `nebula-storage`), or action logic.

---

### `nebula-plugin`

**Layer:** Domain  
**Crate name:** `nebula-plugin`

#### Purpose

The user-visible, versionable packaging unit. A plugin is what a user sees in the workflow editor's node palette: "Slack", "HTTP Request", "PostgreSQL". Each plugin bundles a set of related actions and declares which credential types it needs.

#### Responsibilities

- **`Plugin` trait** — base trait: `metadata()` + `components()`
- **`PluginMetadata`** — key, display name, version, group, icon, docs URL
- **`PluginComponents`** — registered actions (`Vec<Box<dyn Action>>`) and credential requirements
- **`PluginType`** — wraps a single `Plugin` or a `PluginVersions` container
- **`PluginVersions`** — multi-version container keyed by `u32`, for graceful versioning
- **`PluginRegistry`** — in-memory map `PluginKey → PluginType` with lookup and list
- **`PluginLoader` trait** — extensible plugin discovery (filesystem, network, WASM — future)
- **`PluginError`** — error type for registration and lookup failures

#### What does NOT belong here

Action execution logic (that is `nebula-runtime`), action trait definitions (that is `nebula-action`), or engine scheduling.

---

## Execution Crates

---

### `nebula-runtime`

**Layer:** Execution  
**Crate name:** `nebula-runtime`

#### Purpose

The action runner: resolves an action from the registry, enforces data limits, calls the sandbox, and emits telemetry. The runtime sits between the engine (which schedules *what* to run) and the sandbox (which provides isolation).

#### Responsibilities

- **`ActionRuntime`** — main entry point: `run(action_key, input, context)` → `ActionResult`
- **`ActionRegistry`** — maps `ActionKey` → `Box<dyn Action>`, populated at startup via plugins
- **`DataPassingPolicy`** — output size enforcement: `Unlimited`, `Capped(bytes)`, `Reject(threshold)`
- **`Sandbox` trait** — abstraction over execution environment (in-process for now, WASM planned)
- **`ExecutionQueue`** — async bounded queue for backpressure between engine and runtime
- **`StreamBackpressure`** — backpressure logic for `StreamingAction` outputs

#### What does NOT belong here

Workflow scheduling (that is `nebula-engine`), plugin packaging (that is `nebula-plugin`), or action trait definitions (that is `nebula-action`).

---

### `nebula-engine`

**Layer:** Execution  
**Crate name:** `nebula-engine`

#### Purpose

The top-level workflow orchestrator. Given a `WorkflowDefinition` and initial inputs, the engine builds an execution plan, resolves inputs level-by-level, and drives the runtime until the workflow completes or fails.

#### Responsibilities

- **`WorkflowEngine`** — `execute(workflow, inputs)` → `ExecutionResult`
- **Level-by-level scheduling** — all nodes in the same topological level run concurrently with bounded parallelism
- **Input resolution** — `NodeResolver` maps predecessor outputs to node inputs using `nebula-expression`
- **Error handling** — partial failure, retry policy, cancellation propagation
- **`ExecutionResult`** — final outcome: outputs map, status, timing, errors
- **`EngineError`** — structured error hierarchy for scheduling, resolution, and runtime failures

#### What does NOT belong here

Action logic, plugin registration, API handling, or persistence. The engine is a pure orchestrator — it delegates all I/O to the runtime and all state persistence to `nebula-execution` + `nebula-storage`.

---

## Entry-Point Crates

These crates are at the top of their respective stacks. Nothing in the workspace depends on them.

---

### `nebula-api`

**Layer:** Entry point  
**Crate name:** `nebula-api`

#### Purpose

Thin HTTP server exposing Nebula's functionality over REST (and eventually WebSocket). Follows the "API as entry point" principle: handlers extract and delegate, services orchestrate, all business logic lives in lower crates.

#### Responsibilities

- **`App`** — Axum-based application builder with shared state
- **Handlers** — thin: extract request data, call a service, map to HTTP response
- **Services** — orchestrate calls to storage, config, and (Phase 2) engine
- **Routes** — domain-grouped: `health`, `workflows`, `executions`
- **Middleware** — auth, rate limiting, tracing, request-id, security headers
- **Models** — API DTOs (request/response structs, separate from domain types)
- **Errors** — RFC 9457 Problem Details format

#### Current state

`nebula-api` depends on `storage` and `config` but **not yet on `nebula-engine`**. The endpoint for triggering workflow executions is a Phase 2 task. See [ROADMAP.md](../docs/ROADMAP.md).

#### What does NOT belong here

Business logic, domain rules, or anything that should be reusable outside an HTTP context.

---

### `nebula-webhook`

**Layer:** Entry point  
**Crate name:** `nebula-webhook`

#### Purpose

Inbound webhook receiver. Provides a single HTTP server that routes incoming POST requests from external services (Telegram, GitHub, Stripe, etc.) to the correct trigger action via UUID-isolated endpoints.

#### Responsibilities

- **`WebhookServer`** — single Axum server for all webhook traffic
- **`RouteMap`** — `UUID → TriggerHandle` routing table
- **`WebhookHandle`** — RAII handle: registration on acquire, automatic deregistration on drop
- **`WebhookPayload`** — normalised payload (headers, body, query params)
- **`WebhookEnvironment`** — `Test` vs `Production` traffic separation
- **`WebhookStore`** — in-memory store of active webhook registrations

#### What does NOT belong here

Workflow triggering logic, execution management, or authentication of webhook sources (that is the responsibility of the trigger action that owns the webhook handle).

---

### `nebula-sdk`

**Layer:** Entry point  
**Crate name:** `nebula-sdk`

#### Purpose

The all-in-one developer library for building plugins, actions, and workflows. Re-exports the most useful types from across the workspace under a single, stable import path so plugin authors don't need to know the internal crate structure.

#### Responsibilities

- **Re-exports** from `nebula-action`, `nebula-workflow`, `nebula-parameter`, `nebula-credential`, `nebula-plugin`, `nebula-macros`, `nebula-validator`
- **`prelude`** — `use nebula_sdk::prelude::*` gives everything needed to write a plugin
- **`testing` module** — test helpers: mock contexts, fake credential providers, in-memory resources
- **Action helpers** — convenience wrappers and blanket implementations
- **Workflow helpers** — high-level builder adapters

#### What does NOT belong here

Engine internals, API server code, or anything that changes the runtime behaviour of the system. The SDK is a *surface* crate — it adds ergonomics, not new functionality.

---

## Planned Crates

> These crates are fully designed (see `docs/crates/<name>/`) but **not yet implemented** in `crates/`.
> Sections below describe their intended purpose, scope, and relationship to existing crates.

---

### nebula-sandbox (planned)

**Layer:** Execution  
**Priority:** P1 — required before distributing community/untrusted plugins

#### Purpose

The sandbox execution contract and isolation boundary for actions. Workflow actions (especially community or user-defined ones) may be untrusted — running them in-process without isolation risks crashes, infinite loops, resource abuse, and data leakage. The sandbox crate owns the **port** (trait) that the runtime calls, and the **drivers** that implement it.

#### Responsibilities

- **`SandboxRunner` trait** — port: `run(action, sandboxed_context)` → `ActionResult`; decouples runtime from backend
- **`SandboxedContext`** — capability-checked context proxy: only declared credentials and resources are accessible; undeclared access returns an auditable error
- **Cancellation enforcement** — cooperative or periodic check during action execution
- **In-process driver** — low-latency backend for trusted/built-in actions; still enforces capability proxy
- **WASM driver** _(future)_ — hard isolation for untrusted/community actions
- **Process driver** _(future)_ — OS-level isolation for highest-security deployments
- **Audit trail** — sandbox violation and policy decision events

#### Current state

The `SandboxRunner` trait exists today inside `nebula-runtime` (see `runtime/sandbox.rs`). The capability model in `SandboxedContext` currently largely forwards to `NodeContext` without enforcement — full capability checking is the P1 gap. The standalone `nebula-sandbox` crate will extract this port + in-process driver and make the contract stable and independently versioned.

#### What does NOT belong here

Action business logic, workflow scheduling, or generic retry/resilience patterns.

---

### nebula-worker (planned)

**Layer:** Execution  
**Priority:** P1 — required for distributed deployments

#### Purpose

The distributed worker runtime. In a distributed Nebula deployment, the API enqueues workflow tasks and workers claim and execute them in isolation. The worker owns the full task lifecycle from queue pull through heartbeat through result reporting — without knowing anything about workflow semantics.

#### Responsibilities

- **Task state machine** — `queued → claimed → running → succeeded / failed / timed_out`
- **Pull/ack protocol** — claim task from broker, heartbeat to extend lease, ack on success, nack/requeue on failure
- **Bounded concurrency** — configurable `max_concurrent_tasks`; back-pressure to queue when at limit
- **Graceful drain** — on SIGTERM: stop pulling, complete in-flight tasks within timeout, release or requeue leases
- **Isolation** — delegates execution to `nebula-runtime` + `nebula-sandbox`; no business logic in worker itself
- **Observability** — per-task traces, worker-level metrics (tasks/s, concurrency, queue depth, error rate), structured logs

#### Relationship to existing crates

`nebula-engine` schedules nodes within a single execution; `nebula-worker` acquires work from a queue across the fleet and drives `nebula-engine` per-task. Workers are the **fleet-level** execution unit; the engine is the **per-workflow** execution unit.

#### What does NOT belong here

Workflow DAG orchestration (engine), action business logic (action/plugin), or API surface (api/webhook).

---

### nebula-idempotency (planned)

**Layer:** Cross-cutting  
**Priority:** P1 — required for production queue-based deployments

#### Purpose

Deduplication, retry-safety, and exactly-once execution semantics. Workflow runs and API requests can be retried or duplicated (at-least-once queue delivery, client retry on network failure). Idempotency ensures that the same logical operation executed twice produces the same observable result as executing it once.

#### Responsibilities

- **`IdempotencyKey`** — deterministic composite key: `f(execution_id, node_id, attempt)`
- **`IdempotencyManager`** — `check(key)` → `AlreadyDone(result) | Run`; `record(key, result)`
- **Storage backends** — in-memory (MVP, already in `nebula-execution`), PostgreSQL, Redis
- **HTTP `Idempotency-Key` header** — middleware layer for the API: extract key, check before handler, cache response
- **`IdempotentAction` trait** — opt-in per-action exactly-once contract
- **Workflow checkpoint / resume** — persist execution progress so a workflow can resume mid-run after a crash

#### Current state

`IdempotencyKey` and `IdempotencyManager` exist today in `nebula-execution` with an in-memory `HashSet` backend. The gap is: no persistent storage, no HTTP layer, no per-action idempotency, and no checkpoint/resume. Extracting to a dedicated crate makes the storage pluggability and HTTP middleware cleaner.

#### What does NOT belong here

General retry and resilience patterns (those are `nebula-resilience`), credential storage (`nebula-credential`), or execution orchestration (`nebula-engine`).

---

### nebula-testing (planned)

**Layer:** Tooling  
**Priority:** P1 — improves developer experience and test quality across all crates

#### Purpose

Shared test harnesses, mock implementations, and test utilities for the entire workspace. Eliminates the pattern of every crate re-implementing its own mock context, mock credential provider, or fake resource manager.

#### Responsibilities

- **`WorkflowTestHarness`** — run a full workflow against a mock engine; capture output, events, metrics, per-node output
- **`ActionTestHarness`** — run a single action with injected mock resources and credentials
- **Mock implementations** — `MockCredentialProvider`, `MockResourceManager`, `MockTelemetryService`, `MockEventBus`
- **`TestContext`** — implementation of `nebula-action::Context` backed by mocks
- **Assertion helpers** — `assert_node_output`, `assert_execution_status`, `assert_event_emitted`
- **`#[workflow_test]` macro** — ergonomic `#[tokio::test]`-like attribute for workflow tests

#### Relationship to `nebula-sdk`

`nebula-sdk` already has a `testing` module with basic helpers. When `nebula-testing` is extracted, the SDK's testing module will re-export from here to avoid a breaking change for SDK users.

#### What does NOT belong here

Production code, integration against real external services (those are integration tests in individual crates), or performance benchmarking infrastructure.

---

### nebula-tenant (planned)

**Layer:** Domain  
**Priority:** P2 — required for multi-tenant SaaS deployments

#### Purpose

The single authoritative multi-tenancy layer. Provides tenant identity resolution, isolation strategy selection, and quota enforcement so that every other crate that needs to be tenant-aware consults one place rather than implementing its own tenant semantics.

#### Responsibilities

- **`TenantContext`** — resolved tenant identity + isolation strategy + quota policy; passed through execution scope
- **Tenant resolution** — from JWT claim, API key, request header, or execution metadata
- **Isolation strategies** — `Shared` (logical isolation, same process), `Dedicated` (dedicated pool), `Isolated` (hard process/WASM isolation)
- **Quota policy** — max concurrent executions, storage limit, action rate limit; checked before execution start
- **`QuotaEnforcer`** — pre-execution quota check; explicit rejection with structured error
- **Governance hooks** — `TenantLifecycleHook` for provisioning and teardown events
- **Audit trail** — quota decisions and isolation boundary events

#### Relationship to existing crates

`nebula-core` already has `TenantId` and `OrganizationId`. `nebula-resource` and `nebula-credential` already accept a `Scope` that can encode tenant. `nebula-tenant` provides the authoritative *resolution and enforcement* layer on top of these primitives.

#### What does NOT belong here

Storage engine internals, workflow execution orchestration, or credential protocol implementation.

---

### nebula-cluster (planned)

**Layer:** Infrastructure  
**Priority:** P2 — required for distributed, multi-node deployments

#### Purpose

Distributed execution coordination. When multiple Nebula nodes (API servers, workers) run in a fleet, cluster membership, task ownership, and failover must be coordinated so that work is never lost, duplicated, or executed in parallel on two nodes simultaneously.

#### Responsibilities

- **Membership protocol** — node join/leave; convergent membership view; no split-brain
- **Task ownership** — which worker owns which task; handoff or reassignment on node failure
- **Distributed scheduling** — deterministic assignment of tasks to nodes; observable scheduling decisions
- **Consensus-backed state** _(future)_ — Raft or leader-based replication of control-plane state (task ownership, config)
- **Autoscaling signals** — demand metrics fed to cluster scheduler for fleet scaling decisions
- **Failover semantics** — documented: at-most-once, at-least-once, or exactly-once per task class

#### Relationship to existing crates

`nebula-engine` runs a workflow on a single node; `nebula-worker` runs a single task; `nebula-cluster` decides *which* worker runs *which* task and handles node failures. `nebula-resilience` provides retry patterns that cluster uses internally.

#### What does NOT belong here

Local single-node execution internals (engine, runtime), workflow definition semantics (workflow), or tenant policy ownership (tenant).

---

### nebula-locale (planned)

**Layer:** Cross-cutting  
**Priority:** P2 — required for multi-language user-facing deployments

#### Purpose

Centralised internationalisation and localisation. API error messages, validation failures, and UI strings should be returned in the user's language. A single locale crate provides locale negotiation, translation bundle management, and localized formatting so that API, runtime, and action surfaces all behave consistently.

#### Responsibilities

- **Locale negotiation** — resolve from `Accept-Language` header, tenant locale setting, or global default; deterministic fallback chain (e.g. `pt-BR → pt → en`)
- **`TranslationBundle`** — load and cache translation files (Fluent FTL or TOML key-value)
- **`MessageKey`** — stable string identifier for a translatable message
- **`t!(key, args)` macro** — ergonomic lookup with interpolation
- **Localized formatting** — dates, numbers, and units according to locale
- **Cross-crate contract** — `nebula-api`, `nebula-validator`, and `nebula-action` all call the same locale interface; no duplicate locale logic per crate
- **Message key stability** — minor releases: additive only; breaking key changes require a major release with MIGRATION notes

#### What does NOT belong here

Business logic, authentication, workflow scheduling, or any domain-specific Nebula concept.

---

### nebula-cli (planned)

**Layer:** Entry point  
**Priority:** P2

#### Purpose

Command-line interface for developers and operators to manage Nebula from the terminal. A thin CLI shell over the REST API.

#### Responsibilities

- **Workflow management** — `nebula workflow deploy`, `list`, `activate`, `deactivate`
- **Execution management** — `nebula execution watch`, `logs`, `cancel`
- **Action development** — `nebula action create --template`, `test`, `publish`
- **Cluster operations** — `nebula cluster status`, `add-node`, `rebalance`
- **Configuration** — profiles, environments, authentication tokens
- **Output formats** — human-readable and `--output json` for scripting

#### What does NOT belong here

Business logic. The CLI delegates everything to `nebula-api` via HTTP; it is a pure UX surface.

---

### nebula-desktop (planned)

**Layer:** Entry point  
**Priority:** P2

#### Purpose

The visual workflow editor and execution monitor — a Tauri 2 desktop application (React + TypeScript frontend, Rust backend). Provides a native macOS / Windows / Linux app that connects to a local or remote Nebula instance.

#### Responsibilities

- **Rust side (Tauri backend)**
  - OAuth2 deep-link callback handling
  - Secure credential storage via OS keychain (`tauri-plugin-keychain`)
  - Connection profiles (multiple Nebula instances)
  - System tray and native notifications
  - Auto-update via `tauri-plugin-updater`
  - Typed IPC commands/events via `tauri-specta` (no raw `invoke` strings)
- **Frontend (React + TypeScript)**
  - Visual workflow canvas (drag-and-drop node editor)
  - Real-time execution monitor (WebSocket event stream)
  - Node catalog with parameter forms
  - Credential and resource management screens
  - TanStack Query for all API data; Zustand for auth/connection state

#### Technical choices

- **Tauri 2** over pure-Rust UI (gpui/egui/iced) — platform integration features (OAuth deep-link, OS keychain, tray, auto-update) are first-party Tauri plugins, avoiding significant from-scratch implementation work for each platform integration feature
- **React calls HTTP API directly** for workflow/execution data; Rust owns only auth + connection + native features
- **`pnpm`** in `apps/desktop/`; Rust crate at `crates/desktop/` (Tauri backend logic)

#### What does NOT belong here

Business logic, API server code, or any modification of execution semantics.

---

## Design Invariants

### 1. Dependency direction

The dependency graph is **strictly acyclic**. Foundation → Infrastructure → Data → Domain → Execution → Entry points. A crate in layer N must **never** depend on a crate in layer N+1 or higher.

### 2. Ports & Drivers

Business logic never imports concrete implementations. `nebula-action` defines the port; `nebula-runtime` is the driver. `nebula-storage` defines the port; `backend/postgres.rs` is the driver.

### 3. `nebula-core` is the only universal dependency

All crates may depend on `nebula-core`. Any type that needs to be shared across *all layers* (an ID, a key, a scope) must live in `nebula-core`. Domain-specific shared types must not be pushed into `nebula-core`.

### 4. Optional features over new crates

When a crate needs an optional heavy dependency (e.g. Postgres in `nebula-credential`), use a Cargo feature flag rather than spinning up a new crate. New crates are warranted only when the abstraction is independently reusable.
