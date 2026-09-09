# nebula-eventbus — Agent orientation
> Local guide for `crates/eventbus/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** Transport-only generic `EventBus<E>` — in-process pub/sub broadcast with back-pressure over `tokio::sync::broadcast`; defines NO domain event types itself.
**Layer:** Cross-cutting — zero intra-workspace deps (cleanest boundary in the workspace); importable at any level.

## Commands

- `task bench:crate CRATE=nebula-eventbus` — `emit` + `throughput` Criterion benches (`benches/`)

## Key files

- `src/lib.rs` — crate docs + module wiring; canonical `pub use` names (authoritative over README's shorter aliases)
- `src/bus.rs` — `EventBus<E>` broadcaster; non-blocking `emit()`, async `emit_awaited()`
- `src/subscriber.rs` — `Subscriber<E>`: `recv()`/`try_recv()`/`lagged_count()`, auto-decrement on drop
- `src/policy.rs` / `src/outcome.rs` — `BackPressurePolicy`, `PublishOutcome` emit result
- `src/registry.rs` / `src/scope.rs` — `EventBusRegistry` (multi-bus by key), `SubscriptionScope`/`ScopedEvent`
- `src/filter.rs` / `src/filtered_subscriber.rs` / `src/stream.rs` — `EventFilter`, `FilteredSubscriber`, `Stream` adapters

## Conventions & never-do

- **NEVER define concrete domain event structs here** (`ExecutionEvent`, `ResourceEvent`, …) — they live in their owning crates; this crate stays generic over `E: Clone` (Contract L3-§3.10).
- **In-process and ephemeral, not authoritative** — best-effort, no durability/ordering guarantee; a receive-and-log subscriber does NOT satisfy canon §12.2. Durable delivery (cancel/dispatch) uses `execution_control_queue`, not this bus.
- `emit()` never blocks on slow subscribers; `emit_awaited()` is the separate async path. Lag recovery continues at the oldest retained event, not necessarily the latest (`subscriber_tracks_lagged_count` in `src/bus.rs` pins this). Use `lagged_count()` and bus stats to observe loss.
- Keep zero intra-workspace deps — adding a `nebula-*` dependency breaks the layer boundary (`deny.toml` wrappers).

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Backpressure, lag, or subscriber drop | Unit tests in [src/bus.rs](src/bus.rs), including overflow accounting and stream closure; [integration](tests/integration.rs) exercises consumers. |

## See also

- `README.md` — full design · canon [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §3.10 / §4.5 / §12.2 · sibling `nebula-metrics` (consumes this crate)
