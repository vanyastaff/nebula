# API Audit: #[must_use] Coverage Analysis

**Subtask:** subtask-5-1
**Date:** 2026-03-19
**Reviewer:** Auto-Claude (Adversarial Review)

## Objective

Verify `#[must_use]` coverage on all fallible and side-effect-free functions in `nebula-eventbus`.

## Methodology

1. Catalog all public functions across 12 modules
2. Identify functions that return values
3. Classify by risk: ignoring return value could cause bugs
4. Check against Rust ecosystem conventions (tokio, std)
5. Compare with existing #[must_use] attributes (34 found)

## Verification Command

```bash
rg "#\[must_use\]" ./crates/eventbus/src/ --type rust
```

**Result:** 34 #[must_use] attributes found across 8 files.

## Current #[must_use] Coverage

### Excellent Coverage (10/10) - No Issues

#### bus.rs (10 attributes)
- ✅ `new()` - constructor
- ✅ `with_policy()` - constructor
- ✅ `has_subscribers()` - query
- ✅ `subscribe()` - builder
- ✅ `subscribe_filtered()` - builder
- ✅ `subscribe_scoped()` - builder
- ✅ `stats()` - query
- ✅ `buffer_size()` - getter
- ✅ `policy()` - getter
- ✅ `pending_len()` - query

#### subscriber.rs (2 attributes)
- ✅ `lagged_count()` - getter
- ✅ `is_closed()` - query

#### filtered_subscriber.rs (3 attributes)
- ✅ `try_recv()` - fallible receive
- ✅ `lagged_count()` - getter
- ✅ `is_closed()` - query

#### outcome.rs (1 attribute)
- ✅ `is_sent()` - query

#### stats.rs (2 attributes)
- ✅ `total_attempts()` - computed getter
- ✅ `drop_ratio()` - computed getter

#### filter.rs (4 attributes)
- ✅ `all()` - constructor
- ✅ `custom()` - constructor
- ✅ `by_scope()` - constructor
- ✅ `matches()` - predicate

#### stream.rs (2 attributes)
- ✅ `SubscriberStream::lagged_count()` - getter
- ✅ `FilteredStream::lagged_count()` - getter

#### scope.rs (3 attributes)
- ✅ `workflow()` - constructor
- ✅ `execution()` - constructor
- ✅ `resource()` - constructor

#### registry.rs (6 attributes)
- ✅ `new()` - constructor
- ✅ `with_policy()` - constructor
- ✅ `get()` - lookup
- ✅ `len()` - getter
- ✅ `is_empty()` - query
- ✅ `stats()` - aggregate query

## Missing #[must_use] - Critical

### 1. EventBus::emit() (bus.rs:85)

**Location:** `crates/eventbus/src/bus.rs:85`

**Signature:**
```rust
pub fn emit(&self, event: E) -> PublishOutcome
```

**Issue:** Returns `PublishOutcome` (Sent, DroppedNoSubscribers, DroppedByPolicy, DroppedTimeout) but result can be silently ignored.

**Impact:** Callers cannot detect if event was successfully sent or dropped. In production, this could mask:
- No active subscribers (DroppedNoSubscribers)
- Back-pressure policy dropping events (DroppedByPolicy)
- Timeout expiry (DroppedTimeout)

**Severity:** **Footgun** - ignoring return value loses critical observability data

**Recommendation:** Add `#[must_use]` with reason:

```rust
#[must_use = "ignoring PublishOutcome loses delivery status; check if event was sent or dropped"]
pub fn emit(&self, event: E) -> PublishOutcome
```

**Ecosystem Precedent:** `tokio::sync::broadcast::Sender::send()` returns `Result` which is `#[must_use]` by default. Similar pattern applies here.

---

### 2. EventBus::emit_awaited() (bus.rs:138)

**Location:** `crates/eventbus/src/bus.rs:138`

**Signature:**
```rust
pub async fn emit_awaited(&self, event: E) -> PublishOutcome
```

**Issue:** Same as `emit()` - returns `PublishOutcome` but result can be ignored.

**Impact:** Same as `emit()`. For `Block` policy, ignoring means you don't know if timeout occurred.

**Severity:** **Footgun**

**Recommendation:** Add `#[must_use]` with reason:

```rust
#[must_use = "ignoring PublishOutcome loses delivery status; especially important for Block policy timeouts"]
pub async fn emit_awaited(&self, event: E) -> PublishOutcome
```

---

### 3. Subscriber::recv() (subscriber.rs:66)

**Location:** `crates/eventbus/src/subscriber.rs:66`

**Signature:**
```rust
pub async fn recv(&mut self) -> Option<E>
```

**Issue:** Returns `Option<E>` (event or None on closure). Calling without using return value is likely a logic error.

**Impact:** Waiting for event but ignoring it wastes CPU cycles and loses data.

**Severity:** **Footgun**

**Ecosystem Precedent:** `tokio::sync::broadcast::Receiver::recv()` does NOT have `#[must_use]` (async functions rarely do). However, ignoring the return value is still a mistake.

**Recommendation:** Add `#[must_use]`:

```rust
#[must_use = "ignoring received event loses data; if you want to skip events, use try_recv() in a loop"]
pub async fn recv(&mut self) -> Option<E>
```

**Alternative:** Document this in function docs if adding `#[must_use]` to async fn is against project conventions.

---

### 4. Subscriber::try_recv() (subscriber.rs:82)

**Location:** `crates/eventbus/src/subscriber.rs:82`

**Signature:**
```rust
pub fn try_recv(&mut self) -> Option<E>
```

**Issue:** Returns `Option<E>` but has NO `#[must_use]` (unlike `FilteredSubscriber::try_recv()` which has it).

