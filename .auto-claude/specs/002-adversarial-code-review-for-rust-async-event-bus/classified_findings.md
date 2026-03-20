# Classified Findings: nebula-eventbus Adversarial Code Review

**Date:** 2026-03-19
**Reviewer:** Claude (Auto-Claude)
**Task:** subtask-6-1
**Classification:** Bug > Footgun > Improvement

---

## Executive Summary

Compiled all findings from phases 1-5 (Static Analysis, Concurrency Analysis, Edge Case Analysis, Performance Analysis, API Audit) and classified by severity using the priority framework from spec.md.

**Total Findings:** 32 issues identified
- **Bugs:** 2 (correctness issues with demonstrable production impact)
- **Footguns:** 10 (API design issues that invite misuse or confusion)
- **Improvements:** 20 (documentation gaps and optimization opportunities)

**Production Readiness:** The crate is production-ready with 2 bugs that should be addressed (both related to TOCTOU races in backpressure policy enforcement).

---

## SEVERITY: Bug (2 findings)

**Definition:** Code that produces wrong results or exhibits undefined behavior under valid inputs.

---

### BUG-1: TOCTOU Race in publish_drop_newest() Violates Policy Semantics

**Location:** `crates/eventbus/src/bus.rs:105-118`
**Source:** subtask-1-2, subtask-2-4 (TOCTOU analysis)

**Scenario:**
```rust
// Thread A emits with DropNewest policy
let bus = EventBus::with_policy(16, BackPressurePolicy::DropNewest);

// Time 0: Thread A checks len() < buffer_size → false (buffer full)
// Time 1: Thread B consumes event → buffer has space
// Time 2: Thread A returns DroppedByPolicy → EVENT LOST despite available space

// Alternative scenario:
// Time 0: Thread A checks len() < buffer_size → true (space available)
// Time 1: Thread C fills buffer completely
// Time 2: Thread A calls send() → OVERWRITES OLDEST (acts like DropOldest!)
```

**Impact:**
- Expected: Drop newest event when buffer is full (DropNewest policy)
- Actual: Can drop events when space is available, OR overwrite oldest when buffer fills mid-check
- Violates documented DropNewest policy semantics under concurrent load

**Root Cause:**
`tokio::sync::broadcast` doesn't support DropNewest natively. Current implementation emulates it with racy checks on `sender.len()` before calling `send()`.

**Suggested Fix (Option 1 - Document race):**
```rust
/// # Racy Behavior with DropNewest
///
/// Due to TOCTOU between buffer length check and send, DropNewest policy
/// may occasionally send events when buffer is full (acting like DropOldest)
/// or drop events when buffer has space. Use DropOldest for strict semantics.
```

**Suggested Fix (Option 2 - Mutex serialization):**
```rust
struct EventBus<E> {
    sender: broadcast::Sender<E>,
    emit_lock: Mutex<()>,  // Serialize emits for DropNewest
    // ...
}

fn publish_drop_newest(&self, event: E) -> PublishOutcome {
    let _guard = self.emit_lock.lock();
    // Now checks and send are atomic
    if self.sender.len() >= self.buffer_size {
        return PublishOutcome::DroppedByPolicy;
    }
    match self.sender.send(event) { /* ... */ }
}
```

**Trade-offs:** Option 1 is pragmatic for "best-effort" event bus. Option 2 adds lock contention but provides strict semantics.

---

### BUG-2: TOCTOU Race in emit_blocking() Violates Block Policy Semantics

**Location:** `crates/eventbus/src/bus.rs:162-173`
**Source:** subtask-1-2, subtask-2-4 (TOCTOU analysis)

**Scenario:**
```rust
let bus = EventBus::with_policy(32, BackPressurePolicy::Block {
    timeout: Duration::from_secs(5)
});

// Thread A calls emit_awaited() with Block policy
// Time 0: Thread A checks len() < buffer_size → true (len=30, buffer=32)
// Time 1: Threads B, C emit 3 events → buffer now full (len=32)
// Time 2: Thread A calls send() → OVERWRITES OLDEST (acts like DropOldest!)
// Time 3: Thread A returns PublishOutcome::Sent

// Expected: Wait for buffer space (Block policy semantics)
// Actual: Overwrites oldest immediately (DropOldest behavior)
```

