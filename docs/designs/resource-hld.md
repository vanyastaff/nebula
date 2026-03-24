# nebula-resource — High-Level Design

## Overview

nebula-resource manages the lifecycle of external resources (databases, APIs,
message brokers, bots, browsers, SSH connections) for the Nebula workflow engine.

### What it solves

Workflow actions need external resources: query a database, send a Telegram message,
run a command over SSH, scrape a webpage. Each resource type has fundamentally
different access patterns:

| Resource | Pattern | Challenge |
|----------|---------|-----------|
| Postgres | N independent TCP connections | Pool management, per-tenant isolation, server-side `max_connections` limit |
| HTTP client | Single shared client, internal connection pool | No per-caller state, Clone is enough |
| Telegram Bot | Long-lived process + lightweight handles for callers | Polling loop lifecycle, token-based access, incoming events |
| SSH | One expensive TCP connection, many cheap sessions on top | Session multiplexing, keepalive, session count limits |
| Kafka Consumer | Single owner, concurrent access = rebalance storm | Exclusive locking, offset commit between owners |
| Browser | N pages in shared browser process, heavy cleanup | Per-page isolation, ~500ms recycle, process health |

Without nebula-resource, every action author would implement their own connection
management, health checks, retry logic, credential handling, and cleanup.
With it, action authors write one line:

```rust
let db = ctx.resource::<Postgres>().await?;
db.query("SELECT 1", &[]).await?;
// drop(db) → automatic cleanup (pool checkin, recycle, or destroy)
```

The framework handles everything else: pool management, broken connection detection,
credential resolution, recovery from backend failures, config hot-reload,
graceful shutdown, metrics emission.

### Core guarantees

1. **Topology transparency** — action authors never know or care whether a resource
   is pooled, shared, exclusive, or multiplexed. The API is always
   `ctx.resource::<R>().await?` → `ResourceHandle<R>` → `Deref` to `R::Lease`.

2. **RAII cleanup** — `ResourceHandle` Drop triggers automatic release. No manual
   checkin, no forgotten connections, no resource leaks. Three Drop behaviors:
   - `Owned` (Resident clone, Service Cloned token): Drop = noop, value dropped naturally.
   - `Guarded` (Pool checkout, Transport session): Drop = submit release task to ReleaseQueue.
   - `Shared` (Exclusive Arc + permit): Drop = submit reset + permit release to ReleaseQueue.

3. **Credential isolation** — secrets are never stored in Config. Framework resolves
   credentials via `CredentialStore` before calling `create()`. Resource author
   receives typed credential as parameter: `create(config, credential, ctx)`.
   Credential rotation handled via `EventBus<CredentialRotatedEvent>` — resource
   author does nothing, framework evicts stale instances automatically.

4. **Recovery coordination** — when a shared backend goes down (e.g., Postgres primary),
   `RecoveryGate` ensures exactly one probe attempt while all other callers wait.
   No thundering herd. CAS-based state machine: Idle → InProgress → Failed/Idle.
   Multiple resources on same backend share one gate via `RecoveryGroup`.

5. **Scope containment** — multi-tenant safe by default. Resources registered at
   `Organization("acme")` scope cannot be accessed by `Organization("competitor")`.
   `ScopeResolver` trait validates parent-child relationships. `CachedScopeResolver`
   avoids DB round-trip on every acquire. Single-tenant deployments opt into
   simplified scoping via `Manager::with_simplified_scoping()`.

6. **Error classification** — every resource error maps to one of 6 `ErrorKind`
   variants that drive framework retry decisions:
   - `Transient` → retry with backoff (network blip, timeout)
   - `Permanent` → never retry (auth failure, invalid config)
   - `Exhausted` → retry after cooldown (rate limit, quota depleted)
   - `Backpressure` → pool/semaphore full, caller decides
   - `NotFound` → resource key not in registry
   - `Cancelled` → CancellationToken fired

   `ClassifyError` derive macro auto-generates `From<MyError> for Error` mapping
   per variant: `#[classify(transient)]`, `#[classify(permanent)]`, etc.

### System boundaries and data flow

nebula-resource owns everything between "resource registered" and "handle dropped".
The full lifecycle of a single acquire:

```
Action calls ctx.resource::<Postgres>()
       │
       ▼
┌─ ResourceContext::resource() ─────────────────────────────┐
│  1. Check ScopedResourceMap (parent ResourceAction?)      │
│     → found: acquire from scoped runtime                  │
│     → not found: continue to Manager                      │
│                                                           │
│  2. Manager::acquire(resource_id, ctx, options)           │
│     │                                                     │
│     ▼                                                     │
│  3. Registry::get_typed() — scope-aware lookup            │
│     DashMap<(TypeId, ResourceId), ScopedRuntime[]>        │
│     Find most-specific compatible scope via ScopeResolver │
│     Returns Arc<ManagedResource<R>>                       │
│     │                                                     │
│     ▼                                                     │
│  4. RecoveryGate check — is backend alive?                │
│     Idle → proceed                                        │
│     InProgress → wait for probe result                    │
│     Failed → return error with retry_after hint           │
│     PermanentlyFailed → return error, no retry            │
│     │                                                     │
│     ▼                                                     │
│  5. AcquireResilience — timeout → retry → circuit breaker │
│     Wraps the actual topology acquire (step 6)            │
│     │                                                     │
│     ▼                                                     │
│  6. TopologyRuntime dispatch (7 variants):                │
│     Pool:        checkout from idle OR create new         │
│                  → is_broken()? → prepare(ctx) → return   │
│     Resident:    Cell::load() → Arc clone → return        │
│     Service:     acquire_token(runtime, ctx) → return     │
│     Transport:   open_session(runtime, ctx) → return      │
│     Exclusive:   semaphore.acquire() → Arc clone → return │
│     EventSource: subscribe(runtime, ctx) → return         │
│     Daemon:      N/A (no acquire — background only)       │
│     │                                                     │
│     ▼                                                     │
│  7. Wrap in ResourceHandle<R> (Owned/Guarded/Shared)      │
│     Record generation, acquired_at, resource_key          │
└───────────────────────────────────────────────────────────┘
       │
       ▼
Action uses handle via Deref → R::Lease
       │
       ▼
Action drops handle (explicit or scope exit)
       │
       ▼
┌─ ResourceHandle::drop() ─────────────────────────────────┐
│  Owned:   noop (value dropped naturally)                  │
│  Guarded: on_release(lease, tainted) → submit to          │
│           ReleaseQueue for async recycle/destroy           │
│  Shared:  on_release(tainted) → submit reset + permit     │
│           release to ReleaseQueue                         │
└───────────────────────────────────────────────────────────┘
       │
       ▼
┌─ ReleaseQueue worker ─────────────────────────────────────┐
│  N primary workers (each owns rx, no Mutex)               │
│  1 dedicated fallback worker (bounded 10k, OOM protection)│
│  30s timeout per task (prevents worker paralysis)         │
│                                                           │
│  Pool release path:                                       │
│    is_broken()? → destroy                                 │
│    stale fingerprint? → destroy                           │
│    max_lifetime exceeded? → destroy                       │
│    resource.recycle(runtime, metrics) → Keep/Drop         │
│    Keep → push to idle queue                              │
│    Drop → destroy                                         │
│                                                           │
│  Transport: close_session(transport, session, healthy)    │
│  Exclusive: reset(runtime) → release semaphore permit     │
└───────────────────────────────────────────────────────────┘
```

### What nebula-resource does NOT own

| Concern | Owner | Integration |
|---------|-------|-------------|
| Credential storage/encryption | nebula-credential (peer crate) | `EventBus<CredentialRotatedEvent>`, never direct import |
| Credential resolution at runtime | `CredentialStore` trait (object-safe) | `resolve_erased()` → `Box<dyn Any>` → typed downcast via `CredentialStoreExt` |
| Action/trigger execution | nebula-action, nebula-engine | Bridge via `ResourceContext` trait on `ActionContext`/`TriggerContext` |
| Retry/circuit-breaker primitives | nebula-resilience | Reused via `AcquireResilience` config: `PipelineBuilder` → `ResiliencePipeline` |
| Observability emission | nebula-telemetry, nebula-metrics | Wrapped via `ResourceMetrics` adapter (counters, histograms, tracing spans) |
| Config change notifications | nebula-config | `AsyncConfigurable` impl on Manager, per-topology reload via `TopologyRuntime::on_config_changed()` |
| Memory pressure signals | nebula-memory | `MemoryMonitor` → `PressureSnapshot` (AtomicU8), pool maintenance reads lock-free |