**Impact:** Ignoring return value loses event data.

**Severity:** **Footgun** + **Inconsistency**

**Recommendation:** Add `#[must_use]` for consistency with `FilteredSubscriber::try_recv()`:

```rust
#[must_use = "ignoring received event loses data"]
pub fn try_recv(&mut self) -> Option<E>
```

---

### 5. FilteredSubscriber::recv() (filtered_subscriber.rs:34)

**Location:** `crates/eventbus/src/filtered_subscriber.rs:34`

**Signature:**
```rust
pub async fn recv(&mut self) -> Option<E>
```

**Issue:** Same as `Subscriber::recv()` - returns `Option<E>` but can be ignored.

**Impact:** Same as `Subscriber::recv()` - loses filtered event data.

**Severity:** **Footgun**

**Recommendation:** Add `#[must_use]`:

```rust
#[must_use = "ignoring received event loses data"]
pub async fn recv(&mut self) -> Option<E>
```

---

### 6. EventBusRegistry::get_or_create() (registry.rs:69)

**Location:** `crates/eventbus/src/registry.rs:69`

**Signature:**
```rust
pub fn get_or_create(&self, key: K) -> Arc<EventBus<E>>
```

**Issue:** Returns `Arc<EventBus<E>>`. Has side effect (creates bus if missing) but the return value is the primary purpose. Ignoring means you created a bus but can't use it.

**Impact:** Logic error - created bus is unused, possibly causing memory leak (bus stays in registry).

**Severity:** **Footgun**

**Recommendation:** Add `#[must_use]`:

```rust
#[must_use = "the returned EventBus is needed to publish/subscribe; ignoring creates an unused bus"]
pub fn get_or_create(&self, key: K) -> Arc<EventBus<E>>
```

---

### 7. EventBusRegistry::prune_without_subscribers() (registry.rs:114)

**Location:** `crates/eventbus/src/registry.rs:114`

**Signature:**
```rust
pub fn prune_without_subscribers(&self) -> usize
```

**Issue:** Returns count of removed buses, but result can be ignored.

**Impact:** Caller cannot verify if pruning occurred or log how many buses were removed (observability gap).

**Severity:** **Improvement** (not critical, but useful for observability)

**Recommendation:** Add `#[must_use]`:

```rust
#[must_use = "the count of pruned buses is useful for observability and verification"]
pub fn prune_without_subscribers(&self) -> usize
```

---

## Not Missing #[must_use] - Justified

### registry.rs::remove() (registry.rs:90)

**Signature:**
```rust
pub fn remove(&self, key: &K) -> Option<Arc<EventBus<E>>>
```

**Justification:** Side-effect of removal is primary purpose. Return value (the removed bus) is secondary. Many Rust collection APIs (HashMap::remove, Vec::remove) do NOT have `#[must_use]` despite returning the removed element.

**Recommendation:** DO NOT add `#[must_use]` - follows std::collections convention.

---

### registry.rs::clear() (registry.rs:107)

**Signature:**
```rust
pub fn clear(&self)
```

**Justification:** Void function - no return value to use.

**Recommendation:** N/A

---

### Subscriber::into_stream() (subscriber.rs:111)

**Signature:**
```rust
pub fn into_stream(self) -> crate::stream::SubscriberStream<E>
```

**Justification:** Consumes `self`, so ignoring the call is a compile error (moved value). No `#[must_use]` needed - Rust's move semantics prevent misuse.

**Recommendation:** DO NOT add `#[must_use]` - unnecessary.

---

## Summary

### Missing #[must_use] Counts

| Severity | Count | Functions |
|----------|-------|-----------|
| **Footgun** | 6 | `emit()`, `emit_awaited()`, `recv()` (2x), `try_recv()`, `get_or_create()` |
| **Improvement** | 1 | `prune_without_subscribers()` |
| **Total** | 7 | |

### Recommendations by Priority

1. **High Priority (API Consistency):**
   - `Subscriber::try_recv()` - inconsistent with `FilteredSubscriber::try_recv()` which has #[must_use]

2. **High Priority (Observability Loss):**
   - `EventBus::emit()`
   - `EventBus::emit_awaited()`

3. **Medium Priority (Data Loss):**
   - `Subscriber::recv()`
   - `FilteredSubscriber::recv()`

4. **Medium Priority (Logic Error):**
   - `EventBusRegistry::get_or_create()`

5. **Low Priority (Observability):**
   - `EventBusRegistry::prune_without_subscribers()`

## Verification Steps

After adding `#[must_use]` attributes, verify with:

```bash
# Check updated coverage
rg "#\[must_use\]" ./crates/eventbus/src/ --type rust | wc -l
# Expected: 41 (34 current + 7 new)

# Verify no compile warnings
cargo clippy --package nebula-eventbus -- -D warnings

# Run tests
cargo nextest run --package nebula-eventbus
```

## Cross-References

- Related to **subtask-1-1-lib-analysis.md** - Finding #1: Missing #[must_use] on emit methods
- Related to **subtask-1-3-subscriber-analysis.md** - Finding #3: Missing #[must_use] on recv/try_recv
- Related to **subtask-1-6-remaining-modules-analysis.md** - Finding #3: FilteredSubscriber::recv() missing #[must_use]

## Conclusion

**Status:** ✅ Analysis complete

**Findings:** 7 functions missing `#[must_use]` - 6 footguns, 1 observability improvement.

**Coverage Grade:** B+ (34/41 = 82.9% coverage)

After adding recommended attributes, coverage will be **A+ (41/41 = 100%)**.

**Production Impact:** Medium - ignoring `emit()` outcomes and `recv()` values can cause silent data loss and observability gaps in production systems.

**No bugs found** - all missing attributes are API design improvements, not correctness issues.