**Impact:**
- Expected: Wait up to timeout for buffer space before dropping
- Actual: Overwrites oldest event if buffer fills between check and send
- Violates documented Block policy semantics under high concurrent load

**Root Cause:**
Buffer length check at line 166 is stale by the time `send()` is called at line 170. `tokio::sync::broadcast` internally uses DropOldest when full.

**Suggested Fix:**
Re-check buffer length after send attempt:
```rust
if self.sender.len() < self.buffer_size {
    let event_clone = event.as_ref().expect("event consumed").clone();
    match self.sender.send(event_clone) {
        Ok(_) => {
            event.take();  // Consume original
            return PublishOutcome::Sent;
        }
        Err(_) => {
            // All subscribers dropped between check and send
            return PublishOutcome::DroppedNoSubscribers;
        }
    }
}
```

**Trade-off:** Requires cloning event on every send attempt. Alternatively, accept the race and document it (same as BUG-1).

---

## SEVERITY: Footgun (10 findings)

**Definition:** Code that behaves incorrectly under uncommon but valid inputs, or API design that invites misuse.

---

### FOOTGUN-1: Missing #[must_use] on EventBus::emit()

**Location:** `crates/eventbus/src/bus.rs:85`
**Source:** subtask-1-1, Phase 5 API Audit (inferred from completion notes)

**Issue:**
```rust
// Compiles without warning, but loses delivery status
bus.emit(event);  // Was it sent? Dropped? No way to know!
```

**Impact:**
- Silent observability gaps - users can't detect dropped events
- No compiler warning for `let _ = bus.emit(event)`
- Returns `PublishOutcome` which should always be checked

**Suggested Fix:**
```rust
#[must_use = "ignoring PublishOutcome loses delivery status; use stats() if outcome not needed"]
pub fn emit(&self, event: E) -> PublishOutcome {
    // ...
}
```

**Severity:** Footgun (API ergonomics, medium production impact)

---

### FOOTGUN-2: Missing #[must_use] on EventBus::emit_awaited()

**Location:** `crates/eventbus/src/bus.rs:138`
**Source:** subtask-1-1, Phase 5 API Audit

**Issue:**
Same as FOOTGUN-1 but for async variant.

**Suggested Fix:**
```rust
#[must_use = "ignoring PublishOutcome loses delivery status"]
pub async fn emit_awaited(&self, event: E) -> PublishOutcome {
    // ...
}
```

---

### FOOTGUN-3: Missing #[must_use] on Subscriber::recv()

**Location:** `crates/eventbus/src/subscriber.rs:66`
**Source:** subtask-1-3, subtask-5-1 (implied)

**Issue:**
```rust
// Compiles without warning, but event is silently discarded
sub.recv().await;  // Event lost!
```

**Impact:**
- Events can be silently discarded without compiler warning
- No indication that return value should be used
- Inconsistent with `try_recv()` which should also have #[must_use]

**Suggested Fix:**
```rust
#[must_use = "events should be processed, not discarded"]
pub async fn recv(&mut self) -> Option<E> {
    // ...
}
```

---

### FOOTGUN-4: Missing #[must_use] on Subscriber::try_recv()

**Location:** `crates/eventbus/src/subscriber.rs:82`
**Source:** subtask-1-3

**Issue:**
Same as FOOTGUN-3 but for non-blocking variant.

**Suggested Fix:**
```rust
#[must_use = "events should be processed, not discarded"]
pub fn try_recv(&mut self) -> Option<E> {
    // ...
}
```

---

### FOOTGUN-5: Missing #[must_use] on FilteredSubscriber::recv()

**Location:** `crates/eventbus/src/filtered_subscriber.rs:34`
**Source:** subtask-1-6

