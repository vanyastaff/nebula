# nebula-eventbus — Claude Code orientation
> Agent quick-map for `crates/eventbus/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** Transport-only generic `EventBus<E>` — in-process pub/sub broadcast with back-pressure over `tokio::sync::broadcast`; defines NO domain event types itself.
**Layer:** Cross-cutting — zero intra-workspace deps (cleanest boundary in the workspace); importable at any level.

## Commands
- `cargo check -p nebula-eventbus`
- `cargo nextest run -p nebula-eventbus`  ·  doctests: `cargo test -p nebula-eventbus --doc`
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
- `emit()` never blocks on slow subscribers; lag is recovered transparently (subscriber re-positions to latest). Use `lagged_count()` to observe drops; `EventBusStats` for sent/dropped/subscriber counts.
- Keep zero intra-workspace deps — adding a `nebula-*` dependency breaks the layer boundary (`deny.toml` wrappers).
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed errors; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · canon `docs/PRODUCT_CANON.md` §3.10 / §4.5 / §12.2 · sibling `nebula-metrics` (consumes this crate)