### Key types at a glance

| Type | Role | Location |
|------|------|----------|
| `Resource` trait | Core abstraction — 5 associated types + 4 lifecycle methods | resource.rs |
| `ResourceConfig` trait | Config validation + fingerprint for stale detection | resource.rs |
| `Credential` trait | Marker for credential types, `KIND` for store lookup | credential.rs |
| `Ctx` trait | Execution context: scope, execution_id, cancellation, extensions | ctx.rs |
| `Error` / `ErrorKind` | 6-variant classified error driving retry decisions | error.rs |
| `ResourceHandle<R>` | Caller-facing RAII handle, Deref to R::Lease | handle.rs |
| `HandleInner` | 3 variants: Owned / Guarded / Shared | handle.rs |
| `LeaseGuard<L>` | Pool-internal RAII wrapper with taint/poison/detach | lease/guard.rs |
| `AcquireOptions` | Intent, deadline, tags for acquire customization | lease/options.rs |
| `Manager` | Central registry, acquire dispatch, shutdown orchestration | manager/ |
| `RegistrationBuilder` | Typestate builder: NeedsConfig → NeedsId → NeedsTopology → Ready | manager/builder.rs |
| `ManagedResource<R>` | Per-registration: topology runtime + recovery + metrics + status | runtime/managed.rs |
| `TopologyRuntime<R>` | 7-variant enum dispatching acquire/release per topology | runtime/mod.rs |
| `ReloadOutcome` | Result of config reload: Swapped / PendingDrain / Restarting / NoChange | runtime/mod.rs |
| `ResourceStatus` | Operational status: phase + generation + last_error | state.rs |
| `ResourcePhase` | 6 phases: Initializing / Ready / Reloading / Draining / ShuttingDown / Failed | state.rs |
| `RecoveryGate` | CAS state machine for thundering herd prevention | recovery/gate.rs |
| `RecoveryGroup` | Shared gate for multiple resources on same backend | recovery/group.rs |
| `WatchdogHandle` | Opt-in background health probe (Service, Transport) | recovery/watchdog.rs |
| `ReleaseQueue` | N primary workers + 1 fallback worker, async cleanup | release_queue.rs |
| `Cell<T>` | Lock-free ArcSwapOption cell for Resident topology | cell.rs |
| `ResourceKey` | Compile-time validated key via `resource_key!()` macro | nebula-core |
| `ResourceId` | Instance identifier (newtype over CompactString) | resource.rs |
| `AcquireResilience` | Per-resource: timeout → retry → circuit breaker config | integration/resilience.rs |
| `ResourceMetrics` | Counters/histograms wrapper over TelemetryAdapter | metrics.rs |

### Topology quick reference

| Topology | Acquire returns | Internal machinery | Example resources |
|----------|----------------|-------------------|-------------------|
| **Pool** | `Guarded(R::Lease)` — checkout | Idle queue, recycle, maintenance, warmup | Postgres, SMTP, Redis dedicated, Browser |
| **Resident** | `Owned(R::Lease)` — clone | ArcSwap Cell, is_alive_sync polling | HTTP, Redis shared, gRPC, Kafka Producer |
| **Service** | `Owned(R::Lease)` or `Guarded(R::Lease)` | Long-lived runtime, token acquire/release | Telegram Bot, WebSocket, rate-limited API |
| **Transport** | `Guarded(R::Lease)` — session | Shared connection, open/close session, keepalive | SSH, AMQP |
| **Exclusive** | `Shared(Arc<R::Lease>)` + permit | Semaphore, reset between callers | Kafka Consumer, serial port |
| **EventSource** | Subscription (secondary) | subscribe/recv, paired with primary topology | Redis Pub/Sub, Telegram updates |
| **Daemon** | None (secondary) | run() loop, CancellationToken, restart policy | Polling loops, connection maintainers |

---

## Action Author View

This section is for agents implementing **actions and triggers** — the consumers
of resources. You do NOT need to understand topologies, pools, recovery, or
connection management. You need exactly one pattern.

### The one pattern

```rust
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    // 1. Acquire — one line, topology hidden
    let db = ctx.resource::<Postgres>().await?;

    // 2. Use — Deref to R::Lease gives you the resource-specific API
    let rows = db.query("SELECT * FROM users WHERE id = $1", &[&input.user_id]).await?;

    // 3. Drop — automatic. Pool checkin, session close, token release — all handled.
    //    You never call .release(), .checkin(), .close(). Just let the variable drop.
    Ok(ActionResult::new(rows))
}
```

`ctx.resource::<R>()` returns `ResourceHandle<R>` which implements `Deref<Target = R::Lease>`.
You work with `R::Lease` directly — for Postgres that's `PgConnection` (query, execute,
transaction), for Telegram Bot that's `TelegramBotHandle` (send_message, get_me),
for SSH that's `SshSession` (exec, spawn).

### What you see vs what's hidden

| You see | Framework handles |
|---------|-------------------|
| `ctx.resource::<Postgres>().await?` | Pool checkout, or create new if pool empty, or wait if pool full |
| `db.query(...)` | Connection was already `prepare()`-d with tenant search_path |
| Variable goes out of scope | Recycle (DISCARD ALL), return to pool, or destroy if broken |
| Error from resource | Classified as Transient/Permanent, framework decides retry |
| Nothing — it just works | Recovery from backend failure, credential rotation, config reload |

### Resolution order

When you call `ctx.resource::<R>()`, the framework resolves the resource in this order:

```
1. Scoped — is there a ResourceAction for R in a parent graph branch?
   → found: acquire from scoped runtime (closest ancestor wins)
   → not found: fall through

2. Global — is R registered in the Manager with matching resource_id?
   → found: acquire via Manager (scope-aware lookup)
   → not found: error

3. ActionError::ResourceNotFound
```

You don't control this. Scoped resources (from a `ResourceAction` parent node) take
priority automatically, enabling per-execution isolation without any changes to your code.

### Declaring dependencies

Actions declare their resource requirements at registration time via `ActionDependencies`.
This is separate from runtime acquisition — it tells the engine what resources an action
*needs* so the engine can validate configuration before execution starts.

```rust
impl ActionDependencies for QueryUsersAction {
    fn resources() -> Vec<Box<dyn AnyResource>> {
        vec![Box::new(Postgres)]
    }
}
```

The engine uses `resources()` to:
- Validate that required resources are registered before workflow starts.
- Build dependency tree for UI display.
- Provide clear error messages at startup rather than runtime failures.

### Taint — marking a resource as broken

If you detect that a resource is broken during use (not just a query error, but the
connection itself is dead), you can mark the handle as tainted:

```rust
let mut db = ctx.resource::<Postgres>().await?;
match db.query("SELECT 1", &[]).await {
    Ok(rows) => { /* use rows */ }
    Err(e) if is_connection_dead(&e) => {
        db.taint(); // framework will destroy instead of recycle
        return Err(e.into());
    }
    Err(e) => return Err(e.into()), // normal error, connection probably fine
}
```

`taint()` is a hint: "don't put this back in the pool, destroy it."
For `Owned` handles (Resident, Service Cloned) — taint is a noop (no pool to return to).
For `Guarded` handles (Pool, Transport) — taint causes destroy instead of recycle.
For `Shared` handles (Exclusive) — taint causes destroy + recreate instead of reset.

Most actions never need to call `taint()`. The framework runs `is_broken()` automatically
on release. Use it only when you have application-level knowledge that the framework
can't detect (e.g., server returned a protocol-level "connection corrupted" error that
doesn't close the TCP socket).

### Detach — advanced escape hatch

For rare cases where you need to own the resource beyond the normal acquire/release cycle:

```rust
let ssh = ctx.resource::<Ssh>().await?;
let session = ssh.detach()?; // caller now owns SshSession
// session is no longer tracked by the pool
// YOU are responsible for closing it
```

**After detach:**
- Pool does NOT track this instance (no recycle, no destroy, no metrics).
- YOU are responsible for all cleanup (closing connections, dropping handles).
- Pool accounting reflects the instance as "gone".

**When safe:** long-running SSH tunnels, detached transactions, resource migration.
**When wrong:** "working around" pool timeout — fix the timeout instead.

`detach()` consumes the handle (`fn detach(self)`) — you can't use the handle after.
Only works on `Guarded` handles (Pool, Transport). Returns `DetachError::NotDetachable`
for Owned and Shared handles.

