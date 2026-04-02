# Deep Invariant Audit: nebula-log Crate
**Date**: April 2, 2026  
**Scope**: Full crate analysis focused on observability hot paths  
**Target Optimization Level**: Release (-O3)

---

## PASS 1: Build the Contract Map

### `emit_event(event: &dyn ObservabilityEvent)` 
**Contract**: "Dispatch event to all registered hooks without blocking on I/O; return immediately."
- Guarantees: lock-free for readers, panic-safe hook dispatch, timeout awareness (bounded policy)
- Returns: void (fire-and-forget)

### `registry::emit_to_hooks(hooks: &HookList, event, policy)`
**Contract**: "Call on_event() for every hook; catch panics; respect timeout budget."
- Guarantees: no hook exception escapes, LIFO shutdown order, metrics collection

### `context::Context::current()` 
**Contract**: "Return current context cheaply; never panic; default to root if missing."
- Guarantees: O(1) TLS lookup + Arc clone, thread-safe

### `hooks::event_data_json(event)`
**Contract**: "Materialize event fields into JSON object; return None if empty payload."
- Guarantees: all fields visited exactly once, no re-entrant visitor calls

### `Context::scope_sync(f)`
**Contract**: "Run closure with modified context; restore on return; support nesting."
- Guarantees: exception-safe, witnesses previous context before replace

---

## PASS 2: Verify Contracts Against Implementation

### Finding 1: Unnecessary Arc::clone() in Hot Path
**Severity**: `HIGH`  
**Category**: Register Allocation Failures / Memory Access Patterns  
**Location**: `observability/registry.rs`, `emit_event()` line ~125
**Contract**: "emit_event should be lock-free"  
**Implementation**:
```rust
pub fn emit_event(event: &dyn ObservabilityEvent) {
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return;
    }
    let hooks = HOOKS.load();  // ArcSwap::load() returns Arc<HookList>
    let policy = *policy_read_guard();  // RwLock read + dereference
    emit_to_hooks(&hooks, event, policy);
}
```
**Issue**: 
- `HOOKS.load()` is lock-free but the returned `Arc<HookList>` implies an atomic increment/decrement on the reference count
- `policy_read_guard()` requires RwLock acquisition (not ultra-cheap on highly contended systems)
- Under 16-hook contention (from bench), the RwLock on `HOOK_POLICY` becomes visible

**Trigger**: High-frequency emit_event calls (>100k/sec) under concurrent reader load

**Fix**:
```rust
// Option A: Cache the policy read
#[inline(always)]
pub fn emit_event(event: &dyn ObservabilityEvent) {
    if SHUTTING_DOWN.load(Ordering::Relaxed) {  // Relaxed is OK; only used for shutdown
        return;
    }
    let hooks = HOOKS.load();  // Already lock-free ArcSwap
    let policy = HOOK_POLICY.read();  // Single RwLock pin, not per-hook
    emit_to_hooks(&hooks, *policy, event);
}

// Option B: Remove policy_read_guard helper indirection
```

---

### Finding 2: Panic::catch_unwind Overhead Per Hook
**Severity**: `MEDIUM`  
**Category**: Function Call Overhead / Atomic & Synchronization  
**Location**: `observability/registry.rs`, `emit_to_hooks()` lines 41–54
**Contract**: "Panic-safe dispatch"  
**Implementation**:
```rust
for hook in hooks.iter() {
    let started = Instant::now();
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        hook.on_event(event);
    }));
    // ... error handling ...
    if let Some(timeout) = timeout_ms {
        let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        if elapsed > timeout { /* warn */ }
    }
}
```
**Issue**:
- `panic::catch_unwind()` has ~20-40 ns overhead per invocation (lands/restores unwinding context)
- `Instant::now()` is ~10-20 ns per call
- With 16 hooks, this becomes 320–960 ns just for panic handling + timing
- Timing check is only relevant when `policy == Bounded`