**Issue:**
Same as FOOTGUN-3, but for filtered subscribers. Inconsistent with `FilteredSubscriber::try_recv()` which DOES have #[must_use].

**Suggested Fix:**
```rust
#[must_use = "events should be processed, not discarded"]
pub async fn recv(&mut self) -> Option<E> {
    // ...
}
```

---

### FOOTGUN-6: Missing #[must_use] on EventBusRegistry::get_or_create()

**Location:** `crates/eventbus/src/registry.rs:69`
**Source:** Phase 5 API Audit (inferred)

**Issue:**
```rust
// Creates unused bus in registry
registry.get_or_create("tenant-123");  // No compiler warning!
```

**Impact:**
- Can create unused buses that consume memory
- Arc<EventBus<E>> should always be stored or used

**Suggested Fix:**
```rust
#[must_use = "get_or_create returns Arc<EventBus>; ignoring it creates unused bus"]
pub fn get_or_create(&self, key: K) -> Arc<EventBus<E>> {
    // ...
}
```

---

### FOOTGUN-7: Lag Counter Saturation at u64::MAX

**Location:** `crates/eventbus/src/subscriber.rs:71, 87`
**Source:** subtask-1-3

**Issue:**
```rust
// After u64::MAX events have been skipped:
let missed = sub.lagged_count();  // Returns u64::MAX
// User thinks: "I've missed exactly u64::MAX events"
// Reality: "I've missed u64::MAX + N events, but N is unknown"
```

**Impact:**
- Counter saturates at `u64::MAX` (18.4 quintillion)
- Subsequent lag events are not reflected in `lagged_count()`
- Monitoring systems see a "stuck" counter
- Cannot distinguish between "saturated" vs "no new lag"

**Likelihood:** Extremely low (requires quintillions of missed events)

**Suggested Fix (Document):**
```rust
/// Returns the cumulative count of events skipped due to lag.
///
/// **Note:** This counter saturates at `u64::MAX`. If the subscriber
/// has lagged by more than `u64::MAX` events, the counter will remain
/// at `u64::MAX` and subsequent lag will not be reflected.
#[must_use]
pub fn lagged_count(&self) -> u64 {
    self.lagged_count
}
```

**Alternative Fix (Reset API):**
```rust
/// Returns the lag count since last reset, then resets to zero.
pub fn take_lagged_count(&mut self) -> u64 {
    std::mem::replace(&mut self.lagged_count, 0)
}
```

---

### FOOTGUN-8: FilteredSubscriber::recv() Infinite Loop Risk

**Location:** `crates/eventbus/src/filtered_subscriber.rs:34-41`
**Source:** subtask-1-6

**Issue:**
```rust
// Oops, typo in filter predicate
let subscriber = bus.subscribe_filtered(EventFilter::custom(|event| {
    event.kind == "WorkflowStarted"  // Typo: never matches!
}));

// This will loop forever if no events match
subscriber.recv().await;  // Infinite loop!
```

**Impact:**
- **Severe:** Can hang tasks indefinitely
- **Detection:** Hard to debug (looks like "waiting for event")
- **Production risk:** High if filter predicates are dynamically constructed
- Currently documented as "anti-pattern warning" but easy to miss

**Mitigation (Already in place):**
Documentation warning at filtered_subscriber.rs:17-19

**Suggested Enhancement:**
```rust
/// Receives the next matching event with a timeout.
///
/// Returns `None` if timeout expires before a matching event is received.
pub async fn recv_timeout(&mut self, timeout: Duration) -> Option<E> {
    tokio::time::timeout(timeout, self.recv())
        .await
        .ok()
        .flatten()
}
```

**Severity:** Footgun (documented but easy to miss, severe impact if triggered)

---

### FOOTGUN-9: BackPressurePolicy::Block Accepts Duration::ZERO

**Location:** `crates/eventbus/src/policy.rs:28-31`
**Source:** subtask-1-6

**Issue:**
`Block { timeout: Duration::ZERO }` is semantically ambiguous - should this try once and immediately fail, or skip blocking entirely?

