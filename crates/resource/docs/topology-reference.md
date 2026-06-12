# Topology Reference for Resource Authors

> **Audience:** authors writing a new `Provider` impl. Maps each built-in
> topology (`Pooled`, `Resident`, `Bounded`) to a minimal Rust skeleton, the
> hook trait you implement, when to pick it, and the friction points.
>
> See [`README.md`](README.md) for the full library overview, and the
> root `examples/` workspace member for runnable end-to-end demonstrations of
> each pattern.

A resource declares its lease behaviour with a `type Topology` associated type
on its `Provider` impl. The **framework** owns the acquire loop and the
credential-revoke fence; a topology supplies only thin, R-aware hooks (it cannot
touch the idle store or the fence — see
[`topology/contract.rs`](../src/topology/contract.rs)). The three built-ins:

---

## Decision matrix

| Topology                    | Instance model                                              | Use when…                                                                      | Don't use for…                                          |
|-----------------------------|------------------------------------------------------------|--------------------------------------------------------------------------------|---------------------------------------------------------|
| **[Pool](#pool)**           | N interchangeable instances, checkout / recycle / destroy  | N stateful instances, expensive to create, reused (DB connections).            | A single shared client (use Resident).                  |
| **[Resident](#resident)**   | One shared instance, `Arc::clone` on acquire               | One instance shared widely; clone is cheap (`reqwest::Client`, in-mem cache).  | Per-caller mutable state (use Pool).                    |
| **[Bounded](#bounded)**     | Concurrency-capped, **no** warm idle pool                  | Cap concurrent leases without pooling: license seats, serial-exclusive device. | N reusable warm instances (use Pool).                   |

`type Topology` is static per resource type; only its *config* (sizes, cap) is a
runtime value. The author constructs the concrete topology and hands it to
`Manager::register` in a `RegistrationSpec`.

---

## Pool

**N interchangeable instances managed by a checkout / recycle / destroy lifecycle.**

### Trait set

```rust,ignore
use nebula_resource::{
    Provider, ResourceContext, ResourceMetadata, TeardownCx,
    error::Error, resource::ResourceConfig,
    topology::{Pooled, PoolProvider, BrokenCheck, InstanceMetrics, RecycleDecision},
};

#[async_trait::async_trait]
impl Provider for Postgres {
    type Config = PostgresConfig;
    type Instance = PgConnection;
    type Topology = Pooled<Self>;          // ← static topology choice

    fn key() -> ResourceKey { resource_key!("demo.postgres") }

    async fn create(&self, config: &Self::Config, ctx: &ResourceContext)
        -> Result<Self::Instance, Error> { /* … */ }

    async fn destroy(&self, instance: Self::Instance, cx: TeardownCx)
        -> Result<(), Error> { /* flush/close before drop; cx.deadline bounds it */ }

    fn metadata() -> ResourceMetadata { ResourceMetadata::from_key(&Self::key()) }
}

impl PoolProvider for Postgres {
    fn is_broken(&self, instance: &Self::Instance) -> BrokenCheck { /* sync, O(1) */ }

    async fn recycle(&self, instance: &Self::Instance, metrics: &InstanceMetrics)
        -> Result<RecycleDecision, Error> { /* wipe per-lease state → Keep / Drop */ }

    // Optional: `prepare(&self, instance, ctx)` for per-checkout session setup.
}
```

### Registration

```rust,ignore
manager.register(RegistrationSpec {
    resource: Postgres,
    config: pg_config.clone(),
    scope: ScopeLevel::Global,
    slot_identity: SlotIdentity::Unbound,
    topology: Pooled::new(PoolConfig { max_size: 8, ..Default::default() }, pg_config.fingerprint()),
    recovery_gate: None,
})?;
```

### Lifecycle

`create` (lazy on first acquire, eager via warmup) → optional `prepare` (per
checkout) → caller holds `ResourceGuard` → on drop: tainted? destroy. healthy? →
`recycle` returns `Keep` (return to the framework idle store, under the revoke
fence) / `Drop` (destroy).

### Friction points

- **Sync `is_broken`.** Read atomic flags only — no I/O, no async. For a network
  check, do it in `recycle` (async).
- **`fingerprint()` semantics.** Hash only fields that make existing instances
  stale (`application_name`, `statement_timeout`) — not `max_size`.
- **Credentialed pools discard by default.** If the resource declares credential
  slots, the default `recycle` **discards** rather than re-pools a dirty
  connection (cross-lease state bleed prevention, ADR-0093). Override `recycle`
  to wipe per-lease state and return `Keep` to actually pool.

### Runnable example

[`examples/examples/resource_postgres_pool.rs`](../../../examples/examples/resource_postgres_pool.rs)

---

## Resident

**A single shared instance, cloned on every acquire.**

### Trait set

```rust,ignore
use nebula_resource::topology::{Resident, ResidentProvider};

#[async_trait::async_trait]
impl Provider for GoogleSheets {
    type Config = GoogleSheetsConfig;
    type Instance = GoogleSheetsClient;    // Clone — cloned on each acquire
    type Topology = Resident<Self>;

    fn key() -> ResourceKey { resource_key!("demo.google.sheets") }

    async fn create(&self, config: &Self::Config, ctx: &ResourceContext)
        -> Result<Self::Instance, Error> { /* … */ }

    async fn destroy(&self, instance: Self::Instance, cx: TeardownCx)
        -> Result<(), Error> { Ok(()) }

    fn metadata() -> ResourceMetadata { ResourceMetadata::from_key(&Self::key()) }
}

impl ResidentProvider for GoogleSheets {
    fn is_alive_sync(&self, _instance: &Self::Instance) -> bool { true }
    fn stale_after(&self) -> Option<Duration> { None }
}
```

### Registration

```rust,ignore
manager.register(RegistrationSpec {
    resource: GoogleSheets::new(cred),
    config: GoogleSheetsConfig { application: "nebula-demo".into() },
    scope: ScopeLevel::Global,
    slot_identity: SlotIdentity::Unbound,
    topology: Resident::new(ResidentConfig::default()),
    recovery_gate: None,
})?;
```

### Cross-workflow dedupe

The Manager dedupes by `(R::key(), ScopeLevel, SlotIdentity)`. 10 concurrent
acquires at the same scope and credential identity produce **one**
`Provider::create`; every lease is a clone of the one master handle. See
[`examples/examples/resource_telegram_multi_workflow.rs`](../../../examples/examples/resource_telegram_multi_workflow.rs).

### Friction points

- **`Clone` on `Instance`.** Inner state typically lives behind `Arc<Inner>`, so
  `Clone` is a refcount bump.
- **Revoke teardown runs through the credential hook.** The master handle is
  never in the framework idle store, so the store revoke-fence cannot reach it;
  Resident handles its own revoke via `dispatch_credential_hook`
  (`handles_own_revoke() == true`).
- **Token caching inside `Instance`.** OAuth-style integrations cache a token in
  the handle. Reactive (detect 401 → refresh inline) or proactive
  (`stale_after(Some(ttl - margin))` → the framework recreates before expiry).

### Runnable examples

- [`examples/examples/resource_resident_http.rs`](../../../examples/examples/resource_resident_http.rs) — OAuth refresh
- [`examples/examples/resource_telegram_multi_workflow.rs`](../../../examples/examples/resource_telegram_multi_workflow.rs) — cross-workflow dedupe

---

## Bounded

**A runtime concurrency cap over a resource that does *not* keep a warm idle pool.**

Fills the gap Pool and Resident leave open: limit how many leases are live at
once, without an idle pool of interchangeable instances. Three modes, chosen by
a runtime value (not a const generic):

| Mode | Cap | Instance lifecycle | Use case |
|------|-----|--------------------|----------|
| `Bounded::capped(n)` | `Semaphore(n)` | fresh per lease, destroyed on release | license seats, connection cap |
| `Bounded::exclusive()` | `Semaphore(1)` | **one** instance reused, reset on release | serial device / single session |
| `Bounded::unbounded()` | none | fresh per lease, destroyed on release | no cap, no reuse |

### Trait set

```rust,ignore
use nebula_resource::topology::{Bounded, BoundedProvider};

#[async_trait::async_trait]
impl Provider for SerialPort {
    type Config = SerialCfg;
    type Instance = PortHandle;
    type Topology = Bounded<Self>;

    fn key() -> ResourceKey { resource_key!("demo.serial") }

    async fn create(&self, config: &Self::Config, ctx: &ResourceContext)
        -> Result<Self::Instance, Error> { /* open the port */ }

    async fn destroy(&self, instance: Self::Instance, cx: TeardownCx)
        -> Result<(), Error> { /* close */ }

    fn metadata() -> ResourceMetadata { ResourceMetadata::from_key(&Self::key()) }
}

// Only Exclusive reuses its instance and therefore resets; Capped / Unbounded
// destroy on release and use the default no-op reset.
impl BoundedProvider for SerialPort {
    async fn reset(&self, instance: &mut Self::Instance) -> Result<(), Error> {
        /* clear per-lease state before the next acquirer */ Ok(())
    }
}
```

### Registration

```rust,ignore
manager.register(RegistrationSpec {
    resource: SerialPort,
    config: SerialCfg::default(),
    scope: ScopeLevel::Global,
    slot_identity: SlotIdentity::Unbound,
    topology: Bounded::exclusive(),      // or Bounded::capped(seats)? / Bounded::unbounded()
    recovery_gate: None,
})?;
```

### Friction points

- **`capped(0)` is rejected** at construction with a typed `Error` — a zero cap
  can never admit. `Bounded::capped(n)` returns `Result`; use `?`.
- **A failed `Exclusive` reset destroys the instance** instead of reissuing a
  half-reset one (the S4 invariant) and surfaces the error to an awaited
  `release()`; a fresh instance is built on the next acquire. The single permit
  is held until the reset resolves, so no acquirer observes the in-between state.
- **Bounded is not a pool.** `Capped` / `Unbounded` pay a full `create` per
  lease (no idle reuse). If you want a cap *and* warm reuse, use Pool with
  `max_size = n`.
- **`set_cap(n)`** resizes a `Capped` topology at runtime (grow immediately;
  shrink as in-flight leases return).

---

## Background health probes (`CheckCost`)

The framework maintenance reaper health-probes idle Pool instances via
`Provider::check`, spaced by `Provider::check_cost()`:

- `CheckCost::Cheap` (default) — probed every maintenance sweep (an in-process
  liveness flag).
- `CheckCost::Moderate` / `CheckCost::Expensive` — probed less often (every 4th /
  16th sweep) so a network-round-trip check does not hammer an idle pool.

A slot whose `check` fails is evicted and destroyed; the next acquire rebuilds a
fresh one. Override `check_cost()` to match the real cost of your `check`.

---

## Cross-cutting checklist for a new Provider impl

Before sending the PR, verify:

- [ ] **`fingerprint`** hashes only fields that make existing instances stale.
- [ ] **`is_broken` / `is_alive_sync`** are sync, O(1), no I/O.
- [ ] **`check_cost`** reflects the real cost of `check` (default `Cheap`).
- [ ] **`destroy`** consumes the `Instance` and honours `cx.deadline`; the
      sync `Drop` of the `Instance` must release OS resources without `await`.
      If `create` spawns tasks, wrap their `JoinHandle` in an abort-on-drop.
- [ ] **Cancel-safety:** if `create` does anything observable before returning,
      ensure that path is idempotent or cleaned up on cancel.
- [ ] **Errors** classify via `nebula_resource::ClassifyError` (transient /
      permanent / exhausted / backpressure) — action authors retry on this.
- [ ] **Credential slots** are declared as `#[credential(key = "...")]` fields.

---

## See also

- [`README.md`](README.md) — library overview, public API surface
- [`pooling.md`](pooling.md) — Pool topology deep-dive (warmup, metrics, stats)
- [`recovery.md`](recovery.md) — recovery gating and resilience composition
- [`events.md`](events.md) — lifecycle event streaming
- [`api-reference.md`](api-reference.md) — full surface API listing
