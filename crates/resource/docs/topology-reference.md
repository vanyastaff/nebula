# Topology Reference for Resource Authors

> **Audience:** plugin authors writing a new `Resource` impl. Maps each of
> the two topology traits (`Pooled`, `Resident`) to a minimal Rust skeleton,
> the trait set you must implement, when to pick it, and the friction points.
>
> See [`README.md`](README.md) for the full library overview, and the
> `examples/` workspace member for runnable end-to-end demonstrations of
> each pattern.

---

## Decision matrix

| Topology                  | Lease == Runtime?              | Use when…                                                                               | Don't use for…                                                    |
|---------------------------|--------------------------------|-----------------------------------------------------------------------------------------|-------------------------------------------------------------------|
| **[Pool](#pool)**         | Yes (`Lease = Runtime`)        | N interchangeable stateful instances; expensive to create (DB connections).             | Single-shared HTTP client (use Resident).                         |
| **[Resident](#resident)** | Yes (`Lease = Runtime`, `Clone`) | One instance shared widely; `Arc::clone` is cheap (`reqwest::Client`, in-memory cache). | Per-caller mutable state (use Pool).                              |

---

## Pool

**N interchangeable stateful instances managed by a checkout / recycle / destroy lifecycle.**

### Trait set

```rust,ignore
impl Resource for Postgres {
    type Config = PostgresConfig;
    type Runtime = PgConnection;
    type Lease = PgConnection;       // Pool: Lease = Runtime
    type Error = PgError;

    fn key() -> ResourceKey { resource_key!("demo.postgres") }

    fn create(
        &self, config: &Self::Config, ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send { /* … */ }

    async fn destroy(&self, runtime: Self::Runtime) -> Result<(), Self::Error> { /* … */ }
}

impl Pooled for Postgres {
    fn is_broken(&self, runtime: &Self::Runtime) -> BrokenCheck { /* sync, O(1) */ }

    async fn recycle(
        &self, runtime: &Self::Runtime, metrics: &InstanceMetrics,
    ) -> Result<RecycleDecision, Self::Error> { /* … */ }

    // Optional: `prepare(&self, runtime, ctx)` for per-checkout setup.
}
```

### Registration

```rust,ignore
let pool_rt = PoolRuntime::<Postgres>::try_new(
    PoolConfig { min_size: 0, max_size: 8, ..PoolConfig::default() },
    pg_config.fingerprint(),
)?;

manager.register(RegistrationSpec {
    resource: Postgres,
    config: pg_config,
    scope: ScopeLevel::Global,
    slot_identity: SlotIdentity::Unbound,
    topology: TopologyRuntime::Pool(pool_rt),
    acquire: Manager::erased_acquire_pooled_for::<Postgres>(),
    recovery_gate: None,
})?;
```

### Lifecycle

`create` (lazy on first acquire, eager via `warmup`) → optional `prepare`
(per checkout) → caller holds `ResourceGuard` → on drop: tainted? destroy.
healthy? → `recycle` returns `Keep` / `Drop` → idle queue or destroy.

### Friction points

- **Sync `is_broken`.** Runs in the `Drop` path of `ResourceGuard`. No I/O,
  no async — read atomic flags only. If you must check the network, do it
  in `recycle` (async).
- **`fingerprint()` semantics.** Hash only fields that make existing
  instances stale. `application_name` and `statement_timeout` matter;
  `max_size` does not.
- **`AbortOnDrop` for spawned tasks.** If your `create` spawns a background
  task, wrap the `JoinHandle` in an `AbortOnDrop` so it is killed when the
  runtime drops mid-acquire.

### Runnable example

[`examples/examples/resource_postgres_pool.rs`](../../../examples/examples/resource_postgres_pool.rs)

---

## Resident

**A single shared instance, cloned on every acquire.**

### Trait set

```rust,ignore
impl Resource for GoogleSheets {
    type Config = GoogleSheetsConfig;
    type Runtime = GoogleSheetsClient;       // typically wraps Arc internals
    type Lease = GoogleSheetsClient;         // Resident: Lease = Runtime, Clone
    type Error = SheetsError;

    fn key() -> ResourceKey { resource_key!("demo.google.sheets") }

    fn create(
        &self, config: &Self::Config, ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send { /* … */ }
}

impl Resident for GoogleSheets {
    fn is_alive_sync(&self, _runtime: &Self::Runtime) -> bool { true }
    fn stale_after(&self) -> Option<Duration> { None }
}
```

### Registration

```rust,ignore
let resident_rt = ResidentRuntime::<GoogleSheets>::new(ResidentConfig::default());

manager.register(RegistrationSpec {
    resource: GoogleSheets::new(cred),
    config: GoogleSheetsConfig { application: "nebula-demo".into() },
    scope: ScopeLevel::Global,
    slot_identity: SlotIdentity::Unbound,
    topology: TopologyRuntime::Resident(resident_rt),
    acquire: Manager::erased_acquire_resident_for::<GoogleSheets>(),
    recovery_gate: None,
})?;
```

### Cross-workflow dedupe

Manager dedupes by `(R::key(), ScopeLevel)`. 10 concurrent acquires at the
same scope produce **one** `Resource::create` invocation; every lease is
`Arc::ptr_eq` to every other. See
[`examples/examples/resource_telegram_multi_workflow.rs`](../../../examples/examples/resource_telegram_multi_workflow.rs)
for the dedupe assertion.

### Friction points

- **`fingerprint() = 0` is correct.** Resident has only one instance, so a
  config change forces destroy + recreate; there's no "stale fingerprint"
  sweep to drive.
- **Token caching inside `Runtime`.** OAuth-style integrations cache an
  access token inside the Runtime. Two valid patterns:
  - **Reactive:** detect 401 → refresh inline + retry (used by
    [`resource_resident_http.rs`](../../../examples/examples/resource_resident_http.rs)).
  - **Proactive (preferred for high throughput):** use
    `Resident::stale_after(Some(token_ttl - margin))` and let the manager
    destroy + recreate before tokens expire.
- **`Clone` requirement on `Runtime`.** Inner state typically lives behind
  `Arc<Inner>` so `Clone` is a refcount bump.

### Runnable examples

- [`examples/examples/resource_resident_http.rs`](../../../examples/examples/resource_resident_http.rs) — OAuth refresh
- [`examples/examples/resource_telegram_multi_workflow.rs`](../../../examples/examples/resource_telegram_multi_workflow.rs) — cross-workflow dedupe assertion

---

## Cross-cutting checklist for a new Resource impl

Before sending the PR, verify:

- [ ] **`fingerprint`** hashes only fields that make existing instances stale.
- [ ] **`is_broken` / `is_alive_sync`** are sync, O(1), no I/O.
- [ ] **`destroy`** consumes `Runtime` (takes `self: Self::Runtime`, not a reference).
- [ ] **`Drop`** of the `Runtime` releases OS resources without `await`.
      If the runtime spawns tasks, wrap their `JoinHandle` in `AbortOnDrop`.
- [ ] **Cancel-safety:** if `create` does anything observable before
      returning a Runtime, ensure that path is idempotent or has a guard
      that cleans up on cancel.
- [ ] **Errors:** every variant of your `Error` enum classifies via
      `nebula_resource::ClassifyError` (transient / permanent / exhausted /
      backpressure). Action authors rely on this for retry decisions.
- [ ] **Credential slots** are declared as `#[credential(key = "...")]` fields,
      **not** through any deprecated singular associated type.

---

## See also

- [`README.md`](README.md) — library overview, public API surface
- [`pooling.md`](pooling.md) — Pool topology deep-dive (warmup, metrics, stats)
- [`recovery.md`](recovery.md) — recovery gating and resilience composition
- [`events.md`](events.md) — lifecycle event streaming
- [`adapters.md`](adapters.md) — accessor adapters (`HasResources`, `ScopedResourceAccessor`)
- [`api-reference.md`](api-reference.md) — full surface API listing