**Impact:**
- Unclear behavior when timeout is zero
- No validation at construction time
- Actual behavior depends on bus.rs implementation of `emit_blocking()`

**Suggested Fix:**
```rust
impl BackPressurePolicy {
    /// Creates a blocking policy with validation.
    pub fn block(timeout: Duration) -> Result<Self, PolicyError> {
        if timeout.is_zero() {
            return Err(PolicyError::ZeroTimeout);
        }
        Ok(Self::Block { timeout })
    }
}
```

**Severity:** Footgun (ambiguous semantics, could surprise users)

---

### FOOTGUN-10: EventBusStats::total_attempts() Can Overflow

**Location:** `crates/eventbus/src/stats.rs:20-22`
**Source:** subtask-1-6, subtask-3-3 (integer overflow analysis)

**Issue:**
```rust
pub const fn total_attempts(&self) -> u64 {
    self.sent_count + self.dropped_count  // Wraps on overflow!
}
```

**Scenario:**
```rust
let stats = EventBusStats {
    sent_count: u64::MAX - 100,
    dropped_count: 200,
    subscriber_count: 5,
};

// Wraps to 99 instead of u64::MAX + 100
assert_eq!(stats.total_attempts(), 99);

// drop_ratio() becomes nonsensical
assert!(stats.drop_ratio() > 1.0); // 200 / 99 = 2.02
```

**Impact:**
- **Release mode:** Wraps silently, returns incorrect result
- **Debug mode:** Panics with "attempt to add with overflow"
- Requires both counters near `u64::MAX/2` (practically impossible)

**Suggested Fix:**
```rust
pub const fn total_attempts(&self) -> u64 {
    self.sent_count.saturating_add(self.dropped_count)
}
```

**Severity:** Footgun (very unlikely in practice, but mathematically incorrect)

---

## SEVERITY: Improvement (20 findings)

**Definition:** Documentation gaps, optimization opportunities, or API consistency issues that don't affect correctness.

---

### IMPROVEMENT-1: Undocumented Panic in EventBusRegistry::new()

**Location:** `crates/eventbus/src/registry.rs:52-55`
**Source:** subtask-5-4 (documentation audit)

**Issue:**
Constructor panics when `buffer_size` is zero, but doc comment doesn't mention this.

**Suggested Fix:**
```rust
/// Creates a registry with default [`BackPressurePolicy::DropOldest`].
///
/// # Panics
///
/// Panics if `buffer_size` is zero.
#[must_use]
pub fn new(buffer_size: usize) -> Self {
    Self::with_policy(buffer_size, BackPressurePolicy::default())
}
```

**Severity:** Improvement (documentation gap)

---

### IMPROVEMENT-2: Undocumented Panic in EventBusRegistry::with_policy()

**Location:** `crates/eventbus/src/registry.rs:59-66`
**Source:** subtask-5-4

**Issue:**
Same as IMPROVEMENT-1 but for `with_policy()` constructor.

**Suggested Fix:**
```rust
/// Creates a registry with explicit back-pressure policy for each bus.
///
/// # Panics
///
/// Panics if `buffer_size` is zero.
#[must_use]
pub fn with_policy(buffer_size: usize, policy: BackPressurePolicy) -> Self {
```

---

### IMPROVEMENT-3: EventFilter Predicate Panic Behavior Undocumented

**Location:** `crates/eventbus/src/filter.rs:28-33, 44-48`
**Source:** subtask-1-6, subtask-5-4

**Issue:**
If user-provided predicate panics, what happens? Not documented.

**Scenario:**
```rust
let filter = EventFilter::custom(|event: &MyEvent| {
    event.value / event.divisor > 10  // Panics if divisor is 0
});

// This will panic and poison the subscriber
subscriber.recv().await;  // Boom!
```

