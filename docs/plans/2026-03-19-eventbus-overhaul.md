# EventBus Overhaul — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Full API cleanup, Stream support, parking_lot migration, and fix Block policy polling in `nebula-eventbus`.

**Architecture:** Replace duplicated send/emit API with single `emit`/`emit_awaited` pair. Add `futures-core` Stream impl via `into_stream()`. Switch Registry to `parking_lot::RwLock`. Fix Block policy's 1ms polling with exponential backoff. Remove dead aliases and no-op methods.

**Tech Stack:** Rust 1.93+, tokio broadcast, futures-core, parking_lot

---

## Downstream Impact Map

| Crate | Uses `EventSubscriber` | Uses `emit_async` | Files to update |
|-------|----------------------|-------------------|-----------------|
| `nebula-resource` | Yes (`events.rs:13,50`, `metrics.rs:25,39`, `lib.rs:94,139`) | Yes (`events.rs:44-45`) | `events.rs`, `lib.rs`, `metrics.rs`, 5 test files |
| `nebula-telemetry` | Yes (`event.rs:218` — local alias, OK to keep) | No | verify only |
| `nebula-credential` | Yes (`manager.rs:476`) | No | `manager.rs` |
| `nebula-metrics` | No (uses `EventBusStats` only) | No | verify only |

---

## Task 1: Add new dependencies to `Cargo.toml`

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/eventbus/Cargo.toml`

**Step 1: Add `futures-core` to workspace deps**

In root `Cargo.toml` under `[workspace.dependencies]`, add:
```toml
futures-core = "0.3"
```

**Step 2: Update `crates/eventbus/Cargo.toml`**

Add to `[dependencies]`:
```toml
parking_lot = { workspace = true }
futures-core = { workspace = true }
```

**Step 3: Verify**

Run: `cargo check -p nebula-eventbus`

**Step 4: Commit**

```
chore(eventbus): add parking_lot, futures-core deps
```

---

## Task 2: API cleanup — remove aliases and dead methods

**Files:**
- Modify: `crates/eventbus/src/bus.rs`
- Modify: `crates/eventbus/src/subscriber.rs`
- Modify: `crates/eventbus/src/filtered_subscriber.rs`
- Modify: `crates/eventbus/src/lib.rs`
- Modify: `crates/eventbus/src/prelude.rs`

**Step 1: Rewrite `bus.rs` API**

Remove these methods entirely:
- `pub fn send(&self, event: E) -> PublishOutcome`
- `pub async fn send_async(&self, event: E) -> PublishOutcome`
- `pub async fn emit_async(&self, event: E) -> PublishOutcome`

The `emit()` method gets the body that was in `send()`:
```rust
/// Emits an event to all current subscribers (non-blocking).
///
/// When the buffer is full:
/// - **DropOldest**: event is sent (oldest overwritten).
/// - **DropNewest**: event is dropped and counted in stats.
/// - **Block**: behaves as DropOldest; use [`emit_awaited`](Self::emit_awaited) for blocking.
#[inline]
pub fn emit(&self, event: E) -> PublishOutcome {
    let outcome = match &self.policy {
        BackPressurePolicy::DropOldest | BackPressurePolicy::Block { .. } => {
            self.publish_drop_oldest(event)
        }
        BackPressurePolicy::DropNewest => self.publish_drop_newest(event),
    };
    self.record_outcome(outcome);
    outcome
}
```

Rename `send_async` logic → `emit_awaited`:
```rust
/// Emits an event, respecting [`BackPressurePolicy::Block`].
///
/// For `DropOldest` and `DropNewest`, behaves like [`emit`](Self::emit).
/// For `Block { timeout }`, waits up to `timeout` for buffer space before dropping.
pub async fn emit_awaited(&self, event: E) -> PublishOutcome
where
    E: Clone,
{
    match &self.policy {
        BackPressurePolicy::Block { timeout } => {
            let outcome = self.emit_blocking(event, *timeout).await;
            self.record_outcome(outcome);
            outcome
        }
        _ => self.emit(event),
    }
}
```

Rename internal `send_blocking` → `emit_blocking`.

Replace all `#[inline(always)]` with `#[inline]` on `emit()`, `publish_drop_oldest()`, `publish_drop_newest()`, `record_outcome()`.

