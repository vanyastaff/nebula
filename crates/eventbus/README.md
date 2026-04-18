---
name: nebula-eventbus
role: Publish-Subscribe Channel with Back-Pressure (transport only, no domain event types)
status: stable
last-reviewed: 2026-04-17
canon-invariants: []
related: [nebula-metrics]
---

# nebula-eventbus

## Purpose

Domain crates (engine, resource, telemetry) need to broadcast events to multiple in-process
subscribers without coupling producer to consumer. Without a shared transport, each crate builds
its own `tokio::sync::broadcast` wrapper with different back-pressure semantics, lag handling, and
filter APIs. `nebula-eventbus` provides a single generic `EventBus<E>` that any domain crate
parameterizes with its own event type. The crate is deliberately transport-only: it defines no
domain event types itself, ensuring zero upward coupling.

## Role

**Publish-Subscribe Channel with Back-Pressure** — the in-process broadcast backbone for
event-driven communication between cross-cutting and domain layers. Zero intra-workspace
dependencies (the cleanest layer boundary in the workspace). Pattern: async bounded broadcast with
`Lagged` recovery semantics (backed by `tokio::sync::broadcast`). This is an **in-process,
ephemeral** channel — not a durable control plane. Canon §4.5 / §12.2: anything requiring
reliable delivery (cancel, dispatch signals) must use `execution_control_queue`, not this crate.

## Public API

- `EventBus<E>` — generic typed broadcast bus; parameterized by domain event type `E: Clone`.
- `BackPressurePolicy` — configurable behavior for slow subscribers (block, drop, lag).
- `Subscriber<E>` — subscription handle with `recv()`, `try_recv()`, `lagged_count()`.
- `FilteredSubscriber<E>` — subscription handle with an attached `Filter<E>`.
- `Filter<E>` — composable filter predicate.
- `Outcome` — `emit()` return type (`Sent`, `NoSubscribers`, `Lagged`, …).
- `Registry` — manage multiple buses by scope.
- `Scope` — hierarchical bus scoping.
- `Stats` — bus-level statistics (sent, dropped, subscriber count).
- `prelude` — convenience re-exports.

## Contract

- **[L1-§4.5 / §12.2]** This bus is **in-process and ephemeral** — not authoritative. Anything needing durability must use `execution_control_queue` (the durable outbox). A `receive-and-log` subscriber over this bus does **not** satisfy canon §12.2's durable delivery requirement.
- **[L3-§3.10]** Domain event types (`ExecutionEvent`, `ResourceEvent`, etc.) must be defined in their owning crates, not here. This crate never defines concrete event structs.

## Non-goals

- Not a durable message broker — cross-process delivery and persistence are out of scope.
- Not a metrics export layer — `nebula-metrics` uses this crate, not the reverse.
- Not a log system — see `nebula-log`.

## Maturity

See `docs/MATURITY.md` row for `nebula-eventbus`.

- API stability: `stable` — `EventBus<E>`, `Subscriber`, `BackPressurePolicy`, `Outcome` are in active use with 3 unit tests and 2 integration tests.
- `Registry` and `Scope` are functional; multi-bus management patterns may be refined as engine usage grows.

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.10 (cross-cutting transport), §4.5 (operational honesty), §12.2 (durable control plane vs. in-process channels).
- Siblings: `nebula-metrics` (depends on this crate for event dispatch), domain crates (`nebula-engine`, `nebula-resource`) that define their own event types and construct `EventBus<TheirEvent>`.