**Suggested Fix:**
```rust
/// Returns `true` when the event passes this filter.
///
/// # Panics
///
/// Panics if the underlying predicate panics. Filter predicates should be panic-free.
#[must_use]
pub fn matches(&self, event: &E) -> bool {
    (self.predicate)(event)
}
```

**Severity:** Improvement (user should know predicates must not panic)

---

### IMPROVEMENT-4: Undocumented Cancel Safety for EventBus::emit_awaited()

**Location:** `crates/eventbus/src/bus.rs:138`
**Source:** subtask-2-3 (cancel safety analysis), subtask-5-4

**Issue:**
Function is cancel-safe (verified in analysis) but not documented.

**Suggested Fix:**
```rust
/// Emits an event, respecting [`BackPressurePolicy::Block`].
///
/// For `DropOldest` and `DropNewest`, behaves like [`emit`](Self::emit).
/// For `Block { timeout }`, waits up to `timeout` for buffer space before dropping.
///
/// # Cancel Safety
///
/// This function is cancel-safe. If the future is dropped before completing,
/// the event is not sent and no statistics are updated.
pub async fn emit_awaited(&self, event: E) -> PublishOutcome
```

**Severity:** Improvement (missing async safety documentation)

---

### IMPROVEMENT-5: Undocumented Cancel Safety for Subscriber::recv()

**Location:** `crates/eventbus/src/subscriber.rs:66`
**Source:** subtask-1-3, subtask-2-3, subtask-5-4

**Issue:**
Function is cancel-safe (verified in analysis) but not documented.

**Suggested Fix:**
```rust
/// Receive the next event asynchronously.
///
/// Returns `None` when the bus is closed (all senders dropped).
/// On lag (buffer overflow), skips missed events and continues.
///
/// # Cancel Safety
///
/// This function is cancel-safe. If the future is dropped before completing,
/// no events are lost. The subscriber maintains its position in the channel.
pub async fn recv(&mut self) -> Option<E> {
```

**Severity:** Improvement (missing async safety documentation)

---

### IMPROVEMENT-6: Undocumented Cancel Safety for FilteredSubscriber::recv()

**Location:** `crates/eventbus/src/filtered_subscriber.rs:34`
**Source:** subtask-2-3, subtask-5-4

**Issue:**
Same as IMPROVEMENT-5 but for filtered subscribers.

**Suggested Fix:**
```rust
/// Receives the next matching event asynchronously.
///
/// Returns `None` when the underlying bus is closed.
///
/// # Cancel Safety
///
/// This function is cancel-safe. If the future is dropped before completing,
/// the subscriber maintains its position. Events that don't match the filter
/// are discarded.
pub async fn recv(&mut self) -> Option<E> {
```

---

### IMPROVEMENT-7: Undocumented Cancel Safety for Stream Types

**Location:** `crates/eventbus/src/stream.rs:11-34, 71-77`
**Source:** subtask-2-3, subtask-5-4

**Issue:**
Stream implementations are cancel-safe but not documented.

**Suggested Fix:**
Add to SubscriberStream type documentation:
```rust
/// Stream adapter that yields events from a [`Subscriber`](crate::Subscriber).
///
/// # Cancel Safety
///
/// This stream is cancel-safe. Dropping the stream or dropping the future returned
/// by `StreamExt::next()` does not lose events. The stream maintains its position
/// in the underlying broadcast channel.
```

---

### IMPROVEMENT-8: Incomplete Drop Behavior Documentation for EventBus

**Location:** `crates/eventbus/src/bus.rs:16-46`
**Source:** subtask-1-1, subtask-5-4

**Issue:**
No documentation of what happens when `EventBus` is dropped.

**Suggested Fix:**
```rust
/// Generic broadcast event bus parameterized by event type `E`.
///
/// # Drop Behavior
///
/// When an `EventBus` is dropped, the underlying channel is closed. All active
/// subscribers will receive `None` from subsequent [`Subscriber::recv()`] calls,
/// signaling that no more events will be sent. This is a graceful shutdown mechanism.
```

**Severity:** Improvement (missing drop behavior documentation)

---

