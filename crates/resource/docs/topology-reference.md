# Topology Reference for Resource Authors

> **Audience:** plugin authors writing a new `Resource` impl. Maps each of the
> four most-common topologies (Pool, Resident, Service, Transport) to a
> minimal Rust skeleton, the trait set you must implement, when to pick
> that topology over the others, and the friction points that surfaced
> during the M6.3 prototyping pass.
>
> See [`README.md`](README.md) for the full library overview, and
> [`examples/examples/m6_*.rs`](../../../examples/examples) for runnable
> end-to-end demonstrations of each pattern (Pool / Resident / Service).

---

## Decision matrix

| Topology | Lease == Runtime? | Use when… | Don't use for… |
|---|---|---|---|
| **[Pool](#pool)** | Yes (`Lease = Runtime`) | N stateful instances, expensive to create, interchangeable; e.g. database connections. | Single-shared HTTP clients (use Resident); transport multiplexed sessions (use Transport). |
| **[Resident](#resident)** | Yes (`Lease = Runtime`, `Clone`) | One instance shared widely, cloning is cheap (`Arc`); e.g. `reqwest::Client`, in-memory cache. | Per-caller mutable state (use Pool); event-driven sources where lease holds receivers (use Service + EventSource). |
| **[Service](#service)** | No (`Lease ≠ Runtime`) | Long-lived runtime (e.g. bot client + broadcast bus) that hands out short-lived caller tokens. | Stateless HTTP (use Resident); per-tenant connection isolation (use Pool). |
| **[Transport](#transport)** | No (`Lease ≠ Runtime`) | Expensive shared connection + cheap multiplexed sessions; e.g. SSH session, gRPC channel. | Stateless requests (use Resident); short-lived per-call work (use Pool). |

`Exclusive` is a fifth, less-common topology — single-caller serialized
access via `Semaphore(1)`. Use it only when the underlying resource is
genuinely sequential (think a USB-serial adapter); otherwise reach for
Pool with `max_size = 1`.

---

## Pool

**Two interchangeable stateful instances managed by a checkout / recycle / destroy lifecycle.**

### Trait set

```rust
impl Resource for Postgres {
    type Config = PostgresConfig;
    type Runtime = PgConnection;
    type Lease = PgConnection;       // Pool: Lease = Runtime
    type Error = PgError;
    fn key() -> ResourceKey { resource_key!("demo.postgres") }
    async fn create(&self, config: &Self::Config, ctx: &ResourceContext) -> Result<Self::Runtime, Self::Error> { /* … */ }
    async fn destroy(&self, runtime: Self::Runtime) -> Result<(), Self::Error> { /* … */ }
    fn metadata() -> ResourceMetadata { ResourceMetadata::from_key(&Self::key()) }
}

impl Pooled for Postgres {
    fn is_broken(&self, runtime: &Self::Runtime) -> BrokenCheck { /* sync, O(1) */ }
    async fn recycle(&self, runtime: &Self::Runtime, metrics: &InstanceMetrics) -> Result<RecycleDecision, Self::Error> { /* … */ }
    // Optional: `prepare(&self, runtime, ctx)` for per-checkout setup.
}
```

### Registration

```rust
manager.register_pooled(
    Postgres,
    PostgresConfig::default(),
    PoolConfig {
        min_size: 0,
        max_size: 8,
        ..PoolConfig::default()
    },
)?;
```

### Lifecycle

`create` (lazy on first acquire, eager via `warmup`) → `prepare` (per
checkout) → caller holds `ResourceGuard` → on drop: tainted? destroy.
healthy? → `recycle` returns `Keep` / `Drop` → idle queue or destroy.

### Friction points (from M6.3 prototyping)

- **Sync `is_broken`.** Runs in the `Drop` path of `ResourceGuard`. No I/O, no async — read atomic flags only. If you must check the network, do it in `recycle` (async).
- **`fingerprint()` semantics.** Hash only fields that make existing instances stale. `application_name` and `statement_timeout` matter; `max_size` does not.
- **`AbortOnDrop` for spawned tasks.** If your `create` spawns a background task (e.g. a connection-handler task), wrap the `JoinHandle` in an `AbortOnDrop` so it is killed when the runtime drops mid-acquire (cancel-safety contract #10).

### Runnable example

[`examples/examples/m6_postgres_pool.rs`](../../../examples/examples/m6_postgres_pool.rs)

---

## Resident

**A single shared instance, cloned on every acquire.**

### Trait set

```rust
impl Resource for GoogleSheets {
    type Config = GoogleSheetsConfig;
    type Runtime = GoogleSheetsClient;       // typically wraps Arc internals
    type Lease = GoogleSheetsClient;         // Resident: Lease = Runtime, Clone
    type Error = SheetsError;
    fn key() -> ResourceKey { resource_key!("demo.google.sheets") }
    async fn create(&self, config: &Self::Config, ctx: &ResourceContext) -> Result<Self::Runtime, Self::Error> { /* … */ }
    async fn destroy(&self, runtime: Self::Runtime) -> Result<(), Self::Error> { /* … */ }
    fn metadata() -> ResourceMetadata { ResourceMetadata::from_key(&Self::key()) }
}

impl Resident for GoogleSheets {
    fn is_alive_sync(&self, _runtime: &Self::Runtime) -> bool { true }
    fn stale_after(&self) -> Option<Duration> { None } // or Some(Duration::from_secs(45 * 60)) for token TTL
}
```

### Registration

```rust
manager.register(
    GoogleSheets::new(cred),
    GoogleSheetsConfig { application: "nebula-demo".into() },
    ScopeLevel::Global,
    TopologyRuntime::Resident(ResidentRuntime::<GoogleSheets>::new(ResidentConfig::default())),
    None,
    None,
)?;
```

### Cross-workflow dedupe

Manager dedupes by `(R::key(), ScopeLevel)`. 10 concurrent acquires at the
same scope produce **one** `Resource::create` invocation; every lease is
`Arc::ptr_eq` to every other. See `examples/examples/m6_telegram_multi_workflow.rs`
(uses Resident topology to demonstrate the dedupe assertion explicitly).

### Friction points

- **`fingerprint() = 0` is correct.** Resident has only one instance, so a config change forces destroy + recreate; there's no "stale fingerprint" sweep to drive.
- **Token caching inside `Runtime`.** OAuth-style integrations cache an access token inside the Runtime. On token expiry, two valid patterns exist:
  - **Reactive:** detect 401 → refresh inline + retry (used by `m6_resident_http.rs`).
  - **Proactive (preferred for high-throughput):** use `Resident::stale_after(Some(token_ttl - safety_margin))` and let the manager destroy + recreate before tokens expire. Simpler, no inline retry.
- **`Clone` requirement on `Runtime`.** Inner state typically lives behind `Arc<Inner>` so `Clone` is a refcount bump.

### Runnable example

[`examples/examples/m6_resident_http.rs`](../../../examples/examples/m6_resident_http.rs) (with OAuth refresh)

[`examples/examples/m6_telegram_multi_workflow.rs`](../../../examples/examples/m6_telegram_multi_workflow.rs) (cross-workflow sharing assertion)

---

## Service

**Long-lived runtime + short-lived caller tokens (`Lease ≠ Runtime`).**

The runtime owns infrastructure (e.g. a Telegram `Bot` + broadcast bus); the lease is a small token (e.g. `Bot` clone + token-only outbound API). Use it when the caller-facing surface differs from the framework-facing infrastructure.

### Trait set

```rust
impl Resource for TelegramBot {
    type Config = TelegramConfig;
    type Runtime = TelegramBotRuntime;     // owns Arc<BotInner>
    type Lease = TelegramBotHandle;        // outbound-only API
    type Error = TelegramError;
    // create / destroy / metadata as for Pool/Resident
}

impl Service for TelegramBot {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;
    async fn acquire_token(&self, runtime: &Self::Runtime, ctx: &dyn Ctx) -> Result<Self::Lease, Self::Error> { /* … */ }
}
```

### Optional companions

`Service` is one of the three topology traits a single resource can mix.
The Telegram example shows the **hybrid** pattern: `Service` (outbound
calls) + `EventSource` (incoming updates, per ADR-0045 used directly until
the `EventTrigger` DX wrapper ships in §M6.4) + `Daemon` (background
polling loop). Hybrid registration via:

```rust
manager.register_service_with(/* … */)?;
// Or via the underlying `manager.register(...)` form for full control.
```

### Friction points

- **`broadcast::Receiver` + `&self`.** The receiver needs `&mut self`, but `ResourceGuard` derefs as `&Lease`. **Resolution (validated in M6.3):** keep the receiver inside the Runtime + EventSource trait path; the action-facing Lease exposes only outbound methods. Action authors that want to consume events use `EventTrigger` (or directly subscribe to the broadcast channel; see ADR-0045).
- **Service drain on config reload.** Old runtime stays alive while existing leases (which `Arc::clone` `BotInner`) are still in flight; new acquires get the new runtime.

### Runnable example

[`examples/examples/m6_telegram_multi_workflow.rs`](../../../examples/examples/m6_telegram_multi_workflow.rs) (uses Resident in the example for cross-workflow assertion clarity, but documents the Service shape inline)

---

## Transport

**Shared connection + cheap multiplexed sessions (`Lease ≠ Runtime`).**

Used for SSH-like patterns: one expensive TCP + key-exchange + auth, then
cheap "session" channels on top.

### Trait set

```rust
impl Resource for Ssh {
    type Config = SshResourceConfig;
    type Runtime = SshRuntime;          // one TCP session
    type Lease = SshSession;            // cheap multiplexed channel
    type Error = SshError;
    // create / destroy / metadata
}

impl Transport for Ssh {
    async fn open_session(&self, runtime: &Self::Runtime, ctx: &dyn Ctx) -> Result<Self::Lease, Self::Error> { /* … */ }
    async fn close_session(&self, runtime: &Self::Runtime, session: Self::Lease, healthy: bool) -> Result<(), Self::Error> { /* … */ }
    async fn keepalive(&self, runtime: &Self::Runtime) -> Result<(), Self::Error> { /* … */ }
}
```

### Friction points

- **`max_sessions` only in `transport::Config`.** Earlier prototype kept a duplicate in `SshResourceConfig`; resolution: the framework owns the semaphore via `transport::Config.max_sessions`. Resource config is for resource-specific settings only.
- **Temp key file lifecycle.** Most SSH crates need a file path for the private key. Write it to a `tempfile` with `0600` permissions in `create`, delete in `destroy`. If `create` is cancelled mid-flight the file leaks — acceptable; the OS reclaims `/tmp`. Better long-term: use SSH agent forwarding.
- **`keepalive()` runs at the transport level, not the session level.** Prevents `sshd ClientAliveInterval` from closing the idle connection.

### When you'd reach for it

- Multiple commands over a single SSH connection.
- A long-lived gRPC channel with multiplexed RPC streams.
- Anything where the connection itself is expensive (TLS handshake, key exchange) but the per-call work is cheap.

(No runnable example is shipped for Transport in M6.3 — the Pool example
covers the pool semantics, and Service/Resident cover the `Lease ≠ Runtime`
distinction. Add a `m6_ssh_transport.rs` companion if you implement an SSH
plugin.)

---

## Cross-cutting checklist for a new Resource impl

Before sending the PR, verify:

- [ ] **`fingerprint`** hashes only fields that make existing instances stale.
- [ ] **`is_broken` / `is_alive_sync`** are sync, O(1), no I/O. Reads atomic flags or pointer state.
- [ ] **`destroy`** consumes `Runtime` (takes `self: Self::Runtime`, not `&Self::Runtime`).
- [ ] **`Drop`** of the `Runtime` releases OS resources without `await`. If the runtime spawns tasks, wrap their `JoinHandle` in `AbortOnDrop`.
- [ ] **Cancel-safety:** if `create` does anything observable before returning a Runtime, ensure that path is idempotent or has a guard that cleans up on cancel.
- [ ] **Errors:** every variant of your `Error` enum classifies via `nebula_resource::ClassifyError` (transient / permanent / exhausted / backpressure / target-scope). Action authors rely on this for retry decisions.
- [ ] **Credential slots** (per ADR-0044) are declared as `#[credential(key = "...")]` fields, **not** through the deprecated `Resource::Credential` associated type.

---

## See also

- [`README.md`](README.md) — library overview, public API surface
- [`pooling.md`](pooling.md) — Pool topology deep-dive (warmup strategies, metrics, stats)
- [`recovery.md`](recovery.md) — recovery gating and resilience composition
- [`events.md`](events.md) — lifecycle event streaming
- [`adapters.md`](adapters.md) — accessor adapters (`HasResources`, `ScopedResourceAccessor`)
- [`api-reference.md`](api-reference.md) — full surface API listing
- [`crates/resource/plans/02-topology.md`](../plans/02-topology.md) — historical design rationale for the topology split
- [`docs/adr/0044-supersede-0036-resource-credential-singular.md`](../../../docs/adr/0044-supersede-0036-resource-credential-singular.md) — supersession of `Resource::Credential` in favor of `#[credential]` slots
- [`docs/adr/0045-eventtrigger-scope-deferral.md`](../../../docs/adr/0045-eventtrigger-scope-deferral.md) — why EventSource-direct is the canonical pattern until `EventTrigger` ships