**Step 2: Remove `close()` from `subscriber.rs`**

Delete:
```rust
/// Closes this subscription handle by consuming it.
pub fn close(self) {}
```

**Step 3: Remove `close()` from `filtered_subscriber.rs`**

Delete:
```rust
pub fn close(self) {
    self.inner.close();
}
```

**Step 4: Remove `EventSubscriber` alias from `lib.rs`**

Delete:
```rust
/// Alias for [`Subscriber`]; matches INTERACTIONS/ARCHITECTURE naming.
pub type EventSubscriber<E> = Subscriber<E>;
```

**Step 5: Remove `EventSubscriber` from `prelude.rs`**

Delete the line:
```rust
pub use crate::EventSubscriber;
```

**Step 6: Update unit tests in `bus.rs`**

- Replace all `bus.send(...)` → `bus.emit(...)` in tests
- Delete `emit_is_alias_for_send` test
- Update `block_policy_send_async_*` test names to `block_policy_emit_awaited_*` and use `emit_awaited`

**Step 7: Verify**

Run: `cargo check -p nebula-eventbus`

**Step 8: Commit**

```
refactor(eventbus): remove send/send_async/emit_async aliases, close() methods, EventSubscriber alias
```

---

## Task 3: Fix Block policy polling — exponential backoff

**Files:**
- Modify: `crates/eventbus/src/bus.rs`

**Step 1: Rewrite `emit_blocking` with exponential backoff**

Replace the 1ms fixed sleep:
```rust
async fn emit_blocking(&self, event: E, timeout: std::time::Duration) -> PublishOutcome
where
    E: Clone,
{
    let deadline = tokio::time::Instant::now() + timeout;
    let mut event = Some(event);
    let mut backoff = std::time::Duration::from_micros(50);
    const MAX_BACKOFF: std::time::Duration = std::time::Duration::from_millis(1);

    loop {
        if self.sender.receiver_count() == 0 {
            return PublishOutcome::DroppedNoSubscribers;
        }

        if self.sender.len() < self.buffer_size {
            let event = event
                .take()
                .expect("event should only be consumed once when capacity is available");
            return match self.sender.send(event) {
                Ok(_) => PublishOutcome::Sent,
                Err(_) => PublishOutcome::DroppedNoSubscribers,
            };
        }

        if tokio::time::Instant::now() >= deadline {
            return PublishOutcome::DroppedTimeout;
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}
```

**Step 2: Run tests**

Run: `cargo nextest run -p nebula-eventbus`

**Step 3: Commit**

```
fix(eventbus): replace 1ms polling with exponential backoff (50µs→1ms) in Block policy
```

---

## Task 4: Switch Registry to `parking_lot::RwLock`

**Files:**
- Modify: `crates/eventbus/src/registry.rs`

**Step 1: Replace import and remove poison recovery**

```rust
// Old:
use std::sync::{Arc, RwLock};
use tracing::warn;
// New:
use std::sync::Arc;
use parking_lot::RwLock;
```

Every `.read().unwrap_or_else(|poisoned| { warn!(...); poisoned.into_inner() })` → `.read()`.
Every `.write().unwrap_or_else(|poisoned| { warn!(...); poisoned.into_inner() })` → `.write()`.

Example — `get_or_create` becomes:
```rust
pub fn get_or_create(&self, key: K) -> Arc<EventBus<E>> {
    if let Some(existing) = self.buses.read().get(&key).cloned() {
        return existing;
    }
    let mut guard = self.buses.write();
    guard
        .entry(key)
        .or_insert_with(|| Arc::new(EventBus::with_policy(self.buffer_size, self.policy.clone())))
        .clone()
}
```

Apply same simplification to: `get`, `remove`, `len`, `clear`, `prune_without_subscribers`, `stats`.