### Credential access

Actions can also access typed credentials directly (not through a resource):

```rust
let cred = ctx.credential::<DatabaseCredential>().await?;
// cred.host, cred.port, cred.username, cred.password.expose()
```

This is for cases where you need raw credential data (e.g., constructing a custom
connection string). For normal resource use, credentials are resolved automatically
by the framework — you never see them.

### Error handling

Resource errors are automatically classified into 6 categories. You don't classify
them — the resource author does via `ClassifyError` derive macro. But you should
understand what happens:

| ErrorKind | Framework behavior | Your action sees |
|-----------|-------------------|------------------|
| `Transient` | Retry with backoff | Error (after retries exhausted) |
| `Permanent` | No retry, fail immediately | Error |
| `Exhausted` | Wait retry_after, then retry | Error (after wait + retry) |
| `Backpressure` | Pool full, may retry | Error |
| `NotFound` | Resource not registered | Error |
| `Cancelled` | CancellationToken fired | Error |

The retry behavior is per-resource, configured at registration time via
`AcquireResilience`. You don't configure it — the platform operator does.

### Error scope

Some errors are target-scoped, not resource-scoped:

```rust
// Telegram: bot blocked by one user — other users still work
bot.send_message(blocked_chat_id, "hi").await?;
// → Error { kind: Transient, scope: Target { id: "12345" } }
// The resource itself is NOT broken. Framework does NOT taint.
```

`ErrorScope::Target { id }` means: "this specific target failed, but the resource
is fine." `ErrorScope::Resource` (default) means: "the resource itself might be broken."

### EventTrigger — zero-boilerplate event-driven triggers

For incoming events (Telegram messages, Redis pub/sub, WebSocket frames), implement
`EventTrigger` instead of writing a manual polling loop:

```rust
struct IncomingMessageTrigger;

impl EventTrigger for IncomingMessageTrigger {
    type Source = TelegramBot;
    type Event  = IncomingMessage;

    async fn on_event(&self, bot: &TelegramBotHandle, _ctx: &TriggerContext)
        -> Result<Option<IncomingMessage>>
    {
        let update = bot.recv_update().await?;
        match update.kind {
            UpdateKind::Message { text: Some(text), .. } => {
                Ok(Some(IncomingMessage { chat_id: update.chat_id, text }))
            }
            _ => Ok(None), // skip non-text updates
        }
    }
    // on_error: default Reconnect — handles disconnects automatically
}
```

**What you write:** just `on_event()` — transform raw data into typed event.
**What the engine handles:** resource acquisition, reconnection with exponential backoff,
error routing, cancellation, event emission. You never write a retry loop.

The engine generates a full lifecycle wrapper behind the scenes:
1. Acquire resource via `ctx.resource::<Source>()`.
2. Call `on_event()` in a loop.
3. On error → call `on_error()` → Reconnect (re-acquire with backoff) / Stop / Ignore.
4. On cancellation → exit cleanly.

### ResourceAction — per-execution scoped resources

Use `ResourceAction` when a resource should be created for one execution (or one
branch of a DAG) and destroyed when that scope completes:

```rust
pub trait ResourceAction: Action {
    type Resource: Resource;

    /// Provide config for the scoped resource.
    async fn configure(&self, ctx: &ActionContext)
        -> Result<<Self::Resource as Resource>::Config>;

    /// Topology for the scoped resource. Default: Resident.
    fn topology(&self) -> ScopedTopology { ScopedTopology::Resident }

    /// Override acquire resilience. Default: framework default.
    fn acquire_resilience(&self) -> Option<AcquireResilience> { None }

    /// Custom cleanup after all downstream nodes complete. Default: noop.
    async fn cleanup(&self, ctx: &ActionContext) -> Result<()> { Ok(()) }
}
```

**Graph example:**
```
[TenantPoolAction: ResourceAction<Postgres>]  ← creates scoped pool
     │
     ├── [QueryUsers]     ← ctx.resource::<Postgres>() → scoped pool (tenant_a)
     │
     └── [QueryOrders]    ← ctx.resource::<Postgres>() → scoped pool (tenant_a)

After both complete: cleanup() → framework destroys scoped pool
```

Downstream actions use `ctx.resource::<Postgres>()` exactly as with global resources —
the scoped vs global distinction is completely transparent.

**Cleanup ordering:**
1. Scoped resource removed from ScopedResourceMap (new acquires → fall through to global)
2. `cleanup()` called — `ctx.resource::<R>()` here resolves to global, NOT scoped
3. Framework calls `shutdown()` → `destroy()` on scoped runtime

### Complete action examples

**StatelessAction — database query:**
```rust
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    let db = ctx.resource::<Postgres>().await?;       // Pool topology
    let rows = db.query("SELECT * FROM users", &[]).await?;
    Ok(ActionResult::new(Users::from_rows(rows)))
    // drop(db) → pool checkin → recycle
}
```

**StatelessAction — send Telegram message:**
```rust
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    let bot = ctx.resource::<TelegramBot>().await?;   // Service topology
    bot.send_message(input.chat_id, &input.text).await?;
    Ok(ActionResult::new(()))
    // drop(bot) → noop (token release)
}
```

**StatelessAction — run SSH command:**
```rust
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    let ssh = ctx.resource::<Ssh>().await?;            // Transport topology
    let output = ssh.exec(&input.command).await?;
    Ok(ActionResult::new(output))
    // drop(ssh) → close_session via ReleaseQueue
}
```

**StatelessAction — exclusive Kafka consume:**
```rust
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<ActionResult<Self::Output>> {
    let consumer = ctx.resource::<KafkaConsumer>().await?;  // Exclusive topology
    let msg = consumer.poll(Duration::from_secs(5));
    Ok(ActionResult::new(msg))
    // drop(consumer) → reset + release semaphore permit
}
```

**StatefulAction — batch import with cursor:**
```rust
async fn execute(&self, input: Self::Input, state: &mut ImportState, ctx: &ActionContext) -> Result<...> {
    let db = ctx.resource::<Postgres>().await?;
    for row in &input.rows[state.cursor..state.cursor + 1000] {
        db.execute("INSERT INTO ...", &[&row]).await?;
        state.cursor += 1;
    }
    Ok(...)
    // Each execute() → fresh acquire. State holds cursor across invocations.
}
```

### Summary — what action authors need to know

1. `ctx.resource::<R>().await?` — acquire any resource, one line
2. Use via `Deref` — resource-specific API directly on the handle
3. Drop = automatic cleanup — never manually release
4. `taint()` — optional, for when you know it's broken and framework can't detect
5. `detach()` — advanced, you take ownership and cleanup responsibility
6. `ActionDependencies` — declare what you need for engine validation
7. `EventTrigger` — for incoming event streams, just implement `on_event()`
8. `ResourceAction` — for per-execution scoped resources, transparent to downstream

---

## Resource Author View

This section is for agents implementing **new resource types** — Postgres, Redis,
Telegram Bot, SSH, Google Sheets, Stripe, etc. You need to understand the Resource
trait, choose a topology, implement lifecycle methods, and follow the contracts.

### The Resource trait

Every resource implements this trait. It has 5 associated types, 1 constant,
and 4 lifecycle methods:

```rust
pub trait Resource: Send + Sync + 'static {
    /// Operational settings (timeouts, pool size, app name). NO secrets.
    type Config: ResourceConfig;

    /// Internal instance managed by framework (connection, client, process).
    type Runtime: Send + Sync + 'static;

    /// Caller-facing handle via Deref. May equal Runtime or differ.
    /// Pool/Resident: Lease = Runtime.
    /// Service: Lease = TelegramBotHandle (lightweight token).
    /// Transport: Lease = SshSession (multiplexed session).
    type Lease: Send + Sync + 'static;

    /// Typed error enum. ClassifyError macro maps to framework ErrorKind.
    type Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static;

    /// Secret data resolved by framework. () for resources without secrets.
    type Credential: Credential;

    /// Compile-time validated identifier. "postgres", "telegram.bot", "ssh".
    const KEY: ResourceKey;

    /// Create one runtime instance. Framework resolves credential beforehand.
    fn create(
        &self,
        config:     &Self::Config,
        credential: &Self::Credential,
        ctx:        &dyn Ctx,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    /// Liveness check. SELECT 1, PING, check session alive.
    /// Default: Ok(()) — resource always "alive".
    fn check(&self, _runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }

    /// Graceful shutdown hint. Called BEFORE destroy.
    /// Flush buffers, stop background tasks.
    /// Default: noop.
    fn shutdown(&self, _runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send { async { Ok(()) } }

    /// Final destroy. Framework guarantees: this is the sole owner.
    /// Drop client, wait for connection task, kill processes.
    /// Default: noop (Rust Drop handles cleanup).
    fn destroy(&self, runtime: Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = runtime;
        async { Ok(()) }
    }

    /// Metadata for UI and diagnostics.
    fn metadata() -> ResourceMetadata { ResourceMetadata::from_key(&Self::KEY) }
}
```