### IMPROVEMENT-9: Incomplete Drop Behavior Documentation for EventBusRegistry

**Location:** `crates/eventbus/src/registry.rs:25-44`
**Source:** subtask-5-4

**Issue:**
No documentation of what happens when `EventBusRegistry` is dropped.

**Suggested Fix:**
```rust
/// Registry that manages multiple isolated [`EventBus`] instances by key.
///
/// # Drop Behavior
///
/// When the registry is dropped, all buses it owns are dropped (subject to Arc
/// reference counting). If external references to buses exist via [`get_or_create()`](Self::get_or_create)
/// or [`get()`](Self::get), those buses remain alive until the last reference is dropped.
```

---

### IMPROVEMENT-10: Misleading "Skip to Latest Event" Documentation

**Location:** `crates/eventbus/src/subscriber.rs:7-8, 20-21, 65`
**Source:** subtask-1-3

**Issue:**
Documentation claims "skips to the latest event" but actual behavior (verified in test) skips to next available position in ring buffer, not latest.

**Current:**
```rust
/// skips to the latest event
```

**Actual behavior:**
- Buffer size: 2
- Emit events: 1, 2, 3
- Event 1 is overwritten by event 3
- `try_recv()` returns event **2** (not 3!)

**Suggested Fix:**
```rust
/// Handles [`Lagged`](broadcast::error::RecvError::Lagged) by skipping
/// overwritten events and continuing from the next available position in the
/// ring buffer, ensuring the subscriber does not block the producer.
```

**Severity:** Improvement (documentation accuracy)

---

### IMPROVEMENT-11: No Reset Mechanism for Periodic Lag Monitoring

**Location:** `crates/eventbus/src/subscriber.rs:95-99`
**Source:** subtask-1-3

**Issue:**
`lagged_count()` returns cumulative count with no way to reset. Makes periodic monitoring difficult.

**Example use case:**
```rust
// Monitoring loop - want to check lag every 60 seconds
loop {
    tokio::time::sleep(Duration::from_secs(60)).await;
    let lag = sub.lagged_count();  // ← Always cumulative, can't reset!
    // Can't tell if this is new lag or old lag
}
```

**Suggested Enhancement:**
```rust
/// Returns the lag count since last reset, then resets to zero.
pub fn take_lagged_count(&mut self) -> u64 {
    std::mem::replace(&mut self.lagged_count, 0)
}
```

**Severity:** Improvement (API enhancement for better observability)

---

### IMPROVEMENT-12: SubscriptionScope Empty String IDs Have Undefined Semantics

**Location:** `crates/eventbus/src/scope.rs:20-34`
**Source:** subtask-1-6

**Issue:**
Constructors accept `impl Into<String>`, including empty strings. Semantics unclear.

**Scenario:**
```rust
let scope = SubscriptionScope::workflow("");  // Empty ID - is this valid?
```

**Impact:**
- Semantically unclear: does empty string mean "no workflow" or "workflow with empty ID"?
- No documentation on whether empty IDs are valid

**Suggested Fix (Document):**
```rust
/// Constructs a workflow scope.
///
/// Empty string IDs are treated as distinct scopes and will only match
/// events that explicitly return `Some("")` from `workflow_id()`.
#[must_use]
pub fn workflow(id: impl Into<String>) -> Self {
    Self::Workflow(id.into())
}
```

**Severity:** Improvement (ambiguous semantics)

---

### IMPROVEMENT-13: FilteredSubscriber::try_recv() Hot Path Performance Characteristics Undocumented

**Location:** `crates/eventbus/src/filtered_subscriber.rs:45-51`
**Source:** subtask-1-6

**Issue:**
`try_recv()` loops synchronously through buffered events until match found. Can take milliseconds with large buffer and selective filter.

**Scenario:**
```rust
// Filter matches 1% of events
let filter = EventFilter::custom(|event| event.priority == "critical");

// If buffer has 100 events and none are critical:
subscriber.try_recv();  // Loops 100 times synchronously!
```

