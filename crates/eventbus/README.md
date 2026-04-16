# nebula-eventbus

Generic event distribution — a typed broadcast `EventBus<E>` with configurable backpressure policy. **Transport only** — no domain event types.

**Layer:** Cross-cutting
**Canon:** §3.10 (cross-cutting; transport only — domain `E` types live in owning crates)

## Status

**Overall:** `implemented` — the broadcast backbone used by engine, resource, telemetry, and other crates.

**Works today:**

- `EventBus<E>` — generic typed event bus backed by `tokio::sync::broadcast` (bounded, `Lagged` semantics, zero-copy clone on send, no per-send allocation)
- `BackPressurePolicy` — configurable policy for slow subscribers
- `Subscriber`, `FilteredSubscriber` — subscribe with optional filter predicate
- `Stream` integration — treat a subscriber as a `futures::Stream`
- `Filter` — composable filter predicates
- `Outcome` — `emit()` return type (`Sent`, `NoSubscribers`, `Lagged`, etc.)
- `Registry` — manage multiple buses by scope
- `Scope` — hierarchical scoping of buses
- `Stats` — bus-level statistics
- `prelude` — convenience re-exports
- 3 unit test markers, 2 integration tests

**Known gaps / deferred:**

- **Persistence** — this is deliberately an **in-process** broadcast bus. It is **not** the durable control plane (canon §12.2). Anything that needs durability must use `execution_control_queue`, not this crate. `lib.rs` is explicit about this.
- **Cross-process delivery** is not in scope.

## Architecture notes

- **Zero intra-workspace dependencies.** Transport-only crates should not depend on domain crates — this is the cleanest layer in the workspace.
- **Domain event types live in owning crates.** `ExecutionEvent` lives in `nebula-engine`, `ResourceEvent` lives in `nebula-resource`, etc. This crate never defines `struct MyEvent` — canon §3.10 is explicit.
- **Twelve modules for 1425 lines** — cleanly factored: `bus` (core), `policy` (backpressure), `subscriber` + `filtered_subscriber` + `filter`, `stream`, `stats`, `registry`, `scope`, `outcome`, `prelude`.
- **No dead code or compat shims.**
- **No SRP/DRY violations observed.**

## What this crate provides

| Type | Role |
| --- | --- |
| `EventBus<E>` | Broadcast bus parameterised by domain event type. |
| `BackPressurePolicy` | Policy for slow subscribers. |
| `Subscriber<E>`, `FilteredSubscriber<E>` | Subscription handles. |
| `Filter<E>` | Composable filter predicate. |
| `Outcome` | `emit()` result. |
| `Registry` | Multiple-bus management by scope. |
| `Scope` | Hierarchical bus scoping. |
| `Stats` | Bus statistics. |

## Where the contract lives

- Source: `src/lib.rs`, `src/bus.rs`, `src/policy.rs`
- Canon: `docs/PRODUCT_CANON.md` §3.10 ("transport only — domain `E` types live in owning crates")
- Glossary: `docs/GLOSSARY.md` §2 (where consumers like `ExecutionEvent` are listed)

## See also

- `nebula-engine` — owns `ExecutionEvent`
- Any domain crate that needs pub/sub — import this and define its own `E`
