# nebula-resource

Type-safe, topology-aware resource management for the Nebula workflow engine.
`nebula-resource` gives workflow nodes stable, managed access to external
clients — database connections, HTTP clients, message-queue producers, and
anything else that is costly to create and should be reused across executions.
It handles the full operational lifecycle: create → health-check → recycle →
shutdown → destroy, with credential rotation, recovery gating, and lifecycle
event streaming.

> **Maturity: `frontier`.** The public API still evolves between minor releases.
> This crate is `publish = false` — an internal workspace crate, not published
> to crates.io. `nebula-sdk` is the publishable surface.

This page is a **prose map**, not a signature mirror — concrete
signatures drift from hand-maintained copies (see
[`api-reference.md`](api-reference.md)). For runnable code, start at the
crate-root rustdoc Quick Start (`cargo doc -p nebula-resource --open`, or
read `src/lib.rs`'s module doc directly) and the doctest on
[`Manager::register`](../src/manager/registration.rs).

---

## Core Concepts

| Type | Role |
|------|------|
| `Provider` | Central trait — `Config`/`Instance`/`Topology` associated types + lifecycle methods (`create`, `check`, `shutdown`, `destroy`) + per-slot credential-rotation hooks (`on_credential_refresh`, `on_credential_revoke`) |
| `Resource` (derive) | Emits credential-slot plumbing (`HasCredentialSlots`, `<field>_slot()` accessors) for a hand-written `impl Provider` |
| `Pooled` / `Resident` / `Bounded` | The three built-in topologies — see below |
| `Manager` | Central registry — single `register(RegistrationSpec { … })` funnel, typed acquire dispatch (`acquire_any`, `acquire_pooled[_for_identity]`, `acquire_resident[_for_identity]`, `acquire_bounded[_for_identity]`), slot rotation, graceful shutdown |
| `ResourceGuard` | RAII guard; derefs to `R::Instance`, releases on drop, tainting supported |
| `ResourceContext` | Execution context — scope, cancellation, capability traits |
| `Error` / `ErrorKind` | Unified error with retryability, scope, and optional retry-after hint — see the `error` module rustdoc for the full kind → caller-action table |

---

## Topology Decision Guide

| Topology | Instance model | Use when | Example |
|----------|-----------------|----------|---------|
| `Pooled` | N interchangeable instances, checkout/recycle | Stateful, interchangeable connections | PostgreSQL, Redis |
| `Resident` | One shared instance, `Arc::clone` on acquire | Cheap-to-clone client shared widely | `reqwest::Client`, in-memory cache, OAuth/token-gated SDK clients |
| `Bounded` | Concurrency-capped, no warm idle pool | Scarce non-warmable capacity | License seats, a serial-exclusive device |

`type Topology` is static per resource type; only its *config* (sizes, cap) is
a runtime value. See [`topology-reference.md`](topology-reference.md) for a
per-topology trait skeleton, decision matrix, and friction-point checklist.

> **Background workers and event sources** live in
> [`nebula-engine`](https://docs.rs/nebula-engine) (`nebula_engine::daemon::*`).
> They are not part of the `nebula-resource` topology surface.

---

## Quick start

The 90% path — `#[derive(Resource)]` for slot plumbing, a hand-written
`impl Provider` for the lifecycle, `Manager::register`, then
`acquire_<topology>` — is a runnable doctest on
[`Manager::register`](../src/manager/registration.rs), not duplicated here.
Run `cargo doc -p nebula-resource --open` and open the crate root for the
same walkthrough plus the topology decision table and error taxonomy.

---

## Error handling

Every `register` / `acquire_*` call returns `Result<_, Error>`. `ErrorKind`
drives retry decisions (`Transient`, `Permanent`, `Exhausted`,
`Backpressure`, `NotFound`, `Cancelled`, `Revoked`, `Ambiguous`) — see the
`error` module rustdoc for the authoritative kind → caller-action table.
Use `err.is_retryable()` to branch without matching on variants, and
`err.retry_after()` to respect rate-limit hints.

Resource authors bridge a domain error enum in with
`#[derive(ClassifyError)]` rather than a hand-written `From` impl — see the
runnable doctest on the `ClassifyError` re-export in `src/lib.rs`.

---

## Feature matrix

| Capability | How to enable |
|------------|---------------|
| Bounded connection pooling | `RegistrationSpec { topology: Pooled::new(..), .. }` |
| Shared singleton with clone-on-acquire | `RegistrationSpec { topology: Resident::new(..), .. }` |
| Concurrency cap without a warm pool | `RegistrationSpec { topology: Bounded::capped(n) / exclusive() / unbounded(), .. }` |
| Fast-fail during backend recovery | `RegistrationSpec::recovery_gate: Some(Arc<RecoveryGate>)` |
| Config hot-reload (fingerprint-based) | Implement `ResourceConfig::fingerprint`; call `Manager::reload_config` |
| Per-tenant credential isolation | Build `SlotIdentity::from_bindings(…)` and acquire via `acquire_<topology>_for_identity` |
| Lifecycle event stream | `manager.subscribe_events()` → `Subscriber<ResourceEvent>` |
| Async background cleanup | `ReleaseQueue` (owned by `Manager`, transparent to callers) |
| Atomic operation counters | `manager.metrics()` → `Option<&ResourceOpsMetrics>` |
| Credential-rotation fan-out | `rotation` feature — see [`credential-rotation.md`](credential-rotation.md) |

Retry/timeout on the acquire path composes one layer up (action handler /
engine activity). Per-topology configs carry their own `create_timeout` for
the create step.

---

## Crate layout

```
crates/resource/
├── src/
│   ├── lib.rs              re-exports, crate-level docs (Quick Start, topology table, error taxonomy)
│   ├── resource.rs         Provider trait, ResourceConfig, HasCredentialSlots, ResourceMetadata
│   ├── slot.rs / cell.rs   SlotCell (public, generation-stamped) vs internal epoch-blind Cell
│   ├── manager/            Manager: register/registration, acquire, gate, rotation, shutdown, options
│   ├── registry.rs         Registry, type-erased managed-handle storage, scope-aware lookup
│   ├── guard.rs            ResourceGuard — RAII acquire lease
│   ├── context.rs          ResourceContext — execution context
│   ├── dedup.rs            SlotIdentity (Unbound / Structural), DedupKey
│   ├── error.rs            Error, ErrorKind — see module docs for the caller-action table
│   ├── events.rs           ResourceEvent — lifecycle event catalog
│   ├── options.rs          AcquireOptions
│   ├── metrics.rs          ResourceOpsMetrics, ResourceOpsSnapshot
│   ├── release_queue.rs    ReleaseQueue — background async cleanup workers
│   ├── reload.rs           ReloadOutcome (NoChange / SwappedImmediately)
│   ├── recovery/           RecoveryGate, RecoveryTicket, RecoveryWaiter, GateState
│   ├── runtime/            per-topology runtime structs (Pooled, Resident, Bounded) + ManagedResource
│   ├── topology/           the open Topology<R> contract + per-topology hook traits + InstanceStore
│   ├── factory.rs          ResourceFactory / KindActivator — erased plugin-registration bridge
│   └── credential_fanout/  [feature `rotation`] per-slot rotation fan-out driver + reverse index
└── docs/
    ├── README.md              ← this file
    ├── api-reference.md       pointer to rustdoc + prose surface anchor
    ├── topology-reference.md  topology selection guide + minimal skeletons (Pool / Resident / Bounded)
    ├── credential-rotation.md rotate → slot swap → refresh/taint/revoke sequence
    ├── pooling.md             PoolConfig field reference, idle/warmup strategy guidance
    ├── events.md              ResourceEvent catalog
    ├── recovery.md            RecoveryGate state machine
    └── DESIGN.md              architecture / invariants / open questions (agent-oriented)
```

---

## Documentation

| Document | Contents |
|----------|----------|
| [`api-reference.md`](api-reference.md) | Generated-rustdoc pointer + prose surface anchor |
| [`topology-reference.md`](topology-reference.md) | Topology selection guide + minimal skeletons (Pool / Resident / Bounded) |
| [`credential-rotation.md`](credential-rotation.md) | The rotate → slot swap → refresh/taint/revoke sequence |
| [`pooling.md`](pooling.md) | `PoolConfig` field reference, idle-selection and warmup strategy guidance |
| [`events.md`](events.md) | `ResourceEvent` catalog + `subscribe_events` patterns |
| [`recovery.md`](recovery.md) | `RecoveryGate` state machine |
| [`DESIGN.md`](DESIGN.md) | Architecture, invariants, and open questions (agent-oriented) |