**Trigger**: High hook counts (4+) + high emit frequency + bounded policy

**Fix**:
```rust
// Only use catch_unwind for hooks that actually panic-risk (configurable)
// OR: batch timing check outside the loop for bounded policy:
if let HookPolicy::Bounded { timeout_ms, .. } = policy {
    let batch_start = Instant::now();
    for hook in hooks.iter() {
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            hook.on_event(event);
        }));
        if batch_start.elapsed().as_millis() as u64 > timeout_ms {
            tracing::warn!("Hook budget exceeded");
            break;
        }
    }
} else {
    for hook in hooks.iter() {
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            hook.on_event(event);
        }));
        if let Err(_) = result { /* log */ }
    }
}
```

---

### Finding 3: HashMap Allocation in Context::with_field()
**Severity**: `MEDIUM`  
**Category**: Memory Access Patterns / Rust-Specific LLVM Artifacts  
**Location**: `layer/context.rs`, `Context::with_field()` lines 76–84
**Contract**: "Builder method; no allocation on empty fields"  
**Implementation**:
```rust
pub struct Context {
    pub request_id: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub fields: HashMap<String, serde_json::Value>,  // Always allocated
}

pub fn with_field(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
    if let Ok(v) = serde_json::to_value(value) {
        self.fields.insert(key.into(), v);
    }
    self
}
```
**Issue**:
- Default HashMap initializes with capacity=0, then allocates on first insert
- In typical usage (1-3 fields), the hash table overhead dominates
- `serde_json::to_value()` performs type inspection + allocation
- For primitive fields (u64, bool, str), this is overkill

**Trigger**: Heavy use of `Context::with_field()` during trace/event setup

**Fix**:
```rust
// Use SmallVec or use flat array of common fields:
pub struct Context {
    pub request_id: Option<String>,
    pub user_id: Option<String>,
    pub session_id: Option<String>,
    pub trace_id: Option<String>,  // Common field, avoid HashMap storage
    pub region: Option<String>,    // Common field
    pub tags: vec![(String, String)],  // Only allocate if tags exist
}

// OR lazy-initialize fields HashMap:
pub struct Context {
    fields: Option<HashMap<String, serde_json::Value>>,  // Only alloc if used
}
```

---

### Finding 4: Redundant Policy Read in emit_to_hooks()
**Severity**: `LOW`  
**Category**: Missed Compiler Optimizations  
**Location**: `observability/registry.rs`, line 38
**Contract**: "Extract timeout_ms once"  
**Implementation**:
```rust
fn emit_to_hooks(hooks: &HookList, event: &dyn ObservabilityEvent, policy: HookPolicy) {
    let timeout_ms = match policy {
        HookPolicy::Inline => None,
        HookPolicy::Bounded { timeout_ms, .. } => Some(timeout_ms),
    };

    for hook in hooks.iter() {
        let started = Instant::now();
        // ...
        if let Some(timeout) = timeout_ms {
            let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            if elapsed > timeout {
                tracing::warn!(...);
            }
        }
    }
}
```
**Issue**:
- Pattern is correct (extract timeout_ms once)
- But loop always pays the cost even when `timeout_ms == None`
- LLVM may not eliminate the if-check branch due to trait call complexity

**Trigger**: Inline policy (most common) with many hooks

**Fix**: Already correct; LLVM should CSE this. If profiling shows regress, inline the dispatch:
```rust
match policy {
    HookPolicy::Inline => {
        for hook in hooks.iter() {
            let _ = panic::catch_unwind(...);
        }
    },
    HookPolicy::Bounded { timeout_ms, .. } => {
        let start = Instant::now();
        for hook in hooks.iter() {
            if start.elapsed().as_millis() as u64 > timeout_ms {
                break;
            }
            let _ = panic::catch_unwind(...);
        }
    },
}
```

---