**Step 2: Delete `poisoned_lock_recovery_does_not_panic` test**

Remove the entire test — parking_lot doesn't poison.

**Step 3: Remove `tracing` dep if unused**

Check if `tracing` is still used in eventbus. If `warn!` in registry was the only usage, remove `tracing` from `crates/eventbus/Cargo.toml` `[dependencies]`. Keep it in `[dev-dependencies]` if used by tests.

**Step 4: Run tests**

Run: `cargo nextest run -p nebula-eventbus`

**Step 5: Commit**

```
refactor(eventbus): switch Registry to parking_lot::RwLock, remove poison recovery
```

---

## Task 5: Stream impl

**Files:**
- Create: `crates/eventbus/src/stream.rs`
- Modify: `crates/eventbus/src/subscriber.rs`
- Modify: `crates/eventbus/src/filtered_subscriber.rs`
- Modify: `crates/eventbus/src/lib.rs`
- Modify: `crates/eventbus/src/prelude.rs`
- Modify: `crates/eventbus/Cargo.toml` (maybe add `tokio-stream`)

**Important design note:**

`tokio::sync::broadcast::Receiver` does NOT expose `poll_recv()`. Implementing `Stream` correctly without busy-polling requires one of:
- `tokio-stream` crate (provides `BroadcastStream` with correct waker registration)
- Manual pinned-future storage (complex, error-prone)

**Recommended:** Add `tokio-stream` to workspace and use `BroadcastStream` internally, wrapping it to handle `Lagged` errors transparently.

Check if `tokio-stream` is already in workspace. If not, add to root `Cargo.toml`:
```toml
tokio-stream = "0.1"
```

And to `crates/eventbus/Cargo.toml`:
```toml
tokio-stream = { workspace = true }
```

**Step 1: Create `stream.rs`**

```rust
//! `Stream` adapter for event bus subscribers.

use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use tokio_stream::wrappers::BroadcastStream;

/// Stream adapter that yields events from a [`Subscriber`](crate::Subscriber).
///
/// Created via [`Subscriber::into_stream()`](crate::Subscriber::into_stream).
/// Lagged events are skipped automatically (same semantics as
/// [`Subscriber::recv()`](crate::Subscriber::recv)).
///
/// # Example
///
/// ```no_run
/// use nebula_eventbus::EventBus;
/// use futures_core::Stream;
///
/// # #[derive(Clone)]
/// # struct Event(u64);
/// # async fn example() {
/// let bus = EventBus::<Event>::new(64);
/// let stream = bus.subscribe().into_stream();
/// // Use with StreamExt combinators
/// # }
/// ```
pub struct SubscriberStream<E: Clone + Send + 'static> {
    inner: BroadcastStream<E>,
    lagged_count: u64,
}

impl<E: Clone + Send + 'static> SubscriberStream<E> {
    pub(crate) fn new(
        receiver: tokio::sync::broadcast::Receiver<E>,
        lagged_count: u64,
    ) -> Self {
        Self {
            inner: BroadcastStream::new(receiver),
            lagged_count,
        }
    }

    /// Returns the total count of events skipped due to lag.
    #[must_use]
    pub fn lagged_count(&self) -> u64 {
        self.lagged_count
    }
}