### The 5 associated types — why each exists

| Type | Purpose | Why separate |
|------|---------|-------------|
| `Config` | Operational settings (timeouts, pool size) | Must be `Clone` + serializable for config diffing. Never contains secrets. Separate from `Credential` for independent rotation. |
| `Runtime` | Internal instance managed by framework | The "real" resource. Pool/topology owns it. May differ from caller view (Telegram: `BotRuntime` with broadcast channel vs `BotHandle` with send methods). |
| `Lease` | Caller-facing handle via `ResourceHandle<R>` Deref | Decouples caller API from internal structure. Pool/Resident: `Lease = Runtime`. Service: `Lease = Token`. Transport: `Lease = Session`. |
| `Error` | Typed error enum per resource | Each resource has domain-specific errors (PgError, HttpError). `ClassifyError` macro maps to framework `ErrorKind`. Avoids one giant error enum. |
| `Credential` | Secret data resolved by framework | Framework resolves via `CredentialStore` BEFORE `create()`. Resource author declares the type; framework handles lookup, caching, rotation. `()` for resources without secrets. |

**Important distinction: Runtime vs Lease.** They may be the same type (Pool, Resident)
or different (Service, Transport). Never assume interchangeable. Use `R::Lease` for
caller-facing code, `R::Runtime` for internal lifecycle management.

### ResourceConfig — operational settings

```rust
pub trait ResourceConfig: Send + Sync + Clone + 'static {
    /// Validate configuration at registration time.
    fn validate(&self) -> Result<()> { Ok(()) }

    /// Stable fingerprint of compatibility-affecting fields.
    /// When fingerprint changes → existing instances stale → evicted at recycle.
    /// Returns 0 if config does NOT support stale detection (stateless resources).
    fn fingerprint(&self) -> u64 { 0 }
}
```

**fingerprint() contract:**
- `0` is correct for stateless resources (HttpConfig — no per-instance compatibility).
- `0` is a BUG for resources where config changes affect existing instances
  (PgResourceConfig: statement_timeout, search_path → must hash these fields).
- Use `FxHasher` (stable cross-process), NOT `DefaultHasher` (SipHash random seed).

### Credential — secret data

```rust
pub trait Credential: Send + Sync + Clone + 'static {
    /// Unique key. "database", "api_token", "telegram_bot", "ssh_key".
    const KIND: &'static str;
}

/// No credentials (HTTP client, stateless resource).
impl Credential for () { const KIND: &'static str = "none"; }
```

Framework resolution: `CredentialStore::resolve_erased(scope, C::KIND)` → `Box<dyn Any>`
→ downcast via `CredentialStoreExt::resolve::<C>()`. TypeId downcast protects against
KIND collisions — if two types share the same KIND, downcast fails with `TypeMismatch`.

**Config vs Credential separation:**
```
Credential (encrypted, rotatable):           Config (plain, operational):
  ✓ host, port, database                       ✓ timeouts (connect, statement, idle)
  ✓ username, password                          ✓ pool size (min, max)
  ✓ SSL certs                                   ✓ application name, client name
  ✓ API tokens, OAuth tokens                    ✓ recycle method
  ✓ Custom endpoints (GitHub Enterprise URL)    ✓ search_path defaults
                                                ✓ warmup strategy
```

### Choosing a topology

Follow this decision tree top-down. First match = your topology.

```
1. Does the resource need acquire/release?
   │
   ├─ NO → Background process only (polling, scheduling, scraping).
   │        → Daemon
   │
   └─ YES ↓
      │
      2. Is Runtime Clone AND Clone is cheap (Arc inside)?
         │
         ├─ YES → No per-caller mutable state?
         │        ├─ YES → Resident (one shared instance, clone on acquire)
         │        └─ NO  → Continue ↓
         │
         └─ NO ↓
            │
            3. One connection with N multiplexed sessions on top?
               ├─ YES → Transport (SSH: 1 TCP → N shells. AMQP: 1 TCP → N channels)
               │
               └─ NO ↓
                  │
                  4. Only one owner allowed at a time?
                     ├─ YES → Exclusive (Kafka Consumer, serial port)
                     │
                     └─ NO ↓
                        │
                        5. Long-lived runtime + lightweight tokens for callers?
                           ├─ YES → Service (Telegram Bot, WebSocket, rate-limited API)
                           │
                           └─ NO ↓
                              │
                              6. N interchangeable stateful instances?
                                 └─ YES → Pool (Postgres, SMTP, Browser, Redis dedicated)
```

**For incoming events:** add `EventSource` as secondary via `.also_event_source()`.
**For background loops:** add `Daemon` as secondary via `.also_daemon()`.
**Hybrids:** Telegram Bot = Service (primary) + EventSource + Daemon.

### Why 7 topologies (not fewer)

Each topology encodes a fundamentally different ownership × lifecycle pattern:

| Topology | Ownership | Lifecycle | Why not merge? |
|----------|-----------|-----------|----------------|
| **Pool** | Exclusive per-caller, returned after use | Checkout → recycle → idle | Needs idle queue, recycle, maintenance |
| **Resident** | Shared via Clone (Arc) | One instance, all clone | No checkout/recycle — Clone semantics |
| **Exclusive** | Exclusive, one at a time | Lock → use → reset → unlock | Semaphore, not pool (no N instances) |
| **Service** | Shared runtime, lightweight tokens | Runtime long-lived, tokens short | Token ≠ Clone (acquire_token has logic) |
| **Transport** | Shared connection, muxed sessions | Connection expensive, sessions cheap | Sessions have server-side resources |
| **EventSource** | Subscription (secondary) | Subscribe → recv → unsubscribe | Pull-based events, not acquire/release |
| **Daemon** | No caller access (secondary) | run() loop, restart on failure | No acquire at all — pure background |

- Resident ≠ Pool(max_size=1): Pool has idle queue, recycle, prepare, maintenance. Resident has ArcSwap Cell and Clone.
- Service ≠ Transport: Service tokens are local clones (Arc). Transport sessions consume server resources (SSH channel).
- EventSource ≠ Daemon: EventSource is pull-based (subscribe + recv). Daemon is push-based (run loop).

### Topology traits — what you implement

#### Pool (Pooled trait)

```rust
pub trait Pooled: Resource {
    /// Sync O(1) broken check. Called in Drop path — NO async, NO I/O.
    fn is_broken(&self, runtime: &Self::Runtime) -> BrokenCheck { BrokenCheck::Healthy }

    /// Async recycle on return to pool. Instance cleanup only.
    /// Policy decisions (stale config, max age) handled by framework BEFORE this.
    fn recycle(&self, runtime: &Self::Runtime, metrics: &InstanceMetrics)
        -> impl Future<Output = Result<RecycleDecision, Self::Error>> + Send
    { async { Ok(RecycleDecision::Keep) } }

    /// Prepare instance for specific execution context.
    /// Called AFTER checkout, BEFORE handing to caller.
    /// Use for: SET search_path, SET ROLE, inject correlation ID.
    fn prepare(&self, runtime: &Self::Runtime, ctx: &dyn Ctx)
        -> impl Future<Output = Result<(), Self::Error>> + Send
    { async { Ok(()) } }
}
```

**Pool config:** min_size, max_size, strategy (LIFO/FIFO), warmup (None/Sequential/Parallel/Staggered),
idle_timeout, max_lifetime, test_on_checkout, maintenance interval, check_policy, max_acquire_attempts,
recycle_workers (ReleaseQueue), max_concurrent_creates, create_timeout.

**Pool acquire flow:**
1. Try checkout from idle queue → is_broken()? → test_on_checkout? check() → prepare(ctx) → return
2. No idle → create() (respecting max_size, max_concurrent_creates) → prepare(ctx) → return
3. Pool full → wait (with timeout from AcquireOptions.deadline)
4. Retry blacklist: SmallVec<[InstanceId; 4]> tracks broken instances within one acquire cycle