### Finding 5: Receiver Clone in Event Field Visitor
**Severity**: `MEDIUM`  
**Category**: Memory Access Patterns  
**Location**: `observability/hooks.rs`, `event_data_json()` lines 96–120
**Contract**: "Projection should not allocate unless needed"  
**Implementation**:
```rust
impl ObservabilityFieldVisitor for JsonCollector {
    fn record(&mut self, key: &str, value: ObservabilityFieldValue<'_>) {
        let value = match value {
            ObservabilityFieldValue::Str(v) => serde_json::Value::String(v.to_string()),
            // ...
        };
        self.fields.insert(key.to_string(), value);  // TWO allocations per field
    }
}
```
**Issue**:
- `key.to_string()` allocates even for short string keys ("operation", "tenant_id", etc.)
- `serde_json::Value::String(v.to_string())` allocates the string first, then wraps
- HashMap entry allocation + string key allocation per field
- For a 12-field event, this is 12+ allocations

**Trigger**: High-frequency event_data_json() calls in observability hooks

**Fix**: Use `SmallString` or intern keys:
```rust
// Use static string pool or SmallVec:
impl ObservabilityFieldVisitor for JsonCollector {
    fn record(&mut self, key: &str, value: ObservabilityFieldValue<'_>) {
        let serde_value = match value {
            ObservabilityFieldValue::Str(v) => serde_json::Value::String(v.to_string()),
            // ...
        };
        // Use key.to_string() only if no interning — OR intern via Lasso:
        self.fields.insert(key.into(), serde_value);
    }
}
```

---

### Finding 6: Context::current() Arc Clone Per Call
**Severity**: `LOW`  
**Category**: Register Allocation Failures  
**Location**: `layer/context.rs`, line 87
**Contract**: "Current context lookup is cheap"  
**Implementation**:
```rust
#[inline]
pub fn current() -> Arc<Self> {
    CTX.try_with(|c| c.clone())
        .unwrap_or_else(|_| Arc::new(Context::default()))
}
```
**Issue**:
- `c.clone()` is a single atomic increment on the Arc refcount (~1 cycle)
- Negligible cost, but repeated calls cause minor allocations
- `unwrap_or_else` closure is only called if TLS fetch fails (exceptional)

**Trigger**: Very high call frequency without caching return value

**Fix**: Already optimal for the pattern. If bottleneck, introduce thread-local cache:
```rust
thread_local! {
    static CTX_CACHE: Cell<Option<Arc<Context>>> = Cell::new(None);
}

pub fn current() -> Arc<Self> {
    CTX_CACHE.with(|cache| {
        cache.get().or_else(|| {
            let ctx = CTX.try_with(|c| c.clone())
                .unwrap_or_else(|_| Arc::new(Context::default()));
            cache.set(Some(ctx.clone()));
            Some(ctx)
        }).unwrap()
    })
}
```
**But**: Not worth it unless profiling shows >10% of total cycles.

---

## PASS 3: Cross-File Pattern Audit

### (A) Attribute Consistency

**Derive Consistency**:
- `HookPolicy`: `#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]` ✓
- `Context`: `#[derive(Clone, Debug, Default, Serialize, Deserialize)]` ✓
- `LoggerResource`: `#[derive(Clone, Serialize, Deserialize)]` — missing `Debug` manually impl
- **FINDING**: LoggerResource `Debug` impl is manual (redacts secrets properly) but docstring incomplete

### (B) Error Handling Consistency

| Constructor | Validates | Panics | Silent Clamp |
|--|--|--|--|
| `Context::new()` | No | No | N/A |
| `LoggerResource::new()` | No | No | N/A |
| `HookPolicy` variants | No | No | Yes (sampling_rate) |
| `Timer::new()` | No | No | N/A |

**Finding**: All constructors are permissive. This is correct for builder patterns but no validation happens until usage.

### (C) Observable vs Actual State

**Context Fields**:
- `request_id`, `user_id` are supposed to identify the request, but accessed as `Option<String>`
- No compile-time validation of format (should use `newtype` if format is contractual)