impl<E: Clone + Send + 'static> Stream for SubscriberStream<E> {
    type Item = E;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(event))) => return Poll::Ready(Some(event)),
                Poll::Ready(Some(Err(BroadcastStreamRecvError::Lagged(skipped)))) => {
                    self.lagged_count = self.lagged_count.saturating_add(skipped);
                    continue;
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
```

**Step 2: Expose `receiver` from `Subscriber` for stream conversion**

Add to `subscriber.rs`:
```rust
/// Converts this subscriber into a [`Stream`](futures_core::Stream).
///
/// The stream yields events until the bus is closed. Lagged events are
/// skipped automatically (same semantics as [`recv()`](Self::recv)).
pub fn into_stream(self) -> crate::stream::SubscriberStream<E>
where
    E: 'static,
{
    crate::stream::SubscriberStream::new(self.receiver, self.lagged_count)
}
```

**Step 3: Add `into_stream()` to `FilteredSubscriber`**

In `filtered_subscriber.rs`:
```rust
/// Converts this filtered subscriber into a [`Stream`](futures_core::Stream)
/// that only yields events matching the filter.
///
/// Note: non-matching events are silently skipped (same as [`recv()`](Self::recv)).
pub fn into_stream(self) -> impl futures_core::Stream<Item = E>
where
    E: 'static,
{
    let filter = self.filter;
    crate::stream::FilteredStream::new(self.inner.into_stream(), filter)
}
```

Add `FilteredStream` to `stream.rs`:
```rust
/// Stream adapter that filters events by predicate.
pub struct FilteredStream<E: Clone + Send + 'static> {
    inner: SubscriberStream<E>,
    filter: crate::EventFilter<E>,
}

impl<E: Clone + Send + 'static> FilteredStream<E> {
    pub(crate) fn new(inner: SubscriberStream<E>, filter: crate::EventFilter<E>) -> Self {
        Self { inner, filter }
    }

    /// Returns the total count of events skipped due to lag.
    #[must_use]
    pub fn lagged_count(&self) -> u64 {
        self.inner.lagged_count()
    }
}