**Pool release flow** (via ReleaseQueue):
1. is_broken()? → destroy
2. Framework policy: stale fingerprint? → destroy. max_lifetime exceeded? → destroy
3. resource.recycle(runtime, metrics) → Keep → push idle / Drop → destroy

**InstanceMetrics in recycle():** `error_count` (errors during lifetime), `checkout_count`
(times handed out), `created_at`, `age()`.

Example:
```rust
impl Pooled for Postgres {
    fn is_broken(&self, conn: &PgConnection) -> BrokenCheck {
        if conn.client.is_closed() { BrokenCheck::Broken("TCP closed".into()) }
        else if conn.conn_task.is_finished() { BrokenCheck::Broken("conn task done".into()) }
        else { BrokenCheck::Healthy }
    }

    async fn recycle(&self, conn: &PgConnection, metrics: &InstanceMetrics)
        -> Result<RecycleDecision, PgError>
    {
        if metrics.error_count >= 5 { return Ok(RecycleDecision::Drop); }
        if metrics.checkout_count >= 1000 { return Ok(RecycleDecision::Drop); }
        if conn.client.is_closed() { return Ok(RecycleDecision::Drop); }
        conn.client.simple_query("DISCARD ALL").await?;
        Ok(RecycleDecision::Keep)
    }

    async fn prepare(&self, conn: &PgConnection, ctx: &dyn Ctx) -> Result<(), PgError> {
        if let Some(tenant) = ctx.ext::<TenantContext>() {
            conn.client.simple_query(&format!("SET search_path TO {}", tenant.schema)).await?;
        }
        Ok(())
    }
}
```

#### Resident (Resident trait)

```rust
pub trait Resident: Resource where Self::Lease: Clone {
    /// Sync O(1) liveness check. NO I/O, NO blocking. Atomic flag only.
    fn is_alive_sync(&self, _runtime: &Self::Runtime) -> bool { true }

    /// Check interval. None = never (stateless clients).
    fn stale_after(&self) -> Option<Duration> { None }
}
```

Acquire = Cell::load() → Arc<Runtime> → clone → Owned handle. Zero contention.
If is_alive_sync() returns false → destroy old → create new → Cell::store(new).
For I/O health checks → use Resource::check() (async), not is_alive_sync.

Example: `reqwest::Client` — all defaults (stateless, always alive).
Example: `fred::Client` — `is_alive_sync: client.is_connected()`, `stale_after: 15s`.

#### Service (Service trait)

```rust
pub trait Service: Resource {
    /// Cloned: token cheap clone, release = noop. Owned handle.
    /// Tracked: token = tracked resource, release required. Guarded handle.
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    /// Create token for caller from long-lived runtime.
    fn acquire_token(&self, runtime: &Self::Runtime, ctx: &dyn Ctx)
        -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;

    /// Return token. Cloned = noop. Tracked = must implement.
    fn release_token(&self, runtime: &Self::Runtime, token: Self::Lease)
        -> impl Future<Output = Result<(), Self::Error>> + Send
    { let _ = (runtime, token); async { Ok(()) } }
}
```

Example: Telegram Bot — `TokenMode::Cloned`, acquire_token clones Bot + subscribes to broadcast.
Example: Rate-limited API — `TokenMode::Tracked`, acquire_token acquires semaphore permit.

#### Transport (Transport trait)

```rust
pub trait Transport: Resource {
    /// Open multiplexed session on shared transport connection.
    fn open_session(&self, transport: &Self::Runtime, ctx: &dyn Ctx)
        -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;

    /// Close session. healthy = session completed normally (vs error/cancel).
    fn close_session(&self, transport: &Self::Runtime, session: Self::Lease, healthy: bool)
        -> impl Future<Output = Result<(), Self::Error>> + Send
    { let _ = (transport, session, healthy); async { Ok(()) } }

    /// Keepalive on transport connection (not session).
    fn keepalive(&self, transport: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send
    { let _ = transport; async { Ok(()) } }
}
```