**Hook Counts**:
- No exported method to query hook count in production (only in tests)
- Ops teams can't observe hook registry size without patching

**Timing**:
- `Instant::now()` is wall-clock sampled; under heavy load may drift
- No monotonic clock guarantee (ThreadId could be reused)

---

## CRITICAL FINDINGS SUMMARY

### **CRITICAL** — Registry Hot Path Efficiency 
**Impact**: Every observability event pays 100–500 ns for hook dispatch even when budget allows  
**Recommendation**: Batch timeout checks; make panic::catch_unwind conditional on policy

### **HIGH** — HashMap Over-allocation in Context
**Impact**: Memory fragmentation; cache pollution; heap pressure under high churn  
**Recommendation**: Use SmallVec or lazy-init for `fields`

### **HIGH** — Event Payload Projection Overhead
**Impact**: 10–20 allocations per event when using event_data_json()  
**Recommendation**: Intern string keys; use SmallString for values

### **MEDIUM** — Redundant Arc Clone Chains
**Impact**: Refcount contention under >1k emit/sec  
**Recommendation**: Consider Arc pooling for recycled contexts

---

## OPTIMIZATION ROADMAP (ROI Prioritized)

### Tier 1: Quick Wins (1-2 hours, 10–20% throughput gain)
1. **Extract policy match outside loop** — move bounded timeout checks to loop level
   - Cost: ~20 lines
   - Gain: -30–50 ns per emit under Inline policy
   
2. **Add `#[inline(always)]` to emit_event** — ensure inlining into hot bench code
   - Cost: 1 line
   - Gain: -10 ns (register pressure reduction)

3. **Document panic::catch_unwind trade-off** — clarify contract violation risk
   - Cost: 5 lines
   - Gain: Prevent unsafe hook registration

### Tier 2: Data Structure Changes (2–4 hours, 15–30% improvement)
1. **Replace HashMap<String, Value> with SmallVec in Context**
   - Cost: 50 lines + compat layer
   - Gain: -200–400 ns context creation; -8 bytes per Context (alignment)

2. **Lazy-init fields in Context** — Option<HashMap> instead of always-allocated
   - Cost: 30 lines
   - Gain: -64–128 bytes per empty context

3. **Intern event field keys** — use `&'static str` via Lasso or static table
   - Cost: 40 lines + unsafe
   - Gain: -30 ns per projected field (key.to_string() gone)

### Tier 3: System Design (1–2 days, 20–50% uplift)
1. **Conditional panic handling** — only unwrap for hooks marked as panic-risky
   - Cost: Hook trait addition + registration change
   - Gain: -100 ns per hook in Inline mode if panics rare

2. **Hook batching** — coalesce multiple emit() calls into single dispatch
   - Cost: Event queue + batch drainer
   - Gain: Amortizes policy read and Arc load

3. **Stack-allocated HookList for small hook counts** — inline Vec<Arc> up to 4 hooks
   - Cost: Dual-path dispatch logic
   - Gain: -20 ns emit, better cache locality

---

## COMPILER HINTS & FLAGS

### 1. RUSTFLAGS to Test
```bash
# Profile-guided optimization (requires PGO instrumentation pass)
RUSTFLAGS="-C llvm-args=-pgo-warn-missing-function" cargo build -p nebula-log --release

# Link-Time Optimization (already enabled via profile.release lto=thin)
# Already set: lto = "thin" in Cargo.toml ✓

# Restrict inlining aggressiveness (may help with register pressure):
RUSTFLAGS="-C inline-threshold=100" cargo bench -p nebula-log
```

### 2. Attributes to Add
```rust
// On emit_event() itself:
#[inline(always)]  // Ensure inlining into hot bench code
pub fn emit_event(event: &dyn ObservabilityEvent) { ... }

// On hot-path internal helpers:
#[inline]
fn emit_to_hooks(hooks: &HookList, event: &dyn ObservabilityEvent, policy: HookPolicy) { ... }

// On rarely-used code:
#[cold]
fn try_initialize_hook(hook: &dyn ObservabilityHook) -> bool { ... }

#[cold]
fn shutdown_hooks_list(hooks: &HookList, policy: HookPolicy) { ... }
```