impl<E: Clone + Send + 'static> Stream for FilteredStream<E> {
    type Item = E;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            // SAFETY: we never move `inner` out of `self`
            let inner = unsafe { self.as_mut().map_unchecked_mut(|s| &mut s.inner) };
            match inner.poll_next(cx) {
                Poll::Ready(Some(event)) => {
                    if self.filter.matches(&event) {
                        return Poll::Ready(Some(event));
                    }
                    continue;
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
```

Note: the `unsafe` above is needed because `FilteredStream` is not `Unpin` by default. Alternative: add `#[pin_project]` from `pin-project-lite` (already in tokio's dep tree). Or just derive `Unpin` manually since neither `SubscriberStream` nor `EventFilter` are `!Unpin`. Actually, `BroadcastStream` is `Unpin`, so `SubscriberStream` is `Unpin`, so `FilteredStream` is `Unpin`. In that case, no `unsafe` needed — just use `Pin::new(&mut self.inner)`:

```rust
impl<E: Clone + Send + 'static> Stream for FilteredStream<E> {
    type Item = E;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                Poll::Ready(Some(event)) => {
                    if self.filter.matches(&event) {
                        return Poll::Ready(Some(event));
                    }
                    continue;
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
```

**Step 4: Register module in `lib.rs`**

```rust
mod stream;
pub use stream::FilteredStream;
pub use stream::SubscriberStream;
```

Add to `prelude.rs`:
```rust
pub use crate::FilteredStream;
pub use crate::SubscriberStream;
```

**Step 5: Write tests in `bus.rs`**

```rust
#[tokio::test]
async fn subscriber_into_stream_yields_events() {
    use futures_core::Stream;
    use std::pin::Pin;
    use std::task::Poll;

    let bus = EventBus::<TestEvent>::new(16);
    let sub = bus.subscribe();
    bus.emit(TestEvent(1));
    bus.emit(TestEvent(2));

    let mut stream = sub.into_stream();
    // Use poll_next via a helper or tokio_stream::StreamExt
    let e1 = tokio_stream::StreamExt::next(&mut stream).await;
    let e2 = tokio_stream::StreamExt::next(&mut stream).await;
    assert_eq!(e1, Some(TestEvent(1)));
    assert_eq!(e2, Some(TestEvent(2)));
}

#[tokio::test]
async fn subscriber_stream_ends_on_bus_drop() {
    let bus = EventBus::<TestEvent>::new(16);
    let sub = bus.subscribe();
    drop(bus);

    let mut stream = sub.into_stream();
    let result = tokio_stream::StreamExt::next(&mut stream).await;
    assert_eq!(result, None);
}
```

**Step 6: Run tests**

Run: `cargo nextest run -p nebula-eventbus`

**Step 7: Commit**

```
feat(eventbus): add Stream impl via into_stream() for Subscriber and FilteredSubscriber
```

---

## Task 6: Update integration tests, examples, benches

**Files:**
- Modify: `crates/eventbus/tests/integration.rs`
- Modify: `crates/eventbus/examples/subscriber_patterns.rs`
- Modify: `crates/eventbus/benches/emit.rs`
- Modify: `crates/eventbus/benches/throughput.rs`

**Step 1: Update integration tests**

- Replace all `bus.send(...)` → `bus.emit(...)` (grep for `\.send(` in test files)
- Replace `bus.send_async(...)` → `bus.emit_awaited(...)`
- Replace `BackPressurePolicy::Block { timeout: Duration::from_millis(N) }` → `BackPressurePolicy::Block { timeout: Duration::from_millis(N) }` (unchanged — we're not changing the enum structure without resilience)
- Remove the `#[ignore]` test `test_block_policy_not_yet_implemented` — Block is implemented
- Remove any `sub.close()` calls if present

**Step 2: Update example**

In `subscriber_patterns.rs`, no `send()` calls exist — it already uses `emit()`. Verify.

**Step 3: Update benches**

In `emit.rs` and `throughput.rs`, replace any `bus.send(...)` with `bus.emit(...)`. (Check — benches already use `emit()`. Verify.)

**Step 4: Run full eventbus validation**

Run: `cargo nextest run -p nebula-eventbus && cargo bench --no-run -p nebula-eventbus`

**Step 5: Commit**

```
test(eventbus): update integration tests, examples, benches for new API
```

---

## Task 7: Fix downstream — `nebula-resource`

**Files:**
- Modify: `crates/resource/src/events.rs`
- Modify: `crates/resource/src/lib.rs`
- Modify: `crates/resource/src/metrics.rs`
- Modify: `crates/resource/tests/events_integration.rs`
- Modify: `crates/resource/tests/hotreload_integration.rs`
- Modify: `crates/resource/tests/circuit_breaker.rs`
- Modify: `crates/resource/tests/manager_fixes.rs`
- Modify: `crates/resource/tests/metrics_integration.rs`

**Step 1: Update `events.rs`**

Replace imports:
```rust
// Old:
pub use nebula_eventbus::{
    BackPressurePolicy, EventBusStats, EventFilter, EventSubscriber, FilteredSubscriber,
    PublishOutcome, ScopedEvent, SubscriptionScope,
};
// New:
pub use nebula_eventbus::{
    BackPressurePolicy, EventBusStats, EventFilter, FilteredSubscriber,
    PublishOutcome, ScopedEvent, Subscriber, SubscriptionScope,
};
```

Change return type:
```rust
// Old:
pub fn subscribe(&self) -> EventSubscriber<ResourceEvent> {
// New:
pub fn subscribe(&self) -> Subscriber<ResourceEvent> {
```

Rename method:
```rust
// Old:
pub async fn emit_async(&self, event: ResourceEvent) -> PublishOutcome {
    self.0.emit_async(event).await
}
// New:
pub async fn emit_awaited(&self, event: ResourceEvent) -> PublishOutcome {
    self.0.emit_awaited(event).await
}
```

**Step 2: Update `metrics.rs`**

Replace `EventSubscriber` with `Subscriber` in imports and field types.

**Step 3: Update `lib.rs`**

Replace all `EventSubscriber` with `Subscriber` in re-export lines.

**Step 4: Update test files**

In all 5 test files:
- Replace `EventSubscriber` imports with `Subscriber`
- Replace `emit_async` calls with `emit_awaited`

**Step 5: Run resource tests**

Run: `cargo nextest run -p nebula-resource`

**Step 6: Commit**

```
refactor(resource): update to new eventbus API — Subscriber, emit_awaited
```

---

## Task 8: Fix downstream — `nebula-credential`

**Files:**
- Modify: `crates/credential/src/manager/manager.rs`

**Step 1: Replace `EventSubscriber` with `Subscriber`**

```rust
// Old:
) -> nebula_eventbus::EventSubscriber<crate::rotation::events::CredentialRotationEvent> {
// New:
) -> nebula_eventbus::Subscriber<crate::rotation::events::CredentialRotationEvent> {
```

**Step 2: Run credential tests**

Run: `cargo nextest run -p nebula-credential`

**Step 3: Commit**

```
refactor(credential): replace EventSubscriber with Subscriber
```

---

## Task 9: Fix downstream — `nebula-telemetry` and `nebula-metrics`

**Files:**
- Verify: `crates/telemetry/src/event.rs` — defines local `EventSubscriber` alias pointing to `nebula_eventbus::Subscriber<ExecutionEvent>`, this is fine
- Verify: `crates/metrics/src/adapter.rs` — uses `EventBusStats` only, no changes

**Step 1: Verify telemetry compiles**

Run: `cargo check -p nebula-telemetry`

**Step 2: Verify metrics compiles**

Run: `cargo check -p nebula-metrics`

**Step 3: Run tests**

Run: `cargo nextest run -p nebula-telemetry -p nebula-metrics`

**Step 4: Commit (only if changes needed)**

```
refactor(telemetry): update to new eventbus API
```

---

## Task 10: Update docs + context file + lib.rs docs

**Files:**
- Modify: `crates/eventbus/src/lib.rs`
- Modify: `.claude/crates/eventbus.md`

**Step 1: Update lib.rs Quick Start**

Update the doc example to use `emit()` (it already does). Remove `EventSubscriber` from Core Types list. Add `SubscriberStream`, `FilteredStream` to Core Types. Remove mention of `send()`.

**Step 2: Update `.claude/crates/eventbus.md`**

```markdown
# nebula-eventbus
Generic typed pub/sub event bus — transport infrastructure only, no domain event types.

## Invariants
- Transport-only: no domain event types defined here. Domain crates own their event types.
- Best-effort delivery: producers never block; no delivery guarantee; no global ordering.
- In-memory only (Phase 2). No persistence. Events are lost on restart or buffer overflow.
- No nebula deps. Uses parking_lot, futures-core, tokio-stream externally.

## Key Decisions
- Backed by `tokio::sync::broadcast` — bounded, Lagged semantics, zero-copy clone.
- `BackPressurePolicy` controls buffer-full behavior (DropOldest / DropNewest / Block).
- `Block` policy uses exponential backoff (50µs base, 1ms cap) instead of fixed 1ms polling.
- `EventBusRegistry` uses `parking_lot::RwLock` (no poisoning, no recovery code).
- `FilteredSubscriber` + `EventFilter` for predicate-based selective subscription.
- `SubscriptionScope` + `ScopedEvent` for targeted subscriptions.
- `Subscriber::into_stream()` returns a `futures_core::Stream` via `tokio-stream::BroadcastStream`.
- Single emit API: `emit()` (non-blocking) and `emit_awaited()` (Block policy).

## Traps
- Slow subscribers don't block producers — they lag and auto-skip. Check `lagged_count()`.
- Dropping `Subscriber` auto-decrements count — no explicit close needed.
- `emit()` with `Block` policy behaves as `DropOldest` — use `emit_awaited()` for blocking.
- `EventSubscriber<E>` type alias was removed — use `Subscriber<E>` directly.
- `send()` / `send_async()` / `emit_async()` were removed — use `emit()` / `emit_awaited()`.

## Relations
- No nebula deps. Used by nebula-telemetry, nebula-resource, nebula-credential.
```

**Step 3: Commit**

```
docs(eventbus): update module docs and context file for new API
```

---

## Task 11: Full workspace validation

**Step 1: Format + lint + test**

Run: `cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace`

**Step 2: Doc tests**

Run: `cargo test --workspace --doc`

**Step 3: Bench compile**

Run: `cargo bench --no-run -p nebula-eventbus`

**Step 4: Example compile**

Run: `cargo build --example subscriber_patterns -p nebula-eventbus`

All must pass with zero warnings.