Example: SSH — open_session spawns bash child process, close_session drops child,
keepalive checks session alive. Sessions bounded via max_sessions semaphore (amendment #21).

#### Exclusive (Exclusive trait)

```rust
pub trait Exclusive: Resource {
    /// Reset between callers. Must handle partial state from cancelled callers.
    fn reset(&self, runtime: &Self::Runtime)
        -> impl Future<Output = Result<(), Self::Error>> + Send
    { let _ = runtime; async { Ok(()) } }
}
```

Acquire: semaphore.acquire() → Arc<Runtime> clone → Shared handle + permit.
Release: reset(runtime) → drop(permit) → next caller unblocked.
If reset fails → destroy + recreate.

Example: Kafka Consumer — reset commits offsets, pauses/resumes all partitions.

#### EventSource (secondary trait)

```rust
pub trait EventSource: Resource {
    type Event: Send + Clone + 'static;
    type Subscription: Send + 'static;

    fn subscribe(&self, runtime: &Self::Runtime, ctx: &dyn Ctx)
        -> impl Future<Output = Result<Self::Subscription, Self::Error>> + Send;

    fn recv(&self, subscription: &mut Self::Subscription)
        -> impl Future<Output = Result<Self::Event, Self::Error>> + Send;
}
```

Always paired with a primary topology. Redis Pub/Sub = Resident + EventSource.
Telegram Bot = Service + EventSource + Daemon.

#### Daemon (secondary trait)

```rust
pub trait Daemon: Resource {
    /// Background run loop. MUST respect CancellationToken.
    fn run(&self, runtime: &Self::Runtime, ctx: &dyn Ctx, cancel: CancellationToken)
        -> impl Future<Output = Result<(), Self::Error>> + Send;
}
```

Framework manages: start, cancel, restart per RestartPolicy (Never/OnFailure/Always).
RecreateBudget prevents infinite recreate loops.
debug_assert!(state.runtime.is_some()) at each loop iteration catches framework bugs.

### Registration — typestate builder

Invalid states don't compile. Forgot config → compile error. Forgot topology → compile error.

```rust
// Minimal — 3 lines:
manager.register(Postgres).config(pg_config).id(id)
    .pool(pool::Config::default()).build().await?;

// Full — all options:
manager.register(Postgres)
    .config(pg_config)                                       // NeedsConfig → NeedsId
    .id(resource_id)                                         // NeedsId → NeedsTopology
    .scope(ScopeLevel::Organization(org_id))                 // optional
    .recovery_group(RecoveryGroupKey::new("pg-primary"))     // optional
    .acquire_resilience(AcquireResilience { ... })           // optional
    .pool(pool::Config { max_size: 20, .. })                 // NeedsTopology → Ready
    .build().await?;                                         // Ready → registered

// Hybrid — Telegram Bot:
manager.register(TelegramBot)
    .config(tg_config).id(tg_id)
    .service(service::Config::default())               // primary
    .also_event_source(event_source::Config::default()) // secondary
    .also_daemon(daemon::Config::default())             // secondary
    .build().await?;

// Compile errors:
manager.register(Postgres).config(cfg).id(id).resident(res_cfg).build().await?;
//  ERROR: Postgres does NOT impl Resident

manager.register(Postgres).config(cfg).id(id).build().await?;
//  ERROR: topology not selected (still NeedsTopology state)
```

Typestate: `NeedsConfig → NeedsId → NeedsTopology → Ready<T>`.
`.pool()` available only if `R: Pooled`. `.resident()` only if `R: Resident` and `R::Lease: Clone`.
`.also_event_source()` / `.also_daemon()` available only on `Ready` state and if `R` impls those traits.

### Error classification — ClassifyError derive macro

Map resource errors to framework ErrorKind automatically:

```rust
#[derive(Debug, thiserror::Error, nebula_resource::ClassifyError)]
pub enum PgError {
    #[error("authentication failed")]
    #[classify(permanent)]
    Auth { user: String },

    #[error("connection failed: {0}")]
    #[classify(transient)]
    Connect(#[from] tokio_postgres::Error),

    #[error("too many connections")]
    #[classify(exhausted, retry_after = "30s")]
    TooManyConnections,

    #[error("bot blocked by user {chat_id}")]
    #[classify(transient, scope = target)]
    BotBlocked { chat_id: i64 },
}

// scope = target: uses first field as target ID (.to_string()).
// To specify field: #[classify(transient, scope = target, field = "chat_id")]
```

Macro generates `From<PgError> for nebula_resource::Error` with per-variant mapping.

### Resource author contracts

Hard invariants the Rust compiler cannot enforce. Violating these causes subtle
production bugs. Full details in `resource-author-contracts.md`.

| # | Contract | Violation consequence |
|---|----------|----------------------|
| 1 | `is_alive_sync()` — O(1), no I/O, no blocking | Tokio thread pool starvation |
| 2 | `is_broken()` — O(1), sync only (called in Drop) | Release pipeline backpressure |
| 3 | `fingerprint()` — hash all compatibility-affecting fields | Silent stale instances |
| 4 | `Service::Tracked` → must implement `release_token()` | Token/permit exhaustion |
| 5 | Transport sessions should be bounded via config | Unbounded sessions on transport |
| 6 | `Daemon::run()` must respect CancellationToken | Ungraceful shutdown |
| 7 | `detach()` → caller owns cleanup | Connection leak |
| 8 | `Exclusive::reset()` must be idempotent, handle partial state | Leaked state between callers |
| 9 | `recycle()` must not panic | Worker abort, pool exhaustion |
| 10 | `create()` must be cancel-safe — no zombie tasks | Leaked spawned tasks |

### Complete resource examples

**Postgres (Pool):**
```rust
pub struct Postgres;

impl Resource for Postgres {
    type Config     = PgResourceConfig;
    type Runtime    = PgConnection;
    type Lease      = PgConnection;       // = Runtime
    type Error      = PgError;
    type Credential = DatabaseCredential;
    const KEY: ResourceKey = resource_key!("postgres");

    async fn create(&self, config: &PgResourceConfig, cred: &DatabaseCredential, _ctx: &dyn Ctx)
        -> Result<PgConnection, PgError>
    {
        let (client, connection) = tokio_postgres::Config::new()
            .host(&cred.host).port(cred.port)
            .dbname(&cred.database).user(&cred.username)
            .password(cred.password.expose())
            .connect_timeout(config.connect_timeout)
            .connect(NoTls).await.map_err(PgError::Connect)?;
        let conn_task = tokio::spawn(async move { if let Err(e) = connection.await { warn!("{e}"); } });
        Ok(PgConnection::new(client, conn_task))
    }

    async fn check(&self, conn: &PgConnection) -> Result<(), PgError> {
        conn.client.simple_query("SELECT 1").await.map_err(PgError::HealthCheck)?;
        Ok(())
    }

    async fn destroy(&self, conn: PgConnection) -> Result<(), PgError> {
        drop(conn.client);
        let _ = tokio::time::timeout(Duration::from_secs(2), conn.conn_task).await;
        Ok(())
    }
}
```

**HTTP Client (Resident):**
```rust
pub struct HttpClient;

impl Resource for HttpClient {
    type Config     = HttpConfig;
    type Runtime    = reqwest::Client;
    type Lease      = reqwest::Client;    // = Runtime (Clone)
    type Error      = HttpError;
    type Credential = ();                 // no secrets
    const KEY: ResourceKey = resource_key!("http.client");

    async fn create(&self, config: &HttpConfig, _cred: &(), _ctx: &dyn Ctx)
        -> Result<reqwest::Client, HttpError>
    {
        reqwest::Client::builder().timeout(config.timeout).build().map_err(HttpError::Build)
    }
}

impl Resident for HttpClient {
    // All defaults. Stateless, always alive.
}
```

**Telegram Bot (Service + EventSource + Daemon):**
```rust
pub struct TelegramBot;

impl Resource for TelegramBot {
    type Config     = TelegramResourceConfig;
    type Runtime    = TelegramBotRuntime;
    type Lease      = TelegramBotHandle;    // ≠ Runtime
    type Error      = TelegramError;
    type Credential = TelegramCredential;
    const KEY: ResourceKey = resource_key!("telegram.bot");

    async fn create(&self, config: &TelegramResourceConfig, cred: &TelegramCredential, _ctx: &dyn Ctx)
        -> Result<TelegramBotRuntime, TelegramError>
    {
        let bot = Bot::new(cred.token.expose());
        let info = bot.get_me().await.map_err(TelegramError::Api)?;
        let (update_tx, _) = broadcast::channel(config.buffer_size);
        Ok(TelegramBotRuntime { inner: Arc::new(BotInner { bot, info, update_tx }) })
    }
}

impl Service for TelegramBot {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;
    async fn acquire_token(&self, runtime: &TelegramBotRuntime, _ctx: &dyn Ctx)
        -> Result<TelegramBotHandle, TelegramError>
    {
        Ok(TelegramBotHandle {
            bot: runtime.inner.bot.clone(),
            update_rx: runtime.inner.update_tx.subscribe(),
            info: Arc::clone(&runtime.inner.info),
        })
    }
}

impl EventSource for TelegramBot {
    type Event = TelegramUpdate;
    type Subscription = broadcast::Receiver<TelegramUpdate>;
    async fn subscribe(&self, rt: &TelegramBotRuntime, _ctx: &dyn Ctx)
        -> Result<Self::Subscription, TelegramError> { Ok(rt.inner.update_tx.subscribe()) }
    async fn recv(&self, sub: &mut Self::Subscription)
        -> Result<TelegramUpdate, TelegramError> { sub.recv().await.map_err(TelegramError::from) }
}

impl Daemon for TelegramBot {
    async fn run(&self, rt: &TelegramBotRuntime, _ctx: &dyn Ctx, cancel: CancellationToken)
        -> Result<(), TelegramError>
    {
        tokio::select! {
            _ = cancel.cancelled() => Ok(()),
            result = rt.poll_loop() => result,
        }
    }
}

// Registration:
manager.register(TelegramBot)
    .config(tg_config).id(tg_id)
    .service(service::Config::default())
    .also_event_source(event_source::Config::default())
    .also_daemon(daemon::Config { restart_policy: RestartPolicy::OnFailure, .. })
    .build().await?;
```

---

## Framework Internals View

This section is for agents working on **nebula-resource core** — Manager, Registry,
TopologyRuntime, ReleaseQueue, RecoveryGate, and the infrastructure that makes
topology transparency work.

### Architecture layers

```
┌─────────────────────────────────────────────────────┐
│ Action / Trigger Layer                              │
│   ctx.resource::<R>()  →  ResourceHandle<R>         │
│   (Section: Action Author View)                     │
├─────────────────────────────────────────────────────┤
│ Manager Layer                                       │
│   Manager, Registry, RegistrationBuilder            │
│   ShutdownOrchestrator, ScopeResolver               │
├─────────────────────────────────────────────────────┤
│ Runtime Layer                                       │
│   ManagedResource<R>, TopologyRuntime<R>            │
│   pool::Runtime, resident::Runtime, service::Runtime│
│   transport::Runtime, exclusive::Runtime            │
│   event_source::Runtime, daemon::Runtime            │
├─────────────────────────────────────────────────────┤
│ Recovery Layer                                      │
│   RecoveryGate, RecoveryGroup, WatchdogHandle       │
│   AcquireResilience (wraps nebula-resilience)       │
├─────────────────────────────────────────────────────┤
│ Primitive Layer                                     │
│   ResourceHandle<R>, HandleInner (3 variants)       │
│   LeaseGuard<L>, PoisonToken                        │
│   ReleaseQueue, Cell<T>                             │
│   Error, ErrorKind, ErrorScope                      │
│   Ctx, Extensions, BasicCtx                         │
│   ResourceKey, ResourceId, resource_key!()          │
├─────────────────────────────────────────────────────┤
│ Cross-cutting Integration                           │
│   ResourceMetrics (← nebula-telemetry)              │
│   ResourceEvent + EventBus (← nebula-eventbus)      │
│   AsyncConfigurable (← nebula-config)               │
│   MemoryMonitor (← nebula-memory)                   │
│   CredentialStore (← nebula-credential bridge)      │
└─────────────────────────────────────────────────────┘
```

### Manager — central orchestrator

```rust
pub struct Manager {
    registry:          Registry,                    // DashMap-based, type-erased
    recovery_groups:   RecoveryGroupRegistry,       // shared gates per backend
    cancel:            CancellationToken,           // global cancellation
    telemetry:         Arc<dyn TelemetryService>,   // observability
    resource_bus:      Arc<EventBus<ResourceEvent>>,// lifecycle events
    memory_monitor:    Option<Arc<Mutex<MemoryMonitor>>>,
    pressure_snapshot: Arc<PressureSnapshot>,        // AtomicU8, lock-free read
    containment_mode:  ContainmentMode,              // Strict (default) or Simplified
    scope_resolver:    Arc<dyn ScopeResolver>,       // parent-child scope validation
}
```

**Manager responsibilities:**
- `register()` → typestate builder → validate config → create ManagedResource → store in Registry
- `acquire()` → scope-aware lookup → RecoveryGate check → AcquireResilience → TopologyRuntime dispatch → ResourceHandle
- `remove()` → graceful shutdown of one resource
- `shutdown()` → ShutdownOrchestrator: cancel → reverse order → drain ReleaseQueues

**Scope containment:**
- `Strict` (default): `ScopeResolver::is_child_of()` validates parent-child via DB lookup.
  `CachedScopeResolver` wraps with moka cache (feature-gated via `scope-cache`).
- `Simplified`: any Organization contains all Projects. Single-tenant or dev only.
- No resolver configured + Strict mode → panic (RequiresScopeResolver forces explicit choice).

### Registry — type-erased, scope-aware lookup

```rust
struct Registry {
    /// Primary index: (TypeId of R, ResourceId) → scoped runtimes.
    by_type: DashMap<(TypeId, ResourceId), SmallVec<[ScopedRuntime; 4]>>,
    /// Secondary index: ResourceKey → TypeId.
    by_key: DashMap<ResourceKey, TypeId>,
}

struct ScopedRuntime {
    scope:   ScopeLevel,
    managed: Arc<dyn AnyManagedResource>,
}
```

**Two lookup paths:**
- **Typed hot path:** `get_typed::<R>()` → downcast_ref to `Arc<ManagedResource<R>>`. Zero allocation.
- **Erased cold path:** `get_erased()` via ResourceKey → TypeId → AnyManagedResource.

**Scope resolution:** find most-specific registered scope compatible with request scope.
Order: Action → Execution → Workflow → Project → Organization → Global.
ScopeResolver validates cross-level containment (async, cached).

### ManagedResource — per-registration runtime

```rust
pub struct ManagedResource<R: Resource> {
    resource:          R,
    config:            ArcSwap<R::Config>,
    topology:          TopologyRuntime<R>,
    recovery_gate:     Option<Arc<RecoveryGate>>,
    resilience:        Option<ResiliencePipeline>,
    release_queue:     ReleaseQueue,
    metrics:           ResourceMetrics,
    cancel:            CancellationToken,       // child of Manager.cancel
    generation:        AtomicU64,               // incremented on reload/recreate
    status:            ArcSwap<ResourceStatus>,  // phase + generation + last_error
}
```

**generation tracking:** Each reload/recreate increments generation. HandleInner::Guarded
and Shared store generation at acquire time. After reload, handles from old generation
are detected as stale — useful for drain tracking and diagnostics.

**ResourceStatus:** Framework updates phase transitions:
Initializing → Ready → Reloading → Ready (on success)
Ready → Failed (on check/create failure, may recover via RecoveryGate)
Ready → ShuttingDown (on Manager.cancel)
Reloading → Draining → Ready (Service: old runtime draining via Arc refcount)

### TopologyRuntime — dispatch enum

```rust
pub enum TopologyRuntime<R: Resource> {
    Pool(pool::Runtime<R>),
    Resident(resident::Runtime<R>),
    Service(service::Runtime<R>),
    Transport(transport::Runtime<R>),
    Exclusive(exclusive::Runtime<R>),
    EventSource(event_source::Runtime<R>),
    Daemon(daemon::Runtime<R>),
}
```

Each variant implements acquire/release with topology-specific logic.
`on_config_changed()` returns `ReloadOutcome`:

```rust
pub enum ReloadOutcome {
    SwappedImmediately,                     // Pool (fingerprint), Resident (ArcSwap), Transport, Exclusive
    PendingDrain { old_generation: u64 },   // Service (Arc refcount drain)
    Restarting,                             // Daemon (cancel + restart)
    NoChange,                               // fingerprint identical
}
```

### ResourceHandle — unified caller handle

```rust
pub struct ResourceHandle<R: Resource> {
    inner: HandleInner<R>,
    resource_key: ResourceKey,
    topology_tag: &'static str,
}

enum HandleInner<R: Resource> {
    /// Owned value, no cleanup. Resident clone, Service Cloned token.
    Owned(R::Lease),

    /// Owned value + async cleanup callback. Pool, Transport, Service Tracked.
    Guarded {
        value: Option<R::Lease>,
        on_release: Option<Box<dyn FnOnce(R::Lease, bool) + Send>>,
        tainted: bool,
        acquired_at: Instant,
    },

    /// Shared ref + async cleanup callback. Exclusive.
    Shared {
        value: Arc<R::Lease>,
        on_release: Option<Box<dyn FnOnce(bool) + Send>>,
        tainted: bool,
        acquired_at: Instant,
    },
}
```

**Deref:** all three variants Deref to `&R::Lease`.
**Drop:** Owned = noop. Guarded = on_release(lease, tainted) sync submit to ReleaseQueue.
Shared = on_release(tainted) sync submit to ReleaseQueue.
**Public API:** `taint()`, `detach()`, `hold_duration()`, `resource_key()`, `topology_tag()`.

### ReleaseQueue — async cleanup workers

One ReleaseQueue per ManagedResource. Handles post-release cleanup (recycle, destroy,
session close, permit release).

**Architecture:**
- N primary workers — each owns its own `mpsc::Receiver` (no Mutex on hot path).
  Worker count by topology: Pool configurable (Postgres=1, Browser=4), Transport=1, Exclusive=1.
- 1 dedicated fallback worker — sole owner of bounded(10k) fallback receiver.
  Used when primary workers full (burst). No Mutex, no contention.
- Round-robin submit on sender side.
- 30s timeout on every release_fn execution (prevents worker paralysis when backend hangs).

**OOM protection:** Fallback is bounded (10,000 capacity). If both primary and fallback
full → task dropped (intentional fail-open). Leaked connection detected by pool maintenance.

**Metrics:** `submitted`, `fallback_used`, `dropped` (must be 0 in healthy system), `timed_out`.

**Shutdown:** Drop all senders → workers drain remaining tasks → exit. Fallback worker
drains its queue too.

### RecoveryGate — thundering herd prevention

CAS-based state machine preventing multiple callers from probing a dead backend simultaneously.

```
         ┌──────────┐   try_begin() wins CAS    ┌────────────┐
  ──────►│   Idle   │──────────────────────────►│ InProgress │
         └──────────┘                            │ {attempt}  │
               ▲                                 └───┬────┬───┘
               │                                     │    │
        resolve(ticket)                    fail_transient  fail_permanent
               │                                     │    │
               │           ┌──────────────┐          │    │
               │           │   Failed     │◄─────────┘    │
               │           │{err, until,  │               ▼
               │           │ attempt}     │    ┌──────────────────┐
               │           └──────┬───────┘    │PermanentlyFailed │
               │                  │            └──────────────────┘
               │     backoff expires                ▲
               │     + attempt < max                │ attempt >= max
               │          │                         │
               │          └──► InProgress ──────────┘
               │                    │
               └────────────────────┘
                     resolve(ticket)
```

**Key invariant:** Only ONE caller holds a `RecoveryTicket` at a time. All others get
`RecoveryWaiter` and block on `Notify`.

**RecoveryTicket Drop guard:** If ticket dropped without resolve/fail → auto-fail with
transient error + short backoff. Prevents gate stuck in InProgress forever.

**RecoveryGroup:** Multiple resources on same backend (Postgres primary → N pools) share
one RecoveryGate. Registered via `RecoveryGroupKey` on builder.

**NOTE:** Uses `ArcSwap::compare_and_swap`. May be deprecated in future arc-swap versions.
Current usage correct because `rcu()` doesn't support early return. Pin arc-swap version.

### Health check levels

Three distinct levels, from local to global:

| Level | Mechanism | Scope | Question |
|-------|-----------|-------|----------|
| Instance | `is_broken()`, `Resource::check()` | Single runtime | "Is this connection alive?" |
| Acquire | `AcquireResilience` (timeout/retry/CB) | Per-resource | "Can I get an instance right now?" |
| Backend | `RecoveryGate` + `RecoveryGroup` | Shared infra | "Is the backend reachable?" |

**Ordering in acquire path:** Backend (step 1) → Acquire (step 2) → Instance (step 3).
If backend down → don't even try. If acquire retry exhausted → trigger passive recovery probe.

### AcquireResilience — per-resource protection

Wraps acquire operation with timeout → retry → circuit breaker. Built from config at
registration time using `nebula-resilience::PipelineBuilder`.

```rust
pub struct AcquireResilience {
    pub timeout: Option<Duration>,
    pub retry: Option<AcquireRetryConfig>,         // max_attempts, backoff kind
    pub circuit_breaker: Option<AcquireCircuitBreakerPreset>,  // Standard/Fast/Slow
}
```

**Presets:**
- Standard (5 failures, 30s reset) — databases, message brokers
- Fast (3 failures, 10s reset) — caches, HTTP APIs
- Slow (10 failures, 60s reset) — SSH, SMTP, Browser, LLM

### WatchdogHandle — opt-in background probe

Periodic health check for topologies without natural liveness detection:
- Service: monitor polling loop alive
- Transport: monitor connection + keepalive

Not needed for: Pool (test_on_checkout), Resident (stale_after), HTTP (stateless).

Config: interval, probe_timeout, failure_threshold, recovery_threshold, auto_recover.

### Config hot-reload

Signal: `nebula-config` → `AsyncConfigurable` → `Manager` → `TopologyRuntime::on_config_changed()`.

Per-topology strategy:
| Topology | Strategy | ReloadOutcome |
|----------|----------|---------------|
| Pool | Update fingerprint → lazy eviction at recycle | SwappedImmediately |
| Resident | Destroy old → create new (ArcSwap swap) | SwappedImmediately |
| Service | Create new runtime, old drains via Arc refcount | PendingDrain |
| Transport | Destroy old → create new | SwappedImmediately |
| Exclusive | Destroy old → create new | SwappedImmediately |
| EventSource | Unsubscribe → resubscribe | SwappedImmediately |
| Daemon | Cancel → restart with new config | Restarting |

### Credential rotation

Reactive flow via `EventBus<CredentialRotatedEvent>`:
- Pool: sets stale fingerprint → instances evicted at next recycle → recreate with new cred.
- Resident/Service/Daemon: destroy + create with new credential.
- Resource author does nothing — framework responsibility.
- `CredentialStore` always returns current credential.

### Shutdown orchestration

`ShutdownOrchestrator::shutdown(manager, config)`:
1. **Phase 1 — SIGNAL:** `manager.cancel.cancel()` → all resources notified. Wait `drain_timeout`.
2. **Phase 2 — CLEANUP:** Reverse registration order (v1; topological sort in v2).
   Per-resource: `shutdown()` → drain ReleaseQueue → `destroy()` all instances.
   Per-resource timeout = `cleanup_timeout`.
3. **Phase 3 — TERMINATE:** Force-close resources that didn't complete. Log leaked handles.

Service topology: drain uses Arc refcount. Leaked handles prevent old runtime destruction.
Watchdog logs warnings but cannot force-drop Arc holders (amendment #10).

### Scoped resources (per-execution)

`ResourceAction` creates per-execution or per-branch resources. Stored in `ScopedResourceMap`
passed down the DAG execution. Lookup walks ancestor chain (closest wins).

**Ordering:** Scoped removed from map → cleanup() → shutdown() → destroy().
cleanup() ctx resolves to global (scoped already removed).

**Scope conflicts:** Same type + same branch level = registration-time error.
Different branches = independent, no conflict.

### Metrics and events

**ResourceMetrics** wraps `TelemetryAdapter`:
- Counters: acquire_total, acquire_error, release_total, create_total, destroy_total
- Histograms: acquire_duration, hold_duration, create_duration, recycle_duration
- Gauges: pool_idle, pool_active, pool_size

**ResourceEvent** via `EventBus<ResourceEvent>`:
- Registered, Removed, HealthChanged, RecoveryStarted, RecoveryCompleted, ConfigReloaded

### Testing infrastructure (Phase 6)

Behind `test-support` feature flag:

```
nebula-resource/src/testing/
├── mod.rs              // #[cfg(feature = "test-support")]
├── mock_provider.rs    // TestContext::inject::<R>(mock_handle)
├── contract.rs         // resource_contract_tests!() macro
├── fault.rs            // FaultInjector
└── harness.rs          // TestManager, cancel/drop stress helpers
```

`resource_contract_tests!()`: reusable test suite verifying Resource+Topology impls —
happy path, concurrent acquire/release, cancellation, drop/panic paths.

`FaultInjector`: inject transient/permanent errors into create/check/recycle.

Cancel-safety tests: verify Exclusive semaphore fairness under cancellation.

### Module layout

```
nebula-resource/src/
├── lib.rs                      // pub use, feature gates
├── resource.rs                 // Resource, ResourceConfig traits
├── ctx.rs                      // Ctx trait, BasicCtx, Extensions
├── error.rs                    // Error, ErrorKind(6), ErrorScope
├── classify.rs                 // ClassifyError derive support
├── metadata.rs                 // ResourceMetadata
├── credential.rs               // Credential trait, CredentialStore bridge
├── handle.rs                   // ResourceHandle<R>, HandleInner
├── cell.rs                     // Cell<T> (ArcSwap-based)
├── state.rs                    // AtomicRuntimeState, ResourceStatus, ResourcePhase
├── release_queue.rs            // ReleaseQueue + ReleaseQueueHandle
├── scope.rs                    // ResourceScope
├── dependency.rs               // Dependencies trait
├── health.rs                   // HealthStatus, HealthChecker
├── events.rs                   // ResourceEvent + ScopedEvent
├── metrics.rs                  // ResourceMetrics wrapper
├── topology/
│   ├── mod.rs                  // TopologyKind enum
│   ├── pooled.rs               // Pooled trait
│   ├── resident.rs             // Resident trait
│   ├── service.rs              // Service trait
│   ├── transport.rs            // Transport trait
│   ├── exclusive.rs            // Exclusive trait
│   ├── event_source.rs         // EventSource trait
│   └── daemon.rs               // Daemon trait
├── lease/
│   ├── guard.rs                // LeaseGuard<L>
│   ├── poison.rs               // PoisonToken
│   └── options.rs              // AcquireOptions, AcquireIntent
├── runtime/
│   ├── mod.rs                  // TopologyRuntime<R> enum
│   ├── managed.rs              // ManagedResource<R>, AnyManagedResource
│   ├── pool/                   // config, entry, idle_queue, acquire, release, maintenance
│   ├── resident/               // config, health
│   ├── service/                // config
│   ├── transport/              // config, session
│   ├── exclusive/              // config
│   ├── event_source/           // config, handle
│   └── daemon/                 // config, runner
├── recovery/
│   ├── gate.rs                 // RecoveryGate, GateState, RecoveryTicket
│   ├── group.rs                // RecoveryGroup, RecoveryGroupRegistry
│   └── watchdog.rs             // WatchdogHandle, WatchdogConfig
├── integration/
│   ├── resilience.rs           // AcquireResilience
│   ├── config.rs               // AsyncConfigurable
│   └── memory.rs               // Adaptive pool sizing
├── registry/
│   ├── mod.rs                  // Registry (DashMap)
│   ├── lookup.rs               // Scope-aware lookup
│   └── scoped.rs               // ScopedRuntime
├── manager/
│   ├── mod.rs                  // Manager struct
│   ├── builder.rs              // RegistrationBuilder (typestate)
│   ├── acquire.rs              // acquire dispatch
│   └── shutdown.rs             // ShutdownOrchestrator
├── testing/                    // #[cfg(feature = "test-support")]
│   ├── mock_provider.rs
│   ├── contract.rs
│   ├── fault.rs
│   └── harness.rs
└── macros/
    ├── derive_resource.rs      // #[derive(Resource)]
    └── derive_classify.rs      // #[derive(ClassifyError)]
```

### Implementation phases

| Phase | Content | Depends on |
|-------|---------|------------|
| 1 (week 1-2) | Core primitives: Error, Ctx, Resource trait, ResourceConfig, Credential, Cell, LeaseGuard, ResourceHandle, ReleaseQueue | — |
| 2 (week 2-3) | 7 topology traits | Phase 1 |
| 3 (week 3) | RecoveryGate, RecoveryGroup, Watchdog, AcquireResilience, events, metrics | Phase 1 |
| 4 (week 3-4) | 7 topology runtime implementations, TopologyRuntime enum, ManagedResource | Phases 2, 3 |
| 5 (week 4-5) | Registry, Manager, RegistrationBuilder, acquire dispatch, ShutdownOrchestrator, config reload, scope | Phase 4 |
| 6 (week 5-6) | Derive macros, action bridges, plugin integration, testing module, first resources | Phase 5 |