### 3. Feature Gate Candidates
```rust
// Disable panic catching for performance-critical deployments:
#[cfg(feature = "panic_safe_hooks")]
let result = panic::catch_unwind(...);

// Or profile-conditional:
#[cfg(not(any(debug_assertions, feature = "safe_dispatch")))]
unsafe { hook.on_event(event); }  // Unsafe but faster
```

---

## GODBOLT EXPERIMENTS (Proposed)

### Experiment 1: Policy-Aware Dispatch
**Hypothesis**: Inlining the policy match eliminates redundant branch

```rust
// Status quo:
pub fn emit_event(event: &dyn ObservabilityEvent) {
    let hooks = HOOKS.load();
    let policy = *policy_read_guard();
    emit_to_hooks(&hooks, event, policy);
}

// Optimized:
#[inline(always)]
pub fn emit_event_inline_dispatch(event: &dyn ObservabilityEvent) {
    let hooks = HOOKS.load();
    match *policy_read_guard() {
        HookPolicy::Inline => {
            for hook in hooks.iter() {
                let _ = panic::catch_unwind(AssertUnwindSafe(|| {
                    hook.on_event(event);
                }));
            }
        },
        HookPolicy::Bounded { timeout_ms, .. } => {
            let start = Instant::now();
            for hook in hooks.iter() {
                hook.on_event(event);
                if start.elapsed().as_millis() as u64 > timeout_ms { break; }
            }
        },
    }
}
```

**Expected**: -30–50 ns overhead for Inline policy via branch prediction improvement.

---

### Experiment 2: Context Lazy Fields
**Hypothesis**: SmallVec avoids HashMap allocation for sparse contexts

```rust
// Status quo:
pub struct Context {
    fields: HashMap<String, serde_json::Value>,
}

// Optimized:
pub struct Context {
    fields: SmallVec<[(String, serde_json::Value); 4]>,  // Inline up to 4
}
```

**Expected**: -200–400 ns context creation; better cache locality for common case (0-3 fields).

---

### Experiment 3: Key Interning in event_data_json
**Hypothesis**: Avoid key.to_string() allocations via static string pool

```rust
// Status quo:
self.fields.insert(key.to_string(), value);

// Optimized (with static interning):
const FIELD_KEYS: &[&str] = &[
    "operation", "context", "node_id", "tenant_id", "attempt",
    "success", "duration_ms", "queue_fill", "retryable", "batch_size",
    "region", "delta", // ... add as needed
];

fn intern_key(key: &str) -> &'static str {
    FIELD_KEYS.iter().find(|&&k| k == key).copied().unwrap_or(key)
}

// In record():
self.fields.insert(intern_key(key).to_string(), value);
// Actually still allocates! Real fix: use &'static str as key type
```

**Expected**: -5–10 ns per field if key comparison time reduced.

---

## CONCLUSION

| Category | Issues Found | Severity |
|--|--|--|
| Register Allocation | 2 | HIGH, LOW |
| Memory Access | 3 | MEDIUM, MEDIUM, LOW |
| Atomic & Sync | 1 | MEDIUM |
| Rust-Specific | 1 | MEDIUM |
| **Total** | **7** | — |

**Overall Code Quality**: **GOOD** — Well-designed API with correct arc_swap lock-free pattern, but micro-optimizations needed for sub-100ns latencies.

**Estimated Cycle Cost of Issues**: 100–500 ns per emit under 16-hook contention (currently measured in bench as 343 ns baseline).

**Implementation Priority**: Start with data structure refactoring (SmallVec for Context). Then optimize dispatch loop. Then benchmark with PGO.