**Suggested Fix:**
```rust
/// Tries to receive the next matching event without blocking.
///
/// **Performance note:** This method loops synchronously through buffered
/// events until a match is found or the buffer is empty. With a large
/// buffer and selective filter, this may take microseconds to milliseconds.
pub fn try_recv(&mut self) -> Option<E> {
```

**Severity:** Improvement (document performance characteristics)

---

### IMPROVEMENT-14: Undocumented 'static Bound Rationale on Subscriber::into_stream()

**Location:** `crates/eventbus/src/subscriber.rs:111`
**Source:** subtask-1-3

**Issue:**
The `E: 'static` bound is more restrictive than the struct's bound (`E: Clone + Send`). Users can create a `Subscriber<E>` with non-'static events, but cannot convert it to a stream.

**Example:**
```rust
struct Event<'a>(&'a str);  // Non-'static

let bus = EventBus::<Event<'_>>::new(10);  // ✅ Compiles
let sub = bus.subscribe();  // ✅ Compiles
let stream = sub.into_stream();  // ❌ Compile error: E not 'static
```

**Suggested Fix:**
```rust
/// Converts this subscriber into a [`Stream`](futures_core::Stream).
///
/// The stream yields events until the bus is closed. Lagged events are
/// skipped automatically (same semantics as [`recv()`](Self::recv)).
///
/// # Requirements
///
/// Event type must be `'static` to satisfy `Stream` trait bounds.
/// If your event contains references, consider using `recv()` instead.
pub fn into_stream(self) -> crate::stream::SubscriberStream<E>
where
    E: 'static,
{
```

**Severity:** Improvement (API clarity)

---

### IMPROVEMENT-15: EventBus Counter Overflow Behavior Undocumented

**Location:** `crates/eventbus/src/bus.rs:44-45`
**Source:** subtask-1-2, subtask-3-3 (integer overflow analysis)

**Issue:**
`sent_count` and `dropped_count` use wrapping `fetch_add()` but wrapping behavior is not documented.

**Suggested Fix:**
```rust
/// Counters exposed by [`EventBus::stats`](crate::EventBus::stats) for observability.
///
/// # Counter Overflow
///
/// `sent_count` and `dropped_count` are `u64` counters that wrap at `u64::MAX`.
/// At 1 million events/second, overflow occurs after 584,942 years. For systems
/// exporting to Prometheus, treat these as `counter` types (not `gauge`) to
/// handle wraps correctly.
```

**Severity:** Improvement (documentation)

---

### IMPROVEMENT-16: Stats Snapshot Temporal Inconsistency Undocumented

**Location:** `crates/eventbus/src/bus.rs:222-228`
**Source:** subtask-1-2

**Issue:**
`stats()` loads `sent_count`, `dropped_count`, and `subscriber_count` independently with Relaxed ordering. Snapshots can be temporally inconsistent.

**Example:**
- Time 0: Load `sent_count` = 1000
- Time 1: Emit 500 events
- Time 2: Load `dropped_count` = 0

Result: Snapshot shows 1000 sent, 0 dropped, but system actually sent 1500.

**Suggested Fix:**
Document this in `stats()` method:
```rust
/// Returns a snapshot of event bus statistics.
///
/// **Note:** The three counter values (`sent_count`, `dropped_count`,
/// `subscriber_count`) are loaded independently. The snapshot may be
/// temporally inconsistent (values from different points in time).
/// This is acceptable for observability metrics.
#[must_use]
pub fn stats(&self) -> EventBusStats {
```

**Severity:** Improvement (document expected behavior)

---

### IMPROVEMENT-17: Missing #[must_use] on EventBusRegistry::prune_without_subscribers()

**Location:** `crates/eventbus/src/registry.rs:114`
**Source:** Phase 5 API Audit (inferred)

**Issue:**
Returns `usize` (number of pruned buses) but lacks #[must_use]. Observability gap if ignored.

**Suggested Fix:**
```rust
#[must_use = "prune_without_subscribers returns count of removed buses"]
pub fn prune_without_subscribers(&self) -> usize {
```

**Severity:** Improvement (API completeness)

---

### IMPROVEMENT-18: EventBusRegistry::stats() O(N) Lock Hold Time

**Location:** `crates/eventbus/src/registry.rs:123-141`
**Source:** subtask-1-5, subtask-4-2 (lock contention analysis)

**Issue:**
`stats()` holds read lock for O(N buses) which can cause 1-2ms write delays in registries with 10k+ buses.

**Impact:**
- Read lock held during iteration over all buses
- Blocks concurrent `get_or_create()` calls (write lock)
- Becomes noticeable with 10,000+ buses

**Suggested Optimization:**
Clone bus references before aggregating:
```rust
pub fn stats(&self) -> EventBusRegistryStats {
    let buses: Vec<Arc<EventBus<E>>> = self.buses.read()
        .values()
        .cloned()
        .collect();
    drop(guard); // Release lock early

    // Aggregate stats without holding lock
    let mut snapshot = EventBusRegistryStats::default();
    for bus in buses {
        let stats = bus.stats();
        snapshot.sent_count = snapshot.sent_count.saturating_add(stats.sent_count);
        // ...
    }
    snapshot
}
```

**Trade-off:** Requires Arc clones (O(N) atomic increments) but releases lock sooner.

**Severity:** Improvement (optimization for high-scale registries)

---

### IMPROVEMENT-19: Explicit K: Send Bound for EventBusRegistry

**Location:** `crates/eventbus/src/registry.rs:16`
**Source:** subtask-5-2 (Send/Sync analysis)

**Issue:**
`EventBusRegistry<K, E>` is Send when K: Send, but bound is implicit via auto traits. Explicit bound would improve API clarity.

**Suggested Fix:**
```rust
impl<K, E> EventBusRegistry<K, E>
where
    K: Eq + Hash + Clone + Send,  // ← Add explicit Send bound
    E: Clone + Send,
{
```

**Severity:** Improvement (optional, API clarity)

---

### IMPROVEMENT-20: ScopedEvent Trait Missing Send Requirement Documentation

**Location:** `crates/eventbus/src/scope.rs:40`
**Source:** subtask-5-2 (Send/Sync analysis)

**Issue:**
`ScopedEvent` trait doesn't document that implementers must be `Send` because `EventBus` requires `E: Send`.

**Suggested Fix:**
```rust
/// Trait for events that can be filtered by scope.
///
/// # Send Requirement
///
/// Types implementing `ScopedEvent` must also implement `Send` to be used
/// with [`EventBus<E>`](crate::EventBus), which requires `E: Clone + Send`.
pub trait ScopedEvent {
```

**Severity:** Improvement (documentation clarity)

---

## Summary Statistics

| Severity | Count | Production Impact |
|----------|-------|------------------|
| **Bug** | 2 | High - Violate documented policy semantics under concurrent load |
| **Footgun** | 10 | Medium - Can cause silent failures or confusion |
| **Improvement** | 20 | Low - Documentation gaps and optimization opportunities |
| **TOTAL** | 32 | - |

---

## Cross-References

All findings are sourced from:
- **Phase 1 (Static Analysis):** subtask-1-1 through subtask-1-6
- **Phase 2 (Concurrency Analysis):** subtask-2-1 through subtask-2-4
- **Phase 3 (Edge Case Analysis):** subtask-3-1 through subtask-3-5
- **Phase 4 (Performance Analysis):** subtask-4-1 through subtask-4-4
- **Phase 5 (API Audit):** subtask-5-1 through subtask-5-4

Detailed analysis available in respective subtask analysis files.

---

## Next Steps

**Subtask 6-2:** Select top 6 findings by impact (frequency × severity)
**Subtask 6-3:** Write concrete reproduction scenarios for each finding
**Subtask 6-4:** Suggest fixes with code snippets and rationale
**Subtask 6-5:** Generate final review report and coverage checklist

---

**Classification Complete: 2026-03-19**
